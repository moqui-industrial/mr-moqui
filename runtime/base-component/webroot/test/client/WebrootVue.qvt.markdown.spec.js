// m-markdown on the vendored Marked, in the page. Was js/WebrootVue.qvt.markdown.test.js.
// The marked UMD installs globalThis.marked. WebrootVue.qvt.js does not start the app on this page.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/js/MoquiLib.js", "/js/WebrootVue.qvt.js", "/libs/marked/marked.umd.min.js"],
    test: "/files/runtime/base-component/webroot/test/client/pages/WebrootVue.qvt.markdown.page.js",
})
