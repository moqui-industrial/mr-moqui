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

//! The entity database a conf names (A3 of the plan): `entity_ds_db_conf=postgres` loads into that
//! server, here a second embedded cluster reached through its socket directory, and starts no
//! postmaster in the cluster directory. It sets process properties, so it holds the serial lock.

#![cfg(all(unix, feature = "embedded-postgres"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moqui_core::driver::DbDriver;
use moqui_core::transaction::PoolConfig;
use moqui_host::db::embedded::EmbeddedCluster;
use moqui_host::db::entity_facade::open_entity_facade;
use moqui_host::{load_data, LoadFlags};
use moqui_lib::object::Value;

#[test]
fn an_external_postgres_from_the_conf_is_loaded_and_no_postmaster_starts() {
    let _serial = crate::common::serial();
    // The properties name this test's server, and no other test may start on it.
    let _properties = crate::common::PropertyReset::of(&[
        "entity_ds_db_conf",
        "entity_ds_host",
        "entity_ds_port",
        "entity_ds_user",
        "entity_ds_password",
        "entity_ds_database",
    ]);
    let server_dir = temp_dir("moqui-ext");
    let runtime = temp_dir("moqui-ext-rt");
    let cluster_dir = temp_dir("moqui-ext-unused");
    let _guards = (
        DirGuard(server_dir.clone()),
        DirGuard(runtime.clone()),
        DirGuard(cluster_dir.clone()),
    );
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&server_dir).unwrap();
    let (server, driver) = EmbeddedCluster::start(&server_dir, 20).unwrap();
    let password = std::fs::read_to_string(server_dir.join("pgpass")).unwrap();

    for (name, value) in [
        ("entity_ds_db_conf", "postgres"),
        ("entity_ds_host", server.data_dir().to_str().unwrap()),
        ("entity_ds_port", "5432"),
        ("entity_ds_user", "postgres"),
        ("entity_ds_password", password.trim()),
        ("entity_ds_database", "moqui"),
    ] {
        moqui_lib::env::set_property(name, value.to_string());
    }
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    let report = load_data(&runtime, &framework, &cluster_dir, &LoadFlags::default()).unwrap();
    assert!(report.enumeration > 0, "enumeration {}", report.enumeration);
    assert!(
        !cluster_dir.join("data").exists(),
        "an external datasource started a postmaster in {}",
        cluster_dir.display()
    );

    let mut conn = driver.connect().unwrap();
    let rows = conn.query("SELECT COUNT(*) AS n FROM enumeration", &[]).unwrap();
    let count = match rows[0].get("n") {
        Some(Value::I64(n)) => *n,
        other => panic!("count {other:?}"),
    };
    assert_eq!(count, report.enumeration);
    drop(conn);
    drop(server);
}

/// A foreign key is found by its columns, whatever its name, as Java's `foreignKeyExists` (P3-21): after the key is
/// renamed, as MySQL names a `name_fk` key `{table}_ibfk_N`, the check of a new facade adds no second one.
#[test]
fn a_foreign_key_under_another_name_is_not_added_again() {
    let _lock = crate::common::shared();
    let dir = temp_dir("moqui-fk");
    let _guard = DirGuard(dir.clone());
    std::fs::create_dir_all(&dir).unwrap();
    let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    let pool = || PoolConfig {
        min: 0,
        max: 4,
        reserve: 1,
        acquire_timeout: Duration::from_secs(30),
        ..Default::default()
    };
    let names = ["moqui.basic.EnumerationType", "moqui.basic.Enumeration"];
    let facade = open_entity_facade(&framework, Arc::clone(&driver), pool()).unwrap();
    facade.check_tables(&names).unwrap();
    drop(facade);

    let keys = || {
        let mut conn = driver.connect().unwrap();
        let rows = conn
            .query(
                "SELECT c.conname AS name FROM pg_constraint c JOIN pg_class cl ON cl.oid = c.conrelid \
                 JOIN pg_class rf ON rf.oid = c.confrelid \
                 WHERE c.contype = 'f' AND cl.relname = 'enumeration' AND rf.relname = 'enumeration_type'",
                &[],
            )
            .unwrap();
        rows.iter()
            .filter_map(|row| row.get("name").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>()
    };
    let before = keys();
    assert_eq!(before.len(), 1, "{before:?}");
    let mut conn = driver.connect().unwrap();
    conn.execute(
        &format!(
            "ALTER TABLE enumeration RENAME CONSTRAINT \"{}\" TO enumeration_ibfk_1",
            before[0]
        ),
        &[],
    )
    .unwrap();
    drop(conn);

    let facade = open_entity_facade(&framework, Arc::clone(&driver), pool()).unwrap();
    facade.check_tables(&names).unwrap();
    drop(facade);
    assert_eq!(keys(), ["enumeration_ibfk_1"]);
    drop(cluster);
}

/// P3-20 on Postgres: a `LIKE` on a `text-fts` field runs as the full-text search of its generated column, every word
/// of the pattern must be in the text, and a `NOT LIKE` is its complement.
#[test]
fn like_on_a_text_fts_field_finds_by_word_on_postgres() {
    let _lock = crate::common::shared();
    let dir = temp_dir("moqui-fts");
    let _guard = DirGuard(dir.clone());
    std::fs::create_dir_all(&dir).unwrap();
    let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    let facade = open_entity_facade(
        &framework,
        Arc::clone(&driver),
        PoolConfig {
            min: 0,
            max: 4,
            reserve: 1,
            acquire_timeout: Duration::from_secs(30),
            ..Default::default()
        },
    )
    .unwrap();
    facade.check_tables(&["moqui.test.TestFts"]).unwrap();
    use moqui_core::entity::Condition;
    for (id, body) in [
        ("F1", "The big warehouse on the river"),
        ("F2", "A small warehouse"),
        ("F3", "Big boats"),
    ] {
        let mut row = facade.make_value("moqui.test.TestFts").unwrap();
        row.set("testFtsId", Value::string(id));
        row.set("body", Value::string(body));
        row.create(&facade).unwrap();
    }
    let ids = |condition: Condition| {
        let mut ids: Vec<String> = facade
            .find("moqui.test.TestFts")
            .unwrap()
            .condition(condition)
            .list()
            .unwrap()
            .iter()
            .filter_map(|row| row.get("testFtsId").and_then(Value::as_str).map(str::to_string))
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(Condition::like("body", Value::string("%warehouse%"))), ["F1", "F2"]);
    assert_eq!(ids(Condition::like("body", Value::string("%BIG warehouse%"))), ["F1"]);
    assert_eq!(ids(Condition::not_like("body", Value::string("%warehouse%"))), ["F3"]);
    // a prefix-only pattern stays SQL LIKE, case and all
    assert_eq!(ids(Condition::like("body", Value::string("Big%"))), ["F3"]);
    drop(facade);
    drop(cluster);
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{prefix}-{}-{nanos}", std::process::id());
    if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp").join(name)
    } else {
        std::env::temp_dir().join(name)
    }
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
