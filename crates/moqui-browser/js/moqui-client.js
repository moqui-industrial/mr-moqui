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

// UI-thread side: start the worker and turn requests into promises.

export function startMoqui(workerUrl = new URL('./moqui-worker.js', import.meta.url)) {
    const worker = new Worker(workerUrl, { type: 'module' });
    const waiting = new Map();
    let nextId = 1;
    worker.onmessage = (event) => {
        const { id, response } = event.data;
        const settle = waiting.get(id);
        waiting.delete(id);
        if (settle) settle(response);
    };
    return {
        worker,
        /** `{op, ...}`. Resolves to `{ok, result}` or `{ok: false, error}`. */
        request(fields) {
            const id = nextId++;
            return new Promise((resolve) => {
                waiting.set(id, resolve);
                worker.postMessage({ id, fields });
            });
        },
    };
}
