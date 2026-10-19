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

//! The qvt mode: the Vue template each element writes. A fixture screen in `script/testdata/screen/qvt/render/<name>.xml`
//! is rendered and compared with `<name>.qvt` beside it. Set `UPDATE_GOLDEN=1` to rewrite the expected files, then
//! read the diff. Every template, here and in the other tests that render one, is compiled with the vendored
//! Vue compiler in QuickJS and fails on any compiler error.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::context::ExecutionContextFactory;
use crate::screen::{make_render, vue_check};
use moqui_lib::object::Value;

fn render_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/script/testdata/screen/qvt/render")
}

fn compiler_script() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/base-component/webroot/screen/webroot/libs/vue/compiler-dom.cjs.js");
    std::fs::read_to_string(path).expect("compiler-dom.cjs.js")
}

/// Assert that every template compiles with the Vue compiler.
pub(crate) fn assert_compiles(names: &[String], templates: &[String]) {
    assert_eq!(names.len(), templates.len());
    let errors = vue_check::compile_errors(&compiler_script(), templates).unwrap();
    let named: Vec<String> = errors
        .iter()
        .map(|error| {
            let index: usize = error.split(':').next().and_then(|text| text.parse().ok()).unwrap_or(0);
            format!("{}: {error}", names.get(index).map_or("?", String::as_str))
        })
        .collect();
    assert!(named.is_empty(), "Vue compiler errors:\n{}", named.join("\n"));
}

fn render_path(name: &str, path: &str, parameters: &[(&str, &str)]) -> String {
    let context = ExecutionContextFactory::test_context();
    let map: BTreeMap<String, Value> = parameters
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::string(*value)))
        .collect();
    let output = make_render(context.factory())
        .root_screen(render_dir().join(format!("{name}.xml")).to_string_lossy().into_owned())
        .screen_path(path)
        .render_mode("qvt")
        .parameters(map)
        .render()
        .unwrap_or_else(|err| panic!("{name}: {err}"));
    assert_eq!(output.content_type(), Some("text/html;charset=UTF-8"));
    output.body_text()
}

fn check(name: &str, parameters: &[(&str, &str)]) -> String {
    check_path(name, "", name, parameters)
}

fn check_path(name: &str, screen_path: &str, golden: &str, parameters: &[(&str, &str)]) -> String {
    let actual = render_path(name, screen_path, parameters);
    let path = render_dir().join(format!("{golden}.qvt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap();
    } else {
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {}", path.display()));
        assert_eq!(actual, expected, "{name} differs from {}", path.display());
    }
    assert_compiles(&[golden.to_string()], std::slice::from_ref(&actual));
    actual
}

#[test]
fn containers_links_and_labels() {
    let out = check("containers", &[]);
    // data is text and not an interpolation
    assert!(!out.contains("{{"), "{out}");
}

#[test]
fn form_single_with_every_field_type() {
    let out = check("fields", &[("_openDialog", "none")]);
    assert!(out.contains("v-slot:default=\"formProps\""), "{out}");
    assert!(out.contains(":fields-initial=\""), "{out}");
    for wrapper in [
        "m-text-line",
        "m-text-area",
        "m-drop-down",
        "m-check",
        "m-radio",
        "m-date-time",
        "m-date-period",
        "m-display",
        "m-file",
        "m-text-find",
        "m-range-find",
    ] {
        assert!(out.contains(&format!("<{wrapper}")), "{wrapper} in {out}");
    }
    for raw in ["<q-input", "<q-select", "<q-date", "<q-time", "<q-field"] {
        assert!(!out.contains(raw), "{raw} in {out}");
    }
}

/// `context` is an m-screen-context with the request's values (a title localized and encoded, an expression that comes to
/// nothing left out), and `shell-providers`, a child of the screen, renders nothing in any mode.
#[test]
fn the_screen_context_and_the_shell_providers() {
    let out = check("shell", &[]);
    assert!(
        out.contains(r#"<m-screen-context record-entity="mantle.order.OrderHeader" record-id="SO10023" title="Order {{x}} &amp; more" selection-form="ItemList"></m-screen-context>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<m-screen-context record-entity="mantle.order.OrderHeader"></m-screen-context>"#),
        "{out}"
    );
    for provider in ["quick-create", "QuickNote", "palette", "Records", "g l"] {
        assert!(!out.contains(provider), "{provider} in {out}");
    }
    for mode in ["html", "text", "csv", "xml"] {
        let context = ExecutionContextFactory::test_context();
        let output = make_render(context.factory())
            .root_screen(render_dir().join("shell.xml").to_string_lossy().into_owned())
            .screen_path("")
            .render_mode(mode)
            .render()
            .unwrap_or_else(|err| panic!("{mode}: {err}"));
        let text = output.body_text();
        // csv and xml write only forms
        assert!(
            matches!(mode, "csv" | "xml") || text.contains("the context renders nothing visible"),
            "{mode}: {text}"
        );
        for written in ["SO10023", "screen-context", "QuickNote", "Records"] {
            assert!(!text.contains(written), "{written} in {mode}: {text}");
        }
    }
}

#[test]
fn form_lists() {
    let out = check("lists", &[]);
    assert!(out.contains("<m-form-paginate"), "{out}");
    assert!(out.contains("class=\"tr\""), "{out}");
}

/// An anchor to a transition that only redirects is a link to that screen. A transition with actions, and
/// `expand-transition-url="false"`, stay on the transition so a submit still posts it.
#[test]
fn an_alias_transition_link_points_at_the_screen() {
    // `.qvt` is always-standalone, so the body is List. A plain path renders Alias and stops at its placeholder.
    let out = render_path("Alias", "/List.qvt", &[]);
    assert_compiles(&["alias".to_string()], std::slice::from_ref(&out));
    assert!(out.contains("href=\"/Detail?id=I1&amp;fromResponse=yes\""), "{out}");
    assert!(
        out.contains(
            "<m-link href=\"/Detail?id=I1&amp;fromResponse=yes\" id=\"Rows_id_0_detail\" class=\"m-link-text\">"
        ),
        "{out}"
    );
    assert!(out.contains("href=\"/List/detail?id=I1\""), "{out}");
    assert!(out.contains("href=\"/List/acted?id=I1\""), "{out}");
    // expand defaults to true, so this submit goes to the screen. false keeps the post on the transition.
    assert!(out.contains("action=\"/Detail\""), "{out}");
    assert!(out.contains("action=\"/List/detail\""), "{out}");
    // The extra path of an alias is the path-parameter, outside the row so the row's id is not the source.
    assert!(
        out.contains("href=\"/Detail?id=I1\" class=\"m-link-text\">By path"),
        "{out}"
    );
    // type=none is not a screen URL, so the anchor is <a>. The alias above stays m-link. In a list cell a screen link is text.
    assert!(out.contains("<a href=\"/List/gone\" class=\"m-link-text\">"), "{out}");
    assert!(!out.contains("<m-link href=\"/List/gone\">"), "{out}");
    // An http(s) value with url-type screen is that string, not a screen walk.
    assert!(
        out.contains("<a href=\"https://example.com/x\" class=\"m-link-text\">"),
        "{out}"
    );
    // A screen that does not exist is `#`.
    assert!(out.contains(">#</span>"), "{out}");
}

#[test]
fn subscreen_menus_tabs_and_panels() {
    check_path("Menu", "", "menu-one", &[]);
    check_path("Menu", "/Two", "menu-two", &[]);
    check_path("Panel", "", "panel-one", &[]);
    check_path("Panel", "/Two", "panel-two", &[("lastStandalone", "0")]);
}

#[test]
fn request_text_that_looks_like_an_interpolation_is_not_one() {
    let out = check("braces", &[]);
    // text is character references, and an attribute is not an interpolation in Vue
    assert!(
        out.contains("<p class=\"text-inline\">a &#123;&#123;b}} c</p>"),
        "{out}"
    );
    assert!(
        out.contains("<span class=\"text-inline\">&#123;&#123;literal}}</span>"),
        "{out}"
    );
}

#[test]
fn row_selection_multi_forms_find_dialogs_and_the_options_of_a_drop_down() {
    let out = check("extras", &[]);
    for wrapper in [
        "<m-checkbox-set",
        "<m-form-column-config",
        "<m-container-dialog",
        "<m-form-paginate",
        "<m-text-find",
        "<m-check",
        "<m-text-area",
        "<m-date-time",
        "<m-display",
        "<m-form-link",
    ] {
        assert!(out.contains(wrapper), "{wrapper} in {out}");
    }
    assert!(out.contains("editor-type=\"md\""), "{out}");
    // the checkbox of each row points at the state of the set
    assert!(out.contains(":model-value=\"formProps.checkboxStates[0]\""), "{out}");
    assert!(out.contains(":checkbox-values=\"['I1','I2']\""), "{out}");
    // a form that posts all rows has one set of fields with the row number after each name
    assert!(
        out.contains("'title_0':'One'") && out.contains("'title_1':'Two'"),
        "{out}"
    );
}

/// A qvt render error is logged and written in place of what failed, as the html render does: a section's error replaces
/// the section and the screen goes on, and an error in the screen itself replaces the screen, which `.qvt` renders alone.
/// What the failed part wrote is dropped so the template still compiles, and a brace in the message is not an
/// interpolation.
#[test]
fn a_qvt_render_error_is_logged_and_written_in_its_place() {
    let dir = std::env::temp_dir().join(format!("moqui-qvt-error-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Root")).unwrap();
    let write = |path: &str, text: &str| std::fs::write(dir.join(path), text).unwrap();
    write(
        "Root.xml",
        r#"<screen require-authentication="false"><subscreens default-item="Section"/>
            <widgets><subscreens-active/></widgets></screen>"#,
    );
    write(
        "Root/Section.xml",
        r#"<screen require-authentication="false"><widgets>
            <label text="before"/>
            <section name="Broken"><widgets><container><label text="inside"/>
                <render-mode><text type="qvt"><![CDATA[<b>${nope.bad()}</b>]]></text></render-mode>
            </container></widgets></section>
            <label text="after"/>
        </widgets></screen>"#,
    );
    write(
        "Root/Whole.xml",
        r#"<screen require-authentication="false"><widgets><container><label text="first"/>
            <render-mode><text type="qvt"><![CDATA[${(() => { throw new Error('a {{ brace }}') })()}]]></text></render-mode>
        </container></widgets></screen>"#,
    );
    let root = dir.join("Root.xml").to_string_lossy().into_owned();
    let context = ExecutionContextFactory::test_context();
    let render = |path: &str| {
        make_render(context.factory())
            .root_screen(root.clone())
            .screen_path(path)
            .render_mode("qvt")
            .render()
            .unwrap_or_else(|err| panic!("{path}: {err}"))
            .body_text()
    };
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (section, whole) =
        tracing::subscriber::with_default(crate::testing::LogSubscriber(std::sync::Arc::clone(&lines)), || {
            (render("/Section.qvt"), render("/Whole.qvt"))
        });
    let section_location = dir.join("Root/Section.xml").to_string_lossy().into_owned();
    let whole_location = dir.join("Root/Whole.xml").to_string_lossy().into_owned();
    assert!(section.contains(">before<") && section.contains(">after<"), "{section}");
    assert!(
        section.contains(&format!(
            "Error rendering section [Broken] in screen [{section_location}]"
        )),
        "{section}"
    );
    assert!(!section.contains("inside"), "{section}");
    assert!(
        whole.contains(&format!("Error rendering screen [{whole_location}]")),
        "{whole}"
    );
    assert!(!whole.contains("first") && !whole.contains("{{"), "{whole}");
    assert!(whole.contains("a &#123;&#123; brace }}"), "{whole}");
    assert_compiles(&["section".to_string(), "whole".to_string()], &[section, whole]);
    let logged = lines.lock().unwrap().join("\n");
    assert!(
        logged.contains(&format!(
            "Error rendering section [Broken] in screen [{section_location}]"
        )),
        "{logged}"
    );
    assert!(
        logged.contains(&format!("Error rendering screen [{whole_location}]")),
        "{logged}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `WebrootVue.qvt.render.js` is the render functions of the client components as their templates are now: the file
/// is made again and compared. `UPDATE_GOLDEN=1` writes it.
#[test]
fn client_components_are_precompiled() {
    let js = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/base-component/webroot/screen/webroot");
    let read = |path: &str| std::fs::read_to_string(js.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"));
    let compiler = read("libs/vue/compiler-dom.cjs.js");
    let made = crate::screen::vue_check::precompiled_components(
        &compiler,
        &[&read("js/MoquiLib.js"), &read("js/WebrootVue.qvt.js")],
    )
    .unwrap();
    let path = js.join("js/WebrootVue.qvt.render.js");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &made).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        kept == made,
        "{} is not the render functions of the current templates: UPDATE_GOLDEN=1 cargo test -p moqui-core --lib client_components_are_precompiled",
        path.display()
    );
    // no with, no eval: the build without the compiler and a CSP without unsafe-eval can load it
    for banned in ["with (", "with(", "new Function", "eval("] {
        assert!(!made.contains(banned), "{banned}");
    }
}
