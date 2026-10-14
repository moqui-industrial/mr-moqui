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
use std::sync::atomic::AtomicBool;
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
    extensions:
        std::sync::Mutex<std::collections::HashMap<std::any::TypeId, std::sync::Arc<dyn std::any::Any + Send + Sync>>>,
    /// Entity facade for `dbresource://`. Empty until the host calls [`Self::set_entity`].
    entity: Arc<RwLock<Option<EntityFacade>>>,
    /// This factory's pool guard. Drops with the factory. Idle has no workers.
    pools: crate::service::ServicePools,
    /// Idle until [`Self::start_request_pool`]. Drop joins the threads.
    requests: crate::service::RequestPool,
    pub(crate) sessions: std::sync::Mutex<crate::user::session::SessionTable>,
    /// True after this factory's cluster heartbeat hook is registered.
    pub(crate) cluster_heartbeat: AtomicBool,
    /// Artifact hit counts and bins, written by the scheduler tick.
    hits: super::hits::ArtifactHits,
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
        let hits = super::hits::ArtifactHits::from_conf(&conf_text);
        let flush_mode = crate::user::session::flush_mode_from_property()?;
        let pools = crate::service::start_pools(&conf_text)?;
        super::locate::remember(&framework_dir, &components);
        tracing::info!(
            runtime = %runtime_dir.display(),
            framework = %framework_dir.display(),
            "Initialized execution context factory"
        );
        let factory = Arc::new(Self {
            runtime_dir,
            framework_dir,
            components,
            resource,
            cache,
            conf_text,
            conf_cache: std::sync::OnceLock::new(),
            tarpit_hits: std::sync::Mutex::new(std::collections::HashMap::new()),
            property_names,
            extensions: std::sync::Mutex::new(std::collections::HashMap::new()),
            entity,
            pools,
            requests: crate::service::RequestPool::idle(),
            sessions: std::sync::Mutex::new(crate::user::session::SessionTable::new(flush_mode)),
            cluster_heartbeat: AtomicBool::new(false),
            hits,
        });
        factory.pools.attach(&factory);
        // The host calls cluster::start after the entity facade is installed.
        install_session_flush(&factory);
        install_hit_flush(&factory);
        Ok(factory)
    }

    /// Start the request workers from `tools.@request-pool-max` and `tools.@request-queue`.
    ///
    /// # Errors
    ///
    /// Returns an error when a worker thread does not start.
    pub fn start_request_pool(&self) -> Result<(), Error> {
        self.requests.start_from_conf(&self.conf_text)
    }

    /// Write dirty `moqui.server.HttpSession` rows. The host calls this on shutdown so an
    /// `eventual` session is on disk before the entity facade drops.
    ///
    /// # Errors
    ///
    /// Returns an error when a session row cannot be written.
    pub fn flush_sessions(&self) -> Result<(), Error> {
        crate::user::session::flush_dirty(self)
    }

    /// Write the waiting artifact hits and every hit bin, including the bins still open. The host
    /// calls this on shutdown, as Java's `destroy` writes the bins.
    ///
    /// # Errors
    ///
    /// Returns an error when the rows cannot be written.
    pub fn flush_artifact_hits(&self) -> Result<(), Error> {
        super::hits::flush(self, true)
    }

    pub(crate) fn artifact_hits(&self) -> &super::hits::ArtifactHits {
        &self.hits
    }

    /// The factory request pool. Idle until [`Self::start_request_pool`].
    pub fn request_pool(&self) -> &crate::service::RequestPool {
        &self.requests
    }

    /// This factory's service worker pool and scheduler.
    pub fn service_pools(&self) -> &crate::service::ServicePools {
        &self.pools
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

    /// State another facade keeps per factory, made on first use. `init` may use the factory but
    /// not ask for another extension of its own type.
    pub fn extension<T: std::any::Any + Send + Sync>(&self, init: impl FnOnce(&Self) -> T) -> Arc<T> {
        let key = std::any::TypeId::of::<T>();
        let existing = self
            .extensions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&key)
            .cloned();
        let value: Arc<dyn std::any::Any + Send + Sync> = match existing {
            Some(value) => value,
            None => {
                // Not under the lock, so init can read other extensions.
                let made: Arc<dyn std::any::Any + Send + Sync> = Arc::new(init(self));
                let mut map = self.extensions.lock().unwrap_or_else(|err| err.into_inner());
                Arc::clone(map.entry(key).or_insert(made))
            }
        };
        match value.downcast::<T>() {
            Ok(typed) => typed,
            Err(_) => unreachable!("an extension is stored under its own type id"),
        }
    }

    /// True when the `artifact-execution` element for `artifact_type` has `flag="true"`.
    pub fn artifact_flag(&self, artifact_type: &str, flag: &str) -> bool {
        self.conf().artifact_flag(artifact_type, flag)
    }

    /// A non-empty `user-facade` child attribute from the conf.
    pub fn user_facade_attr(&self, child: &str, name: &str) -> Option<String> {
        self.conf().user_facade_attr(child, name)
    }

    /// The root URL of a conf `webapp` for a render with no request: Java's `makeWebappHost` with no
    /// web facade. `https://` and the https host and port when `https-enabled` is `true`, else `http://` and
    /// the http ones; an empty host is `localhost`, and the default port is left out. Empty when the
    /// conf has no such webapp.
    pub fn webapp_root_url(&self, webapp_name: &str) -> String {
        let conf = self.conf();
        let attr = |name: &str| {
            conf.webapp_attr(webapp_name, name)
                .map(|raw| moqui_lib::env::expand(&raw).unwrap_or(raw))
                .map(|value| value.trim().to_string())
                .unwrap_or_default()
        };
        if conf.webapp_attr(webapp_name, "name").is_none() {
            return String::new();
        }
        let https = attr("https-enabled") == "true";
        let (scheme, host, port, default_port) = if https {
            ("https", attr("https-host"), attr("https-port"), "443")
        } else {
            ("http", attr("http-host"), attr("http-port"), "80")
        };
        let host = if host.is_empty() { "localhost".to_string() } else { host };
        if port.is_empty() || port == default_port {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}:{port}")
        }
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

    /// Pop the top frame. A tracked frame is counted as an artifact hit, in memory.
    pub fn pop(&self) -> Result<(), Error> {
        let popped = self.artifacts.borrow_mut().pop()?;
        if popped.hit.track {
            self.count_artifact_hit(&super::hits::Hit {
                artifact_type: &popped.artifact_type,
                sub_type: popped.hit.sub_type.as_deref(),
                name: &popped.name,
                parameters: popped.hit.parameters.as_ref(),
                start_ms: popped.started_ms,
                running_ms: popped.running_ms,
                output_size: None,
            });
        }
        Ok(())
    }

    /// Set the hit sub-type of the top frame, Java's `actionDetail`, and the parameters it records.
    /// The parameters are kept only when the type is counted.
    pub(crate) fn set_hit_detail(
        &self,
        sub_type: Option<&str>,
        parameters: Option<&std::collections::BTreeMap<String, moqui_lib::object::Value>>,
    ) {
        let artifact_type = self.artifacts.borrow().top_type();
        let keep = artifact_type.is_some_and(|name| self.factory.hits.counts(&name));
        if let Some(hit) = self.artifacts.borrow_mut().top_hit_mut() {
            hit.sub_type = sub_type.map(str::to_string);
            hit.parameters = parameters.filter(|_| keep).cloned();
        }
    }

    /// Do not count the top frame as a hit, Java's `setTrackArtifactHit(false)`.
    pub(crate) fn untrack_hit(&self) {
        if let Some(hit) = self.artifacts.borrow_mut().top_hit_mut() {
            hit.track = false;
        }
    }

    /// Java's `countArtifactHit`, for a hit that is not a frame of the stack, such as a transition.
    pub(crate) fn count_artifact_hit(&self, hit: &super::hits::Hit<'_>) {
        self.factory.hits.count(hit, || super::hits::HitContext {
            visit_id: crate::user::visit_id().ok().flatten(),
            user_id: crate::user::current_user_id().ok().flatten(),
            errors: self.messages.borrow().errors().to_vec(),
            request_url: crate::web::request_url(),
            referrer_url: crate::web::referrer_url(),
        });
    }

    pub fn disable_authz(&self) -> AuthzGuard {
        self.artifacts.borrow().disable_authz()
    }

    pub fn authz_disabled(&self) -> bool {
        self.artifacts.borrow().authz_disabled()
    }

    /// Make this context current on this thread until the guard drops.
    ///
    /// Entering while another context is current makes this one current, and dropping its guard
    /// brings the other back. `current` is `Some` while any guard is alive. The thread-local does
    /// not keep the context alive.
    pub fn enter(&self) -> ContextGuard<'_> {
        let previous = ENTERED.with(|slot| {
            let previous = slot.get();
            slot.set((std::ptr::from_ref(self), previous.1.saturating_add(1)));
            previous
        });
        ContextGuard {
            previous,
            _scope: PhantomData,
        }
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
        // SAFETY: `enter` stored this pointer, and a live `ContextGuard` borrows that context. A
        // guard restores the previous pointer when it drops, so the pointer here is always the
        // innermost guard's, whose context is borrowed until that guard drops.
        Some(body(unsafe { ptr.as_ref() }))
    }
}

/// Restores the previously entered context, or none, when dropped.
#[must_use = "dropping the guard clears the thread-local execution context"]
pub struct ContextGuard<'a> {
    /// What was current before this guard, put back on drop.
    previous: (*const ExecutionContext, u32),
    // A guard belongs to the thread it was made on: the raw pointer above already makes it !Send.
    _scope: PhantomData<&'a ExecutionContext>,
}

impl Drop for ContextGuard<'_> {
    fn drop(&mut self) {
        ENTERED.with(|slot| slot.set(self.previous));
    }
}

fn assert_shareable<T: Send + Sync>() {}

fn install_session_flush(factory: &Arc<ExecutionContextFactory>) {
    let weak = Arc::downgrade(factory);
    factory.pools.add_tick_hook(move || {
        let Some(factory) = weak.upgrade() else {
            return;
        };
        if let Err(err) = crate::user::session::flush_dirty(&factory) {
            tracing::warn!("session flush failed: {err}");
        }
    });
}

/// The artifact hit flush is reserve-connection work on the scheduler tick, as the session flush.
fn install_hit_flush(factory: &Arc<ExecutionContextFactory>) {
    let weak = Arc::downgrade(factory);
    factory.pools.add_tick_hook(move || {
        let Some(factory) = weak.upgrade() else {
            return;
        };
        if let Err(err) = super::hits::flush(&factory, false) {
            tracing::warn!("artifact hit flush failed: {err}");
        }
    });
}

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
    let mut cert_slot = None;
    let mut key_slot = None;
    for node in conf_node.children_named("default-property") {
        let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) else {
            continue;
        };
        names.push(name.clone());
        if name == "cluster_tls_cert" || name == "cluster_tls_key" {
            let slot = read_tls_slot(&node, &name)?;
            if name == "cluster_tls_cert" {
                cert_slot = Some(slot);
            } else {
                key_slot = Some(slot);
            }
            continue;
        }
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
    install_tls_pair(&mut generated, &mut set_by_boot, cert_slot, key_slot)?;
    append_env_secrets(env_path, &generated)?;
    Ok((names, set_by_boot))
}

struct TlsSlot {
    from_env: bool,
    value: String,
    secret: bool,
}

fn read_tls_slot(node: &MNode, name: &str) -> Result<TlsSlot, Error> {
    let secret = is_secret(name, node.attribute("is-secret").as_deref());
    let resolved = env::get_prop_or_env(name);
    if !resolved.is_empty() {
        return Ok(TlsSlot {
            from_env: true,
            value: resolved,
            secret,
        });
    }
    let raw = node.attribute("value").unwrap_or_default();
    let value = if raw.is_empty() {
        String::new()
    } else {
        env::expand(&raw)?
    };
    Ok(TlsSlot {
        from_env: false,
        value,
        secret,
    })
}

fn install_tls_pair(
    generated: &mut Vec<(String, String)>,
    set_by_boot: &mut PropertyRollback,
    cert: Option<TlsSlot>,
    key: Option<TlsSlot>,
) -> Result<(), Error> {
    let (cert, key) = match (cert, key) {
        (Some(cert), Some(key)) => (cert, key),
        (None, None) => return Ok(()),
        _ => return Err(Error::new("cluster_tls_cert and cluster_tls_key must be set together")),
    };
    let cert_placeholder = cert.value.ends_with("CHANGEME");
    let key_placeholder = key.value.ends_with("CHANGEME");
    if cert_placeholder && key_placeholder {
        let (cert_b64, key_b64) = crate::cluster::generate_cert_pair()?;
        remember_generated(generated, set_by_boot, "cluster_tls_cert", cert_b64);
        remember_generated(generated, set_by_boot, "cluster_tls_key", key_b64);
        return Ok(());
    }
    if cert.value.is_empty() && key.value.is_empty() {
        return Ok(());
    }
    if cert_placeholder || key_placeholder || cert.value.is_empty() || key.value.is_empty() {
        return Err(Error::new("cluster_tls_cert and cluster_tls_key must be set together"));
    }
    apply_configured(set_by_boot, "cluster_tls_cert", &cert);
    apply_configured(set_by_boot, "cluster_tls_key", &key);
    Ok(())
}

fn remember_generated(
    generated: &mut Vec<(String, String)>,
    set_by_boot: &mut PropertyRollback,
    name: &str,
    value: String,
) {
    env::set_property(name, value.clone());
    set_by_boot.remember(name);
    generated.push((name.to_string(), value));
    tracing::info!("Generated secret property {name}");
}

fn apply_configured(set_by_boot: &mut PropertyRollback, name: &str, slot: &TlsSlot) {
    log_property(name, &slot.value, slot.secret, !slot.from_env);
    if slot.from_env {
        return;
    }
    env::set_property(name, slot.value.clone());
    set_by_boot.remember(name);
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
    name == "entity_ds_crypt_pass_old"
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

pub(crate) fn standard_base64(bytes: &[u8]) -> String {
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

pub(crate) fn append_env_secrets(path: &Path, pairs: &[(String, String)]) -> Result<(), Error> {
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
        Self::test_context_with(conf_text, crate::service::ServicePools::idle())
    }

    /// A test context whose factory owns `pools`, attached so the scheduler can reach the factory.
    pub(crate) fn test_context_with(conf_text: &str, pools: crate::service::ServicePools) -> ExecutionContext {
        Self::test_context_in(conf_text, pools, PathBuf::from("."))
    }

    /// A test context whose framework directory is `framework_dir`, so framework files resolve.
    pub(crate) fn test_context_in(
        conf_text: &str,
        pools: crate::service::ServicePools,
        framework_dir: PathBuf,
    ) -> ExecutionContext {
        let conf = MNode::parse_text("test", "<moqui-conf/>").expect("conf");
        let entity = Arc::new(RwLock::new(None));
        let runtime_dir = PathBuf::from(".");
        let factory = Arc::new(Self {
            runtime_dir: runtime_dir.clone(),
            framework_dir,
            components: Vec::new(),
            resource: ResourceFacade::new(runtime_dir, std::collections::HashMap::new(), entity.clone()),
            cache: CacheFacade::from_conf(&conf).expect("cache"),
            conf_text: conf_text.to_string(),
            conf_cache: std::sync::OnceLock::new(),
            tarpit_hits: std::sync::Mutex::new(std::collections::HashMap::new()),
            property_names: Vec::new(),
            extensions: std::sync::Mutex::new(std::collections::HashMap::new()),
            entity,
            pools,
            requests: crate::service::RequestPool::idle(),
            sessions: std::sync::Mutex::new(crate::user::session::SessionTable::new(
                crate::user::session::FlushMode::Eventual,
            )),
            cluster_heartbeat: AtomicBool::new(false),
            hits: super::hits::ArtifactHits::from_conf(conf_text),
        });
        factory.pools.attach(&factory);
        install_hit_flush(&factory);
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
