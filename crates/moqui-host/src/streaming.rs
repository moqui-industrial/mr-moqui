//! HTTP body ownership, async polling and isolated, bounded blocking producers.
use std::io;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, mpsc};
use std::task::{Context, Poll};

use futures_util::Stream;
use moqui_core::web::{BlockingStreamSource, Cancelled, ResponseStream, StreamLifetime, StreamSource};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc as channel};

type Chunk = io::Result<Vec<u8>>;

pub(super) enum AdmissionError {
    Busy,
    Thread(io::Error),
}

pub(super) fn body(stream: ResponseStream) -> Result<axum::body::Body, AdmissionError> {
    static BLOCKING: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let limit = BLOCKING.get_or_init(|| {
        let count = moqui_lib::env::get_prop_or_env("webapp_stream_blocking_max")
            .parse::<usize>()
            .unwrap_or(16)
            .clamp(1, 1024);
        Arc::new(Semaphore::new(count))
    });
    Ok(axum::body::Body::from_stream(StreamBody::new(stream, limit.clone())?))
}

struct BlockingWorker {
    source: Box<dyn BlockingStreamSource>,
    // Drop the producer before returning its capacity, including during unwind.
    _permit: OwnedSemaphorePermit,
}

enum Source {
    Async(Pin<Box<dyn Stream<Item = Chunk> + Send>>),
    Blocking {
        demand: mpsc::SyncSender<()>,
        chunks: channel::Receiver<Option<Chunk>>,
        requested: bool,
    },
}

struct StreamBody {
    // Drop this guard before dropping the pending future/channels. It signals a
    // worker even when that worker is currently inside a producer call.
    lifetime: StreamLifetime,
    cancelled: Cancelled,
    source: Option<Source>,
}

impl StreamBody {
    fn new(stream: ResponseStream, limit: Arc<Semaphore>) -> Result<Self, AdmissionError> {
        let (lifetime, source) = stream.into_parts();
        let cancelled = lifetime.cancellation().cancelled();
        let source = match source {
            StreamSource::Async(source) => Source::Async(source),
            StreamSource::Blocking(source) => {
                // Fail before response headers; never queue an unbounded number
                // of blocking streams or occupy Tokio's shared blocking pool.
                let permit = limit.try_acquire_owned().map_err(|_| AdmissionError::Busy)?;
                let cancellation = lifetime.cancellation().clone();
                let (demand, requests) = mpsc::sync_channel(1);
                let (sender, chunks) = channel::channel(1);
                std::thread::Builder::new()
                    .name("moqui-http-stream".into())
                    .spawn(move || {
                        let mut worker = BlockingWorker {
                            source,
                            _permit: permit,
                        };
                        loop {
                            if cancellation.is_cancelled() {
                                break;
                            }
                            match requests.recv_timeout(std::time::Duration::from_millis(100)) {
                                Ok(()) => {}
                                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            }
                            if cancellation.is_cancelled() {
                                break;
                            }
                            let chunk = worker.source.next_chunk(&cancellation);
                            if cancellation.is_cancelled() {
                                break;
                            }
                            let terminal = chunk.as_ref().is_none_or(Result::is_err);
                            if sender.blocking_send(chunk).is_err() || terminal {
                                break;
                            }
                        }
                    })
                    .map_err(AdmissionError::Thread)?;
                Source::Blocking {
                    demand,
                    chunks,
                    requested: false,
                }
            }
        };
        Ok(Self {
            lifetime,
            cancelled,
            source: Some(source),
        })
    }

    fn finish(&mut self) {
        self.lifetime.cancellation().cancel();
        self.source = None;
    }
}

impl Stream for StreamBody {
    type Item = Chunk;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Chunk>> {
        use std::future::Future;
        let this = self.get_mut();
        if Pin::new(&mut this.cancelled).poll(cx).is_ready() {
            this.finish();
            return Poll::Ready(None);
        }
        let result = match this.source.as_mut() {
            None => return Poll::Ready(None),
            Some(Source::Async(source)) => source.as_mut().poll_next(cx),
            Some(Source::Blocking {
                demand,
                chunks,
                requested,
            }) => {
                if !*requested {
                    if demand.try_send(()).is_err() {
                        this.finish();
                        return Poll::Ready(Some(Err(io::Error::other("HTTP stream worker stopped unexpectedly"))));
                    }
                    *requested = true;
                }
                match chunks.poll_recv(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Some(chunk)) => {
                        *requested = false;
                        Poll::Ready(chunk)
                    }
                    Poll::Ready(None) => Poll::Ready(Some(Err(io::Error::other(
                        "HTTP stream worker stopped without completion",
                    )))),
                }
            }
        };
        match &result {
            Poll::Ready(Some(Err(error))) => {
                tracing::warn!(%error, "HTTP stream producer failed after response headers");
                this.finish();
            }
            Poll::Ready(None) => this.finish(),
            _ => {}
        }
        result
    }
}

#[cfg(test)]
mod response_stream_tests {
    use super::*;
    use futures_util::StreamExt;
    use moqui_core::web::{StreamCancellation, WebResponse};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    fn response(stream: ResponseStream) -> axum::response::Response {
        let mut response = WebResponse::new(200, Vec::new());
        response.stream = Some(stream);
        crate::http::from_web(response)
    }

    #[tokio::test]
    async fn async_stream_preserves_binary_chunks_and_closes_on_failure() {
        let cancellation = StreamCancellation::default();
        let source = futures_util::stream::iter([
            Ok(vec![0, 255, 1]),
            Err(io::Error::other("producer failed")),
            Ok(vec![2]),
        ]);
        let mut body = response(ResponseStream::from_async(source, cancellation.clone()))
            .into_body()
            .into_data_stream();
        assert_eq!(&body.next().await.unwrap().unwrap()[..], &[0, 255, 1]);
        assert!(body.next().await.unwrap().is_err());
        assert!(cancellation.is_cancelled());
        assert!(body.next().await.is_none());
    }

    #[tokio::test]
    async fn blocking_stream_pulls_on_demand_and_releases_on_disconnect() {
        struct Source(Arc<AtomicUsize>, Arc<AtomicBool>);
        impl moqui_core::web::BlockingStreamSource for Source {
            fn next_chunk(&mut self, _: &StreamCancellation) -> Option<Chunk> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Some(Ok(vec![0, 255, 1]))
            }
        }
        impl Drop for Source {
            fn drop(&mut self) {
                self.1.store(true, Ordering::SeqCst);
            }
        }
        let pulls = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let cancellation = StreamCancellation::default();
        let mut body = response(ResponseStream::from_blocking(
            Source(pulls.clone(), dropped.clone()),
            cancellation.clone(),
        ))
        .into_body()
        .into_data_stream();
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
        assert_eq!(&body.next().await.unwrap().unwrap()[..], &[0, 255, 1]);
        assert_eq!(pulls.load(Ordering::SeqCst), 1);
        drop(body);
        assert!(cancellation.is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(pulls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_releases_an_idle_worker_without_polling_the_body() {
        let cancellation = StreamCancellation::default();
        let limit = Arc::new(Semaphore::new(1));
        let source = |_: &StreamCancellation| -> Option<Chunk> { panic!("idle producer must not be read") };
        let body = StreamBody::new(
            ResponseStream::from_blocking(source, cancellation.clone()),
            limit.clone(),
        )
        .ok()
        .unwrap();
        assert_eq!(limit.available_permits(), 0);
        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), async {
            while limit.available_permits() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(body);
    }

    #[test]
    fn blocking_streams_are_bounded_and_do_not_use_the_shared_blocking_pool() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let limit = Arc::new(Semaphore::new(2));
            let starts = Arc::new(AtomicUsize::new(0));
            let mut tasks = Vec::new();
            for _ in 0..2 {
                let starts = starts.clone();
                let source = move |cancellation: &StreamCancellation| {
                    starts.fetch_add(1, Ordering::SeqCst);
                    assert!(cancellation.wait_timeout(Duration::from_secs(10)));
                    None
                };
                let mut body = StreamBody::new(
                    ResponseStream::from_blocking(source, StreamCancellation::default()),
                    limit.clone(),
                )
                .ok()
                .unwrap();
                tasks.push(tokio::spawn(async move { body.next().await }));
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                while starts.load(Ordering::SeqCst) != 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(matches!(
                StreamBody::new(
                    ResponseStream::from_blocking(|_: &StreamCancellation| None, StreamCancellation::default()),
                    limit.clone()
                ),
                Err(AdmissionError::Busy)
            ));
            // Async sources still admit, and the shared blocking pool still works.
            let async_body = StreamBody::new(
                ResponseStream::from_async(futures_util::stream::empty(), StreamCancellation::default()),
                limit.clone(),
            );
            assert!(async_body.is_ok());
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), tokio::task::spawn_blocking(|| 42))
                    .await
                    .unwrap()
                    .unwrap(),
                42
            );
            for task in tasks {
                task.abort();
                let _ = task.await;
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                while limit.available_permits() != 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
    }

    #[tokio::test]
    async fn explicit_cancellation_wakes_a_pending_async_source() {
        let pending = Arc::new(tokio::sync::Notify::new());
        let signal = pending.clone();
        let cancellation = StreamCancellation::default();
        let source = futures_util::stream::poll_fn(move |_| {
            signal.notify_one();
            Poll::<Option<Chunk>>::Pending
        });
        let mut body = response(ResponseStream::from_async(source, cancellation.clone()))
            .into_body()
            .into_data_stream();
        let task = tokio::spawn(async move { body.next().await });
        pending.notified().await;
        cancellation.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn application_error_events_are_opaque_data_and_keep_the_status() {
        let cancellation = StreamCancellation::default();
        let event = b"event: error\ndata: upstream unavailable\n\n".to_vec();
        let source = futures_util::stream::iter([Ok(event.clone())]);
        let mut response = WebResponse::new(200, Vec::new());
        response
            .headers
            .push(("Content-Type".into(), "text/event-stream".into()));
        response.stream = Some(ResponseStream::from_async(source, cancellation.clone()));
        let response = crate::http::from_web(response);
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let mut body = response.into_body().into_data_stream();
        assert_eq!(&body.next().await.unwrap().unwrap()[..], event);
        assert!(body.next().await.is_none());
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn http_disconnect_cancels_pending_upstream_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        let closed = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\nhello")
                .await
                .unwrap();
            let mut byte = [0];
            assert_eq!(
                socket.read(&mut byte).await.unwrap(),
                0,
                "upstream request must close on downstream disconnect"
            );
        });
        let pending = Arc::new(tokio::sync::Notify::new());
        let cancellation = StreamCancellation::default();
        let observed = cancellation.clone();
        let notification = pending.clone();
        let app = axum::Router::new().route(
            "/",
            axum::routing::get(move || {
                let cancellation = cancellation.clone();
                let pending = notification.clone();
                async move {
                    let mut socket = TcpStream::connect(upstream_address).await.unwrap();
                    socket
                        .write_all(b"GET /stream HTTP/1.1\r\nHost: localhost\r\n\r\n")
                        .await
                        .unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        header.push(socket.read_u8().await.unwrap());
                    }
                    let source =
                        futures_util::stream::unfold((socket, pending, 0), |(mut socket, pending, reads)| async move {
                            let mut bytes = vec![0; 4096];
                            if reads == 1 {
                                pending.notify_one();
                            }
                            match socket.read(&mut bytes).await {
                                Ok(0) => None,
                                Ok(size) => {
                                    bytes.truncate(size);
                                    Some((Ok(bytes), (socket, pending, reads + 1)))
                                }
                                Err(error) => Some((Err(error), (socket, pending, reads + 1))),
                            }
                        });
                    response(ResponseStream::from_async(source, cancellation))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut received = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !received.windows(5).any(|part| part == b"hello") {
                received.push(client.read_u8().await.unwrap());
            }
            // Register/consume notifications after the first payload; the next
            // read remains pending on the unfinished upstream HTTP response.
            pending.notified().await;
        })
        .await
        .unwrap();
        drop(client);
        tokio::time::timeout(Duration::from_secs(3), closed)
            .await
            .unwrap()
            .unwrap();
        assert!(observed.is_cancelled());
        server.abort();
    }
}
