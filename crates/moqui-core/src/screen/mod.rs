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

//! The screen facade: definitions, the URL walk, and rendering.
//!
//! `definition` parses a screen, `cache` holds parsed screens and the `screen-facade` conf,
//! `url` resolves a path against the tree, and `render` runs actions and sends the response.
//! `dispatch` holds the Rust bodies for the `rest.xml` transitions. Widgets are slice 2.

mod cache;
mod definition;
mod dispatch;
mod render;
mod schema;
mod url;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use moqui_lib::object::Value;

use crate::context::{Error, ErrorKind, ExecutionContextFactory};
use crate::user::{self, apply_header_keys, login_user};
use crate::web::{self, WebRequest, WebResponse};

pub use cache::{canonical_render_mode, ConfScreen, ScreenConf, ScreenFacade, TextOutput};
pub use definition::{
    Actions, Authentication, Condition, FormDef, ParameterDef, ResponseDef, ResponseKind, ScreenDef, SectionDef,
    SectionKind, Subscreens, SubscreensItem, TransitionDef, TreeDef, WebSettings, XmlFragment,
};
pub use render::{
    make_render, save_screen_last_info, send_redirect_and_stop_render, with_current_render, RenderOutput, RenderState,
    ScreenRender,
};
pub use url::{parse_last_standalone, resolve_screen_path, ScreenUrlInfo};

const ROOT_SCREEN: &str = "component://webroot/screen/webroot.xml";

/// Where a path landed.
#[derive(Clone, Debug)]
pub struct ScreenHit {
    pub transition: Option<String>,
    pub extra: Vec<String>,
    /// Screen file names from the root, joined with `/`.
    pub screen: String,
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
    factory: &Arc<ExecutionContextFactory>,
    request: &WebRequest,
    root: &str,
    parameters: &BTreeMap<String, Value>,
    json_list: Option<&[Value]>,
    login_error: Option<&str>,
    header_authenticated: bool,
) -> Result<WebResponse, Error> {
    let job = render::Job {
        request,
        root,
        path: &request.path,
        method: &request.method,
        parameters,
        json_list,
        login_error,
        header_authenticated,
        render_mode: None,
        enforce_request_checks: true,
        started: Instant::now(),
    };
    if let Err(err) = render::run_job(factory, &job) {
        // Only a kind names a client error. A script or load failure is a 500 whatever it says.
        let status = if err.kind() == ErrorKind::General {
            500
        } else {
            web::status_for(&err)
        };
        return Ok(web::json_error(status, err.message()));
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

/// Walk `path` from `root` without binding a session.
///
/// # Errors
///
/// Returns `ErrorKind::NotFound` when the path matches no screen or transition.
pub fn locate(factory: &ExecutionContextFactory, root: &str, path: &str) -> Result<ScreenHit, Error> {
    let info = ScreenUrlInfo::parse(factory, root, path, 0)?;
    Ok(ScreenHit {
        transition: info.target_transition_name.clone(),
        extra: info.extra_path.clone(),
        screen: info
            .path_defs
            .iter()
            .map(|def| def.screen_name.as_str())
            .collect::<Vec<_>>()
            .join("/"),
    })
}

fn text_of(value: Option<&Value>) -> String {
    value.map(moqui_lib::object::to_plain_string).unwrap_or_default()
}

#[cfg(test)]
mod tests;
