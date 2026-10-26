// A01 Broken Access Control for /rest/sm, ported from Moqui Java's pytest proofs (framework/test/test_api_rest.py and
// test_a01_more.py): SmatHmacSha256, SmatHmacSha256Timestamp (five minute window and replay), SmatNone, and an unknown
// type. message.rs answers 403 for a missing HMAC header, a bad signature, a timestamp outside the window, and a
// replay, the same codes as those proofs; a missing fixture remote is 400 "not valid" and the feature skips. No login.
// Passes on any conf.

const MESSAGE_TYPE = "SEC_SMT_TEST"
const REMOTE_HMAC = "SEC_SMR_HMAC"
const REMOTE_TS = "SEC_SMR_HMAC_TS"
const REMOTE_NONE = "SEC_SMR_NONE"
const SECRET = "sec-hmac-test-secret"
const HEADER = "X-Moqui-Signature"
const BODY = '{"probe":true}'

// 400 containing "not valid" is a missing SEC_SMR_* fixture, the Python skip. A refusal, including a bad signature's
// 403, is returned and asserted.
const postMessage = (remote, body, signature) => {
    const headers = { "Content-Type": "application/json" }
    if (signature != null) headers[HEADER] = signature
    const res = session().post(`/rest/sm/${MESSAGE_TYPE}/${remote}`, { body, headers, csrf: false })
    if (res.status == 400 && res.text.toLowerCase().includes("not valid")) {
        skip("the SEC_SMR_* remotes are not on this server: run test/ClientFixtures.spec.js on its database " +
            "(moqui test framework --spec ClientFixtures --db existing), or use --serve")
    }
    return res
}

// test_api_rest.py: test_system_message_hmac_missing_header_is_403
feature("A01 a /rest/sm HMAC post with no signature header is 403", {
    when() { return { res: postMessage(REMOTE_HMAC, BODY) } },
    then({ res }) {
        res.status == 403
        res.text.toLowerCase().includes("hmac") || res.text.toLowerCase().includes("header")
    },
})

// test_api_rest.py: test_system_message_hmac_bad_signature_is_403
feature("A01 a /rest/sm HMAC post with a bad signature is 403", {
    when() { return { res: postMessage(REMOTE_HMAC, BODY, "not-a-valid-hmac") } },
    then({ res }) {
        res.status == 403
        res.text.toLowerCase().includes("hmac")
    },
})

// test_api_rest.py: test_system_message_hmac_valid_does_not_require_login
feature("A01 a valid /rest/sm HMAC signature needs no login", {
    when() { return { res: postMessage(REMOTE_HMAC, BODY, hmac.sha256Base64(SECRET, BODY)) } },
    then({ res }) {
        res.status == 200
        !res.text.toLowerCase().includes("hmac verify failed")
        res.text.toLowerCase().includes("systemmessageidlist")
    },
})

// test_api_rest.py: test_system_message_hmac_timestamp_outside_window_is_403
feature("A01 a /rest/sm timestamp HMAC outside the window is 403", {
    when() {
        // 400 seconds ago, as in the Python proof: the receive window is five minutes.
        const ts = Math.floor(Date.now() / 1000) - 400
        return { res: postMessage(REMOTE_TS, BODY, hmac.sha256Timestamp(SECRET, BODY, ts)) }
    },
    then({ res }) {
        res.status == 403
        res.text.toLowerCase().includes("hmac") || res.text.toLowerCase().includes("timestamp")
    },
})

// test_api_rest.py: test_system_message_hmac_timestamp_in_window_is_accepted
// Positive control: a remote that rejected every timestamp would still pass the refusals above.
feature("A01 a /rest/sm timestamp HMAC inside the window is accepted", {
    when() {
        const ts = Math.floor(Date.now() / 1000)
        return { res: postMessage(REMOTE_TS, BODY, hmac.sha256Timestamp(SECRET, BODY, ts)) }
    },
    then({ res }) {
        res.status == 200
        res.text.toLowerCase().includes("systemmessageidlist")
    },
})

// test_api_rest.py: test_system_message_smat_none_accepts_anonymous
feature("A01 a /rest/sm SmatNone remote accepts an anonymous post", {
    when() { return { res: postMessage(REMOTE_NONE, BODY) } },
    then({ res }) {
        res.status == 200
        res.text.toLowerCase().includes("systemmessageidlist")
    },
})

// test_api_rest.py: test_system_message_hmac_timestamp_replay_is_rejected
feature("A01 a replayed /rest/sm timestamp HMAC is rejected", {
    when() {
        // Signed once, at one unix second, and posted twice. A different body from the in-window proof, so the two
        // do not share a replay key when they run in the same second.
        const body = '{"probe":"replay"}'
        const ts = Math.floor(Date.now() / 1000)
        const header = hmac.sha256Timestamp(SECRET, body, ts)
        const first = postMessage(REMOTE_TS, body, header)
        const second = postMessage(REMOTE_TS, body, header)
        return { first, second }
    },
    then({ first, second }) {
        first.status == 200
        first.text.toLowerCase().includes("systemmessageidlist")
        second.status == 403
        second.text.toLowerCase().includes("hmac")
        second.text.toLowerCase().includes("replay")
    },
})

// test_a01_more.py: test_system_message_unknown_remote_is_rejected
// 400 "Message type NotAType not valid" here (the type is checked before the remote). Python accepts 400, 401, or 403.
feature("A01 an unknown /rest/sm remote is rejected", {
    when() {
        return { res: session().post("/rest/sm/NotAType/NotARemote", {
            body: BODY, headers: { "Content-Type": "application/json", Accept: "application/json" }, csrf: false,
        }) }
    },
    then({ res }) {
        res.status == 400
        res.text.toLowerCase().includes("not valid")
    },
})
