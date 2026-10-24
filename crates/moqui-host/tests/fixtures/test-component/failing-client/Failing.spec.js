// A client spec that fails on purpose (crates/moqui-host/tests/it/test_client.rs): the run exits 1 with its line.
feature("passes first", { expect() { http.get("/Login").status == 200 } })

feature("a false condition over HTTP", {
    when() { return { res: http.get("/Login") } },
    then({ res }) { res.status == 404 },
})

feature("a skip is not a failure", { expect() { skip("not here") } })
