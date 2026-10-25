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

//! Reload reports as notifications (plans/design-dev-aide.md 6.4): each report of the framework (a save, or the file
//! watcher seeing an edit from outside) goes to the dev-aide's users as a `DevReload` notification, which the open
//! Workspace pages take to refresh their tree, their status and the tabs of the files. The listener is installed by the
//! first dev-aide service a factory runs (the Workspace asks for its roots when it opens), so the component needs no tool
//! factory: a binary without the dev tools still boots with the component's screens.

use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::devtools::ReloadReport;
use moqui_lib::object::Value;

/// The notification topic of reload reports.
pub const RELOAD_TOPIC: &str = "DevReload";

/// The groups whose users get the reports.
const GROUPS: &[&str] = &["ADMIN", "DEV_AIDE_USERS", "DEV_AIDE_EXTENDERS"];

/// Marks a factory whose reload reports are sent.
struct Listening;

/// Send `factory`'s reload reports from now on, once per factory.
pub(crate) fn ensure(factory: &Arc<ExecutionContextFactory>) {
    factory.extension::<Listening>(|factory| {
        moqui_core::devtools::on_reload(factory, send);
        Listening
    });
}

/// The report as a notification to the dev-aide's users, on its own context.
fn send(factory: &Arc<ExecutionContextFactory>, report: &ReloadReport) {
    if report.files.is_empty() {
        return;
    }
    let ctx = factory.new_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let Some(facade) = factory.entity() else {
        return;
    };
    let mut users = Vec::new();
    for group in GROUPS {
        let Ok(rows) = facade
            .find("moqui.security.UserGroupMember")
            .and_then(|find| find.eq("userGroupId", Value::string(*group)).list())
        else {
            continue;
        };
        for row in rows.iter() {
            if let Some(user) = row.get("userId").map(moqui_lib::object::to_plain_string) {
                if !users.contains(&user) {
                    users.push(user);
                }
            }
        }
    }
    if users.is_empty() {
        return;
    }
    let json = serde_json::to_string(report).unwrap_or_default();
    let sent = moqui_core::notify::NotificationMessage::new()
        .topic(RELOAD_TOPIC)
        .user_ids(users)
        .message_json(json)
        .persist(false)
        .show_alert(false)
        .send();
    if let Err(err) = sent {
        tracing::warn!("Could not send the Dev AIDE reload notification: {err}");
    }
}
