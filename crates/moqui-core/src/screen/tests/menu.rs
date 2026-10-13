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

//! The menu data of the `menuData` transition (`sri.getMenuData`): the user-specific menu as JSON, with the
//! pre-actions of the screens on the path run for their titles and the subscreens the user may not view dropped.

use std::path::PathBuf;

use super::{basic, request, World};
use crate::screen::serve_at;

fn screen_authz_conf() -> &'static str {
    "<moqui-conf><artifact-execution-facade><artifact-execution type=\"AT_XML_SCREEN\" authz-enabled=\"true\"/></artifact-execution-facade></moqui-conf>"
}

/// A root screen with the transition of webroot.xml, an app with an open screen, a screen only the admin group may
/// view, and a screen that takes a required parameter. The open screen's pre-actions set the title its menu item shows.
fn tree(name: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("moqui-menu-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Root/App")).unwrap();
    let base = dir.to_string_lossy().into_owned();
    let write = |path: &str, text: &str| std::fs::write(dir.join(path), text.replace("{dir}", &base)).unwrap();
    write(
        "Root.xml",
        r#"<screen require-authentication="false">
            <transition name="menuData" read-only="true"><actions><script><![CDATA[
                const menuDataList = sri.getMenuData(sri.getScreenUrlInfo().extraPathNameList)
                if (menuDataList != null) ec.web.sendJsonResponse(menuDataList)
            ]]></script></actions><default-response type="none"/></transition>
            <subscreens default-item="App"/>
            <widgets><subscreens-active/></widgets>
        </screen>"#,
    );
    write(
        "Root/App.xml",
        r#"<screen require-authentication="true" default-menu-title="The App">
            <subscreens default-item="Open">
                <subscreens-item name="Open" menu-title="${openTitle}" location="{dir}/Root/App/Open.xml"/>
                <subscreens-item name="Secret" menu-title="Secret" location="{dir}/Root/App/Secret.xml"/>
                <subscreens-item name="Needs" menu-title="Needs" location="{dir}/Root/App/Needs.xml"/>
            </subscreens>
            <widgets><subscreens-active/></widgets>
        </screen>"#,
    );
    write(
        "Root/App/Open.xml",
        r#"<screen require-authentication="true" default-menu-title="Open">
            <pre-actions><set field="openTitle" value="Open Sesame"/></pre-actions>
            <widgets><label text="open"/></widgets></screen>"#,
    );
    write(
        "Root/App/Secret.xml",
        r#"<screen require-authentication="true" default-menu-title="Secret"><widgets><label text="secret"/></widgets></screen>"#,
    );
    write(
        "Root/App/Needs.xml",
        r#"<screen require-authentication="true" default-menu-title="Needs"><parameter name="needId" required="true"/><widgets/></screen>"#,
    );
    let root = dir.join("Root.xml").to_string_lossy().into_owned();
    (dir, root)
}

/// Allow `group` to view the screen at `location`, in an artifact group of its own.
fn allow(world: &World, id: &str, group: &str, location: &str) {
    use moqui_lib::object::Value;
    let mut member = world.facade.make_value("moqui.security.ArtifactGroupMember").unwrap();
    member.set("artifactGroupId", Value::string(id));
    member.set("artifactName", Value::string(location));
    member.set("artifactTypeEnumId", Value::string("AT_XML_SCREEN"));
    member.set("nameIsPattern", Value::string("N"));
    member.set("inheritAuthz", Value::string("Y"));
    member.create(&world.facade).unwrap();
    let mut authz = world.facade.make_value("moqui.security.ArtifactAuthz").unwrap();
    authz.set("artifactAuthzId", Value::string(id));
    authz.set("userGroupId", Value::string(group));
    authz.set("artifactGroupId", Value::string(id));
    authz.set("authzTypeEnumId", Value::string("AUTHZT_ALWAYS"));
    authz.set("authzActionEnumId", Value::string("AUTHZA_VIEW"));
    authz.create(&world.facade).unwrap();
}

fn get_menu(world: &World, root: &str, user: Option<&str>, path: &str) -> (u16, serde_json::Value) {
    let mut get = request("GET", path);
    if let Some(user) = user {
        get.set_header("authorization", basic(user));
    }
    let response = serve_at(&world.factory, get, root);
    let json = serde_json::from_str(&response.body_text()).unwrap_or(serde_json::Value::Null);
    (response.status, json)
}

#[test]
fn menu_data_lists_what_the_user_may_view_and_runs_the_pre_actions_for_titles() {
    let (dir, root) = tree("view");
    let world = World::open(screen_authz_conf());
    world.user("admin", Some("ADMIN"));
    world.user("plain", None);
    // a grant on a screen reaches the screens below it, so the app itself has none
    allow(
        &world,
        "OPEN_VIEW",
        "ALL_USERS",
        &dir.join("Root/App/Open.xml").to_string_lossy(),
    );
    allow(
        &world,
        "SECRET_VIEW",
        "ADMIN",
        &dir.join("Root/App/Secret.xml").to_string_lossy(),
    );
    // the app's subscreens, with the ones the files add
    let names = |menu: &serde_json::Value| -> Vec<String> {
        menu[0]["subscreens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["name"].as_str().unwrap().to_string())
            .collect()
    };

    let (status, menu) = get_menu(&world, &root, Some("plain"), "/menuData/App/Open");
    assert_eq!(status, 200, "{menu}");
    let items = names(&menu);
    assert!(items.contains(&"Open".to_string()), "{items:?}");
    assert!(!items.contains(&"Secret".to_string()), "{items:?}");
    assert!(
        !items.contains(&"Needs".to_string()),
        "a missing required parameter drops the item: {items:?}"
    );
    // the title was set by the pre-actions of the screens on the path (the first two, the root and the app, are skipped), and the last item is the target
    let open = menu[0]["subscreens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "Open")
        .unwrap();
    assert_eq!(open["title"], "Open Sesame", "{menu}");
    assert_eq!(open["active"], true);
    assert_eq!(menu[0]["name"], "App");
    assert_eq!(menu[0]["title"], "The App");
    assert_eq!(menu[1]["name"], "Open");
    assert_eq!(menu[1]["path"], "/App/Open");

    let (_, admin) = get_menu(&world, &root, Some("admin"), "/menuData/App/Open");
    assert!(names(&admin).contains(&"Secret".to_string()), "{admin}");

    // a screen the user may not view is 403, nobody is 401, and no such screen is 404
    let (status, _) = get_menu(&world, &root, Some("plain"), "/menuData/App/Secret");
    assert_eq!(status, 403);
    let (status, _) = get_menu(&world, &root, None, "/menuData/App/Open");
    assert_eq!(status, 401);
    let (status, _) = get_menu(&world, &root, Some("admin"), "/menuData/App/Nothing");
    assert_eq!(status, 404);
    let _ = std::fs::remove_dir_all(&dir);
}
