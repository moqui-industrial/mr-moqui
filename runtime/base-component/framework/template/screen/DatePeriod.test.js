const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

for (const mode of ['Html', 'Qvt']) {
    function summary(values, expectedRangeArgs) {
        const factory = vm.runInNewContext(fs.readFileSync(path.join(__dirname, `Screen${mode}.js`), 'utf8'));
        const module = factory({ stmt: fn => fn, noops: () => ({}) }, {
            anyMacros: () => ({}), allElements: [], opaqueElements: []
        });
        const context = { ec: {
            context: { get: key => values[key] ?? null },
            l10n: {
                localize: text => ({ 'from##period': 'from', 'to##period': 'to', Week: 'Woche' })[text] || text,
                format: (value, format) => {
                    assert.equal(format, 'yyyy-MM-dd');
                    return new Date(value).toISOString().slice(0, 10);
                }
            },
            user: { getPeriodRange: (...args) => {
                assert.deepEqual(args, expectedRangeArgs);
                return [Date.UTC(2026, 6, 19), Date.UTC(2026, 9, 18)];
            } }
        } };
        vm.runInNewContext(module.runtime, context);
        return context[`$mq${mode}`].valueText({ name: 'date-period', parent: {
            parent: { attrs: { name: 'entryDate' } }
        } });
    }

    test(`${mode}: date-period summary uses Java's description and inclusive end date`, () => {
        assert.equal(summary({ entryDate_period: '30D', entryDate_poffset: '-3' }, ['30d', '-3', '']),
            '-3 30 Days (2026-07-19 to 2026-10-17)');
    });

    test(`${mode}: offset words, period labels, basis date and localization`, () => {
        const periods = { day: 'Day', '7d': '7 Days', '30d': '30 Days', week: 'Woche', weeks: 'Weeks',
            month: 'Month', months: 'Months', quarter: 'Quarter', year: 'Year', '7r': '+/-7d', '30r': '+/-30d' };
        for (const [period, label] of Object.entries(periods)) {
            for (const [offset, prefix] of [['0', 'This'], ['-1', 'Last'], ['1', 'Next'], ['2', '2']]) {
                assert.equal(summary({ entryDate_period: period, entryDate_poffset: offset, entryDate_pdate: '2026-10-17' },
                    [period, offset, '2026-10-17']), `${prefix} ${label} from 2026-10-17 (2026-07-19 to 2026-10-17)`);
            }
        }
        assert.match(summary({ entryDate_period: 'month' }, ['month', '0', '']), /^This Month /);
    });

    test(`${mode}: absent period falls through to explicit date bounds, not null/undefined text`, () => {
        assert.equal(summary({}), '');
        assert.equal(summary({ entryDate_from: Date.UTC(2026, 0, 1), entryDate_thru: Date.UTC(2026, 0, 31) }),
            'From 2026-01-01 to 2026-01-31');
    });
}
