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

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::routing::get;
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use super::{header_map, http_port, serve, status_allowed, ListenLimits, TlsConnection};
use crate::guard::Guard;
use crate::tls;

fn unguarded() -> Arc<Guard> {
    Arc::new(Guard::disabled())
}

async fn read_all(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .expect("the listener did not close the connection")
        .unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn a_screen_request_is_logged_with_its_user_and_health_without_one() {
    use moqui_lib::object::Value;
    use tower::ServiceExt as _;

    let _hold = crate::logging::begin_http_capture().await;
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/screen",
            get(|| async {
                let mut response = axum::response::Response::new(axum::body::Body::from("hi"));
                response.extensions_mut().insert(super::LoggedIdentity {
                    session: Some("session-1".to_string()),
                    visitor_id: Some("visitor-1".to_string()),
                    remote_user: Some("ada".to_string()),
                });
                response
            }),
        )
        .layer(axum::middleware::from_fn(super::log_http));
    let screen = Request::builder()
        .uri("/screen?tab=1")
        .body(axum::body::Body::empty())
        .unwrap();
    app.clone().oneshot(screen).await.unwrap();
    let health = Request::builder()
        .uri("/health")
        .body(axum::body::Body::empty())
        .unwrap();
    app.oneshot(health).await.unwrap();
    let entries = crate::logging::take_http_capture();
    let screen = entries
        .iter()
        .find(|entry| entry.get("request_path").and_then(Value::as_str) == Some("/screen"))
        .expect("screen request was not logged");
    assert_eq!(screen.get("session").and_then(Value::as_str), Some("session-1"));
    assert_eq!(screen.get("visitor_id").and_then(Value::as_str), Some("visitor-1"));
    assert_eq!(screen.get("remote_user").and_then(Value::as_str), Some("ada"));
    assert_eq!(screen.get("request_query").and_then(Value::as_str), Some("tab=1"));
    let health = entries
        .iter()
        .find(|entry| entry.get("request_path").and_then(Value::as_str) == Some("/health"))
        .expect("health request was not logged");
    assert!(health.get("session").is_none(), "{health:?}");
    assert!(health.get("visitor_id").is_none(), "{health:?}");
    assert!(health.get("remote_user").is_none(), "{health:?}");
}

#[tokio::test]
async fn a_slow_request_head_is_closed_and_holds_no_connection_slot() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route("/", get(|| async { "ok" }));
    let limits = ListenLimits {
        header: Some(Duration::from_millis(300)),
        connections: 1,
        shutdown: None,
        ..ListenLimits::defaults()
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let permits = Arc::new(Semaphore::new(limits.connections));
    let server = tokio::spawn(serve(listener, None, app, limits, unguarded(), permits, async move {
        let _ = stopped.await;
    }));

    // The one slot is taken by a client that never finishes its request head.
    let started = Instant::now();
    let mut slow = TcpStream::connect(address).await.unwrap();
    slow.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n").await.unwrap();
    // A full request waits for that slot, which the header timeout frees.
    let mut fast = TcpStream::connect(address).await.unwrap();
    fast.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let answered = read_all(&mut fast).await;
    assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );

    let closed = read_all(&mut slow).await;
    assert!(!closed.contains(" 200 "), "{closed}");
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());

    drop(stop);
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the listener did not stop")
        .unwrap();
}

#[tokio::test]
async fn a_stop_waits_for_open_requests_only_until_the_shutdown_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route("/", get(std::future::pending::<&'static str>));
    let limits = ListenLimits {
        header: None,
        connections: 4,
        shutdown: Some(Duration::from_millis(300)),
        ..ListenLimits::defaults()
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let permits = Arc::new(Semaphore::new(limits.connections));
    let server = tokio::spawn(serve(listener, None, app, limits, unguarded(), permits, async move {
        let _ = stopped.await;
    }));
    let mut stuck = TcpStream::connect(address).await.unwrap();
    stuck.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let started = Instant::now();
    drop(stop);
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the listener waited past its shutdown deadline")
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "{:?}",
        started.elapsed()
    );
}

/// A request over the TLS listener reaches the router marked as secure, and one over plain HTTP does not.
#[tokio::test]
async fn a_request_over_tls_is_marked_secure() {
    let dir = tls::tests::temp_dir("serve").join("cert");
    tls::ensure_dev_cert(&dir).unwrap();
    let config = Arc::new(tls::server_config(tls::CertStore::new(dir.clone())).unwrap());
    let app = Router::new().route(
        "/",
        get(|request: Request| async move {
            if request.extensions().get::<TlsConnection>().is_some() {
                "secure"
            } else {
                "plain"
            }
        }),
    );
    let limits = ListenLimits {
        header: Some(Duration::from_secs(5)),
        connections: 4,
        shutdown: None,
        ..ListenLimits::defaults()
    };
    let permits = Arc::new(Semaphore::new(limits.connections));
    let (stop_secure, secure_stopped) = tokio::sync::oneshot::channel::<()>();
    let (stop_plain, plain_stopped) = tokio::sync::oneshot::channel::<()>();
    let secure_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let secure_address = secure_listener.local_addr().unwrap();
    let plain_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plain_address = plain_listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        tokio::join!(
            serve(
                secure_listener,
                Some(tokio_rustls::TlsAcceptor::from(config)),
                app.clone(),
                limits,
                unguarded(),
                Arc::clone(&permits),
                async move {
                    let _ = secure_stopped.await;
                },
            ),
            serve(plain_listener, None, app, limits, unguarded(), permits, async move {
                let _ = plain_stopped.await;
            }),
        );
    });

    let stream = TcpStream::connect(secure_address).await.unwrap();
    let mut secure = tls::tests::client_trusting(&dir.join("localhost.crt"))
        .connect(rustls::pki_types::ServerName::try_from("localhost").unwrap(), stream)
        .await
        .expect("localhost handshake");
    secure
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut answered = String::new();
    let _ = secure.read_to_string(&mut answered).await;
    assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
    assert!(answered.ends_with("secure"), "{answered}");

    let mut plain = TcpStream::connect(plain_address).await.unwrap();
    plain
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let answered = read_all(&mut plain).await;
    assert!(answered.ends_with("plain"), "{answered}");

    drop(stop_secure);
    drop(stop_plain);
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the listeners did not stop")
        .unwrap();
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// A listener with `guard`, answering `ok` to everything the guard lets through, until the sender drops.
async fn guarded(
    guard: Arc<Guard>,
    limits: ListenLimits,
) -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(|| async { "ok" });
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let permits = Arc::new(Semaphore::new(limits.connections));
    let server = tokio::spawn(serve(listener, None, app, limits, guard, permits, async move {
        let _ = stopped.await;
    }));
    (address, stop, server)
}

async fn ask(address: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    read_all(&mut stream).await
}

#[tokio::test]
async fn the_guard_and_the_request_limits_answer_on_the_socket() {
    let guard = crate::guard::tests::guard(
        r#"<web-protection enabled="true">
                <rate-limit name="all" requests="1" seconds="60" burst="2"/>
                <path-rule action="deny" path="/.env"/>
            </web-protection>"#,
        "",
    );
    let limits = ListenLimits {
        header: Some(Duration::from_secs(5)),
        max_uri: 64,
        max_header_bytes: 8192,
        ..ListenLimits::defaults()
    };
    let (address, stop, server) = guarded(guard, limits).await;
    let get = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");

    let answered = ask(address, &get("/.env")).await;
    assert!(answered.starts_with("HTTP/1.1 404"), "{answered}");
    // a burst of two, then the rate of one a minute
    for _ in 0..2 {
        let answered = ask(address, &get("/")).await;
        assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
    }
    let answered = ask(address, &get("/")).await;
    assert!(answered.starts_with("HTTP/1.1 429"), "{answered}");
    assert!(answered.to_ascii_lowercase().contains("retry-after: "), "{answered}");

    let long = format!("/{}", "a".repeat(100));
    let answered = ask(address, &get(&long)).await;
    assert!(answered.starts_with("HTTP/1.1 414"), "{answered}");
    let big = format!(
        "GET / HTTP/1.1\r\nHost: x\r\nX-Big: {}\r\nConnection: close\r\n\r\n",
        "b".repeat(9000)
    );
    let answered = ask(address, &big).await;
    assert!(answered.starts_with("HTTP/1.1 431"), "{answered}");

    drop(stop);
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the listener did not stop")
        .unwrap();
}

#[tokio::test]
async fn a_client_over_its_connections_is_closed_before_its_request() {
    let guard = crate::guard::tests::guard(
        r#"<web-protection enabled="true"><connections max-per-client="1"/></web-protection>"#,
        "",
    );
    let limits = ListenLimits {
        header: Some(Duration::from_secs(5)),
        ..ListenLimits::defaults()
    };
    let (address, stop, server) = guarded(guard, limits).await;
    // the one connection this client may hold, open with half a request
    let mut held = TcpStream::connect(address).await.unwrap();
    held.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // closed without an answer: an end of stream, or a reset when the request was still unread
    let mut refused = TcpStream::connect(address).await.unwrap();
    let _ = refused
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await;
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), refused.read_to_end(&mut buf))
        .await
        .expect("the refused connection was not closed");
    assert!(read.is_err() || buf.is_empty(), "{}", String::from_utf8_lossy(&buf));
    // given back when it closes
    drop(held);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let answered = ask(address, "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");

    drop(stop);
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the listener did not stop")
        .unwrap();
}

/// A header sent twice is one value; two `Cookie` headers join as one cookie list, so each pair stays whole.
#[test]
fn repeated_headers_join_and_cookies_join_as_pairs() {
    let mut headers = axum::http::HeaderMap::new();
    headers.append(axum::http::header::COOKIE, "a=1".parse().unwrap());
    headers.append(axum::http::header::COOKIE, "moqui.session=abc".parse().unwrap());
    headers.append("x-forwarded-for", "10.0.0.1".parse().unwrap());
    headers.append("x-forwarded-for", "10.0.0.2".parse().unwrap());
    let map = header_map(&headers);
    assert_eq!(map.get("cookie").map(String::as_str), Some("a=1; moqui.session=abc"));
    assert_eq!(
        map.get("x-forwarded-for").map(String::as_str),
        Some("10.0.0.1, 10.0.0.2")
    );
    assert_eq!(
        moqui_core::web::cookie_value(&map["cookie"], "moqui.session").as_deref(),
        Some("abc")
    );
}

#[test]
fn empty_webapp_http_port_is_8080() {
    assert_eq!(http_port("").unwrap(), 8080);
    assert_eq!(http_port("  ").unwrap(), 8080);
    assert_eq!(http_port("9090").unwrap(), 9090);
    assert!(http_port("nope").is_err());
}

#[test]
fn status_routes_allow_loopback_only() {
    assert!(status_allowed("", "127.0.0.1"));
    assert!(status_allowed("", "::1"));
    assert!(status_allowed("", "0:0:0:0:0:0:0:1"));
    // an IPv4 peer seen through a dual-stack socket is its IPv4 address
    let mapped: std::net::IpAddr = "::ffff:127.0.0.1".parse().unwrap();
    assert!(status_allowed("", &mapped.to_canonical().to_string()));
    assert!(!status_allowed("127.0.0.1", "203.0.113.5"));
    assert!(status_allowed("10.0.0.9, 203.0.113.5", "203.0.113.5"));
}
