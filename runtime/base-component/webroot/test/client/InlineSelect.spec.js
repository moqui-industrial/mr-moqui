// An inline remote select sizes to its options. Was js/inline-select-browser.test.js.
spec({ server: false })

feature("inline remote options set the natural width and resize after a dependency changes", {
    expect() {
        const page = browser.newPage({ viewport: { width: 1440, height: 1000 } })
        page.setContent(`<!doctype html><html data-theme="light">
            <link rel="stylesheet" href="/css/moqui-html.css">
            <link rel="stylesheet" href="/libs/tom-select/tom-select.css">
            <form><select id="country" name="country"><option value="USA">USA</option><option value="CAN">Canada</option></select>
            <div style="width:840px;padding:0 15px"><div class="field big-row"><label>State</label><div class="widget">
                <div class="big-item"><div class="field"><div class="widget"><select id="state" name="state" data-ts="remote" data-url="/states" data-depends='[["country","country"]]'><option selected value="USA_CO">CO - Colorado</option></select></div></div></div>
                <div class="big-item"><div class="field"><div class="widget"><select id="search" data-ts="remote" data-search><option>A</option></select></div></div></div>
            </div></div><div class="field"><label>Owner</label><div class="widget"><select id="owner" data-ts="remote"><option>A</option></select></div></div></div></form>
            <pre id="result"></pre>
            <script>
            const responses = { USA: [{ value: "USA_FM", label: "FM - Federated States of Micronesia" }, { value: "USA_CO", label: "CO - Colorado" }], CAN: [{ value: "CAN_BC", label: "BC - British Columbia" }] };
            let calls = 0;
            window.fetch = (url, options) => { calls++; const country = new URLSearchParams(options.body).get("country"); return Promise.resolve({ ok: true, headers: new Headers(), text: () => Promise.resolve(JSON.stringify(responses[country])) }); };
            </script>
            <script src="/libs/tom-select/tom-select.complete.min.js"></script>
            <script src="/js/moqui-html.js"></script>
            <script>
            const metric = () => { const select = document.getElementById("state"); return { width: select.tomselect.wrapper.getBoundingClientRect().width, value: select.value, count: Object.keys(select.tomselect.options).filter(Boolean).length, submitted: new FormData(document.querySelector("form")).get("state") }; };
            const natural = () => { const s = document.createElement("select"); s.style.cssText = "position:absolute;width:auto;min-width:0;max-width:none;padding:0;border:1px solid;font:" + getComputedStyle(document.getElementById("state")).font;
                for (const o of document.getElementById("state").options) s.appendChild(new Option(o.text, o.value)); document.body.appendChild(s); const w = s.getBoundingClientRect().width; s.remove(); return w; };
            setTimeout(() => {
                const initial = metric(), expected = natural();
                const owner = document.getElementById("owner").tomselect.wrapper.getBoundingClientRect().width;
                const ownerWidget = document.getElementById("owner").closest(".widget").clientWidth;
                const search = document.getElementById("search").tomselect.wrapper.style.width;
                document.getElementById("country").value = "CAN"; document.getElementById("country").dispatchEvent(new Event("change"));
                setTimeout(() => { document.getElementById("result").textContent = JSON.stringify({ initial, expected, owner, ownerWidget, search, updated: metric(), calls, temporarySelects: document.querySelectorAll("select[aria-hidden=true]").length }); }, 50);
            }, 50);
            </script>`)
        const text = page.waitFor(() => {
            const node = document.getElementById("result")
            return node && node.textContent ? node.textContent : null
        })
        const result = JSON.parse(text)
        assert.ok(Math.abs(result.initial.width - (result.expected + 24)) < 0.5, JSON.stringify(result))
        assert.ok(result.initial.width > 200)
        assert.equal(result.initial.value, "USA_CO")
        assert.equal(result.initial.submitted, "USA_CO")
        assert.equal(result.initial.count, 2)
        assert.ok(result.updated.width < result.initial.width)
        assert.ok(result.updated.width >= 60)
        assert.equal(result.updated.value, "CAN_BC")
        assert.equal(result.updated.submitted, "CAN_BC")
        assert.equal(result.updated.count, 1)
        assert.ok(result.owner > 600, JSON.stringify(result))
        assert.equal(result.search, "")
        assert.equal(result.calls, 2)
        assert.equal(result.temporarySelects, 0)
    },
})
