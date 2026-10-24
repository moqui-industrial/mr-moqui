// A01 Broken Access Control on the notification socket (/notws), ported from Moqui Java's pytest proofs
// (framework/test/test_a01_ws.py; the catalog in README.md). An anonymous upgrade is 401 here, where Java accepts it
// and ignores the null user, so the foreign-origin proof uses a logged-in session: the origin, not the missing user,
// must be what refuses it. That proof needs a server that allows no foreign origin (the production conf) and skips on
// the dev conf, which allows every origin; SecurityCors fails there.

const EVIL_ORIGIN = "https://evil.example"

// test_a01_ws.py: test_websocket_evil_origin_is_rejected
feature("A01 a /notws upgrade from a foreign Origin is refused, with a logged-in session", {
    given() {
        const probe = session().get("/Login", { headers: { Origin: EVIL_ORIGIN } })
        if (probe.status != 401) skip("this server allows every origin (the dev conf): run against MoquiProductionConf.xml or " +
            "MoquiLoadTestConf.xml, as --serve does")
    },
    when() {
        const s = session({ user: params.user ?? "john.doe", password: params.password ?? "moqui" })
        const evil = ws.connect("/notws", { session: s, origin: EVIL_ORIGIN })
        const evilOpened = evil.open
        evil.close()
        // the control: the same session opens it from the server's own origin
        const own = ws.connect("/notws", { session: s, origin: server.url.replace(/\/+$/, "") })
        const ownOpened = own.open
        own.close()
        return { evil, evilOpened, own, ownOpened }
    },
    then({ evil, evilOpened, own, ownOpened }) {
        !evilOpened
        evil.status == 403
        ownOpened
        own.status == 101
    },
})

// Replaces test_a01_ws.py: test_notws_anonymous_upgrade_accepts_or_closes (Java accepts it)
feature("A01 an anonymous /notws upgrade is 401", {
    when() {
        const socket = ws.connect("/notws", { session: session() })
        const opened = socket.open
        socket.close()
        return { socket, opened }
    },
    then({ socket, opened }) {
        !opened
        socket.status == 401
    },
})
