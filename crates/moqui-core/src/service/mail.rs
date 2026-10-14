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

//! Outbound SMTP and inbound IMAP/POP3 for the email services whose locations stay Groovy.
//!
//! `send#EmailTemplate` follows `sendEmailTemplate.groovy`. A template with `bodyScreenLocation` renders
//! that screen with `makeRender()` (no socket, its own web slot, so a request in progress is not touched)
//! in `html` for the body and in `text` for the alternative part, with the service parameters, the
//! `bodyParameters` and the template values (`fromAddress`, `subject`, `webappName`, `webHostName`,
//! `emailMessageId` and the rest) in the context, and links built from the conf `webapp` named by
//! `webappName`. A `body` or `bodyText` the caller passed is used instead of that render. Screen
//! attachments render the same way. A screen attachment in `xsl-fo` is skipped with a logged warning,
//! as Java does when the PDF transform fails, and the email is sent without it: PDF is phase 12. A render
//! that fails stops the send with the message `Error sending email: ...`, as the Groovy outer catch did.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{basic_convert, is_empty, to_plain_string, Value};

use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::{EntityFacade, EntityValue};
use crate::screen::{make_render, ScreenFacade, PDF_UNAVAILABLE};
use crate::transaction::TransactionAttribute;

const TEMPLATE: &str = "moqui.basic.email.EmailTemplate";
const SERVER: &str = "moqui.basic.email.EmailServer";
const MESSAGE: &str = "moqui.basic.email.EmailMessage";
const ATTACHMENT: &str = "moqui.basic.email.EmailTemplateAttachment";

pub(super) fn run(
    name: &str,
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    match name {
        "org.moqui.impl.EmailServices.send#Email" | "org.moqui.impl.EmailServices.send#EmailTemplate" => {
            send_template(parameters, facade)
        }
        "org.moqui.impl.EmailServices.send#EmailMessage" => send_message(parameters, facade),
        "org.moqui.impl.EmailServices.poll#EmailServer" => poll_server(parameters, facade),
        other => Err(LibError::new(format!("mail service {other} is not implemented"))),
    }
}

fn send_template(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    if facade.sim_active() {
        return Ok(ids("SIM", ""));
    }
    let template_id = param_text(parameters, "emailTemplateId");
    let Some(template) = find_one(facade, TEMPLATE, "emailTemplateId", &template_id)? else {
        return fail(format!("No EmailTemplate record found for ID [{template_id}]"));
    };
    // The script's context: the parameters, then the bodyParameters over them, then the template values.
    let _frame = crate::script::ContextFrame::push();
    for (name, value) in parameters {
        put_context(name, value.clone())?;
    }
    if let Some(map) = body_parameters(parameters) {
        for (name, value) in map {
            put_context(name, value.clone())?;
        }
    }
    let mut cc = param_text(parameters, "ccAddresses");
    let mut bcc = param_text(parameters, "bccAddresses");
    cc = merge_lists(&cc, &field_text(&template, "ccAddresses"));
    bcc = merge_lists(&bcc, &field_text(&template, "bccAddresses"));
    let headers = expand_headers(&template, body_parameters(parameters))?;
    let supplied_body = param_text(parameters, "body");
    let supplied_text = param_text(parameters, "bodyText");
    let screen = field_text(&template, "bodyScreenLocation");
    if screen.is_empty() && supplied_body.is_empty() && supplied_text.is_empty() {
        return fail("email has no body");
    }
    let env = RenderEnv::new(&template, parameters);
    let email_type = {
        let given = param_text(parameters, "emailTypeEnumId");
        if given.is_empty() {
            field_text(&template, "emailTypeEnumId")
        } else {
            given
        }
    };
    put_context("emailTypeEnumId", Value::string(&email_type))?;
    put_context("ccAddresses", Value::string(&cc))?;
    put_context("bccAddresses", Value::string(&bcc))?;
    put_context("fromAddress", Value::string(&headers.from_address))?;
    put_context("fromName", Value::string(&headers.from_name))?;
    put_context("subject", Value::string(&headers.subject))?;
    put_context("webappName", Value::string(&env.webapp_name))?;
    put_context("webHostName", Value::string(&env.web_host_name))?;
    let mut email_message_id = String::new();
    if create_message(parameters) {
        email_message_id = insert_draft(facade, parameters, &template, &headers, &cc, &bcc)?;
        put_context("emailMessageId", Value::string(&email_message_id))?;
    }
    let (body, body_text) = match render_bodies(&env, &screen, supplied_body, supplied_text) {
        Ok(pair) => pair,
        Err(err) => return Ok(send_error(&err, &email_message_id, &template_id)),
    };
    if !email_message_id.is_empty() {
        update_ready(facade, &email_message_id, &body, &body_text)?;
    }
    let server_id = field_text(&template, "emailServerId");
    let Some(server) = find_one(facade, SERVER, "emailServerId", &server_id)? else {
        return fail_with(
            format!("No EmailServer record found for EmailTemplate {template_id}"),
            &email_message_id,
        );
    };
    if headers.from_address.is_empty() {
        return fail_with(
            format!("From address is empty for EmailTemplate {template_id}"),
            &email_message_id,
        );
    }
    let host = field_text(&server, "smtpHost");
    if host.is_empty() {
        tracing::warn!(
            server = %server_id,
            message = %email_message_id,
            template = %template_id,
            "SMTP Host is empty, not sending email"
        );
        return Ok(ids(&email_message_id, ""));
    }
    if let Err(text) = require_host(&host) {
        return fail_with(text, &email_message_id);
    }
    let mut letter = Letter {
        subject: headers.subject.clone(),
        from_address: headers.from_address.clone(),
        from_name: headers.from_name.clone(),
        to: split_addresses(&param_text(parameters, "toAddresses")),
        cc: split_addresses(&cc),
        bcc: split_addresses(&bcc),
        reply_to: split_addresses(&field_text(&template, "replyToAddresses")),
        bounce: field_text(&template, "bounceAddress"),
        body,
        body_text,
        attachments: Vec::new(),
        message_id: new_message_id(&headers.from_address),
    };
    if !filter_recipients(&mut letter, &field_text(&server, "allowedToDomains"), &email_message_id) {
        return Ok(ids(&email_message_id, ""));
    }
    letter.attachments = match collect_attachments(facade, parameters, &template_id, &env) {
        Ok(attachments) => attachments,
        Err(err) => return Ok(send_error(&err, &email_message_id, &template_id)),
    };
    if let Err(err) = deliver(&letter, &server) {
        note_message(format!("Error sending email: {err}"));
        return Ok(ids(&email_message_id, ""));
    }
    if !email_message_id.is_empty() {
        update_sent(facade, &email_message_id, &letter.message_id)?;
    }
    Ok(ids(&email_message_id, &letter.message_id))
}

fn send_message(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let email_message_id = param_text(parameters, "emailMessageId");
    if facade.sim_active() {
        return Ok(ids("SIM", ""));
    }
    let Some(message) = find_one(facade, MESSAGE, "emailMessageId", &email_message_id)? else {
        return fail(format!("No EmailMessage record found for ID {email_message_id}"));
    };
    let status = field_text(&message, "statusId");
    if status == "ES_DRAFT" {
        return fail(format!("Email Message {email_message_id} is in Draft status"));
    }
    if status == "ES_CANCELLED" {
        return fail(format!("Email Message {email_message_id} is Cancelled"));
    }
    let body = field_text(&message, "body");
    let body_text = field_text(&message, "bodyText");
    let from_address = field_text(&message, "fromAddress");
    let to_addresses = field_text(&message, "toAddresses");
    if body.is_empty() && body_text.is_empty() {
        return fail(format!("Email Message {email_message_id} has no body"));
    }
    if from_address.is_empty() {
        return fail(format!("Email Message {email_message_id} has no from address"));
    }
    if to_addresses.is_empty() {
        return fail(format!("Email Message {email_message_id} has no to address"));
    }
    let server_id = field_text(&message, "emailServerId");
    let Some(server) = find_one(facade, SERVER, "emailServerId", &server_id)? else {
        return fail(format!(
            "No Email Server record found for Email Message {email_message_id}"
        ));
    };
    let host = field_text(&server, "smtpHost");
    if host.is_empty() {
        tracing::warn!(server = %server_id, message = %email_message_id, "SMTP Host is empty, not sending email");
        return Ok(ids(&email_message_id, ""));
    }
    if let Err(text) = require_host(&host) {
        return fail(text);
    }
    let template_id = field_text(&message, "emailTemplateId");
    let template = find_one(facade, TEMPLATE, "emailTemplateId", &template_id)?;
    let (reply_to, bounce) = match &template {
        Some(row) => (field_text(row, "replyToAddresses"), field_text(row, "bounceAddress")),
        None => (String::new(), String::new()),
    };
    let mut letter = Letter {
        subject: field_text(&message, "subject"),
        from_address,
        from_name: field_text(&message, "fromName"),
        to: split_addresses(&to_addresses),
        cc: split_addresses(&field_text(&message, "ccAddresses")),
        bcc: split_addresses(&field_text(&message, "bccAddresses")),
        reply_to: split_addresses(&reply_to),
        bounce,
        body,
        body_text,
        attachments: Vec::new(),
        message_id: new_message_id(&field_text(&message, "fromAddress")),
    };
    if !filter_recipients(&mut letter, &field_text(&server, "allowedToDomains"), &email_message_id) {
        return Ok(ids(&email_message_id, ""));
    }
    if let Err(err) = deliver(&letter, &server) {
        note_message(format!("Error sending email: {err}"));
        return Ok(ids(&email_message_id, ""));
    }
    if status == "ES_READY" || status == "ES_BOUNCED" {
        update_sent(facade, &email_message_id, &letter.message_id)?;
    }
    Ok(ids(&email_message_id, &letter.message_id))
}

struct Expanded {
    from_address: String,
    from_name: String,
    subject: String,
}

fn expand_headers(template: &EntityValue, parameters: Option<&IndexMap<String, Value>>) -> Result<Expanded, LibError> {
    let _frame = crate::script::ContextFrame::push();
    if let Some(map) = parameters {
        for (name, value) in map {
            crate::script::set_context_name(name, value.clone()).map_err(|err| LibError::new(err.message()))?;
        }
    }
    let expand = |text: &str| crate::script::expand(text).map_err(|err| LibError::new(err.message()));
    Ok(Expanded {
        from_address: expand(&field_text(template, "fromAddress"))?,
        from_name: expand(&field_text(template, "fromName"))?,
        subject: expand(&field_text(template, "subject"))?,
    })
}

/// What a screen render for this email needs: the factory, the webapp the links are for, and the
/// `bodyParameters`, which the render also sees as its request parameters.
struct RenderEnv {
    factory: Option<Arc<ExecutionContextFactory>>,
    webapp_name: String,
    web_host_name: String,
    base_url: String,
    parameters: BTreeMap<String, Value>,
}

impl RenderEnv {
    fn new(template: &EntityValue, parameters: &BTreeMap<String, Value>) -> Self {
        let factory = ExecutionContext::with_current(|ctx| Arc::clone(ctx.factory()));
        let webapp_name = {
            let named = field_text(template, "webappName");
            if named.is_empty() {
                "webroot".to_string()
            } else {
                named
            }
        };
        let base_url = factory
            .as_ref()
            .map(|factory| factory.webapp_root_url(&webapp_name))
            .unwrap_or_default();
        let parameters = body_parameters(parameters)
            .map(|map| map.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
            .unwrap_or_default();
        Self {
            factory,
            webapp_name,
            web_host_name: field_text(template, "webHostName"),
            base_url,
            parameters,
        }
    }

    /// Render `location` as the root screen, or `screen_path` under the default root screen (Java's
    /// `rootScreenFromHost`, which this port has one of) as the last standalone screen, in `mode`.
    fn render(&self, location: &str, screen_path: &str, mode: &str) -> Result<String, String> {
        let Some(factory) = &self.factory else {
            return Err("no execution context to render a screen in".to_string());
        };
        let mut parameters = self.parameters.clone();
        let mut render = make_render(factory).base_url(self.base_url.clone()).render_mode(mode);
        if screen_path.is_empty() {
            render = render.root_screen(location);
        } else {
            parameters.insert("lastStandalone".to_string(), Value::string("true"));
            render = render.screen_path(screen_path);
        }
        let shown = if screen_path.is_empty() { location } else { screen_path };
        let output = render
            .parameters(parameters)
            .render()
            .map_err(|err| format!("could not render {shown} as {mode}: {}", err.message()))?;
        if output.status != 200 {
            return Err(format!("could not render {shown} as {mode}: status {}", output.status));
        }
        Ok(output.body_text())
    }

    /// True when the screen at `location` has a `text` render, as `render-modes` says.
    fn allows_text(&self, location: &str) -> bool {
        let Some(factory) = &self.factory else {
            return false;
        };
        ScreenFacade::of(factory)
            .screen_def(factory, location)
            .map(|def| def.allows_render_mode("text"))
            .unwrap_or(false)
    }

    fn mime_type(&self, mode: &str) -> Option<String> {
        let factory = self.factory.as_ref()?;
        ScreenFacade::of(factory).mime_type(mode).map(str::to_string)
    }
}

/// The html body and the text alternative. A part the caller passed wins; the other is rendered from
/// `bodyScreenLocation` (html, and text when the screen has a text render). Blank output is no part.
fn render_bodies(env: &RenderEnv, screen: &str, body: String, body_text: String) -> Result<(String, String), String> {
    if screen.is_empty() {
        return Ok((body, body_text));
    }
    let body = if body.is_empty() {
        blank_to_empty(env.render(screen, "", "html")?)
    } else {
        body
    };
    let body_text = if body_text.is_empty() && env.allows_text(screen) {
        blank_to_empty(env.render(screen, "", "text")?)
    } else {
        body_text
    };
    Ok((body, body_text))
}

fn blank_to_empty(text: String) -> String {
    if text.trim().is_empty() {
        String::new()
    } else {
        text
    }
}

/// The Groovy script's outer catch: log, add a message (not an error), and send nothing. A created
/// `EmailMessage` stays as it was.
fn send_error(err: &str, email_message_id: &str, template_id: &str) -> BTreeMap<String, Value> {
    tracing::error!(template = %template_id, message = %email_message_id, error = %err, "Error in sendEmailTemplate");
    note_message(format!("Error sending email: {err}"));
    ids(email_message_id, "")
}

fn put_context(name: &str, value: Value) -> Result<(), LibError> {
    crate::script::set_context_name(name, value).map_err(|err| LibError::new(err.message()))
}

fn create_message(parameters: &BTreeMap<String, Value>) -> bool {
    match parameters.get("createEmailMessage") {
        Some(Value::Bool(flag)) => *flag,
        Some(value) if is_empty(value) => false,
        Some(value) => {
            let text = to_plain_string(value).to_ascii_lowercase();
            !matches!(text.as_str(), "false" | "n" | "no" | "0")
        }
        None => true,
    }
}

fn insert_draft(
    facade: &EntityFacade,
    parameters: &BTreeMap<String, Value>,
    template: &EntityValue,
    headers: &Expanded,
    cc: &str,
    bcc: &str,
) -> Result<String, LibError> {
    let email_type = {
        let given = param_text(parameters, "emailTypeEnumId");
        if given.is_empty() {
            field_text(template, "emailTypeEnumId")
        } else {
            given
        }
    };
    let mut from_user = param_text(parameters, "fromUserId");
    if from_user.is_empty() {
        from_user = crate::user::current_user_id().ok().flatten().unwrap_or_default();
    }
    let to_user = param_text(parameters, "toUserId");
    let to = param_text(parameters, "toAddresses");
    let template_id = field_text(template, "emailTemplateId");
    let server_id = field_text(template, "emailServerId");
    force_new(facade, || {
        let _authz = ExecutionContext::with_current(|ctx| ctx.disable_authz());
        let mut row = facade.make_value(MESSAGE)?;
        row.set_sequenced_id_primary(facade)?;
        let id = field_text(&row, "emailMessageId");
        set_text(&mut row, "statusId", "ES_DRAFT");
        set_text(&mut row, "subject", &headers.subject);
        set_text(&mut row, "fromAddress", &headers.from_address);
        set_text(&mut row, "fromName", &headers.from_name);
        set_text(&mut row, "toAddresses", &to);
        set_text(&mut row, "ccAddresses", cc);
        set_text(&mut row, "bccAddresses", bcc);
        set_text(&mut row, "contentType", "text/html");
        set_text(&mut row, "emailTypeEnumId", &email_type);
        set_text(&mut row, "emailTemplateId", &template_id);
        set_text(&mut row, "emailServerId", &server_id);
        set_text(&mut row, "fromUserId", &from_user);
        set_text(&mut row, "toUserId", &to_user);
        row.create(facade)?;
        Ok(id)
    })
}

fn update_ready(facade: &EntityFacade, id: &str, body: &str, body_text: &str) -> Result<(), LibError> {
    force_new(facade, || {
        let _authz = ExecutionContext::with_current(|ctx| ctx.disable_authz());
        let mut row = facade.make_value(MESSAGE)?;
        set_text(&mut row, "emailMessageId", id);
        set_text(&mut row, "statusId", "ES_READY");
        set_text(&mut row, "body", body);
        set_text(&mut row, "bodyText", body_text);
        row.update(facade)?;
        Ok(())
    })
}

fn update_sent(facade: &EntityFacade, id: &str, message_id: &str) -> Result<(), LibError> {
    force_new(facade, || {
        let _authz = ExecutionContext::with_current(|ctx| ctx.disable_authz());
        let mut row = facade.make_value(MESSAGE)?;
        set_text(&mut row, "emailMessageId", id);
        set_text(&mut row, "statusId", "ES_SENT");
        set_text(&mut row, "messageId", message_id);
        row.set("sentDate", now_stamp());
        row.update(facade)?;
        Ok(())
    })
}

fn force_new<T>(facade: &EntityFacade, body: impl FnOnce() -> Result<T, LibError>) -> Result<T, LibError> {
    let tx = facade.transactions_for(MESSAGE)?;
    tx.run(TransactionAttribute::ForceNew, None, body)
}

fn filter_recipients(letter: &mut Letter, allowed: &str, email_message_id: &str) -> bool {
    let domains: Vec<String> = allowed
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect();
    if domains.is_empty() {
        letter.to.retain(|address| !address.trim().is_empty());
        return !letter.to.is_empty();
    }
    let mut skipped = Vec::new();
    retain_domains(&mut letter.to, &domains, &mut skipped);
    retain_domains(&mut letter.cc, &domains, &mut skipped);
    retain_domains(&mut letter.bcc, &domains, &mut skipped);
    if letter.to.is_empty() {
        tracing::warn!(
            message = %email_message_id,
            skipped = %skipped.join(","),
            "not sending email with no To address; domains were not allowed"
        );
        note_message(format!(
            "Not sending email message with no To Address; address(es) skipped because domain not allowed: {skipped:?}"
        ));
        return false;
    }
    if !skipped.is_empty() {
        tracing::warn!(skipped = %skipped.join(","), "some email addresses were skipped because the domain is not allowed");
    }
    true
}

fn retain_domains(addresses: &mut Vec<String>, domains: &[String], skipped: &mut Vec<String>) {
    addresses.retain(|address| {
        if domain_allowed(address, domains) {
            true
        } else {
            skipped.push(address.clone());
            false
        }
    });
}

fn domain_allowed(address: &str, domains: &[String]) -> bool {
    let address = address.trim();
    if address.is_empty() {
        return false;
    }
    if domains.is_empty() {
        return true;
    }
    let Some((_, domain)) = address.rsplit_once('@') else {
        return false;
    };
    domains.iter().any(|entry| domain.ends_with(entry))
}

fn require_host(host: &str) -> Result<(), String> {
    if host_allowed(host) {
        Ok(())
    } else {
        Err(format!("Email host {host} is not in email_allowed_hosts"))
    }
}

fn host_allowed(host: &str) -> bool {
    let allowed = moqui_lib::env::get_prop_or_env("email_allowed_hosts");
    if allowed.trim().is_empty() {
        return true;
    }
    let mut name = host.trim().to_ascii_lowercase();
    if name.is_empty() {
        return false;
    }
    if name.ends_with('.') {
        name.pop();
    }
    for part in allowed.split(',') {
        let mut entry = part.trim().to_ascii_lowercase();
        if entry.is_empty() {
            continue;
        }
        if entry.ends_with('.') {
            entry.pop();
        }
        if name == entry {
            return true;
        }
        if dns_suffix(&entry) && name.ends_with(&format!(".{entry}")) {
            return true;
        }
    }
    false
}

fn dns_suffix(entry: &str) -> bool {
    !entry.contains(':') && entry.chars().any(|ch| ch.is_ascii_alphabetic())
}

/// The `attachments` parameter, then the template's `EmailTemplateAttachment` rows. An error is a
/// screen render or an expression that failed, which stops the send.
fn collect_attachments(
    facade: &EntityFacade,
    parameters: &BTreeMap<String, Value>,
    template_id: &str,
    env: &RenderEnv,
) -> Result<Vec<AttachmentBytes>, String> {
    let mut out = Vec::new();
    if let Some(Value::List(items)) = parameters.get("attachments") {
        for item in items {
            if let Value::Map(map) = item {
                push_parameter_attachment(&mut out, map, env)?;
            }
        }
    }
    let rows = facade
        .find(ATTACHMENT)
        .and_then(|find| find.eq("emailTemplateId", Value::string(template_id)).list())
        .map_err(|err| err.to_string())?;
    for row in rows.iter() {
        push_stored_attachment(&mut out, row, env)?;
    }
    Ok(out)
}

fn push_parameter_attachment(
    out: &mut Vec<AttachmentBytes>,
    map: &IndexMap<String, Value>,
    env: &RenderEnv,
) -> Result<(), String> {
    let filename = expand_text(&map_text(map, "fileName"))?;
    let mode = map_text(map, "screenRenderMode");
    let location = map_text(map, "attachmentLocation");
    let screen_path = map_text(map, "screenPath");
    if let Some(text) = map.get("contentText").filter(|value| !is_empty(value)) {
        let mime = fallback_type(&map_text(map, "contentType"), &filename, "text/plain");
        out.push(AttachmentBytes {
            filename,
            content_type: mime,
            bytes: to_plain_string(text).into_bytes(),
        });
        return Ok(());
    }
    if let Some(Value::Bytes(bytes)) = map.get("contentBytes") {
        let mime = fallback_type(&map_text(map, "contentType"), &filename, "application/octet-stream");
        out.push(AttachmentBytes {
            filename,
            content_type: mime,
            bytes: bytes.clone(),
        });
        return Ok(());
    }
    if !mode.is_empty() && (!location.is_empty() || !screen_path.is_empty()) {
        let screen = ScreenAttachment {
            filename: &filename,
            mode: &mode,
            location: &location,
            screen_path: &screen_path,
            content_type: &map_text(map, "contentType"),
        };
        return render_attachment(out, &screen, env);
    }
    if location.is_empty() {
        tracing::error!(file = %filename, "attachment info is invalid: no content, screen or location");
        return Ok(());
    }
    match read_location(&location) {
        Ok(bytes) => {
            let mime = fallback_type(&map_text(map, "contentType"), &filename, "application/octet-stream");
            out.push(AttachmentBytes {
                filename,
                content_type: mime,
                bytes,
            });
        }
        Err(err) => note_error(err),
    }
    Ok(())
}

/// One `EmailTemplateAttachment`: skipped when `attachmentCondition` is false; a screen render (one for
/// each entry of `forEachIn`, the entry's fields in the context) when `screenRenderMode` is set; else the
/// bytes at `attachmentLocation`.
fn push_stored_attachment(out: &mut Vec<AttachmentBytes>, row: &EntityValue, env: &RenderEnv) -> Result<(), String> {
    let mode = field_text(row, "screenRenderMode");
    let location = field_text(row, "attachmentLocation");
    let filename = field_text(row, "fileName");
    let condition = field_text(row, "attachmentCondition");
    if !condition.is_empty() {
        let holds = crate::script::eval_condition(&condition)
            .map_err(|err| format!("attachmentCondition of {filename}: {}", err.message()))?;
        if !holds {
            return Ok(());
        }
    }
    if !mode.is_empty() {
        let screen = ScreenAttachment {
            filename: &filename,
            mode: &mode,
            location: &location,
            screen_path: &field_text(row, "screenPath"),
            content_type: "",
        };
        let for_each_in = field_text(row, "forEachIn");
        if for_each_in.is_empty() {
            return render_attachment(out, &screen, env);
        }
        let entries = match crate::script::eval_value(&for_each_in)
            .map_err(|err| format!("forEachIn of {filename}: {}", err.message()))?
        {
            Value::List(items) => items,
            Value::Null => Vec::new(),
            other => return Err(format!("forEachIn of {filename} is not a list: {other:?}")),
        };
        for entry in entries {
            let _frame = crate::script::ContextFrame::push();
            match entry {
                Value::Map(map) => {
                    for (name, value) in map {
                        crate::script::set_context_name(&name, value).map_err(|err| err.message().to_string())?;
                    }
                }
                other => {
                    crate::script::set_context_name("forEachEntry", other).map_err(|err| err.message().to_string())?;
                }
            }
            render_attachment(out, &screen, env)?;
        }
        return Ok(());
    }
    if location.is_empty() {
        note_error(format!("attachment {filename} has no location"));
        return Ok(());
    }
    match read_location(&location) {
        Ok(bytes) => out.push(AttachmentBytes {
            filename,
            content_type: mime_from_name(&field_text(row, "fileName")),
            bytes,
        }),
        Err(err) => note_error(err),
    }
    Ok(())
}

struct ScreenAttachment<'a> {
    filename: &'a str,
    mode: &'a str,
    location: &'a str,
    screen_path: &'a str,
    content_type: &'a str,
}

/// `renderScreenAttachment`: the file name defaults to the screen's name with the mode as extension and
/// is expanded in the context; the content type is the caller's or the mode's; blank output is no
/// attachment. `xsl-fo` is logged and skipped, as Java does when the PDF transform fails.
fn render_attachment(
    out: &mut Vec<AttachmentBytes>,
    screen: &ScreenAttachment<'_>,
    env: &RenderEnv,
) -> Result<(), String> {
    let mut filename = screen.filename.to_string();
    if filename.is_empty() {
        let extension = if screen.mode == "xsl-fo" { "pdf" } else { screen.mode };
        let source = if screen.location.is_empty() {
            screen.screen_path
        } else {
            screen.location
        };
        let base = source.rsplit('/').next().unwrap_or(source);
        let stem = base.rsplit_once('.').map_or(base, |(stem, _)| stem);
        filename = format!("{stem}.{extension}");
    }
    let filename = expand_text(&filename)?;
    if screen.mode == "xsl-fo" {
        tracing::warn!(file = %filename, "Error generating PDF from XSL-FO: {PDF_UNAVAILABLE}");
        return Ok(());
    }
    let text = env.render(screen.location, screen.screen_path, screen.mode)?;
    if text.trim().is_empty() {
        return Ok(());
    }
    let content_type = if screen.content_type.is_empty() {
        env.mime_type(screen.mode)
            .unwrap_or_else(|| "application/octet-stream".to_string())
    } else {
        screen.content_type.to_string()
    };
    out.push(AttachmentBytes {
        filename,
        content_type,
        bytes: text.into_bytes(),
    });
    Ok(())
}

fn expand_text(text: &str) -> Result<String, String> {
    crate::script::expand(text).map_err(|err| err.message().to_string())
}

fn read_location(location: &str) -> Result<Vec<u8>, String> {
    let path = crate::context::locate(location).map_err(|err| err.message().to_string())?;
    std::fs::read(&path).map_err(|err| format!("could not read attachment {location}: {err}"))
}

fn fallback_type(given: &str, filename: &str, default: &str) -> String {
    if !given.is_empty() {
        given.to_string()
    } else if filename.is_empty() {
        default.to_string()
    } else {
        mime_from_name(filename)
    }
}

fn mime_from_name(filename: &str) -> String {
    match filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("html" | "htm") => "text/html",
        Some("txt") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("csv") => "text/csv",
        Some("xml") => "application/xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[derive(Clone, Debug)]
struct AttachmentBytes {
    filename: String,
    content_type: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Letter {
    subject: String,
    from_address: String,
    from_name: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    reply_to: Vec<String>,
    bounce: String,
    body: String,
    body_text: String,
    attachments: Vec<AttachmentBytes>,
    message_id: String,
}

fn deliver(letter: &Letter, server: &EntityValue) -> Result<(), String> {
    if push_sink(letter) {
        return Ok(());
    }
    lettre_send(letter, server)
}

fn lettre_send(letter: &Letter, server: &EntityValue) -> Result<(), String> {
    let host = field_text(server, "smtpHost");
    let port = parse_port(&field_text(server, "smtpPort"), 25)?;
    let message = build_lettre(letter)?;
    let user = field_text(server, "mailUsername");
    let password = field_text(server, "mailPassword");
    let transport = if field_text(server, "smtpSsl") == "Y" {
        creds(
            lettre::SmtpTransport::relay(&host)
                .map_err(|err| err.to_string())?
                .port(port),
            &user,
            &password,
        )
        .build()
    } else if field_text(server, "smtpStartTls") == "Y" {
        creds(
            lettre::SmtpTransport::starttls_relay(&host)
                .map_err(|err| err.to_string())?
                .port(port),
            &user,
            &password,
        )
        .build()
    } else {
        creds(
            lettre::SmtpTransport::builder_dangerous(&host).port(port),
            &user,
            &password,
        )
        .build()
    };
    lettre::Transport::send(&transport, &message)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn creds(
    builder: lettre::transport::smtp::SmtpTransportBuilder,
    user: &str,
    password: &str,
) -> lettre::transport::smtp::SmtpTransportBuilder {
    if user.is_empty() {
        builder
    } else {
        builder.credentials(lettre::transport::smtp::authentication::Credentials::new(
            user.to_string(),
            password.to_string(),
        ))
    }
}

fn build_lettre(letter: &Letter) -> Result<lettre::Message, String> {
    let from = mailbox(&letter.from_address, &letter.from_name)?;
    let mut builder = lettre::Message::builder()
        .from(from)
        .message_id(Some(letter.message_id.clone()));
    if !letter.subject.is_empty() {
        builder = builder.subject(letter.subject.clone());
    }
    let mut recipients = Vec::new();
    for address in letter.to.iter().chain(letter.cc.iter()).chain(letter.bcc.iter()) {
        let parsed = mailbox(address, "")?;
        recipients.push(parsed.email.clone());
    }
    for address in &letter.to {
        builder = builder.to(mailbox(address, "")?);
    }
    for address in &letter.cc {
        builder = builder.cc(mailbox(address, "")?);
    }
    for address in &letter.bcc {
        builder = builder.bcc(mailbox(address, "")?);
    }
    for address in &letter.reply_to {
        builder = builder.reply_to(mailbox(address, "")?);
    }
    if !letter.bounce.is_empty() {
        let sender = letter
            .bounce
            .trim()
            .parse()
            .map_err(|err| format!("bounce address {} is not valid: {err}", letter.bounce))?;
        let envelope = lettre::address::Envelope::new(Some(sender), recipients).map_err(|err| err.to_string())?;
        builder = builder.envelope(envelope);
    }
    let message = if letter.attachments.is_empty() {
        finish_body(builder, letter)?
    } else {
        builder
            .multipart(with_attachments(letter)?)
            .map_err(|err| err.to_string())?
    };
    Ok(message)
}

fn finish_body(builder: lettre::message::MessageBuilder, letter: &Letter) -> Result<lettre::Message, String> {
    if !letter.body.is_empty() && !letter.body_text.is_empty() {
        return builder
            .multipart(lettre::message::MultiPart::alternative_plain_html(
                letter.body_text.clone(),
                letter.body.clone(),
            ))
            .map_err(|err| err.to_string());
    }
    if !letter.body.is_empty() {
        return builder
            .header(lettre::message::header::ContentType::TEXT_HTML)
            .body(letter.body.clone())
            .map_err(|err| err.to_string());
    }
    builder
        .header(lettre::message::header::ContentType::TEXT_PLAIN)
        .body(letter.body_text.clone())
        .map_err(|err| err.to_string())
}

fn with_attachments(letter: &Letter) -> Result<lettre::message::MultiPart, String> {
    let mut mixed = if !letter.body.is_empty() && !letter.body_text.is_empty() {
        lettre::message::MultiPart::mixed().multipart(lettre::message::MultiPart::alternative_plain_html(
            letter.body_text.clone(),
            letter.body.clone(),
        ))
    } else if !letter.body.is_empty() {
        lettre::message::MultiPart::mixed().singlepart(lettre::message::SinglePart::html(letter.body.clone()))
    } else {
        lettre::message::MultiPart::mixed().singlepart(lettre::message::SinglePart::plain(letter.body_text.clone()))
    };
    for attachment in &letter.attachments {
        let part = lettre::message::Attachment::new(attachment.filename.clone())
            .body(attachment.bytes.clone(), content_type(&attachment.content_type));
        mixed = mixed.singlepart(part);
    }
    Ok(mixed)
}

fn content_type(name: &str) -> lettre::message::header::ContentType {
    lettre::message::header::ContentType::parse(name).unwrap_or(lettre::message::header::ContentType::TEXT_PLAIN)
}

fn mailbox(address: &str, name: &str) -> Result<lettre::message::Mailbox, String> {
    let address = address.trim();
    let email = address
        .parse()
        .map_err(|err| format!("email address {address} is not valid: {err}"))?;
    let name = name.trim();
    let name = if name.is_empty() { None } else { Some(name.to_string()) };
    Ok(lettre::message::Mailbox::new(name, email))
}

fn new_message_id(from: &str) -> String {
    let mut bytes = [0u8; 12];
    let _ = getrandom::fill(&mut bytes);
    let host = from
        .rsplit_once('@')
        .map(|(_, host)| host.trim())
        .filter(|host| !host.is_empty())
        .unwrap_or("localhost");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("<{hex}@{host}>")
}

thread_local! {
    static SINK: RefCell<Option<Rc<RefCell<Vec<Letter>>>>> = const { RefCell::new(None) };
    static FAKE: RefCell<Option<Vec<Inbound>>> = const { RefCell::new(None) };
}

fn push_sink(letter: &Letter) -> bool {
    let kept = SINK.with(|slot| {
        let Some(sink) = slot.borrow().clone() else {
            return false;
        };
        sink.borrow_mut().push(letter.clone());
        true
    });
    #[cfg(test)]
    if !kept {
        return push_shared_sink(letter);
    }
    kept
}

/// The sink of [`SharedSinkGuard`], for a send on a service worker thread.
#[cfg(test)]
static SHARED_SINK: std::sync::Mutex<Option<Arc<std::sync::Mutex<Vec<Letter>>>>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn push_shared_sink(letter: &Letter) -> bool {
    let shared = SHARED_SINK.lock().unwrap_or_else(|err| err.into_inner()).clone();
    let Some(shared) = shared else {
        return false;
    };
    shared
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .push(letter.clone());
    true
}

/// The test sink: while installed, a send on this thread is kept here instead of going to SMTP.
#[cfg(test)]
pub(crate) struct SinkGuard {
    previous: Option<Rc<RefCell<Vec<Letter>>>>,
    current: Rc<RefCell<Vec<Letter>>>,
}

/// A mail the sink kept, with the MIME text lettre would send.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct SentMail {
    pub subject: String,
    pub to: Vec<String>,
    pub body: String,
    pub body_text: String,
    /// File name, content type and bytes.
    pub attachments: Vec<(String, String, Vec<u8>)>,
    pub mime: String,
}

#[cfg(test)]
impl SinkGuard {
    pub(crate) fn install() -> Self {
        let current = Rc::new(RefCell::new(Vec::new()));
        let previous = SINK.with(|slot| slot.borrow_mut().replace(Rc::clone(&current)));
        Self { previous, current }
    }

    fn messages(&self) -> Vec<Letter> {
        self.current.borrow().clone()
    }

    pub(crate) fn sent(&self) -> Vec<SentMail> {
        self.messages().into_iter().map(sent_mail).collect()
    }
}

#[cfg(test)]
fn sent_mail(letter: Letter) -> SentMail {
    SentMail {
        mime: build_lettre(&letter)
            .map(|message| String::from_utf8_lossy(&message.formatted()).into_owned())
            .unwrap_or_else(|err| format!("not a message: {err}")),
        subject: letter.subject,
        to: letter.to,
        body: letter.body,
        body_text: letter.body_text,
        attachments: letter
            .attachments
            .into_iter()
            .map(|part| (part.filename, part.content_type, part.bytes))
            .collect(),
    }
}

/// A test sink for sends on any thread that has no [`SinkGuard`] of its own, so mail an async
/// service sends from a worker is kept. One is installed at a time; a second waits.
#[cfg(test)]
pub(crate) struct SharedSinkGuard {
    current: Arc<std::sync::Mutex<Vec<Letter>>>,
    _turn: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl SharedSinkGuard {
    pub(crate) fn install() -> Self {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let turn = TURN.lock().unwrap_or_else(|err| err.into_inner());
        let current = Arc::new(std::sync::Mutex::new(Vec::new()));
        *SHARED_SINK.lock().unwrap_or_else(|err| err.into_inner()) = Some(Arc::clone(&current));
        Self { current, _turn: turn }
    }

    /// The mail kept so far to `address`.
    pub(crate) fn sent_to(&self, address: &str) -> Vec<SentMail> {
        let letters = self.current.lock().unwrap_or_else(|err| err.into_inner()).clone();
        letters
            .into_iter()
            .filter(|letter| letter.to.iter().any(|to| to == address))
            .map(sent_mail)
            .collect()
    }

    /// Wait up to `timeout` for mail to `address`, as a worker sends it after the caller returns.
    pub(crate) fn wait_for(&self, address: &str, timeout: Duration) -> Vec<SentMail> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let sent = self.sent_to(address);
            if !sent.is_empty() || std::time::Instant::now() > deadline {
                return sent;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(test)]
impl Drop for SharedSinkGuard {
    fn drop(&mut self) {
        *SHARED_SINK.lock().unwrap_or_else(|err| err.into_inner()) = None;
    }
}

#[cfg(test)]
impl Drop for SinkGuard {
    fn drop(&mut self) {
        SINK.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

fn poll_server(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    if facade.sim_active() {
        return fail("Email poll is disabled in LLM sim session");
    }
    let server_id = param_text(parameters, "emailServerId");
    let Some(server) = find_one(facade, SERVER, "emailServerId", &server_id)? else {
        return fail(format!("No EmailServer found for ID [{server_id}]"));
    };
    let host = field_text(&server, "storeHost");
    let user = field_text(&server, "mailUsername");
    let password = field_text(&server, "mailPassword");
    if host.is_empty() {
        note_error(format!("EmailServer [{server_id}] has no storeHost"));
    }
    if user.is_empty() {
        note_error(format!("EmailServer [{server_id}] has no mailUsername"));
    }
    if password.is_empty() {
        note_error(format!("EmailServer [{server_id}] has no mailPassword"));
    }
    if message_has_error() {
        return Ok(BTreeMap::new());
    }
    if let Err(text) = require_host(&host) {
        return fail(text);
    }
    let protocol = {
        let given = field_text(&server, "storeProtocol");
        if given.is_empty() {
            "imaps".to_string()
        } else {
            given
        }
    };
    let port = match parse_port(&field_text(&server, "storePort"), 993) {
        Ok(port) => port,
        Err(text) => return fail(text),
    };
    let folder = {
        let given = field_text(&server, "storeFolder");
        if given.is_empty() {
            "INBOX".to_string()
        } else {
            given
        }
    };
    let settings = StoreSettings {
        host,
        port,
        user,
        password,
        protocol,
        folder,
        skip_seen: field_text(&server, "storeSkipSeen") == "Y",
        mark_seen: field_text(&server, "storeMarkSeen") == "Y",
        delete: field_text(&server, "storeDelete") == "Y",
    };
    let mut store = match open_store(&settings) {
        Ok(store) => store,
        Err(text) => return fail(text),
    };
    let messages = match store.list() {
        Ok(messages) => messages,
        Err(text) => return fail(text),
    };
    for message in messages {
        if settings.skip_seen && message.seen {
            continue;
        }
        if let Err(err) = super::email_eca::run_email_rules(&eca_fields(&message, &server_id)) {
            note_error(err.message());
        }
        let id = message.id.clone();
        if settings.mark_seen {
            if let Err(err) = store.mark_seen(&id) {
                note_error(err);
            }
        }
        if settings.delete {
            if let Err(err) = store.delete(&id) {
                note_error(err);
            }
        }
    }
    if let Err(err) = store.close() {
        note_error(err);
    }
    Ok(BTreeMap::new())
}

struct StoreSettings {
    host: String,
    port: u16,
    user: String,
    password: String,
    protocol: String,
    folder: String,
    skip_seen: bool,
    mark_seen: bool,
    delete: bool,
}

fn eca_fields(message: &Inbound, server_id: &str) -> BTreeMap<String, Value> {
    let mut fields = IndexMap::new();
    fields.insert("toList".into(), string_list(&message.to));
    fields.insert("ccList".into(), string_list(&message.cc));
    fields.insert("bccList".into(), string_list(&message.bcc));
    fields.insert(
        "from".into(),
        if message.from.is_empty() {
            Value::Null
        } else {
            Value::string(&message.from)
        },
    );
    fields.insert("subject".into(), Value::string(&message.subject));
    fields.insert("sentDate".into(), message.sent.clone());
    fields.insert("receivedDate".into(), message.received.clone());
    let mut flags = IndexMap::new();
    flags.insert("answered".into(), Value::Bool(false));
    flags.insert("deleted".into(), Value::Bool(false));
    flags.insert("draft".into(), Value::Bool(false));
    flags.insert("flagged".into(), Value::Bool(false));
    flags.insert("recent".into(), Value::Bool(false));
    flags.insert("seen".into(), Value::Bool(message.seen));
    let mut out = BTreeMap::new();
    out.insert("emailServerId".into(), Value::string(server_id));
    out.insert("fields".into(), Value::Map(fields));
    out.insert("headers".into(), Value::Map(message.headers.clone()));
    out.insert("bodyPartList".into(), Value::List(message.parts.clone()));
    out.insert("flags".into(), Value::Map(flags));
    out
}

fn string_list(values: &[String]) -> Value {
    Value::List(values.iter().cloned().map(Value::string).collect())
}

#[derive(Clone, Debug)]
struct Inbound {
    id: String,
    seen: bool,
    headers: IndexMap<String, Value>,
    parts: Vec<Value>,
    subject: String,
    from: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    sent: Value,
    received: Value,
}

impl Inbound {
    fn blank() -> Self {
        Self {
            id: String::new(),
            seen: false,
            headers: IndexMap::new(),
            parts: Vec::new(),
            subject: String::new(),
            from: String::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            sent: Value::Null,
            received: Value::Null,
        }
    }
}

trait Mailbox {
    fn list(&mut self) -> Result<Vec<Inbound>, String>;
    fn mark_seen(&mut self, id: &str) -> Result<(), String>;
    fn delete(&mut self, id: &str) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
}

struct FakeBox {
    messages: Vec<Inbound>,
}

impl Mailbox for FakeBox {
    fn list(&mut self) -> Result<Vec<Inbound>, String> {
        Ok(self.messages.clone())
    }

    fn mark_seen(&mut self, id: &str) -> Result<(), String> {
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            message.seen = true;
        }
        Ok(())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.messages.retain(|message| message.id != id);
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
struct FakeGuard {
    previous: Option<Vec<Inbound>>,
}

#[cfg(test)]
impl FakeGuard {
    fn install(messages: Vec<Inbound>) -> Self {
        let previous = FAKE.with(|slot| slot.borrow_mut().replace(messages));
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for FakeGuard {
    fn drop(&mut self) {
        FAKE.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}

fn open_store(settings: &StoreSettings) -> Result<Box<dyn Mailbox>, String> {
    if let Some(messages) = FAKE.with(|slot| slot.borrow().clone()) {
        return Ok(Box::new(FakeBox { messages }));
    }
    match settings.protocol.as_str() {
        "pop3" | "pop3s" => Ok(Box::new(Pop3::open(settings)?)),
        "imap" | "imaps" => Ok(Box::new(ImapBox::open(settings)?)),
        other => Err(format!("storeProtocol {other} is not supported")),
    }
}

fn use_tls(protocol: &str, port: u16) -> bool {
    matches!(protocol, "imaps" | "pop3s") || port == 993 || port == 995
}

struct ImapBox {
    session: imap::Session<Net>,
    closed: bool,
    deleted: bool,
    skip_seen: bool,
}

impl ImapBox {
    fn open(settings: &StoreSettings) -> Result<Self, String> {
        let net = connect_net(
            &settings.host,
            settings.port,
            use_tls(&settings.protocol, settings.port),
        )?;
        let mut client = imap::Client::new(net);
        client.read_greeting().map_err(|err| err.to_string())?;
        let mut session = client
            .login(&settings.user, &settings.password)
            .map_err(|(err, _)| err.to_string())?;
        session
            .select(&settings.folder)
            .map_err(|_| format!("No {} folder found", settings.folder))?;
        Ok(Self {
            session,
            closed: false,
            deleted: false,
            skip_seen: settings.skip_seen,
        })
    }
}

impl Mailbox for ImapBox {
    fn list(&mut self) -> Result<Vec<Inbound>, String> {
        let query = if self.skip_seen {
            "UNSEEN NOT DELETED"
        } else {
            "NOT DELETED"
        };
        let ids = self.session.search(query).map_err(|err| err.to_string())?;
        let mut messages = Vec::new();
        for id in ids {
            let fetches = self
                .session
                .fetch(id.to_string(), "BODY.PEEK[] FLAGS INTERNALDATE")
                .map_err(|err| err.to_string())?;
            for fetch in fetches.iter() {
                let mut message = parse_rfc822(fetch.body().unwrap_or_default());
                message.id = fetch.message.to_string();
                message.seen = fetch.flags().iter().any(|flag| matches!(flag, imap::types::Flag::Seen));
                if let Some(date) = fetch.internal_date() {
                    message.received = stamp_millis(date.timestamp_millis());
                }
                messages.push(message);
            }
        }
        Ok(messages)
    }

    fn mark_seen(&mut self, id: &str) -> Result<(), String> {
        self.session
            .store(id, "+FLAGS (\\Seen)")
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.session
            .store(id, "+FLAGS (\\Deleted)")
            .map(|_| ())
            .map_err(|err| err.to_string())?;
        self.deleted = true;
        Ok(())
    }

    fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if self.deleted {
            self.session.expunge().map_err(|err| err.to_string())?;
        }
        self.session.logout().map_err(|err| err.to_string())
    }
}

impl Drop for ImapBox {
    fn drop(&mut self) {
        if !self.closed {
            if self.deleted {
                let _ = self.session.expunge();
            }
            let _ = self.session.logout();
            self.closed = true;
        }
    }
}

struct Pop3 {
    reader: BufReader<Net>,
    closed: bool,
}

impl Pop3 {
    fn open(settings: &StoreSettings) -> Result<Self, String> {
        if settings.folder != "INBOX" {
            return Err(format!("No {} folder found", settings.folder));
        }
        let net = connect_net(
            &settings.host,
            settings.port,
            use_tls(&settings.protocol, settings.port),
        )?;
        let mut reader = BufReader::new(net);
        let greeting = read_line(&mut reader)?;
        expect_ok(&greeting)?;
        let mut client = Self { reader, closed: false };
        client.command(&format!("USER {}", settings.user))?;
        client.command(&format!("PASS {}", settings.password))?;
        Ok(client)
    }

    fn command(&mut self, line: &str) -> Result<String, String> {
        writeln!(self.reader.get_mut(), "{line}").map_err(|err| err.to_string())?;
        self.reader.get_mut().flush().map_err(|err| err.to_string())?;
        let response = read_line(&mut self.reader)?;
        expect_ok(&response)?;
        Ok(response)
    }

    fn read_dot(&mut self) -> Result<Vec<u8>, String> {
        let mut raw = Vec::new();
        loop {
            let line = read_line(&mut self.reader)?;
            if line == "." {
                break;
            }
            let line = match line.strip_prefix("..") {
                Some(rest) => format!(".{rest}"),
                None => line,
            };
            raw.extend(line.as_bytes());
            raw.extend(b"\r\n");
        }
        Ok(raw)
    }
}

impl Mailbox for Pop3 {
    fn list(&mut self) -> Result<Vec<Inbound>, String> {
        self.command("LIST")?;
        let listing = self.read_dot()?;
        let text = String::from_utf8_lossy(&listing);
        let mut messages = Vec::new();
        for line in text.lines() {
            let Some(id) = line.split_whitespace().next() else {
                continue;
            };
            if id.parse::<u32>().is_err() {
                continue;
            }
            self.command(&format!("RETR {id}"))?;
            let bytes = self.read_dot()?;
            let mut message = parse_rfc822(&bytes);
            message.id = id.to_string();
            message.seen = false;
            messages.push(message);
        }
        Ok(messages)
    }

    fn mark_seen(&mut self, _id: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete(&mut self, id: &str) -> Result<(), String> {
        self.command(&format!("DELE {id}")).map(|_| ())
    }

    fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let _ = writeln!(self.reader.get_mut(), "QUIT");
        let _ = self.reader.get_mut().flush();
        Ok(())
    }
}

impl Drop for Pop3 {
    fn drop(&mut self) {
        if !self.closed {
            let _ = writeln!(self.reader.get_mut(), "QUIT");
            let _ = self.reader.get_mut().flush();
            self.closed = true;
        }
    }
}

enum Net {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Net {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Net::Plain(stream) => stream.read(buf),
            Net::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Net {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Net::Plain(stream) => stream.write(buf),
            Net::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Net::Plain(stream) => stream.flush(),
            Net::Tls(stream) => stream.flush(),
        }
    }
}

fn connect_net(host: &str, port: u16, tls: bool) -> Result<Net, String> {
    let tcp = TcpStream::connect((host, port)).map_err(|err| format!("could not connect to {host}:{port}: {err}"))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(60)));
    if !tls {
        return Ok(Net::Plain(tcp));
    }
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| format!("email host {host} is not a DNS name"))?;
    let config = tls_config()?;
    let conn = rustls::ClientConnection::new(config, name).map_err(|err| err.to_string())?;
    Ok(Net::Tls(Box::new(rustls::StreamOwned::new(conn, tcp))))
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| err.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn read_line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(|err| err.to_string())?;
    if read == 0 {
        return Err("mail server closed the connection".to_string());
    }
    if line.ends_with('\n') {
        line.pop();
    }
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

fn expect_ok(line: &str) -> Result<(), String> {
    if line.starts_with("+OK") {
        Ok(())
    } else {
        Err(format!("mail server rejected the command: {line}"))
    }
}

fn parse_rfc822(bytes: &[u8]) -> Inbound {
    let text = String::from_utf8_lossy(bytes);
    let (header_text, body) = split_header_body(&text);
    let mut headers = IndexMap::new();
    let mut name = String::new();
    let mut value = String::new();
    for line in header_text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            value.push(' ');
            value.push_str(line.trim());
            continue;
        }
        if !name.is_empty() {
            push_header(&mut headers, &name, std::mem::take(&mut value));
        }
        match line.split_once(':') {
            Some((left, right)) => {
                name = left.trim().to_string();
                value = right.trim().to_string();
            }
            None => {
                name.clear();
                value.clear();
            }
        }
    }
    if !name.is_empty() {
        push_header(&mut headers, &name, value);
    }
    let mut message = Inbound::blank();
    message.subject = header_one(&headers, "subject");
    message.from = header_addresses(&headers, "from")
        .into_iter()
        .next()
        .unwrap_or_default();
    message.to = header_addresses(&headers, "to");
    message.cc = header_addresses(&headers, "cc");
    message.bcc = header_addresses(&headers, "bcc");
    message.sent = date_header(&headers, "date");
    message.parts = body_parts(&headers, body.as_bytes());
    message.headers = headers;
    message
}

fn split_header_body(text: &str) -> (&str, &str) {
    if let Some(index) = text.find("\r\n\r\n") {
        return (&text[..index], &text[index + 4..]);
    }
    if let Some(index) = text.find("\n\n") {
        return (&text[..index], &text[index + 2..]);
    }
    (text, "")
}

fn push_header(headers: &mut IndexMap<String, Value>, name: &str, value: String) {
    let key = name.to_ascii_lowercase();
    if let Some(existing) = headers.shift_remove(&key) {
        let list = match existing {
            Value::List(mut items) => {
                items.push(Value::string(value));
                items
            }
            other => vec![other, Value::string(value)],
        };
        headers.insert(key, Value::List(list));
    } else {
        headers.insert(key, Value::string(value));
    }
}

fn header_one(headers: &IndexMap<String, Value>, name: &str) -> String {
    match headers.get(name) {
        Some(Value::List(items)) => items.first().map(to_plain_string).unwrap_or_default(),
        Some(value) => to_plain_string(value),
        None => String::new(),
    }
}

fn header_addresses(headers: &IndexMap<String, Value>, name: &str) -> Vec<String> {
    match headers.get(name) {
        Some(Value::List(items)) => items
            .iter()
            .flat_map(|item| split_addresses(&to_plain_string(item)))
            .collect(),
        Some(value) => split_addresses(&to_plain_string(value)),
        None => Vec::new(),
    }
}

fn date_header(headers: &IndexMap<String, Value>, name: &str) -> Value {
    let text = header_one(headers, name);
    if text.is_empty() {
        return Value::Null;
    }
    match jiff::fmt::rfc2822::parse(text.trim()) {
        Ok(zoned) => stamp_millis(zoned.timestamp().as_millisecond()),
        Err(_) => Value::Null,
    }
}

fn body_parts(headers: &IndexMap<String, Value>, body: &[u8]) -> Vec<Value> {
    let content_type = header_one(headers, "content-type");
    let disposition = header_one(headers, "content-disposition");
    if content_type.to_ascii_lowercase().starts_with("multipart/") {
        if let Some(boundary) = header_param(&content_type, "boundary") {
            return multipart_parts(&boundary, body);
        }
    }
    vec![one_part(
        &content_type,
        &disposition,
        &transfer_encoding(headers, &content_type, &disposition),
        body,
    )]
}

fn multipart_parts(boundary: &str, body: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(body);
    let marker = format!("--{boundary}");
    let mut parts = Vec::new();
    for chunk in text.split(&marker).skip(1) {
        let chunk = chunk.trim_start_matches(['\r', '\n']);
        if chunk.starts_with("--") {
            break;
        }
        let (header_text, part_body) = split_header_body(chunk);
        let mut headers = IndexMap::new();
        for line in header_text.lines() {
            if let Some((name, value)) = line.split_once(':') {
                push_header(&mut headers, name.trim(), value.trim().to_string());
            }
        }
        let content_type = header_one(&headers, "content-type");
        let disposition = header_one(&headers, "content-disposition");
        let encoding = transfer_encoding(&headers, &content_type, &disposition);
        parts.push(one_part(&content_type, &disposition, &encoding, part_body.as_bytes()));
    }
    parts
}

fn transfer_encoding(headers: &IndexMap<String, Value>, content_type: &str, disposition: &str) -> String {
    let named = header_one(headers, "content-transfer-encoding");
    if named.is_empty() {
        encoding_of(content_type, disposition)
    } else {
        named
    }
}

fn one_part(content_type: &str, disposition: &str, encoding: &str, body: &[u8]) -> Value {
    let bytes = decode_transfer(body, encoding);
    let filename = header_param(disposition, "filename")
        .or_else(|| header_param(content_type, "name"))
        .unwrap_or_default();
    let kind = disposition.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let mut map = IndexMap::new();
    if !content_type.is_empty() {
        map.insert("contentType".into(), Value::string(content_type));
    }
    if !filename.is_empty() {
        map.insert("filename".into(), Value::string(filename));
    }
    if !kind.is_empty() {
        map.insert("disposition".into(), Value::string(kind));
    }
    let textual = content_type.is_empty() || content_type.to_ascii_lowercase().starts_with("text/");
    if textual {
        map.insert(
            "contentText".into(),
            Value::string(String::from_utf8_lossy(&bytes).to_string()),
        );
    } else {
        map.insert("contentBytes".into(), Value::Bytes(bytes));
    }
    Value::Map(map)
}

fn encoding_of(content_type: &str, disposition: &str) -> String {
    header_param(content_type, "encoding")
        .or_else(|| header_param(disposition, "encoding"))
        .unwrap_or_default()
}

fn decode_transfer(bytes: &[u8], encoding: &str) -> Vec<u8> {
    match encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => {
            let text: String = String::from_utf8_lossy(bytes)
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text).unwrap_or_else(|_| bytes.to_vec())
        }
        "quoted-printable" => decode_qp(bytes),
        _ => bytes.to_vec(),
    }
}

fn decode_qp(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'=' && index + 1 < bytes.len() && (bytes[index + 1] == b'\n' || bytes[index + 1] == b'\r') {
            index += 1;
            if bytes[index] == b'\r' {
                index += 1;
            }
            if index < bytes.len() && bytes[index] == b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'=' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""), 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn header_param(header: &str, name: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let key = format!("{name}=");
    let index = lower.find(&key)?;
    let rest = header[index + key.len()..].trim();
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"').unwrap_or(quoted.len());
        return Some(quoted[..end].to_string());
    }
    Some(rest.split([';', ' ', '\t']).next().unwrap_or("").trim().to_string())
}

fn find_one(facade: &EntityFacade, entity: &str, field: &str, value: &str) -> Result<Option<EntityValue>, LibError> {
    if value.is_empty() {
        return Ok(None);
    }
    facade.find(entity)?.eq(field, Value::string(value)).one()
}

fn field_text(row: &EntityValue, name: &str) -> String {
    match row.get(name) {
        Some(value) if !is_empty(value) => to_plain_string(value),
        _ => String::new(),
    }
}

fn param_text(parameters: &BTreeMap<String, Value>, name: &str) -> String {
    match parameters.get(name) {
        Some(value) if !is_empty(value) => to_plain_string(value),
        _ => String::new(),
    }
}

fn map_text(map: &IndexMap<String, Value>, name: &str) -> String {
    match map.get(name) {
        Some(value) if !is_empty(value) => to_plain_string(value),
        _ => String::new(),
    }
}

fn body_parameters(parameters: &BTreeMap<String, Value>) -> Option<&IndexMap<String, Value>> {
    match parameters.get("bodyParameters") {
        Some(Value::Map(map)) => Some(map),
        _ => None,
    }
}

fn set_text(row: &mut EntityValue, name: &str, value: &str) {
    if !value.is_empty() {
        row.set(name, Value::string(value));
    }
}

fn merge_lists(given: &str, template: &str) -> String {
    if !given.is_empty() {
        if template.is_empty() {
            given.to_string()
        } else {
            format!("{given},{template}")
        }
    } else {
        template.to_string()
    }
}

fn split_addresses(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut angle = 0i32;
    let mut quote = false;
    for ch in value.chars() {
        match ch {
            '"' => quote = !quote,
            '<' if !quote => angle += 1,
            '>' if !quote && angle > 0 => angle -= 1,
            ',' if !quote && angle == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    out.push(trimmed);
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    out
}

fn parse_port(text: &str, default: u16) -> Result<u16, String> {
    if text.trim().is_empty() {
        return Ok(default);
    }
    text.trim().parse().map_err(|_| format!("port {text} is not a number"))
}

fn ids(email_message_id: &str, message_id: &str) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if !email_message_id.is_empty() {
        out.insert("emailMessageId".into(), Value::string(email_message_id));
    }
    if !message_id.is_empty() {
        out.insert("messageId".into(), Value::string(message_id));
    }
    out
}

fn fail(text: impl Into<String>) -> Result<BTreeMap<String, Value>, LibError> {
    note_error(text);
    Ok(BTreeMap::new())
}

fn fail_with(text: impl Into<String>, email_message_id: &str) -> Result<BTreeMap<String, Value>, LibError> {
    note_error(text);
    Ok(ids(email_message_id, ""))
}

fn note_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

fn note_message(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_message(text));
}

fn message_has_error() -> bool {
    ExecutionContext::with_current(|ctx| !ctx.message().errors().is_empty()).unwrap_or(false)
}

fn now_stamp() -> Value {
    basic_convert(
        &Value::I64(jiff::Timestamp::now().as_millisecond()),
        "java.sql.Timestamp",
    )
    .value
}

fn stamp_millis(millis: i64) -> Value {
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use indexmap::IndexMap;

    use super::{host_allowed, FakeGuard, Inbound, SinkGuard};
    use crate::context::ExecutionContextFactory;
    use crate::entity::EntityFacade;
    use crate::screen::PDF_UNAVAILABLE;
    use crate::service::eca_rules;
    use moqui_lib::object::Value;

    /// The tests set `email_allowed_hosts`, a process property, so they share the property lock.
    fn lock_tests() -> std::sync::MutexGuard<'static, ()> {
        crate::user::session::cookie_property_lock()
    }

    fn fixture(name: &str) -> String {
        format!("{}/src/service/testdata/mail/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    fn smtp_server(facade: &EntityFacade, context: &crate::context::ExecutionContext, id: &str) {
        seed(
            facade,
            context,
            "moqui.basic.email.EmailServer",
            &[
                ("emailServerId", id),
                ("smtpHost", "smtp.example.com"),
                ("smtpPort", "25"),
            ],
        );
    }

    /// `send#ScreenRenderEmail` requires a user, as in Java.
    fn log_in(facade: &EntityFacade, context: &crate::context::ExecutionContext) {
        seed(
            facade,
            context,
            "moqui.security.UserAccount",
            &[
                ("userId", "EMAILER"),
                ("username", "emailer"),
                ("emailAddress", "emailer@example.com"),
            ],
        );
        crate::user::internal_login_user("emailer").expect("login");
    }

    fn map(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect::<IndexMap<_, _>>(),
        )
    }

    struct HostList;

    impl HostList {
        fn set(value: &'static str) -> Self {
            moqui_lib::env::set_property("email_allowed_hosts", value);
            Self
        }
    }

    impl Drop for HostList {
        fn drop(&mut self) {
            moqui_lib::env::clear_property("email_allowed_hosts");
        }
    }

    fn entered() -> (EntityFacade, crate::context::ExecutionContext) {
        let facade = crate::testing::open();
        let context = ExecutionContextFactory::test_context_in(
            "",
            crate::service::ServicePools::idle(),
            crate::context::framework_dir(),
        );
        context.factory().set_entity(facade.clone());
        (facade, context)
    }

    fn seed(facade: &EntityFacade, context: &crate::context::ExecutionContext, entity: &str, fields: &[(&str, &str)]) {
        let _authz = context.disable_authz();
        let mut row = facade.make_value(entity).expect("make");
        for (name, value) in fields {
            row.set(*name, Value::string(*value));
        }
        row.create(facade).expect("create");
    }

    #[test]
    fn host_suffix_matches_on_a_dot_boundary() {
        assert!(host_allowed("smtp.example.com") || !moqui_lib::env::get_prop_or_env("email_allowed_hosts").is_empty());
        let _lock = lock_tests();
        let _list = HostList::set("example.com");
        assert!(host_allowed("smtp.example.com"));
        assert!(host_allowed("example.com"));
        assert!(!host_allowed("notexample.com"));
    }

    #[test]
    fn sink_receives_a_template_send_and_expands_the_subject() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailServer",
            &[
                ("emailServerId", "TEST_SMTP"),
                ("smtpHost", "smtp.example.com"),
                ("smtpPort", "25"),
            ],
        );
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailTemplate",
            &[
                ("emailTemplateId", "HELLO"),
                ("emailServerId", "TEST_SMTP"),
                ("fromAddress", "from@example.com"),
                ("subject", "Hello ${title}"),
            ],
        );
        let sink = SinkGuard::install();
        let mut body_parameters = indexmap::IndexMap::new();
        body_parameters.insert("title".into(), Value::string("there"));
        let mut params = std::collections::BTreeMap::new();
        params.insert("emailTemplateId".into(), Value::string("HELLO"));
        params.insert("toAddresses".into(), Value::string("to@example.com"));
        params.insert("body".into(), Value::string("Hi"));
        params.insert("bodyText".into(), Value::string("Hi"));
        params.insert("bodyParameters".into(), Value::Map(body_parameters));
        let out = crate::service::sync("org.moqui.impl.EmailServices.send#EmailTemplate", &params).expect("send");
        let errors = context.message().errors().join("\n");
        assert!(errors.is_empty(), "{errors}");
        let sent = sink.messages();
        assert_eq!(sent.len(), 1, "{errors}");
        assert_eq!(sent[0].subject, "Hello there");
        assert!(out.contains_key("emailMessageId"));
    }

    #[test]
    fn host_rejected_by_email_allowed_hosts_does_not_touch_the_sink() {
        let _lock = lock_tests();
        let _list = HostList::set("example.com");
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailServer",
            &[("emailServerId", "TEST_BAD"), ("smtpHost", "notexample.com")],
        );
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailTemplate",
            &[
                ("emailTemplateId", "BADHOST"),
                ("emailServerId", "TEST_BAD"),
                ("fromAddress", "from@example.com"),
                ("subject", "No"),
            ],
        );
        let sink = SinkGuard::install();
        let mut params = std::collections::BTreeMap::new();
        params.insert("emailTemplateId".into(), Value::string("BADHOST"));
        params.insert("toAddresses".into(), Value::string("to@example.com"));
        params.insert("body".into(), Value::string("Hi"));
        params.insert("bodyText".into(), Value::string("Hi"));
        let _ = crate::service::sync("org.moqui.impl.EmailServices.send#EmailTemplate", &params);
        let errors = context.message().errors().join("\n");
        assert!(errors.contains("notexample.com"), "{errors}");
        assert!(sink.messages().is_empty(), "{errors}");
    }

    /// Gate 4: a screen body renders in html and text and goes out as multipart/alternative, with
    /// screen attachments (forEachIn, attachmentCondition, csv/text/html), a plain file, and an xsl-fo
    /// attachment that is skipped because PDF waits for phase 12.
    #[test]
    fn screen_body_renders_html_and_text_and_sends_with_attachments() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        smtp_server(&facade, &context, "GATE_SMTP");
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailTemplate",
            &[
                ("emailTemplateId", "GATE_BODY"),
                ("emailServerId", "GATE_SMTP"),
                ("fromAddress", "from@example.com"),
                ("subject", "Report for ${name}"),
                ("bodyScreenLocation", &fixture("MailBody.xml")),
            ],
        );
        let report = fixture("MailReport.xml");
        let attachments: [&[(&str, &str)]; 5] = [
            &[
                ("fileName", "report-${rowName}.csv"),
                ("attachmentLocation", &report),
                ("screenRenderMode", "csv"),
                ("forEachIn", "reportRows"),
            ],
            &[
                ("fileName", "report.txt"),
                ("attachmentLocation", &report),
                ("screenRenderMode", "text"),
                ("attachmentCondition", "includeText"),
            ],
            &[
                ("fileName", "report.xml"),
                ("attachmentLocation", &report),
                ("screenRenderMode", "xml"),
                ("attachmentCondition", "!includeText"),
            ],
            &[
                ("fileName", "report.pdf"),
                ("attachmentLocation", &report),
                ("screenRenderMode", "xsl-fo"),
            ],
            &[("fileName", "plain.txt"), ("attachmentLocation", &fixture("plain.txt"))],
        ];
        for fields in attachments {
            let mut row = vec![("emailTemplateId", "GATE_BODY")];
            row.extend_from_slice(fields);
            seed(&facade, &context, "moqui.basic.email.EmailTemplateAttachment", &row);
        }
        let sink = SinkGuard::install();
        let rows = Value::List(vec![
            map(vec![("rowName", Value::string("one"))]),
            map(vec![("rowName", Value::string("two"))]),
        ]);
        let mut params = BTreeMap::new();
        params.insert("emailTemplateId".into(), Value::string("GATE_BODY"));
        params.insert("toAddresses".into(), Value::string("to@example.com"));
        params.insert(
            "bodyParameters".into(),
            map(vec![
                ("name", Value::string("Ada")),
                ("includeText", Value::Bool(true)),
                ("reportRows", rows),
            ]),
        );
        params.insert(
            "attachments".into(),
            Value::List(vec![map(vec![
                ("fileName", Value::string("inline.html")),
                ("attachmentLocation", Value::string(&report)),
                ("screenRenderMode", Value::string("html")),
            ])]),
        );
        let out = crate::service::sync("org.moqui.impl.EmailServices.send#EmailTemplate", &params).expect("send");
        let errors = context.message().errors().join("\n");
        let messages = context.message().messages().join("\n");
        assert!(errors.is_empty(), "{errors}");
        assert!(!messages.contains("Error sending email"), "{messages}");
        let sent = sink.sent();
        assert_eq!(sent.len(), 1, "{messages}");
        let mail = &sent[0];
        assert_eq!(mail.subject, "Report for Ada");
        let email_message_id = out
            .get("emailMessageId")
            .and_then(Value::as_str)
            .expect("emailMessageId");
        assert!(
            mail.body.contains(r#"<p class="greeting">Hello Ada</p>"#),
            "{}",
            mail.body
        );
        assert!(mail.body.contains("Report for Ada via webroot"), "{}", mail.body);
        assert!(
            mail.body.contains(&format!("message {email_message_id}")),
            "{}",
            mail.body
        );
        assert!(
            mail.body_text.contains("Hello Ada\nSubject: Report for Ada"),
            "{}",
            mail.body_text
        );
        assert!(!mail.body_text.contains('<'), "{}", mail.body_text);

        let names: Vec<&str> = mail.attachments.iter().map(|(name, _, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "inline.html",
                "report-one.csv",
                "report-two.csv",
                "report.txt",
                "plain.txt"
            ]
        );
        let part = |name: &str| mail.attachments.iter().find(|(file, _, _)| file == name).unwrap();
        assert_eq!(part("inline.html").1, "text/html");
        assert!(String::from_utf8_lossy(&part("inline.html").2).contains("<p>Report all</p>"));
        assert_eq!(part("report-one.csv").1, "text/csv");
        assert_eq!(
            String::from_utf8_lossy(&part("report-one.csv").2).trim(),
            "name,greeting\none,Ada"
        );
        assert!(String::from_utf8_lossy(&part("report-two.csv").2).contains("two,Ada"));
        assert_eq!(part("report.txt").1, "text/plain");
        assert_eq!(String::from_utf8_lossy(&part("report.txt").2).trim(), "Report all");
        assert_eq!(String::from_utf8_lossy(&part("plain.txt").2), "plain attachment text\n");

        assert!(mail.mime.contains("multipart/mixed"), "{}", mail.mime);
        assert!(mail.mime.contains("multipart/alternative"), "{}", mail.mime);
        assert!(mail.mime.contains("Content-Type: text/plain"), "{}", mail.mime);
        assert!(mail.mime.contains("Content-Type: text/html"), "{}", mail.mime);
        assert!(mail.mime.contains("report-one.csv"), "{}", mail.mime);
        assert!(!mail.mime.contains("report.pdf"), "{}", mail.mime);

        let _authz = context.disable_authz();
        let row = facade
            .find("moqui.basic.email.EmailMessage")
            .unwrap()
            .eq("emailMessageId", Value::string(email_message_id))
            .one()
            .unwrap()
            .expect("EmailMessage row");
        assert_eq!(row.get("statusId").and_then(Value::as_str), Some("ES_SENT"));
        assert!(row
            .get("bodyText")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("Hello Ada"));
        assert!(row
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("Hello Ada"));
    }

    #[test]
    fn screen_render_email_sends_the_rendered_screen_as_an_attachment() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        smtp_server(&facade, &context, "SYSTEM");
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailTemplate",
            &[
                ("emailTemplateId", "SCREEN_RENDER"),
                ("emailServerId", "SYSTEM"),
                ("emailTypeEnumId", "EMT_SCREEN_RENDER"),
                ("bodyScreenLocation", "classpath://screen/ScreenRenderEmail.xml"),
                ("webappName", "webroot"),
                ("fromAddress", "reports@example.com"),
                ("subject", "${title}"),
            ],
        );
        log_in(&facade, &context);
        let sink = SinkGuard::install();
        let mut params = BTreeMap::new();
        params.insert("screenLocation".into(), Value::string(fixture("MailReport.xml")));
        params.insert("screenRenderMode".into(), Value::string("csv"));
        params.insert("toAddresses".into(), Value::string("boss@example.com"));
        params.insert("title".into(), Value::string("Monthly Report"));
        params.insert("bodyParameters".into(), map(vec![("rowName", Value::string("March"))]));
        crate::service::sync("org.moqui.impl.EmailServices.send#ScreenRenderEmail", &params).expect("send");
        let errors = context.message().errors().join("\n");
        let messages = context.message().messages().join("\n");
        assert!(errors.is_empty(), "{errors}");
        let sent = sink.sent();
        assert_eq!(sent.len(), 1, "{messages}");
        let mail = &sent[0];
        assert_eq!(mail.subject, "Monthly Report");
        assert!(mail.body.contains("<h2>Monthly Report</h2>"), "{}", mail.body);
        assert!(mail.body.contains("<strong>rowName</strong>: March"), "{}", mail.body);
        assert!(!mail.body.contains("lastStandalone"), "{}", mail.body);
        assert!(mail.body_text.contains("Monthly Report"), "{}", mail.body_text);
        assert!(mail.body_text.contains("rowName: March"), "{}", mail.body_text);
        assert_eq!(mail.attachments.len(), 1);
        let (name, content_type, bytes) = &mail.attachments[0];
        assert_eq!(name, "MailReport.csv");
        assert_eq!(content_type, "text/csv");
        assert!(String::from_utf8_lossy(bytes).contains("March"));
        assert!(mail.mime.contains("multipart/alternative"), "{}", mail.mime);
    }

    #[test]
    fn screen_render_email_as_xsl_fo_sends_without_the_pdf() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        smtp_server(&facade, &context, "SYSTEM");
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailTemplate",
            &[
                ("emailTemplateId", "SCREEN_RENDER"),
                ("emailServerId", "SYSTEM"),
                ("bodyScreenLocation", "classpath://screen/ScreenRenderEmail.xml"),
                ("fromAddress", "reports@example.com"),
                ("subject", "${title}"),
            ],
        );
        log_in(&facade, &context);
        let sink = SinkGuard::install();
        let mut params = BTreeMap::new();
        params.insert("screenLocation".into(), Value::string(fixture("MailReport.xml")));
        params.insert("screenRenderMode".into(), Value::string("xsl-fo"));
        params.insert("toAddresses".into(), Value::string("boss@example.com"));
        params.insert("title".into(), Value::string("Printed"));
        crate::service::sync("org.moqui.impl.EmailServices.send#ScreenRenderEmail", &params).expect("send");
        let errors = context.message().errors().join("\n");
        assert!(errors.is_empty(), "{errors}");
        assert!(!errors.contains(PDF_UNAVAILABLE), "{errors}");
        let sent = sink.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].attachments.is_empty());
        assert!(sent[0].body.contains("Printed"), "{}", sent[0].body);
    }

    #[test]
    fn a_body_screen_that_fails_to_render_does_not_send() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        smtp_server(&facade, &context, "BROKEN_SMTP");
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailTemplate",
            &[
                ("emailTemplateId", "BROKEN"),
                ("emailServerId", "BROKEN_SMTP"),
                ("fromAddress", "from@example.com"),
                ("subject", "Broken"),
                ("bodyScreenLocation", &fixture("NoSuchScreen.xml")),
            ],
        );
        let sink = SinkGuard::install();
        let mut params = BTreeMap::new();
        params.insert("emailTemplateId".into(), Value::string("BROKEN"));
        params.insert("toAddresses".into(), Value::string("to@example.com"));
        let _ = crate::service::sync("org.moqui.impl.EmailServices.send#EmailTemplate", &params);
        let messages = context.message().messages().join("\n");
        assert!(messages.contains("Error sending email"), "{messages}");
        assert!(messages.contains("NoSuchScreen.xml"), "{messages}");
        assert!(sink.sent().is_empty());
    }

    #[test]
    fn sim_skip_returns_email_message_id_sim() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        facade.set_sim_active(true);
        let sink = SinkGuard::install();
        let mut params = std::collections::BTreeMap::new();
        params.insert("emailTemplateId".into(), Value::string("MISSING"));
        params.insert("toAddresses".into(), Value::string("to@example.com"));
        let out = crate::service::sync("org.moqui.impl.EmailServices.send#EmailTemplate", &params).expect("sim");
        assert_eq!(out.get("emailMessageId").and_then(Value::as_str), Some("SIM"));
        assert!(sink.messages().is_empty());
        assert!(context.message().errors().is_empty());
    }

    #[test]
    fn fake_store_poll_runs_one_eca_and_skips_seen() {
        let _lock = lock_tests();
        let (facade, context) = entered();
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        eca_rules::install(
            context.factory(),
            eca_rules::from_xml(
                "",
                r#"<emecas>
                <emeca rule-name="Notice">
                    <condition><expression>true</expression></condition>
                    <actions><script>ec.message.addMessage(fields.subject);</script></actions>
                </emeca>
            </emecas>"#,
            ),
        );
        seed(
            &facade,
            &context,
            "moqui.basic.email.EmailServer",
            &[
                ("emailServerId", "TEST_POLL"),
                ("storeHost", "imap.example.com"),
                ("mailUsername", "inbox"),
                ("mailPassword", "secret"),
                ("storeProtocol", "imap"),
                ("storeSkipSeen", "Y"),
            ],
        );
        let mut fresh = Inbound::blank();
        fresh.id = "1".to_string();
        fresh.subject = "new mail".to_string();
        fresh.seen = false;
        let mut seen = Inbound::blank();
        seen.id = "2".to_string();
        seen.subject = "old mail".to_string();
        seen.seen = true;
        let _store = FakeGuard::install(vec![fresh, seen]);
        let mut params = std::collections::BTreeMap::new();
        params.insert("emailServerId".into(), Value::string("TEST_POLL"));
        crate::service::sync("org.moqui.impl.EmailServices.poll#EmailServer", &params).expect("poll");
        let errors = context.message().errors().join("\n");
        assert!(errors.is_empty(), "{errors}");
        let messages = context.message().messages().to_vec();
        assert!(messages.iter().any(|text| text == "new mail"), "{messages:?}");
        assert!(!messages.iter().any(|text| text == "old mail"), "{messages:?}");
    }
}
