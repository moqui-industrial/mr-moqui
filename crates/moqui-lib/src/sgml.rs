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

//! `SimpleSgmlReader`: a simple SGML reader onto [`MNode`]. It does not validate, does not
//! support attributes, and reads the colon separated header of OFX version 1.
//!
//! An element whose start tag is followed by text has that text as its value and no children.
//! Its end tag is optional, as OFX leaves it off. This is not the framework XML parser.

use indexmap::IndexMap;
use jiff::civil::DateTime;
use jiff::tz::{Offset, TimeZone};
use jiff::Timestamp;

use crate::error::Error;
use crate::node::MNode;
use crate::string::decode_from_xml;

/// The header and the root element of one SGML document.
pub struct SimpleSgmlReader {
    header: Option<IndexMap<String, String>>,
    root: Option<MNode>,
}

impl SimpleSgmlReader {
    /// Read the document. Text before the first `<` is the header, one `key:value` per line.
    ///
    /// # Errors
    ///
    /// A `<` as the last character, which Java reports as an index out of bounds.
    pub fn new(sgml: &str) -> Result<Self, Error> {
        let mut reader = Self {
            header: None,
            root: None,
        };
        if sgml.is_empty() {
            return Ok(reader);
        }
        // With no `<` there is no element, and Java returns no root either.
        let Some(first_lt) = sgml.find('<') else {
            return Ok(reader);
        };
        if first_lt > 0 {
            let mut header = IndexMap::new();
            let header_str = java_trim(&sgml[..first_lt]);
            for line in header_str.split(is_regex_space) {
                if line.is_empty() {
                    continue;
                }
                if let Some(colon) = line.find(':') {
                    if colon > 0 && line.len() > colon + 1 {
                        header.insert(
                            java_trim(&line[..colon]).to_string(),
                            java_trim(&line[colon + 1..]).to_string(),
                        );
                    }
                }
            }
            reader.header = Some(header);
        }
        let mut cursor = Cursor { sgml, pos: first_lt };
        reader.root = cursor.start_root()?;
        Ok(reader)
    }

    /// Header keys and values, or `None` when the document starts with `<`.
    pub fn header(&self) -> Option<&IndexMap<String, String>> {
        self.header.as_ref()
    }

    pub fn root(&self) -> Option<&MNode> {
        self.root.as_ref()
    }
}

struct Cursor<'a> {
    sgml: &'a str,
    pos: usize,
}

impl Cursor<'_> {
    fn len(&self) -> usize {
        self.sgml.len()
    }

    fn find(&self, ch: char, from: usize) -> Option<usize> {
        self.sgml.get(from..)?.find(ch).map(|index| index + from)
    }

    fn byte(&self, index: usize) -> Result<u8, Error> {
        self.sgml
            .as_bytes()
            .get(index)
            .copied()
            .ok_or_else(|| Error::new(format!("SGML ends inside a tag at {index}")))
    }

    fn start_root(&mut self) -> Result<Option<MNode>, Error> {
        // Move to the first character of the element name, then find the end of the tag.
        self.pos += 1;
        let Some(gt) = self.find('>', self.pos) else {
            return Ok(None);
        };
        let root = MNode::new(java_trim(&self.sgml[self.pos..gt]));
        self.pos = gt + 1;
        // The root element has child elements, not a value.
        let Some(next_lt) = self.find('<', self.pos) else {
            return Ok(None);
        };
        self.pos = next_lt;
        self.handle_children(&root)?;
        Ok(Some(root))
    }

    fn handle_children(&mut self, parent: &MNode) -> Result<(), Error> {
        while self.pos < self.len() && self.byte(self.pos)? == b'<' && self.byte(self.pos + 1)? != b'/' {
            self.start_node(parent)?;
        }
        if self.pos < self.len() && self.byte(self.pos)? == b'<' && self.byte(self.pos + 1)? == b'/' {
            if let Some(close_gt) = self.find('>', self.pos + 2) {
                self.pos = close_gt + 1;
            }
            // After the end tag. The next element, if any, is a sibling.
            self.pos = self.find('<', self.pos).unwrap_or(self.len());
        }
        Ok(())
    }

    fn start_node(&mut self, parent: &MNode) -> Result<(), Error> {
        self.pos += 1;
        let Some(gt) = self.find('>', self.pos) else {
            return Ok(());
        };
        let node = parent.append(java_trim(&self.sgml[self.pos..gt]));
        self.pos = gt + 1;
        self.skip_whitespace();
        if self.pos == self.len() {
            return Ok(());
        }
        // Text up to the next element or end tag is the value.
        let lt = self.find('<', self.pos).unwrap_or(self.len());
        let mut has_value = false;
        if lt > self.pos {
            let value = java_trim(&self.sgml[self.pos..lt]);
            if !value.is_empty() {
                node.set_text(Some(decode_from_xml(value)));
                has_value = true;
            }
            self.pos = lt;
        }
        if self.pos == self.len() {
            return Ok(());
        }
        if !has_value {
            self.handle_children(&node)?;
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.sgml.get(self.pos..).and_then(|rest| rest.chars().next()) {
            if !is_java_whitespace(ch) {
                break;
            }
            self.pos += ch.len_utf8();
        }
    }
}

/// `String.trim`: characters up to U+0020 at either end.
fn java_trim(text: &str) -> &str {
    text.trim_matches(|ch: char| ch <= ' ')
}

/// The regex `\s`.
fn is_regex_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

/// `Character.isWhitespace`: no non-breaking spaces, and the four separators U+001C to U+001F.
fn is_java_whitespace(ch: char) -> bool {
    match ch {
        '\u{a0}' | '\u{2007}' | '\u{202f}' | '\u{85}' => false,
        '\u{1c}'..='\u{1f}' => true,
        _ => ch.is_whitespace(),
    }
}

/// An OFX date and time: `yyyyMMdd`, `yyyyMMddHHmmss`, or `yyyyMMddHHmmss.SSS`, in GMT, or any of
/// those followed by a zone such as `[-5:EST]`. `None` when the text does not parse.
pub fn parse_ofx_date_time(ofx: &str) -> Option<Timestamp> {
    let (date_time, zone) = if ofx.chars().count() <= 18 {
        (pad_ofx(ofx), "+0000".to_string())
    } else {
        // Strip the zone and make an RFC 822 offset such as -0500.
        let open = ofx.find('[')?;
        let colon = ofx[open..].find(':')? + open;
        let mut zone: String = ofx[open + 1..colon].to_string();
        if zone.chars().next()?.is_ascii_digit() {
            zone.insert(0, '+');
        }
        if zone.chars().count() == 2 {
            zone.insert(1, '0');
        }
        (pad_ofx(&ofx[..open]), format!("{zone}00"))
    };
    let civil = parse_civil(&date_time)?;
    let offset = parse_rfc822_offset(&zone)?;
    civil
        .to_zoned(TimeZone::fixed(offset))
        .ok()
        .map(|zoned| zoned.timestamp())
}

fn pad_ofx(text: &str) -> String {
    let mut out = text.to_string();
    while out.chars().count() < 14 {
        out.push('0');
    }
    if out.chars().count() < 15 {
        out.push('.');
    }
    while out.chars().count() < 18 {
        out.push('0');
    }
    out
}

/// `yyyyMMddHHmmss.SSS`, exactly.
fn parse_civil(text: &str) -> Option<DateTime> {
    let bytes = text.as_bytes();
    if bytes.len() != 18 || bytes[14] != b'.' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i32> {
        let part = text.get(range)?;
        if !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let year = i16::try_from(digits(0..4)?).ok()?;
    let month = i8::try_from(digits(4..6)?).ok()?;
    let day = i8::try_from(digits(6..8)?).ok()?;
    let hour = i8::try_from(digits(8..10)?).ok()?;
    let minute = i8::try_from(digits(10..12)?).ok()?;
    let second = i8::try_from(digits(12..14)?).ok()?;
    let millis = digits(15..18)?;
    DateTime::new(year, month, day, hour, minute, second, millis * 1_000_000).ok()
}

/// `Z` in `SimpleDateFormat`: a sign and four digits.
fn parse_rfc822_offset(text: &str) -> Option<Offset> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 || !bytes[1..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i32 = text[1..3].parse().ok()?;
    let minutes: i32 = text[3..5].parse().ok()?;
    if minutes >= 60 {
        return None;
    }
    Offset::from_seconds(sign * (hours * 3600 + minutes * 60)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFX: &str = "OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\nSECURITY:NONE\r\nENCODING:USASCII\r\n\
        CHARSET:1252\r\nCOMPRESSION:NONE\r\nOLDFILEUID:NONE\r\nNEWFILEUID:NONE\r\n\r\n\
        <OFX>\r\n<SIGNONMSGSRSV1>\r\n<SONRS>\r\n<STATUS>\r\n<CODE>0\r\n<SEVERITY>INFO\r\n</STATUS>\r\n\
        <DTSERVER>20231015120000.000[-5:EST]\r\n<LANGUAGE>ENG\r\n</SONRS>\r\n</SIGNONMSGSRSV1>\r\n\
        <BANKMSGSRSV1>\r\n<STMTTRNRS>\r\n<TRNUID>1\r\n<STMTRS>\r\n<CURDEF>USD\r\n<BANKTRANLIST>\r\n\
        <STMTTRN>\r\n<TRNTYPE>DEBIT\r\n<TRNAMT>-12.50\r\n<NAME>Joe &amp; Sons\r\n</STMTTRN>\r\n\
        <STMTTRN>\r\n<TRNTYPE>CREDIT\r\n<TRNAMT>100.00\r\n</STMTTRN>\r\n</BANKTRANLIST>\r\n</STMTRS>\r\n\
        </STMTTRNRS>\r\n</BANKMSGSRSV1>\r\n</OFX>\r\n";

    fn child_text(node: &MNode, name: &str) -> Option<String> {
        node.first_named(name).and_then(|child| child.text())
    }

    #[test]
    fn ofx_header_and_elements_without_end_tags() {
        let reader = SimpleSgmlReader::new(OFX).unwrap();
        let header = reader.header().unwrap();
        assert_eq!(header.get("OFXHEADER").map(String::as_str), Some("100"));
        assert_eq!(header.get("NEWFILEUID").map(String::as_str), Some("NONE"));
        assert_eq!(header.len(), 9);

        let root = reader.root().unwrap();
        assert_eq!(root.name(), "OFX");
        let sonrs = root
            .first_named("SIGNONMSGSRSV1")
            .unwrap()
            .first_named("SONRS")
            .unwrap();
        let status = sonrs.first_named("STATUS").unwrap();
        assert_eq!(child_text(&status, "CODE").as_deref(), Some("0"));
        assert_eq!(child_text(&status, "SEVERITY").as_deref(), Some("INFO"));
        // After `</STATUS>` the next element is a sibling of STATUS.
        assert_eq!(child_text(&sonrs, "LANGUAGE").as_deref(), Some("ENG"));

        let list = root
            .first_named("BANKMSGSRSV1")
            .and_then(|node| node.first_named("STMTTRNRS"))
            .and_then(|node| node.first_named("STMTRS"))
            .and_then(|node| node.first_named("BANKTRANLIST"))
            .unwrap();
        let transactions = list.children_named("STMTTRN");
        assert_eq!(transactions.len(), 2);
        assert_eq!(child_text(&transactions[0], "TRNAMT").as_deref(), Some("-12.50"));
        assert_eq!(child_text(&transactions[0], "NAME").as_deref(), Some("Joe & Sons"));
        assert_eq!(child_text(&transactions[1], "TRNTYPE").as_deref(), Some("CREDIT"));
        assert!(transactions[0]
            .children()
            .iter()
            .all(|child| child.children().is_empty()));
    }

    #[test]
    fn no_header_closed_values_and_degenerate_text() {
        let reader = SimpleSgmlReader::new("<A><B>one<C><D>two</C><E>three</A>").unwrap();
        assert!(reader.header().is_none());
        let root = reader.root().unwrap();
        let names: Vec<String> = root.children().iter().map(MNode::name).collect();
        assert_eq!(names, ["B", "C", "E"]);
        assert_eq!(child_text(&root.first_named("C").unwrap(), "D").as_deref(), Some("two"));
        assert_eq!(child_text(root, "E").as_deref(), Some("three"));

        // An end tag after a value closes the parent, as in Java, so the root stops at B.
        let closed = SimpleSgmlReader::new("<A><B>one</B><C>two</A>").unwrap();
        let names: Vec<String> = closed.root().unwrap().children().iter().map(MNode::name).collect();
        assert_eq!(names, ["B"]);

        assert!(SimpleSgmlReader::new("").unwrap().root().is_none());
        assert!(SimpleSgmlReader::new("no elements here").unwrap().root().is_none());
        // A root with nothing after its start tag.
        assert!(SimpleSgmlReader::new("<ROOT>").unwrap().root().is_none());
        assert!(SimpleSgmlReader::new("<ROOT><A>x<").is_err());
    }

    #[test]
    fn ofx_date_times() {
        let utc = |text: &str| parse_ofx_date_time(text).map(|stamp| stamp.to_string());
        assert_eq!(utc("20231015").as_deref(), Some("2023-10-15T00:00:00Z"));
        assert_eq!(utc("20231015123456").as_deref(), Some("2023-10-15T12:34:56Z"));
        assert_eq!(utc("20231015123456.789").as_deref(), Some("2023-10-15T12:34:56.789Z"));
        assert_eq!(
            utc("20231015120000.000[-5:EST]").as_deref(),
            Some("2023-10-15T17:00:00Z")
        );
        assert_eq!(
            utc("20231015120000.000[+10:AEST]").as_deref(),
            Some("2023-10-15T02:00:00Z")
        );
        assert_eq!(utc("20231015120000[0:GMT]").as_deref(), Some("2023-10-15T12:00:00Z"));
        assert_eq!(utc("20231315"), None);
        assert_eq!(utc("2023101512000012345"), None);
        assert_eq!(utc("20231015120000.000[+5.5:IST]"), None);
    }
}
