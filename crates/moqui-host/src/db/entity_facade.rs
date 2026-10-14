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

//! Register one entity facade. The driver is the host's. Core does not open Postgres.

use std::path::Path;
use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::driver::DbDriver;
use moqui_core::entity::EntityFacade;
use moqui_core::transaction::PoolConfig;
use moqui_lib::error::Error;

/// Load the framework model onto the `transactional` group using the `postgres` dialect.
pub fn open_entity_facade(
    framework_dir: &Path,
    driver: Arc<dyn DbDriver>,
    pool: PoolConfig,
) -> Result<EntityFacade, Error> {
    open_entity_facade_dialect(framework_dir, driver, pool, "postgres")
}

/// Load the framework model onto the `transactional` group using `dialect`.
///
/// `dialect` is a `database-list` name such as `postgres` or `mysql`.
///
/// # Errors
///
/// Returns an error when the conf or the entity model cannot be read, or when `dialect` is not defined.
pub fn open_entity_facade_dialect(
    framework_dir: &Path,
    driver: Arc<dyn DbDriver>,
    pool: PoolConfig,
    dialect: &str,
) -> Result<EntityFacade, Error> {
    EntityFacade::open_with_driver(framework_dir, driver, pool, dialect, &["transactional"])
}

/// [`open_entity_facade_dialect`] with the `entity` directories of the components
/// ([`component_entity_dirs`]): their entities, extends, and entity ECA rules join the framework's.
///
/// # Errors
///
/// Returns an error when the conf or the entity model cannot be read, or when `dialect` is not defined.
pub fn open_entity_facade_components(
    framework_dir: &Path,
    component_entity_dirs: &[std::path::PathBuf],
    driver: Arc<dyn DbDriver>,
    pool: PoolConfig,
    dialect: &str,
) -> Result<EntityFacade, Error> {
    EntityFacade::open_with_driver_components(
        framework_dir,
        component_entity_dirs,
        driver,
        pool,
        dialect,
        &["transactional"],
    )
}

/// The `entity` directory of each component the factory found, in its load order, the framework
/// left out (its directory is always read).
pub fn component_entity_dirs(factory: &ExecutionContextFactory) -> Vec<std::path::PathBuf> {
    let framework = std::fs::canonicalize(factory.framework_dir()).unwrap_or_else(|_| factory.framework_dir().to_path_buf());
    factory
        .components()
        .iter()
        .filter(|info| std::fs::canonicalize(&info.location).unwrap_or_else(|_| info.location.clone()) != framework)
        .map(|info| info.location.join("entity"))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Install the facade so `dbresource://` reads and writes through it.
pub fn attach(factory: &ExecutionContextFactory, facade: EntityFacade) {
    factory.set_entity(facade);
}

#[cfg(all(test, unix, feature = "embedded-postgres", feature = "postgres"))]
mod tests {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use moqui_core::driver::DbDriver;
    use moqui_core::transaction::PoolConfig;
    use moqui_lib::object::Value;

    use super::super::embedded::EmbeddedCluster;
    use super::open_entity_facade;

    struct DirGuard(PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn empty_database_check_create_view_matches_and_stream() {
        let dir = temp_cluster_dir();
        let _guard = DirGuard(dir.clone());
        provide_libxml2_soname(&dir);
        let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
        let driver: Arc<dyn DbDriver> = Arc::new(driver);
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let facade = open_entity_facade(
            &framework,
            driver,
            PoolConfig {
                min: 0,
                max: 4,
                reserve: 1,
                acquire_timeout: Duration::from_secs(30),
                ..Default::default()
            },
        )
        .unwrap();

        for name in [
            "moqui.test.TestEntity",
            "moqui.test.Foo",
            "moqui.test.Bar",
            "moqui.test.TestFts",
        ] {
            assert!(!facade.table_exists(name).unwrap(), "{name} already exists");
        }
        facade
            .check_tables(&[
                "moqui.test.TestEntity",
                "moqui.test.Foo",
                "moqui.test.Bar",
                "moqui.test.TestFts",
                "moqui.test.FooBar",
            ])
            .unwrap();
        for name in [
            "moqui.test.TestEntity",
            "moqui.test.Foo",
            "moqui.test.Bar",
            "moqui.test.TestFts",
        ] {
            assert!(facade.table_exists(name).unwrap(), "{name} was not created");
        }
        assert!(!facade.table_exists("moqui.test.FooBar").unwrap());

        let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
        row.set("testId", Value::string("T1"));
        row.set("testMedium", Value::string("hello"));
        row.create(&facade).unwrap();
        let found = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("T1"))
            .one()
            .unwrap()
            .unwrap();
        assert_eq!(found.get("testMedium").and_then(Value::as_str), Some("hello"));

        let mut foo = facade.make_value("moqui.test.Foo").unwrap();
        foo.set("fooId", Value::string("F1"));
        foo.set("fooText", Value::string("alpha"));
        foo.create(&facade).unwrap();
        let mut bar = facade.make_value("moqui.test.Bar").unwrap();
        bar.set("barId", Value::string("B1"));
        bar.set("fooId", Value::string("F1"));
        bar.set("barRank", Value::I64(3));
        bar.create(&facade).unwrap();
        let view = facade.find("moqui.test.FooBar").unwrap().list().unwrap();
        let joined = view.get(0).unwrap();
        assert_eq!(joined.get("fooText").and_then(Value::as_str), Some("alpha"));
        assert_eq!(joined.get("barRank"), Some(&Value::I64(3)));

        let mut fts = facade.make_value("moqui.test.TestFts").unwrap();
        fts.set("testFtsId", Value::string("FTS1"));
        fts.set("body", Value::string("the quick brown fox jumps over the lazy dog"));
        fts.create(&facade).unwrap();
        let matched = facade
            .find("moqui.test.TestFts")
            .unwrap()
            .matches("body", Value::string("quick brown"))
            .list()
            .unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(
            matched.get(0).unwrap().get("testFtsId").and_then(Value::as_str),
            Some("FTS1")
        );

        facade.begin("moqui.test.TestEntity", None).unwrap();
        for index in 1..=60 {
            let mut streamed = facade.make_value("moqui.test.TestEntity").unwrap();
            streamed.set("testId", Value::string(format!("s-{index:03}")));
            streamed.set("testMedium", Value::string("stream"));
            streamed.create(&facade).unwrap();
        }
        let mut iter = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testMedium", Value::string("stream"))
            .iterator()
            .unwrap();
        let mut seen = 0_i64;
        while iter.next_value().unwrap().is_some() {
            seen += 1;
        }
        assert_eq!(seen, 60);
        drop(iter);
        let counted = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testMedium", Value::string("stream"))
            .count()
            .unwrap();
        assert_eq!(counted, 60);
        facade.commit("moqui.test.TestEntity").unwrap();

        facade.begin("moqui.test.TestEntity", None).unwrap();
        let committed = stream_ids(&facade);
        assert_eq!(committed.len(), 60, "{committed:?}");
        assert!(committed.iter().any(|id| id == "s-001"));
        let mut doomed = facade.make_value("moqui.test.TestEntity").unwrap();
        doomed.set("testId", Value::string("s-001"));
        doomed.delete(&facade).unwrap();
        let held = stream_ids(&facade);
        assert_eq!(held.len(), 59, "{held:?}");
        assert!(held.iter().all(|id| id != "s-001"));
        facade.rollback("moqui.test.TestEntity").unwrap();

        facade.begin("moqui.test.TestEntity", None).unwrap();
        let restored = stream_ids(&facade);
        assert_eq!(restored.len(), 60, "{restored:?}");
        assert!(restored.iter().any(|id| id == "s-001"));
        drop(restored);
        facade.rollback("moqui.test.TestEntity").unwrap();

        drop(facade);
        drop(cluster);
    }

    /// Every framework view-entity runs a paged select and a count on Postgres, with every
    /// framework table and foreign key index in place.
    #[test]
    fn every_framework_view_selects_and_counts_on_postgres() {
        let dir = temp_cluster_dir();
        let _guard = DirGuard(dir.clone());
        provide_libxml2_soname(&dir);
        let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
        let driver: Arc<dyn DbDriver> = Arc::new(driver);
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let transactions = moqui_core::transaction::TransactionManager::new(
            driver,
            PoolConfig {
                min: 0,
                max: 4,
                reserve: 1,
                acquire_timeout: Duration::from_secs(30),
                ..Default::default()
            },
        )
        .unwrap();
        let facade = moqui_core::entity::EntityFacade::open(
            &framework,
            "transactional",
            vec![
                moqui_core::entity::EntityGroup::new("transactional", "postgres", transactions.clone()),
                moqui_core::entity::EntityGroup::new("logging", "postgres", transactions),
            ],
        )
        .unwrap();
        facade.check_all_tables().unwrap();

        let mut views = Vec::new();
        let mut paths: Vec<PathBuf> = std::fs::read_dir(framework.join("entity"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "xml"))
            .collect();
        paths.sort();
        for path in paths {
            let root = moqui_lib::node::MNode::parse_file(&path).unwrap();
            for view in root.children_named("view-entity") {
                let package = view.attribute("package").unwrap_or_default();
                let name = view.attribute("entity-name").unwrap_or_default();
                views.push(format!("{package}.{name}"));
            }
        }
        assert!(views.len() > 20, "{views:?}");
        let mut failures = Vec::new();
        for view in &views {
            let listed = facade.find(view).and_then(|find| find.limit(1).list());
            if let Err(err) = listed {
                failures.push(format!("{view} select: {err}"));
            }
            let counted = facade.find(view).and_then(|find| find.count());
            if let Err(err) = counted {
                failures.push(format!("{view} count: {err}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));

        drop(facade);
        drop(cluster);
    }

    /// A held update of a row the outer frame locked stays with that frame. A force-new frame's commit
    /// flushed it once, on the inner connection, where it waited on the outer frame's own row lock.
    #[test]
    fn a_held_update_of_a_locked_row_does_not_block_an_inner_force_new_on_postgres() {
        use moqui_core::transaction::TransactionAttribute;

        let dir = temp_cluster_dir();
        let _guard = DirGuard(dir.clone());
        provide_libxml2_soname(&dir);
        let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
        let driver: Arc<dyn DbDriver> = Arc::new(driver);
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let facade = open_entity_facade(
            &framework,
            driver,
            PoolConfig {
                min: 0,
                max: 4,
                reserve: 1,
                acquire_timeout: Duration::from_secs(30),
                ..Default::default()
            },
        )
        .unwrap();
        facade
            .check_tables(&["moqui.test.TestEntity", "moqui.test.Foo"])
            .unwrap();
        let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
        row.set("testId", Value::string("R"));
        row.set("testMedium", Value::string("before"));
        row.create(&facade).unwrap();

        let tx = facade.transactions_for("moqui.test.TestEntity").unwrap();
        // A lock wait ends at the deadline instead of hanging the test.
        let deadline = Some(Duration::from_secs(5));
        tx.run(TransactionAttribute::Cache, deadline, || {
            let mut locked = facade
                .find("moqui.test.TestEntity")?
                .eq("testId", Value::string("R"))
                .for_update(true)
                .one()?
                .expect("R");
            locked.set("testMedium", Value::string("held"));
            locked.update(&facade)?;
            tx.run(TransactionAttribute::ForceNew, deadline, || {
                let mut foo = facade.make_value("moqui.test.Foo")?;
                foo.set("fooId", Value::string("F1"));
                foo.create(&facade)
            })
        })
        .unwrap();

        let found = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("R"))
            .one()
            .unwrap()
            .unwrap();
        assert_eq!(found.get("testMedium").and_then(Value::as_str), Some("held"));
        let foo = facade
            .find("moqui.test.Foo")
            .unwrap()
            .eq("fooId", Value::string("F1"))
            .one()
            .unwrap();
        assert!(foo.is_some());

        drop(facade);
        drop(cluster);
    }

    fn stream_ids(facade: &moqui_core::entity::EntityFacade) -> Vec<String> {
        let mut iter = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testMedium", Value::string("stream"))
            .iterator()
            .unwrap();
        let mut ids = Vec::new();
        while let Some(row) = iter.next_value().unwrap() {
            if let Some(id) = row.get("testId").and_then(Value::as_str) {
                ids.push(id.to_string());
            }
        }
        ids
    }

    fn temp_cluster_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let name = format!("moqui-pg-{}-{nanos}", std::process::id());
        if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp").join(name)
        } else {
            std::env::temp_dir().join(name)
        }
    }

    fn provide_libxml2_soname(dir: &Path) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let cache = ldconfig_cache();
            if cache.lines().any(|line| ldconfig_soname(line) == "libxml2.so.2") {
                return;
            }
            let Some(host) = libxml2_16_path(&cache) else {
                return;
            };
            compile_libxml2_shim(dir, &host);
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        let _ = dir;
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn ldconfig_cache() -> String {
        std::process::Command::new("ldconfig")
            .arg("-p")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn ldconfig_soname(line: &str) -> &str {
        line.split_whitespace().next().unwrap_or("")
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn libxml2_16_path(cache: &str) -> Option<PathBuf> {
        for line in cache.lines() {
            if ldconfig_soname(line) == "libxml2.so.16" && line.contains("x86-64") {
                let path = PathBuf::from(line.split("=>").nth(1)?.trim());
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn compile_libxml2_shim(dir: &Path, host: &Path) {
        let shim_dir = dir.join("xmlshim");
        std::fs::create_dir_all(&shim_dir).unwrap();
        let assembly = shim_dir.join("shim.s");
        let map = shim_dir.join("ver.map");
        let shared = shim_dir.join("libxml2.so.2");
        std::fs::write(&assembly, shim_assembly()).unwrap();
        std::fs::write(&map, version_script()).unwrap();
        let output = std::process::Command::new("gcc")
            .args(["-shared", "-fPIC", "-o"])
            .arg(&shared)
            .arg(&assembly)
            .arg("-Wl,-soname,libxml2.so.2")
            .arg(format!("-Wl,--version-script={}", map.display()))
            .arg(host)
            .output()
            .expect("gcc");
        assert!(
            output.status.success(),
            "libxml2 shim: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let shim = shim_dir.display().to_string();
        let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let library_path = if existing.split(':').any(|part| part == shim) {
            existing
        } else if existing.is_empty() {
            shim
        } else {
            format!("{shim}:{existing}")
        };
        // SAFETY: initdb and the postmaster inherit this. The test writes it before those spawn.
        unsafe { std::env::set_var("LD_LIBRARY_PATH", library_path) };
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn shim_assembly() -> String {
        let mut out = String::new();
        out.push_str(".text\n");
        for (index, (name, version)) in XML_SYMBOLS.iter().enumerate() {
            writeln!(
                out,
                ".globl shim_{index}\n.type shim_{index}, @function\nshim_{index}:\n    jmp {name}@PLT\n.size shim_{index}, .-shim_{index}\n.symver shim_{index}, {name}@{version}"
            )
            .unwrap();
        }
        out
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn version_script() -> String {
        let mut out = String::new();
        for (index, version) in XML_VERSIONS.iter().enumerate() {
            out.push_str(version);
            out.push_str(" {\n  global:\n");
            for (name, symbol_version) in XML_SYMBOLS {
                if symbol_version == version {
                    writeln!(out, "    {name};").unwrap();
                }
            }
            out.push_str("  local: *;\n}");
            if index == 0 {
                out.push_str(";\n");
            } else {
                writeln!(out, " {};", XML_VERSIONS[index - 1]).unwrap();
            }
        }
        out
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const XML_VERSIONS: &[&str] = &[
        "LIBXML2_2.4.30",
        "LIBXML2_2.6.0",
        "LIBXML2_2.6.5",
        "LIBXML2_2.6.8",
        "LIBXML2_2.6.23",
    ];

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const XML_SYMBOLS: &[(&str, &str)] = &[
        ("xmlAddChildList", "LIBXML2_2.4.30"),
        ("xmlBufferContent", "LIBXML2_2.4.30"),
        ("xmlBufferCreate", "LIBXML2_2.4.30"),
        ("xmlBufferFree", "LIBXML2_2.4.30"),
        ("xmlBufferLength", "LIBXML2_2.4.30"),
        ("xmlCharInRange", "LIBXML2_2.6.0"),
        ("xmlCheckVersion", "LIBXML2_2.4.30"),
        ("xmlCopyNode", "LIBXML2_2.4.30"),
        ("xmlCtxtReadDoc", "LIBXML2_2.6.0"),
        ("xmlCtxtReadMemory", "LIBXML2_2.6.0"),
        ("xmlDocSetRootElement", "LIBXML2_2.4.30"),
        ("xmlEncodeSpecialChars", "LIBXML2_2.4.30"),
        ("xmlFreeDoc", "LIBXML2_2.4.30"),
        ("xmlFreeNode", "LIBXML2_2.4.30"),
        ("xmlFreeParserCtxt", "LIBXML2_2.4.30"),
        ("xmlFreeTextWriter", "LIBXML2_2.6.0"),
        ("xmlGetExternalEntityLoader", "LIBXML2_2.4.30"),
        ("xmlGetUTF8Char", "LIBXML2_2.4.30"),
        ("xmlInitParser", "LIBXML2_2.4.30"),
        ("xmlKeepBlanksDefault", "LIBXML2_2.4.30"),
        ("xmlNewDoc", "LIBXML2_2.4.30"),
        ("xmlNewDocText", "LIBXML2_2.4.30"),
        ("xmlNewNode", "LIBXML2_2.4.30"),
        ("xmlNewParserCtxt", "LIBXML2_2.4.30"),
        ("xmlNewStringInputStream", "LIBXML2_2.4.30"),
        ("xmlNewTextWriterMemory", "LIBXML2_2.6.0"),
        ("xmlNodeDump", "LIBXML2_2.4.30"),
        ("xmlParseBalancedChunkMemory", "LIBXML2_2.4.30"),
        ("xmlParserPrintFileContext", "LIBXML2_2.4.30"),
        ("xmlSaveClose", "LIBXML2_2.6.8"),
        ("xmlSaveDoc", "LIBXML2_2.6.8"),
        ("xmlSaveToBuffer", "LIBXML2_2.6.23"),
        ("xmlSaveTree", "LIBXML2_2.6.8"),
        ("xmlSetExternalEntityLoader", "LIBXML2_2.4.30"),
        ("xmlSetGenericErrorFunc", "LIBXML2_2.4.30"),
        ("xmlSetStructuredErrorFunc", "LIBXML2_2.6.0"),
        ("xmlStrchr", "LIBXML2_2.4.30"),
        ("xmlStrcmp", "LIBXML2_2.4.30"),
        ("xmlStrdup", "LIBXML2_2.4.30"),
        ("xmlStrlen", "LIBXML2_2.4.30"),
        ("xmlStrncmp", "LIBXML2_2.4.30"),
        ("xmlStrstr", "LIBXML2_2.4.30"),
        ("xmlTextWriterEndElement", "LIBXML2_2.6.0"),
        ("xmlTextWriterStartElement", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteAttribute", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteBase64", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteBinHex", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteRaw", "LIBXML2_2.6.0"),
        ("xmlXPathCastBooleanToNumber", "LIBXML2_2.4.30"),
        ("xmlXPathCastBooleanToString", "LIBXML2_2.4.30"),
        ("xmlXPathCastNodeSetToString", "LIBXML2_2.4.30"),
        ("xmlXPathCastNodeToString", "LIBXML2_2.4.30"),
        ("xmlXPathCastNumberToString", "LIBXML2_2.4.30"),
        ("xmlXPathCompiledEval", "LIBXML2_2.4.30"),
        ("xmlXPathCtxtCompile", "LIBXML2_2.6.5"),
        ("xmlXPathFreeCompExpr", "LIBXML2_2.4.30"),
        ("xmlXPathFreeContext", "LIBXML2_2.4.30"),
        ("xmlXPathFreeObject", "LIBXML2_2.4.30"),
        ("xmlXPathNewContext", "LIBXML2_2.4.30"),
        ("xmlXPathRegisterNs", "LIBXML2_2.4.30"),
    ];
}
