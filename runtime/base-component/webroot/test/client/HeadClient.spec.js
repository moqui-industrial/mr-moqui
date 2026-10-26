// Header.html.fancy loads moqui-html.js in the head without defer. Was js/head-client-browser.test.js.
spec({ server: false })

feature("the html client in the head has moqui.request while the body parses and binds the page after", {
    expect() {
        const page = browser.newPage()
        page.setContent(`<!doctype html><html><head><meta name="moquiSessionToken" content="initial"><script>
            window.calls = [];
            window.fetch = (url, options) => { calls.push({ url, credentials: options.credentials }); return Promise.resolve(new Response("[]", { headers: { "Content-Type": "application/json" } })); };
            </script><script src="/js/moqui-html.js"></script></head><body>
            <button data-dialog="detail">Details</button><dialog id="detail">Loaded</dialog><pre id="result"></pre><script>
            const before = { request: typeof moqui.request, ready: document.readyState, bound: document.documentElement.classList.contains("js") };
            const answer = moqui.request("/calendar/events");
            document.addEventListener("DOMContentLoaded", () => { setTimeout(async () => { await answer; document.querySelector("button").click();
                document.getElementById("result").textContent = JSON.stringify({ before, calls, bound: document.documentElement.classList.contains("js"), opened: document.querySelector("dialog").open }); }, 0); }, { once: true });
            </script></body></html>`)
        const text = page.waitFor(() => {
            const node = document.getElementById("result")
            return node && node.textContent ? node.textContent : null
        })
        const result = JSON.parse(text)
        assert.deepEqual(result.before, { request: "function", ready: "loading", bound: false })
        assert.deepEqual(result.calls, [{ url: "/calendar/events", credentials: "same-origin" }])
        assert.equal(result.bound, true)
        assert.equal(result.opened, true)
    },
})
