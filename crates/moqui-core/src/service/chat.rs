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

//! The two Rust parts of `org.moqui.impl.ChatServices` (no Java counterpart; the rest are scripts in
//! `framework/service/org/moqui/impl/ChatServices.xml`):
//!
//! - `push#Signal` sends a chat event or signal to users as a NotificationMessage that is not persisted and shows no
//!   alert, on a chat topic (`chatRoom:<id>`, `chatUser:<userId>` or `userStatus`), after the caller's transaction
//!   commits (a rollback sends nothing). The notification socket delivers it only to those users.
//! - `check#RecordVisible` asks the service the user's qapps theme names in `STRT_RECORD_VISIBLE_SERVICE` whether the
//!   current user may see a record, so a user who is not a member may read and join the room bound to it. The framework
//!   cannot read an app's records; IronStone BOSS points the theme value at Mantle's
//!   `mantle.party.CommunicationServices.get#RecordLinkInfo`, which takes `checkOnly` and answers `recordVisible`.
//!   `FileServices.check#RecordPaths` asks it too, for the record folders of a records drive. With `userId` (honored
//!   only with authorization disabled, as `get#RoomAccess` has it) it asks for another user: the hook of that user's
//!   theme runs as that user, with authorization on, in an execution context of its own on this thread (so in the
//!   caller's transaction), and the caller's user is untouched. `send#Message` asks so for a mentioned user who is not a
//!   member of a record room.
//!
//!   The hook is called in the caller's transaction (a record made earlier in it is seen), so with `checkOnly` it must
//!   answer without adding an error: an error in a service call marks the transaction rollback-only even when its
//!   errors are taken back (`call_tolerant`), and the caller's work would then roll back at its commit.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

use indexmap::IndexMap;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, to_plain_string, Value};
use moqui_lib::sync::lock;

use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;
use crate::screen::sri::{theme_resource_values, user_theme_id, DEFAULT_QUASAR_THEME, QUASAR_THEME_TYPE};

pub(in crate::service) fn register(
    map: &mut BTreeMap<&'static str, crate::service::rust_services::RustServiceFn>,
) -> Result<(), String> {
    rust_service!(map, push_signal)?;
    rust_service!(map, check_record_visible)?;
    Ok(())
}

/// The topics a chat push may go out on: a room's events and signals, a user's room list, and presence.
const ROOM_TOPIC: &str = "chatRoom:";
const USER_TOPIC: &str = "chatUser:";
const STATUS_TOPIC: &str = "userStatus";
/// The theme value that names the record visibility service.
const RECORD_VISIBLE: &str = "STRT_RECORD_VISIBLE_SERVICE";

fn text(parameters: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    parameters
        .get(key)
        .filter(|value| !is_empty(value))
        .map(to_plain_string)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn add_error(message: &str) {
    ExecutionContext::with_current(|ec| ec.message().add_error(message));
}

/// Whether `topic` is one a chat push may use: `chatRoom:<id>`, `chatUser:<userId>` or `userStatus`.
pub(crate) fn chat_topic(topic: &str) -> bool {
    let named = |prefix: &str| {
        topic
            .strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace))
    };
    topic == STATUS_TOPIC || named(ROOM_TOPIC) || named(USER_TOPIC)
}

/// `push#Signal`: send `message` to the users of `userIdList` on `topic`, not persisted and with no alert. In a
/// transaction the send waits for the commit, so a rolled back event is never pushed; it returns at once.
fn push_signal(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some(topic) = text(parameters, "topic").filter(|topic| chat_topic(topic)) else {
        add_error("A chat push goes on a chatRoom:, chatUser: or userStatus topic");
        return Ok(BTreeMap::new());
    };
    let users = super::shell::user_ids(parameters.get("userIdList"));
    if users.is_empty() {
        return Ok(BTreeMap::new());
    }
    let message = match parameters.get("message") {
        Some(Value::Map(map)) => map.clone(),
        _ => IndexMap::new(),
    };
    let factory = ExecutionContext::with_current(|ec| Arc::downgrade(ec.factory()));
    let send = move || send_push(factory.as_ref(), &topic, &users, message);
    match facade.default_transactions() {
        Ok(tx) if tx.in_transaction() => {
            // the closure is kept until the commit; a failure to register sends now
            let pending = Arc::new(Mutex::new(Some(send)));
            let held = Arc::clone(&pending);
            let registered = tx.on_commit(move || {
                if let Some(send) = lock(&held).take() {
                    send();
                }
            });
            if let Err(err) = registered {
                tracing::error!("Could not wait for the commit to push a chat signal, pushing now: {err}");
                if let Some(send) = lock(&pending).take() {
                    send();
                }
            }
        }
        _ => send(),
    }
    Ok(BTreeMap::new())
}

/// Send one push. Outside an execution context (a commit callback should have one) a context of the factory is made.
/// A failure is logged: a push is a hint to the client, which reads the timeline again when it reconnects.
fn send_push(
    factory: Option<&Weak<ExecutionContextFactory>>,
    topic: &str,
    users: &[String],
    message: IndexMap<String, Value>,
) {
    let send = |message: IndexMap<String, Value>| {
        crate::notify::NotificationMessage::new()
            .topic(topic)
            .user_ids(users.iter().cloned())
            .message_map(message)
            .persist(false)
            .show_alert(false)
            .send()
    };
    let outcome = if ExecutionContext::with_current(|_| ()).is_some() {
        send(message)
    } else if let Some(factory) = factory.and_then(Weak::upgrade) {
        let ctx = factory.new_context();
        let _entered = ctx.enter();
        let _authz = ctx.disable_authz();
        send(message)
    } else {
        return;
    };
    if let Err(err) = outcome {
        tracing::error!("Could not push the chat signal on {topic}: {err}");
    }
}

/// The record visibility services that failed or are missing, each logged once while the server runs.
#[derive(Default)]
struct Logged(Mutex<HashSet<String>>);

fn first_time(key: &str) -> bool {
    ExecutionContext::with_current(|ec| {
        let logged = ec.factory().extension(|_| Logged::default());
        let first = lock(&logged.0).insert(key.to_string());
        first
    })
    .unwrap_or(true)
}

/// `check#RecordVisible`: whether the current user may see the record (`recordEntityName`, `recordId`), as the
/// service the user's qapps theme names in `STRT_RECORD_VISIBLE_SERVICE` answers it, called as the user (the caller's
/// authorization and entity filters apply) with `checkOnly` true. Out: `recordVisible`, and `hookService`, the service
/// asked (null when the theme names none: then no record is visible this way, which is logged once). A service that
/// fails or answers no `recordVisible` gives false and is logged once by name. It never adds an error, so it does not
/// mark the caller's transaction for rollback.
///
/// `userId`, another user, is asked for only with authorization disabled (else false, logged): the hook of that user's
/// theme runs as that user with authorization on ([`as_user`]); a user who is not found sees nothing.
fn check_record_visible(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let answer = |visible: bool, service: Option<&str>| {
        BTreeMap::from([
            ("recordVisible".to_string(), Value::Bool(visible)),
            ("hookService".to_string(), service.map_or(Value::Null, Value::string)),
        ])
    };
    let (Some(entity_name), Some(record_id)) = (text(parameters, "recordEntityName"), text(parameters, "recordId"))
    else {
        return Ok(answer(false, None));
    };
    let current = crate::user::current_user_id()
        .ok()
        .flatten()
        .filter(|id| !id.is_empty());
    let other = text(parameters, "userId").filter(|id| current.as_deref() != Some(id.as_str()));
    if let Some(other) = &other {
        if !ExecutionContext::with_current(ExecutionContext::authz_disabled).unwrap_or(false) {
            tracing::warn!("check#RecordVisible for user {other} refused: another user is asked for only with authorization disabled");
            return Ok(answer(false, None));
        }
    }
    let user_id = other.clone().or(current);
    let theme = user_theme_id(user_id.as_deref(), QUASAR_THEME_TYPE, DEFAULT_QUASAR_THEME);
    let service = match theme_resource_values(&theme, RECORD_VISIBLE) {
        Ok(values) => values.into_iter().next(),
        Err(err) => {
            tracing::error!("Could not read {RECORD_VISIBLE} of theme {theme}: {err}");
            None
        }
    };
    let Some(service) = service else {
        if first_time(&format!("none:{theme}")) {
            tracing::info!(
                "Theme {theme} names no {RECORD_VISIBLE}: a chat room bound to a record is reached only by its members, \
                 and a records drive's record folders only by its managers"
            );
        }
        return Ok(answer(false, None));
    };
    let call = BTreeMap::from([
        ("recordEntityName".to_string(), Value::string(&entity_name)),
        ("recordId".to_string(), Value::string(&record_id)),
        ("checkOnly".to_string(), Value::Bool(true)),
    ]);
    let asked = match &other {
        Some(other) => match as_user(other, || super::shell::call_tolerant(&service, &call)) {
            Some(asked) => asked,
            None => return Ok(answer(false, Some(&service))),
        },
        None => super::shell::call_tolerant(&service, &call),
    };
    let visible = match asked {
        Ok(out) => match out.get("recordVisible") {
            Some(Value::Bool(visible)) => *visible,
            Some(other) if !is_empty(other) => matches!(to_plain_string(other).as_str(), "true" | "Y"),
            _ => {
                if first_time(&service) {
                    tracing::error!("The record visibility service {service} answered no recordVisible");
                }
                false
            }
        },
        Err(err) => {
            if first_time(&service) {
                tracing::error!("The record visibility service {service} failed: {err}");
            }
            false
        }
    };
    Ok(answer(visible, Some(&service)))
}

/// Run `work` as the user `user_id`, with authorization on: in a context of its own, entered on this thread, so it
/// shares the caller's transaction but not the caller's user, authorization or messages. None when there is no
/// context or the user is not found.
fn as_user<T>(user_id: &str, work: impl FnOnce() -> T) -> Option<T> {
    let factory = ExecutionContext::with_current(|ec| Arc::clone(ec.factory()))?;
    let facade = factory.entity()?;
    let account = {
        let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
        facade
            .find("moqui.security.UserAccount")
            .and_then(|find| find.eq("userId", Value::string(user_id)).one())
    };
    let username = match account {
        Ok(Some(account)) => account
            .get("username")
            .map(to_plain_string)
            .filter(|name| !name.is_empty()),
        Ok(None) => None,
        Err(err) => {
            tracing::error!("Could not read the account of user {user_id}: {err}");
            None
        }
    }?;
    let ctx = factory.new_context();
    let _entered = ctx.enter();
    crate::user::session::set_user(user_id, &username);
    Some(work())
}

#[cfg(test)]
mod tests {
    use super::chat_topic;

    #[test]
    fn a_chat_push_takes_only_the_chat_topics() {
        for topic in ["chatRoom:100", "chatUser:john.doe", "userStatus"] {
            assert!(chat_topic(topic), "{topic}");
        }
        for topic in [
            "chatRoom:",
            "chatUser:",
            "shellCounts",
            "ChatMention",
            "chatRoom:a b",
            "notifications",
            "",
        ] {
            assert!(!chat_topic(topic), "{topic}");
        }
    }
}
