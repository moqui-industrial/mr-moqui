// A01 Broken Access Control for the shipped proxies, ported from framework/test/test_a01_more.py. Java's /elastic
// (ElasticRemote) is /search with SearchRemote; Java's /kibana (KibanaRemote) is /dashboard with DashboardRemote
// (SearchAuthFilter, DashboardAuthFilter, SearchProxy, DashboardProxy). kibana_host defaults to 127.0.0.1, so
// DashboardProxy is mounted and an anonymous request is 401 from the filter, not an open proxy. An empty host makes
// proxyTo "http://:5601", which ProxyServlet answers 404 only after that same filter. Features that need
// sec.none.only skip without that fixture user, as the Python suite does.

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

// test_a01_more.py: test_elastic_proxy_without_login_is_401
feature("A01 /search without a login is 401", {
    when() { return { res: session().get("/search/") } },
    then({ res }) {
        res.status == 401
    },
})

// test_a01_more.py: test_elastic_proxy_logged_in_without_permission_is_denied
feature("A01 a logged-in user without SearchRemote is denied /search", {
    given() { requireFixture("sec.none.only", "SecNone1!!") },
    when() {
        const s = formSession("sec.none.only", "SecNone1!!")
        return {
            probe: s.get("/apps/getPreferences", { params: { keyRegexp: "x" } }),
            res: s.get("/search/"),
        }
    },
    then({ probe, res }) {
        probe.status != 401
        !(probe.status == 302 || probe.status == 303) || !(probe.location ?? "").toLowerCase().includes("login")
        res.status == 403
        res.text.toLowerCase().includes("searchremote") || res.text.toLowerCase().includes("permission")
    },
})

// test_a01_more.py: test_kibana_proxy_without_login_is_401
feature("A01 /dashboard without a login is 401", {
    when() { return { res: session().get("/dashboard/") } },
    then({ res }) {
        res.status == 401
    },
})

// test_a01_more.py: test_elastic_proxy_basic_auth_without_permission_is_403
// AuthFilter takes Basic credentials from the request, not only a session cookie.
feature("A01 Basic auth without SearchRemote is 403 on /search", {
    given() { requireFixture("sec.none.only", "SecNone1!!") },
    when() { return { res: session().get("/search/", { basic: ["sec.none.only", "SecNone1!!"] }) } },
    then({ res }) {
        res.status == 403
    },
})

// test_a01_more.py: test_elastic_proxy_bad_basic_auth_is_401
feature("A01 bad Basic auth on /search is 401", {
    given() { requireFixture("sec.none.only", "SecNone1!!") },
    when() { return { res: session().get("/search/", { basic: ["sec.none.only", "definitely-wrong-password"] }) } },
    then({ res }) {
        res.status == 401
    },
})

// test_a01_more.py: test_elastic_proxy_api_key_header_garbage_is_401
feature("A01 a garbage api_key on /search does not authenticate", {
    when() { return { res: session().get("/search/", { apiKey: "not-a-real-key" }) } },
    then({ res }) {
        res.status == 401 || res.status == 403
    },
})

// test_a01_more.py: test_elastic_proxy_basic_auth_with_permission_reaches_the_cluster
// Java required HTTP 200 and a cluster_name or version body, and skipped when the cluster was down or the user
// lacked ElasticRemote. john.doe is in ADMIN, which seed data grants SearchRemote. The default search backend is
// postgres, so requireElasticApi answers 404 after the filter; a down OpenSearch is a proxy error, not an auth
// failure. This proof only asserts the request got past authorization (not 401 and not 403). A live cluster body
// is not required.
feature("A01 Basic auth with SearchRemote gets past authorization on /search", {
    when() {
        return { res: session().get("/search/", { basic: [params.user ?? "john.doe", params.password ?? "moqui"] }) }
    },
    then({ res }) {
        res.status != 401
        res.status != 403
    },
})
