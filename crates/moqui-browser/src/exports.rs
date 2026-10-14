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

//! `extern "C"` exports. The names are listed in `build.rs` for `-sEXPORTED_FUNCTIONS`.

use std::cell::RefCell;
use std::collections::BTreeMap;

use moqui_browser::api::{handle, handle_op};

thread_local! {
    static RESULTS: RefCell<Results> = RefCell::new(Results::default());
}

/// Response buffers by handle. Handle 0 is never issued.
#[derive(Default)]
struct Results {
    next: u32,
    held: BTreeMap<u32, Vec<u8>>,
}

fn hold(response: String) -> u32 {
    RESULTS.with(|results| {
        let mut results = results.borrow_mut();
        results.next = results.next.wrapping_add(1).max(1);
        let handle = results.next;
        results.held.insert(handle, response.into_bytes());
        handle
    })
}

/// The request bytes at `ptr`. Invalid UTF-8 becomes a request that fails to parse.
///
/// # Safety
///
/// `ptr` must point at `len` readable bytes, or `len` must be 0.
unsafe fn request<'a>(ptr: *const u8, len: usize) -> std::borrow::Cow<'a, str> {
    if len == 0 || ptr.is_null() {
        return std::borrow::Cow::Borrowed("");
    }
    // SAFETY: the caller wrote `len` bytes at `ptr` from moqui_alloc.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf8_lossy(bytes)
}

/// A buffer of `len` bytes for a request. Free it with [`moqui_dealloc`].
#[no_mangle]
pub extern "C" fn moqui_alloc(len: usize) -> *mut u8 {
    let mut buffer = vec![0_u8; len.max(1)].into_boxed_slice();
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Free a buffer from [`moqui_alloc`].
///
/// # Safety
///
/// `ptr` and `len` must be exactly what [`moqui_alloc`] returned and was given.
#[no_mangle]
pub unsafe extern "C" fn moqui_dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr came from moqui_alloc(len), which leaked a boxed slice of len.max(1) bytes.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len.max(1))) });
}

/// Any request, with `op` in the JSON.
///
/// # Safety
///
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn moqui_request(ptr: *const u8, len: usize) -> u32 {
    // SAFETY: forwarded from the caller.
    hold(handle(&unsafe { request(ptr, len) }))
}

macro_rules! op_export {
    ($name:ident, $op:literal) => {
        #[doc = concat!("The `", $op, "` request. See `moqui_browser::api`.")]
        ///
        /// # Safety
        ///
        /// `ptr` must point at `len` readable bytes.
        #[no_mangle]
        pub unsafe extern "C" fn $name(ptr: *const u8, len: usize) -> u32 {
            // SAFETY: forwarded from the caller.
            hold(handle_op($op, &unsafe { request(ptr, len) }))
        }
    };
}

op_export!(moqui_init, "init");
op_export!(moqui_call_service, "callService");
op_export!(moqui_render_screen, "render");
op_export!(moqui_entity_find, "find");

/// Where the response bytes for `handle` start, or null for an unknown handle.
#[no_mangle]
pub extern "C" fn moqui_result_ptr(handle: u32) -> *const u8 {
    RESULTS.with(|results| {
        results
            .borrow()
            .held
            .get(&handle)
            .map_or(std::ptr::null(), |bytes| bytes.as_ptr())
    })
}

/// How many response bytes `handle` holds.
#[no_mangle]
pub extern "C" fn moqui_result_len(handle: u32) -> usize {
    RESULTS.with(|results| results.borrow().held.get(&handle).map_or(0, Vec::len))
}

/// Release the response for `handle`.
#[no_mangle]
pub extern "C" fn moqui_free_handle(handle: u32) {
    RESULTS.with(|results| {
        results.borrow_mut().held.remove(&handle);
    });
}
