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

#[derive(Clone, Debug)]
enum Re {
    Empty,
    Lit(char),
    Any,
    Class { negated: bool, ranges: Vec<(char, char)> },
    Seq(Vec<Re>),
    Alt(Vec<Re>),
    Repeat { inner: Box<Re>, min: u32, max: Option<u32> },
    Start,
    End,
}

pub fn is_full_match(text: &str, pattern: &str) -> Result<bool, String> {
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

fn parse_regex(pattern: &str) -> Result<Re, String> {
    let mut parser = Parser {
        pat: pattern.chars().collect(),
        i: 0,
    };
    let re = parser.parse_alt()?;
    if parser.i != parser.pat.len() {
        return Err("unmatched ')' in regex".to_string());
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

    fn parse_alt(&mut self) -> Result<Re, String> {
        let mut alts = vec![self.parse_seq()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.parse_seq()?);
        }
        if alts.len() == 1 {
            Ok(alts.pop().unwrap())
        } else {
            Ok(Re::Alt(alts))
        }
    }

    fn parse_seq(&mut self) -> Result<Re, String> {
        let mut items = Vec::new();
        while let Some(ch) = self.peek() {
            if ch == '|' || ch == ')' {
                break;
            }
            items.push(self.parse_repeat()?);
        }
        Ok(collapse(items))
    }

    fn parse_repeat(&mut self) -> Result<Re, String> {
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
        Ok(Re::Repeat {
            inner: Box::new(atom),
            min,
            max,
        })
    }

    fn parse_bounds(&mut self) -> Result<(u32, Option<u32>), String> {
        self.i += 1;
        let min = self.take_u32()?;
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok((min, Some(min)));
        }
        if self.peek() != Some(',') {
            return Err("invalid '{n,m}' quantifier".to_string());
        }
        self.i += 1;
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok((min, None));
        }
        let max = self.take_u32()?;
        if self.peek() != Some('}') || max < min {
            return Err("invalid '{n,m}' quantifier".to_string());
        }
        self.i += 1;
        Ok((min, Some(max)))
    }

    fn take_u32(&mut self) -> Result<u32, String> {
        let start = self.i;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == start {
            return Err("invalid '{n,m}' quantifier".to_string());
        }
        self.pat[start..self.i]
            .iter()
            .collect::<String>()
            .parse()
            .map_err(|_| "invalid '{n,m}' quantifier".to_string())
    }

    fn parse_atom(&mut self) -> Result<Re, String> {
        let ch = self.peek().ok_or_else(|| "regex ended before an atom".to_string())?;
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
                let esc = self.peek().ok_or_else(|| "dangling '\\' in regex".to_string())?;
                self.i += 1;
                Ok(escape_atom(esc))
            }
            ')' | '|' | '*' | '+' | '?' | '{' => Err(format!("unexpected '{ch}' in regex")),
            _ => {
                self.i += 1;
                Ok(Re::Lit(ch))
            }
        }
    }

    fn parse_group(&mut self) -> Result<Re, String> {
        self.i += 1;
        let inner = self.parse_alt()?;
        if self.peek() != Some(')') {
            return Err("unclosed '(' in regex".to_string());
        }
        self.i += 1;
        Ok(inner)
    }

    fn parse_class(&mut self) -> Result<Re, String> {
        self.i += 1;
        let negated = if self.peek() == Some('^') {
            self.i += 1;
            true
        } else {
            false
        };
        let mut ranges = Vec::new();
        if self.peek() == Some(']') {
            ranges.push((']', ']'));
            self.i += 1;
        }
        while let Some(ch) = self.peek() {
            if ch == ']' {
                break;
            }
            let start = self.class_char()?;
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|next| next != ']') {
                self.i += 1;
                let end = self.class_char()?;
                ranges.push((start, end));
            } else {
                ranges.push((start, start));
            }
        }
        if self.peek() != Some(']') {
            return Err("unclosed '[' in regex".to_string());
        }
        self.i += 1;
        Ok(Re::Class { negated, ranges })
    }

    fn class_char(&mut self) -> Result<char, String> {
        let ch = self.peek().ok_or_else(|| "unclosed '[' in regex".to_string())?;
        self.i += 1;
        if ch != '\\' {
            return Ok(ch);
        }
        let esc = self
            .peek()
            .ok_or_else(|| "dangling '\\' in character class".to_string())?;
        self.i += 1;
        Ok(match esc {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            other => other,
        })
    }
}

fn collapse(mut items: Vec<Re>) -> Re {
    if items.is_empty() {
        Re::Empty
    } else if items.len() == 1 {
        items.pop().unwrap()
    } else {
        Re::Seq(items)
    }
}

fn escape_atom(ch: char) -> Re {
    match ch {
        'd' => digits(false),
        'D' => digits(true),
        'w' => Re::Class {
            negated: false,
            ranges: vec![('A', 'Z'), ('a', 'z'), ('0', '9'), ('_', '_')],
        },
        'W' => Re::Class {
            negated: true,
            ranges: vec![('A', 'Z'), ('a', 'z'), ('0', '9'), ('_', '_')],
        },
        's' => whitespace(false),
        'S' => whitespace(true),
        'n' => Re::Lit('\n'),
        'r' => Re::Lit('\r'),
        't' => Re::Lit('\t'),
        other => Re::Lit(other),
    }
}

fn digits(negated: bool) -> Re {
    Re::Class {
        negated,
        ranges: vec![('0', '9')],
    }
}

fn whitespace(negated: bool) -> Re {
    Re::Class {
        negated,
        ranges: vec![
            (' ', ' '),
            ('\t', '\t'),
            ('\n', '\n'),
            ('\r', '\r'),
            ('\u{000c}', '\u{000c}'),
        ],
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
            if pos == text.len() {
                vec![pos]
            } else {
                Vec::new()
            }
        }
        Re::Class { negated, ranges } => match text.get(pos) {
            Some(ch) if class_hit(*ch, ranges) ^ negated => vec![pos + 1],
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
        let prev_layer = reached.last().unwrap();
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
