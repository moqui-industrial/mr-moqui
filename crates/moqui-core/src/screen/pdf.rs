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

//! PDF output for the `html-print` mode. The engine is the host's (`plans/archive/Architecture-Proposal.md`, Tool components
//! and print): one long-lived Chromium. Core has no browser, so the host installs a [`PdfEngine`] at startup. With
//! none installed, or a build without the `pdf` feature, a request for PDF bytes reports that PDF is not available.

use std::sync::{Arc, PoisonError, RwLock};

use crate::context::Error;

/// What a caller sees when it asks for PDF bytes and the host installed no engine.
pub const PDF_UNAVAILABLE: &str = "PDF is not available: no Chromium binary was found (set pdf_chromium_path)";

/// Turns an `html-print` document into PDF bytes. It blocks the calling worker until the bytes come back, so a
/// caller renders the document first and holds no enlisted connection while it waits.
pub trait PdfEngine: Send + Sync {
    /// # Errors
    ///
    /// Returns an error when the browser fails, exits, or the render passes its timeout.
    fn render(&self, html_print_document: &str) -> Result<Vec<u8>, Error>;
}

enum Installed {
    Engine(Arc<dyn PdfEngine>),
    /// Why there is no engine, reported to every caller.
    Unavailable(String),
}

fn slot() -> &'static RwLock<Option<Installed>> {
    static SLOT: RwLock<Option<Installed>> = RwLock::new(None);
    &SLOT
}

/// Use `engine` for every PDF in this process.
pub fn install_pdf_engine(engine: Arc<dyn PdfEngine>) {
    *slot().write().unwrap_or_else(PoisonError::into_inner) = Some(Installed::Engine(engine));
}

/// No engine; every caller gets `reason`, for example that the build left out the `pdf` feature.
pub fn install_pdf_unavailable(reason: impl Into<String>) {
    *slot().write().unwrap_or_else(PoisonError::into_inner) = Some(Installed::Unavailable(reason.into()));
}

/// Remove the installed engine or reason.
pub fn clear_pdf_engine() {
    *slot().write().unwrap_or_else(PoisonError::into_inner) = None;
}

/// The PDF renderer: the installed engine.
pub struct PdfRenderer;

impl PdfRenderer {
    /// Whether an engine is installed.
    #[must_use]
    pub fn available() -> bool {
        matches!(
            *slot().read().unwrap_or_else(PoisonError::into_inner),
            Some(Installed::Engine(_))
        )
    }

    /// PDF bytes for an `html-print` document.
    ///
    /// # Errors
    ///
    /// Returns [`PDF_UNAVAILABLE`] (or the installed reason) when there is no engine, else the engine's error.
    pub fn render(html_print_document: &str) -> Result<Vec<u8>, Error> {
        let engine = match &*slot().read().unwrap_or_else(PoisonError::into_inner) {
            Some(Installed::Engine(engine)) => Arc::clone(engine),
            Some(Installed::Unavailable(reason)) => return Err(Error::new(reason.clone())),
            None => return Err(Error::new(PDF_UNAVAILABLE)),
        };
        engine.render(html_print_document)
    }
}

/// Tests that install an engine hold this, since the slot is process-wide.
#[cfg(test)]
pub(crate) fn pdf_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}
