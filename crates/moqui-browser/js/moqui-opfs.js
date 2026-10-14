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

// OPFS persistence through the WasmFS OPFS backend (plans/Wasm.md W5).
//
// mountOpfs() mounts the Origin Private File System at /opfs (crates/moqui-browser/src/exports.rs
// moqui_mount_opfs). A SQLite file under it is durable per commit: every file operation there is a
// JSPI import that suspends on the OPFS sync access handle, so a commit's fsync is on disk when it
// returns. No flush step and no snapshot.
//
// Worker only (sync access handles exist only in dedicated workers), and only through the request
// queue (moqui.exclusive): the mount suspends. Do not use the emscripten FS JS API on /opfs: those
// calls are not JSPI exports and would trap at the first suspension.

export const OPFS_ROOT = '/opfs';

const encoder = new TextEncoder();
let mounted = false;

/** Mount OPFS at OPFS_ROOT once. `module` is the emscripten module. */
export async function mountOpfs(module) {
    if (mounted) return;
    if (!navigator.storage || !navigator.storage.getDirectory) {
        throw new Error('this browser has no Origin Private File System');
    }
    const bytes = encoder.encode(OPFS_ROOT);
    const ptr = module._moqui_alloc(bytes.length);
    module.HEAPU8.set(bytes, ptr);
    let status;
    try {
        status = await module._moqui_mount_opfs(ptr, bytes.length);
    } finally {
        module._moqui_dealloc(ptr, bytes.length);
    }
    // -17 is EEXIST: already mounted.
    if (status !== 0 && status !== -17) throw new Error(`could not mount OPFS at ${OPFS_ROOT} (errno ${-status})`);
    mounted = true;
}

/** The database path for a persisted file name. */
export function persistedPath(name) {
    if (!/^[A-Za-z0-9._-]+$/.test(name)) throw new Error(`persist name ${name} must be a plain file name`);
    return `${OPFS_ROOT}/${name}`;
}
