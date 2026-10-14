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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moqui_db::dialect::Dialects;
use moqui_db::model::EntityModel;
use moqui_db::sql::{select_view, LimitOffset};
use moqui_lib::error::Error;
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::crypto::CryptConfig;
use super::{EntityFacade, EntityGroup};
use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
use crate::transaction::{PoolConfig, TransactionManager};

fn framework_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
}

struct FakeDriver;

struct FakeConn;

struct NoCancel;

impl DbCancel for NoCancel {
    fn cancel(&self) -> Result<(), Error> {
        Ok(())
    }
}

impl DbConnection for FakeConn {
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

impl DbDriver for FakeDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        Ok(Box::new(FakeConn))
    }
}

struct Tape {
    sql: Vec<String>,
    password: Option<String>,
}

struct IterTape {
    opened: bool,
    served: bool,
}

struct IterDriver {
    tape: Arc<Mutex<IterTape>>,
}

struct IterConn {
    tape: Arc<Mutex<IterTape>>,
}

fn test_row() -> DbRow {
    DbRow {
        columns: vec![
            ("testId".to_string(), Value::string("T1")),
            ("testMedium".to_string(), Value::string("kept")),
        ],
    }
}

impl DbConnection for IterConn {
    fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, Error> {
        Ok(1)
    }

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        self.query_once(sql, params)
    }

    fn query_once(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        if sql.contains("TestEntity") && sql.contains("SELECT") {
            return Ok(vec![test_row()]);
        }
        Ok(Vec::new())
    }

    fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
        let mut tape = self.tape.lock().unwrap_or_else(|err| err.into_inner());
        tape.opened = true;
        tape.served = false;
        Ok(())
    }

    fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
        let mut tape = self.tape.lock().unwrap_or_else(|err| err.into_inner());
        if !tape.opened || tape.served {
            return Ok(None);
        }
        tape.served = true;
        Ok(Some(test_row()))
    }

    fn close_stream(&mut self) -> Result<(), Error> {
        Ok(())
    }

    fn stream_open(&self) -> bool {
        self.tape.lock().unwrap_or_else(|err| err.into_inner()).opened
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

impl DbDriver for IterDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        Ok(Box::new(IterConn {
            tape: Arc::clone(&self.tape),
        }))
    }
}

struct RecDriver {
    tape: Arc<Mutex<Tape>>,
}

struct RecConn {
    tape: Arc<Mutex<Tape>>,
}

impl DbConnection for RecConn {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
        let mut tape = self.tape.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(stored) = params.iter().find_map(|value| {
            value
                .as_str()
                .filter(|text| text.starts_with(':') && *text != "TestEntityFieldValue")
                .map(str::to_string)
        }) {
            tape.password = Some(stored);
        }
        tape.sql.push(sql.to_string());
        Ok(1)
    }

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        self.query_once(sql, params)
    }

    fn query_once(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        let tape = self.tape.lock().unwrap_or_else(|err| err.into_inner());
        if sql.contains("InstanceHost") && sql.contains("SELECT") {
            if let Some(password) = tape.password.clone() {
                return Ok(vec![DbRow {
                    columns: vec![
                        ("instanceHostId".to_string(), Value::string("H1")),
                        ("password".to_string(), Value::string(password)),
                    ],
                }]);
            }
        }
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

impl DbDriver for RecDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        Ok(Box::new(RecConn {
            tape: Arc::clone(&self.tape),
        }))
    }
}

fn open_recording(tape: Arc<Mutex<Tape>>, dialect: &str) -> EntityFacade {
    let driver: Arc<dyn DbDriver> = Arc::new(RecDriver { tape });
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
        vec![EntityGroup::new("transactional", dialect, transactions)],
    )
    .unwrap()
}

#[test]
fn enter_sim_starts_the_overlay_once_and_stays_on_this_thread() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::cache::{enter_sim, exit_sim, sim_entered};

    let facade = open_fake();
    let err = enter_sim(&facade).unwrap_err();
    assert!(err.to_string().contains("not configured"), "{err}");
    assert!(!facade.sim_active());

    let starts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&starts);
    facade.set_sim_starter(Some(Box::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        TransactionManager::new(
            Arc::new(FakeDriver),
            PoolConfig {
                min: 0,
                max: 2,
                reserve: 0,
                acquire_timeout: Duration::from_secs(5),
                ..Default::default()
            },
        )
    })));
    enter_sim(&facade).unwrap();
    enter_sim(&facade).unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert!(facade.sim_active() && sim_entered());
    let other = facade.clone();
    let seen = std::thread::spawn(move || other.sim_active()).join().unwrap();
    assert!(!seen, "another thread is not in this thread's sim");
    exit_sim(&facade).unwrap();
    assert!(facade.sim_active(), "the outer schema is still open");
    exit_sim(&facade).unwrap();
    assert!(!facade.sim_active() && !sim_entered());
    exit_sim(&facade).unwrap();
}

fn open_fake() -> EntityFacade {
    let driver: Arc<dyn DbDriver> = Arc::new(FakeDriver);
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

#[test]
fn locale_defaults_to_en_and_crypt_stays_unset() {
    let facade = open_fake();
    assert_eq!(facade.locale(), "en");
    assert!(facade.crypt().is_none());
    facade.set_locale("fr");
    assert_eq!(facade.locale(), "fr");
    assert!(!facade.sim_active());
    facade.set_sim_active(true);
    assert!(facade.sim_active());
    assert!(facade.sim_transactions().is_none());
    facade.set_sim_transactions(None);
    facade.with_slot(
        || String::from("seed"),
        |slot| {
            assert_eq!(slot, "seed");
            slot.push_str("-kept");
        },
    );
    facade.with_slot(|| String::from("other"), |slot| assert_eq!(slot, "seed-kept"));
    let primary = CryptConfig {
        pass: "MoquiDefaultPassword:CHANGEME".to_string(),
        salt: "20201202".to_string(),
        iterations: 10,
        algorithm: "PBEWithHmacSHA256AndAES_128".to_string(),
    };
    facade.set_crypt(primary.clone(), Vec::new());
    let (stored, alts) = facade.crypt().unwrap();
    assert_eq!(stored.pass, primary.pass);
    assert!(alts.is_empty());
}

#[test]
fn try_insert_seals_the_password_before_sql() {
    let tape = Arc::new(Mutex::new(Tape {
        sql: Vec::new(),
        password: None,
    }));
    let facade = open_recording(Arc::clone(&tape), "mysql");
    facade.set_crypt(
        CryptConfig {
            pass: "MoquiDefaultPassword:CHANGEME".to_string(),
            salt: "20201202".to_string(),
            iterations: 10,
            algorithm: "PBEWithHmacSHA256AndAES_128".to_string(),
        },
        Vec::new(),
    );
    facade.set_disable_fk_create(true);
    facade.set_disable_data_feed(true);
    let mut fields = BTreeMap::new();
    fields.insert("instanceHostId".to_string(), Value::string("H1"));
    fields.insert("password".to_string(), Value::string("TestEntityFieldValue"));
    facade
        .upsert_record("moqui.server.instance.InstanceHost", &fields)
        .unwrap();
    fields.insert("instanceHostId".to_string(), Value::string("H2"));
    facade.begin("moqui.server.instance.InstanceHost", None).unwrap();
    facade
        .upsert_record("moqui.server.instance.InstanceHost", &fields)
        .unwrap();
    facade.commit("moqui.server.instance.InstanceHost").unwrap();
    let recorded = tape.lock().unwrap_or_else(|err| err.into_inner());
    let upserts: Vec<_> = recorded
        .sql
        .iter()
        .filter(|sql| sql.contains("ON DUPLICATE KEY UPDATE"))
        .collect();
    assert_eq!(upserts.len(), 2, "{:?}", recorded.sql);
    let stored = recorded.password.clone().unwrap_or_default();
    assert!(stored.starts_with(':'), "{stored}");
    assert_ne!(stored, "TestEntityFieldValue");
    drop(recorded);
    let found = facade
        .find("moqui.server.instance.InstanceHost")
        .unwrap()
        .eq("instanceHostId", Value::string("H1"))
        .one()
        .unwrap()
        .unwrap();
    assert_eq!(
        found.get("password").and_then(Value::as_str),
        Some("TestEntityFieldValue")
    );
}

#[test]
fn iterator_streams_committed_rows_and_hides_a_held_delete() {
    let tape = Arc::new(Mutex::new(IterTape {
        opened: false,
        served: false,
    }));
    let driver: Arc<dyn DbDriver> = Arc::new(IterDriver {
        tape: Arc::clone(&tape),
    });
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
    let facade = EntityFacade::open(
        &framework_dir(),
        "transactional",
        vec![EntityGroup::new("transactional", "postgres", transactions)],
    )
    .unwrap();

    facade.begin("moqui.test.TestEntity", None).unwrap();
    let mut iter = facade.find("moqui.test.TestEntity").unwrap().iterator().unwrap();
    let row = iter.next_value().unwrap().unwrap();
    assert_eq!(row.get("testId").and_then(Value::as_str), Some("T1"));
    assert!(iter.next_value().unwrap().is_none());
    drop(iter);
    assert!(tape.lock().unwrap_or_else(|err| err.into_inner()).opened);
    facade.rollback("moqui.test.TestEntity").unwrap();

    {
        let mut tape = tape.lock().unwrap_or_else(|err| err.into_inner());
        tape.opened = false;
        tape.served = false;
    }
    // The transaction cache is on only for the cache attribute.
    let tx = facade.transactions_for("moqui.test.TestEntity").unwrap();
    let outcome = tx.run(
        crate::transaction::TransactionAttribute::Cache,
        None,
        || -> Result<(), Error> {
            let mut doomed = facade.make_value("moqui.test.TestEntity").unwrap();
            doomed.set("testId", Value::string("T1"));
            doomed.delete(&facade).unwrap();
            let mut held = facade.find("moqui.test.TestEntity").unwrap().iterator().unwrap();
            assert!(held.next_value().unwrap().is_none());
            drop(held);
            assert!(!tape.lock().unwrap_or_else(|err| err.into_inner()).opened);
            Err(Error::new("roll back"))
        },
    );
    assert!(outcome.is_err());
}

#[test]
fn entity_rest_serves_the_verbs() {
    use std::rc::Rc;

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
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

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moqui-writer-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    let facade = open_fake();

    let err = facade.rest("", "moqui.test.TestEntity", &BTreeMap::new()).unwrap_err();
    assert!(err.to_string().contains("must be specified"), "{err}");
    let err = facade
        .rest("nope", "moqui.test.TestEntity", &BTreeMap::new())
        .unwrap_err();
    assert!(err.to_string().contains("not supported"), "{err}");

    match facade.rest("get", "moqui.test.TestEntity", &BTreeMap::new()).unwrap() {
        Value::List(items) => assert!(items.is_empty()),
        other => panic!("find without a primary key returned {other:?}"),
    }

    let created = facade
        .rest(
            "post",
            "moqui.test.TestEntity",
            &params(vec![
                ("testId", Value::string("C1")),
                ("testMedium", Value::string("made")),
            ]),
        )
        .unwrap();
    assert_eq!(map_str(&created, "testId"), Some("C1"));
    let err = facade
        .rest(
            "find",
            "moqui.test.TestEntity",
            &params(vec![("testId", Value::string("C1"))]),
        )
        .unwrap_err();
    assert!(err.to_string().contains("No value found"), "{err}");

    let err = facade
        .rest("patch", "moqui.test.TestEntity", &BTreeMap::new())
        .unwrap_err();
    assert!(err.to_string().contains("cannot update"), "{err}");
    let updated = facade
        .rest(
            "update",
            "moqui.test.TestEntity",
            &params(vec![
                ("testId", Value::string("C1")),
                ("testMedium", Value::string("updated")),
            ]),
        )
        .unwrap();
    assert_eq!(map_str(&updated, "testId"), Some("C1"));
    let stored = facade
        .rest(
            "put",
            "moqui.test.TestEntity",
            &params(vec![
                ("testId", Value::string("S1")),
                ("testMedium", Value::string("stored")),
            ]),
        )
        .unwrap();
    assert_eq!(map_str(&stored, "testId"), Some("S1"));

    let err = facade
        .rest("delete", "moqui.test.TestEntity", &BTreeMap::new())
        .unwrap_err();
    assert!(err.to_string().contains("Must specify all primary key fields"), "{err}");
    let deleted = facade
        .rest(
            "delete",
            "moqui.test.TestEntity",
            &params(vec![("testId", Value::string("C1"))]),
        )
        .unwrap();
    let Value::Map(deleted_map) = deleted else {
        panic!("delete returned {deleted:?}");
    };
    assert!(deleted_map.is_empty(), "{deleted_map:?}");

    let listed = facade
        .rest(
            "find",
            "moqui.basic.Enumeration",
            &params(vec![(
                "path",
                Value::List(vec![Value::string("E1"), Value::string("type")]),
            )]),
        )
        .unwrap();
    assert!(matches!(listed, Value::List(_)), "{listed:?}");
    let err = facade
        .rest("find", "moqui.basic.Enumeration/E1/type/ET1", &BTreeMap::new())
        .unwrap_err();
    assert!(err.to_string().contains("No value found"), "{err}");
    let err = facade
        .rest(
            "find",
            "moqui.basic.Enumeration",
            &params(vec![(
                "path",
                Value::List(vec![
                    Value::string("E1"),
                    Value::string("type"),
                    Value::string("ET1"),
                    Value::string("more"),
                ]),
            )]),
        )
        .unwrap_err();
    assert!(err.to_string().contains("Only one relationship step"), "{err}");
    let err = facade
        .rest(
            "find",
            "moqui.basic.Enumeration",
            &params(vec![(
                "path",
                Value::List(vec![Value::string("E1"), Value::string("missing")]),
            )]),
        )
        .unwrap_err();
    assert!(err.to_string().contains("No relationship found"), "{err}");
    let err = facade
        .rest(
            "find",
            "moqui.basic.Enumeration",
            &params(vec![
                ("masterNameInPath", Value::Bool(true)),
                ("master", Value::string("missing")),
            ]),
        )
        .unwrap_err();
    assert!(err.to_string().contains("Master definition not found"), "{err}");

    let err = facade.dynamic_view("moqui.test.Foo", &BTreeMap::new()).unwrap_err();
    assert!(err.to_string().contains("member-entity"), "{err}");
    let named = facade.dynamic_view("moqui.test.FooBar", &BTreeMap::new()).unwrap();
    assert!(named.is_empty());
    let built = facade
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
                    "alias-all",
                    value_map(vec![
                        ("entity-alias", Value::string("FOO")),
                        ("exclude", Value::string("fooId")),
                    ]),
                ),
                (
                    "alias",
                    value_map(vec![
                        ("entity-alias", Value::string("BAR")),
                        ("name", Value::string("barRank")),
                        ("field", Value::string("barRank")),
                    ]),
                ),
                ("condition", value_map(vec![("FOO.fooId", Value::string("F1"))])),
                ("order-by", Value::string("-fooText")),
                ("limit", Value::I64(5)),
            ]),
        )
        .unwrap();
    assert!(built.is_empty());
    let related = facade
        .dynamic_view(
            "dynamic.EnumType",
            &params(vec![
                (
                    "member-entity",
                    value_map(vec![
                        ("entity-alias", Value::string("ENUM")),
                        ("entity-name", Value::string("moqui.basic.Enumeration")),
                    ]),
                ),
                (
                    "member-relationship",
                    value_map(vec![
                        ("entity-alias", Value::string("ETP")),
                        ("join-from-alias", Value::string("ENUM")),
                        ("relationship", Value::string("type")),
                    ]),
                ),
                (
                    "alias",
                    Value::List(vec![
                        value_map(vec![
                            ("entity-alias", Value::string("ENUM")),
                            ("name", Value::string("enumId")),
                        ]),
                        value_map(vec![
                            ("entity-alias", Value::string("ETP")),
                            ("name", Value::string("enumTypeId")),
                        ]),
                    ]),
                ),
            ]),
        )
        .unwrap();
    assert!(related.is_empty());

    let err = facade.write_data("moqui.test.Foo", &[]).unwrap_err();
    assert!(
        err.to_string().contains("directory") || err.to_string().contains("file"),
        "{err}"
    );
    let export = temp_dir("all");
    let _export_guard = Guard(export.clone());
    let medium = "a,b & <c>";
    let mut record = BTreeMap::new();
    record.insert("testId".to_string(), Value::string("W1"));
    record.insert("testMedium".to_string(), Value::string(medium));
    record.insert("testLong".to_string(), Value::string("line1\nline2"));
    record.insert("directory".to_string(), Value::string(export.display().to_string()));
    record.insert("prefix".to_string(), Value::string("Pre"));
    facade.write_data("moqui.test.TestEntity", &[record]).unwrap();
    let xml = std::fs::read_to_string(export.join("moqui.test.TestEntity.xml")).unwrap();
    assert!(
        xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<entity-facade-xml>\n"),
        "{xml}"
    );
    assert!(xml.contains("<Premoqui.test.TestEntity "), "{xml}");
    assert!(xml.contains("testId=\"W1\""), "{xml}");
    assert!(xml.contains("testMedium=\"a,b &amp; &lt;c&gt;\""), "{xml}");
    assert!(xml.contains("<testLong><![CDATA[line1\nline2]]></testLong>"), "{xml}");
    assert!(xml.contains("</moqui.test.TestEntity>\n"), "{xml}");
    assert!(xml.contains("</entity-facade-xml>\n"), "{xml}");
    let json = std::fs::read_to_string(export.join("moqui.test.TestEntity.json")).unwrap();
    assert!(json.starts_with("[\n"), "{json}");
    assert!(json.contains("\"testId\": \"W1\""), "{json}");
    assert!(json.contains("\"testMedium\": \"a,b & <c>\""), "{json}");
    assert!(json.contains("\"_entity\": \"moqui.test.TestEntity\""), "{json}");
    assert!(json.ends_with("]\n\n"), "{json}");
    let csv = std::fs::read_to_string(export.join("moqui.test.TestEntity.csv")).unwrap();
    assert!(csv.starts_with("moqui.test.TestEntity,export,testId\n"), "{csv}");
    assert!(csv.contains("\"a,b & <c>\""), "{csv}");
    let err = facade
        .write_data(
            "moqui.test.TestEntity",
            &[params(vec![("directory", Value::string(export.display().to_string()))])],
        )
        .unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    let one = temp_dir("one");
    let _one_guard = Guard(one.clone());
    facade
        .write_data(
            "moqui.test.TestEntity",
            &[params(vec![
                ("testId", Value::string("J1")),
                ("directory", Value::string(one.display().to_string())),
                ("fileType", Value::string("JSON")),
            ])],
        )
        .unwrap();
    assert!(one.join("moqui.test.TestEntity.json").is_file());
    assert!(!one.join("moqui.test.TestEntity.xml").exists());
    assert!(!one.join("moqui.test.TestEntity.csv").exists());

    let csv_file = one.join("out.csv");
    facade
        .write_data(
            "moqui.test.Foo",
            &[params(vec![
                ("fooId", Value::string("F9")),
                ("fooText", Value::string("file")),
                ("file", Value::string(csv_file.display().to_string())),
            ])],
        )
        .unwrap();
    let foo_csv = std::fs::read_to_string(&csv_file).unwrap();
    assert!(foo_csv.starts_with("moqui.test.Foo,export,fooId\n"), "{foo_csv}");
    assert!(foo_csv.contains("file"), "{foo_csv}");

    let err = facade
        .write_data(
            "moqui.test.Foo",
            &[params(vec![
                ("directory", Value::string(export.display().to_string())),
                ("fileType", Value::string("yaml")),
            ])],
        )
        .unwrap_err();
    assert!(err.to_string().contains("fileType"), "{err}");
    let not_dir = one.join("notdir");
    std::fs::write(&not_dir, "x").unwrap();
    let err = facade
        .write_data(
            "moqui.test.Foo",
            &[params(vec![(
                "directory",
                Value::string(not_dir.display().to_string()),
            )])],
        )
        .unwrap_err();
    assert!(err.to_string().contains("not a directory"), "{err}");
}

#[test]
fn foobar_select_view_joins_member_entities() {
    let framework = framework_dir();
    let conf = MNode::parse_file(framework.join("conf/MoquiDefaultConf.xml")).unwrap();
    let model = EntityModel::load_directory(&conf, &framework.join("entity")).unwrap();
    let dialects = Dialects::load(&conf).unwrap();
    let dialect = dialects.get("postgres").unwrap();
    let view = model.entity("moqui.test.FooBar").unwrap();
    let sql = select_view(
        view,
        &model,
        dialect,
        &super::find::FindValues,
        None,
        &[],
        LimitOffset::default(),
    )
    .unwrap()
    .sql;
    assert!(sql.contains("JOIN"), "{sql}");
    assert!(sql.contains("Foo"), "{sql}");
    assert!(sql.contains("Bar"), "{sql}");
    assert!(!sql.contains("member-relationship"), "{sql}");
}

/// What the SQL runner asked of the connection: rows taken from the cursor, and whether it began, closed, or read
/// a query whole.
#[derive(Default)]
struct CursorTape {
    fetched: usize,
    begun: bool,
    closed: bool,
    read_whole: bool,
}

struct CursorDriver {
    tape: Arc<Mutex<CursorTape>>,
}

/// Serves 100 rows through the cursor.
struct CursorConn {
    tape: Arc<Mutex<CursorTape>>,
}

impl DbConnection for CursorConn {
    fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, Error> {
        Ok(0)
    }

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        self.query_once(sql, params)
    }

    fn query_once(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        // SELECT 1 is the pool's check of a new connection.
        if sql != "SELECT 1" {
            self.tape.lock().unwrap_or_else(|err| err.into_inner()).read_whole = true;
        }
        Ok(Vec::new())
    }

    fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
        Ok(())
    }

    fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
        let mut tape = self.tape.lock().unwrap_or_else(|err| err.into_inner());
        if tape.fetched >= 100 {
            return Ok(None);
        }
        tape.fetched += 1;
        let number = i64::try_from(tape.fetched).unwrap_or_default();
        Ok(Some(DbRow {
            columns: vec![("n".to_string(), Value::I64(number))],
        }))
    }

    fn close_stream(&mut self) -> Result<(), Error> {
        self.tape.lock().unwrap_or_else(|err| err.into_inner()).closed = true;
        Ok(())
    }

    fn stream_open(&self) -> bool {
        false
    }

    fn begin(&mut self) -> Result<(), Error> {
        self.tape.lock().unwrap_or_else(|err| err.into_inner()).begun = true;
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

impl DbDriver for CursorDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        Ok(Box::new(CursorConn {
            tape: Arc::clone(&self.tape),
        }))
    }
}

#[test]
fn the_sql_runner_reads_a_query_through_a_cursor_to_one_row_past_the_limit() {
    let tape = Arc::new(Mutex::new(CursorTape::default()));
    let driver: Arc<dyn DbDriver> = Arc::new(CursorDriver {
        tape: Arc::clone(&tape),
    });
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
    let facade = EntityFacade::open(
        &framework_dir(),
        "transactional",
        vec![EntityGroup::new("transactional", "postgres", transactions)],
    )
    .unwrap();

    let result = super::info::run_sql(&facade, "transactional", "select n from numbers;", 5).unwrap();
    assert_eq!(result.columns, vec!["n".to_string()]);
    assert_eq!(result.records.len(), 5);
    assert_eq!(result.records[4], vec![Value::I64(5)]);
    assert!(result.limit_reached);
    {
        let mut tape = tape.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(tape.fetched, 6, "the cursor is read to one row past the limit");
        assert!(tape.begun && tape.closed && !tape.read_whole);
        *tape = CursorTape::default();
    }

    // At or under the limit, every row and no limit reached.
    let result = super::info::run_sql(&facade, "transactional", "SELECT n FROM numbers", 0).unwrap();
    assert_eq!(result.records.len(), 100);
    assert!(!result.limit_reached);
    tape.lock().unwrap_or_else(|err| err.into_inner()).fetched = 0;
    let result = super::info::run_sql(&facade, "transactional", "select n from numbers", 100).unwrap();
    assert_eq!(result.records.len(), 100);
    assert!(!result.limit_reached);

    // A SHOW is read whole: a Postgres cursor does not take one.
    let result = super::info::run_sql(&facade, "transactional", "show search_path", 5).unwrap();
    assert!(result.records.is_empty());
    assert!(tape.lock().unwrap_or_else(|err| err.into_inner()).read_whole);
}
