// A02 Security Misconfiguration over HTTP, ported from Moqui Java's pytest proofs (framework/test/test_a02_headers.py;
// the catalog in README.md): the headers of an HTML page, the session cookie's attributes, caching of the login page,
// CORS for a request with no Origin, and no H2 console. Passes on any conf; SecurityCors has the proofs that need the
// production conf. The session cookie is moqui.session (Java's is JSESSIONID).

// test_a02_headers.py: test_html_response_has_csp_and_frame_options
feature("A02 an HTML page has a CSP with frame-ancestors and form-action, X-Frame-Options and nosniff", {
    when() { return { res: session().get("/Login") } },
    then({ res }) {
        res.status == 200
        res.header("Content-Security-Policy").toLowerCase().includes("frame-ancestors")
        res.header("Content-Security-Policy").toLowerCase().includes("form-action")
        !!res.header("X-Frame-Options")
        res.header("X-Content-Type-Options").toLowerCase().includes("nosniff")
    },
})

// test_a02_headers.py: test_session_cookie_is_httponly, test_session_cookie_samesite_lax
feature("A02 the session cookie is HttpOnly and SameSite=Lax", {
    when() { return { cookie: session().get("/Login").cookie("moqui.session") } },
    then({ cookie }) {
        cookie != null
        cookie.httpOnly
        cookie.sameSite.toLowerCase() == "lax"
    },
})

// test_a02_headers.py: test_login_html_is_not_publicly_cacheable
feature("A02 the login page is not publicly cacheable", {
    when() { return { cc: (session().get("/Login").header("Cache-Control") ?? "").toLowerCase() } },
    then({ cc }) {
        cc.includes("no-store") || cc.includes("no-cache") || cc.includes("private")
        !cc.includes("public")
    },
})

// test_a02_headers.py: test_cors_no_origin_header_has_no_allow_origin
feature("A02 a request with no Origin gets no Access-Control-Allow-Origin", {
    when() { return { res: session().get("/Login") } },
    then({ res }) {
        res.status == 200
        res.header("Access-Control-Allow-Origin") == null
    },
})

// test_a02_headers.py: test_h2_console_is_not_mounted. There is no H2 console here; the proof that nothing answers
// at its path is kept.
feature("A02 no H2 console is mounted", {
    when() { return { res: session().get("/h2/") } },
    then({ res }) {
        res.status == 404
        !res.text.toLowerCase().includes("h2 console")
        !res.text.toLowerCase().includes("login.jsp")
        !(res.location ?? "").includes("/h2")
    },
})
