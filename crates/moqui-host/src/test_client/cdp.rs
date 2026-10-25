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

//! A small synchronous Chrome DevTools Protocol client, for the browser of `moqui test-client` (`browser.rs`) and the
//! Gate 3 browser smoke (`tests/gate3_browser.rs`, through `tests/common/cdp.rs`): one Chromium with a temporary
//! profile, pages attached with flat sessions (in browser contexts of their own when asked, or more windows of the
//! same profile, [`Browser::open_tab`], one of them the current at a time), `Runtime.evaluate`, input events, and the
//! console errors, uncaught exceptions, log entries and failed responses the pages report. Each failure is an `Err`
//! with its text. It is not the PDF engine's client (`pdf/cdp.rs`, async) and does not use `chromiumoxide`.

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

/// How long the browser may take to start listening, and a command to answer.
const ANSWER_WAIT: Duration = Duration::from_secs(60);

/// The socket's read timeout, how long a read waits for a message that does not come.
const READ_WAIT: Duration = Duration::from_millis(200);

/// The events kept for the waits; the oldest go first.
const KEPT_EVENTS: usize = 5000;

/// What a page said while it ran: console errors and warnings, uncaught exceptions, log entries, responses of 400
/// and up and failed loads. `session` is the page's.
#[derive(Clone, Debug)]
pub struct Problem {
    /// `exception`, `console.error`, `console.assert`, `console.warning`, `log.error`, `log.warning`, `response` or
    /// `failed`.
    pub kind: String,
    pub text: String,
    pub session: String,
}

impl Problem {
    /// A warning, which is a problem only when asked for.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.kind.ends_with(".warning") || self.kind.ends_with(".warn")
    }
}

/// How to start the browser.
#[derive(Clone, Debug)]
pub struct Launch {
    pub binary: PathBuf,
    /// A visible browser, for debugging.
    pub headed: bool,
    /// The window size, which is the viewport of a page with no override.
    pub window: (u32, u32),
    /// The profile's name starts with this, under the system's temporary directory.
    pub profile_prefix: String,
    /// More arguments, such as `--ignore-certificate-errors`.
    pub extra_args: Vec<String>,
}

pub struct Browser {
    child: Child,
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    session: String,
    /// The first page's session once another was opened.
    first: String,
    next_id: u64,
    events: Vec<Value>,
    problems: Vec<Problem>,
    /// The URLs of the requests in flight, by request id, for a failed load's problem.
    requests: HashMap<String, String>,
    /// The sessions with `Network` on, whose network log entries are left to their responses.
    network: HashSet<String>,
    profile: PathBuf,
}

impl Browser {
    /// Start the browser and attach its first page, the current one.
    ///
    /// # Errors
    ///
    /// Returns why the browser did not start or answer.
    pub fn launch(launch: &Launch) -> Result<Browser, String> {
        let mut browser = Self::start(launch)?;
        browser.session = browser.attach_page(None, false, false)?;
        Ok(browser)
    }

    /// Start the browser with no page attached: [`Browser::new_page`] attaches each.
    ///
    /// # Errors
    ///
    /// Returns why the browser did not start or answer.
    pub fn start(launch: &Launch) -> Result<Browser, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let profile = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            launch.profile_prefix,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).map_err(|err| format!("{}: {err}", profile.display()))?;
        let mut args: Vec<String> = crate::chromium::launch_args()
            .iter()
            .filter(|arg| !launch.headed || !arg.starts_with("--headless"))
            .map(|arg| (*arg).to_string())
            .collect();
        args.extend(launch.extra_args.iter().cloned());
        args.push(format!("--window-size={},{}", launch.window.0, launch.window.1));
        args.push(format!("--user-data-dir={}", profile.display()));
        args.push("about:blank".to_string());
        let spawned = Command::new(&launch.binary)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        let child = match spawned {
            Ok(child) => child,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&profile);
                return Err(format!("could not start {}: {err}", launch.binary.display()));
            }
        };
        let mut guard = ChildGuard(Some(child), profile.clone());
        let child = guard.0.as_mut().ok_or("the browser did not start")?;
        let url = crate::chromium::devtools_url(&profile, child, Duration::from_secs(30)).map_err(|err| {
            if crate::chromium::userns_restricted() {
                format!(
                    "{err}; AppArmor limits unprivileged user namespaces here, so {} may need an AppArmor profile to \
                     start its sandbox (./moq help chromium-download)",
                    launch.binary.display()
                )
            } else {
                err.to_string()
            }
        })?;
        let (ws, _) = connect(url.as_str()).map_err(|err| format!("devtools socket {url}: {err}"))?;
        let child = guard.0.take().ok_or("the browser did not start")?;
        let mut browser = Browser {
            child,
            ws,
            session: String::new(),
            first: String::new(),
            next_id: 0,
            events: Vec::new(),
            problems: Vec::new(),
            requests: HashMap::new(),
            network: HashSet::new(),
            profile,
        };
        browser.set_timeout(READ_WAIT)?;
        Ok(browser)
    }

    /// A browser context of its own (its own cookies and storage), for [`Browser::new_page`].
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn new_context(&mut self) -> Result<String, String> {
        let created = self.call_browser("Target.createBrowserContext", json!({"disposeOnDetach": true}))?;
        created["browserContextId"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "Target.createBrowserContext answered without browserContextId".to_string())
    }

    /// A new page in `context` (or the default one), attached with its own session and made the current page. With
    /// `network`, its responses and failed loads are problems; `scripts` run in each new document before the page's own.
    /// The answer is the page's session.
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn new_page(&mut self, context: Option<&str>, network: bool, scripts: &[&str]) -> Result<String, String> {
        let session = self.attach_page(context, false, network)?;
        for script in scripts {
            self.send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": script}),
                Some(&session),
            )?;
        }
        self.session = session.clone();
        Ok(session)
    }

    /// Close the page of `session` (none when it is empty), and its browser context when it had one of its own.
    pub fn close_page(&mut self, session: &str, context: Option<&str>) {
        let target = if session.is_empty() {
            Err(String::new())
        } else {
            self.send("Target.getTargetInfo", json!({}), Some(session))
        };
        if let Some(target_id) = target
            .ok()
            .and_then(|info| info["targetInfo"]["targetId"].as_str().map(str::to_string))
        {
            let _ = self.call_browser("Target.closeTarget", json!({"targetId": target_id}));
        }
        if let Some(context) = context {
            let _ = self.call_browser("Target.disposeBrowserContext", json!({"browserContextId": context}));
        }
        self.network.remove(session);
        self.problems.retain(|problem| problem.session != session);
        if self.session == session {
            self.session.clear();
        }
    }

    /// A new page attached with its own session: in a window of its own when `window`, so each is visible (a page in a
    /// background tab is hidden).
    fn attach_page(&mut self, context: Option<&str>, window: bool, network: bool) -> Result<String, String> {
        let mut params = json!({"url": "about:blank", "newWindow": window});
        if let Some(context) = context {
            // a context of its own has no window yet, and Chrome opens a tab only in a window
            params["browserContextId"] = Value::String(context.to_string());
            params["newWindow"] = Value::Bool(true);
        }
        let target = self.call_browser("Target.createTarget", params)?;
        let target_id = target["targetId"]
            .as_str()
            .ok_or("Target.createTarget answered without targetId")?
            .to_string();
        let attached = self.call_browser("Target.attachToTarget", json!({"targetId": target_id, "flatten": true}))?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or("Target.attachToTarget answered without sessionId")?
            .to_string();
        let mut domains = vec!["Runtime.enable", "Log.enable", "Page.enable"];
        if network {
            domains.push("Network.enable");
            self.network.insert(session.clone());
        }
        for domain in domains {
            self.send(domain, json!({}), Some(&session))?;
        }
        Ok(session)
    }

    /// Open another page (a second tab of the same profile, so the same sign-in, in a window of its own) and make it the
    /// current one. The answer names it for [`Browser::use_tab`]; the first page is [`Browser::first_tab`].
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn open_tab(&mut self) -> Result<String, String> {
        let first = self.session.clone();
        if self.first.is_empty() {
            self.first = first;
        }
        self.session = self.attach_page(None, true, false)?;
        Ok(self.session.clone())
    }

    /// The page [`Browser::launch`] opened.
    #[must_use]
    pub fn first_tab(&self) -> String {
        if self.first.is_empty() {
            self.session.clone()
        } else {
            self.first.clone()
        }
    }

    /// Make `tab` the page that the commands, the waits and the clicks go to.
    pub fn use_tab(&mut self, tab: &str) {
        self.session = tab.to_string();
    }

    /// The current page's session.
    #[must_use]
    pub fn current(&self) -> &str {
        &self.session
    }

    fn set_timeout(&mut self, timeout: Duration) -> Result<(), String> {
        if let MaybeTlsStream::Plain(stream) = self.ws.get_mut() {
            stream
                .set_read_timeout(Some(timeout))
                .map_err(|err| format!("devtools socket: {err}"))?;
        }
        Ok(())
    }

    fn call_browser(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.send(method, params, None)
    }

    /// A command to the current page.
    ///
    /// # Errors
    ///
    /// Returns the browser's error, or that it did not answer.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let session = self.session.clone();
        self.send(method, params, Some(&session))
    }

    fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_string());
        }
        self.ws
            .send(Message::Text(message.to_string().into()))
            .map_err(|err| format!("{method}: devtools socket: {err}"))?;
        let deadline = Instant::now() + ANSWER_WAIT;
        loop {
            if Instant::now() >= deadline {
                return Err(format!("no answer to {method} within {}s", ANSWER_WAIT.as_secs()));
            }
            let Some(message) = self.read()? else { continue };
            if message["id"].as_u64() == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("{method}: {error}"));
                }
                return Ok(message["result"].clone());
            }
        }
    }

    /// One message, or `None` when nothing came within the read timeout. Events are kept.
    fn read(&mut self) -> Result<Option<Value>, String> {
        match self.ws.read() {
            Ok(Message::Text(text)) => {
                let message: Value = serde_json::from_str(&text)
                    .map_err(|err| format!("devtools socket: a message did not read: {err}"))?;
                if message.get("method").is_some() {
                    self.record(&message);
                }
                Ok(Some(message))
            }
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(format!("devtools socket: {err}")),
        }
    }

    fn problem(&mut self, kind: impl Into<String>, text: String, session: &str) {
        self.problems.push(Problem {
            kind: kind.into(),
            text,
            session: session.to_string(),
        });
    }

    fn record(&mut self, event: &Value) {
        let params = &event["params"];
        let session = event["sessionId"].as_str().unwrap_or("").to_string();
        let method = event["method"].as_str().unwrap_or("");
        match method {
            "Runtime.exceptionThrown" => {
                let details = &params["exceptionDetails"];
                let text = details["exception"]["description"]
                    .as_str()
                    .or_else(|| details["text"].as_str())
                    .unwrap_or("")
                    .to_string();
                let at = format!("{text} at {}:{}", details["url"], details["lineNumber"]);
                self.problem("exception", at, &session);
            }
            "Runtime.consoleAPICalled" => {
                let kind = params["type"].as_str().unwrap_or("");
                if matches!(kind, "error" | "assert" | "warning" | "warn") {
                    let text = params["args"]
                        .as_array()
                        .map(|args| {
                            args.iter()
                                .map(|arg| {
                                    arg["value"]
                                        .as_str()
                                        .map(str::to_string)
                                        .or_else(|| arg["description"].as_str().map(str::to_string))
                                        .unwrap_or_else(|| arg["value"].to_string())
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    self.problem(format!("console.{kind}"), text, &session);
                }
            }
            "Log.entryAdded" => {
                let entry = &params["entry"];
                let level = entry["level"].as_str().unwrap_or("");
                // a page with Network on reports its responses and failed loads from there
                let from_network = entry["source"] == "network" && self.network.contains(&session);
                if matches!(level, "error" | "warning") && !from_network {
                    let text = format!(
                        "{} {}",
                        entry["text"].as_str().unwrap_or(""),
                        entry["url"].as_str().unwrap_or("")
                    );
                    self.problem(format!("log.{level}"), text, &session);
                }
            }
            "Network.requestWillBeSent" => {
                if let (Some(id), Some(url)) = (params["requestId"].as_str(), params["request"]["url"].as_str()) {
                    if self.requests.len() > 10_000 {
                        self.requests.clear();
                    }
                    self.requests.insert(id.to_string(), url.to_string());
                }
                return;
            }
            "Network.responseReceived" => {
                let response = &params["response"];
                let status = response["status"].as_u64().unwrap_or(0);
                if status >= 400 {
                    let text = format!(
                        "{status} {} {}",
                        response["statusText"].as_str().unwrap_or(""),
                        response["url"].as_str().unwrap_or("")
                    );
                    self.problem("response", text.replace("  ", " "), &session);
                }
                // a document's answer is kept for its navigation's status
                if params["type"] != "Document" {
                    return;
                }
            }
            "Network.loadingFinished" => {
                if let Some(id) = params["requestId"].as_str() {
                    self.requests.remove(id);
                }
                return;
            }
            "Network.loadingFailed" => {
                let url = params["requestId"]
                    .as_str()
                    .and_then(|id| self.requests.remove(id))
                    .unwrap_or_default();
                if params["canceled"] != true {
                    let text = format!("{} {url}", params["errorText"].as_str().unwrap_or("failed"));
                    self.problem("failed", text, &session);
                }
                return;
            }
            other if other.starts_with("Network.") => return,
            _ => {}
        }
        if self.events.len() >= KEPT_EVENTS {
            self.events.drain(..KEPT_EVENTS / 2);
        }
        self.events.push(event.clone());
    }

    /// Read events for `duration`, and no longer: each read waits at most what is left of it, not [`READ_WAIT`].
    ///
    /// # Errors
    ///
    /// Returns an error when the socket fails.
    pub fn pump(&mut self, duration: Duration) -> Result<(), String> {
        let deadline = Instant::now() + duration;
        let mut read = Ok(());
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            if let Err(err) = self.set_timeout(left.min(READ_WAIT).max(Duration::from_millis(1))) {
                read = Err(err);
                break;
            }
            if let Err(err) = self.read() {
                read = Err(err);
                break;
            }
        }
        self.set_timeout(READ_WAIT)?;
        read
    }

    /// Go to `url` and wait for its load event; the navigation's loader id, which is the request id of its document.
    ///
    /// # Errors
    ///
    /// Returns the navigation's error, or that the load event did not come within `timeout`.
    pub fn navigate_within(&mut self, url: &str, timeout: Duration) -> Result<String, String> {
        self.events.clear();
        let result = self.call("Page.navigate", json!({"url": url}))?;
        if let Some(error) = result.get("errorText") {
            return Err(format!("navigate {url}: {error}"));
        }
        self.wait_event("Page.loadEventFired", timeout)?;
        Ok(result["loaderId"].as_str().unwrap_or("").to_string())
    }

    /// Go to `url` and wait for its load event, for up to 60 seconds.
    ///
    /// # Errors
    ///
    /// As [`Browser::navigate_within`].
    pub fn navigate(&mut self, url: &str) -> Result<(), String> {
        self.navigate_within(url, ANSWER_WAIT).map(|_| ())
    }

    /// The status of the document `loader` (a navigation's loader id) answered with, once its response came.
    #[must_use]
    pub fn document_status(&self, loader: &str) -> Option<(u64, String)> {
        self.events.iter().rev().find_map(|event| {
            let params = &event["params"];
            (event["method"] == "Network.responseReceived"
                && event["sessionId"] == self.session.as_str()
                && params["requestId"] == loader)
                .then(|| {
                    (
                        params["response"]["status"].as_u64().unwrap_or(0),
                        params["response"]["url"].as_str().unwrap_or("").to_string(),
                    )
                })
        })
    }

    /// Load the current page again and wait for its load event.
    ///
    /// # Errors
    ///
    /// Returns that the load event did not come.
    pub fn reload(&mut self) -> Result<(), String> {
        self.events.clear();
        self.call("Page.reload", json!({}))?;
        self.wait_event("Page.loadEventFired", ANSWER_WAIT)
    }

    /// Whether an event of the current page named `method` came after the last [`Browser::navigate`] or
    /// [`Browser::clear_events`].
    #[must_use]
    pub fn has_event(&self, method: &str) -> bool {
        self.events.iter().any(|event| {
            event["method"] == method
                && event["sessionId"]
                    .as_str()
                    .is_none_or(|session| session == self.session)
        })
    }

    /// Wait for an event of the current page named `method` that came after the last [`Browser::navigate`] or
    /// [`Browser::clear_events`].
    ///
    /// # Errors
    ///
    /// Returns that it did not come within `timeout`.
    pub fn wait_event(&mut self, method: &str, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.has_event(method) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("no {method} within {timeout:?}"));
            }
            self.read()?;
        }
    }

    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    /// The value of a JavaScript expression in the page, after its promise settles. An exception is an `Err` with its
    /// text, and so is a browser error.
    ///
    /// # Errors
    ///
    /// Returns the exception's text or the browser's error.
    pub fn try_eval(&mut self, expression: &str) -> Result<Value, String> {
        let result = self.call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true}),
        )?;
        if let Some(details) = result.get("exceptionDetails") {
            let text = details["exception"]["description"]
                .as_str()
                .or_else(|| details["text"].as_str())
                .unwrap_or("exception")
                .to_string();
            return Err(text);
        }
        Ok(result["result"]["value"].clone())
    }

    /// [`Browser::try_eval`] with the expression in the error.
    ///
    /// # Errors
    ///
    /// Returns the expression and its exception's text.
    pub fn eval(&mut self, expression: &str) -> Result<Value, String> {
        self.try_eval(expression).map_err(|err| format!("{expression}: {err}"))
    }

    /// Poll `expression` until it is truthy. The failure names `what`, the last value, the problems and the page text.
    ///
    /// # Errors
    ///
    /// Returns that it was not true within `timeout`.
    pub fn wait_for(&mut self, what: &str, expression: &str, timeout: Duration) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        let mut last;
        loop {
            match self.try_eval(expression) {
                Ok(value) if truthy(&value) => return Ok(value),
                Ok(value) => last = value,
                Err(err) => last = Value::String(err),
            }
            if Instant::now() >= deadline {
                let text = self
                    .try_eval("document.body ? document.body.innerText.slice(0, 3000) : ''")
                    .unwrap_or_default();
                let session = self.session.clone();
                let problems: Vec<&Problem> = self.problems.iter().filter(|p| p.session == session).collect();
                return Err(format!(
                    "{what}: not true within {timeout:?} (last {last}); problems {problems:?}\npage text: {text}"
                ));
            }
            self.pump(Duration::from_millis(100))?;
        }
    }

    /// A left click at `(x, y)` of the viewport: the mouse moves there, presses and releases, as a user's click.
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn click_at(&mut self, x: f64, y: f64) -> Result<(), String> {
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.call(
                "Input.dispatchMouseEvent",
                json!({"type": kind, "x": x, "y": y, "button": "left", "buttons": if kind == "mousePressed" { 1 } else { 0 }, "clickCount": 1}),
            )?;
        }
        Ok(())
    }

    /// The mouse to `(x, y)` with no button down.
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn hover_at(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.call(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0}),
        )
        .map(|_| ())
    }

    /// A mouse click at the middle of the element `element` (an expression), after it is scrolled into view, as a user's click:
    /// Quasar opens menus and popups on pointer events that `element.click()` does not send.
    ///
    /// # Errors
    ///
    /// Returns that the element was not there with a box within 20 seconds, or the browser's error.
    pub fn click(&mut self, what: &str, element: &str) -> Result<(), String> {
        let rect = self.wait_for(
            what,
            &format!(
                "(() => {{ const el = ({element}); if (!el) return null; el.scrollIntoView({{block: 'center'}}); const r = el.getBoundingClientRect(); \
                 return r.width > 0 && r.height > 0 ? [r.left + r.width / 2, r.top + r.height / 2] : null; }})()"
            ),
            Duration::from_secs(20),
        )?;
        let (x, y) = (rect[0].as_f64().unwrap_or(0.0), rect[1].as_f64().unwrap_or(0.0));
        self.click_at(x, y)?;
        self.pump(Duration::from_millis(150))
    }

    /// Type text into the focused element.
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.call("Input.insertText", json!({"text": text}))?;
        self.pump(Duration::from_millis(100))
    }

    /// Press a key such as `Enter` or `Escape` on the focused element.
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn press(&mut self, key: &str) -> Result<(), String> {
        for kind in ["keyDown", "keyUp"] {
            self.call(
                "Input.dispatchKeyEvent",
                json!({"type": kind, "key": key, "code": key, "windowsVirtualKeyCode": match key { "Enter" => 13, "Escape" => 27, _ => 0 }}),
            )?;
        }
        self.pump(Duration::from_millis(150))
    }

    /// Press a key with its modifiers, as Playwright names them: `Enter`, `Tab`, `ArrowDown`, `a`, `Control+a`,
    /// `Shift+Tab`. A printable key with no Control, Alt or Meta types its text.
    ///
    /// # Errors
    ///
    /// Returns the browser's error.
    pub fn press_chord(&mut self, chord: &str) -> Result<(), String> {
        let parts: Vec<&str> = if chord == "+" {
            vec!["+"]
        } else {
            chord.split('+').filter(|part| !part.is_empty()).collect()
        };
        let Some((key, modifier_names)) = parts.split_last() else {
            return Err(format!("press: '{chord}' names no key"));
        };
        let mut modifiers = 0;
        for name in modifier_names {
            modifiers |= match *name {
                "Alt" => 1,
                "Control" | "Ctrl" => 2,
                "Meta" | "Command" => 4,
                "Shift" => 8,
                other => {
                    return Err(format!(
                        "press: {other} in '{chord}' is not Alt, Control, Meta or Shift"
                    ))
                }
            };
        }
        let (key, code, key_code) = key_codes(key);
        let typed = key.chars().count() == 1 && modifiers & 7 == 0;
        let down = if typed { "keyDown" } else { "rawKeyDown" };
        let mut event =
            json!({"type": down, "key": key, "code": code, "windowsVirtualKeyCode": key_code, "modifiers": modifiers});
        if typed {
            event["text"] = Value::String(key.clone());
        } else if key == "Enter" {
            event["text"] = Value::String("\r".to_string());
            event["type"] = Value::String("keyDown".to_string());
        }
        self.call("Input.dispatchKeyEvent", event)?;
        self.call(
            "Input.dispatchKeyEvent",
            json!({"type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": key_code, "modifiers": modifiers}),
        )?;
        Ok(())
    }

    /// The problems reported since the last call, and forget them.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket fails.
    pub fn take_problems(&mut self) -> Result<Vec<Problem>, String> {
        self.pump(Duration::from_millis(300))?;
        Ok(std::mem::take(&mut self.problems))
    }

    /// The problems of the page of `session` so far, and forget them, with no wait.
    pub fn take_problems_of(&mut self, session: &str) -> Vec<Problem> {
        let (taken, kept): (Vec<Problem>, Vec<Problem>) = std::mem::take(&mut self.problems)
            .into_iter()
            .partition(|problem| problem.session == session);
        self.problems = kept;
        taken
    }

    /// The problems of the page of `session` so far, kept.
    #[must_use]
    pub fn problems_of(&self, session: &str) -> Vec<Problem> {
        self.problems
            .iter()
            .filter(|problem| problem.session == session)
            .cloned()
            .collect()
    }

    /// The profile directory, removed when the browser drops.
    #[must_use]
    pub fn profile(&self) -> &Path {
        &self.profile
    }
}

/// The DOM key, code and Windows key code of a Playwright key name.
fn key_codes(key: &str) -> (String, String, u32) {
    let named = match key {
        "Enter" => Some(("Enter", 13)),
        "Escape" => Some(("Escape", 27)),
        "Tab" => Some(("Tab", 9)),
        "Backspace" => Some(("Backspace", 8)),
        "Delete" => Some(("Delete", 46)),
        "ArrowUp" => Some(("ArrowUp", 38)),
        "ArrowDown" => Some(("ArrowDown", 40)),
        "ArrowLeft" => Some(("ArrowLeft", 37)),
        "ArrowRight" => Some(("ArrowRight", 39)),
        "Home" => Some(("Home", 36)),
        "End" => Some(("End", 35)),
        "PageUp" => Some(("PageUp", 33)),
        "PageDown" => Some(("PageDown", 34)),
        "Space" | " " => return (" ".to_string(), "Space".to_string(), 32),
        _ => None,
    };
    if let Some((code, key_code)) = named {
        return (key.to_string(), code.to_string(), key_code);
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => (
            key.to_string(),
            format!("Key{}", c.to_ascii_uppercase()),
            u32::from(c.to_ascii_uppercase()),
        ),
        (Some(c), None) if c.is_ascii_digit() => (key.to_string(), format!("Digit{c}"), u32::from(c)),
        _ => (key.to_string(), key.to_string(), 0),
    }
}

impl Drop for Browser {
    /// Ask the browser to close, so it stops its helper processes, then kill what is left of its process group and remove the profile,
    /// which a helper may still be writing to for a moment.
    fn drop(&mut self) {
        self.next_id += 1;
        let close = json!({"id": self.next_id, "method": "Browser.close", "params": {}});
        let _ = self.ws.send(Message::Text(close.to_string().into()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(50));
        }
        kill_group(&mut self.child);
        remove_profile(&self.profile);
    }
}

fn kill_group(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{}", child.id())])
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

fn remove_profile(profile: &Path) {
    for _ in 0..20 {
        if std::fs::remove_dir_all(profile).is_ok() || !profile.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// JavaScript's truthiness of a value the page answered.
#[must_use]
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Kills the browser and removes its profile when the launch stops before the `Browser` owns it.
struct ChildGuard(Option<Child>, PathBuf);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            kill_group(child);
            remove_profile(&self.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::key_codes;

    #[test]
    fn keys_have_their_codes() {
        assert_eq!(key_codes("Enter"), ("Enter".to_string(), "Enter".to_string(), 13));
        assert_eq!(key_codes("a"), ("a".to_string(), "KeyA".to_string(), 65));
        assert_eq!(key_codes("7"), ("7".to_string(), "Digit7".to_string(), 55));
        assert_eq!(key_codes("Space"), (" ".to_string(), "Space".to_string(), 32));
    }
}
