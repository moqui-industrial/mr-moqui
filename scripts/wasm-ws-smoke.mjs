// Headless Chrome smoke for the browser WebSocket transport (plans/Wasm.md W6).
//   CARGO_TARGET_DIR=target/browser scripts/wasm-build.sh --release && node scripts/wasm-ws-smoke.mjs target/browser/wasm-pkg/release
// Serves the package and a /notws mock on one origin, loads js/ws-smoke.html, and polls #result.
// The mock speaks /notws: after `subscribe:ALL` it pushes a notification (with the server-side
// `userIds` routing field, which the module must strip) and a binary frame. `echo:<text>` comes
// back as text, binary is echoed, `from-module` is answered with an `Ack` notification, and `drop`
// closes the TCP connection without a close frame. CHROME overrides the binary.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const pkg = path.resolve(process.argv[2] || 'target/browser/wasm-pkg/release');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm' };
const server = createServer(async (req, res) => {
    const file = path.join(pkg, decodeURIComponent(new URL(req.url, 'http://x').pathname));
    try {
        const body = await readFile(file);
        res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
        res.end(body);
    } catch {
        res.writeHead(404).end();
    }
});

// --- A minimal RFC 6455 server: unfragmented frames, client frames masked. ---
function frame(opcode, payload) {
    const length = payload.length;
    const header = length < 126 ? Buffer.from([0x80 | opcode, length])
        : length < 65536 ? Buffer.from([0x80 | opcode, 126, length >> 8, length & 255])
            : (() => { const h = Buffer.alloc(10); h[0] = 0x80 | opcode; h[1] = 127; h.writeBigUInt64BE(BigInt(length), 2); return h; })();
    return Buffer.concat([header, payload]);
}

function parseFrames(buffer, onFrame) {
    let offset = 0;
    while (buffer.length - offset >= 2) {
        const opcode = buffer[offset] & 0x0f;
        let length = buffer[offset + 1] & 0x7f;
        let at = offset + 2;
        if (length === 126) { if (buffer.length < at + 2) break; length = buffer.readUInt16BE(at); at += 2; }
        else if (length === 127) { if (buffer.length < at + 8) break; length = Number(buffer.readBigUInt64BE(at)); at += 8; }
        if (buffer.length < at + 4 + length) break;
        const mask = buffer.subarray(at, at + 4);
        const payload = Buffer.from(buffer.subarray(at + 4, at + 4 + length));
        for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
        onFrame(opcode, payload);
        offset = at + 4 + length;
    }
    return buffer.subarray(offset);
}

let notificationId = 1;
const notification = (topic, title) => JSON.stringify({
    topic, title, notificationMessageId: `MOCK${notificationId++}`, type: 'info', userIds: ['someone'],
});
let connections = 0;
const received = [];

server.on('upgrade', (req, socket) => {
    if (new URL(req.url, 'http://x').pathname !== '/notws') {
        socket.destroy();
        return;
    }
    connections++;
    const accept = createHash('sha1').update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const sendText = (text) => socket.write(frame(1, Buffer.from(text)));
    let pending = Buffer.alloc(0);
    socket.on('data', (chunk) => {
        pending = parseFrames(Buffer.concat([pending, chunk]), (opcode, payload) => {
            if (opcode === 1) {
                const text = payload.toString();
                received.push(text);
                if (text === 'subscribe:ALL') {
                    sendText(notification('Hello', `connection ${connections}`));
                    socket.write(frame(2, Buffer.from([1, 2, 3])));
                } else if (text.startsWith('echo:')) {
                    sendText(text.slice(5));
                } else if (text === 'from-module') {
                    sendText(notification('Ack', 'module frame received'));
                } else if (text === 'drop') {
                    socket.destroy();
                }
            } else if (opcode === 2) {
                received.push(`binary:${payload.length}`);
                socket.write(frame(2, payload));
            } else if (opcode === 8) {
                socket.end(frame(8, payload.subarray(0, 2)));
            } else if (opcode === 9) {
                socket.write(frame(10, payload));
            }
        });
    });
    socket.on('error', () => {});
});

await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const url = `http://127.0.0.1:${server.address().port}/ws-smoke.html`;
const profile = await mkdtemp(path.join(tmpdir(), 'moqui-chrome-ws-'));
const chrome = spawn(process.env.CHROME || 'google-chrome', [
    '--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`,
    '--remote-debugging-port=0', 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
const wsUrl = await new Promise((resolve, reject) => {
    let seen = '';
    chrome.stderr.on('data', (chunk) => {
        seen += chunk;
        const match = seen.match(/DevTools listening on (ws:\/\/\S+)/);
        if (match) resolve(match[1]);
    });
    chrome.on('exit', () => reject(new Error('chrome exited: ' + seen)));
});
const targets = await (await fetch(wsUrl.replace(/^ws/, 'http').replace(/\/devtools\/.*/, '/json'))).json();
const page = targets.find((target) => target.type === 'page');
const devtools = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve) => devtools.addEventListener('open', resolve));
let nextId = 1;
const waiting = new Map();
devtools.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    if (message.id && waiting.has(message.id)) {
        waiting.get(message.id)(message.result);
        waiting.delete(message.id);
    } else if (message.method === 'Runtime.exceptionThrown') {
        console.log('[page]', JSON.stringify(message.params).slice(0, 300));
    }
});
const send = (method, params = {}) => new Promise((resolve) => {
    const id = nextId++;
    waiting.set(id, resolve);
    devtools.send(JSON.stringify({ id, method, params }));
});
await send('Runtime.enable');
await send('Page.navigate', { url });
let result = 'running';
const deadline = Date.now() + 120_000;
while (result === 'running' && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 500));
    const evaluated = await send('Runtime.evaluate', { expression: "document.getElementById('result')?.textContent ?? 'running'" });
    result = evaluated?.result?.value ?? 'running';
}
const log = await send('Runtime.evaluate', { expression: "document.getElementById('log')?.textContent ?? ''" });
console.log(log?.result?.value ?? '');
console.log(`mock: ${connections} connection(s), received ${JSON.stringify(received)}`);
const passed = result === 'WS SMOKE OK' && connections >= 2 && received.includes('from-module') && received.includes('binary:4');
console.log(passed ? 'WS SMOKE OK' : `WS SMOKE FAILED (${result})`);
devtools.close();
const exited = new Promise((resolve) => chrome.on('exit', resolve));
chrome.kill();
await exited;
server.close();
await rm(profile, { recursive: true, force: true, maxRetries: 5 }).catch(() => {});
process.exit(passed ? 0 : 1);
