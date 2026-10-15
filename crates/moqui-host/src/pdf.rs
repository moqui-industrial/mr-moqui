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

//! `PdfRenderer`'s engine: one long-lived headless Chromium, driven over the DevTools Protocol
//! (`plans/archive/Architecture-Proposal.md`, Tool components and print).
//!
//! The binary is `pdf_chromium_path`, else `chromium` then `chromium-browser` on the `PATH`. It is not vendored and not
//! downloaded. When none is found the server still starts and PDF reports that it is not available. The browser starts
//! on the first render and is kept; when it exits, the next render starts it again.
//!
//! Each document gets its own page: the `html-print` text becomes the content of `about:blank`, every request the page
//! makes is failed (the document reaches no network, no server, and no session), `Page.printToPDF` runs with the
//! document's `@page` size, and the page closes. `pdf_page_limit` pages print at once and `pdf_timeout` seconds bound
//! each one. The caller's thread blocks on the edge runtime until the bytes come back.
//!
//! The protocol client is [`cdp`], on `tokio-tungstenite`. `chromiumoxide` was the planned crate, but it parses
//! messages through an untagged enum, which fails on every event with a float once the workspace's `serde_json` has
//! `arbitrary_precision`, so its pages never finish loading.

mod cdp;

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use moqui_lib::error::Error;
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};

use self::cdp::Connection;

/// Default concurrent pages.
const PAGE_LIMIT: u64 = 4;
/// Default seconds a render may take.
const TIMEOUT_SECONDS: u64 = 60;
/// How long the browser may take to start listening.
const START_WAIT: Duration = Duration::from_secs(30);
/// What the page waits for before it prints: the load event, then the fonts.
const LOADED: &str = "new Promise((resolve) => { const done = () => document.fonts.ready.then(() => resolve(true)); \
    if (document.readyState === 'complete') done(); else window.addEventListener('load', done, { once: true }); })";

/// The Chromium binary from `pdf_chromium_path`, else `chromium` or `chromium-browser` on the `PATH`.
///
/// # Errors
///
/// Returns an error when `pdf_chromium_path` names something that is not a file.
pub fn find_chromium() -> Result<Option<PathBuf>, Error> {
    let configured = moqui_lib::env::get_prop_or_env("pdf_chromium_path");
    if !configured.is_empty() {
        let path = PathBuf::from(&configured);
        if !path.is_file() {
            return Err(Error::new(format!(
                "pdf_chromium_path names {configured}, which is not a file"
            )));
        }
        return Ok(Some(path));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for name in ["chromium", "chromium-browser"] {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(None)
}

/// Find the binary and install the engine into core, or install the reason PDF is not available. The returned guard
/// removes the engine and stops the browser when it drops.
#[must_use = "dropping the guard stops the PDF browser"]
pub fn install(runtime_dir: &Path) -> PdfGuard {
    match find_chromium() {
        Ok(Some(binary)) => {
            let engine = Arc::new(ChromiumPdf::new(binary, runtime_dir.join("tmp")));
            moqui_core::screen::install_pdf_engine(engine.clone());
            PdfGuard { engine: Some(engine) }
        }
        Ok(None) => {
            eprintln!("{}", moqui_core::screen::PDF_UNAVAILABLE);
            moqui_core::screen::install_pdf_unavailable(moqui_core::screen::PDF_UNAVAILABLE);
            PdfGuard { engine: None }
        }
        Err(err) => {
            let reason = format!("PDF is not available: {err}");
            eprintln!("{reason}");
            moqui_core::screen::install_pdf_unavailable(reason);
            PdfGuard { engine: None }
        }
    }
}

/// Removes the engine from core and stops its browser.
pub struct PdfGuard {
    engine: Option<Arc<ChromiumPdf>>,
}

impl Drop for PdfGuard {
    fn drop(&mut self) {
        moqui_core::screen::clear_pdf_engine();
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
    }
}

/// The engine.
pub struct ChromiumPdf {
    inner: Arc<Inner>,
}

struct Inner {
    binary: PathBuf,
    temp_dir: PathBuf,
    pages: Semaphore,
    timeout: Duration,
    live: Mutex<Option<Live>>,
}

/// A started browser.
struct Live {
    connection: Connection,
    process: ChromiumProcess,
}

impl ChromiumPdf {
    /// An engine for `binary` whose profiles go under `temp_dir`. Nothing starts until the first render.
    #[must_use]
    pub fn new(binary: PathBuf, temp_dir: PathBuf) -> Self {
        let pages = positive_property("pdf_page_limit", PAGE_LIMIT);
        let timeout = positive_property("pdf_timeout", TIMEOUT_SECONDS);
        Self {
            inner: Arc::new(Inner {
                binary,
                temp_dir,
                pages: Semaphore::new(usize::try_from(pages).unwrap_or(4)),
                timeout: Duration::from_secs(timeout),
                live: Mutex::new(None),
            }),
        }
    }

    /// The binary this engine starts.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.inner.binary
    }

    /// Stop the browser, if one is running. A later render starts it again.
    pub fn shutdown(&self) {
        let inner = Arc::clone(&self.inner);
        let _ = crate::edge::block_on(async move {
            inner.live.lock().await.take();
        });
    }
}

impl moqui_core::screen::PdfEngine for ChromiumPdf {
    fn render(&self, html_print_document: &str) -> Result<Vec<u8>, moqui_core::context::Error> {
        let inner = Arc::clone(&self.inner);
        let html = html_print_document.to_string();
        crate::edge::block_on(async move { inner.render(html).await })
            .and_then(|rendered| rendered)
            .map_err(|err| moqui_core::context::Error::new(err.to_string()))
    }
}

impl Inner {
    async fn render(&self, html: String) -> Result<Vec<u8>, Error> {
        let _permit = self
            .pages
            .acquire()
            .await
            .map_err(|_| Error::new("the PDF engine is shutting down"))?;
        let connection = self.connection().await?;
        let page = Page::open(&connection, self.timeout).await?;
        let printed = match tokio::time::timeout(self.timeout, page.print(&html)).await {
            Ok(printed) => printed,
            Err(_) => Err(Error::new(format!(
                "PDF render did not finish in {} seconds",
                self.timeout.as_secs()
            ))),
        };
        page.close().await;
        printed
    }

    /// The browser connection, starting the browser (or starting it again after it exited) as needed.
    async fn connection(&self) -> Result<Connection, Error> {
        let mut live = self.live.lock().await;
        if let Some(current) = live.as_mut() {
            if !current.connection.is_closed() && current.process.running() {
                return Ok(current.connection.clone());
            }
            tracing::warn!("the PDF browser exited; starting it again");
            live.take();
        }
        let binary = self.binary.clone();
        let temp_dir = self.temp_dir.clone();
        let (process, url) = tokio::task::spawn_blocking(move || ChromiumProcess::spawn(&binary, &temp_dir))
            .await
            .map_err(|err| Error::new(format!("the PDF browser start panicked: {err}")))??;
        let connection = Connection::connect(&url).await?;
        tracing::info!(binary = %self.binary.display(), "started the PDF browser");
        *live = Some(Live {
            connection: connection.clone(),
            process,
        });
        Ok(connection)
    }
}

/// One page, attached with a flat session. Its requests are failed by a task until the page closes.
struct Page {
    connection: Connection,
    target: String,
    session: String,
    timeout: Duration,
    blocker: tokio::task::JoinHandle<()>,
}

impl Page {
    async fn open(connection: &Connection, timeout: Duration) -> Result<Self, Error> {
        let created = connection
            .call(None, "Target.createTarget", json!({ "url": "about:blank" }), timeout)
            .await?;
        let target = text_of(&created, "targetId")?;
        let attached = connection
            .call(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
                timeout,
            )
            .await;
        let session = match attached.and_then(|attached| text_of(&attached, "sessionId")) {
            Ok(session) => session,
            Err(err) => {
                let _ = connection
                    .call(None, "Target.closeTarget", json!({ "targetId": target }), timeout)
                    .await;
                return Err(err);
            }
        };
        let mut events = connection.events(&session);
        let blocker_connection = connection.clone();
        let blocker_session = session.clone();
        let blocker = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                if event.get("method").and_then(Value::as_str) != Some("Fetch.requestPaused") {
                    continue;
                }
                let params = event.get("params").cloned().unwrap_or(Value::Null);
                let url = params.pointer("/request/url").and_then(Value::as_str).unwrap_or("");
                tracing::debug!(%url, "PDF page request blocked");
                let fail = json!({ "requestId": params.get("requestId"), "errorReason": "BlockedByClient" });
                let _ = blocker_connection
                    .call(Some(&blocker_session), "Fetch.failRequest", fail, timeout)
                    .await;
            }
        });
        Ok(Self {
            connection: connection.clone(),
            target,
            session,
            timeout,
            blocker,
        })
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        self.connection
            .call(Some(&self.session), method, params, self.timeout)
            .await
    }

    async fn print(&self, html: &str) -> Result<Vec<u8>, Error> {
        // With no patterns every request pauses; data: URLs never do.
        self.call("Fetch.enable", json!({})).await?;
        let tree = self.call("Page.getFrameTree", json!({})).await?;
        let frame = tree
            .pointer("/frameTree/frame/id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("the PDF page has no frame"))?
            .to_string();
        self.call("Page.setDocumentContent", json!({ "frameId": frame, "html": html }))
            .await
            .map_err(|err| Error::new(format!("could not load the html-print document: {err}")))?;
        let loaded = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": LOADED, "awaitPromise": true, "returnByValue": true }),
            )
            .await?;
        if let Some(thrown) = loaded.get("exceptionDetails") {
            tracing::debug!("the html-print document did not report its load: {thrown}");
        }
        let printed = self
            .call(
                "Page.printToPDF",
                json!({ "preferCSSPageSize": true, "printBackground": true, "transferMode": "ReturnAsBase64" }),
            )
            .await
            .map_err(|err| Error::new(format!("Chromium could not print the document: {err}")))?;
        let data = printed
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("Chromium returned no PDF data"))?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|err| Error::new(format!("Chromium returned PDF data that is not base64: {err}")))
    }

    async fn close(self) {
        self.blocker.abort();
        self.connection.forget(&self.session);
        let _ = self
            .connection
            .call(
                None,
                "Target.closeTarget",
                json!({ "targetId": self.target }),
                self.timeout,
            )
            .await;
    }
}

fn text_of(value: &Value, name: &str) -> Result<String, Error> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::new(format!("the browser answered without {name}")))
}

/// The browser process and its profile directory. Drop kills the process group and removes the profile.
struct ChromiumProcess {
    child: Child,
    profile: PathBuf,
}

impl ChromiumProcess {
    /// Start the browser with a fresh profile and wait for its DevTools address.
    fn spawn(binary: &Path, temp_dir: &Path) -> Result<(Self, String), Error> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let profile = temp_dir.join(format!(
            "chromium-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile)
            .map_err(|err| Error::new(format!("could not create {}", profile.display())).with_source(err))?;
        let child = Command::new(binary)
            .args(launch_args())
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|err| Error::new(format!("could not start {}", binary.display())).with_source(err))?;
        let mut process = Self { child, profile };
        let url = devtools_url(&process.profile, &mut process.child, START_WAIT)?;
        Ok((process, url))
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ChromiumProcess {
    fn drop(&mut self) {
        // The renderer and GPU processes are in the same group.
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{}", self.child.id())])
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// The browser's arguments, apart from the profile: headless, DevTools on an ephemeral loopback port, and nothing
/// that phones home. The sandbox stays on.
#[must_use]
pub fn launch_args() -> [&'static str; 13] {
    [
        "--headless=new",
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-gpu",
        "--disable-extensions",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-sync",
        "--metrics-recording-only",
        "--password-store=basic",
        "--use-mock-keychain",
    ]
}

/// The browser socket from `DevToolsActivePort` in the profile, which Chromium writes once it listens.
///
/// # Errors
///
/// Returns an error when the browser exits first or does not listen within `wait`.
pub fn devtools_url(profile: &Path, child: &mut Child, wait: Duration) -> Result<String, Error> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + wait;
    loop {
        if let Ok(text) = std::fs::read_to_string(&file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                return Ok(format!("ws://127.0.0.1:{}{}", port.trim(), path.trim()));
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(Error::new(format!("Chromium exited before it listened: {status}")));
        }
        if Instant::now() >= deadline {
            return Err(Error::new(format!("Chromium did not write {}", file.display())));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn positive_property(name: &str, default: u64) -> u64 {
    moqui_lib::env::get_prop_or_env(name)
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(default)
}
