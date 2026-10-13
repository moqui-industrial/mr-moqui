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

//! Columns a find reads and their Java types. `moqui_db` binds the condition values itself.

use std::collections::BTreeMap;

use moqui_db::model::{EntityDefinition, EntityModel, FieldDefinition};
use moqui_lib::object::{basic_convert, Value};

use crate::driver::DbRow;

/// Java type name for a dictionary field, when the dictionary has that type.
pub(crate) fn java_of(model: &EntityModel, field: &FieldDefinition) -> Option<String> {
    model
        .dictionary_type(&field.field_type)
        .map(|entry| entry.java_type.clone())
}

pub(crate) fn entity_columns(entity: &EntityDefinition, model: &EntityModel) -> Vec<(String, String)> {
    entity
        .fields
        .iter()
        .map(|field| (field.name.clone(), java_of(model, field).unwrap_or_default()))
        .collect()
}

/// Output column name to Java type for a view. Function aliases stay unconverted.
pub(crate) fn view_columns(view: &EntityDefinition) -> Vec<(String, String)> {
    view.aliases
        .iter()
        .map(|alias| {
            let java = if alias.function.is_some() {
                String::new()
            } else {
                alias.java_type.clone()
            };
            (alias.name.clone(), java)
        })
        .collect()
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
