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

/// A test render setup: the root screen, kept across renders.
#[derive(Clone)]
pub struct ScreenTest {
    factory: Arc<ExecutionContextFactory>,
    root: Option<String>,
    base_url: String,
    render_mode: Option<String>,
}

impl ScreenTest {
    pub fn new(factory: &Arc<ExecutionContextFactory>) -> Self {
        Self {
            factory: Arc::clone(factory),
            root: None,
            base_url: String::new(),
            render_mode: None,
        }
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
        let mut render = make_render(&self.factory)
            .screen_path(path)
            .parameters(parameters)
            .base_url(self.base_url.clone());
        if let Some(root) = &self.root {
            render = render.root_screen(root.clone());
        }
        if let Some(mode) = &self.render_mode {
            render = render.render_mode(mode.clone());
        }
        Ok(TestRender {
            output: render.render()?,
        })
    }
}
