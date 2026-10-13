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

//! Screen URL walk and the transitions `rest.xml` runs in Rust.
//!
//! Widget rendering, always-actions, and pre-actions wait for phase 8. The walk only
//! needs the transition name, method, parameters, session-token flag, and response type.

mod dispatch;
mod schema;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use crate::context::{Error, ExecutionContextFactory};
use crate::user::{self, apply_header_keys, login_user};
use crate::web::{self, WebRequest, WebResponse};

const ROOT_SCREEN: &str = "component://webroot/screen/webroot.xml";
const SCREEN_LATER: &str = "screen render is not available until phase 8";

/// Where a path landed.
#[derive(Clone, Debug)]
pub struct ScreenHit {
    pub transition: Option<String>,
    pub extra: Vec<String>,
    pub screen: String,
}

#[derive(Clone, Debug)]
pub(crate) struct TransitionDef {
    name: String,
    method: Option<String>,
    /// `None` uses the webapp default. `Some(false)` skips the check.
    require_session_token: Option<bool>,
    required: Vec<String>,
    response_type: String,
    response_url: String,
}

#[derive(Clone, Debug)]
struct ScreenDef {
    location: String,
    /// `None` and `Some(true)` require a user. `Some(false)` lets the transition decide.
    require_authentication: Option<bool>,
    transitions: Vec<TransitionDef>,
    subscreens: BTreeMap<String, String>,
}

struct Pushed(usize);

impl Drop for Pushed {
    fn drop(&mut self) {
        for _ in 0..self.0 {
            crate::context::ExecutionContext::with_current(|ec| {
                let _ = ec.pop();
            });
        }
    }
}

/// Handle one request from `component://webroot/screen/webroot.xml`.
pub fn serve(factory: &Arc<ExecutionContextFactory>, request: WebRequest) -> WebResponse {
    serve_at(factory, request, ROOT_SCREEN)
}

/// Handle one request whose root screen is `root`.
pub fn serve_at(factory: &Arc<ExecutionContextFactory>, request: WebRequest, root: &str) -> WebResponse {
    if request.body.len() > 1024 * 1024 {
        return web::early_error(&request, 413, "request body is larger than 1 MiB");
    }
    if request
        .header("content-type")
        .unwrap_or("")
        .to_ascii_lowercase()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .starts_with("multipart/")
    {
        return web::early_error(&request, 415, "multipart bodies are not available until phase 8");
    }
    if request.method.eq_ignore_ascii_case("options") {
        return web::finish_response(&request, WebResponse::new(204, Vec::new()), true);
    }
    let parsed = match web::parse_body(&request) {
        Ok(parsed) => parsed,
        Err(web::BodyError::TooLarge) => return web::early_error(&request, 413, "request body is larger than 1 MiB"),
        Err(web::BodyError::Multipart) => {
            return web::early_error(&request, 415, "multipart bodies are not available until phase 8")
        }
        Err(web::BodyError::Json(message)) => return web::early_error(&request, 400, &message),
    };
    // A request-pool worker already entered a fresh context for this job. Direct callers get one here.
    let context = crate::context::ExecutionContext::current()
        .is_none()
        .then(|| factory.new_context());
    let _entered = context.as_ref().map(|context| context.enter());
    let response = match handle(factory, &request, root, &parsed.parameters, parsed.json_list.as_deref()) {
        Ok(response) => response,
        Err(err) => web::json_error(500, err.message()),
    };
    web::finish_response(&request, response, false)
}

/// Walk `path` from `root` without binding a session.
pub fn locate(factory: &ExecutionContextFactory, root: &str, path: &str) -> Result<ScreenHit, Error> {
    let segments = path_segments(path);
    let located = walk(factory, root, &segments, false)?;
    Ok(ScreenHit {
        transition: located.transition.map(|transition| transition.name),
        extra: located.extra,
        screen: located.screen_path,
    })
}

struct Located {
    transition: Option<TransitionDef>,
    extra: Vec<String>,
    screen_path: String,
    require_user: bool,
    /// Keeps the artifact frames pushed by the walk until the located transition has run.
    _frames: Pushed,
}

fn handle(
    factory: &Arc<ExecutionContextFactory>,
    request: &WebRequest,
    root: &str,
    parameters: &BTreeMap<String, Value>,
    json_list: Option<&[Value]>,
) -> Result<WebResponse, Error> {
    let session_cookie = web::session_cookie_of(request);
    let visitor_cookie = web::visitor_cookie_of(request);
    let presented = session_cookie
        .as_deref()
        .map(user::verify_session_cookie)
        .transpose()?
        .flatten();
    let guard = user::bind(
        factory,
        session_cookie.as_deref(),
        visitor_cookie.as_deref(),
        request.https,
    )?;
    let token_before = peek_token();
    if let Err(err) = user::session::remember_client_ip(&web::client_ip(request)) {
        return Ok(web::json_error(500, err.message()));
    }
    let _web = web::install(parameters.clone());
    let (login_error, header_authenticated) = authenticate(request, parameters)?;
    let response = dispatch_request(
        factory,
        request,
        root,
        parameters,
        json_list,
        login_error.as_deref(),
        header_authenticated,
    )?;
    let response = attach_session(
        response,
        &guard,
        presented.as_deref(),
        visitor_cookie.as_deref(),
        token_before,
    )?;
    drop(guard);
    Ok(response)
}

fn dispatch_request(
    factory: &ExecutionContextFactory,
    request: &WebRequest,
    root: &str,
    parameters: &BTreeMap<String, Value>,
    json_list: Option<&[Value]>,
    login_error: Option<&str>,
    header_authenticated: bool,
) -> Result<WebResponse, Error> {
    let segments = path_segments(&request.path);
    let located = match walk(factory, root, &segments, true) {
        Ok(located) => located,
        Err(err) if err.message().contains("path not found") => return Ok(web::json_error(404, err.message())),
        Err(err) => return Err(err),
    };
    if located.require_user && user::current_username()?.is_none() {
        return Ok(web::json_error(401, "Authentication required"));
    }
    let started = Instant::now();
    if let Some(transition) = &located.transition {
        if let Some(response) = method_rejected(request, transition) {
            return Ok(response);
        }
        if let Some(name) = missing_parameter(parameters, transition) {
            return Ok(web::json_error(400, &format!("Missing required parameter {name}")));
        }
        if !header_authenticated && csrf_rejected(request, parameters, transition)? {
            return Ok(web::json_error(401, "Session token is missing or is not valid"));
        }
        dispatch::run(&dispatch::Call {
            request,
            transition,
            extra: &located.extra,
            parameters,
            json_list,
            login_error,
            started,
        })?;
    } else {
        web::send_json_error(501, SCREEN_LATER);
    }
    if let Err(err) = user::end_request() {
        if !web::output_sent() {
            return Ok(web::json_error(500, err.message()));
        }
    }
    let (status, headers, body, sent) = web::take_output();
    Ok(WebResponse {
        status: if sent { status } else { 200 },
        headers,
        body,
    })
}

fn attach_session(
    mut response: WebResponse,
    guard: &user::SessionGuard,
    presented: Option<&str>,
    visitor_cookie: Option<&str>,
    token_before: Option<String>,
) -> Result<WebResponse, Error> {
    if presented != Some(guard.session_id().as_str()) {
        response
            .headers
            .push(("Set-Cookie".to_string(), guard.session_cookie_header()?));
    }
    if guard.visitor_id().as_deref() != visitor_cookie {
        if let Some(cookie) = guard.visitor_cookie_header() {
            response.headers.push(("Set-Cookie".to_string(), cookie));
        }
    }
    let token_after = user::csrf_token()?;
    if presented != Some(guard.session_id().as_str()) || token_before.as_deref() != Some(token_after.as_str()) {
        response
            .headers
            .push(("moquiSessionToken".to_string(), token_after.clone()));
        response.headers.push(("X-CSRF-Token".to_string(), token_after));
    }
    Ok(response)
}

fn peek_token() -> Option<String> {
    user::session::peek_csrf_token()
}

/// The login error, if any, and whether this request logged a user in from its own credentials
/// (Basic, `api_key`, `login_key`, or `authUsername`). Such a request carries no ambient cookie
/// authority, so it skips the session-token check as it does in the Java `ScreenRenderImpl`.
fn authenticate(request: &WebRequest, parameters: &BTreeMap<String, Value>) -> Result<(Option<String>, bool), Error> {
    let mut login_error = None;
    let mut by_header = false;
    if let Some(header) = request.header("authorization") {
        if let Some((username, password)) = web::basic_credentials(header) {
            match login_user(&username, &password) {
                Ok(()) => by_header = true,
                Err(err) => {
                    let message = err.message().to_string();
                    web::record_login_error(&message);
                    login_error = Some(message);
                }
            }
        }
    }
    if user::current_username()?.is_none() {
        let keyed = request.header("api_key").is_some_and(|text| !text.is_empty())
            || request.header("login_key").is_some_and(|text| !text.is_empty());
        if let Err(err) = apply_header_keys(request.header("api_key"), request.header("login_key")) {
            let message = err.message().to_string();
            web::record_login_error(&message);
            login_error = Some(message);
        } else if keyed && user::current_username()?.is_some() {
            by_header = true;
        }
    }
    if user::current_username()?.is_none() {
        let username = text_of(parameters.get("authUsername"));
        let password = text_of(parameters.get("authPassword"));
        if !username.is_empty() && !password.is_empty() {
            match login_user(&username, &password) {
                Ok(()) => by_header = true,
                Err(err) => {
                    let message = err.message().to_string();
                    web::record_login_error(&message);
                    login_error = Some(message);
                }
            }
        }
    }
    Ok((login_error, by_header))
}

fn method_rejected(request: &WebRequest, transition: &TransitionDef) -> Option<WebResponse> {
    let expected = transition.method.as_deref()?;
    if request.method.eq_ignore_ascii_case(expected) {
        return None;
    }
    let mut response = web::json_error(405, &format!("Method {} is not supported", request.method));
    response
        .headers
        .push(("Allow".to_string(), expected.to_ascii_uppercase()));
    Some(response)
}

fn missing_parameter(parameters: &BTreeMap<String, Value>, transition: &TransitionDef) -> Option<String> {
    transition
        .required
        .iter()
        .find(|name| parameter_missing(parameters, name))
        .cloned()
}

fn parameter_missing(parameters: &BTreeMap<String, Value>, name: &str) -> bool {
    match parameters.get(name) {
        None => true,
        Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(_) => false,
    }
}

fn csrf_rejected(
    request: &WebRequest,
    parameters: &BTreeMap<String, Value>,
    transition: &TransitionDef,
) -> Result<bool, Error> {
    if !matches!(
        request.method.to_ascii_lowercase().as_str(),
        "post" | "put" | "patch" | "delete"
    ) {
        return Ok(false);
    }
    let required = match transition.require_session_token {
        Some(flag) => flag,
        None => web::require_session_token(),
    };
    if !required {
        return Ok(false);
    }
    let expected = user::csrf_token()?;
    if expected.is_empty() {
        return Ok(false);
    }
    let presented = text_of(parameters.get("moquiSessionToken"));
    let presented = if presented.is_empty() {
        request.header("x-csrf-token").unwrap_or("").to_string()
    } else {
        presented
    };
    Ok(!constant_time_eq(presented.as_bytes(), expected.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0_u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn walk(factory: &ExecutionContextFactory, root: &str, segments: &[String], push: bool) -> Result<Located, Error> {
    let mut screen = load_screen(factory, root)?;
    let mut screen_path = stem_of(root);
    let mut pushed = Pushed(0);
    let mut require_user = screen.require_authentication != Some(false);
    if push {
        push_frame(
            "AT_XML_SCREEN",
            &screen_path,
            screen.require_authentication != Some(false),
        )?;
        pushed.0 += 1;
    }
    let mut index = 0;
    while index < segments.len() {
        let segment = &segments[index];
        if let Some(transition) = screen.transitions.iter().find(|item| &item.name == segment) {
            if push {
                let name = format!("{screen_path}/{segment}");
                push_frame(
                    "AT_XML_SCREEN_TRANS",
                    &name,
                    screen.require_authentication != Some(false),
                )?;
                pushed.0 += 1;
            }
            return Ok(Located {
                transition: Some(transition.clone()),
                extra: segments[index + 1..].to_vec(),
                screen_path,
                require_user,
                _frames: pushed,
            });
        }
        let child = screen
            .subscreens
            .get(segment)
            .cloned()
            .unwrap_or_else(|| convention_location(&screen.location, segment));
        if !screen_exists(factory, &child)? {
            return Err(Error::new(format!("screen path not found: {segment}")));
        }
        screen = load_screen(factory, &child)?;
        screen_path = format!("{screen_path}/{segment}");
        require_user = screen.require_authentication != Some(false);
        if push {
            push_frame(
                "AT_XML_SCREEN",
                &screen_path,
                screen.require_authentication != Some(false),
            )?;
            pushed.0 += 1;
        }
        index += 1;
    }
    Ok(Located {
        transition: None,
        extra: Vec::new(),
        screen_path,
        require_user,
        _frames: pushed,
    })
}

fn push_frame(artifact_type: &str, name: &str, requires_authz: bool) -> Result<(), Error> {
    crate::context::ExecutionContext::with_current(|ec| {
        ec.push_checked(artifact_type, name, "view", requires_authz, false, None)
    })
    .unwrap_or(Ok(()))
}

fn load_screen(factory: &ExecutionContextFactory, location: &str) -> Result<ScreenDef, Error> {
    let resource = factory.resource().get(location)?;
    if !resource.exists() {
        return Err(Error::new(format!("screen not found: {location}")));
    }
    let text = resource.read_text()?;
    let node = MNode::parse_text(location, &text).map_err(Error::from)?;
    if node.name() != "screen" {
        return Err(Error::new(format!("{location} root is {} and not screen", node.name())));
    }
    Ok(ScreenDef {
        location: location.to_string(),
        require_authentication: flag_attr(node.attribute("require-authentication")),
        transitions: node.children_named("transition").iter().map(transition_from).collect(),
        subscreens: subscreen_map(&node),
    })
}

fn transition_from(node: &MNode) -> TransitionDef {
    let response = node.first_named("default-response");
    let (response_type, response_url) = match &response {
        Some(response) => (
            response.attribute("type").unwrap_or_else(|| "none".to_string()),
            response.attribute("url").unwrap_or_default(),
        ),
        None => ("none".to_string(), String::new()),
    };
    TransitionDef {
        name: node.attribute("name").unwrap_or_default(),
        method: node
            .attribute("method")
            .map(|text| text.to_ascii_lowercase())
            .filter(|text| !text.is_empty()),
        require_session_token: flag_attr(node.attribute("require-session-token")),
        required: node
            .children_named("parameter")
            .iter()
            .filter(|parameter| parameter.attribute("required").as_deref() == Some("true"))
            .filter_map(|parameter| parameter.attribute("name"))
            .filter(|name| !name.is_empty())
            .collect(),
        response_type,
        response_url,
    }
}

fn subscreen_map(screen: &MNode) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for node in screen.children_named("subscreens-item") {
        insert_subscreen(&mut map, &node);
    }
    for subscreens in screen.children_named("subscreens") {
        for node in subscreens.children_named("subscreens-item") {
            insert_subscreen(&mut map, &node);
        }
    }
    map
}

fn insert_subscreen(map: &mut BTreeMap<String, String>, node: &MNode) {
    let Some(name) = node.attribute("name").filter(|text| !text.is_empty()) else {
        return;
    };
    let Some(location) = node.attribute("location").filter(|text| !text.is_empty()) else {
        return;
    };
    map.insert(name, location);
}

fn screen_exists(factory: &ExecutionContextFactory, location: &str) -> Result<bool, Error> {
    Ok(factory.resource().get(location)?.exists())
}

fn convention_location(parent: &str, name: &str) -> String {
    let slash = parent.rfind('/').map(|index| index + 1).unwrap_or(0);
    let file = &parent[slash..];
    let stem = file.strip_suffix(".xml").unwrap_or(file);
    format!("{}{stem}/{name}.xml", &parent[..slash])
}

fn stem_of(location: &str) -> String {
    let file = location.rsplit('/').next().unwrap_or(location);
    file.strip_suffix(".xml").unwrap_or(file).to_string()
}

fn path_segments(path: &str) -> Vec<String> {
    let path = path.split('?').next().unwrap_or(path);
    let path = path.split('#').next().unwrap_or(path);
    path.split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn flag_attr(value: Option<String>) -> Option<bool> {
    match value.as_deref() {
        Some("false") => Some(false),
        Some("true") => Some(true),
        _ => None,
    }
}

fn text_of(value: Option<&Value>) -> String {
    value.map(moqui_lib::object::to_plain_string).unwrap_or_default()
}

#[cfg(test)]
mod tests;
