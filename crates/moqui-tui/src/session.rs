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

//! The interpreter's state and its key dispatch (D5). It does no I/O: a key queues [`LinkCall`]s, the loop takes them
//! with [`Session::take_requests`], runs them off the UI thread and hands each answer back to [`Session::on_reply`].
//! [`Session::run_now`] does both on this thread, for tests and the headless snapshot.
//!
//! Keys go through the layers of section 3.3, the first that takes one stops it: the confirmation, the login, the help
//! and info boxes, an open drop-down or date picker, the drawer, link numbering, the focused field's editing keys, a
//! list's rows, and last the keymap.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::edit::{self, Edited, Limits};
use crate::forms::{self, FocusItem, FocusKind, Index, Scope};
use crate::keys::{merge_keys, Action, Key, KeyMap, KeyPress, MergeSkip};
use crate::layout::PAGE_ROWS;
use crate::link::{row_link, LinkCall, LinkError, MenuItem, Reply, ScreenLink, ShellInfo, TransitionOutcome};
use crate::model::{Choice, FormList, ScreenModel, Widget};
use crate::nav::{level_path, Location, Nav};
use crate::state::{Confirm, ConfirmKind, Edits, Finished, Lanes, Purpose, Request, Then};

/// How long a success message stays on the message line (section 1.7).
const MESSAGE_SECONDS: u64 = 3;

/// An open drop-down list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropDown {
    pub field: String,
    /// The highlighted entry, among the filtered ones.
    pub at: usize,
    /// Typed text that narrows the list.
    pub filter: String,
}

/// The date picker of a date field (F4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub key: String,
    pub date: jiff::civil::Date,
}

/// The login dialog after a 401 (section 1.7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Login {
    pub username: String,
    pub password: String,
    pub code: String,
    /// 0 username, 1 password, 2 code.
    pub at: usize,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    Dialog(String),
    /// The find dialog of a list: its header fields.
    Find(String),
    DropDown(DropDown),
    Picker(Picker),
    Confirm(Confirm),
    Help,
    Info,
    Login(Login),
    /// The drawer has focus, at this row.
    Drawer(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
    until: Option<Instant>,
}

/// What a level request was for, so its answer is applied the right way.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Intent {
    /// A new screen: the edits go, and with `history` the screen left is remembered.
    Navigate { history: bool },
    /// The same screen again (paging, sort, refresh, after a save): edits that are still dirty stay.
    Reload,
}

/// The form a transition was posted from, to apply its answer.
#[derive(Clone, Debug)]
struct Submitted {
    form: String,
    keys: Vec<String>,
    submit_message: String,
    background_hide_id: String,
    then: Option<Then>,
}

#[derive(Clone, Debug)]
pub struct Session {
    nav: Nav,
    index: Index,
    edits: Edits,
    cursors: BTreeMap<String, usize>,
    focus: Vec<FocusItem>,
    focus_at: usize,
    overlays: Vec<Overlay>,
    rows: BTreeMap<String, usize>,
    selected: BTreeMap<String, BTreeSet<String>>,
    hscroll: BTreeMap<String, usize>,
    message: Message,
    lanes: Lanes,
    intents: BTreeMap<u64, (Intent, BTreeMap<String, String>)>,
    submitted: Option<Submitted>,
    /// A save's message, shown when the screen asked for after it arrives.
    after_level: Option<String>,
    keys: KeyMap,
    key_skips: Vec<MergeSkip>,
    shell: ShellInfo,
    numbering: bool,
    typed_number: String,
    drawer: Option<bool>,
    /// The screen asked for last, asked for again after a login.
    last_location: Option<Location>,
    quit: bool,
    open: Option<String>,
    redraw: bool,
    log_open: bool,
    page_rows: Option<usize>,
    mobile: bool,
}

impl Session {
    pub fn new(model: ScreenModel) -> Self {
        let mut session = Self {
            nav: Nav::new(model),
            index: Index::default(),
            edits: Edits::default(),
            cursors: BTreeMap::new(),
            focus: Vec::new(),
            focus_at: 0,
            overlays: Vec::new(),
            rows: BTreeMap::new(),
            selected: BTreeMap::new(),
            hscroll: BTreeMap::new(),
            message: Message::default(),
            lanes: Lanes::default(),
            intents: BTreeMap::new(),
            submitted: None,
            after_level: None,
            keys: KeyMap::baseline(),
            key_skips: Vec::new(),
            shell: ShellInfo::default(),
            numbering: false,
            typed_number: String::new(),
            drawer: None,
            last_location: None,
            quit: false,
            open: None,
            redraw: false,
            log_open: false,
            page_rows: None,
            mobile: false,
        };
        session.reindex(false);
        session.show_model_messages();
        session
    }

    /// A session that starts by asking for the shell data and the screen at `path`.
    pub fn start(path: &str) -> Self {
        let mut session = Self::new(ScreenModel {
            v: crate::model::MODEL_VERSION,
            ..ScreenModel::default()
        });
        session.request_shell(path);
        session.navigate(path, false);
        session
    }

    // ---- the loop's side ----

    /// The calls to run now.
    pub fn take_requests(&mut self) -> Vec<Request> {
        self.lanes.take()
    }

    /// The answer to request `id`.
    pub fn on_reply(&mut self, id: u64, reply: Reply) {
        let purpose = match self.lanes.finish(id) {
            Finished::Apply(purpose) => purpose,
            Finished::Superseded | Finished::Unknown => {
                self.intents.remove(&id);
                return;
            }
        };
        if let Some(LinkError::Auth(_)) = reply.error() {
            self.intents.remove(&id);
            if purpose != Purpose::Login {
                self.ask_login();
            }
            return;
        }
        match (purpose, reply) {
            (Purpose::Level, Reply::Level(result)) => self.level_reply(id, result),
            (Purpose::Parent(_), Reply::Level(result)) => {
                let parameters = self.intents.remove(&id).map(|(_, params)| params).unwrap_or_default();
                // A parent that does not load leaves the leaf as it is: the header is extra.
                if let Ok(model) = result {
                    self.nav.set_parent(model, parameters);
                    self.reindex(true);
                }
            }
            (Purpose::Transition, Reply::Transition(result)) => self.transition_reply(result),
            (Purpose::Options(field), Reply::Options(result)) => match result {
                Ok(choices) => self.apply_choices(&field, choices),
                Err(err) => self.set_message(format!("Could not load the choices: {err}"), true),
            },
            (Purpose::Shell, Reply::Shell(result)) => match result {
                Ok(shell) => self.apply_shell(shell),
                // The screens work without the drawer.
                Err(err) => self.set_message(format!("Could not load the menu: {err}"), true),
            },
            (Purpose::Login, Reply::Login(result)) => self.login_reply(result),
            _ => {}
        }
    }

    /// Run every queued call on this thread until none is left.
    pub fn run_now(&mut self, link: &dyn ScreenLink) {
        for _ in 0..64 {
            let requests = self.take_requests();
            if requests.is_empty() {
                return;
            }
            for request in requests {
                let reply = link.call(&request.call);
                self.on_reply(request.id, reply);
            }
        }
    }

    /// One key, then the calls it queued, on this thread. Returns true when the session should end.
    pub fn press(&mut self, press: &KeyPress, link: &dyn ScreenLink) -> bool {
        let quit = self.on_key(press);
        self.run_now(link);
        quit
    }

    /// The terminal size changed. The page size of a list follows the rows (10 to 100), and below 80 columns the
    /// mobile column set is asked for (section 1.3). A change reloads the screen.
    pub fn resize(&mut self, columns: u16, rows: u16) {
        let page_rows = usize::from(rows).saturating_sub(14).clamp(PAGE_ROWS, 100);
        let mobile = columns < 80;
        let changed = self.page_rows != Some(page_rows) || self.mobile != mobile;
        let first = self.page_rows.is_none();
        self.page_rows = Some(page_rows);
        self.mobile = mobile;
        if changed && !first && self.has_paged_list() {
            self.reload();
        }
    }

    // ---- what the view reads ----

    pub fn nav(&self) -> &Nav {
        &self.nav
    }

    /// The leaf level.
    pub fn model(&self) -> &ScreenModel {
        self.nav.leaf()
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn path(&self) -> &str {
        &self.model().screen.path
    }

    pub fn paths(&self) -> Vec<&str> {
        self.nav.paths()
    }

    pub fn parameters(&self) -> &BTreeMap<String, String> {
        self.nav.leaf_parameters()
    }

    pub fn order_by(&self) -> &str {
        self.parameters().get("orderByField").map(String::as_str).unwrap_or("")
    }

    pub fn focus(&self) -> Option<&FocusItem> {
        self.focus.get(self.focus_at)
    }

    /// The focused item's key: a node id, or `field#part`.
    pub fn focus_id(&self) -> &str {
        self.focus().map(|item| item.key.as_str()).unwrap_or("")
    }

    pub fn focus_kind(&self) -> Option<FocusKind> {
        self.focus().map(|item| item.kind)
    }

    pub fn focus_order(&self) -> &[FocusItem] {
        &self.focus
    }

    pub fn focused_is_text(&self) -> bool {
        self.focus_kind() == Some(FocusKind::Text)
    }

    pub fn buffer(&self, key: &str) -> &str {
        self.edits.get(key)
    }

    pub fn cursor(&self, key: &str) -> usize {
        self.cursors
            .get(key)
            .copied()
            .unwrap_or_else(|| self.buffer(key).chars().count())
    }

    /// Set a field's value as if typed.
    pub fn set_buffer(&mut self, key: &str, value: String) {
        let count = value.chars().count();
        self.edits.set(key, value);
        self.cursors.insert(key.to_string(), count);
        self.after_edit(key);
    }

    pub fn is_dirty(&self, key: &str) -> bool {
        self.edits.is_dirty(key)
    }

    /// A form has an edit that is not saved: its primary button is drawn strong.
    pub fn form_dirty(&self, form_id: &str) -> bool {
        self.index.forms.get(form_id).is_some_and(|form| {
            form.fields.iter().any(|id| {
                self.index
                    .fields
                    .get(id)
                    .is_some_and(|field| forms::field_keys(field).iter().any(|key| self.edits.is_dirty(key)))
            })
        })
    }

    pub fn any_dirty(&self) -> bool {
        self.edits.any_dirty()
    }

    pub fn mark(&self, key: &str) -> Option<&str> {
        self.edits.mark(key)
    }

    pub fn marks(&self) -> &BTreeMap<String, String> {
        self.edits.marks()
    }

    /// The message line, empty once a success message has had its 3 seconds.
    pub fn message(&self) -> &str {
        if self.message_expired() {
            ""
        } else {
            &self.message.text
        }
    }

    pub fn message_is_error(&self) -> bool {
        self.message.error && !self.message.text.is_empty()
    }

    fn message_expired(&self) -> bool {
        self.message.until.is_some_and(|until| Instant::now() >= until)
    }

    /// When the loop has something timed to do: the success message's expiry.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.message.until.filter(|_| !self.message_expired())
    }

    pub fn is_busy(&self) -> bool {
        self.lanes.any_busy()
    }

    /// Requests in flight and waiting.
    pub fn request_counts(&self) -> (usize, usize) {
        self.lanes.counts()
    }

    pub fn overlays(&self) -> &[Overlay] {
        &self.overlays
    }

    pub fn top_overlay(&self) -> Option<&Overlay> {
        self.overlays.last()
    }

    pub fn dialogs_open(&self) -> Vec<&str> {
        self.overlays
            .iter()
            .filter_map(|overlay| match overlay {
                Overlay::Dialog(id) => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn find_open(&self) -> Option<&str> {
        self.overlays.iter().rev().find_map(|overlay| match overlay {
            Overlay::Find(id) => Some(id.as_str()),
            _ => None,
        })
    }

    pub fn dropdown(&self) -> Option<&DropDown> {
        match self.top_overlay() {
            Some(Overlay::DropDown(drop)) => Some(drop),
            _ => None,
        }
    }

    pub fn confirm(&self) -> Option<&Confirm> {
        self.overlays.iter().rev().find_map(|overlay| match overlay {
            Overlay::Confirm(confirm) => Some(confirm),
            _ => None,
        })
    }

    pub fn confirm_choice(&self) -> Option<&str> {
        self.confirm().map(Confirm::choice)
    }

    /// The choices of the open drop-down, narrowed by what was typed.
    pub fn dropdown_choices(&self) -> Vec<Choice> {
        let Some(drop) = self.dropdown() else {
            return Vec::new();
        };
        filtered(self.index.choices(&drop.field), &drop.filter)
    }

    pub fn row_at(&self, list_id: &str) -> usize {
        self.rows.get(list_id).copied().unwrap_or(0)
    }

    pub fn selected(&self, list_id: &str) -> Option<&BTreeSet<String>> {
        self.selected.get(list_id)
    }

    pub fn hscroll(&self, list_id: &str) -> usize {
        self.hscroll.get(list_id).copied().unwrap_or(0)
    }

    pub fn list(&self, list_id: &str) -> Option<&FormList> {
        self.index.lists.get(list_id)
    }

    pub fn page_index(&self, list_id: &str) -> usize {
        self.list(list_id)
            .map_or(0, |list| usize::try_from(list.paging.index).unwrap_or(0))
    }

    pub fn page_count(&self, list_id: &str) -> usize {
        self.list(list_id).map_or(1, page_count_of)
    }

    pub fn shell(&self) -> &ShellInfo {
        &self.shell
    }

    pub fn menu(&self) -> &[MenuItem] {
        &self.shell.menu
    }

    pub fn set_menu(&mut self, menu: Vec<MenuItem>) {
        self.shell.menu = menu;
    }

    pub fn keys(&self) -> &KeyMap {
        &self.keys
    }

    /// Theme key rows that were not applied, for the log.
    pub fn key_skips(&self) -> &[MergeSkip] {
        &self.key_skips
    }

    pub fn numbering(&self) -> bool {
        self.numbering
    }

    /// The number `#` shows before a link, from 1.
    pub fn link_number(&self, link_id: &str) -> Option<usize> {
        self.numbering
            .then(|| {
                self.index
                    .link_order
                    .iter()
                    .position(|id| id == link_id)
                    .map(|at| at + 1)
            })
            .flatten()
    }

    pub fn typed_number(&self) -> &str {
        &self.typed_number
    }

    /// Whether the user asked for the drawer shown (F9); None follows the width.
    pub fn drawer_choice(&self) -> Option<bool> {
        self.drawer
    }

    pub fn drawer_focus(&self) -> Option<usize> {
        self.overlays.iter().find_map(|overlay| match overlay {
            Overlay::Drawer(at) => Some(*at),
            _ => None,
        })
    }

    /// A url the user asked to open in the browser (`o`), once.
    pub fn take_open(&mut self) -> Option<String> {
        self.open.take()
    }

    /// Ctrl+L asked for a full redraw, once.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// Alt+L showed the log pane.
    pub fn log_open(&self) -> bool {
        self.log_open
    }

    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// The focused field's widget.
    pub fn field_widget(&self, id: &str) -> Option<&Widget> {
        let node = id.split('#').next().unwrap_or(id);
        self.index.fields.get(node).map(|field| &field.widget)
    }

    /// Whether a field shows now, by its `visible-when`.
    pub fn visible(&self, field_id: &str) -> bool {
        self.index.visible(field_id, &|key| self.edits.get(key).to_string())
    }

    /// The rows of a list's page, as the model has them.
    pub fn list_page(&self, list_id: &str) -> Vec<crate::model::ListRow> {
        self.list(list_id).map(|list| list.rows.clone()).unwrap_or_default()
    }

    // ---- keys ----

    /// One key. Returns true when the session should end.
    pub fn on_key(&mut self, press: &KeyPress) -> bool {
        if self.message_expired() {
            self.clear_message();
        }
        let handled =
            self.overlay_key(press) || self.number_key(press) || self.field_key(press) || self.list_key(press);
        if !handled {
            if let Some(action) = self.keys.action(press) {
                self.act(action);
            }
        }
        self.quit
    }

    /// Pasted text, as one event: into the focused text field through its limits. A newline in a one-line field is a
    /// space, so a paste never submits a form.
    pub fn paste(&mut self, text: &str) {
        let Some(item) = self.focus().cloned().filter(|item| item.kind == FocusKind::Text) else {
            return;
        };
        let limits = self.limits(&item.key);
        let mut value = self.buffer(&item.key).to_string();
        let mut cursor = self.cursor(&item.key);
        for ch in text.chars().filter(|ch| *ch != '\r') {
            let press = if ch == '\n' && limits.multiline {
                KeyPress::plain(Key::Enter)
            } else if ch == '\n' || ch == '\t' {
                KeyPress::plain(Key::Char(' '))
            } else {
                KeyPress::plain(Key::Char(ch))
            };
            edit::apply(&mut value, &mut cursor, &press, &limits);
        }
        self.edits.set(&item.key, value);
        self.cursors.insert(item.key.clone(), cursor);
        self.after_edit(&item.key);
    }

    fn overlay_key(&mut self, press: &KeyPress) -> bool {
        match self.overlays.last().cloned() {
            Some(Overlay::Confirm(confirm)) => {
                self.confirm_key(confirm, press);
                true
            }
            Some(Overlay::Login(login)) => {
                self.login_key(login, press);
                true
            }
            Some(Overlay::Help | Overlay::Info) => {
                // Any key closes them; Ctrl+Q still quits.
                self.overlays.pop();
                if self.keys.action(press) == Some(Action::Quit) {
                    self.act(Action::Quit);
                }
                true
            }
            Some(Overlay::DropDown(drop)) => {
                self.dropdown_key(drop, press);
                true
            }
            Some(Overlay::Picker(picker)) => {
                self.picker_key(picker, press);
                true
            }
            Some(Overlay::Drawer(at)) => self.drawer_key(at, press),
            Some(Overlay::Dialog(_) | Overlay::Find(_)) | None => false,
        }
    }

    fn confirm_key(&mut self, mut confirm: Confirm, press: &KeyPress) {
        let chosen = match &press.key {
            Key::Left | Key::Up | Key::BackTab => {
                confirm.move_by(-1);
                None
            }
            Key::Right | Key::Down | Key::Tab => {
                confirm.move_by(1);
                None
            }
            Key::Enter => Some(confirm.choice()),
            Key::Escape | Key::F(12) => Some(confirm.choices()[0]),
            Key::Char(ch) => confirm
                .choices()
                .iter()
                .find(|choice| {
                    choice
                        .chars()
                        .next()
                        .is_some_and(|first| first.eq_ignore_ascii_case(ch))
                })
                .copied(),
            _ => None,
        };
        self.pop_confirm();
        match chosen {
            None => self.overlays.push(Overlay::Confirm(confirm)),
            Some(choice) => self.finish_confirm(&confirm, choice),
        }
    }

    fn pop_confirm(&mut self) {
        if let Some(at) = self
            .overlays
            .iter()
            .rposition(|overlay| matches!(overlay, Overlay::Confirm(_)))
        {
            self.overlays.remove(at);
        }
    }

    fn finish_confirm(&mut self, confirm: &Confirm, choice: &str) {
        match (&confirm.kind, choice) {
            (ConfirmKind::Leave, "Discard") => {
                self.edits.reset();
                self.cursors.clear();
                self.run_then(confirm.then.clone());
            }
            (ConfirmKind::Leave, "Save") => {
                if let Some(form) = self.first_dirty_form() {
                    self.submit_form(&form, Some(confirm.then.clone()));
                } else {
                    self.set_message("These edits are not in a form that can be saved.".to_string(), true);
                }
            }
            (ConfirmKind::Ask(_), "Yes") => self.run_then(confirm.then.clone()),
            _ => {}
        }
    }

    fn run_then(&mut self, then: Then) {
        match then {
            Then::Navigate(url) => self.navigate(&url, true),
            Then::Back => self.go_back_now(),
            Then::Quit => self.quit = true,
            Then::Submit(id) => self.submit_form_of(&id, None, true),
            Then::FollowLink(id) => self.follow_link_now(&id),
            Then::Nothing => {}
        }
    }

    fn login_key(&mut self, mut login: Login, press: &KeyPress) {
        let fields = 3;
        match &press.key {
            Key::Tab | Key::Down => login.at = (login.at + 1) % fields,
            Key::BackTab | Key::Up => login.at = (login.at + fields - 1) % fields,
            Key::Enter => {
                self.overlays.pop();
                self.lanes.push(
                    Purpose::Login,
                    LinkCall::Login {
                        username: login.username.clone(),
                        password: std::mem::take(&mut login.password),
                        code: std::mem::take(&mut login.code),
                    },
                );
                login.error.clear();
                self.overlays.push(Overlay::Login(login));
                return;
            }
            Key::Escape | Key::F(12) => {
                self.overlays.pop();
                self.set_message("Not logged in.".to_string(), true);
                return;
            }
            _ => {
                let text = match login.at {
                    0 => &mut login.username,
                    1 => &mut login.password,
                    _ => &mut login.code,
                };
                let mut cursor = text.chars().count();
                edit::apply(text, &mut cursor, press, &Limits::default());
            }
        }
        self.overlays.pop();
        self.overlays.push(Overlay::Login(login));
    }

    fn dropdown_key(&mut self, mut drop: DropDown, press: &KeyPress) {
        let choices = filtered(self.index.choices(&drop.field), &drop.filter);
        let multiple = matches!(
            self.field_widget(&drop.field),
            Some(Widget::DropDown { multiple: true, .. })
        );
        let combo = matches!(
            self.field_widget(&drop.field),
            Some(Widget::DropDown { combo: true, .. })
        );
        let last = choices.len().saturating_sub(1);
        match &press.key {
            Key::Up => drop.at = drop.at.saturating_sub(1),
            Key::Down => drop.at = (drop.at + 1).min(last),
            Key::PageUp => drop.at = drop.at.saturating_sub(10),
            Key::PageDown => drop.at = (drop.at + 10).min(last),
            Key::Home => drop.at = 0,
            Key::End => drop.at = last,
            Key::Char(' ') if multiple => {
                if let Some(choice) = choices.get(drop.at) {
                    self.toggle_value(&drop.field.clone(), &choice.value);
                }
            }
            Key::Enter => {
                self.overlays.pop();
                if !multiple {
                    let field = drop.field.clone();
                    match choices.get(drop.at) {
                        Some(choice) => self.set_buffer(&field, choice.value.clone()),
                        None if combo && !drop.filter.is_empty() => self.set_buffer(&field, drop.filter.clone()),
                        None => {}
                    }
                    self.submit_on_select(&field);
                }
                return;
            }
            Key::Escape | Key::F(4) | Key::F(12) => {
                self.overlays.pop();
                return;
            }
            Key::Backspace => {
                drop.filter.pop();
                drop.at = 0;
            }
            Key::Char(ch) if press.is_plain() => {
                drop.filter.push(*ch);
                drop.at = 0;
            }
            _ => {}
        }
        self.overlays.pop();
        self.overlays.push(Overlay::DropDown(drop));
    }

    fn submit_on_select(&mut self, field: &str) {
        if matches!(
            self.field_widget(field),
            Some(Widget::DropDown {
                submit_on_select: true,
                ..
            })
        ) {
            self.submit_form_of(field, None, false);
        }
    }

    fn toggle_value(&mut self, field: &str, value: &str) {
        let mut values: Vec<String> = self
            .buffer(field)
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        if let Some(at) = values.iter().position(|item| item == value) {
            values.remove(at);
        } else {
            values.push(value.to_string());
        }
        self.set_buffer(field, values.join(","));
    }

    fn picker_key(&mut self, mut picker: Picker, press: &KeyPress) {
        let days = |n: i64| jiff::Span::new().days(n);
        let moved = match &press.key {
            Key::Left => picker.date.checked_add(days(-1)).ok(),
            Key::Right => picker.date.checked_add(days(1)).ok(),
            Key::Up => picker.date.checked_add(days(-7)).ok(),
            Key::Down => picker.date.checked_add(days(7)).ok(),
            Key::PageUp => picker.date.checked_add(jiff::Span::new().months(-1)).ok(),
            Key::PageDown => picker.date.checked_add(jiff::Span::new().months(1)).ok(),
            Key::Home | Key::Char('t') => Some(jiff::Zoned::now().date()),
            Key::Enter => {
                self.overlays.pop();
                if let Some((format, date_type)) = self.date_format(&picker.key) {
                    let text = forms::date_text(picker.date, self.buffer(&picker.key), &format, &date_type);
                    self.set_buffer(&picker.key.clone(), text);
                }
                return;
            }
            Key::Escape | Key::F(4) | Key::F(12) => {
                self.overlays.pop();
                return;
            }
            _ => None,
        };
        if let Some(date) = moved {
            picker.date = date;
        }
        self.overlays.pop();
        self.overlays.push(Overlay::Picker(picker));
    }

    fn date_format(&self, key: &str) -> Option<(String, String)> {
        match self.field_widget(key)? {
            Widget::DateTime { format, date_type, .. } => Some((format.clone(), date_type.clone())),
            Widget::DateFind { format, date_type, .. } => Some((format.clone(), date_type.clone())),
            Widget::DatePeriod { time, .. } => {
                Some((String::new(), if *time { "date-time" } else { "date" }.to_string()))
            }
            _ => None,
        }
    }

    fn drawer_key(&mut self, at: usize, press: &KeyPress) -> bool {
        let rows = drawer_rows(&self.shell.menu);
        let last = rows.len().saturating_sub(1);
        let next = match &press.key {
            Key::Up => at.saturating_sub(1),
            Key::Down => (at + 1).min(last),
            Key::Enter | Key::Right => {
                self.overlays.pop();
                if let Some(url) = rows.get(at).map(|row| row.1.clone()) {
                    self.follow_url(&url);
                }
                return true;
            }
            Key::F(9) => {
                self.overlays.pop();
                self.drawer = Some(false);
                return true;
            }
            Key::Escape | Key::F(12) | Key::Tab => {
                self.overlays.pop();
                return true;
            }
            _ => return self.keys.action(press) != Some(Action::Quit),
        };
        self.overlays.pop();
        self.overlays.push(Overlay::Drawer(next));
        true
    }

    /// With `#` on, digits name a link and Enter follows it.
    fn number_key(&mut self, press: &KeyPress) -> bool {
        if !self.numbering || self.focused_is_text() || !press.is_plain() {
            return false;
        }
        match &press.key {
            Key::Char(ch) if ch.is_ascii_digit() => {
                self.typed_number.push(*ch);
                self.set_message(format!("Follow link {}", self.typed_number), false);
                true
            }
            Key::Enter if !self.typed_number.is_empty() => {
                let number = std::mem::take(&mut self.typed_number);
                self.clear_message();
                let link = number
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|at| self.index.link_order.get(at))
                    .cloned();
                match link {
                    Some(id) => self.follow_link(&id),
                    None => self.set_message(format!("There is no link {number}"), true),
                }
                true
            }
            Key::Backspace if !self.typed_number.is_empty() => {
                self.typed_number.pop();
                true
            }
            Key::Escape if !self.typed_number.is_empty() => {
                self.typed_number.clear();
                self.clear_message();
                true
            }
            _ => false,
        }
    }

    /// The focused field's own keys: typing in a text field, Space on a check, Left and Right on a radio.
    fn field_key(&mut self, press: &KeyPress) -> bool {
        let Some(item) = self.focus().cloned() else {
            return false;
        };
        match item.kind {
            FocusKind::Text => {
                let limits = self.limits(&item.key);
                let mut text = self.buffer(&item.key).to_string();
                let mut cursor = self.cursor(&item.key);
                match edit::apply(&mut text, &mut cursor, press, &limits) {
                    Edited::No => false,
                    Edited::Moved => {
                        self.cursors.insert(item.key.clone(), cursor);
                        true
                    }
                    Edited::Changed => {
                        self.edits.set(&item.key, text);
                        self.cursors.insert(item.key.clone(), cursor);
                        self.after_edit(&item.key);
                        true
                    }
                }
            }
            FocusKind::Check if press.is_plain() && press.key == Key::Char(' ') => {
                let choices = self.index.choices(&item.key).to_vec();
                match choices.as_slice() {
                    [one] => {
                        let value = if self.buffer(&item.key) == one.value {
                            String::new()
                        } else {
                            one.value.clone()
                        };
                        self.set_buffer(&item.key, value);
                    }
                    _ => self.open_dropdown(&item.key),
                }
                true
            }
            FocusKind::Radio if press.is_plain() && matches!(press.key, Key::Left | Key::Right) => {
                let choices = self.index.choices(&item.key).to_vec();
                if choices.is_empty() {
                    return true;
                }
                let at = choices.iter().position(|choice| choice.value == self.buffer(&item.key));
                let next = match (at, press.key == Key::Right) {
                    (None, _) => 0,
                    (Some(at), true) => (at + 1) % choices.len(),
                    (Some(at), false) => (at + choices.len() - 1) % choices.len(),
                };
                self.set_buffer(&item.key, choices[next].value.clone());
                true
            }
            FocusKind::DropDown if press.is_plain() && press.key == Key::Char(' ') => {
                self.open_dropdown(&item.key);
                true
            }
            _ => false,
        }
    }

    fn limits(&self, key: &str) -> Limits {
        let node = key.split('#').next().unwrap_or(key);
        let Some(field) = self.index.fields.get(node) else {
            return Limits::default();
        };
        let mask = match &field.widget {
            Widget::TextLine { mask, .. } => mask.clone(),
            _ => String::new(),
        };
        Limits {
            maxlength: usize::try_from(field.maxlength).unwrap_or(0),
            mask,
            multiline: matches!(field.widget, Widget::TextArea { .. }),
        }
    }

    /// Up and Down move through a list's rows; past the first or last row they move focus on, as Lynx does.
    fn list_key(&mut self, press: &KeyPress) -> bool {
        let Some(item) = self.focus().cloned().filter(|item| item.kind == FocusKind::List) else {
            return false;
        };
        let rows = self.list(&item.key).map_or(0, |list| list.rows.len());
        let at = self.row_at(&item.key);
        match (&press.key, press.shift) {
            (Key::Up, false) if at > 0 => {
                self.rows.insert(item.key, at - 1);
                true
            }
            (Key::Down, false) if at + 1 < rows => {
                self.rows.insert(item.key, at + 1);
                true
            }
            (Key::Right, true) => {
                *self.hscroll.entry(item.key).or_default() += 1;
                true
            }
            (Key::Left, true) => {
                let scroll = self.hscroll.entry(item.key).or_default();
                *scroll = scroll.saturating_sub(1);
                true
            }
            _ => false,
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Next => self.move_focus(1),
            Action::Prev => self.move_focus(-1),
            Action::Activate => self.activate(),
            Action::Back => self.back(),
            Action::Cancel => self.cancel(),
            Action::Help => self.overlays.push(Overlay::Help),
            Action::Info => self.overlays.push(Overlay::Info),
            Action::Prompt => self.prompt(),
            Action::Refresh => self.reload(),
            Action::PagePrev => self.turn_page(-1),
            Action::PageNext => self.turn_page(1),
            Action::Drawer => self.toggle_drawer(),
            Action::Submit => {
                let id = self.focus_id().to_string();
                if let Some(list) = self.find_open().map(str::to_string) {
                    self.submit_find(&list);
                } else {
                    self.submit_form_of(&id, None, false);
                }
            }
            Action::Select => self.toggle_row(),
            Action::SelectAll => self.select_all(),
            Action::Find => self.open_find(),
            Action::Sort => self.cycle_sort(),
            Action::NumberLinks => {
                self.numbering = !self.numbering;
                self.typed_number.clear();
            }
            Action::OpenBrowser => {
                let url = self.focused_url().unwrap_or_else(|| self.location_url());
                self.set_message(format!("Open in browser: {url}"), false);
                self.open = Some(url);
            }
            Action::Redraw => self.redraw = true,
            Action::Log => self.log_open = !self.log_open,
            Action::Quit => {
                if self.edits.any_dirty() {
                    self.overlays.push(Overlay::Confirm(Confirm::leave(Then::Quit)));
                } else {
                    self.quit = true;
                }
            }
            Action::Tab(n) => {
                let url = self
                    .index
                    .tabs
                    .get(usize::from(n).saturating_sub(1))
                    .map(|tab| tab.url.clone());
                if let Some(url) = url {
                    self.follow_url(&url);
                }
            }
        }
    }

    fn activate(&mut self) {
        let Some(item) = self.focus().cloned() else {
            return;
        };
        match item.kind {
            FocusKind::DropDown => self.open_dropdown(&item.key),
            FocusKind::Link => self.follow_link(&item.key),
            FocusKind::List => {
                let rows = self.list_page(&item.key);
                let link = rows.get(self.row_at(&item.key)).and_then(row_link).map(str::to_string);
                if let Some(url) = link {
                    self.follow_url(&url);
                }
            }
            FocusKind::Dialog => self.open_dialog(&item.key),
            FocusKind::Button => {
                if matches!(self.field_widget(&item.key), Some(Widget::Reset { .. })) {
                    self.reset_form(&item.key);
                } else {
                    self.submit_form_of(&item.key, None, false);
                }
            }
            FocusKind::Text | FocusKind::Check | FocusKind::Radio => {
                if let Scope::Find(list) = &item.scope {
                    let list = list.clone();
                    self.submit_find(&list);
                } else {
                    self.submit_form_of(&item.key, None, false);
                }
            }
        }
    }

    fn back(&mut self) {
        if self.edits.any_dirty() {
            self.overlays.push(Overlay::Confirm(Confirm::leave(Then::Back)));
            return;
        }
        self.go_back_now();
    }

    fn go_back_now(&mut self) {
        let Some(location) = self.nav.pop_history() else {
            self.set_message("This is the first screen.".to_string(), false);
            return;
        };
        self.overlays.clear();
        self.request_level(&location.path, location.parameters, Intent::Navigate { history: false });
    }

    fn cancel(&mut self) {
        if let Some(top) = self.overlays.last() {
            if matches!(top, Overlay::Dialog(_) | Overlay::Find(_)) {
                self.overlays.pop();
                self.rebuild_focus();
            }
            return;
        }
        if !self.message().is_empty() {
            self.clear_message();
            return;
        }
        if self.edits.any_dirty() {
            self.overlays.push(Overlay::Confirm(Confirm::leave(Then::Nothing)));
        }
    }

    fn prompt(&mut self) {
        let Some(item) = self.focus().cloned() else {
            return;
        };
        match self.field_widget(&item.key).cloned() {
            Some(Widget::DropDown { .. } | Widget::Check { .. }) => self.open_dropdown(&item.key),
            Some(Widget::DateTime { format, date_type, .. }) if date_type != "time" => {
                let date = forms::date_of(self.buffer(&item.key), &format, &date_type);
                self.overlays.push(Overlay::Picker(Picker { key: item.key, date }));
            }
            Some(Widget::DateFind { format, date_type, .. }) => {
                let date = forms::date_of(self.buffer(&item.key), &format, &date_type);
                self.overlays.push(Overlay::Picker(Picker { key: item.key, date }));
            }
            Some(Widget::TextFind { .. }) => {
                // F4 on a text find cycles its operator.
                let field = item.key.split('#').next().unwrap_or(&item.key).to_string();
                let key = format!("{field}#op");
                let ops = ["contains", "equals", "begins", "empty"];
                let at = ops
                    .iter()
                    .position(|op| *op == self.buffer(&key))
                    .map_or(0, |at| (at + 1) % ops.len());
                self.set_buffer(&key, ops[at].to_string());
                self.set_message(format!("Find by: {}", ops[at]), false);
            }
            _ => {}
        }
    }

    /// F9: show the drawer and focus it; again, hide it.
    fn toggle_drawer(&mut self) {
        if self.drawer_focus().is_some() {
            self.overlays.retain(|overlay| !matches!(overlay, Overlay::Drawer(_)));
            self.drawer = Some(false);
            return;
        }
        self.drawer = Some(true);
        if !self.shell.menu.is_empty() {
            self.overlays.push(Overlay::Drawer(0));
        }
    }

    fn open_dropdown(&mut self, key: &str) {
        let current = self.buffer(key).to_string();
        if let Some((url, params)) = self.index.option_query(key, &|k| self.edits.get(k).to_string()) {
            self.lanes.push(
                Purpose::Options(key.to_string()),
                LinkCall::Options {
                    url,
                    parameters: params,
                },
            );
        }
        let at = self
            .index
            .choices(key)
            .iter()
            .position(|choice| choice.value == current)
            .unwrap_or(0);
        self.overlays.push(Overlay::DropDown(DropDown {
            field: key.to_string(),
            at,
            filter: String::new(),
        }));
    }

    fn open_dialog(&mut self, id: &str) {
        if !self.dialogs_open().contains(&id) {
            self.overlays.push(Overlay::Dialog(id.to_string()));
        }
        self.rebuild_focus();
        self.focus_first_in_scope();
    }

    fn open_find(&mut self) {
        let list = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
            .or_else(|| self.first_list(true));
        let Some(list) = list else {
            return;
        };
        if self.list(&list).is_none_or(|list| list.header_fields.is_empty()) {
            self.set_message("This list has no find fields.".to_string(), false);
            return;
        }
        self.overlays.push(Overlay::Find(list));
        self.rebuild_focus();
        self.focus_first_in_scope();
    }

    fn toggle_row(&mut self) {
        let Some(list_id) = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
        else {
            return;
        };
        let Some(id) = self.selectable_row(&list_id, self.row_at(&list_id)) else {
            return;
        };
        let set = self.selected.entry(list_id).or_default();
        if !set.remove(&id) {
            set.insert(id);
        }
    }

    fn select_all(&mut self) {
        let Some(list_id) = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
        else {
            return;
        };
        let rows = self.list(&list_id).map_or(0, |list| list.rows.len());
        let ids: BTreeSet<String> = (0..rows).filter_map(|at| self.selectable_row(&list_id, at)).collect();
        let set = self.selected.entry(list_id).or_default();
        if set.len() == ids.len() {
            set.clear();
        } else {
            *set = ids;
        }
    }

    fn selectable_row(&self, list_id: &str, at: usize) -> Option<String> {
        let list = self.list(list_id)?;
        list.row_selection.as_ref()?;
        let row = list.rows.get(at)?;
        Some(if row.select_id.is_empty() {
            row.id.clone()
        } else {
            row.select_id.clone()
        })
    }

    // ---- navigation ----

    fn follow_link(&mut self, id: &str) {
        let Some(link) = self.index.links.get(id).cloned() else {
            return;
        };
        if link.disabled {
            self.set_message("This action is not available.".to_string(), true);
            return;
        }
        if !link.confirm.is_empty() {
            self.overlays.push(Overlay::Confirm(Confirm::ask(
                link.confirm.clone(),
                Then::FollowLink(link.id.clone()),
            )));
            return;
        }
        self.follow_link_now(id);
    }

    fn follow_link_now(&mut self, id: &str) {
        let Some(link) = self.index.links.get(id).cloned() else {
            return;
        };
        if link.posts() {
            let location = Location::of_url(&link.url);
            self.submitted = Some(Submitted {
                form: String::new(),
                keys: Vec::new(),
                submit_message: String::new(),
                background_hide_id: String::new(),
                then: None,
            });
            self.lanes.push(
                Purpose::Transition,
                LinkCall::Transition {
                    url: location.path,
                    method: "post".to_string(),
                    fields: location.parameters,
                },
            );
            return;
        }
        self.follow_url(&link.url);
    }

    /// Open a url: a screen in the client; an outside address is the browser's.
    fn follow_url(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        if url.contains("://") || url.starts_with("mailto:") {
            self.set_message(format!("Open in browser: {url}"), false);
            self.open = Some(url.to_string());
            return;
        }
        if self.edits.any_dirty() {
            self.overlays
                .push(Overlay::Confirm(Confirm::leave(Then::Navigate(url.to_string()))));
            return;
        }
        self.navigate(url, true);
    }

    /// Ask for a screen. With `history` the screen left is remembered for F3.
    pub fn navigate(&mut self, url: &str, history: bool) {
        let location = Location::of_url(url);
        self.request_level(&location.path, location.parameters, Intent::Navigate { history });
    }

    /// The current screen again, with its parameters.
    pub fn reload(&mut self) {
        let location = self.nav.location();
        self.request_level(&location.path, location.parameters, Intent::Reload);
    }

    fn reload_with(&mut self, parameters: BTreeMap<String, String>) {
        let path = self.path().to_string();
        self.request_level(&path, parameters, Intent::Reload);
    }

    fn request_level(&mut self, path: &str, parameters: BTreeMap<String, String>, intent: Intent) {
        self.last_location = Some(Location {
            path: path.to_string(),
            parameters: parameters.clone(),
        });
        let mut sent = parameters.clone();
        if let Some(rows) = self.page_rows {
            sent.entry("pageSize".to_string()).or_insert_with(|| rows.to_string());
        }
        if self.mobile {
            sent.insert("_uiType".to_string(), "mobile".to_string());
        }
        let call = LinkCall::Level {
            path: path.to_string(),
            parameters: sent,
        };
        if let Some(id) = self.lanes.push(Purpose::Level, call) {
            self.intents.insert(id, (intent, parameters));
        }
    }

    fn request_shell(&mut self, path: &str) {
        self.lanes
            .push(Purpose::Shell, LinkCall::Shell { path: path.to_string() });
    }

    fn level_reply(&mut self, id: u64, result: Result<ScreenModel, LinkError>) {
        let (intent, parameters) = self
            .intents
            .remove(&id)
            .unwrap_or((Intent::Reload, self.nav.leaf_parameters().clone()));
        let model = match result {
            Ok(model) => model,
            Err(err) => {
                self.set_message(err.to_string(), true);
                return;
            }
        };
        let navigate = matches!(intent, Intent::Navigate { .. });
        if let Intent::Navigate { history: true } = intent {
            if !self.model().screen.path.is_empty() {
                self.nav.push_history();
            }
        }
        let app_changed = app_root(&model.screen.path) != app_root(self.path());
        let missing = self.nav.set_leaf(model, parameters.clone());
        for path in missing {
            let call = LinkCall::Level {
                path: level_path(&path),
                parameters: parameters.clone(),
            };
            if let Some(id) = self.lanes.push(Purpose::Parent(path), call) {
                self.intents.insert(id, (Intent::Reload, parameters.clone()));
            }
        }
        if navigate {
            self.overlays.clear();
            self.rows.clear();
            self.selected.clear();
            self.hscroll.clear();
            self.cursors.clear();
            if app_changed && !self.path().is_empty() {
                let path = self.path().to_string();
                self.request_shell(&path);
            }
        } else {
            // A find submit closes its dialog; paging keeps a list's row in range.
            self.overlays
                .retain(|overlay| !matches!(overlay, Overlay::Find(_) | Overlay::DropDown(_)));
        }
        self.reindex(!navigate);
        let keep_message = !self.message.text.is_empty() && !self.message_expired() && !navigate;
        if !keep_message {
            self.clear_message();
        }
        self.show_model_messages();
        // A save's message, unless the screen brought its own.
        if let Some(text) = self.after_level.take() {
            if self.message().is_empty() {
                self.set_message(text, false);
            }
        }
    }

    /// The flash messages a transition left for this screen.
    fn show_model_messages(&mut self) {
        let messages = self.model().messages.clone();
        if !messages.errors.is_empty() {
            self.set_message(messages.errors.join("; "), true);
        } else if let Some(info) = messages.info.first() {
            self.set_message(info.clone(), false);
        }
    }

    fn apply_shell(&mut self, shell: ShellInfo) {
        let overrides: Vec<(&str, &str)> = shell.keys.iter().map(|(a, c)| (a.as_str(), c.as_str())).collect();
        let (keys, skips) = merge_keys(&KeyMap::baseline(), &overrides);
        self.keys = keys;
        self.key_skips = skips;
        self.shell = shell;
    }

    fn ask_login(&mut self) {
        if !self.overlays.iter().any(|overlay| matches!(overlay, Overlay::Login(_))) {
            self.overlays.push(Overlay::Login(Login {
                username: self.shell.user.clone(),
                ..Login::default()
            }));
        }
    }

    fn login_reply(&mut self, result: Result<(), LinkError>) {
        match result {
            Ok(()) => {
                self.overlays.retain(|overlay| !matches!(overlay, Overlay::Login(_)));
                // The screen asked for when the server wanted the login, and the shell for it. A save that was refused
                // is not sent again: its edits stay, and the user saves again.
                let location = self.last_location.clone().unwrap_or_else(|| self.nav.location());
                self.request_shell(if location.path.is_empty() { "/" } else { &location.path });
                self.request_level(&location.path, location.parameters, Intent::Navigate { history: false });
            }
            Err(err) => {
                if let Some(Overlay::Login(login)) = self
                    .overlays
                    .iter_mut()
                    .rev()
                    .find(|overlay| matches!(overlay, Overlay::Login(_)))
                {
                    login.error = err.to_string();
                    login.at = 1;
                }
            }
        }
    }

    // ---- forms ----

    fn submit_form_of(&mut self, key: &str, then: Option<Then>, confirmed: bool) {
        let node = key.split('#').next().unwrap_or(key);
        if !confirmed {
            if let Some(Widget::Submit { confirm, .. }) = self.field_widget(node) {
                if !confirm.is_empty() {
                    let ask = Confirm::ask(confirm.clone(), Then::Submit(node.to_string()));
                    self.overlays.push(Overlay::Confirm(ask));
                    return;
                }
            }
        }
        let Some(form) = self.index.form_of(node).map(|form| form.id.clone()) else {
            return;
        };
        self.submit_form(&form, then);
    }

    fn submit_form(&mut self, form_id: &str, then: Option<Then>) {
        if self.lanes.busy(&Purpose::Transition) {
            return;
        }
        let Some(form) = self.index.forms.get(form_id).cloned() else {
            return;
        };
        let values = |key: &str| self.edits.get(key).to_string();
        let marks = forms::check(&self.index, &form.fields, &values);
        if !marks.is_empty() {
            self.apply_marks(marks);
            return;
        }
        let dirty = |key: &str| self.edits.is_dirty(key);
        let fields = forms::posted(&self.index, &form.fields, &form.hidden, &values, &dirty);
        if form.transition_url.is_empty() {
            // A find form: the screen again, with the form's fields as its parameters, from the first page.
            let mut parameters = self.parameters().clone();
            for (name, value) in fields {
                if value.is_empty() {
                    parameters.remove(&name);
                } else {
                    parameters.insert(name, value);
                }
            }
            parameters.remove("pageIndex");
            self.reload_with(parameters);
            return;
        }
        let keys = form
            .fields
            .iter()
            .filter_map(|id| self.index.fields.get(id))
            .flat_map(forms::field_keys)
            .collect();
        self.submitted = Some(Submitted {
            form: form.id.clone(),
            keys,
            submit_message: form.submit_message.clone(),
            background_hide_id: form.background_hide_id.clone(),
            then,
        });
        self.lanes.push(
            Purpose::Transition,
            LinkCall::Transition {
                url: form.transition_url.clone(),
                method: form.method.clone(),
                fields,
            },
        );
    }

    /// The find dialog of a list: the list again with its header fields as parameters.
    fn submit_find(&mut self, list_id: &str) {
        let Some(list) = self.list(list_id) else {
            return;
        };
        let mut ids = Vec::new();
        crate::model::walk(&list.header_fields, &mut |node| {
            if let crate::model::Node::Field { id, .. } = node {
                ids.push(id.clone());
            }
        });
        let values = |key: &str| self.edits.get(key).to_string();
        let marks = forms::check(&self.index, &ids, &values);
        if !marks.is_empty() {
            self.apply_marks(marks);
            return;
        }
        let dirty = |key: &str| self.edits.is_dirty(key);
        let fields = forms::posted(&self.index, &ids, &[], &values, &dirty);
        let mut parameters = self.parameters().clone();
        for (name, value) in fields {
            if value.is_empty() || value == "N" && name.ends_with("_not") {
                parameters.remove(&name);
            } else {
                parameters.insert(name, value);
            }
        }
        parameters.remove("pageIndex");
        self.reload_with(parameters);
    }

    fn transition_reply(&mut self, result: Result<TransitionOutcome, LinkError>) {
        let submitted = self.submitted.take();
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(err) => {
                self.set_message(err.to_string(), true);
                return;
            }
        };
        if !outcome.messages.validation_errors.is_empty() {
            let marks: Vec<(String, String)> = outcome
                .messages
                .validation_errors
                .iter()
                .map(|err| {
                    let id = submitted
                        .as_ref()
                        .and_then(|done| self.index.forms.get(&done.form))
                        .and_then(|form| {
                            form.fields
                                .iter()
                                .find(|id| self.index.fields.get(*id).is_some_and(|field| field.name == err.field))
                        })
                        .cloned()
                        .unwrap_or_else(|| err.field.clone());
                    (id, err.message.clone())
                })
                .collect();
            // An error for a field the form does not show is on the message line, not lost.
            let unshown: Vec<String> = marks
                .iter()
                .filter(|(id, _)| !self.index.fields.contains_key(id))
                .map(|(id, text)| format!("{id}: {text}"))
                .collect();
            // The line shows the first mark, unless the server sent a message or named a field this form does not show.
            self.apply_marks(marks);
            let mut line = outcome.messages.errors.clone();
            line.extend(unshown);
            if !line.is_empty() {
                self.set_message(line.join("; "), true);
            }
            return;
        }
        if !outcome.messages.errors.is_empty() {
            // A service error with no field to mark. The edits stay for another try.
            self.set_message(outcome.messages.errors.join("; "), true);
            return;
        }
        let text = outcome
            .messages
            .info
            .first()
            .cloned()
            .or_else(|| submitted.as_ref().map(|done| done.submit_message.clone()))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "Saved".to_string());
        if let Some(done) = &submitted {
            self.edits.drop_keys(&done.keys);
            self.edits.clear_marks();
            if !done.background_hide_id.is_empty() {
                let hide = done.background_hide_id.clone();
                self.overlays
                    .retain(|overlay| overlay != &Overlay::Dialog(hide.clone()));
            }
        }
        let then = submitted.as_ref().and_then(|done| done.then.clone());
        match then {
            Some(Then::Navigate(url)) => self.navigate(&url, true),
            Some(Then::Quit) => self.quit = true,
            Some(Then::Back) => self.go_back_now(),
            _ if !outcome.screen_url.is_empty() => {
                let mut location = Location::of_url(&outcome.screen_url);
                location.parameters.extend(outcome.parameters.clone());
                self.request_level(&location.path, location.parameters, Intent::Navigate { history: true });
            }
            // Staying here: the screen again, so it shows what was saved, as the web client does. A form's
            // background-reload-id names a container; with each level its own request, that is this screen.
            _ => self.reload(),
        }
        self.set_message(text.clone(), false);
        self.after_level = Some(text);
    }

    fn reset_form(&mut self, key: &str) {
        let Some(form) = self.index.form_of(key).cloned() else {
            return;
        };
        let keys: Vec<String> = form
            .fields
            .iter()
            .filter_map(|id| self.index.fields.get(id))
            .flat_map(forms::field_keys)
            .collect();
        self.edits.drop_keys(&keys);
        for key in &keys {
            self.cursors.remove(key);
        }
    }

    fn apply_marks(&mut self, marks: Vec<(String, String)>) {
        let first = marks.first().map(|(id, _)| id.clone());
        self.edits.set_marks(marks);
        if let Some(id) = first {
            if let Some(at) = self.focus.iter().position(|item| item.key == id || item.node == id) {
                self.focus_at = at;
            }
            if let Some(text) = self.edits.mark(&id) {
                let text = text.to_string();
                self.set_message(text, true);
            }
        }
    }

    fn first_dirty_form(&self) -> Option<String> {
        let key = self.edits.dirty_keys().next()?;
        let node = key.split('#').next().unwrap_or(key);
        self.index.form_of(node).map(|form| form.id.clone())
    }

    /// After a field changed: drop-downs that depend on it fetch their options, and fields shown by its value appear.
    fn after_edit(&mut self, key: &str) {
        let node = key.split('#').next().unwrap_or(key).to_string();
        let Some(name) = self.index.fields.get(&node).map(|field| field.name.clone()) else {
            return;
        };
        for target in self.index.dependents(&name) {
            if let Some((url, params)) = self.index.option_query(&target, &|k| self.edits.get(k).to_string()) {
                self.lanes.push(
                    Purpose::Options(target),
                    LinkCall::Options {
                        url,
                        parameters: params,
                    },
                );
            }
        }
        if self.index.watched_by(&name) {
            self.rebuild_focus();
        }
    }

    fn apply_choices(&mut self, field: &str, choices: Vec<Choice>) {
        let value = self.buffer(field).to_string();
        let multiple = matches!(self.field_widget(field), Some(Widget::DropDown { multiple: true, .. }));
        let combo = matches!(self.field_widget(field), Some(Widget::DropDown { combo: true, .. }));
        let known = |value: &str| choices.iter().any(|choice| choice.value == value);
        if !value.is_empty() && !combo && !multiple && !known(&value) {
            self.edits.set(field, String::new());
        }
        self.index.set_choices(field, choices);
        if let Some(Overlay::DropDown(drop)) = self.overlays.last_mut() {
            if drop.field == field {
                drop.at = drop.at.min(self.index.choices(field).len().saturating_sub(1));
            }
        }
    }

    // ---- lists ----

    fn first_list(&self, with_find: bool) -> Option<String> {
        self.focus
            .iter()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
            .find(|id| !with_find || self.list(id).is_some_and(|list| !list.header_fields.is_empty()))
    }

    /// The focused list, else the first one on the screen.
    fn target_list(&self) -> Option<String> {
        self.focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
            .or_else(|| self.first_list(false))
    }

    fn has_paged_list(&self) -> bool {
        self.index.lists.values().any(|list| page_count_of(list) > 1)
    }

    fn turn_page(&mut self, delta: isize) {
        let Some(list_id) = self.target_list() else {
            return;
        };
        let count = self.page_count(&list_id) as isize;
        let current = self.page_index(&list_id) as isize;
        let next = (current + delta).clamp(0, count - 1);
        if next == current {
            return;
        }
        let mut params = self.parameters().clone();
        params.insert("pageIndex".to_string(), next.to_string());
        self.rows.insert(list_id, 0);
        self.reload_with(params);
    }

    fn cycle_sort(&mut self) {
        let Some(list_id) = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
        else {
            return;
        };
        let fields: Vec<String> = self
            .list(&list_id)
            .map(|list| {
                list.columns
                    .iter()
                    .filter(|column| !column.order_by.is_empty())
                    .map(|column| column.order_by.clone())
                    .collect()
            })
            .unwrap_or_default();
        if fields.is_empty() {
            self.set_message("This list does not sort.".to_string(), false);
            return;
        }
        let next = next_order(&fields, self.order_by());
        let mut params = self.parameters().clone();
        if next.is_empty() {
            params.remove("orderByField");
        } else {
            params.insert("orderByField".to_string(), next);
        }
        params.remove("pageIndex");
        // The rows come back in another order, so the current row starts at the top again.
        self.rows.insert(list_id, 0);
        self.reload_with(params);
    }

    // ---- focus ----

    fn move_focus(&mut self, delta: isize) {
        if self.focus.is_empty() {
            return;
        }
        let len = self.focus.len() as isize;
        self.focus_at = (self.focus_at as isize + delta).rem_euclid(len) as usize;
    }

    fn focus_first_in_scope(&mut self) {
        self.focus_at = 0;
    }

    fn scope(&self) -> Scope {
        self.overlays
            .iter()
            .rev()
            .find_map(|overlay| match overlay {
                Overlay::Dialog(id) => Some(Scope::Dialog(id.clone())),
                Overlay::Find(id) => Some(Scope::Find(id.clone())),
                _ => None,
            })
            .unwrap_or(Scope::Main)
    }

    /// The focus order of the current scope: what a dialog or the find dialog holds while it is open, else the
    /// screen, without fields their `visible-when` hides.
    fn rebuild_focus(&mut self) {
        let previous = self.focus_id().to_string();
        let scope = self.scope();
        let values = |key: &str| self.edits.get(key).to_string();
        self.focus = self
            .index
            .order
            .iter()
            .filter(|item| item.scope == scope && self.index.visible(&item.node, &values))
            .cloned()
            .collect();
        let model_focus = self.model().focus.clone();
        self.focus_at = self
            .focus
            .iter()
            .position(|item| item.key == previous)
            .or_else(|| {
                self.focus
                    .iter()
                    .position(|item| item.key == model_focus && !model_focus.is_empty())
            })
            .unwrap_or(0);
    }

    /// The model changed: index it, take the values it sends, and rebuild the focus. With `keep`, edits that are still
    /// dirty stay.
    fn reindex(&mut self, keep: bool) {
        self.index = Index::of(self.nav.models());
        self.edits.load(self.index.initial_values(), keep);
        let lists: Vec<(String, usize)> = self
            .index
            .lists
            .iter()
            .map(|(id, list)| (id.clone(), list.rows.len()))
            .collect();
        for (id, rows) in lists {
            let at = self.rows.entry(id).or_insert(0);
            *at = (*at).min(rows.saturating_sub(1));
        }
        self.rebuild_focus();
    }

    // ---- messages ----

    fn set_message(&mut self, text: String, error: bool) {
        self.message = Message {
            text,
            error,
            until: (!error).then(|| Instant::now() + Duration::from_secs(MESSAGE_SECONDS)),
        };
    }

    fn clear_message(&mut self) {
        self.message = Message::default();
    }

    fn focused_url(&self) -> Option<String> {
        let item = self.focus()?;
        match item.kind {
            FocusKind::Link => self.index.links.get(&item.key).map(|link| link.url.clone()),
            FocusKind::List => self
                .list_page(&item.key)
                .get(self.row_at(&item.key))
                .and_then(row_link)
                .map(str::to_string),
            _ => None,
        }
    }

    /// The screen's address with its parameters.
    pub fn location_url(&self) -> String {
        let location = self.nav.location();
        if location.parameters.is_empty() {
            return location.path;
        }
        let query: Vec<String> = location
            .parameters
            .iter()
            .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
            .collect();
        format!("{}?{}", location.path, query.join("&"))
    }
}

/// The drawer's rows as `(title, url, depth)`: each app menu item and the items under it.
pub fn drawer_rows(menu: &[MenuItem]) -> Vec<(String, String, usize)> {
    let mut rows = Vec::new();
    for item in menu {
        rows.push((item.title.clone(), item.url.clone(), 0));
        for child in &item.children {
            rows.push((child.title.clone(), child.url.clone(), 1));
        }
    }
    rows
}

fn filtered(choices: &[Choice], filter: &str) -> Vec<Choice> {
    if filter.is_empty() {
        return choices.to_vec();
    }
    let needle = filter.to_lowercase();
    choices
        .iter()
        .filter(|choice| choice.label.to_lowercase().contains(&needle) || choice.value.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

fn app_root(path: &str) -> String {
    path.split('/')
        .filter(|name| !name.is_empty())
        .take(2)
        .collect::<Vec<_>>()
        .join("/")
}

fn page_size_of(list: &FormList) -> usize {
    if list.paging.size == 0 {
        PAGE_ROWS
    } else {
        usize::try_from(list.paging.size).unwrap_or(PAGE_ROWS).max(1)
    }
}

pub fn page_count_of(list: &FormList) -> usize {
    if let Some(max) = list.paging.max_index {
        return usize::try_from(max).unwrap_or(0) + 1;
    }
    let size = page_size_of(list);
    let count = if list.paging.count == 0 {
        list.rows.len()
    } else {
        usize::try_from(list.paging.count).unwrap_or(list.rows.len())
    };
    count.div_ceil(size).max(1)
}

/// Ascending then descending on each sortable column in turn, then unsorted.
fn next_order(fields: &[String], current: &str) -> String {
    let (descending, name) = match current.strip_prefix('-') {
        Some(name) => (true, name),
        None => (false, current),
    };
    let bare = |field: &str| field.trim_start_matches('^').to_string();
    match fields.iter().position(|field| bare(field) == bare(name)) {
        None => fields[0].clone(),
        Some(at) if !descending => format!("-{}", fields[at]),
        Some(at) => fields.get(at + 1).cloned().unwrap_or_default(),
    }
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(char::from(byte)),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
