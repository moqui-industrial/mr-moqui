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

//! The `db` kind: a repository whose objects and refs are rows in `moqui.resource.GitObject` and `GitRef`, so the
//! database backup holds it. Objects are stored raw (no git header, uncompressed; Postgres compresses large bytea
//! values itself) and are byte for byte git's, so `export#GitRepository` writes a repository with the same ids.

use std::collections::HashSet;
use std::sync::Mutex;

use gix::objs::Kind;
use gix::ObjectId;
use moqui_lib::object::{basic_convert, Value};

use super::store::{is_hex_prefix, object_id, GitStore, RefEdit, RefOutcome};
use crate::context::Error;
use crate::entity::{Condition, EntityFacade, EntityValue};
use crate::script::bounded::BoundedMap;

pub(crate) const GIT_OBJECT: &str = "moqui.resource.GitObject";
pub(crate) const GIT_REF: &str = "moqui.resource.GitRef";

/// Objects kept in memory after a read: trees and commits are read over and over by path lookups and history walks.
const CACHE_OBJECTS: usize = 4096;
const CACHE_BYTES: usize = 32 * 1024 * 1024;

/// Ids per `IN (...)` query and objects per insert transaction.
const BATCH: usize = 500;

/// Read objects by id: their kind and content, shared.
type ObjectCache = Mutex<BoundedMap<ObjectId, (Kind, std::sync::Arc<Vec<u8>>)>>;

pub(crate) struct DbStore {
    entity: EntityFacade,
    repository_id: String,
    cache: ObjectCache,
}

impl std::fmt::Debug for DbStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbStore")
            .field("repository_id", &self.repository_id)
            .finish()
    }
}

impl DbStore {
    pub(crate) fn new(entity: EntityFacade, repository_id: &str) -> Self {
        Self {
            entity,
            repository_id: repository_id.to_string(),
            cache: Mutex::new(BoundedMap::with_bytes(CACHE_OBJECTS, CACHE_BYTES)),
        }
    }

    fn row(&self, id: &ObjectId) -> Result<Option<EntityValue>, Error> {
        Ok(self
            .entity
            .find(GIT_OBJECT)?
            .eq("repositoryId", Value::string(&self.repository_id))
            .eq("objectId", Value::string(id.to_string()))
            .one()?)
    }

    fn ref_row(&self, name: &str, for_update: bool) -> Result<Option<EntityValue>, Error> {
        Ok(self
            .entity
            .find(GIT_REF)?
            .eq("repositoryId", Value::string(&self.repository_id))
            .eq("refName", Value::string(name))
            .for_update(for_update)
            .one()?)
    }

    /// Whether a locked ref row still points where `edit` expects.
    fn matches(row: Option<&EntityValue>, edit: &RefEdit) -> bool {
        row.and_then(|row| text(row.get("objectId"))) == edit.expected.map(|id| id.to_string())
    }

    /// Make one edit to its locked row (`None`: the ref is not there).
    fn apply(&self, row: Option<EntityValue>, edit: &RefEdit) -> Result<(), Error> {
        match (row, edit.new) {
            (Some(mut row), Some(new)) => {
                row.set("objectId", Value::string(new.to_string()));
                Ok(row.update(&self.entity)?)
            }
            (Some(row), None) => Ok(row.delete(&self.entity)?),
            (None, Some(new)) => {
                let mut row = EntityValue::make(&self.entity, GIT_REF)?;
                row.set("repositoryId", Value::string(&self.repository_id));
                row.set("refName", Value::string(&edit.name));
                row.set("objectId", Value::string(new.to_string()));
                Ok(row.create(&self.entity)?)
            }
            (None, None) => Ok(()),
        }
    }

    /// Edits in one transaction: the rows are read for update (in name order, so two updates lock in the same order)
    /// and compared before anything is written, so a stale atomic update writes nothing and needs no rollback, which
    /// matters when the scope joins a caller's transaction.
    fn update_in_scope(&self, edits: &[(usize, &RefEdit)], atomic: bool, outcomes: &mut [RefOutcome]) -> Result<(), Error> {
        let scope = self.entity.transactions_for(GIT_REF)?.scope(None)?;
        let mut rows = Vec::with_capacity(edits.len());
        for (index, edit) in edits {
            let row = self.ref_row(&edit.name, true)?;
            let stale = !Self::matches(row.as_ref(), edit);
            rows.push((*index, *edit, row, stale));
        }
        if atomic && rows.iter().any(|(_, _, _, stale)| *stale) {
            for (index, _, _, stale) in &rows {
                outcomes[*index] = if *stale { RefOutcome::Stale } else { RefOutcome::Aborted };
            }
            return Ok(scope.commit()?);
        }
        for (index, edit, row, stale) in rows {
            if stale {
                outcomes[index] = RefOutcome::Stale;
                continue;
            }
            self.apply(row, edit)?;
            outcomes[index] = RefOutcome::Updated;
        }
        Ok(scope.commit()?)
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        value => Some(moqui_lib::object::to_plain_string(value)).filter(|text| !text.is_empty()),
    }
}

impl DbStore {
    fn object_row(&self, id: ObjectId, kind: Kind, data: &[u8]) -> Result<EntityValue, Error> {
        let mut row = EntityValue::make(&self.entity, GIT_OBJECT)?;
        row.set("repositoryId", Value::string(&self.repository_id));
        row.set("objectId", Value::string(id.to_string()));
        row.set("objectType", Value::string(kind.to_string()));
        row.set("objectSize", Value::I64(i64::try_from(data.len()).unwrap_or(i64::MAX)));
        row.set("objectData", Value::Bytes(data.to_vec()));
        Ok(row)
    }
}

fn kind_of(name: &str) -> Result<Kind, Error> {
    Kind::from_bytes(name.as_bytes()).map_err(|_| Error::new(format!("unknown git object type {name}")))
}

impl GitStore for DbStore {
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error> {
        let _rows = super::rows_guard();
        if let Some((kind, data)) = moqui_lib::sync::lock(&self.cache).get(id) {
            return Ok(Some((kind, data.as_ref().clone())));
        }
        let Some(row) = self.row(id)? else {
            return Ok(None);
        };
        let kind = kind_of(&text(row.get("objectType")).unwrap_or_default())?;
        let data = match basic_convert(row.get("objectData").unwrap_or(&Value::Null), "java.sql.Blob").value {
            Value::Bytes(bytes) => bytes,
            Value::Null => Vec::new(),
            other => return Err(Error::new(format!("GitObject {id} data is {other:?}"))),
        };
        let size = data.len();
        moqui_lib::sync::lock(&self.cache).insert_sized(*id, (kind, std::sync::Arc::new(data.clone())), size);
        Ok(Some((kind, data)))
    }

    fn object_size(&self, id: &ObjectId) -> Result<Option<u64>, Error> {
        let _rows = super::rows_guard();
        let row = self
            .entity
            .find(GIT_OBJECT)?
            .eq("repositoryId", Value::string(&self.repository_id))
            .eq("objectId", Value::string(id.to_string()))
            .select_field("objectSize")
            .one()?;
        Ok(row.and_then(
            |row| match basic_convert(row.get("objectSize").unwrap_or(&Value::Null), "Long").value {
                Value::I64(size) => u64::try_from(size).ok(),
                _ => None,
            },
        ))
    }

    fn contains_many(&self, ids: &[ObjectId]) -> Result<HashSet<ObjectId>, Error> {
        let _rows = super::rows_guard();
        let mut found = HashSet::new();
        for chunk in ids.chunks(BATCH) {
            let rows = self
                .entity
                .find(GIT_OBJECT)?
                .eq("repositoryId", Value::string(&self.repository_id))
                .condition(Condition::is_in(
                    "objectId",
                    chunk.iter().map(|id| Value::string(id.to_string())).collect(),
                ))
                .select_field("objectId")
                .list()?;
            for row in rows.iter() {
                if let Some(id) = text(row.get("objectId")).and_then(|hex| ObjectId::from_hex(hex.as_bytes()).ok()) {
                    found.insert(id);
                }
            }
        }
        Ok(found)
    }

    fn find_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<ObjectId>, Error> {
        let _rows = super::rows_guard();
        if !is_hex_prefix(prefix) {
            return Ok(Vec::new());
        }
        let rows = self
            .entity
            .find(GIT_OBJECT)?
            .eq("repositoryId", Value::string(&self.repository_id))
            .condition(Condition::like(
                "objectId",
                Value::string(format!("{}%", prefix.to_ascii_lowercase())),
            ))
            .select_field("objectId")
            .order_by("objectId", false)
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .list()?;
        Ok(rows
            .iter()
            .filter_map(|row| text(row.get("objectId")).and_then(|hex| ObjectId::from_hex(hex.as_bytes()).ok()))
            .collect())
    }

    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error> {
        let _rows = super::rows_guard();
        let id = object_id(kind, data)?;
        if moqui_lib::sync::lock(&self.cache).get(&id).is_some() || self.object_size(&id)?.is_some() {
            return Ok(id);
        }
        self.object_row(id, kind, data)?.create(&self.entity)?;
        Ok(id)
    }

    /// Chunks of objects: one existence query, then the new ones created in one transaction.
    fn write_batch(&self, objects: &mut dyn Iterator<Item = Result<(Kind, Vec<u8>), Error>>) -> Result<usize, Error> {
        let _rows = super::rows_guard();
        let mut written = 0;
        let mut chunk: Vec<(ObjectId, Kind, Vec<u8>)> = Vec::with_capacity(BATCH);
        let mut flush = |chunk: &mut Vec<(ObjectId, Kind, Vec<u8>)>| -> Result<(), Error> {
            if chunk.is_empty() {
                return Ok(());
            }
            let ids: Vec<ObjectId> = chunk.iter().map(|(id, _, _)| *id).collect();
            let present = self.contains_many(&ids)?;
            let scope = self.entity.transactions_for(GIT_OBJECT)?.scope(None)?;
            let mut seen = HashSet::new();
            for (id, kind, data) in chunk.drain(..) {
                if present.contains(&id) || !seen.insert(id) {
                    continue;
                }
                self.object_row(id, kind, &data)?.create(&self.entity)?;
                written += 1;
            }
            Ok(scope.commit()?)
        };
        for object in objects {
            let (kind, data) = object?;
            let id = object_id(kind, &data)?;
            chunk.push((id, kind, data));
            if chunk.len() >= BATCH {
                flush(&mut chunk)?;
            }
        }
        flush(&mut chunk)?;
        Ok(written)
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error> {
        let _rows = super::rows_guard();
        match self.ref_row(name, false)?.and_then(|row| text(row.get("objectId"))) {
            Some(hex) => ObjectId::from_hex(hex.as_bytes())
                .map(Some)
                .map_err(|err| Error::new(format!("GitRef {name} of {}: {err}", self.repository_id))),
            None => Ok(None),
        }
    }

    fn list_refs(&self, prefix: &str) -> Result<Vec<(String, ObjectId)>, Error> {
        let _rows = super::rows_guard();
        let mut find = self
            .entity
            .find(GIT_REF)?
            .eq("repositoryId", Value::string(&self.repository_id))
            .order_by("refName", false);
        if !prefix.is_empty() {
            find = find.condition(Condition::like("refName", Value::string(format!("{prefix}%"))));
        }
        let mut refs = Vec::new();
        for row in find.list()?.iter() {
            let (Some(name), Some(hex)) = (text(row.get("refName")), text(row.get("objectId"))) else {
                continue;
            };
            // LIKE treats _ and % in the prefix as wildcards
            if !name.starts_with(prefix) {
                continue;
            }
            match ObjectId::from_hex(hex.as_bytes()) {
                Ok(id) => refs.push((name, id)),
                Err(err) => tracing::error!(repository = %self.repository_id, "GitRef {name} is not an object id: {err}"),
            }
        }
        Ok(refs)
    }

    /// Read each ref row for update, compare, then update, create or delete it: on Postgres the row lock (or the
    /// primary key, for a ref not there yet) keeps two writers from both moving a branch. An atomic update is one
    /// transaction; otherwise each edit has its own.
    fn update_refs(&self, edits: &[RefEdit], atomic: bool) -> Result<Vec<RefOutcome>, Error> {
        let _rows = super::rows_guard();
        let mut outcomes = vec![RefOutcome::Aborted; edits.len()];
        let mut ordered: Vec<(usize, &RefEdit)> = edits.iter().enumerate().collect();
        ordered.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        let groups: Vec<&[(usize, &RefEdit)]> = if atomic {
            vec![&ordered[..]]
        } else {
            ordered.chunks(1).collect()
        };
        for group in groups {
            if let Err(err) = self.update_in_scope(group, atomic, &mut outcomes) {
                // A duplicate key on create means another writer made the ref first: a lost race, not an error.
                let raced = group.iter().any(|(_, edit)| {
                    edit.expected.is_none() && self.read_ref(&edit.name).ok().flatten().is_some_and(|id| Some(id) != edit.new)
                });
                if !raced {
                    return Err(err);
                }
                for (index, edit) in group {
                    let now = self.read_ref(&edit.name)?;
                    outcomes[*index] = if now != edit.expected { RefOutcome::Stale } else { RefOutcome::Aborted };
                }
            }
        }
        Ok(outcomes)
    }
}
