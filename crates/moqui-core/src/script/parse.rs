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

//! JavaScript parse through `oxc_parser`. Groovy leftovers are rejected by the service loader.

use oxc_parser::{ParseOptions, Parser};

use crate::context::Error;

/// Parse JavaScript. An expression or a program is accepted. `1 + 1` is accepted.
///
/// Groovy that is not also JavaScript fails the parse: Elvis `?:`, closures `{ x -> }`, map
/// literals `[a:1]`, and `as` casts. Optional chaining `?.` is JavaScript and is accepted.
pub fn parse_js(expr: &str) -> Result<(), Error> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Ok(());
    }
    if parse_expression(expr).is_ok() || parse_program(expr).is_ok() {
        return Ok(());
    }
    let shown = clip(expr);
    Err(Error::new(match groovy_hint(expr) {
        Some(kind) => format!("expression `{shown}` is Groovy ({kind})"),
        None => format!("expression `{shown}` is not JavaScript"),
    }))
}

/// Parse a whole generated script. The error names the first problem the parser reports.
pub(crate) fn check_program(source: &str) -> Result<(), Error> {
    let allocator = std::default::Default::default();
    let source_type = std::default::Default::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        let detail = parsed
            .diagnostics
            .errors()
            .next()
            .map_or_else(|| "syntax error".to_string(), |diagnostic| diagnostic.to_string());
        return Err(Error::new(format!("generated script is not JavaScript: {detail}")));
    }
    Ok(())
}

/// `Parser::parse_expression` stops after the first expression and ignores what follows, so
/// `a b` and `def x = 1` would pass. A parenthesized program must consume every token.
fn parse_expression(source: &str) -> Result<(), Error> {
    let wrapped = format!("({source}\n)");
    let allocator = std::default::Default::default();
    let source_type = std::default::Default::default();
    let parsed = Parser::new(&allocator, &wrapped, source_type).parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        return Err(Error::new("not a JavaScript expression"));
    }
    Ok(())
}

/// A `<script>` body runs inside the function the action macros emit, so a top-level `return`
/// ends the script, as it ended a Groovy script.
fn parse_program(source: &str) -> Result<(), Error> {
    let allocator = std::default::Default::default();
    let source_type = std::default::Default::default();
    let options = ParseOptions {
        allow_return_outside_function: true,
        ..ParseOptions::default()
    };
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(options)
        .parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        return Err(Error::new("not JavaScript"));
    }
    Ok(())
}

/// Why a failed parse looks like Groovy, to make the message useful. It never decides a parse.
fn groovy_hint(text: &str) -> Option<&'static str> {
    if text.contains("?:") {
        return Some("?:");
    }
    if has_groovy_arrow(text) {
        return Some("->");
    }
    if has_closure_call(text) {
        return Some("closure");
    }
    None
}

pub(super) fn has_groovy_arrow(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .windows(2)
        .enumerate()
        .any(|(index, pair)| pair == b"->" && (index == 0 || bytes[index - 1] != b'='))
}

pub(super) fn has_closure_call(text: &str) -> bool {
    const WORDS: &[&str] = &[
        "eachWithIndex",
        "findAll",
        "collect",
        "inject",
        "every",
        "each",
        "find",
        "any",
        "sort",
        "with",
    ];
    let bytes = text.as_bytes();
    for word in WORDS {
        let mut start = 0;
        while let Some(found) = text[start..].find(word) {
            let at = start + found;
            let end = at + word.len();
            let before_ok = at == 0 || !is_ident(bytes[at - 1]);
            let after_ok = end >= bytes.len() || !is_ident(bytes[end]);
            if before_ok && after_ok && text[end..].trim_start().starts_with('{') {
                return true;
            }
            start = end;
        }
    }
    false
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(240).collect();
    if text.chars().nth(240).is_some() {
        out.push_str("...");
    }
    out
}
