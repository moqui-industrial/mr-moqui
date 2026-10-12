/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! Execution context factory. One factory is shared. One context is thread-local.

use std::cell::RefCell;
use std::cell::RefMut;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use moqui_lib::env;
use moqui_lib::node::MNode;

use super::artifact::{ArtifactStack, AuthzGuard};
use super::cache::CacheFacade;
use super::component::{self, ComponentInfo};
use super::conf;
use super::error::Error;
use super::message::MessageFacade;
use super::resource::ResourceFacade;
use crate::entity::EntityFacade;

const ENV_HEADER: &str = "# This file holds secrets and must be kept with the instance backup.\n";

pub struct ExecutionContextFactory {
    runtime_dir: PathBuf,
    framework_dir: PathBuf,
    components: Vec<ComponentInfo>,
    resource: ResourceFacade,
    cache: CacheFacade,
    conf_text: String,
    property_names: Vec<String>,
    /// Entity facade for `dbresource://`. Empty until the host calls [`Self::set_entity`].
    entity: Arc<RwLock<Option<EntityFacade>>>,
}

impl ExecutionContextFactory {
    pub fn boot(runtime_dir: impl AsRef<Path>) -> Result<Arc<Self>, Error> {
        let runtime_dir = canonicalize_dir(runtime_dir.as_ref())?;
        let framework_dir = canonicalize_dir(&runtime_dir.join("../framework"))?;
        if !framework_dir.join("component.xml").is_file() {
            return Err(Error::new(format!(
                "framework directory {} has no component.xml",
                framework_dir.display()
            )));
        }
        Self::boot_with_framework(runtime_dir, framework_dir)
    }

    pub fn boot_with_framework(
        runtime_dir: impl AsRef<Path>,
        framework_dir: impl AsRef<Path>,
    ) -> Result<Arc<Self>, Error> {
        assert_shareable::<Self>();
        let runtime_dir = canonicalize_dir(runtime_dir.as_ref())?;
        let framework_dir = canonicalize_dir(framework_dir.as_ref())?;
        if !framework_dir.join("component.xml").is_file() {
            return Err(Error::new(format!(
                "framework directory {} has no component.xml",
                framework_dir.display()
            )));
        }
        let conf_node = conf::load_merged_conf(&framework_dir, &runtime_dir)?;
        let env_path = runtime_dir.join(".env");
        let (property_names, mut set_by_boot) = apply_default_properties(&conf_node, &env_path)?;
        let components = component::discover(&runtime_dir, &framework_dir, &conf_node)?;
        let entity = Arc::new(RwLock::new(None));
        let resource = ResourceFacade::new(runtime_dir.clone(), component_roots(&components), Arc::clone(&entity));
        let cache = CacheFacade::from_conf(&conf_node)?;
        set_by_boot.keep();
        let conf_text = conf_node.write_string();
        tracing::info!(
            runtime = %runtime_dir.display(),
            framework = %framework_dir.display(),
            "Initialized execution context factory"
        );
        Ok(Arc::new(Self {
            runtime_dir,
            framework_dir,
            components,
            resource,
            cache,
            conf_text,
            property_names,
            entity,
        }))
    }

    pub fn new_context(self: &Arc<Self>) -> ExecutionContext {
        ExecutionContext {
            factory: Arc::clone(self),
            messages: RefCell::new(MessageFacade::new()),
            artifacts: RefCell::new(ArtifactStack::new()),
        }
    }

    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn framework_dir(&self) -> &Path {
        &self.framework_dir
    }

    pub fn components(&self) -> &[ComponentInfo] {
        &self.components
    }

    pub fn resource(&self) -> &ResourceFacade {
        &self.resource
    }

    pub fn cache(&self) -> &CacheFacade {
        &self.cache
    }

    pub fn property_names(&self) -> &[String] {
        &self.property_names
    }

    pub fn conf_text(&self) -> &str {
        &self.conf_text
    }

    /// Install the entity facade. `dbresource://` reads and writes through it.
    pub fn set_entity(&self, facade: EntityFacade) {
        *self.entity.write().unwrap_or_else(|err| err.into_inner()) = Some(facade);
    }

    /// The installed entity facade, when the host has registered one.
    pub fn entity(&self) -> Option<EntityFacade> {
        self.entity.read().unwrap_or_else(|err| err.into_inner()).clone()
    }
}

pub struct ExecutionContext {
    factory: Arc<ExecutionContextFactory>,
    messages: RefCell<MessageFacade>,
    artifacts: RefCell<ArtifactStack>,
}

impl ExecutionContext {
    pub fn factory(&self) -> &Arc<ExecutionContextFactory> {
        &self.factory
    }

    pub fn message(&self) -> RefMut<'_, MessageFacade> {
        self.messages.borrow_mut()
    }

    pub fn push(&self, artifact_type: &str, name: &str, action: &str) {
        self.artifacts.borrow_mut().push(artifact_type, name, action);
    }

    pub fn pop(&self) -> Result<(), Error> {
        self.artifacts.borrow_mut().pop()
    }

    pub fn disable_authz(&self) -> AuthzGuard {
        self.artifacts.borrow().disable_authz()
    }

    pub fn allows(&self) -> bool {
        self.artifacts.borrow().allows()
    }

    pub fn authz_disabled(&self) -> bool {
        self.artifacts.borrow().authz_disabled()
    }
}

fn assert_shareable<T: Send + Sync>() {}

fn canonicalize_dir(path: &Path) -> Result<PathBuf, Error> {
    std::fs::canonicalize(path)
        .map_err(|err| Error::new(format!("could not resolve directory {}", path.display())).with_source(err))
}

fn component_roots(components: &[ComponentInfo]) -> std::collections::HashMap<String, PathBuf> {
    components
        .iter()
        .map(|info| (info.name.clone(), info.location.clone()))
        .collect()
}

fn apply_default_properties(conf_node: &MNode, env_path: &Path) -> Result<(Vec<String>, PropertyRollback), Error> {
    ensure_env_readable(env_path)?;
    if env_path.is_file() {
        env::load_dotenv_file(env_path)?;
    }
    let mut names = Vec::new();
    let mut set_by_boot = PropertyRollback::default();
    let mut generated = Vec::new();
    for node in conf_node.children_named("default-property") {
        let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) else {
            continue;
        };
        names.push(name.clone());
        let secret = is_secret(&name, node.attribute("is-secret").as_deref());
        let resolved = env::get_prop_or_env(&name);
        if !resolved.is_empty() {
            log_property(&name, &resolved, secret, false);
            continue;
        }
        let raw = node.attribute("value").unwrap_or_default();
        let mut value = if raw.is_empty() {
            String::new()
        } else {
            env::expand(&raw)?
        };
        if secret && value.ends_with("CHANGEME") && !leave_sentinel(&name) {
            value = generate_secret()?;
            env::set_property(&name, value.clone());
            set_by_boot.remember(&name);
            generated.push((name.clone(), value));
            tracing::info!("Generated secret property {name}");
            continue;
        }
        if value.is_empty() {
            continue;
        }
        env::set_property(&name, value.clone());
        set_by_boot.remember(&name);
        log_property(&name, &value, secret, true);
    }
    append_env_secrets(env_path, &generated)?;
    Ok((names, set_by_boot))
}

fn log_property(name: &str, value: &str, secret: bool, from_default: bool) {
    if secret {
        if from_default {
            tracing::info!("Setting secret property {name} from default");
        } else {
            tracing::info!("Found secret property {name}");
        }
        return;
    }
    if from_default {
        tracing::info!("Setting property {name} from default with value [{value}]");
    } else {
        tracing::info!("Found property {name} with value [{value}]");
    }
}

fn is_secret(name: &str, flag: Option<&str>) -> bool {
    let flag = flag.unwrap_or("");
    flag != "false" && (flag == "true" || name.contains("pass") || name.contains("pw") || name.contains("key"))
}

fn leave_sentinel(name: &str) -> bool {
    matches!(
        name,
        "entity_ds_crypt_pass_old" | "cluster_tls_cert" | "cluster_tls_key"
    )
}

fn ensure_env_readable(path: &Path) -> Result<(), Error> {
    if path.exists() && !path.is_file() {
        Err(Error::new(format!(
            "could not read runtime environment file {}: path is a directory",
            path.display()
        )))
    } else {
        Ok(())
    }
}

fn generate_secret() -> Result<String, Error> {
    let mut bytes = [0_u8; 32];
    let mut file = std::fs::File::open("/dev/urandom")
        .map_err(|err| Error::new("could not read random bytes for a secret property").with_source(err))?;
    std::io::Read::read_exact(&mut file, &mut bytes)
        .map_err(|err| Error::new("could not read random bytes for a secret property").with_source(err))?;
    Ok(standard_base64(&bytes))
}

fn standard_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8) | (bytes[index + 2] as u32);
        push_base64(&mut out, ALPHABET, chunk, 4);
        index += 3;
    }
    let rest = bytes.len() - index;
    if rest == 1 {
        let chunk = (bytes[index] as u32) << 16;
        push_base64(&mut out, ALPHABET, chunk, 2);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let chunk = ((bytes[index] as u32) << 16) | ((bytes[index + 1] as u32) << 8);
        push_base64(&mut out, ALPHABET, chunk, 3);
        out.push('=');
    }
    out
}

fn push_base64(out: &mut String, alphabet: &[u8], chunk: u32, count: usize) {
    let shifts = [18, 12, 6, 0];
    for shift in shifts.into_iter().take(count) {
        let piece = ((chunk >> shift) & 63) as usize;
        out.push(alphabet[piece] as char);
    }
}

fn append_env_secrets(path: &Path, pairs: &[(String, String)]) -> Result<(), Error> {
    if pairs.is_empty() {
        return Ok(());
    }
    if path.exists() && !path.is_file() {
        return Err(Error::new(format!(
            "could not append runtime environment file {}: path is a directory",
            path.display()
        )));
    }
    for (name, value) in pairs {
        if name.contains(['=', '\n', '\r']) || value.contains(['\n', '\r']) {
            return Err(Error::new(format!(
                "could not append runtime environment file {}: invalid secret name",
                path.display()
            )));
        }
    }
    let existing = if path.is_file() {
        std::fs::read_to_string(path).map_err(|err| {
            Error::new(format!("could not read runtime environment file {}", path.display())).with_source(err)
        })?
    } else {
        String::new()
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| {
            Error::new(format!("could not append runtime environment file {}", path.display())).with_source(err)
        })?;
    if existing.is_empty() {
        file.write_all(ENV_HEADER.as_bytes()).map_err(|err| {
            Error::new(format!("could not append runtime environment file {}", path.display())).with_source(err)
        })?;
    }
    let mut body = String::new();
    for (name, value) in pairs {
        body.push_str(name);
        body.push('=');
        body.push_str(value);
        body.push('\n');
    }
    file.write_all(body.as_bytes()).map_err(|err| {
        Error::new(format!("could not append runtime environment file {}", path.display())).with_source(err)
    })?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|err| {
        Error::new(format!(
            "could not set permissions on runtime environment file {}",
            path.display()
        ))
        .with_source(err)
    })?;
    Ok(())
}

#[derive(Default)]
struct PropertyRollback {
    names: Vec<String>,
    rollback: bool,
}

impl PropertyRollback {
    fn remember(&mut self, name: &str) {
        self.names.push(name.to_string());
        self.rollback = true;
    }

    fn keep(&mut self) {
        self.rollback = false;
    }

    fn clear(&self) {
        for name in &self.names {
            env::clear_property(name);
        }
    }
}

impl Drop for PropertyRollback {
    fn drop(&mut self) {
        if self.rollback {
            self.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::standard_base64;

    #[test]
    fn standard_base64_matches_padded_vectors() {
        assert_eq!(standard_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(standard_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(standard_base64(b"foob"), "Zm9vYg==");
        assert_eq!(standard_base64(b"f"), "Zg==");
        assert_eq!(standard_base64(&[0_u8; 32]).len(), 44);
        assert!(standard_base64(&[0_u8; 32]).ends_with('='));
    }
}
