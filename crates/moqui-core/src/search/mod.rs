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

//! `SearchFacade`: data documents searched and aggregated through one backend.
//!
//! Callers pass a [`Query`] tree and named [`Agg`] trees. A backend compiles them: Postgres SQL in
//! [`postgres`], or Elastic JSON on the OpenSearch backend. The design is `plans/SearchFacade.md`.
//!
//! The facade is installed per factory by the host ([`install`]) and found from the entered
//! execution context ([`current`]).

pub mod agg;
pub mod conf;
pub mod es_json;
pub mod feed;
pub mod highlight;
pub mod mapping;
#[cfg(feature = "http")]
pub mod opensearch;
pub mod parse;
pub mod postgres;
pub mod query;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use indexmap::IndexMap;
use moqui_lib::error::Error;
use moqui_lib::object::Value;

pub use agg::Agg;
pub use mapping::{DocumentMapping, FieldKind, FieldMapping};
pub use query::{Query, TextMatch};

use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::document::{self, Document};
use crate::entity::EntityFacade;

/// The deepest result window, as Elastic's `index.max_result_window`.
pub const MAX_WINDOW: usize = 10_000;

/// Control fields a document carries that are not stored in its body.
pub const CONTROL_FIELDS: [&str; 4] = ["_index", "_type", "_id", "_timestamp"];

/// What a search runs over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// These `DataDocument` ids.
    Documents(Vec<String>),
    /// Every document whose `indexName` is this.
    Index(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortField {
    /// A field name. A trailing `.keyword` names the unanalyzed value. `_score` and `_id` are the
    /// rank and the document id.
    pub field: String,
    pub descending: bool,
}

impl SortField {
    /// `-name` descending, `+name` or `name` ascending.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (descending, name) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let name = name.trim();
        (!name.is_empty()).then(|| Self {
            field: name.to_string(),
            descending,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub query: Query,
    pub sort: Vec<SortField>,
    pub offset: usize,
    pub limit: usize,
    /// Field names to return highlighted fragments for.
    pub highlight: Vec<String>,
}

impl SearchRequest {
    pub fn new(query: Query) -> Self {
        Self {
            query,
            sort: Vec::new(),
            offset: 0,
            limit: 20,
            highlight: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    /// The `DataDocument` id, such as `MantleParty`.
    pub document_id: String,
    /// The record id, primary key values joined with `::`.
    pub id: String,
    /// The physical store name, `mantle_party` for `MantleParty`.
    pub index: String,
    pub score: f64,
    pub body: Document,
    pub highlights: IndexMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// Exact count of matching documents, not just this page.
    pub total: u64,
}

/// One search store. The facade resolves targets and mappings; a backend compiles and runs.
pub trait SearchBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Create the store for a document, or rebuild it when its stored shape changed.
    fn ensure_index(&self, mapping: &DocumentMapping) -> Result<(), Error>;
    fn index_exists(&self, mapping: &DocumentMapping) -> Result<bool, Error>;
    /// Insert or replace documents. Control fields are already removed from each body.
    fn index_documents(&self, mapping: &DocumentMapping, documents: &[(String, Document)]) -> Result<usize, Error>;
    fn delete_document(&self, mapping: &DocumentMapping, id: &str) -> Result<bool, Error>;
    fn delete_matching(&self, mapping: &DocumentMapping, filter: &Query) -> Result<u64, Error>;
    fn get_document(&self, mapping: &DocumentMapping, id: &str) -> Result<Option<Document>, Error>;
    /// `mappings` are the stores that exist for the target.
    fn search(&self, mappings: &[Arc<DocumentMapping>], request: &SearchRequest) -> Result<SearchPage, Error>;
    fn aggregate(
        &self,
        mapping: &DocumentMapping,
        filter: &Query,
        aggs: &IndexMap<String, Agg>,
    ) -> Result<IndexMap<String, Value>, Error>;
    fn drop_index(&self, mapping: &DocumentMapping) -> Result<(), Error>;
}

pub struct SearchFacade {
    backend: Arc<dyn SearchBackend>,
    entity: EntityFacade,
    mappings: Mutex<HashMap<String, Arc<DocumentMapping>>>,
    ensured: Mutex<HashSet<String>>,
}

impl SearchFacade {
    pub fn new(backend: Arc<dyn SearchBackend>, entity: EntityFacade) -> Self {
        Self {
            backend,
            entity,
            mappings: Mutex::new(HashMap::new()),
            ensured: Mutex::new(HashSet::new()),
        }
    }

    pub fn backend(&self) -> &Arc<dyn SearchBackend> {
        &self.backend
    }

    pub fn entity(&self) -> &EntityFacade {
        &self.entity
    }

    /// The stored shape of a document, from its `DataDocument` definition.
    ///
    /// # Errors
    ///
    /// Returns an error when the definition does not resolve.
    pub fn mapping(&self, data_document_id: &str) -> Result<Arc<DocumentMapping>, Error> {
        if let Some(found) = lock(&self.mappings).get(data_document_id) {
            return Ok(Arc::clone(found));
        }
        let info = document::document_info(&self.entity, data_document_id)?;
        let mapping = Arc::new(DocumentMapping::from_info(&info));
        lock(&self.mappings).insert(data_document_id.to_string(), Arc::clone(&mapping));
        Ok(mapping)
    }

    /// Forget cached definitions and mappings, so the next call reads `DataDocument` again.
    pub fn clear_definitions(&self) {
        lock(&self.mappings).clear();
        lock(&self.ensured).clear();
        document::clear_document_cache(&self.entity);
    }

    /// Document ids for a target. An index with no documents is empty.
    ///
    /// # Errors
    ///
    /// Returns an error when the `DataDocument` table cannot be read.
    pub fn target_documents(&self, target: &Target) -> Result<Vec<String>, Error> {
        match target {
            Target::Documents(ids) => Ok(ids.clone()),
            Target::Index(name) => document::documents_in_index(&self.entity, name),
        }
    }

    /// `ensure_index`: create or rebuild the store from the current definition.
    ///
    /// # Errors
    ///
    /// Returns an error when the definition does not resolve or the store cannot be made.
    pub fn ensure_index(&self, data_document_id: &str) -> Result<(), Error> {
        lock(&self.ensured).remove(data_document_id);
        lock(&self.mappings).remove(data_document_id);
        document::clear_document_cache(&self.entity);
        self.ensure_cached(data_document_id).map(|_| ())
    }

    fn ensure_cached(&self, data_document_id: &str) -> Result<Arc<DocumentMapping>, Error> {
        let mapping = self.mapping(data_document_id)?;
        if lock(&self.ensured).contains(data_document_id) {
            return Ok(mapping);
        }
        self.backend.ensure_index(&mapping)?;
        lock(&self.ensured).insert(data_document_id.to_string());
        Ok(mapping)
    }

    /// Index documents as `EntityDataDocument` builds them. Each names its document in `_type`
    /// and its record in `_id`. Control fields are not stored in the body.
    ///
    /// # Errors
    ///
    /// Returns an error when a document has no `_type` or `_id`, or the store write fails.
    pub fn index_documents(&self, documents: &[Document]) -> Result<usize, Error> {
        let mut grouped: IndexMap<String, Vec<(String, Document)>> = IndexMap::new();
        for doc in documents {
            let doc_type = control_text(doc, "_type").ok_or_else(|| Error::new("a document to index has no _type"))?;
            let id = control_text(doc, "_id").ok_or_else(|| Error::new(format!("a {doc_type} document has no _id")))?;
            let mut body = doc.clone();
            for name in CONTROL_FIELDS {
                body.shift_remove(name);
            }
            grouped.entry(doc_type).or_default().push((id, body));
        }
        let mut count = 0;
        for (doc_type, docs) in grouped {
            let mapping = self.ensure_cached(&doc_type)?;
            count += self.backend.index_documents(&mapping, &docs)?;
        }
        Ok(count)
    }

    /// Remove one document. False when it was not there.
    ///
    /// # Errors
    ///
    /// Returns an error when the definition does not resolve or the store write fails.
    pub fn delete_document(&self, data_document_id: &str, id: &str) -> Result<bool, Error> {
        let mapping = self.mapping(data_document_id)?;
        if !self.backend.index_exists(&mapping)? {
            return Ok(false);
        }
        self.backend.delete_document(&mapping, id)
    }

    /// Remove every document of the target that matches `filter`. Returns how many.
    ///
    /// # Errors
    ///
    /// Returns an error when the filter does not compile or the store write fails.
    pub fn delete_matching(&self, target: &Target, filter: &Query) -> Result<u64, Error> {
        let mut count = 0;
        for id in self.target_documents(target)? {
            let mapping = self.mapping(&id)?;
            if self.backend.index_exists(&mapping)? {
                count += self.backend.delete_matching(&mapping, filter)?;
            }
        }
        Ok(count)
    }

    /// The stored body of one document, without control fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the definition does not resolve or the read fails.
    pub fn get_document(&self, data_document_id: &str, id: &str) -> Result<Option<Document>, Error> {
        let mapping = self.mapping(data_document_id)?;
        if !self.backend.index_exists(&mapping)? {
            return Ok(None);
        }
        self.backend.get_document(&mapping, id)
    }

    /// True when at least one document of the target has a store.
    ///
    /// # Errors
    ///
    /// Returns an error when the definitions or the store cannot be read.
    pub fn target_exists(&self, target: &Target) -> Result<bool, Error> {
        for id in self.target_documents(target)? {
            if self.backend.index_exists(&*self.mapping(&id)?)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Search. Documents with no store yet are skipped, so a target with none is an empty page.
    ///
    /// # Errors
    ///
    /// Returns an error when the query does not compile against the documents, or the read fails.
    pub fn search(&self, target: &Target, request: &SearchRequest) -> Result<SearchPage, Error> {
        if request.limit > MAX_WINDOW || request.offset.saturating_add(request.limit) > MAX_WINDOW {
            return Err(Error::new(format!(
                "a search page must end within the first {MAX_WINDOW} results"
            )));
        }
        let mut mappings = Vec::new();
        for id in self.target_documents(target)? {
            let mapping = self.mapping(&id)?;
            if self.backend.index_exists(&mapping)? {
                mappings.push(mapping);
            }
        }
        if mappings.is_empty() {
            return Ok(SearchPage::default());
        }
        self.backend.search(&mappings, request)
    }

    /// Aggregate over one document type. `filter` is the top-level constraint and does not score.
    ///
    /// # Errors
    ///
    /// Returns an error when the target is not one document, or a tree does not compile.
    pub fn aggregate(
        &self,
        target: &Target,
        filter: &Query,
        aggs: &IndexMap<String, Agg>,
    ) -> Result<IndexMap<String, Value>, Error> {
        let ids = self.target_documents(target)?;
        let [id] = ids.as_slice() else {
            return Err(Error::new(format!(
                "aggregate runs over one document type, not {}",
                ids.len()
            )));
        };
        let mapping = self.mapping(id)?;
        if !self.backend.index_exists(&mapping)? {
            return Ok(agg_empty_results(aggs));
        }
        self.backend.aggregate(&mapping, filter, aggs)
    }

    /// Drop a document's store. The next index call makes it again.
    ///
    /// # Errors
    ///
    /// Returns an error when the definition does not resolve or the drop fails.
    pub fn drop_index(&self, data_document_id: &str) -> Result<(), Error> {
        let mapping = self.mapping(data_document_id)?;
        lock(&self.ensured).remove(data_document_id);
        self.backend.drop_index(&mapping)
    }
}

/// Results for a store that does not exist yet: empty buckets and zero metrics.
pub fn agg_empty_results(aggs: &IndexMap<String, Agg>) -> IndexMap<String, Value> {
    let mut out = IndexMap::new();
    for (name, agg) in aggs {
        out.insert(name.clone(), agg_empty(agg));
    }
    out
}

/// The result of one aggregation over no documents.
pub fn agg_empty(agg: &Agg) -> Value {
    match agg {
        Agg::Sum { .. } => Value::Decimal(Default::default()),
        Agg::CountDistinct { .. } => Value::I64(0),
        Agg::Filter { aggs, .. } => {
            let mut map = agg_empty_results(aggs);
            map.insert("doc_count".to_string(), Value::I64(0));
            map.move_index(map.len() - 1, 0);
            Value::Map(map)
        }
        Agg::DateHistogram(_) | Agg::Terms(_) => {
            let mut map = IndexMap::new();
            map.insert("buckets".to_string(), Value::List(Vec::new()));
            Value::Map(map)
        }
    }
}

fn control_text(doc: &Document, name: &str) -> Option<String> {
    match doc.get(name) {
        None | Some(Value::Null) => None,
        Some(value) => Some(moqui_lib::object::to_plain_string(value)).filter(|text| !text.is_empty()),
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[derive(Default)]
struct Slot {
    facade: RwLock<Option<Arc<SearchFacade>>>,
}

/// Install the search facade on a factory. Feed receive and `ec.search` use it from then on.
pub fn install(factory: &ExecutionContextFactory, facade: SearchFacade) {
    let slot = factory.extension(|_| Slot::default());
    *slot.facade.write().unwrap_or_else(|err| err.into_inner()) = Some(Arc::new(facade));
}

/// The facade installed on a factory.
pub fn installed(factory: &ExecutionContextFactory) -> Option<Arc<SearchFacade>> {
    let slot = factory.extension(|_| Slot::default());
    let found = slot.facade.read().unwrap_or_else(|err| err.into_inner()).clone();
    found
}

/// The facade for the entered execution context.
///
/// # Errors
///
/// Returns an error when no context is entered or no search backend is configured.
pub fn current() -> Result<Arc<SearchFacade>, Error> {
    ExecutionContext::with_current(|ec| installed(ec.factory()))
        .ok_or_else(|| Error::new("no execution context is entered"))?
        .ok_or_else(|| Error::new("search is not configured"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_fields_parse_signs() {
        assert_eq!(
            SortField::parse("-userAccounts.username.keyword"),
            Some(SortField {
                field: "userAccounts.username.keyword".into(),
                descending: true
            })
        );
        assert_eq!(SortField::parse("+pseudoId").map(|sort| sort.descending), Some(false));
        assert_eq!(SortField::parse("  "), None);
    }

    #[test]
    fn empty_results_have_zero_metrics_and_no_buckets() {
        let aggs = Agg::map_from_value(&moqui_lib::json::from_json(
            &serde_json::from_str(
                r#"{"total":{"sum":{"field":"x"}},"P":{"filter":{"term":{"t":"A"}},"aggs":{"n":{"cardinality":{"field":"o"}}}},
                    "D":{"date_histogram":{"field":"d","unit":"day"}}}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let empty = agg_empty_results(&aggs);
        assert!(matches!(empty["total"], Value::Decimal(_)));
        let Value::Map(filter) = &empty["P"] else { panic!() };
        assert_eq!(filter.keys().next().map(String::as_str), Some("doc_count"));
        assert_eq!(filter["n"], Value::I64(0));
    }
}
