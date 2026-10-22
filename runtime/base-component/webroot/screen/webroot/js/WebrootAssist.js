/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

/*
 * Assist in the /qapps shell (plans/LLM-A2A-Assist.md 4.11; the server's contract is docs/Assist.md): moqui.assist, the
 * drawer of the agent that works the user's own screens. It holds
 *   - the store: a pure reducer of a conversation (assist.snapshot from GET conversation, assist.apply for each run event:
 *     text deltas and chunks by (messageId, part, offset), sequence gaps that ask for a catch-up, idempotence, the card
 *     lifecycles of approvals, questions and hand-offs, messages queued while a run works), and assist.threadRows, what
 *     the thread draws;
 *   - the transport: the run's event stream (SSE over fetch with Last-Event-ID, opened only while a run is active and the
 *     tab visible), the catch-up through events.json, polling when the stream cannot be had, the AgentRun signals of the
 *     notification socket;
 *   - follow: the agent's ui.* events applied by the follow owner (follow.applies, follow.locate over merged qvt names with
 *     the section entry index, static repeated names, dialogs, list rows and headers), the page state each turn carries
 *     (follow.unsaved: never a secret, hidden, password or file field; capped), contextVersion raised by the user's own
 *     navigation, and the hand-off (the filled form with Save marked, the user's save reported through moqui.onFormAnswer);
 *   - safePath, the one path check of the client (the server is the authority), the / commands and @ mentions, record chips
 *     and the Markdown of the thread with remote images as placeholders;
 *   - the components: m-assist-button (the header), m-shell-assist (one right q-drawer, nothing reparented), m-assist-thread,
 *     m-assist-markdown, m-assist-card, m-assist-composer, m-assist-history and m-assist-tasks.
 *
 * Loaded after WebrootChat.js and before WebrootVue.qvt.js (qapps.xml): its components are in moqui.qvtComponents, which
 * WebrootVue.qvt.js registers (and precompiles into WebrootVue.qvt.render.js); loaded later, they go through
 * qvt.registerLate. The shell root calls moqui.assist.start(root) once mounted. Runs under Node for its tests
 * (WebrootAssist.test.js): nothing here touches the page until start.
 */
(function (root) {
    "use strict";

    const moqui = root.moqui;
    const reactive = (obj) => (root.Vue && root.Vue.reactive ? root.Vue.reactive(obj) : obj);

    // the transition parameters a page state never carries, as the screen context leaves them out
    const META_FIELDS = ["moquiFormName", "moquiSessionToken", "_isMulti", "pageIndex", "orderByField", "lastStandalone", "formListFindId"];
    // the widgets that never follow and are never sent
    const CLOSED_TYPES = ["hidden", "password", "file"];
    const PAUSED = ["TASK_STATE_INPUT_REQUIRED", "TASK_STATE_AUTH_REQUIRED"];
    const ENDED = ["TASK_STATE_COMPLETED", "TASK_STATE_FAILED", "TASK_STATE_CANCELED", "TASK_STATE_REJECTED"];
    const DEFAULT_LIMITS = { maxMessageChars: 16000, maxForms: 20, maxFormValues: 100, maxValueChars: 4000 };
    const DEFAULT_SECRETS = { contains: ["password", "passwd", "token", "secret", "credential", "salt"], endsWith: ["key"], names: [] };
    const CHIP_RE = /\[\[record:([^:\]|\n]+):([^\]|\n]+)(?:\|([^\]\n]*))?\]\]/g;
    const TAG_FOR = 8000;
    const MAX_CHIPS = 5;

    const assist = {
        labels: { assist: "Assist", askAssist: "Ask Assist", newConversation: "New conversation", history: "History", tasks: "Tasks",
            send: "Send", stop: "Stop", close: "Close", placeholder: "Ask Assist — / for commands, @ for records", approve: "Approve",
            editInScreen: "Edit in screen", reject: "Reject", rejectWith: "Reject with feedback", feedback: "What should Assist do instead?",
            preview: "Preview effects", previewNote: "a slower simulation", risk: "Risk", riskFrom: "says skill", previewHidden: "(hidden)", arguments: "Arguments",
            editArguments: "Edit arguments", approveEdited: "Approve as edited", queued: "Queued", sending: "Sending", notSent: "Not sent",
            retry: "Retry", followHere: "Follow here", followingElsewhere: "Following in another tab", followOn: "Follow on this page",
            skip: "Skip", handoff: "Assist filled the form. Review it, then Save.", handoffSaved: "Saved", handoffFailed: "The save was refused; fix it and Save again",
            working: "Working", waitingApproval: "Waiting for your approval", waitingAnswer: "Waiting for your answer", waitingHandoff: "Waiting for your save",
            thought: "Thought", step: "step", steps: "steps", tokens: "tokens", seconds: "s", cost: "cost", conflict: "changed since this was proposed",
            reviewAgain: "Review again", show: "Show", opened: "Assist opened", filled: "Assist filled", noConversations: "No conversations",
            searchConversations: "Search conversations", waiting: "Waiting for you", running: "Running", failed: "Failed", recentlyDone: "Recently done",
            open: "Open", answer: "Answer", continueRun: "Continue", stopRun: "Stop", copy: "Copy", image: "Image", unavailable: "Assist is not available",
            empty: "Ask about this page, or tell Assist what to do.", earlier: "Earlier messages", recommended: "recommended", other: "Other",
            commands: "Commands", help: "/new starts a conversation, /tier fast|balanced|thorough picks the tier, /skill <name> asks for a skill, /help shows this.",
            adminHelp: "Admin: /prompt, /tools, /skills, /context, /usage.", tier: "Tier", agent: "Agent", menu: "More", expired: "Expired",
            approved: "Approved", rejected: "Rejected", executed: "Done", executing: "Running", handedOff: "Handed to you", decided: "Decided",
            errorTitle: "Something went wrong", nothingWaiting: "Nothing waiting", up: "Helpful", down: "Not helpful",
            canvas: "Canvas", backToPage: "Back to the page", showCanvas: "Show canvas", closeCanvas: "Close the canvas", loading: "Loading",
            adjust: "Ask Assist to change this canvas", adjustSend: "Adjust", canvasProblems: "The canvas reported", canvasUpdating: "Updating the canvas",
            canvasFailed: "The canvas could not be loaded" },

        // ---- ids and names (WebrootAssist.test.js) ----

        // a random id of letters and digits with a prefix: the tab's clientId, a message's clientMessageId
        newId(prefix) {
            const random = Math.random().toString(36).slice(2, 10) + Math.random().toString(36).slice(2, 6);
            return (prefix || "") + Date.now().toString(36) + random;
        },
        // the tab's id, made once and kept in sessionStorage (each tab its own; a reload keeps it)
        clientIdOf(storage) {
            let id = null;
            try { id = storage ? storage.getItem("moqui.assist.clientId") : null; } catch (err) { id = null; }
            if (id && /^[A-Za-z0-9_.:-]{1,100}$/.test(id)) return id;
            id = assist.newId("tab-");
            try { if (storage) storage.setItem("moqui.assist.clientId", id); } catch (err) { /* without storage the id lasts as long as the page */ }
            return id;
        },
        // Whether a field's value never leaves the browser: its name in lower case contains a secret word, ends with one of the
        // endings, or is one of the names (bootstrap.secretFields; the server drops them too).
        isSecret(name, secretFields) {
            const secrets = secretFields || DEFAULT_SECRETS;
            const lower = String(name || "").toLowerCase();
            if (!lower) return false;
            return (secrets.contains || []).some((word) => lower.indexOf(String(word).toLowerCase()) >= 0) ||
                (secrets.endsWith || []).some((end) => lower.endsWith(String(end).toLowerCase())) ||
                (secrets.names || []).some((other) => String(other).toLowerCase() === lower);
        },

        // ---- safePath: the one path check of the client ----

        // A path the agent asked the page to go to (ui.navigate) or a link of the thread: under /apps or /qapps, no scheme or host,
        // no .., //, backslash, whitespace or control character (also once decoded), and flat parameters with plain names and
        // string, number or boolean values. Answers {ok, path, url, parameters} with url the /qapps address (linkBase), or
        // {ok: false, error}. The server checks what the agent may open; this keeps the page from going anywhere else.
        safePath(path, parameters, options) {
            const opts = options || {};
            const linkBase = opts.linkBase || "/qapps";
            const fail = (error) => ({ ok: false, error });
            if (path === null || path === undefined || path === "") return fail("no path");
            let text = String(path);
            if (text.charAt(0) !== "/") return fail("not a path from the root");
            if (/[\s\\\u0000-\u001f\u007f]/.test(text)) return fail("whitespace, a backslash or a control character");
            if (text.indexOf("://") >= 0 || text.indexOf("//") >= 0) return fail("a scheme or a host");
            let query = "";
            const cut = text.search(/[?#]/);
            if (cut >= 0) {
                if (text.charAt(cut) === "#") return fail("an anchor");
                query = text.slice(cut + 1);
                text = text.slice(0, cut);
                if (query.indexOf("#") >= 0) return fail("an anchor");
            }
            let decoded;
            try { decoded = decodeURIComponent(text); } catch (err) { return fail("not a valid path"); }
            if (/[\s\\\u0000-\u001f\u007f]/.test(decoded) || decoded.indexOf("//") >= 0) return fail("whitespace, a backslash or a control character");
            if (decoded.split("/").some((part) => part === ".." || part === "." || part.indexOf("..") === 0)) return fail("a .. segment");
            if (/^\/[^/]*:/.test(decoded) || /(javascript|data|vbscript|mailto|file):/i.test(decoded)) return fail("a scheme");
            const head = decoded.split("/")[1] || "";
            if (head !== "apps" && head !== "qapps") return fail("not an application screen");
            const params = {};
            const addParam = (key, value) => {
                if (!/^[A-Za-z0-9_.-]{1,100}$/.test(key)) return "a parameter name that is not plain";
                if (value === null || value === undefined || value === "") return null;
                if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") return "a parameter that is not flat";
                const valueText = String(value);
                if (/[\u0000-\u001f\u007f]/.test(valueText)) return "a control character in a parameter";
                params[key] = valueText;
                return null;
            };
            if (query) {
                const pairs = query.split("&").filter((pair) => pair);
                for (let i = 0; i < pairs.length; i++) {
                    const eq = pairs[i].indexOf("=");
                    let key;
                    let value;
                    try {
                        key = decodeURIComponent((eq < 0 ? pairs[i] : pairs[i].slice(0, eq)).replace(/\+/g, " "));
                        value = eq < 0 ? "" : decodeURIComponent(pairs[i].slice(eq + 1).replace(/\+/g, " "));
                    } catch (err) { return fail("a query that does not decode"); }
                    const problem = addParam(key, value);
                    if (problem) return fail(problem);
                }
            }
            if (parameters !== null && parameters !== undefined) {
                if (typeof parameters !== "object" || Array.isArray(parameters)) return fail("parameters that are not an object");
                const keys = Object.keys(parameters);
                if (keys.length > 50) return fail("too many parameters");
                for (let i = 0; i < keys.length; i++) {
                    const problem = addParam(keys[i], parameters[keys[i]]);
                    if (problem) return fail(problem);
                }
            }
            const firstEnd = text.indexOf("/", 1);
            const rest = firstEnd < 0 ? "" : text.slice(firstEnd);
            const screenPath = "/apps" + rest;
            const search = Object.keys(params).map((key) => encodeURIComponent(key) + "=" + encodeURIComponent(params[key])).join("&");
            return { ok: true, path: screenPath, url: linkBase + rest + (search ? "?" + search : ""), parameters: params };
        },

        // ---- / commands and @ mentions ----

        commands: ["new", "skill", "tier", "help"],
        adminCommands: ["prompt", "tools", "skills", "context", "usage"],
        // A composer text that is a command: {name, arg}, a known name only (admin commands for an admin); else null, and the
        // text is sent as it is (a pasted /apps path is a message).
        parseCommand(text, options) {
            const match = /^\/([a-z]+)(?:\s+([\s\S]*))?$/.exec(String(text || "").trim());
            if (!match) return null;
            const name = match[1];
            const admin = !!(options && options.admin);
            if (assist.commands.indexOf(name) < 0 && !(admin && assist.adminCommands.indexOf(name) >= 0)) return null;
            const arg = (match[2] || "").trim();
            if (name === "tier") {
                const tier = arg.toLowerCase();
                return ["fast", "balanced", "thorough"].indexOf(tier) >= 0 ? { name, arg: tier } : { name, arg: "", error: "fast, balanced or thorough" };
            }
            if (name === "skill" && !arg) return { name, arg: "", error: "a skill name" };
            return { name, arg };
        },
        // The @ the caret is in: {start, end, query} when the text before the caret has an @ at its start or after a space with
        // no space after it, else null.
        mentionAt(text, caret) {
            const value = String(text || "");
            const end = caret === undefined || caret === null ? value.length : Math.max(0, Math.min(Number(caret), value.length));
            const before = value.slice(0, end);
            const at = before.lastIndexOf("@");
            if (at < 0) return null;
            if (at > 0 && !/\s/.test(before.charAt(at - 1))) return null;
            const query = before.slice(at + 1);
            if (/\s/.test(query) || query.length > 60) return null;
            return { start: at, end, query };
        },
        // The text with the mention replaced by the chosen row: a record (a palette row with recordEntity and recordId) as its
        // record chip, anything else as @ and its title. {text, caret} after the inserted text and a space.
        insertMention(text, mention, row) {
            const value = String(text || "");
            if (!mention || !row) return { text: value, caret: value.length };
            const label = String(row.title || row.label || row.recordId || "").replace(/[[\]|\n]/g, " ").replace(/\s+/g, " ").trim();
            const inserted = row.recordEntity && row.recordId ? "[[record:" + row.recordEntity + ":" + row.recordId + (label ? "|" + label : "") + "]]" :
                "@" + (row.agentId || row.name || label);
            const out = value.slice(0, mention.start) + inserted + " " + value.slice(mention.end);
            return { text: out, caret: mention.start + inserted.length + 1 };
        },

        // ---- record chips and the thread's Markdown ----

        // The text as segments: plain {text}, or {chip: {entityName, id, label, url}} for a record chip. A chip is drawn only
        // when the chip map (the server's, by "Entity:id") holds it, with the map's url and label; else it is its label as text.
        // own: the user's own message, whose chips are drawn without a link.
        segments(text, chipMap, own) {
            const value = String(text || "");
            const map = chipMap || {};
            const out = [];
            let last = 0;
            const push = (piece) => {
                if (!piece) return;
                const prev = out[out.length - 1];
                if (prev && prev.text !== undefined) prev.text += piece; else out.push({ text: piece });
            };
            value.replace(CHIP_RE, (whole, entityName, id, label, offset) => {
                push(value.slice(last, offset));
                last = offset + whole.length;
                const key = entityName + ":" + id;
                const known = map[key];
                if (known || own) out.push({ chip: { entityName, id, label: (known && known.label) || label || id, url: known && known.url ? known.url : null } });
                else push(label || id);
                return whole;
            });
            push(value.slice(last));
            return out;
        },
        // The chips of a text as Markdown: a known chip a link to its url (marked as a chip), else its label.
        chipsToMarkdown(text, chipMap) {
            return assist.segments(text, chipMap, false).map((segment) => {
                if (segment.text !== undefined) return segment.text;
                const label = segment.chip.label.replace(/[[\]]/g, "");
                return segment.chip.url ? "[" + label + "](" + segment.chip.url + ' "assist-chip")' : label;
            }).join("");
        },
        // The host of an absolute URL, "" for none.
        hostOf(url) {
            const match = /^(?:[a-z][a-z0-9+.-]*:)?\/\/([^/?#:]+)/i.exec(String(url || "").trim());
            return match ? match[1].toLowerCase() : "";
        },
        // Images of the HTML as placeholders with the host, never fetched; an image of this site's own path stays. srcset,
        // video, audio, picture and source go (DOMPurify's options below remove what is left).
        imagePlaceholders(html, strict) {
            const escapeHtml = (text) => String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
            const attr = (tag, name) => {
                const match = new RegExp("\\s" + name + "\\s*=\\s*(\"([^\"]*)\"|'([^']*)'|([^\\s>]+))", "i").exec(tag);
                return match ? (match[2] !== undefined ? match[2] : match[3] !== undefined ? match[3] : match[4]) : "";
            };
            return String(html || "").replace(/<img\b[^>]*>/gi, (tag) => {
                const src = attr(tag, "src").replace(/&amp;/g, "&");
                const alt = attr(tag, "alt");
                if (!strict && /^\/(?!\/)/.test(src) && !/[\s\\]/.test(src) && src.indexOf("..") < 0 && !/\ssrcset\s*=/i.test(tag))
                    return '<img src="' + escapeHtml(src) + '" alt="' + escapeHtml(alt) + '">';
                const host = assist.hostOf(src) || (src.indexOf("data:") === 0 ? "data" : "");
                return '<span class="m-assist-image" data-assist-image="' + escapeHtml(host) + '">' + escapeHtml(assist.labels.image) +
                    (alt ? ": " + alt : "") + (host ? " (" + escapeHtml(host) + ")" : "") + "</span>";
            });
        },
        // The thread's HTML cleaned: DOMPurify (purify, its options below) to a fragment, then on the cleaned DOM
        // (assist.cleanTree) every image that is not this site's own path a placeholder and every URL-bearing attribute that
        // is not a link's checked href or a site image's src removed. A <image> or an SVG image the parser made into
        // something else is caught here too, as it is the DOM that is walked, not the text.
        // With strict (a canvas's Markdown, which the agent wrote and data fills) no image of any kind and no link but a
        // checked path of the apps is kept.
        sanitize(purify, html, doc, strict) {
            const page = doc || root.document;
            const fragment = purify.sanitize(String(html || ""), Object.assign({}, assist.purifyOptions, { RETURN_DOM_FRAGMENT: true }));
            assist.cleanTree(fragment, page, strict);
            const box = page.createElement("div");
            box.appendChild(fragment);
            return box.innerHTML;
        },
        // whether an image source is this site's own path (shown as it is): /..., no //, no .., no whitespace or backslash
        siteImage(src) {
            const text = String(src || "");
            return /^\/(?!\/)/.test(text) && !/[\s\\]/.test(text) && text.indexOf("..") < 0;
        },
        cleanTree(node, doc, strict) {
            const urlAttrs = ["src", "srcset", "href", "xlink:href", "action", "formaction", "poster", "background", "data", "cite", "longdesc", "lowsrc",
                "dynsrc", "ping", "manifest", "codebase", "archive", "usemap", "icon", "profile"];
            Array.from(node.querySelectorAll("*")).forEach((el) => {
                const tag = String(el.tagName || "").toLowerCase();
                if (tag === "img" && (strict || !assist.siteImage(el.getAttribute("src")) || el.hasAttribute("srcset"))) {
                    const src = el.getAttribute("src") || "";
                    const host = assist.hostOf(src) || (src.indexOf("data:") === 0 ? "data" : "");
                    const alt = el.getAttribute("alt") || "";
                    const span = doc.createElement("span");
                    span.className = "m-assist-image";
                    span.setAttribute("data-assist-image", host);
                    span.textContent = assist.labels.image + (alt ? ": " + alt : "") + (host ? " (" + host + ")" : "");
                    el.parentNode.replaceChild(span, el);
                    return;
                }
                Array.from(el.attributes).forEach((attr) => {
                    const name = attr.name.toLowerCase();
                    const value = String(attr.value || "").trim();
                    if (name.indexOf("on") === 0) { el.removeAttribute(attr.name); return; }
                    if (urlAttrs.indexOf(name) < 0 && name.indexOf(":") < 0) return;
                    const keep = strict ? (tag === "a" && name === "href" && /^\/(?!\/)/.test(value) && assist.safePath(value, null).ok)
                        : (tag === "a" && name === "href" && (/^\/(?!\/)/.test(value) || /^https?:\/\//i.test(value))) || (tag === "img" && name === "src");
                    if (!keep) el.removeAttribute(attr.name);
                });
            });
        },
        // what DOMPurify keeps of the thread's HTML
        purifyOptions: { ADD_ATTR: ["target"], FORBID_TAGS: ["style", "form", "input", "button", "textarea", "select", "video", "audio", "source", "picture",
            "iframe", "object", "embed", "svg", "math"], FORBID_ATTR: ["style", "srcset", "poster", "background"] },
        // The HTML of an assistant text, before DOMPurify: Markdown (moqui.markdown.render on Marked) with record chips from the
        // chip map; a path link only when safePath takes it (then through the shell, data-md-link); an external link with its
        // host, in a new tab; a link the server marked invented (answer.references), a mail, anchor or relative link as text;
        // images as placeholders.
        renderMarkdown(marked, text, options) {
            const opts = options || {};
            const invented = (opts.invented || []).map((item) => String(item));
            const linkBase = opts.linkBase || "/qapps";
            const plain = "#assist-plain";
            const resolveLink = (href, info) => {
                if (info && info.image) return { href };
                const target = String(href || "").trim();
                if (invented.some((item) => item && (target === item || target.indexOf(item) >= 0))) return { href: plain, external: false };
                if (/^(https?:)?\/\//i.test(target)) {
                    if (opts.strict || !/^https?:\/\//i.test(target)) return { href: plain, external: false };
                    return { href: target, external: true, className: "m-assist-external", title: assist.hostOf(target) };
                }
                if (target.charAt(0) === "/") {
                    const checked = assist.safePath(target, null, { linkBase });
                    return checked.ok ? { href: checked.url, external: false } : { href: plain, external: false };
                }
                return { href: plain, external: false };
            };
            const source = assist.chipsToMarkdown(text, opts.chips);
            // the model never needs raw HTML: it is shown as text
            const out = moqui.markdown.render(marked, source, { resolveLink, escapeHtml: true });
            const html = out.html
                // a chip link: the title carries the mark
                .replace(/<a href="([^"]*)" title="assist-chip"([^>]*)>/g, '<a href="$1" class="m-assist-chip"$2>')
                .replace(/<a href="#assist-plain"[^>]*>([\s\S]*?)<\/a>/g, "$1");
            return assist.imagePlaceholders(html, opts.strict);
        },

        // ---- the store: a conversation and its run events ----

        isPaused(state) { return PAUSED.indexOf(state) >= 0; },
        isEnded(state) { return ENDED.indexOf(state) >= 0; },
        emptyState() {
            return { conversationId: null, title: "", agentId: null, items: [], results: {}, runs: {}, activeRunId: null, seq: {}, floor: {},
                approvals: {}, questions: {}, handoffs: {}, queued: [], live: {}, references: {}, notes: [], chips: [],
                follow: { clientId: null, contextVersion: 0, following: false }, contextVersion: 0, hasEarlier: false, status: "",
                canvas: null, canvasDraft: null };
        },
        // the store's view of a run (the conversation's activeRun and runs shape, and what run.state, usage and run.done add)
        runOf(state, runId) {
            if (!runId) return null;
            if (!state.runs[runId]) state.runs[runId] = { runId, state: null, statusId: null, inputTokens: 0, outputTokens: 0 };
            return state.runs[runId];
        },
        itemOf(state, messageId, role, runId) {
            let item = state.items.find((other) => other.messageId === messageId);
            if (!item && role) {
                item = { messageId, seq: null, role, runId: runId || null, clientMessageId: null, parts: [], feedback: null, local: null };
                state.items.push(item);
            }
            return item;
        },
        // The text part a streamed (messageId, part) writes: the part-th text part of the message (the stream numbers from 0,
        // the stored parts from 1), made when it is not there yet.
        textPartOf(state, messageId, part, runId) {
            const item = assist.itemOf(state, messageId, "assistant", runId);
            const texts = item.parts.filter((other) => other.kind === "text");
            const index = Math.max(0, Number(part) || 0);
            while (texts.length <= index) {
                const made = { part: item.parts.length + 1, kind: "text", text: "", done: false };
                item.parts.push(made);
                texts.push(made);
            }
            return texts[index];
        },
        // Text at an offset: only what goes past the text already there is added. Answers false when the offset is past
        // the end (a delta that came before the text it follows).
        appendAt(part, offset, text) {
            const at = Number(offset) || 0;
            const have = part.text.length;
            if (at > have) return false;
            const piece = String(text || "");
            if (at + piece.length > have) part.text += piece.slice(have - at);
            return true;
        },
        stepOf(state, toolCallId) {
            for (let i = 0; i < state.items.length; i++) {
                const found = state.items[i].parts.find((part) => part.kind === "tool-call" && part.toolCallId === toolCallId);
                if (found) return found;
            }
            return null;
        },

        // A conversation as GET conversation answers it (docs/Assist.md), merged into the state: the server's items, runs,
        // cards and follow owner, the messages of this tab not answered yet kept at the end. ctx {clientId}. A run first seen
        // here replays from its start (the reducer is idempotent) with its ui.* events at or below its lastEventSeq never applied
        // (a reload never replays a page action); a run the tab streams keeps its place.
        snapshot(state, json, ctx) {
            const data = json || {};
            const conversation = data.conversation || {};
            if (state.conversationId && conversation.conversationId && state.conversationId !== conversation.conversationId) Object.assign(state, assist.emptyState());
            const keepLocal = state.items.filter((item) => !item.messageId);
            const keepLive = state.items.filter((item) => item.messageId && state.activeRunId && item.runId === state.activeRunId);
            state.conversationId = conversation.conversationId || state.conversationId;
            state.title = conversation.title || state.title || "";
            state.agentId = conversation.agentId || state.agentId;
            state.hasEarlier = !!data.hasEarlier;
            const items = (Array.isArray(data.items) ? data.items : []).map((item) => ({ messageId: item.messageId ? String(item.messageId) : null,
                seq: item.seq, role: item.role, runId: item.runId ? String(item.runId) : null, clientMessageId: item.clientMessageId || null,
                parts: (item.parts || []).map((part) => Object.assign({}, part, part.kind === "text" ? { text: String(part.text || ""), done: true } : {})),
                feedback: item.feedback || null, local: null, chips: item.chips || null }));
            // a streaming message the server does not list yet (or lists with less text) stays as the stream made it
            keepLive.forEach((live) => {
                const stored = items.find((item) => item.messageId === live.messageId);
                if (!stored) items.push(live);
                else live.parts.forEach((part, index) => {
                    const other = stored.parts[index];
                    if (part.kind === "text" && other && other.kind === "text" && part.text.length > other.text.length) { other.text = part.text; other.done = part.done; }
                });
            });
            const results = {};
            items.forEach((item) => item.parts.forEach((part) => {
                if (part.kind === "tool-result" && part.toolCallId) results[part.toolCallId] = { status: part.isError ? "failed" : "done", isError: !!part.isError, text: part.text || "", name: part.name };
            }));
            const pending = data.pending || {};
            const queued = (pending.queued || []).map((entry) => ({ inputId: entry.inputId, clientMessageId: entry.clientMessageId || null, text: entry.text || "",
                receivedDate: entry.receivedDate || null }));
            const known = {};
            items.forEach((item) => { if (item.clientMessageId) known[item.clientMessageId] = true; });
            queued.forEach((entry) => { if (entry.clientMessageId) known[entry.clientMessageId] = true; });
            state.items = items.concat(keepLocal.filter((item) => !known[item.clientMessageId]));
            state.results = Object.assign({}, keepLive.length ? state.results : {}, results);
            state.queued = queued;
            const runs = {};
            (data.runs || []).forEach((run) => { if (run && run.runId) runs[run.runId] = Object.assign({}, state.runs[run.runId], run); });
            const active = data.activeRun && data.activeRun.runId ? data.activeRun : null;
            if (active) runs[active.runId] = Object.assign({}, state.runs[active.runId], runs[active.runId], active);
            state.runs = runs;
            const activeId = active ? String(active.runId) : null;
            if (activeId && state.seq[activeId] === undefined) {
                state.seq[activeId] = 0;
                state.floor[activeId] = Number(active.lastEventSeq) || 0;
            }
            state.activeRunId = activeId;
            const byId = (list, key, status) => {
                const out = {};
                (list || []).forEach((card) => { if (card && card[key]) out[card[key]] = Object.assign({ status }, card); });
                return out;
            };
            state.approvals = byId(pending.approvals, "approvalId", "pending");
            // a decided approval comes on its tool call: drawn collapsed into its status
            items.forEach((item) => item.parts.forEach((part) => {
                const decided = part.kind === "tool-call" && part.approval && part.approval.approvalId ? part.approval : null;
                if (decided && !state.approvals[decided.approvalId]) state.approvals[decided.approvalId] = Object.assign({ toolCallId: part.toolCallId }, decided);
            }));
            state.questions = byId(pending.questions, "questionId", "open");
            // a watch keeps what its handoff.started gave (the page, the form, the values, the tab) beside the server's
            const handoffs = byId(pending.handoffs, "handoffId", "watching");
            Object.keys(handoffs).forEach((key) => {
                const before = state.handoffs[key];
                if (!before) return;
                Object.keys(before).forEach((field) => { if (handoffs[key][field] === null || handoffs[key][field] === undefined) handoffs[key][field] = before[field]; });
            });
            state.handoffs = handoffs;
            assist.takeCanvas(state, data.canvas);
            if (data.follow) {
                state.follow = { clientId: data.follow.clientId || null, contextVersion: Number(data.follow.contextVersion) || 0,
                    following: !!data.follow.clientId && data.follow.clientId === (ctx && ctx.clientId) };
                state.contextVersion = Math.max(Number(state.contextVersion) || 0, state.follow.contextVersion);
            }
            state.status = assist.statusOf(state);
            return state;
        },
        // the turn's (or retry's) answer: the message is the run's, or queued for the active one; the tab owns follow
        turnAnswered(state, clientMessageId, json, ctx) {
            const data = json || {};
            if (data.conversationId) state.conversationId = String(data.conversationId);
            const item = state.items.find((other) => !other.messageId && other.clientMessageId === clientMessageId);
            if (item) { item.local = data.queued ? "queued" : null; item.runId = data.runId ? String(data.runId) : null; item.error = null; }
            if (data.follow) {
                state.follow = { clientId: data.follow.clientId || null, contextVersion: Number(data.follow.contextVersion) || 0,
                    following: !!data.follow.clientId && data.follow.clientId === (ctx && ctx.clientId) };
                state.contextVersion = Math.max(Number(state.contextVersion) || 0, state.follow.contextVersion);
            }
            if (data.runId) {
                const runId = String(data.runId);
                const run = assist.runOf(state, runId);
                if (!data.queued || !state.activeRunId) {
                    if (data.state && !run.state) { run.state = data.state; run.statusId = data.statusId || null; }
                    if (!assist.isEnded(run.state)) state.activeRunId = runId;
                    if (state.seq[runId] === undefined) { state.seq[runId] = 0; state.floor[runId] = 0; }
                }
            }
            state.status = assist.statusOf(state);
            return state;
        },
        // a message of this tab before the server answers it
        addOutgoing(state, clientMessageId, text) {
            state.items.push({ messageId: null, seq: null, role: "user", runId: null, clientMessageId, parts: [{ part: 1, kind: "text", text: String(text || "") }],
                feedback: null, local: "sending" });
            return state;
        },
        turnFailed(state, clientMessageId, error) {
            const item = state.items.find((other) => !other.messageId && other.clientMessageId === clientMessageId);
            if (item) { item.local = "failed"; item.error = error || null; }
            return state;
        },

        // One event of a run's stream or catch-up ({seq, type, payload}; seq null for a live one) applied to the state. ctx
        // {clientId, followOn}. A stored event at or below the run's last sequence changes nothing; one past the next answers
        // a gap and changes nothing (the caller catches up from events.json). Returns the effects for the caller:
        //   {kind: "gap", runId, after}   read the stored events after that sequence
        //   {kind: "follow", runId, event} a ui.* or handoff.started event this tab applies to the page
        //   {kind: "refresh"}             read the conversation again (a card or input it cannot place)
        //   {kind: "paused" | "ended", runId}  the stream ends; open it again after the user acts (paused)
        apply(state, runId, event, ctx) {
            const effects = [];
            if (!event || !event.type) return effects;
            const id = String(runId || "");
            const seq = event.seq === null || event.seq === undefined ? null : Number(event.seq);
            if (seq !== null && !isNaN(seq)) {
                const last = Number(state.seq[id]) || 0;
                if (seq <= last) return effects;
                if (seq > last + 1) { effects.push({ kind: "gap", runId: id, after: last }); return effects; }
                state.seq[id] = seq;
            }
            const p = event.payload || {};
            const run = assist.runOf(state, id);
            const type = event.type;
            switch (type) {
                case "run.state": {
                    run.state = p.state || run.state;
                    run.statusId = p.statusId || run.statusId;
                    run.pauseKind = p.pauseKind || null;
                    run.pauseRef = p.pauseRef || null;
                    if (p.errorCode) run.errorCode = p.errorCode;
                    if (p.reason) run.reason = p.reason;
                    if (assist.isEnded(run.state)) {
                        if (state.activeRunId === id) state.activeRunId = null;
                        assist.endStreaming(state, id);
                        effects.push({ kind: "ended", runId: id });
                    } else {
                        if (!state.activeRunId) state.activeRunId = id;
                        if (assist.isPaused(run.state)) effects.push({ kind: "paused", runId: id });
                    }
                    break;
                }
                case "run.done": {
                    run.done = { state: p.state, result: p.result || null, errorCode: p.errorCode || null };
                    if (p.state) run.state = p.state;
                    if (p.errorCode) run.errorCode = p.errorCode;
                    if (state.activeRunId === id) state.activeRunId = null;
                    assist.endStreaming(state, id);
                    effects.push({ kind: "ended", runId: id });
                    break;
                }
                case "error": run.errorCode = p.code || run.errorCode; run.errorText = p.message || run.errorText; break;
                case "message.start": {
                    const item = assist.itemOf(state, String(p.messageId), p.role || "assistant", id);
                    if (!item.runId) item.runId = id;
                    item.streaming = true;
                    break;
                }
                case "text.delta":
                case "text.chunk": {
                    if (!p.messageId) break;
                    const part = assist.textPartOf(state, String(p.messageId), p.part, id);
                    const item = assist.itemOf(state, String(p.messageId));
                    item.streaming = true;
                    part.done = false;
                    // a delta before the text it follows waits for the stored chunk; a stored chunk past the end is lost text
                    if (!assist.appendAt(part, p.offset, p.text) && type === "text.chunk") effects.push({ kind: "refresh" });
                    break;
                }
                case "part.done": {
                    if (!p.messageId) break;
                    const part = assist.textPartOf(state, String(p.messageId), p.part, id);
                    part.done = true;
                    if (Number(p.length) > part.text.length) effects.push({ kind: "refresh" });
                    break;
                }
                case "step.restart": {
                    const item = assist.itemOf(state, String(p.messageId));
                    if (item) item.parts.forEach((part) => { if (part.kind === "text") { part.text = ""; part.done = false; } });
                    delete state.live[p.messageId];
                    break;
                }
                case "reasoning.delta": {
                    const live = state.live[p.messageId] || (state.live[p.messageId] = { reasoning: "", args: "" });
                    live.reasoning += String(p.text || "");
                    break;
                }
                case "reasoning.summary": {
                    const item = assist.itemOf(state, String(p.messageId), "assistant", id);
                    if (!item.parts.some((part) => part.kind === "reasoning" && part.text === p.text)) item.parts.push({ part: item.parts.length + 1, kind: "reasoning", text: String(p.text || "") });
                    delete state.live[p.messageId];
                    break;
                }
                case "tool.args": {
                    const live = state.live[p.messageId] || (state.live[p.messageId] = { reasoning: "", args: "" });
                    if (p.name) live.tool = p.name;
                    live.args += String(p.text || "");
                    break;
                }
                case "tool.call": {
                    const item = assist.itemOf(state, String(p.messageId), "assistant", id);
                    if (!assist.stepOf(state, p.toolCallId)) item.parts.push({ part: item.parts.length + 1, kind: "tool-call", toolCallId: p.toolCallId, name: p.name,
                        arguments: typeof p.arguments === "string" ? p.arguments : JSON.stringify(p.arguments || {}), classification: p.classification || null, status: "running" });
                    if (state.live[p.messageId]) { state.live[p.messageId].args = ""; state.live[p.messageId].tool = null; }
                    break;
                }
                case "tool.result": {
                    state.results[p.toolCallId] = { status: p.status || (p.isError ? "failed" : "done"), isError: !!p.isError, text: p.text || "", name: p.name };
                    const step = assist.stepOf(state, p.toolCallId);
                    if (step) step.status = p.status || (p.isError ? "failed" : "done");
                    break;
                }
                case "input.folded": {
                    const index = state.queued.findIndex((entry) => entry.inputId === p.inputId);
                    if (index < 0) { effects.push({ kind: "refresh" }); break; }
                    const entry = state.queued.splice(index, 1)[0];
                    const local = state.items.findIndex((item) => !item.messageId && item.clientMessageId && item.clientMessageId === entry.clientMessageId);
                    if (local >= 0) state.items.splice(local, 1);
                    state.items.push({ messageId: "input:" + entry.inputId, seq: null, role: "user", runId: id, clientMessageId: entry.clientMessageId,
                        parts: [{ part: 1, kind: "text", text: entry.text }], feedback: null, local: null });
                    break;
                }
                case "usage": {
                    run.inputTokens = (Number(run.inputTokens) || 0) + (Number(p.input) || 0);
                    run.outputTokens = (Number(run.outputTokens) || 0) + (Number(p.output) || 0);
                    if (p.cost !== null && p.cost !== undefined) run.cost = (Number(run.cost) || 0) + Number(p.cost);
                    break;
                }
                case "compaction": run.compactions = (Number(run.compactions) || 0) + 1; break;
                case "answer.references": {
                    const list = Array.isArray(p.invented) ? p.invented : p.invented ? [p.invented] : [];
                    state.references[p.messageId] = (state.references[p.messageId] || []).concat(list.map((item) => (item && typeof item === "object" ? item.href || item.text || "" : String(item))));
                    break;
                }
                case "approval.requested": {
                    if (!p.approvalId) break;
                    const before = state.approvals[p.approvalId];
                    state.approvals[p.approvalId] = Object.assign({}, before, p, { status: p.status || "pending" });
                    break;
                }
                case "approval.decided": {
                    const card = state.approvals[p.approvalId];
                    const status = p.status || ({ approve: "approved", reject: "rejected", edit: "handed-off", expire: "expired" })[p.decision] || "decided";
                    if (card) Object.assign(card, { status, decision: p.decision, feedback: p.feedback || card.feedback || null, handoffId: p.handoffId || card.handoffId || null,
                        decisionChannel: p.channel || card.decisionChannel || null, local: null });
                    else state.approvals[p.approvalId] = { approvalId: p.approvalId, toolCallId: p.toolCallId, status, decision: p.decision, runId: id };
                    break;
                }
                case "question.asked": {
                    if (!p.questionId) break;
                    state.questions[p.questionId] = Object.assign({ runId: id, status: "open" }, state.questions[p.questionId], p,
                        p.kind === "budget" && !p.options ? { options: (p.choices || ["continue", "stop"]).map((label) => ({ label })) } : {}, { status: "open" });
                    break;
                }
                case "question.answered": {
                    Object.keys(state.questions).forEach((key) => {
                        const card = state.questions[key];
                        if ((p.questionId && key === p.questionId) || (p.toolCallId && card.toolCallId === p.toolCallId)) { card.status = "answered"; card.answer = p.answer !== undefined ? p.answer : p.continue; }
                    });
                    break;
                }
                case "handoff.started": {
                    if (!p.handoffId) break;
                    state.handoffs[p.handoffId] = Object.assign({ runId: id }, state.handoffs[p.handoffId], p, { status: "watching" });
                    if (assist.follow.applies(state, id, event, ctx)) effects.push({ kind: "follow", runId: id, event });
                    break;
                }
                case "handoff.done": {
                    const card = state.handoffs[p.handoffId];
                    if (card) Object.assign(card, { status: p.outcome || "saved", outcome: p.outcome, messages: p.messages || null, savedValues: p.values || null });
                    break;
                }
                case "canvas.delta": {
                    // a write_ui program as it streams: a whole one is drawn as it comes, a patch shows "updating"
                    const draft = state.canvasDraft;
                    const offset = Number(p.offset) || 0;
                    if (offset === 0 || !draft || draft.messageId !== p.messageId || draft.index !== p.index) {
                        if (offset === 0) state.canvasDraft = { messageId: p.messageId, index: p.index, patch: !!p.patch, text: String(p.text || "") };
                    } else if (draft.text.length === offset) draft.text += String(p.text || "");
                    if (state.canvasDraft && !state.canvasDraft.patch) effects.push({ kind: "canvas" });
                    break;
                }
                case "canvas.open": {
                    assist.takeCanvas(state, p);
                    state.canvasDraft = null;
                    effects.push({ kind: "canvas" });
                    break;
                }
                case "artifact.update": break;
                case "ui.navigate":
                case "ui.fill":
                case "ui.dialog":
                case "ui.focus":
                case "ui.highlight": {
                    if (assist.follow.applies(state, id, event, ctx)) effects.push({ kind: "follow", runId: id, event });
                    else if (seq !== null && seq > (Number(state.floor[id]) || 0)) assist.addChip(state, event);
                    break;
                }
                default: break;
            }
            state.status = assist.statusOf(state);
            return effects;
        },
        // The conversation's canvas (GET conversation's canvas, a canvas.open payload): a newer version replaces it; the same one
        // keeps what this tab did with it (shown or hidden, the values it has).
        takeCanvas(state, canvas) {
            if (!canvas || !canvas.artifactId) { state.canvas = null; return state; }
            const before = state.canvas;
            const same = before && before.artifactId === String(canvas.artifactId);
            if (same && Number(before.version) > Number(canvas.version)) return state;
            state.canvas = Object.assign({}, canvas, { artifactId: String(canvas.artifactId), version: Number(canvas.version) || 0,
                program: String(canvas.program || ""), problems: Array.isArray(canvas.problems) ? canvas.problems : [], closed: !!canvas.closed,
                state: canvas.state && typeof canvas.state === "object" ? canvas.state : {},
                hidden: same && Number(before.version) === Number(canvas.version) ? !!before.hidden : false });
            return state;
        },
        // What the canvas draws now: the program streaming in (a whole one) while write_ui streams, else the stored one.
        canvasView(state) {
            const canvas = state.canvas;
            const draft = state.canvasDraft;
            if (draft && !draft.patch) return { program: draft.text, streaming: true, title: canvas ? canvas.title : "", version: canvas ? canvas.version + 1 : 1,
                artifactId: canvas ? canvas.artifactId : "", kind: "openui", state: canvas ? canvas.state : {} };
            if (!canvas || canvas.closed) return null;
            return { program: canvas.program, streaming: false, title: canvas.title, version: canvas.version, artifactId: canvas.artifactId,
                kind: canvas.kind || "openui", state: canvas.state || {}, updating: !!draft };
        },
        // the messages of a run that ended stream no more
        endStreaming(state, runId) {
            state.items.forEach((item) => {
                if (item.runId !== runId) return;
                item.streaming = false;
                item.parts.forEach((part) => { if (part.kind === "text") part.done = true; });
            });
            Object.keys(state.live).forEach((key) => {
                const item = state.items.find((other) => other.messageId === key);
                if (!item || item.runId === runId) delete state.live[key];
            });
        },
        // A page action this tab did not apply, as a chip: "Assist opened Orders [Show]", "Assist filled Name". At most MAX_CHIPS,
        // the newest last; a navigation replaces the chips before it.
        addChip(state, event) {
            const p = event.payload || {};
            let chip = null;
            if (event.type === "ui.navigate" && p.path) {
                chip = { key: "nav:" + (event.seq || ""), kind: "navigate", label: p.title || p.path, path: p.path, parameters: p.parameters || {} };
                state.chips = [];
            } else if (event.type === "ui.fill") chip = { key: "fill:" + (event.seq || "") + ":" + p.field, kind: "fill", label: p.label || p.field || "" };
            else if (event.type === "ui.dialog" && p.open) chip = { key: "dialog:" + (event.seq || ""), kind: "dialog", label: p.title || p.id || "" };
            if (!chip) return;
            state.chips.push(chip);
            if (state.chips.length > MAX_CHIPS) state.chips.splice(0, state.chips.length - MAX_CHIPS);
        },
        // the one status line: what the active run is doing or waits for
        statusOf(state) {
            const run = state.activeRunId ? state.runs[state.activeRunId] : null;
            if (!run) return "";
            const labels = assist.labels;
            if (assist.isPaused(run.state)) {
                if (run.pauseKind === "AgpsApproval" || run.pauseKind === "approval" || run.state === "TASK_STATE_AUTH_REQUIRED") return labels.waitingApproval;
                if (run.pauseKind === "AgpsHandoff" || run.pauseKind === "handoff") return labels.waitingHandoff;
                return labels.waitingAnswer;
            }
            const live = Object.keys(state.live).map((key) => state.live[key]).find((entry) => entry.tool);
            return live ? labels.working + ": " + live.tool : labels.working;
        },
        // the follow owner and page version a call answered (turn, follow, context, decide): this tab's version is at least it
        takeFollow(state, follow, clientId) {
            const own = clientId !== undefined ? clientId : (moqui.assist && moqui.assist.clientId);
            state.follow = { clientId: follow.clientId || null, contextVersion: Number(follow.contextVersion) || 0,
                following: !!follow.clientId && follow.clientId === own };
            state.contextVersion = Math.max(Number(state.contextVersion) || 0, state.follow.contextVersion);
            return state;
        },
        // the version of the page this tab reports next (its own navigation raised it)
        nextContextVersion(state) {
            state.contextVersion = Math.max(Number(state.contextVersion) || 0, Number(state.follow && state.follow.contextVersion) || 0) + 1;
            return state.contextVersion;
        },

        // The rows of the thread, in order: user messages (a local one with its status), an assistant message's text,
        // reasoning ("Thought") and step rows (each tool call with its result), the run's done line after its last message, and
        // the queued messages and notes at the end. A tool message is not a row: its result is on its step.
        threadRows(state) {
            const rows = [];
            const lastOfRun = {};
            state.items.forEach((item, index) => { if (item.runId && item.role !== "user") lastOfRun[item.runId] = index; });
            const doneRow = (runId) => {
                const run = state.runs[runId];
                if (!run || !assist.isEnded(run.state) || state.activeRunId === runId) return;
                const tokens = (Number(run.inputTokens) || 0) + (Number(run.outputTokens) || 0);
                rows.push({ key: "done:" + runId, kind: "done", runId, state: run.state, steps: Number(run.stepCount) || 0, tokens,
                    seconds: Number(run.activeSeconds) || 0, cost: run.cost === null || run.cost === undefined ? null : run.cost,
                    errorCode: run.errorCode || (run.done && run.done.errorCode) || null, errorText: run.errorText || null,
                    failed: run.state === "TASK_STATE_FAILED" || run.state === "TASK_STATE_CANCELED" });
            };
            state.items.forEach((item, index) => {
                const key = item.messageId || "local:" + item.clientMessageId;
                if (item.role === "user") {
                    if (item.local === "queued") return;
                    rows.push({ key, kind: "user", text: item.parts.filter((part) => part.kind === "text").map((part) => part.text).join("\n"),
                        local: item.local, error: item.error || null, clientMessageId: item.clientMessageId, segments: assist.segments(
                            item.parts.filter((part) => part.kind === "text").map((part) => part.text).join("\n"), null, true) });
                } else if (item.role === "assistant") {
                    item.parts.forEach((part, partIndex) => {
                        const partKey = key + ":" + partIndex;
                        if (part.kind === "text") {
                            if (!part.text) return;
                            rows.push({ key: partKey, kind: "text", messageId: item.messageId, text: part.text, streaming: !!item.streaming && !part.done,
                                invented: state.references[item.messageId] || [], chips: item.chips || null, feedback: item.feedback, runId: item.runId });
                        } else if (part.kind === "reasoning") rows.push({ key: partKey, kind: "reasoning", text: part.text });
                        else if (part.kind === "tool-call") {
                            const result = state.results[part.toolCallId] || null;
                            rows.push({ key: partKey, kind: "step", toolCallId: part.toolCallId, name: part.name, title: assist.stepTitle(part.name, part.arguments),
                                arguments: part.arguments || "", status: result ? result.status : assist.stepStatus(part.status), result: result ? result.text : "",
                                isError: !!(result && result.isError), approvalId: part.approvalId || null });
                        }
                    });
                    const live = state.live[item.messageId];
                    if (live && live.reasoning) rows.push({ key: key + ":thinking", kind: "thinking", text: live.reasoning });
                }
                if (item.runId && lastOfRun[item.runId] === index) doneRow(item.runId);
            });
            state.queued.forEach((entry) => rows.push({ key: "queued:" + entry.inputId, kind: "queued", text: entry.text, inputId: entry.inputId }));
            state.items.filter((item) => item.role === "user" && item.local === "queued").forEach((item) => rows.push({ key: "queued:" + item.clientMessageId, kind: "queued",
                text: item.parts.map((part) => part.text || "").join("\n"), clientMessageId: item.clientMessageId }));
            state.notes.forEach((note) => rows.push(Object.assign({ kind: "note" }, note)));
            return rows;
        },
        // a stored tool call's status (AgtcDone ...) as a step's
        stepStatus(status) {
            const text = String(status || "");
            if (/Done$/.test(text)) return "done";
            if (/Fail|Error/.test(text)) return "failed";
            if (/Denied|Refused/.test(text)) return "denied";
            if (/Unknown/.test(text)) return "unknown-outcome";
            if (/Await|Wait/.test(text)) return "waiting";
            if (!text || /Run|Start|Pend|running/i.test(text)) return "running";
            return text.toLowerCase();
        },
        // A step's title: its tool and what it was asked, from the arguments (the action, a path, a query or the tool run).
        stepTitle(name, args) {
            let parsed = null;
            if (args && typeof args === "object") parsed = args;
            else try { parsed = args ? JSON.parse(args) : null; } catch (err) { parsed = null; }
            // the screen tool's press and page ("Save on /apps/Orders/Edit"), else the first of the usual arguments
            const screen = parsed && typeof parsed === "object" && (typeof parsed.do === "string" || typeof parsed.go === "string") ?
                [parsed.do, parsed.go].filter((value) => typeof value === "string" && value).join(" on ") : "";
            const detail = parsed && typeof parsed === "object" ? (["action", "path", "query", "name", "tool", "skill", "question", "text"].map((key) => parsed[key])
                .find((value) => typeof value === "string" && value) || screen) : "";
            const short = detail && detail.length > 60 ? detail.slice(0, 59) + "…" : detail;
            return String(name || "") + (short ? " · " + short : "");
        },
        // the done line's text: steps, tokens, time and cost (cost only when the server gave it)
        doneText(row) {
            const labels = assist.labels;
            const parts = [];
            if (row.steps) parts.push(row.steps + " " + (row.steps === 1 ? labels.step : labels.steps));
            if (row.tokens) parts.push(row.tokens.toLocaleString ? row.tokens.toLocaleString() + " " + labels.tokens : row.tokens + " " + labels.tokens);
            if (row.seconds) parts.push(row.seconds + labels.seconds);
            if (row.cost !== null && row.cost !== undefined && Number(row.cost) > 0) parts.push(labels.cost + " " + Number(row.cost).toFixed(4));
            return parts.join(" · ");
        },

        // ---- the page: follow and the page state ----

        follow: {
            // Whether this tab applies a ui.* (or handoff.started) event: follow on, the event for this tab's client id and page
            // version, and past the run's replay floor (a reload never applies an old page action).
            applies(state, runId, event, ctx) {
                const p = event && event.payload;
                if (!p || !ctx || !ctx.followOn || !ctx.clientId) return false;
                if (p.clientId !== ctx.clientId) return false;
                if (Number(p.contextVersion) !== Number(state.contextVersion)) return false;
                const seq = event.seq === null || event.seq === undefined ? null : Number(event.seq);
                if (seq === null || isNaN(seq)) return false;
                return seq > (Number(state.floor[runId]) || 0);
            },
            // Where a fill, focus or highlight goes (target: the event's {scope, form, domId, occurrence, row, dialog, field})
            // among the page's forms (forms: in DOM order, each {name, id, dialog, inReach, fields: [names], types: {name: type},
            // multi}). The form is found
            //   - for a list row (row given): the row form <form>_<row>, else the multi form <form> with the field <field>_<row>;
            //   - by its qvt id (domId) when one form has it; several with it (a static render repeats the bare name) by occurrence;
            //   - else by name: the name or <name>_<n> (a merged render appends the section entry index), the occurrence-th in DOM
            //     order, within the scope (a dialog's forms for dialog, the page's outside dialogs first for page and find).
            // Answers {ok: true, form, index, field} or {ok: false, reason}: none, reach (outside assistReach), closed (a hidden,
            // password or file field), secret or field (the form has no such field).
            locate(target, forms, secretFields) {
                const t = target || {};
                const list = (forms || []).map((form, index) => Object.assign({ index }, form));
                const scoped = (() => {
                    if (t.scope === "dialog") return list.filter((form) => form.dialog && (!t.dialog || form.dialog === t.dialog));
                    const page = list.filter((form) => !form.dialog);
                    return page.length ? page.concat(list.filter((form) => form.dialog)) : list;
                })();
                const nameMatch = (form, name) => {
                    if (!name) return false;
                    if (form.name === name || form.id === name) return true;
                    const escaped = String(name).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
                    return new RegExp("^" + escaped + "_\\d+$").test(form.name || "") || new RegExp("^" + escaped + "_\\d+$").test(form.id || "");
                };
                let found = null;
                let field = t.field ? String(t.field) : "";
                const occurrence = Math.max(0, Number(t.occurrence) || 0);
                const nth = (candidates) => candidates[occurrence] || (candidates.length === 1 ? candidates[0] : null);
                if (t.row !== null && t.row !== undefined && t.row !== "") {
                    const rowName = t.form + "_" + t.row;
                    found = scoped.find((form) => form.name === rowName || form.id === rowName) || null;
                    if (!found) {
                        found = scoped.find((form) => form.multi && (form.name === t.form || form.id === t.form)) || null;
                        if (found) field = field + "_" + t.row;
                    }
                } else {
                    if (t.domId) {
                        const byId = scoped.filter((form) => form.id === t.domId);
                        found = byId.length === 1 ? byId[0] : byId.length ? nth(byId) : null;
                    }
                    if (!found) found = nth(scoped.filter((form) => nameMatch(form, t.form)));
                }
                if (!found) return { ok: false, reason: "none" };
                if (!found.inReach) return { ok: false, reason: "reach" };
                if (!field) return { ok: true, form: found, index: found.index, field: "" };
                if (assist.isSecret(field, secretFields) || assist.isSecret(t.field, secretFields)) return { ok: false, reason: "secret" };
                const type = (found.types || {})[field];
                if (type && CLOSED_TYPES.indexOf(String(type).toLowerCase()) >= 0) return { ok: false, reason: "closed" };
                if (Array.isArray(found.fields) && found.fields.length && found.fields.indexOf(field) < 0) return { ok: false, reason: "field" };
                return { ok: true, form: found, index: found.index, field };
            },
            // The forms part of the page state (docs/Assist.md): for each form the values the user changed (compared with the
            // values it came with) and their names in dirty. A secret name is never read; a hidden, password or file field, a
            // transition parameter and a value that is not text, a number, a boolean or a list of them are left out. Capped at
            // limits {maxForms, maxFormValues, maxValueChars}; a form with nothing changed is not listed.
            unsaved(forms, secretFields, limits) {
                const caps = Object.assign({}, DEFAULT_LIMITS, limits);
                const out = [];
                (forms || []).forEach((form) => {
                    if (out.length >= caps.maxForms || !form || !form.id) return;
                    const values = {};
                    const dirty = [];
                    const now = form.values || {};
                    const original = form.original || {};
                    Object.keys(now).forEach((name) => {
                        if (dirty.length >= caps.maxFormValues) return;
                        if (META_FIELDS.indexOf(name) >= 0 || assist.isSecret(name, secretFields)) return;
                        const type = (form.types || {})[name];
                        if (type && CLOSED_TYPES.indexOf(String(type).toLowerCase()) >= 0) return;
                        const text = assist.valueText(now[name]);
                        if (text === null) return;
                        const before = assist.valueText(original[name]);
                        if ((before === null ? "" : before) === text) return;
                        values[name] = text.length > caps.maxValueChars ? text.slice(0, caps.maxValueChars) : text;
                        dirty.push(name);
                    });
                    if (dirty.length) out.push({ domId: String(form.id), values, dirty });
                });
                return out;
            }
        },
        // a field value as the page state sends it: text, a number or boolean written, a list joined with commas; null otherwise
        valueText(value) {
            if (value === null || value === undefined) return "";
            if (typeof value === "string") return value;
            if (typeof value === "number" || typeof value === "boolean") return String(value);
            if (Array.isArray(value)) return value.every((item) => item === null || ["string", "number", "boolean"].indexOf(typeof item) >= 0) ? value.join(",") : null;
            return null;
        },
        // The page state a turn and context carry (docs/Assist.md "The page state"), from the root's screenContext and what the
        // page adds: {title, forms (follow.unsaved), dialogs, canvas}.
        pageState(context, extra) {
            const ctx = context || {};
            const more = extra || {};
            const state = { path: ctx.path || "", parameters: {} };
            Object.keys(ctx.parameters || {}).slice(0, 50).forEach((key) => {
                const text = assist.valueText(ctx.parameters[key]);
                if (text !== null && text !== "") state.parameters[key] = text;
            });
            if (more.title) state.title = String(more.title);
            state.forms = more.forms || [];
            state.dialogs = (more.dialogs || []).slice(0, 10);
            if (ctx.record && ctx.record.entity && ctx.record.id) state.record = { entityName: ctx.record.entity, id: String(ctx.record.id), label: ctx.record.title || "" };
            const selection = ctx.selection;
            if (selection && Array.isArray(selection.ids) && selection.ids.length)
                state.selections = [{ form: selection.form, ids: selection.ids.slice(0, 20).map(String), total: selection.ids.length }];
            if (Array.isArray(ctx.filters) && ctx.filters.length) state.filters = ctx.filters.slice(0, 10).map((filter) => ({ form: filter.form, values: Object.assign({}, filter.parameters) }));
            if (Array.isArray(ctx.forms) && ctx.forms.length) state.formInfo = ctx.forms.slice(0, 10).map((form) => {
                const info = { name: form.name, kind: form.kind, fields: (form.fields || []).slice(0, 20) };
                if (typeof form.rows === "number") info.rows = form.rows;
                return info;
            });
            if (more.canvas) state.canvas = more.canvas;
            return state;
        },
        // Whether an answered form (moqui.onFormAnswer's form: {name, id, action}) is the hand-off's: its name or id, or a name
        // with the section entry index, or a form posting to the hand-off's transition.
        handoffMatches(handoff, form) {
            if (!handoff || !form) return false;
            const name = String(handoff.form || "");
            const names = [form.name, form.id].filter((value) => value);
            if (name && names.some((value) => value === name || new RegExp("^" + name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "_\\d+$").test(value))) return true;
            const action = String(form.action || "").split("?")[0];
            return !!handoff.transition && action.endsWith("/" + handoff.transition);
        },

        // ---- the event stream ----

        // Server-Sent Events from text: the complete events ({id, event, data}, comments and retry left out) and the rest that
        // waits for more.
        parseSse(text) {
            const events = [];
            const source = String(text || "").replace(/\r\n?/g, "\n");
            let start = 0;
            let end;
            while ((end = source.indexOf("\n\n", start)) >= 0) {
                const block = source.slice(start, end);
                start = end + 2;
                const frame = { id: null, event: "message", data: null };
                const data = [];
                block.split("\n").forEach((line) => {
                    if (!line || line.charAt(0) === ":") return;
                    const colon = line.indexOf(":");
                    const field = colon < 0 ? line : line.slice(0, colon);
                    let value = colon < 0 ? "" : line.slice(colon + 1);
                    if (value.charAt(0) === " ") value = value.slice(1);
                    if (field === "id") frame.id = value;
                    else if (field === "event") frame.event = value;
                    else if (field === "data") data.push(value);
                });
                if (data.length) { frame.data = data.join("\n"); events.push(frame); }
            }
            return { events, rest: source.slice(start) };
        },
        // a frame's event: {seq, type, payload}, the id the sequence when the data has none
        frameEvent(frame) {
            if (!frame || frame.data === null) return null;
            let data;
            try { data = JSON.parse(frame.data); } catch (err) { return null; }
            if (!data || typeof data !== "object") return null;
            const seq = data.seq !== null && data.seq !== undefined ? Number(data.seq) : frame.id ? Number(frame.id) : null;
            return { seq: seq === null || isNaN(seq) ? null : seq, type: data.type || frame.event, payload: data.payload || {} };
        },
        // the parameters of a call as moqui.request posts them: an object or list as JSON text, a boolean as true/false, none left out
        callParams(params) {
            const out = {};
            Object.keys(params || {}).forEach((key) => {
                const value = params[key];
                if (value === null || value === undefined) return;
                out[key] = typeof value === "object" ? JSON.stringify(value) : String(value);
            });
            return out;
        }
    };
    // The canvas dialects (plan 4.10): each registers its component when its script loads (js/WebrootAssistCanvas.js
    // registers openui); the server lists the ones an agent offers in bootstrap's dialects.
    assist.dialects = {
        list: {},
        register(name, def) { assist.dialects.list[name] = Object.assign({ name }, def); return assist.dialects.list[name]; },
        get(name) { return assist.dialects.list[name || "openui"] || null; },
    };
    moqui.assist = assist;

    // ---- the drawer in the page (assist.start; nothing below runs under Node) ----

    // the state the components draw
    assist.state = reactive({ available: false, bootstrap: null, open: false, mounted: false, pane: "chat", width: 420, followOn: true,
        convo: assist.emptyState(), draft: "", tier: null, agentId: null, tasks: null, taskCount: 0, history: { list: [], search: "", loading: false, hasMore: false },
        streaming: false, polling: false, error: "", announce: "", busy: {}, canvasLoaded: false, canvasError: "" });

    const S = () => assist.state;
    const labelsOf = (vm) => Object.assign({}, assist.labels, vm && vm.$root && vm.$root.labels ? pick(vm.$root.labels) : null);
    // the page's labels Assist takes (qapps.xml localizes assist and askAssist; the others are moqui.assist.labels)
    const pick = (labels) => {
        const out = {};
        Object.keys(assist.labels).forEach((key) => { if (labels[key] !== undefined) out[key] = labels[key]; });
        return out;
    };
    const storage = (kind) => { try { return root[kind] || null; } catch (err) { return null; } };
    const keep = (kind, key, value) => {
        const store = storage(kind);
        if (!store) return;
        try { if (value === null || value === undefined) store.removeItem(key); else store.setItem(key, String(value)); } catch (err) { /* not kept */ }
    };
    const kept = (kind, key) => { const store = storage(kind); try { return store ? store.getItem(key) : null; } catch (err) { return null; } };

    Object.assign(assist, {
        vm: null,
        clientId: null,
        stream: null,
        agentNav: false,
        agentNavAt: 0,
        lastInputAt: 0,
        followQueue: [],
        followBusy: false,
        lastSaveOk: 0,
        ctx() { return { clientId: assist.clientId, followOn: S().followOn }; },
        paths() { return (S().bootstrap && S().bootstrap.paths) || { calls: "/assist", events: "/assist/runs/{runId}/events", catchUp: "/assist/runs/{runId}/events.json" }; },
        base() { return (assist.vm && assist.vm.appRootPath) || moqui.appRootPath || ""; },
        // a call of /assist (docs/Assist.md): resolves to its JSON, rejects with {status, json, errors}
        call(name, params, method) {
            const verb = method || "GET";
            return moqui.request(assist.base() + assist.paths().calls + "/" + name, { method: verb, params: assist.callParams(params) }).then((result) => result.json || {}, (err) => {
                const json = err && err.json ? err.json : null;
                throw { status: err ? err.status : 0, json, errors: json && json.errors ? String(json.errors) : (err && err.status ? "HTTP " + err.status : "network") };
            });
        },
        secretFields() { return (S().bootstrap && S().bootstrap.secretFields) || DEFAULT_SECRETS; },
        limits() { return Object.assign({}, DEFAULT_LIMITS, S().bootstrap && S().bootstrap.limits); },
        isAdmin() { return !!(S().bootstrap && S().bootstrap.features && S().bootstrap.features.admin); },

        // Start in the page: the tab's id, the bootstrap (which shows the button for a user who may use Assist), the keys, the
        // URL listener, the hand-off's form answers, the AgentRun signals and the page's visibility.
        start(vm) {
            // the page's settings say whether the user may use Assist (qapps.xml: the llm feature on and AgentUse)
            if (assist.vm || !(moqui.config && moqui.config.assist)) return;
            assist.vm = vm;
            assist.clientId = assist.clientIdOf(storage("sessionStorage"));
            if (vm.labels) Object.assign(assist.labels, pick(vm.labels));
            const width = Number(kept("localStorage", "ASSIST_WIDTH"));
            if (width >= 320 && width <= 720) S().width = width;
            S().followOn = kept("localStorage", "ASSIST_FOLLOW") !== "false";
            if (vm.urlListeners) vm.urlListeners.push((url) => assist.onUrl(url));
            assist.formAnswerOff = moqui.onFormAnswer ? moqui.onFormAnswer((answer) => assist.onFormAnswer(answer)) : null;
            if (moqui.keymap) assist.keysOff = moqui.keymap.registerScope("global", [
                { keys: "Mod+J", action: "assist", label: assist.labels.assist, run: () => { assist.toggle(true); return true; } },
                { keys: "Mod+.", action: "assistStop", label: assist.labels.stop, when: () => !!S().convo.activeRunId, run: () => { assist.stop(); return true; } }
            ]);
            if (vm.notificationClient) vm.notificationClient.registerListener("AgentRun", (message) => assist.onSignal(message));
            const doc = root.document;
            if (doc && doc.addEventListener) {
                doc.addEventListener("visibilitychange", () => assist.onVisibility());
                // the user's own input, which tells a navigation of theirs from one the page makes after the agent's
                const input = () => { assist.lastInputAt = Date.now(); };
                doc.addEventListener("pointerdown", input, true);
                doc.addEventListener("keydown", input, true);
            }
            assist.loadBootstrap();
        },
        loadBootstrap() {
            return assist.call("bootstrap").then((json) => {
                S().bootstrap = json;
                const features = json.features || {};
                S().available = features.enabled !== false && Array.isArray(json.agents) && json.agents.length > 0;
                if (!S().available) return;
                S().agentId = json.defaultAgentId || json.agents[0].agentId;
                const wanted = assist.wantedConversation();
                if (wanted.id) {
                    assist.load(wanted.id).catch(() => {});
                    if (wanted.open) assist.openDrawer();
                }
                assist.refreshTasks();
            }, (err) => { S().available = false; if (err.status && err.status !== 403 && err.status !== 404 && err.status !== 401) console.info("Assist is not available", err.errors); });
        },
        // ?assist=<conversationId> opens the drawer on it; else the tab's conversation of before a reload, open as it was
        wantedConversation() {
            const search = root.location && root.location.search ? root.location.search : "";
            const match = /[?&]assist=([^&#]+)/.exec(search);
            if (match) return { id: decodeURIComponent(match[1]), open: true };
            return { id: kept("sessionStorage", "moqui.assist.conversation"), open: kept("sessionStorage", "moqui.assist.open") === "true" };
        },

        // ---- the drawer ----

        openDrawer(focus) {
            S().open = true;
            S().mounted = true;
            keep("sessionStorage", "moqui.assist.open", "true");
            if (focus) assist.focusComposer();
        },
        closeDrawer() { S().open = false; keep("sessionStorage", "moqui.assist.open", "false"); },
        // Mod+J and the header button: open and focus the composer; from inside the drawer it closes
        toggle(fromKey) {
            const doc = root.document;
            const inside = doc && doc.activeElement && doc.activeElement.closest && doc.activeElement.closest("#m-assist-drawer");
            if (S().open && (!fromKey || inside)) assist.closeDrawer();
            else assist.openDrawer(true);
        },
        focusComposer() {
            setTimeout(() => {
                const el = root.document && root.document.getElementById("m-assist-input");
                if (el && el.focus) el.focus();
            }, 60);
        },
        setWidth(width) {
            S().width = Math.max(320, Math.min(720, Math.round(width)));
            keep("localStorage", "ASSIST_WIDTH", S().width);
        },
        setFollowOn(on) {
            S().followOn = !!on;
            keep("localStorage", "ASSIST_FOLLOW", on ? "true" : "false");
            if (on && S().convo.conversationId && !S().convo.follow.following) assist.followHere();
        },
        // the palette's Ask: open the drawer and send the text
        ask(text) {
            assist.openDrawer(true);
            if (String(text || "").trim()) assist.send(String(text));
        },
        newConversation() {
            assist.closeStream();
            Object.assign(S().convo, assist.emptyState());
            S().pane = "chat";
            keep("sessionStorage", "moqui.assist.conversation", null);
            assist.focusComposer();
        },

        // ---- the conversation ----

        load(conversationId, options) {
            if (!conversationId) return Promise.resolve();
            const opts = options || {};
            return assist.call("conversation", { conversationId, clientId: assist.clientId }).then((json) => {
                const convo = S().convo;
                if (opts.onlyIf && convo.conversationId !== opts.onlyIf) return;
                assist.snapshot(convo, json, assist.ctx());
                keep("sessionStorage", "moqui.assist.conversation", convo.conversationId);
                S().pane = opts.pane || S().pane;
                // a watch of this page: its form's Save ringed (again, as a page drawn since lost the mark)
                Object.keys(convo.handoffs).forEach((key) => { if (assist.watching(convo.handoffs[key])) assist.markSave(convo.handoffs[key]); });
                assist.ensureStream();
            }, (err) => {
                if (err.status === 404) {
                    keep("sessionStorage", "moqui.assist.conversation", null);
                    if (S().convo.conversationId === conversationId) Object.assign(S().convo, assist.emptyState());
                }
                throw err;
            });
        },
        refresh() {
            clearTimeout(assist.refreshTimer);
            const id = S().convo.conversationId;
            assist.refreshTimer = setTimeout(() => { assist.load(id, { onlyIf: id }).catch(() => {}); }, 150);
        },
        loadEarlier() {
            const convo = S().convo;
            const first = convo.items.find((item) => item.seq !== null && item.seq !== undefined);
            if (!convo.conversationId || !first) return Promise.resolve();
            return assist.call("conversation", { conversationId: convo.conversationId, clientId: assist.clientId, beforeSeq: first.seq }).then((json) => {
                const older = (json.items || []).filter((item) => !convo.items.some((other) => other.messageId === String(item.messageId)));
                const scratch = assist.emptyState();
                assist.snapshot(scratch, { conversation: json.conversation, items: older }, assist.ctx());
                convo.items.splice(0, 0, ...scratch.items);
                Object.assign(convo.results, scratch.results);
                convo.hasEarlier = !!json.hasEarlier;
            });
        },

        // ---- the canvas (docs/Assist.md "The canvas"; the dialects are js/WebrootAssistCanvas.js) ----

        // whether the canvas covers the page now: the drawer open on a conversation with a canvas the user has not hidden
        canvasShown() {
            const view = assist.canvasView(S().convo);
            return !!(S().open && view && !(S().convo.canvas && S().convo.canvas.hidden && !view.streaming));
        },
        showCanvas() {
            const canvas = S().convo.canvas;
            if (canvas) canvas.hidden = false;
            assist.loadCanvas();
        },
        hideCanvas() { const canvas = S().convo.canvas; if (canvas) canvas.hidden = true; },
        // the dialect's renderer, loaded on the first canvas: the OpenUI Lang core, then the canvas library
        loadCanvas() {
            if (assist.canvasLoad) return assist.canvasLoad;
            S().canvasLoaded = false;
            assist.canvasLoad = moqui.loadScript("/libs/openui/lang-core.min.js", null, () => !!root.OpenUILang)
                .then(() => moqui.loadScript("/js/WebrootAssistCanvas.js", null, () => !!(assist.canvas && assist.canvas.component)))
                .then(() => { S().canvasLoaded = true; S().canvasError = ""; }, (err) => {
                    assist.canvasLoad = null;
                    S().canvasError = assist.labels.canvasFailed;
                    console.warn("the Assist canvas did not load", err);
                });
            return assist.canvasLoad;
        },
        // a canvas call with the canvas's id and version
        canvasCall(name, params) {
            const canvas = S().convo.canvas;
            if (!canvas) return Promise.reject({ status: 0, errors: "no canvas" });
            return assist.call(name, Object.assign({ artifactId: canvas.artifactId, version: canvas.version }, params), "POST");
        },
        // A turn a canvas makes (a Button's values, Adjust): shown as the user's message, answered as a turn.
        sendCanvasTurn(name, params, shown) {
            const convo = S().convo;
            const clientMessageId = assist.newId("m-");
            assist.addOutgoing(convo, clientMessageId, shown);
            const item = convo.items.find((other) => !other.messageId && other.clientMessageId === clientMessageId);
            if (item) item.local = "sending";
            return assist.canvasCall(name, Object.assign({ clientMessageId, clientId: assist.clientId, contextVersion: convo.contextVersion }, params)).then((json) => {
                assist.turnAnswered(convo, clientMessageId, json, assist.ctx());
                if (json.queued || json.existing) assist.refresh();
                assist.ensureStream();
                return json;
            }, (err) => { assist.turnFailed(convo, clientMessageId, err.errors); S().error = err.errors; return null; });
        },
        // what a canvas asks: a Button's values to the agent, a Mutation the click runs, its state, its problems
        canvasAction(action) {
            if (!action) return null;
            if (action.kind === "arm") {
                // one click: the nonces its Mutations run with, each once (the server keeps them in the session)
                return assist.canvasCall("canvasAction", { kind: "arm", count: action.count || 1, activation: true })
                    .then((json) => { if (action.done) action.done(json.nonces || []); return json; }, (err) => { if (action.failed) action.failed(err.errors); return null; });
            }
            if (action.kind === "mutation") {
                return assist.canvasCall("canvasAction", { kind: "mutation", statement: action.statement, nonce: action.nonce, state: action.state || {},
                    activation: !!action.activation })
                    .then((json) => { if (action.done) action.done(json); return json; }, (err) => { if (action.failed) action.failed(err.errors); return null; });
            }
            const text = action.message || "Submit";
            return assist.sendCanvasTurn("canvasAction", { kind: "continue", message: text, values: action.values || {}, form: action.form || null }, text);
        },
        canvasAdjust(text, notices) {
            const value = String(text || "").trim();
            if (!value) return Promise.resolve(null);
            return assist.sendCanvasTurn("canvasAdjust", { text: value, notices: (notices || []).map((one) => one.problem || one.message || String(one)) }, value);
        },
        canvasState(values) {
            if (assist.canvasStateTimer) clearTimeout(assist.canvasStateTimer);
            assist.canvasStateTimer = setTimeout(() => {
                assist.canvasStateTimer = null;
                assist.canvasCall("canvasState", { state: values || {} }).catch(() => {});
            }, 1000);
        },
        closeCanvas() {
            const canvas = S().convo.canvas;
            if (!canvas) return;
            canvas.closed = true;
            assist.canvasCall("canvasState", { closed: true }).catch(() => {});
        },
        // the page's problems once a render settles: the server maps them to the skill's sections, keeps the new ones and
        // may ask the agent to fix them (a turn)
        canvasProblems(problems) {
            const canvas = S().convo.canvas;
            if (!canvas || !problems || !problems.length) return;
            canvas.reported = problems;
            assist.canvasCall("canvasProblems", { problems, clientId: assist.clientId }).then((json) => {
                if (S().convo.canvas && S().convo.canvas.artifactId === canvas.artifactId) S().convo.canvas.problems = json.problems || [];
                if (json.revalidation) { assist.refresh(); assist.ensureStream(); }
            }, () => {});
        },

        // ---- sending ----

        // The composer's text: a / command, else a turn (queued on the server while the run works).
        send(text) {
            const value = String(text || "").trim();
            if (!value) return Promise.resolve(null);
            const command = assist.parseCommand(value, { admin: assist.isAdmin() });
            if (command) return Promise.resolve(assist.runCommand(command));
            const limits = assist.limits();
            if (value.length > limits.maxMessageChars) { S().error = "At most " + limits.maxMessageChars + " characters"; return Promise.resolve(null); }
            const clientMessageId = assist.newId("m-");
            assist.addOutgoing(S().convo, clientMessageId, value);
            return assist.sendTurn(clientMessageId, value);
        },
        sendTurn(clientMessageId, text) {
            const convo = S().convo;
            S().error = "";
            const params = { text, clientMessageId, clientId: assist.clientId, conversationId: convo.conversationId, contextVersion: convo.contextVersion,
                state: assist.currentPageState() };
            if (!convo.conversationId) params.agentId = S().agentId;
            if (S().tier) params.tier = S().tier;
            const item = convo.items.find((other) => !other.messageId && other.clientMessageId === clientMessageId);
            if (item) item.local = "sending";
            return assist.call("turn", params, "POST").then((json) => {
                assist.turnAnswered(convo, clientMessageId, json, assist.ctx());
                keep("sessionStorage", "moqui.assist.conversation", convo.conversationId);
                // a queued message's input id, and an earlier send's run, come from the conversation
                if (json.queued || json.existing) assist.refresh();
                assist.ensureStream();
                return json;
            }, (err) => {
                assist.turnFailed(convo, clientMessageId, err.errors);
                S().error = err.errors;
                return null;
            });
        },
        // a message not sent: the same send again, with its clientMessageId (the server answers a repeat as the first)
        resend(clientMessageId) {
            const item = S().convo.items.find((other) => !other.messageId && other.clientMessageId === clientMessageId);
            if (!item) return Promise.resolve(null);
            return assist.sendTurn(clientMessageId, item.parts.map((part) => part.text).join("\n"));
        },
        dropLocal(clientMessageId) {
            const convo = S().convo;
            const index = convo.items.findIndex((other) => !other.messageId && other.clientMessageId === clientMessageId);
            if (index >= 0) convo.items.splice(index, 1);
        },
        runCommand(command) {
            const convo = S().convo;
            const note = (text) => convo.notes.push({ key: "note:" + assist.newId(), text });
            if (command.error) { note("/" + command.name + ": " + command.error); return null; }
            const go = (path) => { assist.vm.setUrl(path); };
            switch (command.name) {
                case "new": assist.newConversation(); break;
                case "tier": S().tier = command.arg; note(assist.labels.tier + ": " + command.arg); break;
                case "help": note(assist.labels.help + (assist.isAdmin() ? " " + assist.labels.adminHelp : "")); break;
                case "skill": {
                    // an @name loads the skill on the server, after the message (a synthetic skill call)
                    const parts = command.arg.split(/\s+/);
                    const text = "@" + parts[0].replace(/^@/, "") + (parts.length > 1 ? " " + parts.slice(1).join(" ") : "");
                    const clientMessageId = assist.newId("m-");
                    assist.addOutgoing(convo, clientMessageId, text);
                    return assist.sendTurn(clientMessageId, text);
                }
                case "prompt": go("/qapps/system/Llm/PromptPreview"); break;
                case "tools": go("/qapps/system/Llm/Tools"); break;
                case "usage": go("/qapps/system/Llm/Usage"); break;
                case "context": note(JSON.stringify(assist.currentPageState(), null, 2)); break;
                case "skills":
                    assist.call("mentions", { kind: "skills", q: command.arg }).then((json) => note((json.items || []).map((item) => item.name + (item.description ? " — " + item.description : "")).join("\n") || "-"));
                    break;
                default: break;
            }
            return null;
        },
        stop() {
            const runId = S().convo.activeRunId;
            if (!runId) return Promise.resolve(null);
            return assist.call("cancel", { runId }, "POST").then((json) => {
                const run = assist.runOf(S().convo, runId);
                if (json.state) { run.state = json.state; run.statusId = json.statusId; }
                if (assist.isEnded(run.state)) { S().convo.activeRunId = null; assist.endStreaming(S().convo, runId); }
                return json;
            }, (err) => { S().error = err.errors; });
        },
        retry() {
            const convo = S().convo;
            if (!convo.conversationId) return Promise.resolve(null);
            const params = { conversationId: convo.conversationId, clientMessageId: assist.newId("m-"), clientId: assist.clientId, contextVersion: convo.contextVersion,
                state: assist.currentPageState() };
            if (S().tier) params.tier = S().tier;
            return assist.call("retry", params, "POST").then((json) => {
                assist.turnAnswered(convo, params.clientMessageId, json, assist.ctx());
                assist.refresh();
                return json;
            }, (err) => { S().error = err.errors; });
        },
        feedback(messageId, rating, note) {
            return assist.call("feedback", { messageId, rating: rating || "", note: note || "" }, "POST").then((json) => {
                const item = S().convo.items.find((other) => other.messageId === messageId);
                if (item) item.feedback = json.feedback || null;
            }, (err) => { S().error = err.errors; });
        },
        // "Follow here": this tab applies the agent's page actions; release gives them up
        followHere(release) {
            const convo = S().convo;
            if (!convo.conversationId) return Promise.resolve(null);
            return assist.call("follow", { conversationId: convo.conversationId, clientId: assist.clientId, contextVersion: convo.contextVersion, release: release ? true : null }, "POST")
                .then((json) => {
                    if (json.follow) {
                        convo.follow = { clientId: json.follow.clientId || null, contextVersion: Number(json.follow.contextVersion) || 0, following: !!json.following };
                        convo.contextVersion = Math.max(convo.contextVersion, convo.follow.contextVersion);
                    }
                    return json;
                }, (err) => { S().error = err.errors; });
        },

        // ---- cards ----

        busy(key, promise) {
            S().busy = Object.assign({}, S().busy, { [key]: true });
            const done = () => { const next = Object.assign({}, S().busy); delete next[key]; S().busy = next; };
            promise.then(done, done);
            return promise;
        },
        // a decision: approve (with edited arguments), reject with feedback, or edit (Edit in screen: this tab follows the hand-off)
        decide(card, decision, extra) {
            const params = Object.assign({ approvalId: card.approvalId, decision }, extra);
            if (decision === "edit") { params.clientId = assist.clientId; params.contextVersion = S().convo.contextVersion; }
            return assist.busy("approval:" + card.approvalId, assist.call("decide", params, "POST").then((json) => {
                const own = S().convo.approvals[card.approvalId];
                if (own) Object.assign(own, { status: json.status || own.status, handoffId: json.handoffId || own.handoffId || null });
                if (decision === "edit") {
                    // the hand-off's page events carry the owner and version the server took: this tab's
                    assist.takeFollow(S().convo, json.follow || { clientId: assist.clientId, contextVersion: S().convo.contextVersion });
                    if (!S().followOn) assist.setFollowOn(true);
                }
                assist.resume();
                return json;
            }, (err) => {
                S().error = err.errors;
                if (err.status === 409 || err.status === 404) assist.refresh();
            }));
        },
        preview(card) {
            return assist.busy("preview:" + card.approvalId, assist.call("preview", { approvalId: card.approvalId }, "POST").then((json) => {
                const own = S().convo.approvals[card.approvalId];
                if (own) own.preview = json.preview || null;
            }, (err) => { S().error = err.errors; }));
        },
        // a question's answer: {choices | choice | values | text}
        answer(card, answer) {
            const params = Object.assign({ runId: card.runId || S().convo.activeRunId, questionId: card.questionId }, answer);
            return assist.busy("question:" + card.questionId, assist.call("answer", params, "POST").then((json) => {
                const own = S().convo.questions[card.questionId];
                if (own) own.status = "answered";
                assist.resume();
                return json;
            }, (err) => {
                S().error = err.errors;
                if (err.status === 409) assist.refresh();
            }));
        },
        // a hand-off's Skip, or the user leaving its page: the run goes on with that
        abandonHandoff(handoff, reason) {
            if (!assist.watching(handoff)) return Promise.resolve(null);
            handoff.status = "abandoning";
            return assist.call("submitted", { handoffId: handoff.handoffId, abandoned: reason || "skipped" }, "POST").then((json) => {
                handoff.status = json.status || "abandoned";
                assist.resume();
            }, (err) => { handoff.status = "watching"; S().error = err.errors; });
        },
        // a hand-off still waits for the user's save (a refused save keeps it watching)
        watching(handoff) { return !!handoff && (handoff.status === "watching" || handoff.status === "failed"); },
        // after the user acted on a paused run: its stream again from where it stopped
        resume() { setTimeout(() => assist.ensureStream(true), 50); },

        // ---- the stream ----

        // the stream of the active run, while the tab is visible and the run is not waiting for the user (force: it may go on now)
        ensureStream(force) {
            const convo = S().convo;
            const runId = convo.activeRunId;
            const doc = root.document;
            if (!runId || (doc && doc.visibilityState === "hidden")) return;
            const run = convo.runs[runId];
            if (!force && run && assist.isPaused(run.state) && assist.stream === null) {
                // a paused run's events up to now (cards made while this tab was away)
                assist.catchUp(runId);
                return;
            }
            if (assist.stream && assist.stream.runId === runId) return;
            assist.openStream(runId);
        },
        openStream(runId) {
            assist.closeStream();
            if (!root.fetch || !root.TextDecoder) { assist.startPolling(runId); return; }
            const controller = root.AbortController ? new root.AbortController() : null;
            const stream = { runId, controller, attempts: (assist.lastStream && assist.lastStream.runId === runId ? assist.lastStream.attempts : 0) };
            assist.stream = stream;
            assist.lastStream = stream;
            S().streaming = true;
            const after = Number(S().convo.seq[runId]) || 0;
            const url = assist.base() + assist.paths().events.replace("{runId}", encodeURIComponent(runId)) + "?after=" + after;
            root.fetch(url, { headers: { Accept: "text/event-stream", "Last-Event-ID": String(after) }, credentials: "same-origin", signal: controller ? controller.signal : undefined })
                .then((response) => {
                    if (!response.ok || !response.body) throw { status: response.status };
                    const reader = response.body.getReader();
                    const decoder = new root.TextDecoder();
                    let buffer = "";
                    const pump = () => reader.read().then((chunk) => {
                        if (chunk.done || assist.stream !== stream) return null;
                        buffer += decoder.decode(chunk.value, { stream: true });
                        const parsed = assist.parseSse(buffer);
                        buffer = parsed.rest;
                        // a stream that brings events is a good one: its next end starts the back-off over
                        if (parsed.events.length) stream.attempts = 0;
                        parsed.events.forEach((frame) => { if (assist.stream === stream) assist.dispatch(runId, assist.frameEvent(frame)); });
                        return pump();
                    });
                    return pump();
                })
                .then(() => assist.streamEnded(stream, null), (err) => assist.streamEnded(stream, err));
        },
        closeStream() {
            const stream = assist.stream;
            assist.stream = null;
            S().streaming = false;
            if (stream && stream.controller) { try { stream.controller.abort(); } catch (err) { /* closed */ } }
        },
        // The stream ended: after a pause or the run's end nothing; else again from the last sequence, and after a refusal
        // (503 when the node streams too many, or no stream at all) polling events.json each second.
        streamEnded(stream, err) {
            if (assist.stream !== stream) return;
            assist.stream = null;
            S().streaming = false;
            const convo = S().convo;
            const run = convo.runs[stream.runId];
            if (convo.activeRunId !== stream.runId || !run || assist.isPaused(run.state) || assist.isEnded(run.state)) return;
            stream.attempts += 1;
            if ((err && (err.status === 503 || err.status === 0 || err.status === undefined)) || stream.attempts > 3) assist.startPolling(stream.runId);
            else if (err && err.status === 404) assist.refresh();
            else setTimeout(() => { if (!assist.stream && S().convo.activeRunId === stream.runId) assist.openStream(stream.runId); }, assist.backoff(stream.attempts));
        },
        // the wait before the attempts-th try again: 500 ms doubling, at most 15 s
        backoff(attempts) { return Math.min(15000, 500 * Math.pow(2, Math.max(0, (Number(attempts) || 1) - 1))); },
        // events.json while the run works and the stream cannot be had: each second while events come, slower (to 5 s) while
        // none do; it stops when the run ends or waits for the user (the catch-up's state says so) or the tab is hidden
        startPolling(runId) {
            if (assist.pollTimer) return;
            S().polling = true;
            let wait = 1000;
            const stop = () => { clearTimeout(assist.pollTimer); assist.pollTimer = null; S().polling = false; };
            const tick = () => {
                const convo = S().convo;
                const run = convo.runs[runId];
                const doc = root.document;
                if (convo.activeRunId !== runId || !run || assist.isEnded(run.state) || assist.isPaused(run.state) || (doc && doc.visibilityState === "hidden")) { stop(); return; }
                const before = Number(convo.seq[runId]) || 0;
                assist.catchUp(runId).then(() => {
                    if (!assist.pollTimer) return;
                    wait = (Number(S().convo.seq[runId]) || 0) > before ? 1000 : Math.min(5000, wait * 2);
                    assist.pollTimer = setTimeout(tick, wait);
                });
            };
            assist.pollTimer = setTimeout(tick, 0);
        },
        // the stored events after the run's last sequence (events.json), in order; again while more wait
        catchUp(runId) {
            if (assist.catching) { assist.catchAgain = true; return Promise.resolve(); }
            assist.catching = true;
            const after = Number(S().convo.seq[runId]) || 0;
            const url = assist.base() + assist.paths().catchUp.replace("{runId}", encodeURIComponent(runId));
            return moqui.request(url, { params: { after: String(after) } }).then((result) => {
                const json = result.json || {};
                (json.events || []).forEach((event) => assist.dispatch(runId, event, true));
                assist.catching = false;
                // the run's state as the server has it: an end or a pause the events did not bring (polling stops on it)
                const run = S().convo.runs[runId];
                if (run && json.state && run.state !== json.state && (assist.isEnded(json.state) || assist.isPaused(json.state))) {
                    run.state = json.state;
                    run.statusId = json.statusId || run.statusId;
                    if (assist.isEnded(json.state)) { if (S().convo.activeRunId === runId) S().convo.activeRunId = null; assist.endStreaming(S().convo, runId); assist.refresh(); }
                    S().convo.status = assist.statusOf(S().convo);
                }
                const more = (json.events || []).length >= 1000 || assist.catchAgain;
                assist.catchAgain = false;
                if (more && Number(S().convo.seq[runId]) > after) return assist.catchUp(runId);
                return null;
            }, (err) => {
                assist.catching = false;
                assist.catchAgain = false;
                if (err && err.status === 404) assist.refresh();
            });
        },
        // one event into the store, and its effects
        dispatch(runId, event, fromCatchUp) {
            if (!event) return;
            const effects = assist.apply(S().convo, runId, event, assist.ctx());
            effects.forEach((effect) => {
                if (effect.kind === "gap") { if (!fromCatchUp) assist.catchUp(effect.runId); }
                else if (effect.kind === "follow") assist.enqueueFollow(effect.runId, effect.event);
                else if (effect.kind === "refresh") assist.refresh();
                else if (effect.kind === "ended") { assist.refreshTasks(); assist.refresh(); }
                else if (effect.kind === "paused") assist.refreshTasks();
                else if (effect.kind === "canvas") assist.showCanvas();
            });
            if (event.type === "approval.requested" || event.type === "question.asked") assist.refreshTasks();
            if (event.type === "handoff.done" && event.payload) assist.clearSave(event.payload.handoffId);
        },
        // A run signal of the notification socket (topic AgentRun, to every tab of the user): the tasks badge again, and the
        // conversation of this tab again when it is about it and this tab does not stream it.
        onSignal(message) {
            let payload = message && message.message !== undefined ? message.message : message;
            if (typeof payload === "string") { try { payload = JSON.parse(payload); } catch (err) { payload = {}; } }
            payload = payload || {};
            assist.refreshTasks();
            const convo = S().convo;
            if (!convo.conversationId) return;
            const ours = (payload.conversationId && String(payload.conversationId) === convo.conversationId) || (payload.runId && convo.runs[String(payload.runId)]);
            if (ours && !(assist.stream && assist.stream.runId === String(payload.runId))) assist.refresh();
        },
        onVisibility() {
            const doc = root.document;
            if (doc && doc.visibilityState === "hidden") { assist.closeStream(); return; }
            if (S().convo.activeRunId) assist.catchUp(S().convo.activeRunId).then(() => assist.ensureStream());
            assist.refreshTasks();
        },
        refreshTasks() {
            clearTimeout(assist.tasksTimer);
            assist.tasksTimer = setTimeout(() => {
                if (!S().available) return;
                assist.call("tasks").then((json) => {
                    S().tasks = json;
                    S().taskCount = Number(json.counts && json.counts.waiting) || 0;
                }, () => {});
            }, 300);
        },
        loadHistory() {
            const history = S().history;
            history.loading = true;
            return assist.call("conversations", { search: history.search || null, pageSize: 50 }).then((json) => {
                history.list = json.conversations || [];
                history.hasMore = !!json.hasMore;
                history.loading = false;
            }, (err) => { history.loading = false; S().error = err.errors; });
        },

        // ---- the page ----

        // the page state of now (docs/Assist.md "The page state")
        currentPageState() {
            const vm = assist.vm;
            if (!vm) return {};
            const forms = assist.gatherForms();
            return assist.pageState(vm.screenContext, { title: vm.ScreenTitle, forms: assist.follow.unsaved(forms, assist.secretFields(), assist.limits()),
                dialogs: assist.openDialogs() });
        },
        // The registered forms of the page and its open dialogs, in DOM order, as follow.locate and follow.unsaved read them.
        gatherForms() {
            const vm = assist.vm;
            const doc = root.document;
            if (!vm || !doc) return [];
            const reach = (vm.assistReach || []).map((id) => doc.getElementById(id)).filter((el) => el);
            const out = [];
            (moqui.screenForms || []).forEach((form) => {
                const el = form.$refs && form.$refs.qForm ? form.$refs.qForm.$el : form.$el;
                if (!el || !el.isConnected || !el.querySelectorAll) return;
                const type = form.$options && form.$options.name ? form.$options.name : "";
                if (type === "mCheckboxSet") return;
                const attrs = form.$attrs || {};
                const name = String(attrs.name || form.name || attrs.id || el.id || "");
                if (!name) return;
                const dialogEl = el.closest(".q-dialog");
                const types = {};
                el.querySelectorAll("input[name], textarea[name], select[name]").forEach((input) => {
                    const fieldName = input.getAttribute("name");
                    if (fieldName && !types[fieldName]) types[fieldName] = String(input.type || input.tagName || "").toLowerCase();
                });
                const fields = Object.keys(form.fields || {});
                Object.keys(types).forEach((fieldName) => { if (fields.indexOf(fieldName) < 0) fields.push(fieldName); });
                const inReach = !el.closest("#m-assist-drawer") && (reach.some((area) => area.contains(el)) ||
                    (!!dialogEl && !dialogEl.querySelector("#m-palette, #m-keys-help")));
                out.push({ name, id: el.id || String(attrs.id || name), dialog: dialogEl ? assist.dialogIdOf(dialogEl) : null, inReach, fields, types,
                    multi: !!(form.fields && form.fields._isMulti === "true"), kind: type === "mFormLink" ? "link" : type === "mFormList" ? "list" : "form",
                    values: form.fields || {}, original: form.fieldsOriginal || {}, action: form.action || attrs.action || "", vm: form, el });
            });
            out.sort((left, right) => (left.el === right.el ? 0 : (left.el.compareDocumentPosition(right.el) & 4 ? -1 : 1)));
            return out;
        },
        dialogIdOf(dialogEl) {
            if (dialogEl.id) return dialogEl.id;
            const inner = dialogEl.querySelector("[id]");
            return inner ? inner.id : "";
        },
        openDialogs() {
            const doc = root.document;
            if (!doc) return [];
            return Array.from(doc.querySelectorAll(".q-dialog")).filter((el) => !el.querySelector("#m-palette, #m-keys-help, #m-assist-drawer"))
                .map((el) => assist.dialogIdOf(el)).filter((id) => !!id);
        },

        // The user's own navigation (the URL listener; an agent's ui.navigate sets agentNav): the page version goes up at once,
        // so page actions minted before the server hears of it are dropped, and context reports the page once it has loaded.
        // A hand-off whose page the user leaves is abandoned.
        onUrl(url) {
            if (assist.agentNav) return;
            // a navigation the page makes by itself just after the agent's (a redirect), with no input of the user's between
            if (assist.agentNavAt && Date.now() - assist.agentNavAt < 1500 && assist.lastInputAt < assist.agentNavAt) return;
            const convo = S().convo;
            if (!convo.conversationId) return;
            assist.nextContextVersion(convo);
            assist.followQueue = [];
            const path = String(url || "").split("?")[0];
            Object.keys(convo.handoffs).forEach((key) => {
                const handoff = convo.handoffs[key];
                if (!assist.watching(handoff) || handoff.clientId !== assist.clientId) return;
                const handoffPath = assist.safePath(handoff.path, null);
                const samePage = handoffPath.ok && handoffPath.url.split("?")[0] === path;
                if (!samePage && Date.now() - assist.lastSaveOk > 3000) assist.abandonHandoff(handoff, "navigated");
            });
            if (convo.follow.clientId && convo.follow.clientId !== assist.clientId) return;
            clearTimeout(assist.contextTimer);
            assist.settled().then(() => {
                assist.contextTimer = setTimeout(() => assist.postContext(), 200);
            });
        },
        postContext() {
            const convo = S().convo;
            if (!convo.conversationId) return Promise.resolve(null);
            return assist.call("context", { conversationId: convo.conversationId, clientId: assist.clientId, contextVersion: convo.contextVersion,
                state: assist.currentPageState() }, "POST").then((json) => {
                if (json.follow) assist.takeFollow(convo, json.follow);
                return json;
            }, () => null);
        },
        // the page has loaded: no load running and the last navigation a moment ago (at most 8 s)
        settled() {
            const vm = assist.vm;
            const started = Date.now();
            return new Promise((resolve) => {
                const check = () => {
                    const quiet = vm && !vm.loading && !vm.currentLoadRequest && Date.now() - (vm.lastNavTime || 0) > 250;
                    if (quiet || Date.now() - started > 8000) resolve(); else setTimeout(check, 50);
                };
                check();
            });
        },
        enqueueFollow(runId, event) {
            assist.followQueue.push({ runId, event });
            assist.runFollow();
        },
        runFollow() {
            if (assist.followBusy) return;
            const next = assist.followQueue.shift();
            if (!next) return;
            assist.followBusy = true;
            assist.settled().then(() => {
                // the user may have moved meanwhile: the event must still be this page's
                if (!assist.follow.applies(S().convo, next.runId, next.event, assist.ctx())) return null;
                return assist.applyFollow(next.event);
            }).catch((err) => console.warn("Assist could not follow " + next.event.type, err)).then(() => {
                assist.followBusy = false;
                assist.runFollow();
            });
        },
        // one page action of the agent, on this page
        applyFollow(event) {
            const vm = assist.vm;
            const p = event.payload || {};
            const labels = labelsOf(vm);
            if (event.type === "ui.navigate") {
                const checked = assist.safePath(p.path, p.parameters, { linkBase: vm.linkBasePath || "/qapps" });
                if (!checked.ok) { console.warn("Assist navigation refused: " + checked.error, p.path); return null; }
                assist.agentNav = true;
                assist.agentNavAt = Date.now();
                try { vm.setUrl(checked.url); } finally { assist.agentNav = false; }
                // under a canvas the page moved out of sight: a chip says where ("Assist opened Orders [Show]")
                if (assist.canvasShown()) assist.addChip(S().convo, event);
                return null;
            }
            if (event.type === "ui.dialog") {
                const container = vm.activeContainers ? vm.activeContainers[p.id] : null;
                if (container) { if (p.open && container.show) container.show(); else if (!p.open && container.hide) container.hide(); }
                return null;
            }
            if (event.type === "handoff.started") return null;
            // a fill, focus or highlight: the forms may still be mounting after a navigation
            return assist.locateSoon(p).then((found) => {
                if (!found.ok) { if (found.reason !== "none") console.info("Assist did not " + event.type + " " + (p.field || "") + ": " + found.reason); return; }
                const target = found.form;
                if (event.type === "ui.fill" && found.field) {
                    target.vm.fields[found.field] = p.value === null || p.value === undefined ? "" : String(p.value);
                    S().announce = labels.filled + ": " + (p.label || p.field);
                }
                assist.markField(target.el, found.field, event.type);
                const handoff = p.handoffId ? S().convo.handoffs[p.handoffId] : null;
                if (handoff) assist.markSave(handoff);
            });
        },
        locateSoon(target) {
            const started = Date.now();
            return new Promise((resolve) => {
                const tryIt = () => {
                    const found = assist.follow.locate(target, assist.gatherForms(), assist.secretFields());
                    if (found.ok || found.reason !== "none" || Date.now() - started > 3000) resolve(found); else setTimeout(tryIt, 100);
                };
                tryIt();
            });
        },
        // the field outlined and tagged "Assist" until it is focused or for 8 s (fill), focused (focus) or scrolled to (highlight)
        markField(formEl, field, kind) {
            const input = field ? formEl.querySelector('[name="' + String(field).replace(/["\\]/g, "") + '"]') : null;
            const box = input ? (input.closest(".q-field") || input) : formEl;
            if (box.scrollIntoView) box.scrollIntoView({ block: "nearest", behavior: "smooth" });
            if (kind === "ui.focus" && input && input.focus) { input.focus(); return; }
            // marked by attribute: the field's component sets its class again on each update, an attribute of ours stays
            const mark = kind === "ui.highlight" ? "data-assist-highlight" : "data-assist-touched";
            box.setAttribute(mark, field || "");
            const clear = () => {
                box.removeAttribute(mark);
                if (input) input.removeEventListener("focus", clear);
            };
            if (input) input.addEventListener("focus", clear);
            setTimeout(clear, TAG_FOR);
        },
        // the hand-off's form: its Save ringed (data-assist-save)
        markSave(handoff) {
            const forms = assist.gatherForms().filter((form) => assist.handoffMatches(handoff, form));
            forms.forEach((form) => form.el.querySelectorAll('button[type="submit"], input[type="submit"]').forEach((button) => {
                const target = button.closest(".q-btn") || button;
                target.setAttribute("data-assist-save", handoff.handoffId);
            }));
        },
        clearSave(handoffId) {
            const doc = root.document;
            if (!doc) return;
            doc.querySelectorAll('[data-assist-save="' + String(handoffId).replace(/["\\]/g, "") + '"]').forEach((el) => el.removeAttribute("data-assist-save"));
        },
        // An answered form (moqui.onFormAnswer): the user's own save of a hand-off of this tab is reported (submitted, a signal:
        // the server observes the save itself) with what the page saw, never the form's values.
        onFormAnswer(answer) {
            const convo = S().convo;
            const form = answer && answer.form;
            if (!form) return;
            const attrs = form.$attrs || {};
            const el = form.$refs && form.$refs.qForm ? form.$refs.qForm.$el : form.$el;
            const info = { name: attrs.name || form.name || "", id: (el && el.id) || attrs.id || "", action: form.action || attrs.action || "" };
            Object.keys(convo.handoffs).forEach((key) => {
                const handoff = convo.handoffs[key];
                if (!assist.watching(handoff) || !assist.handoffMatches(handoff, info)) return;
                const json = answer.json || {};
                if (answer.ok) assist.lastSaveOk = Date.now();
                // a signal only (the server observes the save itself): what the page saw, no form values
                assist.call("submitted", { handoffId: handoff.handoffId, ok: !!answer.ok, messageInfos: json.messageInfos || null, errors: json.errors || null,
                    validationErrors: json.validationErrors || null, screenUrl: json.screenUrl || null }, "POST").then((result) => {
                    if (result.status) handoff.status = result.status;
                    if (!assist.watching(handoff)) assist.clearSave(handoff.handoffId);
                    handoff.lastOk = !!answer.ok;
                    assist.resume();
                }, (err) => { S().error = err.errors; });
            });
        }
    });

    // ---- the components (registered by WebrootVue.qvt.js, or late) ----

    const components = {};
    const st = () => assist.state;

    // The header button: opens the drawer; the badge is what waits for the user (tasks.counts.waiting).
    components["m-assist-button"] = {
        name: "mAssistButton",
        computed: { labels() { return labelsOf(this); }, count() { return st().taskCount; }, open() { return st().open; } },
        methods: { toggle() { assist.toggle(false); } },
        template:
            '<q-btn flat round dense id="m-assist-button" icon="sym_o_auto_awesome" :class="{\'text-primary\': open}" :aria-label="labels.assist" :aria-expanded="open ? \'true\' : \'false\'" @click="toggle">' +
                '<q-badge v-if="count" floating rounded color="negative" data-assist-badge>{{count > 99 ? \'99+\' : count}}</q-badge>' +
                '<q-tooltip>{{labels.assist}} ({{$root.paletteKeyHint ? $root.paletteKeyHint.replace(\'K\', \'J\') : \'Ctrl J\'}})</q-tooltip>' +
            '</q-btn>'
    };

    // The drawer: one right q-drawer in the shell's layout, mounted on first open and kept, persistent (an overlay q-drawer would hide on every navigation and on Esc; the drawer closes by its own Close, Mod+J and a second Esc). Docked 420 px (320–720 by the handle,
    // kept as ASSIST_WIDTH), the full width below 600 px; it pushes the page from 1440 px and overlays it below.
    components["m-shell-assist"] = {
        name: "mShellAssist",
        data() { return { escArmed: false }; },
        computed: {
            labels() { return labelsOf(this); },
            s() { return st(); },
            convo() { return st().convo; },
            open: { get() { return st().open; }, set(value) { if (value) assist.openDrawer(); else assist.closeDrawer(); } },
            phone() { return this.$q.screen.width < 600; },
            width() { return this.phone ? this.$q.screen.width : st().width; },
            // a canvas takes the page beside the drawer, so the drawer pushes it rather than covering its right side
            overlay() { return this.$q.screen.width < 1440 && !(this.$q.screen.width >= 600 && assist.canvasShown()); },
            boot() { return st().bootstrap || {}; },
            agents() { return this.boot.agents || []; },
            agent() { return this.agents.find((agent) => agent.agentId === (this.convo.agentId || st().agentId)) || this.agents[0] || {}; },
            tiers() { return this.agent.tierFixed ? [] : (this.agent.tiers && this.agent.tiers.length ? this.agent.tiers : this.boot.tiers || []); },
            tierOptions() { return this.tiers.map((tier) => ({ label: tier.charAt(0).toUpperCase() + tier.slice(1), value: tier })); },
            agentOptions() { return this.agents.map((agent) => ({ label: agent.name || agent.agentId, value: agent.agentId })); },
            paused() { const run = this.convo.activeRunId ? this.convo.runs[this.convo.activeRunId] : null; return !!run && assist.isPaused(run.state); },
            elsewhere() { const follow = this.convo.follow; return !!(this.convo.conversationId && follow.clientId && follow.clientId !== assist.clientId); },
            approvals() { return Object.keys(this.convo.approvals).map((key) => this.convo.approvals[key]); },
            questions() { return Object.keys(this.convo.questions).map((key) => this.convo.questions[key]).filter((card) => card.status === "open" || card.local); },
            handoffs() { return Object.keys(this.convo.handoffs).map((key) => this.convo.handoffs[key]).filter((card) => card.status === "watching" || card.status === "failed" || card.status === "abandoning"); },
            canvasHidden() { return !!(this.convo.canvas && !this.convo.canvas.closed && this.convo.canvas.hidden); }
        },
        methods: {
            showCanvas() { assist.showCanvas(); },
            setTier(tier) { st().tier = tier; },
            setAgent(agentId) { if (agentId !== st().agentId) { st().agentId = agentId; assist.newConversation(); } },
            newConversation() { assist.newConversation(); },
            pane(name) { st().pane = st().pane === name ? "chat" : name; if (st().pane === "history") assist.loadHistory(); if (st().pane === "tasks") assist.refreshTasks(); },
            followHere() { assist.followHere(); },
            toggleFollow(on) { assist.setFollowOn(on); },
            close() { assist.closeDrawer(); },
            showChip(chip) {
                if (chip.kind !== "navigate") return;
                assist.hideCanvas();
                const checked = assist.safePath(chip.path, chip.parameters, { linkBase: this.$root.linkBasePath || "/qapps" });
                if (checked.ok) this.$root.setUrl(checked.url);
            },
            // Esc returns focus to the page; a second Esc closes
            onKey(event) {
                if (event.key !== "Escape" || event.defaultPrevented) return;
                if (root.document.querySelector(".q-menu")) return;
                event.stopPropagation();
                if (this.escArmed) { this.escArmed = false; assist.closeDrawer(); return; }
                this.escArmed = true;
                const active = root.document.activeElement;
                if (active && active.blur) active.blur();
                const canvas = root.document.getElementById("m-canvas");
                if (canvas && canvas.focus) { canvas.setAttribute("tabindex", "-1"); canvas.focus(); }
                setTimeout(() => { this.escArmed = false; }, 1500);
            },
            startResize(event) {
                event.preventDefault();
                const startX = event.clientX;
                const startWidth = st().width;
                const move = (e) => assist.setWidth(startWidth + (startX - e.clientX));
                const up = () => { root.document.removeEventListener("mousemove", move); root.document.removeEventListener("mouseup", up); };
                root.document.addEventListener("mousemove", move);
                root.document.addEventListener("mouseup", up);
            }
        },
        template:
            '<q-drawer v-model="open" side="right" bordered persistent :width="width" :breakpoint="0" :overlay="overlay" class="m-assist" id="m-assist-drawer" data-assist-drawer>' +
                '<div v-if="s.mounted" class="m-assist-body column no-wrap fit" @keydown="onKey">' +
                    '<div v-if="!phone" class="m-assist-resize" @mousedown="startResize" aria-hidden="true"></div>' +
                    '<div class="row items-center no-wrap q-px-sm q-py-xs m-assist-header">' +
                        '<q-icon name="sym_o_auto_awesome" size="sm" class="q-mr-xs text-primary"></q-icon>' +
                        '<q-select v-if="agentOptions.length > 1" :model-value="convo.agentId || s.agentId" @update:model-value="setAgent" :options="agentOptions" emit-value map-options dense borderless options-dense class="m-assist-agent" for="m-assist-agent" :aria-label="labels.agent"></q-select>' +
                        '<div v-else class="text-subtitle2 text-weight-medium ellipsis">{{agent.name || labels.assist}}</div>' +
                        '<q-space></q-space>' +
                        '<q-btn-toggle v-if="tierOptions.length > 1" :model-value="s.tier || tiers[0]" @update:model-value="setTier" :options="tierOptions" dense flat no-caps size="sm" toggle-color="primary" class="m-assist-tier" id="m-assist-tier"></q-btn-toggle>' +
                        '<q-btn flat round dense icon="sym_o_edit_square" id="m-assist-new" :aria-label="labels.newConversation" @click="newConversation"><q-tooltip>{{labels.newConversation}}</q-tooltip></q-btn>' +
                        '<q-btn flat round dense icon="sym_o_history" id="m-assist-history" :class="{\'text-primary\': s.pane === \'history\'}" :aria-label="labels.history" @click="pane(\'history\')"><q-tooltip>{{labels.history}}</q-tooltip></q-btn>' +
                        '<q-btn flat round dense icon="sym_o_checklist" id="m-assist-tasks" :class="{\'text-primary\': s.pane === \'tasks\'}" :aria-label="labels.tasks" @click="pane(\'tasks\')">' +
                            '<q-badge v-if="s.taskCount" floating rounded color="negative">{{s.taskCount}}</q-badge><q-tooltip>{{labels.tasks}}</q-tooltip></q-btn>' +
                        '<q-btn flat round dense icon="sym_o_more_vert" id="m-assist-menu" :aria-label="labels.menu"><q-menu anchor="bottom right" self="top right"><q-list dense style="min-width:220px;">' +
                            '<q-item tag="label" clickable id="m-assist-follow-toggle"><q-item-section>{{labels.followOn}}</q-item-section>' +
                                '<q-item-section side><q-toggle dense :model-value="s.followOn" @update:model-value="toggleFollow"></q-toggle></q-item-section></q-item>' +
                        '</q-list></q-menu></q-btn>' +
                        '<q-btn flat round dense icon="sym_o_close" id="m-assist-close" :aria-label="labels.close" @click="close"></q-btn>' +
                    '</div>' +
                    '<div v-if="canvasHidden" class="q-px-sm q-pb-xs"><q-btn flat dense no-caps size="sm" color="primary" icon="sym_o_dashboard" id="m-assist-show-canvas"' +
                        ' :label="labels.showCanvas + \': \' + convo.canvas.title" @click="showCanvas"></q-btn></div>' +
                    '<div v-if="convo.status || elsewhere || convo.chips.length" class="q-px-sm q-pb-xs m-assist-subheader">' +
                        '<div v-if="convo.status" class="row items-center no-wrap text-caption m-assist-status" id="m-assist-status"><q-spinner-dots v-if="!paused" size="14px" class="q-mr-xs"></q-spinner-dots><q-icon v-else name="sym_o_pending_actions" color="warning" size="xs" class="q-mr-xs"></q-icon>{{convo.status}}</div>' +
                        '<div v-if="elsewhere" class="row items-center no-wrap text-caption" id="m-assist-elsewhere" data-assist-follow="elsewhere">{{labels.followingElsewhere}}<q-space></q-space>' +
                            '<q-btn flat dense no-caps size="sm" color="primary" id="m-assist-follow-here" :label="labels.followHere" @click="followHere"></q-btn></div>' +
                        '<div v-if="convo.chips.length" class="row q-gutter-xs" id="m-assist-chips">' +
                            '<q-chip v-for="chip in convo.chips" :key="chip.key" dense square size="sm" :clickable="chip.kind === \'navigate\'" :data-assist-chip="chip.kind" @click="showChip(chip)">' +
                                '{{chip.kind === \'navigate\' ? labels.opened : labels.filled}} {{chip.label}}<span v-if="chip.kind === \'navigate\'" class="text-primary q-ml-xs">{{labels.show}}</span></q-chip>' +
                        '</div>' +
                    '</div>' +
                    '<q-separator></q-separator>' +
                    '<m-assist-history v-if="s.pane === \'history\'" class="col"></m-assist-history>' +
                    '<m-assist-tasks v-else-if="s.pane === \'tasks\'" class="col"></m-assist-tasks>' +
                    '<template v-else>' +
                        '<m-assist-thread class="col"></m-assist-thread>' +
                        '<div v-if="handoffs.length || approvals.length || questions.length" class="m-assist-cards q-px-sm q-pt-xs">' +
                            '<m-assist-card v-for="card in handoffs" :key="\'h\' + card.handoffId" kind="handoff" :card="card"></m-assist-card>' +
                            '<m-assist-card v-for="card in questions" :key="\'q\' + card.questionId" kind="question" :card="card"></m-assist-card>' +
                        '</div>' +
                        '<m-assist-composer></m-assist-composer>' +
                    '</template>' +
                    '<div class="m-assist-announce" aria-live="polite" role="status">{{s.announce}}</div>' +
                '</div>' +
            '</q-drawer>'
    };

    // The canvas over the page (plan 4.11: it takes the page while chat stays docked): its title and version, Back to the page
    // and Close, the dialect's component (js/WebrootAssistCanvas.js, loaded on the first canvas), the problems the page
    // reported and Adjust. The page under it stays mounted (the root shows it again on Back).
    components["m-assist-canvas"] = {
        name: "mAssistCanvas",
        data() { return { adjust: "" }; },
        computed: {
            labels() { return labelsOf(this); },
            s() { return st(); },
            shown() { return assist.canvasShown(); },
            view() { return assist.canvasView(st().convo) || {}; },
            canvas() { return st().convo.canvas || {}; },
            dialect() { const found = assist.dialects.get(this.view.kind); return found ? found.component : null; },
            problems() { return (this.canvas.problems || []).slice(0, 5); }
        },
        watch: { shown: { immediate: true, handler(value) { if (value) assist.loadCanvas(); } } },
        methods: {
            back() { assist.hideCanvas(); },
            close() { assist.closeCanvas(); },
            onProblems(list) { if (!this.view.streaming) assist.canvasProblems(list); },
            onState(values) { if (!this.view.streaming) assist.canvasState(values); },
            onAction(action) { assist.canvasAction(action); },
            onNavigate(url) { assist.hideCanvas(); if (this.$root.setUrl) this.$root.setUrl(url); },
            sendAdjust() { const text = this.adjust; this.adjust = ""; assist.canvasAdjust(text, this.canvas.problems || []); }
        },
        template:
            '<div v-if="shown" class="m-assist-canvas q-pa-md" id="m-assist-canvas" data-assist-canvas :data-version="view.version" :data-streaming="view.streaming ? \'true\' : undefined">' +
                '<div class="row items-center no-wrap q-mb-sm m-assist-canvas-bar">' +
                    '<q-icon name="sym_o_dashboard" size="sm" class="q-mr-sm text-primary"></q-icon>' +
                    '<div class="text-h6 ellipsis" id="m-assist-canvas-title">{{view.title || labels.canvas}}</div>' +
                    '<q-badge outline color="primary" class="q-ml-sm">v{{view.version}}</q-badge>' +
                    '<q-spinner-dots v-if="view.streaming || view.updating" size="18px" class="q-ml-sm" :aria-label="labels.canvasUpdating"></q-spinner-dots>' +
                    '<q-space></q-space>' +
                    '<q-btn flat dense no-caps icon="sym_o_arrow_back" :label="labels.backToPage" id="m-assist-canvas-back" @click="back"></q-btn>' +
                    '<q-btn flat round dense icon="sym_o_close" id="m-assist-canvas-close" :aria-label="labels.closeCanvas" @click="close"></q-btn>' +
                '</div>' +
                '<div v-if="!s.canvasLoaded || !dialect" class="q-pa-md m-muted">{{s.canvasError || labels.loading}}</div>' +
                '<component v-else :is="dialect" :program="view.program" :streaming="view.streaming" :artifact-id="view.artifactId" :version="view.version"' +
                    ' :initial-state="view.state" @problems="onProblems" @state="onState" @action="onAction" @navigate="onNavigate"></component>' +
                '<div v-if="problems.length" class="q-mt-md m-assist-canvas-problems" id="m-assist-canvas-problems">' +
                    '<div class="text-caption text-negative">{{labels.canvasProblems}}</div>' +
                    '<div v-for="(problem, index) in problems" :key="index" class="text-caption m-muted" data-assist-canvas-problem>{{problem.problem}} ({{problem.skill}})</div>' +
                '</div>' +
                '<div class="row items-center no-wrap q-mt-md q-gutter-sm m-assist-canvas-adjust">' +
                    '<q-input dense outlined v-model="adjust" :placeholder="labels.adjust" class="col" for="m-assist-canvas-adjust" @keydown.enter.prevent="sendAdjust"></q-input>' +
                    '<q-btn flat dense no-caps color="primary" :label="labels.adjustSend" id="m-assist-canvas-adjust-send" :disable="!adjust.trim()" @click="sendAdjust"></q-btn>' +
                '</div>' +
            '</div>'
    };

    // The thread: the rows of assist.threadRows, a log for screen readers (busy while a reply streams), the approval cards in
    // place of their step, and the empty state.
    components["m-assist-thread"] = {
        name: "mAssistThread",
        data() { return { openSteps: {}, pinned: true }; },
        computed: {
            labels() { return labelsOf(this); },
            convo() { return st().convo; },
            rows() { return assist.threadRows(this.convo); },
            streaming() { return !!this.convo.activeRunId; },
            // the approval cards by their tool call, the rest at the end
            approvalsByCall() {
                const out = {};
                Object.keys(this.convo.approvals).forEach((key) => { const card = this.convo.approvals[key]; if (card.toolCallId) out[card.toolCallId] = card; });
                return out;
            },
            loose() {
                const calls = {};
                this.rows.forEach((row) => { if (row.kind === "step") calls[row.toolCallId] = true; });
                return Object.keys(this.convo.approvals).map((key) => this.convo.approvals[key]).filter((card) => !card.toolCallId || !calls[card.toolCallId]);
            },
            scrollKey() { const last = this.rows[this.rows.length - 1]; return this.rows.length + ":" + (last && last.text ? last.text.length : 0) + ":" + this.loose.length; }
        },
        watch: { scrollKey() { this.$nextTick(() => { if (this.pinned && this.$el && this.$el.scrollTo) this.$el.scrollTop = this.$el.scrollHeight; }); } },
        methods: {
            onScroll() { const el = this.$el; this.pinned = el.scrollHeight - el.scrollTop - el.clientHeight < 40; },
            toggleStep(key) { this.openSteps = Object.assign({}, this.openSteps, { [key]: !this.openSteps[key] }); },
            stepIcon(status) { return status === "done" ? "sym_o_check_circle" : status === "failed" ? "sym_o_error" : status === "denied" ? "sym_o_block" : status === "running" ? "sym_o_pending" :
                status === "waiting" ? "sym_o_pending_actions" : "sym_o_help"; },
            resend(row) { assist.resend(row.clientMessageId); },
            drop(row) { assist.dropLocal(row.clientMessageId); },
            retry() { assist.retry(); },
            copy(text) { if (root.navigator && root.navigator.clipboard) root.navigator.clipboard.writeText(text); },
            rate(row, rating) { assist.feedback(row.messageId, row.feedback && row.feedback.rating === rating ? "" : rating); },
            doneText(row) { return assist.doneText(row); },
            earlier() { assist.loadEarlier(); }
        },
        template:
            '<div class="m-assist-thread scroll q-pa-sm" id="m-assist-thread" role="log" :aria-busy="streaming ? \'true\' : \'false\'" @scroll="onScroll">' +
                '<div v-if="convo.hasEarlier" class="text-center q-mb-sm"><q-btn flat dense no-caps size="sm" :label="labels.earlier" @click="earlier"></q-btn></div>' +
                '<div v-if="!rows.length && !loose.length" class="m-assist-empty text-center q-pa-lg m-muted" id="m-assist-empty">{{labels.empty}}</div>' +
                '<template v-for="row in rows" :key="row.key">' +
                    '<div v-if="row.kind === \'user\'" class="m-assist-user q-mb-sm" data-assist-item="user" :data-assist-local="row.local || undefined">' +
                        '<div class="m-assist-bubble"><template v-for="(segment, index) in row.segments" :key="index">' +
                            '<span v-if="segment.text !== undefined" class="m-assist-pre">{{segment.text}}</span>' +
                            '<q-chip v-else dense square size="sm" icon="sym_o_link" class="m-assist-record-chip">{{segment.chip.label}}</q-chip>' +
                        '</template></div>' +
                        '<div v-if="row.local === \'sending\'" class="text-caption m-muted text-right">{{labels.sending}}</div>' +
                        '<div v-else-if="row.local === \'failed\'" class="text-caption text-negative text-right">{{labels.notSent}}{{row.error ? \': \' + row.error : \'\'}}' +
                            '<q-btn flat dense no-caps size="sm" :label="labels.retry" data-assist-action="resend" @click="resend(row)"></q-btn><q-btn flat dense round size="sm" icon="sym_o_close" @click="drop(row)"></q-btn></div>' +
                    '</div>' +
                    '<div v-else-if="row.kind === \'text\'" class="m-assist-answer q-mb-sm" data-assist-item="assistant" :data-message-id="row.messageId">' +
                        '<m-assist-markdown :text="row.text" :invented="row.invented" :chips="row.chips" :streaming="row.streaming"></m-assist-markdown>' +
                        '<div v-if="!row.streaming && row.messageId && row.text" class="row items-center no-wrap m-assist-actions">' +
                            '<q-btn flat round dense size="xs" icon="sym_o_content_copy" :aria-label="labels.copy" @click="copy(row.text)"></q-btn>' +
                            '<q-btn flat round dense size="xs" icon="sym_o_thumb_up" :color="row.feedback && row.feedback.rating === \'up\' ? \'primary\' : undefined" :aria-label="labels.up" @click="rate(row, \'up\')"></q-btn>' +
                            '<q-btn flat round dense size="xs" icon="sym_o_thumb_down" :color="row.feedback && row.feedback.rating === \'down\' ? \'negative\' : undefined" :aria-label="labels.down" @click="rate(row, \'down\')"></q-btn>' +
                        '</div>' +
                    '</div>' +
                    '<div v-else-if="row.kind === \'reasoning\' || row.kind === \'thinking\'" class="m-assist-thought text-caption m-muted q-mb-xs" data-assist-item="reasoning">' +
                        '<span class="cursor-pointer" @click="toggleStep(row.key)"><q-icon name="sym_o_psychology" size="xs"></q-icon> {{labels.thought}}</span>' +
                        '<div v-if="openSteps[row.key] || row.kind === \'thinking\'" class="m-assist-pre q-pl-md">{{row.text}}</div>' +
                    '</div>' +
                    '<template v-else-if="row.kind === \'step\'">' +
                        '<div class="m-assist-step text-caption q-mb-xs" data-assist-item="step" :data-tool="row.name" :data-status="row.status">' +
                            '<div class="row items-center no-wrap cursor-pointer" @click="toggleStep(row.key)">' +
                                '<q-spinner v-if="row.status === \'running\'" size="14px" class="q-mr-xs"></q-spinner><q-icon v-else :name="stepIcon(row.status)" size="xs" :color="row.isError ? \'negative\' : undefined" class="q-mr-xs"></q-icon>' +
                                '<span class="ellipsis">{{row.title}}</span></div>' +
                            '<div v-if="openSteps[row.key]" class="m-assist-step-detail q-pl-md">' +
                                '<div class="m-assist-pre">{{row.arguments}}</div><div v-if="row.result" class="m-assist-pre q-mt-xs">{{row.result}}</div>' +
                                '<q-btn flat dense no-caps size="xs" icon="sym_o_content_copy" :label="labels.copy" @click="copy(row.arguments + \'\\n\' + row.result)"></q-btn>' +
                            '</div>' +
                        '</div>' +
                        '<m-assist-card v-if="approvalsByCall[row.toolCallId]" kind="approval" :card="approvalsByCall[row.toolCallId]"></m-assist-card>' +
                    '</template>' +
                    '<div v-else-if="row.kind === \'done\'" class="m-assist-done text-caption m-muted q-mb-sm" data-assist-item="done" :data-state="row.state">' +
                        '<span v-if="row.failed" class="text-negative">{{row.errorText || row.errorCode || labels.errorTitle}} </span>{{doneText(row)}}' +
                        '<q-btn v-if="row.failed" flat dense no-caps size="sm" :label="labels.retry" data-assist-action="retry" @click="retry"></q-btn>' +
                    '</div>' +
                    '<div v-else-if="row.kind === \'queued\'" class="m-assist-user m-assist-queued q-mb-sm" data-assist-item="queued">' +
                        '<div class="m-assist-bubble"><span class="m-assist-pre">{{row.text}}</span></div><div class="text-caption m-muted text-right">{{labels.queued}}</div>' +
                    '</div>' +
                    '<div v-else-if="row.kind === \'note\'" class="m-assist-note text-caption q-mb-sm m-assist-pre" data-assist-item="note">{{row.text}}</div>' +
                '</template>' +
                '<m-assist-card v-for="card in loose" :key="\'a\' + card.approvalId" kind="approval" :card="card"></m-assist-card>' +
            '</div>'
    };

    // An assistant text as sanitized Markdown (assist.renderMarkdown, then DOMPurify), drawn again at most once a frame while it
    // streams; an application link goes through the shell, as the user's own navigation.
    let markdownLoad = null;
    const loadMarkdown = () => markdownLoad || (markdownLoad = Promise.all([
        moqui.loadScript("/libs/marked/marked.umd.min.js", null, () => !!root.marked),
        moqui.loadScript("/libs/dompurify/purify.min.js", null, () => !!root.DOMPurify)
    ]).catch((err) => { markdownLoad = null; throw err; }));
    components["m-assist-markdown"] = {
        name: "mAssistMarkdown",
        // strict: a canvas's Markdown (no external link, no image of any kind)
        props: { text: String, invented: Array, chips: Object, streaming: Boolean, strict: Boolean },
        data() { return { html: "", failed: false, frame: null }; },
        template: '<div class="m-assist-markdown m-markdown" @click="onClick"><div v-if="failed" class="m-assist-pre">{{text}}</div><div v-else class="m-markdown-body" v-html="html"></div></div>',
        watch: { text() { this.schedule(); }, invented() { this.schedule(); } },
        mounted() { this.schedule(); },
        beforeUnmount() { if (this.frame && root.cancelAnimationFrame) root.cancelAnimationFrame(this.frame); },
        methods: {
            schedule() {
                if (this.frame) return;
                const raf = root.requestAnimationFrame || ((fn) => setTimeout(fn, 16));
                this.frame = raf(() => { this.frame = null; this.render(); });
            },
            render() {
                const text = this.text || "";
                loadMarkdown().then(() => {
                    const html = assist.renderMarkdown(root.marked, text, { invented: this.invented, chips: this.chips, linkBase: this.$root.linkBasePath || "/qapps",
                        strict: this.strict });
                    this.html = assist.sanitize(root.DOMPurify, html, null, this.strict);
                    this.failed = false;
                }, () => { this.failed = true; });
            },
            onClick(event) {
                const anchor = event.target && event.target.closest ? event.target.closest("a[href]") : null;
                if (!anchor || !anchor.hasAttribute("data-md-link") || event.ctrlKey || event.metaKey || event.shiftKey || event.button) return;
                event.preventDefault();
                const checked = assist.safePath(anchor.getAttribute("href"), null, { linkBase: this.$root.linkBasePath || "/qapps" });
                if (checked.ok) this.$root.setUrl(checked.url);
            }
        }
    };

    // A card: an approval (current → proposed, Approve, Edit in screen, Reject with feedback, Preview effects; collapsed once
    // decided, a conflict with Review again), a question or form (options with the recommended one marked, typed fields, free
    // text; the budget question's Continue and Stop) or a hand-off (the values Assist filled, Skip). No keyboard accelerators.
    components["m-assist-card"] = {
        name: "mAssistCard",
        props: { kind: String, card: Object },
        data() { return { rejecting: false, feedback: "", editing: false, argsText: "", argsError: "", picked: [], values: {}, free: "" }; },
        computed: {
            labels() { return labelsOf(this); },
            busy() { const b = st().busy; return !!(b["approval:" + this.card.approvalId] || b["question:" + this.card.questionId]); },
            previewing() { return !!st().busy["preview:" + this.card.approvalId]; },
            status() { return this.card.status || "pending"; },
            pending() { return this.status === "pending"; },
            canPreview() { const features = (st().bootstrap && st().bootstrap.features) || {}; return this.card.canPreview !== false && features.preview !== false; },
            editableArgs() { return this.card.targetKind && this.card.targetKind !== "screen" && (this.card.arguments || (this.card.tool && this.card.tool.arguments)); },
            statusLabel() {
                const map = { approved: "approved", executing: "executing", executed: "executed", failed: "errorTitle", rejected: "rejected", expired: "expired",
                    conflict: "conflict", "handed-off": "handedOff" };
                return this.labels[map[this.status]] || this.status;
            },
            options() { return (this.card.options || []).map((option) => (typeof option === "string" ? { label: option } : option)); },
            fields() { return this.card.fields || []; },
            budget() { return this.card.kind === "budget"; },
            handoffValues() { return this.card.values || []; },
            previewRows() { return (this.card.preview && this.card.preview.rows) || []; }
        },
        methods: {
            approve() {
                if (this.editing) {
                    let args = null;
                    try { args = JSON.parse(this.argsText); } catch (err) { this.argsError = String(err.message || err); return; }
                    assist.decide(this.card, "approve", { arguments: args });
                } else assist.decide(this.card, "approve");
            },
            edit() { assist.decide(this.card, "edit"); },
            startReject() { this.rejecting = true; },
            reject() { assist.decide(this.card, "reject", { feedback: this.feedback }); },
            useAlternative(text) { this.feedback = text; },
            preview() { assist.preview(this.card); },
            review() { assist.refresh(); },
            startEdit() {
                const args = this.card.arguments || (this.card.tool && this.card.tool.arguments) || {};
                this.argsText = typeof args === "string" ? args : JSON.stringify(args, null, 2);
                this.editing = true;
            },
            choose(option) {
                if (this.card.multi) {
                    const index = this.picked.indexOf(option.label);
                    if (index >= 0) this.picked.splice(index, 1); else this.picked.push(option.label);
                    return;
                }
                if (this.budget) assist.answer(this.card, { choice: option.label });
                else assist.answer(this.card, { choices: [option.label] });
            },
            submitAnswer() {
                const answer = {};
                if (this.card.multi && this.picked.length) answer.choices = this.picked.slice();
                if (this.fields.length) answer.values = Object.assign({}, this.values);
                if (this.free) answer.text = this.free;
                assist.answer(this.card, answer);
            },
            skip() { assist.abandonHandoff(this.card, "skipped"); }
        },
        template:
            '<q-card flat bordered class="m-assist-card q-mb-sm" :data-assist-card="kind" :data-status="status" :data-approval-id="card.approvalId" :data-question-id="card.questionId" :data-handoff-id="card.handoffId">' +
                '<template v-if="kind === \'approval\'">' +
                    '<q-card-section class="q-pa-sm">' +
                        '<div class="row items-center no-wrap"><q-icon :name="card.classification === \'destructive\' ? \'sym_o_warning\' : \'sym_o_approval\'" :color="card.classification === \'destructive\' ? \'negative\' : \'primary\'" size="xs" class="q-mr-xs"></q-icon>' +
                            '<div class="text-weight-medium ellipsis">{{card.summary || card.targetName}}</div><q-space></q-space>' +
                            '<q-badge v-if="!pending" outline :color="status === \'executed\' || status === \'approved\' ? \'positive\' : status === \'rejected\' || status === \'failed\' || status === \'conflict\' ? \'negative\' : \'grey\'" data-assist-card-status>{{statusLabel}}</q-badge></div>' +
                        '<div v-if="card.record && card.record.label" class="q-mt-xs"><q-chip dense square size="sm" icon="sym_o_link">{{card.record.label}}</q-chip></div>' +
                        '<div v-if="card.risk && card.risk.risk" class="text-caption q-mt-xs" :class="card.risk.risk === \'irreversible\' ? \'text-negative\' : \'m-muted\'" data-assist-risk>{{labels.risk}}: {{card.risk.risk}} ({{labels.riskFrom}} {{card.risk.skill}})</div>' +
                        '<table v-if="(card.changes || []).length && (pending || status === \'conflict\')" class="m-assist-changes q-mt-xs"><tbody>' +
                            '<tr v-for="change in card.changes" :key="change.field" :data-field="change.field"><td class="text-weight-medium">{{change.label || change.field}}</td>' +
                                '<td><span class="m-assist-current">{{change.current === null || change.current === undefined || change.current === \'\' ? \'—\' : change.current}}</span> → <span class="m-assist-proposed">{{change.proposed}}</span></td></tr>' +
                        '</tbody></table>' +
                        '<div v-if="status === \'conflict\'" class="text-caption text-negative q-mt-xs">{{card.targetName}} {{labels.conflict}}</div>' +
                        '<div v-if="previewRows.length" class="m-assist-preview text-caption q-mt-xs" data-assist-preview>' +
                            '<div v-for="(row, index) in previewRows" :key="index">{{row.action}} {{row.entity}} {{row.key}}: <span v-for="field in row.fields || []" :key="field.field">{{field.field}} {{field.before}} → {{field.after}}; </span></div></div>' +
                        '<div v-if="card.preview && card.preview.ok === false" class="text-caption text-negative q-mt-xs">{{card.preview.error || card.preview.message}}</div>' +
                        '<div v-if="editing" class="q-mt-xs"><q-input v-model="argsText" type="textarea" autogrow dense outlined :label="labels.arguments" :error="!!argsError" :error-message="argsError" data-assist-args></q-input></div>' +
                        '<div v-if="rejecting" class="q-mt-xs"><q-input v-model="feedback" type="textarea" autogrow dense outlined autofocus :label="labels.feedback" data-assist-feedback></q-input>' +
                            '<div v-if="(card.alternatives || []).length" class="q-mt-xs"><q-chip v-for="alt in card.alternatives" :key="alt" dense clickable size="sm" @click="useAlternative(alt)">{{alt}}</q-chip></div></div>' +
                    '</q-card-section>' +
                    '<q-card-actions v-if="pending" align="left" class="q-pt-none q-px-sm q-pb-sm q-gutter-xs">' +
                        '<template v-if="!rejecting">' +
                            '<q-btn unelevated no-caps dense color="primary" :label="editing ? labels.approveEdited : labels.approve" :loading="busy" data-assist-action="approve" @click="approve"></q-btn>' +
                            '<q-btn v-if="card.canEdit" unelevated no-caps dense class="m-btn-tonal" :label="labels.editInScreen" :disable="busy" data-assist-action="edit" @click="edit"></q-btn>' +
                            '<q-btn v-if="editableArgs && !editing" flat no-caps dense :label="labels.editArguments" data-assist-action="edit-arguments" @click="startEdit"></q-btn>' +
                            '<q-btn flat no-caps dense color="negative" :label="labels.reject" :disable="busy" data-assist-action="reject" @click="startReject"></q-btn>' +
                            '<q-btn v-if="canPreview" flat no-caps dense icon="sym_o_science" :label="labels.preview" :loading="previewing" data-assist-action="preview" @click="preview"><q-tooltip>{{labels.previewNote}}</q-tooltip></q-btn>' +
                        '</template>' +
                        '<q-btn v-else unelevated no-caps dense color="negative" :label="labels.rejectWith" :loading="busy" data-assist-action="reject-send" @click="reject"></q-btn>' +
                    '</q-card-actions>' +
                    '<q-card-actions v-else-if="status === \'conflict\'" class="q-pt-none q-px-sm q-pb-sm"><q-btn flat no-caps dense :label="labels.reviewAgain" data-assist-action="review" @click="review"></q-btn></q-card-actions>' +
                '</template>' +
                '<template v-else-if="kind === \'question\'">' +
                    '<q-card-section class="q-pa-sm">' +
                        '<div class="m-assist-pre">{{card.text}}</div>' +
                        '<div v-if="options.length" class="q-mt-xs q-gutter-xs">' +
                            '<q-btn v-for="option in options" :key="option.label" no-caps dense :unelevated="option.recommended || picked.indexOf(option.label) >= 0" :outline="!option.recommended && picked.indexOf(option.label) < 0" color="primary" :label="option.label" :disable="busy" :data-assist-option="option.label" @click="choose(option)">' +
                                '<q-badge v-if="option.recommended" floating color="positive" rounded>★</q-badge></q-btn>' +
                        '</div>' +
                        '<div v-for="field in fields" :key="field.name" class="q-mt-xs">' +
                            '<q-select v-if="field.options && field.options.length" v-model="values[field.name]" :options="field.options" emit-value map-options dense outlined stack-label :label="field.label + (field.required ? \' *\' : \'\')" :data-assist-field="field.name"></q-select>' +
                            '<q-checkbox v-else-if="field.type === \'boolean\'" v-model="values[field.name]" dense :label="field.label" :data-assist-field="field.name"></q-checkbox>' +
                            '<q-input v-else v-model="values[field.name]" dense outlined stack-label :type="field.type === \'number\' ? \'number\' : field.type === \'date\' ? \'date\' : \'text\'" :label="field.label + (field.required ? \' *\' : \'\')" :data-assist-field="field.name"></q-input>' +
                        '</div>' +
                        '<q-input v-if="card.freeText !== false && !budget" v-model="free" dense outlined class="q-mt-xs" :label="options.length ? labels.other : labels.answer" data-assist-free @keyup.enter="submitAnswer"></q-input>' +
                    '</q-card-section>' +
                    '<q-card-actions v-if="!budget && (fields.length || card.multi || card.freeText !== false)" class="q-pt-none q-px-sm q-pb-sm"><q-btn unelevated no-caps dense color="primary" :label="labels.answer" :loading="busy" data-assist-action="answer" @click="submitAnswer"></q-btn></q-card-actions>' +
                '</template>' +
                '<template v-else-if="kind === \'handoff\'">' +
                    '<q-card-section class="q-pa-sm" id="m-assist-handoff">' +
                        '<div class="text-weight-medium">{{card.lastOk === false || status === \'failed\' ? labels.handoffFailed : labels.handoff}}</div>' +
                        '<div v-for="value in handoffValues" :key="value.field" class="text-caption">{{value.label || value.field}}: <span class="m-assist-proposed">{{value.value}}</span></div>' +
                    '</q-card-section>' +
                    '<q-card-actions class="q-pt-none q-px-sm q-pb-sm"><q-btn flat no-caps dense :label="labels.skip" :loading="status === \'abandoning\'" data-assist-action="skip" @click="skip"></q-btn></q-card-actions>' +
                '</template>' +
            '</q-card>'
    };

    // The composer: Enter sends, Shift+Enter is a new line, ↑ in an empty composer recalls the last message, / shows the
    // commands, @ picks a record from the palette's sources; Stop while the run works (the partial answer stays).
    components["m-assist-composer"] = {
        name: "mAssistComposer",
        data() { return { mention: null, mentionRows: [], mentionTimer: null, mentionSeq: 0, selected: 0 }; },
        computed: {
            labels() { return labelsOf(this); },
            s() { return st(); },
            draft: { get() { return st().draft; }, set(value) { st().draft = value; } },
            running() { return !!st().convo.activeRunId; },
            commandHint() { return /^\/[a-z]*$/.test(this.draft) ? this.labels.help + (assist.isAdmin() ? " " + this.labels.adminHelp : "") : ""; }
        },
        methods: {
            input() { return this.$el && this.$el.querySelector ? this.$el.querySelector("textarea") : null; },
            onKey(event) {
                if (this.mention && this.mentionRows.length && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
                    event.preventDefault();
                    const count = this.mentionRows.length;
                    this.selected = (this.selected + (event.key === "ArrowDown" ? 1 : count - 1)) % count;
                    return;
                }
                if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
                    event.preventDefault();
                    if (this.mention && this.mentionRows.length) { this.pick(this.mentionRows[this.selected]); return; }
                    this.send();
                    return;
                }
                if (event.key === "ArrowUp" && !this.draft) {
                    const mine = st().convo.items.filter((item) => item.role === "user");
                    const last = mine[mine.length - 1];
                    if (last) { event.preventDefault(); this.draft = last.parts.map((part) => part.text).join("\n"); }
                }
            },
            onInput(value) {
                const text = value === undefined || value === null ? this.draft : String(value);
                const el = this.input();
                const mention = assist.mentionAt(text, el ? el.selectionStart : text.length);
                this.mention = mention;
                clearTimeout(this.mentionTimer);
                if (!mention) { this.mentionRows = []; return; }
                this.mentionTimer = setTimeout(() => this.searchMention(mention.query), 250);
            },
            // The skills whose name starts with or matches the query first (inserted as @name), then the records (chips).
            searchMention(query) {
                const seq = ++this.mentionSeq;
                if (!query) { this.mentionRows = []; return; }
                const none = () => ({});
                Promise.all([assist.call("mentions", { kind: "skills", q: query }).catch(none),
                    assist.call("mentions", { kind: "records", q: query, limit: 8 }).catch(none)]).then(([skills, records]) => {
                    if (seq !== this.mentionSeq) return;
                    const rows = [];
                    (skills.items || []).slice(0, 3).forEach((item) => { if (item && item.name) rows.push({ name: item.name, title: "@" + item.name, subtitle: item.description || "", key: "skill:" + item.name }); });
                    (records.groups || []).forEach((group) => (group.rows || []).forEach((row) => { if (row && row.recordEntity && row.recordId) rows.push(row); }));
                    this.mentionRows = rows.slice(0, 8);
                    this.selected = 0;
                }, () => { this.mentionRows = []; });
            },
            pick(row) {
                const out = assist.insertMention(this.draft, this.mention, row);
                this.draft = out.text;
                this.mention = null;
                this.mentionRows = [];
                this.$nextTick(() => { const el = this.input(); if (el) { el.focus(); el.setSelectionRange(out.caret, out.caret); } });
            },
            send() {
                const text = this.draft;
                if (!String(text).trim()) return;
                this.draft = "";
                assist.send(text);
            },
            stop() { assist.stop(); }
        },
        template:
            '<div class="m-assist-composer q-pa-sm" id="m-assist-composer">' +
                '<div v-if="s.error" class="text-caption text-negative q-mb-xs" id="m-assist-error">{{s.error}}</div>' +
                '<div v-if="commandHint" class="text-caption m-muted q-mb-xs">{{commandHint}}</div>' +
                '<q-list v-if="mention && mentionRows.length" dense bordered class="m-assist-mentions q-mb-xs" id="m-assist-mentions">' +
                    '<q-item v-for="(row, index) in mentionRows" :key="row.key || (row.recordEntity + row.recordId)" clickable dense :active="index === selected" @click="pick(row)">' +
                        '<q-item-section><q-item-label lines="1">{{row.title}}</q-item-label><q-item-label v-if="row.subtitle" caption lines="1">{{row.subtitle}}</q-item-label></q-item-section></q-item>' +
                '</q-list>' +
                '<div class="row items-end no-wrap q-gutter-x-xs">' +
                    '<q-input v-model="draft" type="textarea" autogrow dense outlined class="col m-assist-input" input-class="m-assist-textarea" :placeholder="labels.placeholder" for="m-assist-input" name="assistText" @keydown="onKey" @update:model-value="onInput"></q-input>' +
                    '<q-btn v-if="running" unelevated round dense color="negative" icon="sym_o_stop" id="m-assist-stop" :aria-label="labels.stop" @click="stop"><q-tooltip>{{labels.stop}}</q-tooltip></q-btn>' +
                    '<q-btn unelevated round dense color="primary" icon="sym_o_send" id="m-assist-send" :aria-label="labels.send" :disable="!draft.trim()" @click="send"></q-btn>' +
                '</div>' +
            '</div>'
    };

    // History: the user's conversations, newest first, with search; one opens in the drawer.
    components["m-assist-history"] = {
        name: "mAssistHistory",
        computed: { labels() { return labelsOf(this); }, history() { return st().history; } },
        methods: {
            search() { clearTimeout(this.timer); this.timer = setTimeout(() => assist.loadHistory(), 250); },
            openOne(item) { assist.load(item.conversationId, { pane: "chat" }).catch((err) => { st().error = err.errors; }); },
            when(date) { return date ? new Date(date).toLocaleString() : ""; }
        },
        template:
            '<div class="m-assist-history column no-wrap" id="m-assist-history-pane">' +
                '<div class="q-pa-sm"><q-input v-model="history.search" dense outlined clearable :placeholder="labels.searchConversations" @update:model-value="search"><template v-slot:prepend><q-icon name="sym_o_search" size="xs"></q-icon></template></q-input></div>' +
                '<q-list class="col scroll" separator dense>' +
                    '<q-item v-if="!history.loading && !history.list.length"><q-item-section class="m-muted">{{labels.noConversations}}</q-item-section></q-item>' +
                    '<q-item v-for="item in history.list" :key="item.conversationId" clickable :data-conversation-id="item.conversationId" @click="openOne(item)">' +
                        '<q-item-section><q-item-label lines="1">{{item.title || item.conversationId}}</q-item-label><q-item-label caption>{{when(item.lastMessageDate || item.createdDate)}}</q-item-label></q-item-section>' +
                        '<q-item-section side><q-icon v-if="item.waiting" name="sym_o_pending_actions" color="warning" size="xs"></q-icon><q-spinner v-else-if="item.activeRunId" size="14px"></q-spinner></q-item-section>' +
                    '</q-item>' +
                '</q-list>' +
            '</div>'
    };

    // Tasks: what waits for the user, what runs, what failed and what was done lately, across every surface; Open shows it.
    components["m-assist-tasks"] = {
        name: "mAssistTasks",
        computed: {
            labels() { return labelsOf(this); },
            tasks() { return st().tasks || { waiting: [], running: [], failed: [], done: [] }; },
            groups() {
                const t = this.tasks;
                return [{ key: "waiting", label: this.labels.waiting, items: t.waiting || [] }, { key: "running", label: this.labels.running, items: t.running || [] },
                    { key: "failed", label: this.labels.failed, items: t.failed || [] }, { key: "done", label: this.labels.recentlyDone, items: t.done || [] }].filter((group) => group.items.length);
            }
        },
        methods: {
            openOne(item) { if (item.conversationId) assist.load(item.conversationId, { pane: "chat" }).catch((err) => { st().error = err.errors; }); },
            cancel(item) { assist.call("cancel", { runId: item.runId }, "POST").then(() => assist.refreshTasks(), (err) => { st().error = err.errors; }); }
        },
        template:
            '<div class="m-assist-tasks scroll" id="m-assist-tasks-pane">' +
                '<div v-if="!groups.length" class="q-pa-md m-muted">{{labels.nothingWaiting}}</div>' +
                '<template v-for="group in groups" :key="group.key">' +
                    '<q-item-label header class="q-pb-xs">{{group.label}}</q-item-label>' +
                    '<q-list dense separator>' +
                        '<q-item v-for="item in group.items" :key="(item.approvalId || item.questionId || item.handoffId || item.runId) + group.key" :data-assist-task="item.kind">' +
                            '<q-item-section><q-item-label lines="2">{{item.summary || item.conversationTitle || item.runId}}</q-item-label>' +
                                '<q-item-label caption lines="1">{{item.conversationTitle}}{{item.surface ? \' · \' + item.surface : \'\'}}{{item.errorText ? \' · \' + item.errorText : \'\'}}</q-item-label></q-item-section>' +
                            '<q-item-section side class="row no-wrap"><div class="row no-wrap">' +
                                '<q-btn flat dense no-caps size="sm" color="primary" :label="labels.open" @click="openOne(item)"></q-btn>' +
                                '<q-btn v-if="group.key === \'running\'" flat dense no-caps size="sm" :label="labels.stop" @click="cancel(item)"></q-btn></div></q-item-section>' +
                        '</q-item>' +
                    '</q-list>' +
                '</template>' +
            '</div>'
    };

    // Before WebrootVue.qvt.js its registry takes them; after it they are registered late on the running application.
    const qvt = moqui.qvt;
    if (qvt && qvt.registerLate) Object.keys(components).forEach((name) => qvt.registerLate(name, components[name]));
    else {
        const registry = moqui.qvtComponents = moqui.qvtComponents || {};
        Object.keys(components).forEach((name) => { registry[name] = components[name]; });
    }
    assist.components = components;

    if (typeof module !== "undefined" && module.exports) module.exports = assist;
})(globalThis);
