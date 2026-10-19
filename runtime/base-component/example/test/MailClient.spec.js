// The framework's mail client (org.moqui.impl.EmailClientServices) on the real database: the SQL the message list
// writes (case-insensitive LIKE, NOT IN with nulls, the date cursor), filing and a delete forever under the foreign
// keys, a send that cannot connect waiting in the Outbox, a send that goes (through the mail capture, docs/Testing.md
// "Mail"), and removing an account with its mail. The access rules, the views and the sends in full are
// the Rust tests in crates/moqui-core/src/service/mail/client_tests.rs.

const SERVICES = "org.moqui.impl.EmailClientServices"
const base = Date.now() - 10 * 86400000
let accountId
let inboxId

const mail = (verb, params) => call(`${SERVICES}.${verb}`, params || {})
const threadIds = (out) => out.threadList.map((row) => row.threadId)
const user = (userId) => {
    if (ec.entity.find("moqui.security.UserAccount").condition("userId", userId).count() > 0) return
    const row = ec.entity.makeValue("moqui.security.UserAccount")
    row.setFields({ userId: userId, username: userId, emailAddress: `${userId}@example.com` }, true, null, null)
    row.create()
}
const message = (id, kind, offset, fields) => {
    const folder = kind ? ec.entity.find("moqui.basic.email.EmailFolder").condition("emailAccountId", accountId)
        .condition("folderKindEnumId", kind).one() : null
    const row = ec.entity.makeValue("moqui.basic.email.EmailMessage")
    row.setFields(Object.assign({ emailMessageId: id, emailAccountId: accountId, emailFolderId: folder ? folder.emailFolderId : null,
        statusId: "ES_RECEIVED", subject: `Subject ${id}`, body: `<p>Body of ${id}</p>`, snippet: `Body of ${id}`,
        fromAddress: `${id.toLowerCase()}@sender.example.com`, toAddresses: "team@example.com", messageDate: new Date(base + offset * 1000),
        isRead: "N" }, fields || {}), true, null, null)
    row.create()
}

spec({
    setup() { ec.artifactExecution.disableAuthz() },
    cleanup() { ec.artifactExecution.enableAuthz() },
})

feature("an account with its folders, and the message list in threads with its cursor", {
    when() {
        for (const id of ["mailClientOwner", "mailClientStranger"]) user(id)
        ec.user.internalLoginUser("mailClientOwner")
        // no server listens on port 1, so a send fails at once
        accountId = mail("create#EmailAccount", { accountKindEnumId: "EakShared", emailAddress: "team@example.com", accountName: "Team",
            smtpHost: "127.0.0.1", smtpPort: "1", smtpStartTls: "N" }).emailAccountId
        inboxId = ec.entity.find("moqui.basic.email.EmailFolder").condition("emailAccountId", accountId)
            .condition("folderKindEnumId", "EfkInbox").one().emailFolderId
        message("MCM1", "EfkInbox", 1, { messageId: "<mcm1@sender.example.com>" })
        message("MCM2", "EfkInbox", 30, { rootEmailMessageId: "MCM1", parentEmailMessageId: "MCM1" })
        message("MCM3", "EfkInbox", 20, { isFlagged: "Y", subject: "Invoice 42" })
        message("MCM4", "EfkInbox", 40, { snoozedThruDate: new Date(Date.now() + 86400000) })
        message("MCM5", "EfkTrash", 50)
        message("MCM6", null, 60, { statusId: "ES_SCHEDULED", scheduledSendDate: new Date(Date.now() + 86400000), isRead: "Y" })
        const inbox = mail("find#Messages")
        const page = mail("find#Messages", { pageSize: 1 })
        const next = mail("find#Messages", { pageSize: 1, beforeDate: page.nextBeforeDate })
        const accounts = mail("get#UserEmailAccounts").accountList
        return { inbox, page, next, accounts,
            search: mail("find#Messages", { searchText: "INVOICE", threadMode: false }),
            flagged: mail("find#Messages", { view: "flagged" }), all: mail("find#Messages", { view: "all" }),
            scheduled: mail("find#Messages", { view: "outbox" }), snoozed: mail("find#Messages", { view: "snoozed" }),
            unread: mail("count#UnreadEmail") }
    },
    then({ inbox, page, next, accounts, search, flagged, all, scheduled, snoozed, unread }) {
        threadIds(inbox) == ["MCM1", "MCM3"]
        inbox.threadList[0].messageCount == 2
        inbox.threadList[0].store == "email"
        page.hasMore == true
        threadIds(next) == ["MCM3"]
        accounts.length == 2
        accounts[0].accountKindEnumId == "EakInternal"
        accounts[1].folderList.length == 6
        accounts[1].unreadCount == 3
        search.threadList.map((row) => row.emailMessageId) == ["MCM3"]
        threadIds(flagged) == ["MCM3"]
        threadIds(all) == ["MCM6", "MCM4", "MCM1", "MCM3"]
        threadIds(scheduled) == ["MCM6"]
        threadIds(snoozed) == ["MCM4"]
        unread.unreadCount == 3
        unread.unreadThreadCount == 2
    },
})

feature("a stranger reaches nothing", {
    when() {
        ec.user.internalLoginUser("mailClientStranger")
        const access = mail("get#EmailAccountAccess", { emailAccountId: accountId })
        ec.service.sync().name(`${SERVICES}.get#Thread`).parameter("emailMessageId", "MCM1").call()
        const refused = ec.message.errorsString
        ec.message.clearErrors()
        return { access, refused }
    },
    then({ access, refused }) {
        access.level == 0
        refused.includes("not found")
    },
})

feature("filing, and a delete forever that keeps the rest of the thread", {
    when() {
        ec.user.internalLoginUser("mailClientOwner")
        mail("set#MessageFlags", { rootEmailMessageId: "MCM1", read: true })
        const folderId = mail("create#EmailFolder", { emailAccountId: accountId, folderName: "Clients" }).emailFolderId
        mail("move#Messages", { emailMessageId: "MCM3", emailFolderId: folderId })
        const labelId = mail("create#EmailLabel", { emailAccountId: accountId, labelName: "Follow up" }).emailLabelId
        mail("apply#Labels", { rootEmailMessageId: "MCM1", addLabelIdList: [labelId] })
        mail("archive#Messages", { rootEmailMessageId: "MCM1" })
        const archived = mail("find#Messages", { view: "archive" })
        mail("archive#Messages", { rootEmailMessageId: "MCM1", unarchive: true })
        mail("trash#Messages", { emailMessageId: "MCM1" })
        const deleted = mail("trash#Messages", { emailMessageId: "MCM1", deleteForever: true })
        const reply = ec.entity.find("moqui.basic.email.EmailMessage").condition("emailMessageId", "MCM2").one()
        return { archived, deleted, reply, gone: ec.entity.find("moqui.basic.email.EmailMessage").condition("emailMessageId", "MCM1").count(),
            labeled: mail("find#Messages", { emailLabelId: labelId }), unread: mail("count#UnreadEmail") }
    },
    then({ archived, deleted, reply, gone, labeled, unread }) {
        threadIds(archived) == ["MCM1"]
        deleted.deletedCount == 1
        gone == 0
        reply.rootEmailMessageId == null
        reply.parentEmailMessageId == null
        threadIds(labeled) == ["MCM2"]
        unread.unreadCount == 0
    },
})

feature("a send that cannot connect waits in the Outbox, and goes back to Drafts", {
    when() {
        // a real connection, to port 1, where nothing listens: the capture is off for this send, whatever the run says
        const capture = ec.factory.getTool("MailCapture")
        capture.disable()
        try {
            const draftId = mail("save#Draft", { emailAccountId: accountId, parentEmailMessageId: "MCM2", toAddresses: "\"Sender\" <mcm2@sender.example.com>",
                body: "<p>Thanks</p>" }).emailMessageId
            const sent = mail("send#Draft", { emailMessageId: draftId })
            const outbox = mail("find#Messages", { view: "outbox", threadMode: false })
            mail("cancel#ScheduledSend", { emailMessageId: draftId })
            const draft = ec.entity.find("moqui.basic.email.EmailMessage").condition("emailMessageId", draftId).one()
            return { sent, outbox, draft, draftId }
        } finally {
            capture.reset()
        }
    },
    then({ sent, outbox, draft, draftId }) {
        sent.sent == false
        sent.statusId == "ES_READY"
        sent.sendError != null
        outbox.threadList.some((row) => row.emailMessageId == draftId && row.sendError != null)
        draft.statusId == "ES_DRAFT"
        draft.subject == "Re: Subject MCM2"
        draft.inReplyTo == null
    },
})

feature("a send goes: kept by the mail capture instead of connecting, and sent with its status", {
    when() {
        const capture = ec.factory.getTool("MailCapture")
        capture.enable()
        try {
            capture.clear()
            const draftId = mail("save#Draft", { emailAccountId: accountId, parentEmailMessageId: "MCM2", toAddresses: "\"Sender\" <mcm2@sender.example.com>",
                subject: "Re: the plan · v2", body: "<p>Agreed</p>" }).emailMessageId
            const sent = mail("send#Draft", { emailMessageId: draftId })
            const row = ec.entity.find("moqui.basic.email.EmailMessage").condition("emailMessageId", draftId).one()
            return { enabled: capture.isEnabled(), sent, row, captured: capture.getMessages() }
        } finally {
            capture.reset()
        }
    },
    then({ enabled, sent, row, captured }) {
        enabled == true
        sent.sent == true
        sent.statusId == "ES_SENT"
        row.statusId == "ES_SENT"
        row.sentDate != null
        captured.length == 1
        captured[0].subject == "Re: the plan · v2"
        captured[0].toList.length == 1
        captured[0].toList[0].includes("mcm2@sender.example.com")
        captured[0].body.includes("Agreed")
    },
})

feature("removing an account deletes its mail, folders, labels and server", {
    when() {
        const serverId = ec.entity.find("moqui.basic.email.EmailAccount").condition("emailAccountId", accountId).one().emailServerId
        const out = mail("delete#EmailAccount", { emailAccountId: accountId, deleteLocalCopies: true })
        return { out, serverId,
            messages: ec.entity.find("moqui.basic.email.EmailMessage").condition("emailAccountId", accountId).count(),
            folders: ec.entity.find("moqui.basic.email.EmailFolder").condition("emailAccountId", accountId).count(),
            account: ec.entity.find("moqui.basic.email.EmailAccount").condition("emailAccountId", accountId).count() }
    },
    then({ out, serverId, messages, folders, account }) {
        out.deletedMessageCount == 7
        out.keptMessageCount == 0
        messages == 0
        folders == 0
        account == 0
        ec.entity.find("moqui.basic.email.EmailServer").condition("emailServerId", serverId).count() == 0
    },
})
