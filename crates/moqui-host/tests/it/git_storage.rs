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

//! The storage of `db` git repositories on the seeded embedded Postgres, measured: the history of a component and of
//! this repository imported, the bytes the rows take against the pack git writes of the same objects, before and
//! after `gc#GitRepositories` repacks them, and a clone. `cargo test -p moqui-host --test it -- --ignored
//! git_storage --nocapture` prints the numbers (docs/Git-Hosting.md, Storage).

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use moqui_lib::object::{to_plain_string, Value};

const DATA: &str = r#"<entity-facade-xml type="seed">
    <moqui.resource.GitRepository repositoryId="bench-usl" repositoryKind="db" branch="master" description="mr-mantle-usl"/>
    <moqui.resource.GitRepository repositoryId="bench-moqui" repositoryKind="db" branch="master" description="mr-moqui"/>
</entity-facade-xml>"#;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "false")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The size of the pack git writes of every object of `branch`, with its default delta settings.
fn git_pack_size(dir: &Path, branch: &str) -> u64 {
    let script = format!("git rev-list --objects {branch} | git pack-objects --stdout -q | wc -c");
    let output = Command::new("sh").arg("-c").arg(script).current_dir(dir).output().unwrap();
    String::from_utf8_lossy(&output.stdout).trim().parse().unwrap()
}

fn sync(name: &str, parameters: &[(&str, Value)]) -> BTreeMap<String, Value> {
    let parameters: BTreeMap<String, Value> = parameters
        .iter()
        .map(|(name, value)| ((*name).to_string(), value.clone()))
        .collect();
    let result = moqui_core::service::sync(name, &parameters).unwrap_or_default();
    let errors = moqui_core::context::ExecutionContext::with_current(|ctx| {
        let text = ctx.message().errors().join("\n");
        ctx.message().clear();
        text
    })
    .unwrap_or_default();
    assert!(errors.is_empty(), "{name}: {errors}");
    result
}

/// One number a query answers.
fn number(facade: &moqui_core::entity::EntityFacade, sql: &str, params: &[Value]) -> i64 {
    let tx = facade.transactions_for("moqui.resource.GitObject").unwrap();
    let rows = tx.with_connection(|conn| conn.query(sql, params)).unwrap();
    rows.first()
        .and_then(|row| row.columns.first())
        .map(|(_, value)| to_plain_string(value).parse().unwrap_or(0))
        .unwrap_or(0)
}

fn mib(bytes: i64) -> String {
    format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[test]
#[ignore = "slow: imports the histories of a component and of this repository"]
fn storage_of_imported_histories() {
    let gate = crate::common::start_with_data("git-storage", DATA);
    let http = gate.server.address();
    let factory = Arc::clone(gate.server.factory());
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    moqui_core::user::internal_login_user("john.doe").unwrap();
    let facade = factory.entity().unwrap();
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sources = [
        ("bench-usl", repo_root.join("runtime/component/mr-mantle-usl")),
        ("bench-moqui", repo_root.clone()),
    ];
    let work = gate.runtime.join("tmp/git-storage");
    std::fs::create_dir_all(&work).unwrap();
    for (id, source) in sources {
        if !source.join(".git").exists() {
            eprintln!("skipped {id}: {} is not a git repository", source.display());
            continue;
        }
        let objects = git(&source, &["rev-list", "--objects", "master"]).lines().count();
        let git_size = git_pack_size(&source, "master") as i64;
        let started = Instant::now();
        sync(
            "org.moqui.impl.GitServices.import#GitRepository",
            &[("repositoryId", Value::string(id)), ("source", Value::string(source.to_string_lossy()))],
        );
        let imported = started.elapsed();
        let params = [Value::string(id)];
        let rows = number(&facade, "SELECT COUNT(*) FROM git_object WHERE repository_id = ?", &params);
        let content = number(&facade, "SELECT SUM(object_size) FROM git_object WHERE repository_id = ?", &params);
        let stored = number(&facade, "SELECT SUM(stored_bytes) FROM git_object WHERE repository_id = ?", &params);
        let deltas = number(
            &facade,
            "SELECT COUNT(*) FROM git_object WHERE repository_id = ? AND storage = 'delta'",
            &params,
        );
        eprintln!(
            "{id}: {objects} objects, git pack {}; imported in {imported:.1?}: {rows} rows, content {}, stored {} \
             ({deltas} deltas), {:.2}x git",
            mib(git_size),
            mib(content),
            mib(stored),
            stored as f64 / git_size as f64
        );
        let started = Instant::now();
        let gc = sync("org.moqui.impl.GitServices.gc#GitRepositories", &[("repositoryId", Value::string(id))]);
        let collected = started.elapsed();
        let stored = number(&facade, "SELECT SUM(stored_bytes) FROM git_object WHERE repository_id = ?", &params);
        let deltas = number(
            &facade,
            "SELECT COUNT(*) FROM git_object WHERE repository_id = ? AND storage = 'delta'",
            &params,
        );
        eprintln!(
            "{id}: gc in {collected:.1?} ({:?}): stored {} ({deltas} deltas), {:.2}x git",
            gc.get("results"),
            mib(stored),
            stored as f64 / git_size as f64
        );
        let clone = work.join(format!("{id}-clone"));
        let url = format!("http://john.doe:moqui@127.0.0.1:{}/git/{id}.git", http.port());
        let started = Instant::now();
        git(&work, &["-c", "credential.helper=", "clone", "--bare", "-q", &url, &clone.to_string_lossy()]);
        let cloned = started.elapsed();
        let pack: u64 = std::fs::read_dir(clone.join("objects/pack"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
            .map(|entry| entry.metadata().unwrap().len())
            .sum();
        git(&clone, &["fsck", "--strict"]);
        eprintln!("{id}: clone in {cloned:.1?}, pack {}", mib(i64::try_from(pack).unwrap()));
        let started = Instant::now();
        let fsck = sync("org.moqui.impl.GitServices.fsck#GitRepository", &[("repositoryId", Value::string(id))]);
        eprintln!("{id}: fsck in {:.1?}: {:?}", started.elapsed(), fsck.get("checkedCount"));
    }
    let table = number(&facade, "SELECT pg_total_relation_size('git_object')", &[]);
    eprintln!("git_object table with its indexes and TOAST: {}", mib(table));
}
