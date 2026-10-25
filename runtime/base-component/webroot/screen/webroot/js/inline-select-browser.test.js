/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const { execFileSync } = require('node:child_process');
const { browser, needsBrowser } = require('./chromium-for-tests');

test('inline remote options set the natural width and resize after a dependency changes', needsBrowser, () => {
    const webroot = path.join(__dirname, '..');
    const css = fs.readFileSync(path.join(webroot, 'css/moqui-html.css'), 'utf8');
    const pluginCss = fs.readFileSync(path.join(webroot, 'libs/tom-select/tom-select.css'), 'utf8');
    const plugin = fs.readFileSync(path.join(webroot, 'libs/tom-select/tom-select.complete.min.js'), 'utf8');
    const client = fs.readFileSync(path.join(__dirname, 'moqui-html.js'), 'utf8');
    const html = `<!doctype html><html data-theme="light"><style>${css}</style><style>${pluginCss}</style>
        <form><select id="country" name="country"><option value="USA">USA</option><option value="CAN">Canada</option></select>
        <div style="width:840px;padding:0 15px"><div class="field big-row"><label>State</label><div class="widget">
            <div class="big-item"><div class="field"><div class="widget"><select id="state" name="state" data-ts="remote" data-url="/states" data-depends='[["country","country"]]'><option selected value="USA_CO">CO - Colorado</option></select></div></div></div>
            <div class="big-item"><div class="field"><div class="widget"><select id="search" data-ts="remote" data-search><option>A</option></select></div></div></div>
        </div></div><div class="field"><label>Owner</label><div class="widget"><select id="owner" data-ts="remote"><option>A</option></select></div></div></div></form>
        <pre id="result"></pre><script>${plugin}</script><script>
        const responses={USA:[{value:'USA_FM',label:'FM - Federated States of Micronesia'},{value:'USA_CO',label:'CO - Colorado'}],CAN:[{value:'CAN_BC',label:'BC - British Columbia'}]};
        let calls=0;window.fetch=(url,options)=>{calls++;const country=new URLSearchParams(options.body).get('country');return Promise.resolve({ok:true,headers:new Headers(),text:()=>Promise.resolve(JSON.stringify(responses[country]))})};
        </script><script>${client}</script><script>
        const metric=()=>{const select=document.getElementById('state');return {width:select.tomselect.wrapper.getBoundingClientRect().width,value:select.value,count:Object.keys(select.tomselect.options).filter(Boolean).length,submitted:new FormData(document.querySelector('form')).get('state')}};
        // a native select of the same options and font, as wide as its longest option
        const natural=()=>{const s=document.createElement('select');s.style.cssText='position:absolute;width:auto;min-width:0;max-width:none;padding:0;border:1px solid;font:'+getComputedStyle(document.getElementById('state')).font;
            for(const o of document.getElementById('state').options)s.appendChild(new Option(o.text,o.value));document.body.appendChild(s);const w=s.getBoundingClientRect().width;s.remove();return w;};
        setTimeout(()=>{
            const initial=metric(),expected=natural();
            const owner=document.getElementById('owner').tomselect.wrapper.getBoundingClientRect().width;
            const ownerWidget=document.getElementById('owner').closest('.widget').clientWidth;
            const search=document.getElementById('search').tomselect.wrapper.style.width;
            document.getElementById('country').value='CAN';document.getElementById('country').dispatchEvent(new Event('change'));
            setTimeout(()=>{document.getElementById('result').textContent=JSON.stringify({initial,expected,owner,ownerWidget,search,updated:metric(),calls,temporarySelects:document.querySelectorAll('select[aria-hidden="true"]').length})},50);
        },50);
        </script>`;
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-inline-select-'));
    let dom;
    try {
        const file = path.join(directory, 'test.html'); fs.writeFileSync(file, html);
        dom = execFileSync(browser, [
            '--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--window-size=1440,1000', '--user-data-dir=' + path.join(directory, 'profile'),
            '--virtual-time-budget=1000', '--dump-dom', 'file://' + file
        ], { encoding: 'utf8', timeout: 20000, maxBuffer: 3 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
    } finally { fs.rmSync(directory, { recursive: true, force: true }); }
    const match = dom.match(/<pre id="result">([^<]+)<\/pre>/); assert.ok(match, 'async option population completes');
    const m = JSON.parse(match[1]);
    // the longest loaded option fits: its native width plus room for the caret, wider than the 10em minimum
    assert.ok(Math.abs(m.initial.width - (m.expected + 24)) < 0.5, JSON.stringify(m)); assert.ok(m.initial.width > 200);
    assert.equal(m.initial.value, 'USA_CO'); assert.equal(m.initial.submitted, 'USA_CO'); assert.equal(m.initial.count, 2);
    assert.ok(m.updated.width < m.initial.width); assert.ok(m.updated.width >= 60);
    assert.equal(m.updated.value, 'CAN_BC'); assert.equal(m.updated.submitted, 'CAN_BC'); assert.equal(m.updated.count, 1);
    // a select that is not inline, and one that searches the server, keep their sizing
    assert.ok(m.owner > 600, JSON.stringify(m)); assert.equal(m.search, ''); assert.equal(m.calls, 2); assert.equal(m.temporarySelects, 0);
});
