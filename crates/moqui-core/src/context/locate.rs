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

//! File paths for `classpath://`, `component://`, and `file:` locations.
//!
//! The roots come from the entered factory, or from the last factory that booted when no context
//! is entered, as `moqui lint` runs. With neither, the process working directory is the root, the
//! way a Moqui install runs from its own directory.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::{Error, ExecutionContext};

#[derive(Clone)]
struct Roots {
    framework: PathBuf,
    components: Vec<(String, PathBuf)>,
}

static LAST_BOOTED: RwLock<Option<Roots>> = RwLock::new(None);

pub(super) fn remember(framework: &Path, components: &[super::ComponentInfo]) {
    let roots = Roots {
        framework: framework.to_path_buf(),
        components: components
            .iter()
            .map(|component| (component.name.clone(), component.location.clone()))
            .collect(),
    };
    if let Ok(mut slot) = LAST_BOOTED.write() {
        *slot = Some(roots);
    }
}

fn roots() -> Roots {
    let entered = ExecutionContext::with_current(|ec| Roots {
        framework: ec.factory().framework_dir().to_path_buf(),
        components: ec
            .factory()
            .components()
            .iter()
            .map(|component| (component.name.clone(), component.location.clone()))
            .collect(),
    });
    if let Some(roots) = entered {
        return roots;
    }
    if let Some(roots) = LAST_BOOTED.read().ok().and_then(|slot| slot.clone()) {
        return roots;
    }
    Roots {
        framework: fallback_root().join("framework"),
        components: Vec::new(),
    }
}

#[cfg(test)]
fn fallback_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(not(test))]
fn fallback_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// The framework directory.
pub fn framework_dir() -> PathBuf {
    roots().framework
}

/// A local path for a `classpath://`, `component://`, `file:`, or plain location.
///
/// # Errors
///
/// Returns an error when a `component://` location has no name or names an unknown component.
pub fn locate(location: &str) -> Result<PathBuf, Error> {
    let roots = roots();
    if let Some(rest) = strip_prefix(location, &["classpath://", "classpath:"]) {
        return Ok(roots.framework.join(rest.trim_start_matches('/')));
    }
    if let Some(rest) = strip_prefix(location, &["component://", "component:"]) {
        let rest = rest.trim_start_matches('/');
        let (name, relative) = match rest.split_once('/') {
            Some((name, relative)) => (name, relative.trim_start_matches('/')),
            None => (rest, ""),
        };
        if name.is_empty() {
            return Err(Error::new(format!("component location has no name: {location}")));
        }
        let root = component_root(&roots, name).ok_or_else(|| Error::new(format!("component {name} was not found")))?;
        return Ok(if relative.is_empty() { root } else { root.join(relative) });
    }
    if let Some(rest) = location.strip_prefix("file:") {
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        return Ok(PathBuf::from(rest));
    }
    Ok(PathBuf::from(location))
}

fn component_root(roots: &Roots, name: &str) -> Option<PathBuf> {
    if let Some((_, location)) = roots.components.iter().find(|(known, _)| known == name) {
        return Some(location.clone());
    }
    // Not discovered: look beside the framework, the way an install lays components out.
    let repo = roots.framework.parent()?;
    [repo.join("runtime/component").join(name), repo.join(name)]
        .into_iter()
        .find(|path| path.is_dir())
}

fn strip_prefix<'a>(location: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| location.strip_prefix(prefix))
}
