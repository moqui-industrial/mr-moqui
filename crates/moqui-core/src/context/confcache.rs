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

//! Conf values the user and authorization code reads on every call, parsed once per factory.

use std::collections::HashMap;

use moqui_lib::node::MNode;

#[derive(Default)]
pub(super) struct ConfCache {
    /// `artifact-execution` attributes: `(type, attribute)` to true when the value is `true`.
    artifact: HashMap<(String, String), bool>,
    /// `user-facade` child attributes: `(child, attribute)` to the non-empty value.
    user_facade: HashMap<(String, String), String>,
    /// `webapp` attributes: `(webapp name, attribute)` to the raw value, `${}` not yet expanded.
    webapp: HashMap<(String, String), String>,
}

impl ConfCache {
    pub(super) fn parse(text: &str) -> Self {
        let mut cache = Self::default();
        if text.trim().is_empty() {
            return cache;
        }
        let Ok(root) = MNode::parse_text("moqui-conf", text) else {
            return cache;
        };
        if let Some(facade) = root.first_named("artifact-execution-facade") {
            for node in facade.children_named("artifact-execution") {
                let Some(artifact_type) = node.attribute("type") else {
                    continue;
                };
                for (name, value) in node.attributes() {
                    // The first entry for a type wins, as the Java conf lookup does.
                    cache
                        .artifact
                        .entry((artifact_type.clone(), name))
                        .or_insert(value == "true");
                }
            }
        }
        if let Some(facade) = root.first_named("user-facade") {
            for child in facade.children() {
                for (name, value) in child.attributes() {
                    if !value.is_empty() {
                        cache.user_facade.entry((child.name(), name)).or_insert(value);
                    }
                }
            }
        }
        if let Some(list) = root.first_named("webapp-list") {
            for node in list.children_named("webapp") {
                let Some(webapp) = node.attribute("name") else {
                    continue;
                };
                for (name, value) in node.attributes() {
                    cache.webapp.entry((webapp.clone(), name)).or_insert(value);
                }
            }
        }
        cache
    }

    pub(super) fn artifact_flag(&self, artifact_type: &str, flag: &str) -> bool {
        self.artifact
            .get(&(artifact_type.to_string(), flag.to_string()))
            .copied()
            .unwrap_or(false)
    }

    pub(super) fn user_facade_attr(&self, child: &str, name: &str) -> Option<String> {
        self.user_facade.get(&(child.to_string(), name.to_string())).cloned()
    }

    pub(super) fn webapp_attr(&self, webapp: &str, name: &str) -> Option<String> {
        self.webapp.get(&(webapp.to_string(), name.to_string())).cloned()
    }
}
