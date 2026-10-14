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

//! The DevTools Protocol over one browser WebSocket: commands with an id and a reply, and events routed by the flat
//! session they belong to. Messages stay `serde_json::Value`, so the workspace's `arbitrary_precision` numbers parse.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use moqui_lib::error::Error;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

/// Largest message the browser may send: a PDF comes back as one base64 string.
const MAX_MESSAGE: usize = 512 * 1024 * 1024;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Error>>>>>;

/// Events of one session, and the events that arrived before anyone listened.
#[derive(Default)]
struct Routes {
    listeners: HashMap<String, mpsc::UnboundedSender<Value>>,
    early: HashMap<String, Vec<Value>>,
}

/// A connected browser. Cloning shares the socket.
#[derive(Clone)]
pub(super) struct Connection {
    outgoing: mpsc::UnboundedSender<String>,
    pending: Pending,
    routes: Arc<Mutex<Routes>>,
    next_id: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}

impl Connection {
    /// Connect to the browser socket. Runs the reader and the writer as tasks on the current runtime.
    pub(super) async fn connect(url: &str) -> Result<Self, Error> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let (socket, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), true)
            .await
            .map_err(|err| Error::new(format!("could not connect to the browser at {url}: {err}")))?;
        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut queue) = mpsc::unbounded_channel::<String>();
        let connection = Self {
            outgoing,
            pending: Arc::default(),
            routes: Arc::default(),
            next_id: Arc::new(AtomicU64::new(1)),
            closed: Arc::new(AtomicBool::new(false)),
        };
        tokio::spawn(async move {
            while let Some(text) = queue.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        let reader = connection.clone();
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let text = match message {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Binary(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => continue,
                };
                match serde_json::from_str::<Value>(&text) {
                    Ok(value) => reader.dispatch(value),
                    Err(err) => tracing::debug!("unreadable DevTools message: {err}"),
                }
            }
            reader.close();
        });
        Ok(connection)
    }

    /// True once the socket closed: the browser exited or dropped the connection.
    pub(super) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Send `method` to the browser, or to a page when `session` is set, and wait for its result.
    pub(super) async fn call(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        if self.is_closed() {
            return Err(Error::new("the browser connection is closed"));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_string());
        }
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        if self.outgoing.send(message.to_string()).is_err() {
            self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
            return Err(Error::new("the browser connection is closed"));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::new(format!("the browser closed before answering {method}"))),
            Err(_) => {
                self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
                Err(Error::new(format!(
                    "the browser did not answer {method} in {} seconds",
                    timeout.as_secs()
                )))
            }
        }
    }

    /// The events of `session`, including any that arrived before this call.
    pub(super) fn events(&self, session: &str) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut routes = self.routes.lock().unwrap_or_else(PoisonError::into_inner);
        for event in routes.early.remove(session).unwrap_or_default() {
            let _ = tx.send(event);
        }
        routes.listeners.insert(session.to_string(), tx);
        rx
    }

    /// Stop routing the events of `session`.
    pub(super) fn forget(&self, session: &str) {
        let mut routes = self.routes.lock().unwrap_or_else(PoisonError::into_inner);
        routes.listeners.remove(session);
        routes.early.remove(session);
    }

    fn dispatch(&self, message: Value) {
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            let waiter = self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
            if let Some(waiter) = waiter {
                let result = match message.get("error") {
                    Some(error) => Err(Error::new(format!(
                        "DevTools error: {}",
                        error.get("message").and_then(Value::as_str).unwrap_or("unknown")
                    ))),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(result);
            }
            return;
        }
        let Some(session) = message.get("sessionId").and_then(Value::as_str) else {
            return;
        };
        let mut routes = self.routes.lock().unwrap_or_else(PoisonError::into_inner);
        match routes.listeners.get(session) {
            Some(listener) => {
                let _ = listener.send(message);
            }
            None => {
                // Events before the page's listener exists; a session that never listens keeps only a few.
                let early = routes.early.entry(session.to_string()).or_default();
                if early.len() < 256 {
                    early.push(message);
                }
            }
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let waiters: Vec<_> = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .map(|(_, waiter)| waiter)
            .collect();
        for waiter in waiters {
            let _ = waiter.send(Err(Error::new("the browser connection closed")));
        }
        self.routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .listeners
            .clear();
    }
}
