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

//! The theme of the render: its id, resource values and icon classes.

use moqui_lib::object::{to_plain_string, Value};

use crate::context::{Error, ExecutionContext};
use crate::screen::render::RenderState;
use crate::user;

/// `sri.getCurrentThemeId`: the user's theme for the screen theme type, else one of the user's groups', else the type's
/// `enumCode`, else a theme of the type with `DEFAULT` in its id. Found once for the render.
fn current_theme_id(state: &RenderState) -> String {
    if let Some(found) = state.theme_id.borrow().as_ref() {
        return found.clone();
    }
    let found = find_theme_id(state);
    *state.theme_id.borrow_mut() = Some(found.clone());
    found
}

fn find_theme_id(state: &RenderState) -> String {
    let mut theme_type = String::new();
    for def in state.url_info.render_defs() {
        if !def.screen_theme_type.is_empty() {
            theme_type.clone_from(&def.screen_theme_type);
        }
    }
    if theme_type.is_empty() {
        theme_type = "STT_INTERNAL".to_string();
    }
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return String::new();
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let text_of = |row: Option<crate::entity::EntityValue>, field: &str| {
        row.and_then(|row| row.get(field).map(to_plain_string))
            .filter(|text| !text.is_empty())
    };
    let mut theme = None;
    if let Some(user_id) = user::current_user_id().ok().flatten() {
        theme = text_of(
            facade.find("moqui.security.UserScreenTheme").ok().and_then(|find| {
                find.eq("userId", Value::string(user_id))
                    .eq("screenThemeTypeEnumId", Value::string(&theme_type))
                    .one()
                    .ok()
                    .flatten()
            }),
            "screenThemeId",
        );
    }
    if theme.is_none() {
        // Reverse id order, so ALL_USERS comes after the named groups at the same sequence.
        let mut groups: Vec<String> = user::current_user_groups()
            .map(|groups| groups.iter().cloned().collect())
            .unwrap_or_default();
        groups.sort_unstable_by(|left, right| right.cmp(left));
        let rows = facade
            .find("moqui.security.UserGroupScreenTheme")
            .ok()
            .and_then(|find| {
                find.condition(moqui_db::sql::Condition::is_in(
                    "userGroupId",
                    groups.into_iter().map(Value::String).collect(),
                ))
                .eq("screenThemeTypeEnumId", Value::string(&theme_type))
                .order_by("sequenceNum", false)
                .order_by("userGroupId", true)
                .list()
                .ok()
            });
        theme = text_of(rows.and_then(|rows| rows.values().first().cloned()), "screenThemeId");
    }
    if theme.is_none() {
        theme = text_of(
            facade
                .find("moqui.basic.Enumeration")
                .ok()
                .and_then(|find| find.eq("enumId", Value::string(&theme_type)).one().ok().flatten()),
            "enumCode",
        );
    }
    if theme.is_none() {
        let rows = facade.find("moqui.screen.ScreenTheme").ok().and_then(|find| {
            find.eq("screenThemeTypeEnumId", Value::string(&theme_type))
                .condition(moqui_db::sql::Condition::like(
                    "screenThemeId",
                    Value::string("%DEFAULT%"),
                ))
                .list()
                .ok()
        });
        theme = text_of(rows.and_then(|rows| rows.values().first().cloned()), "screenThemeId");
    }
    theme.unwrap_or_default()
}

/// `sri.getThemeValues(type, themeId)`: the `ScreenThemeResource` values in sequence, kept for the render for the current theme.
///
/// # Errors
///
/// Returns an error when the entity cannot be read.
pub fn get_theme_values(state: &RenderState, resource_type: &str, theme_id: &str) -> Result<Vec<String>, Error> {
    let current = theme_id.is_empty() || theme_id == "null";
    if current {
        if let Some(found) = state.theme_values.borrow().get(resource_type) {
            return Ok(found.clone());
        }
    }
    let theme = if current {
        current_theme_id(state)
    } else {
        theme_id.to_string()
    };
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return Ok(Vec::new());
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let list = facade
        .find("moqui.screen.ScreenThemeResource")?
        .eq("screenThemeId", Value::string(theme))
        .eq("resourceTypeEnumId", Value::string(resource_type))
        .order_by("sequenceNum", false)
        .list()?;
    let values: Vec<String> = list
        .iter()
        .filter_map(|row| row.get("resourceValue").map(to_plain_string))
        .filter(|text| !text.is_empty())
        .collect();
    if current {
        state
            .theme_values
            .borrow_mut()
            .insert(resource_type.to_string(), values.clone());
    }
    Ok(values)
}

/// `sri.getThemeIconClass(text)`: the icon class of the first `ScreenThemeIcon` pattern that matches, kept by text for the theme.
///
/// # Errors
///
/// Returns an error when the entity cannot be read.
pub fn get_theme_icon_class(state: &RenderState, text: &str) -> Result<Option<String>, Error> {
    let theme = current_theme_id(state);
    if let Some(found) = state.facade.theme_icon(&theme, text) {
        return Ok(found);
    }
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return Ok(None);
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let list = facade
        .find("moqui.screen.ScreenThemeIcon")?
        .eq("screenThemeId", Value::string(&theme))
        .list()?;
    let mut icon = None;
    for row in list.iter() {
        let pattern = row.get("textPattern").map(to_plain_string).unwrap_or_default();
        if moqui_lib::object::regex_full_match(text, &pattern).unwrap_or(false) {
            icon = row.get("iconClass").map(to_plain_string);
            break;
        }
    }
    state.facade.put_theme_icon(&theme, text, icon.clone());
    Ok(icon)
}
