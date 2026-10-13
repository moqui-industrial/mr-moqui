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

//! Plain HTTP listener. Axum accepts the socket. Screen and entity work runs on the request pool.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, Request, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use moqui_core::context::ExecutionContextFactory;
use moqui_core::transaction::PoolStats;
use moqui_core::web::{WebRequest, WebResponse};
use moqui_lib::error::Error;
use tokio::net::TcpListener;

const BODY_LIMIT: usize = 1024 * 1024;

struct HttpState {
    factory: Arc<ExecutionContextFactory>,
}

/// Bind `bind`, publish the local address, and serve until `stop` is dropped or a ctrl-c arrives.
pub(crate) async fn listen(
    factory: Arc<ExecutionContextFactory>,
    bind: SocketAddr,
    stop: tokio::sync::oneshot::Receiver<()>,
    ready: Sender<Result<SocketAddr, String>>,
) -> Result<(), Error> {
    let listener = match TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            let message = format!("could not bind {bind}: {err}");
            let _ = ready.send(Err(message.clone()));
            return Err(Error::new(message));
        }
    };
    let address = match listener.local_addr() {
        Ok(address) => address,
        Err(err) => {
            let message = format!("bound listener has no address: {err}");
            let _ = ready.send(Err(message.clone()));
            return Err(Error::new(message));
        }
    };
    let _ = ready.send(Ok(address));
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .fallback(dispatch_screen)
        .with_state(Arc::new(HttpState { factory }));
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown(stop))
        .await
        .map_err(|err| Error::new(format!("http listener stopped: {err}")))
}

/// Ctrl-c ends the binary. A failed handler waits on `stop`, which is how tests shut the listener down.
async fn shutdown(mut stop: tokio::sync::oneshot::Receiver<()>) {
    tokio::select! {
        _ = &mut stop => {}
        result = tokio::signal::ctrl_c() => {
            if result.is_err() {
                let _ = stop.await;
            }
        }
    }
}

/// Empty `webapp_http_port` is 8080.
pub(crate) fn http_port(raw: &str) -> Result<u16, Error> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(8080);
    }
    raw.parse::<u16>()
        .map_err(|_| Error::new(format!("webapp_http_port {raw} is not a port")))
}

async fn health(State(state): State<Arc<HttpState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if !status_allowed(&peer.ip().to_string()) {
        return plain(404, "not found");
    }
    if readiness(&state.factory) {
        json(200, "{\"ready\":true}")
    } else {
        json(503, "{\"ready\":false}")
    }
}

async fn metrics(State(state): State<Arc<HttpState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if !status_allowed(&peer.ip().to_string()) {
        return plain(404, "not found");
    }
    let Some(stats) = pool_stats(&state.factory) else {
        return plain(503, "service unavailable");
    };
    let depth = state.factory.request_pool().queue_depth();
    let (statements, statement_ms) = moqui_core::entity::statement_stats();
    let idle = stats.ordinary_idle.saturating_add(stats.reserve_idle);
    plain(
        200,
        &format!(
            "checked_out {}\nparked {}\nwaiting {}\nidle {}\ncheckouts {}\nrequest_queue {}\nstatements {}\nstatement_ms {}\n",
            stats.checked_out, stats.parked, stats.waiting, idle, stats.checkouts, depth, statements, statement_ms
        ),
    )
}

async fn dispatch_screen(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, BODY_LIMIT + 1).await {
        Ok(bytes) => bytes,
        Err(_) => return plain(413, "request body is larger than 1 MiB"),
    };
    let mut web = WebRequest::new(parts.method.as_str(), parts.uri.path());
    web.https = forwarded_https(&parts.headers);
    web.peer_ip = peer.ip().to_string();
    web.headers = header_map(&parts.headers);
    web.query = query_map(parts.uri.query().unwrap_or(""));
    web.body = bytes.to_vec();
    let factory = Arc::clone(&state.factory);
    let job_factory = Arc::clone(&factory);
    let serve_factory = Arc::clone(&factory);
    // `try_submit` borrows `factory`. The job keeps its own clone so the borrow can end.
    let submitted = factory
        .request_pool()
        .try_submit(job_factory, move || Ok(moqui_core::screen::serve(&serve_factory, web)));
    let receiver = match submitted {
        Ok(receiver) => receiver,
        Err(err) => {
            let message = err.message();
            if message.contains("request queue is full") || message.contains("request pool is not running") {
                return plain(503, "service unavailable");
            }
            return plain(500, message);
        }
    };
    match wait_for(receiver).await {
        Ok(response) => from_web(response),
        Err(err) => plain(500, &err),
    }
}

fn forwarded_https(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("https")
        })
}

fn header_map(headers: &axum::http::HeaderMap) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for name in headers.keys() {
        let values: Vec<&str> = headers
            .get_all(name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect();
        if values.is_empty() {
            continue;
        }
        map.insert(name.as_str().to_ascii_lowercase(), values.join(", "));
    }
    map
}

fn query_map(query: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        map.insert(percent_decode(key), percent_decode(value));
    }
    map
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let hex = &text[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => out.push(byte),
                    Err(_) => out.extend_from_slice(&bytes[index..index + 3]),
                }
                index += 3;
                continue;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn wait_for(receiver: Receiver<Result<WebResponse, moqui_core::context::Error>>) -> Result<WebResponse, String> {
    loop {
        match receiver.try_recv() {
            Ok(Ok(response)) => return Ok(response),
            Ok(Err(err)) => return Err(err.to_string()),
            Err(TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(1)).await,
            Err(TryRecvError::Disconnected) => return Err("request worker stopped".to_string()),
        }
    }
}

fn readiness(factory: &ExecutionContextFactory) -> bool {
    pool_stats(factory).is_some()
}

fn pool_stats(factory: &ExecutionContextFactory) -> Option<PoolStats> {
    let facade = factory.entity()?;
    let transactions = facade.transactions_for("moqui.basic.Enumeration").ok()?;
    Some(transactions.stats())
}

fn status_allowed(ip: &str) -> bool {
    let mut allowed = Vec::new();
    let configured = moqui_lib::env::get_prop_or_env("webapp_status_ips");
    for part in configured.split(',') {
        let part = part.trim();
        if !part.is_empty() {
            allowed.push(part.to_string());
        }
    }
    for extra in ["127.0.0.1", "::1", "0:0:0:0:0:0:0:1"] {
        if !allowed.iter().any(|item| item == extra) {
            allowed.push(extra.to_string());
        }
    }
    allowed.iter().any(|item| item == ip)
}

fn from_web(response: WebResponse) -> Response {
    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(axum::body::Body::from(response.body))
        .unwrap_or_else(|_| plain(500, "could not build the response"))
}

fn json(status: u16, body: &str) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap_or_else(|_| plain(500, "could not build the response"))
}

fn plain(status: u16, body: &str) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty()))
}

#[cfg(test)]
mod tests {
    use super::{http_port, status_allowed};

    #[test]
    fn empty_webapp_http_port_is_8080() {
        assert_eq!(http_port("").unwrap(), 8080);
        assert_eq!(http_port("  ").unwrap(), 8080);
        assert_eq!(http_port("9090").unwrap(), 9090);
        assert!(http_port("nope").is_err());
    }

    #[test]
    fn status_routes_allow_loopback_only() {
        assert!(status_allowed("127.0.0.1"));
        assert!(status_allowed("::1"));
        assert!(status_allowed("0:0:0:0:0:0:0:1"));
        let configured = moqui_lib::env::get_prop_or_env("webapp_status_ips");
        if !configured.split(',').any(|part| part.trim() == "203.0.113.5") {
            assert!(!status_allowed("203.0.113.5"));
        }
    }
}
