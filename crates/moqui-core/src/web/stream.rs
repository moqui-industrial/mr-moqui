//! Runtime-independent, exclusively owned HTTP stream sources and cancellation.
use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_core::Stream;

pub type StreamChunk = io::Result<Vec<u8>>;

#[derive(Default)]
struct CancellationState {
    cancelled: bool,
    next_id: u64,
    wakers: BTreeMap<u64, Waker>,
    callbacks: Vec<Box<dyn FnOnce() + Send>>,
}

/// A control handle, never a handle to the producer or its data. Cancellation is
/// idempotent and also occurs when the HTTP body ends or is dropped.
#[derive(Clone, Default)]
pub struct StreamCancellation(Arc<(Mutex<CancellationState>, Condvar)>);

impl StreamCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.0.0.lock().unwrap_or_else(|e| e.into_inner()).cancelled
    }

    /// Signal cancellation and wake synchronous/asynchronous waiters. Hooks run
    /// synchronously outside the lock and MUST be nonblocking (for example, abort
    /// an upstream task or shut down a socket). Body destruction will not poll
    /// the source again; awaited protocol cleanup needs a separately supervised,
    /// bounded task.
    pub fn cancel(&self) {
        let (wakers, callbacks) = {
            let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            (std::mem::take(&mut state.wakers), std::mem::take(&mut state.callbacks))
        };
        self.0.1.notify_all();
        for (_, waker) in wakers {
            waker.wake();
        }
        for callback in callbacks {
            run_callback(callback);
        }
    }

    pub fn on_cancel(&self, callback: impl FnOnce() + Send + 'static) {
        let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.cancelled {
            drop(state);
            run_callback(Box::new(callback));
        } else {
            state.callbacks.push(Box::new(callback));
        }
    }

    pub fn cancelled(&self) -> Cancelled {
        Cancelled {
            cancellation: self.clone(),
            id: None,
        }
    }

    /// Interruptible delay for a blocking producer. Returns true if cancelled.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        self.0
            .1
            .wait_timeout_while(state, timeout, |s| !s.cancelled)
            .unwrap_or_else(|e| e.into_inner())
            .0
            .cancelled
    }
}

fn run_callback(callback: Box<dyn FnOnce() + Send>) {
    // Cleanup must not double-panic while the HTTP body is being unwound.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)).is_err() {
        tracing::warn!("HTTP stream cancellation hook panicked");
    }
}

/// A runtime-independent cancellation future. Dropped waiters unregister their waker.
pub struct Cancelled {
    cancellation: StreamCancellation,
    id: Option<u64>,
}

impl Future for Cancelled {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let mut state = this.cancellation.0.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.cancelled {
            return Poll::Ready(());
        }
        let id = *this.id.get_or_insert_with(|| {
            state.next_id += 1;
            state.next_id
        });
        state.wakers.insert(id, cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Cancelled {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.cancellation
                .0
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .wakers
                .remove(&id);
        }
    }
}

/// A blocking producer must check cancellation and bound ALL blocking operations,
/// including lock acquisition and reads. Rust cannot forcibly stop an OS thread.
/// The host admits a bounded number of these sources on dedicated threads.
pub trait BlockingStreamSource: Send + 'static {
    fn next_chunk(&mut self, cancellation: &StreamCancellation) -> Option<StreamChunk>;
}

impl<F> BlockingStreamSource for F
where
    F: FnMut(&StreamCancellation) -> Option<StreamChunk> + Send + 'static,
{
    fn next_chunk(&mut self, cancellation: &StreamCancellation) -> Option<StreamChunk> {
        self(cancellation)
    }
}

/// Transport handoff. This enum is owned and deliberately not clonable.
pub enum StreamSource {
    Async(Pin<Box<dyn Stream<Item = StreamChunk> + Send>>),
    Blocking(Box<dyn BlockingStreamSource>),
}

/// The transport owns this guard independently of any in-flight producer work.
pub struct StreamLifetime(StreamCancellation);

impl StreamLifetime {
    pub fn cancellation(&self) -> &StreamCancellation {
        &self.0
    }
}

impl Drop for StreamLifetime {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// An exclusively owned response source. Moving it into `send_stream` transfers
/// ownership to HTTP; a service may keep only a cancellation handle.
/// Async sources must release owned upstream responses/futures on drop, and
/// register an on_cancel hook for detached upstream tasks requiring an abort.
pub struct ResponseStream {
    lifetime: StreamLifetime,
    source: StreamSource,
}

impl std::fmt::Debug for ResponseStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResponseStream")
    }
}

impl ResponseStream {
    pub fn from_async(
        source: impl Stream<Item = StreamChunk> + Send + 'static,
        cancellation: StreamCancellation,
    ) -> Self {
        Self {
            lifetime: StreamLifetime(cancellation),
            source: StreamSource::Async(Box::pin(source)),
        }
    }

    pub fn from_blocking(source: impl BlockingStreamSource, cancellation: StreamCancellation) -> Self {
        Self {
            lifetime: StreamLifetime(cancellation),
            source: StreamSource::Blocking(Box::new(source)),
        }
    }

    /// Consuming handoff to the HTTP transport; there is no shared read handle.
    pub fn into_parts(self) -> (StreamLifetime, StreamSource) {
        (self.lifetime, self.source)
    }
}

#[cfg(test)]
mod response_stream_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn cancellation_wakes_blocking_waiters_and_runs_hooks_once() {
        let cancellation = StreamCancellation::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        cancellation.on_cancel(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let waiter = cancellation.clone();
        let thread = std::thread::spawn(move || waiter.wait_timeout(Duration::from_secs(30)));
        cancellation.cancel();
        cancellation.cancel();
        assert!(thread.join().unwrap());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let counter = calls.clone();
        cancellation.on_cancel(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn dropping_an_unsubmitted_stream_also_cancels() {
        let cancellation = StreamCancellation::default();
        drop(ResponseStream::from_blocking(
            |_: &StreamCancellation| None,
            cancellation.clone(),
        ));
        assert!(cancellation.is_cancelled());
    }
}
