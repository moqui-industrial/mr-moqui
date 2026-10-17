const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const load = file => vm.runInNewContext(fs.readFileSync(path.join(__dirname, file), 'utf8'));
const T = load('../TemplateSupport.js');
const S = load('ScreenSupport.js')(T);
const node = (name, attrs = {}, children = []) => {
    const result = { name, attrs, children };
    children.forEach(child => { child.parent = result; });
    return result;
};

for (const mode of ['Html', 'Qvt']) {
    const module = load(`Screen${mode}.js`)(T, S);
    function renderSubmit(attrs, subName = 'default-field') {
        const submit = node('submit', { icon: 'fa fa-trash', confirmation: 'Delete role?' });
        node('form-single', { name: 'RolesForm' }, [
            node('field', { name: 'submitButton' }, [node(subName, attrs, [submit])])
        ]);
        let html = '';
        const context = {
            sri: { write: text => { html += text; }, getThemeIconClass: () => '' },
            ec: { context: { get: () => null }, resource: { expand: text => text }, l10n: { localize: text => text } }
        };
        vm.runInNewContext(module.runtime, context);
        context[mode === 'Html' ? '$h' : '$v'] = context[`$mq${mode}`];
        vm.runInNewContext(module.macros.submit(submit, []), context);
        return html;
    }

    test(`${mode}: explicit empty default/conditional titles keep delete buttons icon-only`, () => {
        for (const subName of ['default-field', 'conditional-field']) {
            const html = renderSubmit({ title: '' }, subName);
            assert.match(html, /fa fa-trash/);
            assert.match(html, /data-confirm="Delete role\?"/);
            assert.match(html, /name="submitButton"/);
            assert.doesNotMatch(html, /Submit Button/);
            assert.equal(html.replace(/<[^>]*>/g, '').trim(), '');
        }
    });

    test(`${mode}: missing titles still derive words; nonempty titles still render`, () => {
        assert.equal(renderSubmit({}).replace(/<[^>]*>/g, '').trim(), 'Submit Button');
        assert.equal(renderSubmit({ title: 'Delete Role' }).replace(/<[^>]*>/g, '').trim(), 'Delete Role');
    });

    test(`${mode}: compiled header titles distinguish empty, missing and default fallback`, () => {
        for (const [headerAttrs, defaultAttrs, expected] of [
            [{ title: '' }, { title: 'Default Title' }, ''],
            [{}, { title: '' }, ''],
            [{}, { title: 'Default Title' }, 'Default Title'],
            [{ title: 'Header Title' }, { title: '' }, 'Header Title'],
            [{}, {}, 'Submit Button']
        ]) {
            const form = node('form-list', { name: 'RolesForm' }, [node('field', { name: 'submitButton' }, [
                node('header-field', headerAttrs, [node('display')]),
                node('default-field', defaultAttrs, [node('display')])
            ])]);
            let config;
            const context = {
                sri: { doBoundaryComments: () => false, openFormList: () => ({}), pushContext() {}, popContext() {} },
                ec: { l10n: { localize: text => text }, resource: { expand: text => text } }
            };
            context[mode === 'Html' ? '$h' : '$v'] = { formList: (info, form, columns, cfg) => { config = cfg; } };
            vm.runInNewContext(module.macros['form-list'](form, []), context);
            assert.equal(config.allFields[0].title(), expected);
        }
    });
}
