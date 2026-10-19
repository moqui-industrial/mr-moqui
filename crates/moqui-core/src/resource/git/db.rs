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

use std::sync::Mutex;

use gix::objs::Kind;
use gix::ObjectId;
use moqui_lib::object::{basic_convert, Value};

use super::store::{object_id, GitStore};
use crate::context::Error;
use crate::entity::{EntityFacade, EntityValue};
use crate::script::bounded::BoundedMap;

pub(crate) const GIT_OBJECT: &str = "moqui.resource.GitObject";
pub(crate) const GIT_REF: &str = "moqui.resource.GitRef";

/// Objects kept in memory after a read: trees and commits are read over and over by path lookups and history walks.
const CACHE_OBJECTS: usize = 4096;
const CACHE_BYTES: usize = 32 * 1024 * 1024;

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

    fn cas(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<bool, Error> {
        let current = self.ref_row(name, true)?;
        let found = current.as_ref().and_then(|row| text(row.get("objectId")));
        if found != expected.map(|id| id.to_string()) {
            return Ok(false);
        }
        let mut row = match current {
            Some(row) => row,
            None => {
                let mut row = EntityValue::make(&self.entity, GIT_REF)?;
                row.set("repositoryId", Value::string(&self.repository_id));
                row.set("refName", Value::string(name));
                row.set("objectId", Value::string(new.to_string()));
                row.create(&self.entity)?;
                return Ok(true);
            }
        };
        row.set("objectId", Value::string(new.to_string()));
        row.update(&self.entity)?;
        Ok(true)
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        value => Some(moqui_lib::object::to_plain_string(value)).filter(|text| !text.is_empty()),
    }
}

fn kind_of(name: &str) -> Result<Kind, Error> {
    Kind::from_bytes(name.as_bytes()).map_err(|_| Error::new(format!("unknown git object type {name}")))
}

impl GitStore for DbStore {
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error> {
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

    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error> {
        let id = object_id(kind, data)?;
        if moqui_lib::sync::lock(&self.cache).get(&id).is_some() || self.object_size(&id)?.is_some() {
            return Ok(id);
        }
        let mut row = EntityValue::make(&self.entity, GIT_OBJECT)?;
        row.set("repositoryId", Value::string(&self.repository_id));
        row.set("objectId", Value::string(id.to_string()));
        row.set("objectType", Value::string(kind.to_string()));
        row.set("objectSize", Value::I64(i64::try_from(data.len()).unwrap_or(i64::MAX)));
        row.set("objectData", Value::Bytes(data.to_vec()));
        row.create(&self.entity)?;
        Ok(id)
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error> {
        match self.ref_row(name, false)?.and_then(|row| text(row.get("objectId"))) {
            Some(hex) => ObjectId::from_hex(hex.as_bytes())
                .map(Some)
                .map_err(|err| Error::new(format!("GitRef {name} of {}: {err}", self.repository_id))),
            None => Ok(None),
        }
    }

    /// Read the ref row for update, compare, then update or create it, in one transaction: on Postgres the row lock
    /// (or the primary key, for a ref not there yet) keeps two writers from both moving the branch.
    fn compare_and_set_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<bool, Error> {
        let scope = self.entity.transactions_for(GIT_REF)?.scope(None)?;
        let outcome = self.cas(name, expected, new);
        if outcome.is_ok() {
            scope.commit()?;
        }
        // A duplicate key on create means another writer made the ref first: a lost race, not an error.
        match outcome {
            Err(_) if expected.is_none() && self.read_ref(name)?.is_some_and(|id| id != new) => Ok(false),
            other => other,
        }
    }
}
