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

//! Link arguments only this crate's emscripten module takes: the exports and the preload package.
//! Target-wide flags (memory, stack, module shape) are in `.cargo/config.toml`.

use std::path::PathBuf;

/// `src/exports.rs`, plus `main`. emcc names C symbols with a leading underscore.
const EXPORTS: &[&str] = &[
    "main",
    "moqui_alloc",
    "moqui_dealloc",
    "moqui_request",
    "moqui_init",
    "moqui_call_service",
    "moqui_render_screen",
    "moqui_entity_find",
    "moqui_result_ptr",
    "moqui_result_len",
    "moqui_free_handle",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("emscripten") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let framework = manifest.join("../../framework");
    let runtime = manifest.join("runtime");
    println!("cargo:rerun-if-changed={}", runtime.display());
    let exported: Vec<String> = EXPORTS.iter().map(|name| format!("_{name}")).collect();
    println!("cargo:rustc-link-arg-bins=-sEXPORTED_FUNCTIONS={}", exported.join(","));
    // Read-only framework and the browser runtime conf, in MEMFS at /moqui. The factory writes
    // runtime/.env there; MEMFS is writable, and nothing in it outlives the page.
    println!(
        "cargo:rustc-link-arg-bins=--preload-file={}@/moqui/framework",
        framework.display()
    );
    println!("cargo:rustc-link-arg-bins=--preload-file={}@/moqui/runtime", runtime.display());
}
