# Native HTTP streaming

Native services can send finite or continuous HTTP response bodies without
buffering the complete result. The framework transports opaque byte chunks;
the producer chooses the content type, encoding and application framing.
The API is independent of media, SSE and provider-specific message formats.

Call `moqui_core::web::send_stream(content_type, response_stream)` from an HTTP
screen transition and use its `none` response. There must be an active HTTP
request, and a response must not already have been sent. Service authorization,
input validation and initial upstream setup should complete before handoff.
Errors at that stage follow the normal HTTP error path.

## Ownership and cancellation

`ResponseStream` and `WebResponse` are not clonable. `send_stream` consumes the
source, transferring exclusive ownership to the HTTP transport. A service may
retain a `StreamCancellation` handle; it cannot read or retain a shared clone of
the producer after handoff. `into_parts` is a consuming transport-adapter API.

The HTTP body owns a cancellation guard independently of in-flight producer work.
Disconnect, completion, producer error, failed admission or a rejected handoff
signals cancellation. The signal is idempotent. Producers can use:

- `is_cancelled()` to check the signal;
- `cancelled().await` to wait asynchronously;
- `wait_timeout(duration)` for an interruptible blocking delay;
- `on_cancel(hook)` to register a nonblocking cleanup action, such as aborting
  a detached upstream request task or shutting down a socket.

Hooks run synchronously on cancellation, outside the control lock. They must
not block or perform awaited cleanup. Body destruction will not poll the source
again; an async close handshake needs a separately supervised cleanup task with
a bounded deadline. A retained cancellation handle does not retain the response source. Use a separate signal per response unless group
cancellation is intentional.

## Async sources

`ResponseStream::from_async(source, cancellation)` accepts any owned
`futures_core::Stream<Item = std::io::Result<Vec<u8>>> + Send + 'static`.
The source is polled directly on the HTTP runtime. It must yield promptly and
must not perform blocking reads, sleeps or CPU-intensive work in `poll_next`.
Construct runtime-dependent operations lazily in its futures; native service
setup itself runs on a synchronous request worker.

The transport drops the pending source when cancelled. Own upstream response
bodies, sockets and pending futures inside that source so they close on drop.
For detached upstream tasks, register an abort hook; dropping a join handle
alone does not necessarily cancel its task. Configure upstream connection/read
deadlines as well. These rules apply to any upstream HTTP service, including
token-generating APIs; the framework does not manage provider-specific clients.

For example, with a component dependency on `futures-util`:

```rust
use moqui_core::web::{self, ResponseStream, StreamCancellation};

let cancellation = StreamCancellation::default();
let source = futures_util::stream::iter([
    Ok(b"first line\n".to_vec()),
    Ok(b"second line\n".to_vec()),
]);
web::send_stream("text/plain; charset=utf-8",
    ResponseStream::from_async(source, cancellation))?;
```

Async sources do not reserve blocking-stream workers. Expected concurrency is
bounded by `webapp_max_connections` (default 1024 shared HTTP/HTTPS connections),
deployment limits, source memory, upstream quotas and producer behavior. No process-wide async stream admission limit is
introduced here; deployments must size those limits for their workload.

## Blocking sources

Use `ResponseStream::from_blocking(source, cancellation)` with a
`BlockingStreamSource`, or a closure of the equivalent signature:

```rust
use moqui_core::web::{self, ResponseStream, StreamCancellation};
use std::time::Duration;

let cancellation = StreamCancellation::default();
let source = |cancel: &StreamCancellation| {
    if cancel.wait_timeout(Duration::from_millis(100)) {
        return None;
    }
    Some(Ok(b"sample\n".to_vec()))
};
web::send_stream("text/plain; charset=utf-8",
    ResponseStream::from_blocking(source, cancellation))?;
```

Each admitted blocking response owns one dedicated OS thread. Producers never
use Tokio's shared blocking pool. `webapp_stream_blocking_max` defaults to **16**
per process, is read on first stream admission and accepts 1..1024. Invalid
numbers fall back to 16; numeric values outside the range are clamped. Excess
blocking streams receive **HTTP 503 before their headers are sent**, without a
waiting worker queue. A thread-start failure returns HTTP 500.

Only one chunk is requested at a time, using bounded demand/result channels.
The transport does not read ahead while a client is not consuming the body;
socket buffering still applies. Producers must bound their own chunk sizes.

Cancellation wakes cooperative delays immediately. Every blocking operation,
including locks and upstream reads, must have a deadline or a cancellation
mechanism. Rust cannot safely kill an arbitrary blocked thread. The worker's
admission permit remains held until it actually exits, so an uncooperative
producer cannot cause unbounded replacement threads. Long-lived network
streams should normally use the async source API.

## Failures after headers

Each chunk is `Ok(bytes)` or `Err(io::Error)`; end of stream is `None`.
After HTTP headers have been committed, a producer error **cannot change the
HTTP status or replace the response with a JSON error**. The host logs the error,
cancels the source and terminates the body with a transport error. Clients must
treat an interrupted body without their expected application terminator as an
incomplete response. An empty source simply completes the body.

If the application requires a structured error, the producer must encode it as
a normal chunk in the selected protocol, then end the stream. For SSE this may
be an `event: error` message; for JSONL it may be an error record. That is a
successful byte transfer with an application-level failure, so the existing
HTTP status remains unchanged. The framework does not inject events, interpret
provider errors, or replay partially delivered streams.

`moqui-core` depends only on the runtime-independent stream trait and standard
library cancellation primitives; Tokio and HTTP execution remain in `moqui-host`.
