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

//! `dbresource://` rows in `moqui.resource.DbResource` and `DbResourceFile`.

use std::sync::atomic::Ordering;

use moqui_db::sql::Condition;
use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, Value};

use super::facade::EntityFacade;
use super::value::EntityValue;

const DB_RESOURCE: &str = "moqui.resource.DbResource";
const DB_FILE: &str = "moqui.resource.DbResourceFile";
const DB_HISTORY: &str = "moqui.resource.DbResourceFileHistory";
const SEQUENCE: &str = "moqui.entity.SequenceValueItem";

/// One `dbresource` node. `exists` is false when a path segment has no row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbResourceInfo {
    pub exists: bool,
    pub is_file: bool,
    pub bytes: Vec<u8>,
    pub children: Vec<DbResourceChild>,
}

/// A child name under a `dbresource` directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbResourceChild {
    pub name: String,
    pub is_file: bool,
}

/// What a file browser shows of a `dbresource` node: its kind, its last update in milliseconds, and its size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbResourceStat {
    pub is_file: bool,
    pub last_updated: Option<i64>,
    pub size: u64,
}

struct Node {
    id: String,
    is_file: bool,
    filename: String,
}

impl EntityFacade {
    /// Read a `dbresource://` location. A missing row returns `exists: false` and does not error.
    pub fn read_db_resource(&self, location: &str) -> Result<DbResourceInfo, Error> {
        self.ensure_db_tables()?;
        let parts = db_parts(location)?;
        if parts.is_empty() {
            return Ok(DbResourceInfo {
                exists: true,
                is_file: false,
                bytes: Vec::new(),
                children: self.list_children(None)?,
            });
        }
        let mut parent: Option<String> = None;
        for (index, segment) in parts.iter().enumerate() {
            let Some(node) = self.find_node(parent.as_deref(), segment)? else {
                return Ok(DbResourceInfo {
                    exists: false,
                    is_file: false,
                    bytes: Vec::new(),
                    children: Vec::new(),
                });
            };
            if index + 1 == parts.len() {
                if node.is_file {
                    return Ok(DbResourceInfo {
                        exists: true,
                        is_file: true,
                        bytes: self.read_bytes(&node.id)?,
                        children: Vec::new(),
                    });
                }
                return Ok(DbResourceInfo {
                    exists: true,
                    is_file: false,
                    bytes: Vec::new(),
                    children: self.list_children(Some(&node.id))?,
                });
            }
            parent = Some(node.id);
        }
        Ok(DbResourceInfo {
            exists: false,
            is_file: false,
            bytes: Vec::new(),
            children: Vec::new(),
        })
    }

    /// Create or replace a `dbresource://` file. Missing parent directories are created.
    pub fn write_db_resource(&self, location: &str, bytes: &[u8]) -> Result<(), Error> {
        self.ensure_db_tables()?;
        let parts = db_parts(location)?;
        if parts.is_empty() {
            return Err(Error::new(format!("cannot write the dbresource root: {location}")));
        }
        let mut parent: Option<String> = None;
        for (index, segment) in parts.iter().enumerate() {
            let last = index + 1 == parts.len();
            let id = self.ensure_node(parent.as_deref(), segment, last)?;
            if last {
                self.write_bytes(&id, bytes)?;
            }
            parent = Some(id);
        }
        Ok(())
    }

    /// The kind, time, and size of a `dbresource://` location, or `None` when a segment has no row. The root is a
    /// directory with no time. A file's time is its `DbResourceFile` update, as `DbResourceReference.getLastModified`.
    pub fn stat_db_resource(&self, location: &str) -> Result<Option<DbResourceStat>, Error> {
        self.ensure_db_tables()?;
        let parts = db_parts(location)?;
        if parts.is_empty() {
            return Ok(Some(DbResourceStat {
                is_file: false,
                last_updated: None,
                size: 0,
            }));
        }
        let Some(node) = self.node_at(&parts)? else {
            return Ok(None);
        };
        let resource = self
            .find(DB_RESOURCE)?
            .eq("resourceId", Value::string(&node.id))
            .one()?;
        let mut last_updated = resource
            .as_ref()
            .and_then(|row| stamp_millis(row.get("lastUpdatedStamp")));
        let mut size = 0;
        if node.is_file {
            if let Some(file) = self.find(DB_FILE)?.eq("resourceId", Value::string(&node.id)).one()? {
                last_updated = stamp_millis(file.get("lastUpdatedStamp")).or(last_updated);
                size = self.read_bytes(&node.id)?.len() as u64;
            }
        }
        Ok(Some(DbResourceStat {
            is_file: node.is_file,
            last_updated,
            size,
        }))
    }

    /// Create the directories of a `dbresource://` location that are missing, as `findDirectoryId(path, true)`. A
    /// file in the path is an error.
    pub fn make_db_directory(&self, location: &str) -> Result<(), Error> {
        self.ensure_db_tables()?;
        let parts = db_parts(location)?;
        self.directory_id(&parts).map(|_| ())
    }

    /// Delete a `dbresource://` node, with its file and history rows. A directory's children are not deleted here,
    /// as in `DbResourceReference.delete`. False when there is no row.
    pub fn delete_db_resource(&self, location: &str) -> Result<bool, Error> {
        self.ensure_db_tables()?;
        let parts = db_parts(location)?;
        if parts.is_empty() {
            return Err(Error::new(format!("cannot delete the dbresource root: {location}")));
        }
        let Some(node) = self.node_at(&parts)? else {
            return Ok(false);
        };
        if node.is_file {
            let histories = self
                .find(DB_HISTORY)?
                .eq("resourceId", Value::string(&node.id))
                .list()?;
            for history in histories.iter() {
                history.delete(self)?;
            }
            if let Some(file) = self.find(DB_FILE)?.eq("resourceId", Value::string(&node.id)).one()? {
                file.delete(self)?;
            }
        }
        let mut value = EntityValue::make(self, DB_RESOURCE)?;
        value.set("resourceId", Value::string(&node.id));
        value.delete(self)?;
        Ok(true)
    }

    /// Give a `dbresource://` node a new parent and name, creating the new parent directories, as
    /// `DbResourceReference.move`. Nothing happens when there is no row at `location`.
    pub fn move_db_resource(&self, location: &str, new_location: &str) -> Result<(), Error> {
        self.ensure_db_tables()?;
        let parts = db_parts(location)?;
        let node = if parts.is_empty() { None } else { self.node_at(&parts)? };
        let Some(node) = node else {
            tracing::warn!("Could not find dbresource at [{location}]");
            return Ok(());
        };
        let mut new_parts = db_parts(new_location)?;
        let Some(new_name) = new_parts.pop() else {
            return Err(Error::new(format!(
                "No location specified, not moving resource at {location}"
            )));
        };
        let parent = self.directory_id(&new_parts)?;
        let mut value = EntityValue::make(self, DB_RESOURCE)?;
        value.set("resourceId", Value::string(&node.id));
        value.set("parentResourceId", parent.map_or(Value::Null, Value::string));
        value.set("filename", Value::string(new_name));
        value.set("isFile", Value::string(if node.is_file { "Y" } else { "N" }));
        value.update(self)?;
        Ok(())
    }

    /// The node at a path, walking from the root.
    fn node_at(&self, parts: &[String]) -> Result<Option<Node>, Error> {
        let mut parent: Option<String> = None;
        let mut found = None;
        for segment in parts {
            let Some(node) = self.find_node(parent.as_deref(), segment)? else {
                return Ok(None);
            };
            parent = Some(node.id.clone());
            found = Some(node);
        }
        Ok(found)
    }

    /// The ID of the directory at a path, creating each missing one. `None` is the root.
    fn directory_id(&self, parts: &[String]) -> Result<Option<String>, Error> {
        let mut parent: Option<String> = None;
        for segment in parts {
            if let Some(node) = self.find_node(parent.as_deref(), segment)? {
                if node.is_file {
                    return Err(Error::new(format!(
                        "Tried to find a directory in a path but found file instead at {segment} under DbResource {}",
                        parent.as_deref().unwrap_or("root")
                    )));
                }
                parent = Some(node.id);
            } else {
                parent = Some(self.ensure_node(parent.as_deref(), segment, false)?);
            }
        }
        Ok(parent)
    }

    /// Skip the table check, for the in-memory test database, which has every table and no `information_schema`.
    #[cfg(test)]
    pub(crate) fn assume_db_tables(&self) {
        self.inner.db_ready.store(true, Ordering::Release);
    }

    fn ensure_db_tables(&self) -> Result<(), Error> {
        if self.inner.db_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        self.check_tables(&[SEQUENCE, DB_RESOURCE, DB_FILE, DB_HISTORY])?;
        self.inner.db_ready.store(true, Ordering::Release);
        Ok(())
    }

    fn find_node(&self, parent: Option<&str>, filename: &str) -> Result<Option<Node>, Error> {
        let mut find = self.find(DB_RESOURCE)?.eq("filename", Value::string(filename));
        find = match parent {
            Some(parent) => find.eq("parentResourceId", Value::string(parent)),
            None => find.condition(Condition::is_null("parentResourceId")),
        };
        let rows = find.list()?;
        let Some(row) = rows.get(0) else {
            return Ok(None);
        };
        let id = match row.get("resourceId").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => return Ok(None),
        };
        let is_file = row
            .get("isFile")
            .and_then(Value::as_str)
            .is_some_and(|flag| flag.eq_ignore_ascii_case("Y"));
        let name = row
            .get("filename")
            .and_then(Value::as_str)
            .unwrap_or(filename)
            .to_string();
        Ok(Some(Node {
            id,
            is_file,
            filename: name,
        }))
    }

    fn list_children(&self, parent: Option<&str>) -> Result<Vec<DbResourceChild>, Error> {
        let find = self.find(DB_RESOURCE)?;
        let find = match parent {
            Some(parent) => find.eq("parentResourceId", Value::string(parent)),
            None => find.condition(Condition::is_null("parentResourceId")),
        };
        let rows = find.list()?;
        let mut children = Vec::new();
        for row in rows.iter() {
            let Some(name) = row.get("filename").and_then(Value::as_str) else {
                continue;
            };
            let is_file = row
                .get("isFile")
                .and_then(Value::as_str)
                .is_some_and(|flag| flag.eq_ignore_ascii_case("Y"));
            children.push(DbResourceChild {
                name: name.to_string(),
                is_file,
            });
        }
        Ok(children)
    }

    fn read_bytes(&self, resource_id: &str) -> Result<Vec<u8>, Error> {
        let row = self.find(DB_FILE)?.eq("resourceId", Value::string(resource_id)).one()?;
        let Some(row) = row else {
            return Ok(Vec::new());
        };
        let raw = row.get("fileData").cloned().unwrap_or(Value::Null);
        Ok(match basic_convert(&raw, "java.sql.Blob").value {
            Value::Bytes(bytes) => bytes,
            Value::String(text) => text.into_bytes(),
            Value::Null => Vec::new(),
            other => {
                return Err(Error::new(format!(
                    "DbResourceFile.fileData for {resource_id} is {other:?}"
                )))
            }
        })
    }

    fn ensure_node(&self, parent: Option<&str>, filename: &str, is_file: bool) -> Result<String, Error> {
        if let Some(node) = self.find_node(parent, filename)? {
            if is_file && !node.is_file {
                let mut value = EntityValue::make(self, DB_RESOURCE)?;
                value.set("resourceId", Value::string(&node.id));
                value.set("filename", Value::string(&node.filename));
                if let Some(parent) = parent {
                    value.set("parentResourceId", Value::string(parent));
                }
                value.set("isFile", Value::string("Y"));
                value.update(self)?;
            }
            return Ok(node.id);
        }
        let mut value = EntityValue::make(self, DB_RESOURCE)?;
        value.set_sequenced_id_primary(self)?;
        let id = value
            .get("resourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("sequenced DbResource id is empty"))?
            .to_string();
        if let Some(parent) = parent {
            value.set("parentResourceId", Value::string(parent));
        }
        value.set("filename", Value::string(filename));
        value.set("isFile", Value::string(if is_file { "Y" } else { "N" }));
        value.create(self)?;
        Ok(id)
    }

    fn write_bytes(&self, resource_id: &str, bytes: &[u8]) -> Result<(), Error> {
        let existing = self.find(DB_FILE)?.eq("resourceId", Value::string(resource_id)).one()?;
        let mut value = EntityValue::make(self, DB_FILE)?;
        value.set("resourceId", Value::string(resource_id));
        value.set("fileData", Value::Bytes(bytes.to_vec()));
        if existing.is_some() {
            value.update(self)?;
        } else {
            value.set("versionName", Value::string("01"));
            value.create(self)?;
        }
        Ok(())
    }
}

fn stamp_millis(value: Option<&Value>) -> Option<i64> {
    match basic_convert(value?, "Timestamp").value {
        Value::Timestamp(stamp) => Some(stamp.as_millisecond()),
        _ => None,
    }
}

fn db_parts(location: &str) -> Result<Vec<String>, Error> {
    let rest = location
        .strip_prefix("dbresource://")
        .or_else(|| location.strip_prefix("dbresource:"))
        .ok_or_else(|| Error::new(format!("not a dbresource location: {location}")))?;
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    Ok(rest
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect())
}
