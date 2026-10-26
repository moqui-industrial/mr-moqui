// Menus and drop-downs in a sideways-scrolling table. Was js/table-popup-browser.test.js.
spec({ server: false })

feature("a menu and a drop-down in a scrolling table open outside it, and the menu closes when the table scrolls", {
    expect() {
        const options = Array.from({ length: 12 }, (_, i) => `<option value="${i}">Option ${i}</option>`).join("")
        const page = browser.newPage({ viewport: { width: 1000, height: 800 } })
        page.setContent(`<!doctype html><html data-theme="light">
            <link rel="stylesheet" href="/css/moqui-html.css">
            <link rel="stylesheet" href="/libs/tom-select/tom-select.css">
            <div style="width:500px"><div class="table-wrap" id="wrap"><table class="form-list" style="width:900px"><tbody><tr>
                <td><details class="menu" id="menu"><summary data-btn="primary">Actions</summary><ul><li><a href="#one">One</a></li><li><a href="#two">Two</a></li><li><a href="#three" id="last">Three</a></li></ul></details></td>
                <td><select id="pick" data-ts="select">${options}</select></td><td style="width:600px">Wide</td></tr></tbody></table></div></div>
            <pre id="result"></pre>
            <script src="/libs/tom-select/tom-select.complete.min.js"></script>
            <script src="/js/moqui-html.js"></script>
            <script>
            setTimeout(() => {
                const menu = document.getElementById("menu"); menu.open = true;
                setTimeout(() => {
                    const rect = document.getElementById("last").getBoundingClientRect();
                    const menuVisible = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2) === document.getElementById("last");
                    const pick = document.getElementById("pick").tomselect; pick.open();
                    const dropdownOnBody = pick.dropdown.parentElement === document.body;
                    const wrap = document.getElementById("wrap"); wrap.scrollLeft = 50; wrap.dispatchEvent(new Event("scroll"));
                    setTimeout(() => { document.getElementById("result").textContent = JSON.stringify({ menuVisible, dropdownOnBody, menuClosed: !menu.open, scrolls: wrap.scrollWidth > wrap.clientWidth }); }, 50);
                }, 20);
            }, 50);
            </script>`)
        const text = page.waitFor(() => {
            const node = document.getElementById("result")
            return node && node.textContent ? node.textContent : null
        })
        assert.deepEqual(JSON.parse(text), { menuVisible: true, dropdownOnBody: true, menuClosed: true, scrolls: true })
    },
})
