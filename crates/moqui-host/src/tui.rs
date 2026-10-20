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

//! The in-process [`ScreenLink`](moqui_tui::ScreenLink) (plans/Screen-TUI-Impl.md 2.7, 4.1). Each call runs on the
//! factory's request pool in a fresh execution context, logged in as the link's user, as the SSH shell's commands are;
//! the client holds no permission logic. Until a user is logged in, every call answers [`LinkError::Auth`], so the client
//! shows its login dialog.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use moqui_core::context::{Error, ErrorKind, ExecutionContextFactory};
use moqui_core::screen::RenderOutput;
use moqui_lib::object::Value;
use moqui_tui::{
    query_parameters, Choice, LinkError, MenuItem, Messages, ScreenLink, ScreenModel, ShellInfo, TransitionOutcome,
    ValidationError,
};

/// One screen client bound to a running factory.
#[derive(Clone)]
pub struct HostLink {
    factory: Arc<ExecutionContextFactory>,
    /// The user the calls run as, once logged in.
    user: Arc<Mutex<Option<String>>>,
    /// The path of the last level that rendered, used to resolve a relative transition or options url.
    last_path: Arc<Mutex<String>>,
}

impl HostLink {
    /// A link with no user: the client logs in through its dialog first.
    pub fn new(factory: Arc<ExecutionContextFactory>) -> Self {
        Self {
            factory,
            user: Arc::new(Mutex::new(None)),
            last_path: Arc::new(Mutex::new(String::new())),
        }
    }

    /// A link already logged in as `username`: `moqui tui --user` on a dev instance, and the tests.
    pub fn as_user(factory: Arc<ExecutionContextFactory>, username: impl Into<String>) -> Self {
        let link = Self::new(factory);
        *link.user.lock().unwrap_or_else(PoisonError::into_inner) = Some(username.into());
        link
    }

    /// The drawer for `path`, from the `menuData` transition.
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

    fn user(&self) -> Option<String> {
        self.user.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn absolute(&self, url: &str) -> String {
        let base = self.last_path.lock().unwrap_or_else(PoisonError::into_inner).clone();
        resolve_url(&base, url)
    }

    /// Run `work` on the request pool as the link's user, waiting for its answer. The client calls this off its UI
    /// thread, so the wait holds a worker of the client's, never the keys.
    fn on_pool<T, F>(&self, work: F) -> Result<T, LinkError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, Error> + Send + 'static,
    {
        let Some(username) = self.user() else {
            return Err(LinkError::Auth("Authentication required".into()));
        };
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
                .history(true)
                .render()
        })?;
        let model = model_from(&output)?;
        *self.last_path.lock().unwrap_or_else(PoisonError::into_inner) = model.screen.path.clone();
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
            method.to_ascii_uppercase()
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
        let text = output.body_text();
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return outcome_from(&output);
        }
        // Not JSON: an error page, or a form whose transition is the screen itself, which answers the screen; then the
        // posted fields are the parameters of that screen.
        check_status(&output)?;
        Ok(screen_repost(&url, fields))
    }

    fn options(&self, url: &str, parameters: &BTreeMap<String, String>) -> Result<Vec<Choice>, LinkError> {
        let url = self.absolute(url);
        let values = value_map(parameters);
        let factory = Arc::clone(&self.factory);
        let asked = url.clone();
        let result = self
            .on_pool(move || {
                moqui_core::screen::make_render(&factory)
                    .screen_path(asked)
                    .parameters(values)
                    .header("Accept", "application/json")
                    .render()
            })
            .and_then(|output| choices_from(&output));
        if let Err(err) = &result {
            tracing::warn!("TUI options from {url} failed: {err}");
        }
        result
    }

    fn shell(&self, path: &str) -> Result<ShellInfo, LinkError> {
        let mut shell = ShellInfo {
            user: self.user().unwrap_or_default(),
            ..ShellInfo::default()
        };
        // The drawer of the current app; a screen without one has none, which is not an error.
        match self.menu(path) {
            Ok(menu) => shell.menu = app_menu(menu, path),
            Err(LinkError::Auth(text)) => return Err(LinkError::Auth(text)),
            Err(err) => tracing::warn!("TUI menu for {path} failed: {err}"),
        }
        let factory = Arc::clone(&self.factory);
        let output = self.on_pool(move || {
            moqui_core::screen::make_render(&factory)
                .screen_path("/shellData")
                .render()
        })?;
        match serde_json::from_str::<serde_json::Value>(&output.body_text()) {
            Ok(data) => tui_values(&data, &mut shell),
            Err(err) => tracing::warn!("TUI shell data was not JSON: {err}"),
        }
        Ok(shell)
    }

    fn login(&self, username: &str, password: &str, _code: &str) -> Result<(), LinkError> {
        let factory = Arc::clone(&self.factory);
        let name = username.to_string();
        let secret = password.to_string();
        let rx = self
            .factory
            .request_pool()
            .try_submit(Arc::clone(&factory), move || {
                moqui_core::user::login_user(&name, &secret)?;
                // A user with a second factor is not logged in yet; the browser asks for the code.
                match moqui_core::user::current_user_id()? {
                    Some(id) if !id.is_empty() => Ok(()),
                    _ => Err(Error::new(
                        "This account needs a second factor; log in from the browser",
                    )),
                }
            })
            .map_err(link_error)?;
        match rx.recv() {
            Ok(Ok(())) => {
                *self.user.lock().unwrap_or_else(PoisonError::into_inner) = Some(username.to_string());
                Ok(())
            }
            Ok(Err(err)) => Err(LinkError::Remote(err.message().to_string())),
            Err(_) => Err(LinkError::Remote("request worker dropped the result".into())),
        }
    }
}

fn link_error(err: Error) -> LinkError {
    if err.kind() == ErrorKind::AuthenticationRequired {
        return LinkError::Auth("Authentication required".into());
    }
    LinkError::Remote(err.to_string())
}

fn value_map(fields: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), Value::string(value)))
        .collect()
}

/// A 401 is a login; another status of 400 or more is an error with the start of the body.
fn check_status(output: &RenderOutput) -> Result<(), LinkError> {
    if output.status == 401 {
        return Err(LinkError::Auth("Authentication required".into()));
    }
    if output.status >= 400 {
        return Err(LinkError::Remote(format!(
            "HTTP {}: {}",
            output.status,
            snippet(&output.body_text())
        )));
    }
    Ok(())
}

fn model_from(output: &RenderOutput) -> Result<ScreenModel, LinkError> {
    check_status(output)?;
    let text = output.body_text();
    ScreenModel::from_json(&text).map_err(|err| LinkError::Remote(format!("{err}: {}", snippet(&text))))
}

/// A screen rendered in place of a transition: show that screen with the posted fields.
fn screen_repost(url: &str, fields: &BTreeMap<String, String>) -> TransitionOutcome {
    let (path, query) = split_query(url);
    let mut parameters = query.map(query_parameters).unwrap_or_default();
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(char::from(byte)),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A transition's JSON answer: `send_json_redirect`'s `screenUrl`, `messageInfos` and `validationErrors`, or
/// `sendJsonResponse`'s `messages` and `errors`. A status of 400 or more is never a navigation: its errors stay on the
/// screen with the edits.
fn outcome_from(output: &RenderOutput) -> Result<TransitionOutcome, LinkError> {
    if output.status == 401 {
        return Err(LinkError::Auth("Authentication required".into()));
    }
    let text = output.body_text();
    let value = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value,
        Err(_) if output.status >= 400 => {
            return Err(LinkError::Remote(format!("HTTP {}: {}", output.status, snippet(&text))));
        }
        Err(err) => return Err(LinkError::Remote(format!("transition JSON: {err}"))),
    };
    let mut outcome = outcome_of(&value);
    if output.status >= 400 {
        outcome.screen_url.clear();
        if outcome.messages.errors.is_empty() && outcome.messages.validation_errors.is_empty() {
            outcome
                .messages
                .errors
                .push(format!("HTTP {}: {}", output.status, snippet(&text)));
        }
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
            if is_error_type(&json_field(item, "type")) {
                messages.errors.push(text);
            } else {
                messages.info.push(text);
            }
        }
    }
    for (key, errors) in [("messages", false), ("errors", true)] {
        match value.get(key) {
            Some(serde_json::Value::String(text)) if !text.is_empty() => {
                if errors {
                    &mut messages.errors
                } else {
                    &mut messages.info
                }
                .push(text.clone());
            }
            Some(serde_json::Value::Array(items)) => {
                for text in items.iter().filter_map(json_text).filter(|text| !text.is_empty()) {
                    if errors {
                        &mut messages.errors
                    } else {
                        &mut messages.info
                    }
                    .push(text);
                }
            }
            _ => {}
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
    check_status(output)?;
    let text = output.body_text();
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
    check_status(output)?;
    let text = output.body_text();
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| LinkError::Remote(format!("menu JSON: {err}")))?;
    let Some(list) = value.as_array() else {
        return Err(LinkError::Remote("menu was not a list".into()));
    };
    Ok(list.iter().filter_map(menu_item).collect())
}

/// The drawer shows the app the screen is in: the menu level of the app root and its screens.
fn app_menu(menu: Vec<MenuItem>, path: &str) -> Vec<MenuItem> {
    let names: Vec<&str> = path
        .split('?')
        .next()
        .unwrap_or(path)
        .split('/')
        .filter(|n| !n.is_empty())
        .collect();
    if names.len() < 2 {
        return menu;
    }
    let app = format!("/{}/{}", names[0], names[1]);
    let found: Vec<MenuItem> = menu
        .iter()
        .filter(|item| item.url.split('?').next() == Some(app.as_str()))
        .cloned()
        .collect();
    if found.is_empty() {
        menu
    } else {
        found
    }
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

/// The `tui` values of the shell data: breakpoints, colors and keys (section 2.6).
fn tui_values(data: &serde_json::Value, shell: &mut ShellInfo) {
    let Some(tui) = data.get("tui") else {
        return;
    };
    if let Some(map) = tui.get("breakpoints").and_then(serde_json::Value::as_object) {
        for (name, value) in map {
            if let Some(columns) = value.as_u64().and_then(|n| u16::try_from(n).ok()) {
                shell.breakpoints.insert(name.clone(), columns);
            }
        }
    }
    shell.colors = string_map(tui.get("colors"));
    shell.keys = string_map(tui.get("keys"));
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
    use super::*;

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

    #[test]
    fn a_failed_transition_is_never_a_navigation_and_keeps_its_errors() {
        let output = |status: u16, body: &str| RenderOutput {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        };
        let failed = outcome_from(&output(
            500,
            r#"{"errorCode":500,"errors":"Party is locked","screenUrl":"/x"}"#,
        ))
        .unwrap();
        assert!(failed.screen_url.is_empty());
        assert_eq!(failed.messages.errors, ["Party is locked"]);
        assert!(matches!(outcome_from(&output(401, "{}")), Err(LinkError::Auth(_))));
        let saved = outcome_from(&output(200, r#"{"messages":"Saved","screenUrl":"/y"}"#)).unwrap();
        assert_eq!(saved.messages.info, ["Saved"]);
        assert_eq!(saved.screen_url, "/y");
        let repost = screen_repost("/apps/x?a=%C3%A9&b=100%", &BTreeMap::new());
        assert_eq!(repost.parameters.get("a").map(String::as_str), Some("é"));
        assert_eq!(repost.parameters.get("b").map(String::as_str), Some("100%"));
    }
}
