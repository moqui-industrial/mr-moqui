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

//! One entity record and a list of records.

use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use moqui_db::model::EntityDefinition;
use moqui_db::sql::{delete, insert, insert_or_update, update, Condition};
use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, Value};

use super::cache::WriteGate;
use super::exec::{after_statement, execute_tx};
use super::facade::EntityFacade;
use super::field_rw;
use super::params::java_of;
use super::{cache, fk, load_feed, sequence};

/// One entity value. Field values are [`Value`].
#[derive(Clone, Debug, PartialEq)]
pub struct EntityValue {
    entity_name: String,
    fields: BTreeMap<String, Value>,
}

impl EntityValue {
    pub(crate) fn make(facade: &EntityFacade, entity_name: &str) -> Result<Self, Error> {
        let full_name = facade.with_entity(entity_name, |entity, _, _| Ok(entity.full_name.clone()))?;
        Ok(Self {
            entity_name: full_name,
            fields: BTreeMap::new(),
        })
    }

    pub(crate) fn from_fields(entity_name: String, fields: BTreeMap<String, Value>) -> Self {
        Self { entity_name, fields }
    }

    pub fn entity_name(&self) -> &str {
        &self.entity_name
    }

    pub fn get(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }

    pub fn set(&mut self, field: impl Into<String>, value: Value) {
        self.fields.insert(field.into(), value);
    }

    pub fn fields(&self) -> &BTreeMap<String, Value> {
        &self.fields
    }

    pub fn create(&self, facade: &EntityFacade) -> Result<(), Error> {
        facade.create_value(self)
    }

    pub fn update(&self, facade: &EntityFacade) -> Result<(), Error> {
        facade.update_value(self)
    }

    pub fn delete(&self, facade: &EntityFacade) -> Result<(), Error> {
        facade.delete_value(self)
    }

    pub fn refresh(&mut self, facade: &EntityFacade) -> Result<(), Error> {
        let fresh = facade.refresh_fields(&self.entity_name, &self.fields)?;
        self.fields = fresh;
        Ok(())
    }

    pub fn create_or_update(&self, facade: &EntityFacade) -> Result<(), Error> {
        facade.create_or_update_value(self)
    }

    /// Set the first primary key from the in-memory sequence bank.
    pub fn set_sequenced_id_primary(&mut self, facade: &EntityFacade) -> Result<(), Error> {
        let (field, id) = sequence::assign_primary(facade, &self.entity_name)?;
        self.set(field, Value::string(id));
        Ok(())
    }

    /// Set the last primary key to the next padded integer among rows with the other keys.
    pub fn set_sequenced_id_secondary(&mut self, facade: &EntityFacade) -> Result<(), Error> {
        let (field, id) = facade.next_secondary_id(&self.entity_name, &self.fields)?;
        self.set(field, Value::string(id));
        Ok(())
    }
}

/// A find result.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityList {
    values: Vec<EntityValue>,
}

impl EntityList {
    pub(crate) fn new(values: Vec<EntityValue>) -> Self {
        Self { values }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&EntityValue> {
        self.values.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &EntityValue> {
        self.values.iter()
    }

    pub fn values(&self) -> &[EntityValue] {
        &self.values
    }
}

impl EntityFacade {
    pub fn create_value(&self, value: &EntityValue) -> Result<(), Error> {
        let mut stack = HashSet::new();
        self.insert_record(value, &mut stack)
    }

    pub fn update_value(&self, value: &EntityValue) -> Result<(), Error> {
        self.write_update(&value.entity_name, &value.fields)
    }

    pub fn delete_value(&self, value: &EntityValue) -> Result<(), Error> {
        self.write_delete(&value.entity_name, &value.fields)
    }

    pub fn create_or_update_value(&self, value: &EntityValue) -> Result<(), Error> {
        let keys = self.pk_map(&value.entity_name, &value.fields)?;
        if self.find_by_pk(&value.entity_name, &keys)?.is_some() {
            self.update_value(value)
        } else {
            self.create_value(value)
        }
    }

    pub(crate) fn insert_record(&self, value: &EntityValue, stack: &mut HashSet<String>) -> Result<(), Error> {
        if !stack.insert(value.entity_name.clone()) {
            return Err(Error::new(format!("foreign key cycle at {}", value.entity_name)));
        }
        let result = self.insert_prepared(value, stack);
        stack.remove(&value.entity_name);
        result
    }

    fn insert_prepared(&self, value: &EntityValue, stack: &mut HashSet<String>) -> Result<(), Error> {
        let plain = self.normalize_values(&value.entity_name, &value.fields, false)?;
        self.require_pk(&value.entity_name, &plain)?;
        fk::enforce(self, &value.entity_name, &plain, stack)?;
        self.reject_view(&value.entity_name, "create")?;
        self.finish_write(&value.entity_name, "create", plain, false, false)?;
        Ok(())
    }

    fn write_update(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<(), Error> {
        let plain = self.normalize_values(entity_name, values, true)?;
        let _rules = load_feed::match_rules_with(self, entity_name, "update", Some(&plain));
        self.reject_view(entity_name, "update")?;
        let has_columns = self.with_entity(entity_name, |entity, _, _| {
            Ok(!present_columns(entity, &plain, true, true).is_empty())
        })?;
        if !has_columns {
            return Ok(());
        }
        self.finish_write(entity_name, "update", plain, true, true)
    }

    fn write_delete(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<(), Error> {
        let plain = self.normalize_values(entity_name, values, false)?;
        self.reject_view(entity_name, "delete")?;
        self.finish_write(entity_name, "delete", plain, false, false)
    }

    /// Insert, or update on a primary-key conflict. Values are sealed the same way as a create.
    pub(crate) fn upsert_record(&self, entity_name: &str, fields: &BTreeMap<String, Value>) -> Result<(), Error> {
        let plain = self.normalize_values(entity_name, fields, false)?;
        self.require_pk(entity_name, &plain)?;
        let mut stack = HashSet::new();
        fk::enforce(self, entity_name, &plain, &mut stack)?;
        self.reject_view(entity_name, "create")?;
        self.finish_write(entity_name, "upsert", plain, false, false)
    }

    /// Run the write, or leave it with the cache when the gate handles it. Values are plaintext.
    fn finish_write(
        &self,
        entity_name: &str,
        operation: &str,
        plain: BTreeMap<String, Value>,
        include_null: bool,
        skip_pk: bool,
    ) -> Result<(), Error> {
        // Declared first, so it drops last: the authorization cache is invalidated after the write.
        let _note = WriteNote(entity_name);
        let _artifact = EntityArtifact::push(self, entity_name, operation)?;
        let tx = self.transactions_for(entity_name)?;
        if tx.lock_track_enabled() {
            if let Ok(keys) = self.with_entity(entity_name, |entity, _, _| pk_values(entity, &plain)) {
                let pk = keys
                    .iter()
                    .map(|(name, value)| format!("{name}={}", moqui_lib::object::to_plain_string(value)))
                    .collect::<Vec<_>>()
                    .join(",");
                tx.note_entity_lock(entity_name, &pk);
            }
        }
        // An upsert matches ECA rules as a create, whether or not the row existed. Phase 6 `store`
        // needs a read first to pick create or update.
        let seal_operation = if operation == "upsert" { "create" } else { operation };
        // Update already matched in `write_update`, including the no-column return.
        if operation != "update" {
            let _rules = load_feed::match_rules_with(self, entity_name, seal_operation, Some(&plain));
        }
        let eca = crate::user::context_entered();
        if eca {
            crate::service::run_matched_eca(self, entity_name, seal_operation, true, &plain)?;
        }
        let cache_values = plain.clone();
        if cache::gate_write(self, entity_name, operation, &plain)? == WriteGate::Proceed {
            let after_values = eca.then(|| plain.clone());
            let prepared = self.seal_values(entity_name, seal_operation, plain)?;
            let (sql, params) = self.with_group(entity_name, |entity, dialect, _, _| {
                let columns = present_columns(entity, &prepared, include_null, skip_pk);
                let (sql, params) = if operation == "delete" {
                    let condition = pk_condition(entity, &prepared)?;
                    let sql = delete(entity, dialect, Some(&condition))?;
                    (sql, super::params::condition_params(&condition))
                } else if operation == "update" {
                    let condition = pk_condition(entity, &prepared)?;
                    let sql = update(entity, dialect, &columns, Some(&condition))?;
                    let mut params = bind_columns(&columns, &prepared);
                    params.extend(super::params::condition_params(&condition));
                    (sql, params)
                } else if operation == "upsert" {
                    let sql = insert_or_update(entity, dialect, &columns)?;
                    (sql, bind_columns(&columns, &prepared))
                } else {
                    let sql = insert(entity, dialect, &columns)?;
                    (sql, bind_columns(&columns, &prepared))
                };
                Ok((sql, params))
            })?;
            execute_tx(&tx, &sql, &params)?;
            if let Some(values) = after_values.as_ref() {
                crate::service::run_matched_eca(self, entity_name, seal_operation, false, values)?;
            }
        }
        cache::on_write(self, entity_name, operation, &cache_values)?;
        after_statement(self, &tx, true);
        Ok(())
    }

    pub(crate) fn refresh_fields(
        &self,
        entity_name: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        let keys = self.pk_map(entity_name, values)?;
        self.find_by_pk(entity_name, &keys)?
            .map(|value| value.fields)
            .ok_or_else(|| Error::new(format!("refresh found no {entity_name}")))
    }

    pub(crate) fn find_by_pk(
        &self,
        entity_name: &str,
        keys: &BTreeMap<String, Value>,
    ) -> Result<Option<EntityValue>, Error> {
        let mut find = self.find(entity_name)?;
        for (field, value) in keys {
            find = find.eq(field, value.clone());
        }
        find.one()
    }

    fn pk_map(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, Error> {
        self.with_entity(entity_name, |entity, _, _| pk_values(entity, values))
    }

    fn require_pk(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<(), Error> {
        self.pk_map(entity_name, values).map(|_| ())
    }

    pub(crate) fn next_secondary_id(
        &self,
        entity_name: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<(String, String), Error> {
        let (seq_field, pad, keys) = self.with_entity(entity_name, |entity, _, _| {
            let pks = entity.pk_field_names();
            if pks.len() < 2 {
                return Err(Error::new(format!(
                    "cannot call set_sequenced_id_secondary on {}, it needs at least 2 primary key fields",
                    entity.full_name
                )));
            }
            let seq_field = pks[pks.len() - 1].to_string();
            let mut keys = Vec::new();
            for pk in &pks[..pks.len() - 1] {
                let value = values
                    .get(*pk)
                    .filter(|value| !value.is_null())
                    .cloned()
                    .ok_or_else(|| Error::new(format!("primary key {pk} is required on {}", entity.full_name)))?;
                keys.push(((*pk).to_string(), value));
            }
            Ok((seq_field, entity.sequence_secondary_padded_length, keys))
        })?;
        let mut find = self.find(entity_name)?;
        for (field, value) in keys {
            find = find.eq(&field, value);
        }
        let rows = find.list()?;
        let mut highest: i64 = 0;
        let mut found = false;
        for row in rows.iter() {
            let Some(value) = row.get(&seq_field) else {
                continue;
            };
            if let Value::I64(number) = basic_convert(value, "Long").value {
                found = true;
                highest = highest.max(number);
            }
        }
        let next = if found { highest.saturating_add(1) } else { 1 };
        let width = usize::try_from(pad).unwrap_or(2);
        Ok((seq_field, format!("{next:0width$}")))
    }

    fn normalize_values(
        &self,
        entity_name: &str,
        values: &BTreeMap<String, Value>,
        update: bool,
    ) -> Result<BTreeMap<String, Value>, Error> {
        self.with_entity(entity_name, |entity, _, model| {
            let mut out = values.clone();
            if entity.field("lastUpdatedStamp").is_some() {
                let missing = out.get("lastUpdatedStamp").is_none_or(Value::is_null);
                if update || missing {
                    out.insert("lastUpdatedStamp".to_string(), now_stamp());
                }
            }
            let mut normalized = BTreeMap::new();
            for field in &entity.fields {
                let Some(value) = out.get(&field.name) else {
                    continue;
                };
                if value.is_null() {
                    normalized.insert(field.name.clone(), Value::Null);
                    continue;
                }
                let java = java_of(model, field).unwrap_or_default();
                let converted = if java.is_empty() {
                    value.clone()
                } else {
                    basic_convert(value, &java).value
                };
                normalized.insert(field.name.clone(), converted);
            }
            Ok(normalized)
        })
    }

    fn seal_values(
        &self,
        entity_name: &str,
        operation: &str,
        values: BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        let facade = self.clone();
        self.with_entity(entity_name, |entity, _, _| {
            field_rw::before_write(&facade, entity, operation, values)
        })
    }

    fn reject_view(&self, entity_name: &str, verb: &str) -> Result<(), Error> {
        self.with_entity(entity_name, |entity, _, _| {
            if entity.is_view {
                Err(Error::new(format!("cannot {verb} view-entity {}", entity.full_name)))
            } else {
                Ok(())
            }
        })
    }
}

fn pk_values(entity: &EntityDefinition, values: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, Error> {
    let mut keys = BTreeMap::new();
    for pk in entity.pk_field_names() {
        let value = values
            .get(pk)
            .filter(|value| !value.is_null())
            .cloned()
            .ok_or_else(|| Error::new(format!("primary key {pk} is required on {}", entity.full_name)))?;
        keys.insert(pk.to_string(), value);
    }
    if keys.is_empty() {
        return Err(Error::new(format!("{} has no primary key", entity.full_name)));
    }
    Ok(keys)
}

fn pk_condition(entity: &EntityDefinition, values: &BTreeMap<String, Value>) -> Result<Condition, Error> {
    let keys = pk_values(entity, values)?;
    let mut parts: Vec<Condition> = keys
        .into_iter()
        .map(|(field, value)| Condition::equals(field, value))
        .collect();
    if parts.len() == 1 {
        return Ok(parts.remove(0));
    }
    Ok(Condition::and(parts))
}

fn present_columns<'a>(
    entity: &'a EntityDefinition,
    values: &BTreeMap<String, Value>,
    include_null: bool,
    skip_pk: bool,
) -> Vec<&'a str> {
    entity
        .fields
        .iter()
        .filter(|field| {
            if skip_pk && field.is_pk {
                return false;
            }
            matches!(values.get(&field.name), Some(value) if include_null || !value.is_null())
        })
        .map(|field| field.name.as_str())
        .collect()
}

fn bind_columns(columns: &[&str], values: &BTreeMap<String, Value>) -> Vec<Value> {
    columns
        .iter()
        .map(|column| values.get(*column).cloned().unwrap_or(Value::Null))
        .collect()
}

/// Tells the authorization cache an entity changed, once the write is over.
struct WriteNote<'a>(&'a str);

impl Drop for WriteNote<'_> {
    fn drop(&mut self) {
        crate::user::note_entity_write(self.0);
    }
}

/// An `AT_ENTITY` frame on the artifact stack for one entity operation.
///
/// The frame is decided against the one below it, so a service or screen that was allowed to do
/// this inherits the entity access. An entity whose `authorize-skip` covers the operation is
/// pushed without needing an allow.
pub(crate) struct EntityArtifact {
    pushed: bool,
}

impl EntityArtifact {
    pub(crate) fn push(facade: &EntityFacade, entity_name: &str, operation: &str) -> Result<Self, Error> {
        if !crate::user::context_entered() {
            return Ok(Self { pushed: false });
        }
        let action = if operation == "upsert" { "create" } else { operation };
        let skip = facade
            .with_entity(entity_name, |entity, _, _| Ok(entity.authorize_skip.clone()))
            .unwrap_or_default();
        let requires_authz = !skips_authz(&skip, action);
        let outcome = crate::context::ExecutionContext::with_current(|ctx| {
            ctx.push_checked("AT_ENTITY", entity_name, action, requires_authz, false, None)
        });
        match outcome {
            Some(Ok(())) => Ok(Self { pushed: true }),
            Some(Err(err)) => Err(Error::new(err.message().to_string()).with_source(err)),
            None => Ok(Self { pushed: false }),
        }
    }
}

/// `authorize-skip` is `true`, or a list that can name `create` and `view`. Update and delete
/// are skipped only by `true`.
fn skips_authz(skip: &str, action: &str) -> bool {
    match skip {
        "" | "false" => false,
        "true" => true,
        list => matches!(action, "create" | "view") && list.split(',').any(|part| part.trim() == action),
    }
}

impl Drop for EntityArtifact {
    fn drop(&mut self) {
        if !self.pushed {
            return;
        }
        crate::context::ExecutionContext::with_current(|ctx| {
            let _ = ctx.pop();
        });
    }
}

fn now_stamp() -> Value {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}
