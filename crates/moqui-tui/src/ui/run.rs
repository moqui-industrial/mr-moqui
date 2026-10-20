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

//! The local loop on Revue's `App::run` (section 3.5), and the headless snapshot (4.5). Requests run on worker threads,
//! one per request, which the session's lanes keep to one per purpose; answers come back on a channel and are applied
//! on the next tick, so a slow request never stops the keys.

use std::io::Write;
use std::sync::mpsc;
use std::sync::Arc;

use revue::core::app::App;
use revue::event::{Event, Key as RevueKey, KeyEvent};

use crate::keys::{Key, KeyPress};
use crate::link::{Reply, ScreenLink};
use crate::session::Session;
use crate::theme::Theme;

use super::AppView;

/// What `moqui tui` passes to the local loop.
#[derive(Debug, Default)]
pub struct LocalOptions {
    pub path: String,
    pub theme: Theme,
    /// Called with a note for the log: a theme value that did not apply.
    pub notes: Option<fn(&str)>,
    /// Log lines for the log pane (Alt+L).
    pub log: Option<mpsc::Receiver<String>>,
}

/// Run the client on this terminal until the user quits.
///
/// # Errors
///
/// Returns Revue's error when the terminal cannot be set up or drawn.
pub fn run_local<L: ScreenLink + 'static>(link: L, options: LocalOptions) -> revue::Result<()> {
    let link = Arc::new(link);
    let mut view = AppView::with_session(Session::start(&options.path), Arc::clone(&link));
    view.set_theme(options.theme.clone());
    let (done_tx, done_rx) = mpsc::channel::<(u64, Reply)>();
    let notes = options.notes;
    let log = options.log;
    // Revue's terminal setup does not turn on bracketed paste, so a paste would arrive as keys and a newline in it would
    // submit a form; ask for it before the loop and turn it off after (section 3.3).
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[?2004h");
    let _ = out.flush();
    send(&mut view, &link, &done_tx);
    let result = App::builder()
        .mouse_capture(false)
        .build()
        .run(view, move |event, view, app| {
            let changed = match event {
                Event::Key(key) => {
                    let quit = from_revue(key).is_some_and(|press| view.session_mut().on_key(&press));
                    if quit {
                        app.quit();
                    }
                    true
                }
                Event::Paste(text) => {
                    view.session_mut().paste(text);
                    true
                }
                Event::Resize(width, height) => {
                    view.session_mut().resize(*width, *height);
                    true
                }
                Event::Tick => {
                    let mut any = false;
                    while let Ok((id, reply)) = done_rx.try_recv() {
                        view.session_mut().on_reply(id, reply);
                        any = true;
                    }
                    if let Some(log) = &log {
                        while let Ok(line) = log.try_recv() {
                            view.push_log(line);
                            any |= view.session().log_open();
                        }
                    }
                    // The spinner and the message expiry move without a key.
                    any || view.session().is_busy() || view.session().next_deadline().is_some()
                }
                Event::Mouse(_) | Event::FocusGained | Event::FocusLost => false,
            };
            view.sync_shell();
            if let Some(log) = notes {
                for note in view.take_notes() {
                    log(&note);
                }
            }
            send(view, &link, &done_tx);
            changed
        });
    let _ = out.write_all(b"\x1b[?2004l");
    let _ = out.flush();
    result
}

/// Start the session's queued requests on worker threads.
fn send<L: ScreenLink + 'static>(view: &mut AppView<Arc<L>>, link: &Arc<L>, done: &mpsc::Sender<(u64, Reply)>) {
    for request in view.session_mut().take_requests() {
        let link = Arc::clone(link);
        let done = done.clone();
        std::thread::spawn(move || {
            let reply = link.call(&request.call);
            let _ = done.send((request.id, reply));
        });
    }
}

/// The screen at `path` drawn at `width` by `height`, as text: `moqui tui --snapshot`, and a check of a design without a
/// terminal.
pub fn snapshot<L: ScreenLink>(link: L, path: &str, width: u16, height: u16, theme: Theme) -> String {
    let mut view = AppView::with_session(Session::start(path), link);
    view.set_theme(theme);
    view.session_mut().resize(width, height);
    view.run_requests();
    view.screen_text(width, height)
}

/// A Revue key as the client's.
pub fn from_revue(event: &KeyEvent) -> Option<KeyPress> {
    let key = match event.key {
        RevueKey::Char(ch) => Key::Char(ch),
        RevueKey::Enter => Key::Enter,
        RevueKey::Escape => Key::Escape,
        RevueKey::Tab => Key::Tab,
        RevueKey::BackTab => Key::BackTab,
        RevueKey::Backspace => Key::Backspace,
        RevueKey::Delete => Key::Delete,
        RevueKey::Up => Key::Up,
        RevueKey::Down => Key::Down,
        RevueKey::Left => Key::Left,
        RevueKey::Right => Key::Right,
        RevueKey::Home => Key::Home,
        RevueKey::End => Key::End,
        RevueKey::PageUp => Key::PageUp,
        RevueKey::PageDown => Key::PageDown,
        RevueKey::F(n) => Key::F(n),
        RevueKey::Insert | RevueKey::Null | RevueKey::Unknown => return None,
    };
    Some(KeyPress {
        key,
        alt: event.alt,
        ctrl: event.ctrl,
        shift: event.shift,
    })
}
