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

//! `git://` locations through the resource facade, for each repository kind: the same scenario over a bare
//! repository on disk and over `GitObject`/`GitRef` rows, then export, import and a mirror fetch.

use std::path::{Path, PathBuf};

use moqui_lib::object::Value;

use super::store::GitStore;
use super::{fs, ops};
use crate::context::{ExecutionContext, ExecutionContextFactory, ResourceFacade};
use crate::entity::{EntityFacade, EntityValue};

struct Fixture {
    context: ExecutionContext,
    entity: EntityFacade,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(name: &str) -> Fixture {
    let context = ExecutionContextFactory::test_context();
    let entity = crate::testing::open();
    context.factory().set_entity(entity.clone());
    context.factory().resource().install_git().unwrap();
    let dir = std::env::temp_dir().join(format!("moqui-git-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Fixture { context, entity, dir }
}

fn define(entity: &EntityFacade, id: &str, kind: &str, path: Option<&Path>, remote_url: Option<&str>) {
    let mut row = EntityValue::make(entity, super::GIT_REPOSITORY).unwrap();
    row.set("repositoryId", Value::string(id));
    row.set("repositoryKind", Value::string(kind));
    if let Some(path) = path {
        row.set("localPath", Value::string(path.to_string_lossy()));
    }
    if let Some(url) = remote_url {
        row.set("remoteUrl", Value::string(url));
    }
    row.create(entity).unwrap();
}

fn page(edit: usize) -> String {
    (1..=12)
        .map(|line| {
            if line == edit {
                format!("line {line} edited\n")
            } else {
                format!("line {line}\n")
            }
        })
        .collect()
}

/// Writes, reads, lists, versions, a diff, a move and deletes under `git://<id>/`. The first version's id.
fn scenario(resources: &ResourceFacade, id: &str) -> String {
    let a = format!("git://{id}/notes/a.md");
    let b = format!("git://{id}/notes/b.md");
    let root = format!("git://{id}/");
    assert_eq!(
        resources.stat(&root).unwrap().map(|stat| stat.is_file),
        Some(false),
        "empty root is a directory"
    );
    assert!(resources.list(&root).unwrap().is_empty());
    assert_eq!(resources.stat(&a).unwrap(), None);

    resources.write_bytes(&a, page(0).as_bytes()).unwrap();
    resources.write_bytes(&b, b"other\n").unwrap();
    resources.write_bytes(&a, page(5).as_bytes()).unwrap();
    // the same bytes again make no commit
    resources.write_bytes(&a, page(5).as_bytes()).unwrap();
    assert_eq!(resources.get(&a).unwrap().read_text().unwrap(), page(5));
    let stat = resources.stat(&a).unwrap().expect("a.md");
    assert!(stat.is_file);
    assert_eq!(stat.size, page(5).len() as u64);
    assert!(stat.last_modified.is_some());
    let names: Vec<String> = resources
        .list(&root)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["notes"]);
    let notes: Vec<(String, bool)> = resources
        .list(&format!("git://{id}/notes"))
        .unwrap()
        .into_iter()
        .map(|entry| (entry.location, entry.is_file))
        .collect();
    assert_eq!(notes, [(a.clone(), true), (b.clone(), true)]);

    assert!(resources.supports_version(&a));
    let history = resources.version_history(&a).unwrap();
    assert_eq!(history.len(), 2, "{history:?}");
    let (second, first) = (history[0].version_name.clone(), history[1].version_name.clone());
    assert_eq!(history[0].previous_version_name.as_deref(), Some(first.as_str()));
    assert_eq!(resources.current_version(&a).unwrap().unwrap().version_name, second);
    assert_eq!(resources.root_version(&a).unwrap().unwrap().version_name, first);
    let next: Vec<String> = resources
        .next_versions(&a, &first)
        .unwrap()
        .into_iter()
        .map(|version| version.version_name)
        .collect();
    assert_eq!(next, std::slice::from_ref(&second));
    assert_eq!(
        resources.version_text(&a, &first).unwrap().as_deref(),
        Some(page(0).as_str())
    );
    assert_eq!(
        resources.version_text(&a, &first[..10]).unwrap().as_deref(),
        Some(page(0).as_str())
    );
    assert_eq!(resources.version_text(&a, "0000000").unwrap(), None);
    assert_eq!(resources.location_text(&format!("{a}#{first}")).unwrap(), page(0));
    let diff = resources.version_diff(&a, &first, &second).unwrap();
    assert_eq!((diff.stats.insertions, diff.stats.deletions), (1, 1));
    assert_eq!(resources.version_history(&b).unwrap().len(), 1);

    resources.rename(&a, &a).unwrap();
    assert_eq!(
        resources.version_history(&a).unwrap().len(),
        2,
        "a move onto itself makes no commit"
    );
    let moved = format!("git://{id}/archive/a.md");
    resources.rename(&a, &moved).unwrap();
    assert_eq!(resources.stat(&a).unwrap(), None);
    assert_eq!(resources.get(&moved).unwrap().read_text().unwrap(), page(5));
    let err = resources.delete(&format!("git://{id}/notes")).unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");
    assert!(resources.delete(&moved).unwrap());
    assert!(!resources.delete(&moved).unwrap());
    let names: Vec<String> = resources
        .list(&root)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(names, ["notes"], "the emptied archive directory is gone");
    // the deleted file's old versions are still in the history of its path
    assert_eq!(
        resources.version_text(&a, &first).unwrap().as_deref(),
        Some(page(0).as_str())
    );
    first
}

#[test]
fn a_local_repository_is_a_bare_repository_on_disk() {
    let fixture = fixture("local");
    let _entered = fixture.context.enter();
    let path = fixture.dir.join("local.git");
    define(&fixture.entity, "notes", "local", Some(&path), None);
    let resources = fixture.context.factory().resource();
    scenario(resources, "notes");
    assert!(path.join("HEAD").is_file(), "created on first use");
    let repo = gix::open(&path).unwrap();
    assert!(repo.is_bare());
    let head = repo
        .find_reference("refs/heads/main")
        .unwrap()
        .into_fully_peeled_id()
        .unwrap();
    let commit = repo.find_commit(head).unwrap();
    assert_eq!(
        commit.message_raw_sloppy().to_string().lines().next(),
        Some("Delete archive/a.md")
    );
    let row = fixture
        .entity
        .find(super::GIT_REPOSITORY)
        .unwrap()
        .eq("repositoryId", Value::string("notes"))
        .one()
        .unwrap()
        .unwrap();
    assert_eq!(super::text(row.get("headCommitId")), Some(head.to_string()));
    if let Some(log) = git(&["--git-dir", &path.to_string_lossy(), "log", "--format=%s", "main"]) {
        assert_eq!(log.lines().count(), 5, "{log}");
    }
}

#[test]
fn a_db_repository_exports_and_imports_with_the_same_ids() {
    let fixture = fixture("db");
    let _entered = fixture.context.enter();
    define(&fixture.entity, "wiki", "db", None, None);
    let resources = fixture.context.factory().resource();
    let first = scenario(resources, "wiki");
    let objects = fixture.entity.find(super::db::GIT_OBJECT).unwrap().list().unwrap();
    assert!(objects.len() > 10, "{} objects", objects.len());

    let provider = resources.git().expect("git provider");
    let export = fixture.dir.join("export.git");
    let (path, written) = provider.export("wiki", Some(&export.to_string_lossy())).unwrap();
    assert_eq!(path, export);
    assert!(written > 10);
    let store = fs::FsStore::open(&export).unwrap();
    let db_head = super::db::DbStore::new(fixture.entity.clone(), "wiki")
        .read_ref("refs/heads/main")
        .unwrap();
    assert_eq!(store.read_ref("refs/heads/main").unwrap(), db_head);
    assert_eq!(
        ops::read_at(
            &store,
            &ObjectIdHex::parse(&first),
            &["notes".to_string(), "a.md".to_string()]
        )
        .unwrap(),
        Some(page(0).into_bytes())
    );
    if let Some(fsck) = git(&["--git-dir", &export.to_string_lossy(), "fsck", "--strict"]) {
        assert!(!fsck.contains("error"), "{fsck}");
    }

    define(&fixture.entity, "copy", "db", None, None);
    let (head, copied) = provider.import("copy", Some(&export.to_string_lossy())).unwrap();
    assert_eq!(Some(head.clone()), db_head.map(|id| id.to_string()));
    assert_eq!(copied, written);
    assert_eq!(
        resources.get("git://copy/notes/b.md").unwrap().read_text().unwrap(),
        "other\n"
    );
    define(&fixture.entity, "copy2", "db", None, None);
    let (head2, _) = provider.import("copy2", Some("git://copy")).unwrap();
    assert_eq!(head2, head);
    let err = provider.import("copy2", Some("git://copy2")).unwrap_err();
    assert!(err.to_string().contains("into itself"), "{err}");
}

#[test]
fn a_remote_repository_is_a_read_only_mirror_kept_by_fetch() {
    if git(&["--version"]).is_none() {
        eprintln!("skipped: a file:// fetch runs git-upload-pack, and git is not on the PATH");
        return;
    }
    let fixture = fixture("remote");
    let _entered = fixture.context.enter();
    let source = fixture.dir.join("source.git");
    define(&fixture.entity, "source", "local", Some(&source), None);
    let mirror = fixture.dir.join("mirror.git");
    let url = format!("file://{}", source.display());
    define(&fixture.entity, "mirror", "remote", Some(&mirror), Some(&url));
    let resources = fixture.context.factory().resource();
    let provider = resources.git().expect("git provider");
    resources.write_bytes("git://source/doc.md", b"v1\n").unwrap();

    let err = resources.get("git://mirror/doc.md").unwrap().read_text().unwrap_err();
    assert!(err.to_string().contains("has not been fetched"), "{err}");
    let head = provider.fetch("mirror").unwrap().expect("a head");
    assert_eq!(
        resources.get("git://mirror/doc.md").unwrap().read_text().unwrap(),
        "v1\n"
    );
    let err = resources.write_bytes("git://mirror/doc.md", b"no").unwrap_err();
    assert!(err.to_string().contains("read-only mirror"), "{err}");

    resources.write_bytes("git://source/doc.md", b"v2\n").unwrap();
    let newer = provider.fetch("mirror").unwrap().expect("a head");
    assert_ne!(newer, head);
    assert_eq!(
        resources.get("git://mirror/doc.md").unwrap().read_text().unwrap(),
        "v2\n"
    );
    assert_eq!(resources.version_history("git://mirror/doc.md").unwrap().len(), 2);

    fixture.entity.set_sim_active(true);
    let err = provider.fetch("mirror").unwrap_err();
    fixture.entity.set_sim_active(false);
    assert!(err.to_string().contains("sim mode"), "{err}");

    define(&fixture.entity, "imported", "db", None, Some(&url));
    let (imported, _) = provider.import("imported", None).unwrap();
    assert_eq!(imported, newer);
}

#[test]
fn locations_name_their_repository_and_refuse_odd_paths() {
    let fixture = fixture("errors");
    let _entered = fixture.context.enter();
    let resources = fixture.context.factory().resource();
    let err = resources.stat("git://nosuch/x").unwrap_err();
    assert!(
        err.to_string().contains("git repository nosuch is not defined"),
        "{err}"
    );
    assert!(!resources.get("git://nosuch/x").unwrap().exists());
    let err = resources.stat("git://nosuch/../x").unwrap_err();
    assert!(err.to_string().contains("bad path"), "{err}");
    define(&fixture.entity, "odd", "elsewhere", None, None);
    let err = resources.stat("git://odd/x").unwrap_err();
    assert!(err.to_string().contains("must be remote, local or db"), "{err}");
    define(&fixture.entity, "plain", "db", None, None);
    let err = resources.rename("git://plain/a", "git://odd/a").unwrap_err();
    assert!(err.to_string().contains("between git repositories"), "{err}");
}

/// The output of the git command line, or `None` when git is not installed.
fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr))
}

struct ObjectIdHex;

impl ObjectIdHex {
    fn parse(hex: &str) -> gix::ObjectId {
        gix::ObjectId::from_hex(hex.as_bytes()).unwrap()
    }
}
