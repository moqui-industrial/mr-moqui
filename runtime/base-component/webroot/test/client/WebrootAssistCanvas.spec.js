// The Assist canvas, in the page. Was js/WebrootAssistCanvas.test.js.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/js/MoquiLib.js", "/js/WebrootAssist.js", "/libs/openui/lang-core.min.js",
        "/files/runtime/base-component/webroot/test/client/pages/WebrootAssistCanvas-module.js", "/js/WebrootAssistCanvas.js"],
    test: "/files/runtime/base-component/webroot/test/client/pages/WebrootAssistCanvas.page.js",
})
