/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! Gate 3 browser smoke: headless Chromium over the DevTools Protocol (`common/cdp.rs`) against the seeded server of `gate3.rs`.
//!
//! Each kept mode is loaded in the browser: `html` (with `moqui-html.js`), `html-print`, `text`, `csv` and `xml` (fetched by the page),
//! server-merged `qvt` through the `/qapps` shell, and server-static `qvt` with its `/actions` package. Each page must render its fixture
//! content with a clean console: no uncaught exception, no `console.error` or `console.warn`, and no error or warning log entry (a failed
//! request is one). The client features the macros bind are driven as a user would: dialogs, click to edit, a background submit, the
//! column picker, a form post with the session token, Quasar menus and popups, the lazy Chart.js and Mermaid, and the pages and sort of a
//! form-list. The server-static compatibility report, and the fixture's clean verdict in it, is `it::qvt_report`.
//!
//! The browser is `pdf-chromium-path`, else `chromium`, `chromium-browser` or `google-chrome` on the `PATH`. A missing binary fails.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

mod common;

// the example component loads with the repository runtime; link its tool factory as the moqui binary does
use example as _;

use std::time::Duration;

use common::cdp::{Browser, Problem};
use common::{start, start_with, start_with_conf, start_with_data, FIXTURE};

const WAIT: Duration = Duration::from_secs(30);

/// Records every `fetch` of the page with its answer, as `window.__gate.fetches`.
const FETCH_SPY: &str = r#"(() => {
    if (window.__gate) return true;
    window.__gate = { fetches: [] };
    const original = window.fetch.bind(window);
    window.fetch = (input, init) => original(input, init).then((response) => {
        const url = String((input && input.url) || input);
        const method = (init && init.method) || (input && input.method) || "GET";
        // the fields a form posted, for a check of what a request carried
        const sent = init && init.body && typeof init.body.entries === "function" ? Array.from(init.body.entries()).map(([key, value]) => key + "=" + value) : [];
        response.clone().text().then((body) => window.__gate.fetches.push({ url, method, status: response.status, body, sent }));
        return response;
    });
    return true;
})()"#;

/// The first visible element among `selector` whose trimmed text, or its last line (after the ligature of a Material icon), is `text`,
/// ignoring case (`innerText` follows `text-transform`, and Quasar buttons are upper case).
fn by_text(selector: &str, text: &str) -> String {
    let text = text.to_lowercase();
    format!(
        "Array.from(document.querySelectorAll({selector:?})).find((el) => {{ if (el.offsetParent === null || !el.innerText) return false; \
         const shown = el.innerText.trim().toLowerCase(); return shown === {text:?} || shown.split('\\n').pop().trim() === {text:?}; }})"
    )
}

fn assert_clean(browser: &mut Browser, page: &str) {
    let problems: Vec<Problem> = browser.take_problems();
    assert!(problems.is_empty(), "{page}: the console is not clean: {problems:#?}");
}

/// The answer of the last recorded fetch whose URL contains `part`, once there is one.
fn fetched(browser: &mut Browser, what: &str, part: &str) -> serde_json::Value {
    browser.wait_for(
        what,
        &format!("window.__gate.fetches.filter((f) => f.url.indexOf({part:?}) >= 0).slice(-1)[0] || null"),
        WAIT,
    )
}

fn sign_in(browser: &mut Browser, base: &str) {
    browser.navigate(&format!("{base}/Login"));
    browser.eval(
        "(() => { const form = document.getElementById('login_form'); form.username.value = 'john.doe'; \
         form.password.value = 'moqui'; form.requestSubmit(); return true; })()",
    );
    browser.wait_event("Page.loadEventFired", WAIT);
    browser.wait_for(
        "signed in",
        "location.pathname !== '/Login' && document.readyState === 'complete'",
        WAIT,
    );
    assert_clean(browser, "/Login");
}

#[test]
fn the_server_merged_modes_render_in_chromium_with_a_clean_console() {
    let gate = start("browser");
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    sign_in(&mut browser, &base);
    html_fixture(&mut browser, &base);
    html_widgets(&mut browser, &base);
    print_and_text_modes(&mut browser, &base);
    qapps_fixture(
        &mut browser,
        &base,
        &gate,
        "Posted from Chromium",
        ("AUTHZA_CREATE", "Create"),
    );
    qapps_widgets(&mut browser, &base);
}

#[test]
fn server_static_qvt_renders_in_chromium_with_its_actions_data() {
    let gate = start_with("browser-static", true);
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    sign_in(&mut browser, &base);
    static_fixture(&mut browser, &base);
    qapps_fixture(
        &mut browser,
        &base,
        &gate,
        "System App (via root screen)",
        ("AUTHZA_VIEW", "View"),
    );
    static_list(&mut browser, &base);
}

/// With `webapp_qvt_static_only=true` the page loads Vue without its compiler, and a policy of `script-src 'self'` (no `unsafe-eval`, no inline
/// script) holds: the shell and the components run their precompiled render functions and the static screen its server-compiled one.
#[test]
fn server_static_qvt_runs_on_the_runtime_build_under_a_policy_without_eval() {
    let gate = start_with_conf(
        "browser-runtime",
        r#"<webapp-list><webapp name="webroot">
            <response-header type="screen-render" name="Content-Security-Policy" value="frame-ancestors 'none'; form-action 'self'; script-src 'self'"/>
        </webapp></webapp-list>"#,
        &[("webapp_qvt_static_only", "true")],
    );
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    // the screen first, so the login goes back to it: every screen this page shows must be server-static, and the app list is not
    let page = format!("{base}/qapps/GateStaticList");
    browser.navigate(&page);
    browser.wait_for(
        "the login page",
        "location.pathname === '/Login' && document.readyState === 'complete'",
        WAIT,
    );
    browser.eval(
        "(() => { const form = document.getElementById('login_form'); form.username.value = 'john.doe'; \
         form.password.value = 'moqui'; form.requestSubmit(); return true; })()",
    );
    browser.wait_event("Page.loadEventFired", WAIT);
    browser.wait_for("the list", "/1-20 \\/ \\d+/.test(document.body.innerText)", WAIT);
    assert_eq!(
        browser.eval("!!document.querySelector('script[src*=\"vue.runtime.global.prod.js\"]') && !document.querySelector('script[src*=\"vue.global.prod.js\"]')"),
        true
    );
    // the shell drew its menu and toolbar, and the list its rows
    let text = browser.eval("document.body.innerText");
    let text = text.as_str().unwrap();
    assert!(
        text.contains("Enumerations of john.doe") && text.contains("Static List"),
        "{text}"
    );
    assert_clean(&mut browser, &page);
}

#[test]
fn a_notification_sent_on_the_server_shows_in_the_html_and_qapps_pages() {
    let gate = start("browser-notify");
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    // not signed in: the login page names no endpoint and opens no socket
    browser.navigate(&format!("{base}/Login"));
    browser.wait_for("moqui-html.js bound the page", "!!window.moqui", WAIT);
    assert_eq!(
        browser.eval("!document.querySelector('meta[name=moqui-notify]') && !window.moqui.notificationSocket"),
        true
    );
    assert_clean(&mut browser, "/Login before sign in");
    sign_in(&mut browser, &base);
    let user_id = user_id_of(&gate, "john.doe");

    // html: a toast from moqui-html.js
    browser.navigate(&format!("{base}/apps{FIXTURE}.html?artifactGroupId=SYSTEM_APP"));
    browser.wait_for(
        "the notification socket is open",
        "!!(window.moqui && moqui.notificationSocket && moqui.notificationSocket.readyState === 1)",
        WAIT,
    );
    let shown = notify_until_shown(
        &mut browser,
        &gate,
        &user_id,
        "Html",
        "Array.from(document.querySelectorAll('#toasts .alert')).map((el) => el.textContent).find((text) => text.indexOf('Gate note Html') >= 0) || null",
    );
    assert!(shown.contains("Gate note Html"), "{shown}");
    assert_eq!(
        browser.eval("!!document.querySelector('#toasts .alert[data-type=success] a[href=\"/qapps/System\"]')"),
        true
    );
    assert_clean(&mut browser, "html notification");

    // qvt: a Quasar notify from the WebrootVue client, and the notification history of the shell
    browser.navigate(&format!("{base}/qapps{FIXTURE}?artifactGroupId=SYSTEM_APP"));
    browser.wait_for(
        "the notification socket is open",
        "!!(window.moqui && moqui.webrootVue && moqui.webrootVue.notificationClient && moqui.webrootVue.notificationClient.webSocket \
         && moqui.webrootVue.notificationClient.webSocket.readyState === 1)",
        WAIT,
    );
    let shown = notify_until_shown(
        &mut browser,
        &gate,
        &user_id,
        "Qvt",
        "Array.from(document.querySelectorAll('.q-notification')).map((el) => el.innerText).find((text) => text.indexOf('Gate note Qvt') >= 0) || null",
    );
    assert!(shown.contains("Gate note Qvt"), "{shown}");
    assert_eq!(
        browser.eval(
            "moqui.webrootVue.notifyHistoryList.some((item) => String(item.message).indexOf('Gate note Qvt') >= 0)"
        ),
        true
    );
    assert_clean(&mut browser, "qvt notification");
}

/// A key with modifiers (CDP: 1 Alt, 2 Ctrl, 4 Meta, 8 Shift) as a keydown that types nothing, then its keyup.
fn key_chord(browser: &mut Browser, key: &str, code: &str, key_code: i64, modifiers: i64) {
    for kind in ["rawKeyDown", "keyUp"] {
        browser.call(
            "Input.dispatchKeyEvent",
            serde_json::json!({"type": kind, "key": key, "code": code, "windowsVirtualKeyCode": key_code, "modifiers": modifiers}),
        );
    }
    browser.pump(Duration::from_millis(150));
}

fn ctrl_k(browser: &mut Browser) {
    key_chord(browser, "k", "KeyK", 75, 2);
}

/// Replace the palette's text: select what is there, then type over it.
fn palette_text(browser: &mut Browser, text: &str) {
    browser.eval("(() => { const input = document.getElementById('m-palette-input'); input.focus(); input.select(); return true; })()");
    browser.type_text(text);
}

/// The text of the palette's rows, once one has `shown`.
fn palette_row(browser: &mut Browser, what: &str, shown: &str) -> String {
    let found = browser.wait_for(
        what,
        &format!(
            "Array.from(document.querySelectorAll('#m-palette .q-item')).map((el) => el.innerText).find((text) => text.indexOf({shown:?}) >= 0) || null"
        ),
        WAIT,
    );
    found.as_str().unwrap_or_default().to_string()
}

/// The suite shell of `/qapps` (IronStone impl-drawers-commons M1) over the gallery's shell providers, without BOSS: the
/// launcher and the app title, Ctrl+K and the palette (a screen by title, the gallery's palette source by its `~` prefix,
/// `:` and Enter to open), the `+` menu and the quick-create dialog with the screen context's record, closing when its form
/// submits, and the keys (`?` and the gallery's `g w`). No tray, counters, Mail or Messages without the theme values. The console
/// stays clean. With `MOQUI_SHELL_SHOTS=<dir>` it saves PNGs of the header with the palette and the `+` menu open, light
/// and dark, 1280 and 390 px wide.
#[test]
fn the_suite_shell_palette_quick_create_and_keys_work_in_chromium() {
    let gate = start("browser-shell");
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    sign_in(&mut browser, &base);
    let detail = format!("{base}/qapps/gallery/Detail?enumId=STT_INTERNAL_QUASAR");
    browser.navigate(&detail);
    browser.wait_for(
        "the shell data",
        "!!(window.moqui && moqui.webrootVue && moqui.webrootVue.shellData && moqui.webrootVue.shellData.apps.some((app) => app.name === 'gallery'))",
        WAIT,
    );
    browser.wait_for(
        "the record of the screen context",
        "(moqui.webrootVue.screenRecord || {}).id === 'STT_INTERNAL_QUASAR'",
        WAIT,
    );
    browser.eval(FETCH_SPY);
    // the header: the current app's title, the launcher with the apps, Search; no tray, counters, Mail or Messages without BOSS's
    // theme values, and the notify history stays
    let header = browser.eval("document.getElementById('top').innerText");
    assert!(header.as_str().unwrap().contains("Gallery"), "{header}");
    // the breadcrumbs are the path from the app root: not Applications, nor the app title the header shows
    browser.wait_for(
        "the breadcrumbs",
        "(document.getElementById('m-breadcrumbs') || {}).innerText !== undefined && document.getElementById('m-breadcrumbs').innerText.indexOf('Detail') >= 0",
        WAIT,
    );
    let crumbs = browser.eval("document.getElementById('m-breadcrumbs').innerText");
    let crumbs = crumbs.as_str().unwrap_or_default();
    assert!(
        !crumbs.contains("Applications") && !crumbs.contains("Gallery"),
        "{crumbs}"
    );
    assert_eq!(
        browser.eval(
            "!!document.getElementById('m-launcher') && !!document.getElementById('m-search') && !document.querySelector('[id^=m-counter-]') \
             && !document.getElementById('m-tray') && moqui.webrootVue.trayTabs.length === 0"
        ),
        true
    );
    browser.click("the launcher", "document.getElementById('m-launcher')");
    browser.wait_for(
        "the launcher's apps",
        "!!document.querySelector('.m-launcher .m-launcher-current') && document.querySelector('.m-launcher').innerText.indexOf('All applications') >= 0",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for("the launcher closed", "!document.querySelector('.m-launcher')", WAIT);

    // Ctrl+K: the palette with its input focused; a screen by its title
    ctrl_k(&mut browser);
    browser.wait_for(
        "the palette",
        "!!document.getElementById('m-palette') && document.activeElement && document.activeElement.id === 'm-palette-input'",
        WAIT,
    );
    browser.type_text("widget");
    let row = palette_row(&mut browser, "the Widgets screen", "Widgets");
    assert!(row.contains("Gallery"), "the app is the subtitle: {row}");
    palette_row(&mut browser, "the gallery's palette action", "Open the widget gallery");
    // the gallery's palette source by its prefix: shellSearch with its group only
    palette_text(&mut browser, "~Quasar");
    palette_row(
        &mut browser,
        "a row of the gallery's palette source",
        "Internal Applications - Quasar",
    );
    let searched = fetched(&mut browser, "the palette search", "/shellSearch");
    let url = searched["url"].as_str().unwrap();
    assert!(
        url.contains("groups=GalleryEnumerations") && url.contains("text=Quasar"),
        "{url}"
    );
    assert_eq!(searched["status"], 200, "{searched}");
    assert_eq!(
        browser.eval("Array.from(document.querySelectorAll('#m-palette .q-item__label--header')).map((el) => el.innerText.trim()).join('|')"),
        "Enumerations"
    );
    // : is the screens alone, and Enter opens the first
    palette_text(&mut browser, ":widgets");
    palette_row(&mut browser, "the Widgets screen alone", "Widgets");
    browser.press("Enter");
    browser.wait_for(
        "the Widgets screen",
        "location.pathname === '/qapps/gallery/Widgets'",
        WAIT,
    );
    browser.wait_for("the palette closed", "!document.getElementById('m-palette')", WAIT);
    assert_clean(&mut browser, "the palette");

    // the + menu on a record: the gallery's kind; its dialog gets the record of the screen context and closes on save
    browser.navigate(&detail);
    browser.wait_for(
        "the record of the screen context",
        "!!(moqui.webrootVue && moqui.webrootVue.shellData && (moqui.webrootVue.screenRecord || {}).id === 'STT_INTERNAL_QUASAR')",
        WAIT,
    );
    browser.eval(FETCH_SPY);
    browser.click("the + menu", "document.getElementById('m-quick-create-menu')");
    browser.click(
        "the gallery's quick-create kind",
        &by_text(".q-menu .q-item", "Gallery note"),
    );
    browser.wait_for(
        "the quick-create dialog with the record",
        "!!document.getElementById('m-quick-create') && document.getElementById('m-quick-create').innerText.indexOf('On moqui.basic.Enumeration STT_INTERNAL_QUASAR') >= 0",
        WAIT,
    );
    assert_eq!(
        browser.eval("moqui.webrootVue.assistReach.join(',')"),
        "m-canvas,m-quick-create"
    );
    browser.click("the note field", "document.querySelector('#m-quick-create textarea')");
    browser.type_text("From the gate");
    browser.click("Save", &by_text("#m-quick-create button", "Save"));
    browser.wait_for(
        "the note's message",
        "Array.from(document.querySelectorAll('.q-notification')).some((el) => el.innerText.indexOf('the note would be saved on moqui.basic.Enumeration STT_INTERNAL_QUASAR') >= 0)",
        WAIT,
    );
    browser.wait_for("the dialog closed", "!document.getElementById('m-quick-create')", WAIT);
    let posted = fetched(&mut browser, "the note post", "/saveNote");
    assert_eq!(posted["status"], 200, "{posted}");
    assert!(
        posted["sent"].to_string().contains("recordId=STT_INTERNAL_QUASAR"),
        "{posted}"
    );
    assert_clean(&mut browser, "quick create");

    // the keys: ? lists them, with the gallery's own; g then w goes to the widgets
    browser.eval("document.activeElement && document.activeElement.blur && document.activeElement.blur(), true");
    key_chord(&mut browser, "?", "Slash", 191, 8);
    browser.wait_for(
        "the keys overlay",
        "!!document.getElementById('m-keys-help') && document.getElementById('m-keys-help').innerText.indexOf('Go to the widgets') >= 0",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for("the overlay closed", "!document.getElementById('m-keys-help')", WAIT);
    browser.eval("document.activeElement && document.activeElement.blur && document.activeElement.blur(), true");
    key_chord(&mut browser, "g", "KeyG", 71, 0);
    key_chord(&mut browser, "w", "KeyW", 87, 0);
    browser.wait_for(
        "g w went to the widgets",
        "location.pathname === '/qapps/gallery/Widgets'",
        WAIT,
    );
    assert_clean(&mut browser, "the keys");

    if let Some(dir) = std::env::var_os("MOQUI_SHELL_SHOTS") {
        shell_shots(&mut browser, &detail, std::path::Path::new(&dir));
    }
}

/// The tray panel of the suite shell, with theme values on DEFAULT_QUASAR that name gallery screens as BOSS names its own
/// (`gallery/Panel` for Today, `gallery/Landing` for Notifications, `gallery/List` for Mail, `gallery/Split` for
/// Messages): the counters appear and the notify history goes; Today opens the tray with the Panel archetype inside it, in
/// Assist's reach; a tab is loaded once and kept, the tray stays open across navigation and Esc closes it; a `shellCounts`
/// message sets the badges; below 600 px Mail and Messages are one button. With `MOQUI_SHELL_SHOTS=<dir>` it saves PNGs of
/// the open tray, light and dark, 1280 and 390 px wide.
#[test]
fn the_suite_shell_tray_shows_the_theme_screens_in_chromium() {
    let gate = start_with_data(
        "browser-tray",
        r#"<entity-facade-xml type="seed">
            <moqui.screen.ScreenThemeResource screenThemeId="DEFAULT_QUASAR" sequenceNum="900" resourceTypeEnumId="STRT_TRAY_TODAY" resourceValue="gallery/Panel"/>
            <moqui.screen.ScreenThemeResource screenThemeId="DEFAULT_QUASAR" sequenceNum="901" resourceTypeEnumId="STRT_TRAY_NOTIFICATIONS" resourceValue="gallery/Landing"/>
            <moqui.screen.ScreenThemeResource screenThemeId="DEFAULT_QUASAR" sequenceNum="902" resourceTypeEnumId="STRT_MAIL_SCREEN" resourceValue="gallery/List"/>
            <moqui.screen.ScreenThemeResource screenThemeId="DEFAULT_QUASAR" sequenceNum="903" resourceTypeEnumId="STRT_MESSAGES_SCREEN" resourceValue="gallery/Split"/>
        </entity-facade-xml>"#,
    );
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    sign_in(&mut browser, &base);
    let detail = format!("{base}/qapps/gallery/Detail?enumId=STT_INTERNAL_QUASAR");
    browser.navigate(&detail);
    browser.wait_for(
        "the counters",
        "!!document.getElementById('m-counter-today') && !!document.getElementById('m-counter-notifications') \
         && !!document.getElementById('m-counter-email') && !!document.getElementById('m-counter-messages')",
        WAIT,
    );
    // no Chat tab without its theme value, and the notify history gives way to the Notifications tab
    assert_eq!(
        browser.eval("!document.getElementById('m-counter-chat') && !document.querySelector('#top .q-btn i.q-icon[class*=notifications]') \
             && moqui.webrootVue.trayTabs.map((tab) => tab.key).join(',')"),
        "today,notifications"
    );
    browser.click("Today", "document.getElementById('m-counter-today')");
    browser.wait_for(
        "the Panel archetype in the tray",
        "!!document.getElementById('m-tray') && document.getElementById('m-tray').offsetParent !== null \
         && document.getElementById('m-tray').innerText.indexOf('Overdue') >= 0",
        WAIT,
    );
    assert_eq!(
        browser.eval("moqui.webrootVue.assistReach.join(',')"),
        "m-canvas,m-tray"
    );
    assert_eq!(
        browser.eval("!!document.getElementById('m-canvas').contains(document.getElementById('m-tray')) && Math.round(document.getElementById('m-tray').getBoundingClientRect().width)"),
        400
    );
    // the other tab, then back: Today was loaded once and kept
    browser.click("the Notifications tab", &by_text("#m-tray .q-tab", "Notifications"));
    browser.wait_for(
        "the Landing archetype in the tray",
        "document.getElementById('m-tray').innerText.indexOf('Archetype 3.1') >= 0",
        WAIT,
    );
    browser.eval("(document.querySelectorAll('#m-tray .m-tray-body')[0].firstElementChild.dataset.kept = 'yes'), true");
    browser.click("the Today tab", &by_text("#m-tray .q-tab", "Today"));
    browser.wait_for(
        "Today again, the same content",
        "!!document.querySelector('#m-tray .m-tray-body > [data-kept=yes]') && document.querySelector('#m-tray .m-tray-body > [data-kept=yes]').offsetParent !== null",
        WAIT,
    );
    // a shellCounts message sets the badges
    browser.eval("moqui.webrootVue.onShellNotification({topic: 'shellCounts', message: {today: 4, email: 120, messages: 2, notifications: 0, updates: 9}}), true");
    browser.wait_for(
        "the badges",
        "(document.querySelector('#m-counter-today .q-badge') || {}).innerText === '4' && (document.querySelector('#m-counter-email .q-badge') || {}).innerText === '99+' \
         && (document.querySelector('#m-counter-messages .q-badge') || {}).innerText === '2' && !document.querySelector('#m-counter-notifications .q-badge')",
        WAIT,
    );
    // the tray stays open across navigation; Esc closes it; the Mail and Messages counts are links
    // (a link of the canvas goes through setUrl)
    browser.eval("moqui.webrootVue.setUrl('/qapps/gallery/Landing'), true");
    browser.wait_for(
        "the Landing screen",
        "location.pathname === '/qapps/gallery/Landing' && moqui.webrootVue.loading === 0 \
         && document.getElementById('m-canvas').innerText.indexOf('Archetype 3.1') >= 0",
        WAIT,
    );
    assert_eq!(
        browser.eval("document.getElementById('m-tray').offsetParent !== null"),
        true
    );
    browser.eval("document.activeElement && document.activeElement.blur && document.activeElement.blur(), true");
    browser.press("Escape");
    browser.wait_for(
        "the tray closed",
        "document.getElementById('m-tray').offsetParent === null",
        WAIT,
    );
    // each screen is shown before the next click, so no load is aborted
    browser.click("the Messages count", "document.getElementById('m-counter-messages')");
    browser.wait_for(
        "the Messages screen",
        "location.pathname === '/qapps/gallery/Split' && moqui.webrootVue.loading === 0 \
         && document.getElementById('m-canvas').innerText.indexOf('Archetype 3.8') >= 0",
        WAIT,
    );
    browser.click("the Mail count", "document.getElementById('m-counter-email')");
    browser.wait_for(
        "the Mail screen",
        "location.pathname === '/qapps/gallery/List' && moqui.webrootVue.loading === 0 \
         && document.getElementById('m-canvas').innerText.indexOf('Archetype 3.2') >= 0",
        WAIT,
    );
    // below 600 px Mail and Messages are one button to the mail screen, its badge their sum
    browser.call(
        "Emulation.setDeviceMetricsOverride",
        serde_json::json!({"width": 390, "height": 844, "deviceScaleFactor": 1, "mobile": true}),
    );
    browser
        .eval("moqui.webrootVue.onShellNotification({topic: 'shellCounts', message: {email: 3, messages: 2}}), true");
    browser.wait_for(
        "one mail button",
        "!!document.getElementById('m-counter-mail') && !document.getElementById('m-counter-email') && !document.getElementById('m-counter-messages') \
         && (document.querySelector('#m-counter-mail .q-badge') || {}).innerText === '5'",
        WAIT,
    );
    browser.eval("moqui.webrootVue.setUrl('/qapps/gallery/Landing'), true");
    browser.wait_for(
        "the Landing screen",
        "location.pathname === '/qapps/gallery/Landing' && moqui.webrootVue.loading === 0 \
         && document.getElementById('m-canvas').innerText.indexOf('Archetype 3.1') >= 0",
        WAIT,
    );
    browser.click("the one mail button", "document.getElementById('m-counter-mail')");
    browser.wait_for(
        "the Mail screen",
        "location.pathname === '/qapps/gallery/List' && moqui.webrootVue.loading === 0 \
         && document.getElementById('m-canvas').innerText.indexOf('Archetype 3.2') >= 0",
        WAIT,
    );
    browser.call("Emulation.clearDeviceMetricsOverride", serde_json::json!({}));
    assert_clean(&mut browser, "the tray");

    if let Some(dir) = std::env::var_os("MOQUI_SHELL_SHOTS") {
        use base64::Engine as _;
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).unwrap();
        for (width, height) in [(1280, 900), (390, 844)] {
            browser.call(
                "Emulation.setDeviceMetricsOverride",
                serde_json::json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": width < 600}),
            );
            browser.navigate(&detail);
            browser.wait_for("the counters", "!!document.getElementById('m-counter-today')", WAIT);
            for dark in [false, true] {
                let mode = if dark { "dark" } else { "light" };
                browser.eval(&format!(
                    "moqui.webrootVue.$q.dark.set({dark}), moqui.webrootVue.openTray('today'), true"
                ));
                browser.wait_for(
                    "the tray",
                    "!!document.getElementById('m-tray') && document.getElementById('m-tray').innerText.indexOf('Overdue') >= 0",
                    WAIT,
                );
                browser.pump(Duration::from_millis(500));
                let shot = browser.call("Page.captureScreenshot", serde_json::json!({"format": "png"}));
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(shot["data"].as_str().unwrap())
                    .unwrap();
                std::fs::write(dir.join(format!("tray-{mode}-{width}.png")), bytes).unwrap();
            }
        }
        browser.call("Emulation.clearDeviceMetricsOverride", serde_json::json!({}));
    }
}

/// The record panels and the rich-text editor (IronStone impl-drawers-commons M3), with theme values on DEFAULT_QUASAR that name
/// the gallery's own `Chatter` and `RecordFiles` subscreens as BOSS names its `Shared/` ones: the Detail screen's `chatter` and
/// `record-files` elements load them under the current app root with the record (the shell data says the gallery mounts them),
/// the panel's `m-record-card` asks its card URL and the Log note form is an `m-rich-text` (light): typed text, a word made bold
/// from the toolbar and a mention from the `@` list of the stub mention transition are posted as the editor's HTML, which the
/// note service's `allow-html="safe"` parameter keeps, mention and all. On the Widgets screen the full editor's plain-text
/// toggle turns its HTML into text. With the Tiptap script blocked, the editor is the html editor of `m-text-area`.
#[test]
fn the_record_panels_and_the_rich_text_editor_work_in_chromium() {
    let gate = start_with_data(
        "browser-records",
        r#"<entity-facade-xml type="seed">
            <moqui.screen.ScreenThemeResource screenThemeId="DEFAULT_QUASAR" sequenceNum="910" resourceTypeEnumId="STRT_CHATTER_SCREEN" resourceValue="Chatter"/>
            <moqui.screen.ScreenThemeResource screenThemeId="DEFAULT_QUASAR" sequenceNum="911" resourceTypeEnumId="STRT_RECORD_FILES_SCREEN" resourceValue="RecordFiles"/>
        </entity-facade-xml>"#,
    );
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    sign_in(&mut browser, &base);
    browser.navigate(&format!("{base}/qapps/gallery/Detail?enumId=STT_INTERNAL_QUASAR"));
    browser.eval(FETCH_SPY);
    // the panels of the current app root, with the record and the element's hint
    browser.wait_for(
        "the Chatter panel",
        "!!document.getElementById('DetailChatter') && document.getElementById('DetailChatter').innerText.indexOf('opens on log') >= 0 \
         && !!document.querySelector('#DetailChatter .m-rich-text[data-state=ready] .ProseMirror')",
        WAIT,
    );
    assert_eq!(
        browser.eval(
            "JSON.stringify(moqui.webrootVue.shellData.apps.find((app) => app.name === 'gallery').recordScreens)"
        ),
        r#"{"chatter":{"url":"/qapps/gallery/Chatter","renderModes":["all"]},"recordFiles":{"url":"/qapps/gallery/RecordFiles","renderModes":["all"]}}"#
    );
    let loaded = fetched(&mut browser, "the Chatter request", "/apps/gallery/Chatter.qvt");
    let url = loaded["url"].as_str().unwrap();
    assert!(
        url.contains("recordEntity=moqui.basic.Enumeration")
            && url.contains("recordId=STT_INTERNAL_QUASAR")
            && url.contains("appRoot=gallery")
            && url.contains("defaultTab=log"),
        "{url}"
    );
    browser.wait_for(
        "the record files panel",
        "!!document.getElementById('DetailFiles') && document.getElementById('DetailFiles').innerText.indexOf('spec-sheet.pdf') >= 0",
        WAIT,
    );
    // the record card of the panel, from its card URL
    browser.wait_for(
        "the record card",
        "(document.querySelector('#DetailChatter .m-record-card') || {innerText: ''}).innerText.indexOf('Active') >= 0",
        WAIT,
    );
    let card = browser.eval("document.querySelector('#DetailChatter .m-record-card').innerText");
    assert!(card.as_str().unwrap().contains("STT_INTERNAL_QUASAR"), "{card}");
    assert!(card.as_str().unwrap().contains("Enumeration"), "{card}");
    assert_clean(&mut browser, "the record panels");

    // the light editor: type, bold a word from the toolbar, mention a user from the @ list, submit
    browser.click(
        "the note editor",
        "document.querySelector('#DetailChatter .ProseMirror')",
    );
    browser.type_text("Hello ");
    browser.click(
        "Bold",
        "document.querySelector('#DetailChatter .m-rich-text [data-command=bold]')",
    );
    browser.type_text("bold");
    browser.click(
        "Bold again",
        "document.querySelector('#DetailChatter .m-rich-text [data-command=bold]')",
    );
    browser.type_text(" team @jo");
    browser.wait_for(
        "the mention list",
        &by_text(".m-rich-text-mentions .q-item", "John Doe"),
        WAIT,
    );
    let searched = fetched(&mut browser, "the mention search", "/mentionSearch");
    assert!(searched["url"].as_str().unwrap().contains("term=jo"), "{searched}");
    browser.click("John Doe", &by_text(".m-rich-text-mentions .q-item", "John Doe"));
    browser.wait_for(
        "the mention in the editor",
        "!!document.querySelector('#DetailChatter .ProseMirror [data-mention-id=EX_JOHN_DOE]') && document.querySelector('.m-rich-text-mentions').offsetParent === null",
        WAIT,
    );
    let html = browser.eval("document.querySelector('#DetailChatter input[type=hidden][name=noteText]').value");
    let html = html.as_str().unwrap().to_string();
    assert!(
        html.starts_with("<p>Hello <strong>bold</strong> team <span data-mention-id=\"EX_JOHN_DOE\">@John Doe</span>"),
        "{html}"
    );
    browser.click("Log", &by_text("#DetailChatter button", "Log"));
    browser.wait_for(
        "the note's message",
        "Array.from(document.querySelectorAll('.q-notification')).some((el) => el.innerText.indexOf('mentioning EX_JOHN_DOE') >= 0)",
        WAIT,
    );
    let posted = fetched(&mut browser, "the note post", "/logNote");
    assert_eq!(posted["status"], 200, "{posted}");
    let sent = posted["sent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(sent.contains(&format!("noteText={html}")), "{sent:?}");
    assert!(sent.contains(&"recordId=STT_INTERNAL_QUASAR".to_string()), "{sent:?}");
    // what the safe cleaning kept: the editor's HTML as it was sent
    let answer: serde_json::Value = serde_json::from_str(posted["body"].as_str().unwrap()).unwrap();
    assert_eq!(answer["noteText"], html.as_str(), "{answer}");
    assert_eq!(answer["mentionIds"], serde_json::json!(["EX_JOHN_DOE"]), "{answer}");
    assert_clean(&mut browser, "the rich-text note");

    // the full editor of the Widgets screen: its toolbar, and the plain-text toggle
    browser.navigate(&format!("{base}/qapps/gallery/Widgets"));
    browser.wait_for(
        "the rich editors",
        "document.querySelectorAll('.m-rich-text[data-state=ready] .ProseMirror').length === 2",
        WAIT,
    );
    assert_eq!(
        browser.eval("Array.from(document.querySelector('.m-rich-text').querySelectorAll('[data-command]')).map((el) => el.dataset.command).join(',')"),
        "bold,italic,strike,bulletList,orderedList,link,image,code,codeBlock,blockquote,undo,redo,plain"
    );
    browser.click(
        "Plain text",
        "document.querySelector('.m-rich-text [data-command=plain]')",
    );
    browser.wait_for(
        "the plain text",
        "!!document.querySelector('.m-rich-text textarea.m-rich-text-plain') && document.querySelector('.m-rich-text textarea.m-rich-text-plain').value.indexOf('Rich text\\n\\nThe Tiptap editor: marks, lists, links (https://tiptap.dev) and code.\\n\\n- Type @ for a mention') === 0",
        WAIT,
    );
    assert_clean(&mut browser, "the full editor");

    if let Some(dir) = std::env::var_os("MOQUI_SHELL_SHOTS") {
        record_shots(&mut browser, &base, std::path::Path::new(&dir));
    }

    // with the Tiptap script blocked, the html editor stands in
    browser.call("Network.enable", serde_json::json!({}));
    browser.call(
        "Network.setBlockedURLs",
        serde_json::json!({"urls": ["*tiptap.min.js*"]}),
    );
    browser.navigate(&format!("{base}/qapps/gallery/Widgets"));
    browser.wait_for(
        "the html editor in place of the rich ones",
        "document.querySelectorAll('.m-rich-text[data-state=failed] .q-editor').length === 2",
        WAIT,
    );
    let problems = browser.take_problems();
    assert!(
        !problems.is_empty()
            && problems
                .iter()
                .all(|problem| format!("{problem:?}").to_lowercase().contains("tiptap")),
        "only the blocked script is reported: {problems:#?}"
    );
}

/// The gallery Split: a container-split side by side at 1280 px (the side at its width, both panes shown), stacked at 390 px with the
/// list alone, a country chosen showing it with the back button, and back to the list. The gallery Wizard: Next with an empty
/// customer name stays with the error by the field, Next with a name goes to Options with the step in the URL, Back keeps the
/// draft, and Create customer on the review goes back to the first step.
#[test]
fn the_split_stacks_and_the_wizard_steps_in_chromium() {
    let gate = start("browser-layout");
    let base = format!("http://{}", gate.server.address());
    let mut browser = Browser::launch();
    sign_in(&mut browser, &base);
    let metrics = |browser: &mut Browser, width: i64, height: i64| {
        browser.call(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": width < 600}),
        );
    };
    let shown = |selector: &str| {
        format!(
            "(() => {{ const el = document.querySelector({selector:?}); return !!el && el.offsetParent !== null; }})()"
        )
    };

    metrics(&mut browser, 1280, 900);
    browser.navigate(&format!("{base}/qapps/gallery/Split"));
    browser.wait_for(
        "the split side by side",
        "!!document.querySelector('#CountrySplit .q-splitter') && document.getElementById('CountrySplit').dataset.stacked === 'false' \
         && document.querySelector('#CountrySplit .m-split-main').innerText.indexOf('States and regions') >= 0",
        WAIT,
    );
    let side = browser
        .eval("Math.round(document.querySelector('#CountrySplit .q-splitter__before').getBoundingClientRect().width)");
    assert_eq!(side, 320, "the side at its width");
    assert_eq!(browser.eval(&shown("#CountrySplit .m-split-side")), true);
    assert_eq!(
        browser.eval("!!document.querySelector('#CountrySplit .m-split-back')"),
        false
    );
    assert_clean(&mut browser, "the split side by side");

    metrics(&mut browser, 390, 844);
    browser.navigate(&format!("{base}/qapps/gallery/Split"));
    browser.wait_for(
        "the split stacked with the list alone",
        "!!document.getElementById('CountrySplit') && document.getElementById('CountrySplit').dataset.stacked === 'true' \
         && document.querySelector('#CountrySplit .m-split-side').offsetParent !== null \
         && document.querySelector('#CountrySplit .m-split-main').offsetParent === null",
        WAIT,
    );
    assert!(
        browser
            .eval("document.documentElement.scrollWidth <= window.innerWidth + 1")
            .as_bool()
            .unwrap(),
        "no horizontal scroll of the page at 390 px"
    );
    browser.click(
        "a country",
        &by_text(
            "#CountrySplit .m-split-side a, #CountrySplit .m-split-side button",
            "Albania",
        ),
    );
    browser.wait_for(
        "the country alone with the back button",
        "location.search.indexOf('geoId=ALB') >= 0 && document.querySelector('#CountrySplit .m-split-side').offsetParent === null \
         && document.querySelector('#CountrySplit .m-split-main').offsetParent !== null \
         && document.querySelector('#CountrySplit .m-split-main').innerText.indexOf('Albania') >= 0 && !!document.querySelector('#CountrySplit .m-split-back')",
        WAIT,
    );
    browser.click("back", "document.querySelector('#CountrySplit .m-split-back')");
    browser.wait_for(
        "the list again",
        "location.search.indexOf('geoId=') < 0 && document.querySelector('#CountrySplit .m-split-side').offsetParent !== null",
        WAIT,
    );
    assert_clean(&mut browser, "the split stacked");
    browser.call("Emulation.clearDeviceMetricsOverride", serde_json::json!({}));

    browser.navigate(&format!("{base}/qapps/gallery/Wizard"));
    browser.eval(FETCH_SPY);
    browser.wait_for(
        "the wizard on its first step",
        "!!document.querySelector('#CustomerWizard .q-stepper') && document.querySelectorAll('#CustomerWizard .q-stepper__tab').length === 3 \
         && !!document.querySelector('#CustomerWizard [name=customerName]')",
        WAIT,
    );
    assert_eq!(
        browser.eval("Array.from(document.querySelectorAll('#CustomerWizard .q-stepper__tab')).map((el) => el.innerText.trim().split('\\n').pop()).join(',')"),
        "Customer,Options,Review"
    );
    assert_eq!(
        browser.eval("!!document.querySelector('#CustomerWizard .m-wizard-back')"),
        false
    );
    // an empty name: the error by the field, and the wizard stays
    browser.eval(
        "(() => { const input = document.querySelector('#CustomerWizard [name=customerName]'); input.focus(); input.select(); return true; })()",
    );
    browser.press("Delete");
    browser.eval(
        "(() => { const input = document.querySelector('#CustomerWizard [name=customerName]'); input.value = ''; input.dispatchEvent(new Event('input', {bubbles: true})); return true; })()",
    );
    browser.click("Next", "document.querySelector('#CustomerWizard .m-wizard-next')");
    browser.wait_for(
        "the error by the name",
        "(document.querySelector('#CustomerWizard .q-field--error') || {innerText: ''}).innerText.indexOf('Enter the customer name') >= 0",
        WAIT,
    );
    let answer = fetched(&mut browser, "the step post", "/saveCustomer");
    assert_eq!(answer["status"], 500, "{answer}");
    assert_eq!(
        browser.eval("/\\/gallery\\/Wizard(\\/Customer)?$/.test(location.pathname)"),
        true,
        "still on the first step"
    );
    let problems = browser.take_problems();
    assert!(
        problems.iter().all(|problem| format!("{problem:?}").contains("500")),
        "only the 500 of the answer: {problems:#?}"
    );
    // a name: Next goes to Options, the step in the URL
    browser.click(
        "the name",
        "document.querySelector('#CustomerWizard [name=customerName]')",
    );
    browser.type_text("Gate Customer");
    browser.click("Next", "document.querySelector('#CustomerWizard .m-wizard-next')");
    browser.wait_for(
        "the second step",
        "location.pathname.endsWith('/gallery/Wizard/Options') && !!document.querySelector('#CustomerWizard [name=countryGeoId]') \
         && !!document.querySelector('#CustomerWizard .m-wizard-back')",
        WAIT,
    );
    assert_eq!(
        browser.eval("document.querySelectorAll('#CustomerWizard .q-stepper__tab--done').length"),
        1,
        "the first step is done"
    );
    // Back keeps what the step saved
    browser.click("Back", "document.querySelector('#CustomerWizard .m-wizard-back')");
    browser.wait_for(
        "the first step with its draft",
        "location.pathname.endsWith('/gallery/Wizard/Customer') && (document.querySelector('#CustomerWizard [name=customerName]') || {value: ''}).value === 'Gate Customer'",
        WAIT,
    );
    browser.click("Next", "document.querySelector('#CustomerWizard .m-wizard-next')");
    browser.wait_for("Options again", "location.pathname.endsWith('/gallery/Wizard/Options') && !!document.querySelector('#CustomerWizard [name=countryGeoId]')", WAIT);
    browser.click("Next", "document.querySelector('#CustomerWizard .m-wizard-next')");
    browser.wait_for(
        "the review",
        "location.pathname.endsWith('/gallery/Wizard/Review') && document.querySelector('#CustomerWizard .m-wizard-body').innerText.indexOf('Gate Customer') >= 0",
        WAIT,
    );
    let options = fetched(&mut browser, "the options post", "/saveOptions");
    assert_eq!(options["status"], 200, "{options}");
    assert_eq!(
        browser.eval("document.querySelector('#CustomerWizard .m-wizard-next .block').innerText.trim()"),
        "Create customer"
    );
    browser.click(
        "Create customer",
        "document.querySelector('#CustomerWizard .m-wizard-next')",
    );
    browser.wait_for(
        "back on the first step with the message",
        "location.pathname.endsWith('/gallery/Wizard/Customer') && Array.from(document.querySelectorAll('.q-notification')).some((el) => el.innerText.indexOf('Gate Customer created') >= 0)",
        WAIT,
    );
    assert_clean(&mut browser, "the wizard");
}

/// The Detail screen's Chatter column with the @ list of the note editor open, and the rich editors of the Widgets screen, light
/// and dark, 1280 and 390 px wide, as PNGs in `dir`.
fn record_shots(browser: &mut Browser, base: &str, dir: &std::path::Path) {
    use base64::Engine as _;
    std::fs::create_dir_all(dir).unwrap();
    let save = |browser: &mut Browser, name: &str| {
        browser.pump(Duration::from_millis(500));
        let shot = browser.call("Page.captureScreenshot", serde_json::json!({"format": "png"}));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap())
            .unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    };
    for (width, height) in [(1280, 900), (390, 844)] {
        browser.call(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": width < 600}),
        );
        for dark in [false, true] {
            let mode = if dark { "dark" } else { "light" };
            browser.navigate(&format!("{base}/qapps/gallery/Detail?enumId=STT_INTERNAL_QUASAR"));
            browser.wait_for(
                "the Chatter panel",
                "!!document.querySelector('#DetailChatter .m-rich-text[data-state=ready] .ProseMirror') \
                 && (document.querySelector('#DetailChatter .m-record-card') || {innerText: ''}).innerText.indexOf('Active') >= 0",
                WAIT,
            );
            browser.eval(&format!("moqui.webrootVue.$q.dark.set({dark}), true"));
            browser.click(
                "the note editor",
                "document.querySelector('#DetailChatter .ProseMirror')",
            );
            browser.type_text("Reviewed with @jo");
            browser.wait_for(
                "the mention list",
                &by_text(".m-rich-text-mentions .q-item", "John Doe"),
                WAIT,
            );
            browser.eval("document.getElementById('DetailChatter').scrollIntoView({block: 'start'}), true");
            save(browser, &format!("chatter-{mode}-{width}.png"));
            browser.press("Escape");
            browser.navigate(&format!("{base}/qapps/gallery/Widgets"));
            browser.wait_for(
                "the rich editors",
                "document.querySelectorAll('.m-rich-text[data-state=ready] .ProseMirror').length === 2",
                WAIT,
            );
            browser.eval(&format!(
                "moqui.webrootVue.$q.dark.set({dark}), document.querySelector('.m-rich-text').scrollIntoView({{block: 'start'}}), true"
            ));
            save(browser, &format!("rich-text-{mode}-{width}.png"));
        }
    }
    browser.call("Emulation.clearDeviceMetricsOverride", serde_json::json!({}));
    browser.eval("moqui.webrootVue.$q.dark.set(false), true");
}

/// The header with the palette open and with the `+` menu open, light and dark, 1280 and 390 px wide, as PNGs in `dir`.
fn shell_shots(browser: &mut Browser, page: &str, dir: &std::path::Path) {
    use base64::Engine as _;
    std::fs::create_dir_all(dir).unwrap();
    let save = |browser: &mut Browser, name: &str| {
        browser.pump(Duration::from_millis(500));
        let shot = browser.call("Page.captureScreenshot", serde_json::json!({"format": "png"}));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap())
            .unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    };
    for (width, height) in [(1280, 900), (390, 844)] {
        browser.call(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": width < 600}),
        );
        browser.navigate(page);
        browser.wait_for(
            "the shell data",
            "!!(window.moqui && moqui.webrootVue && moqui.webrootVue.shellData && moqui.webrootVue.screenRecord)",
            WAIT,
        );
        for dark in [false, true] {
            let mode = if dark { "dark" } else { "light" };
            browser.eval(&format!("moqui.webrootVue.$q.dark.set({dark}), true"));
            ctrl_k(browser);
            browser.wait_for(
                "the palette",
                "document.activeElement && document.activeElement.id === 'm-palette-input'",
                WAIT,
            );
            browser.type_text("gallery");
            palette_row(browser, "the gallery screens", "Gallery");
            browser.wait_for(
                "the palette search",
                "Array.from(document.querySelectorAll('#m-palette .q-item__label--header')).some((el) => el.innerText.indexOf('Enumerations') >= 0) \
                 || !document.querySelector('#m-palette .q-spinner')",
                WAIT,
            );
            save(browser, &format!("palette-{mode}-{width}.png"));
            browser.press("Escape");
            browser.wait_for("the palette closed", "!document.getElementById('m-palette')", WAIT);
            browser.click("the + menu", "document.getElementById('m-quick-create-menu')");
            browser.wait_for("the + menu open", "!!document.querySelector('.q-menu')", WAIT);
            save(browser, &format!("quick-create-{mode}-{width}.png"));
            browser.press("Escape");
            browser.wait_for("the + menu closed", "!document.querySelector('.q-menu')", WAIT);
        }
    }
    browser.call("Emulation.clearDeviceMetricsOverride", serde_json::json!({}));
    // dark mode was set on the page only, not saved as the user's preference
    browser.eval("moqui.webrootVue.$q.dark.set(false), true");
}

/// The user id of a seeded user.
fn user_id_of(gate: &common::Gate, username: &str) -> String {
    use moqui_lib::object::Value;
    let factory = gate.server.factory();
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let authz = ctx.disable_authz();
    let row = factory
        .entity()
        .unwrap()
        .find("moqui.security.UserAccount")
        .unwrap()
        .eq("username", Value::string(username))
        .one()
        .unwrap()
        .unwrap();
    let user_id = row.get("userId").and_then(Value::as_str).unwrap_or("").to_string();
    drop(authz);
    drop(entered);
    user_id
}

/// Send a notification to the user through the core API until the page shows it: the socket's subscription is applied on the
/// service pool after it opens, so the first sends can come before it. Returns the shown text.
fn notify_until_shown(browser: &mut Browser, gate: &common::Gate, user_id: &str, label: &str, shown: &str) -> String {
    let deadline = std::time::Instant::now() + WAIT;
    let mut sent = 0;
    while std::time::Instant::now() < deadline {
        sent += 1;
        let factory = gate.server.factory();
        let ctx = factory.new_context();
        let entered = ctx.enter();
        let authz = ctx.disable_authz();
        let mut note = moqui_core::notify::NotificationMessage::new();
        note.topic("GateNotify")
            .user_id(user_id)
            .title(format!("Gate note {label} {sent}"))
            .link("/qapps/System")
            .notification_type("success")
            .show_alert(true)
            .persist(false)
            .message_json("{\"n\":1}")
            .send()
            .unwrap();
        drop(authz);
        drop(entered);
        let until = std::time::Instant::now() + Duration::from_millis(750);
        while std::time::Instant::now() < until {
            browser.pump(Duration::from_millis(100));
            if let Ok(serde_json::Value::String(text)) = browser.try_eval(shown) {
                return text;
            }
        }
    }
    panic!("the {label} page did not show a notification after {sent} sends");
}

/// The authorization row the qvt tests change: its action from the database.
fn authz_action(gate: &common::Gate) -> String {
    use moqui_lib::object::Value;
    let factory = gate.server.factory();
    let ctx = factory.new_context();
    let entered = ctx.enter();
    let authz = ctx.disable_authz();
    let row = factory
        .entity()
        .unwrap()
        .find("moqui.security.ArtifactAuthz")
        .unwrap()
        .eq("artifactAuthzId", Value::string("SYSTEM_APP_ALL"))
        .one()
        .unwrap()
        .unwrap();
    let action = row
        .get("authzActionEnumId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    drop(authz);
    drop(entered);
    action
}

/// The fixture through the `/qapps` shell (server-merged, or server-static when the gate marks it): the shell mounts, the screen renders, a
/// Quasar menu chooses a value, a dialog opens and closes, and a row's m-form posts with the session token.
fn qapps_fixture(browser: &mut Browser, base: &str, gate: &common::Gate, description: &str, action: (&str, &str)) {
    let (action_id, action_label) = action;
    let page = format!("{base}/qapps{FIXTURE}?artifactGroupId=SYSTEM_APP");
    browser.navigate(&page);
    browser.wait_for(
        "the shell mounted",
        "!!(window.moqui && moqui.webrootVue) && document.getElementById('apps-root').style.display !== 'none'",
        WAIT,
    );
    browser.wait_for(
        "the form-single shows its value",
        &format!("Array.from(document.querySelectorAll('input')).some((el) => el.value === {description:?})"),
        WAIT,
    );
    browser.wait_for(
        "the rows of the authorizations",
        "document.body.innerText.indexOf('SYSTEM_APP_ADMIN') >= 0",
        WAIT,
    );
    browser.eval(FETCH_SPY);
    // a single m-drop-down shows its option's label as the value of its input (fill-input), which innerText leaves out
    let text = browser.eval(
        "document.body.innerText + '\\n' + Array.from(document.querySelectorAll('input')).map((el) => el.value).join('\\n')",
    );
    let text = text.as_str().unwrap();
    for wanted in [
        "Artifact Group Detail",
        "SYSTEM_APP_ALL",
        "Administrators (full access) -",
        "Always Allow (overrides Deny)",
        "component://tools/screen/System.xml",
    ] {
        assert!(text.contains(wanted), "{wanted} in {text}");
    }
    for literal in ["{{", "${", "ec."] {
        assert!(!text.contains(literal), "{literal} in {text}");
    }
    // the m-form is a QForm, and Chart.js and Mermaid are not loaded on a page that does not use them
    assert_eq!(
        browser.eval("document.getElementById('ArtifactGroupForm').classList.contains('q-form')"),
        true
    );
    assert_eq!(
        browser.eval("typeof window.Chart + typeof window.mermaid"),
        "undefinedundefined"
    );

    // a dialog: its button opens a QDialog with a drop-down whose options the server gave
    browser.click("the add authorization button", &by_text("button", "Add Authorization"));
    browser.wait_for(
        "the dialog",
        "Array.from(document.querySelectorAll('.q-dialog')).some((el) => el.innerText.indexOf('User Group') >= 0)",
        WAIT,
    );
    browser.click(
        "the user group drop-down",
        "document.querySelector('.q-dialog [id=\"CreateArtifactAuthz_userGroupId\"]')",
    );
    browser.wait_for(
        "the options of the user group",
        "Array.from(document.querySelectorAll('.q-menu .q-item')).some((el) => el.innerText.indexOf('Administrators (full access)') >= 0)",
        WAIT,
    );
    // Escape closes the menu, and the close button of the dialog the dialog
    browser.press("Escape");
    browser.wait_for(
        "the menu closed",
        "document.querySelectorAll('.q-menu').length === 0",
        WAIT,
    );
    if browser.eval("document.querySelectorAll('.q-dialog').length > 0") == true {
        browser.click("the close button of the dialog", &by_text(".q-dialog button", "close"));
    }
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );

    // a row of the list: choose another action in its Quasar menu and post the row
    browser.click(
        "the action of the second authorization",
        "document.getElementById('ArtifactAuthzList_authzActionEnumId_1')",
    );
    browser.click(
        "the new action",
        &format!("Array.from(document.querySelectorAll('.q-menu .q-item')).find((el) => el.innerText.trim() === {action_label:?})"),
    );
    browser.wait_for(
        "the menu closed",
        "document.querySelectorAll('.q-menu').length === 0",
        WAIT,
    );
    browser.click(
        "the update button of the row",
        "Array.from(document.querySelectorAll('[id=\"ArtifactAuthzList_1\"] button')).find((el) => el.innerText.trim() === 'Update')",
    );
    let posted = fetched(browser, "the post of the row", "/updateArtifactAuthz");
    assert_eq!(posted["method"], "POST", "{posted}");
    assert_eq!(posted["status"], 200, "{posted}");
    browser.wait_for("the row was stored", "window.__gate.fetches.length > 0", WAIT);
    let deadline = std::time::Instant::now() + WAIT;
    while authz_action(gate) != action_id {
        assert!(
            std::time::Instant::now() < deadline,
            "the row post did not store {action_id}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    browser.pump(Duration::from_millis(500));
    assert_clean(browser, &page);
}

/// Server-merged qvt widgets in the shell: Chart.js and Mermaid load on first use, click to edit is a QPopupEdit, a date comes from
/// a QDate popup, the drop-down from a QMenu, the form posts in the background, the column dialog opens, and the list pages.
fn qapps_widgets(browser: &mut Browser, base: &str) {
    let page = format!("{base}/qapps/GateWidgets");
    browser.navigate(&page);
    browser.wait_for(
        "the screen",
        "document.body.innerText.indexOf('Gate widgets') >= 0",
        WAIT,
    );
    browser.eval(FETCH_SPY);
    browser.wait_for(
        "Chart.js drew the chart",
        "!!(window.Chart && Chart.getChart(document.querySelector('#GateCharts canvas')))",
        WAIT,
    );
    browser.wait_for(
        "Mermaid drew the diagram",
        "!!(window.mermaid && document.querySelector('#GateCharts svg') && document.querySelector('#GateCharts svg').textContent.indexOf('GateB') >= 0)",
        WAIT,
    );

    // click to edit
    browser.click("the editable text", "document.getElementById('GateEditable')");
    browser.wait_for(
        "the popup editor",
        "!!document.querySelector('.q-popup-edit input')",
        WAIT,
    );
    browser.eval(
        "(() => { const input = document.querySelector('.q-popup-edit input'); input.value = 'Edited in Quasar';          input.dispatchEvent(new Event('input', {bubbles: true})); return true; })()",
    );
    browser.click("the save button", &by_text(".q-popup-edit button", "Save"));
    let saved = fetched(browser, "the editable post", "/saveValue");
    assert_eq!(saved["status"], 200, "{saved}");
    let body: serde_json::Value = serde_json::from_str(saved["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["saved"], "Edited in Quasar", "{saved}");
    browser.wait_for(
        "the new text",
        "document.getElementById('GateEditable').innerText.trim() === 'Edited in Quasar'",
        WAIT,
    );

    // the note, a date from the QDate popup, and Beta from the drop-down's menu
    browser.click("the note", "document.querySelector('[id=\"NoteForm_note\"]')");
    browser.type_text("hello");
    browser.click(
        "the calendar button of the date",
        "(document.querySelector('[id=\"NoteForm_when\"]') || {closest: () => null}).closest('.q-field').querySelector('.q-icon, button')",
    );
    browser.wait_for("the date popup", "!!document.querySelector('.q-date')", WAIT);
    browser.click("day 15", &by_text(".q-date__calendar-item button", "15"));
    browser.wait_for(
        "the date",
        "/^\\d{4}-\\d\\d-15$/.test(document.querySelector('[id=\"NoteForm_when\"]').value)",
        WAIT,
    );
    browser.press("Escape");
    browser.click("the kind drop-down", "document.querySelector('[id=\"NoteForm_kind\"]')");
    browser.click(
        "Beta",
        "Array.from(document.querySelectorAll('.q-menu .q-item')).find((el) => el.innerText.trim() === 'Beta')",
    );
    browser.wait_for(
        "the menu closed",
        "document.querySelectorAll('.q-menu').length === 0",
        WAIT,
    );
    browser.click("the send button", &by_text("button", "Send"));
    let sent = fetched(browser, "the form post", "/saveNote");
    assert_eq!(sent["status"], 200, "{sent}");
    let body: serde_json::Value = serde_json::from_str(sent["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["note"], "hello", "{sent}");
    assert_eq!(body["kind"], "B", "{sent}");
    assert!(body["when"].as_str().unwrap().ends_with("-15"), "{sent}");
    browser.wait_for(
        "the message of the form",
        "document.body.innerText.indexOf('Note sent') >= 0",
        WAIT,
    );

    // a validation error from the transition is drawn on its Quasar field
    browser.click("the code", "document.querySelector('[id=\"CodeForm_code\"]')");
    browser.type_text("abc");
    field_error_by_its_field(
        browser,
        &by_text("button", "Check"),
        "Array.from(document.querySelectorAll('.q-field--error')).some((el) => el.innerText.indexOf('Code needs a digit') >= 0)",
        "Array.from(document.querySelectorAll('.q-notification')).some((el) => el.innerText.indexOf('Code needs a digit') >= 0)",
    );
    browser.type_text("1");
    browser.wait_for(
        "the error cleared by a new value",
        "document.querySelectorAll('.q-field--error').length === 0",
        WAIT,
    );

    // the column dialog
    browser.click("the columns button", &by_text("button", "view_column\nColumns"));
    browser.wait_for(
        "the column dialog",
        "Array.from(document.querySelectorAll('.q-dialog')).some((el) => el.innerText.indexOf('Enum ID') >= 0)",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );

    // the pages of the list
    browser.click("page 2", &by_text(".q-pagination button", "2"));
    browser.wait_for("page 2", "/\\b21-40 \\/ \\d+/.test(document.body.innerText)", WAIT);
    browser.pump(Duration::from_millis(500));
    assert_clean(browser, &page);
}

/// A background submit whose transition leaves a validation error for `code`: the error shows by the field (`shown` finds it) and is
/// not also a toast (`toast` finds one). The 500 of the answer is the only problem on the console.
fn field_error_by_its_field(browser: &mut Browser, submit: &str, shown: &str, toast: &str) {
    browser.click("the check button", submit);
    let answer = fetched(browser, "the check post", "/checkCode");
    assert_eq!(answer["status"], 500, "{answer}");
    let body: serde_json::Value = serde_json::from_str(answer["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["validationErrors"][0]["field"], "code", "{answer}");
    assert_eq!(body["validationErrors"][0]["fieldPretty"], "Code", "{answer}");
    browser.wait_for("the error by the field", shown, WAIT);
    browser.pump(Duration::from_millis(300));
    assert_eq!(
        browser.eval(toast),
        serde_json::Value::Bool(false),
        "the field error is not also a toast"
    );
    let problems = browser.take_problems();
    assert!(
        problems.iter().all(|problem| format!("{problem:?}").contains("500")),
        "only the 500 of the answer: {problems:#?}"
    );
}

/// The server-static fixture: the template is a script that registers a render function, and the data comes from `/actions`.
fn static_fixture(browser: &mut Browser, base: &str) {
    let page = format!("{base}/qapps{FIXTURE}?artifactGroupId=SYSTEM_APP");
    browser.navigate(&page);
    browser.wait_for(
        "the form-single shows its value from /actions",
        "Array.from(document.querySelectorAll('input')).some((el) => el.value === 'System App (via root screen)')",
        WAIT,
    );
    browser.wait_for(
        "the rows from /actions/ArtifactAuthzList",
        "document.body.innerText.indexOf('SYSTEM_APP_ADMIN') >= 0",
        WAIT,
    );
    let path = format!("/apps{FIXTURE}");
    assert_eq!(
        browser.eval(&format!("typeof (moqui.qvtStatic || {{}})[{path:?}]")),
        "function"
    );
    let resources = browser.eval("performance.getEntriesByType('resource').map((entry) => entry.name)");
    let resources: Vec<&str> = resources
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|name| name.as_str())
        .collect();
    assert!(
        resources.iter().any(|name| name.contains(&format!("{path}.qvt"))),
        "{resources:?}"
    );
    assert!(
        resources.iter().any(|name| name.contains(&format!("{path}/actions?"))),
        "{resources:?}"
    );
    assert!(
        resources
            .iter()
            .any(|name| name.contains(&format!("{path}/actions/ArtifactAuthzList"))),
        "{resources:?}"
    );
    assert_clean(browser, &page);
}

/// A server-static list: its pages and sort fetch `/actions/EnumList` and redraw.
fn static_list(browser: &mut Browser, base: &str) {
    let page = format!("{base}/qapps/GateStaticList");
    browser.navigate(&page);
    browser.wait_for("the list", "/1-20 \\/ \\d+/.test(document.body.innerText)", WAIT);
    assert_eq!(
        browser.eval("typeof (moqui.qvtStatic || {})['/apps/GateStaticList']"),
        "function"
    );
    // every component runs its precompiled render function: none is left with a string template for the browser to compile
    assert_eq!(
        browser.eval("Object.keys(moqui.qvt.components).filter((name) => typeof moqui.qvt.components[name].template === 'string').join(',')"),
        ""
    );
    let text = browser.eval("document.body.innerText");
    let text = text.as_str().unwrap();
    assert!(text.contains("Enumerations of john.doe"), "{text}");
    for literal in ["{{", "${", "ec."] {
        assert!(!text.contains(literal), "{literal} in {text}");
    }
    browser.eval(FETCH_SPY);
    // the first cell of a row is its row-selection checkbox; the enum id is the second
    let first_row =
        "document.querySelectorAll('#EnumList_table .tbody .tr')[0].querySelectorAll('.td')[1].innerText.trim()";
    let first = browser.eval(first_row);
    browser.click("page 2", &by_text(".q-pagination button", "2"));
    browser.wait_for("page 2", "/\\b21-40 \\/ \\d+/.test(document.body.innerText)", WAIT);
    let rows = fetched(browser, "the rows of page 2", "/actions/EnumList");
    assert!(rows["url"].as_str().unwrap().contains("pageIndex=1"), "{rows}");
    assert_ne!(browser.eval(first_row), first);
    // descending by the enum id: the first row is the last id
    browser.click(
        "the descending sort of the enum id",
        "document.querySelectorAll('#EnumList_table .thead .form-order-by a')[1]",
    );
    browser.wait_for(
        "sorted, back on the first page",
        "/1-20 \\/ \\d+/.test(document.body.innerText) && window.__gate.fetches.some((f) => f.url.indexOf('orderByField=-enumId') >= 0)",
        WAIT,
    );
    let ids = browser.eval("Array.from(document.querySelectorAll('#EnumList_table .tbody .tr')).map((tr) => tr.querySelectorAll('.td')[1].innerText.trim())");
    let ids: Vec<String> = ids
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_string())
        .collect();
    // the database collation orders, so the check is the ends of the page, not a byte order
    assert_eq!(ids.len(), 20, "{ids:?}");
    assert!(ids[0].to_lowercase() > ids[19].to_lowercase(), "{ids:?}");
    assert!(
        ids[0].to_lowercase() > first.as_str().unwrap().to_lowercase(),
        "{ids:?}"
    );

    // the csv export, an item of the Export menu, goes to the screen in csv with the current sort
    browser.click(
        "the Export menu",
        "Array.from(document.querySelectorAll('.q-btn-dropdown')).find((el) => el.innerText.toLowerCase().indexOf('export') >= 0)",
    );
    let csv = browser.wait_for(
        "the csv export item",
        "(Array.from(document.querySelectorAll('.q-menu a')).find((a) => a.href.indexOf('renderMode=csv') >= 0) || {}).href",
        WAIT,
    );
    let csv = csv.as_str().unwrap();
    assert!(
        csv.contains("/apps/GateStaticList?") && csv.contains("orderByField=-enumId"),
        "{csv}"
    );
    browser.press("Escape");
    browser.wait_for(
        "the menu closed",
        "document.querySelectorAll('.q-menu').length === 0",
        WAIT,
    );

    // the find dialog: the header field and the order by
    browser.click("Find Options", &by_text(".q-btn", "Find Options"));
    browser.wait_for(
        "the find dialog",
        "Array.from(document.querySelectorAll('.q-dialog')).some((el) => el.innerText.indexOf('Clear Parameters') >= 0 && el.innerText.indexOf('Order By') >= 0)",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );

    // the saved finds dialog: a new saved find of the current sort
    browser.click("Saved Finds", &by_text(".q-btn", "Saved Finds"));
    browser.wait_for(
        "the saved finds dialog",
        "Array.from(document.querySelectorAll('.q-dialog')).some((el) => el.innerText.indexOf('Save New Find') >= 0)",
        WAIT,
    );
    browser.click(
        "the find description",
        "document.querySelector('#EnumList_NewFind_description')",
    );
    browser.type_text("Gate find");
    browser.click("Save New Find", &by_text(".q-dialog .q-btn", "Save New Find"));
    fetched(browser, "the saved find post", "/formSaveFind");
    browser.wait_for(
        "the saved find in the next form request",
        "window.__gate.fetches.some((f) => f.url.indexOf('/actions/EnumList') >= 0 && f.url.indexOf('formListFindId=') >= 0)",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );
    browser.click("Saved Finds", &by_text(".q-btn", "Saved Finds"));
    browser.wait_for(
        "the saved find listed",
        "Array.from(document.querySelectorAll('.q-dialog')).some((el) => el.innerText.indexOf('Gate find') >= 0)",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );

    // row selection: check the first row, open the row actions, and the action posts the selected id as enumId_0
    let picked = browser
        .eval("document.querySelectorAll('#EnumList_table .tbody .tr')[0].querySelectorAll('.td')[1].innerText.trim()");
    browser.click(
        "the first row's checkbox",
        "document.querySelector('#EnumList_table .tbody .q-checkbox')",
    );
    browser.click(
        "the row actions",
        "document.querySelector('#EnumList_table .thead .q-btn')",
    );
    browser.click("Pick", &by_text(".q-menu .q-btn", "Pick"));
    browser.click("Pick Them", &by_text(".q-dialog .q-btn", "Pick Them"));
    let post = fetched(browser, "the row action post", "/pickEnums");
    let sent: Vec<String> = post["sent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect();
    assert!(
        sent.contains(&format!("enumId_0={}", picked.as_str().unwrap())) && sent.contains(&"_isMulti=true".to_string()),
        "{sent:?}"
    );
    browser.press("Escape");
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for(
        "the menu closed",
        "document.querySelectorAll('.q-menu').length === 0",
        WAIT,
    );

    // the column picker: the dialog lists the columns, and Reset to Default posts and reloads the list
    browser.click("Columns", &by_text(".q-btn", "Columns"));
    browser.wait_for(
        "the column dialog",
        "Array.from(document.querySelectorAll('.q-dialog')).some((el) => el.innerText.indexOf('Do Not Display') >= 0 && el.innerText.indexOf('Enum ID') >= 0)",
        WAIT,
    );
    let before = browser.eval("window.__gate.fetches.filter((f) => f.url.indexOf('/actions/EnumList') >= 0).length");
    browser.click("Reset to Default", &by_text(".q-dialog .q-btn", "Reset to Default"));
    fetched(browser, "the column reset post", "/formSelectColumns");
    browser.wait_for(
        "the list fetched again",
        &format!("window.__gate.fetches.filter((f) => f.url.indexOf('/actions/EnumList') >= 0).length > {before}"),
        WAIT,
    );
    browser.press("Escape");
    browser.wait_for(
        "the dialog closed",
        "document.querySelectorAll('.q-dialog').length === 0",
        WAIT,
    );
    assert_clean(browser, &page);
}

/// The fixture as `html`: the client is bound, the dialogs open and close, and the form posts through its transition.
fn html_fixture(browser: &mut Browser, base: &str) {
    // under /apps, so the page has the shell: the navbar the screen's menus move into
    let page = format!("{base}/apps{FIXTURE}.html?artifactGroupId=SYSTEM_APP");
    browser.navigate(&page);
    browser.wait_for(
        "moqui-html.js bound the page",
        "!!window.moqui && document.documentElement.classList.contains('js')",
        WAIT,
    );
    assert_eq!(
        browser.eval("document.querySelectorAll('script[src*=\"moqui-html.js\"]').length"),
        1
    );
    assert_eq!(
        browser.eval("document.getElementById('ArtifactGroupForm_description').value"),
        "System App (via root screen)"
    );
    let text = browser.eval("document.body.innerText");
    let text = text.as_str().unwrap();
    for wanted in [
        "SYSTEM_APP_ADMIN",
        "Always Allow (overrides Deny)",
        "Administrators (full access) -",
        "component://tools/screen/System.xml",
    ] {
        assert!(text.contains(wanted), "{wanted} in {text}");
    }
    assert_eq!(browser.eval("document.querySelectorAll('table.form-list').length"), 3);
    // menus moved into the navbar
    browser.wait_for(
        "menus in the navbar",
        "document.querySelectorAll('#header-menus > li').length > 0",
        WAIT,
    );
    // a native dialog opens from its button and closes from its close button
    browser.click(
        "the add authorization button",
        "document.querySelector('[data-dialog=\"AddAuthzDialog\"]')",
    );
    browser.wait_for(
        "the dialog is open",
        "document.getElementById('AddAuthzDialog').open",
        WAIT,
    );
    browser.click(
        "the close button",
        "document.querySelector('#AddAuthzDialog [data-close]')",
    );
    browser.wait_for(
        "the dialog is closed",
        "!document.getElementById('AddAuthzDialog').open",
        WAIT,
    );
    assert_clean(browser, &page);

    // the form posts with the token of the page, the transition redirects back, and the page shows the new value
    browser.eval(
        "(() => { document.getElementById('ArtifactGroupForm_description').value = 'Posted from Chromium'; \
         document.getElementById('ArtifactGroupForm').requestSubmit(); return true; })()",
    );
    browser.wait_event("Page.loadEventFired", WAIT);
    browser.wait_for(
        "the page after the post shows the new value",
        "(document.getElementById('ArtifactGroupForm_description') || {}).value === 'Posted from Chromium'",
        WAIT,
    );
    assert_eq!(
        browser.eval("location.pathname + location.search"),
        format!("/apps{FIXTURE}?artifactGroupId=SYSTEM_APP")
    );
    assert_clean(browser, "the fixture after the post");
}

/// Click to edit, a background submit, the column picker with SortableJS loaded on use, and the pages of a form-list, in `html`.
fn html_widgets(browser: &mut Browser, base: &str) {
    let page = format!("{base}/apps/GateWidgets");
    browser.navigate(&page);
    browser.wait_for("the client", "!!window.moqui", WAIT);
    browser.eval(FETCH_SPY);

    // click to edit
    browser.click("the editable text", "document.getElementById('GateEditable')");
    browser.wait_for(
        "the editor",
        "!!document.querySelector('#GateEditable form input')",
        WAIT,
    );
    browser.eval(
        "(() => { const form = document.querySelector('#GateEditable form'); form.querySelector('input').value = 'Edited in Chromium'; \
         form.requestSubmit(); return true; })()",
    );
    let saved = fetched(browser, "the editable post", "/saveValue");
    assert_eq!(saved["status"], 200, "{saved}");
    let body: serde_json::Value = serde_json::from_str(saved["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["saved"], "Edited in Chromium", "{saved}");
    browser.wait_for(
        "the new text",
        "document.getElementById('GateEditable').textContent === 'Edited in Chromium'",
        WAIT,
    );

    // a background submit with a date, a drop-down, a multiple drop-down that Tom Select (loaded on use) runs, and the success message
    browser.wait_for(
        "Tom Select on the multiple drop-down",
        "!!(window.TomSelect && document.querySelector('#NoteForm select[name=\"tags\"]').tomselect)",
        WAIT,
    );
    browser.eval("document.querySelector('#NoteForm select[name=\"tags\"]').tomselect.setValue(['X', 'Y'])");
    browser.eval(
        "(() => { const form = document.getElementById('NoteForm'); form.note.value = 'hello'; form.when.value = '2026-10-13'; \
         const kind = form.kind; if (kind.tomselect) kind.tomselect.setValue('B'); else kind.value = 'B'; return true; })()",
    );
    browser.click("the send button", "document.querySelector('#NoteForm [type=submit]')");
    let sent = fetched(browser, "the background submit", "/saveNote");
    assert_eq!(sent["status"], 200, "{sent}");
    let body: serde_json::Value = serde_json::from_str(sent["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["note"], "hello", "{sent}");
    assert_eq!(body["when"], "2026-10-13", "{sent}");
    assert_eq!(body["kind"], "B", "{sent}");
    assert_eq!(body["tags"], serde_json::json!(["X", "Y"]), "{sent}");
    browser.wait_for(
        "the message of the form",
        "Array.from(document.querySelectorAll('#toasts .alert')).some((el) => el.textContent.indexOf('Note sent') >= 0)",
        WAIT,
    );
    assert_eq!(
        browser.eval("location.pathname"),
        "/apps/GateWidgets",
        "a background submit stays on the page"
    );

    // a validation error from the transition shows by its field, with the markup of the form macros
    browser.eval("(() => { document.getElementById('CodeForm').code.value = 'abc'; return true; })()");
    field_error_by_its_field(
        browser,
        "document.querySelector('#CodeForm [type=submit]')",
        "(() => { const box = document.querySelector('#CodeForm [name=\"code\"]').closest('.field'); return box.hasAttribute('data-invalid') \
         && box.querySelector('.field-error').textContent === 'Code needs a digit'; })()",
        "Array.from(document.querySelectorAll('#toasts .alert')).some((el) => el.textContent.indexOf('Code needs a digit') >= 0)",
    );

    // the column picker: SortableJS is loaded when the dialog's list is bound, and the tree is written to the hidden input
    browser.wait_for("SortableJS loaded on use", "!!window.Sortable", WAIT);
    browser.click(
        "the columns button",
        "document.querySelector('[data-dialog=\"EnumList_SelColsDialog\"]')",
    );
    browser.wait_for(
        "the columns dialog",
        "document.getElementById('EnumList_SelColsDialog').open",
        WAIT,
    );
    let tree = browser.wait_for(
        "the columns tree",
        "document.getElementById('EnumList_SelColsForm_columnsTree').value",
        WAIT,
    );
    let tree: serde_json::Value = serde_json::from_str(tree.as_str().unwrap()).unwrap();
    assert_eq!(tree[0]["id"], "hidden", "{tree}");
    assert!(
        tree.as_array().unwrap().iter().any(|column| column["children"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field["id"] == "enumId"))),
        "{tree}"
    );
    browser.click(
        "the close button",
        "document.querySelector('#EnumList_SelColsDialog [data-close]')",
    );
    assert_clean(browser, &page);

    // the pages of the list
    let first = browser.eval("document.querySelector('#EnumList_table tbody tr td').innerText.trim()");
    browser.click("page 2", &by_text("ul.pagination a", "2"));
    browser.wait_event("Page.loadEventFired", WAIT);
    browser.wait_for(
        "page 2",
        "/Page 2 of \\d+ \\(21 - 40 of \\d+\\)/.test(document.body.innerText)",
        WAIT,
    );
    assert_ne!(
        browser.eval("document.querySelector('#EnumList_table tbody tr td').innerText.trim()"),
        first
    );
    // sort by the description, descending
    browser.click(
        "the sort of the description",
        "document.querySelector('#EnumList_table thead a[href*=\"orderByField=-description\"]')",
    );
    browser.wait_event("Page.loadEventFired", WAIT);
    browser.wait_for(
        "sorted",
        "location.search.indexOf('orderByField=-description') >= 0",
        WAIT,
    );
    let descriptions = browser.eval(
        "Array.from(document.querySelectorAll('#EnumList_table tbody tr')).map((tr) => tr.children[1].innerText.trim())",
    );
    let descriptions: Vec<String> = descriptions
        .as_array()
        .unwrap()
        .iter()
        .map(|text| text.as_str().unwrap().to_string())
        .collect();
    // the database collation orders, so the check is the ends of the page, not a byte order
    assert_eq!(descriptions.len(), 20);
    assert!(
        descriptions[0].to_lowercase() > descriptions[19].to_lowercase(),
        "{descriptions:?}"
    );
    assert_clean(browser, "the widgets after paging and sorting");
}

/// `html-print` has no client, and `text`, `csv` and `xml` come back to a page's fetch with their types.
fn print_and_text_modes(browser: &mut Browser, base: &str) {
    let page = format!("{base}{FIXTURE}.html-print?artifactGroupId=SYSTEM_APP");
    browser.navigate(&page);
    assert_eq!(browser.eval("typeof window.moqui"), "undefined");
    assert_eq!(browser.eval("document.scripts.length"), 0);
    assert_eq!(browser.eval("document.querySelectorAll('table.form-list').length"), 3);
    let text = browser.eval("document.body.innerText");
    assert!(
        text.as_str().unwrap().contains("Always Allow (overrides Deny)"),
        "{text}"
    );
    assert_clean(browser, &page);

    for (mode, content_type, wanted) in [
        ("text", "text/plain;charset=UTF-8", "Artifact Group ID"),
        (
            "csv",
            "text/csv;charset=UTF-8",
            "Artifact Type,Artifact Name,Name Is Pattern",
        ),
        (
            "xml",
            "text/xml;charset=UTF-8",
            "<ArtifactAuthzListEntry artifactAuthzId=\"SYSTEM_APP_ADMIN\"",
        ),
    ] {
        let answer = browser.eval(&format!(
            "fetch('{FIXTURE}.{mode}?artifactGroupId=SYSTEM_APP').then((r) => r.text().then((body) => ({{status: r.status, type: r.headers.get('content-type'), body}})))"
        ));
        assert_eq!(answer["status"], 200, "{mode}: {answer}");
        assert_eq!(answer["type"], content_type, "{mode}");
        assert!(answer["body"].as_str().unwrap().contains(wanted), "{mode}: {answer}");
    }
    let parsed = browser.eval(&format!(
        "fetch('{FIXTURE}.xml?artifactGroupId=SYSTEM_APP').then((r) => r.text()).then((t) => {{ const d = new DOMParser().parseFromString(t, 'text/xml'); \
         return d.querySelector('parsererror') ? 'error' : d.querySelectorAll('ArtifactAuthzListEntry').length; }})"
    ));
    assert_eq!(parsed, 2, "the xml mode parses as XML");
    assert_clean(browser, "the text modes");
}
