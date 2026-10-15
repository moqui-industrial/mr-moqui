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

//! Host checks for the entity one-cache, the transaction hold, and the sim overlay.

#[cfg(all(test, unix, feature = "embedded-postgres", feature = "postgres"))]
mod tests {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use moqui_core::driver::{DbDriver, DbRow};
    use moqui_core::entity::cache::{enter_sim, exit_sim};
    use moqui_core::entity::{EntityFacade, EntityValue};
    use moqui_core::transaction::{PoolConfig, TransactionManager};
    use moqui_lib::object::Value;

    use super::super::embedded::EmbeddedCluster;
    use super::super::entity_facade::open_entity_facade;
    use super::super::sim::SimOverlay;

    struct DirGuard(PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn one_cache_hits_and_sim_write_stays_off_main() {
        let main_dir = temp_cluster_dir("main");
        let overlay_dir = temp_cluster_dir("overlay");
        let _main_guard = DirGuard(main_dir.clone());
        let _overlay_guard = DirGuard(overlay_dir.clone());
        provide_libxml2_soname(&main_dir);
        let (main_cluster, main_driver) = EmbeddedCluster::start(&main_dir, 20).unwrap();
        let (overlay_cluster, overlay_driver) = EmbeddedCluster::start(&overlay_dir, 20).unwrap();
        let main_driver: Arc<dyn DbDriver> = Arc::new(main_driver);
        let overlay_driver: Arc<dyn DbDriver> = Arc::new(overlay_driver);
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
        let facade = open_entity_facade(&framework, Arc::clone(&main_driver), test_pool()).unwrap();

        let err = enter_sim(&facade).unwrap_err();
        assert!(!facade.sim_active(), "{err}");
        let sim = TransactionManager::new(overlay_driver, test_pool()).unwrap();
        facade.set_sim_transactions(Some(sim));

        facade
            .check_tables(&["moqui.test.TestEntity", "moqui.basic.EnumerationType"])
            .unwrap();

        assert_one_cache(&facade, &main_driver);
        assert_hold_commit_and_rollback(&facade, &main_driver);
        assert_sim_overlay(&facade, &main_driver);

        drop(facade);
        drop(main_cluster);
        drop(overlay_cluster);
    }

    #[test]
    fn first_enter_sim_starts_the_overlay_and_stop_removes_it() {
        let main_dir = temp_cluster_dir("lazy");
        let _main_guard = DirGuard(main_dir.clone());
        provide_libxml2_soname(&main_dir);
        let (main_cluster, main_driver) = EmbeddedCluster::start(&main_dir, 20).unwrap();
        let main_driver: Arc<dyn DbDriver> = Arc::new(main_driver);
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
        let facade = open_entity_facade(&framework, Arc::clone(&main_driver), test_pool()).unwrap();
        facade.check_tables(&["moqui.test.TestEntity"]).unwrap();

        // A directory left by a process that is gone goes away at install.
        let orphan = main_dir.join("sim-4000000000");
        std::fs::create_dir_all(orphan.join("data")).unwrap();
        let overlay = SimOverlay::install(&facade, &main_dir, Some(main_dir.join("install")));
        assert!(!orphan.exists());
        assert!(!overlay.started());
        assert!(facade.sim_transactions().is_none());

        enter_sim(&facade).unwrap();
        assert!(overlay.started());
        let sim_dir = overlay.data_dir().unwrap();
        assert!(sim_dir.starts_with(main_dir.join(format!("sim-{}", std::process::id()))));
        assert!(
            !sim_dir.parent().unwrap().join("install").exists(),
            "the overlay shares the entity install"
        );
        let settings = facade
            .sim_transactions()
            .unwrap()
            .with_connection(|conn| {
                let mut shown = Vec::new();
                for name in ["synchronous_commit", "shared_buffers", "fsync"] {
                    let rows = conn.query(&format!("SHOW {name}"), &[])?;
                    shown.push(rows[0].columns[0].1.as_str().unwrap_or_default().to_string());
                }
                Ok(shown)
            })
            .unwrap();
        assert_eq!(settings[0], "off");
        assert!(
            settings[1] == "8MB" || settings[1] == "8192kB",
            "shared_buffers={}",
            settings[1]
        );
        assert_eq!(settings[2], "on");

        create_test(&facade, "LAZY1", "sim");
        assert!(find_one(&facade, "moqui.test.TestEntity", "testId", "LAZY1").is_some());
        exit_sim(&facade).unwrap();
        assert!(find_one(&facade, "moqui.test.TestEntity", "testId", "LAZY1").is_none());
        // A second sim reuses the running overlay.
        enter_sim(&facade).unwrap();
        exit_sim(&facade).unwrap();

        drop(facade);
        drop(overlay);
        assert!(!sim_dir.exists(), "{} is still there", sim_dir.display());
        drop(main_cluster);
    }

    fn assert_one_cache(facade: &EntityFacade, driver: &Arc<dyn DbDriver>) {
        let mut row = facade.make_value("moqui.basic.EnumerationType").unwrap();
        row.set("enumTypeId", Value::string("CACHE1"));
        row.set("description", Value::string("cached"));
        row.create(facade).unwrap();
        let found = find_one(facade, "moqui.basic.EnumerationType", "enumTypeId", "CACHE1").unwrap();
        assert_eq!(found.get("description").and_then(Value::as_str), Some("cached"));

        let mut conn = driver.connect().unwrap();
        let deleted = conn
            .execute(
                "DELETE FROM \"EnumerationType\" WHERE \"enumTypeId\" = ?",
                &[Value::string("CACHE1")],
            )
            .unwrap();
        assert_eq!(deleted, 1);
        drop(conn);
        assert!(raw_rows(
            driver,
            "SELECT \"enumTypeId\" FROM \"EnumerationType\" WHERE \"enumTypeId\" = ?",
            "CACHE1"
        )
        .is_empty());

        let cached = find_one(facade, "moqui.basic.EnumerationType", "enumTypeId", "CACHE1").unwrap();
        assert_eq!(cached.get("description").and_then(Value::as_str), Some("cached"));
    }

    fn assert_hold_commit_and_rollback(facade: &EntityFacade, driver: &Arc<dyn DbDriver>) {
        facade.begin("moqui.test.TestEntity", None).unwrap();
        create_test(facade, "COMMIT1", "committed");
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "COMMIT1").is_some());
        assert!(raw_rows(
            driver,
            "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
            "COMMIT1"
        )
        .is_empty());
        facade.commit("moqui.test.TestEntity").unwrap();
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "COMMIT1").is_some());
        assert_eq!(
            raw_rows(
                driver,
                "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
                "COMMIT1"
            )
            .len(),
            1
        );

        facade.begin("moqui.test.TestEntity", None).unwrap();
        create_test(facade, "HOLD1", "held");
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "HOLD1").is_some());
        facade.rollback("moqui.test.TestEntity").unwrap();
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "HOLD1").is_none());
        assert!(raw_rows(
            driver,
            "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
            "HOLD1"
        )
        .is_empty());
    }

    fn assert_sim_overlay(facade: &EntityFacade, driver: &Arc<dyn DbDriver>) {
        create_test(facade, "T1", "main");
        assert_eq!(
            raw_rows(
                driver,
                "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
                "T1"
            )
            .len(),
            1
        );

        enter_sim(facade).unwrap();
        assert!(facade.sim_active());
        create_test(facade, "SIM2", "overlay");
        let sim_row = find_one(facade, "moqui.test.TestEntity", "testId", "SIM2").unwrap();
        assert_eq!(sim_row.get("testMedium").and_then(Value::as_str), Some("overlay"));
        assert!(raw_rows(
            driver,
            "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
            "SIM2"
        )
        .is_empty());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "T1").is_some());

        let mut hidden = facade.make_value("moqui.test.TestEntity").unwrap();
        hidden.set("testId", Value::string("T1"));
        hidden.delete(facade).unwrap();
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "T1").is_none());
        assert_eq!(
            raw_rows(
                driver,
                "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
                "T1"
            )
            .len(),
            1
        );

        exit_sim(facade).unwrap();
        assert!(!facade.sim_active());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "SIM2").is_none());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "T1").is_some());

        enter_sim(facade).unwrap();
        create_test(facade, "SIM-OUTER", "outer");
        enter_sim(facade).unwrap();
        assert!(facade.sim_active());
        create_test(facade, "SIM-INNER", "inner");
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "SIM-INNER").is_some());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "SIM-OUTER").is_none());
        exit_sim(facade).unwrap();
        assert!(facade.sim_active());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "SIM-OUTER").is_some());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "SIM-INNER").is_none());
        exit_sim(facade).unwrap();
        assert!(!facade.sim_active());
        assert!(find_one(facade, "moqui.test.TestEntity", "testId", "SIM-OUTER").is_none());
        assert!(raw_rows(
            driver,
            "SELECT \"testId\" FROM \"TestEntity\" WHERE \"testId\" = ?",
            "SIM-OUTER"
        )
        .is_empty());
    }

    fn create_test(facade: &EntityFacade, id: &str, medium: &str) {
        let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
        row.set("testId", Value::string(id));
        row.set("testMedium", Value::string(medium));
        row.create(facade).unwrap();
    }

    fn test_pool() -> PoolConfig {
        PoolConfig {
            min: 0,
            max: 4,
            reserve: 1,
            acquire_timeout: Duration::from_secs(30),
            ..Default::default()
        }
    }

    fn find_one(facade: &EntityFacade, entity: &str, field: &str, id: &str) -> Option<EntityValue> {
        facade.find(entity).unwrap().eq(field, Value::string(id)).one().unwrap()
    }

    fn raw_rows(driver: &Arc<dyn DbDriver>, sql: &str, id: &str) -> Vec<DbRow> {
        let mut conn = driver.connect().unwrap();
        conn.query(sql, &[Value::string(id)]).unwrap()
    }

    fn temp_cluster_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let name = format!("moqui-pg-{label}-{}-{nanos}", std::process::id());
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
