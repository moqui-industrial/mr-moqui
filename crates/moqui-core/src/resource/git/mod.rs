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

//! `git://<repositoryId>/<path>` locations: files on the configured branch of a git repository, with versions that are
//! the commits that changed each file. No Java counterpart. A `moqui.resource.GitRepository` row defines each
//! repository and its kind:
//!
//! - `remote`: a bare mirror of `remoteUrl` under `localPath`, read only, changed only by `fetch#GitRepository`.
//! - `local`: a bare repository under `localPath` (created on first use); a write is a commit on the branch.
//! - `db`: the objects and refs are `GitObject` and `GitRef` rows, so the database backup holds the repository.
//!
//! A write commits at once, by the current user, with a `Moqui-User-Id` trailer the history reads back. Guide:
//! `docs/Revision-Management.md`.

pub(crate) mod db;
pub(crate) mod fs;
pub(crate) mod ops;
pub(crate) mod store;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use gix::objs::tree::EntryKind;
use gix::objs::Kind;
use gix::ObjectId;
use moqui_lib::object::{basic_convert, Value};

use self::ops::{Author, Edit};
use self::store::GitStore;
use crate::context::{Error, ResourceEntry, ResourceStat, ResourceVersion};
use crate::entity::{EntityFacade, EntityValue};

pub(crate) const GIT_REPOSITORY: &str = "moqui.resource.GitRepository";

/// The `resource-reference` class that names this provider in `MoquiDefaultConf.xml`.
pub const GIT_CLASS: &str = crate::native::GIT_RESOURCE_CLASS;

const DEFAULT_BRANCH: &str = "main";

/// How a repository is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryKind {
    Remote,
    Local,
    Db,
}

impl RepositoryKind {
    fn parse(repository_id: &str, text: &str) -> Result<Self, Error> {
        match text {
            "remote" => Ok(Self::Remote),
            "local" | "" => Ok(Self::Local),
            "db" => Ok(Self::Db),
            other => Err(Error::new(format!(
                "git repository {repository_id} has kind {other}; it must be remote, local or db"
            ))),
        }
    }
}

/// A `GitRepository` row.
#[derive(Clone, Debug)]
pub(crate) struct Definition {
    pub id: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub branch: String,
    pub path: PathBuf,
    pub credentials: fs::Credentials,
}

/// An open repository.
#[derive(Debug)]
struct Repo {
    definition: Definition,
    store: Box<dyn GitStore>,
    /// Writers in this process take turns; across processes the ref compare-and-set keeps a branch consistent.
    lock: Mutex<()>,
}

/// The `git` scheme of the resource facade, and the operations of `org.moqui.impl.GitServices`.
#[derive(Clone)]
pub struct GitProvider {
    inner: Arc<Inner>,
}

struct Inner {
    /// The factory's entity facade slot: it is installed after boot, so repositories are read on first use.
    entity: Arc<RwLock<Option<EntityFacade>>>,
    runtime_dir: PathBuf,
    repos: RwLock<HashMap<String, Arc<Repo>>>,
}

impl std::fmt::Debug for GitProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitProvider")
            .field("runtime_dir", &self.inner.runtime_dir)
            .finish()
    }
}

/// `git://repo/a/b` as the repository id and the path segments. `.` and `..` segments are refused.
fn parse_location(location: &str) -> Result<(String, Vec<String>), Error> {
    let rest = location
        .strip_prefix("git://")
        .or_else(|| location.strip_prefix("git:"))
        .ok_or_else(|| Error::new(format!("not a git location: {location}")))?;
    let mut segments = rest.split('/').filter(|segment| !segment.is_empty());
    let repository = segments
        .next()
        .ok_or_else(|| Error::new(format!("no repository in git location {location}")))?
        .to_string();
    let path: Vec<String> = segments.map(str::to_string).collect();
    if path
        .iter()
        .any(|segment| segment == "." || segment == ".." || segment == ".git")
    {
        return Err(Error::new(format!("bad path in git location {location}")));
    }
    Ok((repository, path))
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        value => Some(moqui_lib::object::to_plain_string(value)).filter(|text| !text.trim().is_empty()),
    }
}

fn long(value: Option<&Value>) -> Option<i64> {
    match basic_convert(value?, "Long").value {
        Value::I64(number) => Some(number),
        _ => None,
    }
}

fn millis(value: Option<&Value>) -> Option<i64> {
    match basic_convert(value?, "Timestamp").value {
        Value::Timestamp(stamp) => Some(stamp.as_millisecond()),
        _ => None,
    }
}

impl GitProvider {
    pub(crate) fn new(entity: Arc<RwLock<Option<EntityFacade>>>, runtime_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                entity,
                runtime_dir,
                repos: RwLock::new(HashMap::new()),
            }),
        }
    }

    fn entity(&self) -> Result<EntityFacade, Error> {
        moqui_lib::sync::read(&self.inner.entity)
            .clone()
            .ok_or_else(|| Error::new("git resources need the entity facade, which is not installed"))
    }

    /// The `GitRepository` row of `repository_id`.
    pub(crate) fn definition(&self, repository_id: &str) -> Result<Definition, Error> {
        let entity = self.entity()?;
        let row = entity
            .find(GIT_REPOSITORY)?
            .eq("repositoryId", Value::string(repository_id))
            .one()?
            .ok_or_else(|| {
                Error::new(format!(
                    "git repository {repository_id} is not defined (no moqui.resource.GitRepository row)"
                ))
            })?;
        let kind = RepositoryKind::parse(repository_id, &text(row.get("repositoryKind")).unwrap_or_default())?;
        let path = match text(row.get("localPath")) {
            Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
            Some(path) => self.inner.runtime_dir.join(path),
            None => self.inner.runtime_dir.join("git").join(repository_id),
        };
        Ok(Definition {
            id: repository_id.to_string(),
            kind,
            remote_url: text(row.get("remoteUrl")),
            branch: text(row.get("branch")).unwrap_or_else(|| DEFAULT_BRANCH.to_string()),
            path,
            credentials: fs::Credentials {
                username: text(row.get("username")),
                token: text(row.get("accessToken")),
            },
        })
    }

    fn open(&self, definition: Definition) -> Result<Repo, Error> {
        let store: Box<dyn GitStore> = match definition.kind {
            RepositoryKind::Remote => {
                if !definition.path.join("HEAD").is_file() {
                    return Err(Error::new(format!(
                        "git repository {} has not been fetched yet (org.moqui.impl.GitServices.fetch#GitRepository)",
                        definition.id
                    )));
                }
                Box::new(fs::FsStore::open(&definition.path)?)
            }
            RepositoryKind::Local => Box::new(fs::FsStore::init(&definition.path, &definition.branch)?),
            RepositoryKind::Db => Box::new(db::DbStore::new(self.entity()?, &definition.id)),
        };
        Ok(Repo {
            definition,
            store,
            lock: Mutex::new(()),
        })
    }

    fn repo(&self, repository_id: &str) -> Result<Arc<Repo>, Error> {
        if let Some(repo) = moqui_lib::sync::read(&self.inner.repos).get(repository_id) {
            return Ok(Arc::clone(repo));
        }
        let repo = Arc::new(self.open(self.definition(repository_id)?)?);
        moqui_lib::sync::write(&self.inner.repos).insert(repository_id.to_string(), Arc::clone(&repo));
        Ok(repo)
    }

    /// Forget an open repository, so its row is read again: after the row changed, or a service replaced its storage.
    pub fn refresh(&self, repository_id: &str) {
        moqui_lib::sync::write(&self.inner.repos).remove(repository_id);
    }

    fn located(&self, location: &str) -> Result<(Arc<Repo>, Vec<String>), Error> {
        let (repository_id, path) = parse_location(location)?;
        Ok((self.repo(&repository_id)?, path))
    }

    /// The current user as a commit author: the `UserAccount` name and email, else the username, else `moqui`.
    fn author(&self) -> Author {
        let user_id = crate::user::current_user_id().ok().flatten();
        let account = user_id.as_ref().and_then(|user_id| {
            let entity = self.entity().ok()?;
            entity
                .find("moqui.security.UserAccount")
                .ok()?
                .eq("userId", Value::string(user_id))
                .one()
                .ok()
                .flatten()
        });
        let field = |name: &str| account.as_ref().and_then(|row| text(row.get(name)));
        let username = field("username").or_else(|| crate::user::current_username().ok().flatten());
        Author {
            name: field("userFullName")
                .or_else(|| username.clone())
                .unwrap_or_else(|| "moqui".to_string()),
            email: field("emailAddress")
                .unwrap_or_else(|| format!("{}@localhost", username.as_deref().unwrap_or("moqui"))),
            user_id,
        }
    }

    fn writable(repo: &Repo, location: &str) -> Result<(), Error> {
        if repo.definition.kind == RepositoryKind::Remote {
            return Err(Error::new(format!(
                "git repository {} is a read-only mirror: {location}",
                repo.definition.id
            )));
        }
        Ok(())
    }

    /// Commit on the repository's branch, under its lock, and remember the new head on its row.
    fn commit(
        &self,
        repo: &Repo,
        edits: impl FnMut(Option<&ops::CommitInfo>) -> Result<Option<(Vec<Edit>, String)>, Error>,
    ) -> Result<Option<ObjectId>, Error> {
        let _turn = moqui_lib::sync::lock(&repo.lock);
        let made = ops::commit(repo.store.as_ref(), &repo.definition.branch, &self.author(), edits)?;
        if let Some(id) = made {
            self.remember_head(&repo.definition.id, Some(id), None);
        }
        Ok(made)
    }

    /// Set `headCommitId` (and the fetch outcome, when given) on a repository's row. A failure is logged: the commit
    /// or fetch it describes already happened.
    fn remember_head(&self, repository_id: &str, head: Option<ObjectId>, fetch: Option<Option<&str>>) {
        let outcome = (|| -> Result<(), Error> {
            let entity = self.entity()?;
            let mut row = EntityValue::make(&entity, GIT_REPOSITORY)?;
            row.set("repositoryId", Value::string(repository_id));
            row.set(
                "headCommitId",
                head.map_or(Value::Null, |id| Value::string(id.to_string())),
            );
            if let Some(error) = fetch {
                row.set("lastFetchDate", crate::user::now_effective_value());
                row.set("lastFetchError", error.map_or(Value::Null, Value::string));
            }
            row.update(&entity)?;
            Ok(())
        })();
        if let Err(err) = outcome {
            tracing::error!(
                repository = repository_id,
                "cannot record the git repository head: {err}"
            );
        }
    }

    fn head(repo: &Repo) -> Result<Option<ops::CommitInfo>, Error> {
        ops::head(repo.store.as_ref(), &repo.definition.branch)
    }

    /// The commit a version name names: a full id, or a prefix of at least 7 hex digits of one of the file's versions.
    fn commit_named(repo: &Repo, path: &[String], name: &str) -> Result<Option<ObjectId>, Error> {
        let name = name.trim();
        if name.len() == 40 {
            return Ok(ObjectId::from_hex(name.as_bytes()).ok());
        }
        if name.len() < 7 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(None);
        }
        let Some(head) = Self::head(repo)? else {
            return Ok(None);
        };
        let versions = ops::history(repo.store.as_ref(), &head.id, path)?;
        let mut found = versions.iter().filter(|version| version.version_name.starts_with(name));
        match (found.next(), found.next()) {
            (Some(version), None) => Ok(ObjectId::from_hex(version.version_name.as_bytes()).ok()),
            _ => Ok(None),
        }
    }

    fn history(&self, location: &str) -> Result<Vec<ResourceVersion>, Error> {
        let (repo, path) = self.located(location)?;
        match Self::head(&repo)? {
            Some(head) => ops::history(repo.store.as_ref(), &head.id, &path),
            None => Ok(Vec::new()),
        }
    }

    // ---- operations of org.moqui.impl.GitServices ----

    /// Fetch a `remote` repository's branch into its mirror (created on the first fetch), and record the outcome on
    /// its row. The new head.
    pub fn fetch(&self, repository_id: &str) -> Result<Option<String>, Error> {
        let definition = self.definition(repository_id)?;
        if definition.kind != RepositoryKind::Remote {
            return Err(Error::new(format!(
                "git repository {repository_id} is not a remote mirror; import#GitRepository copies from another repository"
            )));
        }
        let url = definition
            .remote_url
            .clone()
            .ok_or_else(|| Error::new(format!("git repository {repository_id} has no remoteUrl")))?;
        if self.entity()?.sim_active() {
            return Err(Error::new(format!(
                "fetch of git repository {repository_id} denied because sim mode is active"
            )));
        }
        let outcome = {
            let repo = self.repo(repository_id).ok();
            let _turn = repo.as_ref().map(|repo| moqui_lib::sync::lock(&repo.lock));
            fs::fetch(&definition.path, &url, &definition.branch, &definition.credentials)
        };
        self.refresh(repository_id);
        match outcome {
            Ok(head) => {
                self.remember_head(repository_id, head, Some(None));
                Ok(head.map(|id| id.to_string()))
            }
            Err(err) => {
                let current = fs::FsStore::open(&definition.path)
                    .ok()
                    .and_then(|store| store.read_ref(&ops::branch_ref(&definition.branch)).ok().flatten());
                self.remember_head(repository_id, current, Some(Some(err.message())));
                Err(err)
            }
        }
    }

    /// Fetch every `remote` repository whose `fetchIntervalMinutes` has passed since its last fetch. A failing
    /// repository is recorded on its row and logged, and the others are still fetched. (fetched, failed) ids.
    pub fn fetch_due(&self) -> Result<(Vec<String>, Vec<String>), Error> {
        let entity = self.entity()?;
        let rows = entity
            .find(GIT_REPOSITORY)?
            .eq("repositoryKind", Value::string("remote"))
            .list()?;
        let now = jiff::Timestamp::now().as_millisecond();
        let (mut fetched, mut failed) = (Vec::new(), Vec::new());
        for row in rows.iter() {
            let Some(id) = text(row.get("repositoryId")) else {
                continue;
            };
            let Some(interval) = long(row.get("fetchIntervalMinutes")).filter(|minutes| *minutes > 0) else {
                continue;
            };
            if millis(row.get("lastFetchDate")).is_some_and(|last| last + interval * 60_000 > now) {
                continue;
            }
            match self.fetch(&id) {
                Ok(_) => fetched.push(id),
                Err(err) => {
                    tracing::error!(repository = %id, "git fetch failed: {err}");
                    failed.push(id);
                }
            }
        }
        Ok((fetched, failed))
    }

    /// Make a repository ready: create the bare repository of a `local` kind, fetch a `remote` one. A `db` repository
    /// needs nothing. What was done, for the message.
    pub fn init(&self, repository_id: &str) -> Result<String, Error> {
        let definition = self.definition(repository_id)?;
        let done = match definition.kind {
            RepositoryKind::Local => {
                let existed = definition.path.join("HEAD").is_file();
                fs::FsStore::init(&definition.path, &definition.branch)?;
                if existed {
                    format!(
                        "git repository {repository_id} is already at {}",
                        definition.path.display()
                    )
                } else {
                    format!(
                        "created bare git repository {repository_id} at {}",
                        definition.path.display()
                    )
                }
            }
            RepositoryKind::Remote => {
                let head = self.fetch(repository_id)?;
                format!(
                    "fetched git repository {repository_id} at {}",
                    head.unwrap_or_else(|| "an empty branch".into())
                )
            }
            RepositoryKind::Db => format!("git repository {repository_id} is kept in the database; nothing to create"),
        };
        self.refresh(repository_id);
        Ok(done)
    }

    /// Write a `db` repository as a bare repository on disk (at `target`, or its `localPath`), with its branch and
    /// `HEAD`, so it can be cloned and pushed with the git command line. (path, objects written).
    pub fn export(&self, repository_id: &str, target: Option<&str>) -> Result<(PathBuf, usize), Error> {
        let definition = self.definition(repository_id)?;
        if definition.kind != RepositoryKind::Db {
            return Err(Error::new(format!(
                "git repository {repository_id} is already on disk at {}",
                definition.path.display()
            )));
        }
        let path = match target.map(str::trim).filter(|target| !target.is_empty()) {
            Some(target) if Path::new(target).is_absolute() => PathBuf::from(target),
            Some(target) => self.inner.runtime_dir.join(target),
            None => definition.path.clone(),
        };
        let repo = self.repo(repository_id)?;
        let _turn = moqui_lib::sync::lock(&repo.lock);
        let target = fs::FsStore::init(&path, &definition.branch)?;
        let Some(head) = Self::head(&repo)? else {
            return Ok((path, 0));
        };
        let mut seen = HashSet::new();
        let count = ops::reachable(repo.store.as_ref(), &head.id, &mut seen, |id, kind, data| {
            let written = target.write(kind, data)?;
            if written != *id {
                return Err(Error::new(format!("object {id} was written as {written}")));
            }
            Ok(())
        })?;
        let name = ops::branch_ref(&definition.branch);
        let current = target.read_ref(&name)?;
        if !target.compare_and_set_ref(&name, current, head.id)? {
            return Err(Error::new(format!(
                "branch {} of {} moved during the export",
                definition.branch,
                path.display()
            )));
        }
        fs::set_head(&path, &definition.branch)?;
        Ok((path, count))
    }

    /// Copy a repository's branch into a `db` repository: from `source` (a `git://` repository id, a bare or
    /// non-bare repository directory, or a URL fetched into a scratch directory under `runtime/tmp`), else from the
    /// repository's `remoteUrl`. The branch is replaced. (new head, objects copied).
    pub fn import(&self, repository_id: &str, source: Option<&str>) -> Result<(String, usize), Error> {
        let definition = self.definition(repository_id)?;
        if definition.kind != RepositoryKind::Db {
            return Err(Error::new(format!(
                "import is for db repositories; {repository_id} is not one"
            )));
        }
        let source = source
            .map(str::trim)
            .filter(|source| !source.is_empty())
            .map(str::to_string)
            .or_else(|| definition.remote_url.clone())
            .ok_or_else(|| Error::new(format!("no source to import git repository {repository_id} from")))?;
        let mut scratch = None;
        let (from, branch): (Box<dyn GitStore>, String) = if let Some(other) = source.strip_prefix("git://") {
            let other = other.trim_matches('/');
            if other == repository_id {
                return Err(Error::new("a repository cannot be imported into itself"));
            }
            let other = self.definition(other)?;
            let store: Box<dyn GitStore> = match other.kind {
                RepositoryKind::Db => Box::new(db::DbStore::new(self.entity()?, &other.id)),
                _ => Box::new(fs::FsStore::open(&other.path)?),
            };
            (store, other.branch)
        } else if Path::new(&source).is_dir() {
            let path = Path::new(&source);
            let path = if path.join(".git").is_dir() {
                path.join(".git")
            } else {
                path.to_path_buf()
            };
            (Box::new(fs::FsStore::open(&path)?), definition.branch.clone())
        } else {
            if self.entity()?.sim_active() {
                return Err(Error::new(format!(
                    "import of {} denied because sim mode is active",
                    crate::outbound::redacted(&source)
                )));
            }
            let path = self.inner.runtime_dir.join("tmp").join(format!(
                "git-import-{repository_id}-{}",
                jiff::Timestamp::now().as_millisecond()
            ));
            fs::fetch(&path, &source, &definition.branch, &definition.credentials)?;
            scratch = Some(path.clone());
            (Box::new(fs::FsStore::open(&path)?), definition.branch.clone())
        };
        let outcome = self.copy_branch(repository_id, &definition.branch, from.as_ref(), &branch);
        if let Some(path) = scratch {
            if let Err(err) = std::fs::remove_dir_all(&path) {
                tracing::warn!("cannot remove {}: {err}", path.display());
            }
        }
        outcome
    }

    fn copy_branch(
        &self,
        repository_id: &str,
        branch: &str,
        from: &dyn GitStore,
        from_branch: &str,
    ) -> Result<(String, usize), Error> {
        let head = from
            .read_ref(&ops::branch_ref(from_branch))?
            .ok_or_else(|| Error::new(format!("the source has no branch {from_branch}")))?;
        let repo = self.repo(repository_id)?;
        let _turn = moqui_lib::sync::lock(&repo.lock);
        let mut seen = HashSet::new();
        let count = ops::reachable(from, &head, &mut seen, |id, kind, data| {
            let written = repo.store.write(kind, data)?;
            if written != *id {
                return Err(Error::new(format!("object {id} was written as {written}")));
            }
            Ok(())
        })?;
        let name = ops::branch_ref(branch);
        let current = repo.store.read_ref(&name)?;
        if !repo.store.compare_and_set_ref(&name, current, head)? {
            return Err(Error::new(format!(
                "branch {branch} of {repository_id} moved during the import"
            )));
        }
        drop(_turn);
        self.remember_head(repository_id, Some(head), None);
        Ok((head.to_string(), count))
    }
}

fn join(location: &str, name: &str) -> String {
    format!("{}/{name}", location.trim_end_matches('/'))
}

impl crate::native::ResourceProvider for GitProvider {
    fn read_bytes(&self, location: &str) -> Result<Vec<u8>, Error> {
        let (repo, path) = self.located(location)?;
        let not_found = || Error::new(format!("resource not found: {location}"));
        let head = Self::head(&repo)?.ok_or_else(not_found)?;
        match ops::resolve(repo.store.as_ref(), &head.tree, &path)? {
            Some((mode, blob)) if !mode.is_tree() => ops::object(repo.store.as_ref(), &blob, Kind::Blob),
            _ => Err(not_found()),
        }
    }

    fn stat(&self, location: &str) -> Result<Option<ResourceStat>, Error> {
        let (repo, path) = self.located(location)?;
        let Some(head) = Self::head(&repo)? else {
            // an empty branch still has its root directory
            return Ok(path.is_empty().then_some(ResourceStat {
                is_file: false,
                size: 0,
                last_modified: None,
            }));
        };
        let last_modified = Some(head.seconds.saturating_mul(1000));
        Ok(match ops::resolve(repo.store.as_ref(), &head.tree, &path)? {
            Some((mode, _)) if mode.is_tree() => Some(ResourceStat {
                is_file: false,
                size: 0,
                last_modified,
            }),
            Some((_, blob)) => Some(ResourceStat {
                is_file: true,
                size: repo.store.object_size(&blob)?.unwrap_or(0),
                last_modified,
            }),
            None => None,
        })
    }

    fn list(&self, location: &str) -> Result<Vec<ResourceEntry>, Error> {
        let (repo, path) = self.located(location)?;
        let Some(head) = Self::head(&repo)? else {
            return if path.is_empty() {
                Ok(Vec::new())
            } else {
                Err(Error::new(format!("not a directory: {location}")))
            };
        };
        match ops::resolve(repo.store.as_ref(), &head.tree, &path)? {
            Some((mode, tree)) if mode.is_tree() => Ok(ops::list(repo.store.as_ref(), &tree)?
                .into_iter()
                .map(|entry| ResourceEntry {
                    location: join(location, &entry.name),
                    name: entry.name,
                    is_file: entry.is_file,
                })
                .collect()),
            _ => Err(Error::new(format!("not a directory: {location}"))),
        }
    }

    fn write_bytes(&self, location: &str, bytes: &[u8]) -> Result<(), Error> {
        let (repo, path) = self.located(location)?;
        Self::writable(&repo, location)?;
        if path.is_empty() {
            return Err(Error::new(format!("cannot write the root of a repository: {location}")));
        }
        let blob = repo.store.write(Kind::Blob, bytes)?;
        let shown = path.join("/");
        self.commit(&repo, |head| {
            let existing = match head {
                Some(head) => ops::resolve(repo.store.as_ref(), &head.tree, &path)?,
                None => None,
            };
            let kind = match existing {
                Some((mode, _)) if mode.is_tree() => {
                    return Err(Error::new(format!("is a directory: {location}")));
                }
                Some((_, id)) if id == blob => return Ok(None),
                Some((mode, _)) if mode.is_executable() => EntryKind::BlobExecutable,
                _ => EntryKind::Blob,
            };
            let verb = if existing.is_some() { "Update" } else { "Create" };
            Ok(Some((
                vec![Edit::Put(path.clone(), kind, blob)],
                format!("{verb} {shown}"),
            )))
        })?;
        Ok(())
    }

    /// Git keeps no empty directories, so a new directory gets an empty `.gitkeep` file, the usual convention. A
    /// directory already there is left alone; a file there is an error.
    fn make_directory(&self, location: &str) -> Result<(), Error> {
        let (repo, path) = self.located(location)?;
        Self::writable(&repo, location)?;
        if path.is_empty() {
            return Ok(());
        }
        let blob = repo.store.write(Kind::Blob, b"")?;
        let mut keep = path.clone();
        keep.push(".gitkeep".to_string());
        let shown = path.join("/");
        self.commit(&repo, |head| {
            let existing = match head {
                Some(head) => ops::resolve(repo.store.as_ref(), &head.tree, &path)?,
                None => None,
            };
            match existing {
                Some((mode, _)) if mode.is_tree() => Ok(None),
                Some(_) => Err(Error::new(format!("found file instead of directory: {location}"))),
                None => Ok(Some((
                    vec![Edit::Put(keep.clone(), EntryKind::Blob, blob)],
                    format!("Create directory {shown}"),
                ))),
            }
        })?;
        Ok(())
    }

    fn delete(&self, location: &str) -> Result<bool, Error> {
        let (repo, path) = self.located(location)?;
        Self::writable(&repo, location)?;
        if path.is_empty() {
            return Err(Error::new(format!(
                "cannot delete the root of a repository: {location}"
            )));
        }
        let shown = path.join("/");
        let made = self.commit(&repo, |head| {
            let Some(head) = head else { return Ok(None) };
            match ops::resolve(repo.store.as_ref(), &head.tree, &path)? {
                None => Ok(None),
                Some((mode, _)) if mode.is_tree() => Err(Error::new(format!("directory is not empty: {location}"))),
                Some(_) => Ok(Some((vec![Edit::Remove(path.clone())], format!("Delete {shown}")))),
            }
        })?;
        Ok(made.is_some())
    }

    fn rename(&self, location: &str, new_location: &str) -> Result<(), Error> {
        let (repo, path) = self.located(location)?;
        let (other, new_path) = parse_location(new_location)?;
        if other != repo.definition.id {
            return Err(Error::new(format!(
                "cannot move between git repositories: {location} to {new_location}"
            )));
        }
        Self::writable(&repo, location)?;
        if path.is_empty() || new_path.is_empty() {
            return Err(Error::new("cannot move the root of a repository"));
        }
        // a move onto itself is nothing to do, as for a dbresource (update#WikiPage asks for one on every save)
        if path == new_path {
            return Ok(());
        }
        let message = format!("Move {} to {}", path.join("/"), new_path.join("/"));
        let made = self.commit(&repo, |head| {
            let found = match head {
                Some(head) => ops::resolve(repo.store.as_ref(), &head.tree, &path)?,
                None => None,
            };
            let Some((mode, id)) = found else {
                return Err(Error::new(format!("resource not found: {location}")));
            };
            if let Some(head) = head {
                if ops::resolve(repo.store.as_ref(), &head.tree, &new_path)?.is_some() {
                    return Err(Error::new(format!("already exists: {new_location}")));
                }
            }
            Ok(Some((
                vec![Edit::Remove(path.clone()), Edit::Put(new_path.clone(), mode.kind(), id)],
                message.clone(),
            )))
        })?;
        if made.is_none() {
            return Err(Error::new(format!("nothing moved: {location}")));
        }
        Ok(())
    }

    fn supports_version(&self, location: &str) -> bool {
        parse_location(location).is_ok()
    }

    fn version(&self, location: &str, version_name: &str) -> Result<Option<ResourceVersion>, Error> {
        let (repo, path) = self.located(location)?;
        let Some(id) = Self::commit_named(&repo, &path, version_name)? else {
            return Ok(None);
        };
        let id = id.to_string();
        Ok(self
            .history(location)?
            .into_iter()
            .find(|version| version.version_name == id))
    }

    fn current_version(&self, location: &str) -> Result<Option<ResourceVersion>, Error> {
        Ok(self.history(location)?.into_iter().next())
    }

    fn root_version(&self, location: &str) -> Result<Option<ResourceVersion>, Error> {
        Ok(self.history(location)?.pop())
    }

    fn version_history(&self, location: &str) -> Result<Vec<ResourceVersion>, Error> {
        self.history(location)
    }

    fn next_versions(&self, location: &str, version_name: &str) -> Result<Vec<ResourceVersion>, Error> {
        let (repo, path) = self.located(location)?;
        let Some(id) = Self::commit_named(&repo, &path, version_name)? else {
            return Ok(Vec::new());
        };
        let id = id.to_string();
        Ok(self
            .history(location)?
            .into_iter()
            .filter(|version| version.previous_version_name.as_deref() == Some(id.as_str()))
            .collect())
    }

    fn read_version(&self, location: &str, version_name: &str) -> Result<Option<Vec<u8>>, Error> {
        let (repo, path) = self.located(location)?;
        let Some(id) = Self::commit_named(&repo, &path, version_name)? else {
            return Ok(None);
        };
        if repo.store.find(&id)?.is_none() {
            return Ok(None);
        }
        ops::read_at(repo.store.as_ref(), &id, &path)
    }
}
