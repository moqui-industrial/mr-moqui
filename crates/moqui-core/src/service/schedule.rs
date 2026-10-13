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

//! Cron jobs for one [`ServicePools`](super::ServicePools). The scheduler only enqueues.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use croner::errors::CronError;
use croner::parser::{CronParser, Seconds};
use croner::{CivilDateTime, Cron, CronDateTime, Resolution, Weekday};
use moqui_lib::object::Value;

use super::pool::{capture_factory, lock_mutex, Shared, Work};
use crate::context::{Error, ExecutionContext};

const SLICE: Duration = Duration::from_millis(250);

pub(crate) struct CronJob {
    pub name: String,
    pub cron: Cron,
    pub service: String,
    pub parameters: BTreeMap<String, Value>,
    pub factory: Arc<crate::context::ExecutionContextFactory>,
    pub next_fire: i64,
}

/// No-op entry retained for `service::schedule`. Does not start threads or read the factory.
pub fn schedule(name: &str) -> Result<(), Error> {
    let _ = name;
    let _ = ExecutionContext::current();
    Ok(())
}

impl super::ServicePools {
    /// Register a cron job on this pool. The scheduler enqueues it when due.
    pub fn add_job(
        &self,
        name: &str,
        cron: &str,
        service: &str,
        parameters: &BTreeMap<String, Value>,
    ) -> Result<(), Error> {
        let shared = self
            .shared
            .as_ref()
            .ok_or_else(|| Error::new("service worker pool is not running"))?;
        let factory = capture_factory()?;
        let cron = parse_cron(cron)?;
        let next_fire = following(&cron, unix_now(), true);
        {
            let mut state = lock_mutex(&shared.state);
            if state.stop {
                return Err(Error::new("service worker pool is not running"));
            }
            state.jobs.push(CronJob {
                name: name.to_string(),
                cron,
                service: service.to_string(),
                parameters: parameters.clone(),
                factory,
                next_fire,
            });
        }
        shared.wake.notify_all();
        Ok(())
    }

    /// One scheduler pass: enqueue a free slot, or leave a due job due.
    pub fn tick(&self) {
        if let Some(shared) = &self.shared {
            dispatch_and_wake(shared);
        }
    }

    /// Completed runs of a named job. Increments after the worker's sync returns Ok.
    pub fn job_runs(&self, name: &str) -> u32 {
        let Some(shared) = &self.shared else {
            return 0;
        };
        lock_mutex(&shared.state).runs.get(name).copied().unwrap_or(0)
    }
}

pub(crate) fn scheduler_loop(shared: Arc<Shared>) {
    loop {
        if schedule_slice(&shared).is_none() {
            return;
        }
        let state = lock_mutex(&shared.state);
        if state.stop {
            return;
        }
        let (state, _) = shared
            .wake
            .wait_timeout(state, SLICE)
            .unwrap_or_else(|err| err.into_inner());
        drop(state);
    }
}

pub(crate) fn parse_cron(pattern: &str) -> Result<Cron, Error> {
    CronParser::builder()
        .seconds(Seconds::Optional)
        .alternative_weekdays(true)
        .sloppy_ranges(true)
        .build()
        .parse(pattern)
        .map_err(|err| Error::new(format!("cron pattern: {err}")))
}

fn schedule_slice(shared: &Shared) -> Option<()> {
    let queued = {
        let mut state = lock_mutex(&shared.state);
        if state.stop {
            return None;
        }
        dispatch(&mut state, shared.queue_cap)
    };
    if queued {
        shared.wake.notify_all();
    }
    let hook = lock_mutex(&shared.tick_hook).clone();
    if let Some(hook) = hook {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook())).is_err() {
            tracing::warn!("scheduler tick hook panicked");
        }
    }
    Some(())
}

fn dispatch_and_wake(shared: &Shared) {
    let _ = schedule_slice(shared);
}

fn dispatch(state: &mut super::pool::State, cap: usize) -> bool {
    let oneshots = drain_oneshots(state, cap);
    let jobs = enqueue_due(state, cap);
    oneshots || jobs
}

fn drain_oneshots(state: &mut super::pool::State, cap: usize) -> bool {
    let mut queued = false;
    while state.queue.len() < cap {
        let Some(shot) = state.oneshots.pop_front() else {
            break;
        };
        state
            .queue
            .push_back(Work::service(shot.name, shot.parameters, shot.factory, None));
        queued = true;
    }
    queued
}

fn enqueue_due(state: &mut super::pool::State, cap: usize) -> bool {
    let now = unix_now();
    let mut queued = false;
    for job in &mut state.jobs {
        if job.next_fire > now {
            continue;
        }
        if state.queue.len() >= cap {
            break;
        }
        state.queue.push_back(Work::service(
            job.service.clone(),
            job.parameters.clone(),
            Arc::clone(&job.factory),
            Some(job.name.clone()),
        ));
        job.next_fire = following(&job.cron, now, false).max(now.saturating_add(1));
        queued = true;
    }
    queued
}

fn following(cron: &Cron, now: i64, inclusive: bool) -> i64 {
    match cron.find_next_occurrence(&UtcSec(now), inclusive) {
        Ok(next) => next.0,
        Err(_) => i64::MAX,
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[derive(Clone, Copy)]
struct UtcSec(i64);

impl CronDateTime for UtcSec {
    fn to_civil(&self) -> CivilDateTime {
        civil_from_unix(self.0)
    }

    fn civil_weekday(&self) -> Weekday {
        let days = self.0.div_euclid(86_400);
        let index = (days + 4).rem_euclid(7);
        Weekday::from_days_from_sunday(u32::try_from(index).unwrap_or(4))
    }

    fn resolve_civil(&self, civil: CivilDateTime) -> Result<Resolution<Self>, CronError> {
        Ok(Resolution::Single(UtcSec(unix_from_civil(civil)?)))
    }

    fn checked_add_seconds(&self, seconds: i64) -> Option<Self> {
        self.0.checked_add(seconds).map(UtcSec)
    }

    fn cmp_instant(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

fn civil_from_unix(secs: i64) -> CivilDateTime {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = u32::try_from(sod / 3_600).unwrap_or(0);
    let minute = u32::try_from((sod % 3_600) / 60).unwrap_or(0);
    let second = u32::try_from(sod % 60).unwrap_or(0);
    CivilDateTime::from_ymd_hms(year, month, day, hour, minute, second)
        .unwrap_or_else(|_| CivilDateTime::from_ymd_hms(1970, 1, 1, 0, 0, 0).unwrap_or_else(|_| unreachable!("epoch")))
}

fn unix_from_civil(civil: CivilDateTime) -> Result<i64, CronError> {
    let days = days_from_civil(civil.year(), civil.month(), civil.day());
    let sod = i64::from(civil.hour()) * 3_600 + i64::from(civil.minute()) * 60 + i64::from(civil.second());
    days.checked_mul(86_400)
        .and_then(|base| base.checked_add(sod))
        .ok_or(CronError::InvalidDate)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (
        i32::try_from(year).unwrap_or(1970),
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let mut y = i64::from(year);
    if month <= 2 {
        y -= 1;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = u64::try_from(y - era * 400).unwrap_or(0);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * u64::from(mp) + 2) / 5 + u64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + i64::try_from(doe).unwrap_or(0) - 719_468
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use moqui_lib::object::Value;

    use croner::CronDateTime;

    use super::super::pool::ServicePools;
    use super::{parse_cron, schedule};
    use crate::context::ExecutionContextFactory;

    #[test]
    fn quartz_patterns_parse_and_describe() {
        schedule("poll#EmailServer").unwrap();
        parse_cron("0 0/15 * * * ?").unwrap();
        let nightly = parse_cron("0 0 2 * * ?").unwrap();
        assert!(!nightly.describe().is_empty());
        let every = parse_cron("* * * * * *").unwrap();
        let now = super::UtcSec(super::unix_now());
        let next = every.find_next_occurrence(&now, true).unwrap();
        assert!(next.0 >= now.0 && next.0 - now.0 <= 1);
        let epoch = super::UtcSec(0);
        let civil = epoch.to_civil();
        assert_eq!((civil.year(), civil.month(), civil.day()), (1970, 1, 1));
        assert_eq!(epoch.civil_weekday(), croner::Weekday::Thursday);
    }

    #[test]
    fn call_future_returns_the_out_parameter() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        register_echo();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(mem::open());
        let _entered = ctx.enter();
        let pools = ServicePools::bounded(2, 8);
        let mut params = BTreeMap::new();
        params.insert("note".into(), Value::string("hello"));
        let out = pools.call_future("echo#Note", &params).unwrap();
        assert_eq!(out.get("note").and_then(Value::as_str), Some("hello"));
    }

    #[test]
    fn full_queue_call_future_returns_immediately() {
        let ctx = ExecutionContextFactory::test_context();
        let _entered = ctx.enter();
        let pools = ServicePools::bounded(1, 1);
        let hold = Hold::new();
        let task_hold = hold.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        pools
            .submit_task(move || {
                started_tx.send(()).unwrap();
                task_hold.wait();
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        pools.submit_task(|| {}).unwrap();
        let started = Instant::now();
        let err = pools.call_future("echo#Note", &BTreeMap::new()).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1), "call_future blocked");
        assert!(err.to_string().contains("queue"), "{err}");
    }

    #[test]
    fn every_second_job_reaches_one_run() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        register_echo();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(mem::open());
        let _entered = ctx.enter();
        let pools = ServicePools::bounded(2, 8);
        let mut params = BTreeMap::new();
        params.insert("note".into(), Value::string("pulse"));
        pools.add_job("pulse", "* * * * * *", "echo#Note", &params).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pools.job_runs("pulse") < 1 {
            assert!(Instant::now() < deadline, "scheduler did not run the service");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn tick_leaves_a_due_job_when_the_queue_is_full() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        register_echo();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(mem::open());
        let _entered = ctx.enter();
        let pools = ServicePools::bounded(1, 1);
        let hold = Hold::new();
        let task_hold = hold.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        pools
            .submit_task(move || {
                started_tx.send(()).unwrap();
                task_hold.wait();
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        pools.submit_task(|| {}).unwrap();
        let mut params = BTreeMap::new();
        params.insert("note".into(), Value::string("later"));
        pools.add_job("later", "* * * * * *", "echo#Note", &params).unwrap();
        pools.tick();
        assert_eq!(pools.job_runs("later"), 0);
        hold.open();
        let deadline = Instant::now() + Duration::from_secs(3);
        while pools.job_runs("later") < 1 {
            pools.tick();
            assert!(Instant::now() < deadline, "tick did not submit the due job");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn one_worker_call_future_uses_a_reserved_thread() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        register_echo();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(mem::open());
        let factory = Arc::clone(ctx.factory());
        let pools = ServicePools::bounded(1, 1);
        let shared = pools.shared_handle();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        pools
            .submit_task(move || {
                let worker_ctx = factory.new_context();
                let _entered = worker_ctx.enter();
                let mut params = BTreeMap::new();
                params.insert("note".into(), Value::string("nested"));
                let out = shared.call_future("echo#Note", &params).unwrap();
                done_tx.send(out).unwrap();
            })
            .unwrap();
        let out = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reserved call finished");
        assert_eq!(out.get("note").and_then(Value::as_str), Some("nested"));
    }

    #[test]
    fn dropping_idle_does_not_stop_another_pool() {
        let pools = ServicePools::bounded(1, 4);
        drop(ServicePools::idle());
        let (tx, rx) = std::sync::mpsc::channel();
        pools.submit_task(move || tx.send(()).unwrap()).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn call_future_without_a_context_names_it() {
        let pools = ServicePools::bounded(1, 1);
        let err = pools.call_future("echo#Note", &BTreeMap::new()).unwrap_err();
        assert!(err.to_string().contains("execution context"), "{err}");
    }

    #[test]
    fn default_conf_sizes_and_drop_joins() {
        let text = std::fs::read_to_string(framework_dir().join("conf/MoquiDefaultConf.xml")).unwrap();
        let pools = super::super::start_pools(&text).unwrap();
        assert_eq!(pools.sizes(), (32, 65_535));
        drop(pools);
        let missing = super::super::start_pools("<moqui-conf/>").unwrap();
        assert_eq!(missing.sizes(), (32, 65_535));
        drop(missing);
        let junk = super::super::start_pools(
            r#"<moqui-conf><tools worker-pool-max="nope" worker-queue="nope"/></moqui-conf>"#,
        )
        .unwrap();
        assert_eq!(junk.sizes(), (32, 65_535));
        drop(junk);
        let zero =
            super::super::start_pools(r#"<moqui-conf><tools worker-pool-max="0" worker-queue="0"/></moqui-conf>"#)
                .unwrap();
        assert_eq!(zero.sizes(), (0, 0));
        drop(zero);
    }

    fn register_echo() {
        let path = std::env::temp_dir().join(format!("moqui-svc-echo-{}.xml", std::process::id()));
        std::fs::write(
            &path,
            r#"<services>
                <service verb="echo" noun="Note" authenticate="false" transaction="ignore">
                    <in-parameters><parameter name="note"/></in-parameters>
                    <out-parameters><parameter name="note"/></out-parameters>
                </service>
            </services>"#,
        )
        .unwrap();
        let failures = crate::service::define::lint(std::slice::from_ref(&path)).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[derive(Clone)]
    struct Hold {
        pair: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Hold {
        fn new() -> Self {
            Self {
                pair: Arc::new((Mutex::new(false), Condvar::new())),
            }
        }

        fn wait(&self) {
            let (lock, cv) = &*self.pair;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = cv.wait(open).unwrap();
            }
        }

        fn open(&self) {
            let (lock, cv) = &*self.pair;
            let mut open = lock.lock().unwrap();
            *open = true;
            cv.notify_all();
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            self.open();
        }
    }

    fn framework_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
    }

    mod mem {
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::time::Duration;

        use moqui_lib::error::Error;
        use moqui_lib::object::Value;

        use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
        use crate::entity::{EntityFacade, EntityGroup};
        use crate::transaction::{PoolConfig, TransactionManager};

        pub(super) fn open() -> EntityFacade {
            let driver: Arc<dyn DbDriver> = Arc::new(StubDriver);
            let transactions = TransactionManager::new(
                driver,
                PoolConfig {
                    min: 0,
                    max: 2,
                    reserve: 0,
                    acquire_timeout: Duration::from_secs(5),
                },
            )
            .expect("memory pool");
            let facade = EntityFacade::open(
                &framework_dir(),
                "transactional",
                vec![EntityGroup::new("transactional", "postgres", transactions)],
            )
            .expect("entity facade");
            facade.set_disable_audit_log(true);
            facade.set_disable_entity_eca(true);
            facade.set_disable_fk_create(true);
            facade.set_disable_data_feed(true);
            facade
        }

        fn framework_dir() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
        }

        struct StubDriver;
        struct StubConn;
        struct StubCancel;

        impl DbCancel for StubCancel {
            fn cancel(&self) -> Result<(), Error> {
                Ok(())
            }
        }

        impl DbDriver for StubDriver {
            fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
                Ok(Box::new(StubConn))
            }
        }

        impl DbConnection for StubConn {
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
                Box::new(StubCancel)
            }
        }
    }
}
