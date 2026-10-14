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
// Two ops are handled here: `init` with `persist: "<opfs name>"` restores that OPFS file to `db`
// before booting, and `flush` writes `db` back to OPFS (moqui-opfs.js).
// Serve moqui_browser.mjs, .wasm, and .data next to this file (scripts/wasm-build.sh stages them).

import createMoqui from './moqui_browser.mjs';
import { loadMoqui } from './moqui.js';
import { flush, restore } from './moqui-opfs.js';

let persisted = null;

const ready = loadMoqui(createMoqui, {
    print: (text) => console.log(text),
    printErr: (text) => console.warn(text),
});

self.onmessage = async (event) => {
    const { id, fields } = event.data;
    let response;
    try {
        const moqui = await ready;
        if (fields.op === 'flush') {
            if (!persisted) throw new Error('init did not name a persist file');
            const { name, path } = persisted;
            const bytes = await moqui.exclusive(() => flush(moqui.module.FS, name, path));
            response = { ok: true, result: bytes };
        } else {
            if (fields.op === 'init' && fields.persist) {
                if (!fields.db || fields.db === ':memory:') throw new Error('persist needs a db path');
                persisted = { name: fields.persist, path: fields.db };
                await moqui.exclusive(() => restore(moqui.module.FS, persisted.name, persisted.path));
            }
            response = await moqui.request(fields);
        }
    } catch (err) {
        response = { ok: false, error: String(err && err.stack || err) };
    }
    self.postMessage({ id, response });
};
