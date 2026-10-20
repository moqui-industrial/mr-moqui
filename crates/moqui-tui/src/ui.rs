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

//! Thin Revue interpreter for the spike fixtures.
//!
//! `Select` and `DateTimePicker` are not used. A drop-down is an `OptionList`.
//! A date is the date string in an `Input`. Layout comes from [`crate::layout`],
//! which does not import Revue.

use std::io::Write;
use std::time::Instant;

use revue::core::app::App;
use revue::event::{Event, Key as RevueKey, KeyEvent};
use revue::layout::Rect;
use revue::style::Color;
use revue::testing::PipelineHarness;
use revue::widget::{Input, OptionItem, OptionList, View};

use crate::display_width;
use crate::keys::{Key, KeyPress};
use crate::layout::{layout, list_column_widths, truncate_width, Block, BoxChild, LabelMode, LaidField};
use crate::link::{FakeLink, ScreenLink};
use crate::model::{Node, ScreenModel, Widget};
use crate::session::Session;

pub struct AppView {
    session: Session,
    link: FakeLink,
    live_id: Option<String>,
    live: Option<Input>,
    dropdown: Option<OptionList>,
}

impl AppView {
    pub fn new(model: ScreenModel, link: FakeLink) -> Self {
        Self {
            session: Session::new(model),
            link,
            live_id: None,
            live: None,
            dropdown: None,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn prepare(&mut self) {
        if self.session.focused_is_text() {
            self.ensure_input();
        }
    }

    /// Returns true when the local loop should quit.
    pub fn on_key(&mut self, press: &KeyPress) -> bool {
        if self.dropdown.is_some() {
            self.dropdown_key(press);
            return false;
        }
        if self.edit_key(press) {
            return false;
        }
        let quit = self.session.on_key(press, &self.link);
        self.after_key();
        quit
    }

    pub fn screen_text(&mut self, width: u16, height: u16) -> String {
        self.prepare();
        let mut harness = PipelineHarness::new(width, height);
        harness.draw(self);
        // The second cell of a wide glyph is a NUL spacer. Drop it so the text is the
        // characters a reader sees.
        harness.screen_text().replace('\0', "")
    }

    fn edit_key(&mut self, press: &KeyPress) -> bool {
        if !self.session.focused_is_text() || press.ctrl || press.alt {
            return false;
        }
        let revue_key = revue_key(&press.key);
        if !matches!(
            revue_key,
            RevueKey::Char(_)
                | RevueKey::Backspace
                | RevueKey::Delete
                | RevueKey::Left
                | RevueKey::Right
                | RevueKey::Home
                | RevueKey::End
        ) {
            return false;
        }
        self.ensure_input();
        let mut event = KeyEvent::new(revue_key);
        event.shift = press.shift;
        let Some(input) = self.live.as_mut() else {
            return false;
        };
        input.handle_key_event(&event);
        let text = input.text().to_string();
        let id = self.session.focus_id().to_string();
        self.session.set_buffer(&id, text);
        true
    }

    fn dropdown_key(&mut self, press: &KeyPress) {
        match press.key {
            Key::Up => {
                if let Some(list) = self.dropdown.as_mut() {
                    list.highlight_previous();
                }
            }
            Key::Down => {
                if let Some(list) = self.dropdown.as_mut() {
                    list.highlight_next();
                }
            }
            Key::Enter => {
                let chosen = self.dropdown.as_ref().and_then(|list| {
                    list.get_highlighted()
                        .map(|item| item.value.clone().unwrap_or_default())
                });
                let id = self.session.focus_id().to_string();
                if let Some(value) = chosen {
                    self.session.set_buffer(&id, value);
                }
                self.session.close_dropdown();
                self.dropdown = None;
            }
            Key::Escape | Key::F(4) => {
                self.session.close_dropdown();
                self.dropdown = None;
            }
            _ => {}
        }
    }

    fn after_key(&mut self) {
        if self.session.dropdown_open() && self.dropdown.is_none() {
            self.dropdown = Some(self.build_list());
        }
        if !self.session.dropdown_open() {
            self.dropdown = None;
        }
        if self.session.focused_is_text() {
            self.ensure_input();
        } else if self.live_id.as_deref() != Some(self.session.focus_id()) {
            self.live = None;
            self.live_id = None;
        }
    }

    fn ensure_input(&mut self) {
        let id = self.session.focus_id().to_string();
        if self.live_id.as_deref() == Some(id.as_str()) && self.live.is_some() {
            return;
        }
        let value = self.session.buffer(&id).to_string();
        self.live = Some(Input::new().value(value).focused(true));
        self.live_id = Some(id);
    }

    fn build_list(&self) -> OptionList {
        let choices = self.session.focused_choices();
        let mut list = OptionList::new()
            .focused(true)
            .show_icons(false)
            .width(32)
            .max_visible(choices.len().max(1));
        for choice in choices {
            let label = if choice.label.is_empty() {
                "(empty)".to_string()
            } else {
                choice.label
            };
            list = list.add_option(OptionItem::new(label).value(choice.value));
        }
        for _ in 0..self.session.dropdown_at() {
            list.highlight_next();
        }
        list
    }
}

impl View for AppView {
    fn render(&self, ctx: &mut revue::widget::RenderContext) {
        let width = ctx.area.width;
        let height = ctx.area.height;
        let lines = screen_lines(&self.session, width);
        for (index, line) in lines.iter().enumerate() {
            if index as u16 >= height {
                break;
            }
            ctx.draw_text(0, index as u16, line, Color::WHITE);
        }
        // Input edits the buffer. The line already shows that buffer. Painting the
        // widget on top of the brackets leaves the closing bracket behind.
        if let Some(list) = &self.dropdown {
            if let Some((x, y)) = dropdown_spot(&self.session, width) {
                let rows = u16::try_from(self.session.focused_choices().len().max(1)).unwrap_or(1);
                ctx.render_child(list, Rect::new(x, y, 32, rows));
            }
        }
    }
}

pub fn render_text(model: &ScreenModel, link: &FakeLink, width: u16, height: u16) -> String {
    AppView::new(model.clone(), link.clone()).screen_text(width, height)
}

pub fn run_local(link: FakeLink, path: &str) -> revue::Result<()> {
    let model = link.level(path).unwrap_or_else(|err| panic!("{err}"));
    let mut view = AppView::new(model, link);
    view.prepare();
    App::builder()
        .mouse_capture(false)
        .build()
        .run(view, |event, view, app| {
            if let Event::Key(key) = event {
                if let Some(press) = from_revue(key) {
                    if view.on_key(&press) {
                        app.quit();
                    }
                }
            }
            true
        })
}

/// Draw with [`PipelineHarness`] and copy each new byte suffix to `out`.
///
/// The harness keeps every byte it has written. `harness_len` stays equal to the
/// number of bytes copied and grows with each changed frame, which is why this
/// path is not the SSH session loop.
pub struct DrawProbe {
    pub harness_len: usize,
    pub copied_len: usize,
    pub frames: usize,
    pub millis: u128,
}

pub fn run_session<W: Write>(
    model: &ScreenModel,
    link: &FakeLink,
    events: &[Event],
    size: (u16, u16),
    out: &mut W,
) -> DrawProbe {
    let started = Instant::now();
    let mut harness = PipelineHarness::new(size.0, size.1);
    let mut view = AppView::new(model.clone(), link.clone());
    view.prepare();
    harness.draw(&view);
    let mut copied = 0usize;
    copy_new(&harness, out, &mut copied);
    for event in events {
        if let Event::Key(key) = event {
            if let Some(press) = from_revue(key) {
                view.on_key(&press);
            }
        }
        view.prepare();
        harness.draw(&view);
        copy_new(&harness, out, &mut copied);
    }
    DrawProbe {
        harness_len: harness.terminal_output().len(),
        copied_len: copied,
        frames: harness.frame_count(),
        millis: started.elapsed().as_millis(),
    }
}

fn copy_new(harness: &PipelineHarness, out: &mut impl Write, copied: &mut usize) {
    let bytes = harness.terminal_output();
    if bytes.len() > *copied {
        let _ = out.write_all(&bytes[*copied..]);
        *copied = bytes.len();
    }
}

fn from_revue(event: &KeyEvent) -> Option<KeyPress> {
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

fn revue_key(key: &Key) -> RevueKey {
    match key {
        Key::Char(ch) => RevueKey::Char(*ch),
        Key::Enter => RevueKey::Enter,
        Key::Escape => RevueKey::Escape,
        Key::Tab => RevueKey::Tab,
        Key::BackTab => RevueKey::BackTab,
        Key::Backspace => RevueKey::Backspace,
        Key::Delete => RevueKey::Delete,
        Key::Up => RevueKey::Up,
        Key::Down => RevueKey::Down,
        Key::Left => RevueKey::Left,
        Key::Right => RevueKey::Right,
        Key::Home => RevueKey::Home,
        Key::End => RevueKey::End,
        Key::PageUp => RevueKey::PageUp,
        Key::PageDown => RevueKey::PageDown,
        Key::F(n) => RevueKey::F(*n),
    }
}

/// [`run_session`] fed from spike [`KeyPress`] values.
pub fn run_key_session<W: Write>(
    model: &ScreenModel,
    link: &FakeLink,
    keys: &[KeyPress],
    size: (u16, u16),
    out: &mut W,
) -> DrawProbe {
    let events: Vec<Event> = keys
        .iter()
        .map(|press| {
            let mut event = KeyEvent::new(revue_key(&press.key));
            event.alt = press.alt;
            event.ctrl = press.ctrl;
            event.shift = press.shift;
            Event::Key(event)
        })
        .collect();
    run_session(model, link, &events, size, out)
}

fn screen_lines(session: &Session, canvas: u16) -> Vec<String> {
    let laid = layout(&session.model, canvas);
    let mut lines = vec![session.model.screen.title.clone()];
    for block in &laid.blocks {
        match block {
            Block::Boxes(boxes) => {
                if boxes.iter().any(|box_| box_.x > 0) {
                    lines.extend(merge_boxes(session, boxes));
                } else {
                    for box_ in boxes {
                        lines.extend(one_box(session, box_));
                    }
                }
            }
            Block::Form(form) => {
                for field in &form.fields {
                    lines.extend(field_lines(session, field, canvas));
                }
            }
            Block::List(list) => lines.extend(list_lines(session, &list.id, canvas)),
            Block::Link(link) => {
                let mark = if session.focus_id() == link.id { ">" } else { " " };
                lines.push(format!("{mark} {}", link.text));
            }
            Block::Text(text) => lines.push(text.clone()),
        }
    }
    if let Some(list_id) = first_list_id(session) {
        let page = session.page_index(&list_id) + 1;
        let count = session.page_count(&list_id);
        lines.push(format!("Page {page}/{count}"));
    }
    lines.push(format!("focus {}", session.focus_id()));
    lines
        .into_iter()
        .map(|line| truncate_width(&line, usize::from(canvas)))
        .collect()
}

fn one_box(session: &Session, box_: &crate::layout::LaidBox) -> Vec<String> {
    let mut lines = vec![format!("+ {} ", box_.title)];
    for child in &box_.children {
        match child {
            BoxChild::Field(field) => lines.extend(field_lines(session, field, box_.w)),
            BoxChild::Link(link) => lines.push(link.text.clone()),
            BoxChild::Text(text) => lines.push(text.clone()),
        }
    }
    lines
        .into_iter()
        .map(|line| truncate_width(&line, usize::from(box_.w)))
        .collect()
}

fn merge_boxes(session: &Session, boxes: &[crate::layout::LaidBox]) -> Vec<String> {
    let rendered: Vec<Vec<String>> = boxes.iter().map(|box_| one_box(session, box_)).collect();
    let height = rendered.iter().map(Vec::len).max().unwrap_or(0);
    let mut rows = Vec::new();
    for index in 0..height {
        let mut row = String::new();
        for (box_, lines) in boxes.iter().zip(&rendered) {
            let pad = usize::from(box_.x).saturating_sub(display_width(&row));
            row.push_str(&" ".repeat(pad));
            if let Some(line) = lines.get(index) {
                row.push_str(line);
            }
        }
        rows.push(row);
    }
    rows
}

fn field_lines(session: &Session, field: &LaidField, width: u16) -> Vec<String> {
    let shown = field_shown(session, &field.id);
    let mark = if session.focus_id() == field.id { ">" } else { " " };
    match field.mode {
        LabelMode::Above => vec![format!("{mark} {}", field.title), format!("  {shown}")],
        LabelMode::Beside => vec![format!("{mark} {} {shown}", field.title)],
    }
    .into_iter()
    .map(|line| truncate_width(&line, usize::from(width.max(1))))
    .collect()
}

fn field_shown(session: &Session, id: &str) -> String {
    let Some(widget) = session.field_widget(id) else {
        return String::new();
    };
    match widget {
        Widget::Display { text } => text.clone(),
        Widget::Submit { text } => format!("< {text} >"),
        Widget::DropDown { options, .. } => {
            let value = session.buffer(id);
            let label = options
                .iter()
                .find(|choice| choice.value == value)
                .map(|choice| choice.label.as_str())
                .unwrap_or(value);
            let label = if label.is_empty() { " " } else { label };
            format!("[{label} ▾]")
        }
        Widget::TextLine { .. } | Widget::DateTime { .. } => {
            format!("[{}]", session.buffer(id))
        }
    }
}

fn list_lines(session: &Session, list_id: &str, canvas: u16) -> Vec<String> {
    let widths = list_column_widths(&session.model, canvas, list_id);
    let titles = list_titles(session, list_id);
    if titles.is_empty() {
        return Vec::new();
    }
    let scroll = widths.iter().sum::<usize>() + widths.len().saturating_sub(1) > usize::from(canvas);
    let mut header_cells = Vec::new();
    for (index, title) in titles.iter().enumerate() {
        let width = widths.get(index).copied().unwrap_or(0);
        header_cells.push(fit_cell(title, width));
    }
    let mut header = header_cells.join(" ");
    if scroll {
        let room = usize::from(canvas).saturating_sub(2);
        header = truncate_width(&header, room);
        header.push_str(" >");
    }
    let mut lines = vec![header];
    for row in session.list_page(list_id) {
        let mut cells = Vec::new();
        for (index, width) in widths.iter().enumerate() {
            let text = row.cells.get(index).map(|cell| cell.text()).unwrap_or("");
            cells.push(fit_cell(text, *width));
        }
        lines.push(cells.join(" "));
    }
    lines
}

fn list_titles(session: &Session, list_id: &str) -> Vec<String> {
    let mut titles = Vec::new();
    crate::model::walk(&session.model.c, &mut |node| {
        if let Node::FormList { list } = node {
            if list.id == list_id {
                titles = list.columns.iter().map(|column| column.title.clone()).collect();
            }
        }
    });
    titles
}

fn first_list_id(session: &Session) -> Option<String> {
    let mut id = None;
    crate::model::walk(&session.model.c, &mut |node| {
        if id.is_none() {
            if let Node::FormList { list } = node {
                id = Some(list.id.clone());
            }
        }
    });
    id
}

fn fit_cell(text: &str, width: usize) -> String {
    let fitted = truncate_width(text, width);
    let pad = width.saturating_sub(display_width(&fitted));
    format!("{fitted}{}", " ".repeat(pad))
}

fn dropdown_spot(session: &Session, canvas: u16) -> Option<(u16, u16)> {
    if !session.dropdown_open() {
        return None;
    }
    spot_of(session, canvas, session.focus_id()).map(|(x, y)| (x, y.saturating_add(1)))
}

/// Column where the focused value starts, and its row.
fn spot_of(session: &Session, canvas: u16, id: &str) -> Option<(u16, u16)> {
    let lines = screen_lines(session, canvas);
    let needle = field_shown(session, id);
    for (index, line) in lines.iter().enumerate() {
        if let Some(at) = line.find(&needle) {
            let x = display_width(&line[..at]);
            return Some((u16::try_from(x).unwrap_or(0), u16::try_from(index).unwrap_or(0)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Layout width and Revue's table, on the strings the fixtures paint.
    /// A ZWJ family is three emoji plus two joiners. Revue counts 6.
    /// `unicode-width` 0.2 counts 2.
    #[test]
    fn display_width_matches_revue() {
        use unicode_width::UnicodeWidthStr;
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let samples = ["東京", "🎉", family, "cafe\u{0301}", "family-note", "ScreenThemeType"];
        for sample in samples {
            assert_eq!(display_width(sample), revue::utils::display_width(sample), "{sample:?}");
        }
        assert_eq!(display_width("東京"), 4);
        assert_eq!(display_width("🎉"), 2);
        assert_eq!(display_width(family), 6);
        assert_eq!(display_width("cafe\u{0301}"), 4);
        assert_eq!("東京".width(), 4);
        assert_eq!("🎉".width(), 2);
        assert_eq!("cafe\u{0301}".width(), 4);
        // unicode-width 0.2 collapses a ZWJ family to one cell. Revue counts each emoji.
        assert_eq!(family.width(), 2);
    }
}
