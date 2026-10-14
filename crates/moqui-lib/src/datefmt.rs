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

//! Java `DateTimeFormatter` and `SimpleDateFormat` patterns, root locale (English names).
//!
//! The letters are the subset screens and reports use: `G y u M L d D E a h H k K m s S Z X x z`,
//! quoted literals, and `''` for a quote. Any other ASCII letter is an error, so a pattern this
//! does not understand is reported instead of printed wrong. Zone letters follow
//! `SimpleDateFormat`, which `L10nFacade` uses: `z` is the short name (`PST`), `Z` is `-0800` at
//! any count, and `X`, `XX`, `XXX` are `-08`, `-0800`, `-08:00`.

use jiff::civil::Weekday;
use jiff::Zoned;

use crate::error::Error;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Format `zoned` with a Java date pattern.
///
/// # Errors
///
/// Returns an error for a pattern letter outside the supported subset or an unclosed quote.
pub fn format(zoned: &Zoned, pattern: &str) -> Result<String, Error> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\'' {
            if chars.get(index + 1) == Some(&'\'') {
                out.push('\'');
                index += 2;
                continue;
            }
            let mut end = index + 1;
            loop {
                match chars.get(end) {
                    None => return Err(Error::new(format!("unclosed quote in date pattern {pattern}"))),
                    Some('\'') if chars.get(end + 1) == Some(&'\'') => {
                        out.push('\'');
                        end += 2;
                    }
                    Some('\'') => break,
                    Some(other) => {
                        out.push(*other);
                        end += 1;
                    }
                }
            }
            index = end + 1;
            continue;
        }
        if !ch.is_ascii_alphabetic() {
            out.push(ch);
            index += 1;
            continue;
        }
        let mut count = 1;
        while chars.get(index + count) == Some(&ch) {
            count += 1;
        }
        field(zoned, ch, count, pattern, &mut out)?;
        index += count;
    }
    Ok(out)
}

fn field(zoned: &Zoned, letter: char, count: usize, pattern: &str, out: &mut String) -> Result<(), Error> {
    match letter {
        'G' => out.push_str(if zoned.year() > 0 { "AD" } else { "BC" }),
        'y' | 'u' => {
            let year = i32::from(zoned.year());
            if count == 2 {
                pad(out, i64::from(year.rem_euclid(100)), 2);
            } else {
                pad(out, i64::from(year), count);
            }
        }
        'M' | 'L' => {
            let month = usize::try_from(zoned.month()).unwrap_or(1).clamp(1, 12);
            match count {
                1 | 2 => pad(out, month as i64, count),
                3 => out.push_str(&MONTHS[month - 1][..3]),
                _ => out.push_str(MONTHS[month - 1]),
            }
        }
        'd' => pad(out, i64::from(zoned.day()), count),
        'D' => pad(out, i64::from(zoned.day_of_year()), count),
        'E' => {
            let name = weekday_name(zoned.weekday());
            if count >= 4 {
                out.push_str(name);
            } else {
                out.push_str(&name[..3]);
            }
        }
        'a' => out.push_str(if zoned.hour() < 12 { "AM" } else { "PM" }),
        'H' => pad(out, i64::from(zoned.hour()), count),
        'k' => pad(out, if zoned.hour() == 0 { 24 } else { i64::from(zoned.hour()) }, count),
        'K' => pad(out, i64::from(zoned.hour() % 12), count),
        'h' => {
            let hour = i64::from(zoned.hour() % 12);
            pad(out, if hour == 0 { 12 } else { hour }, count);
        }
        'm' => pad(out, i64::from(zoned.minute()), count),
        's' => pad(out, i64::from(zoned.second()), count),
        'S' => {
            let nanos = format!("{:09}", zoned.subsec_nanosecond().max(0));
            let take = count.min(9);
            out.push_str(&nanos[..take]);
            for _ in take..count {
                out.push('0');
            }
        }
        'Z' => offset(out, zoned.offset().seconds(), OffsetWidth::Minutes, false),
        'X' | 'x' => {
            let width = match count {
                1 => OffsetWidth::Hours,
                2 => OffsetWidth::Minutes,
                _ => OffsetWidth::Colon,
            };
            offset(out, zoned.offset().seconds(), width, letter == 'X');
        }
        'z' => out.push_str(&zone_text(zoned)),
        _ => {
            return Err(Error::new(format!(
                "date pattern letter {letter} is not supported in {pattern}"
            )))
        }
    }
    Ok(())
}

fn pad(out: &mut String, value: i64, width: usize) {
    let text = value.unsigned_abs().to_string();
    if value < 0 {
        out.push('-');
    }
    for _ in text.len()..width {
        out.push('0');
    }
    out.push_str(&text);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OffsetWidth {
    /// `+HH`, for `X`.
    Hours,
    /// `+HHMM`, for `Z` at any count and `XX`.
    Minutes,
    /// `+HH:MM`, for `XXX`.
    Colon,
}

/// A zone offset. `X` prints `Z` for a zero offset.
fn offset(out: &mut String, seconds: i32, width: OffsetWidth, z_for_zero: bool) {
    if seconds == 0 && z_for_zero {
        out.push('Z');
        return;
    }
    out.push(if seconds < 0 { '-' } else { '+' });
    let total = seconds.unsigned_abs() / 60;
    pad(out, i64::from(total / 60), 2);
    match width {
        OffsetWidth::Hours => {}
        OffsetWidth::Minutes => pad(out, i64::from(total % 60), 2),
        OffsetWidth::Colon => {
            out.push(':');
            pad(out, i64::from(total % 60), 2);
        }
    }
}

/// The short zone name as `SimpleDateFormat` prints it, such as `PST` or `CET`. A zone whose
/// tz database abbreviation is only an offset, such as `-03`, prints as `GMT-03:00`.
fn zone_text(zoned: &Zoned) -> String {
    let info = zoned.time_zone().to_offset_info(zoned.timestamp());
    let abbreviation = info.abbreviation();
    if !abbreviation.is_empty() && abbreviation.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return abbreviation.to_string();
    }
    let mut text = String::from("GMT");
    offset(&mut text, zoned.offset().seconds(), OffsetWidth::Colon, false);
    text
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

/// The fields a Java date pattern read from text. A field the pattern does not have is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParsedDate {
    pub year: Option<i16>,
    pub month: Option<i8>,
    pub day: Option<i8>,
    pub hour: Option<i8>,
    pub minute: Option<i8>,
    pub second: Option<i8>,
    /// From `S`, which Java reads as a count of milliseconds.
    pub millisecond: Option<i32>,
    /// From `Z`, `X` or a `UTC`/`GMT` `z`, in seconds east of UTC.
    pub offset_seconds: Option<i32>,
}

enum PatternPart {
    Literal(String),
    Field(char, usize),
}

fn pattern_parts(pattern: &str) -> Option<Vec<PatternPart>> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\'' {
            if chars.get(index + 1) == Some(&'\'') {
                parts.push(PatternPart::Literal("'".to_string()));
                index += 2;
                continue;
            }
            let mut text = String::new();
            let mut end = index + 1;
            loop {
                match chars.get(end) {
                    None => return None,
                    Some('\'') if chars.get(end + 1) == Some(&'\'') => {
                        text.push('\'');
                        end += 2;
                    }
                    Some('\'') => break,
                    Some(other) => {
                        text.push(*other);
                        end += 1;
                    }
                }
            }
            parts.push(PatternPart::Literal(text));
            index = end + 1;
        } else if ch.is_ascii_alphabetic() {
            let mut count = 1;
            while chars.get(index + count) == Some(&ch) {
                count += 1;
            }
            parts.push(PatternPart::Field(ch, count));
            index += count;
        } else {
            parts.push(PatternPart::Literal(ch.to_string()));
            index += 1;
        }
    }
    Some(parts)
}

/// Read `input` with a Java `SimpleDateFormat` pattern, as `L10nFacade.parseTimestamp(input, format)` does through the
/// commons validator: the whole text must match. A numeric field right before another field takes exactly as many
/// digits as its letters (`yyyyMMdd`); otherwise it takes the digits that are there. `yy` is 1970 to 2069. Month
/// names are English. None when the text does not match.
pub fn parse(input: &str, pattern: &str) -> Option<ParsedDate> {
    let parts = pattern_parts(pattern)?;
    let text: Vec<char> = input.trim().chars().collect();
    let mut at = 0;
    let mut out = ParsedDate::default();
    let mut pm = None;
    for (index, part) in parts.iter().enumerate() {
        match part {
            PatternPart::Literal(literal) => {
                for expected in literal.chars() {
                    if text.get(at) != Some(&expected) {
                        return None;
                    }
                    at += 1;
                }
            }
            PatternPart::Field(letter, count) => {
                let abutting = matches!(parts.get(index + 1), Some(PatternPart::Field(..)));
                match letter {
                    'M' | 'L' if *count >= 3 => {
                        let rest: String = text[at..].iter().collect::<String>().to_ascii_lowercase();
                        let (month, length) = MONTHS.iter().enumerate().find_map(|(number, name)| {
                            let name = name.to_ascii_lowercase();
                            if rest.starts_with(&name) {
                                Some((number, name.len()))
                            } else if rest.starts_with(&name[..3]) {
                                Some((number, 3))
                            } else {
                                None
                            }
                        })?;
                        out.month = Some(i8::try_from(month + 1).ok()?);
                        at += length;
                    }
                    'E' => {
                        while text.get(at).is_some_and(char::is_ascii_alphabetic) {
                            at += 1;
                        }
                    }
                    'a' => {
                        let marker: String = text.get(at..at + 2)?.iter().collect::<String>().to_ascii_uppercase();
                        pm = Some(match marker.as_str() {
                            "AM" => false,
                            "PM" => true,
                            _ => return None,
                        });
                        at += 2;
                    }
                    'Z' | 'X' | 'x' => {
                        if *letter == 'X' && text.get(at) == Some(&'Z') {
                            out.offset_seconds = Some(0);
                            at += 1;
                            continue;
                        }
                        let sign = match text.get(at)? {
                            '+' => 1,
                            '-' => -1,
                            _ => return None,
                        };
                        at += 1;
                        let hours = take_digits(&text, &mut at, Some(2))?;
                        if text.get(at) == Some(&':') {
                            at += 1;
                        }
                        let minutes = if text.get(at).is_some_and(char::is_ascii_digit) {
                            take_digits(&text, &mut at, Some(2))?
                        } else {
                            0
                        };
                        out.offset_seconds = Some(sign * i32::try_from(hours * 3600 + minutes * 60).ok()?);
                    }
                    'z' => {
                        let start = at;
                        while text.get(at).is_some_and(char::is_ascii_alphabetic) {
                            at += 1;
                        }
                        let name: String = text[start..at].iter().collect();
                        if matches!(name.as_str(), "UTC" | "GMT" | "Z") {
                            out.offset_seconds = Some(0);
                        }
                    }
                    'y' | 'u' | 'M' | 'L' | 'd' | 'H' | 'k' | 'K' | 'h' | 'm' | 's' | 'S' => {
                        let width = abutting.then_some(*count);
                        let start = at;
                        let value = take_digits(&text, &mut at, width)?;
                        match letter {
                            'y' | 'u' => {
                                let year = if *count == 2 && at - start == 2 {
                                    if value < 70 { 2000 + value } else { 1900 + value }
                                } else {
                                    value
                                };
                                out.year = Some(i16::try_from(year).ok()?);
                            }
                            'M' | 'L' => out.month = Some(i8::try_from(value).ok()?),
                            'd' => out.day = Some(i8::try_from(value).ok()?),
                            'H' => out.hour = Some(i8::try_from(value).ok()?),
                            'k' => out.hour = Some(i8::try_from(value % 24).ok()?),
                            'K' | 'h' => out.hour = Some(i8::try_from(value % 12).ok()?),
                            'm' => out.minute = Some(i8::try_from(value).ok()?),
                            's' => out.second = Some(i8::try_from(value).ok()?),
                            _ => out.millisecond = Some(i32::try_from(value).ok()?),
                        }
                    }
                    _ => return None,
                }
            }
        }
    }
    if at != text.len() {
        return None;
    }
    if pm == Some(true) {
        out.hour = Some(out.hour.unwrap_or(0) % 12 + 12);
    }
    Some(out)
}

/// Digits at `at`: exactly `width` of them, or as many as there are (at least one).
fn take_digits(text: &[char], at: &mut usize, width: Option<usize>) -> Option<i64> {
    let start = *at;
    let limit = width.map_or(text.len(), |width| (start + width).min(text.len()));
    while *at < limit && text[*at].is_ascii_digit() {
        *at += 1;
    }
    if *at == start || width.is_some_and(|width| *at - start != width) {
        return None;
    }
    text[start..*at].iter().collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{format, parse};
    use jiff::civil::date;
    use jiff::tz::TimeZone;

    fn at(zone: &str) -> jiff::Zoned {
        date(2026, 1, 5)
            .at(9, 7, 3, 45_000_000)
            .to_zoned(TimeZone::get(zone).unwrap())
            .unwrap()
    }

    #[test]
    fn report_patterns_use_english_short_months() {
        let zoned = at("UTC");
        assert_eq!(format(&zoned, "dd MMM yyyy").unwrap(), "05 Jan 2026");
        assert_eq!(
            format(&zoned, "yyyy-MM-dd HH:mm:ss.SSS").unwrap(),
            "2026-01-05 09:07:03.045"
        );
        assert_eq!(format(&zoned, "EEE, d MMMM yy").unwrap(), "Mon, 5 January 26");
        assert_eq!(format(&zoned, "h:mm a").unwrap(), "9:07 AM");
    }

    #[test]
    fn quotes_offsets_and_unknown_letters() {
        let zoned = at("America/Los_Angeles");
        assert_eq!(
            format(&zoned, "yyyy-MM-dd'T'HH:mmXXX").unwrap(),
            "2026-01-05T09:07-08:00"
        );
        assert_eq!(format(&zoned, "''yy'' Z").unwrap(), "'26' -0800");
        assert_eq!(format(&at("UTC"), "X").unwrap(), "Z");
        assert!(format(&zoned, "yyyy ww").is_err());
        assert!(format(&zoned, "'open").is_err());
    }

    #[test]
    fn zone_names_and_offset_widths_follow_simple_date_format() {
        let zoned = at("America/Los_Angeles");
        assert_eq!(format(&zoned, "z").unwrap(), "PST");
        assert_eq!(format(&zoned, "X XX XXX").unwrap(), "-08 -0800 -08:00");
        assert_eq!(format(&zoned, "Z ZZZZ").unwrap(), "-0800 -0800");
        assert_eq!(format(&at("UTC"), "z").unwrap(), "UTC");
        assert_eq!(format(&at("Asia/Tokyo"), "z").unwrap(), "JST");
        assert_eq!(format(&at("Europe/Berlin"), "z").unwrap(), "CET");
        assert_eq!(format(&at("America/Sao_Paulo"), "z").unwrap(), "GMT-03:00");
    }

    #[test]
    fn the_patterns_the_components_parse_with_read_their_fields() {
        let full = parse("2024-03-05T10:20:30.123-0800", "yyyy-MM-dd'T'HH:mm:ss.SSSZ").unwrap();
        assert_eq!(
            (full.year, full.month, full.day, full.hour, full.minute, full.second, full.millisecond, full.offset_seconds),
            (Some(2024), Some(3), Some(5), Some(10), Some(20), Some(30), Some(123), Some(-8 * 3600))
        );
        let abutting = parse("20240305 1020", "yyyyMMdd HHmm").unwrap();
        assert_eq!((abutting.year, abutting.month, abutting.day, abutting.hour), (Some(2024), Some(3), Some(5), Some(10)));
        let us = parse("3/5/2024 1:2:03 PM", "M/d/yyyy h:m:ss a").unwrap();
        assert_eq!((us.month, us.day, us.hour, us.minute), (Some(3), Some(5), Some(13), Some(2)));
        let named = parse("05 Mar 24", "dd MMM yy").unwrap();
        assert_eq!((named.year, named.month), (Some(2024), Some(3)));
        // The whole text must match, and a short abutting field does not.
        assert!(parse("2024-03-05 junk", "yyyy-MM-dd").is_none());
        assert!(parse("202403", "yyyyMMdd").is_none());
    }
}
