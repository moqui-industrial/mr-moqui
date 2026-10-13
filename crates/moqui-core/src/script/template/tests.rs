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

//! Golden tests for the fancy dialect. A fixture `<name>.fancy` has its data in `<name>.json`, its
//! generated statements in `<name>.js` and its output in `<name>.out` (`<name>.html.out` with HTML
//! escaping). Both renderers must give the output. `UPDATE_GOLDEN=1` rewrites the expected files.

use std::path::PathBuf;

use moqui_lib::object::Value;

use super::*;

fn testdata() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/script/testdata/template")
}

fn expect(file: &str, actual: &str) {
    let path = testdata().join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {}", path.display()));
    assert_eq!(actual, expected, "{file}");
}

fn data(name: &str) -> Value {
    let text = std::fs::read_to_string(testdata().join(format!("{name}.json"))).unwrap();
    moqui_lib::json::from_json(&serde_json::from_str(&text).unwrap())
}

fn check(name: &str, escape: Escape, out_file: &str) {
    let source = std::fs::read_to_string(testdata().join(format!("{name}.fancy"))).unwrap();
    if escape == Escape::None {
        expect(&format!("{name}.js"), &compile_body(&source, escape).unwrap());
    }
    let values = data(name);
    let trusted = FancyRenderer { escape }.render(&source, Some(&values)).unwrap();
    let locked = LockedRenderer { escape }.render(&source, Some(&values)).unwrap();
    assert_eq!(trusted, locked, "{name}: trusted and locked-down differ");
    expect(out_file, &trusted);
}

#[test]
fn golden_if() {
    check("if", Escape::None, "if.out");
}

#[test]
fn golden_list() {
    check("list", Escape::None, "list.out");
}

#[test]
fn golden_assign_and_comments() {
    check("assign", Escape::None, "assign.out");
}

#[test]
fn golden_interpolation_and_escape_modes() {
    check("interp", Escape::None, "interp.out");
    check("interp", Escape::Html, "interp.html.out");
}

#[test]
fn compile_errors_name_the_line() {
    for (source, needle) in [
        ("a\n<#if x>\nb", "<#if> is not closed"),
        ("</#if>", "without an open"),
        ("<#else>", "without an open"),
        ("x\n<#visit node>", "line 2: unknown tag <#visit>"),
        ("<@macro/>", "macro calls"),
        ("<#list items>x</#list>", "expression as name"),
        ("<#assign x>", "name = expression"),
        ("${a b}", "not JavaScript"),
        ("${unclosed", "no closing"),
        ("<#if def x = 1>y</#if>", "not JavaScript"),
    ] {
        let err = check_template(source).expect_err(source);
        assert!(err.message().contains(needle), "{source}: {}", err.message());
    }
}

#[test]
fn tag_only_lines_leave_no_blank_lines() {
    let out = render_trusted("a\n  <#if true>\nb\n  </#if>\nc\n", Escape::None).unwrap();
    assert_eq!(out, "a\nb\nc\n");
}

#[test]
fn trusted_reads_the_live_context_and_locked_down_cannot_reach_ec() {
    let probe = "${typeof ec} ${typeof sri} ${typeof context} ${typeof globalThis.ec}";
    let trusted = render_trusted(probe, Escape::None).unwrap();
    assert!(trusted.starts_with("object object"), "{trusted}");
    let locked = render_locked_with(probe, Escape::None, &Value::Map(Default::default())).unwrap();
    // A name that is not in the snapshot is null, which is an object.
    assert_eq!(locked, "object object object undefined");
    let err = render_locked_with(
        "${ec.entity.find('Enumeration')}",
        Escape::None,
        &Value::Map(Default::default()),
    )
    .expect_err("ec is null");
    assert!(
        err.message().contains("null") || err.message().contains("not an object"),
        "{}",
        err.message()
    );
    let escape_attempt = "${[].constructor.constructor('return typeof ec')()}";
    assert_eq!(
        render_locked_with(escape_attempt, Escape::None, &Value::Map(Default::default())).unwrap(),
        "undefined"
    );
}

#[test]
fn locked_down_sees_plain_values_and_decimals_as_strings() {
    use std::str::FromStr;
    let mut entries = indexmap::IndexMap::new();
    entries.insert(
        "amount".to_string(),
        Value::Decimal(moqui_lib::format::Decimal::from_str("10.50").unwrap()),
    );
    entries.insert("count".to_string(), Value::I64(3));
    let data = Value::Map(entries);
    let out = render_locked_with(
        "${typeof amount} ${amount} ${amount < '9'} ${typeof count} ${count + 1}",
        Escape::None,
        &data,
    )
    .unwrap();
    assert_eq!(out, "string 10.50 true number 4");
}

#[test]
fn locked_down_format_and_localize_are_plain_functions() {
    let data = moqui_lib::json::from_json(&serde_json::json!({"n": 1234.5, "when": "2024-03-05 10:20:30.0"}));
    let out = render_locked_with(
        "${format(n, '#,##0.00')} ${format(when, 'yyyy-MM-dd')} ${localize('Hello')}",
        Escape::None,
        &data,
    )
    .unwrap();
    assert_eq!(out, "1,234.50 2024-03-05 Hello");
}

#[test]
fn locked_down_stops_a_runaway_loop() {
    let started = std::time::Instant::now();
    let err = render_locked_with(
        "${(() => { while (true) {} })()}",
        Escape::None,
        &Value::Map(Default::default()),
    )
    .expect_err("interrupted");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "{}",
        err.message()
    );
    assert!(err.message().contains("interrupted"), "{}", err.message());
}

#[test]
fn compiled_functions_are_cached_by_source() {
    let source = "cache probe ${1 + 1}";
    let first = compiled(source, Escape::None, Scope::Live).unwrap();
    let second = compiled(source, Escape::None, Scope::Live).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let html = compiled(source, Escape::Html, Scope::Live).unwrap();
    assert!(!Arc::ptr_eq(&first, &html));
}

#[test]
fn registry_picks_the_longest_extension() {
    let registry = TemplateRegistry::parse(
        r#"<moqui-conf><resource-facade>
        <template-renderer extension="fancy" class="fancy"/>
        <template-renderer extension="html.fancy" class="fancy-html"/>
        <template-renderer extension="ftl" class="org.moqui.impl.context.renderer.FtlTemplateRenderer"/>
        </resource-facade></moqui-conf>"#,
    );
    assert!(registry.renderer_for("component://x/a.fancy").is_some());
    assert!(registry.renderer_for("component://x/a.html.fancy").is_some());
    assert!(registry.renderer_for("component://x/a.ftl").is_none());
    let html = registry
        .renderer_for("component://x/a.html.fancy")
        .unwrap()
        .render(
            "${v}",
            Some(&moqui_lib::json::from_json(&serde_json::json!({"v": "<"}))),
        )
        .unwrap();
    assert_eq!(html, "&lt;");
}
