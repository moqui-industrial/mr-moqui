/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

/*
 * The canvas of Assist (plans/LLM-A2A-Assist.md 4.10 and 4.11; the server's contract is docs/Assist.md "The canvas"):
 * moqui.assist.canvas, loaded on the first canvas with the vendored OpenUI Lang core (libs/openui/lang-core.min.js, the
 * global OpenUILang). It holds
 *   - the component library of the OpenUI Lang dialect: each component's declaration (its positional parameters, their
 *     types, the widget it is drawn with), from which canvas.librarySpec() makes openui-library.spec.json, the JSON
 *     Schema lang-core parses with in the browser and on the server (a node test fails when the file is stale), and
 *     canvas.signature() the signatures of the openui-lang skill;
 *   - the pure parts (tested under node): the state reconciliation of a program that changes under the user's edits
 *     (persistedState, reconcileUserEdits), what a canvas read answers (rows of a screen list through the read proxy),
 *     the problems a render reports, chart configurations for Chart.js 4, the paths a canvas may open;
 *   - the renderer: Vue 3 render functions (no template, so a webapp_qvt_static_only page and a CSP without unsafe-eval
 *     run it), a port of the Java prototype's AssistOpenUi.js (state reconciliation, action routing, the streaming parse,
 *     evaluation) without its regex repair layer: what lang-core does not parse is reported, never rewritten. The
 *     components are drawn with the shell's widgets (m-text-line, m-date-time, m-date-period, m-chart, m-markdown,
 *     m-mermaid, Quasar's layout), so a generated view looks like a screen.
 *
 * Reads (a Query, a Lookup's options, a Display's value) go through the one read proxy, GET /assist/read, never to a
 * screen from the page. A Mutation is run by the server from the stored program (POST canvasAction): the user's click on
 * a Button the framework drew is the approval, so no card is asked for it; a Mutation the agent proposes without a click
 * is an approval card in the drawer (write_ui run). The dialect registers itself: moqui.assist.dialects.register("openui").
 */
(function (root) {
    "use strict";

    const moqui = root.moqui = root.moqui || {};
    const canvas = {};

    // ---- the component library: the declarations the spec is made from ----

    // A parameter: its name, its JSON Schema type (none is any), whether it is required, and a note for the skill.
    const p = (name, type, required, note) => ({ name, type: type || null, required: !!required, note: note || "" });
    const LIBRARY = [
        // layout
        { name: "Stack", group: "layout", description: "Vertical or horizontal layout. Root is Stack. direction: column (default) or row. gap: s, m, or l. wrap: true to wrap a row of cards/charts.",
            params: [p("children", "array", true), p("direction", "string"), p("gap", "string"), p("wrap", "boolean")] },
        { name: "Card", group: "layout", description: "Card container for a section of the canvas.", params: [p("children", "array", true)] },
        { name: "Separator", group: "layout", description: "Horizontal rule between sections.", params: [p("inset", "boolean")] },
        { name: "Accordion", group: "layout", description: "Expandable sections. Accordion([AccordionItem(value, title, content), ...]).", params: [p("children", "array", true)] },
        { name: "AccordionItem", group: "layout", description: "Accordion panel. AccordionItem(value, title, content).",
            params: [p("value", "string", true), p("title", "string", true), p("content", null, true)] },
        { name: "Steps", group: "layout", description: "Linear stepper. Steps([StepsItem(value, title, content), ...]).", params: [p("children", "array", true)] },
        { name: "StepsItem", group: "layout", description: "Stepper panel. StepsItem(value, title, content).",
            params: [p("value", "string", true), p("title", "string", true), p("content", null, true)] },
        { name: "Modal", group: "layout", description: "Dialog bound to $name (boolean). Modal(name, title, [children]). Open with @Set($name, true); X/Escape sets $name false.",
            params: [p("name", "string", true), p("title", "string", true), p("children", "array", true)] },
        { name: "Tabs", group: "layout", description: "Tabs. Tabs([TabItem(value, trigger, content), ...]).", params: [p("children", "array", true)] },
        { name: "TabItem", group: "layout", description: "Tab panel. TabItem(value, triggerLabel, content).",
            params: [p("value", "string", true), p("trigger", "string", true), p("content", null, true)] },
        // content
        { name: "CardHeader", group: "content", description: "Section title and optional subtitle.", params: [p("title", "string", true), p("subtitle", "string")] },
        { name: "TextContent", group: "content", description: "Plain text. Do not emit HTML. size: body (default), small, or large.", params: [p("text", "string", true), p("size", "string")] },
        { name: "Callout", group: "content", description: "Banner. type: info, warning, positive, or negative.", params: [p("text", "string", true), p("type", "string")] },
        { name: "Stat", group: "content", description: "KPI card. Stat(label, value, caption?).", params: [p("label", "string", true), p("value"), p("caption", "string")] },
        { name: "Tag", group: "content", description: "Chip. Tag(label, color?, size?). color: primary, positive, negative, warning, info, grey. size: sm or md.",
            params: [p("label", "string", true), p("color", "string"), p("size", "string")] },
        { name: "ListBlock", group: "content", description: "List. ListBlock([ListItem(title, caption?, side?), ...]).", params: [p("children", "array", true)] },
        { name: "ListItem", group: "content", description: "List row. ListItem(title, caption?, side?). side is often a Link or Tag.",
            params: [p("title", "string", true), p("caption", "string"), p("side")] },
        { name: "Image", group: "content", description: "An image of this site. Image(src, alt?). src is a path starting with / (not a host URL).", params: [p("src", "string", true), p("alt", "string")] },
        { name: "MarkDownRenderer", group: "content", description: "Sanitized markdown (GFM, code highlight, mermaid fences). MarkDownRenderer(textMarkdown, variant?). variant: clear or card. Do not emit HTML.",
            params: [p("textMarkdown", "string", true), p("variant", "string")] },
        { name: "Mermaid", group: "content", description: "Mermaid diagram. Mermaid(text, height?). Use for status/flow diagrams.", params: [p("text", "string", true), p("height", "string")] },
        { name: "CodeBlock", group: "content", description: "Code. CodeBlock(code, language?).", params: [p("code", "string", true), p("language", "string")] },
        // tables
        { name: "Table", group: "tables", description: "Data table. Table([Col(label, query.rows.field), ...]). Col values may be Link/Tag via @Each.", params: [p("columns", "array", true)] },
        { name: "Col", group: "tables", description: "Table column. Col(label, valuesArray): a plucked query column or @Each of components.", params: [p("label", "string", true), p("values", "array")] },
        // charts
        { name: "Series", group: "charts", description: "Chart series. Series(category, valuesArray).", params: [p("category", "string", true), p("values", "array", true)] },
        { name: "BarChart", group: "charts", description: "Vertical bars. BarChart(labels, [Series(name, values)], variant?, xLabel?, yLabel?). variant: grouped or stacked.",
            params: [p("labels", "array", true), p("series", "array", true), p("variant", "string"), p("xLabel", "string"), p("yLabel", "string")] },
        { name: "LineChart", group: "charts", description: "Lines over categories. LineChart(labels, [Series(name, values)], variant?, xLabel?, yLabel?). variant: linear, natural, or step.",
            params: [p("labels", "array", true), p("series", "array", true), p("variant", "string"), p("xLabel", "string"), p("yLabel", "string")] },
        { name: "AreaChart", group: "charts", description: "Filled lines. AreaChart(labels, [Series(name, values)], variant?, xLabel?, yLabel?).",
            params: [p("labels", "array", true), p("series", "array", true), p("variant", "string"), p("xLabel", "string"), p("yLabel", "string")] },
        { name: "PieChart", group: "charts", description: "Pie or donut. PieChart(labels, values, variant?). variant: pie or donut. Values are numbers (@Count, @Sum).",
            params: [p("labels", "array", true), p("values", "array", true), p("variant", "string")] },
        // forms
        { name: "Form", group: "forms", description: "Form wrapper. Form(name, submitButton, [controls]).", params: [p("name", "string", true), p("submit"), p("children", "array", true)] },
        { name: "FormControl", group: "forms", description: "Labeled control. FormControl(label, input).", params: [p("label", "string", true), p("control", null, true)] },
        { name: "Input", group: "forms", description: "Single-line text. Input(name, $name, placeholder?, label?).",
            params: [p("name", "string", true), p("value"), p("placeholder", "string"), p("label", "string")] },
        { name: "TextArea", group: "forms", description: "Multi-line text. TextArea(name, $name, placeholder?, label?).",
            params: [p("name", "string", true), p("value"), p("placeholder", "string"), p("label", "string")] },
        { name: "Select", group: "forms", description: "Static dropdown. Select(name, $name, [SelectItem(value, label), ...], label?).",
            params: [p("name", "string", true), p("value"), p("items", "array", true), p("label", "string")] },
        { name: "SelectItem", group: "forms", description: "Select option. SelectItem(value, label).", params: [p("value", "string", true), p("label", "string", true)] },
        { name: "CheckBox", group: "forms", description: "Checkbox. CheckBox(name, $name, label?).", params: [p("name", "string", true), p("value"), p("label", "string")] },
        { name: "RadioGroup", group: "forms", description: "Radio set. RadioGroup(name, $name, [RadioItem(value, label), ...], label?).",
            params: [p("name", "string", true), p("value"), p("items", "array", true), p("label", "string")] },
        { name: "RadioItem", group: "forms", description: "Radio option. RadioItem(value, label).", params: [p("value", "string", true), p("label", "string", true)] },
        { name: "Switch", group: "forms", description: "Boolean toggle. Switch(name, $name, label?).", params: [p("name", "string", true), p("value"), p("label", "string")] },
        { name: "Slider", group: "forms", description: "Numeric slider. Slider(name, $name, min?, max?, label?). min default 0, max default 100.",
            params: [p("name", "string", true), p("value"), p("min", "number"), p("max", "number"), p("label", "string")] },
        { name: "DateTime", group: "forms", description: "Date/time. DateTime(name, $name, type?, label?). type: date, time, or date-time; the value is YYYY-MM-DD HH:mm, YYYY-MM-DD, HH:mm or now.",
            params: [p("name", "string", true), p("value"), p("type", "string"), p("label", "string")] },
        { name: "DatePeriod", group: "forms", description: "Find period. DatePeriod(name, label?, type?). Sets $name_period, $name_poffset, $name_pdate, $name_from and $name_thru for Query arguments.",
            params: [p("name", "string", true), p("label", "string"), p("type", "string")] },
        { name: "Lookup", group: "forms", description: "Server-search dropdown. Lookup(name, $name, optionsUrl, valueField?, labelField?, dependsOn?, label?). optionsUrl is an /apps transition that answers options.",
            params: [p("name", "string", true), p("value"), p("optionsUrl", "string", true), p("valueField", "string"), p("labelField", "string"), p("dependsOn"), p("label", "string")] },
        { name: "Display", group: "forms", description: "Read-only value. Display(text) or Display(text, valueUrl, label?).", params: [p("text", "string"), p("valueUrl", "string"), p("label", "string")] },
        // actions
        { name: "Button", group: "actions", description: "Button. Button(label, Action([@Run(mutation), @Run(query), @Reset($a)]), color?). color: primary (default), secondary, or negative.",
            params: [p("label", "string", true), p("action"), p("color", "string")] },
        { name: "Buttons", group: "actions", description: "Row of buttons.", params: [p("children", "array", true)] },
        { name: "Link", group: "actions", description: "A link to a screen of the apps. Link(label, path, params?, hash?). path is an /apps or /qapps screen path.",
            params: [p("label", "string", true), p("path", "string", true), p("params"), p("hash", "string")] },
    ];
    canvas.library = LIBRARY;
    canvas.GROUPS = ["layout", "content", "tables", "charts", "forms", "actions"];

    // The JSON Schema of the library, as lang-core's createParser reads it ($defs by component, properties in argument order,
    // required names): what openui-library.spec.json holds.
    canvas.librarySpec = () => {
        const defs = {};
        LIBRARY.forEach((def) => {
            const properties = {};
            def.params.forEach((param) => { properties[param.name] = param.type ? { type: param.type } : {}; });
            const entry = { description: def.description, properties };
            const required = def.params.filter((param) => param.required).map((param) => param.name);
            if (required.length) entry.required = required;
            defs[def.name] = entry;
        });
        return { $defs: defs };
    };
    // openui-library.spec.json as written: two-space JSON and a final newline
    canvas.librarySpecText = () => JSON.stringify(canvas.librarySpec(), null, 2) + "\n";
    // Select(name, value?, items, label?): the signature the skill and the server's errors quote
    canvas.signature = (name) => {
        const def = LIBRARY.find((one) => one.name === name);
        if (!def) return "";
        return def.name + "(" + def.params.map((param) => param.name + (param.required ? "" : "?")).join(", ") + ")";
    };

    // ---- state: a program that changes under the user's edits ----

    const has = (obj, key) => !!obj && Object.prototype.hasOwnProperty.call(obj, key);
    const unwrap = (value) => (value && typeof value === "object" && !Array.isArray(value) && has(value, "value") ? value.value : value);
    const sameLiteral = (a, b) => {
        const av = unwrap(a);
        const bv = unwrap(b);
        if (av === bv) return true;
        if (av === null || av === undefined || bv === null || bv === undefined) return false;
        if (typeof av === "object" || typeof bv === "object") return false;
        return String(av) === String(bv);
    };
    const isScalar = (value) => value === null || value === undefined || typeof value !== "object";
    canvas.unwrap = unwrap;
    canvas.sameLiteral = sameLiteral;

    // What the store starts from when a program is parsed again (a new version, a streamed chunk): the state the page had
    // (initial, by $name or bare name), and a $name whose literal default changed in the program takes the new default
    // when the user had not changed it (it still holds the old literal). A value the user typed stays.
    canvas.persistedState = (decls, prevDecls, initial) => {
        const persisted = {};
        const src = initial || {};
        const prior = prevDecls || {};
        Object.keys(src).forEach((key) => { persisted[key.charAt(0) === "$" ? key : "$" + key] = src[key]; });
        Object.keys(decls || {}).forEach((dollar) => {
            if (!isScalar(decls[dollar]) || !has(prior, dollar) || !isScalar(prior[dollar])) return;
            if (sameLiteral(prior[dollar], decls[dollar])) return;
            const bare = dollar.charAt(0) === "$" ? dollar.slice(1) : dollar;
            const incoming = has(src, dollar) ? src[dollar] : has(src, bare) ? src[bare] : undefined;
            if (incoming !== undefined && !sameLiteral(incoming, prior[dollar])) return;
            persisted[dollar] = decls[dollar];
        });
        return persisted;
    };
    // The names the user edited that still count: not one the new program removed, not one that only holds the old or the new
    // literal default.
    canvas.reconcileUserEdits = (edited, prevDecls, decls, current) => {
        const next = {};
        const prior = prevDecls || {};
        Object.keys(edited || {}).forEach((name) => {
            if (!edited[name] || !has(decls, name)) return;
            const decl = decls[name];
            if (!isScalar(decl)) { next[name] = true; return; }
            const value = current ? current[name] : undefined;
            const old = has(prior, name) ? prior[name] : undefined;
            if (old !== undefined && isScalar(old) && sameLiteral(value, old)) return;
            if (sameLiteral(value, decl)) return;
            next[name] = true;
        });
        return next;
    };
    // The values of a store snapshot by bare name, a form's fields flattened: what an action sends and canvasState keeps.
    canvas.valuesOf = (snapshot) => {
        const out = {};
        Object.keys(snapshot || {}).forEach((key) => {
            const value = unwrap(snapshot[key]);
            if (key.indexOf("__openui") === 0) return;
            if (key.charAt(0) === "$") out[key.slice(1)] = value;
            else if (value && typeof value === "object" && !Array.isArray(value)) Object.keys(value).forEach((field) => { out[field] = unwrap(value[field]); });
        });
        return out;
    };

    // ---- reads and problems ----

    // The read proxy's parameters of a Query or a lookup: the canvas, the path and, as JSON, its query; only GET.
    canvas.readParams = (artifactId, args) => {
        const a = args || {};
        const method = String(a.method || "GET").toUpperCase();
        if (method !== "GET") return { error: "a canvas reads with GET; a write is a Mutation run by a Button" };
        const path = String(a.path || "");
        const checked = moqui.assist && moqui.assist.safePath ? moqui.assist.safePath(path.split("?")[0], null) : { ok: path.charAt(0) === "/" };
        if (!checked.ok) return { error: "the path " + path + " is not a screen of the apps (" + (checked.error || "not a path") + ")" };
        const out = { artifactId: artifactId || "", path };
        if (a.list) out.list = String(a.list);
        const query = {};
        Object.keys(a.query || {}).forEach((key) => {
            const value = unwrap(a.query[key]);
            if (value !== null && value !== undefined && value !== "") query[key] = Array.isArray(value) ? value.join(",") : String(value);
        });
        if (Object.keys(query).length) out.query = JSON.stringify(query);
        return out;
    };
    // The problems of a render as the server takes them (canvasProblems): lang-core's parse errors, names it could not resolve,
    // the evaluation's errors and failed Queries, each once; nothing while it streams.
    canvas.problemsOf = (parseResult, runtimeErrors, queryErrors) => {
        const out = [];
        const seen = {};
        const add = (problem) => {
            const key = [problem.code, problem.statementId || "", String(problem.message || "").replace(/\s+/g, " ").trim()].join("|");
            if (seen[key]) return;
            seen[key] = true;
            out.push({ source: problem.source || "parser", code: problem.code || "error", message: String(problem.message || ""),
                statementId: problem.statementId || null, component: problem.component || null });
        };
        const meta = (parseResult && parseResult.meta) || {};
        (meta.errors || []).forEach((error) => add(Object.assign({ source: "parser" }, error)));
        (meta.unresolved || []).forEach((name) => add({ source: "parser", code: "unresolved", message: name + " is used but not defined", statementId: name }));
        if (parseResult && !parseResult.root) add({ source: "parser", code: "no-root", message: "the program has no root to show" });
        (runtimeErrors || []).forEach((error) => add(Object.assign({ source: "runtime" }, error)));
        (queryErrors || []).forEach((error) => add(Object.assign({ source: "query" }, error)));
        return out.slice(0, 10);
    };
    // The messages of a screen's JSON answer: a transition answers 200 even when its service refused the input.
    canvas.responseProblems = (parsed) => {
        const errors = [];
        const warnings = [];
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return { errors, warnings };
        (Array.isArray(parsed.validationErrors) ? parsed.validationErrors : []).forEach((ve) => {
            const field = ve && (ve.fieldPretty || ve.field);
            errors.push(String((ve && ve.message) || "Validation error") + (field ? " (for field " + field + ")" : ""));
        });
        (Array.isArray(parsed.errors) ? parsed.errors : parsed.errors ? [parsed.errors] : []).forEach((error) => { if (error) errors.push(String(error)); });
        (Array.isArray(parsed.messageInfos) ? parsed.messageInfos : []).forEach((info) => {
            if (!info || !info.message) return;
            const type = String(info.type || "").toLowerCase();
            if (type === "danger" || type === "error") errors.push(String(info.message));
            else if (type === "warning") warnings.push(String(info.message));
        });
        return { errors, warnings };
    };

    // ---- charts and values ----

    const CHART_COLORS = ["#1976d2", "#26a69a", "#9c27b0", "#ef6c00", "#c62828", "#546e7a", "#7cb342", "#f9a825"];
    const CHART_MAX_SERIES = 8;
    const CHART_MAX_POINTS = 200;
    const asArray = (value) => (value === null || value === undefined ? [] : Array.isArray(value) ? value : [value]);
    const nodeProps = (value) => (value && value.type === "element" && value.props ? value.props : value && typeof value === "object" ? value : {});
    // a number of a cell as a list shows it: "1,234.50", "$12", "45%" are numbers
    canvas.toNumber = (value) => {
        if (typeof value === "number") return isFinite(value) ? value : 0;
        if (value === null || value === undefined || typeof value === "object") return 0;
        const text = String(value).replace(/[\s,$€£%]/g, "");
        const number = parseFloat(text);
        return isFinite(number) ? number : 0;
    };
    const asNumbers = (list) => asArray(list).slice(0, CHART_MAX_POINTS).map(canvas.toNumber);
    const asLabels = (list) => asArray(list).slice(0, CHART_MAX_POINTS).map((value) => (value === null || value === undefined ? "" : String(value)));
    // The Chart.js 4 configuration of a bar, line or area chart.
    canvas.xyChart = (type, props, fill) => {
        const variant = props.variant || "";
        const datasets = asArray(props.series).slice(0, CHART_MAX_SERIES).map((series, index) => {
            const sp = nodeProps(series);
            const color = CHART_COLORS[index % CHART_COLORS.length];
            const set = { label: String(sp.category || sp.label || "Series " + (index + 1)), data: asNumbers(sp.values), backgroundColor: color, borderColor: color };
            if (type === "line") {
                set.fill = !!fill;
                set.tension = variant === "natural" ? 0.4 : 0;
                if (variant === "step") set.stepped = true;
                if (fill) set.backgroundColor = color + "55";
            }
            return set;
        });
        const stacked = variant === "stacked";
        const axis = (label) => ({ stacked, title: { display: !!label, text: label || "" } });
        return {
            type,
            data: { labels: asLabels(props.labels), datasets },
            options: { maintainAspectRatio: false, animation: false, plugins: { legend: { display: datasets.length > 1 } },
                scales: { x: axis(props.xLabel), y: Object.assign(axis(props.yLabel), { beginAtZero: true }) } },
        };
    };
    canvas.pieChart = (props) => {
        const labels = asLabels(props.labels);
        const values = asNumbers(props.values);
        const n = Math.min(labels.length, values.length);
        return {
            type: props.variant === "donut" || props.variant === "doughnut" ? "doughnut" : "pie",
            data: { labels: labels.slice(0, n), datasets: [{ data: values.slice(0, n), backgroundColor: labels.slice(0, n).map((_, i) => CHART_COLORS[i % CHART_COLORS.length]) }] },
            options: { maintainAspectRatio: false, animation: false, plugins: { legend: { position: "right" } } },
        };
    };
    // A date value for m-date-time: YYYY-MM-DD HH:mm (or the date or the time alone); now and epoch milliseconds become that.
    canvas.dateValue = (value, type) => {
        if (value === null || value === undefined || value === "" || typeof value === "object") return "";
        const kind = type === "date" || type === "time" ? type : "date-time";
        const text = String(value).trim();
        const pad = (n) => (n < 10 ? "0" : "") + n;
        const format = (ms) => {
            const d = new Date(ms);
            if (isNaN(d.getTime())) return "";
            const day = d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate());
            const time = pad(d.getHours()) + ":" + pad(d.getMinutes());
            return kind === "date" ? day : kind === "time" ? time : day + " " + time;
        };
        if (text.toLowerCase() === "now") return format(Date.now());
        if (/^-?\d{12,13}$/.test(text)) return format(Number(text));
        return text;
    };
    // A text a component shows: a value, never an object (an unevaluated tree would show as JSON).
    const textOf = (value) => (value === null || value === undefined || typeof value === "object" ? "" : String(value));
    canvas.textOf = textOf;
    // An image of this site only: a static file path from the root (an image's extension, no query), never another host
    // and never a screen, a transition or a call of the apps (a GET of one would run it with the user's cookie).
    canvas.siteImage = (src) => {
        const text = String(src || "");
        if (text.charAt(0) !== "/" || text.charAt(1) === "/" || /[\s\\?#]/.test(text) || /^\/[^/]*:/.test(text) || text.indexOf("..") >= 0) return null;
        if (/^\/(apps|qapps|rest|assist|vapps|s1|e1|m1)(\/|$)/i.test(text)) return null;
        return /\.(png|jpe?g|gif|svg|webp|ico)$/i.test(text) ? text : null;
    };
    // A CSS length a component may take (a Mermaid height): a number and a unit, nothing else, so no url() reaches a style.
    canvas.cssLength = (value, fallback) => (/^\d{1,4}(\.\d+)?(px|em|rem|vh|%)$/.test(String(value || "")) ? String(value) : fallback);
    // The Mutation steps of a Button's Action plan, and what a Mutation saves as the Button shows it: its transition, the
    // last part of its path ("saves: updateEnumType"), so a click is an informed approval.
    canvas.mutationSteps = (action) => (action && Array.isArray(action.steps) ? action.steps.filter((step) => step.type === "run" && step.refType === "mutation") : []);
    canvas.savesText = (path) => {
        const bare = String(path || "").split("?")[0].replace(/\/+$/, "");
        return bare ? bare.slice(bare.lastIndexOf("/") + 1) : "";
    };
    // A click that may run a Mutation: a real event of the user's (isTrusted) while the page has the user's activation.
    canvas.trustedClick = (event, nav) => !!(event && event.isTrusted === true && (!nav || !nav.userActivation || nav.userActivation.isActive));

    if (typeof module !== "undefined" && module.exports) module.exports = canvas;
    const Vue = root.Vue;
    if (!Vue || !Vue.h) return;

    // ---- the renderer (a browser with Vue 3) ----

    const { h, resolveComponent, reactive, markRaw } = Vue;
    // the key of a Mutation's token: a Symbol, which no program can write, so a Query never reaches a Mutation
    const RUN = Symbol("assist.canvas.run");
    const R = (name) => resolveComponent(name);
    const GAP = { s: "q-gutter-xs", m: "q-gutter-sm", l: "q-gutter-md" };
    const COLOR = { primary: "primary", positive: "positive", negative: "negative", warning: "warning", info: "info", grey: "grey-7", secondary: "secondary" };
    const BANNER = { info: "bg-blue-1 text-blue-10", warning: "bg-orange-1 text-orange-10", positive: "bg-green-1 text-green-10", negative: "bg-red-1 text-red-10" };

    // How each component is drawn: view(ctx, props, node) answers a vnode. ctx: render (a child value), field (a control's
    // value and setter), act (a Button's action), local (a reactive map for tabs and steppers), formName.
    const VIEWS = {
        Stack(ctx, props) {
            const row = props.direction === "row";
            const wrap = props.wrap === true || props.wrap === "true";
            const kids = ctx.list(props.children);
            return h("div", { class: ["m-assist-oui-stack", row ? "row items-start" : "column", row ? (wrap ? "wrap" : "no-wrap") : "", GAP[props.gap] || "q-gutter-sm"] },
                kids.map((kid, index) => h("div", { key: index, class: row ? "col" : "" }, [kid])));
        },
        Card: (ctx, props) => h(R("q-card"), { flat: true, bordered: true, class: "m-assist-oui-card" }, { default: () => [h(R("q-card-section"), null, { default: () => ctx.list(props.children) })] }),
        Separator: (ctx, props) => h(R("q-separator"), { inset: props.inset === true, class: "q-my-sm" }),
        Accordion: (ctx, props) => h(R("q-list"), { bordered: true, class: "rounded-borders" }, { default: () => ctx.list(props.children) }),
        AccordionItem: (ctx, props) => h(R("q-expansion-item"), { label: textOf(props.title), dense: true }, { default: () => [h("div", { class: "q-pa-sm" }, ctx.list(props.content))] }),
        Steps(ctx, props, node) {
            const items = asArray(props.children).map(nodeProps);
            const key = "steps:" + (node.statementId || "");
            const current = ctx.local[key] || (items[0] && textOf(items[0].value));
            return h(R("q-stepper"), { modelValue: current, "onUpdate:modelValue": (value) => { ctx.local[key] = value; }, vertical: true, flat: true, animated: true },
                { default: () => items.map((item, index) => h(R("q-step"), { key: index, name: textOf(item.value), title: textOf(item.title), done: false, headerNav: true },
                    { default: () => ctx.list(item.content) })) });
        },
        Modal(ctx, props) {
            const name = "$" + textOf(props.name);
            return h(R("q-dialog"), { modelValue: !!unwrap(ctx.getState(name)), "onUpdate:modelValue": (value) => ctx.setState(name, !!value) }, {
                default: () => [h(R("q-card"), { style: "min-width:320px;max-width:90vw;" }, { default: () => [
                    h(R("q-card-section"), { class: "row items-center q-pb-none" }, { default: () => [h("div", { class: "text-h6" }, textOf(props.title)), h(R("q-space")),
                        h(R("q-btn"), { icon: "sym_o_close", flat: true, round: true, dense: true, onClick: () => ctx.setState(name, false) })] }),
                    h(R("q-card-section"), null, { default: () => ctx.list(props.children) })] })],
            });
        },
        Tabs(ctx, props, node) {
            const items = asArray(props.children).map(nodeProps);
            const key = "tabs:" + (node.statementId || "");
            const current = ctx.local[key] || (items[0] && textOf(items[0].value));
            const set = (value) => { ctx.local[key] = value; };
            return h("div", null, [
                h(R("q-tabs"), { modelValue: current, "onUpdate:modelValue": set, dense: true, align: "left", class: "text-primary", noCaps: true },
                    { default: () => items.map((item, index) => h(R("q-tab"), { key: index, name: textOf(item.value), label: textOf(item.trigger) })) }),
                h(R("q-separator")),
                h(R("q-tab-panels"), { modelValue: current, "onUpdate:modelValue": set, animated: false },
                    { default: () => items.map((item, index) => h(R("q-tab-panel"), { key: index, name: textOf(item.value), class: "q-px-none" }, { default: () => ctx.list(item.content) })) }),
            ]);
        },
        TabItem: (ctx, props) => h("div", null, ctx.list(props.content)),
        StepsItem: (ctx, props) => h("div", null, ctx.list(props.content)),
        CardHeader: (ctx, props) => h("div", { class: "q-mb-sm" }, [h("div", { class: "text-h6" }, textOf(props.title)),
            props.subtitle ? h("div", { class: "text-caption m-muted" }, textOf(props.subtitle)) : null]),
        TextContent: (ctx, props) => h("div", { class: props.size === "small" ? "text-caption" : props.size === "large" ? "text-subtitle1" : "text-body2" }, textOf(props.text)),
        Callout: (ctx, props) => h(R("q-banner"), { dense: true, rounded: true, class: ["q-my-xs", BANNER[props.type] || BANNER.info] }, { default: () => textOf(props.text) }),
        Stat: (ctx, props) => h(R("q-card"), { flat: true, bordered: true, class: "m-assist-oui-stat m-number-card" }, { default: () => [h(R("q-card-section"), { class: "q-pa-md" }, { default: () => [
            h("div", { class: "text-overline ellipsis" }, textOf(props.label)),
            h("div", { class: "text-h5" }, props.value === null || props.value === undefined || props.value === "" ? "–" : textOf(props.value)),
            props.caption ? h("div", { class: "text-caption m-muted" }, textOf(props.caption)) : null] })] }),
        Tag: (ctx, props) => h(R("q-chip"), { dense: true, square: true, size: props.size === "md" ? "md" : "sm", color: COLOR[props.color] || undefined,
            textColor: COLOR[props.color] ? "white" : undefined }, { default: () => textOf(props.label) }),
        ListBlock: (ctx, props) => h(R("q-list"), { dense: true, separator: true }, { default: () => ctx.list(props.children) }),
        ListItem: (ctx, props) => h(R("q-item"), null, { default: () => [
            h(R("q-item-section"), null, { default: () => [h(R("q-item-label"), null, { default: () => textOf(props.title) }),
                props.caption ? h(R("q-item-label"), { caption: true }, { default: () => textOf(props.caption) }) : null] }),
            props.side !== null && props.side !== undefined ? h(R("q-item-section"), { side: true }, { default: () => ctx.list(props.side) }) : null] }),
        Image(ctx, props) {
            const src = canvas.siteImage(props.src);
            return src ? h("img", { src, alt: textOf(props.alt), style: "max-width:100%;" }) : h("div", { class: "text-caption m-muted" }, "[image: " + textOf(props.alt || props.src) + "]");
        },
        // the drawer's own Markdown in its strict mode: no image of any kind and no link but a checked path of the apps, so
        // data in the text cannot reach another site (m-markdown keeps remote images)
        MarkDownRenderer: (ctx, props) => h(R("m-assist-markdown"), { text: textOf(props.textMarkdown), strict: true,
            class: props.variant === "card" ? "q-pa-sm bordered rounded-borders" : "" }),
        Mermaid: (ctx, props) => h(R("m-mermaid"), { text: textOf(props.text), height: canvas.cssLength(props.height, "300px") }),
        CodeBlock: (ctx, props) => h("pre", { class: "m-assist-oui-code q-pa-sm rounded-borders" }, [h("code", null, textOf(props.code))]),
        Table(ctx, props) {
            const cols = asArray(props.columns).map(nodeProps);
            const count = cols.reduce((max, col) => Math.max(max, asArray(col.values).length), 0);
            const rows = [];
            for (let i = 0; i < Math.min(count, 500); i++) rows.push(i);
            return h(R("q-markup-table"), { dense: true, flat: true, bordered: true, separator: "horizontal", class: "m-assist-oui-table" }, { default: () => [
                h("thead", null, [h("tr", null, cols.map((col, c) => h("th", { key: c, class: "text-left" }, textOf(col.label))))]),
                h("tbody", null, rows.map((row) => h("tr", { key: row }, cols.map((col, c) => {
                    const value = asArray(col.values)[row];
                    return h("td", { key: c }, value && typeof value === "object" ? ctx.list(value) : textOf(value));
                })))),
            ] });
        },
        Col: () => null,
        Series: () => null,
        BarChart: (ctx, props) => h(R("m-chart"), { config: canvas.xyChart("bar", props), height: "300px" }),
        LineChart: (ctx, props) => h(R("m-chart"), { config: canvas.xyChart("line", props), height: "300px" }),
        AreaChart: (ctx, props) => h(R("m-chart"), { config: canvas.xyChart("line", props, true), height: "300px" }),
        PieChart: (ctx, props) => h(R("m-chart"), { config: canvas.pieChart(props), height: "300px" }),
        Form(ctx, props) {
            const name = textOf(props.name);
            const inner = ctx.within(name);
            return h("div", { class: "m-assist-oui-form column q-gutter-sm", "data-assist-canvas-form": name },
                inner.list(props.children).concat(props.submit ? [h("div", { class: "row q-gutter-sm" }, inner.list(props.submit))] : []));
        },
        FormControl: (ctx, props) => h("div", { class: "m-assist-oui-control" }, [h("div", { class: "text-caption m-muted" }, textOf(props.label))].concat(ctx.list(props.control))),
        Input: (ctx, props) => { const f = ctx.field(props); return h(R("m-text-line"), { name: f.name, label: textOf(props.label) || undefined, modelValue: textOf(f.value), "onUpdate:modelValue": f.set, placeholder: textOf(props.placeholder) || undefined }); },
        TextArea: (ctx, props) => { const f = ctx.field(props); return h(R("m-text-area"), { name: f.name, label: textOf(props.label) || undefined, modelValue: textOf(f.value), "onUpdate:modelValue": f.set }); },
        Select(ctx, props) {
            const f = ctx.field(props);
            const options = asArray(props.items).map(nodeProps).map((item) => ({ value: textOf(item.value), label: textOf(item.label || item.value) }));
            return h(R("q-select"), { dense: true, outlined: true, emitValue: true, mapOptions: true, stackLabel: true, optionsDense: true, label: textOf(props.label) || undefined,
                modelValue: unwrap(f.value) === null || unwrap(f.value) === undefined ? null : textOf(f.value), options, "onUpdate:modelValue": f.set, style: "min-width:180px;", name: f.name });
        },
        SelectItem: () => null,
        RadioItem: () => null,
        CheckBox: (ctx, props) => { const f = ctx.field(props); return h(R("q-checkbox"), { label: textOf(props.label) || f.name, modelValue: !!unwrap(f.value), "onUpdate:modelValue": f.set, dense: true }); },
        Switch: (ctx, props) => { const f = ctx.field(props); return h(R("q-toggle"), { label: textOf(props.label) || f.name, modelValue: !!unwrap(f.value), "onUpdate:modelValue": f.set, dense: true }); },
        RadioGroup(ctx, props) {
            const f = ctx.field(props);
            const options = asArray(props.items).map(nodeProps).map((item) => ({ value: textOf(item.value), label: textOf(item.label || item.value) }));
            return h("div", null, [props.label ? h("div", { class: "text-caption m-muted" }, textOf(props.label)) : null,
                h(R("q-option-group"), { type: "radio", dense: true, inline: true, options, modelValue: textOf(f.value), "onUpdate:modelValue": f.set })]);
        },
        Slider(ctx, props) {
            const f = ctx.field(props);
            const min = isFinite(parseFloat(props.min)) ? parseFloat(props.min) : 0;
            const max = isFinite(parseFloat(props.max)) ? parseFloat(props.max) : 100;
            const value = isFinite(parseFloat(unwrap(f.value))) ? parseFloat(unwrap(f.value)) : min;
            return h("div", { class: "q-px-sm" }, [h("div", { class: "text-caption m-muted" }, textOf(props.label || f.name) + " (" + value + ")"),
                h(R("q-slider"), { modelValue: value, min, max, label: true, "onUpdate:modelValue": f.set })]);
        },
        DateTime(ctx, props) {
            const f = ctx.field(props);
            const type = props.type === "date" || props.type === "time" ? props.type : "date-time";
            return h(R("m-date-time"), { name: f.name, type, label: textOf(props.label) || undefined, modelValue: canvas.dateValue(unwrap(f.value), type), "onUpdate:modelValue": f.set });
        },
        DatePeriod(ctx, props) {
            const name = textOf(props.name) || "period";
            const fields = ctx.periodFields(name);
            return h(R("m-date-period"), { name, label: textOf(props.label) || name, fields, fromThruType: props.type === "date-time" ? "date-time" : "date" });
        },
        Lookup: (ctx, props) => h(LookupView, { ctx: markRaw(ctx), props }),
        Display: (ctx, props) => h(DisplayView, { ctx: markRaw(ctx), props }),
        Button(ctx, props) {
            const color = props.color === "negative" ? "negative" : props.color === "secondary" ? "secondary" : "primary";
            const busy = ctx.busy[ctx.actionKey(props.action)] === true;
            const button = h(R("q-btn"), { label: textOf(props.label), color, unelevated: color !== "secondary", outline: color === "secondary", noCaps: true, loading: busy,
                "data-assist-canvas-button": textOf(props.label), onClick: (event) => ctx.act(props.action, textOf(props.label), event) });
            // a Button that saves says what, beside the label the model gave it
            const saves = ctx.saves(props.action);
            if (!saves.length) return button;
            return h("span", { class: "inline-flex items-center no-wrap" }, [button, h("span", { class: "text-caption m-muted q-ml-sm m-assist-oui-saves",
                "data-assist-canvas-saves": saves.join(" "), title: saves.join(", ") }, "saves: " + saves.map(canvas.savesText).join(", "))]);
        },
        Buttons: (ctx, props) => h("div", { class: "row q-gutter-sm" }, ctx.list(props.children)),
        Link(ctx, props) {
            const params = props.params && typeof props.params === "object" && !Array.isArray(props.params) ? props.params : null;
            const flat = {};
            Object.keys(params || {}).forEach((key) => { const value = unwrap(params[key]); if (value !== null && value !== undefined && value !== "") flat[key] = textOf(value); });
            const checked = moqui.assist && moqui.assist.safePath ? moqui.assist.safePath(textOf(props.path), flat, { linkBase: "/qapps" }) : { ok: false };
            if (!checked.ok) return h("span", { class: "m-muted", title: checked.error || "" }, textOf(props.label));
            return h("a", { href: checked.url, class: "m-link-text", "data-assist-canvas-link": checked.path,
                onClick: (event) => { if (event.ctrlKey || event.metaKey || event.shiftKey) return; event.preventDefault(); ctx.navigate(checked.url); } }, textOf(props.label));
        },
    };

    // A Lookup: a q-select whose options come from the read proxy as the user types (an /apps transition that answers options).
    const LookupView = {
        name: "mAssistOuiLookup",
        props: { ctx: Object, props: Object },
        data() { return { options: [], loading: false }; },
        methods: {
            search(text, update, abort) {
                const props = this.props || {};
                const extra = {};
                const depends = props.dependsOn;
                const map = typeof depends === "string" ? { [depends]: depends } : depends && typeof depends === "object" ? depends : {};
                Object.keys(map).forEach((param) => { const value = unwrap(this.ctx.getState("$" + map[param])); if (value !== null && value !== undefined) extra[param] = value; });
                extra.term = text;
                this.loading = true;
                this.ctx.read({ path: props.optionsUrl, query: extra }).then((json) => {
                    const list = Array.isArray(json) ? json : json && Array.isArray(json.options) ? json.options : json && Array.isArray(json.rows) ? json.rows : [];
                    const vf = props.valueField || "value";
                    const lf = props.labelField || "label";
                    this.loading = false;
                    update(() => { this.options = list.slice(0, 100).map((one) => ({ value: textOf(one[vf]), label: textOf(one[lf] || one[vf]) })); });
                }, () => { this.loading = false; abort(); });
            },
        },
        render() {
            const f = this.ctx.field(this.props);
            return h(R("q-select"), { dense: true, outlined: true, useInput: true, fillInput: false, emitValue: true, mapOptions: true, stackLabel: true, inputDebounce: 300,
                label: textOf(this.props.label) || undefined, options: this.options, loading: this.loading, modelValue: unwrap(f.value) === undefined ? null : unwrap(f.value),
                onFilter: this.search, "onUpdate:modelValue": f.set, style: "min-width:200px;", name: f.name });
        },
    };
    // A Display: its text, or with valueUrl the text the read proxy answers.
    const DisplayView = {
        name: "mAssistOuiDisplay",
        props: { ctx: Object, props: Object },
        data() { return { fetched: null }; },
        mounted() {
            const url = this.props && this.props.valueUrl;
            if (url) this.ctx.read({ path: url, query: {} }).then((json) => {
                const one = Array.isArray(json) ? json[0] : json;
                this.fetched = one && typeof one === "object" ? textOf(one.label || one.value) : textOf(one);
            }, () => {});
        },
        render() {
            const text = this.fetched !== null ? this.fetched : textOf(this.props.text);
            if (!this.props.label) return h("span", { class: "m-assist-oui-display" }, text);
            return h(R("q-field"), { dense: true, borderless: true, readonly: true, stackLabel: true, label: textOf(this.props.label) },
                { control: () => h("div", { class: "self-center full-width" }, text) });
        },
    };

    // One element of the evaluated tree.
    const NodeView = {
        name: "mAssistOuiNode",
        props: { node: Object, ctx: Object },
        render() {
            const node = this.node;
            if (!node || node.type !== "element") return null;
            const view = VIEWS[node.typeName];
            if (!view) return h("div", { class: "text-caption text-negative" }, "Unknown component: " + node.typeName);
            return view(this.ctx, node.props || {}, node);
        },
    };

    // The program: parse (whole or streaming), state, queries through the proxy, actions, problems.
    components_register();
    function components_register() {
        const component = {
            name: "mAssistOpenui",
            props: { program: String, streaming: Boolean, artifactId: String, version: Number, initialState: Object, readOnly: Boolean },
            emits: ["problems", "state", "action", "navigate"],
            data() { return { evaluated: null, problems: [], tick: 0, local: reactive({}), busy: reactive({}), periods: {}, failed: null }; },
            created() {
                const O = root.OpenUILang;
                if (!O) { this.failed = "OpenUI Lang is not loaded"; return; }
                this.O = O;
                this.parser = O.createStreamingParser(canvas.librarySpec(), "Stack");
                this.store = O.createStore();
                this.qm = O.createQueryManager({ callTool: (tool, args) => this.callTool(tool, args) });
                this.qm.activate();
                this.libraryShape = { components: {} };
                LIBRARY.forEach((def) => { this.libraryShape.components[def.name] = { props: {} }; });
                this.userEdited = {};
                this.lastDecls = null;
                this.lastProblemKey = "";
                this.offStore = this.store.subscribe(() => { this.tick++; this.$emit("state", canvas.valuesOf(this.store.getSnapshot())); if (!this.streaming) this.reeval(); });
                this.offQm = this.qm.subscribe(() => { if (!this.streaming) this.reeval(); });
                this.reparse();
            },
            beforeUnmount() {
                if (this.offStore) this.offStore();
                if (this.offQm) this.offQm();
                if (this.qm) this.qm.dispose();
                if (this.store) this.store.dispose();
            },
            watch: {
                streaming(value) { if (!value) { this.lastProblemKey = ""; this.reparse(); } },
                program() { this.reparse(); },
            },
            methods: {
                getState(name) { return this.store ? unwrap(this.store.get(name)) : undefined; },
                setState(name, value) {
                    if (!this.store) return;
                    const decls = (this.result && this.result.stateDeclarations) || {};
                    if (has(decls, name) && isScalar(decls[name]) && !sameLiteral(value, decls[name])) this.userEdited[name] = true;
                    this.store.set(name, value);
                },
                // Query("request", {path, list, query}) through the read proxy; a write never goes from the page
                // A Query reads; only act() runs a Mutation, through a token no program can write (a Symbol key), with the nonce
                // of the click that runs it.
                callTool(tool, args) {
                    if (tool !== "request") return Promise.reject(new Error("the only tool is request"));
                    if (args && args[RUN]) return this.runMutationCall(args[RUN]);
                    return this.read(args);
                },
                read(args) {
                    const params = canvas.readParams(this.artifactId, args);
                    if (params.error) return Promise.reject(new Error(params.error));
                    return moqui.assist.call("read", params).then((json) => json, (err) => { throw new Error(err.errors || "the read failed"); });
                },
                // the Mutation a click runs: the server evaluates it from the stored program with this state and runs it
                runMutationCall(token) {
                    return new Promise((resolve, reject) => {
                        this.$emit("action", { kind: "mutation", statement: token.statement, nonce: token.nonce, state: canvas.valuesOf(this.store.getSnapshot()), activation: true,
                            done: (json) => {
                                const problems = canvas.responseProblems(json && json.data);
                                if (json && json.ok === false) reject(new Error(json.error || problems.errors.join("\n") || "the write failed"));
                                else if (problems.errors.length) reject(new Error(problems.errors.join("\n")));
                                else resolve(json ? json.data : null);
                            },
                            failed: (error) => reject(new Error(error || "the write failed")) });
                    });
                },
                evaluation() {
                    const store = this.store;
                    const qm = this.qm;
                    const O = this.O;
                    const evaluating = {};
                    const ctx = {
                        getState: (name) => {
                            if (evaluating[name]) return null;
                            const value = store ? unwrap(store.get(name)) : undefined;
                            if (value && typeof value === "object" && value.k && O.evaluate) {
                                evaluating[name] = true;
                                try { return O.evaluate(value, ctx); } catch (err) { return null; } finally { delete evaluating[name]; }
                            }
                            return value;
                        },
                        resolveRef: (name) => qm.getMutationResult(name) || qm.getResult(name),
                    };
                    return ctx;
                },
                reparse() {
                    if (!this.parser) return;
                    let result;
                    try { result = this.parser.set(this.program || ""); } catch (err) {
                        this.evaluated = null;
                        this.report([{ source: "parser", code: "parse-exception", message: String((err && err.message) || err) }]);
                        return;
                    }
                    this.result = result;
                    const decls = result.stateDeclarations || {};
                    const current = {};
                    Object.keys(decls).forEach((name) => { current[name] = this.getState(name); });
                    this.userEdited = canvas.reconcileUserEdits(this.userEdited, this.lastDecls, decls, current);
                    const persisted = canvas.persistedState(decls, this.lastDecls, this.lastDecls ? canvas.valuesOf(this.store.getSnapshot()) : this.initialState);
                    const key = JSON.stringify(decls) + "::" + JSON.stringify(persisted);
                    if (key !== this.storeKey) { this.storeKey = key; this.store.initialize(decls, persisted); }
                    this.lastDecls = decls;
                    this.reeval();
                },
                reeval() {
                    const res = this.result;
                    if (!res) return;
                    const O = this.O;
                    if (!this.streaming) {
                        const ctx = this.evaluation();
                        const evaluate = (ast) => { try { return ast ? O.evaluate(ast, ctx) : null; } catch (err) { return null; } };
                        this.qm.evaluateQueries((res.queryStatements || []).map((q) => ({ statementId: q.statementId, toolName: evaluate(q.toolAST) || "", args: evaluate(q.argsAST),
                            defaults: evaluate(q.defaultsAST), refreshInterval: q.refreshAST ? evaluate(q.refreshAST) : undefined, deps: q.deps, complete: q.complete })));
                        this.qm.registerMutations((res.mutationStatements || []).map((m) => ({ statementId: m.statementId, toolName: evaluate(m.toolAST) || "" })));
                    }
                    const runtime = [];
                    if (res.root) {
                        try { this.evaluated = O.evaluateElementProps(res.root, { ctx: this.evaluation(), library: this.libraryShape, store: this.store, errors: runtime }); }
                        catch (err) { runtime.push({ source: "runtime", code: "runtime-error", message: String((err && err.message) || err) }); this.evaluated = null; }
                    } else this.evaluated = null;
                    const snapshot = this.qm.getSnapshot ? this.qm.getSnapshot() : {};
                    this.report(canvas.problemsOf(res, runtime, snapshot.__openui_errors || []));
                },
                // the problems once a render settles (none while it streams), each distinct list once
                report(problems) {
                    this.problems = problems;
                    if (this.streaming) return;
                    const key = JSON.stringify(problems.map((one) => [one.code, one.statementId, one.message]));
                    if (key === this.lastProblemKey) return;
                    this.lastProblemKey = key;
                    if (problems.length) this.$emit("problems", problems);
                },
                actionKey(action) { return action && action.steps ? JSON.stringify(action.steps.map((step) => step.statementId || step.type)) : "submit"; },
                // A Button's Action plan, step by step: a Mutation is run by the server (the click is the approval); a Query
                // fetches again; @Set and @Reset change state; @ToAssistant sends the canvas's values to the agent. A Button
                // with no plan submits its form's values to the agent.
                act(action, label, event, formName) {
                    if (this.readOnly) return;
                    const O = this.O;
                    const values = () => canvas.valuesOf(this.store.getSnapshot());
                    // the click, read now and used once: its nonces run this plan's Mutations, one each
                    const mutations = canvas.mutationSteps(action);
                    if (mutations.length) {
                        if (!canvas.trustedClick(event, root.navigator)) return;
                        const key = this.actionKey(action);
                        this.busy[key] = true;
                        this.$emit("action", { kind: "arm", count: mutations.length,
                            done: (nonces) => this.runPlan(action, label, formName, (nonces || []).slice()),
                            failed: () => { this.busy[key] = false; } });
                        return;
                    }
                    this.runPlan(action, label, formName, []);
                },
                runPlan(action, label, formName, nonces) {
                    const O = this.O;
                    const values = () => canvas.valuesOf(this.store.getSnapshot());
                    if (!action || !action.steps) {
                        this.$emit("action", { kind: "continue", message: label || "Submit", form: formName || null, values: values() });
                        return;
                    }
                    const key = this.actionKey(action);
                    const steps = action.steps.slice();
                    const next = () => {
                        const step = steps.shift();
                        if (!step) { this.busy[key] = false; return; }
                        if (step.type === "run" && step.refType === "mutation") {
                            this.busy[key] = true;
                            // the arguments are the server's to evaluate: the page only names the Mutation, with a nonce of the click
                            const nonce = nonces.shift();
                            if (!nonce) { this.busy[key] = false; return; }
                            this.qm.fireMutation(step.statementId, { [RUN]: { statement: step.statementId, nonce } }).then((ok) => { if (ok) next(); else this.busy[key] = false; });
                        } else if (step.type === "run") { this.qm.invalidate([step.statementId]); next(); }
                        else if (step.type === "continue_conversation") {
                            this.$emit("action", { kind: "continue", message: step.message || label || "Continue", context: step.context || null, form: formName || null, values: values() });
                            next();
                        } else if (step.type === "set") {
                            if (step.valueAST) this.store.set(step.target, O.evaluate(step.valueAST, this.evaluation()));
                            next();
                        } else if (step.type === "reset") {
                            const decls = (this.result && this.result.stateDeclarations) || {};
                            (step.targets || []).forEach((target) => this.store.set(target, decls[target] !== undefined ? decls[target] : null));
                            next();
                        } else next();
                    };
                    next();
                },
                // the paths the Mutations of a plan post to, evaluated from the program for the Button's mark
                saves(action) {
                    const O = this.O;
                    const list = (this.result && this.result.mutationStatements) || [];
                    return canvas.mutationSteps(action).map((step) => {
                        const found = list.find((m) => m.statementId === step.statementId);
                        let args = null;
                        try { args = found && found.argsAST ? O.evaluate(found.argsAST, this.evaluation()) : null; } catch (err) { args = null; }
                        return args && args.path ? String(args.path) : step.statementId;
                    });
                },
                periodFields(name) {
                    if (!this.periods[name]) {
                        const fields = reactive({});
                        ["_period", "_poffset", "_pdate", "_from", "_thru"].forEach((suffix) => {
                            const value = this.getState("$" + name + suffix);
                            fields[name + suffix] = value === undefined ? null : value;
                        });
                        this.$watch(() => Object.assign({}, fields), (now) => { Object.keys(now).forEach((key) => { if (!sameLiteral(this.getState("$" + key), now[key])) this.setState("$" + key, now[key]); }); });
                        this.periods[name] = fields;
                    }
                    return this.periods[name];
                },
                ctxFor(formName) {
                    const vm = this;
                    const ctx = {
                        formName,
                        local: vm.local,
                        busy: vm.busy,
                        getState: (name) => vm.getState(name),
                        setState: (name, value) => vm.setState(name, value),
                        read: (args) => vm.read(args),
                        navigate: (url) => vm.$emit("navigate", url),
                        actionKey: (action) => vm.actionKey(action),
                        saves: (action) => vm.saves(action),
                        act: (action, label, event) => vm.act(action, label, event, formName),
                        periodFields: (name) => vm.periodFields(name),
                        within: (name) => vm.ctxFor(name),
                        // a control's name and value: the $name of its second argument, else its name
                        field(props) {
                            const name = textOf(props.name);
                            const dollar = "$" + name;
                            const stored = vm.getState(dollar);
                            const value = stored !== undefined && stored !== null ? stored : unwrap(props.value);
                            return { name, value: value === undefined ? null : value, set: (next) => vm.setState(dollar, next) };
                        },
                        list(value) {
                            const out = [];
                            const add = (item) => {
                                if (item === null || item === undefined || item === false) return;
                                if (Array.isArray(item)) { item.forEach(add); return; }
                                if (typeof item === "object" && item.type === "element") out.push(h(NodeView, { key: item.statementId || out.length, node: item, ctx }));
                                else if (typeof item !== "object") out.push(String(item));
                            };
                            add(value);
                            return out;
                        },
                    };
                    return ctx;
                },
            },
            render() {
                void this.tick;
                if (this.failed) return h("div", { class: "text-negative q-pa-md" }, this.failed);
                const ctx = this.ctxFor(null);
                return h("div", { class: "m-assist-openui", "data-assist-canvas-version": String(this.version || "") },
                    this.evaluated ? [h(NodeView, { node: this.evaluated, ctx })] : [h("div", { class: "m-muted q-pa-md" }, this.streaming ? "…" : "")]);
            },
        };
        canvas.component = component;
        const qvt = moqui.qvt;
        if (qvt && qvt.registerLate) qvt.registerLate("m-assist-openui", component);
        if (moqui.assist && moqui.assist.dialects) moqui.assist.dialects.register("openui", { label: "OpenUI Lang", component: "m-assist-openui", streaming: true });
    }
    if (moqui.assist) moqui.assist.canvas = canvas;
})(globalThis);
