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

//! `export#EmailMessage`: a stored message as an RFC 5322 file (`.eml`) to download, with its `EmailMessageAttachment`
//! files. The mail client keeps no raw message, so the MIME is made again by the send path's builder
//! ([`build_lettre`]) from the stored fields, as the copy of sent mail the sync appends to Sent is: the From, To, Cc,
//! Reply-To of its identity, Subject, Date, Message-ID, In-Reply-To and References; the text and HTML bodies as
//! alternatives; inline files by Content-ID beside the HTML and the others attached. Bcc is not written (lettre leaves it
//! out of the message, as a sent copy has it).

use std::collections::BTreeMap;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use super::implementation::{
    build_lettre, client_headers, find_one, new_message_id, note_error, split_addresses, stored_attachments, Letter,
};
use super::store::MESSAGE;
use super::sync::{value_millis, value_number};
use crate::context::ExecutionContext;
use crate::entity::{EntityFacade, EntityValue};
use crate::service::util::{field_text, has_error, param_text};

const SERVICES: &str = "org.moqui.impl.EmailClientServices";

/// `export#EmailMessage`: `mimeBytes` (and the same as `mimeText`, which is 7-bit text), `fileName` (the subject made safe,
/// with `.eml`) and `contentType` (`message/rfc822`). A message the user may not read (level 1 on its account) is "not
/// found"; one with no mail client account only with authorization disabled. A file that cannot be read is an error, so no
/// export leaves out an attachment without saying so.
pub(super) fn export_message(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let mut out = BTreeMap::new();
    let email_message_id = param_text(parameters, "emailMessageId");
    let not_found = || note_error(format!("Email message {email_message_id} not found"));
    let Some(message) = find_one(facade, MESSAGE, "emailMessageId", &email_message_id)? else {
        not_found();
        return Ok(out);
    };
    if !may_read(&message)? {
        if !has_error() {
            not_found();
        }
        return Ok(out);
    }
    let bytes = match message_mime(facade, &message) {
        Ok(bytes) => bytes,
        Err(text) => {
            note_error(format!(
                "Email message {email_message_id} could not be exported: {text}"
            ));
            return Ok(out);
        }
    };
    out.insert("mimeText".into(), Value::string(String::from_utf8_lossy(&bytes)));
    out.insert("mimeBytes".into(), Value::Bytes(bytes));
    out.insert(
        "fileName".into(),
        Value::string(file_name(&field_text(&message, "subject"))),
    );
    out.insert("contentType".into(), Value::string("message/rfc822"));
    Ok(out)
}

/// The user reaches the message's account at read level or more, or authorization is disabled.
fn may_read(message: &EntityValue) -> Result<bool, LibError> {
    if ExecutionContext::with_current(ExecutionContext::authz_disabled).unwrap_or(false) {
        return Ok(true);
    }
    let account_id = field_text(message, "emailAccountId");
    if account_id.is_empty() {
        return Ok(false);
    }
    let mut access_in = BTreeMap::new();
    access_in.insert("emailAccountId".to_string(), Value::string(&account_id));
    let access = crate::service::sync(&format!("{SERVICES}.get#EmailAccountAccess"), &access_in)
        .map_err(|err| LibError::new(err.message()))?;
    Ok(!has_error() && value_number(access.get("level")).unwrap_or(0) >= 1)
}

/// The RFC 5322 bytes of a stored message and its files.
fn message_mime(facade: &EntityFacade, message: &EntityValue) -> Result<Vec<u8>, String> {
    let id = field_text(message, "emailMessageId");
    let mut reply_to = Vec::new();
    let identity = find_one(
        facade,
        "moqui.basic.email.EmailIdentity",
        "emailIdentityId",
        &field_text(message, "emailIdentityId"),
    )
    .map_err(|err| err.to_string())?;
    if let Some(identity) = identity {
        reply_to = split_addresses(&field_text(&identity, "replyToAddress"));
    }
    let from_address = field_text(message, "fromAddress");
    let mut message_id = field_text(message, "messageId").trim().to_string();
    if message_id.is_empty() {
        // a draft that was never sent nor appended has none yet
        message_id = new_message_id(&from_address).map_err(|err| err.message().to_string())?;
    }
    let date = ["sentDate", "messageDate", "receivedDate", "lastUpdatedStamp"]
        .iter()
        .find_map(|name| value_millis(message.get(name)));
    let mut letter = Letter {
        subject: field_text(message, "subject"),
        from_address,
        from_name: field_text(message, "fromName"),
        to: split_addresses(&field_text(message, "toAddresses")),
        cc: split_addresses(&field_text(message, "ccAddresses")),
        bcc: Vec::new(),
        reply_to,
        bounce: String::new(),
        body: field_text(message, "body"),
        body_text: field_text(message, "bodyText"),
        attachments: Vec::new(),
        inline: Vec::new(),
        message_id,
        in_reply_to: field_text(message, "inReplyTo"),
        references: field_text(message, "referencesHeader"),
        date,
        extra_headers: client_headers(&field_text(message, "headersString")),
    };
    stored_attachments(facade, &id, &mut letter).map_err(|err| err.message().to_string())?;
    let built = build_lettre(&letter).map_err(|err| err.message().to_string())?;
    Ok(built.formatted())
}

/// The subject as a file name: what is not a word character, space, dot or hyphen becomes `_`, at most 80 characters,
/// `message` when nothing is left, and `.eml`.
pub(super) fn file_name(subject: &str) -> String {
    let mut name = String::new();
    let mut replaced = false;
    for ch in subject.chars() {
        if ch.is_alphanumeric() || matches!(ch, '_' | ' ' | '.' | '-') {
            name.push(ch);
            replaced = false;
        } else if !replaced {
            name.push('_');
            replaced = true;
        }
    }
    let name: String = name.trim().trim_matches('.').chars().take(80).collect();
    let name = name.trim();
    format!("{}.eml", if name.is_empty() { "message" } else { name })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use moqui_lib::object::{basic_convert, to_plain_string, Value};

    use crate::context::{ExecutionContext, ExecutionContextFactory};
    use crate::entity::EntityFacade;

    fn text(value: &str) -> Value {
        Value::string(value)
    }

    fn row(facade: &EntityFacade, entity: &str, fields: &[(&str, Value)]) {
        let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
        let mut row = facade.make_value(entity).unwrap();
        for (field, value) in fields {
            row.set(*field, value.clone());
        }
        row.create(facade).unwrap();
    }

    fn login(user_id: &str) {
        let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
        crate::user::internal_login_user(user_id).unwrap();
    }

    fn call(name: &str, parameters: &[(&str, Value)]) -> (BTreeMap<String, Value>, String) {
        let parameters: BTreeMap<String, Value> = parameters
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect();
        let out = crate::service::sync(&format!("{}.{name}", super::SERVICES), &parameters).unwrap();
        let errors = ExecutionContext::with_current(|ec| {
            let text = ec.message().errors().join("\n");
            ec.message().clear_errors();
            text
        })
        .unwrap_or_default();
        (out, errors)
    }

    /// A sent message with an attachment and an inline image exports as one RFC 5322 file: the headers of the message, the
    /// text and HTML as alternatives, the image by its Content-ID beside the HTML, and the file's bytes as they are.
    #[test]
    fn a_sent_message_exports_with_its_files() {
        let ctx = ExecutionContextFactory::test_context_in(
            "",
            crate::service::ServicePools::idle(),
            crate::context::framework_dir(),
        );
        let facade = crate::testing::open();
        facade.assume_db_tables();
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        let framework = crate::context::framework_dir().join("service/org/moqui");
        let failures = crate::service::lint(&[
            framework.join("EmailServices.xml"),
            framework.join("impl/EmailServices.xml"),
            framework.join("impl/EmailClientServices.xml"),
            framework.join("impl/ShellServices.xml"),
        ])
        .unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        for id in ["alice", "eve"] {
            row(
                &facade,
                "moqui.security.UserAccount",
                &[
                    ("userId", text(id)),
                    ("username", text(id)),
                    ("emailAddress", text(&format!("{id}@example.com"))),
                ],
            );
        }
        login("alice");
        let (created, errors) = call(
            "create#EmailAccount",
            &[
                ("accountKindEnumId", text("EakShared")),
                ("emailAddress", text("team@example.com")),
                ("accountName", text("Team")),
                ("smtpHost", text("smtp.example.com")),
                ("smtpStartTls", text("N")),
                ("storeHost", text("imap.example.com")),
                ("mailPassword", text("secret")),
            ],
        );
        assert!(errors.is_empty(), "{errors}");
        let account = to_plain_string(created.get("emailAccountId").unwrap());
        let sent = basic_convert(&Value::I64(1_760_000_000_000), "java.sql.Timestamp").value;
        row(
            &facade,
            super::MESSAGE,
            &[
                ("emailMessageId", text("EXP1")),
                ("emailAccountId", text(&account)),
                ("statusId", text("ES_SENT")),
                ("subject", text("Plan für Q3")),
                ("body", text("<p>See the plan <img src=\"cid:logo1\"></p>")),
                ("bodyText", text("See the plan")),
                ("fromAddress", text("team@example.com")),
                ("fromName", text("Team")),
                ("toAddresses", text("bob@example.com")),
                ("ccAddresses", text("\"Doe, Jane\" <jane@example.com>")),
                ("messageId", text("<exp1@example.com>")),
                ("inReplyTo", text("<earlier@example.com>")),
                ("sentDate", sent),
            ],
        );
        let pdf: Vec<u8> = (0..=255u8).chain([0xFF, 0x00, 0x0D, 0x0A]).collect();
        let resource = ctx.factory().resource();
        resource
            .write_bytes("dbresource://test/export/report.pdf", &pdf)
            .unwrap();
        resource
            .write_bytes("dbresource://test/export/logo.png", b"\x89PNG\r\n\x1a\nlogo")
            .unwrap();
        for (id, name, kind, location, inline) in [
            (
                "EXPA1",
                "report.pdf",
                "application/pdf",
                "dbresource://test/export/report.pdf",
                "",
            ),
            (
                "EXPA2",
                "logo.png",
                "image/png",
                "dbresource://test/export/logo.png",
                "logo1",
            ),
        ] {
            let mut fields = vec![
                ("emailMessageAttachmentId", text(id)),
                ("emailMessageId", text("EXP1")),
                ("fileName", text(name)),
                ("contentType", text(kind)),
                ("attachmentLocation", text(location)),
            ];
            if !inline.is_empty() {
                fields.push(("isInline", text("Y")));
                fields.push(("contentId", text(&format!("<{inline}>"))));
            }
            row(&facade, "moqui.basic.email.EmailMessageAttachment", &fields);
        }

        let (out, errors) = call("export#EmailMessage", &[("emailMessageId", text("EXP1"))]);
        assert!(errors.is_empty(), "{errors}");
        assert_eq!(to_plain_string(out.get("contentType").unwrap()), "message/rfc822");
        assert_eq!(to_plain_string(out.get("fileName").unwrap()), "Plan für Q3.eml");
        let Some(Value::Bytes(bytes)) = out.get("mimeBytes") else {
            panic!("no mimeBytes: {out:?}")
        };
        let mime = to_plain_string(out.get("mimeText").unwrap());
        assert_eq!(mime.as_bytes(), bytes.as_slice());
        assert!(bytes.is_ascii(), "{mime}");
        for header in [
            "Message-ID: <exp1@example.com>",
            "In-Reply-To: <earlier@example.com>",
            "multipart/mixed",
            "multipart/related",
        ] {
            assert!(mime.contains(header), "{header} in {mime}");
        }
        let parsed = mail_parser::MessageParser::default().parse(bytes.as_slice()).unwrap();
        assert_eq!(parsed.subject(), Some("Plan für Q3"));
        assert_eq!(
            parsed
                .from()
                .and_then(|from| from.first())
                .and_then(|from| from.address()),
            Some("team@example.com")
        );
        assert_eq!(
            parsed.cc().and_then(|cc| cc.first()).and_then(|cc| cc.name()),
            Some("Doe, Jane")
        );
        assert_eq!(parsed.date().map(|date| date.to_timestamp()), Some(1_760_000_000));
        assert_eq!(parsed.body_text(0).as_deref().map(str::trim), Some("See the plan"));
        assert!(parsed.body_html(0).is_some_and(|html| html.contains("cid:logo1")));
        let files: Vec<(String, Vec<u8>, Option<String>)> = parsed
            .attachments()
            .map(|part| {
                (
                    mail_parser::MimeHeaders::attachment_name(part)
                        .unwrap_or_default()
                        .to_string(),
                    part.contents().to_vec(),
                    mail_parser::MimeHeaders::content_id(part).map(str::to_string),
                )
            })
            .collect();
        assert!(
            files.iter().any(|(name, body, _)| name == "report.pdf" && body == &pdf),
            "{files:?}"
        );
        assert!(
            files
                .iter()
                .any(|(_, body, cid)| body.as_slice() == b"\x89PNG\r\n\x1a\nlogo" && cid.as_deref() == Some("logo1")),
            "{files:?}"
        );

        // a user who does not reach the account gets nothing
        login("eve");
        let (out, errors) = call("export#EmailMessage", &[("emailMessageId", text("EXP1"))]);
        assert!(errors.contains("Email message EXP1 not found"), "{errors}");
        assert!(!out.contains_key("mimeBytes"));
    }

    #[test]
    fn a_subject_is_a_safe_file_name() {
        assert_eq!(super::file_name("Re: Q3 plan / budget?"), "Re_ Q3 plan _ budget_.eml");
        assert_eq!(super::file_name(""), "message.eml");
        assert_eq!(super::file_name("../.."), "_.eml");
        assert_eq!(super::file_name("Café menu"), "Café menu.eml");
        assert_eq!(super::file_name(&"x".repeat(200)).len(), 84);
    }
}
