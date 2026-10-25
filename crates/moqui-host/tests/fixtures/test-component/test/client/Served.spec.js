// The server --serve starts: the data its fixture spec made (Basics) and its test/MoquiConf.xml, over HTTP. Against a
// server Basics did not run on, the spec skips, as a client spec does when its fixtures are missing.
const fixtureType = "/rest/s1/moqui/basic/enumTypes/FixtureType"

spec({
    setupSpec() {
        const res = session().get(fixtureType, { basic: ["john.doe", "moqui"], accept: "json" })
        // a missing record is an empty 200
        if (res.status != 200 || !res.text.includes("FixtureType")) skip("the fixtures of the Basics spec are not on this server (moqui test-client --serve)")
    },
})

feature("the enum type Basics made is there", {
    when() { return { res: session().get(fixtureType, { basic: ["john.doe", "moqui"], accept: "json" }) } },
    then({ res }) {
        res.status == 200
        res.json().description == "Fixture Type"
    },
})

feature("the component's test/MoquiConf.xml is merged", {
    when() { return { res: http.get("/Login") } },
    then({ res }) {
        res.status == 200
        res.header("X-Moqui-Test-Conf") == "moqui-test-fixture"
    },
})
