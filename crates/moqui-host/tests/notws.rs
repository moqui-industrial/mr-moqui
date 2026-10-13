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

//! Notification socket and one cluster cache post. `cluster_https_port=0` is an ephemeral bind.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moqui_core::context::ExecutionContextFactory;
use moqui_core::notify::NotificationMessage;
use moqui_host::{load_data, start_server, LoadFlags};
use moqui_lib::object::{basic_convert, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, ClientRequestBuilder, Message, WebSocket};

const SHARED_TOPIC: &str = "Wave3Shared";
const ONLY_TOPIC: &str = "Wave3Only";

#[test]
fn notws_push_and_one_cluster_invalidation() {
    let runtime = temp_dir("moqui-notws-rt");
    let cluster_dir = temp_dir("moqui-notws-pg");
    let _runtime_guard = DirGuard(runtime.clone());
    let _cluster_guard = DirGuard(cluster_dir.clone());
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&cluster_dir).unwrap();
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
    let base =
        std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component")).unwrap();
    std::os::unix::fs::symlink(&base, runtime.join("base-component")).unwrap();

    load_data(&runtime, &framework, &cluster_dir, &LoadFlags::default()).unwrap();
    let demo = base.join("webroot/data/WebrootSecurityDemoData.xml");
    let demo_flags = LoadFlags {
        location: Some(demo.display().to_string()),
        data_types: Some(vec!["demo".to_string()]),
        ..LoadFlags::default()
    };
    load_data(&runtime, &framework, &cluster_dir, &demo_flags).unwrap();

    // 0 binds an ephemeral cluster port. Empty would listen on 8443. This does not touch the repo .env.
    moqui_lib::env::set_property("cluster_https_port", "0");
    let server = start_server(
        &runtime,
        &framework,
        &cluster_dir,
        SocketAddr::from(([127, 0, 0, 1], 0)),
    )
    .unwrap();
    let http = server.address();
    assert_ne!(
        server.cluster_address().port(),
        0,
        "ephemeral cluster port was not recorded"
    );

    // Two logins, two sessions. A second socket on the first session stays open and gets nothing.
    let cookie_all = login(http);
    let cookie_other = login(http);
    assert_ne!(cookie_all, cookie_other, "each login mints its own session");
    server.factory().flush_sessions().unwrap();

    assert_upgrade_rejected(http, None);
    assert_upgrade_rejected(http, Some("not-a-session"));

    let mut all = open_socket(http, &cookie_all);
    let mut other = open_socket(http, &cookie_other);
    all.send(Message::Text("subscribe:ALL".into())).unwrap();
    other
        .send(Message::Text(format!("subscribe:{SHARED_TOPIC}").into()))
        .unwrap();

    let factory = Arc::clone(server.factory());
    let user_id = user_id_of(&factory);
    prove_both_subscribed(&factory, &user_id, &mut all, &mut other);
    drain(&mut all);
    drain(&mut other);
    let mut extra = open_socket(http, &cookie_all);
    extra.send(Message::Text("subscribe:ALL".into())).unwrap();
    send_note(&factory, &user_id, ONLY_TOPIC);
    let frame = read_until(&mut all, ONLY_TOPIC, Duration::from_secs(3));
    assert!(frame.contains(&user_id), "{frame}");
    assert_absent(&mut other, ONLY_TOPIC, Duration::from_millis(400));
    assert_absent(&mut extra, ONLY_TOPIC, Duration::from_millis(400));
    drop(all);
    drop(other);
    drop(extra);

    let (session_fp, cert_fp) = member_fingerprints(&factory);
    let port = server.cluster_address().port();
    insert_peer(&factory, &session_fp, &cert_fp, port);
    let before = server.cluster_posts().len();
    create_test_entity(&factory);
    let posts = server.cluster_posts();
    let added = &posts[before..];
    assert_eq!(added, ["entity-cache-invalidate"], "posts {posts:?} before {before}");
}

fn prove_both_subscribed(factory: &Arc<ExecutionContextFactory>, user_id: &str, all: &mut Ws, other: &mut Ws) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut saw_all = false;
    let mut saw_other = false;
    while Instant::now() < deadline && !(saw_all && saw_other) {
        send_note(factory, user_id, SHARED_TOPIC);
        if !saw_all {
            saw_all = read_for(all, SHARED_TOPIC, Duration::from_millis(250));
        }
        if !saw_other {
            saw_other = read_for(other, SHARED_TOPIC, Duration::from_millis(250));
        }
    }
    assert!(saw_all && saw_other, "all={saw_all} other={saw_other}");
}

fn read_until(socket: &mut Ws, needle: &str, total: Duration) -> String {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if let Some(text) = next_text(socket, Duration::from_millis(200)) {
            if text.contains(needle) {
                return text;
            }
        }
    }
    panic!("missing {needle}");
}

fn read_for(socket: &mut Ws, needle: &str, timeout: Duration) -> bool {
    next_text(socket, timeout).is_some_and(|text| text.contains(needle))
}

fn assert_absent(socket: &mut Ws, needle: &str, total: Duration) {
    let deadline = Instant::now() + total;
    while Instant::now() < deadline {
        if let Some(text) = next_text(socket, Duration::from_millis(150)) {
            assert!(!text.contains(needle), "{text}");
        }
    }
}

fn drain(socket: &mut Ws) {
    while next_text(socket, Duration::from_millis(100)).is_some() {}
}

fn send_note(factory: &Arc<ExecutionContextFactory>, user_id: &str, topic: &str) {
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let mut note = NotificationMessage::new();
    note.topic(topic)
        .user_id(user_id)
        .persist(false)
        .message_json("{\"n\":1}")
        .send()
        .unwrap();
    drop(entered);
}

fn login(address: SocketAddr) -> String {
    let form = "username=john.doe&password=moqui";
    let (status, headers, body) = exchange(
        address,
        &http_request(
            "POST",
            "/rest/login",
            &[("Content-Type", "application/x-www-form-urlencoded")],
            form,
        ),
    );
    assert_eq!(status, 200, "{body}");
    session_cookie(&headers)
}

fn user_id_of(factory: &Arc<ExecutionContextFactory>) -> String {
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let facade = factory.entity().unwrap();
    let row = facade
        .find("moqui.security.UserAccount")
        .unwrap()
        .eq("username", Value::string("john.doe"))
        .one()
        .unwrap()
        .unwrap();
    let user_id = row.get("userId").and_then(Value::as_str).unwrap().to_string();
    drop(entered);
    user_id
}

fn member_fingerprints(factory: &Arc<ExecutionContextFactory>) -> (String, String) {
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let facade = factory.entity().unwrap();
    let members = facade.find("moqui.server.ClusterMember").unwrap().list().unwrap();
    let row = members.values().first().expect("cluster member");
    let session_fp = field_text(row, "sessionKeyFingerprint");
    let cert_fp = field_text(row, "certFingerprint");
    drop(entered);
    assert!(!session_fp.is_empty());
    assert!(!cert_fp.is_empty());
    (session_fp, cert_fp)
}

fn insert_peer(factory: &Arc<ExecutionContextFactory>, session_fp: &str, cert_fp: &str, port: u16) {
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let facade = factory.entity().unwrap();
    let millis = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(0);
    let heartbeat = basic_convert(&Value::I64(millis), "java.sql.Timestamp").value;
    let mut row = facade.make_value("moqui.server.ClusterMember").unwrap();
    row.set("instanceId", Value::string("wave3-peer"));
    row.set("clusterBaseUrl", Value::string(format!("https://127.0.0.1:{port}")));
    row.set("heartbeat", heartbeat);
    row.set("sessionKeyFingerprint", Value::string(session_fp));
    row.set("certFingerprint", Value::string(cert_fp));
    facade.begin("moqui.server.ClusterMember", None).unwrap();
    row.create(&facade).unwrap();
    facade.commit("moqui.server.ClusterMember").unwrap();
    drop(entered);
}

fn create_test_entity(factory: &Arc<ExecutionContextFactory>) {
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let facade = factory.entity().unwrap();
    let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
    row.set("testId", Value::string("WAVE3"));
    row.set("testMedium", Value::string("cluster"));
    facade.begin("moqui.test.TestEntity", None).unwrap();
    row.create(&facade).unwrap();
    facade.commit("moqui.test.TestEntity").unwrap();
    drop(entered);
}

fn field_text(row: &moqui_core::entity::EntityValue, name: &str) -> String {
    row.get(name).and_then(Value::as_str).unwrap_or("").to_string()
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn open_socket(address: SocketAddr, cookie: &str) -> Ws {
    let uri: tungstenite::http::Uri = format!("ws://{address}/notws").parse().unwrap();
    let request = ClientRequestBuilder::new(uri).with_header("Cookie", format!("moqui.session={cookie}"));
    let (socket, response) = connect(request).unwrap();
    assert_eq!(response.status().as_u16(), 101);
    socket
}

fn assert_upgrade_rejected(address: SocketAddr, cookie: Option<&str>) {
    let uri: tungstenite::http::Uri = format!("ws://{address}/notws").parse().unwrap();
    let mut request = ClientRequestBuilder::new(uri);
    if let Some(cookie) = cookie {
        request = request.with_header("Cookie", format!("moqui.session={cookie}"));
    }
    match connect(request) {
        Err(tungstenite::Error::Http(response)) => {
            let status = response.status().as_u16();
            assert_ne!(status, 101);
            assert_eq!(status, 401, "{status}");
        }
        Ok(_) => panic!("websocket upgraded without a valid session"),
        Err(err) => panic!("expected HTTP 401, got {err}"),
    }
}

fn next_text(socket: &mut Ws, timeout: Duration) -> Option<String> {
    set_read_timeout(socket, timeout);
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => return Some(text.to_string()),
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Close(_)) => return None,
            Err(tungstenite::Error::Io(err)) if timed_out(&err) => return None,
            Err(tungstenite::Error::ConnectionClosed) => return None,
            Err(err) => panic!("websocket read: {err}"),
            Ok(other) => panic!("unexpected websocket frame {other:?}"),
        }
    }
}

fn set_read_timeout(socket: &mut Ws, timeout: Duration) {
    match socket.get_mut() {
        MaybeTlsStream::Plain(tcp) => tcp.set_read_timeout(Some(timeout)).unwrap(),
        _ => panic!("notification socket is not a plain tcp stream"),
    }
}

fn timed_out(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::TimedOut || err.kind() == std::io::ErrorKind::WouldBlock
}

fn http_request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut out = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in headers {
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    if !body.is_empty() || method.eq_ignore_ascii_case("POST") {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

fn exchange(address: SocketAddr, request: &str) -> (u16, Vec<(String, String)>, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(30))).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(count) => buf.extend_from_slice(&tmp[..count]),
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut || err.kind() == std::io::ErrorKind::WouldBlock => {
                if response_complete(&buf) {
                    break;
                }
                panic!("timed out after {} bytes: {}", buf.len(), String::from_utf8_lossy(&buf));
            }
            Err(err) => panic!("read failed after {} bytes: {err}", buf.len()),
        }
        if response_complete(&buf) {
            break;
        }
    }
    parse_response(&buf)
}

fn response_complete(buf: &[u8]) -> bool {
    let Some(split) = find_header_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..split]);
    let Some(length) = content_length(&head) else {
        return false;
    };
    buf.len() >= split + 4 + length
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &str) -> Option<usize> {
    for line in head.split("\r\n").skip(1) {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

fn parse_response(buf: &[u8]) -> (u16, Vec<(String, String)>, String) {
    let text = String::from_utf8_lossy(buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_ref(), ""));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    (status, headers, body.to_string())
}

fn session_cookie(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .find_map(|(name, value)| {
            if !name.eq_ignore_ascii_case("set-cookie") {
                return None;
            }
            let pair = value.split(';').next().unwrap_or("").trim();
            let (key, cookie) = pair.split_once('=')?;
            if key.trim() == "moqui.session" {
                Some(cookie.trim().to_string())
            } else {
                None
            }
        })
        .expect("Set-Cookie moqui.session")
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!("{prefix}-{}-{nanos}", std::process::id());
    if Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp").join(name)
    } else {
        std::env::temp_dir().join(name)
    }
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
