// The screen archetype gallery (screen/Gallery.xml): every gallery screen renders in its render mode without errors
// and with real content. The screens are the fixture of the IronStone BOSS style guide
// (runtime/component/ironstone-boss/plan/style-guide.md §10); a placeholder or a render error fails here.
// Run: ./moq test example
let screens

// name under the Gallery root, and the render mode the screen is written for (qvue screens answer <path>.qvue)
const galleryScreens = [
    ["Landing", "qvt"], ["List", "qvt"], ["Detail", "qvt"], ["Record360", "qvt"], ["Task", "qvt"],
    ["Board", "qvue"], ["Calendar", "qvue"], ["Split", "qvt"], ["Settings", "qvt"], ["Wizard", "qvt"],
    ["Dialog", "qvt"], ["Report", "qvt"], ["Panel", "qvt"], ["Widgets", "qvt"], ["Editor", "qvt"],
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

// The Editor archetype writes into its demo wiki spaces: a save makes a version, compare renders the server diff,
// an old version renders by location#version, and a restore makes the old text the newest version. Each storage:
// EXAMPLE (dbresource history) and EXGIT (a git repository in the database, made by the screen's createGitSpace).
const editor = (path, parameters, method) => {
    const test = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt")
    return test.render("Editor" + (path ? "/" + path : ""), Object.assign({ lastStandalone: "-2" }, parameters || {}), method || null)
}
const pageText = (space, path) => ec.resource.getLocationReference(
    ec.entity.find("moqui.resource.wiki.WikiSpace").condition("wikiSpaceId", space).one().rootPageLocation
        .replace(/\.md$/, "") + "/" + path + ".md")

feature("the editor's #space space keeps versions, compares, views and restores them", {
    where: [{ space: "EXAMPLE" }, { space: "EXGIT" }],
    expect({ space }) {
        if (space === "EXGIT" && !ec.entity.find("moqui.resource.wiki.WikiSpace").condition("wikiSpaceId", "EXGIT").one())
            editor("createGitSpace", {}, "post")
        const page = pageText(space, "Welcome")
        const before = page.getVersionHistory().length
        const saved = editor("savePage", { wikiSpaceId: space, pagePath: "Welcome", pageText: "# Welcome\n\nSaved by the gallery spec.\n" }, "post")
        const history = page.getVersionHistory()
        const newest = history[0].versionName, previous = history[1].versionName
        const compared = editor("", { wikiSpaceId: space, pagePath: "Welcome", versionNameLhs: previous, versionNameRhs: newest })
        const viewed = editor("", { wikiSpaceId: space, pagePath: "Welcome", versionName: previous })
        // a plain line only the old version has; the Edit dialog holds the current text, so look for the old one
        const oldLine = page.getText(previous).split("\n").map((line) => line.replace(/^[-#\s]+/, ""))
            .find((line) => line.length > 10 && !line.includes("*"))
        editor("restoreVersion", { wikiSpaceId: space, pagePath: "Welcome", versionName: previous }, "post")
        const restored = page.getVersionHistory()
        saved.errorMessages.length == 0
        history.length == before + 1
        compared.status == 200
        compared.errorMessages.length == 0
        compared.output.includes("moqui-diff")
        compared.output.includes("Saved by the gallery spec.")
        viewed.status == 200
        viewed.output.includes("not the current text")
        viewed.output.includes(oldLine)
        restored.length == before + 2
        page.getText() == page.getText(previous)
    },
})
