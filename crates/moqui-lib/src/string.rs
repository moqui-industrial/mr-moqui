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

//! String helpers from `StringUtilities`.
//!
//! Groovy class-name cleanup, DOM element text, Elastic query helpers, and
//! `numberToWords` are not ported.

use std::io::Read;

use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;

use crate::error::Error;

/// Encode text for an XML attribute. Characters above ASCII become decimal
/// character references. The control-character substitutions match
/// `StringUtilities.encodeForXmlAttribute`.
pub fn encode_for_xml_attribute(original: &str, add_zero_width_spaces: bool) -> String {
    let mut out = String::with_capacity(original.len());
    for ch in original.chars() {
        match ch {
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{0005}' => out.push_str("..."),
            '\u{0012}' => out.push_str("&apos;"),
            '\u{0013}' | '\u{0014}' => out.push_str("&quot;"),
            '\u{0016}' | '\u{0017}' => out.push('-'),
            '\u{0019}' => out.push_str("tm"),
            other => {
                let code = other as u32;
                if code < 0x20 && other != '\t' && other != '\n' && other != '\r' {
                    continue;
                }
                if code > 0x7F {
                    out.push_str("&#");
                    out.push_str(&code.to_string());
                    out.push(';');
                } else {
                    if add_zero_width_spaces {
                        out.push_str("&#8203;");
                    }
                    out.push(other);
                }
            }
        }
    }
    out
}

/// Expand the five predefined entities and decimal or hex character references.
///
/// An unknown or broken entity is left in the text. This is the
/// `StringUtilities.decodeFromXml` pass, which is separate from the XML parser.
pub fn decode_from_xml(original: &str) -> String {
    if !original.contains('&') {
        return original.to_string();
    }
    let chars: Vec<char> = original.chars().collect();
    let mut out = String::with_capacity(original.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let Some(end) = chars[i + 1..].iter().position(|ch| *ch == ';') else {
            out.push('&');
            i += 1;
            continue;
        };
        let body: String = chars[i + 1..i + 1 + end].iter().collect();
        if let Some(ch) = entity_char(&body) {
            out.push(ch);
            i += end + 2;
        } else {
            out.push('&');
            i += 1;
        }
    }
    out
}

fn entity_char(body: &str) -> Option<char> {
    match body {
        "apos" => Some('\''),
        "quot" => Some('"'),
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        _ => {
            let digits = body.strip_prefix('#')?;
            let code = if let Some(hex) = digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                digits.parse().ok()?
            };
            char::from_u32(code)
        }
    }
}

/// True when `URLDecoder` would leave the text unchanged: letters, digits,
/// `._-*+`, and well-formed `%HH` escapes.
pub fn is_url_decoder_safe(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-' | '*' | '+') {
            i += 1;
            continue;
        }
        if ch == '%' && i + 2 < chars.len() && is_hex(chars[i + 1]) && is_hex(chars[i + 2]) {
            i += 3;
            continue;
        }
        return false;
    }
    true
}

/// `URLEncoder.encode` when the text is not already decoder-safe. Space is `+`.
pub fn url_encode_if_needed(text: &str) -> String {
    if is_url_decoder_safe(text) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => out.push(*byte as char),
            b' ' => out.push('+'),
            _ => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0F) as usize] as char);
            }
        }
    }
    out
}

/// RFC 3986 path characters, the set `isUrlSafeRfc3986` allows.
pub fn is_url_safe_rfc3986(text: &str) -> bool {
    text.chars().all(|ch| {
        ch.is_alphanumeric()
            || matches!(
                ch,
                '.' | '_' | '-' | '+' | '~' | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | ',' | ';' | '=' | ':' | '@'
            )
    })
}

/// Split a URL path. `+` becomes a space, the way `URLDecoder` does. A bad
/// escape keeps that segment as it was written.
pub fn path_string_to_list(path: &str, skip_segments: usize) -> Vec<String> {
    let path = path.strip_prefix('/').unwrap_or(path);
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .skip(skip_segments)
        .map(|segment| url_decode(segment).unwrap_or_else(|| segment.to_string()))
        .collect()
}

fn url_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let high = from_hex(bytes[i + 1])?;
                let low = from_hex(bytes[i + 2])?;
                out.push((high << 4) | low);
                i += 3;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn is_hex(ch: char) -> bool {
    ch.is_ascii_hexdigit()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `camelCaseToPretty`. A part equal to `id` in any case becomes `ID`.
pub fn camel_case_to_pretty(camel: &str) -> String {
    if camel.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = camel.chars().collect();
    let mut raw_parts = Vec::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().copied().enumerate() {
        if index > 0
            && (ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '.' || ch == '#')
            && !current.is_empty()
        {
            raw_parts.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        raw_parts.push(current);
    }
    let mut pretty = String::new();
    let mut last: Option<String> = None;
    for part in raw_parts {
        let mut part = match part.chars().next() {
            Some('.' | '#') => part.chars().skip(1).collect(),
            _ => part,
        };
        if part.is_empty() {
            continue;
        }
        let mut part_chars: Vec<char> = part.chars().collect();
        if part_chars[0].is_lowercase() {
            part_chars[0] = part_chars[0].to_ascii_uppercase();
        }
        part = part_chars.into_iter().collect();
        if part.eq_ignore_ascii_case("id") {
            part = "ID".to_string();
        }
        if last.as_ref().is_some_and(|prev| prev == &part) {
            continue;
        }
        last = Some(part.clone());
        if !pretty.is_empty() {
            pretty.push(' ');
        }
        pretty.push_str(&part);
    }
    pretty
}

/// `prettyToCamelCase`. Characters that are not letters or digits start a new word.
pub fn pretty_to_camel_case(pretty: &str, first_upper: bool) -> String {
    let mut out = String::new();
    let mut upper_next = first_upper;
    for ch in pretty.chars() {
        if ch.is_alphanumeric() {
            if upper_next {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            upper_next = false;
        } else {
            upper_next = true;
        }
    }
    out
}

pub fn remove_non_alphanumeric(orig: &str) -> String {
    orig.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

/// Replace runs of non-alphanumeric characters with a single `pad`.
pub fn replace_non_alphanumeric(orig: &str, pad: char) -> String {
    let mut out = String::new();
    for ch in orig.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with(pad) {
            out.push(pad);
        }
    }
    out
}

pub fn is_alphanumeric_text(text: &str, allowed: &str) -> bool {
    text.chars().all(|ch| ch.is_alphanumeric() || allowed.contains(ch))
}

/// The first digit run, keeping `.` and `,` once digits have started.
/// An empty input is an empty string. No digits is `None`.
pub fn find_first_number(orig: &str) -> Option<String> {
    if orig.is_empty() {
        return Some(String::new());
    }
    let mut num = String::new();
    for ch in orig.chars() {
        if ch.is_ascii_digit() || (!num.is_empty() && (ch == '.' || ch == ',')) {
            num.push(ch);
        } else if !num.is_empty() {
            break;
        }
    }
    if num.is_empty() {
        None
    } else {
        Some(num)
    }
}

/// Drop a leading UTF-8 BOM and decode. Invalid sequences become U+FFFD,
/// which is what `new String(bytes, "UTF-8")` does.
pub fn to_string_clean_bom(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Insert `0` at index 0 until the text is long enough, including in front of a minus.
/// `-1` padded to 4 is `00-1`, matching `StringUtilities.paddedNumber`.
pub fn padded_number(number: i64, desired_length: Option<usize>) -> String {
    let mut out = number.to_string();
    if let Some(len) = desired_length {
        while out.chars().count() < len {
            out.insert(0, '0');
        }
    }
    out
}

pub fn padded_string(input: &str, desired_length: Option<usize>, pad: char, right_pad: bool) -> String {
    let mut out = input.to_string();
    if let Some(len) = desired_length {
        while out.chars().count() < len {
            if right_pad {
                out.push(pad);
            } else {
                out.insert(0, pad);
            }
        }
    }
    out
}

/// `MaskFormatter` with `valueContainsLiteralCharacters` false.
///
/// `#` digit, `U` upper letter, `L` lower letter, `A` letter or digit,
/// `?` letter, `*` anything, `H` hex. `'` escapes the next character.
/// A one-character placeholder fills every missing slot. A longer placeholder
/// is indexed by the mask position, and positions past it use a space.
pub fn format_mask(mask: &str, value: &str, placeholder: Option<&str>) -> Result<String, Error> {
    let tokens = parse_mask(mask)?;
    let value: Vec<char> = value.chars().collect();
    let mut value_index = 0;
    let mut out = String::new();
    for (mask_index, token) in tokens.iter().enumerate() {
        match token {
            MaskToken::Literal(ch) => out.push(*ch),
            MaskToken::Slot(slot) => {
                if value_index < value.len() {
                    let ch = value[value_index];
                    let mapped = map_slot(*slot, ch)
                        .ok_or_else(|| Error::new(format!("mask character '{ch}' does not fit slot {mask_index}")))?;
                    out.push(mapped);
                    value_index += 1;
                } else {
                    out.push(placeholder_at(placeholder, mask_index));
                }
            }
        }
    }
    if value_index != value.len() {
        return Err(Error::new("value is longer than the mask"));
    }
    Ok(out)
}

#[derive(Clone, Copy)]
enum Slot {
    Digit,
    Upper,
    Lower,
    AlphaNum,
    Letter,
    Any,
    Hex,
}

enum MaskToken {
    Literal(char),
    Slot(Slot),
}

fn parse_mask(mask: &str) -> Result<Vec<MaskToken>, Error> {
    let chars: Vec<char> = mask.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' {
            let next = chars
                .get(i + 1)
                .copied()
                .ok_or_else(|| Error::new("mask ends with an escape"))?;
            tokens.push(MaskToken::Literal(next));
            i += 2;
            continue;
        }
        let slot = match ch {
            '#' => Some(Slot::Digit),
            'U' => Some(Slot::Upper),
            'L' => Some(Slot::Lower),
            'A' => Some(Slot::AlphaNum),
            '?' => Some(Slot::Letter),
            '*' => Some(Slot::Any),
            'H' => Some(Slot::Hex),
            _ => None,
        };
        if let Some(slot) = slot {
            tokens.push(MaskToken::Slot(slot));
        } else {
            tokens.push(MaskToken::Literal(ch));
        }
        i += 1;
    }
    Ok(tokens)
}

fn map_slot(slot: Slot, ch: char) -> Option<char> {
    match slot {
        Slot::Digit => ch.is_ascii_digit().then_some(ch),
        Slot::Upper => ch.is_alphabetic().then_some(ch.to_uppercase().next().unwrap_or(ch)),
        Slot::Lower => ch.is_alphabetic().then_some(ch.to_lowercase().next().unwrap_or(ch)),
        Slot::AlphaNum => ch.is_alphanumeric().then_some(ch),
        Slot::Letter => ch.is_alphabetic().then_some(ch),
        Slot::Any => Some(ch),
        Slot::Hex => ch.is_ascii_hexdigit().then_some(ch),
    }
}

fn placeholder_at(placeholder: Option<&str>, mask_index: usize) -> char {
    match placeholder {
        Some(text) if text.chars().count() == 1 => text.chars().next().unwrap(),
        Some(text) => text.chars().nth(mask_index).unwrap_or(' '),
        None => ' ',
    }
}

/// URL-safe Base64 of `length` random bytes, truncated to `length` characters.
pub fn random_string(length: usize) -> Result<String, Error> {
    if length == 0 {
        return Ok(String::new());
    }
    let mut bytes = vec![0_u8; length];
    let mut file = std::fs::File::open("/dev/urandom")
        .map_err(|err| Error::new("could not open /dev/urandom").with_source(err))?;
    file.read_exact(&mut bytes)
        .map_err(|err| Error::new("could not read random bytes").with_source(err))?;
    let encoded = URL_SAFE.encode(bytes);
    Ok(encoded.chars().take(length).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_attribute_encoding_and_decoding() {
        assert_eq!(
            encode_for_xml_attribute(r#"a&b<c>"d'e"#, false),
            "a&amp;b&lt;c&gt;&quot;d&apos;e"
        );
        assert_eq!(encode_for_xml_attribute("\u{0005}\u{0001}A", false), "...A");
        assert_eq!(encode_for_xml_attribute("é", false), "&#233;");
        assert_eq!(decode_from_xml("a&amp;b&lt;&gt;&apos;&quot;&#65;&#x42;"), "a&b<>'\"AB");
        assert_eq!(decode_from_xml("keep &unknown;"), "keep &unknown;");
    }

    #[test]
    fn urls_paths_and_pretty_names() {
        assert_eq!(url_encode_if_needed("a b"), "a+b");
        assert_eq!(url_encode_if_needed("a-b_c.d*e"), "a-b_c.d*e");
        assert_eq!(url_encode_if_needed("%20"), "%20");
        assert_eq!(url_encode_if_needed("%2"), "%252");
        assert!(!is_url_safe_rfc3986("a b"));
        assert_eq!(
            path_string_to_list("/a/b%20c/d+e", 1),
            vec!["b c".to_string(), "d e".to_string()]
        );
        assert_eq!(camel_case_to_pretty("orderId"), "Order ID");
        assert_eq!(camel_case_to_pretty("foo.bar"), "Foo Bar");
        // The browser copy in WebrootVue.qvt.js (qvt.StringUtilities) is checked against the same vectors.
        assert_eq!(camel_case_to_pretty("productStoreId"), "Product Store ID");
        assert_eq!(camel_case_to_pretty("toFieldNameAlias"), "To Field Name Alias");
        assert_eq!(camel_case_to_pretty("aaAa"), "Aa");
        assert_eq!(camel_case_to_pretty("item2Id"), "Item 2 ID");
        assert_eq!(camel_case_to_pretty("#hash"), "Hash");
        assert_eq!(camel_case_to_pretty(""), "");
        assert_eq!(pretty_to_camel_case("Order ID", false), "orderId");
        assert_eq!(pretty_to_camel_case("order id", true), "OrderId");
    }

    #[test]
    fn padding_mask_bom_and_random() {
        assert_eq!(padded_number(12, Some(5)), "00012");
        assert_eq!(padded_number(-1, Some(4)), "00-1");
        assert_eq!(padded_string("ab", Some(4), ' ', true), "ab  ");
        assert_eq!(
            format_mask("(###) ###-####", "1234567890", None).unwrap(),
            "(123) 456-7890"
        );
        assert_eq!(format_mask("##", "1", Some("0")).unwrap(), "10");
        assert!(format_mask("##", "ab", None).is_err());
        let bom = b"\xEF\xBB\xBFhi";
        assert_eq!(to_string_clean_bom(bom), "hi");
        let random = random_string(16).unwrap();
        assert_eq!(random.chars().count(), 16);
        assert!(random
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '='));
        assert_eq!(find_first_number("id 12.5x"), Some("12.5".to_string()));
        assert_eq!(replace_non_alphanumeric("a--b", '_'), "a_b");
    }
}
