// A01 Broken Access Control over HTTP, ported from Moqui Java's pytest proofs (framework/test, the catalog in
// README.md): anonymous requests to the apps and REST, the REST transitions that were removed, a path with .. that is
// not normalized, the fixture users' limits (test/ClientFixtures.spec.js), and the login's returnTo. Passes on any
// conf; the features that need the fixture users skip without them, as the Python suite does.

// The fixture user can log in from here, else the feature skips. A session of its own, not logged out: a REST logout
// ends every session of the user (UserAccount.hasLoggedOut).
const requireFixture = (username, password) => {
    const res = session().post("/rest/login", { form: { username, password }, accept: "json", csrf: false })
    if (res.status == 401) skip(`${username} is not on this server: run test/ClientFixtures.spec.js on its database (moqui test framework --spec ClientFixtures --db existing), or use --serve`)
    return res
}
// A session logged in through the login form, as the Python screen_login.
const formSession = (username, password) => {
    const s = session()
    const res = s.formLogin(username, password)
    assert(res.status == 302, `the login form did not log ${username} in: HTTP ${res.status}`)
    return s
}

// test_a01_access.py: test_tools_without_session_is_not_the_tool
feature("A01 Tools without a session redirects to the login, not the tool", {
    when() { return { res: session().get("/apps/tools/dashboard") } },
    then({ res }) {
        res.status == 302
        res.location.includes("/Login")
        !res.text.toLowerCase().includes("auto screens")
    },
})

// test_a01_access.py: test_rest_e1_without_credentials_is_401, test_rest_s1_without_credentials_is_unauthorized
feature("A01 #path without credentials is 401", {
    where: [{ path: "/rest/e1/moqui.basic.Enumeration" }, { path: "/rest/s1/moqui/basic/enums" }],
    when({ path }) { return { res: session().get(path) } },
    then({ res }) {
        res.status == 401
        !res.text.includes("enumId")
    },
})

// test_a01_access.py: test_removed_rest_api_key_is_not_found, test_removed_rest_moqui_session_token_is_not_found;
// test_api_rest.py: test_rest_userInfo_transition_is_gone
feature("A01 the removed REST transition #path is 404, not merely unauthenticated", {
    where: [{ path: "/rest/api_key" }, { path: "/rest/moquiSessionToken" }, { path: "/rest/userInfo" }],
    when({ path }) { return { res: session().get(path) } },
    then({ res }) {
        res.status == 404
    },
})

// test_a01_more.py: test_apps_get_preferences_without_login_is_401
feature("A01 getPreferences without a login is 401 with no preferences", {
    when() { return { res: session().get("/apps/getPreferences", { params: { keyRegexp: ".*" } }) } },
    then({ res }) {
        res.status == 401
        !res.text.toLowerCase().includes("mantle.content.root")
        !res.text.toLowerCase().includes("authorize") || res.text.toLowerCase().includes("authentication")
    },
})

// test_a01_more.py: test_static_path_dotdot_does_not_escape. The path goes as it is (http.raw); it resolves to
// /Login.xml, the Login screen in the xml render mode, and the screen's source is not served.
feature("A01 a static path with .. does not serve screen XML", {
    when() { return { res: http.raw("GET /js/../../Login.xml HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", { cookies: false }) } },
    then({ res }) {
        !res.text.includes("<screen")
        !res.text.includes("<transition")
        !res.text.toLowerCase().includes("require-session-token")
    },
})

// test_a01_more.py: test_none_user_cannot_open_tools
feature("A01 a user with no authorization cannot open Tools", {
    given() { requireFixture("sec.none.only", "SecNone1!!") },
    when() { return { res: formSession("sec.none.only", "SecNone1!!").get("/apps/tools/dashboard") } },
    then({ res }) {
        res.status == 403
        !res.text.toLowerCase().includes("auto screens")
    },
})

// test_a01_more.py: test_view_only_cannot_open_sql_runner
feature("A01 a VIEW-only user cannot open the SQL Runner", {
    given() { requireFixture("sec.view.only", "SecView1!!") },
    when() {
        const s = formSession("sec.view.only", "SecView1!!")
        return {
            tools: s.get("/apps/tools/dashboard"),
            res: s.get("/apps/tools/Entity/SqlRunner", { params: { groupName: "transactional", sql: "SELECT 1" } }),
        }
    },
    then({ tools, res }) {
        // the control: the user is logged in and may view Tools
        tools.status == 200
        // the screen's actions refuse the user (SQL_RUNNER_WEB) and the page shows why, as in Java; no query ran
        res.status == 403 || res.text.includes("does not have permission to use the SQL Runner")
        !res.text.includes("Showing all")
        !res.text.includes("Query altered")
    },
})

// test_a01_redirect.py: test_login_returnTo_does_not_redirect_offsite
feature("A01 the login does not follow a returnTo off the site", {
    when() {
        const s = session()
        s.get("/Login", { params: { returnTo: "https://evil.example/phish" } })
        return { res: s.post("/Login/login", { form: { username: params.user ?? "john.doe", password: params.password ?? "moqui" } }) }
    },
    then({ res }) {
        res.status == 302
        !res.location.toLowerCase().includes("evil.example")
        !res.location.includes("/phish")
    },
})

// test_a01_redirect.py: test_login_returnTo_same_origin_path_is_allowed
feature("A01 the login follows a returnTo on the site", {
    when() {
        const s = session()
        s.get("/Login", { params: { returnTo: "/apps" } })
        return { res: s.post("/Login/login", { form: { username: params.user ?? "john.doe", password: params.password ?? "moqui" } }) }
    },
    then({ res }) {
        res.status == 302
        res.location.startsWith("/") || res.location.startsWith(server.url)
        res.location.includes("/apps")
    },
})
