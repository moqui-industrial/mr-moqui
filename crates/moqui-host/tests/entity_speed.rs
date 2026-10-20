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

//! Entity facade timings, called directly. No screen, no QuickJS, no service layer.
//!
//! Out of the default `cargo test` link, and ignored so naming the binary does not start a postmaster.
//! Run it optimized. A dev-profile binary is a different program.
//!
//! ```text
//! CARGO_PROFILE_RELEASE_DEBUG=1 cargo test --release -p moqui-host --test entity_speed -- --ignored --nocapture
//! ```
//!
//! `ENTITY_SPEED_CALLS` is the loop size (default 1000). `ENTITY_SPEED_BATCHES` is how many timed
//! batches to keep (default 5). The printed per-call figures use the upper median batch.
//!
//! There is no `ExecutionContext`, so artifact authorization and hit counting do not run. `fsync` and
//! synchronous commit stay on. The pool is one ordinary connection and one reserve connection (sequence
//! refill). A connection returned within the last second is checked out again without `SELECT 1`.
//!
//! Two modes:
//!
//! - **one transaction** matches `SpeedTest.xml` (`begin-transaction="true"`): the batch holds one
//!   connection and commits once, after the timed rows. That commit is its own row, for the whole batch.
//! - **autocommit** is the durable rate: each call checks out a connection, runs its statement, and commits.

#![cfg(all(unix, feature = "embedded-postgres"))]

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moqui_core::driver::DbDriver;
use moqui_core::entity::{EntityFacade, EntityValue};
use moqui_core::transaction::PoolConfig;
use moqui_host::db::embedded::EmbeddedCluster;
use moqui_host::db::entity_facade::open_entity_facade;
use moqui_lib::cache::{CacheConfig, MCache};
use moqui_lib::object::Value;

const ENUM: &str = "moqui.basic.Enumeration";
const ENUM_TYPE: &str = "moqui.basic.EnumerationType";
const TEST: &str = "moqui.test.TestEntity";
const WARMUP: usize = 32;

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy)]
enum Mode {
    /// One frame for the batch. Commit is timed on its own, after the operations.
    Transaction,
    /// No frame. Each statement autocommits, including the pool checkout test.
    Autocommit,
}

struct Row {
    operation: String,
    calls: u64,
    batches: Vec<Duration>,
}

#[test]
#[ignore = "entity timings: cargo test --release -p moqui-host --test entity_speed -- --ignored --nocapture"]
fn entity_facade_speed() {
    if cfg!(debug_assertions) {
        eprintln!(
            "entity_speed: this binary has debug assertions, so it is not a release profile. \
             The numbers are not comparable to ./moq build."
        );
    }
    let calls = env_usize("ENTITY_SPEED_CALLS", 1000);
    let batches = env_usize("ENTITY_SPEED_BATCHES", 5);

    let dir = temp_dir();
    let _guard = DirGuard(dir.clone());
    let (cluster, driver) = EmbeddedCluster::start(&dir, 20).expect("start embedded Postgres");
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    let facade = open_entity_facade(&framework, driver, pool()).expect("open entity facade");
    facade
        .check_tables(&[ENUM_TYPE, ENUM, TEST, "moqui.entity.SequenceValueItem"])
        .expect("check tables");
    seed(&facade);
    let now = Value::Timestamp(jiff::Timestamp::now());
    let cache: MCache<String, String> = MCache::new("entity-speed", CacheConfig::default());

    println!();
    println!(
        "entity facade, {calls} calls x {batches} batches, warmup {WARMUP}. \
         Pool is 1 ordinary + 1 reserve. No ExecutionContext."
    );
    run_mode(&facade, &cache, &now, Mode::Transaction, calls, batches);
    run_mode(&facade, &cache, &now, Mode::Autocommit, calls, batches);

    drop(facade);
    drop(cluster);
}

fn run_mode(
    facade: &EntityFacade,
    cache: &MCache<String, String>,
    now: &Value,
    mode: Mode,
    calls: usize,
    batches: usize,
) {
    let label = match mode {
        Mode::Transaction => "one transaction (commit is the last row, once for the whole batch)",
        Mode::Autocommit => "autocommit (each call checks out and commits; a just-returned connection skips SELECT 1)",
    };
    println!("\n== {label}");
    one_batch(facade, cache, now, mode, WARMUP, 0, None);
    let mut rows = Vec::new();
    for batch in 1..=batches {
        eprintln!("  batch {batch}/{batches}");
        one_batch(facade, cache, now, mode, calls, batch, Some(&mut rows));
    }
    print_rows(&rows);
}

fn one_batch(
    facade: &EntityFacade,
    cache: &MCache<String, String>,
    now: &Value,
    mode: Mode,
    calls: usize,
    batch: usize,
    mut record: Option<&mut Vec<Row>>,
) {
    if matches!(mode, Mode::Transaction) {
        facade.begin(TEST, None).expect("begin");
        // Enlist and BEGIN before the timer, so the first timed call is not the checkout.
        let _ = find_one(facade, "_NA_", false);
    }

    note(
        &mut record,
        "build Enumeration find",
        calls,
        time(|| {
            for i in 0..calls {
                let find = facade
                    .find(ENUM)
                    .expect("find")
                    .eq("enumId", Value::string(format!("_NA_{i}")));
                black_box(find);
            }
        }),
    );

    let keys = note_keys(
        &mut record,
        "MCache put",
        calls,
        time_keys(calls, |i| {
            let key = format!("speed-{batch}-{i}");
            cache.put(key.clone(), "_NA_".to_string()).expect("cache put");
            key
        }),
    );
    note(
        &mut record,
        "MCache get",
        calls,
        time(|| {
            for key in &keys {
                black_box(cache.get(key).expect("cache get"));
            }
        }),
    );

    note(
        &mut record,
        "one PK Enumeration, entity cache",
        calls,
        time(|| {
            for _ in 0..calls {
                black_box(find_one(facade, "_NA_", true));
            }
        }),
    );
    note(
        &mut record,
        "list Enumeration by type, entity cache",
        calls,
        time(|| {
            for _ in 0..calls {
                black_box(find_list(facade, true));
            }
        }),
    );
    note(
        &mut record,
        "one PK Enumeration, database",
        calls,
        time(|| {
            for _ in 0..calls {
                black_box(find_one(facade, "_NA_", false));
            }
        }),
    );
    note(
        &mut record,
        "list Enumeration by type, database",
        calls,
        time(|| {
            for _ in 0..calls {
                black_box(find_list(facade, false));
            }
        }),
    );

    let mut sequenced = Vec::with_capacity(calls);
    note(
        &mut record,
        "create TestEntity, sequenced id",
        calls,
        time(|| {
            for i in 0..calls {
                sequenced.push(create_sequenced(facade, i, now));
            }
        }),
    );
    let mut preset = Vec::with_capacity(calls);
    note(
        &mut record,
        "create TestEntity, preset id",
        calls,
        time(|| {
            for i in 0..calls {
                preset.push(create_preset(facade, batch, i, now));
            }
        }),
    );
    let written = sequenced.len() + preset.len();
    note(
        &mut record,
        "update TestEntity",
        written,
        time(|| {
            for value in sequenced.iter_mut().chain(preset.iter_mut()) {
                value.set("testMedium", Value::string("Updated"));
                value.update(facade).expect("update");
            }
        }),
    );
    note(
        &mut record,
        "delete TestEntity",
        written,
        time(|| {
            for value in sequenced.iter().chain(preset.iter()) {
                value.delete(facade).expect("delete");
            }
        }),
    );

    if matches!(mode, Mode::Transaction) {
        note(
            &mut record,
            "commit whole batch",
            1,
            time(|| {
                facade.commit(TEST).expect("commit");
            }),
        );
    }
}

fn note(record: &mut Option<&mut Vec<Row>>, operation: &str, calls: usize, elapsed: Duration) {
    let Some(rows) = record else {
        return;
    };
    if let Some(row) = rows.iter_mut().find(|row| row.operation == operation) {
        row.batches.push(elapsed);
        return;
    }
    rows.push(Row {
        operation: operation.to_string(),
        calls: u64::try_from(calls).unwrap_or(u64::MAX),
        batches: vec![elapsed],
    });
}

fn note_keys(
    record: &mut Option<&mut Vec<Row>>,
    operation: &str,
    calls: usize,
    timed: (Duration, Vec<String>),
) -> Vec<String> {
    note(record, operation, calls, timed.0);
    timed.1
}

fn time(body: impl FnOnce()) -> Duration {
    let started = Instant::now();
    body();
    started.elapsed()
}

fn time_keys(calls: usize, mut body: impl FnMut(usize) -> String) -> (Duration, Vec<String>) {
    let mut keys = Vec::with_capacity(calls);
    let started = Instant::now();
    for i in 0..calls {
        keys.push(body(i));
    }
    (started.elapsed(), keys)
}

fn print_rows(rows: &[Row]) {
    println!(
        "  {:<44} {:>8} {:>10} {:>10} {:>12} {:>16}",
        "operation", "calls", "median ms", "us/call", "calls/s", "min-max ms"
    );
    for row in rows {
        let mut batches = row.batches.clone();
        batches.sort();
        let median = batches[batches.len() / 2];
        let seconds = median.as_secs_f64();
        let calls = row.calls as f64;
        let per_call_us = if calls == 0.0 { 0.0 } else { seconds * 1.0e6 / calls };
        let per_second = if seconds == 0.0 { 0.0 } else { calls / seconds };
        println!(
            "  {:<44} {:>8} {:>10.3} {:>10.1} {:>12.0} {:>7.3}-{:<7.3}",
            row.operation,
            row.calls,
            seconds * 1.0e3,
            per_call_us,
            per_second,
            batches[0].as_secs_f64() * 1.0e3,
            batches.last().unwrap_or(&median).as_secs_f64() * 1.0e3,
        );
    }
}

fn find_one(facade: &EntityFacade, id: &str, cache: bool) -> EntityValue {
    facade
        .find(ENUM)
        .expect("enumeration find")
        .eq("enumId", Value::string(id))
        .use_cache(cache)
        .one()
        .expect("enumeration one")
        .unwrap_or_else(|| panic!("no Enumeration {id}"))
}

fn find_list(facade: &EntityFacade, cache: bool) -> usize {
    let list = facade
        .find(ENUM)
        .expect("enumeration list")
        .eq("enumTypeId", Value::string("EntitySyncArtifactAppl"))
        .use_cache(cache)
        .list()
        .expect("enumeration list");
    assert_eq!(list.len(), 2, "EntitySyncArtifactAppl rows");
    list.len()
}

fn create_sequenced(facade: &EntityFacade, index: usize, now: &Value) -> EntityValue {
    let mut value = facade.make_value(TEST).expect("make TestEntity");
    fill(&mut value, index, now);
    value.set_sequenced_id_primary(facade).expect("sequenced id");
    value.create(facade).expect("create sequenced");
    value
}

fn create_preset(facade: &EntityFacade, batch: usize, index: usize, now: &Value) -> EntityValue {
    let mut value = facade.make_value(TEST).expect("make TestEntity");
    value.set("testId", Value::string(format!("s{batch}-{index}")));
    fill(&mut value, index, now);
    value.create(facade).expect("create preset");
    value
}

fn fill(value: &mut EntityValue, index: usize, now: &Value) {
    value.set("testMedium", Value::string(format!("Medium {index}")));
    value.set("testLong", Value::string(format!("Long {index}")));
    value.set("testIndicator", Value::string("Y"));
    value.set("testDateTime", now.clone());
    value.set(
        "testNumberInteger",
        Value::I64(i64::try_from(index).unwrap_or(i64::MAX)),
    );
    value.set("testNumberDecimal", Value::string(format!("{index}.5")));
}

fn seed(facade: &EntityFacade) {
    facade.begin(ENUM, None).expect("begin seed");
    enum_type(facade, "_NA_", "Not Applicable");
    enum_type(
        facade,
        "EntitySyncArtifactAppl",
        "Entity Sync Artifact Application Type",
    );
    enumeration(facade, "_NA_", "_NA_", "Not Applicable");
    enumeration(facade, "EsaaInclude", "EntitySyncArtifactAppl", "Include");
    enumeration(facade, "EsaaExclude", "EntitySyncArtifactAppl", "Exclude");
    facade.commit(ENUM).expect("commit seed");
    let _ = find_one(facade, "_NA_", false);
    assert_eq!(find_list(facade, false), 2);
}

fn enum_type(facade: &EntityFacade, id: &str, description: &str) {
    let mut row = facade.make_value(ENUM_TYPE).expect("make EnumerationType");
    row.set("enumTypeId", Value::string(id));
    row.set("description", Value::string(description));
    row.create(facade).expect("create EnumerationType");
}

fn enumeration(facade: &EntityFacade, id: &str, type_id: &str, description: &str) {
    let mut row = facade.make_value(ENUM).expect("make Enumeration");
    row.set("enumId", Value::string(id));
    row.set("enumTypeId", Value::string(type_id));
    row.set("description", Value::string(description));
    row.create(facade).expect("create Enumeration");
}

fn pool() -> PoolConfig {
    PoolConfig {
        min: 1,
        max: 2,
        reserve: 1,
        acquire_timeout: Duration::from_secs(30),
        ..Default::default()
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|text| text.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    PathBuf::from("/tmp").join(format!("entity-speed-{}-{nanos}", std::process::id()))
}
