// The pure functions of moqui-html.js, in the page. Was js/moqui-html.test.js.
// The module script is what the file assigns api.pure to. The client still starts on this document.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: [
        "/files/runtime/base-component/webroot/test/client/pages/moqui-html-module.js",
        "/js/moqui-html.js",
    ],
    test: "/files/runtime/base-component/webroot/test/client/pages/moqui-html.page.js",
})
