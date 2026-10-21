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

//! Mail the SMTP listener received for the hosted and shared accounts (moqui-host's `inbound-smtp` feature, behind the
//! edge gateway; docs/Mail-Inbound.md). The listener asks [`find_recipient`] at each `RCPT TO` and hands the message to
//! [`deliver`] after `DATA`; both run on a request worker with authorization disabled.
//!
//! Routing: a recipient is an account of kind EakHosted or EakShared that is not removed and has no incoming (IMAP or
//! POP) server, since an account the sync feeds would get every message twice. The address matches the account's
//! address, then one of its `EmailIdentity` addresses, without regard to case; then the same for the address without
//! its plus tag (`sales+news@` is `sales@`), the delimiters being the listener's `smtp-server` `plus-separator` characters. One
//! address is one account: when several accounts claim it at the same step, the one made first (the lowest sequenced
//! id) gets the mail and a warning names the others, so an identity added later cannot read another account's mail.
//!
//! Delivery, once per account however many of its addresses were recipients:
//!
//! 1. The trace fields go on top: `Return-Path` (the envelope sender; any the message had is removed), a
//!    `Delivered-To` per recipient address of the account, and `Received`. With `smtp_authserv_id` set only the
//!    `Authentication-Results` fields of the gateway (that authserv-id) are kept; the gateway's verdict is trusted and
//!    nothing is verified again.
//! 2. A message the account already received (the same Message-ID) is not stored again, so the gateway's retry after a
//!    451 does not double the accounts that took it the first time.
//! 3. The message is stored as the sync stores fetched mail ([`super::store::store_message`]: the `EmailMessage` in the
//!    Inbox, its files as DbResource, its thread), in a transaction of its own; the app's `EmailMessage` ECA files it
//!    on the records by the account's filing policy. Mail the gateway marked as spam (one of `smtp-server` `spam-headers` says
//!    yes) goes to the Junk folder instead and is not filed.
//! 4. For Inbox mail: the email ECAs, the account's rules (`run#EmailRules`), the snooze wake and the out-of-office
//!    reply; then the account's users' counts. Each of these logs its own failure and the message stays stored.
//!
//! A store that fails is that account's [`Delivery`] error, which the listener answers with 451; the other accounts
//! keep their copy.

use std::collections::HashSet;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use super::implementation::{find_one, force_new};
use super::inbound::{run_stored_ecas, StoredMessage};
use super::store::{self, header_value, parse_message, Placement, MESSAGE};
use super::sync::{account_info, after_new_mail, load_folders, publish_counts, run_rules, ACCOUNT};
use crate::entity::{Condition, EntityFacade, EntityValue};
use crate::service::util::{field_text, now_millis};

const IDENTITY: &str = "moqui.basic.email.EmailIdentity";
/// The account kinds the listener delivers to.
const KINDS: [&str; 2] = ["EakHosted", "EakShared"];

/// The hosted or shared account mail to `address` goes to, or none when no account takes it. `delimiters` are the
/// plus-addressing characters (`+` by default); empty turns plus addressing off.
///
/// # Errors
///
/// Returns an error when the database cannot be read; the listener answers that with a temporary failure.
pub fn find_recipient(facade: &EntityFacade, address: &str, delimiters: &str) -> Result<Option<String>, LibError> {
    let address = address.trim().to_lowercase();
    let Some((local, domain)) = address.rsplit_once('@') else {
        return Ok(None);
    };
    if local.is_empty() || domain.is_empty() {
        return Ok(None);
    }
    let mut candidates = vec![address.clone()];
    if let Some(at) = local.find(|ch| delimiters.contains(ch)).filter(|at| *at > 0) {
        candidates.push(format!("{}@{domain}", &local[..at]));
    }
    for candidate in &candidates {
        if let Some(id) = by_account_address(facade, candidate)? {
            return Ok(Some(id));
        }
        if let Some(id) = by_identity_address(facade, candidate)? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Whether the listener delivers to this account: hosted or shared, not removed, no incoming server.
fn receives(facade: &EntityFacade, row: &EntityValue) -> Result<bool, LibError> {
    let kind = field_text(row, "accountKindEnumId");
    if !KINDS.contains(&kind.as_str()) || field_text(row, "statusId") == "EasRemoved" {
        return Ok(false);
    }
    Ok(account_info(facade, row)?.server.is_none())
}

/// Sequenced ids in the order they were made: shorter first, then by text.
fn made_order(left: &str, right: &str) -> std::cmp::Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

/// The first of the accounts that claim `address`, with a warning when there are more.
fn first_claim(mut ids: Vec<String>, address: &str, how: &str) -> Option<String> {
    ids.dedup();
    if ids.len() > 1 {
        tracing::warn!(
            "Mail to {address} goes to email account {}, the first of the accounts with it as {how}: {}",
            ids[0],
            ids.join(", ")
        );
    }
    ids.into_iter().next()
}

fn by_account_address(facade: &EntityFacade, address: &str) -> Result<Option<String>, LibError> {
    let rows = facade
        .find(ACCOUNT)?
        .condition(Condition::ignore_case(Condition::equals(
            "emailAddress",
            Value::string(address),
        )))
        .condition(Condition::is_in(
            "accountKindEnumId",
            KINDS.iter().map(|kind| Value::string(*kind)).collect(),
        ))
        .list()?;
    let mut ids = Vec::new();
    for row in rows.values() {
        if receives(facade, row)? {
            ids.push(field_text(row, "emailAccountId"));
        }
    }
    ids.sort_by(|left, right| made_order(left, right));
    Ok(first_claim(ids, address, "its address"))
}

fn by_identity_address(facade: &EntityFacade, address: &str) -> Result<Option<String>, LibError> {
    let rows = facade
        .find(IDENTITY)?
        .condition(Condition::ignore_case(Condition::equals(
            "emailAddress",
            Value::string(address),
        )))
        .select_field("emailIdentityId")
        .select_field("emailAccountId")
        .list()?;
    let mut identities: Vec<(String, String)> = rows
        .values()
        .iter()
        .map(|row| (field_text(row, "emailIdentityId"), field_text(row, "emailAccountId")))
        .filter(|(_, account)| !account.is_empty())
        .collect();
    identities.sort_by(|left, right| made_order(&left.0, &right.0));
    let mut ids: Vec<String> = Vec::new();
    for (_, account_id) in identities {
        if ids.contains(&account_id) {
            continue;
        }
        if let Some(row) = find_one(facade, ACCOUNT, "emailAccountId", &account_id)? {
            if receives(facade, &row)? {
                ids.push(account_id);
            }
        }
    }
    Ok(first_claim(ids, address, "an identity"))
}

/// The SMTP session a message came in, for its trace fields.
#[derive(Clone, Debug, Default)]
pub struct Envelope {
    /// `MAIL FROM`, empty for a bounce (the null reverse path).
    pub mail_from: String,
    /// The client's `EHLO` or `HELO` name.
    pub helo: String,
    /// The client's address (the gateway's, or the one its PROXY header named).
    pub client_ip: String,
    /// `ESMTP`, `SMTP` or `UTF8ESMTP`.
    pub protocol: String,
    pub session_id: String,
    /// This server's name.
    pub hostname: String,
}

/// What the listener was told to trust.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The gateway's spam verdict fields (`X-Spam-Flag`, `X-Spam-Status`): mail one of them says yes to goes to Junk.
    pub spam_headers: Vec<String>,
    /// The gateway's authserv-id: only its `Authentication-Results` are kept. Empty keeps them all as they came.
    pub authserv_id: String,
}

/// An account and the recipient addresses that named it.
#[derive(Clone, Debug)]
pub struct Recipients {
    pub email_account_id: String,
    pub addresses: Vec<String>,
}

/// What became of the message in one account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stored {
        email_message_id: String,
        junk: bool,
    },
    /// The account had the message already: the gateway sent it again after a temporary failure.
    Duplicate,
    /// The account was removed after `RCPT TO`; there is nothing to retry.
    Gone,
}

/// The message in one account: an error is a store that failed.
#[derive(Debug)]
pub struct Delivery {
    pub email_account_id: String,
    pub result: Result<Outcome, String>,
}

/// The message in each account, with the gateway's verdicts.
#[derive(Debug, Default)]
pub struct Report {
    pub deliveries: Vec<Delivery>,
    /// The gateway marked it as spam.
    pub spam: bool,
    /// The trusted `Authentication-Results` in short (`spf=pass dkim=pass dmarc=pass`), for the log.
    pub authentication: String,
}

impl Report {
    /// Some account could not store it: the listener answers 451 and the gateway tries again later.
    #[must_use]
    pub fn store_failed(&self) -> bool {
        self.deliveries.iter().any(|delivery| delivery.result.is_err())
    }
}

/// Store `raw` (the message as `DATA` brought it) for each account of `recipients` and run what new mail runs.
///
/// # Errors
///
/// Returns why when the message is not mail (no header fields), a permanent refusal.
pub fn deliver(
    facade: &EntityFacade,
    raw: &[u8],
    envelope: &Envelope,
    recipients: &[Recipients],
    options: &Options,
) -> Result<Report, String> {
    let (head, body) = split_head(raw);
    let fields = header_fields(head);
    if fields.is_empty() {
        return Err("the message has no header fields".to_string());
    }
    parse_message(raw)?;
    let head_text = String::from_utf8_lossy(head);
    let spam = options
        .spam_headers
        .iter()
        .any(|name| says_yes(&header_value(&head_text, name)));
    let authserv_id = options.authserv_id.trim().to_ascii_lowercase();
    let mut kept: Vec<&[u8]> = Vec::with_capacity(fields.len());
    let mut results: Vec<String> = Vec::new();
    for field in fields {
        let (name, value) = field_name_value(field);
        if name.eq_ignore_ascii_case("Return-Path") {
            continue;
        }
        if name.eq_ignore_ascii_case("Authentication-Results") {
            if !authserv_id.is_empty() && authserv_of(&value) != authserv_id {
                continue;
            }
            results.push(value);
        }
        kept.push(field);
    }
    let authentication = verdict(&results);
    let mut report = Report {
        deliveries: Vec::new(),
        spam,
        authentication,
    };
    let mut seen = HashSet::new();
    for recipient in recipients {
        if !seen.insert(recipient.email_account_id.clone()) {
            continue;
        }
        let result = deliver_one(facade, &kept, body, envelope, recipient, spam);
        if let Err(text) = &result {
            tracing::error!(
                account = %recipient.email_account_id,
                "Could not store the mail of SMTP session {} for email account {}: {text}",
                envelope.session_id,
                recipient.email_account_id
            );
        }
        report.deliveries.push(Delivery {
            email_account_id: recipient.email_account_id.clone(),
            result,
        });
    }
    Ok(report)
}

fn deliver_one(
    facade: &EntityFacade,
    kept: &[&[u8]],
    body: &[u8],
    envelope: &Envelope,
    recipient: &Recipients,
    spam: bool,
) -> Result<Outcome, String> {
    let account_id = recipient.email_account_id.as_str();
    let failed = |err: LibError| err.to_string();
    let Some(row) = find_one(facade, ACCOUNT, "emailAccountId", account_id).map_err(failed)? else {
        tracing::warn!(
            "Email account {account_id} is gone; the mail of SMTP session {} is not kept for it",
            envelope.session_id
        );
        return Ok(Outcome::Gone);
    };
    if field_text(&row, "statusId") == "EasRemoved" {
        tracing::warn!(
            "Email account {account_id} is removed; the mail of SMTP session {} is not kept for it",
            envelope.session_id
        );
        return Ok(Outcome::Gone);
    }
    let folders = load_folders(facade, account_id).map_err(failed)?;
    let inbox = folders
        .iter()
        .find(|folder| folder.kind == "EfkInbox")
        .ok_or_else(|| format!("email account {account_id} has no Inbox folder"))?;
    let junk = spam
        .then(|| folders.iter().find(|folder| folder.kind == "EfkJunk"))
        .flatten();
    let folder = junk.unwrap_or(inbox);
    let raw = with_trace(kept, body, envelope, &recipient.addresses);
    let parsed = parse_message(&raw)?;
    if !parsed.message_id.is_empty() {
        let had = facade
            .find(MESSAGE)
            .and_then(|find| {
                find.eq("emailAccountId", Value::string(account_id))
                    .eq("messageId", Value::string(&parsed.message_id))
                    .condition(Condition::is_not_null("receivedDate"))
                    .count()
            })
            .map_err(failed)?;
        if had > 0 {
            tracing::info!(
                "Email account {account_id} has message {} already; SMTP session {} does not store it again",
                parsed.message_id,
                envelope.session_id
            );
            return Ok(Outcome::Duplicate);
        }
    }
    let received = now_millis();
    let place = Placement {
        account_id,
        folder_id: Some(&folder.id),
        remote_folder_id: None,
        remote_uid: None,
        remote_uidl: None,
        status_id: "ES_RECEIVED",
        read: false,
        flagged: false,
        answered: false,
        received_millis: Some(received),
        server_size: None,
        archived: false,
    };
    let id = force_new(facade, || store::store_message(facade, &parsed, &place)).map_err(failed)?;
    if junk.is_none() {
        let server_id = field_text(&row, "emailServerId");
        let ids = vec![id.clone()];
        step(account_id, "email ECAs", || {
            run_stored_ecas(
                facade,
                &StoredMessage {
                    raw: &raw,
                    server_id: &server_id,
                    account_id,
                    email_message_id: &id,
                    received_millis: Some(received),
                    seen: false,
                    answered: false,
                    flagged: false,
                },
            );
        });
        step(account_id, "rules", || run_rules(account_id, &ids));
        step(account_id, "new mail", || after_new_mail(account_id, &ids));
    }
    step(account_id, "counts", || publish_counts(account_id));
    Ok(Outcome::Stored {
        email_message_id: id,
        junk: junk.is_some(),
    })
}

/// A step after the store, which logs its own failures; a panic is logged here and the delivery goes on.
fn step(account_id: &str, name: &str, run: impl FnOnce()) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        tracing::error!(
            "The {name} step on new mail of email account {account_id} failed: {}",
            crate::service::pool::panic_text(payload.as_ref())
        );
    }
}

/// The header block (each field with its line end) and the rest (from the empty line on).
fn split_head(raw: &[u8]) -> (&[u8], &[u8]) {
    let mut start = 0;
    while start < raw.len() {
        let end = raw[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(raw.len(), |at| start + at + 1);
        let line = &raw[start..end];
        if line == b"\n" || line == b"\r\n" {
            return (&raw[..start], &raw[start..]);
        }
        start = end;
    }
    (raw, &[])
}

/// The fields of a header block, each with its folded lines and line ends.
fn header_fields(head: &[u8]) -> Vec<&[u8]> {
    let mut fields: Vec<&[u8]> = Vec::new();
    let mut field_start: Option<usize> = None;
    let mut start = 0;
    while start < head.len() {
        let end = head[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(head.len(), |at| start + at + 1);
        let continues = matches!(head[start], b' ' | b'\t');
        if !continues {
            if let Some(from) = field_start {
                fields.push(&head[from..start]);
            }
            field_start = Some(start);
        } else if field_start.is_none() {
            // a folded line with no field before it: kept as one of its own
            field_start = Some(start);
        }
        start = end;
    }
    if let Some(from) = field_start {
        fields.push(&head[from..]);
    }
    fields
}

/// A field's name and its unfolded value.
fn field_name_value(field: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(field);
    let Some((name, value)) = text.split_once(':') else {
        return (String::new(), String::new());
    };
    let value = value
        .split('\n')
        .map(|line| line.trim_matches(|ch: char| ch == '\r' || ch == ' ' || ch == '\t'))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (name.trim().to_string(), value)
}

/// The authserv-id of an `Authentication-Results` value: its first word before the first `;`.
fn authserv_of(value: &str) -> String {
    value
        .split(';')
        .next()
        .and_then(|first| first.split_whitespace().next())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The first result of each method (`spf=pass dkim=pass dmarc=pass`) in these `Authentication-Results` values.
fn verdict(values: &[String]) -> String {
    let mut found: Vec<String> = Vec::new();
    for value in values {
        for part in value.split(';').skip(1) {
            let Some(token) = part.split_whitespace().next() else {
                continue;
            };
            let Some((method, result)) = token.split_once('=') else {
                continue;
            };
            let method = method.to_ascii_lowercase();
            if !matches!(method.as_str(), "spf" | "dkim" | "dmarc" | "arc") {
                continue;
            }
            if found.iter().any(|seen| seen.starts_with(&format!("{method}="))) {
                continue;
            }
            found.push(format!("{method}={}", result.to_ascii_lowercase()));
        }
    }
    found.join(" ")
}

/// A spam verdict field that says yes: `YES`, `Yes, score=7.2`, `true`.
fn says_yes(value: &str) -> bool {
    let word = value
        .trim()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(word.as_str(), "yes" | "true")
}

/// Text for a trace field: no control characters, at most 200 characters.
fn trace_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .take(200)
        .collect()
}

/// The message for one account: the trace fields, the fields kept, then the body.
fn with_trace(kept: &[&[u8]], body: &[u8], envelope: &Envelope, addresses: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 1024);
    let mut push = |line: String| out.extend_from_slice(line.as_bytes());
    push(format!("Return-Path: <{}>\r\n", trace_text(&envelope.mail_from)));
    for address in addresses {
        push(format!("Delivered-To: {}\r\n", trace_text(address)));
    }
    let date = jiff::Zoned::now().strftime("%a, %d %b %Y %H:%M:%S %z").to_string();
    let helo = if envelope.helo.trim().is_empty() {
        "unknown".to_string()
    } else {
        trace_text(envelope.helo.trim())
    };
    let protocol = if envelope.protocol.is_empty() {
        "ESMTP"
    } else {
        envelope.protocol.as_str()
    };
    let recipient = match addresses {
        [only] => format!("\r\n\tfor <{}>", trace_text(only)),
        _ => String::new(),
    };
    push(format!(
        "Received: from {helo} ([{}])\r\n\tby {} (mr-moqui) with {} id {}{recipient};\r\n\t{date}\r\n",
        trace_text(&envelope.client_ip),
        trace_text(&envelope.hostname),
        trace_text(protocol),
        trace_text(&envelope.session_id),
    ));
    for field in kept {
        out.extend_from_slice(field);
        if !field.ends_with(b"\n") {
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields_keep_their_folded_lines() {
        let raw = b"A: one\r\nB: two\r\n  more\r\nC: three\r\n\r\nbody\r\n";
        let (head, body) = split_head(raw);
        assert_eq!(body, b"\r\nbody\r\n");
        let fields = header_fields(head);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[1], b"B: two\r\n  more\r\n");
        assert_eq!(field_name_value(fields[1]), ("B".to_string(), "two more".to_string()));
        let (head, body) = split_head(b"Only: header\n");
        assert_eq!(head, b"Only: header\n");
        assert!(body.is_empty());
    }

    #[test]
    fn verdicts_read_the_gateway_fields() {
        assert!(says_yes("YES"));
        assert!(says_yes(" Yes, score=7.1 required=5.0"));
        assert!(!says_yes("No, score=0.1"));
        assert!(!says_yes(""));
        assert_eq!(authserv_of("mx.example.com 1; spf=pass"), "mx.example.com");
        assert_eq!(
            verdict(&[
                "mx.example.com; spf=pass smtp.mailfrom=a.org; dkim=fail header.d=a.org".to_string(),
                "mx.example.com; dkim=pass; dmarc=pass (p=none)".to_string(),
            ]),
            "spf=pass dkim=fail dmarc=pass"
        );
    }

    #[test]
    fn the_trace_fields_go_on_top() {
        let raw = b"Return-Path: <forged@x.org>\r\nSubject: Hi\r\n\r\nbody\r\n";
        let (head, body) = split_head(raw);
        let fields: Vec<&[u8]> = header_fields(head)
            .into_iter()
            .filter(|field| !field.starts_with(b"Return-Path"))
            .collect();
        let envelope = Envelope {
            mail_from: "a@x.org".to_string(),
            helo: "gw.example.com\u{7}".to_string(),
            client_ip: "10.0.0.5".to_string(),
            protocol: "ESMTP".to_string(),
            session_id: "s1".to_string(),
            hostname: "moqui.example.com".to_string(),
        };
        let text = String::from_utf8(with_trace(&fields, body, &envelope, &["Sales@example.com".to_string()])).unwrap();
        assert!(text.starts_with("Return-Path: <a@x.org>\r\nDelivered-To: Sales@example.com\r\nReceived: from gw.example.com? ([10.0.0.5])\r\n\tby moqui.example.com (mr-moqui) with ESMTP id s1\r\n\tfor <Sales@example.com>;"), "{text}");
        assert!(text.ends_with("\r\nSubject: Hi\r\n\r\nbody\r\n"), "{text}");
        assert_eq!(text.matches("Return-Path").count(), 1);
    }
}
