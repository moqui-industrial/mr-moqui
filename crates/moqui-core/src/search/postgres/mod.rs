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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use jiff::Timestamp;
use moqui_lib::error::Error;
use moqui_lib::object::{to_plain_string, Value};

use compile::{
    column, column_type, ident, literal, typed_param, Compiler, Located, Sql, TextSetup, NORM_TITLE, RANKED_TEXT,
};

use super::highlight::highlights;
use super::mapping::{DocumentMapping, FieldKind};
use super::query::Query;
use super::{lock, Agg, Capability, Rank, ReplicaStatus, SearchBackend, SearchHit, SearchPage, SearchRequest};
use crate::driver::DbRow;
use crate::entity::document::Document;
use crate::transaction::{TransactionAttribute, TransactionManager};

pub const DEFAULT_SCHEMA: &str = "search";

pub struct PostgresBackend {
    tx: TransactionManager,
    /// A physical replica of the store for searches, aggregations, and `get_document`.
    read: Option<TransactionManager>,
    /// Reads that ran on the primary because a fresh read timed out or the replica failed.
    fallbacks: AtomicU64,
    schema: String,
    trigram: bool,
    /// The text configuration and title fold the tables are made with.
    text: TextSetup,
    /// True when `text` folds accents (`unaccent`).
    accent: bool,
    /// Embedding storage, when semantic search is enabled and `pgvector` is installed.
    vectors: Option<VectorStore>,
    /// Signature of each table known to exist.
    known: Mutex<HashMap<String, String>>,
    /// One `ensure_index` at a time in this process; the advisory lock covers other processes.
    ensuring: Mutex<()>,
}

/// How embeddings are stored: `halfvec` or `vector`, of `dimensions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorStore {
    pub representation: String,
    pub dimensions: usize,
}

impl VectorStore {
    fn column_type(&self) -> String {
        format!("{}({})", self.representation, self.dimensions)
    }
}

/// `[0.1,0.2,..]`, the text form `pgvector` reads.
pub fn vector_literal(vector: &[f32]) -> String {
    let parts: Vec<String> = vector.iter().map(|value| format!("{value}")).collect();
    format!("[{}]", parts.join(","))
}

/// The nearest-neighbor statement: per document type, the closest `k` embedded with `model` that
/// match `filter`, cosine distance on the HNSW index, then the closest `k` of those.
///
/// # Errors
///
/// Returns an error when the filter does not compile against a document.
#[allow(clippy::too_many_arguments)]
pub fn semantic_sql(
    schema: &str,
    text: &TextSetup,
    store: &VectorStore,
    mappings: &[Arc<DocumentMapping>],
    vector: &[f32],
    model: &str,
    filter: Option<&Query>,
    k: usize,
) -> Result<Sql, Error> {
    let cast = store.column_type();
    let literal_vector = vector_literal(vector);
    let mut parts = Vec::new();
    for mapping in mappings {
        let compiler = Compiler::with_text(mapping, text);
        let table = format!("{}.{}", ident(schema), ident(&mapping.physical_name));
        let mut part = Sql::new(format!(
            "(SELECT {} AS document_id, {} AS index_name, doc_id, body::text AS body, (1 - (embedding <=> ",
            literal(&mapping.document_id),
            literal(&mapping.physical_name)
        ));
        part.param(Value::string(&literal_vector))
            .push(&format!("::{cast}))::float8 AS score FROM {table} WHERE embedding IS NOT NULL AND embedding_model = "));
        part.param(Value::string(model));
        if let Some(filter) = filter {
            part.push(" AND ");
            part.append(compiler.where_sql(filter)?);
        }
        part.push(" ORDER BY embedding <=> ");
        part.param(Value::string(&literal_vector))
            .push(&format!("::{cast} LIMIT {k})"));
        parts.push(part);
    }
    let mut out = Sql::new("SELECT * FROM (");
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            out.push(" UNION ALL ");
        }
        out.append(part);
    }
    out.push(&format!(") AS s ORDER BY score DESC, document_id ASC, doc_id ASC LIMIT {k}"));
    Ok(out)
}

/// Where one read runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Primary,
    Replica,
}

/// How often a fresh read asks the replica how far it has replayed.
const REPLAY_POLL: Duration = Duration::from_millis(20);

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
            read: None,
            fallbacks: AtomicU64::new(0),
            schema,
            trigram,
            text: TextSetup::default(),
            accent: false,
            vectors: None,
            known: Mutex::new(HashMap::new()),
            ensuring: Mutex::new(()),
        })
    }

    /// Store embeddings when `conf` enables semantic search and the `pgvector` extension can be
    /// created; otherwise a warning, and no semantic capability. Enabling it rebuilds the tables.
    ///
    /// # Errors
    ///
    /// Returns an error when the conf is enabled with zero dimensions.
    pub fn with_semantic(mut self, conf: &crate::search::semantic::SemanticConf) -> Result<Self, Error> {
        if !conf.enabled {
            return Ok(self);
        }
        if conf.dimensions == 0 {
            return Err(Error::new("semantic search needs dimensions above 0"));
        }
        let created = self
            .tx
            .with_reserve(|conn| conn.execute("CREATE EXTENSION IF NOT EXISTS vector", &[]));
        match created {
            Ok(_) => {
                self.vectors = Some(VectorStore {
                    representation: conf.representation.clone(),
                    dimensions: conf.dimensions,
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, "pgvector is not available; semantic search is off");
            }
        }
        Ok(self)
    }

    /// Use the text search configuration `config` (default `simple`), folding accents when
    /// `unaccent` is set: a copy of the configuration with `unaccent` before its word dictionaries,
    /// and a lower-and-unaccent function for titles, both in the store's schema. When the
    /// `unaccent` extension cannot be created, accents are not folded and a warning says so. A
    /// change of either rebuilds every table on its next `ensure_index`.
    ///
    /// # Errors
    ///
    /// Returns an error when `config` does not exist or the configuration cannot be made.
    pub fn with_analysis(mut self, config: Option<&str>, unaccent: bool) -> Result<Self, Error> {
        let base = config.unwrap_or(compile::TS_CONFIG).to_string();
        let quoted = ident(&self.schema);
        let schema = self.schema.clone();
        let setup = self.tx.with_reserve(|conn| {
            let found = conn.query("SELECT 1 FROM pg_ts_config WHERE cfgname = ?", &[Value::string(&base)])?;
            if found.is_empty() {
                return Err(Error::new(format!("text search configuration {base} does not exist")));
            }
            if !unaccent {
                return Ok((
                    TextSetup {
                        config: base.clone(),
                        fold: "lower".to_string(),
                    },
                    false,
                ));
            }
            if let Err(err) = conn.execute("CREATE EXTENSION IF NOT EXISTS unaccent", &[]) {
                tracing::warn!(error = %err, "unaccent is not available; search does not fold accents");
                return Ok((
                    TextSetup {
                        config: base.clone(),
                        fold: "lower".to_string(),
                    },
                    false,
                ));
            }
            let rows = conn.query(
                "SELECT n.nspname AS schema FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace \
                 WHERE e.extname = 'unaccent'",
                &[],
            )?;
            let ext_schema = rows
                .first()
                .and_then(|row| row.get("schema"))
                .map(to_plain_string)
                .unwrap_or_else(|| "public".to_string());
            let ext = ident(&ext_schema);
            let name = format!("moqui_unaccent_{}", base.replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"));
            let exists = conn.query(
                "SELECT 1 FROM pg_ts_config c JOIN pg_namespace n ON n.oid = c.cfgnamespace \
                 WHERE c.cfgname = ? AND n.nspname = ?",
                &[Value::string(&name), Value::string(&schema)],
            )?;
            if exists.is_empty() {
                // The dictionaries the base configuration gives words, after unaccent.
                let dicts = conn.query(
                    "SELECT d.dictname AS dict FROM pg_ts_config c JOIN pg_ts_config_map m ON m.mapcfg = c.oid \
                     JOIN pg_ts_dict d ON d.oid = m.mapdict \
                     WHERE c.cfgname = ? AND m.maptokentype = (SELECT tokid FROM ts_token_type('default') WHERE alias = 'word') \
                     ORDER BY m.mapseqno",
                    &[Value::string(&base)],
                )?;
                let dicts: Vec<String> = dicts
                    .iter()
                    .filter_map(|row| row.get("dict").map(to_plain_string))
                    .collect();
                let dicts = if dicts.is_empty() { vec!["simple".to_string()] } else { dicts };
                conn.execute(
                    &format!("CREATE TEXT SEARCH CONFIGURATION {quoted}.{} (COPY = {})", ident(&name), ident(&base)),
                    &[],
                )?;
                conn.execute(
                    &format!(
                        "ALTER TEXT SEARCH CONFIGURATION {quoted}.{} ALTER MAPPING FOR hword, hword_part, word WITH {ext}.unaccent, {}",
                        ident(&name),
                        dicts.iter().map(|dict| ident(dict)).collect::<Vec<_>>().join(", ")
                    ),
                    &[],
                )?;
            }
            // unaccent() is only stable; a generated column needs an immutable function.
            conn.execute(
                &format!(
                    "CREATE OR REPLACE FUNCTION {quoted}.moqui_fold(text) RETURNS text LANGUAGE sql IMMUTABLE PARALLEL SAFE \
                     AS $$ SELECT lower({ext}.unaccent({}::regdictionary, $1)) $$",
                    literal(&format!("{ext_schema}.unaccent"))
                ),
                &[],
            )?;
            Ok((
                TextSetup {
                    config: format!("{schema}.{name}"),
                    fold: format!("{quoted}.moqui_fold"),
                },
                true,
            ))
        })?;
        self.text = setup.0;
        self.accent = setup.1;
        Ok(self)
    }

    /// The registry signature: the stored shape and everything a table derives from text setup.
    fn signature(&self, mapping: &DocumentMapping) -> String {
        let text = format!(
            "{}|config={}|fold={}|trigram={}|vectors={}",
            mapping.signature(),
            self.text.config,
            self.text.fold,
            self.trigram,
            self.vectors.as_ref().map(VectorStore::column_type).unwrap_or_default()
        );
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    /// Run searches, aggregations, and `get_document` on `read`, a physical replica of this store
    /// (`read-url`). Index and schema writes, the registry, and reads inside a transaction on the
    /// primary stay on the primary.
    #[must_use]
    pub fn with_read_replica(mut self, read: TransactionManager) -> Self {
        self.read = Some(read);
        self
    }

    /// Where a read runs. With no replica, or inside a transaction on the primary, the primary.
    /// Otherwise the replica; for read-your-writes (`fresh_by`) only once it has replayed the
    /// primary's WAL position as of now, which covers each commit the caller already waited for.
    /// A read that cannot be fresh by the deadline runs on the primary.
    fn read_route(&self, fresh_by: Option<Instant>) -> Route {
        let Some(read) = &self.read else {
            return Route::Primary;
        };
        if self.tx.in_transaction() {
            return Route::Primary;
        }
        let Some(deadline) = fresh_by else {
            return Route::Replica;
        };
        let lsn = match self
            .tx
            .with_connection(|conn| conn.query("SELECT pg_current_wal_lsn()::text AS lsn", &[]))
        {
            Ok(rows) => rows.first().and_then(|row| row.get("lsn")).map(to_plain_string),
            Err(err) => {
                tracing::warn!(error = %err, "could not read the primary WAL position; the read runs on the primary");
                None
            }
        };
        let Some(lsn) = lsn else {
            self.fallbacks.fetch_add(1, Ordering::Relaxed);
            return Route::Primary;
        };
        // A read-url that is not in recovery has no replay position; it is as current as it gets.
        let check = "SELECT COALESCE(pg_last_wal_replay_lsn() >= ?::pg_lsn, true) AS caught_up";
        loop {
            match read.with_connection(|conn| conn.query(check, &[Value::string(&lsn)])) {
                Ok(rows) if rows.first().and_then(|row| row.get("caught_up")) == Some(&Value::Bool(true)) => {
                    return Route::Replica;
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "the search replica cannot be read; the read runs on the primary");
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(REPLAY_POLL);
        }
        self.fallbacks.fetch_add(1, Ordering::Relaxed);
        Route::Primary
    }

    /// Run a read where `route` says. A replica error, such as a statement cancelled for replay,
    /// runs it again on the primary.
    fn query_on(&self, route: Route, sql: &Sql) -> Result<Vec<DbRow>, Error> {
        match (route, &self.read) {
            (Route::Replica, Some(read)) => {
                tracing::debug!(sql = %sql.text, "search query on the replica");
                match read.with_connection(|conn| conn.query(&sql.text, &sql.params)) {
                    Ok(rows) => Ok(rows),
                    Err(err) => {
                        tracing::warn!(error = %err, "search query failed on the replica; running it on the primary");
                        self.fallbacks.fetch_add(1, Ordering::Relaxed);
                        self.query(sql)
                    }
                }
            }
            _ => self.query(sql),
        }
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
        let config = literal(&self.text.config);
        let mut columns = vec![
            "doc_id text PRIMARY KEY".to_string(),
            "indexed_at timestamptz NOT NULL".to_string(),
            "body jsonb NOT NULL".to_string(),
            format!(
                "all_text tsvector GENERATED ALWAYS AS (jsonb_to_tsvector({config}, body, '[\"string\",\"numeric\"]')) STORED"
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
                    indexes.push(format!("CREATE INDEX ON {table} USING gin (to_tsvector({config}, {col}))"));
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
        let compiler = Compiler::with_text(mapping, &self.text);
        if let Some(title) = compiler.title_field() {
            // The folded title for fuzzy (trigram nearest neighbor) and autocomplete (prefix).
            columns.push(format!(
                "{NORM_TITLE} text GENERATED ALWAYS AS ({}({})) STORED",
                self.text.fold,
                column(title)
            ));
            indexes.push(format!("CREATE INDEX ON {table} ({NORM_TITLE} text_pattern_ops)"));
            if self.trigram {
                indexes.push(format!("CREATE INDEX ON {table} USING gist ({NORM_TITLE} gist_trgm_ops)"));
            }
        }
        if let Some(store) = &self.vectors {
            // A partial HNSW index: documents not yet embedded are not in it.
            columns.push(format!("embedding {}", store.column_type()));
            columns.push("embedding_model text".to_string());
            columns.push("embedding_indexed_at timestamptz".to_string());
            indexes.push(format!(
                "CREATE INDEX ON {table} USING hnsw (embedding {}_cosine_ops) WHERE embedding IS NOT NULL",
                store.representation
            ));
        }
        if !mapping.display.weights.is_empty() {
            // Weighted fields at their weight, then all the text at D, for cover-density ranking.
            let mut parts: Vec<String> = mapping
                .display
                .weights
                .iter()
                .filter(|(name, _)| mapping.root.fields.contains_key(*name))
                .map(|(name, weight)| {
                    format!("setweight(to_tsvector({config}, coalesce({}, '')), '{weight}')", column(name))
                })
                .collect();
            parts.push(format!(
                "setweight(jsonb_to_tsvector({config}, body, '[\"string\",\"numeric\"]'), 'D')"
            ));
            columns.push(format!(
                "{RANKED_TEXT} tsvector GENERATED ALWAYS AS ({}) STORED",
                parts.join(" || ")
            ));
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

    fn capabilities(&self) -> Vec<Capability> {
        let mut out = vec![
            Capability::Proximity,
            Capability::Autocomplete,
            Capability::WeightedRank,
            Capability::Hybrid,
        ];
        if self.trigram {
            out.push(Capability::Fuzzy);
        }
        if self.accent {
            out.push(Capability::Accent);
        }
        if self.vectors.is_some() {
            out.push(Capability::Semantic);
        }
        out
    }

    fn semantic_search(
        &self,
        mappings: &[Arc<DocumentMapping>],
        vector: &[f32],
        model: &str,
        filter: Option<&Query>,
        k: usize,
    ) -> Result<Vec<SearchHit>, Error> {
        let store = self
            .vectors
            .as_ref()
            .ok_or_else(|| super::capability_error(Capability::Semantic, "postgres"))?;
        let sql = semantic_sql(&self.schema, &self.text, store, mappings, vector, model, filter, k)?;
        let rows = self.query_on(self.read_route(None), &sql)?;
        rows.iter()
            .map(|row| {
                Ok(SearchHit {
                    document_id: row.get("document_id").map(to_plain_string).unwrap_or_default(),
                    id: row.get("doc_id").map(to_plain_string).unwrap_or_default(),
                    index: row.get("index_name").map(to_plain_string).unwrap_or_default(),
                    score: row
                        .get("score")
                        .map(|value| to_plain_string(value).parse::<f64>().unwrap_or(0.0))
                        .unwrap_or(0.0),
                    body: parse_body(row.get("body"))?,
                    highlights: IndexMap::new(),
                })
            })
            .collect()
    }

    fn pending_embeddings(
        &self,
        mapping: &DocumentMapping,
        model: &str,
        limit: usize,
    ) -> Result<Vec<crate::search::semantic::PendingEmbedding>, Error> {
        if self.vectors.is_none() {
            return Ok(Vec::new());
        }
        let mut sql = Sql::new(format!(
            "SELECT doc_id, indexed_at::text AS seen, body::text AS body FROM {} \
             WHERE embedding IS NULL OR embedding_model IS DISTINCT FROM ",
            self.table(mapping)
        ));
        sql.param(Value::string(model))
            .push(&format!(" OR embedding_indexed_at < indexed_at ORDER BY indexed_at LIMIT {limit}"));
        let rows = self.query(&sql)?;
        rows.iter()
            .map(|row| {
                Ok(crate::search::semantic::PendingEmbedding {
                    id: row.get("doc_id").map(to_plain_string).unwrap_or_default(),
                    seen: row.get("seen").map(to_plain_string).unwrap_or_default(),
                    body: parse_body(row.get("body"))?,
                })
            })
            .collect()
    }

    fn store_embedding(
        &self,
        mapping: &DocumentMapping,
        id: &str,
        seen: &str,
        model: &str,
        vector: &[f32],
    ) -> Result<bool, Error> {
        let store = self
            .vectors
            .as_ref()
            .ok_or_else(|| super::capability_error(Capability::Semantic, "postgres"))?;
        let statement = format!(
            "UPDATE {} SET embedding = ?::{}, embedding_model = ?, embedding_indexed_at = indexed_at \
             WHERE doc_id = ? AND indexed_at = ?::timestamptz",
            self.table(mapping),
            store.column_type()
        );
        let params = [
            Value::String(vector_literal(vector)),
            Value::string(model),
            Value::string(id),
            Value::string(seen),
        ];
        let count = self.tx.run(TransactionAttribute::UseOrBegin, None, || {
            self.tx.with_connection(|conn| conn.execute(&statement, &params))
        })?;
        Ok(count > 0)
    }

    fn ensure_index(&self, mapping: &DocumentMapping) -> Result<(), Error> {
        let signature = self.signature(mapping);
        if self.registered(&mapping.document_id)?.as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        // Feed workers index documents of one type at once; only one makes the table.
        let _ensuring = lock(&self.ensuring);
        lock(&self.known).remove(&mapping.document_id);
        let current = self.registered(&mapping.document_id)?;
        if current.as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        let table = self.table(mapping);
        let registry = format!("{}.document_index", ident(&self.schema));
        let statements = self.create_statements(mapping);
        let advisory = format!("{}.{}", self.schema, mapping.physical_name);
        // True when this call made the table; false when another made it first.
        self.tx.with_reserve(|conn| {
            conn.begin()?;
            let result = (|| {
                // Another process may be making it: wait for it, then look again.
                conn.query("SELECT pg_advisory_xact_lock(hashtext(?))", &[Value::string(&advisory)])?;
                let rows = conn.query(
                    &format!("SELECT signature FROM {registry} WHERE document_id = ?"),
                    &[Value::string(&mapping.document_id)],
                )?;
                if rows.first().and_then(|row| row.get("signature")).map(to_plain_string).as_deref()
                    == Some(signature.as_str())
                {
                    return Ok(false);
                }
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
                Ok(true)
            })();
            match result {
                Ok(made) => conn.commit().map(|()| made),
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
        let compiler = Compiler::with_text(mapping, &self.text);
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
        let rows = self.query_on(self.read_route(None), &sql)?;
        rows.first().map(|row| parse_body(row.get("body"))).transpose()
    }

    fn search(&self, mappings: &[Arc<DocumentMapping>], request: &SearchRequest) -> Result<SearchPage, Error> {
        let plan = search_sql(&self.schema, &self.text, mappings, request)?;
        let route = self.read_route(request.fresh_by);
        let total = self
            .query_on(route, &plan.count)?
            .first()
            .and_then(|row| row.get("total"))
            .map(|value| to_plain_string(value).parse::<u64>().unwrap_or(0))
            .unwrap_or(0);
        let mut hits = Vec::new();
        if request.limit > 0 && total > 0 {
            for row in self.query_on(route, &plan.select)? {
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
        Ok(SearchPage {
            hits,
            total,
            total_relation: super::TotalRelation::Eq,
        })
    }

    fn aggregate(
        &self,
        mapping: &DocumentMapping,
        filter: &Query,
        aggs: &IndexMap<String, Agg>,
    ) -> Result<IndexMap<String, Value>, Error> {
        self.aggregate_fresh(mapping, filter, aggs, None)
    }

    fn aggregate_fresh(
        &self,
        mapping: &DocumentMapping,
        filter: &Query,
        aggs: &IndexMap<String, Agg>,
        fresh_by: Option<Instant>,
    ) -> Result<IndexMap<String, Value>, Error> {
        let table = self.table(mapping);
        let route = self.read_route(fresh_by);
        let mut run = |sql: &Sql| self.query_on(route, sql);
        agg::aggregate(&table, mapping, &self.text, filter, aggs, &mut run)
    }

    fn replica_status(&self) -> Option<ReplicaStatus> {
        let read = self.read.as_ref()?;
        let lag_seconds = read
            .with_connection(|conn| {
                conn.query(
                    "SELECT EXTRACT(EPOCH FROM (now() - pg_last_xact_replay_timestamp()))::float8 AS lag",
                    &[],
                )
            })
            .ok()
            .and_then(|rows| rows.first().and_then(|row| row.get("lag")).cloned())
            .and_then(|value| to_plain_string(&value).parse::<f64>().ok());
        Some(ReplicaStatus {
            lag_seconds,
            primary_fallbacks: self.fallbacks.load(Ordering::Relaxed),
        })
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
    text: &TextSetup,
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
        let compiler = Compiler::with_text(mapping, text);
        let table = format!("{}.{}", ident(schema), ident(&mapping.physical_name));
        // The trusted filter narrows the rows; only the query scores.
        let where_sql = compiler.where_sql(&request.matching())?;
        let mut part = Sql::new(format!(
            "SELECT {} AS document_id, {} AS index_name, doc_id, body::text AS body, ",
            literal(&mapping.document_id),
            literal(&mapping.physical_name)
        ));
        part.push("(");
        match compiler.rank_query(&request.query) {
            // Cover density over the weighted vector when the document has weights, else all text.
            Some(rank) if request.rank == Rank::CoverDensity => {
                let vector = if mapping.display.weights.is_empty() { "all_text" } else { RANKED_TEXT };
                part.push(&format!("ts_rank_cd({vector}, "));
                part.append(rank);
                part.push(")");
            }
            Some(rank) => {
                part.push("ts_rank(all_text, ");
                part.append(rank);
                part.push(")");
            }
            None => {
                part.push("0");
            }
        }
        for extra in compiler.extra_score(&request.query) {
            part.push(" + ");
            part.append(extra);
        }
        part.push(")::float8 AS score");
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
            display: Default::default(),
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
        let plan = search_sql("search", &TextSetup::default(), &mappings, &request).unwrap();
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

    #[test]
    fn semantic_sql_ranks_by_cosine_distance_per_type_with_the_filter() {
        let mut mapping = super::super::logs::log_mapping("moqui_logs").unwrap();
        mapping.document_id = "TestDoc".into();
        mapping.physical_name = "test_doc".into();
        let store = VectorStore {
            representation: "halfvec".into(),
            dimensions: 3,
        };
        let filter = Query::eq("level", Value::string("WARN"));
        let sql = semantic_sql(
            "search",
            &TextSetup::default(),
            &store,
            &[Arc::new(mapping)],
            &[0.5, 0.25, 0.0],
            "moqui-hash",
            Some(&filter),
            10,
        )
        .unwrap();
        assert_eq!(
            sql.text,
            "SELECT * FROM ((SELECT 'TestDoc' AS document_id, 'test_doc' AS index_name, doc_id, body::text AS body, \
             (1 - (embedding <=> ?::halfvec(3)))::float8 AS score FROM \"search\".\"test_doc\" WHERE embedding IS NOT NULL \
             AND embedding_model = ? AND COALESCE(\"f_level\" = ?, FALSE) ORDER BY embedding <=> ?::halfvec(3) LIMIT 10)) AS s \
             ORDER BY score DESC, document_id ASC, doc_id ASC LIMIT 10"
        );
        assert_eq!(sql.params[0], Value::string("[0.5,0.25,0]"));
        assert_eq!(sql.params[1], Value::string("moqui-hash"));
    }
}
