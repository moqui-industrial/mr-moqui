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

//! The agent mode (plans/Screen-TUI-Impl.md Phase 3c, docs/TUI.md "For agents"): the same session, driven one command
//! line at a time by a program rather than a person. Each command runs its keys and requests on this thread, fires the
//! waits for typing to stop, and answers once the screen has settled: no timing to guess, no escape sequences to read.
//! The answer names the place, the focus, any overlay and the message, then gives the screen as text with the focused
//! element in `[[ ]]`; `--json` gives the same as one object, with the focusable elements. Elements are found by id,
//! by number, or by their label, so an agent never counts Tab presses.

use std::collections::BTreeMap;

use revue::layout::Rect;
use revue::render::{Buffer, Modifier};
use revue::widget::{RenderContext, View};
use serde_json::{json, Value};

use crate::forms::{FocusItem, FocusKind};
use crate::keys::{Key, KeyPress, ACTIONS};
use crate::link::ScreenLink;
use crate::model::walk;
use crate::session::{Overlay, Session};
use crate::state::{Confirm, ConfirmKind};
use crate::theme::Theme;

use super::AppView;

/// The commands, for `help` and for an unknown command's answer.
pub const AGENT_HELP: &str = "Commands (an element is its id, its number from `elements`, as #3, or text of its label):
  look                     the screen again
  key <chord>...           keys: F10 Enter Esc Tab Shift+Tab Up Down PgDn Space Ctrl+K Alt+M F4 ...
  type <text>              characters as typed; \\r is Enter, \\t is Tab
  go <path[?query]>        go to a screen
  focus <element>          move the focus to an element
  set <element> <value>    replace a field's value; a drop-down takes a choice's value or label
  do <element>             focus an element and press Enter: a button, link, list row, menu
  find <text>              move to the text on the screen, a list row's included (as /)
  elements                 the focusable elements: number, kind, id, label, value
  size <cols>x<rows>       the drawing size (a tall one shows long lists whole)
  format text|json         how answers are written; --json on one command for that one
  help                     this list, and the screen's keys
  close                    end the session";

/// One command's answer: whether it did what was asked, and the text to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAnswer {
    pub ok: bool,
    pub text: String,
}

pub struct Agent<L: ScreenLink> {
    view: AppView<L>,
    columns: u16,
    rows: u16,
    json: bool,
    closed: bool,
    /// Notifications the link pushed since the last command, applied before the next answer.
    pushes: std::sync::mpsc::Receiver<crate::link::Push>,
    _subscription: Option<crate::link::Subscription>,
}

impl<L: ScreenLink> Agent<L> {
    /// A session on `path`, drawn at `columns` by `rows`, settled.
    pub fn open(link: L, path: &str, columns: u16, rows: u16) -> Self {
        let (push_tx, pushes) = std::sync::mpsc::channel();
        let subscription = link.subscribe(std::sync::Arc::new(move |push| {
            let _ = push_tx.send(push);
        }));
        let mut view = AppView::with_session(Session::start(path), link);
        // Plain text is all an agent reads; the theme only has to keep the focus in reverse video.
        view.set_theme(Theme::default());
        view.session_mut().resize(columns, rows);
        view.session_mut().set_pushed(subscription.is_some());
        let mut agent = Self {
            view,
            columns,
            rows,
            json: false,
            closed: false,
            pushes,
            _subscription: subscription,
        };
        agent.settle();
        agent
    }

    /// Answer in JSON from now on.
    pub fn set_json(&mut self, json: bool) {
        self.json = json;
    }

    /// The user quit (Ctrl+Q) or sent `close`.
    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn session(&self) -> &Session {
        self.view.session()
    }

    /// The answer for the screen as it is: `look`.
    pub fn look(&self) -> AgentAnswer {
        self.answer(Ok(()), self.json)
    }

    /// Run one command line and answer it. A command that fails leaves the session as it was, as far as it got.
    pub fn run(&mut self, line: &str) -> AgentAnswer {
        let mut words: Vec<&str> = line.split_whitespace().collect();
        let json = match words.iter().position(|word| *word == "--json") {
            Some(at) => {
                words.remove(at);
                true
            }
            None => self.json,
        };
        let command = words.first().copied().unwrap_or("look");
        // The rest of the line after the command word, spaces kept: `type` and `set` values may hold them.
        let rest = line
            .trim_start()
            .strip_prefix(command)
            .unwrap_or("")
            .replace("--json", "")
            .trim()
            .to_string();
        if self.closed {
            return self.answer(Err("the session is closed".to_string()), json);
        }
        while let Ok(push) = self.pushes.try_recv() {
            self.view.session_mut().on_push(&push);
        }
        let result = match command {
            "look" => Ok(()),
            "key" | "keys" => self.keys(&words[1..]),
            "type" => {
                self.type_text(&unescape(&rest));
                Ok(())
            }
            "go" => self.go(&rest),
            "focus" => self.target(&rest).map(|key| {
                self.view.session_mut().focus_key(&key);
            }),
            "set" => self.set(&rest),
            "do" => match self.session().confirm().map(Confirm::choices) {
                // A confirm's choices are not elements: `do Yes` moves to the choice, then Enter.
                Some(choices) => self.confirm(choices, &rest),
                None => self.target(&rest).map(|key| {
                    self.view.session_mut().focus_key(&key);
                    self.press(KeyPress::plain(Key::Enter));
                }),
            },
            "find" => {
                self.press(KeyPress::plain(Key::Char('/')));
                self.type_text(&rest);
                self.press(KeyPress::plain(Key::Enter));
                let message = self.session().message().to_string();
                if message.starts_with("Found") {
                    Ok(())
                } else {
                    Err(message)
                }
            }
            "elements" => {
                let text = self.elements_text();
                return if json {
                    self.answer(Ok(()), true)
                } else {
                    AgentAnswer {
                        ok: true,
                        text: format!("{text}\n--- ok"),
                    }
                };
            }
            "size" => self.size(&rest),
            "format" => match rest.as_str() {
                "json" => {
                    self.json = true;
                    Ok(())
                }
                "text" => {
                    self.json = false;
                    Ok(())
                }
                _ => Err("format is text or json".to_string()),
            },
            "help" => {
                let map = self.session().keys();
                let keys = ACTIONS
                    .into_iter()
                    .filter_map(|action| {
                        let chords: Vec<String> = map.chords(action).iter().map(|press| press.label()).collect();
                        (!chords.is_empty()).then(|| format!("  {:<16} {}", action.title(), chords.join(", ")))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                return AgentAnswer {
                    ok: true,
                    text: format!("{AGENT_HELP}\n\nKeys on this screen:\n{keys}\n--- ok"),
                };
            }
            "close" | "quit" => {
                self.closed = true;
                return AgentAnswer {
                    ok: true,
                    text: "closed\n--- ok".to_string(),
                };
            }
            other => Err(format!("unknown command {other}; try help")),
        };
        self.settle();
        self.answer(result, json)
    }

    /// Choose `wanted` in the open confirm dialog, by its text, and press Enter.
    fn confirm(&mut self, choices: &[&str], wanted: &str) -> Result<(), String> {
        let lower = wanted.to_lowercase();
        let Some(at) = choices.iter().position(|choice| choice.to_lowercase() == lower) else {
            return Err(format!("the confirm's choices are {}", choices.join(", ")));
        };
        for _ in 0..choices.len() {
            if self.session().confirm_choice() == Some(choices[at]) {
                break;
            }
            self.press(KeyPress::plain(Key::Right));
        }
        self.press(KeyPress::plain(Key::Enter));
        Ok(())
    }

    fn press(&mut self, press: KeyPress) {
        if self.view.on_key(&press) {
            self.closed = true;
        }
        self.settle();
    }

    fn keys(&mut self, chords: &[&str]) -> Result<(), String> {
        if chords.is_empty() {
            return Err("key needs at least one key".to_string());
        }
        let presses: Vec<KeyPress> = chords
            .iter()
            .map(|chord| KeyPress::parse(chord).ok_or_else(|| format!("no key {chord}")))
            .collect::<Result<_, _>>()?;
        for press in presses {
            self.press(press);
            if self.closed {
                break;
            }
        }
        Ok(())
    }

    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            let key = match ch {
                '\r' | '\n' => Key::Enter,
                '\t' => Key::Tab,
                other => Key::Char(other),
            };
            self.press(KeyPress::plain(key));
        }
    }

    fn go(&mut self, path: &str) -> Result<(), String> {
        if !path.starts_with('/') {
            return Err("go takes a path from the root, as /apps/...".to_string());
        }
        self.view.session_mut().navigate(path, true);
        Ok(())
    }

    fn size(&mut self, text: &str) -> Result<(), String> {
        let (columns, rows) = text
            .split_once('x')
            .and_then(|(columns, rows)| Some((columns.parse::<u16>().ok()?, rows.parse::<u16>().ok()?)))
            .filter(|(columns, rows)| (40..=500).contains(columns) && (10..=500).contains(rows))
            .ok_or_else(|| "size is <cols>x<rows>, 40 to 500 by 10 to 500".to_string())?;
        self.columns = columns;
        self.rows = rows;
        self.view.session_mut().resize(columns, rows);
        Ok(())
    }

    /// `set <element> <value>`: the element is the longest leading run of words that names one, so a label with spaces
    /// works without quotes.
    fn set(&mut self, rest: &str) -> Result<(), String> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.len() < 2 {
            return Err("set takes an element and a value".to_string());
        }
        let (key, value) = (1..words.len())
            .rev()
            .find_map(|split| {
                let key = self.resolve(&words[..split].join(" "))?;
                Some((key, words[split..].join(" ")))
            })
            .ok_or_else(|| format!("no element named by {rest:?}; see elements"))?;
        let kind = self.item(&key).map(|item| item.kind);
        self.view.session_mut().focus_key(&key);
        match kind {
            Some(FocusKind::Text) => {
                self.view.session_mut().set_buffer(&key, value);
                Ok(())
            }
            Some(FocusKind::DropDown) => self.choose(&key, &value),
            Some(FocusKind::Check) => {
                if self.session().buffer(&key) != value {
                    self.press(KeyPress::plain(Key::Char(' ')));
                }
                Ok(())
            }
            _ => Err(format!("{key} is not a field to set; use do or key")),
        }
    }

    /// A drop-down's choice, as a person makes it: F4, the text to narrow the list, Enter. Then the value it took is
    /// checked against what was asked.
    fn choose(&mut self, key: &str, wanted: &str) -> Result<(), String> {
        self.press(KeyPress::plain(Key::F(4)));
        if !matches!(self.session().top_overlay(), Some(Overlay::DropDown(_))) {
            return Err(format!("{key} did not open its choices"));
        }
        self.type_text(wanted);
        let choices = self.session().dropdown_choices();
        let lower = wanted.to_lowercase();
        let Some(first) = choices.first().cloned() else {
            self.press(KeyPress::plain(Key::Escape));
            return Err(format!("no choice of {key} matches {wanted:?}"));
        };
        let exact = choices
            .iter()
            .position(|choice| choice.value == wanted || choice.label.to_lowercase() == lower);
        for _ in 0..exact.unwrap_or(0) {
            self.press(KeyPress::plain(Key::Down));
        }
        self.press(KeyPress::plain(Key::Enter));
        let took = self.session().buffer(key).to_string();
        let chosen = exact.and_then(|at| choices.get(at)).unwrap_or(&first);
        if took == chosen.value {
            Ok(())
        } else {
            Err(format!("{key} took {took:?}, not {wanted:?}"))
        }
    }

    fn item(&self, key: &str) -> Option<&FocusItem> {
        self.session().focus_order().iter().find(|item| item.key == key)
    }

    fn target(&self, text: &str) -> Result<String, String> {
        if text.is_empty() {
            return Err("name an element: its id, #n from elements, or its label".to_string());
        }
        self.resolve(text)
            .ok_or_else(|| format!("no element named {text:?}; see elements"))
    }

    /// An element by `#n`, by its key, or by its label: the same label first, then one that holds the text.
    fn resolve(&self, text: &str) -> Option<String> {
        let order = self.session().focus_order();
        if let Some(n) = text.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()) {
            return order.get(n.checked_sub(1)?).map(|item| item.key.clone());
        }
        if let Some(item) = order.iter().find(|item| item.key == text) {
            return Some(item.key.clone());
        }
        let lower = text.to_lowercase();
        let labelled: Vec<(String, String)> = order
            .iter()
            .map(|item| (item.key.clone(), self.label(item).to_lowercase()))
            .collect();
        let tidy = |label: &str| label.trim_end_matches('…').trim().to_string();
        labelled
            .iter()
            .find(|(_, label)| tidy(label) == lower)
            .or_else(|| labelled.iter().find(|(key, _)| key.to_lowercase() == lower))
            .or_else(|| labelled.iter().find(|(_, label)| label.contains(&lower)))
            .map(|(key, _)| key.clone())
    }

    /// What an element says: a field's title, a link's or button's text, a dialog's title, a menu's text, else the text
    /// of its node.
    fn label(&self, item: &FocusItem) -> String {
        let index = self.session().index();
        let part = item.key.split_once('#').map(|(_, part)| part);
        let found = match item.kind {
            FocusKind::Text | FocusKind::DropDown | FocusKind::Check | FocusKind::Radio => {
                index.fields.get(&item.node).map(|field| match part {
                    Some(part) => format!("{} ({part})", field.title),
                    None => field.title.clone(),
                })
            }
            FocusKind::Link | FocusKind::Button => index.links.get(&item.key).map(|link| link.text.clone()),
            FocusKind::Dialog => index.dialogs.get(&item.key).cloned(),
            FocusKind::Menu => index.menus.get(&item.key).map(|menu| menu.text.clone()),
            _ => None,
        };
        found
            .filter(|label| !label.is_empty())
            .or_else(|| self.node_text(&item.node))
            .unwrap_or_else(|| item.key.clone())
    }

    fn node_text(&self, id: &str) -> Option<String> {
        let mut found = None;
        for model in self.session().nav().models() {
            walk(&model.c, &mut |node| {
                if found.is_none() && node.id() == id {
                    found = serde_json::to_value(node).ok();
                }
            });
        }
        let node = found?;
        ["text", "button", "title", "name"]
            .into_iter()
            .find_map(|name| node.get(name).and_then(Value::as_str).filter(|text| !text.is_empty()))
            .map(str::to_string)
    }

    fn kind_name(kind: FocusKind) -> &'static str {
        match kind {
            FocusKind::Text => "text",
            FocusKind::DropDown => "drop-down",
            FocusKind::Check => "check",
            FocusKind::Radio => "radio",
            FocusKind::Button => "button",
            FocusKind::Link => "link",
            FocusKind::List => "list",
            FocusKind::Dialog => "dialog",
            FocusKind::Menu => "menu",
            FocusKind::Tree => "tree",
            FocusKind::Toggle => "toggle",
            FocusKind::Edit => "edit",
            FocusKind::Component => "component",
        }
    }

    fn element(&self, n: usize, item: &FocusItem) -> Value {
        let session = self.session();
        let value = match item.kind {
            FocusKind::Text | FocusKind::DropDown | FocusKind::Check | FocusKind::Radio => {
                Some(session.buffer(&item.key).to_string())
            }
            _ => None,
        };
        let mut element = json!({
            "n": n,
            "kind": Self::kind_name(item.kind),
            "id": item.key,
            "label": self.label(item),
            "focused": item.key == session.focus_id(),
        });
        if let Some(value) = value {
            element["value"] = json!(value);
            if session.is_dirty(&item.key) {
                element["changed"] = json!(true);
            }
            if let Some(mark) = session.mark(&item.key) {
                element["error"] = json!(mark);
            }
            if let Some(field) = session.index().fields.get(&item.node) {
                if field.required {
                    element["required"] = json!(true);
                }
            }
        }
        if item.kind == FocusKind::List {
            if let Some(list) = session.list(&item.key) {
                element["rows"] = json!(list.rows.len());
            }
        }
        element
    }

    fn elements(&self) -> Vec<Value> {
        self.session()
            .focus_order()
            .iter()
            .enumerate()
            .map(|(at, item)| self.element(at + 1, item))
            .collect()
    }

    fn elements_text(&self) -> String {
        self.elements()
            .iter()
            .map(|element| {
                let mut line = format!(
                    "{}#{:<3} {:<9} {:<28} {}",
                    if element["focused"] == json!(true) { ">" } else { " " },
                    element["n"],
                    element["kind"].as_str().unwrap_or(""),
                    element["id"].as_str().unwrap_or(""),
                    element["label"].as_str().unwrap_or("")
                );
                if let Some(value) = element.get("value").and_then(Value::as_str) {
                    line.push_str(&format!(" = {value:?}"));
                }
                for flag in ["required", "changed"] {
                    if element.get(flag).is_some() {
                        line.push_str(&format!(" [{flag}]"));
                    }
                }
                if let Some(error) = element.get("error").and_then(Value::as_str) {
                    line.push_str(&format!(" [error: {error}]"));
                }
                if let Some(rows) = element.get("rows") {
                    line.push_str(&format!(" ({rows} rows)"));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Run what the keys queued, and the waits for typing to stop, until nothing is left to do.
    fn settle(&mut self) {
        for _ in 0..16 {
            self.view.run_requests();
            if !self.view.session_mut().fire_waits() {
                break;
            }
        }
    }

    fn overlay(&self) -> Option<String> {
        let session = self.session();
        let overlay = session.top_overlay()?;
        Some(match overlay {
            Overlay::Dialog(id) => format!(
                "dialog {:?}",
                session.index().dialogs.get(id).cloned().unwrap_or_else(|| id.clone())
            ),
            Overlay::Find(list) => format!("find {list:?}"),
            Overlay::DropDown(drop) => format!("drop-down {:?} filter {:?}", drop.field, drop.filter),
            Overlay::Picker(picker) => format!("date picker {:?}", picker.key),
            Overlay::Confirm(confirm) => format!(
                "confirm {:?}, choices {}, at {:?}",
                match &confirm.kind {
                    ConfirmKind::Ask(text) => text.as_str(),
                    ConfirmKind::Leave => "leave with unsaved changes?",
                },
                confirm.choices().join("|"),
                confirm.choice()
            ),
            Overlay::Help => "help".to_string(),
            Overlay::Info => "info".to_string(),
            Overlay::Login(_) => "login".to_string(),
            Overlay::Drawer(_) => "drawer".to_string(),
            Overlay::Menu(menu) => format!(
                "menu, at {:?}",
                menu.items.get(menu.at).map(|item| item.label.as_str()).unwrap_or("")
            ),
            Overlay::Palette(palette) => format!("palette {:?}", palette.text),
            Overlay::Search(text) => format!("find text {text:?}"),
            Overlay::Messages => "messages".to_string(),
            Overlay::Editor(_) => "editor".to_string(),
            Overlay::Row(list, row) => format!("row editor {list:?} row {}", row + 1),
            Overlay::Columns(_) => "columns".to_string(),
            Overlay::Prompt(_) => "prompt".to_string(),
        })
    }

    /// The screen as text, with the cells drawn in reverse video (the focus) between `[[` and `]]`.
    fn screen_lines(&self) -> Vec<String> {
        let (columns, rows) = (self.columns, self.rows);
        let mut buffer = Buffer::new(columns, rows);
        {
            let mut ctx = RenderContext::new(&mut buffer, Rect::new(0, 0, columns, rows));
            self.view.render(&mut ctx);
        }
        let mut lines: Vec<String> = (0..rows)
            .map(|y| {
                let mut line = String::new();
                let mut reversed = false;
                for x in 0..columns {
                    let Some(cell) = buffer.get(x, y) else {
                        continue;
                    };
                    if cell.is_continuation() {
                        continue;
                    }
                    let reverse = cell.modifier.contains(Modifier::REVERSE);
                    if reverse != reversed {
                        line.push_str(if reverse { "[[" } else { "]]" });
                        reversed = reverse;
                    }
                    line.push(cell.symbol);
                }
                if reversed {
                    line.push_str("]]");
                }
                line.trim_end().to_string()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    fn answer(&self, result: Result<(), String>, json: bool) -> AgentAnswer {
        let session = self.session();
        let focus = session
            .focus()
            .map(|item| (Self::kind_name(item.kind), item.key.clone(), self.label(item)));
        let message = session.message().to_string();
        let message_error = session.message_is_error();
        let ok = result.is_ok();
        if json {
            let value = json!({
                "ok": ok,
                "reason": result.as_ref().err(),
                "path": session.path(),
                "parameters": session.parameters(),
                "title": session.crumbs().join(" ▸ "),
                "focus": focus.as_ref().map(|(kind, id, label)| json!({"kind": kind, "id": id, "label": label})),
                "overlay": self.overlay(),
                "message": message,
                "messageError": message_error,
                "screen": self.screen_lines(),
                "elements": self.elements(),
            });
            return AgentAnswer {
                ok,
                text: value.to_string(),
            };
        }
        let mut out = Vec::new();
        out.push(format!(
            "path {}",
            path_with_query(session.path(), session.parameters())
        ));
        out.push(format!("title {}", session.crumbs().join(" ▸ ")));
        out.push(match &focus {
            Some((kind, id, label)) => format!("focus {kind} {id} {label:?}"),
            None => "focus none".to_string(),
        });
        out.push(format!(
            "overlay {}",
            self.overlay().unwrap_or_else(|| "none".to_string())
        ));
        if !message.is_empty() {
            out.push(format!("{} {message}", if message_error { "error" } else { "message" }));
        }
        out.extend(collapse_blank(self.screen_lines()));
        out.push(match result {
            Ok(()) => "--- ok".to_string(),
            Err(reason) => format!("--- error {reason}"),
        });
        AgentAnswer {
            ok,
            text: out.join("\n"),
        }
    }
}

/// Runs of three or more empty rows as one line that counts them: they say nothing and cost an agent tokens. JSON
/// keeps every row, so a program can index them.
fn collapse_blank(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut blank = 0;
    let flush = |out: &mut Vec<String>, blank: usize| match blank {
        0 => {}
        1 | 2 => out.extend(std::iter::repeat_n(String::new(), blank)),
        n => out.push(format!("  ({n} empty rows)")),
    };
    for line in lines {
        if line.is_empty() {
            blank += 1;
            continue;
        }
        flush(&mut out, blank);
        blank = 0;
        out.push(line);
    }
    flush(&mut out, blank);
    out
}

fn path_with_query(path: &str, parameters: &BTreeMap<String, String>) -> String {
    if parameters.is_empty() {
        return path.to_string();
    }
    let query: Vec<String> = parameters
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{path}?{}", query.join("&"))
}

/// `\r`, `\n`, `\t` and `\\` in a `type` line.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}
