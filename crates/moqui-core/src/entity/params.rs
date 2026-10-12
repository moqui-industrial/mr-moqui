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

//! Bind values in the same order `moqui_db` renders them.

use std::collections::BTreeMap;

use moqui_db::dialect::Dialect;
use moqui_db::model::{EntityDefinition, EntityModel, FieldDefinition};
use moqui_db::sql::{Condition, LimitOffset};
use moqui_lib::object::{basic_convert, Value};

use crate::driver::DbRow;

/// Java type name for a dictionary field, when the dictionary has that type.
pub(crate) fn java_of(model: &EntityModel, field: &FieldDefinition) -> Option<String> {
    model
        .dictionary_type(&field.field_type)
        .map(|entry| entry.java_type.clone())
}

pub(crate) fn condition_params(condition: &Condition) -> Vec<Value> {
    let mut out = Vec::new();
    if renders(condition) {
        push_params(&mut out, condition);
    }
    out
}

pub(crate) fn page_params(dialect: &Dialect, page: LimitOffset) -> Vec<Value> {
    let limit_style = dialect.offset_style.as_deref() == Some("limit");
    match (limit_style, page.limit, page.offset) {
        (false, Some(limit), offset) => vec![Value::I64(limit), Value::I64(offset.unwrap_or(0))],
        (false, None, Some(offset)) => vec![Value::I64(offset)],
        (true, Some(limit), None) => vec![Value::I64(limit)],
        (true, limit, Some(offset)) => {
            let mut params = vec![Value::I64(offset)];
            if let Some(limit) = limit {
                params.push(Value::I64(limit));
            } else {
                params.push(Value::I64(i64::MAX));
            }
            params
        }
        _ => Vec::new(),
    }
}

pub(crate) fn entity_columns(entity: &EntityDefinition, model: &EntityModel) -> Vec<(String, String)> {
    entity
        .fields
        .iter()
        .map(|field| (field.name.clone(), java_of(model, field).unwrap_or_default()))
        .collect()
}

/// Output column name to Java type. Function aliases stay unconverted.
pub(crate) fn view_columns(
    view: &EntityDefinition,
    model: &EntityModel,
) -> Result<Vec<(String, String)>, moqui_lib::error::Error> {
    let mut types = BTreeMap::new();
    for alias in &view.aliases {
        if alias.function.is_some() {
            types.entry(alias.name.clone()).or_insert_with(String::new);
            continue;
        }
        let java = alias_java(
            view,
            model,
            alias.entity_alias.as_deref(),
            alias.field.as_deref().unwrap_or(alias.name.as_str()),
        )?
        .unwrap_or_default();
        types.insert(alias.name.clone(), java);
    }
    for alias_all in &view.alias_alls {
        let Some(member) = view.member(&alias_all.entity_alias) else {
            continue;
        };
        let member_entity = model.entity(&member.entity_name)?;
        let excluded: Vec<&str> = alias_all.exclude.iter().map(String::as_str).collect();
        for field in &member_entity.fields {
            if excluded.contains(&field.name.as_str()) {
                continue;
            }
            let out_name = match alias_all.prefix.as_deref() {
                Some(prefix) => prefixed_name(prefix, &field.name),
                None => field.name.clone(),
            };
            types
                .entry(out_name)
                .or_insert_with(|| java_of(model, field).unwrap_or_default());
        }
    }
    Ok(types.into_iter().collect())
}

fn alias_java(
    view: &EntityDefinition,
    model: &EntityModel,
    entity_alias: Option<&str>,
    field_name: &str,
) -> Result<Option<String>, moqui_lib::error::Error> {
    let Some(entity_alias) = entity_alias else {
        return Ok(None);
    };
    let Some(member) = view.member(entity_alias) else {
        return Ok(None);
    };
    let member_entity = model.entity(&member.entity_name)?;
    Ok(member_entity.field(field_name).and_then(|field| java_of(model, field)))
}

fn prefixed_name(prefix: &str, field: &str) -> String {
    let mut name = String::with_capacity(prefix.len() + field.len());
    name.push_str(prefix);
    let mut chars = field.chars();
    if let Some(first) = chars.next() {
        name.push(first.to_ascii_uppercase());
        name.push_str(chars.as_str());
    }
    name
}

pub(crate) fn map_row(columns: &[(String, String)], row: &DbRow) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    if columns.is_empty() {
        for (name, value) in &row.columns {
            values.insert(name.clone(), value.clone());
        }
        return values;
    }
    for (name, java) in columns {
        let raw = row.get(name).cloned().unwrap_or(Value::Null);
        let value = if java.is_empty() {
            raw
        } else {
            basic_convert(&raw, java).value
        };
        values.insert(name.clone(), value);
    }
    for (name, value) in &row.columns {
        values.entry(name.clone()).or_insert_with(|| value.clone());
    }
    values
}

fn renders(condition: &Condition) -> bool {
    match condition {
        Condition::And(list) | Condition::Or(list) => list.iter().any(renders),
        Condition::Not(inner) => renders(inner),
        _ => true,
    }
}

fn push_params(out: &mut Vec<Value>, condition: &Condition) {
    match condition {
        Condition::Equals(_, value)
        | Condition::NotEquals(_, value)
        | Condition::Less(_, value)
        | Condition::Greater(_, value)
        | Condition::LessEqual(_, value)
        | Condition::GreaterEqual(_, value)
        | Condition::Like(_, value)
        | Condition::NotLike(_, value)
        | Condition::Matches(_, value) => out.push(value.clone()),
        Condition::Between(_, low, high) => {
            out.push(low.clone());
            out.push(high.clone());
        }
        Condition::In(_, values) | Condition::NotIn(_, values) => {
            if !values.is_empty() {
                out.extend(values.iter().cloned());
            }
        }
        Condition::IsNull(_) | Condition::IsNotNull(_) => {}
        Condition::And(list) | Condition::Or(list) => {
            for child in list {
                if renders(child) {
                    push_params(out, child);
                }
            }
        }
        Condition::Not(inner) => {
            if renders(inner) {
                push_params(out, inner);
            }
        }
    }
}
