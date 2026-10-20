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
//! are ignored so a later key does not break a fixture. A `v` other than 1 is refused.

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

/// One screen level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenModel {
    pub v: u32,
    pub screen: ScreenMeta,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenMeta {
    pub path: String,
    #[serde(default)]
    pub title: String,
}

/// A node the spike draws. Later node types stay out until Phase 2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Node {
    Box {
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        c: Vec<Node>,
    },
    Text {
        #[serde(default)]
        id: String,
        text: String,
    },
    Link {
        id: String,
        text: String,
        url: String,
    },
    Form {
        id: String,
        name: String,
        #[serde(default)]
        kind: String,
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
        w: Widget,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormList {
    pub id: String,
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<ListRow>,
    pub paging: Paging,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListRow {
    pub cells: Vec<Cell>,
}

/// A list cell is either plain text or one node (a link, in the fixtures).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Node(Node),
}

impl Cell {
    pub fn text(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Node(Node::Link { text, .. }) => text,
            Self::Node(Node::Text { text, .. }) => text,
            Self::Node(_) => "",
        }
    }

    pub fn link_url(&self) -> Option<&str> {
        match self {
            Self::Node(Node::Link { url, .. }) => Some(url),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paging {
    pub index: u32,
    pub size: u32,
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
    DropDown {
        #[serde(default)]
        value: String,
        #[serde(default)]
        options: Vec<Choice>,
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
    Submit {
        #[serde(default)]
        text: String,
    },
}

impl Widget {
    pub fn value(&self) -> &str {
        match self {
            Self::TextLine { value } | Self::DropDown { value, .. } | Self::DateTime { value, .. } => value,
            Self::Display { text } => text,
            Self::Submit { text } => text,
        }
    }

    pub fn is_editable_text(&self) -> bool {
        matches!(self, Self::TextLine { .. } | Self::DateTime { .. })
    }

    pub fn is_display(&self) -> bool {
        matches!(self, Self::Display { .. })
    }
}

impl Node {
    pub fn id(&self) -> &str {
        match self {
            Self::Box { id, .. }
            | Self::Text { id, .. }
            | Self::Link { id, .. }
            | Self::Form { id, .. }
            | Self::Field { id, .. } => id,
            Self::FormList { list } => &list.id,
        }
    }
}

/// Walk every node, including form children and list-cell nodes.
pub fn walk<'a>(nodes: &'a [Node], visit: &mut impl FnMut(&'a Node)) {
    for node in nodes {
        visit(node);
        match node {
            Node::Box { c, .. } | Node::Form { c, .. } => walk(c, visit),
            Node::FormList { list } => {
                for row in &list.rows {
                    for cell in &row.cells {
                        if let Cell::Node(inner) = cell {
                            visit(inner);
                        }
                    }
                }
            }
            Node::Text { .. } | Node::Link { .. } | Node::Field { .. } => {}
        }
    }
}
