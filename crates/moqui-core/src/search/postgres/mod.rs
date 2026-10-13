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

//! The Postgres search backend.
//!
//! One table per data document in the `search` schema: `doc_id` (the record id), `indexed_at`,
//! `body` (the nested document as `jsonb`), a typed column `f_<name>` for each root field, and a
//! generated `all_text` vector over every string and number in the body. Child arrays stay in
//! `body`. `search.document_index` records each table and the signature of the shape it was made
//! with, so a definition change rebuilds the table.
//!
//! DDL runs on a reserve connection, autocommit. Reads run on the caller's connection, inside its
//! transaction when one is open. Writes join the caller's transaction or begin their own.

pub mod agg;
pub mod compile;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use jiff::Timestamp;
use moqui_lib::error::Error;
use moqui_lib::object::{to_plain_string, Value};

use compile::{column, column_type, ident, literal, typed_param, Compiler, Located, Sql, TS_CONFIG};

use super::highlight::highlights;
use super::mapping::{DocumentMapping, FieldKind};
use super::query::Query;
use super::{lock, Agg, SearchBackend, SearchHit, SearchPage, SearchRequest};
use crate::driver::DbRow;
use crate::entity::document::Document;
use crate::transaction::{TransactionAttribute, TransactionManager};

pub const DEFAULT_SCHEMA: &str = "search";

pub struct PostgresBackend {
    tx: TransactionManager,
    schema: String,
    trigram: bool,
    /// Signature of each table known to exist.
    known: Mutex<HashMap<String, String>>,
}

impl PostgresBackend {
    /// Open the store: create the schema, `pg_trgm` when it can, and the registry table.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema or the registry cannot be created.
    pub fn open(tx: TransactionManager, schema: Option<&str>) -> Result<Self, Error> {
        let schema = schema.unwrap_or(DEFAULT_SCHEMA).to_string();
        let quoted = ident(&schema);
        let trigram = tx.with_reserve(|conn| {
            conn.execute(&format!("CREATE SCHEMA IF NOT EXISTS {quoted}"), &[])?;
            let trigram = match conn.execute("CREATE EXTENSION IF NOT EXISTS pg_trgm", &[]) {
                Ok(_) => true,
                Err(err) => {
                    tracing::warn!(error = %err, "pg_trgm is not available; contains searches will scan");
                    false
                }
            };
            conn.execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {quoted}.document_index (document_id text PRIMARY KEY, \
                     index_name text, physical_name text NOT NULL, signature text NOT NULL, \
                     created_at timestamptz NOT NULL DEFAULT now())"
                ),
                &[],
            )?;
            Ok(trigram)
        })?;
        Ok(Self {
            tx,
            schema,
            trigram,
            known: Mutex::new(HashMap::new()),
        })
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    fn table(&self, mapping: &DocumentMapping) -> String {
        format!("{}.{}", ident(&self.schema), ident(&mapping.physical_name))
    }

    fn query(&self, sql: &Sql) -> Result<Vec<DbRow>, Error> {
        tracing::debug!(sql = %sql.text, "search query");
        self.tx.with_connection(|conn| conn.query(&sql.text, &sql.params))
    }

    fn registered(&self, document_id: &str) -> Result<Option<String>, Error> {
        if let Some(signature) = lock(&self.known).get(document_id) {
            return Ok(Some(signature.clone()));
        }
        let mut sql = Sql::new(format!(
            "SELECT signature FROM {}.document_index WHERE document_id = ",
            ident(&self.schema)
        ));
        sql.param(Value::string(document_id));
        let rows = self.tx.with_reserve(|conn| conn.query(&sql.text, &sql.params))?;
        let found = rows.first().and_then(|row| row.get("signature")).map(to_plain_string);
        if let Some(signature) = &found {
            lock(&self.known).insert(document_id.to_string(), signature.clone());
        }
        Ok(found)
    }

    fn create_statements(&self, mapping: &DocumentMapping) -> Vec<String> {
        let table = self.table(mapping);
        let mut columns = vec![
            "doc_id text PRIMARY KEY".to_string(),
            "indexed_at timestamptz NOT NULL".to_string(),
            "body jsonb NOT NULL".to_string(),
            format!(
                "all_text tsvector GENERATED ALWAYS AS (jsonb_to_tsvector('{TS_CONFIG}', body, '[\"string\",\"numeric\"]')) STORED"
            ),
        ];
        let mut indexes = vec![
            format!("CREATE INDEX ON {table} USING gin (all_text)"),
            format!("CREATE INDEX ON {table} (indexed_at)"),
        ];
        for (name, field) in &mapping.root.fields {
            let Some(sql_type) = column_type(field.kind) else {
                continue;
            };
            let col = column(name);
            columns.push(format!("{col} {sql_type}"));
            match field.kind {
                FieldKind::Text => {
                    indexes.push(format!(
                        "CREATE INDEX ON {table} USING gin (to_tsvector('{TS_CONFIG}', {col}))"
                    ));
                    if self.trigram {
                        indexes.push(format!("CREATE INDEX ON {table} USING gin ({col} gin_trgm_ops)"));
                    }
                }
                FieldKind::Keyword | FieldKind::Date | FieldKind::Long | FieldKind::Double => {
                    indexes.push(format!("CREATE INDEX ON {table} ({col})"));
                }
                FieldKind::Boolean | FieldKind::Binary => {}
            }
        }
        let mut out = vec![format!("CREATE TABLE {table} ({})", columns.join(", "))];
        out.extend(indexes);
        out
    }

    fn row_values(mapping: &DocumentMapping, body: &Document) -> Vec<(String, Option<(Value, &'static str)>)> {
        let mut out = Vec::new();
        for (name, field) in &mapping.root.fields {
            if column_type(field.kind).is_none() {
                continue;
            }
            let value = match body.get(name) {
                None | Some(Value::Null) | Some(Value::List(_)) | Some(Value::Map(_)) => None,
                Some(value) => typed_param(field.kind, value),
            };
            out.push((name.clone(), value));
        }
        out
    }
}

impl SearchBackend for PostgresBackend {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn ensure_index(&self, mapping: &DocumentMapping) -> Result<(), Error> {
        let signature = mapping.signature_hash();
        let current = self.registered(&mapping.document_id)?;
        if current.as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        let table = self.table(mapping);
        let registry = format!("{}.document_index", ident(&self.schema));
        let statements = self.create_statements(mapping);
        self.tx.with_reserve(|conn| {
            conn.begin()?;
            let result = (|| {
                if current.is_some() {
                    tracing::warn!(
                        document = %mapping.document_id,
                        "search store shape changed; the table is rebuilt and must be reindexed"
                    );
                }
                conn.execute(&format!("DROP TABLE IF EXISTS {table}"), &[])?;
                for statement in &statements {
                    conn.execute(statement, &[])?;
                }
                conn.execute(
                    &format!(
                        "INSERT INTO {registry} (document_id, index_name, physical_name, signature) VALUES (?, ?, ?, ?) \
                         ON CONFLICT (document_id) DO UPDATE SET index_name = EXCLUDED.index_name, \
                         physical_name = EXCLUDED.physical_name, signature = EXCLUDED.signature, created_at = now()"
                    ),
                    &[
                        Value::string(&mapping.document_id),
                        mapping.index_name.clone().map_or(Value::Null, Value::String),
                        Value::string(&mapping.physical_name),
                        Value::string(&signature),
                    ],
                )?;
                Ok(())
            })();
            match result {
                Ok(()) => conn.commit(),
                Err(err) => {
                    let _ = conn.rollback();
                    Err(err)
                }
            }
        })?;
        lock(&self.known).insert(mapping.document_id.clone(), signature);
        Ok(())
    }

    fn index_exists(&self, mapping: &DocumentMapping) -> Result<bool, Error> {
        Ok(self.registered(&mapping.document_id)?.is_some())
    }

    fn index_documents(&self, mapping: &DocumentMapping, documents: &[(String, Document)]) -> Result<usize, Error> {
        if documents.is_empty() {
            return Ok(0);
        }
        let table = self.table(mapping);
        let names: Vec<String> = Self::row_values(mapping, &Document::new())
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let mut statement = format!("INSERT INTO {table} (doc_id, indexed_at, body");
        for name in &names {
            statement.push_str(", ");
            statement.push_str(&column(name));
        }
        statement.push_str(") VALUES (?, ?::timestamptz, ?::jsonb");
        let mut casts = Vec::new();
        for (name, field) in &mapping.root.fields {
            if column_type(field.kind).is_none() {
                continue;
            }
            let cast = match field.kind {
                FieldKind::Long => "::bigint",
                FieldKind::Double => "::numeric",
                FieldKind::Date => "::timestamptz",
                FieldKind::Boolean => "::boolean",
                _ => "",
            };
            let _ = name;
            casts.push(cast);
            statement.push_str(&format!(", ?{cast}"));
        }
        statement
            .push_str(") ON CONFLICT (doc_id) DO UPDATE SET indexed_at = EXCLUDED.indexed_at, body = EXCLUDED.body");
        for name in &names {
            let col = column(name);
            statement.push_str(&format!(", {col} = EXCLUDED.{col}"));
        }
        let now = Timestamp::now();
        let mut rows = Vec::with_capacity(documents.len());
        for (id, body) in documents {
            let json = serde_json::to_string(&body_json(&Value::Map(body.clone())))
                .map_err(|err| Error::new(format!("document {id} is not JSON: {err}")))?;
            let mut params = vec![Value::string(id), Value::Timestamp(now), Value::String(json)];
            for ((_, value), cast) in Self::row_values(mapping, body).into_iter().zip(&casts) {
                params.push(match value {
                    // A long column takes a whole number; numeric text like 3.0 would not cast.
                    Some((Value::Decimal(decimal), _)) if *cast == "::bigint" => {
                        Value::String(decimal.trunc().normalize().to_string())
                    }
                    Some((value, _)) => value,
                    None => Value::Null,
                });
            }
            rows.push(params);
        }
        self.tx.run(TransactionAttribute::UseOrBegin, None, || {
            self.tx.with_connection(|conn| {
                for params in &rows {
                    conn.execute(&statement, params)?;
                }
                Ok(())
            })
        })?;
        Ok(documents.len())
    }

    fn delete_document(&self, mapping: &DocumentMapping, id: &str) -> Result<bool, Error> {
        let table = self.table(mapping);
        let count = self.tx.run(TransactionAttribute::UseOrBegin, None, || {
            self.tx.with_connection(|conn| {
                conn.execute(&format!("DELETE FROM {table} WHERE doc_id = ?"), &[Value::string(id)])
            })
        })?;
        Ok(count > 0)
    }

    fn delete_matching(&self, mapping: &DocumentMapping, filter: &Query) -> Result<u64, Error> {
        let compiler = Compiler::new(mapping);
        let mut sql = Sql::new(format!("DELETE FROM {} WHERE ", self.table(mapping)));
        sql.append(compiler.where_sql(filter)?);
        self.tx.run(TransactionAttribute::UseOrBegin, None, || {
            self.tx.with_connection(|conn| conn.execute(&sql.text, &sql.params))
        })
    }

    fn get_document(&self, mapping: &DocumentMapping, id: &str) -> Result<Option<Document>, Error> {
        let mut sql = Sql::new(format!(
            "SELECT body::text AS body FROM {} WHERE doc_id = ",
            self.table(mapping)
        ));
        sql.param(Value::string(id));
        let rows = self.query(&sql)?;
        rows.first().map(|row| parse_body(row.get("body"))).transpose()
    }

    fn search(&self, mappings: &[Arc<DocumentMapping>], request: &SearchRequest) -> Result<SearchPage, Error> {
        let plan = search_sql(&self.schema, mappings, request)?;
        let total = self
            .query(&plan.count)?
            .first()
            .and_then(|row| row.get("total"))
            .map(|value| to_plain_string(value).parse::<u64>().unwrap_or(0))
            .unwrap_or(0);
        let mut hits = Vec::new();
        if request.limit > 0 && total > 0 {
            for row in self.query(&plan.select)? {
                let body = parse_body(row.get("body"))?;
                let highlights = if request.highlight.is_empty() {
                    IndexMap::new()
                } else {
                    highlights(&request.query, &request.highlight, &body)
                };
                hits.push(SearchHit {
                    document_id: row.get("document_id").map(to_plain_string).unwrap_or_default(),
                    id: row.get("doc_id").map(to_plain_string).unwrap_or_default(),
                    index: row.get("index_name").map(to_plain_string).unwrap_or_default(),
                    score: row
                        .get("score")
                        .map(|value| to_plain_string(value).parse::<f64>().unwrap_or(0.0))
                        .unwrap_or(0.0),
                    body,
                    highlights,
                });
            }
        }
        Ok(SearchPage { hits, total })
    }

    fn aggregate(
        &self,
        mapping: &DocumentMapping,
        filter: &Query,
        aggs: &IndexMap<String, Agg>,
    ) -> Result<IndexMap<String, Value>, Error> {
        let table = self.table(mapping);
        let mut run = |sql: &Sql| self.query(sql);
        agg::aggregate(&table, mapping, filter, aggs, &mut run)
    }

    fn drop_index(&self, mapping: &DocumentMapping) -> Result<(), Error> {
        let table = self.table(mapping);
        let registry = format!("{}.document_index", ident(&self.schema));
        self.tx.with_reserve(|conn| {
            conn.execute(&format!("DROP TABLE IF EXISTS {table}"), &[])?;
            conn.execute(
                &format!("DELETE FROM {registry} WHERE document_id = ?"),
                &[Value::string(&mapping.document_id)],
            )?;
            Ok(())
        })?;
        lock(&self.known).remove(&mapping.document_id);
        Ok(())
    }
}

/// The page statement and the count statement for a search.
pub struct SearchPlan {
    pub select: Sql,
    pub count: Sql,
}

/// Build the search statements. One document is one `SELECT`; several are a `UNION ALL`.
///
/// # Errors
///
/// Returns an error for a sort on a text field that is not sortable, or a value that does not fit
/// its field.
pub fn search_sql(
    schema: &str,
    mappings: &[Arc<DocumentMapping>],
    request: &SearchRequest,
) -> Result<SearchPlan, Error> {
    // The kind of each sort key, from the first document that has the field.
    let mut sort_kinds: Vec<Option<FieldKind>> = Vec::new();
    for sort in &request.sort {
        let mut kind = None;
        for mapping in mappings {
            match sort.field.as_str() {
                "_score" | "_id" => break,
                _ => {}
            }
            let compiler = Compiler::new(mapping);
            if let Located::Field { field, .. } = compiler.locate_root(&sort.field) {
                if !field.can_sort() {
                    return Err(Error::new(format!(
                        "{} is a text field that is not sortable",
                        sort.field
                    )));
                }
                kind = match kind {
                    None => Some(field.kind),
                    Some(existing) if existing == field.kind => Some(existing),
                    Some(_) => Some(FieldKind::Keyword),
                };
            }
        }
        sort_kinds.push(kind);
    }

    let mut parts = Vec::new();
    let mut counts = Vec::new();
    for mapping in mappings {
        let compiler = Compiler::new(mapping);
        let table = format!("{}.{}", ident(schema), ident(&mapping.physical_name));
        let where_sql = compiler.where_sql(&request.query)?;
        let mut part = Sql::new(format!(
            "SELECT {} AS document_id, {} AS index_name, doc_id, body::text AS body, ",
            literal(&mapping.document_id),
            literal(&mapping.physical_name)
        ));
        match compiler.rank_query(&request.query) {
            Some(rank) => {
                part.push("ts_rank(all_text, ");
                part.append(rank);
                part.push(")::float8 AS score");
            }
            None => {
                part.push("0::float8 AS score");
            }
        }
        for (index, (sort, kind)) in request.sort.iter().zip(&sort_kinds).enumerate() {
            let expr = sort_expr(&compiler, sort.field.as_str(), sort.descending, *kind);
            part.push(&format!(", {expr} AS s{index}"));
        }
        part.push(&format!(" FROM {table} WHERE "));
        part.append(where_sql.clone());
        parts.push(part);

        let mut count = Sql::new(format!("(SELECT count(*) FROM {table} WHERE "));
        count.append(where_sql);
        count.push(")");
        counts.push(count);
    }

    let mut order = Vec::new();
    for (index, sort) in request.sort.iter().enumerate() {
        let direction = if sort.descending { "DESC" } else { "ASC" };
        order.push(format!("s{index} {direction} NULLS LAST"));
    }
    if request.sort.is_empty() {
        order.push("score DESC".to_string());
    }
    order.push("document_id ASC".to_string());
    order.push("doc_id ASC".to_string());

    let mut select = if parts.len() == 1 {
        parts.pop().unwrap_or_default()
    } else {
        let mut union = Sql::new("SELECT * FROM (");
        for (index, part) in parts.into_iter().enumerate() {
            if index > 0 {
                union.push(" UNION ALL ");
            }
            union.append(part);
        }
        union.push(") AS u");
        union
    };
    select.push(&format!(
        " ORDER BY {} LIMIT {} OFFSET {}",
        order.join(", "),
        request.limit,
        request.offset
    ));

    let mut count = Sql::new("SELECT ");
    for (index, part) in counts.into_iter().enumerate() {
        if index > 0 {
            count.push(" + ");
        }
        count.append(part);
    }
    count.push(" AS total");
    Ok(SearchPlan { select, count })
}

fn sort_expr(compiler: &Compiler<'_>, field: &str, descending: bool, kind: Option<FieldKind>) -> String {
    let null_of = |kind: Option<FieldKind>| match kind.and_then(column_type) {
        Some(sql_type) => format!("NULL::{sql_type}"),
        None => "NULL::text".to_string(),
    };
    match field {
        "_score" => return "score".to_string(),
        "_id" => return "doc_id".to_string(),
        _ => {}
    }
    match compiler.locate_root(field) {
        Located::Missing => null_of(kind),
        Located::Field {
            arrays,
            expr,
            field: found,
        } => {
            let expr = if kind.is_some_and(|kind| kind != found.kind) {
                format!("({expr})::text")
            } else {
                expr
            };
            if arrays.is_empty() {
                expr
            } else {
                // A child array sorts by its smallest value ascending and its largest descending.
                let function = if descending { "max" } else { "min" };
                let from = arrays
                    .iter()
                    .map(|(var, source)| format!("{source} AS {var}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("(SELECT {function}({expr}) FROM {from})")
            }
        }
    }
}

fn parse_body(value: Option<&Value>) -> Result<Document, Error> {
    let text = value.map(to_plain_string).unwrap_or_default();
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| Error::new(format!("stored document is not JSON: {err}")))?;
    match moqui_lib::json::from_json(&json) {
        Value::Map(map) => Ok(map),
        _ => Err(Error::new("stored document is not an object")),
    }
}

/// JSON for a document body. Timestamps are ISO-8601 UTC, dates are `yyyy-MM-dd`, decimals
/// stay exact.
pub fn body_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Timestamp(at) => serde_json::Value::String(at.to_string()),
        Value::Date(date) => serde_json::Value::String(date.to_string()),
        Value::Time(time) => serde_json::Value::String(time.to_string()),
        Value::List(items) => serde_json::Value::Array(items.iter().map(body_json).collect()),
        Value::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .filter(|(_, item)| !item.is_null())
                .map(|(key, item)| (key.clone(), body_json(item)))
                .collect(),
        ),
        other => moqui_lib::json::to_json(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::mapping::{FieldMapping, ObjectMapping};
    use crate::search::{SortField, Target};

    fn mapping(id: &str, physical: &str) -> Arc<DocumentMapping> {
        let mut root = ObjectMapping::default();
        root.fields.insert(
            "combinedName".into(),
            FieldMapping {
                kind: FieldKind::Text,
                sortable: true,
            },
        );
        let mut accounts = ObjectMapping::default();
        accounts.fields.insert(
            "username".into(),
            FieldMapping {
                kind: FieldKind::Text,
                sortable: true,
            },
        );
        root.children.insert("userAccounts".into(), accounts);
        Arc::new(DocumentMapping {
            document_id: id.into(),
            index_name: Some("mantle".into()),
            physical_name: physical.into(),
            root,
            manual_mapping_service: None,
        })
    }

    #[test]
    fn union_search_sorts_child_by_max_descending() {
        let mappings = vec![
            mapping("MantleParty", "mantle_party"),
            mapping("MantleProduct", "mantle_product"),
        ];
        let mut request = SearchRequest::new(crate::search::parse::parse_query_string("*:jo*").unwrap());
        request.sort = vec![SortField::parse("-userAccounts.username.keyword").unwrap()];
        request.limit = 20;
        let plan = search_sql("search", &mappings, &request).unwrap();
        assert!(
            plan.select
                .text
                .starts_with("SELECT * FROM (SELECT 'MantleParty' AS document_id"),
            "{}",
            plan.select.text
        );
        assert!(plan.select.text.contains(
            "(SELECT max((e0 ->> 'username')) FROM jsonb_array_elements(body -> 'userAccounts') AS e0) AS s0"
        ));
        assert!(plan
            .select
            .text
            .ends_with("ORDER BY s0 DESC NULLS LAST, document_id ASC, doc_id ASC LIMIT 20 OFFSET 0"));
        assert_eq!(plan.select.params.len(), 4);
        assert!(plan
            .count
            .text
            .starts_with("SELECT (SELECT count(*) FROM \"search\".\"mantle_party\" WHERE"));
        let _ = Target::Index("mantle".into());
    }

    #[test]
    fn body_json_keeps_dates_and_drops_nulls() {
        let mut body = Document::new();
        body.insert(
            "placedDate".into(),
            Value::Timestamp("2026-01-05T10:00:00Z".parse().unwrap()),
        );
        body.insert("missing".into(), Value::Null);
        let json = body_json(&Value::Map(body));
        assert_eq!(json.to_string(), r#"{"placedDate":"2026-01-05T10:00:00Z"}"#);
    }
}
