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

//! The git server against the git command line, without the host: a small HTTP/1.1 listener here calls
//! [`super::serve_http`] (stateless RPC, protocol v0 and v2) and a `git://` listener calls [`super::serve_ssh`] (one
//! stateful conversation, as over SSH), on the in-memory database. Each test is skipped when git is not installed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use gix::objs::Kind;
use moqui_lib::object::Value;

use super::{pack_out, plan, GitHttpRequest, GitService, HttpResponder};
use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::entity::{EntityFacade, EntityValue};
use crate::resource::git::ops::{self, Author, Edit};
use crate::resource::git::store::{GitStore, MemoryStore};

const WRITER: &str = "git.writer";
const READER: &str = "git.reader";
const PASSWORD: &str = "moqui-test-1";

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

fn create(entity: &EntityFacade, name: &str, pairs: &[(&str, &str)]) {
    let mut row = EntityValue::make(entity, name).unwrap();
    for (field, value) in pairs {
        row.set(*field, Value::string(*value));
    }
    row.create(entity).unwrap();
}

fn user(entity: &EntityFacade, username: &str) {
    let mut row = EntityValue::make(entity, "moqui.security.UserAccount").unwrap();
    row.set("userId", Value::string(username));
    row.set("username", Value::string(username));
    row.set("userFullName", Value::string(format!("Test {username}")));
    row.set("emailAddress", Value::string(format!("{username}@example.com")));
    row.set("currentPassword", Value::string(crate::user::hash_password(PASSWORD).unwrap()));
    row.create(entity).unwrap();
}

/// Two users: the writer has every action on every repository, the reader only `view`.
fn fixture(name: &str) -> Fixture {
    let context = ExecutionContextFactory::test_context_with_conf(
        r#"<moqui-conf><artifact-execution-facade>
            <artifact-execution type="AT_GIT_REPOSITORY" authz-enabled="true"/>
            <artifact-execution type="AT_ENTITY" authz-enabled="true"/>
        </artifact-execution-facade></moqui-conf>"#,
    );
    let entity = crate::testing::open();
    context.factory().set_entity(entity.clone());
    context.factory().resource().install_git().unwrap();
    let dir = std::env::temp_dir().join(format!("moqui-git-server-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    {
        let _entered = context.enter();
        let _authz = context.disable_authz();
        user(&entity, WRITER);
        user(&entity, READER);
        create(&entity, "moqui.security.ArtifactGroup", &[("artifactGroupId", "GIT_ALL")]);
        create(
            &entity,
            "moqui.security.ArtifactGroupMember",
            &[
                ("artifactGroupId", "GIT_ALL"),
                ("artifactName", ".*"),
                ("artifactTypeEnumId", "AT_GIT_REPOSITORY"),
                ("nameIsPattern", "Y"),
                ("inheritAuthz", "N"),
            ],
        );
        for (group, member, action) in [("GIT_WRITERS", WRITER, "AUTHZA_ALL"), ("GIT_READERS", READER, "AUTHZA_VIEW")] {
            create(&entity, "moqui.security.UserGroup", &[("userGroupId", group)]);
            let mut row = EntityValue::make(&entity, "moqui.security.UserGroupMember").unwrap();
            row.set("userId", Value::string(member));
            row.set("userGroupId", Value::string(group));
            row.set(
                "fromDate",
                moqui_lib::object::basic_convert(&Value::I64(1_000), "java.sql.Timestamp").value,
            );
            row.create(&entity).unwrap();
            create(
                &entity,
                "moqui.security.ArtifactAuthz",
                &[
                    ("artifactAuthzId", group),
                    ("userGroupId", group),
                    ("artifactGroupId", "GIT_ALL"),
                    ("authzTypeEnumId", "AUTHZT_ALWAYS"),
                    ("authzActionEnumId", action),
                ],
            );
        }
    }
    Fixture { context, entity, dir }
}

fn define(fixture: &Fixture, id: &str, kind: &str) {
    let _entered = fixture.context.enter();
    let _authz = fixture.context.disable_authz();
    let mut row = EntityValue::make(&fixture.entity, crate::resource::git::GIT_REPOSITORY).unwrap();
    row.set("repositoryId", Value::string(id));
    row.set("repositoryKind", Value::string(kind));
    if kind != "db" {
        row.set("localPath", Value::string(fixture.dir.join(format!("{id}.git")).to_string_lossy()));
    }
    row.create(&fixture.entity).unwrap();
}

// ---- an HTTP/1.1 listener over serve_http ----

struct Chunked<R> {
    inner: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.left == 0 {
            let mut line = String::new();
            self.inner.read_line(&mut line)?;
            if line.trim().is_empty() {
                line.clear();
                self.inner.read_line(&mut line)?;
            }
            self.left = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| std::io::Error::other(format!("bad chunk size {line:?}")))?;
            if self.left == 0 {
                self.done = true;
                let mut end = String::new();
                self.inner.read_line(&mut end)?;
                return Ok(0);
            }
        }
        let wanted = buf.len().min(self.left);
        let read = self.inner.read(&mut buf[..wanted])?;
        self.left -= read;
        if self.left == 0 {
            let mut end = [0u8; 2];
            self.inner.read_exact(&mut end)?;
        }
        Ok(read)
    }
}

struct Responder {
    stream: std::io::BufWriter<TcpStream>,
    status: Option<u16>,
}

impl HttpResponder for Responder {
    fn head(&mut self, status: u16, headers: &[(&str, String)]) -> std::io::Result<()> {
        self.status = Some(status);
        write!(self.stream, "HTTP/1.1 {status} X\r\n")?;
        for (name, value) in headers {
            write!(self.stream, "{name}: {value}\r\n")?;
        }
        write!(self.stream, "Connection: close\r\n\r\n")
    }

    fn body(&mut self) -> &mut dyn Write {
        &mut self.stream
    }
}

fn handle(factory: &Arc<ExecutionContextFactory>, stream: TcpStream) {
    let context = factory.new_context();
    let _entered = context.enter();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let rest = path.strip_prefix("/git/").unwrap_or("");
    let (repository, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let (service, advertise) = match (method.as_str(), tail) {
        ("GET", "info/refs") => (query.strip_prefix("service=").and_then(GitService::from_name), true),
        ("POST", name) => (GitService::from_name(name), false),
        _ => (None, false),
    };
    let mut responder = Responder {
        stream: std::io::BufWriter::new(stream),
        status: None,
    };
    let Some(service) = service else {
        responder.head(404, &[]).unwrap();
        return;
    };
    let request = GitHttpRequest {
        repository: super::repository_of(repository).unwrap_or_default(),
        service,
        advertise,
        git_protocol: headers.get("git-protocol").cloned(),
        gzip: headers.get("content-encoding").is_some_and(|value| value == "gzip"),
        authorization: headers.get("authorization").cloned(),
        api_key: headers.get("api_key").cloned(),
        secure: true,
        client_ip: "127.0.0.1".into(),
    };
    let mut body: Box<dyn Read> = if headers.get("transfer-encoding").is_some_and(|value| value == "chunked") {
        Box::new(Chunked {
            inner: reader,
            left: 0,
            done: false,
        })
    } else {
        let length: u64 = headers.get("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
        Box::new(reader.take(length))
    };
    super::serve_http(&request, &mut body, &mut responder);
    let _ = responder.stream.flush();
}

/// Serve HTTP on a free port of loopback; the base URL.
fn http_server(factory: &Arc<ExecutionContextFactory>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let factory = Arc::clone(factory);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let factory = Arc::clone(&factory);
            std::thread::spawn(move || handle(&factory, stream));
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Serve `git://` on a free port: the first pkt-line names the service and repository (and `version=2` in its extra
/// parameters), then the conversation is stateful, as over SSH, as `username`.
fn daemon(factory: &Arc<ExecutionContextFactory>, username: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let factory = Arc::clone(factory);
    let username = username.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let factory = Arc::clone(&factory);
            let username = username.clone();
            std::thread::spawn(move || {
                let context = factory.new_context();
                let _entered = context.enter();
                let mut input = stream.try_clone().unwrap();
                let Some(super::pktline::Packet::Data(first)) = super::pktline::read(&mut input).unwrap() else {
                    return;
                };
                let mut fields = first.split(|byte| *byte == 0);
                let command = String::from_utf8_lossy(fields.next().unwrap_or_default()).into_owned();
                let protocol = fields
                    .map(|field| String::from_utf8_lossy(field).into_owned())
                    .find(|field| field.starts_with("version="));
                let (service, path) = command.split_once(' ').unwrap_or((&command, ""));
                let mut output = std::io::BufWriter::new(stream);
                let mut errors = Vec::new();
                super::serve_ssh(
                    &username,
                    &format!("{service} '{path}'"),
                    protocol.as_deref(),
                    &mut input,
                    &mut output,
                    &mut errors,
                );
                let _ = output.flush();
            });
        }
    });
    format!("git://127.0.0.1:{port}")
}

// ---- the git command line ----

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|output| output.status.success())
}

/// Run git in `dir` (with a home of its own, so no user config is read). Its output, and whether it succeeded.
fn git_in(dir: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let mut command = Command::new("git");
    command
        .args(["-c", "user.name=Test Pusher", "-c", "user.email=pusher@example.com", "-c", "init.defaultBranch=main"])
        .args(["-c", "credential.helper="])
        .args(args)
        .current_dir(dir)
        // nothing of the desktop: no config but this, no credential helper or keyring, and no askpass dialog when a
        // test sends no credentials (GIT_TERMINAL_PROMPT only stops the terminal prompt)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("XDG_CONFIG_HOME")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "false")
        .env("SSH_ASKPASS", "false")
        .env_remove("SSH_ASKPASS_REQUIRE")
        .env("GIT_ALLOW_PROTOCOL", "file:git:http:https");
    // MOQUI_GIT_TRACE=1 shows each pkt-line in the failure output
    if std::env::var_os("MOQUI_GIT_TRACE").is_some() {
        command.env("GIT_TRACE_PACKET", "1");
    }
    let started = std::time::Instant::now();
    let output = command.output().expect("git runs");
    if std::env::var_os("MOQUI_GIT_TIMES").is_some() {
        eprintln!("{:>6} ms git {args:?}", started.elapsed().as_millis());
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
    (output.status.success(), text)
}

fn git_ok(dir: &Path, home: &Path, args: &[&str]) -> String {
    let (ok, text) = git_in(dir, home, args);
    assert!(ok, "git {args:?} failed:\n{text}");
    text
}

fn url_with(base: &str, username: &str, id: &str) -> String {
    base.replacen("http://", &format!("http://{username}:{PASSWORD}@"), 1) + &format!("/git/{id}.git")
}

fn write_file(dir: &Path, name: &str, text: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn head_of(fixture: &Fixture, id: &str, name: &str) -> Option<String> {
    let _entered = fixture.context.enter();
    let provider = fixture.context.factory().resource().git().unwrap().clone();
    provider.repo(id).unwrap().store.read_ref(name).unwrap().map(|id| id.to_string())
}

/// Clone, commit, push, fetch into another clone, and read the pushed file through `git://`, at a protocol version.
fn round_trip(fixture: &Fixture, kind: &str, version: &str) {
    let id = format!("rt-{kind}-v{version}");
    define(fixture, &id, kind);
    let base = http_server(fixture.context.factory());
    let home = fixture.dir.join(format!("home-{id}"));
    std::fs::create_dir_all(&home).unwrap();
    let url = url_with(&base, WRITER, &id);
    let protocol = format!("protocol.version={version}");

    // an empty repository clones, then takes a first push that names its default branch
    let first = fixture.dir.join(format!("first-{id}"));
    let out = git_ok(&fixture.dir, &home, &["-c", &protocol, "clone", &url, &first.to_string_lossy()]);
    assert!(out.contains("empty repository"), "{out}");
    write_file(&first, "README.md", "# Round trip\n");
    write_file(&first, "src/lib.rs", "pub fn one() -> u32 {\n    1\n}\n");
    git_ok(&first, &home, &["add", "."]);
    git_ok(&first, &home, &["commit", "-m", "First commit\n\nTask: RT-100001"]);
    git_ok(&first, &home, &["-c", &protocol, "push", "origin", "main"]);
    let pushed = git_ok(&first, &home, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head_of(fixture, &id, "refs/heads/main").as_deref(), Some(pushed.as_str()));

    // a second clone sees it; a commit there is fetched into the first
    let second = fixture.dir.join(format!("second-{id}"));
    git_ok(&fixture.dir, &home, &["-c", &protocol, "clone", &url, &second.to_string_lossy()]);
    assert_eq!(std::fs::read_to_string(second.join("README.md")).unwrap(), "# Round trip\n");
    for round in 0..3 {
        write_file(&second, &format!("notes/{round}.md"), &format!("note {round}\n"));
        git_ok(&second, &home, &["add", "."]);
        git_ok(&second, &home, &["commit", "-m", &format!("Note {round}")]);
    }
    git_ok(&second, &home, &["tag", "-a", "v1.0", "-m", "Release 1.0"]);
    git_ok(&second, &home, &["-c", &protocol, "push", "--follow-tags", "origin", "main"]);
    git_ok(&first, &home, &["-c", &protocol, "pull", "--ff-only", "origin", "main"]);
    git_ok(&first, &home, &["-c", &protocol, "fetch", "--tags", "origin"]);
    assert_eq!(std::fs::read_to_string(first.join("notes/2.md")).unwrap(), "note 2\n");
    assert!(git_ok(&first, &home, &["tag", "-l"]).contains("v1.0"));
    if let Some(fsck) = Some(git_ok(&first, &home, &["fsck", "--strict"])) {
        assert!(!fsck.contains("error"), "{fsck}");
    }

    // the pushed files are git:// resources
    {
        let _entered = fixture.context.enter();
        let resources = fixture.context.factory().resource();
        assert_eq!(
            resources.get(&format!("git://{id}/notes/1.md")).unwrap().read_text().unwrap(),
            "note 1\n"
        );
        resources.write_bytes(&format!("git://{id}/notes/web.md"), b"from the web\n").unwrap();
    }
    git_ok(&first, &home, &["-c", &protocol, "pull", "--ff-only", "origin", "main"]);
    assert_eq!(std::fs::read_to_string(first.join("notes/web.md")).unwrap(), "from the web\n");

    // a branch, a force push, a delete
    git_ok(&first, &home, &["checkout", "-b", "feature"]);
    write_file(&first, "feature.txt", "feature\n");
    git_ok(&first, &home, &["add", "."]);
    git_ok(&first, &home, &["commit", "-m", "Feature"]);
    git_ok(&first, &home, &["-c", &protocol, "push", "origin", "feature"]);
    git_ok(&first, &home, &["commit", "--amend", "-m", "Feature, amended"]);
    let (ok, out) = git_in(&first, &home, &["-c", &protocol, "push", "origin", "feature"]);
    assert!(!ok && out.contains("rejected"), "{out}");
    git_ok(&first, &home, &["-c", &protocol, "push", "--force", "origin", "feature"]);
    git_ok(&first, &home, &["-c", &protocol, "push", "origin", "--delete", "feature"]);
    assert_eq!(head_of(fixture, &id, "refs/heads/feature"), None);
    let (ok, out) = git_in(&first, &home, &["-c", &protocol, "push", "origin", "--delete", "main"]);
    assert!(!ok && out.contains("deletion of the current branch prohibited"), "{out}");
}

#[test]
fn http_clone_push_and_fetch_on_each_kind_and_protocol() {
    if !git_available() {
        eprintln!("skipped: git is not on the PATH");
        return;
    }
    let fixture = fixture("http");
    for kind in ["local", "db"] {
        for version in ["0", "2"] {
            round_trip(&fixture, kind, version);
        }
    }
}

#[test]
fn shallow_fetches_deepen_and_unshallow() {
    if !git_available() {
        eprintln!("skipped: git is not on the PATH");
        return;
    }
    let fixture = fixture("shallow");
    let id = "shallow";
    define(&fixture, id, "local");
    let base = http_server(fixture.context.factory());
    let home = fixture.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let url = url_with(&base, WRITER, id);
    let source = fixture.dir.join("source");
    git_ok(&fixture.dir, &home, &["clone", &url, &source.to_string_lossy()]);
    for round in 0..6 {
        write_file(&source, "file.txt", &format!("version {round}\n"));
        git_ok(&source, &home, &["add", "."]);
        git_ok(&source, &home, &["commit", "-m", &format!("Version {round}")]);
    }
    git_ok(&source, &home, &["push", "origin", "main"]);
    for version in ["0", "2"] {
        let protocol = format!("protocol.version={version}");
        let shallow = fixture.dir.join(format!("shallow-v{version}"));
        git_ok(
            &fixture.dir,
            &home,
            &["-c", &protocol, "clone", "--depth", "1", &url, &shallow.to_string_lossy()],
        );
        let count = |dir: &Path| git_ok(dir, &home, &["rev-list", "--count", "HEAD"]).trim().to_string();
        assert_eq!(count(&shallow), "1");
        let out = git_ok(&shallow, &home, &["-c", &protocol, "fetch", "--deepen", "2", "origin"]);
        assert_eq!(count(&shallow), "3", "{out}");
        git_ok(&shallow, &home, &["-c", &protocol, "fetch", "--unshallow", "origin"]);
        assert_eq!(count(&shallow), "6");
        assert!(!shallow.join(".git/shallow").exists());
        let fsck = git_ok(&shallow, &home, &["fsck", "--strict"]);
        assert!(!fsck.contains("error"), "{fsck}");
    }
}

#[test]
fn stateful_conversations_over_a_daemon_connection() {
    if !git_available() {
        eprintln!("skipped: git is not on the PATH");
        return;
    }
    let fixture = fixture("daemon");
    let id = "daemon";
    define(&fixture, id, "local");
    let base = daemon(fixture.context.factory(), WRITER);
    let home = fixture.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let url = format!("{base}/{id}.git");
    let work = fixture.dir.join("work");
    git_ok(&fixture.dir, &home, &["clone", &url, &work.to_string_lossy()]);
    write_file(&work, "a.txt", "a\n");
    git_ok(&work, &home, &["add", "."]);
    git_ok(&work, &home, &["commit", "-m", "A"]);
    git_ok(&work, &home, &["push", "origin", "main"]);
    for version in ["0", "2"] {
        let copy = fixture.dir.join(format!("copy-v{version}"));
        git_ok(
            &fixture.dir,
            &home,
            &["-c", &format!("protocol.version={version}"), "clone", &url, &copy.to_string_lossy()],
        );
        assert_eq!(std::fs::read_to_string(copy.join("a.txt")).unwrap(), "a\n");
    }
}

#[test]
fn credentials_and_rights_are_checked() {
    if !git_available() {
        eprintln!("skipped: git is not on the PATH");
        return;
    }
    let fixture = fixture("rights");
    define(&fixture, "guarded", "db");
    define(&fixture, "mirror", "remote");
    let base = http_server(fixture.context.factory());
    let home = fixture.dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let work = fixture.dir.join("work");
    git_ok(&fixture.dir, &home, &["clone", &url_with(&base, WRITER, "guarded"), &work.to_string_lossy()]);
    write_file(&work, "a.txt", "a\n");
    git_ok(&work, &home, &["add", "."]);
    git_ok(&work, &home, &["commit", "-m", "A"]);
    git_ok(&work, &home, &["push", "origin", "main"]);

    // the reader may clone and not push
    let read = fixture.dir.join("read");
    git_ok(&fixture.dir, &home, &["clone", &url_with(&base, READER, "guarded"), &read.to_string_lossy()]);
    write_file(&read, "b.txt", "b\n");
    git_ok(&read, &home, &["add", "."]);
    git_ok(&read, &home, &["commit", "-m", "B"]);
    let (ok, out) = git_in(&read, &home, &["push", "origin", "main"]);
    assert!(!ok && out.contains("403"), "{out}");

    // a wrong password, no credentials, an unknown repository, a mirror
    let wrong = base.replacen("http://", &format!("http://{WRITER}:wrong@"), 1) + "/git/guarded.git";
    let (ok, out) = git_in(&fixture.dir, &home, &["ls-remote", &wrong]);
    assert!(!ok && (out.contains("401") || out.contains("Authentication failed")), "{out}");
    let (ok, out) = git_in(&fixture.dir, &home, &["ls-remote", &format!("{base}/git/guarded.git")]);
    assert!(!ok, "{out}");
    let (ok, out) = git_in(&fixture.dir, &home, &["ls-remote", &url_with(&base, WRITER, "nosuch")]);
    assert!(!ok && out.contains("not found"), "{out}");
    let (ok, out) = git_in(&work, &home, &["push", &url_with(&base, WRITER, "mirror"), "main"]);
    assert!(!ok, "{out}");

    // an access token (a UserLoginKey) as the password
    let token = {
        let _entered = fixture.context.enter();
        crate::user::internal_login_user(WRITER).unwrap();
        let key = crate::user::get_login_key(Some(1.0)).unwrap();
        crate::user::logout().unwrap();
        key
    };
    let with_token = base.replacen("http://", &format!("http://{WRITER}:{token}@"), 1) + "/git/guarded.git";
    let out = git_ok(&fixture.dir, &home, &["ls-remote", &with_token]);
    assert!(out.contains("refs/heads/main"), "{out}");
}

// ---- packs without the command line ----

fn commit_files(store: &MemoryStore, files: &[(&str, &str)], message: &str) -> gix::ObjectId {
    let edits: Vec<Edit> = files
        .iter()
        .map(|(path, text)| {
            let blob = store.write(Kind::Blob, text.as_bytes()).unwrap();
            Edit::Put(
                path.split('/').map(str::to_string).collect(),
                gix::objs::tree::EntryKind::Blob,
                blob,
            )
        })
        .collect();
    let author = Author {
        name: "Test".into(),
        email: "test@example.com".into(),
        user_id: None,
    };
    ops::commit(store, "main", &author, |_| Ok(Some((edits.clone(), message.into()))))
        .unwrap()
        .unwrap()
}

#[test]
fn a_planned_pack_holds_what_the_client_lacks() {
    let store = MemoryStore::default();
    let first = commit_files(&store, &[("a.txt", "a\n"), ("dir/b.txt", "b\n")], "one");
    let second = commit_files(&store, &[("a.txt", "a2\n")], "two");
    let all = plan::plan(&store, &[second], &[], &[], &Default::default(), &[]).unwrap();
    // two commits, the trees (root twice, dir once) and three blobs
    assert_eq!(all.commits, 2);
    assert_eq!(all.objects.len(), 2 + 3 + 3);
    let delta = plan::plan(&store, &[second], &[first], &[], &Default::default(), &[]).unwrap();
    // the new commit, its root tree and the new blob: dir/ and b.txt are under the client's tree
    assert_eq!(delta.commits, 1);
    assert_eq!(delta.objects.len(), 3);
    let mut pack = Vec::new();
    pack_out::write_pack(&store, &all.objects, true, None, &mut pack).unwrap();
    let read = pack_out::read_back(&pack);
    assert_eq!(read.len(), all.objects.len());
    for (id, kind) in read {
        assert_eq!(store.find(&id).unwrap().map(|(found, _)| found), Some(kind));
    }
}

#[test]
fn ssh_commands_name_a_service_and_a_repository() {
    use super::parse_ssh_command;
    assert_eq!(
        parse_ssh_command("git-upload-pack '/code.git'"),
        Some((GitService::UploadPack, "code".to_string()))
    );
    assert_eq!(
        parse_ssh_command("git receive-pack 'git/code'"),
        Some((GitService::ReceivePack, "code".to_string()))
    );
    assert_eq!(
        parse_ssh_command("git-upload-pack code.git"),
        Some((GitService::UploadPack, "code".to_string()))
    );
    assert_eq!(parse_ssh_command("git-upload-archive '/code.git'"), None);
    assert_eq!(parse_ssh_command("git-upload-pack '../etc/passwd'"), None);
    assert_eq!(parse_ssh_command("git-upload-packx 'code'"), None);
}
