//! Run-time hazards that a ported script parses and runs, and that behave differently from the Groovy it came from.
//! `moqui lint --groovy` adds them to the sites of [`super::screen_lint`] as [`Idiom::DecimalOperator`],
//! [`Idiom::FindListMutation`] and [`Idiom::InOnList`], and drops the empty-collection truthiness warnings that a
//! parameter's declared type rules out.
//!
//! The scan is by name and by service: the names a service declares as `BigDecimal`, sets from a decimal call, or
//! fills from an `entity-find list`. It has no types, so it only knows what the service text says.

use std::collections::{BTreeMap, BTreeSet};

use moqui_lib::node::MNode;

use super::screen_lint::{FileReport, Idiom, SiteKind, is_expression_attr, mask_strings, snippet};

/// What one service (or one top-level element of another file) tells about its variables.
#[derive(Default)]
struct Facts {
    decimals: BTreeSet<String>,
    find_lists: BTreeSet<String>,
    arrays: BTreeSet<String>,
    /// Parameters declared `List`, `Set` or `Collection`: an empty one is false in Groovy and true here.
    collections: BTreeSet<String>,
    /// Names that something sets to a value of another kind. They are not tracked.
    other_decimal: BTreeSet<String>,
    other_array: BTreeSet<String>,
    assigned: BTreeSet<String>,
}

/// One expression or script of a scope, with where it is.
struct Text {
    line: u32,
    element: String,
    attribute: String,
    text: String,
    /// A `<script>` directly under the service's `<actions>`: its `const` and `let` are in the scope of the whole
    /// generated function, so another one with the same name is a redeclaration. In an `if` or `iterate` they are in a block.
    function_level: bool,
}

/// Adds the hazards to the sites of a scanned file and removes the truthiness warnings that a declared type
/// rules out. `root` is the file's root element.
pub(crate) fn refine(root: &MNode, file: &mut FileReport) {
    let mut types: BTreeMap<String, Vec<String>> = BTreeMap::new();
    declared_types(root, &mut types);
    for site in &mut file.sites {
        site.idioms.retain(|(idiom, operand)| {
            *idiom != Idiom::EmptyCollectionTruthiness
                || types.get(last_segment(operand)).is_none_or(|declared| {
                    declared.iter().any(|kind| {
                        ["List", "Set", "Map", "Collection", "Iterable", "Object", "[]"]
                            .iter()
                            .any(|name| kind.contains(name))
                    })
                })
        });
    }
    for scope in root.children() {
        let mut facts = Facts::default();
        let mut texts = Vec::new();
        gather(&scope, &mut facts, &mut texts);
        let mut declared: BTreeSet<String> = BTreeSet::new();
        for text in &texts {
            let masked = mask_strings(&text.text);
            let mut found_hazards = hazards(&masked, &facts);
            if text.function_level {
                for (name, at) in top_level_declarations(&masked) {
                    if !declared.insert(name) {
                        found_hazards.push((Idiom::DuplicateDeclaration, at));
                    }
                }
            }
            for (idiom, at) in found_hazards {
                let sample = snippet(&text.text, at);
                let found = file.sites.iter_mut().find(|site| {
                    site.line == text.line
                        && site.element == text.element
                        && site.attribute == text.attribute
                        && site.kind != SiteKind::Template
                });
                if let Some(site) = found {
                    if !site.idioms.iter().any(|(have, was)| *have == idiom && *was == sample) {
                        site.idioms.push((idiom, sample));
                    }
                }
            }
        }
    }
}

fn last_segment(operand: &str) -> &str {
    operand.rsplit('.').next().unwrap_or(operand)
}

fn declared_types(node: &MNode, types: &mut BTreeMap<String, Vec<String>>) {
    if node.name() == "parameter" {
        if let (Some(name), Some(kind)) = (node.attribute("name"), node.attribute("type")) {
            types.entry(name).or_default().push(kind);
        }
    }
    for child in node.children() {
        declared_types(&child, types);
    }
}

/// Collects the facts and the expression texts under `node`.
fn gather(node: &MNode, facts: &mut Facts, texts: &mut Vec<Text>) {
    gather_at(node, facts, texts, None);
}

/// `actions_depth` is how many elements below an `actions` or `always-actions` element `node` is: 1 is a direct child.
fn gather_at(node: &MNode, facts: &mut Facts, texts: &mut Vec<Text>, actions_depth: Option<usize>) {
    let element = node.name();
    let actions_depth = match element.as_str() {
        "actions" | "always-actions" => Some(0),
        _ => actions_depth.map(|depth| depth + 1),
    };
    let line = node.line();
    match element.as_str() {
        "parameter" => {
            if let (Some(name), Some(kind)) = (node.attribute("name"), node.attribute("type")) {
                if kind.contains("BigDecimal") {
                    facts.decimals.insert(name);
                } else if ["List", "Set", "Collection"].iter().any(|word| kind.contains(word)) {
                    facts.collections.insert(name);
                }
            }
        }
        "entity-find" => {
            if let Some(list) = node.attribute("list") {
                // An empty find result is an empty list: false in Groovy and true here.
                facts.collections.insert(list.clone());
                facts.find_lists.insert(list);
            }
        }
        "set" => {
            if let Some(field) = node.attribute("field") {
                facts.assigned.insert(field.clone());
                if node.attribute("type").is_some_and(|kind| kind.contains("BigDecimal")) {
                    facts.decimals.insert(field.clone());
                }
                match (node.attribute("from"), node.attribute("value")) {
                    (Some(from), _) => record_assignment(facts, &field, &from),
                    (None, Some(value)) if value.trim().is_empty() => {}
                    (None, Some(_)) => {
                        facts.other_decimal.insert(field.clone());
                        facts.other_array.insert(field);
                    }
                    (None, None) => {}
                }
            }
        }
        _ => {}
    }
    for (attr, value) in node.attributes() {
        if !value.trim().is_empty() && !attr.contains(':') && is_expression_attr(&element, &attr) {
            texts.push(Text {
                line,
                element: element.clone(),
                attribute: attr,
                text: value,
                function_level: false,
            });
        }
    }
    if matches!(element.as_str(), "script" | "expression") {
        if let Some(body) = node.text().filter(|body| !body.trim().is_empty()) {
            script_assignments(&mask_strings(&body), &body, facts);
            texts.push(Text {
                line,
                element: element.clone(),
                attribute: "text".to_string(),
                text: body,
                function_level: element == "script" && actions_depth == Some(1),
            });
        }
    }
    for child in node.children() {
        gather_at(&child, facts, texts, actions_depth);
    }
}

fn record_assignment(facts: &mut Facts, name: &str, rhs: &str) {
    let rhs = rhs.trim();
    if rhs.is_empty() || rhs == "null" || rhs == "undefined" {
        return;
    }
    if produces_decimal(rhs) {
        facts.decimals.insert(name.to_string());
    } else if produces_other_value(rhs) {
        facts.other_decimal.insert(name.to_string());
    }
    if produces_array(rhs) {
        facts.arrays.insert(name.to_string());
    } else {
        facts.other_array.insert(name.to_string());
    }
    facts.assigned.insert(name.to_string());
}

/// `name = rhs` statements of a script, found on the masked text and read from the original.
fn script_assignments(masked: &str, original: &str, facts: &mut Facts) {
    for (start, end) in identifiers(masked) {
        if start > 0 && masked.as_bytes()[start - 1] == b'.' {
            continue;
        }
        let rest = masked[end..].trim_start();
        let after = rest.strip_prefix('=');
        let plain = after.is_some_and(|tail| !tail.starts_with('=') && !tail.starts_with('>'));
        if !plain {
            continue;
        }
        let rhs_start = masked.len() - after.map_or(0, str::len);
        let rhs_end = statement_end(masked, rhs_start);
        record_assignment(
            facts,
            &masked[start..end],
            original.get(rhs_start..rhs_end).unwrap_or("").trim(),
        );
    }
}

/// The end of the statement that starts at `from`: a `;` or a newline outside brackets.
fn statement_end(masked: &str, from: usize) -> usize {
    let mut depth = 0i32;
    for (offset, byte) in masked.as_bytes()[from..].iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth <= 0 => return from + offset,
            b'\n' if depth <= 0 => {
                let rest = masked[from + offset..].trim_start();
                // A line that goes on with `.method(`, an operator or `?` belongs to the same statement.
                if !(rest.starts_with('.') || rest.starts_with("?.") || rest.starts_with('?') || rest.starts_with(':'))
                {
                    return from + offset;
                }
            }
            _ => {}
        }
        if depth < 0 {
            return from + offset;
        }
    }
    masked.len()
}

const DECIMAL_METHODS: &[&str] = &["add", "subtract", "multiply", "divide", "setScale", "abs", "negate"];

/// A value that is a host decimal: `toDecimal(..)` or a chain of decimal methods, a `reduce` that starts from
/// `toDecimal(..)`, or a ternary whose branches are decimals or null.
fn produces_decimal(rhs: &str) -> bool {
    let rhs = rhs.trim();
    if let Some((then_branch, else_branch)) = split_ternary(rhs) {
        let branch = |part: &str| {
            let part = part.trim();
            part == "null" || part == "undefined" || produces_decimal(part)
        };
        return branch(then_branch)
            && branch(else_branch)
            && (produces_decimal(then_branch) || produces_decimal(else_branch));
    }
    match last_method(rhs) {
        Some(method) if DECIMAL_METHODS.contains(&method) => true,
        Some("reduce") => rhs.contains(", toDecimal(") || rhs.contains(",toDecimal("),
        Some(_) => false,
        None => rhs.starts_with("toDecimal(") && matching_close(rhs, "toDecimal".len()) == Some(rhs.len() - 1),
    }
}

/// A value that is known not to be a host decimal: a number, text, a boolean, a list or a map. An entity field or a
/// call result may be a decimal and is not listed here.
fn produces_other_value(rhs: &str) -> bool {
    let rhs = rhs.trim();
    if let Some((then_branch, else_branch)) = split_ternary(rhs) {
        return produces_other_value(then_branch) || produces_other_value(else_branch);
    }
    const PREFIXES: &[&str] = &[
        "Number(",
        "parseInt(",
        "parseFloat(",
        "Math.",
        "String(",
        "Boolean(",
        "[",
        "{",
        "true",
        "false",
        "new ",
        "'",
        "\"",
        "`",
        "!",
    ];
    PREFIXES.iter().any(|prefix| rhs.starts_with(prefix))
        || rhs.starts_with(|c: char| c.is_ascii_digit())
        || rhs.ends_with(".length")
        || rhs.ends_with(".toNumber()")
        || rhs.ends_with(".isZero()")
}

/// The index of the `)` that closes the `(` at `open`.
fn matching_close(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (offset, byte) in text.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// The two branches of a top-level `c ? a : b`.
fn split_ternary(expr: &str) -> Option<(&str, &str)> {
    let masked = mask_strings(expr);
    let bytes = masked.as_bytes();
    let mut depth = 0i32;
    let mut question = None;
    for index in 0..bytes.len() {
        match bytes[index] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'?' if depth == 0 && question.is_none() => {
                let next = bytes.get(index + 1).copied();
                if next != Some(b'.') && next != Some(b'?') && (index == 0 || bytes[index - 1] != b'?') {
                    question = Some(index);
                }
            }
            b':' if depth == 0 => {
                if let Some(q) = question {
                    return Some((&expr[q + 1..index], &expr[index + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn produces_array(rhs: &str) -> bool {
    let rhs = rhs.trim();
    rhs.starts_with('[')
        || rhs.starts_with("Array.from(")
        || matches!(
            last_method(rhs),
            Some("map" | "filter" | "split" | "slice" | "concat" | "flat" | "flatMap")
        )
}

/// The name of the last call of a chain that ends in `)`, such as `add` for `a.b(c).add(d)`.
fn last_method(expr: &str) -> Option<&str> {
    let expr = expr.trim();
    if !expr.ends_with(')') {
        return None;
    }
    let bytes = expr.as_bytes();
    let mut depth = 0i32;
    let mut open = None;
    for index in (0..bytes.len()).rev() {
        match bytes[index] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let open = open?;
    let head = &expr[..open];
    let name_start = head
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
        .map_or(0, |at| at + 1);
    let name = &head[name_start..];
    if name.is_empty() || name_start == 0 || head.as_bytes()[name_start - 1] != b'.' {
        return None;
    }
    Some(name)
}

/// Start and end of each identifier in `text`, skipping numbers and the pieces of numbers.
fn identifiers(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == b'$')
            {
                index += 1;
            }
            out.push((start, index));
        } else if byte.is_ascii_digit() {
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.' || bytes[index] == b'_')
            {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    out
}

const LIST_MUTATIONS: &[&str] = &[
    "push",
    "add",
    "addAll",
    "pop",
    "splice",
    "shift",
    "unshift",
    "first",
    "last",
    "each",
    "collect",
    "findAll",
    "remove",
    "removeAll",
    "clear",
];

/// The names a masked script declares with `const` or `let` outside any brace, bracket or parenthesis, with the offset.
fn top_level_declarations(masked: &str) -> Vec<(String, usize)> {
    let bytes = masked.as_bytes();
    let mut depth = 0i32;
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && (index == 0 || !is_word(bytes[index - 1])) {
            for keyword in ["const", "let"] {
                let rest = &masked[index..];
                if rest.starts_with(keyword)
                    && rest
                        .as_bytes()
                        .get(keyword.len())
                        .is_some_and(|b| b.is_ascii_whitespace())
                {
                    let name: String = rest[keyword.len()..]
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
                        .collect();
                    if !name.is_empty() {
                        out.push((name, index));
                    }
                }
            }
        }
        index += 1;
    }
    out
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// The hazards in one masked text, each with the offset.
fn hazards(masked: &str, facts: &Facts) -> Vec<(Idiom, usize)> {
    let decimals: BTreeSet<&String> = facts.decimals.difference(&facts.other_decimal).collect();
    let find_lists: BTreeSet<&String> = facts.find_lists.difference(&facts.assigned).collect();
    let arrays: BTreeSet<&String> = facts
        .arrays
        .difference(&facts.other_array)
        .chain(facts.find_lists.difference(&facts.other_array))
        .collect();
    let mut out = Vec::new();
    for (start, end) in identifiers(masked) {
        let name = &masked[start..end];
        let before = masked[..start].trim_end();
        if before.ends_with('.') {
            continue;
        }
        let after = masked[end..].trim_start();
        if decimals.iter().any(|decimal| decimal.as_str() == name) && decimal_hazard(before, after) {
            out.push((Idiom::DecimalOperator, start));
        }
        if facts.collections.contains(name) && !facts.other_array.contains(name) && collection_truthiness(before, after)
        {
            out.push((Idiom::EmptyCollectionTruthiness, start));
        }
        if find_lists.iter().any(|list| list.as_str() == name) {
            if let Some(method) = after.strip_prefix('.') {
                let method: String = method
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if LIST_MUTATIONS.contains(&method.as_str()) && after.contains('(') {
                    out.push((Idiom::FindListMutation, start));
                }
            }
        }
        if before.ends_with("in")
            && before[..before.len() - 2].ends_with(|c: char| c.is_whitespace() || c == ')' || c == '\'' || c == '"')
            && !in_for_head(&masked[..start])
            && arrays.iter().any(|array| array.as_str() == name)
            && !after.starts_with('.')
            && !after.starts_with('(')
        {
            out.push((Idiom::InOnList, start));
        }
    }
    out
}

/// A collection name tested for truthiness: `!x`, `if (x)`, `x && ..`, `x || ..`, `x ? a : b`.
fn collection_truthiness(before: &str, after: &str) -> bool {
    if after.starts_with(['.', '[', ':', '(']) || after.starts_with("?.") {
        return false;
    }
    if before.ends_with('!') {
        return true;
    }
    let condition_head = before
        .strip_suffix('(')
        .map(str::trim_end)
        .is_some_and(|head| head.ends_with("if") || head.ends_with("while"));
    (condition_head && after.starts_with(')'))
        || after.starts_with("&&")
        || after.starts_with("||")
        || (after.starts_with('?') && !after.starts_with("??") && !after.starts_with("?."))
}

/// Whether the `in` before `at` belongs to a `for (x in ..)` head.
fn in_for_head(prefix: &str) -> bool {
    let Some(at) = prefix.rfind("for") else {
        return false;
    };
    let head = &prefix[at..];
    head.trim_start_matches("for").trim_start().starts_with('(') && !head.contains(')')
}

/// A decimal name between `before` and `after` that the host decimal does not support as JavaScript does.
fn decimal_hazard(before: &str, after: &str) -> bool {
    if after.starts_with('.') || after.starts_with('(') || after.starts_with('[') || after.starts_with("?.") {
        return false;
    }
    // An object key or the else branch of a ternary.
    if after.starts_with(':') {
        return false;
    }
    if before.ends_with('!') {
        return true;
    }
    if after.starts_with("||") || after.starts_with("&&") {
        return true;
    }
    if after.starts_with('?') && !after.starts_with("??") {
        return true;
    }
    for op in ["===", "!==", "==", "!="] {
        if let Some(rhs) = after.strip_prefix(op) {
            let rhs = rhs.trim_start();
            return !(rhs.starts_with("null") || rhs.starts_with("undefined"));
        }
    }
    if after.starts_with('<') || after.starts_with('>') {
        return true;
    }
    for op in ['+', '-', '*', '/', '%'] {
        if let Some(rest) = after.strip_prefix(op) {
            if rest.starts_with(op) && (op == '+' || op == '-') {
                return false;
            }
            return true;
        }
    }
    for op in ["===", "!==", "==", "!="] {
        if let Some(lhs) = before.strip_suffix(op) {
            let lhs = lhs.trim_end();
            return !(lhs.ends_with("null") || lhs.ends_with("undefined"));
        }
    }
    if let Some(prev) = before.strip_suffix(['<', '>']) {
        // `=>` and `->` are arrows, not comparisons.
        return !prev.ends_with(['=', '-']);
    }
    for op in ['+', '-', '*', '/', '%'] {
        if let Some(prev) = before.strip_suffix(op) {
            return !((op == '+' || op == '-') && prev.ends_with(op));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(xml: &str) -> Vec<(Idiom, String)> {
        let root = MNode::parse_text("test.xml", xml).unwrap();
        let mut file = FileReport::default();
        super::super::screen_lint::scan_for_tests(&root, &mut file);
        refine(&root, &mut file);
        file.sites
            .into_iter()
            .flat_map(|site| site.idioms)
            .filter(|(idiom, _)| {
                matches!(
                    idiom,
                    Idiom::DecimalOperator
                        | Idiom::FindListMutation
                        | Idiom::InOnList
                        | Idiom::EmptyCollectionTruthiness
                        | Idiom::DuplicateDeclaration
                )
            })
            .collect()
    }

    fn kinds(xml: &str) -> Vec<Idiom> {
        scan(xml).into_iter().map(|(idiom, _)| idiom).collect()
    }

    #[test]
    fn decimal_parameters_and_sets_are_tracked() {
        let xml = r#"<services><service verb="a" noun="B">
            <in-parameters><parameter name="hours" type="BigDecimal"/></in-parameters>
            <actions>
                <set field="total" from="toDecimal(0)"/>
                <if condition="!hours"><set field="x" from="1"/></if>
                <if condition="hours != 0"><set field="x" from="1"/></if>
                <if condition="hours != null &amp;&amp; total > 1"><set field="x" from="1"/></if>
                <set field="y" from="hours || 1"/>
                <set field="z" from="hours + 1"/>
            </actions></service></services>"#;
        let found = kinds(xml);
        assert_eq!(
            found.iter().filter(|k| **k == Idiom::DecimalOperator).count(),
            5,
            "{:?}",
            scan(xml)
        );
    }

    #[test]
    fn decimal_methods_nulls_and_keys_are_fine() {
        let xml = r#"<services><service verb="a" noun="B">
            <in-parameters><parameter name="hours" type="BigDecimal"/></in-parameters>
            <actions>
                <if condition="hours == null || toDecimal(hours).isZero()"><set field="x" from="1"/></if>
                <set field="y" from="hours.add(2)"/>
                <set field="z" from="{hours: hours, q: n}"/>
                <set field="c" from="hours != null ? hours : 0"/>
                <set field="d" from="x => x.hours > 1"/>
            </actions></service></services>"#;
        assert_eq!(scan(xml), Vec::new());
    }

    #[test]
    fn a_name_set_to_a_number_elsewhere_is_not_a_decimal() {
        let xml = r#"<services><service verb="a" noun="B">
            <in-parameters><parameter name="n" type="BigDecimal"/></in-parameters>
            <actions>
                <set field="n" from="parseInt(m, 10)"/>
                <if condition="n > 3"><set field="x" from="1"/></if>
            </actions></service></services>"#;
        assert!(kinds(xml).is_empty());
    }

    #[test]
    fn find_list_mutations_are_found_until_the_list_is_copied() {
        let xml = r#"<services><service verb="a" noun="B"><actions>
            <entity-find entity-name="Foo" list="fooList"/>
            <script>fooList.push({a: 1}); const first = fooList.first()</script>
            <entity-find entity-name="Foo" list="copyList"/>
            <set field="copyList" from="Array.from(copyList)"/>
            <script>copyList.push({a: 1})</script>
        </actions></service></services>"#;
        let found = kinds(xml);
        // `push` and `first()` on fooList; copyList was copied.
        assert_eq!(found, vec![Idiom::FindListMutation; 2], "{:?}", scan(xml));
    }

    #[test]
    fn in_on_an_array_is_found_and_in_on_an_object_is_not() {
        let xml = r#"<services><service verb="a" noun="B"><actions>
            <set field="ids" from="[]"/>
            <set field="seen" from="{}"/>
            <if condition="'a' in ids"><set field="x" from="1"/></if>
            <if condition="'a' in seen"><set field="x" from="1"/></if>
            <script>for (const k in ids) { log(k) }</script>
        </actions></service></services>"#;
        assert_eq!(kinds(xml), vec![Idiom::InOnList], "{:?}", scan(xml));
    }

    #[test]
    fn a_collection_parameter_tested_in_a_script_is_a_hazard() {
        let xml = r#"<services><service verb="a" noun="B">
            <in-parameters><parameter name="ids" type="List"/><parameter name="flag" type="Boolean"/></in-parameters>
            <actions><script>
                if (ids) filter.push(ids)
                if (ids?.length) filter.push(ids)
                if (flag) filter.push(flag)
                const n = ids ? 1 : 2
                for (const id of ids) {}
            </script></actions></service></services>"#;
        assert_eq!(kinds(xml), vec![Idiom::EmptyCollectionTruthiness; 2], "{:?}", scan(xml));
    }

    #[test]
    fn an_entity_find_list_tested_for_truthiness_is_a_hazard() {
        let xml = r#"<services><service verb="a" noun="B"><actions>
            <entity-find entity-name="Foo" list="fooList"/>
            <script>
                const first = fooList ? fooList[0].id : null
                const safe = fooList?.length ? fooList[0].id : null
            </script>
            <set field="x" from="fooList &amp;&amp; fooList[0]"/>
        </actions></service></services>"#;
        assert_eq!(kinds(xml), vec![Idiom::EmptyCollectionTruthiness; 2], "{:?}", scan(xml));
    }

    #[test]
    fn a_const_declared_twice_at_function_level_is_a_hazard() {
        let xml = r#"<services><service verb="a" noun="B"><actions>
            <script>const dec = (v) => v; let total = 0</script>
            <if condition="x"><script>const dec = (v) => v + 1</script></if>
            <script>for (const dec of rows) {}; const other = 1</script>
            <script>const dec = 2</script>
        </actions></service></services>"#;
        assert_eq!(kinds(xml), vec![Idiom::DuplicateDeclaration], "{:?}", scan(xml));
    }

    #[test]
    fn a_declared_type_rules_out_a_truthiness_warning() {
        let xml = r#"<services><service verb="a" noun="B">
            <in-parameters><parameter name="deleteEntries" type="Boolean"/><parameter name="rowList" type="List"/></in-parameters>
            <actions>
                <if condition="deleteEntries"><set field="x" from="1"/></if>
                <if condition="rowList"><set field="x" from="1"/></if>
            </actions></service></services>"#;
        let found = scan(xml);
        assert_eq!(found, vec![(Idiom::EmptyCollectionTruthiness, "rowList".to_string())]);
    }

    #[test]
    fn last_method_reads_the_end_of_a_chain() {
        assert_eq!(last_method("a.b(c).add(d)"), Some("add"));
        assert_eq!(last_method("toDecimal(x)"), None);
        assert_eq!(last_method("a.map((x) => f(x))"), Some("map"));
        assert!(produces_decimal("toDecimal(x).multiply(y)"));
        assert!(produces_decimal("a ? toDecimal(x) : toDecimal(x).multiply(-1)"));
        assert!(!produces_decimal("toDecimal(x).compare(y) > 0"));
        assert!(!produces_decimal("toDecimal(x).isZero()"));
        assert!(produces_decimal(
            "Array.from(l).reduce((t, i) => t.add(i.q ?? 0), toDecimal(0))"
        ));
        assert!(!produces_decimal("list.map(x => x.id)"));
    }
}
