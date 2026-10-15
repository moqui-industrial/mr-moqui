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

//! Entity ECA match, the entity data loader, and the `DataFeedPending` insert.
//!
//! Matching ECA rules run their conditions and actions (`service::run_matched_eca`), and data-file
//! service elements are called through `service::run_data_file_service` and also recorded here.
//! Feed receive and delete service names are recorded, and `service::schedule_feed_receive` hands
//! the pending rows to a service worker after commit. Documents are indexed only when search is attached.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use moqui_db::model::Relationship;
use moqui_db::sql::{insert, quote_ident, select, Condition, FieldRef, LimitOffset};
use moqui_lib::error::Error;
use moqui_lib::node::MNode;
use moqui_lib::object::{basic_convert, Value};

use super::facade::EntityFacade;
use super::fk::FkScope;
use super::value::EntityValue;
use crate::driver::{DbConnection, DbRow};
use crate::transaction::{TransactionAttribute, TransactionManager};

const SKIP_FEED_ENTITY: &str = "moqui.entity.SequenceValueItem";
const PENDING_ENTITY: &str = "moqui.entity.feed.DataFeedPending";
/// Most primary documents one related write may reach before the rest are left to a rebuild.
const MAX_RELATED_DOCS: usize = 10_000;
const REALTIME_PUSH: &str = "DTFDTP_RT_PUSH";
const MAX_INFLATE: usize = 16 * 1024 * 1024;
/// Largest single XML element or JSON value the streaming loader holds at once.
const MAX_UNIT: usize = 64 * 1024 * 1024;
const MAX_ZIP_FILE: u64 = 256 * 1024 * 1024;
const SAVEPOINT: &str = "moqui_feed";

static NEXT_RANDOM: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Data-feed touches of each open transaction frame on this thread, by frame token.
    static TOUCHES: RefCell<HashMap<u64, Vec<Touch>>> = RefCell::new(HashMap::new());
}

/// One ECA rule. `actions` is the stored action body. It is not executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EcaRule {
    pub id: String,
    pub entity_name: String,
    pub on_create: bool,
    pub on_update: bool,
    pub on_delete: bool,
    pub run_before: bool,
    /// Fill fields the write does not set from the database row, for update and delete.
    pub get_entire_entity: bool,
    /// Put the database row before the write in `originalValue`, for update and delete.
    pub get_original_value: bool,
    /// Copy context names the actions set that are non-primary-key fields onto the value.
    pub set_results: bool,
    /// Run even when the message facade has errors.
    pub run_on_error: bool,
    pub condition: String,
    pub actions: String,
}

/// A data-file service element. It is called as it is read, and kept here for [`data_file_services`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataFileService {
    pub name: String,
    pub location: String,
    pub parameters: BTreeMap<String, String>,
}

/// A real-time feed's receive and delete service names, recorded as feed rows are inserted. A service worker calls them after commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedServiceNames {
    pub data_feed_id: String,
    pub receive_service: String,
    pub delete_service: String,
}

/// Filters and write mode for [`load_entity_data`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadOptions {
    pub data_types: Vec<String>,
    pub components: Vec<String>,
    pub only_create: bool,
    pub use_try_insert: bool,
    pub dummy_fks: bool,
    pub disable_fk_create: bool,
    /// Transaction timeout for each record. `None` uses the transaction manager default.
    pub timeout: Option<std::time::Duration>,
    /// Java `restrictSensitiveEntities`: refuse every record, root or nested, whose entity is a
    /// [`RESTRICTED_ENTITIES`] entity. Here service elements are refused too, a hardening beyond the Java, which ran
    /// them and checked only root and CSV entity names.
    pub restrict_sensitive_entities: bool,
}

/// The identity-admin and secret-config entities (`WebUtilities.isRestrictedGenericEntity`), by simple name in lower
/// case.
pub const RESTRICTED_ENTITIES: &[&str] = &[
    "userloginkey",
    "usergroupmember",
    "usergrouppermission",
    "userpermission",
    "artifactauthz",
    "artifactgroup",
    "artifactgroupmember",
    "artifacttarpit",
    "usergrouppermissions",
    "userpermissions",
    "artifactgroups",
    "userauthcfactor",
    "userpasswordhistory",
    "emailserver",
    "systemmessageremote",
];

/// How many entity records were written. Data-file service calls are not included.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadResult {
    pub records: u64,
}

struct Touch {
    entity_name: String,
    operation: String,
    /// The written values. The feed reads keys from them, and a related delete walks back to the
    /// primary entity from them. `None` falls back to the Postgres `xmin` probe.
    fields: Option<BTreeMap<String, Value>>,
}

#[derive(Default)]
struct LoadSlot {
    services: Vec<DataFileService>,
    /// By `data_feed_id`, so each commit replaces the names of the feeds it saw.
    feed_services: BTreeMap<String, FeedServiceNames>,
}

struct Record {
    entity: String,
    fields: BTreeMap<String, Value>,
    children: Vec<Record>,
}

struct Shape {
    full_name: String,
    fields: HashMap<String, String>,
    pk_names: Vec<String>,
    relations: HashMap<String, RelInfo>,
}

struct RelInfo {
    related: String,
    pairs: Vec<(String, String)>,
}

struct Agg {
    saw_write: bool,
    saw_delete: bool,
}

struct DocTarget {
    data_document_id: String,
    entity_name: String,
    primary: bool,
    primary_entity: String,
    /// Relationship steps from the primary entity to this one, `(entity, relationship)` each.
    paths: Vec<Vec<(String, String)>>,
}

fn with_load_slot<R>(facade: &EntityFacade, body: impl FnOnce(&mut LoadSlot) -> R) -> R {
    facade.with_slot(LoadSlot::default, body)
}

/// Match ECA rules for a write, and note the write for the data feed. Returns the ids of the matched rules.
///
/// `operation` is `create`, `update`, or `delete`. Do not call this from inside `with_slot`.
pub fn match_rules(facade: &EntityFacade, entity_name: &str, operation: &str) -> Vec<String> {
    let ids = match_rules_with(facade, entity_name, operation)
        .into_iter()
        .map(|rule| rule.id)
        .collect();
    note_touch(facade, entity_name, operation, None);
    ids
}

/// The ECA rules for a write, which the write hands to `service::run_matched_eca`: nothing is kept on the
/// facade, so a nested or concurrent write cannot change the rules of this one.
pub(crate) fn match_rules_with(facade: &EntityFacade, entity_name: &str, operation: &str) -> Vec<EcaRule> {
    if facade.flags().disable_entity_eca {
        return Vec::new();
    }
    eca_index(facade)
        .of_entity(entity_name)
        .filter(|rule| rule_matches(rule, entity_name, operation))
        .cloned()
        .collect()
}

/// Note a write for the data feed when it ran inside a transaction. Call it after the statement succeeded, so a
/// failed write leaves nothing. The data feed uses `fields` to find documents.
///
/// The frame's first touch registers dropping the frame's touches when it ends. The feed insert has taken them
/// before a commit; a rollback, or a frame with no feed insert, leaves nothing behind either.
pub(crate) fn note_touch(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    fields: Option<&BTreeMap<String, Value>>,
) {
    let Some(tx) = facade.transactions_for(entity_name).ok() else {
        return;
    };
    let Some(frame) = tx.frame_token() else {
        return;
    };
    if !TOUCHES.with(|touches| touches.borrow().contains_key(&frame))
        && (tx.on_commit(move || drop_frame_touches(frame)).is_err()
            || tx.on_rollback(move || drop_frame_touches(frame)).is_err())
    {
        return;
    }
    let keep_fields =
        !facade.flags().disable_data_feed && entity_name != SKIP_FEED_ENTITY && entity_name != PENDING_ENTITY;
    let touch = Touch {
        entity_name: entity_name.to_string(),
        operation: operation.to_string(),
        fields: fields.filter(|_| keep_fields).cloned(),
    };
    TOUCHES.with(|touches| touches.borrow_mut().entry(frame).or_default().push(touch));
}

fn drop_frame_touches(frame: u64) {
    TOUCHES.with(|touches| touches.borrow_mut().remove(&frame));
}

/// Forget the touches of every frame on this thread. Only for a thread with no transaction frame left.
pub(super) fn clear_frame_touches() {
    TOUCHES.with(|touches| touches.borrow_mut().clear());
}

/// Rules stored for `entity_name`. This lookup does not run them; `service::run_matched_eca` does.
pub fn eca_rules(facade: &EntityFacade, entity_name: &str) -> Vec<EcaRule> {
    eca_index(facade).of_entity(entity_name).cloned().collect()
}

/// How many entity ECA rules are loaded, for the tools artifact stats.
pub fn eca_rule_count(facade: &EntityFacade) -> usize {
    eca_index(facade).rules.len()
}

/// Add the `<eeca>` rules of an `<eecas>` document after the stored ones, for a test that needs its own rules.
#[cfg(test)]
pub(crate) fn add_eca_rules(facade: &EntityFacade, eecas: &str) -> Result<(), Error> {
    let root = MNode::parse_text("eecas", eecas)?;
    let mut index = (*eca_index(facade)).clone();
    index.ingest(&root);
    *facade.inner.eca_rules.write().unwrap_or_else(|err| err.into_inner()) = Some(Arc::new(index));
    Ok(())
}

/// Service elements the loader has read, in order. They were called as they were read.
pub fn data_file_services(facade: &EntityFacade) -> Vec<DataFileService> {
    with_load_slot(facade, |slot| slot.services.clone())
}

/// Receive and delete service names seen while inserting feed rows. A service worker calls them after commit, not the loader.
pub fn feed_service_names(facade: &EntityFacade) -> Vec<FeedServiceNames> {
    with_load_slot(facade, |slot| slot.feed_services.values().cloned().collect())
}

/// `load-data` locations from the framework conf. This does not load those files.
pub fn conf_data_locations(facade: &EntityFacade) -> Result<Vec<String>, Error> {
    let conf = MNode::parse_file(facade.framework_dir().join("conf/MoquiDefaultConf.xml"))?;
    let facade = conf
        .first_named("entity-facade")
        .ok_or_else(|| Error::new("conf has no entity-facade"))?;
    let mut locations = Vec::new();
    for node in facade.children_named("load-data") {
        if let Some(location) = node.attribute("location") {
            locations.push(location);
        }
    }
    Ok(locations)
}

/// Register the hooks for the open frame: before commit, insert `DataFeedPending` on the
/// enlisted connection; after commit, hand the inserted rows to the feed receive. The touches
/// drop themselves at rollback (see [`note_touch`]).
pub fn register_feed_insert(facade: &EntityFacade, tx: &TransactionManager) -> Result<(), Error> {
    let frame = tx.frame_token();
    let inserted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let insert_facade = facade.clone();
    let insert_ids = std::sync::Arc::clone(&inserted);
    tx.on_feed_insert(move |conn| {
        let ids = insert_pending(&insert_facade, conn, frame)?;
        insert_ids.lock().unwrap_or_else(|err| err.into_inner()).extend(ids);
        Ok(())
    })?;
    let commit_facade = facade.clone();
    tx.on_commit(move || {
        let ids = std::mem::take(&mut *inserted.lock().unwrap_or_else(|err| err.into_inner()));
        if let Err(err) = crate::service::schedule_feed_receive(&commit_facade, ids) {
            tracing::warn!(error = %err, "data feed receive was not scheduled; pending rows stay");
        }
    })
}

/// Load one XML, CSV, JSON, or zip location, or a directory of those files.
pub fn load_entity_data(facade: &EntityFacade, location: &str, options: &LoadOptions) -> Result<LoadResult, Error> {
    if !component_allowed(location, &options.components) {
        return Ok(LoadResult { records: 0 });
    }
    let _scope = FkScope::enter(options.dummy_fks, options.disable_fk_create);
    let path = resolve_location(facade, location)?;
    let records = load_path(facade, location, &path, options, 0)?;
    Ok(LoadResult { records })
}

/// Load entity XML text, as `EntityDataLoader.xmlText`. `location` names the text in errors.
pub fn load_entity_xml_text(
    facade: &EntityFacade,
    location: &str,
    text: &str,
    options: &LoadOptions,
) -> Result<LoadResult, Error> {
    let _scope = FkScope::enter(options.dummy_fks, options.disable_fk_create);
    let records = load_xml_text(facade, location, text, options)?;
    Ok(LoadResult { records })
}

fn rule_matches(rule: &EcaRule, entity_name: &str, operation: &str) -> bool {
    if rule.entity_name != entity_name {
        return false;
    }
    match operation {
        "create" => rule.on_create,
        "update" => rule.on_update,
        "delete" => rule.on_delete,
        _ => false,
    }
}

/// Entity ECA rules in file order, with the positions of each entity's rules, so matching a write is a map
/// lookup. Built once per facade; a reader holds the `Arc` and takes no lock while matching.
#[derive(Clone, Default)]
pub(crate) struct EcaIndex {
    rules: Vec<EcaRule>,
    by_entity: HashMap<String, Vec<usize>>,
}

impl EcaIndex {
    /// The rules of `entity_name`, in file order.
    fn of_entity<'a>(&'a self, entity_name: &str) -> impl Iterator<Item = &'a EcaRule> + 'a {
        self.by_entity
            .get(entity_name)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|at| &self.rules[*at])
    }

    /// Read every `*.eecas.xml` of `dirs`: the framework's rules, then each component's, as Java loads them from
    /// every entity directory.
    fn load(dirs: &[PathBuf]) -> Self {
        let mut index = Self::default();
        let mut files = 0;
        for dir in dirs {
            let read = match std::fs::read_dir(dir) {
                Ok(read) => read,
                Err(err) => {
                    tracing::warn!(error = %err, dir = %dir.display(), "could not read eeca directory");
                    continue;
                }
            };
            let mut paths: Vec<PathBuf> = read
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with(".eecas.xml"))
                })
                .collect();
            paths.sort();
            files += paths.len();
            for path in paths {
                match MNode::parse_file(&path) {
                    Ok(root) => index.ingest(&root),
                    Err(err) => tracing::warn!(error = %err, path = %path.display(), "could not parse eeca file"),
                }
            }
        }
        tracing::info!(
            "Loaded {} Entity ECA rules from {files} .eecas.xml files, {} rules have no id",
            index.rules.len(),
            index.rules.iter().filter(|rule| rule.id.is_empty()).count()
        );
        index
    }

    /// Add the `<eeca>` rules of `root`. A rule with the id of a stored one replaces it in place.
    fn ingest(&mut self, root: &MNode) {
        for node in root.children_named("eeca") {
            let rule = EcaRule {
                id: node.attribute("id").unwrap_or_default(),
                entity_name: node.attribute("entity").unwrap_or_default(),
                on_create: attr_true(&node, "on-create"),
                on_update: attr_true(&node, "on-update"),
                on_delete: attr_true(&node, "on-delete"),
                run_before: attr_true(&node, "run-before"),
                get_entire_entity: attr_true(&node, "get-entire-entity"),
                get_original_value: attr_true(&node, "get-original-value"),
                set_results: attr_true(&node, "set-results"),
                run_on_error: attr_true(&node, "run-on-error"),
                condition: node
                    .first_named("condition")
                    .map(|child| child.write_string())
                    .unwrap_or_default(),
                actions: node
                    .first_named("actions")
                    .map(|child| child.write_string())
                    .unwrap_or_default(),
            };
            if let Some(pos) = self
                .rules
                .iter()
                .position(|existing| existing.id == rule.id && !rule.id.is_empty())
            {
                self.rules[pos] = rule;
            } else {
                self.rules.push(rule);
            }
        }
        self.by_entity.clear();
        for (at, rule) in self.rules.iter().enumerate() {
            self.by_entity.entry(rule.entity_name.clone()).or_default().push(at);
        }
    }
}

/// The facade's ECA rules, read from the entity directories on first use.
fn eca_index(facade: &EntityFacade) -> Arc<EcaIndex> {
    let rules = &facade.inner.eca_rules;
    if let Some(index) = rules.read().unwrap_or_else(|err| err.into_inner()).as_ref() {
        return Arc::clone(index);
    }
    let mut slot = rules.write().unwrap_or_else(|err| err.into_inner());
    Arc::clone(slot.get_or_insert_with(|| Arc::new(EcaIndex::load(&facade.entity_dirs()))))
}

fn attr_true(node: &MNode, name: &str) -> bool {
    node.attribute(name).is_some_and(|value| value == "true")
}

/// Insert `DataFeedPending` rows for this frame's writes. Returns the inserted row ids.
fn insert_pending(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    frame: Option<u64>,
) -> Result<Vec<String>, Error> {
    let touches = take_frame_touches(frame);
    if facade.sim_active() || facade.flags().disable_data_feed {
        return Ok(Vec::new());
    }
    let aggs = aggregate_touches(&touches);
    if aggs.is_empty() {
        return Ok(Vec::new());
    }
    let (feed_sql, feed_params) = select_sql(
        facade,
        "moqui.entity.feed.DataFeed",
        &["dataFeedId", "feedReceiveServiceName", "feedDeleteServiceName"],
        Some(Condition::equals("dataFeedTypeEnumId", Value::string(REALTIME_PUSH))),
    )?;
    let feed_rows = probe_query(conn, &feed_sql, &feed_params)?;
    if feed_rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut services = Vec::new();
    let mut feed_ids = Vec::new();
    for row in &feed_rows {
        let data_feed_id = cell_string(row, "dataFeedId");
        if data_feed_id.is_empty() {
            continue;
        }
        services.push(FeedServiceNames {
            data_feed_id: data_feed_id.clone(),
            receive_service: cell_string(row, "feedReceiveServiceName"),
            delete_service: cell_string(row, "feedDeleteServiceName"),
        });
        feed_ids.push(data_feed_id);
    }
    remember_feed_services(facade, services);
    if feed_ids.is_empty() {
        return Ok(Vec::new());
    }
    let targets = feed_targets(facade, conn, &feed_ids)?;
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let insert_sql = pending_insert_sql(facade)?;
    let mut inserted = Vec::new();
    // Writes that carry their values: the document id is found from them, and the last write to
    // a document decides between index and delete.
    let mut resolved: indexmap::IndexMap<(String, String), (String, &'static str)> = indexmap::IndexMap::new();
    for touch in &touches {
        let Some(fields) = &touch.fields else {
            continue;
        };
        for hit in targets.iter().filter(|target| target.entity_name == touch.entity_name) {
            if hit.primary {
                let Some(id) = pk_text(facade, &touch.entity_name, fields) else {
                    continue;
                };
                let operation = if touch.operation == "delete" { "delete" } else { "index" };
                let key = (hit.data_document_id.clone(), id);
                resolved.shift_remove(&key);
                resolved.insert(key, (hit.primary_entity.clone(), operation));
            } else {
                let row = if touch.operation == "delete" {
                    fields.clone()
                } else {
                    reread(facade, conn, &touch.entity_name, fields)?.unwrap_or_else(|| fields.clone())
                };
                for path in &hit.paths {
                    for id in walk_to_primary(facade, conn, path, vec![row.clone()])? {
                        let key = (hit.data_document_id.clone(), id);
                        if !resolved.contains_key(&key) {
                            resolved.insert(key, (hit.primary_entity.clone(), "index"));
                        }
                    }
                }
            }
        }
    }
    for ((document_id, id), (entity_name, operation)) in &resolved {
        let row_id = random_id();
        let params = vec![
            Value::string(&row_id),
            Value::string(document_id),
            Value::string(entity_name),
            Value::string(id),
            Value::string(*operation),
        ];
        probe_execute(conn, &insert_sql, &params)?;
        inserted.push(row_id);
    }
    // Writes recorded without values, which only the Postgres xmin probe can key.
    let legacy: HashSet<&str> = touches
        .iter()
        .filter(|touch| touch.fields.is_none())
        .map(|touch| touch.entity_name.as_str())
        .collect();
    let mut seen = HashSet::new();
    for (entity_name, agg) in &aggs {
        if !legacy.contains(entity_name.as_str()) {
            continue;
        }
        let hits: Vec<&DocTarget> = targets
            .iter()
            .filter(|target| &target.entity_name == entity_name)
            .collect();
        if hits.is_empty() {
            continue;
        }
        let Some(keys) = primary_keys(facade, conn, entity_name)? else {
            continue;
        };
        for key in keys {
            for hit in &hits {
                let operation = if agg.saw_write {
                    "index"
                } else if hit.primary {
                    "delete"
                } else {
                    "index"
                };
                let dedup = (
                    hit.data_document_id.clone(),
                    entity_name.clone(),
                    key.clone(),
                    operation.to_string(),
                );
                if !seen.insert(dedup) {
                    continue;
                }
                let row_id = random_id();
                let params = vec![
                    Value::string(&row_id),
                    Value::string(&hit.data_document_id),
                    Value::string(entity_name),
                    Value::string(&key),
                    Value::string(operation),
                ];
                probe_execute(conn, &insert_sql, &params)?;
                inserted.push(row_id);
            }
        }
    }
    Ok(inserted)
}

/// Primary key values joined with `::`, or `None` when one is missing.
fn pk_text(facade: &EntityFacade, entity_name: &str, fields: &BTreeMap<String, Value>) -> Option<String> {
    let names = facade
        .with_entity(entity_name, |entity, _, _| {
            Ok(entity
                .pk_field_names()
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>())
        })
        .ok()?;
    let mut parts = Vec::with_capacity(names.len());
    for name in &names {
        let text = scalar_text(fields.get(name)?)?;
        if text.is_empty() {
            return None;
        }
        parts.push(text);
    }
    (!parts.is_empty()).then(|| parts.join("::"))
}

/// Every field of the row with these primary key values, read on the enlisted connection.
fn reread(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    entity_name: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<Option<BTreeMap<String, Value>>, Error> {
    let (names, pk_names) = facade.with_entity(entity_name, |entity, _, _| {
        Ok((
            entity.fields.iter().map(|field| field.name.clone()).collect::<Vec<_>>(),
            entity
                .pk_field_names()
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ))
    })?;
    let mut conds = Vec::new();
    for name in &pk_names {
        match fields.get(name) {
            Some(value) if !value.is_null() => {
                conds.push(Condition::equals(FieldRef::new(name.clone()), value.clone()))
            }
            _ => return Ok(None),
        }
    }
    let rows = select_rows(facade, conn, entity_name, &names, Condition::and(conds))?;
    Ok(rows.into_iter().next())
}

fn select_rows(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    entity_name: &str,
    names: &[String],
    condition: Condition,
) -> Result<Vec<BTreeMap<String, Value>>, Error> {
    let columns: Vec<&str> = names.iter().map(String::as_str).collect();
    let (sql, params) = select_sql(facade, entity_name, &columns, Some(condition))?;
    let rows = probe_query(conn, &sql, &params)?;
    Ok(rows
        .iter()
        .map(|row| {
            names
                .iter()
                .map(|name| (name.clone(), cell_value(row, name).cloned().unwrap_or(Value::Null)))
                .collect()
        })
        .collect())
}

/// Walk relationship steps back from rows of the last entity to the primary entity's ids.
fn walk_to_primary(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    path: &[(String, String)],
    rows: Vec<BTreeMap<String, Value>>,
) -> Result<Vec<String>, Error> {
    let mut rows = rows;
    for (parent, relationship) in path.iter().rev() {
        let (_, pairs) = super::query_rest::relationship_pairs(facade, parent, relationship)?;
        let names = facade.with_entity(parent, |entity, _, _| {
            Ok(entity.fields.iter().map(|field| field.name.clone()).collect::<Vec<_>>())
        })?;
        let mut next = Vec::new();
        let mut seen = HashSet::new();
        for row in &rows {
            let mut conds = Vec::with_capacity(pairs.len());
            for (field, related) in &pairs {
                match row.get(related) {
                    Some(value) if !value.is_null() => {
                        conds.push(Condition::equals(FieldRef::new(field.clone()), value.clone()))
                    }
                    _ => {
                        conds.clear();
                        break;
                    }
                }
            }
            if conds.is_empty() {
                continue;
            }
            let key: Vec<String> = conds.iter().map(|cond| format!("{cond:?}")).collect();
            if !seen.insert(key) {
                continue;
            }
            next.extend(select_rows(facade, conn, parent, &names, Condition::and(conds))?);
            if next.len() > MAX_RELATED_DOCS {
                tracing::warn!(
                    entity = %parent,
                    limit = MAX_RELATED_DOCS,
                    "a related write reaches too many documents; rebuild the feed to index the rest"
                );
                next.truncate(MAX_RELATED_DOCS);
                break;
            }
        }
        rows = next;
    }
    let primary = path.first().map(|(entity, _)| entity.clone()).unwrap_or_default();
    let mut ids = Vec::new();
    for row in &rows {
        if let Some(id) = pk_text(facade, &primary, row) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

fn aggregate_touches(touches: &[Touch]) -> HashMap<String, Agg> {
    let mut aggs = HashMap::new();
    for touch in touches {
        if touch.entity_name == SKIP_FEED_ENTITY {
            continue;
        }
        let agg = aggs.entry(touch.entity_name.clone()).or_insert(Agg {
            saw_write: false,
            saw_delete: false,
        });
        match touch.operation.as_str() {
            "create" | "update" => agg.saw_write = true,
            "delete" => agg.saw_delete = true,
            _ => {}
        }
    }
    aggs.retain(|_, agg| agg.saw_write || agg.saw_delete);
    aggs
}

/// Take this frame's touches, so a `force-new` commit takes only its own.
fn take_frame_touches(frame: Option<u64>) -> Vec<Touch> {
    let Some(frame) = frame else {
        return Vec::new();
    };
    TOUCHES
        .with(|touches| touches.borrow_mut().remove(&frame))
        .unwrap_or_default()
}

fn remember_feed_services(facade: &EntityFacade, services: Vec<FeedServiceNames>) {
    if services.is_empty() {
        return;
    }
    with_load_slot(facade, |slot| {
        for service in services {
            slot.feed_services.insert(service.data_feed_id.clone(), service);
        }
    });
}

fn feed_targets(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    feed_ids: &[String],
) -> Result<Vec<DocTarget>, Error> {
    let (link_sql, link_params) = select_sql(
        facade,
        "moqui.entity.feed.DataFeedDocument",
        &["dataFeedId", "dataDocumentId"],
        None,
    )?;
    let links = probe_query(conn, &link_sql, &link_params)?;
    let (doc_sql, doc_params) = select_sql(
        facade,
        "moqui.entity.document.DataDocument",
        &["dataDocumentId", "primaryEntityName"],
        None,
    )?;
    let documents = probe_query(conn, &doc_sql, &doc_params)?;
    let (field_sql, field_params) = select_sql(
        facade,
        "moqui.entity.document.DataDocumentField",
        &["dataDocumentId", "fieldPath"],
        None,
    )?;
    let fields = probe_query(conn, &field_sql, &field_params)?;
    let mut paths: HashMap<String, Vec<String>> = HashMap::new();
    for row in &fields {
        let document = cell_string(row, "dataDocumentId");
        let path = cell_string(row, "fieldPath");
        if document.is_empty() || path.is_empty() || path.contains('(') {
            continue;
        }
        paths.entry(document).or_default().push(path);
    }
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for link in &links {
        let feed_id = cell_string(link, "dataFeedId");
        if !feed_ids.iter().any(|id| id == &feed_id) {
            continue;
        }
        let document_id = cell_string(link, "dataDocumentId");
        let Some(primary) = documents.iter().find_map(|row| {
            if cell_string(row, "dataDocumentId") == document_id {
                let name = cell_string(row, "primaryEntityName");
                if name.is_empty() {
                    None
                } else {
                    Some(name)
                }
            } else {
                None
            }
        }) else {
            continue;
        };
        let Ok(primary_full) = canonical_name(facade, &primary) else {
            tracing::warn!(entity = %primary, document = %document_id, "data document primary entity is not defined");
            continue;
        };
        push_target(
            &mut targets,
            &mut seen,
            &document_id,
            &primary_full,
            &primary_full,
            None,
        );
        let related = related_entities(
            facade,
            &primary_full,
            paths.get(&document_id).map(Vec::as_slice).unwrap_or(&[]),
        );
        for (name, steps) in related {
            push_target(&mut targets, &mut seen, &document_id, &name, &primary_full, Some(steps));
        }
    }
    Ok(targets)
}

/// Add a target, or add a path to an existing one. `steps` is `None` for the primary entity.
fn push_target(
    targets: &mut Vec<DocTarget>,
    seen: &mut HashSet<(String, String)>,
    document_id: &str,
    entity_name: &str,
    primary_entity: &str,
    steps: Option<Vec<(String, String)>>,
) {
    if seen.insert((document_id.to_string(), entity_name.to_string())) {
        targets.push(DocTarget {
            data_document_id: document_id.to_string(),
            entity_name: entity_name.to_string(),
            primary: steps.is_none(),
            primary_entity: primary_entity.to_string(),
            paths: steps.into_iter().collect(),
        });
    } else if let Some(steps) = steps {
        if let Some(target) = targets
            .iter_mut()
            .find(|target| target.data_document_id == document_id && target.entity_name == entity_name)
        {
            if !target.paths.contains(&steps) {
                target.paths.push(steps);
            }
        }
    }
}

/// Each entity on a field path below the primary, with the `(entity, relationship)` steps that
/// reach it.
fn related_entities(facade: &EntityFacade, primary: &str, paths: &[String]) -> Vec<(String, Vec<(String, String)>)> {
    let mut names: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for path in paths {
        let parts: Vec<&str> = path.split(':').collect();
        if parts.len() < 2 {
            continue;
        }
        let mut current = primary.to_string();
        let mut steps: Vec<(String, String)> = Vec::new();
        for rel_name in &parts[..parts.len() - 1] {
            let next = facade
                .with_entity(&current, |entity, _, _| {
                    Ok(entity.relationship(rel_name).map(|rel| rel.related.clone()))
                })
                .ok()
                .flatten();
            let Some(next) = next else {
                break;
            };
            let Ok(full) = canonical_name(facade, &next) else {
                break;
            };
            steps.push((current.clone(), (*rel_name).to_string()));
            let entry = (full.clone(), steps.clone());
            if !names.contains(&entry) {
                names.push(entry);
            }
            current = full;
        }
    }
    names
}

fn primary_keys(
    facade: &EntityFacade,
    conn: &mut dyn DbConnection,
    entity_name: &str,
) -> Result<Option<Vec<String>>, Error> {
    let (sql, pk_names) = match xmin_query(facade, entity_name)? {
        Some(query) => query,
        None => {
            tracing::warn!(entity = %entity_name, "data feed primary keys need postgres xmin");
            return Ok(Some(Vec::new()));
        }
    };
    let rows = probe_query(conn, &sql, &[])?;
    let mut keys = Vec::new();
    for row in &rows {
        let mut parts = Vec::new();
        let mut complete = true;
        for name in &pk_names {
            match cell_pk(row, name) {
                Some(text) => parts.push(text),
                None => complete = false,
            }
        }
        if complete && !parts.is_empty() {
            keys.push(parts.join("::"));
        }
    }
    Ok(Some(keys))
}

fn xmin_query(facade: &EntityFacade, entity_name: &str) -> Result<Option<(String, Vec<String>)>, Error> {
    facade.with_entity(entity_name, |entity, dialect, _| {
        if !dialect.name.to_ascii_lowercase().starts_with("postgres") {
            return Ok(None);
        }
        let names: Vec<String> = entity.pk_field_names().into_iter().map(str::to_string).collect();
        if names.is_empty() {
            return Ok(None);
        }
        let columns = names
            .iter()
            .map(|name| quote_ident(dialect, name))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {columns} FROM {} WHERE xmin::text = txid_current()::text",
            quote_ident(dialect, &entity.table_name)
        );
        Ok(Some((sql, names)))
    })
}

fn pending_insert_sql(facade: &EntityFacade) -> Result<String, Error> {
    facade.with_entity("moqui.entity.feed.DataFeedPending", |entity, dialect, _| {
        insert(
            entity,
            dialect,
            &[
                "dataFeedPendingId",
                "dataDocumentId",
                "entityName",
                "primaryKey",
                "operation",
            ],
        )
    })
}

fn select_sql(
    facade: &EntityFacade,
    entity_name: &str,
    columns: &[&str],
    condition: Option<Condition>,
) -> Result<(String, Vec<Value>), Error> {
    facade.with_entity(entity_name, |entity, dialect, _| {
        let fields: Vec<FieldRef> = columns.iter().copied().map(FieldRef::new).collect();
        let statement = select(
            entity,
            dialect,
            &fields,
            condition.as_ref(),
            &[],
            LimitOffset::default(),
        )?;
        Ok((statement.sql, statement.params))
    })
}

fn probe_query(conn: &mut dyn DbConnection, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
    conn.savepoint(SAVEPOINT)?;
    match conn.query(sql, params) {
        Ok(rows) => {
            conn.release_savepoint(SAVEPOINT)?;
            Ok(rows)
        }
        Err(err) if relation_missing(&err) => {
            let _ = conn.rollback_to_savepoint(SAVEPOINT);
            let _ = conn.release_savepoint(SAVEPOINT);
            Ok(Vec::new())
        }
        Err(err) => {
            let _ = conn.rollback_to_savepoint(SAVEPOINT);
            let _ = conn.release_savepoint(SAVEPOINT);
            Err(err)
        }
    }
}

fn probe_execute(conn: &mut dyn DbConnection, sql: &str, params: &[Value]) -> Result<(), Error> {
    conn.savepoint(SAVEPOINT)?;
    match conn.execute(sql, params) {
        Ok(_) => {
            conn.release_savepoint(SAVEPOINT)?;
            Ok(())
        }
        Err(err) if relation_missing(&err) => {
            let _ = conn.rollback_to_savepoint(SAVEPOINT);
            let _ = conn.release_savepoint(SAVEPOINT);
            Ok(())
        }
        Err(err) => {
            let _ = conn.rollback_to_savepoint(SAVEPOINT);
            let _ = conn.release_savepoint(SAVEPOINT);
            Err(err)
        }
    }
}

fn relation_missing(err: &Error) -> bool {
    let text = error_chain(err).to_ascii_lowercase();
    let missing = text.contains("does not exist")
        || text.contains("doesn't exist")
        || text.contains("unknown table")
        || text.contains("42p01");
    missing && (text.contains("relation") || text.contains("table") || text.contains("42p01"))
}

fn error_chain(err: &Error) -> String {
    let mut text = String::new();
    let mut current = Some(err as &dyn std::error::Error);
    while let Some(item) = current {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&item.to_string());
        current = item.source();
    }
    text
}

fn cell_value<'a>(row: &'a DbRow, name: &str) -> Option<&'a Value> {
    row.columns
        .iter()
        .find(|(column, _)| column.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

fn cell_string(row: &DbRow, name: &str) -> String {
    match cell_value(row, name) {
        Some(value) => scalar_text(value).unwrap_or_default(),
        None => String::new(),
    }
}

fn cell_pk(row: &DbRow, name: &str) -> Option<String> {
    let text = cell_string(row, name);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::I32(number) => Some(number.to_string()),
        Value::I64(number) => Some(number.to_string()),
        Value::F64(number) => Some(number.to_string()),
        Value::Decimal(number) => Some(number.to_string()),
        Value::Timestamp(timestamp) => Some(timestamp.to_string()),
        Value::Date(date) => Some(date.to_string()),
        Value::Time(time) => Some(time.to_string()),
        Value::Bytes(_) | Value::List(_) | Value::Map(_) | Value::Object(_) => None,
    }
}

fn random_id() -> String {
    let mut bytes = [0_u8; 16];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok();
    if !filled {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let tick = NEXT_RANDOM.fetch_add(1, Ordering::Relaxed);
        bytes[..8].copy_from_slice(&nanos.to_le_bytes());
        bytes[8..].copy_from_slice(&tick.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format_uuid(&bytes)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if index == 4 || index == 6 || index == 8 || index == 10 {
            out.push('-');
        }
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

fn load_path(
    facade: &EntityFacade,
    location: &str,
    path: &Path,
    options: &LoadOptions,
    depth: u32,
) -> Result<u64, Error> {
    if depth > 4 {
        return Err(Error::new(format!("zip nesting is too deep at {location}")));
    }
    if path.is_dir() {
        return load_directory(facade, path, options, depth);
    }
    match extension(path).as_str() {
        "xml" => {
            let file = open_data(path, location)?;
            load_xml_reader(facade, location, BufReader::new(file), options)
        }
        "csv" => {
            let file = open_data(path, location)?;
            load_csv_reader(facade, location, file, options)
        }
        "json" => {
            let file = open_data(path, location)?;
            load_json_reader(facade, location, BufReader::new(file), options)
        }
        "zip" => {
            let file = open_data(path, location)?;
            let mut bytes = Vec::new();
            file.take(MAX_ZIP_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|err| Error::new(format!("could not read {location}")).with_source(err))?;
            if bytes.len() as u64 > MAX_ZIP_FILE {
                return Err(Error::new(format!("{location} is too large to load")));
            }
            load_zip_bytes(facade, location, &bytes, options, depth)
        }
        _ => Err(Error::new(format!("unsupported data file {location}"))),
    }
}

fn load_directory(facade: &EntityFacade, path: &Path, options: &LoadOptions, depth: u32) -> Result<u64, Error> {
    let mut names = Vec::new();
    let read = std::fs::read_dir(path)
        .map_err(|err| Error::new(format!("could not read {}", path.display())).with_source(err))?;
    for entry in read {
        let entry = entry.map_err(|err| Error::new(format!("could not read {}", path.display())).with_source(err))?;
        let child = entry.path();
        if child.is_file() && is_data_file(&child) {
            names.push(child);
        }
    }
    names.sort();
    let mut total = 0_u64;
    for child in names {
        let location = child.display().to_string();
        total = total.saturating_add(load_path(facade, &location, &child, options, depth)?);
    }
    Ok(total)
}

fn load_xml_text(facade: &EntityFacade, location: &str, text: &str, options: &LoadOptions) -> Result<u64, Error> {
    load_xml_reader(facade, location, std::io::Cursor::new(text.as_bytes()), options)
}

/// An entity-definition document is type `seed`. Only `<seed-data>` children are records.
fn load_definition_document<R: BufRead>(
    facade: &EntityFacade,
    location: &str,
    input: &mut Pull<R>,
    mark: usize,
    empty: bool,
    options: &LoadOptions,
) -> Result<u64, Error> {
    if skip_type(Some("seed"), &options.data_types) {
        return Ok(0);
    }
    if empty {
        return Ok(0);
    }
    input.read_to_end()?;
    let document = input.text_from(mark)?;
    let root = MNode::parse_text(location, &document)?;
    let mut seeds = Vec::new();
    collect_named(&root, "seed-data", &mut seeds);
    let mut total = 0_u64;
    for seed in seeds {
        for child in seed.children() {
            if child.name() == "seed-data" {
                continue;
            }
            match parse_top(facade, location, &child)? {
                Some(Top::Record(record)) => {
                    total = total.saturating_add(store_record(facade, options, &record)?);
                }
                Some(Top::Service(service)) => call_data_file_service(facade, options, service)?,
                None => {}
            }
        }
    }
    Ok(total)
}

fn collect_named(node: &MNode, name: &str, out: &mut Vec<MNode>) {
    if node.name() == name {
        out.push(node.clone());
    }
    for child in node.children() {
        collect_named(&child, name, out);
    }
}

fn load_xml_reader(
    facade: &EntityFacade,
    location: &str,
    reader: impl BufRead,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let mut input = Pull::new(reader);
    input.skip_bom()?;
    input.consume_match(xml_misc, "xml prologue")?;
    // Keep the root bytes. `advance` would drop them, and an `<entities>` file is parsed whole.
    input.hold = true;
    let mark = input.pos;
    let start = input.take_with(xml_start_tag, "xml root")?;
    if start.name != "seed-data" && start.name != "entity-facade-xml" {
        // Entity definition files carry seed data under `<entities>`. Any other root is a typo.
        if start.name != "entities" {
            return Err(Error::new(format!(
                "{location} root element {} is not entity-facade-xml, seed-data, or entities",
                start.name
            )));
        }
        return load_definition_document(facade, location, &mut input, mark, start.empty, options);
    }
    input.release_hold();
    let file_type = if start.name == "seed-data" {
        Some("seed".to_string())
    } else {
        start.attr("type").filter(|value| !value.is_empty())
    };
    if skip_type(file_type.as_deref(), &options.data_types) {
        return Ok(0);
    }
    if start.empty {
        return Ok(0);
    }
    let mut total = 0_u64;
    loop {
        input.consume_match(xml_misc, "xml prologue")?;
        if input.starts_with("</")? {
            input.consume_match(xml_close_tag, "xml end tag")?;
            break;
        }
        if input.exhausted()? {
            return Err(Error::new(format!("{location} ended before the root was closed")));
        }
        let element = input.take_text(xml_element_end, "xml element")?;
        let node = MNode::parse_text(location, &element)?;
        match parse_top(facade, location, &node)? {
            Some(Top::Record(record)) => total = total.saturating_add(store_record(facade, options, &record)?),
            Some(Top::Service(service)) => call_data_file_service(facade, options, service)?,
            None => {}
        }
    }
    input.consume_match(xml_misc, "xml prologue")?;
    if !input.exhausted()? {
        return Err(Error::new(format!("{location} has trailing content after the root")));
    }
    Ok(total)
}

fn load_csv_reader(
    facade: &EntityFacade,
    location: &str,
    reader: impl Read,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b',')
        .comment(Some(b'#'))
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(reader);
    let mut records = reader.records();
    let first = loop {
        match records.next() {
            Some(Ok(record)) if !record_blank(&record) => break record,
            Some(Ok(_)) => {}
            Some(Err(err)) => return Err(Error::new(format!("could not read csv {location}")).with_source(err)),
            None => return Err(Error::new(format!("csv {location} has no data"))),
        }
    };
    let entity_name = first.get(0).unwrap_or("").trim().to_string();
    if entity_name.is_empty() {
        return Err(Error::new(format!("csv {location} has no entity name")));
    }
    let file_type = first.get(1).map(str::trim).filter(|value| !value.is_empty());
    if skip_type(file_type, &options.data_types) {
        return Ok(0);
    }
    let header = loop {
        match records.next() {
            Some(Ok(record)) if !record_blank(&record) => break record,
            Some(Ok(_)) => {}
            Some(Err(err)) => return Err(Error::new(format!("could not read csv {location}")).with_source(err)),
            None => return Err(Error::new(format!("csv {location} has no header"))),
        }
    };
    let mut headers = Vec::new();
    for (index, field) in header.iter().enumerate() {
        if field.is_empty() || headers.iter().any(|(name, _)| name == field) {
            continue;
        }
        headers.push((field.to_string(), index));
    }
    let is_entity = entity_defined(facade, &entity_name);
    if !is_entity && !entity_name.contains('#') {
        return Err(Error::new(format!(
            "csv {location} names {entity_name}, which is not an entity or a service"
        )));
    }
    let mut total = 0_u64;
    for record in records {
        let record = record.map_err(|err| Error::new(format!("could not read csv {location}")).with_source(err))?;
        if record_blank(&record) {
            continue;
        }
        if is_entity {
            let mut fields = BTreeMap::new();
            for (name, index) in &headers {
                let Some(text) = record.get(*index) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                fields.insert(name.clone(), Value::string(text));
            }
            let stored = store_record(
                facade,
                options,
                &Record {
                    entity: entity_name.clone(),
                    fields,
                    children: Vec::new(),
                },
            )?;
            total = total.saturating_add(stored);
        } else {
            let mut parameters = BTreeMap::new();
            for (name, index) in &headers {
                if let Some(text) = record.get(*index).filter(|text| !text.is_empty()) {
                    parameters.insert(name.clone(), text.to_string());
                }
            }
            call_data_file_service(
                facade,
                options,
                DataFileService {
                    name: entity_name.clone(),
                    location: location.to_string(),
                    parameters,
                },
            )?;
        }
    }
    Ok(total)
}

fn load_json_text(facade: &EntityFacade, location: &str, text: &str, options: &LoadOptions) -> Result<u64, Error> {
    load_json_reader(facade, location, std::io::Cursor::new(text.as_bytes()), options)
}

fn load_json_reader(
    facade: &EntityFacade,
    location: &str,
    reader: impl BufRead,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let mut input = Pull::new(reader);
    input.skip_bom()?;
    let kind = input.take_with(json_start, "json root")?;
    match kind {
        JsonStart::Object => {
            let text = input.take_text(json_value_end, "json value")?;
            let loaded = load_json_text_value(facade, location, &text, options)?;
            input.consume_match(json_ws, "json space")?;
            if !input.exhausted()? {
                return Err(Error::new(format!("{location} json has trailing data")));
            }
            Ok(loaded)
        }
        JsonStart::Array => {
            let mut total = 0_u64;
            let mut file_type = None;
            let mut first = true;
            loop {
                input.consume_match(json_ws, "json space")?;
                if input.starts_with("]")? {
                    input.consume(1);
                    break;
                }
                if input.exhausted()? {
                    return Err(Error::new(format!("{location} json array ended early")));
                }
                let text = input.take_text(json_value_end, "json value")?;
                let value = parse_json(&text)?;
                let mut marker = false;
                if first {
                    first = false;
                    if let Json::Object(pairs) = &value {
                        if let Some(kind) = json_object_string(pairs, "_dataType") {
                            file_type = Some(kind);
                            marker = true;
                        }
                    }
                }
                if !marker && !skip_type(file_type.as_deref(), &options.data_types) {
                    total = total.saturating_add(load_json_document(facade, location, value, options)?);
                }
                let end = input.take_with(json_after_value, "json separator")?;
                if end {
                    break;
                }
            }
            Ok(total)
        }
    }
}

fn load_json_text_value(
    facade: &EntityFacade,
    location: &str,
    text: &str,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let root = parse_json(text)?;
    let (file_type, values) = json_documents(root)?;
    if skip_type(file_type.as_deref(), &options.data_types) {
        return Ok(0);
    }
    let mut total = 0_u64;
    for value in values {
        total = total.saturating_add(load_json_document(facade, location, value, options)?);
    }
    Ok(total)
}

fn load_json_document(facade: &EntityFacade, location: &str, value: Json, options: &LoadOptions) -> Result<u64, Error> {
    let Json::Object(pairs) = value else {
        tracing::warn!(location, "json value is not an object");
        return Ok(0);
    };
    let entity_name =
        json_object_string(&pairs, "_entity").ok_or_else(|| Error::new(format!("json {location} has no _entity")))?;
    if entity_defined(facade, &entity_name) {
        let record = json_record(facade, &entity_name, &pairs)?;
        store_record(facade, options, &record)
    } else if entity_name.contains('#') {
        call_data_file_service(
            facade,
            options,
            DataFileService {
                name: entity_name,
                location: location.to_string(),
                parameters: json_service_parameters(&pairs)?,
            },
        )?;
        Ok(0)
    } else {
        Err(Error::new(format!(
            "json {location} _entity {entity_name} is not an entity or a service"
        )))
    }
}

fn json_documents(root: Json) -> Result<(Option<String>, Vec<Json>), Error> {
    match root {
        Json::Object(pairs) => {
            let file_type = json_object_string(&pairs, "_dataType");
            Ok((file_type, vec![Json::Object(pairs)]))
        }
        Json::Array(mut values) => {
            let file_type = values.first().and_then(|value| {
                if let Json::Object(pairs) = value {
                    json_object_string(pairs, "_dataType")
                } else {
                    None
                }
            });
            // A leading array object that carries `_dataType` is the file type and is not a record.
            if file_type.is_some() {
                values.remove(0);
            }
            Ok((file_type, values))
        }
        _ => Err(Error::new("json root must be an object or an array")),
    }
}

fn json_record(facade: &EntityFacade, entity_name: &str, pairs: &[(String, Json)]) -> Result<Record, Error> {
    json_record_in(facade, entity_name, pairs, &BTreeMap::new())
}

fn json_record_in(
    facade: &EntityFacade,
    entity_name: &str,
    pairs: &[(String, Json)],
    ancestor: &BTreeMap<String, Value>,
) -> Result<Record, Error> {
    let shape = shape_of(facade, entity_name)?;
    let mut fields = BTreeMap::new();
    let mut nested_values = Vec::new();
    for (key, value) in pairs {
        if key == "_entity" || key == "_dataType" {
            continue;
        }
        match value {
            Json::Object(_) | Json::Array(_) => nested_values.push((key, value)),
            _ => {
                if !shape.fields.contains_key(key) {
                    tracing::warn!(entity = %shape.full_name, field = %key, "json field is not on the entity");
                    continue;
                }
                if let Some(converted) =
                    json_field_value(shape.fields.get(key).map(String::as_str).unwrap_or(""), value)?
                {
                    fields.insert(key.clone(), converted);
                }
            }
        }
    }
    let shared = shared_pks(&shape, &fields, ancestor);
    let mut children = Vec::new();
    for (key, value) in nested_values {
        let Some(rel) = shape.relations.get(key) else {
            if entity_defined(facade, key) {
                let mut nested = json_children(facade, key, value, &shared)?;
                for child in &mut nested {
                    inherit_pk_fields(facade, &child.entity, &shared, &mut child.fields)?;
                }
                children.extend(nested);
            } else {
                tracing::warn!(entity = %shape.full_name, field = %key, "json nested value is not a relationship");
            }
            continue;
        };
        let mut nested = json_children(facade, &rel.related, value, &shared)?;
        for child in &mut nested {
            apply_key_pairs(&fields, rel, &mut child.fields);
            inherit_pk_fields(facade, &child.entity, &shared, &mut child.fields)?;
        }
        children.extend(nested);
    }
    Ok(Record {
        entity: shape.full_name,
        fields,
        children,
    })
}

fn json_children(
    facade: &EntityFacade,
    entity_name: &str,
    value: &Json,
    ancestor: &BTreeMap<String, Value>,
) -> Result<Vec<Record>, Error> {
    match value {
        Json::Object(pairs) => Ok(vec![json_record_in(facade, entity_name, pairs, ancestor)?]),
        Json::Array(values) => {
            let mut records = Vec::new();
            for value in values {
                let Json::Object(pairs) = value else {
                    return Err(Error::new(format!("nested {entity_name} value is not an object")));
                };
                records.push(json_record_in(facade, entity_name, pairs, ancestor)?);
            }
            Ok(records)
        }
        _ => Err(Error::new(format!("nested {entity_name} value is not an object"))),
    }
}

fn json_service_parameters(pairs: &[(String, Json)]) -> Result<BTreeMap<String, String>, Error> {
    let mut parameters = BTreeMap::new();
    for (key, value) in pairs {
        if key == "_entity" || key == "_dataType" {
            continue;
        }
        match value {
            Json::Object(_) | Json::Array(_) => {
                return Err(Error::new(format!("service parameter {key} is not a scalar")));
            }
            Json::Null => {}
            Json::String(text) if text.is_empty() => {}
            other => {
                if let Some(text) = json_scalar_string(other) {
                    parameters.insert(key.clone(), text);
                }
            }
        }
    }
    Ok(parameters)
}

fn json_object_string(pairs: &[(String, Json)], key: &str) -> Option<String> {
    pairs.iter().find_map(|(name, value)| {
        if name == key {
            json_scalar_string(value).filter(|text| !text.is_empty())
        } else {
            None
        }
    })
}

fn json_scalar_string(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(text) => Some(text.clone()),
        Json::String(text) => Some(text.clone()),
        Json::Array(_) | Json::Object(_) => None,
    }
}

fn json_field_value(field_type: &str, value: &Json) -> Result<Option<Value>, Error> {
    match value {
        Json::Null => Ok(None),
        Json::String(text) if text.is_empty() => Ok(None),
        Json::String(text) if field_type == "binary-very-long" => Ok(Some(Value::Bytes(decode_binary(text)))),
        Json::String(text) => Ok(Some(Value::string(text))),
        Json::Bool(flag) => Ok(Some(Value::Bool(*flag))),
        Json::Number(text) => Ok(Some(parse_json_number(text))),
        Json::Array(_) | Json::Object(_) => Ok(None),
    }
}

fn parse_json_number(text: &str) -> Value {
    if text.contains(['.', 'e', 'E']) {
        text.parse::<f64>()
            .map(Value::F64)
            .unwrap_or_else(|_| Value::string(text))
    } else {
        text.parse::<i64>()
            .map(Value::I64)
            .unwrap_or_else(|_| Value::string(text))
    }
}

enum Top {
    Record(Record),
    Service(DataFileService),
}

fn parse_top(facade: &EntityFacade, location: &str, node: &MNode) -> Result<Option<Top>, Error> {
    let key = element_key(&node.name());
    if entity_defined(facade, &key) {
        Ok(Some(Top::Record(parse_record(facade, &key, node)?)))
    } else if let Some(name) =
        crate::service::define::defined_name(&key).or_else(|| key.contains('#').then(|| key.clone()))
    {
        Ok(Some(Top::Service(DataFileService {
            name,
            location: location.to_string(),
            parameters: service_parameters(node),
        })))
    } else {
        let failures = crate::service::define::registration_failures(&key);
        if failures.is_empty() {
            Err(Error::new(format!(
                "{location} element {} is not an entity or a service",
                node.name()
            )))
        } else {
            Err(Error::new(format!(
                "{location} element {} is a service that did not register: {}",
                node.name(),
                failures.join("; ")
            )))
        }
    }
}

fn parse_record(facade: &EntityFacade, entity_name: &str, node: &MNode) -> Result<Record, Error> {
    parse_record_in(facade, entity_name, node, &BTreeMap::new())
}

fn parse_record_in(
    facade: &EntityFacade,
    entity_name: &str,
    node: &MNode,
    ancestor: &BTreeMap<String, Value>,
) -> Result<Record, Error> {
    let shape = shape_of(facade, entity_name)?;
    let mut fields = BTreeMap::new();
    for (name, value) in node.attributes() {
        if !shape.fields.contains_key(&name) {
            tracing::warn!(entity = %shape.full_name, field = %name, "attribute is not a field");
            continue;
        }
        if value.is_empty() {
            continue;
        }
        let field_type = shape.fields.get(&name).map(String::as_str).unwrap_or("").to_string();
        fields.insert(name, field_text(&field_type, &value));
    }
    let mut pending = Vec::new();
    for child in node.children() {
        let key = element_key(&child.name());
        if shape.fields.contains_key(&key) {
            if let Some(text) = child.text() {
                let text = text.trim();
                if !text.is_empty() {
                    let field_type = shape.fields.get(&key).map(String::as_str).unwrap_or("");
                    fields.insert(key, field_text(field_type, text));
                }
            }
        } else {
            pending.push(child);
        }
    }
    // Field elements are in `fields` before nested records, so a later element still supplies a primary key.
    let shared = shared_pks(&shape, &fields, ancestor);
    let mut children = Vec::new();
    for child in pending {
        let key = element_key(&child.name());
        if let Some(rel) = shape.relations.get(&key) {
            let mut nested = parse_record_in(facade, &rel.related, &child, &shared)?;
            apply_key_pairs(&fields, rel, &mut nested.fields);
            inherit_pk_fields(facade, &nested.entity, &shared, &mut nested.fields)?;
            children.push(nested);
        } else if entity_defined(facade, &key) {
            let mut nested = parse_record_in(facade, &key, &child, &shared)?;
            inherit_pk_fields(facade, &nested.entity, &shared, &mut nested.fields)?;
            children.push(nested);
        } else {
            tracing::warn!(entity = %shape.full_name, element = %key, "element is not a field or relationship");
        }
    }
    Ok(Record {
        entity: shape.full_name,
        fields,
        children,
    })
}

fn service_parameters(node: &MNode) -> BTreeMap<String, String> {
    let mut parameters = BTreeMap::new();
    for (name, value) in node.attributes() {
        if !value.is_empty() {
            parameters.insert(name, value);
        }
    }
    for child in node.children() {
        if let Some(text) = child.text() {
            let text = text.trim();
            if !text.is_empty() {
                parameters.insert(element_key(&child.name()), text.to_string());
            }
        }
    }
    parameters
}

fn field_text(field_type: &str, text: &str) -> Value {
    if field_type == "binary-very-long" {
        Value::Bytes(decode_binary(text))
    } else {
        Value::string(text)
    }
}

fn decode_binary(text: &str) -> Vec<u8> {
    STANDARD
        .decode(text.trim())
        .unwrap_or_else(|_| text.as_bytes().to_vec())
}

fn apply_key_pairs(parent: &BTreeMap<String, Value>, rel: &RelInfo, child: &mut BTreeMap<String, Value>) {
    for (source, target) in &rel.pairs {
        if child.contains_key(target) {
            continue;
        }
        if let Some(value) = parent.get(source) {
            child.insert(target.clone(), value.clone());
        }
    }
}

/// This record's primary keys, then ancestor keys whose names are not already there.
fn shared_pks(
    shape: &Shape,
    fields: &BTreeMap<String, Value>,
    ancestor: &BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    let mut shared = BTreeMap::new();
    for name in &shape.pk_names {
        if let Some(value) = fields.get(name).filter(|value| !value.is_null()) {
            shared.insert(name.clone(), value.clone());
        }
    }
    for (name, value) in ancestor {
        if !value.is_null() {
            shared.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    shared
}

/// Copy primary-key values the child did not set. A key-map value already in `fields` stays.
fn inherit_pk_fields(
    facade: &EntityFacade,
    entity_name: &str,
    source: &BTreeMap<String, Value>,
    fields: &mut BTreeMap<String, Value>,
) -> Result<(), Error> {
    if source.is_empty() {
        return Ok(());
    }
    let names = facade.with_entity(entity_name, |entity, _, _| {
        let mut names = Vec::new();
        for name in source.keys() {
            if fields.get(name).is_some_and(|value| !value.is_null()) {
                continue;
            }
            if entity.field(name).is_some() {
                names.push(name.clone());
            }
        }
        Ok(names)
    })?;
    for name in names {
        if let Some(value) = source.get(&name).filter(|value| !value.is_null()) {
            fields.insert(name, value.clone());
        }
    }
    Ok(())
}

fn store_record(facade: &EntityFacade, options: &LoadOptions, record: &Record) -> Result<u64, Error> {
    if options.restrict_sensitive_entities {
        refuse_restricted(facade, record)?;
    }
    // Inside an outer transaction a failed record marks it rollback-only and gives back its join, so the
    // owner's rollback still ends the frame.
    facade
        .transactions_for(&record.entity)?
        .run(TransactionAttribute::UseOrBegin, options.timeout, || {
            write_tree(facade, options, record, &BTreeMap::new())
        })
}

/// Refuses the record when it or any nested record, by relationship name, short alias, or entity name, resolves to
/// a [`RESTRICTED_ENTITIES`] entity. Checked before anything of the tree is written.
fn refuse_restricted(facade: &EntityFacade, record: &Record) -> Result<(), Error> {
    let full_name = canonical_name(facade, &record.entity).unwrap_or_else(|_| record.entity.clone());
    if is_restricted_entity(&full_name) {
        return Err(Error::new(format!(
            "Entity {full_name} is not allowed in this data load"
        )));
    }
    for child in &record.children {
        refuse_restricted(facade, child)?;
    }
    Ok(())
}

/// `WebUtilities.isRestrictedGenericEntity`: the simple name, case-insensitive, is in [`RESTRICTED_ENTITIES`].
pub fn is_restricted_entity(entity_name: &str) -> bool {
    let simple = entity_name
        .rsplit(['.', '#'])
        .next()
        .unwrap_or(entity_name)
        .to_ascii_lowercase();
    RESTRICTED_ENTITIES.contains(&simple.as_str())
}

fn write_tree(
    facade: &EntityFacade,
    options: &LoadOptions,
    record: &Record,
    ancestor: &BTreeMap<String, Value>,
) -> Result<u64, Error> {
    let mut fields = record.fields.clone();
    inherit_pk_fields(facade, &record.entity, ancestor, &mut fields)?;
    prepare_fields(facade, &record.entity, &mut fields, options.only_create)?;
    // `prepare_fields` may have assigned a sequenced primary key. Children need that value.
    let shared = stored_pks(facade, &record.entity, &fields, ancestor)?;
    let partial = options.only_create && !pk_complete(facade, &record.entity, &fields)?;
    if partial {
        tracing::info!(entity = %record.entity, "onlyCreate skipped a partial primary key");
    }
    let exists = !partial && options.only_create && row_exists(facade, &record.entity, &fields)?;
    let mut total = 0_u64;
    if !partial && !exists {
        let try_insert = options.use_try_insert && try_insert_allowed(facade, &record.entity)?;
        if try_insert {
            write_upsert(facade, &record.entity, &fields)?;
        } else if options.only_create {
            value_from(facade, &record.entity, &fields)?.create(facade)?;
        } else {
            value_from(facade, &record.entity, &fields)?.create_or_update(facade)?;
        }
        total = 1;
    }
    for child in &record.children {
        total = total.saturating_add(write_tree(facade, options, child, &shared)?);
    }
    Ok(total)
}

fn stored_pks(
    facade: &EntityFacade,
    entity_name: &str,
    fields: &BTreeMap<String, Value>,
    ancestor: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    facade.with_entity(entity_name, |entity, _, _| {
        let mut shared = BTreeMap::new();
        for name in entity.pk_field_names() {
            if let Some(value) = fields.get(name).filter(|value| !value.is_null()) {
                shared.insert(name.to_string(), value.clone());
            }
        }
        for (name, value) in ancestor {
            if !value.is_null() {
                shared.entry(name.clone()).or_insert_with(|| value.clone());
            }
        }
        Ok(shared)
    })
}

fn prepare_fields(
    facade: &EntityFacade,
    entity_name: &str,
    fields: &mut BTreeMap<String, Value>,
    only_create: bool,
) -> Result<(), Error> {
    convert_fields(facade, entity_name, fields)?;
    // `EntityAutoServiceRunner.storeEntity`, which Java's loader uses for nested records: a primary key
    // `fromDate` that is not given is now.
    let from_date_pk = facade.with_entity(entity_name, |entity, _, _| {
        Ok(entity.pk_field_names().contains(&"fromDate"))
    })?;
    if from_date_pk && !only_create && fields.get("fromDate").is_none_or(Value::is_null) {
        fields.insert("fromDate".to_string(), Value::Timestamp(jiff::Timestamp::now()));
    }
    let (single, missing, only_last) = facade.with_entity(entity_name, |entity, _, _| {
        let names = entity.pk_field_names();
        let absent = |name: &&str| fields.get(*name).is_none_or(Value::is_null);
        let missing = names.iter().any(absent);
        // Every key but the last given: the last is a secondary sequence (`setSequencedIdSecondary`).
        let only_last = names.len() > 1
            && names.last().is_some_and(absent)
            && names[..names.len() - 1].iter().all(|name| !absent(name));
        Ok((names.len() == 1, missing, only_last))
    })?;
    if only_last && !only_create {
        let mut value = value_from(facade, entity_name, fields)?;
        value.set_sequenced_id_secondary(facade)?;
        let pk_name = facade.with_entity(entity_name, |entity, _, _| {
            Ok(entity.pk_field_names().last().copied().unwrap_or("").to_string())
        })?;
        if let Some(value) = value.get(&pk_name).cloned() {
            fields.insert(pk_name, value);
        }
    }
    if missing && single && !only_create {
        let mut value = value_from(facade, entity_name, fields)?;
        value.set_sequenced_id_primary(facade)?;
        let pk_name = facade.with_entity(entity_name, |entity, _, _| {
            Ok(entity.pk_field_names().first().copied().unwrap_or("").to_string())
        })?;
        if let Some(value) = value.get(&pk_name).cloned() {
            fields.insert(pk_name, value);
        }
    }
    Ok(())
}

fn convert_fields(facade: &EntityFacade, entity_name: &str, fields: &mut BTreeMap<String, Value>) -> Result<(), Error> {
    let updates = facade.with_entity(entity_name, |entity, _, model| {
        let mut updates = BTreeMap::new();
        for field in &entity.fields {
            let Some(value) = fields.get(&field.name) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            let Some(java) = model
                .dictionary_type(&field.field_type)
                .map(|entry| entry.java_type.clone())
            else {
                continue;
            };
            let mut converted = basic_convert(value, &java);
            // Java parses number text with the locale (`L10nFacade.parseNumber`), so `118,500` is 118500.
            if let Value::String(text) = value {
                if !converted.changed && text.contains(',') && is_number_type(&java) {
                    let plain = Value::String(text.replace(',', ""));
                    let retried = basic_convert(&plain, &java);
                    if retried.changed {
                        converted = retried;
                    }
                }
            }
            updates.insert(field.name.clone(), converted.value);
        }
        Ok(updates)
    })?;
    fields.extend(updates);
    Ok(())
}

fn is_number_type(java_type: &str) -> bool {
    matches!(
        java_type.rsplit('.').next().unwrap_or(java_type),
        "BigDecimal" | "BigInteger" | "Long" | "Integer" | "Double" | "Float" | "long" | "int" | "double" | "float"
    )
}

fn pk_complete(facade: &EntityFacade, entity_name: &str, fields: &BTreeMap<String, Value>) -> Result<bool, Error> {
    facade.with_entity(entity_name, |entity, _, _| {
        Ok(entity
            .pk_field_names()
            .iter()
            .all(|name| fields.get(*name).is_some_and(|value| !value.is_null())))
    })
}

fn row_exists(facade: &EntityFacade, entity_name: &str, fields: &BTreeMap<String, Value>) -> Result<bool, Error> {
    let keys = facade.with_entity(entity_name, |entity, _, _| {
        let mut keys = BTreeMap::new();
        for name in entity.pk_field_names() {
            let value = fields
                .get(name)
                .filter(|value| !value.is_null())
                .cloned()
                .ok_or_else(|| Error::new(format!("primary key {name} is required on {}", entity.full_name)))?;
            keys.insert(name.to_string(), value);
        }
        Ok(keys)
    })?;
    let mut find = facade.find(entity_name)?;
    for (field, value) in keys {
        find = find.eq(&field, value);
    }
    Ok(find.one()?.is_some())
}

fn try_insert_allowed(facade: &EntityFacade, entity_name: &str) -> Result<bool, Error> {
    facade.with_entity(entity_name, |_, dialect, _| Ok(!dialect.never_try_insert))
}

fn write_upsert(facade: &EntityFacade, entity_name: &str, fields: &BTreeMap<String, Value>) -> Result<(), Error> {
    facade.upsert_record(entity_name, fields)
}

fn value_from(
    facade: &EntityFacade,
    entity_name: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<EntityValue, Error> {
    let mut value = EntityValue::make(facade, entity_name)?;
    for (name, field) in fields {
        value.set(name, field.clone());
    }
    Ok(value)
}

fn call_data_file_service(facade: &EntityFacade, options: &LoadOptions, service: DataFileService) -> Result<(), Error> {
    // A restricted load takes records only: a service element could write anything.
    if options.restrict_sensitive_entities {
        tracing::warn!(
            "Refusing service {} from {}: restricted data loads take records only",
            service.name,
            service.location
        );
        return Err(Error::new(format!(
            "Service {} is not allowed in this data load",
            service.name
        )));
    }
    with_load_slot(facade, |slot| slot.services.push(service.clone()));
    crate::service::run_data_file_service(facade, &service)
}

fn shape_of(facade: &EntityFacade, entity_name: &str) -> Result<Shape, Error> {
    facade.with_entity(entity_name, |entity, _, model| {
        let mut fields = HashMap::new();
        for field in &entity.fields {
            fields.insert(field.name.clone(), field.field_type.clone());
        }
        let mut relations = HashMap::new();
        for relationship in &entity.relationships {
            let related = model
                .entity(&relationship.related)
                .map(|related| related.full_name.clone())
                .unwrap_or_else(|_| relationship.related.clone());
            let info = RelInfo {
                related,
                pairs: key_pairs(relationship),
            };
            for key in relation_keys(relationship) {
                relations.entry(key).or_insert_with(|| RelInfo {
                    related: info.related.clone(),
                    pairs: info.pairs.clone(),
                });
            }
        }
        let pk_names = entity.pk_field_names().into_iter().map(str::to_string).collect();
        Ok(Shape {
            full_name: entity.full_name.clone(),
            fields,
            pk_names,
            relations,
        })
    })
}

fn key_pairs(relationship: &Relationship) -> Vec<(String, String)> {
    relationship
        .key_map
        .iter()
        .map(|pair| {
            let target = pair.related.clone().unwrap_or_else(|| pair.field_name.clone());
            (pair.field_name.clone(), target)
        })
        .collect()
}

fn relation_keys(relationship: &Relationship) -> Vec<String> {
    let mut keys = vec![relationship.name.clone()];
    if let Some(alias) = &relationship.short_alias {
        if !keys.iter().any(|key| key == alias) {
            keys.push(alias.clone());
        }
    }
    if relationship.title.is_none() {
        for name in [&relationship.related_entity_name, &relationship.related] {
            if !keys.iter().any(|key| key == name) {
                keys.push(name.clone());
            }
        }
    }
    keys
}

fn entity_defined(facade: &EntityFacade, name: &str) -> bool {
    facade.with_entity(name, |_, _, _| Ok(())).is_ok()
}

fn canonical_name(facade: &EntityFacade, name: &str) -> Result<String, Error> {
    facade.with_entity(name, |entity, _, _| Ok(entity.full_name.clone()))
}

fn element_key(name: &str) -> String {
    let rest = name.rsplit(':').next().unwrap_or(name);
    rest.replace('-', "#")
}

fn skip_type(file_type: Option<&str>, data_types: &[String]) -> bool {
    match file_type {
        Some(file_type) if !data_types.is_empty() => !data_types.iter().any(|allowed| allowed == file_type),
        _ => false,
    }
}

fn component_allowed(location: &str, components: &[String]) -> bool {
    if components.is_empty() {
        return true;
    }
    match component_name(location) {
        Some(name) => components.iter().any(|allowed| allowed == name),
        None => true,
    }
}

fn component_name(location: &str) -> Option<&str> {
    let rest = location
        .strip_prefix("component://")
        .or_else(|| location.strip_prefix("component:"))?;
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return None;
    }
    rest.split('/').next()
}

/// `classpath://` is under the framework directory this facade was opened from. Other schemes
/// use the entered factory's components.
fn resolve_location(facade: &EntityFacade, location: &str) -> Result<PathBuf, Error> {
    for prefix in ["classpath://", "classpath:"] {
        if let Some(rest) = location.strip_prefix(prefix) {
            return Ok(facade.framework_dir().join(rest.trim_start_matches('/')));
        }
    }
    crate::context::locate(location).map_err(|err| Error::new(err.message().to_string()))
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn is_data_file(path: &Path) -> bool {
    matches!(extension(path).as_str(), "xml" | "csv" | "json" | "zip")
}

fn open_data(path: &Path, location: &str) -> Result<std::fs::File, Error> {
    std::fs::File::open(path).map_err(|err| Error::new(format!("could not read {location}")).with_source(err))
}

fn record_blank(record: &csv::StringRecord) -> bool {
    record.iter().all(|field| field.is_empty())
}

struct Pull<R> {
    reader: R,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    /// When set, consumed bytes stay in `buf` so a non-data root can be parsed whole.
    hold: bool,
}

impl<R: BufRead> Pull<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::new(),
            pos: 0,
            eof: false,
            hold: false,
        }
    }

    fn read_to_end(&mut self) -> Result<(), Error> {
        while !self.eof {
            self.fill()?;
        }
        Ok(())
    }

    fn text_from(&self, mark: usize) -> Result<String, Error> {
        if mark > self.buf.len() {
            return Err(Error::new("xml root was dropped before it could be parsed"));
        }
        let bytes = &self.buf[mark..];
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|err| Error::new("data is not utf-8").with_source(err))
    }

    fn release_hold(&mut self) {
        self.hold = false;
        if self.pos >= 8192 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }

    fn skip_bom(&mut self) -> Result<(), Error> {
        loop {
            let have = self.buf.len() - self.pos;
            if have >= 3 || self.eof {
                break;
            }
            let before = self.buf.len();
            self.fill()?;
            if self.buf.len() == before {
                break;
            }
        }
        if self.buf[self.pos..].starts_with(&[0xef, 0xbb, 0xbf]) {
            self.advance(3);
        }
        Ok(())
    }

    fn starts_with(&mut self, prefix: &str) -> Result<bool, Error> {
        let needle = prefix.as_bytes();
        loop {
            let have = self.buf.len() - self.pos;
            if have >= needle.len() || self.eof {
                return Ok(self.buf[self.pos..].starts_with(needle));
            }
            let before = self.buf.len();
            self.fill()?;
            if self.buf.len() == before {
                return Ok(self.buf[self.pos..].starts_with(needle));
            }
        }
    }

    fn consume(&mut self, n: usize) {
        self.advance(n);
    }

    fn exhausted(&mut self) -> Result<bool, Error> {
        if self.pos < self.buf.len() {
            return Ok(false);
        }
        if !self.eof {
            self.fill()?;
        }
        Ok(self.pos >= self.buf.len())
    }

    fn consume_match(
        &mut self,
        scan: impl Fn(&str, bool) -> Result<Option<usize>, Error>,
        label: &str,
    ) -> Result<(), Error> {
        self.drive(|text, eof| Ok(scan(text, eof)?.map(|n| (n, ()))), label)?;
        Ok(())
    }

    fn take_text(
        &mut self,
        scan: impl Fn(&str, bool) -> Result<Option<usize>, Error>,
        label: &str,
    ) -> Result<String, Error> {
        self.drive(
            |text, eof| {
                let Some(n) = scan(text, eof)? else {
                    return Ok(None);
                };
                if n > text.len() || !text.is_char_boundary(n) {
                    return Err(Error::new("scan left the buffer"));
                }
                Ok(Some((n, text[..n].to_string())))
            },
            label,
        )
    }

    fn take_with<T>(
        &mut self,
        scan: impl Fn(&str, bool) -> Result<Option<(usize, T)>, Error>,
        label: &str,
    ) -> Result<T, Error> {
        self.drive(scan, label)
    }

    fn drive<T>(
        &mut self,
        mut scan: impl FnMut(&str, bool) -> Result<Option<(usize, T)>, Error>,
        label: &str,
    ) -> Result<T, Error> {
        loop {
            if self.pos >= self.buf.len() && !self.eof {
                self.fill()?;
            }
            let decision = {
                let (text, at_eof) = self.view()?;
                scan(text, at_eof)?
            };
            match decision {
                Some((n, value)) => {
                    let (text, _) = self.view()?;
                    if n > text.len() || !text.is_char_boundary(n) {
                        return Err(Error::new(format!("{label} scan left the buffer")));
                    }
                    self.advance(n);
                    return Ok(value);
                }
                None => {
                    let (_, at_eof) = self.view()?;
                    if at_eof {
                        return Err(Error::new(format!("{label} ended early")));
                    }
                    // Rescanning after every chunk is quadratic for one large element. Waiting for the
                    // pending text to double keeps the total scan linear.
                    let pending = self.buf.len() - self.pos;
                    if pending > MAX_UNIT {
                        return Err(Error::new(format!("{label} is larger than {MAX_UNIT} bytes")));
                    }
                    let target = pending.saturating_mul(2).max(pending + 8192);
                    while self.buf.len() - self.pos < target && !self.eof {
                        self.fill()?;
                    }
                }
            }
        }
    }

    fn view(&self) -> Result<(&str, bool), Error> {
        let bytes = &self.buf[self.pos..];
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok((text, self.eof)),
            Err(err) if err.error_len().is_none() && !self.eof => {
                let valid = err.valid_up_to();
                let text = std::str::from_utf8(&bytes[..valid]).map_err(|_| Error::new("data is not utf-8"))?;
                Ok((text, false))
            }
            Err(err) => Err(Error::new("data is not utf-8").with_source(err)),
        }
    }

    fn fill(&mut self) -> Result<(), Error> {
        if self.eof {
            return Ok(());
        }
        let mut chunk = [0_u8; 8192];
        loop {
            match self.reader.read(&mut chunk) {
                Ok(0) => {
                    self.eof = true;
                    return Ok(());
                }
                Ok(n) => {
                    self.buf.extend_from_slice(&chunk[..n]);
                    return Ok(());
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(Error::new("could not read data").with_source(err)),
            }
        }
    }

    fn advance(&mut self, n: usize) {
        self.pos += n;
        if !self.hold && self.pos >= 8192 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
    }
}

struct Start {
    name: String,
    empty: bool,
    attrs: Vec<(String, String)>,
}

impl Start {
    fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }
}

enum JsonStart {
    Object,
    Array,
}

fn xml_misc(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    if text.is_empty() {
        return if eof { Ok(Some(0)) } else { Ok(None) };
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    loop {
        let next = skip_xml_ws(text, i);
        if next >= bytes.len() {
            return if eof { Ok(Some(next)) } else { Ok(None) };
        }
        i = next;
        if bytes[i] != b'<' {
            return Ok(Some(i));
        }
        let rest = &bytes[i..];
        if rest.starts_with(b"<!--") {
            match find_marker(rest, b"-->") {
                Some(n) => i += n,
                None if eof => return Err(Error::new("unclosed comment")),
                None => return Ok(None),
            }
            continue;
        }
        if rest.starts_with(b"<?") {
            match find_marker(rest, b"?>") {
                Some(n) => i += n,
                None if eof => return Err(Error::new("unclosed processing instruction")),
                None => return Ok(None),
            }
            continue;
        }
        if doctype_prefix(rest) {
            match skip_doctype(rest, eof)? {
                Some(n) => i += n,
                None => return Ok(None),
            }
            continue;
        }
        if misc_prefix(rest) {
            return if eof { Ok(Some(i)) } else { Ok(None) };
        }
        return Ok(Some(i));
    }
}

fn xml_start_tag(text: &str, eof: bool) -> Result<Option<(usize, Start)>, Error> {
    if text.is_empty() {
        return if eof {
            Err(Error::new("xml root ended early"))
        } else {
            Ok(None)
        };
    }
    let bytes = text.as_bytes();
    if bytes[0] != b'<' {
        return Err(Error::new("xml root does not start with '<'"));
    }
    if bytes.len() == 1 {
        return if eof {
            Err(Error::new("xml root ended early"))
        } else {
            Ok(None)
        };
    }
    if matches!(bytes[1], b'/' | b'!' | b'?') {
        return Err(Error::new("xml root is not an element"));
    }
    let Some((name, mut i)) = read_xml_name(text, 1, eof)? else {
        return Ok(None);
    };
    let mut attrs = Vec::new();
    loop {
        let Some(next) = skip_ws_index(text, i, eof)? else {
            return Ok(None);
        };
        i = next;
        if i >= bytes.len() {
            return if eof {
                Err(Error::new("unclosed start tag"))
            } else {
                Ok(None)
            };
        }
        if bytes[i] == b'/' {
            if i + 1 >= bytes.len() {
                return if eof {
                    Err(Error::new("unclosed start tag"))
                } else {
                    Ok(None)
                };
            }
            if bytes[i + 1] != b'>' {
                return Err(Error::new("xml start tag is not closed"));
            }
            return Ok(Some((
                i + 2,
                Start {
                    name,
                    empty: true,
                    attrs,
                },
            )));
        }
        if bytes[i] == b'>' {
            return Ok(Some((
                i + 1,
                Start {
                    name,
                    empty: false,
                    attrs,
                },
            )));
        }
        let Some((attr_name, next)) = read_xml_name(text, i, eof)? else {
            return Ok(None);
        };
        i = next;
        let Some(next) = skip_ws_index(text, i, eof)? else {
            return Ok(None);
        };
        i = next;
        if i >= bytes.len() {
            return if eof {
                Err(Error::new("attribute value is missing"))
            } else {
                Ok(None)
            };
        }
        if bytes[i] != b'=' {
            return Err(Error::new(format!("attribute {attr_name} has no value")));
        }
        i += 1;
        let Some(next) = skip_ws_index(text, i, eof)? else {
            return Ok(None);
        };
        i = next;
        let Some((value, next)) = read_attr_value(text, i, eof)? else {
            return Ok(None);
        };
        i = next;
        attrs.push((attr_name, value));
    }
}

fn xml_close_tag(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return if eof {
            Err(Error::new("xml end tag ended early"))
        } else {
            Ok(None)
        };
    }
    if !bytes.starts_with(b"</") {
        return Err(Error::new("xml end tag does not start with '</'"));
    }
    if bytes.len() < 3 {
        return if eof {
            Err(Error::new("xml end tag ended early"))
        } else {
            Ok(None)
        };
    }
    let Some((_, mut i)) = read_xml_name(text, 2, eof)? else {
        return Ok(None);
    };
    let Some(next) = skip_ws_index(text, i, eof)? else {
        return Ok(None);
    };
    i = next;
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("xml end tag is not closed"))
        } else {
            Ok(None)
        };
    }
    if bytes[i] != b'>' {
        return Err(Error::new("xml end tag is not closed"));
    }
    Ok(Some(i + 1))
}

fn xml_element_end(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return if eof {
            Err(Error::new("xml element ended early"))
        } else {
            Ok(None)
        };
    }
    if bytes[0] != b'<' {
        return Err(Error::new("xml element does not start with '<'"));
    }
    let mut i = 0;
    let mut depth = 0_i32;
    loop {
        if i >= bytes.len() {
            return if eof {
                Err(Error::new("xml element ended early"))
            } else {
                Ok(None)
            };
        }
        if bytes[i] != b'<' {
            if depth == 0 {
                return Err(Error::new("xml element does not start with '<'"));
            }
            i += 1;
            continue;
        }
        match skip_special(&bytes[i..], eof)? {
            Special::Pending => return Ok(None),
            Special::Comment(n) | Special::Cdata(n) | Special::Pi(n) => {
                i += n;
                continue;
            }
            Special::Decl => return Err(Error::new("this declaration is not allowed")),
            Special::Tag => {}
        }
        let Some((consumed, kind)) = scan_tag(&bytes[i..], eof)? else {
            return if eof {
                Err(Error::new("xml element ended early"))
            } else {
                Ok(None)
            };
        };
        if consumed == 0 {
            return Err(Error::new("xml element ended early"));
        }
        i += consumed;
        match kind {
            TagKind::Empty => {
                if depth == 0 {
                    return Ok(Some(i));
                }
            }
            TagKind::Open => depth += 1,
            TagKind::Close => {
                if depth == 0 {
                    return Err(Error::new("xml end tag does not match"));
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(i));
                }
            }
        }
    }
}

enum TagKind {
    Empty,
    Open,
    Close,
}

enum Special {
    Pending,
    Comment(usize),
    Cdata(usize),
    Pi(usize),
    Decl,
    Tag,
}

fn skip_special(bytes: &[u8], eof: bool) -> Result<Special, Error> {
    if bytes.starts_with(b"<!--") {
        return match find_marker(bytes, b"-->") {
            Some(end) => Ok(Special::Comment(end)),
            None if eof => Err(Error::new("unclosed comment")),
            None => Ok(Special::Pending),
        };
    }
    if bytes.starts_with(b"<![CDATA[") {
        return match find_marker(bytes, b"]]>") {
            Some(end) => Ok(Special::Cdata(end)),
            None if eof => Err(Error::new("unclosed cdata")),
            None => Ok(Special::Pending),
        };
    }
    if bytes.starts_with(b"<?") {
        return match find_marker(bytes, b"?>") {
            Some(end) => Ok(Special::Pi(end)),
            None if eof => Err(Error::new("unclosed processing instruction")),
            None => Ok(Special::Pending),
        };
    }
    if markup_prefix(bytes) {
        return if eof {
            Err(Error::new("xml element ended early"))
        } else {
            Ok(Special::Pending)
        };
    }
    if bytes.starts_with(b"<!") {
        Ok(Special::Decl)
    } else {
        Ok(Special::Tag)
    }
}

fn scan_tag(bytes: &[u8], eof: bool) -> Result<Option<(usize, TagKind)>, Error> {
    if bytes.first() != Some(&b'<') {
        return Err(Error::new("xml element does not start with '<'"));
    }
    if bytes.len() < 2 {
        return if eof {
            Err(Error::new("xml element ended early"))
        } else {
            Ok(None)
        };
    }
    let close = bytes[1] == b'/';
    let mut quote: Option<u8> = None;
    let mut i = 1;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(q) = quote {
            if byte == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'"' | b'\'' => {
                quote = Some(byte);
                i += 1;
            }
            b'/' => {
                if i + 1 >= bytes.len() {
                    return if eof {
                        Err(Error::new("xml element ended early"))
                    } else {
                        Ok(None)
                    };
                }
                if bytes[i + 1] == b'>' {
                    let kind = if close { TagKind::Close } else { TagKind::Empty };
                    return Ok(Some((i + 2, kind)));
                }
                i += 1;
            }
            b'>' => {
                let kind = if close { TagKind::Close } else { TagKind::Open };
                return Ok(Some((i + 1, kind)));
            }
            _ => i += 1,
        }
    }
    if eof {
        Err(Error::new("xml element ended early"))
    } else {
        Ok(None)
    }
}

fn json_ws(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    let n = text.bytes().take_while(|byte| is_json_ws(*byte)).count();
    if n == text.len() && !eof {
        return Ok(None);
    }
    Ok(Some(n))
}

fn json_start(text: &str, eof: bool) -> Result<Option<(usize, JsonStart)>, Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_json_ws(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("json ended early"))
        } else {
            Ok(None)
        };
    }
    match bytes[i] {
        b'{' => Ok(Some((i, JsonStart::Object))),
        b'[' => Ok(Some((i + 1, JsonStart::Array))),
        found => Err(Error::new(format!("unexpected json character {}", char::from(found)))),
    }
}

fn json_value_end(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_json_ws(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("json ended early"))
        } else {
            Ok(None)
        };
    }
    match bytes[i] {
        b'"' => scan_string(bytes, i, eof),
        b'{' | b'[' => scan_container(bytes, i, eof),
        b't' => scan_lit(bytes, i, b"true", eof),
        b'f' => scan_lit(bytes, i, b"false", eof),
        b'n' => scan_lit(bytes, i, b"null", eof),
        b'-' | b'0'..=b'9' => scan_number(bytes, i, eof),
        found => Err(Error::new(format!("unexpected json character {}", char::from(found)))),
    }
}

fn json_after_value(text: &str, eof: bool) -> Result<Option<(usize, bool)>, Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_json_ws(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("json separator ended early"))
        } else {
            Ok(None)
        };
    }
    match bytes[i] {
        b',' => Ok(Some((i + 1, false))),
        b']' => Ok(Some((i + 1, true))),
        found => Err(Error::new(format!("unexpected json character {}", char::from(found)))),
    }
}

fn scan_string(bytes: &[u8], start: usize, eof: bool) -> Result<Option<usize>, Error> {
    let mut i = start + 1;
    let mut escape = false;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if escape {
            escape = false;
            continue;
        }
        if byte == b'\\' {
            escape = true;
            continue;
        }
        if byte == b'"' {
            return Ok(Some(i));
        }
    }
    if eof {
        Err(Error::new("json string ended early"))
    } else {
        Ok(None)
    }
}

fn scan_container(bytes: &[u8], start: usize, eof: bool) -> Result<Option<usize>, Error> {
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escape = false;
    let mut i = start;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if byte == b'\\' {
                escape = true;
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(i));
                }
                if depth < 0 {
                    return Err(Error::new("json container is not closed"));
                }
            }
            _ => {}
        }
    }
    if eof {
        Err(Error::new("json ended early"))
    } else {
        Ok(None)
    }
}

fn scan_lit(bytes: &[u8], start: usize, lit: &[u8], eof: bool) -> Result<Option<usize>, Error> {
    let have = bytes.len().saturating_sub(start);
    if have < lit.len() {
        if bytes[start..].iter().zip(lit.iter()).all(|(got, want)| got == want) {
            return if eof {
                Err(Error::new("json literal is incomplete"))
            } else {
                Ok(None)
            };
        }
        return Err(Error::new("json literal is incomplete"));
    }
    if &bytes[start..start + lit.len()] != lit {
        return Err(Error::new("json literal is incomplete"));
    }
    if start + lit.len() == bytes.len() && !eof {
        return Ok(None);
    }
    Ok(Some(start + lit.len()))
}

fn scan_number(bytes: &[u8], start: usize, eof: bool) -> Result<Option<usize>, Error> {
    let mut i = start;
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    if i >= bytes.len() {
        return number_end(i, eof, false);
    }
    if bytes[i] == b'0' {
        i += 1;
    } else if bytes[i].is_ascii_digit() {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        return Err(Error::new("json number is incomplete"));
    }
    if i >= bytes.len() {
        return number_end(i, eof, true);
    }
    if bytes[i] == b'.' {
        i += 1;
        if i >= bytes.len() {
            return number_end(i, eof, false);
        }
        if !bytes[i].is_ascii_digit() {
            return Err(Error::new("json fraction is incomplete"));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i >= bytes.len() {
            return number_end(i, eof, true);
        }
    }
    if bytes[i] == b'e' || bytes[i] == b'E' {
        i += 1;
        if i >= bytes.len() {
            return number_end(i, eof, false);
        }
        if bytes[i] == b'+' || bytes[i] == b'-' {
            i += 1;
        }
        if i >= bytes.len() {
            return number_end(i, eof, false);
        }
        if !bytes[i].is_ascii_digit() {
            return Err(Error::new("json exponent is incomplete"));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i >= bytes.len() {
            return number_end(i, eof, true);
        }
    }
    Ok(Some(i))
}

fn number_end(i: usize, eof: bool, complete: bool) -> Result<Option<usize>, Error> {
    if !eof {
        Ok(None)
    } else if complete {
        Ok(Some(i))
    } else {
        Err(Error::new("json number is incomplete"))
    }
}

fn is_json_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn skip_xml_ws(text: &str, mut i: usize) -> usize {
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i].is_ascii() || !text.is_char_boundary(i) {
            break;
        }
        let Some(ch) = text[i..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        i += ch.len_utf8();
    }
    i
}

fn skip_ws_index(text: &str, mut i: usize, eof: bool) -> Result<Option<usize>, Error> {
    while i < text.len() {
        let Some(ch) = text[i..].chars().next() else {
            return Err(Error::new("xml split a character"));
        };
        if !ch.is_whitespace() {
            return Ok(Some(i));
        }
        i += ch.len_utf8();
    }
    if eof {
        Ok(Some(i))
    } else {
        Ok(None)
    }
}

fn read_xml_name(text: &str, start: usize, eof: bool) -> Result<Option<(String, usize)>, Error> {
    if start >= text.len() {
        return if eof {
            Err(Error::new("expected a name"))
        } else {
            Ok(None)
        };
    }
    if !text.is_char_boundary(start) {
        return Err(Error::new("xml split a character"));
    }
    let Some(first) = text[start..].chars().next() else {
        return if eof {
            Err(Error::new("expected a name"))
        } else {
            Ok(None)
        };
    };
    if !xml_name_start(first) {
        return Err(Error::new(format!("invalid name start '{first}'")));
    }
    let mut end = start + first.len_utf8();
    while end < text.len() {
        let Some(ch) = text[end..].chars().next() else {
            break;
        };
        if !xml_name_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    if end == text.len() && !eof {
        return Ok(None);
    }
    Ok(Some((text[start..end].to_string(), end)))
}

fn read_attr_value(text: &str, i: usize, eof: bool) -> Result<Option<(String, usize)>, Error> {
    if i >= text.len() {
        return if eof {
            Err(Error::new("attribute value is missing"))
        } else {
            Ok(None)
        };
    }
    let quote = text.as_bytes()[i];
    if quote != b'"' && quote != b'\'' {
        return Err(Error::new("attribute value must be quoted"));
    }
    let mut j = i + 1;
    let mut raw = String::new();
    while j < text.len() {
        let Some(ch) = text[j..].chars().next() else {
            return Err(Error::new("xml split a character"));
        };
        if ch == char::from(quote) {
            return Ok(Some((decode_xml_entities(&raw)?, j + 1)));
        }
        if ch == '<' {
            return Err(Error::new("'<' is not allowed in an attribute value"));
        }
        raw.push(ch);
        j += ch.len_utf8();
    }
    if eof {
        Err(Error::new("unclosed attribute value"))
    } else {
        Ok(None)
    }
}

fn decode_xml_entities(raw: &str) -> Result<String, Error> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::new();
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let Some(end) = rest.find(';') else {
            return Err(Error::new(format!("unterminated entity '&{rest}'")));
        };
        let body = &rest[..end];
        let ch = decode_xml_entity(body).ok_or_else(|| Error::new(format!("unknown entity '&{body};'")))?;
        out.push(ch);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn decode_xml_entity(body: &str) -> Option<char> {
    match body {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ if body.starts_with("#x") || body.starts_with("#X") => {
            u32::from_str_radix(body.get(2..)?, 16).ok().and_then(char::from_u32)
        }
        _ if body.starts_with('#') => body.get(1..)?.parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}

fn xml_name_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == ':' || (!ch.is_ascii() && ch.is_alphabetic())
}

fn xml_name_char(ch: char) -> bool {
    xml_name_start(ch) || ch.is_ascii_digit() || ch == '-' || ch == '.' || (!ch.is_ascii() && ch.is_numeric())
}

fn doctype_prefix(bytes: &[u8]) -> bool {
    const WORD: &[u8] = b"<!DOCTYPE";
    if bytes.len() >= WORD.len() {
        bytes[..WORD.len()].eq_ignore_ascii_case(WORD)
    } else {
        bytes.len() > 2 && WORD[..bytes.len()].eq_ignore_ascii_case(bytes)
    }
}

fn skip_doctype(bytes: &[u8], eof: bool) -> Result<Option<usize>, Error> {
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let mut brackets = 0_i32;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(q) = quote {
            if byte == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'"' | b'\'' => {
                quote = Some(byte);
                i += 1;
            }
            b'[' => {
                brackets += 1;
                i += 1;
            }
            b']' => {
                if brackets == 0 {
                    return Err(Error::new("DOCTYPE subset is not closed"));
                }
                brackets -= 1;
                i += 1;
            }
            b'>' if brackets == 0 => return Ok(Some(i + 1)),
            _ => i += 1,
        }
    }
    if eof {
        Err(Error::new("unclosed DOCTYPE"))
    } else {
        Ok(None)
    }
}

fn misc_prefix(bytes: &[u8]) -> bool {
    const MARKERS: [&[u8]; 2] = [b"<!--", b"<?"];
    MARKERS
        .iter()
        .any(|marker| bytes.len() < marker.len() && marker.starts_with(bytes))
}

fn markup_prefix(bytes: &[u8]) -> bool {
    const MARKERS: [&[u8]; 3] = [b"<!--", b"<![CDATA[", b"<?"];
    MARKERS
        .iter()
        .any(|marker| bytes.len() < marker.len() && marker.starts_with(bytes))
}

fn find_marker(bytes: &[u8], marker: &[u8]) -> Option<usize> {
    if marker.is_empty() || bytes.len() < marker.len() {
        return None;
    }
    bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .map(|at| at + marker.len())
}

fn load_zip_bytes(
    facade: &EntityFacade,
    location: &str,
    data: &[u8],
    options: &LoadOptions,
    depth: u32,
) -> Result<u64, Error> {
    let index = zip_index(data)?;
    let mut total = 0_u64;
    for meta in index {
        if meta.directory {
            continue;
        }
        if unsafe_zip_name(&meta.name) {
            return Err(Error::new(format!("zip entry {} is not a safe path", meta.name)));
        }
        let compressed = slice_at(
            data,
            meta.data_at,
            usize::try_from(meta.comp_size).unwrap_or(usize::MAX),
        )?;
        let plain = match meta.method {
            0 => compressed.to_vec(),
            8 => inflate(compressed)?,
            _ => {
                return Err(Error::new(format!(
                    "zip entry {} uses an unsupported method",
                    meta.name
                )));
            }
        };
        if meta.method == 8
            && meta.uncomp_size != 0
            && plain.len() != usize::try_from(meta.uncomp_size).unwrap_or(usize::MAX)
        {
            return Err(Error::new(format!(
                "zip entry {} inflated to the wrong size",
                meta.name
            )));
        }
        let started = std::time::Instant::now();
        let records = load_entry_bytes(facade, location, &meta.name, &plain, options, depth)?;
        if !matches!(extension(Path::new(&meta.name)).as_str(), "zip") {
            tracing::info!(
                "Loaded {records} records from {} in zip file {location} in {:.2}s",
                meta.name,
                started.elapsed().as_secs_f64()
            );
        }
        total = total.saturating_add(records);
    }
    Ok(total)
}

fn load_entry_bytes(
    facade: &EntityFacade,
    location: &str,
    name: &str,
    bytes: &[u8],
    options: &LoadOptions,
    depth: u32,
) -> Result<u64, Error> {
    let entry_location = format!("{location}!{name}");
    match extension(Path::new(name)).as_str() {
        "xml" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|err| Error::new(format!("{entry_location} is not utf-8")).with_source(err))?;
            load_xml_text(facade, &entry_location, text, options)
        }
        "csv" => load_csv_reader(facade, &entry_location, bytes, options),
        "json" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|err| Error::new(format!("{entry_location} is not utf-8")).with_source(err))?;
            load_json_text(facade, &entry_location, text, options)
        }
        "zip" => load_zip_bytes(facade, &entry_location, bytes, options, depth + 1),
        _ => Ok(0),
    }
}

struct ZipMeta {
    name: String,
    method: u16,
    comp_size: u32,
    uncomp_size: u32,
    data_at: usize,
    directory: bool,
}

fn zip_index(data: &[u8]) -> Result<Vec<ZipMeta>, Error> {
    let eocd = find_eocd(data)?;
    let cd_size = read_u32(data, eocd + 12)?;
    let cd_offset = read_u32(data, eocd + 16)?;
    if cd_size == u32::MAX || cd_offset == u32::MAX {
        return Err(Error::new("zip64 is not supported"));
    }
    let entries = usize::from(read_u16(data, eocd + 10)?);
    let mut at = usize::try_from(cd_offset).map_err(|_| Error::new("zip directory offset is too large"))?;
    let mut metas = Vec::with_capacity(entries);
    for _ in 0..entries {
        if read_u32(data, at)? != 0x0201_4b50 {
            return Err(Error::new("zip directory entry is not valid"));
        }
        let flags = read_u16(data, at + 8)?;
        if flags & 1 != 0 {
            return Err(Error::new("encrypted zip entries are not supported"));
        }
        let method = read_u16(data, at + 10)?;
        let comp_size = read_u32(data, at + 20)?;
        let uncomp_size = read_u32(data, at + 24)?;
        let name_len = usize::from(read_u16(data, at + 28)?);
        let extra_len = usize::from(read_u16(data, at + 30)?);
        let comment_len = usize::from(read_u16(data, at + 32)?);
        let local_off = read_u32(data, at + 42)?;
        if comp_size == u32::MAX || uncomp_size == u32::MAX || local_off == u32::MAX {
            return Err(Error::new("zip64 is not supported"));
        }
        let name_bytes = slice_at(data, at + 46, name_len)?;
        let name = String::from_utf8(name_bytes.to_vec())
            .map_err(|err| Error::new("zip entry name is not utf-8").with_source(err))?;
        let local = usize::try_from(local_off).map_err(|_| Error::new("zip local offset is too large"))?;
        if read_u32(data, local)? != 0x0403_4b50 {
            return Err(Error::new(format!("zip local header for {name} is not valid")));
        }
        let local_name = usize::from(read_u16(data, local + 26)?);
        let local_extra = usize::from(read_u16(data, local + 28)?);
        let data_at = local
            .checked_add(30)
            .and_then(|value| value.checked_add(local_name))
            .and_then(|value| value.checked_add(local_extra))
            .ok_or_else(|| Error::new("zip local header offset overflow"))?;
        metas.push(ZipMeta {
            directory: name.ends_with('/'),
            name,
            method,
            comp_size,
            uncomp_size,
            data_at,
        });
        at = at
            .checked_add(46)
            .and_then(|value| value.checked_add(name_len))
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| Error::new("zip directory offset overflow"))?;
    }
    Ok(metas)
}

fn find_eocd(data: &[u8]) -> Result<usize, Error> {
    if data.len() < 22 {
        return Err(Error::new("zip is shorter than the end record"));
    }
    let earliest = data.len().saturating_sub(22 + 65_535);
    let mut index = data.len() - 22;
    loop {
        if data.get(index..index + 4) == Some(&[0x50, 0x4b, 0x05, 0x06][..]) {
            let comment = usize::from(u16::from_le_bytes([data[index + 20], data[index + 21]]));
            if index.checked_add(22).and_then(|value| value.checked_add(comment)) == Some(data.len()) {
                return Ok(index);
            }
        }
        if index == earliest {
            break;
        }
        index -= 1;
    }
    Err(Error::new("zip end record was not found"))
}

fn unsafe_zip_name(name: &str) -> bool {
    name.starts_with('/') || name.starts_with('\\') || name.split(['/', '\\']).any(|part| part == "..")
}

fn read_u16(data: &[u8], at: usize) -> Result<u16, Error> {
    let bytes = slice_at(data, at, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], at: usize) -> Result<u32, Error> {
    let bytes = slice_at(data, at, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn slice_at(data: &[u8], start: usize, len: usize) -> Result<&[u8], Error> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| Error::new("zip offset overflow"))?;
    data.get(start..end)
        .ok_or_else(|| Error::new("zip entry overruns the file"))
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut bits = Bits { data, index: 0, bit: 0 };
    let mut out = Vec::new();
    loop {
        let final_block = bits.bit()? == 1;
        match bits.bits(2)? {
            0 => inflate_stored(&mut bits, &mut out)?,
            1 => {
                let lit = Huffman::from_lengths(&fixed_lit_lengths())?;
                let dist = Huffman::from_lengths(&fixed_dist_lengths())?;
                inflate_codes(&mut bits, &mut out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_trees(&mut bits)?;
                inflate_codes(&mut bits, &mut out, &lit, &dist)?;
            }
            _ => return Err(Error::new("deflate block type is invalid")),
        }
        if out.len() > MAX_INFLATE {
            return Err(Error::new("deflate output is too large"));
        }
        if final_block {
            break;
        }
    }
    Ok(out)
}

fn inflate_stored(bits: &mut Bits<'_>, out: &mut Vec<u8>) -> Result<(), Error> {
    bits.align();
    let len = bits.bits(16)?;
    let nlen = bits.bits(16)?;
    if len ^ 0xffff != nlen {
        return Err(Error::new("deflate stored length check failed"));
    }
    let bytes = bits.take_bytes(usize::try_from(len).unwrap_or(usize::MAX))?;
    if out.len().saturating_add(bytes.len()) > MAX_INFLATE {
        return Err(Error::new("deflate output is too large"));
    }
    out.extend_from_slice(&bytes);
    Ok(())
}

fn inflate_codes(bits: &mut Bits<'_>, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman) -> Result<(), Error> {
    loop {
        let symbol = lit.decode(bits)?;
        if symbol < 256 {
            if out.len() >= MAX_INFLATE {
                return Err(Error::new("deflate output is too large"));
            }
            out.push(u8::try_from(symbol).unwrap_or(0));
        } else if symbol == 256 {
            return Ok(());
        } else {
            let length = length_of(bits, symbol)?;
            let coded = dist.decode(bits)?;
            let distance = distance_of(bits, coded)?;
            copy_back(out, distance, length)?;
        }
    }
}

fn dynamic_trees(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), Error> {
    let lit_count = usize::try_from(bits.bits(5)? + 257).unwrap_or(0);
    let dist_count = usize::try_from(bits.bits(5)? + 1).unwrap_or(0);
    let code_count = usize::try_from(bits.bits(4)? + 4).unwrap_or(0);
    if lit_count > 286 || dist_count > 32 || code_count > 19 {
        return Err(Error::new("deflate dynamic header is invalid"));
    }
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut code_lengths = vec![0_u16; 19];
    for slot in ORDER.iter().take(code_count) {
        code_lengths[*slot] = u16::try_from(bits.bits(3)?).unwrap_or(0);
    }
    let coder = Huffman::from_lengths(&code_lengths)?;
    let all = read_code_lengths(bits, &coder, lit_count + dist_count)?;
    let lit = Huffman::from_lengths(&all[..lit_count])?;
    let dist = Huffman::from_lengths(&all[lit_count..])?;
    Ok((lit, dist))
}

fn read_code_lengths(bits: &mut Bits<'_>, coder: &Huffman, count: usize) -> Result<Vec<u16>, Error> {
    let mut lengths = Vec::with_capacity(count);
    while lengths.len() < count {
        match coder.decode(bits)? {
            symbol @ 0..=15 => lengths.push(symbol),
            16 => {
                let previous = *lengths
                    .last()
                    .ok_or_else(|| Error::new("deflate repeat has no previous length"))?;
                let repeat = 3 + usize::try_from(bits.bits(2)?).unwrap_or(0);
                push_repeat(&mut lengths, count, previous, repeat)?;
            }
            17 => {
                let repeat = 3 + usize::try_from(bits.bits(3)?).unwrap_or(0);
                push_repeat(&mut lengths, count, 0, repeat)?;
            }
            18 => {
                let repeat = 11 + usize::try_from(bits.bits(7)?).unwrap_or(0);
                push_repeat(&mut lengths, count, 0, repeat)?;
            }
            _ => return Err(Error::new("deflate code length symbol is invalid")),
        }
    }
    Ok(lengths)
}

fn push_repeat(lengths: &mut Vec<u16>, count: usize, value: u16, repeat: usize) -> Result<(), Error> {
    if lengths.len().saturating_add(repeat) > count {
        return Err(Error::new("deflate code length repeat overruns the alphabet"));
    }
    for _ in 0..repeat {
        lengths.push(value);
    }
    Ok(())
}

fn length_of(bits: &mut Bits<'_>, symbol: u16) -> Result<usize, Error> {
    const BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227,
        258,
    ];
    const EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    if !(257..=285).contains(&symbol) {
        return Err(Error::new("deflate length symbol is invalid"));
    }
    let index = usize::from(symbol - 257);
    let extra = if EXTRA[index] == 0 { 0 } else { bits.bits(EXTRA[index])? };
    Ok(usize::from(BASE[index]).saturating_add(usize::try_from(extra).unwrap_or(0)))
}

fn distance_of(bits: &mut Bits<'_>, symbol: u16) -> Result<usize, Error> {
    const BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
        6145, 8193, 12289, 16385, 24577,
    ];
    const EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
    ];
    let index = usize::from(symbol);
    if index >= BASE.len() {
        return Err(Error::new("deflate distance symbol is invalid"));
    }
    let extra = if EXTRA[index] == 0 { 0 } else { bits.bits(EXTRA[index])? };
    Ok(usize::from(BASE[index]).saturating_add(usize::try_from(extra).unwrap_or(0)))
}

fn copy_back(out: &mut Vec<u8>, distance: usize, length: usize) -> Result<(), Error> {
    if distance == 0 || distance > out.len() || length > MAX_INFLATE {
        return Err(Error::new("deflate distance is out of range"));
    }
    for _ in 0..length {
        if out.len() >= MAX_INFLATE {
            return Err(Error::new("deflate output is too large"));
        }
        let byte = out[out.len() - distance];
        out.push(byte);
    }
    Ok(())
}

fn fixed_lit_lengths() -> Vec<u16> {
    let mut lengths = vec![8_u16; 288];
    for length in &mut lengths[144..256] {
        *length = 9;
    }
    for length in &mut lengths[256..280] {
        *length = 7;
    }
    lengths
}

fn fixed_dist_lengths() -> Vec<u16> {
    vec![5_u16; 32]
}

struct Bits<'a> {
    data: &'a [u8],
    index: usize,
    bit: u8,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, Error> {
        let Some(byte) = self.data.get(self.index) else {
            return Err(Error::new("deflate input ended early"));
        };
        let value = u32::from((byte >> self.bit) & 1);
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.index += 1;
        }
        Ok(value)
    }

    fn bits(&mut self, count: u8) -> Result<u32, Error> {
        let mut value = 0_u32;
        for shift in 0..count {
            value |= self.bit()? << shift;
        }
        Ok(value)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.index += 1;
        }
    }

    fn take_bytes(&mut self, len: usize) -> Result<Vec<u8>, Error> {
        self.align();
        let end = self
            .index
            .checked_add(len)
            .ok_or_else(|| Error::new("deflate length overflow"))?;
        if end > self.data.len() {
            return Err(Error::new("deflate stored block overruns input"));
        }
        let out = self.data[self.index..end].to_vec();
        self.index = end;
        Ok(out)
    }
}

struct Huffman {
    entries: Vec<(u8, u16, u16)>,
}

impl Huffman {
    fn from_lengths(lengths: &[u16]) -> Result<Self, Error> {
        let mut counts = [0_u16; 16];
        for &length in lengths {
            if length > 15 {
                return Err(Error::new("huffman code is longer than 15 bits"));
            }
            if length > 0 {
                counts[usize::from(length)] = counts[usize::from(length)]
                    .checked_add(1)
                    .ok_or_else(|| Error::new("huffman length count overflow"))?;
            }
        }
        let mut next = [0_u16; 16];
        let mut code = 0_u16;
        for bits in 1..=15 {
            code = code
                .checked_add(counts[bits - 1])
                .ok_or_else(|| Error::new("huffman code overflow"))?
                .checked_shl(1)
                .ok_or_else(|| Error::new("huffman code overflow"))?;
            next[bits] = code;
        }
        let mut entries = Vec::new();
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let assigned = next[usize::from(length)];
            next[usize::from(length)] = assigned.saturating_add(1);
            let symbol = u16::try_from(symbol).map_err(|_| Error::new("huffman symbol does not fit"))?;
            let bits = u8::try_from(length).unwrap_or(0);
            entries.push((bits, assigned, symbol));
        }
        Ok(Self { entries })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        let mut code = 0_u16;
        for length in 1..=15 {
            let bit = u16::try_from(bits.bit()?).map_err(|_| Error::new("deflate bit does not fit"))?;
            code = code.checked_shl(1).ok_or_else(|| Error::new("deflate code overflow"))? | bit;
            if let Some((_, _, symbol)) = self
                .entries
                .iter()
                .find(|(entry_len, entry_code, _)| *entry_len == length && *entry_code == code)
            {
                return Ok(*symbol);
            }
        }
        Err(Error::new("deflate huffman code is not in the alphabet"))
    }
}

enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

struct JsonParser<'a> {
    text: &'a str,
    index: usize,
}

fn parse_json(text: &str) -> Result<Json, Error> {
    let mut parser = JsonParser { text, index: 0 };
    parser.skip();
    let value = parser.value()?;
    parser.skip();
    if parser.index != parser.text.len() {
        return Err(Error::new("json has trailing data"));
    }
    Ok(value)
}

impl JsonParser<'_> {
    fn skip(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.index..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn value(&mut self) -> Result<Json, Error> {
        self.skip();
        match self.peek() {
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('"') => Ok(Json::String(self.string()?)),
            Some('[') => self.array(),
            Some('{') => self.object(),
            Some('-') | Some('0'..='9') => Ok(Json::Number(self.number()?)),
            Some(ch) => Err(Error::new(format!("unexpected json character {ch}"))),
            None => Err(Error::new("json ended early")),
        }
    }

    fn literal(&mut self, expected: &str, value: Json) -> Result<Json, Error> {
        for ch in expected.chars() {
            if self.bump() != Some(ch) {
                return Err(Error::new(format!("json literal {expected} is incomplete")));
            }
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, Error> {
        if self.bump() != Some('"') {
            return Err(Error::new("json string does not start with a quote"));
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                Some('"') => return Ok(out),
                Some('\\') => out.push(self.escape()?),
                Some(ch) => out.push(ch),
                None => return Err(Error::new("json string ended early")),
            }
        }
    }

    fn escape(&mut self) -> Result<char, Error> {
        match self.bump() {
            Some('"') => Ok('"'),
            Some('\\') => Ok('\\'),
            Some('/') => Ok('/'),
            Some('b') => Ok('\u{0008}'),
            Some('f') => Ok('\u{000c}'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('u') => self.hex_char(),
            Some(ch) => Err(Error::new(format!("json escape {ch} is not valid"))),
            None => Err(Error::new("json escape ended early")),
        }
    }

    fn hex_char(&mut self) -> Result<char, Error> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let Some(ch) = self.bump() else {
                return Err(Error::new("json hex escape ended early"));
            };
            let digit = ch
                .to_digit(16)
                .ok_or_else(|| Error::new("json hex escape is not hex"))?;
            value = (value << 4) | digit;
        }
        char::from_u32(value).ok_or_else(|| Error::new("json hex escape is not a character"))
    }

    fn number(&mut self) -> Result<String, Error> {
        let start = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        if self.peek() == Some('0') {
            self.bump();
        } else if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.bump();
            }
        } else {
            return Err(Error::new("json number is incomplete"));
        }
        if self.peek() == Some('.') {
            self.bump();
            if !self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                return Err(Error::new("json fraction is incomplete"));
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.bump();
            }
        }
        if self.peek().is_some_and(|ch| ch == 'e' || ch == 'E') {
            self.bump();
            if self.peek().is_some_and(|ch| ch == '+' || ch == '-') {
                self.bump();
            }
            if !self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                return Err(Error::new("json exponent is incomplete"));
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.bump();
            }
        }
        Ok(self.text[start..self.index].to_string())
    }

    fn array(&mut self) -> Result<Json, Error> {
        self.bump();
        let mut values = Vec::new();
        self.skip();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.skip();
            match self.bump() {
                Some(',') => self.skip(),
                Some(']') => return Ok(Json::Array(values)),
                _ => return Err(Error::new("json array is not closed")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, Error> {
        self.bump();
        let mut pairs = Vec::new();
        self.skip();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Json::Object(pairs));
        }
        loop {
            self.skip();
            if self.peek() != Some('"') {
                return Err(Error::new("json object key is not a string"));
            }
            let key = self.string()?;
            self.skip();
            if self.bump() != Some(':') {
                return Err(Error::new("json object is missing a colon"));
            }
            pairs.push((key, self.value()?));
            self.skip();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Json::Object(pairs)),
                _ => return Err(Error::new("json object is not closed")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use moqui_db::dialect::Dialects;
    use moqui_db::model::EntityModel;
    use moqui_db::sql::insert_or_update;
    use moqui_lib::error::Error;
    use moqui_lib::node::MNode;
    use moqui_lib::object::Value;

    use super::{
        conf_data_locations, data_file_services, eca_rules, feed_service_names, load_entity_data, match_rules,
        register_feed_insert, LoadOptions, TOUCHES,
    };
    use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use crate::entity::{EntityFacade, EntityGroup};
    use crate::transaction::{PoolConfig, TransactionManager};

    struct NoCancel;

    impl DbCancel for NoCancel {
        fn cancel(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    struct EmptyConn;

    impl DbConnection for EmptyConn {
        fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, Error> {
            Ok(0)
        }
        fn query(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
            Ok(Vec::new())
        }
        fn query_once(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
            Ok(Vec::new())
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
            Ok(())
        }
        fn commit(&mut self) -> Result<(), Error> {
            Ok(())
        }
        fn rollback(&mut self) -> Result<(), Error> {
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

    struct EmptyDriver;

    impl DbDriver for EmptyDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
            Ok(Box::new(EmptyConn))
        }
    }

    struct Script {
        inserts: Mutex<Vec<Vec<Value>>>,
        missing_feed: bool,
        /// Entity inserts fail while set, as a duplicate key would.
        fail_inserts: AtomicBool,
    }

    struct ScriptConn {
        script: Arc<Script>,
    }

    impl DbConnection for ScriptConn {
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
            if sql.contains("DataFeedPending") {
                self.script.inserts.lock().expect("inserts").push(params.to_vec());
            } else if sql.starts_with("INSERT") && self.script.fail_inserts.load(Ordering::SeqCst) {
                return Err(Error::new("duplicate key value violates unique constraint"));
            }
            Ok(1)
        }
        fn query(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, Error> {
            if self.script.missing_feed && sql.contains("DataFeed") {
                let source = std::io::Error::other("relation \"DataFeed\" does not exist");
                return Err(Error::new("postgres query").with_source(source));
            }
            if sql.contains("xmin") {
                return Ok(vec![db_row(&[("fooId", "F1")])]);
            }
            if sql.contains("DataDocumentField") {
                let source = std::io::Error::other("relation \"DataDocumentField\" does not exist");
                return Err(Error::new("postgres query").with_source(source));
            }
            if sql.contains("DataFeedDocument") {
                return Ok(vec![db_row(&[("dataFeedId", "FEED1"), ("dataDocumentId", "DOC1")])]);
            }
            if sql.contains("DataDocument") {
                return Ok(vec![db_row(&[
                    ("dataDocumentId", "DOC1"),
                    ("primaryEntityName", "moqui.test.Foo"),
                ])]);
            }
            if sql.contains("DataFeed") {
                return Ok(vec![db_row(&[
                    ("dataFeedId", "FEED1"),
                    ("feedReceiveServiceName", "index#DataDocuments"),
                    ("feedDeleteServiceName", "delete#DataDocument"),
                ])]);
            }
            Ok(Vec::new())
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
            Ok(())
        }
        fn commit(&mut self) -> Result<(), Error> {
            Ok(())
        }
        fn rollback(&mut self) -> Result<(), Error> {
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

    struct ScriptDriver {
        script: Arc<Script>,
    }

    impl DbDriver for ScriptDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
            Ok(Box::new(ScriptConn {
                script: Arc::clone(&self.script),
            }))
        }
    }

    fn db_row(pairs: &[(&str, &str)]) -> DbRow {
        DbRow {
            columns: pairs
                .iter()
                .map(|(name, value)| ((*name).to_string(), Value::string(*value)))
                .collect(),
        }
    }

    fn open_with(driver: Arc<dyn DbDriver>) -> EntityFacade {
        let transactions = TransactionManager::new(
            driver,
            PoolConfig {
                min: 0,
                max: 2,
                reserve: 1,
                acquire_timeout: Duration::from_secs(5),
                ..Default::default()
            },
        )
        .unwrap();
        EntityFacade::open(
            &framework_dir(),
            "transactional",
            vec![EntityGroup::new("transactional", "postgres", transactions)],
        )
        .unwrap()
    }

    fn open_empty() -> EntityFacade {
        open_with(Arc::new(EmptyDriver))
    }

    fn framework_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
    }

    struct DirGuard(PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> (PathBuf, DirGuard) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moqui-load-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let guard = DirGuard(path.clone());
        (path, guard)
    }

    /// A stored entry the way Java `ZipOutputStream` writes it: flag bit 3, zero local sizes, and a
    /// data descriptor after the data. Only the central directory carries the sizes.
    fn descriptor_zip(name: &str, data: &[u8]) -> Vec<u8> {
        let plain = stored_zip(name, data);
        let local_len = 30 + name.len() + data.len();
        let mut out = plain[..local_len].to_vec();
        out[6] |= 0x08;
        out[18..26].fill(0);
        let shift = 16_u32;
        out.extend_from_slice(&0x0807_4b50_u32.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
        out.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
        let mut rest = plain[local_len..].to_vec();
        // The end record's directory offset moves past the descriptor.
        let end_at = rest.len() - 22;
        let offset_at = end_at + 16;
        let moved = u32::from_le_bytes(rest[offset_at..offset_at + 4].try_into().unwrap()) + shift;
        rest[offset_at..offset_at + 4].copy_from_slice(&moved.to_le_bytes());
        out.extend(rest);
        out
    }

    fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
        let name_bytes = name.as_bytes();
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        local.extend_from_slice(&20_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u32.to_le_bytes());
        local.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
        local.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
        local.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(data);
        let mut central = Vec::new();
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
        central.extend_from_slice(&(u32::try_from(data.len()).unwrap()).to_le_bytes());
        central.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(name_bytes);
        let mut end = Vec::new();
        end.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        end.extend_from_slice(&0_u16.to_le_bytes());
        end.extend_from_slice(&0_u16.to_le_bytes());
        end.extend_from_slice(&1_u16.to_le_bytes());
        end.extend_from_slice(&1_u16.to_le_bytes());
        end.extend_from_slice(&(u32::try_from(central.len()).unwrap()).to_le_bytes());
        end.extend_from_slice(&(u32::try_from(local.len()).unwrap()).to_le_bytes());
        end.extend_from_slice(&0_u16.to_le_bytes());
        let mut out = local;
        out.extend(central);
        out.extend(end);
        out
    }

    #[test]
    fn eca_rules_are_stored_and_disable_matches_nothing() {
        let facade = open_empty();
        let rules = eca_rules(&facade, "moqui.screen.form.DbForm");
        let form = rules.iter().find(|rule| rule.id == "DbFormCache").expect("DbFormCache");
        assert!(form.actions.contains("ec.cache"), "{}", form.actions);
        assert!(form.on_create && form.on_update && form.on_delete);
        let member = eca_rules(&facade, "moqui.security.UserGroupMember");
        let privileged = member
            .iter()
            .find(|rule| rule.id == "UserGroupMemberPrivileged")
            .expect("privileged");
        assert!(privileged.run_before);
        assert!(privileged.actions.contains("assertPrivilegedGroupMembership"));
        facade.begin("moqui.screen.form.DbForm", None).unwrap();
        let ids = match_rules(&facade, "moqui.screen.form.DbForm", "create");
        assert!(ids.iter().any(|id| id == "DbFormCache"));
        facade.set_disable_entity_eca(true);
        assert!(match_rules(&facade, "moqui.screen.form.DbForm", "update").is_empty());
        facade.rollback("moqui.screen.form.DbForm").unwrap();
    }

    #[test]
    fn conf_locations_name_the_seed_files() {
        let locations = conf_data_locations(&open_empty()).unwrap();
        assert!(locations.iter().any(|location| location.contains("MoquiSetupData.xml")));
    }

    #[test]
    fn loader_filters_types_holds_services_and_reads_zip() {
        let facade = open_empty();
        // A data-file service element runs in the entered context.
        let context = crate::context::ExecutionContextFactory::test_context();
        context.factory().set_entity(facade.clone());
        let _entered = context.enter();
        let _authz = context.disable_authz();
        let (dir, _guard) = temp_dir();
        let xml = dir.join("rows.xml");
        std::fs::write(
            &xml,
            r#"<entity-facade-xml type="seed">
                <moqui.test.TestEntity testId="T1" testMedium="xml"/>
                <create-moqui.test.TestEntity testId="SVC" testMedium="held"/>
            </entity-facade-xml>"#,
        )
        .unwrap();
        let loaded = load_entity_data(&facade, xml.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(loaded.records, 1);
        let held = data_file_services(&facade);
        assert!(held
            .iter()
            .any(|service| service.name == "create#moqui.test.TestEntity"));
        assert_eq!(held[0].parameters.get("testId").map(String::as_str), Some("SVC"));

        let demo = LoadOptions {
            data_types: vec!["demo".to_string()],
            ..LoadOptions::default()
        };
        let skipped = load_entity_data(&facade, xml.to_str().unwrap(), &demo).unwrap();
        assert_eq!(skipped.records, 0);

        let csv_path = dir.join("rows.csv");
        std::fs::write(&csv_path, "moqui.test.TestEntity,seed\ntestId,testMedium\nC1,csv\n").unwrap();
        let csv_loaded = load_entity_data(&facade, csv_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(csv_loaded.records, 1);

        let json_path = dir.join("rows.json");
        std::fs::write(
            &json_path,
            r#"{"_entity":"moqui.test.TestEntity","_dataType":"seed","testId":"J1","testMedium":"json"}"#,
        )
        .unwrap();
        let json_loaded = load_entity_data(&facade, json_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(json_loaded.records, 1);

        let nested = dir.join("nested.json");
        std::fs::write(
            &nested,
            r#"{"_entity":"moqui.security.UserGroup","userGroupId":"G1","description":"g","permissions":{"userPermissionId":"P1","fromDate":"0"}}"#,
        )
        .unwrap();
        // The empty driver cannot see the parent row, so the child foreign key uses a dummy.
        let nested_options = LoadOptions {
            dummy_fks: true,
            ..LoadOptions::default()
        };
        let nested_loaded = load_entity_data(&facade, nested.to_str().unwrap(), &nested_options).unwrap();
        assert_eq!(nested_loaded.records, 2);
        assert!(!facade.flags().dummy_fks);

        let zip_path = dir.join("rows.zip");
        let csv = b"moqui.test.TestEntity,seed\ntestId,testMedium\nZ1,zip\n";
        std::fs::write(&zip_path, stored_zip("rows.csv", csv)).unwrap();
        let zip_loaded = load_entity_data(&facade, zip_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(zip_loaded.records, 1);

        let described = dir.join("described.zip");
        std::fs::write(&described, descriptor_zip("rows.csv", csv)).unwrap();
        let described_loaded = load_entity_data(&facade, described.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(described_loaded.records, 1);

        let deflated = dir.join("deflated.zip");
        let payload: &[u8] = &[
            203, 205, 47, 44, 205, 212, 43, 73, 45, 46, 209, 11, 1, 18, 174, 121, 37, 153, 37, 149, 58, 197, 169, 169,
            41, 92, 32, 65, 207, 20, 29, 16, 229, 155, 154, 146, 89, 154, 203, 21, 101, 168, 83, 149, 89, 192, 5, 0,
        ];
        std::fs::write(&deflated, deflate_zip("rows.csv", payload, csv.len())).unwrap();
        let deflated_loaded = load_entity_data(&facade, deflated.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(deflated_loaded.records, 1);

        let tools = LoadOptions {
            components: vec!["tools".to_string()],
            ..LoadOptions::default()
        };
        let blocked = load_entity_data(&facade, "component://other/data/x.xml", &tools).unwrap();
        assert_eq!(blocked.records, 0);

        facade.set_dummy_fks(false);
        facade.set_disable_fk_create(false);
        let flags = LoadOptions {
            dummy_fks: true,
            disable_fk_create: true,
            use_try_insert: true,
            only_create: true,
            ..LoadOptions::default()
        };
        let again = load_entity_data(&facade, xml.to_str().unwrap(), &flags).unwrap();
        assert_eq!(again.records, 1);
        assert!(!facade.flags().dummy_fks);
        assert!(!facade.flags().disable_fk_create);
    }

    #[test]
    fn loader_reads_seed_data_inside_entity_definitions() {
        let facade = open_empty();
        let (dir, _guard) = temp_dir();
        let xml = dir.join("entities.xml");
        std::fs::write(
            &xml,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<entities xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <entity entity-name="One" package="moqui.test">
    <field name="testId" type="id" is-pk="true"/>
    <field name="testMedium" type="text-medium"/>
    <relationship type="one" related="moqui.test.Foo"/>
    <seed-data>
      <moqui.test.TestEntity testId="S1" testMedium="one"/>
    </seed-data>
  </entity>
  <entity entity-name="Two" package="moqui.test">
    <field name="testId" type="id" is-pk="true"/>
    <seed-data>
      <moqui.test.TestEntity testId="S2" testMedium="two"/>
    </seed-data>
  </entity>
</entities>"#,
        )
        .unwrap();
        let loaded = load_entity_data(&facade, xml.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(loaded.records, 2);
        let demo = LoadOptions {
            data_types: vec!["demo".to_string()],
            ..LoadOptions::default()
        };
        let skipped = load_entity_data(&facade, xml.to_str().unwrap(), &demo).unwrap();
        assert_eq!(skipped.records, 0);

        let bare = dir.join("bare.xml");
        std::fs::write(
            &bare,
            r#"<entities><entity entity-name="Only" package="moqui.test"><field name="testId" type="id" is-pk="true"/></entity></entities>"#,
        )
        .unwrap();
        let none = load_entity_data(&facade, bare.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(none.records, 0);
    }

    #[test]
    fn nested_many_relationship_inherits_parent_primary_key() {
        let facade = open_empty();
        let xml = r#"<moqui.service.job.ServiceJob jobName="clean_test" serviceName="example" cronExpression="0 0 2 * * ?">
  <parameters parameterName="daysToKeep" parameterValue="1"/>
  <parameters jobName="other" parameterName="kept" parameterValue="2"/>
</moqui.service.job.ServiceJob>"#;
        let node = MNode::parse_text("job", xml).unwrap();
        let record = super::parse_record(&facade, "moqui.service.job.ServiceJob", &node).unwrap();
        assert_eq!(record.children.len(), 2);
        assert_eq!(record.children[0].entity, "moqui.service.job.ServiceJobParameter");
        assert_eq!(
            record.children[0].fields.get("jobName").and_then(Value::as_str),
            Some("clean_test")
        );
        assert_eq!(
            record.children[0].fields.get("parameterName").and_then(Value::as_str),
            Some("daysToKeep")
        );
        assert_eq!(
            record.children[1].fields.get("jobName").and_then(Value::as_str),
            Some("other")
        );
    }

    #[test]
    fn loader_streams_attributes_comments_and_json_arrays() {
        let element = r#"<moqui.test.TestEntity testId="A1" testMedium="a>b"/>"#;
        let end = super::xml_element_end(element, true).unwrap().unwrap();
        assert_eq!(end, element.len());
        let node = MNode::parse_text("element", element).unwrap();
        assert_eq!(node.attribute("testMedium").as_deref(), Some("a>b"));

        let quoted = r#"{"testMedium":"say \"hi\""}"#;
        let value_end = super::json_value_end(quoted, true).unwrap().unwrap();
        assert_eq!(value_end, quoted.len());
        let super::Json::Object(pairs) = super::parse_json(quoted).unwrap() else {
            panic!("quoted json is not an object");
        };
        let medium = pairs.iter().find(|(key, _)| key == "testMedium").expect("testMedium");
        assert!(matches!(&medium.1, super::Json::String(text) if text == "say \"hi\""));

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- see <http://creativecommons.org/publicdomain/zero/1.0/> -->
<!DOCTYPE entity-facade-xml [
  <!ELEMENT entity-facade-xml ANY>
]>
<entity-facade-xml type="seed">
  <moqui.test.TestEntity testId="A1" testMedium="a>b"/>
  <moqui.test.TestEntity testId="A2" testMedium="plain"/>
</entity-facade-xml>
"#;
        let facade = open_empty();
        let xml_reader = std::io::BufReader::with_capacity(1, std::io::Cursor::new(xml.as_bytes()));
        let xml_loaded = super::load_xml_reader(&facade, "chunk.xml", xml_reader, &LoadOptions::default()).unwrap();
        assert_eq!(xml_loaded, 2);

        let json = r#"[{"_dataType":"seed"},{"_entity":"moqui.test.TestEntity","testId":"B1","testMedium":"one"},{"_entity":"moqui.test.TestEntity","testId":"B2","testMedium":"say \"hi\""}]"#;
        let json_reader = std::io::BufReader::with_capacity(1, std::io::Cursor::new(json.as_bytes()));
        let json_loaded = super::load_json_reader(&facade, "chunk.json", json_reader, &LoadOptions::default()).unwrap();
        assert_eq!(json_loaded, 2);

        let (dir, _guard) = temp_dir();
        let json_path = dir.join("rows.json");
        std::fs::write(&json_path, json).unwrap();
        let from_file = load_entity_data(&facade, json_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(from_file.records, 2);
    }

    #[test]
    fn loader_scans_one_large_element_in_linear_time() {
        let mut medium = String::new();
        while medium.len() < 8 * 1024 * 1024 {
            medium.push_str("0123456789abcdef");
        }
        let xml = format!(
            "<entity-facade-xml type=\"seed\"><moqui.test.TestEntity testId=\"BIG\" testMedium=\"{medium}\"/></entity-facade-xml>"
        );
        let facade = open_empty();
        let started = std::time::Instant::now();
        let reader = std::io::BufReader::with_capacity(4096, std::io::Cursor::new(xml.as_bytes()));
        let loaded = super::load_xml_reader(&facade, "big.xml", reader, &LoadOptions::default()).unwrap();
        assert_eq!(loaded, 1);
        assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
    }

    fn deflate_zip(name: &str, compressed: &[u8], plain_len: usize) -> Vec<u8> {
        let name_bytes = name.as_bytes();
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        local.extend_from_slice(&20_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&8_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u32.to_le_bytes());
        local.extend_from_slice(&(u32::try_from(compressed.len()).unwrap()).to_le_bytes());
        local.extend_from_slice(&(u32::try_from(plain_len).unwrap()).to_le_bytes());
        local.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        local.extend_from_slice(compressed);
        let mut central = Vec::new();
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&8_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&(u32::try_from(compressed.len()).unwrap()).to_le_bytes());
        central.extend_from_slice(&(u32::try_from(plain_len).unwrap()).to_le_bytes());
        central.extend_from_slice(&(u16::try_from(name_bytes.len()).unwrap()).to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(name_bytes);
        let mut end = Vec::new();
        end.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        end.extend_from_slice(&0_u16.to_le_bytes());
        end.extend_from_slice(&0_u16.to_le_bytes());
        end.extend_from_slice(&1_u16.to_le_bytes());
        end.extend_from_slice(&1_u16.to_le_bytes());
        end.extend_from_slice(&(u32::try_from(central.len()).unwrap()).to_le_bytes());
        end.extend_from_slice(&(u32::try_from(local.len()).unwrap()).to_le_bytes());
        end.extend_from_slice(&0_u16.to_le_bytes());
        let mut out = local;
        out.extend(central);
        out.extend(end);
        out
    }

    #[test]
    fn inflate_covers_stored_fixed_and_dynamic_blocks() {
        let stored = [1_u8, 5, 0, 0xfa, 0xff, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(super::inflate(&stored).unwrap(), b"hello");
        let fixed = [203_u8, 72, 205, 201, 201, 7, 0];
        assert_eq!(super::inflate(&fixed).unwrap(), b"hello");
        let dynamic = [5_u8, 192, 129, 0, 0, 0, 0, 0, 144, 54, 255, 83, 8];
        assert_eq!(super::inflate(&dynamic).unwrap(), b"A");
    }

    #[test]
    fn mysql_try_insert_uses_duplicate_key_update() {
        let framework = framework_dir();
        let conf = MNode::parse_file(framework.join("conf/MoquiDefaultConf.xml")).unwrap();
        let dialects = Dialects::load(&conf).unwrap();
        let model = EntityModel::load_directory(&conf, &framework.join("entity")).unwrap();
        let entity = model.entity("moqui.test.Foo").unwrap();
        let mysql = dialects.get("mysql").unwrap();
        let sql = insert_or_update(entity, mysql, &["fooId", "fooText"]).unwrap();
        assert!(sql.contains("ON DUPLICATE KEY UPDATE"), "{sql}");
        let postgres = dialects.get("postgres").unwrap();
        let err = insert_or_update(entity, postgres, &["fooId"]).unwrap_err();
        assert!(err.to_string().contains("try-insert is off"), "{err}");
    }

    #[test]
    fn feed_insert_uses_the_enlisted_connection_and_skips_a_missing_catalog() {
        let script = Arc::new(Script {
            inserts: Mutex::new(Vec::new()),
            missing_feed: false,
            fail_inserts: AtomicBool::new(false),
        });
        let facade = open_with(Arc::new(ScriptDriver {
            script: Arc::clone(&script),
        }));
        facade.begin("moqui.test.Foo", None).unwrap();
        match_rules(&facade, "moqui.test.Foo", "create");
        let tx = facade.transactions_for("moqui.test.Foo").unwrap();
        register_feed_insert(&facade, &tx).unwrap();
        enlist(&tx);
        facade.commit("moqui.test.Foo").unwrap();
        let inserts = script.inserts.lock().expect("inserts");
        assert_eq!(inserts.len(), 1, "{inserts:?}");
        assert_eq!(inserts[0][1], Value::string("DOC1"));
        assert_eq!(inserts[0][2], Value::string("moqui.test.Foo"));
        assert_eq!(inserts[0][3], Value::string("F1"));
        assert_eq!(inserts[0][4], Value::string("index"));
        drop(inserts);
        let held = feed_service_names(&facade);
        assert!(held
            .iter()
            .any(|service| service.receive_service == "index#DataDocuments"));
        assert!(held
            .iter()
            .any(|service| service.delete_service == "delete#DataDocument"));

        facade.set_disable_data_feed(true);
        facade.begin("moqui.test.Foo", None).unwrap();
        match_rules(&facade, "moqui.test.Foo", "create");
        let tx = facade.transactions_for("moqui.test.Foo").unwrap();
        register_feed_insert(&facade, &tx).unwrap();
        enlist(&tx);
        facade.commit("moqui.test.Foo").unwrap();
        assert_eq!(script.inserts.lock().expect("inserts").len(), 1);
        facade.set_disable_data_feed(false);

        facade.set_sim_active(true);
        facade.begin("moqui.test.Foo", None).unwrap();
        match_rules(&facade, "moqui.test.Foo", "delete");
        let tx = facade.transactions_for("moqui.test.Foo").unwrap();
        register_feed_insert(&facade, &tx).unwrap();
        enlist(&tx);
        facade.commit("moqui.test.Foo").unwrap();
        assert_eq!(script.inserts.lock().expect("inserts").len(), 1);
        facade.set_sim_active(false);

        let missing = Arc::new(Script {
            inserts: Mutex::new(Vec::new()),
            missing_feed: true,
            fail_inserts: AtomicBool::new(false),
        });
        let facade = open_with(Arc::new(ScriptDriver {
            script: Arc::clone(&missing),
        }));
        facade.begin("moqui.test.Foo", None).unwrap();
        match_rules(&facade, "moqui.test.Foo", "create");
        let tx = facade.transactions_for("moqui.test.Foo").unwrap();
        register_feed_insert(&facade, &tx).unwrap();
        enlist(&tx);
        facade.commit("moqui.test.Foo").unwrap();
        assert!(missing.inserts.lock().expect("inserts").is_empty());
    }

    fn enlist(tx: &TransactionManager) {
        tx.with_connection(|conn| conn.execute("touch", &[])).unwrap();
    }

    #[test]
    fn autocommit_match_does_not_insert_on_a_later_commit() {
        let script = Arc::new(Script {
            inserts: Mutex::new(Vec::new()),
            missing_feed: false,
            fail_inserts: AtomicBool::new(false),
        });
        let facade = open_with(Arc::new(ScriptDriver {
            script: Arc::clone(&script),
        }));
        assert!(match_rules(&facade, "moqui.test.Foo", "create").is_empty());
        facade.begin("moqui.test.Foo", None).unwrap();
        let tx = facade.transactions_for("moqui.test.Foo").unwrap();
        register_feed_insert(&facade, &tx).unwrap();
        enlist(&tx);
        facade.commit("moqui.test.Foo").unwrap();
        assert!(script.inserts.lock().expect("inserts").is_empty());
    }

    fn create_foo(facade: &EntityFacade, id: &str) -> Result<(), Error> {
        let mut value = facade.make_value("moqui.test.Foo")?;
        value.set("fooId", Value::string(id));
        value.create(facade)
    }

    fn frame_touches() -> usize {
        TOUCHES.with(|touches| touches.borrow().values().map(Vec::len).sum())
    }

    #[test]
    fn a_failed_first_write_leaves_no_touch_and_feed_names_do_not_grow() {
        let script = Arc::new(Script {
            inserts: Mutex::new(Vec::new()),
            missing_feed: false,
            fail_inserts: AtomicBool::new(true),
        });
        let facade = open_with(Arc::new(ScriptDriver {
            script: Arc::clone(&script),
        }));
        facade.begin("moqui.test.Foo", None).unwrap();
        assert!(create_foo(&facade, "F9").is_err());
        assert_eq!(frame_touches(), 0, "the failed write noted nothing");
        facade.rollback("moqui.test.Foo").unwrap();
        assert_eq!(frame_touches(), 0);
        assert!(script.inserts.lock().expect("inserts").is_empty());

        script.fail_inserts.store(false, Ordering::SeqCst);
        facade.begin("moqui.test.Foo", None).unwrap();
        create_foo(&facade, "F1").unwrap();
        assert_eq!(frame_touches(), 1);
        facade.commit("moqui.test.Foo").unwrap();
        assert_eq!(frame_touches(), 0, "the commit took this frame's touches");
        let ids: Vec<Value> = script
            .inserts
            .lock()
            .expect("inserts")
            .iter()
            .map(|params| params[3].clone())
            .collect();
        assert_eq!(ids, vec![Value::string("F1")], "pending rows only for the written row");

        for id in ["F2", "F3"] {
            facade.begin("moqui.test.Foo", None).unwrap();
            create_foo(&facade, id).unwrap();
            facade.commit("moqui.test.Foo").unwrap();
        }
        assert_eq!(script.inserts.lock().expect("inserts").len(), 3);
        assert_eq!(
            feed_service_names(&facade).len(),
            1,
            "one entry per feed, whatever the commits"
        );
        assert_eq!(frame_touches(), 0);
    }
}
