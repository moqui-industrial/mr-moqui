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
//! does not understand is reported instead of printed wrong.

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
        'Z' => offset(out, zoned.offset().seconds(), count >= 4, false),
        'X' | 'x' => offset(out, zoned.offset().seconds(), count >= 3, letter == 'X'),
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

/// `+HHMM`, or `+HH:MM` when `colon`. `X` prints `Z` for a zero offset.
fn offset(out: &mut String, seconds: i32, colon: bool, z_for_zero: bool) {
    if seconds == 0 && z_for_zero {
        out.push('Z');
        return;
    }
    out.push(if seconds < 0 { '-' } else { '+' });
    let total = seconds.unsigned_abs() / 60;
    pad(out, i64::from(total / 60), 2);
    if colon {
        out.push(':');
    }
    pad(out, i64::from(total % 60), 2);
}

fn zone_text(zoned: &Zoned) -> String {
    match zoned.time_zone().iana_name() {
        Some(name) => name.to_string(),
        None => {
            let mut text = String::from("GMT");
            offset(&mut text, zoned.offset().seconds(), true, false);
            text
        }
    }
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

#[cfg(test)]
mod tests {
    use super::format;
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
}
