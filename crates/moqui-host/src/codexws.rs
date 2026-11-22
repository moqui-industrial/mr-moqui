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

//! `GET /codexws`: the Workbench agent of the Assist drawer, Codex's app-server in the user's own sandbox
//! (moqui-agent-workbench, plan/design-workbench.md). The server here is what the Codex extension's host is in VS
//! Code: it starts the app-server (here, the user's sandbox container), opens its connection, sends `initialize`, and
//! then passes the app-server's JSON-RPC messages both ways unchanged: the page's requests, notifications and answers
//! to the app-server, and the app-server's answers, notifications and requests (approvals, questions) to the page. The
//! page is the client of the protocol, as the extension's webview is.
//!
//! The socket is checked as `/termws` checks it, by origin and session cookie, and needs the `AgentWorkbench`
//! permission. The sandbox is the user's alone: its WebSocket URL and capability token are read from
//! `runtime/workbench/<userId>/{url,token}`, which `workbench_sandbox_command start <userId>` writes (the component's
//! `scripts/sandbox.sh`); the token never reaches the page. The endpoint is served only while `workbench_sandbox_command`
//! is set (a property or the environment).
//!
//! Codex reaches Moqui's services over Moqui's own MCP server (docs/ai/MCP.md), as the user: when
//! `workbench_mcp_url` is set (the address the sandbox reaches this server's HTTPS listener at, such as
//! `https://host.docker.internal:8445/mcp`), each start of a sandbox makes a new key of the user's that opens `/mcp`
//! alone (`UserLoginKey.scope` `mcp`) and gives it and the URL to the command (`MOQUI_MCP_TOKEN`, `MOQUI_MCP_URL`), which
//! gives them to Codex's `moqui` MCP server. A write through MCP waits for the user's approval in Moqui, which Codex
//! asks for as a URL elicitation the page shows.
//!
//! The page's `initialize` is answered here with the app-server's answer to the server's own, and its `initialized` is
//! dropped: the connection is initialized once, by the server. The HTTP session is checked again each minute, so a
//! logout closes the socket; the sandbox goes on (a turn that runs keeps running, and a page that connects again
//! resumes its thread).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use moqui_core::context::{ErrorKind, ExecutionContextFactory};
use moqui_core::user::verify_session_cookie;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as UpMessage;

use crate::http::{plain, HttpState};
use crate::notws::{origin_allowed, session_cookie, session_user, RequestSite};

type CoreError = moqui_core::context::Error;

/// Set to `/codexws` while this server serves the socket; the Assist bootstrap offers the Workbench agent when it is.
pub(crate) const ENDPOINT_PROPERTY: &str = "webapp_codex_endpoint";
/// The command that starts a user's sandbox: `<command> start <userId>` writes its URL and token.
pub(crate) const COMMAND_PROPERTY: &str = "workbench_sandbox_command";
/// The agent the MCP server serves the sandbox's key as: its tool policy is the Workbench's
/// (moqui-agent-workbench's data/10WorkbenchSeedData.xml).
const MCP_AGENT: &str = "workbench";
/// The URL the sandbox reaches the MCP server at; none for a Workbench without Moqui's tools.
pub(crate) const MCP_URL_PROPERTY: &str = "workbench_mcp_url";
/// The permission a user needs for the Workbench.
pub(crate) const PERMISSION: &str = "AgentWorkbench";

/// A ping goes out this often, so a dead peer is noticed and proxies keep the connection.
const PING_EVERY: Duration = Duration::from_secs(30);
/// How often the HTTP session is checked for a logout.
const SESSION_CHECK: Duration = Duration::from_secs(60);
/// How long the sandbox has to start (an image pull is not done here: `sandbox.sh build` comes first).
const START_WAIT: Duration = Duration::from_secs(120);
/// How many times a sandbox just started is tried, half a second apart.
const CONNECT_TRIES: usize = 40;
/// How long the app-server has to answer `initialize`.
const INITIALIZE_WAIT: Duration = Duration::from_secs(30);
/// The largest message either way: a file the page writes into the workspace is base64 in one `fs/writeFile`.
const MAX_MESSAGE: usize = 64 << 20;

/// Whether the endpoint is served: the sandbox command is set.
pub(crate) fn enabled() -> bool {
    !moqui_lib::env::get_prop_or_env(COMMAND_PROPERTY).trim().is_empty()
}

pub(crate) async fn upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    site: RequestSite,
    State(state): State<Arc<HttpState>>,
) -> Response {
    if !origin_allowed(&state.factory, &headers, &site, "workbench") {
        return plain(403, "origin not allowed");
    }
    let Some(cookie) = session_cookie(&headers, site.https()) else {
        return plain(401, "unauthorized");
    };
    let session_id = match verify_session_cookie(&cookie) {
        Ok(Some(id)) if !id.is_empty() => id,
        Ok(_) | Err(_) => return plain(401, "unauthorized"),
    };
    let factory = Arc::clone(&state.factory);
    let lookup = session_id.clone();
    let loaded = tokio::task::spawn_blocking(move || -> Result<Option<(String, bool)>, CoreError> {
        let Some(user_id) = session_user(Arc::clone(&factory), lookup)? else {
            return Ok(None);
        };
        let permitted = permitted(&factory, &user_id)?;
        Ok(Some((user_id, permitted)))
    })
    .await;
    let user_id = match loaded {
        Ok(Ok(Some((user_id, true)))) => user_id,
        Ok(Ok(Some((_, false)))) => return plain(403, "the Workbench needs the AgentWorkbench permission"),
        Ok(Ok(None)) => {
            site.record_auth_failure(&state);
            return plain(401, "unauthorized");
        }
        Ok(Err(err)) if err.kind() == ErrorKind::Unavailable => return state.guard.unavailable("service unavailable"),
        Ok(Err(err)) => return crate::http::internal_error(err.message()),
        Err(_) => return plain(500, "request worker stopped"),
    };
    if !safe_user_id(&user_id) {
        return plain(403, "this user has no Workbench sandbox");
    }
    let shutdown = state.shutdown.clone();
    let factory = Arc::clone(&state.factory);
    ws.max_message_size(MAX_MESSAGE)
        .on_upgrade(move |socket| run_socket(socket, factory, session_id, user_id, shutdown))
}

/// Whether `user_id` holds the Workbench permission, read on the request pool with authz off.
fn permitted(factory: &Arc<ExecutionContextFactory>, user_id: &str) -> Result<bool, CoreError> {
    let owner = Arc::clone(factory);
    let user_id = user_id.to_string();
    let rx = factory.request_pool().try_submit(owner, move || {
        let _authz =
            moqui_core::context::ExecutionContext::with_current(moqui_core::context::ExecutionContext::disable_authz)
                .ok_or_else(|| CoreError::new("no execution context is entered"))?;
        moqui_core::user::user_has_permission(&user_id, PERMISSION)
    })?;
    match rx.recv() {
        Ok(result) => result,
        Err(_) => Err(CoreError::new("request worker stopped")),
    }
}

/// A userId the sandbox's names take (Docker's `[A-Za-z0-9_.-]`, as `sandbox.sh` checks), and no path part.
fn safe_user_id(user_id: &str) -> bool {
    !user_id.is_empty()
        && user_id.len() <= 64
        && !user_id.starts_with('.')
        && user_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// Where a user's sandbox is: its WebSocket URL and capability token.
struct Sandbox {
    url: String,
    token: String,
}

impl Sandbox {
    fn dir(runtime: &Path, user_id: &str) -> PathBuf {
        runtime.join("workbench").join(user_id)
    }

    fn read(dir: &Path) -> Option<Self> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        };
        let url = read("url")?;
        // the app-server listens on this machine; a URL elsewhere was not written by the sandbox command
        if !(url.starts_with("ws://127.0.0.1:") || url.starts_with("ws://[::1]:") || url.starts_with("ws://localhost:"))
        {
            return None;
        }
        Some(Self {
            url,
            token: read("token")?,
        })
    }

    /// Start (or find) the user's sandbox with the command; it answers at once when the container runs. `mcp` is the
    /// MCP server's URL and the user's key for it, for a sandbox the command starts now.
    fn start(runtime: &Path, user_id: &str, mcp: Option<(String, String)>) -> Result<Self, String> {
        let command = moqui_lib::env::get_prop_or_env(COMMAND_PROPERTY);
        let command = command.trim();
        if command.is_empty() {
            return Err("no sandbox command is set".to_string());
        }
        let mut process = std::process::Command::new(command);
        process
            .arg("start")
            .arg(user_id)
            .current_dir(runtime)
            .stdin(std::process::Stdio::null());
        if let Some((url, key)) = mcp {
            process.env("MOQUI_MCP_URL", url).env("MOQUI_MCP_TOKEN", key);
        }
        let output = process
            .output()
            .map_err(|err| format!("the sandbox command did not run: {err}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(user_id, status = ?output.status, stderr = %stderr.trim(), "workbench sandbox did not start");
            return Err("the sandbox did not start".to_string());
        }
        Self::read(&Self::dir(runtime, user_id)).ok_or_else(|| "the sandbox wrote no URL and token".to_string())
    }
}

type Upstream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect to the app-server and initialize the connection as the Codex extension's host does; the app-server's
/// answer to `initialize`, for the page's own.
async fn connect(sandbox: &Sandbox) -> Result<(Upstream, serde_json::Value), String> {
    let mut request = sandbox
        .url
        .as_str()
        .into_client_request()
        .map_err(|err| format!("the sandbox URL does not read: {err}"))?;
    let bearer = format!("Bearer {}", sandbox.token)
        .parse()
        .map_err(|_| "the sandbox token does not read".to_string())?;
    request.headers_mut().insert(axum::http::header::AUTHORIZATION, bearer);
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (mut upstream, _) = tokio_tungstenite::connect_async_with_config(request, Some(config), false)
        .await
        .map_err(|err| format!("the sandbox does not answer: {err}"))?;
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "clientInfo": {"name": "moqui_workbench", "title": "Moqui Workbench", "version": env!("CARGO_PKG_VERSION")},
            // the dynamic tools and the experimental methods, as the extension asks for them
            "capabilities": {"experimentalApi": true},
        },
    });
    upstream
        .send(UpMessage::Text(initialize.to_string().into()))
        .await
        .map_err(|err| format!("the sandbox closed: {err}"))?;
    let answer = tokio::time::timeout(INITIALIZE_WAIT, async {
        while let Some(message) = upstream.next().await {
            let Ok(UpMessage::Text(text)) = message else { continue };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if value.get("id") == Some(&serde_json::json!(0)) && value.get("method").is_none() {
                return Some(value);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| "the sandbox did not answer initialize".to_string())?;
    if let Some(error) = answer.get("error") {
        return Err(format!("the sandbox refused initialize: {error}"));
    }
    upstream
        .send(UpMessage::Text(r#"{"jsonrpc":"2.0","method":"initialized"}"#.into()))
        .await
        .map_err(|err| format!("the sandbox closed: {err}"))?;
    Ok((
        upstream,
        answer.get("result").cloned().unwrap_or(serde_json::Value::Null),
    ))
}

/// A new key of `user_id`'s that opens the MCP server alone, named for the Workbench in the user's device list.
fn mcp_key(factory: &Arc<ExecutionContextFactory>, user_id: &str) -> Result<String, CoreError> {
    let owner = Arc::clone(factory);
    let looked = Arc::clone(factory);
    let user_id = user_id.to_string();
    let rx = factory.request_pool().try_submit(owner, move || {
        let username = {
            let _authz = moqui_core::context::ExecutionContext::with_current(
                moqui_core::context::ExecutionContext::disable_authz,
            )
            .ok_or_else(|| CoreError::new("no execution context is entered"))?;
            let facade = looked
                .entity()
                .ok_or_else(|| CoreError::new("the entity facade is not installed"))?;
            facade
                .find("moqui.security.UserAccount")?
                .eq("userId", moqui_lib::object::Value::string(&user_id))
                .one()?
                .and_then(|row| row.get("username").map(moqui_lib::object::to_plain_string))
                .filter(|username| !username.is_empty())
                .ok_or_else(|| CoreError::new("the user has no username"))?
        };
        moqui_core::user::internal_login_user(&username)?;
        let key = moqui_core::user::get_scoped_key("mcp", "Workbench sandbox");
        let _ = moqui_core::user::logout_local();
        let key = key?;
        // the key's client: the MCP server serves it as the Workbench's agent, with the Workbench's own tool policy
        // apart from the other MCP clients' (crates/moqui-llm/src/mcp/server.rs, authenticate)
        let _authz = moqui_core::context::ExecutionContext::with_current(
            moqui_core::context::ExecutionContext::disable_authz,
        )
        .ok_or_else(|| CoreError::new("no execution context is entered"))?;
        let facade = looked
            .entity()
            .ok_or_else(|| CoreError::new("the entity facade is not installed"))?;
        let mut client = facade.make_value("moqui.llm.LlmApiClient")?;
        client.set("clientId", moqui_lib::object::Value::string(format!("WB_{user_id}")));
        client.set("description", moqui_lib::object::Value::string("Workbench sandbox (Codex), its MCP key"));
        client.set("userId", moqui_lib::object::Value::string(&user_id));
        client.set(
            "loginKey",
            moqui_lib::object::Value::string(moqui_core::user::hash_login_key(&key)?),
        );
        client.set("scopes", moqui_lib::object::Value::string("mcp"));
        client.set("allowedAgents", moqui_lib::object::Value::string(MCP_AGENT));
        client.set("disabled", moqui_lib::object::Value::string("N"));
        client.create_or_update(&facade)?;
        Ok(key)
    })?;
    match rx.recv() {
        Ok(result) => result,
        Err(_) => Err(CoreError::new("request worker stopped")),
    }
}

/// Open the user's sandbox: the running one, else started by the command.
async fn open(factory: &Arc<ExecutionContextFactory>, user_id: &str) -> Result<(Upstream, serde_json::Value), String> {
    let runtime = factory.runtime_dir().to_path_buf();
    if let Some(sandbox) = Sandbox::read(&Sandbox::dir(&runtime, user_id)) {
        if let Ok(opened) = connect(&sandbox).await {
            return Ok(opened);
        }
    }
    let user = user_id.to_string();
    let factory = Arc::clone(factory);
    let started = tokio::time::timeout(
        START_WAIT,
        tokio::task::spawn_blocking(move || {
            // Moqui's tools for Codex: a key that opens /mcp alone, made only when a sandbox starts
            let mcp_url = moqui_lib::env::get_prop_or_env(MCP_URL_PROPERTY).trim().to_string();
            let mcp = if mcp_url.is_empty() {
                None
            } else {
                match mcp_key(&factory, &user) {
                    Ok(key) => Some((mcp_url, key)),
                    Err(err) => {
                        tracing::warn!(user_id = %user, "workbench sandbox starts without Moqui's tools: {}", err.message());
                        None
                    }
                }
            };
            Sandbox::start(&runtime, &user, mcp)
        }),
    )
        .await
        .map_err(|_| "the sandbox did not start in time".to_string())?
        .map_err(|_| "the sandbox start stopped".to_string())??;
    // a container just started listens a moment later
    let mut last = String::new();
    for _ in 0..CONNECT_TRIES {
        match connect(&started).await {
            Ok(opened) => return Ok(opened),
            Err(why) => last = why,
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(last)
}

/// What the page sends: the JSON-RPC message to pass on, or the answer the server gives itself.
enum FromPage {
    Pass(String),
    Answer(String),
    Drop,
}

/// The page's `initialize` gets the server's answer; its `initialized` is dropped (the connection was initialized
/// once, by the server); anything that is not a JSON object is dropped; the rest passes unchanged.
fn from_page(text: &str, initialized: &serde_json::Value) -> FromPage {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return FromPage::Drop;
    };
    if !value.is_object() {
        return FromPage::Drop;
    }
    match value.get("method").and_then(serde_json::Value::as_str) {
        Some("initialize") => match value.get("id") {
            Some(id) => {
                FromPage::Answer(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": initialized}).to_string())
            }
            None => FromPage::Drop,
        },
        Some("initialized") => FromPage::Drop,
        _ => FromPage::Pass(text.to_string()),
    }
}

async fn close(socket: &mut WebSocket, reason: &str) {
    let mut end = reason.len().min(123);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    let _ = crate::notws::send_within(
        socket,
        Message::Close(Some(CloseFrame {
            code: 1000,
            reason: reason[..end].into(),
        })),
    )
    .await;
}

async fn run_socket(
    mut socket: WebSocket,
    factory: Arc<ExecutionContextFactory>,
    session_id: String,
    user_id: String,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let (mut upstream, initialized) = match open(&factory, &user_id).await {
        Ok(opened) => opened,
        Err(why) => {
            tracing::info!(user_id, "workbench socket closed: {why}");
            close(&mut socket, &why).await;
            return;
        }
    };
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut check = tokio::time::interval(SESSION_CHECK);
    check.tick().await;
    let mut used = false;
    let mut reason = String::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    reason = "the server is stopping".to_string();
                    break;
                }
            }
            incoming = upstream.next() => match incoming {
                Some(Ok(UpMessage::Text(text))) => {
                    if !crate::notws::send_within(&mut socket, Message::Text(text.as_str().into())).await {
                        break;
                    }
                }
                Some(Ok(UpMessage::Ping(payload))) => {
                    let _ = upstream.send(UpMessage::Pong(payload)).await;
                }
                Some(Ok(UpMessage::Close(_)) | Err(_)) | None => {
                    reason = "the sandbox closed the connection".to_string();
                    break;
                }
                Some(Ok(_)) => {}
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    used = true;
                    match from_page(&text, &initialized) {
                        FromPage::Pass(text) => {
                            if upstream.send(UpMessage::Text(text.into())).await.is_err() {
                                reason = "the sandbox closed the connection".to_string();
                                break;
                            }
                        }
                        FromPage::Answer(text) => {
                            if !crate::notws::send_within(&mut socket, Message::Text(text.into())).await {
                                break;
                            }
                        }
                        FromPage::Drop => {}
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = crate::notws::send_within(&mut socket, Message::Pong(payload)).await;
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if !crate::notws::send_within(&mut socket, Message::Ping(Vec::new().into())).await {
                    break;
                }
                let _ = upstream.send(UpMessage::Ping(Vec::new().into())).await;
            }
            _ = check.tick() => {
                let lookup = (Arc::clone(&factory), session_id.clone());
                let current = tokio::task::spawn_blocking(move || session_user(lookup.0, lookup.1)).await;
                // a pool that is busy or a read that fails is not a logout; only another user, or none, is
                if let Ok(Ok(current)) = current {
                    if current.as_deref() != Some(user_id.as_str()) {
                        reason = "logged out".to_string();
                        break;
                    }
                }
                // working in the Workbench is use of the session
                if std::mem::take(&mut used) {
                    moqui_core::user::session::note_use(&factory, &session_id);
                }
            }
        }
    }
    let _ = upstream.close(None).await;
    if !reason.is_empty() {
        close(&mut socket, &reason).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_initialize_is_answered_here_and_the_rest_passes() {
        let initialized = serde_json::json!({"userAgent": "codex"});
        match from_page(
            r#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}"#,
            &initialized,
        ) {
            FromPage::Answer(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["id"], 7);
                assert_eq!(value["result"]["userAgent"], "codex");
            }
            _ => panic!("initialize is answered here"),
        }
        assert!(matches!(
            from_page(r#"{"jsonrpc":"2.0","method":"initialized"}"#, &initialized),
            FromPage::Drop
        ));
        assert!(matches!(from_page("[1,2]", &initialized), FromPage::Drop));
        assert!(matches!(from_page("not json", &initialized), FromPage::Drop));
        let turn = r#"{"jsonrpc":"2.0","id":8,"method":"turn/start","params":{"threadId":"t"}}"#;
        assert!(matches!(from_page(turn, &initialized), FromPage::Pass(text) if text == turn));
        let answer = r#"{"jsonrpc":"2.0","id":3,"result":{"decision":"accept"}}"#;
        assert!(matches!(from_page(answer, &initialized), FromPage::Pass(_)));
    }

    #[test]
    fn a_sandbox_is_named_by_a_plain_user_id_and_listens_here() {
        assert!(safe_user_id("EX_JOHN_DOE"));
        assert!(safe_user_id("100050"));
        assert!(!safe_user_id("../etc"));
        assert!(!safe_user_id(".hidden"));
        assert!(!safe_user_id("a/b"));
        assert!(!safe_user_id(""));
        let dir = std::env::temp_dir().join(format!("codexws-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("token"), "abc\n").unwrap();
        std::fs::write(dir.join("url"), "ws://10.0.0.5:4500\n").unwrap();
        assert!(Sandbox::read(&dir).is_none(), "a URL off this machine is refused");
        std::fs::write(dir.join("url"), "ws://127.0.0.1:32769\n").unwrap();
        let sandbox = Sandbox::read(&dir).unwrap();
        assert_eq!(sandbox.url, "ws://127.0.0.1:32769");
        assert_eq!(sandbox.token, "abc");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
