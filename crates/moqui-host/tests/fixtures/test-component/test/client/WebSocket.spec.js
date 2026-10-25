// The notification socket: an anonymous upgrade is refused with its status, and a logged-in session's cookies open
// it.
feature("an anonymous /notws upgrade is refused, not thrown", {
    when() { return { socket: ws.connect("/notws", { session: session() }) } },
    then({ socket }) {
        !socket.open
        socket.status == 401
    },
})

feature("a logged-in session opens /notws, sends and closes", {
    when() {
        const s = session({ user: "john.doe" })
        const socket = ws.connect("/notws", { session: s })
        socket.send("subscribe:ALL")
        const frame = socket.receive({ timeout: 0.3 })
        socket.close()
        return { socket, frame }
    },
    then({ socket, frame }) {
        socket.status == 101
        frame == null
        !socket.open
    },
})
