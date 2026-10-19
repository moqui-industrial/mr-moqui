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

//! Where a repository's objects and refs live. Everything above this (paths, trees, commits, history) is written once
//! in `ops` over [`GitStore`]; a bare repository on disk ([`super::fs::FsStore`]) and rows in `GitObject` and `GitRef`
//! ([`super::db::DbStore`]) are the two kinds of storage.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

use gix::objs::Kind;
use gix::ObjectId;

use crate::context::Error;

/// Objects by id and refs by full name (`refs/heads/main`). Objects are immutable, so a store may cache them.
pub(crate) trait GitStore: Send + Sync + std::fmt::Debug {
    /// The kind and raw content (no git header) of an object, or `None` when the store does not have it.
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error>;

    /// The size of an object's content without reading it all, where the store can.
    fn object_size(&self, id: &ObjectId) -> Result<Option<u64>, Error> {
        Ok(self.find(id)?.map(|(_, data)| data.len() as u64))
    }

    /// Store an object and answer its id. Writing an object that is already there is not an error.
    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error>;

    /// The object a ref points to, or `None` when there is no such ref.
    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error>;

    /// Point `name` at `new` if it still points at `expected` (`None`: the ref must not exist). False, and nothing
    /// changed, when another writer moved the ref first.
    fn compare_and_set_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<bool, Error>;
}

/// The SHA-1 id of an object, as git computes it.
pub(crate) fn object_id(kind: Kind, data: &[u8]) -> Result<ObjectId, Error> {
    gix::objs::compute_hash(super::ops::HASH, kind, data).map_err(|err| Error::new(format!("hashing a {kind}: {err}")))
}

/// A store in memory, for tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MemoryStore {
    objects: Mutex<HashMap<ObjectId, (Kind, Vec<u8>)>>,
    refs: Mutex<HashMap<String, ObjectId>>,
}

#[cfg(test)]
impl GitStore for MemoryStore {
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error> {
        Ok(moqui_lib::sync::lock(&self.objects).get(id).cloned())
    }

    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error> {
        let id = object_id(kind, data)?;
        moqui_lib::sync::lock(&self.objects).insert(id, (kind, data.to_vec()));
        Ok(id)
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error> {
        Ok(moqui_lib::sync::lock(&self.refs).get(name).copied())
    }

    fn compare_and_set_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<bool, Error> {
        let mut refs = moqui_lib::sync::lock(&self.refs);
        if refs.get(name).copied() != expected {
            return Ok(false);
        }
        refs.insert(name.to_string(), new);
        Ok(true)
    }
}
