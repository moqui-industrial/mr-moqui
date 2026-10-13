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

//! The macro engine: text survives generation, a mode module can be replaced, errors say what is
//! wrong, and a section's code is made once for the life of its screen definition.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use moqui_lib::object::Value;

use super::World;
use crate::context::ExecutionContextFactory;
use crate::screen::generate::generate_unit_source;
use crate::screen::{make_render, ScreenFacade};

/// Values that break code made by pasting text into it.
const NASTY: &[&str] = &[
    "plain",
    "double \" quote",
    "single ' quote",
    "back\\slash and \\n and \\\\",
    "line one\nline two\r\nthree",
    "dollar ${danger} and $",
    "`backtick` and ${`nested`}",
    "\"; throw new Error('injected'); \"",
    "'; throw new Error('injected'); '",
    "</script><!-- and \u{2028}\u{2029} separators",
    "unicode \u{e9}\u{4e2d}\u{1f600}",
];

fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
}

/// Run a unit with stand-ins for `sri` and `ec` that record each string they are given.
fn recorded_strings(source: &str) -> Vec<String> {
    let script = format!(
        r#"(function () {{
        var recorded = [];
        var rec = function (v) {{
            if (typeof v === 'string') recorded.push(v);
            else if (v && typeof v === 'object') Object.keys(v).forEach(function (k) {{ rec(k); rec(v[k]); }});
            return v;
        }};
        var sri = new Proxy({{}}, {{ get: function (t, name) {{ return function () {{
            for (var i = 0; i < arguments.length; i++) rec(arguments[i]);
            if (name === 'getFieldOptions') return new Map();
            if (name === 'openFormList') return {{id: 0, count: 1}};
            if (name === 'formListColumnCharWidths') return arguments[0].map(function () {{ return 20; }});
            if (name === 'getActiveScreenDef') return {{getScreenName: function () {{ return 'S'; }}}};
            if (name === 'doBoundaryComments') return false;
            if (name === 'write') return undefined;
            if (name === 'makeUrlByType') return {{ url: '', getUrlWithParams: function () {{ return ''; }} }};
            return '';
        }}; }} }});
        var ec = {{ resource: {{ expand: rec, expandNoL10n: rec, expression: rec, condition: rec }},
            l10n: {{ localize: rec, format: rec, formatCurrency: rec, formatCurrencyNoSymbol: rec }} }};
        var StringUtilities = {{ encodeForXmlAttribute: function (s) {{ return s; }} }};
        var lineCharacters, pageLines, lineWrap, m, l, layoutMaster, documentTitle;
        ({source})();
        return JSON.stringify(recorded);
    }})()"#
    );
    let json = crate::script::walker_context(|ctx| {
        ctx.eval::<String, _>(script.as_str())
            .map_err(|err| crate::context::Error::new(format!("{err}\n{script}")))
    })
    .unwrap();
    match crate::service::parse_json(&json).unwrap() {
        Value::List(items) => items.iter().map(moqui_lib::object::to_plain_string).collect(),
        other => panic!("{other:?}"),
    }
}

/// Each template puts `{}` where a value goes and lists the modes that must write that value out.
const TEMPLATES: &[(&str, &[&str])] = &[
    (r#"<label text="{}"/>"#, &["text", "csv", "html-print"]),
    (r#"<container-dialog button-text="{}"/>"#, &["text"]),
    (r#"<image alt="{}"/>"#, &["text", "html-print"]),
    (r#"<section name="{}"/>"#, &["text", "csv", "xml", "html-print"]),
    (
        r#"<section-iterate name="{}" list="rows" entry="row"/>"#,
        &["text", "csv", "xml", "html-print"],
    ),
    (
        r#"<section-include name="{}" location="{}"/>"#,
        &["text", "csv", "html-print"],
    ),
    (
        r#"<include-screen location="{}" share-scope="{}"/>"#,
        &["text", "csv", "xml", "html-print"],
    ),
    (
        r#"<text location="{}" template="{}"/>"#,
        &["text", "csv", "xml", "html-print"],
    ),
    ("<text>{}</text>", &["text", "csv", "xml", "html-print"]),
    (
        r#"<form-single name="F"><field name="f"><default-field title="{}"><text-line/></default-field></field></form-single>"#,
        &["text", "html-print"],
    ),
    (
        r#"<form-list name="{}" list="rows"><field name="f"><default-field title="{}"><display text="{}"/></default-field></field></form-list>"#,
        &["text", "csv", "xml", "html-print"],
    ),
    (
        r#"<form-list name="L" list="rows"><field name="f"><default-field><link link-type="anchor" text="{}" url="u"/></default-field></field></form-list>"#,
        &["text", "csv", "xml"],
    ),
    (
        r#"<form-single name="F"><field name="f"><default-field><drop-down no-current-selected-key="{}"><option key="{}" text="{}"/></drop-down></default-field></field></form-single>"#,
        &["text"],
    ),
    (
        r#"<form-single name="F"><field name="f"><default-field><display currency-unit-field="'USD'" text="{}"/></default-field></field></form-single>"#,
        &["text"],
    ),
];

#[test]
fn text_with_quotes_backslashes_newlines_and_dollar_braces_survives_every_macro() {
    for mode in ["text", "csv", "xml", "html-print"] {
        for (template, shown_in) in TEMPLATES {
            for nasty in NASTY {
                let element = template.replace("{}", &xml_text(nasty));
                let xml = format!("<widgets>{element}</widgets>");
                let source = generate_unit_source(mode, &xml).unwrap_or_else(|err| panic!("{mode} {element}: {err}"));
                let recorded = recorded_strings(&source);
                if shown_in.contains(&mode) {
                    assert!(
                        recorded.iter().any(|text| text.contains(nasty)),
                        "{mode} {element}: {nasty:?} not in {recorded:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_mode_module_at_a_real_file_replaces_the_built_in_one() {
    let dir = std::env::temp_dir().join(format!("moqui-macro-module-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let module = dir.join("ThemeText.js");
    std::fs::write(
        &module,
        r#"(function (T, S) {
            const macros = Object.assign({}, T.noops(S.allElements), {
                "screen": T.stmt((node, children) => children),
                "widgets": T.stmt((node, children) => children),
                "label": T.stmt((node) => `sri.write("themed " + ec.resource.expand(${T.jsString(node.attrs.text)}));`)
            });
            return { macros, opaque: S.opaqueElements, prelude: "" };
        })"#,
    )
    .unwrap();
    let conf = format!(
        r#"<moqui-conf><screen-facade><screen-text-output type="text" mime-type="text/plain" macro-template-location="{}"/></screen-facade></moqui-conf>"#,
        module.display()
    );
    let world = World::open(&conf);
    let context = world.factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let output = make_render(&world.factory)
        .root_screen(format!(
            "{}/src/screen/testdata/Render/Render.xml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .screen_path("/Labels.text")
        .render()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    // only the label macro writes; Render's subscreens-active is a no-op in the replacement module
    assert_eq!(output.body_text(), "");
}

#[test]
fn an_unknown_element_a_bad_expression_and_a_missing_module_are_clear_errors() {
    let dir = std::env::temp_dir().join(format!("moqui-macro-errors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, widgets: &str| {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(r#"<screen require-authentication="false"><widgets>{widgets}</widgets></screen>"#),
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    };
    let world = World::open("");
    let context = world.factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let render = |location: String, mode: &str| {
        make_render(&world.factory)
            .root_screen(location)
            .render_mode(mode)
            .render()
    };

    let unknown = render(write("Unknown.xml", "<label text=\"a\"/><no-such-widget/>"), "text").unwrap_err();
    assert!(
        unknown
            .message()
            .contains("no screen macro in text, element for no-such-widget"),
        "{unknown}"
    );
    assert!(unknown.message().contains("Unknown.xml"), "{unknown}");

    let bad = render(
        write(
            "Bad.xml",
            "<form-single name=\"F\"><field name=\"f\"><conditional-field condition=\"a b\"><display/></conditional-field>\
             <default-field><display/></default-field></field></form-single>",
        ),
        "text",
    )
    .unwrap_err();
    assert!(bad.message().contains("not JavaScript"), "{bad}");

    // html has no module yet: empty widgets render nothing, others say why not
    let empty = render(write("Empty.xml", ""), "html").unwrap();
    assert!(empty.body.is_empty());
    let html = render(write("Html.xml", "<label text=\"a\"/>"), "html").unwrap_err();
    assert!(html.message().contains("no macro module ScreenHtml.js"), "{html}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_section_is_generated_once_per_mode_and_again_when_its_file_changes() {
    let world = World::open("");
    let context = world.factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let dir = std::env::temp_dir().join(format!("moqui-macro-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Cached.xml");
    std::fs::write(
        &path,
        r#"<screen require-authentication="false"><widgets><label text="one"/>
            <section name="S"><widgets><label text="two"/></widgets><fail-widgets><label text="no"/></fail-widgets></section>
        </widgets></screen>"#,
    )
    .unwrap();
    let location = path.to_string_lossy().into_owned();
    let render = |mode: &str| {
        make_render(&world.factory)
            .root_screen(location.clone())
            .render_mode(mode)
            .render()
            .unwrap()
    };
    let def = || {
        ScreenFacade::of(&world.factory)
            .screen_def(&world.factory, &location)
            .unwrap()
    };

    assert_eq!(render("text").body_text(), format!("{:<132}{:<132}", "one", "two"));
    let first = def();
    assert_eq!(first.generated.unit_count(), 2);
    for _ in 0..3 {
        render("text");
    }
    assert_eq!(first.generated.unit_count(), 2);
    assert!(Arc::ptr_eq(&first, &def()));
    // another mode makes its own units
    assert_eq!(render("csv").body_text(), "onetwo");
    assert_eq!(first.generated.unit_count(), 4);

    std::fs::write(
        &path,
        r#"<screen require-authentication="false"><widgets><label text="changed"/></widgets></screen>"#,
    )
    .unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(30)).unwrap();
    drop(file);
    assert_eq!(render("text").body_text(), format!("{:<132}", "changed"));
    let second = def();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(second.generated.unit_count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_generated_code_of_the_whole_render_is_one_function_per_section() {
    // The unit is a function expression with the mode's prelude first and statements after.
    let source = generate_unit_source("text", r#"<widgets><label text="a"/></widgets>"#).unwrap();
    assert!(
        source.starts_with("(function () {\n\"use strict\";\nlet $lineChars = 132"),
        "{source}"
    );
    assert!(
        source.ends_with("sri.write($pad(ec.resource.expand(\"a\")));\n})"),
        "{source}"
    );
}

/// Every screen file of the framework and the base components generates code in all three modes, with
/// its forms assembled: an element without a macro, an expression that is not JavaScript, or a form that
/// cannot be assembled fails here and not on the first request that reaches the screen.
#[test]
fn every_base_component_screen_generates_in_all_three_modes() {
    use crate::screen::generate::{unit_for, UnitRequest};

    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "xml") && path.to_string_lossy().contains("/screen/") {
                out.push(path);
            }
        }
    }
    // A factory that knows the components, so component:// locations resolve.
    let _env = super::Env::new();
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let factory = ExecutionContextFactory::boot(runtime).unwrap();
    factory.set_entity(crate::testing::open());
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let facade = ScreenFacade::of(&factory);
    let mut files = Vec::new();
    collect(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component"),
        &mut files,
    );
    collect(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../framework/screen"),
        &mut files,
    );
    assert!(files.len() > 100, "{}", files.len());
    let mut failures = Vec::new();
    let mut units = 0;
    for file in &files {
        let location = file.to_string_lossy().into_owned();
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        if !text.contains("<screen") {
            continue;
        }
        let def = match facade.screen_def(&factory, &location) {
            Ok(def) => def,
            Err(err) => {
                failures.push(format!("{location}: load: {}", err.message()));
                continue;
            }
        };
        let mut names = vec![String::new()];
        names.extend(def.sections.keys().filter(|name| !name.contains("${")).cloned());
        for mode in ["text", "csv", "xml", "html-print"] {
            let macro_location = format!(
                "framework/template/screen/{}",
                match mode {
                    "text" => "ScreenText.js",
                    "csv" => "ScreenCsv.js",
                    "html-print" => "ScreenHtmlPrint.js",
                    _ => "ScreenXml.js",
                }
            );
            for name in &names {
                for fail in [false, true] {
                    let request = UnitRequest {
                        factory: &factory,
                        facade: &facade,
                        def: &def,
                        section: name,
                        fail,
                        mode,
                        macro_location: &macro_location,
                    };
                    match crate::script::walker_context(|ctx| unit_for(ctx, &request)) {
                        Ok(Some(_)) => units += 1,
                        Ok(None) => {}
                        Err(err) => failures.push(format!(
                            "{location} [{mode}] {name}: {}",
                            err.message().lines().next().unwrap_or("")
                        )),
                    }
                }
            }
        }
    }
    assert!(units > 300, "{units} units");
    // Three Tools screens keep Groovy-only code (java.sql imports) that the slice 0 migration left, so they
    // do not load at all. They are listed here so they stay in view.
    let known = |failure: &String| {
        failure.contains(": load: ")
            && ["SqlRunner.xml", "SqlScriptRunner.xml", "SpeedTest.xml"]
                .iter()
                .any(|file| failure.contains(file))
    };
    let unexpected: Vec<&String> = failures.iter().filter(|failure| !known(failure)).collect();
    assert!(
        unexpected.is_empty(),
        "{} failures:\n{}",
        unexpected.len(),
        unexpected
            .iter()
            .map(|failure| failure.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(failures.len() - unexpected.len() <= 3, "{failures:?}");
}
