/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! `org.moqui.impl.ChatServices`: who reaches a room (members, a public room's readers, a private room's strangers, a
//! record room through a stub of the record visibility hook), direct rooms one per set of users, the timeline (pages,
//! edits, redactions, reactions, threads, replies, pins), unread and mention counts by notification level, the pushes
//! (to members only, after the commit), presence (status, heartbeat, auto-away, clear-at, do not disturb), the search
//! document and the search by membership, files, and the agent entry point. The SQL on the real database and the
//! search index are `runtime/base-component/example/test/Chat.spec.js`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use moqui_lib::object::{to_plain_string, Value};

use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;

const SERVICES: &str = "org.moqui.impl.ChatServices";
const MINUTE: i64 = 60_000;

/// The stubs the tests point the hooks at: the record visibility service (a record whose id starts with OPEN is
/// visible), a service that pushes and then fails (so its transaction rolls back), and an agent that answers.
const STUBS: &str = r#"<services>
    <service verb="check" noun="Visible">
        <in-parameters><parameter name="recordEntityName"/><parameter name="recordId"/><parameter name="checkOnly" type="Boolean"/></in-parameters>
        <out-parameters><parameter name="recordVisible" type="Boolean"/></out-parameters>
        <actions><script><![CDATA[ recordVisible = checkOnly === true && String(recordId).startsWith('OPEN') ]]></script></actions>
    </service>
    <service verb="push" noun="ThenFail">
        <in-parameters><parameter name="userId"/></in-parameters>
        <actions><script><![CDATA[
            ec.service.sync().name('org.moqui.impl.ChatServices.push#Signal').parameters({topic: 'chatUser:' + userId,
                userIdList: [userId], message: {kind: 'room', action: 'rolledBack'}}).call()
            ec.message.addError('failed after the push')
        ]]></script></actions>
    </service>
    <service verb="receive" noun="AgentMessage">
        <in-parameters><parameter name="chatRoomId"/><parameter name="eventSeq" type="Integer"/><parameter name="agentUserId"/>
            <parameter name="senderUserId"/><parameter name="body" allow-html="any"/><parameter name="bodyText" allow-html="any"/>
            <parameter name="threadRootEventSeq" type="Integer"/></in-parameters>
        <actions><script><![CDATA[
            ec.artifactExecution.disableAuthz()
            ec.service.sync().name('org.moqui.impl.ChatServices.send#Message').parameters({chatRoomId: chatRoomId,
                senderUserId: agentUserId, body: '<p>echo: ' + bodyText + '</p>', threadRootEventSeq: threadRootEventSeq}).call()
        ]]></script></actions>
    </service>
</services>"#;

fn setup(name: &str, pools: crate::service::ServicePools) -> (ExecutionContext, EntityFacade, PathBuf) {
    let ctx = ExecutionContextFactory::test_context_in("", pools, crate::context::framework_dir());
    let facade = crate::testing::open();
    ctx.factory().set_entity(facade.clone());
    let dir = std::env::temp_dir().join(format!("moqui-chat-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("service/test")).unwrap();
    std::fs::write(dir.join("service/test/ChatStub.xml"), STUBS).unwrap();
    (ctx, facade, dir)
}

fn install(dir: &std::path::Path) {
    let framework = crate::context::framework_dir().join("service/org/moqui");
    let paths = vec![
        framework.join("impl/ChatServices.xml"),
        framework.join("impl/ShellServices.xml"),
        dir.join("service/test/ChatStub.xml"),
    ];
    let failures = crate::service::lint(&paths).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
}

fn text(value: &str) -> Value {
    Value::string(value)
}

fn list(items: &[&str]) -> Value {
    Value::List(items.iter().map(|item| Value::string(*item)).collect())
}

fn int(value: i64) -> Value {
    Value::I64(value)
}

fn row(facade: &EntityFacade, entity: &str, fields: &[(&str, Value)]) {
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let mut row = facade.make_value(entity).unwrap();
    for (field, value) in fields {
        row.set(*field, value.clone());
    }
    row.create(facade).unwrap();
}

/// Users with these ids, each named by the id with a capital letter.
fn users(facade: &EntityFacade, ids: &[&str]) {
    for id in ids {
        let mut name: Vec<char> = id.chars().collect();
        name[0] = name[0].to_ascii_uppercase();
        row(
            facade,
            "moqui.security.UserAccount",
            &[
                ("userId", text(id)),
                ("username", text(id)),
                ("userFullName", text(&name.into_iter().collect::<String>())),
                ("emailAddress", text(&format!("{id}@example.com"))),
            ],
        );
    }
}

fn login(user_id: &str) {
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    crate::user::internal_login_user(user_id).unwrap();
}

fn call(name: &str, parameters: &[(&str, Value)]) -> BTreeMap<String, Value> {
    let parameters: BTreeMap<String, Value> = parameters
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect();
    let full = if name.contains('.') {
        name.to_string()
    } else {
        format!("{SERVICES}.{name}")
    };
    crate::service::sync(&full, &parameters).unwrap_or_else(|err| panic!("{name}: {err}"))
}

/// The errors so far, cleared.
fn errors() -> String {
    ExecutionContext::with_current(|ec| {
        let text = ec.message().errors().join("\n");
        ec.message().clear_errors();
        text
    })
    .unwrap_or_default()
}

/// `call`, which must leave no error.
fn ok(name: &str, parameters: &[(&str, Value)]) -> BTreeMap<String, Value> {
    let out = call(name, parameters);
    let errors = errors();
    assert!(errors.is_empty(), "{name}: {errors}");
    out
}

/// `call`, which must leave an error containing `expected`.
fn refused(name: &str, parameters: &[(&str, Value)], expected: &str) {
    let _ = call(name, parameters);
    let errors = errors();
    assert!(
        errors.contains(expected),
        "{name}: expected an error with {expected:?}, got {errors:?}"
    );
}

fn out_text(out: &BTreeMap<String, Value>, key: &str) -> String {
    out.get(key).map(to_plain_string).unwrap_or_default()
}

fn out_number(out: &BTreeMap<String, Value>, key: &str) -> i64 {
    out.get(key)
        .map(to_plain_string)
        .and_then(|text| text.parse().ok())
        .unwrap_or(-1)
}

fn maps(value: Option<&Value>) -> Vec<IndexMap<String, Value>> {
    match value {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Map(map) => Some(map.clone()),
                _ => None,
            })
            .collect(),
        other => panic!("not a list: {other:?}"),
    }
}

fn field(map: &IndexMap<String, Value>, key: &str) -> String {
    map.get(key).map(to_plain_string).unwrap_or_default()
}

fn number(map: &IndexMap<String, Value>, key: &str) -> i64 {
    field(map, key).parse().unwrap_or(-1)
}

fn sub(map: &IndexMap<String, Value>, key: &str) -> IndexMap<String, Value> {
    match map.get(key) {
        Some(Value::Map(inner)) => inner.clone(),
        other => panic!("{key} is not a map: {other:?}"),
    }
}

fn out_map(out: &BTreeMap<String, Value>, key: &str) -> IndexMap<String, Value> {
    match out.get(key) {
        Some(Value::Map(inner)) => inner.clone(),
        other => panic!("{key} is not a map: {other:?}"),
    }
}

/// A room of the current user: type, name, members.
fn room(kind: &str, name: &str, members: &[&str]) -> String {
    let mut parameters = vec![("roomTypeEnumId", text(kind)), ("memberUserIdList", list(members))];
    if !name.is_empty() {
        parameters.push(("roomName", text(name)));
    }
    out_text(&ok("create#Room", &parameters), "chatRoomId")
}

fn send(room: &str, body: &str) -> i64 {
    out_number(
        &ok("send#Message", &[("chatRoomId", text(room)), ("body", text(body))]),
        "eventSeq",
    )
}

fn timeline(room: &str, more: &[(&str, Value)]) -> BTreeMap<String, Value> {
    let mut parameters = vec![("chatRoomId", text(room))];
    parameters.extend(more.iter().cloned());
    ok("get#Timeline", &parameters)
}

fn events(out: &BTreeMap<String, Value>) -> Vec<IndexMap<String, Value>> {
    maps(out.get("eventList"))
}

fn seqs(out: &BTreeMap<String, Value>) -> Vec<i64> {
    events(out).iter().map(|event| number(event, "eventSeq")).collect()
}

fn texts(out: &BTreeMap<String, Value>) -> Vec<String> {
    events(out)
        .iter()
        .filter(|event| field(event, "type") == "message")
        .map(|event| field(event, "bodyText"))
        .collect()
}

fn counts(user_id: &str) -> BTreeMap<String, Value> {
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    ok("get#UnreadCounts", &[("userId", text(user_id))])
}

fn room_ids(out: &BTreeMap<String, Value>) -> Vec<String> {
    maps(out.get("roomList"))
        .iter()
        .map(|room| field(room, "chatRoomId"))
        .collect()
}

fn set_time(ctx: &ExecutionContext, millis: i64) {
    ctx.set_effective_time(Some(millis));
}

fn now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// A push: its topic, the users it was routed to, and its message.
type Pushed = (String, Vec<String>, serde_json::Value);

/// The pushes of the notification bus: (topic, routed user ids, message) of each NotificationMessage, kept while the
/// subscription lives. Tests run at once in one process, so each test keeps only the pushes to its own users.
struct Tape {
    _subscription: crate::notify::Subscription,
    seen: Arc<Mutex<Vec<Pushed>>>,
}

impl Tape {
    fn start(users: &[&str]) -> Self {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&seen);
        let users: Vec<String> = users.iter().map(|id| (*id).to_string()).collect();
        let subscription = crate::notify::subscribe(
            crate::notify::NOTIFICATION_MESSAGE,
            Arc::new(move |message: &crate::notify::TopicMessage| {
                let topic = message.body["topic"].as_str().unwrap_or_default().to_string();
                let routed: Vec<String> = message.body[crate::notify::ROUTE_USER_IDS]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if routed.iter().any(|id| users.contains(id)) {
                    kept.lock()
                        .unwrap()
                        .push((topic, routed, message.body["message"].clone()));
                }
            }),
        )
        .unwrap();
        Self {
            _subscription: subscription,
            seen,
        }
    }

    /// The pushes on topics starting with `prefix`, taken off the tape.
    fn take(&self, prefix: &str) -> Vec<Pushed> {
        let mut seen = self.seen.lock().unwrap();
        let (taken, kept): (Vec<_>, Vec<_>) = seen.drain(..).partition(|(topic, _, _)| topic.starts_with(prefix));
        *seen = kept;
        taken
    }

    fn clear(&self) {
        self.seen.lock().unwrap().clear();
    }
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// Who reaches a room: members, readers of a public room who may join it, strangers to a private room, and a record
/// room through the record visibility hook (a stub that sees the records whose id starts with OPEN).
#[test]
fn room_access_for_members_public_readers_strangers_and_record_rooms() {
    let (ctx, facade, dir) = setup("access", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["acowner", "acmember", "acstranger"]);
    login("acowner");
    let private = room("private", "Ops", &["acmember"]);
    let public = room("public", "General", &[]);
    refused("create#Room", &[("roomTypeEnumId", text("private"))], "needs a name");
    refused(
        "create#Room",
        &[
            ("roomTypeEnumId", text("private")),
            ("roomName", text("X")),
            ("memberUserIdList", list(&["nobody"])),
        ],
        "User nobody not found",
    );
    let first = send(&private, "<p>hello team</p>");
    let access = ok("get#RoomAccess", &[("chatRoomId", text(&private))]);
    assert_eq!(out_number(&access, "level"), 4);
    assert_eq!(access.get("canModerate"), Some(&Value::Bool(true)));

    // a member posts; a member who is not a moderator gives no roles and deletes no one else's message
    login("acmember");
    let access = ok("get#RoomAccess", &[("chatRoomId", text(&private))]);
    assert_eq!(out_number(&access, "level"), 2);
    send(&private, "<p>hi</p>");
    refused(
        "set#MemberRole",
        &[
            ("chatRoomId", text(&private)),
            ("userId", text("acmember")),
            ("roleEnumId", text("moderator")),
        ],
        "Only a moderator or owner",
    );
    refused(
        "redact#Message",
        &[("chatRoomId", text(&private)), ("eventSeq", int(first))],
        "only your own",
    );

    // a stranger: the private room is not there; the public one is read only until joined
    login("acstranger");
    let none = ok("get#RoomAccess", &[("chatRoomId", text(&private))]);
    assert_eq!(out_number(&none, "level"), 0);
    assert_eq!(none.get("room"), Some(&Value::Null));
    refused("get#Timeline", &[("chatRoomId", text(&private))], "not found");
    refused(
        "send#Message",
        &[("chatRoomId", text(&private)), ("body", text("<p>x</p>"))],
        "not found",
    );
    refused("signal#Typing", &[("chatRoomId", text(&private))], "not found");
    refused("add#Member", &[("chatRoomId", text(&private))], "not found");
    refused(
        "get#RoomAccess",
        &[("chatRoomId", text(&private)), ("userId", text("acowner"))],
        "not yours to see",
    );
    assert!(room_ids(&ok("find#Rooms", &[])).is_empty());
    let joinable = ok("find#Rooms", &[("includeJoinable", Value::Bool(true))]);
    assert_eq!(room_ids(&joinable), std::slice::from_ref(&public));
    assert_eq!(field(&maps(joinable.get("roomList"))[0], "isMember"), "false");
    let read = timeline(&public, &[]);
    assert_eq!(out_map(&read, "access").get("canJoin"), Some(&Value::Bool(true)));
    refused(
        "send#Message",
        &[("chatRoomId", text(&public)), ("body", text("<p>x</p>"))],
        "Join the room",
    );
    ok("add#Member", &[("chatRoomId", text(&public))]);
    send(&public, "<p>joined</p>");
    assert_eq!(room_ids(&ok("find#Rooms", &[])), std::slice::from_ref(&public));

    // the owner makes the member a moderator, who then deletes another's message
    login("acowner");
    ok(
        "set#MemberRole",
        &[
            ("chatRoomId", text(&private)),
            ("userId", text("acmember")),
            ("roleEnumId", text("moderator")),
        ],
    );
    refused(
        "set#MemberRole",
        &[
            ("chatRoomId", text(&private)),
            ("userId", text("acowner")),
            ("roleEnumId", text("member")),
        ],
        "at least one owner",
    );
    login("acmember");
    ok(
        "redact#Message",
        &[("chatRoomId", text(&private)), ("eventSeq", int(first))],
    );

    // record rooms: none is visible before the theme names the hook
    login("acowner");
    let hook = ok(
        "check#RecordVisible",
        &[("recordEntityName", text("test.Thing")), ("recordId", text("OPEN1"))],
    );
    assert_eq!(hook.get("recordVisible"), Some(&Value::Bool(false)));
    assert_eq!(hook.get("hookService"), Some(&Value::Null));
    refused(
        "create#Room",
        &[
            ("roomTypeEnumId", text("record")),
            ("recordEntityName", text("test.Thing")),
            ("recordId", text("OPEN1")),
        ],
        "not found",
    );
    row(
        &facade,
        "moqui.screen.ScreenThemeResource",
        &[
            ("screenThemeId", text("DEFAULT_QUASAR")),
            ("sequenceNum", int(990)),
            ("resourceTypeEnumId", text("STRT_RECORD_VISIBLE_SERVICE")),
            ("resourceValue", text("test.ChatStub.check#Visible")),
        ],
    );
    let hook = ok(
        "check#RecordVisible",
        &[("recordEntityName", text("test.Thing")), ("recordId", text("OPEN1"))],
    );
    assert_eq!(hook.get("recordVisible"), Some(&Value::Bool(true)));
    assert_eq!(out_text(&hook, "hookService"), "test.ChatStub.check#Visible");
    let record = out_text(
        &ok(
            "create#Room",
            &[
                ("roomTypeEnumId", text("record")),
                ("recordEntityName", text("test.Thing")),
                ("recordId", text("OPEN1")),
            ],
        ),
        "chatRoomId",
    );
    refused(
        "create#Room",
        &[
            ("roomTypeEnumId", text("record")),
            ("recordEntityName", text("test.Thing")),
            ("recordId", text("SECRET1")),
        ],
        "not found",
    );
    // a record room only for its members
    let members_only = out_text(
        &ok(
            "create#Room",
            &[
                ("roomTypeEnumId", text("record")),
                ("recordEntityName", text("test.Thing")),
                ("recordId", text("OPEN2")),
                ("visibilityEnumId", text("members")),
            ],
        ),
        "chatRoomId",
    );
    send(&record, "<p>about the thing</p>");

    // a user who may see the record reads the room, and joins it by asking for the record's room
    login("acstranger");
    let access = ok("get#RoomAccess", &[("chatRoomId", text(&record))]);
    assert_eq!(out_number(&access, "level"), 1);
    assert_eq!(access.get("canJoin"), Some(&Value::Bool(true)));
    let bound = timeline(
        "",
        &[("recordEntityName", text("test.Thing")), ("recordId", text("OPEN1"))],
    );
    assert_eq!(texts(&bound), ["about the thing"]);
    let none = ok(
        "get#Timeline",
        &[("recordEntityName", text("test.Thing")), ("recordId", text("NONE"))],
    );
    assert_eq!(none.get("room"), Some(&Value::Null));
    let again = ok(
        "create#Room",
        &[
            ("roomTypeEnumId", text("record")),
            ("recordEntityName", text("test.Thing")),
            ("recordId", text("OPEN1")),
        ],
    );
    assert_eq!(out_text(&again, "chatRoomId"), record);
    assert_eq!(again.get("created"), Some(&Value::Bool(false)));
    assert_eq!(
        out_number(&ok("get#RoomAccess", &[("chatRoomId", text(&record))]), "level"),
        2
    );
    let closed = ok("get#RoomAccess", &[("chatRoomId", text(&members_only))]);
    assert_eq!(out_number(&closed, "level"), 0);
    // a record's Chatter asks by the record: no room, and no error
    let unread_room = ok(
        "get#Timeline",
        &[("recordEntityName", text("test.Thing")), ("recordId", text("OPEN2"))],
    );
    assert_eq!(unread_room.get("room"), Some(&Value::Null));
    refused("get#Timeline", &[("chatRoomId", text(&members_only))], "not found");
    refused(
        "create#Room",
        &[
            ("roomTypeEnumId", text("record")),
            ("recordEntityName", text("test.Thing")),
            ("recordId", text("OPEN2")),
        ],
        "for its members",
    );

    // archived: read only
    login("acowner");
    ok(
        "update#Room",
        &[("chatRoomId", text(&public)), ("archived", Value::Bool(true))],
    );
    refused(
        "send#Message",
        &[("chatRoomId", text(&public)), ("body", text("<p>x</p>"))],
        "archived",
    );
    login("acmember");
    refused(
        "update#Room",
        &[("chatRoomId", text(&private)), ("archived", Value::Bool(true))],
        "Only an owner",
    );
    ok(
        "update#Room",
        &[("chatRoomId", text(&private)), ("topic", text("Weekly ops"))],
    );
    let changed = timeline(&private, &[]);
    let last = events(&changed).pop().unwrap();
    assert_eq!(field(&last, "type"), "system");
    assert_eq!(field(&sub(&last, "content"), "action"), "topic");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A direct room is one per set of users, whoever starts it; leaving hides it until its next message.
#[test]
fn direct_rooms_are_reused_and_come_back_with_a_new_message() {
    let (ctx, facade, dir) = setup("direct", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["dra", "drb", "drc"]);
    login("dra");
    let created = ok(
        "create#Room",
        &[("roomTypeEnumId", text("direct")), ("memberUserIdList", list(&["drb"]))],
    );
    let pair = out_text(&created, "chatRoomId");
    assert_eq!(created.get("created"), Some(&Value::Bool(true)));
    let again = ok(
        "create#Room",
        &[
            ("roomTypeEnumId", text("direct")),
            ("memberUserIdList", list(&["drb", "dra"])),
        ],
    );
    assert_eq!(out_text(&again, "chatRoomId"), pair);
    assert_eq!(again.get("created"), Some(&Value::Bool(false)));
    let group = room("direct", "", &["drb", "drc"]);
    assert_ne!(group, pair);
    refused(
        "add#Member",
        &[("chatRoomId", text(&pair)), ("userId", text("drc"))],
        "fixed members",
    );
    refused(
        "update#Room",
        &[("chatRoomId", text(&pair)), ("topic", text("x"))],
        "no settings",
    );
    login("drb");
    let theirs = ok(
        "create#Room",
        &[("roomTypeEnumId", text("direct")), ("memberUserIdList", list(&["dra"]))],
    );
    assert_eq!(out_text(&theirs, "chatRoomId"), pair);

    // the list names a direct room by the other members, with the other's presence
    login("dra");
    let mine = ok("find#Rooms", &[("filter", text("direct"))]);
    let rooms = maps(mine.get("roomList"));
    assert_eq!(rooms.len(), 2);
    let listed = rooms.iter().find(|room| field(room, "chatRoomId") == pair).unwrap();
    assert_eq!(field(listed, "displayName"), "Drb");
    assert_eq!(field(listed, "otherUserId"), "drb");
    assert_eq!(field(listed, "presence"), "offline");
    let grouped = rooms.iter().find(|room| field(room, "chatRoomId") == group).unwrap();
    assert_eq!(field(grouped, "displayName"), "Drb, Drc");
    assert_eq!(field(grouped, "otherUserId"), "");

    // leaving hides it; a new message brings it back, unread
    ok("remove#Member", &[("chatRoomId", text(&pair))]);
    assert!(!room_ids(&ok("find#Rooms", &[])).contains(&pair));
    login("drb");
    send(&pair, "<p>still there?</p>");
    login("dra");
    let back = ok("find#Rooms", &[("filter", text("unread"))]);
    assert_eq!(room_ids(&back), std::slice::from_ref(&pair));
    let listed = &maps(back.get("roomList"))[0];
    assert_eq!(number(listed, "unreadCount"), 1);
    assert_eq!(field(&sub(listed, "lastMessage"), "snippet"), "still there?");
    assert_eq!(field(&sub(listed, "lastMessage"), "senderName"), "Drb");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The timeline: pages by eventSeq both ways and around one, edits applied, a redaction as "message removed",
/// reactions aggregated and toggled, threads apart from the main timeline with their counts, replies with a preview,
/// pins.
#[test]
fn the_timeline_pages_and_applies_edits_redactions_reactions_threads_and_pins() {
    let (ctx, facade, dir) = setup("timeline", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["tla", "tlb"]);
    login("tla");
    let chat = room("private", "Timeline", &["tlb"]);
    let seq: Vec<i64> = (1..=7).map(|n| send(&chat, &format!("<p>m{n}</p>"))).collect();
    // event 1 is the room's creation
    assert_eq!(seq, [2, 3, 4, 5, 6, 7, 8]);

    let latest = timeline(&chat, &[("limit", int(3))]);
    assert_eq!(texts(&latest), ["m5", "m6", "m7"]);
    assert_eq!(latest.get("hasMoreBefore"), Some(&Value::Bool(true)));
    assert_eq!(latest.get("hasMoreAfter"), Some(&Value::Bool(false)));
    let before = timeline(&chat, &[("limit", int(3)), ("beforeSeq", int(6))]);
    assert_eq!(texts(&before), ["m2", "m3", "m4"]);
    assert_eq!(before.get("hasMoreAfter"), Some(&Value::Bool(true)));
    let oldest = timeline(&chat, &[("limit", int(3)), ("beforeSeq", int(3))]);
    assert_eq!(seqs(&oldest), [1, 2]);
    assert_eq!(field(&events(&oldest)[0], "type"), "system");
    assert_eq!(oldest.get("hasMoreBefore"), Some(&Value::Bool(false)));
    let after = timeline(&chat, &[("limit", int(2)), ("afterSeq", int(5))]);
    assert_eq!(texts(&after), ["m5", "m6"]);
    assert_eq!(after.get("hasMoreAfter"), Some(&Value::Bool(true)));
    assert_eq!(after.get("hasMoreBefore"), Some(&Value::Bool(true)));
    let around = timeline(&chat, &[("limit", int(4)), ("aroundSeq", int(5))]);
    assert_eq!(texts(&around), ["m2", "m3", "m4", "m5"]);

    // an edit of one's own message, in its place
    ok(
        "edit#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("eventSeq", int(8)),
            ("body", text("<p>m7, edited</p>")),
        ],
    );
    let edited = timeline(&chat, &[("limit", int(1))]);
    let last = &events(&edited)[0];
    assert_eq!(number(last, "eventSeq"), 8);
    assert_eq!(field(last, "body"), "<p>m7, edited</p>");
    assert_eq!(field(last, "edited"), "true");
    login("tlb");
    refused(
        "edit#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("eventSeq", int(8)),
            ("body", text("<p>mine now</p>")),
        ],
        "only your own",
    );

    // reactions: two people, two keys, and one taken back
    for (user, key) in [("tla", "👍"), ("tlb", "👍"), ("tlb", "🎉")] {
        login(user);
        let out = ok(
            "react#Message",
            &[("chatRoomId", text(&chat)), ("eventSeq", int(7)), ("key", text(key))],
        );
        assert_eq!(out.get("added"), Some(&Value::Bool(true)));
    }
    let back = ok(
        "react#Message",
        &[("chatRoomId", text(&chat)), ("eventSeq", int(7)), ("key", text("👍"))],
    );
    assert_eq!(back.get("added"), Some(&Value::Bool(false)));
    login("tla");
    let reacted = timeline(&chat, &[("limit", int(2))]);
    let message = &events(&reacted)[0];
    assert_eq!(field(message, "bodyText"), "m6");
    let reactions = maps(message.get("reactions"));
    assert_eq!(reactions.len(), 2);
    assert_eq!(field(&reactions[0], "key"), "👍");
    assert_eq!(number(&reactions[0], "count"), 1);
    assert_eq!(field(&reactions[0], "mine"), "true");
    assert_eq!(field(&reactions[1], "key"), "🎉");
    assert_eq!(field(&reactions[1], "mine"), "false");

    // a thread: replies stay out of the main timeline, the root counts them
    login("tlb");
    let replies: Vec<i64> = ["<p>r1</p>", "<p>r2</p>"]
        .iter()
        .map(|body| {
            out_number(
                &ok(
                    "send#Message",
                    &[
                        ("chatRoomId", text(&chat)),
                        ("body", text(body)),
                        ("threadRootEventSeq", int(6)),
                    ],
                ),
                "eventSeq",
            )
        })
        .collect();
    let main = timeline(&chat, &[("limit", int(50))]);
    assert!(!texts(&main).contains(&"r1".to_string()));
    let root = events(&main)
        .into_iter()
        .find(|event| number(event, "eventSeq") == 6)
        .unwrap();
    assert_eq!(number(&root, "threadReplyCount"), 2);
    let thread = timeline(&chat, &[("threadRootEventSeq", int(6))]);
    assert_eq!(texts(&thread), ["m5", "r1", "r2"]);
    // a reply to a reply goes to the root's thread
    let deeper = ok(
        "send#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("body", text("<p>r3</p>")),
            ("threadRootEventSeq", int(replies[1])),
        ],
    );
    assert_eq!(field(&out_map(&deeper, "event"), "threadRootEventSeq"), "6");

    // a reply that quotes a message
    let quoted = ok(
        "send#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("body", text("<p>about m1</p>")),
            ("replyToEventSeq", int(2)),
        ],
    );
    let page = timeline(&chat, &[("limit", int(1))]);
    let reply = &events(&page)[0];
    assert_eq!(number(reply, "eventSeq"), out_number(&quoted, "eventSeq"));
    let preview = sub(reply, "replyTo");
    assert_eq!(field(&preview, "snippet"), "m1");
    assert_eq!(field(&preview, "senderName"), "Tla");

    // a redaction: "message removed", no body, no reactions; only the sender (or a moderator)
    refused(
        "redact#Message",
        &[("chatRoomId", text(&chat)), ("eventSeq", int(7))],
        "only your own",
    );
    login("tla");
    ok("redact#Message", &[("chatRoomId", text(&chat)), ("eventSeq", int(7))]);
    let removed = events(&timeline(&chat, &[("limit", int(50))]))
        .into_iter()
        .find(|event| number(event, "eventSeq") == 7)
        .unwrap();
    assert_eq!(field(&removed, "redacted"), "true");
    assert_eq!(removed.get("body"), Some(&Value::Null));
    assert_eq!(removed.get("bodyText"), Some(&Value::Null));
    assert_eq!(field(&removed, "removedLabel"), "Message removed");
    assert!(maps(removed.get("reactions")).is_empty());
    refused(
        "react#Message",
        &[("chatRoomId", text(&chat)), ("eventSeq", int(7)), ("key", text("👍"))],
        "not found",
    );

    // pins
    ok("pin#Message", &[("chatRoomId", text(&chat)), ("eventSeq", int(3))]);
    ok("pin#Message", &[("chatRoomId", text(&chat)), ("eventSeq", int(4))]);
    ok(
        "pin#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("eventSeq", int(4)),
            ("pinned", Value::Bool(false)),
        ],
    );
    assert_eq!(texts(&timeline(&chat, &[("pinnedOnly", Value::Bool(true))])), ["m2"]);
    let pin_events = events(&timeline(&chat, &[("limit", int(3))]));
    assert_eq!(field(pin_events.last().unwrap(), "type"), "pin");

    // the edit window: a day by default
    set_time(&ctx, now() + 2 * 24 * 60 * MINUTE);
    refused(
        "edit#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("eventSeq", int(2)),
            ("body", text("<p>late</p>")),
        ],
        "too old",
    );
    ctx.set_effective_time(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A membership starts at the member's own default level (CHAT_NOTIFY_DEFAULT) in create#Room and add#Member, but in a direct room;
/// a mention's notification leaves out the message text for a user whose CHAT_NOTIFY_PREVIEW is N.
#[test]
fn a_member_starts_at_their_default_level_and_a_mention_may_leave_out_the_text() {
    let (ctx, facade, dir) = setup("defaults", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["dla", "dlb", "dlc", "dld"]);
    for (user, key, value) in [
        ("dlb", "CHAT_NOTIFY_DEFAULT", "none"),
        ("dlc", "CHAT_NOTIFY_DEFAULT", "mentions"),
        ("dld", "CHAT_NOTIFY_DEFAULT", "bogus"),
        ("dlc", "CHAT_NOTIFY_PREVIEW", "N"),
    ] {
        row(
            &facade,
            "moqui.security.UserPreference",
            &[
                ("userId", text(user)),
                ("preferenceKey", text(key)),
                ("preferenceValue", text(value)),
            ],
        );
    }
    row(
        &facade,
        "moqui.security.user.NotificationTopic",
        &[
            ("topic", text("ChatMention")),
            ("persistOnSend", text("Y")),
            (
                "titleTemplate",
                text("${authorName} mentioned you in ${roomName}: ${summary}"),
            ),
        ],
    );
    let level = |room: &str, user: &str| {
        let _authz = ctx.disable_authz();
        facade
            .find("moqui.chat.ChatRoomMember")
            .unwrap()
            .eq("chatRoomId", text(room))
            .eq("userId", text(user))
            .one()
            .unwrap()
            .and_then(|row| row.get("notificationLevelEnumId").map(to_plain_string))
            .unwrap_or_default()
    };
    login("dla");
    let team = room("private", "Defaults", &["dlb", "dlc"]);
    assert_eq!(level(&team, "dla"), "CnlAll", "no preference: the room's");
    assert_eq!(level(&team, "dlb"), "CnlNone");
    assert_eq!(level(&team, "dlc"), "CnlMentions");
    ok("add#Member", &[("chatRoomId", text(&team)), ("userId", text("dld"))]);
    assert_eq!(level(&team, "dld"), "CnlAll", "a value that is no level: the room's");
    let open = room("public", "Lobby", &[]);
    login("dlb");
    ok("add#Member", &[("chatRoomId", text(&open))]);
    assert_eq!(level(&open, "dlb"), "CnlNone", "joining a public room");
    login("dla");
    let direct = room("direct", "", &["dlb"]);
    assert_eq!(level(&direct, "dlb"), "CnlAll", "a direct room's messages always count");

    // the mention of dlc (no preview) and of dld (preview): the text is in dld's notification only
    send(
        &team,
        r#"<p>secret plan <span data-mention-id="dlc">@Dlc</span> <span data-mention-id="dld">@Dld</span></p>"#,
    );
    let _authz = ctx.disable_authz();
    let title_of = |user: &str| {
        let users = facade
            .find("moqui.security.user.NotificationMessageUser")
            .unwrap()
            .eq("userId", text(user))
            .list()
            .unwrap();
        assert_eq!(users.len(), 1, "{user}");
        let id = to_plain_string(users.get(0).unwrap().get("notificationMessageId").unwrap());
        let message = facade
            .find("moqui.security.user.NotificationMessage")
            .unwrap()
            .eq("notificationMessageId", text(&id))
            .one()
            .unwrap()
            .unwrap();
        (
            to_plain_string(message.get("titleText").unwrap()),
            message.get("messageJson").map(to_plain_string).unwrap_or_default(),
        )
    };
    let (shown_title, shown_json) = title_of("dld");
    assert_eq!(shown_title, "Dla mentioned you in Defaults: secret plan @Dlc @Dld");
    assert!(shown_json.contains("secret plan"), "{shown_json}");
    let (hidden_title, hidden_json) = title_of("dlc");
    assert_eq!(hidden_title, "Dla mentioned you in Defaults");
    assert!(!hidden_json.contains("secret"), "{hidden_json}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unread and mention counts: messages from others after the read marker, mentions among them, by notification level;
/// a mention is notified on ChatMention unless the level is none; a redaction takes its mention back.
#[test]
fn unread_and_mention_counts_follow_the_read_marker_and_the_notification_level() {
    let (ctx, facade, dir) = setup("unread", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["una", "unb", "unc", "und"]);
    row(
        &facade,
        "moqui.security.user.NotificationTopic",
        &[
            ("topic", text("ChatMention")),
            ("persistOnSend", text("Y")),
            ("showAlert", text("Y")),
            (
                "titleTemplate",
                text("${authorName} mentioned you in ${roomName}: ${summary}"),
            ),
        ],
    );
    login("una");
    let chat = room("private", "Counts", &["unb", "unc"]);
    send(&chat, "<p>one</p>");
    let mention = send(
        &chat,
        r#"<p>two <span data-mention-id="unb">@Unb</span> and <span data-mention-id="und">@Und</span></p>"#,
    );
    send(&chat, r#"<p>three <span data-mention-id="unc">@Unc</span></p>"#);
    assert_eq!(out_number(&counts("una"), "total"), 0, "one's own messages are read");
    let b = counts("unb");
    assert_eq!(out_number(&b, "total"), 3);
    assert_eq!(out_number(&b, "unread"), 3);
    assert_eq!(out_number(&b, "mentions"), 1);
    assert_eq!(out_number(&b, "unreadRoomCount"), 1);
    assert_eq!(number(&sub(&out_map(&b, "roomCounts"), &chat), "mentions"), 1);
    // und is not a member of the private room: not mentioned there
    assert_eq!(out_number(&counts("und"), "mentions"), 0);
    let notified = |user: &str| {
        let _authz = ctx.disable_authz();
        facade
            .find("moqui.security.user.NotificationMessageUser")
            .unwrap()
            .eq("userId", text(user))
            .list()
            .unwrap()
            .len()
    };
    assert_eq!(notified("unb"), 1);
    assert_eq!(notified("und"), 0);
    {
        let _authz = ctx.disable_authz();
        let messages = facade
            .find("moqui.security.user.NotificationMessage")
            .unwrap()
            .eq("topic", text("ChatMention"))
            .list()
            .unwrap();
        let titles: Vec<String> = messages
            .iter()
            .map(|row| to_plain_string(row.get("titleText").unwrap()))
            .collect();
        assert!(
            titles.contains(&"Una mentioned you in Counts: two @Unb and @Und".to_string()),
            "{titles:?}"
        );
        assert!(messages
            .iter()
            .all(|row| to_plain_string(row.get("reasonEnumId").unwrap()) == "NrMention"));
    }

    // the read marker
    login("unb");
    let read = ok("mark#Read", &[("chatRoomId", text(&chat)), ("eventSeq", int(mention))]);
    assert_eq!(out_number(&read, "lastReadEventSeq"), mention);
    assert_eq!(out_number(&counts("unb"), "total"), 1);
    assert_eq!(out_number(&counts("unb"), "mentions"), 0);
    // a marker does not go back, except to mark unread from a message
    ok("mark#Read", &[("chatRoomId", text(&chat)), ("eventSeq", int(2))]);
    assert_eq!(out_number(&counts("unb"), "total"), 1);
    ok(
        "mark#Read",
        &[
            ("chatRoomId", text(&chat)),
            ("eventSeq", int(mention)),
            ("markUnread", Value::Bool(true)),
        ],
    );
    assert_eq!(out_number(&counts("unb"), "total"), 2);
    assert_eq!(
        room_ids(&ok("find#Rooms", &[("filter", text("mentions"))])),
        std::slice::from_ref(&chat)
    );
    ok("mark#AllRead", &[]);
    assert_eq!(out_number(&counts("unb"), "total"), 0);
    assert!(room_ids(&ok("find#Rooms", &[("filter", text("unread"))])).is_empty());

    // notification levels: mentions only, nothing, muted
    login("unc");
    assert_eq!(out_number(&counts("unc"), "total"), 3);
    ok(
        "set#Notification",
        &[
            ("chatRoomId", text(&chat)),
            ("notificationLevelEnumId", text("mentions")),
        ],
    );
    assert_eq!(out_number(&counts("unc"), "total"), 1);
    assert_eq!(out_number(&counts("unc"), "unread"), 0);
    ok(
        "set#Notification",
        &[("chatRoomId", text(&chat)), ("notificationLevelEnumId", text("none"))],
    );
    let none = counts("unc");
    assert_eq!(out_number(&none, "total"), 0);
    assert_eq!(out_number(&none, "mentions"), 0);
    assert_eq!(number(&sub(&out_map(&none, "roomCounts"), &chat), "unread"), 3);
    ok(
        "set#Notification",
        &[
            ("chatRoomId", text(&chat)),
            ("notificationLevelEnumId", text("all")),
            ("isMuted", Value::Bool(true)),
        ],
    );
    assert_eq!(out_number(&counts("unc"), "total"), 1);
    ok(
        "set#Notification",
        &[
            ("chatRoomId", text(&chat)),
            ("notificationLevelEnumId", text("none")),
            ("isMuted", Value::Bool(false)),
        ],
    );
    // at level none a mention is counted in the room but not notified
    login("una");
    send(&chat, r#"<p>four <span data-mention-id="unc">@Unc</span></p>"#);
    assert_eq!(notified("unc"), 1, "the first mention only");

    // a redaction takes the message and its mention out of the counts
    login("una");
    let before = counts("unb");
    ok(
        "redact#Message",
        &[("chatRoomId", text(&chat)), ("eventSeq", int(mention))],
    );
    login("unb");
    ok(
        "mark#Read",
        &[
            ("chatRoomId", text(&chat)),
            ("eventSeq", int(mention)),
            ("markUnread", Value::Bool(true)),
        ],
    );
    let after = counts("unb");
    assert_eq!(out_number(&after, "mentions"), 0);
    assert!(out_number(&after, "unread") >= out_number(&before, "unread"));
    assert_eq!(out_number(&after, "unread"), 2, "three and four, not the removed one");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pushes: each event goes on chatRoom:<id> to the room's members only, not persisted; typing and read markers go to
/// the other members; room list changes on chatUser:<id> to that user; presence to the user's room members; a push in
/// a transaction that rolls back is never sent.
#[test]
fn pushes_go_only_to_the_room_members_and_only_after_the_commit() {
    let (ctx, facade, dir) = setup("pushes", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["psa", "psb", "psc"]);
    let tape = Tape::start(&["psa", "psb", "psc"]);
    login("psa");
    let chat = room("private", "Pushes", &["psb"]);
    let joined = tape.take("chatUser:");
    assert_eq!(
        sorted(joined.iter().map(|(topic, _, _)| topic.clone()).collect()),
        ["chatUser:psa", "chatUser:psb"]
    );
    assert!(joined.iter().all(|(topic, routed, message)| routed.len() == 1
        && topic.ends_with(&routed[0])
        && message["kind"] == "room"
        && message["action"] == "joined"));
    tape.clear();

    let seq = send(&chat, "<p>for members</p>");
    let pushed = tape.take("chatRoom:");
    assert_eq!(pushed.len(), 1, "{pushed:?}");
    let (topic, routed, message) = &pushed[0];
    assert_eq!(topic, &format!("chatRoom:{chat}"));
    assert_eq!(sorted(routed.clone()), ["psa", "psb"]);
    assert_eq!(message["kind"], "event");
    assert_eq!(message["eventSeq"], seq);
    assert_eq!(message["event"]["body"], "<p>for members</p>");
    assert_eq!(message["event"]["senderName"], "Psa");
    {
        let _authz = ctx.disable_authz();
        let stored = facade
            .find("moqui.security.user.NotificationMessage")
            .unwrap()
            .condition(crate::entity::Condition::like("topic", text("chatRoom:%")))
            .list()
            .unwrap();
        assert!(stored.is_empty(), "chat pushes are not persisted");
    }

    // typing: the others only; read: my tabs, and the others' read receipts
    tape.clear();
    ok("signal#Typing", &[("chatRoomId", text(&chat))]);
    let typing = tape.take("chatRoom:");
    assert_eq!(typing.len(), 1);
    assert_eq!(typing[0].1, ["psb"]);
    assert_eq!(typing[0].2["kind"], "typing");
    assert_eq!(typing[0].2["userId"], "psa");
    login("psb");
    tape.clear();
    ok("mark#Read", &[("chatRoomId", text(&chat))]);
    let mine = tape.take("chatUser:");
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].1, ["psb"]);
    assert_eq!(mine[0].2["kind"], "read");
    assert_eq!(mine[0].2["lastReadEventSeq"], seq);
    let receipts = tape.take("chatRoom:");
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].1, ["psa"]);
    assert_eq!(receipts[0].2["userId"], "psb");

    // a stranger gets none of it
    assert!(tape
        .seen
        .lock()
        .unwrap()
        .iter()
        .all(|(_, routed, _)| !routed.contains(&"psc".to_string())));

    // presence: to the members of the user's rooms and the user's own tabs
    tape.clear();
    login("psa");
    ok("heartbeat#UserStatus", &[("idleSeconds", int(0))]);
    let status = tape.take("userStatus");
    assert_eq!(status.len(), 2, "{status:?}");
    let to_others = status.iter().find(|(_, routed, _)| routed == &["psb"]).unwrap();
    assert_eq!(to_others.2["presence"], "online");
    assert_eq!(to_others.2.get("self"), None);
    let to_self = status.iter().find(|(_, routed, _)| routed == &["psa"]).unwrap();
    assert_eq!(to_self.2["self"], true);
    // the same presence is not published again
    ok("heartbeat#UserStatus", &[("idleSeconds", int(5))]);
    assert!(tape.take("userStatus").is_empty());

    // a rolled back transaction pushes nothing
    tape.clear();
    let _ = call("test.ChatStub.push#ThenFail", &[("userId", text("psc"))]);
    assert!(errors().contains("failed after the push"));
    assert!(tape.take("chatUser:").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Presence: none without a heartbeat, online with one, away after the idle minutes (the user's preference), offline
/// three minutes after the last beat; a set status with its text and clear-at time; invisible is offline to others;
/// do not disturb still records the notifications.
#[test]
fn presence_follows_the_heartbeat_the_status_and_its_clear_at_time() {
    let (ctx, facade, dir) = setup("presence", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["pra", "prb"]);
    row(
        &facade,
        "moqui.security.user.NotificationTopic",
        &[
            ("topic", text("ChatMention")),
            ("persistOnSend", text("Y")),
            ("showAlert", text("Y")),
        ],
    );
    let tape = Tape::start(&["pra", "prb"]);
    let presence_of = |user: &str| {
        let out = ok("get#UserStatuses", &[("userIdList", list(&[user]))]);
        sub(&out_map(&out, "statusMap"), user)
    };
    let start = now();
    set_time(&ctx, start);
    login("prb");
    room("direct", "", &["pra"]);
    assert_eq!(field(&presence_of("pra"), "presence"), "offline");

    login("pra");
    let mine = ok("heartbeat#UserStatus", &[("idleSeconds", int(0))]);
    assert_eq!(field(&out_map(&mine, "myStatus"), "presence"), "online");
    assert_eq!(field(&out_map(&mine, "myStatus"), "status"), "online");
    assert_eq!(number(&out_map(&mine, "myStatus"), "autoAwayMinutes"), 10);
    login("prb");
    assert_eq!(field(&presence_of("pra"), "presence"), "online");

    // idle past the auto-away minutes: away; a longer preference keeps the user online
    set_time(&ctx, start + 11 * MINUTE);
    login("pra");
    ok("heartbeat#UserStatus", &[("idleSeconds", int(11 * 60))]);
    assert_eq!(field(&presence_of("pra"), "presence"), "away");
    let pushed = tape.take("userStatus");
    assert!(
        pushed
            .iter()
            .any(|(_, routed, message)| routed == &["prb"] && message["presence"] == "away"),
        "{pushed:?}"
    );
    row(
        &facade,
        "moqui.security.UserPreference",
        &[
            ("userId", text("pra")),
            ("preferenceKey", text("CHAT_AUTO_AWAY_MINUTES")),
            ("preferenceValue", text("30")),
        ],
    );
    assert_eq!(field(&presence_of("pra"), "presence"), "online");
    // back from idle at once
    ok("heartbeat#UserStatus", &[("active", Value::Bool(true))]);
    assert_eq!(field(&presence_of("pra"), "presence"), "online");

    // no heartbeat: offline after three minutes, published by the frequent job
    tape.clear();
    set_time(&ctx, start + 15 * MINUTE);
    assert_eq!(field(&presence_of("pra"), "presence"), "offline");
    {
        let _authz = ctx.disable_authz();
        let job = ok("expire#UserStatuses", &[]);
        assert!(out_number(&job, "publishedCount") >= 1);
    }
    let pushed = tape.take("userStatus");
    assert!(
        pushed
            .iter()
            .any(|(_, routed, message)| routed == &["prb"] && message["presence"] == "offline"),
        "{pushed:?}"
    );

    // do not disturb with a text that clears in 30 minutes
    let set = ok(
        "set#UserStatus",
        &[
            ("status", text("dnd")),
            ("statusText", text("Focus time")),
            ("emoji", text("🎧")),
            ("clearAfterMinutes", int(30)),
        ],
    );
    let status = out_map(&set, "myStatus");
    assert_eq!(field(&status, "presence"), "dnd");
    assert_eq!(field(&status, "status"), "dnd");
    assert_eq!(field(&status, "statusText"), "Focus time");
    assert_eq!(number(&status, "clearAtDate"), start + 45 * MINUTE);
    let shown = presence_of("pra");
    assert_eq!(field(&shown, "presence"), "dnd");
    assert_eq!(field(&shown, "emoji"), "🎧");
    {
        let _authz = ctx.disable_authz();
        assert_eq!(
            out_number(&ok("expire#UserStatuses", &[]), "clearedCount"),
            0,
            "not yet"
        );
    }
    // do not disturb: the mention is still recorded
    login("prb");
    let pair = room("direct", "", &["pra"]);
    send(&pair, r#"<p>ping <span data-mention-id="pra">@Pra</span></p>"#);
    {
        let _authz = ctx.disable_authz();
        let rows = facade
            .find("moqui.security.user.NotificationMessageUser")
            .unwrap()
            .eq("userId", text("pra"))
            .list()
            .unwrap();
        assert_eq!(rows.len(), 1);
    }
    assert_eq!(out_number(&counts("pra"), "mentions"), 1);

    // the clear-at time: the status reads as online at once, and the job writes it
    set_time(&ctx, start + 46 * MINUTE);
    login("pra");
    ok("heartbeat#UserStatus", &[("idleSeconds", int(0))]);
    let cleared = presence_of("pra");
    assert_eq!(field(&cleared, "presence"), "online");
    assert_eq!(cleared.get("statusText"), Some(&Value::Null));
    {
        let _authz = ctx.disable_authz();
        let job = ok("expire#UserStatuses", &[]);
        assert_eq!(out_number(&job, "clearedCount"), 1);
        let stored = facade
            .find("moqui.security.UserStatus")
            .unwrap()
            .eq("userId", text("pra"))
            .one()
            .unwrap()
            .unwrap();
        assert_eq!(stored.get("statusText"), Some(&Value::Null));
        assert_eq!(to_plain_string(stored.get("statusEnumId").unwrap()), "UpsOnline");
    }

    // invisible: offline to others, invisible to oneself
    let set = ok("set#UserStatus", &[("status", text("invisible"))]);
    assert_eq!(field(&out_map(&set, "myStatus"), "status"), "invisible");
    assert_eq!(field(&out_map(&set, "myStatus"), "presence"), "offline");
    login("prb");
    assert_eq!(field(&presence_of("pra"), "presence"), "offline");
    refused(
        "set#UserStatus",
        &[("userId", text("pra")), ("status", text("online"))],
        "only your own",
    );
    refused("set#UserStatus", &[("status", text("busy"))], "Unknown status");
    ctx.set_effective_time(None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The MoquiChatEvent document: a message with its room and sender, no other events; and the search by membership
/// (without a search backend, the text of the messages of the rooms the user may read).
#[test]
fn the_search_document_and_the_search_by_membership() {
    let (ctx, facade, dir) = setup("search", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    {
        let _authz = ctx.disable_authz();
        let data = std::fs::read_to_string(crate::context::framework_dir().join("data/ChatSetupData.xml")).unwrap();
        crate::entity::load_feed::load_entity_xml_text(&facade, "ChatSetupData.xml", &data, &Default::default())
            .unwrap();
    }
    users(&facade, &["sea", "seb", "sec", "sed"]);
    login("sea");
    let team = room("private", "Team", &["seb"]);
    let quarterly = send(&team, "<p>the quarterly <strong>forecast</strong> is due</p>");
    login("sec");
    let lobby = room("public", "Lobby", &[]);
    send(&lobby, "<p>forecast party on Friday</p>");
    let own = room("private", "Notes", &[]);
    send(&own, "<p>my forecast notes</p>");

    // the document of a message; the room's creation is not one
    let documents = {
        let _authz = ctx.disable_authz();
        crate::entity::document::data_documents(
            &facade,
            "MoquiChatEvent",
            Some(crate::entity::Condition::equals("chatRoomId", text(&team))),
            None,
            None,
        )
        .unwrap()
    };
    assert_eq!(documents.len(), 1, "{documents:?}");
    let document = &documents[0];
    assert_eq!(field(document, "_id"), format!("{team}::{quarterly}"));
    assert_eq!(field(document, "_type"), "MoquiChatEvent");
    assert_eq!(field(document, "bodyText"), "the quarterly forecast is due");
    assert_eq!(field(document, "senderName"), "Sea");
    assert_eq!(field(document, "roomName"), "Team");
    assert_eq!(field(document, "roomVisibilityEnumId"), "CrvMembers");

    let found = |user: &str, more: &[(&str, Value)]| {
        login(user);
        let mut parameters = vec![("searchText", text("forecast"))];
        parameters.extend(more.iter().cloned());
        let out = ok("search#ChatEvents", &parameters);
        let mut hits: Vec<String> = maps(out.get("eventList"))
            .iter()
            .map(|hit| format!("{}:{}", field(hit, "roomName"), field(hit, "snippet")))
            .collect();
        hits.sort();
        hits
    };
    assert_eq!(found("sed", &[]), ["Lobby:forecast party on Friday"]);
    assert_eq!(
        found("sea", &[]),
        ["Lobby:forecast party on Friday", "Team:the quarterly forecast is due"]
    );
    assert_eq!(
        found("sec", &[]),
        ["Lobby:forecast party on Friday", "Notes:my forecast notes"]
    );
    assert_eq!(
        found("seb", &[("chatRoomId", text(&team))]),
        ["Team:the quarterly forecast is due"]
    );
    login("sed");
    refused(
        "search#ChatEvents",
        &[("searchText", text("forecast")), ("chatRoomId", text(&team))],
        "not found",
    );
    // the current text: an edit is found, a redaction is not
    login("sea");
    ok(
        "edit#Message",
        &[
            ("chatRoomId", text(&team)),
            ("eventSeq", int(quarterly)),
            ("body", text("<p>the quarterly budget</p>")),
        ],
    );
    assert_eq!(found("seb", &[("chatRoomId", text(&team))]), Vec::<String>::new());
    login("sea");
    let budget = ok("search#ChatEvents", &[("searchText", text("budget"))]);
    assert_eq!(out_number(&budget, "total"), 1);
    login("sec");
    ok("redact#Message", &[("chatRoomId", text(&own)), ("eventSeq", int(2))]);
    assert_eq!(found("sec", &[]), ["Lobby:forecast party on Friday"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Files: uploaded to a room, sent with a message, read by those who may read the room, removed with the message; a
/// file never sent goes after a day. The in-memory database has no DbResource store, so the files go under a file:
/// root here (`chat_attachment_root`); `Chat.spec.js` stores them as DbResource files.
#[test]
fn files_go_with_their_message_and_go_away_with_it() {
    let (ctx, facade, dir) = setup("files", crate::service::ServicePools::idle());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["fla", "flb", "flc"]);
    std::fs::create_dir_all(dir.join("chat")).unwrap();
    let root = format!("file:{}", dir.join("chat").display());
    moqui_lib::env::set_property("chat_attachment_root", &root);
    let upload = |name: &str, bytes: &[u8]| {
        crate::web::multipart::UploadedFile {
            field_name: "file".to_string(),
            file_name: name.to_string(),
            content_type: "text/plain".to_string(),
            bytes: bytes.to_vec(),
        }
        .into_value()
    };
    login("fla");
    let chat = room("private", "Files", &["flb"]);
    let stored = ok(
        "upload#Attachment",
        &[
            ("chatRoomId", text(&chat)),
            ("file", upload("../notes/plan.txt", b"the plan")),
        ],
    );
    let attachment = out_text(&stored, "chatAttachmentId");
    let location = out_text(&stored, "location");
    assert_eq!(location, format!("{root}/{chat}/{attachment}/plan.txt"));
    assert_eq!(out_number(&stored, "fileSize"), 8);
    let bytes = ctx.factory().resource().get(&location).unwrap().read_bytes().unwrap();
    assert_eq!(bytes, b"the plan");

    // another member may not send it; the uploader sends it alone, with no text
    login("flb");
    refused(
        "send#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("chatAttachmentIdList", list(&[&attachment])),
        ],
        "not found",
    );
    refused(
        "get#Attachment",
        &[("chatAttachmentId", text(&attachment))],
        "not found",
    );
    login("fla");
    refused("send#Message", &[("chatRoomId", text(&chat))], "needs text or a file");
    let sent = ok(
        "send#Message",
        &[
            ("chatRoomId", text(&chat)),
            ("chatAttachmentIdList", list(&[&attachment])),
        ],
    );
    let seq = out_number(&sent, "eventSeq");
    let listed = events(&timeline(&chat, &[("limit", int(1))]));
    let files = maps(listed[0].get("attachments"));
    assert_eq!(files.len(), 1);
    assert_eq!(field(&files[0], "fileName"), "plan.txt");
    let rooms = ok("find#Rooms", &[]);
    assert_eq!(
        field(&sub(&maps(rooms.get("roomList"))[0], "lastMessage"), "snippet"),
        "📎 plan.txt"
    );
    login("flb");
    let info = ok("get#Attachment", &[("chatAttachmentId", text(&attachment))]);
    assert_eq!(out_text(&info, "location"), location);
    login("flc");
    refused(
        "get#Attachment",
        &[("chatAttachmentId", text(&attachment))],
        "not found",
    );
    refused(
        "upload#Attachment",
        &[("chatRoomId", text(&chat)), ("file", upload("x.txt", b"x"))],
        "not found",
    );

    // the message deleted: the file goes too
    login("fla");
    ok("redact#Message", &[("chatRoomId", text(&chat)), ("eventSeq", int(seq))]);
    assert!(
        !ctx.factory().resource().get(&location).unwrap().exists(),
        "the file is gone"
    );
    {
        let _authz = ctx.disable_authz();
        assert!(facade
            .find("moqui.chat.ChatAttachment")
            .unwrap()
            .eq("chatAttachmentId", text(&attachment))
            .one()
            .unwrap()
            .is_none());
    }

    // a file never sent goes after a day
    let pending = out_text(
        &ok(
            "upload#Attachment",
            &[("chatRoomId", text(&chat)), ("file", upload("draft.txt", b"draft"))],
        ),
        "chatAttachmentId",
    );
    set_time(&ctx, now() + 25 * 60 * MINUTE);
    {
        let _authz = ctx.disable_authz();
        let cleaned = ok("clean#PendingAttachments", &[]);
        assert_eq!(out_number(&cleaned, "removedCount"), 1);
        assert!(facade
            .find("moqui.chat.ChatAttachment")
            .unwrap()
            .eq("chatAttachmentId", text(&pending))
            .one()
            .unwrap()
            .is_none());
    }
    ctx.set_effective_time(None);
    moqui_lib::env::clear_property("chat_attachment_root");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The agent entry point: an agent member's service gets each message from another member, with the message in its
/// parameters, and answers in the room as the agent.
#[test]
fn an_agent_member_gets_each_message_through_its_entry_point() {
    let (ctx, facade, dir) = setup("agent", crate::service::ServicePools::inline());
    let _entered = ctx.enter();
    install(&dir);
    users(&facade, &["aga", "agm", "agbot"]);
    login("aga");
    let chat = room("private", "Assist", &["agm"]);
    // a member who is not a moderator adds no agent
    login("agm");
    refused(
        "add#Member",
        &[
            ("chatRoomId", text(&chat)),
            ("userId", text("agbot")),
            ("roleEnumId", text("agent")),
        ],
        "Only a moderator or owner",
    );
    // the owner adds the agent, then names its service
    login("aga");
    ok(
        "add#Member",
        &[
            ("chatRoomId", text(&chat)),
            ("userId", text("agbot")),
            ("roleEnumId", text("agent")),
        ],
    );
    ok(
        "set#MemberRole",
        &[
            ("chatRoomId", text(&chat)),
            ("userId", text("agbot")),
            ("roleEnumId", text("agent")),
            ("agentRef", text("test.ChatStub.receive#AgentMessage")),
        ],
    );
    send(&chat, "<p>what is due today?</p>");
    let ran = ctx.factory().service_pools().run_pending();
    assert!(ran >= 1, "the agent's delivery was queued");
    let page = timeline(&chat, &[("limit", int(2))]);
    let shown = events(&page);
    assert_eq!(field(&shown[0], "bodyText"), "what is due today?");
    assert_eq!(field(&shown[1], "bodyText"), "echo: what is due today?");
    assert_eq!(field(&shown[1], "senderUserId"), "agbot");
    // the agent's own answer is not handed back to it
    assert_eq!(ctx.factory().service_pools().run_pending(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
