// Knowledge and notes (org.moqui.impl.WikiServices, IronStone M9) on the real database: a note in the user's notes space
// (R22) moved into a space in a git repository with its history replayed, a page's links and backlinks across a rename,
// a review approved and refused from the notification tray, and search that leaves out another user's notes. The rules
// in full are the Rust tests in crates/moqui-core/src/service/wiki_tests.rs.

const WIKI = "org.moqui.impl.WikiServices"
const NOTIFY = "org.moqui.impl.NotificationServices"
// space ids are 3 to 8 characters; a suffix keeps each run's spaces apart on a database kept between runs
const RUN = Date.now().toString(36).slice(-4).toUpperCase()
const KB = "SK" + RUN
const LNK = "SL" + RUN

const wiki = (verb, params) => call(`${WIKI}.${verb}`, params || {})
const user = (userId) => {
    if (ec.entity.find("moqui.security.UserAccount").condition("userId", userId).count() > 0) return
    const row = ec.entity.makeValue("moqui.security.UserAccount")
    row.setFields({ userId: userId, username: userId, userFullName: `${userId} Example` }, true, null, null)
    row.create()
}
const save = (space, path, text, note) => {
    const slash = path.lastIndexOf("/")
    return wiki("update#WikiPage", { wikiSpaceId: space, parentPath: slash > 0 ? path.slice(0, slash) : "",
        pageName: path.slice(slash + 1), pageText: text, versionNote: note || null }).wikiPageId
}
const approvalFor = (reviewId) => call(`${NOTIFY}.get#UserNotifications`, { approvals: true }).notificationList
    .find((row) => row.recordId === reviewId)

spec({
    setup() { ec.artifactExecution.disableAuthz() },
    cleanup() { ec.artifactExecution.enableAuthz() },
})

feature("a note moves into a git space with its history replayed", {
    when() {
        for (const id of ["wikiAuthor", "wikiReviewer", "wikiOutsider"]) user(id)
        ec.user.internalLoginUser("wikiAuthor")
        const notes = wiki("ensure#UserNotesSpace").wikiSpaceId
        const pageId = save(notes, "Groceries", "# Groceries\n\n- milk\n", "First list")
        save(notes, "Groceries", "# Groceries\n\n- milk\n- bread\n", null)
        save(notes, "Groceries", "# Groceries\n\n- milk\n- bread\n- eggs\n", "Added eggs")
        const made = wiki("create#WikiSpace", { wikiSpaceId: KB, description: "Spec knowledge", repositoryKind: "db" })
        const moved = wiki("move#WikiPage", { wikiPageId: pageId, toWikiSpaceId: KB })
        const history = wiki("get#WikiPageHistory", { wikiPageId: pageId })
        const byOldPath = wiki("get#WikiPageInfo", { wikiSpaceId: notes, pagePath: "Groceries" })
        return { notes, made, moved, history, byOldPath }
    },
    then({ notes, made, moved, history, byOldPath }) {
        notes == "NOTES_wikiAuthor"
        made.repositoryId == `WIKI_${KB}`
        moved.replayed == true
        moved.versionCount == 3
        history.storage == "git"
        history.versionList.map((version) => version.note.split("\n")[0]) == ["Added eggs", "Version 02", "First list"]
        history.versionList[0].note.includes(`Moved from ${notes}/Groceries, version 03`)
        history.versionList[0].versionName.length == 40
        history.versionList[0].movedFrom == `${notes}/Groceries`
        history.versionList.every((version) => version.userId === "wikiAuthor")
        byOldPath.pageLocation == `git://WIKI_${KB}/${KB}/Groceries.md`
    },
})

feature("backlinks follow a page across a rename", {
    when() {
        ec.user.internalLoginUser("wikiAuthor")
        wiki("create#WikiSpace", { wikiSpaceId: LNK, description: "Spec links" })
        const alpha = save(LNK, "Alpha", "# Alpha\n")
        save(LNK, "Beta", "See [alpha](Alpha), [[Missing]] and [a site](https://example.com).\n")
        wiki("update#WikiPage", { wikiSpaceId: LNK, wikiPageId: alpha, pageName: "First", pageText: "# Alpha\n" })
        return {
            backlinks: wiki("get#WikiPageBacklinks", { wikiPageId: alpha }).backlinkList,
            links: wiki("get#WikiPageLinks", { wikiSpaceId: LNK, pagePath: "Beta" }).linkList,
            byAlias: wiki("get#WikiPageInfo", { wikiSpaceId: LNK, pagePath: "Alpha" }),
        }
    },
    then({ backlinks, links, byAlias }) {
        backlinks.map((row) => row.pagePath) == ["Beta"]
        links.map((row) => row.toPagePath) == ["First", "Missing"]
        links.map((row) => row.found) == [true, false]
        byAlias.pagePath == "First"
    },
})

feature("a review is approved and refused from the tray", {
    when() {
        ec.user.internalLoginUser("wikiAuthor")
        const page = save(LNK, "Policy", "one\n")
        save(LNK, "Policy", "two\n")
        const first = wiki("request#WikiPageReview", { wikiPageId: page, reviewerUserId: "wikiReviewer", note: "Please check" })
        ec.user.internalLoginUser("wikiReviewer")
        const toDo = wiki("find#WikiPageReviews", { wikiSpaceId: LNK }).reviewList
        const notification = approvalFor(first.wikiPageReviewId)
        call(`${NOTIFY}.approve#NotificationMessage`, { notificationMessageId: notification.notificationMessageId })
        ec.user.internalLoginUser("wikiAuthor")
        save(LNK, "Policy", "three\n")
        const second = wiki("request#WikiPageReview", { wikiPageId: page, reviewerUserId: "wikiReviewer" })
        ec.user.internalLoginUser("wikiReviewer")
        call(`${NOTIFY}.refuse#NotificationMessage`, { notificationMessageId: approvalFor(second.wikiPageReviewId).notificationMessageId,
            reason: "Fix the typo" })
        const left = wiki("find#WikiPageReviews", { wikiSpaceId: LNK }).reviewList
        ec.user.internalLoginUser("wikiAuthor")
        const decisions = call(`${NOTIFY}.get#UserNotifications`, { topic: "WikiPageReviewDecision" }).notificationList
        const reviews = wiki("find#WikiPageReviews", { filter: "requested", wikiSpaceId: LNK, statusId: "any" }).reviewList
        const published = ec.entity.find("moqui.resource.wiki.WikiPage").condition("wikiPageId", page).one().publishedVersionName
        return { toDo, notification, left, decisions, reviews, published }
    },
    then({ toDo, notification, left, decisions, reviews, published }) {
        toDo.map((row) => row.pageName) == ["Policy"]
        toDo[0].canDecide == true
        notification.canApprove == true
        notification.refuseReasonRequired == true
        left.length == 0
        published == "02"
        reviews.map((row) => row.statusId) == ["WprChangesRequested", "WprApproved"]
        reviews[0].decisionNote == "Fix the typo"
        decisions.map((row) => row.titleText) == ["wikiReviewer Example asked for changes to Policy", "wikiReviewer Example approved Policy"]
    },
})

feature("search leaves out another user's notes", {
    when() {
        ec.user.internalLoginUser("wikiAuthor")
        save("NOTES_wikiAuthor", `Secret recipe ${RUN}`, "# Secret recipe\n")
        const mine = wiki("search#WikiPages", { text: `secret recipe ${RUN}` }).pageList
        ec.user.internalLoginUser("wikiOutsider")
        const theirs = wiki("search#WikiPages", { text: `secret recipe ${RUN}` }).pageList
        return { mine, theirs }
    },
    then({ mine, theirs }) {
        mine.map((row) => row.wikiSpaceId) == ["NOTES_wikiAuthor"]
        theirs.length == 0
    },
})
