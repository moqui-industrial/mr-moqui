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

//! Blocking HTTP. No `ExecutionContext`.
//!
//! [`sim_allows_http`] is a pure function. The caller passes `sim`.
//! `allow_in_sim` defaults to false, so a sim call is denied. There is no
//! provider allow-list. A denied call returns before a [`PreparedRequest`]
//! exists. The first [`Call::send`] builds a `reqwest::blocking` client,
//! which runs its own runtime on its own thread.
//!
//! Call it from a framework worker or a `spawn_blocking` thread, never from an
//! async task. On a Tokio worker a debug build panics (reqwest's check) and a
//! release build blocks the worker for the whole request. There is no guard
//! here, because Tokio does not say whether blocking is allowed on a thread and
//! a check for "inside a runtime" would also refuse `spawn_blocking`.
//!
//! TLS certificates and host names are verified. Java `RestClient` trusts any
//! certificate; a remote with a self-signed certificate needs it trusted at the OS.

use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use serde::Serialize;

use crate::error::Error;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest response body read. A larger one is an error, not a truncated body.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// Whether an outbound call is allowed.
///
/// `sim` is supplied by the caller. This does not look up a Moqui context.
/// The call is allowed when `sim` is false. When `sim` is true it is denied
/// unless `allow_in_sim` is true.
pub fn sim_allows_http(sim: bool, allow_in_sim: bool) -> bool {
    !sim || allow_in_sim
}

/// HTTP method this client sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    /// The method for a name such as `put`, ignoring case. `None` when it is not one this client sends.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "POST" => Some(Self::Post),
            "PUT" => Some(Self::Put),
            "PATCH" => Some(Self::Patch),
            "DELETE" => Some(Self::Delete),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    fn as_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Method, URL, headers, and body assembled before any socket is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Duration,
}

impl PreparedRequest {
    pub fn method(&self) -> Method {
        self.method
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Status code and response body text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestResponse {
    status: u16,
    body: String,
}

impl RestResponse {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

/// Reusable blocking client. The reqwest runtime starts on the first send.
#[derive(Debug)]
pub struct RestClient {
    timeout: Duration,
    deny_internal: bool,
    transport: OnceLock<reqwest::blocking::Client>,
}

impl Default for RestClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RestClient {
    pub const fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            deny_internal: false,
            transport: OnceLock::new(),
        }
    }

    /// Refuse a host that resolves to a loopback, private, link-local, or unspecified address,
    /// including the cloud metadata address. Use it for URLs a user or a remote system supplied.
    /// The framework's own calls to a local server leave it off.
    #[must_use]
    pub fn deny_internal_addresses(mut self, deny: bool) -> Self {
        self.deny_internal = deny;
        self
    }

    /// Full-response timeout. The default is 30 seconds.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn get(&self, url: impl Into<String>) -> Call<'_> {
        self.call(Method::Get, url)
    }

    pub fn post(&self, url: impl Into<String>) -> Call<'_> {
        self.call(Method::Post, url)
    }

    pub fn put(&self, url: impl Into<String>) -> Call<'_> {
        self.call(Method::Put, url)
    }

    pub fn patch(&self, url: impl Into<String>) -> Call<'_> {
        self.call(Method::Patch, url)
    }

    pub fn delete(&self, url: impl Into<String>) -> Call<'_> {
        self.call(Method::Delete, url)
    }

    /// A call with a method chosen at run time.
    pub fn request(&self, method: Method, url: impl Into<String>) -> Call<'_> {
        self.call(method, url)
    }

    /// Send a request that already passed [`sim_allows_http`].
    pub fn execute(&self, request: PreparedRequest) -> Result<RestResponse, Error> {
        let client = self.transport()?;
        let method = request.method;
        let url = request.url;
        let shown = redacted(&url);
        if self.deny_internal {
            deny_internal(&url)?;
        }
        let mut builder = client.request(method.as_reqwest(), &url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder
            .timeout(request.timeout)
            .send()
            .map_err(|err| Error::new(format!("HTTP {method} {shown} failed")).with_source(err.without_url()))?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(Error::new(format!(
                "HTTP {method} {shown} response is larger than the limit"
            )));
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(response, MAX_RESPONSE_BYTES + 1), &mut bytes)
            .map_err(|err| Error::new(format!("HTTP {method} {shown} response body failed")).with_source(err))?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_RESPONSE_BYTES {
            return Err(Error::new(format!(
                "HTTP {method} {shown} response is larger than the limit"
            )));
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        Ok(RestResponse { status, body })
    }

    fn call(&self, method: Method, url: impl Into<String>) -> Call<'_> {
        Call {
            client: self,
            method,
            url: url.into(),
            headers: Vec::new(),
            body: PendingBody::Absent,
            timeout: self.timeout,
            allow_in_sim: false,
        }
    }

    fn transport(&self) -> Result<&reqwest::blocking::Client, Error> {
        if let Some(client) = self.transport.get() {
            return Ok(client);
        }
        let built = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(redirect_policy())
            .build()
            .map_err(|err| Error::new("could not build the HTTP client").with_source(err))?;
        Ok(self.transport.get_or_init(|| built))
    }
}

#[derive(Debug)]
enum PendingBody {
    Absent,
    Json(String),
    Text(String),
    Invalid(Error),
}

/// One call on a [`RestClient`]. `allow_in_sim` defaults to false.
#[derive(Debug)]
#[must_use = "call prepare or send to use this HTTP call"]
pub struct Call<'a> {
    client: &'a RestClient,
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    body: PendingBody,
    timeout: Duration,
    allow_in_sim: bool,
}

impl Call<'_> {
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// JSON body. Sets `content-type: application/json` unless one is already set.
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.body = match serde_json::to_string(value) {
            Ok(text) => PendingBody::Json(text),
            Err(err) => PendingBody::Invalid(Error::new("could not encode the JSON body").with_source(err)),
        };
        self
    }

    /// A text body sent as given, such as newline-delimited JSON. Sets `content-type` unless one is
    /// already set.
    pub fn text(mut self, content_type: &str, body: impl Into<String>) -> Self {
        if !has_content_type(&self.headers) {
            self.headers
                .push(("content-type".to_string(), content_type.to_string()));
        }
        self.body = PendingBody::Text(body.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Allow this call when `sim` is true. Defaults to false.
    pub fn allow_in_sim(mut self, allow_in_sim: bool) -> Self {
        self.allow_in_sim = allow_in_sim;
        self
    }

    /// Build the request when the sim predicate allows it.
    ///
    /// A denial returns before [`PreparedRequest`] is constructed.
    pub fn prepare(self, sim: bool) -> Result<PreparedRequest, Error> {
        if !sim_allows_http(sim, self.allow_in_sim) {
            return Err(Error::new(format!(
                "HTTP {} {} denied because sim mode is active",
                self.method,
                redacted(&self.url)
            )));
        }
        self.into_prepared()
    }

    /// [`prepare`](Self::prepare), then send with blocking reqwest.
    pub fn send(self, sim: bool) -> Result<RestResponse, Error> {
        let client = self.client;
        let request = self.prepare(sim)?;
        client.execute(request)
    }

    fn into_prepared(self) -> Result<PreparedRequest, Error> {
        let parsed = reqwest::Url::parse(&self.url)
            .map_err(|err| Error::new(format!("invalid URL {}", redacted(&self.url))).with_source(err))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::new(format!("URL {} must be http or https", redacted(&self.url))));
        }
        for (name, value) in &self.headers {
            check_header(name, value)?;
        }
        let mut headers = self.headers;
        let body = match self.body {
            PendingBody::Absent => None,
            PendingBody::Invalid(err) => return Err(err),
            PendingBody::Json(text) => {
                if !has_content_type(&headers) {
                    headers.push(("content-type".to_string(), "application/json".to_string()));
                }
                Some(text)
            }
            PendingBody::Text(text) => Some(text),
        };
        Ok(PreparedRequest {
            method: self.method,
            url: self.url,
            headers,
            body,
            timeout: self.timeout,
        })
    }
}

/// Follow a few redirects, and only to the same host over the same or a safer scheme. A redirect
/// anywhere else comes back as the 3xx response, so a header meant for one server never goes to another.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.stop();
        }
        let next = attempt.url();
        let origin = attempt.previous().first().cloned();
        let same_host = origin.as_ref().is_some_and(|first| {
            first.host_str() == next.host_str() && first.port_or_known_default() == next.port_or_known_default()
        });
        let downgrade = origin
            .as_ref()
            .is_some_and(|first| first.scheme() == "https" && next.scheme() != "https");
        if same_host && !downgrade && matches!(next.scheme(), "http" | "https") {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// `scheme://host/path` with no user, password, or query, for messages and logs.
pub fn redacted(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("");
            let port = parsed.port().map(|port| format!(":{port}")).unwrap_or_default();
            format!("{}://{host}{port}{}", parsed.scheme(), parsed.path())
        }
        Err(_) => "(invalid URL)".to_string(),
    }
}

fn deny_internal(url: &str) -> Result<(), Error> {
    use std::net::ToSocketAddrs;

    let parsed = reqwest::Url::parse(url)
        .map_err(|err| Error::new(format!("invalid URL {}", redacted(url))).with_source(err))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::new(format!("URL {} has no host", redacted(url))))?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|err| Error::new(format!("could not resolve {host}")).with_source(err))?;
    for address in addresses {
        if internal_address(address.ip()) {
            return Err(Error::new(format!(
                "{} is an internal address and is not allowed here",
                redacted(url)
            )));
        }
    }
    Ok(())
}

fn internal_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified() || v4.is_broadcast()
        }
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                // unique local fc00::/7 and link local fe80::/10
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| internal_address(std::net::IpAddr::V4(v4)))
        }
    }
}

fn has_content_type(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
}

fn check_header(name: &str, value: &str) -> Result<(), Error> {
    HeaderName::try_from(name)
        .map_err(|err| Error::new(format!("invalid HTTP header name {name}")).with_source(err))?;
    HeaderValue::try_from(value)
        .map_err(|err| Error::new(format!("invalid HTTP header value for {name}")).with_source(err))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_true_and_allow_false_denies_and_does_not_produce_a_send() {
        assert!(!sim_allows_http(true, false));
        let client = RestClient::new();
        let error = client
            .post("http://example.test/denied")
            .json(&serde_json::json!({"name": "nope"}))
            .prepare(true)
            .expect_err("sim denies");
        assert!(error.message().contains("denied because sim mode is active"), "{error}");
        assert!(error.message().contains("http://example.test/denied"));
        assert!(client.transport.get().is_none());
    }

    #[test]
    fn redaction_drops_user_password_and_query() {
        assert_eq!(
            redacted("https://user:secret@example.test:8443/a/b?token=1#x"),
            "https://example.test:8443/a/b"
        );
        assert_eq!(redacted("not a url"), "(invalid URL)");
    }

    #[test]
    fn only_http_and_https_are_sent_and_methods_parse() {
        let client = RestClient::new();
        let err = client.get("file:///etc/passwd").prepare(false).expect_err("scheme");
        assert!(err.message().contains("http or https"), "{err}");
        assert_eq!(Method::parse("put"), Some(Method::Put));
        assert_eq!(Method::parse("TRACE"), None);
        let delete = client.delete("http://example.test/x").prepare(false).unwrap();
        assert_eq!(delete.method(), Method::Delete);
    }

    #[test]
    fn internal_addresses_are_recognized() {
        for text in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.9",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(internal_address(text.parse().unwrap()), "{text}");
        }
        assert!(!internal_address("93.184.216.34".parse().unwrap()));
        let client = RestClient::new().deny_internal_addresses(true);
        let request = client.get("http://127.0.0.1:9/x").prepare(false).unwrap();
        let err = client.execute(request).expect_err("loopback is denied before a socket");
        assert!(err.message().contains("internal address"), "{err}");
    }

    #[test]
    fn sim_true_and_allow_true_prepares_a_request() {
        assert!(sim_allows_http(true, true));
        let client = RestClient::new();
        let request = client
            .get("http://example.test/allowed")
            .allow_in_sim(true)
            .prepare(true)
            .expect("allow in sim");
        assert_eq!(request.method(), Method::Get);
        assert_eq!(request.url(), "http://example.test/allowed");
        assert!(client.transport.get().is_none());
    }

    #[test]
    fn sim_false_allows() {
        assert!(sim_allows_http(false, false));
        let client = RestClient::new();
        let request = client.get("http://example.test/live").prepare(false).expect("not sim");
        assert_eq!(request.method(), Method::Get);
        assert_eq!(request.url(), "http://example.test/live");
        assert!(request.body().is_none());
        assert!(client.transport.get().is_none());
    }

    #[test]
    fn get_and_post_json_show_method_url_and_body() {
        let client = RestClient::new();
        let get = client
            .get("http://example.test/items")
            .header("accept", "application/json")
            .prepare(false)
            .expect("get");
        assert_eq!(get.method(), Method::Get);
        assert_eq!(get.url(), "http://example.test/items");
        assert_eq!(get.body(), None);
        assert_eq!(get.headers(), [("accept".to_string(), "application/json".to_string())]);

        let post = client
            .post("http://example.test/items")
            .header("x-request-id", "one")
            .json(&serde_json::json!({"name": "widget"}))
            .prepare(false)
            .expect("post");
        assert_eq!(post.method(), Method::Post);
        assert_eq!(post.url(), "http://example.test/items");
        assert_eq!(post.body(), Some(r#"{"name":"widget"}"#));
        assert_eq!(
            post.headers(),
            [
                ("x-request-id".to_string(), "one".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ]
        );
        assert!(client.transport.get().is_none());
    }
}
