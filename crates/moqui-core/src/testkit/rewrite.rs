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

//! The power-assert rewrite of a spec file, the part of Spock's AST transform the harness needs.
//!
//! In each top-level `feature(name, { ... })` call, every expression statement directly in the body of the `then` or
//! `expect` member is a condition, as a statement in a Spock `then:` block is. The rewrite wraps each condition so a
//! false one fails the feature with a diagram of the values of its parts, and gives `==`, `!=`, `<` and the rest, `!`,
//! `&&`, `||` and the condition itself Groovy semantics: decimals compare by value and an empty list is false.
//!
//! The rewrite inserts text at node boundaries and adds no line, so a line in an error is the spec file's line. The
//! `then` and `expect` keys it rewrote are renamed `then$pa` and `expect$pa`; a `then` the rewrite could not see, such
//! as one in a feature object a helper function builds, keeps its name and the harness refuses it, so no block of
//! conditions runs unchecked.

use oxc_ast::ast::{
    Argument, BinaryOperator, ChainElement, Expression, LogicalOperator, ObjectExpression, ObjectPropertyKind,
    SourceType, Statement, UnaryOperator,
};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::GetSpan;

use crate::context::Error;

/// The name a rewritten `then` or `expect` member gets.
pub const REWRITTEN_SUFFIX: &str = "$pa";

/// One condition: its source text, where it is, and the parts whose values a failure shows.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Condition {
    pub text: String,
    /// 1-based line of the condition in the spec file.
    pub line: u32,
    /// Whether the condition is a call: one that returns `undefined` passes, as a void method call in Spock.
    pub call: bool,
    /// For each value slot, the byte offset in `text` where its value is shown, and the part's own text.
    pub slots: Vec<(u32, String)>,
}

/// A rewritten spec: the source to run, and the conditions it refers to by index.
#[derive(Clone, Debug)]
pub struct Rewritten {
    pub source: String,
    pub conditions: Vec<Condition>,
}

/// Rewrite a spec file. A file that does not parse is an error naming the first problem, with its line.
pub fn rewrite_spec(source: &str) -> Result<Rewritten, Error> {
    let allocator = std::default::Default::default();
    let parsed = Parser::new(&allocator, source, SourceType::script())
        .with_options(ParseOptions::default())
        .parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        let detail = parsed
            .diagnostics
            .errors()
            .next()
            .map_or_else(|| "syntax error".to_string(), |diagnostic| diagnostic.to_string());
        let line = parsed
            .diagnostics
            .errors()
            .next()
            .and_then(|diagnostic| diagnostic.labels.as_ref().first().map(|label| label.offset()))
            .map(|offset| line_of(source, offset));
        return Err(Error::new(match line {
            Some(line) => format!("line {line}: {detail}"),
            None => detail,
        }));
    }
    let mut rewriter = Rewriter {
        source,
        edits: Vec::new(),
        conditions: Vec::new(),
        seq: 0,
    };
    for statement in &parsed.program.body {
        if let Statement::ExpressionStatement(statement) = statement {
            if let Expression::CallExpression(call) = &statement.expression {
                let is_feature = matches!(&call.callee, Expression::Identifier(name) if name.name == "feature");
                if !is_feature {
                    continue;
                }
                for argument in &call.arguments {
                    if let Some(Expression::ObjectExpression(object)) = argument.as_expression() {
                        rewriter.feature_object(object);
                    }
                }
            }
        }
    }
    Ok(Rewritten {
        source: rewriter.apply(),
        conditions: rewriter.conditions,
    })
}

/// The `dependsOn` names of a spec file, from the string literals of `dependsOn` in its top-level `spec({ ... })`
/// call, read without running the file. A file that does not parse has none.
pub fn spec_depends_on(source: &str) -> Vec<String> {
    let allocator = std::default::Default::default();
    let parsed = Parser::new(&allocator, source, SourceType::script()).parse();
    let mut names = Vec::new();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        return names;
    }
    for statement in &parsed.program.body {
        let Statement::ExpressionStatement(statement) = statement else {
            continue;
        };
        let Expression::CallExpression(call) = &statement.expression else {
            continue;
        };
        if !matches!(&call.callee, Expression::Identifier(name) if name.name == "spec") {
            continue;
        }
        let Some(Expression::ObjectExpression(object)) = call.arguments.first().and_then(Argument::as_expression)
        else {
            continue;
        };
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            if property.key.static_name().as_deref() != Some("dependsOn") {
                continue;
            }
            if let Expression::ArrayExpression(array) = &property.value {
                for element in &array.elements {
                    if let Some(Expression::StringLiteral(text)) = element.as_expression() {
                        names.push(text.value.to_string());
                    }
                }
            }
        }
    }
    names
}

/// The spec names of a component's `test/suite.js`, `suite(["SpecA", "SpecB"])`, in order: the string literals of the
/// array in its top-level `suite(...)` call. A file that does not parse is an error.
pub fn suite_order(source: &str) -> Result<Vec<String>, Error> {
    let allocator = std::default::Default::default();
    let parsed = Parser::new(&allocator, source, SourceType::script()).parse();
    if parsed.panicked || parsed.diagnostics.has_errors() {
        return Err(Error::new("suite.js is not JavaScript"));
    }
    for statement in &parsed.program.body {
        let Statement::ExpressionStatement(statement) = statement else {
            continue;
        };
        let Expression::CallExpression(call) = &statement.expression else {
            continue;
        };
        if !matches!(&call.callee, Expression::Identifier(name) if name.name == "suite") {
            continue;
        }
        let Some(Expression::ArrayExpression(array)) = call.arguments.first().and_then(Argument::as_expression) else {
            return Err(Error::new("suite() takes an array of spec names"));
        };
        let mut names = Vec::new();
        for element in &array.elements {
            match element.as_expression() {
                Some(Expression::StringLiteral(text)) => names.push(text.value.to_string()),
                _ => return Err(Error::new("suite() takes an array of spec names as strings")),
            }
        }
        return Ok(names);
    }
    Err(Error::new("suite.js has no suite([...]) call"))
}

/// 1-based line of a byte offset.
fn line_of(source: &str, offset: u32) -> u32 {
    let end = (offset as usize).min(source.len());
    let newlines = source.as_bytes()[..end].iter().filter(|byte| **byte == b'\n').count();
    u32::try_from(newlines).unwrap_or(u32::MAX).saturating_add(1)
}

/// Text inserted at one offset. Closing text goes first, innermost first; then opening and replacement text, outermost
/// first, and in the order made when two have the same span.
struct Edit {
    at: u32,
    close: bool,
    span_len: u32,
    seq: u32,
    text: String,
    /// Source bytes after `at` that this edit replaces.
    delete: u32,
}

struct Rewriter<'s> {
    source: &'s str,
    edits: Vec<Edit>,
    conditions: Vec<Condition>,
    seq: u32,
}

impl Rewriter<'_> {
    fn push(&mut self, at: u32, close: bool, span_len: u32, text: impl Into<String>, delete: u32) {
        self.seq += 1;
        self.edits.push(Edit {
            at,
            close,
            span_len,
            seq: self.seq,
            text: text.into(),
            delete,
        });
    }

    fn feature_object(&mut self, object: &ObjectExpression<'_>) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            if property.computed || property.shorthand {
                continue;
            }
            let Some(name) = property.key.static_name() else {
                continue;
            };
            if name != "then" && name != "expect" {
                continue;
            }
            let body = match &property.value {
                Expression::FunctionExpression(function) => function.body.as_deref(),
                Expression::ArrowFunctionExpression(arrow) => Some(&*arrow.body),
                _ => None,
            };
            let Some(body) = body else {
                continue;
            };
            let key = property.key.span();
            // The key as written may be quoted; it becomes a plain name.
            self.push(
                key.start,
                false,
                u32::MAX,
                format!("{name}{REWRITTEN_SUFFIX}"),
                key.size(),
            );
            for statement in &body.statements {
                if let Statement::ExpressionStatement(statement) = statement {
                    self.condition(&statement.expression);
                }
            }
        }
    }

    fn condition(&mut self, expression: &Expression<'_>) {
        // An assignment in a `then` block is not a condition, as in Spock.
        if matches!(expression, Expression::AssignmentExpression(_)) {
            return;
        }
        let span = expression.span();
        let index = self.conditions.len();
        let call = matches!(expression, Expression::CallExpression(_))
            || matches!(expression, Expression::ChainExpression(chain) if matches!(chain.expression, ChainElement::CallExpression(_)));
        self.conditions.push(Condition {
            text: self.text(span.start, span.end).to_string(),
            line: line_of(self.source, span.start),
            call,
            slots: Vec::new(),
        });
        self.push(span.start, false, u32::MAX, format!("__pa({index}, ($pa_v) => ("), 0);
        self.instrument(expression, index, span.start);
        self.push(span.end, true, u32::MAX, "))", 0);
    }

    fn text(&self, start: u32, end: u32) -> &str {
        self.source.get(start as usize..end as usize).unwrap_or("")
    }

    /// Record the value of `expression` in the next slot of condition `index`, shown at `show_at`.
    fn slot(&mut self, expression: &Expression<'_>, index: usize, base: u32, show_at: u32) {
        let span = expression.span();
        let part = self.text(span.start, span.end).to_string();
        let condition = &mut self.conditions[index];
        let slot = condition.slots.len();
        condition.slots.push((show_at.saturating_sub(base), part));
        self.push(span.start, false, span.size(), format!("$pa_v({slot}, "), 0);
        self.push(span.end, true, span.size(), ")", 0);
    }

    /// The offset of `operator` between two operands, or `from` when it is not found.
    fn operator_at(&self, from: u32, to: u32, operator: &str) -> u32 {
        self.text(from, to)
            .find(operator)
            .map_or(from, |found| from + u32::try_from(found).unwrap_or(0))
    }

    fn instrument(&mut self, expression: &Expression<'_>, index: usize, base: u32) {
        match expression {
            Expression::Identifier(identifier) => {
                if identifier.name != "undefined" {
                    self.slot(expression, index, base, identifier.span.start);
                }
            }
            Expression::StaticMemberExpression(member) => {
                self.slot(expression, index, base, member.property.span.start);
                self.instrument(&member.object, index, base);
            }
            Expression::ComputedMemberExpression(member) => {
                let bracket = self.operator_at(member.object.span().end, member.expression.span().start, "[");
                self.slot(expression, index, base, bracket);
                self.instrument(&member.object, index, base);
                self.instrument(&member.expression, index, base);
            }
            Expression::CallExpression(call) => {
                let show_at = match &call.callee {
                    Expression::StaticMemberExpression(member) => member.property.span.start,
                    other => other.span().start,
                };
                self.slot(expression, index, base, show_at);
                // The callee itself is not wrapped, so a method keeps its `this`; its object is.
                match &call.callee {
                    Expression::StaticMemberExpression(member) => self.instrument(&member.object, index, base),
                    Expression::ComputedMemberExpression(member) => {
                        self.instrument(&member.object, index, base);
                        self.instrument(&member.expression, index, base);
                    }
                    _ => {}
                }
                for argument in &call.arguments {
                    if let Argument::SpreadElement(_) = argument {
                        continue;
                    }
                    if let Some(argument) = argument.as_expression() {
                        self.instrument(argument, index, base);
                    }
                }
            }
            Expression::BinaryExpression(binary) => {
                let operator = binary.operator.as_str();
                let left = binary.left.span();
                let right = binary.right.span();
                let at = self.operator_at(left.end, right.start, operator);
                self.slot(expression, index, base, at);
                let groovy = matches!(
                    binary.operator,
                    BinaryOperator::Equality
                        | BinaryOperator::Inequality
                        | BinaryOperator::LessThan
                        | BinaryOperator::LessEqualThan
                        | BinaryOperator::GreaterThan
                        | BinaryOperator::GreaterEqualThan
                );
                if groovy {
                    let span = binary.span;
                    self.push(span.start, false, span.size(), format!("__cmp(\"{operator}\", "), 0);
                    self.push(left.end, false, 0, ", ", right.start - left.end);
                    self.push(span.end, true, span.size(), ")", 0);
                }
                self.instrument(&binary.left, index, base);
                self.instrument(&binary.right, index, base);
            }
            Expression::LogicalExpression(logical) => {
                let operator = logical.operator.as_str();
                let left = logical.left.span();
                let right = logical.right.span();
                let at = self.operator_at(left.end, right.start, operator);
                self.slot(expression, index, base, at);
                if logical.operator != LogicalOperator::Coalesce {
                    let span = logical.span;
                    self.push(span.start, false, span.size(), "(__truthy(", 0);
                    self.push(
                        left.end,
                        false,
                        0,
                        format!(") {operator} __truthy("),
                        right.start - left.end,
                    );
                    self.push(span.end, true, span.size(), "))", 0);
                }
                self.instrument(&logical.left, index, base);
                self.instrument(&logical.right, index, base);
            }
            Expression::UnaryExpression(unary) => {
                self.slot(expression, index, base, unary.span.start);
                if unary.operator == UnaryOperator::LogicalNot {
                    let span = unary.span;
                    self.push(span.start, false, span.size(), "!__truthy(", 1);
                    self.push(span.end, true, span.size(), ")", 0);
                }
                if !matches!(unary.operator, UnaryOperator::Typeof | UnaryOperator::Delete) {
                    self.instrument(&unary.argument, index, base);
                }
            }
            Expression::ParenthesizedExpression(parenthesized) => {
                self.instrument(&parenthesized.expression, index, base);
            }
            // The value of an optional chain, a conditional and a `new` is shown; their parts are not instrumented,
            // which would change when they run.
            Expression::ChainExpression(_) | Expression::ConditionalExpression(_) | Expression::NewExpression(_) => {
                let start = expression.span().start;
                self.slot(expression, index, base, start);
            }
            _ => {}
        }
    }

    fn apply(&mut self) -> String {
        let mut edits = std::mem::take(&mut self.edits);
        edits.sort_by(|a, b| {
            a.at.cmp(&b.at).then_with(|| b.close.cmp(&a.close)).then_with(|| {
                if a.close {
                    a.span_len.cmp(&b.span_len).then(a.seq.cmp(&b.seq))
                } else {
                    b.span_len.cmp(&a.span_len).then(a.seq.cmp(&b.seq))
                }
            })
        });
        let mut out = String::with_capacity(self.source.len() + edits.len() * 12);
        let mut at = 0_usize;
        for edit in edits {
            let position = edit.at as usize;
            if position > at {
                out.push_str(&self.source[at..position]);
                at = position;
            }
            out.push_str(&edit.text);
            at = at.max(position + edit.delete as usize);
        }
        out.push_str(&self.source[at.min(self.source.len())..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conditions_of(source: &str) -> Rewritten {
        rewrite_spec(source).unwrap()
    }

    #[test]
    fn comparisons_become_groovy_compare_with_value_slots() {
        let rewritten =
            conditions_of("feature(\"f\", {\n  then({ priceMap }) {\n    priceMap.price == 16.99\n  },\n})\n");
        assert_eq!(
            rewritten.source,
            "feature(\"f\", {\n  then$pa({ priceMap }) {\n    __pa(0, ($pa_v) => ($pa_v(0, __cmp(\"==\", $pa_v(1, $pa_v(2, priceMap).price), 16.99))))\n  },\n})\n"
        );
        let condition = &rewritten.conditions[0];
        assert_eq!(condition.text, "priceMap.price == 16.99");
        assert_eq!(condition.line, 3);
        assert!(!condition.call);
        assert_eq!(
            condition.slots,
            vec![
                (15, "priceMap.price == 16.99".to_string()),
                (9, "priceMap.price".to_string()),
                (0, "priceMap".to_string())
            ]
        );
    }

    #[test]
    fn not_and_logical_use_groovy_truth_and_calls_keep_this() {
        let rewritten = conditions_of("feature('f', { expect: () => { !str.errorMessages && list.every(t => t) } })");
        assert_eq!(
            rewritten.source,
            "feature('f', { expect$pa: () => { __pa(0, ($pa_v) => ($pa_v(0, (__truthy($pa_v(1, !__truthy($pa_v(2, $pa_v(3, str).errorMessages)))) && __truthy($pa_v(4, $pa_v(5, list).every(t => t))))))) } })"
        );
        assert_eq!(rewritten.conditions[0].slots[0].0, 19);
    }

    #[test]
    fn assignments_are_not_conditions_and_other_members_and_calls_are_left() {
        let source = "let a\nfeature('f', {\n  when() { x == 1 },\n  then: function () { a = 2; check(a) }\n})\nother({ then() { b == 1 } })\n";
        let rewritten = conditions_of(source);
        assert_eq!(rewritten.conditions.len(), 1);
        assert!(rewritten.conditions[0].call);
        assert!(rewritten.source.contains("when() { x == 1 }"));
        assert!(rewritten.source.contains("then$pa: function () { a = 2; __pa(0,"));
        assert!(rewritten.source.contains("other({ then() { b == 1 } })"));
        assert_eq!(rewritten.source.lines().count(), source.lines().count());
    }

    #[test]
    fn concise_arrow_body_and_quoted_key_and_where_rows() {
        let rewritten = conditions_of("feature('f #a', { where: [{ a: 1 }], 'expect': ({ a }) => a < 2 })");
        assert_eq!(
            rewritten.source,
            "feature('f #a', { where: [{ a: 1 }], expect$pa: ({ a }) => __pa(0, ($pa_v) => ($pa_v(0, __cmp(\"<\", $pa_v(1, a), 2)))) })"
        );
    }

    #[test]
    fn strict_equality_and_arithmetic_keep_their_operators() {
        let rewritten = conditions_of("feature('f', { then() { a.b + 1 === c[0] } })");
        assert_eq!(
            rewritten.source,
            "feature('f', { then$pa() { __pa(0, ($pa_v) => ($pa_v(0, $pa_v(1, $pa_v(2, $pa_v(3, a).b) + 1) === $pa_v(4, $pa_v(5, c)[0])))) } })"
        );
    }

    #[test]
    fn depends_on_is_read_without_running() {
        let source =
            "const t = Date.now()\nspec({ dependsOn: ['A', \"B\"], stepwise: true })\nfeature('f', { expect() { 1 } })";
        assert_eq!(spec_depends_on(source), vec!["A".to_string(), "B".to_string()]);
        assert!(spec_depends_on("spec({})").is_empty());
    }

    #[test]
    fn suite_order_reads_the_names() {
        assert_eq!(
            suite_order("// the order of MantleUslSuite\nsuite([\"A\", 'B'])\n").unwrap(),
            vec!["A".to_string(), "B".to_string()]
        );
        assert!(suite_order("suite([A])").is_err());
    }

    #[test]
    fn a_syntax_error_names_its_line() {
        let err = rewrite_spec("feature('f', {\n  then() { a == }\n})").unwrap_err();
        assert!(err.to_string().starts_with("line 2: "), "{err}");
    }
}
