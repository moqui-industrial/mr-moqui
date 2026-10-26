// The dialog of moqui-html.js: focus and which clicks close it. Was js/dialog-browser.test.js.
spec({ server: false })

feature("a dialog focuses its first form field, not a Tom Select search, and closes on a backdrop click only", {
    expect() {
        const page = browser.newPage({ viewport: { width: 1440, height: 1000 } })
        // The body is taller than the dialog, so the dialog scrolls and a click on its scroll bar has the dialog as target.
        page.setContent(`<!doctype html><html data-theme="light">
            <link rel="stylesheet" href="/css/moqui-html.css">
            <button id="open" data-dialog="customer">New Customer</button>
            <dialog id="customer"><header><h4>New Customer</h4><button class="close" data-close>×</button></header>
                <div class="body"><div class="ts-wrapper"><div class="ts-control"><input name="generatedSearch"></div></div><input name="firstName"><div style="height:2000px"></div></div></dialog>
            <pre id="result"></pre>
            <script src="/js/moqui-html.js"></script>
            <script>
            document.addEventListener("DOMContentLoaded", () => {
                document.getElementById("open").click();
                const dialog = document.getElementById("customer");
                const rect = dialog.getBoundingClientRect();
                const focused = document.activeElement.name;
                dialog.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: rect.right - 2, clientY: rect.top + rect.height / 2 }));
                const staysOpenInside = dialog.open;
                dialog.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: 1, clientY: 1 }));
                document.getElementById("result").textContent = JSON.stringify({ focused, staysOpenInside, closesOutside: !dialog.open });
            });
            </script>`)
        const text = page.waitFor(() => {
            const node = document.getElementById("result")
            return node && node.textContent ? node.textContent : null
        })
        assert.deepEqual(JSON.parse(text), { focused: "firstName", staysOpenInside: true, closesOutside: true })
    },
})
