// The code editor's helpers and the vendored CodeMirror bundle, in the page. Was js/WebrootCode.test.js.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/js/MoquiLib.js", "/libs/codemirror/codemirror.min.js", "/js/WebrootCode.js"],
    test: "/files/runtime/base-component/webroot/test/client/pages/WebrootCode.page.js",
})
