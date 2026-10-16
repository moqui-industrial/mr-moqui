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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use moqui_db::model::EntityDefinition;
use moqui_db::sql::{delete, insert, insert_or_update, update, Condition};
use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, Value};
use moqui_lib::sync::lock;

use super::cache::WriteGate;
use super::exec::{after_statement, execute_tx};
use super::facade::EntityFacade;
use super::field_rw;
use super::load_feed::EcaRule;
use super::params::java_of;
use super::{cache, fk, load_feed, sequence};

/// One entity value. Field values are [`Value`].
///
/// The fields are the stored values, decrypted. A value read by a find of an entity with localized fields also reads
/// those fields in the user's locale through [`Self::get_localized`] and [`Self::localized_fields`], as Java's
/// `get` and `getMap`; [`Self::get`], [`Self::fields`] and every write use the stored values.
#[derive(Clone, Debug)]
pub struct EntityValue {
    entity_name: String,
    fields: BTreeMap<String, Value>,
    l10n: Option<Box<Localizer>>,
}

/// Two values are equal when they hold the same stored fields of the same entity.
impl PartialEq for EntityValue {
    fn eq(&self, other: &Self) -> bool {
        self.entity_name == other.entity_name && self.fields == other.fields
    }
}

type LocalizedValues = HashMap<(String, String), Option<Value>>;

/// The localized fields of a value, and what each read in each locale, as Java's `localizedByLocaleByField`. A clone
/// shares what was read until either one sets a field.
#[derive(Clone)]
struct Localizer {
    targets: Arc<field_rw::L10nTargets>,
    memo: Arc<Mutex<LocalizedValues>>,
}

impl std::fmt::Debug for Localizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Localizer")
            .field("targets", &self.targets)
            .finish_non_exhaustive()
    }
}

/// A localized read that needs a lookup, taken from the value so a caller that holds the value behind a lock can let
/// go of it before the lookup runs. See [`EntityValue::read_localized`].
pub(crate) struct LocalizedRead {
    l10n: Localizer,
    field: String,
    locale: String,
    /// The stored fields the lookup reads: the field itself and the primary key.
    fields: BTreeMap<String, Value>,
}

impl LocalizedRead {
    pub(crate) fn run(self, facade: &EntityFacade) -> Option<Value> {
        let found = field_rw::localize_field(facade, &self.l10n.targets, &self.fields, &self.field, &self.locale);
        let out = found.clone().or_else(|| self.fields.get(&self.field).cloned());
        lock(&self.l10n.memo).insert((self.field, self.locale), found);
        out
    }
}

impl EntityValue {
    pub(crate) fn make(facade: &EntityFacade, entity_name: &str) -> Result<Self, Error> {
        let full_name = facade.with_entity(entity_name, |entity, _, _| Ok(entity.full_name.clone()))?;
        Ok(Self {
            entity_name: full_name,
            fields: BTreeMap::new(),
            l10n: None,
        })
    }

    pub(crate) fn from_fields(entity_name: String, fields: BTreeMap<String, Value>) -> Self {
        Self {
            entity_name,
            fields,
            l10n: None,
        }
    }

    /// Read the fields in `targets` localized. `None` leaves the value plain.
    pub(crate) fn with_l10n(mut self, targets: Option<&Arc<field_rw::L10nTargets>>) -> Self {
        self.l10n = targets.map(|targets| {
            Box::new(Localizer {
                targets: Arc::clone(targets),
                memo: Arc::new(Mutex::new(HashMap::new())),
            })
        });
        self
    }

    pub fn entity_name(&self) -> &str {
        &self.entity_name
    }

    /// The stored value of `field`.
    pub fn get(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }

    /// Java's `get`: `field` in the user's locale when it is a localized field, else the stored value. A lookup is made
    /// once per field and locale.
    pub fn get_localized(&self, facade: &EntityFacade, field: &str) -> Option<Value> {
        match self.read_localized(facade, field) {
            Ok(value) => value,
            Err(read) => read.run(facade),
        }
    }

    /// Whether `field` reads localized, so a caller can skip finding the facade for every other field.
    pub(crate) fn localizes(&self, field: &str) -> bool {
        self.l10n.as_deref().is_some_and(|l10n| l10n.targets.has(field))
    }

    /// Whether any field reads localized.
    pub(crate) fn has_l10n(&self) -> bool {
        self.l10n.is_some()
    }

    /// `field` as [`Self::get_localized`] reads it when that needs no lookup; otherwise the lookup to run.
    pub(crate) fn read_localized(&self, facade: &EntityFacade, field: &str) -> Result<Option<Value>, LocalizedRead> {
        let stored = self.fields.get(field);
        let Some(l10n) = self.l10n.as_deref().filter(|l10n| l10n.targets.has(field)) else {
            return Ok(stored.cloned());
        };
        let locale = field_rw::current_locale(facade);
        let key = (field.to_string(), locale);
        if let Some(known) = lock(&l10n.memo).get(&key) {
            return Ok(known.clone().or_else(|| stored.cloned()));
        }
        let (field, locale) = key;
        let fields = l10n
            .targets
            .read_columns(&field)
            .filter_map(|name| self.fields.get(name).map(|value| (name.to_string(), value.clone())))
            .collect();
        Err(LocalizedRead {
            l10n: l10n.clone(),
            field,
            locale,
            fields,
        })
    }

    /// Java's `getMap`: every field held, the localized ones in the user's locale.
    pub fn localized_fields(&self, facade: &EntityFacade) -> BTreeMap<String, Value> {
        let mut out = self.fields.clone();
        let Some(l10n) = self.l10n.as_deref() else {
            return out;
        };
        for column in l10n.targets.columns() {
            if let Some(value) = self.get_localized(facade, column) {
                out.insert(column.to_string(), value);
            }
        }
        out
    }

    pub fn set(&mut self, field: impl Into<String>, value: Value) {
        let field = field.into();
        if let Some(l10n) = self.l10n.as_deref_mut() {
            // A copy of its own, so a clone that shares the reads does not see this field's.
            let kept: HashMap<_, _> = lock(&l10n.memo)
                .iter()
                .filter(|((name, _), _)| *name != field)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            l10n.memo = Arc::new(Mutex::new(kept));
        }
        self.fields.insert(field, value);
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
        if let Some(l10n) = self.l10n.as_deref_mut() {
            l10n.memo = Arc::new(Mutex::new(HashMap::new()));
        }
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
    pub fn into_values(self) -> Vec<EntityValue> {
        self.values
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
        // As Java, fill primary-key defaults first.
        let mut value = value.clone();
        let defaults = self.apply_field_defaults(&value.entity_name, &value.fields)?;
        let keys = self.pk_map(&value.entity_name, &defaults)?;
        value.fields.extend(keys.clone());
        // Rust then tries one UPDATE by primary key, for a row that is probably there: an `update` hit when it
        // updates the row, the Rust-only `update-probe` when it matches none. Otherwise as Java: refresh a clone
        // before choosing the write.
        if self.update_if_exists(&value.entity_name, &value.fields)? {
            return Ok(());
        }
        let exists = {
            // Java refreshes a throwaway clone without accessing its localized fields.
            let _raw = super::field_rw::RawReadGuard::enter();
            self.refresh_by_pk(&value.entity_name, &keys)?.is_some()
        };
        if exists {
            self.update_value(&value)
        } else {
            self.create_value(&value)
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
        let with_defaults = self.apply_field_defaults(&value.entity_name, &value.fields)?;
        let plain = self.normalize_values(&value.entity_name, &with_defaults, false)?;
        self.require_pk(&value.entity_name, &plain)?;
        fk::enforce(self, &value.entity_name, &plain, stack)?;
        self.reject_view(&value.entity_name, "create")?;
        self.finish_write(&value.entity_name, "create", plain, None, false, false)
            .map(|_| ())
    }

    /// Fill the fields an entity definition gives a `default` (a quoted literal, a number, the name of another
    /// field of the same value, `ec.user.nowTimestamp` or `ec.user.userId`) when a create leaves them out or null, as
    /// Moqui does on create.
    pub(crate) fn apply_field_defaults(
        &self,
        entity_name: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        self.with_entity(entity_name, |entity, _, _| {
            let mut out = values.clone();
            for field in &entity.fields {
                let Some(default) = field.default_value.as_deref().map(str::trim).filter(|d| !d.is_empty()) else {
                    continue;
                };
                if out.get(&field.name).is_some_and(|v| !v.is_null()) {
                    continue;
                }
                let bytes = default.as_bytes();
                let quoted =
                    bytes.len() >= 2 && (bytes[0] == b'\'' || bytes[0] == b'"') && bytes[bytes.len() - 1] == bytes[0];
                let value = if quoted {
                    Some(Value::String(default[1..default.len() - 1].to_string()))
                } else if default.parse::<f64>().is_ok() {
                    Some(Value::String(default.to_string()))
                } else if default == "ec.user.nowTimestamp" {
                    // the execution context's effective time, as the Groovy default expression read it
                    Some(crate::user::now_effective_value())
                } else if default == "ec.user.userId" {
                    crate::user::current_user_id().ok().flatten().map(Value::string)
                } else {
                    out.get(default).filter(|v| !v.is_null()).cloned()
                };
                if let Some(value) = value {
                    out.insert(field.name.clone(), value);
                }
            }
            Ok(out)
        })
    }

    fn write_update(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<(), Error> {
        let plain = self.normalize_values(entity_name, values, true)?;
        let rules = load_feed::match_rules_with(self, entity_name, "update");
        self.reject_view(entity_name, "update")?;
        let has_columns = self.with_entity(entity_name, |entity, _, _| {
            Ok(!present_columns(entity, &plain, true, true).is_empty())
        })?;
        if !has_columns {
            return Ok(());
        }
        self.check_create_only_fields(entity_name, &plain)?;
        self.finish_write(entity_name, "update", plain, Some(rules), true, true)
            .map(|_| ())
    }

    /// As Java's update: a create-only field (every field of a `create-only` entity) that would change is an error.
    /// The row is read only when the entity has such a field. Fields that keep their stored value pass.
    fn check_create_only_fields(&self, entity_name: &str, plain: &BTreeMap<String, Value>) -> Result<(), Error> {
        let (full_name, create_only) = self.with_entity(entity_name, |entity, _, _| {
            let names: Vec<String> = entity
                .fields
                .iter()
                .filter(|field| {
                    // The stamp is set on every write, as Java's update which skips it in this check.
                    field.create_only
                        && !field.is_pk
                        && field.name != "lastUpdatedStamp"
                        && plain.contains_key(&field.name)
                })
                .map(|field| field.name.clone())
                .collect();
            Ok((entity.full_name.clone(), names))
        })?;
        if create_only.is_empty() {
            return Ok(());
        }
        let keys = self.pk_map(entity_name, plain)?;
        let stored = {
            let _raw = super::field_rw::RawReadGuard::enter();
            self.find_by_pk(entity_name, &keys)?
        };
        let changed: Vec<String> = create_only
            .into_iter()
            .filter(|name| {
                let new_value = plain.get(name).unwrap_or(&Value::Null);
                let old_value = stored.as_ref().and_then(|row| row.get(name)).unwrap_or(&Value::Null);
                old_value != new_value
                    && moqui_lib::object::to_plain_string(old_value) != moqui_lib::object::to_plain_string(new_value)
            })
            .collect();
        if changed.is_empty() {
            return Ok(());
        }
        Err(Error::new(format!(
            "Cannot update create-only (immutable) fields [{}] on entity {full_name}",
            changed.join(", ")
        )))
    }

    /// The first step of [`Self::create_or_update_value`]: one `UPDATE` by primary key, for a row that is probably
    /// there. Returns `true` when it updated a row. Returns `false` when nothing was written and the caller must
    /// take the find-then-write path: the row is absent, or this write is one where an update without a prior read
    /// could differ from the find-then-write path.
    ///
    /// Those are: an ECA rule matches `update` (it must not run for a row that turns out to be new), an
    /// audit-logged field applies to `update` (the old value is read and logged), the dialect does not report
    /// matched rows (only Postgres does; MySQL counts changed rows), a transaction cache or simulation is in use
    /// (a write there is held, not run), authorization is on (an update checks `update`, a new row checks
    /// `create`), or the update has no column to set. Foreign key checks and field defaults belong to create only,
    /// and the path that follows a miss is the unchanged one, so they are not skipped. A probe that updates no row
    /// leaves no feed touch, cache clear, or ECA run behind.
    fn update_if_exists(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<bool, Error> {
        let eligible = self.with_entity(entity_name, |entity, dialect, _| {
            Ok(dialect.name == "postgres"
                && !entity.is_view
                && !entity.fields.iter().any(|field| field.create_only && !field.is_pk)
                && !entity.fields.iter().any(|field| {
                    field
                        .enable_audit_log
                        .as_deref()
                        .is_some_and(|flag| matches!(flag, "true" | "update"))
                })
                && (!crate::user::context_entered()
                    || entity.authorize_skip.trim() == "true"
                    || crate::context::ExecutionContext::with_current(|ctx| ctx.authz_disabled()).unwrap_or(false)))
        })?;
        if !eligible || self.sim_active() || self.transactions_for(entity_name)?.transaction_cache_active() {
            return Ok(false);
        }
        let plain = self.normalize_values(entity_name, values, true)?;
        let has_columns = self.with_entity(entity_name, |entity, _, _| {
            Ok(!present_columns(entity, &plain, true, true).is_empty())
        })?;
        if !has_columns {
            return Ok(false);
        }
        let rules = load_feed::match_rules_with(self, entity_name, "update");
        if !rules.is_empty() {
            return Ok(false);
        }
        Ok(self.finish_write(entity_name, "update-probe", plain, Some(rules), true, true)? > 0)
    }

    fn write_delete(&self, entity_name: &str, values: &BTreeMap<String, Value>) -> Result<(), Error> {
        let plain = self.normalize_values(entity_name, values, false)?;
        self.reject_view(entity_name, "delete")?;
        self.with_entity(entity_name, |entity, _, _| {
            if entity.create_only {
                return Err(Error::new(format!(
                    "Entity {} is create-only (immutable), cannot be deleted.",
                    entity.full_name
                )));
            }
            Ok(())
        })?;
        self.finish_write(entity_name, "delete", plain, None, false, false)
            .map(|_| ())
    }

    /// Insert, or update on a primary-key conflict. Values are sealed the same way as a create.
    pub(crate) fn upsert_record(&self, entity_name: &str, fields: &BTreeMap<String, Value>) -> Result<(), Error> {
        let plain = self.normalize_values(entity_name, fields, false)?;
        self.require_pk(entity_name, &plain)?;
        let mut stack = HashSet::new();
        fk::enforce(self, entity_name, &plain, &mut stack)?;
        self.reject_view(entity_name, "create")?;
        self.finish_write(entity_name, "upsert", plain, None, false, false)
            .map(|_| ())
    }

    /// Run the write, or leave it with the cache when the gate handles it. Values are plaintext. `rules` are the
    /// ECA rules a caller already matched for this write; `None` matches them here.
    fn finish_write(
        &self,
        entity_name: &str,
        operation: &str,
        plain: BTreeMap<String, Value>,
        rules: Option<Vec<EcaRule>>,
        include_null: bool,
        skip_pk: bool,
    ) -> Result<u64, Error> {
        // `update-probe` is an update that leaves no trace when it matches no row; see `update_if_exists`. It is
        // authorized as an update. Its hit is an `update` when it updated a row, and the Rust-only `update-probe`
        // when it matched none.
        let probe = operation == "update-probe";
        let operation = if probe { "update" } else { operation };
        // Declared first, so it drops last: the authorization cache is invalidated after the write.
        let _note = WriteNote(entity_name);
        let sub_type = if probe {
            RustOnlyEntityHits::UPDATE_PROBE
        } else if operation == "upsert" {
            "create"
        } else {
            operation
        };
        let _artifact = EntityArtifact::push(self, entity_name, operation, sub_type)?;
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
        let rules = match rules {
            Some(rules) => rules,
            None => load_feed::match_rules_with(self, entity_name, seal_operation),
        };
        let eca = crate::user::context_entered();
        let mut plain = plain;
        if eca {
            let before = plain.clone();
            crate::service::run_matched_eca(self, entity_name, seal_operation, true, &rules, &mut plain, None)?;
            // A run-before rule may have set fields (`entityValue` or `set-results`): convert them as a write would.
            if plain != before {
                plain = self.normalize_values(entity_name, &plain, operation == "update")?;
            }
        }
        let cache_values = plain.clone();
        let mut affected = 1;
        if cache::gate_write(self, entity_name, operation, &plain)? == WriteGate::Handled {
            load_feed::note_touch(self, entity_name, seal_operation, Some(&cache_values), None);
        } else {
            let mut after_values = eca.then(|| plain.clone());
            // An after rule with `get-original-value` on an update or delete gets the row as it was before the write.
            let pre_row = if eca
                && (operation == "update" || operation == "delete")
                && rules.iter().any(|rule| !rule.run_before && rule.get_original_value)
            {
                let keys = self.pk_map(entity_name, &plain)?;
                self.find_by_pk(entity_name, &keys)?
            } else {
                None
            };
            // The data feed feeds an update only when a document field changes, so it needs the row as it was.
            let feed_old = if operation == "update" && !probe && load_feed::feed_wants_old(self, entity_name) {
                match &pre_row {
                    Some(row) => Some(row.fields().clone()),
                    None => {
                        let keys = self.pk_map(entity_name, &plain)?;
                        let _raw = super::field_rw::RawReadGuard::enter();
                        self.find_by_pk(entity_name, &keys)?.map(|row| row.fields().clone())
                    }
                }
            } else {
                None
            };
            // A run-after rule on a delete sees the row as it was: read it now, before the row is gone.
            if let Some(values) = after_values.as_mut().filter(|_| operation == "delete") {
                if rules.iter().any(|rule| !rule.run_before && rule.get_entire_entity) {
                    if let Some(row) = self
                        .refresh_fields(entity_name, values)
                        .ok()
                        .filter(|row| !row.is_empty())
                    {
                        for (name, value) in row {
                            values.entry(name).or_insert(value);
                        }
                    }
                }
            }
            // Java's `checkTableRuntime` before each write, when the group adds missing tables at runtime.
            self.check_table_runtime(entity_name);
            let prepared = self.seal_values(entity_name, seal_operation, plain)?;
            let (sql, params) = self.with_group(entity_name, |entity, dialect, _, _| {
                let columns = present_columns(entity, &prepared, include_null, skip_pk);
                let (sql, params) = if operation == "delete" {
                    let condition = pk_condition(entity, &prepared)?;
                    let statement = delete(entity, dialect, Some(&condition))?;
                    (statement.sql, statement.params)
                } else if operation == "update" {
                    let condition = pk_condition(entity, &prepared)?;
                    let statement = update(entity, dialect, &columns, Some(&condition))?;
                    let mut params = bind_columns(&columns, &prepared);
                    params.extend(statement.params);
                    (statement.sql, params)
                } else if operation == "upsert" {
                    let sql = insert_or_update(entity, dialect, &columns)?;
                    (sql, bind_columns(&columns, &prepared))
                } else {
                    let sql = insert(entity, dialect, &columns)?;
                    (sql, bind_columns(&columns, &prepared))
                };
                Ok((sql, params))
            })?;
            affected = execute_tx(&tx, &sql, &params)?;
            if probe {
                if affected == 0 {
                    return Ok(0);
                }
                crate::context::ExecutionContext::with_current(|ctx| ctx.set_hit_detail(Some("update"), None));
            }
            load_feed::note_touch(self, entity_name, seal_operation, Some(&cache_values), feed_old);
            if let Some(mut values) = after_values {
                crate::service::run_matched_eca(
                    self,
                    entity_name,
                    seal_operation,
                    false,
                    &rules,
                    &mut values,
                    pre_row.as_ref(),
                )?;
            }
        }
        // As Java's `EntityCache`, a cache clear or cluster publish that fails is logged; the write already ran.
        if let Err(err) = cache::on_write(self, entity_name, operation, &cache_values) {
            tracing::error!(error = %err, "Could not clear or publish the cache for {operation} of {entity_name}");
        }
        after_statement(self, &tx, true);
        Ok(affected)
    }

    pub(crate) fn refresh_fields(
        &self,
        entity_name: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
        let keys = self.pk_map(entity_name, values)?;
        self.refresh_by_pk(entity_name, &keys)?
            .map(|value| value.fields)
            .ok_or_else(|| Error::new(format!("refresh found no {entity_name}")))
    }

    fn refresh_by_pk(&self, entity_name: &str, keys: &BTreeMap<String, Value>) -> Result<Option<EntityValue>, Error> {
        // Refresh bypasses the shared entity cache but still sees transaction-held rows.
        let mut find = self.find(entity_name)?.use_cache(false);
        for (field, value) in keys {
            find = find.eq(field, value.clone());
        }
        find.one_internal("refresh")
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

    pub(crate) fn pk_map(
        &self,
        entity_name: &str,
        values: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>, Error> {
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

    pub(crate) fn normalize_values(
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

thread_local! {
    static RUST_ONLY_SUB_TYPE: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// While held, entity operations on this thread are counted as artifact hits under `sub_type` instead of Java's find
/// method or write operation. For reads Java does not make: FK existence checks (Java leaves those to the database)
/// and the load report counts. They are still pushed, authorized and counted, but in bins of their own, so a
/// comparison with Java can set them apart. The previous sub type is restored on drop.
#[must_use = "the sub type is restored when the guard drops"]
pub struct RustOnlyEntityHits(Option<&'static str>);

impl RustOnlyEntityHits {
    /// FK existence reads before a write.
    pub const FK_CHECK: &'static str = "fk-check";
    /// The record counts the startup load reports.
    pub const LOAD_REPORT: &'static str = "load-report";
    /// The UPDATE create-or-update tries before it reads the row, when it matched no row (Java reads first).
    pub const UPDATE_PROBE: &'static str = "update-probe";

    pub fn enter(sub_type: &'static str) -> Self {
        Self(RUST_ONLY_SUB_TYPE.with(|current| current.replace(Some(sub_type))))
    }
}

impl Drop for RustOnlyEntityHits {
    fn drop(&mut self) {
        RUST_ONLY_SUB_TYPE.with(|current| current.set(self.0));
    }
}

fn rust_only_sub_type() -> Option<&'static str> {
    RUST_ONLY_SUB_TYPE.with(std::cell::Cell::get)
}

/// Whether `sub_type` is one [`RustOnlyEntityHits`] counts under.
pub(crate) fn is_rust_only_sub_type(sub_type: Option<&str>) -> bool {
    matches!(
        sub_type,
        Some(RustOnlyEntityHits::FK_CHECK | RustOnlyEntityHits::LOAD_REPORT | RustOnlyEntityHits::UPDATE_PROBE)
    )
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
    /// `sub_type` is the hit sub-type, Java's find method or write operation: `list`, `count`, `create`.
    pub(crate) fn push(
        facade: &EntityFacade,
        entity_name: &str,
        operation: &str,
        sub_type: &str,
    ) -> Result<Self, Error> {
        if !crate::user::context_entered() {
            return Ok(Self { pushed: false });
        }
        let action = if operation == "upsert" { "create" } else { operation };
        let skip = facade
            .with_entity(entity_name, |entity, _, _| Ok(entity.authorize_skip.clone()))
            .unwrap_or_default();
        let requires_authz = !skips_authz(&skip, action);
        let outcome = crate::context::ExecutionContext::with_current(|ctx| {
            ctx.push_checked("AT_ENTITY", entity_name, action, requires_authz, false, None)?;
            ctx.set_hit_detail(Some(rust_only_sub_type().unwrap_or(sub_type)), None);
            Ok::<(), crate::context::Error>(())
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
