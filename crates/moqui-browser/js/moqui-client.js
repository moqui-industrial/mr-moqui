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

// UI-thread side: start the worker and turn requests into promises. The WebSocket transport
// (moqui-ws.js) runs in the worker; connect/send/close drive it and on* receive its events.

export function startMoqui(workerUrl = new URL('./moqui-worker.js', import.meta.url)) {
    const worker = new Worker(workerUrl, { type: 'module' });
    const waiting = new Map();
    const listeners = { notification: new Set(), text: new Set(), binary: new Set(), socket: new Set() };
    let nextId = 1;
    worker.onmessage = (event) => {
        const { id, response, event: kind } = event.data;
        if (kind) {
            for (const listener of listeners[kind] ?? []) listener(event.data);
            return;
        }
        const settle = waiting.get(id);
        waiting.delete(id);
        if (settle) settle(response);
    };
    function request(fields, transfer = []) {
        const id = nextId++;
        return new Promise((resolve) => {
            waiting.set(id, resolve);
            worker.postMessage({ id, fields }, transfer);
        });
    }
    function on(kind, listener) {
        listeners[kind].add(listener);
        return () => listeners[kind].delete(listener);
    }
    return {
        worker,
        /** `{op, ...}`. Resolves to `{ok, result}` or `{ok: false, error}`. */
        request,
        /**
         * Open (or reopen) the socket `name`, default `notws`. `subscribe` is a topic list or 'ALL'
         * and is sent on every (re)connect, as `/notws` expects. It reconnects until close().
         */
        connect(url, { name = 'notws', subscribe } = {}) {
            return request({ op: 'socketConnect', name, url, subscribe });
        },
        /** Send text or bytes. An ArrayBuffer is transferred, so the caller loses it. */
        send(data, name = 'notws') {
            if (typeof data === 'string') return request({ op: 'socketSend', name, text: data });
            const bytes = data instanceof ArrayBuffer ? data : data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
            return request({ op: 'socketSend', name, bytes }, [bytes]);
        },
        close(name = 'notws') {
            return request({ op: 'socketClose', name });
        },
        /** `{name, body}` after the module published it locally. Returns an unsubscribe function. */
        onNotification: (listener) => on('notification', listener),
        /** Non-notification text frames: `{name, text}`. */
        onText: (listener) => on('text', listener),
        /** Binary frames, kept in the module: `{name, handle, length}`. */
        onBinary: (listener) => on('binary', listener),
        /** Socket state: `{name, state, attempt, code, reason, retryIn}`. */
        onSocket: (listener) => on('socket', listener),
    };
}
