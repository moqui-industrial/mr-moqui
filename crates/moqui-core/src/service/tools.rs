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

//! Rust bodies for the tools services whose Groovy holds bytes or reaches Java internals: the print jobs of
//! `PrintServices.xml`, the data document notifications, and the encrypted field rewrite of `EntityServices.xml`.
//!
//! Print jobs are records only; there is no print server client (`moqui-cups` stays outside). A screen document is
//! rendered as `html-print` and turned into PDF by the host's engine.
//! Sending to a print server is phase 12 too (`moqui-cups`): the send records the failure on the job, as the
//! `moqui-cups` send does when the server does not take the job, and reports that network printing is not available.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use crate::context::ExecutionContext;
use crate::entity::{Condition, EntityFacade};
use crate::service::call::{sync_with, CallOptions};

const PRINT_DOCUMENT: &str = "org.moqui.impl.PrintServices.print#Document";
const PRINT_RESOURCE: &str = "org.moqui.impl.PrintServices.print#ResourceDocument";
const PRINT_SCREEN: &str = "org.moqui.impl.PrintServices.print#ScreenDocument";
const SEND_INTERNAL: &str = "org.moqui.impl.PrintServices.send#PrintJobInternal";
const DOCUMENT_NOTIFICATIONS: &str = "org.moqui.impl.EntityServices.send#DataDocumentNotifications";
const REWRITE_ENCRYPTED: &str = "org.moqui.impl.EntityServices.rewrite#EntityEncryptedFields";

const PRINT_JOB: &str = "moqui.basic.print.PrintJob";
/// The message of a send that has no print server to go to.
pub(crate) const PRINTING_UNAVAILABLE: &str =
    "Network printing support not installed (moqui-cups is not part of this build)";

/// True for a service this module runs. The definition keeps its parameters and has no actions.
pub(super) fn runs(name: &str) -> bool {
    matches!(
        name,
        PRINT_DOCUMENT | PRINT_RESOURCE | PRINT_SCREEN | SEND_INTERNAL | DOCUMENT_NOTIFICATIONS | REWRITE_ENCRYPTED
    )
}

pub(super) fn run(
    name: &str,
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    match name {
        PRINT_DOCUMENT => print_document(parameters),
        PRINT_RESOURCE => print_resource_document(parameters),
        PRINT_SCREEN => print_screen_document(parameters),
        SEND_INTERNAL => send_print_job_internal(parameters, facade),
        DOCUMENT_NOTIFICATIONS => send_document_notifications(parameters, facade),
        REWRITE_ENCRYPTED => rewrite_encrypted_fields(parameters, facade),
        other => Err(LibError::new(format!("tools service {other} is not implemented"))),
    }
}

fn text(parameters: &BTreeMap<String, Value>, name: &str) -> String {
    match parameters.get(name) {
        Some(value) if !is_empty(value) => to_plain_string(value),
        _ => String::new(),
    }
}

fn add_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

fn has_error() -> bool {
    ExecutionContext::with_current(|ctx| !ctx.message().errors().is_empty()).unwrap_or(false)
}

/// A service call from a body, as an XML `service-call`: a failure is left as a message error.
fn call(name: &str, parameters: &BTreeMap<String, Value>, options: &CallOptions) -> BTreeMap<String, Value> {
    match sync_with(name, parameters, options) {
        Ok(results) => results,
        Err(err) => {
            let message = err.message().to_string();
            ExecutionContext::with_current(|ctx| {
                if !ctx.message().errors().iter().any(|known| known == &message) {
                    ctx.message().add_error(message);
                }
            });
            BTreeMap::new()
        }
    }
}

/// The document bytes: `serialBlob` or `inputStream` (Java's two forms), as bytes or text.
fn document_bytes(parameters: &BTreeMap<String, Value>) -> Option<Vec<u8>> {
    ["serialBlob", "inputStream"]
        .iter()
        .find_map(|name| match parameters.get(*name) {
            Some(Value::Bytes(bytes)) => Some(bytes.to_vec()),
            Some(Value::String(text)) => Some(text.as_bytes().to_vec()),
            _ => None,
        })
}

/// The `print#DocumentInterface` fields of a print job.
fn job_fields(parameters: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    [
        "networkPrinterId",
        "createdDate",
        "username",
        "jobName",
        "copies",
        "duplex",
        "pageRanges",
        "contentType",
    ]
    .iter()
    .filter_map(|name| {
        parameters
            .get(*name)
            .filter(|value| !value.is_null())
            .map(|value| ((*name).to_string(), value.clone()))
    })
    .collect()
}

/// `print#Document`: create the `PrintJob` (not sent) in its own transaction, then send it.
fn print_document(parameters: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, LibError> {
    let Some(bytes) = document_bytes(parameters) else {
        add_error("Both inputStream and serialBlob are null, must specify one or the other");
        return Ok(BTreeMap::new());
    };
    let store = !matches!(parameters.get("storeDocument"), Some(Value::Bool(false)));
    let mut job = job_fields(parameters);
    job.insert("statusId".to_string(), Value::string("PtjNotSent"));
    if store {
        job.insert("document".to_string(), Value::Bytes(bytes.clone()));
    }
    let force_new = CallOptions {
        require_new_transaction: Some(true),
        ..CallOptions::default()
    };
    let created = call(&format!("create#{PRINT_JOB}"), &job, &force_new);
    if has_error() {
        return Ok(BTreeMap::new());
    }
    let print_job_id = created.get("printJobId").cloned().unwrap_or(Value::Null);
    let mut out = BTreeMap::new();
    out.insert("printJobId".to_string(), print_job_id.clone());
    job.insert("printJobId".to_string(), print_job_id);
    job.insert("document".to_string(), Value::Bytes(bytes));
    let mut send = BTreeMap::new();
    send.insert("printJob".to_string(), Value::Map(job.into_iter().collect()));
    let sent = call(SEND_INTERNAL, &send, &CallOptions::default());
    if let Some(job_id) = sent.get("jobId") {
        out.insert("jobId".to_string(), job_id.clone());
    }
    Ok(out)
}

/// `print#ResourceDocument`: the bytes at a resource location, printed and not stored.
fn print_resource_document(parameters: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, LibError> {
    let location = text(parameters, "resourceLocation");
    let found = ExecutionContext::with_current(|ctx| ctx.factory().resource().get(&location))
        .ok_or_else(|| LibError::new("no execution context is entered"))?;
    let resource = match found {
        Ok(resource) if resource.exists() => resource,
        _ => {
            add_error(format!("Could not find resource at [{location}]"));
            return Ok(BTreeMap::new());
        }
    };
    let bytes = resource.read_bytes().map_err(|err| LibError::new(err.message()))?;
    let mut print = parameters.clone();
    print.insert("inputStream".to_string(), Value::Bytes(bytes));
    if let Some(content_type) = moqui_lib::resource::content_type_for_name(&location) {
        print.insert("contentType".to_string(), Value::string(content_type));
    }
    print.insert("storeDocument".to_string(), Value::Bool(false));
    Ok(call(PRINT_DOCUMENT, &print, &CallOptions::default()))
}

/// `print#ScreenDocument`: render the screen as `html-print` (the `xsl-fo` mode), turn it into PDF, and pass the bytes
/// to `print#Document`, which stores the `PrintJob` and sends it. The screen parameters are `screenParameters` plus the
/// map `screenParametersStr` evaluates to; the job name defaults to the screen name and those parameter values.
fn print_screen_document(parameters: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, LibError> {
    let location = text(parameters, "screenLocation");
    let Some(factory) = ExecutionContext::with_current(|ctx| ctx.factory().clone()) else {
        return Err(LibError::new("no execution context is entered"));
    };
    let screens = crate::screen::ScreenFacade::of(&factory);
    let Ok(def) = screens.screen_def(&factory, &location) else {
        add_error(format!("Screen not found at [{location}]"));
        return Ok(BTreeMap::new());
    };
    let mut screen_parameters = BTreeMap::new();
    if let Some(Value::Map(given)) = parameters.get("screenParameters") {
        screen_parameters.extend(given.iter().map(|(name, value)| (name.clone(), value.clone())));
    }
    let expression = text(parameters, "screenParametersStr");
    if !expression.is_empty() {
        let evaluated = crate::script::eval_map(&expression)
            .map_err(|err| LibError::new(format!("screenParametersStr: {}", err.message())))?;
        screen_parameters.extend(evaluated);
    }
    let content_type = match text(parameters, "contentType") {
        found if found.is_empty() => "application/pdf".to_string(),
        found => found,
    };
    if content_type != "application/pdf" {
        add_error(format!(
            "A screen document prints as application/pdf, not {content_type}"
        ));
        return Ok(BTreeMap::new());
    }
    let rendered = crate::screen::make_render(&factory)
        .root_screen(&location)
        .render_mode("xsl-fo")
        .parameters(screen_parameters.clone())
        .render();
    let output = match rendered {
        Ok(output) => output,
        Err(err) => {
            add_error(format!("Could not render screen at [{location}]: {}", err.message()));
            return Ok(BTreeMap::new());
        }
    };
    if output.status != 200 {
        add_error(format!(
            "Could not render screen at [{location}]: status {}",
            output.status
        ));
        return Ok(BTreeMap::new());
    }
    let bytes = match crate::screen::PdfRenderer::render(&output.body_text()) {
        Ok(bytes) => bytes,
        Err(err) => {
            add_error(err.message());
            return Ok(BTreeMap::new());
        }
    };
    let mut print = parameters.clone();
    print.extend(screen_parameters.clone());
    if text(parameters, "jobName").is_empty() {
        let mut job_name = def.screen_name.clone();
        for value in screen_parameters.values() {
            job_name.push(' ');
            job_name.push_str(&to_plain_string(value));
        }
        print.insert("jobName".to_string(), Value::string(job_name));
    }
    print.insert("inputStream".to_string(), Value::Bytes(bytes));
    print.insert("contentType".to_string(), Value::string(content_type));
    Ok(call(PRINT_DOCUMENT, &print, &CallOptions::default()))
}

/// `send#PrintJobInternal`: there is no print server client yet, so the job is marked as a failed send, as the
/// `moqui-cups` send records a failure, and the call reports that network printing is not available.
fn send_print_job_internal(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    if facade.sim_active() {
        add_error("Printing is disabled in LLM sim session");
        return Ok(BTreeMap::new());
    }
    let print_job_id = match parameters.get("printJob") {
        Some(Value::Map(job)) => job.get("printJobId").map(to_plain_string).unwrap_or_default(),
        _ => String::new(),
    };
    if !print_job_id.is_empty() {
        let mut update = BTreeMap::new();
        update.insert("printJobId".to_string(), Value::string(&print_job_id));
        update.insert("statusId".to_string(), Value::string("PtjSendFailed"));
        update.insert("errorMessage".to_string(), Value::string(PRINTING_UNAVAILABLE));
        let force_new = CallOptions {
            require_new_transaction: Some(true),
            ..CallOptions::default()
        };
        call(&format!("update#{PRINT_JOB}"), &update, &force_new);
    }
    add_error(PRINTING_UNAVAILABLE);
    Ok(BTreeMap::new())
}

/// `StringUtilities.findAllFieldsNestedMap`: every value of `key` in nested maps and lists.
fn find_nested(key: &str, value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Map(map) => {
            for (name, item) in map {
                if name == key && !is_empty(item) {
                    found.insert(to_plain_string(item));
                }
                find_nested(key, item, found);
            }
        }
        Value::List(items) => items.iter().for_each(|item| find_nested(key, item, found)),
        _ => {}
    }
}

/// `send#DataDocumentNotifications`: a notification for each document to the users named anywhere in it (`userId`,
/// `fromUserId`, `toUserId`). The topic is the document type; with no `NotificationTopic` for it the title is the
/// document's title and the link is its first link.
fn send_document_notifications(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let documents = match parameters.get("documentList") {
        Some(Value::List(items)) => items.clone(),
        _ => Vec::new(),
    };
    for document in &documents {
        let Value::Map(map) = document else {
            continue;
        };
        let doc_type = map.get("_type").map(to_plain_string).unwrap_or_default();
        let mut user_ids = BTreeSet::new();
        for key in ["userId", "fromUserId", "toUserId"] {
            find_nested(key, document, &mut user_ids);
        }
        if user_ids.is_empty() {
            tracing::info!("Not sending {doc_type} DataDocument notification, no userIds found");
            continue;
        }
        let mut message = crate::notify::NotificationMessage::new();
        message
            .topic(doc_type.clone())
            .message_map(map.clone())
            .user_ids(user_ids.iter().cloned());
        let topic = facade
            .find("moqui.security.user.NotificationTopic")?
            .disable_authz()
            .eq("topic", Value::string(&doc_type))
            .one()?;
        if topic.is_none() && !doc_type.is_empty() {
            let data_document = facade
                .find("moqui.entity.document.DataDocument")?
                .disable_authz()
                .eq("dataDocumentId", Value::string(&doc_type))
                .one()?;
            if let Some(title) = data_document
                .as_ref()
                .and_then(|row| row.get("documentTitle"))
                .filter(|title| !is_empty(title))
            {
                message.title(to_plain_string(title));
            }
            let links = facade
                .find("moqui.entity.document.DataDocumentLink")?
                .disable_authz()
                .eq("dataDocumentId", Value::string(&doc_type))
                .list()?;
            if let Some(link) = links
                .get(0)
                .and_then(|row| row.get("linkUrl"))
                .filter(|link| !is_empty(link))
            {
                message.link(to_plain_string(link));
            }
        }
        message.send().map_err(|err| LibError::new(err.message()))?;
    }
    Ok(BTreeMap::new())
}

/// `rewrite#EntityEncryptedFields`: read each record with an encrypted field set and write it back, so it is stored
/// with the current key and algorithm. Java skips a record whose field does not decrypt; here a value that does not
/// decrypt fails the read, and the service reports that error.
fn rewrite_encrypted_fields(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let entity_name = text(parameters, "entityName");
    let (full_name, pk_names, encrypt_names) = facade.with_entity(&entity_name, |entity, _, _| {
        Ok((
            entity.full_name.clone(),
            entity
                .pk_field_names()
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
            entity
                .fields
                .iter()
                .filter(|field| field.encrypt)
                .map(|field| field.name.clone())
                .collect::<Vec<_>>(),
        ))
    })?;
    let mut out = BTreeMap::new();
    if encrypt_names.is_empty() {
        tracing::warn!("Not rewriting encrypted fields for entity {entity_name}, has no encrypted fields");
        return Ok(out);
    }
    let not_null: Vec<Condition> = encrypt_names
        .iter()
        .map(|name| Condition::is_not_null(name.as_str()))
        .collect();
    let mut find = facade
        .find(&full_name)?
        .disable_authz()
        .condition(Condition::or(not_null));
    for name in pk_names.iter().chain(encrypt_names.iter()) {
        find = find.select_field(name);
    }
    let mut updated = 0_i64;
    for value in find.list()?.values() {
        let mut row: IndexMap<String, Value> = IndexMap::new();
        for name in pk_names.iter().chain(encrypt_names.iter()) {
            row.insert(name.clone(), value.get(name).cloned().unwrap_or(Value::Null));
        }
        if encrypt_names
            .iter()
            .all(|name| row.get(name).is_none_or(Value::is_null))
        {
            continue;
        }
        let mut rewrite = facade.make_value(&full_name)?;
        for (name, field_value) in row {
            rewrite.set(name, field_value);
        }
        rewrite.update(facade)?;
        updated += 1;
    }
    out.insert("recordsUpdated".to_string(), Value::I64(updated));
    Ok(out)
}

#[cfg(test)]
mod tests;
