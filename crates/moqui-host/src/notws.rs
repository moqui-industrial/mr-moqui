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

//! `GET /notws`. The socket is checked with the session cookie, then pushed notification JSON.
//! No execution context is kept for the life of the socket. A second socket for the same session
//! stays open and receives nothing.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use moqui_core::context::{ErrorKind, ExecutionContext, ExecutionContextFactory};
use moqui_core::notify::{TopicMessage, NOTIFICATION_MESSAGE};
use moqui_core::user::{verify_session_cookie, SESSION_COOKIE_NAME};
use moqui_core::web::cookie_value;
use moqui_lib::object::Value;

use crate::http::{plain, HttpState};

type CoreError = moqui_core::context::Error;

static PUSH: OnceLock<moqui_core::notify::Subscription> = OnceLock::new();
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

fn sockets() -> &'static Mutex<HashMap<u64, Arc<Slot>>> {
    static SOCKETS: OnceLock<Mutex<HashMap<u64, Arc<Slot>>>> = OnceLock::new();
    SOCKETS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct Slot {
    id: u64,
    user_id: String,
    session_id: String,
    topics: Mutex<HashSet<String>>,
    tx: tokio::sync::mpsc::UnboundedSender<Push>,
}

struct Push {
    text: String,
    message_id: String,
}

enum TopicCommand {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
}

/// Subscribe once to `notification-message`. A second boot keeps the first listener.
pub(crate) fn install_push() -> Result<(), CoreError> {
    if PUSH.get().is_some() {
        return Ok(());
    }
    let subscription = moqui_core::notify::subscribe(NOTIFICATION_MESSAGE, Arc::new(deliver_notice))?;
    let _ = PUSH.set(subscription);
    Ok(())
}

pub(crate) async fn upgrade(ws: WebSocketUpgrade, headers: HeaderMap, State(state): State<Arc<HttpState>>) -> Response {
    let Some(cookie) = session_cookie(&headers) else {
        return plain(401, "unauthorized");
    };
    let session_id = match verify_session_cookie(&cookie) {
        Ok(Some(id)) if !id.is_empty() => id,
        Ok(_) | Err(_) => return plain(401, "unauthorized"),
    };
    let lookup_id = session_id.clone();
    let factory = Arc::clone(&state.factory);
    let loaded = tokio::task::spawn_blocking(move || session_user(factory, lookup_id)).await;
    let user_id = match loaded {
        Ok(Ok(Some(user_id))) => user_id,
        Ok(Ok(None)) => return plain(401, "unauthorized"),
        Ok(Err(err)) if err.kind() == ErrorKind::Unavailable => return plain(503, "service unavailable"),
        Ok(Err(err)) => return plain(500, err.message()),
        Err(_) => return plain(500, "request worker stopped"),
    };
    let shutdown = state.shutdown.clone();
    let factory = Arc::clone(&state.factory);
    ws.on_upgrade(move |socket| run_socket(socket, factory, session_id, user_id, shutdown))
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| cookie_value(header, SESSION_COOKIE_NAME))
        .filter(|value| !value.is_empty())
}

fn session_user(factory: Arc<ExecutionContextFactory>, session_id: String) -> Result<Option<String>, CoreError> {
    let owner = Arc::clone(&factory);
    let looked = Arc::clone(&factory);
    let rx = factory
        .request_pool()
        .try_submit(owner, move || user_of(&looked, &session_id))?;
    match rx.recv() {
        Ok(result) => result,
        Err(_) => Err(CoreError::new("request worker stopped")),
    }
}

fn user_of(factory: &ExecutionContextFactory, session_id: &str) -> Result<Option<String>, CoreError> {
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz)
        .ok_or_else(|| CoreError::new("no execution context is entered"))?;
    let facade = factory
        .entity()
        .ok_or_else(|| CoreError::new("the entity facade is not installed"))?;
    let row = facade
        .find("moqui.server.HttpSession")?
        .eq("sessionId", Value::string(session_id))
        .one()?;
    Ok(row.and_then(|row| {
        row.get("userId")
            .and_then(Value::as_str)
            .filter(|user_id| !user_id.is_empty())
            .map(str::to_string)
    }))
}

async fn run_socket(
    mut socket: WebSocket,
    factory: Arc<ExecutionContextFactory>,
    session_id: String,
    user_id: String,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
    let slot = Arc::new(Slot {
        id,
        user_id,
        session_id,
        topics: Mutex::new(HashSet::new()),
        tx,
    });
    if !try_register(&slot) {
        tracing::warn!(
            session_id = %slot.session_id,
            user_id = %slot.user_id,
            "notification socket already registered for this session; not registering another"
        );
        idle_unregistered(&mut socket, &mut shutdown).await;
        return;
    }
    tracing::debug!(session_id = %slot.session_id, user_id = %slot.user_id, "notification socket open");
    if *shutdown.borrow() {
        unregister(id);
        return;
    }
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = socket.recv() => {
                if !handle_incoming(&mut socket, &factory, &slot, incoming).await {
                    break;
                }
            }
            push = rx.recv() => {
                let Some(push) = push else {
                    break;
                };
                if socket.send(Message::Text(push.text.into())).await.is_err() {
                    break;
                }
                mark_after_send(&factory, &slot.user_id, &push.message_id).await;
            }
        }
    }
    tracing::debug!(session_id = %slot.session_id, user_id = %slot.user_id, "notification socket closed");
    unregister(id);
}

async fn handle_incoming(
    socket: &mut WebSocket,
    factory: &Arc<ExecutionContextFactory>,
    slot: &Arc<Slot>,
    incoming: Option<Result<Message, axum::Error>>,
) -> bool {
    match incoming {
        Some(Ok(Message::Text(text))) => {
            apply_frame(factory, slot, &text).await;
            true
        }
        Some(Ok(Message::Ping(payload))) => socket.send(Message::Pong(payload)).await.is_ok(),
        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => false,
        Some(Ok(_)) => true,
    }
}

async fn apply_frame(factory: &Arc<ExecutionContextFactory>, slot: &Arc<Slot>, text: &str) {
    let Some(command) = parse_command(text) else {
        return;
    };
    let slot = Arc::clone(slot);
    let factory = Arc::clone(factory);
    let ran = tokio::task::spawn_blocking(move || {
        factory.service_pools().call(move || {
            apply_command(&slot, command);
            Ok(())
        })
    })
    .await;
    if let Err(err) = flatten_call(ran) {
        tracing::warn!(error = %err, "notification topic update failed");
    }
}

fn parse_command(text: &str) -> Option<TopicCommand> {
    let text = text.trim();
    let (subscribe, names) = if let Some(rest) = text.strip_prefix("subscribe:") {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix("unsubscribe:") {
        (false, rest)
    } else {
        return None;
    };
    let names = names
        .split(',')
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    if subscribe {
        Some(TopicCommand::Subscribe(names))
    } else {
        Some(TopicCommand::Unsubscribe(names))
    }
}

fn apply_command(slot: &Slot, command: TopicCommand) {
    let mut topics = lock(&slot.topics);
    match command {
        TopicCommand::Subscribe(names) => {
            for name in names {
                topics.insert(name);
            }
        }
        TopicCommand::Unsubscribe(names) => {
            for name in names {
                topics.remove(&name);
            }
        }
    }
}

async fn mark_after_send(factory: &Arc<ExecutionContextFactory>, user_id: &str, message_id: &str) {
    if message_id.is_empty() {
        return;
    }
    let factory = Arc::clone(factory);
    let user_id = user_id.to_string();
    let message_id = message_id.to_string();
    let ran = tokio::task::spawn_blocking(move || {
        factory
            .service_pools()
            .call(move || moqui_core::notify::mark_sent(&message_id, &user_id))
    })
    .await;
    if let Err(err) = flatten_call(ran) {
        tracing::warn!(error = %err, "could not mark notification sent");
    }
}

fn flatten_call(ran: Result<Result<(), CoreError>, tokio::task::JoinError>) -> Result<(), CoreError> {
    match ran {
        Ok(result) => result,
        Err(_) => Err(CoreError::new("service worker stopped")),
    }
}

fn deliver_notice(message: &TopicMessage) {
    if catch_unwind(AssertUnwindSafe(|| fan_out_notice(message))).is_err() {
        tracing::warn!("notification websocket fan-out panicked");
    }
}

fn fan_out_notice(message: &TopicMessage) {
    let text = message.body.to_string();
    let topic = json_str(&message.body, "topic");
    let message_id = json_str(&message.body, "notificationMessageId");
    let users = json_users(&message.body);
    let slots: Vec<Arc<Slot>> = lock(sockets()).values().cloned().collect();
    for slot in slots {
        if !users.iter().any(|user_id| user_id == &slot.user_id) {
            continue;
        }
        let wanted = {
            let topics = lock(&slot.topics);
            topics.contains("ALL") || (!topic.is_empty() && topics.contains(&topic))
        };
        if !wanted {
            continue;
        }
        let push = Push {
            text: text.clone(),
            message_id: message_id.clone(),
        };
        if slot.tx.send(push).is_err() {
            unregister(slot.id);
        }
    }
}

fn json_str(body: &serde_json::Value, field: &str) -> String {
    body.get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn json_users(body: &serde_json::Value) -> Vec<String> {
    body.get("userIds")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// One socket per session. The first connection stays registered.
fn try_register(slot: &Arc<Slot>) -> bool {
    let mut open = lock(sockets());
    if open.values().any(|existing| existing.session_id == slot.session_id) {
        return false;
    }
    open.insert(slot.id, Arc::clone(slot));
    true
}

async fn idle_unregistered(socket: &mut WebSocket, shutdown: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

fn unregister(id: u64) {
    lock(sockets()).remove(&id);
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}
