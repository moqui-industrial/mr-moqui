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

// Binding over the emscripten module (crates/moqui-browser/src/exports.rs). One request at a
// time: calls queue here, so a later call never enters the module while an earlier one runs.
// The request exports are JSPI exports (crates/moqui-browser/build.rs): each returns a Promise and
// may suspend on fetch or an OPFS file operation; the queue keeps the module from being re-entered
// while one is suspended (the module also refuses a second call, src/fetch.rs Busy).

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * @param {(options?: object) => Promise<any>} createModule the `createMoqui` factory from moqui_browser.mjs
 * @param {object} [moduleOptions] emscripten Module overrides (print, printErr, locateFile)
 */
export async function loadMoqui(createModule, moduleOptions = {}) {
    const module = await createModule(moduleOptions);
    let tail = Promise.resolve();

    async function callNow(exportName, fields) {
        const bytes = encoder.encode(fields === undefined ? '' : JSON.stringify(fields));
        const ptr = module[`_moqui_alloc`](bytes.length);
        // Read HEAPU8 after every call into WASM: memory growth replaces the buffer.
        module.HEAPU8.set(bytes, ptr);
        let handle;
        try {
            handle = await module[`_${exportName}`](ptr, bytes.length);
        } finally {
            module[`_moqui_dealloc`](ptr, bytes.length);
        }
        try {
            const at = module[`_moqui_result_ptr`](handle);
            const len = module[`_moqui_result_len`](handle);
            const text = decoder.decode(module.HEAPU8.slice(at, at + len));
            return JSON.parse(text);
        } finally {
            module[`_moqui_free_handle`](handle);
        }
    }

    function enqueue(exportName, fields) {
        const next = tail.then(() => callNow(exportName, fields));
        tail = next.catch(() => {});
        return next;
    }

    return {
        module,
        /** Call an export now; resolves to the response. Only inside `exclusive`: anywhere else it
         * could enter a running call. */
        callNow,
        /** Run `fn` (sync or async) in the request queue, so no module call is in progress. */
        exclusive: (fn) => {
            const next = tail.then(fn);
            tail = next.catch(() => {});
            return next;
        },
        /** Any op: `{op: 'find', entity: ...}`. Resolves to `{ok, result}` or `{ok: false, error}`. */
        request: (fields) => enqueue('moqui_request', fields),
        init: (fields) => enqueue('moqui_init', fields),
        callService: (fields) => enqueue('moqui_call_service', fields),
        render: (fields) => enqueue('moqui_render_screen', fields),
        find: (fields) => enqueue('moqui_entity_find', fields),
    };
}
