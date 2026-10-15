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

//! Dynamic view, entity data writer, and `EntityFacade.rest` against embedded Postgres.

#[cfg(all(test, unix, feature = "embedded-postgres", feature = "postgres"))]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use moqui_core::driver::DbDriver;
    use moqui_core::transaction::PoolConfig;
    use moqui_lib::object::Value;

    use super::super::embedded::EmbeddedCluster;
    use super::super::entity_facade::open_entity_facade;

    #[test]
    fn rest_dynamic_view_and_writer() {
        let dir = temp_cluster_dir();
        let _data_guard = DirGuard(dir.clone());
        let mut export_name = dir.file_name().unwrap().to_os_string();
        export_name.push("-export");
        let export = dir.with_file_name(export_name);
        let _export_guard = DirGuard(export.clone());
        std::fs::create_dir_all(&export).unwrap();
        provide_libxml2_soname(&dir);
        let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
        let driver: Arc<dyn DbDriver> = Arc::new(driver);
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
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
            .check_tables(&["moqui.test.TestEntity", "moqui.test.Foo", "moqui.test.Bar"])
            .unwrap();

        let created = facade
            .rest(
                "create",
                "moqui.test.TestEntity",
                &params(vec![
                    ("path", Value::List(vec![Value::string("R1")])),
                    ("testMedium", Value::string("created")),
                ]),
            )
            .unwrap();
        assert_eq!(map_str(&created, "testId"), Some("R1"));
        let found = facade
            .rest(
                "find",
                "moqui.test.TestEntity",
                &params(vec![("path", Value::List(vec![Value::string("R1")]))]),
            )
            .unwrap();
        assert_eq!(map_str(&found, "testMedium"), Some("created"));
        assert_eq!(map_str(&found, "_entity"), Some("moqui.test.TestEntity"));

        facade
            .rest(
                "update",
                "moqui.test.TestEntity",
                &params(vec![
                    ("testId", Value::string("R1")),
                    ("testMedium", Value::string("updated")),
                ]),
            )
            .unwrap();
        let updated = facade
            .rest(
                "find",
                "moqui.test.TestEntity",
                &params(vec![("testId", Value::string("R1"))]),
            )
            .unwrap();
        assert_eq!(map_str(&updated, "testMedium"), Some("updated"));

        facade
            .rest(
                "store",
                "moqui.test.TestEntity",
                &params(vec![
                    ("testId", Value::string("R1")),
                    ("testMedium", Value::string("stored")),
                ]),
            )
            .unwrap();
        let stored = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("R1"))
            .one()
            .unwrap()
            .unwrap();
        assert_eq!(stored.get("testMedium").and_then(Value::as_str), Some("stored"));
        let via_rest = facade
            .rest(
                "find",
                "moqui.test.TestEntity",
                &params(vec![("testId", Value::string("R1"))]),
            )
            .unwrap();
        assert_eq!(map_str(&via_rest, "testMedium"), Some("stored"));
        let err = facade
            .rest(
                "find",
                "moqui.test.TestEntity",
                &params(vec![
                    ("testId", Value::string("R1")),
                    ("master", Value::string("default")),
                ]),
            )
            .unwrap_err();
        assert!(err.to_string().contains("No master definition found"), "{err}");

        facade
            .rest(
                "delete",
                "moqui.test.TestEntity",
                &params(vec![("testId", Value::string("R1"))]),
            )
            .unwrap();
        assert!(facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("R1"))
            .one()
            .unwrap()
            .is_none());
        let err = facade
            .rest(
                "get",
                "moqui.test.TestEntity",
                &params(vec![("testId", Value::string("R1"))]),
            )
            .unwrap_err();
        assert!(err.to_string().contains("No value found"), "{err}");

        for (foo_id, text) in [("F1", "alpha"), ("F2", "beta")] {
            let mut foo = facade.make_value("moqui.test.Foo").unwrap();
            foo.set("fooId", Value::string(foo_id));
            foo.set("fooText", Value::string(text));
            foo.create(&facade).unwrap();
        }
        let mut bar = facade.make_value("moqui.test.Bar").unwrap();
        bar.set("barId", Value::string("B1"));
        bar.set("fooId", Value::string("F1"));
        bar.set("barRank", Value::I64(3));
        bar.create(&facade).unwrap();
        let rows = facade
            .dynamic_view(
                "dynamic.FooJoin",
                &params(vec![
                    (
                        "member-entity",
                        Value::List(vec![
                            value_map(vec![
                                ("entity-alias", Value::string("FOO")),
                                ("entity-name", Value::string("moqui.test.Foo")),
                            ]),
                            value_map(vec![
                                ("entity-alias", Value::string("BAR")),
                                ("entity-name", Value::string("moqui.test.Bar")),
                                ("join-from-alias", Value::string("FOO")),
                                ("key-map", value_map(vec![("fooId", Value::string("fooId"))])),
                            ]),
                        ]),
                    ),
                    (
                        "alias",
                        Value::List(vec![
                            value_map(vec![
                                ("entity-alias", Value::string("FOO")),
                                ("name", Value::string("fooText")),
                                ("field", Value::string("fooText")),
                            ]),
                            value_map(vec![
                                ("entity-alias", Value::string("BAR")),
                                ("name", Value::string("barRank")),
                                ("field", Value::string("barRank")),
                            ]),
                        ]),
                    ),
                ]),
            )
            .unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].get("fooText").and_then(Value::as_str), Some("alpha"));
        assert_eq!(rows[0].get("barRank"), Some(&Value::I64(3)));

        facade
            .write_data(
                "moqui.test.TestEntity",
                &[params(vec![
                    ("testId", Value::string("W1")),
                    ("testMedium", Value::string("written")),
                    ("directory", Value::string(export.display().to_string())),
                ])],
            )
            .unwrap();
        let xml = std::fs::read_to_string(export.join("moqui.test.TestEntity.xml")).unwrap();
        assert!(xml.contains("<entity-facade-xml>"), "{xml}");
        assert!(xml.contains("testId=\"W1\""), "{xml}");
        let csv = std::fs::read_to_string(export.join("moqui.test.TestEntity.csv")).unwrap();
        assert!(csv.starts_with("moqui.test.TestEntity,export,testId\n"), "{csv}");
        let json = std::fs::read_to_string(export.join("moqui.test.TestEntity.json")).unwrap();
        assert!(json.contains("\"_entity\": \"moqui.test.TestEntity\""), "{json}");
        assert!(json.contains("\"testId\": \"W1\""), "{json}");

        drop(facade);
        drop(cluster);
    }

    fn params(entries: Vec<(&str, Value)>) -> BTreeMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn map_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
        let Value::Map(map) = value else {
            return None;
        };
        map.get(key).and_then(Value::as_str)
    }

    fn value_map(entries: Vec<(&str, Value)>) -> Value {
        let stack = moqui_lib::context_stack::ContextStack::new();
        let frame = stack.shared_map();
        {
            let mut map = frame.borrow_mut();
            for (key, value) in entries {
                map.insert(key.to_string(), value);
            }
        }
        drop(stack);
        Value::Map(match Rc::try_unwrap(frame) {
            Ok(cell) => cell.into_inner(),
            Err(frame) => frame.borrow().clone(),
        })
    }

    struct DirGuard(PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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
