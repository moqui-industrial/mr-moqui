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
    rust_service!(map, apply_xml_ops)?;
    Ok(())
}

/// Whether the current user may use the development tools here: a dev instance, and the `DEV_AIDE` permission or the
/// `ADMIN` group. Adds the error and answers false otherwise.
fn allowed(what: &str) -> bool {
    if !crate::devtools::is_dev_instance() {
        ExecutionContext::with_current(|ec| {
            ec.message()
                .add_error(format!("{what} is only available when instance_purpose is dev"));
        });
        return false;
    }
    let permitted =
        crate::user::has_permission("DEV_AIDE").unwrap_or(false) || crate::user::is_in_group("ADMIN").unwrap_or(false);
    if !permitted {
        ExecutionContext::with_current(|ec| {
            ec.message().add_error(format!("{what} needs the DEV_AIDE permission"));
        });
    }
    permitted
}

/// `apply#XmlOps`: the operations of the lossless XML layer (`moqui_lib::xml_edit`, plans/design-dev-aide.md 5.2) on a
/// text, answered as the new text and each operation's edits. Pure text: no file is read or written. Out: `text`,
/// `steps` (a list of lists of `{from, to, from16, to16, text}`).
fn apply_xml_ops(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let mut out = BTreeMap::new();
    if !allowed("The XML operations") {
        return Ok(out);
    }
    let text = parameters
        .get("text")
        .map(moqui_lib::object::to_plain_string)
        .unwrap_or_default();
    let ops = parameters
        .get("ops")
        .map(moqui_lib::json::to_json)
        .unwrap_or(serde_json::Value::Null);
    // ops may come as a JSON string (a form post) or a list (a JSON body)
    let ops = match ops {
        serde_json::Value::String(raw) => serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null),
        other => other,
    };
    let ops: Vec<moqui_lib::xml_edit::Op> = match serde_json::from_value(ops) {
        Ok(ops) => ops,
        Err(err) => {
            ExecutionContext::with_current(|ec| ec.message().add_error(format!("ops: {err}")));
            return Ok(out);
        }
    };
    match moqui_lib::xml_edit::apply(&text, &ops) {
        Ok(applied) => {
            let steps = serde_json::to_value(&applied.steps).unwrap_or(serde_json::Value::Null);
            out.insert("text".to_string(), Value::string(applied.text));
            out.insert("steps".to_string(), moqui_lib::json::from_json(&steps));
        }
        Err(err) => {
            ExecutionContext::with_current(|ec| ec.message().add_error(err.to_string()));
        }
    }
    Ok(out)
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
    let user = crate::user::session::current_user_id()
        .ok()
        .flatten()
        .unwrap_or_default();
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
