/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// The split and the wizard in the page. moqui is /js/MoquiLib.js and /js/WebrootVue.qvt.js.
// The library starts the app only when #moqui-config, Vue and Quasar are all present.
"use strict";

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

test("a split side by side fills the viewport below its top, so each pane scrolls in itself; stacked the page scrolls", () => {
    assert.equal(moqui.splitHeight({ stacked: false, top: 120, viewport: 900 }), "764px");
    // at least 360 px, the height attribute as given, nothing stacked or before the viewport is known
    assert.equal(moqui.splitHeight({ stacked: false, top: 700, viewport: 800 }), "360px");
    assert.equal(moqui.splitHeight({ stacked: false, height: "70vh", top: 120, viewport: 900 }), "70vh");
    assert.equal(moqui.splitHeight({ stacked: true, top: 120, viewport: 900 }), undefined);
    assert.equal(moqui.splitHeight({ stacked: false, top: 120, viewport: 0 }), undefined);
    assert.equal(moqui.splitHeight({ stacked: false, top: 100, viewport: 900, margin: 0 }), "800px");
});

test("the wizard covers the step while its form posts and waits for the screen the answer names", () => {
    const wizard = moqui.qvt.components["m-subscreens-wizard"];
    assert.ok(wizard.template.includes('<q-inner-loading :showing="submitting"'), "the step is covered while it posts");
    assert.ok(wizard.template.includes(':disable="submitting"'), "Back waits");
    const calls = [];
    const self = Object.assign({ submitting: true, steps: [{ name: "A", pathWithParams: "/apps/x/W/A" }, { name: "B", pathWithParams: "/apps/x/W/B" }], activeIndex: 1,
        $root: { currentPathList: ["x", "W", "B"], currentParameters: {}, getLinkPath: (url) => url, setUrl: (url, body, done) => calls.push([url, done]) } }, wizard.methods);
    // the last step answered the same step with done=Y: the spinner stays until that screen is there
    wizard.methods.stepAnswered.call(self, null, { screenPathList: ["apps", "x", "W", "B"], screenUrl: "/apps/x/W/B?done=Y" });
    assert.equal(self.submitting, true);
    assert.equal(calls.length, 1);
    assert.equal(calls[0][0], "/apps/x/W/B?done=Y");
    calls[0][1]();
    assert.equal(self.submitting, false);
    // an answer with errors stays, and the spinner goes at once
    self.submitting = true;
    wizard.methods.stepAnswered.call(self, null, { errors: ["no"] });
    assert.equal(self.submitting, false);
    // a step form that posts by itself (Enter, its own button) shows the spinner too
    wizard.methods.stepSubmitting.call(self);
    assert.equal(self.submitting, true);
});
