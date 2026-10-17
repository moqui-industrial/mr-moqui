/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// CHROMIUM_PATH=/usr/bin/chromium node --test runtime/base-component/webroot/screen/webroot/css/WebrootVue.qvt.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');

test('Quasar 2 theme colors reach links, status text and focused inputs', () => {
    const css = fs.readFileSync(path.join(__dirname, 'WebrootVue.qvt.css'), 'utf8');
    const html = `<style>
        :root { --q-primary:rgb(1,2,3); --q-positive:rgb(4,5,6); --q-negative:rgb(7,8,9); }
        ${css}
        .q-menu, .q-toolbar { color:rgb(10,11,12); }
    </style>
    <a id="link" href="#">Link</a>
    <span id="success" class="text-success">Success</span>
    <span id="danger" class="text-danger">Danger</span>
    <span class="form-order-by"><a id="active" class="active" href="#">Sort</a></span>
    <div class="q-field--focused"><input id="input" class="q-field__input"></div>
    <div class="q-menu"><a id="menu" href="#">Menu</a></div>
    <div class="q-toolbar"><a id="toolbar" href="#">Toolbar</a></div>
    <pre id="result"></pre>
    <script>
        const style = id => getComputedStyle(document.getElementById(id));
        document.getElementById('result').textContent = JSON.stringify({
            link:style('link').color, success:style('success').color, danger:style('danger').color,
            active:style('active').color, border:style('input').borderBottomColor,
            borderStyle:style('input').borderBottomStyle,
            menu:style('menu').color, toolbar:style('toolbar').color
        });
    </script>`;
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-qvt-css-'));
    let dom;
    try {
        dom = execFileSync(process.env.CHROMIUM_PATH || 'chromium', [
            '--headless', '--disable-gpu', '--user-data-dir=' + profile,
            '--dump-dom', 'data:text/html,' + encodeURIComponent(html)
        ], { encoding: 'utf8', timeout: 20000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
    } finally {
        fs.rmSync(profile, { recursive: true, force: true });
    }
    const result = JSON.parse(dom.match(/<pre id="result">([^<]+)<\/pre>/)[1]);
    assert.deepEqual(result, {
        link: 'rgb(1, 2, 3)', success: 'rgb(4, 5, 6)', danger: 'rgb(7, 8, 9)',
        active: 'rgb(1, 2, 3)', border: 'rgb(1, 2, 3)', borderStyle: 'solid',
        menu: 'rgb(10, 11, 12)', toolbar: 'rgb(10, 11, 12)'
    });
});
