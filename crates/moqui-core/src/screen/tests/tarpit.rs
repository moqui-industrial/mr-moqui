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

//! Screen and transition hits against the tarpits, as Java's `ScreenDefinition.render` and `TransitionItem.run` count
//! them: the target screen when it requires authentication, every transition, and nothing of a screen test.

use std::path::{Path, PathBuf};

use moqui_lib::object::Value;

use super::{basic, request, World};
use crate::screen::{serve_at, ScreenTest};
use crate::web::WebResponse;

/// The limit of the test tarpits: the hit past it locks the artifact, the next is refused.
const LIMIT: usize = 3;

fn conf(screen_tarpit: bool) -> String {
    format!(
        "<moqui-conf><artifact-execution-facade>\
         <artifact-execution type=\"AT_XML_SCREEN\" authz-enabled=\"false\" tarpit-enabled=\"{screen_tarpit}\"/>\
         <artifact-execution type=\"AT_XML_SCREEN_TRANS\" authz-enabled=\"false\" tarpit-enabled=\"true\"/>\
         </artifact-execution-facade></moqui-conf>"
    )
}

/// A screen tree: the root (no authentication), a target that requires it with a transition, an anonymous screen with a
/// transition, a parent with a child, and a screen that includes another.
fn screens(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("moqui-screen-tarpit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Root/Parent")).unwrap();
    let write = |path: &str, text: &str| std::fs::write(dir.join(path), text).unwrap();
    write(
        "Root.xml",
        r#"<screen require-authentication="false"><widgets><subscreens-active/></widgets></screen>"#,
    );
    write(
        "Root/Target.xml",
        r#"<screen><transition name="doIt"><default-response type="none"/></transition>
            <widgets><label text="target"/></widgets></screen>"#,
    );
    write(
        "Root/Open.xml",
        r#"<screen require-authentication="anonymous-view"><transition name="doIt"><default-response type="none"/></transition>
            <widgets><label text="open"/></widgets></screen>"#,
    );
    write(
        "Root/Parent.xml",
        r#"<screen><widgets><label text="parent"/><subscreens-active/></widgets></screen>"#,
    );
    write(
        "Root/Parent/Child.xml",
        r#"<screen><widgets><label text="child"/></widgets></screen>"#,
    );
    write(
        "Root/Includer.xml",
        &format!(
            r#"<screen><widgets><label text="includer"/><include-screen location="{}"/></widgets></screen>"#,
            dir.join("Included.xml").display()
        ),
    );
    write(
        "Included.xml",
        r#"<screen><widgets><label text="included"/></widgets></screen>"#,
    );
    dir
}

fn location(dir: &Path, path: &str) -> String {
    dir.join(path).to_string_lossy().into_owned()
}

/// A world with the user `john` and a tarpit of [`LIMIT`] hits a minute on every screen and transition for all users.
fn world(screen_tarpit: bool) -> World {
    let world = World::open(&conf(screen_tarpit));
    world.user("john", None);
    for (group, artifact_type) in [("TP_SCREENS", "AT_XML_SCREEN"), ("TP_TRANS", "AT_XML_SCREEN_TRANS")] {
        let mut row = world.facade.make_value("moqui.security.ArtifactGroup").unwrap();
        row.set("artifactGroupId", Value::string(group));
        row.create(&world.facade).unwrap();
        let mut member = world.facade.make_value("moqui.security.ArtifactGroupMember").unwrap();
        member.set("artifactGroupId", Value::string(group));
        member.set("artifactName", Value::string(".*"));
        member.set("nameIsPattern", Value::string("Y"));
        member.set("artifactTypeEnumId", Value::string(artifact_type));
        member.create(&world.facade).unwrap();
        let mut tarpit = world.facade.make_value("moqui.security.ArtifactTarpit").unwrap();
        tarpit.set("userGroupId", Value::string("ALL_USERS"));
        tarpit.set("artifactGroupId", Value::string(group));
        tarpit.set("maxHitsCount", Value::I64(LIMIT as i64));
        tarpit.set("maxHitsDuration", Value::I64(60));
        tarpit.set("tarpitDuration", Value::I64(60));
        tarpit.create(&world.facade).unwrap();
    }
    world
}

fn send(world: &World, root: &str, method: &str, path: &str) -> WebResponse {
    let mut req = request(method, path);
    req.set_header("authorization", basic("john"));
    serve_at(&world.factory, req, root)
}

/// The hits counted for `john` on the artifact, within the tarpit window.
fn hits(world: &World, artifact_type: &str, name: &str) -> usize {
    world
        .factory
        .tarpit_times(&format!("john@{artifact_type}:{name}"))
        .len()
}

#[test]
fn the_target_screen_counts_and_past_its_limit_is_429_with_retry_after() {
    let world = world(true);
    let dir = screens("target");
    let root = location(&dir, "Root.xml");
    let target = location(&dir, "Root/Target.xml");
    for count in 1..=LIMIT {
        let response = send(&world, &root, "GET", "/Target");
        assert_eq!(response.status, 200, "{}", response.body_text());
        assert_eq!(hits(&world, "AT_XML_SCREEN", &target), count);
    }
    // The root above the target is not counted.
    assert_eq!(hits(&world, "AT_XML_SCREEN", &root), 0);
    // The hit past the limit sets the lock; the next is refused.
    assert_eq!(send(&world, &root, "GET", "/Target").status, 200);
    let refused = send(&world, &root, "GET", "/Target");
    assert_eq!(refused.status, 429, "{}", refused.body_text());
    let retry: u64 = refused
        .header("Retry-After")
        .unwrap_or_else(|| panic!("no Retry-After: {:?} {}", refused.headers, refused.body_text()))
        .parse()
        .unwrap();
    assert!(retry > 0 && retry <= 60, "Retry-After {retry}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_target_anonymous_and_included_screens_do_not_count() {
    let world = world(true);
    let dir = screens("others");
    let root = location(&dir, "Root.xml");
    for _ in 0..LIMIT + 3 {
        assert_eq!(send(&world, &root, "GET", "/Open").status, 200);
    }
    assert_eq!(hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Open.xml")), 0);
    for count in 1..=LIMIT {
        let response = send(&world, &root, "GET", "/Parent/Child");
        assert_eq!(response.status, 200, "{}", response.body_text());
        assert_eq!(
            hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Parent/Child.xml")),
            count
        );
        let response = send(&world, &root, "GET", "/Includer");
        assert_eq!(response.status, 200, "{}", response.body_text());
        assert!(response.body_text().contains("included"), "{}", response.body_text());
        assert_eq!(
            hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Includer.xml")),
            count
        );
    }
    assert_eq!(hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Parent.xml")), 0);
    assert_eq!(hits(&world, "AT_XML_SCREEN", &location(&dir, "Included.xml")), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_transition_counts_on_any_screen_and_its_screens_do_not() {
    let world = world(true);
    let dir = screens("transition");
    let root = location(&dir, "Root.xml");
    let target = location(&dir, "Root/Target.xml");
    for count in 1..=LIMIT {
        let response = send(&world, &root, "POST", "/Target/doIt");
        assert!(response.status < 400, "{} {}", response.status, response.body_text());
        assert_eq!(hits(&world, "AT_XML_SCREEN_TRANS", &format!("{target}/doIt")), count);
        // Java counts a transition whatever its screen's authentication.
        let response = send(&world, &root, "POST", "/Open/doIt");
        assert!(response.status < 400, "{} {}", response.status, response.body_text());
        let open = location(&dir, "Root/Open.xml");
        assert_eq!(hits(&world, "AT_XML_SCREEN_TRANS", &format!("{open}/doIt")), count);
    }
    assert_eq!(hits(&world, "AT_XML_SCREEN", &target), 0);
    assert!(send(&world, &root, "POST", "/Target/doIt").status < 400);
    assert_eq!(send(&world, &root, "POST", "/Target/doIt").status, 429);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tarpit_enabled_false_turns_screen_counting_off() {
    let world = world(false);
    let dir = screens("off");
    let root = location(&dir, "Root.xml");
    for _ in 0..LIMIT + 3 {
        assert_eq!(send(&world, &root, "GET", "/Target").status, 200);
    }
    assert_eq!(hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Target.xml")), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_screen_test_render_is_not_counted() {
    let world = world(true);
    let dir = screens("screen-test");
    let root = location(&dir, "Root.xml");
    let context = world.factory.new_context();
    let _entered = context.enter();
    crate::user::internal_login_user("john").unwrap();
    let test = ScreenTest::new(&world.factory).root_screen(root.clone());
    for _ in 0..LIMIT + 3 {
        let rendered = test.test_render("Target", Default::default(), None);
        assert!(rendered.error_messages.is_empty(), "{:?}", rendered.error_messages);
        assert!(rendered.text().contains("target"), "{}", rendered.text());
    }
    assert_eq!(hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Target.xml")), 0);
    assert!(!context.tarpit_disabled(), "the test render enables the tarpit again");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A page of the Vue client fetches a `.qvt` per screen it shows; each counts its own target once, and the screens a
/// render shows inside it (its parents on the path) are not counted with it.
#[test]
fn a_qvt_render_counts_its_target_once() {
    let world = world(true);
    let dir = screens("qvt");
    let root = location(&dir, "Root.xml");
    for count in 1..=LIMIT {
        let response = send(&world, &root, "GET", "/Parent/Child.qvt");
        assert_eq!(response.status, 200, "{}", response.body_text());
        assert_eq!(
            hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Parent/Child.xml")),
            count
        );
    }
    assert_eq!(hits(&world, "AT_XML_SCREEN", &location(&dir, "Root/Parent.xml")), 0);
    assert_eq!(hits(&world, "AT_XML_SCREEN", &root), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// webroot renders the screens below it from `<text template="true">${sri.renderSubscreen()}</text>`: the refusal
/// crosses the template and the scripts around it and is still a 429 with `Retry-After`.
#[test]
fn a_refusal_under_a_template_render_is_still_429() {
    let world = world(true);
    let dir = screens("template");
    std::fs::write(
        dir.join("Root.xml"),
        r#"<screen require-authentication="false"><widgets>
            <render-mode><text template="true">${sri.renderSubscreen()}</text></render-mode></widgets></screen>"#,
    )
    .unwrap();
    let root = location(&dir, "Root.xml");
    for _ in 0..=LIMIT {
        let response = send(&world, &root, "GET", "/Target");
        assert_eq!(response.status, 200, "{}", response.body_text());
    }
    let refused = send(&world, &root, "GET", "/Target");
    assert_eq!(refused.status, 429, "{}", refused.body_text());
    assert!(refused.header("Retry-After").is_some(), "{:?}", refused.headers);
    let _ = std::fs::remove_dir_all(&dir);
}
