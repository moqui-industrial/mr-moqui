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

//! Outbound HTTP for `remote-rest` services and `http` resource locations.
//!
//! A host may install an [`HttpSender`]; it is used first. Without one, the `http` feature sends
//! with the blocking reqwest [`moqui_lib::http::RestClient`] (the server). Without either, a call
//! fails with "not built into this host". The browser installs a sender over `fetch`
//! (`plans/archive/Wasm.md` W5). Sim denial and URL redaction are the caller's, before [`send`].

use std::fmt;
use std::sync::Arc;

use moqui_lib::error::Error;
use moqui_lib::sync::InstallSlot;

/// The methods `remote-rest` sends, those of Java's `RestClient.Method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

impl Method {
    /// The method for a name such as `put`, ignoring case. `None` when it is not one of these.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "PATCH" => Some(Self::Patch),
            "DELETE" => Some(Self::Delete),
            "HEAD" => Some(Self::Head),
            "OPTIONS" => Some(Self::Options),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One outbound request. `url` is http or https; [`send`] checks the scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    /// The whole call's limit; `None` is the client's default.
    pub timeout: Option<std::time::Duration>,
    /// Refuse a host that is or resolves to an internal address (loopback, private, link-local), redirects included:
    /// for a URL that came from outside, such as a mail's unsubscribe link.
    pub deny_internal: bool,
}

/// Status, body text, and what a binary body and the headers need. A sender that has no bytes or headers leaves them
/// empty, and the body text stands for the bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
    /// The body as received. Empty when the sender did not keep it.
    pub bytes: Vec<u8>,
    /// The response headers, names as sent.
    pub headers: Vec<(String, String)>,
}

/// Sends a request on the host's transport. A transport failure is an error; an HTTP error status is
/// a [`Response`].
pub trait HttpSender: Send + Sync {
    fn send(&self, request: Request) -> Result<Response, Error>;
}

static SENDER: InstallSlot<dyn HttpSender> = InstallSlot::new();

/// Use `sender` for every outbound call in this process.
pub fn install_http_sender(sender: Arc<dyn HttpSender>) {
    SENDER.install(sender);
}

/// Remove the installed sender. The `http` feature's client is used again, if built.
pub fn clear_http_sender() {
    SENDER.clear();
}

/// Send `request` on the installed sender, else on reqwest when the `http` feature is built.
///
/// # Errors
///
/// Returns an error when the URL is not http or https, no transport is built into this host, or
/// the transport fails.
pub fn send(mut request: Request) -> Result<Response, Error> {
    let mut span = http_span(request.method, &request.url);
    let context = span
        .as_ref()
        .and_then(crate::telemetry::Span::context)
        .or_else(crate::telemetry::capture);
    inject_context(&mut request.headers, context.as_ref());
    let _trace = crate::telemetry::attach(context);
    let result = send_inner(request);
    if let Some(mut span) = span.take() {
        match &result {
            Ok(response) => {
                span.set_attribute(
                    "http.response.status_code",
                    moqui_lib::object::Value::I64(i64::from(response.status)),
                );
                if response.status >= 500 {
                    span.set_error("outbound HTTP server error");
                }
            }
            Err(error) => span.set_error(error.message()),
        }
        span.finish();
    }
    result
}

fn http_span(method: Method, url: &str) -> Option<crate::telemetry::Span> {
    crate::telemetry::current().map(|telemetry| {
        let mut span = telemetry.start_span(&format!("HTTP {method}"), crate::telemetry::SpanKind::Client, None);
        span.set_attribute("http.request.method", moqui_lib::object::Value::string(method.as_str()));
        span.set_attribute("url.full", moqui_lib::object::Value::String(redacted(url)));
        span
    })
}

fn inject_context(headers: &mut Vec<(String, String)>, context: Option<&crate::telemetry::TraceContext>) {
    if let Some(context) = context {
        headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("traceparent") && !name.eq_ignore_ascii_case("tracestate"));
        headers.push(("traceparent".into(), context.traceparent()));
        if !context.tracestate.is_empty() {
            headers.push(("tracestate".into(), context.tracestate.clone()));
        }
    }
}

fn send_inner(request: Request) -> Result<Response, Error> {
    // Under the read fence nothing goes out this way: remote REST, script and resource HTTP, system sync and messages.
    // A model provider's call goes through `open`, which the fence leaves alone.
    crate::entity::fence::check_effect(
        crate::entity::fence::Effect::RemoteCall,
        &format!("HTTP {} {}", request.method, redacted(&request.url)),
    )?;
    let scheme = request
        .url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    if !matches!(scheme.as_deref(), Some("http" | "https")) {
        return Err(Error::new(format!(
            "URL {} must be http or https",
            redacted(&request.url)
        )));
    }
    if let Some(sender) = SENDER.get() {
        return sender.send(request);
    }
    send_default(request)
}

/// [`send`] on the built-in transport, past any installed sender: for a test sender that answers some addresses and
/// passes the others on.
#[cfg(feature = "http")]
pub(crate) fn send_default(request: Request) -> Result<Response, Error> {
    let method = match request.method {
        Method::Get => moqui_lib::http::Method::Get,
        Method::Post => moqui_lib::http::Method::Post,
        Method::Put => moqui_lib::http::Method::Put,
        Method::Patch => moqui_lib::http::Method::Patch,
        Method::Delete => moqui_lib::http::Method::Delete,
        Method::Head => moqui_lib::http::Method::Head,
        Method::Options => moqui_lib::http::Method::Options,
    };
    /// The shared client's behavior, but refusing internal addresses.
    fn denying_internal() -> &'static moqui_lib::http::RestClient {
        static CLIENT: std::sync::OnceLock<moqui_lib::http::RestClient> = std::sync::OnceLock::new();
        CLIENT.get_or_init(|| moqui_lib::http::RestClient::new().deny_internal_addresses(true))
    }
    let client = if request.deny_internal {
        denying_internal()
    } else {
        crate::service::shared_http_client()
    };
    let mut call = client.request(method, request.url);
    let content_type = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone());
    for (name, value) in request.headers {
        call = call.header(name, value);
    }
    if let Some(body) = request.body {
        call = call.text(content_type.as_deref().unwrap_or("text/plain"), body);
    }
    if let Some(timeout) = request.timeout {
        call = call.timeout(timeout);
    }
    // The caller already applied the sim predicate.
    let response = call.send(false)?;
    Ok(Response {
        status: response.status(),
        body: response.body().to_string(),
        bytes: response.bytes().to_vec(),
        headers: response.response_headers().to_vec(),
    })
}

#[cfg(not(feature = "http"))]
pub(crate) fn send_default(request: Request) -> Result<Response, Error> {
    Err(Error::new(format!(
        "HTTP {} {} is not built into this host",
        request.method,
        redacted(&request.url)
    )))
}

/// A request whose response body is read as it is passed on ([`open`]), such as a proxy's: the body is bytes, sent as
/// they are with the headers given.
#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<bytes::Bytes>,
    /// The whole exchange's limit, the body's read included; `None` is the client's default.
    pub timeout: Option<std::time::Duration>,
    /// Whether a redirect is followed (to the same host); a proxy passes the 3xx on instead.
    pub follow_redirects: bool,
}

/// The status and headers of an [`open`] response, and its body still to be read.
pub struct OpenResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// The body's length when the server said it.
    pub length: Option<u64>,
    pub reader: Box<dyn std::io::Read + Send>,
}

/// Send `request` and return once the response headers are in, its body a reader. An installed [`HttpSender`] has no
/// streaming: its whole response is the reader, and a body that is not UTF-8 text goes to it lossily.
///
/// # Errors
///
/// Returns an error when the URL is not http or https, no transport is built into this host, or the transport fails
/// before the response headers.
pub fn open(mut request: OpenRequest) -> Result<OpenResponse, Error> {
    let mut span = http_span(request.method, &request.url);
    let context = span
        .as_ref()
        .and_then(crate::telemetry::Span::context)
        .or_else(crate::telemetry::capture);
    inject_context(&mut request.headers, context.as_ref());
    let _trace = crate::telemetry::attach(context);
    match open_inner(request) {
        Ok(mut response) => {
            if let Some(mut span) = span.take() {
                span.set_attribute(
                    "http.response.status_code",
                    moqui_lib::object::Value::I64(i64::from(response.status)),
                );
                if response.status >= 500 {
                    span.set_error("outbound HTTP server error");
                }
                response.reader = Box::new(TracedReader {
                    reader: response.reader,
                    span: Some(span),
                });
            }
            Ok(response)
        }
        Err(error) => {
            if let Some(mut span) = span.take() {
                span.set_error(error.message());
                span.finish();
            }
            Err(error)
        }
    }
}

struct TracedReader {
    reader: Box<dyn std::io::Read + Send>,
    span: Option<crate::telemetry::Span>,
}

impl std::io::Read for TracedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let result = self.reader.read(buffer);
        match &result {
            Ok(0) if !buffer.is_empty() => {
                if let Some(span) = self.span.take() {
                    span.finish();
                }
            }
            Err(error) => {
                if let Some(mut span) = self.span.take() {
                    span.set_error(&error.to_string());
                    span.finish();
                }
            }
            _ => {}
        }
        result
    }
}

fn open_inner(request: OpenRequest) -> Result<OpenResponse, Error> {
    let scheme = request
        .url
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    if !matches!(scheme.as_deref(), Some("http" | "https")) {
        return Err(Error::new(format!(
            "URL {} must be http or https",
            redacted(&request.url)
        )));
    }
    if let Some(sender) = SENDER.get() {
        let response = sender.send(Request {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body.map(|body| String::from_utf8_lossy(&body).into_owned()),
            timeout: request.timeout,
            deny_internal: false,
        })?;
        let bytes = if response.bytes.is_empty() {
            response.body.into_bytes()
        } else {
            response.bytes
        };
        return Ok(OpenResponse {
            status: response.status,
            headers: response.headers,
            length: Some(bytes.len() as u64),
            reader: Box::new(std::io::Cursor::new(bytes)),
        });
    }
    open_default(request)
}

#[cfg(feature = "http")]
fn open_default(request: OpenRequest) -> Result<OpenResponse, Error> {
    /// A client that passes a redirect back instead of following it.
    fn not_following() -> &'static moqui_lib::http::RestClient {
        static CLIENT: std::sync::OnceLock<moqui_lib::http::RestClient> = std::sync::OnceLock::new();
        CLIENT.get_or_init(|| moqui_lib::http::RestClient::new().redirects(0, false))
    }
    let method = match request.method {
        Method::Get => moqui_lib::http::Method::Get,
        Method::Post => moqui_lib::http::Method::Post,
        Method::Put => moqui_lib::http::Method::Put,
        Method::Patch => moqui_lib::http::Method::Patch,
        Method::Delete => moqui_lib::http::Method::Delete,
        Method::Head => moqui_lib::http::Method::Head,
        Method::Options => moqui_lib::http::Method::Options,
    };
    let client = if request.follow_redirects {
        crate::service::shared_http_client()
    } else {
        not_following()
    };
    let opened = client.open(moqui_lib::http::OpenRequest {
        method,
        url: request.url,
        headers: request.headers,
        body: request.body,
        timeout: request.timeout,
    })?;
    Ok(OpenResponse {
        status: opened.status,
        headers: opened.headers,
        length: opened.length,
        reader: opened.reader,
    })
}

#[cfg(not(feature = "http"))]
fn open_default(request: OpenRequest) -> Result<OpenResponse, Error> {
    Err(Error::new(format!(
        "HTTP {} {} is not built into this host",
        request.method,
        redacted(&request.url)
    )))
}

/// The standard reason phrase of an HTTP status; the transport does not pass the server's own.
pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// `scheme://host/path` with no user, password, query, or fragment, for messages and logs.
#[cfg(feature = "http")]
pub fn redacted(url: &str) -> String {
    moqui_lib::http::redacted(url)
}

/// `scheme://host/path` with no user, password, query, or fragment, for messages and logs.
#[cfg(not(feature = "http"))]
pub fn redacted(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "(invalid URL)".to_string();
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    if host.is_empty() {
        return "(invalid URL)".to_string();
    }
    format!("{}://{host}{path}", scheme.to_ascii_lowercase())
}

#[cfg(all(test, not(feature = "http")))]
mod tests {
    use super::redacted;

    #[test]
    fn redacted_drops_credentials_query_and_fragment() {
        assert_eq!(
            redacted("https://user:pw@example.test:8443/a/b?token=x#frag"),
            "https://example.test:8443/a/b"
        );
        assert_eq!(redacted("http://example.test"), "http://example.test/");
        assert_eq!(redacted("not a url"), "(invalid URL)");
    }
}

#[cfg(all(test, feature = "http"))]
mod http_tests {
    use super::{send_default, Method, Request};

    /// `deny_internal` sends on the client that refuses internal addresses, before any socket.
    #[test]
    fn deny_internal_refuses_an_internal_address() {
        let request = |deny_internal| Request {
            method: Method::Post,
            url: "http://127.0.0.1:1/unsubscribe".to_string(),
            headers: Vec::new(),
            body: None,
            timeout: Some(std::time::Duration::from_secs(2)),
            deny_internal,
        };
        let denied = send_default(request(true)).unwrap_err();
        assert!(denied.message().contains("internal address"), "{denied}");
        // the shared client tries it (and finds nothing listening on port 1)
        let tried = send_default(request(false)).unwrap_err();
        assert!(!tried.message().contains("internal address"), "{tried}");
    }
}
