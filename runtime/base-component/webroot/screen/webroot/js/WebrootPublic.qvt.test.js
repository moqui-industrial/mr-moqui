/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of the public shell of the token routes (WebrootPublic.qvt.js) that need no browser: the root loads the route's
// screen at its base path in the render mode the screen has, keeps a link under the route on the page, and lets any other
// leave. The page itself is in crates/moqui-host/tests/gate3.rs (the token route).
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootPublic.qvt.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");

globalThis.moqui = require("./MoquiLib.js");
require("./WebrootVue.qvt.js");
require("./WebrootPublic.qvt.js");

const config = { route: { name: "s", basePath: "/s/TOKEN1234", renderModes: ["qvue"] }, context: { name: "Plans" },
    sessionToken: "st", labels: { notFound: "gone" } };

const instance = () => {
    const options = moqui.publicRootOptions(config);
    const vm = Object.assign({}, options.data(), options.methods);
    Object.defineProperty(vm, "currentLinkPath", { get: () => options.computed.currentLinkPath.call(vm) });
    return { vm, options };
};

test("the root provides the route's screen as mScreen", () => {
    const options = moqui.publicRootOptions(config);
    const vm = { basePath: "/s/TOKEN1234", currentParameters: { folder: "a b" } };
    const screen = options.provide.call(vm).mScreen;
    assert.deepEqual([screen.url, screen.path, screen.parameters], ["/s/TOKEN1234?folder=a%20b", "/s/TOKEN1234", { folder: "a b" }]);
});

test("the root loads the route's screen at its base path with its render modes", () => {
    const { vm } = instance();
    const loaded = [];
    const original = moqui.loadComponent;
    moqui.loadComponent = (urlInfo, callback) => { loaded.push(urlInfo); callback({ template: "<div/>" }); return { abort() {} }; };
    try {
        vm.currentParameters = { folder: "a b" };
        vm.load();
    } finally {
        moqui.loadComponent = original;
    }
    assert.equal(loaded.length, 1);
    assert.equal(loaded[0].path, "/s/TOKEN1234");
    assert.deepEqual(loaded[0].renderModes, ["qvue"]);
    assert.equal(loaded[0].search, "folder=a%20b");
    assert.deepEqual(vm.activeComponent, { template: "<div/>" });
    assert.equal(vm.failed, false);
    assert.equal(vm.basePath, "/s/TOKEN1234");
    assert.equal(vm.currentLinkPath, "/s/TOKEN1234");
    assert.deepEqual(vm.context, { name: "Plans" });
});

test("a screen that is not found shows the not-found text", () => {
    const { vm } = instance();
    const original = moqui.loadComponent;
    moqui.loadComponent = (urlInfo, callback) => { callback(moqui.NotFound); return { abort() {} }; };
    try { vm.load(); } finally { moqui.loadComponent = original; }
    assert.equal(vm.failed, true);
    assert.equal(vm.activeComponent, null);
});

test("a link under the route reloads the screen with its parameters; another URL leaves the page", () => {
    const { vm } = instance();
    let loads = 0;
    vm.load = () => { loads++; };
    const assigned = [];
    const location = globalThis.location;
    globalThis.location = { assign: (href) => assigned.push(href), search: "" };
    try {
        vm.setUrl("/s/TOKEN1234?path=docs");
        assert.equal(loads, 1);
        assert.deepEqual(vm.currentParameters, { path: "docs" });
        vm.setUrl("/qapps/Hub");
        assert.deepEqual(assigned, ["/qapps/Hub"]);
        assert.equal(loads, 1);
    } finally {
        globalThis.location = location;
    }
    // the parts of the /qapps root the components call are there and do nothing
    for (const name of ["addContainer", "removeContainer", "setScreenContext", "removeScreenContext", "setPreference"]) vm[name]();
    vm.addNotify("Saved", "positive");
    assert.equal(vm.notifyHistoryList[0].message, "Saved");
});
