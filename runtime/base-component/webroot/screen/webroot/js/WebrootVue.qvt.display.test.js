/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// CHROMIUM_PATH=/usr/bin/chromium node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.display.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');

test('labelled display values use Java read-only outlined inputs and inline values stay text', () => {
    const webroot = path.join(__dirname, '..');
    const read = name => fs.readFileSync(path.join(webroot, name), 'utf8');
    const scripts = ['libs/vue/vue.global.prod.js', 'libs/quasar/quasar.umd.prod.js', 'js/MoquiLib.js', 'js/WebrootVue.qvt.js'];
    const html = `<style>${read('libs/quasar/quasar.prod.css')}</style><div id="app">
        <m-display id="invoice" name="invoiceId" label="Invoice ID" label-wrapper model-value="UI_PARITY_INVOICE"></m-display>
        <m-display id="total" name="total" label="Total" label-wrapper display="$0.00"></m-display>
        <m-display id="zero" name="zero" label="Zero" label-wrapper display="0"></m-display>
        <m-display id="multiline" name="multiline" label="Notes" label-wrapper display="First&#10;Second"></m-display>
        <m-display id="inline" model-value="inline text"></m-display>
    </div><pre id="result"></pre>` + scripts.map(name => '<script>' + read(name).replace(/<\/script/gi, '<\\/script') + '</script>').join('') + `<script>
        const app = Vue.createApp({});
        app.use(Quasar);
        app.component('m-display', moqui.qvt.components['m-display']);
        app.mount('#app');
        Vue.nextTick(() => {
            const input = document.getElementById('invoice');
            const total = document.getElementById('total');
            const zero = document.getElementById('zero');
            const multiline = document.getElementById('multiline');
            document.getElementById('result').textContent = JSON.stringify({
                value: input.value, name: input.name, readOnly: input.readOnly,
                outlined: !!input.closest('.q-field--outlined'), label: input.closest('.q-field').textContent,
                total: total.value, zero: zero.value, multiline: multiline.value,
                multilineIsTextarea: multiline.matches('textarea'),
                inline: document.getElementById('inline').textContent,
                inlineIsInput: document.getElementById('inline').matches('input,textarea')
            });
        });
    </script>`;
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-display-'));
    try {
        const fixture = path.join(profile, 'fixture.html');
        fs.writeFileSync(fixture, html);
        const dom = execFileSync(process.env.CHROMIUM_PATH || 'chromium', [
            '--headless', '--disable-gpu', '--user-data-dir=' + profile, '--dump-dom', 'file://' + fixture
        ], { encoding: 'utf8', timeout: 20000, maxBuffer: 8 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
        const match = dom.match(/<pre id="result">([^<]+)<\/pre>/);
        assert.ok(match, 'browser did not report rendered fields');
        const result = JSON.parse(match[1].replace(/&amp;/g, '&').replace(/&quot;/g, '"'));
        assert.equal(result.value, 'UI_PARITY_INVOICE');
        assert.equal(result.name, 'invoiceId');
        assert.equal(result.readOnly, true);
        assert.equal(result.outlined, true);
        assert.match(result.label, /Invoice ID/);
        assert.equal(result.total, '$0.00');
        assert.equal(result.zero, '0');
        assert.equal(result.multiline, 'First\nSecond');
        assert.equal(result.multilineIsTextarea, true);
        assert.equal(result.inline, 'inline text');
        assert.equal(result.inlineIsInput, false);
    } finally {
        fs.rmSync(profile, { recursive: true, force: true });
    }
});
