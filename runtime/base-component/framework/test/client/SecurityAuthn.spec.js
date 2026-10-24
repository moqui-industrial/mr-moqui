// A07 Identification and Authentication Failures over HTTP, ported from Moqui Java's pytest proofs
// (framework/test/test_a07_authn.py, test_a07_more.py, test_api_rest.py; the catalog in README.md): the REST login,
// the session token on a POST, a new session id at login, api_key and Basic. Passes on any conf.
//
// Where Java's proofs give john.doe a wrong password, these use an unknown user: MR-Moqui locks an account on its
// fourth failed login, and the suite must not lock the demo user. The session-token failure is 401 "Session token is
// missing or is not valid", and the session cookie is moqui.session (Java's is JSESSIONID).

const USER = params.user ?? "john.doe"
const PASSWORD = params.password ?? "moqui"
const SHARED_TEXT = "the username or password is not valid"

spec({ setupSpec() { login(USER, PASSWORD) } })

// test_a07_authn.py: test_rest_login_succeeds_without_csrf_token
feature("A07 the REST login needs no session token", {
    when() { return { res: session().post("/rest/login", { form: { username: USER, password: PASSWORD }, accept: "json", csrf: false }) } },
    then({ res }) {
        res.status == 200
        res.json().loggedIn == true
        !res.text.toLowerCase().includes("token required")
        !res.text.toLowerCase().includes("csrf")
    },
})

// test_a07_more.py: test_rest_login_unknown_user_is_401
feature("A07 a REST login of an unknown user is 401 with the shared text", {
    when() {
        return { res: session().post("/rest/login", { json: { username: "sec.no.such.user.zzz", password: "wrong" }, accept: "json" }) }
    },
    then({ res }) {
        res.status == 401
        res.text.toLowerCase().includes(SHARED_TEXT)
        !res.text.replace(/\s/g, "").toLowerCase().includes('"loggedin":true')
    },
})

// test_a07_authn.py: test_post_with_csrf_token_executes_preference_round_trip, test_post_without_csrf_does_not_store_preference
feature("A07 a POST runs with the session token and stores nothing without it (csrf: #csrf)", {
    where: [{ csrf: true, status: 200, stored: true }, { csrf: false, status: 401, stored: false }],
    when({ csrf }) {
        const key = `secCsrfHttp-${csrf}-${Date.now()}`
        const res = http.post("/apps/setPreference", { form: { preferenceKey: key, preferenceValue: "csrf-ran" }, csrf })
        const prefs = http.get("/apps/getPreferences", { params: { keyRegexp: key } })
        // blank it again, as the Python proofs do
        if (csrf) http.post("/apps/setPreference", { form: { preferenceKey: key, preferenceValue: "" } })
        return { res, prefs, key }
    },
    then({ res, status, prefs, key, stored }) {
        res.status == status
        res.status != 401 || res.text.includes("Session token is missing or is not valid")
        prefs.status == 200
        (prefs.json()[key] == "csrf-ran") == stored
    },
})

// test_a07_authn.py: test_login_issues_a_new_session_id
feature("A07 the login form issues a new session id", {
    when() {
        const s = session()
        const before = s.get("/Login").cookie("moqui.session")
        const res = s.post("/Login/login", { form: { username: USER, password: PASSWORD } })
        return { before, res, after: res.cookie("moqui.session") }
    },
    then({ before, res, after }) {
        res.status == 302
        before != null
        after != null
        after.value != before.value
    },
})

// test_a07_authn.py: test_api_key_in_query_string_does_not_authenticate, test_api_key_header_with_garbage_does_not_authenticate
feature("A07 an api_key #where does not log in", {
    where: [
        { where: "in the query", opts: { params: { api_key: "not-a-real-key" } } },
        { where: "header that is not a key", opts: { headers: { api_key: "not-a-real-key" } } },
    ],
    when({ opts }) { return { res: session().get("/rest/e1/moqui.basic.Enumeration", opts) } },
    then({ res }) {
        res.status == 401
    },
})

// test_api_rest.py: test_rest_e1_basic_auth_wrong_password_is_401 (an unknown user here, not john.doe with a wrong
// password), test_rest_e1_basic_auth_valid_is_not_unauthenticated
feature("A07 Basic on /rest/e1: #who", {
    where: [
        { who: "an unknown user is 401", user: "sec.no.such.user.zzz", password: "wrong", valid: false },
        { who: "a valid user is not unauthenticated", user: USER, password: PASSWORD, valid: true },
    ],
    when({ user, password }) { return { res: session().get("/rest/e1/moqui.basic.Enumeration", { basic: [user, password], accept: "json" }) } },
    then({ res, valid }) {
        (res.status == 401) == !valid
        // entity REST has no catch-all authz: 403 is authenticated but not authorized
        !valid || [200, 403, 404].includes(res.status)
        !res.text.includes("currentPassword")
    },
})
