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

use super::*;
use serde_json::json;

const ME: Ctx = Ctx {
    client_id: "tui-1",
    follow_on: true,
};

fn event(seq: u64, kind: &str, payload: Value) -> Value {
    json!({"seq": seq, "type": kind, "payload": payload})
}

fn started() -> Thread {
    let mut thread = Thread::default();
    thread.outgoing("m-1", "Close the task");
    thread.turn_answered(
        "m-1",
        &json!({"conversationId": "c1", "runId": "r1", "state": "TASK_STATE_SUBMITTED", "queued": false,
                "follow": {"clientId": "tui-1", "contextVersion": 2}}),
        "tui-1",
    );
    thread
}

#[test]
fn text_chunks_append_only_their_new_suffix_and_a_gap_asks_for_a_catch_up() {
    let mut thread = started();
    assert_eq!(thread.active_run.as_deref(), Some("r1"));
    assert!(thread.owner.following);
    let chunk = |seq, offset, text: &str| {
        event(
            seq,
            "text.chunk",
            json!({"messageId": "a1", "part": 0, "offset": offset, "text": text}),
        )
    };
    assert!(thread.apply("r1", &chunk(1, 0, "Hel"), ME).is_empty());
    thread.apply("r1", &chunk(2, 0, "Hello"), ME);
    thread.apply("r1", &chunk(2, 0, "ignored, seen"), ME);
    assert_eq!(
        thread.apply("r1", &chunk(5, 5, "!"), ME),
        vec![Effect::Gap {
            run_id: "r1".into(),
            after: 2
        }]
    );
    thread.apply("r1", &chunk(3, 5, " there"), ME);
    let rows = thread.rows();
    assert_eq!(rows[0].text, "> Close the task");
    assert_eq!(rows[1].text, "Hello there");
    assert_eq!(thread.apply("r1", &chunk(4, 40, "lost"), ME), vec![Effect::Refresh]);
}

#[test]
fn steps_cards_and_the_done_line() {
    let mut thread = started();
    thread.apply(
        "r1",
        &event(
            1,
            "tool.call",
            json!({"toolCallId": "t1", "messageId": "a1", "name": "screen",
                                     "arguments": "{\"go\": \"/apps/tasks/EditTask\"}"}),
        ),
        ME,
    );
    assert_eq!(thread.status(), "Working: screen");
    thread.apply(
        "r1",
        &event(2, "tool.result", json!({"toolCallId": "t1", "status": "done"})),
        ME,
    );
    thread.apply(
        "r1",
        &event(
            3,
            "approval.requested",
            json!({"approvalId": "ap1", "summary": "Update task Ship it",
            "classification": "write", "canEdit": true,
            "changes": [{"field": "statusId", "label": "Status", "current": "Open", "proposed": "Closed"}]}),
        ),
        ME,
    );
    let paused = event(
        4,
        "run.state",
        json!({"state": "TASK_STATE_INPUT_REQUIRED", "pauseKind": "approval"}),
    );
    assert_eq!(thread.apply("r1", &paused, ME), vec![Effect::Paused]);
    assert_eq!(thread.status(), "Waiting for your approval");
    assert_eq!(thread.approvals[0].changes[0].proposed, "Closed");
    thread.apply(
        "r1",
        &event(
            5,
            "approval.decided",
            json!({"approvalId": "ap1", "decision": "approve", "status": "executed"}),
        ),
        ME,
    );
    assert!(thread.approvals.is_empty());
    let done = event(6, "run.done", json!({"state": "TASK_STATE_COMPLETED"}));
    assert_eq!(thread.apply("r1", &done, ME), vec![Effect::Ended]);
    assert!(thread.active_run.is_none());
    let rows: Vec<String> = thread.rows().into_iter().map(|row| row.text).collect();
    assert_eq!(
        rows,
        [
            "> Close the task",
            "· screen /apps/tasks/EditTask  ok",
            "Approved: Update task Ship it",
            "✓ Done · 1 step"
        ]
    );
}

#[test]
fn only_this_clients_events_at_its_page_version_move_the_screen() {
    let mut thread = started();
    let fill = |seq, client: &str, version| {
        event(
            seq,
            "ui.fill",
            json!({"form": "EditTask", "field": "statusId", "value": "Closed", "label": "Status",
            "tuiId": "EditTask.statusId", "clientId": client, "contextVersion": version}),
        )
    };
    let effects = thread.apply("r1", &fill(1, "tui-1", 2), ME);
    assert!(
        matches!(&effects[..], [Effect::Follow(Follow::Fill { tui_id, value, .. })]
        if tui_id == "EditTask.statusId" && value == "Closed")
    );
    // another client's, an older page version, or with follow off: a note, not a fill
    assert!(thread.apply("r1", &fill(2, "tab-a", 2), ME).is_empty());
    assert!(thread.apply("r1", &fill(3, "tui-1", 1), ME).is_empty());
    let off = Ctx { follow_on: false, ..ME };
    assert!(thread.apply("r1", &fill(4, "tui-1", 2), off).is_empty());
    assert_eq!(
        thread.rows().last().unwrap().text,
        "Assist filled Status (not followed here)"
    );
    // a path that is not a plain /apps path never navigates
    let nav = event(
        5,
        "ui.navigate",
        json!({"path": "https://evil.example/apps", "clientId": "tui-1", "contextVersion": 2}),
    );
    assert!(thread.apply("r1", &nav, ME).is_empty());
    assert_eq!(
        safe_path("/qapps/tasks/EditTask").as_deref(),
        Some("/apps/tasks/EditTask")
    );
    assert_eq!(safe_path("/apps/../x"), None);
}

#[test]
fn a_snapshot_never_replays_its_page_events_and_keeps_an_unanswered_message() {
    let mut thread = Thread::default();
    thread.outgoing("m-9", "Also this");
    let json = json!({
        "conversation": {"conversationId": "c1", "title": "Close the task"},
        "activeRun": {"runId": "r1", "state": "TASK_STATE_WORKING", "lastEventSeq": 7},
        "items": [
            {"messageId": "u1", "role": "user", "runId": "r1", "clientMessageId": "m-1",
             "parts": [{"part": 1, "kind": "text", "text": "Close the task"}]},
            {"messageId": "a1", "role": "assistant", "runId": "r1", "parts": [
                {"part": 1, "kind": "tool-call", "toolCallId": "t1", "name": "screen", "arguments": "{}", "status": "AgtcDone"}]},
            {"messageId": "x1", "role": "tool", "runId": "r1", "parts": [
                {"part": 1, "kind": "tool-result", "toolCallId": "t1", "isError": true, "text": "no"}]}],
        "pending": {"approvals": [], "questions": [{"questionId": "q1", "text": "Which one?",
                    "options": [{"label": "A", "recommended": true}, {"label": "B"}]}], "handoffs": [], "queued": []},
        "canvas": {"artifactId": "9", "title": "Hits", "closed": false},
        "follow": {"clientId": "tui-1", "contextVersion": 3, "following": true}
    });
    thread.snapshot(&json, "tui-1");
    assert_eq!(thread.context_version, 3);
    assert_eq!(thread.questions[0].options[0], ("A".to_string(), true));
    let old = event(
        7,
        "ui.navigate",
        json!({"path": "/apps/x", "clientId": "tui-1", "contextVersion": 3}),
    );
    assert!(thread.apply("r1", &old, ME).is_empty());
    let rows: Vec<(RowKind, String)> = thread.rows().into_iter().map(|row| (row.kind, row.text)).collect();
    assert_eq!(rows[1], (RowKind::StepFailed, "· screen   failed".replace("   ", "  ")));
    assert_eq!(rows[2].1, "> Also this  (sending)");
    assert!(rows[3].1.starts_with("This answer has a canvas \"Hits\""));
}
