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

//! The run engine against the scripted fake provider on the in-memory database (plan 9, Runtime): a streamed answer
//! and its events, tool round trips with parallel reads, a reply cut at the output limit, the empty-reply nudge, the
//! overflow hook, cancel mid-stream and through the lease renewal, the budget pause, steering input, a question that
//! pauses, the read fence, and the leases (a stale run taken over, `UnknownOutcome`, the attempts, one active run).

use std::sync::Arc;
use std::time::{Duration, Instant};

use moqui_lib::object::Value;
use serde_json::{json, Value as Json};

use super::*;
use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;
use crate::llm::provider::fake::{self, Step};
use crate::llm::tools::tests::{calls, TEST_CAP};

const USER: &str = "run.tester";
const WAIT: Duration = Duration::from_secs(15);

struct Bed {
    ctx: ExecutionContext,
    factory: Arc<ExecutionContextFactory>,
    facade: EntityFacade,
    endpoint: String,
}

fn bed(name: &str) -> Bed {
    bed_with(name, "", "")
}

/// [`bed`] with more of the conf: `tools` (the request and service pools) and llm-facade `limits`.
fn bed_with(name: &str, tools: &str, limits: &str) -> Bed {
    let endpoint = format!("run-{name}");
    let conf = format!(
        r#"<moqui-conf>{tools}<llm-facade>{limits}
            <endpoint id="{endpoint}" protocol="fake"/>
            <model id="{endpoint}" endpoint="{endpoint}" name="fake-1" context-window="32768" price-input="2" price-output="8"/>
            <bind feature="assist" model="{endpoint}" reasoning="none"/>
        </llm-facade></moqui-conf>"#
    );
    let ctx = ExecutionContextFactory::test_context_in(
        &conf,
        crate::service::ServicePools::idle(),
        crate::context::framework_dir(),
    );
    let facade = crate::testing::open();
    ctx.factory().set_entity(facade.clone());
    fake::clear(&endpoint);
    row(
        &facade,
        "moqui.security.UserAccount",
        &[("userId", USER), ("username", USER), ("userFullName", "Run Tester")],
    );
    row(
        &facade,
        "moqui.agent.AgentDefinition",
        &[
            ("agentId", "assist"),
            ("agentName", "Assist"),
            ("featureId", "assist"),
            ("surfaces", "drawer,chat,job,tui"),
        ],
    );
    Bed {
        factory: Arc::clone(ctx.factory()),
        ctx,
        facade,
        endpoint,
    }
}

fn row(facade: &EntityFacade, entity: &str, fields: &[(&str, &str)]) {
    let mut value = facade.make_value(entity).unwrap();
    for (field, text) in fields {
        value.set(*field, Value::string(*text));
    }
    value.create(facade).unwrap();
}

fn request(text: &str) -> StartRequest {
    StartRequest {
        user_id: USER.to_string(),
        text: text.to_string(),
        extra_caps: vec![TEST_CAP.to_string()],
        ..StartRequest::default()
    }
}

fn tool(id: &str, name: &str, arguments: Json) -> Step {
    Step::Tool {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
        pieces: 2,
    }
}

fn tools(steps: Vec<Step>) -> fake::Reply {
    let mut steps = steps;
    steps.push(Step::Finish("tool_calls".to_string()));
    fake::Reply::steps(steps)
}

fn stored(bed: &Bed, run_id: &str) -> Vec<StoredEvent> {
    events::stored_after(&bed.facade, run_id, 0, 10_000).unwrap()
}

fn types(events: &[StoredEvent]) -> Vec<&str> {
    events.iter().map(|event| event.event_type.as_str()).collect()
}

fn run_row(bed: &Bed, run_id: &str) -> crate::entity::EntityValue {
    bed.facade
        .find(RUN)
        .unwrap()
        .eq("runId", Value::string(run_id))
        .one()
        .unwrap()
        .unwrap()
}

fn call_statuses(bed: &Bed, run_id: &str) -> Vec<(String, String)> {
    bed.facade
        .find(TOOL_CALL)
        .unwrap()
        .eq("runId", Value::string(run_id))
        .order_by("toolCallId", false)
        .list()
        .unwrap()
        .values()
        .iter()
        .map(|row| {
            (
                text_of(row, "toolName").unwrap_or_default(),
                text_of(row, "statusId").unwrap_or_default(),
            )
        })
        .collect()
}

fn roles(request: &Json) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["role"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn a_run_streams_its_answer_as_numbered_events_and_completes() {
    let bed = bed("stream");
    let _entered = bed.ctx.enter();
    let answer = "Hello there. ".repeat(50);
    fake::script(&bed.endpoint, vec![fake::Reply::text(&answer)]);
    let started = start_run(&bed.factory, &request("Say hello")).unwrap();
    assert_eq!(started.state, RunState::Submitted);
    assert!(!started.queued);
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Completed
    );

    let events = stored(&bed, &started.run_id);
    let seqs: Vec<i64> = events.iter().map(|event| event.seq).collect();
    assert_eq!(
        seqs,
        (1..=i64::try_from(events.len()).unwrap()).collect::<Vec<_>>(),
        "numbered from 1 with no gap"
    );
    let kinds = types(&events);
    assert_eq!(kinds.first(), Some(&"run.state"));
    assert_eq!(kinds.last(), Some(&"run.done"));
    for kind in ["message.start", "text.chunk", "part.done", "usage"] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    // the chunks are the answer, each at its offset, at most 512 characters a chunk past the first delta
    let chunks: Vec<&StoredEvent> = events.iter().filter(|event| event.event_type == "text.chunk").collect();
    assert!(
        chunks.len() >= 2,
        "{} characters in {} chunks",
        answer.len(),
        chunks.len()
    );
    let mut text = String::new();
    for chunk in &chunks {
        assert_eq!(chunk.payload["offset"].as_u64().unwrap() as usize, text.chars().count());
        text.push_str(chunk.payload["text"].as_str().unwrap());
    }
    assert_eq!(text, answer);
    let done = events.iter().find(|event| event.event_type == "part.done").unwrap();
    assert_eq!(
        done.payload["length"].as_u64().unwrap() as usize,
        answer.chars().count()
    );

    // the live deltas went to the hub with their offsets, the stored events after them
    let hub = engine(&bed.factory).hub();
    let live = hub.wait(&started.run_id, None, Duration::ZERO);
    let deltas: Vec<&hub::HubEvent> = live.iter().filter(|event| event.event_type == "text.delta").collect();
    assert!(!deltas.is_empty());
    assert!(deltas
        .iter()
        .all(|event| event.seq.is_none() && event.payload.get("offset").is_some()));
    assert_eq!(hub.last_seq(&started.run_id), *seqs.last().unwrap());

    let run = run_row(&bed, &started.run_id);
    assert_eq!(text_of(&run, "statusId").as_deref(), Some("AgrCompleted"));
    assert!(text_of(&run, "leaseOwner").is_none());
    assert_eq!(int_of(&run, "stepCount"), Some(1));
    assert!(text_of(&run, "resultJson").unwrap().contains("Hello there."));
    let conversation = bed
        .facade
        .find(CONVERSATION)
        .unwrap()
        .eq("conversationId", Value::string(&started.conversation_id))
        .one()
        .unwrap()
        .unwrap();
    assert!(text_of(&conversation, "activeRunId").is_none());
    assert!(text_of(&conversation, "systemText").is_some_and(|text| text.contains("Run Tester")));
    let step = bed
        .facade
        .find("moqui.agent.AgentRunStep")
        .unwrap()
        .eq("runId", Value::string(&started.run_id))
        .one()
        .unwrap()
        .unwrap();
    let call = bed
        .facade
        .find("moqui.llm.LlmCall")
        .unwrap()
        .eq("callId", Value::string(text_of(&step, "callId").unwrap()))
        .one()
        .unwrap()
        .unwrap();
    assert_eq!(text_of(&call, "runId").as_deref(), Some(started.run_id.as_str()));
    // the same request again is the same run
    let again = start_run(
        &bed.factory,
        &StartRequest {
            client_request_id: Some("req-1".to_string()),
            ..request("once")
        },
    )
    .unwrap();
    wait_for(&bed.factory, &again.run_id, WAIT).unwrap();
    let repeated = start_run(
        &bed.factory,
        &StartRequest {
            client_request_id: Some("req-1".to_string()),
            ..request("once")
        },
    )
    .unwrap();
    assert!(repeated.existing && repeated.run_id == again.run_id);
}

#[test]
fn tool_calls_run_in_model_order_with_independent_reads_in_parallel() {
    // the in-memory pool (8, 2 reserve) with one request and one service worker leaves 2 connections: one agent
    // worker and one more read beside its own
    let bed = bed_with(
        "tools",
        r#"<tools request-pool-max="1" worker-pool-max="1"/>"#,
        r#"<limits max-concurrent-runs="1"/>"#,
    );
    let _entered = bed.ctx.enter();
    assert_eq!(engine(&bed.factory).parallel_reads(), 2);
    // with the default request and service workers there is no room, and reads run one at a time
    assert_eq!(engine(&self::bed("tools-small").factory).parallel_reads(), 1);
    fake::script(
        &bed.endpoint,
        vec![
            tools(vec![
                tool("c1", "test_read", json!({"key": "par-a", "waitMs": 400})),
                tool("c2", "test_read", json!({"key": "par-b", "waitMs": 400})),
                tool("c3", "test_write", json!({"key": "par-w"})),
                tool("c4", "no_such_tool", json!({})),
            ]),
            fake::Reply::text("All done."),
        ],
    );
    let started = start_run(&bed.factory, &request("Read and write")).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    let (a, b) = (calls("par-a"), calls("par-b"));
    assert_eq!((a.len(), b.len(), calls("par-w").len()), (1, 1, 1));
    assert!(a[0].0 < b[0].1 && b[0].0 < a[0].1, "the two reads overlapped");
    assert!(
        calls("par-w")[0].0 >= a[0].1.max(b[0].1),
        "the write ran after the reads"
    );
    assert_eq!(
        call_statuses(&bed, &started.run_id),
        [
            ("test_read".to_string(), "AgtcDone".to_string()),
            ("test_read".to_string(), "AgtcDone".to_string()),
            ("test_write".to_string(), "AgtcDone".to_string()),
            ("no_such_tool".to_string(), "AgtcFailed".to_string()),
        ]
    );
    let sent = fake::requests(&bed.endpoint);
    assert_eq!(sent.len(), 2);
    assert_eq!(roles(&sent[1]), ["user", "assistant", "tool", "tool", "tool", "tool"]);
    let results: Vec<&str> = sent[1]["messages"].as_array().unwrap()[2..]
        .iter()
        .map(|message| message["tool_call_id"].as_str().unwrap())
        .collect();
    assert_eq!(results, ["c1", "c2", "c3", "c4"], "results in model order");
    assert!(sent[1]["messages"][5]["text"]
        .as_str()
        .unwrap()
        .contains("There is no tool named no_such_tool"));
    let all = stored(&bed, &started.run_id);
    let kinds = types(&all)
        .into_iter()
        .filter(|kind| kind.starts_with("tool."))
        .collect::<Vec<_>>();
    assert_eq!(kinds.iter().filter(|kind| **kind == "tool.call").count(), 4);
    assert_eq!(kinds.iter().filter(|kind| **kind == "tool.result").count(), 4);
    assert_eq!(int_of(&run_row(&bed, &started.run_id), "toolCallCount"), Some(3));
}

#[test]
fn a_reply_cut_at_the_limit_keeps_its_complete_calls_and_drops_the_cut_one() {
    let bed = bed("length");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![
            fake::Reply::steps(vec![
                tool("l1", "test_read", json!({"key": "len-a"})),
                Step::Tool {
                    id: "l2".to_string(),
                    name: "test_read".to_string(),
                    arguments: "{\"key\": \"len-b".to_string(),
                    pieces: 1,
                },
                Step::Finish("length".to_string()),
            ]),
            fake::Reply::text("Read one."),
        ],
    );
    let started = start_run(&bed.factory, &request("Read two")).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    assert_eq!((calls("len-a").len(), calls("len-b").len()), (1, 0));
    assert_eq!(call_statuses(&bed, &started.run_id).len(), 1);
    assert_eq!(
        fake::requests(&bed.endpoint).len(),
        2,
        "no re-ask: a complete call was kept"
    );
}

#[test]
fn an_empty_reply_is_nudged_once_and_an_overflow_goes_to_the_compaction_hook() {
    let bed = bed("nudge");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![
            fake::Reply::steps(vec![Step::Finish("stop".to_string())]),
            fake::Reply::text("Here it is."),
        ],
    );
    let first = start_run(&bed.factory, &request("Answer me")).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &first.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    let sent = fake::requests(&bed.endpoint);
    assert_eq!(sent.len(), 2);
    assert!(sent[1].to_string().contains("Your last reply was empty"));
    assert!(text_of(&run_row(&bed, &first.run_id), "resultJson")
        .unwrap()
        .contains("Here it is."));

    // the second run of the conversation overflows once; the stub compaction keeps the newest turn
    fake::script(
        &bed.endpoint,
        vec![
            fake::Reply::steps(vec![Step::Finish("context_overflow".to_string())]),
            fake::Reply::text("Shorter."),
        ],
    );
    let second = start_run(
        &bed.factory,
        &StartRequest {
            conversation_id: Some(first.conversation_id.clone()),
            ..request("And again")
        },
    )
    .unwrap();
    assert_eq!(
        wait_for(&bed.factory, &second.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    let sent = fake::requests(&bed.endpoint);
    assert_eq!(roles(&sent[2]), ["user", "assistant", "user"]);
    assert_eq!(roles(&sent[3]), ["user"]);
    assert!(types(&stored(&bed, &second.run_id)).contains(&"compaction"));
    // both runs sent the conversation's frozen prefix, byte for byte
    assert!(sent
        .iter()
        .all(|body| body["system"] == sent[0]["system"] && body["tools"] == sent[0]["tools"]));
}

#[test]
fn a_cancel_mid_stream_stops_the_run_and_keeps_the_partial_answer() {
    let bed = bed("cancel");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![fake::Reply::steps(vec![
            Step::Text("Partial answer".to_string()),
            Step::Delay(8000),
            Step::Text(" never sent".to_string()),
            Step::Finish("stop".to_string()),
        ])],
    );
    let started = start_run(&bed.factory, &request("Talk slowly")).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let asked = Instant::now();
    cancel_run(&bed.factory, &started.run_id, Some(USER)).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Canceled
    );
    assert!(asked.elapsed() < Duration::from_secs(4), "{:?}", asked.elapsed());
    let (history, _) = step::load_history(&bed.facade, &started.conversation_id).unwrap();
    assert_eq!(history.last().unwrap().text, "Partial answer");
    // another user cannot cancel it
    assert!(cancel_run(&bed.factory, &started.run_id, Some("someone.else")).is_err());
}

#[test]
fn a_cancel_from_another_node_reaches_the_run_through_the_lease_renewal() {
    let bed = bed("renewal");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![fake::Reply::steps(vec![
            Step::Text("Working on it".to_string()),
            Step::Delay(8000),
            Step::Finish("stop".to_string()),
        ])],
    );
    let started = start_run(&bed.factory, &request("Take your time")).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    // another node sets the flag in the database; this node's token knows nothing yet
    bed.facade
        .find(RUN)
        .unwrap()
        .eq("runId", Value::string(&started.run_id))
        .update_all([("cancelRequested".to_string(), Value::string("Y"))])
        .unwrap();
    let renewal = lease::renew(&bed.factory).unwrap();
    assert_eq!(renewal.canceled, std::slice::from_ref(&started.run_id));
    assert!(renewal.renewed >= 1);
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Canceled
    );
}

#[test]
fn an_interactive_run_pauses_at_its_budget_and_a_background_one_fails() {
    let bed = bed("budget");
    let _entered = bed.ctx.enter();
    let script = || {
        vec![
            tools(vec![tool("b1", "test_read", json!({"key": "budget"}))]),
            fake::Reply::text("Finished after the continue."),
        ]
    };
    fake::script(&bed.endpoint, script());
    let limited = |surface| StartRequest {
        surface,
        limits: RunLimits {
            max_steps: Some(1),
            ..RunLimits::default()
        },
        ..request("Use one step")
    };
    let started = start_run(&bed.factory, &limited(Surface::Drawer)).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::InputRequired
    );
    let run = load_run(&bed.facade, &started.run_id).unwrap().unwrap();
    assert_eq!(run.pause_kind, Some(PauseKind::Budget));
    let question = bed
        .facade
        .find(QUESTION)
        .unwrap()
        .eq("questionId", Value::string(run.pause_ref.clone().unwrap()))
        .one()
        .unwrap()
        .unwrap();
    assert!(text_of(&question, "questionText").unwrap().contains("steps"));
    assert!(types(&stored(&bed, &started.run_id)).contains(&"question.asked"));
    assert!(
        text_of(&run_row(&bed, &started.run_id), "leaseOwner").is_none(),
        "a paused run holds no lease"
    );

    submit_input(&bed.factory, &started.run_id, USER, "answer", "continue", None).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    assert!(int_of(&run_row(&bed, &started.run_id), "maxSteps").unwrap() > 1);

    fake::script(&bed.endpoint, script());
    let job = start_run(&bed.factory, &limited(Surface::Job)).unwrap();
    assert_eq!(wait_for(&bed.factory, &job.run_id, WAIT).unwrap(), RunState::Failed);
    assert_eq!(
        text_of(&run_row(&bed, &job.run_id), "errorCode").as_deref(),
        Some("budget_exhausted")
    );
}

#[test]
fn a_message_sent_while_a_run_works_is_folded_in_at_its_next_step() {
    let bed = bed("steer");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![
            tools(vec![tool("s1", "test_read", json!({"key": "steer", "waitMs": 800}))]),
            fake::Reply::text("Both checked."),
        ],
    );
    let first = start_run(&bed.factory, &request("Check A")).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let steering = start_run(
        &bed.factory,
        &StartRequest {
            conversation_id: Some(first.conversation_id.clone()),
            ..request("Also check B")
        },
    )
    .unwrap();
    assert!(steering.queued);
    assert_eq!(steering.run_id, first.run_id);
    assert_eq!(
        wait_for(&bed.factory, &first.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    let sent = fake::requests(&bed.endpoint);
    assert_eq!(roles(&sent[1]), ["user", "assistant", "tool", "user"]);
    assert_eq!(sent[1]["messages"][3]["text"], "Also check B");
    assert!(types(&stored(&bed, &first.run_id)).contains(&"input.folded"));
}

#[test]
fn a_tool_that_asks_pauses_the_run_and_the_answer_resumes_it() {
    let bed = bed("ask");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![
            tools(vec![
                tool("a1", "test_ask", json!({"key": "color"})),
                tool("a2", "test_read", json!({"key": "after-ask"})),
            ]),
            fake::Reply::text("Blue it is."),
        ],
    );
    let started = start_run(&bed.factory, &request("Ask me")).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::InputRequired
    );
    let run = load_run(&bed.facade, &started.run_id).unwrap().unwrap();
    assert_eq!(
        (run.pause_kind, run.pause_ref.as_deref()),
        (Some(PauseKind::Question), Some("color"))
    );
    assert!(calls("after-ask").is_empty(), "the call after the paused one waits");
    submit_input(&bed.factory, &started.run_id, USER, "answer", "blue", None).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    assert_eq!(calls("after-ask").len(), 1);
    let sent = fake::requests(&bed.endpoint);
    assert_eq!(sent[1]["messages"][2]["text"], "The user answered: blue");
}

#[test]
fn a_read_tool_that_writes_is_stopped_by_the_fence() {
    let bed = bed("fence");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![
            tools(vec![tool("f1", "test_sneaky", json!({"key": "SNEAKY_ENUM"}))]),
            fake::Reply::text("Hm."),
        ],
    );
    let started = start_run(&bed.factory, &request("Look")).unwrap();
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::Completed
    );
    assert_eq!(
        call_statuses(&bed, &started.run_id),
        [("test_sneaky".to_string(), "AgtcFailed".to_string())]
    );
    let sent = fake::requests(&bed.endpoint);
    let result = sent[1]["messages"][2]["text"].as_str().unwrap();
    assert!(
        result.contains("read tool") && result.contains("moqui.basic.Enumeration"),
        "{result}"
    );
    let written = bed
        .facade
        .find("moqui.basic.Enumeration")
        .unwrap()
        .eq("enumId", Value::string("SNEAKY_ENUM"))
        .one()
        .unwrap();
    assert!(written.is_none());
}

/// A run another node was working when it stopped: Working, its lease lapsed, two calls left running and one waiting.
fn stranded(bed: &Bed, attempt: &str, keys: [&str; 3]) -> (String, [String; 3]) {
    let facade = &bed.facade;
    let conversation_id = format!("conv-{}", keys[0]);
    let run_id = format!("run-{}", keys[0]);
    let past = stamp(now_millis() - 5_000);
    row(
        facade,
        CONVERSATION,
        &[
            ("conversationId", &conversation_id),
            ("userId", USER),
            ("agentId", "assist"),
            ("statusId", "AgcActive"),
            ("activeRunId", &run_id),
            ("lastMessageSeq", "0"),
        ],
    );
    let caps = crate::llm::tools::ClientCaps::for_surface(Surface::Drawer)
        .with([TEST_CAP])
        .to_text();
    row(
        facade,
        RUN,
        &[
            ("runId", &run_id),
            ("conversationId", &conversation_id),
            ("agentId", "assist"),
            ("userId", USER),
            ("surfaceEnumId", "AgsfDrawer"),
            ("capsText", &caps),
            ("statusId", "AgrWorking"),
            ("leaseOwner", "gone-node"),
            ("attempt", attempt),
            ("stepCount", "1"),
            ("maxSteps", "10"),
            ("lastEventSeq", "0"),
        ],
    );
    let mut run = facade
        .find(RUN)
        .unwrap()
        .eq("runId", Value::string(&run_id))
        .one()
        .unwrap()
        .unwrap();
    run.set("leaseExpireDate", past);
    run.update(facade).unwrap();
    step::append_message(
        facade,
        &conversation_id,
        Some(&run_id),
        "user",
        Some(USER),
        None,
        &[step::Part::text("Do three things")],
        None,
        None,
    )
    .unwrap();
    let specs = [
        ("test_read", keys[0], "AgtcRunning"),
        ("test_write", keys[1], "AgtcRunning"),
        ("test_read", keys[2], "AgtcPending"),
    ];
    let mut ids = Vec::new();
    let mut parts = Vec::new();
    for (index, (name, key, status)) in specs.iter().enumerate() {
        let id = format!("tc-{key}");
        let arguments = json!({"key": key});
        row(
            facade,
            TOOL_CALL,
            &[
                ("toolCallId", &id),
                ("providerCallId", &format!("p{index}")),
                ("runId", &run_id),
                ("stepSeq", "1"),
                ("toolName", name),
                ("argumentsJson", &arguments.to_string()),
                ("statusId", status),
            ],
        );
        parts.push(step::Part {
            kind: "AgpkToolCall",
            data: Some(json!({"id": format!("p{index}"), "name": name, "arguments": arguments})),
            tool_call_id: Some(id.clone()),
            ..step::Part::default()
        });
        ids.push(id);
    }
    step::append_message(
        facade,
        &conversation_id,
        Some(&run_id),
        "assistant",
        None,
        None,
        &parts,
        None,
        None,
    )
    .unwrap();
    (run_id, [ids[0].clone(), ids[1].clone(), ids[2].clone()])
}

#[test]
fn a_stale_run_is_taken_over_and_a_write_left_running_is_an_unknown_outcome() {
    let bed = bed("stale");
    let _entered = bed.ctx.enter();
    fake::script(&bed.endpoint, vec![fake::Reply::text("Checked what happened.")]);
    let (run_id, [read, write, next]) = stranded(&bed, "1", ["uo-read", "uo-write", "uo-next"]);
    // a run whose lease another node still holds is left alone
    row(
        &bed.facade,
        RUN,
        &[
            ("runId", "run-held"),
            ("userId", USER),
            ("agentId", "assist"),
            ("statusId", "AgrWorking"),
            ("leaseOwner", "live-node"),
        ],
    );
    let mut held = run_row(&bed, "run-held");
    held.set("leaseExpireDate", stamp(now_millis() + 60_000));
    held.update(&bed.facade).unwrap();

    assert_eq!(lease::sweep(&bed.factory).unwrap(), 1);
    assert_eq!(wait_for(&bed.factory, &run_id, WAIT).unwrap(), RunState::Completed);
    assert_eq!(
        (calls("uo-read").len(), calls("uo-write").len(), calls("uo-next").len()),
        (1, 0, 1)
    );
    let status = |id: &str| {
        text_of(
            &bed.facade
                .find(TOOL_CALL)
                .unwrap()
                .eq("toolCallId", Value::string(id))
                .one()
                .unwrap()
                .unwrap(),
            "statusId",
        )
        .unwrap()
    };
    assert_eq!(
        (status(&read).as_str(), status(&write).as_str(), status(&next).as_str()),
        ("AgtcDone", "AgtcUnknownOutcome", "AgtcDone")
    );
    let sent = fake::requests(&bed.endpoint);
    assert!(sent[0]["messages"][3]["text"]
        .as_str()
        .unwrap()
        .contains("outcome of this test_write call is unknown"));
    assert_eq!(int_of(&run_row(&bed, &run_id), "attempt"), Some(2));
    assert_eq!(
        text_of(&run_row(&bed, "run-held"), "leaseOwner").as_deref(),
        Some("live-node")
    );
}

#[test]
fn a_run_past_its_attempts_fails_and_a_lease_is_held_by_one_node() {
    let bed = bed("attempts");
    let _entered = bed.ctx.enter();
    let (run_id, _) = stranded(&bed, "3", ["at-read", "at-write", "at-next"]);
    assert_eq!(lease::sweep(&bed.factory).unwrap(), 1);
    let run = run_row(&bed, &run_id);
    assert_eq!(text_of(&run, "statusId").as_deref(), Some("AgrFailed"));
    assert_eq!(text_of(&run, "errorCode").as_deref(), Some("recovery_exhausted"));
    // its calls were closed, so the conversation's history has a result for each
    let (history, _) = step::load_history(&bed.facade, "conv-at-read").unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|message| message.role == crate::llm::provider::Role::Tool)
            .count(),
        3
    );
    assert!(calls("at-read").is_empty() && calls("at-write").is_empty());

    row(
        &bed.facade,
        RUN,
        &[("runId", "run-lease"), ("userId", USER), ("statusId", "AgrWorking")],
    );
    assert!(lease::claim(&bed.facade, "run-lease", "node-a", 60_000).unwrap());
    assert!(
        lease::claim(&bed.facade, "run-lease", "node-a", 60_000).unwrap(),
        "a renewal"
    );
    assert!(!lease::claim(&bed.facade, "run-lease", "node-b", 60_000).unwrap());
    lease::release(&bed.facade, "run-lease", "node-a").unwrap();
    assert!(lease::claim(&bed.facade, "run-lease", "node-b", 60_000).unwrap());
}

#[test]
fn a_conversation_has_one_active_run() {
    let bed = bed("active");
    let _entered = bed.ctx.enter();
    row(
        &bed.facade,
        CONVERSATION,
        &[("conversationId", "conv-one"), ("userId", USER)],
    );
    assert!(take_active(&bed.facade, "conv-one", "run-1").unwrap());
    assert!(
        take_active(&bed.facade, "conv-one", "run-1").unwrap(),
        "the run that has it"
    );
    assert!(!take_active(&bed.facade, "conv-one", "run-2").unwrap());
    clear_active(&bed.facade, "conv-one", "run-2").unwrap();
    assert!(
        !take_active(&bed.facade, "conv-one", "run-2").unwrap(),
        "only the run that has it clears it"
    );
    clear_active(&bed.facade, "conv-one", "run-1").unwrap();
    assert!(take_active(&bed.facade, "conv-one", "run-2").unwrap());
    // another user's conversation is not found
    let error = start_run(
        &bed.factory,
        &StartRequest {
            user_id: "someone.else".to_string(),
            conversation_id: Some("conv-one".to_string()),
            ..request("hi")
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("no agent conversation"), "{error}");
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_node_that_lost_the_lease_mid_step_writes_nothing_more_and_the_new_holder_completes() {
    let bed = bed("lost");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![
            tools(vec![tool(
                "x1",
                "test_read",
                json!({"key": "lost-read", "waitMs": 1200}),
            )]),
            fake::Reply::text("Done after the takeover."),
        ],
    );
    let started = start_run(&bed.factory, &request("Read slowly")).unwrap();
    let run_id = started.run_id.clone();
    wait_until("the call to run", || {
        call_statuses(&bed, &run_id) == [("test_read".to_string(), "AgtcRunning".to_string())]
    });
    // node B takes the lease while this node is inside the tool
    let mut row = run_row(&bed, &run_id);
    row.set("leaseOwner", Value::string("node-b"));
    row.set("leaseExpireDate", stamp(now_millis() + 60_000));
    row.update(&bed.facade).unwrap();
    let renewal = lease::renew(&bed.factory).unwrap();
    assert_eq!(renewal.lost, std::slice::from_ref(&run_id));
    let events_then = stored(&bed, &run_id).len();
    wait_until("the old step to stop", || !engine(&bed.factory).is_local(&run_id));
    assert_eq!(calls("lost-read").len(), 1, "the tool itself finished");
    // nothing more from this node: the call is not closed, no result, no event, the state is node B's
    assert_eq!(
        call_statuses(&bed, &run_id),
        [("test_read".to_string(), "AgtcRunning".to_string())]
    );
    let (history, _) = step::load_history(&bed.facade, &started.conversation_id).unwrap();
    assert!(history
        .iter()
        .all(|message| message.role != crate::llm::provider::Role::Tool));
    assert_eq!(stored(&bed, &run_id).len(), events_then);
    let run = run_row(&bed, &run_id);
    assert_eq!(text_of(&run, "statusId").as_deref(), Some("AgrWorking"));
    assert_eq!(text_of(&run, "leaseOwner").as_deref(), Some("node-b"));
    assert_eq!(fake::requests(&bed.endpoint).len(), 1);

    // node B stops in turn; this node is the new holder, re-runs the read and completes
    let mut row = run_row(&bed, &run_id);
    row.set("leaseExpireDate", stamp(now_millis() - 1_000));
    row.update(&bed.facade).unwrap();
    assert_eq!(lease::sweep(&bed.factory).unwrap(), 1);
    assert_eq!(wait_for(&bed.factory, &run_id, WAIT).unwrap(), RunState::Completed);
    assert_eq!(calls("lost-read").len(), 2);
    assert_eq!(
        call_statuses(&bed, &run_id),
        [("test_read".to_string(), "AgtcDone".to_string())]
    );
    let results = stored(&bed, &run_id)
        .iter()
        .filter(|event| event.event_type == "tool.result")
        .count();
    assert_eq!(results, 1, "only the new holder's result");
}

#[test]
fn a_write_needs_the_lease_and_a_lost_node_cannot_end_the_run() {
    let bed = bed("hold");
    let _entered = bed.ctx.enter();
    row(
        &bed.facade,
        RUN,
        &[
            ("runId", "run-hold"),
            ("userId", USER),
            ("statusId", "AgrWorking"),
            ("leaseOwner", "node-b"),
        ],
    );
    let node = engine(&bed.factory).node().to_string();
    let mut sink = EventSink::new(&bed.factory, &bed.facade, "run-hold", USER);
    let refused = transition(
        &bed.facade,
        &mut sink,
        "run-hold",
        RunState::Canceled,
        &Change::reason("canceled"),
    )
    .unwrap_err();
    assert!(is_lease_lost(&refused), "{refused}");
    sink.emit("error", json!({}));
    assert!(sink.flush().is_err_and(|error| is_lease_lost(&error)));
    assert_eq!(
        text_of(&run_row(&bed, "run-hold"), "statusId").as_deref(),
        Some("AgrWorking")
    );
    assert!(stored(&bed, "run-hold").is_empty());
    assert!(in_tx(&bed.facade, || held(&bed.facade, "run-hold", "node-b", Hold::Lease)).is_ok());
    assert!(in_tx(&bed.facade, || held(&bed.facade, "run-hold", &node, Hold::LeaseOrNone)).is_err());
}

#[test]
fn an_agent_serves_only_the_surfaces_it_names_and_run_as_is_for_internal_callers() {
    let bed = bed("surfaces");
    let _entered = bed.ctx.enter();
    row(
        &bed.facade,
        "moqui.security.UserAccount",
        &[("userId", "job.runner"), ("username", "job.runner")],
    );
    row(
        &bed.facade,
        "moqui.agent.AgentDefinition",
        &[("agentId", "plain"), ("featureId", "assist")],
    );
    row(
        &bed.facade,
        "moqui.agent.AgentDefinition",
        &[
            ("agentId", "jobber"),
            ("featureId", "assist"),
            ("surfaces", "job"),
            ("runAsUserId", "job.runner"),
        ],
    );
    let plain = AgentDef::load(&bed.facade, "plain").unwrap().unwrap();
    assert!(plain.serves(Surface::Drawer) && plain.serves(Surface::Tui));
    assert!(!plain.serves(Surface::Job) && !plain.serves(Surface::Chat) && !plain.serves(Surface::Mcp));
    let refused = start_run(
        &bed.factory,
        &StartRequest {
            agent_id: "plain".to_string(),
            surface: Surface::Job,
            ..request("hi")
        },
    )
    .unwrap_err();
    assert!(
        refused.to_string().contains("does not serve the job surface"),
        "{refused}"
    );
    for (flag, expected) in [(false, None), (true, Some("job.runner"))] {
        let started = start_run(
            &bed.factory,
            &StartRequest {
                agent_id: "jobber".to_string(),
                surface: Surface::Job,
                run_as_agent_user: flag,
                ..request("nightly")
            },
        )
        .unwrap();
        assert_eq!(
            text_of(&run_row(&bed, &started.run_id), "runAsUserId").as_deref(),
            expected
        );
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap();
    }
}

#[test]
fn a_parallel_batch_keeps_to_the_tool_call_cap() {
    let bed = bed("batchcap");
    let _entered = bed.ctx.enter();
    fake::script(
        &bed.endpoint,
        vec![tools(vec![
            tool("q1", "test_read", json!({"key": "cap-a"})),
            tool("q2", "test_read", json!({"key": "cap-b"})),
            tool("q3", "test_read", json!({"key": "cap-c"})),
        ])],
    );
    let started = start_run(
        &bed.factory,
        &StartRequest {
            limits: RunLimits {
                max_tool_calls: Some(2),
                ..RunLimits::default()
            },
            ..request("Read three")
        },
    )
    .unwrap();
    // the cap is reached: the interactive run asks to go on
    assert_eq!(
        wait_for(&bed.factory, &started.run_id, WAIT).unwrap(),
        RunState::InputRequired
    );
    assert_eq!(
        (calls("cap-a").len(), calls("cap-b").len(), calls("cap-c").len()),
        (1, 1, 0)
    );
    assert_eq!(int_of(&run_row(&bed, &started.run_id), "toolCallCount"), Some(2));
    let statuses: Vec<String> = call_statuses(&bed, &started.run_id)
        .into_iter()
        .map(|(_, status)| status)
        .collect();
    assert_eq!(statuses, ["AgtcDone", "AgtcDone", "AgtcFailed"]);
}

#[test]
fn a_pause_past_its_expiry_fails_the_run_at_the_sweep() {
    let bed = bed("expiry");
    let _entered = bed.ctx.enter();
    row(
        &bed.facade,
        QUESTION,
        &[
            ("questionId", "q-old"),
            ("runId", "run-paused"),
            ("statusId", "AgqOpen"),
        ],
    );
    row(
        &bed.facade,
        RUN,
        &[
            ("runId", "run-paused"),
            ("userId", USER),
            ("statusId", "AgrInputRequired"),
            ("pauseKindEnumId", "AgpsQuestion"),
            ("pauseRef", "q-old"),
        ],
    );
    row(
        &bed.facade,
        RUN,
        &[
            ("runId", "run-waiting"),
            ("userId", USER),
            ("statusId", "AgrInputRequired"),
        ],
    );
    let mut paused = run_row(&bed, "run-paused");
    paused.set("pauseExpireDate", stamp(now_millis() - 1_000));
    paused.update(&bed.facade).unwrap();
    let mut waiting = run_row(&bed, "run-waiting");
    waiting.set("pauseExpireDate", stamp(now_millis() + 60_000));
    waiting.update(&bed.facade).unwrap();
    assert_eq!(lease::expire_pauses(&bed.factory).unwrap(), 1);
    let ended = run_row(&bed, "run-paused");
    assert_eq!(text_of(&ended, "statusId").as_deref(), Some("AgrFailed"));
    assert_eq!(text_of(&ended, "errorCode").as_deref(), Some("pause_expired"));
    let question = bed
        .facade
        .find(QUESTION)
        .unwrap()
        .eq("questionId", Value::string("q-old"))
        .one()
        .unwrap()
        .unwrap();
    assert_eq!(text_of(&question, "statusId").as_deref(), Some("AgqExpired"));
    assert_eq!(
        text_of(&run_row(&bed, "run-waiting"), "statusId").as_deref(),
        Some("AgrInputRequired")
    );
}

#[test]
fn a_watch_hides_nothing_from_another_stream_and_a_follower_fills_a_gap_from_the_database() {
    let bed = bed("hub");
    let _entered = bed.ctx.enter();
    let hub = engine(&bed.factory).hub();
    // one client replayed through 100; another starts at 0 and still gets everything
    let _high = hub.watch(&bed.factory, "run-hub", 100);
    let mut low = hub::Follower::new(&hub, "run-hub", 0);
    hub.push_stored(
        "run-hub",
        (1..=3)
            .map(|seq| (seq, "status".to_string(), json!({"n": seq})))
            .collect(),
    );
    let got: Vec<i64> = low
        .next(&hub, &bed.facade, Duration::ZERO)
        .unwrap()
        .iter()
        .filter_map(|event| event.seq)
        .collect();
    assert_eq!(got, [1, 2, 3]);

    // a run that moved from a remote node to this one: the tail had 1 and 2, this node writes 5 and 6
    row(
        &bed.facade,
        RUN,
        &[("runId", "run-moved"), ("userId", USER), ("statusId", "AgrWorking")],
    );
    for seq in 1..=6 {
        let mut event = bed.facade.make_value(events::RUN_EVENT).unwrap();
        event.set("runId", Value::string("run-moved"));
        event.set("eventSeq", Value::I64(seq));
        event.set("eventType", Value::string("status"));
        event.set("payloadJson", Value::string(json!({"n": seq}).to_string()));
        event.create(&bed.facade).unwrap();
    }
    let mut follower = hub::Follower::new(&hub, "run-moved", 0);
    let stored_event = |seq: i64| (seq, "status".to_string(), json!({"n": seq}));
    hub.push_stored("run-moved", vec![stored_event(1), stored_event(2)]);
    hub.push_stored("run-moved", vec![stored_event(5), stored_event(6)]);
    let mut seqs: Vec<i64> = follower
        .next(&hub, &bed.facade, Duration::ZERO)
        .unwrap()
        .iter()
        .filter_map(|event| event.seq)
        .collect();
    // the late ones go in the ring too, and the follower that sent them leaves them out
    hub.push_stored("run-moved", vec![stored_event(3), stored_event(4)]);
    seqs.extend(
        follower
            .next(&hub, &bed.facade, Duration::ZERO)
            .unwrap()
            .iter()
            .filter_map(|event| event.seq),
    );
    assert_eq!(seqs, [1, 2, 3, 4, 5, 6]);
    let ring: Vec<i64> = hub
        .wait("run-moved", None, Duration::ZERO)
        .iter()
        .filter_map(|event| event.seq)
        .collect();
    assert_eq!(ring, [1, 2, 5, 6, 3, 4]);
}
