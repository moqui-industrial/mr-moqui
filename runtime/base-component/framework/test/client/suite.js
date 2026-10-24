// The order the framework's client specs run in (moqui test-client framework), and the server they run against with
// --serve: the framework alone under the load test conf, which is the production conf (no foreign origin allowed) with
// the screen, transition and service tarpits off so LoadBasic is not answered 429, with the users
// test/ClientFixtures.spec.js makes (plans/ClientTestHarness.md 6, 7, 8). The tarpit proofs, when they are ported, need
// a server with the tarpits on.
suite(["SecurityAccess", "SecurityHeaders", "SecurityCors", "SecurityAuthn", "SecurityErrors", "SecurityWebSocket",
    "LoadBasic"], {
    server: { components: ["framework"], specs: ["ClientFixtures"], conf: "conf/MoquiLoadTestConf.xml" },
})
