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

//! The Revue view (section 3.2). [`AppView`] holds the session, the theme and the breakpoints, and its `render` draws the
//! frame and the canvas from them every frame: the session owns the state, the view owns none but the canvas scroll.

mod agent;
mod draw;
mod encode;
mod overlays;
mod paint;
mod run;
mod session_loop;

use std::cell::Cell;
use std::collections::VecDeque;

use revue::testing::PipelineHarness;
use revue::widget::{RenderContext, View};

use crate::keys::KeyPress;
use crate::layout::Breakpoints;
use crate::link::{FakeLink, ScreenLink, ShellInfo};
use crate::model::ScreenModel;
use crate::session::Session;
use crate::theme::Theme;

pub use agent::{Agent, AgentAnswer, AGENT_HELP};
pub use encode::{Screen, ENTER, LEAVE};
pub use run::{from_revue, run_local, snapshot, LocalOptions};
pub use session_loop::{run_session, SessionEvent, SessionOptions};

pub struct AppView<L: ScreenLink = FakeLink> {
    session: Session,
    link: L,
    theme: Theme,
    base_theme: Theme,
    breakpoints: Breakpoints,
    scroll: Cell<usize>,
    /// The title line's clock, fixed for a snapshot; live when None.
    clock: Option<String>,
    /// The shell data the theme and breakpoints were last taken from.
    applied: ShellInfo,
    notes: Vec<String>,
    /// The newest log lines, for the log pane (Alt+L).
    log: VecDeque<String>,
}

/// Lines the log pane keeps.
const LOG_LINES: usize = 500;

impl<L: ScreenLink> AppView<L> {
    pub fn new(model: ScreenModel, link: L) -> Self {
        Self::with_session(Session::new(model), link)
    }

    pub fn with_session(session: Session, link: L) -> Self {
        let theme = Theme::default();
        Self {
            session,
            link,
            base_theme: theme.clone(),
            theme,
            breakpoints: Breakpoints::default(),
            scroll: Cell::new(0),
            clock: None,
            applied: ShellInfo::default(),
            notes: Vec::new(),
            log: VecDeque::new(),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn link(&self) -> &L {
        &self.link
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The terminal's theme (colors, ASCII); the server theme's colors are merged over it when the shell data comes.
    pub fn set_theme(&mut self, theme: Theme) {
        self.base_theme = theme.clone();
        self.theme = theme;
        self.applied = ShellInfo::default();
        self.sync_shell();
    }

    /// Show this in place of the clock, so a snapshot does not change by the minute.
    pub fn set_clock(&mut self, clock: &str) {
        self.clock = Some(clock.to_string());
    }

    /// A key, and its requests run on this thread. Returns true when the client should end.
    pub fn on_key(&mut self, press: &KeyPress) -> bool {
        let quit = self.session.press(press, &self.link);
        self.sync_shell();
        quit
    }

    /// Run what the session queued, on this thread.
    pub fn run_requests(&mut self) {
        self.session.run_now(&self.link);
        self.sync_shell();
    }

    /// Take the theme's colors, breakpoints and key bindings once the shell data changes. What does not read is noted
    /// for the log, and the defaults stay (D11).
    pub fn sync_shell(&mut self) {
        if self.session.shell() == &self.applied {
            return;
        }
        let shell = self.session.shell().clone();
        let mut theme = self.base_theme.clone();
        self.notes.extend(theme.merge_colors(&shell.colors));
        self.theme = theme;
        let (breakpoints, skipped) = Breakpoints::with(&shell.breakpoints);
        self.breakpoints = breakpoints;
        self.notes.extend(skipped);
        self.notes
            .extend(self.session.key_skips().iter().map(ToString::to_string));
        self.applied = shell;
    }

    /// A log line for the log pane.
    pub fn push_log(&mut self, line: String) {
        if self.log.len() == LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// What did not apply from the theme, for the log.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// The screen as text at `width` by `height`, drawn through Revue's real pipeline.
    pub fn screen_text(&mut self, width: u16, height: u16) -> String {
        let mut harness = PipelineHarness::new(width, height);
        harness.draw(self);
        // The second cell of a wide glyph is a NUL spacer. Drop it so the text is the characters a reader sees.
        harness
            .screen_text()
            .replace('\0', "")
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl<L: ScreenLink> View for AppView<L> {
    fn render(&self, ctx: &mut RenderContext) {
        let log = self.session.log_open().then_some(&self.log);
        draw::draw(
            &self.session,
            &self.theme,
            &self.breakpoints,
            &self.scroll,
            self.clock.as_deref(),
            log,
            ctx,
        );
    }
}

/// One screen as text, for a snapshot test.
pub fn render_text(model: &ScreenModel, link: &FakeLink, width: u16, height: u16) -> String {
    let mut view = AppView::new(model.clone(), link.clone());
    view.set_clock("Tue 14:32");
    view.screen_text(width, height)
}
