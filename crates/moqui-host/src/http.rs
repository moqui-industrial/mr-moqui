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
use std::sync::mpsc::Sender;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::response::Response;
use axum::routing::{any, get};
use axum::Router;
use moqui_core::context::ErrorKind;
use moqui_core::context::ExecutionContextFactory;
use moqui_core::transaction::PoolStats;
use moqui_core::web::{WebRequest, WebResponse};
use moqui_lib::error::Error;
use tokio::net::TcpListener;

use crate::{notws, search_proxy};

pub(crate) struct HttpState {
    pub factory: Arc<ExecutionContextFactory>,
    pub shutdown: tokio::sync::watch::Receiver<bool>,
}

/// Addresses and handles the test uses after the plain HTTP listener binds.
pub(crate) struct ListenReport {
    pub http: SocketAddr,
    pub cluster: SocketAddr,
    pub factory: Arc<ExecutionContextFactory>,
    pub posts: Arc<std::sync::Mutex<Vec<String>>>,
}

/// Bind `bind`, publish the local address, and serve until `stop` is dropped or a ctrl-c arrives.
pub(crate) async fn listen(
    factory: Arc<ExecutionContextFactory>,
    bind: SocketAddr,
    cluster: SocketAddr,
    posts: Arc<std::sync::Mutex<Vec<String>>>,
    stop: tokio::sync::oneshot::Receiver<()>,
    ready: Sender<Result<ListenReport, String>>,
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
    // Text responses are compressed when the client accepts gzip. Images and other types that are
    // compressed already are left alone by the layer's default predicate.
    let _ = ready.send(Ok(ListenReport {
        http: address,
        cluster,
        factory: Arc::clone(&factory),
        posts,
    }));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/notws", get(notws::upgrade))
        .route("/elastic", any(search_proxy::elastic))
        .route("/elastic/", any(search_proxy::elastic))
        .route("/elastic/{*rest}", any(search_proxy::elastic))
        .route("/kibana", any(search_proxy::kibana))
        .route("/kibana/", any(search_proxy::kibana))
        .route("/kibana/{*rest}", any(search_proxy::kibana))
        .fallback(dispatch_screen)
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(Arc::new(HttpState {
            factory,
            shutdown: shutdown_rx,
        }));
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown(stop, shutdown_tx))
        .await
        .map_err(|err| Error::new(format!("http listener stopped: {err}")))
}

/// Ctrl-c ends the binary. A failed handler waits on `stop`, which is how tests shut the listener down.
/// The watch tells open notification sockets to close so graceful shutdown can finish.
async fn shutdown(mut stop: tokio::sync::oneshot::Receiver<()>, notify: tokio::sync::watch::Sender<bool>) {
    tokio::select! {
        _ = &mut stop => {}
        result = tokio::signal::ctrl_c() => {
            if result.is_err() {
                let _ = stop.await;
            }
        }
    }
    let _ = notify.send(true);
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
    let mut text = format!(
        "checked_out {}\nparked {}\nwaiting {}\nidle {}\ncheckouts {}\nrequest_queue {}\nstatements {}\nstatement_ms {}\n",
        stats.checked_out, stats.parked, stats.waiting, idle, stats.checkouts, depth, statements, statement_ms
    );
    // With a search read replica: its replay lag (-1 when it cannot be read) and the reads that ran on the primary.
    let factory = Arc::clone(&state.factory);
    let replica = tokio::task::spawn_blocking(move || {
        moqui_core::search::installed(&factory).and_then(|facade| facade.backend().replica_status())
    })
    .await
    .ok()
    .flatten();
    if let Some(replica) = replica {
        let lag = replica.lag_seconds.unwrap_or(-1.0);
        text.push_str(&format!(
            "search_replica_lag_seconds {lag}\nsearch_replica_primary_fallbacks {}\n",
            replica.primary_fallbacks
        ));
    }
    plain(200, &text)
}

async fn dispatch_screen(
    State(state): State<Arc<HttpState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let started = std::time::Instant::now();
    let started_millis = crate::logging::now_millis();
    let (parts, body) = request.into_parts();
    let mut web = WebRequest::new(parts.method.as_str(), parts.uri.path());
    web.https = forwarded_https(&parts.headers);
    web.peer_ip = peer.ip().to_string();
    web.headers = header_map(&parts.headers);
    web.query = moqui_core::web::form_pairs(parts.uri.query().unwrap_or(""));
    // The limit is `webapp_max_body` (10 MiB), or `webapp_max_upload` for a multipart body, and it stops the read.
    let limit = moqui_core::web::body_limit(&web);
    let declared = parts
        .headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared.is_some_and(|length| length > limit) {
        return plain(413, &format!("request body is larger than {limit} bytes"));
    }
    let bytes = match axum::body::to_bytes(body, limit.saturating_add(1)).await {
        Ok(bytes) if bytes.len() <= limit => bytes,
        _ => return plain(413, &format!("request body is larger than {limit} bytes")),
    };
    web.body = bytes.to_vec();
    let log = RequestLog {
        started_millis,
        client_ip: moqui_core::web::client_ip(&web),
        method: parts.method.as_str().to_string(),
        https: web.https,
        host: web.header("host").unwrap_or("").to_string(),
        path: parts.uri.path().to_string(),
        query: parts.uri.query().unwrap_or("").to_string(),
        version: parts.version,
        referrer: web.header("referer").unwrap_or("").to_string(),
        agent: web.header("user-agent").unwrap_or("").to_string(),
    };
    let factory = Arc::clone(&state.factory);
    let job_factory = Arc::clone(&factory);
    let serve_factory = Arc::clone(&factory);
    let (done, result) = tokio::sync::oneshot::channel();
    // The job owns `done`. A worker that panics or a pool that stops drops it, which ends the wait.
    let submitted = factory.request_pool().try_submit(job_factory, move || {
        let _ = done.send(moqui_core::screen::serve(&serve_factory, web));
        Ok(())
    });
    if let Err(err) = submitted {
        if err.kind() == ErrorKind::Unavailable {
            return plain(503, "service unavailable");
        }
        return plain(500, err.message());
    }
    match result.await {
        Ok(response) => {
            log.queue(&response, started.elapsed());
            from_web(response)
        }
        Err(_) => plain(500, "request worker stopped"),
    }
}

/// One `moqui_http_log` entry, the fields of Java's `ElasticRequestLogFilter`. Queued only while
/// logs ship to the search backend.
struct RequestLog {
    started_millis: i64,
    client_ip: String,
    method: String,
    https: bool,
    host: String,
    path: String,
    query: String,
    version: axum::http::Version,
    referrer: String,
    agent: String,
}

impl RequestLog {
    fn queue(self, response: &WebResponse, elapsed: std::time::Duration) {
        use moqui_lib::object::Value;

        let millis = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);
        let version = match self.version {
            axum::http::Version::HTTP_09 => 0.9,
            axum::http::Version::HTTP_10 => 1.0,
            axum::http::Version::HTTP_2 => 2.0,
            axum::http::Version::HTTP_3 => 3.0,
            _ => 1.1,
        };
        let content_type = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.clone());
        let mut entry = moqui_core::entity::document::Document::new();
        entry.insert("@timestamp".into(), Value::I64(self.started_millis));
        entry.insert("remote_ip".into(), Value::String(self.client_ip));
        entry.insert("server_ip".into(), Value::string(crate::logging::host_name()));
        if let Some(content_type) = content_type {
            entry.insert("content_type".into(), Value::String(content_type));
        }
        entry.insert("request_method".into(), Value::String(self.method));
        entry.insert("request_scheme".into(), Value::string(if self.https { "https" } else { "http" }));
        entry.insert("request_host".into(), Value::String(self.host));
        entry.insert("request_path".into(), Value::String(self.path));
        if !self.query.is_empty() {
            entry.insert("request_query".into(), Value::String(self.query));
        }
        entry.insert("http_version".into(), Value::F64(version));
        entry.insert("response".into(), Value::I64(i64::from(response.status)));
        entry.insert("time_initial_ms".into(), Value::I64(millis));
        entry.insert("time_final_ms".into(), Value::I64(millis));
        entry.insert(
            "bytes".into(),
            Value::I64(i64::try_from(response.body.len()).unwrap_or(i64::MAX)),
        );
        if !self.referrer.is_empty() {
            entry.insert("referrer".into(), Value::String(self.referrer));
        }
        if !self.agent.is_empty() {
            entry.insert("agent".into(), Value::String(self.agent));
        }
        crate::logging::queue_http_request(entry);
    }
}

/// `X-Forwarded-Proto` is believed only when `webapp_client_ip_header` marks a reverse proxy in front.
/// A direct client could otherwise force `Secure` cookies and HSTS.
fn forwarded_https(headers: &axum::http::HeaderMap) -> bool {
    if moqui_lib::env::get_prop_or_env("webapp_client_ip_header").is_empty() {
        return false;
    }
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

pub(crate) fn header_map(headers: &axum::http::HeaderMap) -> BTreeMap<String, String> {
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

pub(crate) fn plain(status: u16, body: &str) -> Response {
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
