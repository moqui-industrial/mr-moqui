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

//! Entity find. The condition tree is `moqui_db`. A view uses `select_view`.

use moqui_db::sql::{count, select, select_view, Condition, FieldRef, LimitOffset, OrderBy};
use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, Value};

use super::cache::{self, FindLookup};
use super::exec::{after_statement, query_tx};
use super::facade::EntityFacade;
use super::field_rw;
use super::params::{condition_params, entity_columns, map_row, page_params, view_columns};
use super::value::{EntityList, EntityValue};
use crate::driver::DbRow;
use crate::transaction::TransactionManager;

/// A find builder. Methods consume `self` so conditions chain.
pub struct EntityFind {
    facade: EntityFacade,
    entity_name: String,
    conditions: Vec<Condition>,
    order: Vec<OrderBy>,
    limit: Option<i64>,
    offset: Option<i64>,
}

impl EntityFind {
    pub(crate) fn new(facade: EntityFacade, entity_name: String) -> Self {
        Self {
            facade,
            entity_name,
            conditions: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    pub fn eq(mut self, field: &str, value: Value) -> Self {
        self.conditions.push(Condition::equals(field, value));
        self
    }

    pub fn matches(mut self, field: &str, value: Value) -> Self {
        self.conditions.push(Condition::matches(field, value));
        self
    }

    pub fn condition(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    pub fn order_by(mut self, field: &str, descending: bool) -> Self {
        let entry = if descending {
            OrderBy::desc(field)
        } else {
            OrderBy::asc(field)
        };
        self.order.push(entry);
        self
    }

    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn list(self) -> Result<EntityList, Error> {
        let condition = self.combined();
        let lookup = self.lookup(&condition);
        if let Some(cached) = cache::before_find(&self.facade, &lookup) {
            return self.localize_list(cached);
        }
        let plan = self.plan(false)?;
        let rows = query_tx(&plan.tx, &plan.sql, &plan.params)?;
        after_statement(&self.facade, &plan.tx, false);
        let values = self.rows_to_values(&plan, &rows)?;
        let list = EntityList::new(values);
        cache::after_find(&self.facade, &lookup, &list)?;
        self.localize_list(list)
    }

    pub fn one(self) -> Result<Option<EntityValue>, Error> {
        let list = self.list()?;
        if list.len() > 1 {
            return Err(Error::new(format!("find one found {} rows", list.len())));
        }
        Ok(list.values().first().cloned())
    }

    pub fn count(self) -> Result<i64, Error> {
        let condition = self.combined();
        if let Some(count) = cache::before_count(&self.facade, &self.entity_name, condition.as_ref()) {
            return Ok(count);
        }
        let plan = self.plan(true)?;
        let rows = query_tx(&plan.tx, &plan.sql, &plan.params)?;
        after_statement(&self.facade, &plan.tx, false);
        let value = rows
            .first()
            .and_then(|row| row.columns.first())
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null);
        match basic_convert(&value, "Long").value {
            Value::I64(number) => Ok(number),
            other => Err(Error::new(format!("count returned {other:?}"))),
        }
    }

    /// Open a cursor when this find has no transaction hold and no sim schema.
    ///
    /// A hold or a sim schema is merged in memory. The cursor stays closed, so an open stream is
    /// not asked to mix in rows that are not in the database yet. The cursor path needs an active
    /// transaction: the open stream is that transaction's enlisted connection. Drop closes it.
    pub fn iterator(self) -> Result<EntityListIterator, Error> {
        let condition = self.combined();
        let lookup = self.lookup(&condition);
        if let Some(cached) = cache::before_find(&self.facade, &lookup) {
            return Ok(EntityListIterator::memory(self.facade, self.entity_name, cached));
        }
        let plan = self.plan(false)?;
        plan.tx
            .with_connection(|conn| conn.open_stream(&plan.sql, &plan.params))?;
        after_statement(&self.facade, &plan.tx, false);
        Ok(EntityListIterator::stream(
            self.facade,
            self.entity_name,
            plan.columns,
            plan.tx,
        ))
    }

    fn lookup(&self, condition: &Option<Condition>) -> FindLookup {
        FindLookup {
            entity_name: self.entity_name.clone(),
            condition: condition.clone(),
            order: self.order.clone(),
            limit: self.limit,
            offset: self.offset,
        }
    }

    fn combined(&self) -> Option<Condition> {
        let condition = match self.conditions.len() {
            0 => None,
            1 => Some(self.conditions[0].clone()),
            _ => Some(Condition::and(self.conditions.clone())),
        };
        if !authorization_filters() {
            return condition;
        }
        crate::user::apply_entity_filters(&self.entity_name, condition)
    }

    fn plan(&self, count_only: bool) -> Result<ReadPlan, Error> {
        let condition = self.combined();
        let page = LimitOffset {
            limit: self.limit,
            offset: self.offset,
        };
        self.facade.with_group(&self.entity_name, |entity, dialect, model, tx| {
            let (sql, columns) = if count_only {
                if entity.is_view {
                    return Err(Error::new(format!("cannot count view-entity {}", entity.full_name)));
                }
                (count(entity, dialect, condition.as_ref())?, Vec::new())
            } else if entity.is_view {
                (
                    select_view(entity, model, dialect, condition.as_ref(), &self.order, page)?,
                    view_columns(entity, model)?,
                )
            } else {
                let fields: Vec<FieldRef> = entity.fields.iter().map(|field| FieldRef::new(&field.name)).collect();
                (
                    select(entity, dialect, &fields, condition.as_ref(), &self.order, page)?,
                    entity_columns(entity, model),
                )
            };
            let mut params = condition.as_ref().map(condition_params).unwrap_or_default();
            if !count_only {
                params.extend(page_params(dialect, page));
            }
            Ok(ReadPlan {
                sql,
                params,
                columns,
                tx: tx.clone(),
            })
        })
    }

    fn rows_to_values(&self, plan: &ReadPlan, rows: &[DbRow]) -> Result<Vec<EntityValue>, Error> {
        let facade = self.facade.clone();
        self.facade.with_entity(&self.entity_name, |entity, _, _| {
            let mut values = Vec::with_capacity(rows.len());
            for row in rows {
                let mapped = map_row(&plan.columns, row);
                let adjusted = field_rw::decrypt_stored(&facade, entity, mapped)?;
                values.push(EntityValue::from_fields(entity.full_name.clone(), adjusted));
            }
            Ok(values)
        })
    }

    /// Localization uses the locale at read time. The list cache keeps the decrypted row.
    fn localize_list(&self, list: EntityList) -> Result<EntityList, Error> {
        let facade = self.facade.clone();
        self.facade.with_entity(&self.entity_name, |entity, _, _| {
            let mut values = Vec::with_capacity(list.len());
            for row in list.values() {
                let adjusted = field_rw::localize_stored(&facade, entity, row.fields().clone())?;
                values.push(EntityValue::from_fields(row.entity_name().to_string(), adjusted));
            }
            Ok(EntityList::new(values))
        })
    }
}

fn authorization_filters() -> bool {
    if !crate::user::context_entered() {
        return false;
    }
    crate::context::ExecutionContext::with_current(|ctx| !ctx.authz_disabled()).unwrap_or(false)
}

struct ReadPlan {
    sql: String,
    params: Vec<Value>,
    columns: Vec<(String, String)>,
    tx: TransactionManager,
}

/// A streaming find. Drop calls `close_stream` and releases the pinned connection.
pub struct EntityListIterator {
    facade: EntityFacade,
    entity_name: String,
    columns: Vec<(String, String)>,
    tx: Option<TransactionManager>,
    open: bool,
    memory: Vec<EntityValue>,
    index: usize,
}

impl EntityListIterator {
    fn stream(
        facade: EntityFacade,
        entity_name: String,
        columns: Vec<(String, String)>,
        tx: TransactionManager,
    ) -> Self {
        Self {
            facade,
            entity_name,
            columns,
            tx: Some(tx),
            open: true,
            memory: Vec::new(),
            index: 0,
        }
    }

    fn memory(facade: EntityFacade, entity_name: String, list: EntityList) -> Self {
        Self {
            facade,
            entity_name,
            columns: Vec::new(),
            tx: None,
            open: false,
            memory: list.values().to_vec(),
            index: 0,
        }
    }

    pub fn next_value(&mut self) -> Result<Option<EntityValue>, Error> {
        if !self.memory.is_empty() || !self.open {
            if self.index >= self.memory.len() {
                return Ok(None);
            }
            let value = self.memory[self.index].clone();
            self.index += 1;
            let facade = self.facade.clone();
            let adjusted = self.facade.with_entity(&self.entity_name, |entity, _, _| {
                field_rw::localize_stored(&facade, entity, value.fields().clone())
            })?;
            return Ok(Some(EntityValue::from_fields(
                value.entity_name().to_string(),
                adjusted,
            )));
        }
        let Some(tx) = self.tx.clone() else {
            return Ok(None);
        };
        let row = tx.with_connection(|conn| conn.next_row())?;
        match row {
            Some(row) => {
                let mapped = map_row(&self.columns, &row);
                let facade = self.facade.clone();
                let hook = facade.clone();
                let adjusted = facade.with_entity(&self.entity_name, |entity, _, _| {
                    field_rw::after_read(&hook, entity, mapped)
                })?;
                Ok(Some(EntityValue::from_fields(self.entity_name.clone(), adjusted)))
            }
            None => {
                self.close()?;
                Ok(None)
            }
        }
    }

    pub fn close(&mut self) -> Result<(), Error> {
        if !self.open {
            return Ok(());
        }
        let Some(tx) = self.tx.clone() else {
            self.open = false;
            return Ok(());
        };
        tx.with_connection(|conn| conn.close_stream())?;
        self.open = false;
        Ok(())
    }
}

impl Drop for EntityListIterator {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
