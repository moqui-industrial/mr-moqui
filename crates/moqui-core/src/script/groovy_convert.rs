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

//! `moqui convert --groovy`: the mechanical part of the Groovy migration of a component tree such as mantle.
//!
//! Only rewrites whose meaning is the same in JavaScript are made, and only in the expression places of XML (the
//! expression attributes, `${}` in any other attribute or text, and `<script>` and `<expression>` bodies):
//!
//! - a Groovy map literal `[a: 1, (k): v]` becomes `{a: 1, [k]: v}`, and `[:]` becomes `{}`;
//! - `context + [a: 1]` becomes `Object.assign({}, context, {a: 1})`;
//! - Elvis `a ?: b` becomes `a || b`, except before a ternary at the same depth, where `||` would bind differently;
//! - in a `condition` attribute, a name that is the `list` of an `entity-find` or `entity-find-related` in the same file
//!   becomes `(name?.length > 0)`, and `!name` becomes `!name?.length`: an empty list is false in Groovy and an object
//!   (always true) in JavaScript. `?.length` holds for an entity list, an array, and null;
//! - an EDI grammar file (a `.groovy` file that is one list literal) gets a `.js` copy in JavaScript, and an XML
//!   location that names the `.groovy` file names the `.js` one.
//!
//! Everything else is left for the AI pass the pre-scan feeds; `moqui lint --groovy` reports it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::context::Error;

/// What a rewrite changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RewriteKind {
    MapLiteral,
    ContextPlus,
    Elvis,
    EmptyList,
    GrammarFile,
    GrammarLocation,
}

impl RewriteKind {
    pub fn name(self) -> &'static str {
        match self {
            RewriteKind::MapLiteral => "map-literal",
            RewriteKind::ContextPlus => "context-plus",
            RewriteKind::Elvis => "elvis",
            RewriteKind::EmptyList => "empty-list",
            RewriteKind::GrammarFile => "grammar-file",
            RewriteKind::GrammarLocation => "grammar-location",
        }
    }
}

/// One changed expression place: the line of the file it starts on, what changed in it, and its text before and after.
#[derive(Clone, Debug)]
pub struct Rewrite {
    pub line: u32,
    pub kinds: Vec<RewriteKind>,
    pub before: String,
    pub after: String,
}

/// A place the converter saw and did not rewrite, with the reason.
#[derive(Clone, Debug)]
pub struct Left {
    pub line: u32,
    pub reason: &'static str,
    pub text: String,
}

/// The conversion of one file.
#[derive(Clone, Debug, Default)]
pub struct FileConversion {
    pub path: PathBuf,
    /// For a grammar file, the `.js` file written beside it.
    pub written: Option<PathBuf>,
    pub text: String,
    pub rewrites: Vec<Rewrite>,
    pub left: Vec<Left>,
}

/// The conversion of a tree.
#[derive(Clone, Debug, Default)]
pub struct ConvertReport {
    pub files: Vec<FileConversion>,
    /// Grammar files that did not become JavaScript, with the parse error.
    pub failed_grammars: Vec<(PathBuf, String)>,
}

impl ConvertReport {
    pub fn count(&self, kind: RewriteKind) -> usize {
        self.files
            .iter()
            .flat_map(|file| &file.rewrites)
            .filter(|rewrite| rewrite.kinds.contains(&kind))
            .count()
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Groovy conversion");
        let _ = writeln!(out, "files changed: {}", self.files.len());
        for kind in [
            RewriteKind::MapLiteral,
            RewriteKind::ContextPlus,
            RewriteKind::Elvis,
            RewriteKind::EmptyList,
            RewriteKind::GrammarFile,
            RewriteKind::GrammarLocation,
        ] {
            let _ = writeln!(out, "  {:<20} {:>5}", kind.name(), self.count(kind));
        }
        let left: usize = self.files.iter().map(|file| file.left.len()).sum();
        let _ = writeln!(out, "places left for review: {left}");
        for (path, error) in &self.failed_grammars {
            let _ = writeln!(out, "grammar not converted: {}: {error}", path.display());
        }
        for file in &self.files {
            match &file.written {
                Some(written) => {
                    let _ = writeln!(out, "\n{} -> {}", file.path.display(), written.display());
                }
                None => {
                    let _ = writeln!(out, "\n{}", file.path.display());
                }
            }
            for rewrite in &file.rewrites {
                let kinds: Vec<&str> = rewrite.kinds.iter().map(|kind| kind.name()).collect();
                let _ = writeln!(out, "  {}: [{}]", rewrite.line, kinds.join(","));
                let _ = writeln!(out, "    - {}", one_line(&rewrite.before));
                let _ = writeln!(out, "    + {}", one_line(&rewrite.after));
            }
            for left in &file.left {
                let _ = writeln!(out, "  {}: left, {}: {}", left.line, left.reason, one_line(&left.text));
            }
        }
        out
    }
}

/// Convert every XML file and grammar file under `roots`. With `write` the changed files are written in place and each
/// grammar's `.js` file is written beside it; without it nothing is written.
///
/// # Errors
///
/// Returns an error when a directory or file cannot be read or written.
pub fn convert_tree(roots: &[PathBuf], write: bool) -> Result<ConvertReport, Error> {
    let mut xml = Vec::new();
    let mut groovy = Vec::new();
    for root in roots {
        super::screen_lint::collect_all(root, &mut xml, &mut groovy)?;
    }
    xml.sort();
    groovy.sort();
    let mut report = ConvertReport::default();
    let mut grammars = BTreeSet::new();
    for path in groovy {
        let text = read(&path)?;
        if !is_grammar(&text) {
            continue;
        }
        let converted = convert_literals(&text);
        if let Err(err) = super::parse_js(&converted) {
            report.failed_grammars.push((path, err.to_string()));
            continue;
        }
        let target = path.with_extension("js");
        if write {
            std::fs::write(&target, &converted).map_err(|err| Error::new(format!("{}: {err}", target.display())))?;
        }
        if let Some(stem) = path.file_stem() {
            grammars.insert(stem.to_string_lossy().to_string());
        }
        report.files.push(FileConversion {
            path,
            written: Some(target),
            rewrites: vec![Rewrite {
                line: 1,
                kinds: vec![RewriteKind::GrammarFile],
                before: first_line(&text),
                after: first_line(&converted),
            }],
            text: converted,
            left: Vec::new(),
        });
    }
    for path in xml {
        let text = read(&path)?;
        let mut file = convert_xml(&text);
        let located = rename_grammar_locations(&file.text, &grammars, &mut file.rewrites);
        file.text = located;
        if file.rewrites.is_empty() && file.left.is_empty() {
            continue;
        }
        if write && !file.rewrites.is_empty() {
            std::fs::write(&path, &file.text).map_err(|err| Error::new(format!("{}: {err}", path.display())))?;
        }
        file.path = path;
        report.files.push(file);
    }
    Ok(report)
}

fn read(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|err| Error::new(format!("{}: {err}", path.display())))
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with("//"))
        .unwrap_or("")
        .to_string()
}

/// A grammar file is one Groovy list literal, after line comments.
fn is_grammar(text: &str) -> bool {
    let body: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim();
    body.starts_with('[') && body.ends_with(']')
}

/// `component://.../X12Envelope.groovy` becomes `.js` for each grammar that was converted.
fn rename_grammar_locations(text: &str, grammars: &BTreeSet<String>, rewrites: &mut Vec<Rewrite>) -> String {
    let mut out = text.to_string();
    for stem in grammars {
        let from = format!("/{stem}.groovy\"");
        let to = format!("/{stem}.js\"");
        let mut search = 0;
        while let Some(found) = out[search..].find(&from) {
            let at = search + found;
            let line = line_of(&out, at);
            let start = out[..at].rfind('"').map_or(at, |quote| quote + 1);
            let before = out[start..at + from.len() - 1].to_string();
            out.replace_range(at..at + from.len(), &to);
            let after = out[start..at + to.len() - 1].to_string();
            rewrites.push(Rewrite {
                line,
                kinds: vec![RewriteKind::GrammarLocation],
                before,
                after,
            });
            search = at + to.len();
        }
    }
    rewrites.sort_by_key(|rewrite| rewrite.line);
    out
}

/// Expression attributes: the whole value is an expression.
fn is_expression_attr(element: &str, attr: &str) -> bool {
    match attr {
        "condition" | "from" | "in-map" | "parameter-map" | "text-map" | "default" => {
            attr != "default" || element == "parameter"
        }
        "list" => matches!(
            element,
            "section-iterate"
                | "iterate"
                | "form-list"
                | "tree-sub-node"
                | "list-options"
                | "order-map-list"
                | "filter-map-list"
        ),
        "map" => matches!(element, "form-single" | "iterate"),
        _ => false,
    }
}

/// Convert the expression places of one XML file. The rest of the text is copied as it is.
pub fn convert_xml(text: &str) -> FileConversion {
    let mut file = FileConversion::default();
    let lists = entity_find_lists(text);
    let mut out = String::with_capacity(text.len());
    let mut stack: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if text[index..].starts_with("<!--") {
            let end = text[index..].find("-->").map_or(text.len(), |end| index + end + 3);
            out.push_str(&text[index..end]);
            index = end;
        } else if text[index..].starts_with("<![CDATA[") {
            let start = index + "<![CDATA[".len();
            let end = text[start..].find("]]>").map_or(text.len(), |end| start + end);
            out.push_str("<![CDATA[");
            let in_script = stack
                .last()
                .is_some_and(|name| name == "script" || name == "expression");
            out.push_str(&convert_place(&mut file, text, start, &text[start..end], in_script));
            out.push_str("]]>");
            index = (end + 3).min(text.len());
        } else if text[index..].starts_with("<?") || text[index..].starts_with("<!") {
            let end = text[index..].find('>').map_or(text.len(), |end| index + end + 1);
            out.push_str(&text[index..end]);
            index = end;
        } else if text[index..].starts_with("</") {
            let end = text[index..].find('>').map_or(text.len(), |end| index + end + 1);
            stack.pop();
            out.push_str(&text[index..end]);
            index = end;
        } else if bytes[index] == b'<' {
            index = convert_tag(&mut file, text, index, &mut out, &mut stack, &lists);
        } else {
            let end = text[index..].find('<').map_or(text.len(), |end| index + end);
            let in_script = stack
                .last()
                .is_some_and(|name| name == "script" || name == "expression");
            out.push_str(&convert_place(&mut file, text, index, &text[index..end], in_script));
            index = end;
        }
    }
    file.text = out;
    file
}

/// Copy one start tag, converting the expression places of its attributes. Returns the index after the tag.
fn convert_tag(
    file: &mut FileConversion,
    text: &str,
    start: usize,
    out: &mut String,
    stack: &mut Vec<String>,
    lists: &BTreeSet<String>,
) -> usize {
    let bytes = text.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>' && bytes[index] != b'/' {
        index += 1;
    }
    let element = text[start + 1..index].to_string();
    out.push_str(&text[start..index]);
    loop {
        let rest = index;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            out.push_str(&text[rest..]);
            return bytes.len();
        }
        if bytes[index] == b'>' || text[index..].starts_with("/>") {
            let self_closing = bytes[index] == b'/';
            let end = if self_closing { index + 2 } else { index + 1 };
            out.push_str(&text[rest..end]);
            if !self_closing {
                stack.push(element);
            }
            return end;
        }
        let name_start = index;
        while index < bytes.len() && bytes[index] != b'=' && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>'
        {
            index += 1;
        }
        let attr = text[name_start..index].to_string();
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'=') {
            index += 1;
        }
        if index >= bytes.len() || (bytes[index] != b'"' && bytes[index] != b'\'') {
            out.push_str(&text[rest..index]);
            continue;
        }
        let quote = bytes[index];
        let value_start = index + 1;
        let value_end = text[value_start..]
            .find(quote as char)
            .map_or(text.len(), |end| value_start + end);
        out.push_str(&text[rest..value_start]);
        let value = &text[value_start..value_end];
        if attr == "condition" && is_expression_attr(&element, &attr) {
            let converted = convert_place(file, text, value_start, value, true);
            let (checked, count) = empty_list(&converted, lists);
            if count > 0 {
                file.rewrites.push(Rewrite {
                    line: line_of(text, value_start),
                    kinds: vec![RewriteKind::EmptyList],
                    before: converted,
                    after: checked.clone(),
                });
            }
            out.push_str(&checked);
        } else if is_expression_attr(&element, &attr) {
            out.push_str(&convert_place(file, text, value_start, value, true));
        } else {
            out.push_str(&convert_place(file, text, value_start, value, false));
        }
        index = value_end;
        if index < bytes.len() {
            out.push(quote as char);
            index += 1;
        }
    }
}

/// Convert one place: the whole text when it is an expression or a script, else each `${}` in it.
fn convert_place(file: &mut FileConversion, whole: &str, offset: usize, text: &str, expression: bool) -> String {
    if expression {
        return convert_expression_at(file, whole, offset, text);
    }
    if !text.contains("${") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end) in dollar_spans(text) {
        out.push_str(&text[last..start]);
        out.push_str(&convert_expression_at(file, whole, offset + start, &text[start..end]));
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

fn convert_expression_at(file: &mut FileConversion, whole: &str, offset: usize, text: &str) -> String {
    let line = line_of(whole, offset);
    let (converted, kinds, left) = rewrite_expression(text);
    for reason in left {
        file.left.push(Left {
            line,
            reason,
            text: text.to_string(),
        });
    }
    if !kinds.is_empty() {
        file.rewrites.push(Rewrite {
            line,
            kinds,
            before: text.to_string(),
            after: converted.clone(),
        });
    }
    converted
}

/// The `list` names of the `entity-find` and `entity-find-related` elements of a file: names that always hold a list.
fn entity_find_lists(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for tag in ["<entity-find ", "<entity-find-related "] {
        let mut rest = text;
        while let Some(at) = rest.find(tag) {
            let after = &rest[at + tag.len()..];
            let end = after.find('>').unwrap_or(after.len());
            let attrs = &after[..end];
            if let Some(start) = attrs.find(" list=\"").map(|at| at + 7).or_else(|| attrs.strip_prefix("list=\"").map(|_| 6)) {
                if let Some(len) = attrs[start..].find('"') {
                    let name = &attrs[start..start + len];
                    if !name.is_empty() && name.bytes().all(is_ident) {
                        names.insert(name.to_string());
                    }
                }
            }
            rest = &after[end..];
        }
    }
    names
}

/// `name` to `(name?.length > 0)` and `!name` to `!name?.length` for each standalone name of `lists` in a condition.
/// Strings are skipped, and a name used as an object (`name.size()`, `name[0]`, `a.name`) is left alone.
fn empty_list(text: &str, lists: &BTreeSet<String>) -> (String, usize) {
    if lists.is_empty() {
        return (text.to_string(), 0);
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 16);
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\'' || byte == b'"' {
            let end = text[index + 1..].find(byte as char).map_or(text.len(), |end| index + 1 + end + 1);
            out.push_str(&text[index..end]);
            index = end;
            continue;
        }
        if !is_ident(byte) || (index > 0 && is_ident(bytes[index - 1])) {
            out.push(byte as char);
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_ident(bytes[index]) {
            index += 1;
        }
        let name = &text[start..index];
        let before = text[..start].trim_end();
        let next = text[index..].trim_start().as_bytes().first().copied();
        let used_as_object = matches!(next, Some(b'.' | b'[' | b'(' | b'?' | b'='))
            || before.ends_with('.')
            || before.ends_with("?.");
        if !lists.contains(name) || used_as_object || name.as_bytes()[0].is_ascii_digit() {
            out.push_str(name);
            continue;
        }
        count += 1;
        if out.trim_end().ends_with('!') && !out.trim_end().ends_with("!!") {
            out.push_str(&format!("{name}?.length"));
        } else {
            out.push_str(&format!("({name}?.length > 0)"));
        }
    }
    (out, count)
}

fn line_of(text: &str, offset: usize) -> u32 {
    u32::try_from(text[..offset.min(text.len())].matches('\n').count() + 1).unwrap_or(u32::MAX)
}

/// The inside of each `${...}` of `text`, as byte ranges.
fn dollar_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'$' && bytes[index + 1] == b'{' && (index == 0 || bytes[index - 1] != b'\\') {
            let start = index + 2;
            let mut depth = 1;
            let mut end = start;
            let mut quote = 0u8;
            while end < bytes.len() {
                let byte = bytes[end];
                if quote != 0 {
                    if byte == b'\\' {
                        end += 1;
                    } else if byte == quote {
                        quote = 0;
                    }
                } else if byte == b'\'' || byte == b'"' {
                    quote = byte;
                } else if byte == b'{' {
                    depth += 1;
                } else if byte == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                end += 1;
            }
            let end = end.min(bytes.len());
            out.push((start, end));
            index = end;
        }
        index += 1;
    }
    out
}

/// Rewrite one expression or script. Returns the text, what was rewritten, and why any place was left.
pub fn rewrite_expression(text: &str) -> (String, Vec<RewriteKind>, Vec<&'static str>) {
    let mut kinds = BTreeSet::new();
    let mut left = Vec::new();
    let (literals, maps, plus_left) = convert_literals_counted(text);
    if maps > 0 {
        kinds.insert(RewriteKind::MapLiteral);
    }
    if plus_left > 0 {
        left.push("map plus: `x + [a: b]` is a copy with entries in Groovy and text in JavaScript");
    }
    let (context, plus) = context_plus(&literals);
    if plus > 0 {
        kinds.insert(RewriteKind::ContextPlus);
        // The literal in `context + [...]` was a map rewrite only to make the copy.
        if maps == plus {
            kinds.remove(&RewriteKind::MapLiteral);
        }
    }
    let (elvis, rewritten, skipped) = elvis(&context);
    if rewritten > 0 {
        kinds.insert(RewriteKind::Elvis);
    }
    if skipped > 0 {
        left.push("Elvis before a ternary");
    }
    (elvis, kinds.into_iter().collect(), left)
}

/// Every Groovy list and map literal of `text` in JavaScript: maps become objects, lists stay arrays. Strings and
/// comments are copied as they are, and an index (`a[0]`) is not a literal.
pub fn convert_literals(text: &str) -> String {
    convert_literals_counted(text).0
}

/// The converted text, the maps rewritten, and the maps left because a `+` that is not `context +` comes before them.
fn convert_literals_counted(text: &str) -> (String, usize, usize) {
    let mut counts = (0, 0);
    let out = convert_range(text, &mut counts);
    (out, counts.0, counts.1)
}

/// Whether the text before a map literal ends in `+`, and whether the operand of that `+` is `context`.
fn after_plus(before: &str) -> Option<bool> {
    let left = before.trim_end().strip_suffix('+')?;
    if left.ends_with('+') {
        return None;
    }
    let left = left.trim_end();
    let word: String = left
        .chars()
        .rev()
        .take_while(|c| is_ident(u8::try_from(*c).unwrap_or(b' ')) || *c == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Some(word == "context")
}

fn convert_range(text: &str, counts: &mut (usize, usize)) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(end) = skip_string_or_comment(text, index) {
            out.push_str(&text[index..end]);
            index = end;
            continue;
        }
        if byte == b'[' && !is_index(&out) {
            if let Some(close) = matching(text, index, b'[', b']') {
                let inner = &text[index + 1..close];
                let converted = convert_range(inner, counts);
                if is_map(inner) && after_plus(&out) == Some(false) {
                    // `x + [a: b]` needs the operand's type to convert; leave it whole.
                    counts.1 += 1;
                    out.push('[');
                    out.push_str(&converted);
                    out.push(']');
                } else if is_map(inner) {
                    counts.0 += 1;
                    out.push('{');
                    out.push_str(&dynamic_keys(&converted));
                    out.push('}');
                } else {
                    out.push('[');
                    out.push_str(&converted);
                    out.push(']');
                }
                index = close + 1;
                continue;
            }
        }
        let ch = text[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// The end of the string literal or comment at `index`, when one starts there.
fn skip_string_or_comment(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let byte = bytes[index];
    if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
        return Some(text[index..].find('\n').map_or(text.len(), |end| index + end));
    }
    if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
        return Some(
            text[index + 2..]
                .find("*/")
                .map_or(text.len(), |end| index + 2 + end + 2),
        );
    }
    if byte == b'\'' || byte == b'"' || byte == b'`' {
        for quote in ["'''", "\"\"\""] {
            if text[index..].starts_with(quote) {
                return Some(
                    text[index + 3..]
                        .find(quote)
                        .map_or(text.len(), |end| index + 3 + end + 3),
                );
            }
        }
        let mut end = index + 1;
        while end < bytes.len() {
            if bytes[end] == b'\\' {
                end += 2;
                continue;
            }
            if bytes[end] == byte {
                return Some(end + 1);
            }
            end += 1;
        }
        return Some(bytes.len());
    }
    None
}

/// Whether a `[` that follows `before` indexes the value before it rather than starting a literal.
fn is_index(before: &str) -> bool {
    let trimmed = before.trim_end();
    let Some(last) = trimmed.chars().last() else {
        return false;
    };
    if last == ')' || last == ']' || last == '}' {
        return trimmed.len() == before.len();
    }
    if !(last.is_alphanumeric() || last == '_' || last == '$') {
        return false;
    }
    // A keyword before `[` starts a literal: `return [a: 1]`, `in [1, 2]`.
    let word: String = trimmed
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    !matches!(
        word.as_str(),
        "return" | "in" | "case" | "else" | "typeof" | "yield" | "new" | "of"
    )
}

/// The index of the bracket that closes the one at `open`, skipping strings and nested brackets of every kind.
fn matching(text: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut index = open;
    while index < bytes.len() {
        if index > open {
            if let Some(end) = skip_string_or_comment(text, index) {
                index = end;
                continue;
            }
        }
        let byte = bytes[index];
        if byte == opening || (byte != opening && matches!(byte, b'[' | b'(' | b'{')) {
            depth += 1;
        } else if byte == closing || matches!(byte, b']' | b')' | b'}') {
            depth -= 1;
            if depth == 0 {
                return (byte == closing).then_some(index);
            }
        }
        index += 1;
    }
    None
}

/// A Groovy map literal: `:` alone, or a first entry that is a key and a colon. A ternary in a list is not a key.
fn is_map(inner: &str) -> bool {
    let trimmed = inner.trim();
    if trimmed == ":" {
        return true;
    }
    let bytes = trimmed.as_bytes();
    let key_end = if let Some(end) = bytes
        .first()
        .filter(|byte| matches!(**byte, b'\'' | b'"'))
        .and_then(|_| skip_string_or_comment(trimmed, 0))
    {
        end
    } else if bytes.first() == Some(&b'(') {
        match matching(trimmed, 0, b'(', b')') {
            Some(close) => close + 1,
            None => return false,
        }
    } else {
        let end = bytes
            .iter()
            .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$'))
            .unwrap_or(bytes.len());
        if end == 0 || bytes[0].is_ascii_digit() {
            return false;
        }
        end
    };
    let rest = trimmed[key_end..].trim_start();
    rest.starts_with(':') && !rest.starts_with("::")
}

/// `[:]` is `{}`, and a key in parentheses is a computed key.
fn dynamic_keys(inner: &str) -> String {
    if inner.trim() == ":" {
        return String::new();
    }
    let bytes = inner.as_bytes();
    let mut out = String::with_capacity(inner.len());
    let mut index = 0;
    let mut entry_start = true;
    let mut depth = 0i32;
    while index < bytes.len() {
        if let Some(end) = skip_string_or_comment(inner, index) {
            out.push_str(&inner[index..end]);
            index = end;
            entry_start = false;
            continue;
        }
        let byte = bytes[index];
        if entry_start && byte.is_ascii_whitespace() {
            out.push(byte as char);
            index += 1;
            continue;
        }
        if entry_start && depth == 0 && byte == b'(' {
            if let Some(close) = matching(inner, index, b'(', b')') {
                if inner[close + 1..].trim_start().starts_with(':') {
                    out.push('[');
                    out.push_str(&inner[index + 1..close]);
                    out.push(']');
                    index = close + 1;
                    entry_start = false;
                    continue;
                }
            }
        }
        entry_start = false;
        match byte {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => depth -= 1,
            b',' if depth == 0 => entry_start = true,
            _ => {}
        }
        let ch = inner[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// `context + {…}`, after the literal pass made the map an object, becomes `Object.assign({}, context, {…})`.
fn context_plus(text: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut index = 0;
    let bytes = text.as_bytes();
    while index < bytes.len() {
        if let Some(end) = skip_string_or_comment(text, index) {
            out.push_str(&text[index..end]);
            index = end;
            continue;
        }
        if text[index..].starts_with("context")
            && (index == 0 || !is_ident(bytes[index - 1]))
            && !bytes.get(index + 7).is_some_and(|byte| is_ident(*byte))
        {
            let after = &text[index + 7..];
            let plus = after.len() - after.trim_start().len();
            if after.trim_start().starts_with('+') {
                let rest_at = index + 7 + plus + 1;
                let brace = rest_at + (text[rest_at..].len() - text[rest_at..].trim_start().len());
                if bytes.get(brace) == Some(&b'{') {
                    if let Some(close) = matching(text, brace, b'{', b'}') {
                        out.push_str("Object.assign({}, context, ");
                        out.push_str(&text[brace..=close]);
                        out.push(')');
                        count += 1;
                        index = close + 1;
                        continue;
                    }
                }
            }
        }
        let ch = text[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    (out, count)
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// `a ?: b` as `a || b`. A `(?:` is a regular expression group and is not Elvis. An Elvis followed by a ternary at
/// its depth is left, because `a ?: b ? c : d` is `a ?: (b ? c : d)` in Groovy and `(a || b) ? c : d` in JavaScript.
fn elvis(text: &str) -> (String, usize, usize) {
    let mut out = String::with_capacity(text.len());
    let mut rewritten = 0;
    let mut skipped = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = skip_string_or_comment(text, index) {
            out.push_str(&text[index..end]);
            index = end;
            continue;
        }
        if text[index..].starts_with("?:") && !out.ends_with('(') {
            if ternary_follows(&text[index + 2..]) {
                skipped += 1;
                out.push_str("?:");
            } else {
                rewritten += 1;
                out.push_str("||");
            }
            index += 2;
            continue;
        }
        let ch = text[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    (out, rewritten, skipped)
}

/// Whether a `?` of a ternary comes before the operand ends: a closing bracket, a comma or a statement end.
fn ternary_follows(rest: &str) -> bool {
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = skip_string_or_comment(rest, index) {
            index = end;
            continue;
        }
        match bytes[index] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            b',' | b';' | b'\n' if depth == 0 => return false,
            b'?' if depth == 0 => {
                let next = bytes.get(index + 1).copied();
                if next == Some(b':') {
                    index += 2;
                    continue;
                }
                if next != Some(b'.') && next != Some(b'?') {
                    return true;
                }
            }
            _ => {}
        }
        index += 1;
    }
    false
}

fn one_line(text: &str) -> String {
    let joined: Vec<&str> = text.split_whitespace().collect();
    let line = joined.join(" ");
    if line.chars().count() > 240 {
        let mut clipped: String = line.chars().take(240).collect();
        clipped.push_str("...");
        clipped
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(text: &str) -> String {
        rewrite_expression(text).0
    }

    #[test]
    fn map_literals_become_objects_and_lists_and_indexes_stay() {
        assert_eq!(rewrite("[a: 1, b: 'x']"), "{a: 1, b: 'x'}");
        assert_eq!(rewrite("[:]"), "{}");
        assert_eq!(rewrite("[1, 2, [a:3]]"), "[1, 2, {a:3}]");
        assert_eq!(rewrite("list[0].x"), "list[0].x");
        assert_eq!(rewrite("map['k']"), "map['k']");
        assert_eq!(rewrite("[(name): v, 'q r': 2]"), "{[name]: v, 'q r': 2}");
        assert_eq!(rewrite("[x ? 1 : 2, y]"), "[x ? 1 : 2, y]");
        assert_eq!(rewrite("f('[a: 1]', [b: \"[c:2]\"])"), "f('[a: 1]', {b: \"[c:2]\"})");
        assert_eq!(rewrite("return [ok: true]"), "return {ok: true}");
        assert_eq!(rewrite("[a: m[0], b: [c: f(x)[1]]]"), "{a: m[0], b: {c: f(x)[1]}}");
    }

    #[test]
    fn context_plus_becomes_an_object_assign() {
        let (text, kinds, _) = rewrite_expression("context + [flattenDocument: false]");
        assert_eq!(text, "Object.assign({}, context, {flattenDocument: false})");
        assert_eq!(kinds, vec![RewriteKind::ContextPlus]);
        assert_eq!(
            rewrite("context+[a:1, b:[c:2]]"),
            "Object.assign({}, context, {a:1, b:{c:2}})"
        );
        let (text, kinds, left) = rewrite_expression("baseMap + [a: 1]");
        assert_eq!(text, "baseMap + [a: 1]");
        assert!(kinds.is_empty());
        assert_eq!(left.len(), 1);
        assert_eq!(rewrite("mycontext + [a: 1]"), "mycontext + [a: 1]");
        assert_eq!(rewrite("x = [a: 1] + 2"), "x = {a: 1} + 2");
    }

    #[test]
    fn elvis_becomes_or_unless_a_ternary_follows() {
        assert_eq!(rewrite("a ?: 'x'"), "a || 'x'");
        assert_eq!(
            rewrite("${product?.productName ?: productId}"),
            "${product?.productName || productId}"
        );
        assert_eq!(rewrite("a?:b?:c"), "a||b||c");
        assert_eq!(rewrite("'?:' + x"), "'?:' + x");
        assert_eq!(rewrite("s.replaceAll(/(?:x)/, '')"), "s.replaceAll(/(?:x)/, '')");
        let (text, kinds, left) = rewrite_expression("a ?: b ? c : d");
        assert_eq!(text, "a ?: b ? c : d");
        assert!(kinds.is_empty());
        assert_eq!(left, vec!["Elvis before a ternary"]);
        assert_eq!(rewrite("f(a ?: b, c ? d : e)"), "f(a || b, c ? d : e)");
    }

    #[test]
    fn xml_places_are_converted_and_the_rest_is_copied() {
        let xml = r#"<services>
    <!-- a ?: b in a comment -->
    <service verb="get" noun="Thing">
        <description>Prose with ?: stays</description>
        <actions>
            <service-call name="x.y#Z" in-map="context + [flattenDocument:false]" out-map="context"/>
            <set field="name" from="product?.productName ?: productId"/>
            <message>Name ${name ?: 'none'}</message>
            <script><![CDATA[def m = [a: 1]; out = m.a ?: 2]]></script>
        </actions>
    </service>
</services>"#;
        let file = convert_xml(xml);
        assert!(
            file.text
                .contains("in-map=\"Object.assign({}, context, {flattenDocument:false})\""),
            "{}",
            file.text
        );
        assert!(
            file.text.contains("from=\"product?.productName || productId\""),
            "{}",
            file.text
        );
        assert!(file.text.contains("Name ${name || 'none'}"), "{}", file.text);
        assert!(file.text.contains("def m = {a: 1}; out = m.a || 2"), "{}", file.text);
        assert!(file.text.contains("<!-- a ?: b in a comment -->"), "{}", file.text);
        assert!(file.text.contains("Prose with ?: stays"), "{}", file.text);
        assert_eq!(file.rewrites.len(), 4);
        assert_eq!(file.rewrites[0].line, 6);
    }

    #[test]
    fn a_grammar_file_becomes_javascript_the_edi_handler_reads() {
        let groovy = "// File Structure\n[\n    [ID:'ISA',MIN:0,MAX:99999,\n        LEVEL: [\n            [ID:'GS',MIN:0,MAX:99999, LEVEL: [\n                    [ID:'ST',MIN:0,MAX:99999],\n                ]\n            ],\n            [ID:'IEA',MIN:1,MAX:1]\n        ]\n    ]\n]\n";
        assert!(is_grammar(groovy));
        let js = convert_literals(groovy);
        assert!(js.contains("{ID:'ISA',MIN:0,MAX:99999,"), "{js}");
        assert!(js.contains("{ID:'ST',MIN:0,MAX:99999},"), "{js}");
        super::super::parse_js(&js).unwrap();
        assert!(!is_grammar("def x = 1\n[a]"));
    }

    #[test]
    fn a_tree_converts_grammars_and_their_locations_and_writes_only_when_asked() {
        let dir = std::env::temp_dir().join(format!("moqui-groovy-convert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("service/edi")).unwrap();
        std::fs::write(dir.join("service/edi/X12Test.groovy"), "[[ID:'ST', MIN:1, MAX:1]]\n").unwrap();
        let services = r#"<services><service verb="a" noun="B"><in-parameters>
            <parameter name="bodyLocation" default-value="component://c/service/edi/X12Test.groovy"/></in-parameters>
            <actions><set field="x" from="y ?: 1"/></actions></service></services>"#;
        std::fs::write(dir.join("service/EdiServices.xml"), services).unwrap();
        let report = convert_tree(std::slice::from_ref(&dir), false).unwrap();
        assert_eq!(report.count(RewriteKind::GrammarFile), 1);
        assert_eq!(report.count(RewriteKind::GrammarLocation), 1);
        assert_eq!(report.count(RewriteKind::Elvis), 1);
        assert!(!dir.join("service/edi/X12Test.js").exists(), "a dry run writes nothing");
        assert!(report.render().contains("X12Test.js"));
        convert_tree(std::slice::from_ref(&dir), true).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("service/edi/X12Test.js")).unwrap(),
            "[{ID:'ST', MIN:1, MAX:1}]\n"
        );
        let written = std::fs::read_to_string(dir.join("service/EdiServices.xml")).unwrap();
        assert!(written.contains("component://c/service/edi/X12Test.js\""), "{written}");
        assert!(written.contains("from=\"y || 1\""), "{written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_found_list_in_a_condition_is_tested_for_rows() {
        let xml = r#"<actions>
            <entity-find entity-name="a.B" list="fooList"><econdition field-name="x"/></entity-find>
            <if condition="fooList"><log message="x"/></if>
            <if condition="!fooList &amp;&amp; otherList"><log message="x"/></if>
            <if condition="fooList.size() &gt; 1 || fooList[0] || 'fooList'"><log message="x"/></if>
        </actions>"#;
        let converted = convert_xml(xml);
        assert!(converted.text.contains(r#"<if condition="(fooList?.length > 0)">"#), "{}", converted.text);
        assert!(converted.text.contains(r#"<if condition="!fooList?.length &amp;&amp; otherList">"#), "{}", converted.text);
        assert!(converted.text.contains(r#"<if condition="fooList.size() &gt; 1 || fooList[0] || 'fooList'">"#), "{}", converted.text);
        assert_eq!(converted.rewrites.iter().filter(|rewrite| rewrite.kinds == vec![RewriteKind::EmptyList]).count(), 2);
    }
}
