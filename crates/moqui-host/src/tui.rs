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

//! In-process [`ScreenLink`](moqui_tui::ScreenLink). Each call runs on the factory request pool, in a fresh
//! execution context, and logs in as the configured user. The client stays free of the server.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use moqui_core::context::{Error, ErrorKind, ExecutionContextFactory};
use moqui_core::screen::RenderOutput;
use moqui_lib::object::Value;
use moqui_tui::{Choice, LinkError, MenuItem, Messages, ScreenLink, ScreenModel, TransitionOutcome, ValidationError};

/// One screen client bound to a running factory. Clone it so a view can own one and the caller can ask for the menu.
#[derive(Clone)]
pub struct HostLink {
    factory: Arc<ExecutionContextFactory>,
    username: String,
    /// The path of the last level that rendered, used to resolve a relative transition or options url.
    last_path: Arc<Mutex<String>>,
}

impl HostLink {
    pub fn new(factory: Arc<ExecutionContextFactory>, username: impl Into<String>) -> Self {
        Self {
            factory,
            username: username.into(),
            last_path: Arc::new(Mutex::new(String::new())),
        }
    }

    /// The drawer for `path`. Not part of [`ScreenLink`](moqui_tui::ScreenLink): the session does not refresh it.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError::Remote`] when the menu request fails or the body is not a JSON array.
    pub fn menu(&self, path: &str) -> Result<Vec<MenuItem>, LinkError> {
        let url = menu_url(path);
        let factory = Arc::clone(&self.factory);
        let output = self.on_pool(move || moqui_core::screen::make_render(&factory).screen_path(url).render())?;
        menu_from(&output)
    }

    fn absolute(&self, url: &str) -> String {
        let base = self.last_path.lock().unwrap_or_else(|err| err.into_inner()).clone();
        resolve_url(&base, url)
    }

    fn on_pool<T, F>(&self, work: F) -> Result<T, LinkError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, Error> + Send + 'static,
    {
        let username = self.username.clone();
        let rx = self
            .factory
            .request_pool()
            .try_submit(Arc::clone(&self.factory), move || {
                moqui_core::user::internal_login_user(&username)?;
                work()
            })
            .map_err(link_error)?;
        match rx.recv() {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(link_error(err)),
            Err(_) => Err(LinkError::Remote("request worker dropped the result".into())),
        }
    }
}

impl ScreenLink for HostLink {
    fn level(&self, path: &str, parameters: &BTreeMap<String, String>) -> Result<ScreenModel, LinkError> {
        let path = self.absolute(path);
        let values = value_map(parameters);
        let factory = Arc::clone(&self.factory);
        let output = self.on_pool(move || {
            moqui_core::screen::make_render(&factory)
                .screen_path(path)
                .parameters(values)
                .render_mode("tui")
                .render()
        })?;
        let model = model_from(&output)?;
        *self.last_path.lock().unwrap_or_else(|err| err.into_inner()) = model.screen.path.clone();
        Ok(model)
    }

    fn transition(
        &self,
        url: &str,
        method: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<TransitionOutcome, LinkError> {
        let url = self.absolute(url);
        let method = if method.is_empty() {
            "POST".to_string()
        } else {
            method.to_string()
        };
        let values = value_map(fields);
        let factory = Arc::clone(&self.factory);
        let posted = url.clone();
        let output = self.on_pool(move || {
            moqui_core::screen::make_render(&factory)
                .screen_path(posted)
                .method(method)
                .parameters(values)
                .header("Accept", "application/json")
                .render()
        })?;
        // A transition answers JSON. A form whose transition is the screen itself answers the screen, and the
        // posted fields are the parameters of that screen.
        let text = output.body_text();
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return outcome_from(&output);
        }
        Ok(screen_repost(&url, fields))
    }

    fn options(&self, url: &str, parameters: &BTreeMap<String, String>) -> Result<Vec<Choice>, LinkError> {
        let url = self.absolute(url);
        let values = value_map(parameters);
        let factory = Arc::clone(&self.factory);
        let output = self.on_pool(move || {
            moqui_core::screen::make_render(&factory)
                .screen_path(url)
                .parameters(values)
                .render()
        })?;
        choices_from(&output)
    }
}

fn link_error(err: Error) -> LinkError {
    if err.kind() == ErrorKind::AuthenticationRequired {
        return LinkError::Remote("Authentication required".into());
    }
    LinkError::Remote(err.to_string())
}

fn value_map(fields: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), Value::string(value)))
        .collect()
}

fn model_from(output: &RenderOutput) -> Result<ScreenModel, LinkError> {
    if output.status == 401 {
        return Err(LinkError::Remote("Authentication required".into()));
    }
    let text = output.body_text();
    if output.status >= 400 {
        return Err(LinkError::Remote(format!("HTTP {}: {}", output.status, snippet(&text))));
    }
    ScreenModel::from_json(&text).map_err(|err| LinkError::Remote(format!("{err}: {}", snippet(&text))))
}

/// A screen rendered in place of a transition: show that screen with the posted fields.
fn screen_repost(url: &str, fields: &BTreeMap<String, String>) -> TransitionOutcome {
    let (path, query) = split_query(url);
    let mut parameters = query_map(query);
    for (name, value) in fields {
        parameters.insert(name.clone(), value.clone());
    }
    let screen_url = match form_query(&parameters) {
        query if query.is_empty() => path.to_string(),
        query => format!("{path}?{query}"),
    };
    TransitionOutcome {
        screen_url,
        messages: Messages::default(),
        parameters,
    }
}

fn query_map(query: Option<&str>) -> BTreeMap<String, String> {
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return BTreeMap::new();
    };
    query
        .split('&')
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode_query(name), decode_query(value))
        })
        .collect()
}

fn form_query(fields: &BTreeMap<String, String>) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", encode_query(name), encode_query(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_query(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn decode_query(text: &str) -> String {
    let plus = text.replace('+', " ");
    let bytes = plus.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &plus[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn outcome_from(output: &RenderOutput) -> Result<TransitionOutcome, LinkError> {
    if output.status == 401 {
        return Err(LinkError::Remote("Authentication required".into()));
    }
    let text = output.body_text();
    let value = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value,
        Err(_) if output.status >= 400 => {
            return Err(LinkError::Remote(format!("HTTP {}: {}", output.status, snippet(&text))));
        }
        Err(err) => return Err(LinkError::Remote(format!("transition JSON: {err}"))),
    };
    let outcome = outcome_of(&value);
    let quiet = outcome.screen_url.is_empty()
        && outcome.messages.errors.is_empty()
        && outcome.messages.validation_errors.is_empty();
    if output.status >= 400 && quiet {
        return Err(LinkError::Remote(format!("HTTP {}: {}", output.status, snippet(&text))));
    }
    Ok(outcome)
}

fn outcome_of(value: &serde_json::Value) -> TransitionOutcome {
    let mut messages = Messages::default();
    if let Some(list) = value.get("messageInfos").and_then(serde_json::Value::as_array) {
        for item in list {
            let text = json_field(item, "message");
            if text.is_empty() {
                continue;
            }
            let kind = json_field(item, "type");
            if is_error_type(&kind) {
                messages.errors.push(text);
            } else {
                messages.info.push(text);
            }
        }
    }
    messages.validation_errors = validation_of(value.get("validationErrors"));
    TransitionOutcome {
        screen_url: screen_url_of(value),
        messages,
        parameters: string_map(value.get("screenParameters")),
    }
}

fn is_error_type(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("danger") || kind.to_ascii_lowercase().contains("error")
}

fn screen_url_of(value: &serde_json::Value) -> String {
    let screen = json_field(value, "screenUrl");
    if screen.is_empty() {
        json_field(value, "redirectUrl")
    } else {
        screen
    }
}

fn validation_of(value: Option<&serde_json::Value>) -> Vec<ValidationError> {
    let Some(list) = value.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|item| ValidationError {
            form: json_field(item, "form"),
            field: json_field(item, "field"),
            message: json_field(item, "message"),
        })
        .collect()
}

fn choices_from(output: &RenderOutput) -> Result<Vec<Choice>, LinkError> {
    if output.status == 401 {
        return Err(LinkError::Remote("Authentication required".into()));
    }
    let text = output.body_text();
    if output.status != 200 {
        return Err(LinkError::Remote(format!("HTTP {}: {}", output.status, snippet(&text))));
    }
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|err| LinkError::Remote(format!("options JSON: {err}: {}", snippet(&text))))?;
    let list =
        option_list(&value).ok_or_else(|| LinkError::Remote(format!("options were not a list: {}", snippet(&text))))?;
    Ok(list.iter().map(choice_of).collect())
}

fn option_list(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(list) = value.as_array() {
        return Some(list);
    }
    value
        .get("options")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.get("resultList").and_then(serde_json::Value::as_array))
}

fn choice_of(value: &serde_json::Value) -> Choice {
    if let Some(text) = value.as_str() {
        return Choice {
            value: text.to_string(),
            label: text.to_string(),
        };
    }
    let picked = first_text(value, &["value", "geoId", "key", "id"]);
    let mut label = first_text(value, &["label", "text", "geoName", "description"]);
    if label.is_empty() {
        label.clone_from(&picked);
    }
    Choice { value: picked, label }
}

fn menu_from(output: &RenderOutput) -> Result<Vec<MenuItem>, LinkError> {
    if output.status == 401 {
        return Err(LinkError::Remote("Authentication required".into()));
    }
    let text = output.body_text();
    if output.status >= 400 {
        return Err(LinkError::Remote(format!("HTTP {}: {}", output.status, snippet(&text))));
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| LinkError::Remote(format!("menu JSON: {err}")))?;
    let Some(list) = value.as_array() else {
        return Err(LinkError::Remote("menu was not a list".into()));
    };
    Ok(list.iter().filter_map(menu_item).collect())
}

fn menu_item(value: &serde_json::Value) -> Option<MenuItem> {
    let title = json_field(value, "title");
    let url = menu_path(value);
    if title.is_empty() && url.is_empty() {
        return None;
    }
    let children = value
        .get("subscreens")
        .and_then(serde_json::Value::as_array)
        .map(|list| list.iter().filter_map(menu_child).collect())
        .unwrap_or_default();
    Some(MenuItem { title, url, children })
}

/// One child level. A grandchild menu is not walked.
fn menu_child(value: &serde_json::Value) -> Option<MenuItem> {
    let title = json_field(value, "title");
    let url = menu_path(value);
    if title.is_empty() && url.is_empty() {
        None
    } else {
        Some(MenuItem {
            title,
            url,
            children: Vec::new(),
        })
    }
}

fn menu_path(value: &serde_json::Value) -> String {
    let with_params = json_field(value, "pathWithParams");
    if with_params.is_empty() {
        json_field(value, "path")
    } else {
        with_params
    }
}

fn menu_url(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() || path == "/" {
        "/menuData".to_string()
    } else if let Some(rest) = path.strip_prefix('/') {
        format!("/menuData/{rest}")
    } else {
        format!("/menuData/{path}")
    }
}

fn json_field(value: &serde_json::Value, name: &str) -> String {
    value.get(name).and_then(json_text).unwrap_or_default()
}

fn first_text(value: &serde_json::Value, names: &[&str]) -> String {
    for name in names {
        if let Some(text) = value.get(*name).and_then(json_text) {
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

fn json_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn string_map(value: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    let Some(object) = value.and_then(serde_json::Value::as_object) else {
        return BTreeMap::new();
    };
    object
        .iter()
        .filter_map(|(name, item)| json_text(item).map(|text| (name.clone(), text)))
        .collect()
}

fn snippet(text: &str) -> String {
    text.chars().take(400).collect::<String>().replace('\n', " ")
}

/// Resolve `url` against the directory of `base`. An absolute url keeps its own path. `..` applies after the last
/// segment of `base` is dropped, so `/apps/gallery/List` plus `../Detail` is `/apps/Detail`.
pub fn resolve_url(base: &str, url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let (path, query) = split_query(url);
    let mut segments = if path.starts_with('/') {
        Vec::new()
    } else {
        let (base_path, _) = split_query(base);
        let mut segments = split_segments(base_path);
        segments.pop();
        segments
    };
    segments.extend(split_segments(path));
    let mut stack = Vec::new();
    for segment in segments {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    let mut out = format!("/{}", stack.join("/"));
    if stack.is_empty() {
        out = "/".to_string();
    }
    if let Some(query) = query {
        out.push('?');
        out.push_str(query);
    }
    out
}

fn split_query(url: &str) -> (&str, Option<&str>) {
    match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    }
}

fn split_segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|segment| !segment.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::resolve_url;

    #[test]
    fn a_relative_url_drops_the_last_segment_before_dot_dot() {
        assert_eq!(resolve_url("/apps/gallery/List", "../Detail"), "/apps/Detail");
        assert_eq!(
            resolve_url("/apps/SimpleScreens/Party/FindParty", "../EditParty?partyId=1"),
            "/apps/SimpleScreens/EditParty?partyId=1"
        );
        assert_eq!(
            resolve_url("/apps/gallery/List", "/apps/gallery/Detail?enumId=A"),
            "/apps/gallery/Detail?enumId=A"
        );
        assert_eq!(
            resolve_url("/apps/gallery/List?pageIndex=1", "detail"),
            "/apps/gallery/detail"
        );
        assert_eq!(resolve_url("/apps/gallery/List", "./Detail"), "/apps/gallery/Detail");
    }
}
