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

// Dedicated Worker that owns the module. The UI thread never runs framework code.
// Message in: {id, fields} where fields has `op`. Message out: {id, response}.
// `init` with `persist: "<opfs name>"` mounts OPFS and puts the database at /opfs/<name>, durable
// per commit (moqui-opfs.js); `db` is then ignored. `flush` is accepted and does nothing.
// `cancel` aborts the fetch the running call is suspended on, without waiting in the queue; that
// call fails and its transaction rolls back. JSPI is required: an old browser gets a clear error
// instead of a module that cannot load. The `socket*` ops and the
// unsolicited `{event, ...}` messages are the WebSocket transport (moqui-ws.js).
// Serve moqui_browser.mjs, .wasm, and .data next to this file (scripts/wasm-build.sh stages them).

import createMoqui from './moqui_browser.mjs';
import { loadMoqui } from './moqui.js';
import { mountOpfs, persistedPath } from './moqui-opfs.js';
import { WorkerSockets } from './moqui-ws.js';

let persisted = null;

const ready = !('Suspending' in WebAssembly)
    ? Promise.reject(new Error('this browser is too old: Moqui needs WebAssembly JavaScript Promise Integration (JSPI): Chrome or Edge 137, Firefox 153, Safari 27, or later'))
    : loadMoqui(createMoqui, {
        print: (text) => console.log(text),
        printErr: (text) => console.warn(text),
    });
const sockets = new WorkerSockets(ready, (message) => self.postMessage(message));

self.onmessage = async (event) => {
    const { id, fields } = event.data;
    let response;
    try {
        const moqui = await ready;
        if (fields.op === 'cancel') {
            moqui.module.moquiAbortFetch?.(new Error(fields.reason || 'cancelled'));
            response = { ok: true, result: null };
        } else if (fields.op === 'flush') {
            // Commits are already on OPFS.
            if (!persisted) throw new Error('init did not name a persist file');
            response = { ok: true, result: persisted };
        } else if (sockets.handles(fields.op)) {
            response = await sockets.op(fields);
        } else {
            let request = fields;
            if (fields.op === 'init' && fields.persist) {
                persisted = persistedPath(fields.persist);
                await moqui.exclusive(() => mountOpfs(moqui.module));
                request = { ...fields, db: persisted };
            }
            response = await moqui.request(request);
            // Frames the request queued for a socket (moqui_browser::ws).
            await sockets.flush();
        }
    } catch (err) {
        response = { ok: false, error: String(err && err.stack || err) };
    }
    self.postMessage({ id, response });
};
