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
//!
//! A received message (one whose `headersString` holds the header block it came with, not only the headers the client
//! writes itself) keeps that block: its Received, Authentication-Results, DKIM-Signature, List-Unsubscribe and the rest
//! as they came, folded lines and all, with the MIME structure (MIME-Version, Content-Type, Content-Transfer-Encoding)
//! of the body made again in place of the original's. A header block cut at the most the store keeps loses its last,
//! perhaps partial, field.

use std::collections::BTreeMap;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use super::implementation::{
    build_lettre, client_headers, find_one, new_message_id, note_error, split_addresses, stored_attachments, Letter,
    CLIENT_HEADERS,
};
use super::store::{HEADERS_MAX, MESSAGE};
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
    let built = build_lettre(&letter)
        .map_err(|err| err.message().to_string())?
        .formatted();
    Ok(with_received_headers(built, &field_text(message, "headersString")))
}

/// The header fields the body made again has its own of: the MIME structure of the top part.
const STRUCTURE_HEADERS: [&str; 7] = [
    "MIME-Version",
    "Content-Type",
    "Content-Transfer-Encoding",
    "Content-Disposition",
    "Content-ID",
    "Content-Length",
    "Content-MD5",
];

/// The fields of a header block, each its first line and the folded lines after it, without line ends. A line that
/// is not a field (a stray line, an mbox `From ` line) is left out with its folded lines.
fn header_fields(block: &str) -> Vec<(String, Vec<String>)> {
    let mut fields: Vec<(String, Vec<String>)> = Vec::new();
    let mut skipping = false;
    for line in block.split('\n').map(|line| line.trim_end_matches('\r')) {
        if line.starts_with([' ', '\t']) {
            if !skipping {
                if let Some((_, lines)) = fields.last_mut() {
                    lines.push(line.to_string());
                }
            }
            continue;
        }
        let name = line.split_once(':').map(|(name, _)| name).unwrap_or_default();
        // RFC 5322 field name: printable ASCII but space and colon
        skipping = name.is_empty() || !name.bytes().all(|byte| (33..=126).contains(&byte) && byte != b':');
        if !skipping {
            fields.push((name.to_string(), vec![line.to_string()]));
        }
    }
    fields
}

fn named(name: &str, names: &[&str]) -> bool {
    names.iter().any(|known| known.eq_ignore_ascii_case(name))
}

/// The message with the header block a received message came with in place of the one made again, keeping the made
/// MIME structure. A message whose stored headers are only the client's own (a draft, a reply sent from here), or none,
/// is as it was made.
fn with_received_headers(built: Vec<u8>, stored: &str) -> Vec<u8> {
    let mut fields = header_fields(stored);
    if !fields.iter().any(|(name, _)| !named(name, &CLIENT_HEADERS)) {
        return built;
    }
    // a block cut at the most the store keeps may end in the middle of a field
    if stored.len() + 2 >= HEADERS_MAX {
        fields.pop();
    }
    let Some(split) = built.windows(4).position(|window| window == b"\r\n\r\n") else {
        return built;
    };
    let made = String::from_utf8_lossy(&built[..split]).into_owned();
    let mut out = String::new();
    for (_, lines) in fields.iter().filter(|(name, _)| !named(name, &STRUCTURE_HEADERS)) {
        out.push_str(&lines.join("\r\n"));
        out.push_str("\r\n");
    }
    for (_, lines) in header_fields(&made)
        .iter()
        .filter(|(name, _)| named(name, &STRUCTURE_HEADERS[..3]))
    {
        out.push_str(&lines.join("\r\n"));
        out.push_str("\r\n");
    }
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&built[split + 2..]);
    bytes
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

        // a received message: its stored headers are in the file
        row(
            &facade,
            super::MESSAGE,
            &[
                ("emailMessageId", text("EXP2")),
                ("emailAccountId", text(&account)),
                ("statusId", text("ES_RECEIVED")),
                ("subject", text("News")),
                ("bodyText", text("Hello")),
                ("fromAddress", text("news@example.org")),
                ("toAddresses", text("team@example.com")),
                ("messageId", text("<news2@example.org>")),
                (
                    "headersString",
                    text("Received: from mail.example.org\n\tby mx.example.com\nFrom: news@example.org\nTo: team@example.com\nSubject: News\nMessage-ID: <news2@example.org>\nList-Unsubscribe: <mailto:leave@example.org>\nContent-Type: text/plain"),
                ),
            ],
        );
        let (out, errors) = call("export#EmailMessage", &[("emailMessageId", text("EXP2"))]);
        assert!(errors.is_empty(), "{errors}");
        let mime = to_plain_string(out.get("mimeText").unwrap());
        assert!(
            mime.starts_with("Received: from mail.example.org\r\n\tby mx.example.com\r\n"),
            "{mime}"
        );
        assert!(
            mime.contains("List-Unsubscribe: <mailto:leave@example.org>\r\n"),
            "{mime}"
        );

        // a user who does not reach the account gets nothing
        login("eve");
        let (out, errors) = call("export#EmailMessage", &[("emailMessageId", text("EXP1"))]);
        assert!(errors.contains("Email message EXP1 not found"), "{errors}");
        assert!(!out.contains_key("mimeBytes"));
    }

    /// A received message keeps the header block it came with (folded fields too), with the MIME structure of the body
    /// made again and not the original's; a stray line is left out, and a draft's own headers are the made ones.
    #[test]
    fn a_received_message_keeps_its_stored_headers() {
        let stored = concat!(
            "From MAILER-DAEMON Mon Oct 19 10:00:00 2026\n",
            "Received: from mail.example.org (mail.example.org [192.0.2.1])\n",
            "\tby mx.example.com with ESMTPS id abc123\n",
            "\tfor <team@example.com>; Mon, 19 Oct 2026 10:00:00 +0000\n",
            "Authentication-Results: mx.example.com; dkim=pass header.d=example.org; spf=pass\n",
            "DKIM-Signature: v=1; a=rsa-sha256; d=example.org; s=s1;\n",
            " h=from:to:subject; bh=AAAA; b=BBBB\n",
            "From: \"News\" <news@example.org>\n",
            "To: team@example.com\n",
            "Subject: =?utf-8?q?Caf=C3=A9_news?=\n",
            "Date: Mon, 19 Oct 2026 10:00:00 +0000\n",
            "Message-ID: <news1@example.org>\n",
            "List-Unsubscribe: <mailto:leave@example.org>, <https://example.org/leave?u=1>\n",
            "List-Unsubscribe-Post: List-Unsubscribe=One-Click\n",
            "MIME-Version: 1.0\n",
            "Content-Type: multipart/alternative; boundary=\"old-boundary\"",
        );
        let letter = super::Letter {
            subject: "Café news".to_string(),
            from_address: "news@example.org".to_string(),
            from_name: "News".to_string(),
            to: vec!["team@example.com".to_string()],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: vec!["identity@example.com".to_string()],
            bounce: String::new(),
            body: "<p>Hello</p>".to_string(),
            body_text: "Hello".to_string(),
            attachments: Vec::new(),
            inline: Vec::new(),
            message_id: "<news1@example.org>".to_string(),
            in_reply_to: String::new(),
            references: String::new(),
            date: Some(1_760_000_000_000),
            extra_headers: Vec::new(),
        };
        let built = super::build_lettre(&letter).unwrap().formatted();
        let mime = String::from_utf8(super::with_received_headers(built.clone(), stored)).unwrap();
        let (head, body) = mime.split_once("\r\n\r\n").expect("a header block");
        for expected in [
            "Received: from mail.example.org (mail.example.org [192.0.2.1])\r\n\tby mx.example.com with ESMTPS id abc123\r\n\tfor <team@example.com>; Mon, 19 Oct 2026 10:00:00 +0000\r\n",
            "Authentication-Results: mx.example.com; dkim=pass",
            "DKIM-Signature: v=1; a=rsa-sha256; d=example.org; s=s1;\r\n h=from:to:subject; bh=AAAA; b=BBBB\r\n",
            "List-Unsubscribe: <mailto:leave@example.org>, <https://example.org/leave?u=1>\r\n",
            "List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
            "Subject: =?utf-8?q?Caf=C3=A9_news?=\r\n",
        ] {
            assert!(format!("{head}\r\n").contains(expected), "{expected} in {head}");
        }
        assert!(head.starts_with("Received: "), "{head}");
        assert!(
            !head.contains("MAILER-DAEMON") && !head.contains("old-boundary"),
            "{head}"
        );
        // the made structure, once; not the identity's Reply-To the builder would add
        assert_eq!(head.matches("MIME-Version:").count(), 1, "{head}");
        assert_eq!(head.to_ascii_lowercase().matches("content-type:").count(), 1, "{head}");
        assert!(!head.contains("identity@example.com"), "{head}");
        let made = String::from_utf8_lossy(&built).into_owned();
        assert!(made.ends_with(body), "the made body");
        let parsed = mail_parser::MessageParser::default().parse(mime.as_bytes()).unwrap();
        assert_eq!(parsed.subject(), Some("Café news"));
        assert_eq!(parsed.body_text(0).as_deref().map(str::trim), Some("Hello"));
        assert!(parsed.body_html(0).is_some_and(|html| html.contains("<p>Hello</p>")));
        assert_eq!(parsed.message_id(), Some("news1@example.org"));
        assert!(parsed.header("List-Unsubscribe").is_some());

        // only the client's own headers (an automatic reply's): the message as made
        let own = super::with_received_headers(
            built.clone(),
            "Auto-Submitted: auto-replied\nX-Auto-Response-Suppress: All",
        );
        assert_eq!(own, built);
        assert_eq!(super::with_received_headers(built.clone(), ""), built);
        // a block cut at the most the store keeps loses its last field
        let long = format!("Received: from a\nX-Long: {}", "x".repeat(super::HEADERS_MAX));
        let cut = String::from_utf8(super::with_received_headers(built, &long)).unwrap();
        assert!(
            cut.starts_with("Received: from a\r\n") && !cut.contains("X-Long"),
            "{}",
            &cut[..80]
        );
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
