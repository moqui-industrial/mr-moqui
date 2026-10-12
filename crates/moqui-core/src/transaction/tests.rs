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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};

use super::{PoolConfig, TransactionManager};

struct Shared {
    connects: AtomicUsize,
    statements: AtomicUsize,
    fail_at: AtomicUsize,
    fail_test: AtomicBool,
    log: Mutex<Vec<String>>,
    next_id: AtomicU64,
    cancels: Mutex<Vec<Arc<AtomicBool>>>,
}

struct FakeDriver {
    shared: Arc<Shared>,
}

struct FakeConn {
    id: u64,
    shared: Arc<Shared>,
    cancelled: Arc<AtomicBool>,
    streaming: bool,
}

struct FlagCancel(Arc<AtomicBool>);

impl FakeDriver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Shared {
                connects: AtomicUsize::new(0),
                statements: AtomicUsize::new(0),
                fail_at: AtomicUsize::new(0),
                fail_test: AtomicBool::new(false),
                log: Mutex::new(Vec::new()),
                next_id: AtomicU64::new(1),
                cancels: Mutex::new(Vec::new()),
            }),
        })
    }

    fn connects(&self) -> usize {
        self.shared.connects.load(Ordering::SeqCst)
    }

    fn events(&self) -> Vec<String> {
        self.shared.log.lock().expect("log").clone()
    }

    fn event_count(&self, prefix: &str) -> usize {
        self.events().iter().filter(|event| event.starts_with(prefix)).count()
    }

    fn any_cancelled(&self) -> bool {
        self.shared
            .cancels
            .lock()
            .expect("cancels")
            .iter()
            .any(|flag| flag.load(Ordering::SeqCst))
    }
}

impl FakeConn {
    fn touch(&mut self, event: &str) -> Result<(), Error> {
        let n = self.shared.statements.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.log.lock().expect("log").push(format!("sql:{event}"));
        let fail_at = self.shared.fail_at.load(Ordering::SeqCst);
        let fail_test = event == "SELECT 1" && self.shared.fail_test.load(Ordering::SeqCst);
        if fail_test || (fail_at > 0 && n == fail_at) {
            return Err(Error::new(format!("statement failed: {event}")));
        }
        Ok(())
    }

    fn mark(&self, event: &str) {
        self.shared
            .log
            .lock()
            .expect("log")
            .push(format!("{event}:{}", self.id));
    }
}

impl DbCancel for FlagCancel {
    fn cancel(&self) -> Result<(), Error> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl DbConnection for FakeConn {
    fn execute(&mut self, sql: &str, _params: &[Value]) -> Result<u64, Error> {
        self.touch(sql)?;
        Ok(1)
    }

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        self.query_once(sql, params)
    }

    fn query_once(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
        self.touch(sql)?;
        Ok(vec![DbRow {
            columns: vec![("id".to_string(), Value::I64(i64::try_from(self.id).unwrap_or(i64::MAX)))],
        }])
    }

    fn open_stream(&mut self, sql: &str, _params: &[Value]) -> Result<(), Error> {
        self.touch(sql)?;
        self.streaming = true;
        Ok(())
    }

    fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
        Ok(None)
    }

    fn close_stream(&mut self) -> Result<(), Error> {
        self.streaming = false;
        Ok(())
    }

    fn stream_open(&self) -> bool {
        self.streaming
    }

    fn begin(&mut self) -> Result<(), Error> {
        self.mark("begin");
        Ok(())
    }

    fn commit(&mut self) -> Result<(), Error> {
        self.mark("commit");
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), Error> {
        self.mark("rollback");
        Ok(())
    }

    fn savepoint(&mut self, name: &str) -> Result<(), Error> {
        self.mark(&format!("savepoint:{name}"));
        Ok(())
    }

    fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), Error> {
        self.mark(&format!("rollback_to:{name}"));
        Ok(())
    }

    fn release_savepoint(&mut self, name: &str) -> Result<(), Error> {
        self.mark(&format!("release:{name}"));
        Ok(())
    }

    fn canceller(&self) -> Box<dyn DbCancel> {
        Box::new(FlagCancel(Arc::clone(&self.cancelled)))
    }
}

impl DbDriver for FakeDriver {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        self.shared.connects.fetch_add(1, Ordering::SeqCst);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.shared
            .cancels
            .lock()
            .expect("cancels")
            .push(Arc::clone(&cancelled));
        self.shared.log.lock().expect("log").push(format!("connect:{id}"));
        Ok(Box::new(FakeConn {
            id,
            shared: Arc::clone(&self.shared),
            cancelled,
            streaming: false,
        }))
    }
}

struct Clean<'a>(&'a TransactionManager);

impl Drop for Clean<'_> {
    fn drop(&mut self) {
        let _ = self.0.destroy_thread();
    }
}

fn harness(config: PoolConfig) -> (TransactionManager, Arc<FakeDriver>) {
    let driver = FakeDriver::new();
    let mgr = TransactionManager::new(Arc::clone(&driver) as Arc<dyn DbDriver>, config).expect("pool");
    (mgr, driver)
}

fn small() -> PoolConfig {
    PoolConfig {
        min: 0,
        max: 8,
        reserve: 2,
        acquire_timeout: Duration::from_secs(2),
    }
}

fn conn_id(conn: &mut dyn DbConnection) -> Result<i64, Error> {
    let rows = conn.query_once("id", &[])?;
    match rows[0].get("id") {
        Some(Value::I64(id)) => Ok(*id),
        _ => Err(Error::new("id column missing")),
    }
}

fn position(events: &[String], needle: &str) -> usize {
    events.iter().position(|event| event.contains(needle)).expect(needle)
}

#[test]
fn manager_is_send_sync() {
    fn assert_bounds<T: Clone + Send + Sync>() {}
    assert_bounds::<TransactionManager>();
}

#[test]
fn begin_does_not_connect_and_commit_reuses_idle() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    assert!(mgr.begin(None).expect("begin"));
    assert_eq!(driver.connects(), 0);
    assert_eq!(mgr.stats().checkouts, 0);

    mgr.with_connection(|conn| conn.execute("one", &[])).expect("first");
    assert_eq!(driver.connects(), 1);
    assert_eq!(mgr.stats().checkouts, 1);

    mgr.with_connection(|conn| conn.execute("two", &[])).expect("second");
    assert_eq!(driver.connects(), 1);
    assert_eq!(mgr.stats().checkouts, 1);

    mgr.commit().expect("commit");
    assert_eq!(mgr.stats().checked_out, 0);
    assert_eq!(mgr.stats().ordinary_idle, 1);
    assert!(!mgr.in_transaction());

    assert!(mgr.begin(None).expect("second begin"));
    mgr.with_connection(|_| Ok(())).expect("reuse");
    assert_eq!(driver.connects(), 1);
    assert_eq!(mgr.stats().checkouts, 2);
    mgr.commit().expect("second commit");
}

#[test]
fn joined_begin_commit_does_not_commit_the_client() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    let ran = Arc::new(AtomicUsize::new(0));
    assert!(mgr.begin(None).expect("outer"));
    mgr.with_connection(|_| Ok(())).expect("enlist");
    let ran_hook = Arc::clone(&ran);
    mgr.on_commit(move || {
        ran_hook.fetch_add(1, Ordering::SeqCst);
    })
    .expect("hook");
    assert!(!mgr.begin(None).expect("join"));
    mgr.commit().expect("inner commit");
    assert_eq!(driver.event_count("commit:"), 0);
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert!(mgr.in_transaction());
    mgr.commit().expect("outer commit");
    assert_eq!(driver.event_count("commit:"), 1);
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert!(!mgr.in_transaction());
}

#[test]
fn force_new_parks_one_client_and_rejects_a_second() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    assert!(mgr.begin(None).expect("outer"));
    let outer_id = mgr.with_connection(conn_id).expect("outer conn");
    assert!(mgr.force_new(None).expect("force"));
    assert_eq!(mgr.stats().parked, 1);
    assert_eq!(mgr.stats().ordinary_idle, 0);
    let inner_id = mgr.with_connection(conn_id).expect("inner conn");
    assert_ne!(outer_id, inner_id);
    assert_eq!(driver.connects(), 2);
    mgr.commit().expect("inner commit");
    assert_eq!(mgr.stats().parked, 0);
    assert_eq!(mgr.stats().checked_out, 1);
    let resumed = mgr.with_connection(conn_id).expect("resumed");
    assert_eq!(resumed, outer_id);
    assert_eq!(driver.connects(), 2);
    mgr.commit().expect("outer commit");

    assert!(mgr.begin(None).expect("again"));
    mgr.with_connection(|_| Ok(())).expect("hold");
    assert!(mgr.force_new(None).is_ok());
    let err = mgr.force_new(None).expect_err("second park");
    assert!(err.to_string().contains("park budget"), "{err}");
}

#[test]
fn ordinary_checkout_fails_when_only_reserve_is_idle() {
    let (mgr, driver) = harness(PoolConfig {
        min: 0,
        max: 2,
        reserve: 1,
        acquire_timeout: Duration::from_secs(30),
    });
    let _clean = Clean(&mgr);
    assert!(mgr.begin(None).expect("begin"));
    mgr.with_connection(|_| Ok(())).expect("ordinary");
    mgr.with_reserve(|_| Ok(())).expect("reserve");
    assert_eq!(mgr.stats().reserve_idle, 1);
    assert_eq!(mgr.stats().ordinary_idle, 0);
    assert!(mgr.force_new(None).expect("park"));
    assert_eq!(mgr.stats().parked, 1);
    let started = Instant::now();
    let err = mgr.with_connection(|_| Ok(())).expect_err("ordinary must fail");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "waited {:?}",
        started.elapsed()
    );
    assert!(err.to_string().contains("reserve"), "{err}");
    assert_eq!(mgr.stats().reserve_idle, 1);
    assert_eq!(driver.connects(), 2);
}

#[test]
fn check_caps_allows_40_and_rejects_80() {
    let (mgr, _) = harness(PoolConfig::default());
    assert!(mgr.check_caps(10, 10).is_ok());
    let err = mgr.check_caps(20, 20).expect_err("80 > 42");
    assert!(err.to_string().contains("80"), "{err}");
    assert!(err.to_string().contains("42"), "{err}");
}

#[test]
fn statement_error_marks_rollback_only_and_commit_rolls_back() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    driver.shared.fail_at.store(2, Ordering::SeqCst);
    assert!(mgr.begin(None).expect("begin"));
    let err = mgr.with_connection(|conn| conn.execute("boom", &[])).expect_err("boom");
    assert!(err.to_string().contains("boom"), "{err}");
    let next = mgr.with_connection(|_| Ok(())).expect_err("blocked");
    assert!(next.to_string().contains("rollback-only"), "{next}");
    let commit = mgr.commit().expect_err("rollback commit");
    assert!(commit.to_string().contains("rollback-only"), "{commit}");
    assert!(commit.to_string().contains("boom"), "{commit}");
    assert_eq!(driver.event_count("rollback:"), 1);
    assert_eq!(driver.event_count("commit:"), 0);
    assert!(!mgr.in_transaction());
}

#[test]
fn callbacks_run_cache_then_feed_then_commit_then_after() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    let idle_at_after = Arc::new(AtomicBool::new(false));
    assert!(mgr.begin(None).expect("begin"));
    mgr.with_connection(|_| Ok(())).expect("enlist");
    mgr.on_cache_flush(|conn| conn.execute("cache-flush", &[]).map(|_| ()))
        .expect("cache");
    mgr.on_feed_insert(|conn| conn.execute("feed-insert", &[]).map(|_| ()))
        .expect("feed");
    let mgr2 = mgr.clone();
    let flag = Arc::clone(&idle_at_after);
    let log = Arc::clone(&driver.shared);
    mgr.on_commit(move || {
        let stats = mgr2.stats();
        flag.store(stats.checked_out == 0 && stats.ordinary_idle >= 1, Ordering::SeqCst);
        log.log.lock().expect("log").push("after-commit".to_string());
    })
    .expect("after");
    mgr.commit().expect("commit");
    let events = driver.events();
    let cache = position(&events, "sql:cache-flush");
    let feed = position(&events, "sql:feed-insert");
    let commit = position(&events, "commit:");
    let after = position(&events, "after-commit");
    assert!(cache < feed && feed < commit && commit < after, "{events:?}");
    assert!(idle_at_after.load(Ordering::SeqCst));
}

#[test]
fn destroy_thread_commits_open_and_parked_frames() {
    let (mgr, driver) = harness(small());
    assert!(mgr.begin(None).expect("begin"));
    mgr.with_connection(|_| Ok(())).expect("work");
    mgr.destroy_thread().expect("destroy");
    assert!(!mgr.in_transaction());
    assert_eq!(driver.event_count("commit:"), 1);
    assert_eq!(mgr.stats().checked_out, 0);
    assert_eq!(mgr.stats().parked, 0);
    mgr.destroy_thread().expect("empty");

    assert!(mgr.begin(None).expect("outer"));
    mgr.with_connection(|_| Ok(())).expect("outer work");
    assert!(mgr.force_new(None).expect("inner"));
    mgr.with_connection(|_| Ok(())).expect("inner work");
    mgr.destroy_thread().expect("both");
    assert!(!mgr.in_transaction());
    assert_eq!(driver.event_count("commit:"), 3);
    assert_eq!(mgr.stats().parked, 0);
    assert_eq!(mgr.stats().checked_out, 0);
}

#[test]
fn managers_do_not_share_a_stack() {
    let (first, _) = harness(small());
    let (second, _) = harness(small());
    let _clean_first = Clean(&first);
    let _clean_second = Clean(&second);
    assert!(first.begin(None).expect("first"));
    assert!(second.begin(None).expect("second"));
    first.commit().expect("first commit");
    assert!(second.in_transaction());
    second.commit().expect("second commit");
}

#[test]
fn reserve_work_does_not_take_the_enlisted_connection() {
    let (mgr, _) = harness(small());
    let _clean = Clean(&mgr);
    assert!(mgr.begin(None).expect("begin"));
    let enlisted = mgr.with_connection(conn_id).expect("enlist");
    let reserve = mgr
        .with_reserve(|conn| {
            let id = conn_id(conn)?;
            conn.begin()?;
            conn.execute("reserve-work", &[])?;
            conn.commit()?;
            Ok(id)
        })
        .expect("reserve");
    assert_ne!(enlisted, reserve);
    assert_eq!(mgr.stats().reserve_idle, 1);
    assert_eq!(mgr.with_connection(conn_id).expect("still enlisted"), enlisted);
}

#[test]
fn failed_checkout_test_discards_the_idle_connection() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    mgr.with_connection(|_| Ok(())).expect("warm");
    assert_eq!(mgr.stats().ordinary_idle, 1);
    driver.shared.fail_test.store(true, Ordering::SeqCst);
    let err = mgr.with_connection(|_| Ok(())).expect_err("bad test");
    assert!(err.to_string().contains("connection test failed"), "{err}");
    assert_eq!(driver.connects(), 1);
    assert_eq!(mgr.stats().ordinary_idle, 0);
    assert_eq!(mgr.stats().checked_out, 0);
}

#[test]
fn poll_deadline_marks_rollback_only_and_cancels() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    assert!(mgr.begin(Some(Duration::ZERO)).expect("begin"));
    mgr.with_connection(|_| Ok(())).expect("enlist");
    mgr.poll_deadline().expect("poll");
    assert!(driver.any_cancelled());
    let blocked = mgr.with_connection(|_| Ok(())).expect_err("blocked");
    assert!(blocked.to_string().contains("rollback-only"), "{blocked}");
    let err = mgr.commit().expect_err("deadline commit");
    assert!(err.to_string().contains("deadline"), "{err}");
    assert_eq!(driver.event_count("rollback:"), 1);
    assert_eq!(driver.event_count("commit:"), 0);
}

#[test]
fn savepoint_forwards_and_errors_without_a_connection() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    let missing = mgr.savepoint("s").expect_err("no conn");
    assert!(missing.to_string().contains("no connection enlisted"), "{missing}");
    assert!(mgr.begin(None).expect("begin"));
    mgr.with_connection(|_| Ok(())).expect("enlist");
    mgr.savepoint("s").expect("savepoint");
    mgr.rollback_to_savepoint("s").expect("rollback to");
    mgr.release_savepoint("s").expect("release");
    let events = driver.events().join(" ");
    assert!(events.contains("savepoint:s"), "{events}");
    assert!(events.contains("rollback_to:s"), "{events}");
    assert!(events.contains("release:s"), "{events}");
}

#[test]
fn open_stream_reuses_the_enlisted_connection() {
    let (mgr, driver) = harness(small());
    let _clean = Clean(&mgr);
    assert!(mgr.begin(None).expect("begin"));
    mgr.with_connection(|conn| conn.open_stream("stream", &[]))
        .expect("open");
    assert_eq!(driver.connects(), 1);
    mgr.with_connection(|conn| {
        assert!(conn.stream_open());
        Ok(())
    })
    .expect("reuse");
    assert_eq!(driver.connects(), 1);
    assert_eq!(mgr.stats().checkouts, 1);
    let err = mgr.force_new(None).expect_err("stream");
    assert!(err.to_string().contains("open stream"), "{err}");
}
