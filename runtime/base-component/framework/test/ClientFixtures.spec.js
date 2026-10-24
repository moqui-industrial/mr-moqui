// The users the framework's client specs (test/client, moqui test-client) log in as: sec.view.only (VIEW on Tools and
// System), sec.none.only (no artifact authz) and the load user test.load (VIEW on System, MOQUI_API and the
// Enumeration entity, no tarpit of its own), with Moqui Java's passwords, made as SecurityTestSupport.ensureUsers made
// them in a Spock setupSpec (plans/ClientTestHarness.md D10, 7). moqui test-client framework --serve runs this spec on
// its server's scratch database; for another server run it on that server's database:
//     moqui test framework --spec ClientFixtures --db existing
// Each row is made only when it is missing, so a database kept between runs takes it again. These users are not demo
// data, so no production load can reach them.

const USERS = [
    { username: "sec.view.only", password: "SecView1!!", group: "SEC_VIEW_GROUP" },
    { username: "sec.none.only", password: "SecNone1!!", group: "SEC_NONE_GROUP" },
    { username: "test.load", password: "TestLoad1!!", group: "TEST_LOAD_GROUP" },
]

const one = (entity, fields) => {
    let find = ec.entity.find(entity)
    for (const [name, value] of Object.entries(fields)) find = find.condition(name, value)
    return find.one()
}
const ensure = (entity, key, fields) => {
    if (one(entity, key) != null) return
    const row = ec.entity.makeValue(entity)
    row.setFields(Object.assign({}, key, fields || {}), true, null, null)
    row.create()
}
const ensureGroup = (userGroupId, description) => ensure("moqui.security.UserGroup", { userGroupId }, { description })
const ensureAuthz = (artifactAuthzId, userGroupId, artifactGroupId, authzActionEnumId) =>
    ensure("moqui.security.ArtifactAuthz", { artifactAuthzId },
        { userGroupId, artifactGroupId, authzTypeEnumId: "AUTHZT_ALWAYS", authzActionEnumId })
const ensureUser = ({ username, password, group }) => {
    const existing = one("moqui.security.UserAccount", { username })
    const userId = existing != null ? existing.userId : call("org.moqui.impl.UserServices.create#UserAccount",
        { username, newPassword: password, newPasswordVerify: password, userFullName: username }).userId
    if (one("moqui.security.UserGroupMember", { userId, userGroupId: group }) == null) {
        call("create#moqui.security.UserGroupMember", { userId, userGroupId: group, fromDate: ec.user.nowTimestamp })
    }
    return userId
}

spec({
    setupSpec() {
        ec.artifactExecution.disableAuthz()
        try {
            ensureGroup("SEC_VIEW_GROUP", "Security test VIEW-only")
            ensureAuthz("SEC_VIEW_TOOLS", "SEC_VIEW_GROUP", "TOOLS_APP", "AUTHZA_VIEW")
            ensureAuthz("SEC_VIEW_SYS", "SEC_VIEW_GROUP", "SYSTEM_APP", "AUTHZA_VIEW")
            ensureGroup("SEC_NONE_GROUP", "Security test no artifact authz")

            ensureGroup("TEST_LOAD_GROUP", "Load test user")
            ensureAuthz("TEST_LOAD_SYS", "TEST_LOAD_GROUP", "SYSTEM_APP", "AUTHZA_VIEW")
            // /rest/s1/moqui/... (runtime/base-component/tools/data/ToolsSecurityData.xml)
            ensureAuthz("TEST_LOAD_API", "TEST_LOAD_GROUP", "MOQUI_API", "AUTHZA_VIEW")
            // /rest/e1/moqui.basic.Enumeration
            ensure("moqui.security.ArtifactGroup", { artifactGroupId: "TEST_LOAD_ENUM" }, { description: "Load test Enumeration entity" })
            ensure("moqui.security.ArtifactGroupMember",
                { artifactGroupId: "TEST_LOAD_ENUM", artifactName: "moqui.basic.Enumeration", artifactTypeEnumId: "AT_ENTITY" },
                { nameIsPattern: "N" })
            ensureAuthz("TEST_LOAD_ENUM_AUTHZ", "TEST_LOAD_GROUP", "TEST_LOAD_ENUM", "AUTHZA_VIEW")

            for (const user of USERS) ensureUser(user)
        } finally {
            ec.artifactExecution.enableAuthz()
        }
    },
})

feature("#username is in #group and logs in with its password", {
    where: USERS,
    when({ username, password, group }) {
        ec.artifactExecution.disableAuthz()
        let account, member
        try {
            account = one("moqui.security.UserAccount", { username })
            member = account == null ? null : one("moqui.security.UserGroupMember", { userId: account.userId, userGroupId: group })
        } finally {
            ec.artifactExecution.enableAuthz()
        }
        const loggedIn = ec.user.loginUser(username, password)
        const userId = ec.user.userId
        if (loggedIn) logout()
        return { account, member, loggedIn, userId }
    },
    then({ account, member, loggedIn, userId }) {
        account != null
        account.disabled != "Y"
        member != null
        loggedIn
        userId == account.userId
    },
})

feature("the access each user is given", {
    when() {
        ec.artifactExecution.disableAuthz()
        try {
            const authz = (artifactAuthzId) => one("moqui.security.ArtifactAuthz", { artifactAuthzId })
            const tarpits = (userGroupId) => ec.entity.find("moqui.security.ArtifactTarpit").condition("userGroupId", userGroupId).count()
            const authzOf = (userGroupId) => ec.entity.find("moqui.security.ArtifactAuthz").condition("userGroupId", userGroupId).count()
            return {
                viewTools: authz("SEC_VIEW_TOOLS"), viewSys: authz("SEC_VIEW_SYS"), noneAuthz: authzOf("SEC_NONE_GROUP"),
                loadSys: authz("TEST_LOAD_SYS"), loadApi: authz("TEST_LOAD_API"), loadEnum: authz("TEST_LOAD_ENUM_AUTHZ"),
                loadTarpits: tarpits("TEST_LOAD_GROUP"),
            }
        } finally {
            ec.artifactExecution.enableAuthz()
        }
    },
    then({ viewTools, viewSys, noneAuthz, loadSys, loadApi, loadEnum, loadTarpits }) {
        viewTools.artifactGroupId == "TOOLS_APP"
        viewTools.authzActionEnumId == "AUTHZA_VIEW"
        viewSys.artifactGroupId == "SYSTEM_APP"
        viewSys.authzActionEnumId == "AUTHZA_VIEW"
        noneAuthz == 0
        loadSys.artifactGroupId == "SYSTEM_APP"
        loadApi.artifactGroupId == "MOQUI_API"
        loadApi.authzActionEnumId == "AUTHZA_VIEW"
        loadEnum.artifactGroupId == "TEST_LOAD_ENUM"
        loadTarpits == 0
    },
})
