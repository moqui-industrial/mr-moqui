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

//! The connector commands through `run#Command` on the in-memory database, for a `dbresource://` volume and a `file:`
//! volume in a temporary directory. The transition, the multipart body and the file response over HTTP are
//! `moqui-host/tests/elfinder.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use moqui_lib::object::{to_plain_string, Value};

use super::{safe_entry_name, unhash, writes_restricted, Connector, RUN_COMMAND};
use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;
use crate::service::ServicePools;
use crate::web::multipart::UploadedFile;

struct Setup {
    facade: EntityFacade,
    context: ExecutionContext,
}

fn setup() -> Setup {
    let facade = crate::testing::open();
    facade.assume_db_tables();
    let context = ExecutionContextFactory::test_context_in("", ServicePools::idle(), crate::context::framework_dir());
    context.factory().set_entity(facade.clone());
    Setup { facade, context }
}

fn install() {
    let services = crate::context::framework_dir().join("service/org/moqui/impl/ElFinderServices.xml");
    let failures = crate::service::lint(&[services]).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
}

/// The hash Java makes of a path under the `db_` volume.
fn hash(path: &str) -> String {
    use base64::Engine as _;
    format!("db_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path))
}

/// Run one command. `other` holds the request parameters as `ec.web.parameters` would.
fn command(root: &str, cmd: &str, target: &str, other: Vec<(&str, Value)>) -> BTreeMap<String, Value> {
    let other: IndexMap<String, Value> = other.into_iter().map(|(key, value)| (key.to_string(), value)).collect();
    let mut parameters = BTreeMap::new();
    parameters.insert("cmd".to_string(), Value::string(cmd));
    parameters.insert("target".to_string(), Value::string(target));
    parameters.insert("resourceRoot".to_string(), Value::string(root));
    parameters.insert("otherParameters".to_string(), Value::Map(other));
    crate::service::sync(RUN_COMMAND, &parameters).unwrap()
}

fn response(out: &BTreeMap<String, Value>) -> serde_json::Value {
    let value = out.get("responseMap").cloned().unwrap_or(Value::Null);
    serde_json::from_str(&crate::entity::query_rest::encode_json(&value)).unwrap()
}

fn names(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn upload(name: &str, bytes: &[u8]) -> Value {
    UploadedFile {
        field_name: "upload[]".to_string(),
        file_name: name.to_string(),
        content_type: "text/plain".to_string(),
        bytes: bytes.to_vec(),
    }
    .into_value()
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(label: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("moqui-elfinder-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

#[test]
fn hashes_are_the_java_hashes() {
    let setup = setup();
    let connector = Connector {
        facade: &setup.facade,
        resource_root: "dbresource://mantle/content".to_string(),
        volume_id: "db_".to_string(),
    };
    // "root".bytes.encodeBase64() is cm9vdA== in Groovy, and the hash drops the padding.
    assert_eq!(connector.hash("root"), "db_cm9vdA");
    assert_eq!(connector.hash("a/b?c>"), "db_YS9iP2M-");
    assert_eq!(unhash("db_YS9iP2M-"), "a/b?c>");
    assert_eq!(unhash("db_cm9vdA.."), "root");
    assert_eq!(unhash("db_cm9vdA=="), "root");
    assert_eq!(unhash("db"), "");
    assert_eq!(unhash("db_!!!"), "");
    assert_eq!(connector.location(""), "dbresource://mantle/content");
    assert_eq!(connector.location("db_cm9vdA"), "dbresource://mantle/content");
    assert_eq!(
        connector.location(&hash("docs/a.txt")),
        "dbresource://mantle/content/docs/a.txt"
    );
    assert_eq!(connector.relative_to_root("dbresource://mantle/content/docs/"), "docs");
    assert_eq!(safe_entry_name("C:\\temp\\a.txt"), Some("a.txt".to_string()));
    assert_eq!(safe_entry_name("../../a.txt"), Some("a.txt".to_string()));
    assert_eq!(safe_entry_name(".."), None);
    assert_eq!(safe_entry_name("a:b"), None);
    assert_eq!(safe_entry_name(""), None);
}

#[test]
fn commands_run_against_a_dbresource_volume() {
    let setup = setup();
    let _entered = setup.context.enter();
    let _authz = setup.context.disable_authz();
    install();
    let root = "dbresource://elfinder";

    let opened = response(&command(
        root,
        "open",
        "",
        vec![("init", Value::string("1")), ("tree", Value::string("1"))],
    ));
    assert_eq!(opened["api"], "2.0", "{opened}");
    assert_eq!(opened["cwd"]["hash"], "db_cm9vdA", "{opened}");
    assert_eq!(opened["cwd"]["name"], root, "{opened}");
    assert_eq!(opened["cwd"]["mime"], "directory", "{opened}");
    assert_eq!(opened["cwd"]["volumeid"], "db_", "{opened}");
    assert_eq!(opened["options"]["path"], root, "{opened}");
    assert_eq!(names(&opened["files"]), vec![root.to_string()], "{opened}");

    let made = response(&command(
        root,
        "mkdir",
        "db_cm9vdA",
        vec![("name", Value::string("docs"))],
    ));
    assert_eq!(made["added"][0]["name"], "docs", "{made}");
    assert_eq!(made["added"][0]["hash"], hash("docs"), "{made}");
    assert_eq!(made["added"][0]["phash"], "db_cm9vdA", "{made}");
    assert_eq!(made["added"][0]["mime"], "directory", "{made}");

    let docs = hash("docs");
    let uploaded = response(&command(
        root,
        "upload",
        &docs,
        vec![(
            "_fileUploadList",
            Value::List(vec![upload("C:\\Users\\me\\a.txt", b"hello"), upload("..", b"skipped")]),
        )],
    ));
    assert_eq!(names(&uploaded["added"]), vec!["a.txt"], "{uploaded}");
    assert_eq!(uploaded["added"][0]["hash"], hash("docs/a.txt"), "{uploaded}");
    assert_eq!(uploaded["added"][0]["phash"], docs, "{uploaded}");
    assert_eq!(uploaded["added"][0]["mime"], "text/plain", "{uploaded}");
    assert_eq!(uploaded["added"][0]["size"], 5, "{uploaded}");

    let listed = response(&command(root, "ls", &docs, vec![]));
    assert_eq!(listed["list"], serde_json::json!(["a.txt"]), "{listed}");
    let opened = response(&command(root, "open", &docs, vec![]));
    assert_eq!(opened["cwd"]["name"], "docs", "{opened}");
    assert_eq!(names(&opened["files"]), vec!["docs", "a.txt"], "{opened}");
    let tree = response(&command(root, "tree", "db_cm9vdA", vec![]));
    assert_eq!(names(&tree["tree"]), vec![root, "docs"], "{tree}");
    let parents = response(&command(root, "parents", &docs, vec![]));
    assert_eq!(names(&parents["tree"]), vec![root, "docs"], "{parents}");
    // The root reports a child directory now.
    assert_eq!(parents["tree"][0]["dirs"], 1, "{parents}");

    let file = hash("docs/a.txt");
    let got = response(&command(root, "get", &file, vec![]));
    assert_eq!(got["content"], "hello", "{got}");
    let put = response(&command(
        root,
        "put",
        &file,
        vec![("content", Value::string("hello again"))],
    ));
    assert_eq!(put["changed"][0]["size"], 11, "{put}");
    let info = setup
        .facade
        .read_db_resource("dbresource://elfinder/docs/a.txt")
        .unwrap();
    assert_eq!(info.bytes, b"hello again");
    let sent = command(root, "file", &file, vec![("download", Value::string("1"))]);
    assert_eq!(
        sent.get("fileLocation").map(to_plain_string).as_deref(),
        Some("dbresource://elfinder/docs/a.txt")
    );
    assert_eq!(sent.get("fileInline"), Some(&Value::Bool(false)));

    let made = response(&command(
        root,
        "mkfile",
        &docs,
        vec![("name", Value::string("empty.md"))],
    ));
    assert_eq!(made["added"][0]["size"], 0, "{made}");

    let renamed = response(&command(root, "rename", &file, vec![("name", Value::string("b.txt"))]));
    assert_eq!(renamed["added"][0]["hash"], hash("docs/b.txt"), "{renamed}");
    assert_eq!(renamed["removed"], serde_json::json!([file]), "{renamed}");
    let listed = response(&command(root, "ls", &docs, vec![]));
    assert_eq!(listed["list"], serde_json::json!(["b.txt", "empty.md"]), "{listed}");
    let got = response(&command(root, "get", &hash("docs/b.txt"), vec![]));
    assert_eq!(got["content"], "hello again", "{got}");

    let missing = response(&command(root, "mkdir", &docs, vec![("name", Value::string(".."))]));
    assert_eq!(missing["error"], "No name specified for new directory", "{missing}");

    let removed = response(&command(
        root,
        "rm",
        "",
        vec![("targets[]", Value::List(vec![Value::string(&docs)]))],
    ));
    let removed: Vec<&str> = removed["removed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(
        removed,
        vec![
            hash("docs/b.txt").as_str(),
            hash("docs/empty.md").as_str(),
            docs.as_str()
        ]
    );
    let listed = response(&command(root, "ls", "db_cm9vdA", vec![]));
    assert_eq!(listed["list"], serde_json::json!([]), "{listed}");
    assert!(
        !setup
            .facade
            .read_db_resource("dbresource://elfinder/docs")
            .unwrap()
            .exists
    );
}

#[test]
fn a_target_or_a_name_outside_the_volume_stays_inside_it() {
    let setup = setup();
    let _entered = setup.context.enter();
    let _authz = setup.context.disable_authz();
    install();
    let dir = temp_dir("traverse");
    let volume = dir.0.join("volume");
    std::fs::create_dir_all(volume.join("sub")).unwrap();
    std::fs::write(dir.0.join("secret.txt"), "outside").unwrap();
    std::fs::write(volume.join("sub/inside.txt"), "inside").unwrap();
    let root = format!("file:{}", volume.display());

    for path in [
        "../secret.txt",
        "sub/../../secret.txt",
        "..\\secret.txt",
        "/../secret.txt",
        "file:/etc/passwd",
    ] {
        let sent = command(&root, "file", &hash(path), vec![]);
        assert_eq!(
            sent.get("fileLocation").map(to_plain_string).as_deref(),
            Some(root.as_str()),
            "{path}"
        );
        let got = response(&command(&root, "get", &hash(path), vec![]));
        assert_eq!(got["content"], "", "{path}: {got}");
    }
    let listed = response(&command(&root, "ls", &hash("sub/../.."), vec![]));
    assert_eq!(listed["list"], serde_json::json!(["sub"]), "{listed}");
    let got = response(&command(&root, "get", &hash("sub/./inside.txt"), vec![]));
    assert_eq!(got["content"], "inside", "{got}");

    // A name keeps only its last part, so a new directory or upload lands in the target.
    let made = response(&command(
        &root,
        "mkdir",
        &hash("sub"),
        vec![("name", Value::string("../../evil"))],
    ));
    assert_eq!(made["added"][0]["hash"], hash("sub/evil"), "{made}");
    assert!(volume.join("sub/evil").is_dir());
    assert!(!dir.0.join("evil").exists());
    let uploaded = response(&command(
        &root,
        "upload",
        &hash("../.."),
        vec![("_fileUploadList", Value::List(vec![upload("../up.txt", b"up")]))],
    ));
    assert_eq!(uploaded["added"][0]["hash"], hash("up.txt"), "{uploaded}");
    assert_eq!(std::fs::read_to_string(volume.join("up.txt")).unwrap(), "up");
    let renamed = response(&command(
        &root,
        "rename",
        &hash("up.txt"),
        vec![("name", Value::string("../moved.txt"))],
    ));
    assert_eq!(renamed["added"][0]["hash"], hash("moved.txt"), "{renamed}");
    assert!(volume.join("moved.txt").is_file());
    assert!(!dir.0.join("moved.txt").exists());

    // A content:// root is an error that names it.
    let mut parameters = BTreeMap::new();
    parameters.insert("cmd".to_string(), Value::string("ls"));
    parameters.insert("target".to_string(), Value::string("db_cm9vdA"));
    parameters.insert("resourceRoot".to_string(), Value::string("content://main/files"));
    let err = crate::service::sync(RUN_COMMAND, &parameters).unwrap_err();
    assert!(err.message().contains("content://main/files"), "{err}");
    assert_eq!(std::fs::read_to_string(dir.0.join("secret.txt")).unwrap(), "outside");
}

#[test]
fn writes_are_refused_in_production_except_for_a_dbresource_root() {
    // The property is process-wide, so the check is tested on its inputs rather than set for one test.
    assert!(writes_restricted("production", "file:runtime"));
    assert!(writes_restricted("production", "component://webroot"));
    assert!(!writes_restricted("production", "dbresource://mantle/content"));
    assert!(!writes_restricted("production", "content://main"));
    assert!(!writes_restricted("dev", "file:runtime"));
    assert!(!writes_restricted("", "component://webroot"));
}
