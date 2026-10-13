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

//! Reading context-stack values from Rust: entity values and lists as maps and items, and frame
//! calls the screen renderer makes between its own steps.

use indexmap::IndexMap;

use super::*;

/// The fields of a map or an entity value.
pub(crate) fn value_map(value: &Value) -> Option<IndexMap<String, Value>> {
    match value {
        Value::Map(entries) => Some(entries.clone()),
        Value::Object(object) => {
            let cell = object.downcast::<EntityCell>()?;
            let guard = cell.lock().unwrap_or_else(|err| err.into_inner());
            Some(
                guard
                    .fields()
                    .iter()
                    .map(|(key, item)| (key.clone(), item.clone()))
                    .collect(),
            )
        }
        _ => None,
    }
}

/// One field of a map or entity value. None when the field is not there.
pub(crate) fn value_field(value: &Value, field: &str) -> Option<Value> {
    match value {
        Value::Map(entries) => entries.get(field).cloned(),
        Value::Object(object) => {
            let cell = object.downcast::<EntityCell>()?;
            let guard = cell.lock().unwrap_or_else(|err| err.into_inner());
            guard.get(field).cloned()
        }
        _ => None,
    }
}

/// The entity name of an entity value.
pub(crate) fn value_entity_name(value: &Value) -> Option<String> {
    let Value::Object(object) = value else {
        return None;
    };
    let cell = object.downcast::<EntityCell>()?;
    let guard = cell.lock().unwrap_or_else(|err| err.into_inner());
    Some(guard.entity_name().to_string())
}

/// The items of a list, an entity list, or the values of a map.
pub(crate) fn value_items(value: &Value) -> Option<Vec<Value>> {
    match value {
        Value::List(items) => Some(items.clone()),
        Value::Map(entries) => Some(entries.values().cloned().collect()),
        Value::Object(object) => {
            let list = object.downcast::<EntityList>()?;
            Some(list.values().iter().map(|item| entity_value(item.clone())).collect())
        }
        _ => None,
    }
}

/// An entity value as a context-stack value, shared by reference with scripts.
fn entity_value(value: EntityValue) -> Value {
    Value::Object(HostObject(Arc::new(Mutex::new(value))))
}

/// Pop the top frame and return what it held.
pub(crate) fn pop_context_frame() -> IndexMap<String, Value> {
    STACK.with(|stack| {
        stack
            .borrow_mut()
            .pop()
            .ok()
            .flatten()
            .map(|frame| frame.borrow().clone())
            .unwrap_or_default()
    })
}

/// Put every entry in the top frame.
pub(crate) fn put_context_all(entries: &IndexMap<String, Value>) {
    STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        for (key, value) in entries {
            let _ = stack.put(key.clone(), value.clone());
        }
    });
}

/// The combined visible names.
pub(crate) fn context_combined() -> IndexMap<String, Value> {
    STACK.with(|stack| stack.borrow().combined_map())
}
