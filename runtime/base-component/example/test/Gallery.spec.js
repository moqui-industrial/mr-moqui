// The screen archetype gallery (screen/Gallery.xml): every gallery screen renders in its render mode without errors
// and with real content. The screens are the fixture of the IronStone BOSS style guide
// (runtime/component/ironstone-boss/plan/style-guide.md §10); a placeholder or a render error fails here.
// Run: ./moq test example
let screens

// name under the Gallery root, and the render mode the screen is written for (qvue screens answer <path>.qvue)
const galleryScreens = [
    ["Landing", "qvt"], ["List", "qvt"], ["Detail", "qvt"], ["Record360", "qvt"], ["Task", "qvt"],
    ["Board", "qvue"], ["Calendar", "qvue"], ["Split", "qvt"], ["Settings", "qvt"], ["Wizard", "qvt"],
    ["Dialog", "qvt"], ["Report", "qvt"], ["Panel", "qvt"], ["Widgets", "qvt"], ["Editor", "qvt"], ["QuickNote", "qvt"],
    ["Chatter", "qvt"], ["RecordFiles", "qvt"], ["Message", "qvt"],
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

// The Landing counters are number cards in a dashboard, each a link into the List with its type filter.
feature("the Landing counters are number cards in a dashboard that link to the filtered list", {
    expect() {
        const test = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt")
        const str = test.render("Landing", { lastStandalone: "-2" }, null)
        const cards = str.output.match(/<m-number-card /g) || []
        const linked = /<m-number-card title="[^"]+" value="[0-9,]+" href="\/[^"]*List\?enumTypeId=[A-Za-z]+"/.test(str.output)
        str.status == 200
        str.output.includes('<div class="m-dashboard" style="--m-dashboard-min:150px">')
        str.output.includes('m-dashboard-each')
        str.output.includes('<m-number-card title="All enumerations"')
        cards.length == 6
        linked
        !str.output.includes("text-overline text-grey-6")
    },
})

// The gallery's shell-providers (Gallery.xml) as the shell gets them from the shellData transition of the root screen, and its
// palette source run by shellSearch; the Detail screen's context is an m-screen-context with the record.
feature("the shell data has the gallery's providers, and its palette source finds enumerations", {
    expect() {
        const webroot = ec.screen.makeTest().rootScreen("component://webroot/screen/webroot.xml")
        const data = webroot.render("shellData", {}, null)
        const apps = JSON.parse(data.output).apps
        log.info(`Shell data: ${apps.length} apps (${apps.map((app) => app.name).join(", ")}) in ${data.renderTime}ms`)
        const gallery = apps.find((app) => app.name === "gallery")
        const search = webroot.render("shellSearch", { text: "Quasar", groups: "GalleryEnumerations" }, null)
        const groups = JSON.parse(search.output).groups
        const detail = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt")
            .render("Detail", { lastStandalone: "-2", enumId: "STT_INTERNAL_QUASAR" }, null)
        data.status == 200
        data.errorMessages.length == 0
        gallery.providers.quickCreate[0].url == "/qapps/gallery/QuickNote"
        gallery.providers.paletteActions[0].url == "/qapps/gallery/Widgets"
        gallery.providers.paletteSources[0].group == "GalleryEnumerations"
        gallery.providers.paletteSources[0].service === undefined
        gallery.providers.keymaps[0].keys[0].url == "/qapps/gallery/Widgets"
        gallery.menu.some((item) => item.path == "/qapps/gallery/Widgets" && item.depth == 1)
        search.status == 200
        groups.length == 1
        groups[0].rows.length > 0
        groups[0].rows[0].url.startsWith("/qapps/gallery/Detail?enumId=")
        groups[0].moreUrl == "/qapps/gallery/List?description=Quasar"
        detail.output.includes('<m-screen-context record-entity="moqui.basic.Enumeration" record-id="STT_INTERNAL_QUASAR"')
        detail.output.includes('<m-chatter id="DetailChatter" record-entity="moqui.basic.Enumeration" record-id="STT_INTERNAL_QUASAR" default-tab="log">')
        detail.output.includes('<m-record-files id="DetailFiles" record-entity="moqui.basic.Enumeration" record-id="STT_INTERNAL_QUASAR"')
        // every app root gets its record panels; the gallery mounts its own Chatter and RecordFiles under the names a theme may give
        apps.every((app) => app.recordScreens && "chatter" in app.recordScreens && "recordFiles" in app.recordScreens)
    },
})

// The gallery's Chatter panel, as the chatter element loads it: the record and the hints from the query, the record card's
// transition, and the Log note form with the light rich-text editor whose @ mentions search users. Its note service cleans the
// editor's HTML with allow-html="safe", which keeps the mention span, and reads the mentions back.
feature("the gallery's Chatter panel has the rich-text note form, its card and mention transitions, and keeps a mention", {
    expect() {
        const gallery = () => ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt")
        const record = { recordEntity: "moqui.basic.Enumeration", recordId: "STT_INTERNAL_QUASAR", appRoot: "gallery", defaultTab: "log" }
        const panel = gallery().render("Chatter", Object.assign({ lastStandalone: "-2" }, record), null)
        const card = JSON.parse(gallery().render("Chatter/recordCard", record, null).output)
        const mentions = JSON.parse(gallery().render("Chatter/mentionSearch", { term: "john" }, null).output)
        const html = '<p>Hi <span data-mention-id="EX_JOHN_DOE">@John Doe</span> <strong>now</strong><script>x()</script></p>'
        const cleaned = ec.service.sync().name("example.ExampleServices.clean#GalleryNote").parameters({ noteText: html }).call()
        panel.status == 200
        panel.errorMessages.length == 0
        panel.output.includes('editor-type="rich-light"')
        panel.output.match(/mention-url="[^"]*\/Chatter\/mentionSearch"/) !== null
        panel.output.includes("<m-record-card ")
        panel.output.includes("opens on log")
        card.title == "Internal Applications - Quasar"
        card.status.label == "Active"
        mentions.some((row) => row.id == "EX_JOHN_DOE" && row.label == "John Doe")
        cleaned.noteText == '<p>Hi <span data-mention-id="EX_JOHN_DOE">@John Doe</span> <strong>now</strong></p>'
        cleaned.mentionIds.length == 1
        cleaned.mentionIds[0] == "EX_JOHN_DOE"
    },
})

// The Split archetype is a container-split whose selection is the geoId of the URL, and the side list keeps its find.
feature("the Split is a container-split with the country in the URL", {
    expect() {
        const test = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt")
        const str = test.render("Split", { lastStandalone: "-2", geoId: "CAN", geoName: "C", geoName_op: "begins" }, null)
        str.status == 200
        str.errorMessages.length == 0
        str.output.includes('<m-container-split id="CountrySplit" side-width="320" min-main="480" select-parameter="geoId" back-text="Countries">')
        str.output.includes("<template v-slot:side>")
        str.output.includes("<template v-slot:main>")
        str.output.includes("Canada")
        // the find narrows the side list, the selection does not
        str.output.includes("Cameroon")
        !str.output.includes("Albania")
    },
})

// The Wizard archetype: subscreens-panel type="wizard" over three step screens, each of whose forms posts to its own
// transition. A step's answer with errors keeps the wizard on it; the draft the steps keep reaches the review, and the review's
// answer names the first step again.
feature("the Wizard's steps check their entries, keep a draft and end on the first step", {
    expect() {
        const wizard = (path, parameters, method) => ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt")
            .render("Wizard" + (path ? "/" + path : ""), Object.assign({ lastStandalone: "-2" }, parameters || {}), method || null)
        const panel = ec.screen.makeTest().rootScreen("component://example/screen/Gallery.xml").renderMode("qvt").render("Wizard", { lastStandalone: "-1" }, null)
        const empty = wizard("Customer/saveCustomer", { customerName: "" }, "post")
        const saved = wizard("Customer/saveCustomer", { customerName: "Spec Customer", emailAddress: "spec@example.com" }, "post")
        const options = wizard("Options/saveOptions", { countryGeoId: "CAN", currencyUomId: "CAD", newsletter: "Y" }, "post")
        const review = wizard("Review", {}, null)
        const created = wizard("Review/createCustomer", {}, "post")
        const after = wizard("Review", {}, null)
        panel.status == 200
        panel.output.includes('<m-subscreens-wizard id="CustomerWizard" finish-text="Create customer"></m-subscreens-wizard>')
        empty.errorMessages.join(" ").includes("Enter the customer name")
        saved.errorMessages.length == 0
        options.errorMessages.length == 0
        review.output.includes("Spec Customer")
        review.output.includes("Canada")
        created.errorMessages.length == 0
        !after.output.includes("Spec Customer")
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
