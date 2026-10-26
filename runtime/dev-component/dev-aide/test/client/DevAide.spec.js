// The Workspace client's pure parts, in the page. Was screen/devstatic/DevAide.test.js.
spec({ server: false, timeout: 60 })

browser.runInPage({
    scripts: ["/files/runtime/dev-component/dev-aide/screen/devstatic/DevAide.js"],
    test: "/files/runtime/dev-component/dev-aide/test/client/pages/DevAide.page.js",
})
