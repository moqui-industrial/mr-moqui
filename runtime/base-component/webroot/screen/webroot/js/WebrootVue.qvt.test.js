/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of WebrootVue.qvt.js that need no browser: every component template compiles with the Vue compiler, the router
// stand-in answers Quasar's calls, and loadComponent chooses by the X-Moqui-Qvt header.
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const webroot = path.join(__dirname, "..");
globalThis.moqui = require("./MoquiLib.js");
globalThis.moqui.l10n = require("./MoquiL10n.js");
vm.runInThisContext(fs.readFileSync(path.join(webroot, "libs/vue/compiler-dom.cjs.js"), "utf8"));
require("./WebrootVue.qvt.js");
const compiler = globalThis.VueCompilerDOM;
const qvt = moqui.qvt;

test("every component has a template that compiles", () => {
    const names = Object.keys(qvt.components);
    assert.ok(names.length >= 38, "components: " + names.length);
    names.forEach((name) => {
        const template = qvt.components[name].template;
        if (!template) return;
        const errors = [];
        compiler.compile(template, { mode: "function", onError: (err) => errors.push(err.message) });
        assert.deepEqual(errors, [], name);
    });
});

test("the components the macros write are all defined", () => {
    const used = new Set();
    for (const file of ["ScreenQvt.js", "ScreenQvtStatic.js"]) {
        const macros = fs.readFileSync(path.join(__dirname, "../../../../framework/template/screen/" + file), "utf8");
        for (const match of macros.matchAll(/<(m-[a-z-]+)/g)) used.add(match[1]);
    }
    // m-box-body and the like are written through a string that holds the whole tag
    assert.ok(used.size >= 20, "tags: " + used.size);
    used.forEach((name) => assert.ok(qvt.components[name], name + " is written by ScreenQvt.js but not defined"));
});

test("the shell mounts: links, dark mode, and the page address", () => {
    const options = qvt.rootOptions({ basePath: "/apps", linkBasePath: "/qapps", appRootPath: "" });
    const link = options.methods.getLinkPath;
    const vmLike = { appRootPath: "", linkBasePath: "/qapps" };
    assert.equal(link.call(vmLike, "/apps/System/Foo?a=1"), "/qapps/System/Foo?a=1");
    assert.equal(link.call(vmLike, "/qapps/System"), "/qapps/System");
    assert.equal(link.call({ appRootPath: "/moqui", linkBasePath: "/moqui/qapps" }, "/apps/x"), "/moqui/qapps/x");
    assert.equal(link.call(vmLike, { path: "/apps/a", query: { b: "1 2" } }), "/qapps/a?b=1%202");
    const data = options.data();
    assert.equal(data.basePath, "/apps");
    assert.equal(options.computed.basePathSize.call({ basePath: "/moqui/apps", appRootPath: "/moqui" }), 1);
    assert.equal(options.computed.basePathSize.call({ basePath: "/apps", appRootPath: "" }), 1);
    // the current path text is the path list, then the extra path
    const cp = options.computed.currentPath;
    assert.equal(cp.get.call({ basePath: "/apps", currentPathList: ["System", "Foo"], extraPathList: ["x"] }), "/apps/System/Foo/x");
    const target = { basePath: "/apps", linkBasePath: "/qapps", currentPathList: [] };
    cp.set.call(target, "/qapps/System/Foo/");
    assert.deepEqual(target.currentPathList, ["System", "Foo"]);
    cp.set.call(target, "/apps/Bar");
    assert.deepEqual(target.currentPathList, ["Bar"]);
});

test("theme footer columns have a full-width row as their immediate parent", () => {
    // Quasar's .row > .col-md-6 selector needs a DOM parent, not display:contents.
    assert.match(qvt.components["m-qapps-root"].template,
        /<div v-html="shell.footerHtml" class="row full-width"><\/div>/);
});

test("the router stand-in resolves what Quasar asks and pushes through setUrl", async () => {
    const router = qvt.makeRouter();
    moqui.webrootVue = { getLinkPath: (p) => p.replace("/apps", "/qapps"), setUrl: (location) => { moqui.pushed = location; } };
    const resolved = router.resolve("/apps/System/Foo?x=1");
    assert.equal(resolved.href, "/qapps/System/Foo?x=1");
    assert.equal(resolved.matched.length, 1);
    await router.push("/apps/Other");
    assert.equal(moqui.pushed, "/apps/Other");
    delete moqui.webrootVue;
});

test("a merged fragment is the template in a div, with script and link made into components", () => {
    const comp = moqui.mergedFragmentComponent('<p>x</p><script>a()</script><link href="a.css">', "Dyn");
    assert.equal(comp.template, '<div id="Dyn"><p>x</p><m-script>a()</m-script><m-stylesheet href="a.css"></m-stylesheet></div>');
    assert.equal(moqui.mergedFragmentComponent("<b/>", "").template, "<div><b/></div>");
    const errors = [];
    compiler.compile(comp.template, { onError: (err) => errors.push(err.message) });
    assert.deepEqual(errors, []);
});

const answer = (text, headers) => Promise.resolve({ ok: true, status: 200, text, json: undefined, headers: new Map(Object.entries(headers || {})) });

test("loadComponent picks the handler from X-Moqui-Qvt and caches by Cache-Control", async () => {
    const requests = [];
    const original = moqui.request;
    moqui.request = (url) => {
        requests.push(url);
        if (url.indexOf(".qvt") > 0) return answer("<p>merged</p>", { "X-Moqui-Qvt": "merged-fragment", "Cache-Control": "private, no-store" });
        if (url.indexOf(".qjs") > 0) return answer("{ data: function () { return { n: 1 }; }, template: '<i/>' }", { "Cache-Control": "public, max-age=60" });
        return answer("<template><b>{{n}}</b></template><script>module.exports = { data() { return { n: 2 }; } }</script>");
    };
    const load = (info) => new Promise((resolve) => moqui.loadComponent(info, resolve, "id1"));
    const merged = await load({ path: "/apps/A" });
    assert.equal(merged.template, '<div id="id1"><p>merged</p></div>');
    assert.deepEqual(requests, ["/apps/A.qvt"]);
    const js = await load({ path: "/apps/B", renderModes: ["qjs"], search: "a=1" });
    assert.equal(js.data().n, 1);
    assert.equal(requests[1], "/apps/B.qjs?a=1");
    // max-age puts the component in the cache by path
    const again = await load({ path: "/apps/B", renderModes: ["qjs"] });
    assert.equal(again, js);
    assert.equal(requests.length, 2);
    const sfc = await load({ path: "/apps/C", renderModes: ["qvue"] });
    assert.equal(sfc.template, "<b>{{n}}</b>");
    assert.equal(sfc.data().n, 2);
    assert.equal(requests[2], "/apps/C.qvue");
    // the handler of a server-static template is installed (qvt.static tests run it), and a handler can be replaced
    assert.equal(typeof moqui.qvtHandlers["static-template"], "function");
    moqui.qvtHandlers["static-template"] = (text) => ({ template: "static:" + text });
    moqui.request = () => answer("<p/>", { "X-Moqui-Qvt": "static-template" });
    const stat = await load({ path: "/apps/E" });
    assert.equal(stat.template, "static:<p/>");
    moqui.request = original;
});

test("a message list notifies once for each item and says whether it did", () => {
    const seen = [];
    moqui.webrootVue = { $q: { notify: (options) => seen.push(options) }, addNotify() {} };
    assert.equal(moqui.notifyMessages(null, null, null), false);
    assert.equal(moqui.notifyMessages(["one", { message: "two", type: "success" }], "bad"), true);
    assert.deepEqual(seen.map((o) => [o.message, o.type]), [["one", "info"], ["two", "positive"], ["bad", "negative"]]);
    delete moqui.webrootVue;
});

test("the QZ Tray component compiles and defines the print functions", () => {
    globalThis.qz = { websocket: { isActive: () => false }, security: {}, printers: {}, api: {}, configs: {}, print() {} };
    require("./MoquiQzComponent.js");
    const errors = [];
    compiler.compile(moqui.qzComponent.template, { mode: "function", onError: (err) => errors.push(err.message) });
    assert.deepEqual(errors, []);
    ["printUrlMain", "printUrlLabel", "printUrlFile", "getQzConfig", "isQzActive"].forEach((name) => assert.equal(typeof moqui[name], "function", name));
    delete globalThis.qz;
});

test("an m-form keeps its own field errors and leaves the rest to notify", () => {
    const options = qvt.components["m-form"];
    const form = options.data.call({ fieldsInitial: { moquiFormName: "EditThing", title: "x", note: "" } });
    form.setFieldErrors = options.methods.setFieldErrors.bind(form);
    const resp = {
        errors: "Title needs a letter\nOther failure",
        validationErrors: [
            { form: "EditThing", field: "title", message: "Title needs a letter", fieldPretty: "Title" },
            { form: "OtherForm", field: "title", message: "Other form" },
            { field: "missing", message: "No such field" }
        ]
    };
    const rest = moqui.placeFieldErrors(form, resp);
    assert.deepEqual(form.fieldErrors, { title: "Title needs a letter" });
    assert.equal(form.fieldErrorValues.title, "x");
    assert.equal(rest.errors, "Other failure");
    assert.deepEqual(rest.validationErrors.map((ve) => ve.message), ["Other form", "No such field"]);
    assert.equal(resp.validationErrors.length, 3, "the response itself is not changed");
    // a changed value drops its error
    form.fields.title = "xy";
    options.watch.fields.handler.call(form);
    assert.deepEqual(form.fieldErrors, {});
    // nothing placed is the same response
    assert.equal(moqui.placeFieldErrors(form, { errors: "e" }).errors, "e");
    // the server writes a validation error's line as Java's toStringPretty; a placed one's line goes too
    const pretty = { field: "title", message: "Field cannot be empty", fieldPretty: "Title", serviceName: "save#Thing", serviceNamePretty: "Save#Thing" };
    assert.equal(moqui.validationErrorText(pretty), "Field cannot be empty(for field Title of service Save#Thing");
    const placedPretty = moqui.placeFieldErrors(form, { errors: "Field cannot be empty(for field Title of service Save#Thing\nOther", validationErrors: [pretty] });
    assert.equal(placedPretty.errors, "Other");
});

test("a tree node draws the children it loads, is a leaf on false, and keeps the ones that came inline", async () => {
    const options = qvt.components["m-tree-item"];
    assert.match(options.template, /v-for="child in children"/, "the list drawn is the children computed, not model.children (the server's boolean)");
    const node = (model, loadedChildren, top) => {
        const self = { model, loadedChildren: loadedChildren === undefined ? null : loadedChildren, top: top || { items: "/apps/Gallery/Widgets/actions/EnumTree", currentPath: null, parameters: {} } };
        Object.keys(options.computed).forEach((name) => Object.defineProperty(self, name, { get: () => options.computed[name].call(self) }));
        return self;
    };
    // children: true is a folder with nothing to draw yet; false is a leaf; a list came inline (open-path)
    const lazy = node({ id: "A", text: "A", li_attr: { treeNodeName: "TypeNode" }, children: true });
    assert.equal(lazy.isFolder, true);
    assert.equal(lazy.hasChildren, false);
    const leaf = node({ id: "L", text: "L", children: false });
    assert.equal(leaf.isFolder, false);
    assert.equal(leaf.hasChildren, false);
    const inline = node({ id: "I", text: "I", state: { opened: true }, children: [{ id: "I/1", text: "one", children: false }] });
    assert.equal(inline.hasChildren, true);
    assert.equal(inline.children[0].id, "I/1");
    // opening a lazy node fetches <tree url> with the node's id and name, and the list answered is what it draws
    const original = moqui.request;
    const requests = [];
    moqui.request = (url, opts) => { requests.push([url, opts.params]); return Promise.resolve({ json: [{ id: "A/1", text: "a one", children: false }] }); };
    options.watch.open.call(lazy, true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.deepEqual(requests, [["/apps/Gallery/Widgets/actions/EnumTree", { treeNodeId: "A", treeNodeName: "TypeNode", treeOpenPath: null }]]);
    assert.equal(lazy.hasChildren, true);
    assert.deepEqual(lazy.children.map((child) => child.id), ["A/1"]);
    // an inline node does not fetch, and an answer that is not a list leaves a lazy node to fetch again
    options.watch.open.call(inline, true);
    moqui.request = () => Promise.resolve({ json: { errors: "no" } });
    const other = node({ id: "B", text: "B", children: true });
    options.watch.open.call(other, true);
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(requests.length, 1);
    assert.equal(other.loadedChildren, null);
    assert.equal(other.isFolder, true);
    moqui.request = original;
});

test("a field widget puts its form's error on the Quasar field", () => {
    const mixinOwner = qvt.components["m-text-line"];
    const errorAttrs = mixinOwner.mixins.map((m) => m.computed && m.computed.errorAttrs).find((f) => f);
    assert.deepEqual(errorAttrs.call({ name: "title", mForm: { fieldErrors: { title: "Bad" } } }), { error: true, errorMessage: "Bad" });
    assert.deepEqual(errorAttrs.call({ name: "note", mForm: { fieldErrors: { title: "Bad" } } }), {});
    assert.deepEqual(errorAttrs.call({ name: "note", mForm: null }), {});
});

test("a menu image of type icon is a Quasar icon name for q-icon, or CSS classes for an i element", () => {
    ["fa fa-home", "fa-home", "glyphicon glyphicon-cog"].forEach((image) => assert.equal(moqui.isIconClass(image), true, image));
    ["sym_o_hub", "home", "o_home", "mdi-home", "", null, undefined].forEach((image) => assert.equal(moqui.isIconClass(image), false, String(image)));
    const icon = qvt.components["m-menu-icon"];
    assert.equal(icon.computed.isClass.call({ name: "sym_o_receipt_long" }), false);
    assert.equal(icon.computed.isClass.call({ name: "fa fa-cog" }), true);
    // the drawer, the breadcrumbs' subscreen menus and the screen history all draw a menu icon through it
    assert.match(qvt.components["m-menu-item-content"].template, /<m-menu-icon v-if="menuItem.imageType === 'icon'" :name="menuItem.image"/);
    const shell = qvt.components["m-qapps-root"].template;
    assert.match(shell, /<m-menu-icon v-if="subscreen.image && subscreen.imageType === 'icon'" :name="subscreen.image"/);
    assert.match(shell, /<m-menu-icon v-if="histItem.image && histItem.imageType === 'icon'" :name="histItem.image"/);
    assert.doesNotMatch(shell, /:class="(subscreen|histItem)\.image"/);
});
