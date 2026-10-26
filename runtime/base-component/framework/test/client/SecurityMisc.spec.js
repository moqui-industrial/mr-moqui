// A01, A05 and A10 proofs that did not fit the other client specs, ported from Moqui Java's pytest
// (framework/test/test_a01_redirect.py, test_a01_more.py, test_a05_injection.py, test_a10_errors.py; the catalog
// in README.md): a login Referer, a spoofed Host on returnTo, a SQL-looking preference, anonymous HTML errors, and
// an executable upload. Passes on any conf. The demo user is john.doe.

const USER = params.user ?? "john.doe"
const PASSWORD = params.password ?? "moqui"
const EVIL = "https://evil.example/phish"
// Spock's payload (SecurityErrorTests): the page must escape it, and Rust will not emit a Java stack class.
const PAYLOAD = "<script>alert(1)</script>"
const ESCAPED = "&lt;script&gt;alert(1)&lt;/script&gt;"

// No inline script, and no Rust panic or stack frame. Same stack rule as SecurityErrors' JSON proof: "Exception"
// alone is not a frame. A Java class name is not required. assert, because a call outside then is not rewritten.
const noScriptOrStack = (text) => {
    assert(!text.toLowerCase().includes("<script>"), "inline script tag")
    assert(!text.includes(PAYLOAD), "reflected script tag")
    assert(!/\.rs:\d+/.test(text), "Rust source location")
    assert(!text.includes("stack backtrace"), "stack backtrace")
    assert(!text.toLowerCase().includes("panic"), "panic")
    assert(!text.toLowerCase().includes("stack"), "stack")
    assert(!text.includes("Exception") || !text.toLowerCase().includes("stack"), "exception stack")
    return true
}

// test_a01_redirect.py: test_login_referer_does_not_redirect_offsite
feature("A01 the login does not follow a Referer off the site", {
    when() {
        const s = session()
        s.get("/Login", { headers: { Referer: EVIL } })
        return { res: s.post("/Login/login", { form: { username: USER, password: PASSWORD } }) }
    },
    then({ res }) {
        [200, 301, 302, 303, 307, 308].includes(res.status)
        !(res.location ?? "").toLowerCase().includes("evil.example")
    },
})

// test_a01_redirect.py: test_login_returnTo_does_not_follow_host_header
// returnTo is compared with the request Host (webroot/Login.xml). The Host is forged with raw(); a failure is a
// finding: the redirect must stay on the site, and a CR/LF in the path is rejected, not turned into a header.
feature("A01 the login does not follow a returnTo from a spoofed Host", {
    when() {
        const s = session()
        const form = `username=${encodeURIComponent(USER)}&password=${encodeURIComponent(PASSWORD)}`
        const page = s.raw("GET /Login?returnTo=" + encodeURIComponent(EVIL) +
            " HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n")
        const res = s.raw("POST /Login/login HTTP/1.1\r\nHost: evil.example\r\n" +
            "Content-Type: application/x-www-form-urlencoded\r\nContent-Length: " + form.length +
            "\r\nConnection: close\r\n\r\n" + form)
        const split = s.raw("GET /Login\r\nLocation: https://evil.example/phish HTTP/1.1\r\n" +
            "Host: localhost\r\nConnection: close\r\n\r\n", { cookies: false })
        return { page, res, split }
    },
    then({ page, res, split }) {
        page.status == 200
        page.cookie("moqui.session") != null
        res.status == 301 || res.status == 302 || res.status == 303 || res.status == 307 || res.status == 308
        !(res.location ?? "").toLowerCase().includes("evil.example")
        !(res.location ?? "").includes("/phish")
        split.status == 400
        !(split.location ?? "").toLowerCase().includes("evil.example")
        !(split.location ?? "").includes("/phish")
    },
})

// test_a05_injection.py: test_sql_looking_preference_is_stored_as_data
feature("A05 a SQL-looking preference is stored as data", {
    when() {
        const s = session()
        s.login(USER, PASSWORD)
        const key = "secSqliHttp"
        const sqli = "' OR '1'='1"
        const set = s.post("/apps/setPreference", { form: { preferenceKey: key, preferenceValue: sqli } })
        const prefs = s.get("/apps/getPreferences", { params: { keyRegexp: key } })
        s.post("/apps/setPreference", { form: { preferenceKey: key, preferenceValue: "" } })
        return { set, prefs, key, sqli }
    },
    then({ set, prefs, key, sqli }) {
        set.status != 401
        set.status != 403
        prefs.status == 200
        prefs.text.includes(sqli)
        prefs.json()[key] == sqli
        !prefs.text.toLowerCase().includes("syntax error")
        !prefs.text.toLowerCase().includes("sql exception")
    },
})

// test_a10_errors.py: test_unauthorized_error_page_html_has_no_script_tag
feature("A10 the unauthorized page escapes a message and has no script tag", {
    when() {
        return { res: session().get("/error/Unauthorized", { params: { errorMessage: PAYLOAD }, accept: "html" }) }
    },
    then({ res }) {
        res.text.includes(ESCAPED)
        noScriptOrStack(res.text)
    },
})

// test_a10_errors.py: test_rest_unauth_html_error_has_no_script_or_stack
feature("A10 a REST 401 asked for as HTML has no script tag or stack frame", {
    when() {
        return { res: session().get("/rest/e1/moqui.basic.Enumeration", { accept: "html" }) }
    },
    then({ res }) {
        res.status == 401
        noScriptOrStack(res.text)
    },
})

// test_a10_errors.py: test_internal_error_page_does_not_include_stack_trace_when_hit_directly
feature("A10 the internal error page hit directly has no stack trace", {
    when() {
        return { res: session().get("/error/InternalError", { params: { errorMessage: PAYLOAD }, accept: "html" }) }
    },
    then({ res }) {
        res.text.includes(ESCAPED)
        !res.text.toLowerCase().includes("groovy:") || !res.text.includes("at ")
        noScriptOrStack(res.text)
    },
})

// test_a01_more.py: test_multipart_executable_upload_is_rejected
// The check is on every upload (WebUtilities.isExecutable). This transition is the Python path. An MZ program and a
// #! script are rejected; a ZIP (PK) is not a program. magic is the response's first four bytes, not the upload.
feature("A01 an uploaded #kind is #verdict", {
    where: [
        { kind: "MZ program", verdict: "rejected", fileName: "evil.exe", contentType: "application/octet-stream",
            base64: "TVoAAFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhY", magic: "4d5a0000" },
        { kind: "shell script", verdict: "rejected", fileName: "run.sh", contentType: "text/x-sh",
            text: "#!/bin/sh\necho hi\n", magic: "#!/b" },
        { kind: "ZIP", verdict: "accepted", fileName: "ok.zip", contentType: "application/zip",
            base64: "UEsDBFhYWFhYWFhYWFhYWFhYWFg=", magic: "504b0304" },
    ],
    when({ fileName, contentType, base64, text }) {
        const s = session()
        s.login(USER, PASSWORD)
        const part = { fileName, contentType }
        if (base64 != null) part.base64 = base64
        if (text != null) part.text = text
        return {
            res: s.post("/apps/tools/Entity/DataSnapshot/uploadSnapshot", { multipart: { snapshotFile: part } }),
        }
    },
    then({ res, verdict, fileName, magic }) {
        const rejected = verdict == "rejected"
        rejected == (res.status == 415)
        !rejected || res.text.toLowerCase().includes("executable")
        !rejected || res.text.includes(fileName)
        rejected || res.status == 302
        res.magic != magic
    },
})
