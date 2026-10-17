const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function renderList({ saved = true, header = true, denied = false, hideNav = false, finds = [] } = {}) {
    const factory = vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'ScreenQvt.js'), 'utf8'));
    const module = factory({ stmt: fn => fn, noops: () => ({}) }, {
        anyMacros: () => ({}), allElements: [], opaqueElements: []
    });
    let html = '';
    const url = () => ({
        path: '/qapps/FindPayment', getDisableLink: () => denied,
        cloneUrlInstance: url, removeParameter() { return this; },
        getParameterMap: () => ({ statusId: 'PmntProposed' })
    });
    const context = {
        sri: {
            write: text => { html += text; }, buildUrl: url, makeUrlByType: url,
            getScreenUrlInstance: url, getCurrentScreenUrl: url,
            getFormHiddenParameters: () => ({}), getFormListHeaderValues: () => ({}),
            getUserFormListFinds: () => finds, getUserDefaultFormListFindId: () => '',
            pushSingleFormMapContext() {}, popContext() {}, safeCloseList() {}
        },
        ec: {
            context: { get: key => key === 'hideNav' && hideNav ? 'true' : null },
            l10n: { localize: text => text }, resource: { expand: text => text }
        }
    };
    vm.runInNewContext(module.runtime, context);
    context.$mqQvt.formList({ count: 0 }, { attrs: {
        name: 'PaymentList', list: 'payments', 'saved-finds': String(saved), 'header-dialog': String(header)
    } }, [], { headerForm: header, valueDescs: { h: [] } });
    return html;
}

function dialog(html, id) {
    return html.match(new RegExp('<m-container-dialog id="' + id + '"[^>]*>([\\s\\S]*?)</m-container-dialog>'))?.[1];
}

test('saved finds have a separate named dialog, not inside Find Options', () => {
    const html = renderList();
    const saved = dialog(html, 'PaymentList_sfdialog');
    assert.ok(saved);
    assert.match(saved, /icon="find_in_page" label="Saved Finds"/);
    assert.match(saved, /label="Save New Find"/);
    const find = dialog(html, 'PaymentList_hdialog');
    assert.match(find, /label="Find Options"/);
    assert.match(find, /label="Clear Parameters"/);
    assert.doesNotMatch(find, /Save New Find|_NewFind/);
});

test('saved finds remain available without a header dialog', () => {
    const html = renderList({ header: false });
    assert.ok(dialog(html, 'PaymentList_sfdialog'));
    assert.equal(dialog(html, 'PaymentList_hdialog'), undefined);
});

test('disabled, unauthorized and hidden navigation do not expose saved finds', () => {
    for (const options of [{ saved: false }, { denied: true }, { hideNav: true }]) {
        assert.equal(dialog(renderList(options), 'PaymentList_sfdialog'), undefined);
    }
});

test('server-static forms also separate Saved Finds from Find Options', () => {
    const factory = vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'ScreenQvtStatic.js'), 'utf8'));
    const module = factory({ stmt: fn => fn, noops: () => ({}) }, {
        anyMacros: () => ({}), allElements: [], opaqueElements: []
    });
    let html = '';
    const context = { sri: { write: text => { html += text; }, staticUrl: () => ({ path: '/qapps/FindPayment' }) } };
    vm.runInNewContext(module.runtime, context);
    context.$mqQvtS.formList({ attrs: { name: 'PaymentList', list: 'payments', 'saved-finds': 'true', 'header-dialog': 'true' }, children: [] }, [], []);
    assert.match(dialog(html, 'PaymentList_sfdialog'), /icon="find_in_page"/);
    assert.match(dialog(html, 'PaymentList_sfdialog'), /Save New Find/);
    assert.doesNotMatch(dialog(html, 'PaymentList_hdialog'), /Save New Find|_NewFind/);
});

test('existing saved find actions stay in the Saved Finds dialog', () => {
    const html = renderList({ finds: [{
        formListFind: { formListFindId: 'mine' }, description: 'My payments', isByUserId: 'true'
    }] });
    const saved = dialog(html, 'PaymentList_sfdialog');
    assert.ok(saved);
    for (const label of ['My payments', 'Update to Current', 'Make Default', 'Delete']) {
        assert.ok(saved.includes(label), label);
    }
});
