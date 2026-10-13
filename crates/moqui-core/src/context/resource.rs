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

//! `file`, `runtime`, `component`, `dbresource`, and `http` resources. Other schemes are named errors.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use moqui_lib::resource::{FileResource, Resource};

use super::error::Error;
use crate::entity::{DbResourceInfo, EntityFacade};

/// A resolved resource with its logical location.
///
/// A database resource is a snapshot. Directory children are names only; their bytes are not loaded.
#[derive(Clone, Debug)]
pub struct ResourceRef {
    location: String,
    stored: Stored,
}

#[derive(Clone, Debug)]
enum Stored {
    File(FileResource),
    Db(DbSnap),
    /// Fetched on first use, so `get` itself opens no connection.
    Http(HttpRef),
}

/// An `http` or `https` location. The first read or existence check does the GET, and its outcome is kept.
#[derive(Clone, Debug)]
struct HttpRef {
    url: String,
    /// True when sim mode forbids the call.
    sim: bool,
    fetched: Arc<OnceLock<Result<String, String>>>,
}

impl HttpRef {
    fn fetch(&self) -> &Result<String, String> {
        self.fetched.get_or_init(|| {
            if !moqui_lib::http::sim_allows_http(self.sim, false) {
                return Err(format!(
                    "HTTP GET {} denied because sim mode is active",
                    moqui_lib::http::redacted(&self.url)
                ));
            }
            let client = crate::service::shared_http_client();
            match client.get(self.url.clone()).send(false) {
                Ok(response) if (200..300).contains(&response.status()) => Ok(response.body().to_string()),
                Ok(response) => Err(format!("HTTP GET returned {}", response.status())),
                Err(err) => Err(err.message().to_string()),
            }
        })
    }
}

#[derive(Clone, Debug)]
struct DbSnap {
    exists: bool,
    is_file: bool,
    bytes: Vec<u8>,
    children: Vec<DbSnapChild>,
}

#[derive(Clone, Debug)]
struct DbSnapChild {
    location: String,
    name: String,
    is_file: bool,
}

impl ResourceRef {
    pub fn location(&self) -> &str {
        &self.location
    }

    pub fn exists(&self) -> bool {
        match &self.stored {
            Stored::File(file) => file.exists(),
            Stored::Db(snap) => snap.exists,
            Stored::Http(http) => http.fetch().is_ok(),
        }
    }

    pub fn is_file(&self) -> bool {
        match &self.stored {
            Stored::File(file) => file.is_file(),
            Stored::Db(snap) => snap.exists && snap.is_file,
            Stored::Http(http) => http.fetch().is_ok(),
        }
    }

    pub fn is_directory(&self) -> bool {
        match &self.stored {
            Stored::File(file) => file.is_directory(),
            Stored::Db(snap) => snap.exists && !snap.is_file,
            Stored::Http(_) => false,
        }
    }

    /// Milliseconds since the epoch for a file resource. Other kinds have none, so a cache over them
    /// does not invalidate on change.
    pub fn last_modified(&self) -> Option<i64> {
        match &self.stored {
            Stored::File(file) => file.last_modified().ok(),
            Stored::Db(_) | Stored::Http(_) => None,
        }
    }

    pub fn read_text(&self) -> Result<String, Error> {
        match &self.stored {
            Stored::File(file) => file.read_text().map_err(Error::from),
            Stored::Http(http) => http
                .fetch()
                .clone()
                .map_err(|message| Error::new(format!("{message}: {}", self.location))),
            Stored::Db(snap) => {
                if !snap.exists || !snap.is_file {
                    return Err(Error::new(format!("not a file: {}", self.location)));
                }
                String::from_utf8(snap.bytes.clone())
                    .map_err(|_| Error::new(format!("dbresource is not utf-8: {}", self.location)))
            }
        }
    }

    /// The bytes of the file, whatever they hold.
    pub fn read_bytes(&self) -> Result<Vec<u8>, Error> {
        match &self.stored {
            Stored::File(file) => file.read_bytes().map_err(Error::from),
            Stored::Http(http) => http
                .fetch()
                .clone()
                .map(String::into_bytes)
                .map_err(|message| Error::new(format!("{message}: {}", self.location))),
            Stored::Db(snap) => {
                if !snap.exists || !snap.is_file {
                    return Err(Error::new(format!("not a file: {}", self.location)));
                }
                Ok(snap.bytes.clone())
            }
        }
    }

    /// The size in bytes of a file resource. Other kinds have none.
    pub fn size(&self) -> Option<u64> {
        match &self.stored {
            Stored::File(file) => file.size().ok(),
            Stored::Db(_) | Stored::Http(_) => None,
        }
    }

    /// The path of a file resource, for a check that it stays inside its directory.
    pub fn file_path(&self) -> Option<&std::path::Path> {
        match &self.stored {
            Stored::File(file) => Some(file.path()),
            Stored::Db(_) | Stored::Http(_) => None,
        }
    }

    pub fn children(&self) -> Result<Vec<Self>, Error> {
        match &self.stored {
            Stored::File(file) => {
                let files = file.children().map_err(Error::from)?;
                Ok(files
                    .into_iter()
                    .map(|file| {
                        let name = file.file_name().to_string();
                        let location = join_location(&self.location, &name);
                        Self {
                            location,
                            stored: Stored::File(file),
                        }
                    })
                    .collect())
            }
            Stored::Http(_) => Ok(Vec::new()),
            Stored::Db(snap) => Ok(snap
                .children
                .iter()
                .filter(|child| child.is_file)
                .map(|child| Self {
                    location: child.location.clone(),
                    stored: Stored::Db(DbSnap {
                        exists: true,
                        is_file: true,
                        bytes: Vec::new(),
                        children: Vec::new(),
                    }),
                })
                .collect()),
        }
    }

    pub fn find_child_file(&self, relative: &str) -> Self {
        match &self.stored {
            Stored::File(file) => {
                let file = file.find_child_file(relative);
                let location = if relative.is_empty() {
                    self.location.clone()
                } else {
                    join_location(&self.location, relative)
                };
                Self {
                    location,
                    stored: Stored::File(file),
                }
            }
            Stored::Http(_) => self.clone(),
            Stored::Db(snap) => {
                if relative.is_empty() {
                    return self.clone();
                }
                if let Some(child) = snap
                    .children
                    .iter()
                    .find(|child| child.is_file && child.name == relative)
                {
                    return Self {
                        location: child.location.clone(),
                        stored: Stored::Db(DbSnap {
                            exists: true,
                            is_file: true,
                            bytes: Vec::new(),
                            children: Vec::new(),
                        }),
                    };
                }
                Self {
                    location: join_location(&self.location, relative),
                    stored: Stored::Db(DbSnap {
                        exists: false,
                        is_file: false,
                        bytes: Vec::new(),
                        children: Vec::new(),
                    }),
                }
            }
        }
    }
}

pub struct ResourceFacade {
    runtime_dir: PathBuf,
    components: HashMap<String, PathBuf>,
    /// Shared with the factory. `None` until the host installs an entity facade.
    entity: Arc<RwLock<Option<EntityFacade>>>,
}

impl ResourceFacade {
    pub(super) fn new(
        runtime_dir: PathBuf,
        components: HashMap<String, PathBuf>,
        entity: Arc<RwLock<Option<EntityFacade>>>,
    ) -> Self {
        Self {
            runtime_dir,
            components,
            entity,
        }
    }

    pub fn get(&self, location: &str) -> Result<ResourceRef, Error> {
        if location.is_empty() {
            return Err(Error::new("resource location is empty"));
        }
        let scheme = scheme_of(location);
        match scheme {
            "content" => Err(Error::new(format!("content resource is not available: {location}"))),
            "http" | "https" => Ok(ResourceRef {
                location: location.to_string(),
                stored: Stored::Http(HttpRef {
                    url: strip_hash(location).to_string(),
                    sim: self.installed().is_some_and(|facade| facade.sim_active()),
                    fetched: Arc::new(OnceLock::new()),
                }),
            }),
            "dbresource" => self.db_ref(location),
            "file" => Ok(self.file_ref(location)),
            "runtime" => Ok(self.runtime_ref(location)),
            "component" => self.component_ref(location),
            other => Err(Error::new(format!(
                "scheme {other} is not supported for location {location}"
            ))),
        }
    }

    /// Write bytes to a `dbresource://` file. `get` does not create a missing row.
    pub fn write_bytes(&self, location: &str, bytes: &[u8]) -> Result<(), Error> {
        if scheme_of(location) != "dbresource" {
            return Err(Error::new(format!("write_bytes supports dbresource only: {location}")));
        }
        let facade = self
            .installed()
            .ok_or_else(|| Error::new("database resource is not available"))?;
        facade.write_db_resource(location, bytes).map_err(Error::from)
    }

    fn installed(&self) -> Option<EntityFacade> {
        self.entity.read().unwrap_or_else(|err| err.into_inner()).clone()
    }

    fn db_ref(&self, location: &str) -> Result<ResourceRef, Error> {
        let Some(facade) = self.installed() else {
            return Err(Error::new("database resource is not available"));
        };
        let info = facade.read_db_resource(location).map_err(Error::from)?;
        if !info.exists {
            return Err(Error::new(format!("resource not found: {location}")));
        }
        Ok(ResourceRef {
            location: location.to_string(),
            stored: Stored::Db(db_snap(location, &info)),
        })
    }

    fn file_ref(&self, location: &str) -> ResourceRef {
        let path = file_path(strip_hash(location));
        ResourceRef {
            location: location.to_string(),
            stored: Stored::File(FileResource::new(path.to_string_lossy())),
        }
    }

    fn runtime_ref(&self, location: &str) -> ResourceRef {
        let relative = strip_prefix(strip_hash(location), &["runtime://", "runtime:"]);
        let relative = relative.trim_start_matches('/');
        let path = if relative.is_empty() {
            self.runtime_dir.clone()
        } else {
            self.runtime_dir.join(relative)
        };
        ResourceRef {
            location: location.to_string(),
            stored: Stored::File(FileResource::new(path.to_string_lossy())),
        }
    }

    fn component_ref(&self, location: &str) -> Result<ResourceRef, Error> {
        let stripped = strip_prefix(strip_hash(location), &["component://", "component:"]);
        let stripped = stripped.trim_start_matches('/');
        if stripped.is_empty() {
            return Err(Error::new(format!("component location has no name: {location}")));
        }
        let (name, relative) = match stripped.split_once('/') {
            Some((name, relative)) => (name, relative.trim_start_matches('/')),
            None => (stripped, ""),
        };
        let Some(root) = self.components.get(name) else {
            return Err(Error::new(format!(
                "component {name} is not registered for location {location}"
            )));
        };
        let path = if relative.is_empty() {
            root.clone()
        } else {
            root.join(relative)
        };
        Ok(ResourceRef {
            location: location.to_string(),
            stored: Stored::File(FileResource::new(path.to_string_lossy())),
        })
    }
}

fn db_snap(location: &str, info: &DbResourceInfo) -> DbSnap {
    DbSnap {
        exists: info.exists,
        is_file: info.is_file,
        bytes: info.bytes.clone(),
        children: info
            .children
            .iter()
            .map(|child| DbSnapChild {
                location: join_location(location, &child.name),
                name: child.name.clone(),
                is_file: child.is_file,
            })
            .collect(),
    }
}

fn scheme_of(location: &str) -> &str {
    match location.find(':') {
        Some(index) if index > 1 && !location[..index].contains('/') => &location[..index],
        _ => "file",
    }
}

fn strip_hash(location: &str) -> &str {
    match location.find('#') {
        Some(index) if index > 0 => &location[..index],
        _ => location,
    }
}

fn strip_prefix<'a>(location: &'a str, prefixes: &[&str]) -> &'a str {
    for prefix in prefixes {
        if let Some(rest) = location.strip_prefix(prefix) {
            return rest;
        }
    }
    location
}

fn file_path(location: &str) -> PathBuf {
    let Some(rest) = location.strip_prefix("file:") else {
        return PathBuf::from(location);
    };
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    if rest.starts_with('/') {
        return PathBuf::from(rest);
    }
    match rest.find('/') {
        Some(index) => PathBuf::from(&rest[index..]),
        None => PathBuf::from(rest),
    }
}

fn join_location(parent: &str, child: &str) -> String {
    let parent = parent.trim_end_matches('/');
    let child = child.trim_start_matches('/');
    if parent.is_empty() {
        child.to_string()
    } else if child.is_empty() {
        parent.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use moqui_lib::error::Error as LibError;
    use moqui_lib::object::Value;

    use super::*;
    use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use crate::entity::EntityGroup;
    use crate::transaction::{PoolConfig, TransactionManager};

    struct NoCancel;

    impl DbCancel for NoCancel {
        fn cancel(&self) -> Result<(), LibError> {
            Ok(())
        }
    }

    struct FakeConn;

    impl DbConnection for FakeConn {
        fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, LibError> {
            Ok(0)
        }

        fn query(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, LibError> {
            Ok(Vec::new())
        }

        fn query_once(&mut self, _sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, LibError> {
            Ok(Vec::new())
        }

        fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), LibError> {
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, LibError> {
            Ok(None)
        }

        fn close_stream(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn stream_open(&self) -> bool {
            false
        }

        fn begin(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn commit(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn release_savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(NoCancel)
        }
    }

    struct FakeDriver;

    impl DbDriver for FakeDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, LibError> {
            Ok(Box::new(FakeConn))
        }
    }

    fn facade() -> ResourceFacade {
        let driver: Arc<dyn DbDriver> = Arc::new(FakeDriver);
        let transactions = TransactionManager::new(
            driver,
            PoolConfig {
                min: 0,
                max: 2,
                reserve: 1,
                acquire_timeout: Duration::from_secs(5),
            },
        )
        .unwrap();
        let entity = EntityFacade::open(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework"),
            "transactional",
            vec![EntityGroup::new("transactional", "postgres", transactions)],
        )
        .unwrap();
        ResourceFacade::new(
            PathBuf::from("/tmp/moqui-resource-test"),
            HashMap::new(),
            Arc::new(RwLock::new(Some(entity))),
        )
    }

    fn empty_facade() -> ResourceFacade {
        ResourceFacade::new(
            PathBuf::from("/tmp/moqui-resource-test"),
            HashMap::new(),
            Arc::new(RwLock::new(None)),
        )
    }

    #[test]
    fn dbresource_without_facade_is_not_available() {
        let err = empty_facade().get("dbresource://notes/hello.txt").unwrap_err();
        assert!(err.to_string().contains("not available"), "{err}");
    }

    #[test]
    fn missing_dbresource_names_the_location() {
        let resources = facade();
        let root = resources.get("dbresource://").unwrap();
        assert!(root.exists());
        assert!(root.is_directory());
        let location = "dbresource://notes/hello.txt";
        let err = resources.get(location).unwrap_err();
        assert!(err.to_string().contains("resource not found"), "{err}");
        assert!(err.to_string().contains(location), "{err}");
    }

    #[test]
    fn db_scheme_stays_unsupported() {
        let err = empty_facade().get("db://notes/hello.txt").unwrap_err();
        assert!(err.to_string().contains("scheme db"), "{err}");
        let err = facade().get("db://notes/hello.txt").unwrap_err();
        assert!(err.to_string().contains("scheme db"), "{err}");
    }
}
