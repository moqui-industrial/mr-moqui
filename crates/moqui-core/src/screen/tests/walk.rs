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

//! Path walks: default items, subscreens, path parameters, render-mode extensions, conditional
//! defaults, standalone screens, and the missing `/vapps`.

use moqui_lib::object::Value;

use super::{fixture_root, webroot};
use crate::context::{Error, ErrorKind, ExecutionContextFactory};
use crate::screen::{ScreenFacade, ScreenUrlInfo};

fn walk(factory: &ExecutionContextFactory, path: &str) -> Result<std::sync::Arc<ScreenUrlInfo>, Error> {
    ScreenUrlInfo::parse(factory, &fixture_root(), path, 0)
}

fn names(info: &ScreenUrlInfo) -> Vec<&str> {
    info.path_defs.iter().map(|def| def.screen_name.as_str()).collect()
}

#[test]
fn an_empty_path_follows_default_items_to_the_end() {
    let context = ExecutionContextFactory::test_context();
    let info = walk(context.factory(), "/").unwrap();
    assert_eq!(names(&info), ["Root", "Home"]);
    assert_eq!(info.full_path, ["Home"]);
    assert!(info.minimal_path.is_empty(), "{:?}", info.minimal_path);
    assert_eq!(info.render_path_difference, 0);
    assert_eq!(info.target_screen.screen_name, "Home");

    let deep = walk(context.factory(), "/Sub/Deep").unwrap();
    assert_eq!(names(&deep), ["Root", "Sub", "Deep", "Leaf"]);
    assert_eq!(deep.full_path, ["Sub", "Deep", "Leaf"]);
    assert_eq!(deep.minimal_path, ["Sub", "Deep"]);
    assert_eq!(deep.pre_transition_path, ["Sub", "Deep", "Leaf"]);
    assert_eq!(deep.minimal_path_url(), "/Sub/Deep");
    assert_eq!(deep.full_path_url(), "/Sub/Deep/Leaf");
}

#[test]
fn subscreens_come_from_the_directory_and_from_subscreens_items() {
    let context = ExecutionContextFactory::test_context();
    let facade = ScreenFacade::of(context.factory());
    let root = facade.screen_def(context.factory(), &fixture_root()).unwrap();
    let by_menu: Vec<&str> = root.subscreens.sorted().iter().map(|item| item.name.as_str()).collect();
    // No index first by title, then by index: Linked is index 1 and Home is index 2.
    assert_eq!(by_menu, ["Picker", "Secure", "Standalone", "Sub", "Linked", "Home"]);
    let linked = root.subscreens.get("Linked").unwrap();
    assert_eq!(linked.menu_title, "Linked Items");
    assert_eq!(linked.menu_index, Some(1));
    assert!(linked.menu_include);
    assert_eq!(root.subscreens.get("Home").unwrap().menu_title, "Start");

    let info = walk(context.factory(), "/Linked").unwrap();
    assert_eq!(names(&info), ["Root", "Other"]);
    // A subscreens-item without a location is the file named like it under the screen directory.
    let sub = facade
        .screen_def(
            context.factory(),
            &format!("{}/src/screen/testdata/Root/Sub.xml", env!("CARGO_MANIFEST_DIR")),
        )
        .unwrap();
    assert!(sub
        .subscreens
        .get("Deep")
        .unwrap()
        .location
        .ends_with("Root/Sub/Deep.xml"));
}

#[test]
fn a_transition_ends_the_walk_and_the_rest_are_extra_path_names() {
    let context = ExecutionContextFactory::test_context();
    let info = walk(context.factory(), "/byId/42/more").unwrap();
    assert_eq!(info.target_transition_name.as_deref(), Some("byId"));
    assert_eq!(info.extra_path, ["42", "more"]);
    let transition = info.transition(Some("get")).unwrap();
    assert_eq!(transition.path_parameters, ["itemId"]);
    // A method that does not match a method-specific transition finds none.
    assert!(walk(context.factory(), "/postOnly")
        .unwrap()
        .transition(Some("get"))
        .is_none());
    assert!(walk(context.factory(), "/postOnly")
        .unwrap()
        .transition(Some("post"))
        .is_some());

    // allow-extra-path lets names that match nothing follow a screen.
    let extra = walk(context.factory(), "/Sub/anything/at/all").unwrap();
    assert_eq!(extra.target_screen.screen_name, "Sub");
    assert_eq!(extra.extra_path, ["anything", "at", "all"]);

    let missing = walk(context.factory(), "/NoSuchScreen").unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound, "{missing}");
    let under_deep = walk(context.factory(), "/Sub/Deep/Leaf/Nothing").unwrap_err();
    assert_eq!(under_deep.kind(), ErrorKind::NotFound);
}

#[test]
fn a_query_tail_becomes_inline_parameters() {
    let context = ExecutionContextFactory::test_context();
    let info = walk(context.factory(), "/Sub?a=1&b=x%20y").unwrap();
    assert_eq!(info.inline_parameters.get("a").map(String::as_str), Some("1"));
    assert_eq!(info.inline_parameters.get("b").map(String::as_str), Some("x y"));
    assert_eq!(info.target_screen.screen_name, "Sub");
    // The parameters are on this result only: the cached walk of the same path is shared and stays without them.
    let plain = walk(context.factory(), "/Sub").unwrap();
    assert!(plain.inline_parameters.is_empty());
    let again = walk(context.factory(), "/Sub").unwrap();
    assert!(std::sync::Arc::ptr_eq(&plain, &again));
}

#[test]
fn render_mode_extensions_select_the_mode_and_xsl_fo_is_html_print() {
    let context = ExecutionContextFactory::test_context();
    for (path, mode, standalone) in [
        ("/Sub.html", "html", 0),
        ("/Sub.text", "text", 0),
        ("/Sub.xml", "xml", 0),
        ("/Sub.csv", "csv", 1),
        ("/Sub.html-print", "html-print", 1),
        ("/Sub.xsl-fo", "html-print", 1),
        ("/Sub.qvt", "qvt", 1),
        ("/Sub.qjs", "qjs", 1),
        ("/Sub.qvue", "qvue", 1),
    ] {
        let info = walk(context.factory(), path).unwrap();
        assert_eq!(info.target_render_mode.as_deref(), Some(mode), "{path}");
        assert_eq!(info.last_standalone, standalone, "{path}");
        assert_eq!(info.target_screen.screen_name, "Sub", "{path}");
        // An explicit mode renders that screen and does not follow default items below it.
        assert_eq!(info.full_path, ["Sub"], "{path}");
    }
    // The old Vue modes and anything else are not render modes, so no screen has that name.
    for path in ["/Sub.vuet", "/Sub.js", "/Sub.vue", "/Sub.vapps", "/Sub.nonsense"] {
        let err = walk(context.factory(), path).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound, "{path}: {err}");
    }
    // Always-standalone modes render only the target: the screens above only run actions.
    let csv = walk(context.factory(), "/Sub/Deep/Leaf.csv").unwrap();
    assert_eq!(names(&csv), ["Root", "Sub", "Deep", "Leaf"]);
    assert_eq!(csv.render_path_difference, 3);
    assert_eq!(csv.render_defs().len(), 1);
    let html = walk(context.factory(), "/Sub/Deep/Leaf.html").unwrap();
    assert_eq!(html.render_path_difference, 0);
}

#[test]
fn a_transition_takes_a_render_mode_extension_too() {
    let context = ExecutionContextFactory::test_context();
    let info = walk(context.factory(), "/goUrl.json").err();
    // json is not a render mode, so the name stays whole and matches nothing.
    assert_eq!(info.map(|err| err.kind()), Some(ErrorKind::NotFound));
    let info = walk(context.factory(), "/goUrl.text").unwrap();
    assert_eq!(info.target_transition_name.as_deref(), Some("goUrl"));
    assert_eq!(info.target_transition_extension.as_deref(), Some("text"));
}

#[test]
fn a_standalone_screen_drops_the_screens_above_it_from_the_render() {
    let context = ExecutionContextFactory::test_context();
    let info = walk(context.factory(), "/Standalone/Child").unwrap();
    assert_eq!(names(&info), ["Root", "Standalone", "Child"]);
    assert_eq!(info.render_path_difference, 1);
    // A negative lastStandalone drops that many from the top.
    let info = ScreenUrlInfo::parse(context.factory(), &fixture_root(), "/Sub/Deep/Leaf", -2).unwrap();
    assert_eq!(info.render_path_difference, 2);
}

#[test]
fn a_conditional_default_replaces_the_default_item() {
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    let a = walk(context.factory(), "/Picker").unwrap();
    assert_eq!(a.full_path, ["Picker", "A"]);
    assert!(!a.reusable);
    let _frame = crate::script::ContextFrame::push();
    crate::script::set_context_name("mode", Value::string("b")).unwrap();
    let b = walk(context.factory(), "/Picker").unwrap();
    assert_eq!(b.full_path, ["Picker", "B"]);
}

#[test]
fn vapps_is_not_a_screen_under_webroot() {
    let context = ExecutionContextFactory::test_context();
    let root = webroot().to_string_lossy().into_owned();
    for path in ["/vapps", "/vapps/ScreenTree", "/vapps.qvt"] {
        let err = ScreenUrlInfo::parse(context.factory(), &root, path, 0).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound, "{path}: {err}");
    }
    // The apps that stay are there.
    for path in ["/qapps", "/apps", "/rest/s1/x", "/Login"] {
        ScreenUrlInfo::parse(context.factory(), &root, path, 0).unwrap_or_else(|err| panic!("{path}: {err}"));
    }
    let facade = ScreenFacade::of(context.factory());
    let webroot_def = facade.screen_def(context.factory(), &root).unwrap();
    assert!(webroot_def.subscreens.get("vapps").is_none());
}

#[test]
fn text_output_conf_has_the_modes_and_the_print_alias() {
    let context = ExecutionContextFactory::test_context();
    let facade = ScreenFacade::of(context.factory());
    assert_eq!(facade.mime_type("csv"), Some("text/csv"));
    assert_eq!(facade.mime_type("xsl-fo"), Some("text/html"));
    assert!(facade.is_always_standalone("xsl-fo"));
    assert!(facade.is_always_standalone("html-print"));
    assert!(!facade.is_always_standalone("html"));
    assert!(!facade.is_render_mode_valid("vuet"));
    assert!(facade
        .text_output("qvt")
        .unwrap()
        .macro_template_location
        .ends_with("ScreenQvt.js"));
}

#[test]
fn the_default_conf_parses_into_the_factory_conf() {
    let conf = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../runtime/base-component/framework/conf/MoquiDefaultConf.xml"
    ))
    .unwrap();
    let parsed = super::super::cache::ScreenConf::parse(&conf);
    let mut modes: Vec<&str> = parsed.text_output.keys().map(String::as_str).collect();
    modes.sort_unstable();
    assert_eq!(
        modes,
        ["csv", "html", "html-print", "qjs", "qvt", "qvue", "text", "xlsx", "xml"]
    );
    // xlsx is the one screen-output (binary) mode
    let xlsx = &parsed.text_output["xlsx"];
    assert!(xlsx.binary && xlsx.always_standalone);
    assert_eq!(xlsx.macro_template_location, "framework/template/screen/ScreenXlsx.js");
    assert!(!parsed.text_output["csv"].binary);
    let print = &parsed.text_output["html-print"];
    assert_eq!(print.mime_type, "text/html");
    assert!(print.always_standalone);
    assert_eq!(
        print.macro_template_location,
        "framework/template/screen/ScreenHtmlPrint.js"
    );
    assert!(!parsed.text_output["html"].always_standalone);
    assert_eq!(parsed.default_paginate_rows, 20);
    for dropped in ["vuet", "js", "vue"] {
        assert!(!parsed.text_output.contains_key(dropped), "{dropped}");
    }
}
