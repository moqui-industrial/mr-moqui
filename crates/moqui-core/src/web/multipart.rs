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

//! `multipart/form-data` bodies (RFC 7578): text fields and uploaded files.
//!
//! The body is already in memory, read by the host up to the upload cap, so this is a parser of a byte
//! slice and has no stream to suspend. A file part is kept as an [`UploadedFile`], the value a form
//! `file` widget's field holds in `ec.web.multiPartParameters`.

use moqui_lib::object::{HostObject, Value};

const MAX_PARTS: usize = 2000;
const MAX_PART_HEADERS: usize = 16 * 1024;

/// Windows PE, Linux ELF, Java class, macOS Mach-O and byte-swapped Mach-O.
const EXECUTABLE_MAGIC: [&[u8]; 7] = [
    b"MZ",
    &[0x7f, 0x45, 0x4c, 0x46],
    &[0xca, 0xfe, 0xba, 0xbe],
    &[0xfe, 0xed, 0xfa, 0xce],
    &[0xfe, 0xed, 0xfa, 0xcf],
    &[0xce, 0xfa, 0xed, 0xfe],
    &[0xcf, 0xfa, 0xed, 0xfe],
];

/// An uploaded file: the part's form field, its file name and type, and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadedFile {
    pub field_name: String,
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl UploadedFile {
    /// Whether the first bytes are those of a program (see `WebUtilities.isExecutable`).
    #[must_use]
    pub fn is_executable(&self) -> bool {
        is_executable(&self.bytes)
    }

    #[must_use]
    pub fn into_value(self) -> Value {
        Value::Object(HostObject::new(self))
    }
}

/// A shebang or one of the program headers. A ZIP is not a program.
#[must_use]
pub fn is_executable(bytes: &[u8]) -> bool {
    bytes.starts_with(b"#!") || EXECUTABLE_MAGIC.iter().any(|magic| bytes.starts_with(magic))
}

/// The parts of a body, in order.
#[derive(Debug, Default)]
pub struct Parsed {
    pub fields: Vec<(String, String)>,
    pub files: Vec<UploadedFile>,
}

/// Why a body could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum MultipartError {
    /// No boundary in the content type, or the body does not follow it.
    Malformed(String),
    /// More parts than the parser allows.
    TooManyParts,
}

impl std::fmt::Display for MultipartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "multipart body is not valid: {why}"),
            Self::TooManyParts => write!(f, "multipart body has more than {MAX_PARTS} parts"),
        }
    }
}

/// The boundary from a `Content-Type` value, with or without quotes.
#[must_use]
pub fn boundary_of(content_type: &str) -> Option<String> {
    for part in content_type.split(';').skip(1) {
        let (key, value) = part.trim().split_once('=')?;
        if key.trim().eq_ignore_ascii_case("boundary") {
            let value = value.trim().trim_matches('"');
            return Some(value.to_string()).filter(|text| !text.is_empty() && text.len() <= 200);
        }
    }
    None
}

/// Read every part of `body`. Text fields are decoded as UTF-8, with replacement for bad bytes.
///
/// # Errors
///
/// Returns an error when there is no boundary, a part has no header end, or the closing boundary is missing.
pub fn parse(content_type: &str, body: &[u8]) -> Result<Parsed, MultipartError> {
    let boundary =
        boundary_of(content_type).ok_or_else(|| MultipartError::Malformed("no boundary in the content type".into()))?;
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut position = find(body, delimiter, 0).ok_or_else(|| MultipartError::Malformed("no first boundary".into()))?;
    let mut parsed = Parsed::default();
    let mut count = 0;
    loop {
        position += delimiter.len();
        if body[position..].starts_with(b"--") {
            return Ok(parsed);
        }
        // The rest of the boundary line is white space, then the line break.
        let line_end = find(body, b"\r\n", position)
            .ok_or_else(|| MultipartError::Malformed("a boundary line has no end".into()))?;
        if body[position..line_end]
            .iter()
            .any(|byte| !matches!(byte, b' ' | b'\t'))
        {
            return Err(MultipartError::Malformed("text follows a boundary".into()));
        }
        let start = line_end + 2;
        // Search from the boundary line's own break: an empty header block is a blank line right after it.
        let header_end = find(body, b"\r\n\r\n", line_end)
            .ok_or_else(|| MultipartError::Malformed("a part has no end of headers".into()))?;
        let headers = if header_end < start {
            &body[0..0]
        } else {
            &body[start..header_end]
        };
        if headers.len() > MAX_PART_HEADERS {
            return Err(MultipartError::Malformed("part headers are too large".into()));
        }
        let data_start = header_end + 4;
        let mut next = b"\r\n".to_vec();
        next.extend_from_slice(delimiter);
        let data_end = find(body, &next, data_start.saturating_sub(2))
            .ok_or_else(|| MultipartError::Malformed("no closing boundary".into()))?;
        let data = &body[data_start.min(data_end)..data_end];
        count += 1;
        if count > MAX_PARTS {
            return Err(MultipartError::TooManyParts);
        }
        add_part(&mut parsed, headers, data)?;
        position = data_end + 2;
    }
}

fn add_part(parsed: &mut Parsed, headers: &[u8], data: &[u8]) -> Result<(), MultipartError> {
    let text = String::from_utf8_lossy(headers);
    let mut name = None;
    let mut file_name = None;
    let mut content_type = String::new();
    for line in text.split("\r\n") {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("content-disposition") {
            name = disposition_param(value, "name");
            file_name = disposition_param(value, "filename");
        } else if key.trim().eq_ignore_ascii_case("content-type") {
            content_type = value.trim().to_string();
        }
    }
    let name = name.ok_or_else(|| MultipartError::Malformed("a part has no name".into()))?;
    match file_name {
        // A file input left empty posts a part with an empty file name.
        Some(file_name) if file_name.is_empty() && data.is_empty() => {}
        Some(file_name) => parsed.files.push(UploadedFile {
            field_name: name,
            file_name: base_name(&file_name),
            content_type: if content_type.is_empty() {
                "application/octet-stream".to_string()
            } else {
                content_type
            },
            bytes: data.to_vec(),
        }),
        None => parsed.fields.push((name, String::from_utf8_lossy(data).into_owned())),
    }
    Ok(())
}

/// A file name without the directories some browsers send.
fn base_name(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).to_string()
}

/// `name="x"` or `name=x` in a `Content-Disposition` value. A quoted value may hold `\"` and `\\`.
fn disposition_param(value: &str, wanted: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        // A parameter starts after a semicolon.
        let semicolon = value[index..].find(';')?;
        index += semicolon + 1;
        let rest = value[index..].trim_start();
        index = value.len() - rest.len();
        let equals = rest.find('=')?;
        let key = rest[..equals].trim();
        let after = &rest[equals + 1..];
        let (parsed, used) = if let Some(quoted) = after.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = quoted.char_indices();
            let mut end = quoted.len();
            while let Some((at, ch)) = chars.next() {
                match ch {
                    '\\' => match quoted[at + 1..].chars().next() {
                        Some(escaped @ ('"' | '\\')) => {
                            chars.next();
                            out.push(escaped);
                        }
                        _ => out.push('\\'),
                    },
                    '"' => {
                        end = at + 1;
                        break;
                    }
                    other => out.push(other),
                }
            }
            (out, 1 + end)
        } else {
            let end = after.find(';').unwrap_or(after.len());
            (after[..end].trim().to_string(), end)
        };
        if key.eq_ignore_ascii_case(wanted) {
            return Some(parsed);
        }
        index += equals + 1 + used;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > haystack.len() || haystack.len() - from < needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (headers, data) in parts {
            out.extend_from_slice(b"--XyZ\r\n");
            out.extend_from_slice(headers.as_bytes());
            out.extend_from_slice(b"\r\n\r\n");
            out.extend_from_slice(data);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"--XyZ--\r\n");
        out
    }

    const TYPE: &str = "multipart/form-data; boundary=XyZ";

    #[test]
    fn fields_and_files_are_separated_and_keep_their_bytes() {
        let data = body(&[
            ("Content-Disposition: form-data; name=\"title\"", "Hello \u{e9}".as_bytes()),
            (
                "Content-Disposition: form-data; name=\"doc\"; filename=\"C:\\tmp\\a b.bin\"\r\nContent-Type: image/png",
                &[0, 1, 2, 13, 10, 45, 45, 255],
            ),
            ("Content-Disposition: form-data; name=\"empty\"", b""),
            ("Content-Disposition: form-data; name=\"title\"", b"again"),
        ]);
        let parsed = parse(TYPE, &data).unwrap();
        assert_eq!(
            parsed.fields,
            vec![
                ("title".to_string(), "Hello \u{e9}".to_string()),
                ("empty".to_string(), String::new()),
                ("title".to_string(), "again".to_string())
            ]
        );
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.field_name, "doc");
        assert_eq!(file.file_name, "a b.bin");
        assert_eq!(file.content_type, "image/png");
        assert_eq!(file.bytes, vec![0, 1, 2, 13, 10, 45, 45, 255]);
    }

    #[test]
    fn an_empty_file_input_posts_nothing_and_quotes_in_names_are_unescaped() {
        let data = body(&[
            (
                "Content-Disposition: form-data; name=\"doc\"; filename=\"\"\r\nContent-Type: application/octet-stream",
                b"",
            ),
            (
                "Content-Disposition: form-data; name=\"a\\\"b\"; filename=\"x\\\\y.txt\"",
                b"t",
            ),
        ]);
        let parsed = parse("multipart/form-data; boundary=\"XyZ\"", &data).unwrap();
        assert!(parsed.fields.is_empty());
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].field_name, "a\"b");
        assert_eq!(parsed.files[0].file_name, "y.txt");
        assert_eq!(parsed.files[0].content_type, "application/octet-stream");
    }

    #[test]
    fn a_part_that_looks_like_a_boundary_inside_data_does_not_split() {
        let data = body(&[(
            "Content-Disposition: form-data; name=\"t\"",
            b"line\r\n--Xy not a boundary\r\nend",
        )]);
        let parsed = parse(TYPE, &data).unwrap();
        assert_eq!(parsed.fields[0].1, "line\r\n--Xy not a boundary\r\nend");
    }

    #[test]
    fn bad_bodies_are_errors_not_panics() {
        assert!(parse("multipart/form-data", b"--x\r\n").is_err());
        assert!(parse(TYPE, b"").is_err());
        assert!(parse(
            TYPE,
            b"--XyZ\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\nno end"
        )
        .is_err());
        assert!(parse(TYPE, b"--XyZ\r\nContent-Type: text/plain\r\n\r\ndata\r\n--XyZ--").is_err());
        assert!(parse(TYPE, b"--XyZ junk\r\n").is_err());
        assert!(parse(TYPE, b"--XyZ\r\nContent-Disposition: form-data; name=\"a\"").is_err());
        // The closing marker alone is an empty form.
        assert!(parse(TYPE, b"--XyZ--\r\n").unwrap().fields.is_empty());
    }

    #[test]
    fn the_boundary_is_read_with_or_without_quotes() {
        assert_eq!(boundary_of("multipart/form-data; boundary=abc").as_deref(), Some("abc"));
        assert_eq!(
            boundary_of("multipart/form-data; charset=x; boundary=\"a b\"").as_deref(),
            Some("a b")
        );
        assert_eq!(boundary_of("multipart/form-data"), None);
        assert_eq!(boundary_of("multipart/form-data; boundary="), None);
    }

    #[test]
    fn programs_are_recognised_by_their_first_bytes() {
        assert!(is_executable(b"MZ\x90\x00"));
        assert!(is_executable(&[0x7f, 0x45, 0x4c, 0x46, 2]));
        assert!(is_executable(&[0xca, 0xfe, 0xba, 0xbe]));
        assert!(is_executable(&[0xcf, 0xfa, 0xed, 0xfe]));
        assert!(is_executable(b"#!/bin/sh\n"));
        assert!(!is_executable(b"PK\x03\x04"));
        assert!(!is_executable(b"plain text"));
        assert!(!is_executable(b""));
    }
}
