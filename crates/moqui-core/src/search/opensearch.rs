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

//! The OpenSearch backend: the shared `_search`, `_bulk`, and mapping subset of OpenSearch and
//! ElasticSearch 7+, sent through the blocking [`RestClient`].
//!
//! The compiler emits the JSON the Java services emitted. `Text` is a `query_string` in `must`,
//! so it scores. `Field` clauses are `term`, `terms`, `prefix`, `wildcard`, `range`, and `exists`.
//! `Child` is `nested`. Many-relationships are mapped `nested` with `include_in_root`, so dotted
//! names and OpenSearch Dashboards keep working. A date histogram sends `calendar_interval`.
//! `CountDistinct` sends `precision_threshold` and is approximate past it. Each stored document
//! carries `_indexed_at`, the index time a retention delete filters on.
//!
//! Highlights are made from `_source` by [`super::highlight`], the same as on Postgres.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use indexmap::IndexMap;
use jiff::Timestamp;
use moqui_lib::error::Error;
use moqui_lib::http::RestClient;
use moqui_lib::object::{to_plain_string, Value};
use serde_json::{json, Map, Value as Json};

use super::agg::{histogram_zone, zone_name, Agg, DateHistogram, Terms, TermsOrder};
use super::conf::ClusterConf;
use super::mapping::{DocumentMapping, FieldKind, ObjectMapping, Resolved};
use super::postgres::body_json;
use super::postgres::compile::parse_date_bound;
use super::query::{FieldOp, FieldQuery, Query, TextFields, TextMatch};
use super::{lock, Capability, Rank, SearchBackend, SearchHit, SearchPage, SearchRequest};
use crate::entity::document::Document;

/// The index-time field each stored document carries.
pub const INDEXED_AT: &str = "_indexed_at";
const PRECISION_THRESHOLD: u32 = 40_000;

/// A mapping as Elastic JSON: `{_meta, properties}`.
/// The keyword subfield of the `title` role, lower-cased by [`NORMALIZER`], for autocomplete.
pub const NORM_SUBFIELD: &str = "norm";
/// The normalizer [`NORM_SUBFIELD`] uses.
pub const NORMALIZER: &str = "moqui_fold";

/// Index settings the mapping needs: the title normalizer when there is a title.
pub fn settings_json(mapping: &DocumentMapping) -> Option<Json> {
    mapping.display.role("title")?;
    Some(json!({"analysis": {"normalizer": {NORMALIZER: {"type": "custom", "filter": ["lowercase"]}}}}))
}

pub fn mapping_json(mapping: &DocumentMapping) -> Json {
    let mut properties = object_properties(&mapping.root);
    if let Some(Json::Object(title)) = mapping
        .display
        .role("title")
        .and_then(|name| properties.get_mut(name))
    {
        let fields = title.entry("fields").or_insert_with(|| json!({}));
        if let Json::Object(fields) = fields {
            fields.insert(NORM_SUBFIELD.into(), json!({"type": "keyword", "normalizer": NORMALIZER}));
        }
    }
    properties.insert(
        INDEXED_AT.to_string(),
        json!({"type": "date", "format": "strict_date_optional_time||epoch_millis"}),
    );
    json!({
        "_meta": {"moqui_document_id": mapping.document_id, "moqui_signature": mapping.signature_hash()},
        "properties": properties,
    })
}

fn object_properties(object: &ObjectMapping) -> Map<String, Json> {
    let mut properties = Map::new();
    for (name, field) in &object.fields {
        let mut property = Map::new();
        property.insert("type".into(), Json::String(field.kind.search_type().to_string()));
        if field.kind == FieldKind::Date {
            property.insert(
                "format".into(),
                Json::String("strict_date_optional_time||epoch_millis||yyyy-MM-dd HH:mm:ss.SSS||yyyy-MM-dd".into()),
            );
        }
        if field.kind == FieldKind::Text && field.sortable {
            property.insert("fields".into(), json!({"keyword": {"type": "keyword"}}));
        }
        properties.insert(name.clone(), Json::Object(property));
    }
    for (name, child) in &object.children {
        properties.insert(
            name.clone(),
            json!({"type": "nested", "include_in_root": true, "properties": object_properties(child)}),
        );
    }
    properties
}

/// The name a query, sort, or aggregation sends for a field. Sortable text sorts and buckets on
/// its `.keyword` subfield. Other `.keyword` suffixes are dropped, since the field is one.
fn field_name(mapping: &DocumentMapping, name: &str, unanalyzed: bool) -> String {
    match name {
        // A log index has its own `@timestamp`, the event time.
        "@timestamp" if mapping.root.fields.contains_key(name) => return name.to_string(),
        "@timestamp" | "_timestamp" | "indexed_at" => return INDEXED_AT.to_string(),
        "_id" | "_score" => return name.to_string(),
        _ => {}
    }
    let base = name.strip_suffix(".keyword").unwrap_or(name);
    match mapping.resolve(base) {
        Resolved::Root { field, .. } | Resolved::Child { field, .. }
            if field.kind == FieldKind::Text && field.sortable && (unanalyzed || name.ends_with(".keyword")) =>
        {
            format!("{base}.keyword")
        }
        _ => base.to_string(),
    }
}

fn kind_of(mapping: &DocumentMapping, name: &str) -> Option<FieldKind> {
    match name {
        "@timestamp" if mapping.root.fields.contains_key(name) => Some(FieldKind::Date),
        "@timestamp" | "_timestamp" | "indexed_at" => Some(FieldKind::Date),
        _ => mapping.resolve(name).field().map(|field| field.kind),
    }
}

/// Lucene query-string escaping for one term.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for ch in text.chars() {
        if "+-=&|><!(){}[]^\"~*?:\\/ ".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A field value for Elastic: dates as epoch millis, decimals as numbers.
fn value_json(kind: Option<FieldKind>, value: &Value) -> Json {
    if kind == Some(FieldKind::Date) {
        if let Some(at) = parse_date_bound(value) {
            return Json::from(at.as_millisecond());
        }
    }
    moqui_lib::json::to_json(value)
}

/// [`Query`] to Elastic query JSON. `path` is the enclosing nested path, which full names need.
pub fn query_json(mapping: &DocumentMapping, query: &Query) -> Json {
    compile(mapping, query, "")
}

fn full(prefix: &str, name: &str) -> String {
    if prefix.is_empty() || name.starts_with(prefix) {
        name.to_string()
    } else {
        format!("{prefix}{name}")
    }
}

fn compile(mapping: &DocumentMapping, query: &Query, prefix: &str) -> Json {
    match query {
        Query::MatchAll => json!({"match_all": {}}),
        Query::Lucene(text) => json!({"query_string": {"query": text, "lenient": true, "analyze_wildcard": true}}),
        Query::Near { terms, fields, distance } => {
            let names: Vec<String> = match fields {
                // Every text field in scope: intervals need one field each.
                TextFields::All => mapping
                    .textual_fields()
                    .into_iter()
                    .filter(|(name, field)| field.kind == FieldKind::Text && name.starts_with(prefix))
                    .map(|(name, _)| name)
                    .collect(),
                TextFields::Only(names) => names
                    .iter()
                    .map(|name| field_name(mapping, &full(prefix, name), false))
                    .collect(),
            };
            let should: Vec<Json> = names
                .iter()
                .map(|name| {
                    json!({"intervals": {name: {"match": {"query": terms, "max_gaps": distance, "ordered": true}}}})
                })
                .collect();
            if should.is_empty() {
                json!({"match_none": {}})
            } else {
                json!({"bool": {"should": should, "minimum_should_match": 1}})
            }
        }
        Query::Fuzzy { terms } => match mapping.display.role("title") {
            Some(title) if prefix.is_empty() && !terms.trim().is_empty() => {
                json!({"match": {title: {"query": terms.trim(), "fuzziness": "AUTO", "operator": "and"}}})
            }
            _ => json!({"match_none": {}}),
        },
        Query::Suggest { text } => match mapping.display.role("title") {
            Some(_) if !prefix.is_empty() => json!({"match_none": {}}),
            Some(_) if text.trim().is_empty() => json!({"match_all": {}}),
            Some(title) => json!({"bool": {"should": [
                {"prefix": {format!("{title}.{NORM_SUBFIELD}"): {"value": text.trim().to_lowercase(), "boost": 2.0}}},
                {"match_bool_prefix": {title: {"query": text.trim(), "operator": "and"}}}
            ], "minimum_should_match": 1}}),
            None => json!({"match_none": {}}),
        },
        Query::Text { terms, fields, kind } => {
            let text = match kind {
                TextMatch::Term => terms.split_whitespace().map(escape).collect::<Vec<_>>().join(" "),
                TextMatch::Prefix => terms
                    .split_whitespace()
                    .map(|word| format!("{}*", escape(word)))
                    .collect::<Vec<_>>()
                    .join(" "),
                TextMatch::Contains => format!("*{}*", escape(terms)),
                TextMatch::Phrase => format!("\"{}\"", terms.replace('\\', "\\\\").replace('"', "\\\"")),
                // Already escaped by the parser: `\*` is literal, `*` and `?` are wildcards.
                TextMatch::Wildcard => terms.clone(),
            };
            let names: Vec<Json> = match fields {
                TextFields::All => vec![Json::String("*".into())],
                TextFields::Only(names) => names
                    .iter()
                    .map(|name| Json::String(field_name(mapping, &full(prefix, name), false)))
                    .collect(),
            };
            json!({"query_string": {"query": text, "fields": names, "default_operator": "AND",
                "lenient": true, "analyze_wildcard": true}})
        }
        Query::Field(field) if prefix.is_empty() && field.name == super::query::DOCUMENT_FIELD => {
            if field.document_match(&mapping.document_id).unwrap_or(false) {
                json!({"match_all": {}})
            } else {
                json!({"match_none": {}})
            }
        }
        Query::Field(field) => {
            let positive = field_json(mapping, field, prefix);
            if field.negate {
                json!({"bool": {"must_not": [positive]}})
            } else {
                positive
            }
        }
        Query::Bool {
            all,
            any,
            none,
            min_any,
        } => {
            let mut body = Map::new();
            let list = |items: &[Query]| Json::Array(items.iter().map(|item| compile(mapping, item, prefix)).collect());
            if !all.is_empty() {
                body.insert("must".into(), list(all));
            }
            if !any.is_empty() {
                body.insert("should".into(), list(any));
            }
            if !none.is_empty() {
                body.insert("must_not".into(), list(none));
            }
            if let Some(min) = min_any {
                body.insert("minimum_should_match".into(), Json::from(*min));
            }
            json!({"bool": body})
        }
        Query::Child { path, query } => {
            let path = full(prefix, path);
            let inner = compile(mapping, query, &format!("{path}."));
            json!({"nested": {"path": path, "query": inner, "score_mode": "avg"}})
        }
    }
}

fn field_json(mapping: &DocumentMapping, field: &FieldQuery, prefix: &str) -> Json {
    let name = full(prefix, &field.name);
    let kind = kind_of(mapping, name.strip_suffix(".keyword").unwrap_or(&name));
    let sent = field_name(mapping, &name, false);
    match field.op {
        FieldOp::Exists => json!({"exists": {"field": sent}}),
        FieldOp::Eq if kind == Some(FieldKind::Text) => {
            json!({"match": {sent: {"query": to_plain_string(&field.value), "operator": "and"}}})
        }
        FieldOp::Eq => json!({"term": {sent: value_json(kind, &field.value)}}),
        FieldOp::In => {
            let values: Vec<Json> = if field.values.is_empty() {
                vec![value_json(kind, &field.value)]
            } else {
                field.values.iter().map(|value| value_json(kind, value)).collect()
            };
            json!({"terms": {sent: values}})
        }
        FieldOp::Prefix => json!({"prefix": {sent: to_plain_string(&field.value)}}),
        FieldOp::Wildcard => json!({"wildcard": {sent: to_plain_string(&field.value)}}),
        FieldOp::Range => {
            let mut bounds = Map::new();
            for (key, bound) in [
                ("gt", &field.gt),
                ("gte", &field.gte),
                ("lt", &field.lt),
                ("lte", &field.lte),
            ] {
                if let Some(bound) = bound {
                    bounds.insert(key.into(), value_json(kind, bound));
                }
            }
            json!({"range": {sent: bounds}})
        }
    }
}

/// The query of a request for one document type. Cover-density ranking adds the positive text
/// terms as an optional `multi_match` over the weighted fields, boosted A 4 through D 1, so the
/// weights change the score and not what matches.
pub fn request_query(mapping: &DocumentMapping, request: &SearchRequest) -> Json {
    let query = scored_query(mapping, request);
    match &request.filter {
        // The trusted filter narrows without scoring.
        Some(filter) => json!({"bool": {"must": [query], "filter": [query_json(mapping, filter)]}}),
        None => query,
    }
}

fn scored_query(mapping: &DocumentMapping, request: &SearchRequest) -> Json {
    let query = query_json(mapping, &request.query);
    if request.rank != Rank::CoverDensity || mapping.display.weights.is_empty() {
        return query;
    }
    let mut terms = Vec::new();
    positive_terms(&request.query, false, &mut terms);
    if terms.is_empty() {
        return query;
    }
    let fields: Vec<Json> = mapping
        .display
        .weights
        .iter()
        .map(|(name, weight)| {
            let boost = match weight {
                'A' => 4,
                'B' => 3,
                'C' => 2,
                _ => 1,
            };
            Json::String(format!("{name}^{boost}"))
        })
        .collect();
    json!({"bool": {"must": [query], "should": [
        {"multi_match": {"query": terms.join(" "), "fields": fields, "type": "best_fields"}}
    ]}})
}

fn positive_terms(query: &Query, negated: bool, out: &mut Vec<String>) {
    match query {
        Query::Text { terms, .. } | Query::Near { terms, .. } if !negated => out.push(terms.clone()),
        Query::Bool { all, any, none, .. } => {
            for item in all.iter().chain(any) {
                positive_terms(item, negated, out);
            }
            for item in none {
                positive_terms(item, !negated, out);
            }
        }
        Query::Child { query, .. } => positive_terms(query, negated, out),
        _ => {}
    }
}

/// The `_search` body for a request.
pub fn search_json(mapping: &DocumentMapping, request: &SearchRequest) -> Json {
    let mut body = Map::new();
    body.insert("query".into(), request_query(mapping, request));
    body.insert("from".into(), Json::from(request.offset));
    body.insert("size".into(), Json::from(request.limit));
    body.insert("track_total_hits".into(), Json::Bool(true));
    if !request.sort.is_empty() {
        let sort: Vec<Json> = request
            .sort
            .iter()
            .map(|sort| {
                let order = if sort.descending { "desc" } else { "asc" };
                let name = field_name(mapping, &sort.field, true);
                if name == "_score" {
                    json!({"_score": {"order": order}})
                } else {
                    let mode = if sort.descending { "max" } else { "min" };
                    json!({name: {"order": order, "missing": "_last", "mode": mode}})
                }
            })
            .collect();
        body.insert("sort".into(), Json::Array(sort));
    }
    Json::Object(body)
}

/// Named aggregations as Elastic JSON.
pub fn aggs_json(mapping: &DocumentMapping, aggs: &IndexMap<String, Agg>) -> Result<Json, Error> {
    let mut out = Map::new();
    for (name, agg) in aggs {
        out.insert(name.clone(), agg_json(mapping, agg)?);
    }
    Ok(Json::Object(out))
}

fn agg_json(mapping: &DocumentMapping, agg: &Agg) -> Result<Json, Error> {
    let sub = |aggs: &IndexMap<String, Agg>, body: &mut Map<String, Json>| -> Result<(), Error> {
        if !aggs.is_empty() {
            body.insert("aggs".into(), aggs_json(mapping, aggs)?);
        }
        Ok(())
    };
    Ok(match agg {
        Agg::Sum { field } => json!({"sum": {"field": field_name(mapping, field, false)}}),
        Agg::CountDistinct { field } => json!({"cardinality": {"field": field_name(mapping, field, true),
            "precision_threshold": PRECISION_THRESHOLD}}),
        Agg::Filter { query, aggs } => {
            let mut body = Map::new();
            body.insert("filter".into(), query_json(mapping, query));
            sub(aggs, &mut body)?;
            Json::Object(body)
        }
        Agg::DateHistogram(histogram) => {
            let zone = histogram_zone(histogram.time_zone.as_deref())?;
            let mut inner = Map::new();
            inner.insert(
                "field".into(),
                Json::String(field_name(mapping, &histogram.field, false)),
            );
            inner.insert(
                "calendar_interval".into(),
                Json::String(histogram.unit.calendar_interval().into()),
            );
            inner.insert("time_zone".into(), Json::String(zone_name(&zone)));
            if let Some(format) = &histogram.format {
                inner.insert("format".into(), Json::String(format.clone()));
            }
            if let Some(offset) = &histogram.offset_text {
                inner.insert("offset".into(), Json::String(offset.clone()));
            }
            inner.insert(
                "order".into(),
                json!({"_key": if histogram.descending { "desc" } else { "asc" }}),
            );
            inner.insert("min_doc_count".into(), Json::from(histogram.min_doc_count));
            let mut body = Map::new();
            body.insert("date_histogram".into(), Json::Object(inner));
            sub(&histogram.aggs, &mut body)?;
            Json::Object(body)
        }
        Agg::Terms(terms) => {
            let order = match &terms.order {
                TermsOrder::Count { descending } => json!({"_count": dir(*descending)}),
                TermsOrder::Key { descending } => json!({"_key": dir(*descending)}),
                TermsOrder::Metric { name, descending } => json!({name: dir(*descending)}),
            };
            let mut body = Map::new();
            body.insert(
                "terms".into(),
                json!({"field": field_name(mapping, &terms.field, true), "size": terms.size, "order": order}),
            );
            sub(&terms.aggs, &mut body)?;
            Json::Object(body)
        }
    })
}

fn dir(descending: bool) -> &'static str {
    if descending {
        "desc"
    } else {
        "asc"
    }
}

/// Read an Elastic `aggregations` response into the facade's result shape.
pub fn read_aggs(aggs: &IndexMap<String, Agg>, response: &Json) -> IndexMap<String, Value> {
    let mut out = IndexMap::new();
    for (name, agg) in aggs {
        let part = response.get(name).unwrap_or(&Json::Null);
        out.insert(name.clone(), read_agg(agg, part));
    }
    out
}

fn read_agg(agg: &Agg, part: &Json) -> Value {
    match agg {
        Agg::Sum { .. } => {
            let value = part.get("value").cloned().unwrap_or(Json::Null);
            match moqui_lib::json::from_json(&value) {
                Value::Null => Value::Decimal(Default::default()),
                Value::I64(whole) => Value::Decimal(whole.into()),
                Value::F64(float) => moqui_lib::object::basic_convert(&Value::F64(float), "BigDecimal").value,
                other => other,
            }
        }
        Agg::CountDistinct { .. } => Value::I64(part.get("value").and_then(Json::as_i64).unwrap_or(0)),
        Agg::Filter { aggs, .. } => {
            let mut map = IndexMap::new();
            map.insert(
                "doc_count".into(),
                Value::I64(part.get("doc_count").and_then(Json::as_i64).unwrap_or(0)),
            );
            map.extend(read_aggs(aggs, part));
            Value::Map(map)
        }
        Agg::DateHistogram(DateHistogram { aggs, .. }) | Agg::Terms(Terms { aggs, .. }) => {
            let histogram = matches!(agg, Agg::DateHistogram(_));
            let mut buckets = Vec::new();
            for bucket in part.get("buckets").and_then(Json::as_array).into_iter().flatten() {
                let mut map = IndexMap::new();
                let key = bucket.get("key").cloned().unwrap_or(Json::Null);
                if histogram {
                    let at = key.as_i64().and_then(|millis| Timestamp::from_millisecond(millis).ok());
                    map.insert("key".into(), at.map_or(Value::Null, Value::Timestamp));
                    let shown = bucket.get("key_as_string").and_then(Json::as_str).unwrap_or_default();
                    map.insert("key_display".into(), Value::string(shown));
                } else {
                    map.insert("key".into(), moqui_lib::json::from_json(&key));
                }
                map.insert(
                    "doc_count".into(),
                    Value::I64(bucket.get("doc_count").and_then(Json::as_i64).unwrap_or(0)),
                );
                map.extend(read_aggs(aggs, bucket));
                buckets.push(Value::Map(map));
            }
            let mut map = IndexMap::new();
            map.insert("buckets".into(), Value::List(buckets));
            Value::Map(map)
        }
    }
}

pub struct OpenSearchBackend {
    client: RestClient,
    base_url: String,
    authorization: Option<String>,
    index_prefix: String,
    known: Mutex<HashMap<String, String>>,
}

impl OpenSearchBackend {
    pub fn new(conf: &ClusterConf) -> Self {
        let authorization = conf.user.as_ref().map(|user| {
            let pair = format!("{user}:{}", conf.password.clone().unwrap_or_default());
            format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(pair))
        });
        Self {
            client: RestClient::new(),
            base_url: conf.url.trim_end_matches('/').to_string(),
            authorization,
            index_prefix: conf.index_prefix.clone().unwrap_or_default(),
            known: Mutex::new(HashMap::new()),
        }
    }

    /// True when the cluster answers `GET /`.
    pub fn reachable(&self) -> bool {
        matches!(self.send("GET", "/", None), Ok((status, _)) if status < 300)
    }

    pub fn index_name(&self, mapping: &DocumentMapping) -> String {
        format!("{}{}", self.index_prefix, mapping.physical_name)
    }

    fn send(&self, method: &str, path: &str, body: Option<Body>) -> Result<(u16, Json), Error> {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let method =
            moqui_lib::http::Method::parse(method).ok_or_else(|| Error::new(format!("HTTP method {method}")))?;
        let mut call = self.client.request(method, url);
        if let Some(auth) = &self.authorization {
            call = call.header("authorization", auth.clone());
        }
        call = match body {
            Some(Body::Json(json)) => call.json(&json),
            Some(Body::NdJson(text)) => call.text("application/x-ndjson", text),
            None => call,
        };
        let response = call.send(false)?;
        let status = response.status();
        let json = if response.body().trim().is_empty() {
            Json::Null
        } else {
            serde_json::from_str(response.body()).unwrap_or_else(|_| Json::String(response.body().to_string()))
        };
        Ok((status, json))
    }

    fn checked(&self, method: &str, path: &str, body: Option<Body>) -> Result<Json, Error> {
        let (status, json) = self.send(method, path, body)?;
        if status >= 300 {
            let reason = json
                .pointer("/error/reason")
                .and_then(Json::as_str)
                .map_or_else(|| json.to_string(), str::to_string);
            return Err(Error::new(format!(
                "OpenSearch {method} {path} returned {status}: {reason}"
            )));
        }
        Ok(json)
    }

    fn create(&self, mapping: &DocumentMapping) -> Result<(), Error> {
        let index = self.index_name(mapping);
        let mut mappings = mapping_json(mapping);
        let mut settings = Json::Null;
        if let Some(service) = &mapping.manual_mapping_service {
            let mut params = std::collections::BTreeMap::new();
            params.insert("mapping".to_string(), moqui_lib::json::from_json(&mappings));
            if let Ok(result) = crate::service::sync(service, &params) {
                if let Some(changed) = result.get("mapping").filter(|value| !value.is_null()) {
                    mappings = moqui_lib::json::to_json(changed);
                }
                if let Some(given) = result.get("settings").filter(|value| !value.is_null()) {
                    settings = moqui_lib::json::to_json(given);
                }
            }
        }
        if let Some(needed) = settings_json(mapping) {
            settings = match settings {
                Json::Object(mut given) => {
                    if let Json::Object(needed) = needed {
                        for (key, value) in needed {
                            given.entry(key).or_insert(value);
                        }
                    }
                    Json::Object(given)
                }
                _ => needed,
            };
        }
        let mut body = Map::new();
        body.insert("mappings".into(), mappings);
        if !settings.is_null() {
            body.insert("settings".into(), settings);
        }
        if let Some(alias) = &mapping.index_name {
            body.insert("aliases".into(), json!({format!("{}{alias}", self.index_prefix): {}}));
        }
        let (status, json) = self.send("PUT", &index, Some(Body::Json(Json::Object(body))))?;
        // Another thread or node made it between the mapping check and here: the same definition.
        let exists = json.pointer("/error/type").and_then(Json::as_str) == Some("resource_already_exists_exception");
        if status >= 300 && !exists {
            let reason = json
                .pointer("/error/reason")
                .and_then(Json::as_str)
                .map_or_else(|| json.to_string(), str::to_string);
            return Err(Error::new(format!("OpenSearch PUT {index} returned {status}: {reason}")));
        }
        Ok(())
    }
}

enum Body {
    Json(Json),
    NdJson(String),
}

impl SearchBackend for OpenSearchBackend {
    fn keeps_log_indexes(&self) -> bool {
        true
    }

    fn has_elastic_api(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability::Proximity,
            Capability::Fuzzy,
            Capability::Autocomplete,
            Capability::WeightedRank,
            Capability::Hybrid,
        ]
    }

    fn name(&self) -> &'static str {
        "opensearch"
    }

    fn ensure_index(&self, mapping: &DocumentMapping) -> Result<(), Error> {
        let signature = mapping.signature_hash();
        if lock(&self.known).get(&mapping.document_id) == Some(&signature) {
            return Ok(());
        }
        let index = self.index_name(mapping);
        let (status, json) = self.send("GET", &format!("{index}/_mapping"), None)?;
        if status == 200 {
            let stored = json
                .get(&index)
                .and_then(|entry| entry.pointer("/mappings/_meta/moqui_signature"))
                .and_then(Json::as_str);
            if stored == Some(signature.as_str()) {
                lock(&self.known).insert(mapping.document_id.clone(), signature);
                return Ok(());
            }
            tracing::warn!(index = %index, "search index shape changed; the index is rebuilt and must be reindexed");
            self.checked("DELETE", &index, None)?;
        } else if status != 404 {
            return Err(Error::new(format!("OpenSearch GET {index}/_mapping returned {status}")));
        }
        self.create(mapping)?;
        lock(&self.known).insert(mapping.document_id.clone(), signature);
        Ok(())
    }

    fn index_exists(&self, mapping: &DocumentMapping) -> Result<bool, Error> {
        if lock(&self.known).contains_key(&mapping.document_id) {
            return Ok(true);
        }
        let (status, _) = self.send("GET", &format!("{}/_mapping", self.index_name(mapping)), None)?;
        Ok(status == 200)
    }

    fn index_documents(&self, mapping: &DocumentMapping, documents: &[(String, Document)]) -> Result<usize, Error> {
        if documents.is_empty() {
            return Ok(0);
        }
        let index = self.index_name(mapping);
        let now = Timestamp::now().as_millisecond();
        let mut text = String::new();
        // Log entries come without ids: OpenSearch makes them, and the write does not wait for a refresh.
        let generated_ids = documents.iter().all(|(id, _)| id.is_empty());
        for (id, body) in documents {
            let action = if id.is_empty() {
                json!({"index": {"_index": index}})
            } else {
                json!({"index": {"_index": index, "_id": id}})
            };
            text.push_str(&action.to_string());
            text.push('\n');
            let mut source = body_json(&Value::Map(body.clone()));
            if let Json::Object(map) = &mut source {
                map.insert(INDEXED_AT.into(), Json::from(now));
            }
            text.push_str(&source.to_string());
            text.push('\n');
        }
        let path = if generated_ids { "_bulk" } else { "_bulk?refresh=wait_for" };
        let response = self.checked("POST", path, Some(Body::NdJson(text)))?;
        if response.get("errors").and_then(Json::as_bool) == Some(true) {
            let first = response
                .get("items")
                .and_then(Json::as_array)
                .and_then(|items| items.iter().find_map(|item| item.pointer("/index/error/reason")))
                .map_or_else(String::new, Json::to_string);
            return Err(Error::new(format!("OpenSearch bulk index failed: {first}")));
        }
        Ok(documents.len())
    }

    fn delete_document(&self, mapping: &DocumentMapping, id: &str) -> Result<bool, Error> {
        let path = format!("{}/_doc/{}?refresh=wait_for", self.index_name(mapping), url_part(id));
        let (status, json) = self.send("DELETE", &path, None)?;
        match status {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(Error::new(format!(
                "OpenSearch DELETE {path} returned {status}: {json}"
            ))),
        }
    }

    fn delete_matching(&self, mapping: &DocumentMapping, filter: &Query) -> Result<u64, Error> {
        let path = format!("{}/_delete_by_query?refresh=true", self.index_name(mapping));
        let body = json!({"query": query_json(mapping, filter)});
        let response = self.checked("POST", &path, Some(Body::Json(body)))?;
        Ok(response.get("deleted").and_then(Json::as_u64).unwrap_or(0))
    }

    fn get_document(&self, mapping: &DocumentMapping, id: &str) -> Result<Option<Document>, Error> {
        let path = format!("{}/_doc/{}", self.index_name(mapping), url_part(id));
        let (status, json) = self.send("GET", &path, None)?;
        if status == 404 || json.get("found").and_then(Json::as_bool) == Some(false) {
            return Ok(None);
        }
        if status >= 300 {
            return Err(Error::new(format!("OpenSearch GET {path} returned {status}")));
        }
        Ok(Some(source_of(json.get("_source"))))
    }

    fn search(&self, mappings: &[Arc<DocumentMapping>], request: &SearchRequest) -> Result<SearchPage, Error> {
        let Some(first) = mappings.first() else {
            return Ok(SearchPage::default());
        };
        let indexes: Vec<String> = mappings.iter().map(|mapping| self.index_name(mapping)).collect();
        let mut body = search_json(first, request);
        if mappings.len() > 1 {
            // Each type's fields and roles differ: its own query, only on its own index.
            let should: Vec<Json> = mappings
                .iter()
                .zip(&indexes)
                .map(|(mapping, index)| {
                    json!({"bool": {"filter": [{"term": {"_index": index}}], "must": [request_query(mapping, request)]}})
                })
                .collect();
            if let Json::Object(map) = &mut body {
                map.insert("query".into(), json!({"bool": {"should": should, "minimum_should_match": 1}}));
            }
        }
        let response = self.checked(
            "POST",
            &format!("{}/_search", indexes.join(",")),
            Some(Body::Json(body)),
        )?;
        let total = response
            .pointer("/hits/total/value")
            .and_then(Json::as_u64)
            .unwrap_or(0);
        let mut hits = Vec::new();
        for hit in response
            .pointer("/hits/hits")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let index = hit.get("_index").and_then(Json::as_str).unwrap_or_default();
            let physical = index.strip_prefix(&self.index_prefix).unwrap_or(index).to_string();
            let document_id = mappings
                .iter()
                .find(|mapping| mapping.physical_name == physical)
                .map_or_else(
                    || super::mapping::document_id_of(&physical),
                    |mapping| mapping.document_id.clone(),
                );
            let body = source_of(hit.get("_source"));
            let highlights = if request.highlight.is_empty() {
                IndexMap::new()
            } else {
                super::highlight::highlights(&request.query, &request.highlight, &body)
            };
            hits.push(SearchHit {
                document_id,
                id: hit.get("_id").and_then(Json::as_str).unwrap_or_default().to_string(),
                index: physical,
                score: hit.get("_score").and_then(Json::as_f64).unwrap_or(0.0),
                body,
                highlights,
            });
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
        let body = json!({"size": 0, "query": {"bool": {"filter": [query_json(mapping, filter)]}},
            "aggs": aggs_json(mapping, aggs)?});
        let response = self.checked(
            "POST",
            &format!("{}/_search", self.index_name(mapping)),
            Some(Body::Json(body)),
        )?;
        Ok(read_aggs(aggs, response.get("aggregations").unwrap_or(&Json::Null)))
    }

    fn drop_index(&self, mapping: &DocumentMapping) -> Result<(), Error> {
        let index = self.index_name(mapping);
        let (status, json) = self.send("DELETE", &index, None)?;
        lock(&self.known).remove(&mapping.document_id);
        if status >= 300 && status != 404 {
            return Err(Error::new(format!(
                "OpenSearch DELETE {index} returned {status}: {json}"
            )));
        }
        Ok(())
    }
}

fn source_of(source: Option<&Json>) -> Document {
    match source.map(moqui_lib::json::from_json) {
        Some(Value::Map(mut map)) => {
            map.shift_remove(INDEXED_AT);
            map
        }
        _ => Document::new(),
    }
}

/// Percent-encode a document id for a URL path.
fn url_part(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::mapping::FieldMapping;
    use crate::search::parse::parse_query_string;

    fn sales() -> DocumentMapping {
        let field = |kind| FieldMapping { kind, sortable: false };
        let mut root = ObjectMapping::default();
        for (name, kind) in [
            ("orderId", FieldKind::Keyword),
            ("productId", FieldKind::Keyword),
            ("itemTypeEnumId", FieldKind::Keyword),
            ("placedDate", FieldKind::Date),
            ("itemSaleTotal", FieldKind::Double),
            ("statusId", FieldKind::Keyword),
        ] {
            root.fields.insert(name.into(), field(kind));
        }
        root.fields.insert(
            "combinedName".into(),
            FieldMapping {
                kind: FieldKind::Text,
                sortable: true,
            },
        );
        let mut issuances = ObjectMapping::default();
        issuances
            .fields
            .insert("issuedSaleTotal".into(), field(FieldKind::Double));
        root.children.insert("issuances".into(), issuances);
        DocumentMapping {
            document_id: "MantleSalesOrderItem".into(),
            index_name: Some("mantle_sales".into()),
            physical_name: "mantle_sales_order_item".into(),
            root,
            manual_mapping_service: None,
            display: Default::default(),
        }
    }

    #[test]
    fn sales_order_summary_body_matches_the_java_service() {
        let mapping = sales();
        let aggs = Agg::map_from_value(&moqui_lib::json::from_json(
            &serde_json::from_str(
                r#"{"DateOuter":{"date_histogram":{"field":"placedDate","calendar_interval":"1w","time_zone":"UTC",
                    "format":"dd MMM yyyy","order":{"_key":"desc"},"offset":"-1d"},
                  "aggs":{"orderCount":{"cardinality":{"field":"orderId","precision_threshold":40000}},
                          "ItemProduct":{"filter":{"term":{"itemTypeEnumId":"ItemProduct"}},
                             "aggs":{"productIssuedTotal":{"sum":{"field":"issuances.issuedSaleTotal"}}}}}}}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let json = aggs_json(&mapping, &aggs).unwrap();
        assert_eq!(
            json,
            json!({"DateOuter": {
            "date_histogram": {"field": "placedDate", "calendar_interval": "1w", "time_zone": "UTC",
                "format": "dd MMM yyyy", "offset": "-1d", "order": {"_key": "desc"}, "min_doc_count": 0},
            "aggs": {
                "ItemProduct": {"filter": {"term": {"itemTypeEnumId": "ItemProduct"}},
                    "aggs": {"productIssuedTotal": {"sum": {"field": "issuances.issuedSaleTotal"}}}},
                "orderCount": {"cardinality": {"field": "orderId", "precision_threshold": 40000}}
            }}})
        );
        let range = Query::range(
            "placedDate",
            Some(Value::string("2026-01-01")),
            Some(Value::I64(1_769_904_000_000)),
        );
        assert_eq!(
            query_json(&mapping, &range),
            json!({"range": {"placedDate": {"gte": 1_767_225_600_000_i64, "lt": 1_769_904_000_000_i64}}})
        );
    }

    #[test]
    fn query_strings_compile_to_query_string_and_nested() {
        let mapping = sales();
        let query = Query::all(vec![
            parse_query_string("*:jo* AND combinedName:\"jo smith\"").unwrap(),
            Query::child("issuances", Query::range("issuedSaleTotal", Some(Value::I64(1)), None)),
        ]);
        let json = query_json(&mapping, &query);
        let text = json.to_string();
        assert!(text.contains(r#""query":"jo*""#), "{text}");
        assert!(text.contains(r#""query":"\"jo smith\"""#), "{text}");
        assert!(
            text.contains(r#""nested":{"path":"issuances","query":{"range":{"issuances.issuedSaleTotal":{"gte":1}}}"#),
            "{text}"
        );
        let mut request = SearchRequest::new(Query::MatchAll);
        request.sort = vec![crate::search::SortField::parse("-combinedName").unwrap()];
        let body = search_json(&mapping, &request);
        assert_eq!(
            body["sort"],
            json!([{"combinedName.keyword": {"order": "desc", "missing": "_last", "mode": "max"}}])
        );
        assert_eq!(body["track_total_hits"], json!(true));
    }

    #[test]
    fn reads_bucket_responses() {
        let mapping = sales();
        let aggs = Agg::map_from_value(&moqui_lib::json::from_json(
            &serde_json::from_str(
                r#"{"T":{"terms":{"field":"productId"},"aggs":{"s":{"sum":{"field":"itemSaleTotal"}},
                    "F":{"filter":{"term":{"itemTypeEnumId":"ItemProduct"}}}}}}"#,
            )
            .unwrap(),
        ))
        .unwrap();
        let _ = aggs_json(&mapping, &aggs).unwrap();
        let response =
            json!({"T": {"buckets": [{"key": "P1", "doc_count": 2, "s": {"value": 130.5}, "F": {"doc_count": 1}}]}});
        let out = read_aggs(&aggs, &response);
        let Value::Map(terms) = &out["T"] else { panic!() };
        let Value::List(buckets) = &terms["buckets"] else {
            panic!()
        };
        let Value::Map(bucket) = &buckets[0] else { panic!() };
        assert_eq!(to_plain_string(&bucket["key"]), "P1");
        assert_eq!(bucket["doc_count"], Value::I64(2));
        assert_eq!(to_plain_string(&bucket["s"]), "130.5");
        let Value::Map(filter) = &bucket["F"] else { panic!() };
        assert_eq!(filter["doc_count"], Value::I64(1));
    }
}
