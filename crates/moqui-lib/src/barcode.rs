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

//! Linear barcodes as SVG, for print documents (`ec.resource.code128Svg`). Java's XSL-FO documents drew them with the
//! barcode4j FOP extension (`<barcode:code128>`); this is the same symbol and layout as inline SVG.
//!
//! An encoder gives a [`Linear`] barcode, its bar and space widths in modules, and [`Linear::svg`] draws any linear
//! barcode with its quiet zones and the text under the bars. Code 128 is the one symbology so far; another linear one
//! is an encoder beside [`code128`].

use crate::error::Error;

/// A linear barcode: the widths in modules of its bars and spaces, starting with a bar, and its human-readable text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Linear {
    pub widths: Vec<u8>,
    pub text: String,
}

/// How [`Linear::svg`] draws a barcode. Lengths are millimetres. The defaults are barcode4j's for Code 128.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgOptions {
    /// The whole height, the text under the bars included.
    pub height: f64,
    /// The width of the narrowest bar.
    pub module_width: f64,
    pub font_size: f64,
    pub font_family: String,
    /// The text under the bars.
    pub human_readable: bool,
    /// The blank margin at each side, in modules.
    pub quiet_zone: u32,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self {
            height: 15.0,
            module_width: 0.21,
            font_size: 8.0 * MM_PER_POINT,
            font_family: "Helvetica, Arial, sans-serif".to_string(),
            human_readable: true,
            quiet_zone: 10,
        }
    }
}

const MM_PER_INCH: f64 = 25.4;
const MM_PER_POINT: f64 = MM_PER_INCH / 72.0;

/// A length such as `0.4in`, `0.3mm`, `12pt`, `1cm` or `96px`, in millimetres; a number with no unit is millimetres.
///
/// # Errors
///
/// Returns an error when the text is not a number with one of those units.
pub fn length_mm(text: &str) -> Result<f64, Error> {
    let text = text.trim();
    let split = text.find(|ch: char| ch.is_ascii_alphabetic()).unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| Error::new(format!("barcode length {text} is not a number with a unit")))?;
    let per_unit = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "mm" => 1.0,
        "cm" => 10.0,
        "in" => MM_PER_INCH,
        "pt" => MM_PER_POINT,
        "px" => MM_PER_INCH / 96.0,
        other => {
            return Err(Error::new(format!(
                "barcode length {text} has unit {other}, not mm, cm, in, pt or px"
            )))
        }
    };
    if !value.is_finite() || value < 0.0 {
        return Err(Error::new(format!("barcode length {text} is not a length")));
    }
    Ok(value * per_unit)
}

impl Linear {
    /// The barcode as an `<svg>` element, sized in millimetres: the quiet zones, the bars, and the text centred under
    /// them in the bottom 1.3 font sizes of the height, as barcode4j lays it out.
    #[must_use]
    pub fn svg(&self, options: &SvgOptions) -> String {
        let module = options.module_width;
        let quiet = f64::from(options.quiet_zone) * module;
        let modules: u32 = self.widths.iter().map(|width| u32::from(*width)).sum();
        let width = quiet * 2.0 + f64::from(modules) * module;
        let height = options.height;
        let text_height = if options.human_readable {
            options.font_size * 1.3
        } else {
            0.0
        };
        let bar_height = (height - text_height).max(height / 4.0);
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\" \
             shape-rendering=\"crispEdges\"><g fill=\"black\" stroke=\"none\">",
            w = number(width),
            h = number(height),
        );
        let mut x = quiet;
        for (index, modules) in self.widths.iter().enumerate() {
            let bar = f64::from(*modules) * module;
            if index % 2 == 0 {
                svg.push_str(&format!(
                    "<rect x=\"{}\" y=\"0\" width=\"{}\" height=\"{}\"/>",
                    number(x),
                    number(bar),
                    number(bar_height)
                ));
            }
            x += bar;
        }
        if options.human_readable && !self.text.is_empty() {
            svg.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\" text-anchor=\"middle\">{}</text>",
                number(width / 2.0),
                number(bar_height + options.font_size * 0.93),
                escape(&options.font_family),
                number(options.font_size),
                escape(&self.text)
            ));
        }
        svg.push_str("</g></svg>");
        svg
    }
}

/// A number for an SVG attribute: at most four decimals, no trailing zeros.
fn number(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML has no control characters but tab, newline and return
            ch if ch < ' ' && !matches!(ch, '\t' | '\n' | '\r') => out.push(' '),
            ch => out.push(ch),
        }
    }
    out
}

/// The bar and space widths of each Code 128 symbol value, 0 to 105 (ISO/IEC 15417 table 1, as barcode4j has it).
const CODE128: [&[u8; 6]; 106] = [
    b"212222", b"222122", b"222221", b"121223", b"121322", b"131222", b"122213", b"122312", b"132212", b"221213",
    b"221312", b"231212", b"112232", b"122132", b"122231", b"113222", b"123122", b"123221", b"223211", b"221132",
    b"221231", b"213212", b"223112", b"312131", b"311222", b"321122", b"321221", b"312212", b"322112", b"322211",
    b"212123", b"212321", b"232121", b"111323", b"131123", b"131321", b"112313", b"132113", b"132311", b"211313",
    b"231113", b"231311", b"112133", b"112331", b"132131", b"113123", b"113321", b"133121", b"313121", b"211331",
    b"231131", b"213113", b"213311", b"213131", b"311123", b"311321", b"331121", b"312113", b"312311", b"332111",
    b"314111", b"221411", b"431111", b"111224", b"111422", b"121124", b"121421", b"141122", b"141221", b"112214",
    b"112412", b"122114", b"122411", b"142112", b"142211", b"241211", b"221114", b"413111", b"241112", b"134111",
    b"111242", b"121142", b"121241", b"114212", b"124112", b"124211", b"411212", b"421112", b"421211", b"212141",
    b"214121", b"412121", b"111143", b"111341", b"131141", b"114113", b"114311", b"411113", b"411311", b"113141",
    b"114131", b"311141", b"411131", b"211412", b"211214", b"211232",
];
const CODE128_STOP: &[u8; 7] = b"2331112";
const START: [u8; 3] = [103, 104, 105];
const TO_C: u8 = 99;
/// Code B in sets A and C; FNC 4 in set B.
const TO_B: u8 = 100;
/// Code A in sets B and C; FNC 4 in set A.
const TO_A: u8 = 101;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Set {
    A,
    B,
    C,
}

/// The Code 128 barcode of `text`, ASCII only. Digits go in set C, two to a symbol: from the start when the text
/// starts with an even run of four or more, else after one digit in set B, as barcode4j does; later for a run of six or
/// more, or four or more that ends the text. Other characters go in set B, or set A for a control character.
///
/// # Errors
///
/// Returns an error when the text is empty or has a character outside ASCII.
pub fn code128(text: &str) -> Result<Linear, Error> {
    if text.is_empty() {
        return Err(Error::new("a Code 128 barcode needs text"));
    }
    if let Some(ch) = text.chars().find(|ch| !ch.is_ascii()) {
        return Err(Error::new(format!(
            "{ch:?} in {text:?} cannot be encoded in Code 128 (ASCII only)"
        )));
    }
    let bytes = text.as_bytes();
    let digits_at = |at: usize| bytes[at..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    let leading = digits_at(0);
    let mut set = if (leading >= 4 && leading % 2 == 0) || (leading == 2 && bytes.len() == 2) {
        Set::C
    } else {
        a_or_b(&bytes[0..])
    };
    let mut values = vec![START[set as usize]];
    let mut at = 0;
    while at < bytes.len() {
        if set == Set::C {
            if digits_at(at) >= 2 {
                values.push((bytes[at] - b'0') * 10 + (bytes[at + 1] - b'0'));
                at += 2;
                continue;
            }
            set = a_or_b(&bytes[at..]);
            values.push(if set == Set::A { TO_A } else { TO_B });
            continue;
        }
        let run = digits_at(at);
        if run >= 6 || (run >= 4 && at + run == bytes.len()) {
            if run % 2 == 1 {
                values.push(value_in(set, bytes[at]));
                at += 1;
            }
            values.push(TO_C);
            set = Set::C;
            continue;
        }
        let byte = bytes[at];
        if set == Set::A && byte >= 96 {
            values.push(TO_B);
            set = Set::B;
        } else if set == Set::B && byte < 32 {
            values.push(TO_A);
            set = Set::A;
        }
        values.push(value_in(set, byte));
        at += 1;
    }
    let check = values
        .iter()
        .enumerate()
        .map(|(position, value)| position.max(1) * usize::from(*value))
        .sum::<usize>()
        % 103;
    values.push(u8::try_from(check).unwrap_or_default());
    let mut widths = Vec::with_capacity(values.len() * 6 + 7);
    for value in values {
        widths.extend(CODE128[usize::from(value)].iter().map(|digit| digit - b'0'));
    }
    widths.extend(CODE128_STOP.iter().map(|digit| digit - b'0'));
    Ok(Linear {
        widths,
        text: text.to_string(),
    })
}

/// Set A when a control character comes before a lower case one in `rest`, else set B.
fn a_or_b(rest: &[u8]) -> Set {
    match rest.iter().find(|byte| **byte < 32 || **byte >= 96) {
        Some(byte) if *byte < 32 => Set::A,
        _ => Set::B,
    }
}

/// The value of an ASCII character in set A or B.
fn value_in(set: Set, byte: u8) -> u8 {
    match set {
        Set::A if byte < 32 => byte + 64,
        _ => byte - 32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Code 128 text of `widths`, read back with the same table, the checksum checked.
    fn decode(widths: &[u8]) -> String {
        assert!(widths.ends_with(&[2, 3, 3, 1, 1, 1, 2]), "no stop: {widths:?}");
        let symbols = &widths[..widths.len() - 7];
        assert_eq!(symbols.len() % 6, 0);
        let values: Vec<u8> = symbols
            .chunks(6)
            .map(|chunk| {
                let pattern: Vec<u8> = chunk.iter().map(|width| width + b'0').collect();
                let found = CODE128.iter().position(|entry| entry.as_slice() == pattern.as_slice());
                u8::try_from(found.expect("a Code 128 symbol")).unwrap()
            })
            .collect();
        let (check, data) = values.split_last().unwrap();
        let sum: usize = data
            .iter()
            .enumerate()
            .map(|(at, value)| at.max(1) * usize::from(*value))
            .sum();
        assert_eq!(sum % 103, usize::from(*check), "checksum");
        let mut set = match data[0] {
            103 => Set::A,
            104 => Set::B,
            105 => Set::C,
            other => panic!("start {other}"),
        };
        let mut text = String::new();
        for value in &data[1..] {
            match (set, *value) {
                (_, TO_C) if set != Set::C => set = Set::C,
                (Set::A | Set::C, TO_B) => set = Set::B,
                (Set::B | Set::C, TO_A) => set = Set::A,
                (Set::C, pair) => text.push_str(&format!("{pair:02}")),
                (Set::A, value) if value >= 64 => text.push(char::from(value - 64)),
                (_, value) => text.push(char::from(value + 32)),
            }
        }
        text
    }

    fn digits(widths: &[u8]) -> String {
        widths.iter().map(|width| char::from(width + b'0')).collect()
    }

    #[test]
    fn each_symbol_is_eleven_modules_with_even_bars() {
        for (value, pattern) in CODE128.iter().enumerate() {
            let widths: Vec<u32> = pattern.iter().map(|digit| u32::from(digit - b'0')).collect();
            assert_eq!(widths.iter().sum::<u32>(), 11, "{value}");
            assert_eq!((widths[0] + widths[2] + widths[4]) % 2, 0, "{value}");
        }
        let unique: std::collections::BTreeSet<_> = CODE128.iter().collect();
        assert_eq!(unique.len(), CODE128.len());
    }

    /// Bars as barcode4j 2.1 (the library of Java's FOP extension) draws them: the same code sets, symbols, check
    /// symbol and stop.
    #[test]
    fn the_bars_are_barcode4j_bars() {
        for (text, expected) in [
            ("ABC123", "2112141113231311231313211232212232112211321411222331112"),
            ("123456", "2112321122321311233311211321312331112"),
            ("55500", "2112142132121131413113212122222211142331112"),
            (
                "DEMO_1_1",
                "2112141123131321131131231331211112241232211112241232212121412331112",
            ),
            (
                "Hello, World!",
                "2112142311131122142211142211141341111122322122223113211341111212412211141412212221222211142331112",
            ),
            ("100045", "2112322213122122221131231321312331112"),
            (
                "ab12345678cd",
                "2112141211241214211131411122321311233311212411121141311411221412211421122331112",
            ),
            ("1234567", "2112141232211131413121311131231411221341112331112"),
        ] {
            assert_eq!(digits(&code128(text).unwrap().widths), expected, "{text}");
        }
    }

    #[test]
    fn every_ascii_text_reads_back() {
        let printable: String = (32u8..127).map(char::from).collect();
        for text in [
            "12",
            "1",
            "123",
            "1234AB",
            "12345AB",
            "AB1234",
            "AB12345",
            "A1234567B",
            "\tTAB\nline",
            "lower\u{1}control",
            "a\u{1f}b`c",
            "00",
            "0000000",
            printable.as_str(),
        ] {
            assert_eq!(decode(&code128(text).unwrap().widths), text, "{text:?}");
        }
    }

    #[test]
    fn text_outside_ascii_or_empty_is_an_error() {
        assert!(code128("caf\u{e9}").unwrap_err().message().contains("ASCII only"));
        assert!(code128("").is_err());
    }

    #[test]
    fn lengths_take_the_css_units() {
        let near = |text: &str, mm: f64| assert!((length_mm(text).unwrap() - mm).abs() < 1e-9, "{text}");
        near("0.4in", 10.16);
        near("12pt", 12.0 * 25.4 / 72.0);
        near("0.3mm", 0.3);
        near(" 1.5 cm ", 15.0);
        near("96px", 25.4);
        near("5", 5.0);
        assert!(length_mm("4em").is_err());
        assert!(length_mm("wide").is_err());
    }

    #[test]
    fn the_svg_has_the_quiet_zones_bars_and_text_in_millimetres() {
        let options = SvgOptions {
            height: length_mm("0.4in").unwrap(),
            module_width: 0.3,
            font_size: length_mm("12pt").unwrap(),
            ..SvgOptions::default()
        };
        let barcode = code128("55500").unwrap();
        let svg = barcode.svg(&options);
        // 79 modules (start, 4 symbols, check, stop) and 10 each side, 0.3mm a module
        assert!(
            svg.starts_with(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"29.7mm\" height=\"10.16mm\" viewBox=\"0 0 29.7 10.16\""
            ),
            "{svg}"
        );
        // the first bar, two modules wide, after the 3mm quiet zone; the bars leave 1.3 font sizes for the text
        assert!(
            svg.contains("<rect x=\"3\" y=\"0\" width=\"0.6\" height=\"4.6567\"/>"),
            "{svg}"
        );
        assert_eq!(svg.matches("<rect ").count(), barcode.widths.len().div_ceil(2));
        assert!(
            svg.contains(
                "<text x=\"14.85\" y=\"8.5937\" font-family=\"Helvetica, Arial, sans-serif\" font-size=\"4.2333\" \
             text-anchor=\"middle\">55500</text>"
            ),
            "{svg}"
        );
        let plain = barcode.svg(&SvgOptions {
            human_readable: false,
            quiet_zone: 0,
            ..options
        });
        assert!(!plain.contains("<text"));
        assert!(
            plain.contains("<rect x=\"0\" y=\"0\" width=\"0.6\" height=\"10.16\"/>"),
            "{plain}"
        );
        let escaped = code128("A<B&\"C\"").unwrap().svg(&SvgOptions::default());
        assert!(escaped.contains(">A&lt;B&amp;&quot;C&quot;</text>"), "{escaped}");
    }
}
