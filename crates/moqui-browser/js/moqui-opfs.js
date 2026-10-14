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

// Snapshot persistence of the SQLite file between MEMFS and OPFS (plans/Wasm.md).
//
// SQLite runs on a MEMFS file. restore() copies the OPFS copy in before init; flush() writes the
// whole file back. A flush runs between requests, when no transaction is open, so the file is a
// committed state (rollback journal mode: the journal is gone after each commit). Anything after
// the last flush is lost on reload. Per-commit durability needs a SQLite VFS over
// FileSystemSyncAccessHandle (the SQLite WASM opfs-sahpool design) or the WasmFS OPFS backend,
// which needs pthreads or JSPI.
//
// Worker only: createSyncAccessHandle exists only in dedicated workers.

async function directory() {
    return navigator.storage.getDirectory();
}

/** Copy OPFS `name` to MEMFS `path`. Returns the byte count, 0 when there is no copy yet. */
export async function restore(FS, name, path) {
    const root = await directory();
    let handle;
    try {
        handle = await root.getFileHandle(name);
    } catch (err) {
        if (err && err.name === 'NotFoundError') return 0;
        throw err;
    }
    const bytes = new Uint8Array(await (await handle.getFile()).arrayBuffer());
    const parent = path.substring(0, path.lastIndexOf('/'));
    if (parent) FS.mkdirTree(parent);
    FS.writeFile(path, bytes);
    return bytes.length;
}

/** Write MEMFS `path` to OPFS `name`: a temp file first, then the name, so a failed write leaves the old copy. */
export async function flush(FS, name, path) {
    const bytes = FS.readFile(path);
    const root = await directory();
    await writeAll(root, `${name}.tmp`, bytes);
    const temp = await root.getFileHandle(`${name}.tmp`);
    if (typeof temp.move === 'function') {
        await temp.move(name);
    } else {
        await writeAll(root, name, bytes);
        await root.removeEntry(`${name}.tmp`);
    }
    return bytes.length;
}

async function writeAll(root, name, bytes) {
    const handle = await root.getFileHandle(name, { create: true });
    const access = await handle.createSyncAccessHandle();
    try {
        access.truncate(0);
        let at = 0;
        while (at < bytes.length) {
            at += access.write(bytes.subarray(at), { at });
        }
        access.flush();
    } finally {
        access.close();
    }
}
