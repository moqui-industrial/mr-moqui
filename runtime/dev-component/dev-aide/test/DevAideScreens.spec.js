// The Dev AIDE screens on a dev instance (plans/design-dev-aide.md P2): Home and Settings render for the full tier in the
// browser's mode and the terminal's (their forms and lists are plain, so `moqui tui` reaches them), and the Workspace
// answers its roots as JSON. The services' own behaviour (saves, the upstream lock, git, Publish) is in the host's
// integration test (crates/moqui-host/tests/it/devaide.rs), which makes scratch repositories.
// Run: ./moq test dev-aide
const dev = () => ec.screen.makeTest().rootScreen("component://dev-aide/screen/DevAide.xml")

spec({
    setupSpec() { login("john.doe") },
})

feature("Home renders the roots and the reload log", {
    expect() {
        const page = dev().render("Home", null, null)
        page.status == 200
        page.errorMessages.length == 0
        page.output.includes("RootList")
        page.output.includes("Reload log")
    },
})

feature("Settings renders the site components and the roots", {
    expect() {
        const page = dev().render("Settings", null, null)
        page.status == 200
        page.errorMessages.length == 0
        page.output.includes("SiteComponentsForm")
        page.output.includes("RootSettings")
    },
})

feature("Home and Settings render in the terminal", {
    expect() {
        const home = dev().renderMode("tui").render("Home", null, null)
        const settings = dev().renderMode("tui").render("Settings", null, null)
        home.status == 200 && home.errorMessages.length == 0
        settings.status == 200 && settings.errorMessages.length == 0
        home.output.includes("Reload log")
    },
})

feature("the Workspace answers its roots", {
    expect() {
        const answer = dev().render("Workspace/roots", null, null)
        const roots = JSON.parse(answer.output)
        answer.status == 200
        roots.tier == "full"
        roots.roots.some((root) => root.components.some((component) => component.name == "dev-aide"))
    },
})
