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
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::schedule::CronJob;
use crate::context::{Error, ExecutionContext, ExecutionContextFactory};

const DEFAULT_WORKERS: usize = 32;
const DEFAULT_QUEUE: usize = 65_535;

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
    workers: usize,
    pub(crate) queue_cap: usize,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) stop: bool,
    pub(crate) queue: VecDeque<Work>,
    pub(crate) oneshots: VecDeque<OneShot>,
    pub(crate) jobs: Vec<CronJob>,
    pub(crate) runs: BTreeMap<String, u32>,
}

pub(crate) struct OneShot {
    pub(crate) name: String,
    pub(crate) parameters: BTreeMap<String, Value>,
    pub(crate) factory: Arc<ExecutionContextFactory>,
}

pub(crate) enum Work {
    Call(CallJob),
    #[cfg(test)]
    Task(Box<dyn FnOnce() + Send>),
}

pub(crate) struct CallJob {
    name: String,
    parameters: BTreeMap<String, Value>,
    factory: Arc<ExecutionContextFactory>,
    job_name: Option<String>,
    reply: Option<Sender<Result<BTreeMap<String, Value>, Error>>>,
}

impl Work {
    pub(crate) fn service(
        name: String,
        parameters: BTreeMap<String, Value>,
        factory: Arc<ExecutionContextFactory>,
        job_name: Option<String>,
    ) -> Self {
        Work::Call(CallJob {
            name,
            parameters,
            factory,
            job_name,
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
            job_name: None,
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
        start(workers.max(1), queue).expect("service worker pool")
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

    /// Fire-and-forget. A full queue keeps one in-memory shot and returns.
    pub fn submit(&self, name: &str, parameters: &BTreeMap<String, Value>) -> Result<(), Error> {
        let shared = self.running()?;
        let factory = capture_factory()?;
        {
            let mut state = lock_mutex(&shared.state);
            if state.stop {
                return Err(Error::new("service worker pool is not running"));
            }
            if state.queue.len() >= shared.queue_cap {
                state.oneshots.push_back(OneShot {
                    name: name.to_string(),
                    parameters: parameters.clone(),
                    factory,
                });
                return Ok(());
            }
            state
                .queue
                .push_back(Work::service(name.to_string(), parameters.clone(), factory, None));
        }
        shared.wake.notify_all();
        Ok(())
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
            return reserved(factory, name, parameters);
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

    pub(crate) fn record_run(&self, name: &str) {
        let mut state = lock_mutex(&self.state);
        let count = state.runs.entry(name.to_string()).or_insert(0);
        *count = count.saturating_add(1);
    }
}

/// Read `tools.@worker-pool-max` and `tools.@worker-queue`. Missing or non-integer values are 32 and 65535.
pub fn start_pools(conf_text: &str) -> Result<ServicePools, Error> {
    let (workers, queue) = pool_sizes(conf_text)?;
    start(workers, queue)
}

pub(crate) fn lock_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

pub(crate) fn capture_factory() -> Result<Arc<ExecutionContextFactory>, Error> {
    ExecutionContext::with_current(|ctx| Arc::clone(ctx.factory()))
        .ok_or_else(|| Error::new("no execution context is entered"))
}

fn pool_sizes(conf_text: &str) -> Result<(usize, usize), Error> {
    let root = MNode::parse_text("moqui-conf", conf_text)?;
    let Some(tools) = root.first_named("tools") else {
        return Ok((DEFAULT_WORKERS, DEFAULT_QUEUE));
    };
    Ok((
        usize_attr(&tools, "worker-pool-max", DEFAULT_WORKERS),
        usize_attr(&tools, "worker-queue", DEFAULT_QUEUE),
    ))
}

fn usize_attr(node: &MNode, name: &str, default: usize) -> usize {
    node.attribute(name)
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(default)
}

fn start(workers: usize, queue: usize) -> Result<ServicePools, Error> {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::default()),
        wake: Condvar::new(),
        workers,
        queue_cap: queue,
        handles: Mutex::new(Vec::new()),
    });
    let handles = spawn_all(&shared, workers)?;
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
    let Some(job) = call_job(work) else {
        return;
    };
    run_call(shared, job);
}

#[cfg(test)]
fn call_job(work: Work) -> Option<CallJob> {
    match work {
        Work::Call(job) => Some(job),
        Work::Task(task) => {
            run_task(task);
            None
        }
    }
}

#[cfg(not(test))]
fn call_job(work: Work) -> Option<CallJob> {
    let Work::Call(job) = work;
    Some(job)
}

#[cfg(test)]
fn run_task(task: Box<dyn FnOnce() + Send>) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _flag = WorkerFlag::enter();
        let _clear = ClearStack;
        task();
    }));
}

fn run_call(shared: &Shared, job: CallJob) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _flag = WorkerFlag::enter();
        let _clear = ClearStack;
        let produced = invoke(&job.factory, &job.name, &job.parameters)?;
        if let Some(name) = &job.job_name {
            shared.record_run(name);
        }
        Ok(produced)
    }));
    if let Some(reply) = job.reply {
        let _ = reply.send(unwind_result(result));
    }
}

fn invoke(
    factory: &Arc<ExecutionContextFactory>,
    name: &str,
    parameters: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let ctx = factory.new_context();
    let _entered = ctx.enter();
    super::call::sync(name, parameters)
}

fn reserved(
    factory: Arc<ExecutionContextFactory>,
    name: &str,
    parameters: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let (tx, rx) = mpsc::channel();
    let name = name.to_string();
    let parameters = parameters.clone();
    let handle = thread::Builder::new()
        .name("moqui-reserved".to_string())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                let _flag = WorkerFlag::enter();
                let _clear = ClearStack;
                invoke(&factory, &name, &parameters)
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

#[cfg(test)]
fn stopped_call(work: Work) -> Option<CallJob> {
    match work {
        Work::Call(job) => Some(job),
        Work::Task(_) => None,
    }
}

#[cfg(not(test))]
fn stopped_call(work: Work) -> Option<CallJob> {
    let Work::Call(job) = work;
    Some(job)
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
