/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of the pure functions of MoquiLib.js. Run: node --test runtime/base-component/webroot/screen/webroot/js/MoquiLib.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const moqui = require("./MoquiLib.js");

test("type checks", () => {
    assert.ok(moqui.isString("a") && !moqui.isString(1));
    assert.ok(moqui.isArray([]) && !moqui.isArray({}));
    assert.ok(moqui.isPlainObject({}) && !moqui.isPlainObject([]) && !moqui.isPlainObject(null) && !moqui.isPlainObject(new Date()));
    assert.ok(moqui.isNumber(1) && moqui.isBoolean(false) && moqui.isFunction(() => 1));
    assert.ok(moqui.isEmptyObject({}) && moqui.isEmptyObject(null) && !moqui.isEmptyObject({ a: 1 }));
});

test("deepCopy copies objects and lists and leaves values", () => {
    const source = { a: [1, { b: 2 }], c: "x", d: null };
    const copy = moqui.deepCopy(source);
    assert.deepEqual(copy, source);
    copy.a[1].b = 3;
    assert.equal(source.a[1].b, 2);
    assert.equal(moqui.deepCopy(5), 5);
    assert.equal(moqui.deepCopy(null), null);
});

test("lists are equal in order, or ignoring order, and empty is equal to missing", () => {
    assert.ok(moqui.arraysEqual([1, 2], [1, 2]));
    assert.ok(!moqui.arraysEqual([1, 2], [2, 1]));
    assert.ok(moqui.arraysEqual([1, 2], [2, 1], true));
    assert.ok(!moqui.arraysEqual([1, 2], [1], true));
    assert.ok(moqui.arraysEqual(null, []));
    assert.ok(moqui.arraysEqual([], undefined));
    assert.ok(!moqui.arraysEqual(["a"], null));
    assert.ok(moqui.arraysEqual("a", "a"));
});

test("a mask placeholder equals an empty value", () => {
    assert.ok(moqui.equalsOrPlaceholder("", null));
    assert.ok(moqui.equalsOrPlaceholder(undefined, ""));
    assert.ok(moqui.equalsOrPlaceholder("", "__-__"));
    assert.ok(moqui.equalsOrPlaceholder("__-__", ""));
    assert.ok(!moqui.equalsOrPlaceholder("a", "b"));
    assert.ok(moqui.equalsOrPlaceholder("a", "a"));
});

test("fieldValuesDiff finds a changed value or list", () => {
    assert.ok(!moqui.fieldValuesDiff({ a: "1", b: ["x", "y"], c: "" }, { a: "1", b: ["y", "x"], c: null }));
    assert.ok(moqui.fieldValuesDiff({ a: "1", b: ["x"] }, { a: "1", b: ["x", "y"] }));
    assert.ok(moqui.fieldValuesDiff({ a: "2" }, { a: "1" }));
    assert.ok(moqui.fieldChanged({ a: "2" }, { a: "1" }, "a"));
    assert.ok(!moqui.fieldChanged({ a: "1" }, { a: "1" }, "a"));
});

test("search text round trips with encoding and repeated names", () => {
    const text = moqui.objToSearch({ a: "x y&z", b: ["1", "2"], c: null, d: 0 });
    assert.equal(text, "a=x%20y%26z&b=1&b=2&d=0");
    assert.deepEqual(moqui.searchToObj(text), { a: "x y&z", b: ["1", "2"], d: "0" });
    assert.deepEqual(moqui.searchToObj(""), {});
    assert.deepEqual(moqui.searchToObj("a=b+c&=skip&novalue"), { a: "b c" });
    assert.equal(moqui.objToSearch(null), "");
});

test("a href splits and joins", () => {
    const info = moqui.parseHref("https://example.com:8443/qapps/tools/Foo?a=1&b=x%20y#top");
    assert.equal(info.protocol, "https");
    assert.equal(info.host, "example.com:8443");
    assert.equal(info.path, "/qapps/tools/Foo");
    assert.equal(info.name, "Foo");
    assert.equal(info.hash, "top");
    assert.deepEqual(info.query, { a: "1", b: "x y" });
    assert.equal(moqui.makeHref(info), "https://example.com:8443/qapps/tools/Foo?a=1&b=x%20y#top");
    assert.equal(moqui.makeHref({ path: "/p", query: { q: "1 2" } }), "/p?q=1%202");
    assert.equal(moqui.parseHref("/a/b").path, "/a/b");
    assert.equal(moqui.parseHref("http://host").host, "host");
    assert.equal(moqui.parseHref("/a?x=1?2").search, "x=1?2");
});

test("html encoding", () => {
    assert.equal(moqui.htmlEncode(`<a href="x">T&C's</a>`), "&lt;a href=&quot;x&quot;&gt;T&amp;C&#39;s&lt;/a&gt;");
    assert.equal(moqui.htmlEncode(null), "");
    assert.equal(moqui.htmlDecode("&lt;b&gt; &amp;amp; &quot;q&quot;"), '<b> &amp; "q"');
});

test("number text checks and parsing", () => {
    assert.ok(moqui.isStringNumber("1,234.50") && moqui.isStringNumber("-5") && !moqui.isStringNumber("1a") && !moqui.isStringNumber(5));
    assert.ok(moqui.isStringInteger("1,234") && !moqui.isStringInteger("1.5"));
    assert.equal(moqui.parseNumber("1,234.5"), 1234.5);
    assert.equal(moqui.parseNumber("1,234"), 1234);
    moqui.setNumberSeparators("de-DE");
    assert.equal(moqui.parseNumber("1.234,5"), 1234.5);
    moqui.setNumberSeparators("en-US");
    assert.equal(moqui.parseNumber("1,234.5"), 1234.5);
});

test("LruMap keeps the most recently used entries", () => {
    const map = new moqui.LruMap(2);
    map.put("a", 1);
    map.put("b", 2);
    assert.equal(map.get("a"), 1);
    map.put("c", 3);
    assert.ok(map.containsKey("a") && map.containsKey("c") && !map.containsKey("b"));
    map.remove("a");
    assert.ok(!map.containsKey("a"));
    assert.deepEqual(map.lruList, ["c"]);
});

test("debounce runs once after the wait", async () => {
    let calls = 0;
    const fn = moqui.debounce((value) => { calls += value; }, 20);
    fn(1); fn(1); fn(1);
    assert.equal(calls, 0);
    await new Promise((resolve) => setTimeout(resolve, 60));
    assert.equal(calls, 1);
});

test("a component file splits into template, script and style", () => {
    const styles = [];
    const comp = moqui.parseVue(`<template><div class="a">{{x}}</div></template>
        <script>
        module.exports = { data: function () { return { x: 3 }; }, name: "Demo" };
        </script>
        <style scoped>.a { color: red }</style>`, (text, scoped) => styles.push([text, scoped]));
    assert.equal(comp.template, '<div class="a">{{x}}</div>');
    assert.equal(comp.name, "Demo");
    assert.equal(comp.data().x, 3);
    assert.deepEqual(styles, [[".a { color: red }", true]]);

    const exported = moqui.parseVue("<template><p/></template><script>export default { props: ['id'] }</script>");
    assert.deepEqual(exported.props, ["id"]);
    const defined = moqui.parseVue("<script>define([], function () { return { methods: { go() {} } }; });</script>");
    assert.equal(typeof defined.methods.go, "function");
    assert.deepEqual(moqui.parseVue("no parts here"), {});
});

test("request sends the token on writes, parses json, and rejects with the response on an error", async () => {
    const calls = [];
    globalThis.fetch = (url, init) => {
        calls.push([url, init]);
        const ok = !String(url).includes("fail");
        return Promise.resolve({
            ok, status: ok ? 200 : 500, statusText: "",
            headers: new Map([["X-CSRF-Token", "t2"], ["X-Moqui-Preferences", encodeURIComponent(JSON.stringify({ SHELL_DENSITY: "compact", KEYMAP: "É" }))]]),
            text: () => Promise.resolve(ok ? '{"a":1}' : '{"errors":"bad"}')
        });
    };
    moqui.sessionToken = "t1";
    let seen = "";
    moqui.onSessionToken = (token) => { seen = token; };
    // the preferences the request set, decoded for the page to apply
    const preferences = [];
    moqui.onPreferences = (map) => preferences.push(map);
    // a Map stands in for Headers: get() works on both
    const good = await moqui.request("/x", { method: "post", params: { a: "b c" } });
    assert.deepEqual(good.json, { a: 1 });
    assert.equal(calls[0][1].headers["X-CSRF-Token"], "t1");
    assert.equal(calls[0][1].body, "a=b%20c");
    assert.equal(seen, "t2");
    assert.deepEqual(preferences[0], { SHELL_DENSITY: "compact", KEYMAP: "É" });
    delete moqui.onPreferences;
    await moqui.request("/x?y=1", { params: { a: 1 } });
    assert.equal(calls[1][0], "/x?y=1&a=1");
    assert.equal(calls[1][1].headers["X-CSRF-Token"], undefined);
    await assert.rejects(moqui.request("/fail"), (err) => err.status === 500 && err.json.errors === "bad");
    delete globalThis.fetch;
});

test("a qjs response is an object expression, define, or statements", () => {
    assert.equal(moqui.evalComponentJs("{ data: function () { return { a: 1 }; } }").data().a, 1);
    assert.equal(moqui.evalComponentJs("define([], function () { return { name: 'D' }; });").name, "D");
    assert.equal(moqui.evalComponentJs("var x = { name: 'S' }; return x;").name, "S");
    assert.throws(() => moqui.evalComponentJs("{ nope"));
});

// A document with a head whose scripts load or fail when the test says (fail: the srcs that fail on their next load).
const fakeDocument = (fail) => {
    const head = { children: [], appendChild(node) { node.parentNode = head; head.children.push(node);
        setTimeout(() => { if (fail.has(node.src)) { fail.delete(node.src); node.onerror && node.onerror(); } else if (node.onload) node.onload(); }, 5); },
        removeChild(node) { head.children = head.children.filter((item) => item !== node); node.parentNode = null; } };
    return { head, created: 0,
        createElement() { this.created += 1; return { dataset: {}, getAttribute(name) { return name === "src" ? this.src : null; } }; },
        querySelectorAll() { return head.children.slice(); } };
};

test("loadScript: a failed load rejects every waiter and a later call tries again; a load is shared", async () => {
    const savedDocument = globalThis.document;
    const savedError = console.error;
    console.error = () => {};
    try {
        const fail = new Set(["/libs/x.js"]);
        globalThis.document = fakeDocument(fail);
        const calls = [];
        const first = moqui.loadScript("/libs/x.js", (error) => calls.push(error ? "error" : "ok"));
        const second = moqui.loadScript("/libs/x.js");
        await assert.rejects(first, /Error loading script \/libs\/x.js/);
        await assert.rejects(second, /Error loading script/);
        assert.deepEqual(calls, ["error"]);
        assert.equal(document.created, 1, "the waiters share one load");
        assert.equal(document.head.children.length, 0, "the failed element is removed");
        // a retry loads it, and a call after that is answered by the same load
        const script = await moqui.loadScript("/libs/x.js");
        assert.equal(script.dataset.mqState, "loaded");
        assert.equal(await moqui.loadScript("/libs/x.js"), script);
        assert.equal(document.created, 2);
        // a library that never sets its global rejects after the validate tries (shortened here)
        const savedRetry = moqui.retryValidateCallback;
        moqui.retryValidateCallback = (callback, validate, count, onFail) => (validate() ? callback() : onFail());
        try {
            await assert.rejects(moqui.loadScript("/libs/x.js", null, () => false), /library is not there/);
            assert.equal(await moqui.loadScript("/libs/x.js", null, () => true), script);
        } finally {
            moqui.retryValidateCallback = savedRetry;
        }
    } finally {
        moqui.scriptLoads.clear();
        globalThis.document = savedDocument;
        console.error = savedError;
    }
});
