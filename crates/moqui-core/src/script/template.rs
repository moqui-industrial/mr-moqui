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

//! The fancy template dialect: `if`, `list` and `assign` tags plus `${expr}`, compiled to one
//! JavaScript function and cached.
//!
//! The trusted renderer runs that function on the script runtime with the live context. The
//! locked-down renderer runs it in a separate QuickJS context that holds nothing but the plain
//! snapshot of a map, `format` and `localize`. Tags are listed in Templating in
//! `plans/Architecture-Proposal.md`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moqui_lib::object::Value;
use rquickjs::function::Opt;
use rquickjs::{CatchResultExt, Context, Ctx, Exception, Function, Persistent, Runtime, Value as JsValue};

use crate::context::{Error, ExecutionContextFactory};

const MAX_COMPILED: usize = 4096;
const LOCKED_TIME_LIMIT: Duration = Duration::from_secs(5);
const LOCKED_MEMORY_LIMIT: usize = 32 * 1024 * 1024;

/// How `${expr}` output is encoded. `Html` also serves XML: both escape `& < > " '`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Escape {
    None,
    Html,
}

/// Which names the template sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Live,
    Snapshot,
}

/// A renderer for one template syntax. Conf maps a file extension to an implementation.
pub trait TemplateRenderer: Send + Sync {
    /// Render `source`. Trusted renderers read the live context and add the entries of `data` in a
    /// frame of their own. A locked-down renderer sees only `data`.
    ///
    /// # Errors
    ///
    /// Returns an error when the template does not compile or throws.
    fn render(&self, source: &str, data: Option<&Value>) -> Result<String, Error>;

    /// The compiled form, for a renderer that has one. Used to check a template without running it.
    ///
    /// # Errors
    ///
    /// Returns an error when the template does not compile.
    fn check(&self, source: &str) -> Result<(), Error>;
}

/// The trusted fancy renderer: the live context, `ec` and `sri`.
pub struct FancyRenderer {
    escape: Escape,
}

/// The locked-down fancy renderer: only the snapshot of the map passed in.
pub struct LockedRenderer {
    escape: Escape,
}

impl TemplateRenderer for FancyRenderer {
    fn render(&self, source: &str, data: Option<&Value>) -> Result<String, Error> {
        let compiled = compiled(source, self.escape, Scope::Live)?;
        let framed = data.is_some();
        if let Some(Value::Map(entries)) = data {
            super::push_context_frame();
            super::put_context_all(entries);
        } else if framed {
            super::push_context_frame();
        }
        let result = super::runtime::run_template_function(&compiled);
        if framed {
            super::pop_context_frame();
        }
        result
    }

    fn check(&self, source: &str) -> Result<(), Error> {
        compiled(source, self.escape, Scope::Live).map(|_| ())
    }
}

impl TemplateRenderer for LockedRenderer {
    fn render(&self, source: &str, data: Option<&Value>) -> Result<String, Error> {
        let compiled = compiled(source, self.escape, Scope::Snapshot)?;
        let json = data.map_or_else(|| "{}".to_string(), |value| unwrap_json(value).to_string());
        render_locked(&compiled, &json)
    }

    fn check(&self, source: &str) -> Result<(), Error> {
        compiled(source, self.escape, Scope::Snapshot).map(|_| ())
    }
}

/// The renderer a conf `class` names: `fancy`, `fancy-html`, `fancy-locked` or `fancy-locked-html`.
pub fn renderer_by_class(class: &str) -> Option<Arc<dyn TemplateRenderer>> {
    Some(match class {
        "fancy" => Arc::new(FancyRenderer { escape: Escape::None }),
        "fancy-html" => Arc::new(FancyRenderer { escape: Escape::Html }),
        "fancy-locked" => Arc::new(LockedRenderer { escape: Escape::None }),
        "fancy-locked-html" => Arc::new(LockedRenderer { escape: Escape::Html }),
        _ => return None,
    })
}

/// Render `source` as a trusted fancy template with the live context.
///
/// # Errors
///
/// Returns an error when the template does not compile or throws.
pub fn render_trusted(source: &str, escape: Escape) -> Result<String, Error> {
    FancyRenderer { escape }.render(source, None)
}

/// Render `source` in the locked-down renderer with the snapshot of `data`.
///
/// # Errors
///
/// Returns an error when the template does not compile or throws.
pub fn render_locked_with(source: &str, escape: Escape, data: &Value) -> Result<String, Error> {
    LockedRenderer { escape }.render(source, Some(data))
}

/// Render `source` as a trusted fancy template on the context of a running host call.
pub(crate) fn render_trusted_here(ctx: &Ctx<'_>, source: &str, escape: Escape) -> Result<String, Error> {
    let compiled = compiled(source, escape, Scope::Live)?;
    super::runtime::run_template_function_here(ctx, &compiled)
}

/// Check that a fancy template compiles, including a parse of every expression.
///
/// # Errors
///
/// Returns the first compile error with its line.
pub fn check_template(source: &str) -> Result<(), Error> {
    compile(source, Escape::None, Scope::Live).map(|_| ())
}

/// The renderers registered in conf, by extension, and the compiled form of file templates.
pub struct TemplateRegistry {
    by_extension: Vec<(String, Arc<dyn TemplateRenderer>)>,
    files: Mutex<HashMap<String, (Option<i64>, String)>>,
}

impl TemplateRegistry {
    pub fn of(factory: &ExecutionContextFactory) -> Arc<Self> {
        factory.extension(|factory| Self::parse(factory.conf_text()))
    }

    fn parse(conf_text: &str) -> Self {
        let mut by_extension = Vec::new();
        if let Ok(root) = moqui_lib::node::MNode::parse_text("moqui-conf", conf_text) {
            for facade in root.children_named("resource-facade") {
                for entry in facade.children_named("template-renderer") {
                    let (Some(extension), Some(class)) = (entry.attribute("extension"), entry.attribute("class"))
                    else {
                        continue;
                    };
                    if let Some(renderer) = renderer_by_class(&class) {
                        by_extension.push((extension, renderer));
                    }
                }
            }
        }
        // A factory with no conf still renders `.fancy` files.
        if by_extension.is_empty() {
            for (extension, class) in [
                ("fancy", "fancy"),
                ("html.fancy", "fancy-html"),
                ("fancy-locked", "fancy-locked"),
                ("html.fancy-locked", "fancy-locked-html"),
            ] {
                if let Some(renderer) = renderer_by_class(class) {
                    by_extension.push((extension.to_string(), renderer));
                }
            }
        }
        Self {
            by_extension,
            files: Mutex::new(HashMap::new()),
        }
    }

    /// The renderer for the longest registered extension `location` ends with.
    pub fn renderer_for(&self, location: &str) -> Option<Arc<dyn TemplateRenderer>> {
        let name = location.rsplit('/').next().unwrap_or(location);
        self.by_extension
            .iter()
            .filter(|(extension, _)| name.len() > extension.len() && name.ends_with(&format!(".{extension}")))
            .max_by_key(|(extension, _)| extension.len())
            .map(|(_, renderer)| Arc::clone(renderer))
    }

    /// Render a file template. `ec.resource.template`. The compiled form is kept by location and
    /// last-modified time. A location whose extension has no renderer is an error.
    ///
    /// # Errors
    ///
    /// Returns an error when no renderer matches, the file is missing, or the template fails.
    pub fn render_location(
        &self,
        factory: &ExecutionContextFactory,
        location: &str,
        data: Option<&Value>,
    ) -> Result<String, Error> {
        let renderer = self
            .renderer_for(location)
            .ok_or_else(|| Error::new(format!("no template renderer for location {location}")))?;
        let resource = factory.resource().get(location)?;
        let modified = resource.last_modified();
        let cached = self
            .files
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(location)
            .filter(|(stamp, _)| *stamp == modified && modified.is_some())
            .map(|(_, text)| text.clone());
        let text = match cached {
            Some(text) => text,
            None => {
                let text = resource.read_text()?;
                let mut files = self.files.lock().unwrap_or_else(|err| err.into_inner());
                if files.len() >= MAX_COMPILED {
                    files.clear();
                }
                files.insert(location.to_string(), (modified, text.clone()));
                text
            }
        };
        renderer.render(&text, data)
    }
}

// ---- cache of compiled functions, by source text ----

type CompiledCache = Mutex<HashMap<(Escape, bool, String), Arc<str>>>;

fn cache() -> &'static CompiledCache {
    static CACHE: std::sync::OnceLock<CompiledCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn compiled(source: &str, escape: Escape, scope: Scope) -> Result<Arc<str>, Error> {
    let key = (escape, scope == Scope::Snapshot, source.to_string());
    if let Some(found) = cache().lock().unwrap_or_else(|err| err.into_inner()).get(&key) {
        return Ok(Arc::clone(found));
    }
    let made: Arc<str> = Arc::from(compile(source, escape, scope)?);
    let mut guard = cache().lock().unwrap_or_else(|err| err.into_inner());
    if guard.len() >= MAX_COMPILED {
        guard.clear();
    }
    guard.insert(key, Arc::clone(&made));
    Ok(made)
}

// ---- compiler ----

#[derive(Debug, PartialEq)]
enum Token {
    Text(String),
    Interp(String),
    If(String),
    ElseIf(String),
    Else,
    EndIf,
    List(String, String),
    EndList,
    Assign(String, String),
    Comment,
}

impl Token {
    fn is_tag(&self) -> bool {
        !matches!(self, Token::Text(_) | Token::Interp(_))
    }
}

const PRELUDE: &str = r#"const $esc = (v) => String(v).replace(/[&<>"']/g, (c) => c === "&" ? "&amp;" : c === "<" ? "&lt;" : c === ">" ? "&gt;" : c === '"' ? "&quot;" : "&#39;");
class $Raw { constructor(s) { this.s = s; } }
const $s = (v) => v instanceof $Raw ? v.s : v == null ? "" : String(v);
const $h = (v) => v instanceof $Raw ? v.s : v == null ? "" : $esc(v);
const html = (v) => new $Raw($h(v));
const raw = (v) => new $Raw($s(v));
const $list = (v) => v == null ? [] : Array.isArray(v) ? v : typeof v === "string" ? [v] : typeof v[Symbol.iterator] === "function" ? Array.from(v) : typeof v.length === "number" ? Array.from({length: v.length}, (_, i) => v[i]) : Object.values(v);
const $o = [];
"#;

/// The statements for a template, without the prelude and the function around them.
fn compile_body(source: &str, escape: Escape) -> Result<String, Error> {
    let tokens = tokenize(source)?;
    generate(&tokens, escape, source)
}

fn compile(source: &str, escape: Escape, scope: Scope) -> Result<String, Error> {
    let body = compile_body(source, escape)?;
    let live = format!("(function(){{\n{PRELUDE}{body}return $o.join(\"\");\n}})");
    // `with` is not legal in the strict parse, so the snapshot form is checked as the live form.
    super::check_program(&live).map_err(|err| Error::new(format!("template does not compile: {}", err.message())))?;
    let text = match scope {
        Scope::Live => live,
        Scope::Snapshot => {
            format!("(function($data){{\n{PRELUDE}with ($scope($data)) {{\n{body}}}\nreturn $o.join(\"\");\n}})")
        }
    };
    Ok(text)
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())].matches('\n').count() + 1
}

fn fail(source: &str, offset: usize, message: &str) -> Error {
    Error::new(format!("template line {}: {message}", line_of(source, offset)))
}

/// Where the expression that starts at `from` ends: the first `>` (or `}`) outside brackets and
/// strings. An arrow `=>` does not end a tag.
fn scan_expression(source: &str, from: usize, terminator: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = from;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'\'' | b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != byte {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            b'`' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'`' {
                    if bytes[index] == b'\\' {
                        index += 1;
                    } else if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
                        index = scan_expression(source, index + 2, b'}')?;
                    }
                    index += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            b'}' if terminator == b'}' && depth == 0 => return Some(index),
            b'}' => depth = depth.saturating_sub(1),
            b'>' if terminator == b'>' && depth == 0 && (index == 0 || bytes[index - 1] != b'=') => {
                return Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn tokenize(source: &str) -> Result<Vec<Token>, Error> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    macro_rules! flush {
        () => {
            if !text.is_empty() {
                tokens.push(Token::Text(std::mem::take(&mut text)));
            }
        };
    }
    while index < source.len() {
        let rest = &source[index..];
        if rest.starts_with("\\${") {
            text.push_str("${");
            index += 3;
        } else if rest.starts_with("\\<#") {
            text.push_str("<#");
            index += 3;
        } else if rest.starts_with("<#--") {
            let end = rest
                .find("-->")
                .ok_or_else(|| fail(source, index, "comment is not closed"))?;
            flush!();
            tokens.push(Token::Comment);
            index += end + 3;
        } else if rest.starts_with("${") {
            let end =
                scan_expression(source, index + 2, b'}').ok_or_else(|| fail(source, index, "${ has no closing }"))?;
            flush!();
            tokens.push(Token::Interp(source[index + 2..end].to_string()));
            index = end + 1;
        } else if rest.starts_with("<#") || rest.starts_with("</#") || rest.starts_with("<@") || rest.starts_with("</@")
        {
            flush!();
            let (token, next) = tag(source, index)?;
            tokens.push(token);
            index = next;
        } else {
            let ch = rest.chars().next().unwrap_or('\u{FFFD}');
            text.push(ch);
            index += ch.len_utf8();
        }
    }
    flush!();
    Ok(strip_tag_lines(tokens))
}

fn tag(source: &str, start: usize) -> Result<(Token, usize), Error> {
    let rest = &source[start..];
    if rest.starts_with("<@") || rest.starts_with("</@") {
        return Err(fail(
            source,
            start,
            "FreeMarker macro calls are not part of the fancy dialect",
        ));
    }
    let closing = rest.starts_with("</#");
    let name_start = start + if closing { 3 } else { 2 };
    let name_end = source[name_start..]
        .find(|ch: char| !ch.is_ascii_alphabetic())
        .map_or(source.len(), |at| name_start + at);
    let name = &source[name_start..name_end];
    let simple = |source: &str, from: usize| -> Result<usize, Error> {
        let trimmed = source[from..].trim_start();
        if trimmed.starts_with('>') {
            Ok(source.len() - trimmed.len() + 1)
        } else {
            Err(fail(source, start, &format!("<#{name}> takes no arguments")))
        }
    };
    if closing {
        let next = simple(source, name_end)?;
        return match name {
            "if" => Ok((Token::EndIf, next)),
            "list" => Ok((Token::EndList, next)),
            _ => Err(fail(source, start, &format!("unknown tag </#{name}>"))),
        };
    }
    if name == "else" {
        return Ok((Token::Else, simple(source, name_end)?));
    }
    if !matches!(name, "if" | "elseif" | "list" | "assign") {
        return Err(fail(
            source,
            start,
            &format!("unknown tag <#{name}>; the dialect has if, elseif, else, list and assign"),
        ));
    }
    let end = scan_expression(source, name_end, b'>')
        .ok_or_else(|| fail(source, start, &format!("<#{name} has no closing >")))?;
    let argument = source[name_end..end].trim().to_string();
    if argument.is_empty() {
        return Err(fail(source, start, &format!("<#{name}> needs an expression")));
    }
    let token = match name {
        "if" => Token::If(argument),
        "elseif" => Token::ElseIf(argument),
        "list" => {
            let (items, variable) = split_last_word(&argument, "as")
                .ok_or_else(|| fail(source, start, "<#list> needs `expression as name`"))?;
            if !is_identifier(variable) {
                return Err(fail(source, start, &format!("`{variable}` is not a variable name")));
            }
            Token::List(items.to_string(), variable.to_string())
        }
        _ => {
            let (variable, value) = argument
                .split_once('=')
                .filter(|(_, value)| !value.starts_with('=') && !value.trim().is_empty())
                .ok_or_else(|| fail(source, start, "<#assign> needs `name = expression`"))?;
            let variable = variable.trim();
            if !is_identifier(variable) {
                return Err(fail(source, start, &format!("`{variable}` is not a variable name")));
            }
            Token::Assign(variable.to_string(), value.trim().to_string())
        }
    };
    Ok((token, end + 1))
}

/// `a.b as item` split at the last ` as ` word.
fn split_last_word<'a>(text: &'a str, word: &str) -> Option<(&'a str, &'a str)> {
    let needle = format!(" {word} ");
    let at = text.rfind(&needle)?;
    Some((text[..at].trim(), text[at + needle.len()..].trim()))
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !name.starts_with('$')
}

/// A line that holds only tags and white space produces no output, as in FreeMarker.
fn strip_tag_lines(tokens: Vec<Token>) -> Vec<Token> {
    // Split text so a newline ends a piece.
    let mut pieces: Vec<Token> = Vec::new();
    for token in tokens {
        match token {
            Token::Text(text) => {
                for part in text.split_inclusive('\n') {
                    pieces.push(Token::Text(part.to_string()));
                }
            }
            other => pieces.push(other),
        }
    }
    let mut out = Vec::new();
    let mut line: Vec<Token> = Vec::new();
    let flush = |line: &mut Vec<Token>, out: &mut Vec<Token>| {
        let tags_only = line.iter().any(Token::is_tag)
            && line
                .iter()
                .all(|token| token.is_tag() || matches!(token, Token::Text(text) if text.trim().is_empty()));
        for token in line.drain(..) {
            if !tags_only || token.is_tag() {
                out.push(token);
            }
        }
    };
    for piece in pieces {
        let ends = matches!(&piece, Token::Text(text) if text.ends_with('\n'));
        line.push(piece);
        if ends {
            flush(&mut line, &mut out);
        }
    }
    flush(&mut line, &mut out);
    out
}

fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

fn generate(tokens: &[Token], escape: Escape, source: &str) -> Result<String, Error> {
    let encode = if escape == Escape::Html { "$h" } else { "$s" };
    let mut out = String::new();
    // `true` for an open list, `false` for an open if, with whether an else was seen.
    let mut stack: Vec<(bool, bool)> = Vec::new();
    let mut lists = 0usize;
    for token in tokens {
        match token {
            Token::Text(text) => {
                out.push_str(&format!("$o.push({});\n", js_string(text)));
            }
            Token::Interp(expr) => {
                check_expression(expr, source)?;
                out.push_str(&format!("$o.push({encode}({expr}));\n"));
            }
            Token::If(cond) => {
                check_expression(cond, source)?;
                out.push_str(&format!("if ({cond}) {{\n"));
                stack.push((false, false));
            }
            Token::ElseIf(cond) => {
                check_expression(cond, source)?;
                match stack.last() {
                    Some((false, false)) => out.push_str(&format!("}} else if ({cond}) {{\n")),
                    _ => {
                        return Err(Error::new(
                            "template: <#elseif> without an open <#if>, or after <#else>",
                        ))
                    }
                }
            }
            Token::Else => match stack.last_mut() {
                Some((false, seen @ false)) => {
                    *seen = true;
                    out.push_str("} else {\n");
                }
                _ => return Err(Error::new("template: <#else> without an open <#if>, or twice")),
            },
            Token::EndIf => match stack.pop() {
                Some((false, _)) => out.push_str("}\n"),
                _ => return Err(Error::new("template: </#if> without an open <#if>")),
            },
            Token::List(items, name) => {
                check_expression(items, source)?;
                lists += 1;
                out.push_str(&format!(
                    "{{\nconst $l{lists} = $list({items});\nfor (let {name}_index = 0; {name}_index < $l{lists}.length; {name}_index++) {{\nconst {name} = $l{lists}[{name}_index];\nconst {name}_has_next = {name}_index < $l{lists}.length - 1;\n"
                ));
                stack.push((true, false));
            }
            Token::EndList => match stack.pop() {
                Some((true, _)) => out.push_str("}\n}\n"),
                _ => return Err(Error::new("template: </#list> without an open <#list>")),
            },
            Token::Comment => {}
            Token::Assign(name, value) => {
                check_expression(value, source)?;
                out.push_str(&format!("var {name} = {value};\n"));
            }
        }
    }
    if let Some((is_list, _)) = stack.last() {
        let open = if *is_list { "<#list>" } else { "<#if>" };
        return Err(Error::new(format!("template: {open} is not closed")));
    }
    Ok(out)
}

fn check_expression(expr: &str, _source: &str) -> Result<(), Error> {
    super::parse_js(expr).map_err(|err| Error::new(format!("template: {}", err.message())))
}

// ---- unwrap snapshot ----

/// The plain projection of a value: strings, booleans and numbers pass through, a decimal is a
/// string, timestamps are text, an entity value is its field map and an entity list is a list of
/// those. Anything else (`ec`, `sri`, finds) is dropped.
pub fn unwrap_json(value: &Value) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Value::Decimal(number) => Json::String(number.to_string()),
        Value::List(items) => Json::Array(items.iter().map(unwrap_json).collect()),
        Value::Map(entries) => Json::Object(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), unwrap_json(item)))
                .collect(),
        ),
        Value::Object(_) => {
            if let Some(entries) = super::value_map(value) {
                Json::Object(
                    entries
                        .iter()
                        .map(|(key, item)| (key.clone(), unwrap_json(item)))
                        .collect(),
                )
            } else if let Some(items) = super::value_items(value) {
                Json::Array(items.iter().map(unwrap_json).collect())
            } else {
                tracing::warn!("a host object is not part of a locked-down template snapshot and was dropped");
                Json::Null
            }
        }
        other => moqui_lib::json::to_json(other),
    }
}

// ---- locked-down context ----

const SANDBOX_PRELUDE: &str = r#"
const $reserved = new Set(["html", "raw"]);
globalThis.$scope = (data) => new Proxy(JSON.parse(data), {
    has: (t, k) => typeof k === "string" && !k.startsWith("$") && !$reserved.has(k) && !(k in globalThis),
    get: (t, k) => k === Symbol.unscopables ? undefined : (t[k] === undefined ? null : t[k]),
    set: (t, k, v) => { t[k] = v; return true; },
});
"#;

struct Sandbox {
    functions: RefCell<HashMap<String, Persistent<Function<'static>>>>,
    context: Context,
    // The context borrows this runtime. Drop order frees the context first.
    #[allow(dead_code)]
    runtime: Runtime,
}

thread_local! {
    static SANDBOX: RefCell<Option<Sandbox>> = const { RefCell::new(None) };
    static LOCKED_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

impl Sandbox {
    fn start() -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(js_error)?;
        runtime.set_memory_limit(LOCKED_MEMORY_LIMIT);
        runtime.set_interrupt_handler(Some(Box::new(|| {
            LOCKED_DEADLINE.with(Cell::get).is_some_and(|at| Instant::now() >= at)
        })));
        let context = Context::full(&runtime).map_err(js_error)?;
        context.with(|ctx| -> Result<(), Error> {
            let globals = ctx.globals();
            globals
                .set("format", Function::new(ctx.clone(), locked_format).map_err(js_error)?)
                .map_err(js_error)?;
            globals
                .set(
                    "localize",
                    Function::new(ctx.clone(), locked_localize).map_err(js_error)?,
                )
                .map_err(js_error)?;
            ctx.eval::<(), _>(SANDBOX_PRELUDE)
                .catch(&ctx)
                .map_err(|err| Error::new(err.to_string()))
        })?;
        Ok(Self {
            functions: RefCell::new(HashMap::new()),
            context,
            runtime,
        })
    }
}

fn js_error(err: impl ToString) -> Error {
    Error::new(err.to_string())
}

fn locked_format(ctx: Ctx<'_>, value: JsValue<'_>, pattern: Opt<String>) -> rquickjs::Result<String> {
    let host = if let Some(text) = value.as_string() {
        let text = text.to_string()?;
        let wants_pattern = pattern.0.as_deref().is_some_and(|pattern| !pattern.is_empty());
        if wants_pattern {
            crate::l10n::parse_timestamp(&text)
                .or_else(|| text.parse::<moqui_lib::format::Decimal>().ok().map(Value::Decimal))
                .unwrap_or(Value::String(text))
        } else {
            Value::String(text)
        }
    } else if let Some(number) = value.as_int() {
        Value::I64(i64::from(number))
    } else if let Some(number) = value.as_float() {
        Value::F64(number)
    } else if let Some(flag) = value.as_bool() {
        Value::Bool(flag)
    } else {
        Value::Null
    };
    crate::l10n::format_value(&host, pattern.0.as_deref()).map_err(|err| Exception::throw_message(&ctx, err.message()))
}

fn locked_localize(ctx: Ctx<'_>, text: String) -> rquickjs::Result<String> {
    crate::l10n::localize(&text).map_err(|err| Exception::throw_message(&ctx, err.message()))
}

fn render_locked(compiled: &str, json: &str) -> Result<String, Error> {
    SANDBOX.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(Sandbox::start()?);
        }
        let guard = cell.borrow();
        let sandbox = guard
            .as_ref()
            .ok_or_else(|| Error::new("template sandbox did not start"))?;
        let previous = LOCKED_DEADLINE.with(Cell::get);
        LOCKED_DEADLINE.with(|deadline| deadline.set(Some(Instant::now() + LOCKED_TIME_LIMIT)));
        let result = sandbox.context.with(|ctx| {
            let saved = sandbox.functions.borrow().get(compiled).cloned();
            let function = match saved {
                Some(saved) => saved.restore(&ctx).map_err(js_error)?,
                None => {
                    let mut options = rquickjs::context::EvalOptions::default();
                    options.strict = false;
                    options.global = true;
                    let function: Function = ctx
                        .eval_with_options(compiled, options)
                        .catch(&ctx)
                        .map_err(|err| Error::new(err.to_string()))?;
                    let mut functions = sandbox.functions.borrow_mut();
                    if functions.len() >= MAX_COMPILED {
                        functions.clear();
                    }
                    functions.insert(compiled.to_string(), Persistent::save(&ctx, function.clone()));
                    function
                }
            };
            function
                .call::<_, String>((json,))
                .catch(&ctx)
                .map_err(|err| Error::new(err.to_string()))
        });
        LOCKED_DEADLINE.with(|deadline| deadline.set(previous));
        result
    })
}

#[cfg(test)]
mod tests;
