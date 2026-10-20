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

//! `ScreenLink` for the spike is one call: load a level by path.
//! Paging slices fixture rows on the client. Phase 2 paging is a real request.

use std::collections::BTreeMap;

use crate::model::{ModelError, ScreenModel};

pub const GALLERY_LIST_JSON: &str = include_str!("../fixtures/list.json");
pub const GALLERY_DETAIL_JSON: &str = include_str!("../fixtures/detail.json");

pub const GALLERY_LIST_PATH: &str = "/Gallery/List";
pub const GALLERY_DETAIL_PATH: &str = "/Gallery/Detail";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    Model(ModelError),
    Missing(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model(err) => write!(f, "{err}"),
            Self::Missing(path) => write!(f, "no screen at {path}"),
        }
    }
}

pub trait ScreenLink {
    fn level(&self, path: &str) -> Result<ScreenModel, LinkError>;
}

/// Fixture files keyed by `screen.path`.
#[derive(Clone, Debug)]
pub struct FakeLink {
    levels: BTreeMap<String, ScreenModel>,
}

impl FakeLink {
    pub fn from_json(files: &[&str]) -> Result<Self, LinkError> {
        let mut levels = BTreeMap::new();
        for file in files {
            let model = ScreenModel::from_json(file).map_err(LinkError::Model)?;
            levels.insert(model.screen.path.clone(), model);
        }
        Ok(Self { levels })
    }
}

impl ScreenLink for FakeLink {
    fn level(&self, path: &str) -> Result<ScreenModel, LinkError> {
        self.levels
            .get(path)
            .cloned()
            .ok_or_else(|| LinkError::Missing(path.to_string()))
    }
}

pub fn gallery_link() -> FakeLink {
    FakeLink::from_json(&[GALLERY_LIST_JSON, GALLERY_DETAIL_JSON]).expect("gallery fixtures parse")
}
