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

//! Thread-local transaction stack. Two managers do not share a stack.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use moqui_lib::error::Error;

use crate::driver::{DbConnection, DbDriver};

use super::pool::{Pool, PoolConfig, PoolStats, PooledConn};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static STACKS: RefCell<HashMap<u64, Vec<Frame>>> = RefCell::new(HashMap::new());
}

type ConnHook = Box<dyn FnOnce(&mut dyn DbConnection) -> Result<(), Error> + Send>;
type AfterHook = Box<dyn FnOnce() + Send>;

struct Frame {
    join_depth: u32,
    deadline: Instant,
    conn: Rc<RefCell<Option<PooledConn>>>,
    suspended: bool,
    rollback_only: Option<String>,
    cache_flush: Vec<ConnHook>,
    feed_insert: Vec<ConnHook>,
    on_commit: Vec<AfterHook>,
    on_rollback: Vec<AfterHook>,
}

struct Phase {
    conn: Rc<RefCell<Option<PooledConn>>>,
    cache_flush: Vec<ConnHook>,
    feed_insert: Vec<ConnHook>,
    on_commit: Vec<AfterHook>,
    on_rollback: Vec<AfterHook>,
}

enum Top {
    Empty,
    Active,
    Suspended,
}

enum Prepared {
    Joined,
    Ready {
        rollback_only: Option<String>,
        phase: Phase,
    },
}

struct Inner {
    id: u64,
    pool: Pool,
}

/// Transaction manager. `Clone` shares the pool and the thread-local stack key.
#[derive(Clone)]
pub struct TransactionManager {
    inner: Arc<Inner>,
}

impl TransactionManager {
    pub fn new(driver: Arc<dyn DbDriver>, config: PoolConfig) -> Result<Self, Error> {
        Ok(Self {
            inner: Arc::new(Inner {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                pool: Pool::new(driver, config)?,
            }),
        })
    }

    pub fn stats(&self) -> PoolStats {
        self.inner.pool.stats()
    }

    /// Errors when `(request_cap + service_cap) * 2 > max - reserve`.
    pub fn check_caps(&self, request_cap: u32, service_cap: u32) -> Result<(), Error> {
        self.inner.pool.check_caps(request_cap, service_cap)
    }

    /// Records the deadline and pushes a frame. Does not check out a connection.
    ///
    /// Returns `true` when this call began the transaction. Returns `false` when this thread
    /// already has an active frame. Commit and rollback run only for the call that returned `true`.
    /// `timeout` defaults to 60 seconds and is not changed when this call joins.
    pub fn begin(&self, timeout: Option<Duration>) -> Result<bool, Error> {
        let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
        self.with_frames(|frames| {
            if active_mut(frames).is_some() {
                let frame = active_mut(frames).expect("active frame");
                frame.join_depth += 1;
                return Ok(false);
            }
            frames.push(Frame {
                join_depth: 0,
                deadline: Instant::now() + timeout,
                conn: Rc::new(RefCell::new(None)),
                suspended: false,
                rollback_only: None,
                cache_flush: Vec::new(),
                feed_insert: Vec::new(),
                on_commit: Vec::new(),
                on_rollback: Vec::new(),
            });
            Ok(true)
        })
    }

    pub fn commit(&self) -> Result<(), Error> {
        match self.prepare()? {
            Prepared::Joined => Ok(()),
            Prepared::Ready {
                rollback_only: Some(message),
                phase,
            } => {
                tracing::warn!(%message, "commit marked rollback-only; rolling back");
                self.complete(phase, false)?;
                Err(Error::new(format!("transaction marked rollback-only: {message}")))
            }
            Prepared::Ready {
                rollback_only: None,
                phase,
            } => self.complete(phase, true),
        }
    }

    pub fn rollback(&self) -> Result<(), Error> {
        match self.prepare()? {
            Prepared::Joined => Ok(()),
            Prepared::Ready { phase, .. } => self.complete(phase, false),
        }
    }

    /// A later commit rolls back. The commit error includes `message`.
    pub fn set_rollback_only(&self, message: impl Into<String>) -> Result<(), Error> {
        let message = message.into();
        let stored = self.mark_rollback_only(message)?;
        if stored {
            tracing::warn!("transaction set rollback-only");
        }
        Ok(())
    }

    /// Checks out once per frame and reuses that connection.
    ///
    /// With no active frame, checks a connection out and returns it before this function returns.
    /// An open stream is that enlisted connection: this reuses it and does not check out another.
    /// An error from the closure marks the frame rollback-only. Later calls fail until rollback.
    pub fn with_connection<T, F>(&self, body: F) -> Result<T, Error>
    where
        F: FnOnce(&mut dyn DbConnection) -> Result<T, Error>,
    {
        if !self.has_active() {
            let mut pooled = self.inner.pool.checkout_ordinary()?;
            let result = body(pooled.conn());
            drop(pooled);
            return result;
        }
        if let Some(message) = self.rollback_message() {
            return Err(Error::new(format!("transaction is rollback-only: {message}")));
        }
        let slot = self.active_conn()?;
        if slot.borrow().is_none() {
            let mut pooled = self.inner.pool.checkout_ordinary()?;
            if let Err(err) = pooled.conn().begin() {
                pooled.discard();
                return Err(err);
            }
            *slot.borrow_mut() = Some(pooled);
        }
        let mut borrow = match slot.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => return Err(Error::new("connection already in use")),
        };
        let pooled = borrow.as_mut().ok_or_else(|| Error::new("no connection enlisted"))?;
        let result = body(pooled.conn());
        if let Err(err) = &result {
            let message = err.to_string();
            drop(borrow);
            let _ = self.mark_rollback_only(message);
        }
        result
    }

    /// Reserve checkout. The closure begins and commits its own transaction, or stays autocommit.
    /// Does not use the caller's enlisted connection.
    pub fn with_reserve<T, F>(&self, body: F) -> Result<T, Error>
    where
        F: FnOnce(&mut dyn DbConnection) -> Result<T, Error>,
    {
        let mut pooled = self.inner.pool.checkout_reserve()?;
        let result = body(pooled.conn());
        drop(pooled);
        result
    }

    /// Parks the enlisted client on the frame, off the free list. Returns `false` when nothing is active.
    pub fn suspend(&self) -> Result<bool, Error> {
        if !self.has_active() {
            return Ok(false);
        }
        if self.stream_open() {
            return Err(Error::new("an open stream is the enlisted connection"));
        }
        let slot = self.active_conn()?;
        if let Some(conn) = slot.borrow_mut().as_mut() {
            self.inner.pool.park(conn);
        }
        self.with_frames(|frames| {
            let frame = active_mut(frames).ok_or_else(|| Error::new("no transaction in place"))?;
            frame.suspended = true;
            Ok(true)
        })
    }

    /// Checks the parked client out again.
    pub fn resume(&self) -> Result<(), Error> {
        if self.has_active() {
            return Err(Error::new("cannot resume while a transaction is active"));
        }
        let slot = self.with_frames(|frames| {
            let frame = frames.last().ok_or_else(|| Error::new("no suspended transaction"))?;
            if !frame.suspended {
                return Err(Error::new("no suspended transaction"));
            }
            Ok(Rc::clone(&frame.conn))
        })?;
        if let Some(conn) = slot.borrow_mut().as_mut() {
            self.inner.pool.unpark(conn);
        }
        self.with_frames(|frames| {
            if let Some(frame) = frames.last_mut() {
                frame.suspended = false;
            }
            Ok(())
        })
    }

    /// Suspends the current frame when it has a client, then begins a frame.
    ///
    /// Fails when a client is already parked. The park budget is one per thread on this pool.
    pub fn force_new(&self, timeout: Option<Duration>) -> Result<bool, Error> {
        if self.stats().parked >= 1 {
            return Err(Error::new("park budget exceeded"));
        }
        if self.stream_open() {
            return Err(Error::new("an open stream is the enlisted connection"));
        }
        if self.has_active() {
            self.suspend()?;
        }
        self.begin(timeout)
    }

    pub fn savepoint(&self, name: &str) -> Result<(), Error> {
        self.with_enlisted(|conn| conn.savepoint(name))
    }

    pub fn rollback_to_savepoint(&self, name: &str) -> Result<(), Error> {
        self.with_enlisted(|conn| conn.rollback_to_savepoint(name))
    }

    pub fn release_savepoint(&self, name: &str) -> Result<(), Error> {
        self.with_enlisted(|conn| conn.release_savepoint(name))
    }

    pub fn on_cache_flush<F>(&self, hook: F) -> Result<(), Error>
    where
        F: FnOnce(&mut dyn DbConnection) -> Result<(), Error> + Send + 'static,
    {
        self.push_conn_hook(HookKind::Cache, Box::new(hook))
    }

    pub fn on_feed_insert<F>(&self, hook: F) -> Result<(), Error>
    where
        F: FnOnce(&mut dyn DbConnection) -> Result<(), Error> + Send + 'static,
    {
        self.push_conn_hook(HookKind::Feed, Box::new(hook))
    }

    /// True when this frame already has a cache-flush hook.
    pub fn has_cache_flush(&self) -> bool {
        self.with_frames(|frames| active_mut(frames).is_some_and(|frame| !frame.cache_flush.is_empty()))
    }

    /// True when this frame already has a feed-insert hook.
    pub fn has_feed_insert(&self) -> bool {
        self.with_frames(|frames| active_mut(frames).is_some_and(|frame| !frame.feed_insert.is_empty()))
    }

    pub fn on_commit<F>(&self, hook: F) -> Result<(), Error>
    where
        F: FnOnce() + Send + 'static,
    {
        self.push_after_hook(HookKind::Commit, Box::new(hook))
    }

    pub fn on_rollback<F>(&self, hook: F) -> Result<(), Error>
    where
        F: FnOnce() + Send + 'static,
    {
        self.push_after_hook(HookKind::Rollback, Box::new(hook))
    }

    /// Commits a frame left open, resumes and commits parked frames, and returns parked connections.
    /// Does nothing when this thread has no frame for this manager.
    pub fn destroy_thread(&self) -> Result<(), Error> {
        if self.depth() > 0 {
            tracing::warn!("thread ending with a transaction in place; committing");
        }
        loop {
            match self.top() {
                Top::Empty => return Ok(()),
                Top::Suspended => self.resume()?,
                Top::Active => {
                    let before = self.depth();
                    if let Err(err) = self.commit() {
                        if self.depth() >= before {
                            return Err(err);
                        }
                        tracing::warn!(error = %err, "destroy_thread rolled back a transaction");
                    }
                }
            }
        }
    }

    /// When a frame's deadline has passed, marks it rollback-only and cancels the active statement.
    /// Does not start a timer.
    pub fn poll_deadline(&self) -> Result<(), Error> {
        let now = Instant::now();
        let canceller = self.with_frames(|frames| {
            let mut canceller = None;
            for frame in frames.iter_mut() {
                if now < frame.deadline {
                    continue;
                }
                if frame.rollback_only.is_none() {
                    frame.rollback_only = Some("transaction deadline exceeded".to_string());
                    tracing::warn!("transaction deadline exceeded");
                }
            }
            if let Some(frame) = frames.last() {
                if !frame.suspended && now >= frame.deadline {
                    if let Some(conn) = frame.conn.borrow().as_ref() {
                        canceller = conn.canceller();
                    }
                }
            }
            canceller
        });
        if let Some(canceller) = canceller {
            canceller.cancel()?;
        }
        Ok(())
    }

    pub fn in_transaction(&self) -> bool {
        self.depth() > 0
    }

    fn complete(&self, mut phase: Phase, commit: bool) -> Result<(), Error> {
        let result = if commit {
            try_commit(&mut phase)
        } else {
            rollback_enlisted(&phase.conn)
        };
        return_conn(&phase.conn);
        self.pop_active()?;
        let hook_result = {
            let _resume = ResumeOnDrop { mgr: self };
            if result.is_ok() && commit {
                for hook in phase.on_commit.drain(..) {
                    hook();
                }
            } else {
                for hook in phase.on_rollback.drain(..) {
                    hook();
                }
            }
            result
        };
        hook_result
    }

    fn prepare(&self) -> Result<Prepared, Error> {
        self.with_frames(|frames| {
            let frame = active_mut(frames).ok_or_else(|| Error::new("no transaction in place"))?;
            if frame.join_depth > 0 {
                frame.join_depth -= 1;
                return Ok(Prepared::Joined);
            }
            let rollback_only = frame.rollback_only.clone();
            let phase = Phase {
                conn: Rc::clone(&frame.conn),
                cache_flush: std::mem::take(&mut frame.cache_flush),
                feed_insert: std::mem::take(&mut frame.feed_insert),
                on_commit: std::mem::take(&mut frame.on_commit),
                on_rollback: std::mem::take(&mut frame.on_rollback),
            };
            Ok(Prepared::Ready { rollback_only, phase })
        })
    }

    fn pop_active(&self) -> Result<(), Error> {
        self.with_frames(|frames| match frames.last() {
            Some(frame) if !frame.suspended => {
                frames.pop();
                Ok(())
            }
            _ => Err(Error::new("no transaction in place")),
        })
    }

    fn push_conn_hook(&self, kind: HookKind, hook: ConnHook) -> Result<(), Error> {
        self.with_frames(|frames| {
            let frame = active_mut(frames).ok_or_else(|| Error::new("no transaction in place"))?;
            match kind {
                HookKind::Cache => frame.cache_flush.push(hook),
                HookKind::Feed => frame.feed_insert.push(hook),
                HookKind::Commit | HookKind::Rollback => {}
            }
            Ok(())
        })
    }

    fn push_after_hook(&self, kind: HookKind, hook: AfterHook) -> Result<(), Error> {
        self.with_frames(|frames| {
            let frame = active_mut(frames).ok_or_else(|| Error::new("no transaction in place"))?;
            match kind {
                HookKind::Commit => frame.on_commit.push(hook),
                HookKind::Rollback => frame.on_rollback.push(hook),
                HookKind::Cache | HookKind::Feed => {}
            }
            Ok(())
        })
    }

    fn mark_rollback_only(&self, message: String) -> Result<bool, Error> {
        self.with_frames(|frames| {
            let frame = active_mut(frames).ok_or_else(|| Error::new("no transaction in place"))?;
            if frame.rollback_only.is_none() {
                frame.rollback_only = Some(message);
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }

    fn rollback_message(&self) -> Option<String> {
        self.with_frames(|frames| active(frames).and_then(|frame| frame.rollback_only.clone()))
    }

    fn active_conn(&self) -> Result<Rc<RefCell<Option<PooledConn>>>, Error> {
        self.with_frames(|frames| {
            let frame = active_mut(frames).ok_or_else(|| Error::new("no transaction in place"))?;
            Ok(Rc::clone(&frame.conn))
        })
    }

    fn with_enlisted<T>(&self, body: impl FnOnce(&mut dyn DbConnection) -> Result<T, Error>) -> Result<T, Error> {
        let slot = self.active_conn().map_err(|_| Error::new("no connection enlisted"))?;
        let mut borrow = slot.borrow_mut();
        let pooled = borrow.as_mut().ok_or_else(|| Error::new("no connection enlisted"))?;
        body(pooled.conn())
    }

    fn stream_open(&self) -> bool {
        let Ok(slot) = self.active_conn() else {
            return false;
        };
        let borrow = slot.borrow();
        borrow.as_ref().is_some_and(|conn| conn.stream_open())
    }

    fn has_active(&self) -> bool {
        self.with_frames(|frames| active(frames).is_some())
    }

    fn depth(&self) -> usize {
        self.with_frames(|frames| frames.len())
    }

    fn top(&self) -> Top {
        self.with_frames(|frames| match frames.last() {
            None => Top::Empty,
            Some(frame) if frame.suspended => Top::Suspended,
            Some(_) => Top::Active,
        })
    }

    fn resume_parked_parent(&self) {
        let suspended = self.with_frames(|frames| matches!(frames.last(), Some(frame) if frame.suspended));
        if suspended {
            if let Err(err) = self.resume() {
                tracing::warn!(error = %err, "resume after completion failed");
            }
        }
    }

    fn with_frames<T>(&self, body: impl FnOnce(&mut Vec<Frame>) -> T) -> T {
        STACKS.with(|cell| {
            let mut map = cell.borrow_mut();
            let result = {
                let frames = map.entry(self.inner.id).or_default();
                body(frames)
            };
            if map.get(&self.inner.id).is_some_and(|frames| frames.is_empty()) {
                map.remove(&self.inner.id);
            }
            result
        })
    }
}

enum HookKind {
    Cache,
    Feed,
    Commit,
    Rollback,
}

struct ResumeOnDrop<'a> {
    mgr: &'a TransactionManager,
}

impl Drop for ResumeOnDrop<'_> {
    fn drop(&mut self) {
        self.mgr.resume_parked_parent();
    }
}

fn active(frames: &[Frame]) -> Option<&Frame> {
    match frames.last() {
        Some(frame) if !frame.suspended => Some(frame),
        _ => None,
    }
}

fn active_mut(frames: &mut [Frame]) -> Option<&mut Frame> {
    match frames.last_mut() {
        Some(frame) if !frame.suspended => Some(frame),
        _ => None,
    }
}

fn try_commit(phase: &mut Phase) -> Result<(), Error> {
    let enlisted = phase.conn.borrow().is_some();
    if !enlisted {
        if phase.cache_flush.is_empty() && phase.feed_insert.is_empty() {
            return Ok(());
        }
        return Err(Error::new("no connection enlisted"));
    }
    if let Err(err) = run_conn_hooks(&phase.conn, std::mem::take(&mut phase.cache_flush)) {
        let _ = rollback_enlisted(&phase.conn);
        return Err(err);
    }
    if let Err(err) = run_conn_hooks(&phase.conn, std::mem::take(&mut phase.feed_insert)) {
        let _ = rollback_enlisted(&phase.conn);
        return Err(err);
    }
    let committed = {
        let mut slot = phase.conn.borrow_mut();
        let pooled = slot.as_mut().ok_or_else(|| Error::new("no connection enlisted"))?;
        pooled.conn().commit()
    };
    if let Err(err) = committed {
        let _ = rollback_enlisted(&phase.conn);
        return Err(Error::new("commit failed").with_source(err));
    }
    Ok(())
}

fn run_conn_hooks(conn: &Rc<RefCell<Option<PooledConn>>>, hooks: Vec<ConnHook>) -> Result<(), Error> {
    for hook in hooks {
        let mut slot = conn.borrow_mut();
        let pooled = slot.as_mut().ok_or_else(|| Error::new("no connection enlisted"))?;
        hook(pooled.conn())?;
    }
    Ok(())
}

fn rollback_enlisted(conn: &Rc<RefCell<Option<PooledConn>>>) -> Result<(), Error> {
    let mut slot = conn.borrow_mut();
    if let Some(pooled) = slot.as_mut() {
        pooled.conn().rollback()?;
    }
    Ok(())
}

fn return_conn(conn: &Rc<RefCell<Option<PooledConn>>>) {
    drop(conn.borrow_mut().take());
}
