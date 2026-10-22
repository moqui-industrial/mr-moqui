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

//! The Assist drawer's JSON calls (docs/Assist.md; plans/LLM-A2A-Assist.md 4.5, 4.11) over HTTP on a seeded server on
//! embedded Postgres, with the scripted fake provider: each call's happy path and its refusal for another user; a turn
//! sent twice with one client message id; a turn while a run is active queued as input and not counted by the AT_LLM
//! tarpit, which counts runs; the client's tier; a decision refused for a header login and taken from the owner's
//! browser session with its token; a run that streams to its approval, is decided over HTTP and resumes; and the
//! admin screens, where another user's content needs AgentAudit and each such read is logged.

#![cfg(all(unix, feature = "embedded-postgres", feature = "axum", feature = "llm"))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::llm::provider::fake::{self, Step};
use moqui_core::llm::run::state::RunState;
use moqui_lib::object::{Value, to_plain_string};
use serde_json::{Value as Json, json};

use crate::common::{basic, exchange, http_request, merge_cookies, session_token, sign_in, start_with_data};

const ENDPOINT: &str = "assist-fake";
const WAIT: Duration = Duration::from_secs(60);

const DATA: &str = r#"<entity-facade-xml type="seed">
    <moqui.llm.LlmEndpoint endpointId="assist-fake" protocol="fake" description="Assist fake" isLocal="Y"/>
    <moqui.llm.LlmModel modelId="assist-fast" endpointId="assist-fake" providerModel="fake-fast" tier="fast"
        contextWindow="32768"/>
    <moqui.llm.LlmModel modelId="assist-thorough" endpointId="assist-fake" providerModel="fake-thorough" tier="thorough"
        contextWindow="32768"/>
    <moqui.llm.LlmFeatureBinding featureId="assist" modelId="assist-fast" reasoningLevel="none"/>
    <moqui.basic.EnumerationType enumTypeId="AST_TYPE" description="Before"/>
    <moqui.agent.AgentToolExposure toolName="update_enum_type" serviceName="update#moqui.basic.EnumerationType"
        title="Change an enumeration type" lockVersion="1"/>
    <moqui.security.ArtifactGroup artifactGroupId="ASSIST_TEST" description="Assist drawer test"/>
    <moqui.security.ArtifactGroupMember artifactGroupId="ASSIST_TEST" artifactName=".*EnumerationType"
        nameIsPattern="Y" artifactTypeEnumId="AT_SERVICE"/>
    <moqui.security.ArtifactGroupMember artifactGroupId="ASSIST_TEST" artifactName="moqui.basic.EnumerationType"
        artifactTypeEnumId="AT_ENTITY"/>
    <moqui.security.ArtifactAuthz artifactAuthzId="ASSIST_TEST_ADMIN" userGroupId="ADMIN" artifactGroupId="ASSIST_TEST"
        authzTypeEnumId="AUTHZT_ALWAYS" authzActionEnumId="AUTHZA_ALL"/>
    <!-- ann uses the agents and nothing else; bob may not use them; both sign in with the password moqui -->
    <moqui.security.UserAccount userId="AST_ANN" username="ann.assist" userFullName="Ann Assist"
        currentPassword="16ac58bbfa332c1c55bd98b53e60720bfa90d394" passwordHashType="SHA" locale="en_US" timeZone="US/Central"/>
    <moqui.security.UserAccount userId="AST_BOB" username="bob.assist" userFullName="Bob Assist"
        currentPassword="16ac58bbfa332c1c55bd98b53e60720bfa90d394" passwordHashType="SHA"/>
    <moqui.security.UserGroup userGroupId="AST_AGENT_USERS" description="Assist users"/>
    <moqui.security.UserGroupMember userGroupId="AST_AGENT_USERS" userId="AST_ANN" fromDate="1265184000000"/>
    <moqui.security.UserGroupPermission userGroupId="AST_AGENT_USERS" userPermissionId="AgentUse" fromDate="1265184000000"/>
    <!-- ann's runs are limited to 2 a minute: the third locks the fourth out -->
    <moqui.security.ArtifactTarpit artifactTarpitId="AST_ANN_RUNS" userGroupId="AST_AGENT_USERS" artifactGroupId="AgentRuns"
        maxHitsCount="2" maxHitsDuration="60" tarpitDuration="60"/>
    <moqui.agent.AgentSuggestedPrompt promptId="AST_P1" label="Find users" promptText="Find the user accounts"
        sendMode="send" screenPathPattern="/apps/system.*" sequenceNum="1"/>
</entity-facade-xml>"#;

/// A browser session of `username`: its cookie and the session token of a page.
fn browser(address: SocketAddr, username: &str) -> (String, String) {
    let cookie = sign_in(address, username, "moqui");
    let (status, headers, html) = exchange(
        address,
        &http_request("GET", "/qapps", &[("Cookie", &cookie), ("Accept", "text/html")], ""),
    );
    let token = session_token(&html)
        .or_else(|| crate::common::header(&headers, "moquiSessionToken"))
        .unwrap_or_else(|| {
            panic!(
                "no session token for {username}: {status} {}",
                html.chars().take(500).collect::<String>()
            )
        });
    (merge_cookies(&cookie, &headers), token)
}

/// A signed-in user with the token of their session.
struct Client {
    address: SocketAddr,
    cookie: String,
    token: String,
}

impl Client {
    fn sign_in(address: SocketAddr, username: &str) -> Self {
        let (cookie, token) = browser(address, username);
        Self { address, cookie, token }
    }

    fn get(&self, call: &str, query: &str) -> (u16, Json) {
        let path = if query.is_empty() {
            format!("/assist/{call}")
        } else {
            format!("/assist/{call}?{query}")
        };
        let (status, _, body) = exchange(
            self.address,
            &http_request(
                "GET",
                &path,
                &[("Cookie", &self.cookie), ("Accept", "application/json")],
                "",
            ),
        );
        (status, parse(&body))
    }

    fn post(&self, call: &str, body: &Json) -> (u16, Json) {
        let (status, _, text) = self.post_raw(call, body, true);
        (status, parse(&text))
    }

    /// A form post, as `moqui.request` sends a plain object: an object parameter is JSON text.
    fn post_form(&self, call: &str, form: &[(&str, &str)]) -> (u16, Json) {
        let body: Vec<String> = form
            .iter()
            .map(|(name, value)| format!("{name}={}", urlencoding(value)))
            .collect();
        let headers = [
            ("Cookie", self.cookie.as_str()),
            ("Accept", "application/json"),
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("X-CSRF-Token", self.token.as_str()),
        ];
        let (status, _, text) = exchange(
            self.address,
            &http_request("POST", &format!("/assist/{call}"), &headers, &body.join("&")),
        );
        (status, parse(&text))
    }

    fn post_raw(&self, call: &str, body: &Json, with_token: bool) -> (u16, Vec<(String, String)>, String) {
        let text = body.to_string();
        let mut headers = vec![
            ("Cookie", self.cookie.as_str()),
            ("Accept", "application/json"),
            ("Content-Type", "application/json"),
        ];
        if with_token {
            headers.push(("X-CSRF-Token", self.token.as_str()));
        }
        exchange(
            self.address,
            &http_request("POST", &format!("/assist/{call}"), &headers, &text),
        )
    }
}

fn urlencoding(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn parse(body: &str) -> Json {
    serde_json::from_str(body).unwrap_or_else(|_| json!({"unparsed": body}))
}

fn read_stream(address: SocketAddr, request: &str, limit: Duration) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(limit)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut out = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => out.extend_from_slice(&buffer[..count]),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn rows(
    factory: &Arc<ExecutionContextFactory>,
    entity: &str,
    field: &str,
    value: &str,
) -> Vec<moqui_core::entity::EntityValue> {
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    factory
        .entity()
        .unwrap()
        .find(entity)
        .unwrap()
        .eq(field, Value::string(value))
        .list()
        .unwrap()
        .values()
        .to_vec()
}

fn field(row: &moqui_core::entity::EntityValue, name: &str) -> String {
    row.get(name).map(to_plain_string).unwrap_or_default()
}

fn wait(factory: &Arc<ExecutionContextFactory>, run_id: &str) -> RunState {
    moqui_core::llm::run::wait_for(factory, run_id, WAIT).unwrap()
}

fn description(factory: &Arc<ExecutionContextFactory>) -> String {
    field(
        &rows(factory, "moqui.basic.EnumerationType", "enumTypeId", "AST_TYPE")[0],
        "description",
    )
}

fn str_of(value: &Json) -> String {
    value.as_str().unwrap_or_default().to_string()
}

#[test]
fn the_drawer_calls_over_http_on_postgres() {
    let gate = start_with_data("assist", DATA);
    let factory = Arc::clone(gate.server.factory());
    let address = gate.server.address();
    let context = factory.new_context();
    let _entered = context.enter();
    fake::clear(ENDPOINT);

    let john = Client::sign_in(address, "john.doe");
    let ann = Client::sign_in(address, "ann.assist");
    let bob = Client::sign_in(address, "bob.assist");

    // bootstrap: the agents, tiers and features; no user is 401, a user without AgentUse 403
    let (status, boot) = john.get("bootstrap", "");
    assert_eq!(status, 200, "{boot}");
    assert_eq!(boot["defaultAgentId"], "assist", "{boot}");
    let assist = boot["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agentId"] == "assist")
        .unwrap();
    assert_eq!(assist["tiers"], json!(["fast", "thorough"]), "{boot}");
    assert_eq!(assist["tierFixed"], false);
    assert!(
        boot["secretFields"]["contains"]
            .as_array()
            .unwrap()
            .contains(&json!("password"))
    );
    let (status, _, _) = exchange(
        address,
        &http_request("GET", "/assist/bootstrap", &[("Accept", "application/json")], ""),
    );
    assert_eq!(status, 401);
    let (status, refused) = bob.get("bootstrap", "");
    assert_eq!((status, refused["code"].as_str()), (403, Some("refused")), "{refused}");

    // a turn starts a run in a new conversation, at the tier asked for, from the page the user is on
    fake::script(ENDPOINT, vec![fake::Reply::text("Hello from Assist.")]);
    let turn = json!({"text": "Say hello\nplease", "clientMessageId": "john-m1", "clientId": "tab-a", "tier": "thorough",
        "contextVersion": 1, "state": {"path": "/qapps/system/Llm", "title": "LLM and Agents",
            "forms": [{"domId": "Find", "values": {"name": "x", "password": "hunter2"}, "dirty": ["name", "password"]}]}});
    let (status, _, text) = john.post_raw("turn", &turn, false);
    assert_eq!(status, 401, "a POST without the session token: {text}");
    let (status, started) = john.post("turn", &turn);
    assert_eq!(status, 200, "{started}");
    assert_eq!(
        (started["queued"].as_bool(), started["existing"].as_bool()),
        (Some(false), Some(false)),
        "{started}"
    );
    let conversation_id = str_of(&started["conversationId"]);
    let run_id = str_of(&started["runId"]);
    assert_eq!(started["follow"]["clientId"], "tab-a");
    assert_eq!(wait(&factory, &run_id), RunState::Completed);
    let sent = fake::requests(ENDPOINT);
    assert_eq!(
        sent.last().unwrap()["model"],
        "fake-thorough",
        "the client's tier chose the model"
    );
    let run_row = &rows(&factory, "moqui.agent.AgentRun", "runId", &run_id)[0];
    assert_eq!(field(run_row, "tier"), "thorough");
    let message = &rows(&factory, "moqui.agent.AgentMessage", "clientMessageId", "john-m1")[0];
    let metadata = field(message, "metadataJson");
    assert!(
        metadata.contains("page /apps/system/Llm") && !metadata.contains("hunter2"),
        "{metadata}"
    );
    let conversation = &rows(
        &factory,
        "moqui.agent.AgentConversation",
        "conversationId",
        &conversation_id,
    )[0];
    assert_eq!(field(conversation, "title"), "Say hello");
    assert!(field(conversation, "screenStateJson").contains("/apps/system/Llm"));
    assert!(!field(conversation, "screenStateJson").contains("hunter2"));

    // the same message again is the same run
    let (status, again) = john.post("turn", &turn);
    assert_eq!(
        (status, again["existing"].as_bool(), str_of(&again["runId"])),
        (200, Some(true), run_id.clone()),
        "{again}"
    );
    assert_eq!(
        rows(&factory, "moqui.agent.AgentRun", "conversationId", &conversation_id).len(),
        1
    );

    // the conversation: its items, runs and follow owner; another user's is not found
    let (status, read) = john.get(
        "conversation",
        &format!("conversationId={conversation_id}&clientId=tab-a"),
    );
    assert_eq!(status, 200, "{read}");
    let items = read["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{read}");
    assert_eq!(
        (items[0]["role"].as_str(), items[0]["parts"][0]["text"].as_str()),
        (Some("user"), Some("Say hello\nplease"))
    );
    assert_eq!(items[1]["parts"][0]["text"], "Hello from Assist.");
    assert_eq!(read["follow"]["following"], true);
    assert!(read["activeRun"].is_null());
    let assistant_message = str_of(&items[1]["messageId"]);
    let (status, _) = ann.get("conversation", &format!("conversationId={conversation_id}"));
    assert_eq!(status, 404);

    // the list and its search; another user's list does not show it
    let (status, list) = john.get("conversations", "search=hello");
    assert_eq!(status, 200, "{list}");
    assert_eq!(
        list["conversations"][0]["conversationId"],
        conversation_id.as_str(),
        "{list}"
    );
    let (_, list) = john.get("conversations", "search=nothing-like-it");
    assert_eq!(list["conversations"].as_array().unwrap().len(), 0);
    let (_, list) = ann.get("conversations", "");
    assert!(!list.to_string().contains(&conversation_id), "{list}");

    // feedback on the assistant's message; not on another user's
    let (status, kept) = john.post(
        "feedback",
        &json!({"messageId": assistant_message, "rating": "up", "note": "good"}),
    );
    assert_eq!(
        (status, kept["feedback"]["rating"].as_str()),
        (200, Some("up")),
        "{kept}"
    );
    let (_, read) = john.get("conversation", &format!("conversationId={conversation_id}"));
    assert_eq!(read["items"][1]["feedback"]["note"], "good");
    let (status, _) = ann.post("feedback", &json!({"messageId": assistant_message, "rating": "down"}));
    assert_eq!(status, 404);

    // follow: another tab claims it; a navigation in a tab that does not follow changes nothing
    let (status, follow) = john.post(
        "follow",
        &json!({"conversationId": conversation_id, "clientId": "tab-b"}),
    );
    assert_eq!((status, follow["following"].as_bool()), (200, Some(true)), "{follow}");
    let (status, ignored) = john.post(
        "context",
        &json!({"conversationId": conversation_id, "clientId": "tab-a",
        "contextVersion": 9, "state": {"path": "/qapps/tools"}}),
    );
    assert_eq!((status, ignored["applied"].as_bool()), (200, Some(false)), "{ignored}");
    let (status, applied) = john.post(
        "context",
        &json!({"conversationId": conversation_id, "clientId": "tab-b",
        "contextVersion": 3, "state": {"path": "/qapps/system/Llm/Runs"}}),
    );
    assert_eq!((status, applied["applied"].as_bool()), (200, Some(true)), "{applied}");
    assert_eq!(applied["follow"], json!({"clientId": "tab-b", "contextVersion": 3}));
    let conversation = &rows(
        &factory,
        "moqui.agent.AgentConversation",
        "conversationId",
        &conversation_id,
    )[0];
    assert!(
        field(conversation, "screenStateJson").contains("/apps/system/Llm/Runs"),
        "the page synced in"
    );
    // the same as a form, whose page state is JSON text
    let (status, applied) = john.post_form(
        "context",
        &[
            ("conversationId", conversation_id.as_str()),
            ("clientId", "tab-b"),
            ("contextVersion", "4"),
            ("state", r#"{"path": "/qapps/system/Llm/Approvals"}"#),
        ],
    );
    assert_eq!(
        (status, applied["follow"]["contextVersion"].as_i64()),
        (200, Some(4)),
        "{applied}"
    );
    let conversation = &rows(
        &factory,
        "moqui.agent.AgentConversation",
        "conversationId",
        &conversation_id,
    )[0];
    assert!(field(conversation, "screenStateJson").contains("/apps/system/Llm/Approvals"));
    let (status, _) = ann.post(
        "follow",
        &json!({"conversationId": conversation_id, "clientId": "tab-x"}),
    );
    assert_eq!(status, 404);
    let (status, _) = ann.post(
        "context",
        &json!({"conversationId": conversation_id, "clientId": "tab-x", "contextVersion": 1}),
    );
    assert_eq!(status, 404);

    // retry runs the last message again; another user cannot
    fake::script(ENDPOINT, vec![fake::Reply::text("Hello again.")]);
    let (status, retried) = john.post(
        "retry",
        &json!({"conversationId": conversation_id, "clientId": "tab-b",
        "clientMessageId": "john-retry"}),
    );
    assert_eq!(status, 200, "{retried}");
    let retry_run = str_of(&retried["runId"]);
    assert_ne!(retry_run, run_id);
    assert_eq!(wait(&factory, &retry_run), RunState::Completed);
    let (status, _) = ann.post(
        "retry",
        &json!({"conversationId": conversation_id, "clientId": "tab-x",
        "clientMessageId": "ann-retry"}),
    );
    assert_eq!(status, 404);

    // a message while the run works is queued for it (an AgentInput), once for one client message id
    fake::script(
        ENDPOINT,
        vec![
            fake::Reply::steps(vec![
                Step::Delay(2500),
                Step::Text("Working slowly.".to_string()),
                Step::Finish("stop".to_string()),
            ]),
            fake::Reply::text("And the second message, folded in."),
        ],
    );
    let (status, slow) = john.post(
        "turn",
        &json!({"conversationId": conversation_id, "text": "Take your time",
        "clientMessageId": "john-m2", "clientId": "tab-b"}),
    );
    assert_eq!(status, 200, "{slow}");
    let slow_run = str_of(&slow["runId"]);
    let queued_turn = json!({"conversationId": conversation_id, "text": "Also this", "clientMessageId": "john-m3",
        "clientId": "tab-b"});
    let (status, queued) = john.post("turn", &queued_turn);
    assert_eq!(
        (status, queued["queued"].as_bool(), str_of(&queued["runId"])),
        (200, Some(true), slow_run.clone()),
        "{queued}"
    );
    let (_, queued_again) = john.post("turn", &queued_turn);
    assert_eq!(
        (queued_again["existing"].as_bool(), queued_again["queued"].as_bool()),
        (Some(true), Some(true)),
        "{queued_again}"
    );
    assert_eq!(
        rows(&factory, "moqui.agent.AgentInput", "clientMessageId", "john-m3").len(),
        1
    );
    let (_, read) = john.get("conversation", &format!("conversationId={conversation_id}"));
    assert_eq!(read["activeRun"]["runId"], slow_run.as_str(), "{read}");
    let (status, _) = ann.post("cancel", &json!({"runId": slow_run}));
    assert_eq!(status, 404, "another user's run is not found");
    let (status, refused) = john.post(
        "retry",
        &json!({"conversationId": conversation_id, "clientId": "tab-b",
        "clientMessageId": "john-retry-2"}),
    );
    assert_eq!(status, 409, "no retry while a run is active: {refused}");
    wait(&factory, &slow_run);

    // cancel stops a run of the user's
    fake::script(
        ENDPOINT,
        vec![fake::Reply::steps(vec![
            Step::Delay(5000),
            Step::Text("Too late.".to_string()),
            Step::Finish("stop".to_string()),
        ])],
    );
    let (_, slow) = john.post(
        "turn",
        &json!({"text": "Something long", "clientMessageId": "john-m4", "clientId": "tab-c"}),
    );
    let cancel_run = str_of(&slow["runId"]);
    let (status, canceled) = john.post("cancel", &json!({"runId": cancel_run}));
    assert_eq!(status, 200, "{canceled}");
    assert_eq!(wait(&factory, &cancel_run), RunState::Canceled);
    fake::clear(ENDPOINT);

    // the tarpit counts ann's runs, not her queued messages: two runs, a queued message, a third run, then refused
    fake::script(
        ENDPOINT,
        vec![
            fake::Reply::text("One."),
            fake::Reply::steps(vec![
                Step::Delay(2000),
                Step::Text("Two.".to_string()),
                Step::Finish("stop".to_string()),
            ]),
            fake::Reply::text("Two, with the message."),
            fake::Reply::text("Three."),
        ],
    );
    let (status, first) = ann.post(
        "turn",
        &json!({"text": "One", "clientMessageId": "ann-1", "clientId": "ann-tab"}),
    );
    assert_eq!(status, 200, "{first}");
    let ann_conversation = str_of(&first["conversationId"]);
    let ann_run = str_of(&first["runId"]);
    wait(&factory, &ann_run);
    let (_, second) = ann.post(
        "turn",
        &json!({"text": "Two", "clientMessageId": "ann-2", "clientId": "ann-tab",
        "conversationId": ann_conversation}),
    );
    let (status, queued) = ann.post(
        "turn",
        &json!({"text": "Two more", "clientMessageId": "ann-2b", "clientId": "ann-tab",
        "conversationId": ann_conversation}),
    );
    assert_eq!((status, queued["queued"].as_bool()), (200, Some(true)), "{queued}");
    wait(&factory, &str_of(&second["runId"]));
    let (status, third) = ann.post(
        "turn",
        &json!({"text": "Three", "clientMessageId": "ann-3", "clientId": "ann-tab"}),
    );
    assert_eq!(
        status, 200,
        "the third run is still let through, and locks the next: {third}"
    );
    wait(&factory, &str_of(&third["runId"]));
    let (status, headers, text) = ann.post_raw(
        "turn",
        &json!({"text": "Four", "clientMessageId": "ann-4",
        "clientId": "ann-tab"}),
        true,
    );
    assert_eq!(status, 429, "{text}");
    assert!(crate::common::header(&headers, "Retry-After").is_some(), "{headers:?}");
    assert_eq!(parse(&text)["code"], "tarpit");
    // a refused turn into a conversation moves neither its follow owner nor its page
    let before = &rows(
        &factory,
        "moqui.agent.AgentConversation",
        "conversationId",
        &ann_conversation,
    )[0];
    let (owner, page) = (field(before, "clientId"), field(before, "screenStateJson"));
    let (status, _, text) = ann.post_raw(
        "turn",
        &json!({"text": "Five", "clientMessageId": "ann-5", "clientId": "ann-other-tab", "conversationId": ann_conversation,
            "contextVersion": 40, "state": {"path": "/apps/system/Security/UserGroup"}}),
        true,
    );
    assert_eq!(status, 429, "{text}");
    let after = &rows(
        &factory,
        "moqui.agent.AgentConversation",
        "conversationId",
        &ann_conversation,
    )[0];
    assert_eq!(
        (field(after, "clientId"), field(after, "screenStateJson")),
        (owner, page)
    );
    assert_ne!(field(after, "contextVersion"), "40");

    // a run that asks for approval: it streams to the pause, the decision is refused for a header login and for
    // another user, taken from the owner's browser session, and the run resumes and writes
    fake::script(
        ENDPOINT,
        vec![
            fake::Reply::steps(vec![
                Step::Tool {
                    id: "u1".to_string(),
                    name: "run_tool".to_string(),
                    arguments: json!({"name": "update_enum_type",
                        "parameters": {"enumTypeId": "AST_TYPE", "description": "After"}})
                    .to_string(),
                    pieces: 1,
                },
                Step::Finish("tool_calls".to_string()),
            ]),
            fake::Reply::text("Changed it."),
        ],
    );
    let (status, started) = john.post(
        "turn",
        &json!({"text": "Change AST_TYPE to After", "clientMessageId": "john-m5",
        "clientId": "tab-a"}),
    );
    assert_eq!(status, 200, "{started}");
    let approval_run = str_of(&started["runId"]);
    let approval_conversation = str_of(&started["conversationId"]);
    let events_path = str_of(&started["eventsPath"]);
    let body = read_stream(
        address,
        &http_request("GET", &events_path, &[("Cookie", &john.cookie)], ""),
        WAIT,
    );
    assert!(
        body.contains("event: approval.requested") && body.contains("TASK_STATE_INPUT_REQUIRED"),
        "{body}"
    );
    assert_eq!(wait(&factory, &approval_run), RunState::InputRequired);
    let last_seq: i64 = body
        .lines()
        .filter_map(|line| line.strip_prefix("id: "))
        .filter_map(|id| id.trim().parse().ok())
        .max()
        .unwrap();
    assert_eq!(description(&factory), "Before");
    let (_, read) = john.get("conversation", &format!("conversationId={approval_conversation}"));
    let card = &read["pending"]["approvals"][0];
    assert_eq!(
        (card["targetName"].as_str(), card["tool"].as_str()),
        (Some("update#moqui.basic.EnumerationType"), Some("run_tool")),
        "{read}"
    );
    let approval_id = str_of(&card["approvalId"]);
    assert_eq!(read["activeRun"]["pauseKind"], "approval");
    // the tasks pane has it waiting; another user's does not
    let (status, tasks) = john.get("tasks", "");
    assert_eq!(status, 200, "{tasks}");
    assert!(
        tasks["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["approvalId"] == approval_id.as_str()),
        "{tasks}"
    );
    let (_, tasks) = ann.get("tasks", "");
    assert!(!tasks.to_string().contains(&approval_id), "{tasks}");
    // the preview: the owner's (when the sim overlay is there), never another user's
    let (status, previewed) = john.post("preview", &json!({"approvalId": approval_id}));
    if boot["features"]["preview"] == true {
        assert_eq!(status, 200, "{previewed}");
    } else {
        assert_ne!(status, 404, "{previewed}");
    }
    let (status, _) = ann.post("preview", &json!({"approvalId": approval_id}));
    assert_eq!(status, 404);
    // a header login may not decide (Basic authentication carries no session token, and no person decided)
    let decision = json!({"approvalId": approval_id, "decision": "approve"});
    let (status, _, text) = exchange(
        address,
        &http_request(
            "POST",
            "/assist/decide",
            &[
                ("Authorization", &basic("john.doe", "moqui")),
                ("Accept", "application/json"),
                ("Content-Type", "application/json"),
            ],
            &decision.to_string(),
        ),
    );
    assert_eq!(status, 403, "{text}");
    assert!(text.contains("decided only by its user"), "{text}");
    let (status, _) = ann.post("decide", &decision);
    assert_eq!(status, 404);
    let (status, _, text) = john.post_raw("decide", &decision, false);
    assert_eq!(status, 401, "the browser session without its token: {text}");
    assert_eq!(description(&factory), "Before");
    let (status, decided) = john.post("decide", &decision);
    assert_eq!(
        (status, decided["status"].as_str()),
        (200, Some("approved")),
        "{decided}"
    );
    assert_eq!(wait(&factory, &approval_run), RunState::Completed);
    assert_eq!(description(&factory), "After", "the approved write ran");
    let (status, decided_again) = john.post("decide", &decision);
    assert_eq!(status, 409, "a second decision: {decided_again}");
    // the resumed run streams from where the client was
    let body = read_stream(
        address,
        &http_request(
            "GET",
            &events_path,
            &[("Cookie", &john.cookie), ("Last-Event-ID", &last_seq.to_string())],
            "",
        ),
        WAIT,
    );
    assert!(
        body.contains("event: approval.decided") && body.contains("Changed it.") && body.contains("event: run.done"),
        "{body}"
    );

    // a question: answered by its run's owner, refused for another user and for a question it does not wait on
    fake::script(
        ENDPOINT,
        vec![
            fake::Reply::steps(vec![
                Step::Tool {
                    id: "q1".to_string(),
                    name: "ask".to_string(),
                    arguments: json!({"question": "Which one?", "options": ["Red", "Blue"]}).to_string(),
                    pieces: 1,
                },
                Step::Finish("tool_calls".to_string()),
            ]),
            fake::Reply::text("Blue it is."),
        ],
    );
    let (_, asked) = john.post(
        "turn",
        &json!({"text": "Pick a color", "clientMessageId": "john-m6", "clientId": "tab-a"}),
    );
    let ask_run = str_of(&asked["runId"]);
    assert_eq!(wait(&factory, &ask_run), RunState::InputRequired);
    let (_, read) = john.get(
        "conversation",
        &format!("conversationId={}", str_of(&asked["conversationId"])),
    );
    let question = &read["pending"]["questions"][0];
    assert_eq!(
        question["options"][0],
        json!({"label": "Red", "recommended": true}),
        "{read}"
    );
    let question_id = str_of(&question["questionId"]);
    let (status, _) = ann.post(
        "answer",
        &json!({"runId": ask_run, "questionId": question_id, "choices": ["Blue"]}),
    );
    assert_eq!(status, 404);
    let (status, _) = john.post(
        "answer",
        &json!({"runId": ask_run, "questionId": "NOT_IT", "choices": ["Blue"]}),
    );
    assert_eq!(status, 409);
    let (status, answered) = john.post(
        "answer",
        &json!({"runId": ask_run, "questionId": question_id, "choices": ["Blue"]}),
    );
    assert_eq!(status, 200, "{answered}");
    assert_eq!(wait(&factory, &ask_run), RunState::Completed);

    // a hand-off's outcome is a signal: the watch's status as the server has it; another user's is not found
    {
        let _authz = context.disable_authz();
        let facade = factory.entity().unwrap();
        let mut watch = facade.make_value("moqui.agent.AgentHandoff").unwrap();
        for (name, value) in [
            ("handoffId", "AST_WATCH"),
            ("runId", approval_run.as_str()),
            ("userId", "EX_JOHN_DOE"),
            ("statusId", "AghWatching"),
        ] {
            watch.set(name, Value::string(value));
        }
        watch.create(&facade).unwrap();
    }
    let (status, signal) = john.post(
        "submitted",
        &json!({"handoffId": "AST_WATCH", "ok": false,
        "errors": ["Description is required"]}),
    );
    assert_eq!((status, signal["status"].as_str()), (200, Some("watching")), "{signal}");
    let (status, _) = ann.post("submitted", &json!({"handoffId": "AST_WATCH", "ok": true}));
    assert_eq!(status, 404);

    // suggested prompts for the page, the @ pickers, and the calls of later phases
    let (status, prompts) = john.get("prompts", "path=/qapps/system/Llm");
    assert_eq!(status, 200, "{prompts}");
    assert_eq!(prompts["prompts"][0]["label"], "Find users", "{prompts}");
    assert_eq!(prompts["prompts"][0]["send"], true);
    let (_, prompts) = john.get("prompts", "path=/qapps/tools");
    assert_eq!(prompts["prompts"].as_array().unwrap().len(), 0, "{prompts}");
    let (status, mentions) = john.get("mentions", "kind=agents&q=assi");
    assert_eq!(
        (status, mentions["items"][0]["agentId"].as_str()),
        (200, Some("assist")),
        "{mentions}"
    );
    let (status, mentions) = john.get("mentions", "kind=skills");
    assert_eq!((status, mentions["kind"].as_str()), (200, Some("skills")), "{mentions}");
    let (status, mentions) = john.get("mentions", "kind=records&q=john");
    assert_eq!(
        (status, mentions["kind"].as_str()),
        (200, Some("records")),
        "{mentions}"
    );
    assert!(mentions["groups"].is_array(), "{mentions}");
    let (status, _) = bob.get("mentions", "kind=records&q=john");
    assert_eq!(status, 403);
    for call in [
        "attachments",
        "transcribe",
        "textAction",
        "canvasState",
        "canvasAction",
        "canvasAdjust",
        "canvasProblems",
    ] {
        let (status, stub) = john.post(call, &json!({}));
        assert_eq!(
            (status, stub["code"].as_str()),
            (501, Some("not_available")),
            "{call}: {stub}"
        );
    }
    let (status, _) = john.get("read", "");
    assert_eq!(status, 501);

    // the admin screens: lists for an admin; another user's content needs AgentAudit, and each such read is logged
    let admin = basic("john.doe", "moqui");
    let screen = |path: &str| {
        let (base, query) = path.split_once('?').unwrap_or((path, ""));
        let target = if query.is_empty() {
            format!("{base}.html")
        } else {
            format!("{base}.html?{query}")
        };
        let (status, _, page) = exchange(address, &http_request("GET", &target, &[("Authorization", &admin)], ""));
        assert!(
            status == 200 && !page.contains("Error rendering"),
            "{path}: {status}: {}",
            page.chars().take(3000).collect::<String>()
        );
        page
    };
    assert!(screen("/System/Llm/Conversations/ConversationList").contains(&ann_conversation));
    assert!(screen("/System/Llm/Runs/RunList").contains(&ann_run));
    assert!(screen("/System/Llm/Approvals/ApprovalList").contains(&approval_id));
    assert!(
        screen(&format!(
            "/System/Llm/Approvals/ApprovalDetail?approvalId={approval_id}"
        ))
        .contains("After")
    );
    assert!(screen(&format!("/System/Llm/Runs/RunDetail?runId={approval_run}")).contains("approval.requested"));
    let audit_rows = || {
        rows(
            &factory,
            "moqui.agent.AgentAuditLog",
            "conversationId",
            &ann_conversation,
        )
        .len()
    };
    let page = screen(&format!(
        "/System/Llm/Conversations/ConversationDetail?conversationId={ann_conversation}"
    ));
    assert!(
        !page.contains("Two more") && page.contains("needs the AgentAudit permission"),
        "{page}"
    );
    let page = screen(&format!("/System/Llm/Runs/RunDetail?runId={ann_run}"));
    assert!(
        !page.contains("One.") && page.contains("needs the AgentAudit permission"),
        "{page}"
    );
    assert_eq!(audit_rows(), 0);
    // the service itself: for an admin without AgentAudit the runs are metadata only, no answer, reason or error text
    {
        let viewer = factory.new_context();
        let _entered = viewer.enter();
        moqui_core::user::internal_login_user("john.doe").unwrap();
        // as the screen calls it: the artifact check is the screen's, the content check the service's own
        let _authz = viewer.disable_authz();
        let parameters = [("conversationId".to_string(), Value::string(&ann_conversation))]
            .into_iter()
            .collect();
        let out =
            moqui_core::service::sync("org.moqui.agent.AgentAdminServices.get#AgentAuditView", &parameters).unwrap();
        assert_eq!(out.get("shown"), Some(&Value::Bool(false)), "{out:?}");
        let Some(Value::List(runs)) = out.get("runList") else {
            panic!("no runList: {out:?}")
        };
        assert!(!runs.is_empty());
        for run in runs {
            let Value::Map(run) = run else { panic!("{run:?}") };
            for field in ["resultJson", "statusReason", "errorText"] {
                assert!(!run.contains_key(field), "{field} of another user's run: {run:?}");
            }
            assert!(run.contains_key("statusId"));
        }
        assert!(!format!("{out:?}").contains("One."), "no answer text: {out:?}");
    }
    {
        let _authz = context.disable_authz();
        let facade = factory.entity().unwrap();
        for (entity, fields) in [
            (
                "moqui.security.UserGroup",
                vec![("userGroupId", "AST_AUDIT"), ("description", "Assist auditors")],
            ),
            (
                "moqui.security.UserGroupMember",
                vec![
                    ("userGroupId", "AST_AUDIT"),
                    ("userId", "EX_JOHN_DOE"),
                    ("fromDate", "2020-01-01 00:00:00"),
                ],
            ),
            (
                "moqui.security.UserGroupPermission",
                vec![
                    ("userGroupId", "AST_AUDIT"),
                    ("userPermissionId", "AgentAudit"),
                    ("fromDate", "2020-01-01 00:00:00"),
                ],
            ),
        ] {
            let mut row = facade.make_value(entity).unwrap();
            for (name, value) in fields {
                row.set(name, Value::string(value));
            }
            row.create(&facade).unwrap();
        }
    }
    let page = screen(&format!(
        "/System/Llm/Conversations/ConversationDetail?conversationId={ann_conversation}"
    ));
    assert!(page.contains("Two more") && page.contains("is logged"), "{page}");
    assert_eq!(audit_rows(), 1);
    let log = &rows(
        &factory,
        "moqui.agent.AgentAuditLog",
        "conversationId",
        &ann_conversation,
    )[0];
    assert_eq!(
        (field(log, "auditorUserId"), field(log, "targetUserId")),
        ("EX_JOHN_DOE".to_string(), "AST_ANN".to_string())
    );
    screen(&format!("/System/Llm/Runs/RunDetail?runId={ann_run}"));
    assert_eq!(audit_rows(), 2, "every read is logged");
    // reading one's own content is not an audit
    screen(&format!(
        "/System/Llm/Conversations/ConversationDetail?conversationId={conversation_id}"
    ));
    assert_eq!(
        rows(
            &factory,
            "moqui.agent.AgentAuditLog",
            "conversationId",
            &conversation_id
        )
        .len(),
        0
    );
}
