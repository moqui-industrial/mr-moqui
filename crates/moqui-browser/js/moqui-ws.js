// WebSocket transport for the worker (plans/Wasm.md W6). The socket lives in the worker; frames
// never call into WASM from a socket callback. Inbound frames wait in a bounded queue until the
// worker's request queue is free (moqui.exclusive), then enter the module through ordinary exports.
// No call suspends for the network, so this needs neither JSPI nor Asyncify.
//
// Outbound frames are queued in WASM by local code (moqui_browser::ws) and drained after each
// request with drainOutbox(); JavaScript callers use MoquiSocket.send directly.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** Reconnect waits: doubles from `min` to `max`, with up to 25% jitter. */
function backoff(attempt, min, max) {
    const base = Math.min(max, min * 2 ** attempt);
    return base + Math.random() * base * 0.25;
}

/**
 * One named socket that reconnects until close().
 *
 * options:
 * - `onText(text)`, `onBinary(arrayBuffer)`: inbound frames, called from the worker's event loop.
 * - `onState({name, state, attempt, code, reason})`: `connecting`, `open`, `closed`, `stopped`.
 * - `onOpen(socket)`: after each (re)connect, before queued frames are sent; `/notws` subscribes here.
 * - `maxOutbound` (256): frames held while the socket is not open. A full queue refuses a send.
 * - `minDelay` (500 ms), `maxDelay` (30 s): reconnect backoff.
 */
export class MoquiSocket {
    constructor(name, url, options = {}) {
        this.name = name;
        this.url = url;
        this.options = options;
        this.maxOutbound = options.maxOutbound ?? 256;
        this.minDelay = options.minDelay ?? 500;
        this.maxDelay = options.maxDelay ?? 30_000;
        this.outbound = [];
        this.attempt = 0;
        this.stopped = false;
        this.timer = null;
        this.socket = null;
        this.connect();
    }

    connect() {
        if (this.stopped) return;
        this.state('connecting');
        let socket;
        try {
            socket = new WebSocket(this.url, this.options.protocols);
        } catch (err) {
            this.retry(1006, String(err));
            return;
        }
        socket.binaryType = 'arraybuffer';
        this.socket = socket;
        socket.onopen = () => {
            this.attempt = 0;
            this.state('open');
            this.options.onOpen?.(this);
            const waiting = this.outbound;
            this.outbound = [];
            for (const data of waiting) socket.send(data);
        };
        socket.onmessage = (event) => {
            if (typeof event.data === 'string') this.options.onText?.(event.data);
            else this.options.onBinary?.(event.data);
        };
        socket.onclose = (event) => {
            if (this.socket === socket) this.socket = null;
            this.retry(event.code, event.reason);
        };
        // An error is followed by close, which schedules the retry.
        socket.onerror = () => {};
    }

    retry(code, reason) {
        if (this.stopped) {
            this.state('stopped', { code, reason });
            return;
        }
        const wait = backoff(this.attempt++, this.minDelay, this.maxDelay);
        this.state('closed', { code, reason, retryIn: Math.round(wait) });
        this.timer = setTimeout(() => {
            this.timer = null;
            this.connect();
        }, wait);
    }

    state(state, extra = {}) {
        this.options.onState?.({ name: this.name, state, attempt: this.attempt, ...extra });
    }

    get open() {
        return this.socket?.readyState === WebSocket.OPEN;
    }

    /** Send text, an ArrayBuffer, or a typed array. Returns false when the queue is full. */
    send(data) {
        if (this.open) {
            this.socket.send(data);
            return true;
        }
        if (this.outbound.length >= this.maxOutbound) return false;
        this.outbound.push(data);
        return true;
    }

    /** Stop reconnecting and close. Queued outbound frames are dropped. */
    close(code = 1000, reason = '') {
        this.stopped = true;
        this.outbound = [];
        if (this.timer) clearTimeout(this.timer);
        this.timer = null;
        if (this.socket) this.socket.close(code, reason);
        else this.state('stopped', { code, reason });
    }
}

/**
 * Inbound frames waiting for the module, bounded. Past `max` the oldest frame is dropped and
 * counted: a `/notws` notification that is dropped is still in the user's list on the server.
 */
export class InboundQueue {
    constructor(max = 1024) {
        this.max = max;
        this.frames = [];
        this.dropped = 0;
    }

    push(frame) {
        this.frames.push(frame);
        while (this.frames.length > this.max) {
            this.frames.shift();
            this.dropped++;
        }
    }

    take() {
        const frames = this.frames;
        this.frames = [];
        return frames;
    }
}

/** Copy `bytes` into module memory, call `fn(ptr, len)`, and free it. */
function withBytes(module, bytes, fn) {
    const ptr = module._moqui_alloc(bytes.length);
    module.HEAPU8.set(bytes, ptr);
    try {
        return fn(ptr, bytes.length);
    } finally {
        module._moqui_dealloc(ptr, bytes.length);
    }
}

/** Read and free one result handle as bytes. A fresh ArrayBuffer, so it can be transferred. */
function takeResult(module, handle) {
    try {
        const at = module._moqui_result_ptr(handle);
        const len = module._moqui_result_len(handle);
        return module.HEAPU8.slice(at, at + len);
    } finally {
        module._moqui_free_handle(handle);
    }
}

/**
 * Keep one inbound binary frame in the module. Returns the frame handle
 * (moqui_browser::ws::take_binary). Call inside the request queue.
 */
export function deliverBinary(module, conn, arrayBuffer) {
    const name = encoder.encode(conn);
    return withBytes(module, name, (namePtr, nameLen) =>
        withBytes(module, new Uint8Array(arrayBuffer), (ptr, len) => module._moqui_ws_binary(namePtr, nameLen, ptr, len)));
}

/**
 * Take the outbound frames local code queued in the module: `[{conn, text}]` or
 * `[{conn, bytes: Uint8Array}]`. Call inside the request queue.
 */
export function drainOutbox(module) {
    if (module._moqui_ws_outbox_len() === 0) return [];
    const frames = JSON.parse(decoder.decode(takeResult(module, module._moqui_ws_drain())));
    return frames.map((frame) =>
        frame.binary === undefined ? frame : { conn: frame.conn, bytes: takeResult(module, frame.binary) });
}

/** `ws:`/`wss:` URL for `url`, which may be relative to the worker or use http(s). */
function socketUrl(url) {
    const resolved = new URL(url, self.location.href);
    if (resolved.protocol === 'http:') resolved.protocol = 'ws:';
    if (resolved.protocol === 'https:') resolved.protocol = 'wss:';
    return resolved.href;
}

/**
 * The worker's sockets and the pump that feeds their frames into the module.
 *
 * Worker ops (not module ops): `socketConnect {name = 'notws', url, subscribe}`,
 * `socketSend {name, text | bytes}`, `socketClose {name}`, `socketStats`. With `subscribe` (a topic
 * list, or 'ALL'), each open sends `/notws`'s `subscribe:` command.
 *
 * Events posted to the UI thread: `{event: 'notification', name, body}` after the notification was
 * published in the module, `{event: 'text', name, text}`, `{event: 'binary', name, handle, length}`
 * (the module keeps the bytes), and `{event: 'socket', name, state, ...}`.
 */
export class WorkerSockets {
    constructor(ready, post, maxInbound = 1024) {
        this.ready = ready;
        this.post = post;
        this.sockets = new Map();
        this.inbound = new InboundQueue(maxInbound);
        this.pumping = false;
        this.sent = 0;
        this.received = 0;
    }

    handles(op) {
        return op === 'socketConnect' || op === 'socketSend' || op === 'socketClose' || op === 'socketStats';
    }

    async op(fields) {
        const name = fields.name || 'notws';
        if (fields.op === 'socketConnect') {
            this.sockets.get(name)?.close();
            const subscribe = fields.subscribe;
            const topics = Array.isArray(subscribe) ? subscribe.join(',') : subscribe;
            const socket = new MoquiSocket(name, socketUrl(fields.url), {
                onText: (text) => this.receive({ name, text }),
                onBinary: (buffer) => this.receive({ name, buffer }),
                onState: (state) => this.post({ event: 'socket', ...state }),
                onOpen: topics ? (open) => open.send(`subscribe:${topics}`) : undefined,
            });
            this.sockets.set(name, socket);
            return { ok: true, result: null };
        }
        if (fields.op === 'socketSend') {
            const socket = this.sockets.get(name);
            if (!socket) return { ok: false, error: `no socket ${name}` };
            const queued = socket.send(fields.text ?? fields.bytes);
            if (queued) this.sent++;
            return queued ? { ok: true, result: null } : { ok: false, error: `socket ${name} queue is full` };
        }
        if (fields.op === 'socketClose') {
            this.sockets.get(name)?.close();
            this.sockets.delete(name);
            return { ok: true, result: null };
        }
        const open = [...this.sockets.values()].filter((socket) => socket.open).map((socket) => socket.name);
        return {
            ok: true,
            result: { open, sent: this.sent, received: this.received, dropped: this.inbound.dropped },
        };
    }

    receive(frame) {
        this.received++;
        this.inbound.push(frame);
        this.schedule();
    }

    /** One pump waits in the request queue at a time; it takes every frame queued by then. */
    schedule() {
        if (this.pumping) return;
        this.pumping = true;
        this.ready
            .then((moqui) => moqui.exclusive(() => this.pump(moqui)))
            .catch((err) => console.warn(`websocket pump failed: ${err}`))
            .finally(() => {
                this.pumping = false;
                if (this.inbound.frames.length) this.schedule();
            });
    }

    /** Deliver queued inbound frames, then send what the module queued. Runs inside the queue.
     * `callNow` is async: `moqui_request` suspends under JSPI. The ws exports stay synchronous. */
    async pump(moqui) {
        for (const frame of this.inbound.take()) {
            if (frame.text !== undefined) {
                const response = await moqui.callNow('moqui_request', { op: 'wsDeliver', conn: frame.name, text: frame.text });
                if (!response.ok) {
                    console.warn(`websocket ${frame.name} frame not delivered: ${response.error}`);
                } else if (response.result.kind === 'notification') {
                    this.post({ event: 'notification', name: frame.name, body: response.result.body });
                } else {
                    this.post({ event: 'text', name: frame.name, text: frame.text });
                }
            } else {
                const handle = deliverBinary(moqui.module, frame.name, frame.buffer);
                this.post({ event: 'binary', name: frame.name, handle, length: frame.buffer.byteLength });
            }
        }
        this.flushNow(moqui);
    }

    /** Send what local code queued in the module. Runs inside the queue. */
    flushNow(moqui) {
        for (const frame of drainOutbox(moqui.module)) {
            const socket = this.sockets.get(frame.conn);
            if (!socket) {
                console.warn(`websocket frame for ${frame.conn} dropped: no such socket`);
                continue;
            }
            if (socket.send(frame.text ?? frame.bytes)) this.sent++;
            else console.warn(`websocket frame for ${frame.conn} dropped: queue full`);
        }
    }

    /** After each request: send what it queued. */
    async flush() {
        const moqui = await this.ready;
        await moqui.exclusive(() => this.flushNow(moqui));
    }
}
