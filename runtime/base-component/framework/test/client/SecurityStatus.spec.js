// A01 Broken Access Control for /status over HTTP, ported from Moqui Java's pytest proofs
// (framework/test/test_a01_more.py; the catalog in README.md). Each request connects from loopback, as that suite
// does. With webapp_client_ip_header empty, X-Forwarded-For and Forwarded are ignored, so they neither hide the
// loopback JSON nor grant the sensitive map. /status calls getStatusMap() with no flag, so DataSources is omitted;
// Heap and VmVendor are Java keys this server never sends. Passes on any conf.

// test_a01_more.py: test_status_from_loopback_is_json
feature("A01 /status from loopback is JSON", {
    when() { return { res: session().get("/status") } },
    then({ res }) {
        res.status == 200
        res.contentType.includes("json")
        res.text.trim().startsWith("{")
        // Utilization is this server's load key. Heap is a Java key it never sends, so it is absent, not required.
        res.text.toLowerCase().includes("utilization")
        !res.text.toLowerCase().includes("heap")
        // DataSources is the sensitive key this map can carry; the public call leaves it out. VmVendor is not a key.
        !res.text.toLowerCase().includes("datasources")
        !res.text.toLowerCase().includes("vmvendor")
        res.json().Utilization != null
        res.json().DataSources == null
        res.json().Heap == null
        res.json().VmVendor == null
    },
})

// test_a01_more.py: test_status_x_forwarded_for_does_not_grant_access_from_spoofed_loopback
feature("A01 X-Forwarded-For does not grant a spoofed loopback on /status", {
    when() { return { res: session().get("/status", { headers: { "X-Forwarded-For": "8.8.8.8" } }) } },
    then({ res }) {
        // still the loopback map: the header is not the client address, and it adds no DataSources or VmVendor
        res.status == 200
        res.contentType.includes("json")
        res.text.trim().startsWith("{")
        res.text.toLowerCase().includes("utilization")
        !res.text.toLowerCase().includes("heap")
        !res.text.toLowerCase().includes("datasources")
        !res.text.toLowerCase().includes("vmvendor")
        res.json().Utilization != null
        res.json().DataSources == null
        res.json().Heap == null
        res.json().VmVendor == null
    },
})

// test_a01_more.py: test_status_forwarded_for_header_does_not_hide_loopback
feature("A01 Forwarded does not hide /status from loopback", {
    when() { return { res: session().get("/status", { headers: { Forwarded: "for=8.8.8.8" } }) } },
    then({ res }) {
        // the header does not hide the loopback JSON, and DataSources stays out
        res.status == 200
        res.contentType.includes("json")
        res.text.trim().startsWith("{")
        res.text.toLowerCase().includes("utilization")
        !res.text.toLowerCase().includes("heap")
        !res.text.toLowerCase().includes("datasources")
        !res.text.toLowerCase().includes("vmvendor")
        res.json().Utilization != null
        res.json().DataSources == null
        res.json().Heap == null
        res.json().VmVendor == null
    },
})
