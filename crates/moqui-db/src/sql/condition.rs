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

//! Condition tree. `EntityFind` builds it. This module only renders it.
//!
//! An empty `In` list is false: `1 = 0`. An empty `NotIn` list is true: `1 = 1`.
//! `And` and `Or` with nothing to render are omitted.

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use crate::dialect::Dialect;

use super::ident::{placeholders, quote_ident};

/// Entity alias (optional) plus a field name. Names are rendered as stored, after `name-replace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRef {
    pub entity_alias: Option<String>,
    pub name: String,
    /// SQL written in place of the column. Only the view renderer sets it, from the model's own
    /// aggregate and function aliases, so no input ever reaches it.
    pub expr: Option<String>,
}

impl FieldRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            entity_alias: None,
            name: name.into(),
            expr: None,
        }
    }

    pub fn aliased(entity_alias: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            entity_alias: Some(entity_alias.into()),
            name: name.into(),
            expr: None,
        }
    }

    /// A field that is a SQL expression, such as `SUM("OI"."quantity")`. `name` is its alias.
    pub fn expression(name: impl Into<String>, expr: impl Into<String>) -> Self {
        Self {
            entity_alias: None,
            name: name.into(),
            expr: Some(expr.into()),
        }
    }
}

impl From<&str> for FieldRef {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for FieldRef {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

/// One order-by entry. `nulls` is written only for a Postgres profile that does not set `never-nulls`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBy {
    pub field: FieldRef,
    pub descending: bool,
    pub nulls: Option<Nulls>,
}

impl OrderBy {
    pub fn asc(field: impl Into<FieldRef>) -> Self {
        Self {
            field: field.into(),
            descending: false,
            nulls: None,
        }
    }

    pub fn desc(field: impl Into<FieldRef>) -> Self {
        Self {
            field: field.into(),
            descending: true,
            nulls: None,
        }
    }

    pub fn nulls(mut self, nulls: Nulls) -> Self {
        self.nulls = Some(nulls);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nulls {
    First,
    Last,
}

/// Limit and offset. Absent means that clause is left out, except Postgres with a limit.
///
/// Postgres (`offset-style` absent) writes `LIMIT ? OFFSET ?` whenever a limit is set, and
/// `OFFSET ?` when only an offset is set. MySQL (`offset-style` `limit`) writes `LIMIT ?`, or
/// `LIMIT ?, ?` (offset, then limit) when both are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LimitOffset {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Predicate `EntityFind` will build. Values are not inlined. Each bound value is `?`.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Equals(FieldRef, Value),
    NotEquals(FieldRef, Value),
    Less(FieldRef, Value),
    Greater(FieldRef, Value),
    LessEqual(FieldRef, Value),
    GreaterEqual(FieldRef, Value),
    In(FieldRef, Vec<Value>),
    NotIn(FieldRef, Vec<Value>),
    Between(FieldRef, Value, Value),
    Like(FieldRef, Value),
    NotLike(FieldRef, Value),
    IsNull(FieldRef),
    IsNotNull(FieldRef),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Not(Box<Condition>),
    /// Full text. The SQL comes from the dialect `fts-style` and `fts-config`.
    Matches(FieldRef, Value),
    /// One field against another: `to-field-name` on an `econdition`.
    CompareFields(FieldRef, CompareOp, FieldRef),
    /// A comparison made on `UPPER(field)` and `UPPER(value)`. The inner condition must be one
    /// comparison against a value, not a join of conditions.
    IgnoreCase(Box<Condition>),
}

/// The comparison between two fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Equals,
    NotEquals,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

impl CompareOp {
    fn sql(self) -> &'static str {
        match self {
            Self::Equals => "=",
            Self::NotEquals => "<>",
            Self::Less => "<",
            Self::Greater => ">",
            Self::LessEqual => "<=",
            Self::GreaterEqual => ">=",
        }
    }
}

impl Condition {
    pub fn equals(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::Equals(field.into(), value)
    }

    pub fn not_equals(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::NotEquals(field.into(), value)
    }

    pub fn less(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::Less(field.into(), value)
    }

    pub fn greater(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::Greater(field.into(), value)
    }

    pub fn less_equal(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::LessEqual(field.into(), value)
    }

    pub fn greater_equal(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::GreaterEqual(field.into(), value)
    }

    pub fn is_in(field: impl Into<FieldRef>, values: Vec<Value>) -> Self {
        Self::In(field.into(), values)
    }

    pub fn not_in(field: impl Into<FieldRef>, values: Vec<Value>) -> Self {
        Self::NotIn(field.into(), values)
    }

    pub fn between(field: impl Into<FieldRef>, low: Value, high: Value) -> Self {
        Self::Between(field.into(), low, high)
    }

    pub fn like(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::Like(field.into(), value)
    }

    pub fn not_like(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::NotLike(field.into(), value)
    }

    pub fn is_null(field: impl Into<FieldRef>) -> Self {
        Self::IsNull(field.into())
    }

    pub fn is_not_null(field: impl Into<FieldRef>) -> Self {
        Self::IsNotNull(field.into())
    }

    pub fn and(conditions: Vec<Condition>) -> Self {
        Self::And(conditions)
    }

    pub fn or(conditions: Vec<Condition>) -> Self {
        Self::Or(conditions)
    }

    pub fn negate(condition: Condition) -> Self {
        Self::Not(Box::new(condition))
    }

    pub fn matches(field: impl Into<FieldRef>, value: Value) -> Self {
        Self::Matches(field.into(), value)
    }

    pub fn compare_fields(left: impl Into<FieldRef>, op: CompareOp, right: impl Into<FieldRef>) -> Self {
        Self::CompareFields(left.into(), op, right.into())
    }

    /// `condition` compared without regard to case.
    pub fn ignore_case(condition: Condition) -> Self {
        Self::IgnoreCase(Box::new(condition))
    }

    /// Rewrite every field in the tree. The view renderer uses this to turn alias names into the
    /// columns or expressions they stand for.
    #[must_use]
    pub fn map_fields(self, change: &mut impl FnMut(FieldRef) -> FieldRef) -> Self {
        match self {
            Self::Equals(field, value) => Self::Equals(change(field), value),
            Self::NotEquals(field, value) => Self::NotEquals(change(field), value),
            Self::Less(field, value) => Self::Less(change(field), value),
            Self::Greater(field, value) => Self::Greater(change(field), value),
            Self::LessEqual(field, value) => Self::LessEqual(change(field), value),
            Self::GreaterEqual(field, value) => Self::GreaterEqual(change(field), value),
            Self::In(field, values) => Self::In(change(field), values),
            Self::NotIn(field, values) => Self::NotIn(change(field), values),
            Self::Between(field, low, high) => Self::Between(change(field), low, high),
            Self::Like(field, value) => Self::Like(change(field), value),
            Self::NotLike(field, value) => Self::NotLike(change(field), value),
            Self::IsNull(field) => Self::IsNull(change(field)),
            Self::IsNotNull(field) => Self::IsNotNull(change(field)),
            Self::Matches(field, value) => Self::Matches(change(field), value),
            Self::CompareFields(left, op, right) => {
                let left = change(left);
                Self::CompareFields(left, op, change(right))
            }
            Self::And(list) => Self::And(list.into_iter().map(|child| child.map_fields(change)).collect()),
            Self::Or(list) => Self::Or(list.into_iter().map(|child| child.map_fields(change)).collect()),
            Self::Not(inner) => Self::Not(Box::new(inner.map_fields(change))),
            Self::IgnoreCase(inner) => Self::IgnoreCase(Box::new(inner.map_fields(change))),
        }
    }

    /// The same condition with every text value upper-cased: what an ignore-case comparison binds.
    #[must_use]
    pub fn upper_values(self) -> Self {
        let upper = |value: Value| match value {
            Value::String(text) => Value::String(text.to_uppercase()),
            other => other,
        };
        match self {
            Self::Equals(field, value) => Self::Equals(field, upper(value)),
            Self::NotEquals(field, value) => Self::NotEquals(field, upper(value)),
            Self::Less(field, value) => Self::Less(field, upper(value)),
            Self::Greater(field, value) => Self::Greater(field, upper(value)),
            Self::LessEqual(field, value) => Self::LessEqual(field, upper(value)),
            Self::GreaterEqual(field, value) => Self::GreaterEqual(field, upper(value)),
            Self::Like(field, value) => Self::Like(field, upper(value)),
            Self::NotLike(field, value) => Self::NotLike(field, upper(value)),
            Self::In(field, values) => Self::In(field, values.into_iter().map(upper).collect()),
            Self::NotIn(field, values) => Self::NotIn(field, values.into_iter().map(upper).collect()),
            Self::Between(field, low, high) => Self::Between(field, upper(low), upper(high)),
            Self::And(list) => Self::And(list.into_iter().map(Self::upper_values).collect()),
            Self::Or(list) => Self::Or(list.into_iter().map(Self::upper_values).collect()),
            Self::Not(inner) => Self::Not(Box::new(inner.upper_values())),
            Self::IgnoreCase(inner) => Self::IgnoreCase(Box::new(inner.upper_values())),
            other => other,
        }
    }

    /// True when any field in the tree satisfies `test`.
    pub fn any_field(&self, test: &mut impl FnMut(&FieldRef) -> bool) -> bool {
        match self {
            Self::Equals(field, _)
            | Self::NotEquals(field, _)
            | Self::Less(field, _)
            | Self::Greater(field, _)
            | Self::LessEqual(field, _)
            | Self::GreaterEqual(field, _)
            | Self::In(field, _)
            | Self::NotIn(field, _)
            | Self::Between(field, _, _)
            | Self::Like(field, _)
            | Self::NotLike(field, _)
            | Self::IsNull(field)
            | Self::IsNotNull(field)
            | Self::Matches(field, _) => test(field),
            Self::CompareFields(left, _, right) => test(left) || test(right),
            Self::And(list) | Self::Or(list) => list.iter().any(|child| child.any_field(test)),
            Self::Not(inner) | Self::IgnoreCase(inner) => inner.any_field(test),
        }
    }
}

/// Render a condition. An empty `And` or `Or` returns an empty string.
pub fn render_condition(condition: &Condition, dialect: &Dialect) -> Result<String, Error> {
    let mut out = String::new();
    write_condition(&mut out, condition, dialect)?;
    Ok(out)
}

fn write_condition(out: &mut String, condition: &Condition, dialect: &Dialect) -> Result<(), Error> {
    match condition {
        Condition::Equals(field, _) => compare(out, dialect, field, "=", 1),
        Condition::NotEquals(field, _) => compare(out, dialect, field, "<>", 1),
        Condition::Less(field, _) => compare(out, dialect, field, "<", 1),
        Condition::Greater(field, _) => compare(out, dialect, field, ">", 1),
        Condition::LessEqual(field, _) => compare(out, dialect, field, "<=", 1),
        Condition::GreaterEqual(field, _) => compare(out, dialect, field, ">=", 1),
        Condition::Like(field, _) => compare(out, dialect, field, "LIKE", 1),
        Condition::NotLike(field, _) => compare(out, dialect, field, "NOT LIKE", 1),
        Condition::IsNull(field) => compare(out, dialect, field, "IS NULL", 0),
        Condition::IsNotNull(field) => compare(out, dialect, field, "IS NOT NULL", 0),
        Condition::Between(field, _, _) => compare(out, dialect, field, "BETWEEN", 2),
        Condition::In(field, values) => write_in(out, dialect, field, values, false),
        Condition::NotIn(field, values) => write_in(out, dialect, field, values, true),
        Condition::And(conditions) => write_join(out, conditions, "AND", dialect)?,
        Condition::Or(conditions) => write_join(out, conditions, "OR", dialect)?,
        Condition::Not(inner) => {
            let rendered = render_condition(inner, dialect)?;
            if !rendered.is_empty() {
                out.push_str("NOT (");
                out.push_str(&rendered);
                out.push(')');
            }
        }
        Condition::Matches(field, _) => write_matches(out, dialect, field)?,
        Condition::CompareFields(left, op, right) => {
            write_field(out, dialect, left);
            out.push(' ');
            out.push_str(op.sql());
            out.push(' ');
            write_field(out, dialect, right);
        }
        Condition::IgnoreCase(inner) => write_ignore_case(out, dialect, inner)?,
    }
    Ok(())
}

/// `UPPER(field) op UPPER(?)`. The caller upper-cases the bound values.
fn write_ignore_case(out: &mut String, dialect: &Dialect, inner: &Condition) -> Result<(), Error> {
    let upper_field = |out: &mut String, field: &FieldRef| {
        out.push_str("UPPER(");
        write_field(out, dialect, field);
        out.push(')');
    };
    let upper_value = |out: &mut String| {
        out.push_str("UPPER(");
        out.push_str(dialect.placeholder);
        out.push(')');
    };
    let simple = |out: &mut String, field: &FieldRef, operator: &str| {
        upper_field(out, field);
        out.push(' ');
        out.push_str(operator);
        out.push(' ');
        upper_value(out);
    };
    match inner {
        Condition::Equals(field, _) => simple(out, field, "="),
        Condition::NotEquals(field, _) => simple(out, field, "<>"),
        Condition::Less(field, _) => simple(out, field, "<"),
        Condition::Greater(field, _) => simple(out, field, ">"),
        Condition::LessEqual(field, _) => simple(out, field, "<="),
        Condition::GreaterEqual(field, _) => simple(out, field, ">="),
        Condition::Like(field, _) => simple(out, field, "LIKE"),
        Condition::NotLike(field, _) => simple(out, field, "NOT LIKE"),
        Condition::Between(field, _, _) => {
            upper_field(out, field);
            out.push_str(" BETWEEN ");
            upper_value(out);
            out.push_str(" AND ");
            upper_value(out);
        }
        Condition::In(field, values) | Condition::NotIn(field, values) => {
            let negated = matches!(inner, Condition::NotIn(..));
            if values.is_empty() {
                out.push_str(if negated { "1 = 1" } else { "1 = 0" });
                return Ok(());
            }
            upper_field(out, field);
            out.push_str(if negated { " NOT IN (" } else { " IN (" });
            for index in 0..values.len() {
                if index > 0 {
                    out.push_str(", ");
                }
                upper_value(out);
            }
            out.push(')');
        }
        _ => return Err(Error::new("ignore-case applies to one comparison against a value")),
    }
    Ok(())
}

fn compare(out: &mut String, dialect: &Dialect, field: &FieldRef, operator: &str, value_count: usize) {
    write_field(out, dialect, field);
    out.push(' ');
    out.push_str(operator);
    if value_count == 0 {
        return;
    }
    if operator == "BETWEEN" {
        out.push(' ');
        out.push_str(dialect.placeholder);
        out.push_str(" AND ");
        out.push_str(dialect.placeholder);
        return;
    }
    out.push(' ');
    out.push_str(dialect.placeholder);
}

fn write_in(out: &mut String, dialect: &Dialect, field: &FieldRef, values: &[Value], negated: bool) {
    if values.is_empty() {
        out.push_str(if negated { "1 = 1" } else { "1 = 0" });
        return;
    }
    write_field(out, dialect, field);
    out.push(' ');
    out.push_str(if negated { "NOT IN (" } else { "IN (" });
    out.push_str(&placeholders(dialect, values.len()));
    out.push(')');
}

fn write_join(out: &mut String, conditions: &[Condition], operator: &str, dialect: &Dialect) -> Result<(), Error> {
    let mut parts = Vec::new();
    for condition in conditions {
        let rendered = render_condition(condition, dialect)?;
        if !rendered.is_empty() {
            parts.push(rendered);
        }
    }
    if parts.is_empty() {
        return Ok(());
    }
    if parts.len() == 1 {
        out.push_str(&parts[0]);
        return Ok(());
    }
    out.push('(');
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            out.push(' ');
            out.push_str(operator);
            out.push(' ');
        }
        out.push_str(part);
    }
    out.push(')');
    Ok(())
}

fn write_matches(out: &mut String, dialect: &Dialect, field: &FieldRef) -> Result<(), Error> {
    match dialect.fts_style.as_deref() {
        Some("postgres-tsvector") => {
            let config = fts_config(dialect)?;
            out.push_str("to_tsvector('");
            out.push_str(&config);
            out.push_str("', ");
            write_field(out, dialect, field);
            out.push_str(") @@ plainto_tsquery('");
            out.push_str(&config);
            out.push_str("', ");
            out.push_str(dialect.placeholder);
            out.push(')');
            Ok(())
        }
        Some("mysql-fulltext") => {
            out.push_str("MATCH (");
            write_field(out, dialect, field);
            out.push_str(") AGAINST (");
            out.push_str(dialect.placeholder);
            out.push_str(" IN NATURAL LANGUAGE MODE)");
            Ok(())
        }
        Some(style) => Err(Error::new(format!("matches does not support fts-style {style}"))),
        None => Err(Error::new(
            "matches needs fts-style mysql-fulltext or postgres-tsvector",
        )),
    }
}

pub(crate) fn fts_config(dialect: &Dialect) -> Result<String, Error> {
    let config = dialect.fts_config.as_deref().unwrap_or("english");
    if !is_fts_config(config) {
        return Err(Error::new(format!("fts-config {config} is not an identifier")));
    }
    Ok(config.to_string())
}

fn is_fts_config(config: &str) -> bool {
    let mut chars = config.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(crate) fn write_field(out: &mut String, dialect: &Dialect, field: &FieldRef) {
    if let Some(expr) = &field.expr {
        out.push_str(expr);
        return;
    }
    if let Some(alias) = &field.entity_alias {
        out.push_str(&quote_ident(dialect, alias));
        out.push('.');
    }
    out.push_str(&quote_ident(dialect, &field.name));
}
