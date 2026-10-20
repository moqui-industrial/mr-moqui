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

//! The fields, forms, lists, links and dialogs of the levels on screen, found once per model change, with what a form
//! posts and the checks it makes before a submit (plans/Screen-TUI-Impl.md 1.7, 2.3).
//!
//! A field's value is in the edit buffers under its id. A find field has parts, under `id#part`: a date or range find
//! `#from` and `#thru`, a text find `#op`, `#not` and `#ic`, a drop-down with `show-not` `#not`.

use std::collections::BTreeMap;

use crate::model::{walk, Choice, FormList, Node, Rule, ScreenModel, VisibleWhen, Widget};

/// Which overlay a focusable item belongs to. A dialog or the find dialog of a list keeps focus inside it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    Main,
    Dialog(String),
    Find(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusKind {
    /// A text input: text line, text area, password, date, a find part.
    Text,
    DropDown,
    Check,
    Radio,
    Button,
    Link,
    List,
    /// The button that opens a dialog.
    Dialog,
}

/// One item of the focus order, which is the reading order of the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusItem {
    /// The node id, or `field id#part` for a part of a find field.
    pub key: String,
    /// The node the item is drawn in: the field of a part.
    pub node: String,
    pub kind: FocusKind,
    pub scope: Scope,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldInfo {
    pub id: String,
    pub name: String,
    pub title: String,
    /// The form the field posts with, or a list whose header (find) field it is.
    pub form: Option<String>,
    pub list: Option<String>,
    pub required: bool,
    pub maxlength: u32,
    pub rules: Vec<Rule>,
    pub tooltip: String,
    pub visible_when: Option<VisibleWhen>,
    pub widget: Widget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormInfo {
    pub id: String,
    pub name: String,
    pub transition_url: String,
    pub method: String,
    pub hidden: Vec<(String, String)>,
    pub submit_message: String,
    pub background_reload_id: String,
    pub background_hide_id: String,
    /// Field ids in document order.
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    pub id: String,
    pub text: String,
    pub url: String,
    pub kind: String,
    pub method: String,
    pub confirm: String,
    pub disabled: bool,
}

impl LinkInfo {
    /// A link that runs a transition rather than opening a screen: a hidden-form link posts, as `m-form-link` does.
    pub fn posts(&self) -> bool {
        self.kind == "hidden-form" || self.kind == "hidden-form-link" || self.method.eq_ignore_ascii_case("post")
    }
}

/// A subscreen tab, for Alt+1 to Alt+9.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub fields: BTreeMap<String, FieldInfo>,
    pub forms: BTreeMap<String, FormInfo>,
    pub lists: BTreeMap<String, FormList>,
    pub links: BTreeMap<String, LinkInfo>,
    /// Link ids in reading order, row links included: link `[n]` of `#` numbering is `link_order[n - 1]`.
    pub link_order: Vec<String>,
    /// Dialog id to title.
    pub dialogs: BTreeMap<String, String>,
    pub tabs: Vec<TabInfo>,
    /// Every focusable item in reading order, in every scope.
    pub order: Vec<FocusItem>,
}

impl Index {
    /// Index the levels from the outermost to the leaf.
    pub fn of<'a>(levels: impl IntoIterator<Item = &'a ScreenModel>) -> Self {
        let mut index = Self::default();
        for model in levels {
            index.add(&model.c, &Scope::Main, None, None);
        }
        index
    }

    fn add(&mut self, nodes: &[Node], scope: &Scope, form: Option<&str>, list: Option<&str>) {
        for node in nodes {
            match node {
                Node::Form {
                    id,
                    name,
                    transition_url,
                    method,
                    hidden,
                    submit_message,
                    background_reload_id,
                    background_hide_id,
                    c,
                    ..
                } => {
                    let mut fields = Vec::new();
                    walk(c, &mut |inner| {
                        if let Node::Field { id, .. } = inner {
                            fields.push(id.clone());
                        }
                    });
                    self.forms.insert(
                        id.clone(),
                        FormInfo {
                            id: id.clone(),
                            name: name.clone(),
                            transition_url: transition_url.clone(),
                            method: if method.is_empty() {
                                "post".to_string()
                            } else {
                                method.clone()
                            },
                            hidden: hidden
                                .iter()
                                .map(|item| (item.name.clone(), item.value.clone()))
                                .collect(),
                            submit_message: submit_message.clone(),
                            background_reload_id: background_reload_id.clone(),
                            background_hide_id: background_hide_id.clone(),
                            fields,
                        },
                    );
                    self.add(c, scope, Some(id), None);
                }
                Node::Field {
                    id,
                    name,
                    title,
                    required,
                    maxlength,
                    rules,
                    tooltip,
                    visible_when,
                    w,
                } => {
                    self.fields.insert(
                        id.clone(),
                        FieldInfo {
                            id: id.clone(),
                            name: name.clone(),
                            title: title.clone(),
                            form: form.map(str::to_string),
                            list: list.map(str::to_string),
                            required: *required,
                            maxlength: *maxlength,
                            rules: rules.clone(),
                            tooltip: tooltip.clone(),
                            visible_when: visible_when.clone(),
                            widget: w.clone(),
                        },
                    );
                    for (key, kind) in focus_parts(id, w) {
                        self.order.push(FocusItem {
                            key,
                            node: id.clone(),
                            kind,
                            scope: scope.clone(),
                        });
                    }
                }
                Node::Link {
                    id,
                    text,
                    url,
                    kind,
                    method,
                    confirm,
                    disabled,
                    ..
                } => {
                    self.add_link(id, text, url, kind, method, confirm, *disabled);
                    if !disabled {
                        self.push(id, FocusKind::Link, scope);
                    }
                }
                Node::FormList { list: formlist } => {
                    self.lists.insert(formlist.id.clone(), formlist.clone());
                    self.push(&formlist.id, FocusKind::List, scope);
                    for row in &formlist.rows {
                        for cell in &row.cells {
                            if let crate::model::Cell::Node(inner) = cell {
                                if let Node::Link {
                                    id,
                                    text,
                                    url,
                                    kind,
                                    method,
                                    confirm,
                                    disabled,
                                    ..
                                } = inner.as_ref()
                                {
                                    self.add_link(id, text, url, kind, method, confirm, *disabled);
                                }
                            }
                        }
                    }
                    self.add(
                        &formlist.header_fields,
                        &Scope::Find(formlist.id.clone()),
                        None,
                        Some(&formlist.id),
                    );
                    self.add(&formlist.c, scope, form, list);
                }
                Node::Dialog { id, title, c, .. } => {
                    self.dialogs.insert(id.clone(), title.clone());
                    self.push(id, FocusKind::Dialog, scope);
                    self.add(c, &Scope::Dialog(id.clone()), form, list);
                }
                Node::Subscreen { items, c, .. } => {
                    // The deepest tab bar wins: Alt+N picks among the tabs nearest the content.
                    let tabs: Vec<TabInfo> = items
                        .iter()
                        .map(|item| TabInfo {
                            title: item.title.clone(),
                            url: item.url.clone(),
                            active: item.active,
                        })
                        .collect();
                    if tabs.len() > 1 {
                        self.tabs = tabs;
                    }
                    self.add(c, scope, form, list);
                }
                other => self.add(other.children(), scope, form, list),
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn add_link(&mut self, id: &str, text: &str, url: &str, kind: &str, method: &str, confirm: &str, disabled: bool) {
        if !disabled && !url.is_empty() && !self.links.contains_key(id) {
            self.link_order.push(id.to_string());
        }
        self.links.insert(
            id.to_string(),
            LinkInfo {
                id: id.to_string(),
                text: text.to_string(),
                url: url.to_string(),
                kind: kind.to_string(),
                method: method.to_string(),
                confirm: confirm.to_string(),
                disabled,
            },
        );
    }

    fn push(&mut self, id: &str, kind: FocusKind, scope: &Scope) {
        self.order.push(FocusItem {
            key: id.to_string(),
            node: id.to_string(),
            kind,
            scope: scope.clone(),
        });
    }

    /// The form a field or button posts with.
    pub fn form_of(&self, field_id: &str) -> Option<&FormInfo> {
        self.fields
            .get(field_id)
            .and_then(|field| field.form.as_ref())
            .and_then(|form| self.forms.get(form))
            .or_else(|| self.forms.get(field_id))
    }

    /// The values every field starts with, by buffer key.
    pub fn initial_values(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for field in self.fields.values() {
            for (key, value) in initial_parts(&field.id, &field.widget) {
                out.insert(key, value);
            }
        }
        out
    }

    /// Whether a field shows: its `visible-when` field (in the same form) has the value it names.
    pub fn visible(&self, field_id: &str, values: &dyn Fn(&str) -> String) -> bool {
        let Some(field) = self.fields.get(field_id) else {
            return true;
        };
        let Some(when) = &field.visible_when else {
            return true;
        };
        let other = self
            .fields
            .values()
            .find(|other| other.name == when.field && other.form == field.form);
        let value = other.map(|other| values(&other.id)).unwrap_or_default();
        if when.value.is_empty() {
            !value.is_empty()
        } else {
            value.split(',').any(|part| part.trim() == when.value)
        }
    }

    /// The field ids whose `visible-when` names `name`.
    pub fn watched_by(&self, name: &str) -> bool {
        self.fields
            .values()
            .any(|field| field.visible_when.as_ref().is_some_and(|when| when.field == name))
    }

    /// The drop-downs that depend on the field `name`.
    pub fn dependents(&self, name: &str) -> Vec<String> {
        self.fields
            .values()
            .filter(|field| match &field.widget {
                Widget::DropDown {
                    options_url,
                    depends_on,
                    ..
                } => !options_url.is_empty() && depends_on.values().any(|dep| dep == name),
                _ => false,
            })
            .map(|field| field.id.clone())
            .collect()
    }

    /// The url and parameters of a drop-down's options: its fixed parameters, then each depends-on field's value.
    pub fn option_query(
        &self,
        field_id: &str,
        values: &dyn Fn(&str) -> String,
    ) -> Option<(String, BTreeMap<String, String>)> {
        let field = self.fields.get(field_id)?;
        let Widget::DropDown {
            options_url,
            options_parameters,
            depends_on,
            ..
        } = &field.widget
        else {
            return None;
        };
        if options_url.is_empty() {
            return None;
        }
        let mut params = options_parameters.clone();
        for (parameter, name) in depends_on {
            if let Some(other) = self
                .fields
                .values()
                .find(|other| &other.name == name && other.form == field.form)
            {
                params.insert(parameter.clone(), values(&other.id));
            }
        }
        Some((options_url.clone(), params))
    }

    pub fn choices(&self, field_id: &str) -> &[Choice] {
        match self.fields.get(field_id).map(|field| &field.widget) {
            Some(Widget::DropDown { options, .. } | Widget::Radio { options, .. } | Widget::Check { options, .. }) => {
                options
            }
            _ => &[],
        }
    }

    pub fn set_choices(&mut self, field_id: &str, choices: Vec<Choice>) {
        if let Some(FieldInfo {
            widget: Widget::DropDown { options, .. },
            ..
        }) = self.fields.get_mut(field_id)
        {
            *options = choices;
        }
    }
}

/// The focusable parts of a field: one item, two for a from/thru find, none for a display or hidden field.
fn focus_parts(id: &str, widget: &Widget) -> Vec<(String, FocusKind)> {
    let one = |kind| vec![(id.to_string(), kind)];
    match widget {
        Widget::TextLine { .. } | Widget::TextFind { .. } | Widget::DateTime { .. } | Widget::Password {} => {
            one(FocusKind::Text)
        }
        Widget::TextArea { read_only, .. } => {
            if *read_only {
                Vec::new()
            } else {
                one(FocusKind::Text)
            }
        }
        Widget::DateFind { .. } | Widget::RangeFind { .. } | Widget::DatePeriod { .. } => vec![
            (format!("{id}#from"), FocusKind::Text),
            (format!("{id}#thru"), FocusKind::Text),
        ],
        Widget::DropDown { .. } => one(FocusKind::DropDown),
        Widget::Check { .. } => one(FocusKind::Check),
        Widget::Radio { .. } => one(FocusKind::Radio),
        Widget::Submit { .. } | Widget::Reset { .. } => one(FocusKind::Button),
        Widget::Display { .. }
        | Widget::Hidden { .. }
        | Widget::File { .. }
        | Widget::Label { .. }
        | Widget::Image { .. } => Vec::new(),
    }
}

/// The buffer keys and starting values of a field.
fn initial_parts(id: &str, widget: &Widget) -> Vec<(String, String)> {
    let main = |value: &str| vec![(id.to_string(), value.to_string())];
    let part = |name: &str, value: &str| (format!("{id}#{name}"), value.to_string());
    let yes = |flag: bool| if flag { "Y" } else { "N" };
    match widget {
        Widget::TextLine { value, .. }
        | Widget::TextArea { value, .. }
        | Widget::DateTime { value, .. }
        | Widget::Check { value, .. }
        | Widget::Radio { value, .. }
        | Widget::Hidden { value } => main(value),
        Widget::Password {} => main(""),
        Widget::TextFind { value, op, not, ic, .. } => vec![
            (id.to_string(), value.clone()),
            part("op", if op.is_empty() { "contains" } else { op }),
            part("not", yes(*not)),
            part("ic", yes(*ic)),
        ],
        Widget::DropDown {
            value, show_not, not, ..
        } => {
            let mut out = main(value);
            if *show_not {
                out.push(part("not", yes(*not)));
            }
            out
        }
        Widget::DateFind { from, thru, .. }
        | Widget::RangeFind { from, thru }
        | Widget::DatePeriod { from, thru, .. } => vec![part("from", from), part("thru", thru)],
        Widget::Display { value, .. } => value.as_deref().map(main).unwrap_or_default(),
        Widget::Submit { .. }
        | Widget::Reset { .. }
        | Widget::File { .. }
        | Widget::Label { .. }
        | Widget::Image { .. } => Vec::new(),
    }
}

/// The request parameters of one field, from its buffers.
pub fn field_parameters(field: &FieldInfo, values: &dyn Fn(&str) -> String) -> Vec<(String, String)> {
    let id = &field.id;
    let name = &field.name;
    let part = |suffix: &str| values(&format!("{id}#{suffix}"));
    match &field.widget {
        Widget::TextLine { .. }
        | Widget::DateTime { .. }
        | Widget::Check { .. }
        | Widget::Radio { .. }
        | Widget::Hidden { .. }
        | Widget::Password {} => vec![(name.clone(), values(id))],
        Widget::TextArea { read_only, .. } => {
            if *read_only {
                Vec::new()
            } else {
                vec![(name.clone(), values(id))]
            }
        }
        Widget::TextFind { .. } => vec![
            (name.clone(), values(id)),
            (format!("{name}_op"), part("op")),
            (format!("{name}_not"), part("not")),
            (format!("{name}_ic"), part("ic")),
        ],
        Widget::DropDown { show_not, multiple, .. } => {
            let mut out = vec![(name.clone(), values(id))];
            if *multiple && values(id).contains(',') {
                out.push((format!("{name}_op"), "in".to_string()));
            }
            if *show_not {
                out.push((format!("{name}_not"), part("not")));
            }
            out
        }
        Widget::DateFind { .. } | Widget::RangeFind { .. } | Widget::DatePeriod { .. } => vec![
            (format!("{name}_from"), part("from")),
            (format!("{name}_thru"), part("thru")),
        ],
        Widget::Display { value, .. } => {
            if value.is_some() {
                vec![(name.clone(), values(id))]
            } else {
                Vec::new()
            }
        }
        Widget::Submit { .. }
        | Widget::Reset { .. }
        | Widget::File { .. }
        | Widget::Label { .. }
        | Widget::Image { .. } => Vec::new(),
    }
}

/// The buffer keys of a field, so a save drops exactly its edits.
pub fn field_keys(field: &FieldInfo) -> Vec<String> {
    initial_parts(&field.id, &field.widget)
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// What a form posts: its hidden values, then each shown field. An empty optional value the user did not touch is left
/// out, so an untouched empty field does not write "" into a foreign key.
pub fn posted(
    index: &Index,
    field_ids: &[String],
    hidden: &[(String, String)],
    values: &dyn Fn(&str) -> String,
    dirty: &dyn Fn(&str) -> bool,
) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = hidden.iter().cloned().collect();
    for id in field_ids {
        let Some(field) = index.fields.get(id) else {
            continue;
        };
        if !index.visible(id, values) {
            continue;
        }
        let keys = field_keys(field);
        let touched = keys.iter().any(|key| dirty(key));
        for (name, value) in field_parameters(field, values) {
            if value.is_empty() && !field.required && !touched {
                continue;
            }
            out.insert(name, value);
        }
    }
    out
}

/// The checks of section 1.7 made before a submit, as `(buffer key, message)`: required, length, the field's rules, and
/// a date that does not read in its format.
pub fn check(index: &Index, field_ids: &[String], values: &dyn Fn(&str) -> String) -> Vec<(String, String)> {
    let mut marks = Vec::new();
    for id in field_ids {
        let Some(field) = index.fields.get(id) else {
            continue;
        };
        if !index.visible(id, values) || matches!(field.widget, Widget::Display { .. } | Widget::Hidden { .. }) {
            continue;
        }
        let value = values(id);
        if field.required && value.trim().is_empty() && !matches!(field.widget, Widget::Submit { .. }) {
            marks.push((id.clone(), "A value is required".to_string()));
            continue;
        }
        let limit = usize::try_from(field.maxlength).unwrap_or(usize::MAX);
        if field.maxlength > 0 && value.chars().count() > limit {
            marks.push((id.clone(), format!("At most {limit} characters")));
            continue;
        }
        if let Some(message) = field.rules.iter().find_map(|rule| broken_rule(rule, &value)) {
            marks.push((id.clone(), message));
            continue;
        }
        if let Widget::DateTime { format, date_type, .. } = &field.widget {
            if !value.is_empty() && !date_reads(&value, format, date_type) {
                marks.push((
                    id.clone(),
                    format!("Not a date in the form {}", shown_format(format, date_type)),
                ));
            }
        }
    }
    marks
}

fn broken_rule(rule: &Rule, value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let message = |text: &str, fallback: &str| {
        if text.is_empty() {
            fallback.to_string()
        } else {
            text.to_string()
        }
    };
    match rule {
        Rule::Number { message: text } => parse_number(value)
            .is_none()
            .then(|| message(text, "Please enter a valid number")),
        Rule::Integer { message: text } => {
            let whole = value.trim().trim_start_matches(['-', '+']);
            (whole.is_empty() || !whole.chars().all(|ch| ch.is_ascii_digit() || ch == ','))
                .then(|| message(text, "Please enter a valid whole number"))
        }
        Rule::Pattern { pattern, message: text } => match regex::Regex::new(pattern) {
            Ok(re) => (!re.is_match(value)).then(|| message(text, "The value is not in the right form")),
            // A JavaScript pattern this engine does not read: the server checks it again.
            Err(_) => None,
        },
        Rule::Range {
            min,
            min_eq,
            max,
            max_eq,
            message: text,
        } => {
            let Some(number) = parse_number(value) else {
                return Some(message(text, "Please enter a valid number"));
            };
            let bound = |text: &Option<String>| text.as_deref().and_then(parse_number);
            let low = bound(min).is_some_and(|low| if *min_eq { number < low } else { number <= low });
            let high = bound(max).is_some_and(|high| if *max_eq { number > high } else { number >= high });
            (low || high).then(|| message(text, "The number is out of range"))
        }
        Rule::Other => None,
    }
}

/// A number as a person types one: digits with grouping commas, a point, a sign.
fn parse_number(text: &str) -> Option<f64> {
    text.trim().replace(',', "").parse::<f64>().ok()
}

/// The Java date pattern as strftime, or None when it has a letter this does not map.
fn strftime_of(format: &str) -> Option<String> {
    let mut out = String::new();
    let chars: Vec<char> = format.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        let mut run = 1;
        while index + run < chars.len() && chars[index + run] == ch {
            run += 1;
        }
        let piece = match (ch, run) {
            ('y', 4) => "%Y",
            ('y', 2) => "%y",
            ('M', 2) => "%m",
            ('M', 3) => "%b",
            ('d', 2) => "%d",
            ('H', 2) => "%H",
            ('h', 2) => "%I",
            ('m', 2) => "%M",
            ('s', 2) => "%S",
            ('a', 1) => "%p",
            (letter, _) if letter.is_ascii_alphabetic() => return None,
            _ => "",
        };
        if piece.is_empty() {
            for _ in 0..run {
                out.push(ch);
            }
        } else {
            out.push_str(piece);
        }
        index += run;
    }
    Some(out)
}

fn date_pattern(format: &str, date_type: &str) -> String {
    if !format.is_empty() {
        return format.to_string();
    }
    match date_type {
        "date" => "yyyy-MM-dd".to_string(),
        "time" => "HH:mm".to_string(),
        _ => "yyyy-MM-dd HH:mm".to_string(),
    }
}

pub fn shown_format(format: &str, date_type: &str) -> String {
    date_pattern(format, date_type)
}

/// Whether `value` reads as a date, time or timestamp in `format` (a Java pattern). A pattern this does not map is
/// not checked here; the server checks it.
pub fn date_reads(value: &str, format: &str, date_type: &str) -> bool {
    let pattern = date_pattern(format, date_type);
    let Some(strf) = strftime_of(&pattern) else {
        return true;
    };
    let value = value.trim();
    match date_type {
        "date" => jiff::civil::Date::strptime(&strf, value).is_ok(),
        "time" => jiff::civil::Time::strptime(&strf, value).is_ok(),
        _ => {
            jiff::civil::DateTime::strptime(&strf, value).is_ok()
                // A timestamp field also takes a date alone, as the server does.
                || jiff::civil::Date::strptime("%Y-%m-%d", value).is_ok()
        }
    }
}

/// The date a field's text names, for the picker to start at; today when it names none.
pub fn date_of(value: &str, format: &str, date_type: &str) -> jiff::civil::Date {
    let pattern = date_pattern(format, date_type);
    let parsed = strftime_of(&pattern).and_then(|strf| match date_type {
        "date" => jiff::civil::Date::strptime(&strf, value.trim()).ok(),
        "time" => None,
        _ => jiff::civil::DateTime::strptime(&strf, value.trim())
            .map(|dt| dt.date())
            .ok()
            .or_else(|| jiff::civil::Date::strptime("%Y-%m-%d", value.trim()).ok()),
    });
    parsed.unwrap_or_else(|| jiff::Zoned::now().date())
}

/// `date` written in the field's format; a timestamp keeps the time it had, or midnight.
pub fn date_text(date: jiff::civil::Date, previous: &str, format: &str, date_type: &str) -> String {
    let pattern = date_pattern(format, date_type);
    let Some(strf) = strftime_of(&pattern) else {
        return date.to_string();
    };
    match date_type {
        "date" => date.strftime(&strf).to_string(),
        _ => {
            let time = jiff::civil::DateTime::strptime(&strf, previous.trim())
                .map(|dt| dt.time())
                .unwrap_or(jiff::civil::Time::midnight());
            date.to_datetime(time).strftime(&strf).to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_and_dates_are_checked() {
        let number = Rule::Number { message: String::new() };
        assert_eq!(broken_rule(&number, "1,234.5"), None);
        assert!(broken_rule(&number, "abc").is_some());
        let pattern = Rule::Pattern {
            pattern: r"^\d*$".to_string(),
            message: "digits".to_string(),
        };
        assert_eq!(broken_rule(&pattern, "12a").as_deref(), Some("digits"));
        let range = Rule::Range {
            min: Some("1".to_string()),
            min_eq: true,
            max: Some("10".to_string()),
            max_eq: false,
            message: String::new(),
        };
        assert_eq!(broken_rule(&range, "1"), None);
        assert!(broken_rule(&range, "10").is_some());
        assert!(date_reads("2026-10-20", "", "date"));
        assert!(!date_reads("2026-13-40", "", "date"));
        assert!(date_reads("2026-10-20 14:30", "", "date-time"));
        assert!(date_reads("10/20/2026", "MM/dd/yyyy", "date"));
        assert!(
            date_reads("whatever", "EEE d", "date"),
            "an unmapped pattern is the server's to check"
        );
        let day = jiff::civil::date(2026, 10, 21);
        assert_eq!(date_text(day, "2026-10-20 14:30", "", "date-time"), "2026-10-21 14:30");
    }
}
