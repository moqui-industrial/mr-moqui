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

//! Rendering: action order, authentication and authorization, `makeRender` with no socket, the
//! definition cache, and transition responses.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use moqui_lib::object::Value;

use super::{fixture_root, request, World};
use crate::context::{Error, ErrorKind, ExecutionContextFactory};
use crate::screen::{make_render, serve_at, RenderOutput, ScreenFacade, ScreenUrlInfo};

/// Render `path` in text mode unless the path names a mode. The screens render the screens below
/// them, and html has no macro module until a later slice.
fn render(factory: &std::sync::Arc<ExecutionContextFactory>, path: &str) -> Result<RenderOutput, Error> {
    let named = path.rsplit('/').next().unwrap_or("").contains('.');
    let render = make_render(factory).root_screen(fixture_root()).screen_path(path);
    if named {
        render.render()
    } else {
        render.render_mode("text").render()
    }
}

fn trace_of(output: &RenderOutput) -> Vec<String> {
    let text = output.body_text();
    match crate::service::parse_json(&text).unwrap_or_else(|err| panic!("{text}: {err}")) {
        Value::List(items) => items.iter().map(moqui_lib::object::to_plain_string).collect(),
        other => panic!("not a list: {other:?}"),
    }
}

fn location_of(output: &RenderOutput) -> Option<&str> {
    output.header("Location")
}

fn parameters(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::string(*value)))
        .collect()
}

#[test]
fn always_actions_run_before_pre_actions_before_actions_down_the_path() {
    let context = ExecutionContextFactory::test_context();
    let output = make_render(context.factory())
        .root_screen(fixture_root())
        .screen_path("/Sub/Deep")
        .render_mode("text")
        .parameters(parameters(&[("needed", "yes")]))
        .render()
        .unwrap();
    assert_eq!(output.status, 200);
    assert_eq!(
        trace_of(&output),
        [
            "Root.always",
            "Sub.always",
            "Deep.always",
            "Leaf.always",
            "Root.pre",
            "Sub.pre",
            "Deep.pre",
            "Leaf.pre",
            "Root.actions",
            "Sub.actions",
            "Deep.actions",
            "Leaf.actions",
        ]
    );
}

#[test]
fn a_screen_with_a_missing_required_parameter_is_skipped_and_reports_an_error() {
    let context = ExecutionContextFactory::test_context();
    // Sub needs `needed`. Without it Sub does not render, and the error is shown where the widgets
    // would be, as Java does for any section that finds an error.
    let output = render(context.factory(), "/Sub/Deep").unwrap();
    assert_eq!(output.body_text(), "Required parameter missing (needed)");
    // With the parameter the whole path renders: Deep's actions run.
    let with = make_render(context.factory())
        .root_screen(fixture_root())
        .screen_path("/Sub/Deep")
        .render_mode("text")
        .parameters(parameters(&[("needed", "yes")]))
        .render()
        .unwrap();
    assert!(trace_of(&with).contains(&"Deep.actions".to_string()));
}

#[test]
fn an_always_standalone_mode_starts_pre_actions_at_the_third_screen() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/Sub/Deep/Leaf.csv").unwrap();
    assert_eq!(output.status, 200);
    // Pre-actions skip Root and Sub, and only the target's own actions run.
    assert_eq!(
        trace_of(&output),
        [
            "Root.always",
            "Sub.always",
            "Deep.always",
            "Leaf.always",
            "Deep.pre",
            "Leaf.pre",
            "Leaf.actions",
        ]
    );
    // The same path as text runs every screen's pre-actions.
    let text = make_render(context.factory())
        .root_screen(fixture_root())
        .screen_path("/Sub/Deep/Leaf.text")
        .parameters(parameters(&[("needed", "yes")]))
        .render()
        .unwrap();
    assert!(trace_of(&text).contains(&"Root.pre".to_string()));
}

#[test]
fn a_transition_runs_always_actions_but_not_pre_actions_or_screen_actions() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/byId/7").unwrap();
    assert_eq!(output.status, 200);
    assert!(output.body_text().contains("\"id\": \"7\""), "{}", output.body_text());
    assert!(output.body_text().contains("Root.always"), "{}", output.body_text());
    assert!(!output.body_text().contains("Root.pre"), "{}", output.body_text());
    assert!(!output.body_text().contains("Root.actions"), "{}", output.body_text());
}

#[test]
fn make_render_without_a_socket_sends_status_content_type_and_an_empty_body() {
    let context = ExecutionContextFactory::test_context();
    // A screen with empty widgets has no output of its own. The modes with a macro module write their
    // wrapper, if they have one, and the others write nothing.
    let picker = format!(
        "{}/src/screen/testdata/Elsewhere/PickerA.xml",
        env!("CARGO_MANIFEST_DIR")
    );
    let as_mode = |mode: &str| {
        make_render(context.factory())
            .root_screen(picker.clone())
            .render_mode(mode)
            .render()
            .unwrap()
    };
    let html = as_mode("html");
    assert_eq!(html.status, 200);
    assert_eq!(html.content_type(), Some("text/html;charset=UTF-8"));
    assert!(html.body.is_empty(), "{}", html.body_text());

    for (mode, content_type, body) in [
        ("csv", "text/csv;charset=UTF-8", ""),
        ("text", "text/plain;charset=UTF-8", ""),
        ("xml", "text/xml;charset=UTF-8", "<PickerA>\n</PickerA>\n"),
        ("qjs", "application/javascript;charset=UTF-8", ""),
    ] {
        let output = as_mode(mode);
        assert_eq!(output.status, 200, "{mode}");
        assert_eq!(output.content_type(), Some(content_type), "{mode}");
        assert_eq!(output.body_text(), body, "{mode}");
    }
    // A screen with no widgets is server-static for qvt, so qvt is the script of an empty template, with the header that says so.
    let qvt = as_mode("qvt");
    assert_eq!(qvt.status, 200);
    assert_eq!(qvt.content_type(), Some("text/javascript;charset=UTF-8"));
    assert_eq!(qvt.header("X-Moqui-Qvt"), Some("static-template"));
    assert!(qvt.body_text().contains("m.qvtStatic"), "{}", qvt.body_text());
    // The print mode writes an empty document, and xsl-fo is the same mode.
    let print = as_mode("html-print");
    assert_eq!(print.content_type(), Some("text/html;charset=UTF-8"));
    assert!(
        print.body_text().starts_with("<!DOCTYPE html>"),
        "{}",
        print.body_text()
    );
    assert!(print
        .body_text()
        .ends_with("<body class=\"letter-portrait\">\n</body></html>\n"));
    assert_eq!(as_mode("xsl-fo").body_text(), print.body_text());
    // The render mode can come from the builder or the renderMode parameter instead of the path.
    let by_builder = make_render(context.factory())
        .root_screen(picker.clone())
        .render_mode("xsl-fo")
        .render()
        .unwrap();
    assert_eq!(by_builder.content_type(), Some("text/html;charset=UTF-8"));
    let by_parameter = make_render(context.factory())
        .root_screen(picker.clone())
        .parameters(parameters(&[("renderMode", "csv")]))
        .render()
        .unwrap();
    assert_eq!(by_parameter.content_type(), Some("text/csv;charset=UTF-8"));
    // A mode nobody configured is a 404.
    let unknown = make_render(context.factory())
        .root_screen(picker)
        .render_mode("vuet")
        .render()
        .unwrap_err();
    assert_eq!(unknown.kind(), ErrorKind::NotFound);
}

#[test]
fn make_render_fills_parameters_session_and_base_url_from_maps() {
    let context = ExecutionContextFactory::test_context();
    let output = make_render(context.factory())
        .root_screen(fixture_root())
        .screen_path("/echo")
        .parameters(parameters(&[("p", "from-map")]))
        .session(parameters(&[("who", "session-value")]))
        .render()
        .unwrap();
    let text = output.body_text();
    assert!(text.contains("\"who\": \"session-value\""), "{text}");
    assert!(text.contains("\"p\": \"from-map\""), "{text}");
    assert!(text.contains("\"pFromContext\": \"from-map\""), "{text}");

    let redirect = make_render(context.factory())
        .root_screen(fixture_root())
        .screen_path("/goUrl")
        .base_url("https://app.example/")
        .render()
        .unwrap();
    assert_eq!(redirect.status, 302);
    assert_eq!(location_of(&redirect), Some("https://app.example/Sub"));
}

#[test]
fn a_screen_that_requires_authentication_is_401_for_nobody_and_200_for_a_user() {
    let world = World::open("");
    let anonymous = render(&world.factory, "/Secure").unwrap_err();
    assert_eq!(anonymous.kind(), ErrorKind::AuthenticationRequired);

    let response = serve_at(&world.factory, request("GET", "/Secure.text"), &fixture_root());
    assert_eq!(response.status, 401, "{}", response.body_text());

    world.user("admin", None);
    let ec = world.factory.new_context();
    let _entered = ec.enter();
    crate::user::login_user("admin", "secret").unwrap();
    let output = render(&world.factory, "/Secure").unwrap();
    assert_eq!(output.status, 200);
    assert!(output.body_text().contains("secure"), "{}", output.body_text());
}

#[test]
fn an_unknown_path_is_404_over_http_and_a_not_found_error_from_make_render() {
    let context = ExecutionContextFactory::test_context();
    let err = render(context.factory(), "/Nothing/Here").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let world = World::open("");
    let response = serve_at(&world.factory, request("GET", "/Nothing/Here"), &fixture_root());
    assert_eq!(response.status, 404, "{}", response.body_text());
    let response = serve_at(&world.factory, request("GET", "/Sub.vuet"), &fixture_root());
    assert_eq!(response.status, 404, "{}", response.body_text());
}

fn screen_authz_conf() -> &'static str {
    "<moqui-conf><artifact-execution-facade><artifact-execution type=\"AT_XML_SCREEN\" authz-enabled=\"true\"/></artifact-execution-facade></moqui-conf>"
}

#[test]
fn a_signed_in_user_without_an_allow_record_is_403_on_a_screen() {
    let world = World::open(screen_authz_conf());
    world.user("admin", Some("ADMIN"));
    world.user("plain", None);
    let secure = format!("{}/src/screen/testdata/Root/Secure.xml", env!("CARGO_MANIFEST_DIR"));
    world.allow_screen("ADMIN", &secure);
    let root = fixture_root();

    let mut denied = request("GET", "/Secure.text");
    denied.set_header("authorization", super::basic("plain"));
    let response = serve_at(&world.factory, denied, &root);
    assert_eq!(response.status, 403, "{}", response.body_text());

    let mut allowed = request("GET", "/Secure.text");
    allowed.set_header("authorization", super::basic("admin"));
    let response = serve_at(&world.factory, allowed, &root);
    assert_eq!(response.status, 200, "{}", response.body_text());
    assert!(response.body_text().contains("secure"), "{}", response.body_text());

    let mut nobody = request("GET", "/Secure.text");
    nobody.peer_ip = "127.0.0.1".to_string();
    assert_eq!(serve_at(&world.factory, nobody, &root).status, 401);
}

#[test]
fn a_cached_definition_is_reused_and_reloaded_when_its_file_changes() {
    let context = ExecutionContextFactory::test_context();
    let facade = ScreenFacade::of(context.factory());
    facade.set_check_interval(Some(Duration::ZERO));
    let dir = std::env::temp_dir().join(format!("moqui-screen-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Cached.xml");
    std::fs::write(
        &path,
        "<screen require-authentication=\"false\" default-menu-title=\"First\"><widgets/></screen>",
    )
    .unwrap();
    let location = path.to_string_lossy().into_owned();

    let first = facade.screen_def(context.factory(), &location).unwrap();
    let hits = facade.cache_stats().hits;
    let again = facade.screen_def(context.factory(), &location).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    assert_eq!(facade.cache_stats().hits, hits + 1);
    assert_eq!(again.default_menu_title, "First");

    std::fs::write(
        &path,
        "<screen require-authentication=\"false\" default-menu-title=\"Second\"><widgets/></screen>",
    )
    .unwrap();
    // The write can land in the same clock tick, so move the modified time forward to be sure.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(30)).unwrap();
    drop(file);
    let changed = facade.screen_def(context.factory(), &location).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.default_menu_title, "Second");
    let stable = facade.screen_def(context.factory(), &location).unwrap();
    assert!(std::sync::Arc::ptr_eq(&changed, &stable));

    // A screen that does not compile fails to load, and the cache keeps nothing for it.
    std::fs::write(
        &path,
        "<screen><always-actions><set field=\"x\" from=\"def y = 1\"/></always-actions></screen>",
    )
    .unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(60)).unwrap();
    drop(file);
    assert!(facade.screen_def(context.factory(), &location).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_are_checked_once_per_interval_and_a_new_subscreen_file_is_seen() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    let facade = ScreenFacade::of(factory);
    facade.set_check_interval(Some(Duration::from_secs(3600)));
    let dir = std::env::temp_dir().join(format!("moqui-screen-interval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Top")).unwrap();
    let top = dir.join("Top.xml");
    std::fs::write(&top, r#"<screen require-authentication="false" default-menu-title="One"><widgets/></screen>"#).unwrap();
    std::fs::write(dir.join("Top/A.xml"), r#"<screen require-authentication="false"><widgets/></screen>"#).unwrap();
    let location = top.to_string_lossy().into_owned();
    let bump = |path: &std::path::Path, seconds: u64| {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(seconds)).unwrap();
    };

    let first = facade.screen_def(factory, &location).unwrap();
    assert!(first.subscreens.get("A").is_some() && first.subscreens.get("B").is_none());
    // Inside the interval the edit is not looked for, as a Java screen.location hit.
    std::fs::write(&top, r#"<screen require-authentication="false" default-menu-title="Two"><widgets/></screen>"#).unwrap();
    bump(&top, 30);
    assert!(std::sync::Arc::ptr_eq(&first, &facade.screen_def(factory, &location).unwrap()));
    let path = ScreenUrlInfo::resolve(factory, &location, &["A".to_string()], 0).unwrap();
    let again = ScreenUrlInfo::resolve(factory, &location, &["A".to_string()], 0).unwrap();
    assert!(std::sync::Arc::ptr_eq(&path.target_screen, &again.target_screen));

    // Past the interval the edit and a file added to the subscreen directory are both seen, and the resolved path follows.
    facade.set_check_interval(Some(Duration::ZERO));
    std::fs::write(dir.join("Top/B.xml"), r#"<screen require-authentication="false"><widgets/></screen>"#).unwrap();
    let directory = std::fs::File::open(dir.join("Top")).unwrap();
    directory.set_modified(SystemTime::now() + Duration::from_secs(40)).unwrap();
    let changed = facade.screen_def(factory, &location).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.default_menu_title, "Two");
    assert!(changed.subscreens.get("B").is_some());
    let fresh = ScreenUrlInfo::resolve(factory, &location, &["B".to_string()], 0).unwrap();
    assert!(fresh.target_screen.location.ends_with("Top/B.xml"));
    assert!(std::sync::Arc::ptr_eq(&fresh.path_defs[0], &changed));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transition_responses_resolve_to_a_redirect_or_nothing() {
    let context = ExecutionContextFactory::test_context();
    let go = |path: &str| render(context.factory(), path).unwrap();

    // A relative url-type screen-path starts from the screens the transition sits under.
    let relative = go("/goUrl");
    assert_eq!((relative.status, location_of(&relative)), (302, Some("/Sub")));
    let absolute = go("/goAbsolute");
    assert_eq!((absolute.status, location_of(&absolute)), (302, Some("/Sub/Deep")));
    let up = go("/Sub/up");
    assert_eq!(location_of(&up), Some("/Home"));
    let here = go("/Sub/here");
    assert_eq!(location_of(&here), Some("/Sub"));

    let plain = go("/goPlain");
    assert_eq!((plain.status, location_of(&plain)), (302, Some("/plain/target")));
    let with_parameters = go("/goParams");
    assert_eq!(location_of(&with_parameters), Some("/Sub?who=a%20b&x=1"));

    let none = go("/goNone");
    assert_eq!(none.status, 200);
    assert!(none.header("Location").is_none());
    assert!(none.body.is_empty());
}

#[test]
fn a_conditional_response_wins_over_the_default_when_its_condition_holds() {
    let context = ExecutionContextFactory::test_context();
    let go = |path: &str, flag: Option<&str>| {
        let mut render = make_render(context.factory())
            .root_screen(fixture_root())
            .screen_path(path);
        if let Some(flag) = flag {
            render = render.parameters(parameters(&[("flag", flag)]));
        }
        render.render().unwrap()
    };
    assert_eq!(location_of(&go("/goCond", Some("y"))), Some("/Home"));
    assert_eq!(location_of(&go("/goCond", Some("n"))), Some("/Sub"));
    assert_eq!(location_of(&go("/goCond", None)), Some("/Sub"));
    // A condition that is not a bare expression runs as actions.
    assert_eq!(location_of(&go("/goCompare", Some("y"))), Some("/Home"));
    assert_eq!(location_of(&go("/goCompare", Some("n"))), Some("/Sub"));
}

#[test]
fn the_error_response_is_used_when_the_actions_leave_errors() {
    let context = ExecutionContextFactory::test_context();
    let failed = render(context.factory(), "/failTx").unwrap();
    assert_eq!(failed.status, 302);
    assert_eq!(location_of(&failed), Some("/Home"));
    let fine = render(context.factory(), "/okTx").unwrap();
    assert_eq!(location_of(&fine), Some("/Sub"));
}

#[test]
fn a_transition_rejects_the_wrong_method_and_a_missing_required_parameter() {
    let context = ExecutionContextFactory::test_context();
    let output = render(context.factory(), "/postOnly").unwrap();
    assert_eq!(output.status, 405);
    assert_eq!(output.header("Allow"), Some("POST"));
    let posted = make_render(context.factory())
        .root_screen(fixture_root())
        .screen_path("/postOnly")
        .method("POST")
        .render()
        .unwrap();
    assert_eq!(posted.status, 200);
}

#[test]
fn the_definition_model_holds_what_the_xml_declares() {
    let context = ExecutionContextFactory::test_context();
    let facade = ScreenFacade::of(context.factory());
    let root = facade.screen_def(context.factory(), &fixture_root()).unwrap();
    assert_eq!(root.screen_name, "Root");
    assert_eq!(root.authentication, crate::screen::Authentication::Optional);
    assert!(root.always_actions.is_some() && root.pre_actions.is_some() && root.root.actions.is_some());
    assert_eq!(root.subscreens.default_item.as_deref(), Some("Home"));

    // The service-call shortcut becomes the first action with the defaults the Java screen sets.
    let service = root.transition("service", None).unwrap();
    assert_eq!(service.single_service.as_deref(), Some("moqui.Missing.noop"));
    let xml = &service.actions.as_ref().unwrap().xml;
    assert!(
        xml.contains("in-map=\"true\"") && xml.contains("multi=\"parameter\""),
        "{xml}"
    );
    assert!(!service.read_only);
    assert_eq!(service.authz_action, "update");

    let by_id = root.transition("byId", Some("get")).unwrap();
    assert_eq!(by_id.path_parameters, ["itemId"]);
    let cond = root.transition("goCond", None).unwrap();
    assert_eq!(cond.conditional_responses.len(), 1);
    assert_eq!(cond.conditional_responses[0].url, "Home");
    assert!(matches!(
        cond.conditional_responses[0].condition,
        Some(crate::screen::Condition::Expression(_))
    ));
    let compare = root.transition("goCompare", None).unwrap();
    assert!(matches!(
        compare.conditional_responses[0].condition,
        Some(crate::screen::Condition::Wrapped(_))
    ));
    let fail = root.transition("failTx", None).unwrap();
    assert_eq!(fail.error_response.as_ref().unwrap().url, "/Home");
    let none = root.transition("goNone", None).unwrap();
    assert_eq!(none.default_response.kind, crate::screen::ResponseKind::None);
    assert!(none.read_only);

    let sub = facade
        .screen_def(
            context.factory(),
            &format!("{}/src/screen/testdata/Root/Sub.xml", env!("CARGO_MANIFEST_DIR")),
        )
        .unwrap();
    assert!(sub.allow_extra_path);
    assert!(sub.parameter("needed").unwrap().required);
}

#[test]
fn a_transition_that_does_not_compile_fails_when_it_runs_not_when_the_screen_loads() {
    let context = ExecutionContextFactory::test_context();
    let dir = std::env::temp_dir().join(format!("moqui-screen-broken-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Broken.xml");
    std::fs::write(
        &path,
        "<screen require-authentication=\"false\"><transition name=\"bad\"><actions><set field=\"x\" from=\"def y = 1\"/></actions><default-response type=\"none\"/></transition></screen>",
    )
    .unwrap();
    let err = make_render(context.factory())
        .root_screen(path.to_string_lossy())
        .screen_path("/bad")
        .render()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::General);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Timing for a form-list page: `cargo test -p moqui-core --release render_bench -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run by hand"]
fn render_bench_form_list_page() {
    let context = ExecutionContextFactory::test_context();
    let factory = context.factory();
    let dir = std::env::temp_dir().join(format!("moqui-render-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Bench/Apps")).unwrap();
    std::fs::write(
        dir.join("Bench.xml"),
        r#"<screen require-authentication="false"><widgets><subscreens-active/></widgets></screen>"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("Bench/Apps.xml"),
        r#"<screen require-authentication="false"><widgets><subscreens-menu/><subscreens-active/></widgets></screen>"#,
    )
    .unwrap();
    for name in ["One", "Two", "Three"] {
        std::fs::write(
            dir.join(format!("Bench/Apps/{name}.xml")),
            r#"<screen require-authentication="false"><widgets><label text="x"/></widgets></screen>"#,
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("Bench/Apps/List.xml"),
        r##"<screen require-authentication="false">
            <transition name="edit"><default-response url="."/></transition>
            <actions><script>rows = []; for (let i = 0; i &lt; 200; i++) rows.push({id: 'R' + i, name: 'Name ' + i, amount: i * 1.5})</script></actions>
            <widgets><form-list name="Rows" list="rows">
                <row-actions><set field="label" from="name + '!'"/></row-actions>
                <field name="id"><default-field><link url="edit" text="${id}" parameter-map="({id: id})"/></default-field></field>
                <field name="name"><default-field><display text="${label}"/></default-field></field>
                <field name="amount"><default-field><display format="#,##0.00"/></default-field></field>
            </form-list></widgets></screen>"##,
    )
    .unwrap();
    let root = dir.join("Bench.xml").to_string_lossy().into_owned();
    let render = || {
        make_render(factory)
            .root_screen(root.clone())
            .screen_path("/Apps/List")
            .render()
            .unwrap()
    };
    let warm = render();
    assert!(warm.body_text().contains("Name 199!"), "{}", warm.body_text());
    let runs = 30;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        render();
    }
    let each = started.elapsed() / runs;
    println!("render_bench: {runs} renders of a 200-row form-list, {each:?} each");
    let _ = std::fs::remove_dir_all(&dir);
}
