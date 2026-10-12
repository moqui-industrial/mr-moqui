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

//! Encrypt on write, decrypt on read, audit changed fields, and overlay localized fields.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use moqui_db::model::{EntityDefinition, EntityModel};
use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, decimal_compare, to_plain_string, Value};

use super::crypto::{decrypt_with_alts, encrypt};
use super::facade::EntityFacade;

const AUDIT_ENTITY: &str = "moqui.entity.EntityAuditLog";
const LOCALIZED_ENTITY: &str = "moqui.basic.LocalizedEntityField";
const TEXT_LIMIT: usize = 4000;

thread_local! {
    static RAW_READ: Cell<bool> = const { Cell::new(false) };
}

struct RawReadGuard;

impl RawReadGuard {
    fn enter() -> Self {
        RAW_READ.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for RawReadGuard {
    fn drop(&mut self) {
        RAW_READ.with(|flag| flag.set(false));
    }
}

fn raw_read() -> bool {
    RAW_READ.with(Cell::get)
}

/// Prepare field values before SQL. Encrypts, then the returned map is what the binder uses.
pub fn before_write(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    operation: &str,
    values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    audit_log(facade, entity, operation, &values)?;
    encrypt_fields(facade, entity, values)
}

/// Adjust a row after SQL and before the find cache stores it.
pub fn after_read(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let values = decrypt_fields(facade, entity, values)?;
    if raw_read() {
        return Ok(values);
    }
    localize(facade, entity, values)
}

fn encrypt_fields(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    mut values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let Some((primary, _)) = facade.crypt() else {
        return Ok(values);
    };
    for field in &entity.fields {
        if !field.encrypt {
            continue;
        }
        let Some(plain) = take_string(entity, &field.name, &values)? else {
            continue;
        };
        let stored = encrypt(&plain, &primary)?;
        values.insert(field.name.clone(), Value::string(stored));
    }
    Ok(values)
}

fn decrypt_fields(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    mut values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let Some((primary, alts)) = facade.crypt() else {
        return Ok(values);
    };
    for field in &entity.fields {
        if !field.encrypt {
            continue;
        }
        let Some(stored) = take_string(entity, &field.name, &values)? else {
            continue;
        };
        match decrypt_with_alts(&stored, &primary, &alts) {
            Ok(plain) => {
                values.insert(field.name.clone(), Value::string(plain));
            }
            // A value with no colon that is not hex was stored as plaintext.
            Err(_) if !stored.contains(':') && !is_hex_ciphertext(&stored) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(values)
}

fn take_string(
    entity: &EntityDefinition,
    field: &str,
    values: &BTreeMap<String, Value>,
) -> Result<Option<String>, Error> {
    let Some(value) = values.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    match value.as_str() {
        Some(text) => Ok(Some(text.to_string())),
        None => Err(Error::new(format!(
            "The encrypt attribute was set to true on non-String field {field} of entity {}",
            entity.full_name
        ))),
    }
}

fn is_hex_ciphertext(value: &str) -> bool {
    // Same acceptance as crypto::parse_hex, including an empty string.
    let bytes = value.as_bytes();
    bytes.len().is_multiple_of(2) && bytes.iter().all(u8::is_ascii_hexdigit)
}

fn audit_log(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    operation: &str,
    values: &BTreeMap<String, Value>,
) -> Result<(), Error> {
    if operation == "delete" || facade.flags().disable_audit_log || facade.sim_active() {
        return Ok(());
    }
    if entity.full_name == AUDIT_ENTITY {
        return Ok(());
    }
    if !entity.fields.iter().any(|field| {
        field
            .enable_audit_log
            .as_deref()
            .is_some_and(|flag| field_selected(operation, flag))
    }) {
        return Ok(());
    }
    let pk_names = entity.pk_field_names();
    if pk_names.is_empty() || pk_names.iter().any(|name| values.get(*name).is_none_or(Value::is_null)) {
        return Ok(());
    }
    let old = if operation == "update" {
        stored_row(facade, entity, values)?
    } else {
        BTreeMap::new()
    };
    for field in &entity.fields {
        let Some(flag) = field.enable_audit_log.as_deref() else {
            continue;
        };
        if !field_selected(operation, flag) || !values.contains_key(&field.name) {
            continue;
        }
        let new_value = values.get(&field.name);
        let old_value = old.get(&field.name);
        if operation == "create" && new_value.is_none_or(Value::is_null) {
            continue;
        }
        // "update" does not log the first non-null value. That is the initial set.
        if operation == "update" && flag == "update" && old_value.is_none_or(Value::is_null) {
            continue;
        }
        if operation == "update" && same_stored(old_value, new_value) {
            continue;
        }
        let new_text = audit_text(facade, field.encrypt, new_value)?;
        let old_text = audit_text(facade, field.encrypt, old_value)?;
        insert_audit(facade, entity, &field.name, values, new_text, old_text)?;
    }
    Ok(())
}

fn field_selected(operation: &str, flag: &str) -> bool {
    match flag {
        "true" => operation == "create" || operation == "update",
        "create" => operation == "create",
        "update" => operation == "update",
        _ => false,
    }
}

fn stored_row(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    values: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let _guard = RawReadGuard::enter();
    let mut find = facade.find(&entity.full_name)?;
    for name in entity.pk_field_names() {
        let Some(value) = values.get(name).cloned() else {
            return Ok(BTreeMap::new());
        };
        find = find.eq(name, value);
    }
    Ok(find.one()?.map(|row| row.fields().clone()).unwrap_or_default())
}

fn same_stored(old: Option<&Value>, new: Option<&Value>) -> bool {
    match (present(old), present(new)) {
        (None, None) => true,
        (Some(Value::Decimal(left)), Some(Value::Decimal(right))) => decimal_compare(*left, *right) == Ordering::Equal,
        (Some(left), Some(right)) => to_plain_string(left) == to_plain_string(right),
        _ => false,
    }
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|item| !item.is_null())
}

fn audit_text(facade: &EntityFacade, encrypt_field: bool, value: Option<&Value>) -> Result<Option<String>, Error> {
    let Some(value) = present(value) else {
        return Ok(None);
    };
    let plain = to_plain_string(value);
    let text = if encrypt_field {
        if let Some((primary, _)) = facade.crypt() {
            encrypt(&plain, &primary)?
        } else {
            plain
        }
    } else {
        plain
    };
    Ok(Some(clip_text(&text)))
}

fn clip_text(text: &str) -> String {
    if text.chars().nth(TEXT_LIMIT).is_none() {
        return text.to_string();
    }
    text.chars().take(TEXT_LIMIT).collect()
}

fn insert_audit(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    field_name: &str,
    values: &BTreeMap<String, Value>,
    new_text: Option<String>,
    old_text: Option<String>,
) -> Result<(), Error> {
    let mut row = facade.make_value(AUDIT_ENTITY)?;
    row.set_sequenced_id_primary(facade)?;
    row.set("changedEntityName", Value::string(entity.full_name.as_str()));
    row.set("changedFieldName", Value::string(field_name));
    row.set("changedDate", changed_now());
    if let Some(text) = new_text {
        row.set("newValueText", Value::string(text));
    }
    if let Some(text) = old_text {
        row.set("oldValueText", Value::string(text));
    }
    put_pk(&mut row, entity, values);
    row.create(facade)
}

fn put_pk(row: &mut super::value::EntityValue, entity: &EntityDefinition, values: &BTreeMap<String, Value>) {
    let names = entity.pk_field_names();
    if let Some(name) = names.first() {
        if let Some(text) = plain_text(values.get(*name)) {
            row.set("pkPrimaryValue", Value::string(text));
        }
    }
    if let Some(name) = names.get(1) {
        if let Some(text) = plain_text(values.get(*name)) {
            row.set("pkSecondaryValue", Value::string(text));
        }
    }
    if names.len() <= 2 {
        return;
    }
    let mut combined = String::new();
    for (index, name) in names.iter().skip(2).enumerate() {
        if index > 0 {
            combined.push(',');
        }
        let text = plain_text(values.get(*name)).unwrap_or_else(|| "null".to_string());
        combined.push_str(name);
        combined.push_str(":'");
        combined.push_str(&text);
        combined.push('\'');
    }
    if !combined.is_empty() {
        row.set("pkRestCombinedValue", Value::string(combined));
    }
}

fn plain_text(value: Option<&Value>) -> Option<String> {
    present(value).map(to_plain_string)
}

fn changed_now() -> Value {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}

struct LocalTarget {
    entity_name: String,
    field_name: String,
    column: String,
    pk_column: String,
}

fn localize(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let targets = if entity.is_view {
        facade.with_entity(&entity.full_name, |view, _, model| view_targets(view, model))?
    } else {
        plain_targets(entity)
    };
    if targets.is_empty() {
        return Ok(values);
    }
    let locales = locale_chain(&facade.locale());
    if locales.is_empty() {
        return Ok(values);
    }
    let mut values = values;
    for target in &targets {
        let Some(pk) = pk_text(values.get(&target.pk_column)) else {
            continue;
        };
        if let Some(localized) = find_localized(facade, target, &pk, &locales)? {
            values.insert(target.column.clone(), localized);
        }
    }
    Ok(values)
}

fn plain_targets(entity: &EntityDefinition) -> Vec<LocalTarget> {
    let Some(pk_column) = entity.pk_field_names().into_iter().next() else {
        return Vec::new();
    };
    let pk_column = pk_column.to_string();
    entity
        .fields
        .iter()
        .filter(|field| field.enable_localization)
        .map(|field| LocalTarget {
            entity_name: entity.full_name.clone(),
            field_name: field.name.clone(),
            column: field.name.clone(),
            pk_column: pk_column.clone(),
        })
        .collect()
}

fn view_targets(view: &EntityDefinition, model: &EntityModel) -> Result<Vec<LocalTarget>, Error> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for alias in &view.aliases {
        if alias.function.is_some() {
            continue;
        }
        let Some(entity_alias) = alias.entity_alias.as_deref() else {
            continue;
        };
        let Some(member_name) = member_entity_name(view, model, entity_alias)? else {
            continue;
        };
        let member = model.entity(&member_name)?;
        let field_name = alias.field.clone().unwrap_or_else(|| alias.name.clone());
        if !member.field(&field_name).is_some_and(|field| field.enable_localization) {
            continue;
        }
        let Some(pk_column) = member_pk_column(view, member, entity_alias) else {
            continue;
        };
        push_target(
            &mut targets,
            &mut seen,
            LocalTarget {
                entity_name: member.full_name.clone(),
                field_name,
                column: alias.name.clone(),
                pk_column,
            },
        );
    }
    for alias_all in &view.alias_alls {
        let Some(member_name) = member_entity_name(view, model, &alias_all.entity_alias)? else {
            continue;
        };
        let member = model.entity(&member_name)?;
        let Some(pk_column) = member_pk_column(view, member, &alias_all.entity_alias) else {
            continue;
        };
        for field in &member.fields {
            if !field.enable_localization || alias_all.exclude.iter().any(|name| name == &field.name) {
                continue;
            }
            let column = match alias_all.prefix.as_deref() {
                Some(prefix) => prefixed_name(prefix, &field.name),
                None => field.name.clone(),
            };
            push_target(
                &mut targets,
                &mut seen,
                LocalTarget {
                    entity_name: member.full_name.clone(),
                    field_name: field.name.clone(),
                    column,
                    pk_column: pk_column.clone(),
                },
            );
        }
    }
    Ok(targets)
}

fn push_target(targets: &mut Vec<LocalTarget>, seen: &mut HashSet<String>, target: LocalTarget) {
    if seen.insert(target.column.clone()) {
        targets.push(target);
    }
}

fn member_entity_name(view: &EntityDefinition, model: &EntityModel, alias: &str) -> Result<Option<String>, Error> {
    if let Some(member) = view.member(alias) {
        return Ok(Some(member.entity_name.clone()));
    }
    let Some(link) = view.member_relationships.iter().find(|item| item.entity_alias == alias) else {
        return Ok(None);
    };
    let Some(from) = view.member(&link.join_from_alias) else {
        return Ok(None);
    };
    let from_entity = model.entity(&from.entity_name)?;
    Ok(from_entity
        .relationship(&link.relationship)
        .map(|relationship| relationship.related.clone()))
}

fn member_pk_column(view: &EntityDefinition, member: &EntityDefinition, entity_alias: &str) -> Option<String> {
    let pk = member.pk_field_names().into_iter().next()?.to_string();
    for alias in &view.aliases {
        if alias.function.is_some() || alias.entity_alias.as_deref() != Some(entity_alias) {
            continue;
        }
        let source = alias.field.as_deref().unwrap_or(alias.name.as_str());
        if source == pk {
            return Some(alias.name.clone());
        }
    }
    for alias_all in &view.alias_alls {
        if alias_all.entity_alias != entity_alias || alias_all.exclude.iter().any(|name| name == &pk) {
            continue;
        }
        return Some(match alias_all.prefix.as_deref() {
            Some(prefix) => prefixed_name(prefix, &pk),
            None => pk,
        });
    }
    None
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

fn pk_text(value: Option<&Value>) -> Option<String> {
    let text = plain_text(value)?;
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Full locale, then drop the region, then the language.
fn locale_chain(locale: &str) -> Vec<String> {
    let mut chain = Vec::new();
    push_locale(&mut chain, locale);
    let normalized = locale.replace('-', "_");
    push_locale(&mut chain, &normalized);
    let mut current = normalized;
    while let Some((head, _)) = current.rsplit_once('_') {
        if head.is_empty() {
            break;
        }
        push_locale(&mut chain, head);
        current = head.to_string();
    }
    chain
}

fn push_locale(chain: &mut Vec<String>, locale: &str) {
    if locale.is_empty() || chain.iter().any(|item| item == locale) {
        return;
    }
    chain.push(locale.to_string());
}

fn find_localized(
    facade: &EntityFacade,
    target: &LocalTarget,
    pk: &str,
    locales: &[String],
) -> Result<Option<Value>, Error> {
    for locale in locales {
        let row = facade
            .find(LOCALIZED_ENTITY)?
            .eq("entityName", Value::string(target.entity_name.as_str()))
            .eq("fieldName", Value::string(target.field_name.as_str()))
            .eq("pkValue", Value::string(pk))
            .eq("locale", Value::string(locale.as_str()))
            .one()?;
        if let Some(row) = row {
            return Ok(Some(row.get("localized").cloned().unwrap_or(Value::Null)));
        }
    }
    Ok(None)
}
