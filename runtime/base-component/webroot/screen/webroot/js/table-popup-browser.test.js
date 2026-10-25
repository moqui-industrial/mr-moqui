/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const { execFileSync } = require('node:child_process');
const { browser, needsBrowser } = require('./chromium-for-tests');

// A table that scrolls sideways (.table-wrap) clips what overflows it, so its menus and drop-downs open outside it. The
// scroll event is dispatched, as headless Chromium with a virtual time budget sends none.
test('a menu and a drop-down in a scrolling table open outside it, and the menu closes when the table scrolls', needsBrowser, () => {
    const webroot = path.join(__dirname, '..');
    const css = fs.readFileSync(path.join(webroot, 'css/moqui-html.css'), 'utf8');
    const pluginCss = fs.readFileSync(path.join(webroot, 'libs/tom-select/tom-select.css'), 'utf8');
    const plugin = fs.readFileSync(path.join(webroot, 'libs/tom-select/tom-select.complete.min.js'), 'utf8');
    const client = fs.readFileSync(path.join(__dirname, 'moqui-html.js'), 'utf8');
    const options = Array.from({ length: 12 }, (_, i) => `<option value="${i}">Option ${i}</option>`).join('');
    const html = `<!doctype html><html data-theme="light"><style>${css}</style><style>${pluginCss}</style>
        <div style="width:500px"><div class="table-wrap" id="wrap"><table class="form-list" style="width:900px"><tbody><tr>
            <td><details class="menu" id="menu"><summary data-btn="primary">Actions</summary><ul><li><a href="#one">One</a></li><li><a href="#two">Two</a></li><li><a href="#three" id="last">Three</a></li></ul></details></td>
            <td><select id="pick" data-ts="select">${options}</select></td><td style="width:600px">Wide</td></tr></tbody></table></div></div>
        <pre id="result"></pre><script>${plugin}</script><script>${client}</script><script>
        setTimeout(()=>{
            const menu=document.getElementById('menu');menu.open=true;
            setTimeout(()=>{
                const r=document.getElementById('last').getBoundingClientRect();
                const menuVisible=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)===document.getElementById('last');
                const pick=document.getElementById('pick').tomselect;pick.open();
                const dropdownOnBody=pick.dropdown.parentElement===document.body;
                const wrap=document.getElementById('wrap');wrap.scrollLeft=50;wrap.dispatchEvent(new Event('scroll'));
                setTimeout(()=>{document.getElementById('result').textContent=JSON.stringify({menuVisible,dropdownOnBody,menuClosed:!menu.open,scrolls:wrap.scrollWidth>wrap.clientWidth});},50);
            },20);
        },50);
        </script>`;
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-table-popup-'));
    let dom;
    try {
        const file = path.join(directory, 'test.html'); fs.writeFileSync(file, html);
        dom = execFileSync(browser, ['--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--window-size=1000,800',
            '--user-data-dir=' + path.join(directory, 'profile'), '--virtual-time-budget=2000', '--dump-dom', 'file://' + file],
            { encoding: 'utf8', timeout: 20000, maxBuffer: 3 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
    } finally { fs.rmSync(directory, { recursive: true, force: true }); }
    const match = dom.match(/<pre id="result">([^<]+)<\/pre>/); assert.ok(match, 'the page script finishes');
    assert.deepEqual(JSON.parse(match[1]), { menuVisible: true, dropdownOnBody: true, menuClosed: true, scrolls: true });
});
