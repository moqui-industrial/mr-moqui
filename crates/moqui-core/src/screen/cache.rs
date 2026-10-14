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

//! The screen facade: `screen-facade` conf, render modes, and the screen-definition cache.
//!
//! One `ScreenFacade` lives on each factory as an extension. Definitions sit in a `moqui_lib`
//! `MCache` keyed by location. A cached definition is used while every file it was read from
//! still has the modified time it had at load.

use std::collections::BTreeMap;
use std::sync::Arc;

use moqui_lib::cache::{CacheConfig, MCache};
use moqui_lib::node::MNode;

use super::definition::ScreenDef;
use crate::context::{Error, ExecutionContextFactory};

/// `screen-text-output`: how one render mode is sent and which macro module writes it.
#[derive(Clone, Debug)]
pub struct TextOutput {
    pub mime_type: String,
    pub macro_template_location: String,
    pub always_standalone: bool,
}

/// One `subscreens-item` from a conf `screen-facade/screen` element.
#[derive(Clone, Debug)]
pub struct ConfItem {
    attributes: Vec<(String, String)>,
}

impl ConfItem {
    pub(super) fn node(&self) -> MNode {
        let node = MNode::new("subscreens-item");
        for (key, value) in &self.attributes {
            node.set_attribute(key.clone(), value.clone());
        }
        node
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConfScreen {
    pub items: Vec<ConfItem>,
    pub default_subscreen: Option<String>,
}

/// The parsed `screen-facade` conf.
#[derive(Clone, Debug, Default)]
pub struct ScreenConf {
    pub text_output: BTreeMap<String, TextOutput>,
    pub screens: BTreeMap<String, ConfScreen>,
    pub boundary_comments: bool,
    pub default_paginate_rows: i64,
    pub default_autocomplete_rows: i64,
}

const BUILT_IN_MODES: [(&str, &str, &str, bool); 8] = [
    ("html", "text/html", "ScreenHtml.js", false),
    ("text", "text/plain", "ScreenText.js", false),
    ("csv", "text/csv", "ScreenCsv.js", true),
    ("xml", "text/xml", "ScreenXml.js", false),
    ("html-print", "text/html", "ScreenHtmlPrint.js", true),
    ("qvt", "text/html", "ScreenQvt.js", true),
    ("qjs", "application/javascript", "ScreenPlain.js", true),
    ("qvue", "text/html", "ScreenPlain.js", true),
];

/// The render mode name the macros see: `xsl-fo` is `html-print`.
pub fn canonical_render_mode(mode: &str) -> &str {
    if mode == "xsl-fo" {
        "html-print"
    } else {
        mode
    }
}

impl ScreenConf {
    /// Built-in modes first, so a factory with no `screen-facade` still renders. Conf entries
    /// replace them by type.
    pub(super) fn parse(conf_text: &str) -> Self {
        let mut conf = Self {
            default_paginate_rows: 20,
            default_autocomplete_rows: 20,
            ..Self::default()
        };
        for (mode, mime, module, standalone) in BUILT_IN_MODES {
            conf.text_output.insert(
                mode.to_string(),
                TextOutput {
                    mime_type: mime.to_string(),
                    macro_template_location: format!("framework/template/screen/{module}"),
                    always_standalone: standalone,
                },
            );
        }
        let Ok(root) = MNode::parse_text("moqui-conf", conf_text) else {
            return conf;
        };
        let Some(facade) = root.first_named("screen-facade") else {
            return conf;
        };
        conf.boundary_comments = facade.attribute("boundary-comments").as_deref() == Some("true");
        if let Some(rows) = facade
            .attribute("default-paginate-rows")
            .and_then(|text| text.parse().ok())
        {
            conf.default_paginate_rows = rows;
        }
        if let Some(rows) = facade
            .attribute("default-autocomplete-rows")
            .and_then(|text| text.parse().ok())
        {
            conf.default_autocomplete_rows = rows;
        }
        for output in facade.children_named("screen-text-output") {
            let Some(kind) = output.attribute("type").filter(|text| !text.is_empty()) else {
                continue;
            };
            // `xsl-fo` is the print mode, so its entry configures `html-print`.
            let kind = canonical_render_mode(&kind).to_string();
            let existing = conf.text_output.get(&kind).cloned();
            conf.text_output.insert(
                kind,
                TextOutput {
                    mime_type: output
                        .attribute("mime-type")
                        .filter(|text| !text.is_empty())
                        .or_else(|| existing.as_ref().map(|found| found.mime_type.clone()))
                        .unwrap_or_else(|| "text/html".to_string()),
                    macro_template_location: output
                        .attribute("macro-template-location")
                        .filter(|text| !text.is_empty())
                        .or_else(|| existing.as_ref().map(|found| found.macro_template_location.clone()))
                        .unwrap_or_default(),
                    always_standalone: output.attribute("always-standalone").as_deref() == Some("true"),
                },
            );
        }
        for screen in facade.children_named("screen") {
            let Some(location) = screen.attribute("location").filter(|text| !text.is_empty()) else {
                continue;
            };
            let entry = conf.screens.entry(location).or_default();
            for item in screen.children_named("subscreens-item") {
                entry.items.push(ConfItem {
                    attributes: item.attributes(),
                });
            }
            if let Some(default) = screen.attribute("default-subscreen").filter(|text| !text.is_empty()) {
                entry.default_subscreen = Some(default);
            }
        }
        conf
    }
}

pub struct ScreenFacade {
    conf: ScreenConf,
    definitions: MCache<String, Arc<ScreenDef>>,
}

impl ScreenFacade {
    fn new(factory: &ExecutionContextFactory) -> Self {
        Self {
            conf: ScreenConf::parse(factory.conf_text()),
            definitions: MCache::new("screen.definition", CacheConfig::new().background(false)),
        }
    }

    /// The facade for `factory`, made on first use.
    pub fn of(factory: &ExecutionContextFactory) -> Arc<Self> {
        factory.extension(Self::new)
    }

    pub fn conf(&self) -> &ScreenConf {
        &self.conf
    }

    /// Whether `mode` is a render mode with a `screen-text-output`. `xsl-fo` counts as `html-print`.
    pub fn is_render_mode_valid(&self, mode: &str) -> bool {
        self.conf.text_output.contains_key(canonical_render_mode(mode))
    }

    pub fn text_output(&self, mode: &str) -> Option<&TextOutput> {
        self.conf.text_output.get(canonical_render_mode(mode))
    }

    pub fn mime_type(&self, mode: &str) -> Option<&str> {
        self.text_output(mode).map(|output| output.mime_type.as_str())
    }

    pub fn is_always_standalone(&self, mode: &str) -> bool {
        self.text_output(mode).is_some_and(|output| output.always_standalone)
    }

    /// The definition at `location`, from the cache when no source file has changed.
    ///
    /// # Errors
    ///
    /// Returns an error when the screen is missing or does not load. A missing screen is
    /// `ErrorKind::NotFound`.
    pub fn screen_def(&self, factory: &ExecutionContextFactory, location: &str) -> Result<Arc<ScreenDef>, Error> {
        self.screen_def_at(factory, location, 0)
    }

    pub(super) fn screen_def_at(
        &self,
        factory: &ExecutionContextFactory,
        location: &str,
        depth: usize,
    ) -> Result<Arc<ScreenDef>, Error> {
        if let Some(cached) = self.definitions.get(&location.to_string())? {
            if is_current(factory, &cached) {
                return Ok(cached);
            }
        }
        let loaded = Arc::new(ScreenDef::load(self, factory, location, depth)?);
        // A put in the same millisecond as the last one keeps the old value, so clear the key first.
        self.definitions.remove(&location.to_string())?;
        self.definitions.put(location.to_string(), Arc::clone(&loaded))?;
        Ok(loaded)
    }

    /// Whether `location` is a screen file. A load error counts as not a screen.
    pub fn is_screen(&self, factory: &ExecutionContextFactory, location: &str) -> bool {
        let Ok(resource) = factory.resource().get(location) else {
            return false;
        };
        if !resource.exists() || !resource.is_file() {
            return false;
        }
        if self.definitions.contains_key(&location.to_string()).unwrap_or(false) {
            return true;
        }
        resource
            .read_text()
            .ok()
            .and_then(|text| MNode::parse_root_only(location, &text).ok())
            .is_some_and(|root| root.name() == "screen")
    }

    /// Drop every cached definition.
    pub fn clear(&self) {
        let _ = self.definitions.clear();
    }

    /// Cache hit and miss counts, for tests and the status page.
    pub fn cache_stats(&self) -> moqui_lib::cache::CacheStats {
        self.definitions.stats()
    }

    /// The definition cache as the cache admin screen lists it.
    pub fn cache_info(&self) -> moqui_lib::cache::CacheInfo {
        self.definitions.info()
    }
}

fn is_current(factory: &ExecutionContextFactory, def: &ScreenDef) -> bool {
    def.sources
        .iter()
        .all(|source| match factory.resource().get(&source.location) {
            Ok(resource) => resource.exists() && resource.last_modified() == source.modified,
            Err(_) => false,
        })
}
