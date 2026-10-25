/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// node --test dialog-browser.test.js (in headless Chromium, which chromium-for-tests.js finds; skipped without one)
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const { browser, needsBrowser } = require('./chromium-for-tests');

test('a dialog focuses its first form field, not a Tom Select search, and closes on a backdrop click only', needsBrowser, () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-dialog-'));
    try {
        const css = fs.readFileSync(path.join(__dirname, '../css/moqui-html.css'), 'utf8');
        const js = fs.readFileSync(path.join(__dirname, 'moqui-html.js'), 'utf8');
        const file = path.join(directory, 'dialog.html');
        // The body is taller than the dialog, so the dialog scrolls and a click on its scroll bar has the dialog as target.
        fs.writeFileSync(file, `<!doctype html><html data-theme="light"><style>${css}</style>
            <button id="open" data-dialog="customer">New Customer</button>
            <dialog id="customer"><header><h4>New Customer</h4><button class="close" data-close>×</button></header>
                <div class="body"><div class="ts-wrapper"><div class="ts-control"><input name="generatedSearch"></div></div><input name="firstName"><div style="height:2000px"></div></div></dialog>
            <pre id="result"></pre><script>${js}</script><script>
            // after the client binds the page, at DOMContentLoaded
            document.addEventListener('DOMContentLoaded', () => {
            document.getElementById('open').click();
            const dialog = document.getElementById('customer');
            const rect = dialog.getBoundingClientRect();
            const focused = document.activeElement.name;
            dialog.dispatchEvent(new MouseEvent('click', {bubbles:true,clientX:rect.right-2,clientY:rect.top+rect.height/2}));
            const staysOpenInside = dialog.open;
            dialog.dispatchEvent(new MouseEvent('click', {bubbles:true,clientX:1,clientY:1}));
            document.getElementById('result').textContent = JSON.stringify({focused,staysOpenInside,closesOutside:!dialog.open});
            });</script>`);
        const html = execFileSync(browser, [
            '--headless', '--virtual-time-budget=2000', '--disable-extensions', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
            '--window-size=1440,1000', '--dump-dom', file
        ], { encoding:'utf8', timeout:15000, stdio:['ignore','pipe','ignore'] });
        const result = JSON.parse(html.match(/<pre id="result">(.*?)<\/pre>/s)[1]);
        assert.deepEqual(result, { focused:'firstName', staysOpenInside:true, closesOutside:true });
    } finally {
        fs.rmSync(directory, {recursive:true, force:true});
    }
});
