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

//! Service semaphore. The lock row is written with SQL on a reserve connection.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use moqui_db::sql::{insert, select, update, Condition, FieldRef, LimitOffset};
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{basic_convert, is_empty, to_plain_string, Value};

use super::define::{SemaphoreMode, ServiceDef};
use crate::context::Error;
use crate::driver::{DbConnection, DbRow};
use crate::entity::EntityFacade;
use crate::transaction::TransactionManager;

const COLUMNS: [&str; 4] = ["serviceName", "parameterValue", "lockThread", "lockTime"];

pub fn acquire(name: &str) -> Result<(), Error> {
    let Some(def) = super::define::find(name) else {
        return Ok(());
    };
    if def.semaphore == SemaphoreMode::None {
        return Ok(());
    }
    let facade = current_facade().ok_or_else(|| Error::new("no entity facade is installed"))?;
    let _guard = take(&def, &BTreeMap::new(), &facade)?;
    Ok(())
}

/// `None` when the service has no semaphore. The guard clears the row on drop.
pub(crate) fn take(
    def: &ServiceDef,
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<Option<Guard>, Error> {
    if def.semaphore == SemaphoreMode::None {
        return Ok(None);
    }
    let plan = plan(facade)?;
    let name = def.semaphore_name.clone().unwrap_or_else(|| def.name.clone());
    let scope = scope_value(def, parameters);
    let owner = owner_label();
    let started = Instant::now();
    let pause = if def.semaphore_sleep.is_zero() {
        Duration::from_millis(1)
    } else {
        def.semaphore_sleep
    };
    loop {
        if try_lock(&plan, &name, &scope, def.semaphore_ignore, &owner)? {
            return Ok(Some(Guard {
                tx: plan.tx.clone(),
                clear_sql: plan.clear_sql.clone(),
                owner,
                name,
                scope,
            }));
        }
        let timed_out = started.elapsed() >= def.semaphore_timeout;
        if def.semaphore == SemaphoreMode::Fail || timed_out {
            return Err(Error::new(format!("semaphore {name} is held for {scope}")));
        }
        std::thread::sleep(pause);
    }
}

pub(crate) struct Guard {
    tx: TransactionManager,
    clear_sql: String,
    /// The label this call wrote. The clear only applies while the row still carries it, so a
    /// holder whose lock was taken over after the ignore time does not free the new holder's lock.
    owner: Value,
    name: String,
    scope: String,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let params = vec![
            Value::Null,
            Value::Null,
            Value::string(&self.name),
            Value::string(&self.scope),
            self.owner.clone(),
        ];
        let cleared = reserve(&self.tx, |conn| {
            conn.execute(&self.clear_sql, &params)?;
            Ok(())
        });
        if let Err(err) = cleared {
            tracing::warn!(error = %err, "semaphore clear failed");
        }
    }
}

struct Plan {
    tx: TransactionManager,
    select_sql: String,
    insert_sql: String,
    /// Claim a row nobody holds. Applies only while `lockThread` is still null.
    claim_free_sql: String,
    /// Claim a row whose hold expired. Applies only while `lockThread` is still the expired one.
    claim_stale_sql: String,
    clear_sql: String,
}

fn plan(facade: &EntityFacade) -> Result<Plan, Error> {
    facade
        .with_group(ServiceDef::semaphore_entity(), |entity, dialect, _, tx| {
            let condition = Condition::and(vec![
                Condition::equals("serviceName", Value::Null),
                Condition::equals("parameterValue", Value::Null),
            ]);
            let fields: Vec<FieldRef> = COLUMNS.iter().copied().map(FieldRef::new).collect();
            let select_sql = select(entity, dialect, &fields, Some(&condition), &[], LimitOffset::default())?;
            let insert_sql = insert(entity, dialect, &COLUMNS)?;
            let set = ["lockThread", "lockTime"];
            let free = Condition::and(vec![condition.clone(), Condition::is_null("lockThread")]);
            let owned = Condition::and(vec![condition.clone(), Condition::equals("lockThread", Value::Null)]);
            Ok(Plan {
                tx: tx.clone(),
                select_sql,
                insert_sql,
                claim_free_sql: update(entity, dialect, &set, Some(&free))?,
                claim_stale_sql: update(entity, dialect, &set, Some(&owned))?,
                clear_sql: update(entity, dialect, &set, Some(&owned))?,
            })
        })
        .map_err(Error::from)
}

/// True when this call owns the row.
///
/// Every way to take the row is a conditional write, and one that changes no row means someone
/// else got there first. Two callers cannot both win, and a lost insert race reads as held.
fn try_lock(plan: &Plan, name: &str, scope: &str, ignore: Duration, owner: &Value) -> Result<bool, Error> {
    let name_value = Value::string(name);
    let scope_value = Value::string(scope);
    reserve(&plan.tx, |conn| {
        let rows = conn.query(&plan.select_sql, &[name_value.clone(), scope_value.clone()])?;
        let now = now_millis();
        if let Some(row) = rows.first() {
            if held(row, ignore, now) {
                return Ok(false);
            }
            let previous = row.get("lockThread").cloned().unwrap_or(Value::Null);
            if previous.is_null() {
                let params = vec![owner.clone(), now_stamp(), name_value, scope_value];
                return Ok(conn.execute(&plan.claim_free_sql, &params)? > 0);
            }
            let params = vec![owner.clone(), now_stamp(), name_value, scope_value, previous];
            return Ok(conn.execute(&plan.claim_stale_sql, &params)? > 0);
        }
        let params = vec![name_value.clone(), scope_value.clone(), owner.clone(), now_stamp()];
        match conn.execute(&plan.insert_sql, &params) {
            Ok(_) => Ok(true),
            Err(err) => {
                // A concurrent insert of the same key is a held lock. Anything else is a failure.
                let _ = conn.rollback();
                conn.begin()?;
                if conn.query(&plan.select_sql, &[name_value, scope_value])?.is_empty() {
                    return Err(err);
                }
                Ok(false)
            }
        }
    })
}

fn held(row: &DbRow, ignore: Duration, now: i64) -> bool {
    let thread = row.get("lockThread").cloned().unwrap_or(Value::Null);
    let time = row.get("lockTime").cloned().unwrap_or(Value::Null);
    let thread_set = !thread.is_null() && !is_empty(&thread);
    let time_set = !time.is_null() && !is_empty(&time);
    if !thread_set && !time_set {
        return false;
    }
    if let Some(locked) = stamp_millis(&time) {
        let ignore_ms = i64::try_from(ignore.as_millis()).unwrap_or(i64::MAX);
        if now > locked.saturating_add(ignore_ms) {
            return false;
        }
    }
    true
}

fn scope_value(def: &ServiceDef, parameters: &BTreeMap<String, Value>) -> String {
    let Some(name) = &def.semaphore_parameter else {
        return "_NA_".to_string();
    };
    let text = parameters.get(name).map(to_plain_string).unwrap_or_default();
    if text.is_empty() {
        "_NULL_".to_string()
    } else {
        text
    }
}

/// The thread, plus a number that is new on every call, so a hold can be told from a later hold
/// by the same thread.
fn owner_label() -> Value {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let current = std::thread::current();
    let label = current.name().map(str::to_string).unwrap_or_else(|| {
        let id = current.id();
        format!("{id:?}")
    });
    let number = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Value::string(format!("{label}#{number}"))
}

fn now_stamp() -> Value {
    basic_convert(&Value::I64(now_millis()), "Timestamp").value
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn stamp_millis(value: &Value) -> Option<i64> {
    if value.is_null() || is_empty(value) {
        return None;
    }
    to_plain_string(value).parse().ok()
}

fn reserve<T>(
    tx: &TransactionManager,
    body: impl FnOnce(&mut dyn DbConnection) -> Result<T, LibError>,
) -> Result<T, Error> {
    tx.with_reserve(|conn| {
        conn.begin()?;
        match body(conn) {
            Ok(value) => {
                if let Err(err) = conn.commit() {
                    let _ = conn.rollback();
                    return Err(err);
                }
                Ok(value)
            }
            Err(err) => {
                let _ = conn.rollback();
                Err(err)
            }
        }
    })
    .map_err(Error::from)
}

fn current_facade() -> Option<EntityFacade> {
    crate::context::ExecutionContext::with_current(|ctx| ctx.factory().entity()).flatten()
}
