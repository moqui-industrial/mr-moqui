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

//! The emscripten module. `js/moqui-worker.js` loads it in a Web Worker.
//!
//! Every export takes a JSON request in WASM memory and returns a result handle. The caller reads
//! the response bytes at [`moqui_result_ptr`] for [`moqui_result_len`] bytes, then frees the handle.
//! Input buffers come from [`moqui_alloc`] and go back through [`moqui_dealloc`].

#[cfg(target_os = "emscripten")]
mod exports;

/// The module stays loaded after `main` returns (no EXIT_RUNTIME); the exports do the work.
#[cfg(target_os = "emscripten")]
fn main() {}

#[cfg(not(target_os = "emscripten"))]
fn main() {
    eprintln!("moqui-browser is a wasm32-unknown-emscripten module; the native build is a stub.");
    eprintln!("Build it with: . scripts/wasm-env.sh && cargo build -p moqui-browser --target wasm32-unknown-emscripten");
}
