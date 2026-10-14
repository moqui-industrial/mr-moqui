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

//! Outbound HTTP over the browser's `fetch` (`plans/archive/Wasm.md` W5).
//!
//! `moqui_fetch_send` is a JSPI import (`emlib/moqui_fetch.js`, `__async`): the WASM stack suspends
//! while `fetch` runs and resumes with the status, or -1 for a transport failure (network, CORS,
//! abort, timeout). The body or the error text waits in JS until [`take`] copies it into WASM
//! memory. The export that led here must be a JSPI export (`build.rs`), and the worker keeps one call
//! in flight (`js/moqui.js`), so nothing re-enters the module while it is suspended; a second
//! export call while suspended is refused by [`Busy`].

use std::sync::atomic::{AtomicBool, Ordering};

use moqui_core::outbound::{HttpSender, Request, Response};
use moqui_lib::error::Error;
use serde_json::json;

/// Set while an export is running, so a call that arrives while one is suspended fails instead of
/// running on top of its open transaction.
static IN_CALL: AtomicBool = AtomicBool::new(false);

/// Held for one export call. [`Busy::enter`] fails when another export is in progress.
pub struct Busy;

impl Busy {
    /// # Errors
    ///
    /// Returns an error when an export is already running (suspended on `fetch`).
    pub fn enter() -> Result<Self, Error> {
        if IN_CALL.swap(true, Ordering::AcqRel) {
            return Err(Error::new(
                "the module is busy: another call is suspended on fetch; calls must be serialized",
            ));
        }
        Ok(Self)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        IN_CALL.store(false, Ordering::Release);
    }
}

/// Largest response body kept, as on the server client. A larger one is an error.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// The [`HttpSender`] the browser installs at startup.
pub struct FetchSender;

/// Install [`FetchSender`] for every `remote-rest` call and `http` resource.
pub fn install() {
    moqui_core::outbound::install_http_sender(std::sync::Arc::new(FetchSender));
}

impl HttpSender for FetchSender {
    fn send(&self, request: Request) -> Result<Response, Error> {
        let shown = moqui_core::outbound::redacted(&request.url);
        let wire = json!({
            "method": request.method.as_str(),
            "url": request.url,
            "headers": request.headers,
            "body": request.body,
            "maxBytes": MAX_RESPONSE_BYTES,
        })
        .to_string();
        let status = transport::send(wire.as_bytes());
        let text = String::from_utf8_lossy(&transport::take()).into_owned();
        match u16::try_from(status) {
            Ok(status) => Ok(Response { status, body: text }),
            Err(_) => Err(Error::new(format!("HTTP {} {shown} failed: {text}", request.method))),
        }
    }
}

#[cfg(target_os = "emscripten")]
mod transport {
    extern "C" {
        /// JSPI import. Suspends until `fetch` settles; the status, or -1 on a transport failure.
        fn moqui_fetch_send(request: *const u8, len: usize) -> i32;
        /// Bytes waiting from the last send.
        fn moqui_fetch_len() -> usize;
        /// Copy the waiting bytes to `ptr` and drop them.
        fn moqui_fetch_take(ptr: *mut u8);
    }

    pub fn send(request: &[u8]) -> i32 {
        // SAFETY: the import reads `len` bytes at `request` before it suspends.
        unsafe { moqui_fetch_send(request.as_ptr(), request.len()) }
    }

    pub fn take() -> Vec<u8> {
        // SAFETY: the length and the copy come from the same waiting buffer; nothing runs between.
        unsafe {
            let mut bytes = vec![0_u8; moqui_fetch_len()];
            moqui_fetch_take(bytes.as_mut_ptr());
            bytes
        }
    }
}

/// Native builds have no `fetch`; the sender is only installed in the emscripten module.
#[cfg(not(target_os = "emscripten"))]
mod transport {
    pub fn send(_request: &[u8]) -> i32 {
        -1
    }

    pub fn take() -> Vec<u8> {
        b"fetch exists only in the emscripten module".to_vec()
    }
}

#[cfg(test)]
mod tests;
