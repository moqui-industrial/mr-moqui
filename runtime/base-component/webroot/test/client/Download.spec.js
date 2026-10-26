// moqui.downloadData writes a blob link and revokes it. Was js/download-browser.test.js.
spec({ server: false })

feature("inline chart CSV export preserves payload, filename and MIME type and cleans its object URL", {
    expect() {
        const page = browser.newPage()
        page.setContent(`<!doctype html><html><button id="csv" onclick="moqui.downloadData('Name,Value\\nCafé,2\\n','report.csv','text/csv')">Get CSV</button><pre id="result"></pre>
            <script>
            const state = {}; let blob;
            URL.createObjectURL = (value) => { blob = value; return "blob:fixture"; };
            URL.revokeObjectURL = (value) => { state.revoked = value; };
            HTMLAnchorElement.prototype.click = function () { state.filename = this.download; state.url = this.href; state.attached = this.isConnected; };
            window.addEventListener("error", (event) => { state.error = event.message; });
            </script>
            <script src="/js/moqui-html.js"></script>
            <script>
            document.getElementById("csv").click();
            setTimeout(async () => {
                if (blob) { state.type = blob.type; state.text = await blob.text(); }
                state.links = document.querySelectorAll("a[download]").length;
                document.getElementById("result").textContent = JSON.stringify(state);
            }, 100);
            </script>`)
        const text = page.waitFor(() => {
            const node = document.getElementById("result")
            return node && node.textContent ? node.textContent : null
        })
        assert.deepEqual(JSON.parse(text), {
            filename: "report.csv", url: "blob:fixture", attached: true, revoked: "blob:fixture",
            type: "text/csv", text: "Name,Value\nCafé,2\n", links: 0,
        })
    },
})
