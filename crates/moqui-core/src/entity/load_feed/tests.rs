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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moqui_db::dialect::Dialects;
use moqui_db::model::EntityModel;
use moqui_db::sql::insert_or_update;
use moqui_lib::error::Error;
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::feed::TOUCHES;
use super::{
    conf_data_locations, data_file_services, eca_rules, feed_service_names, load_entity_data, match_rules,
    register_feed_insert, LoadOptions,
};
use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
use crate::entity::{EntityFacade, EntityGroup};
use crate::transaction::{PoolConfig, TransactionManager};

struct NoCancel;

impl DbCancel for NoCancel {
    fn cancel(&self) -> Result<(), Error> {
        Ok(())
    }
}

struct EmptyConn;

impl DbConnection for EmptyConn {
    fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, Error> {
        Ok(0)
    }
    fn query(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        Ok(Vec::new())
    }
    fn query_once(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        Ok(Vec::new())
    }
    fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
        Ok(())
    }
    fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
        Ok(None)
    }
    fn close_stream(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn stream_open(&self) -> bool {
        false
    }
    fn begin(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn commit(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn rollback(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn savepoint(&mut self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn release_savepoint(&mut self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn canceller(&self) -> Box<dyn DbCancel> {
        Box::new(NoCancel)
    }
}

struct EmptyDriver;

impl DbDriver for EmptyDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        Ok(Box::new(EmptyConn))
    }
}

struct Script {
    inserts: Mutex<Vec<Vec<Value>>>,
    missing_feed: bool,
    /// Entity inserts fail while set, as a duplicate key would.
    fail_inserts: AtomicBool,
}

struct ScriptConn {
    script: Arc<Script>,
}

impl DbConnection for ScriptConn {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
        if sql.contains("DataFeedPending") {
            self.script.inserts.lock().expect("inserts").push(params.to_vec());
        } else if sql.starts_with("INSERT") && self.script.fail_inserts.load(Ordering::SeqCst) {
            return Err(Error::new("duplicate key value violates unique constraint"));
        }
        Ok(1)
    }
    fn query(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        if self.script.missing_feed && sql.contains("DataFeed") {
            let source = std::io::Error::other("relation \"DataFeed\" does not exist");
            return Err(Error::new("postgres query").with_source(source));
        }
        if sql.contains("xmin") {
            return Ok(vec![db_row(&[("fooId", "F1")])]);
        }
        if sql.contains("DataDocumentField") {
            let source = std::io::Error::other("relation \"DataDocumentField\" does not exist");
            return Err(Error::new("postgres query").with_source(source));
        }
        if sql.contains("DataFeedDocument") {
            return Ok(vec![db_row(&[("dataFeedId", "FEED1"), ("dataDocumentId", "DOC1")])]);
        }
        if sql.contains("DataDocument") {
            return Ok(vec![db_row(&[
                ("dataDocumentId", "DOC1"),
                ("primaryEntityName", "moqui.test.Foo"),
            ])]);
        }
        if sql.contains("DataFeed") {
            return Ok(vec![db_row(&[
                ("dataFeedId", "FEED1"),
                ("feedReceiveServiceName", "index#DataDocuments"),
                ("feedDeleteServiceName", "delete#DataDocument"),
            ])]);
        }
        Ok(Vec::new())
    }
    fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        self.query(sql, params)
    }
    fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
        Ok(())
    }
    fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
        Ok(None)
    }
    fn close_stream(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn stream_open(&self) -> bool {
        false
    }
    fn begin(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn commit(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn rollback(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn savepoint(&mut self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn release_savepoint(&mut self, _name: &str) -> Result<(), Error> {
        Ok(())
    }
    fn canceller(&self) -> Box<dyn DbCancel> {
        Box::new(NoCancel)
    }
}

struct ScriptDriver {
    script: Arc<Script>,
}

impl DbDriver for ScriptDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        Ok(Box::new(ScriptConn {
            script: Arc::clone(&self.script),
        }))
    }
}

fn db_row(pairs: &[(&str, &str)]) -> DbRow {
    DbRow {
        columns: pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), Value::string(*value)))
            .collect(),
    }
}

fn open_with(driver: Arc<dyn DbDriver>) -> EntityFacade {
    let transactions = TransactionManager::new(
        driver,
        PoolConfig {
            min: 0,
            max: 2,
            reserve: 1,
            acquire_timeout: Duration::from_secs(5),
            ..Default::default()
        },
    )
    .unwrap();
    EntityFacade::open(
        &framework_dir(),
        "transactional",
        vec![EntityGroup::new("transactional", "postgres", transactions)],
    )
    .unwrap()
}

fn open_empty() -> EntityFacade {
    open_with(Arc::new(EmptyDriver))
}

fn framework_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir() -> (PathBuf, DirGuard) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("moqui-load-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let guard = DirGuard(path.clone());
    (path, guard)
}

/// A stored entry the way Java `ZipOutputStream` writes it: flag bit 3, zero local sizes, and a
/// data descriptor after the data. Only the central directory carries the sizes.
fn descriptor_zip(name: &str, data: &[u8]) -> Vec<u8> {
    let plain = stored_zip(name, data);
    let local_len = 30 + name.len() + data.len();
    let mut out = plain[..local_len].to_vec();
    out[6] |= 0x08;
    out[18..26].fill(0);
    let shift = 16_u32;
    out.extend_from_slice(&0x0807_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    out.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    let mut rest = plain[local_len..].to_vec();
    // The end record's directory offset moves past the descriptor.
    let end_at = rest.len() - 22;
    let offset_at = end_at + 16;
    let moved = u32::from_le_bytes(rest[offset_at..offset_at + 4].try_into().unwrap()) + shift;
    rest[offset_at..offset_at + 4].copy_from_slice(&moved.to_le_bytes());
    out.extend(rest);
    out
}

fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut local = Vec::new();
    local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    local.extend_from_slice(&20_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&0_u32.to_le_bytes());
    local.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    local.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    local.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(name_bytes);
    local.extend_from_slice(data);
    let mut central = Vec::new();
    central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    central.extend_from_slice(&20_u16.to_le_bytes());
    central.extend_from_slice(&20_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    central.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
    central.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(name_bytes);
    let mut end = Vec::new();
    end.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    end.extend_from_slice(&1_u16.to_le_bytes());
    end.extend_from_slice(&1_u16.to_le_bytes());
    end.extend_from_slice(&(u32::try_from(central.len()).unwrap()).to_le_bytes());
    end.extend_from_slice(&(u32::try_from(local.len()).unwrap()).to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    let mut out = local;
    out.extend(central);
    out.extend(end);
    out
}

#[test]
fn eca_rules_are_stored_and_disable_matches_nothing() {
    let facade = open_empty();
    let rules = eca_rules(&facade, "moqui.screen.form.DbForm");
    let form = rules.iter().find(|rule| rule.id == "DbFormCache").expect("DbFormCache");
    assert!(form.actions.contains("ec.cache"), "{}", form.actions);
    assert!(form.on_create && form.on_update && form.on_delete);
    let member = eca_rules(&facade, "moqui.security.UserGroupMember");
    let privileged = member
        .iter()
        .find(|rule| rule.id == "UserGroupMemberPrivileged")
        .expect("privileged");
    assert!(privileged.run_before);
    assert!(privileged.actions.contains("assertPrivilegedGroupMembership"));
    facade.begin("moqui.screen.form.DbForm", None).unwrap();
    let ids = match_rules(&facade, "moqui.screen.form.DbForm", "create");
    assert!(ids.iter().any(|id| id == "DbFormCache"));
    facade.set_disable_entity_eca(true);
    assert!(match_rules(&facade, "moqui.screen.form.DbForm", "update").is_empty());
    facade.rollback("moqui.screen.form.DbForm").unwrap();
}

#[test]
fn conf_locations_name_the_seed_files() {
    let locations = conf_data_locations(&open_empty()).unwrap();
    assert!(locations.iter().any(|location| location.contains("MoquiSetupData.xml")));
}

#[test]
fn loader_filters_types_holds_services_and_reads_zip() {
    let facade = open_empty();
    // A data-file service element runs in the entered context.
    let context = crate::context::ExecutionContextFactory::test_context();
    context.factory().set_entity(facade.clone());
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let (dir, _guard) = temp_dir();
    let xml = dir.join("rows.xml");
    std::fs::write(
        &xml,
        r#"<entity-facade-xml type="seed">
            <moqui.test.TestEntity testId="T1" testMedium="xml"/>
            <create-moqui.test.TestEntity testId="SVC" testMedium="held"/>
        </entity-facade-xml>"#,
    )
    .unwrap();
    let loaded = load_entity_data(&facade, xml.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(loaded.records, 1);
    let held = data_file_services(&facade);
    assert!(held
        .iter()
        .any(|service| service.name == "create#moqui.test.TestEntity"));
    assert_eq!(held[0].parameters.get("testId").map(String::as_str), Some("SVC"));

    let demo = LoadOptions {
        data_types: vec!["demo".to_string()],
        ..LoadOptions::default()
    };
    let skipped = load_entity_data(&facade, xml.to_str().unwrap(), &demo).unwrap();
    assert_eq!(skipped.records, 0);

    let csv_path = dir.join("rows.csv");
    std::fs::write(&csv_path, "moqui.test.TestEntity,seed\ntestId,testMedium\nC1,csv\n").unwrap();
    let csv_loaded = load_entity_data(&facade, csv_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(csv_loaded.records, 1);

    let json_path = dir.join("rows.json");
    std::fs::write(
        &json_path,
        r#"{"_entity":"moqui.test.TestEntity","_dataType":"seed","testId":"J1","testMedium":"json"}"#,
    )
    .unwrap();
    let json_loaded = load_entity_data(&facade, json_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(json_loaded.records, 1);

    let nested = dir.join("nested.json");
    std::fs::write(
        &nested,
        r#"{"_entity":"moqui.security.UserGroup","userGroupId":"G1","description":"g","permissions":{"userPermissionId":"P1","fromDate":"0"}}"#,
    )
    .unwrap();
    // The empty driver cannot see the parent row, so the child foreign key uses a dummy.
    let nested_options = LoadOptions {
        dummy_fks: true,
        ..LoadOptions::default()
    };
    let nested_loaded = load_entity_data(&facade, nested.to_str().unwrap(), &nested_options).unwrap();
    assert_eq!(nested_loaded.records, 2);
    assert!(!facade.flags().dummy_fks);

    let zip_path = dir.join("rows.zip");
    let csv = b"moqui.test.TestEntity,seed\ntestId,testMedium\nZ1,zip\n";
    std::fs::write(&zip_path, stored_zip("rows.csv", csv)).unwrap();
    let zip_loaded = load_entity_data(&facade, zip_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(zip_loaded.records, 1);

    let described = dir.join("described.zip");
    std::fs::write(&described, descriptor_zip("rows.csv", csv)).unwrap();
    let described_loaded = load_entity_data(&facade, described.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(described_loaded.records, 1);

    let deflated = dir.join("deflated.zip");
    let payload: &[u8] = &[
        203, 205, 47, 44, 205, 212, 43, 73, 45, 46, 209, 11, 1, 18, 174, 121, 37, 153, 37, 149, 58, 197, 169, 169, 41,
        92, 32, 65, 207, 20, 29, 16, 229, 155, 154, 146, 89, 154, 203, 21, 101, 168, 83, 149, 89, 192, 5, 0,
    ];
    std::fs::write(&deflated, deflate_zip("rows.csv", payload, csv.len())).unwrap();
    let deflated_loaded = load_entity_data(&facade, deflated.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(deflated_loaded.records, 1);

    let tools = LoadOptions {
        components: vec!["tools".to_string()],
        ..LoadOptions::default()
    };
    let blocked = load_entity_data(&facade, "component://other/data/x.xml", &tools).unwrap();
    assert_eq!(blocked.records, 0);

    facade.set_dummy_fks(false);
    facade.set_disable_fk_create(false);
    let flags = LoadOptions {
        dummy_fks: true,
        disable_fk_create: true,
        use_try_insert: true,
        only_create: true,
        ..LoadOptions::default()
    };
    let again = load_entity_data(&facade, xml.to_str().unwrap(), &flags).unwrap();
    assert_eq!(again.records, 1);
    assert!(!facade.flags().dummy_fks);
    assert!(!facade.flags().disable_fk_create);
}

#[test]
fn loader_reads_seed_data_inside_entity_definitions() {
    let facade = open_empty();
    let (dir, _guard) = temp_dir();
    let xml = dir.join("entities.xml");
    std::fs::write(
        &xml,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<entities xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <entity entity-name="One" package="moqui.test">
<field name="testId" type="id" is-pk="true"/>
<field name="testMedium" type="text-medium"/>
<relationship type="one" related="moqui.test.Foo"/>
<seed-data>
  <moqui.test.TestEntity testId="S1" testMedium="one"/>
</seed-data>
  </entity>
  <entity entity-name="Two" package="moqui.test">
<field name="testId" type="id" is-pk="true"/>
<seed-data>
  <moqui.test.TestEntity testId="S2" testMedium="two"/>
</seed-data>
  </entity>
</entities>"#,
    )
    .unwrap();
    let loaded = load_entity_data(&facade, xml.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(loaded.records, 2);
    let demo = LoadOptions {
        data_types: vec!["demo".to_string()],
        ..LoadOptions::default()
    };
    let skipped = load_entity_data(&facade, xml.to_str().unwrap(), &demo).unwrap();
    assert_eq!(skipped.records, 0);

    let bare = dir.join("bare.xml");
    std::fs::write(
        &bare,
        r#"<entities><entity entity-name="Only" package="moqui.test"><field name="testId" type="id" is-pk="true"/></entity></entities>"#,
    )
    .unwrap();
    let none = load_entity_data(&facade, bare.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(none.records, 0);
}

#[test]
fn nested_many_relationship_inherits_parent_primary_key() {
    let facade = open_empty();
    let xml = r#"<moqui.service.job.ServiceJob jobName="clean_test" serviceName="example" cronExpression="0 0 2 * * ?">
  <parameters parameterName="daysToKeep" parameterValue="1"/>
  <parameters jobName="other" parameterName="kept" parameterValue="2"/>
</moqui.service.job.ServiceJob>"#;
    let node = MNode::parse_text("job", xml).unwrap();
    let record = super::loader::parse_record(&facade, "moqui.service.job.ServiceJob", &node).unwrap();
    assert_eq!(record.children.len(), 2);
    assert_eq!(record.children[0].entity, "moqui.service.job.ServiceJobParameter");
    assert_eq!(
        record.children[0].fields.get("jobName").and_then(Value::as_str),
        Some("clean_test")
    );
    assert_eq!(
        record.children[0].fields.get("parameterName").and_then(Value::as_str),
        Some("daysToKeep")
    );
    assert_eq!(
        record.children[1].fields.get("jobName").and_then(Value::as_str),
        Some("other")
    );
}

#[test]
fn loader_streams_attributes_comments_and_json_arrays() {
    let element = r#"<moqui.test.TestEntity testId="A1" testMedium="a>b"/>"#;
    let end = super::xml::xml_element_end(element, true).unwrap().unwrap();
    assert_eq!(end, element.len());
    let node = MNode::parse_text("element", element).unwrap();
    assert_eq!(node.attribute("testMedium").as_deref(), Some("a>b"));

    let quoted = r#"{"testMedium":"say \"hi\""}"#;
    let value_end = super::json::json_value_end(quoted, true).unwrap().unwrap();
    assert_eq!(value_end, quoted.len());
    let super::json::Json::Object(pairs) = super::json::parse_json(quoted).unwrap() else {
        panic!("quoted json is not an object");
    };
    let medium = pairs.iter().find(|(key, _)| key == "testMedium").expect("testMedium");
    assert!(matches!(&medium.1, super::json::Json::String(text) if text == "say \"hi\""));

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- see <http://creativecommons.org/publicdomain/zero/1.0/> -->
<!DOCTYPE entity-facade-xml [
  <!ELEMENT entity-facade-xml ANY>
]>
<entity-facade-xml type="seed">
  <moqui.test.TestEntity testId="A1" testMedium="a>b"/>
  <moqui.test.TestEntity testId="A2" testMedium="plain"/>
</entity-facade-xml>
"#;
    let facade = open_empty();
    let xml_reader = std::io::BufReader::with_capacity(1, std::io::Cursor::new(xml.as_bytes()));
    let xml_loaded = super::xml::load_xml_reader(&facade, "chunk.xml", xml_reader, &LoadOptions::default()).unwrap();
    assert_eq!(xml_loaded, 2);

    let json = r#"[{"_dataType":"seed"},{"_entity":"moqui.test.TestEntity","testId":"B1","testMedium":"one"},{"_entity":"moqui.test.TestEntity","testId":"B2","testMedium":"say \"hi\""}]"#;
    let json_reader = std::io::BufReader::with_capacity(1, std::io::Cursor::new(json.as_bytes()));
    let json_loaded =
        super::json::load_json_reader(&facade, "chunk.json", json_reader, &LoadOptions::default()).unwrap();
    assert_eq!(json_loaded, 2);

    let (dir, _guard) = temp_dir();
    let json_path = dir.join("rows.json");
    std::fs::write(&json_path, json).unwrap();
    let from_file = load_entity_data(&facade, json_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(from_file.records, 2);
}

#[test]
fn loader_scans_one_large_element_in_linear_time() {
    let mut medium = String::new();
    while medium.len() < 8 * 1024 * 1024 {
        medium.push_str("0123456789abcdef");
    }
    let xml = format!(
        "<entity-facade-xml type=\"seed\"><moqui.test.TestEntity testId=\"BIG\" testMedium=\"{medium}\"/></entity-facade-xml>"
    );
    let facade = open_empty();
    let started = std::time::Instant::now();
    let reader = std::io::BufReader::with_capacity(4096, std::io::Cursor::new(xml.as_bytes()));
    let loaded = super::xml::load_xml_reader(&facade, "big.xml", reader, &LoadOptions::default()).unwrap();
    assert_eq!(loaded, 1);
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
}

fn deflate_zip(name: &str, compressed: &[u8], plain_len: usize) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut local = Vec::new();
    local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    local.extend_from_slice(&20_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&8_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(&0_u32.to_le_bytes());
    local.extend_from_slice(&(u32::try_from(compressed.len()).unwrap()).to_le_bytes());
    local.extend_from_slice(&(u32::try_from(plain_len).unwrap()).to_le_bytes());
    local.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
    local.extend_from_slice(&0_u16.to_le_bytes());
    local.extend_from_slice(name_bytes);
    local.extend_from_slice(compressed);
    let mut central = Vec::new();
    central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    central.extend_from_slice(&20_u16.to_le_bytes());
    central.extend_from_slice(&20_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&8_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(&(u32::try_from(compressed.len()).unwrap()).to_le_bytes());
    central.extend_from_slice(&(u32::try_from(plain_len).unwrap()).to_le_bytes());
    central.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u16.to_le_bytes());
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(name_bytes);
    let mut end = Vec::new();
    end.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    end.extend_from_slice(&1_u16.to_le_bytes());
    end.extend_from_slice(&1_u16.to_le_bytes());
    end.extend_from_slice(&(u32::try_from(central.len()).unwrap()).to_le_bytes());
    end.extend_from_slice(&(u32::try_from(local.len()).unwrap()).to_le_bytes());
    end.extend_from_slice(&0_u16.to_le_bytes());
    let mut out = local;
    out.extend(central);
    out.extend(end);
    out
}

#[test]
fn inflate_covers_stored_fixed_and_dynamic_blocks() {
    let stored = [1_u8, 5, 0, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'];
    assert_eq!(super::zip::inflate(&stored).unwrap(), b"hello");
    let fixed = [203_u8, 72, 205, 201, 201, 7, 0];
    assert_eq!(super::zip::inflate(&fixed).unwrap(), b"hello");
    let dynamic = [5_u8, 192, 129, 0, 0, 0, 0, 0, 144, 54, 255, 83, 8];
    assert_eq!(super::zip::inflate(&dynamic).unwrap(), b"A");
}

#[test]
fn mysql_try_insert_uses_duplicate_key_update() {
    let framework = framework_dir();
    let conf = MNode::parse_file(framework.join("conf/MoquiDefaultConf.xml")).unwrap();
    let dialects = Dialects::load(&conf).unwrap();
    let model = EntityModel::load_directory(&conf, &framework.join("entity")).unwrap();
    let entity = model.entity("moqui.test.Foo").unwrap();
    let mysql = dialects.get("mysql").unwrap();
    let sql = insert_or_update(entity, mysql, &["fooId", "fooText"]).unwrap();
    assert!(sql.contains("ON DUPLICATE KEY UPDATE"), "{sql}");
    let postgres = dialects.get("postgres").unwrap();
    let err = insert_or_update(entity, postgres, &["fooId"]).unwrap_err();
    assert!(err.to_string().contains("try-insert is off"), "{err}");
}

#[test]
fn feed_insert_uses_the_enlisted_connection_and_skips_a_missing_catalog() {
    let script = Arc::new(Script {
        inserts: Mutex::new(Vec::new()),
        missing_feed: false,
        fail_inserts: AtomicBool::new(false),
    });
    let facade = open_with(Arc::new(ScriptDriver {
        script: Arc::clone(&script),
    }));
    facade.begin("moqui.test.Foo", None).unwrap();
    match_rules(&facade, "moqui.test.Foo", "create");
    let tx = facade.transactions_for("moqui.test.Foo").unwrap();
    register_feed_insert(&facade, &tx).unwrap();
    enlist(&tx);
    facade.commit("moqui.test.Foo").unwrap();
    let inserts = script.inserts.lock().expect("inserts");
    assert_eq!(inserts.len(), 1, "{inserts:?}");
    assert_eq!(inserts[0][1], Value::string("DOC1"));
    assert_eq!(inserts[0][2], Value::string("moqui.test.Foo"));
    assert_eq!(inserts[0][3], Value::string("F1"));
    assert_eq!(inserts[0][4], Value::string("index"));
    drop(inserts);
    let held = feed_service_names(&facade);
    assert!(held
        .iter()
        .any(|service| service.receive_service == "index#DataDocuments"));
    assert!(held
        .iter()
        .any(|service| service.delete_service == "delete#DataDocument"));

    facade.set_disable_data_feed(true);
    facade.begin("moqui.test.Foo", None).unwrap();
    match_rules(&facade, "moqui.test.Foo", "create");
    let tx = facade.transactions_for("moqui.test.Foo").unwrap();
    register_feed_insert(&facade, &tx).unwrap();
    enlist(&tx);
    facade.commit("moqui.test.Foo").unwrap();
    assert_eq!(script.inserts.lock().expect("inserts").len(), 1);
    facade.set_disable_data_feed(false);

    facade.set_sim_active(true);
    facade.begin("moqui.test.Foo", None).unwrap();
    match_rules(&facade, "moqui.test.Foo", "delete");
    let tx = facade.transactions_for("moqui.test.Foo").unwrap();
    register_feed_insert(&facade, &tx).unwrap();
    enlist(&tx);
    facade.commit("moqui.test.Foo").unwrap();
    assert_eq!(script.inserts.lock().expect("inserts").len(), 1);
    facade.set_sim_active(false);

    let missing = Arc::new(Script {
        inserts: Mutex::new(Vec::new()),
        missing_feed: true,
        fail_inserts: AtomicBool::new(false),
    });
    let facade = open_with(Arc::new(ScriptDriver {
        script: Arc::clone(&missing),
    }));
    facade.begin("moqui.test.Foo", None).unwrap();
    match_rules(&facade, "moqui.test.Foo", "create");
    let tx = facade.transactions_for("moqui.test.Foo").unwrap();
    register_feed_insert(&facade, &tx).unwrap();
    enlist(&tx);
    facade.commit("moqui.test.Foo").unwrap();
    assert!(missing.inserts.lock().expect("inserts").is_empty());
}

fn enlist(tx: &TransactionManager) {
    tx.with_connection(|conn| conn.execute("touch", &[])).unwrap();
}

#[test]
fn autocommit_match_does_not_insert_on_a_later_commit() {
    let script = Arc::new(Script {
        inserts: Mutex::new(Vec::new()),
        missing_feed: false,
        fail_inserts: AtomicBool::new(false),
    });
    let facade = open_with(Arc::new(ScriptDriver {
        script: Arc::clone(&script),
    }));
    assert!(match_rules(&facade, "moqui.test.Foo", "create").is_empty());
    facade.begin("moqui.test.Foo", None).unwrap();
    let tx = facade.transactions_for("moqui.test.Foo").unwrap();
    register_feed_insert(&facade, &tx).unwrap();
    enlist(&tx);
    facade.commit("moqui.test.Foo").unwrap();
    assert!(script.inserts.lock().expect("inserts").is_empty());
}

fn create_foo(facade: &EntityFacade, id: &str) -> Result<(), Error> {
    let mut value = facade.make_value("moqui.test.Foo")?;
    value.set("fooId", Value::string(id));
    value.create(facade)
}

fn frame_touches() -> usize {
    TOUCHES.with(|touches| touches.borrow().values().map(Vec::len).sum())
}

#[test]
fn a_failed_first_write_leaves_no_touch_and_feed_names_do_not_grow() {
    let script = Arc::new(Script {
        inserts: Mutex::new(Vec::new()),
        missing_feed: false,
        fail_inserts: AtomicBool::new(true),
    });
    let facade = open_with(Arc::new(ScriptDriver {
        script: Arc::clone(&script),
    }));
    facade.begin("moqui.test.Foo", None).unwrap();
    assert!(create_foo(&facade, "F9").is_err());
    assert_eq!(frame_touches(), 0, "the failed write noted nothing");
    facade.rollback("moqui.test.Foo").unwrap();
    assert_eq!(frame_touches(), 0);
    assert!(script.inserts.lock().expect("inserts").is_empty());

    script.fail_inserts.store(false, Ordering::SeqCst);
    facade.begin("moqui.test.Foo", None).unwrap();
    create_foo(&facade, "F1").unwrap();
    assert_eq!(frame_touches(), 1);
    facade.commit("moqui.test.Foo").unwrap();
    assert_eq!(frame_touches(), 0, "the commit took this frame's touches");
    let ids: Vec<Value> = script
        .inserts
        .lock()
        .expect("inserts")
        .iter()
        .map(|params| params[3].clone())
        .collect();
    assert_eq!(ids, vec![Value::string("F1")], "pending rows only for the written row");

    for id in ["F2", "F3"] {
        facade.begin("moqui.test.Foo", None).unwrap();
        create_foo(&facade, id).unwrap();
        facade.commit("moqui.test.Foo").unwrap();
    }
    assert_eq!(script.inserts.lock().expect("inserts").len(), 3);
    assert_eq!(
        feed_service_names(&facade).len(),
        1,
        "one entry per feed, whatever the commits"
    );
    assert_eq!(frame_touches(), 0);
}
