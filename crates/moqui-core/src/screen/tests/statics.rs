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

//! Server-static qvt: the template holds no request data, compiles to a render function with no `with`, is cached and validated, and the
//! `actions` transition answers the data of the screen and of each form-list. The fixtures are under `testdata/Static`.

use std::collections::BTreeMap;

use moqui_lib::object::Value;

use crate::context::ExecutionContextFactory;
use crate::screen::{make_render, RenderOutput, ScreenFacade};

fn root() -> String {
    format!("{}/src/screen/testdata/Static/Static.xml", env!("CARGO_MANIFEST_DIR"))
}

fn render(factory: &std::sync::Arc<ExecutionContextFactory>, path: &str, parameters: &[(&str, &str)]) -> RenderOutput {
    let map: BTreeMap<String, Value> = parameters
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::string(*value)))
        .collect();
    make_render(factory)
        .root_screen(root())
        .screen_path(path)
        .parameters(map)
        .render()
        .unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn html_of(factory: &std::sync::Arc<ExecutionContextFactory>, screen: &str, key: &str) -> String {
    let location = format!(
        "{}/src/screen/testdata/Static/Static/{screen}.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    let def = ScreenFacade::of(factory).screen_def(factory, &location).unwrap();
    def.generated()
        .static_template(key)
        .expect("the template was cached")
        .html
        .clone()
}

/// Compare `actual` with the checked-in file under `testdata/Static/golden`. `UPDATE_GOLDEN=1` rewrites it. The Node tests of the client read the same files.
fn golden(name: &str, actual: &str) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/screen/testdata/Static/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {}", path.display()));
    assert_eq!(actual, expected, "{name} differs from {}", path.display());
}

fn header<'a>(output: &'a RenderOutput, name: &str) -> &'a str {
    output
        .header(name)
        .unwrap_or_else(|| panic!("no {name} in {:?}", output.headers))
}

#[test]
fn the_template_holds_no_request_data_and_compiles_without_with() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/Items.qvt", &[("password", "ZEBRA-secret")]);
    assert_eq!(output.status, 200, "{}", output.body_text());
    assert_eq!(header(&output, "X-Moqui-Qvt"), "static-template");
    assert_eq!(
        header(&output, "Cache-Control"),
        crate::screen::statics::SERVER_STATIC_CACHE_CONTROL
    );
    assert_eq!(header(&output, "Content-Type"), "text/javascript;charset=UTF-8");
    assert!(header(&output, "ETag").starts_with("W/\""), "{:?}", output.headers);
    let script = output.body_text();
    assert!(
        script.contains("(m.qvtStatic=m.qvtStatic||{})[\"/Items\"]=function(Vue){"),
        "{script}"
    );
    assert!(
        script.contains("_resolveComponent(\"m-form\")") && script.contains("m_static_form_list"),
        "{script}"
    );
    // the compiler wrote a render function over _ctx: no with, no new Function, no eval
    for banned in ["with (", "with(", "new Function", "eval("] {
        assert!(!script.contains(banned), "{banned} in {script}");
    }
    // nothing the actions made, and nothing of the request, is in the template or in the script
    let html = html_of(context.factory(), "Items", "/Items");
    for text in [&script, &html] {
        for data in ["ZEBRA", "ada.lovelace", "always-ran", "1234"] {
            assert!(!text.contains(data), "{data} in {text}");
        }
    }
    golden("Items.script.js", &script);
    golden("Items.template.html", &html);
    assert!(html.contains("<m-static-form-list name=\"ItemList\""), "{html}");
    assert!(html.contains("v-for=\"(tag, tag_index) in (tags)\""), "{html}");
    assert!(html.contains("<template v-if=\"(item != null)\">"), "{html}");
}

#[test]
fn every_request_gets_the_same_cached_template() {
    let context = ExecutionContextFactory::test_context();
    let first = render(context.factory(), "/Items.qvt", &[("itemId", "one")]);
    let second = render(
        context.factory(),
        "/Items.qvt?x=2",
        &[("itemId", "two"), ("other", "y")],
    );
    assert_eq!(first.body_text(), second.body_text());
    assert_eq!(first.header("ETag"), second.header("ETag"));
}

#[test]
fn a_screen_not_marked_for_qvt_stays_a_server_merge() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/Plain.qvt", &[]);
    assert_eq!(header(&output, "X-Moqui-Qvt"), "merged-fragment");
    assert_eq!(header(&output, "Cache-Control"), "private, no-store");
    assert!(
        output.body_text().contains("PLAIN-MERGED-VALUE"),
        "{}",
        output.body_text()
    );
    // marked, but dynamic: no template and no static cache header, with the reason
    let output = render(context.factory(), "/Dynamic.qvt", &[]);
    assert_eq!(header(&output, "X-Moqui-Qvt"), "merged-fragment");
    assert_eq!(header(&output, "Cache-Control"), "private, no-store");
    let location = format!(
        "{}/src/screen/testdata/Static/Static/Dynamic.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    let facade = ScreenFacade::of(context.factory());
    let def = facade.screen_def(context.factory(), &location).unwrap();
    let eligibility = crate::screen::statics::eligibility_of(
        &facade,
        context.factory(),
        &def,
        "framework/template/screen/ScreenQvt.js",
        false,
    );
    assert!(!eligibility.eligible && eligibility.marked, "{eligibility:?}");
    assert!(
        eligibility.reasons.iter().any(|reason| reason.contains("dynamic")),
        "{eligibility:?}"
    );
    assert!(
        eligibility
            .reasons
            .iter()
            .any(|reason| reason.contains("extends an expression")),
        "{eligibility:?}"
    );
}

fn json_of(output: &RenderOutput) -> serde_json::Value {
    serde_json::from_slice(&output.body).unwrap_or_else(|err| panic!("{err}: {}", output.body_text()))
}

#[test]
fn actions_answers_the_package_without_the_lists_and_with_plain_values() {
    let context = ExecutionContextFactory::test_context();
    let output = render(
        context.factory(),
        "/Items/actions",
        &[("password", "ZEBRA-secret"), ("keep", "yes")],
    );
    assert_eq!(output.status, 200, "{}", output.body_text());
    assert_eq!(header(&output, "Cache-Control"), "private, no-store");
    assert_eq!(header(&output, "Content-Type"), "application/json");
    let package = json_of(&output);
    assert_eq!(package["greeting"], "ZEBRA greeting for Ada Lovelace");
    assert_eq!(package["item"]["itemId"], "ZEBRA-I1");
    assert_eq!(package["tags"][1], "ZEBRA-t2");
    // always-actions ran, a decimal is a string, the request's own parameters are there, and a credential is not
    assert_eq!(package["alwaysTag"], "always-ran");
    assert_eq!(package["amount"], "1234.50");
    assert_eq!(package["keep"], "yes");
    assert!(package.get("password").is_none(), "{package}");
    assert!(package["item"].get("currentPassword").is_none(), "{package}");
    // the list of the form-list is a request of its own
    assert!(package.get("itemList").is_none(), "{package}");
    golden("Items.package.json", &String::from_utf8(output.body.clone()).unwrap());
    // the context holds nothing the browser cannot use: no ec, sri, finds
    for dropped in ["ec", "sri", "context"] {
        assert!(package.get(dropped).is_none(), "{dropped} in {package}");
    }
}

#[test]
fn a_form_request_answers_its_rows_and_page_keys() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/Items/actions/ItemList", &[]);
    assert_eq!(output.status, 200, "{}", output.body_text());
    assert_eq!(header(&output, "Cache-Control"), "private, no-store");
    let body = json_of(&output);
    let rows = body["itemList"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{body}");
    assert_eq!(rows[0]["itemId"], "ZEBRA-R1");
    assert_eq!(rows[1]["name"], "ZEBRA row two");
    // the body is the context-shaped map, not a bare array
    assert!(body.is_object());
    golden("Items.ItemList.json", &output.body_text());
    // the multi form-list of the same list answers the same rows
    let multi = render(context.factory(), "/Items/actions/ItemMulti", &[]);
    assert_eq!(multi.status, 200, "{}", multi.body_text());
    assert_eq!(json_of(&multi)["itemList"].as_array().map(Vec::len), Some(2));
    golden("Items.ItemMulti.json", &multi.body_text());
    // a form that is not a list, and a name that is not a form
    let single = render(context.factory(), "/Items/actions/ItemForm", &[]);
    assert_eq!(single.status, 400, "{}", single.body_text());
    let missing = render(context.factory(), "/Items/actions/NoSuch", &[]);
    assert_eq!(missing.status, 404, "{}", missing.body_text());
}

fn render_fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/script/testdata/screen/qvt/render")
}

/// The static template of a fixture screen of the qvt render tests, as the report makes it: the screen is not marked, so it is forced.
fn static_fixture(name: &str) -> (String, crate::screen::statics::StaticReport) {
    let context = ExecutionContextFactory::test_context();
    let guard = crate::screen::statics::ReportGuard::start(true);
    let location = render_fixture_dir()
        .join(format!("{name}.xml"))
        .to_string_lossy()
        .into_owned();
    make_render(context.factory())
        .root_screen(location.clone())
        .screen_path("")
        .render_mode("qvt")
        .render()
        .unwrap_or_else(|err| panic!("{name}: {err}"));
    let report = guard.finish();
    let def = ScreenFacade::of(context.factory())
        .screen_def(context.factory(), &location)
        .unwrap();
    let template = def
        .generated()
        .static_template("")
        .unwrap_or_else(|| panic!("{name} made no template: {report:?}"));
    (template.html.clone(), report)
}

#[test]
fn the_qvt_fixture_screens_become_templates_that_compile() {
    for name in ["containers", "fields", "lists", "extras", "braces"] {
        let (html, report) = static_fixture(name);
        golden(&format!("render/{name}.html"), &html);
        eprintln!("{}", report.render());
    }
}

#[test]
fn entity_screens_look_up_display_options_and_sets_on_the_server_and_page_by_the_request() {
    use crate::entity::EntityFacade;
    let world = super::World::open("");
    world.enumeration_type();
    let make = |facade: &EntityFacade, id: &str, sequence: i64| {
        let mut row = facade.make_value("moqui.basic.Enumeration").unwrap();
        row.set("enumId", Value::string(id));
        row.set("enumTypeId", Value::string("TestType"));
        row.set("description", Value::string(format!("Desc {id}")));
        row.set("sequenceNum", Value::I64(sequence));
        row.create(facade).unwrap();
    };
    for (index, id) in ["E1", "E2", "E3", "E4", "E5"].iter().enumerate() {
        make(&world.facade, id, i64::try_from(index).unwrap());
    }
    // the screen package: the form-single's extras hold the looked-up text, the option list and the field the set made; the list and its page keys are not in it
    let screen = render(&world.factory, "/Enums/actions", &[("enumTypeId", "TestType")]);
    assert_eq!(screen.status, 200, "{}", screen.body_text());
    let package = json_of(&screen);
    let extras = &package["mqExtra_TypeForm"];
    assert_eq!(extras["typeText_display"], "Type: Test", "{package}");
    assert_eq!(extras["shown"], "TestType-set", "{package}");
    let options = extras["mqOpt_TypeForm_pick_0_0"]
        .as_array()
        .unwrap_or_else(|| panic!("{package}"));
    assert_eq!(options[0]["value"], "TestType");
    assert_eq!(options[0]["label"], "Test");
    assert!(
        package.get("enumList").is_none() && package.get("enumListCount").is_none(),
        "{package}"
    );
    // the form request: rows with their lookups, and the page keys the find set, which the request's page and size choose
    let page = render(
        &world.factory,
        "/Enums/actions/EnumList",
        &[("enumTypeId", "TestType"), ("pageSize", "2"), ("pageIndex", "1")],
    );
    assert_eq!(page.status, 200, "{}", page.body_text());
    let body = json_of(&page);
    let rows = body["enumList"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{body}");
    assert_eq!(rows[0]["enumId"], "E3");
    assert_eq!(rows[0]["upper"], "DESC E3");
    assert_eq!(rows[0]["enumTypeId_display"], "Test");
    assert_eq!(body["enumListCount"], 5, "{body}");
    assert_eq!(body["enumListPageIndex"], 1);
    assert_eq!(body["enumListPageSize"], 2);
    assert_eq!(body["enumListPageMaxIndex"], 2);
    assert_eq!(body["enumListPageRangeLow"], 3);
    assert_eq!(body["enumListPageRangeHigh"], 4);
    assert_eq!(header(&page, "X-Total-Count"), "5");
    assert_eq!(header(&page, "X-Page-Max-Index"), "2");
    // the template reads the page keys of the list from the form scope
    let template = render(&world.factory, "/Enums.qvt", &[]);
    assert_eq!(header(&template, "X-Moqui-Qvt"), "static-template");
    assert!(
        template.body_text().contains("_ctx.enumListCount"),
        "{}",
        template.body_text()
    );
}

fn report(
    factory: &std::sync::Arc<ExecutionContextFactory>,
    screen: &str,
    full: bool,
) -> crate::screen::statics::StaticReport {
    crate::screen::statics::static_report(factory, &root(), &format!("/{screen}"), &BTreeMap::new(), full).unwrap()
}

#[test]
fn a_screen_with_nothing_for_the_browser_to_refuse_has_a_clean_report() {
    let context = ExecutionContextFactory::test_context();
    let report = report(context.factory(), "Clean", false);
    assert!(report.is_clean(), "{}", report.render());
    assert!(
        report.marked && report.eligible && report.template_bytes > 0,
        "{}",
        report.render()
    );
    assert!(report.render().contains("verdict clean"), "{}", report.render());
}

#[test]
fn the_report_names_what_a_browser_cannot_do_and_what_the_form_request_repeats() {
    let world = super::World::open("");
    world.enumeration_type();
    let report = report(&world.factory, "Report", true);
    let text = report.render();
    assert!(!report.is_clean() && report.eligible && report.full_check, "{text}");
    let has = |kind: &str, needle: &str| {
        report.violations.iter().any(|violation| {
            violation.kind == kind && format!("{} {}", violation.element, violation.detail).contains(needle)
        })
    };
    // the static walk of the expressions the macros wrote
    assert!(has("ec", "ec.entity.find"), "{text}");
    assert!(has("decimal", "amount.add"), "{text}");
    assert!(has("sri", "sri.getScreenUrlInstance"), "{text}");
    // the full check: the same widget expression ran with the wrapped ec and was recorded, and the render went on
    assert!(has("wrapped-ec", "ec.entity"), "{text}");
    assert!(!has("full-check", ""), "the render itself did not fail: {text}");
    // always-actions and pre-actions with side effects, which the form request repeats
    assert!(has("side-effect", "always-actions/message"), "{text}");
    assert!(has("side-effect", "pre-actions/entity-make-value"), "{text}");
    // the list of a form-list in a section-iterate and a label that reads its page keys
    assert!(has("form-list-key", "iterates enumList"), "{text}");
    assert!(has("form-list-key", "enumListCount"), "{text}");
    // a content screen marked for html is a stale-cache bug
    assert!(has("stale-cache", "server-static names html"), "{text}");
    // the report is one block: location, mode, verdict, and each violation with its screen and element
    assert!(text.starts_with("screen "), "{text}");
    assert!(
        text.contains("  mode qvt\n") && text.contains("verdict violations"),
        "{text}"
    );
    assert!(text.contains("violation [ec] Report.xml label@text"), "{text}");
    let kinds = report.kinds();
    for kind in [
        "decimal",
        "ec",
        "form-list-key",
        "side-effect",
        "sri",
        "stale-cache",
        "wrapped-ec",
    ] {
        assert!(kinds.iter().any(|found| found == kind), "{kind} in {kinds:?}");
    }
}

#[test]
fn a_screen_that_stays_a_server_merge_says_why_and_has_no_template() {
    let context = ExecutionContextFactory::test_context();
    let report = report(context.factory(), "Dynamic", false);
    assert!(
        !report.eligible && report.marked && report.template_bytes == 0,
        "{}",
        report.render()
    );
    assert!(
        report
            .reasons
            .iter()
            .any(|reason| reason.contains("extends an expression")),
        "{}",
        report.render()
    );
    assert!(
        report.render().contains("verdict stays a server merge"),
        "{}",
        report.render()
    );
}

fn eligibility(factory: &std::sync::Arc<ExecutionContextFactory>, screen: &str) -> crate::screen::statics::Eligibility {
    let location = format!(
        "{}/src/screen/testdata/Static/Static/{screen}.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    let facade = ScreenFacade::of(factory);
    let def = facade.screen_def(factory, &location).unwrap();
    (*crate::screen::statics::eligibility_of(&facade, factory, &def, "framework/template/screen/ScreenQvt.js", false))
        .clone()
}

#[test]
fn server_static_names_qvt_through_a_list_all_the_automatic_case_and_the_forms() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    // the list "qvt,html" and "all" mark it, and the subscreens-only screen is marked by the schema's own rule
    assert!(eligibility(factory, "Items").eligible);
    assert!(eligibility(factory, "Report").marked);
    assert!(eligibility(factory, "All").eligible);
    assert!(eligibility(factory, "Auto").eligible);
    // not named: not marked, and the reason is that
    let html_only = eligibility(factory, "HtmlOnly");
    assert!(!html_only.marked && !html_only.eligible, "{html_only:?}");
    assert!(html_only.reasons[0].contains("does not name qvt"), "{html_only:?}");
    // a form that says it is not static for qvt keeps a marked screen a server merge, and a form that says qvt does not mark an unmarked screen
    let out = eligibility(factory, "FormOut");
    assert!(
        out.marked && !out.eligible && out.reasons.iter().any(|reason| reason.contains("leaves out qvt")),
        "{out:?}"
    );
    assert!(!eligibility(factory, "FormIn").eligible);
    // an extra path is lost by the data request
    let extra = eligibility(factory, "ExtraPath");
    assert!(
        !extra.eligible && extra.reasons.iter().any(|reason| reason.contains("allow-extra-path")),
        "{extra:?}"
    );
}

#[test]
fn qvt_of_a_marked_screen_is_a_template_and_of_an_unmarked_one_a_merge_with_the_header_to_say_so() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    for (screen, static_template) in [
        ("All", true),
        ("Auto", true),
        ("HtmlOnly", false),
        ("FormOut", false),
        ("ExtraPath", false),
    ] {
        let output = render(factory, &format!("/{screen}.qvt"), &[]);
        assert_eq!(output.status, 200, "{screen}: {}", output.body_text());
        let expected = if static_template {
            "static-template"
        } else {
            "merged-fragment"
        };
        assert_eq!(header(&output, "X-Moqui-Qvt"), expected, "{screen}");
        let cache = header(&output, "Cache-Control");
        if static_template {
            assert_eq!(cache, crate::screen::statics::SERVER_STATIC_CACHE_CONTROL, "{screen}");
        } else {
            assert_eq!(cache, "private, no-store", "{screen}");
        }
    }
}

#[test]
fn a_mode_other_than_qvt_marked_static_sends_the_cache_header_and_the_automatic_case_does_not() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    // explicit: html gets the header; the automatic subscreens case does not, since an html page holds the navbar of the user
    let marked = render(factory, "/HtmlOnly.html", &[]);
    assert_eq!(
        header(&marked, "Cache-Control"),
        crate::screen::statics::SERVER_STATIC_CACHE_CONTROL
    );
    let all = render(factory, "/All.html", &[]);
    assert_eq!(
        header(&all, "Cache-Control"),
        crate::screen::statics::SERVER_STATIC_CACHE_CONTROL
    );
    let auto = render(factory, "/Auto.html", &[]);
    assert!(auto.header("Cache-Control").is_none(), "{:?}", auto.headers);
    // a screen not marked for the mode has none
    let plain = render(factory, "/Plain.html", &[]);
    assert!(plain.header("Cache-Control").is_none(), "{:?}", plain.headers);
}

fn write_screen(dir: &std::path::Path, field: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let root = dir.join("Root.xml");
    std::fs::write(
        &root,
        "<screen require-authentication=\"false\"><widgets><subscreens-active/></widgets><subscreens default-item=\"Edited\"/></screen>",
    )
    .unwrap();
    let screen = dir.join("Root/Edited.xml");
    std::fs::create_dir_all(dir.join("Root")).unwrap();
    std::fs::write(
        &screen,
        format!(
            "<screen require-authentication=\"false\" server-static=\"qvt\"><widgets>\
             <form-single name=\"F\"><field name=\"{field}\"><default-field><text-line/></default-field></field></form-single>\
             </widgets></screen>"
        ),
    )
    .unwrap();
    // a later modification time, so the cached definition is seen as old
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&screen)
        .unwrap()
        .set_modified(later)
        .unwrap();
    root.to_string_lossy().into_owned()
}

#[test]
fn the_cached_template_follows_the_resolved_definition() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    crate::screen::ScreenFacade::of(factory).set_check_interval(Some(std::time::Duration::ZERO));
    let dir = std::env::temp_dir().join(format!("moqui-static-identity-{}", std::process::id()));
    let root = write_screen(&dir, "alpha");
    let first = make_render(factory)
        .root_screen(root.clone())
        .screen_path("/Edited.qvt")
        .render()
        .unwrap();
    assert!(first.body_text().contains("alpha"), "{}", first.body_text());
    let tag = header(&first, "ETag").to_string();
    // the same definition gives the same validator
    let again = make_render(factory)
        .root_screen(root.clone())
        .screen_path("/Edited.qvt")
        .render()
        .unwrap();
    assert_eq!(header(&again, "ETag"), tag);
    // an edit of the screen file makes a new definition, a new template, and a new validator
    let root = write_screen(&dir, "beta");
    let edited = make_render(factory)
        .root_screen(root)
        .screen_path("/Edited.qvt")
        .render()
        .unwrap();
    assert!(
        edited.body_text().contains("beta") && !edited.body_text().contains("alpha"),
        "{}",
        edited.body_text()
    );
    assert_ne!(header(&edited, "ETag"), tag);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_template_has_a_validator_and_answers_304_and_head() {
    use crate::web::WebRequest;
    let world = super::World::open("");
    let get = |method: &str, tag: Option<&str>| {
        let mut request = WebRequest::new(method, "/Items.qvt");
        request.peer_ip = "127.0.0.1".to_string();
        if let Some(tag) = tag {
            request.set_header("if-none-match", tag);
        }
        crate::screen::serve_at(&world.factory, request, &root())
    };
    let first = get("GET", None);
    assert_eq!(first.status, 200, "{}", first.body_text());
    let tag = first.header("ETag").expect("a validator").to_string();
    assert_eq!(first.header("X-Moqui-Qvt"), Some("static-template"));
    // the shared headers of a screen response are there too, and the cache header of the template was not replaced by the private one
    assert_eq!(
        first.header("Cache-Control"),
        Some(crate::screen::statics::SERVER_STATIC_CACHE_CONTROL)
    );
    assert_eq!(first.header("X-Content-Type-Options"), Some("nosniff"));
    let same = get("GET", Some(&tag));
    assert_eq!(same.status, 304);
    assert!(same.body.is_empty());
    assert_eq!(same.header("ETag"), Some(tag.as_str()));
    let other = get("GET", Some("W/\"nope\""));
    assert_eq!(other.status, 200);
    assert_eq!(other.body, first.body);
    let head = get("HEAD", None);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    assert_eq!(head.header("ETag"), Some(tag.as_str()));
    // the data is not cached anywhere
    let mut request = WebRequest::new("GET", "/Items/actions");
    request.peer_ip = "127.0.0.1".to_string();
    let actions = crate::screen::serve_at(&world.factory, request, &root());
    assert_eq!(actions.status, 200, "{}", actions.body_text());
    assert_eq!(actions.header("Cache-Control"), Some("private, no-store"));
    assert!(actions.header("ETag").is_none());
}

#[test]
fn include_screen_and_section_include_are_written_inline_with_their_actions_in_the_package() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    let dir = std::env::temp_dir().join(format!("moqui-static-include-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Root")).unwrap();
    let part = format!(
        "{}/src/screen/testdata/Static/Static/Part.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::write(
        dir.join("Root.xml"),
        "<screen require-authentication=\"false\"><widgets><subscreens-active/></widgets><subscreens default-item=\"Includer\"/></screen>",
    )
    .unwrap();
    std::fs::write(
        dir.join("Root/Includer.xml"),
        format!(
            "<screen require-authentication=\"false\" server-static=\"qvt\"><widgets><label text=\"before\"/>\
             <include-screen location=\"{part}\"/><section-include name=\"Shared\" location=\"{part}\"/></widgets></screen>"
        ),
    )
    .unwrap();
    let root = dir.join("Root.xml").to_string_lossy().into_owned();
    let template = make_render(factory)
        .root_screen(root.clone())
        .screen_path("/Includer.qvt")
        .render()
        .unwrap();
    assert_eq!(
        header(&template, "X-Moqui-Qvt"),
        "static-template",
        "{}",
        template.body_text()
    );
    // the included widgets are in the template, the values are not
    let script = template.body_text();
    assert!(
        !script.contains("PART-ZEBRA-VALUE") && !script.contains("SECTION-ZEBRA"),
        "{script}"
    );
    assert!(
        script.contains("part says ${partValue}") && script.contains("section says ${sectionValue}"),
        "{script}"
    );
    assert!(script.contains("_ctx.partValue"), "{script}");
    // the package has what the actions of the included screen and of its sections made
    let package = make_render(factory)
        .root_screen(root)
        .screen_path("/Includer/actions")
        .render()
        .unwrap();
    let json = json_of(&package);
    assert_eq!(json["partValue"], "PART-ZEBRA-VALUE", "{json}");
    assert_eq!(json["sectionValue"], "SECTION-ZEBRA", "{json}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_template_the_compiler_rejects_leaves_a_clean_merged_render_and_is_not_built_again() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    let dir = std::env::temp_dir().join(format!("moqui-static-broken-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Root")).unwrap();
    std::fs::write(
        dir.join("Root.xml"),
        r#"<screen require-authentication="false"><widgets><subscreens-active/></widgets></screen>"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("Root/Broken.xml"),
        r#"<screen require-authentication="false" server-static="qvt"><widgets>
            <label text="before"/><render-mode><text type="qvt"><![CDATA[<div><span>not closed</div>]]></text></render-mode>
        </widgets></screen>"#,
    )
    .unwrap();
    let root = dir.join("Root.xml").to_string_lossy().into_owned();
    let render = || {
        make_render(factory)
            .root_screen(root.clone())
            .screen_path("/Broken.qvt")
            .render()
            .unwrap()
    };
    let first = render();
    assert_eq!(
        header(&first, "X-Moqui-Qvt"),
        "merged-fragment",
        "{}",
        first.body_text()
    );
    // Only the merged render's text: the template that was written before the compiler refused it is gone.
    assert_eq!(first.body_text().matches("before").count(), 1, "{}", first.body_text());
    let def = ScreenFacade::of(factory)
        .screen_def(factory, &dir.join("Root/Broken.xml").to_string_lossy())
        .unwrap();
    assert!(def.generated().static_failure("/Broken").is_some());
    let second = render();
    assert_eq!(header(&second, "X-Moqui-Qvt"), "merged-fragment");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Parity fixture: the XML screen and form features at parity with Java Moqui, as a template, its package and its form requests.
#[test]
fn the_parity_fixture_is_a_template_with_its_data_in_the_packages() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/Parity.qvt", &[]);
    assert_eq!(output.status, 200, "{}", output.body_text());
    assert_eq!(header(&output, "X-Moqui-Qvt"), "static-template");
    let html = html_of(context.factory(), "Parity", "/Parity");
    // a condition element is one v-if: and, or, not, compare and expression, with its fail-widgets as the v-else
    assert!(
        html.contains("$mq.compare(status,'equals','ACTIVE',null,null,'Object') &amp;&amp; (qty &gt; 3)"),
        "{html}"
    );
    assert!(html.contains("<template v-else>"), "{html}");
    golden("Parity.script.js", &output.body_text());
    golden("Parity.template.html", &html);
    let package = render(context.factory(), "/Parity/actions", &[]);
    assert_eq!(package.status, 200, "{}", package.body_text());
    golden("Parity.package.json", &package.body_text());
    // a paginated section-iterate: the page keys in the package, and the list is the page
    let json = json_of(&package);
    assert_eq!(json["linesCount"], 25, "{json}");
    assert_eq!(json["lines"].as_array().map(Vec::len), Some(20), "{json}");
    let second = json_of(&render(context.factory(), "/Parity/actions", &[("pageIndex", "1")]));
    assert_eq!(second["lines"][0]["lineId"], "L21", "{second}");
    assert_eq!(second["linesPageRangeLow"], 21, "{second}");
    assert!(html.contains("<m-form-paginate v-if=\"$scope['linesCount']\""), "{html}");
    // text-map: a name of the text reads the map first
    assert!(html.contains("$mq.inMap((product),'productName')"), "{html}");
    // the accordion: a group of expansion items, the second open; the button-menu: a dropdown of its children
    assert!(html.contains("group=\"ProductForm_accordion\""), "{html}");
    assert!(html.contains("<q-btn-dropdown"), "{html}");
    // the aggregated list: rows grouped by order, each with its sub-list of items and their total
    let orders = render(context.factory(), "/Parity/actions/OrderList", &[]);
    assert_eq!(orders.status, 200, "{}", orders.body_text());
    let orders = json_of(&orders);
    assert_eq!(orders["orderLines"].as_array().map(Vec::len), Some(2), "{orders}");
    assert_eq!(orders["orderLines"][0]["aggregateSubList"][1]["item"], "Bolt", "{orders}");
    golden("Parity.OrderList.json", &serde_json::to_string(&orders).unwrap());
    assert!(html.contains("v-slot:subrow"), "{html}");
}
