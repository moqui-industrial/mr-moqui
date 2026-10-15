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

//! `EntityDataLoader.check()`: read entity data as a load would and compare each record, nested records included,
//! with the database instead of writing it. Java's `CheckValueHandler` and `EntityValueBase.checkAgainstDatabase`.
//!
//! The readers hand every record to `store_record` and every service element to `call_data_file_service`. While a
//! check runs on this thread those divert here, so XML, CSV, JSON and zip data all check the same way.

use std::cell::RefCell;
use std::collections::BTreeMap;

use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, compare_obj, to_plain_string, Value};

use super::loader::{convert_fields, inherit_pk_fields, load_entity_data, load_entity_xml_text, Record};
use super::{DataFileService, LoadOptions};
use crate::entity::facade::EntityFacade;

/// What a check found: one message for each record not found and each field that did not match, and how many
/// fields were compared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckResult {
    pub messages: Vec<String>,
    pub fields_checked: u64,
}

thread_local! {
    static CHECKING: RefCell<Option<CheckResult>> = const { RefCell::new(None) };
}

/// Ends the check of this thread, also on an error or a panic.
struct CheckScope;

impl CheckScope {
    fn enter() -> Result<Self, Error> {
        CHECKING.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return Err(Error::new("a data check is already running on this thread"));
            }
            *slot = Some(CheckResult::default());
            Ok(CheckScope)
        })
    }

    fn finish(self) -> CheckResult {
        CHECKING.with(|slot| slot.borrow_mut().take()).unwrap_or_default()
    }
}

impl Drop for CheckScope {
    fn drop(&mut self) {
        CHECKING.with(|slot| slot.borrow_mut().take());
    }
}

/// Run `body`, a load of any kind, as a check: the records it reads are compared with the database and none is
/// written.
pub fn check_with<R>(body: impl FnOnce() -> Result<R, Error>) -> Result<(R, CheckResult), Error> {
    let scope = CheckScope::enter()?;
    let out = body()?;
    Ok((out, scope.finish()))
}

/// Check entity XML text against the database, as `makeDataLoader().xmlText(text).check()`.
pub fn check_entity_xml_text(
    facade: &EntityFacade,
    location: &str,
    text: &str,
    options: &LoadOptions,
) -> Result<CheckResult, Error> {
    check_with(|| load_entity_xml_text(facade, location, text, options)).map(|(_, result)| result)
}

/// Check the data of one location, a file or a directory, as `makeDataLoader().location(location).check()`. When
/// the location gave messages, a last one names it, as Java's.
pub fn check_entity_data(facade: &EntityFacade, location: &str, options: &LoadOptions) -> Result<CheckResult, Error> {
    let (loaded, mut result) = check_with(|| load_entity_data(facade, location, options))?;
    if !result.messages.is_empty() {
        result
            .messages
            .push(format!("-- Checked data ({} records) in {location}", loaded.records));
    }
    Ok(result)
}

/// True while a check runs on this thread: the readers then check records instead of storing them.
pub(super) fn checking() -> bool {
    CHECKING.with(|slot| slot.borrow().is_some())
}

/// Check one record read from the data and its nested records. Returns how many records it read, as a store
/// returns how many it wrote.
pub(super) fn check_record(facade: &EntityFacade, record: &Record) -> Result<u64, Error> {
    let mut messages = Vec::new();
    let mut fields_checked = 0_u64;
    let records = check_tree(facade, record, &BTreeMap::new(), &mut messages, &mut fields_checked)?;
    CHECKING.with(|slot| {
        if let Some(result) = slot.borrow_mut().as_mut() {
            result.messages.extend(messages);
            result.fields_checked = result.fields_checked.saturating_add(fields_checked);
        }
    });
    Ok(records)
}

/// A service element is not called by a check, as Java's.
pub(super) fn skip_service(service: &DataFileService) {
    let message = format!(
        "Doing check only so not calling service [{}] with parameters {:?}",
        service.name, service.parameters
    );
    CHECKING.with(|slot| {
        if let Some(result) = slot.borrow_mut().as_mut() {
            result.messages.push(message);
        }
    });
}

fn check_tree(
    facade: &EntityFacade,
    record: &Record,
    ancestor: &BTreeMap<String, Value>,
    messages: &mut Vec<String>,
    fields_checked: &mut u64,
) -> Result<u64, Error> {
    let mut fields = record.fields.clone();
    inherit_pk_fields(facade, &record.entity, ancestor, &mut fields)?;
    convert_fields(facade, &record.entity, &mut fields)?;
    let (pk_names, non_pk_names) = facade.with_entity(&record.entity, |entity, _, _| {
        let pk_names: Vec<String> = entity.pk_field_names().into_iter().map(str::to_string).collect();
        let non_pk_names: Vec<String> = entity
            .fields
            .iter()
            .filter(|field| !field.is_pk && field.name != "lastUpdatedStamp")
            .map(|field| field.name.clone())
            .collect();
        Ok((pk_names, non_pk_names))
    })?;
    let pk: Vec<(String, Value)> = pk_names
        .iter()
        .map(|name| (name.clone(), fields.get(name).cloned().unwrap_or(Value::Null)))
        .collect();
    let pk_text = pk_display(&pk);
    let found = if pk.iter().any(|(_, value)| value.is_null()) {
        None
    } else {
        let mut find = facade.find(&record.entity)?;
        for (name, value) in &pk {
            find = find.eq(name, value.clone());
        }
        find.one()?
    };
    match found {
        None => messages.push(format!(
            "Entity {} record not found for primary key {pk_text}",
            record.entity
        )),
        Some(db) => {
            for name in &non_pk_names {
                *fields_checked = fields_checked.saturating_add(1);
                let Some(check) = fields.get(name).filter(|value| !value.is_null()) else {
                    continue;
                };
                let stored = db.get(name).cloned().unwrap_or(Value::Null);
                if !same(check, &stored) {
                    messages.push(format!(
                        "Field {}.{name} did not match; check (file) value [{}], db value [{}] for primary key {pk_text}",
                        record.entity,
                        value_display(check),
                        value_display(&stored)
                    ));
                }
            }
        }
    }
    // The nested records get this record's keys, as a load passes them down.
    let mut shared: BTreeMap<String, Value> = pk.into_iter().filter(|(_, value)| !value.is_null()).collect();
    for (name, value) in ancestor {
        if !value.is_null() {
            shared.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    let mut records = 1_u64;
    for child in &record.children {
        records = records.saturating_add(check_tree(facade, child, &shared, messages, fields_checked)?);
    }
    Ok(records)
}

/// Java compares with `compareTo` when it can, so `1.50` matches `1.5`; otherwise with `equals`. A field the database
/// holds as null never matches a value.
fn same(check: &Value, stored: &Value) -> bool {
    if stored.is_null() {
        return false;
    }
    match compare_obj(check, stored) {
        Ok(order) => order == std::cmp::Ordering::Equal,
        // Text the field type did not take, such as `1800.0` for an integer field, still matches by number.
        Err(_) => match (as_decimal(check), as_decimal(stored)) {
            (Some(left), Some(right)) => compare_obj(&left, &right).is_ok_and(|order| order.is_eq()),
            _ => to_plain_string(check) == to_plain_string(stored),
        },
    }
}

fn as_decimal(value: &Value) -> Option<Value> {
    let converted = basic_convert(value, "BigDecimal");
    matches!(converted.value, Value::Decimal(_)).then_some(converted.value)
}

/// Java's `Map.toString` of the primary key: `{orderId=55500, orderItemSeqId=01}`.
fn pk_display(pk: &[(String, Value)]) -> String {
    let parts: Vec<String> = pk
        .iter()
        .map(|(name, value)| format!("{name}={}", value_display(value)))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// A time stamp shows as its date and time, which reads better in a message than the epoch millis a plain string
/// gives.
fn value_display(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Timestamp(stamp) => stamp.to_string(),
        other => to_plain_string(other),
    }
}
