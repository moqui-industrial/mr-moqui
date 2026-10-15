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

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use moqui_core::driver::{DbDriver, DbRow};
use moqui_core::entity::cache::{enter_sim, exit_sim};
use moqui_core::entity::{EntityFacade, EntityValue};
use moqui_core::transaction::{PoolConfig, TransactionManager};
use moqui_lib::object::Value;

use moqui_host::db::embedded::EmbeddedCluster;
use moqui_host::db::entity_facade::open_entity_facade;
use moqui_host::db::sim::SimOverlay;

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn one_cache_hits_and_sim_write_stays_off_main() {
    let _lock = crate::common::shared();
    let main_dir = temp_cluster_dir("main");
    let overlay_dir = temp_cluster_dir("overlay");
    let _main_guard = DirGuard(main_dir.clone());
    let _overlay_guard = DirGuard(overlay_dir.clone());
    let (main_cluster, main_driver) = EmbeddedCluster::start(&main_dir, 20).unwrap();
    let (overlay_cluster, overlay_driver) = EmbeddedCluster::start(&overlay_dir, 20).unwrap();
    let main_driver: Arc<dyn DbDriver> = Arc::new(main_driver);
    let overlay_driver: Arc<dyn DbDriver> = Arc::new(overlay_driver);
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
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
    let _lock = crate::common::shared();
    let main_dir = temp_cluster_dir("lazy");
    let _main_guard = DirGuard(main_dir.clone());
    let (main_cluster, main_driver) = EmbeddedCluster::start(&main_dir, 20).unwrap();
    let main_driver: Arc<dyn DbDriver> = Arc::new(main_driver);
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
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
