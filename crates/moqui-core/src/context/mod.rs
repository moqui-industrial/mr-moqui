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

//! Factory, conf, components, resources, cache, messages, and the artifact stack.

mod artifact;
mod cache;
mod component;
mod conf;
mod confcache;
mod error;
mod factory;
mod hits;
mod locate;
mod message;
mod resource;
pub mod status;

pub use artifact::AuthzGuard;
pub use cache::CacheFacade;
pub use component::ComponentInfo;
pub use conf::{load_merged_conf, merge_conf};
pub use error::{Error, ErrorKind};
#[cfg(feature = "cluster-tls")]
pub(crate) use factory::standard_base64;
pub use factory::{BootOptions, ContextGuard, ExecutionContext, ExecutionContextFactory, Executor};
pub(crate) use hits::Hit;
pub use locate::{framework_dir, locate};
pub use message::{MessageFacade, ValidationError};
pub use resource::{ResourceFacade, ResourceRef};

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use moqui_lib::env;
    use moqui_lib::node::MNode;

    use super::ExecutionContextFactory;

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    struct EnvGuard {
        root: PathBuf,
    }

    impl EnvGuard {
        fn new(root: PathBuf) -> Self {
            Self { root }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for relative in ["framework/conf/MoquiDefaultConf.xml", "runtime/conf/MoquiDevConf.xml"] {
                let Ok(node) = MNode::parse_file(self.root.join(relative)) else {
                    continue;
                };
                for property in node.children_named("default-property") {
                    if let Some(name) = property.attribute("name") {
                        env::clear_property(&name);
                    }
                }
            }
            env::clear_property("phase2_probe_secret");
            env::clear_property("phase2_probe_unwritable");
            env::clear_property("moqui_version");
            env::clear_property("moqui.version");
        }
    }

    struct TempRuntime(PathBuf);

    impl TempRuntime {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("moqui-phase2-{label}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(path.join("conf")).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRuntime {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn repo_env_stamp(root: &Path) -> Option<(u64, SystemTime)> {
        let meta = std::fs::metadata(root.join("runtime/.env")).ok()?;
        Some((meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
    }

    #[test]
    fn dev_conf_merge_sets_purpose_and_entity_definition_idle() {
        let root = workspace_root();
        let base = MNode::parse_file(root.join("framework/conf/MoquiDefaultConf.xml")).unwrap();
        let over = MNode::parse_file(root.join("runtime/conf/MoquiDevConf.xml")).unwrap();
        super::merge_conf(&base, &over);
        let purpose = base
            .first_named_attrs("default-property", &[("name", "instance_purpose")])
            .unwrap();
        assert_eq!(purpose.attribute("value").as_deref(), Some("dev"));
        let locale = base
            .first_named_attrs("default-property", &[("name", "default_locale")])
            .unwrap();
        assert_eq!(locale.attribute("value").as_deref(), Some("en_US"));
        let entity = base
            .first_named("cache-list")
            .unwrap()
            .first_named_attrs("cache", &[("name", "entity.definition")])
            .unwrap();
        assert_eq!(entity.attribute("expire-time-idle").as_deref(), Some("30"));
        assert!(entity.attribute("value-type").is_some());
    }

    #[test]
    fn boot_registers_components_and_named_resource_errors() {
        let _lock = crate::user::session::cookie_property_lock();
        let root = workspace_root();
        let _env = EnvGuard::new(root.clone());
        let before = repo_env_stamp(&root);
        let temp = TempRuntime::new("boot");
        std::fs::copy(
            root.join("runtime/conf/MoquiDevConf.xml"),
            temp.path().join("conf/MoquiDevConf.xml"),
        )
        .unwrap();
        std::os::unix::fs::symlink(root.join("runtime/base-component"), temp.path().join("base-component")).unwrap();
        env::set_property("moqui_version", "phase2-test");
        let framework = root.join("framework");
        let factory = ExecutionContextFactory::boot_with_framework(temp.path(), &framework).unwrap();
        let names: Vec<&str> = factory.components().iter().map(|info| info.name.as_str()).collect();
        assert_eq!(names, vec!["framework", "tools", "webroot"]);
        for info in factory.components() {
            assert_eq!(info.version, "phase2-test");
        }
        let tools = factory.resource().get("component://tools/screen/Tools.xml").unwrap();
        assert!(tools.exists());
        let screen = factory.resource().get("component://tools/screen").unwrap();
        assert!(!screen.children().unwrap().is_empty());
        assert!(screen.find_child_file("Tools.xml").exists());
        assert!(factory
            .resource()
            .get("runtime://conf/MoquiDevConf.xml")
            .unwrap()
            .exists());
        let caches = factory.cache().names().unwrap();
        assert!(caches.iter().any(|name| name == "entity.definition"));
        assert!(caches.iter().any(|name| name == "service.location"));
        let content = factory.resource().get("content://main/x").unwrap_err();
        assert!(content.to_string().contains("content://main/x"));
        // An http location resolves without a connection; it is fetched when read.
        factory.resource().get("https://example.com/").expect("https resolves");
        let database = factory.resource().get("dbresource://x").unwrap_err();
        assert!(database.to_string().contains("not available"));
        let unknown = factory.resource().get("ftp://files.example/a").unwrap_err();
        assert!(unknown.to_string().contains("ftp"));
        assert_eq!(repo_env_stamp(&root), before);
    }

    #[test]
    fn generated_secret_is_stable_and_unwritable_env_fails() {
        let _lock = crate::user::session::cookie_property_lock();
        let root = workspace_root();
        let _env = EnvGuard::new(root.clone());
        let before = repo_env_stamp(&root);
        let temp = TempRuntime::new("secret");
        write_probe_conf(temp.path(), "phase2_probe_secret", "probe-secret-CHANGEME");
        let framework = root.join("framework");
        let factory = ExecutionContextFactory::boot_with_framework(temp.path(), &framework).unwrap();
        let first = env::get_prop_or_env("phase2_probe_secret");
        assert_eq!(first.len(), 44);
        assert!(!first.ends_with("CHANGEME"));
        assert!(!first.contains("probe-secret"));
        let env_text = std::fs::read_to_string(temp.path().join(".env")).unwrap();
        assert!(env_text.contains("phase2_probe_secret="));
        assert!(!env_text.contains("probe-secret-CHANGEME"));
        assert!(!env_text.contains("entity_ds_crypt_pass_old="));
        assert_eq!(
            env_text
                .lines()
                .filter(|line| line.starts_with("phase2_probe_secret="))
                .count(),
            1
        );
        for name in factory.property_names() {
            env::clear_property(name);
        }
        let again = ExecutionContextFactory::boot_with_framework(temp.path(), &framework).unwrap();
        assert_eq!(env::get_prop_or_env("phase2_probe_secret"), first);
        let env_again = std::fs::read_to_string(temp.path().join(".env")).unwrap();
        assert_eq!(
            env_again
                .lines()
                .filter(|line| line.starts_with("phase2_probe_secret="))
                .count(),
            1
        );
        for name in again.property_names() {
            env::clear_property(name);
        }

        let blocked = TempRuntime::new("blocked");
        write_probe_conf(blocked.path(), "phase2_probe_unwritable", "probe-secret-CHANGEME");
        std::fs::create_dir(blocked.path().join(".env")).unwrap();
        let err = match ExecutionContextFactory::boot_with_framework(blocked.path(), &framework) {
            Ok(_) => panic!("boot succeeded with an unwritable runtime environment file"),
            Err(err) => err,
        };
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("probe-secret-CHANGEME"));
        assert!(!shown.contains("MoquiDefaultPassword"));
        assert!(!shown.contains("CHANGEME"));
        assert!(shown.contains("directory"));
        assert_eq!(repo_env_stamp(&root), before);
    }

    fn write_probe_conf(runtime: &Path, name: &str, value: &str) {
        let text = format!(
            "<moqui-conf><default-property name=\"{name}\" value=\"{value}\" is-secret=\"true\"/></moqui-conf>\n"
        );
        std::fs::write(runtime.join("conf/MoquiDevConf.xml"), text).unwrap();
    }

    #[test]
    fn artifact_stack_allows_and_authz_depth_follows_the_guard() {
        let _lock = crate::user::session::cookie_property_lock();
        let root = workspace_root();
        let _env = EnvGuard::new(root.clone());
        let before = repo_env_stamp(&root);
        let temp = TempRuntime::new("artifact");
        std::fs::copy(
            root.join("runtime/conf/MoquiDevConf.xml"),
            temp.path().join("conf/MoquiDevConf.xml"),
        )
        .unwrap();
        std::os::unix::fs::symlink(root.join("runtime/base-component"), temp.path().join("base-component")).unwrap();
        let factory = ExecutionContextFactory::boot_with_framework(temp.path(), root.join("framework")).unwrap();
        let context = factory.new_context();
        assert!(!context.authz_disabled());
        context
            .push_checked("AT_XML_SCREEN", "tools", "view", true, false, None)
            .unwrap();
        {
            let guard = context.disable_authz();
            assert!(context.authz_disabled());
            context.push("AT_SERVICE", "nested", "run");
            context.pop().unwrap();
            drop(guard);
        }
        assert!(!context.authz_disabled());
        context.pop().unwrap();
        assert!(context.pop().is_err());

        context.message().add_message("hello");
        context.message().add_error("bad");
        assert_eq!(context.message().messages(), &["hello".to_string()]);
        assert_eq!(context.message().errors(), &["bad".to_string()]);
        let other = factory.new_context();
        assert!(other.message().messages().is_empty());
        context.message().clear();
        assert!(context.message().messages().is_empty());
        assert!(context.message().errors().is_empty());
        assert_eq!(repo_env_stamp(&root), before);
    }
}
