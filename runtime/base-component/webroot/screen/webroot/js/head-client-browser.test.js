/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const { execFileSync } = require('node:child_process');
const { browser, needsBrowser } = require('./chromium-for-tests');

// Header.html.fancy loads moqui-html.js in the head without defer: a custom element in the body (mr-calendar) can call
// moqui.request as it connects, and the client binds the page's elements once it has parsed.
test('the html client in the head has moqui.request while the body parses and binds the page after', needsBrowser, () => {
    const source = fs.readFileSync(path.join(__dirname, 'moqui-html.js'), 'utf8');
    const html = `<!doctype html><html><head><meta name="moquiSessionToken" content="initial"><script>
        window.calls=[];window.fetch=(url,options)=>{calls.push({url,credentials:options.credentials});return Promise.resolve(new Response('[]',{headers:{'Content-Type':'application/json'}}))};
        </script><script>${source}</script></head><body><button data-dialog="detail">Details</button><dialog id="detail">Loaded</dialog><pre id="result"></pre><script>
        const before={request:typeof moqui.request,ready:document.readyState,bound:document.documentElement.classList.contains('js')};
        const answer=moqui.request('/calendar/events');
        document.addEventListener('DOMContentLoaded',()=>{setTimeout(async()=>{await answer;document.querySelector('button').click();
            document.getElementById('result').textContent=JSON.stringify({before,calls,bound:document.documentElement.classList.contains('js'),opened:document.querySelector('dialog').open});},0)},{once:true});
        </script></body></html>`;
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-head-client-')); let dom;
    try {
        const file = path.join(dir, 'test.html'); fs.writeFileSync(file, html);
        dom = execFileSync(browser, ['--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--user-data-dir=' + path.join(dir, 'profile'), '--virtual-time-budget=1000', '--dump-dom', 'file://' + file],
            { encoding: 'utf8', timeout: 20000, maxBuffer: 3 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
    } finally { fs.rmSync(dir, { recursive: true, force: true }); }
    const match = dom.match(/<pre id="result">([^<]+)<\/pre>/); assert.ok(match, 'the request made while parsing finishes');
    const m = JSON.parse(match[1]);
    assert.deepEqual(m.before, { request: 'function', ready: 'loading', bound: false });
    assert.deepEqual(m.calls, [{ url: '/calendar/events', credentials: 'same-origin' }]);
    assert.equal(m.bound, true); assert.equal(m.opened, true);
});
