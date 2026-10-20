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
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let model: Self = serde_json::from_str(text).map_err(|err| ModelError::Json(err.to_string()))?;
        if model.v != MODEL_VERSION {
            return Err(ModelError::Version(model.v));
        }
        Ok(model)
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
    Text {
        #[serde(default)]
        id: String,
        text: String,
        #[serde(default)]
        style: String,
    },
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
    Field {
        id: String,
        name: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        maxlength: u32,
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
    Split {
        id: String,
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
    #[serde(default)]
    pub c: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub title: String,
    /// The `orderByField` value that sorts on this column, empty when it does not sort.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub order_by: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListRow {
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
pub struct Paging {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Widget {
    TextLine {
        #[serde(default)]
        value: String,
    },
    /// One string. A find with `hide-options` is this widget.
    TextFind {
        #[serde(default)]
        value: String,
    },
    DropDown {
        #[serde(default)]
        value: String,
        #[serde(default)]
        options: Vec<Choice>,
        #[serde(default, rename = "optionsUrl")]
        options_url: String,
        #[serde(default, rename = "dependsOn")]
        depends_on: Vec<String>,
        #[serde(default, rename = "optionsParameters")]
        options_parameters: Vec<String>,
    },
    DateTime {
        #[serde(default)]
        value: String,
        #[serde(default)]
        format: String,
    },
    Display {
        #[serde(default)]
        text: String,
    },
    /// No value. A password in the document would be a secret, so a `value` key is ignored.
    Password {},
    Check {
        #[serde(default)]
        value: String,
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
    },
    Reset {
        #[serde(default)]
        text: String,
    },
}

impl Widget {
    pub fn value(&self) -> &str {
        match self {
            Self::TextLine { value }
            | Self::TextFind { value }
            | Self::DropDown { value, .. }
            | Self::DateTime { value, .. }
            | Self::Check { value }
            | Self::Radio { value, .. }
            | Self::Hidden { value } => value,
            Self::Display { text } | Self::Submit { text } | Self::Reset { text } => text,
            Self::Password {} => "",
        }
    }

    pub fn is_editable_text(&self) -> bool {
        matches!(
            self,
            Self::TextLine { .. } | Self::TextFind { .. } | Self::DateTime { .. } | Self::Password {}
        )
    }

    pub fn is_display(&self) -> bool {
        matches!(self, Self::Display { .. })
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
