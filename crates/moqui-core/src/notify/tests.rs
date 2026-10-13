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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use moqui_lib::object::{basic_convert, Value};

use super::{
    install, mark_sent, mark_viewed, origin_id, publish, subscribe, NotificationMessage, TopicMessage,
    ENTITY_CACHE_INVALIDATE, NOTIFICATION_MESSAGE,
};
use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;

const TOPIC: &str = "moqui.security.user.NotificationTopic";
const TOPIC_USER: &str = "moqui.security.user.NotificationTopicUser";
const MESSAGE: &str = "moqui.security.user.NotificationMessage";
const MESSAGE_USER: &str = "moqui.security.user.NotificationMessageUser";
const USER_ACCOUNT: &str = "moqui.security.UserAccount";
const GROUP_MEMBER: &str = "moqui.security.UserGroupMember";

fn unique(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn stamp(millis: i64) -> Value {
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}

fn open() -> (ExecutionContext, EntityFacade) {
    let ctx = ExecutionContextFactory::test_context();
    let facade = crate::testing::open();
    ctx.factory().set_entity(facade.clone());
    (ctx, facade)
}

fn open_taped() -> (ExecutionContext, EntityFacade, Arc<Mutex<Vec<String>>>) {
    let ctx = ExecutionContextFactory::test_context();
    let tape = Arc::new(Mutex::new(Vec::new()));
    let facade = crate::testing::open_with_tape(Arc::clone(&tape));
    ctx.factory().set_entity(facade.clone());
    (ctx, facade, tape)
}

fn account(facade: &EntityFacade, user_id: &str, email: Option<&str>, terminate: Option<Value>) {
    let mut row = facade.make_value(USER_ACCOUNT).unwrap();
    row.set("userId", Value::string(user_id));
    row.set("username", Value::string(user_id));
    if let Some(email) = email {
        row.set("emailAddress", Value::string(email));
    }
    if let Some(terminate) = terminate {
        row.set("terminateDate", terminate);
    }
    row.create(facade).unwrap();
}

fn member(facade: &EntityFacade, group: &str, user_id: &str, from: Value, thru: Option<Value>) {
    let mut row = facade.make_value(GROUP_MEMBER).unwrap();
    row.set("userGroupId", Value::string(group));
    row.set("userId", Value::string(user_id));
    row.set("fromDate", from);
    if let Some(thru) = thru {
        row.set("thruDate", thru);
    }
    row.create(facade).unwrap();
}

fn topic_row(facade: &EntityFacade, name: &str, fields: &[(&str, &str)]) {
    let mut row = facade.make_value(TOPIC).unwrap();
    row.set("topic", Value::string(name));
    for (field, value) in fields {
        row.set(*field, Value::string(*value));
    }
    row.create(facade).unwrap();
}

fn topic_user(facade: &EntityFacade, topic: &str, user_id: &str, fields: &[(&str, &str)]) {
    let mut row = facade.make_value(TOPIC_USER).unwrap();
    row.set("topic", Value::string(topic));
    row.set("userId", Value::string(user_id));
    for (field, value) in fields {
        row.set(*field, Value::string(*value));
    }
    row.create(facade).unwrap();
}

fn text(row: &crate::entity::EntityValue, field: &str) -> String {
    row.get(field).and_then(Value::as_str).unwrap_or("").to_string()
}

fn select_count(sqls: &[String], table: &str) -> usize {
    let quoted = format!("\"{table}\"");
    sqls.iter()
        .filter(|sql| sql.to_ascii_uppercase().starts_with("SELECT") && sql.contains(&quoted))
        .count()
}

fn kept(body_title: &str) -> (super::Subscription, Arc<Mutex<Vec<TopicMessage>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&seen);
    let title = body_title.to_string();
    let sub = subscribe(
        NOTIFICATION_MESSAGE,
        Arc::new(move |message| {
            if message.body.get("title").and_then(|value| value.as_str()) == Some(title.as_str()) {
                keep.lock().unwrap_or_else(|err| err.into_inner()).push(message.clone());
            }
        }),
    )
    .unwrap();
    (sub, seen)
}

#[test]
fn recipients_dedup_group_explicit_terminated_and_all() {
    let (ctx, facade, tape) = open_taped();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let topic = "Desk";
    topic_row(&facade, topic, &[("description", "Desk notes")]);
    for user in ["alice", "bob", "carol", "eve", "quiet"] {
        account(&facade, user, None, None);
    }
    account(&facade, "dave", None, Some(stamp(1_000)));
    let past = stamp(1_000);
    member(&facade, "OPS", "bob", past.clone(), None);
    member(&facade, "OPS", "carol", past.clone(), None);
    member(&facade, "OPS", "old", past, Some(stamp(2_000)));
    let future = jiff::Timestamp::now()
        .as_millisecond()
        .saturating_add(30 * 24 * 60 * 60 * 1_000);
    member(&facade, "OPS", "future", stamp(future), None);
    topic_user(&facade, topic, "quiet", &[("receiveNotifications", "N")]);
    topic_user(
        &facade,
        topic,
        "eve",
        &[("allNotifications", "Y"), ("receiveNotifications", "N")],
    );
    let before = tape.lock().unwrap_or_else(|err| err.into_inner()).len();
    let mut note = NotificationMessage::new();
    note.topic(topic)
        .user_group_id("OPS")
        .user_id("alice")
        .user_id("carol")
        .user_id("dave")
        .user_id("quiet")
        .user_id("ghost")
        .persist(false)
        .send()
        .unwrap();
    assert_eq!(
        note.notify_user_ids(),
        ["alice", "bob", "carol", "eve", "ghost"].as_slice()
    );
    let sqls = tape.lock().unwrap_or_else(|err| err.into_inner())[before..].to_vec();
    assert_eq!(select_count(&sqls, "NotificationTopic"), 1, "{sqls:?}");
    assert_eq!(select_count(&sqls, "NotificationTopicUser"), 1, "{sqls:?}");
    assert_eq!(select_count(&sqls, "UserGroupMember"), 1, "{sqls:?}");
    assert_eq!(select_count(&sqls, "UserAccount"), 1, "{sqls:?}");
}

#[test]
fn persist_survives_caller_rollback() {
    let (ctx, facade) = open();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    account(&facade, "alice", None, None);
    facade.begin(MESSAGE, None).unwrap();
    let mut body = IndexMap::new();
    body.insert("note".to_string(), Value::string("kept-body"));
    let mut note = NotificationMessage::new();
    note.topic("PersistTopic")
        .user_id("alice")
        .persist(true)
        .title("kept")
        .message_map(body)
        .send()
        .unwrap();
    let id = note.notification_message_id().unwrap().to_string();
    facade.rollback(MESSAGE).unwrap();
    let row = facade
        .find(MESSAGE)
        .unwrap()
        .eq("notificationMessageId", Value::string(&id))
        .one()
        .unwrap()
        .expect("persisted row");
    assert_eq!(text(&row, "titleText"), "kept");
    assert!(text(&row, "messageJson").contains("kept-body"));
    assert!(row.get("sentDate").is_some_and(|value| !value.is_null()));
    let users = facade
        .find(MESSAGE_USER)
        .unwrap()
        .eq("notificationMessageId", Value::string(&id))
        .list()
        .unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(text(&users.values()[0], "userId"), "alice");
    mark_sent("", "alice").unwrap();
    mark_sent(&id, "alice").unwrap();
    let sent = facade
        .find(MESSAGE_USER)
        .unwrap()
        .eq("notificationMessageId", Value::string(&id))
        .eq("userId", Value::string("alice"))
        .one()
        .unwrap()
        .unwrap();
    assert!(sent.get("sentDate").is_some_and(|value| !value.is_null()));
    mark_viewed(&id, "alice").unwrap();
    let viewed = facade
        .find(MESSAGE_USER)
        .unwrap()
        .eq("notificationMessageId", Value::string(&id))
        .eq("userId", Value::string("alice"))
        .one()
        .unwrap()
        .unwrap();
    assert!(viewed.get("viewedDate").is_some_and(|value| !value.is_null()));
    assert!(mark_viewed("", "alice").is_err());
    assert!(mark_viewed(&id, "").is_err());
    assert!(mark_sent(&id, "").is_err());
}

#[test]
fn subscriber_receives_one_message() {
    let (ctx, facade) = open();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let label = unique("Hello");
    let title = format!("Hello {label}");
    topic_row(
        &facade,
        "HelloTopic",
        &[
            ("titleTemplate", "ignored ${label}"),
            ("linkTemplate", "go/${label}"),
            ("errorTitleTemplate", "bad ${label}"),
        ],
    );
    let (sub, seen) = kept(&title);
    let mut message = IndexMap::new();
    message.insert("label".to_string(), Value::string(&label));
    let mut note = NotificationMessage::new();
    note.topic("HelloTopic")
        .title("Hello ${label}")
        .message_map(message)
        .persist(false)
        .send()
        .unwrap();
    assert_eq!(note.title_text(), Some(title.as_str()));
    assert_eq!(note.link_text(), Some(format!("go/{label}").as_str()));
    crate::script::set_context_name("label", Value::string("changed")).unwrap();
    assert_eq!(note.title_text(), Some(title.as_str()));
    assert_eq!(note.link_text(), Some(format!("go/{label}").as_str()));
    let got = seen.lock().unwrap_or_else(|err| err.into_inner());
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].origin_id, origin_id());
    assert!(!got[0].message_id.is_empty());
    assert!(got[0]
        .body
        .get("notificationMessageId")
        .is_some_and(|value| value.is_null()));
    drop(got);
    drop(sub);

    let bad = unique("bad");
    topic_row(
        &facade,
        "DangerTopic",
        &[
            ("titleTemplate", "ok ${label}"),
            ("errorTitleTemplate", "bad ${label}"),
            ("typeString", "danger"),
        ],
    );
    let mut message = IndexMap::new();
    message.insert("label".to_string(), Value::string(&bad));
    let mut note = NotificationMessage::new();
    note.topic("DangerTopic")
        .message_map(message)
        .persist(false)
        .send()
        .unwrap();
    assert_eq!(note.title_text(), Some(format!("bad {bad}").as_str()));
}

#[test]
fn same_message_id_delivers_once() {
    install();
    install();
    let id = unique("msg");
    let seen = Arc::new(Mutex::new(0_u32));
    let count = Arc::clone(&seen);
    let watch = id.clone();
    let _sub = subscribe(
        NOTIFICATION_MESSAGE,
        Arc::new(move |message| {
            if message.message_id == watch {
                *count.lock().unwrap_or_else(|err| err.into_inner()) += 1;
            }
        }),
    )
    .unwrap();
    let message = TopicMessage {
        origin_id: origin_id().to_string(),
        message_id: id,
        body: serde_json::Value::Null,
    };
    publish(NOTIFICATION_MESSAGE, &message).unwrap();
    publish(NOTIFICATION_MESSAGE, &message).unwrap();
    assert_eq!(*seen.lock().unwrap_or_else(|err| err.into_inner()), 1);

    let cache_id = unique("cache");
    let cache_seen = Arc::new(Mutex::new(0_u32));
    let cache_count = Arc::clone(&cache_seen);
    let watch = cache_id.clone();
    let _cache = subscribe(
        ENTITY_CACHE_INVALIDATE,
        Arc::new(move |message| {
            if message.message_id == watch {
                *cache_count.lock().unwrap_or_else(|err| err.into_inner()) += 1;
            }
        }),
    )
    .unwrap();
    let cache = TopicMessage {
        origin_id: origin_id().to_string(),
        message_id: cache_id,
        body: serde_json::Value::Null,
    };
    publish(ENTITY_CACHE_INVALIDATE, &cache).unwrap();
    publish(ENTITY_CACHE_INVALIDATE, &cache).unwrap();
    assert_eq!(*cache_seen.lock().unwrap_or_else(|err| err.into_inner()), 2);
    let err = publish(
        "no-such",
        &TopicMessage {
            origin_id: origin_id().to_string(),
            message_id: unique("nope"),
            body: serde_json::Value::Null,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("unknown topic"), "{err}");
}

#[test]
fn email_failure_does_not_fail_send() {
    let (ctx, facade) = open();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let title = unique("mail");
    topic_row(
        &facade,
        "MailTopic",
        &[("emailNotifications", "Y"), ("emailTemplateId", "NOTE")],
    );
    account(&facade, "alice", Some("alice@example.com"), None);
    ctx.message().add_error("keep-me");
    let (sub, seen) = kept(&title);
    let mut note = NotificationMessage::new();
    note.topic("MailTopic")
        .user_id("alice")
        .title(&title)
        .persist(false)
        .send()
        .unwrap();
    assert_eq!(ctx.message().errors(), ["keep-me".to_string()]);
    assert_eq!(seen.lock().unwrap_or_else(|err| err.into_inner()).len(), 1);
    drop(sub);
}

#[test]
fn job_error_publishes_danger_with_job_name() {
    let (ctx, facade) = open();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let job = unique("job");
    let error = unique("broke");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&seen);
    let watch = job.clone();
    let _sub = subscribe(
        NOTIFICATION_MESSAGE,
        Arc::new(move |message| {
            let name = message
                .body
                .get("message")
                .and_then(|value| value.get("jobName"))
                .and_then(|value| value.as_str());
            if name == Some(watch.as_str()) {
                keep.lock().unwrap_or_else(|err| err.into_inner()).push(message.clone());
            }
        }),
    )
    .unwrap();
    crate::service::send_job_error_notification(&job, "run-1", &error);
    let got = seen.lock().unwrap_or_else(|err| err.into_inner());
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].body.get("topic").and_then(|value| value.as_str()),
        Some("ServiceJobError")
    );
    assert_eq!(got[0].body.get("type").and_then(|value| value.as_str()), Some("danger"));
    assert_eq!(
        got[0]
            .body
            .get("message")
            .and_then(|value| value.get("errors"))
            .and_then(|value| value.as_str()),
        Some(error.as_str())
    );
    assert_eq!(
        got[0]
            .body
            .get("message")
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str()),
        Some(error.as_str())
    );
    let title = got[0].body.get("title").and_then(|value| value.as_str()).unwrap_or("");
    assert!(title.contains(&job), "{title}");
    drop(got);
    let rows = facade
        .find(MESSAGE)
        .unwrap()
        .eq("topic", Value::string("ServiceJobError"))
        .list()
        .unwrap();
    assert!(rows.is_empty());
}

#[test]
fn job_error_notification_failure_does_not_panic() {
    crate::service::send_job_error_notification("orphan-job", "run-9", "no facade");
}
