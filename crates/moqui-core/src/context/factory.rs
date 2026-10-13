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

use std::cell::{Cell, RefCell, RefMut};
use std::fs::OpenOptions;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Arc, RwLock};

use moqui_lib::env;
use moqui_lib::node::MNode;

use super::artifact::{ArtifactStack, AuthzGuard};

thread_local! {
    /// Depth and the context the outermost guard borrowed. This does not own the context.
    static ENTERED: Cell<(*const ExecutionContext, u32)> = const { Cell::new((std::ptr::null(), 0)) };
}
use super::cache::CacheFacade;
use super::component::{self, ComponentInfo};
use super::conf;
use super::confcache::ConfCache;
use super::error::Error;
use super::message::MessageFacade;
use super::resource::ResourceFacade;
use crate::entity::EntityFacade;
use crate::user::AuthInfo;

const ENV_HEADER: &str = "# This file holds secrets and must be kept with the instance backup.\n";

pub struct ExecutionContextFactory {
    runtime_dir: PathBuf,
    framework_dir: PathBuf,
    components: Vec<ComponentInfo>,
    resource: ResourceFacade,
    cache: CacheFacade,
    conf_text: String,
    conf_cache: std::sync::OnceLock<ConfCache>,
    /// Hit times per `userId@type:name`, for tarpit counting.
    tarpit_hits: std::sync::Mutex<std::collections::HashMap<String, Vec<i64>>>,
    property_names: Vec<String>,
    /// Entity facade for `dbresource://`. Empty until the host calls [`Self::set_entity`].
    entity: Arc<RwLock<Option<EntityFacade>>>,
    /// This factory's pool guard. Drops with the factory. Idle has no workers.
    #[allow(dead_code)] // Later slice reads this to start and stop its own workers.
    pools: crate::service::ServicePools,
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
        let pools = crate::service::start_pools(&conf_text)?;
        super::locate::remember(&framework_dir, &components);
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
            conf_cache: std::sync::OnceLock::new(),
            tarpit_hits: std::sync::Mutex::new(std::collections::HashMap::new()),
            property_names,
            entity,
            pools,
        }))
    }

    pub fn new_context(self: &Arc<Self>) -> ExecutionContext {
        ExecutionContext {
            factory: Arc::clone(self),
            messages: RefCell::new(MessageFacade::new()),
            artifacts: RefCell::new(ArtifactStack::new()),
            session: RefCell::new(crate::user::Memory::default()),
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

    /// True when the `artifact-execution` element for `artifact_type` has `flag="true"`.
    pub fn artifact_flag(&self, artifact_type: &str, flag: &str) -> bool {
        self.conf().artifact_flag(artifact_type, flag)
    }

    /// A non-empty `user-facade` child attribute from the conf.
    pub fn user_facade_attr(&self, child: &str, name: &str) -> Option<String> {
        self.conf().user_facade_attr(child, name)
    }

    fn conf(&self) -> &ConfCache {
        self.conf_cache.get_or_init(|| ConfCache::parse(&self.conf_text))
    }

    /// Hit times recorded for a tarpit key, oldest first.
    pub fn tarpit_times(&self, key: &str) -> Vec<i64> {
        self.tarpit_hits
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// Record a hit. Times older than `keep_ms` are dropped so the list stays small.
    pub fn record_tarpit_hit(&self, key: &str, now_ms: i64, keep_ms: i64) {
        let mut hits = self.tarpit_hits.lock().unwrap_or_else(|err| err.into_inner());
        let list = hits.entry(key.to_string()).or_default();
        list.retain(|time| now_ms - *time < keep_ms.max(1));
        list.push(now_ms);
    }

    pub fn clear_tarpit_hits(&self, key: &str) {
        self.tarpit_hits
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(key);
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
    session: RefCell<crate::user::Memory>,
}

impl ExecutionContext {
    pub fn factory(&self) -> &Arc<ExecutionContextFactory> {
        &self.factory
    }

    pub(crate) fn session(&self) -> RefMut<'_, crate::user::Memory> {
        self.session.borrow_mut()
    }

    pub fn message(&self) -> RefMut<'_, MessageFacade> {
        self.messages.borrow_mut()
    }

    /// Push an artifact frame without any check, for a frame that is already authorized.
    pub fn push(&self, artifact_type: &str, name: &str, action: &str) {
        self.artifacts
            .borrow_mut()
            .push_with(artifact_type, name, action, AuthInfo::default());
    }

    /// Decide an artifact against the frame below it, then push it. A denied artifact is not pushed.
    ///
    /// `requires_authz` false pushes the frame whatever the records say, so that it can still
    /// inherit. `parameters` feed the `filterMap` of an authorization record.
    ///
    /// # Errors
    ///
    /// Returns an error when the user is not authorized, or a tarpit lock applies.
    pub fn push_checked(
        &self,
        artifact_type: &str,
        name: &str,
        action: &str,
        requires_authz: bool,
        count_tarpit: bool,
        parameters: Option<&std::collections::BTreeMap<String, moqui_lib::object::Value>>,
    ) -> Result<(), Error> {
        let (last, ancestor_deny, authz_disabled) = self.stack_authz_state();
        // The decision reads entities, which push their own frames, so no borrow is held here.
        let (permitted, info) = crate::user::is_permitted(
            artifact_type,
            name,
            action,
            &crate::user::Permit {
                requires_authz,
                authz_disabled,
                count_tarpit,
                parameters,
            },
            last.as_ref(),
            ancestor_deny,
        )?;
        if !permitted {
            tracing::warn!(stack = %self.artifacts.borrow().names(), "artifact authorization failed");
            return Err(crate::user::denied(artifact_type, name, action, &info));
        }
        self.artifacts.borrow_mut().push_with(artifact_type, name, action, info);
        Ok(())
    }

    /// The frame below, any deny on the stack, and whether authorization is disabled.
    pub(crate) fn stack_authz_state(&self) -> (Option<AuthInfo>, bool, bool) {
        let artifacts = self.artifacts.borrow();
        let (last, deny) = artifacts.authz_state();
        (last, deny, artifacts.authz_disabled())
    }

    /// Auth info of the top frame.
    pub(crate) fn top_authz_info(&self) -> Option<AuthInfo> {
        self.artifacts.borrow().authz_state().0
    }

    /// Mark the top frame inheritable for an anonymous user: all actions, or view only.
    pub fn set_anonymous_authorized(&self, all_actions: bool, user_id: &str) {
        if let Some(info) = self.artifacts.borrow_mut().top_info_mut() {
            if all_actions {
                info.anonymous_all(user_id);
            } else {
                info.anonymous_view(user_id);
            }
        }
    }

    pub fn pop(&self) -> Result<(), Error> {
        self.artifacts.borrow_mut().pop()
    }

    pub fn disable_authz(&self) -> AuthzGuard {
        self.artifacts.borrow().disable_authz()
    }

    pub fn authz_disabled(&self) -> bool {
        self.artifacts.borrow().authz_disabled()
    }

    /// Make this context current on this thread until the guard drops.
    ///
    /// Nested enter increments a depth and keeps the outermost context. `current` is `Some` while
    /// the depth is positive. The thread-local does not keep the context alive.
    pub fn enter(&self) -> ContextGuard<'_> {
        ENTERED.with(|slot| {
            let (ptr, depth) = slot.get();
            let next = if depth == 0 { std::ptr::from_ref(self) } else { ptr };
            slot.set((next, depth.saturating_add(1)));
        });
        ContextGuard { _scope: PhantomData }
    }

    /// The context entered on this thread, if a guard is still alive.
    pub fn current() -> Option<NonNull<ExecutionContext>> {
        ENTERED.with(|slot| {
            let (ptr, depth) = slot.get();
            if depth == 0 {
                None
            } else {
                NonNull::new(ptr.cast_mut())
            }
        })
    }

    /// Run `body` with the entered context. The reference cannot outlive the callback.
    pub fn with_current<R>(body: impl FnOnce(&ExecutionContext) -> R) -> Option<R> {
        let ptr = Self::current()?;
        // SAFETY: `enter` stored this pointer, and a live `ContextGuard` borrows that context.
        // Nested enter keeps the outer pointer, which stays borrowed until its guard drops.
        Some(body(unsafe { ptr.as_ref() }))
    }
}

/// Clears the thread-local context when dropped. Depth falls by one.
#[must_use = "dropping the guard clears the thread-local execution context"]
pub struct ContextGuard<'a> {
    _scope: PhantomData<&'a ExecutionContext>,
}

impl Drop for ContextGuard<'_> {
    fn drop(&mut self) {
        ENTERED.with(|slot| {
            let (ptr, depth) = slot.get();
            if depth <= 1 {
                slot.set((std::ptr::null(), 0));
            } else {
                slot.set((ptr, depth - 1));
            }
        });
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
    getrandom::fill(&mut bytes)
        .map_err(|err| Error::new(format!("could not read random bytes for a secret property: {err}")))?;
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
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|err| {
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|err| {
            Error::new(format!(
                "could not set permissions on runtime environment file {}",
                path.display()
            ))
            .with_source(err)
        })?;
    }
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
impl ExecutionContextFactory {
    pub(crate) fn test_context() -> ExecutionContext {
        Self::test_context_with_conf("")
    }

    /// A test context whose conf text is `conf_text`, for example an `artifact-execution-facade`.
    pub(crate) fn test_context_with_conf(conf_text: &str) -> ExecutionContext {
        let conf = MNode::parse_text("test", "<moqui-conf/>").expect("conf");
        let entity = Arc::new(RwLock::new(None));
        let runtime_dir = PathBuf::from(".");
        let factory = Arc::new(Self {
            runtime_dir: runtime_dir.clone(),
            framework_dir: PathBuf::from("."),
            components: Vec::new(),
            resource: ResourceFacade::new(runtime_dir, std::collections::HashMap::new(), entity.clone()),
            cache: CacheFacade::from_conf(&conf).expect("cache"),
            conf_text: conf_text.to_string(),
            conf_cache: std::sync::OnceLock::new(),
            tarpit_hits: std::sync::Mutex::new(std::collections::HashMap::new()),
            property_names: Vec::new(),
            entity,
            pools: crate::service::ServicePools::idle(),
        });
        factory.new_context()
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
