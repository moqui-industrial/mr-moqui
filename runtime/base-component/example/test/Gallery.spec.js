// The screen archetype gallery (screen/Gallery.xml): every gallery screen renders in its render mode without errors
// and with real content. The screens are the fixture of the IronStone BOSS style guide
// (runtime/component/ironstone-boss/plan/style-guide.md §10); a placeholder or a render error fails here.
// Run: ./moq test example
let screens

// name under the Gallery root, and the render mode the screen is written for (qvue screens answer <path>.qvue)
const galleryScreens = [
    ["Landing", "qvt"], ["List", "qvt"], ["Detail", "qvt"], ["Record360", "qvt"], ["Task", "qvt"],
    ["Board", "qvue"], ["Calendar", "qvue"], ["Split", "qvt"], ["Settings", "qvt"], ["Wizard", "qvt"],
    ["Dialog", "qvt"], ["Report", "qvt"], ["Panel", "qvt"], ["Widgets", "qvt"],
]

spec({
    setupSpec() {
        login("john.doe")
        screens = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml")
    },
    cleanupSpec() {
        log.info(`Rendered ${screens.renderCount} gallery screens (${screens.errorCount} errors) in ${(Date.now() - screens.startTime) / 1000}s`)
    },
    setup() { ec.artifactExecution.disableAuthz() },
    cleanup() { ec.artifactExecution.enableAuthz() },
})

feature("the gallery root lists every archetype screen", {
    when() {
        return screens.render("", {}, null)
    },
    then(str) {
        str.status == 200
        str.errorMessages.length == 0
    },
})

feature("gallery screen #name renders in #mode without errors", {
    where: galleryScreens.map(([name, mode]) => ({ name, mode })),
    expect({ name, mode }) {
        const test = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode(mode)
        // the screen as the shell fetches it: a fragment of the last standalone screen, not the root placeholder
        const str = test.render(name, { lastStandalone: "-2" }, null)
        log.info(`Rendered gallery ${name} (${mode}) in ${str.renderTime}ms, ${str.output?.length} characters`)
        str.status == 200
        str.errorMessages.length == 0
        str.output.length > 200
        !str.output.includes("is not written yet")
        !/Error rendering|Exception in render/.test(str.output)
    },
})
