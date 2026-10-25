// A10 Mishandling of Exceptional Conditions over HTTP, ported from Moqui Java's pytest proofs
// (framework/test/test_a10_errors.py; the catalog in README.md): an error answer shows no stack. Passes on any conf.

// test_a10_errors.py: test_rest_unauth_json_does_not_include_stack_frames
feature("A10 a REST 401 in JSON has no stack frames", {
    when() { return { res: session().get("/rest/e1/moqui.basic.Enumeration", { accept: "json" }) } },
    then({ res }) {
        res.status == 401
        res.contentType.includes("json")
        typeof res.json().errors == "string"
        !res.text.includes("at org.moqui")
        !res.text.includes("Exception") || !res.text.toLowerCase().includes("stack")
        // nor a Rust location or backtrace
        !/\.rs:\d+/.test(res.text)
        !res.text.includes("stack backtrace")
    },
})
