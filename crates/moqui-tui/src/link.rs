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

//! `ScreenLink` is the client's only server call: a level, a transition, or a drop-down's options.
//! Paging sends `pageIndex`. [`FakeLink`] records each call and, when asked for a page, returns that slice.

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::layout::PAGE_ROWS;
use crate::model::{Cell, Choice, ListRow, Messages, ModelError, Node, ScreenModel, ValidationError};

pub const GALLERY_LIST_JSON: &str = include_str!("../fixtures/list.json");
pub const GALLERY_DETAIL_JSON: &str = include_str!("../fixtures/detail.json");

pub const GALLERY_LIST_PATH: &str = "/Gallery/List";
pub const GALLERY_DETAIL_PATH: &str = "/Gallery/Detail";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    Model(ModelError),
    Missing(String),
    /// The server refused the call, or the worker did not return one.
    Remote(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model(err) => write!(f, "{err}"),
            Self::Missing(path) => write!(f, "no screen at {path}"),
            Self::Remote(text) => write!(f, "{text}"),
        }
    }
}

/// What a transition posted. `screen_url` is empty when the client stays on this screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionOutcome {
    pub screen_url: String,
    pub messages: Messages,
    pub parameters: BTreeMap<String, String>,
}

impl TransitionOutcome {
    pub fn saved(text: &str) -> Self {
        Self {
            screen_url: String::new(),
            messages: Messages {
                info: vec![text.to_string()],
                ..Messages::default()
            },
            parameters: BTreeMap::new(),
        }
    }

    pub fn invalid(errors: Vec<ValidationError>) -> Self {
        Self {
            screen_url: String::new(),
            messages: Messages {
                validation_errors: errors,
                ..Messages::default()
            },
            parameters: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkCall {
    Level {
        path: String,
        parameters: BTreeMap<String, String>,
    },
    Transition {
        url: String,
        method: String,
        fields: BTreeMap<String, String>,
    },
    Options {
        url: String,
        parameters: BTreeMap<String, String>,
    },
}

pub trait ScreenLink {
    fn level(&self, path: &str, parameters: &BTreeMap<String, String>) -> Result<ScreenModel, LinkError>;

    fn transition(
        &self,
        url: &str,
        method: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<TransitionOutcome, LinkError>;

    fn options(&self, url: &str, parameters: &BTreeMap<String, String>) -> Result<Vec<Choice>, LinkError>;
}

/// One canned options answer. An empty parameter map is the default for that url.
#[derive(Clone, Debug)]
struct OptionAnswer {
    parameters: BTreeMap<String, String>,
    choices: Vec<Choice>,
}

/// Fixture files keyed by `screen.path`.
#[derive(Clone, Debug)]
pub struct FakeLink {
    levels: BTreeMap<String, ScreenModel>,
    log: RefCell<Vec<LinkCall>>,
    transitions: RefCell<BTreeMap<String, TransitionOutcome>>,
    /// Options for a url, matched by the parameter map. An empty map is the default answer.
    options: RefCell<BTreeMap<String, Vec<OptionAnswer>>>,
}

impl FakeLink {
    pub fn from_json(files: &[&str]) -> Result<Self, LinkError> {
        let mut levels = BTreeMap::new();
        for file in files {
            let model = ScreenModel::from_json(file).map_err(LinkError::Model)?;
            levels.insert(model.screen.path.clone(), model);
        }
        Ok(Self {
            levels,
            log: RefCell::new(Vec::new()),
            transitions: RefCell::new(BTreeMap::new()),
            options: RefCell::new(BTreeMap::new()),
        })
    }

    pub fn calls(&self) -> Vec<LinkCall> {
        self.log.borrow().clone()
    }

    pub fn answer_transition(&self, url: &str, outcome: TransitionOutcome) {
        self.transitions.borrow_mut().insert(url.to_string(), outcome);
    }

    pub fn answer_options(&self, url: &str, parameters: BTreeMap<String, String>, choices: Vec<Choice>) {
        self.options
            .borrow_mut()
            .entry(url.to_string())
            .or_default()
            .push(OptionAnswer { parameters, choices });
    }
}

impl ScreenLink for FakeLink {
    fn level(&self, path: &str, parameters: &BTreeMap<String, String>) -> Result<ScreenModel, LinkError> {
        self.log.borrow_mut().push(LinkCall::Level {
            path: path.to_string(),
            parameters: parameters.clone(),
        });
        let model = self
            .levels
            .get(path)
            .cloned()
            .ok_or_else(|| LinkError::Missing(path.to_string()))?;
        Ok(page_model(model, parameters))
    }

    fn transition(
        &self,
        url: &str,
        method: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<TransitionOutcome, LinkError> {
        self.log.borrow_mut().push(LinkCall::Transition {
            url: url.to_string(),
            method: method.to_string(),
            fields: fields.clone(),
        });
        Ok(self
            .transitions
            .borrow()
            .get(url)
            .cloned()
            .unwrap_or_else(|| TransitionOutcome::saved("Saved")))
    }

    fn options(&self, url: &str, parameters: &BTreeMap<String, String>) -> Result<Vec<Choice>, LinkError> {
        self.log.borrow_mut().push(LinkCall::Options {
            url: url.to_string(),
            parameters: parameters.clone(),
        });
        let table = self.options.borrow();
        let Some(answers) = table.get(url) else {
            return Ok(Vec::new());
        };
        let matched = answers
            .iter()
            .find(|answer| answer.parameters == *parameters)
            .or_else(|| answers.iter().find(|answer| answer.parameters.is_empty()));
        Ok(matched.map(|answer| answer.choices.clone()).unwrap_or_default())
    }
}

pub fn gallery_link() -> FakeLink {
    FakeLink::from_json(&[GALLERY_LIST_JSON, GALLERY_DETAIL_JSON]).expect("gallery fixtures parse")
}

fn page_model(mut model: ScreenModel, parameters: &BTreeMap<String, String>) -> ScreenModel {
    let Some(index) = parameters.get("pageIndex").and_then(|text| text.parse::<usize>().ok()) else {
        return model;
    };
    page_nodes(&mut model.c, index);
    model
}

fn page_nodes(nodes: &mut [Node], index: usize) {
    for node in nodes {
        if let Node::FormList { list } = node {
            let size = if list.paging.size == 0 {
                PAGE_ROWS
            } else {
                usize::try_from(list.paging.size).unwrap_or(PAGE_ROWS).max(1)
            };
            if list.rows.len() > size {
                if list.paging.count == 0 {
                    list.paging.count = u32::try_from(list.rows.len()).unwrap_or(u32::MAX);
                }
                let start = index.saturating_mul(size);
                list.rows = list.rows.iter().skip(start).take(size).cloned().collect();
            }
            list.paging.index = u32::try_from(index).unwrap_or(u32::MAX);
            if list.paging.size == 0 {
                list.paging.size = u32::try_from(size).unwrap_or(u32::MAX);
            }
        }
        page_nodes(node.children_mut(), index);
    }
}

/// First link url in a row, if the row has one.
pub fn row_link(row: &ListRow) -> Option<&str> {
    row.cells.iter().find_map(|cell| match cell {
        Cell::Node(node) => match node.as_ref() {
            Node::Link { url, .. } if !url.is_empty() => Some(url.as_str()),
            _ => None,
        },
        Cell::Text(_) => None,
    })
}
