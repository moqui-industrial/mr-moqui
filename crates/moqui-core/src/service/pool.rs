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

//! Per-factory service workers. Drop stops and joins only this guard's threads.

use std::collections::{BTreeMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use crate::context::{Error, ExecutionContext, ExecutionContextFactory};

const DEFAULT_WORKERS: usize = 32;
const DEFAULT_QUEUE: usize = 65_535;
const DEFAULT_CHECK_SECONDS: u64 = 60;

thread_local! {
    static IN_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn in_worker() -> bool {
    IN_WORKER.with(std::cell::Cell::get)
}

struct WorkerFlag;

impl WorkerFlag {
    fn enter() -> Self {
        IN_WORKER.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for WorkerFlag {
    fn drop(&mut self) {
        IN_WORKER.with(|flag| flag.set(false));
    }
}

struct ClearStack;

impl Drop for ClearStack {
    fn drop(&mut self) {
        crate::script::clear_context_stack();
    }
}

struct ReservedSlot<'a>(&'a Shared);

impl<'a> ReservedSlot<'a> {
    fn take(shared: &'a Shared, limit: usize) -> Result<Self, Error> {
        let before = shared.reserved.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if before >= limit {
            shared.reserved.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            return Err(Error::new("no reserved service worker is free for a nested call"));
        }
        Ok(Self(shared))
    }
}

impl Drop for ReservedSlot<'_> {
    fn drop(&mut self) {
        self.0.reserved.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct JoinOnDrop(Option<JoinHandle<()>>);

impl Drop for JoinOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            join_one(handle);
        }
    }
}

/// Factory-owned pool guard. [`ServicePools::idle`] starts no threads.
///
/// Drop sets the stop flag, wakes this guard's workers, and joins them. Another factory's guard
/// is a different value and keeps running.
#[must_use = "dropping the guard stops and joins this pool's threads"]
pub struct ServicePools {
    pub(crate) shared: Option<Arc<Shared>>,
}

pub(crate) struct Shared {
    pub(crate) state: Mutex<State>,
    pub(crate) wake: Condvar,
    pub(crate) workers: usize,
    pub(crate) queue_cap: usize,
    /// Jobs allowed to wait beyond the workers, from `service-facade/@job-queue-max`.
    pub(crate) job_queue_max: usize,
    /// How often the scheduler thread runs a pass, from `service-facade/@scheduled-job-check-time`.
    pub(crate) check_interval: Duration,
    /// The factory the pool serves, set once it is built. Weak, so the pool never keeps it alive.
    pub(crate) factory: Mutex<Weak<ExecutionContextFactory>>,
    /// Threads running a nested `call_future` for a worker. Bounded by the worker count.
    pub(crate) reserved: std::sync::atomic::AtomicUsize,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) stop: bool,
    pub(crate) queue: VecDeque<Work>,
    /// Workers running something now.
    pub(crate) active: usize,
}

pub(crate) enum Work {
    Call(CallJob),
    Job(Box<super::jobs::JobRun>),
    #[cfg(test)]
    Task(Box<dyn FnOnce() + Send>),
}

pub(crate) struct CallJob {
    name: String,
    parameters: BTreeMap<String, Value>,
    factory: Arc<ExecutionContextFactory>,
    /// The user who made the call; the worker acts as them.
    user_name: Option<String>,
    reply: Option<Sender<Result<BTreeMap<String, Value>, Error>>>,
}

impl Work {
    pub(crate) fn service(
        name: String,
        parameters: BTreeMap<String, Value>,
        factory: Arc<ExecutionContextFactory>,
        user_name: Option<String>,
    ) -> Self {
        Work::Call(CallJob {
            name,
            parameters,
            factory,
            user_name,
            reply: None,
        })
    }

    fn replied(
        name: String,
        parameters: BTreeMap<String, Value>,
        factory: Arc<ExecutionContextFactory>,
        reply: Sender<Result<BTreeMap<String, Value>, Error>>,
    ) -> Self {
        Work::Call(CallJob {
            name,
            parameters,
            factory,
            user_name: crate::user::current_username().ok().flatten(),
            reply: Some(reply),
        })
    }
}

impl ServicePools {
    /// No workers. `test_context` stores this.
    pub fn idle() -> Self {
        Self { shared: None }
    }

    /// Test pool. `workers` is at least 1. Same Drop as a booted pool.
    pub fn bounded(workers: usize, queue: usize) -> Self {
        start(Settings::test(workers.max(1), queue)).expect("service worker pool")
    }

    /// Worker count and queue capacity. Idle is `(0, 0)`.
    pub fn sizes(&self) -> (usize, usize) {
        self.shared
            .as_ref()
            .map(|shared| (shared.workers, shared.queue_cap))
            .unwrap_or((0, 0))
    }

    /// Submit and wait. A full queue returns immediately. An in-worker caller uses a fresh thread.
    pub fn call_future(
        &self,
        name: &str,
        parameters: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        self.running()?.call_future(name, parameters)
    }

    /// Tell the pool which factory it serves. Boot calls this once the factory is built.
    pub fn attach(&self, factory: &Arc<ExecutionContextFactory>) {
        if let Some(shared) = &self.shared {
            *lock_mutex(&shared.factory) = Arc::downgrade(factory);
        }
    }

    /// Fire-and-forget. A full queue stores the call as a one-shot service job, which the scheduler
    /// pass submits when a worker is free. With no database to store it in, a full queue is an error.
    pub fn submit(&self, name: &str, parameters: &BTreeMap<String, Value>) -> Result<(), Error> {
        let shared = self.running()?;
        let factory = capture_factory()?;
        let user_name = crate::user::current_username().ok().flatten();
        {
            let mut state = lock_mutex(&shared.state);
            if state.stop {
                return Err(Error::new("service worker pool is not running"));
            }
            if state.queue.len() < shared.queue_cap {
                state
                    .queue
                    .push_back(Work::service(name.to_string(), parameters.clone(), factory, user_name));
                drop(state);
                shared.wake.notify_all();
                return Ok(());
            }
        }
        super::jobs::persist_queued(&factory, name, parameters)
    }

    /// One scheduler pass now, as the scheduler thread runs it on its interval.
    pub fn tick(&self) -> super::jobs::PassReport {
        match &self.shared {
            Some(shared) => super::jobs::run_scheduled(shared),
            None => super::jobs::PassReport::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn submit_task(&self, task: impl FnOnce() + Send + 'static) -> Result<(), Error> {
        let shared = self.running()?;
        {
            let mut state = lock_mutex(&shared.state);
            if state.stop {
                return Err(Error::new("service worker pool is not running"));
            }
            if state.queue.len() >= shared.queue_cap {
                return Err(Error::new("service worker queue is full"));
            }
            state.queue.push_back(Work::Task(Box::new(task)));
        }
        shared.wake.notify_all();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn shared_handle(&self) -> Arc<Shared> {
        Arc::clone(self.shared.as_ref().expect("service worker pool"))
    }

    fn running(&self) -> Result<&Arc<Shared>, Error> {
        self.shared
            .as_ref()
            .ok_or_else(|| Error::new("service worker pool is not running"))
    }
}

impl Drop for ServicePools {
    fn drop(&mut self) {
        let Some(shared) = self.shared.take() else {
            return;
        };
        fail_pending(&shared);
        let handles = {
            let mut guard = lock_mutex(&shared.handles);
            std::mem::take(&mut *guard)
        };
        join_all(handles);
    }
}

impl Shared {
    pub(crate) fn call_future(
        self: &Arc<Self>,
        name: &str,
        parameters: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        let factory = capture_factory()?;
        if in_worker() {
            return reserved(self, factory, name, parameters);
        }
        let (tx, rx) = mpsc::channel();
        {
            let mut state = lock_mutex(&self.state);
            if state.stop {
                return Err(Error::new("service worker pool is not running"));
            }
            if state.queue.len() >= self.queue_cap {
                return Err(Error::new("service worker queue is full"));
            }
            state
                .queue
                .push_back(Work::replied(name.to_string(), parameters.clone(), factory, tx));
        }
        self.wake.notify_all();
        rx.recv()
            .unwrap_or_else(|_| Err(Error::new("service worker pool is not running")))
    }
}

/// Read `tools.@worker-pool-max` and `tools.@worker-queue`, and from `service-facade` the
/// `job-queue-max` and `scheduled-job-check-time`. Missing or non-integer values are the defaults.
///
/// # Errors
///
/// Returns an error when the conf does not parse, or the pool would have no workers.
pub fn start_pools(conf_text: &str) -> Result<ServicePools, Error> {
    start(settings(conf_text)?)
}

struct Settings {
    workers: usize,
    queue: usize,
    job_queue_max: usize,
    check_interval: Duration,
}

impl Settings {
    fn test(workers: usize, queue: usize) -> Self {
        Self {
            workers,
            queue,
            job_queue_max: 0,
            check_interval: Duration::from_secs(DEFAULT_CHECK_SECONDS),
        }
    }
}

pub(crate) fn lock_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

pub(crate) fn capture_factory() -> Result<Arc<ExecutionContextFactory>, Error> {
    ExecutionContext::with_current(|ctx| Arc::clone(ctx.factory()))
        .ok_or_else(|| Error::new("no execution context is entered"))
}

fn settings(conf_text: &str) -> Result<Settings, Error> {
    let root = MNode::parse_text("moqui-conf", conf_text)?;
    let tools = root.first_named("tools");
    let facade = root.first_named("service-facade");
    let tool = |name: &str, default: usize| tools.as_ref().map_or(default, |node| usize_attr(node, name, default));
    let service = |name: &str, default: usize| facade.as_ref().map_or(default, |node| usize_attr(node, name, default));
    let seconds = service(
        "scheduled-job-check-time",
        usize::try_from(DEFAULT_CHECK_SECONDS).unwrap_or(60),
    )
    .max(1);
    Ok(Settings {
        workers: tool("worker-pool-max", DEFAULT_WORKERS),
        queue: tool("worker-queue", DEFAULT_QUEUE),
        job_queue_max: service("job-queue-max", 0),
        check_interval: Duration::from_secs(u64::try_from(seconds).unwrap_or(DEFAULT_CHECK_SECONDS)),
    })
}

fn usize_attr(node: &MNode, name: &str, default: usize) -> usize {
    node.attribute(name)
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(default)
}

fn start(settings: Settings) -> Result<ServicePools, Error> {
    if settings.workers == 0 {
        return Err(Error::new("the service worker pool needs at least one worker"));
    }
    let shared = Arc::new(Shared {
        state: Mutex::new(State::default()),
        wake: Condvar::new(),
        workers: settings.workers,
        queue_cap: settings.queue,
        job_queue_max: settings.job_queue_max,
        check_interval: settings.check_interval,
        factory: Mutex::new(Weak::new()),
        reserved: std::sync::atomic::AtomicUsize::new(0),
        handles: Mutex::new(Vec::new()),
    });
    let handles = spawn_all(&shared, settings.workers)?;
    *lock_mutex(&shared.handles) = handles;
    Ok(ServicePools { shared: Some(shared) })
}

fn spawn_all(shared: &Arc<Shared>, workers: usize) -> Result<Vec<JoinHandle<()>>, Error> {
    let mut handles = Vec::with_capacity(workers.saturating_add(1));
    for index in 0..workers {
        if let Err(err) = push_handle(&mut handles, spawn_worker(shared, index)) {
            stop_joined(shared, handles);
            return Err(err);
        }
    }
    if let Err(err) = push_handle(&mut handles, spawn_scheduler(shared)) {
        stop_joined(shared, handles);
        return Err(err);
    }
    Ok(handles)
}

fn push_handle(handles: &mut Vec<JoinHandle<()>>, spawned: Result<JoinHandle<()>, Error>) -> Result<(), Error> {
    handles.push(spawned?);
    Ok(())
}

fn spawn_worker(shared: &Arc<Shared>, index: usize) -> Result<JoinHandle<()>, Error> {
    let shared = Arc::clone(shared);
    thread::Builder::new()
        .name(format!("moqui-worker-{index}"))
        .spawn(move || worker_loop(shared))
        .map_err(|err| Error::new(format!("service worker did not start: {err}")))
}

fn spawn_scheduler(shared: &Arc<Shared>) -> Result<JoinHandle<()>, Error> {
    let shared = Arc::clone(shared);
    thread::Builder::new()
        .name("moqui-scheduler".to_string())
        .spawn(move || super::schedule::scheduler_loop(shared))
        .map_err(|err| Error::new(format!("service scheduler did not start: {err}")))
}

fn worker_loop(shared: Arc<Shared>) {
    while let Some(work) = next_work(&shared) {
        run_work(&shared, work);
    }
}

fn next_work(shared: &Shared) -> Option<Work> {
    let mut state = lock_mutex(&shared.state);
    loop {
        if state.stop {
            return None;
        }
        if let Some(work) = state.queue.pop_front() {
            return Some(work);
        }
        state = shared.wake.wait(state).unwrap_or_else(|err| err.into_inner());
    }
}

fn run_work(shared: &Arc<Shared>, work: Work) {
    lock_mutex(&shared.state).active += 1;
    match work {
        Work::Job(run) => {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _flag = WorkerFlag::enter();
                let _clear = ClearStack;
                super::jobs::run_job(*run);
            }));
        }
        other => {
            if let Some(job) = call_job(other) {
                run_call(job);
            }
        }
    }
    lock_mutex(&shared.state).active -= 1;
}

fn call_job(work: Work) -> Option<CallJob> {
    match work {
        Work::Call(job) => Some(job),
        Work::Job(_) => None,
        #[cfg(test)]
        Work::Task(task) => {
            run_task(task);
            None
        }
    }
}

#[cfg(test)]
fn run_task(task: Box<dyn FnOnce() + Send>) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _flag = WorkerFlag::enter();
        let _clear = ClearStack;
        task();
    }));
}

fn run_call(job: CallJob) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _flag = WorkerFlag::enter();
        let _clear = ClearStack;
        invoke(&job.factory, &job.name, &job.parameters, job.user_name.as_deref())
    }));
    let outcome = unwind_result(result);
    match job.reply {
        Some(reply) => {
            let _ = reply.send(outcome);
        }
        None => {
            // Nobody waits on a fire-and-forget call, so a failure is only visible here.
            if let Err(err) = outcome {
                tracing::error!(service = %job.name, error = %err, "async service call failed");
            }
        }
    }
}

fn invoke(
    factory: &Arc<ExecutionContextFactory>,
    name: &str,
    parameters: &BTreeMap<String, Value>,
    user_name: Option<&str>,
) -> Result<BTreeMap<String, Value>, Error> {
    let ctx = factory.new_context();
    let _entered = ctx.enter();
    if let Some(user_name) = user_name {
        crate::user::internal_login_user(user_name)?;
    }
    super::call::sync(name, parameters)
}

fn reserved(
    shared: &Shared,
    factory: Arc<ExecutionContextFactory>,
    name: &str,
    parameters: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    // A nested wait must not take a thread the pool cannot spare: at most one per worker.
    let _slot = ReservedSlot::take(shared, shared.workers)?;
    let (tx, rx) = mpsc::channel();
    let name = name.to_string();
    let parameters = parameters.clone();
    let user_name = crate::user::current_username().ok().flatten();
    let handle = thread::Builder::new()
        .name("moqui-reserved".to_string())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let _flag = WorkerFlag::enter();
                let _clear = ClearStack;
                invoke(&factory, &name, &parameters, user_name.as_deref())
            }));
            let _ = tx.send(unwind_result(result));
        })
        .map_err(|err| Error::new(format!("reserved worker did not start: {err}")))?;
    let _join = JoinOnDrop(Some(handle));
    rx.recv()
        .unwrap_or_else(|_| Err(Error::new("service worker pool is not running")))
}

fn unwind_result(
    result: Result<Result<BTreeMap<String, Value>, Error>, Box<dyn std::any::Any + Send>>,
) -> Result<BTreeMap<String, Value>, Error> {
    match result {
        Ok(inner) => inner,
        Err(_) => Err(Error::new("service worker panicked")),
    }
}

fn fail_pending(shared: &Shared) {
    let pending = {
        let mut state = lock_mutex(&shared.state);
        state.stop = true;
        std::mem::take(&mut state.queue)
    };
    shared.wake.notify_all();
    for work in pending {
        fail_one(work);
    }
}

fn fail_one(work: Work) {
    let Some(job) = stopped_call(work) else {
        return;
    };
    if let Some(reply) = job.reply {
        let _ = reply.send(Err(Error::new("service worker pool is not running")));
    }
}

fn stopped_call(work: Work) -> Option<CallJob> {
    match work {
        Work::Call(job) => Some(job),
        Work::Job(_) => None,
        #[cfg(test)]
        Work::Task(_) => None,
    }
}

fn stop_joined(shared: &Shared, handles: Vec<JoinHandle<()>>) {
    fail_pending(shared);
    join_all(handles);
}

fn join_all(handles: Vec<JoinHandle<()>>) {
    for handle in handles {
        join_one(handle);
    }
}

fn join_one(handle: JoinHandle<()>) {
    if handle.thread().id() == thread::current().id() {
        std::mem::forget(handle);
        return;
    }
    let _ = handle.join();
}
