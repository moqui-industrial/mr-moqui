/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// node --test keys-browser.test.js (in headless Chromium, which chromium-for-tests.js finds; skipped without one)
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const { browser, needsBrowser } = require('./chromium-for-tests');

// The page runs `script` after moqui-html.js has bound the fixture, and the test reads what it writes to #result.
const run = (body, script) => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-keys-'));
    const web = path.join(__dirname, '..');
    const read = (relative) => fs.readFileSync(path.join(web, relative), 'utf8').replace(/<\/script/gi, '<\\/script');
    try {
        const file = path.join(directory, 'keys.html');
        fs.writeFileSync(file, `<!doctype html><html data-theme="light"><style>${read('css/moqui-html.css')}${read('libs/tom-select/tom-select.css')}</style>
            ${body}<pre id="result"></pre>
            <script>${read('libs/tom-select/tom-select.complete.min.js')}</script><script>${read('js/moqui-html.js')}</script><script>
            const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
            const key = (node, name, extra) => { const event = new KeyboardEvent('keydown', Object.assign({ key: name, bubbles: true, cancelable: true,
                keyCode: { Enter: 13, Tab: 9, ArrowUp: 38, ArrowDown: 40 }[name] || name.toUpperCase().charCodeAt(0) }, extra || {})); node.dispatchEvent(event); return event; };
            const type = (node, text) => { node.focus(); node.value = text; node.dispatchEvent(new Event('input', { bubbles: true })); };
            (async () => { try { await sleep(200); ${script} } catch (error) { document.getElementById('result').textContent = JSON.stringify({ error: error.stack }); } })();
            </script>`);
        const dom = execFileSync(browser, ['--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--virtual-time-budget=10000',
            '--window-size=1200,900', '--dump-dom', file], { encoding: 'utf8', timeout: 30000, maxBuffer: 8 * 1024 * 1024, stdio: ['ignore', 'pipe', 'ignore'] });
        return JSON.parse(dom.match(/<pre id="result">(.*?)<\/pre>/s)[1]);
    } finally {
        fs.rmSync(directory, { recursive: true, force: true });
    }
};

const states = '<option value=""></option><option value="AL">Alabama</option><option value="AK">Alaska</option><option value="AZ">Arizona</option>';

test('a Tom Select drop-down takes the typed match on a quick ENTER or TAB, and TAB moves on', needsBrowser, () => {
    const result = run(`<form id="form"><select id="state" name="state" data-ts="select" data-empty>${states}</select>
        <select id="state2" name="state2" data-ts="select" data-empty>${states}</select><input id="next" name="next"></form>`, `
        const one = document.getElementById('state').tomselect.control_input;
        type(one, 'Ariz');
        const enter = key(one, 'Enter');
        await sleep(1000);
        const afterEnter = document.getElementById('state').value;
        const two = document.getElementById('state2').tomselect.control_input;
        type(two, 'Alas');
        key(two, 'Tab');
        await sleep(1000);
        document.getElementById('result').textContent = JSON.stringify({ afterEnter, enterHeld: enter.defaultPrevented,
            afterTab: document.getElementById('state2').value, focus: document.activeElement.id });`);
    assert.deepEqual(result, { afterEnter: 'AZ', enterHeld: true, afterTab: 'AK', focus: 'next' });
});

test('a drop-down with the style submit-on-close submits its form', needsBrowser, () => {
    const result = run(`<form id="form"><select id="ts" name="ts" class="submit-on-close" data-ts="select" data-empty>${states}</select>
        <select id="plain" name="plain" class="submit-on-close">${states}</select></form>`, `
        let submits = [];
        document.getElementById('form').addEventListener('submit', (event) => { event.preventDefault(); submits.push(new FormData(event.target).get('ts') + '/' + new FormData(event.target).get('plain')); });
        const input = document.getElementById('ts').tomselect.control_input;
        type(input, 'Alab');
        key(input, 'Enter');
        await sleep(1000);
        const plain = document.getElementById('plain');
        plain.value = 'AK';
        plain.dispatchEvent(new Event('change', { bubbles: true }));
        await sleep(100);
        document.getElementById('result').textContent = JSON.stringify({ submits });`);
    assert.deepEqual(result, { submits: ['AL/', 'AL/AK'] });
});

test('date keys step a native date input and a text date field, and leave the native arrows alone', needsBrowser, () => {
    const result = run(`<form><input type="date" id="date" name="date" value="2026-10-25">
        <input type="datetime-local" id="stamp" name="stamp" value="2026-10-25T10:00" step="900">
        <input type="text" id="text" name="text" value="25/10/2026" data-format="dd/MM/yyyy"><input type="text" id="plain" value="x"></form>`, `
        const date = document.getElementById('date'), stamp = document.getElementById('stamp'), text = document.getElementById('text');
        let changes = 0;
        document.addEventListener('change', () => changes++);
        const out = {};
        key(date, 'd'); out.d = date.value;
        key(date, 'W', { shiftKey: true }); out.shiftW = date.value;
        key(date, 'm'); out.m = date.value;
        const arrow = key(date, 'ArrowUp'); out.nativeArrow = [date.value, arrow.defaultPrevented];
        const ctrlD = key(date, 'd', { ctrlKey: true }); out.ctrlD = [date.value, ctrlD.defaultPrevented];
        key(stamp, 'i'); out.i = stamp.value;
        key(stamp, 'h', { shiftKey: true }); out.shiftH = stamp.value;
        key(text, 'ArrowUp'); out.textArrow = text.value;
        key(text, 'y'); out.y = text.value;
        const plain = key(document.getElementById('plain'), 'd'); out.plain = plain.defaultPrevented;
        out.changes = changes;
        document.getElementById('result').textContent = JSON.stringify(out);`);
    assert.deepEqual(result, { d: '2026-10-26', shiftW: '2026-10-19', m: '2026-11-19', nativeArrow: ['2026-11-19', false],
        ctrlD: ['2026-11-19', false], i: '2026-10-25T10:15', shiftH: '2026-10-25T09:15', textArrow: '26/10/2026', y: '26/10/2027',
        plain: false, changes: 7 });
});
