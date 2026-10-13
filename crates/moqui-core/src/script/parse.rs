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

use oxc_parser::Parser;

use crate::context::Error;

/// Parse JavaScript. An expression or a program is accepted. `1 + 1` is accepted.
///
/// Groovy leftovers fail before the parser, because oxc treats `?.` as optional chaining.
pub fn parse_js(expr: &str) -> Result<(), Error> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Ok(());
    }
    let shown = clip(expr);
    if let Some(kind) = groovy_marker(expr) {
        return Err(Error::new(format!("expression `{shown}` is Groovy ({kind})")));
    }
    if parse_expression(expr).is_ok() || parse_program(expr).is_ok() {
        return Ok(());
    }
    Err(Error::new(format!("expression `{shown}` is not JavaScript")))
}

fn parse_expression(source: &str) -> Result<(), Error> {
    // `Allocator` and `SourceType` are arguments of `Parser::new`. They stay inferred so this
    // crate does not name those dependencies.
    let allocator = std::default::Default::default();
    let source_type = std::default::Default::default();
    match Parser::new(&allocator, source, source_type).parse_expression() {
        Ok(_) => Ok(()),
        Err(_) => Err(Error::new("not a JavaScript expression")),
    }
}

fn parse_program(source: &str) -> Result<(), Error> {
    let allocator = std::default::Default::default();
    let source_type = std::default::Default::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        return Err(Error::new("not JavaScript"));
    }
    Ok(())
}

fn groovy_marker(text: &str) -> Option<&'static str> {
    if text.contains("?:") {
        return Some("?:");
    }
    if text.contains("?.") {
        return Some("?.");
    }
    if has_groovy_arrow(text) {
        return Some("->");
    }
    if has_closure_call(text) {
        return Some("closure");
    }
    None
}

fn has_groovy_arrow(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .windows(2)
        .enumerate()
        .any(|(index, pair)| pair == b"->" && (index == 0 || bytes[index - 1] != b'='))
}

fn has_closure_call(text: &str) -> bool {
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
