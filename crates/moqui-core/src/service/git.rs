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

use indexmap::IndexMap;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use crate::context::ExecutionContext;
use crate::entity::EntityFacade;
use crate::resource::git::browse::{short_name, RefInfo};
use crate::resource::git::diff::{FileChange, FileDiff};
use crate::resource::git::ops::{CommitDetails, Signature, TagDetails};
use crate::resource::git::{GitProvider, GIT_ARTIFACT_TYPE};

pub(in crate::service) fn register(
    map: &mut BTreeMap<&'static str, crate::service::rust_services::RustServiceFn>,
) -> Result<(), String> {
    rust_service!(map, fetch_repository)?;
    rust_service!(map, fetch_all_repositories)?;
    rust_service!(map, init_repository)?;
    rust_service!(map, export_repository)?;
    rust_service!(map, import_repository)?;
    rust_service!(map, get_refs)?;
    rust_service!(map, get_log)?;
    rust_service!(map, get_commit)?;
    rust_service!(map, get_commit_diff)?;
    rust_service!(map, get_blob)?;
    rust_service!(map, get_clone_urls)?;
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

// ---- reading a repository: refs, log, commits, diffs, files ----

/// The provider and repository id when the current user may `view` it; otherwise the error recorded.
fn viewable(parameters: &BTreeMap<String, Value>) -> Option<(GitProvider, String)> {
    let (provider, id) = provider_and_id(parameters, true)?;
    match crate::user::authorize(GIT_ARTIFACT_TYPE, &id, "view") {
        Ok(()) => Some((provider, id)),
        Err(err) => {
            add_error(err.message());
            None
        }
    }
}

fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn opt_text(text: &str) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        Value::string(text)
    }
}

fn timestamp(seconds: i64) -> Value {
    jiff::Timestamp::from_second(seconds).map_or(Value::Null, Value::Timestamp)
}

fn signature_into(entries: &mut Vec<(&'static str, Value)>, names: [&'static str; 3], signature: &Signature) {
    entries.push((names[0], opt_text(&signature.name)));
    entries.push((names[1], opt_text(&signature.email)));
    entries.push((names[2], timestamp(signature.seconds)));
}

/// A commit as a map: ids, people and dates, the message, its summary and body, its trailers by name (each a list of
/// values), and the Moqui user of a commit made through `git://`.
fn commit_value(commit: &CommitDetails) -> Value {
    let id = commit.id.to_string();
    let summary = commit.summary().to_string();
    let body = commit
        .message
        .trim()
        .split_once('\n')
        .map_or("", |(_, rest)| rest)
        .trim()
        .to_string();
    let mut trailers: IndexMap<String, Value> = IndexMap::new();
    for (name, value) in commit.trailers() {
        if let Value::List(values) = trailers.entry(name).or_insert_with(|| Value::List(Vec::new())) {
            values.push(Value::string(value));
        }
    }
    let mut entries = vec![
        ("commitId", Value::string(&id)),
        ("shortId", Value::string(&id[..10])),
        ("treeId", Value::string(commit.tree.to_string())),
        (
            "parentIds",
            Value::List(commit.parents.iter().map(|parent| Value::string(parent.to_string())).collect()),
        ),
    ];
    signature_into(&mut entries, ["authorName", "authorEmail", "authorDate"], &commit.author);
    signature_into(&mut entries, ["committerName", "committerEmail", "committerDate"], &commit.committer);
    entries.extend([
        ("summary", Value::string(summary)),
        ("body", opt_text(&body)),
        ("message", Value::string(&commit.message)),
        ("trailers", Value::Map(trailers)),
        (
            "moquiUserId",
            crate::resource::git::ops::user_of(&commit.message).map_or(Value::Null, Value::string),
        ),
    ]);
    map(entries)
}

fn tag_value(tag: &TagDetails) -> Value {
    let mut entries = vec![
        ("tagName", Value::string(&tag.name)),
        ("targetId", Value::string(tag.target.to_string())),
        ("targetType", Value::string(tag.target_kind.to_string())),
        ("message", Value::string(tag.message.trim_end())),
    ];
    if let Some(tagger) = &tag.tagger {
        signature_into(&mut entries, ["taggerName", "taggerEmail", "taggerDate"], tagger);
    }
    map(entries)
}

fn ref_value(info: &RefInfo, branch: &str) -> Value {
    let commit = info.commit.as_ref();
    map([
        ("refName", Value::string(&info.name)),
        ("shortName", Value::string(short_name(&info.name))),
        ("refType", Value::string(info.kind.name())),
        ("objectId", Value::string(info.target.to_string())),
        ("isDefault", Value::Bool(info.name == format!("refs/heads/{branch}"))),
        ("annotated", Value::Bool(info.tag.is_some())),
        ("commitId", commit.map_or(Value::Null, |commit| Value::string(commit.id.to_string()))),
        ("summary", commit.map_or(Value::Null, |commit| Value::string(commit.summary()))),
        ("commitDate", commit.map_or(Value::Null, |commit| timestamp(commit.committer.seconds))),
        ("authorName", commit.map_or(Value::Null, |commit| opt_text(&commit.author.name))),
        ("tag", info.tag.as_ref().map_or(Value::Null, tag_value)),
    ])
}

fn change_value(change: &FileChange, diff: Option<&FileDiff>) -> Value {
    let mut entries = vec![
        ("path", Value::string(&change.path)),
        ("changeType", Value::string(change.kind.name())),
        ("oldObjectId", change.old.map_or(Value::Null, |(_, id)| Value::string(id.to_string()))),
        ("newObjectId", change.new.map_or(Value::Null, |(_, id)| Value::string(id.to_string()))),
    ];
    if let Some(diff) = diff {
        let stats = diff.diff.as_ref().map(|diff| diff.stats);
        entries.extend([
            ("binary", Value::Bool(diff.binary)),
            ("tooLarge", Value::Bool(diff.too_large)),
            ("insertions", stats.map_or(Value::Null, |stats| count(stats.insertions))),
            ("deletions", stats.map_or(Value::Null, |stats| count(stats.deletions))),
        ]);
    }
    map(entries)
}

fn number(parameters: &BTreeMap<String, Value>, key: &str, default: usize) -> usize {
    text(parameters, key)
        .and_then(|text| text.parse().ok())
        .unwrap_or(default)
}

/// A path parameter as segments; empty for none.
fn path_segments(parameters: &BTreeMap<String, Value>, key: &str) -> Vec<String> {
    text(parameters, key)
        .map(|path| {
            path.split('/')
                .filter(|segment| !segment.is_empty() && *segment != ".")
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `get#GitRefs`: the branches and tags. Out: `defaultBranch`, `headCommitId`, `refList`.
fn get_refs(parameters: &BTreeMap<String, Value>, _facade: &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = viewable(parameters) else {
        return Ok(BTreeMap::new());
    };
    let prefix = match text(parameters, "refType").as_deref() {
        Some("branch") => "refs/heads/",
        Some("tag") => "refs/tags/",
        _ => "refs/",
    };
    match provider.refs(&id, prefix) {
        Ok(refs) => Ok(out([
            ("defaultBranch", Value::string(&refs.branch)),
            ("headCommitId", refs.head.map_or(Value::Null, |head| Value::string(head.to_string()))),
            (
                "refList",
                Value::List(refs.refs.iter().map(|info| ref_value(info, &refs.branch)).collect()),
            ),
        ])),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// `get#GitLog`: commits newest first from a revision, optionally only those that changed a path. Out: `commitList`,
/// `hasMore` (pass the last commit's id as `afterCommitId` for the next page).
fn get_log(parameters: &BTreeMap<String, Value>, _facade: &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = viewable(parameters) else {
        return Ok(BTreeMap::new());
    };
    let revision = text(parameters, "revision").unwrap_or_default();
    let after = text(parameters, "afterCommitId");
    let limit = number(parameters, "limit", 50);
    match provider.log(&id, &revision, &path_segments(parameters, "path"), after.as_deref(), limit) {
        Ok(page) => Ok(out([
            ("commitList", Value::List(page.commits.iter().map(commit_value).collect())),
            ("hasMore", Value::Bool(page.more)),
        ])),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// `get#GitCommit`: a commit and the files it changed against its first parent, with line counts. Out: `commit`,
/// `fileList`, `insertions`, `deletions`; no `commit` when the revision names no commit.
fn get_commit(parameters: &BTreeMap<String, Value>, _facade: &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = viewable(parameters) else {
        return Ok(BTreeMap::new());
    };
    let revision = text(parameters, "commitId").unwrap_or_default();
    let with_stats = text(parameters, "stats").is_none_or(|stats| stats != "false" && stats != "N");
    let outcome = provider.commit_changes(&id, &revision).and_then(|found| {
        let Some((commit, changes)) = found else {
            return Ok(None);
        };
        let diffs = if with_stats {
            provider.commit_diffs(&id, &commit.id.to_string(), None)?.unwrap_or_default()
        } else {
            changes.into_iter().map(|change| (change, empty_diff())).collect()
        };
        Ok(Some((commit, diffs)))
    });
    match outcome {
        Ok(Some((commit, diffs))) => {
            let (mut insertions, mut deletions) = (0, 0);
            for (_, diff) in &diffs {
                if let Some(diff) = &diff.diff {
                    insertions += diff.stats.insertions;
                    deletions += diff.stats.deletions;
                }
            }
            Ok(out([
                ("commit", commit_value(&commit)),
                (
                    "fileList",
                    Value::List(
                        diffs
                            .iter()
                            .map(|(change, diff)| change_value(change, with_stats.then_some(diff)))
                            .collect(),
                    ),
                ),
                ("insertions", count(insertions)),
                ("deletions", count(deletions)),
            ]))
        }
        Ok(None) => Ok(BTreeMap::new()),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

fn empty_diff() -> FileDiff {
    FileDiff {
        binary: false,
        too_large: false,
        diff: None,
    }
}

/// `get#GitCommitDiff`: the line diff of each file a commit changed (or of the one at `path`): the file's change with
/// `unifiedDiff` and `html` (the escaped `moqui-diff` table) when it is text under the size limit. Out:
/// `fileDiffList`.
fn get_commit_diff(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = viewable(parameters) else {
        return Ok(BTreeMap::new());
    };
    let revision = text(parameters, "commitId").unwrap_or_default();
    let path = text(parameters, "path");
    match provider.commit_diffs(&id, &revision, path.as_deref()) {
        Ok(Some(diffs)) => Ok(out([(
            "fileDiffList",
            Value::List(
                diffs
                    .iter()
                    .map(|(change, diff)| {
                        let mut value = change_value(change, Some(diff));
                        if let (Value::Map(entries), Some(text)) = (&mut value, &diff.diff) {
                            entries.insert("unifiedDiff".to_string(), Value::string(text.unified()));
                            entries.insert("html".to_string(), Value::string(text.render_html()));
                        }
                        value
                    })
                    .collect(),
            ),
        )])),
        Ok(None) => Ok(BTreeMap::new()),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// `get#GitBlob`: the file at a path in a revision. Out: `size`, `binary`, and `text` for a UTF-8 file without NUL
/// bytes (else `bytes`); nothing when it is not a file there.
fn get_blob(parameters: &BTreeMap<String, Value>, _facade: &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((provider, id)) = viewable(parameters) else {
        return Ok(BTreeMap::new());
    };
    let revision = text(parameters, "revision").unwrap_or_default();
    match provider.blob_at(&id, &revision, &path_segments(parameters, "path")) {
        Ok(Some(bytes)) => {
            let size = count(bytes.len());
            let binary = bytes.iter().take(8000).any(|byte| *byte == 0);
            let content = match (binary, String::from_utf8(bytes)) {
                (false, Ok(text)) => ("text", Value::string(text)),
                (_, Ok(text)) => ("bytes", Value::Bytes(text.into_bytes())),
                (_, Err(err)) => ("bytes", Value::Bytes(err.into_bytes())),
            };
            let binary = content.0 == "bytes";
            Ok(out([("size", size), ("binary", Value::Bool(binary)), content]))
        }
        Ok(None) => Ok(BTreeMap::new()),
        Err(err) => {
            add_error(err.message());
            Ok(BTreeMap::new())
        }
    }
}

/// The base URL git clients reach the web server at: `git_public_url`, else from the webapp host and port
/// properties.
fn public_base_url() -> String {
    let property = |name: &str| moqui_lib::env::get_prop_or_env(name).trim().to_string();
    let configured = property("git_public_url");
    if !configured.is_empty() {
        return configured.trim_end_matches('/').to_string();
    }
    let host = Some(property("webapp_http_host"))
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    let (scheme, port, default) = if property("webapp_https_enabled") == "true" {
        ("https", property("webapp_https_port"), "443")
    } else {
        ("http", property("webapp_http_port"), "80")
    };
    if port.is_empty() || port == default {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

/// `get#GitCloneUrls`: how to clone a repository. Out: `httpsUrl` (`<base>/git/<id>.git`), and `sshUrl` when the SSH
/// server is on (`ssh_port`), with the current user's username, as the SSH server logs in Moqui users.
fn get_clone_urls(
    parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    let Some((_, id)) = viewable(parameters) else {
        return Ok(BTreeMap::new());
    };
    let property = |name: &str| moqui_lib::env::get_prop_or_env(name).trim().to_string();
    let http = format!("{}/git/{id}.git", public_base_url());
    let ssh_port = Some(property("ssh_public_port"))
        .filter(|port| !port.is_empty())
        .unwrap_or_else(|| property("ssh_port"));
    let ssh = (!ssh_port.is_empty() && ssh_port != "0").then(|| {
        let host = [property("ssh_public_host"), property("webapp_http_host"), property("ssh_host")]
            .into_iter()
            .find(|host| !host.is_empty() && host != "0.0.0.0")
            .unwrap_or_else(|| "localhost".to_string());
        let user = crate::user::current_username()
            .ok()
            .flatten()
            .unwrap_or_else(|| "username".to_string());
        if ssh_port == "22" {
            format!("{user}@{host}:{id}.git")
        } else {
            format!("ssh://{user}@{host}:{ssh_port}/{id}.git")
        }
    });
    Ok(out([
        ("httpsUrl", Value::string(http)),
        ("sshUrl", ssh.map_or(Value::Null, Value::string)),
    ]))
}
