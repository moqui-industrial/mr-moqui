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

//! Owned HTTP request and response. The host copies bytes in and writes bytes out.
//!
//! The slot is thread-local and cleared when its guard drops, including on panic.
//! A request-pool worker reuses its thread, so a leaked slot would leak into the next request.

use std::cell::RefCell;
use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use indexmap::IndexMap;
use moqui_lib::object::Value;

pub mod error_info;
pub mod multipart;

use crate::context::{Error, ErrorKind, ExecutionContext};
use crate::entity::query_rest;

const BODY_LIMIT: usize = 1024 * 1024;
const PREFLIGHT_HEADERS: &str = "Accept,Accept-Encoding,Content-Type,Content-Encoding,Origin,X-Requested-With,Access-Control-Request-Method,Access-Control-Request-Headers,api_key,login_key,X-HTTP-Method-Override,moquiSessionToken,SessionToken,X-CSRF-Token,Authorization,A2A-Version";
const EXPOSE_HEADERS: &str =
    "Access-Control-Allow-Origin,Access-Control-Allow-Credentials,X-CSRF-Token,moquiSessionToken";

thread_local! {
    static WEB: RefCell<Option<WebSlot>> = const { RefCell::new(None) };
}

/// One HTTP request, already read off the socket.
#[derive(Clone, Debug)]
pub struct WebRequest {
    pub method: String,
    pub path: String,
    pub https: bool,
    pub headers: BTreeMap<String, String>,
    /// Decoded query pairs in request order. A repeated name becomes a list parameter.
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub peer_ip: String,
    pub session_cookie: Option<String>,
    pub visitor_cookie: Option<String>,
}

impl WebRequest {
    /// `GET` with an empty body. Headers are stored under lowercase names.
    pub fn new(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            https: false,
            headers: BTreeMap::new(),
            query: Vec::new(),
            body: Vec::new(),
            peer_ip: String::new(),
            session_cookie: None,
            visitor_cookie: None,
        }
    }

    /// Header value. The lookup lowercases `name`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(String::as_str)
    }

    pub fn set_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.headers.insert(name.into().to_ascii_lowercase(), value.into());
    }
}

/// One HTTP response. Repeated names are separate headers, so `Set-Cookie` can appear twice.
#[derive(Clone, Debug)]
pub struct WebResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl WebResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

struct WebSlot {
    parameters: BTreeMap<String, Value>,
    /// Uploaded files by field name, and `_fileUploadList`.
    multipart: BTreeMap<String, Value>,
    /// What the last response saved for this page, taken from the session when first asked for.
    saved: Option<crate::user::session::SavedForPage>,
    /// The `Host` header of the request.
    host: String,
    /// Scheme, host, and context path that links start with. Empty for a path-only link.
    base_url: String,
    sent: bool,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Puts back the thread-local web slot that was there before the install, or clears it when there was
/// none, including when a panic unwinds. A render started from inside another render keeps the outer
/// one's slot this way.
pub struct WebGuard {
    previous: Option<WebSlot>,
}

impl Drop for WebGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        WEB.with(|slot| *slot.borrow_mut() = previous);
    }
}

/// Install the parameter map for this request. A second install replaces the first.
pub(crate) fn install(parameters: BTreeMap<String, Value>) -> WebGuard {
    install_with(parameters, String::new())
}

/// [`install`] with the base URL a screen render builds links from. A `makeRender` with no socket
/// fills the slot this way: the parameter map and base URL here, the session map on the session,
/// and the output writer is the slot's body (see [`start_output`] and [`write_output`]).
pub(crate) fn install_with(parameters: BTreeMap<String, Value>, base_url: String) -> WebGuard {
    let previous = WEB.with(|slot| {
        slot.borrow_mut().replace(WebSlot {
            parameters,
            multipart: BTreeMap::new(),
            saved: None,
            host: String::new(),
            base_url,
            sent: false,
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        })
    });
    WebGuard { previous }
}

pub(crate) fn parameters() -> BTreeMap<String, Value> {
    WEB.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|web| web.parameters.clone())
            .unwrap_or_default()
    })
}

/// Keep the uploaded files of this request for `ec.web.multiPartParameters`.
pub(crate) fn set_multipart(files: BTreeMap<String, Value>) {
    WEB.with(|slot| {
        if let Some(web) = slot.borrow_mut().as_mut() {
            web.multipart = files;
        }
    });
}

/// The messages, errors and failed-submit parameters the last response saved, which this page shows. The
/// session gives them up the first time they are asked for in a request.
pub(crate) fn saved_for_page() -> crate::user::session::SavedForPage {
    let loaded = WEB.with(|slot| slot.borrow().as_ref().and_then(|web| web.saved.clone()));
    if let Some(saved) = loaded {
        return saved;
    }
    let taken = crate::user::session::take_saved_for_page();
    WEB.with(|slot| {
        if let Some(web) = slot.borrow_mut().as_mut() {
            web.saved = Some(taken.clone());
        }
    });
    taken
}

/// Lower-case hex of the time this process first asked, so every link made in one run shares it.
pub(crate) fn resource_distinct_value() -> String {
    static START: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    START
        .get_or_init(|| {
            let millis = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_millis());
            format!("{millis:x}")
        })
        .clone()
}

pub(crate) fn set_host(host: &str) {
    WEB.with(|slot| {
        if let Some(web) = slot.borrow_mut().as_mut() {
            web.host = host.to_string();
        }
    });
}

/// The host name the request was for, with the port when asked.
pub(crate) fn host_name(with_port: bool) -> String {
    let host = WEB.with(|slot| slot.borrow().as_ref().map(|web| web.host.clone()).unwrap_or_default());
    if with_port {
        return host;
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|ch| ch.is_ascii_digit()) => name.to_string(),
        _ => host,
    }
}

pub(crate) fn multipart_parameters() -> BTreeMap<String, Value> {
    WEB.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|web| web.multipart.clone())
            .unwrap_or_default()
    })
}

pub(crate) fn base_url() -> String {
    WEB.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|web| web.base_url.clone())
            .unwrap_or_default()
    })
}

/// Open the response body for a screen render: status, `Content-Type`, and an empty body that
/// [`write_output`] appends to. A response something already sent is left alone and `false` returns.
pub(crate) fn start_output(status: u16, content_type: &str) -> bool {
    WEB.with(|slot| {
        let mut web = slot.borrow_mut();
        let Some(web) = web.as_mut() else {
            return false;
        };
        if web.sent {
            return false;
        }
        web.sent = true;
        web.status = status;
        web.headers.push(("Content-Type".to_string(), content_type.to_string()));
        true
    })
}

/// Append text to the open body. Nothing happens before [`start_output`].
// The macro walker writes through this from slice 2.
#[allow(dead_code)]
pub(crate) fn write_output(text: &str) {
    WEB.with(|slot| {
        if let Some(web) = slot.borrow_mut().as_mut() {
            if web.sent {
                web.body.extend_from_slice(text.as_bytes());
            }
        }
    });
}

pub(crate) fn add_header(name: impl Into<String>, value: impl Into<String>) {
    WEB.with(|slot| {
        if let Some(web) = slot.borrow_mut().as_mut() {
            web.headers.push((name.into(), value.into()));
        }
    });
}

/// `application/json`. A map gains `errors` and status 500 when the message facade has one.
pub(crate) fn send_json_response(value: Value) {
    let mut status = 200;
    let mut value = value;
    if let Value::Map(map) = &mut value {
        let errors = message_errors();
        if !errors.is_empty() {
            status = 500;
            map.insert("errors".to_string(), Value::string(errors.join("\n")));
        }
    }
    send_bytes(status, "application/json", query_rest::encode_json(&value).into_bytes());
}

/// `{"errorCode","errors"}` as JSON.
pub(crate) fn send_json_error(status: u16, message: &str) {
    let mut map = IndexMap::new();
    map.insert("errorCode".to_string(), Value::I64(i64::from(status)));
    map.insert("errors".to_string(), Value::string(message));
    send_bytes(
        status,
        "application/json",
        query_rest::encode_json(&Value::Map(map)).into_bytes(),
    );
}

/// `text/plain` with status 200.
pub(crate) fn send_text_response(text: &str) {
    send_bytes(200, "text/plain", text.as_bytes().to_vec());
}

/// The signer for QZ Tray, from `qz-private-key.pem`. Java reads it once from the classpath at class load. This
/// looks in `classpath://` (the framework directory), then `classes/` and `conf/` under the runtime directory,
/// and keeps the first signer that loaded, so a key added later is found without a restart.
fn qz_signer() -> std::sync::Arc<moqui_lib::signer::SimpleSigner> {
    use moqui_lib::signer::SimpleSigner;
    use std::sync::{Arc, Mutex};
    static SIGNER: Mutex<Option<Arc<SimpleSigner>>> = Mutex::new(None);
    if let Some(signer) = SIGNER.lock().ok().and_then(|slot| slot.clone()) {
        return signer;
    }
    let mut candidates = Vec::new();
    if let Ok(path) = crate::context::locate("classpath://qz-private-key.pem") {
        candidates.push(path);
    }
    if let Some(runtime) = ExecutionContext::with_current(|ec| ec.factory().runtime_dir().to_path_buf()) {
        candidates.push(runtime.join("classes/qz-private-key.pem"));
        candidates.push(runtime.join("conf/qz-private-key.pem"));
    }
    let mut found = Arc::new(SimpleSigner::from_pem("qz-private-key.pem", ""));
    for path in candidates.iter().filter(|path| path.is_file()) {
        let signer = Arc::new(SimpleSigner::from_file(path));
        let loaded = signer.is_loaded();
        found = signer;
        if loaded {
            if let Ok(mut slot) = SIGNER.lock() {
                *slot = Some(Arc::clone(&found));
            }
            break;
        }
    }
    found
}

/// `ec.web.sendQzSignedResponse(message)`: the Base64 `SHA1withRSA` signature of `message` as `text/plain`.
/// A request with no user gets 401. A key that cannot be read gets 500 with the reason, as Java does.
pub(crate) fn send_qz_signed_response(message: &str) {
    match crate::user::current_user_id() {
        Ok(Some(_)) => {}
        _ => {
            send_bytes(401, "text/plain", b"Authentication required".to_vec());
            return;
        }
    }
    match qz_signer().sign(message) {
        Ok(signature) => send_bytes(200, "text/plain", signature.into_bytes()),
        Err(err) => {
            tracing::error!("Error signing QZ message, sending error response: {}", err.message());
            send_bytes(500, "text/plain", err.message().as_bytes().to_vec());
        }
    }
}

/// 302 with a `Location` header. A response that was already sent is left as it is.
pub(crate) fn send_redirect(location: &str) {
    WEB.with(|slot| {
        let mut web = slot.borrow_mut();
        let Some(web) = web.as_mut() else {
            return;
        };
        if web.sent {
            return;
        }
        web.sent = true;
        web.status = 302;
        web.headers.push(("Location".to_string(), location.to_string()));
    });
}

/// 205 with `X-Redirect-To`, the redirect a client router follows for a standalone render.
pub(crate) fn send_reset_content(location: &str) {
    WEB.with(|slot| {
        let mut web = slot.borrow_mut();
        let Some(web) = web.as_mut() else {
            return;
        };
        if web.sent {
            return;
        }
        web.sent = true;
        web.status = 205;
        web.headers.push(("X-Redirect-To".to_string(), location.to_string()));
    });
}

/// A response with its own headers and bytes, such as a static file. One already sent is left alone.
pub(crate) fn send_bytes_with_headers(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) {
    WEB.with(|slot| {
        let mut web = slot.borrow_mut();
        let Some(web) = web.as_mut() else {
            return;
        };
        if web.sent {
            return;
        }
        web.sent = true;
        web.status = status;
        web.headers.extend(headers);
        web.body = body;
    });
}

fn send_bytes(status: u16, content_type: &str, body: Vec<u8>) {
    WEB.with(|slot| {
        let mut web = slot.borrow_mut();
        let Some(web) = web.as_mut() else {
            return;
        };
        if web.sent {
            return;
        }
        web.sent = true;
        web.status = status;
        web.headers.push(("Content-Type".to_string(), content_type.to_string()));
        web.body = body;
    });
}

/// The status the response has so far.
pub(crate) fn output_status() -> u16 {
    WEB.with(|slot| slot.borrow().as_ref().map_or(200, |web| web.status))
}

/// Whether a response was already written for this request.
pub(crate) fn output_sent() -> bool {
    WEB.with(|slot| slot.borrow().as_ref().is_some_and(|web| web.sent))
}

/// Forget what was written, so a response can be sent after a body was taken for another use.
pub(crate) fn clear_output() {
    WEB.with(|slot| {
        if let Some(web) = slot.borrow_mut().as_mut() {
            web.sent = false;
            web.status = 200;
            web.headers.clear();
            web.body.clear();
        }
    });
}

/// Move the status, headers, body, and sent flag out of the slot.
pub(crate) fn take_output() -> (u16, Vec<(String, String)>, Vec<u8>, bool) {
    WEB.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|web| {
                (
                    web.status,
                    std::mem::take(&mut web.headers),
                    std::mem::take(&mut web.body),
                    web.sent,
                )
            })
            .unwrap_or((200, Vec::new(), Vec::new(), false))
    })
}

fn message_errors() -> Vec<String> {
    ExecutionContext::with_current(|ec| ec.message().errors().to_vec()).unwrap_or_default()
}

/// Parsed inputs for one request. A JSON list stays beside the parameter map.
pub(crate) struct ParsedBody {
    pub parameters: BTreeMap<String, Value>,
    pub json_list: Option<Vec<Value>>,
    /// Uploaded files by field name: one [`multipart::UploadedFile`] or a list of them, and `_fileUploadList`.
    pub multipart: BTreeMap<String, Value>,
}

/// Body larger than its limit, a multipart body that cannot be read or holds a program, or JSON that is not
/// an object or a list.
pub(crate) enum BodyError {
    TooLarge,
    Multipart(String),
    /// A file part that starts like a program, when the web app does not allow one.
    Executable(String),
    Json(String),
}

/// The largest request body: 1 MiB, and for a `multipart/form-data` body `webapp_max_upload`.
pub fn body_limit(request: &WebRequest) -> usize {
    if is_multipart(request) {
        max_upload()
    } else {
        BODY_LIMIT
    }
}

pub(crate) fn is_multipart(request: &WebRequest) -> bool {
    request
        .header("content-type")
        .unwrap_or("")
        .trim_start()
        .get(..10)
        .is_some_and(|start| start.eq_ignore_ascii_case("multipart/"))
}

const DEFAULT_MAX_UPLOAD: usize = 100 * 1024 * 1024;

/// `webapp_max_upload`: bytes, or a number with a `k`, `m` or `g` suffix. 100 MiB when unset or not a size.
pub fn max_upload() -> usize {
    parse_size(&moqui_lib::env::get_prop_or_env("webapp_max_upload")).unwrap_or(DEFAULT_MAX_UPLOAD)
}

fn parse_size(text: &str) -> Option<usize> {
    let text = text.trim().to_ascii_lowercase();
    let (digits, unit) = match text.chars().last()? {
        'k' => (&text[..text.len() - 1], 1024),
        'm' => (&text[..text.len() - 1], 1024 * 1024),
        'g' => (&text[..text.len() - 1], 1024 * 1024 * 1024),
        _ => (text.as_str(), 1),
    };
    digits
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_mul(unit))
        .filter(|size| *size > 0)
}

fn upload_executable_allowed() -> bool {
    moqui_lib::env::get_prop_or_env("webapp_upload_executable_allow") == "true"
}

/// Query, then form, then a JSON object. A JSON list is not merged into the map.
pub(crate) fn parse_body(request: &WebRequest) -> Result<ParsedBody, BodyError> {
    if request.body.len() > body_limit(request) {
        return Err(BodyError::TooLarge);
    }
    let content_type = request.header("content-type").unwrap_or("").to_ascii_lowercase();
    let media = content_type.split(';').next().unwrap_or("").trim();
    let mut pairs = request.query.clone();
    let mut json_list = None;
    let mut multipart_files = BTreeMap::new();
    if media.starts_with("multipart/") {
        let parsed = multipart::parse(request.header("content-type").unwrap_or(""), &request.body)
            .map_err(|err| BodyError::Multipart(err.to_string()))?;
        pairs.extend(parsed.fields);
        if !upload_executable_allowed() {
            if let Some(file) = parsed.files.iter().find(|file| file.is_executable()) {
                return Err(BodyError::Executable(file.file_name.clone()));
            }
        }
        let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut all = Vec::new();
        for file in parsed.files {
            let name = file.field_name.clone();
            let value = file.into_value();
            all.push(value.clone());
            grouped.entry(name).or_default().push(value);
        }
        for (name, mut values) in grouped {
            multipart_files.insert(
                name,
                if values.len() == 1 {
                    values.remove(0)
                } else {
                    Value::List(values)
                },
            );
        }
        multipart_files.insert("_fileUploadList".to_string(), Value::List(all));
        return Ok(ParsedBody {
            parameters: pairs_to_parameters(pairs),
            json_list,
            multipart: multipart_files,
        });
    }
    if request.body.is_empty() {
        return Ok(ParsedBody {
            parameters: pairs_to_parameters(pairs),
            json_list,
            multipart: multipart_files,
        });
    }
    if media == "application/x-www-form-urlencoded" || (media.is_empty() && request.body.contains(&b'=')) {
        pairs.extend(form_pairs(&String::from_utf8_lossy(&request.body)));
        return Ok(ParsedBody {
            parameters: pairs_to_parameters(pairs),
            json_list,
            multipart: multipart_files,
        });
    }
    let mut parameters = pairs_to_parameters(pairs);
    if media == "application/json" || media.ends_with("+json") {
        let text =
            std::str::from_utf8(&request.body).map_err(|_| BodyError::Json("JSON body is not utf-8".to_string()))?;
        match crate::service::parse_json(text) {
            Ok(Value::Map(map)) => {
                for (key, value) in map {
                    parameters.insert(key, value);
                }
            }
            Ok(Value::List(items)) => {
                if items.iter().any(|item| !matches!(item, Value::Map(_))) {
                    return Err(BodyError::Json("JSON list elements must be objects".to_string()));
                }
                json_list = Some(items);
            }
            Ok(_) => return Err(BodyError::Json("JSON body must be an object or a list".to_string())),
            Err(err) => return Err(BodyError::Json(err.message().to_string())),
        }
    }
    Ok(ParsedBody {
        parameters,
        json_list,
        multipart: multipart_files,
    })
}

/// One value stays a string. A name that repeats becomes a list in request order.
fn pairs_to_parameters(pairs: Vec<(String, String)>) -> BTreeMap<String, Value> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, value) in pairs {
        grouped.entry(key).or_default().push(value);
    }
    grouped
        .into_iter()
        .map(|(key, mut values)| {
            let value = if values.len() == 1 {
                Value::string(values.remove(0))
            } else {
                Value::List(values.into_iter().map(Value::string).collect())
            };
            (key, value)
        })
        .collect()
}

/// Split `a=1&b=2` into decoded pairs.
pub fn form_pairs(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for part in text.split('&') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        pairs.push((percent_decode(key), percent_decode(value)));
    }
    pairs
}

/// Decode `+` and `%XX`. A malformed escape is kept as written.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() && hex_pair(bytes[index + 1], bytes[index + 2]).is_some() => {
                out.push(hex_pair(bytes[index + 1], bytes[index + 2]).unwrap_or(b'%'));
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    let high = char::from(high).to_digit(16)?;
    let low = char::from(low).to_digit(16)?;
    u8::try_from(high * 16 + low).ok()
}

/// `Authorization: Basic` user and password. The split is the first colon.
pub(crate) fn basic_credentials(header: &str) -> Option<(String, String)> {
    let rest = header.trim();
    let rest = rest
        .strip_prefix("Basic ")
        .or_else(|| rest.strip_prefix("basic "))
        .or_else(|| rest.strip_prefix("BASIC "))?;
    let bytes = STANDARD.decode(rest.trim()).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let (user, password) = text.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

/// Cookie value for `name` from a `Cookie` header.
pub fn cookie_value(header: &str, name: &str) -> Option<String> {
    for part in header.split(';') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        if key.trim() == name {
            return Some(value.trim().to_string());
        }
    }
    None
}

pub(crate) fn session_cookie_of(request: &WebRequest) -> Option<String> {
    if let Some(value) = request.session_cookie.as_ref().filter(|text| !text.is_empty()) {
        return Some(value.clone());
    }
    request
        .header("cookie")
        .and_then(|header| cookie_value(header, crate::user::SESSION_COOKIE_NAME))
}

pub(crate) fn visitor_cookie_of(request: &WebRequest) -> Option<String> {
    if let Some(value) = request.visitor_cookie.as_ref().filter(|text| !text.is_empty()) {
        return Some(value.clone());
    }
    request
        .header("cookie")
        .and_then(|header| cookie_value(header, crate::user::VISITOR_COOKIE_NAME))
}

/// First hop of `webapp_client_ip_header` when that property is set and the header is present.
pub fn client_ip(request: &WebRequest) -> String {
    let name = moqui_lib::env::get_prop_or_env("webapp_client_ip_header");
    if !name.is_empty() {
        if let Some(value) = request.header(&name) {
            let first = value.split(',').next().unwrap_or("").trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    request.peer_ip.clone()
}

pub(crate) fn require_session_token() -> bool {
    !matches!(
        moqui_lib::env::get_prop_or_env("webapp_require_session_token").as_str(),
        "false"
    )
}

fn handle_cors() -> bool {
    !matches!(moqui_lib::env::get_prop_or_env("webapp_handle_cors").as_str(), "false")
}

fn allow_origins() -> Vec<String> {
    moqui_lib::env::get_prop_or_env("webapp_allow_origins")
        .split(',')
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .collect()
}

fn origin_allowed(origin: &str) -> bool {
    allow_origins().iter().any(|allowed| allowed == origin)
}

/// Screen-render headers, and CORS when the origin is allowed.
pub fn finish_response(request: &WebRequest, mut response: WebResponse, preflight: bool) -> WebResponse {
    push_unique(
        &mut response.headers,
        "Cache-Control",
        "no-cache, no-store, must-revalidate, private",
    );
    push_unique(
        &mut response.headers,
        "Content-Security-Policy",
        "frame-ancestors 'none'; form-action 'self';",
    );
    push_unique(&mut response.headers, "X-Frame-Options", "sameorigin");
    push_unique(&mut response.headers, "X-Content-Type-Options", "nosniff");
    if request.https {
        push_unique(&mut response.headers, "Strict-Transport-Security", "max-age=31536000");
    }
    if handle_cors() {
        if let Some(origin) = request.header("origin").filter(|text| !text.is_empty()) {
            if origin_allowed(origin) {
                apply_cors(&mut response.headers, origin, preflight);
            }
        }
    }
    response
}

fn apply_cors(headers: &mut Vec<(String, String)>, origin: &str, preflight: bool) {
    headers.push(("Vary".to_string(), "Origin".to_string()));
    headers.push(("Access-Control-Allow-Credentials".to_string(), "true".to_string()));
    headers.push(("Access-Control-Allow-Origin".to_string(), origin.to_string()));
    if preflight {
        headers.push((
            "Access-Control-Allow-Headers".to_string(),
            PREFLIGHT_HEADERS.to_string(),
        ));
        headers.push((
            "Access-Control-Allow-Methods".to_string(),
            "GET,PUT,POST,PATCH,DELETE,OPTIONS".to_string(),
        ));
        headers.push(("Access-Control-Max-Age".to_string(), "3600".to_string()));
    } else {
        headers.push(("Access-Control-Expose-Headers".to_string(), EXPOSE_HEADERS.to_string()));
    }
}

fn push_unique(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if headers.iter().any(|(key, _)| key.eq_ignore_ascii_case(name)) {
        return;
    }
    headers.push((name.to_string(), value.to_string()));
}

/// Names a response never carries: `WebUtilities.stripCredentialParameters`, compared without case.
const CREDENTIAL_PARAMETER_NAMES: [&str; 13] = [
    "password",
    "oldpassword",
    "newpassword",
    "newpasswordverify",
    "currentpassword",
    "resetpassword",
    "authpassword",
    "api_key",
    "login_key",
    "passwordsalt",
    "passwordhashtype",
    "passwordbase64",
    "passwordverify",
];

/// Remove credential parameters from a JSON value, at any depth. Java strips the top level of a parameter map; a
/// `UserAccount` value in a screen's data holds the same names as columns, so the nested maps lose them too.
pub(crate) fn strip_credential_parameters(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|key, _| !CREDENTIAL_PARAMETER_NAMES.contains(&key.to_ascii_lowercase().as_str()));
            map.values_mut().for_each(strip_credential_parameters);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_credential_parameters),
        _ => {}
    }
}

pub(crate) fn json_error(status: u16, message: &str) -> WebResponse {
    let mut map = IndexMap::new();
    map.insert("errorCode".to_string(), Value::I64(i64::from(status)));
    map.insert("errors".to_string(), Value::string(message));
    let mut response = WebResponse::new(status, query_rest::encode_json(&Value::Map(map)).into_bytes());
    response
        .headers
        .push(("Content-Type".to_string(), "application/json".to_string()));
    response
}

pub(crate) fn early_error(request: &WebRequest, status: u16, message: &str) -> WebResponse {
    finish_response(request, json_error(status, message), false)
}

/// Map an entity or service error onto 401, 403, 404, 405, 429, or 500.
///
/// [`ErrorKind`] wins. A general error still matches the entity messages that have no kind.
pub(crate) fn status_for(err: &Error) -> u16 {
    match err.kind() {
        ErrorKind::AuthenticationRequired => return 401,
        ErrorKind::Unauthorized => return 403,
        ErrorKind::Tarpit => return 429,
        ErrorKind::NotFound => return 404,
        ErrorKind::MethodNotAllowed => return 405,
        ErrorKind::Unavailable => return 503,
        ErrorKind::General => {}
    }
    let message = err.message();
    let lower = message.to_ascii_lowercase();
    if lower.contains("user must be logged in") || lower.contains("authentication required") {
        return 401;
    }
    if lower.contains("not authorized") || lower.contains("is not authorized") || lower.contains("tarpit") {
        return 403;
    }
    if message.contains("No value found") || lower.contains("no entity named") || lower.contains("no entity name") {
        return 404;
    }
    if lower.contains("not supported") {
        return 405;
    }
    500
}

pub(crate) fn record_login_error(message: &str) {
    ExecutionContext::with_current(|ec| ec.message().add_error(message.to_string()));
}

#[cfg(test)]
mod tests {
    use super::{cookie_value, form_pairs, pairs_to_parameters, percent_decode};
    use moqui_lib::object::Value;

    #[test]
    fn a_repeated_name_becomes_a_list() {
        let map = pairs_to_parameters(form_pairs("a=1&b=x&a=2&a=3"));
        assert_eq!(map.get("b"), Some(&Value::string("x")));
        assert_eq!(
            map.get("a"),
            Some(&Value::List(vec![
                Value::string("1"),
                Value::string("2"),
                Value::string("3")
            ]))
        );
    }

    #[test]
    fn percent_decode_keeps_bad_escapes_and_never_panics() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("%a\u{e9}"), "%a\u{e9}");
        assert_eq!(percent_decode("tail%"), "tail%");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
    }

    #[test]
    fn cookie_value_skips_parts_without_an_equals_sign() {
        assert_eq!(
            cookie_value("junk; moqui.session=abc; x=y", "moqui.session").as_deref(),
            Some("abc")
        );
        assert_eq!(cookie_value("a=b", "moqui.session"), None);
    }
}
