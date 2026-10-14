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

//! Rust bodies for the system message and entity sync services whose Groovy did not fit a script: the REST sender, the
//! `/rest/sm` receive, the SFTP send and poll of `moqui-sftp`, and the entity sync runs. The rest of `SystemMessageServices.xml` and `run#EntitySyncAll` are XML
//! actions. JSON-RPC is not part of this port, so a remote call is REST through [`moqui_lib::http::RestClient`].
//! This is not the cluster sender: it moves application data between systems an admin configured.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{basic_convert, is_empty, to_plain_string, Value};

use crate::context::ExecutionContext;
use crate::entity::EntityFacade;

mod entity_sync;
mod message;
mod sftp;
#[cfg(test)]
mod tests;

pub(crate) use message::receive_rest_message;

const SEND_REST: &str = "org.moqui.impl.SystemMessageServices.send#SystemMessageRest";
const SEND_SFTP: &str = "org.moqui.sftp.SftpMessageServices.send#SystemMessageSftp";
const POLL_SFTP: &str = "org.moqui.sftp.SftpMessageServices.poll#SystemMessageSftp";
const RUN_SYNC: &str = "org.moqui.impl.EntitySyncServices.run#EntitySync";
const INTERNAL_RUN: &str = "org.moqui.impl.EntitySyncServices.internalRun#EntitySync";
const PUT_DATA: &str = "org.moqui.impl.EntitySyncServices.put#EntitySyncData";
const GET_DATA: &str = "org.moqui.impl.EntitySyncServices.get#EntitySyncData";
const INCLUDE_LIST: &str = "org.moqui.impl.EntitySyncServices.get#EntitySyncIncludeList";

/// True for a service this module runs. The definition keeps its parameters and has no actions.
pub(super) fn runs(name: &str) -> bool {
    matches!(
        name,
        SEND_REST | SEND_SFTP | POLL_SFTP | RUN_SYNC | INTERNAL_RUN | PUT_DATA | GET_DATA | INCLUDE_LIST
    )
}

pub(super) fn run(
    name: &str,
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    match name {
        SEND_REST => message::send_rest(parameters, facade),
        SEND_SFTP => sftp::send(parameters, facade),
        POLL_SFTP => sftp::poll(parameters, facade),
        RUN_SYNC => entity_sync::run_sync(parameters, facade),
        INTERNAL_RUN => entity_sync::internal_run(parameters, facade),
        PUT_DATA => entity_sync::put_data(parameters, facade),
        GET_DATA => entity_sync::get_data(parameters, facade),
        INCLUDE_LIST => entity_sync::include_list(parameters, facade),
        other => Err(LibError::new(format!("system sync service {other} is not implemented"))),
    }
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(value) if !is_empty(value) => to_plain_string(value),
        _ => String::new(),
    }
}

fn param_text(parameters: &BTreeMap<String, Value>, name: &str) -> String {
    text_of(parameters.get(name))
}

fn map_text(map: &IndexMap<String, Value>, name: &str) -> String {
    text_of(map.get(name))
}

/// Milliseconds of a time stamp value, or `None` for null or a value that is not a time.
fn millis_of(value: Option<&Value>) -> Option<i64> {
    match basic_convert(value?, "Timestamp").value {
        Value::Timestamp(stamp) => Some(stamp.as_millisecond()),
        _ => None,
    }
}

fn integer_of(value: Option<&Value>) -> Option<i64> {
    match basic_convert(value?, "Long").value {
        Value::I64(number) => Some(number),
        _ => None,
    }
}

fn stamp(millis: i64) -> Value {
    jiff::Timestamp::from_millisecond(millis).map_or(Value::Null, Value::Timestamp)
}

fn now_millis() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

fn add_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

fn has_error() -> bool {
    ExecutionContext::with_current(|ctx| !ctx.message().errors().is_empty()).unwrap_or(false)
}

/// The errors joined by newlines, as `getErrorsString`, then cleared with the messages, as `clearErrors`.
fn take_errors() -> String {
    ExecutionContext::with_current(|ctx| {
        let mut message = ctx.message();
        let text = message.errors().join("\n");
        message.clear();
        text
    })
    .unwrap_or_default()
}

/// A service call from a body, as an XML `service-call`: a failure becomes a message, as the script call builder
/// records it, and is left for the caller's `has_error` check, the way the generated `if (ec.message.hasError()) return`
/// reads it.
fn call_service(name: &str, parameters: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    match super::call::sync(name, &parameters) {
        Ok(results) => results,
        Err(err) => {
            let text = err.message().to_string();
            ExecutionContext::with_current(|ctx| {
                let known = ctx.message().errors().iter().any(|known| known == &text);
                if !known {
                    ctx.message().add_error(text);
                }
            });
            BTreeMap::new()
        }
    }
}

fn pairs<const N: usize>(entries: [(&str, Value); N]) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}
