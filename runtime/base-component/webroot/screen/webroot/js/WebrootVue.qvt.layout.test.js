/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of the layout components of WebrootVue.qvt.js that need no browser: the layout m-container-split takes from the
// viewport, its width and its attributes, and where m-subscreens-wizard goes after its step form's answer. The splitter
// itself, the stacked panes at phone width and a wizard stepping with Next and Back are the Chromium smoke of Gate 3.
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.layout.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");

globalThis.moqui = require("./MoquiLib.js");
require("./WebrootVue.qvt.js");

test("a split stacks below 1024 px and when side and main do not fit, and keeps the side width in its limits", () => {
    assert.deepEqual(moqui.splitLayout({ wide: true, width: 1200, sideWidth: 320, minMain: 480 }), { stacked: false, side: 320, min: 200, max: 720 });
    assert.equal(moqui.splitLayout({ wide: false, width: 1200, sideWidth: 320, minMain: 480 }).stacked, true);
    // a container narrower than 200 + 480, as beside an open Assist drawer
    assert.equal(moqui.splitLayout({ wide: true, width: 640, sideWidth: 320, minMain: 480 }).stacked, true);
    // a saved width, and one that would squeeze the main pane
    assert.equal(moqui.splitLayout({ wide: true, width: 1200, sideWidth: 320, minMain: 480, saved: 410 }).side, 410);
    assert.equal(moqui.splitLayout({ wide: true, width: 1200, sideWidth: 320, minMain: 480, saved: 900 }).side, 720);
    assert.equal(moqui.splitLayout({ wide: true, width: 1200, sideWidth: 320, minMain: 480, saved: 50 }).side, 200);
    // not measured yet: the attribute's width, the defaults for text and nonsense
    assert.equal(moqui.splitLayout({ wide: true, width: 0, sideWidth: "280", minMain: "x" }).side, 280);
    assert.equal(moqui.splitLayout({ wide: true, width: 0, sideWidth: 280 }).stacked, false);
    assert.deepEqual(moqui.splitLayout({ wide: true, width: 1000 }), { stacked: false, side: 320, min: 200, max: 520 });
});

test("a step's URL keeps the current parameters under its own and the answer's, without the session token", () => {
    assert.equal(moqui.wizardStepUrl("/apps/gallery/Wizard/Options?step=2", { importId: "I1", moquiSessionToken: "t", empty: "" }, { importId: "I2", rows: "5" }),
        "/apps/gallery/Wizard/Options?importId=I2&step=2&rows=5");
    assert.equal(moqui.wizardStepUrl("/apps/x/Wizard/Source", {}, null), "/apps/x/Wizard/Source");
    assert.equal(moqui.wizardStepUrl("/apps/x/Wizard/Source", { q: "a b" }, null), "/apps/x/Wizard/Source?q=a%20b");
});

test("after an answer the wizard stays on errors, goes where another screen is named, and else to the next step", () => {
    const path = ["gallery", "Wizard", "Customer"];
    assert.deepEqual(moqui.wizardAfterAnswer({ errors: ["Customer name is required"] }, path, 3, 0), { kind: "stay" });
    assert.deepEqual(moqui.wizardAfterAnswer({ validationErrors: [{ field: "x", message: "bad" }] }, path, 3, 0), { kind: "stay" });
    // the transition answered url=".": the next step, with the answer's parameters
    assert.deepEqual(moqui.wizardAfterAnswer({ screenPathList: ["apps", "gallery", "Wizard", "Customer"], screenUrl: "/apps/gallery/Wizard/Customer?draftId=D1",
        screenParameters: { draftId: "D1" } }, path, 3, 0), { kind: "step", index: 1, parameters: { draftId: "D1" } });
    // no screen named (a JSON answer of a service): the next step
    assert.deepEqual(moqui.wizardAfterAnswer({ messageInfos: [] }, path, 3, 1), { kind: "step", index: 2, parameters: {} });
    // the transition chose another screen: there
    assert.deepEqual(moqui.wizardAfterAnswer({ screenPathList: ["apps", "gallery", "List"], screenUrl: "/apps/gallery/List" }, path, 3, 0),
        { kind: "url", url: "/apps/gallery/List" });
    assert.deepEqual(moqui.wizardAfterAnswer({ redirectUrl: "https://example.com/" }, path, 3, 0), { kind: "redirect", url: "https://example.com/" });
    // the last step follows the answer, or stays
    assert.deepEqual(moqui.wizardAfterAnswer({ screenPathList: ["apps", "gallery", "Wizard", "Review"], screenUrl: "/apps/gallery/Wizard/Review?done=Y" },
        ["gallery", "Wizard", "Review"], 3, 2), { kind: "url", url: "/apps/gallery/Wizard/Review?done=Y" });
    assert.deepEqual(moqui.wizardAfterAnswer({}, ["gallery", "Wizard", "Review"], 3, 2), { kind: "stay" });
});
