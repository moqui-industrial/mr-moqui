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

//! Entity one-cache, the per-thread transaction cache, and the sim overlay.
//!
//! A find is stored only when the entity `cache` attribute is `true`. An open
//! transaction holds writes in memory until the commit flush. Sim writes go to
//! the overlay pool and stay off the main schema.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use moqui_db::dialect::Dialect;
use moqui_db::model::EntityDefinition;
use moqui_db::sql::{
    create_table, delete, insert, insert_or_update, select, update, CompareOp, Condition, FieldRef, LimitOffset,
    OrderBy,
};
use moqui_lib::cache::{CacheConfig, MCache};
use moqui_lib::error::Error;
use moqui_lib::object::{compare_like, compare_obj, to_plain_string, Value};

use super::exec::{execute_conn, execute_script, query_conn, query_tx};
use super::facade::EntityFacade;
use super::field_rw;
use super::params::{condition_params, entity_columns, map_row};
use super::value::{EntityList, EntityValue};
use crate::driver::{DbConnection, DbRow};
use crate::transaction::TransactionManager;

const LIST_MAX: usize = 10_000;
const PK_SEP: char = '\u{1f}';

static SIM_SEQ: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static TX_HOLD: RefCell<Vec<Held>> = const { RefCell::new(Vec::new()) };
    static SIM: RefCell<Vec<SimSession>> = const { RefCell::new(Vec::new()) };
}

/// The find `list` and `iterator` ask the cache about. Owned so a cache can store the key.
pub struct FindLookup {
    pub entity_name: String,
    pub condition: Option<Condition>,
    pub order: Vec<OrderBy>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Whether the caller should run the write on the enlisted connection.
///
/// `Handled` means this module stored the write or ran it itself. Return that only when the write
/// will not be lost: a transaction cache inside an open frame, or a sim write already sent to the
/// overlay. `on_write` still runs after either variant and is the one-cache invalidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteGate {
    Proceed,
    Handled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Create,
    Update,
    Delete,
    Upsert,
}

#[derive(Clone)]
struct Held {
    entity_name: String,
    operation: Op,
    plain: BTreeMap<String, Value>,
    sealed: BTreeMap<String, Value>,
    pk: String,
}

struct SealedWrite {
    full_name: String,
    pk: String,
    sealed: BTreeMap<String, Value>,
}

struct ReadMeta {
    full_name: String,
    is_view: bool,
    cache: bool,
    pk_fields: Vec<String>,
}

struct MemRow {
    fields: BTreeMap<String, Value>,
    trusted: bool,
}

struct SimSession {
    schema: String,
    tables: HashSet<String>,
    tombstones: HashMap<String, HashSet<String>>,
    written: HashSet<String>,
}

struct SimHit {
    schema: String,
    table: bool,
    tombstone: bool,
    written: bool,
}

struct SimView {
    schema: String,
    table: bool,
    hide: HashSet<String>,
}

#[derive(Default)]
struct OneCaches {
    lists: HashMap<String, MCache<String, EntityList>>,
}

/// Look up a cached list, or rows held by this thread's transaction or sim overlay.
///
/// `Some` skips SQL. Plaintext only: `after_find` stored decrypted rows, and held rows were
/// normalized before `gate_write`.
pub fn before_find(facade: &EntityFacade, lookup: &FindLookup) -> Option<EntityList> {
    let meta = match read_meta(facade, &lookup.entity_name) {
        Ok(meta) => meta,
        Err(err) => {
            tracing::error!(error = %err, "entity cache find skipped");
            return None;
        }
    };
    if !meta.is_view {
        match overlay_rows(facade, &meta, lookup.condition.as_ref(), &lookup.order) {
            Ok(Some(rows)) => {
                return Some(EntityList::new(page_rows(rows, lookup.limit, lookup.offset)));
            }
            Ok(None) => {}
            Err(err) => {
                tracing::error!(error = %err, entity = %meta.full_name, "entity cache find skipped");
                return None;
            }
        }
    }
    if meta.cache {
        return cache_get(facade, &meta.full_name, lookup);
    }
    None
}

/// Count held and sim rows when this thread has overlaid that entity. Otherwise miss.
///
/// `Some` is the count itself. The merged list is unlimited, so its length is that count.
pub fn before_count(facade: &EntityFacade, entity_name: &str, condition: Option<&Condition>) -> Option<i64> {
    let meta = match read_meta(facade, entity_name) {
        Ok(meta) => meta,
        Err(err) => {
            tracing::error!(error = %err, "entity cache count skipped");
            return None;
        }
    };
    if meta.is_view {
        return None;
    }
    match overlay_rows(facade, &meta, condition, &[]) {
        Ok(Some(rows)) => Some(i64::try_from(rows.len()).unwrap_or(i64::MAX)),
        Ok(None) => None,
        Err(err) => {
            tracing::error!(error = %err, entity = %meta.full_name, "entity cache count skipped");
            None
        }
    }
}

/// Store a find result when the entity cache attribute is `true`. Rows are already plaintext.
pub fn after_find(facade: &EntityFacade, lookup: &FindLookup, rows: &EntityList) -> Result<(), Error> {
    let meta = read_meta(facade, &lookup.entity_name)?;
    if !meta.cache {
        return Ok(());
    }
    let key = lookup_key(lookup);
    let stored = rows.clone();
    let name = meta.full_name;
    facade.with_slot(OneCaches::default, |caches| {
        let cache = caches.lists.entry(name.clone()).or_insert_with(|| list_cache(&name));
        cache.put(key, stored)
    })
}

/// Hold the write, or send it to the sim overlay. `Proceed` leaves SQL to the caller.
pub fn gate_write(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    values: &BTreeMap<String, Value>,
) -> Result<WriteGate, Error> {
    let op = parse_op(operation)?;
    let sim = sim_live(facade);
    let tx = facade.transactions_for(entity_name)?;
    if !sim && !tx.transaction_cache_active() {
        return Ok(WriteGate::Proceed);
    }
    let prepared = prepare_handled(facade, entity_name, operation, values)?;
    if sim {
        if tx.in_transaction() {
            tx.with_connection(|_| Ok(()))?;
        }
        write_sim(facade, entity_name, op, values, &prepared)?;
        return Ok(WriteGate::Handled);
    }
    tx.with_connection(|_| Ok(()))?;
    push_hold(Held {
        entity_name: prepared.full_name,
        operation: op,
        plain: values.clone(),
        sealed: prepared.sealed,
        pk: prepared.pk,
    })?;
    Ok(WriteGate::Handled)
}

/// Clear this entity's one-cache. The transaction flush is separate and runs at commit.
pub fn on_write(facade: &EntityFacade, entity_name: &str, operation: &str) -> Result<(), Error> {
    let _ = operation;
    let meta = read_meta(facade, entity_name)?;
    facade.with_slot(OneCaches::default, |caches| {
        if let Some(cache) = caches.lists.get(&meta.full_name) {
            cache.clear()?;
        }
        Ok(())
    })
}

/// Register the commit flush and the rollback drop for this frame.
///
/// The closure reads the thread-local list when it runs, so later writes in the frame are included.
/// It uses the enlisted connection directly.
pub fn register_cache_flush(facade: &EntityFacade, tx: &TransactionManager) -> Result<(), Error> {
    let facade = facade.clone();
    tx.on_cache_flush(move |conn| flush_held(&facade, conn))?;
    tx.on_rollback(|| {
        TX_HOLD.with(|slot| slot.borrow_mut().clear());
    })?;
    Ok(())
}

/// Open a sim schema on the overlay pool and mark sim active.
///
/// A second call on this thread pushes another schema. Reads and writes use the innermost one.
/// Fails without changing `sim_active` when no overlay pool is set. Does not start a cluster.
pub fn enter_sim(facade: &EntityFacade) -> Result<(), Error> {
    let Some(sim) = facade.sim_transactions() else {
        return Err(Error::new("sim transactions are not configured"));
    };
    let schema = next_schema();
    if !schema_name_ok(&schema) {
        return Err(Error::new("invalid sim schema"));
    }
    sim.with_connection(|conn| execute_conn(conn, &format!("CREATE SCHEMA {schema}"), &[]))?;
    SIM.with(|slot| {
        slot.borrow_mut().push(SimSession {
            schema,
            tables: HashSet::new(),
            tombstones: HashMap::new(),
            written: HashSet::new(),
        });
    });
    facade.set_sim_active(true);
    Ok(())
}

/// Drop this thread's innermost sim schema. `sim_active` stays set while an outer schema remains.
///
/// No session is a no-op.
pub fn exit_sim(facade: &EntityFacade) -> Result<(), Error> {
    let session = SIM.with(|slot| slot.borrow_mut().pop());
    if !sim_session_open() {
        facade.set_sim_active(false);
    }
    let Some(session) = session else {
        return Ok(());
    };
    let Some(sim) = facade.sim_transactions() else {
        return Err(Error::new("sim transactions are not configured"));
    };
    if !schema_name_ok(&session.schema) {
        return Err(Error::new("refusing to drop an invalid sim schema"));
    }
    sim.with_connection(|conn| {
        execute_conn(conn, &format!("DROP SCHEMA IF EXISTS {} CASCADE", session.schema), &[])?;
        Ok(())
    })
}

fn flush_held(facade: &EntityFacade, conn: &mut dyn DbConnection) -> Result<(), Error> {
    let held_rows = TX_HOLD.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    for row in held_rows {
        let statement = facade.with_group(&row.entity_name, |entity, dialect, _, _| {
            statement_for(entity, dialect, row.operation, &row.sealed)
        })?;
        if let Some((sql, params)) = statement {
            conn.execute(&sql, &params)?;
        }
    }
    Ok(())
}

fn push_hold(held: Held) -> Result<(), Error> {
    TX_HOLD.with(|slot| {
        let mut list = slot.borrow_mut();
        let mut first = None;
        let mut last = None;
        for existing in list.iter() {
            if existing.entity_name == held.entity_name && existing.pk == held.pk {
                if first.is_none() {
                    first = Some(existing.operation);
                }
                last = Some(existing.operation);
            }
        }
        match held.operation {
            Op::Create => {
                if last.is_some_and(|op| op != Op::Delete) {
                    return Err(Error::new(format!(
                        "create of existing transaction cache row {} {}",
                        held.entity_name, held.pk
                    )));
                }
                list.push(held);
                Ok(())
            }
            Op::Update | Op::Upsert => {
                list.push(held);
                Ok(())
            }
            Op::Delete => {
                if first == Some(Op::Create) {
                    list.retain(|existing| existing.entity_name != held.entity_name || existing.pk != held.pk);
                    Ok(())
                } else {
                    list.push(held);
                    Ok(())
                }
            }
        }
    })
}

fn write_sim(
    facade: &EntityFacade,
    entity_name: &str,
    op: Op,
    plain: &BTreeMap<String, Value>,
    prepared: &SealedWrite,
) -> Result<(), Error> {
    let hit = sim_hit(&prepared.full_name, &prepared.pk)?;
    // The overlay dialect rejects insert-or-update. A new sim row is an insert; a written row is an update.
    let op = match op {
        Op::Upsert if hit.written => Op::Update,
        Op::Upsert => Op::Create,
        other => other,
    };
    if op == Op::Update && hit.tombstone {
        return Ok(());
    }
    if op == Op::Create && hit.written {
        return Err(Error::new(format!(
            "create of existing sim row {} {}",
            prepared.full_name, prepared.pk
        )));
    }
    if op == Op::Delete && !hit.table {
        note_sim(&prepared.full_name, &prepared.pk, Op::Delete, false);
        return Ok(());
    }
    let sealed_copy = if op == Op::Update && !hit.written {
        match load_by_pk(facade, entity_name, plain)? {
            Some(row) => Some(seal(facade, entity_name, "create", row)?),
            None => None,
        }
    } else {
        None
    };
    let ddl = if hit.table {
        None
    } else {
        Some(create_sql(facade, entity_name)?)
    };
    let statements = sim_statements(facade, entity_name, op, &hit, &prepared.sealed, sealed_copy.as_ref())?;
    let sim = facade
        .sim_transactions()
        .ok_or_else(|| Error::new("sim transactions are not configured"))?;
    run_overlay(&sim, &hit.schema, ddl.as_deref(), &statements)?;
    note_sim(&prepared.full_name, &prepared.pk, op, ddl.is_some());
    Ok(())
}

fn sim_statements(
    facade: &EntityFacade,
    entity_name: &str,
    op: Op,
    hit: &SimHit,
    sealed: &BTreeMap<String, Value>,
    sealed_copy: Option<&BTreeMap<String, Value>>,
) -> Result<Vec<(String, Vec<Value>)>, Error> {
    let mut statements = Vec::new();
    if let Some(copied) = sealed_copy {
        push_statement(&mut statements, dml(facade, entity_name, Op::Create, copied)?);
        push_statement(&mut statements, dml(facade, entity_name, Op::Update, sealed)?);
    } else if op == Op::Update && !hit.written {
        push_statement(&mut statements, dml(facade, entity_name, Op::Create, sealed)?);
    } else {
        push_statement(&mut statements, dml(facade, entity_name, op, sealed)?);
    }
    Ok(statements)
}

fn push_statement(statements: &mut Vec<(String, Vec<Value>)>, statement: Option<(String, Vec<Value>)>) {
    if let Some(statement) = statement {
        statements.push(statement);
    }
}

fn run_overlay(
    sim: &TransactionManager,
    schema: &str,
    ddl: Option<&str>,
    statements: &[(String, Vec<Value>)],
) -> Result<(), Error> {
    sim.with_connection(|conn| {
        conn.begin()?;
        let result = apply_overlay(conn, schema, ddl, statements);
        match result {
            Ok(()) => conn.commit(),
            Err(err) => {
                let _ = conn.rollback();
                Err(err)
            }
        }
    })
}

fn apply_overlay(
    conn: &mut dyn DbConnection,
    schema: &str,
    ddl: Option<&str>,
    statements: &[(String, Vec<Value>)],
) -> Result<(), Error> {
    query_conn(
        conn,
        "SELECT set_config('search_path', ?, false)",
        &[Value::string(schema)],
    )?;
    if let Some(ddl) = ddl {
        execute_script(conn, ddl)?;
    }
    for (sql, params) in statements {
        execute_conn(conn, sql, params)?;
    }
    Ok(())
}

fn note_sim(entity: &str, pk: &str, op: Op, created_table: bool) {
    let row_key = row_key(entity, pk);
    SIM.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(session) = guard.last_mut() else {
            return;
        };
        if created_table {
            session.tables.insert(entity.to_string());
        }
        match op {
            Op::Create | Op::Update | Op::Upsert => {
                if let Some(set) = session.tombstones.get_mut(entity) {
                    set.remove(pk);
                }
                session.written.insert(row_key);
            }
            Op::Delete => {
                session.written.remove(&row_key);
                session
                    .tombstones
                    .entry(entity.to_string())
                    .or_default()
                    .insert(pk.to_string());
            }
        }
    });
}

fn overlay_rows(
    facade: &EntityFacade,
    meta: &ReadMeta,
    condition: Option<&Condition>,
    order: &[OrderBy],
) -> Result<Option<Vec<EntityValue>>, Error> {
    let holds = holds_for(&meta.full_name);
    let sim = if sim_live(facade) {
        sim_view(&meta.full_name)
    } else {
        SimView::empty()
    };
    if holds.is_empty() && !sim.affects() {
        return Ok(None);
    }
    let loaded = load_plain(facade, meta, condition, order, None)?;
    let mut rows: Vec<MemRow> = loaded
        .into_iter()
        .map(|fields| MemRow { fields, trusted: true })
        .collect();
    apply_holds(&mut rows, &holds, &meta.pk_fields);
    rows.retain(|row| row.trusted || row_matches(condition, &row.fields));
    if sim.affects() {
        rows.retain(|row| match pk_of(&row.fields, &meta.pk_fields) {
            Some(pk) => !sim.hide.contains(&row_key(&meta.full_name, &pk)),
            None => true,
        });
        if sim.table {
            let extra = load_plain(facade, meta, condition, order, Some(&sim.schema))?;
            for fields in extra {
                if let Some(pk) = pk_of(&fields, &meta.pk_fields) {
                    rows.retain(|row| pk_of(&row.fields, &meta.pk_fields).as_deref() != Some(pk.as_str()));
                }
                rows.push(MemRow { fields, trusted: true });
            }
        }
    }
    if !order.is_empty() {
        sort_rows(&mut rows, order);
    }
    let values = rows
        .into_iter()
        .map(|row| EntityValue::from_fields(meta.full_name.clone(), row.fields))
        .collect();
    Ok(Some(values))
}

fn apply_holds(rows: &mut Vec<MemRow>, holds: &[Held], pk_fields: &[String]) {
    for held in holds {
        match held.operation {
            Op::Delete => rows.retain(|row| pk_of(&row.fields, pk_fields).as_deref() != Some(held.pk.as_str())),
            Op::Create => {
                rows.retain(|row| pk_of(&row.fields, pk_fields).as_deref() != Some(held.pk.as_str()));
                rows.push(MemRow {
                    fields: held.plain.clone(),
                    trusted: false,
                });
            }
            Op::Update | Op::Upsert => {
                if let Some(row) = rows
                    .iter_mut()
                    .find(|row| pk_of(&row.fields, pk_fields).as_deref() == Some(held.pk.as_str()))
                {
                    for (key, value) in &held.plain {
                        row.fields.insert(key.clone(), value.clone());
                    }
                    row.trusted = false;
                } else {
                    rows.push(MemRow {
                        fields: held.plain.clone(),
                        trusted: false,
                    });
                }
            }
        }
    }
}

fn load_plain(
    facade: &EntityFacade,
    meta: &ReadMeta,
    condition: Option<&Condition>,
    order: &[OrderBy],
    overlay_schema: Option<&str>,
) -> Result<Vec<BTreeMap<String, Value>>, Error> {
    let plan = plan_select(facade, &meta.full_name, condition, order)?;
    let db_rows = if let Some(schema) = overlay_schema {
        let sim = facade
            .sim_transactions()
            .ok_or_else(|| Error::new("sim transactions are not configured"))?;
        sim.with_connection(|conn| {
            query_conn(
                conn,
                "SELECT set_config('search_path', ?, false)",
                &[Value::string(schema)],
            )?;
            query_conn(conn, &plan.sql, &plan.params)
        })?
    } else {
        query_tx(&plan.tx, &plan.sql, &plan.params)?
    };
    rows_plain(facade, &meta.full_name, &plan.columns, &db_rows)
}

fn load_by_pk(
    facade: &EntityFacade,
    entity_name: &str,
    values: &BTreeMap<String, Value>,
) -> Result<Option<BTreeMap<String, Value>>, Error> {
    let owner = facade.clone();
    let plan = facade.with_group(entity_name, |entity, dialect, model, tx| {
        let condition = pk_condition(entity, values)?;
        let fields: Vec<FieldRef> = entity.fields.iter().map(|field| FieldRef::new(&field.name)).collect();
        let sql = select(entity, dialect, &fields, Some(&condition), &[], LimitOffset::default())?;
        let params = condition_params(&condition);
        Ok(SelectPlan {
            sql,
            params,
            columns: entity_columns(entity, model),
            tx: tx.clone(),
        })
    })?;
    let db_rows = query_tx(&plan.tx, &plan.sql, &plan.params)?;
    let mut plain = rows_plain(&owner, entity_name, &plan.columns, &db_rows)?;
    Ok(plain.pop())
}

struct SelectPlan {
    sql: String,
    params: Vec<Value>,
    columns: Vec<(String, String)>,
    tx: TransactionManager,
}

fn plan_select(
    facade: &EntityFacade,
    entity_name: &str,
    condition: Option<&Condition>,
    order: &[OrderBy],
) -> Result<SelectPlan, Error> {
    facade.with_group(entity_name, |entity, dialect, model, tx| {
        if entity.is_view {
            return Err(Error::new(format!("cannot overlay view-entity {}", entity.full_name)));
        }
        let fields: Vec<FieldRef> = entity.fields.iter().map(|field| FieldRef::new(&field.name)).collect();
        let sql = select(entity, dialect, &fields, condition, order, LimitOffset::default())?;
        let params = match condition {
            Some(condition) => condition_params(condition),
            None => Vec::new(),
        };
        Ok(SelectPlan {
            sql,
            params,
            columns: entity_columns(entity, model),
            tx: tx.clone(),
        })
    })
}

fn rows_plain(
    facade: &EntityFacade,
    entity_name: &str,
    columns: &[(String, String)],
    rows: &[DbRow],
) -> Result<Vec<BTreeMap<String, Value>>, Error> {
    let owner = facade.clone();
    facade.with_entity(entity_name, |entity, _, _| {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let mapped = map_row(columns, row);
            out.push(field_rw::after_read(&owner, entity, mapped)?);
        }
        Ok(out)
    })
}

fn create_sql(facade: &EntityFacade, entity_name: &str) -> Result<String, Error> {
    facade.with_group(entity_name, |entity, dialect, model, _| {
        create_table(entity, model, dialect)
    })
}

fn dml(
    facade: &EntityFacade,
    entity_name: &str,
    op: Op,
    values: &BTreeMap<String, Value>,
) -> Result<Option<(String, Vec<Value>)>, Error> {
    facade.with_group(entity_name, |entity, dialect, _, _| {
        statement_for(entity, dialect, op, values)
    })
}

fn statement_for(
    entity: &EntityDefinition,
    dialect: &Dialect,
    operation: Op,
    values: &BTreeMap<String, Value>,
) -> Result<Option<(String, Vec<Value>)>, Error> {
    match operation {
        Op::Delete => {
            let condition = pk_condition(entity, values)?;
            let sql = delete(entity, dialect, Some(&condition))?;
            Ok(Some((sql, condition_params(&condition))))
        }
        Op::Update => {
            let columns = present_columns(entity, values, true, true);
            if columns.is_empty() {
                return Ok(None);
            }
            let condition = pk_condition(entity, values)?;
            let sql = update(entity, dialect, &columns, Some(&condition))?;
            let mut params = bind_columns(&columns, values);
            params.extend(condition_params(&condition));
            Ok(Some((sql, params)))
        }
        Op::Create => {
            let columns = present_columns(entity, values, false, false);
            if columns.is_empty() {
                return Err(Error::new(format!("insert has no columns for {}", entity.full_name)));
            }
            let sql = insert(entity, dialect, &columns)?;
            Ok(Some((sql, bind_columns(&columns, values))))
        }
        Op::Upsert => {
            let columns = present_columns(entity, values, false, false);
            if columns.is_empty() {
                return Err(Error::new(format!("upsert has no columns for {}", entity.full_name)));
            }
            let sql = insert_or_update(entity, dialect, &columns)?;
            Ok(Some((sql, bind_columns(&columns, values))))
        }
    }
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

fn pk_condition(entity: &EntityDefinition, values: &BTreeMap<String, Value>) -> Result<Condition, Error> {
    let mut parts = Vec::new();
    for name in entity.pk_field_names() {
        let value = values
            .get(name)
            .filter(|value| !value.is_null())
            .cloned()
            .ok_or_else(|| Error::new(format!("primary key {name} is required on {}", entity.full_name)))?;
        parts.push(Condition::equals(name, value));
    }
    if parts.is_empty() {
        return Err(Error::new(format!("{} has no primary key", entity.full_name)));
    }
    if parts.len() == 1 {
        return Ok(parts.remove(0));
    }
    Ok(Condition::and(parts))
}

fn prepare_handled(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    values: &BTreeMap<String, Value>,
) -> Result<SealedWrite, Error> {
    let owner = facade.clone();
    let operation = operation.to_string();
    facade.with_entity(entity_name, |entity, _, _| {
        let pk = identity(entity, values)?;
        let seal_operation = if operation == "upsert" {
            "create"
        } else {
            operation.as_str()
        };
        let sealed = field_rw::before_write(&owner, entity, seal_operation, values.clone())?;
        Ok(SealedWrite {
            full_name: entity.full_name.clone(),
            pk,
            sealed,
        })
    })
}

fn seal(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let owner = facade.clone();
    let operation = operation.to_string();
    facade.with_entity(entity_name, |entity, _, _| {
        field_rw::before_write(&owner, entity, &operation, values)
    })
}

fn read_meta(facade: &EntityFacade, entity_name: &str) -> Result<ReadMeta, Error> {
    facade.with_entity(entity_name, |entity, _, _| {
        Ok(ReadMeta {
            full_name: entity.full_name.clone(),
            is_view: entity.is_view,
            cache: entity.cache == "true",
            pk_fields: entity.pk_field_names().into_iter().map(str::to_string).collect(),
        })
    })
}

fn identity(entity: &EntityDefinition, values: &BTreeMap<String, Value>) -> Result<String, Error> {
    let names = entity.pk_field_names();
    if names.is_empty() {
        return Err(Error::new(format!("{} has no primary key", entity.full_name)));
    }
    let mut key = String::new();
    for (index, name) in names.iter().enumerate() {
        let value = values
            .get(*name)
            .filter(|value| !value.is_null())
            .ok_or_else(|| Error::new(format!("primary key {name} is required on {}", entity.full_name)))?;
        if index > 0 {
            key.push(PK_SEP);
        }
        key.push_str(name);
        key.push('=');
        key.push_str(&to_plain_string(value));
    }
    Ok(key)
}

fn pk_of(fields: &BTreeMap<String, Value>, pk_fields: &[String]) -> Option<String> {
    if pk_fields.is_empty() {
        return None;
    }
    let mut key = String::new();
    for (index, name) in pk_fields.iter().enumerate() {
        let value = fields.get(name).filter(|value| !value.is_null())?;
        if index > 0 {
            key.push(PK_SEP);
        }
        key.push_str(name);
        key.push('=');
        key.push_str(&to_plain_string(value));
    }
    Some(key)
}

fn row_key(entity: &str, pk: &str) -> String {
    format!("{entity}{PK_SEP}{pk}")
}

fn holds_for(entity: &str) -> Vec<Held> {
    TX_HOLD.with(|slot| {
        slot.borrow()
            .iter()
            .filter(|held| held.entity_name == entity)
            .cloned()
            .collect()
    })
}

fn sim_live(facade: &EntityFacade) -> bool {
    facade.sim_active() && sim_session_open()
}

fn sim_session_open() -> bool {
    SIM.with(|slot| !slot.borrow().is_empty())
}

fn sim_hit(entity: &str, pk: &str) -> Result<SimHit, Error> {
    let key = row_key(entity, pk);
    SIM.with(|slot| {
        let guard = slot.borrow();
        let session = guard.last().ok_or_else(|| Error::new("sim session is missing"))?;
        Ok(SimHit {
            schema: session.schema.clone(),
            table: session.tables.contains(entity),
            tombstone: session.tombstones.get(entity).is_some_and(|set| set.contains(pk)),
            written: session.written.contains(&key),
        })
    })
}

fn sim_view(entity: &str) -> SimView {
    SIM.with(|slot| {
        let guard = slot.borrow();
        let Some(session) = guard.last() else {
            return SimView::empty();
        };
        let mut hide = HashSet::new();
        if let Some(set) = session.tombstones.get(entity) {
            for pk in set {
                hide.insert(row_key(entity, pk));
            }
        }
        let prefix = format!("{entity}{PK_SEP}");
        for key in &session.written {
            if key.starts_with(&prefix) {
                hide.insert(key.clone());
            }
        }
        SimView {
            schema: session.schema.clone(),
            table: session.tables.contains(entity),
            hide,
        }
    })
}

impl SimView {
    fn empty() -> Self {
        Self {
            schema: String::new(),
            table: false,
            hide: HashSet::new(),
        }
    }

    fn affects(&self) -> bool {
        self.table || !self.hide.is_empty()
    }
}

fn cache_get(facade: &EntityFacade, entity_name: &str, lookup: &FindLookup) -> Option<EntityList> {
    let key = lookup_key(lookup);
    facade.with_slot(OneCaches::default, |caches| {
        let cache = caches.lists.get(entity_name)?;
        cache.get(&key).ok().flatten()
    })
}

fn list_cache(name: &str) -> MCache<String, EntityList> {
    MCache::new(
        format!("entity.record.list.{name}"),
        CacheConfig::new().max_entries(LIST_MAX).background(true),
    )
}

fn lookup_key(lookup: &FindLookup) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}",
        lookup.condition, lookup.order, lookup.limit, lookup.offset
    )
}

fn page_rows(mut rows: Vec<EntityValue>, limit: Option<i64>, offset: Option<i64>) -> Vec<EntityValue> {
    let start = usize::try_from(offset.unwrap_or(0).max(0)).unwrap_or(usize::MAX);
    if start >= rows.len() {
        return Vec::new();
    }
    if start > 0 {
        rows.drain(..start);
    }
    if let Some(limit) = limit {
        let limit = usize::try_from(limit.max(0)).unwrap_or(0);
        if rows.len() > limit {
            rows.truncate(limit);
        }
    }
    rows
}

fn sort_rows(rows: &mut [MemRow], order: &[OrderBy]) {
    rows.sort_by(|left, right| {
        for entry in order {
            let left_value = left.fields.get(&entry.field.name).cloned().unwrap_or(Value::Null);
            let right_value = right.fields.get(&entry.field.name).cloned().unwrap_or(Value::Null);
            let mut ord = cmp_nulls_last(&left_value, &right_value);
            if entry.descending {
                ord = ord.reverse();
            }
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
}

fn cmp_nulls_last(left: &Value, right: &Value) -> Ordering {
    if left.is_null() && right.is_null() {
        return Ordering::Equal;
    }
    if left.is_null() {
        return Ordering::Greater;
    }
    if right.is_null() {
        return Ordering::Less;
    }
    compare_obj(left, right).unwrap_or(Ordering::Equal)
}

fn row_matches(condition: Option<&Condition>, fields: &BTreeMap<String, Value>) -> bool {
    match condition {
        None => true,
        Some(condition) => eval_condition(condition, fields),
    }
}

fn eval_condition(condition: &Condition, fields: &BTreeMap<String, Value>) -> bool {
    match condition {
        Condition::Equals(field, expected) => field_value(fields, &field.name) == *expected,
        Condition::NotEquals(field, expected) => {
            let got = field_value(fields, &field.name);
            !got.is_null() && got != *expected
        }
        Condition::Less(field, expected) => {
            cmp_present(&field_value(fields, &field.name), expected) == Some(Ordering::Less)
        }
        Condition::Greater(field, expected) => {
            cmp_present(&field_value(fields, &field.name), expected) == Some(Ordering::Greater)
        }
        Condition::LessEqual(field, expected) => {
            cmp_present(&field_value(fields, &field.name), expected).is_some_and(|ord| ord != Ordering::Greater)
        }
        Condition::GreaterEqual(field, expected) => {
            cmp_present(&field_value(fields, &field.name), expected).is_some_and(|ord| ord != Ordering::Less)
        }
        Condition::In(field, values) => {
            if values.is_empty() {
                return false;
            }
            let got = field_value(fields, &field.name);
            values.contains(&got)
        }
        Condition::NotIn(field, values) => {
            if values.is_empty() {
                return true;
            }
            let got = field_value(fields, &field.name);
            !got.is_null() && !values.contains(&got)
        }
        Condition::Between(field, low, high) => {
            let got = field_value(fields, &field.name);
            let above = cmp_present(&got, low).is_some_and(|ord| ord != Ordering::Less);
            let below = cmp_present(&got, high).is_some_and(|ord| ord != Ordering::Greater);
            above && below
        }
        Condition::Like(field, pattern) => compare_like(&field_value(fields, &field.name), pattern),
        Condition::NotLike(field, pattern) => !compare_like(&field_value(fields, &field.name), pattern),
        Condition::IsNull(field) => field_value(fields, &field.name).is_null(),
        Condition::IsNotNull(field) => !field_value(fields, &field.name).is_null(),
        Condition::And(list) => list.iter().all(|child| eval_condition(child, fields)),
        Condition::Or(list) => list.iter().any(|child| eval_condition(child, fields)),
        Condition::Not(inner) => !eval_condition(inner, fields),
        Condition::Matches(_, _) => false,
        Condition::CompareFields(left, op, right) => {
            let ordering = cmp_present(&field_value(fields, &left.name), &field_value(fields, &right.name));
            match (op, ordering) {
                (CompareOp::Equals, _) => field_value(fields, &left.name) == field_value(fields, &right.name),
                (CompareOp::NotEquals, ordering) => ordering.is_some_and(|ord| ord != Ordering::Equal),
                (CompareOp::Less, ordering) => ordering == Some(Ordering::Less),
                (CompareOp::Greater, ordering) => ordering == Some(Ordering::Greater),
                (CompareOp::LessEqual, ordering) => ordering.is_some_and(|ord| ord != Ordering::Greater),
                (CompareOp::GreaterEqual, ordering) => ordering.is_some_and(|ord| ord != Ordering::Less),
            }
        }
        Condition::IgnoreCase(inner) => {
            let upper: BTreeMap<String, Value> = fields
                .iter()
                .map(|(name, value)| match value {
                    Value::String(text) => (name.clone(), Value::String(text.to_uppercase())),
                    other => (name.clone(), other.clone()),
                })
                .collect();
            eval_condition(&inner.as_ref().clone().upper_values(), &upper)
        }
    }
}

fn field_value(fields: &BTreeMap<String, Value>, name: &str) -> Value {
    fields.get(name).cloned().unwrap_or(Value::Null)
}

fn cmp_present(left: &Value, right: &Value) -> Option<Ordering> {
    if left.is_null() || right.is_null() {
        return None;
    }
    compare_obj(left, right).ok()
}

fn parse_op(operation: &str) -> Result<Op, Error> {
    match operation {
        "create" => Ok(Op::Create),
        "update" => Ok(Op::Update),
        "delete" => Ok(Op::Delete),
        "upsert" => Ok(Op::Upsert),
        other => Err(Error::new(format!("unknown entity write {other}"))),
    }
}

fn next_schema() -> String {
    let seq = SIM_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
    format!("sim_{}_{seq}", std::process::id())
}

fn schema_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {
            chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') && name.len() <= 63
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use moqui_db::sql::{CompareOp, Condition};
    use moqui_lib::object::Value;

    use super::{eval_condition, WriteGate};

    fn row(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn held_rows_answer_ignore_case_and_field_comparisons() {
        let fields = row(&[
            ("name", Value::string("Gamma")),
            ("low", Value::I32(1)),
            ("high", Value::I32(5)),
        ]);
        let same_case = Condition::ignore_case(Condition::equals("name", Value::string("gAMMA")));
        assert!(eval_condition(&same_case, &fields));
        let other = Condition::ignore_case(Condition::like("name", Value::string("GAM%")));
        assert!(eval_condition(&other, &fields));
        assert!(!eval_condition(
            &Condition::ignore_case(Condition::equals("name", Value::string("beta"))),
            &fields
        ));
        assert!(eval_condition(
            &Condition::compare_fields("low", CompareOp::Less, "high"),
            &fields
        ));
        assert!(!eval_condition(
            &Condition::compare_fields("low", CompareOp::GreaterEqual, "high"),
            &fields
        ));
        assert!(eval_condition(
            &Condition::compare_fields("low", CompareOp::NotEquals, "high"),
            &fields
        ));
    }

    #[test]
    fn handled_is_a_write_gate() {
        assert_eq!(WriteGate::Handled, WriteGate::Handled);
        assert_ne!(WriteGate::Proceed, WriteGate::Handled);
    }
}
