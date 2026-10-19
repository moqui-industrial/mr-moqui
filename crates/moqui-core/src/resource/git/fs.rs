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

//! A bare repository on disk, through gitoxide: the `local` kind (written to) and the `remote` kind (a mirror that
//! only fetch changes). Every gix call for a repository on disk is in this file.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use gix::objs::{Find as _, Kind, Write as _};
use gix::refs::transaction::PreviousValue;
use gix::refs::Target;
use gix::ObjectId;

use super::store::GitStore;
use crate::context::Error;

fn git_error(what: &str, path: &Path, err: impl std::fmt::Display) -> Error {
    Error::new(format!("{what} {}: {err}", path.display()))
}

/// A bare repository directory. `gix::ThreadSafeRepository` is shared; each call makes a thread-local handle.
#[derive(Debug)]
pub(crate) struct FsStore {
    path: PathBuf,
    repo: gix::ThreadSafeRepository,
}

impl FsStore {
    /// Open the bare repository at `path`.
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        let repo =
            gix::ThreadSafeRepository::open(path).map_err(|err| git_error("cannot open git repository", path, err))?;
        Ok(Self {
            path: path.to_path_buf(),
            repo,
        })
    }

    /// Create a bare repository at `path` whose `HEAD` names `branch`, or open the one already there.
    pub(crate) fn init(path: &Path, branch: &str) -> Result<Self, Error> {
        if path.join("HEAD").is_file() {
            return Self::open(path);
        }
        std::fs::create_dir_all(path).map_err(|err| git_error("cannot create", path, err))?;
        gix::init_bare(path).map_err(|err| git_error("cannot create git repository", path, err))?;
        set_head(path, branch)?;
        Self::open(path)
    }

    fn local(&self) -> gix::Repository {
        self.repo.to_thread_local()
    }
}

/// Point `HEAD` of a bare repository at `branch`, so `git clone` of it checks that branch out.
pub(crate) fn set_head(path: &Path, branch: &str) -> Result<(), Error> {
    std::fs::write(path.join("HEAD"), format!("ref: refs/heads/{branch}\n"))
        .map_err(|err| git_error("cannot write HEAD of", path, err))
}

impl GitStore for FsStore {
    fn find(&self, id: &ObjectId) -> Result<Option<(Kind, Vec<u8>)>, Error> {
        let repo = self.local();
        let mut buf = Vec::new();
        match repo.objects.try_find(id, &mut buf) {
            Ok(Some(data)) => {
                let kind = data.kind;
                Ok(Some((kind, buf)))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(git_error(&format!("cannot read object {id} of"), &self.path, err)),
        }
    }

    fn object_size(&self, id: &ObjectId) -> Result<Option<u64>, Error> {
        self.local()
            .try_find_header(*id)
            .map(|header| header.map(|header| header.size()))
            .map_err(|err| git_error(&format!("cannot read object {id} of"), &self.path, err))
    }

    fn write(&self, kind: Kind, data: &[u8]) -> Result<ObjectId, Error> {
        self.local()
            .objects
            .write_buf(kind, data)
            .map_err(|err| git_error("cannot write an object to", &self.path, err))
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>, Error> {
        let repo = self.local();
        let found = repo
            .try_find_reference(name)
            .map_err(|err| git_error(&format!("cannot read ref {name} of"), &self.path, err))?;
        match found {
            Some(mut reference) => reference
                .peel_to_id()
                .map(|id| Some(id.detach()))
                .map_err(|err| git_error(&format!("cannot read ref {name} of"), &self.path, err)),
            None => Ok(None),
        }
    }

    fn compare_and_set_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<bool, Error> {
        let constraint = match expected {
            Some(previous) => PreviousValue::MustExistAndMatch(Target::Object(previous)),
            None => PreviousValue::MustNotExist,
        };
        match self.local().reference(name, new, constraint, "moqui: commit") {
            Ok(_) => Ok(true),
            // a lost race shows as a constraint failure; anything else is an error
            Err(err) => {
                if self.read_ref(name)? != expected {
                    Ok(false)
                } else {
                    Err(git_error(&format!("cannot update ref {name} of"), &self.path, err))
                }
            }
        }
    }
}

/// HTTPS credentials for a fetch.
#[derive(Clone, Debug, Default)]
pub(crate) struct Credentials {
    pub username: Option<String>,
    pub token: Option<String>,
}

/// Fetch `branch` of `url` into the bare repository at `path` (created when it is not there), replacing the local
/// branch with the remote one: a remote repository is a mirror. The new head, or `None` when the remote branch has no
/// commits.
pub(crate) fn fetch(
    path: &Path,
    url: &str,
    branch: &str,
    credentials: &Credentials,
) -> Result<Option<ObjectId>, Error> {
    let store = FsStore::init(path, branch)?;
    let repo = store.local();
    let spec = format!("+refs/heads/{branch}:refs/heads/{branch}");
    let remote = repo
        .remote_at(url)
        .and_then(|remote| remote.with_refspecs([spec.as_str()], gix::remote::Direction::Fetch))
        .map_err(|err| Error::new(format!("bad git remote {}: {err}", crate::outbound::redacted(url))))?;
    let connection = remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|err| Error::new(format!("cannot connect to {}: {err}", crate::outbound::redacted(url))))?;
    let username = credentials.username.clone().unwrap_or_default();
    let token = credentials.token.clone();
    let connection = connection.with_credentials(move |action| match action {
        gix::credentials::helper::Action::Get(context) => {
            Ok(token.clone().map(|password| gix::credentials::protocol::Outcome {
                identity: gix::sec::identity::Account {
                    username: if username.is_empty() {
                        "git".to_string()
                    } else {
                        username.clone()
                    },
                    password,
                    oauth_refresh_token: None,
                },
                next: context.into(),
            }))
        }
        _ => Ok(None),
    });
    let interrupt = AtomicBool::new(false);
    connection
        .prepare_fetch(gix::progress::Discard, gix::remote::ref_map::Options::default())
        .and_then(|prepare| prepare.receive(gix::progress::Discard, &interrupt))
        .map_err(|err| Error::new(format!("fetching {}: {err}", crate::outbound::redacted(url))))?;
    store.read_ref(&super::ops::branch_ref(branch))
}
