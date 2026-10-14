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

//! Service jobs: the scheduler pass and the job run. A port of `ScheduledJobRunner` and
//! `ServiceCallJobImpl`.
//!
//! The pass reads `ServiceJob` rows, takes the `ServiceJobRunLock` row with a conditional write so
//! two instances cannot both run a job, creates the `ServiceJobRun` history row, and hands the run
//! to the service pool. It never runs the service body. A worker does, and records the outcome.
//!
//! A fire-and-forget call that finds the pool queue full is stored as a `ServiceJob` named with
//! [`QUEUE_PREFIX`] and no cron expression. The pass submits it when a slot is free and deletes it
//! when it has run. Parameters of such a call are stored as text, as for any job.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use moqui_db::sql::{insert, update, CompareOp, Condition};
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{basic_convert, to_plain_string, Value};

use super::call::CallOptions;
use super::pool::{lock_mutex, Shared, Work};
use crate::context::{Error, ExecutionContext, ExecutionContextFactory};
use crate::driver::DbConnection;
use crate::entity::{EntityFacade, EntityValue};
use crate::transaction::{TransactionAttribute, TransactionManager};

/// Marks a `ServiceJob` that is a queued fire-and-forget call, not a scheduled job.
pub(crate) const QUEUE_PREFIX: &str = "ASYNC_QUEUE_";

const SERVICE_JOB: &str = "moqui.service.job.ServiceJob";
const JOB_PARAMETER: &str = "moqui.service.job.ServiceJobParameter";
const JOB_RUN: &str = "moqui.service.job.ServiceJobRun";
const JOB_LOCK: &str = "moqui.service.job.ServiceJobRunLock";
const DEFAULT_EXPIRE_MINUTES: i64 = 1440;
const DEFAULT_RETRY_MINUTES: i64 = 5;
const DEFAULT_JOB_TIMEOUT_SECONDS: u64 = 1800;
const TEXT_LIMIT: usize = 4000;

/// A job handed to a worker.
pub(crate) struct JobRun {
    job_name: String,
    service_name: String,
    parameters: BTreeMap<String, Value>,
    timeout: Duration,
    job_run_id: String,
    /// The lock's `lastRunTime` before this run, restored when the run fails.
    last_run_time: Value,
    delete_when_done: bool,
    user_name: Option<String>,
    factory: Arc<ExecutionContextFactory>,
}

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PassReport {
    pub jobs_run: usize,
    pub jobs_active: usize,
    pub jobs_paused: usize,
    pub jobs_ready_not_run: usize,
}

/// One scheduler pass for the pool's factory. Errors are logged, so a bad pass never ends the thread.
pub(crate) fn run_scheduled(shared: &Arc<Shared>) -> PassReport {
    let Some(factory) = shared.factory() else {
        return PassReport::default();
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let context = factory.new_context();
        let _entered = context.enter();
        let _authz = context.disable_authz();
        pass(shared, &factory)
    }));
    match outcome {
        Ok(Ok(report)) => report,
        Ok(Err(err)) => {
            tracing::error!(error = %err, "scheduled job pass failed");
            PassReport::default()
        }
        Err(_) => {
            tracing::error!("scheduled job pass panicked");
            PassReport::default()
        }
    }
}

fn pass(shared: &Arc<Shared>, factory: &Arc<ExecutionContextFactory>) -> Result<PassReport, Error> {
    let mut report = PassReport::default();
    let Some(facade) = factory.entity() else {
        return Ok(report);
    };
    let mut slots = shared.job_slots();
    let now = jiff::Timestamp::now();
    let now_stamp = timestamp_value(now.as_millisecond());
    let plan = LockPlan::new(&facade)?;

    let jobs = facade
        .find(SERVICE_JOB)?
        .order_by("priority", false)
        .order_by("jobName", false)
        .list()?;
    for job in jobs.values() {
        let job_name = text(job, "jobName");
        let queued = job_name.starts_with(QUEUE_PREFIX);
        let cron_text = text(job, "cronExpression");
        if !queued && cron_text.is_empty() {
            continue;
        }
        if !within_dates(job, now.as_millisecond()) {
            continue;
        }
        if !paused_ok(job) {
            report.jobs_paused += 1;
            continue;
        }
        if !queued && repeat_exhausted(&facade, job, &job_name, &now_stamp)? {
            continue;
        }
        report.jobs_active += 1;
        let ready = readiness(&facade, job, &job_name, &cron_text, now)?;
        let Some(ready) = ready else {
            continue;
        };
        if slots <= 0 {
            report.jobs_ready_not_run += 1;
            continue;
        }
        if dispatch(shared, factory, &facade, &plan, job, ready, &now_stamp)? {
            report.jobs_run += 1;
            slots -= 1;
        }
    }
    Ok(report)
}

/// What the lock row allows this pass to do for a job that is otherwise due.
struct Ready {
    expect: Expect,
    last_run_time: Value,
}

/// `None` when the job is not due, is running, or failed too recently.
fn readiness(
    facade: &EntityFacade,
    job: &EntityValue,
    job_name: &str,
    cron_text: &str,
    now: jiff::Timestamp,
) -> Result<Option<Ready>, Error> {
    let lock = facade.find(JOB_LOCK)?.eq("jobName", Value::string(job_name)).one()?;
    let mut expect = Expect::Absent;
    let mut last_run_time = Value::Null;
    let mut last_run_ms = None;
    if let Some(lock) = &lock {
        let run_id = text(lock, "jobRunId");
        last_run_time = lock.get("lastRunTime").cloned().unwrap_or(Value::Null);
        last_run_ms = millis_of(&last_run_time);
        expect = if run_id.is_empty() {
            Expect::Free
        } else {
            Expect::Run(run_id)
        };
        if let (Expect::Run(run_id), Some(last)) = (&expect, last_run_ms) {
            let expire = number(job, "expireLockTime").unwrap_or(DEFAULT_EXPIRE_MINUTES);
            if last < now.as_millisecond().saturating_sub(expire.saturating_mul(60_000)) {
                tracing::warn!(job = job_name, run = %run_id, "lock expired, ignoring it");
            } else {
                tracing::info!(job = job_name, run = %run_id, "lock held, not running");
                return Ok(None);
            }
        } else if matches!(expect, Expect::Run(_)) {
            // A run id with no time: there is nothing to say the lock is old, so it holds.
            return Ok(None);
        }
    }
    if !cron_text.is_empty() {
        let cron = super::schedule::parse_cron(cron_text)?;
        let zoned = now.to_zoned(jiff::tz::TimeZone::system());
        // A schedule with nothing before now has not come due yet.
        let Ok(last_schedule) = cron.find_previous_occurrence(&zoned, true) else {
            return Ok(None);
        };
        if last_run_ms.is_some_and(|last_run| last_schedule.timestamp().as_millisecond() < last_run) {
            return Ok(None);
        }
        // After an error, wait the minimum retry time.
        let runs = facade
            .find(JOB_RUN)?
            .eq("jobName", Value::string(job_name))
            .order_by("startTime", true)
            .limit(1)
            .list()?;
        if let Some(last) = runs.values().first().filter(|run| text(run, "hasError") == "Y") {
            let stamp = millis_of(last.get("endTime").unwrap_or(&Value::Null))
                .or_else(|| millis_of(last.get("startTime").unwrap_or(&Value::Null)));
            let retry = number(job, "minRetryTime").unwrap_or(DEFAULT_RETRY_MINUTES);
            if let Some(stamp) = stamp {
                if stamp > now.as_millisecond().saturating_sub(retry.saturating_mul(60_000)) {
                    tracing::info!(job = job_name, "not retrying after an error yet");
                    return Ok(None);
                }
            }
        }
    }
    Ok(Some(Ready { expect, last_run_time }))
}

fn dispatch(
    shared: &Arc<Shared>,
    factory: &Arc<ExecutionContextFactory>,
    facade: &EntityFacade,
    plan: &LockPlan,
    job: &EntityValue,
    ready: Ready,
    now_stamp: &Value,
) -> Result<bool, Error> {
    let job_name = text(job, "jobName");
    let run_id = create_run(facade, &job_name, None, &BTreeMap::new())?;
    if !plan.claim(&job_name, &run_id, now_stamp, &ready.expect)? {
        // Another instance took it. Its run row would only confuse the history.
        delete_run(facade, &run_id);
        return Ok(false);
    }
    let queued = job_name.starts_with(QUEUE_PREFIX);
    let run = JobRun {
        job_name: job_name.clone(),
        service_name: text(job, "serviceName"),
        parameters: job_parameters(facade, &job_name)?,
        timeout: Duration::from_secs(
            number(job, "transactionTimeout").map_or(DEFAULT_JOB_TIMEOUT_SECONDS, |secs| {
                u64::try_from(secs).unwrap_or(DEFAULT_JOB_TIMEOUT_SECONDS)
            }),
        ),
        job_run_id: run_id.clone(),
        last_run_time: ready.last_run_time,
        delete_when_done: queued,
        user_name: None,
        factory: Arc::clone(factory),
    };
    if shared.submit_job(run) {
        tracing::info!(job = %job_name, run = %run_id, "submitted job");
        return Ok(true);
    }
    // The pool filled between the slot count and the submit. Put everything back.
    let _ = plan.release(&job_name, &run_id, &Value::Null);
    delete_run(facade, &run_id);
    Ok(false)
}

/// Store a call the pool queue had no room for. The pass will submit it.
pub(crate) fn persist_queued(
    factory: &Arc<ExecutionContextFactory>,
    service: &str,
    parameters: &BTreeMap<String, Value>,
) -> Result<(), Error> {
    let facade = factory
        .entity()
        .ok_or_else(|| Error::new("service worker queue is full and there is no database to queue in"))?;
    // A sequence number makes the name unique without inserting anything.
    let mut numbered = facade.make_value(JOB_RUN)?;
    numbered.set_sequenced_id_primary(&facade)?;
    let id = text(&numbered, "jobRunId");
    let name = format!("{QUEUE_PREFIX}{id}");
    let _authz = ExecutionContext::with_current(|ctx| ctx.disable_authz());
    let tx = facade.transactions_for(SERVICE_JOB)?;
    tx.run(TransactionAttribute::UseOrBegin, None, || {
        let mut job = facade.make_value(SERVICE_JOB)?;
        job.set("jobName", Value::string(&name));
        job.set("description", Value::string(format!("Queued call of {service}")));
        job.set("serviceName", Value::string(service));
        job.set("paused", Value::string("N"));
        // Queued calls go after the jobs people scheduled.
        job.set("priority", Value::I32(9));
        job.create(&facade)?;
        for (key, value) in parameters {
            if value.is_null() {
                continue;
            }
            let mut row = facade.make_value(JOB_PARAMETER)?;
            row.set("jobName", Value::string(&name));
            row.set("parameterName", Value::string(key));
            row.set("parameterValue", Value::string(to_plain_string(value)));
            row.create(&facade)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// Run a named job now, outside its schedule. Returns the run id.
///
/// # Errors
///
/// Returns an error when there is no such job, the pool is full, or a row cannot be written.
pub fn run_job_now(job_name: &str, extra: &BTreeMap<String, Value>) -> Result<String, Error> {
    let factory = super::pool::capture_factory()?;
    let facade = factory
        .entity()
        .ok_or_else(|| Error::new("no entity facade is installed"))?;
    let job = facade
        .find(SERVICE_JOB)?
        .eq("jobName", Value::string(job_name))
        .one()?
        .ok_or_else(|| Error::new(format!("No ServiceJob record found for jobName {job_name}")))?;
    let mut parameters = job_parameters(&facade, job_name)?;
    parameters.extend(extra.clone());
    let run_id = create_run(
        &facade,
        job_name,
        crate::user::current_user_id().ok().flatten(),
        &parameters,
    )?;
    let pools = factory.service_pools();
    let shared = pools.shared.clone();
    let run = JobRun {
        job_name: job_name.to_string(),
        service_name: text(&job, "serviceName"),
        parameters,
        timeout: Duration::from_secs(
            number(&job, "transactionTimeout")
                .and_then(|secs| u64::try_from(secs).ok())
                .unwrap_or(DEFAULT_JOB_TIMEOUT_SECONDS),
        ),
        job_run_id: run_id.clone(),
        last_run_time: Value::Null,
        delete_when_done: false,
        user_name: crate::user::current_username().ok().flatten(),
        factory: Arc::clone(&factory),
    };
    // The inline executor runs it at the next drain point.
    let run = match pools.submit_inline_job(Box::new(run)) {
        Ok(()) => return Ok(run_id),
        Err(run) => *run,
    };
    let Some(shared) = shared else {
        delete_run(&facade, &run_id);
        return Err(Error::new("service worker pool is not running"));
    };
    if !shared.submit_job(run) {
        delete_run(&facade, &run_id);
        return Err(Error::new("service worker queue is full"));
    }
    Ok(run_id)
}

/// Send the ServiceJobError notification. A failure is logged and does not change the job row.
pub(crate) fn send_job_error_notification(job_name: &str, job_run_id: &str, errors: &str) {
    if let Err(err) = crate::notify::service_job_error(job_name, errors) {
        tracing::warn!(
            job = %job_name,
            run = %job_run_id,
            error = %err,
            "could not send the service job error notification"
        );
    }
}

/// The worker side: run the service and record what happened.
pub(crate) fn run_job(job: JobRun) {
    let factory = Arc::clone(&job.factory);
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    if let Some(user) = &job.user_name {
        if let Err(err) = crate::user::internal_login_user(user) {
            tracing::warn!(job = %job.job_name, error = %err, "could not log in the job's user");
        }
    }
    let Some(facade) = factory.entity() else {
        return;
    };
    let thread = std::thread::current().name().unwrap_or("").to_string();
    record(
        &facade,
        &job.job_run_id,
        &[("startTime", now_value()), ("runThread", Value::string(thread))],
    );
    let mut parameters = job.parameters.clone();
    if !job.last_run_time.is_null() {
        parameters.insert("lastRunTime".to_string(), job.last_run_time.clone());
    }
    let options = CallOptions {
        disable_authz: true,
        timeout: Some(job.timeout),
        ..CallOptions::default()
    };
    let results = match super::call::sync_with(&job.service_name, &parameters, &options) {
        Ok(results) => results,
        Err(err) => {
            tracing::error!(job = %job.job_name, error = %err, "error in service job call");
            context.message().add_error(err.to_string());
            BTreeMap::new()
        }
    };
    let (has_error, messages, errors) = {
        let messages = context.message();
        (
            !messages.errors().is_empty(),
            clip(&messages.messages().join("\n")),
            clip(&messages.errors().join("\n")),
        )
    };
    let result_text = serde_json::to_string(&moqui_lib::json::to_json(&Value::Map(
        results.into_iter().collect::<indexmap::IndexMap<_, _>>(),
    )))
    .ok();
    // The lock goes first: a failure restores the previous run time so the job is due again.
    if !job.delete_when_done {
        match LockPlan::new(&facade) {
            Ok(plan) => {
                let restored = if has_error {
                    job.last_run_time.clone()
                } else {
                    now_value()
                };
                if let Err(err) = plan.release(&job.job_name, &job.job_run_id, &restored) {
                    tracing::warn!(job = %job.job_name, error = %err, "could not clear the job lock");
                }
            }
            Err(err) => tracing::warn!(job = %job.job_name, error = %err, "could not clear the job lock"),
        }
    }
    record(
        &facade,
        &job.job_run_id,
        &[
            ("endTime", now_value()),
            ("results", result_text.map_or(Value::Null, Value::String)),
            ("messages", Value::string(messages)),
            ("hasError", Value::string(if has_error { "Y" } else { "N" })),
            (
                "errors",
                if has_error {
                    Value::string(errors.clone())
                } else {
                    Value::Null
                },
            ),
        ],
    );
    if has_error {
        tracing::warn!(job = %job.job_name, run = %job.job_run_id, errors = %errors, "service job ended with errors");
        send_job_error_notification(&job.job_name, &job.job_run_id, &errors);
    }
    if job.delete_when_done {
        delete_queued(&facade, &job.job_name);
        if let Ok(plan) = LockPlan::new(&facade) {
            let _ = plan.delete_lock(&job.job_name);
        }
    }
}

fn delete_queued(facade: &EntityFacade, job_name: &str) {
    let outcome = (|| -> Result<(), Error> {
        let tx = facade.transactions_for(SERVICE_JOB)?;
        tx.run(TransactionAttribute::UseOrBegin, None, || {
            let params = facade
                .find(JOB_PARAMETER)?
                .eq("jobName", Value::string(job_name))
                .list()?;
            for row in params.values() {
                row.delete(facade)?;
            }
            if let Some(job) = facade.find(SERVICE_JOB)?.eq("jobName", Value::string(job_name)).one()? {
                job.delete(facade)?;
            }
            Ok(())
        })?;
        Ok(())
    })();
    if let Err(err) = outcome {
        tracing::warn!(job = job_name, error = %err, "could not remove a queued job");
    }
}

fn create_run(
    facade: &EntityFacade,
    job_name: &str,
    user_id: Option<String>,
    parameters: &BTreeMap<String, Value>,
) -> Result<String, Error> {
    let tx = facade.transactions_for(JOB_RUN)?;
    let id = tx.run(TransactionAttribute::UseOrBegin, None, || {
        let mut run = facade.make_value(JOB_RUN)?;
        run.set_sequenced_id_primary(facade)?;
        run.set("jobName", Value::string(job_name));
        if let Some(user_id) = user_id {
            run.set("userId", Value::string(user_id));
        }
        if !parameters.is_empty() {
            let json = moqui_lib::json::to_json(&Value::Map(
                parameters.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            ));
            run.set("parameters", Value::string(json.to_string()));
        }
        let id = text(&run, "jobRunId");
        run.create(facade)?;
        Ok(id)
    })?;
    Ok(id)
}

fn delete_run(facade: &EntityFacade, run_id: &str) {
    let outcome = (|| -> Result<(), Error> {
        if let Some(run) = facade.find(JOB_RUN)?.eq("jobRunId", Value::string(run_id)).one()? {
            run.delete(facade)?;
        }
        Ok(())
    })();
    if let Err(err) = outcome {
        tracing::warn!(run = run_id, error = %err, "could not remove an unused job run");
    }
}

/// Update fields of a run row. The run is history, so a failure is logged.
fn record(facade: &EntityFacade, run_id: &str, fields: &[(&str, Value)]) {
    let outcome = (|| -> Result<(), Error> {
        let tx = facade.transactions_for(JOB_RUN)?;
        tx.run(TransactionAttribute::UseOrBegin, None, || {
            let mut run = facade.make_value(JOB_RUN)?;
            run.set("jobRunId", Value::string(run_id));
            for (name, value) in fields {
                run.set(*name, value.clone());
            }
            run.update(facade)?;
            Ok(())
        })?;
        Ok(())
    })();
    if let Err(err) = outcome {
        tracing::warn!(run = run_id, error = %err, "could not record a service job run");
    }
}

fn job_parameters(facade: &EntityFacade, job_name: &str) -> Result<BTreeMap<String, Value>, Error> {
    let rows = facade
        .find(JOB_PARAMETER)?
        .eq("jobName", Value::string(job_name))
        .list()?;
    Ok(rows
        .values()
        .iter()
        .map(|row| (text(row, "parameterName"), Value::string(text(row, "parameterValue"))))
        .collect())
}

/// Stop a repeating job that has run `repeatCount` times. True when it was stopped.
fn repeat_exhausted(facade: &EntityFacade, job: &EntityValue, job_name: &str, now: &Value) -> Result<bool, Error> {
    let Some(limit) = number(job, "repeatCount") else {
        return Ok(false);
    };
    let runs = facade.find(JOB_RUN)?.eq("jobName", Value::string(job_name)).count()?;
    if runs < limit {
        return Ok(false);
    }
    let mut stop = facade.make_value(SERVICE_JOB)?;
    stop.set("jobName", Value::string(job_name));
    stop.set("paused", Value::string("Y"));
    stop.set("thruDate", now.clone());
    stop.update(facade)?;
    Ok(true)
}

fn within_dates(job: &EntityValue, now_ms: i64) -> bool {
    let from = millis_of(job.get("fromDate").unwrap_or(&Value::Null));
    let thru = millis_of(job.get("thruDate").unwrap_or(&Value::Null));
    from.is_none_or(|from| from <= now_ms) && thru.is_none_or(|thru| thru > now_ms)
}

/// Active when `paused` is null or `N`.
fn paused_ok(job: &EntityValue) -> bool {
    let paused = text(job, "paused");
    paused.is_empty() || paused == "N"
}

// ---- the lock row -------------------------------------------------------------------------

/// What the lock row looked like when the job was found due.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Expect {
    /// No row yet.
    Absent,
    /// A row nobody holds.
    Free,
    /// A row held by this run, whose hold has expired.
    Run(String),
}

struct LockPlan {
    tx: TransactionManager,
    insert_sql: String,
    claim_free_sql: String,
    claim_run_sql: String,
    release_sql: String,
    delete_sql: String,
}

impl LockPlan {
    fn new(facade: &EntityFacade) -> Result<Self, Error> {
        facade
            .with_group(JOB_LOCK, |entity, dialect, _, tx| {
                let by_job = Condition::bound("jobName", CompareOp::Equals);
                let free = Condition::and(vec![by_job.clone(), Condition::is_null("jobRunId")]);
                let held = Condition::and(vec![by_job.clone(), Condition::bound("jobRunId", CompareOp::Equals)]);
                let set = ["jobRunId", "lastRunTime"];
                Ok(Self {
                    tx: tx.clone(),
                    insert_sql: insert(entity, dialect, &["jobName", "jobRunId", "lastRunTime"])?,
                    claim_free_sql: update(entity, dialect, &set, Some(&free))?.sql,
                    claim_run_sql: update(entity, dialect, &set, Some(&held))?.sql,
                    release_sql: update(entity, dialect, &set, Some(&held))?.sql,
                    delete_sql: moqui_db::sql::delete(entity, dialect, Some(&by_job))?.sql,
                })
            })
            .map_err(Error::from)
    }

    /// True when this call took the lock. Every path is a conditional write, so only one caller wins.
    fn claim(&self, job: &str, run_id: &str, now: &Value, expect: &Expect) -> Result<bool, Error> {
        let job_value = Value::string(job);
        reserve(&self.tx, |conn| match expect {
            Expect::Absent => {
                let params = [job_value, Value::string(run_id), now.clone()];
                // A lost insert race is a held lock.
                Ok(conn.execute(&self.insert_sql, &params).is_ok())
            }
            Expect::Free => {
                let params = [Value::string(run_id), now.clone(), job_value];
                Ok(conn.execute(&self.claim_free_sql, &params)? > 0)
            }
            Expect::Run(old) => {
                let params = [Value::string(run_id), now.clone(), job_value, Value::string(old)];
                Ok(conn.execute(&self.claim_run_sql, &params)? > 0)
            }
        })
    }

    /// Clear `run_id`'s hold and set the run time. Does nothing if another run holds the lock now.
    fn release(&self, job: &str, run_id: &str, last_run: &Value) -> Result<(), Error> {
        let params = [Value::Null, last_run.clone(), Value::string(job), Value::string(run_id)];
        reserve(&self.tx, |conn| {
            conn.execute(&self.release_sql, &params)?;
            Ok(())
        })
    }

    fn delete_lock(&self, job: &str) -> Result<(), Error> {
        reserve(&self.tx, |conn| {
            conn.execute(&self.delete_sql, &[Value::string(job)])?;
            Ok(())
        })
    }
}

fn reserve<T>(
    tx: &TransactionManager,
    body: impl FnOnce(&mut dyn DbConnection) -> Result<T, LibError>,
) -> Result<T, Error> {
    tx.with_reserve(|conn| {
        conn.begin()?;
        match body(conn) {
            Ok(value) => {
                if let Err(err) = conn.commit() {
                    let _ = conn.rollback();
                    return Err(err);
                }
                Ok(value)
            }
            Err(err) => {
                let _ = conn.rollback();
                Err(err)
            }
        }
    })
    .map_err(Error::from)
}

// ---- values -------------------------------------------------------------------------------

fn text(row: &EntityValue, field: &str) -> String {
    match row.get(field) {
        Some(Value::String(value)) => value.clone(),
        Some(other) if !other.is_null() => to_plain_string(other),
        _ => String::new(),
    }
}

fn number(row: &EntityValue, field: &str) -> Option<i64> {
    match basic_convert(row.get(field)?, "Long").value {
        Value::I64(number) => Some(number),
        _ => None,
    }
}

fn millis_of(value: &Value) -> Option<i64> {
    match value {
        Value::Timestamp(stamp) => Some(stamp.as_millisecond()),
        Value::Null => None,
        other => to_plain_string(other).parse().ok(),
    }
}

fn timestamp_value(millis: i64) -> Value {
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}

fn now_value() -> Value {
    timestamp_value(jiff::Timestamp::now().as_millisecond())
}

fn clip(text: &str) -> String {
    text.chars().take(TEXT_LIMIT).collect()
}

impl Shared {
    /// The factory this pool serves. `None` before boot attaches it or after it is dropped.
    pub(crate) fn factory(&self) -> Option<Arc<ExecutionContextFactory>> {
        lock_mutex(&self.factory).upgrade()
    }

    /// Room for more jobs: workers plus the conf's job queue, less what is running or waiting.
    pub(crate) fn job_slots(&self) -> i64 {
        let state = lock_mutex(&self.state);
        let total = i64::try_from(self.workers + self.job_queue_max).unwrap_or(i64::MAX);
        let busy = i64::try_from(state.active + state.queue.len()).unwrap_or(i64::MAX);
        total - busy
    }

    /// Queue a job run. False when the queue is full or the pool stopped.
    pub(crate) fn submit_job(&self, run: JobRun) -> bool {
        {
            let mut state = lock_mutex(&self.state);
            if state.stop || state.queue.len() >= self.queue_cap {
                return false;
            }
            state.queue.push_back(Work::Job(Box::new(run)));
        }
        self.wake.notify_all();
        true
    }
}
