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

//! Host checks for the entity data loader, stored ECA rules, and `DataFeedPending`.

#[cfg(all(test, unix, feature = "embedded-postgres", feature = "postgres"))]
mod tests {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use moqui_core::driver::DbDriver;
    use moqui_core::entity::load_feed::{
        eca_rules, held_feed_services, held_services, load_entity_data, match_rules, LoadOptions,
    };
    use moqui_core::entity::EntityFacade;
    use moqui_core::transaction::PoolConfig;
    use moqui_lib::object::Value;

    use super::super::embedded::EmbeddedCluster;
    use super::super::entity_facade::open_entity_facade;

    #[test]
    fn load_data_stores_eca_and_inserts_pending() {
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
            },
        )
        .unwrap();
        facade
            .check_tables(&[
                "moqui.test.TestEntity",
                "moqui.test.Foo",
                "moqui.screen.form.DbForm",
                "moqui.basic.EnumerationType",
                "moqui.basic.Enumeration",
                "moqui.basic.LocalizedEntityField",
                "moqui.entity.document.DataDocument",
                "moqui.entity.feed.DataFeed",
                "moqui.entity.feed.DataFeedDocument",
                "moqui.entity.feed.DataFeedPending",
                "moqui.security.UserGroup",
                "moqui.security.UserGroupPermission",
                "moqui.security.UserGroupMember",
            ])
            .unwrap();

        let fix = dir.join("fixtures");
        std::fs::create_dir_all(&fix).unwrap();
        let xml = fix.join("rows.xml");
        std::fs::write(
            &xml,
            r#"<entity-facade-xml type="seed">
                <moqui.test.TestEntity testId="XML1" testMedium="from-xml"/>
                <create-moqui.test.TestEntity testId="SVC" testMedium="held"/>
            </entity-facade-xml>"#,
        )
        .unwrap();
        let loaded = load_entity_data(&facade, xml.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(loaded.records, 1);
        assert_eq!(
            text_of(&facade, "moqui.test.TestEntity", "testId", "XML1", "testMedium"),
            Some("from-xml".to_string())
        );
        let held = held_services(&facade);
        assert!(held
            .iter()
            .any(|service| service.name == "create#moqui.test.TestEntity"));
        assert!(row_missing(&facade, "moqui.test.TestEntity", "testId", "SVC"));

        let demo = fix.join("demo.xml");
        std::fs::write(
            &demo,
            r#"<entity-facade-xml type="demo"><moqui.test.TestEntity testId="DEMO" testMedium="no"/></entity-facade-xml>"#,
        )
        .unwrap();
        let seed_only = LoadOptions {
            data_types: vec!["seed".to_string()],
            ..LoadOptions::default()
        };
        let skipped = load_entity_data(&facade, demo.to_str().unwrap(), &seed_only).unwrap();
        assert_eq!(skipped.records, 0);
        assert!(row_missing(&facade, "moqui.test.TestEntity", "testId", "DEMO"));

        let csv_path = fix.join("rows.csv");
        std::fs::write(
            &csv_path,
            "moqui.test.TestEntity,seed\ntestId,testMedium\nCSV1,from-csv\n",
        )
        .unwrap();
        let csv_loaded = load_entity_data(&facade, csv_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(csv_loaded.records, 1);
        assert_eq!(
            text_of(&facade, "moqui.test.TestEntity", "testId", "CSV1", "testMedium"),
            Some("from-csv".to_string())
        );

        let once = fix.join("once.xml");
        std::fs::write(
            &once,
            r#"<entity-facade-xml type="seed"><moqui.test.TestEntity testId="OC1" testMedium="first"/></entity-facade-xml>"#,
        )
        .unwrap();
        assert_eq!(
            load_entity_data(&facade, once.to_str().unwrap(), &LoadOptions::default())
                .unwrap()
                .records,
            1
        );
        let again = fix.join("once-again.xml");
        std::fs::write(
            &again,
            r#"<entity-facade-xml type="seed"><moqui.test.TestEntity testId="OC1" testMedium="second"/></entity-facade-xml>"#,
        )
        .unwrap();
        let only_create = LoadOptions {
            only_create: true,
            ..LoadOptions::default()
        };
        assert_eq!(
            load_entity_data(&facade, again.to_str().unwrap(), &only_create)
                .unwrap()
                .records,
            0
        );
        assert_eq!(
            text_of(&facade, "moqui.test.TestEntity", "testId", "OC1", "testMedium"),
            Some("first".to_string())
        );

        let try_insert = LoadOptions {
            use_try_insert: true,
            ..LoadOptions::default()
        };
        assert_eq!(
            load_entity_data(&facade, again.to_str().unwrap(), &try_insert)
                .unwrap()
                .records,
            1
        );
        assert_eq!(
            text_of(&facade, "moqui.test.TestEntity", "testId", "OC1", "testMedium"),
            Some("second".to_string())
        );

        let nested = fix.join("nested.xml");
        std::fs::write(
            &nested,
            r#"<entity-facade-xml type="seed">
                <moqui.security.UserGroup userGroupId="G1" description="group">
                    <permissions userPermissionId="PERM1" fromDate="0"/>
                </moqui.security.UserGroup>
            </entity-facade-xml>"#,
        )
        .unwrap();
        assert_eq!(
            load_entity_data(&facade, nested.to_str().unwrap(), &LoadOptions::default())
                .unwrap()
                .records,
            2
        );
        assert_eq!(
            text_of(&facade, "moqui.security.UserGroup", "userGroupId", "G1", "description"),
            Some("group".to_string())
        );
        let perms = facade
            .find("moqui.security.UserGroupPermission")
            .unwrap()
            .eq("userGroupId", Value::string("G1"))
            .eq("userPermissionId", Value::string("PERM1"))
            .list()
            .unwrap();
        assert_eq!(perms.len(), 1);

        let member = fix.join("member.xml");
        std::fs::write(
            &member,
            r#"<entity-facade-xml type="seed">
                <moqui.security.UserGroupMember userGroupId="GDUMMY" userId="U1" fromDate="0"/>
            </entity-facade-xml>"#,
        )
        .unwrap();
        let dummy = LoadOptions {
            dummy_fks: true,
            ..LoadOptions::default()
        };
        assert_eq!(
            load_entity_data(&facade, member.to_str().unwrap(), &dummy)
                .unwrap()
                .records,
            1
        );
        assert!(!facade.flags().dummy_fks);
        assert_eq!(
            text_of(
                &facade,
                "moqui.security.UserGroup",
                "userGroupId",
                "GDUMMY",
                "userGroupId"
            ),
            Some("GDUMMY".to_string())
        );

        let missing = fix.join("missing-group.xml");
        std::fs::write(
            &missing,
            r#"<entity-facade-xml type="seed">
                <moqui.security.UserGroupMember userGroupId="GMISSING" userId="U2" fromDate="0"/>
            </entity-facade-xml>"#,
        )
        .unwrap();
        let blocked = load_entity_data(&facade, missing.to_str().unwrap(), &LoadOptions::default()).unwrap_err();
        assert!(blocked.to_string().contains("has no related"), "{blocked}");
        let no_fk = LoadOptions {
            disable_fk_create: true,
            ..LoadOptions::default()
        };
        let db_err = load_entity_data(&facade, missing.to_str().unwrap(), &no_fk).unwrap_err();
        let db_text = format!("{db_err:?}");
        assert!(db_text.contains("UserGroupMemberUserGroup"), "{db_text}");
        assert!(!facade.flags().disable_fk_create);
        assert!(row_missing(
            &facade,
            "moqui.security.UserGroup",
            "userGroupId",
            "GMISSING"
        ));

        let other = LoadOptions {
            components: vec!["tools".to_string()],
            ..LoadOptions::default()
        };
        assert_eq!(
            load_entity_data(&facade, "component://other/data/x.xml", &other)
                .unwrap()
                .records,
            0
        );

        let rules = eca_rules(&facade, "moqui.screen.form.DbForm");
        let form_rule = rules.iter().find(|rule| rule.id == "DbFormCache").expect("DbFormCache");
        assert!(form_rule.actions.contains("ec.cache"), "{}", form_rule.actions);
        assert!(form_rule.on_create && form_rule.on_update && form_rule.on_delete);
        let mut form = facade.make_value("moqui.screen.form.DbForm").unwrap();
        form.set("formId", Value::string("FORM1"));
        form.create(&facade).expect("eca action must not run");
        assert!(match_rules(&facade, "moqui.screen.form.DbForm", "create")
            .iter()
            .any(|id| id == "DbFormCache"));
        facade.set_disable_entity_eca(true);
        assert!(match_rules(&facade, "moqui.screen.form.DbForm", "update").is_empty());
        facade.set_disable_entity_eca(false);

        create_row(
            &facade,
            "moqui.basic.EnumerationType",
            &[("enumTypeId", "DataFeedType")],
        );
        create_row(
            &facade,
            "moqui.basic.Enumeration",
            &[("enumId", "DTFDTP_RT_PUSH"), ("enumTypeId", "DataFeedType")],
        );
        create_row(
            &facade,
            "moqui.entity.document.DataDocument",
            &[("dataDocumentId", "DOC1"), ("primaryEntityName", "moqui.test.Foo")],
        );
        create_row(
            &facade,
            "moqui.entity.feed.DataFeed",
            &[
                ("dataFeedId", "FEED1"),
                ("dataFeedTypeEnumId", "DTFDTP_RT_PUSH"),
                (
                    "feedReceiveServiceName",
                    "org.moqui.search.SearchServices.index#DataDocuments",
                ),
                (
                    "feedDeleteServiceName",
                    "org.moqui.search.SearchServices.delete#DataDocument",
                ),
            ],
        );
        create_row(
            &facade,
            "moqui.entity.feed.DataFeedDocument",
            &[("dataFeedId", "FEED1"), ("dataDocumentId", "DOC1")],
        );

        facade.begin("moqui.test.Foo", None).unwrap();
        create_row(&facade, "moqui.test.Foo", &[("fooId", "F1"), ("fooText", "alpha")]);
        facade.commit("moqui.test.Foo").unwrap();
        let pending = pending_for(&facade, "F1");
        assert_eq!(
            pending,
            vec![("DOC1".to_string(), "moqui.test.Foo".to_string(), "index".to_string())]
        );
        let services = held_feed_services(&facade);
        assert!(services
            .iter()
            .any(|service| service.receive_service == "org.moqui.search.SearchServices.index#DataDocuments"));
        assert!(services
            .iter()
            .any(|service| service.delete_service == "org.moqui.search.SearchServices.delete#DataDocument"));

        facade.begin("moqui.test.Foo", None).unwrap();
        create_row(&facade, "moqui.test.Foo", &[("fooId", "F2"), ("fooText", "beta")]);
        facade.rollback("moqui.test.Foo").unwrap();
        assert!(pending_for(&facade, "F2").is_empty());
        assert!(row_missing(&facade, "moqui.test.Foo", "fooId", "F2"));

        facade.set_disable_data_feed(true);
        facade.begin("moqui.test.Foo", None).unwrap();
        create_row(&facade, "moqui.test.Foo", &[("fooId", "F3"), ("fooText", "gamma")]);
        facade.commit("moqui.test.Foo").unwrap();
        assert!(pending_for(&facade, "F3").is_empty());
        assert_eq!(
            text_of(&facade, "moqui.test.Foo", "fooId", "F3", "fooText"),
            Some("gamma".to_string())
        );
        facade.set_disable_data_feed(false);

        drop(facade);
        drop(cluster);
    }

    fn create_row(facade: &EntityFacade, entity: &str, pairs: &[(&str, &str)]) {
        let mut row = facade.make_value(entity).unwrap();
        for (name, value) in pairs {
            row.set(*name, Value::string(*value));
        }
        row.create(facade).unwrap();
    }

    fn text_of(facade: &EntityFacade, entity: &str, pk_field: &str, pk: &str, field: &str) -> Option<String> {
        facade
            .find(entity)
            .unwrap()
            .eq(pk_field, Value::string(pk))
            .one()
            .unwrap()
            .and_then(|row| row.get(field).and_then(Value::as_str).map(str::to_string))
    }

    fn row_missing(facade: &EntityFacade, entity: &str, pk_field: &str, pk: &str) -> bool {
        facade
            .find(entity)
            .unwrap()
            .eq(pk_field, Value::string(pk))
            .one()
            .unwrap()
            .is_none()
    }

    fn pending_for(facade: &EntityFacade, pk: &str) -> Vec<(String, String, String)> {
        facade
            .find("moqui.entity.feed.DataFeedPending")
            .unwrap()
            .eq("primaryKey", Value::string(pk))
            .list()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    cell(row, "dataDocumentId"),
                    cell(row, "entityName"),
                    cell(row, "operation"),
                )
            })
            .collect()
    }

    fn cell(row: &moqui_core::entity::EntityValue, name: &str) -> String {
        row.get(name).and_then(Value::as_str).unwrap_or("").to_string()
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
