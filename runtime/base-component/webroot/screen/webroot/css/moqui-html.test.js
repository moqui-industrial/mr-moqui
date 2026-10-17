/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// CHROMIUM_PATH=/usr/bin/chromium node --test runtime/base-component/webroot/screen/webroot/css/moqui-html.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');

test('Java CSS metrics and untitled big-row layout are preserved', () => {
    const css = fs.readFileSync(path.join(__dirname, 'moqui-html.css'), 'utf8');
    const row = (id, width, title = '') => `<div style="width:${width}px"><div id="${id}" class="field big-row">
        ${title ? '<label>Title</label>' : ''}<div class="widget">
            <div class="big-item"><div class="field"><div class="widget"><input size="40"></div></div></div>
            <div class="big-item"><div class="field"><div class="widget"><button>Text Search</button></div></div></div>
        </div></div></div>`;
    const html = `<!DOCTYPE html><html data-theme="light"><style>${css}</style>${row('wide', 690)}${row('titled', 690, 'Title')}${row('narrow', 250)}
        <table class="form-list"><thead><tr id="heading"><th><div class="title">ID <span class="order-by"><a href="#">▲</a><a class="active" href="#">▼</a></span></div><div class="title">Name</div></th></tr></thead>
        <tbody><tr id="product"><td id="cell"><div><a href="#">DEMO_1_1</a></div><div><a href="#">Demo Product One-One</a></div></td>
        <td><div><span class="display">Inventory</span></div><div><span class="display">Inventory - Finished Good</span></div></td></tr></tbody></table>
        <button id="button">Find Options</button><button id="NewProductDialog-button" data-dialog="NewProductDialog">New Product</button><strong id="strong">Label</strong>
        <table class="form-list"><thead><tr class="nav-row"><th><nav><button id="find">Find Options</button><ul class="pagination"><li><span id="page">Page 1</span></li></ul><a id="csv" data-btn="default">CSV</a></nav></th></tr></thead></table>
        <pre id="result"></pre><script>
        const measure = id => {
            const row = document.getElementById(id);
            const rect = selector => {
                const { x, y, width } = row.querySelector(selector).getBoundingClientRect();
                return { x, y, width };
            };
            return { width:row.getBoundingClientRect().width, widget:rect(':scope > .widget'),
                input:rect('input'), button:rect('button') };
        };
        document.getElementById('result').textContent = JSON.stringify({
            wide:measure('wide'), titled:measure('titled'), narrow:measure('narrow'),
            javaMetrics: {
                font:getComputedStyle(document.body).fontFamily, fontSize:getComputedStyle(document.body).fontSize,
                lineHeight:getComputedStyle(document.body).lineHeight, background:getComputedStyle(document.body).backgroundColor,
                color:getComputedStyle(document.body).color, cellPadding:getComputedStyle(document.getElementById('cell')).padding,
                cellAlign:getComputedStyle(document.getElementById('cell')).verticalAlign,
                rowHeight:document.getElementById('product').getBoundingClientRect().height,
                headerHeight:document.getElementById('heading').getBoundingClientRect().height,
                sortColor:getComputedStyle(document.querySelector('.order-by a.active')).color,
                dialogFont:getComputedStyle(document.getElementById('NewProductDialog-button')).fontSize,
                dialogPadding:getComputedStyle(document.getElementById('NewProductDialog-button')).padding,
                buttonPadding:getComputedStyle(document.getElementById('button')).padding,
                strongWeight:getComputedStyle(document.getElementById('strong')).fontWeight,
                stripe:getComputedStyle(document.getElementById('product')).backgroundColor,
                navMargin:getComputedStyle(document.getElementById('find')).margin,
                pagePadding:getComputedStyle(document.getElementById('page')).padding,
                csvBeforePagination:document.getElementById('csv').getBoundingClientRect().x < document.getElementById('page').getBoundingClientRect().x
            }
        });
        </script>`;
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'moqui-html-css-'));
    let dom;
    try {
        dom = execFileSync(process.env.CHROMIUM_PATH || 'chromium', [
            '--headless', '--disable-gpu', '--window-size=1440,1000', '--user-data-dir=' + profile,
            '--dump-dom', 'data:text/html,' + encodeURIComponent(html)
        ], { encoding: 'utf8', timeout: 20000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
    } finally {
        fs.rmSync(profile, { recursive: true, force: true });
    }
    const { wide, titled, narrow, javaMetrics } = JSON.parse(dom.match(/<pre id="result">([^<]+)<\/pre>/)[1]);
    // Java webroot-layout.css/table-condensed and webroot-theme.css flattening, with Bootstrap typography.
    assert.deepEqual(javaMetrics, {
        font: '"Helvetica Neue", Helvetica, Arial, sans-serif', fontSize: '14px', lineHeight: '20px',
        background: 'rgb(255, 255, 255)', color: 'rgb(34, 34, 34)', cellPadding: '5px', cellAlign: 'middle',
        rowHeight: 51, headerHeight: 51, sortColor: 'rgb(255, 0, 0)', dialogFont: '12px', dialogPadding: '5px 10px',
        buttonPadding: '6px 12px', strongWeight: '700', stripe: 'rgba(0, 0, 0, 0)',
        navMargin: '6px 8px 0px 0px', pagePadding: '6px 12px', csvBeforePagination: true
    });
    assert.equal(wide.widget.width, wide.width);
    assert.ok(wide.input.width > 300, 'size=40 input is not squeezed into the unused label column');
    assert.ok(Math.abs(wide.input.y - wide.button.y) < 10, 'input and button share a line');
    assert.ok(titled.widget.x > wide.widget.x + 50, 'titled rows still reserve label space');
    assert.ok(titled.widget.width < titled.width);
    assert.equal(narrow.widget.width, narrow.width);
    assert.ok(narrow.input.width <= narrow.width, 'inputs stay constrained at narrow widths');
    assert.ok(narrow.button.y > narrow.input.y + 20, 'controls may wrap when space is genuinely limited');
});
