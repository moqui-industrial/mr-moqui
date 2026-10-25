// skip(): a spec whose setupSpec skips its features when a fixture it needs is missing, as the Python suite skips.
spec({
    setupSpec() {
        const fixture = session().get("/rest/s1/moqui/basic/enumTypes", { basic: ["moqui.test.no.fixture", "x"] })
        if (fixture.status == 401) skip("the moqui.test.no.fixture user is not on this server")
    },
})

feature("needs the fixture user", { expect() { false } })
