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

//! `org.moqui.impl.DevToolsServices`: the framework's development tools (`crate::devtools`) as services, on dev
//! instances only (plans/design-dev-aide.md 17).

use std::collections::BTreeMap;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use crate::context::ExecutionContext;
use crate::entity::EntityFacade;

pub(in crate::service) fn register(
    map: &mut BTreeMap<&'static str, crate::service::rust_services::RustServiceFn>,
) -> Result<(), String> {
    rust_service!(map, restart_server)?;
    Ok(())
}

/// Whether the current user may use the development tools here: a dev instance, and the `DEV_AIDE` permission or the
/// `ADMIN` group. Adds the error and answers false otherwise.
fn allowed(what: &str) -> bool {
    if !crate::devtools::is_dev_instance() {
        ExecutionContext::with_current(|ec| {
            ec.message().add_error(format!("{what} is only available when instance_purpose is dev"));
        });
        return false;
    }
    let permitted = crate::user::has_permission("DEV_AIDE").unwrap_or(false)
        || crate::user::is_in_group("ADMIN").unwrap_or(false);
    if !permitted {
        ExecutionContext::with_current(|ec| {
            ec.message().add_error(format!("{what} needs the DEV_AIDE permission"));
        });
    }
    permitted
}

/// `restart#Server`: shut the server down gracefully and run it again (plans/design-dev-aide.md 6.5). Out: `restarting`.
fn restart_server(
    _parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let mut out = BTreeMap::new();
    if !allowed("Restart") {
        return Ok(out);
    }
    let user = crate::user::session::current_user_id().ok().flatten().unwrap_or_default();
    tracing::warn!("Restart of the server requested by user {user}");
    // After the response: the shutdown waits for open requests, this one among them.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !crate::devtools::restart::request_restart() {
            tracing::error!("Restart requested, but this process is not a server that can restart");
        }
    });
    out.insert("restarting".to_string(), Value::Bool(true));
    Ok(out)
}
