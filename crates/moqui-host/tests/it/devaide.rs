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

//! The dev-aide's services on a dev instance (plans/design-dev-aide.md P2): a site component and an upstream app, each its
//! own scratch git repository with a bare remote, and git isolated from the desktop (no global or system config, no
//! askpass, no credential helper). Saves reload and check, a save over a changed file is refused with the disk's text,
//! the upstream lock and the tiers hold, paths stay in their roots, writes are audited, commits name the user as author
//! with the task trailer, and Publish pushes the site's changes, to a publish branch when one is set, and not at all on a
//! shared server whose network git is off.

#![cfg(all(unix, feature = "embedded-postgres", feature = "axum", feature = "devtools"))]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_lib::object::{to_plain_string, Value};

use crate::common::start_with_files_and_links;

const SITE_COMPONENT: &str = r#"<component name="my-site"/>"#;
const SITE_SERVICES: &str = r#"<services>
    <service verb="get" noun="Thing">
        <out-parameters><parameter name="result"/></out-parameters>
        <actions><set field="result" value="first"/></actions>
    </service>
</services>
"#;
const UPSTREAM_COMPONENT: &str = r#"<component name="upstream-app"/>"#;
const UPSTREAM_SERVICES: &str = "<services>\n    <service verb=\"run\" noun=\"Up\"/>\n</services>\n";

/// git with the desktop left out: no global or system config, no prompt, no askpass.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The process's git, for the server and this test alike, kept from the desktop's config and credentials; the committer is
/// the server's identity.
fn isolate_git(home: &Path) {
    for (name, value) in [
        ("HOME", home.display().to_string()),
        ("GIT_CONFIG_GLOBAL", "/dev/null".to_string()),
        ("GIT_CONFIG_NOSYSTEM", "1".to_string()),
        ("GIT_TERMINAL_PROMPT", "0".to_string()),
        ("GIT_ASKPASS", String::new()),
        ("GIT_COMMITTER_NAME", "Dev Server".to_string()),
        ("GIT_COMMITTER_EMAIL", "server@example.com".to_string()),
        ("GIT_AUTHOR_NAME", "Setup".to_string()),
        ("GIT_AUTHOR_EMAIL", "setup@example.com".to_string()),
    ] {
        std::env::set_var(name, value);
    }
}

/// A repository with one commit on main, pushed to a bare remote beside it.
fn repository(dir: &Path, remote: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "Start"]);
    git(
        remote.parent().unwrap(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git(dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(dir, &["push", "-q", "-u", "origin", "main"]);
}

fn call(
    factory: &Arc<ExecutionContextFactory>,
    user: &str,
    name: &str,
    parameters: &[(&str, &str)],
) -> Result<BTreeMap<String, Value>, String> {
    let context = factory.new_context();
    let _entered = context.enter();
    moqui_core::user::internal_login_user(user).unwrap();
    let parameters: BTreeMap<String, Value> = parameters
        .iter()
        .map(|(key, value)| ((*key).to_string(), Value::string(*value)))
        .collect();
    let result = moqui_core::service::sync(name, &parameters).map_err(|err| err.to_string());
    let errors = context.message().errors().to_vec();
    match result {
        Ok(out) if errors.is_empty() => Ok(out),
        Ok(_) => Err(errors.join("; ")),
        Err(err) => Err(format!("{err} {}", errors.join("; "))),
    }
}

/// A service of the test's own component, which no artifact group grants.
fn run(factory: &Arc<ExecutionContextFactory>, name: &str) -> BTreeMap<String, Value> {
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    moqui_core::service::sync(name, &BTreeMap::new()).unwrap()
}

fn text(out: &BTreeMap<String, Value>, key: &str) -> String {
    out.get(key).map(to_plain_string).unwrap_or_default()
}

const WS: &str = "org.moqui.devaide.WorkspaceServices.";
const VCS: &str = "org.moqui.devaide.VcsServices.";

#[test]
fn the_workspace_saves_checks_commits_and_publishes() {
    let users = r#"
    <moqui.security.UserAccount userId="DEV_EXT" username="ext.user" userFullName="Ext User" emailAddress="ext@example.com"/>
    <moqui.security.UserGroupMember userGroupId="DEV_AIDE_EXTENDERS" userId="DEV_EXT" fromDate="0"/>
</entity-facade-xml>"#;
    let security = include_str!("../../../../runtime/dev-component/dev-aide/data/DevAideSecurityData.xml")
        .replace("</entity-facade-xml>", users);
    let dev_aide = std::fs::canonicalize(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../runtime/dev-component/dev-aide"
    ))
    .unwrap();
    let files = [
        ("component/my-site/component.xml", SITE_COMPONENT),
        ("component/my-site/service/mysite/Things.xml", SITE_SERVICES),
        ("component/upstream-app/component.xml", UPSTREAM_COMPONENT),
        ("component/upstream-app/service/upapp/Up.xml", UPSTREAM_SERVICES),
    ];
    let gate = start_with_files_and_links(
        "dev-aide",
        Some(&security),
        &[
            ("instance_purpose", "dev"),
            ("dev_aide_site_component", "my-site"),
            ("dev_aide_shared", "false"),
        ],
        &files,
        &[(dev_aide.to_str().unwrap(), "dev-component/dev-aide")],
    );
    let factory = Arc::clone(gate.server.factory());
    let site = gate.runtime.join("component/my-site");
    let upstream = gate.runtime.join("component/upstream-app");
    isolate_git(&gate.runtime);
    repository(&site, &gate.runtime.join("site-remote.git"));
    repository(&upstream, &gate.runtime.join("upstream-remote.git"));

    // the roots: the site component's repository and the upstream app's, by their directory names
    let roots = call(&factory, "john.doe", &format!("{WS}get#Roots"), &[]).unwrap();
    let listed = moqui_lib::json::to_json(&roots["roots"]);
    let root_of = |id: &str| {
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|root| root["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(root_of("my-site")["upstream"], false, "{listed} {roots:?}");
    assert_eq!(root_of("my-site")["kind"], "git");
    assert_eq!(root_of("upstream-app")["upstream"], true);
    assert_eq!(text(&roots, "tier"), "full");

    // a save of a service file: written, reloaded, checked; the new service runs at once
    let path = "service/mysite/Things.xml";
    let file = call(
        &factory,
        "john.doe",
        &format!("{WS}get#File"),
        &[("rootId", "my-site"), ("path", path)],
    )
    .unwrap();
    let base = text(&file, "hash");
    let changed = SITE_SERVICES.replace("value=\"first\"", "value=\"second\"").replace(
        "</services>",
        "    <service verb=\"get\" noun=\"Other\"><out-parameters><parameter name=\"result\"/></out-parameters>\n        <actions><set field=\"result\" value=\"other\"/></actions></service>\n</services>",
    );
    let saved = call(
        &factory,
        "john.doe",
        &format!("{WS}save#File"),
        &[
            ("rootId", "my-site"),
            ("path", path),
            ("text", &changed),
            ("baseHash", &base),
        ],
    )
    .unwrap();
    assert_eq!(text(&saved, "conflict"), "false");
    let report = moqui_lib::json::to_json(&saved["reload"]);
    assert!(
        report["files"][0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("service file reloaded"),
        "{report}"
    );
    assert_eq!(text(&saved, "status"), "M");
    let thing = run(&factory, "mysite.Things.get#Thing");
    assert_eq!(text(&thing, "result"), "second");
    let other = run(&factory, "mysite.Things.get#Other");
    assert_eq!(text(&other, "result"), "other");

    // a save from the old hash: the file changed since, so it is refused with the disk's text
    let stale = call(
        &factory,
        "john.doe",
        &format!("{WS}save#File"),
        &[
            ("rootId", "my-site"),
            ("path", path),
            ("text", "<services/>"),
            ("baseHash", &base),
        ],
    )
    .unwrap();
    assert_eq!(text(&stale, "conflict"), "true");
    assert_eq!(text(&stale, "diskText"), changed);
    assert_eq!(std::fs::read_to_string(site.join(path)).unwrap(), changed);

    // the upstream lock: refused, naming the site component to extend in
    let up = call(
        &factory,
        "john.doe",
        &format!("{WS}get#File"),
        &[("rootId", "upstream-app"), ("path", "service/upapp/Up.xml")],
    )
    .unwrap();
    assert_eq!(text(&up, "readOnly"), "true");
    let refused = call(
        &factory,
        "john.doe",
        &format!("{WS}save#File"),
        &[
            ("rootId", "upstream-app"),
            ("path", "service/upapp/Up.xml"),
            ("text", UPSTREAM_SERVICES),
            ("baseHash", &text(&up, "hash")),
        ],
    )
    .unwrap_err();
    assert!(
        refused.contains("upstream-app is upstream: extend it in my-site"),
        "{refused}"
    );

    // the extend tier: writes the site component, not upstream, and cannot turn the lock off
    let ext_file = call(
        &factory,
        "ext.user",
        &format!("{WS}get#File"),
        &[("rootId", "my-site"), ("path", path)],
    )
    .unwrap();
    call(
        &factory,
        "ext.user",
        &format!("{WS}save#File"),
        &[
            ("rootId", "my-site"),
            ("path", path),
            ("text", &changed.replace("other", "other2")),
            ("baseHash", &text(&ext_file, "hash")),
        ],
    )
    .unwrap();
    let ext_refused = call(
        &factory,
        "ext.user",
        &format!("{WS}set#UpstreamEdits"),
        &[("enabled", "true")],
    )
    .unwrap_err();
    assert!(ext_refused.contains("full tier"), "{ext_refused}");

    // the full tier turns the lock off: the upstream save goes through, with the upstream-edit warning
    call(
        &factory,
        "john.doe",
        &format!("{WS}set#UpstreamEdits"),
        &[("enabled", "true")],
    )
    .unwrap();
    let forked = call(
        &factory,
        "john.doe",
        &format!("{WS}save#File"),
        &[
            ("rootId", "upstream-app"),
            ("path", "service/upapp/Up.xml"),
            ("text", &UPSTREAM_SERVICES.replace("Up\"", "Up2\"")),
            ("baseHash", &text(&up, "hash")),
        ],
    )
    .unwrap();
    assert!(moqui_lib::json::to_json(&forked["problems"])
        .to_string()
        .contains("upstream.edit"));
    call(
        &factory,
        "john.doe",
        &format!("{WS}set#UpstreamEdits"),
        &[("enabled", "false")],
    )
    .unwrap();

    // paths stay in their root
    for bad in ["../../etc/passwd", ".git/config", "/etc/passwd"] {
        let err = call(
            &factory,
            "john.doe",
            &format!("{WS}get#File"),
            &[("rootId", "my-site"), ("path", bad)],
        )
        .unwrap_err();
        assert!(!err.is_empty(), "{bad}");
    }

    // a CRLF file keeps its line endings through a save
    std::fs::write(site.join("data.txt"), "one\r\ntwo\r\n").unwrap();
    let crlf = call(
        &factory,
        "john.doe",
        &format!("{WS}get#File"),
        &[("rootId", "my-site"), ("path", "data.txt")],
    )
    .unwrap();
    assert_eq!(text(&crlf, "lineEnding"), "CRLF");
    call(
        &factory,
        "john.doe",
        &format!("{WS}save#File"),
        &[
            ("rootId", "my-site"),
            ("path", "data.txt"),
            ("text", "one\ntwo\nthree\n"),
            ("baseHash", &text(&crlf, "hash")),
        ],
    )
    .unwrap();
    assert_eq!(
        std::fs::read(site.join("data.txt")).unwrap(),
        b"one\r\ntwo\r\nthree\r\n"
    );

    // every write is audited
    let audits = {
        let context = factory.new_context();
        let _entered = context.enter();
        let _authz = context.disable_authz();
        factory
            .entity()
            .unwrap()
            .find("moqui.dev.DevAideAudit")
            .unwrap()
            .list()
            .unwrap()
            .len()
    };
    assert!(audits >= 4, "{audits} audit rows");

    // commit: the user as author, the server as committer, the task trailer
    call(
        &factory,
        "john.doe",
        &format!("{VCS}stage#Files"),
        &[("rootId", "my-site"), ("paths", path)],
    )
    .unwrap();
    let status = call(
        &factory,
        "john.doe",
        &format!("{VCS}get#Status"),
        &[("rootId", "my-site")],
    )
    .unwrap();
    assert_eq!(text(&status, "authorName"), "John Doe");
    let committed = call(
        &factory,
        "john.doe",
        &format!("{VCS}commit#Changes"),
        &[("rootId", "my-site"), ("summary", "Add a service"), ("task", "PROJ-12")],
    )
    .unwrap();
    let id = text(&committed, "commitId");
    assert_eq!(
        git(&site, &["log", "-1", "--format=%an|%cn|%s"]).trim(),
        "John Doe|Dev Server|Add a service"
    );
    assert!(git(&site, &["log", "-1", "--format=%B"]).contains("Task: PROJ-12"));
    assert_eq!(git(&site, &["rev-parse", "HEAD"]).trim(), id);

    // Publish: the rest of the site's changes committed and pushed to the remote
    let published = call(
        &factory,
        "john.doe",
        &format!("{VCS}publish#Site"),
        &[("rootId", "my-site")],
    )
    .unwrap();
    assert_eq!(text(&published, "published"), "true", "{published:?}");
    let remote = gate.runtime.join("site-remote.git");
    assert_eq!(
        git(&remote, &["rev-parse", "main"]).trim(),
        git(&site, &["rev-parse", "HEAD"]).trim()
    );
    assert!(git(&site, &["log", "-1", "--format=%s"]).starts_with("Customize: data.txt"));

    // with a publish branch an admin set for the user, Publish pushes there and says which branch to merge
    {
        let context = factory.new_context();
        let _entered = context.enter();
        moqui_core::user::internal_login_user("john.doe").unwrap();
        moqui_core::user::set_preference("DEV_AIDE_PUBLISH_BRANCH", Some("publish/{user}")).unwrap();
    }
    std::fs::write(site.join("data.txt"), "four\r\n").unwrap();
    let branched = call(
        &factory,
        "john.doe",
        &format!("{VCS}publish#Site"),
        &[("rootId", "my-site")],
    )
    .unwrap();
    assert_eq!(text(&branched, "pushedBranch"), "publish/john.doe");
    assert!(
        text(&branched, "message").contains("merge publish/john.doe into main"),
        "{branched:?}"
    );
    assert_eq!(
        git(&remote, &["rev-parse", "publish/john.doe"]).trim(),
        git(&site, &["rev-parse", "HEAD"]).trim()
    );

    // a shared server with its network git off: the push is refused, nothing goes out
    moqui_lib::env::set_property("dev_aide_shared", "true");
    let shared = call(
        &factory,
        "john.doe",
        &format!("{VCS}push#Remote"),
        &[("rootId", "my-site")],
    )
    .unwrap_err();
    moqui_lib::env::set_property("dev_aide_shared", "false");
    assert!(shared.contains("off on this shared dev server"), "{shared}");
}
