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

//! Check and create tables, columns, indexes, foreign keys, and text-fts indexes.
//! Runtime DDL uses a reserve connection.

use std::collections::{HashMap, HashSet};

use moqui_db::dialect::Dialect;
use moqui_db::sql::{add_column, create_foreign_key, create_index, create_table, quote_ident};
use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::exec::{already_exists, execute_conn, execute_script, query_conn, query_tx};
use super::facade::EntityFacade;
use crate::driver::DbConnection;
use crate::transaction::TransactionManager;

/// Database metadata for one facade.
pub struct EntityDbMeta<'a> {
    facade: &'a EntityFacade,
}

impl EntityFacade {
    pub fn meta(&self) -> EntityDbMeta<'_> {
        EntityDbMeta { facade: self }
    }

    pub fn check_tables(&self, names: &[&str]) -> Result<(), Error> {
        self.meta().check(names)
    }

    pub fn check_all_tables(&self) -> Result<(), Error> {
        self.meta().check_all()
    }

    pub fn table_exists(&self, entity_name: &str) -> Result<bool, Error> {
        let (probe, tx) = self.with_group(entity_name, |entity, dialect, _, tx| {
            Ok((table_probe(dialect, &entity.table_name), tx.clone()))
        })?;
        let rows = query_tx(&tx, &probe.sql, &probe.params)?;
        Ok(rows.iter().any(|row| row_matches(&row.columns, &probe.table)))
    }
}

impl EntityDbMeta<'_> {
    /// Create tables for these entities. View-entities expand to their member entities.
    pub fn check(&self, names: &[&str]) -> Result<(), Error> {
        self.run(names, false)
    }

    /// Create every non-view entity. A foreign key that cannot be created is logged and skipped.
    pub fn check_all(&self) -> Result<(), Error> {
        let owned: Vec<String> = self
            .facade
            .inner
            .model
            .model()
            .entities()
            .iter()
            .filter(|entity| !entity.is_view)
            .map(|entity| entity.full_name.clone())
            .collect();
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.run(&refs, true)
    }

    fn run(&self, names: &[&str], lenient_fk: bool) -> Result<(), Error> {
        let concrete = expand(self.facade, names)?;
        let default_group = self.facade.inner.default_group.clone();
        let mut batches: HashMap<String, Batch> = HashMap::new();
        for name in concrete {
            let (group, tx) = self.facade.with_group(&name, |entity, _, _, tx| {
                Ok((
                    entity.group.clone().unwrap_or_else(|| default_group.clone()),
                    tx.clone(),
                ))
            })?;
            match batches.entry(group) {
                std::collections::hash_map::Entry::Occupied(mut entry) => entry.get_mut().names.push(name),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(Batch { tx, names: vec![name] });
                }
            }
        }
        let skip_fk = self.facade.flags().disable_fk_create;
        for batch in batches.into_values() {
            let facade = self.facade;
            let names = batch.names;
            batch
                .tx
                .with_reserve(|conn| apply(facade, conn, &names, skip_fk, lenient_fk))?;
        }
        Ok(())
    }
}

struct Batch {
    tx: TransactionManager,
    names: Vec<String>,
}

struct Probe {
    sql: String,
    params: Vec<Value>,
    table: String,
}

fn expand(facade: &EntityFacade, names: &[&str]) -> Result<Vec<String>, Error> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in names {
        collect(facade, name, &mut out, &mut seen)?;
    }
    Ok(out)
}

fn collect(facade: &EntityFacade, name: &str, out: &mut Vec<String>, seen: &mut HashSet<String>) -> Result<(), Error> {
    let (is_view, full_name, members) = facade.with_entity(name, |entity, _, _| {
        let members = if entity.is_view {
            entity
                .member_entities
                .iter()
                .map(|member| member.entity_name.clone())
                .collect()
        } else {
            Vec::new()
        };
        Ok((entity.is_view, entity.full_name.clone(), members))
    })?;
    if is_view {
        for member in members {
            collect(facade, &member, out, seen)?;
        }
        return Ok(());
    }
    if seen.insert(full_name.clone()) {
        out.push(full_name);
    }
    Ok(())
}

fn apply(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    names: &[String],
    skip_fk: bool,
    lenient_fk: bool,
) -> Result<(), Error> {
    for name in names {
        let sql = facade.with_entity(name, |entity, dialect, model| {
            if table_exists_conn(conn, dialect, &entity.table_name)? {
                Ok(None)
            } else {
                Ok(Some(create_table(entity, model, dialect)?))
            }
        })?;
        if let Some(sql) = sql {
            execute_script(conn, &sql)?;
        }
    }
    for name in names {
        ensure_columns(facade, conn, name)?;
        ensure_indexes(facade, conn, name)?;
        ensure_fts(facade, conn, name)?;
    }
    if !skip_fk {
        for name in names {
            ensure_fks(facade, conn, name, lenient_fk)?;
        }
    }
    Ok(())
}

fn ensure_columns(facade: &EntityFacade, conn: &mut dyn DbConnection, name: &str) -> Result<(), Error> {
    let statements = facade.with_entity(name, |entity, dialect, model| {
        let present = column_names(conn, dialect, &entity.table_name)?;
        let mut statements = Vec::new();
        for field in &entity.fields {
            if has_name(&present, &field.name) {
                continue;
            }
            statements.push(add_column(entity, field, model, dialect)?);
        }
        Ok(statements)
    })?;
    for sql in statements {
        run_statement(conn, &sql, false)?;
    }
    Ok(())
}

fn ensure_indexes(facade: &EntityFacade, conn: &mut dyn DbConnection, name: &str) -> Result<(), Error> {
    let statements = facade.with_entity(name, |entity, dialect, _| {
        let mut statements = Vec::new();
        for index in &entity.indexes {
            statements.push(create_index(entity, index, dialect)?);
        }
        Ok(statements)
    })?;
    for sql in statements {
        run_statement(conn, &sql, false)?;
    }
    Ok(())
}

fn ensure_fts(facade: &EntityFacade, conn: &mut dyn DbConnection, name: &str) -> Result<(), Error> {
    let statements = facade.with_entity(name, |entity, dialect, _| {
        let mut statements = Vec::new();
        for field in entity.fields.iter().filter(|field| field.field_type == "text-fts") {
            if let Some(sql) = fts_sql(entity, &field.name, dialect)? {
                statements.push(sql);
            }
        }
        Ok(statements)
    })?;
    for sql in statements {
        run_statement(conn, &sql, false)?;
    }
    Ok(())
}

fn ensure_fks(facade: &EntityFacade, conn: &mut dyn DbConnection, name: &str, lenient: bool) -> Result<(), Error> {
    let statements = facade.with_entity(name, |entity, dialect, model| {
        let mut statements = Vec::new();
        for relationship in &entity.relationships {
            if relationship.is_auto_reverse || relationship.relationship_type != "one" {
                continue;
            }
            let related = model.entity(&relationship.related)?;
            if related.is_view || !table_exists_conn(conn, dialect, &related.table_name)? {
                continue;
            }
            statements.push(create_foreign_key(entity, relationship, related, dialect)?);
        }
        Ok(statements)
    })?;
    for sql in statements {
        run_statement(conn, &sql, lenient)?;
    }
    Ok(())
}

fn run_statement(conn: &mut dyn DbConnection, sql: &str, lenient: bool) -> Result<(), Error> {
    match execute_conn(conn, sql, &[]) {
        Ok(_) => Ok(()),
        Err(err) if already_exists(&err) => Ok(()),
        Err(err) if lenient => {
            tracing::warn!(%err, sql = %sql, "foreign key was not created");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn table_exists_conn(conn: &mut dyn DbConnection, dialect: &Dialect, table: &str) -> Result<bool, Error> {
    let probe = table_probe(dialect, table);
    let rows = query_conn(conn, &probe.sql, &probe.params)?;
    Ok(rows.iter().any(|row| row_matches(&row.columns, table)))
}

fn column_names(conn: &mut dyn DbConnection, dialect: &Dialect, table: &str) -> Result<Vec<String>, Error> {
    let probe = column_probe(dialect, table);
    let rows = query_conn(conn, &probe.sql, &probe.params)?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(Value::String(name)) = row.columns.first().map(|(_, value)| value) {
            names.push(name.clone());
        }
    }
    Ok(names)
}

fn table_probe(dialect: &Dialect, table: &str) -> Probe {
    Probe {
        sql: format!(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = {} AND lower(table_name) = lower(?)",
            schema_expr(dialect)
        ),
        params: vec![Value::string(table)],
        table: table.to_string(),
    }
}

fn column_probe(dialect: &Dialect, table: &str) -> Probe {
    Probe {
        sql: format!(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = {} AND lower(table_name) = lower(?)",
            schema_expr(dialect)
        ),
        params: vec![Value::string(table)],
        table: table.to_string(),
    }
}

fn schema_expr(dialect: &Dialect) -> &'static str {
    if dialect.name.len() >= 5 && dialect.name[..5].eq_ignore_ascii_case("mysql") {
        "database()"
    } else {
        "current_schema()"
    }
}

fn row_matches(columns: &[(String, Value)], table: &str) -> bool {
    let Some(Value::String(name)) = columns.first().map(|(_, value)| value) else {
        return false;
    };
    name == table || name.eq_ignore_ascii_case(table)
}

fn has_name(names: &[String], want: &str) -> bool {
    names.iter().any(|name| name == want) || names.iter().any(|name| name.eq_ignore_ascii_case(want))
}

fn fts_sql(
    entity: &moqui_db::model::EntityDefinition,
    field: &str,
    dialect: &Dialect,
) -> Result<Option<String>, Error> {
    let index_name = clip(
        &format!("{}_{}_FTS", entity.table_name, field),
        dialect.constraint_name_clip_length,
    );
    match dialect.fts_style.as_deref() {
        Some("postgres-tsvector") => {
            let config = dialect.fts_config.as_deref().unwrap_or("english");
            if config.contains('\'') {
                return Err(Error::new("fts-config contains a quote"));
            }
            Ok(Some(format!(
                "CREATE INDEX {} ON {} USING gin (to_tsvector('{config}', {}))",
                quote_ident(dialect, &index_name),
                quote_ident(dialect, &entity.table_name),
                quote_ident(dialect, field),
            )))
        }
        Some("mysql-fulltext") => Ok(Some(format!(
            "ALTER TABLE {} ADD FULLTEXT INDEX {} ({})",
            quote_ident(dialect, &entity.table_name),
            quote_ident(dialect, &index_name),
            quote_ident(dialect, field),
        ))),
        _ => Ok(None),
    }
}

fn clip(name: &str, limit: Option<u32>) -> String {
    let Some(limit) = limit else {
        return name.to_string();
    };
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if name.len() <= limit {
        return name.to_string();
    }
    let mut end = limit;
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}
