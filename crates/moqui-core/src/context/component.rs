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

//! Component directories. `depends-on` is one insertion pass.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use moqui_lib::env;
use moqui_lib::node::MNode;

use super::error::Error;

#[derive(Clone, Debug)]
pub struct ComponentInfo {
    pub name: String,
    pub version: String,
    pub location: PathBuf,
    pub depends_on: Vec<String>,
}

pub(super) fn discover(runtime_dir: &Path, framework_dir: &Path, conf: &MNode) -> Result<Vec<ComponentInfo>, Error> {
    let mut found = ComponentSet::default();
    found.insert(component_from_dir(framework_dir, None)?)?;
    if let Some(list) = conf.first_named("component-list") {
        for child in list.children() {
            match child.name().as_str() {
                "component" => found.insert(component_from_element(runtime_dir, &child)?)?,
                "component-dir" => add_component_dir(&mut found, runtime_dir, &child)?,
                _ => {}
            }
        }
    }
    sort_components(found.items)
}

fn add_component_dir(found: &mut ComponentSet, base: &Path, node: &MNode) -> Result<(), Error> {
    let location = node
        .attribute("location")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::new("component-dir location is empty"))?;
    let dir = resolve_location(base, &location);
    if !dir.exists() {
        tracing::info!("Skipping component directory {}, not found", dir.display());
        return Ok(());
    }
    if !dir.is_dir() {
        return Err(Error::new(format!(
            "component directory is not a directory: {}",
            dir.display()
        )));
    }
    let manifest = dir.join("components.xml");
    if manifest.is_file() {
        let list = MNode::parse_file(&manifest)?;
        for child in list.children() {
            match child.name().as_str() {
                "component" => found.insert(component_from_element(&dir, &child)?)?,
                "component-dir" => add_component_dir(found, &dir, &child)?,
                _ => {}
            }
        }
        return Ok(());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|err| Error::new(format!("could not read component directory {}", dir.display())).with_source(err))?
    {
        let entry = entry.map_err(|err| {
            Error::new(format!("could not read component directory {}", dir.display())).with_source(err)
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !path.is_dir() || !path.join("component.xml").is_file() {
            continue;
        }
        entries.push(path);
    }
    entries.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    for path in entries {
        found.insert(component_from_dir(&path, None)?)?;
    }
    Ok(())
}

fn component_from_element(base: &Path, node: &MNode) -> Result<ComponentInfo, Error> {
    let location = node
        .attribute("location")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::new("component location is empty"))?;
    if location.ends_with(".zip") {
        return Err(Error::new(format!("component archives are not expanded: {location}")));
    }
    let path = resolve_location(base, &location);
    component_from_dir(&path, Some(node))
}

fn component_from_dir(path: &Path, fallback: Option<&MNode>) -> Result<ComponentInfo, Error> {
    if !path.exists() {
        return Err(Error::new(format!(
            "Could not find component directory at: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(Error::new(format!(
            "Component location is not a directory: {}",
            path.display()
        )));
    }
    let manifest = path.join("component.xml");
    let parsed = if manifest.is_file() {
        Some(MNode::parse_file(&manifest)?)
    } else {
        None
    };
    let node = parsed.as_ref().or(fallback);
    let name = node
        .and_then(|node| node.attribute("name"))
        .filter(|value| !value.is_empty())
        .or_else(|| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::new(format!("component at {} has no name", path.display())))?;
    let version_attr = node
        .and_then(|node| node.attribute("version"))
        .filter(|value| !value.is_empty());
    let version = match version_attr {
        Some(value) => env::expand(&value)?,
        None => "unknown".to_string(),
    };
    let mut depends_on = Vec::new();
    if let Some(node) = node {
        for child in node.children_named("depends-on") {
            if let Some(dep) = child.attribute("name").filter(|value| !value.is_empty()) {
                depends_on.push(dep);
            }
        }
    }
    Ok(ComponentInfo {
        name,
        version,
        location: path.to_path_buf(),
        depends_on,
    })
}

fn resolve_location(base: &Path, location: &str) -> PathBuf {
    let location = location.trim_end_matches('/');
    let path = Path::new(location);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[derive(Default)]
struct ComponentSet {
    items: Vec<ComponentInfo>,
    index: HashMap<String, usize>,
}

impl ComponentSet {
    fn insert(&mut self, info: ComponentInfo) -> Result<(), Error> {
        if let Some(&slot) = self.index.get(&info.name) {
            tracing::warn!(
                component = %info.name,
                location = %info.location.display(),
                "Overriding component because another component of the same name was initialized"
            );
            self.items[slot] = info;
            return Ok(());
        }
        tracing::info!(component = %info.name, location = %info.location.display(), "Added component");
        self.index.insert(info.name.clone(), self.items.len());
        self.items.push(info);
        Ok(())
    }
}

fn sort_components(items: Vec<ComponentInfo>) -> Result<Vec<ComponentInfo>, Error> {
    let order: Vec<String> = items.iter().map(|item| item.name.clone()).collect();
    let mut by_name: HashMap<String, ComponentInfo> = HashMap::new();
    for item in items {
        by_name.insert(item.name.clone(), item);
    }
    let mut sorted_names = Vec::new();
    let mut stack = Vec::new();
    for name in &order {
        let info = by_name
            .get(name)
            .ok_or_else(|| Error::new(format!("component {name} is missing")))?;
        add_with_dependencies(info, &by_name, &mut sorted_names, &mut stack)?;
    }
    check_dependency_order(&sorted_names, &by_name)?;
    let mut sorted = Vec::with_capacity(sorted_names.len());
    for name in sorted_names {
        let info = by_name
            .remove(&name)
            .ok_or_else(|| Error::new(format!("component {name} is missing")))?;
        sorted.push(info);
    }
    tracing::info!("Components after depends-on sort: {}", names_of(&sorted));
    Ok(sorted)
}

fn names_of(items: &[ComponentInfo]) -> String {
    let mut out = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&item.name);
    }
    out
}

fn add_with_dependencies(
    info: &ComponentInfo,
    by_name: &HashMap<String, ComponentInfo>,
    sorted: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> Result<(), Error> {
    if sorted.iter().any(|name| name == &info.name) {
        return Ok(());
    }
    // A cycle is an error. The Java walk recurses until the stack overflows.
    if stack.iter().any(|name| name == &info.name) {
        return Err(Error::new(format!(
            "component dependency cycle involving {}",
            info.name
        )));
    }
    stack.push(info.name.clone());
    for dep_name in &info.depends_on {
        let dep = by_name.get(dep_name).ok_or_else(|| {
            Error::new(format!(
                "Component {} depends on component {dep_name} which is not initialized",
                info.name
            ))
        })?;
        add_with_dependencies(dep, by_name, sorted, stack)?;
    }
    stack.pop();
    if !sorted.iter().any(|name| name == &info.name) {
        sorted.push(info.name.clone());
    }
    Ok(())
}

fn check_dependency_order(sorted: &[String], by_name: &HashMap<String, ComponentInfo>) -> Result<(), Error> {
    let mut messages = Vec::new();
    for (index, name) in sorted.iter().enumerate() {
        let Some(info) = by_name.get(name) else {
            continue;
        };
        for dep in &info.depends_on {
            if let Some(dep_index) = sorted.iter().position(|item| item == dep) {
                if dep_index > index {
                    messages.push(format!(
                        "Broken dependency order after initial pass: [{dep}] is after [{name}]"
                    ));
                }
            }
        }
    }
    if messages.is_empty() {
        Ok(())
    } else {
        Err(Error::new(messages.join(" ")))
    }
}
