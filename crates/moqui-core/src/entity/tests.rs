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
use std::sync::Arc;
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

fn open_fake() -> EntityFacade {
    let driver: Arc<dyn DbDriver> = Arc::new(FakeDriver);
    let transactions = TransactionManager::new(
        driver,
        PoolConfig {
            min: 0,
            max: 2,
            reserve: 1,
            acquire_timeout: Duration::from_secs(5),
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
fn entity_rest_is_not_implemented() {
    let facade = open_fake();
    let err = facade
        .rest("find", "moqui.test.TestEntity", &BTreeMap::new())
        .unwrap_err();
    assert!(err.to_string().contains("not implemented"), "{err}");
    let err = facade.dynamic_view("moqui.test.Foo", &BTreeMap::new()).unwrap_err();
    assert!(err.to_string().contains("not implemented"), "{err}");
    let err = facade.write_data("moqui.test.Foo", &[]).unwrap_err();
    assert!(err.to_string().contains("not implemented"), "{err}");
}

#[test]
fn foobar_select_view_joins_member_entities() {
    let framework = framework_dir();
    let conf = MNode::parse_file(framework.join("conf/MoquiDefaultConf.xml")).unwrap();
    let model = EntityModel::load_directory(&conf, &framework.join("entity")).unwrap();
    let dialects = Dialects::load(&conf).unwrap();
    let dialect = dialects.get("postgres").unwrap();
    let view = model.entity("moqui.test.FooBar").unwrap();
    let sql = select_view(view, &model, dialect, None, &[], LimitOffset::default()).unwrap();
    assert!(sql.contains("JOIN"), "{sql}");
    assert!(sql.contains("Foo"), "{sql}");
    assert!(sql.contains("Bar"), "{sql}");
    assert!(!sql.contains("member-relationship"), "{sql}");
}
