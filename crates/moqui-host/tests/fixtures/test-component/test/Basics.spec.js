// The harness on a real database: pinned ids and clock, a component service, entity-auto, data checks, where rows,
// threads and a REST render.
const effectiveTime = 1265198400000 // 2010-02-03 12:00 UTC
let firstEnumId

spec({
    tempSequences: { start: 55500, banks: { "moqui.basic.Enumeration": 10 } },
    setupSpec() {
        ec.user.setEffectiveTime(effectiveTime)
        ec.artifactExecution.disableAuthz()
        call("create#moqui.basic.EnumerationType", { enumTypeId: "FixtureType", description: "Fixture Type" })
        call("create#moqui.security.UserAccount", { userId: "FixtureUser", username: "fixture", userFullName: "Fixture User" })
        ec.artifactExecution.enableAuthz()
    },
    setup() { ec.artifactExecution.disableAuthz() },
    cleanup() { ec.artifactExecution.enableAuthz() },
})

feature("a component service with a pinned id and clock", {
    when() {
        const out = call("fixture.FixtureServices.create#StampedEnum", { enumTypeId: "FixtureType", description: "First" })
        firstEnumId = out.enumId
        const dataCheckErrors = checkData(`<entity-facade-xml>
            <moqui.basic.Enumeration enumId="55500" enumTypeId="FixtureType" description="First" enumCode="${effectiveTime}"/>
        </entity-facade-xml>`)
        return { out, dataCheckErrors }
    },
    then({ out, dataCheckErrors }) {
        out.enumId == "55500"
        out.stamp == effectiveTime
        dataCheckErrors.length == 0
    },
})

feature("entity-auto fromDate is the effective time", {
    when() {
        call("create#moqui.security.UserGroupMember", { userGroupId: "ADMIN", userId: "FixtureUser" })
        return {
            member: ec.entity.find("moqui.security.UserGroupMember").condition("userId", "FixtureUser").one(),
            errs: checkData(`<entity-facade-xml>
                <moqui.security.UserGroupMember userGroupId="ADMIN" userId="FixtureUser" fromDate="${effectiveTime}"/>
            </entity-facade-xml>`),
        }
    },
    then({ member, errs }) {
        member.fromDate == effectiveTime
        errs.length == 0
    },
})

feature("enum #enumId has description #description", {
    where: [
        { enumId: "55500", description: "First" },
        { enumId: "55501", description: "Second" },
    ],
    given({ enumId, description }) {
        if (enumId !== firstEnumId) call("fixture.FixtureServices.create#StampedEnum", { enumTypeId: "FixtureType", description })
    },
    expect({ enumId, description }) {
        ec.entity.find("moqui.basic.Enumeration").condition("enumId", enumId).one().description == description
    },
})

feature("threads each make an enum in their own transaction", {
    when() {
        const ids = parallel(3, (n, args) => {
            ec.artifactExecution.disableAuthz()
            ec.user.setEffectiveTime(args.effectiveTime + n)
            return ec.service.sync().name("fixture.FixtureServices.create#StampedEnum")
                .parameters({ enumTypeId: "FixtureType", description: "Thread " + n }).call().enumId
        }, { effectiveTime })
        return { ids, count: ec.entity.find("moqui.basic.Enumeration").condition("enumTypeId", "FixtureType").count() }
    },
    then({ ids, count }) {
        ids.length == 3
        new Set(ids).size == 3
        count == 5
    },
})

feature("a REST render under a base screen path", {
    given() { ec.user.internalLoginUser("fixture") },
    expect() {
        const str = screenTest("rest").render("e1/moqui.basic.Enumeration/55500", null, "get")
        !str.errorMessages
        str.assertContains('"description"')
        str.assertContains("First")
    },
    cleanup() { logout() },
})
