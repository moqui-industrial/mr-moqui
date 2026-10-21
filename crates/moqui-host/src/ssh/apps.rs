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

//! `apps` over SSH (plans/Screen-TUI-Impl.md 4.2): the terminal client on the channel, in this process. The client's
//! session loop (`moqui_tui::run_session`) runs on a thread of its own, since it and the in-process link block; a task
//! here carries the channel's bytes and window changes to it and its frames back. Every request runs as the user
//! through the request pool, so `SSH_APPS` gives nothing beyond the screens the user may already see. At most
//! `tui_max_sessions` run at once, `tui_max_sessions_per_user` of them for one user. Files are not downloaded.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::{mpsc, Arc, Mutex, PoisonError, Weak};

use moqui_core::context::ExecutionContextFactory;
use moqui_tui::{SessionEvent, SessionOptions, Theme};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc as tokio_mpsc, oneshot, watch};

use crate::tui::HostLink;

const DEFAULT_MAX: usize = 16;
const DEFAULT_MAX_PER_USER: usize = 4;
/// Where a session starts: the apps' root, whose menu lists the apps the user may open.
const START_PATH: &str = "/apps";

/// What the client asked for its terminal: the pty request's `TERM` and size, and the variables of its `env` requests
/// that say how it draws (`COLORTERM`, `NO_COLOR`, `COLORFGBG`, the locale).
#[derive(Clone, Debug, Default)]
pub(super) struct Terminal {
    pub term: String,
    pub env: BTreeMap<String, String>,
}

/// The variables an `env` request may set for `apps`.
pub(super) fn is_terminal_variable(name: &str) -> bool {
    matches!(
        name,
        "COLORTERM" | "NO_COLOR" | "COLORFGBG" | "LANG" | "LC_ALL" | "LC_CTYPE"
    )
}

impl Terminal {
    /// The theme for this terminal. A locale that is set and is not UTF-8 draws in ASCII.
    fn theme(&self) -> Theme {
        let env = |name: &str| match name {
            "TERM" => Some(self.term.clone()),
            other => self.env.get(other).cloned(),
        };
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .find_map(|name| self.env.get(name).filter(|value| !value.is_empty()));
        let ascii = locale.is_some_and(|locale| {
            let locale = locale.to_ascii_lowercase();
            !(locale.contains("utf-8") || locale.contains("utf8"))
        });
        Theme::detect(&env, ascii, false)
    }
}

/// The live `apps` sessions, against `tui_max_sessions` and `tui_max_sessions_per_user`.
pub(super) struct Sessions {
    max: usize,
    max_per_user: usize,
    live: Mutex<HashMap<String, usize>>,
}

impl Sessions {
    pub fn from_properties() -> Self {
        Self::new(
            super::server::number_property("tui_max_sessions", DEFAULT_MAX),
            super::server::number_property("tui_max_sessions_per_user", DEFAULT_MAX_PER_USER),
        )
    }

    pub fn new(max: usize, max_per_user: usize) -> Self {
        Self {
            max,
            max_per_user,
            live: Mutex::new(HashMap::new()),
        }
    }

    /// A place for one more session of `username`, or why there is none. Drop gives it back.
    pub(super) fn take(self: &Arc<Self>, username: &str) -> Result<SessionSlot, String> {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        if live.values().sum::<usize>() >= self.max {
            return Err(format!("{} terminal sessions are open (tui_max_sessions)", self.max));
        }
        let mine = live.entry(username.to_string()).or_default();
        if *mine >= self.max_per_user {
            return Err(format!(
                "you have {} terminal sessions open (tui_max_sessions_per_user)",
                self.max_per_user
            ));
        }
        *mine += 1;
        Ok(SessionSlot {
            sessions: Arc::clone(self),
            username: username.to_string(),
        })
    }
}

pub(super) struct SessionSlot {
    sessions: Arc<Sessions>,
    username: String,
}

impl Drop for SessionSlot {
    fn drop(&mut self) {
        let mut live = self.sessions.live.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = live.get_mut(&self.username) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                live.remove(&self.username);
            }
        }
    }
}

/// What one channel's `apps` needs from the connection.
pub(super) struct Apps {
    pub factory: Weak<ExecutionContextFactory>,
    pub sessions: Arc<Sessions>,
    pub username: String,
    pub terminal: Terminal,
    /// The window's columns and rows: the pty request's, then each window change.
    pub size: watch::Receiver<(u16, u16)>,
}

/// Frames to the channel task: each `flush` sends what was written since as one chunk, so a frame goes out whole.
struct ChannelWriter {
    pending: Vec<u8>,
    out: tokio_mpsc::UnboundedSender<Vec<u8>>,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        self.out
            .send(std::mem::take(&mut self.pending))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the channel closed"))
    }
}

/// Log lines an SSH session's pane holds while it is closed.
const LOG_LINES: usize = 500;

/// Whether `username` may view the System app (`component://tools/screen/System.xml`, which its Log Viewer inherits),
/// checked on the request pool as that user. A check that fails is a no.
async fn may_read_log(factory: &Weak<ExecutionContextFactory>, username: &str) -> bool {
    let Some(factory) = factory.upgrade() else {
        return false;
    };
    let username = username.to_string();
    let checked = tokio::task::spawn_blocking(move || {
        let owner = Arc::clone(&factory);
        let rx = factory
            .request_pool()
            .try_submit(owner, move || {
                moqui_core::user::internal_login_user(&username)?;
                Ok(moqui_core::user::authorize("AT_XML_SCREEN", "component://tools/screen/System.xml", "view").is_ok())
            })
            .ok()?;
        rx.recv().ok()?.ok()
    })
    .await;
    checked.ok().flatten().unwrap_or(false)
}

/// Run the client on `stream` until the user quits or the channel closes. A refusal (the session limits) is
/// written to the stream as one line. Returns the exit status.
pub(super) async fn serve<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, apps: &mut Apps, newline: &str) -> u32 {
    let slot = match apps.sessions.take(&apps.username) {
        Ok(slot) => slot,
        Err(reason) => {
            tracing::warn!("SSH apps for {} refused: {reason}", apps.username);
            let _ = stream.write_all(format!("apps: {reason}{newline}").as_bytes()).await;
            return 1;
        }
    };
    let (columns, rows) = *apps.size.borrow_and_update();
    let (events, inbox) = mpsc::channel::<SessionEvent>();
    // The log pane, WARN and ERROR only, for a user who may view the System app, whose Log Viewer reads the same log.
    let log = may_read_log(&apps.factory, &apps.username).await;
    if log {
        let lines = crate::logging::tap_at(LOG_LINES, tracing::Level::WARN);
        let to_session = events.clone();
        let _ = std::thread::Builder::new()
            .name(format!("ssh-apps-log-{}", apps.username))
            .spawn(move || {
                // Ends when the session's inbox is gone, at the next line after it.
                while let Ok(line) = lines.recv() {
                    if to_session.send(SessionEvent::Log(line)).is_err() {
                        break;
                    }
                }
            });
    }
    let (out, mut frames) = tokio_mpsc::unbounded_channel::<Vec<u8>>();
    let (finished, ended) = oneshot::channel();
    let options = SessionOptions {
        path: START_PATH.to_string(),
        theme: apps.terminal.theme(),
        columns: columns.max(20),
        rows: rows.max(10),
        notes: Some(|note| tracing::warn!("terminal theme: {note}")),
        log,
    };
    let link = HostLink::weak_as_user(Weak::clone(&apps.factory), apps.username.clone());
    let replies = events.clone();
    let username = apps.username.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("ssh-apps-{username}"))
        .spawn(move || {
            let mut writer = ChannelWriter {
                pending: Vec::new(),
                out,
            };
            let result = moqui_tui::run_session(link, options, &inbox, &replies, &mut writer);
            let _ = writer.flush();
            drop(writer);
            let _ = finished.send(result);
        });
    if let Err(err) = spawned {
        tracing::error!("SSH apps for {username}: could not start its thread: {err}");
        let _ = stream
            .write_all(format!("apps: could not start{newline}").as_bytes())
            .await;
        return 1;
    }
    tracing::info!("SSH apps session of {username} started at {columns}x{rows}");
    let mut buffer = [0u8; 4096];
    let mut reading = true;
    // Off once the connection drops the size's sender, so a closed watch does not wake the loop again and again.
    let mut sizing = true;
    let mut idle_until = tokio::time::Instant::now() + super::server::IDLE;
    let mut timed_out = false;
    loop {
        tokio::select! {
            read = stream.read(&mut buffer), if reading => match read {
                Ok(0) | Err(_) => {
                    reading = false;
                    let _ = events.send(SessionEvent::Close);
                }
                Ok(read) => {
                    idle_until = tokio::time::Instant::now() + super::server::IDLE;
                    let _ = events.send(SessionEvent::Input(buffer[..read].to_vec()));
                }
            },
            () = tokio::time::sleep_until(idle_until), if reading => {
                reading = false;
                timed_out = true;
                let _ = events.send(SessionEvent::Close);
            }
            frame = frames.recv() => match frame {
                Some(frame) => {
                    if stream.write_all(&frame).await.is_err() {
                        reading = false;
                        let _ = events.send(SessionEvent::Close);
                    }
                }
                // The session ended and dropped its writer.
                None => break,
            },
            changed = apps.size.changed(), if sizing => {
                if changed.is_ok() {
                    let (columns, rows) = *apps.size.borrow_and_update();
                    let _ = events.send(SessionEvent::Resize { columns, rows });
                } else {
                    sizing = false;
                }
            }
        }
    }
    if timed_out {
        tracing::info!(
            "SSH apps session of {username} ended after {:?} without input",
            super::server::IDLE
        );
        let minutes = super::server::IDLE.as_secs() / 60;
        let _ = stream
            .write_all(format!("apps: closed after {minutes} minutes without input{newline}").as_bytes())
            .await;
    }
    let _ = stream.flush().await;
    drop(slot);
    match ended.await {
        Ok(Ok(())) => {
            tracing::info!("SSH apps session of {username} ended");
            0
        }
        Ok(Err(err)) => {
            tracing::error!("SSH apps session of {username} failed: {err}");
            1
        }
        Err(_) => {
            tracing::error!("SSH apps session of {username} stopped without a result");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Sessions, Terminal};

    #[test]
    fn sessions_are_limited_in_all_and_per_user() {
        let sessions = Arc::new(Sessions::new(3, 2));
        let first = sessions.take("a").unwrap();
        let _second = sessions.take("a").unwrap();
        assert!(sessions.take("a").err().unwrap().contains("tui_max_sessions_per_user"));
        let _third = sessions.take("b").unwrap();
        assert!(sessions.take("c").err().unwrap().contains("tui_max_sessions"));
        drop(first);
        assert!(sessions.take("c").is_ok());
    }

    #[test]
    fn the_terminal_sets_colors_and_ascii() {
        let mut terminal = Terminal {
            term: "xterm-256color".to_string(),
            ..Terminal::default()
        };
        assert_eq!(terminal.theme().depth, moqui_tui::Depth::Ansi256);
        assert!(!terminal.theme().ascii);
        terminal.env.insert("COLORTERM".to_string(), "truecolor".to_string());
        terminal.env.insert("LANG".to_string(), "C".to_string());
        assert_eq!(terminal.theme().depth, moqui_tui::Depth::TrueColor);
        assert!(terminal.theme().ascii);
        terminal.env.insert("LC_ALL".to_string(), "en_US.UTF-8".to_string());
        assert!(!terminal.theme().ascii);
    }
}
