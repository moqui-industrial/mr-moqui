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

//! `ScreenRender`: resolve a path, run actions down it, and send a transition response or a screen.
//!
//! The pipeline follows `ScreenRenderImpl.internalRender`. For a screen: `always-actions` for
//! every screen in the path from index 0, `pre-actions` from `renderPathDifference`, then
//! [`render_body`], which slice 1 fills with the root-section actions and an empty body. For a
//! transition: `always-actions`, the transition's actions, and the first response that holds.
//!
//! Output goes to the thread's web slot. A request from the host already installed one.
//! [`make_render`] installs its own from a parameter map, a session map, and a base URL, so it
//! needs no socket.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use moqui_lib::object::Value;

use super::cache::{canonical_render_mode, ScreenFacade};
use super::definition::{
    Actions, Authentication, Condition, ParameterDef, ResponseDef, ResponseKind, ScreenDef, TransitionDef,
    CONDITION_FIELD,
};
use super::dispatch;
use super::url::{encode_path, parse_last_standalone, resolve_screen_path, ScreenUrlInfo};
use crate::context::{Error, ErrorKind, ExecutionContext, ExecutionContextFactory};
use crate::script::{self, ContextFrame};
use crate::user;
use crate::web::{self, WebRequest};

const DEFAULT_ENCODING: &str = "UTF-8";
const DEFAULT_MODE: &str = "html";

/// The result of a [`ScreenRender`]: what the host would send.
#[derive(Clone, Debug)]
pub struct RenderOutput {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RenderOutput {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn content_type(&self) -> Option<&str> {
        self.header("Content-Type")
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A screen render built with [`make_render`].
pub struct ScreenRender {
    factory: Arc<ExecutionContextFactory>,
    root: String,
    path: String,
    method: String,
    parameters: BTreeMap<String, Value>,
    session: BTreeMap<String, Value>,
    base_url: String,
    render_mode: Option<String>,
}

/// A render that needs no socket. Set the root screen and path, then call [`ScreenRender::render`].
pub fn make_render(factory: &Arc<ExecutionContextFactory>) -> ScreenRender {
    ScreenRender {
        factory: Arc::clone(factory),
        root: super::ROOT_SCREEN.to_string(),
        path: String::new(),
        method: "GET".to_string(),
        parameters: BTreeMap::new(),
        session: BTreeMap::new(),
        base_url: String::new(),
        render_mode: None,
    }
}

impl ScreenRender {
    /// Location of the root screen. The default is `webroot.xml`.
    #[must_use]
    pub fn root_screen(mut self, location: impl Into<String>) -> Self {
        self.root = location.into();
        self
    }

    /// The screen path under the root, such as `/apps/Foo.csv`. A `?a=b` tail adds parameters.
    #[must_use]
    pub fn screen_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// HTTP method a transition sees. The default is `GET`.
    #[must_use]
    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    /// The request parameters: `ec.web.parameters`, and names in the context for actions.
    #[must_use]
    pub fn parameters(mut self, parameters: BTreeMap<String, Value>) -> Self {
        self.parameters = parameters;
        self
    }

    /// Session attributes, set before the render starts.
    #[must_use]
    pub fn session(mut self, session: BTreeMap<String, Value>) -> Self {
        self.session = session;
        self
    }

    /// What generated links start with, such as `https://example.com`.
    #[must_use]
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// A render mode that wins over the path extension and the `renderMode` parameter.
    #[must_use]
    pub fn render_mode(mut self, mode: impl Into<String>) -> Self {
        self.render_mode = Some(mode.into());
        self
    }

    /// Run the render in this thread's execution context, or in a new one when none is entered.
    ///
    /// # Errors
    ///
    /// Returns the error the render raised. Its kind says 401, 403, or 404 for the web layer.
    pub fn render(self) -> Result<RenderOutput, Error> {
        let owned = ExecutionContext::current()
            .is_none()
            .then(|| self.factory.new_context());
        let _entered = owned.as_ref().map(ExecutionContext::enter);
        let _web = web::install_with(self.parameters.clone(), self.base_url.clone());
        for (name, value) in &self.session {
            user::session::set_session_attribute(name, Some(&moqui_lib::object::to_plain_string(value)));
        }
        let request = WebRequest::new(self.method.clone(), self.path.clone());
        let job = Job {
            request: &request,
            root: &self.root,
            path: &self.path,
            method: &self.method,
            parameters: &self.parameters,
            json_list: None,
            login_error: None,
            header_authenticated: true,
            render_mode: self.render_mode.as_deref(),
            enforce_request_checks: false,
            started: Instant::now(),
        };
        run_job(&self.factory, &job)?;
        let (status, headers, body, sent) = web::take_output();
        Ok(RenderOutput {
            status: if sent { status } else { 200 },
            headers,
            body,
        })
    }
}

/// One request into the screen tree.
pub(crate) struct Job<'a> {
    pub request: &'a WebRequest,
    pub root: &'a str,
    pub path: &'a str,
    pub method: &'a str,
    pub parameters: &'a BTreeMap<String, Value>,
    pub json_list: Option<&'a [Value]>,
    pub login_error: Option<&'a str>,
    pub header_authenticated: bool,
    pub render_mode: Option<&'a str>,
    /// False for a render with no socket: no session-token check.
    pub enforce_request_checks: bool,
    pub started: Instant,
}

/// What a running render exposes to the code that writes the screen. Slice 2 attaches `sri` here.
pub struct RenderState {
    pub url_info: Arc<ScreenUrlInfo>,
    /// Canonical render mode: `html-print` for `xsl-fo`.
    pub render_mode: String,
    pub content_type: String,
    pub character_encoding: String,
    /// The mode's `screen-text-output` says it is always standalone.
    pub always_standalone: bool,
    /// Index into `url_info.path_defs` of the screen being rendered.
    pub screen_path_index: Cell<usize>,
    /// Set by actions that sent their own response, so the screen is not rendered.
    pub dont_do_render: Cell<bool>,
    /// Locations of screens that are skipped because a required parameter is missing.
    pub stopped: RefCell<Vec<String>>,
}

thread_local! {
    static ACTIVE: RefCell<Vec<Rc<RenderState>>> = const { RefCell::new(Vec::new()) };
}

struct ActiveGuard;

impl ActiveGuard {
    fn install(state: Rc<RenderState>) -> Self {
        ACTIVE.with(|stack| stack.borrow_mut().push(state));
        Self
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Run `body` with the render in progress on this thread, if there is one.
pub fn with_current_render<R>(body: impl FnOnce(&RenderState) -> R) -> Option<R> {
    let state = ACTIVE.with(|stack| stack.borrow().last().cloned())?;
    Some(body(&state))
}

/// `sri.sendRedirectAndStopRender`: redirect and skip the screen. A standalone mode, which a
/// client router fetches, gets 205 and `X-Redirect-To` instead of a 302.
pub fn send_redirect_and_stop_render(url: &str) {
    let base = web::base_url();
    let location = if url.starts_with('/') && !url.starts_with("//") {
        format!("{}{url}", base.trim_end_matches('/'))
    } else {
        url.to_string()
    };
    let standalone = with_current_render(|state| {
        state.dont_do_render.set(true);
        state.always_standalone
    })
    .unwrap_or(false);
    if standalone {
        web::send_reset_content(&location);
    } else {
        web::send_redirect(&with_query(location, &BTreeMap::new()));
    }
}

/// `ec.web.saveScreenLastInfo`: remember `path`, or the screen being rendered, for a later
/// `screen-last` response such as the one after login.
pub fn save_screen_last_info(path: Option<&str>) {
    let current = with_current_render(|state| state.url_info.full_path_url());
    if let Some(path) = path.map(str::to_string).or(current) {
        user::session::push_screen_history(&path);
    }
}

/// Resolve and run one job. Anything written is in the thread's web slot.
pub(crate) fn run_job(factory: &Arc<ExecutionContextFactory>, job: &Job<'_>) -> Result<(), Error> {
    let last_standalone = parse_last_standalone(&text_of(job.parameters.get("lastStandalone")), 0);
    let info = ScreenUrlInfo::parse(factory, job.root, job.path, last_standalone)?;
    let facade = ScreenFacade::of(factory);
    check_web_settings(&info)?;
    check_authentication(&info)?;

    // Parameters live in a context frame for the request, so nothing leaks into the next one.
    let _frame = ContextFrame::push();
    for (name, value) in job.parameters {
        script::set_context_name(name, value.clone())?;
    }
    for (name, value) in &info.inline_parameters {
        if !job.parameters.contains_key(name) {
            script::set_context_name(name, Value::string(value))?;
        }
    }

    if info.target_transition_name.is_some() {
        return run_transition(factory, job, info);
    }

    let mode = pick_render_mode(&facade, job, &info)?;
    let target = Arc::clone(&info.target_screen);
    if !target.allows_render_mode(&mode) {
        return Err(Error::of_kind(
            ErrorKind::NotFound,
            format!("render mode {mode} is not supported by {}", target.location),
        ));
    }
    let (content_type, encoding) = output_type(&facade, &info, &mode);
    let always_standalone = facade.is_always_standalone(&mode);
    let state = Rc::new(RenderState {
        url_info: Arc::new(info),
        render_mode: mode,
        content_type,
        character_encoding: encoding,
        always_standalone,
        screen_path_index: Cell::new(0),
        dont_do_render: Cell::new(false),
        stopped: RefCell::new(Vec::new()),
    });
    let _active = ActiveGuard::install(Rc::clone(&state));
    let begin = state.url_info.begin_transaction;
    let timeout = state.url_info.transaction_timeout;
    in_transaction(begin, timeout, || render_screens(&state, &facade))
}

fn render_screens(state: &RenderState, facade: &ScreenFacade) -> Result<(), Error> {
    let info = &state.url_info;
    let defs = &info.path_defs;
    let last = defs.len().saturating_sub(1);
    {
        // always-actions for every screen in the path, each under the frames of the screens above it
        let mut frames = Frames::default();
        for (index, def) in defs.iter().enumerate() {
            frames.enter_screen(def, index == last)?;
            state.screen_path_index.set(index);
            if let Some(actions) = &def.always_actions {
                script::run_actions(&actions.xml)?;
            }
        }
    }

    // Screens before the render start only sit on the stack, unless the mode is always standalone:
    // then the parents' pre-actions run too, from the third screen on (not webroot and apps).
    let first_pre =
        if info.target_render_mode.is_some() && facade.is_always_standalone(&state.render_mode) && defs.len() > 2 {
            2
        } else {
            info.render_path_difference
        };
    let mut frames = Frames::default();
    for (index, def) in defs.iter().enumerate() {
        frames.enter_screen(def, index == last)?;
        if index >= first_pre {
            state.screen_path_index.set(index);
            if let Some(actions) = &def.pre_actions {
                script::run_actions(&actions.xml)?;
            }
        }
    }
    if state.dont_do_render.get() || web::output_sent() {
        return Ok(());
    }

    // Past always-actions and pre-actions it is too late to supply a required parameter.
    for def in info.render_defs() {
        for parameter in def.parameters.iter().filter(|parameter| parameter.required) {
            if is_empty(script::context_value(&parameter.name).as_ref()) {
                ExecutionContext::with_current(|ec| {
                    ec.message()
                        .add_error(format!("Required parameter missing ({})", parameter.name));
                });
                state.stopped.borrow_mut().push(def.location.clone());
            }
        }
    }
    state.screen_path_index.set(info.render_path_difference);
    render_body(state)
}

/// Where slice 2 plugs in the macro walker. It now runs each rendered screen's root-section
/// actions in order and sends the headers with an empty body. The walker takes over from here:
/// it runs the sections itself and writes through [`web::write_output`].
fn render_body(state: &RenderState) -> Result<(), Error> {
    for index in state.url_info.render_path_difference..state.url_info.path_defs.len() {
        let def = &state.url_info.path_defs[index];
        if state.stopped.borrow().contains(&def.location) {
            continue;
        }
        state.screen_path_index.set(index);
        let root = &def.root;
        let holds = match &root.condition {
            Some(condition) => check_condition(condition)?,
            None => true,
        };
        if holds {
            if let Some(actions) = &root.actions {
                script::run_actions(&actions.xml)?;
            }
        }
    }
    if web::output_sent() || state.dont_do_render.get() {
        return Ok(());
    }
    web::start_output(
        200,
        &format!("{};charset={}", state.content_type, state.character_encoding),
    );
    Ok(())
}

fn pick_render_mode(facade: &ScreenFacade, job: &Job<'_>, info: &ScreenUrlInfo) -> Result<String, Error> {
    let parameter = text_of(job.parameters.get("renderMode"));
    let chosen = job
        .render_mode
        .map(str::to_string)
        .filter(|text| !text.is_empty())
        .or_else(|| Some(parameter).filter(|text| !text.is_empty()))
        .or_else(|| info.target_render_mode.clone())
        .unwrap_or_else(|| DEFAULT_MODE.to_string());
    if !facade.is_render_mode_valid(&chosen) {
        return Err(Error::of_kind(
            ErrorKind::NotFound,
            format!("no such render mode: {chosen}"),
        ));
    }
    Ok(canonical_render_mode(&chosen).to_string())
}

/// Content type and encoding. A `web-settings` mime type on a screen in the render path wins over
/// the mode's `screen-text-output`, as in Java.
fn output_type(facade: &ScreenFacade, info: &ScreenUrlInfo, mode: &str) -> (String, String) {
    let mut content_type = None;
    let mut encoding = None;
    for def in info.render_defs() {
        if let Some(mime) = &def.web.mime_type {
            content_type = Some(mime.clone());
        }
        if let Some(found) = &def.web.character_encoding {
            encoding = Some(found.clone());
        }
    }
    let content_type = content_type
        .or_else(|| facade.mime_type(mode).map(str::to_string))
        .unwrap_or_else(|| "text/html".to_string());
    (content_type, encoding.unwrap_or_else(|| DEFAULT_ENCODING.to_string()))
}

fn check_web_settings(info: &ScreenUrlInfo) -> Result<(), Error> {
    for def in info.render_defs() {
        if !def.web.allow_web_request {
            return Err(Error::of_kind(
                ErrorKind::Unauthorized,
                format!(
                    "The screen {} cannot be used in a web request (allow-web-request=false)",
                    def.location
                ),
            ));
        }
    }
    Ok(())
}

/// Every screen that renders and requires authentication needs a user. Anonymous access screens
/// and `require-authentication="false"` screens do not.
fn check_authentication(info: &ScreenUrlInfo) -> Result<(), Error> {
    if user::current_username()?.is_some() {
        return Ok(());
    }
    for def in info.render_defs() {
        if def.authentication == Authentication::Required {
            return Err(Error::of_kind(
                ErrorKind::AuthenticationRequired,
                "Authentication required",
            ));
        }
    }
    Ok(())
}

/// Artifact frames pushed for a request, popped in reverse when this drops. Anonymous-access
/// screens log the session in as anonymous until then.
#[derive(Default)]
struct Frames {
    pushed: usize,
    anonymous: bool,
}

impl Frames {
    fn push(&mut self, artifact_type: &str, name: &str, action: &str, requires_authz: bool) -> Result<(), Error> {
        ExecutionContext::with_current(|ec| ec.push_checked(artifact_type, name, action, requires_authz, false, None))
            .unwrap_or(Ok(()))?;
        self.pushed += 1;
        Ok(())
    }

    fn push_screen(&mut self, def: &ScreenDef, requires_authz: bool) -> Result<(), Error> {
        self.push("AT_XML_SCREEN", &def.location, "view", requires_authz)
    }

    /// Push a screen's frame, with authorization required only for the last screen of the path
    /// when it requires authentication, and apply `anonymous-view` and `anonymous-all`.
    fn enter_screen(&mut self, def: &ScreenDef, is_last: bool) -> Result<(), Error> {
        self.push_screen(def, is_last && def.authentication == Authentication::Required)?;
        self.apply_anonymous(def)
    }

    fn apply_anonymous(&mut self, def: &ScreenDef) -> Result<(), Error> {
        let all = match def.authentication {
            Authentication::AnonymousAll => true,
            Authentication::AnonymousView => false,
            _ => return Ok(()),
        };
        if user::login_anonymous()? {
            self.anonymous = true;
        }
        ExecutionContext::with_current(|ec| ec.set_anonymous_authorized(all, ""));
        Ok(())
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        for _ in 0..self.pushed {
            ExecutionContext::with_current(|ec| {
                let _ = ec.pop();
            });
        }
        if self.anonymous {
            user::logout_anonymous_only();
        }
    }
}

fn run_transition(factory: &Arc<ExecutionContextFactory>, job: &Job<'_>, info: ScreenUrlInfo) -> Result<(), Error> {
    let Some(transition) = info.transition(Some(job.method)).cloned() else {
        let name = info.target_transition_name.clone().unwrap_or_default();
        let allow: Vec<String> = info
            .target_screen
            .transitions
            .iter()
            .filter(|item| item.name == name)
            .map(|item| item.method.to_ascii_uppercase())
            .collect();
        web::send_json_error(405, &format!("Method {} is not supported", job.method));
        web::add_header("Allow", allow.join(", "));
        return Ok(());
    };
    if let Some(name) = missing_parameter(job.parameters, &transition) {
        web::send_json_error(400, &format!("Missing required parameter {name}"));
        return Ok(());
    }
    if job.enforce_request_checks && !job.header_authenticated && csrf_rejected(job, &transition)? {
        web::send_json_error(401, "Session token is missing or is not valid");
        return Ok(());
    }

    let info = Arc::new(info);
    let mode = DEFAULT_MODE.to_string();
    let facade = ScreenFacade::of(factory);
    let (content_type, encoding) = output_type(&facade, &info, &mode);
    let state = Rc::new(RenderState {
        url_info: Arc::clone(&info),
        render_mode: mode,
        content_type,
        character_encoding: encoding,
        always_standalone: false,
        screen_path_index: Cell::new(0),
        dont_do_render: Cell::new(false),
        stopped: RefCell::new(Vec::new()),
    });
    let _active = ActiveGuard::install(Rc::clone(&state));

    // Screens above the transition are only on the stack, so they inherit but are not decided.
    let mut frames = Frames::default();
    for (index, def) in info.path_defs.iter().enumerate() {
        frames.push_screen(def, false)?;
        frames.apply_anonymous(def)?;
        state.screen_path_index.set(index);
        if let Some(actions) = &def.always_actions {
            script::run_actions(&actions.xml)?;
        }
    }
    let screen = Arc::clone(&info.target_screen);
    frames.push(
        "AT_XML_SCREEN_TRANS",
        &format!("{}/{}", screen.location, transition.name),
        &transition.authz_action,
        screen.authentication == Authentication::Required,
    )?;

    set_all_parameters(&info, &transition)?;
    if let Some(condition) = &transition.condition {
        if !check_condition(condition)? {
            ExecutionContext::with_current(|ec| {
                ec.message().add_error(format!(
                    "Condition failed for transition {}/{}, not running actions or redirecting",
                    screen.location, transition.name
                ));
            });
            let response = transition
                .error_response
                .as_ref()
                .unwrap_or(&transition.default_response);
            return respond(&info, response);
        }
    }

    let wired = dispatch::run(&dispatch::Call {
        request: job.request,
        transition: &transition,
        extra: &info.extra_path,
        parameters: job.parameters,
        json_list: job.json_list,
        login_error: job.login_error,
        started: job.started,
    })?;
    if wired.is_some() {
        return Ok(());
    }
    if let Some(message) = &transition.actions_error {
        return Err(Error::new(message.clone()));
    }
    if let Some(actions) = &transition.actions {
        in_transaction(transition.begin_transaction, None, || run_transition_actions(actions))?;
    }
    let response = pick_response(&transition)?;
    respond(&info, response)
}

fn run_transition_actions(actions: &Actions) -> Result<(), Error> {
    script::run_actions(&actions.xml)
}

/// The error response when there are errors, else the first conditional response that holds,
/// else the default response.
fn pick_response(transition: &TransitionDef) -> Result<&ResponseDef, Error> {
    let has_error = ExecutionContext::with_current(|ec| !ec.message().errors().is_empty()).unwrap_or(false);
    if has_error {
        if let Some(response) = &transition.error_response {
            return Ok(response);
        }
    }
    for response in &transition.conditional_responses {
        let holds = match &response.condition {
            Some(condition) => check_condition(condition)?,
            None => true,
        };
        if holds {
            return Ok(response);
        }
    }
    Ok(&transition.default_response)
}

/// Bind the extra path names to the transition's `path-parameter` names, then evaluate the
/// screen and transition `parameter` entries into the context.
fn set_all_parameters(info: &ScreenUrlInfo, transition: &TransitionDef) -> Result<(), Error> {
    for (name, value) in transition.path_parameters.iter().zip(info.extra_path.iter()) {
        script::set_context_name(name, Value::string(value))?;
    }
    for parameter in info.target_screen.parameters.iter().chain(transition.parameters.iter()) {
        if let Some(value) = parameter_value(parameter)? {
            script::set_context_name(&parameter.name, value)?;
        }
    }
    Ok(())
}

fn parameter_value(parameter: &ParameterDef) -> Result<Option<Value>, Error> {
    if let Some(from) = &parameter.from {
        return Ok(Some(script::eval_value(from)?));
    }
    if let Some(value) = &parameter.value {
        return Ok(Some(Value::string(script::expand(value)?)));
    }
    Ok(script::context_value(&parameter.name).or_else(|| web::parameters().get(&parameter.name).cloned()))
}

fn respond(info: &ScreenUrlInfo, response: &ResponseDef) -> Result<(), Error> {
    if response.save_current_screen {
        user::session::push_screen_history(&info.full_path_url());
    }
    if response.kind == ResponseKind::None {
        if response.save_parameters {
            save_parameters();
        }
        return Ok(());
    }
    if response.save_parameters {
        save_parameters();
    }
    save_messages();
    let mut url = script::expand(&response.url)?;
    let mut url_type = response.url_type.clone();
    let mut carried = BTreeMap::new();
    if matches!(
        response.kind,
        ResponseKind::ScreenLast | ResponseKind::ScreenLastNoParam
    ) {
        url_type = "screen-path".to_string();
        // The newest history entry, else the root, so a login never ends up stuck on itself.
        url = user::screen_history()
            .ok()
            .and_then(|history| history.first().cloned())
            .filter(|path| path.starts_with('/'))
            .unwrap_or_else(|| "/".to_string());
        if response.kind == ResponseKind::ScreenLast {
            for (name, value) in user::saved_parameters().unwrap_or_default() {
                carried.insert(name, value);
            }
        }
    }
    let mut parameters = carried;
    for parameter in &response.parameters {
        if let Some(value) = parameter_value(parameter)? {
            parameters.insert(parameter.name.clone(), moqui_lib::object::to_plain_string(&value));
        }
    }
    if let Some(expression) = &response.parameter_map {
        for (name, value) in script::eval_map(expression)? {
            parameters.insert(name, moqui_lib::object::to_plain_string(&value));
        }
    }
    let location = if url_type == "plain" {
        url
    } else {
        // Keep the page and sort a list was on, so a form submit returns to the same view.
        let request_parameters = web::parameters();
        for name in ["pageIndex", "orderByField"] {
            if let Some(value) = request_parameters.get(name) {
                parameters
                    .entry(name.to_string())
                    .or_insert_with(|| moqui_lib::object::to_plain_string(value));
            }
        }
        let names = resolve_screen_path(&info.pre_transition_path, &url);
        let base = web::base_url();
        format!("{}{}", base.trim_end_matches('/'), encode_path(&names))
    };
    web::send_redirect(&with_query(location, &parameters));
    Ok(())
}

/// Messages ride the session to the next screen, which shows them.
fn save_messages() {
    let (messages, errors) = ExecutionContext::with_current(|ec| {
        let facade = ec.message();
        (facade.messages().to_vec(), facade.errors().to_vec())
    })
    .unwrap_or_default();
    for message in messages.iter().chain(errors.iter()) {
        user::session::add_flash_message(message);
    }
}

fn save_parameters() {
    for (name, value) in web::parameters() {
        user::session::save_parameter(&name, &moqui_lib::object::to_plain_string(&value));
    }
}

fn with_query(mut location: String, parameters: &BTreeMap<String, String>) -> String {
    let mut query = String::new();
    for (name, value) in parameters {
        if value.is_empty() {
            continue;
        }
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(&encode_component(name));
        query.push('=');
        query.push_str(&encode_component(value));
    }
    if !query.is_empty() {
        location.push(if location.contains('?') { '&' } else { '?' });
        location.push_str(&query);
    }
    // A header cannot carry a line break.
    location.retain(|ch| ch != '\r' && ch != '\n');
    location
}

fn encode_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Whether `condition` holds in the current context.
pub(crate) fn check_condition(condition: &Condition) -> Result<bool, Error> {
    match condition {
        Condition::Expression(expression) => script::eval_condition(expression),
        Condition::Wrapped(actions) => {
            let _frame = ContextFrame::push();
            script::run_actions(&actions.xml)?;
            Ok(matches!(
                script::context_value(CONDITION_FIELD),
                Some(Value::Bool(true))
            ))
        }
    }
}

/// Run `body`, in a transaction when `begin` is set. The transaction joins one already in place.
/// Errors the actions left in the message facade roll it back without failing the call.
fn in_transaction(begin: bool, timeout: Option<i64>, body: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
    let facade = if begin {
        ExecutionContext::with_current(|ec| ec.factory().entity()).flatten()
    } else {
        None
    };
    let Some(facade) = facade else {
        return body();
    };
    let tx = facade.transactions_for(crate::service::semaphore_entity_name())?;
    let timeout = timeout
        .and_then(|seconds| u64::try_from(seconds).ok())
        .map(std::time::Duration::from_secs);
    let mut rolled_back = false;
    let outcome = tx.run(crate::transaction::TransactionAttribute::UseOrBegin, timeout, || {
        let _deadline = script::DeadlineGuard::new(tx.time_remaining());
        body()?;
        let has_error = ExecutionContext::with_current(|ec| !ec.message().errors().is_empty()).unwrap_or(false);
        if has_error {
            rolled_back = true;
            tx.set_rollback_only("Errors running screen actions")?;
        }
        Ok(())
    });
    match outcome {
        Err(err) if rolled_back && err.to_string().contains("rollback-only") => Ok(()),
        other => other.map_err(Error::from),
    }
}

fn missing_parameter(parameters: &BTreeMap<String, Value>, transition: &TransitionDef) -> Option<String> {
    transition
        .required_parameters()
        .into_iter()
        .find(|name| is_empty(parameters.get(*name)))
        .map(str::to_string)
}

fn is_empty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.is_empty(),
        Some(_) => false,
    }
}

fn csrf_rejected(job: &Job<'_>, transition: &TransitionDef) -> Result<bool, Error> {
    if !matches!(
        job.method.to_ascii_lowercase().as_str(),
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
    let presented = text_of(job.parameters.get("moquiSessionToken"));
    let presented = if presented.is_empty() {
        job.request.header("x-csrf-token").unwrap_or("").to_string()
    } else {
        presented
    };
    Ok(!constant_time_eq(presented.as_bytes(), expected.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0_u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn text_of(value: Option<&Value>) -> String {
    value.map(moqui_lib::object::to_plain_string).unwrap_or_default()
}
