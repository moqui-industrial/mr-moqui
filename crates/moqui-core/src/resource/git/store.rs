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

use std::collections::HashSet;
#[cfg(test)]
use std::collections::{BTreeMap, HashMap};
#[cfg(test)]
use std::sync::Mutex;

use gix::objs::Kind;
use gix::ObjectId;

use crate::context::Error;

/// A change to one ref: point `name` at `new` (`None`: delete it) if it still points at `expected` (`None`: the ref
/// must not exist).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefEdit {
    pub name: String,
    pub expected: Option<ObjectId>,
    pub new: Option<ObjectId>,
}

/// A ref that was created (`old` empty), moved, or deleted (`new` empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefChange {
    pub name: String,
    pub old: Option<ObjectId>,
    pub new: Option<ObjectId>,
}

/// The refs that differ between two listings of [`GitStore::list_refs`], by name.
pub(crate) fn ref_changes(before: &[(String, ObjectId)], after: &[(String, ObjectId)]) -> Vec<RefChange> {
    let before: std::collections::BTreeMap<&str, ObjectId> = before.iter().map(|(name, id)| (name.as_str(), *id)).collect();
    let after: std::collections::BTreeMap<&str, ObjectId> = after.iter().map(|(name, id)| (name.as_str(), *id)).collect();
    let mut names: Vec<&str> = before.keys().chain(after.keys()).copied().collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| {
            let (old, new) = (before.get(name).copied(), after.get(name).copied());
            (old != new).then(|| RefChange {
                name: name.to_string(),
                old,
                new,
            })
        })
        .collect()
}

/// What became of one [`RefEdit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RefOutcome {
    Updated,
    /// The ref no longer pointed at the expected object: another writer moved it first.
    Stale,
    /// Not tried, as another edit of the same atomic update could not be made.
    Aborted,
}

/// Objects by id and refs by full name (`refs/heads/main`). Objects are immutable, so a store may cache them.
pub(crate) trait GitStore: Send + Sync + std::fmt::Debug {
    /// The kind and raw content (no git header) of an object, or `None` when the store does not have it.
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error>;

    /// The size of an object's content without reading it all, where the store can.
    fn object_size(&self, id: &ObjectId) -> Result<Option<u64>, Error> {
        Ok(self.find(id)?.map(|(_, data)| data.len() as u64))
    }

    /// Which of `ids` the store has.
    fn contains_many(&self, ids: &[ObjectId]) -> Result<HashSet<ObjectId>, Error> {
        let mut found = HashSet::new();
        for id in ids {
            if self.object_size(id)?.is_some() {
                found.insert(*id);
            }
        }
        Ok(found)
    }

    /// The objects whose ids start with the hex digits `prefix`, at most `limit` of them.
    fn find_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<ObjectId>, Error>;

    /// Store an object and answer its id. Writing an object that is already there is not an error.
    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error>;

    /// Store many objects, skipping those already there. How many were new.
    fn write_batch(&self, objects: &mut dyn Iterator<Item = Result<(Kind, Vec<u8>), Error>>) -> Result<usize, Error> {
        let mut written = 0;
        for object in objects {
            let (kind, data) = object?;
            let id = object_id(kind, &data)?;
            if self.object_size(&id)?.is_none() {
                self.write(kind, &data)?;
                written += 1;
            }
        }
        Ok(written)
    }

    /// The object a ref points to, or `None` when there is no such ref.
    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error>;

    /// The refs whose full names start with `prefix` (`refs/` for all of them) and the objects they point to, by
    /// name. Symbolic refs (`HEAD`) are not listed.
    fn list_refs(&self, prefix: &str) -> Result<Vec<(String, ObjectId)>, Error>;

    /// Make `edits`, each only if its ref still points at its expected object. With `atomic` either every edit is
    /// made or none is: one that cannot be made is [`RefOutcome::Stale`] and the others [`RefOutcome::Aborted`].
    /// Without it each edit stands alone. One outcome per edit, in order.
    fn update_refs(&self, edits: &[RefEdit], atomic: bool) -> Result<Vec<RefOutcome>, Error>;

    /// The directory of a repository kept on disk, whose `objects/pack` a received pack is moved into.
    fn disk_path(&self) -> Option<&std::path::Path> {
        None
    }

    /// A handle on the object database of a repository on disk, for gix-pack's pack generator (which copies deltas
    /// from the packs there).
    fn pack_handle(&self) -> Option<gix::odb::store::Handle<std::sync::Arc<gix::odb::Store>>> {
        None
    }

    /// Point `name` at `new` if it still points at `expected` (`None`: the ref must not exist). False, and nothing
    /// changed, when another writer moved the ref first.
    fn compare_and_set_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<bool, Error> {
        let edit = RefEdit {
            name: name.to_string(),
            expected,
            new: Some(new),
        };
        Ok(self.update_refs(&[edit], true)?.first() == Some(&RefOutcome::Updated))
    }
}

/// Whether `text` is a usable object id prefix: hex digits, 4 to 40 of them.
pub(crate) fn is_hex_prefix(text: &str) -> bool {
    (4..=40).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
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
    refs: Mutex<BTreeMap<String, ObjectId>>,
}

#[cfg(test)]
impl GitStore for MemoryStore {
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error> {
        Ok(moqui_lib::sync::lock(&self.objects).get(id).cloned())
    }

    fn find_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<ObjectId>, Error> {
        let prefix = prefix.to_ascii_lowercase();
        let mut found: Vec<ObjectId> = moqui_lib::sync::lock(&self.objects)
            .keys()
            .filter(|id| id.to_string().starts_with(&prefix))
            .copied()
            .collect();
        found.sort();
        found.truncate(limit);
        Ok(found)
    }

    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error> {
        let id = object_id(kind, data)?;
        moqui_lib::sync::lock(&self.objects).insert(id, (kind, data.to_vec()));
        Ok(id)
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error> {
        Ok(moqui_lib::sync::lock(&self.refs).get(name).copied())
    }

    fn list_refs(&self, prefix: &str) -> Result<Vec<(String, ObjectId)>, Error> {
        Ok(moqui_lib::sync::lock(&self.refs)
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, id)| (name.clone(), *id))
            .collect())
    }

    fn update_refs(&self, edits: &[RefEdit], atomic: bool) -> Result<Vec<RefOutcome>, Error> {
        let mut refs = moqui_lib::sync::lock(&self.refs);
        let stale: Vec<bool> = edits
            .iter()
            .map(|edit| refs.get(&edit.name).copied() != edit.expected)
            .collect();
        if atomic && stale.contains(&true) {
            return Ok(stale
                .iter()
                .map(|stale| if *stale { RefOutcome::Stale } else { RefOutcome::Aborted })
                .collect());
        }
        Ok(edits
            .iter()
            .map(|edit| {
                if refs.get(&edit.name).copied() != edit.expected {
                    return RefOutcome::Stale;
                }
                match edit.new {
                    Some(new) => refs.insert(edit.name.clone(), new),
                    None => refs.remove(&edit.name),
                };
                RefOutcome::Updated
            })
            .collect())
    }
}
