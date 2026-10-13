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

//! XML actions. One cached script per action XML text. Names in that script are bare.

use std::cell::RefCell;
use std::collections::HashMap;

use moqui_lib::node::MNode;
use rquickjs::{Array, CatchResultExt, Ctx, Function, Object};

use crate::context::Error;

const TEMPLATE_SUPPORT: &str = include_str!("../../../../framework/template/TemplateSupport.js");
const XML_ACTIONS: &str = include_str!("../../../../framework/template/XmlActions.js");

/// Set this to add a comment naming the XML element before each generated statement.
const DEBUG_ENV: &str = "MOQUI_ACTIONS_DEBUG";

const MAX_CACHED_SCRIPTS: usize = 4096;

thread_local! {
    static SCRIPTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

pub fn run_actions(actions: &str) -> Result<(), Error> {
    if actions.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    let source = generated(actions)?;
    super::runtime::eval_script(&source)
}

/// Generate the script for `xml` and parse it, without running anything. Definition load calls
/// this, so an expression that is not JavaScript fails the service there.
///
/// # Errors
///
/// Returns an error when the XML does not generate, or the generated script does not parse.
pub fn check_actions(xml: &str) -> Result<(), Error> {
    if xml.trim().is_empty() {
        return Ok(());
    }
    let source = generated(xml)?;
    super::parse::check_program(&source)
}

fn generated(xml: &str) -> Result<String, Error> {
    let cached = SCRIPTS.with(|scripts| scripts.borrow().get(xml).cloned());
    if let Some(source) = cached {
        return Ok(source);
    }
    let source = generate(xml)?;
    SCRIPTS.with(|scripts| {
        let mut scripts = scripts.borrow_mut();
        // Dynamic action text would otherwise grow this without bound.
        if scripts.len() >= MAX_CACHED_SCRIPTS {
            scripts.clear();
        }
        scripts.insert(xml.to_string(), source.clone());
    });
    Ok(source)
}

fn generate(xml: &str) -> Result<String, Error> {
    generate_with(xml, std::env::var_os(DEBUG_ENV).is_some())
}

fn generate_with(xml: &str, debug: bool) -> Result<String, Error> {
    let root = MNode::parse_text("actions", xml)?;
    reject(&root)?;
    super::runtime::with_ctx(|ctx| {
        let macros = load_macros(ctx, debug)?;
        let (source, _) = render(ctx, &macros, &root)?;
        Ok(source)
    })
}

/// The macro table. The support script evaluates to an object, and the actions script
/// is a function of that object, so no names are left in the global scope.
fn load_macros<'js>(ctx: &Ctx<'js>, debug: bool) -> Result<Object<'js>, Error> {
    let support: Object = super::runtime::eval_source(ctx, TEMPLATE_SUPPORT)?;
    let options: Object = support.get("options").map_err(js_err)?;
    options.set("debug", debug).map_err(js_err)?;
    let build: Function = super::runtime::eval_source(ctx, XML_ACTIONS)?;
    build
        .call((support,))
        .catch(ctx)
        .map_err(|caught| Error::new(caught.to_string()))
}

fn reject(node: &MNode) -> Result<(), Error> {
    let name = node.name();
    if name == "entity-data" {
        return Err(Error::new("entity-data is not implemented"));
    }
    if name == "service-call" && node.attribute("async").as_deref() == Some("distribute") {
        return Err(Error::new("async=\"distribute\" is not a host method"));
    }
    for child in node.children() {
        reject(&child)?;
    }
    Ok(())
}

fn render<'js>(ctx: &Ctx<'js>, macros: &Object<'js>, node: &MNode) -> Result<(String, Object<'js>), Error> {
    let children = Array::new(ctx.clone()).map_err(js_err)?;
    let mut combined = String::new();
    for (index, child) in node.children().iter().enumerate() {
        let (source, child_obj) = render(ctx, macros, child)?;
        combined.push_str(&source);
        children.set(index, child_obj).map_err(js_err)?;
    }
    let node_obj = Object::new(ctx.clone()).map_err(js_err)?;
    node_obj.set("name", node.name()).map_err(js_err)?;
    node_obj.set("text", node.text().unwrap_or_default()).map_err(js_err)?;
    node_obj.set("attrs", attrs_object(ctx, node)?).map_err(js_err)?;
    node_obj.set("children", children).map_err(js_err)?;
    let name = node.name();
    if !macros.contains_key(name.as_str()).map_err(js_err)? {
        return Err(Error::new(format!("no XML action macro for {name}")));
    }
    let function: Function = macros.get(name.as_str()).map_err(js_err)?;
    let source: String = function
        .call((node_obj.clone(), combined))
        .catch(ctx)
        .map_err(|caught| Error::new(caught.to_string()))?;
    node_obj.set("source", source.clone()).map_err(js_err)?;
    Ok((source, node_obj))
}

fn attrs_object<'js>(ctx: &Ctx<'js>, node: &MNode) -> Result<Object<'js>, Error> {
    let attrs = Object::new(ctx.clone()).map_err(js_err)?;
    for (key, value) in node.attributes() {
        attrs.set(key, value).map_err(js_err)?;
    }
    Ok(attrs)
}

fn js_err(err: impl ToString) -> Error {
    Error::new(err.to_string())
}

#[cfg(test)]
mod tests {

    use moqui_lib::object::Value;
    use super::{generate, generate_with, load_macros, run_actions};
    use crate::context::{Error, ExecutionContext, ExecutionContextFactory};

    use crate::entity::{EntityFacade, EntityValue};
    use crate::script::runtime::{clear_context_stack, eval_condition, set_context_name};

    const MACROS: &[&str] = &[
        "actions",
        "and",
        "assert",
        "break",
        "check-errors",
        "compare",
        "condition",
        "continue",
        "date-filter",
        "default-parameters",
        "econdition",
        "econdition-object",
        "econditions",
        "else",
        "else-if",
        "entity-create",
        "entity-data",
        "entity-delete",
        "entity-delete-by-condition",
        "entity-delete-related",
        "entity-find",
        "entity-find-count",
        "entity-find-one",
        "entity-find-related",
        "entity-find-related-one",
        "entity-make-value",
        "entity-sequenced-id-primary",
        "entity-sequenced-id-secondary",
        "entity-set",
        "entity-update",
        "expression",
        "field-map",
        "filter-map-list",
        "having-econditions",
        "if",
        "iterate",
        "limit-range",
        "limit-view",
        "log",
        "message",
        "not",
        "or",
        "order-by",
        "order-map-list",
        "return",
        "script",
        "search-form-inputs",
        "select-field",
        "service-call",
        "set",
        "then",
        "use-iterator",
        "while",
    ];

    const INLINE: &str = r#"<actions>
    <entity-find entity-name="moqui.test.TestEntity" list="found">
        <econdition field-name="testId" from="seedId"/>
    </entity-find>
    <entity-find-one entity-name="moqui.test.TestEntity" value-field="oneRow">
        <field-map field-name="testId" from="seedId"/>
    </entity-find-one>
    <script>foundCount = found.length; foundId = found.get(0).get("testId"); oneId = oneRow.get("testId");</script>
    <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
    <script>made.set("testId", "B"); made.set("testMedium", "made");</script>
    <entity-create value-field="made"/>
</actions>"#;

    fn assert_golden(xml: &str, expected: &str) {
        assert_eq!(generate(xml).unwrap(), expected);
        assert_eq!(generate(xml).unwrap(), expected);
    }

    #[test]
    fn list_helpers_and_the_condition_factory_run() {
        clear_context_stack();
        run_actions(
            r#"<actions>
                <set field="items" from="[{n: 'b', q: 2}, {n: 'c', q: 2}, {n: 'a', q: 1}]"/>
                <order-map-list list="items"><order-by field-name="n"/></order-map-list>
                <filter-map-list list="items"><field-map field-name="q" value="2"/></filter-map-list>
                <set field="names" from="items.map(function (item) { return item.n; }).join(',')"/>
                <set field="seesNames" from="context.names"/>
                <set field="made" from="ec.entity.conditionFactory.makeCondition('a', 'greater', 3) != null"/>
                <set field="skipped" from="ec.entity.conditionFactory.makeActionConditionDirect('a', 'equals', null, null, null, false, true, false, 'false') === null"/>
                <set field="dated" from="ec.entity.conditionFactory.makeConditionDate('fromDate', 'thruDate', null, false, 'false') != null"/>
            </actions>"#,
        )
        .unwrap();
        let text =
            |name: &str| crate::script::context_value(name).map(|value| moqui_lib::object::to_plain_string(&value));
        assert_eq!(text("names").as_deref(), Some("b,c"));
        assert_eq!(text("seesNames").as_deref(), Some("b,c"));
        assert_eq!(text("made").as_deref(), Some("true"));
        assert_eq!(text("skipped").as_deref(), Some("true"));
        assert_eq!(text("dated").as_deref(), Some("true"));
    }

    #[test]
    fn golden_service_call() {
        assert_golden(
            include_str!("testdata/service-call.xml"),
            include_str!("testdata/service-call.js"),
        );
    }

    #[test]
    fn golden_script() {
        assert_golden(include_str!("testdata/script.xml"), include_str!("testdata/script.js"));
    }

    #[test]
    fn golden_set() {
        assert_golden(include_str!("testdata/set.xml"), include_str!("testdata/set.js"));
    }

    #[test]
    fn golden_entity_find() {
        assert_golden(
            include_str!("testdata/entity-find.xml"),
            include_str!("testdata/entity-find.js"),
        );
    }

    #[test]
    fn golden_entity_value() {
        assert_golden(
            include_str!("testdata/entity-value.xml"),
            include_str!("testdata/entity-value.js"),
        );
    }

    #[test]
    fn golden_control() {
        assert_golden(
            include_str!("testdata/control.xml"),
            include_str!("testdata/control.js"),
        );
    }

    #[test]
    fn golden_message() {
        assert_golden(
            include_str!("testdata/message.xml"),
            include_str!("testdata/message.js"),
        );
    }

    #[test]
    fn golden_condition() {
        assert_golden(
            include_str!("testdata/condition.xml"),
            include_str!("testdata/condition.js"),
        );
    }

    #[test]
    fn nested_blocks_indent_but_inline_script_text_is_unchanged() {
        let source = generate(
            "<actions><if condition=\"a\"><script>var s = `one\ntwo`;</script></if></actions>",
        )
        .unwrap();
        assert!(source.contains("        // begin inline script\n        var s = `one\ntwo`;\n        // end inline script\n"));
        assert!(!source.contains('\u{1}') && !source.contains('\u{2}'));
    }

    #[test]
    fn debug_adds_a_comment_naming_each_element() {
        let source = generate_with("<actions><set field=\"a\" value=\"b\"/></actions>", true).unwrap();
        assert!(source.contains("    // <set field=\"a\" value=\"b\">\n    a = ec.resource.expand(\"b\");\n"));
        let plain = generate_with("<actions><set field=\"a\" value=\"b\"/></actions>", false).unwrap();
        assert!(!plain.contains("//"));
    }

    #[test]
    fn a_bare_expression_statement_ends_before_the_next_statement() {
        let source = generate(
            "<actions><expression>foo()</expression><if condition=\"a\"><expression>bar()</expression></if></actions>",
        )
        .unwrap();
        assert!(source.contains("    foo();\n    if (a) {\n        bar();\n    }\n"), "{source}");
    }

    #[test]
    fn a_missing_macro_names_the_element() {
        let err = generate("<actions><no-such-action/></actions>").unwrap_err();
        assert!(err.to_string().contains("no-such-action"));
    }

    #[test]
    fn each_concrete_element_has_a_macro() {
        let names = macro_names().unwrap();
        assert_eq!(names, MACROS);
        assert!(!names
            .iter()
            .any(|name| *name == "service-group" || *name == "service-invoke"));
    }

    #[test]
    fn empty_actions_are_ok() {
        run_actions("<actions/>").unwrap();
    }

    #[test]
    fn distribute_is_a_load_error() {
        let err =
            run_actions(r#"<actions><service-call name="example.do#It" async="distribute"/></actions>"#).unwrap_err();
        assert!(err.to_string().contains("distribute"), "{err}");
    }

    #[test]
    fn entity_data_is_not_implemented() {
        let err = run_actions("<actions><entity-data/></actions>").unwrap_err();
        assert!(err.to_string().contains("entity-data"), "{err}");
    }

    #[test]
    fn call_future_returns_a_future_the_script_can_wait_on() {
        let path = std::env::temp_dir().join(format!("moqui-script-future-{}.xml", std::process::id()));
        std::fs::write(
            &path,
            r#"<services>
                <service verb="echo" noun="Note" authenticate="false" transaction="ignore">
                    <in-parameters><parameter name="note"/></in-parameters>
                    <out-parameters><parameter name="note"/></out-parameters>
                </service>
            </services>"#,
        )
        .unwrap();
        let ctx =
            crate::context::ExecutionContextFactory::test_context_with("", crate::service::ServicePools::bounded(2, 8));
        ctx.factory().set_entity(crate::testing::open());
        let _entered = ctx.enter();
        let failures = crate::service::lint(std::slice::from_ref(&path)).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        let done = eval_condition(
            "var f = ec.service.async().name('echo#Note').parameter('note', 'hi').callFuture(); \
             var out = f.get(5000); out.note === 'hi' && f.isDone() && f.get().note === 'hi'",
        )
        .unwrap();
        assert!(done);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn service_async_has_no_distribute_method() {
        clear_context_stack();
        let present =
            eval_condition("typeof ec.service.async === \"function\" && ec.service.async().distribute == null")
                .unwrap();
        assert!(present);
    }

    #[test]
    fn entity_find_without_an_execution_context_errors() {
        clear_context_stack();
        let err = run_actions(
            r#"<actions><entity-find entity-name="moqui.test.TestEntity" list="found"><econdition field-name="testId" from="seedId"/></entity-find></actions>"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("no execution context is entered"), "{err}");
    }

    #[test]
    fn message_expands_and_records_an_error() {
        clear_context_stack();
        let ctx = ExecutionContextFactory::test_context();
        let _guard = ctx.enter();
        set_context_name("seedId", Value::string("A")).unwrap();
        run_actions(r#"<actions><message error="true">failed ${seedId}</message></actions>"#).unwrap();
        let errors = ExecutionContext::with_current(|ec| ec.message().errors().to_vec()).unwrap();
        assert_eq!(errors, vec!["failed A".to_string()]);
        clear_context_stack();
    }

    #[test]
    fn inline_action_finds_and_creates() -> Result<(), Error> {
        clear_context_stack();
        let facade = open_facade();
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(facade.clone());
        let _guard = ctx.enter();
        let mut seed = facade.make_value("moqui.test.TestEntity").expect("make seed");
        seed.set("testId", Value::string("A"));
        seed.set("testMedium", Value::string("seed"));
        seed.create(&facade).expect("create seed");
        set_context_name("seedId", Value::string("A"))?;
        run_actions(INLINE)?;
        assert!(eval_condition("foundCount === 1")?, "foundCount");
        assert!(eval_condition("foundId === \"A\"")?, "foundId");
        assert!(eval_condition("oneId === \"A\"")?, "oneId");
        let rows = facade
            .find("moqui.test.TestEntity")
            .expect("find")
            .list()
            .expect("list");
        let mut ids = Vec::new();
        for row in rows.iter() {
            ids.push(field_str(row, "testId"));
        }
        ids.sort();
        assert_eq!(ids, vec!["A".to_string(), "B".to_string()]);
        let made = rows.iter().find(|row| field_str(row, "testId") == "B").expect("row B");
        assert_eq!(field_str(made, "testMedium"), "made");
        clear_context_stack();
        Ok(())
    }

    fn macro_names() -> Result<Vec<String>, Error> {
        crate::script::runtime::with_ctx(|ctx| {
            let macros = load_macros(ctx, false)?;
            let mut names = Vec::new();
            for key in macros.keys::<String>() {
                names.push(key.map_err(|err| Error::new(err.to_string()))?);
            }
            names.sort();
            Ok(names)
        })
    }

    fn field_str(row: &EntityValue, name: &str) -> String {
        row.get(name).and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn open_facade() -> EntityFacade {
        crate::testing::open()
    }
}
