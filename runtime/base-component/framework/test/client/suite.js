// The order the framework's client specs run in (moqui test-client framework), and the server they run against with
// --serve: the framework alone under the production conf (no foreign origin allowed, the tarpits on), with the users
// test/ClientFixtures.spec.js makes (plans/ClientTestHarness.md 6, 7). LoadBasic joins the order with P4.
suite(["SecurityAccess", "SecurityHeaders", "SecurityCors", "SecurityAuthn", "SecurityErrors", "SecurityWebSocket"], {
    server: { components: ["framework"], specs: ["ClientFixtures"], conf: "conf/MoquiProductionConf.xml" },
})
