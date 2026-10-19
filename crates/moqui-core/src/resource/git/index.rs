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

//! The path index of a branch: each commit on its first-parent line with the files it changed (against its first
//! parent), and for each path (a file, or a directory with anything under it) the last commit that changed it. It is
//! built once per branch head and extended in place when the head moves forward, so a file list's modified dates and
//! users, a file's history (which follows an exact move: the same blob removed at one path and added at another in one
//! commit) and the log of commits under a directory read memory rather than walking the branch from its head on each
//! call. Indexes are kept per repository and branch, bounded in number and in bytes ([`super::GitProvider`]).

use std::collections::HashMap;
use std::sync::Arc;

use gix::objs::tree::EntryMode;
use gix::ObjectId;

use super::ops::{self, CommitInfo};
use super::store::GitStore;
use crate::context::{Error, ResourceVersion};

/// How many commits an index holds; older history of a longer branch is not indexed (its files' last change is then
/// the oldest indexed commit, and their history stops there).
pub(crate) const MAX_INDEX_COMMITS: usize = 200_000;

/// A file that a commit changed: its blob before (`None`: added) and after (`None`: removed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileChange {
    pub path: String,
    pub old: Option<ObjectId>,
    pub new: Option<ObjectId>,
}

/// A commit of the first-parent line and the files it changed.
#[derive(Debug)]
pub(crate) struct IndexedCommit {
    pub info: CommitInfo,
    pub changes: Vec<FileChange>,
}

impl IndexedCommit {
    /// As a version: the commit id, date, `Moqui-User-Id` and message (the note).
    pub(crate) fn version(&self, previous: Option<&IndexedCommit>) -> ResourceVersion {
        ResourceVersion {
            version_name: self.info.id.to_string(),
            previous_version_name: previous.map(|older| older.info.id.to_string()),
            version_date: ops::version_date(self.info.seconds),
            user_id: ops::user_of(&self.info.message),
            note: ops::note_of(&self.info.message),
        }
    }
}

/// One version of a file in its history: the commit and the file's blob there.
#[derive(Clone, Debug)]
pub(crate) struct FileVersion {
    pub commit: usize,
    pub blob: ObjectId,
}

/// The index of one branch at `head`.
#[derive(Debug)]
pub(crate) struct BranchIndex {
    pub head: ObjectId,
    /// Oldest first, so extending the index keeps the positions `last` points to.
    commits: Vec<Arc<IndexedCommit>>,
    /// Path (a file, or a directory: "" is the root) to the position of the last commit that changed it.
    last: HashMap<String, usize>,
    /// The branch has more commits than were indexed.
    pub truncated: bool,
    /// A rough size, for the bound on the cache.
    pub bytes: usize,
}

/// The files that differ between two trees (`None`: no tree), under `prefix`.
pub(crate) fn diff_trees(
    store: &dyn GitStore,
    old: Option<&ObjectId>,
    new: Option<&ObjectId>,
    prefix: &str,
    out: &mut Vec<FileChange>,
) -> Result<(), Error> {
    if old == new {
        return Ok(());
    }
    let entries = |id: Option<&ObjectId>| -> Result<Vec<(String, EntryMode, ObjectId)>, Error> {
        match id {
            Some(id) => Ok(ops::read_tree(store, id)?
                .entries
                .into_iter()
                .map(|entry| {
                    (
                        String::from_utf8_lossy(&entry.filename).into_owned(),
                        entry.mode,
                        entry.oid,
                    )
                })
                .collect()),
            None => Ok(Vec::new()),
        }
    };
    let (old_entries, new_entries) = (entries(old)?, entries(new)?);
    let old_map: HashMap<&str, (EntryMode, ObjectId)> = old_entries
        .iter()
        .map(|(name, mode, id)| (name.as_str(), (*mode, *id)))
        .collect();
    let new_map: HashMap<&str, (EntryMode, ObjectId)> = new_entries
        .iter()
        .map(|(name, mode, id)| (name.as_str(), (*mode, *id)))
        .collect();
    let mut names: Vec<&str> = old_map.keys().chain(new_map.keys()).copied().collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        // a submodule entry names a commit of another repository; it has no file here
        let old = old_map.get(name).copied().filter(|(mode, _)| !mode.is_commit());
        let new = new_map.get(name).copied().filter(|(mode, _)| !mode.is_commit());
        if old.map(|(_, id)| id) == new.map(|(_, id)| id) {
            continue;
        }
        let tree = |entry: Option<(EntryMode, ObjectId)>| entry.filter(|(mode, _)| mode.is_tree()).map(|(_, id)| id);
        let blob = |entry: Option<(EntryMode, ObjectId)>| entry.filter(|(mode, _)| !mode.is_tree()).map(|(_, id)| id);
        let (old_tree, new_tree) = (tree(old), tree(new));
        if old_tree.is_some() || new_tree.is_some() {
            diff_trees(store, old_tree.as_ref(), new_tree.as_ref(), &path, out)?;
        }
        let (old_blob, new_blob) = (blob(old), blob(new));
        if old_blob != new_blob {
            out.push(FileChange {
                path,
                old: old_blob,
                new: new_blob,
            });
        }
    }
    Ok(())
}

/// Commits from `from` along the first-parent line, newest first, until `stop` (not included) or the root, or `max`.
/// Whether `stop` was reached.
fn walk(
    store: &dyn GitStore,
    from: &ObjectId,
    stop: Option<&ObjectId>,
    max: usize,
) -> Result<(Vec<IndexedCommit>, bool, bool), Error> {
    let mut found = Vec::new();
    let mut current = ops::read_commit(store, from)?;
    loop {
        if Some(&current.id) == stop {
            return Ok((found, true, false));
        }
        if found.len() >= max {
            return Ok((found, false, true));
        }
        let parent = match current.parents.first() {
            Some(id) => Some(ops::read_commit(store, id)?),
            None => None,
        };
        let mut changes = Vec::new();
        diff_trees(
            store,
            parent.as_ref().map(|parent| &parent.tree),
            Some(&current.tree),
            "",
            &mut changes,
        )?;
        found.push(IndexedCommit { info: current, changes });
        match parent {
            Some(parent) => current = parent,
            None => return Ok((found, false, false)),
        }
    }
}

fn commit_bytes(commit: &IndexedCommit) -> usize {
    160 + commit.info.message.len()
        + commit
            .changes
            .iter()
            .map(|change| 64 + change.path.len())
            .sum::<usize>()
}

impl BranchIndex {
    /// The index of the branch whose head is `head`.
    pub(crate) fn build(store: &dyn GitStore, head: &ObjectId) -> Result<Self, Error> {
        let (mut commits, _, truncated) = walk(store, head, None, MAX_INDEX_COMMITS)?;
        commits.reverse();
        let mut index = Self {
            head: *head,
            commits: Vec::with_capacity(commits.len()),
            last: HashMap::new(),
            truncated,
            bytes: 0,
        };
        for commit in commits {
            index.push(commit);
        }
        Ok(index)
    }

    fn push(&mut self, commit: IndexedCommit) {
        let position = self.commits.len();
        self.bytes += commit_bytes(&commit);
        for change in &commit.changes {
            let mut path = change.path.as_str();
            loop {
                if let Some(existing) = self.last.get_mut(path) {
                    *existing = position;
                } else {
                    self.bytes += 48 + path.len();
                    self.last.insert(path.to_string(), position);
                }
                if path.is_empty() {
                    break;
                }
                path = path.rfind('/').map_or("", |slash| &path[..slash]);
            }
        }
        self.head = commit.info.id;
        self.commits.push(Arc::new(commit));
    }

    /// Bring the index to `head`: the new commits when the old head is on its first-parent line, else a new index (the
    /// branch was replaced, as a fetch of a rewritten remote does).
    pub(crate) fn advance(&mut self, store: &dyn GitStore, head: &ObjectId) -> Result<(), Error> {
        if self.head == *head {
            return Ok(());
        }
        let room = MAX_INDEX_COMMITS.saturating_sub(self.commits.len()).max(1);
        let (mut commits, reached, _) = walk(store, head, Some(&self.head), room)?;
        if !reached {
            *self = Self::build(store, head)?;
            return Ok(());
        }
        commits.reverse();
        for commit in commits {
            self.push(commit);
        }
        Ok(())
    }

    fn at(&self, position: usize) -> &IndexedCommit {
        &self.commits[position]
    }

    /// The last commit that changed `path` (a file, or anything under a directory; "" is the whole branch). A path
    /// older than the indexed commits of a truncated index answers the oldest one.
    pub(crate) fn last_change(&self, path: &str) -> Option<&IndexedCommit> {
        match self.last.get(path) {
            Some(position) => Some(self.at(*position)),
            None if self.truncated => self.commits.first().map(Arc::as_ref),
            None => None,
        }
    }

    /// The versions of the file at `path`, newest first: each commit whose copy of the file differs from its parent's
    /// (a commit that removes it is not one). The history follows a move back to the path the file had before: a
    /// commit that adds the file with the blob another path of the same commit removed.
    pub(crate) fn history(&self, path: &str) -> Vec<FileVersion> {
        let mut tracking = path.to_string();
        let mut found = Vec::new();
        for position in (0..self.commits.len()).rev() {
            let commit = self.at(position);
            let Some(change) = commit.changes.iter().find(|change| change.path == tracking) else {
                continue;
            };
            let Some(blob) = change.new else {
                continue;
            };
            found.push(FileVersion { commit: position, blob });
            if change.old.is_none() {
                if let Some(source) = commit
                    .changes
                    .iter()
                    .find(|other| other.new.is_none() && other.old == Some(blob) && other.path != tracking)
                {
                    tracking = source.path.clone();
                }
            }
        }
        found
    }

    /// [`Self::history`] as versions, each one's previous version the next older.
    pub(crate) fn versions(&self, path: &str) -> Vec<(ResourceVersion, FileVersion)> {
        let history = self.history(path);
        history
            .iter()
            .enumerate()
            .map(|(index, version)| {
                let previous = history.get(index + 1).map(|older| self.at(older.commit));
                (self.at(version.commit).version(previous), version.clone())
            })
            .collect()
    }

    /// The commits that changed anything at or under `path`, newest first, at most `limit`, each with the paths it
    /// changed there (relative to `path`; empty for the file itself).
    pub(crate) fn log(&self, path: &str, limit: usize) -> Vec<(ResourceVersion, Vec<String>)> {
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let mut found = Vec::new();
        for position in (0..self.commits.len()).rev() {
            if found.len() >= limit {
                break;
            }
            let commit = self.at(position);
            let paths: Vec<String> = commit
                .changes
                .iter()
                .filter_map(|change| {
                    if path.is_empty() {
                        Some(change.path.clone())
                    } else if change.path == path {
                        Some(String::new())
                    } else {
                        change.path.strip_prefix(&prefix).map(str::to_string)
                    }
                })
                .collect();
            if paths.is_empty() {
                continue;
            }
            let previous = position.checked_sub(1).map(|older| self.at(older));
            found.push((commit.version(previous), paths));
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use gix::objs::tree::EntryKind;
    use gix::objs::Kind;

    use super::super::ops::{commit, Author, Edit};
    use super::super::store::MemoryStore;
    use super::*;

    fn path(text: &str) -> Vec<String> {
        text.split('/').map(str::to_string).collect()
    }

    fn author(user: &str) -> Author {
        Author {
            name: user.to_string(),
            email: format!("{user}@example.com"),
            user_id: Some(user.to_string()),
        }
    }

    /// One commit of `edits`: `(path, Some(text))` writes, `(path, None)` removes, by `user` with `message`.
    fn change(store: &MemoryStore, user: &str, message: &str, edits: &[(&str, Option<&str>)]) -> ObjectId {
        let edits: Vec<Edit> = edits
            .iter()
            .map(|(at, text)| match text {
                Some(text) => Edit::Put(
                    path(at),
                    EntryKind::Blob,
                    store.write(Kind::Blob, text.as_bytes()).unwrap(),
                ),
                None => Edit::Remove(path(at)),
            })
            .collect();
        commit(store, "main", &author(user), |_| {
            Ok(Some((edits.clone(), message.to_string())))
        })
        .unwrap()
        .unwrap()
    }

    /// A move: the entry at `from` put at `to` and `from` removed, in one commit.
    fn rename(store: &MemoryStore, user: &str, from: &str, to: &str) -> ObjectId {
        let head = ops::head(store, "main").unwrap().unwrap();
        let (mode, id) = ops::resolve(store, &head.tree, &path(from)).unwrap().unwrap();
        commit(store, "main", &author(user), |_| {
            Ok(Some((
                vec![Edit::Remove(path(from)), Edit::Put(path(to), mode.kind(), id)],
                format!("Move {from} to {to}"),
            )))
        })
        .unwrap()
        .unwrap()
    }

    #[test]
    fn last_changes_are_per_path_with_their_commit_user_and_note() {
        let store = MemoryStore::default();
        let first = change(
            &store,
            "alice",
            "Add notes",
            &[("notes/a.md", Some("a1")), ("top.txt", Some("t"))],
        );
        let second = change(&store, "bob", "Bob's b", &[("notes/b.md", Some("b1"))]);
        let head = ops::head(&store, "main").unwrap().unwrap();
        let index = BranchIndex::build(&store, &head.id).unwrap();
        let last = |at: &str| index.last_change(at).map(|commit| commit.info.id);
        assert_eq!(last("notes/a.md"), Some(first));
        assert_eq!(last("top.txt"), Some(first));
        assert_eq!(last("notes/b.md"), Some(second));
        assert_eq!(
            last("notes"),
            Some(second),
            "a folder's last change is the newest under it"
        );
        assert_eq!(last(""), Some(second));
        assert_eq!(last("missing.txt"), None);
        let version = index.last_change("notes/b.md").unwrap().version(None);
        assert_eq!(version.user_id.as_deref(), Some("bob"));
        assert_eq!(version.note.as_deref(), Some("Bob's b"));
    }

    #[test]
    fn an_index_advances_in_place_and_rebuilds_when_the_branch_is_replaced() {
        let store = MemoryStore::default();
        change(&store, "alice", "one", &[("a.txt", Some("1"))]);
        let head = ops::head(&store, "main").unwrap().unwrap();
        let mut index = BranchIndex::build(&store, &head.id).unwrap();
        let before = index.bytes;
        let next = change(&store, "bob", "two", &[("a.txt", Some("2"))]);
        index.advance(&store, &next).unwrap();
        assert_eq!(index.head, next);
        assert_eq!(index.commits.len(), 2);
        assert!(index.bytes > before);
        assert_eq!(index.last_change("a.txt").unwrap().info.id, next);
        // a head whose line does not hold the old head (a replaced branch) is indexed anew
        let other = MemoryStore::default();
        let replaced = change(&other, "carol", "elsewhere", &[("z.txt", Some("z"))]);
        let tree = ops::read_commit(&other, &replaced).unwrap().tree;
        let blob = ops::resolve(&other, &tree, &path("z.txt")).unwrap().unwrap().1;
        store
            .write(Kind::Blob, &ops::object(&other, &blob, Kind::Blob).unwrap())
            .unwrap();
        store
            .write(Kind::Tree, &ops::object(&other, &tree, Kind::Tree).unwrap())
            .unwrap();
        store
            .write(Kind::Commit, &ops::object(&other, &replaced, Kind::Commit).unwrap())
            .unwrap();
        index.advance(&store, &replaced).unwrap();
        assert_eq!(index.commits.len(), 1);
        assert!(index.last_change("a.txt").is_none());
        assert_eq!(index.last_change("z.txt").unwrap().info.id, replaced);
    }

    #[test]
    fn history_follows_an_exact_move_and_a_directory_move() {
        let store = MemoryStore::default();
        let v1 = change(&store, "alice", "v1", &[("docs/plan.md", Some("one\n"))]);
        let v2 = change(&store, "bob", "v2", &[("docs/plan.md", Some("two\n"))]);
        let moved = rename(&store, "alice", "docs/plan.md", "archive/plan-2026.md");
        let v3 = change(&store, "carol", "v3", &[("archive/plan-2026.md", Some("three\n"))]);
        let folder = rename(&store, "bob", "archive", "old");
        let head = ops::head(&store, "main").unwrap().unwrap();
        let index = BranchIndex::build(&store, &head.id).unwrap();
        let versions = index.versions("old/plan-2026.md");
        let names: Vec<String> = versions
            .iter()
            .map(|(version, _)| version.version_name.clone())
            .collect();
        assert_eq!(
            names,
            [folder, v3, moved, v2, v1].map(|id| id.to_string()),
            "the moves are versions, and the history goes on at the old paths"
        );
        assert_eq!(versions[4].0.previous_version_name, None);
        assert_eq!(
            versions[0].0.previous_version_name.as_deref(),
            Some(v3.to_string().as_str())
        );
        let oldest = ops::object(&store, &versions[4].1.blob, Kind::Blob).unwrap();
        assert_eq!(oldest, b"one\n");
        // a file removed and added again keeps its older versions; the removal is not one
        change(&store, "alice", "gone", &[("x.txt", Some("x1"))]);
        change(&store, "alice", "rm", &[("x.txt", None)]);
        change(&store, "alice", "back", &[("x.txt", Some("x2"))]);
        let head = ops::head(&store, "main").unwrap().unwrap();
        let index = BranchIndex::build(&store, &head.id).unwrap();
        let notes: Vec<Option<String>> = index
            .versions("x.txt")
            .into_iter()
            .map(|(version, _)| version.note)
            .collect();
        assert_eq!(notes, [Some("back".to_string()), Some("gone".to_string())]);
    }

    #[test]
    fn the_log_under_a_folder_lists_the_commits_and_paths_there() {
        let store = MemoryStore::default();
        change(
            &store,
            "alice",
            "a",
            &[("team/a.md", Some("a")), ("other.txt", Some("o"))],
        );
        change(&store, "bob", "b", &[("other.txt", Some("o2"))]);
        change(
            &store,
            "carol",
            "c",
            &[("team/sub/c.md", Some("c")), ("team/a.md", Some("a2"))],
        );
        let head = ops::head(&store, "main").unwrap().unwrap();
        let index = BranchIndex::build(&store, &head.id).unwrap();
        let summary: Vec<(Option<String>, Vec<String>)> = index
            .log("team", 10)
            .into_iter()
            .map(|(version, paths)| (version.note, paths))
            .collect();
        assert_eq!(
            summary,
            [
                (Some("c".to_string()), vec!["a.md".to_string(), "sub/c.md".to_string()]),
                (Some("a".to_string()), vec!["a.md".to_string()]),
            ]
        );
        assert_eq!(index.log("", 1).len(), 1, "the limit applies");
        assert_eq!(
            index.log("team/a.md", 10)[0].1,
            [String::new()],
            "a file's own change has the empty path"
        );
    }
}
