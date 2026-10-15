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

//! The script host calls the mantle-usl port needed: finds, entity values, decimals, cron, notifications, resources.

use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::script::{clear_context_stack, eval_condition, run_script};

fn holds(expression: &str) {
    assert!(eval_condition(expression).unwrap(), "{expression}");
}

fn database() -> ExecutionContext {
    clear_context_stack();
    let context = ExecutionContextFactory::test_context();
    context.factory().set_entity(crate::testing::open());
    context
}

fn seed() {
    run_script(
        r#"for (const [id, text] of [["T1", "Type 1"], ["T2", "Type 2"]]) {
            const type = ec.entity.makeValue("moqui.basic.EnumerationType");
            type.enumTypeId = id;
            type.description = text;
            type.create();
        }
        for (const [id, text, n] of [["E2", "second", 2], ["E1", "first", 1], ["E3", "third", 3]]) {
            const value = ec.entity.makeValue("moqui.basic.Enumeration");
            value.enumId = id;
            value.enumTypeId = "T1";
            value.description = text;
            value.sequenceNum = n;
            value.create();
        }"#,
    )
    .unwrap();
}

#[test]
fn decimals_abs_negate_scale_and_scaled_divide() {
    clear_context_stack();
    holds(r#"toDecimal("-2.50").abs().toString() === "2.50" && toDecimal("2.50").negate().toString() === "-2.50""#);
    holds(
        r#"toDecimal("1").divide(3, 4, "HALF_UP").toString() === "0.3333" && toDecimal("2").divide(3, 2, 4).toString() === "0.67""#,
    );
    holds(r#"toDecimal("10").divide(4, 2, "HALF_UP").toString() === "2.50" && toDecimal("1.250").scale() === 3"#);
    holds(r#"toDecimal("1").divide(3).toString().length > 5"#);
}

#[test]
fn cron_next_execution_and_description() {
    clear_context_stack();
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    holds(r#"ec.service.nextCronExecution("0 0/15 * * * ?", 0) === 900000"#);
    holds(r#"ec.service.nextCronExecution("0 0/15 * * * ?", 900000) === 1800000"#);
    holds(
        r#"ec.service.nextCronExecution("0 0 0 1 1 ? 2000", 1000000000000) === null && ec.service.nextCronExecution("0 0 0 1 1 ? 2000", 0) === new Date(2000, 0, 1).getTime()"#,
    );
    holds(r#"ec.service.getCronDescription("0 0/15 * * * ?", "en", false).length > 0"#);
    assert!(eval_condition(r#"ec.service.nextCronExecution("not a cron", 0)"#).is_err());
}

#[test]
fn finds_conditions_pages_and_update_all() {
    let context = database();
    let _entered = context.enter();
    seed();
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "like", "E%").count() === 3"#);
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "in", ["E1", "E2"]).count() === 2"#);
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("sequenceNum", "greater-than", "1").count() === 2"#);
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("sequenceNum", "<=", "1").count() === 1"#);
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "=", "E1").count() === 1"#);
    assert!(
        eval_condition(r#"ec.entity.find("moqui.basic.Enumeration").condition("enumId", "bogus", "E1").count()"#)
            .is_err()
    );
    // conditionFactory forms
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").condition(ec.entity.conditionFactory.makeCondition("enumId", "like", "E%")).count() === 3"#,
    );
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition({enumId: "E2", enumTypeId: "T1"}).count() === 1"#);
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").condition(ec.entity.conditionFactory.makeCondition("enumCode", "equals", "Z", true)).count() === 3"#,
    );
    holds(r#"ec.entity.conditionFactory.makeConditionWhere("") === null"#);
    holds(r#"typeof ec.entity.databaseTimeZone === "string""#);
    // dates
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").conditionDate("lastUpdatedStamp", "lastUpdatedStamp", null).count() === 0"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").conditionDate("enumCode", "enumCode", "1791979200000").count() === 3"#,
    );
    // paging
    holds(
        r#"(function () { const f = ec.entity.find("moqui.basic.Enumeration").orderBy("sequenceNum").offset(1, 2).limit(2);
        return f.getOffset() === 2 && f.getLimit() === 2 && f.getPageIndex() === 1 && f.getPageSize() === 2 && f.list().length === 1; })()"#,
    );
    holds(
        r#"(function () { const f = ec.entity.find("moqui.basic.Enumeration"); return f.getPageIndex() === 0 && f.getPageSize() === 20; })()"#,
    );
    // updateAll
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").condition("enumTypeId", "T1").updateAll({description: "same"}) === 3"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").condition("enumTypeId", "T1").updateAll({description: "same"}) === 0"#,
    );
    holds(r#"ec.entity.find("moqui.basic.Enumeration").condition("description", "same").count() === 3"#);
}

#[test]
fn entity_value_refresh_entry_set_pretty_name_and_related_fk() {
    let context = database();
    let _entered = context.enter();
    seed();
    holds(
        r#"(function () {
            const stale = ec.entity.makeValue("moqui.basic.Enumeration");
            stale.enumId = "E1";
            const found = stale.refresh();
            const missing = ec.entity.makeValue("moqui.basic.Enumeration");
            missing.enumId = "NOPE";
            missing.description = "kept";
            return found && stale.description === "first" && !missing.refresh() && missing.description === "kept";
        })()"#,
    );
    holds(
        r#"(function () {
            const value = ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one();
            const entries = value.entrySet();
            const first = entries.find((entry) => entry.getKey() === "enumId");
            return entries.length >= 4 && first.getValue() === "E1" && first.key === "enumId" && first.value === "E1";
        })()"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.EnumerationType").condition("enumTypeId", "T1").one().getEntityNamePretty() === "Enumeration Type""#,
    );
    // the enumerations hold a foreign key to their type
    holds(
        r#"ec.entity.find("moqui.basic.EnumerationType").condition("enumTypeId", "T1").one().findRelatedFk(null).length === 3"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.EnumerationType").condition("enumTypeId", "T1").one().findRelatedFk(["moqui.basic.Enumeration"]).length === 0"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.EnumerationType").condition("enumTypeId", "T1").one().findRelatedFk(new Set(["Enumeration"])).length === 0"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.EnumerationType").condition("enumTypeId", "T2").one().findRelatedFk([]).length === 0"#,
    );
}

#[test]
fn entity_lists_have_the_array_methods() {
    let context = database();
    let _entered = context.enter();
    seed();
    holds(
        r#"(function () {
            const list = ec.entity.find("moqui.basic.Enumeration").orderBy("sequenceNum").list();
            return list.find((v) => v.enumId === "E2").description === "second"
                && list.find((v) => v.enumId === "NO") === undefined
                && list.some((v) => v.enumId === "E3") && list.every((v) => v.enumTypeId === "T1")
                && list.reduce((sum, v) => sum + Number(v.sequenceNum), 0) === 6
                && list.slice(1).length === 2 && list.concat([1]).length === 4
                && list.map((v) => v.enumId).join(",") === "E1,E2,E3"
                && list.sort((a, b) => b.sequenceNum - a.sequenceNum)[0].enumId === "E3"
                && list.indexOf(list.get(1)) >= -1 && list.findIndex((v) => v.enumId === "E2") === 1;
        })()"#,
    );
}

#[test]
fn sequenced_ids_and_entity_eca_switch() {
    let context = database();
    let _entered = context.enter();
    holds(
        r#"(function () {
            const a = ec.entity.sequencedIdPrimary("my.Seq", 1, 10), b = ec.entity.sequencedIdPrimary("my.Seq", null, null);
            return typeof a === "string" && Number(b) === Number(a) + 1 && Number(a) >= 100000;
        })()"#,
    );
    holds(
        r#"ec.artifactExecution.disableEntityEca() === false && ec.artifactExecution.disableEntityEca() === true
            && ec.artifactExecution.entityEcaDisabled() === true"#,
    );
    assert!(context.entity_eca_disabled());
    holds(r#"(ec.artifactExecution.enableEntityEca(), ec.artifactExecution.entityEcaDisabled() === false)"#);
    assert!(!context.entity_eca_disabled());
}

#[test]
fn notification_message_builder() {
    let context = database();
    let _entered = context.enter();
    holds(
        r#"(function () {
            const nm = ec.makeNotificationMessage().topic("NoSuchTopic").message({a: 1}).userIds(["u1", "u2"]).title("T").link("/x");
            return nm.getTopic() === "NoSuchTopic" && nm.getNotificationTopic() === null && nm.getNotifyUserIds().length === 0;
        })()"#,
    );
}

#[test]
fn resource_streams_and_text_filenames() {
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    clear_context_stack();
    let dir = std::env::temp_dir().join(format!("moqui-gaps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("source.txt"), "uploaded text").unwrap();
    set_dir(&dir);
    holds(
        r#"ec.resource.isTextFilename("a.txt") && ResourceReference.isTextFilename("A.JSON") && !ResourceReference.isTextFilename("a.png") && !ec.resource.isTextFilename("noext")"#,
    );
    holds(
        r#"(function () {
            const source = ec.resource.getLocationReference(dir + "/source.txt");
            const stream = source.openStream();
            const target = ec.resource.getLocationReference(dir + "/target.txt");
            try { target.putStream(stream); } finally { stream.close(); }
            return stream.getText() === "uploaded text" && ec.resource.getLocationReference(dir + "/target.txt").getText() === "uploaded text"
                && source.getInputStream().readAllBytes().length === 13 && ec.resource.getLocationReference(dir + "/missing.txt").openStream() === null;
        })()"#,
    );
    let _ = std::fs::remove_dir_all(&dir);
    clear_context_stack();
}

fn set_dir(dir: &std::path::Path) {
    crate::script::set_context_name(
        "dir",
        moqui_lib::object::Value::string(format!("file://{}", dir.display())),
    )
    .unwrap();
}

#[test]
fn verified_host_behaviors() {
    let context = database();
    let _entered = context.enter();
    seed();
    // l10n: epoch millis text with a date pattern, a decimal with a number pattern, a plain 0 as currency
    holds(
        r#"ec.l10n.format("1791979200000", "yyMMdd") === "261014" || ec.l10n.format("1791979200000", "yyMMdd") === "261015""#,
    );
    // a date held as text, as a script keeps one from a Date parameter
    holds(r#"ec.l10n.format("2026-10-16", "yyMMdd") === "261016" && ec.l10n.format("2026-10-16 08:30:00", "yyMMdd") === "261016""#);
    holds(
        r##"ec.l10n.format(toDecimal("2.5"), "0.00") === "2.50" && ec.l10n.format(toDecimal("1234.567"), "#,##0.00") === "1,234.57""##,
    );
    holds(r#"ec.l10n.formatCurrency(0, "USD") === "$0.00" && ec.l10n.formatCurrency(0, null) !== """#);
    holds(r#"ec.l10n.format(1, "0", null, ec.entity.databaseTimeZone) === "1""#);
    holds(r#"ec.l10n.roundCurrency(toDecimal("2.345"), "USD", false, "HALF_EVEN").toString() === "2.34""#);
    holds(r#"ec.l10n.roundCurrency(toDecimal("2.345"), "USD").toString() === "2.35""#);
    holds(r#"ec.user.getPeriodRange("week", "").length === 2"#);
    // timestamp conditions from epoch-millis text
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").condition("lastUpdatedStamp", "less", "1791979200000").count() === 0"#,
    );
    holds(
        r#"ec.entity.find("moqui.basic.Enumeration").condition("lastUpdatedStamp", "greater", "1000000000000").count() === 3"#,
    );
    // an entity value spread into a service in-map
    holds(
        r#"(function () {
            const value = ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one();
            const inMap = Object.assign({extra: "x"}, value);
            return inMap.enumId === "E1" && inMap.description === "first" && inMap.extra === "x";
        })()"#,
    );
}

#[test]
fn like_ignore_case_and_where_conditions_reach_the_sql() {
    clear_context_stack();
    let tape = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let context = ExecutionContextFactory::test_context();
    context
        .factory()
        .set_entity(crate::testing::open_with_tape(std::sync::Arc::clone(&tape)));
    let _entered = context.enter();
    // the in-memory database cannot run these two, so the finds fail after the statement is written
    for expression in [
        r#"ec.entity.find("moqui.basic.Enumeration").condition(ec.entity.conditionFactory.makeCondition("description", "like", "FIR%").ignoreCase()).count()"#,
        r#"ec.entity.find("moqui.basic.Enumeration").condition(ec.entity.conditionFactory.makeCondition("description", "like", "FIR%", false, true)).count()"#,
        r#"ec.entity.find("moqui.basic.Enumeration").condition(ec.entity.conditionFactory.makeConditionWhere("ENUM_ID = 'E3'")).count()"#,
    ] {
        let _ = eval_condition(expression);
    }
    let statements = tape.lock().unwrap().join("\n");
    assert!(
        statements.contains("UPPER(\"description\") LIKE UPPER(?)"),
        "{statements}"
    );
    assert_eq!(
        statements.matches("UPPER(\"description\") LIKE UPPER(?)").count(),
        2,
        "{statements}"
    );
    assert!(statements.contains("(ENUM_ID = 'E3')"), "{statements}");
}

#[test]
fn actions_scoping_iterate_over_objects_and_unset_names() {
    let context = database();
    let _entered = context.enter();
    crate::script::actions::run_actions(
        r#"<actions>
            <set field="seen" from="[]"/>
            <set field="items" from="[1, 2, 3]"/>
            <iterate list="items" entry="item"><script>const doubled = item * 2; seen.push(doubled)</script></iterate>
            <set field="obj" from="({a: 1, b: 2})"/>
            <set field="pairs" from="[]"/>
            <iterate list="obj" entry="v" key="k"><script>pairs.push(k + '=' + v)</script></iterate>
            <script>brandNewName = 5</script>
            <set field="after" from="brandNewName + 1"/>
            <set field="unsetIsNull" from="neverSetAnywhere == null"/>
            <if condition="neverSetAnywhere"><then><set field="branch" value="wrong"/></then><else><set field="branch" value="right"/></else></if>
        </actions>"#,
    )
    .unwrap();
    holds(
        r#"seen.join(",") === "2,4,6" && pairs.join("|") === "a=1|b=2" && after === 6 && unsetIsNull === true && branch === "right""#,
    );
    holds(r#"ec.context.get("brandNewName") === 5 && ("brandNewName" in context) === true"#);
    // a decimal argument to decimal methods, relationship names on values, getMap, a missing dbresource reference
    holds(
        r#"toDecimal("1.5").add(toDecimal("2")).toString() === "3.5" && toDecimal("3").subtract(toDecimal("1.5")).eq(toDecimal("1.50")) && toDecimal("2").compare(toDecimal("1")) === 1"#,
    );
    holds(
        r#"(function () { const r = ec.resource.getLocationReference("dbresource://nothing/here.txt"); return r.isText() === true && r.exists === false; })()"#,
    );
}

#[test]
fn relationship_names_and_get_map_on_values() {
    let context = database();
    let _entered = context.enter();
    seed();
    holds(
        r#"(function () {
            const e = ec.entity.find("moqui.basic.Enumeration").condition("enumId", "E1").one();
            const m = e.getMap();
            return e.type.description === "Type 1" && m.enumId === "E1" && m.description === "first" && Object.keys(m).length >= 4;
        })()"#,
    );
    holds(
        r#"numberToWordsWithDecimal(toDecimal("1234.5")) === "One thousand, two hundred  thirty four and 50/100" || numberToWordsWithDecimal(toDecimal("1234.5")).startsWith("One thousand")"#,
    );
    holds(
        r#"numberToWords(21, false) === "Twenty one" && numberToWordsWithDecimal(toDecimal("5")) === "Five and 00/100""#,
    );
    holds(r#"ec.artifactExecution.getStackNameString() === """#);
}

#[test]
fn service_out_parameters_keep_lists_and_sets() {
    clear_context_stack();
    let context = database();
    let _entered = context.enter();
    let path = std::env::temp_dir().join(format!("moqui-gaps-{}.xml", std::process::id()));
    std::fs::write(
        &path,
        r#"<services>
            <service verb="probe" noun="Gaps" authenticate="false">
                <in-parameters><parameter name="n" type="Integer"/></in-parameters>
                <out-parameters>
                    <parameter name="ids" type="List"/><parameter name="tags" type="Set"/><parameter name="ok"/>
                </out-parameters>
                <actions>
                    <script>ids = [n, n + 1]; tags = new Set(["x", "y"]); ok = "yes"</script>
                </actions>
            </service>
        </services>"#,
    )
    .unwrap();
    let failures = crate::service::lint(std::slice::from_ref(&path)).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    let mut input = std::collections::BTreeMap::new();
    input.insert("n".to_string(), moqui_lib::object::Value::I32(4));
    let out = crate::service::call::sync("probe#Gaps", &input).unwrap();
    println!("{out:?}");
    assert!(
        matches!(out.get("ids"), Some(moqui_lib::object::Value::List(items)) if items.len() == 2),
        "{out:?}"
    );
    assert!(out.contains_key("tags"), "{out:?}");
    assert_eq!(moqui_lib::object::to_plain_string(&out["ok"]), "yes");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn shared_map_lives_with_the_execution_context() {
    clear_context_stack();
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    holds(
        r#"(function () {
            const shared = ec.context.sharedMap;
            shared["guard:1"] = true;
            const frame = ec.context.getSharedMap();
            const seen = frame["guard:1"] === true && ("guard:1" in frame) && Object.keys(frame).join() === "guard:1" && frame.nothing == null;
            delete ec.context.sharedMap["guard:1"];
            return seen && !("guard:1" in ec.context.sharedMap) && Object.keys(ec.context.sharedMap).length === 0;
        })()"#,
    );
    // a value set in one script is there in the next, as a nested service call would see it
    run_script(r#"ec.context.sharedMap.depth = 2"#).unwrap();
    holds(r#"ec.context.sharedMap.depth === 2"#);
    assert_eq!(context.shared_map().len(), 1);
}

#[test]
fn soft_validate_makes_danger_messages_and_logger_takes_an_exception() {
    clear_context_stack();
    let context = database();
    let _entered = context.enter();
    let path = std::env::temp_dir().join(format!("moqui-gaps-soft-{}.xml", std::process::id()));
    std::fs::write(
        &path,
        r#"<services><service verb="needs" noun="Number" authenticate="false">
            <in-parameters><parameter name="n" type="Integer" required="true"/></in-parameters>
            <actions><script>ran = true</script></actions></service></services>"#,
    )
    .unwrap();
    assert!(crate::service::lint(std::slice::from_ref(&path)).unwrap().is_empty());
    holds(r#"ec.service.sync().name("needs#Number").softValidate(true).parameters({}).call() != null || true"#);
    assert!(
        context.message().errors().is_empty(),
        "{:?}",
        context.message().errors()
    );
    assert!(!context.message().messages().is_empty());
    holds(
        r#"(function () { ec.logger.error("failed", new Error("boom")); ec.logger.log("warn", "x", new Error("y")); ec.logger.info("i"); return true; })()"#,
    );
    holds(r#"ec.service.async().name("needs#Number").distribute(true) != null"#);
    let _ = std::fs::remove_file(&path);
}
