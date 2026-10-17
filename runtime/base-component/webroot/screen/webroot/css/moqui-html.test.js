/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// CHROMIUM_PATH=/usr/bin/chromium node --test runtime/base-component/webroot/screen/webroot/css/moqui-html.test.js
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');

test('untitled big rows use full width while titled rows retain their label column', () => {
    const css = fs.readFileSync(path.join(__dirname, 'moqui-html.css'), 'utf8');
    const row = (id, width, title = '') => `<div style="width:${width}px"><div id="${id}" class="field big-row">
        ${title ? '<label>Title</label>' : ''}<div class="widget">
            <div class="big-item"><div class="field"><div class="widget"><input size="40"></div></div></div>
            <div class="big-item"><div class="field"><div class="widget"><button>Text Search</button></div></div></div>
        </div></div></div>`;
    const html = `<style>${css}</style>${row('wide', 690)}${row('titled', 690, 'Title')}${row('narrow', 250)}
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
            wide:measure('wide'), titled:measure('titled'), narrow:measure('narrow')
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
    const { wide, titled, narrow } = JSON.parse(dom.match(/<pre id="result">([^<]+)<\/pre>/)[1]);
    assert.equal(wide.widget.width, wide.width);
    assert.ok(wide.input.width > 300, 'size=40 input is not squeezed into the unused label column');
    assert.ok(Math.abs(wide.input.y - wide.button.y) < 10, 'input and button share a line');
    assert.ok(titled.widget.x > wide.widget.x + 50, 'titled rows still reserve label space');
    assert.ok(titled.widget.width < titled.width);
    assert.equal(narrow.widget.width, narrow.width);
    assert.ok(narrow.input.width <= narrow.width, 'inputs stay constrained at narrow widths');
    assert.ok(narrow.button.y > narrow.input.y + 20, 'controls may wrap when space is genuinely limited');
});
