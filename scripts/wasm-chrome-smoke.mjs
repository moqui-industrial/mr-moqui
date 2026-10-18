// Headless Chrome smoke for the browser module: js/smoke.html runs the module in a Web Worker.
//   CARGO_TARGET_DIR=target/browser scripts/wasm-build.sh && node scripts/wasm-chrome-smoke.mjs target/browser/wasm-pkg/debug
// Serves the package, starts Chrome with a DevTools port, loads the page twice (the second load
// must find the row the first committed to OPFS, with no flush), and polls #result. Then, with that
// tab still open, a second tab of the page runs too and the result is reported: without pthreads the
// WasmFS OPFS backend uses its async access handle, not the exclusive sync one, so nothing keeps a
// second tab from opening and writing the same database (plans/Remaining-Work.md). CHROME overrides
// the binary. The server also answers the page's remote-rest calls: /echo (JSON), /fail (500), and
// /slow (answers after 20 s, so the page can cancel it).
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const pkg = path.resolve(process.argv[2] || 'target/wasm-pkg/debug');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm' };
const server = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://x');
    if (url.pathname === '/echo') {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ echo: 'ok', method: req.method, note: url.searchParams.get('note') }));
        return;
    }
    if (url.pathname === '/fail') {
        res.writeHead(500, { 'content-type': 'text/plain' }).end('no');
        return;
    }
    if (url.pathname === '/slow') {
        const timer = setTimeout(() => res.writeHead(200).end('{}'), 20_000);
        req.on('close', () => clearTimeout(timer));
        return;
    }
    const file = path.join(pkg, decodeURIComponent(url.pathname));
    try {
        const body = await readFile(file);
        res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
        res.end(body);
    } catch {
        res.writeHead(404).end();
    }
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const url = `http://127.0.0.1:${server.address().port}/smoke.html`;
const profile = await mkdtemp(path.join(tmpdir(), 'moqui-chrome-'));
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
const listUrl = wsUrl.replace(/^ws/, 'http').replace(/\/devtools\/.*/, '/json');
// A DevTools session on one page: send a command and get its result.
async function attach(debuggerUrl) {
    const socket = new WebSocket(debuggerUrl);
    await new Promise((resolve) => socket.addEventListener('open', resolve));
    let nextId = 1;
    const pending = new Map();
    socket.addEventListener('message', (event) => {
        const message = JSON.parse(event.data);
        if (message.id && pending.has(message.id)) {
            pending.get(message.id)(message.result);
            pending.delete(message.id);
        } else if (message.method === 'Runtime.consoleAPICalled' || message.method === 'Runtime.exceptionThrown') {
            console.log('[page]', JSON.stringify(message.params).slice(0, 300));
        }
    });
    const send = (method, params = {}) => new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, resolve);
        socket.send(JSON.stringify({ id, method, params }));
    });
    await send('Runtime.enable');
    return { socket, send };
}
const page = (await (await fetch(listUrl)).json()).find((target) => target.type === 'page');
const { socket, send } = await attach(page.webSocketDebuggerUrl);
async function run(send) {
    await send('Page.navigate', { url });
    let result = 'running';
    const deadline = Date.now() + 120_000;
    while (result === 'running' && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 500));
        const evaluated = await send('Runtime.evaluate', {
            expression: "document.getElementById('result')?.textContent ?? 'running'",
        });
        result = evaluated?.result?.value ?? 'running';
    }
    const log = await send('Runtime.evaluate', { expression: "document.getElementById('log')?.textContent ?? ''" });
    return { result, log: log?.result?.value ?? '' };
}
// Two loads in one profile: the second must find the row the first committed to OPFS.
const first = await run(send);
console.log(first.log + first.result);
const second = first.result === 'SMOKE OK' ? await run(send) : { result: 'skipped', log: '' };
console.log(second.log + second.result);
// A second tab while the first still holds the database: its init fails, and says so.
let other = { result: 'skipped', log: '' };
if (second.result === 'SMOKE OK') {
    const opened = await (await fetch(`${listUrl}/new?about:blank`, { method: 'PUT' })).json();
    const tab = await attach(opened.webSocketDebuggerUrl);
    other = await run(tab.send);
    tab.socket.close();
    console.log('[second tab] ' + other.log + other.result);
}
if (other.result === 'SMOKE OK') {
    console.log('WARN a second tab opened and wrote the database the first tab has open: there is no lock between tabs');
} else if (other.result !== 'skipped') {
    console.log(`second tab: ${other.result}`);
}
const passed = first.log.includes('jspi true') && first.log.includes('restored 0 row(s) from OPFS')
    && second.result === 'SMOKE OK' && second.log.includes('restored 1 row(s) from OPFS');
console.log(passed ? 'CHROME SMOKE OK (JSPI fetch, rollback, cancel, OPFS durable across reload)' : 'CHROME SMOKE FAILED');
socket.close();
const exited = new Promise((resolve) => chrome.on('exit', resolve));
chrome.kill();
await exited;
server.close();
await rm(profile, { recursive: true, force: true, maxRetries: 5 }).catch(() => {});
process.exit(passed ? 0 : 1);
