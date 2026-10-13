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
//! form-list. The run also executes the server-static compatibility report.
//!
//! The browser is `pdf-chromium-path`, else `chromium`, `chromium-browser` or `google-chrome` on the `PATH`. A missing binary fails.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

mod common;

use std::time::Duration;

use common::cdp::{Browser, Problem};
use common::{start, start_with, FIXTURE};

const WAIT: Duration = Duration::from_secs(30);

/// Records every `fetch` of the page with its answer, as `window.__gate.fetches`.
const FETCH_SPY: &str = r#"(() => {
    if (window.__gate) return true;
    window.__gate = { fetches: [] };
    const original = window.fetch.bind(window);
    window.fetch = (input, init) => original(input, init).then((response) => {
        const url = String((input && input.url) || input);
        const method = (init && init.method) || (input && input.method) || "GET";
        response.clone().text().then((body) => window.__gate.fetches.push({ url, method, status: response.status, body }));
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
    // the same run executes the compatibility report, and the fixture is clean in it
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reports = moqui_host::qvt_report::qvt_static_reports(&repo.join("runtime"), &repo.join("framework")).unwrap();
    let fixture = reports
        .iter()
        .find(|(location, _)| location.ends_with("ArtifactGroup/ArtifactGroupDetail.xml"))
        .and_then(|(_, report)| report.as_ref().ok())
        .expect("a report for the fixture");
    assert!(fixture.is_clean(), "{}", fixture.render());

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
    let text = browser.eval("document.body.innerText");
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

    // the column dialog
    browser.click("the columns button", &by_text("button", "table_chart\nColumns"));
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
    browser.wait_for(
        "page 2",
        "document.body.innerText.indexOf('2 / 10 (21-40 / 186)') >= 0",
        WAIT,
    );
    browser.pump(Duration::from_millis(500));
    assert_clean(browser, &page);
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
    browser.wait_for(
        "the list",
        "document.body.innerText.indexOf('1 / 10 (1-20 / 186)') >= 0",
        WAIT,
    );
    assert_eq!(
        browser.eval("typeof (moqui.qvtStatic || {})['/apps/GateStaticList']"),
        "function"
    );
    let text = browser.eval("document.body.innerText");
    let text = text.as_str().unwrap();
    assert!(text.contains("Enumerations of john.doe"), "{text}");
    for literal in ["{{", "${", "ec."] {
        assert!(!text.contains(literal), "{literal} in {text}");
    }
    browser.eval(FETCH_SPY);
    let first_row = "document.querySelector('#EnumList_table .tbody .tr .td').innerText.trim()";
    let first = browser.eval(first_row);
    browser.click("page 2", &by_text(".q-pagination button", "2"));
    browser.wait_for(
        "page 2",
        "document.body.innerText.indexOf('2 / 10 (21-40 / 186)') >= 0",
        WAIT,
    );
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
        "document.body.innerText.indexOf('1 / 10 (1-20 / 186)') >= 0 && window.__gate.fetches.some((f) => f.url.indexOf('orderByField=-enumId') >= 0)",
        WAIT,
    );
    let ids = browser.eval("Array.from(document.querySelectorAll('#EnumList_table .tbody .tr')).map((tr) => tr.querySelector('.td').innerText.trim())");
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
        "document.body.innerText.indexOf('Page 2 of 10 (21 - 40 of 186)') >= 0",
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
