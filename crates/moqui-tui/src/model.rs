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

//! Screen model subset from `plans/Screen-TUI-Impl.md` section 2.3.
//!
//! Node names match that section (`box`, `formList`, `textLine`). Unknown object fields
//! are ignored so a later key does not break a fixture. An unknown `t` fails. A `v` other than 1 is refused.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Model version this crate reads.
pub const MODEL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    Json(String),
    Version(u32),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(text) => write!(f, "screen model json: {text}"),
            Self::Version(v) => write!(f, "screen model version {v} is not {MODEL_VERSION}"),
        }
    }
}

/// One screen level. The envelope around `c`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScreenModel {
    pub v: u32,
    pub screen: ScreenMeta,
    /// Request parameters. Credential names are already gone.
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    #[serde(default)]
    pub messages: Messages,
    #[serde(default)]
    pub focus: String,
    #[serde(default)]
    pub c: Vec<Node>,
}

impl ScreenModel {
    /// Read a document. Every string is cleaned first ([`clean_text`], D9), and a node or widget of a kind this client
    /// does not know becomes a placeholder, so one new node type does not lose the page.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let mut value: serde_json::Value =
            serde_json::from_str(text).map_err(|err| ModelError::Json(err.to_string()))?;
        clean_value(&mut value);
        let model: Self = serde_json::from_value(value).map_err(|err| ModelError::Json(err.to_string()))?;
        if model.v != MODEL_VERSION {
            return Err(ModelError::Version(model.v));
        }
        Ok(model)
    }
}

/// The `t` values of [`Node`].
const NODE_TYPES: [&str; 29] = [
    "section",
    "container",
    "box",
    "row",
    "col",
    "dialog",
    "subscreens",
    "active",
    "fragment",
    "region",
    "text",
    "link",
    "form",
    "formList",
    "field",
    "placeholder",
    "error",
    "buttonMenu",
    "panel",
    "split",
    "dashboard",
    "numberCard",
    "statusBar",
    "dynamicDialog",
    "dynamicContainer",
    "tree",
    "image",
    "html",
    "markdown",
];

/// The `kind` values of [`Widget`].
const WIDGET_KINDS: [&str; 18] = [
    "textLine",
    "textArea",
    "textFind",
    "dropDown",
    "dateTime",
    "dateFind",
    "datePeriod",
    "rangeFind",
    "file",
    "display",
    "password",
    "check",
    "radio",
    "hidden",
    "submit",
    "reset",
    "label",
    "image",
];

/// Text as it may be drawn: C0 and C1 controls other than tab and newline, and the bidirectional overrides and
/// isolates, are dropped, so a value cannot drive the terminal or reorder what is shown around it (D9).
pub fn clean_text(text: &str) -> String {
    text.chars().filter(|ch| !is_unsafe(*ch)).collect()
}

fn is_unsafe(ch: char) -> bool {
    let code = u32::from(ch);
    (code < 0x20 && ch != '\t' && ch != '\n')
        || (0x7F..0xA0).contains(&code)
        || (0x202A..=0x202E).contains(&code)
        || (0x2066..=0x2069).contains(&code)
}

fn clean_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if text.chars().any(is_unsafe) {
                *text = clean_text(text);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(clean_value),
        serde_json::Value::Object(map) => {
            if map.keys().any(|key| key.chars().any(is_unsafe)) {
                let entries = std::mem::take(map);
                *map = entries
                    .into_iter()
                    .map(|(key, item)| (clean_text(&key), item))
                    .collect();
            }
            map.values_mut().for_each(clean_value);
            let unknown_node = map
                .get("t")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|t| !NODE_TYPES.contains(&t));
            if unknown_node {
                let t = map
                    .get("t")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let id = map.get("id").cloned().unwrap_or_default();
                *map = serde_json::Map::new();
                map.insert("t".to_string(), "placeholder".into());
                map.insert("id".to_string(), id);
                map.insert("text".to_string(), t.into());
            } else if let Some(serde_json::Value::Object(widget)) = map.get_mut("w") {
                let unknown_widget = widget
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|kind| !WIDGET_KINDS.contains(&kind));
                if unknown_widget {
                    let kind = widget
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    *widget = serde_json::Map::new();
                    widget.insert("kind".to_string(), "display".into());
                    widget.insert("text".to_string(), format!("[{kind}]").into());
                }
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenMeta {
    pub path: String,
    #[serde(default)]
    pub path_list: Vec<String>,
    #[serde(default)]
    pub title: String,
    /// Index of the always-standalone screen in `path_list`. The root screen is 0.
    #[serde(default)]
    pub standalone: i64,
    /// The screen's `render-modes`; `all` when it names none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_modes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Messages {
    #[serde(default)]
    pub info: Vec<String>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub validation_errors: Vec<ValidationError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    #[serde(default)]
    pub form: String,
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub message: String,
}

/// A hidden input posted with a form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HiddenValue {
    pub name: String,
    #[serde(default)]
    pub value: String,
}

/// One tab on a subscreens node.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscreenItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub active: bool,
}

/// A node the client can draw. Names are the `t` values from section 2.3.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Node {
    Section {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        iterate: bool,
        #[serde(default)]
        c: Vec<Node>,
    },
    Container {
        id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Box {
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default, rename = "type")]
        kind: String,
        #[serde(default)]
        initial: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Row {
        id: String,
        #[serde(default)]
        xs: String,
        #[serde(default)]
        sm: String,
        #[serde(default)]
        md: String,
        #[serde(default)]
        lg: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Col {
        id: String,
        #[serde(default)]
        xs: String,
        #[serde(default)]
        sm: String,
        #[serde(default)]
        md: String,
        #[serde(default)]
        lg: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Dialog {
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        button: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    #[serde(rename = "subscreens")]
    Subscreen {
        id: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        items: Vec<SubscreenItem>,
        #[serde(default)]
        c: Vec<Node>,
    },
    Active {
        id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Fragment {
        #[serde(default)]
        id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    /// A named part of its parent: a box `toolbar`; a panel `header`, `left`, `center`, `right` or `footer`; a split
    /// `side` or `main`. `size` is a panel side's width in pixels.
    Region {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "is_zero")]
        size: u32,
        #[serde(default)]
        c: Vec<Node>,
    },
    Text {
        #[serde(default)]
        id: String,
        text: String,
        #[serde(default)]
        style: String,
    },
    #[serde(rename_all = "camelCase")]
    Link {
        id: String,
        text: String,
        url: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        method: String,
        #[serde(default)]
        confirm: String,
        /// `primary`, `success`, `info`, `warning`, `danger`, or empty.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        btn_type: String,
        /// A Material Symbols name.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        icon: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        badge: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tooltip: String,
        /// The user may not follow it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        disabled: bool,
    },
    Form {
        id: String,
        name: String,
        #[serde(default)]
        kind: String,
        #[serde(default, rename = "transitionUrl")]
        transition_url: String,
        #[serde(default)]
        method: String,
        #[serde(default)]
        hidden: Vec<HiddenValue>,
        #[serde(default, rename = "submitMessage")]
        submit_message: String,
        #[serde(default, rename = "backgroundReloadId")]
        background_reload_id: String,
        #[serde(default, rename = "backgroundHideId")]
        background_hide_id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    FormList {
        #[serde(flatten)]
        list: FormList,
    },
    #[serde(rename_all = "camelCase")]
    Field {
        id: String,
        name: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        maxlength: u32,
        /// Checks made before a submit, other than `required`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rules: Vec<Rule>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        tooltip: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_when: Option<VisibleWhen>,
        w: Widget,
    },
    Placeholder {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        url: String,
    },
    Error {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: String,
    },
    ButtonMenu {
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Panel {
        id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    #[serde(rename_all = "camelCase")]
    Split {
        id: String,
        /// The side's width in pixels.
        #[serde(default, skip_serializing_if = "is_zero")]
        side_width: u32,
        /// The parameter that holds the selected record's id.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        select_parameter: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Dashboard {
        id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    NumberCard {
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        url: String,
    },
    StatusBar {
        id: String,
        #[serde(default)]
        text: String,
    },
    DynamicDialog {
        id: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        button: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    DynamicContainer {
        id: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Tree {
        id: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Image {
        id: String,
        #[serde(default)]
        alt: String,
        #[serde(default)]
        url: String,
    },
    /// HTML converted on the server: blocks of text, and the links numbered `[n]` in them.
    Html {
        id: String,
        #[serde(default)]
        blocks: Vec<HtmlBlock>,
        #[serde(default)]
        links: Vec<HtmlLink>,
    },
    Markdown {
        id: String,
        #[serde(default)]
        text: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormList {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub columns: Vec<Column>,
    #[serde(default)]
    pub rows: Vec<ListRow>,
    #[serde(default)]
    pub paging: Paging,
    /// Find fields drawn above the rows, not as a second form.
    #[serde(default)]
    pub header_fields: Vec<Node>,
    /// Rows can be selected, and the selected rows' `select_id`s are posted as `parameter`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_selection: Option<RowSelection>,
    #[serde(default)]
    pub c: Vec<Node>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowSelection {
    #[serde(default)]
    pub id_field: String,
    #[serde(default)]
    pub parameter: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub list_mode: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub title: String,
    /// The `orderByField` value that sorts on this column, empty when it does not sort.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub order_by: String,
    /// `print-width`: characters, or a percent of the line with `print_width_type` `percent`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub print_width: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub print_width_type: String,
    /// `right` for a column of numbers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub align: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListRow {
    /// The row's `id-field` value, else its index on the page.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The row-selection `id-field` value of the row.
    #[serde(default, rename = "selectId", skip_serializing_if = "String::is_empty")]
    pub select_id: String,
    pub cells: Vec<Cell>,
}

/// A list cell is either plain text or one node (a link, in the fixtures).
///
/// The node is boxed so a cell is not the size of every node variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Node(Box<Node>),
}

impl Cell {
    pub fn text(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Node(node) => match node.as_ref() {
                Node::Link { text, .. } | Node::Text { text, .. } => text,
                _ => "",
            },
        }
    }

    pub fn link_url(&self) -> Option<&str> {
        match self {
            Self::Node(node) => match node.as_ref() {
                Node::Link { url, .. } => Some(url),
                _ => None,
            },
            Self::Text(_) => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paging {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_index: Option<u32>,
    /// The 1-based number of the page's first and last row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_low: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_high: Option<u32>,
}

/// A check of a field's value before a submit. A pattern is a JavaScript regular expression; one Rust's `regex` cannot
/// read is skipped by the client, and the server checks it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Rule {
    Number {
        #[serde(default)]
        message: String,
    },
    Integer {
        #[serde(default)]
        message: String,
    },
    Pattern {
        pattern: String,
        #[serde(default)]
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    Range {
        #[serde(default)]
        min: Option<String>,
        #[serde(default)]
        min_eq: bool,
        #[serde(default)]
        max: Option<String>,
        #[serde(default)]
        max_eq: bool,
        #[serde(default)]
        message: String,
    },
    /// A kind this client does not know: not checked here.
    #[serde(other)]
    Other,
}

/// One block of an `html` node: `p`, `h1`…`h6`, `li`, `quote`, `pre`, `row` or `rule`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlBlock {
    pub kind: String,
    #[serde(default)]
    pub text: String,
    /// List or quote nesting.
    #[serde(default)]
    pub depth: u8,
    /// A list item's `•` or `1.`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub marker: String,
}

/// Link `[n]` of an `html` node is `links[n - 1]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlLink {
    #[serde(default)]
    pub text: String,
    pub url: String,
}

/// Show the field only while another field of its form has a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleWhen {
    pub field: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Widget {
    #[serde(rename_all = "camelCase")]
    TextLine {
        #[serde(default)]
        value: String,
        /// `number` right-aligns the value.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        input_type: String,
        /// An Inputmask pattern: `9` a digit, `a` a letter, `*` either; other characters are typed for the user.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        mask: String,
        #[serde(default, skip_serializing_if = "is_zero")]
        size: u32,
    },
    #[serde(rename_all = "camelCase")]
    TextArea {
        #[serde(default)]
        value: String,
        #[serde(default, skip_serializing_if = "is_zero")]
        rows: u32,
        /// `md` is Markdown source; `html`, `rich` and `rich-light` come with `read_only` (D12).
        #[serde(default, skip_serializing_if = "String::is_empty")]
        editor_type: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        read_only: bool,
    },
    /// A text find posts `name`, `name_op`, `name_not` and `name_ic`.
    #[serde(rename_all = "camelCase")]
    TextFind {
        #[serde(default)]
        value: String,
        #[serde(default)]
        op: String,
        #[serde(default)]
        not: bool,
        #[serde(default)]
        ic: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        hide_options: bool,
    },
    #[serde(rename_all = "camelCase")]
    DropDown {
        #[serde(default)]
        value: String,
        #[serde(default)]
        options: Vec<Choice>,
        #[serde(default)]
        options_url: String,
        /// Request parameter name to the field whose value it sends.
        #[serde(default)]
        depends_on: BTreeMap<String, String>,
        /// Fixed parameters of the options transition.
        #[serde(default)]
        options_parameters: BTreeMap<String, String>,
        /// The options transition filters by a `term` parameter.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        server_search: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        /// Free text is allowed as well as the options.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        combo: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_empty: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        submit_on_select: bool,
        /// A find drop-down that also posts `name_not`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        show_not: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        not: bool,
    },
    #[serde(rename_all = "camelCase")]
    DateTime {
        #[serde(default)]
        value: String,
        /// `date`, `time`, or `date-time`.
        #[serde(default, rename = "type")]
        date_type: String,
        /// A Java date pattern such as `yyyy-MM-dd HH:mm`.
        #[serde(default)]
        format: String,
    },
    /// Posts `name_from` and `name_thru`.
    #[serde(rename_all = "camelCase")]
    DateFind {
        #[serde(default, rename = "type")]
        date_type: String,
        #[serde(default)]
        format: String,
        #[serde(default)]
        from: String,
        #[serde(default)]
        thru: String,
    },
    /// Posts `name_period`, `name_poffset`, `name_pdate`, `name_from` and `name_thru`.
    #[serde(rename_all = "camelCase")]
    DatePeriod {
        #[serde(default)]
        period: String,
        #[serde(default)]
        poffset: String,
        #[serde(default)]
        pdate: String,
        #[serde(default)]
        from: String,
        #[serde(default)]
        thru: String,
        #[serde(default)]
        allow_empty: bool,
        #[serde(default)]
        time: bool,
    },
    /// Posts `name_from` and `name_thru`.
    RangeFind {
        #[serde(default)]
        from: String,
        #[serde(default)]
        thru: String,
    },
    /// Upload is off where the client runs on the server (SSH, browser terminal).
    File {
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        accept: String,
    },
    /// `value` is the plain value posted with the form (`also-hidden`); absent, nothing is posted.
    Display {
        #[serde(default)]
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// No value. A password in the document would be a secret, so a `value` key is ignored.
    Password {},
    Check {
        #[serde(default)]
        value: String,
        #[serde(default)]
        options: Vec<Choice>,
    },
    Radio {
        #[serde(default)]
        value: String,
        #[serde(default)]
        options: Vec<Choice>,
    },
    Hidden {
        #[serde(default)]
        value: String,
    },
    Submit {
        #[serde(default)]
        text: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        confirm: String,
    },
    Reset {
        #[serde(default)]
        text: String,
    },
    Label {
        #[serde(default)]
        text: String,
        #[serde(default)]
        style: String,
    },
    Image {
        #[serde(default)]
        alt: String,
        #[serde(default)]
        url: String,
    },
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl Widget {
    /// The value the field holds as one string: the input's value, a display's text, a button's caption.
    pub fn value(&self) -> &str {
        match self {
            Self::TextLine { value, .. }
            | Self::TextArea { value, .. }
            | Self::TextFind { value, .. }
            | Self::DropDown { value, .. }
            | Self::DateTime { value, .. }
            | Self::Check { value, .. }
            | Self::Radio { value, .. }
            | Self::Hidden { value } => value,
            Self::Display { text, .. }
            | Self::Submit { text, .. }
            | Self::Reset { text }
            | Self::Label { text, .. } => text,
            Self::DateFind { from, .. } | Self::DatePeriod { from, .. } | Self::RangeFind { from, .. } => from,
            Self::Password {} | Self::File { .. } | Self::Image { .. } => "",
        }
    }

    pub fn is_editable_text(&self) -> bool {
        match self {
            Self::TextLine { .. } | Self::TextFind { .. } | Self::DateTime { .. } | Self::Password {} => true,
            Self::TextArea { read_only, .. } => !read_only,
            _ => false,
        }
    }

    /// Shown, not edited: a display, a label, an image.
    pub fn is_display(&self) -> bool {
        matches!(self, Self::Display { .. } | Self::Label { .. } | Self::Image { .. })
    }
}

impl Node {
    pub fn id(&self) -> &str {
        match self {
            Self::FormList { list } => &list.id,
            Self::Section { id, .. }
            | Self::Container { id, .. }
            | Self::Box { id, .. }
            | Self::Row { id, .. }
            | Self::Col { id, .. }
            | Self::Dialog { id, .. }
            | Self::Subscreen { id, .. }
            | Self::Active { id, .. }
            | Self::Fragment { id, .. }
            | Self::Region { id, .. }
            | Self::Text { id, .. }
            | Self::Link { id, .. }
            | Self::Form { id, .. }
            | Self::Field { id, .. }
            | Self::Placeholder { id, .. }
            | Self::Error { id, .. }
            | Self::ButtonMenu { id, .. }
            | Self::Panel { id, .. }
            | Self::Split { id, .. }
            | Self::Dashboard { id, .. }
            | Self::NumberCard { id, .. }
            | Self::StatusBar { id, .. }
            | Self::DynamicDialog { id, .. }
            | Self::DynamicContainer { id, .. }
            | Self::Tree { id, .. }
            | Self::Image { id, .. }
            | Self::Html { id, .. }
            | Self::Markdown { id, .. } => id,
        }
    }

    /// Child nodes in document order. List row cells are not children; [`walk`] visits those too.
    pub fn children(&self) -> &[Node] {
        match self {
            Self::Section { c, .. }
            | Self::Container { c, .. }
            | Self::Box { c, .. }
            | Self::Row { c, .. }
            | Self::Col { c, .. }
            | Self::Dialog { c, .. }
            | Self::Subscreen { c, .. }
            | Self::Active { c, .. }
            | Self::Fragment { c, .. }
            | Self::Region { c, .. }
            | Self::Form { c, .. }
            | Self::ButtonMenu { c, .. }
            | Self::Panel { c, .. }
            | Self::Split { c, .. }
            | Self::Dashboard { c, .. }
            | Self::DynamicDialog { c, .. }
            | Self::DynamicContainer { c, .. }
            | Self::Tree { c, .. } => c,
            Self::FormList { list } => &list.c,
            Self::Text { .. }
            | Self::Link { .. }
            | Self::Field { .. }
            | Self::Placeholder { .. }
            | Self::Error { .. }
            | Self::NumberCard { .. }
            | Self::StatusBar { .. }
            | Self::Image { .. }
            | Self::Html { .. }
            | Self::Markdown { .. } => &[],
        }
    }

    /// Mutable children. The empty cases return an empty slice.
    pub fn children_mut(&mut self) -> &mut [Node] {
        match self {
            Self::Section { c, .. }
            | Self::Container { c, .. }
            | Self::Box { c, .. }
            | Self::Row { c, .. }
            | Self::Col { c, .. }
            | Self::Dialog { c, .. }
            | Self::Subscreen { c, .. }
            | Self::Active { c, .. }
            | Self::Fragment { c, .. }
            | Self::Region { c, .. }
            | Self::Form { c, .. }
            | Self::ButtonMenu { c, .. }
            | Self::Panel { c, .. }
            | Self::Split { c, .. }
            | Self::Dashboard { c, .. }
            | Self::DynamicDialog { c, .. }
            | Self::DynamicContainer { c, .. }
            | Self::Tree { c, .. } => c,
            Self::FormList { list } => &mut list.c,
            Self::Text { .. }
            | Self::Link { .. }
            | Self::Field { .. }
            | Self::Placeholder { .. }
            | Self::Error { .. }
            | Self::NumberCard { .. }
            | Self::StatusBar { .. }
            | Self::Image { .. }
            | Self::Html { .. }
            | Self::Markdown { .. } => &mut [],
        }
    }
}

/// Walk every node, including nested children and list-cell nodes.
pub fn walk<'a>(nodes: &'a [Node], visit: &mut impl FnMut(&'a Node)) {
    for node in nodes {
        visit(node);
        if let Node::FormList { list } = node {
            for row in &list.rows {
                for cell in &row.cells {
                    if let Cell::Node(inner) = cell {
                        visit(inner);
                        walk(inner.children(), visit);
                    }
                }
            }
            walk(&list.header_fields, visit);
        }
        walk(node.children(), visit);
    }
}
