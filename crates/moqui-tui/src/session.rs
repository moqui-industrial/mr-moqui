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

//! Focus, buffers, and key dispatch. Paging and sort call [`ScreenLink::level`].
//! The `ui` feature mirrors the focused text field into a Revue `Input`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::keys::{Action, Key, KeyMap, KeyPress};
use crate::layout::PAGE_ROWS;
use crate::link::{row_link, ScreenLink, TransitionOutcome};
use crate::model::{walk, Choice, ListRow, Node, ScreenModel, Widget};
use crate::nav::Nav;
use crate::state::{Confirm, ConfirmKind, OptionLane, OptionRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusKind {
    Text,
    DropDown,
    Submit,
    Link,
    List,
    Dialog,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusItem {
    pub id: String,
    pub kind: FocusKind,
    pub url: Option<String>,
}

/// One drawer row. Children are the second level only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub title: String,
    pub url: String,
    pub children: Vec<MenuItem>,
}

#[derive(Clone, Debug)]
pub struct Session {
    nav: Nav,
    focus: Vec<FocusItem>,
    focus_at: usize,
    buffers: BTreeMap<String, String>,
    originals: BTreeMap<String, String>,
    dirty: BTreeSet<String>,
    pages: BTreeMap<String, usize>,
    row_at: BTreeMap<String, usize>,
    dropdown_open: bool,
    dropdown_at: usize,
    dialogs: Vec<String>,
    find_open: bool,
    message: String,
    marks: BTreeMap<String, String>,
    confirm: Option<Confirm>,
    lane: OptionLane,
    keys: KeyMap,
    busy: bool,
    menu: Vec<MenuItem>,
    message_sticky: bool,
    message_until: Option<Instant>,
}

impl Session {
    pub fn new(model: ScreenModel) -> Self {
        let mut session = Self {
            nav: Nav::new(model),
            focus: Vec::new(),
            focus_at: 0,
            buffers: BTreeMap::new(),
            originals: BTreeMap::new(),
            dirty: BTreeSet::new(),
            pages: BTreeMap::new(),
            row_at: BTreeMap::new(),
            dropdown_open: false,
            dropdown_at: 0,
            dialogs: Vec::new(),
            find_open: false,
            message: String::new(),
            marks: BTreeMap::new(),
            confirm: None,
            lane: OptionLane::default(),
            keys: KeyMap::baseline(),
            busy: false,
            menu: Vec::new(),
            message_sticky: false,
            message_until: None,
        };
        session.rebuild(false);
        session
    }

    pub fn model(&self) -> &ScreenModel {
        self.nav.leaf()
    }

    pub fn load(&mut self, model: ScreenModel) {
        let params = self.nav.leaf_parameters().clone();
        self.nav.replace_leaf(model, params);
        self.rebuild(false);
    }

    pub fn path(&self) -> &str {
        &self.model().screen.path
    }

    /// The current `orderByField`, empty when the list is unsorted.
    pub fn order_by(&self) -> &str {
        self.nav
            .leaf_parameters()
            .get("orderByField")
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn paths(&self) -> Vec<&str> {
        self.nav.paths()
    }

    pub fn focus_id(&self) -> &str {
        self.focus.get(self.focus_at).map(|item| item.id.as_str()).unwrap_or("")
    }

    pub fn focus_kind(&self) -> Option<FocusKind> {
        self.focus.get(self.focus_at).map(|item| item.kind)
    }

    pub fn focused_is_text(&self) -> bool {
        self.focus_kind() == Some(FocusKind::Text)
    }

    pub fn buffer(&self, id: &str) -> &str {
        self.buffers.get(id).map(String::as_str).unwrap_or("")
    }

    pub fn set_buffer(&mut self, id: &str, value: String) {
        self.store_buffer(id, value);
    }

    /// A field value changed. Dependent drop-downs refetch, one request in flight per field.
    pub fn changed(&mut self, id: &str, link: &dyn ScreenLink) {
        self.refresh_dependents(id, link);
    }

    pub fn is_dirty(&self, id: &str) -> bool {
        self.dirty.contains(id)
    }

    /// The message line. A success clears itself after 3 seconds. An error stays.
    pub fn message(&self) -> &str {
        if !self.message_sticky && self.message_until.is_some_and(|until| Instant::now() >= until) {
            ""
        } else {
            &self.message
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// The host sets this while a level, transition, or options call is in flight.
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub fn dialogs_open(&self) -> &[String] {
        &self.dialogs
    }

    pub fn menu(&self) -> &[MenuItem] {
        &self.menu
    }

    pub fn set_menu(&mut self, menu: Vec<MenuItem>) {
        self.menu = menu;
    }

    pub fn mark(&self, id: &str) -> Option<&str> {
        self.marks.get(id).map(String::as_str)
    }

    pub fn confirm_choice(&self) -> Option<&str> {
        self.confirm.as_ref().map(Confirm::choice)
    }

    pub fn find_open(&self) -> bool {
        self.find_open
    }

    pub fn page_index(&self, list_id: &str) -> usize {
        self.pages.get(list_id).copied().unwrap_or(0)
    }

    pub fn page_count(&self, list_id: &str) -> usize {
        form_list(self.model(), list_id).map_or(1, page_count_of)
    }

    pub fn dropdown_open(&self) -> bool {
        self.dropdown_open
    }

    pub fn close_dropdown(&mut self) {
        self.dropdown_open = false;
    }

    pub fn dropdown_at(&self) -> usize {
        self.dropdown_at
    }

    pub fn focused_choices(&self) -> Vec<Choice> {
        self.focused_widget()
            .and_then(|widget| match widget {
                Widget::DropDown { options, .. } => Some(options.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Handle one key. Returns true when the local loop should quit.
    pub fn on_key(&mut self, press: &KeyPress, link: &dyn ScreenLink) -> bool {
        if self.confirm.is_some() {
            self.confirm_key(press, link);
            return false;
        }
        if self.dropdown_open {
            return self.dropdown_key(press, link);
        }
        if self.consume_text_key(press, link) {
            return false;
        }
        if self.consume_row_key(press) {
            return false;
        }
        self.dispatch(press, link)
    }

    pub fn field_widget(&self, id: &str) -> Option<&Widget> {
        let mut found = None;
        walk(&self.model().c, &mut |node| {
            if let Node::Field { id: field_id, w, .. } = node {
                if field_id == id {
                    found = Some(w);
                }
            }
        });
        found
    }

    pub fn list_page(&self, list_id: &str) -> Vec<ListRow> {
        let Some(list) = form_list(self.model(), list_id) else {
            return Vec::new();
        };
        let size = page_size_of(list);
        if list.rows.len() <= size {
            return list.rows.clone();
        }
        let start = self.page_index(list_id).saturating_mul(size);
        list.rows.iter().skip(start).take(size).cloned().collect()
    }

    fn consume_text_key(&mut self, press: &KeyPress, link: &dyn ScreenLink) -> bool {
        if !self.focused_is_text() || press.ctrl || press.alt {
            return false;
        }
        match press.key {
            Key::Char(ch) => {
                self.insert_char(ch, link);
                true
            }
            Key::Backspace => {
                self.backspace(link);
                true
            }
            Key::Left | Key::Right => true,
            _ => false,
        }
    }

    fn consume_row_key(&mut self, press: &KeyPress) -> bool {
        if self.focus_kind() != Some(FocusKind::List) {
            return false;
        }
        match press.key {
            Key::Up => {
                self.move_row(-1);
                true
            }
            Key::Down => {
                self.move_row(1);
                true
            }
            _ => false,
        }
    }

    fn dispatch(&mut self, press: &KeyPress, link: &dyn ScreenLink) -> bool {
        match self.keys.action(press) {
            Some(Action::Quit) => true,
            Some(Action::Next) => {
                self.move_focus(1);
                false
            }
            Some(Action::Prev) => {
                self.move_focus(-1);
                false
            }
            Some(Action::Activate) => {
                self.activate(link);
                false
            }
            Some(Action::Back) => {
                self.go_back();
                false
            }
            Some(Action::Cancel) => {
                self.cancel();
                false
            }
            Some(Action::Prompt) => {
                self.open_dropdown(link);
                false
            }
            Some(Action::Reload) => {
                self.reload(link);
                false
            }
            Some(Action::PagePrev) => {
                self.turn_page(-1, link);
                false
            }
            Some(Action::PageNext) => {
                self.turn_page(1, link);
                false
            }
            Some(Action::Sort) => {
                self.cycle_sort(link);
                false
            }
            Some(Action::Find) => {
                self.open_find();
                false
            }
            None => false,
        }
    }

    fn confirm_key(&mut self, press: &KeyPress, link: &dyn ScreenLink) {
        let Some(confirm) = self.confirm.as_mut() else {
            return;
        };
        match press.key {
            Key::Left | Key::Up => confirm.move_by(-1),
            Key::Right | Key::Down | Key::Tab => confirm.move_by(1),
            Key::Enter => {
                let choice = confirm.choice().to_string();
                let kind = confirm.kind.clone();
                self.confirm = None;
                self.finish_confirm(&choice, kind, link);
            }
            Key::Escape | Key::F(12) => self.confirm = None,
            _ => {}
        }
    }

    fn finish_confirm(&mut self, choice: &str, kind: ConfirmKind, link: &dyn ScreenLink) {
        let ConfirmKind::Leave { url } = kind;
        match choice {
            "Discard" => {
                self.reset_edits();
                if !url.is_empty() {
                    self.navigate(&url, link);
                }
            }
            "Save" if self.submit_dirty(link) && !url.is_empty() => self.navigate(&url, link),
            "Save" => {}
            _ => {}
        }
    }

    fn dropdown_key(&mut self, press: &KeyPress, link: &dyn ScreenLink) -> bool {
        let count = self.focused_choices().len();
        match press.key {
            Key::Up => {
                if count > 0 {
                    self.dropdown_at = self.dropdown_at.saturating_sub(1);
                }
            }
            Key::Down => {
                if count > 0 && self.dropdown_at + 1 < count {
                    self.dropdown_at += 1;
                }
            }
            Key::Enter => self.commit_dropdown(link),
            Key::Escape | Key::F(4) => self.dropdown_open = false,
            _ => {}
        }
        false
    }

    fn open_dropdown(&mut self, link: &dyn ScreenLink) {
        if self.focus_kind() != Some(FocusKind::DropDown) {
            return;
        }
        let id = self.focus_id().to_string();
        self.fetch_options(id, link);
        let current = self.buffer(self.focus_id()).to_string();
        let at = self
            .focused_choices()
            .iter()
            .position(|choice| choice.value == current)
            .unwrap_or(0);
        self.dropdown_at = at;
        self.dropdown_open = true;
    }

    fn commit_dropdown(&mut self, link: &dyn ScreenLink) {
        let id = self.focus_id().to_string();
        if let Some(choice) = self.focused_choices().get(self.dropdown_at).cloned() {
            self.store_buffer(&id, choice.value);
            self.refresh_dependents(&id, link);
        }
        self.dropdown_open = false;
    }

    fn activate(&mut self, link: &dyn ScreenLink) {
        match self.focus_kind() {
            Some(FocusKind::DropDown) => self.open_dropdown(link),
            Some(FocusKind::Link) => {
                if let Some(url) = self.focus.get(self.focus_at).and_then(|item| item.url.clone()) {
                    self.navigate(&url, link);
                }
            }
            Some(FocusKind::List) => self.activate_row(link),
            Some(FocusKind::Dialog) => {
                let id = self.focus_id().to_string();
                self.open_dialog(&id);
            }
            Some(FocusKind::Submit | FocusKind::Text) => {
                self.submit_focused(link);
            }
            None => {}
        }
    }

    fn activate_row(&mut self, link: &dyn ScreenLink) {
        let id = self.focus_id().to_string();
        let row_index = self.row_at.get(&id).copied().unwrap_or(0);
        let rows = self.list_page(&id);
        if let Some(url) = rows.get(row_index).and_then(row_link) {
            let url = url.to_string();
            self.navigate(&url, link);
        }
    }

    fn navigate(&mut self, url: &str, link: &dyn ScreenLink) {
        if self.busy || url.is_empty() {
            return;
        }
        if !self.dirty.is_empty() {
            self.confirm = Some(Confirm::leave(url.to_string()));
            return;
        }
        match link.level(url, &BTreeMap::new()) {
            Ok(model) => {
                self.nav.follow(model);
                self.clear_message();
                self.dialogs.clear();
                self.find_open = false;
                self.rebuild(false);
            }
            Err(err) => self.set_message(err.to_string(), true),
        }
    }

    fn go_back(&mut self) {
        if self.nav.back() {
            self.clear_message();
            self.dialogs.clear();
            self.find_open = false;
            self.rebuild(false);
        }
    }

    fn cancel(&mut self) {
        if self.find_open {
            self.find_open = false;
            self.rebuild(true);
            return;
        }
        if self.dialogs.pop().is_some() {
            self.rebuild(true);
            return;
        }
        if !self.message.is_empty() {
            self.clear_message();
            return;
        }
        if !self.dirty.is_empty() {
            self.confirm = Some(Confirm::leave(String::new()));
        }
    }

    fn reload(&mut self, link: &dyn ScreenLink) {
        let params = self.nav.leaf_parameters().clone();
        self.reload_with(params, link);
    }

    fn turn_page(&mut self, delta: isize, link: &dyn ScreenLink) {
        let Some(list_id) = self.first_list() else {
            return;
        };
        let count = self.page_count(&list_id) as isize;
        let current = self.page_index(&list_id) as isize;
        let next = (current + delta).clamp(0, count - 1);
        if next == current {
            return;
        }
        let mut params = self.nav.leaf_parameters().clone();
        params.insert("pageIndex".to_string(), next.to_string());
        self.reload_with(params, link);
    }

    fn cycle_sort(&mut self, link: &dyn ScreenLink) {
        if self.focus_kind() != Some(FocusKind::List) {
            return;
        }
        let id = self.focus_id().to_string();
        let fields = sort_fields(self.model(), &id);
        if fields.is_empty() {
            return;
        }
        let next = next_order(&fields, self.order_by());
        let mut params = self.nav.leaf_parameters().clone();
        if next.is_empty() {
            params.remove("orderByField");
        } else {
            params.insert("orderByField".to_string(), next);
        }
        self.reload_with(params, link);
    }

    fn reload_with(&mut self, params: BTreeMap<String, String>, link: &dyn ScreenLink) {
        if self.busy {
            return;
        }
        let path = self.path().to_string();
        match link.level(&path, &params) {
            Ok(model) => {
                self.nav.replace_leaf(model, params);
                self.rebuild(true);
            }
            Err(err) => self.set_message(err.to_string(), true),
        }
    }

    fn open_find(&mut self) {
        if self.focus_kind() != Some(FocusKind::List) && self.first_list().is_none() {
            return;
        }
        self.find_open = true;
        self.rebuild(true);
    }

    fn open_dialog(&mut self, id: &str) {
        if !self.dialogs.iter().any(|open| open == id) {
            self.dialogs.push(id.to_string());
        }
        self.rebuild(true);
    }

    fn submit_focused(&mut self, link: &dyn ScreenLink) -> bool {
        let id = self.focus_id().to_string();
        self.submit_form(&id, link)
    }

    /// Posts the form holding the first edited field. A leave can start from a link or a list, so
    /// the focused item does not name the form.
    fn submit_dirty(&mut self, link: &dyn ScreenLink) -> bool {
        let Some(id) = self.dirty.iter().next().cloned() else {
            return true;
        };
        if find_form(self.model(), &id).is_none() {
            self.set_message("These edits are not in a form that can be saved.".to_string(), true);
            return false;
        }
        self.submit_form(&id, link)
    }

    fn submit_form(&mut self, id: &str, link: &dyn ScreenLink) -> bool {
        if self.busy {
            return false;
        }
        let Some(form) = find_form(self.model(), id) else {
            return false;
        };
        let marks = client_marks(&form, &self.buffers);
        if !marks.is_empty() {
            self.apply_marks(marks);
            return false;
        }
        if form.transition_url.is_empty() {
            return false;
        }
        let fields = self.posted_fields(&form);
        match link.transition(&form.transition_url, &form.method, &fields) {
            Ok(outcome) => self.apply_transition(&form, &outcome, link),
            Err(err) => {
                self.set_message(err.to_string(), true);
                false
            }
        }
    }

    fn apply_transition(&mut self, form: &FormBits, outcome: &TransitionOutcome, link: &dyn ScreenLink) -> bool {
        if outcome.messages.validation_errors.is_empty() && !outcome.messages.errors.is_empty() {
            // A service error with no field to mark. The edits stay for another try.
            self.set_message(outcome.messages.errors.join("; "), true);
            return false;
        }
        if !outcome.messages.validation_errors.is_empty() {
            let marks = outcome
                .messages
                .validation_errors
                .iter()
                .map(|err| {
                    let id = form
                        .fields
                        .iter()
                        .find(|field| field.name == err.field)
                        .map(|field| field.id.clone())
                        .unwrap_or_else(|| err.field.clone());
                    (id, err.message.clone())
                })
                .collect();
            self.apply_marks(marks);
            self.set_message(outcome.messages.errors.first().cloned().unwrap_or_default(), true);
            return false;
        }
        self.drop_form_edits(form);
        self.marks.clear();
        let text = outcome
            .messages
            .info
            .first()
            .cloned()
            .unwrap_or_else(|| form.submit_message.clone());
        if !form.background_hide_id.is_empty() {
            let hide = form.background_hide_id.clone();
            self.dialogs.retain(|id| id != &hide);
            self.rebuild(true);
        }
        if outcome.screen_url.is_empty() {
            // Staying here: reload so the form shows what was saved, as the web client does.
            self.reload(link);
        } else {
            let url = outcome.screen_url.clone();
            self.navigate(&url, link);
        }
        // A redirect clears the line. The text from this post is what the line shows afterwards.
        if self.message().is_empty() && !text.is_empty() {
            self.set_message(text, false);
        }
        true
    }

    fn apply_marks(&mut self, marks: Vec<(String, String)>) {
        self.marks.clear();
        let first = marks.first().map(|(id, _)| id.clone());
        for (id, text) in marks {
            self.marks.insert(id, text);
        }
        if let Some(id) = first {
            if let Some(index) = self.focus.iter().position(|item| item.id == id) {
                self.focus_at = index;
            }
        }
    }

    fn posted_fields(&self, form: &FormBits) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::new();
        for (name, value) in &form.hidden {
            fields.insert(name.clone(), value.clone());
        }
        for field in &form.fields {
            if !field.posts {
                continue;
            }
            let value = self.buffer(&field.id).to_string();
            // An empty optional value is left out unless the user cleared it, so an untouched empty
            // field does not write "" into a foreign key.
            if value.is_empty() && !field.required && !self.dirty.contains(&field.id) {
                continue;
            }
            fields.insert(field.name.clone(), value);
        }
        fields
    }

    /// Clears the posted fields from `dirty`, so the next rebuild takes their values from the model.
    fn drop_form_edits(&mut self, form: &FormBits) {
        for field in &form.fields {
            self.dirty.remove(&field.id);
        }
    }

    fn set_message(&mut self, text: String, sticky: bool) {
        self.message = text;
        self.message_sticky = sticky;
        self.message_until = if sticky {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(3))
        };
    }

    fn clear_message(&mut self) {
        self.message.clear();
        self.message_sticky = false;
        self.message_until = None;
    }

    fn reset_edits(&mut self) {
        self.buffers.clone_from(&self.originals);
        self.dirty.clear();
        self.marks.clear();
    }

    fn move_focus(&mut self, delta: isize) {
        if self.focus.is_empty() {
            return;
        }
        let len = self.focus.len() as isize;
        self.focus_at = (self.focus_at as isize + delta).rem_euclid(len) as usize;
        self.dropdown_open = false;
    }

    fn move_row(&mut self, delta: isize) {
        let id = self.focus_id().to_string();
        let len = self.list_page(&id).len() as isize;
        if len == 0 {
            return;
        }
        let current = self.row_at.get(&id).copied().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len - 1) as usize;
        self.row_at.insert(id, next);
    }

    fn insert_char(&mut self, ch: char, link: &dyn ScreenLink) {
        let id = self.focus_id().to_string();
        let mut text = self.buffer(&id).to_string();
        text.push(ch);
        self.store_buffer(&id, text);
        self.refresh_dependents(&id, link);
    }

    fn backspace(&mut self, link: &dyn ScreenLink) {
        let id = self.focus_id().to_string();
        let mut text = self.buffer(&id).to_string();
        text.pop();
        self.store_buffer(&id, text);
        self.refresh_dependents(&id, link);
    }

    fn store_buffer(&mut self, id: &str, value: String) {
        if self.originals.get(id).is_some_and(|original| original == &value) {
            self.dirty.remove(id);
        } else if self.originals.contains_key(id) || self.buffers.contains_key(id) {
            self.dirty.insert(id.to_string());
        }
        self.buffers.insert(id.to_string(), value);
        self.marks.remove(id);
    }

    fn refresh_dependents(&mut self, edited_id: &str, link: &dyn ScreenLink) {
        let Some(name) = field_name(self.model(), edited_id).map(str::to_string) else {
            return;
        };
        let targets = dependent_dropdowns(self.model(), &name);
        for target in targets {
            self.fetch_options(target, link);
        }
    }

    fn fetch_options(&mut self, field_id: String, link: &dyn ScreenLink) {
        let Some((url, params)) = option_query(self.model(), &field_id, &self.buffers) else {
            return;
        };
        if let Some(request) = self.lane.push(field_id, url, params) {
            self.drive_options(request, link);
        }
    }

    fn drive_options(&mut self, mut request: OptionRequest, link: &dyn ScreenLink) {
        loop {
            let choices = link.options(&request.url, &request.parameters).unwrap_or_default();
            let (apply, next) = self.lane.finish(request.generation);
            if apply {
                self.apply_choices(&request.field_id, &choices);
            }
            match next {
                Some(next) => request = next,
                None => break,
            }
        }
    }

    fn apply_choices(&mut self, field_id: &str, choices: &[Choice]) {
        let value = self.buffer(field_id).to_string();
        if !value.is_empty() && !choices.iter().any(|choice| choice.value == value) {
            self.store_buffer(field_id, String::new());
        }
        let model = self.nav.leaf().clone();
        let mut model = model;
        set_dropdown_options(&mut model.c, field_id, choices);
        let params = self.nav.leaf_parameters().clone();
        self.nav.replace_leaf(model, params);
    }

    fn rebuild(&mut self, keep: bool) {
        let previous = self.focus_id().to_string();
        let model_focus = self.model().focus.clone();
        let old_buffers = self.buffers.clone();
        let old_dirty = self.dirty.clone();
        self.focus = focus_items(self.model(), &self.dialogs, self.find_open);
        self.place_focus(&previous, &model_focus);
        let fresh = initial_buffers(self.model());
        self.originals.clone_from(&fresh);
        if keep {
            // Only edits carry over. Every other field takes the model's value, which a reload may have changed.
            self.buffers.clone_from(&fresh);
            self.dirty = old_dirty
                .into_iter()
                .filter(|id| self.originals.contains_key(id))
                .collect();
            for id in &self.dirty {
                if let Some(value) = old_buffers.get(id) {
                    self.buffers.insert(id.clone(), value.clone());
                }
            }
        } else {
            self.buffers = fresh;
            self.dirty.clear();
            self.marks.clear();
        }
        self.dropdown_open = false;
        self.dropdown_at = 0;
        self.pages = pages_from(self.model());
    }

    fn place_focus(&mut self, previous: &str, model_focus: &str) {
        if let Some(index) = self.focus.iter().position(|item| item.id == previous) {
            self.focus_at = index;
            return;
        }
        if let Some(index) = self.focus.iter().position(|item| item.id == model_focus) {
            self.focus_at = index;
            return;
        }
        self.focus_at = 0;
    }

    fn first_list(&self) -> Option<String> {
        let mut found = None;
        walk(&self.model().c, &mut |node| {
            if found.is_none() {
                if let Node::FormList { list } = node {
                    found = Some(list.id.clone());
                }
            }
        });
        found
    }

    fn focused_widget(&self) -> Option<&Widget> {
        self.field_widget(self.focus_id())
    }
}

struct FieldBits {
    id: String,
    name: String,
    required: bool,
    maxlength: u32,
    posts: bool,
}

struct FormBits {
    transition_url: String,
    method: String,
    submit_message: String,
    background_hide_id: String,
    fields: Vec<FieldBits>,
    hidden: Vec<(String, String)>,
}

fn focus_items(model: &ScreenModel, dialogs: &[String], find_open: bool) -> Vec<FocusItem> {
    let mut items = Vec::new();
    collect_focus(&model.c, dialogs, find_open, &mut items);
    items
}

fn collect_focus(nodes: &[Node], dialogs: &[String], find_open: bool, items: &mut Vec<FocusItem>) {
    for node in nodes {
        match node {
            Node::Field { id, w, .. } => {
                if let Some(kind) = focus_kind(w) {
                    items.push(FocusItem {
                        id: id.clone(),
                        kind,
                        url: None,
                    });
                }
            }
            Node::Link { id, url, .. } => items.push(FocusItem {
                id: id.clone(),
                kind: FocusKind::Link,
                url: Some(url.clone()),
            }),
            Node::FormList { list } => {
                items.push(FocusItem {
                    id: list.id.clone(),
                    kind: FocusKind::List,
                    url: None,
                });
                if find_open {
                    collect_focus(&list.header_fields, dialogs, find_open, items);
                }
            }
            Node::Dialog { id, c, .. } => {
                items.push(FocusItem {
                    id: id.clone(),
                    kind: FocusKind::Dialog,
                    url: None,
                });
                if dialogs.iter().any(|open| open == id) {
                    collect_focus(c, dialogs, find_open, items);
                }
            }
            other => collect_focus(other.children(), dialogs, find_open, items),
        }
    }
}

fn focus_kind(widget: &Widget) -> Option<FocusKind> {
    if widget.is_editable_text() {
        return Some(FocusKind::Text);
    }
    match widget {
        Widget::DropDown { .. } => Some(FocusKind::DropDown),
        Widget::Submit { .. } => Some(FocusKind::Submit),
        _ => None,
    }
}

fn initial_buffers(model: &ScreenModel) -> BTreeMap<String, String> {
    let mut buffers = BTreeMap::new();
    walk(&model.c, &mut |node| {
        if let Node::Field { id, w, .. } = node {
            if !w.is_display() && !matches!(w, Widget::Submit { .. } | Widget::Reset { .. }) {
                buffers.insert(id.clone(), w.value().to_string());
            }
        }
    });
    buffers
}

fn pages_from(model: &ScreenModel) -> BTreeMap<String, usize> {
    let mut pages = BTreeMap::new();
    walk(&model.c, &mut |node| {
        if let Node::FormList { list } = node {
            pages.insert(list.id.clone(), usize::try_from(list.paging.index).unwrap_or(0));
        }
    });
    pages
}

fn form_list<'a>(model: &'a ScreenModel, list_id: &str) -> Option<&'a crate::model::FormList> {
    let mut found = None;
    walk(&model.c, &mut |node| {
        if let Node::FormList { list } = node {
            if list.id == list_id {
                found = Some(list);
            }
        }
    });
    found
}

fn page_size_of(list: &crate::model::FormList) -> usize {
    if list.paging.size == 0 {
        PAGE_ROWS
    } else {
        usize::try_from(list.paging.size).unwrap_or(PAGE_ROWS).max(1)
    }
}

fn page_count_of(list: &crate::model::FormList) -> usize {
    let size = page_size_of(list);
    let count = if list.paging.count == 0 {
        list.rows.len()
    } else {
        usize::try_from(list.paging.count).unwrap_or(list.rows.len())
    };
    count.div_ceil(size).max(1)
}

fn sort_fields(model: &ScreenModel, list_id: &str) -> Vec<String> {
    form_list(model, list_id)
        .map(|list| {
            list.columns
                .iter()
                .filter(|column| !column.order_by.is_empty())
                .map(|column| column.order_by.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Ascending then descending on each sortable column in turn, then unsorted.
fn next_order(fields: &[String], current: &str) -> String {
    let (descending, name) = match current.strip_prefix('-') {
        Some(name) => (true, name),
        None => (false, current),
    };
    match fields.iter().position(|field| field == name) {
        None => fields[0].clone(),
        Some(_) if !descending => format!("-{name}"),
        Some(at) => fields.get(at + 1).cloned().unwrap_or_default(),
    }
}

fn field_name<'a>(model: &'a ScreenModel, id: &str) -> Option<&'a str> {
    let mut found = None;
    walk(&model.c, &mut |node| {
        if let Node::Field { id: field_id, name, .. } = node {
            if field_id == id {
                found = Some(name.as_str());
            }
        }
    });
    found
}

fn dependent_dropdowns(model: &ScreenModel, name: &str) -> Vec<String> {
    let mut ids = Vec::new();
    walk(&model.c, &mut |node| {
        if let Node::Field {
            id,
            w:
                Widget::DropDown {
                    options_url,
                    depends_on,
                    ..
                },
            ..
        } = node
        {
            if !options_url.is_empty() && depends_on.iter().any(|dep| dep == name) {
                ids.push(id.clone());
            }
        }
    });
    ids
}

fn option_query(
    model: &ScreenModel,
    field_id: &str,
    buffers: &BTreeMap<String, String>,
) -> Option<(String, BTreeMap<String, String>)> {
    let mut query = None;
    walk(&model.c, &mut |node| {
        if let Node::Field { id, w, .. } = node {
            if id == field_id {
                if let Widget::DropDown {
                    options_url,
                    options_parameters,
                    depends_on,
                    ..
                } = w
                {
                    if options_url.is_empty() {
                        return;
                    }
                    let mut params = BTreeMap::new();
                    let names = if options_parameters.is_empty() {
                        depends_on
                    } else {
                        options_parameters
                    };
                    for name in names {
                        if let Some(value) = buffer_named(model, buffers, name) {
                            params.insert(name.clone(), value);
                        }
                    }
                    query = Some((options_url.clone(), params));
                }
            }
        }
    });
    query
}

fn buffer_named(model: &ScreenModel, buffers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    let mut found = None;
    walk(&model.c, &mut |node| {
        if let Node::Field {
            id, name: field_name, ..
        } = node
        {
            if field_name == name {
                found = Some(buffers.get(id).cloned().unwrap_or_default());
            }
        }
    });
    found
}

fn set_dropdown_options(nodes: &mut [Node], field_id: &str, choices: &[Choice]) {
    for node in nodes {
        if let Node::Field { id, w, .. } = node {
            if id == field_id {
                if let Widget::DropDown { options, value, .. } = w {
                    *options = choices.to_vec();
                    if !value.is_empty() && !choices.iter().any(|choice| choice.value == *value) {
                        value.clear();
                    }
                }
            }
        }
        set_dropdown_options(node.children_mut(), field_id, choices);
    }
}

fn find_form(model: &ScreenModel, field_id: &str) -> Option<FormBits> {
    find_form_in(&model.c, field_id)
}

fn find_form_in(nodes: &[Node], field_id: &str) -> Option<FormBits> {
    for node in nodes {
        if let Node::Form {
            id,
            transition_url,
            method,
            submit_message,
            background_hide_id,
            hidden,
            c,
            ..
        } = node
        {
            let fields = input_fields(c);
            let contains = id == field_id || fields.iter().any(|field| field.id == field_id);
            if contains {
                return Some(FormBits {
                    transition_url: transition_url.clone(),
                    method: if method.is_empty() {
                        "post".to_string()
                    } else {
                        method.clone()
                    },
                    submit_message: submit_message.clone(),
                    background_hide_id: background_hide_id.clone(),
                    fields,
                    hidden: hidden
                        .iter()
                        .map(|item| (item.name.clone(), item.value.clone()))
                        .collect(),
                });
            }
            if let Some(found) = find_form_in(c, field_id) {
                return Some(found);
            }
        } else if let Some(found) = find_form_in(node.children(), field_id) {
            return Some(found);
        }
    }
    None
}

fn input_fields(nodes: &[Node]) -> Vec<FieldBits> {
    let mut fields = Vec::new();
    walk(nodes, &mut |node| {
        if let Node::Field {
            id,
            name,
            required,
            maxlength,
            w,
            ..
        } = node
        {
            let posts =
                !w.is_display() && !matches!(w, Widget::Submit { .. } | Widget::Reset { .. } | Widget::Password {});
            if posts || matches!(w, Widget::Submit { .. } | Widget::Password {}) {
                fields.push(FieldBits {
                    id: id.clone(),
                    name: name.clone(),
                    required: *required,
                    maxlength: *maxlength,
                    posts,
                });
            }
        }
    });
    fields
}

fn client_marks(form: &FormBits, buffers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut marks = Vec::new();
    for field in &form.fields {
        if !field.posts {
            continue;
        }
        let value = buffers.get(&field.id).map(String::as_str).unwrap_or("");
        if field.required && value.is_empty() {
            marks.push((field.id.clone(), "required".to_string()));
        } else if field.maxlength > 0 && value.chars().count() > usize::try_from(field.maxlength).unwrap_or(usize::MAX)
        {
            marks.push((field.id.clone(), "maxlength".to_string()));
        }
    }
    marks
}
