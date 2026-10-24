// The globals of a client spec (`moqui test-client`, plans/ClientTestHarness.md 5): the server under test, the params,
// HTTP sessions, raw requests, WebSocket and the component's files, over __moquiTestHost.call. The host
// (test_client/mod.rs) evaluates this after the core prelude (spec, feature, the conditions, assert, dec, sum, skip),
// before each spec file. There is no execution context: `ec` and the server helpers throw, naming what to use instead.
(function (g) {
    const define = (name, value) => Object.defineProperty(g, name, { value, writable: true, configurable: true });
    const host = (name, args) => JSON.parse(__moquiTestHost.call(name, JSON.stringify(args === undefined ? null : args)));
    const config = host("config");
    const OUTSIDE = "a client test (test/client) runs outside the server";

    Object.defineProperty(g, "ec", {
        get() { throw new Error(`\`ec\` is the server's: ${OUTSIDE}; use http, ws or browser`); },
        configurable: true
    });
    for (const [name, instead] of [
        ["call", "http.post or http.get on the service's REST path (/rest/s1/...)"],
        ["checkData", "http.get on /rest/e1/... with conditions on the answer"],
        ["screenTest", "http.get on the screen path, or browser"],
        ["uploadedFile", "http.post with multipart: { name: { fileName, text, contentType } }"],
    ]) define(name, () => { throw new Error(`${name}() is the server's: ${OUTSIDE}; use ${instead}`); });
    Object.defineProperty(g, "browser", {
        get() { throw new Error("browser is not built yet (plans/ClientTestHarness.md P5)"); },
        configurable: true
    });
    define("load", () => { throw new Error("load is not built yet (plans/ClientTestHarness.md P4)"); });
    define("parallel", () => { throw new Error("parallel in a client spec is not built yet (plans/ClientTestHarness.md P4)"); });

    define("server", Object.freeze(config.server));
    define("params", Object.freeze(config.params));

    const options = (opts) => (opts && typeof opts === "object") ? opts : {};

    class Response {
        constructor(data) { Object.assign(this, data); }
        // The first value of a header, case-insensitive; null when the response has none.
        header(name) {
            const key = String(name).toLowerCase();
            const found = this.headerList.find(([n]) => n.toLowerCase() === key);
            return found ? found[1] : null;
        }
        // Every value of a header, as for Set-Cookie; with no name, every [name, value].
        headers(name) {
            if (name == null) return this.headerList.slice();
            const key = String(name).toLowerCase();
            return this.headerList.filter(([n]) => n.toLowerCase() === key).map(([, v]) => v);
        }
        json() {
            try {
                return JSON.parse(this.text);
            } catch (e) {
                throw new Error(`the response of ${this.url} is not JSON (status ${this.status}, ${this.contentType || "no content type"}): ` +
                    this.text.slice(0, 200));
            }
        }
        cookie(name) { return this.cookies.find((c) => c.name === name) || null; }
        assertContains(text) {
            assert(this.text.includes(text), `the response of ${this.url} (status ${this.status}) does not contain ` +
                `${JSON.stringify(text)}: ${this.text.slice(0, 300)}`);
            return true;
        }
        assertNotContains(text) {
            assert(!this.text.includes(text), `the response of ${this.url} (status ${this.status}) contains ${JSON.stringify(text)}`);
            return true;
        }
        // The body into the output directory; the path written.
        save(name) { return host("response.save", { id: this.id, name: String(name) }); }
    }

    class Session {
        constructor(id) { this.id = id; }
        request(method, path, opts) {
            return new Response(host("http.request", { session: this.id, method: String(method).toUpperCase(), path: String(path),
                opts: options(opts) }));
        }
        get(path, opts) { return this.request("GET", path, opts); }
        post(path, opts) { return this.request("POST", path, opts); }
        put(path, opts) { return this.request("PUT", path, opts); }
        patch(path, opts) { return this.request("PATCH", path, opts); }
        delete(path, opts) { return this.request("DELETE", path, opts); }
        options(path, opts) { return this.request("OPTIONS", path, opts); }
        head(path, opts) { return this.request("HEAD", path, opts); }
        // The request text as it is, over TCP or TLS, with this session's Cookie header unless cookies: false.
        raw(text, opts) { return new Response(host("http.raw", { session: this.id, text: String(text), opts: options(opts) })); }
        // The REST login, which needs no session token. Throws when it fails or needs a second factor.
        login(user, password) {
            const username = String(user);
            const res = this.post("/rest/login", { form: { username, password: password == null ? "moqui" : String(password) },
                accept: "json", csrf: false });
            let body = null;
            try { body = JSON.parse(res.text); } catch (e) { /* the status says what failed */ }
            if (res.status < 400 && body && body.loggedIn === true) return res;
            if (res.status < 400 && body && body.secondFactorRequired) throw new Error(`login of ${username} needs a second factor`);
            const why = body && (body.errors || body.error || body.message);
            throw new Error(`login of ${username} failed: HTTP ${res.status}${why ? ": " + why : ""}`);
        }
        logout() { return this.get("/rest/logout"); }
        // The login form: GET /Login for its cookie, then POST /Login/login with redirects off; that response.
        formLogin(user, password, opts) {
            const o = options(opts);
            this.get("/Login", { headers: o.headers });
            const form = { username: String(user), password: password == null ? "moqui" : String(password) };
            if (o.returnTo != null) form.returnTo = String(o.returnTo);
            return this.post("/Login/login", { form, headers: o.headers });
        }
        cookies() { return host("session.cookies", { session: this.id }); }
        clearCookies() { host("session.clearCookies", { session: this.id }); }
        token() { return host("session.token", { session: this.id }); }
    }

    // A new session with its own cookie jar and token; opts.headers go on each request, opts.user logs it in.
    const newSession = (opts) => {
        const o = options(opts);
        const s = new Session(host("session.new", { headers: o.headers || {} }));
        if (o.user != null) s.login(o.user, o.password);
        return s;
    };
    define("session", newSession);
    define("http", newSession());
    define("login", (user, password) => g.http.login(user, password));
    define("logout", () => g.http.logout());

    class Socket {
        constructor(data) { Object.assign(this, data); }
        send(text) { host("ws.send", { id: this.id, text: String(text) }); }
        // The next text frame, or null on timeout (seconds) or close.
        receive(opts) {
            const o = options(opts);
            return host("ws.receive", { id: this.id, timeout: o.timeout == null ? null : Number(o.timeout) });
        }
        close() { if (this.open) host("ws.close", { id: this.id }); this.open = false; }
    }
    define("ws", Object.freeze({
        // A refused handshake does not throw: open is false and status is the HTTP answer.
        connect(path, opts) {
            const o = options(opts);
            return new Socket(host("ws.connect", { path: String(path), session: (o.session || g.http).id, headers: o.headers || {},
                origin: o.origin == null ? null : String(o.origin), timeout: o.timeout == null ? null : Number(o.timeout) }));
        }
    }));

    // The spec's own component directory, read-only.
    define("files", Object.freeze({
        // The files under dir (relative to the component), with paths relative to it, matching regex when given.
        list(dir, regex) {
            const all = host("files.list", { dir: dir == null ? "" : String(dir) });
            if (regex == null) return all;
            const re = regex instanceof RegExp ? regex : new RegExp(String(regex));
            return all.filter((path) => { re.lastIndex = 0; return re.test(path); });
        },
        read(path) { return host("files.read", { path: String(path) }); }
    }));

    define("sleep", (ms) => { host("sleep", { ms: Number(ms) || 0 }); });
    // Poll fn until it is truthy and answer its value; else throw after timeout seconds (--timeout by default).
    define("waitUntil", (fn, opts) => {
        if (typeof fn !== "function") throw new Error("waitUntil needs a function");
        const o = options(opts);
        const timeout = o.timeout == null ? config.timeout : Number(o.timeout);
        const interval = o.interval == null ? 100 : Number(o.interval);
        const end = Date.now() + timeout * 1000;
        for (;;) {
            const value = fn();
            if (__truthy(value)) return value;
            if (Date.now() >= end) {
                throw new Error(`waitUntil: not true after ${timeout}s; the last value was ${__moquiTest.display(value, 100)}`);
            }
            sleep(interval);
        }
    });

    const logAt = (level) => (message) => { host("log", { level, message: typeof message === "string" ? message : __moquiTest.display(message, 2000) }); };
    define("log", Object.freeze({ trace: logAt("trace"), debug: logAt("debug"), info: logAt("info"), warn: logAt("warn"), error: logAt("error") }));
})(globalThis);
