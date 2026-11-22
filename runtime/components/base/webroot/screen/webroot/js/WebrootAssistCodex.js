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

/*
 * The Workbench agent of the Assist drawer: a client of Codex's app-server protocol, as the Codex extension's webview is
 * in VS Code. The server (/codexws, crates/moqui-host/src/codexws.rs) is the extension's host: it opens the user's
 * sandbox, initializes the connection and passes the JSON-RPC messages both ways unchanged. Everything here is the
 * protocol of the pinned Codex (moqui-agent-workbench/sandbox/schema):
 *
 * - threads: thread/list (the history), thread/start (cwd /workspace), thread/resume (a thread with its turns),
 *   thread/fork (a copy to go on from), thread/compact/start (the context compacted now);
 * - reviews: review/start of the workspace's uncommitted changes, or as the draft's instructions say, inline;
 * - turns: turn/start (text, and files the user adds written to /workspace/inbox with fs/writeFile and given as
 *   mentions, images as localImage), turn/steer while a turn runs, turn/interrupt (Stop);
 * - what a turn does: item/started and item/completed, with the deltas of agent messages, reasoning summaries, command
 *   output and plans; turn/plan/updated, turn/diff/updated, thread/tokenUsage/updated, error;
 * - what Codex asks: command, file change and permission approvals, questions (item/tool/requestUserInput) and MCP
 *   elicitations (a Moqui write's approval link is one, mode url), answered as the user decides;
 * - the account: account/read, account/login/start (the ChatGPT device code, or an API key), account/login/completed;
 *   model/list for the model and the reasoning effort.
 *
 * It registers moqui.assistCodex.component, which the drawer's m-assist-codex-host draws when the Workbench is the agent.
 */
(function (root) {
    "use strict";

    const moqui = root.moqui = root.moqui || {};
    const codex = moqui.assistCodex = moqui.assistCodex || {};

    const WORKSPACE = "/workspace";
    const INBOX = WORKSPACE + "/inbox";
    const MAX_OUTPUT = 20000;

    // ---- the connection: JSON-RPC 2.0 over the host's socket ----

    // One client for the page, kept while the drawer switches panes or agents; it reconnects after a drop and the view
    // resumes its thread.
    class Client {
        constructor(endpoint) {
            this.endpoint = endpoint;
            this.nextId = 1;
            this.pending = new Map();
            this.listeners = new Set();
            this.ws = null;
            this.status = "closed";
            this.closeReason = "";
            this.retry = 0;
        }
        url() {
            const loc = root.location;
            return (loc.protocol === "https:" ? "wss://" : "ws://") + loc.host + this.endpoint;
        }
        on(listener) { this.listeners.add(listener); return () => this.listeners.delete(listener); }
        emit(event) { this.listeners.forEach((listener) => { try { listener(event); } catch (err) { console.warn("Workbench listener", err); } }); }
        connect() {
            if (this.ws && (this.status === "open" || this.status === "connecting")) return;
            this.status = "connecting";
            this.emit({ type: "status" });
            const ws = this.ws = new root.WebSocket(this.url());
            ws.onopen = () => {
                // the host initialized the connection; the page's initialize gets the app-server's answer
                this.request("initialize", { clientInfo: { name: "moqui_workbench", title: "Moqui Workbench", version: "1" }, capabilities: { experimentalApi: true } })
                    .then((result) => { this.status = "open"; this.retry = 0; this.closeReason = ""; this.emit({ type: "open", result }); },
                        (err) => { this.closeReason = err.message || "initialize failed"; ws.close(); });
            };
            ws.onmessage = (event) => {
                let message;
                try { message = JSON.parse(event.data); } catch (err) { return; }
                if (message.id !== undefined && message.method === undefined) {
                    const waiting = this.pending.get(message.id);
                    if (!waiting) return;
                    this.pending.delete(message.id);
                    if (message.error) waiting.reject(Object.assign(new Error(message.error.message || "error"), { code: message.error.code, data: message.error.data }));
                    else waiting.resolve(message.result);
                } else if (message.id !== undefined) {
                    this.emit({ type: "request", id: message.id, method: message.method, params: message.params || {} });
                } else if (message.method) {
                    this.emit({ type: "notification", method: message.method, params: message.params || {} });
                }
            };
            ws.onclose = (event) => {
                if (this.ws !== ws) return;
                this.ws = null;
                this.status = "closed";
                this.closeReason = event.reason || this.closeReason || "";
                this.pending.forEach((waiting) => waiting.reject(new Error(this.closeReason || "the Workbench connection closed")));
                this.pending.clear();
                this.emit({ type: "close", reason: this.closeReason, code: event.code });
                // a refusal (no permission, not signed in) is not tried again; a drop is, with a growing wait
                if (event.code === 1006 || event.code === 1001 || (event.code === 1000 && /closed the connection|stopping/.test(this.closeReason))) {
                    const wait = Math.min(30000, 1000 * Math.pow(2, this.retry++));
                    setTimeout(() => { if (this.status === "closed" && this.wanted) this.connect(); }, wait);
                }
            };
        }
        close() { this.wanted = false; if (this.ws) this.ws.close(1000, "closed"); }
        send(message) {
            if (!this.ws || this.ws.readyState !== 1) return false;
            this.ws.send(JSON.stringify(Object.assign({ jsonrpc: "2.0" }, message)));
            return true;
        }
        request(method, params) {
            return new Promise((resolve, reject) => {
                const id = this.nextId++;
                this.pending.set(id, { resolve, reject });
                if (!this.send({ id, method, params: params || {} })) {
                    this.pending.delete(id);
                    reject(new Error("the Workbench is not connected"));
                }
            });
        }
        respond(id, result) { return this.send({ id, result }); }
        respondError(id, code, message) { return this.send({ id, error: { code, message } }); }
    }
    codex.Client = Client;
    codex.client = (endpoint) => {
        if (!codex.shared || codex.shared.endpoint !== endpoint) codex.shared = new Client(endpoint);
        codex.shared.wanted = true;
        return codex.shared;
    };

    // ---- pure helpers (tested in WebrootAssistCodex.spec) ----

    // The text of a user message's content.
    codex.userText = (content) => (content || []).map((part) => part.type === "text" ? part.text
        : part.type === "mention" ? "@" + (part.name || part.path)
        : part.type === "localImage" ? "[" + (part.path || "image").split("/").pop() + "]"
        : part.type === "image" ? "[image]" : "").filter(Boolean).join(" ");

    // The label of an approval decision, a string or an object one.
    codex.decisionLabel = (decision) => {
        if (typeof decision === "string") {
            return { accept: "Approve", acceptForSession: "Approve for this session", decline: "Decline", cancel: "Decline and stop",
                approved: "Approve", approved_for_session: "Approve for this session", denied: "Decline", abort: "Decline and stop" }[decision] || decision;
        }
        if (decision && decision.acceptWithExecpolicyAmendment) return "Always allow commands like this";
        if (decision && decision.applyNetworkPolicyAmendment) {
            const rule = decision.applyNetworkPolicyAmendment.network_policy_amendment || {};
            return (rule.action === "deny" ? "Always deny " : "Always allow ") + (rule.host || "this host");
        }
        return JSON.stringify(decision);
    };

    // The decisions a request offers: its own list, else the protocol's.
    codex.decisionsOf = (request) => {
        const params = request.params || {};
        if (request.method === "item/commandExecution/requestApproval") {
            return Array.isArray(params.availableDecisions) && params.availableDecisions.length ? params.availableDecisions
                : ["accept", "acceptForSession", "decline", "cancel"];
        }
        if (request.method === "item/fileChange/requestApproval") return ["accept", "acceptForSession", "decline", "cancel"];
        if (request.method === "execCommandApproval" || request.method === "applyPatchApproval") return ["approved", "approved_for_session", "denied", "abort"];
        return [];
    };

    // The fields of an MCP form elicitation: its requested schema's properties.
    codex.formFields = (schema) => {
        const properties = (schema && schema.properties) || {};
        const required = (schema && schema.required) || [];
        return Object.keys(properties).map((name) => {
            const property = properties[name] || {};
            const options = Array.isArray(property.enum) ? property.enum.map((value, index) => ({ value, label: (property.enumNames || [])[index] || String(value) }))
                : Array.isArray(property.oneOf) ? property.oneOf.map((option) => ({ value: option.const, label: option.title || String(option.const) })) : null;
            return { name, title: property.title || name, description: property.description || "", type: property.type || "string",
                options, required: required.indexOf(name) >= 0, value: property.default !== undefined ? property.default : (property.type === "boolean" ? false : "") };
        });
    };

    // The values of a form, typed as its schema says.
    codex.formContent = (fields) => {
        const content = {};
        fields.forEach((field) => {
            if (field.value === "" && !field.required) return;
            if (field.type === "number" || field.type === "integer") content[field.name] = Number(field.value);
            else if (field.type === "boolean") content[field.name] = !!field.value;
            else content[field.name] = field.value;
        });
        return content;
    };

    // The text of an MCP tool result's content.
    codex.mcpResultText = (result) => {
        if (!result) return "";
        return (result.content || []).map((part) => part && part.type === "text" ? part.text
            : part && part.type ? "[" + part.type + "]" : JSON.stringify(part)).join("\n");
    };

    // A file name safe in the inbox: its base name, plain characters only.
    codex.inboxName = (name) => {
        const base = String(name || "file").split(/[\\/]/).pop().replace(/[^A-Za-z0-9._ -]/g, "_").replace(/^\.+/, "_").slice(0, 120);
        return base || "file";
    };

    // The last part of a long output.
    codex.tail = (text) => {
        const value = text || "";
        return value.length > MAX_OUTPUT ? "…" + value.slice(value.length - MAX_OUTPUT) : value;
    };

    // The link of a URL elicitation as the page opens it: Moqui's own approval link (its Assist drawer on the call's
    // conversation, `/qapps?assist=<id>`) on this page's origin, since the sandbox reaches Moqui by another name; any other
    // link as it is, when it is http(s).
    codex.elicitationHref = (url, origin) => {
        let parsed;
        try { parsed = new URL(url); } catch (err) { return null; }
        if (parsed.protocol !== "https:" && parsed.protocol !== "http:") return null;
        if (/^\/qapps(\/|$)/.test(parsed.pathname) && parsed.searchParams.has("assist")) return origin + parsed.pathname + parsed.search;
        return parsed.href;
    };

    // A path the Files panel shows, kept inside /workspace.
    codex.workspacePath = (path) => {
        const parts = [];
        for (const part of String(path || WORKSPACE).split("/")) {
            if (!part || part === ".") continue;
            if (part === "..") parts.pop(); else parts.push(part);
        }
        const joined = "/" + parts.join("/");
        return joined === WORKSPACE || joined.startsWith(WORKSPACE + "/") ? joined : WORKSPACE;
    };

    // The base64 of a file the user adds.
    const readBase64 = (file) => new Promise((resolve, reject) => {
        const reader = new root.FileReader();
        reader.onload = () => { const url = String(reader.result || ""); resolve(url.slice(url.indexOf(",") + 1)); };
        reader.onerror = () => reject(reader.error || new Error("the file did not read"));
        reader.readAsDataURL(file);
    });

    // ---- the view ----

    const emptyThread = () => ({ id: null, name: "", preview: "", turns: [], activeTurnId: null, plan: null, diff: "", usage: null, status: "" });

    codex.component = {
        name: "mAssistCodex",
        props: { workbench: { type: Object, required: true } },
        data() {
            return {
                status: "connecting", error: "", account: undefined, requiresAuth: false, login: null, apiKey: "", loginBusy: false,
                models: [], model: null, effort: null, approval: "on-request", pane: "chat",
                browse: { open: false, path: "", entries: [], loading: false, busy: null, error: "" }, threads: [], threadsCursor: null, threadsLoading: false,
                thread: emptyThread(), requests: [], draft: "", files: [], sending: false, rateLimits: null, notice: ""
            };
        },
        computed: {
            running() { return !!this.thread.activeTurnId; },
            modelOptions() { return this.models.filter((model) => !model.hidden).map((model) => ({ label: model.displayName || model.model, value: model.model })); },
            // when Codex asks before it acts, as the extension's modes: before any command that is not a plain read, when it
            // judges it needs to, or never (full access); the turn's approvalPolicy
            approvalOptions() {
                return [{ label: "Ask before commands", value: "untrusted" }, { label: "Ask when needed", value: "on-request" },
                    { label: "Full access", value: "never" }];
            },
            effortOptions() {
                const model = this.models.find((one) => one.model === this.model);
                return model && model.supportedReasoningEfforts ? model.supportedReasoningEfforts.map((option) => ({ label: option.reasoningEffort, value: option.reasoningEffort })) : [];
            },
            usageText() {
                const usage = this.thread.usage;
                if (!usage || !usage.total) return "";
                const used = usage.total.totalTokens || 0;
                return usage.modelContextWindow ? Math.round(100 * (usage.last ? usage.last.inputTokens || 0 : used) / usage.modelContextWindow) + "% of context" : used + " tokens";
            },
            signedIn() { return !!this.account; },
            canSend() { return this.status === "open" && this.signedIn && (this.draft.trim() || this.files.length) && !this.sending; }
        },
        created() {
            this.items = new Map();
            this.client = codex.client(this.workbench.endpoint);
            this.off = this.client.on((event) => this.onEvent(event));
            if (this.client.status === "open") this.onOpen(); else this.client.connect();
            this.status = this.client.status;
        },
        beforeUnmount() { if (this.off) this.off(); },
        methods: {
            // ---- the connection ----
            onEvent(event) {
                if (event.type === "status") this.status = this.client.status;
                else if (event.type === "open") { this.status = "open"; this.error = ""; this.onOpen(); }
                else if (event.type === "close") {
                    this.status = "closed";
                    this.error = event.reason || "The Workbench is not connected.";
                    this.requests = [];
                } else if (event.type === "notification") this.onNotification(event.method, event.params);
                else if (event.type === "request") this.onRequest(event);
            },
            onOpen() {
                this.status = "open";
                this.readAccount();
                this.client.request("model/list", { limit: 50 }).then((result) => {
                    this.models = (result && result.data) || [];
                    const preferred = this.models.find((model) => model.isDefault) || this.models[0];
                    if (!this.model && preferred) { this.model = preferred.model; this.effort = preferred.defaultReasoningEffort || null; }
                }, () => {});
                // a thread open before a drop is resumed: its turns again, and its running turn's events
                if (this.thread.id) this.openThread(this.thread.id);
            },
            reconnect() { this.client.wanted = true; this.client.connect(); },
            call(method, params) {
                return this.client.request(method, params).catch((err) => { this.notice = method + ": " + (err.message || err); throw err; });
            },

            // ---- the account ----
            readAccount() {
                return this.client.request("account/read", { refreshToken: false }).then((result) => {
                    this.account = result.account || null;
                    this.requiresAuth = !!result.requiresOpenaiAuth;
                }, (err) => { this.account = null; this.notice = err.message; });
            },
            loginChatgpt() {
                this.loginBusy = true;
                this.call("account/login/start", { type: "chatgptDeviceCode" }).then((result) => { this.login = result; })
                    .finally(() => { this.loginBusy = false; });
            },
            loginApiKey() {
                const key = this.apiKey.trim();
                if (!key) return;
                this.loginBusy = true;
                this.call("account/login/start", { type: "apiKey", apiKey: key }).then(() => { this.apiKey = ""; return this.readAccount(); })
                    .finally(() => { this.loginBusy = false; });
            },
            cancelLogin() {
                if (this.login && this.login.loginId) this.client.request("account/login/cancel", { loginId: this.login.loginId }).catch(() => {});
                this.login = null;
            },
            logout() { this.call("account/logout", {}).then(() => this.readAccount()); },

            // ---- threads ----
            showHistory() {
                this.pane = this.pane === "history" ? "chat" : "history";
                if (this.pane === "history") this.loadThreads(false);
            },
            loadThreads(more) {
                this.threadsLoading = true;
                const params = { limit: 25, sortKey: "updated_at" };
                if (more && this.threadsCursor) params.cursor = this.threadsCursor;
                this.call("thread/list", params).then((result) => {
                    const list = (result && result.data) || [];
                    this.threads = more ? this.threads.concat(list) : list;
                    this.threadsCursor = (result && result.nextCursor) || null;
                }).finally(() => { this.threadsLoading = false; });
            },
            newThread() {
                this.thread = emptyThread();
                this.items = new Map();
                this.requests = [];
                this.pane = "chat";
            },
            startThread() {
                return this.call("thread/start", { cwd: WORKSPACE, model: this.model || null }).then((result) => {
                    this.thread.id = result.thread.id;
                    this.thread.name = result.thread.name || "";
                    return result.thread.id;
                });
            },
            openThread(threadId) {
                this.pane = "chat";
                return this.call("thread/resume", { threadId }).then((result) => {
                    const thread = result.thread;
                    const view = emptyThread();
                    view.id = thread.id;
                    view.name = thread.name || "";
                    view.preview = thread.preview || "";
                    const turns = (result.initialTurnsPage && result.initialTurnsPage.data) || thread.turns || [];
                    view.turns = turns.map((turn) => this.turnView(turn));
                    const active = turns.find((turn) => turn.status === "inProgress");
                    view.activeTurnId = active ? active.id : null;
                    this.thread = view;
                    // the item map holds the reactive items, which the deltas change
                    this.items = new Map();
                    this.thread.turns.forEach((turn) => turn.items.forEach((item) => this.items.set(item.id, item)));
                    if (result.model) this.model = result.model;
                    if (result.reasoningEffort) this.effort = result.reasoningEffort;
                    this.scrollDown();
                });
            },
            turnView(turn) {
                const view = { id: turn.id, status: turn.status, error: turn.error || null, items: [] };
                (turn.items || []).forEach((item) => view.items.push(this.itemView(item)));
                return view;
            },
            itemView(item) {
                const view = Object.assign({ open: false }, item);
                if (view.type === "commandExecution") view.aggregatedOutput = codex.tail(view.aggregatedOutput || "");
                if (view.type === "reasoning") view.summaryText = (view.summary || []).join("\n\n");
                return view;
            },
            turnOf(turnId) {
                let turn = this.thread.turns.find((one) => one.id === turnId);
                if (!turn && turnId) {
                    turn = { id: turnId, status: "inProgress", error: null, items: [] };
                    this.thread.turns.push(turn);
                }
                return turn;
            },
            // the answer to turn/start or review/start: the turn runs, unless its turn/completed came first
            turnBegun(turnId) {
                if (this.turnOf(turnId).status === "inProgress") this.thread.activeTurnId = turnId;
            },
            mine(params) { return params && params.threadId && params.threadId === this.thread.id; },

            // ---- what the app-server says ----
            onNotification(method, params) {
                if (method === "account/login/completed") {
                    this.login = null;
                    if (!params.success) this.notice = params.error || "Sign in failed";
                    this.readAccount();
                    return;
                }
                if (method === "account/updated") { this.readAccount(); return; }
                if (method === "account/rateLimits/updated") { this.rateLimits = params.rateLimits || params; return; }
                if (method === "serverRequest/resolved") {
                    this.requests = this.requests.filter((request) => request.id !== params.requestId);
                    return;
                }
                if (method === "thread/name/updated" && this.mine(params)) { this.thread.name = params.threadName || params.name || ""; return; }
                if (!this.mine(params)) return;
                switch (method) {
                case "turn/started": {
                    const turn = this.turnOf(params.turn.id);
                    turn.status = "inProgress";
                    this.thread.activeTurnId = params.turn.id;
                    this.thread.plan = null;
                    break;
                }
                case "turn/completed": {
                    const turn = this.turnOf(params.turn.id);
                    turn.status = params.turn.status;
                    turn.error = params.turn.error || null;
                    if (this.thread.activeTurnId === params.turn.id) this.thread.activeTurnId = null;
                    this.requests = this.requests.filter((request) => !request.params || request.params.turnId !== params.turn.id);
                    break;
                }
                case "item/started":
                case "item/completed": {
                    const turn = this.turnOf(params.turnId);
                    const known = this.items.get(params.item.id);
                    if (known) {
                        const open = known.open;
                        Object.assign(known, params.item, { open });
                        if (known.type === "commandExecution") known.aggregatedOutput = codex.tail(params.item.aggregatedOutput || known.aggregatedOutput || "");
                        if (known.type === "reasoning" && params.item.summary && params.item.summary.length) known.summaryText = params.item.summary.join("\n\n");
                    } else if (turn) {
                        turn.items.push(this.itemView(params.item));
                        this.items.set(params.item.id, turn.items[turn.items.length - 1]);
                    }
                    break;
                }
                case "item/agentMessage/delta": this.appendTo(params.itemId, "text", params.delta); break;
                case "item/plan/delta": this.appendTo(params.itemId, "text", params.delta); break;
                case "item/reasoning/summaryTextDelta": this.appendTo(params.itemId, "summaryText", params.delta); break;
                case "item/reasoning/summaryPartAdded": this.appendTo(params.itemId, "summaryText", "\n\n"); break;
                case "item/commandExecution/outputDelta": {
                    const item = this.items.get(params.itemId);
                    if (item) item.aggregatedOutput = codex.tail((item.aggregatedOutput || "") + (params.delta || ""));
                    break;
                }
                case "turn/plan/updated": this.thread.plan = { explanation: params.explanation || "", steps: params.plan || [] }; break;
                case "turn/diff/updated": this.thread.diff = params.diff || ""; break;
                case "thread/tokenUsage/updated": this.thread.usage = params.tokenUsage; break;
                case "error": {
                    const turn = this.turnOf(params.turnId);
                    if (turn && !params.willRetry) turn.error = params.error;
                    break;
                }
                default: break;
                }
                this.scrollDown();
            },
            appendTo(itemId, field, delta) {
                const item = this.items.get(itemId);
                if (item) item[field] = (item[field] || "") + (delta || "");
            },
            onRequest(request) {
                // the time, when Codex asks for it
                if (request.method === "currentTime/read") { this.client.respond(request.id, { currentTimeAt: Math.floor(Date.now() / 1000) }); return; }
                // no dynamic tools are declared by this client: Moqui's services come to Codex over MCP
                if (request.method === "item/tool/call") {
                    this.client.respond(request.id, { success: false, contentItems: [{ type: "inputText", text: "This client declares no dynamic tools." }] });
                    return;
                }
                const known = ["item/commandExecution/requestApproval", "item/fileChange/requestApproval", "item/permissions/requestApproval",
                    "item/tool/requestUserInput", "mcpServer/elicitation/request", "execCommandApproval", "applyPatchApproval"];
                if (known.indexOf(request.method) < 0) { this.client.respondError(request.id, -32601, "not supported by this client"); return; }
                const card = { id: request.id, method: request.method, params: request.params, busy: false };
                if (request.method === "item/tool/requestUserInput") {
                    card.answers = {};
                    (request.params.questions || []).forEach((question) => { card.answers[question.id] = ""; });
                }
                if (request.method === "mcpServer/elicitation/request" && request.params.mode === "form") card.fields = codex.formFields(request.params.requestedSchema);
                // a request of another thread (one this page does not show) is shown too: it waits for this user alone
                this.requests.push(card);
                this.scrollDown();
            },
            answer(card, result) {
                card.busy = true;
                this.client.respond(card.id, result);
                this.requests = this.requests.filter((one) => one !== card);
            },
            decide(card, decision) { this.answer(card, { decision }); },
            grantPermissions(card, scope) { this.answer(card, { permissions: card.params.permissions || {}, scope }); },
            denyPermissions(card) { this.answer(card, { permissions: {}, scope: "turn" }); },
            answerQuestions(card) {
                const answers = {};
                Object.keys(card.answers).forEach((id) => { answers[id] = { answers: card.answers[id] === "" ? [] : [String(card.answers[id])] }; });
                this.answer(card, { answers });
            },
            elicit(card, action) {
                const result = { action };
                if (action === "accept" && card.fields) result.content = codex.formContent(card.fields);
                this.answer(card, result);
            },
            requestTitle(card) {
                const params = card.params || {};
                switch (card.method) {
                case "item/commandExecution/requestApproval": case "execCommandApproval": return "Run a command?";
                case "item/fileChange/requestApproval": case "applyPatchApproval": return "Change files?";
                case "item/permissions/requestApproval": return "Allow more permissions?";
                case "item/tool/requestUserInput": return "Codex asks";
                case "mcpServer/elicitation/request": return params.serverName === "moqui" ? "Moqui asks for your approval" : (params.serverName || "A tool") + " asks";
                default: return card.method;
                }
            },
            commandOf(card) {
                const params = card.params || {};
                return Array.isArray(params.command) ? params.command.join(" ") : (params.command || "");
            },
            changesOf(card) {
                const item = this.items.get((card.params || {}).itemId);
                return item && item.changes ? item.changes : [];
            },

            // ---- the composer ----
            addFiles(event) {
                const list = Array.from((event.target && event.target.files) || []);
                list.forEach((file) => this.files.push(file));
                if (event.target) event.target.value = "";
            },
            removeFile(index) { this.files.splice(index, 1); },
            // the user's files go to /workspace/inbox; an image is given to the model as one, any other as a mention
            uploadFiles() {
                if (!this.files.length) return Promise.resolve([]);
                const files = this.files.slice();
                return this.call("fs/createDirectory", { path: INBOX, recursive: true })
                    .then(() => Promise.all(files.map((file) => readBase64(file).then((data) => {
                        const path = INBOX + "/" + codex.inboxName(file.name);
                        return this.call("fs/writeFile", { path, dataBase64: data }).then(() => (/^image\//.test(file.type)
                            ? { type: "localImage", path } : { type: "mention", name: codex.inboxName(file.name), path }));
                    }))));
            },
            onKey(event) {
                if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); this.send(); }
            },
            send() {
                if (!this.canSend) return;
                const text = this.draft.trim();
                this.sending = true;
                const threadReady = this.thread.id ? Promise.resolve(this.thread.id) : this.startThread();
                threadReady.then((threadId) => this.uploadFiles().then((attached) => {
                    const input = [];
                    if (text) input.push({ type: "text", text, text_elements: [] });
                    attached.forEach((part) => input.push(part));
                    const clientUserMessageId = "m" + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
                    // typed while a turn runs: steer it, as the extension does; else a new turn
                    const sent = this.running
                        ? this.call("turn/steer", { threadId, expectedTurnId: this.thread.activeTurnId, input, clientUserMessageId })
                        : this.call("turn/start", { threadId, input, model: this.model || null, effort: this.effort || null,
                            approvalPolicy: this.approval || null, clientUserMessageId })
                            .then((result) => { if (result && result.turn) this.turnBegun(result.turn.id); });
                    return sent.then(() => { this.draft = ""; this.files = []; });
                })).catch(() => {}).finally(() => { this.sending = false; this.scrollDown(); });
            },
            // ---- the menu's thread commands, as the extension's ----
            forkThread() {
                if (!this.thread.id || this.running) return;
                this.call("thread/fork", { threadId: this.thread.id }).then((result) => {
                    if (result && result.thread) return this.openThread(result.thread.id);
                    return null;
                }).catch(() => {});
            },
            compactThread() {
                if (!this.thread.id || this.running) return;
                this.call("thread/compact/start", { threadId: this.thread.id }).catch(() => {});
            },
            // a review of the workspace's uncommitted changes, or as the draft says (the draft is used and cleared)
            startReview(custom) {
                if (this.running) return;
                const instructions = this.draft.trim();
                if (custom && !instructions) return;
                const target = custom ? { type: "custom", instructions } : { type: "uncommittedChanges" };
                const threadReady = this.thread.id ? Promise.resolve(this.thread.id) : this.startThread();
                threadReady.then((threadId) => this.call("review/start", { threadId, target, delivery: "inline" }))
                    .then((result) => {
                        if (custom) this.draft = "";
                        if (result && result.turn) this.turnBegun(result.turn.id);
                    }).catch(() => {}).finally(() => this.scrollDown());
            },
            // ---- the workspace's files, to the user's own computer only ----
            // As the extension does, from the sandbox through the protocol (fs/readDirectory, fs/readFile): a file goes from
            // the sandbox to the browser, which saves it; nothing is kept on Moqui's server.
            openFiles() {
                this.browse.open = true;
                this.listFiles(this.browse.path || WORKSPACE + "/out");
            },
            listFiles(path) {
                const target = codex.workspacePath(path);
                this.browse.loading = true;
                this.browse.error = "";
                this.client.request("fs/readDirectory", { path: target }).then((result) => {
                    this.browse.path = target;
                    this.browse.entries = ((result && result.entries) || []).filter((entry) => !entry.fileName.startsWith("."))
                        .sort((a, b) => (b.isDirectory - a.isDirectory) || a.fileName.localeCompare(b.fileName));
                }, (err) => {
                    // an out folder Codex has not made yet: the workspace itself
                    if (target !== WORKSPACE) { this.browse.path = WORKSPACE; this.listFiles(WORKSPACE); return; }
                    this.browse.error = (err && err.message) || "The workspace could not be read";
                }).catch((err) => { this.browse.error = String((err && err.message) || err); })
                    .finally(() => { this.browse.loading = false; });
            },
            openEntry(entry) {
                const path = this.browse.path + "/" + entry.fileName;
                if (entry.isDirectory) this.listFiles(path); else this.download(path, entry.fileName);
            },
            filesUp() { this.listFiles(codex.workspacePath(this.browse.path.replace(/\/[^/]*$/, ""))); },
            download(path, name) {
                this.browse.busy = path;
                this.client.request("fs/readFile", { path }).then((result) => {
                    const binary = root.atob((result && result.dataBase64) || "");
                    const bytes = new Uint8Array(binary.length);
                    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
                    const url = root.URL.createObjectURL(new root.Blob([bytes]));
                    const link = root.document.createElement("a");
                    link.href = url;
                    link.download = name;
                    root.document.body.appendChild(link);
                    link.click();
                    link.remove();
                    setTimeout(() => root.URL.revokeObjectURL(url), 1000);
                }, (err) => { this.browse.error = (err && err.message) || "The file could not be read"; })
                    .finally(() => { this.browse.busy = null; });
            },
            stop() {
                if (!this.running) return;
                this.call("turn/interrupt", { threadId: this.thread.id, turnId: this.thread.activeTurnId }).catch(() => {});
            },
            scrollDown() {
                this.$nextTick(() => {
                    const el = this.$refs.scroller;
                    if (el) el.scrollTop = el.scrollHeight;
                });
            },
            when(seconds) { return seconds ? new Date(seconds * 1000).toLocaleString() : ""; },
            json(value) { try { return JSON.stringify(value, null, 2); } catch (err) { return String(value); } },
            mcpText(result) { return codex.mcpResultText(result); },
            userText(item) { return codex.userText(item.content); },
            decisionLabel(decision) { return codex.decisionLabel(decision); },
            decisionsOf(card) { return codex.decisionsOf(card); },
            // a tool call's name; through a lazy catalog's run_tool, the tool it runs
            toolLabel(item) {
                const args = item.arguments || {};
                return item.tool === "run_tool" && args.name ? "run_tool: " + args.name : item.tool;
            },
            elicitationHref(card) { return codex.elicitationHref(card.params.url, root.location.origin); },
            stepIcon(status) { return status === "completed" ? "sym_o_check_circle" : status === "inProgress" ? "sym_o_progress_activity" : "sym_o_radio_button_unchecked"; },
            statusIcon(item) {
                const status = item.status;
                if (status === "completed") return item.type === "commandExecution" && item.exitCode ? "sym_o_error" : "sym_o_check";
                if (status === "failed" || status === "declined") return "sym_o_block";
                return "sym_o_progress_activity";
            }
        },
        template:
            '<div class="m-assist-codex column no-wrap fit" data-assist-codex :data-status="status">' +
                '<div class="row items-center no-wrap q-px-sm q-py-xs m-assist-codex-bar">' +
                    '<div class="text-caption ellipsis col" :title="thread.name || thread.preview">{{thread.name || thread.preview || (thread.id ? "Thread" : "New thread")}}</div>' +
                    '<q-select v-if="modelOptions.length" v-model="model" :options="modelOptions" emit-value map-options dense borderless options-dense class="m-assist-codex-model" aria-label="Model"></q-select>' +
                    '<q-select v-if="effortOptions.length" v-model="effort" :options="effortOptions" emit-value map-options dense borderless options-dense class="q-ml-xs" aria-label="Reasoning effort"></q-select>' +
                    '<span v-if="usageText" class="text-caption m-muted q-ml-xs" data-codex-usage>{{usageText}}</span>' +
                    '<q-select v-model="approval" :options="approvalOptions" emit-value map-options dense borderless options-dense class="q-ml-xs" aria-label="Approvals" data-codex-approval><q-tooltip>When Codex asks before it acts</q-tooltip></q-select>' +
                    '<q-btn flat round dense icon="sym_o_folder_open" aria-label="Files" data-codex-action="files" @click="openFiles"><q-tooltip>The workspace\'s files, to download</q-tooltip></q-btn>' +
                    '<q-btn flat round dense icon="sym_o_edit_square" aria-label="New thread" data-codex-action="new" @click="newThread"><q-tooltip>New thread</q-tooltip></q-btn>' +
                    '<q-btn flat round dense icon="sym_o_history" :class="{\'text-primary\': pane === \'history\'}" aria-label="Threads" data-codex-action="history" @click="showHistory"><q-tooltip>Threads</q-tooltip></q-btn>' +
                    '<q-btn v-if="signedIn" flat round dense icon="sym_o_more_vert" aria-label="Workbench menu"><q-menu anchor="bottom right" self="top right"><q-list dense style="min-width:220px;">' +
                        '<q-item clickable v-close-popup :disable="running" data-codex-action="review" @click="startReview(false)"><q-item-section>Review uncommitted changes</q-item-section></q-item>' +
                        '<q-item clickable v-close-popup :disable="running || !draft.trim()" data-codex-action="review-custom" @click="startReview(true)"><q-item-section>Review as the message says</q-item-section></q-item>' +
                        '<q-item clickable v-close-popup :disable="running || !thread.id" data-codex-action="fork" @click="forkThread"><q-item-section>Fork this thread</q-item-section></q-item>' +
                        '<q-item clickable v-close-popup :disable="running || !thread.id" data-codex-action="compact" @click="compactThread"><q-item-section>Compact the context</q-item-section></q-item>' +
                        '<q-separator></q-separator>' +
                        '<q-item clickable v-close-popup data-codex-action="logout" @click="logout"><q-item-section>Sign out of Codex</q-item-section></q-item>' +
                    '</q-list></q-menu></q-btn>' +
                '</div>' +
                '<q-separator></q-separator>' +
                // not connected
                '<div v-if="status !== \'open\'" class="q-pa-md column items-start" data-codex-connection>' +
                    '<div v-if="status === \'connecting\'" class="row items-center m-muted"><q-spinner-dots size="18px" class="q-mr-sm"></q-spinner-dots>Starting your Workbench…</div>' +
                    '<template v-else><div class="text-negative q-mb-sm">{{error || "The Workbench is not connected."}}</div>' +
                        '<q-btn unelevated dense no-caps color="primary" label="Connect again" data-codex-action="reconnect" @click="reconnect"></q-btn></template>' +
                '</div>' +
                // the sign in card
                '<div v-else-if="account === null" class="q-pa-md" data-codex-signin>' +
                    '<div class="text-subtitle2 q-mb-xs">Sign in to Codex</div>' +
                    '<div class="text-caption m-muted q-mb-sm">The Workbench runs Codex in your own sandbox. Sign in with your ChatGPT account, or use an OpenAI API key; the sign-in stays in your sandbox.</div>' +
                    '<div v-if="login" class="q-mb-md" data-codex-device-code>' +
                        '<div>Open <a :href="login.verificationUrl" target="_blank" rel="noopener noreferrer">{{login.verificationUrl}}</a> and enter the code</div>' +
                        '<div class="text-h6 q-my-sm" style="letter-spacing:2px;" data-codex-user-code>{{login.userCode}}</div>' +
                        '<div class="row items-center m-muted text-caption"><q-spinner-dots size="14px" class="q-mr-xs"></q-spinner-dots>Waiting for the sign in…' +
                            '<q-btn flat dense no-caps size="sm" label="Cancel" class="q-ml-sm" @click="cancelLogin"></q-btn></div>' +
                    '</div>' +
                    '<q-btn v-else unelevated dense no-caps color="primary" label="Sign in with ChatGPT" :loading="loginBusy" data-codex-action="login-chatgpt" @click="loginChatgpt"></q-btn>' +
                    '<div class="row items-center no-wrap q-mt-md"><q-input v-model="apiKey" type="password" dense outlined class="col" label="OpenAI API key" autocomplete="off" @keydown.enter="loginApiKey"></q-input>' +
                        '<q-btn flat dense no-caps color="primary" label="Use key" class="q-ml-xs" :disable="!apiKey.trim()" :loading="loginBusy" @click="loginApiKey"></q-btn></div>' +
                '</div>' +
                '<div v-else-if="account === undefined" class="q-pa-md m-muted row items-center"><q-spinner-dots size="18px" class="q-mr-sm"></q-spinner-dots></div>' +
                // the threads
                '<div v-else-if="pane === \'history\'" class="col scroll q-pa-sm" data-codex-threads>' +
                    '<div v-if="threadsLoading && !threads.length" class="m-muted row items-center"><q-spinner-dots size="18px"></q-spinner-dots></div>' +
                    '<div v-else-if="!threads.length" class="m-muted text-caption">No threads yet.</div>' +
                    '<q-list dense separator>' +
                        '<q-item v-for="one in threads" :key="one.id" clickable :active="one.id === thread.id" :data-codex-thread="one.id" @click="openThread(one.id)">' +
                            '<q-item-section><q-item-label lines="1">{{one.name || one.preview || one.id}}</q-item-label>' +
                                '<q-item-label caption>{{when(one.updatedAt)}}</q-item-label></q-item-section>' +
                        '</q-item>' +
                    '</q-list>' +
                    '<q-btn v-if="threadsCursor" flat dense no-caps color="primary" label="More" class="q-mt-sm" :loading="threadsLoading" @click="loadThreads(true)"></q-btn>' +
                '</div>' +
                // the thread
                '<template v-else>' +
                    '<div ref="scroller" class="col scroll q-px-sm q-py-xs" data-codex-thread-view>' +
                        '<div v-if="!thread.turns.length" class="m-muted text-caption q-pa-sm">Ask Codex to research, read files, write and run code in your Workbench (/workspace). Files you add go to /workspace/inbox.</div>' +
                        '<div v-for="turn in thread.turns" :key="turn.id" class="q-mb-sm" :data-codex-turn="turn.id" :data-turn-status="turn.status">' +
                            '<template v-for="item in turn.items" :key="item.id">' +
                                '<div v-if="item.type === \'userMessage\'" class="m-assist-bubble m-assist-user q-my-xs" data-codex-item="userMessage"><span class="m-assist-pre">{{userText(item)}}</span></div>' +
                                '<div v-else-if="item.type === \'agentMessage\'" class="q-my-xs" data-codex-item="agentMessage">' +
                                    '<m-assist-markdown :text="item.text || \'\'" :streaming="item.status !== \'completed\' && turn.status === \'inProgress\'"></m-assist-markdown></div>' +
                                '<div v-else-if="item.type === \'reasoning\'" class="q-my-xs" data-codex-item="reasoning">' +
                                    '<div v-if="item.summaryText" class="text-caption m-muted cursor-pointer" @click="item.open = !item.open"><q-icon name="sym_o_psychology" size="xs" class="q-mr-xs"></q-icon>Thinking' +
                                        '<div v-if="item.open" class="m-assist-pre q-mt-xs">{{item.summaryText}}</div></div></div>' +
                                '<div v-else-if="item.type === \'plan\'" class="q-my-xs text-caption m-assist-pre" data-codex-item="plan">{{item.text}}</div>' +
                                '<div v-else-if="item.type === \'commandExecution\'" class="q-my-xs m-assist-codex-step" data-codex-item="commandExecution" :data-status="item.status">' +
                                    '<div class="row items-center no-wrap cursor-pointer text-caption" @click="item.open = !item.open"><q-icon :name="statusIcon(item)" size="xs" class="q-mr-xs"></q-icon>' +
                                        '<code class="ellipsis col">{{item.command}}</code><span v-if="item.exitCode !== null && item.exitCode !== undefined" class="m-muted q-ml-xs">exit {{item.exitCode}}</span></div>' +
                                    '<pre v-if="item.open" class="m-assist-codex-output">{{item.aggregatedOutput || ""}}</pre></div>' +
                                '<div v-else-if="item.type === \'fileChange\'" class="q-my-xs m-assist-codex-step" data-codex-item="fileChange" :data-status="item.status">' +
                                    '<div class="row items-center no-wrap cursor-pointer text-caption" @click="item.open = !item.open"><q-icon :name="statusIcon(item)" size="xs" class="q-mr-xs"></q-icon>' +
                                        '<span class="ellipsis col">{{(item.changes || []).map((c) => c.path).join(", ") || "Files"}}</span></div>' +
                                    '<template v-if="item.open"><div v-for="change in item.changes || []" :key="change.path"><div class="text-caption text-weight-medium">{{change.kind && change.kind.type}} {{change.path}}</div>' +
                                        '<pre class="m-assist-codex-output">{{change.diff}}</pre></div></template></div>' +
                                '<div v-else-if="item.type === \'mcpToolCall\' || item.type === \'dynamicToolCall\'" class="q-my-xs m-assist-codex-step" :data-codex-item="item.type" :data-status="item.status">' +
                                    '<div class="row items-center no-wrap cursor-pointer text-caption" @click="item.open = !item.open"><q-icon :name="statusIcon(item)" size="xs" class="q-mr-xs"></q-icon>' +
                                        '<span class="ellipsis col">{{item.server ? item.server + " · " : ""}}{{toolLabel(item)}}</span></div>' +
                                    '<template v-if="item.open"><pre class="m-assist-codex-output">{{json(item.arguments)}}</pre>' +
                                        '<pre v-if="item.result" class="m-assist-codex-output">{{mcpText(item.result)}}</pre>' +
                                        '<div v-if="item.error" class="text-negative text-caption">{{item.error.message || json(item.error)}}</div></template></div>' +
                                '<div v-else-if="item.type === \'webSearch\'" class="q-my-xs text-caption m-muted" data-codex-item="webSearch"><q-icon name="sym_o_travel_explore" size="xs" class="q-mr-xs"></q-icon>{{item.query}}</div>' +
                                '<div v-else-if="item.type === \'imageView\'" class="q-my-xs text-caption m-muted" data-codex-item="imageView"><q-icon name="sym_o_image" size="xs" class="q-mr-xs"></q-icon>{{item.path}}</div>' +
                                '<div v-else-if="item.type === \'contextCompaction\'" class="q-my-xs text-caption m-muted" data-codex-item="contextCompaction">Context compacted</div>' +
                                '<div v-else-if="item.type === \'enteredReviewMode\'" class="q-my-xs text-caption m-muted" data-codex-item="enteredReviewMode"><q-icon name="sym_o_rate_review" size="xs" class="q-mr-xs"></q-icon>Review: {{item.review}}</div>' +
                                '<div v-else-if="item.type === \'exitedReviewMode\'" class="q-my-xs" data-codex-item="exitedReviewMode"><m-assist-markdown :text="item.review || \'\'"></m-assist-markdown></div>' +
                            '</template>' +
                            '<div v-if="turn.error" class="text-caption text-negative q-my-xs" data-codex-turn-error>{{turn.error.message || json(turn.error)}}</div>' +
                            '<div v-if="turn.status === \'interrupted\'" class="text-caption m-muted q-my-xs">Stopped</div>' +
                        '</div>' +
                        '<div v-if="thread.plan && thread.plan.steps.length && running" class="q-my-xs text-caption" data-codex-plan>' +
                            '<div v-if="thread.plan.explanation" class="m-muted">{{thread.plan.explanation}}</div>' +
                            '<div v-for="(step, index) in thread.plan.steps" :key="index" class="row items-center no-wrap"><q-icon :name="stepIcon(step.status)" size="xs" class="q-mr-xs"></q-icon>{{step.step}}</div>' +
                        '</div>' +
                        '<div v-if="running" class="row items-center text-caption m-muted q-my-xs"><q-spinner-dots size="14px" class="q-mr-xs"></q-spinner-dots>Working…</div>' +
                    '</div>' +
                    // what Codex asks
                    '<div v-if="requests.length" class="q-px-sm q-pt-xs m-assist-cards" data-codex-requests>' +
                        '<q-card v-for="card in requests" :key="card.id" flat bordered class="q-mb-xs q-pa-sm" :data-codex-request="card.method">' +
                            '<div class="text-subtitle2">{{requestTitle(card)}}</div>' +
                            '<div v-if="card.params.reason" class="text-caption q-mb-xs">{{card.params.reason}}</div>' +
                            '<template v-if="decisionsOf(card).length">' +
                                '<pre v-if="commandOf(card)" class="m-assist-codex-output">{{commandOf(card)}}</pre>' +
                                '<div v-if="card.params.cwd" class="text-caption m-muted">in {{card.params.cwd}}</div>' +
                                '<div v-for="change in changesOf(card)" :key="change.path"><div class="text-caption text-weight-medium">{{change.path}}</div><pre class="m-assist-codex-output">{{change.diff}}</pre></div>' +
                                '<div class="row q-gutter-xs q-mt-xs"><q-btn v-for="(decision, index) in decisionsOf(card)" :key="index" dense no-caps :unelevated="index === 0" :flat="index > 0"' +
                                    ' :color="index === 0 ? \'primary\' : undefined" :label="decisionLabel(decision)" :disable="card.busy" :data-codex-decision="typeof decision === \'string\' ? decision : Object.keys(decision)[0]" @click="decide(card, decision)"></q-btn></div>' +
                            '</template>' +
                            '<template v-else-if="card.method === \'item/permissions/requestApproval\'">' +
                                '<pre class="m-assist-codex-output">{{json(card.params.permissions)}}</pre>' +
                                '<div class="row q-gutter-xs q-mt-xs"><q-btn unelevated dense no-caps color="primary" label="Allow for this turn" @click="grantPermissions(card, \'turn\')"></q-btn>' +
                                    '<q-btn flat dense no-caps label="Allow for this session" @click="grantPermissions(card, \'session\')"></q-btn>' +
                                    '<q-btn flat dense no-caps label="Decline" @click="denyPermissions(card)"></q-btn></div>' +
                            '</template>' +
                            '<template v-else-if="card.method === \'item/tool/requestUserInput\'">' +
                                '<div v-for="question in card.params.questions" :key="question.id" class="q-mb-sm">' +
                                    '<div class="text-caption text-weight-medium">{{question.header}}</div><div class="q-mb-xs">{{question.question}}</div>' +
                                    '<q-option-group v-if="question.options && question.options.length" v-model="card.answers[question.id]" dense' +
                                        ' :options="question.options.map((option) => ({label: option.label + (option.description ? \' — \' + option.description : \'\'), value: option.label}))"></q-option-group>' +
                                    '<q-input v-if="!question.options || !question.options.length || question.isOther" v-model="card.answers[question.id]" dense outlined :type="question.isSecret ? \'password\' : \'text\'"></q-input>' +
                                '</div>' +
                                '<q-btn unelevated dense no-caps color="primary" label="Answer" @click="answerQuestions(card)"></q-btn>' +
                            '</template>' +
                            '<template v-else-if="card.method === \'mcpServer/elicitation/request\'">' +
                                '<div class="q-mb-xs m-assist-pre">{{card.params.message}}</div>' +
                                '<template v-if="card.params.mode === \'url\'">' +
                                    '<a v-if="elicitationHref(card)" :href="elicitationHref(card)" target="_blank" rel="noopener" data-codex-elicitation-url>Open the approval</a>' +
                                    '<div class="text-caption m-muted q-my-xs">Open the link, decide there, then go on.</div>' +
                                    '<div class="row q-gutter-xs"><q-btn unelevated dense no-caps color="primary" label="Done, go on" data-codex-decision="accept" @click="elicit(card, \'accept\')"></q-btn>' +
                                        '<q-btn flat dense no-caps label="Decline" data-codex-decision="decline" @click="elicit(card, \'decline\')"></q-btn>' +
                                        '<q-btn flat dense no-caps label="Cancel" @click="elicit(card, \'cancel\')"></q-btn></div>' +
                                '</template>' +
                                '<template v-else-if="card.fields">' +
                                    '<div v-for="field in card.fields" :key="field.name" class="q-mb-xs">' +
                                        '<q-select v-if="field.options" v-model="field.value" :options="field.options" emit-value map-options dense outlined :label="field.title" :hint="field.description"></q-select>' +
                                        '<q-toggle v-else-if="field.type === \'boolean\'" v-model="field.value" :label="field.title"></q-toggle>' +
                                        '<q-input v-else v-model="field.value" dense outlined :label="field.title" :hint="field.description" :type="field.type === \'number\' || field.type === \'integer\' ? \'number\' : \'text\'"></q-input>' +
                                    '</div>' +
                                    '<div class="row q-gutter-xs"><q-btn unelevated dense no-caps color="primary" label="Submit" @click="elicit(card, \'accept\')"></q-btn>' +
                                        '<q-btn flat dense no-caps label="Decline" @click="elicit(card, \'decline\')"></q-btn></div>' +
                                '</template>' +
                                '<div v-else class="row q-gutter-xs"><q-btn flat dense no-caps label="Decline" @click="elicit(card, \'decline\')"></q-btn></div>' +
                            '</template>' +
                        '</q-card>' +
                    '</div>' +
                    // the composer
                    '<div class="q-pa-sm m-assist-codex-composer">' +
                        '<div v-if="notice" class="text-caption text-negative q-mb-xs cursor-pointer" @click="notice = \'\'">{{notice}}</div>' +
                        '<div v-if="files.length" class="row q-gutter-xs q-mb-xs"><q-chip v-for="(file, index) in files" :key="index" dense removable size="sm" @remove="removeFile(index)">{{file.name}}</q-chip></div>' +
                        '<q-input v-model="draft" type="textarea" autogrow dense outlined :placeholder="running ? \'Steer Codex…\' : \'Ask Codex…\'" input-style="max-height:200px;" data-codex-composer @keydown="onKey"></q-input>' +
                        '<div class="row items-center no-wrap q-mt-xs">' +
                            '<q-btn flat round dense icon="sym_o_attach_file" aria-label="Add files" @click="$refs.files.click()"><q-tooltip>Add files to /workspace/inbox</q-tooltip></q-btn>' +
                            '<input ref="files" type="file" multiple style="display:none" @change="addFiles">' +
                            '<q-space></q-space>' +
                            '<q-btn v-if="running" flat dense no-caps icon="sym_o_stop_circle" label="Stop" data-codex-action="stop" @click="stop"></q-btn>' +
                            '<q-btn unelevated dense no-caps color="primary" icon="sym_o_send" :label="running ? \'Steer\' : \'Send\'" :disable="!canSend" :loading="sending" data-codex-action="send" @click="send"></q-btn>' +
                        '</div>' +
                    '</div>' +
                '</template>' +
                '<q-dialog v-model="browse.open"><q-card style="min-width:min(560px,92vw);max-width:92vw;" data-codex-files>' +
                    '<q-card-section class="row items-center no-wrap q-pb-none">' +
                        '<div class="text-subtitle2 ellipsis col">{{browse.path || "/workspace"}}</div>' +
                        '<q-btn flat round dense icon="sym_o_arrow_upward" aria-label="Up" :disable="browse.path === \'/workspace\' || browse.loading" data-codex-action="files-up" @click="filesUp"></q-btn>' +
                        '<q-btn flat round dense icon="sym_o_refresh" aria-label="Refresh" :disable="browse.loading" @click="listFiles(browse.path)"></q-btn>' +
                        '<q-btn flat round dense icon="sym_o_close" aria-label="Close" v-close-popup></q-btn></q-card-section>' +
                    '<q-card-section class="q-pt-xs">' +
                        '<div class="text-caption m-muted q-mb-xs">The sandbox\'s files: a download goes to this computer, nothing is kept in Moqui.</div>' +
                        '<div v-if="browse.error" class="text-caption text-negative q-mb-xs">{{browse.error}}</div>' +
                        '<div v-if="browse.loading" class="row items-center m-muted"><q-spinner-dots size="18px"></q-spinner-dots></div>' +
                        '<div v-else-if="!browse.entries.length" class="text-caption m-muted">No files here.</div>' +
                        '<q-list v-else dense separator style="max-height:60vh;overflow:auto;">' +
                            '<q-item v-for="entry in browse.entries" :key="entry.fileName" clickable :data-codex-file="entry.fileName" @click="openEntry(entry)">' +
                                '<q-item-section avatar><q-icon :name="entry.isDirectory ? \'sym_o_folder\' : \'sym_o_description\'" size="sm"></q-icon></q-item-section>' +
                                '<q-item-section><q-item-label lines="1">{{entry.fileName}}</q-item-label></q-item-section>' +
                                '<q-item-section side><q-spinner-dots v-if="browse.busy === browse.path + \'/\' + entry.fileName" size="16px"></q-spinner-dots>' +
                                    '<q-icon v-else-if="!entry.isDirectory" name="sym_o_download" size="xs"></q-icon></q-item-section>' +
                            '</q-item>' +
                        '</q-list>' +
                    '</q-card-section></q-card></q-dialog>' +
            '</div>'
    };

    if (typeof module !== "undefined" && module.exports) module.exports = codex;
})(globalThis);
