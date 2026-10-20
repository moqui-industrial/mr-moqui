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

//! Level stack and history. A navigation reloads from the first level whose path is not a prefix
//! of the new path. Parent levels stay.

use std::collections::BTreeMap;

use crate::model::ScreenModel;

#[derive(Clone, Debug)]
pub struct Level {
    pub model: ScreenModel,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct Nav {
    stack: Vec<Level>,
    history: Vec<Vec<Level>>,
}

impl Nav {
    pub fn new(model: ScreenModel) -> Self {
        Self {
            stack: vec![Level {
                model,
                parameters: BTreeMap::new(),
            }],
            history: Vec::new(),
        }
    }

    pub fn leaf(&self) -> &ScreenModel {
        &self.stack.last().expect("nav always has a level").model
    }

    pub fn leaf_parameters(&self) -> &BTreeMap<String, String> {
        &self.stack.last().expect("nav always has a level").parameters
    }

    pub fn paths(&self) -> Vec<&str> {
        self.stack
            .iter()
            .map(|level| level.model.screen.path.as_str())
            .collect()
    }

    /// Replace the leaf after a paging or reload request. History stays.
    pub fn replace_leaf(&mut self, model: ScreenModel, parameters: BTreeMap<String, String>) {
        if let Some(level) = self.stack.last_mut() {
            level.model = model;
            level.parameters = parameters;
        }
    }

    /// Open `model`. Levels whose path is still a prefix of the new path stay.
    pub fn follow(&mut self, model: ScreenModel) {
        let new_path = model.screen.path.clone();
        let change = self
            .stack
            .iter()
            .position(|level| !path_is_prefix(&level.model.screen.path, &new_path))
            .unwrap_or(self.stack.len());
        self.history.push(self.stack.clone());
        self.stack.truncate(change);
        if self
            .stack
            .last()
            .is_some_and(|level| level.model.screen.path == new_path)
        {
            self.stack.pop();
        }
        self.stack.push(Level {
            model,
            parameters: BTreeMap::new(),
        });
    }

    pub fn back(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        if previous.is_empty() {
            return false;
        }
        self.stack = previous;
        true
    }
}

fn path_is_prefix(level_path: &str, new_path: &str) -> bool {
    level_path == new_path || (new_path.starts_with(level_path) && new_path[level_path.len()..].starts_with('/'))
}
