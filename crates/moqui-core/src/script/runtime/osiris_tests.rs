//! Regression coverage for the metadata and graph contracts used by OSIRIS.
use super::*;
use crate::context::ExecutionContextFactory;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "moqui-osiris-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("entity")).unwrap();
        std::fs::create_dir_all(dir.join("conf")).unwrap();
        std::fs::create_dir_all(dir.join("service")).unwrap();
        std::fs::copy(
            crate::testing::framework_dir().join("conf/MoquiDefaultConf.xml"),
            dir.join("conf/MoquiDefaultConf.xml"),
        )
        .unwrap();
        std::fs::write(
            dir.join("entity/Graph.xml"),
            include_str!("../testdata/osiris-graph.xml"),
        )
        .unwrap();
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn holds(source: &str) {
    assert!(eval_condition(source).unwrap(), "{source}");
}

#[test]
fn master_graph_round_trip_inherits_keys_and_respects_read_only_relationships() {
    clear_context_stack();
    let fixture = Fixture::new();
    let context = ExecutionContextFactory::test_context();
    context.factory().set_entity(crate::testing::open_at(&fixture.0));
    let _entered = context.enter();
    run_script(
        r#"
        const graph = {processId:"P", text:"Actor works", phrases:[
            {phraseId:"A", startChar:0},
            {phraseId:"B", startChar:6, links:[{fromPhraseId:"A", action:"view"}]}
        ], readOnlyPhrases:[{phraseId:"must-not-write"}], metadata:{unexpected:"ignored"}};
        rows = ec.entity.getValueListFromPlainMap(graph, "test.osiris.Process");
        for (const row of rows) row.create();
        loaded = ec.entity.find("test.osiris.Process").condition("processId", "P").oneMaster("default");
        roundTrip = ec.entity.getValueListFromPlainMap(loaded, "test.osiris.Process");
    "#,
    )
    .unwrap();
    holds(r#"rows.length === 4 && roundTrip.length === 4"#);
    holds(
        r#"rows[0].getEntityName() === "test.osiris.Process" && rows[1].resolveEntityName() === "test.osiris.Phrase""#,
    );
    holds(r#"rows[3].toPhraseId === "B" && rows[3].fromPhraseId === "A""#);
    holds(r#"loaded.processId === "P" && loaded.text === "Actor works" && loaded.phrases.length === 2"#);
    holds(r#"ec.entity.find("test.osiris.Phrase").condition("phraseId", "must-not-write").one() === null"#);
    holds(r#"roundTrip.find(v => v.getEntityName() === "test.osiris.Link").toPhraseId === "B""#);
    holds(r#"ec.entity.find("test.osiris.Process").condition("processId", "absent").oneMaster() === null"#);
    assert!(eval_condition(
        r#"ec.entity.find("test.osiris.Process").condition("processId", "P").oneMaster("missing")"#
    )
    .is_err());
}

#[test]
fn error_scopes_preserve_caller_errors_and_messages() {
    clear_context_stack();
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    run_script(
        r#"
        ec.message.addMessage("caller message");
        ec.message.addError("caller error");
        ec.message.pushErrors();
        isolated = !ec.message.hasError();
        ec.message.addError("tool error");
        toolError = ec.message.errorsString;
        ec.message.clearErrors();
        ec.message.popErrors();
    "#,
    )
    .unwrap();
    holds(r#"isolated && toolError === "tool error" && ec.message.getErrorsString() === "caller error""#);
    assert_eq!(context.message().messages(), ["caller message"]);
}

#[test]
fn mounted_transition_metadata_has_service_parameters_and_a_path() {
    clear_context_stack();
    let fixture = Fixture::new();
    let root = fixture.0.join("Root.xml");
    let child = fixture.0.join("Child.xml");
    let outside = fixture.0.join("Outside.xml");
    let screen = r#"<screen><transition name="submit" method="post"><parameter name="id" required="true"/>
        <service-call name="test.store#Thing"/><default-response type="none"/></transition><widgets/></screen>"#;
    std::fs::write(&child, screen).unwrap();
    std::fs::write(&outside, screen).unwrap();
    std::fs::write(
        &root,
        format!(
            r#"<screen><subscreens><subscreens-item name="bad" location="{}"/><subscreens-item name="child" location="{}"/></subscreens><widgets/></screen>"#,
            fixture.0.join("missing.xml").display(), child.display()
        ),
    )
    .unwrap();
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    put_context_all(&indexmap::indexmap! {
        "rootLocation".to_string() => Value::string(root.to_str().unwrap()),
        "childLocation".to_string() => Value::string(child.to_str().unwrap()),
        "outsideLocation".to_string() => Value::string(outside.to_str().unwrap()),
    });
    run_script(r#"transition = ec.screen.getTransitionInfo(rootLocation, childLocation, "submit", "post")"#).unwrap();
    holds(
        r#"transition.path === "/child/submit" && transition.serviceName === "test.store#Thing" && transition.parameterMap.id.required"#,
    );
    assert!(eval_condition(r#"ec.screen.getTransitionInfo(rootLocation, childLocation, "submit", "get")"#).is_err());
    assert!(eval_condition(r#"ec.screen.getTransitionInfo(rootLocation, outsideLocation, "submit", "post")"#).is_err());
}

#[test]
fn service_parameter_metadata_is_available_to_scripts() {
    clear_context_stack();
    let fixture = Fixture::new();
    let service = fixture.0.join("service/Meta.xml");
    std::fs::write(
        &service,
        r#"<services><service verb="inspect" noun="Metadata" authenticate="false">
        <in-parameters><parameter name="requiredId" required="true"/><parameter name="optionalId"/>
            <auto-parameters entity-name="moqui.basic.Enumeration" include="pk"/></in-parameters>
        <out-parameters><parameter name="result" type="Map"/></out-parameters>
        <actions><set field="result" from="{}"/></actions>
    </service></services>"#,
    )
    .unwrap();
    let context = ExecutionContextFactory::test_context();
    let _entered = context.enter();
    context.factory().set_entity(crate::testing::open());
    assert!(crate::service::lint(&[service]).unwrap().is_empty());
    run_script(r#"definition = ec.service.getServiceDefinition("Meta.inspect#Metadata")"#).unwrap();
    holds(r#"definition.inParameterInfoMap.requiredId.required && !definition.inParameterInfoMap.optionalId.required"#);
    holds(
        r#"definition.inParameterNames.includes("requiredId") && definition.inParameterNames.includes("enumId") && definition.getInParameter("requiredId").attribute("required") === "true""#,
    );
    holds(r#"definition.getInParameter("missing") === null && definition.outParameterInfoMap.result.type === "Map""#);
}
