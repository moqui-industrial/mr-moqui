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

//! The scheduler thread and the Quartz cron parser. The thread only sleeps and starts passes; a
//! pass is in `jobs`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use moqui_lib::sync::lock;

use croner::parser::{CronParser, Seconds};
use croner::Cron;

use super::pool::Shared;
use crate::context::Error;

/// How often the thread looks at the stop flag. A pass runs every `check_interval`.
const SLICE: Duration = Duration::from_millis(250);

pub(crate) fn scheduler_loop(shared: Arc<Shared>) {
    let mut last_pass = Instant::now();
    loop {
        {
            let state = lock(&shared.state);
            if state.stop {
                return;
            }
            let (state, _) = shared
                .wake
                .wait_timeout(state, SLICE)
                .unwrap_or_else(|err| err.into_inner());
            if state.stop {
                return;
            }
        }
        if last_pass.elapsed() >= shared.check_interval {
            last_pass = Instant::now();
            let _cleanup = super::pool::WorkCleanup::new(shared.factory());
            super::jobs::run_scheduled(&shared);
            super::pool::run_tick_hook(&shared);
        }
    }
}

/// Quartz syntax: optional seconds, Sunday is day 1, `?` `L` `W` `#` allowed.
pub(crate) fn parse_cron(pattern: &str) -> Result<Cron, Error> {
    CronParser::builder()
        .seconds(Seconds::Optional)
        .alternative_weekdays(true)
        .sloppy_ranges(true)
        .build()
        .parse(pattern)
        .map_err(|err| Error::new(format!("cron pattern: {err}")))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use moqui_lib::object::Value;

    use super::super::pool::ServicePools;
    use super::parse_cron;
    use crate::context::{ExecutionContext, ExecutionContextFactory};
    use crate::entity::EntityFacade;

    #[test]
    fn quartz_patterns_parse_and_describe() {
        parse_cron("0 0/15 * * * ?").unwrap();
        let nightly = parse_cron("0 0 2 * * ?").unwrap();
        assert!(!nightly.describe().is_empty());
        let every = parse_cron("* * * * * *").unwrap();
        let now = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::system());
        let next = every.find_next_occurrence(&now, true).unwrap();
        let wait = next.timestamp().as_second() - now.timestamp().as_second();
        assert!((0..=1).contains(&wait), "{wait}");
        // Sunday is day 1 and Monday day 2, as in Quartz.
        parse_cron("0 0 8 ? * 2").unwrap();
    }

    #[test]
    fn call_future_returns_the_out_parameter() {
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(crate::testing::open());
        let _entered = ctx.enter();
        register_echo();
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
        fill_queue(&pools, &hold);
        let started = Instant::now();
        let err = pools.call_future("echo#Note", &BTreeMap::new()).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1), "call_future blocked");
        assert!(err.to_string().contains("queue"), "{err}");
    }

    #[test]
    fn one_worker_call_future_uses_a_reserved_thread() {
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(crate::testing::open());
        let _entered = ctx.enter();
        register_echo();
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
        let none = super::super::start_pools(r#"<moqui-conf><tools worker-pool-max="0"/></moqui-conf>"#);
        assert!(none.is_err(), "a pool with no workers can never run anything");
    }

    // ---- scheduled jobs ----------------------------------------------------------------------

    const SERVICES: &str = r#"<services>
        <service verb="echo" noun="Note" authenticate="false" transaction="ignore">
            <in-parameters><parameter name="note"/></in-parameters>
            <out-parameters><parameter name="note"/></out-parameters>
        </service>
        <service verb="mark" noun="Row">
            <in-parameters><parameter name="note"/></in-parameters>
            <actions>
                <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
                <script>made.set("testId", note); made.set("testMedium", "from-job");</script>
                <entity-create value-field="made"/>
            </actions>
        </service>
        <service verb="markOpen" noun="Row" authenticate="false">
            <in-parameters><parameter name="note"/></in-parameters>
            <actions>
                <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
                <script>made.set("testId", note); made.set("testMedium", "open");</script>
                <entity-create value-field="made"/>
            </actions>
        </service>
        <service verb="fail" noun="Always">
            <actions><message error="true">job failed</message></actions>
        </service>
    </services>"#;

    struct Jobs {
        ctx: ExecutionContext,
        facade: EntityFacade,
        pools_handle: Arc<super::super::pool::Shared>,
    }

    fn jobs_world(workers: usize, queue: usize) -> Jobs {
        // a file for each harness: tests in parallel would otherwise write and lint one shared file
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("moqui-jobs-{}-{seq}.xml", std::process::id()));
        std::fs::write(&path, SERVICES).unwrap();
        let pools = ServicePools::bounded(workers, queue);
        let pools_handle = pools.shared_handle();
        let ctx = ExecutionContextFactory::test_context_with("", pools);
        let facade = crate::testing::open();
        ctx.factory().set_entity(facade.clone());
        {
            let _entered = ctx.enter();
            let failures = crate::service::lint(std::slice::from_ref(&path)).unwrap();
            assert!(failures.is_empty(), "{failures:?}");
        }
        Jobs {
            ctx,
            facade,
            pools_handle,
        }
    }

    fn create(facade: &EntityFacade, name: &str, fields: &[(&str, &str)]) {
        let mut value = facade.make_value(name).unwrap();
        for (field, text) in fields {
            value.set(*field, Value::string(*text));
        }
        value.create(facade).unwrap();
    }

    fn add_job(world: &Jobs, name: &str, service: &str, cron: &str, extra: &[(&str, &str)]) {
        let mut fields = vec![
            ("jobName", name),
            ("serviceName", service),
            ("cronExpression", cron),
            ("paused", "N"),
        ];
        fields.extend_from_slice(extra);
        create(&world.facade, "moqui.service.job.ServiceJob", &fields);
    }

    fn runs(world: &Jobs, name: &str) -> Vec<crate::entity::EntityValue> {
        world
            .facade
            .find("moqui.service.job.ServiceJobRun")
            .unwrap()
            .eq("jobName", Value::string(name))
            .list()
            .unwrap()
            .values()
            .to_vec()
    }

    fn text(row: &crate::entity::EntityValue, field: &str) -> String {
        row.get(field)
            .map(moqui_lib::object::to_plain_string)
            .unwrap_or_default()
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_due_job_runs_once_and_leaves_its_history() {
        let world = jobs_world(2, 8);
        let _entered = world.ctx.enter();
        let _authz = world.ctx.disable_authz();
        add_job(&world, "J1", "mark#Row", "* * * * * *", &[]);
        create(
            &world.facade,
            "moqui.service.job.ServiceJobParameter",
            &[
                ("jobName", "J1"),
                ("parameterName", "note"),
                ("parameterValue", "ran-once"),
            ],
        );
        let pools = world.ctx.factory().service_pools();
        let report = pools.tick();
        assert_eq!(report.jobs_run, 1, "{report:?}");
        wait_for("the run to finish", || {
            runs(&world, "J1").iter().any(|run| text(run, "hasError") == "N")
        });
        let row = world
            .facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("ran-once"))
            .one()
            .unwrap()
            .expect("the job's service ran with its stored parameter");
        assert_eq!(text(&row, "testMedium"), "from-job");
        let lock = world
            .facade
            .find("moqui.service.job.ServiceJobRunLock")
            .unwrap()
            .eq("jobName", Value::string("J1"))
            .one()
            .unwrap()
            .expect("the lock row");
        assert_eq!(text(&lock, "jobRunId"), "", "the lock is cleared after the run");
        assert!(!text(&lock, "lastRunTime").is_empty());
        assert_eq!(runs(&world, "J1").len(), 1);
    }

    #[test]
    fn paused_and_future_jobs_do_not_run_and_a_held_lock_blocks_a_job() {
        let world = jobs_world(2, 8);
        let _entered = world.ctx.enter();
        let _authz = world.ctx.disable_authz();
        add_job(&world, "PAUSED", "echo#Note", "* * * * * *", &[("paused", "Y")]);
        // Every first of January: nothing before now has been missed yet this year only if it ran.
        add_job(&world, "HELD", "echo#Note", "* * * * * *", &[]);
        create(
            &world.facade,
            "moqui.service.job.ServiceJobRunLock",
            &[("jobName", "HELD"), ("jobRunId", "OTHER-INSTANCE")],
        );
        let mut lock = world
            .facade
            .find("moqui.service.job.ServiceJobRunLock")
            .unwrap()
            .eq("jobName", Value::string("HELD"))
            .one()
            .unwrap()
            .unwrap();
        lock.set(
            "lastRunTime",
            moqui_lib::object::basic_convert(
                &Value::I64(jiff::Timestamp::now().as_millisecond()),
                "java.sql.Timestamp",
            )
            .value,
        );
        lock.update(&world.facade).unwrap();
        let report = world.ctx.factory().service_pools().tick();
        assert_eq!(report.jobs_run, 0, "{report:?}");
        assert_eq!(report.jobs_paused, 1);
        assert!(runs(&world, "HELD").is_empty());
        assert!(runs(&world, "PAUSED").is_empty());
    }

    #[test]
    fn a_failed_run_is_recorded_and_not_retried_before_the_minimum_retry_time() {
        let world = jobs_world(2, 8);
        let _entered = world.ctx.enter();
        let _authz = world.ctx.disable_authz();
        add_job(&world, "BAD", "fail#Always", "* * * * * *", &[("minRetryTime", "60")]);
        assert_eq!(world.ctx.factory().service_pools().tick().jobs_run, 1);
        wait_for("the failed run", || {
            runs(&world, "BAD").iter().any(|run| text(run, "hasError") == "Y")
        });
        let run = runs(&world, "BAD").remove(0);
        assert!(text(&run, "errors").contains("job failed"), "{}", text(&run, "errors"));
        std::thread::sleep(Duration::from_millis(1100));
        let report = world.ctx.factory().service_pools().tick();
        assert_eq!(report.jobs_run, 0, "{report:?}");
        assert_eq!(runs(&world, "BAD").len(), 1);
    }

    #[test]
    fn a_repeat_count_pauses_the_job_after_that_many_runs() {
        let world = jobs_world(2, 8);
        let _entered = world.ctx.enter();
        let _authz = world.ctx.disable_authz();
        add_job(&world, "ONCE", "echo#Note", "* * * * * *", &[("repeatCount", "1")]);
        assert_eq!(world.ctx.factory().service_pools().tick().jobs_run, 1);
        wait_for("the run", || {
            runs(&world, "ONCE").iter().any(|run| !text(run, "endTime").is_empty())
        });
        std::thread::sleep(Duration::from_millis(1100));
        let report = world.ctx.factory().service_pools().tick();
        assert_eq!(report.jobs_run, 0, "{report:?}");
        let job = world
            .facade
            .find("moqui.service.job.ServiceJob")
            .unwrap()
            .eq("jobName", Value::string("ONCE"))
            .one()
            .unwrap()
            .unwrap();
        assert_eq!(text(&job, "paused"), "Y");
    }

    #[test]
    fn a_call_that_finds_the_queue_full_is_stored_as_a_job_and_run_later() {
        let world = jobs_world(1, 1);
        let _entered = world.ctx.enter();
        let _authz = world.ctx.disable_authz();
        let pools = world.ctx.factory().service_pools();
        let hold = Hold::new();
        fill_queue(pools, &hold);
        let mut params = BTreeMap::new();
        params.insert("note".into(), Value::string("queued-call"));
        pools.submit("mark#Row", &params).expect("stored, not lost");
        let stored: Vec<_> = world
            .facade
            .find("moqui.service.job.ServiceJob")
            .unwrap()
            .list()
            .unwrap()
            .values()
            .iter()
            .filter(|job| text(job, "jobName").starts_with(super::super::jobs::QUEUE_PREFIX))
            .cloned()
            .collect();
        assert_eq!(stored.len(), 1);
        assert_eq!(text(&stored[0], "serviceName"), "mark#Row");
        // The queue is still full, so the pass leaves the stored call alone.
        assert_eq!(pools.tick().jobs_run, 0);
        hold.open();
        wait_for("the queue to drain", || world.pools_handle.job_slots() > 0);
        let mut submitted = false;
        wait_for("the queued call to run", || {
            if !submitted {
                submitted = pools.tick().jobs_run == 1;
            }
            world
                .facade
                .find("moqui.test.TestEntity")
                .unwrap()
                .eq("testId", Value::string("queued-call"))
                .one()
                .unwrap()
                .is_some()
        });
        wait_for("the stored call to be removed", || {
            world
                .facade
                .find("moqui.service.job.ServiceJob")
                .unwrap()
                .list()
                .unwrap()
                .values()
                .iter()
                .all(|job| !text(job, "jobName").starts_with(super::super::jobs::QUEUE_PREFIX))
        });
    }

    #[test]
    fn a_failed_async_call_is_not_silent() {
        // The worker logs the failure; the caller of a fire-and-forget call gets nothing back, so
        // the observable part is that the pool survives and runs the next call.
        let world = jobs_world(1, 8);
        let _entered = world.ctx.enter();
        let _authz = world.ctx.disable_authz();
        let pools = world.ctx.factory().service_pools();
        pools.submit("nothing#Registered", &BTreeMap::new()).unwrap();
        let mut params = BTreeMap::new();
        params.insert("note".into(), Value::string("after-failure"));
        pools.submit("markOpen#Row", &params).unwrap();
        wait_for("the second call", || {
            world
                .facade
                .find("moqui.test.TestEntity")
                .unwrap()
                .eq("testId", Value::string("after-failure"))
                .one()
                .unwrap()
                .is_some()
        });
    }

    fn fill_queue(pools: &ServicePools, hold: &Hold) {
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
        let failures = crate::service::lint(std::slice::from_ref(&path)).unwrap();
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework")
    }
}
