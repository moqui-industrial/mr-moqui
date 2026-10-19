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

//! Scripted local mail servers for the sync tests, on 127.0.0.1 only: an IMAP server with folders, UIDs, flags, modseqs
//! and the extensions a test turns on (CONDSTORE, QRESYNC, MOVE, UIDPLUS, IDLE, ENABLE), a POP3 server with UIDL, an
//! HTTP server for autoconfig files and a DNS server for SRV answers.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moqui_lib::sync::lock;

// ---- IMAP ----

#[derive(Clone, Debug)]
pub(super) struct FakeMessage {
    pub(super) uid: u32,
    pub(super) flags: Vec<String>,
    pub(super) internal_millis: i64,
    pub(super) raw: Vec<u8>,
    pub(super) modseq: u64,
}

#[derive(Clone, Debug)]
pub(super) struct FakeFolder {
    pub(super) name: String,
    pub(super) attributes: String,
    pub(super) validity: u32,
    pub(super) uid_next: u32,
    pub(super) messages: Vec<FakeMessage>,
    /// UIDs expunged, with the modseq of the expunge.
    pub(super) expunged: Vec<(u32, u64)>,
    pub(super) subscribed: bool,
    pub(super) selectable: bool,
}

pub(super) struct FakeImapState {
    pub(super) user: String,
    pub(super) password: String,
    pub(super) caps: Vec<String>,
    pub(super) folders: Vec<FakeFolder>,
    pub(super) modseq: u64,
    /// Every command received, tag removed.
    pub(super) log: Vec<String>,
    next_validity: u32,
}

impl FakeImapState {
    fn folder(&mut self, name: &str) -> Option<&mut FakeFolder> {
        self.folders
            .iter_mut()
            .find(|folder| folder.name.eq_ignore_ascii_case(name) || folder.name == name)
    }

    fn has(&self, cap: &str) -> bool {
        self.caps.iter().any(|known| known.eq_ignore_ascii_case(cap))
    }
}

/// A fake IMAP server. The state is shared with the test, which changes it between syncs as another client would.
pub(super) struct FakeImap {
    pub(super) port: u16,
    pub(super) state: Arc<Mutex<FakeImapState>>,
}

pub(super) fn internal_date(millis: i64) -> String {
    jiff::Timestamp::from_millisecond(millis)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%d-%b-%Y %H:%M:%S +0000")
        .to_string()
}

impl FakeImap {
    /// A server with INBOX, Sent Mail (\Sent), Drafts, Trash, Junk and Archive, signing in `user` with `password`.
    pub(super) fn start(caps: &[&str]) -> Self {
        let mut folders = Vec::new();
        for (index, (name, attributes)) in [
            ("INBOX", "\\HasNoChildren"),
            ("Sent Mail", "\\HasNoChildren \\Sent"),
            ("Drafts", "\\HasNoChildren \\Drafts"),
            ("Trash", "\\HasNoChildren \\Trash"),
            ("Junk", "\\HasNoChildren \\Junk"),
            ("Archive", "\\HasNoChildren \\Archive"),
        ]
        .iter()
        .enumerate()
        {
            folders.push(FakeFolder {
                name: (*name).to_string(),
                attributes: (*attributes).to_string(),
                validity: 1000 + u32::try_from(index).unwrap_or(0),
                uid_next: 1,
                messages: Vec::new(),
                expunged: Vec::new(),
                subscribed: true,
                selectable: true,
            });
        }
        let state = Arc::new(Mutex::new(FakeImapState {
            user: "alice".to_string(),
            password: "app-password".to_string(),
            caps: caps.iter().map(|cap| (*cap).to_string()).collect(),
            folders,
            modseq: 1,
            log: Vec::new(),
            next_validity: 5000,
        }));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let state = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let _ = serve_imap(stream, &state);
                });
            }
        });
        Self { port, state }
    }

    pub(super) fn add_folder(&self, name: &str, attributes: &str, selectable: bool) {
        let mut state = lock(&self.state);
        let validity = state.next_validity;
        state.next_validity += 1;
        state.folders.push(FakeFolder {
            name: name.to_string(),
            attributes: attributes.to_string(),
            validity,
            uid_next: 1,
            messages: Vec::new(),
            expunged: Vec::new(),
            subscribed: true,
            selectable,
        });
    }

    /// A message into a folder, as delivered; its UID.
    pub(super) fn add(&self, folder: &str, raw: &str, flags: &[&str]) -> u32 {
        self.add_dated(folder, raw, flags, jiff::Timestamp::now().as_millisecond())
    }

    pub(super) fn add_dated(&self, folder: &str, raw: &str, flags: &[&str], internal_millis: i64) -> u32 {
        let mut state = lock(&self.state);
        state.modseq += 1;
        let modseq = state.modseq;
        let folder = state.folder(folder).expect("fake folder");
        let uid = folder.uid_next;
        folder.uid_next += 1;
        folder.messages.push(FakeMessage {
            uid,
            flags: flags.iter().map(|flag| (*flag).to_string()).collect(),
            internal_millis,
            raw: raw.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes(),
            modseq,
        });
        uid
    }

    pub(super) fn set_flags(&self, folder: &str, uid: u32, flags: &[&str]) {
        let mut state = lock(&self.state);
        state.modseq += 1;
        let modseq = state.modseq;
        let folder = state.folder(folder).expect("fake folder");
        let message = folder
            .messages
            .iter_mut()
            .find(|message| message.uid == uid)
            .expect("fake message");
        message.flags = flags.iter().map(|flag| (*flag).to_string()).collect();
        message.modseq = modseq;
    }

    pub(super) fn expunge(&self, folder: &str, uid: u32) {
        let mut state = lock(&self.state);
        state.modseq += 1;
        let modseq = state.modseq;
        let folder = state.folder(folder).expect("fake folder");
        folder.messages.retain(|message| message.uid != uid);
        folder.expunged.push((uid, modseq));
    }

    /// Another client moves a message; the UID it has in `to`.
    pub(super) fn move_message(&self, from: &str, uid: u32, to: &str) -> u32 {
        let mut state = lock(&self.state);
        state.modseq += 1;
        let modseq = state.modseq;
        let source = state.folder(from).expect("fake folder");
        let position = source
            .messages
            .iter()
            .position(|message| message.uid == uid)
            .expect("fake message");
        let mut message = source.messages.remove(position);
        source.expunged.push((uid, modseq));
        let target = state.folder(to).expect("fake folder");
        message.uid = target.uid_next;
        message.modseq = modseq;
        target.uid_next += 1;
        let new_uid = message.uid;
        target.messages.push(message);
        new_uid
    }

    /// A new UIDVALIDITY with every message renumbered, as a server rebuilt.
    pub(super) fn reset_validity(&self, folder: &str) {
        let mut state = lock(&self.state);
        let validity = state.next_validity;
        state.next_validity += 1;
        let folder = state.folder(folder).expect("fake folder");
        folder.validity = validity;
        folder.expunged.clear();
        let mut next = 100;
        for message in &mut folder.messages {
            message.uid = next;
            next += 1;
        }
        folder.uid_next = next;
    }

    /// Each message of a folder: UID, flags and its Message-ID header line.
    pub(super) fn messages(&self, folder: &str) -> Vec<(u32, Vec<String>, String)> {
        let mut state = lock(&self.state);
        let folder = state.folder(folder).expect("fake folder");
        folder
            .messages
            .iter()
            .map(|message| {
                let text = String::from_utf8_lossy(&message.raw).to_string();
                let message_id = text
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("message-id:"))
                    .map(|line| line[11..].trim().to_string())
                    .unwrap_or_default();
                (message.uid, message.flags.clone(), message_id)
            })
            .collect()
    }

    /// The raw text of each message in a folder, by UID.
    pub(super) fn raw_messages(&self, folder: &str) -> Vec<(u32, String)> {
        let mut state = lock(&self.state);
        let folder = state.folder(folder).expect("fake folder");
        folder
            .messages
            .iter()
            .map(|message| (message.uid, String::from_utf8_lossy(&message.raw).to_string()))
            .collect()
    }

    pub(super) fn folder_names(&self) -> Vec<String> {
        lock(&self.state)
            .folders
            .iter()
            .map(|folder| folder.name.clone())
            .collect()
    }

    pub(super) fn commands(&self) -> Vec<String> {
        lock(&self.state).log.clone()
    }

    pub(super) fn clear_log(&self) {
        lock(&self.state).log.clear();
    }
}

/// The words of an IMAP command line: atoms, quoted strings (unquoted), and parenthesized lists kept whole.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch == ' ' {
            chars.next();
            continue;
        }
        if ch == '"' {
            chars.next();
            let mut word = String::new();
            while let Some(ch) = chars.next() {
                match ch {
                    '\\' => {
                        if let Some(next) = chars.next() {
                            word.push(next);
                        }
                    }
                    '"' => break,
                    other => word.push(other),
                }
            }
            out.push(word);
            continue;
        }
        if ch == '(' {
            let mut depth = 0;
            let mut word = String::new();
            for ch in chars.by_ref() {
                word.push(ch);
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push(word);
            continue;
        }
        let mut word = String::new();
        while let Some(&ch) = chars.peek() {
            if ch == ' ' {
                break;
            }
            word.push(ch);
            chars.next();
            // a section like BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)] stays one word
            if ch == '[' {
                for inner in chars.by_ref() {
                    word.push(inner);
                    if inner == ']' {
                        break;
                    }
                }
            }
        }
        out.push(word);
    }
    out
}

/// The UIDs a set names among `uids` (sorted): `*` is the largest, and `n:*` with n past it is the largest alone.
fn select_set(set: &str, uids: &[u32]) -> Vec<u32> {
    let max = uids.iter().copied().max().unwrap_or(0);
    let value = |text: &str| {
        if text == "*" {
            max
        } else {
            text.parse::<u32>().unwrap_or(0)
        }
    };
    let mut out = Vec::new();
    for part in set.split(',') {
        let (low, high) = match part.split_once(':') {
            Some((low, high)) => {
                let (low, high) = (value(low), value(high));
                if low <= high {
                    (low, high)
                } else {
                    (high, low)
                }
            }
            None => (value(part), value(part)),
        };
        out.extend(uids.iter().copied().filter(|uid| *uid >= low && *uid <= high));
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn uid_ranges(uids: &[u32]) -> String {
    super::sync::uid_set(uids)
}

fn write(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

fn serve_imap(stream: TcpStream, state: &Mutex<FakeImapState>) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    write(&mut writer, "* OK fake IMAP ready\r\n")?;
    let mut selected: Option<String> = None;
    let mut qresync = false;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let text = line.trim_end().to_string();
        let mut parts = text.splitn(2, ' ');
        let tag = parts.next().unwrap_or("*").to_string();
        let rest = parts.next().unwrap_or("").to_string();
        let all = words(&rest);
        let mut command = all.first().map(|word| word.to_ascii_uppercase()).unwrap_or_default();
        let mut args: Vec<String> = all.iter().skip(1).cloned().collect();
        let uid = command == "UID";
        if uid {
            command = args.first().map(|word| word.to_ascii_uppercase()).unwrap_or_default();
            args.remove(0);
        }
        lock(state).log.push(if command == "LOGIN" {
            "LOGIN".to_string()
        } else {
            rest.clone()
        });
        match command.as_str() {
            "CAPABILITY" => {
                let caps = lock(state).caps.join(" ");
                write(
                    &mut writer,
                    &format!("* CAPABILITY IMAP4rev1 {caps}\r\n{tag} OK CAPABILITY done\r\n"),
                )?;
            }
            "LOGIN" => {
                let ok = {
                    let state = lock(state);
                    args.first() == Some(&state.user) && args.get(1) == Some(&state.password)
                };
                if ok {
                    write(&mut writer, &format!("{tag} OK LOGIN completed\r\n"))?;
                } else {
                    write(
                        &mut writer,
                        &format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"),
                    )?;
                }
            }
            "ENABLE" => {
                let names: Vec<String> = args.iter().map(|name| name.to_ascii_uppercase()).collect();
                if names.iter().any(|name| name == "QRESYNC") {
                    qresync = true;
                }
                write(
                    &mut writer,
                    &format!("* ENABLED {}\r\n{tag} OK ENABLE done\r\n", names.join(" ")),
                )?;
            }
            "LIST" | "LSUB" => {
                let mut out = String::new();
                for folder in &lock(state).folders {
                    if command == "LSUB" && !folder.subscribed {
                        continue;
                    }
                    let attributes = if folder.selectable {
                        folder.attributes.clone()
                    } else {
                        format!("\\Noselect {}", folder.attributes)
                    };
                    out.push_str(&format!(
                        "* {command} ({}) \"/\" \"{}\"\r\n",
                        attributes.trim(),
                        folder.name
                    ));
                }
                out.push_str(&format!("{tag} OK {command} completed\r\n"));
                write(&mut writer, &out)?;
            }
            "STATUS" => {
                let name = args.first().cloned().unwrap_or_default();
                let mut state = lock(state);
                let condstore = state.has("CONDSTORE");
                let modseq = state.modseq;
                let reply = match state.folder(&name) {
                    Some(folder) => {
                        let unseen = folder
                            .messages
                            .iter()
                            .filter(|message| !message.flags.iter().any(|flag| flag == "\\Seen"))
                            .count();
                        let highest = if condstore {
                            format!(" HIGHESTMODSEQ {modseq}")
                        } else {
                            String::new()
                        };
                        format!(
                            "* STATUS \"{}\" (MESSAGES {} UIDNEXT {} UIDVALIDITY {} UNSEEN {unseen}{highest})\r\n{tag} OK STATUS done\r\n",
                            folder.name,
                            folder.messages.len(),
                            folder.uid_next,
                            folder.validity
                        )
                    }
                    None => format!("{tag} NO no such mailbox\r\n"),
                };
                drop(state);
                write(&mut writer, &reply)?;
            }
            "SELECT" | "EXAMINE" => {
                let name = args.first().cloned().unwrap_or_default();
                let mut state = lock(state);
                let condstore = state.has("CONDSTORE");
                let modseq = state.modseq;
                let reply = match state.folder(&name) {
                    Some(folder) if folder.selectable => {
                        selected = Some(folder.name.clone());
                        let mut out = String::new();
                        out.push_str("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n");
                        out.push_str(&format!("* {} EXISTS\r\n* 0 RECENT\r\n", folder.messages.len()));
                        out.push_str(&format!("* OK [UIDVALIDITY {}] UIDs valid\r\n", folder.validity));
                        out.push_str(&format!("* OK [UIDNEXT {}] Predicted next UID\r\n", folder.uid_next));
                        out.push_str(
                            "* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Limited\r\n",
                        );
                        if condstore {
                            out.push_str(&format!("* OK [HIGHESTMODSEQ {modseq}] Highest\r\n"));
                        }
                        out.push_str(&format!("{tag} OK [READ-WRITE] {command} completed\r\n"));
                        out
                    }
                    _ => {
                        selected = None;
                        format!("{tag} NO no such mailbox\r\n")
                    }
                };
                drop(state);
                write(&mut writer, &reply)?;
            }
            "CLOSE" | "UNSELECT" => {
                selected = None;
                write(&mut writer, &format!("{tag} OK {command} done\r\n"))?;
            }
            "SEARCH" => {
                let mut state = lock(state);
                let Some(folder) = selected.clone().and_then(|name| state.folder(&name).cloned()) else {
                    drop(state);
                    write(&mut writer, &format!("{tag} BAD no folder selected\r\n"))?;
                    continue;
                };
                drop(state);
                let uids: Vec<u32> = folder.messages.iter().map(|message| message.uid).collect();
                let mut matching: Vec<&FakeMessage> = folder.messages.iter().collect();
                let mut index = 0;
                while index < args.len() {
                    match args[index].to_ascii_uppercase().as_str() {
                        "ALL" => {}
                        "UID" => {
                            let set = select_set(args.get(index + 1).map_or("", String::as_str), &uids);
                            matching.retain(|message| set.contains(&message.uid));
                            index += 1;
                        }
                        "SINCE" => {
                            let date = args.get(index + 1).cloned().unwrap_or_default();
                            let since = jiff::civil::Date::strptime("%d-%b-%Y", &date)
                                .ok()
                                .and_then(|date| date.to_zoned(jiff::tz::TimeZone::UTC).ok())
                                .map_or(0, |zoned| zoned.timestamp().as_millisecond());
                            matching.retain(|message| message.internal_millis >= since);
                            index += 1;
                        }
                        "HEADER" => {
                            let field = args.get(index + 1).cloned().unwrap_or_default().to_ascii_lowercase();
                            let value = args.get(index + 2).cloned().unwrap_or_default().to_ascii_lowercase();
                            matching.retain(|message| {
                                String::from_utf8_lossy(&message.raw).lines().any(|line| {
                                    line.to_ascii_lowercase().starts_with(&format!("{field}:"))
                                        && line.to_ascii_lowercase().contains(&value)
                                })
                            });
                            index += 2;
                        }
                        _ => {}
                    }
                    index += 1;
                }
                let found: Vec<String> = matching.iter().map(|message| message.uid.to_string()).collect();
                write(
                    &mut writer,
                    &format!("* SEARCH {}\r\n{tag} OK SEARCH completed\r\n", found.join(" "))
                        .replace("SEARCH \r\n", "SEARCH\r\n"),
                )?;
            }
            "FETCH" => {
                let state_now = lock(state);
                let Some(folder) = selected
                    .as_ref()
                    .and_then(|name| state_now.folders.iter().find(|folder| &folder.name == name))
                    .cloned()
                else {
                    drop(state_now);
                    write(&mut writer, &format!("{tag} BAD no folder selected\r\n"))?;
                    continue;
                };
                drop(state_now);
                let uids: Vec<u32> = folder.messages.iter().map(|message| message.uid).collect();
                let set = select_set(args.first().map_or("", String::as_str), &uids);
                let items = args.get(1).cloned().unwrap_or_default().to_ascii_uppercase();
                let modifiers = args.get(2).cloned().unwrap_or_default().to_ascii_uppercase();
                let changed_since = modifiers
                    .split_whitespace()
                    .skip_while(|word| !word.contains("CHANGEDSINCE"))
                    .nth(1)
                    .and_then(|word| word.trim_matches(')').parse::<u64>().ok());
                let vanished = modifiers.contains("VANISHED") && qresync;
                let mut out: Vec<u8> = Vec::new();
                if let (Some(since), true) = (changed_since, vanished) {
                    let gone: Vec<u32> = folder
                        .expunged
                        .iter()
                        .filter(|(_, modseq)| *modseq > since)
                        .map(|(uid, _)| *uid)
                        .collect();
                    if !gone.is_empty() {
                        out.extend_from_slice(format!("* VANISHED (EARLIER) {}\r\n", uid_ranges(&gone)).as_bytes());
                    }
                }
                for (position, message) in folder.messages.iter().enumerate() {
                    if !set.contains(&message.uid) {
                        continue;
                    }
                    if changed_since.is_some_and(|since| message.modseq <= since) {
                        continue;
                    }
                    let mut parts: Vec<Vec<u8>> = vec![format!("UID {}", message.uid).into_bytes()];
                    if items.contains("FLAGS") {
                        parts.push(format!("FLAGS ({})", message.flags.join(" ")).into_bytes());
                    }
                    if changed_since.is_some() || items.contains("MODSEQ") {
                        parts.push(format!("MODSEQ ({})", message.modseq).into_bytes());
                    }
                    if items.contains("INTERNALDATE") {
                        parts.push(format!("INTERNALDATE \"{}\"", internal_date(message.internal_millis)).into_bytes());
                    }
                    if items.contains("RFC822.SIZE") {
                        parts.push(format!("RFC822.SIZE {}", message.raw.len()).into_bytes());
                    }
                    if items.contains("HEADER.FIELDS") {
                        let text = String::from_utf8_lossy(&message.raw).to_string();
                        let header = text
                            .split("\r\n")
                            .find(|line| line.to_ascii_lowercase().starts_with("message-id:"))
                            .map(|line| format!("{line}\r\n\r\n"))
                            .unwrap_or_else(|| "\r\n".to_string());
                        let mut part =
                            format!("BODY[HEADER.FIELDS (MESSAGE-ID)] {{{}}}\r\n", header.len()).into_bytes();
                        part.extend_from_slice(header.as_bytes());
                        parts.push(part);
                    } else if items.contains("BODY.PEEK[]") || items.contains("BODY[]") {
                        let mut part = format!("BODY[] {{{}}}\r\n", message.raw.len()).into_bytes();
                        part.extend_from_slice(&message.raw);
                        parts.push(part);
                    }
                    out.extend_from_slice(format!("* {} FETCH (", position + 1).as_bytes());
                    for (index, part) in parts.iter().enumerate() {
                        if index > 0 {
                            out.push(b' ');
                        }
                        out.extend_from_slice(part);
                    }
                    out.extend_from_slice(b")\r\n");
                }
                out.extend_from_slice(format!("{tag} OK FETCH completed\r\n").as_bytes());
                writer.write_all(&out)?;
                writer.flush()?;
            }
            "STORE" => {
                let mut state = lock(state);
                state.modseq += 1;
                let modseq = state.modseq;
                let Some(name) = selected.clone() else {
                    drop(state);
                    write(&mut writer, &format!("{tag} BAD no folder selected\r\n"))?;
                    continue;
                };
                let folder = state.folder(&name).expect("selected folder");
                let uids: Vec<u32> = folder.messages.iter().map(|message| message.uid).collect();
                let set = select_set(args.first().map_or("", String::as_str), &uids);
                let mode = args.get(1).cloned().unwrap_or_default().to_ascii_uppercase();
                let flags: Vec<String> = args
                    .get(2)
                    .cloned()
                    .unwrap_or_default()
                    .trim_matches(['(', ')'])
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                let mut out = String::new();
                for (position, message) in folder.messages.iter_mut().enumerate() {
                    if !set.contains(&message.uid) {
                        continue;
                    }
                    if mode.starts_with('+') {
                        for flag in &flags {
                            if !message.flags.iter().any(|known| known.eq_ignore_ascii_case(flag)) {
                                message.flags.push(flag.clone());
                            }
                        }
                    } else if mode.starts_with('-') {
                        message
                            .flags
                            .retain(|known| !flags.iter().any(|flag| flag.eq_ignore_ascii_case(known)));
                    } else {
                        message.flags.clone_from(&flags);
                    }
                    message.modseq = modseq;
                    if !mode.contains("SILENT") {
                        out.push_str(&format!(
                            "* {} FETCH (UID {} FLAGS ({}))\r\n",
                            position + 1,
                            message.uid,
                            message.flags.join(" ")
                        ));
                    }
                }
                drop(state);
                out.push_str(&format!("{tag} OK STORE completed\r\n"));
                write(&mut writer, &out)?;
            }
            "MOVE" | "COPY" => {
                let mut state = lock(state);
                state.modseq += 1;
                let modseq = state.modseq;
                let target_name = args.get(1).cloned().unwrap_or_default();
                let Some(name) = selected.clone() else {
                    drop(state);
                    write(&mut writer, &format!("{tag} BAD no folder selected\r\n"))?;
                    continue;
                };
                if state.folder(&target_name).is_none() {
                    drop(state);
                    write(&mut writer, &format!("{tag} NO [TRYCREATE] no such mailbox\r\n"))?;
                    continue;
                }
                let source = state.folder(&name).expect("selected folder");
                let uids: Vec<u32> = source.messages.iter().map(|message| message.uid).collect();
                let set = select_set(args.first().map_or("", String::as_str), &uids);
                let mut moving = Vec::new();
                let mut positions = Vec::new();
                for (position, message) in source.messages.iter().enumerate() {
                    if set.contains(&message.uid) {
                        moving.push(message.clone());
                        positions.push(position + 1);
                    }
                }
                if command == "MOVE" {
                    source.messages.retain(|message| !set.contains(&message.uid));
                    for uid in &set {
                        source.expunged.push((*uid, modseq));
                    }
                }
                let target = state.folder(&target_name).expect("target folder");
                let validity = target.validity;
                let mut new_uids = Vec::new();
                for mut message in moving {
                    message.uid = target.uid_next;
                    message.modseq = modseq;
                    target.uid_next += 1;
                    new_uids.push(message.uid);
                    target.messages.push(message);
                }
                drop(state);
                let copy_uid = format!("[COPYUID {validity} {} {}]", uid_ranges(&set), uid_ranges(&new_uids));
                let mut out = String::new();
                if command == "MOVE" {
                    out.push_str(&format!("* OK {copy_uid} Moved\r\n"));
                    if qresync {
                        out.push_str(&format!("* VANISHED {}\r\n", uid_ranges(&set)));
                    } else {
                        for position in positions.iter().rev() {
                            out.push_str(&format!("* {position} EXPUNGE\r\n"));
                        }
                    }
                    out.push_str(&format!("{tag} OK MOVE completed\r\n"));
                } else {
                    out.push_str(&format!("{tag} OK {copy_uid} COPY completed\r\n"));
                }
                write(&mut writer, &out)?;
            }
            "EXPUNGE" => {
                let mut state = lock(state);
                state.modseq += 1;
                let modseq = state.modseq;
                let Some(name) = selected.clone() else {
                    drop(state);
                    write(&mut writer, &format!("{tag} BAD no folder selected\r\n"))?;
                    continue;
                };
                let folder = state.folder(&name).expect("selected folder");
                let uids: Vec<u32> = folder.messages.iter().map(|message| message.uid).collect();
                let set = if uid {
                    select_set(args.first().map_or("", String::as_str), &uids)
                } else {
                    uids.clone()
                };
                let mut out = String::new();
                let mut position = 0;
                let mut kept = Vec::new();
                for message in folder.messages.drain(..) {
                    position += 1;
                    if set.contains(&message.uid) && message.flags.iter().any(|flag| flag == "\\Deleted") {
                        out.push_str(&format!("* {position} EXPUNGE\r\n"));
                        position -= 1;
                        folder.expunged.push((message.uid, modseq));
                    } else {
                        kept.push(message);
                    }
                }
                folder.messages = kept;
                drop(state);
                out.push_str(&format!("{tag} OK EXPUNGE completed\r\n"));
                write(&mut writer, &out)?;
            }
            "CREATE" => {
                let name = args.first().cloned().unwrap_or_default();
                let exists = lock(state).folder(&name).is_some();
                if exists {
                    write(&mut writer, &format!("{tag} NO [ALREADYEXISTS] Mailbox exists\r\n"))?;
                } else {
                    let mut state = lock(state);
                    let validity = state.next_validity;
                    state.next_validity += 1;
                    state.folders.push(FakeFolder {
                        name,
                        attributes: "\\HasNoChildren".to_string(),
                        validity,
                        uid_next: 1,
                        messages: Vec::new(),
                        expunged: Vec::new(),
                        subscribed: false,
                        selectable: true,
                    });
                    drop(state);
                    write(&mut writer, &format!("{tag} OK CREATE completed\r\n"))?;
                }
            }
            "RENAME" => {
                let from = args.first().cloned().unwrap_or_default();
                let to = args.get(1).cloned().unwrap_or_default();
                let mut state = lock(state);
                let renamed = match state.folder(&from) {
                    Some(folder) => {
                        folder.name = to;
                        true
                    }
                    None => false,
                };
                drop(state);
                if renamed {
                    write(&mut writer, &format!("{tag} OK RENAME completed\r\n"))?;
                } else {
                    write(&mut writer, &format!("{tag} NO no such mailbox\r\n"))?;
                }
            }
            "SUBSCRIBE" | "UNSUBSCRIBE" => {
                let name = args.first().cloned().unwrap_or_default();
                if let Some(folder) = lock(state).folder(&name) {
                    folder.subscribed = command == "SUBSCRIBE";
                }
                write(&mut writer, &format!("{tag} OK {command} completed\r\n"))?;
            }
            "APPEND" => {
                let name = args.first().cloned().unwrap_or_default();
                let flags: Vec<String> = args
                    .get(1)
                    .filter(|word| word.starts_with('('))
                    .map(|word| {
                        word.trim_matches(['(', ')'])
                            .split_whitespace()
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let size: usize = text
                    .rsplit_once('{')
                    .and_then(|(_, size)| size.trim_end_matches('}').parse().ok())
                    .unwrap_or(0);
                write(&mut writer, "+ Ready for literal data\r\n")?;
                let mut bytes = vec![0u8; size];
                reader.read_exact(&mut bytes)?;
                let mut end = String::new();
                reader.read_line(&mut end)?;
                let mut state = lock(state);
                state.modseq += 1;
                let modseq = state.modseq;
                let has_uidplus = state.has("UIDPLUS");
                let reply = match state.folder(&name) {
                    Some(folder) => {
                        let uid = folder.uid_next;
                        folder.uid_next += 1;
                        folder.messages.push(FakeMessage {
                            uid,
                            flags,
                            internal_millis: jiff::Timestamp::now().as_millisecond(),
                            raw: bytes,
                            modseq,
                        });
                        if has_uidplus {
                            format!("{tag} OK [APPENDUID {} {uid}] APPEND completed\r\n", folder.validity)
                        } else {
                            format!("{tag} OK APPEND completed\r\n")
                        }
                    }
                    None => format!("{tag} NO [TRYCREATE] no such mailbox\r\n"),
                };
                drop(state);
                write(&mut writer, &reply)?;
            }
            "NOOP" | "CHECK" => write(&mut writer, &format!("{tag} OK {command} completed\r\n"))?,
            "IDLE" => {
                let count = |state: &Mutex<FakeImapState>| {
                    selected
                        .as_ref()
                        .and_then(|name| {
                            lock(state)
                                .folders
                                .iter()
                                .find(|folder| &folder.name == name)
                                .map(|folder| folder.messages.len())
                        })
                        .unwrap_or(0)
                };
                write(&mut writer, "+ idling\r\n")?;
                let mut known = count(state);
                reader.get_ref().set_read_timeout(Some(Duration::from_millis(50)))?;
                let mut done = String::new();
                loop {
                    match reader.read_line(&mut done) {
                        Ok(0) => return Ok(()),
                        Ok(_) => {
                            if done.trim().eq_ignore_ascii_case("DONE") {
                                break;
                            }
                            done.clear();
                        }
                        Err(err)
                            if matches!(
                                err.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(err) => return Err(err),
                    }
                    let now = count(state);
                    if now != known {
                        write(&mut writer, &format!("* {now} EXISTS\r\n"))?;
                        known = now;
                    }
                }
                reader.get_ref().set_read_timeout(None)?;
                write(&mut writer, &format!("{tag} OK IDLE terminated\r\n"))?;
            }
            "LOGOUT" => {
                write(
                    &mut writer,
                    &format!("* BYE logging out\r\n{tag} OK LOGOUT completed\r\n"),
                )?;
                return Ok(());
            }
            _ => write(&mut writer, &format!("{tag} BAD unknown command {command}\r\n"))?,
        }
    }
}

// ---- POP3 ----

/// Each message of the fake POP3 server: its UIDL, bytes, and whether it is deleted.
type PopMessages = Arc<Mutex<Vec<(String, Vec<u8>, bool)>>>;

pub(super) struct FakePop {
    pub(super) port: u16,
    pub(super) messages: PopMessages,
    pub(super) log: Arc<Mutex<Vec<String>>>,
}

impl FakePop {
    pub(super) fn start() -> Self {
        let messages: PopMessages = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shared, shared_log) = (Arc::clone(&messages), Arc::clone(&log));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let _ = serve_pop(stream, &shared, &shared_log);
            }
        });
        Self { port, messages, log }
    }

    pub(super) fn add(&self, uidl: &str, raw: &str) {
        lock(&self.messages).push((
            uidl.to_string(),
            raw.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes(),
            false,
        ));
    }

    pub(super) fn uidls(&self) -> Vec<String> {
        lock(&self.messages).iter().map(|(uidl, _, _)| uidl.clone()).collect()
    }
}

fn serve_pop(
    stream: TcpStream,
    messages: &Mutex<Vec<(String, Vec<u8>, bool)>>,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    write(&mut writer, "+OK fake POP3 ready\r\n")?;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let text = line.trim_end().to_string();
        lock(log).push(text.clone());
        let mut words = text.split_whitespace();
        let command = words.next().unwrap_or("").to_ascii_uppercase();
        let argument: usize = words.next().and_then(|word| word.parse().ok()).unwrap_or(0);
        match command.as_str() {
            "USER" => write(&mut writer, "+OK\r\n")?,
            "PASS" => {
                if text.ends_with(" pop-password") {
                    write(&mut writer, "+OK signed in\r\n")?;
                } else {
                    write(&mut writer, "-ERR [AUTH] invalid password\r\n")?;
                }
            }
            "UIDL" | "LIST" => {
                let mut out = "+OK\r\n".to_string();
                for (index, (uidl, raw, deleted)) in lock(messages).iter().enumerate() {
                    if *deleted {
                        continue;
                    }
                    if command == "UIDL" {
                        out.push_str(&format!("{} {uidl}\r\n", index + 1));
                    } else {
                        out.push_str(&format!("{} {}\r\n", index + 1, raw.len()));
                    }
                }
                out.push_str(".\r\n");
                write(&mut writer, &out)?;
            }
            "RETR" => {
                let found = lock(messages)
                    .get(argument.saturating_sub(1))
                    .map(|(_, raw, _)| raw.clone());
                match found {
                    Some(raw) => {
                        let mut out = b"+OK\r\n".to_vec();
                        for line in String::from_utf8_lossy(&raw).split("\r\n") {
                            if line.starts_with('.') {
                                out.push(b'.');
                            }
                            out.extend_from_slice(line.as_bytes());
                            out.extend_from_slice(b"\r\n");
                        }
                        out.extend_from_slice(b".\r\n");
                        writer.write_all(&out)?;
                        writer.flush()?;
                    }
                    None => write(&mut writer, "-ERR no such message\r\n")?,
                }
            }
            "DELE" => {
                if let Some(entry) = lock(messages).get_mut(argument.saturating_sub(1)) {
                    entry.2 = true;
                }
                write(&mut writer, "+OK deleted\r\n")?;
            }
            "QUIT" => {
                lock(messages).retain(|(_, _, deleted)| !deleted);
                write(&mut writer, "+OK bye\r\n")?;
                return Ok(());
            }
            _ => write(&mut writer, "+OK\r\n")?,
        }
    }
}

// ---- HTTP ----

/// An HTTP server answering GET by path from `pages`; 404 otherwise. Returns its port.
pub(super) fn fake_http(pages: HashMap<String, String>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let pages = pages.clone();
            std::thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                if reader.read_line(&mut request).unwrap_or(0) == 0 {
                    return;
                }
                let mut header = String::new();
                while reader.read_line(&mut header).unwrap_or(0) > 0 && header.trim() != "" {
                    header.clear();
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let path = path.split('?').next().unwrap_or(path).to_string();
                let response = match pages.get(&path) {
                    Some(body) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = writer.write_all(response.as_bytes());
                let _ = writer.flush();
            });
        }
    });
    port
}

// ---- DNS ----

/// A DNS server answering SRV queries from `records` (name to priority, weight, port, target). Returns its port.
pub(super) fn fake_dns(records: HashMap<String, (u16, u16, u16, String)>) -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 1500];
        loop {
            let Ok((read, from)) = socket.recv_from(&mut buffer) else {
                return;
            };
            let query = &buffer[..read];
            if query.len() < 12 {
                continue;
            }
            // the question name, then type and class
            let mut offset = 12;
            let mut labels = Vec::new();
            while offset < query.len() && query[offset] != 0 {
                let length = usize::from(query[offset]);
                labels.push(String::from_utf8_lossy(&query[offset + 1..offset + 1 + length]).to_string());
                offset += 1 + length;
            }
            let question_end = offset + 5;
            let name = labels.join(".");
            let mut answer = query[..question_end].to_vec();
            answer[2] = 0x81;
            answer[3] = 0x80;
            if let Some((priority, weight, port, target)) = records.get(&name) {
                answer[6] = 0;
                answer[7] = 1;
                let mut target_bytes = Vec::new();
                for label in target.split('.') {
                    target_bytes.push(u8::try_from(label.len()).unwrap());
                    target_bytes.extend_from_slice(label.as_bytes());
                }
                target_bytes.push(0);
                answer.extend_from_slice(&[0xC0, 0x0C, 0x00, 0x21, 0x00, 0x01, 0, 0, 0, 60]);
                answer.extend_from_slice(&u16::try_from(6 + target_bytes.len()).unwrap().to_be_bytes());
                answer.extend_from_slice(&priority.to_be_bytes());
                answer.extend_from_slice(&weight.to_be_bytes());
                answer.extend_from_slice(&port.to_be_bytes());
                answer.extend_from_slice(&target_bytes);
            } else {
                answer[3] = 0x83;
            }
            let _ = socket.send_to(&answer, from);
        }
    });
    port
}
