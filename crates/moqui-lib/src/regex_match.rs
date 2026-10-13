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

//! Full-string matcher for the `matches` operator.
//!
//! Java `String.matches` compiles the pattern with `java.util.regex` and requires
//! the whole input to match. `.` does not cross a line ending. There is no regex crate.
//!
//! Groups do not capture, so `(?:...)` is the same as `(...)` and a reluctant quantifier
//! accepts the same strings as a greedy one. Backreferences, lookaround, inline flags,
//! possessive quantifiers and escapes outside `\d\D\w\W\s\S\n\r\t\f` are errors rather
//! than literals, so an unsupported pattern fails closed.

use crate::error::Error;

#[derive(Clone, Debug)]
enum Re {
    Empty,
    Lit(char),
    Any,
    /// A char hits when it is in `ranges`, or outside any one of the `outside` sets
    /// (`\D`, `\W`, `\S` inside a class).
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
        outside: Vec<Vec<(char, char)>>,
    },
    Seq(Vec<Re>),
    Alt(Vec<Re>),
    Repeat { inner: Box<Re>, min: u32, max: Option<u32> },
    Start,
    End,
}

pub fn is_full_match(text: &str, pattern: &str) -> Result<bool, Error> {
    if pattern.is_empty() {
        return Ok(text.is_empty());
    }
    let re = parse_regex(pattern)?;
    let chars: Vec<char> = text.chars().collect();
    Ok(ends(&re, &chars, 0).contains(&chars.len()))
}

struct Parser {
    pat: Vec<char>,
    i: usize,
}

fn parse_regex(pattern: &str) -> Result<Re, Error> {
    let mut parser = Parser {
        pat: pattern.chars().collect(),
        i: 0,
    };
    let re = parser.parse_alt()?;
    if parser.i != parser.pat.len() {
        return Err(Error::new("unmatched ')' in regex"));
    }
    Ok(re)
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.pat.get(self.i).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.pat.get(self.i + offset).copied()
    }

    fn parse_alt(&mut self) -> Result<Re, Error> {
        let mut alts = vec![self.parse_seq()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.parse_seq()?);
        }
        if alts.len() == 1 {
            Ok(alts.pop().expect("one regex alternative"))
        } else {
            Ok(Re::Alt(alts))
        }
    }

    fn parse_seq(&mut self) -> Result<Re, Error> {
        let mut items = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            items.push(self.parse_repeat()?);
        }
        Ok(collapse(items))
    }

    fn parse_repeat(&mut self) -> Result<Re, Error> {
        let atom = self.parse_atom()?;
        let (min, max) = match self.peek() {
            Some('*') => {
                self.i += 1;
                (0, None)
            }
            Some('+') => {
                self.i += 1;
                (1, None)
            }
            Some('?') => {
                self.i += 1;
                (0, Some(1))
            }
            Some('{') => self.parse_bounds()?,
            _ => return Ok(atom),
        };
        match self.peek() {
            Some('?') => self.i += 1,
            Some('+') => return Err(Error::new("possessive quantifiers are not supported in regex")),
            _ => {}
        }
        Ok(Re::Repeat {
            inner: Box::new(atom),
            min,
            max,
        })
    }

    fn parse_bounds(&mut self) -> Result<(u32, Option<u32>), Error> {
        self.i += 1;
        let min = self.take_u32()?;
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok((min, Some(min)));
        }
        if self.peek() != Some(',') {
            return Err(Error::new("invalid '{n,m}' quantifier"));
        }
        self.i += 1;
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok((min, None));
        }
        let max = self.take_u32()?;
        if self.peek() != Some('}') || max < min {
            return Err(Error::new("invalid '{n,m}' quantifier"));
        }
        self.i += 1;
        Ok((min, Some(max)))
    }

    fn take_u32(&mut self) -> Result<u32, Error> {
        let start = self.i;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return Err(Error::new("invalid '{n,m}' quantifier"));
        }
        self.pat[start..self.i]
            .iter()
            .collect::<String>()
            .parse()
            .map_err(|_| Error::new("invalid '{n,m}' quantifier"))
    }

    fn parse_atom(&mut self) -> Result<Re, Error> {
        let ch = self.peek().ok_or_else(|| Error::new("regex ended before an atom"))?;
        match ch {
            '(' => self.parse_group(),
            '[' => self.parse_class(),
            '.' => {
                self.i += 1;
                Ok(Re::Any)
            }
            '^' => {
                self.i += 1;
                Ok(Re::Start)
            }
            '$' => {
                self.i += 1;
                Ok(Re::End)
            }
            '\\' => {
                self.i += 1;
                let esc = self.peek().ok_or_else(|| Error::new("dangling '\\' in regex"))?;
                self.i += 1;
                escape_atom(esc)
            }
            ')' | '|' | '*' | '+' | '?' | '{' => Err(Error::new(format!("unexpected '{ch}' in regex"))),
            _ => {
                self.i += 1;
                Ok(Re::Lit(ch))
            }
        }
    }

    fn parse_group(&mut self) -> Result<Re, Error> {
        self.i += 1;
        if self.peek() == Some('?') {
            if self.peek_at(1) != Some(':') {
                return Err(Error::new(
                    "lookaround, named groups and inline flags are not supported in regex",
                ));
            }
            self.i += 2;
        }
        let inner = self.parse_alt()?;
        if self.peek() != Some(')') {
            return Err(Error::new("unclosed '(' in regex"));
        }
        self.i += 1;
        Ok(inner)
    }

    fn parse_class(&mut self) -> Result<Re, Error> {
        self.i += 1;
        let negated = if self.peek() == Some('^') {
            self.i += 1;
            true
        } else {
            false
        };
        let mut ranges = Vec::new();
        let mut outside = Vec::new();
        if self.peek() == Some(']') {
            ranges.push((']', ']'));
            self.i += 1;
        }
        while let Some(ch) = self.peek() {
            if ch == ']' {
                break;
            }
            if ch == '[' || (ch == '&' && self.peek_at(1) == Some('&')) {
                return Err(Error::new("nested classes and '&&' are not supported in regex"));
            }
            let start = match self.class_item()? {
                ClassItem::Char(start) => start,
                ClassItem::Set { negated: false, ranges: set } => {
                    ranges.extend(set);
                    continue;
                }
                ClassItem::Set { negated: true, ranges: set } => {
                    outside.push(set);
                    continue;
                }
            };
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|next| next != ']') {
                self.i += 1;
                let ClassItem::Char(end) = self.class_item()? else {
                    return Err(Error::new("illegal character range in regex"));
                };
                if end < start {
                    return Err(Error::new("illegal character range in regex"));
                }
                ranges.push((start, end));
            } else {
                ranges.push((start, start));
            }
        }
        if self.peek() != Some(']') {
            return Err(Error::new("unclosed '[' in regex"));
        }
        self.i += 1;
        Ok(Re::Class {
            negated,
            ranges,
            outside,
        })
    }

    fn class_item(&mut self) -> Result<ClassItem, Error> {
        let ch = self.peek().ok_or_else(|| Error::new("unclosed '[' in regex"))?;
        self.i += 1;
        if ch != '\\' {
            return Ok(ClassItem::Char(ch));
        }
        let esc = self
            .peek()
            .ok_or_else(|| Error::new("dangling '\\' in character class"))?;
        self.i += 1;
        Ok(match escape_atom(esc)? {
            Re::Lit(lit) => ClassItem::Char(lit),
            Re::Class { negated, ranges, .. } => ClassItem::Set { negated, ranges },
            _ => return Err(Error::new(format!("unsupported escape '\\{esc}' in character class"))),
        })
    }
}

enum ClassItem {
    Char(char),
    Set { negated: bool, ranges: Vec<(char, char)> },
}

fn collapse(mut items: Vec<Re>) -> Re {
    if items.is_empty() {
        Re::Empty
    } else if items.len() == 1 {
        items.pop().expect("one regex item")
    } else {
        Re::Seq(items)
    }
}

/// Java rejects an escaped letter it does not define and reads an escaped digit as a
/// backreference or octal value. Neither is supported here, so both are errors.
fn escape_atom(ch: char) -> Result<Re, Error> {
    Ok(match ch {
        'd' => shorthand(false, &[('0', '9')]),
        'D' => shorthand(true, &[('0', '9')]),
        'w' => shorthand(false, WORD),
        'W' => shorthand(true, WORD),
        's' => shorthand(false, SPACE),
        'S' => shorthand(true, SPACE),
        'n' => Re::Lit('\n'),
        'r' => Re::Lit('\r'),
        't' => Re::Lit('\t'),
        'f' => Re::Lit('\u{000c}'),
        other if other.is_ascii_alphanumeric() => {
            return Err(Error::new(format!("unsupported escape '\\{other}' in regex")));
        }
        other => Re::Lit(other),
    })
}

const WORD: &[(char, char)] = &[('A', 'Z'), ('a', 'z'), ('0', '9'), ('_', '_')];

const SPACE: &[(char, char)] = &[
    (' ', ' '),
    ('\t', '\t'),
    ('\n', '\n'),
    ('\u{000b}', '\u{000b}'),
    ('\u{000c}', '\u{000c}'),
    ('\r', '\r'),
];

fn shorthand(negated: bool, ranges: &[(char, char)]) -> Re {
    Re::Class {
        negated,
        ranges: ranges.to_vec(),
        outside: Vec::new(),
    }
}

fn ends(re: &Re, text: &[char], pos: usize) -> Vec<usize> {
    match re {
        Re::Empty => vec![pos],
        Re::Lit(ch) => {
            if text.get(pos) == Some(ch) {
                vec![pos + 1]
            } else {
                Vec::new()
            }
        }
        Re::Any => match text.get(pos) {
            Some(ch) if !is_line_end(*ch) => vec![pos + 1],
            _ => Vec::new(),
        },
        Re::Start => {
            if pos == 0 {
                vec![pos]
            } else {
                Vec::new()
            }
        }
        Re::End => {
            if pos == text.len() || is_final_terminator(&text[pos..]) {
                vec![pos]
            } else {
                Vec::new()
            }
        }
        Re::Class {
            negated,
            ranges,
            outside,
        } => match text.get(pos) {
            Some(ch)
                if (class_hit(*ch, ranges) || outside.iter().any(|set| !class_hit(*ch, set))) ^ negated =>
            {
                vec![pos + 1]
            }
            _ => Vec::new(),
        },
        Re::Seq(items) => {
            let mut positions = vec![pos];
            for item in items {
                let mut next = Vec::new();
                for at in positions {
                    next.extend(ends(item, text, at));
                }
                next.sort_unstable();
                next.dedup();
                positions = next;
                if positions.is_empty() {
                    break;
                }
            }
            positions
        }
        Re::Alt(alts) => {
            let mut out = Vec::new();
            for alt in alts {
                out.extend(ends(alt, text, pos));
            }
            out.sort_unstable();
            out.dedup();
            out
        }
        Re::Repeat { inner, min, max } => repeat_ends(inner, *min, *max, text, pos),
    }
}

fn class_hit(ch: char, ranges: &[(char, char)]) -> bool {
    ranges.iter().any(|(start, end)| ch >= *start && ch <= *end)
}

fn is_line_end(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Without `MULTILINE`, Java `$` also matches before one line terminator that ends the input.
fn is_final_terminator(rest: &[char]) -> bool {
    match rest {
        [ch] => is_line_end(*ch),
        ['\r', '\n'] => true,
        _ => false,
    }
}

fn repeat_ends(inner: &Re, min: u32, max: Option<u32>, text: &[char], start: usize) -> Vec<usize> {
    let cap = max.unwrap_or_else(|| (text.len() as u32).saturating_add(1));
    let mut reached = vec![vec![start]];
    while (reached.len() as u32) - 1 < cap {
        let prev = reached.last().cloned().unwrap_or_default();
        if prev.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for pos in prev {
            next.extend(ends(inner, text, pos));
        }
        next.sort_unstable();
        next.dedup();
        if next.is_empty() {
            break;
        }
        let prev_layer = reached.last().expect("repeat layer");
        let only_previous = next.iter().all(|end| prev_layer.contains(end));
        reached.push(next);
        if only_previous {
            let pad = reached.last().cloned().unwrap_or_default();
            while (reached.len() as u32) - 1 < min.min(cap) {
                reached.push(pad.clone());
            }
            break;
        }
    }
    let mut out = Vec::new();
    for (count, layer) in reached.iter().enumerate() {
        if (count as u32) >= min && max.is_none_or(|limit| (count as u32) <= limit) {
            out.extend(layer.iter().copied());
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::is_full_match;

    fn hit(text: &str, pattern: &str) -> bool {
        is_full_match(text, pattern).unwrap()
    }

    #[test]
    fn stock_moqui_patterns() {
        let wiki = r"[\w\.\-,':()!\? ]*";
        assert!(hit("Page1", wiki));
        assert!(hit("My Page (v2)!", wiki));
        assert!(hit("it's_a-b.c,d:e?", wiki));
        assert!(!hit("bad/name", wiki));
        let phone = r"^\+?\d[-\. \d]*\d\d$";
        assert!(hit("+1 801-555.1234", phone));
        assert!(!hit("801x5551234", phone));
        assert!(hit("snake_case9", "[a-z][a-z0-9_]*"));
        assert!(!hit("Snake", "[a-z][a-z0-9_]*"));
        assert!(hit("org.moqui2", r"[a-z][a-z0-9\.]*"));
        assert!(hit("camelCase_1", r"[A-Za-z]\w*"));
        assert!(hit("Pascal", r"[A-Z]\w*"));
        assert!(!hit("pascal", r"[A-Z]\w*"));
    }

    #[test]
    fn shorthands_inside_classes() {
        assert!(hit("a1 _", r"[\d\w\s]+"));
        assert!(!hit("-", r"[\d\w\s]"));
        assert!(hit("x", r"[\D]"));
        assert!(!hit("5", r"[\D]"));
        assert!(hit("5", r"[^\D]"));
        assert!(hit("-", r"[\d-]"));
        assert!(hit("\u{000b}", r"\s"));
        assert!(is_full_match("a", r"[\d-z]").is_ok());
        assert!(is_full_match("a", r"[a-\d]").is_err());
    }

    #[test]
    fn groups_quantifiers_and_anchors() {
        assert!(hit("abab", "(?:ab)+"));
        assert!(hit("aaa", "a*?"));
        assert!(hit("ab", "a{1,2}?b"));
        assert!(hit("a\n", "a$\n"));
        assert!(!hit("a\n", "a$"));
    }

    #[test]
    fn unsupported_features_fail_closed() {
        for pattern in [r"(a)\1", r"\bword", r"a*+", "(?=a)a", "(?i)a", r"\p{L}", "[a[b]]", "[a&&b]", r"\x41"] {
            assert!(is_full_match("a1", pattern).is_err(), "{pattern}");
        }
    }
}
