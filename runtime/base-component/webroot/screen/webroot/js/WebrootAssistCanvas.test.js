/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of the Assist canvas (WebrootAssistCanvas.js) that need no browser: the library spec golden (openui-library.spec.json
// is what the component declarations make), the signatures the skill quotes, the vendored lang-core parsing valid and invalid
// fixtures against that spec, the state reconciliation of a program that changes under the user's edits, the read proxy's
// parameters, the problems a render reports, the chart configurations, and the drawer's canvas state (canvas.open,
// canvas.delta, a snapshot). The canvas in a page is the Chromium smoke of Gate 3 (crates/moqui-host/tests/gate3_browser.rs).
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootAssistCanvas.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const webroot = path.join(__dirname, "..");
globalThis.moqui = require("./MoquiLib.js");
const assist = require("./WebrootAssist.js");
const canvas = require("./WebrootAssistCanvas.js");
vm.runInThisContext(fs.readFileSync(path.join(webroot, "libs/openui/lang-core.min.js"), "utf8"));
const O = globalThis.OpenUILang;
const spec = JSON.parse(fs.readFileSync(path.join(__dirname, "openui-library.spec.json"), "utf8"));

test("the library spec is what the component declarations make (UPDATE: node -e the librarySpecText)", () => {
    const made = canvas.librarySpecText();
    const kept = fs.readFileSync(path.join(__dirname, "openui-library.spec.json"), "utf8");
    assert.equal(kept, made, "openui-library.spec.json is stale: write canvas.librarySpecText() to it");
    assert.equal(Object.keys(spec.$defs).length, 46);
    assert.deepEqual(Object.keys(spec.$defs.Select.properties), ["name", "value", "items", "label"]);
    assert.deepEqual(spec.$defs.Select.required, ["name", "items"]);
    assert.equal(canvas.signature("Select"), "Select(name, value?, items, label?)");
    assert.equal(canvas.signature("Lookup"), "Lookup(name, value?, optionsUrl, valueField?, labelField?, dependsOn?, label?)");
    // every component is drawn, and every group is known
    canvas.library.forEach((def) => assert.ok(canvas.GROUPS.includes(def.group), def.name));
});

test("the skill gives every signature", () => {
    const skill = fs.readFileSync(path.join(webroot, "../../../framework/skill/openui-lang.md"), "utf8");
    canvas.library.forEach((def) => assert.ok(skill.includes(canvas.signature(def.name)), canvas.signature(def.name)));
});

test("lang-core parses valid programs against the spec and reports invalid ones", () => {
    const parser = O.createParser(spec, "Stack");
    const valid = parser.parse([
        "root = Stack([CardHeader(\"Hits\"), chart])",
        "hits = Query(\"request\", {path: \"/apps/system/ArtifactHitSummary\", query: {artifactType: $type}}, {rows: []})",
        "chart = BarChart(hits.rows.artifactName, [Series(\"Hits\", hits.rows.hitCount)])",
    ].join("\n"));
    assert.equal(valid.root.typeName, "Stack");
    assert.deepEqual(valid.meta.errors, []);
    assert.deepEqual(valid.queryStatements.map((q) => q.statementId), ["hits"]);
    assert.deepEqual(valid.stateDeclarations, { $type: null });
    // a Query whose options are not closed swallows the next line
    const open = parser.parse("root = Stack([t])\nq = Query(\"request\", {path: \"/apps/x\", query: {a: $a}, {rows: []})\nt = Table([Col(\"A\", q.rows.a)])");
    assert.equal(open.meta.incomplete, true);
    assert.ok(open.meta.unresolved.includes("t"));
    // an unknown component and a child that is not an array
    const bad = parser.parse("root = Stack([Chartt([1]), Card(Table([]))])");
    const codes = bad.meta.errors.map((error) => error.code).sort();
    assert.deepEqual(codes, ["type-mismatch", "unknown-component"]);
    // the streaming parser draws a program as it comes
    const streaming = O.createStreamingParser(spec, "Stack");
    const part = streaming.set("root = Stack([CardHeader(\"Hi\"), more])\nmore = TextContent(\"x");
    assert.equal(part.root.typeName, "Stack");
    // the patch merge of the server and the page
    const merged = O.mergeStatements("root = Stack([a, b])\na = TextContent(\"1\")\nb = TextContent(\"2\")", "b = null\nroot = Stack([a])");
    assert.equal(merged, "root = Stack([a])\na = TextContent(\"1\")");
});

test("state reconciliation: a changed default replaces only what the user left alone", () => {
    // the user typed into $q; $days still holds the old default; the program changes both defaults
    const prev = { $q: "", $days: "7" };
    const decls = { $q: "x", $days: "14" };
    const persisted = canvas.persistedState(decls, prev, { q: "typed", days: "7" });
    assert.deepEqual(persisted, { $q: "typed", $days: "14" });
    // the first parse keeps what the page had
    assert.deepEqual(canvas.persistedState({ $q: "" }, null, { q: "kept" }), { $q: "kept" });
    // edits that only hold a default, or of a name the program dropped, no longer count
    const edited = canvas.reconcileUserEdits({ $q: true, $days: true, $gone: true }, prev, decls, { $q: "typed", $days: "14" });
    assert.deepEqual(edited, { $q: true });
    assert.equal(canvas.sameLiteral({ value: 3 }, "3"), true);
    assert.deepEqual(canvas.valuesOf({ $q: "a", form: { name: { value: "n", componentType: "Input" } }, __openui_loading: [] }), { q: "a", name: "n" });
});

test("reads go to the proxy with GET only, and paths of the apps only", () => {
    assert.deepEqual(canvas.readParams("A1", { path: "/apps/system/Visit/VisitList", list: "VisitList", query: { q: "x", empty: "", n: 3 } }),
        { artifactId: "A1", path: "/apps/system/Visit/VisitList", list: "VisitList", query: "{\"q\":\"x\",\"n\":\"3\"}" });
    assert.ok(canvas.readParams("A1", { method: "POST", path: "/apps/x" }).error);
    assert.ok(canvas.readParams("A1", { path: "https://example.com/" }).error);
    assert.ok(canvas.readParams("A1", { path: "/rest/s1/x" }).error);
    assert.equal(canvas.siteImage("/static/a.png"), "/static/a.png");
    assert.equal(canvas.siteImage("//evil/a.png"), null);
    assert.equal(canvas.siteImage("https://evil/a.png"), null);
    // never a screen, a transition or a call of the apps (a GET of one runs it with the cookie), never a query
    assert.equal(canvas.siteImage("/apps/x/deleteThing.png"), null);
    assert.equal(canvas.siteImage("/rest/s1/x.png"), null);
    assert.equal(canvas.siteImage("/static/a.png?d=x"), null);
    assert.equal(canvas.siteImage("/static/a"), null);
    // no url() or other style reaches a component's size
    assert.equal(canvas.cssLength("320px", "300px"), "320px");
    assert.equal(canvas.cssLength("1px;background:url(https://evil/)", "300px"), "300px");
});

test("the problems of a render are each once, and the screen's refusals are errors", () => {
    const problems = canvas.problemsOf({ root: null, meta: { errors: [{ code: "unknown-component", message: "Unknown X", statementId: "root" }], unresolved: ["t", "t"] } },
        [{ code: "runtime-error", message: "bad" }], [{ source: "query", code: "tool-error", message: "no list", statementId: "q" }]);
    assert.deepEqual(problems.map((one) => one.code), ["unknown-component", "unresolved", "no-root", "runtime-error", "tool-error"]);
    const refused = canvas.responseProblems({ validationErrors: [{ message: "is required", field: "description" }], messageInfos: [{ type: "warning", message: "careful" }] });
    assert.deepEqual(refused, { errors: ["is required (for field description)"], warnings: ["careful"] });
});

test("charts: Chart.js 4 configurations from the series, numbers as a list shows them", () => {
    const bar = canvas.xyChart("bar", { labels: ["a", "b"], series: [{ type: "element", props: { category: "Hits", values: ["1,200", "3"] } }], variant: "stacked", yLabel: "Hits" });
    assert.deepEqual(bar.data, { labels: ["a", "b"], datasets: [{ label: "Hits", data: [1200, 3], backgroundColor: "#1976d2", borderColor: "#1976d2" }] });
    assert.equal(bar.options.scales.x.stacked, true);
    assert.equal(bar.options.scales.y.title.text, "Hits");
    const line = canvas.xyChart("line", { labels: [1], series: [{ category: "A", values: [2] }], variant: "step" }, true);
    assert.equal(line.data.datasets[0].stepped, true);
    assert.equal(line.data.datasets[0].fill, true);
    const pie = canvas.pieChart({ labels: ["x", "y", "z"], values: [1, 2], variant: "donut" });
    assert.equal(pie.type, "doughnut");
    assert.deepEqual(pie.data.labels, ["x", "y"]);
    assert.equal(canvas.dateValue("2026-10-22 10:00", "date-time"), "2026-10-22 10:00");
    assert.match(canvas.dateValue("now", "date"), /^\d{4}-\d{2}-\d{2}$/);
});

test("the drawer's canvas: canvas.open replaces it, canvas.delta streams a whole program, a snapshot keeps what the tab did", () => {
    const state = assist.emptyState();
    state.conversationId = "C1";
    state.activeRunId = "R1";
    state.seq.R1 = 0;
    const ctx = { clientId: "tab-a", followOn: true };
    // a whole program streams in: drawn as it comes
    let effects = assist.apply(state, "R1", { seq: null, type: "canvas.delta", payload: { messageId: "M1", index: 0, patch: false, offset: 0, text: "root = Stack([" } }, ctx);
    assert.deepEqual(effects, [{ kind: "canvas" }]);
    assist.apply(state, "R1", { seq: null, type: "canvas.delta", payload: { messageId: "M1", index: 0, offset: 14, text: "a])" } }, ctx);
    assert.equal(assist.canvasView(state).program, "root = Stack([a])");
    assert.equal(assist.canvasView(state).streaming, true);
    effects = assist.apply(state, "R1", { seq: 1, type: "canvas.open", payload: { artifactId: 7, version: 1, title: "Hits", kind: "openui", program: "root = Stack([a])\na = TextContent(\"x\")", problems: [] } }, ctx);
    assert.deepEqual(effects, [{ kind: "canvas" }]);
    assert.equal(state.canvasDraft, null);
    const view = assist.canvasView(state);
    assert.deepEqual([view.artifactId, view.version, view.streaming, view.title], ["7", 1, false, "Hits"]);
    // the tab hides it; the same version from a snapshot stays hidden, a newer one shows
    state.canvas.hidden = true;
    assist.takeCanvas(state, { artifactId: "7", version: 1, title: "Hits", program: "x" });
    assert.equal(state.canvas.hidden, true);
    assist.takeCanvas(state, { artifactId: "7", version: 0, title: "Old", program: "y" });
    assert.equal(state.canvas.title, "Hits", "an older version does not replace a newer one");
    assist.takeCanvas(state, { artifactId: "7", version: 2, title: "Hits", program: "z" });
    assert.equal(state.canvas.hidden, false);
    // a patch streams with no drawing: the canvas shows that it updates
    assist.apply(state, "R1", { seq: null, type: "canvas.delta", payload: { messageId: "M2", index: 0, patch: true, offset: 0, text: "a = null" } }, ctx);
    assert.equal(assist.canvasView(state).program, "z");
    assert.equal(assist.canvasView(state).updating, true);
    // a closed canvas is not drawn
    state.canvasDraft = null;
    state.canvas.closed = true;
    assert.equal(assist.canvasView(state), null);
    assist.snapshot(state, { conversation: { conversationId: "C1" }, items: [], canvas: null }, ctx);
    assert.equal(state.canvas, null);
});

test("the dialect registers itself", () => {
    assert.equal(typeof assist.dialects.register, "function");
    assert.equal(assist.dialects.get("vue-sfc"), null);
});

test("a Mutation runs only from a trusted click, and its Button says what it saves", () => {
    const plan = { steps: [{ type: "set", target: "$a" }, { type: "run", statementId: "save", refType: "mutation" }, { type: "run", statementId: "q", refType: "query" }] };
    assert.deepEqual(canvas.mutationSteps(plan).map((step) => step.statementId), ["save"]);
    assert.deepEqual(canvas.mutationSteps({ steps: [{ type: "run", statementId: "q", refType: "query" }] }), []);
    assert.equal(canvas.savesText("/apps/assistgate/Edit/updateEnumType"), "updateEnumType");
    const active = { userActivation: { isActive: true } };
    assert.equal(canvas.trustedClick({ isTrusted: true }, active), true);
    assert.equal(canvas.trustedClick({ isTrusted: false }, active), false, "a scripted click");
    assert.equal(canvas.trustedClick({ isTrusted: true }, { userActivation: { isActive: false } }), false);
    assert.equal(canvas.trustedClick(null, active), false, "a refresh or a render has no click");
    // the renderer has no other way to a Mutation: a Query's arguments go to the read proxy
    const source = fs.readFileSync(path.join(__dirname, "WebrootAssistCanvas.js"), "utf8");
    assert.doesNotMatch(source, /__mutation|this\.activation/);
    assert.match(source, /args\[RUN\]/);
});
