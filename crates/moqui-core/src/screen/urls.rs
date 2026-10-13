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

//! URL instances for `sri.buildUrl` and `sri.makeUrlByType`: a screen path or a transition resolved
//! against the screen being rendered, with its parameters.
//!
//! Not ported: the transition-as-alias expansion, parameters taken from the single service a
//! transition calls, `ScreenPathAlias`, and `content` URLs.

use indexmap::IndexMap;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use super::render::parameter_value;
use super::sri::{self, active_screen_path, NodeLit};
use super::url::{encode_path, resolve_screen_path, ScreenUrlInfo};
use super::RenderState;
use crate::context::{Error, ErrorKind};
use crate::{script, web};

/// A URL for a link, with the parameters added to it.
#[derive(Clone, Debug, Default)]
pub struct UrlInstance {
    /// Path names from the root, with the default subscreens a plain path leads to.
    pub names: Vec<String>,
    /// Path names as given, without the defaults.
    pub minimal_names: Vec<String>,
    pub base: String,
    /// A url-type plain URL, used as is.
    pub plain: Option<String>,
    pub parameters: IndexMap<String, String>,
    /// False for a transition that is not read-only.
    pub read_only: bool,
    pub exists: bool,
    pub is_transition: bool,
}

impl UrlInstance {
    /// The path with no host.
    pub fn path(&self) -> String {
        match &self.plain {
            Some(plain) => plain.clone(),
            None => encode_path(&self.names),
        }
    }

    pub fn url(&self) -> String {
        match &self.plain {
            Some(plain) if plain.contains("://") || plain.starts_with("//") => plain.clone(),
            Some(plain) => format!("{}{plain}", self.base.trim_end_matches('/')),
            None => format!("{}{}", self.base.trim_end_matches('/'), encode_path(&self.names)),
        }
    }

    pub fn minimal_path_url(&self) -> String {
        match &self.plain {
            Some(_) => self.url(),
            None => format!(
                "{}{}",
                self.base.trim_end_matches('/'),
                encode_path(&self.minimal_names)
            ),
        }
    }

    pub fn minimal_path_url_with_params(&self) -> String {
        join_query(self.minimal_path_url(), &self.parameter_string())
    }

    pub fn parameter_string(&self) -> String {
        let mut out = String::new();
        for (key, value) in &self.parameters {
            if value.is_empty() || key == "moquiSessionToken" {
                continue;
            }
            if !out.is_empty() {
                out.push('&');
            }
            out.push_str(&moqui_lib::string::url_encode_if_needed(key));
            out.push('=');
            out.push_str(&moqui_lib::string::url_encode_if_needed(value));
        }
        out
    }

    pub fn path_with_params(&self) -> String {
        join_query(self.path(), &self.parameter_string())
    }

    pub fn url_with_params(&self, extension: &str) -> String {
        let mut url = self.url();
        if !extension.is_empty() {
            url.push('.');
            url.push_str(extension);
        }
        join_query(url, &self.parameter_string())
    }

    pub fn add_parameter(&mut self, name: &str, value: &Value) {
        if name.is_empty() || value.is_null() {
            return;
        }
        self.parameters.insert(name.to_string(), to_plain_string(value));
    }

    pub fn remove_parameter(&mut self, name: &str) {
        self.parameters.shift_remove(name);
    }

    /// The parameters without the ones that belong to one request.
    pub fn pass_through_parameters(&self) -> IndexMap<String, String> {
        let mut map = self.parameters.clone();
        for name in [
            "moquiFormName",
            "moquiSessionToken",
            "lastStandalone",
            "formListFindId",
            "moquiRequestStartTime",
            "webrootTT",
        ] {
            map.shift_remove(name);
        }
        map
    }
}

fn join_query(mut base: String, query: &str) -> String {
    if !query.is_empty() {
        base.push('?');
        base.push_str(query);
    }
    base
}

/// `sri.buildUrl(subscreenPath)`: a path from the active screen.
///
/// # Errors
///
/// Returns an error when an expression in the path does not expand.
pub fn build_url(state: &RenderState, subscreen_path: &str) -> Result<UrlInstance, Error> {
    let expanded = if subscreen_path.contains("${") {
        script::expand_localized(subscreen_path)?
    } else {
        subscreen_path.to_string()
    };
    let names = resolve_screen_path(&active_screen_path(state), &expanded);
    resolve_names(state, names)
}

/// A URL for path names from the root.
///
/// # Errors
///
/// Returns an error when the names cannot be resolved for a reason other than a missing screen.
pub fn build_url_names(state: &RenderState, names: Vec<String>) -> Result<UrlInstance, Error> {
    resolve_names(state, names)
}

/// `sri.buildUrlFromTarget`: a path from the target screen of the request.
///
/// # Errors
///
/// Same as [`build_url`].
pub fn build_url_from_target(state: &RenderState, subscreen_path: &str) -> Result<UrlInstance, Error> {
    let expanded = script::expand_localized(subscreen_path)?;
    let names = resolve_screen_path(&state.url_info.pre_transition_path, &expanded);
    resolve_names(state, names)
}

/// `sri.getScreenUrlInstance`/`getCurrentScreenUrl`: the URL of the request being rendered.
///
/// # Errors
///
/// Returns an error when the request's path cannot be resolved again.
pub fn current_url(state: &RenderState) -> Result<UrlInstance, Error> {
    resolve_names(state, state.url_info.full_path.clone())
}

fn resolve_names(state: &RenderState, names: Vec<String>) -> Result<UrlInstance, Error> {
    let root = state
        .url_info
        .path_defs
        .first()
        .map(|def| def.location.clone())
        .ok_or_else(|| Error::new("no root screen"))?;
    let mut url = UrlInstance {
        names: names.clone(),
        minimal_names: names.clone(),
        base: web::base_url(),
        exists: true,
        read_only: true,
        ..UrlInstance::default()
    };
    match ScreenUrlInfo::resolve(&state.factory, &root, &names, state.url_info.last_standalone) {
        Ok(info) => {
            url.names = info.full_path.clone();
            url.minimal_names = if info.always_use_full_path {
                info.full_path.clone()
            } else {
                info.minimal_path.clone()
            };
            for item in info.parameter_items.values() {
                if let Some(value) = parameter_value(item)? {
                    let text = to_plain_string(&value);
                    if !text.is_empty() {
                        url.parameters.insert(item.name.clone(), text);
                    }
                }
            }
            if let Some(transition) = info.transition(None) {
                url.is_transition = true;
                url.read_only = transition.read_only;
                for item in &transition.parameters {
                    if let Some(value) = parameter_value(item)? {
                        let text = to_plain_string(&value);
                        if !text.is_empty() {
                            url.parameters.insert(item.name.clone(), text);
                        }
                    }
                }
            } else if info.target_transition_name.is_some() {
                url.is_transition = true;
                url.read_only = false;
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => url.exists = false,
        Err(err) => return Err(err),
    }
    Ok(url)
}

/// `sri.makeUrlByType(url, urlType, parameterParent, expandTransitionUrl)`.
///
/// # Errors
///
/// Returns an error for a `content` URL, or when an expression fails.
pub fn make_url_by_type(
    state: &RenderState,
    orig_url: &str,
    url_type: &str,
    parameter_parent: Option<&NodeLit>,
) -> Result<UrlInstance, Error> {
    let kind = script::expand_localized(url_type)?;
    let mut url = match kind.as_str() {
        "transition" | "screen" => build_url(state, orig_url)?,
        "content" => return Err(Error::new("The url-type of content is not yet supported")),
        _ => {
            let plain = script::expand_localized(orig_url)?;
            UrlInstance {
                plain: Some(plain),
                base: web::base_url(),
                exists: true,
                read_only: true,
                ..UrlInstance::default()
            }
        }
    };
    if let Some(parent) = parameter_parent {
        let map_expr = parent.attr("parameter-map");
        if !map_expr.is_empty() {
            if let Some(map) = script::value_map(&script::eval_value(map_expr)?) {
                for (key, value) in map {
                    if key != "context" {
                        url.add_parameter(&key, &value);
                    }
                }
            }
        }
        for parameter in parent.children.iter().filter(|child| child.name == "parameter") {
            let name = parameter.attr("name");
            let from = if parameter.attr("from").is_empty() {
                name
            } else {
                parameter.attr("from")
            };
            let value = sri::context_value(from, parameter.attr("value"))?;
            if !is_empty(&value) || !value.is_null() {
                url.add_parameter(name, &value);
            }
        }
    }
    Ok(url)
}

/// `sri.isAnchorLink(linkNode, urlInstance)`.
pub fn is_anchor_link(link: &NodeLit, url: &UrlInstance) -> bool {
    let link_type = link.attr("link-type");
    let url_type = link.attr("url-type");
    link_type == "anchor"
        || link_type == "anchor-button"
        || ((link_type.is_empty() || link_type == "auto")
            && ((!url_type.is_empty() && url_type != "transition") || url.read_only))
}

/// `sri.activeInCurrentMenu`: whether a subscreen of the active screen that appears in the menu is
/// on the path of the request.
pub fn active_in_current_menu(state: &RenderState) -> bool {
    let def = sri::active_def(state);
    let path = &state.url_info.full_path;
    def.subscreens
        .items
        .iter()
        .filter(|item| item.menu_include)
        .any(|item| {
            let mut names = active_screen_path(state);
            names.push(item.name.clone());
            path.len() >= names.len() && path[..names.len()] == names[..]
        })
}
