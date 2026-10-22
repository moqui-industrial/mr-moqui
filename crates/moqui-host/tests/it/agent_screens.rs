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

//! The agents' screen sessions on a seeded server on embedded Postgres (plans/LLM-A2A-Assist.md 4.7, 9 Screens): a
//! press that writes is held and leaves the row as it was, the held request run again with a valid approval grant
//! writes it and with a tampered one does not, the browser's values come in by form `domId` and field name, the
//! session answers follow events, and a run of the scripted fake provider uses the `screen` tool end to end (a `go` to
//! a transition refused, a held save, the follow events stored, the conversation's screen state kept). Approvals
//! themselves are phase 13.4, so the gate is exercised through `ScreenSessions::execute_approved` directly.

#![cfg(all(
    unix,
    feature = "embedded-postgres",
    feature = "axum",
    feature = "llm",
    feature = "tui"
))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::llm::provider::fake::{self, Step};
use moqui_core::llm::screen::{
    self, ApprovalGrant, BrowserForm, BrowserState, OpenSpec, ScreenCommand, ScreenError, ScreenSessions,
};
use moqui_lib::object::{to_plain_string, Value};
use serde_json::json;

use crate::common::start_with_conf_files;

const ENDPOINT: &str = "screens-fake";
const OWNER: &str = "EX_JOHN_DOE";
const PATH: &str = "/apps/agentscreens";

const DATA: &str = r#"<entity-facade-xml type="seed">
    <moqui.llm.LlmEndpoint endpointId="screens-fake" protocol="fake" description="Screens fake" isLocal="Y"/>
    <moqui.llm.LlmModel modelId="screens-model" endpointId="screens-fake" providerModel="fake-1" tier="fast"
        contextWindow="32768"/>
    <moqui.llm.LlmFeatureBinding featureId="assist" modelId="screens-model" reasoningLevel="none"/>
    <moqui.basic.EnumerationType enumTypeId="AGT_SCREEN" description="Before"/>
    <moqui.security.ArtifactGroup artifactGroupId="AGENT_SCREENS" description="Agent screen sessions test"/>
    <moqui.security.ArtifactGroupMember artifactGroupId="AGENT_SCREENS" artifactName="component://agentscreens/.*"
        nameIsPattern="Y" artifactTypeEnumId="AT_XML_SCREEN" inheritAuthz="Y"/>
    <moqui.security.ArtifactGroupMember artifactGroupId="AGENT_SCREENS" artifactName="component://agentscreens/.*"
        nameIsPattern="Y" artifactTypeEnumId="AT_XML_SCREEN_TRANS" inheritAuthz="Y"/>
    <moqui.security.ArtifactGroupMember artifactGroupId="AGENT_SCREENS" artifactName=".*EnumerationType"
        nameIsPattern="Y" artifactTypeEnumId="AT_SERVICE"/>
    <moqui.security.ArtifactAuthz artifactAuthzId="AGENT_SCREENS_ADMIN" userGroupId="ADMIN" artifactGroupId="AGENT_SCREENS"
        authzTypeEnumId="AUTHZT_ALWAYS" authzActionEnumId="AUTHZA_ALL"/>
</entity-facade-xml>"#;

const CONF: &str = r#"<screen-facade>
    <screen location="component://webroot/screen/webroot/apps.xml">
        <subscreens-item name="agentscreens" menu-title="Agent Screens" location="component://agentscreens/screen/AgentScreens.xml"/>
    </screen>
</screen-facade>"#;

const COMPONENT: &str = r#"<component name="agentscreens"/>"#;

/// One form that writes an `EnumerationType` through a service, and a second screen to go to.
const SCREEN: &str = r#"<screen require-authentication="true" default-menu-title="Agent Screens">
    <transition name="updateEnumType"><service-call name="update#moqui.basic.EnumerationType"/>
        <default-response url="."/></transition>
    <subscreens default-item="Edit"/>
    <widgets><subscreens-active/></widgets>
</screen>"#;

const EDIT: &str = r#"<screen require-authentication="true" default-menu-title="Edit">
    <transition name="updateEnumType"><service-call name="update#moqui.basic.EnumerationType"/>
        <default-response url="."/></transition>
    <!-- lookups with actions and no read-only, as many apps' search transitions: the gate would hold them as writes -->
    <transition name="getEnumTypes"><actions>
        <entity-find entity-name="moqui.basic.EnumerationType" list="typeList">
            <econdition field-name="enumTypeId" operator="like" value="AGT_%"/></entity-find>
        <script>ec.web.sendJsonResponse(typeList.map((row) => ({value: row.enumTypeId, label: row.enumTypeId + ' ' + row.description})))</script>
    </actions><default-response type="none"/></transition>
    <transition name="writeOnLookup"><actions><script>
        const made = ec.entity.makeValue('moqui.basic.EnumerationType'); made.set('enumTypeId', 'AGT_LOOKUP_WROTE');
        made.set('description', 'a lookup that writes'); made.create();
        ec.web.sendJsonResponse([{value: 'X', label: 'X'}])
    </script></actions><default-response type="none"/></transition>
    <actions><entity-find-one entity-name="moqui.basic.EnumerationType" value-field="enumType">
        <field-map field-name="enumTypeId" value="AGT_SCREEN"/></entity-find-one></actions>
    <widgets>
        <form-single name="EditEnumType" transition="updateEnumType" map="enumType">
            <field name="enumTypeId"><default-field><hidden/></default-field></field>
            <field name="description"><default-field title="Description"><text-line/></default-field></field>
            <field name="submitButton"><default-field title="Save"><submit/></default-field></field>
        </form-single>
        <form-single name="LookupForm">
            <field name="lookupType"><default-field title="Lookup Type"><drop-down allow-empty="true">
                <dynamic-options transition="getEnumTypes"/></drop-down></default-field></field>
            <field name="writingLookup"><default-field title="Writing Lookup"><drop-down allow-empty="true">
                <dynamic-options transition="writeOnLookup"/></drop-down></default-field></field>
        </form-single>
    </widgets>
</screen>"#;

const OTHER: &str = r#"<screen require-authentication="true" default-menu-title="Other">
    <widgets><label text="the other screen"/></widgets>
</screen>"#;

fn enum_type(factory: &Arc<ExecutionContextFactory>, id: &str) -> Option<String> {
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let facade = factory.entity().unwrap();
    facade
        .find("moqui.basic.EnumerationType")
        .unwrap()
        .eq("enumTypeId", Value::string(id))
        .one()
        .unwrap()
        .map(|row| to_plain_string(row.get("description").unwrap()))
}

fn description(factory: &Arc<ExecutionContextFactory>) -> String {
    enum_type(factory, "AGT_SCREEN").expect("AGT_SCREEN")
}

/// Change the row underneath a session, as another user's save would.
fn set_description(factory: &Arc<ExecutionContextFactory>, text: &str) {
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let facade = factory.entity().unwrap();
    let mut row = facade
        .find("moqui.basic.EnumerationType")
        .unwrap()
        .eq("enumTypeId", Value::string("AGT_SCREEN"))
        .one()
        .unwrap()
        .unwrap();
    row.set("description", Value::string(text));
    row.update(&facade).unwrap();
}

fn spec(conversation: &str, follow: bool) -> OpenSpec {
    OpenSpec {
        user_id: OWNER.to_string(),
        username: "john.doe".to_string(),
        conversation_id: conversation.to_string(),
        agent_id: "assist".to_string(),
        run_id: Some("AGS_RUN".to_string()),
        start_path: format!("{PATH}/Edit"),
        columns: 100,
        rows: 30,
        follow,
        state: None,
    }
}

fn command() -> ScreenCommand {
    ScreenCommand {
        agent_id: "assist".to_string(),
        run_id: Some("AGS_RUN".to_string()),
        ..ScreenCommand::default()
    }
}

#[test]
fn agent_screen_sessions_hold_writes_sync_in_and_follow() {
    let files = [
        ("component/agentscreens/component.xml", COMPONENT),
        ("component/agentscreens/screen/AgentScreens.xml", SCREEN),
        ("component/agentscreens/screen/AgentScreens/Edit.xml", EDIT),
        ("component/agentscreens/screen/AgentScreens/Other.xml", OTHER),
    ];
    let gate = start_with_conf_files("agent-screens", Some(DATA), CONF, &[], &files);
    let factory = Arc::clone(gate.server.factory());
    let sessions = screen::sessions().expect("the host installs the screen sessions");
    held_and_approved(&factory, sessions.as_ref());
    run_on_a_fresh_link(&factory, sessions.as_ref());
    lookups_run_fenced(&factory, sessions.as_ref());
    synced_and_followed(sessions.as_ref());
    a_run_uses_the_screen_tool(&factory);
}

/// A save is held, the row unchanged; the held request with a tampered field is refused, with the approved hash it
/// writes.
fn held_and_approved(factory: &Arc<ExecutionContextFactory>, sessions: &dyn ScreenSessions) {
    let id = sessions.open(&spec("AGS_C1", false)).unwrap();
    assert_eq!(sessions.find(OWNER, "AGS_C1").as_deref(), Some(id.as_str()));
    assert!(
        sessions.find("EX_JANE", "AGS_C1").is_none(),
        "another user does not find it"
    );
    assert_eq!(
        sessions.command(&id, "EX_JANE", &command()).unwrap_err(),
        ScreenError::NotFound,
        "nor use it"
    );
    let look = sessions.command(&id, OWNER, &command()).unwrap();
    assert!(look.text.contains("[needs approval] Save"), "{}", look.text);
    assert!(look.path.ends_with("/Edit"), "{}", look.path);

    let saved = sessions
        .command(
            &id,
            OWNER,
            &ScreenCommand {
                set: vec![("Description".to_string(), "After".to_string())],
                press: Some("Save".to_string()),
                ..command()
            },
        )
        .unwrap();
    let write = saved.held.clone().unwrap_or_else(|| panic!("held: {}", saved.text));
    assert!(saved.text.contains("waits for the user's approval"), "{}", saved.text);
    assert_eq!(description(factory), "Before", "a held write runs nothing");
    assert_eq!(write.transition(), "updateEnumType");
    assert_eq!(write.held["service"], "update#moqui.basic.EnumerationType");
    assert_eq!(write.form, "EditEnumType");
    assert_eq!(write.fields.get("description").map(String::as_str), Some("After"));
    assert_eq!(write.changes.len(), 1, "{:?}", write.changes);
    assert_eq!(
        (write.changes[0].original.as_str(), write.changes[0].proposed.as_str()),
        ("Before", "After")
    );
    assert_eq!(
        saved.state.values["EditEnumType"]["description"], "After",
        "the edit is kept"
    );
    assert_eq!(write.owner, OWNER);
    assert_eq!(write.agent_id, "assist");
    assert!(
        sessions.recheck(&id, OWNER, &write).unwrap().is_empty(),
        "nothing changed since"
    );
    // the field changed underneath: a conflict with the value now
    set_description(factory, "Changed");
    let conflicts = sessions.recheck(&id, OWNER, &write).unwrap();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(
        (
            conflicts[0].field.as_str(),
            conflicts[0].original.as_str(),
            conflicts[0].current.as_str()
        ),
        ("description", "Before", "Changed")
    );
    assert_eq!(conflicts[0].proposed, "After");
    set_description(factory, "Before");
    // another user may neither recheck nor run it, with the session's id or without one
    assert_eq!(
        sessions.recheck(&id, "EX_JANE", &write).unwrap_err(),
        ScreenError::NotFound
    );

    let grant = ApprovalGrant {
        approval_id: "AGS_AP1".to_string(),
        args_hash: write.args_hash.clone(),
        agent_id: "assist".to_string(),
        run_id: Some("AGS_RUN".to_string()),
    };
    assert_eq!(
        sessions.execute_approved(&id, "EX_JANE", &write, &grant).unwrap_err(),
        ScreenError::NotFound
    );
    assert_eq!(
        sessions
            .execute_approved("gone", "EX_JANE", &write, &grant)
            .unwrap_err(),
        ScreenError::NotFound
    );
    assert_eq!(description(factory), "Before");
    let mut tampered = write.clone();
    tampered.fields.insert("description".to_string(), "Other".to_string());
    assert!(
        matches!(
            sessions.execute_approved(&id, OWNER, &tampered, &grant),
            Err(ScreenError::Refused(_))
        ),
        "another value is another write"
    );
    assert_eq!(description(factory), "Before");
    let done = sessions.execute_approved(&id, OWNER, &write, &grant).unwrap();
    assert!(done.ok, "{}", done.text);
    assert_eq!(description(factory), "After", "the approved write ran");
    assert!(done.state.values.is_empty(), "nothing left unsaved: {:?}", done.state);
    sessions.close(&id);
    assert!(sessions.find(OWNER, "AGS_C1").is_none());
}

/// The session is gone (reaped, another node): the approved write runs on a link of its own.
fn run_on_a_fresh_link(factory: &Arc<ExecutionContextFactory>, sessions: &dyn ScreenSessions) {
    let id = sessions.open(&spec("AGS_C4", false)).unwrap();
    let saved = sessions
        .command(
            &id,
            OWNER,
            &ScreenCommand {
                set: vec![("Description".to_string(), "Fresh".to_string())],
                press: Some("Save".to_string()),
                ..command()
            },
        )
        .unwrap();
    let write = saved.held.unwrap_or_else(|| panic!("held: {}", saved.text));
    sessions.close(&id);
    assert!(sessions.find(OWNER, "AGS_C4").is_none());
    let grant = ApprovalGrant {
        approval_id: "AGS_AP2".to_string(),
        args_hash: write.args_hash.clone(),
        agent_id: "assist".to_string(),
        run_id: None,
    };
    assert!(
        sessions.recheck(&id, OWNER, &write).unwrap().is_empty(),
        "rechecked with no session"
    );
    let done = sessions.execute_approved(&id, OWNER, &write, &grant).unwrap();
    assert!(done.ok, "{}", done.text);
    assert_eq!(description(factory), "Fresh", "the approved write ran with no session");
}

/// A drop-down's lookup transition with actions and no `read-only` answers its choices rather than being held; one
/// that writes fails at the fence as a failed lookup, and neither is a write waiting for approval.
fn lookups_run_fenced(factory: &Arc<ExecutionContextFactory>, sessions: &dyn ScreenSessions) {
    let id = sessions.open(&spec("AGS_C3", false)).unwrap();
    let chosen = sessions
        .command(
            &id,
            OWNER,
            &ScreenCommand {
                set: vec![("Lookup Type".to_string(), "AGT_SCREEN".to_string())],
                ..command()
            },
        )
        .unwrap();
    assert!(chosen.ok, "{}", chosen.text);
    assert!(chosen.held.is_none(), "a lookup is no pending write");
    assert_eq!(chosen.state.values["LookupForm"]["lookupType"], "AGT_SCREEN");
    let refused = sessions
        .command(
            &id,
            OWNER,
            &ScreenCommand {
                set: vec![("Writing Lookup".to_string(), "X".to_string())],
                ..command()
            },
        )
        .unwrap();
    assert!(!refused.ok, "{}", refused.text);
    assert!(
        refused.held.is_none(),
        "a lookup that writes is no pending write: {}",
        refused.text
    );
    assert_eq!(
        enum_type(factory, "AGT_LOOKUP_WROTE"),
        None,
        "the fence stopped its write"
    );
    sessions.close(&id);
}

/// The browser's path and typed values come into the session; the session's own changes answer follow events.
fn synced_and_followed(sessions: &dyn ScreenSessions) {
    let id = sessions.open(&spec("AGS_C2", true)).unwrap();
    let state = BrowserState {
        path: format!("{PATH}/Edit"),
        parameters: BTreeMap::new(),
        forms: vec![BrowserForm {
            dom_id: "EditEnumType".to_string(),
            values: BTreeMap::from([
                ("description".to_string(), "Typed".to_string()),
                ("enumTypeId".to_string(), "AGT_SCREEN".to_string()),
            ]),
            dirty: vec!["description".to_string()],
        }],
        dialogs: Vec::new(),
    };
    assert!(sessions.sync(&id, OWNER, &state).unwrap().is_empty());
    let look = sessions.command(&id, OWNER, &command()).unwrap();
    assert!(look.text.contains("Typed"), "{}", look.text);
    assert_eq!(look.state.values["EditEnumType"]["description"], "Typed");

    let set = sessions
        .command(
            &id,
            OWNER,
            &ScreenCommand {
                set: vec![("Description".to_string(), "Followed".to_string())],
                ..command()
            },
        )
        .unwrap();
    let fill = set
        .follow
        .iter()
        .find(|event| event.event_type == "ui.fill")
        .unwrap_or_else(|| panic!("{:?}", set.follow));
    assert_eq!(fill.payload["form"], "EditEnumType");
    assert_eq!(fill.payload["domId"], "EditEnumType");
    assert_eq!(fill.payload["occurrence"], 0);
    assert_eq!(fill.payload["field"], "description");
    assert_eq!(fill.payload["value"], "Followed");
    assert_eq!(fill.payload["label"], "Description");

    let went = sessions
        .command(
            &id,
            OWNER,
            &ScreenCommand {
                go: Some(format!("{PATH}/Other")),
                ..command()
            },
        )
        .unwrap();
    assert!(went.ok, "{}", went.text);
    let navigate = went
        .follow
        .iter()
        .find(|event| event.event_type == "ui.navigate")
        .unwrap_or_else(|| panic!("{:?}", went.follow));
    assert!(
        navigate.payload["path"].as_str().unwrap().ends_with("/Other"),
        "{navigate:?}"
    );
    sessions.close(&id);
}

/// A drawer run of the scripted fake provider: a `go` to a transition is refused, a save is held for approval and the
/// row stays, the follow events are stored events of the run, and the conversation keeps the screen state.
fn a_run_uses_the_screen_tool(factory: &Arc<ExecutionContextFactory>) {
    let tool = |id: &str, arguments: serde_json::Value| Step::Tool {
        id: id.to_string(),
        name: "screen".to_string(),
        arguments: arguments.to_string(),
        pieces: 2,
    };
    fake::clear(ENDPOINT);
    fake::script(
        ENDPOINT,
        vec![
            fake::Reply::steps(vec![
                tool("s1", json!({"go": format!("{PATH}/updateEnumType")})),
                tool(
                    "s2",
                    json!({"go": format!("{PATH}/Edit"), "set": {"Description": "From the run"}, "do": "Save"}),
                ),
                Step::Finish("tool_calls".to_string()),
            ]),
            fake::Reply::text("The change waits for your approval."),
        ],
    );
    let before = description(factory);
    let context = factory.new_context();
    let _entered = context.enter();
    let started = moqui_core::llm::run::start_run(
        factory,
        &moqui_core::llm::run::StartRequest {
            user_id: OWNER.to_string(),
            text: "Change the description".to_string(),
            surface: moqui_core::llm::prompt::Surface::Drawer,
            ..moqui_core::llm::run::StartRequest::default()
        },
    )
    .unwrap();
    let state = moqui_core::llm::run::wait_for(factory, &started.run_id, Duration::from_secs(60)).unwrap();
    assert_eq!(state, moqui_core::llm::run::state::RunState::Completed);
    assert_eq!(description(factory), before, "the run's save was held");

    let _authz = context.disable_authz();
    let facade = factory.entity().unwrap();
    let results: BTreeMap<String, String> = facade
        .find("moqui.agent.AgentToolCall")
        .unwrap()
        .eq("runId", Value::string(&started.run_id))
        .list()
        .unwrap()
        .values()
        .iter()
        .map(|row| {
            (
                row.get("providerCallId").map(to_plain_string).unwrap_or_default(),
                row.get("resultSummary").map(to_plain_string).unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(results.len(), 2, "{results:?}");
    let (refused, held) = (&results["s1"], &results["s2"]);
    assert!(
        refused.contains("is an action (updateEnumType), not a screen"),
        "{refused}"
    );
    assert!(refused.contains("--- error"), "{refused}");
    assert!(held.contains("approval pending: updateEnumType"), "{held}");
    assert!(
        held.contains(&format!("Description: {before} → From the run")),
        "{held}"
    );

    let events = moqui_core::llm::run::events::stored_after(&facade, &started.run_id, 0, 1000).unwrap();
    let types: Vec<&str> = events.iter().map(|event| event.event_type.as_str()).collect();
    assert!(types.contains(&"ui.fill"), "{types:?}");
    let fill = events.iter().find(|event| event.event_type == "ui.fill").unwrap();
    assert_eq!(fill.payload["field"], "description", "{:?}", fill.payload);

    let conversation = facade
        .find("moqui.agent.AgentConversation")
        .unwrap()
        .eq("conversationId", Value::string(&started.conversation_id))
        .one()
        .unwrap()
        .unwrap();
    let kept: screen::ScreenState =
        serde_json::from_str(&to_plain_string(conversation.get("screenStateJson").unwrap())).unwrap();
    assert!(kept.path.ends_with("/Edit"), "{kept:?}");
    assert_eq!(kept.values["EditEnumType"]["description"], "From the run");
    let sent = fake::requests(ENDPOINT);
    let tools: Vec<&str> = sent[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tools.contains(&"screen"), "the screen tool is offered: {tools:?}");
}
