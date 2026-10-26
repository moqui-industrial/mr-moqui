// m-code on the vendored highlight.js, in the page. Was js/WebrootVue.qvt.code.test.js.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/js/MoquiLib.js", "/js/WebrootVue.qvt.js", "/libs/highlightjs/highlight.min.js"],
    test: "/files/runtime/base-component/webroot/test/client/pages/WebrootVue.qvt.code.page.js",
})
