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

//! Focus, buffers, paging, and the small key set. Text keys update the buffer here.
//! The `ui` feature mirrors the focused text field into a Revue `Input`.

use std::collections::BTreeMap;

use crate::keys::{Key, KeyPress};
use crate::layout::PAGE_ROWS;
use crate::link::ScreenLink;
use crate::model::{walk, Choice, Node, ScreenModel, Widget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusKind {
    Text,
    DropDown,
    Submit,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusItem {
    pub id: String,
    pub kind: FocusKind,
    pub url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub model: ScreenModel,
    focus: Vec<FocusItem>,
    focus_at: usize,
    buffers: BTreeMap<String, String>,
    pages: BTreeMap<String, usize>,
    dropdown_open: bool,
    dropdown_at: usize,
}

impl Session {
    pub fn new(model: ScreenModel) -> Self {
        let mut session = Self {
            model,
            focus: Vec::new(),
            focus_at: 0,
            buffers: BTreeMap::new(),
            pages: BTreeMap::new(),
            dropdown_open: false,
            dropdown_at: 0,
        };
        session.rebuild();
        session
    }

    pub fn load(&mut self, model: ScreenModel) {
        self.model = model;
        self.pages.clear();
        self.rebuild();
    }

    pub fn path(&self) -> &str {
        &self.model.screen.path
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
        self.buffers.insert(id.to_string(), value);
    }

    pub fn page_index(&self, list_id: &str) -> usize {
        self.pages.get(list_id).copied().unwrap_or(0)
    }

    pub fn page_count(&self, list_id: &str) -> usize {
        let rows = self.list_len(list_id);
        rows.div_ceil(PAGE_ROWS).max(1)
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
    ///
    /// Text editing updates the buffer directly. A Revue `Input` for the focused field,
    /// when the `ui` feature is on, is rebuilt from that buffer so the cursor stays at
    /// the end of the spike's edits. `Left` and `Right` move the caret only inside the
    /// widget copy; the buffer is the value.
    pub fn on_key(&mut self, press: &KeyPress, link: &dyn ScreenLink) -> bool {
        if self.dropdown_open {
            return self.dropdown_key(press);
        }
        if self.focused_is_text() && !press.ctrl && !press.alt {
            if let Key::Char(ch) = press.key {
                self.insert_char(ch);
                return false;
            }
            if press.key == Key::Backspace {
                self.backspace();
                return false;
            }
        }
        match press.key {
            Key::Char('q') if !self.focused_is_text() => true,
            Key::Up | Key::BackTab => {
                self.move_focus(-1);
                false
            }
            Key::Down | Key::Tab => {
                self.move_focus(1);
                false
            }
            Key::Enter => self.activate(link),
            Key::F(4) => {
                self.open_dropdown();
                false
            }
            Key::F(7) | Key::PageUp => {
                self.page_lists(-1);
                false
            }
            Key::F(8) | Key::PageDown => {
                self.page_lists(1);
                false
            }
            _ => false,
        }
    }

    fn dropdown_key(&mut self, press: &KeyPress) -> bool {
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
            Key::Enter => self.commit_dropdown(),
            Key::Escape | Key::F(4) => self.dropdown_open = false,
            _ => {}
        }
        false
    }

    fn open_dropdown(&mut self) {
        if self.focus_kind() != Some(FocusKind::DropDown) {
            return;
        }
        let current = self.buffer(self.focus_id()).to_string();
        let at = self
            .focused_choices()
            .iter()
            .position(|choice| choice.value == current)
            .unwrap_or(0);
        self.dropdown_at = at;
        self.dropdown_open = true;
    }

    fn commit_dropdown(&mut self) {
        let id = self.focus_id().to_string();
        if let Some(choice) = self.focused_choices().get(self.dropdown_at).cloned() {
            self.buffers.insert(id, choice.value);
        }
        self.dropdown_open = false;
    }

    fn activate(&mut self, link: &dyn ScreenLink) -> bool {
        if self.focus_kind() == Some(FocusKind::DropDown) {
            self.open_dropdown();
            return false;
        }
        let Some(url) = self.focus.get(self.focus_at).and_then(|item| item.url.clone()) else {
            return false;
        };
        if let Ok(model) = link.level(&url) {
            self.load(model);
        }
        false
    }

    fn move_focus(&mut self, delta: isize) {
        if self.focus.is_empty() {
            return;
        }
        let len = self.focus.len() as isize;
        let next = (self.focus_at as isize + delta).rem_euclid(len) as usize;
        self.focus_at = next;
        self.dropdown_open = false;
    }

    fn page_lists(&mut self, delta: isize) {
        let ids: Vec<String> = self.pages.keys().cloned().collect();
        for id in ids {
            let count = self.page_count(&id);
            let current = self.page_index(&id) as isize;
            let next = (current + delta).clamp(0, count as isize - 1) as usize;
            self.pages.insert(id, next);
        }
    }

    fn insert_char(&mut self, ch: char) {
        let id = self.focus_id().to_string();
        let mut text = self.buffer(&id).to_string();
        text.push(ch);
        self.buffers.insert(id, text);
    }

    fn backspace(&mut self) {
        let id = self.focus_id().to_string();
        let mut text = self.buffer(&id).to_string();
        text.pop();
        self.buffers.insert(id, text);
    }

    fn rebuild(&mut self) {
        self.focus = focus_items(&self.model);
        self.focus_at = 0;
        self.buffers = initial_buffers(&self.model);
        self.dropdown_open = false;
        self.dropdown_at = 0;
        self.pages = list_ids(&self.model).into_iter().map(|id| (id, 0)).collect();
    }

    fn list_len(&self, list_id: &str) -> usize {
        let mut found = 0;
        walk(&self.model.c, &mut |node| {
            if let Node::FormList { list } = node {
                if list.id == list_id {
                    found = list.rows.len();
                }
            }
        });
        found
    }

    fn focused_widget(&self) -> Option<&Widget> {
        let id = self.focus_id();
        let mut found = None;
        walk(&self.model.c, &mut |node| {
            if let Node::Field { id: field_id, w, .. } = node {
                if field_id == id {
                    found = Some(w);
                }
            }
        });
        found
    }

    pub fn field_widget(&self, id: &str) -> Option<&Widget> {
        let mut found = None;
        walk(&self.model.c, &mut |node| {
            if let Node::Field { id: field_id, w, .. } = node {
                if field_id == id {
                    found = Some(w);
                }
            }
        });
        found
    }

    pub fn list_page(&self, list_id: &str) -> Vec<crate::model::ListRow> {
        let mut rows = Vec::new();
        walk(&self.model.c, &mut |node| {
            if let Node::FormList { list } = node {
                if list.id == list_id {
                    let start = self.page_index(list_id) * PAGE_ROWS;
                    rows = list.rows.iter().skip(start).take(PAGE_ROWS).cloned().collect();
                }
            }
        });
        rows
    }
}

fn focus_items(model: &ScreenModel) -> Vec<FocusItem> {
    let mut items = Vec::new();
    collect_focus(&model.c, &mut items);
    items
}

fn collect_focus(nodes: &[Node], items: &mut Vec<FocusItem>) {
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
            Node::Box { c, .. } | Node::Form { c, .. } => collect_focus(c, items),
            Node::FormList { .. } | Node::Text { .. } => {}
        }
    }
}

fn focus_kind(widget: &Widget) -> Option<FocusKind> {
    match widget {
        Widget::TextLine { .. } | Widget::DateTime { .. } => Some(FocusKind::Text),
        Widget::DropDown { .. } => Some(FocusKind::DropDown),
        Widget::Submit { .. } => Some(FocusKind::Submit),
        Widget::Display { .. } => None,
    }
}

fn initial_buffers(model: &ScreenModel) -> BTreeMap<String, String> {
    let mut buffers = BTreeMap::new();
    walk(&model.c, &mut |node| {
        if let Node::Field { id, w, .. } = node {
            if !w.is_display() {
                buffers.insert(id.clone(), w.value().to_string());
            }
        }
    });
    buffers
}

fn list_ids(model: &ScreenModel) -> Vec<String> {
    let mut ids = Vec::new();
    walk(&model.c, &mut |node| {
        if let Node::FormList { list } = node {
            ids.push(list.id.clone());
        }
    });
    ids
}
