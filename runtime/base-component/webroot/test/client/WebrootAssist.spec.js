// Assist, in the page. Was js/WebrootAssist.test.js.
// Assist loads before WebrootVue.qvt.js so qvt.registerLate still sees components registered later.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/js/MoquiLib.js", "/js/MoquiL10n.js", "/js/WebrootChat.js", "/js/WebrootAssist.js", "/libs/vue/compiler-dom.cjs.js", "/js/WebrootVue.qvt.js", "/libs/marked/marked.umd.min.js"],
    test: "/files/runtime/base-component/webroot/test/client/pages/WebrootAssist.page.js",
})
