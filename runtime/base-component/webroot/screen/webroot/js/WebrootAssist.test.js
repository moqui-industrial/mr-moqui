/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of Assist (WebrootAssist.js) that need no browser: the reducer of a conversation (text deltas and chunks by offset,
// sequence gaps, snapshots and a replay after them, idempotence, the approval, question and hand-off lifecycles, queued
// messages, the thread's rows), follow (whether an event applies: the owner tab, its page version, the replay floor;
// follow.locate over merged qvt names, static repeated names, dialogs, list rows and headers; follow.unsaved with secrets and
// caps), safePath (the cases of Java's path validators), the / commands and @ mentions, record chips, the Markdown with
// remote images as placeholders, the SSE parser, the page state, and the shell's hooks (palette Ask and record rows,
// moqui.onFormAnswer, qvt.registerLate, the components compiling). The drawer in a page is the Chromium smoke of Gate 3
// (crates/moqui-host/tests/gate3_browser.rs).
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootAssist.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const webroot = path.join(__dirname, "..");
globalThis.moqui = require("./MoquiLib.js");
globalThis.moqui.l10n = require("./MoquiL10n.js");
require("./WebrootChat.js");
const assist = require("./WebrootAssist.js");
vm.runInThisContext(fs.readFileSync(path.join(webroot, "libs/vue/compiler-dom.cjs.js"), "utf8"));
require("./WebrootVue.qvt.js");
const marked = require("../libs/marked/marked.umd.min.js");
const compiler = globalThis.VueCompilerDOM;
const qvt = moqui.qvt;
const palette = moqui.shellPalette;

const ctx = { clientId: "tab-a", followOn: true };
const ev = (seq, type, payload) => ({ seq, type, payload: payload || {} });
// a conversation with an active run R1 that the tab streams from its start
const streaming = () => {
    const state = assist.emptyState();
    state.conversationId = "C1";
    state.activeRunId = "R1";
    state.seq.R1 = 0;
    state.floor.R1 = 0;
    return state;
};

test("text deltas and chunks append by (messageId, part, offset): only the new suffix, a delta past the end waits", () => {
    const state = streaming();
    assert.deepEqual(assist.apply(state, "R1", ev(1, "message.start", { messageId: "M1", role: "assistant" }), ctx), []);
    assist.apply(state, "R1", ev(null, "text.delta", { messageId: "M1", part: 0, offset: 0, text: "Hel" }), ctx);
    assist.apply(state, "R1", ev(null, "text.delta", { messageId: "M1", part: 0, offset: 3, text: "lo" }), ctx);
    // a delta seen twice, and one that overlaps
    assist.apply(state, "R1", ev(null, "text.delta", { messageId: "M1", part: 0, offset: 3, text: "lo" }), ctx);
    assist.apply(state, "R1", ev(null, "text.delta", { messageId: "M1", part: 0, offset: 4, text: "o, w" }), ctx);
    const item = state.items.find((one) => one.messageId === "M1");
    assert.equal(item.parts[0].text, "Hello, w");
    // a delta past the end (one was missed) is not applied; the stored chunk brings the text
    assist.apply(state, "R1", ev(null, "text.delta", { messageId: "M1", part: 0, offset: 12, text: "!!" }), ctx);
    assert.equal(item.parts[0].text, "Hello, w");
    assist.apply(state, "R1", ev(2, "text.chunk", { messageId: "M1", part: 0, offset: 0, text: "Hello, world" }), ctx);
    assert.equal(item.parts[0].text, "Hello, world");
    assist.apply(state, "R1", ev(null, "text.delta", { messageId: "M1", part: 0, offset: 12, text: "!!" }), ctx);
    assert.equal(item.parts[0].text, "Hello, world!!");
    assert.deepEqual(assist.apply(state, "R1", ev(3, "part.done", { messageId: "M1", part: 0, length: 14 }), ctx), []);
    assert.equal(item.parts[0].done, true);
    // a part.done longer than the text, or a stored chunk past the end, reads the conversation again
    assert.deepEqual(assist.apply(state, "R1", ev(4, "part.done", { messageId: "M1", part: 0, length: 20 }), ctx), [{ kind: "refresh" }]);
    assert.deepEqual(assist.apply(state, "R1", ev(5, "text.chunk", { messageId: "M1", part: 1, offset: 5, text: "x" }), ctx), [{ kind: "refresh" }]);
    // step.restart drops the text streamed for the message
    assist.apply(state, "R1", ev(6, "step.restart", { messageId: "M1", attempt: 2 }), ctx);
    assert.equal(item.parts[0].text, "");
});

test("stored events: a sequence seen is dropped, a gap asks for a catch-up and changes nothing, live events have no sequence", () => {
    const state = streaming();
    assist.apply(state, "R1", ev(1, "run.state", { runId: "R1", state: "TASK_STATE_WORKING", statusId: "AgrWorking" }), ctx);
    assert.equal(state.seq.R1, 1);
    assert.deepEqual(assist.apply(state, "R1", ev(1, "run.state", { state: "TASK_STATE_FAILED" }), ctx), [], "the same event again");
    assert.equal(state.runs.R1.state, "TASK_STATE_WORKING");
    assert.deepEqual(assist.apply(state, "R1", ev(4, "message.start", { messageId: "M9" }), ctx), [{ kind: "gap", runId: "R1", after: 1 }]);
    assert.equal(state.seq.R1, 1);
    assert.equal(state.items.length, 0);
    // the catch-up's events in order
    [ev(2, "message.start", { messageId: "M9" }), ev(3, "text.chunk", { messageId: "M9", part: 0, offset: 0, text: "Hi" }), ev(4, "part.done", { messageId: "M9", part: 0, length: 2 })]
        .forEach((event) => assert.deepEqual(assist.apply(state, "R1", event, ctx), []));
    assert.equal(state.seq.R1, 4);
    assert.equal(state.items[0].parts[0].text, "Hi");
    // a live event changes no sequence
    assist.apply(state, "R1", ev(null, "reasoning.delta", { messageId: "M9", text: "thinking" }), ctx);
    assert.equal(state.seq.R1, 4);
    assert.equal(state.live.M9.reasoning, "thinking");
    assert.equal(assist.apply(state, "R1", null, ctx).length, 0);
});

test("a snapshot: the server's items, runs and cards; this tab's unanswered message kept; a replay after it adds nothing twice", () => {
    const state = assist.emptyState();
    assist.addOutgoing(state, "m-local", "Still sending");
    const json = {
        conversation: { conversationId: "C1", title: "Say hello", agentId: "assist" },
        activeRun: { runId: "R1", state: "TASK_STATE_WORKING", statusId: "AgrWorking", lastEventSeq: 6, stepCount: 1 },
        runs: [{ runId: "R0", state: "TASK_STATE_COMPLETED", stepCount: 2, inputTokens: 100, outputTokens: 20, activeSeconds: 3, cost: null }],
        items: [
            { messageId: "U0", seq: 1, role: "user", runId: "R0", clientMessageId: "m-0", parts: [{ part: 1, kind: "text", text: "First" }] },
            { messageId: "A0", seq: 2, role: "assistant", runId: "R0", parts: [{ part: 1, kind: "text", text: "Done." }] },
            { messageId: "U1", seq: 3, role: "user", runId: "R1", clientMessageId: "m-1", parts: [{ part: 1, kind: "text", text: "Say hello" }] },
            { messageId: "A1", seq: 4, role: "assistant", runId: "R1", parts: [{ part: 1, kind: "text", text: "Hello" },
                { part: 2, kind: "tool-call", toolCallId: "T1", name: "screen", arguments: "{\"action\":\"go\"}", status: "AgtcDone" }] },
            { messageId: "X1", seq: 5, role: "tool", runId: "R1", parts: [{ part: 1, kind: "tool-result", toolCallId: "T1", name: "screen", isError: false, text: "on Orders" }] }
        ],
        pending: { approvals: [{ approvalId: "AP1", toolCallId: "T2", summary: "Update order", changes: [{ field: "statusId", label: "Status", current: "Open", proposed: "Closed" }] }],
            questions: [], handoffs: [], queued: [{ inputId: "IN1", text: "Also this", clientMessageId: "m-2" }] },
        follow: { clientId: "tab-a", contextVersion: 3, following: true }
    };
    assist.snapshot(state, json, ctx);
    assert.equal(state.conversationId, "C1");
    assert.equal(state.activeRunId, "R1");
    assert.equal(state.seq.R1, 0, "a run first seen replays from its start");
    assert.equal(state.floor.R1, 6, "with its page actions until now never applied");
    assert.deepEqual(state.follow, { clientId: "tab-a", contextVersion: 3, following: true });
    assert.equal(state.contextVersion, 3);
    assert.equal(state.approvals.AP1.status, "pending");
    assert.deepEqual(state.results.T1, { status: "done", isError: false, text: "on Orders", name: "screen" });
    assert.deepEqual(state.items.map((item) => item.messageId || item.clientMessageId), ["U0", "A0", "U1", "A1", "X1", "m-local"]);
    // the replay: the streamed text the snapshot has, the tool call it has; then what is new
    [ev(1, "run.state", { state: "TASK_STATE_WORKING" }), ev(2, "message.start", { messageId: "A1" }),
        ev(3, "text.chunk", { messageId: "A1", part: 0, offset: 0, text: "Hello" }),
        ev(4, "tool.call", { toolCallId: "T1", messageId: "A1", name: "screen", arguments: "{}" }),
        ev(5, "tool.result", { toolCallId: "T1", name: "screen", status: "done", text: "on Orders" }),
        ev(6, "ui.navigate", { path: "/apps/Orders", clientId: "tab-a", contextVersion: 3 })]
        .forEach((event) => assert.deepEqual(assist.apply(state, "R1", event, ctx), []));
    const a1 = state.items.find((item) => item.messageId === "A1");
    assert.equal(a1.parts.filter((part) => part.kind === "text").map((part) => part.text).join(), "Hello");
    assert.equal(a1.parts.filter((part) => part.kind === "tool-call").length, 1);
    assert.deepEqual(state.chips, [], "a replayed page action is neither applied nor a chip");
    assist.apply(state, "R1", ev(7, "text.chunk", { messageId: "A1", part: 0, offset: 5, text: ", world" }), ctx);
    assert.equal(a1.parts[0].text, "Hello, world");
    // the same snapshot again, while streaming: the run keeps its place and the streamed text stays
    assist.snapshot(state, json, ctx);
    assert.equal(state.seq.R1, 7);
    assert.equal(state.items.find((item) => item.messageId === "A1").parts[0].text, "Hello, world");
    // the thread: the done line of the run that ended, the step with its result, the queued message at the end
    const rows = assist.threadRows(state);
    assert.deepEqual(rows.map((row) => row.kind), ["user", "text", "done", "user", "text", "step", "user", "queued"]);
    const done = rows.find((row) => row.kind === "done");
    assert.deepEqual([done.steps, done.tokens, done.seconds, done.cost], [2, 120, 3, null]);
    assert.equal(assist.doneText(done), "2 steps · 120 tokens · 3s");
    const step = rows.find((row) => row.kind === "step");
    assert.deepEqual([step.status, step.result, step.title], ["done", "on Orders", "screen · go"]);
    assert.equal(rows[6].local, "sending");
    assert.equal(rows[7].text, "Also this");
    // another conversation's snapshot starts over
    assist.snapshot(state, { conversation: { conversationId: "C2" }, items: [] }, ctx);
    assert.deepEqual([state.conversationId, state.items.length, state.activeRunId], ["C2", 0, null]);
});

test("a decided approval comes on its tool call and is drawn collapsed after a snapshot; one step is a step", () => {
    const state = assist.emptyState();
    assist.snapshot(state, {
        conversation: { conversationId: "C2" },
        runs: [{ runId: "R1", state: "TASK_STATE_COMPLETED", stepCount: 1 }],
        items: [{ messageId: "A1", seq: 2, role: "assistant", runId: "R1", parts: [{ part: 1, kind: "tool-call", toolCallId: "T1", name: "screen",
            arguments: "{}", status: "AgtcDone", approvalId: "AP1", approval: { approvalId: "AP1", toolCallId: "T1", summary: "Save the order", status: "executed" } }] }],
        pending: { approvals: [], questions: [], handoffs: [], queued: [] }
    }, { clientId: "tab-a" });
    assert.deepEqual([state.approvals.AP1.status, state.approvals.AP1.toolCallId, state.approvals.AP1.summary], ["executed", "T1", "Save the order"]);
    const done = assist.threadRows(state).find((row) => row.kind === "done");
    assert.equal(assist.doneText(done), "1 step");
    assert.equal(assist.doneText(Object.assign({}, done, { cost: "0" })), "1 step", "no cost of nothing");
    // a step of the screen tool by its press and page, one waiting for the user by its own status
    assert.equal(assist.stepTitle("screen", '{"go":"/apps/Orders/Edit","set":{"Name":"A"},"do":"Save"}'), "screen · Save on /apps/Orders/Edit");
    assert.equal(assist.stepTitle("screen", '{"go":"/apps/Orders"}'), "screen · /apps/Orders");
    assert.equal(assist.stepStatus("AgtcAwaiting"), "waiting");
});

test("follow from a call's answer (decide, context): the owner and version taken, the tab's version never lower", () => {
    const state = assist.emptyState();
    // a tab that navigated without reporting (it was not the owner) is ahead of the server: decide sends its version and keeps it
    state.contextVersion = 5;
    assist.takeFollow(state, { clientId: "tab-b", contextVersion: 5 }, "tab-b");
    assert.deepEqual([state.follow.following, state.contextVersion], [true, 5]);
    // one that is behind takes the server's
    state.contextVersion = 1;
    assist.takeFollow(state, { clientId: "tab-b", contextVersion: 3 }, "tab-b");
    assert.equal(state.contextVersion, 3);
    // the events of the hand-off, stamped with what the server took, apply in this tab
    assert.equal(assist.follow.applies(state, "R1", { seq: 4, type: "ui.navigate", payload: { clientId: "tab-b", contextVersion: 3 } }, { clientId: "tab-b", followOn: true }), true);
    assist.takeFollow(state, { clientId: "tab-a", contextVersion: 4 }, "tab-b");
    assert.equal(state.follow.following, false);
});

test("the stream's back-off: 500 ms doubling to at most 15 s", () => {
    assert.deepEqual([1, 2, 3, 4, 5, 6, 10].map((attempts) => assist.backoff(attempts)), [500, 1000, 2000, 4000, 8000, 15000, 15000]);
});

test("a message: sending, then the run's or queued; a queued one folds into the history; a failed send keeps its id", () => {
    const state = assist.emptyState();
    assist.addOutgoing(state, "m-1", "Hello");
    assert.equal(assist.threadRows(state)[0].local, "sending");
    assist.turnAnswered(state, "m-1", { conversationId: "C1", runId: "R1", state: "TASK_STATE_SUBMITTED", statusId: "AgrSubmitted", queued: false,
        follow: { clientId: "tab-a", contextVersion: 2 } }, ctx);
    assert.deepEqual([state.conversationId, state.activeRunId, state.seq.R1, state.floor.R1], ["C1", "R1", 0, 0]);
    assert.deepEqual(state.follow, { clientId: "tab-a", contextVersion: 2, following: true }, "sending a turn makes the tab the follow owner");
    assert.equal(state.contextVersion, 2);
    assert.equal(state.items[0].local, null);
    // a second message while the run works is queued
    assist.addOutgoing(state, "m-2", "Also this");
    assist.turnAnswered(state, "m-2", { conversationId: "C1", runId: "R1", queued: true }, ctx);
    assert.deepEqual(assist.threadRows(state).map((row) => row.kind), ["user", "queued"]);
    // the conversation names its input; the run folds it in
    assist.snapshot(state, { conversation: { conversationId: "C1" }, activeRun: { runId: "R1", state: "TASK_STATE_WORKING", lastEventSeq: 1 },
        items: [{ messageId: "U1", role: "user", runId: "R1", clientMessageId: "m-1", parts: [{ part: 1, kind: "text", text: "Hello" }] }],
        pending: { queued: [{ inputId: "IN2", clientMessageId: "m-2", text: "Also this" }] } }, ctx);
    assert.deepEqual(assist.threadRows(state).map((row) => row.kind + ":" + row.text), ["user:Hello", "queued:Also this"]);
    assist.apply(state, "R1", ev(1, "run.state", { state: "TASK_STATE_WORKING" }), ctx);
    assert.deepEqual(assist.apply(state, "R1", ev(2, "input.folded", { inputId: "IN2", kind: "message" }), ctx), []);
    assert.deepEqual(assist.threadRows(state).map((row) => row.kind + ":" + row.text), ["user:Hello", "user:Also this"]);
    assert.deepEqual(assist.apply(state, "R1", ev(3, "input.folded", { inputId: "IN9" }), ctx), [{ kind: "refresh" }], "an input it does not know");
    // a send that failed shows so, with its client message id for Retry
    assist.addOutgoing(state, "m-3", "Lost");
    assist.turnFailed(state, "m-3", "network");
    const failed = assist.threadRows(state).find((row) => row.clientMessageId === "m-3");
    assert.deepEqual([failed.local, failed.error], ["failed", "network"]);
});

test("card lifecycles: an approval requested and decided, a question asked and answered, a hand-off started and done", () => {
    const state = streaming();
    let seq = 0;
    const next = (type, payload) => assist.apply(state, "R1", ev(++seq, type, payload), ctx);
    next("message.start", { messageId: "M1" });
    next("tool.call", { toolCallId: "T1", messageId: "M1", name: "screen", arguments: { action: "do" }, classification: "write" });
    next("approval.requested", { approvalId: "AP1", runId: "R1", toolCallId: "T1", classification: "write", targetKind: "screen", summary: "Save order",
        changes: [{ field: "name", label: "Name", current: "A", proposed: "B" }], canEdit: true, canPreview: false });
    assert.deepEqual(next("run.state", { state: "TASK_STATE_AUTH_REQUIRED", pauseKind: "AgpsApproval", pauseRef: "AP1" }), [{ kind: "paused", runId: "R1" }]);
    assert.equal(state.approvals.AP1.status, "pending");
    assert.equal(state.status, assist.labels.waitingApproval);
    assert.equal(state.activeRunId, "R1", "a paused run is still the active one");
    next("approval.decided", { approvalId: "AP1", toolCallId: "T1", decision: "approve", status: "approved", channel: "drawer" });
    assert.equal(state.approvals.AP1.status, "approved");
    next("run.state", { state: "TASK_STATE_WORKING" });
    next("tool.result", { toolCallId: "T1", name: "screen", status: "done", text: "saved" });
    assert.equal(assist.stepOf(state, "T1").status, "done");
    // a decision with no status comes from its kind
    next("approval.requested", { approvalId: "AP2", toolCallId: "T2", summary: "Delete" });
    next("approval.decided", { approvalId: "AP2", decision: "reject", feedback: "Not that one" });
    assert.deepEqual([state.approvals.AP2.status, state.approvals.AP2.feedback], ["rejected", "Not that one"]);
    next("approval.decided", { approvalId: "AP9", decision: "expire" });
    assert.equal(state.approvals.AP9.status, "expired");
    // a question and the budget question
    next("question.asked", { questionId: "Q1", toolCallId: "T3", text: "Which one?", options: [{ label: "A", recommended: true }, { label: "B" }], surface: "drawer" });
    assert.equal(state.questions.Q1.status, "open");
    assert.equal(state.status, assist.labels.working);
    next("question.answered", { toolCallId: "T3", answer: "A" });
    assert.deepEqual([state.questions.Q1.status, state.questions.Q1.answer], ["answered", "A"]);
    next("question.asked", { questionId: "Q2", text: "Continue?", choices: ["continue", "stop"], kind: "budget" });
    assert.deepEqual(state.questions.Q2.options.map((option) => option.label), ["continue", "stop"]);
    next("question.answered", { questionId: "Q2", continue: true });
    assert.equal(state.questions.Q2.status, "answered");
    // a hand-off for this tab and page is followed; its end is kept
    const started = next("handoff.started", { handoffId: "H1", approvalId: "AP3", path: "/apps/Orders/Edit", form: "EditOrder", transition: "updateOrder",
        values: [{ field: "name", label: "Name", value: "B" }], clientId: "tab-a", contextVersion: 0 });
    assert.equal(started.length, 1);
    assert.equal(started[0].kind, "follow");
    assert.equal(state.handoffs.H1.status, "watching");
    next("handoff.done", { handoffId: "H1", approvalId: "AP3", outcome: "saved", values: { name: "B" } });
    assert.equal(state.handoffs.H1.status, "saved");
    // the run ends: no active run, its messages stream no more
    assert.deepEqual(next("run.done", { runId: "R1", state: "TASK_STATE_COMPLETED", result: { text: "Done", messageId: "M1" } }), [{ kind: "ended", runId: "R1" }]);
    assert.equal(state.activeRunId, null);
    assert.equal(state.items[0].streaming, false);
    assert.equal(state.status, "");
});

test("follow: only the owner tab, with follow on, for its page version and past the replay floor; the rest are chips", () => {
    const state = streaming();
    state.contextVersion = 4;
    const nav = (seq, payload) => ev(seq, "ui.navigate", Object.assign({ path: "/apps/Orders", title: "Orders", clientId: "tab-a", contextVersion: 4 }, payload));
    assert.equal(assist.follow.applies(state, "R1", nav(1), ctx), true);
    assert.equal(assist.follow.applies(state, "R1", nav(1, { clientId: "tab-b" }), ctx), false, "another tab's");
    assert.equal(assist.follow.applies(state, "R1", nav(1, { contextVersion: 3 }), ctx), false, "minted before the user moved");
    assert.equal(assist.follow.applies(state, "R1", nav(1), { clientId: "tab-a", followOn: false }), false, "follow off");
    assert.equal(assist.follow.applies(state, "R1", nav(null), ctx), false, "a page action is a stored event");
    state.floor.R1 = 5;
    assert.equal(assist.follow.applies(state, "R1", nav(5), ctx), false, "a replay after a reload");
    assert.equal(assist.follow.applies(state, "R1", nav(6), ctx), true);
    state.floor.R1 = 0;
    // in the reducer: the effect for this tab, a chip for another's
    assert.deepEqual(assist.apply(state, "R1", nav(1), ctx).map((effect) => effect.kind), ["follow"]);
    assert.deepEqual(assist.apply(state, "R1", nav(2, { clientId: "tab-b", title: "SO10023" }), ctx), []);
    assert.deepEqual(state.chips.map((chip) => [chip.kind, chip.label]), [["navigate", "SO10023"]]);
    assist.apply(state, "R1", ev(3, "ui.fill", { field: "name", label: "Name", value: "x", clientId: "tab-b", contextVersion: 4 }), ctx);
    assist.apply(state, "R1", ev(4, "ui.dialog", { id: "D1", title: "Edit", open: true, clientId: "tab-b", contextVersion: 4 }), ctx);
    assert.deepEqual(state.chips.map((chip) => chip.kind), ["navigate", "fill", "dialog"]);
    for (let i = 5; i < 12; i++) assist.apply(state, "R1", ev(i, "ui.fill", { field: "f" + i, clientId: "tab-b", contextVersion: 4 }), ctx);
    assert.equal(state.chips.length, 5, "at most five chips");
    assist.apply(state, "R1", nav(12, { clientId: "tab-b", title: "Next" }), ctx);
    assert.deepEqual(state.chips.map((chip) => chip.label), ["Next"], "a navigation starts the chips over");
    // contextVersion: the user's own navigation raises it past the server's
    state.follow.contextVersion = 7;
    assert.equal(assist.nextContextVersion(state), 8);
    assert.equal(assist.nextContextVersion(state), 9);
    assert.equal(assist.follow.applies(state, "R1", nav(13, { contextVersion: 8 }), ctx), false);
    assert.equal(assist.follow.applies(state, "R1", nav(13, { contextVersion: 9 }), ctx), true);
});

// forms as gatherForms reads them, in DOM order
const form = (name, extra) => Object.assign({ name, id: name, dialog: null, inReach: true, fields: ["name", "description"], types: {}, multi: false }, extra);

test("follow.locate over merged names: the qvt id, else the name or name_<n> by occurrence within the scope", () => {
    // a merged render: a section-iterate's form EditItem once per entry, EditItem_0 and EditItem_1, and a plain form
    const forms = [form("FindItems", { inReach: true }), form("EditItem_0"), form("EditItem_1"), form("Note")];
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", domId: "EditItem_1", occurrence: 1, field: "name" }, forms).index, 2);
    // the tui id may name the entry the page does not have: then the occurrence among the forms of the name
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", domId: "EditItem_9", occurrence: 0, field: "name" }, forms).index, 1);
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", occurrence: 1, field: "name" }, forms).index, 2);
    assert.deepEqual(assist.follow.locate({ scope: "page", form: "Note", domId: "Note", occurrence: 0, field: "description" }, forms),
        { ok: true, form: Object.assign({ index: 3 }, forms[3]), index: 3, field: "description" });
    assert.deepEqual(assist.follow.locate({ scope: "page", form: "Missing", field: "name" }, forms), { ok: false, reason: "none" });
    // one candidate takes any occurrence; several and an occurrence past them is none
    assert.equal(assist.follow.locate({ form: "Note", occurrence: 3, field: "name" }, forms).index, 3);
    assert.equal(assist.follow.locate({ form: "EditItem", occurrence: 5, field: "name" }, forms).ok, false);
    // a name that is a prefix of another is not that other
    assert.equal(assist.follow.locate({ form: "Edit", field: "name" }, forms).ok, false);
});

test("follow.locate: static renders repeat the bare name, dialogs are their own scope, list rows and headers", () => {
    // a static render: the same id twice, in DOM order
    const statics = [form("EditItem"), form("EditItem")];
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", domId: "EditItem_1", occurrence: 1, field: "name" }, statics).index, 1);
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", domId: "EditItem", occurrence: 1, field: "name" }, statics).index, 1);
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", domId: "EditItem", occurrence: 0, field: "name" }, statics).index, 0);
    // a dialog: the form of that dialog, not the page's of the same name; the page's first for page
    const withDialog = [form("EditItem"), form("EditItem", { dialog: "EditDialog" }), form("EditItem", { dialog: "OtherDialog" })];
    assert.equal(assist.follow.locate({ scope: "dialog", dialog: "EditDialog", form: "EditItem", occurrence: 0, field: "name" }, withDialog).index, 1);
    assert.equal(assist.follow.locate({ scope: "dialog", dialog: "OtherDialog", form: "EditItem", occurrence: 0, field: "name" }, withDialog).index, 2);
    assert.equal(assist.follow.locate({ scope: "page", form: "EditItem", occurrence: 0, field: "name" }, withDialog).index, 0);
    assert.equal(assist.follow.locate({ scope: "dialog", dialog: "Gone", form: "EditItem", field: "name" }, withDialog).ok, false);
    // list rows: the row form <list>_<row>, else the multi form's <field>_<row>
    const rows = [form("ItemList_header", { fields: ["name", "name_op"] }), form("ItemList_0"), form("ItemList_1")];
    const row = assist.follow.locate({ scope: "row", form: "ItemList", row: 1, field: "name" }, rows);
    assert.deepEqual([row.index, row.field], [2, "name"]);
    const multi = [form("ItemList", { multi: true, fields: ["name_0", "name_1", "_isMulti"] })];
    const multiRow = assist.follow.locate({ scope: "row", form: "ItemList", row: 1, field: "name" }, multi);
    assert.deepEqual([multiRow.index, multiRow.field], [0, "name_1"]);
    assert.equal(assist.follow.locate({ scope: "row", form: "ItemList", row: 7, field: "name" }, rows).ok, false);
    // the header find form of a list
    assert.equal(assist.follow.locate({ scope: "find", form: "ItemList_header", domId: "ItemList_header", occurrence: 0, field: "name_op" }, rows).index, 0);
    // refused: outside assistReach, a hidden, password or file field, a secret name, a field the form does not have
    const refused = [form("Shell", { inReach: false }), form("Edit", { fields: ["name", "token", "photo", "userId"], types: { photo: "file", userId: "hidden" } })];
    assert.equal(assist.follow.locate({ form: "Shell", field: "name" }, refused).reason, "reach");
    assert.equal(assist.follow.locate({ form: "Edit", field: "photo" }, refused).reason, "closed");
    assert.equal(assist.follow.locate({ form: "Edit", field: "userId" }, refused).reason, "closed");
    assert.equal(assist.follow.locate({ form: "Edit", field: "token" }, refused).reason, "secret");
    assert.equal(assist.follow.locate({ form: "Edit", field: "currentPassword" }, refused).reason, "secret");
    assert.equal(assist.follow.locate({ form: "Edit", field: "other" }, refused).reason, "field");
    assert.equal(assist.follow.locate({ form: "Edit" }, refused).ok, true, "a highlight of the form itself");
});

test("follow.unsaved: the changed values by qvt id, never a secret (not even read), no closed widget, capped", () => {
    const values = { name: "New", description: "Same", moquiSessionToken: "T", count: 3, tags: ["a", "b"], note: { nested: true }, photo: "x.png", userId: "U1" };
    Object.defineProperty(values, "password", { enumerable: true, get() { throw new Error("a secret was read"); } });
    Object.defineProperty(values, "apiKey", { enumerable: true, get() { throw new Error("a secret was read"); } });
    const forms = [{ id: "EditUser", values, original: { name: "Old", description: "Same", count: 2, tags: [] }, types: { photo: "file", userId: "hidden" } },
        { id: "Untouched", values: { a: "1" }, original: { a: "1" } }];
    assert.deepEqual(assist.follow.unsaved(forms), [{ domId: "EditUser", values: { name: "New", count: "3", tags: "a,b" }, dirty: ["name", "count", "tags"] }]);
    // the server's extra secret names
    assert.deepEqual(assist.follow.unsaved([{ id: "F", values: { pin: "1234", ok: "y" }, original: {} }], { contains: ["password"], endsWith: [], names: ["pin"] }),
        [{ domId: "F", values: { ok: "y" }, dirty: ["ok"] }]);
    // caps: forms, values per form, characters per value
    const many = Array.from({ length: 30 }, (v, i) => ({ id: "F" + i, values: { a: "x" }, original: {} }));
    assert.equal(assist.follow.unsaved(many).length, 20);
    const wide = {};
    for (let i = 0; i < 150; i++) wide["f" + i] = "v";
    assert.equal(assist.follow.unsaved([{ id: "W", values: wide, original: {} }])[0].dirty.length, 100);
    const long = assist.follow.unsaved([{ id: "L", values: { text: "y".repeat(5000) }, original: {} }], null, { maxValueChars: 10 });
    assert.equal(long[0].values.text, "yyyyyyyyyy");
    assert.deepEqual(assist.follow.unsaved([{ values: { a: "1" }, original: {} }]), [], "a form with no qvt id");
});

test("safePath: an application path with flat parameters; Java's validator cases and the rest refused", () => {
    const sp = (p, params) => assist.safePath(p, params);
    assert.deepEqual(sp("/apps/system/Security/UserAccount", { username: "john", active: true, page: 2 }),
        { ok: true, path: "/apps/system/Security/UserAccount", url: "/qapps/system/Security/UserAccount?username=john&active=true&page=2",
            parameters: { username: "john", active: "true", page: "2" } });
    assert.deepEqual(sp("/qapps/Orders?orderId=SO%2010&x=").parameters, { orderId: "SO 10" });
    assert.equal(sp("/qapps").url, "/qapps");
    assert.equal(sp("/apps/Orders", { empty: "", none: null }).url, "/qapps/Orders");
    assert.equal(assist.safePath("/apps/x", null, { linkBase: "/vapps" }).url, "/vapps/x");
    // Java's RequestTool.validatePath and AssistOpenUiNav.validatePath cases
    [["http://evil", "not a path from the root"], ["//evil", "a scheme or a host"], ["//evil.host/x", "a scheme or a host"], ["/foo/../bar", "a .. segment"],
        ["/foo/%2e%2e%2fsecret", "a .. segment"], ["/rest/s1/moqui", "not an application screen"], ["", "no path"], [null, "no path"],
        ["apps/x", "not a path from the root"], ["/apps/a b", "whitespace, a backslash or a control character"],
        ["/apps\\x", "whitespace, a backslash or a control character"], ["/apps/x\u0000", "whitespace, a backslash or a control character"],
        ["/apps/%0a", "whitespace, a backslash or a control character"], ["/apps/%5cx", "whitespace, a backslash or a control character"],
        ["/javascript:alert(1)", "a scheme"], ["/apps/data:text", "a scheme"], ["/apps/x/mailto:a@b", "a scheme"], ["/apps//evil", "a scheme or a host"],
        ["/apps/https://evil", "a scheme or a host"], ["/apps/x#frag", "an anchor"], ["/apps/%E0%A4%A", "not a valid path"], ["/apps/./x", "a .. segment"],
        ["/appsx/y", "not an application screen"], ["/apps/x?a=%E0%A4%A", "a query that does not decode"]]
        .forEach(([given, error]) => assert.deepEqual(sp(given), { ok: false, error }, String(given)));
    // parameters: flat, plain names, no control characters, an object
    assert.equal(sp("/apps/x", { "a b": "1" }).error, "a parameter name that is not plain");
    assert.equal(sp("/apps/x", { a: { b: 1 } }).error, "a parameter that is not flat");
    assert.equal(sp("/apps/x", { a: ["1"] }).error, "a parameter that is not flat");
    assert.equal(sp("/apps/x", { a: "1\n2" }).error, "a control character in a parameter");
    assert.equal(sp("/apps/x", ["a"]).error, "parameters that are not an object");
    assert.equal(sp("/apps/x?a%20b=1").error, "a parameter name that is not plain");
    const tooMany = {};
    for (let i = 0; i < 51; i++) tooMany["p" + i] = "1";
    assert.equal(sp("/apps/x", tooMany).error, "too many parameters");
});

test("commands and mentions: known / commands only, admin ones for an admin; @ at the caret and the inserted record chip", () => {
    assert.deepEqual(assist.parseCommand("/new"), { name: "new", arg: "" });
    assert.deepEqual(assist.parseCommand("  /tier Thorough "), { name: "tier", arg: "thorough" });
    assert.deepEqual(assist.parseCommand("/tier fastest"), { name: "tier", arg: "", error: "fast, balanced or thorough" });
    assert.deepEqual(assist.parseCommand("/skill order-entry check the totals"), { name: "skill", arg: "order-entry check the totals" });
    assert.deepEqual(assist.parseCommand("/skill"), { name: "skill", arg: "", error: "a skill name" });
    assert.deepEqual(assist.parseCommand("/help"), { name: "help", arg: "" });
    assert.equal(assist.parseCommand("/prompt"), null, "an admin command for a user who is not one");
    assert.deepEqual(assist.parseCommand("/prompt", { admin: true }), { name: "prompt", arg: "" });
    assert.deepEqual(assist.parseCommand("/usage", { admin: true }), { name: "usage", arg: "" });
    // not a command: a path, an unknown name, text
    assert.equal(assist.parseCommand("/apps/Orders"), null);
    assert.equal(assist.parseCommand("/unknown x"), null);
    assert.equal(assist.parseCommand("hello /new"), null);
    // @: at the start or after a space, no space in the query
    assert.deepEqual(assist.mentionAt("Look at @SO10", 13), { start: 8, end: 13, query: "SO10" });
    assert.deepEqual(assist.mentionAt("@jo"), { start: 0, end: 3, query: "jo" });
    assert.equal(assist.mentionAt("mail me at a@b.com"), null, "an @ in a word");
    assert.equal(assist.mentionAt("@john doe"), null, "a space after it ends the mention");
    assert.deepEqual(assist.mentionAt("@john doe", 5), { start: 0, end: 5, query: "john" });
    const mention = assist.mentionAt("Close @SO1 now", 10);
    assert.deepEqual(assist.insertMention("Close @SO1 now", mention, { title: "SO10023 [Acme|West]", recordEntity: "moqui.basic.Enumeration", recordId: "SO10023" }),
        { text: "Close [[record:moqui.basic.Enumeration:SO10023|SO10023 Acme West]]  now", caret: 67 });
    assert.deepEqual(assist.insertMention("@as", { start: 0, end: 3 }, { agentId: "assist", label: "Assist" }), { text: "@assist ", caret: 8 });
});

test("record chips: drawn only from the chip map (the user's own without a link), else their label; in Markdown a chip link", () => {
    const text = "See [[record:moqui.security.UserAccount:EX_JOHN|john.doe]] and [[record:Other:9]].";
    assert.deepEqual(assist.segments(text, null), [{ text: "See john.doe and 9." }]);
    const map = { "moqui.security.UserAccount:EX_JOHN": { label: "John Doe", url: "/qapps/system/Security/UserAccount/UserAccountDetail?userId=EX_JOHN" } };
    assert.deepEqual(assist.segments(text, map), [{ text: "See " },
        { chip: { entityName: "moqui.security.UserAccount", id: "EX_JOHN", label: "John Doe", url: "/qapps/system/Security/UserAccount/UserAccountDetail?userId=EX_JOHN" } },
        { text: " and 9." }]);
    assert.deepEqual(assist.segments(text, null, true).filter((segment) => segment.chip).map((segment) => [segment.chip.label, segment.chip.url]),
        [["john.doe", null], ["9", null]]);
    assert.equal(assist.chipsToMarkdown(text, map), 'See [John Doe](/qapps/system/Security/UserAccount/UserAccountDetail?userId=EX_JOHN "assist-chip") and 9.');
    const html = assist.renderMarkdown(marked, text, { chips: map });
    assert.match(html, /<a href="\/qapps\/system\/Security\/UserAccount\/UserAccountDetail\?userId=EX_JOHN" class="m-assist-chip" data-md-link>John Doe<\/a>/);
    assert.doesNotMatch(html, /record:/);
});

test("the thread's Markdown: remote images as placeholders, links through safePath, external ones with their host, invented ones plain", () => {
    const html = assist.renderMarkdown(marked, [
        "![chart](https://evil.example.com/p.png) ![local](/apps/static/logo.png) ![tracker](//evil.example.com/t.gif)",
        "<img src=\"https://evil.example.com/raw.png\" srcset=\"https://evil.example.com/a.png 1x\"> <img src=\"data:image/png;base64,AAAA\" alt=\"inline\">",
        "<image src=\"https://evil.example.com/image.png\"> <svg><image href=\"https://evil.example.com/svg.png\"></image></svg> <a href=\"javascript:alert(1)\">raw</a>",
        "inline <b onclick=\"alert(1)\">bold</b> and <iframe src=\"https://evil.example.com/\"></iframe>",
        "[Orders](/qapps/Orders?status=open) [admin](/rest/s1/moqui) [up](/apps/../x) [docs](https://docs.example.org/page) [protocol](//cdn.example.org/x)",
        "[made up](/qapps/Orders/SO99999) [mail](mailto:a@b.com) [js](javascript:alert(1)) [rel](other/page) [anchor](#top)"
    ].join("\n\n"), { invented: ["/qapps/Orders/SO99999"] });
    // no image source of another host is left, nor a srcset
    assert.doesNotMatch(html, /src="https?:|src="\/\/|srcset="|src="data:/);
    assert.match(html, /<span class="m-assist-image" data-assist-image="evil.example.com">Image: chart \(evil.example.com\)<\/span>/);
    // raw HTML (an <img>, <image>, an SVG image, a script link, an event attribute, a frame) is never markup: it is shown as text
    assert.doesNotMatch(html, /<(img|image|svg|iframe|b)\b[^>]*(evil|onclick)|<a href="javascript|<a href="https:\/\/evil/i);
    assert.match(html, /&lt;image src=&quot;https:\/\/evil.example.com\/image.png&quot;&gt;/);
    assert.match(html, /&lt;a href=&quot;javascript:alert\(1\)&quot;&gt;raw&lt;\/a&gt;/);
    assert.match(html, /inline &lt;b onclick=/);
    assert.match(html, /<img src="\/apps\/static\/logo.png" alt="local">/);
    // an application path through the shell; a path that is not one, as text
    assert.match(html, /<a href="\/qapps\/Orders\?status=open" data-md-link>Orders<\/a>/);
    assert.doesNotMatch(html, /rest\/s1/);
    assert.match(html, / admin /);
    assert.doesNotMatch(html, /href="\/apps\/\.\./);
    // external: its host, a new tab
    assert.match(html, /<a href="https:\/\/docs.example.org\/page" title="docs.example.org" class="m-assist-external" target="_blank" rel="noopener noreferrer">docs<\/a>/);
    assert.doesNotMatch(html, /cdn.example.org/);
    // invented, mail, script, relative and anchor links are text
    assert.doesNotMatch(html.replace(/&lt;[\s\S]*?&gt;/g, ""), /SO99999|mailto|javascript|other\/page|#top/);
    assert.match(html, /made up/);
    // DOMPurify (in the browser) also takes out what is left of media and styles
    ["video", "audio", "source", "picture", "iframe", "style", "form"].forEach((tag) => assert.ok(assist.purifyOptions.FORBID_TAGS.indexOf(tag) >= 0, tag));
    ["style", "srcset", "poster"].forEach((attr) => assert.ok(assist.purifyOptions.FORBID_ATTR.indexOf(attr) >= 0, attr));
    assert.equal(assist.hostOf("https://A.Example.com:8443/x"), "a.example.com");
    assert.equal(assist.hostOf("/apps/x"), "");
});

test("the event stream: SSE frames with their ids, comments and retry left out, the rest kept for more", () => {
    const text = "retry: 2000\n\n: ping\n\nid: 3\nevent: text.chunk\ndata: {\"seq\":3,\"type\":\"text.chunk\",\"payload\":{\"messageId\":\"M1\",\"text\":\"a\"}}\n\n" +
        "event: text.delta\ndata: {\"seq\":null,\"type\":\"text.delta\",\"payload\":{\"text\":\"b\"}}\r\n\r\nid: 4\nevent: part.done\ndata: {\"seq\":4";
    const parsed = assist.parseSse(text);
    assert.equal(parsed.events.length, 2);
    assert.deepEqual(parsed.events.map((frame) => [frame.id, frame.event]), [["3", "text.chunk"], [null, "text.delta"]]);
    assert.equal(parsed.rest, "id: 4\nevent: part.done\ndata: {\"seq\":4");
    assert.deepEqual(assist.frameEvent(parsed.events[0]), { seq: 3, type: "text.chunk", payload: { messageId: "M1", text: "a" } });
    assert.deepEqual(assist.frameEvent(parsed.events[1]), { seq: null, type: "text.delta", payload: { text: "b" } });
    assert.deepEqual(assist.frameEvent({ id: "7", event: "usage", data: "{\"type\":\"usage\",\"payload\":{}}" }), { seq: 7, type: "usage", payload: {} });
    assert.equal(assist.frameEvent({ id: "1", event: "x", data: "not json" }), null);
    assert.deepEqual(assist.parseSse("data: one\ndata: two\n\n").events[0].data, "one\ntwo");
});

test("the page state of a turn: the screen context mapped to the contract, and the parameters a call posts", () => {
    const context = { path: "/qapps/system/Security/UserAccount", parameters: { username: "john", n: 2, bad: { x: 1 } },
        record: { entity: "moqui.security.UserAccount", id: "EX_JOHN", title: "john.doe" }, selection: { form: "UserList", ids: ["A", "B"] },
        filters: [{ form: "UserList", parameters: { username: "john" } }], forms: [{ name: "UserList", kind: "list", fields: ["username"], rows: 25 }] };
    assert.deepEqual(assist.pageState(context, { title: "User Accounts", forms: [{ domId: "F", values: { a: "1" }, dirty: ["a"] }], dialogs: ["D1"] }), {
        path: "/qapps/system/Security/UserAccount", parameters: { username: "john", n: "2" }, title: "User Accounts",
        forms: [{ domId: "F", values: { a: "1" }, dirty: ["a"] }], dialogs: ["D1"], record: { entityName: "moqui.security.UserAccount", id: "EX_JOHN", label: "john.doe" },
        selections: [{ form: "UserList", ids: ["A", "B"], total: 2 }], filters: [{ form: "UserList", values: { username: "john" } }],
        formInfo: [{ name: "UserList", kind: "list", fields: ["username"], rows: 25 }] });
    assert.deepEqual(assist.pageState({}, {}), { path: "", parameters: {}, forms: [], dialogs: [] });
    // moqui.request posts a plain object as a form: an object or list goes as JSON text
    assert.deepEqual(assist.callParams({ text: "hi", state: { path: "/x" }, choices: ["A"], release: true, none: null, missing: undefined, n: 3 }),
        { text: "hi", state: "{\"path\":\"/x\"}", choices: "[\"A\"]", release: "true", n: "3" });
    // the hand-off's form: by name, with the section entry index, or by the transition it posts to
    const handoff = { form: "EditOrder", transition: "updateOrder" };
    assert.equal(assist.handoffMatches(handoff, { name: "EditOrder" }), true);
    assert.equal(assist.handoffMatches(handoff, { id: "EditOrder_2" }), true);
    assert.equal(assist.handoffMatches(handoff, { name: "Other", action: "/apps/Orders/EditOrder/updateOrder?x=1" }), true);
    assert.equal(assist.handoffMatches(handoff, { name: "EditOrderLines", action: "/apps/Orders/updateLines" }), false);
    // the tab's id is made once and kept
    const kept = {};
    const store = { getItem: (key) => (key in kept ? kept[key] : null), setItem: (key, value) => { kept[key] = value; } };
    const first = assist.clientIdOf(store);
    assert.match(first, /^tab-[a-z0-9]+$/);
    assert.equal(assist.clientIdOf(store), first);
    assert.match(assist.clientIdOf(null), /^tab-/);
    assert.equal(assist.isSecret("newPassword"), true);
    assert.equal(assist.isSecret("apiKey"), true);
    assert.equal(assist.isSecret("monkey"), true, "ends with key, as the server's rule");
    assert.equal(assist.isSecret("username"), false);
});

test("the shell's hooks: the palette's Ask row and record rows, the form answers, late registration, the components", () => {
    const sources = [{ group: "Records", label: "Records", prefix: null }];
    const input = { labels: { askAssist: "Ask Assist" }, ask: true, screens: [], actions: [], savedFinds: [],
        remote: [{ group: "Records", label: "Records", rows: [{ title: "SO10023", url: "/qapps/Orders/SO10023", recordEntity: "moqui.basic.Enumeration", recordId: "SO10023" },
            { title: "Plain", url: "/qapps/x" }] }] };
    const groups = palette.groups(palette.parse("close SO10023", sources), input);
    assert.deepEqual(groups.map((group) => group.key), ["source:Records", "ask"]);
    assert.deepEqual([groups[0].rows[0].recordEntity, groups[0].rows[0].recordId], ["moqui.basic.Enumeration", "SO10023"]);
    assert.equal(groups[0].rows[1].recordEntity, undefined);
    assert.deepEqual(groups[1].rows[0], { ask: true, text: "close SO10023", title: "Ask Assist ‘close SO10023’", icon: "sym_o_auto_awesome", index: 2 });
    // ? asks alone; no Ask without Assist or without text
    assert.deepEqual(palette.parse("? what is this", sources), { text: "what is this", mode: "ask", prefix: "?", groups: [] });
    assert.deepEqual(palette.groups(palette.parse("?what", sources), input).map((group) => group.key), ["ask"]);
    assert.deepEqual(palette.groups(palette.parse("close", sources), Object.assign({}, input, { ask: false })).map((group) => group.key), ["source:Records"]);
    assert.deepEqual(palette.groups(palette.parse("?", sources), input), []);
    // moqui.onFormAnswer: once per answer of m-form, a refused one too; off takes it back
    const told = [];
    const off = moqui.onFormAnswer((answer) => told.push([answer.form.me, answer.ok, answer.json]));
    const comp = qvt.components["m-form"];
    const make = (me) => ({ me, $root: { setUrl() {}, hideContainer() {}, reloadContainer() {} }, submitHideId: null, submitReloadId: null, submitMessage: "Saved",
        mSubmitHide: null, fields: {}, $emit() {} });
    const saved = make(1);
    comp.methods.handleResponse.call(saved, { messageInfos: ["ok"] });
    comp.methods.handleResponse.call(saved, { messageInfos: ["again"] });
    comp.methods.handleResponse.call(make(2), { validationErrors: [{ field: "x", message: "bad" }] });
    moqui.formAnswered(make(3), { ok: false, json: { errors: "refused" }, status: 400 });
    off();
    comp.methods.handleResponse.call(make(4), {});
    assert.deepEqual(told, [[1, true, { messageInfos: ["ok"] }], [2, false, { validationErrors: [{ field: "x", message: "bad" }] }], [3, false, { errors: "refused" }]]);
    // qvt.registerLate: kept for the start, and on the running application at once
    const late = { template: "<div>late</div>" };
    qvt.registerLate("m-late-test", late);
    assert.equal(qvt.components["m-late-test"], late);
    const registered = [];
    qvt.app = { component: (name, def) => registered.push([name, def]) };
    qvt.registerLate("m-late-test-2", { template: "<div>later</div>" });
    assert.deepEqual(registered.map((entry) => entry[0]), ["m-late-test-2"]);
    qvt.app = null;
    // the components compile, WebrootVue.qvt.js registered them before the shell, and the shell has the button and the drawer
    Object.keys(assist.components).forEach((name) => {
        assert.equal(qvt.components[name], assist.components[name], name);
        const errors = [];
        compiler.compile(assist.components[name].template, { mode: "function", onError: (err) => errors.push(err.message) });
        assert.deepEqual(errors, [], name);
    });
    const shell = qvt.components["m-qapps-root"].template;
    assert.match(shell, /<m-assist-button v-if="assistOn"><\/m-assist-button>\n {4}<\/q-toolbar><\/q-header>/);
    assert.match(shell, /<m-shell-assist v-if="assistOn"><\/m-shell-assist>/);
    // the stable ids and data attributes the browser smoke drives
    const drawer = assist.components["m-shell-assist"].template + assist.components["m-assist-composer"].template + assist.components["m-assist-card"].template;
    ["m-assist-drawer", "m-assist-send", "m-assist-stop", "m-assist-follow-here", "m-assist-history", "m-assist-tasks"].forEach((id) =>
        assert.match(drawer, new RegExp('id="' + id + '"'), id));
    // the composer's id is the textarea's own (q-input puts its for on the native control), so focus and the smoke reach it
    assert.match(drawer, /for="m-assist-input"/);
    // persistent: an overlay q-drawer would close itself on every navigation of the shell and on Esc
    assert.match(drawer, /<q-drawer v-model="open" side="right" bordered persistent /);
    ["approve", "edit", "reject", "reject-send", "preview", "skip", "answer"].forEach((action) => assert.match(drawer, new RegExp('data-assist-action="' + action + '"'), action));
});
