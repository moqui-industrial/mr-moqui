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

//! `org.moqui.impl.GitServices`: fetch, init, export and import of the repositories behind `git://` locations
//! (`crate::resource::git`). No Java counterpart. A failure is an error in the message facade, as a Java service's is.

use std::collections::BTreeMap;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use crate::context::ExecutionContext;
use crate::entity::EntityFacade;
use crate::resource::git::GitProvider;

pub(in crate::service) fn register(
    map: &mut BTreeMap<&'static str, crate::service::rust_services::RustServiceFn>,
) -> Result<(), String> {
    rust_service!(map, fetch_repository)?;
    rust_service!(map, fetch_all_repositories)?;
    rust_service!(map, init_repository)?;
    rust_service!(map, export_repository)?;
    rust_service!(map, import_repository)?;
    Ok(())
}

fn text(parameters: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    parameters
        .get(key)
        .filter(|value| !is_empty(value))
        .map(to_plain_string)
        .filter(|text| !text.trim().is_empty())
}

fn add_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

fn add_message(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_message(text));
}

/// The provider and the required `repositoryId`, or the error recorded.
fn provider_and_id(parameters: &BTreeMap<String, Value>, need_id: bool) -> Option<(GitProvider, String)> {
    let Some(provider) = ExecutionContext::with_current(|ec| ec.factory().resource().git().cloned()).flatten() else {
        add_error("git resources are not installed (resource-reference scheme=\"git\" in the conf)");
        return None;
    };
    let id = text(parameters, "repositoryId").unwrap_or_default();
    if need_id && id.is_empty() {
        add_error("repositoryId is required");
        return None;
    }
    Some((provider, id))
}

fn out(entries: impl IntoIterator<Item = (&'static str, Value)>) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn count(value: usize) -> Value {
    Value::I64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// `fetch#GitRepository`: fetch a remote mirror's branch. Out: `headCommitId`.
fn fetch_repository(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = provider_and_id(parameters, true) else {
        return Ok(BTreeMap::new());
    };
    match provider.fetch(&id) {
        Ok(head) => Ok(out([("headCommitId", head.map_or(Value::Null, Value::string))])),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// `fetch#AllGitRepositories`: fetch every remote mirror whose interval has passed. A failure is recorded on that
/// repository's row and logged; the job goes on with the rest. Out: `fetchedIds`, `failedIds`.
fn fetch_all_repositories(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, _)) = provider_and_id(parameters, false) else {
        return Ok(BTreeMap::new());
    };
    match provider.fetch_due() {
        Ok((fetched, failed)) => Ok(out([
            (
                "fetchedIds",
                Value::List(fetched.into_iter().map(Value::string).collect()),
            ),
            (
                "failedIds",
                Value::List(failed.into_iter().map(Value::string).collect()),
            ),
        ])),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// `init#GitRepository`: create a local bare repository, or fetch a remote one for the first time.
fn init_repository(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = provider_and_id(parameters, true) else {
        return Ok(BTreeMap::new());
    };
    match provider.init(&id) {
        Ok(done) => add_message(done),
        Err(err) => add_error(err.message()),
    }
    Ok(BTreeMap::new())
}

/// `export#GitRepository`: write a db repository as a bare repository on disk. Out: `exportPath`, `objectCount`.
fn export_repository(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = provider_and_id(parameters, true) else {
        return Ok(BTreeMap::new());
    };
    match provider.export(&id, text(parameters, "targetPath").as_deref()) {
        Ok((path, written)) => {
            let path = path.to_string_lossy().into_owned();
            add_message(format!("Exported git repository {id} to {path} ({written} objects)"));
            Ok(out([
                ("exportPath", Value::string(path)),
                ("objectCount", count(written)),
            ]))
        }
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// `import#GitRepository`: copy a branch into a db repository. Out: `headCommitId`, `objectCount`.
fn import_repository(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = provider_and_id(parameters, true) else {
        return Ok(BTreeMap::new());
    };
    match provider.import(&id, text(parameters, "source").as_deref()) {
        Ok((head, copied)) => {
            add_message(format!(
                "Imported {copied} objects into git repository {id}, head {head}"
            ));
            Ok(out([
                ("headCommitId", Value::string(head)),
                ("objectCount", count(copied)),
            ]))
        }
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}
