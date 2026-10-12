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

    fn ensure_db_tables(&self) -> Result<(), Error> {
        if self.inner.db_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        self.check_tables(&[SEQUENCE, DB_RESOURCE, DB_FILE])?;
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
