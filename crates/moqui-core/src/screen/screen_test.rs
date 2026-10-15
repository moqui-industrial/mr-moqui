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

//! `ScreenTest`: render a screen path or a transition with a parameter map and read the result,
//! with no socket. `ec.screen.makeTest()` in Java. It is `make_render` with the web slot kept
//! apart from a render in progress, so a screen can call it while it renders. `sri.getFieldOptions`
//! uses it for `dynamic-options`, and the server-static `qvt` actions will use it for the same.

use std::collections::BTreeMap;
use std::sync::Arc;

use moqui_lib::object::Value;

use super::render::{make_render, RenderOutput};
use crate::context::{Error, ExecutionContextFactory};

/// What a test render sent.
#[derive(Clone, Debug)]
pub struct TestRender {
    pub output: RenderOutput,
}

impl TestRender {
    pub fn status(&self) -> u16 {
        self.output.status
    }

    /// The body as text: the screen, or a transition's JSON.
    pub fn text(&self) -> String {
        self.output.body_text()
    }

    /// The body parsed as JSON, as `ec.web.sendJsonResponse` sent it.
    ///
    /// # Errors
    ///
    /// Returns an error when the body is not JSON.
    pub fn json(&self) -> Result<serde_json::Value, Error> {
        serde_json::from_slice(&self.output.body)
            .map_err(|err| Error::new(format!("the render did not send JSON: {err}")))
    }

    /// The JSON as a value, null for a body that is empty.
    ///
    /// # Errors
    ///
    /// Returns an error when the body is not JSON.
    pub fn json_value(&self) -> Result<Value, Error> {
        if self.output.body.is_empty() {
            return Ok(Value::Null);
        }
        Ok(moqui_lib::json::from_json(&self.json()?))
    }
}

/// What a [`ScreenTest::test_render`] found, as Java's `ScreenTestRender`: the output, and every error of the
/// render (an error it raised, `ec.message` errors it added, error text in its output) instead of a failure.
#[derive(Clone, Debug)]
pub struct ScreenTestRender {
    pub screen_path: String,
    /// The response, or `None` when the render raised an error.
    pub output: Option<RenderOutput>,
    pub error_messages: Vec<String>,
    pub render_time_ms: u64,
}

impl ScreenTestRender {
    /// The body as text, empty when the render raised an error.
    pub fn text(&self) -> String {
        self.output.as_ref().map(RenderOutput::body_text).unwrap_or_default()
    }

    pub fn status(&self) -> Option<u16> {
        self.output.as_ref().map(|output| output.status)
    }

    /// Java's `assertContains`: the output has `text`.
    pub fn contains(&self, text: &str) -> bool {
        self.output
            .as_ref()
            .is_some_and(|output| output.body_text().contains(text))
    }
}

/// Counts over the renders of one [`ScreenTest`] and its clones, as Java's `renderCount`, `errorCount`, `renderTotalChars`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenTestStats {
    pub render_count: u64,
    pub error_count: u64,
    pub total_chars: u64,
}

/// Text in an output that means the render failed, as Java's `ScreenTestImpl.errorStrings`.
const ERROR_STRINGS: &[&str] = &[
    "[Template Error",
    "FTL stack trace",
    "Could not find subscreen or transition",
];

/// A test render setup: the root screen, kept across renders.
#[derive(Clone)]
pub struct ScreenTest {
    factory: Arc<ExecutionContextFactory>,
    root: Option<String>,
    base_url: String,
    render_mode: Option<String>,
    base_screen_path: String,
    stats: Arc<std::sync::Mutex<ScreenTestStats>>,
    start_time_ms: i64,
}

impl ScreenTest {
    pub fn new(factory: &Arc<ExecutionContextFactory>) -> Self {
        Self {
            factory: Arc::clone(factory),
            root: None,
            base_url: String::new(),
            render_mode: None,
            base_screen_path: String::new(),
            stats: Arc::new(std::sync::Mutex::new(ScreenTestStats::default())),
            start_time_ms: crate::user::now_millis(),
        }
    }

    /// Java's `baseScreenPath`: render paths are under this path of the root screen, such as `rest` or `apps/my`.
    #[must_use]
    pub fn base_screen_path(mut self, path: impl Into<String>) -> Self {
        self.base_screen_path = path.into().trim_matches('/').to_string();
        self
    }

    /// The counts of the renders so far.
    pub fn stats(&self) -> ScreenTestStats {
        *self.stats.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// When this test setup was made, epoch millis.
    pub fn start_time_ms(&self) -> i64 {
        self.start_time_ms
    }

    fn full_path(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if self.base_screen_path.is_empty() {
            path.to_string()
        } else if path.is_empty() {
            self.base_screen_path.clone()
        } else {
            format!("{}/{path}", self.base_screen_path)
        }
    }

    /// Java's `ScreenTest.render(screenPath, parameters, requestMethod)`: render `path` under the base screen path
    /// with `method` (GET when `None`) as the current user. Errors are collected, not returned: one the render
    /// raised, the `ec.message` errors it added (taken off the context, as Java's render runs in its own), and error
    /// text in the output.
    pub fn test_render(
        &self,
        path: &str,
        parameters: BTreeMap<String, Value>,
        method: Option<&str>,
    ) -> ScreenTestRender {
        let started = std::time::Instant::now();
        let mark = crate::context::ExecutionContext::with_current(|ec| ec.message().error_mark());
        let mut render = self.prepared(path, parameters);
        if let Some(method) = method.filter(|method| !method.is_empty()) {
            render = render.method(method.to_ascii_uppercase());
        }
        let mut error_messages = Vec::new();
        let output = match render.render() {
            Ok(output) => Some(output),
            Err(err) => {
                let message = format!("Exception in render of {path}: {err}");
                tracing::warn!("{message}");
                error_messages.push(message);
                None
            }
        };
        let render_time_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if let Some(mark) = mark {
            let added = crate::context::ExecutionContext::with_current(|ec| ec.message().take_errors_since(mark))
                .unwrap_or_default();
            if !added.is_empty() {
                tracing::warn!("Error messages from {path}: {}", added.join("\n"));
            }
            error_messages.extend(added);
        }
        let text = output.as_ref().map(RenderOutput::body_text).unwrap_or_default();
        for error in ERROR_STRINGS {
            if text.contains(error) {
                let message = format!("Found error [{error}] in output from {path}");
                tracing::warn!("{message}");
                error_messages.push(message);
            }
        }
        {
            let mut stats = self.stats.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            stats.render_count += 1;
            stats.error_count += error_messages.len() as u64;
            stats.total_chars += text.chars().count() as u64;
        }
        ScreenTestRender {
            screen_path: path.to_string(),
            output,
            error_messages,
            render_time_ms,
        }
    }

    fn prepared(&self, path: &str, parameters: BTreeMap<String, Value>) -> super::render::ScreenRender {
        let mut render = make_render(&self.factory)
            .screen_path(self.full_path(path))
            .parameters(parameters)
            .base_url(self.base_url.clone());
        if let Some(root) = &self.root {
            render = render.root_screen(root.clone());
        }
        if let Some(mode) = &self.render_mode {
            render = render.render_mode(mode.clone());
        }
        render
    }

    /// Location of the root screen. The default is `webroot.xml`.
    #[must_use]
    pub fn root_screen(mut self, location: impl Into<String>) -> Self {
        self.root = Some(location.into());
        self
    }

    #[must_use]
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// A render mode for every render, so a path with no extension is not rendered as `html`.
    #[must_use]
    pub fn render_mode(mut self, mode: impl Into<String>) -> Self {
        self.render_mode = Some(mode.into());
        self
    }

    /// Render `path` (a `?a=b` tail adds parameters) with `parameters`, in the current execution
    /// context when there is one, so the user and the transaction are the caller's.
    ///
    /// # Errors
    ///
    /// Returns the error the render raised.
    pub fn render(&self, path: &str, parameters: BTreeMap<String, Value>) -> Result<TestRender, Error> {
        Ok(TestRender {
            output: self.prepared(path, parameters).render()?,
        })
    }
}
