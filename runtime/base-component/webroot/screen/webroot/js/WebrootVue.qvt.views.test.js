/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of the form-list views of WebrootVue.qvt.js that need no browser (moqui.listViews, which m-list-views draws): the views
// offered, a board's columns (the status flow's, column-order's, then the rows' values) with counts and sums, a card moved, the
// wall clock of a time stamp in the user's zone and back, the events of a calendar (all day, no end, a date only, moved), the
// lanes of overlapping events, a month's weeks, and what a move posts. Dragging cards and events, the popover and the quick
// create are the Chromium smoke of Gate 3 (crates/moqui-host/tests/gate3_browser.rs).
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.views.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");

globalThis.moqui = require("./MoquiLib.js");
require("./WebrootVue.qvt.js");
const lv = moqui.listViews;
const CHICAGO = "America/Chicago";

test("the views offered are those named that have an element, else the list and each element there is", () => {
    assert.deepEqual(lv.views("board,list,calendar", {}, {}), ["board", "list", "calendar"]);
    assert.deepEqual(lv.views("list,calendar,board", {}, null), ["list", "board"], "calendar has no element");
    assert.deepEqual(lv.views(["calendar", "calendar", "list"], null, {}), ["calendar", "list"]);
    assert.deepEqual(lv.views("", {}, {}), ["list", "board", "calendar"]);
    assert.deepEqual(lv.views([], null, {}), ["list", "calendar"]);
    assert.deepEqual(lv.views("pivot", {}, null), ["list", "board"]);
    assert.deepEqual(lv.list(" a, b ,,c"), ["a", "b", "c"]);
    assert.equal(lv.isStatusField("statusId"), true);
    assert.equal(lv.isStatusField("workEffortStatusId"), true);
    assert.equal(lv.isStatusField("stage"), false);
});

test("a board has the known columns in order, then a column for each other value, with counts, sums and the cards moved", () => {
    const rows = [
        { exampleId: "E1", statusId: "EXST_DEFINED", statusId_display: "Defined", size: "3" },
        { exampleId: "E2", statusId: "EXST_IN_DESIGN", size: 2 },
        { exampleId: "E3", statusId: "EXST_DEFINED", size: "1.5" },
        { exampleId: "E4", statusId: "OTHER", statusId_display: "Other one", size: null },
        { exampleId: "E5", statusId: null }
    ];
    const items = rows.map((row, index) => ({ index, row }));
    const known = [{ value: "EXST_IN_DESIGN", label: "In Design", color: "grey-7" }, { value: "EXST_DEFINED", label: "Defined", color: "primary" },
        { value: "EXST_APPROVED", label: "Approved", color: "primary" }];
    const board = { groupField: "statusId", sumField: "size" };
    const columns = lv.boardColumns(items, board, known, {});
    assert.deepEqual(columns.map((column) => column.value), ["EXST_IN_DESIGN", "EXST_DEFINED", "EXST_APPROVED", "OTHER", ""]);
    assert.deepEqual(columns.map((column) => column.count), [1, 2, 0, 1, 1]);
    assert.deepEqual(columns.map((column) => column.sum), [2, 4.5, 0, 0, 0]);
    assert.equal(columns[3].label, "Other one", "a value not known takes the rows' text");
    assert.equal(columns[4].label, "None");
    assert.equal(columns[1].color, "primary");
    // a card moved to Approved, until the list loads again
    const moved = lv.boardColumns(items, board, known, { 1: { group: "EXST_APPROVED" } });
    assert.deepEqual(moved.map((column) => column.items.map((item) => item.row.exampleId)), [[], ["E1", "E3"], ["E2"], ["E4"], ["E5"]]);
    // column-order picks the columns and their order; labels from the known columns, else the rows; a value outside the order
    // takes the label and color the server sent for it (a status's description), not its id
    const ordered = lv.boardColumns(items, { groupField: "statusId", columnOrder: "EXST_DEFINED, OTHER" }, known, {});
    assert.deepEqual(ordered.map((column) => column.value + ":" + column.label), ["EXST_DEFINED:Defined", "OTHER:Other one", "EXST_IN_DESIGN:In Design", ":None"]);
    assert.equal(ordered[2].color, "grey-7");
    // no known columns: the values in the order of the rows
    assert.deepEqual(lv.boardColumns(items, { groupField: "statusId" }, null, {}).map((column) => column.value), ["EXST_DEFINED", "EXST_IN_DESIGN", "OTHER", ""]);
});

test("a click on a control of a card (a link, a button, a field, an in-place edit) is the control's, not the card's", () => {
    // a minimal element: its tag, classes, attributes and parent, with closest and contains over that chain
    const el = (tag, classes, attrs, parent) => {
        const node = { tagName: tag.toUpperCase(), classList: classes || [], attrs: attrs || {}, parentElement: parent || null };
        const matches = (at, selector) => selector.split(",").map((part) => part.trim()).some((part) => {
            if (/^[a-z]+$/.test(part)) return at.tagName === part.toUpperCase();
            if (part.startsWith(".")) return at.classList.includes(part.slice(1));
            const attr = /^\[([a-z-]+)(?:='([^']*)')?\]$/.exec(part);
            return !!attr && Object.prototype.hasOwnProperty.call(at.attrs, attr[1]) && (attr[2] === undefined || at.attrs[attr[1]] === attr[2]);
        });
        node.closest = (selector) => { for (let at = node; at; at = at.parentElement) if (matches(at, selector)) return at; return null; };
        node.contains = (other) => { for (let at = other; at; at = at.parentElement) if (at === node) return true; return false; };
        return node;
    };
    const board = el("div", ["m-board"]);
    const card = el("div", ["m-board-card"], {}, board);
    const body = el("div", ["m-card-field"], {}, card);
    const editable = el("span", ["editable-label", "cursor-pointer"], {}, body);
    const shown = el("span", [], {}, editable);
    const plain = el("span", ["m-card-value"], {}, body);
    const titleLink = el("a", ["m-link-text"], { href: "/x" }, el("div", ["m-card-title"], {}, card));
    const chip = el("div", ["q-chip", "q-chip--clickable"], {}, body);
    const marked = el("div", [], { "data-card-control": "" }, body);
    assert.equal(lv.onControl(shown, card), true, "the text inside an in-place edit");
    assert.equal(lv.onControl(editable, card), true);
    assert.equal(lv.onControl(titleLink, card), true, "the title link follows itself");
    assert.equal(lv.onControl(chip, card), true);
    assert.equal(lv.onControl(marked, card), true);
    assert.equal(lv.onControl(plain, card), false, "plain text opens the card");
    assert.equal(lv.onControl(card, card), false);
    // a control around the board is not the card's
    board.parentElement = el("label", [], {}, null);
    assert.equal(lv.onControl(plain, card), false);
    assert.equal(lv.onControl(null, card), false);
});

test("range-find: the days each mode shows, the finds of those days, and the rows they answer as the calendar's items", () => {
    const oct19 = lv.dayOf(2026, 10, 19);
    // a Monday; the week from Sunday (0)
    assert.deepEqual(lv.visibleDays("day", oct19, 0), { first: oct19, last: oct19 });
    assert.deepEqual(lv.visibleDays("week", oct19, 0), { first: lv.dayOf(2026, 10, 18), last: lv.dayOf(2026, 10, 24) });
    assert.deepEqual(lv.visibleDays("month", oct19, 0), { first: lv.dayOf(2026, 9, 27), last: lv.dayOf(2026, 10, 31) });
    assert.deepEqual(lv.visibleDays("list", oct19, 0), { first: lv.dayOf(2026, 10, 1), last: lv.dayOf(2026, 10, 31) });
    // a day wider on each side, in the user's zone, as epoch millis; with an end field what overlaps and what starts in the days
    const days = { first: oct19, last: oct19 };
    const from = String(lv.millis(oct19 - 1, 0, CHICAGO));
    const thru = String(lv.millis(oct19 + 2, 0, CHICAGO) - 1);
    assert.equal(from, String(Date.UTC(2026, 9, 18, 5)), "midnight in Chicago (CDT) the day before");
    assert.deepEqual(lv.rangeFinds({ startField: "start", endField: "thru" }, days, CHICAGO), [
        { pageNoLimit: "true", start_thru: thru, thru_from: from },
        { pageNoLimit: "true", start_from: from, start_thru: thru }
    ]);
    assert.deepEqual(lv.rangeFinds({ startField: "start" }, days, CHICAGO), [{ pageNoLimit: "true", start_from: from, start_thru: thru }]);
    // the rows of both finds once each; a row on the list's page is its item (its card slot), another gets an index of its own
    const page = [{ index: 0, row: { taskId: "T1", name: "On the page", start: "1760882400000" } }];
    const lists = [
        [{ taskId: "T1", name: "On the page (as found)", start: 1760882400000 }, { taskId: "T7", name: "Next month", start: 1763560800000 }],
        [{ taskId: "T7", name: "Next month", start: 1763560800000 }, { taskId: "T8", name: "No end" }, { _moquiRowType: "total" }]
    ];
    const items = lv.rangeItems(page, lists, ["taskId"]);
    assert.deepEqual(items.map((item) => item.index + ":" + item.row.name), ["0:On the page", "100000:Next month", "100001:No end"]);
    assert.equal(items[0], page[0]);
    // without key fields a row is known by its values
    assert.equal(lv.rangeItems([], [[{ a: 1 }, { a: 1 }, { a: 2 }]], []).length, 2);
});

test("a row's text is its display text, else its value; numbers read from text; key fields as parameters", () => {
    assert.equal(lv.text({ a: "X", a_display: "Shown" }, "a"), "Shown");
    assert.equal(lv.text({ a: "X", a_display: "" }, "a"), "X");
    assert.equal(lv.text({ a: 0 }, "a"), "0");
    assert.equal(lv.text(null, "a"), "");
    assert.equal(lv.number("1,234.50"), 1234.5);
    assert.equal(lv.number("x"), 0);
    assert.deepEqual(lv.keyParams({ exampleId: "E1", other: 2, none: null }, "exampleId,none"), { exampleId: "E1" });
});

test("a value's color is the map's, else one of the brand palette by its hash, the same every time", () => {
    assert.equal(lv.colorOf("A", { A: "negative" }), "negative");
    const color = lv.colorOf("EXT_MADE_UP", null);
    assert.ok(lv.palette.includes(color));
    assert.equal(lv.colorOf("EXT_MADE_UP", {}), color);
    assert.equal(lv.colorOf("", null), "grey-7");
    assert.equal(lv.lightColor("warning"), true);
    assert.equal(lv.lightColor("primary"), false);
});

test("a time stamp is the wall clock of the user's zone, and a wall clock time is a time stamp again, over daylight saving", () => {
    const millis = Date.UTC(2026, 9, 19, 15, 30);
    assert.deepEqual(lv.wall(millis, CHICAGO), { day: lv.dayOf(2026, 10, 19), minutes: 10 * 60 + 30 });
    assert.deepEqual(lv.wall(millis, "UTC"), { day: lv.dayOf(2026, 10, 19), minutes: 15 * 60 + 30 });
    assert.equal(lv.millis(lv.dayOf(2026, 10, 19), 630, CHICAGO), millis);
    // after the change back to standard time (Nov 1, 2026): six hours behind
    assert.equal(lv.millis(lv.dayOf(2026, 11, 2), 9 * 60, CHICAGO), Date.UTC(2026, 10, 2, 15, 0));
    // a night of the change: midnight before, noon after
    assert.equal(lv.millis(lv.dayOf(2026, 11, 1), 0, CHICAGO), Date.UTC(2026, 10, 1, 5, 0));
    assert.equal(lv.millis(lv.dayOf(2026, 11, 1), 12 * 60, CHICAGO), Date.UTC(2026, 10, 1, 18, 0));
    assert.equal(lv.dayKey(lv.dayOf(2026, 2, 28) + 1), "2026-03-01");
    assert.equal(lv.ymd(lv.dayOf(2026, 10, 19)).dow, 1, "a Monday");
});

test("a row's value as a time: epoch millis, a date only, a wall clock text, or nothing", () => {
    assert.deepEqual(lv.parseTime(String(Date.UTC(2026, 9, 19, 15, 30)), CHICAGO), { day: lv.dayOf(2026, 10, 19), minutes: 630, dateOnly: false });
    assert.deepEqual(lv.parseTime("2026-10-21", CHICAGO), { day: lv.dayOf(2026, 10, 21), minutes: 0, dateOnly: true });
    assert.deepEqual(lv.parseTime("2026-10-21 08:15:00.0", CHICAGO), { day: lv.dayOf(2026, 10, 21), minutes: 495, dateOnly: false });
    assert.deepEqual(lv.parseTime("2026-10-21T08:15", CHICAGO), { day: lv.dayOf(2026, 10, 21), minutes: 495, dateOnly: false });
    assert.equal(lv.parseTime("", CHICAGO), null);
    assert.equal(lv.parseTime(null, CHICAGO), null);
    assert.equal(lv.parseTime("soon", CHICAGO), null);
    assert.equal(lv.timeOut(lv.dayOf(2026, 10, 19) * 1440 + 630, false, CHICAGO), String(Date.UTC(2026, 9, 19, 15, 30)));
    assert.equal(lv.timeOut(lv.dayOf(2026, 10, 19) * 1440 + 630, true, CHICAGO), "2026-10-19");
    assert.equal(lv.timeText(lv.dayOf(2026, 10, 19) * 1440 + 545, false), "2026-10-19 09:05");
    assert.equal(lv.timeText(lv.dayOf(2026, 10, 19) * 1440, true), "2026-10-19");
});

test("the events of a calendar: timed with an end or an hour long, all day by a date or the all-day field, moved and previewed", () => {
    const day = lv.dayOf(2026, 10, 19);
    const at = (h, m) => String(lv.millis(day, h * 60 + (m || 0), CHICAGO));
    const rows = [
        { id: "A", start: at(9), end: at(10, 30), name: "Standup", status: "S1", room: "R1" },
        { id: "B", start: at(13), end: null, name: "Lunch", room: "R2", room_display: "Room two" },
        { id: "C", start: "2026-10-21", end: "2026-10-22", name: "Offsite" },
        { id: "D", start: at(0), end: null, allDay: "Y", name: "Holiday" },
        { id: "E", start: null, name: "No date" },
        { _moquiRowType: "total" }
    ];
    const items = rows.map((row, index) => ({ index, row }));
    const cal = { startField: "start", endField: "end", titleField: "name", colorField: "status", allDayField: "allDay", resourceField: "room", colors: { S1: "warning" } };
    const events = lv.events(items, cal, CHICAGO, {}, null);
    assert.deepEqual(events.map((ev) => ev.row.id), ["D", "A", "B", "C"]);
    const byId = {};
    events.forEach((ev) => { byId[ev.row.id] = ev; });
    assert.deepEqual([byId.A.start, byId.A.end, byId.A.allDay], [day * 1440 + 540, day * 1440 + 630, false]);
    assert.equal(byId.B.end - byId.B.start, 60, "no end: an hour");
    assert.deepEqual([byId.C.allDay, byId.C.start, byId.C.end, byId.C.startDateOnly], [true, (day + 2) * 1440, (day + 4) * 1440, true], "the last day is in");
    assert.deepEqual([byId.D.allDay, byId.D.end - byId.D.start], [true, 1440]);
    assert.equal(byId.A.color, "warning");
    assert.ok(lv.palette.includes(byId.B.color) || byId.B.color === "grey-7");
    assert.equal(byId.A.resource, "R1");
    // moved, then a drag's preview on top
    const moved = lv.events(items, cal, CHICAGO, { 0: { start: day * 1440 + 600, end: day * 1440 + 690, resource: "R2" } }, { index: 1, start: day * 1440 + 900, end: day * 1440 + 930 });
    const movedA = moved.find((ev) => ev.row.id === "A");
    const previewB = moved.find((ev) => ev.row.id === "B");
    assert.deepEqual([movedA.start, movedA.end, movedA.resource], [day * 1440 + 600, day * 1440 + 690, "R2"]);
    assert.deepEqual([previewB.start, previewB.end], [day * 1440 + 900, day * 1440 + 930]);
});

test("overlapping events share a group's lanes; one after them starts a new group", () => {
    const places = lv.layoutDay([
        { id: "a", start: 540, end: 600 }, { id: "b", start: 570, end: 660 }, { id: "c", start: 600, end: 630 }, { id: "d", start: 700, end: 760 }
    ]);
    const of = (id) => places.find((place) => place.ev.id === id);
    assert.deepEqual([of("a").lane, of("a").lanes], [0, 2]);
    assert.deepEqual([of("b").lane, of("b").lanes], [1, 2]);
    assert.deepEqual([of("c").lane, of("c").lanes], [0, 2], "c takes a's lane once a ends");
    assert.deepEqual([of("d").lane, of("d").lanes], [0, 1]);
    assert.deepEqual(lv.layoutDay([]), []);
});

test("a month's weeks from the week of its first day, the first day of the week from the language, and shifts", () => {
    const anchor = lv.dayOf(2026, 10, 19);
    const sunday = lv.monthWeeks(anchor, 0);
    assert.equal(sunday.length, 5);
    assert.equal(lv.dayKey(sunday[0][0]), "2026-09-27");
    assert.equal(lv.dayKey(sunday[4][6]), "2026-10-31");
    const monday = lv.monthWeeks(anchor, 1);
    assert.equal(lv.dayKey(monday[0][0]), "2026-09-28");
    assert.equal(lv.dayKey(lv.weekStart(anchor, 1)), "2026-10-19");
    assert.equal(lv.dayKey(lv.shift(lv.dayOf(2026, 1, 31), "month", 1)), "2026-02-28");
    assert.equal(lv.dayKey(lv.shift(lv.dayOf(2026, 12, 15), "month", 1)), "2027-01-15");
    assert.equal(lv.dayKey(lv.shift(anchor, "week", -1)), "2026-10-12");
    assert.equal(lv.dayKey(lv.shift(anchor, "day", 1)), "2026-10-20");
    assert.equal(lv.snap(37, 15), 30);
    assert.equal(lv.snap(38, 15), 45);
});

test("a move posts the key fields, the transition's parameters and the new start and end as the row had them", () => {
    const day = lv.dayOf(2026, 10, 19);
    const cal = { startField: "fromDate", endField: "thruDate", resourceField: "facilityId", keyFields: ["workEffortId"], parameters: { projectId: "P1" } };
    const timed = { row: { workEffortId: "W1", facilityId: "R1" }, startDateOnly: false, endDateOnly: false, resource: "R1" };
    assert.deepEqual(lv.moveParams(timed, cal, CHICAGO, { start: day * 1440 + 600, end: day * 1440 + 660, resource: "R2" }), {
        projectId: "P1", workEffortId: "W1", fromDate: String(Date.UTC(2026, 9, 19, 15, 0)), thruDate: String(Date.UTC(2026, 9, 19, 16, 0)), facilityId: "R2"
    });
    // the same resource is not posted; a date-only event posts its first and last day
    const dated = { row: { workEffortId: "W2" }, startDateOnly: true, endDateOnly: true, resource: "R1" };
    assert.deepEqual(lv.moveParams(dated, cal, CHICAGO, { start: (day + 1) * 1440, end: (day + 3) * 1440, resource: "R1" }), {
        projectId: "P1", workEffortId: "W2", fromDate: "2026-10-20", thruDate: "2026-10-21"
    });
    // other parameter names
    const named = lv.moveParams(timed, Object.assign({}, cal, { endField: null, startParameter: "estimatedStartDate" }), "UTC", { start: day * 1440, end: day * 1440 + 60 });
    assert.deepEqual(Object.keys(named).sort(), ["estimatedStartDate", "projectId", "workEffortId"]);
});

test("the list views component's template compiles and declares the slots and props the macros write", () => {
    const comp = moqui.qvt.components["m-list-views"];
    assert.ok(comp, "m-list-views is a component");
    const props = Object.keys(comp.props);
    ["views", "rows", "board", "calendar", "formLocation", "reload", "view", "infoUrl"].forEach((name) => assert.ok(props.includes(name), name));
    assert.ok(/card_' \+ item\.index/.test(comp.template) || comp.template.indexOf("'card_' + item.index") >= 0);
    assert.ok(comp.template.indexOf('name="card"') >= 0);
});
