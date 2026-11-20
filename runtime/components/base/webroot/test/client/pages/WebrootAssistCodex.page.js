/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// The Workbench agent of the Assist drawer (js/WebrootAssistCodex.js), in the page: its pure parts, and the view mounted with
// Vue and Quasar against a fake app-server behind a fake WebSocket that speaks Codex's app-server protocol as the host
// passes it (/codexws): the sign in, a thread, a streamed turn with a command, an approval, a question, Stop and steer.
"use strict";
const codex = moqui.assistCodex;

test("helpers: user text, decisions, forms, MCP results, inbox names, approval links", () => {
    assert.equal(codex.userText([{ type: "text", text: "Read" }, { type: "mention", name: "bom.pdf", path: "/workspace/inbox/bom.pdf" },
        { type: "localImage", path: "/workspace/inbox/a.png" }]), "Read @bom.pdf [a.png]");
    assert.deepEqual(codex.decisionsOf({ method: "item/commandExecution/requestApproval", params: {} }), ["accept", "acceptForSession", "decline", "cancel"]);
    assert.deepEqual(codex.decisionsOf({ method: "item/commandExecution/requestApproval", params: { availableDecisions: ["accept", "decline"] } }), ["accept", "decline"]);
    assert.deepEqual(codex.decisionsOf({ method: "item/fileChange/requestApproval", params: {} }), ["accept", "acceptForSession", "decline", "cancel"]);
    assert.deepEqual(codex.decisionsOf({ method: "item/tool/requestUserInput", params: {} }), []);
    assert.equal(codex.decisionLabel("acceptForSession"), "Approve for this session");
    assert.equal(codex.decisionLabel({ acceptWithExecpolicyAmendment: { execpolicy_amendment: ["ls"] } }), "Always allow commands like this");
    const fields = codex.formFields({ properties: { qty: { type: "integer", title: "Quantity" }, ok: { type: "boolean" },
        color: { type: "string", enum: ["r", "g"], enumNames: ["Red", "Green"] } }, required: ["qty"] });
    assert.deepEqual(fields.map((field) => [field.name, field.type, field.required]), [["qty", "integer", true], ["ok", "boolean", false], ["color", "string", false]]);
    assert.deepEqual(fields[2].options, [{ value: "r", label: "Red" }, { value: "g", label: "Green" }]);
    fields[0].value = "3"; fields[2].value = "g";
    assert.deepEqual(codex.formContent(fields), { qty: 3, ok: false, color: "g" });
    assert.equal(codex.mcpResultText({ content: [{ type: "text", text: "a" }, { type: "image" }] }), "a\n[image]");
    assert.equal(codex.inboxName("../../etc/passwd"), "passwd");
    assert.equal(codex.inboxName(".env"), "_env");
    assert.equal(codex.inboxName("Datasheet (rev 2).pdf"), "Datasheet _rev 2_.pdf");
    assert.equal(codex.elicitationHref("https://host.docker.internal:8445/qapps?assist=100230", "http://localhost:8080"), "http://localhost:8080/qapps?assist=100230");
    assert.equal(codex.elicitationHref("https://example.com/approve?x=1", "http://localhost:8080"), "https://example.com/approve?x=1");
    assert.equal(codex.elicitationHref("javascript:alert(1)", "http://localhost:8080"), null);
    assert.equal(codex.tail("x".repeat(25000)).length, 20001);
});

// ---- a fake app-server behind a fake socket ----

const server = { received: [], signedIn: false, turn: 0 };
class FakeSocket {
    constructor(url) {
        this.url = url;
        this.readyState = 0;
        server.socket = this;
        setTimeout(() => { this.readyState = 1; if (this.onopen) this.onopen(); }, 0);
    }
    send(text) {
        const message = JSON.parse(text);
        server.received.push(message);
        setTimeout(() => server.handle(message), 0);
    }
    close(code, reason) { this.readyState = 3; if (this.onclose) this.onclose({ code: code || 1000, reason: reason || "" }); }
}
const push = (message) => server.socket.onmessage({ data: JSON.stringify(Object.assign({ jsonrpc: "2.0" }, message)) });
const answer = (id, result) => push({ id, result });
const notify = (method, params) => push({ method, params });
server.handle = (message) => {
    if (message.method === undefined) return;
    const params = message.params || {};
    switch (message.method) {
    case "initialize": answer(message.id, { userAgent: "codex-fake" }); break;
    case "account/read": answer(message.id, { account: server.signedIn ? { type: "chatgpt", email: null, planType: "plus" } : null, requiresOpenaiAuth: true }); break;
    case "account/login/start": answer(message.id, { type: "chatgptDeviceCode", loginId: "L1", verificationUrl: "https://auth.example/device", userCode: "ABCD-EFGH" }); break;
    case "model/list": answer(message.id, { data: [{ model: "gpt-x", displayName: "GPT X", isDefault: true, defaultReasoningEffort: "medium",
        supportedReasoningEfforts: [{ reasoningEffort: "low", description: "" }, { reasoningEffort: "medium", description: "" }] }] }); break;
    case "thread/start": answer(message.id, { thread: { id: "T1", name: null, preview: "", turns: [] }, model: "gpt-x" }); break;
    case "thread/list": answer(message.id, { data: [{ id: "T0", name: "Datasheets", preview: "find the datasheets", updatedAt: 1763650000 }], nextCursor: null }); break;
    case "thread/resume": answer(message.id, { thread: { id: "T0", name: "Datasheets", preview: "", turns: [{ id: "U0", status: "completed", items: [
        { type: "userMessage", id: "i0", content: [{ type: "text", text: "find the datasheets" }] },
        { type: "agentMessage", id: "i1", text: "Found **3** datasheets." }] }] }, model: "gpt-x" }); break;
    case "turn/start": {
        const turnId = "U" + (++server.turn);
        answer(message.id, { turn: { id: turnId, status: "inProgress", items: [] } });
        notify("turn/started", { threadId: params.threadId, turn: { id: turnId, status: "inProgress", items: [] } });
        notify("item/started", { threadId: params.threadId, turnId, item: { type: "userMessage", id: "u" + turnId, content: params.input } });
        notify("item/started", { threadId: params.threadId, turnId, item: { type: "commandExecution", id: "c" + turnId, command: "python3 bom.py", cwd: "/workspace", status: "inProgress", aggregatedOutput: "" } });
        notify("item/commandExecution/outputDelta", { threadId: params.threadId, turnId, itemId: "c" + turnId, delta: "3 rows\n" });
        notify("item/completed", { threadId: params.threadId, turnId, item: { type: "commandExecution", id: "c" + turnId, command: "python3 bom.py", cwd: "/workspace", status: "completed", exitCode: 0, aggregatedOutput: "3 rows\n" } });
        notify("item/started", { threadId: params.threadId, turnId, item: { type: "agentMessage", id: "a" + turnId, text: "" } });
        notify("item/agentMessage/delta", { threadId: params.threadId, turnId, itemId: "a" + turnId, delta: "The BOM has " });
        notify("item/agentMessage/delta", { threadId: params.threadId, turnId, itemId: "a" + turnId, delta: "3 rows." });
        // Codex asks before a command
        push({ id: 900 + server.turn, method: "item/commandExecution/requestApproval", params: { threadId: params.threadId, turnId, itemId: "x" + turnId,
            command: "curl https://example.com/ds.pdf", cwd: "/workspace", reason: "download the datasheet", startedAtMs: 1 } });
        break;
    }
    case "turn/steer": answer(message.id, { turnId: params.expectedTurnId }); break;
    case "turn/interrupt": answer(message.id, {});
        notify("turn/completed", { threadId: params.threadId, turn: { id: params.turnId, status: "interrupted", items: [] } }); break;
    default: push({ id: message.id, error: { code: -32601, message: "not in the fake" } });
    }
};
globalThis.WebSocket = FakeSocket;

const host = document.createElement("div");
document.body.appendChild(host);
const app = Vue.createApp({ data: () => ({ workbench: { agentId: "codex", name: "Workbench", endpoint: "/codexws" } }),
    template: '<div style="height:700px;width:420px;display:flex"><m-assist-codex :workbench="workbench" style="flex:1"></m-assist-codex></div>' });
app.use(Quasar);
app.component("m-assist-codex", codex.component);
if (moqui.qvt && moqui.qvt.components) Object.keys(moqui.qvt.components).forEach((name) => app.component(name, moqui.qvt.components[name]));
app.config.globalProperties.moqui = moqui;
app.mount(host);

const settle = async () => { for (let i = 0; i < 8; i++) { await Vue.nextTick(); await new Promise((resolve) => setTimeout(resolve, 0)); } };
const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => Array.from(document.querySelectorAll(selector));
const sent = (method) => server.received.filter((message) => message.method === method);
const responses = () => server.received.filter((message) => message.method === undefined && message.id !== undefined);
const click = (el) => { assert.equal(!!el, true, "the element is there"); el.click(); };
const type = async (text) => {
    const input = $("textarea[data-codex-composer]") || $("[data-codex-composer] textarea");
    input.value = text;
    input.dispatchEvent(new Event("input"));
    await settle();
};

test("connected: the page's initialize, then the account and the models; not signed in, the sign in card", async () => {
    await settle();
    assert.equal(sent("initialize").length, 1);
    assert.equal(sent("initialize")[0].params.capabilities.experimentalApi, true);
    assert.equal(sent("account/read").length >= 1, true);
    assert.equal(sent("model/list").length, 1);
    assert.equal(!!$("[data-codex-signin]"), true);
    click($("[data-codex-action=login-chatgpt]"));
    await settle();
    assert.deepEqual(sent("account/login/start")[0].params, { type: "chatgptDeviceCode" });
    assert.equal($("[data-codex-user-code]").textContent, "ABCD-EFGH");
    assert.equal($("[data-codex-device-code] a").getAttribute("href"), "https://auth.example/device");
    server.signedIn = true;
    notify("account/login/completed", { loginId: "L1", success: true });
    await settle();
    assert.equal(!!$("[data-codex-signin]"), false, "signed in: the thread and the composer");
    assert.equal(!!$("[data-codex-composer]"), true);
});

test("a message starts a thread in /workspace and a turn; the turn streams its command, its answer and an approval", async () => {
    await type("Read the BOM");
    click($("[data-codex-action=send]"));
    await settle();
    assert.equal(sent("thread/start")[0].params.cwd, "/workspace");
    const turn = sent("turn/start")[0].params;
    assert.equal(turn.threadId, "T1");
    assert.equal(turn.model, "gpt-x");
    assert.equal(turn.effort, "medium");
    assert.deepEqual(turn.input, [{ type: "text", text: "Read the BOM", text_elements: [] }]);
    assert.equal($("[data-codex-item=userMessage]").textContent.trim(), "Read the BOM");
    const command = $("[data-codex-item=commandExecution]");
    assert.equal(command.getAttribute("data-status"), "completed");
    assert.equal(command.textContent.includes("python3 bom.py"), true);
    command.querySelector(".cursor-pointer").click();
    await settle();
    assert.equal($("[data-codex-item=commandExecution] pre").textContent, "3 rows\n");
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal($("[data-codex-item=agentMessage]").textContent.includes("The BOM has 3 rows."), true);
    const card = $("[data-codex-request='item/commandExecution/requestApproval']");
    assert.equal(card.textContent.includes("curl https://example.com/ds.pdf"), true);
    assert.equal(card.textContent.includes("download the datasheet"), true);
    assert.deepEqual($$("[data-codex-request] [data-codex-decision]").map((el) => el.getAttribute("data-codex-decision")), ["accept", "acceptForSession", "decline", "cancel"]);
    click(card.querySelector("[data-codex-decision=acceptForSession]"));
    await settle();
    assert.deepEqual(responses().pop(), { jsonrpc: "2.0", id: 901, result: { decision: "acceptForSession" } });
    assert.equal(!!$("[data-codex-request]"), false, "answered, the card goes");
});

test("typed while the turn runs it steers; Stop interrupts the turn", async () => {
    await type("only the valves");
    click($("[data-codex-action=send]"));
    await settle();
    const steer = sent("turn/steer")[0].params;
    assert.equal(steer.expectedTurnId, "U1");
    assert.deepEqual(steer.input, [{ type: "text", text: "only the valves", text_elements: [] }]);
    click($("[data-codex-action=stop]"));
    await settle();
    assert.deepEqual(sent("turn/interrupt")[0].params, { threadId: "T1", turnId: "U1" });
    assert.equal($("[data-codex-turn=U1]").getAttribute("data-turn-status"), "interrupted");
    assert.equal(!!$("[data-codex-action=stop]"), false);
});

test("a question and a Moqui approval link: answered as the user decides", async () => {
    push({ id: 950, method: "item/tool/requestUserInput", params: { threadId: "T1", turnId: "U1", itemId: "q1", isBlocking: true,
        questions: [{ id: "unit", header: "Unit", question: "Which unit?", options: [{ label: "mm", description: "" }, { label: "inch", description: "" }] }] } });
    push({ id: 951, method: "mcpServer/elicitation/request", params: { serverName: "moqui", threadId: "T1", turnId: "U1", mode: "url", elicitationId: "e1",
        message: "Create products: this waits for your approval in Moqui.", url: "https://host.docker.internal:8445/qapps?assist=100230" } });
    await settle();
    const link = $("[data-codex-elicitation-url]");
    assert.equal(link.getAttribute("href"), location.origin + "/qapps?assist=100230", "Moqui's approval on this page's origin");
    click($("[data-codex-request='mcpServer/elicitation/request'] [data-codex-decision=accept]"));
    await settle();
    assert.deepEqual(responses().pop(), { jsonrpc: "2.0", id: 951, result: { action: "accept" } });
    const question = $("[data-codex-request='item/tool/requestUserInput']");
    question.querySelectorAll("[role=radio]")[1].click();
    await settle();
    click(Array.from(question.querySelectorAll("button")).find((button) => button.textContent.includes("Answer")));
    await settle();
    assert.deepEqual(responses().pop(), { jsonrpc: "2.0", id: 950, result: { answers: { unit: { answers: ["inch"] } } } });
});

test("a request this client does not serve is refused; the time is answered", async () => {
    push({ id: 960, method: "attestation/generate", params: {} });
    push({ id: 961, method: "currentTime/read", params: {} });
    await settle();
    const all = responses();
    assert.equal(all.find((message) => message.id === 960).error.code, -32601);
    assert.equal(typeof all.find((message) => message.id === 961).result.currentTimeAt, "number");
});

test("the threads: listed from the app-server, and one opened with its turns", async () => {
    click($("[data-codex-action=history]"));
    await settle();
    assert.equal(sent("thread/list").length, 1);
    click($("[data-codex-thread=T0]"));
    await settle();
    assert.deepEqual(sent("thread/resume")[0].params, { threadId: "T0" });
    assert.equal($("[data-codex-item=userMessage]").textContent.trim(), "find the datasheets");
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.equal($("[data-codex-item=agentMessage]").textContent.includes("Found 3 datasheets."), true);
});

test("a dropped connection says so and connects again", async () => {
    server.socket.onclose({ code: 1006, reason: "" });
    await settle();
    assert.equal(!!$("[data-codex-connection]"), true);
    await new Promise((resolve) => setTimeout(resolve, 1200));
    await settle();
    assert.equal(sent("initialize").length, 2, "connected again");
    assert.equal(!!$("[data-codex-connection]"), false);
    assert.equal(sent("thread/resume").length, 2, "the open thread resumed");
});
