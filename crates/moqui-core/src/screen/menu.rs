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

//! `sri.getMenuData`: the user-specific menu for a screen path as the `qvt` client reads it from the
//! `menuData` transition of the root screen. One entry for each screen on the path above the target with
//! the subscreens the user may view, and a last entry for the target.
//!
//! Not ported: saved finds in the last entry (the list is there and empty) and screen documents.

use indexmap::IndexMap;
use moqui_lib::object::Value;

use super::render::{Frames, RenderState};
use super::sri::screen_permitted;
use super::url::ScreenUrlInfo;
use super::urls::{self, UrlInstance};
use crate::context::{Error, ErrorKind};
use crate::{l10n, script, user, web};

type Item = IndexMap<String, Value>;

fn text(value: &str) -> Value {
    Value::string(value)
}

/// Whether the user may view `target` below `parents`, which are on the artifact stack as they are when the screen renders:
/// a grant on a screen reaches the screens below it.
fn permitted_under(
    parents: &[std::sync::Arc<super::definition::ScreenDef>],
    target: &super::definition::ScreenDef,
) -> bool {
    let mut frames = Frames::untracked();
    for parent in parents {
        if frames.push_screen(parent, false).is_err() {
            return false;
        }
    }
    screen_permitted(target)
}

/// The image of a menu entry. A `url-screen` image is a path from the root screen.
fn menu_image(state: &RenderState, image: Option<&(String, String)>) -> (Value, Value) {
    let Some((path, kind)) = image.filter(|(path, _)| !path.is_empty()) else {
        return (Value::Null, Value::Null);
    };
    let path = if kind.is_empty() || kind == "url-screen" {
        urls::build_url(state, path).map_or_else(|_| path.clone(), |url| url.url())
    } else {
        path.clone()
    };
    (text(&path), text(kind))
}

fn path_with_params(url: &UrlInstance) -> String {
    let query = url.parameter_string();
    if query.is_empty() {
        url.path()
    } else {
        format!("{}?{query}", url.path())
    }
}

/// The menu data for `path_names`, or None when a JSON error was sent instead: 401 without a user, 404
/// for a path that names no screen, 403 for a screen the user may not view.
///
/// # Errors
///
/// Returns an error when a screen on the path does not load or a `disable-when` expression fails.
pub fn menu_data(state: &RenderState, path_names: &[String]) -> Result<Option<Value>, Error> {
    if user::current_username()?.is_none() {
        web::send_json_error(401, "Authentication required");
        return Ok(None);
    }
    let root = state
        .url_info
        .path_defs
        .first()
        .map(|def| def.location.clone())
        .ok_or_else(|| Error::new("no root screen"))?;
    let info = match ScreenUrlInfo::resolve(&state.factory, &root, path_names, 0) {
        Ok(info) => info,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            web::send_json_error(404, &format!("Screen not found for path {path_names:?}"));
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    if !permitted_under(
        &info.path_defs[..info.path_defs.len().saturating_sub(1)],
        &info.target_screen,
    ) {
        web::send_json_error(403, &format!("View not permitted for path {path_names:?}"));
        return Ok(None);
    }

    let mut full_path = info.full_path.clone();
    let extra = info.extra_path.clone();
    if !extra.is_empty() {
        full_path.truncate(full_path.len().saturating_sub(extra.len()));
    }
    if full_path.is_empty() {
        web::send_json_error(404, &format!("Screen not found for path {path_names:?}"));
        return Ok(None);
    }

    // Menu titles may come from values set in pre-actions: they run for every screen except the first two
    // (webroot and the app root), and an error in one is logged and does not stop the menu.
    {
        let mut frames = Frames::untracked();
        for (index, def) in info.path_defs.iter().enumerate() {
            frames.enter_screen(def, false)?;
            if index >= 2 {
                if let Some(actions) = &def.pre_actions {
                    if let Err(err) = script::run_actions(&actions.xml) {
                        tracing::warn!(
                            "Error running pre-actions in {} while getting menu data: {err}",
                            def.location
                        );
                    }
                }
            }
        }
    }

    let mut current_path = String::new();
    let mut out: Vec<Value> = Vec::new();
    let mut cur_screen = std::sync::Arc::clone(&info.path_defs[0]);
    for index in 0..full_path.len() - 1 {
        let path_item = &full_path[index];
        let next_item = &full_path[index + 1];
        current_path.push('/');
        current_path.push_str(&moqui_lib::string::url_encode_if_needed(path_item));
        let Some(ssi) = cur_screen.subscreens.get(path_item).cloned() else {
            break;
        };
        cur_screen = state.facade.screen_def(&state.factory, &ssi.location)?;

        let mut subscreens: Vec<Value> = Vec::new();
        for item in cur_screen.subscreens.sorted() {
            if (!item.menu_include && &item.name != next_item) || !item.is_valid_in_current_context() {
                continue;
            }
            let screen_path = format!("{current_path}/{}", moqui_lib::string::url_encode_if_needed(&item.name));
            let mut names = full_path[..=index].to_vec();
            names.push(item.name.clone());
            let url = urls::build_url_names(state, names.clone())?;
            if !url.exists {
                continue;
            }
            let Ok(target) = state.facade.screen_def(&state.factory, &item.location) else {
                continue;
            };
            if !permitted_under(&info.path_defs[..(index + 2).min(info.path_defs.len())], &target) {
                continue;
            }
            let sui = ScreenUrlInfo::resolve(&state.factory, &root, &names, 0)?;
            let missing = sui.parameter_items.values().any(|parameter| {
                parameter.required && url.parameters.get(&parameter.name).is_none_or(String::is_empty)
            });
            if missing {
                continue;
            }
            let (image, image_type) = menu_image(state, sui.menu_image.as_ref());
            let disabled = match &item.disable_when {
                Some(expression) => script::eval_condition(expression)?,
                None => false,
            };
            let mut entry = Item::new();
            entry.insert("name".into(), text(&item.name));
            entry.insert("title".into(), text(&script::expand(&item.menu_title)?));
            entry.insert("path".into(), text(&screen_path));
            entry.insert("pathWithParams".into(), text(&path_with_params(&url)));
            entry.insert("image".into(), image);
            entry.insert("imageType".into(), image_type);
            if item.menu_include {
                entry.insert("menuInclude".into(), Value::Bool(true));
            }
            if &item.name == next_item {
                entry.insert("active".into(), Value::Bool(true));
            }
            if disabled {
                entry.insert("disableLink".into(), Value::Bool(true));
            }
            subscreens.push(Value::Map(entry));
        }

        let cur_url = urls::build_url_names(state, full_path[..=index].to_vec())?;
        let sui = ScreenUrlInfo::resolve(&state.factory, &root, &full_path[..=index], 0)?;
        let (image, image_type) = menu_image(state, sui.menu_image.as_ref());
        let localized = l10n::localize(&ssi.menu_title)?;
        let title = if localized.is_empty() {
            cur_screen.default_menu_title.clone()
        } else {
            localized
        };
        let mut entry = Item::new();
        entry.insert("name".into(), text(path_item));
        entry.insert("title".into(), text(&title));
        entry.insert("subscreens".into(), Value::List(subscreens));
        entry.insert("path".into(), text(&current_path));
        let query = cur_url.parameter_string();
        entry.insert(
            "pathWithParams".into(),
            text(&if query.is_empty() {
                current_path.clone()
            } else {
                format!("{current_path}?{query}")
            }),
        );
        entry.insert("hasTabMenu".into(), Value::Bool(cur_screen.has_tab_menu));
        entry.insert("renderModes".into(), render_modes(&cur_screen.render_modes));
        entry.insert("image".into(), image);
        entry.insert("imageType".into(), image_type);
        out.push(Value::Map(entry));
    }

    let last_item = full_path[full_path.len() - 1].clone();
    let mut last_url = urls::build_url_names(state, full_path.clone())?;
    for (name, value) in web::parameters() {
        last_url.add_parameter(&name, &value);
    }
    current_path.push('/');
    current_path.push_str(&moqui_lib::string::url_encode_if_needed(&last_item));
    let last_path = current_path.clone();
    let query = last_url.parameter_string();
    if !query.is_empty() {
        current_path.push('?');
        current_path.push_str(&query);
    }
    let (image, image_type) = menu_image(state, info.menu_image.as_ref());
    let last_ssi = cur_screen.subscreens.get(&last_item);
    let localized = match last_ssi {
        Some(item) => l10n::localize(&item.menu_title)?,
        None => String::new(),
    };
    let mut last_title = if localized.is_empty() {
        info.target_screen.default_menu_title.clone()
    } else {
        localized
    };
    if last_title.contains("${") {
        last_title = script::expand(&last_title)?;
    }
    let extra_encoded: Vec<Value> = extra
        .iter()
        .map(|name| text(&moqui_lib::string::url_encode_if_needed(name)))
        .collect();
    let mut entry = Item::new();
    entry.insert("name".into(), text(&last_item));
    entry.insert("title".into(), text(&last_title));
    entry.insert("path".into(), text(&last_path));
    entry.insert("pathWithParams".into(), text(&current_path));
    entry.insert("image".into(), image);
    entry.insert("imageType".into(), image_type);
    entry.insert("extraPathList".into(), Value::List(extra_encoded));
    entry.insert("screenDocList".into(), Value::List(Vec::new()));
    entry.insert("renderModes".into(), render_modes(&info.target_screen.render_modes));
    entry.insert("savedFinds".into(), Value::List(Vec::new()));
    out.push(Value::Map(entry));
    Ok(Some(Value::List(out)))
}

fn render_modes(modes: &[String]) -> Value {
    Value::List(modes.iter().map(|mode| text(mode)).collect())
}
