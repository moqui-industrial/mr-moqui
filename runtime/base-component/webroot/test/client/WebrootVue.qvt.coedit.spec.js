// m-coedit's provider on the vendored Tiptap bundle, in the page. Was js/WebrootVue.qvt.coedit.test.js.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/libs/tiptap/tiptap.min.js", "/js/MoquiLib.js", "/js/WebrootVue.qvt.js"],
    test: "/files/runtime/base-component/webroot/test/client/pages/WebrootVue.qvt.coedit.page.js",
})
