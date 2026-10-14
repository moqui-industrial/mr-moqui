/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// The client of server-static qvt (plans/QvtServerStatic.md) without a browser: the script the server sends for crates/moqui-core's
// Static/Items fixture is registered and run on the real Vue runtime with a fake DOM, against the package and the rows the server answered
// for the same fixture (testdata/Static/golden, checked by the Rust tests). The restricted ec, the scope chain, the helpers, the static form-list
// and the handler that loads the script are checked too.
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.static.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const webroot = path.join(__dirname, "..");
const golden = path.join(__dirname, "../../../../../../crates/moqui-core/src/screen/testdata/Static/golden");
vm.runInThisContext(fs.readFileSync(path.join(webroot, "libs/vue/vue.global.prod.js"), "utf8"));
globalThis.moqui = require("./MoquiLib.js");
globalThis.moqui.l10n = require("./MoquiL10n.js");
require("./WebrootVue.qvt.js");
const { Vue } = globalThis;
const qvt = moqui.qvt;

const goldenText = (name) => fs.readFileSync(path.join(golden, name), "utf8");
const scriptText = goldenText("Items.script.js");
const pkg = JSON.parse(goldenText("Items.package.json"));
const formPkg = JSON.parse(goldenText("Items.ItemList.json"));

// ---- a Vue renderer over plain objects ----

const nodeOps = {
    patchProp(el, key, prev, next) { if (next === null || next === undefined) delete el.props[key]; else el.props[key] = next; },
    insert(child, parent, anchor) {
        if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
        const at = anchor ? parent.children.indexOf(anchor) : parent.children.length;
        parent.children.splice(at, 0, child);
        child.parent = parent;
    },
    remove(child) { if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = null; },
    createElement(tag) { return { tag, props: {}, children: [], parent: null }; },
    createText(text) { return { text, parent: null }; },
    createComment(text) { return { comment: text, parent: null }; },
    setText(node, text) { node.text = text; },
    setElementText(el, text) { el.children = [{ text, parent: el }]; },
    parentNode(node) { return node.parent; },
    nextSibling(node) { const siblings = node.parent ? node.parent.children : []; return siblings[siblings.indexOf(node) + 1] || null; },
    querySelector() { return null; }
};
const renderer = Vue.createRenderer(nodeOps);

const html = (node) => {
    if (node.text !== undefined) return node.text;
    if (node.comment !== undefined) return "";
    const attrs = Object.keys(node.props).filter((key) => typeof node.props[key] !== "function")
        .map((key) => " " + key + "='" + (typeof node.props[key] === "object" ? JSON.stringify(node.props[key]) : String(node.props[key])) + "'").join("");
    return "<" + node.tag + attrs + ">" + node.children.map(html).join("") + "</" + node.tag + ">";
};

// the m-* wrappers are stubs that write their attributes; the form keeps its fields as the real one does
const leaf = (tag) => ({ inheritAttrs: false, render() { return Vue.h(tag, this.$attrs, this.$slots.default ? this.$slots.default() : undefined); } });
const stubs = {
    "m-form": {
        inheritAttrs: false, props: ["fieldsInitial"],
        data() { return { fields: Object.assign({}, this.fieldsInitial) }; },
        render() { return Vue.h("m-form", { name: this.$attrs.name, fields: this.fields }, this.$slots.default({ fields: this.fields, hasFieldsChanged: false })); }
    },
    "m-form-paginate": { props: ["paginate", "formList"], render() { captured.formList = this.formList; return Vue.h("m-form-paginate", { paginate: this.paginate }); } },
    "m-form-go-page": { props: ["idVal", "formList"], render() { return Vue.h("m-form-go-page"); } }
};
["m-display", "m-text-line", "m-drop-down", "m-link", "q-btn", "m-form-link", "m-container-box"].forEach((tag) => { stubs[tag] = leaf(tag); });

const captured = {};
const answers = {};
const requests = [];
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const setup = () => {
    requests.length = 0;
    moqui.request = (url, options) => {
        requests.push({ url, params: options && options.params });
        const body = answers[url];
        if (body === undefined) return Promise.reject({ status: 404, text: "no answer for " + url });
        return Promise.resolve({ ok: true, status: 200, text: JSON.stringify(body), json: body, headers: new Map() });
    };
    moqui.config = { userId: "EX_JOHN_DOE", username: "john.doe", locale: "en_US", timeZone: "UTC", userFullName: "John Doe",
        userAccount: { partyId: "P1", emailAddress: "j@example.com", currentPassword: "NOPE", passwordSalt: "NOPE" }, preferences: { QUASAR_DARK: "false" } };
    qvt.config = moqui.config;
    qvt.hostEc = null;
    moqui.l10n.setLocale("en_US");
    moqui.l10n.setTimeZone("UTC");
    moqui.l10n.setCatalog({});
    answers["/Items/actions"] = pkg;
    answers["/Items/actions/ItemList"] = formPkg;
};

const registerScript = () => {
    delete moqui.qvtStatic["/Items"];
    vm.runInThisContext(scriptText);
    return moqui.qvtStatic["/Items"];
};

const mountStatic = async (search) => {
    const factory = registerScript();
    const comp = qvt.staticComponent("/Items", factory, { path: "/Items", search: search || "", divId: "ItemsDiv" });
    const app = renderer.createApp(comp);
    Object.keys(qvt.components).forEach((name) => { if (!stubs[name]) app.component(name, qvt.components[name]); });
    Object.keys(stubs).forEach((name) => app.component(name, stubs[name]));
    const container = nodeOps.createElement("root");
    app.mount(container);
    await flush();
    await flush();
    return { app, container };
};

test("the script registers a render function and holds no request data", () => {
    const factory = registerScript();
    assert.equal(typeof factory, "function");
    assert.equal(typeof factory(Vue), "function");
    assert.ok(!/\bwith\s*\(/.test(scriptText) && !scriptText.includes("new Function") && !scriptText.includes("eval("));
    assert.ok(!scriptText.includes("ZEBRA"));
});

test("the screen is fetched from actions, and renders with the package and the form rows", async () => {
    setup();
    const { container } = await mountStatic("itemId=I9&page=2");
    assert.equal(requests[0].url, "/Items/actions");
    assert.deepEqual(requests[0].params, { itemId: "I9", page: "2" });
    assert.equal(requests[1].url, "/Items/actions/ItemList");
    const text = html(container);
    // the label has the package's greeting through ec.resource.expand with its expression table
    assert.match(text, /<h4 class='text-inline' textContent='Hello ZEBRA greeting for Ada Lovelace'><\/h4>/);
    // the container div has the id of its place
    assert.match(text, /<div id='ItemsDiv'>/);
    // the form-single reads its map: the item
    assert.match(text, /<m-form name='ItemForm' fields=/);
    assert.match(text, /"name":"ZEBRA Widget"/);
    assert.match(text, /"itemId":"ZEBRA-I1"/);
    // the decimal price of the item is formatted from text; the conditional field picks the branch that holds
    assert.match(text, /"price_display":"1,234.50"/);
    assert.match(text, /"mode_display":"is active: ZEBRA Widget"/);
    // section-iterate pushes the entry: a v-for over tags with the entry and the form's map
    assert.match(text, /textContent='Tag ZEBRA-t1 of ZEBRA Widget'/);
    assert.match(text, /textContent='Tag ZEBRA-t2 of ZEBRA Widget'/);
    // the link has its path and its parameter from the scope
    assert.match(text, /href='\/Items\/viewItem\?itemId=ZEBRA-I1'/);
    // each row of the form-list pushes the row on the scope: its own fields start the row form
    assert.match(text, /"itemId":"ZEBRA-R1","itemId_display":"ZEBRA-R1","name":"ZEBRA row one"/);
    assert.match(text, /"itemId":"ZEBRA-R2"/);
    assert.match(text, /"price_display":"12.50"/);
    // the column titles are localized in the browser
    assert.match(text, /textContent='Item ID'/);
});

test("paging and sorting a static form-list change the request and fetch again", async () => {
    setup();
    answers["/Items/actions/ItemList"] = Object.assign({}, formPkg, { itemListCount: 45, itemListPageIndex: 1, itemListPageSize: 20, itemListPageMaxIndex: 2, itemListPageRangeLow: 21, itemListPageRangeHigh: 40 });
    const { container } = await mountStatic("itemGroup=G1");
    // the page keys of the answer are the paginate of the page buttons
    assert.match(html(container), /<m-form-paginate paginate='\{"count":45,"pageIndex":1,"pageSize":20,"pageMaxIndex":2,"pageRangeLow":21,"pageRangeHigh":40\}'/);
    const list = captured.formList;
    assert.ok(list, "the page buttons got the form-list");
    assert.equal(requests.length, 2);
    list.setPageIndex(1);
    await flush();
    assert.equal(requests.length, 3);
    assert.equal(requests[2].url, "/Items/actions/ItemList");
    assert.deepEqual(requests[2].params, { itemGroup: "G1", pageIndex: 1 });
    list.sortBy("-itemId");
    await flush();
    assert.deepEqual(requests[3].params, { itemGroup: "G1", pageIndex: 0, orderByField: "-itemId" });
    assert.equal(list.orderBy("-itemId"), "desc");
    assert.equal(list.orderBy("itemId"), "");
    // the screen package is not asked for again
    assert.equal(requests.filter((request) => request.url === "/Items/actions").length, 1);
});

test("the restricted ec has the table of members and throws on anything else", () => {
    setup();
    const ec = qvt.makeEc(moqui.config);
    assert.equal(ec.user.username, "john.doe");
    assert.equal(ec.user.userId, "EX_JOHN_DOE");
    assert.equal(ec.user.locale, "en_US");
    assert.equal(ec.user.timeZone, "UTC");
    assert.equal(ec.user.userAccount.partyId, "P1");
    assert.equal(ec.user.userAccount.userFullName, "John Doe");
    assert.ok(!("currentPassword" in ec.user.userAccount) && !("passwordSalt" in ec.user.userAccount), "credential columns stay on the server");
    assert.equal(ec.user.getPreference("QUASAR_DARK"), "false");
    assert.equal(ec.user.getPreference("NO_SUCH"), null);
    assert.equal(ec.l10n.format("1234.125", "#,##0.00"), "1,234.12");
    assert.equal(ec.l10n.format("2024-03-05 14:07:00.0", "yyyy-MM-dd HH:mm"), "2024-03-05 14:07");
    assert.equal(ec.l10n.formatCurrency("1234.5", "USD"), "$1,234.50");
    assert.equal(ec.l10n.formatCurrencyNoSymbol("1234.5", "USD"), "1,234.50");
    assert.equal(ec.l10n.localize("Plain"), "Plain");
    // anything else throws with a message that says what to do
    for (const area of ["entity", "web", "context", "service", "message", "screen", "logger"]) {
        assert.throws(() => ec[area], new RegExp("ec\\." + area + " is not available in a server-static qvt screen"));
    }
    assert.throws(() => ec.entity.find("A"), /ec\.entity is not available/);
    assert.equal(ec.then, undefined);
});

test("the browser utilities give what the server's do", () => {
    setup();
    // the same vectors as moqui_lib::string::camel_case_to_pretty
    assert.equal(qvt.StringUtilities.camelCaseToPretty("orderId"), "Order ID");
    assert.equal(qvt.StringUtilities.camelCaseToPretty("foo.bar"), "Foo Bar");
    assert.equal(qvt.StringUtilities.camelCaseToPretty("productStoreId"), "Product Store ID");
    assert.equal(qvt.StringUtilities.camelCaseToPretty("toFieldNameAlias"), "To Field Name Alias");
    assert.equal(qvt.StringUtilities.camelCaseToPretty("aaAa"), "Aa");
    assert.equal(qvt.StringUtilities.camelCaseToPretty("item2Id"), "Item 2 ID");
    assert.equal(qvt.StringUtilities.camelCaseToPretty("#hash"), "Hash");
    assert.equal(qvt.StringUtilities.camelCaseToPretty(""), "");
    assert.equal(qvt.StringUtilities.camelCaseToPretty(null), "");
    assert.equal(qvt.makeEc({ moquiVersion: "1.2.3" }).factory.moquiVersion, "1.2.3");
});

test("ec.resource.expand localizes the pattern, then reads the expressions from the table", () => {
    setup();
    moqui.l10n.setCatalog({ "Total ${sum} for ${who}": "Summe ${sum} fuer ${who}" });
    const ec = qvt.makeEc(moqui.config);
    const table = { sum: () => 12.5, who: () => "Ada" };
    assert.equal(ec.resource.expand("Total ${sum} for ${who}", table), "Summe 12.5 fuer Ada");
    assert.equal(ec.resource.expandNoL10n("Total ${sum} for ${who}", table), "Total 12.5 for Ada");
    assert.equal(ec.resource.expand("no expression"), "no expression");
    assert.equal(ec.resource.expand("price $"), "price $");
    assert.equal(ec.resource.expand("a ${x} b", { x: () => null }), "a  b");
    // an expression that the template did not carry cannot be evaluated: no eval
    assert.throws(() => ec.resource.expand("Total ${other}", table), /not part of this screen's template/);
    // braces and quotes inside an expression are parsed
    assert.equal(ec.resource.expand("v ${ a.b('}') }", { "a.b('}')": () => "ok" }), "v ok");
    moqui.l10n.setCatalog({});
});

test("a scope looks in the extra map, the map, then the scope around it, and $scope is the scope", () => {
    const outer = qvt.makeScope({ a: 1, b: 2, shared: "outer" }, null);
    const inner = qvt.makeScope({ b: 20, shared: "inner" }, outer, { shared: "extra" });
    assert.equal(inner.a, 1);
    assert.equal(inner.b, 20);
    assert.equal(inner.shared, "extra");
    assert.equal(outer.shared, "outer");
    assert.equal(inner.$scope, inner);
    assert.equal(inner["not here"], undefined);
    assert.ok("a" in inner && !("zzz" in inner));
    const lazy = qvt.makeScope(() => ({ q: 5 }), outer);
    assert.equal(lazy.q, 5);
    inner.b = 21;
    assert.equal(inner.b, 21);
    inner.a = 9;
    assert.equal(outer.a, 9);
});

test("the helpers a template calls", () => {
    const mq = qvt.helpers;
    assert.equal(mq.str(null), "");
    assert.equal(mq.str(0), "0");
    assert.equal(mq.dflt("", "x"), "x");
    assert.equal(mq.dflt("v", "x"), "v");
    assert.equal(mq.nul(""), null);
    assert.deepEqual(mq.cur("A,B", true, ""), ["A", "B"]);
    assert.equal(mq.cur("A,B", false, ""), "A");
    assert.equal(mq.cur("[A, B]", false, ""), "A");
    assert.equal(mq.cur(null, false, "N/A"), "N/A");
    assert.deepEqual(mq.cur(["x"], true, ""), ["x"]);
    assert.equal(mq.fmt("1234.5", "#,##0.00"), "1,234.50");
    assert.equal(mq.fmt("2024-03-05", "MM/dd/yyyy"), "03/05/2024");
    assert.equal(mq.fmt("2024-03-05 14:07:09.0", "yyyy-MM-dd HH:mm", "date-time"), "2024-03-05 14:07");
    assert.equal(mq.fmt("12:30:00", "HH:mm"), "12:30");
    assert.equal(mq.fmt("", "#"), "");
    assert.equal(mq.url("/a/b", { x: "1 2", y: null, z: "" }, { m: "q&r" }), "/a/b?m=q%26r&x=1%202");
    assert.equal(mq.url("/a", {}, null), "/a");
    assert.deepEqual(mq.shown("B,C", "C"), { display: "" });
    assert.deepEqual(mq.shown("B", "A"), { display: "none" });
});

test("the handler loads the script with a script element and builds a component for each use", async () => {
    setup();
    const added = [];
    globalThis.document = {
        createElement: () => ({ remove() {} }),
        head: { appendChild: (el) => { added.push(el); setTimeout(() => { vm.runInThisContext(scriptText); el.onload(); }, 0); } }
    };
    delete moqui.qvtStatic["/Items"];
    delete moqui.qvtStaticTags["/Items"];
    const headers = new Map([["ETag", 'W/"abc"'], ["X-Moqui-Qvt", "static-template"], ["Cache-Control", "max-age=86400"]]);
    const made = await moqui.qvtHandlers["static-template"]("(script text is not evaluated)", { path: "/Items", headers });
    assert.equal(added.length, 1);
    assert.equal(added[0].src, "/Items.qvt");
    const comp = made.mqStatic({ path: "/Items", search: "a=1" });
    assert.equal(typeof comp.created, "function");
    // the same validator does not load the script again
    await moqui.qvtHandlers["static-template"]("", { path: "/Items", headers });
    assert.equal(added.length, 1);
    // another validator does
    await moqui.qvtHandlers["static-template"]("", { path: "/Items", headers: new Map([["ETag", 'W/"def"']]) });
    assert.equal(added.length, 2);
    // a script that does not register is an error
    delete moqui.qvtStatic["/Missing"];
    globalThis.document.head.appendChild = (el) => { setTimeout(() => el.onload(), 0); };
    await assert.rejects(moqui.qvtHandlers["static-template"]("", { path: "/Missing", headers: new Map() }), /did not register/);
    globalThis.document.head.appendChild = (el) => { setTimeout(() => el.onerror(), 0); };
    await assert.rejects(moqui.qvtHandlers["static-template"]("", { path: "/Other", headers: new Map() }), /Could not load/);
    delete globalThis.document;
});

test("loadComponent gives each use its own component from a cached static template", async () => {
    setup();
    globalThis.document = {
        createElement: () => ({ remove() {} }),
        head: { appendChild: (el) => { setTimeout(() => { vm.runInThisContext(scriptText); el.onload(); }, 0); } }
    };
    delete moqui.qvtStatic["/Items"];
    delete moqui.qvtStaticTags["/Items"];
    moqui.componentCache.clear && moqui.componentCache.clear();
    const headers = new Map([["ETag", 'W/"abc"'], ["X-Moqui-Qvt", "static-template"], ["Cache-Control", "max-age=86400, must-revalidate, public"]]);
    moqui.request = () => Promise.resolve({ ok: true, status: 200, text: "(script)", json: undefined, headers });
    const load = (info) => new Promise((resolve) => moqui.loadComponent(info, resolve, "dc"));
    const first = await load({ path: "/Items", search: "a=1" });
    const second = await load({ path: "/Items", search: "a=2" });
    assert.notEqual(first, second, "a component for each use");
    assert.equal(first.name, "mStaticScreen");
    delete globalThis.document;
});

test("a page on the runtime build refuses a merged fragment", () => {
    qvt.config = { qvt: { build: "runtime" } };
    assert.throws(() => moqui.qvtHandlers["merged-fragment"]("<p/>", { divId: "x" }), /runtime build/);
    qvt.config = { qvt: { build: "compiler" } };
    assert.equal(moqui.qvtHandlers["merged-fragment"]("<p/>", { divId: "x" }).template, '<div id="x"><p/></div>');
});
