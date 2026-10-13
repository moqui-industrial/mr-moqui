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

//! Synchronous service call.

use std::collections::{BTreeMap, BTreeSet};

use moqui_db::model::{EntityDefinition, KeyMap};
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, Value};

use super::define::{Authenticate, ServiceDef, ServiceKind};
use super::{semaphore, validate};
use crate::context::Error;
use crate::entity::{EntityFacade, EntityValue};

/// Run `name` and return its out parameters. `create-entity` is `create#entity`.
pub fn sync(name: &str, parameters: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, Error> {
    let name = canonical(name);
    let facade = current_facade();
    let def = resolve(&name, facade.as_ref())?;
    if def.authenticate != Authenticate::Off {
        crate::user::authorize("AT_SERVICE", &def.name, auth_action(&def))?;
    }
    let cleaned = validate::clean(&def, parameters, facade.as_ref())?;
    let facade = facade.ok_or_else(|| Error::new("no entity facade is installed"))?;
    let guard = semaphore::take(&def, &cleaned, &facade)?;
    let entity_name = if def.kind == ServiceKind::EntityAuto && !def.noun.is_empty() {
        def.noun.clone()
    } else {
        ServiceDef::semaphore_entity().to_string()
    };
    let tx = facade.transactions_for(&entity_name)?;
    let produced = tx.run(def.transaction, def.timeout, || execute(&def, &cleaned, &facade))?;
    drop(guard);
    Ok(project(&def, produced))
}

pub fn call(name: &str) -> Result<(), Error> {
    sync(name, &BTreeMap::new()).map(|_| ())
}

fn canonical(name: &str) -> String {
    if name.contains('#') {
        return name.to_string();
    }
    for verb in ["create", "update", "store", "delete"] {
        let prefix = format!("{verb}-");
        if let Some(rest) = name.strip_prefix(&prefix) {
            return format!("{verb}#{rest}");
        }
    }
    name.to_string()
}

fn resolve(name: &str, facade: Option<&EntityFacade>) -> Result<ServiceDef, Error> {
    if let Some(def) = super::define::find(name) {
        return Ok(def);
    }
    let (verb, noun) = split_name(name);
    if matches!(verb.as_str(), "create" | "update" | "store" | "delete") && !noun.is_empty() {
        if let Some(facade) = facade {
            if facade.with_entity(&noun, |_, _, _| Ok(())).is_ok() {
                return Ok(ServiceDef::implicit(name, &verb, &noun));
            }
        }
    }
    Err(Error::new(format!("no service named {name}")))
}

fn split_name(name: &str) -> (String, String) {
    match name.split_once('#') {
        Some((verb, noun)) => (verb.to_string(), noun.to_string()),
        None => (name.to_string(), String::new()),
    }
}

fn auth_action(def: &ServiceDef) -> &str {
    match def.authenticate {
        Authenticate::AnonymousView => "view",
        Authenticate::AnonymousAll => "all",
        _ => def.authz_action.as_deref().unwrap_or_else(|| verb_action(&def.verb)),
    }
}

fn verb_action(verb: &str) -> &'static str {
    match verb {
        "create" => "create",
        "update" | "store" => "update",
        "delete" => "delete",
        "view" | "find" => "view",
        _ => "all",
    }
}

fn execute(
    def: &ServiceDef,
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    match def.kind {
        ServiceKind::Mail => Err(LibError::new(format!("mail is not configured ({})", def.location))),
        ServiceKind::Interface => Err(LibError::new(format!("interface {} has no runner", def.name))),
        ServiceKind::Script => Err(LibError::new("script runner is not running yet")),
        ServiceKind::RemoteRest => Err(LibError::new(format!("service {} has no REST dispatcher", def.name))),
        ServiceKind::EntityAuto => run_entity(facade, &def.verb, &def.noun, parameters, 0),
        ServiceKind::Inline => Err(LibError::new("actions not run in this slice")),
    }
}

fn project(def: &ServiceDef, values: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let names: Vec<&str> = def
        .out_params
        .iter()
        .filter(|param| !param.name.is_empty() && param.required != super::define::Required::Disabled)
        .map(|param| param.name.as_str())
        .collect();
    if names.is_empty() {
        return values;
    }
    values
        .into_iter()
        .filter(|(name, _)| names.contains(&name.as_str()))
        .collect()
}

fn run_entity(
    facade: &EntityFacade,
    verb: &str,
    noun: &str,
    parameters: &BTreeMap<String, Value>,
    depth: u32,
) -> Result<BTreeMap<String, Value>, LibError> {
    if depth > 8 {
        return Err(LibError::new("nested entity-auto is too deep"));
    }
    let pk_names = facade.with_entity(noun, |entity, _, _| {
        Ok(entity
            .pk_field_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>())
    })?;
    let mut record = facade.make_value(noun)?;
    for (key, value) in parameters {
        if reserved(key) || matches!(value, Value::Map(_) | Value::List(_)) {
            continue;
        }
        record.set(key, value.clone());
    }
    match verb {
        "delete" => record.delete(facade)?,
        "update" => record.update(facade)?,
        "create" => {
            assign_sequence(facade, &mut record, &pk_names)?;
            record.create(facade)?;
        }
        "store" => {
            if pk_missing(&record, &pk_names) {
                assign_sequence(facade, &mut record, &pk_names)?;
                record.create(facade)?;
            } else if row_found(facade, noun, &record, &pk_names)? {
                record.update(facade)?;
            } else {
                record.create(facade)?;
            }
        }
        other => return Err(LibError::new(format!("entity-auto verb {other} is not supported"))),
    }
    nest(facade, verb, noun, parameters, &record, depth)?;
    let mut out = BTreeMap::new();
    for pk in &pk_names {
        if let Some(value) = record.get(pk) {
            out.insert(pk.clone(), value.clone());
        }
    }
    Ok(out)
}

fn assign_sequence(facade: &EntityFacade, record: &mut EntityValue, pk_names: &[String]) -> Result<(), LibError> {
    if pk_names.is_empty() || !pk_missing(record, pk_names) {
        return Ok(());
    }
    if pk_names.len() == 1 {
        record.set_sequenced_id_primary(facade)?;
        return Ok(());
    }
    if let Some(last) = pk_names.last() {
        if record.get(last).is_none_or(|value| value.is_null() || is_empty(value)) {
            record.set_sequenced_id_secondary(facade)?;
        }
    }
    Ok(())
}

fn pk_missing(record: &EntityValue, pk_names: &[String]) -> bool {
    pk_names
        .iter()
        .any(|name| record.get(name).is_none_or(|value| value.is_null() || is_empty(value)))
}

fn row_found(facade: &EntityFacade, noun: &str, record: &EntityValue, pk_names: &[String]) -> Result<bool, LibError> {
    let mut find = facade.find(noun)?;
    for name in pk_names {
        let value = record.get(name).cloned().unwrap_or(Value::Null);
        find = find.eq(name, value);
    }
    Ok(find.one()?.is_some())
}

fn nest(
    facade: &EntityFacade,
    verb: &str,
    noun: &str,
    parameters: &BTreeMap<String, Value>,
    record: &EntityValue,
    depth: u32,
) -> Result<(), LibError> {
    let child_verb = if verb == "delete" { "delete" } else { "store" };
    for (key, value) in parameters {
        if reserved(key) {
            continue;
        }
        let children = nested_maps(value);
        if children.is_empty() {
            continue;
        }
        let Some(link) = link_for(facade, noun, key)? else {
            continue;
        };
        for mut child in children {
            fill_link(&mut child, parameters, record, &link);
            run_entity(facade, child_verb, &link.entity, &child, depth + 1)?;
        }
    }
    Ok(())
}

fn nested_maps(value: &Value) -> Vec<BTreeMap<String, Value>> {
    match value {
        Value::Map(entries) => vec![copy_map(entries)],
        Value::List(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::Map(entries) => Some(copy_map(entries)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn copy_map<'a, I, K>(entries: I) -> BTreeMap<String, Value>
where
    I: IntoIterator<Item = (&'a K, &'a Value)>,
    K: AsRef<str> + 'a,
{
    entries
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_string(), value.clone()))
        .collect()
}

struct Link {
    entity: String,
    pairs: Vec<(String, String)>,
    constants: Vec<(String, String)>,
    parent_pks: Vec<String>,
    child_fields: BTreeSet<String>,
}

struct FieldSnap {
    name: String,
    is_pk: bool,
}

fn link_for(facade: &EntityFacade, parent: &str, key: &str) -> Result<Option<Link>, LibError> {
    facade.with_entity(parent, |entity, _, model| {
        let parent_pks = entity
            .pk_field_names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if let Some(relationship) = entity.relationship(key) {
            let key_is_entity = model.entity(key).is_ok();
            if relationship.mutable || key_is_entity {
                let target = model.entity(&relationship.related_entity_name)?;
                let fields = snap_fields(target);
                let pairs = expand_key_map(&relationship.relationship_type, &relationship.key_map, &fields);
                let constants = relationship
                    .key_values
                    .iter()
                    .map(|entry| (entry.related.clone(), entry.value.clone()))
                    .collect();
                return Ok(Some(owned_link(
                    target.full_name.clone(),
                    pairs,
                    constants,
                    parent_pks,
                    &fields,
                )));
            }
        }
        if let Ok(target) = model.entity(key) {
            let fields = snap_fields(target);
            return Ok(Some(owned_link(
                target.full_name.clone(),
                Vec::new(),
                Vec::new(),
                parent_pks,
                &fields,
            )));
        }
        Ok(None)
    })
}

fn owned_link(
    entity: String,
    pairs: Vec<(String, String)>,
    constants: Vec<(String, String)>,
    parent_pks: Vec<String>,
    fields: &[FieldSnap],
) -> Link {
    Link {
        entity,
        pairs,
        constants,
        parent_pks,
        child_fields: fields.iter().map(|field| field.name.clone()).collect(),
    }
}

fn snap_fields(entity: &EntityDefinition) -> Vec<FieldSnap> {
    entity
        .fields
        .iter()
        .map(|field| FieldSnap {
            name: field.name.clone(),
            is_pk: field.is_pk,
        })
        .collect()
}

fn expand_key_map(relationship_type: &str, key_map: &[KeyMap], fields: &[FieldSnap]) -> Vec<(String, String)> {
    let pks: Vec<String> = fields
        .iter()
        .filter(|field| field.is_pk)
        .map(|field| field.name.clone())
        .collect();
    let one = relationship_type.starts_with("one");
    if key_map.is_empty() && one {
        return pks.into_iter().map(|pk| (pk.clone(), pk)).collect();
    }
    if key_map.len() == 1 {
        let map = &key_map[0];
        let related = match map.related.as_deref().filter(|value| !value.is_empty()) {
            Some(related) => related.to_string(),
            None if one && pks.len() == 1 => pks[0].clone(),
            None => map.field_name.clone(),
        };
        return vec![(map.field_name.clone(), related)];
    }
    let names: BTreeSet<&str> = fields.iter().map(|field| field.name.as_str()).collect();
    key_map
        .iter()
        .map(|map| {
            let mut related = map
                .related
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| map.field_name.clone());
            if one && pks.len() == 1 && !names.contains(related.as_str()) {
                related = pks[0].clone();
            }
            (map.field_name.clone(), related)
        })
        .collect()
}

fn fill_link(child: &mut BTreeMap<String, Value>, parent: &BTreeMap<String, Value>, record: &EntityValue, link: &Link) {
    for (source, target) in &link.pairs {
        if !blank(child.get(target)) {
            continue;
        }
        let value = parent.get(source).cloned().or_else(|| record.get(source).cloned());
        if let Some(value) = value {
            if !is_empty(&value) {
                child.insert(target.clone(), value);
            }
        }
    }
    for (field, value) in &link.constants {
        if blank(child.get(field)) {
            child.insert(field.clone(), Value::string(value));
        }
    }
    for pk in &link.parent_pks {
        if !link.child_fields.contains(pk) || !blank(child.get(pk)) {
            continue;
        }
        if let Some(value) = record.get(pk).filter(|value| !is_empty(value)).cloned() {
            child.insert(pk.clone(), value);
        }
    }
}

fn blank(value: Option<&Value>) -> bool {
    value.is_none_or(is_empty)
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "ec" | "_entity" | "authUsername" | "authPassword" | "authUserAccount"
    )
}

fn current_facade() -> Option<EntityFacade> {
    crate::context::ExecutionContext::with_current(|ctx| ctx.factory().entity()).flatten()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use moqui_lib::object::{basic_convert, Value};

    use super::sync;
    use crate::context::ExecutionContextFactory;

    #[test]
    fn entity_auto_store_inserts_then_updates() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        let tape = Arc::new(Mutex::new(Vec::new()));
        let facade = mem::open(Arc::clone(&tape));
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        let mut params = BTreeMap::new();
        params.insert("testId".into(), Value::string("T1"));
        params.insert("testMedium".into(), Value::string("first"));
        let created = sync("store-moqui.test.TestEntity", &params).unwrap();
        assert_eq!(created.get("testId").and_then(Value::as_str), Some("T1"));
        params.insert("testMedium".into(), Value::string("second"));
        sync("store#moqui.test.TestEntity", &params).unwrap();
        let row = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("T1"))
            .one()
            .unwrap()
            .expect("stored row");
        assert_eq!(row.get("testMedium").and_then(Value::as_str), Some("second"));
        let sql = tape.lock().unwrap_or_else(|err| err.into_inner()).join("\n");
        assert!(sql.contains("INSERT") && sql.contains("TestEntity"), "{sql}");
        assert!(sql.contains("UPDATE") && sql.contains("TestEntity"), "{sql}");
        assert!(!sql.contains("ON CONFLICT"), "{sql}");
    }

    #[test]
    fn missing_required_parameter_is_named() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        let path = write_fixture(
            "required",
            r#"<services>
                <service verb="need" noun="Value">
                    <in-parameters><parameter name="amount" required="true"/></in-parameters>
                </service>
            </services>"#,
        );
        crate::service::define::lint(std::slice::from_ref(&path)).unwrap();
        let err = sync("need#Value", &BTreeMap::new()).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("amount"), "{text}");
        assert!(text.contains("required"), "{text}");
    }

    #[test]
    fn held_fail_semaphore_errors() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        let path = write_fixture(
            "semaphore",
            r#"<services>
                <service verb="lock" noun="Row" semaphore="fail">
                    <in-parameters><parameter name="note"/></in-parameters>
                </service>
            </services>"#,
        );
        crate::service::define::lint(std::slice::from_ref(&path)).unwrap();
        let tape = Arc::new(Mutex::new(Vec::new()));
        let facade = mem::open(tape);
        let ctx = ExecutionContextFactory::test_context();
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        let mut row = facade
            .make_value("moqui.service.semaphore.ServiceParameterSemaphore")
            .unwrap();
        row.set("serviceName", Value::string("lock#Row"));
        row.set("parameterValue", Value::string("_NA_"));
        row.set("lockThread", Value::string("other"));
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        row.set("lockTime", basic_convert(&Value::I64(millis), "Timestamp").value);
        row.create(&facade).unwrap();
        let err = sync("lock#Row", &BTreeMap::new()).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("semaphore"), "{text}");
        assert!(!text.contains("actions not run"), "{text}");
    }

    fn write_fixture(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("moqui-svc-{name}-{}.xml", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    mod mem {
        use std::collections::{BTreeMap, BTreeSet};
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use moqui_lib::error::Error;
        use moqui_lib::object::{compare_obj, Value};

        use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
        use crate::entity::{EntityFacade, EntityGroup};
        use crate::transaction::{PoolConfig, TransactionManager};

        type Row = BTreeMap<String, Value>;
        type Tables = BTreeMap<String, Vec<Row>>;

        pub(super) fn open(tape: Arc<Mutex<Vec<String>>>) -> EntityFacade {
            let driver: Arc<dyn DbDriver> = Arc::new(MemDriver {
                store: Arc::new(Mutex::new(BTreeMap::new())),
                tape: Arc::clone(&tape),
            });
            let transactions = TransactionManager::new(
                driver,
                PoolConfig {
                    min: 0,
                    max: 4,
                    reserve: 2,
                    acquire_timeout: Duration::from_secs(5),
                },
            )
            .expect("memory pool");
            let facade = EntityFacade::open(
                &framework_dir(),
                "transactional",
                vec![EntityGroup::new("transactional", "postgres", transactions)],
            )
            .expect("entity facade");
            facade.set_disable_audit_log(true);
            facade.set_disable_entity_eca(true);
            facade.set_disable_fk_create(true);
            facade.set_disable_data_feed(true);
            facade
        }

        fn framework_dir() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
        }

        struct MemDriver {
            store: Arc<Mutex<Tables>>,
            tape: Arc<Mutex<Vec<String>>>,
        }

        struct MemConn {
            store: Arc<Mutex<Tables>>,
            tape: Arc<Mutex<Vec<String>>>,
            local: Option<Tables>,
            dirty: BTreeSet<String>,
        }

        struct NoCancel;

        impl DbCancel for NoCancel {
            fn cancel(&self) -> Result<(), Error> {
                Ok(())
            }
        }

        impl DbDriver for MemDriver {
            fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
                Ok(Box::new(MemConn {
                    store: Arc::clone(&self.store),
                    tape: Arc::clone(&self.tape),
                    local: None,
                    dirty: BTreeSet::new(),
                }))
            }
        }

        impl DbConnection for MemConn {
            fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
                Ok(self.apply(sql, params)?.0)
            }

            fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
                Ok(self.apply(sql, params)?.1)
            }

            fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
                self.query(sql, params)
            }

            fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
                Ok(())
            }

            fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
                Ok(None)
            }

            fn close_stream(&mut self) -> Result<(), Error> {
                Ok(())
            }

            fn stream_open(&self) -> bool {
                false
            }

            fn begin(&mut self) -> Result<(), Error> {
                if self.local.is_none() {
                    let committed = self.store.lock().unwrap_or_else(|err| err.into_inner()).clone();
                    self.local = Some(committed);
                    self.dirty.clear();
                }
                Ok(())
            }

            fn commit(&mut self) -> Result<(), Error> {
                let Some(local) = self.local.take() else {
                    return Ok(());
                };
                let dirty = std::mem::take(&mut self.dirty);
                let mut committed = self.store.lock().unwrap_or_else(|err| err.into_inner());
                for table in dirty {
                    let rows = local.get(&table).cloned().unwrap_or_default();
                    committed.insert(table, rows);
                }
                Ok(())
            }

            fn rollback(&mut self) -> Result<(), Error> {
                self.local = None;
                self.dirty.clear();
                Ok(())
            }

            fn savepoint(&mut self, _name: &str) -> Result<(), Error> {
                Ok(())
            }

            fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), Error> {
                Ok(())
            }

            fn release_savepoint(&mut self, _name: &str) -> Result<(), Error> {
                Ok(())
            }

            fn canceller(&self) -> Box<dyn DbCancel> {
                Box::new(NoCancel)
            }
        }

        impl MemConn {
            fn apply(&mut self, sql: &str, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                let sql = sql.trim();
                // The pool checks out a connection with `SELECT 1`.
                if !sql.eq_ignore_ascii_case("SELECT 1") {
                    self.tape
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .push(sql.to_string());
                }
                if sql.eq_ignore_ascii_case("SELECT 1") {
                    return Ok((
                        1,
                        vec![DbRow {
                            columns: vec![("1".to_string(), Value::I64(1))],
                        }],
                    ));
                }
                if sql.contains("ON CONFLICT") && sql.contains("SequenceValueItem") {
                    return self.sequence(params);
                }
                let stmt = parse(sql).map_err(|err| Error::new(format!("{err} sql={sql}")))?;
                self.exec(stmt, params)
            }

            fn sequence(&mut self, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                if params.len() < 4 {
                    return Err(Error::new("sequence refill is missing parameters"));
                }
                let name = params[0].clone();
                let first_end = as_i64(&params[1])?;
                let increment = as_i64(&params[2])?;
                let hint_end = as_i64(&params[3])?;
                let mut rows = self.read_table("SequenceValueItem");
                let end = if let Some(row) = rows.iter_mut().find(|row| row.get("seqName") == Some(&name)) {
                    let old = row.get("seqNum").and_then(|value| as_i64(value).ok()).unwrap_or(0);
                    let new_end = old.saturating_add(increment).max(hint_end);
                    row.insert("seqNum".to_string(), Value::I64(new_end));
                    new_end
                } else {
                    let mut row = BTreeMap::new();
                    row.insert("seqName".to_string(), name);
                    row.insert("seqNum".to_string(), Value::I64(first_end));
                    rows.push(row);
                    first_end
                };
                self.put_table("SequenceValueItem", rows);
                Ok((
                    1,
                    vec![DbRow {
                        columns: vec![("seqNum".to_string(), Value::I64(end))],
                    }],
                ))
            }

            fn exec(&mut self, stmt: Stmt, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                match stmt {
                    Stmt::Insert { table, columns } => {
                        let mut row = BTreeMap::new();
                        for (index, column) in columns.iter().enumerate() {
                            let value = params
                                .get(index)
                                .cloned()
                                .ok_or_else(|| Error::new("insert is missing a parameter"))?;
                            row.insert(column.clone(), value);
                        }
                        let mut rows = self.read_table(&table);
                        rows.push(row);
                        self.put_table(&table, rows);
                        Ok((1, Vec::new()))
                    }
                    Stmt::Update { table, columns, pred } => {
                        let values = columns
                            .iter()
                            .enumerate()
                            .map(|(index, _)| {
                                params
                                    .get(index)
                                    .cloned()
                                    .ok_or_else(|| Error::new("update is missing a parameter"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let start = columns.len();
                        let mut rows = self.read_table(&table);
                        let mut count = 0_u64;
                        for row in &mut rows {
                            if keep(&pred, row, params, start)? {
                                for (column, value) in columns.iter().zip(&values) {
                                    row.insert(column.clone(), value.clone());
                                }
                                count += 1;
                            }
                        }
                        self.put_table(&table, rows);
                        Ok((count, Vec::new()))
                    }
                    Stmt::Delete { table, pred } => {
                        let rows = self.read_table(&table);
                        let mut kept = Vec::new();
                        let mut count = 0_u64;
                        for row in rows {
                            if keep(&pred, &row, params, 0)? {
                                count += 1;
                            } else {
                                kept.push(row);
                            }
                        }
                        self.put_table(&table, kept);
                        Ok((count, Vec::new()))
                    }
                    Stmt::Select(query) => self.exec_select(&query, params),
                }
            }

            fn exec_select(&mut self, query: &SelectQuery, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                let mut matched = Vec::new();
                for row in self.read_table(&query.table) {
                    if keep(&query.pred, &row, params, 0)? {
                        matched.push(row);
                    }
                }
                if query.count {
                    let n = i64::try_from(matched.len()).unwrap_or(i64::MAX);
                    return Ok((
                        1,
                        vec![DbRow {
                            columns: vec![("count".to_string(), Value::I64(n))],
                        }],
                    ));
                }
                matched.sort_by(|left, right| sort_rows(left, right, &query.order));
                let mut idx = pred_slots(&query.pred);
                let skip = if query.offset {
                    as_i64(take_param(params, &mut idx)?)?
                } else {
                    0
                };
                let take_n = if query.limit {
                    Some(as_i64(take_param(params, &mut idx)?)?)
                } else {
                    None
                };
                let start = usize::try_from(skip.max(0)).unwrap_or(0);
                let window: Vec<Row> = match take_n {
                    Some(limit) => {
                        let len = usize::try_from(limit.max(0)).unwrap_or(0);
                        matched.into_iter().skip(start).take(len).collect()
                    }
                    None => matched.into_iter().skip(start).collect(),
                };
                let projected: Vec<DbRow> = window.iter().map(|row| project(row, &query.columns)).collect();
                let n = u64::try_from(projected.len()).unwrap_or(u64::MAX);
                Ok((n, projected))
            }

            fn read_table(&self, table: &str) -> Vec<Row> {
                if let Some(local) = &self.local {
                    return local.get(table).cloned().unwrap_or_default();
                }
                self.store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .get(table)
                    .cloned()
                    .unwrap_or_default()
            }

            fn put_table(&mut self, table: &str, rows: Vec<Row>) {
                if let Some(local) = self.local.as_mut() {
                    local.insert(table.to_string(), rows);
                    self.dirty.insert(table.to_string());
                    return;
                }
                self.store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .insert(table.to_string(), rows);
            }
        }

        fn keep(pred: &Option<Pred>, row: &Row, params: &[Value], start: usize) -> Result<bool, Error> {
            let Some(pred) = pred else {
                return Ok(true);
            };
            let mut idx = start;
            eval_pred(pred, row, params, &mut idx)
        }

        fn pred_slots(pred: &Option<Pred>) -> usize {
            pred.as_ref().map(count_slots).unwrap_or(0)
        }

        fn count_slots(pred: &Pred) -> usize {
            match pred {
                Pred::Cmp { .. } => 1,
                Pred::IsNull { .. } => 0,
                Pred::And(parts) | Pred::Or(parts) => parts.iter().map(count_slots).sum(),
            }
        }

        fn eval_pred(pred: &Pred, row: &Row, params: &[Value], idx: &mut usize) -> Result<bool, Error> {
            match pred {
                Pred::Cmp { field, op } => {
                    let param = take_param(params, idx)?;
                    let left = row.get(field).cloned().unwrap_or(Value::Null);
                    Ok(compare(&left, *op, param))
                }
                Pred::IsNull { field, negated } => {
                    let is_null = row.get(field).is_none_or(Value::is_null);
                    Ok(if *negated { !is_null } else { is_null })
                }
                Pred::And(parts) => {
                    let mut all = true;
                    for part in parts {
                        if !eval_pred(part, row, params, idx)? {
                            all = false;
                        }
                    }
                    Ok(all)
                }
                Pred::Or(parts) => {
                    let mut any = false;
                    for part in parts {
                        if eval_pred(part, row, params, idx)? {
                            any = true;
                        }
                    }
                    Ok(any)
                }
            }
        }

        fn compare(left: &Value, op: Op, right: &Value) -> bool {
            let ordering = cmp_values(left, right);
            match op {
                Op::Eq => left == right,
                Op::Ne => left != right,
                Op::Lt => ordering == std::cmp::Ordering::Less,
                Op::Gt => ordering == std::cmp::Ordering::Greater,
                Op::Le => ordering != std::cmp::Ordering::Greater,
                Op::Ge => ordering != std::cmp::Ordering::Less,
            }
        }

        fn cmp_values(left: &Value, right: &Value) -> std::cmp::Ordering {
            if left.is_null() && right.is_null() {
                return std::cmp::Ordering::Equal;
            }
            if left.is_null() {
                return std::cmp::Ordering::Greater;
            }
            if right.is_null() {
                return std::cmp::Ordering::Less;
            }
            compare_obj(left, right).unwrap_or(std::cmp::Ordering::Equal)
        }

        fn sort_rows(left: &Row, right: &Row, order: &[OrderKey]) -> std::cmp::Ordering {
            for key in order {
                let left_value = left.get(&key.field).cloned().unwrap_or(Value::Null);
                let right_value = right.get(&key.field).cloned().unwrap_or(Value::Null);
                let mut ordering = cmp_values(&left_value, &right_value);
                if key.desc {
                    ordering = ordering.reverse();
                }
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        }

        fn project(row: &Row, columns: &[String]) -> DbRow {
            DbRow {
                columns: columns
                    .iter()
                    .map(|name| (name.clone(), row.get(name).cloned().unwrap_or(Value::Null)))
                    .collect(),
            }
        }

        fn take_param<'a>(params: &'a [Value], idx: &mut usize) -> Result<&'a Value, Error> {
            let value = params.get(*idx).ok_or_else(|| Error::new("sql parameter missing"))?;
            *idx += 1;
            Ok(value)
        }

        fn as_i64(value: &Value) -> Result<i64, Error> {
            match value {
                Value::I64(number) => Ok(*number),
                Value::I32(number) => Ok(i64::from(*number)),
                Value::String(text) => text.parse().map_err(|_| Error::new("parameter is not an integer")),
                _ => Err(Error::new("parameter is not an integer")),
            }
        }

        enum Stmt {
            Insert {
                table: String,
                columns: Vec<String>,
            },
            Update {
                table: String,
                columns: Vec<String>,
                pred: Option<Pred>,
            },
            Delete {
                table: String,
                pred: Option<Pred>,
            },
            Select(SelectQuery),
        }

        struct SelectQuery {
            count: bool,
            columns: Vec<String>,
            table: String,
            pred: Option<Pred>,
            order: Vec<OrderKey>,
            limit: bool,
            offset: bool,
        }

        enum Pred {
            Cmp { field: String, op: Op },
            IsNull { field: String, negated: bool },
            And(Vec<Pred>),
            Or(Vec<Pred>),
        }

        #[derive(Clone, Copy)]
        enum Op {
            Eq,
            Ne,
            Lt,
            Gt,
            Le,
            Ge,
        }

        struct OrderKey {
            field: String,
            desc: bool,
        }

        fn parse(sql: &str) -> Result<Stmt, Error> {
            let mut cur = Cur { sql, i: 0 };
            let stmt = if cur.eat_keyword("INSERT") {
                parse_insert(&mut cur)?
            } else if cur.eat_keyword("UPDATE") {
                parse_update(&mut cur)?
            } else if cur.eat_keyword("DELETE") {
                parse_delete(&mut cur)?
            } else if cur.eat_keyword("SELECT") {
                parse_select(&mut cur)?
            } else {
                return Err(cur.fail("unsupported sql"));
            };
            Ok(stmt)
        }

        fn parse_insert(cur: &mut Cur) -> Result<Stmt, Error> {
            cur.expect_keyword("INTO")?;
            let table = cur.quoted()?;
            cur.expect_symbol(b'(')?;
            let mut columns = Vec::new();
            loop {
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            cur.expect_symbol(b')')?;
            cur.expect_keyword("VALUES")?;
            cur.expect_symbol(b'(')?;
            for index in 0..columns.len() {
                if index > 0 {
                    cur.expect_symbol(b',')?;
                }
                cur.expect_symbol(b'?')?;
            }
            cur.expect_symbol(b')')?;
            cur.expect_end()?;
            Ok(Stmt::Insert { table, columns })
        }

        fn parse_update(cur: &mut Cur) -> Result<Stmt, Error> {
            let table = cur.quoted()?;
            cur.expect_keyword("SET")?;
            let mut columns = Vec::new();
            loop {
                columns.push(cur.quoted()?);
                cur.expect_symbol(b'=')?;
                cur.expect_symbol(b'?')?;
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            cur.expect_end()?;
            Ok(Stmt::Update { table, columns, pred })
        }

        fn parse_delete(cur: &mut Cur) -> Result<Stmt, Error> {
            cur.expect_keyword("FROM")?;
            let table = cur.quoted()?;
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            cur.expect_end()?;
            Ok(Stmt::Delete { table, pred })
        }

        fn parse_select(cur: &mut Cur) -> Result<Stmt, Error> {
            let count = if cur.eat_keyword("COUNT") {
                cur.expect_symbol(b'(')?;
                cur.expect_symbol(b'*')?;
                cur.expect_symbol(b')')?;
                true
            } else {
                false
            };
            let columns = if count { Vec::new() } else { parse_columns(cur)? };
            cur.expect_keyword("FROM")?;
            let table = cur.quoted()?;
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            let order = if cur.eat_keyword("ORDER") {
                cur.expect_keyword("BY")?;
                parse_order(cur)?
            } else {
                Vec::new()
            };
            let limit = if cur.eat_keyword("LIMIT") {
                cur.expect_symbol(b'?')?;
                true
            } else {
                false
            };
            let offset = if cur.eat_keyword("OFFSET") {
                cur.expect_symbol(b'?')?;
                true
            } else {
                false
            };
            cur.expect_end()?;
            Ok(Stmt::Select(SelectQuery {
                count,
                columns,
                table,
                pred,
                order,
                limit,
                offset,
            }))
        }

        fn parse_columns(cur: &mut Cur) -> Result<Vec<String>, Error> {
            let mut columns = Vec::new();
            loop {
                if cur.peek_keyword("FROM") {
                    break;
                }
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            if columns.is_empty() {
                return Err(cur.fail("select has no columns"));
            }
            Ok(columns)
        }

        fn parse_order(cur: &mut Cur) -> Result<Vec<OrderKey>, Error> {
            let mut order = Vec::new();
            loop {
                let field = cur.quoted()?;
                let desc = if cur.eat_keyword("DESC") {
                    true
                } else if cur.eat_keyword("ASC") {
                    false
                } else {
                    return Err(cur.fail("expected ASC or DESC"));
                };
                order.push(OrderKey { field, desc });
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            Ok(order)
        }

        fn parse_or(cur: &mut Cur) -> Result<Pred, Error> {
            let mut parts = vec![parse_and(cur)?];
            while cur.eat_keyword("OR") {
                parts.push(parse_and(cur)?);
            }
            Ok(fold(parts, false))
        }

        fn parse_and(cur: &mut Cur) -> Result<Pred, Error> {
            let mut parts = vec![parse_primary(cur)?];
            while cur.eat_keyword("AND") {
                parts.push(parse_primary(cur)?);
            }
            Ok(fold(parts, true))
        }

        fn parse_primary(cur: &mut Cur) -> Result<Pred, Error> {
            if cur.eat_symbol(b'(') {
                let pred = parse_or(cur)?;
                cur.expect_symbol(b')')?;
                return Ok(pred);
            }
            let field = cur.quoted()?;
            if cur.eat_keyword("IS") {
                let negated = cur.eat_keyword("NOT");
                cur.expect_keyword("NULL")?;
                return Ok(Pred::IsNull { field, negated });
            }
            let Some(op) = cur.eat_op() else {
                return Err(cur.fail("expected comparison"));
            };
            cur.expect_symbol(b'?')?;
            Ok(Pred::Cmp { field, op })
        }

        fn fold(mut parts: Vec<Pred>, and: bool) -> Pred {
            if parts.len() == 1 {
                return parts.remove(0);
            }
            if and {
                Pred::And(parts)
            } else {
                Pred::Or(parts)
            }
        }

        struct Cur<'a> {
            sql: &'a str,
            i: usize,
        }

        impl Cur<'_> {
            fn skip(&mut self) {
                while self.i < self.sql.len() && self.sql.as_bytes()[self.i].is_ascii_whitespace() {
                    self.i += 1;
                }
            }

            fn eat_keyword(&mut self, word: &str) -> bool {
                self.skip();
                let rest = &self.sql[self.i..];
                if rest.len() < word.len() || !rest.starts_with(word) {
                    return false;
                }
                let end = self.i + word.len();
                if end < self.sql.len() {
                    let next = self.sql.as_bytes()[end];
                    if next.is_ascii_alphanumeric() || next == b'_' {
                        return false;
                    }
                }
                self.i = end;
                true
            }

            fn peek_keyword(&mut self, word: &str) -> bool {
                let saved = self.i;
                let found = self.eat_keyword(word);
                self.i = saved;
                found
            }

            fn eat_symbol(&mut self, symbol: u8) -> bool {
                self.skip();
                if self.i < self.sql.len() && self.sql.as_bytes()[self.i] == symbol {
                    self.i += 1;
                    true
                } else {
                    false
                }
            }

            fn eat_op(&mut self) -> Option<Op> {
                self.skip();
                let rest = &self.sql[self.i..];
                let (op, len) = if rest.starts_with("<>") {
                    (Op::Ne, 2)
                } else if rest.starts_with("<=") {
                    (Op::Le, 2)
                } else if rest.starts_with(">=") {
                    (Op::Ge, 2)
                } else if rest.starts_with('<') {
                    (Op::Lt, 1)
                } else if rest.starts_with('>') {
                    (Op::Gt, 1)
                } else if rest.starts_with('=') {
                    (Op::Eq, 1)
                } else {
                    return None;
                };
                self.i += len;
                Some(op)
            }

            fn quoted(&mut self) -> Result<String, Error> {
                if !self.eat_symbol(b'"') {
                    return Err(self.fail("expected identifier"));
                }
                let start = self.i;
                while self.i < self.sql.len() && self.sql.as_bytes()[self.i] != b'"' {
                    self.i += 1;
                }
                if self.i >= self.sql.len() {
                    return Err(self.fail("unterminated identifier"));
                }
                let name = self.sql[start..self.i].to_string();
                self.i += 1;
                Ok(name)
            }

            fn expect_keyword(&mut self, word: &str) -> Result<(), Error> {
                if self.eat_keyword(word) {
                    Ok(())
                } else {
                    Err(self.fail(&format!("expected {word}")))
                }
            }

            fn expect_symbol(&mut self, symbol: u8) -> Result<(), Error> {
                if self.eat_symbol(symbol) {
                    Ok(())
                } else {
                    Err(self.fail(&format!("expected {}", char::from(symbol))))
                }
            }

            fn expect_end(&mut self) -> Result<(), Error> {
                self.skip();
                if self.i == self.sql.len() {
                    Ok(())
                } else {
                    Err(self.fail("unexpected sql"))
                }
            }

            fn fail(&self, message: &str) -> Error {
                let rest: String = self.sql[self.i..].chars().take(160).collect();
                Error::new(format!("{message} at '{rest}'"))
            }
        }
    }
}
