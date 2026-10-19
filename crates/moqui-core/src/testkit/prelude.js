// The globals of a spec file: spec(), feature(), the helpers, and what the power-assert rewrite calls. The harness
// (testkit/run.rs) evaluates this before each spec file and drives the spec through __moquiTest.
(function (g) {
    // A plain assignment to a global goes through the context stack's proxy, which takes only data; these are defined.
    const define = (name, value) => Object.defineProperty(g, name, { value, writable: true, configurable: true });
    const SPEC_FIELDS = ["dependsOn", "requires", "stepwise", "allowMessageErrors", "timeout", "tempSequences",
        "setupSpec", "cleanupSpec", "setup", "cleanup"];
    const FEATURE_FIELDS = ["given", "when", "then", "expect", "cleanup", "where", "ignore", "requires", "timeout",
        "allowMessageErrors"];
    const REWRITTEN = "$pa";

    const T = {};
    define("__moquiTest", T);
    T.reset = (file) => {
        T.file = file;
        T.spec = null;
        T.features = [];
        T.errors = [];
        T.conditions = [];
        T.cases = [];
        T.tempSequences = [];
        T.dataErrors = [];
    };
    T.reset(null);
    T.setConditions = (conditions) => { T.conditions = conditions; };

    class ConditionNotSatisfied extends Error {
        constructor(message, line) {
            super(message);
            this.name = "ConditionNotSatisfied";
            this.line = line;
        }
    }

    // ---- registration ----

    define("spec", (opts) => {
        if (T.spec) T.errors.push("spec() is called more than once");
        opts = opts || {};
        for (const key of Object.keys(opts)) if (!SPEC_FIELDS.includes(key)) T.errors.push(`spec() has an unknown field ${key}`);
        T.spec = opts;
    });

    define("feature", (name, def) => {
        if (typeof name !== "string" || !name) { T.errors.push("feature() needs a name"); return; }
        if (!def || typeof def !== "object") { T.errors.push(`feature "${name}" needs an object of blocks`); return; }
        for (const key of Object.keys(def)) {
            const field = key.endsWith(REWRITTEN) ? key.slice(0, -REWRITTEN.length) : key;
            if (!FEATURE_FIELDS.includes(field)) T.errors.push(`feature "${name}" has an unknown field ${key}`);
        }
        if (def.then || def.expect) T.errors.push(`feature "${name}": its then or expect block was not rewritten, so ` +
            "its conditions would not be checked; write feature(name, { ... }) as a statement at the top level of the " +
            "file, with the block a function in the object literal");
        const then = def["then" + REWRITTEN], expect = def["expect" + REWRITTEN];
        if (then && expect) T.errors.push(`feature "${name}" has both then and expect`);
        if (then && !def.when) T.errors.push(`feature "${name}" has then but no when`);
        if (def.when && !then) T.errors.push(`feature "${name}" has when but no then`);
        if (!then && !expect && !def.then && !def.expect) T.errors.push(`feature "${name}" has no then or expect block`);
        T.features.push({ name, def });
    });

    // Spock's @Unroll: `#field` in the name is the row's value.
    const unroll = (name, row) => name.replace(/#([A-Za-z_$][\w$]*)/g, (all, field) =>
        Object.prototype.hasOwnProperty.call(row, field) ? display(row[field], 60) : all);

    T.plan = () => {
        T.cases = [];
        T.features.forEach((feature, index) => {
            let rows = [null];
            const where = feature.def.where;
            if (where != null) {
                try {
                    rows = typeof where === "function" ? where() : where;
                } catch (e) {
                    T.errors.push(`feature "${feature.name}": where failed: ${e && e.message ? e.message : e}`);
                    rows = [];
                }
                if (!Array.isArray(rows)) {
                    T.errors.push(`feature "${feature.name}": where is not an array of rows`);
                    rows = [];
                }
            }
            for (const row of rows) {
                T.cases.push({ feature: index, row, name: row ? unroll(feature.name, row) : feature.name });
            }
        });
        const spec = T.spec || {};
        const ignoreOf = (value) => value ? (value === true ? "ignored" : String(value)) : null;
        return JSON.stringify({
            errors: T.errors,
            spec: {
                dependsOn: Array.from(spec.dependsOn || []).map(String), stepwise: !!spec.stepwise,
                allowMessageErrors: !!spec.allowMessageErrors, timeout: spec.timeout == null ? null : Number(spec.timeout)
            },
            cases: T.cases.map((c) => {
                const def = T.features[c.feature].def;
                return {
                    name: c.name, ignore: ignoreOf(def.ignore),
                    requires: [...Array.from(def.requires || []), ...Array.from((c.row && c.row.requires) || [])].map(String),
                    timeout: def.timeout == null ? null : Number(def.timeout), allowMessageErrors: !!def.allowMessageErrors
                };
            })
        });
    };

    // ---- running ----

    const lineOf = (e) => {
        if (!e || !e.stack || !T.file) return null;
        for (const line of String(e.stack).split("\n")) {
            const at = line.lastIndexOf(T.file + ":");
            if (at >= 0) {
                const number = parseInt(line.slice(at + T.file.length + 1), 10);
                if (!Number.isNaN(number)) return number;
            }
        }
        return null;
    };

    T.failureOf = (e, block) => {
        if (e instanceof ConditionNotSatisfied) return { kind: "assertion", block, message: e.message, line: e.line == null ? lineOf(e) : e.line, stack: "" };
        const message = e && e.message !== undefined ? `${e.name || "Error"}: ${e.message}` : String(e);
        return { kind: "error", block, message, line: lineOf(e), stack: e && e.stack ? String(e.stack) : "" };
    };

    const result = (failure) => JSON.stringify({ failure, dataErrors: T.dataErrors });

    T.runSpecPhase = (phase) => {
        T.dataErrors = [];
        const spec = T.spec || {};
        try {
            if (phase === "setupSpec") {
                const sequences = spec.tempSequences;
                if (sequences) {
                    const start = Number(sequences.start);
                    for (const [name, bank] of Object.entries(sequences.banks || {})) {
                        ec.entity.tempSetSequencedIdPrimary(name, start, Number(bank));
                        T.tempSequences.push(name);
                    }
                }
                if (spec.setupSpec) spec.setupSpec();
            } else if (phase === "cleanupSpec") {
                try {
                    if (spec.cleanupSpec) spec.cleanupSpec();
                } finally {
                    for (const name of T.tempSequences) ec.entity.tempResetSequencedIdPrimary(name);
                    T.tempSequences = [];
                }
            }
        } catch (e) {
            return result(T.failureOf(e, phase));
        }
        return result(null);
    };

    T.runCase = (index) => {
        T.dataErrors = [];
        const c = T.cases[index];
        const def = T.features[c.feature].def;
        const spec = T.spec || {};
        const ctx = Object.assign({}, c.row || {});
        let failure = null;
        const run = (block, fn, withCtx) => {
            if (!fn || failure) return;
            try {
                const out = withCtx ? fn(ctx) : fn();
                if (out && typeof out === "object" && !Array.isArray(out)) Object.assign(ctx, out);
            } catch (e) {
                failure = T.failureOf(e, block);
            }
        };
        run("setup", spec.setup, false);
        run("given", def.given, true);
        run("when", def.when, true);
        run("then", def["then" + REWRITTEN], true);
        run("expect", def["expect" + REWRITTEN], true);
        // cleanup blocks run whatever happened before them
        for (const [block, fn, withCtx] of [["cleanup", def.cleanup, true], ["spec cleanup", spec.cleanup, false]]) {
            if (!fn) continue;
            try {
                withCtx ? fn(ctx) : fn();
            } catch (e) {
                if (!failure) failure = T.failureOf(e, block);
            }
        }
        return result(failure);
    };

    // ---- Groovy semantics for conditions ----

    const isDecimal = (v) => v != null && typeof v === "object" && typeof v.compare === "function" &&
        typeof v.setScale === "function" && typeof v.isZero === "function";
    const isHostList = (v) => v != null && typeof v === "object" && typeof v.size === "function" &&
        typeof v.getPartialList === "function";
    const isEntity = (v) => v != null && typeof v === "object" && typeof v.getMap === "function" &&
        typeof v.getPrimaryKeys === "function";
    const isPlain = (v) => v != null && typeof v === "object" && (Object.getPrototypeOf(v) === Object.prototype ||
        Object.getPrototypeOf(v) === null);
    const NUMBER_TEXT = /^\s*[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?\s*$/;
    const asDecimal = (v) => {
        if (isDecimal(v)) return v;
        if (typeof v === "number" && Number.isFinite(v)) return toDecimal(String(v));
        if (typeof v === "string" && NUMBER_TEXT.test(v)) return toDecimal(v.trim());
        return null;
    };
    const listOf = (v) => Array.isArray(v) ? v : isHostList(v) ? Array.from(v) : null;
    const mapOf = (v) => isEntity(v) ? v.getMap() : isPlain(v) ? v : null;

    // Groovy truth: null, false, zero, and an empty string, list or map are false.
    const truthy = (v) => {
        if (v === null || v === undefined) return false;
        if (typeof v === "boolean") return v;
        if (typeof v === "number") return v !== 0 && !Number.isNaN(v);
        if (typeof v === "string") return v.length > 0;
        if (Array.isArray(v)) return v.length > 0;
        if (isDecimal(v)) return !v.isZero();
        if (isHostList(v)) return v.size() > 0;
        if (v instanceof Map || v instanceof Set) return v.size > 0;
        if (isPlain(v)) return Object.keys(v).length > 0;
        return true;
    };
    define("__truthy", truthy);

    // Groovy ==: numbers, decimals and number text by value (16.99 == 16.990), lists and maps by their items.
    const equal = (a, b) => {
        if (a === b) return true;
        if (a == null || b == null) return a == null && b == null;
        const numberish = (v) => isDecimal(v) || typeof v === "number";
        if (numberish(a) || numberish(b)) {
            const x = asDecimal(a), y = asDecimal(b);
            return x !== null && y !== null && x.compare(y) === 0;
        }
        const la = listOf(a), lb = listOf(b);
        if (la && lb) return la.length === lb.length && la.every((item, i) => equal(item, lb[i]));
        const ma = mapOf(a), mb = mapOf(b);
        if (ma && mb) {
            const ka = Object.keys(ma), kb = Object.keys(mb);
            return ka.length === kb.length && ka.every((key) => Object.prototype.hasOwnProperty.call(mb, key) && equal(ma[key], mb[key]));
        }
        return false;
    };
    const order = (a, b) => {
        const x = asDecimal(a), y = asDecimal(b);
        if (x !== null && y !== null && (isDecimal(a) || isDecimal(b) || typeof a === "number" || typeof b === "number")) {
            return x.compare(y);
        }
        if (a < b) return -1;
        if (a > b) return 1;
        if (a == b) return 0; // eslint-disable-line eqeqeq
        throw new Error(`cannot compare ${display(a, 40)} with ${display(b, 40)}`);
    };
    define("__cmp", (op, a, b) => {
        switch (op) {
            case "==": return equal(a, b);
            case "!=": return !equal(a, b);
            case "<": return order(a, b) < 0;
            case "<=": return order(a, b) <= 0;
            case ">": return order(a, b) > 0;
            case ">=": return order(a, b) >= 0;
            default: throw new Error(`unknown comparison ${op}`);
        }
    });

    // ---- the power-assert diagram ----

    const clip = (text, max) => text.length > max ? text.slice(0, max - 3) + "..." : text;
    const plain = (v, depth) => {
        if (depth > 6) return "...";
        if (isDecimal(v)) return v.toString();
        if (isEntity(v)) return plain(v.getMap(), depth + 1);
        const list = listOf(v);
        if (list) return list.map((item) => plain(item, depth + 1));
        if (v instanceof Set) return Array.from(v).map((item) => plain(item, depth + 1));
        if (isPlain(v)) {
            const out = {};
            for (const key of Object.keys(v)) out[key] = plain(v[key], depth + 1);
            return out;
        }
        return v;
    };
    const display = (v, max) => {
        if (v === undefined) return "undefined";
        if (v === null) return "null";
        if (typeof v === "string") return clip(JSON.stringify(v), max);
        if (typeof v === "number" || typeof v === "boolean") return String(v);
        if (typeof v === "function") return "function";
        if (isDecimal(v)) return v.toString();
        try {
            return clip(JSON.stringify(plain(v, 0)), max);
        } catch (e) {
            return clip(String(v), max);
        }
    };
    T.display = display;

    const diagram = (condition, seen) => {
        const entries = [];
        const used = new Set();
        condition.slots.forEach(([col, part], slot) => {
            if (!seen.has(slot) || used.has(col)) return;
            used.add(col);
            entries.push({ col, part, value: display(seen.get(slot), 100) });
        });
        const head = "Condition not satisfied:\n\n";
        if (condition.text.includes("\n") || !entries.length) {
            const lines = entries.sort((a, b) => a.col - b.col).map((e) => `  ${e.part}  ->  ${e.value}`);
            return head + condition.text + (lines.length ? "\n\n" + lines.join("\n") : "");
        }
        entries.sort((a, b) => b.col - a.col);
        const rows = [];
        const free = (row, from, to) => {
            for (let i = from; i <= to; i++) if (row[i] !== undefined && row[i] !== " ") return false;
            return true;
        };
        for (const entry of entries) {
            let r = 0;
            while (r < rows.length && !free(rows[r], entry.col, entry.col + entry.value.length)) r++;
            if (r === rows.length) rows.push([]);
            for (let i = 0; i < entry.value.length; i++) rows[r][entry.col + i] = entry.value[i];
            for (let above = 0; above < r; above++) rows[above][entry.col] = "|";
        }
        const pipes = [];
        for (const entry of entries) pipes[entry.col] = "|";
        const text = (row) => Array.from(row, (ch) => ch === undefined ? " " : ch).join("").replace(/\s+$/, "");
        return head + [condition.text, text(pipes), ...rows.map(text)].join("\n");
    };

    define("__pa", (index, fn) => {
        const condition = T.conditions[index];
        const seen = new Map();
        const value = fn((slot, v) => { seen.set(slot, v); return v; });
        if (condition.call && value === undefined) return;
        if (truthy(value)) return;
        throw new ConditionNotSatisfied(diagram(condition, seen), condition.line);
    });

    // ---- helpers ----

    // A condition anywhere, such as in a loop of a then block, where statements are not rewritten: Groovy's assert.
    define("assert", (condition, message) => {
        if (!truthy(condition)) {
            throw new ConditionNotSatisfied(`Assertion failed${message == null ? "" : ": " + message}` +
                `\n\nvalue: ${display(condition, 200)}`, null);
        }
    });

    // A service call that throws, naming the service and its errors, when the service fails.
    define("call", (name, params) => {
        const before = ec.message.errors || 0;
        const out = ec.service.sync().name(name).parameters(params || {}).call();
        const after = ec.message.errors || 0;
        if (after > before || out == null) {
            const errors = ec.message.getErrorsString();
            throw new Error(`service ${name} failed${errors ? ": " + errors : ""}`);
        }
        return out;
    });

    // Compare entity XML with the database; the messages of what did not match, which a failure also shows.
    define("checkData", (xml) => {
        const errors = ec.entity.makeDataLoader().xmlText(xml).check();
        for (const error of errors) {
            ec.logger.info(`checkData: ${error}`);
            T.dataErrors.push(error);
        }
        return errors;
    });

    define("login", (username, password) => {
        if (!ec.user.loginUser(username, password == null ? "moqui" : password)) {
            throw new Error(`login of ${username} failed: ${ec.message.getErrorsString() || "a second factor is required"}`);
        }
    });
    define("logout", () => { ec.user.logoutUser(); });

    define("screenTest", (baseScreenPath) => {
        const test = ec.screen.makeTest();
        return baseScreenPath ? test.baseScreenPath(baseScreenPath) : test;
    });

    define("dec", (v) => toDecimal(v));
    // The sum of a field (or of the items) of a list, as a decimal; null and empty count as zero.
    define("sum", (list, field) => {
        let total = toDecimal("0");
        for (const item of listOf(list) || []) {
            const v = field ? item[field] : item;
            if (v != null && v !== "") total = total.add(asDecimal(v) || toDecimal(v));
        }
        return total;
    });

    // Run fn(threadNum, args) on n threads, each with its own execution context. fn runs from its source text in each
    // thread's own engine, so it must not use variables of the spec file; args are copied as JSON.
    define("parallel", (n, fn, args, opts) => {
        if (typeof fn !== "function") throw new Error("parallel needs a function");
        const options = opts || {};
        const out = JSON.parse(__moquiTestHost.parallel(Number(n), String(fn), JSON.stringify(args === undefined ? null : plain(args, 0)),
            options.transaction !== false, options.timeout == null ? 120 : Number(options.timeout)));
        if (out.errors.length) throw new Error(`parallel: ${out.errors.length} of ${n} threads failed: ${out.errors.join("; ")}`);
        return out.results;
    });

    // An uploaded file with the text as its content, for the services that take a FileItem: uploadedFile("a.txt", "text", "text/plain").
    define("uploadedFile", (fileName, text, contentType) => __moquiTestHost.uploadedFile(String(fileName), String(text == null ? "" : text),
        contentType == null ? "text/plain" : String(contentType)));

    Object.defineProperty(g, "log", { get: () => ec.logger, configurable: true });
})(globalThis);
