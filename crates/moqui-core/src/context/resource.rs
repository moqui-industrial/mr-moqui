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

//! `file`, `runtime`, and `component` resources. Other schemes are named errors.

use std::collections::HashMap;
use std::path::PathBuf;

use moqui_lib::resource::{FileResource, Resource};

use super::error::Error;

/// A resolved filesystem resource with its logical location.
#[derive(Clone, Debug)]
pub struct ResourceRef {
    location: String,
    file: FileResource,
}

impl ResourceRef {
    pub fn location(&self) -> &str {
        &self.location
    }

    pub fn exists(&self) -> bool {
        self.file.exists()
    }

    pub fn is_file(&self) -> bool {
        self.file.is_file()
    }

    pub fn is_directory(&self) -> bool {
        self.file.is_directory()
    }

    pub fn read_text(&self) -> Result<String, Error> {
        self.file.read_text().map_err(Error::from)
    }

    pub fn children(&self) -> Result<Vec<Self>, Error> {
        let files = self.file.children().map_err(Error::from)?;
        Ok(files
            .into_iter()
            .map(|file| {
                let name = file.file_name().to_string();
                let location = join_location(&self.location, &name);
                Self { location, file }
            })
            .collect())
    }

    pub fn find_child_file(&self, relative: &str) -> Self {
        let file = self.file.find_child_file(relative);
        let location = if relative.is_empty() {
            self.location.clone()
        } else {
            join_location(&self.location, relative)
        };
        Self { location, file }
    }
}

pub struct ResourceFacade {
    runtime_dir: PathBuf,
    components: HashMap<String, PathBuf>,
}

impl ResourceFacade {
    pub(super) fn new(runtime_dir: PathBuf, components: HashMap<String, PathBuf>) -> Self {
        Self {
            runtime_dir,
            components,
        }
    }

    pub fn get(&self, location: &str) -> Result<ResourceRef, Error> {
        if location.is_empty() {
            return Err(Error::new("resource location is empty"));
        }
        let scheme = scheme_of(location);
        match scheme {
            "content" => Err(Error::new(format!("content resource is not available: {location}"))),
            "http" | "https" => Err(Error::new("HTTP resources are not available")),
            "dbresource" => Err(Error::new("database resource is not available")),
            "file" => Ok(self.file_ref(location)),
            "runtime" => Ok(self.runtime_ref(location)),
            "component" => self.component_ref(location),
            other => Err(Error::new(format!(
                "scheme {other} is not supported for location {location}"
            ))),
        }
    }

    fn file_ref(&self, location: &str) -> ResourceRef {
        let path = file_path(strip_hash(location));
        ResourceRef {
            location: location.to_string(),
            file: FileResource::new(path.to_string_lossy()),
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
            file: FileResource::new(path.to_string_lossy()),
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
            file: FileResource::new(path.to_string_lossy()),
        })
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
