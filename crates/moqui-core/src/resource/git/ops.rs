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

//! Paths, trees, commits and file history over any [`GitStore`]. Trees are decoded and written with `gix-object`
//! (whose entry order is git's), so the objects are byte for byte what `git` writes and a repository exported from the
//! database has the same commit ids.

use gix::bstr::{BString, ByteSlice};
use gix::objs::tree::{Entry, EntryKind, EntryMode};
use gix::objs::{Kind, Tree, WriteTo};
use gix::ObjectId;
use moqui_lib::object::Value;

use super::store::GitStore;
use crate::context::{Error, ResourceVersion};

/// Repositories are SHA-1.
pub(crate) const HASH: gix::hash::Kind = gix::hash::Kind::Sha1;

/// The trailer a commit names its Moqui user with, read back as a version's `userId`.
pub(crate) const USER_TRAILER: &str = "Moqui-User-Id: ";

/// How many commits a history walk reads before it stops, so a damaged repository cannot loop.
const MAX_WALK: usize = 1_000_000;

/// How many times a commit is rebuilt on a fresh head when another writer moved the branch first.
const COMMIT_ATTEMPTS: usize = 4;

/// A commit's parts the provider reads.
#[derive(Clone, Debug)]
pub(crate) struct CommitInfo {
    pub id: ObjectId,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    /// Committer time, seconds since the epoch.
    pub seconds: i64,
    pub message: String,
}

/// One entry of a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirEntry {
    pub name: String,
    pub is_file: bool,
    pub id: ObjectId,
}

/// A change to a tree: put an entry (a blob, or a whole subtree on a move) at a path, or remove what is there.
#[derive(Clone, Debug)]
pub(crate) enum Edit {
    Put(Vec<String>, EntryKind, ObjectId),
    Remove(Vec<String>),
}

/// Who a commit is by.
#[derive(Clone, Debug)]
pub(crate) struct Author {
    pub name: String,
    pub email: String,
    pub user_id: Option<String>,
}

fn missing(id: &ObjectId) -> Error {
    Error::new(format!("git object {id} is missing"))
}

/// The content of an object that must be of `kind`.
pub(crate) fn object(store: &dyn GitStore, id: &ObjectId, kind: Kind) -> Result<Vec<u8>, Error> {
    let (found, data) = store.find(id)?.ok_or_else(|| missing(id))?;
    if found != kind {
        return Err(Error::new(format!("git object {id} is a {found}, not a {kind}")));
    }
    Ok(data)
}

pub(crate) fn read_tree(store: &dyn GitStore, id: &ObjectId) -> Result<Tree, Error> {
    let data = object(store, id, Kind::Tree)?;
    gix::objs::TreeRef::from_bytes(&data, HASH)
        .map(|tree| tree.into_owned())
        .map_err(|err| Error::new(format!("git tree {id}: {err}")))
}

pub(crate) fn read_commit(store: &dyn GitStore, id: &ObjectId) -> Result<CommitInfo, Error> {
    let data = object(store, id, Kind::Commit)?;
    let commit =
        gix::objs::CommitRef::from_bytes(&data, HASH).map_err(|err| Error::new(format!("git commit {id}: {err}")))?;
    let seconds = commit
        .committer()
        .ok()
        .and_then(|signature| signature.time().ok())
        .map_or(0, |time| time.seconds);
    Ok(CommitInfo {
        id: *id,
        tree: commit.tree(),
        parents: commit.parents().collect(),
        seconds,
        message: commit.message.to_str_lossy().into_owned(),
    })
}

/// The full name of a branch's ref.
pub(crate) fn branch_ref(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

/// The commit a branch points to, or `None` for a branch with no commits yet.
pub(crate) fn head(store: &dyn GitStore, branch: &str) -> Result<Option<CommitInfo>, Error> {
    match store.read_ref(&branch_ref(branch))? {
        Some(id) => read_commit(store, &id).map(Some),
        None => Ok(None),
    }
}

/// The entry at `path` under the tree `root`: its mode and id. The empty path is the root tree itself.
pub(crate) fn resolve(
    store: &dyn GitStore,
    root: &ObjectId,
    path: &[String],
) -> Result<Option<(EntryMode, ObjectId)>, Error> {
    let mut current = (EntryMode::from(EntryKind::Tree), *root);
    for segment in path {
        if !current.0.is_tree() {
            return Ok(None);
        }
        let tree = read_tree(store, &current.1)?;
        let Some(entry) = tree.entries.iter().find(|entry| entry.filename == segment.as_bytes()) else {
            return Ok(None);
        };
        current = (entry.mode, entry.oid);
    }
    Ok(Some(current))
}

/// The entries of a tree, in git's order.
pub(crate) fn list(store: &dyn GitStore, tree: &ObjectId) -> Result<Vec<DirEntry>, Error> {
    Ok(read_tree(store, tree)?
        .entries
        .into_iter()
        .map(|entry| DirEntry {
            name: entry.filename.to_str_lossy().into_owned(),
            is_file: !entry.mode.is_tree(),
            id: entry.oid,
        })
        .collect())
}

fn write_tree(store: &dyn GitStore, tree: &Tree) -> Result<ObjectId, Error> {
    let mut buf = Vec::with_capacity(64 * tree.entries.len());
    tree.write_to(&mut buf)
        .map_err(|err| Error::new(format!("encoding a git tree: {err}")))?;
    store.write(Kind::Tree, &buf)
}

/// Apply one edit below the tree `id` (`None`: no tree yet). The new tree's id, or `None` when it is left empty, as
/// git does not keep empty directories.
fn edit_tree(
    store: &dyn GitStore,
    id: Option<&ObjectId>,
    path: &[String],
    put: Option<(EntryKind, ObjectId)>,
) -> Result<Option<ObjectId>, Error> {
    let mut tree = match id {
        Some(id) => read_tree(store, id)?,
        None => Tree::empty(),
    };
    let Some((name, rest)) = path.split_first() else {
        return Err(Error::new("a git path is empty"));
    };
    let position = tree.entries.iter().position(|entry| entry.filename == name.as_bytes());
    let replacement = if rest.is_empty() {
        put
    } else {
        let child = match position.map(|index| &tree.entries[index]) {
            Some(entry) if entry.mode.is_tree() => Some(entry.oid),
            Some(_) => return Err(Error::new(format!("found file instead of directory at {name}"))),
            None if put.is_none() => return Ok(id.copied()),
            None => None,
        };
        edit_tree(store, child.as_ref(), rest, put)?.map(|child| (EntryKind::Tree, child))
    };
    match (position, replacement) {
        (Some(index), Some((kind, oid))) => {
            tree.entries[index].mode = kind.into();
            tree.entries[index].oid = oid;
        }
        (Some(index), None) => {
            tree.entries.remove(index);
        }
        (None, Some((kind, oid))) => tree.entries.push(Entry {
            mode: kind.into(),
            filename: BString::from(name.as_str()),
            oid,
        }),
        (None, None) => return Ok(id.copied()),
    }
    if tree.entries.is_empty() {
        return Ok(None);
    }
    tree.entries.sort();
    write_tree(store, &tree).map(Some)
}

/// The tree `root` with `edits` applied; the empty tree when nothing is left.
pub(crate) fn apply_edits(store: &dyn GitStore, root: Option<&ObjectId>, edits: &[Edit]) -> Result<ObjectId, Error> {
    let mut current = root.copied();
    for edit in edits {
        current = match edit {
            Edit::Put(path, kind, id) => edit_tree(store, current.as_ref(), path, Some((*kind, *id)))?,
            Edit::Remove(path) => edit_tree(store, current.as_ref(), path, None)?,
        };
    }
    match current {
        Some(id) => Ok(id),
        None => write_tree(store, &Tree::empty()),
    }
}

/// Commit `edits` on `branch`. `edits` is called with the head commit (or `None` on an empty branch) and answers the
/// changes and the message, or `None` for nothing to commit; it is called again on a fresh head if another writer
/// moves the branch first. The new commit's id, or `None` when there was nothing to commit or the tree is unchanged.
pub(crate) fn commit(
    store: &dyn GitStore,
    branch: &str,
    author: &Author,
    mut edits: impl FnMut(Option<&CommitInfo>) -> Result<Option<(Vec<Edit>, String)>, Error>,
) -> Result<Option<ObjectId>, Error> {
    let name = branch_ref(branch);
    for _ in 0..COMMIT_ATTEMPTS {
        let head = head(store, branch)?;
        let Some((changes, message)) = edits(head.as_ref())? else {
            return Ok(None);
        };
        let tree = apply_edits(store, head.as_ref().map(|head| &head.tree), &changes)?;
        if head.as_ref().is_some_and(|head| head.tree == tree) {
            return Ok(None);
        }
        let signature = gix::actor::Signature {
            name: author.name.as_str().into(),
            email: author.email.as_str().into(),
            time: gix::date::Time::now_local_or_utc(),
        };
        let mut message = message.trim_end().to_string();
        message.push('\n');
        if let Some(user_id) = &author.user_id {
            message.push('\n');
            message.push_str(USER_TRAILER);
            message.push_str(user_id);
            message.push('\n');
        }
        let commit = gix::objs::Commit {
            tree,
            parents: head.as_ref().map(|head| head.id).into_iter().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: message.into(),
            extra_headers: Vec::new(),
        };
        let mut buf = Vec::with_capacity(256);
        commit
            .write_to(&mut buf)
            .map_err(|err| Error::new(format!("encoding a git commit: {err}")))?;
        let id = store.write(Kind::Commit, &buf)?;
        if store.compare_and_set_ref(&name, head.as_ref().map(|head| head.id), id)? {
            return Ok(Some(id));
        }
    }
    Err(Error::new(format!(
        "branch {branch} kept moving; the commit was not made after {COMMIT_ATTEMPTS} attempts"
    )))
}

/// A person and time on a commit or tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the epoch.
    pub seconds: i64,
    /// The time zone offset, in seconds east of UTC.
    pub offset: i32,
}

impl Signature {
    fn of(signature: gix::actor::SignatureRef<'_>) -> Self {
        let time = signature.time().unwrap_or_default();
        Self {
            name: signature.name.trim().to_str_lossy().into_owned(),
            email: signature.email.trim().to_str_lossy().into_owned(),
            seconds: time.seconds,
            offset: time.offset,
        }
    }

    fn unknown() -> Self {
        Self {
            name: String::new(),
            email: String::new(),
            seconds: 0,
            offset: 0,
        }
    }
}

/// Everything a commit says, for logs, the commit view and gitECA rules.
#[derive(Clone, Debug)]
pub(crate) struct CommitDetails {
    pub id: ObjectId,
    pub tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
}

impl CommitDetails {
    /// The first line of the message.
    pub(crate) fn summary(&self) -> &str {
        self.message.lines().next().unwrap_or("").trim()
    }

    /// The `Name: value` lines of the message's last paragraph, when every line of it is one (a continuation line
    /// that starts with a space or tab is joined to the one before), as git reads trailers. The subject alone is not a
    /// trailer block.
    pub(crate) fn trailers(&self) -> Vec<(String, String)> {
        trailers(&self.message)
    }
}

/// The trailers of a commit or tag message: see [`CommitDetails::trailers`].
pub(crate) fn trailers(message: &str) -> Vec<(String, String)> {
    let paragraphs: Vec<&str> = message
        .trim()
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .collect();
    if paragraphs.len() < 2 {
        return Vec::new();
    }
    let mut found: Vec<(String, String)> = Vec::new();
    for line in paragraphs[paragraphs.len() - 1].lines() {
        if line.starts_with([' ', '\t']) {
            match found.last_mut() {
                Some((_, value)) => {
                    value.push(' ');
                    value.push_str(line.trim());
                    continue;
                }
                None => return Vec::new(),
            }
        }
        let Some((name, value)) = line.split_once(':') else {
            return Vec::new();
        };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
            return Vec::new();
        }
        found.push((name.to_string(), value.trim().to_string()));
    }
    found
}

pub(crate) fn commit_details(store: &dyn GitStore, id: &ObjectId) -> Result<CommitDetails, Error> {
    let data = object(store, id, Kind::Commit)?;
    let commit =
        gix::objs::CommitRef::from_bytes(&data, HASH).map_err(|err| Error::new(format!("git commit {id}: {err}")))?;
    Ok(CommitDetails {
        id: *id,
        tree: commit.tree(),
        parents: commit.parents().collect(),
        author: commit.author().map_or_else(|_| Signature::unknown(), Signature::of),
        committer: commit.committer().map_or_else(|_| Signature::unknown(), Signature::of),
        message: commit.message.to_str_lossy().into_owned(),
    })
}

/// An annotated tag object.
#[derive(Clone, Debug)]
pub(crate) struct TagDetails {
    pub name: String,
    pub target: ObjectId,
    pub target_kind: Kind,
    pub tagger: Option<Signature>,
    pub message: String,
}

pub(crate) fn tag_details(store: &dyn GitStore, id: &ObjectId) -> Result<TagDetails, Error> {
    let data = object(store, id, Kind::Tag)?;
    let tag = gix::objs::TagRef::from_bytes(&data, HASH).map_err(|err| Error::new(format!("git tag {id}: {err}")))?;
    Ok(TagDetails {
        name: tag.name.to_str_lossy().into_owned(),
        target: tag.target(),
        target_kind: tag.target_kind,
        tagger: tag.tagger().ok().flatten().map(Signature::of),
        message: tag.message.to_str_lossy().into_owned(),
    })
}

/// The object `revision` names in a repository whose default branch is `branch`: empty or `HEAD` for that branch's
/// head, a full ref name, a branch or tag name, a full object id, or a unique prefix of at least 4 hex digits.
/// `None` when it names nothing; an ambiguous prefix is an error.
pub(crate) fn resolve_revision(store: &dyn GitStore, branch: &str, revision: &str) -> Result<Option<ObjectId>, Error> {
    let revision = revision.trim();
    if revision.is_empty() || revision == "HEAD" {
        return store.read_ref(&branch_ref(branch));
    }
    if revision.len() == 40 {
        if let Ok(id) = ObjectId::from_hex(revision.as_bytes()) {
            return Ok(store.object_size(&id)?.map(|_| id));
        }
    }
    let names = if revision.starts_with("refs/") {
        vec![revision.to_string()]
    } else {
        vec![branch_ref(revision), format!("refs/tags/{revision}")]
    };
    for name in names {
        if let Some(id) = store.read_ref(&name)? {
            return Ok(Some(id));
        }
    }
    let found = store.find_prefix(revision, 2)?;
    match found.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => Err(Error::new(format!("{revision} names more than one object; give more digits"))),
    }
}

/// The `Moqui-User-Id` trailer of a commit message.
pub(crate) fn user_of(message: &str) -> Option<String> {
    message
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(USER_TRAILER))
        .map(|user| user.trim().to_string())
        .filter(|user| !user.is_empty())
}

fn version_date(seconds: i64) -> Value {
    jiff::Timestamp::from_second(seconds).map_or(Value::Null, Value::Timestamp)
}

/// The versions of the file at `path`, newest first: the commits on the first-parent line from `head` whose file
/// differs from the one in their first parent (a commit that deletes the file is not a version). Each version's
/// previous version is the next older one.
pub(crate) fn history(store: &dyn GitStore, head: &ObjectId, path: &[String]) -> Result<Vec<ResourceVersion>, Error> {
    // (commit, the file's blob there) along the first-parent line
    let mut line: Vec<(CommitInfo, Option<ObjectId>)> = Vec::new();
    let mut next = Some(*head);
    while let Some(id) = next {
        if line.len() >= MAX_WALK {
            return Err(Error::new(format!("history walk passed {MAX_WALK} commits")));
        }
        let commit = read_commit(store, &id)?;
        let blob = resolve(store, &commit.tree, path)?
            .filter(|(mode, _)| !mode.is_tree())
            .map(|(_, blob)| blob);
        next = commit.parents.first().copied();
        line.push((commit, blob));
    }
    let mut kept: Vec<&(CommitInfo, Option<ObjectId>)> = Vec::new();
    for (index, entry) in line.iter().enumerate() {
        let older = line.get(index + 1).and_then(|(_, blob)| *blob);
        if entry.1.is_some() && entry.1 != older {
            kept.push(entry);
        }
    }
    Ok(kept
        .iter()
        .enumerate()
        .map(|(index, (commit, _))| ResourceVersion {
            version_name: commit.id.to_string(),
            previous_version_name: kept.get(index + 1).map(|(older, _)| older.id.to_string()),
            version_date: version_date(commit.seconds),
            user_id: user_of(&commit.message),
        })
        .collect())
}

/// The bytes of the file at `path` in commit `commit`, or `None` when it is not a file there.
pub(crate) fn read_at(store: &dyn GitStore, commit: &ObjectId, path: &[String]) -> Result<Option<Vec<u8>>, Error> {
    let commit = read_commit(store, commit)?;
    match resolve(store, &commit.tree, path)? {
        Some((mode, blob)) if !mode.is_tree() => object(store, &blob, Kind::Blob).map(Some),
        _ => Ok(None),
    }
}

/// Every object reachable from `head`: its commits, their trees and the blobs in them. For copying a repository from
/// one store to another; `seen` holds what the target already has, so a later copy only walks what is new.
pub(crate) fn reachable(
    store: &dyn GitStore,
    head: &ObjectId,
    seen: &mut std::collections::HashSet<ObjectId>,
    mut each: impl FnMut(&ObjectId, Kind, &[u8]) -> Result<(), Error>,
) -> Result<usize, Error> {
    let mut count = 0;
    let mut pending = vec![(*head, Kind::Commit)];
    while let Some((id, kind)) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        // a submodule entry names a commit of another repository; it is not here
        if kind == Kind::Commit && id != *head && store.find(&id)?.is_none() {
            continue;
        }
        let (found, data) = store.find(&id)?.ok_or_else(|| missing(&id))?;
        match found {
            Kind::Commit => {
                let commit = gix::objs::CommitRef::from_bytes(&data, HASH)
                    .map_err(|err| Error::new(format!("git commit {id}: {err}")))?;
                pending.push((commit.tree(), Kind::Tree));
                pending.extend(commit.parents().map(|parent| (parent, Kind::Commit)));
            }
            Kind::Tree => {
                let tree = gix::objs::TreeRef::from_bytes(&data, HASH)
                    .map_err(|err| Error::new(format!("git tree {id}: {err}")))?;
                for entry in tree.entries {
                    let kind = if entry.mode.is_tree() {
                        Kind::Tree
                    } else if entry.mode.is_commit() {
                        Kind::Commit
                    } else {
                        Kind::Blob
                    };
                    pending.push((entry.oid.to_owned(), kind));
                }
            }
            Kind::Tag => {
                let tag = gix::objs::TagRef::from_bytes(&data, HASH)
                    .map_err(|err| Error::new(format!("git tag {id}: {err}")))?;
                pending.push((tag.target(), tag.target_kind));
            }
            Kind::Blob => {}
        }
        each(&id, found, &data)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::super::store::MemoryStore;
    use super::*;

    fn path(text: &str) -> Vec<String> {
        text.split('/').map(str::to_string).collect()
    }

    fn author() -> Author {
        Author {
            name: "Test User".to_string(),
            email: "test@example.com".to_string(),
            user_id: Some("EX_JOHN_DOE".to_string()),
        }
    }

    fn put(store: &dyn GitStore, at: &str, text: &str) -> Option<ObjectId> {
        let blob = store.write(Kind::Blob, text.as_bytes()).unwrap();
        let at = path(at);
        commit(store, "main", &author(), |_| {
            Ok(Some((
                vec![Edit::Put(at.clone(), EntryKind::Blob, blob)],
                "put".to_string(),
            )))
        })
        .unwrap()
    }

    #[test]
    fn trees_commits_and_history_over_a_store() {
        let store = MemoryStore::default();
        let first = put(&store, "notes/a.md", "one\n").expect("first commit");
        put(&store, "notes/b.md", "other\n").expect("second commit");
        let third = put(&store, "notes/a.md", "two\n").expect("third commit");
        assert_eq!(
            put(&store, "notes/a.md", "two\n"),
            None,
            "an unchanged tree makes no commit"
        );
        let head = head(&store, "main").unwrap().unwrap();
        assert_eq!(head.id, third);
        assert_eq!(head.parents.len(), 1);
        assert_eq!(user_of(&head.message).as_deref(), Some("EX_JOHN_DOE"));
        let names: Vec<String> = list(&store, &head.tree)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["notes"]);
        let notes = resolve(&store, &head.tree, &path("notes")).unwrap().unwrap();
        assert!(notes.0.is_tree());
        assert_eq!(list(&store, &notes.1).unwrap().len(), 2);
        let versions = history(&store, &head.id, &path("notes/a.md")).unwrap();
        let names: Vec<&str> = versions.iter().map(|version| version.version_name.as_str()).collect();
        assert_eq!(names, [third.to_string(), first.to_string()]);
        assert_eq!(
            versions[0].previous_version_name.as_deref(),
            Some(first.to_string().as_str())
        );
        assert_eq!(versions[0].user_id.as_deref(), Some("EX_JOHN_DOE"));
        assert_eq!(
            read_at(&store, &first, &path("notes/a.md")).unwrap(),
            Some(b"one\n".to_vec())
        );
        assert_eq!(read_at(&store, &first, &path("notes/b.md")).unwrap(), None);
    }

    #[test]
    fn removing_the_last_file_removes_its_directories() {
        let store = MemoryStore::default();
        put(&store, "a/b/c.txt", "x").unwrap();
        put(&store, "top.txt", "y").unwrap();
        commit(&store, "main", &author(), |_| {
            Ok(Some((vec![Edit::Remove(path("a/b/c.txt"))], "rm".to_string())))
        })
        .unwrap()
        .unwrap();
        let head = head(&store, "main").unwrap().unwrap();
        let names: Vec<String> = list(&store, &head.tree)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["top.txt"]);
        let missing = commit(&store, "main", &author(), |_| {
            Ok(Some((vec![Edit::Remove(path("no/such.txt"))], "rm".to_string())))
        })
        .unwrap();
        assert_eq!(missing, None);
        let blob = store.write(Kind::Blob, b"z").unwrap();
        let err = commit(&store, "main", &author(), |_| {
            Ok(Some((
                vec![Edit::Put(path("top.txt/below.txt"), EntryKind::Blob, blob)],
                "x".to_string(),
            )))
        })
        .unwrap_err();
        assert!(err.to_string().contains("found file instead"), "{err}");
    }

    #[test]
    fn a_moved_branch_is_retried_on_the_new_head() {
        let store = MemoryStore::default();
        put(&store, "a.txt", "1").unwrap();
        let blob = store.write(Kind::Blob, b"2").unwrap();
        let mut calls = 0;
        let made = commit(&store, "main", &author(), |_| {
            calls += 1;
            if calls == 1 {
                // another writer commits between this one's read of the head and its ref update
                put(&store, "b.txt", "other writer").unwrap();
            }
            Ok(Some((
                vec![Edit::Put(path("c.txt"), EntryKind::Blob, blob)],
                "c".to_string(),
            )))
        })
        .unwrap()
        .unwrap();
        assert_eq!(calls, 2);
        let head = head(&store, "main").unwrap().unwrap();
        assert_eq!(head.id, made);
        let names: Vec<String> = list(&store, &head.tree)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["a.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn trailers_are_the_last_paragraph_when_every_line_is_one() {
        let found = trailers("Fix login\n\nLonger text.\n\nTask: PROJ-100023\nSigned-off-by: A <a@b.c>\n  continued\n");
        assert_eq!(
            found,
            [
                ("Task".to_string(), "PROJ-100023".to_string()),
                ("Signed-off-by".to_string(), "A <a@b.c> continued".to_string())
            ]
        );
        assert!(trailers("Task: only a subject\n").is_empty());
        assert!(trailers("Subject\n\nTask: X\nnot a trailer\n").is_empty());
        assert!(trailers("Subject\n\nSee: http://x\n").len() == 1);
    }

    #[test]
    fn git_entry_order_puts_a_directory_after_a_dotted_file() {
        let store = MemoryStore::default();
        put(&store, "a.b", "file").unwrap();
        put(&store, "a/x", "in dir").unwrap();
        put(&store, "a-c", "file").unwrap();
        let head = head(&store, "main").unwrap().unwrap();
        let names: Vec<String> = list(&store, &head.tree)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        // git compares a directory name as if it ended in '/', so "a" sorts after "a-c" and "a.b"
        assert_eq!(names, ["a-c", "a.b", "a"]);
    }
}
