/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// node --test qvt-keys-browser.test.js (in headless Chromium, which chromium-for-tests.js finds; skipped without one)
// The keys of the qvt widgets with Vue, Quasar and WebrootVue.qvt.js in the page, without a server.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const { browser, needsBrowser } = require('./chromium-for-tests');

// Mounts `template` with `data` and runs `script`, which writes its result to #result.
const run = (template, data, script) => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-qvt-keys-'));
    const web = path.join(__dirname, '..');
    const read = (relative) => fs.readFileSync(path.join(web, relative), 'utf8').replace(/<\/script/gi, '<\\/script');
    try {
        const file = path.join(directory, 'keys.html');
        fs.writeFileSync(file, `<!doctype html><html><head><style>${read('libs/quasar/quasar.prod.css')}</style></head><body>
            <div id="app"></div><pre id="result"></pre>
            <script>${read('libs/vue/vue.global.prod.js')}</script><script>${read('libs/quasar/quasar.umd.prod.js')}</script>
            <script>${read('js/MoquiLib.js')}</script><script>${read('js/MoquiL10n.js')}</script><script>${read('js/WebrootVue.qvt.js')}</script>
            <script>
            const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
            const key = (node, name, extra) => { const event = new KeyboardEvent('keydown', Object.assign({ key: name, bubbles: true, cancelable: true,
                keyCode: { Enter: 13, Tab: 9, ArrowUp: 38, ArrowDown: 40 }[name] || name.toUpperCase().charCodeAt(0) }, extra || {})); node.dispatchEvent(event); return event; };
            const type = (node, text) => { node.focus(); node.value = text; node.dispatchEvent(new Event('input', { bubbles: true })); };
            const app = Vue.createApp({ data() { return ${JSON.stringify(data)}; }, template: ${JSON.stringify(template)} });
            app.use(Quasar);
            Object.keys(moqui.qvt.components).forEach((name) => app.component(name, moqui.qvt.components[name]));
            const vm = app.mount('#app');
            (async () => { try { await sleep(200); ${script} } catch (error) { document.getElementById('result').textContent = JSON.stringify({ error: error.stack }); } })();
            </script></body></html>`);
        const dom = execFileSync(browser, ['--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--virtual-time-budget=10000',
            '--window-size=1200,900', '--dump-dom', file], { encoding: 'utf8', timeout: 60000, maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'ignore'] });
        return JSON.parse(dom.match(/<pre id="result">(.*?)<\/pre>/s)[1]);
    } finally {
        fs.rmSync(directory, { recursive: true, force: true });
    }
};

const states = "[{value:'AL',label:'Alabama'},{value:'AK',label:'Alaska'},{value:'AZ',label:'Arizona'}]";

test('an m-drop-down takes the typed match on a quick ENTER or TAB, and TAB moves on', needsBrowser, () => {
    const result = run(`<div><m-drop-down id="one" name="one" v-model="one" allow-empty :options="${states}"></m-drop-down>
        <m-drop-down id="two" name="two" v-model="two" allow-empty :options="${states}"></m-drop-down><input id="next"></div>`, { one: null, two: null }, `
        const one = document.querySelector('#app input');
        type(one, 'Ariz');
        key(one, 'Enter');
        await sleep(1000);
        const afterEnter = vm.one;
        const two = document.querySelectorAll('#app input')[1];
        type(two, 'Alas');
        key(two, 'Tab');
        await sleep(1000);
        const afterTab = vm.two, focus = document.activeElement.id;
        // typed at a person's pace: the first match is highlighted, and ENTER takes it
        type(one, 'Alab');
        await sleep(700);
        key(one, 'Enter');
        await sleep(300);
        document.getElementById('result').textContent = JSON.stringify({ afterEnter, afterTab, focus, slowEnter: vm.one });`);
    assert.deepEqual(result, { afterEnter: 'AZ', afterTab: 'AK', focus: 'next', slowEnter: 'AL' });
});
