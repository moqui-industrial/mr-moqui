// load and the client's parallel: a tiny load timed by name, with an expected 401 that is not an error, a response
// that is (and ends its iteration), and a throw of the scenario; then threads that each log in on a session of their
// own, one of them throwing.
const ARGS = { user: params.user ?? "john.doe", password: params.password ?? "moqui" }

feature("a tiny load times its requests by name", {
    when() {
        return load({
            users: 2, iterations: 3, think: 10, args: ARGS,
            setup(user, args) { login(args.user, args.password) },
            scenario(user, args, iteration) {
                http.get("/rest/s1/moqui/basic/enumTypes", { accept: "json", name: "enum types" })
                session().get("/rest/s1/moqui/basic/enumTypes", { accept: "json", expect: [401] })
                if (user == 1 && iteration == 2) throw new Error("planned")
            },
        })
    },
    then(stats) {
        stats.users == 2
        stats.iterations == 6
        stats.requests == 12
        stats.errors == 1
        stats.errorRate > 0
        stats.names["enum types"].count == 6
        stats.names["enum types"].statuses["200"] == 6
        stats.names["GET /rest/s1/moqui/basic/enumTypes"].statuses["401"] == 6
        stats.names["GET /rest/s1/moqui/basic/enumTypes"].errors == 0
        stats.errorSamples[0].message == "Error: planned"
        stats.latency.max >= stats.latency.p50
    },
})

feature("an error response ends its iteration", {
    when() {
        return load({
            users: 1, iterations: 2,
            scenario() {
                http.get("/rest/s1/moqui/basic/enumTypes", { accept: "json", name: "anonymous" })
                http.get("/Login", { name: "never" })
            },
        })
    },
    then(stats) {
        stats.requests == 2
        stats.errors == 2
        stats.names["anonymous"].statuses["401"] == 2
        stats.names["never"] == null
        stats.errorSamples[0].status == 401
    },
})

feature("parallel threads log in on sessions of their own", {
    when() {
        return parallel(3, (index, args) => {
            login(args.user, args.password)
            if (index == 2) throw new Error("thread two throws")
            return { index, cookie: http.cookies().some((c) => c.name == "moqui.session") }
        }, ARGS)
    },
    then({ results, errors }) {
        results.length == 3
        results[0].index == 0
        results[1].cookie
        results[2] == null
        errors.length == 1
        errors[0].startsWith("thread 2: Error: thread two throws")
    },
})
