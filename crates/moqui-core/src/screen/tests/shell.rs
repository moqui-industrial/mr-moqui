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

//! The suite shell's data (`shellData`, `sri.getShellData`), the palette search (`shellSearch`, `sri.getShellSearch`)
//! and `org.moqui.impl.ShellServices`: the apps the user may open with their menus and `<shell-providers>`, an app root
//! that fails logged and left out, the palette sources of those apps only, a failing source logged and left out, the
//! theme values of the user's qapps theme, and the counts sent on the `shellCounts` topic, coalesced per user.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moqui_lib::object::Value;
use serde_json::json;

use super::{basic, request, World};
use crate::screen::serve_at;

fn screen_authz_conf() -> &'static str {
    "<moqui-conf><artifact-execution-facade><artifact-execution type=\"AT_XML_SCREEN\" authz-enabled=\"true\"/></artifact-execution-facade></moqui-conf>"
}

/// A root screen with the `shellData` and `shellSearch` transitions of webroot.xml, and its apps root.
const ROOT: &str = r#"<screen require-authentication="false">
    <transition name="shellData" read-only="true" begin-transaction="false"><actions><script><![CDATA[
        const shellData = sri.getShellData()
        if (shellData != null) ec.web.sendJsonResponse(shellData)
    ]]></script></actions><default-response type="none"/></transition>
    <transition name="shellSearch" read-only="true" begin-transaction="false"><actions><script><![CDATA[
        const params = ec.web.parameters
        const found = sri.getShellSearch(params.text, params.groups, params.limit)
        if (found != null) ec.web.sendJsonResponse(found)
    ]]></script></actions><default-response type="none"/></transition>
    <subscreens default-item="apps"/>
    <widgets><subscreens-active/></widgets>
</screen>"#;

/// The palette sources, the today source and the counts services the providers and themes name.
const SERVICES: &str = r#"<services>
    <service verb="get" noun="Counts" authenticate="false">
        <in-parameters><parameter name="userId"/></in-parameters>
        <out-parameters><parameter name="counts" type="Map"/></out-parameters>
        <actions><script>counts = {today: 3, chat: 1, user: userId}</script></actions>
    </service>
    <service verb="get" noun="OtherCounts" authenticate="false">
        <in-parameters><parameter name="userId"/></in-parameters>
        <out-parameters><parameter name="counts" type="Map"/></out-parameters>
        <actions><script>counts = {other: true}</script></actions>
    </service>
    <service verb="get" noun="ZonedCounts">
        <description>authenticate and authorization as a Mantle service has them: no app's authorization covers it</description>
        <in-parameters><parameter name="userId"/></in-parameters>
        <out-parameters><parameter name="counts" type="Map"/></out-parameters>
        <actions><script>counts = {zone: ec.user.timeZone, locale: ec.user.locale, user: userId}</script></actions>
    </service>
    <service verb="get" noun="BadCounts" authenticate="false">
        <in-parameters><parameter name="userId"/></in-parameters>
        <out-parameters><parameter name="counts" type="Map"/></out-parameters>
        <actions><script>throw new Error('the counts are down')</script></actions>
    </service>
    <service verb="search" noun="Records" authenticate="false">
        <in-parameters><parameter name="text"/><parameter name="limit" type="Integer"/><parameter name="docTypes" type="List"/>
            <parameter name="idMatch" type="Boolean"/></in-parameters>
        <out-parameters><parameter name="rows" type="List"/><parameter name="moreUrl"/></out-parameters>
        <actions><script><![CDATA[
            rows = []
            for (let i = 0; i < 10; i++) rows.push({title: text + ' ' + i, subtitle: (docTypes || []).join(',') + (idMatch ? ' id' : ''),
                icon: 'sym_o_label', url: '/qapps/open/List?i=' + i})
            moreUrl = '/qapps/open/List?text=' + text
        ]]></script></actions>
    </service>
    <service verb="search" noun="Stack" authenticate="false">
        <in-parameters><parameter name="text"/><parameter name="limit" type="Integer"/><parameter name="docTypes" type="List"/>
            <parameter name="idMatch" type="Boolean"/></in-parameters>
        <out-parameters><parameter name="rows" type="List"/></out-parameters>
        <actions><script>rows = [{title: 'stack', subtitle: ec.artifactExecution.getStackNameString()}]</script></actions>
    </service>
    <service verb="search" noun="Ids" authenticate="false">
        <in-parameters><parameter name="text"/><parameter name="limit" type="Integer"/><parameter name="docTypes" type="List"/>
            <parameter name="idMatch" type="Boolean"/></in-parameters>
        <out-parameters><parameter name="rows" type="List"/></out-parameters>
        <actions><script>rows = [{title: 'Order ' + text, subtitle: idMatch ? 'id' : 'text', url: '/qapps/open/List?id=' + text}]</script></actions>
    </service>
    <service verb="search" noun="Fail" authenticate="false">
        <in-parameters><parameter name="text"/></in-parameters>
        <out-parameters><parameter name="rows" type="List"/></out-parameters>
        <actions><script>throw new Error('the source is down')</script></actions>
    </service>
    <service verb="search" noun="Empty" authenticate="false">
        <in-parameters><parameter name="text"/></in-parameters>
        <out-parameters><parameter name="rows" type="List"/></out-parameters>
        <actions><script>rows = []</script></actions>
    </service>
    <service verb="search" noun="Secret" authenticate="false">
        <in-parameters><parameter name="text"/></in-parameters>
        <out-parameters><parameter name="rows" type="List"/></out-parameters>
        <actions><script>rows = [{title: 'secret ' + text, url: '/qapps/secret'}]</script></actions>
    </service>
</services>"#;

/// [`ROOT`] and an apps root with: `open`, which everyone may view, with a menu three deep and every kind of provider;
/// `secret`, for the ADMIN group, with a palette source; `broken`, which does not parse; `needs`, which takes a parameter;
/// and `hidden`, out of the menu. The services are under `service/test`.
fn tree(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("moqui-shell-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for path in ["Root", "Apps/Open/Sub/Deep1", "service/test"] {
        std::fs::create_dir_all(dir.join(path)).unwrap();
    }
    let base = dir.to_string_lossy().into_owned();
    let write = |path: &str, text: &str| std::fs::write(dir.join(path), text.replace("{dir}", &base)).unwrap();
    write("Root.xml", ROOT);
    write(
        "Root/apps.xml",
        r#"<screen require-authentication="false" default-menu-title="Applications">
            <subscreens default-item="open">
                <subscreens-item name="open" menu-title="Open App" menu-index="1" location="{dir}/Apps/Open.xml"/>
                <subscreens-item name="secret" menu-title="Secret App" menu-index="2" location="{dir}/Apps/Secret.xml"/>
                <subscreens-item name="broken" menu-title="Broken App" menu-index="3" location="{dir}/Apps/Broken.xml"/>
                <subscreens-item name="needs" menu-title="Needs App" menu-index="4" location="{dir}/Apps/Needs.xml"/>
                <subscreens-item name="hidden" menu-title="Hidden App" menu-index="5" menu-include="false" location="{dir}/Apps/Hidden.xml"/>
            </subscreens>
            <widgets><subscreens-active/></widgets>
        </screen>"#,
    );
    write(
        "Apps/Open.xml",
        r#"<screen require-authentication="true" default-menu-title="Open App" menu-image="sym_o_hub" menu-image-type="icon">
            <subscreens default-item="List">
                <subscreens-item name="List" menu-title="List" menu-index="1" location="{dir}/Apps/Open/List.xml"/>
                <subscreens-item name="Detail" menu-title="Detail" menu-index="2" location="{dir}/Apps/Open/Detail.xml"/>
                <subscreens-item name="Sub" menu-title="Sub" menu-index="3" location="{dir}/Apps/Open/Sub.xml"/>
                <subscreens-item name="QuickNote" menu-include="false" location="{dir}/Apps/Open/QuickNote.xml"/>
                <subscreens-item name="QuickCreate" menu-include="false" location="{dir}/Apps/Open/QuickCreate.xml"/>
            </subscreens>
            <shell-providers>
                <quick-create kind="note" label="Quick note" icon="sym_o_note_add" screen="open/QuickNote" sequence="20">
                    <parameter name="from" value="the shell"/></quick-create>
                <quick-create kind="task" label="Task" sequence="10"/>
                <quick-create kind="secret" label="Secret thing" screen="secret"/>
                <quick-create kind="lost" label="Lost" screen="open/NoSuchScreen"/>
                <quick-create kind="activity" label="Activity" requires-record="true" sequence="30"/>
                <today-source service="test.Shell.get#TodayItems" label="Open items"/>
                <palette-action label="Open the list" icon="sym_o_list" screen="open/List"/>
                <palette-type doc-type="OpenDoc"/>
                <palette-source group="Fail" label="Failing" service="test.Shell.search#Fail" sequence="10"/>
                <palette-source group="Records" label="Records" icon="sym_o_search" service="test.Shell.search#Records" prefix="~" sequence="20"/>
                <palette-source group="Ids" label="Id lookup" service="test.Shell.search#Ids" id-pattern="[A-Z]{2}[0-9]+" sequence="5"/>
                <palette-source group="Stack" service="test.Shell.search#Stack" id-pattern="STACK" sequence="40"/>
                <palette-source group="Empty" service="test.Shell.search#Empty" sequence="30"/>
                <notification-topic topic="OpenAlert"/>
                <keymap scope="open">
                    <key keys="g l" label="Go to the list" screen="open/List"/>
                    <key keys="c" label="Create" action="quickCreate"/>
                    <key keys="g s" label="Secret" screen="secret"/>
                </keymap>
                <account-menu-item label="Preferences" icon="sym_o_tune" screen="open/List"/>
                <frobnicate/>
            </shell-providers>
            <widgets><subscreens-active/></widgets>
        </screen>"#,
    );
    let plain = |title: &str| {
        format!(
            r#"<screen require-authentication="true" default-menu-title="{title}"><widgets><label text="{title}"/></widgets></screen>"#
        )
    };
    write("Apps/Open/List.xml", &plain("List"));
    write("Apps/Open/QuickNote.xml", &plain("Quick note"));
    write("Apps/Open/QuickCreate.xml", &plain("Quick create"));
    write(
        "Apps/Open/Detail.xml",
        r#"<screen require-authentication="true" default-menu-title="Detail"><parameter name="detailId" required="true"/><widgets/></screen>"#,
    );
    write(
        "Apps/Open/Sub.xml",
        r#"<screen require-authentication="true" default-menu-title="Sub">
            <subscreens default-item="Deep1"><subscreens-item name="Deep1" menu-title="Deep one" location="{dir}/Apps/Open/Sub/Deep1.xml"/></subscreens>
            <widgets><subscreens-active/></widgets></screen>"#,
    );
    write(
        "Apps/Open/Sub/Deep1.xml",
        r#"<screen require-authentication="true" default-menu-title="Deep one">
            <subscreens default-item="Deep2"><subscreens-item name="Deep2" menu-title="Deep two" location="{dir}/Apps/Open/Sub/Deep1/Deep2.xml"/></subscreens>
            <widgets><subscreens-active/></widgets></screen>"#,
    );
    std::fs::create_dir_all(dir.join("Apps/Open/Sub/Deep1")).unwrap();
    write("Apps/Open/Sub/Deep1/Deep2.xml", &plain("Deep two"));
    write(
        "Apps/Secret.xml",
        r#"<screen require-authentication="true" default-menu-title="Secret App">
            <shell-providers>
                <palette-type doc-type="SecretDoc"/>
                <palette-source group="Secret" service="test.Shell.search#Secret"/>
            </shell-providers>
            <widgets><label text="secret"/></widgets></screen>"#,
    );
    write(
        "Apps/Broken.xml",
        r#"<screen require-authentication="false"><widgets><label text="broken"></widgets></screen>"#,
    );
    write(
        "Apps/Needs.xml",
        r#"<screen require-authentication="false"><parameter name="needId"/><widgets/></screen>"#,
    );
    write("Apps/Hidden.xml", &plain("Hidden"));
    write("service/test/Shell.xml", SERVICES);
    dir
}

fn root_of(dir: &std::path::Path) -> String {
    dir.join("Root.xml").to_string_lossy().into_owned()
}

/// Allow `group` to view the screen at `location`, in an artifact group of its own.
fn allow(world: &World, id: &str, group: &str, location: &str) {
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

fn row(world: &World, entity: &str, fields: &[(&str, Value)]) {
    let mut value = world.facade.make_value(entity).unwrap();
    for (name, field) in fields {
        value.set(*name, field.clone());
    }
    value.create(&world.facade).unwrap();
}

fn theme_value(world: &World, theme: &str, sequence: i64, kind: &str, value: &str) {
    row(
        world,
        "moqui.screen.ScreenThemeResource",
        &[
            ("screenThemeId", Value::string(theme)),
            ("sequenceNum", Value::I64(sequence)),
            ("resourceTypeEnumId", Value::string(kind)),
            ("resourceValue", Value::string(value)),
        ],
    );
}

/// The users (plain; admin in ADMIN; other with a theme of its own; grouped in a group with a theme), the screen
/// grants, the themes and the services of [`tree`].
fn world(dir: &std::path::Path) -> World {
    world_with(dir, screen_authz_conf())
}

/// [`world`] with authorization enabled for screens and services, as the default configuration has it.
fn service_authz_world(dir: &std::path::Path) -> World {
    world_with(
        dir,
        "<moqui-conf><artifact-execution-facade><artifact-execution type=\"AT_XML_SCREEN\" authz-enabled=\"true\"/>\
         <artifact-execution type=\"AT_SERVICE\" authz-enabled=\"true\"/></artifact-execution-facade></moqui-conf>",
    )
}

fn world_with(dir: &std::path::Path, conf: &str) -> World {
    let world = World::open(conf);
    world.user("plain", None);
    world.user("admin", Some("ADMIN"));
    world.user("other", None);
    world.user("grouped", Some("COUNTERS"));
    allow(
        &world,
        "OPEN_VIEW",
        "ALL_USERS",
        &dir.join("Apps/Open.xml").to_string_lossy(),
    );
    allow(
        &world,
        "SECRET_VIEW",
        "ADMIN",
        &dir.join("Apps/Secret.xml").to_string_lossy(),
    );
    // No ScreenTheme of the Quasar type is in this database, so a user with no theme of their own gets DEFAULT_QUASAR.
    theme_value(&world, "DEFAULT_QUASAR", 1, "STRT_TRAY_TODAY", "open/List");
    theme_value(&world, "DEFAULT_QUASAR", 2, "STRT_TRAY_CHAT", "secret");
    theme_value(&world, "DEFAULT_QUASAR", 3, "STRT_QUICK_CREATE", "open/QuickCreate");
    theme_value(&world, "DEFAULT_QUASAR", 4, "STRT_CHATTER_SCREEN", "Chatter");
    theme_value(
        &world,
        "DEFAULT_QUASAR",
        5,
        "STRT_SHELL_COUNTS_SERVICE",
        "test.Shell.get#Counts",
    );
    theme_value(
        &world,
        "OTHER_QUASAR",
        1,
        "STRT_SHELL_COUNTS_SERVICE",
        "test.Shell.get#OtherCounts",
    );
    theme_value(
        &world,
        "GROUP_QUASAR",
        1,
        "STRT_SHELL_COUNTS_SERVICE",
        "test.Shell.get#BadCounts",
    );
    theme_value(
        &world,
        "ZONED_QUASAR",
        1,
        "STRT_SHELL_COUNTS_SERVICE",
        "test.Shell.get#ZonedCounts",
    );
    theme_value(
        &world,
        "MISSING_QUASAR",
        1,
        "STRT_SHELL_COUNTS_SERVICE",
        "test.Shell.get#NoSuchCounts",
    );
    row(
        &world,
        "moqui.security.UserScreenTheme",
        &[
            ("userId", Value::string("other")),
            ("screenThemeTypeEnumId", Value::string("STT_INTERNAL_QUASAR")),
            ("screenThemeId", Value::string("OTHER_QUASAR")),
        ],
    );
    row(
        &world,
        "moqui.security.UserGroupScreenTheme",
        &[
            ("userGroupId", Value::string("COUNTERS")),
            ("screenThemeTypeEnumId", Value::string("STT_INTERNAL_QUASAR")),
            ("screenThemeId", Value::string("GROUP_QUASAR")),
            ("sequenceNum", Value::I64(1)),
        ],
    );
    row(
        &world,
        "moqui.basic.LocalizedMessage",
        &[
            ("original", Value::string("Quick note")),
            ("locale", Value::string("en")),
            ("localized", Value::string("Quick note (en)")),
        ],
    );
    let services = [
        dir.join("service/test/Shell.xml"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../runtime/base-component/framework/service/org/moqui/impl/ShellServices.xml"),
    ];
    {
        let ctx = world.factory.new_context();
        let _entered = ctx.enter();
        let failures = crate::service::lint(&services).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
    }
    world
}

fn get(world: &World, root: &str, user: Option<&str>, path: &str, query: &[(&str, &str)]) -> (u16, serde_json::Value) {
    let mut get = request("GET", path);
    for (name, value) in query {
        get.query.push(((*name).to_string(), (*value).to_string()));
    }
    if let Some(user) = user {
        get.set_header("authorization", basic(user));
    }
    let response = serve_at(&world.factory, get, root);
    let json = serde_json::from_str(&response.body_text()).unwrap_or(serde_json::Value::Null);
    (response.status, json)
}

fn names(list: &serde_json::Value, key: &str) -> Vec<String> {
    list.as_array()
        .unwrap_or_else(|| panic!("not a list: {list}"))
        .iter()
        .map(|item| item[key].as_str().unwrap_or("").to_string())
        .collect()
}

fn logged(lines: &Arc<Mutex<Vec<String>>>) -> String {
    lines.lock().unwrap().join("\n")
}

#[test]
fn shell_data_lists_the_apps_the_user_may_open_with_their_menus_providers_theme_values_and_counts() {
    let dir = tree("data");
    let world = world(&dir);
    let root = root_of(&dir);
    let lines = Arc::new(Mutex::new(Vec::new()));
    let (status, data) = tracing::subscriber::with_default(crate::testing::LogSubscriber(Arc::clone(&lines)), || {
        get(&world, &root, Some("plain"), "/shellData", &[])
    });
    assert_eq!(status, 200, "{data}");
    // the app list's set and order: secret needs ADMIN, broken does not parse, needs takes a parameter, hidden is not in the menu
    assert_eq!(names(&data["apps"], "name"), ["open"], "{data}");
    let open = &data["apps"][0];
    assert_eq!(open["title"], "Open App");
    assert_eq!(open["image"], "sym_o_hub");
    assert_eq!(open["imageType"], "icon");
    assert_eq!(open["path"], "/qapps/open");
    // the menu to depth two: a screen with a required parameter, one out of the menu and the third level are not in it
    let menu: Vec<(String, String, i64)> = open["menu"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["title"].as_str().unwrap().to_string(),
                item["path"].as_str().unwrap().to_string(),
                item["depth"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        menu,
        [
            ("List".to_string(), "/qapps/open/List".to_string(), 1),
            ("Sub".to_string(), "/qapps/open/Sub".to_string(), 1),
            ("Deep one".to_string(), "/qapps/open/Sub/Deep1".to_string(), 2),
        ],
        "{data}"
    );

    let providers = &open["providers"];
    // in sequence order; a kind with no screen opens the theme's quick create screen for it; one the user may not open
    // and one to no screen are left out
    assert_eq!(
        providers["quickCreate"],
        json!([
            {"kind": "task", "label": "Task", "icon": null, "url": "/qapps/open/QuickCreate?kind=task", "requiresRecord": false, "sequence": 10},
            {"kind": "note", "label": "Quick note (en)", "icon": "sym_o_note_add", "url": "/qapps/open/QuickNote?from=the+shell",
                "requiresRecord": false, "sequence": 20},
            {"kind": "activity", "label": "Activity", "icon": null, "url": "/qapps/open/QuickCreate?kind=activity", "requiresRecord": true,
                "sequence": 30}
        ]),
        "{data}"
    );
    assert_eq!(
        providers["todaySources"],
        json!([{"service": "test.Shell.get#TodayItems", "label": "Open items"}])
    );
    assert_eq!(
        providers["paletteActions"],
        json!([{"label": "Open the list", "icon": "sym_o_list", "url": "/qapps/open/List", "sequence": null}])
    );
    assert_eq!(providers["paletteTypes"], json!(["OpenDoc"]));
    assert_eq!(
        providers["paletteSources"],
        json!([
            {"group": "Ids", "label": "Id lookup", "icon": null, "prefix": null, "idPattern": "[A-Z]{2}[0-9]+", "sequence": 5},
            {"group": "Fail", "label": "Failing", "icon": null, "prefix": null, "idPattern": null, "sequence": 10},
            {"group": "Records", "label": "Records", "icon": "sym_o_search", "prefix": "~", "idPattern": null, "sequence": 20},
            {"group": "Empty", "label": "Empty", "icon": null, "prefix": null, "idPattern": null, "sequence": 30},
            {"group": "Stack", "label": "Stack", "icon": null, "prefix": null, "idPattern": "STACK", "sequence": 40}
        ])
    );
    // the palette sources' services stay on the server
    assert!(!data.to_string().contains("test.Shell.search"), "{data}");
    assert_eq!(providers["notificationTopics"], json!(["OpenAlert"]));
    assert_eq!(
        providers["keymaps"],
        json!([{"scope": "open", "keys": [
            {"keys": "g l", "label": "Go to the list", "url": "/qapps/open/List", "action": null},
            {"keys": "c", "label": "Create", "url": null, "action": "quickCreate"}
        ]}])
    );
    assert_eq!(
        providers["accountMenuItems"],
        json!([{"label": "Preferences", "icon": "sym_o_tune", "url": "/qapps/open/List", "sequence": null}])
    );
    // the theme values of DEFAULT_QUASAR: a screen the user may not open is null, a subscreen name is as written
    assert_eq!(
        data["shell"],
        json!({"trayToday": "/qapps/open/List", "trayChat": null, "trayNotifications": null, "quickCreate": "/qapps/open/QuickCreate",
            "mailScreen": null, "messagesScreen": null, "chatterScreen": "Chatter", "recordFilesScreen": null})
    );
    assert_eq!(data["counts"], json!({"today": 3, "chat": 1, "user": "plain"}));
    let text = logged(&lines);
    let broken = dir.join("Apps/Broken.xml").to_string_lossy().into_owned();
    assert!(
        text.contains(&format!("Leaving app broken [{broken}] out of the shell")),
        "{text}"
    );
    assert!(
        text.contains("Leaving quick-create open/NoSuchScreen of app open out of the shell"),
        "{text}"
    );
    assert!(text.contains("shell-providers has no element frobnicate"), "{text}");

    // ADMIN may open the secret app, and the providers and theme values that open it
    let (_, admin) = get(&world, &root, Some("admin"), "/shellData", &[]);
    assert_eq!(names(&admin["apps"], "name"), ["open", "secret"], "{admin}");
    assert_eq!(
        names(&admin["apps"][0]["providers"]["quickCreate"], "kind"),
        ["secret", "task", "note", "activity"]
    );
    assert_eq!(
        admin["apps"][0]["providers"]["keymaps"][0]["keys"][2]["url"],
        "/qapps/secret"
    );
    assert_eq!(admin["shell"]["trayChat"], "/qapps/secret");
    assert_eq!(admin["apps"][1]["providers"]["paletteTypes"], json!(["SecretDoc"]));

    // a user's own theme: its counts service and none of the default's values
    let (_, other) = get(&world, &root, Some("other"), "/shellData", &[]);
    assert_eq!(other["counts"], json!({"other": true}), "{other}");
    assert_eq!(other["shell"]["trayToday"], serde_json::Value::Null);
    assert_eq!(
        other["apps"][0]["providers"]["quickCreate"][0]["kind"], "note",
        "no quick create screen in this theme"
    );
    // a group's theme whose counts service fails: no counts, and the rest is there
    let (status, grouped) = get(&world, &root, Some("grouped"), "/shellData", &[]);
    assert_eq!(status, 200);
    assert_eq!(grouped["counts"], serde_json::Value::Null);
    assert_eq!(names(&grouped["apps"], "name"), ["open"]);

    let (status, _) = get(&world, &root, None, "/shellData", &[]);
    assert_eq!(status, 401);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shell_search_runs_only_the_sources_of_the_apps_the_user_may_open() {
    let dir = tree("search");
    let world = world(&dir);
    let root = root_of(&dir);
    let search = |user: Option<&str>, query: &[(&str, &str)]| get(&world, &root, user, "/shellSearch", query);

    let lines = Arc::new(Mutex::new(Vec::new()));
    let (status, found) = tracing::subscriber::with_default(crate::testing::LogSubscriber(Arc::clone(&lines)), || {
        search(Some("plain"), &[("text", "abc"), ("limit", "3")])
    });
    assert_eq!(status, 200, "{found}");
    // Ids needs an id, Fail fails (logged), Empty answers nothing, and Secret is of an app plain may not open
    assert_eq!(names(&found["groups"], "group"), ["Records"], "{found}");
    let records = &found["groups"][0];
    assert_eq!(records["label"], "Records");
    assert_eq!(records["icon"], "sym_o_search");
    assert_eq!(records["prefix"], "~");
    assert_eq!(records["moreUrl"], "/qapps/open/List?text=abc");
    assert_eq!(records["rows"].as_array().unwrap().len(), 3, "{found}");
    assert_eq!(
        records["rows"][0],
        json!({"title": "abc 0", "subtitle": "OpenDoc", "icon": "sym_o_label", "url": "/qapps/open/List?i=0"}),
        "the document types of the apps the user may open, and no idMatch"
    );
    let text = logged(&lines);
    assert!(
        text.contains("Leaving palette group Fail (test.Shell.search#Fail) of app open out of the search"),
        "{text}"
    );
    assert!(text.contains("the source is down"), "{text}");

    // ADMIN runs the secret app's source too, first by sequence, and the document types of both apps go to each source
    let (_, admin) = search(Some("admin"), &[("text", "abc")]);
    assert_eq!(names(&admin["groups"], "group"), ["Secret", "Records"], "{admin}");
    assert_eq!(
        admin["groups"][1]["rows"].as_array().unwrap().len(),
        5,
        "five rows by default"
    );
    assert_eq!(admin["groups"][1]["rows"][0]["subtitle"], "OpenDoc,SecretDoc");
    assert_eq!(admin["groups"][0]["label"], "Secret", "the group name without a label");

    // an id runs the id-pattern source, with idMatch
    let (_, ids) = search(Some("plain"), &[("text", "SO123")]);
    assert_eq!(names(&ids["groups"], "group"), ["Ids", "Records"], "{ids}");
    assert_eq!(ids["groups"][0]["rows"][0]["subtitle"], "id");
    assert_eq!(ids["groups"][1]["rows"][0]["subtitle"], "OpenDoc");

    // a source runs as the app root that registered it, so that app's authorization record and its entity filters apply
    let (_, stack) = search(Some("plain"), &[("text", "STACK")]);
    assert_eq!(names(&stack["groups"], "group"), ["Records", "Stack"], "{stack}");
    let frames = stack["groups"][1]["rows"][0]["subtitle"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        frames.contains("Apps/Open.xml"),
        "the app root is on the stack under the source: {frames}"
    );

    // groups narrows the sources, and names only the whitelisted ones: never another app's or a service
    let (_, narrowed) = search(Some("plain"), &[("text", "SO123"), ("groups", "Records, Secret")]);
    assert_eq!(names(&narrowed["groups"], "group"), ["Records"], "{narrowed}");
    let (_, service) = search(
        Some("plain"),
        &[("text", "abc"), ("groups", "test.Shell.search#Secret")],
    );
    assert_eq!(service["groups"], json!([]), "{service}");
    let (_, empty) = search(Some("plain"), &[("text", "  ")]);
    assert_eq!(empty["groups"], json!([]));
    let (status, _) = search(None, &[("text", "abc")]);
    assert_eq!(status, 401);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shell_counts_come_from_the_service_of_the_users_qapps_theme_and_a_failing_one_is_logged_once() {
    let dir = tree("counts");
    let world = world(&dir);
    let ctx = world.factory.new_context();
    let _entered = ctx.enter();
    let counts = |user: &str| crate::service::shell::shell_counts(user).map(Value::Map);
    // DEFAULT_QUASAR without a theme of the user's; a user's own; none without the theme value
    assert_eq!(
        counts("plain").map(|value| moqui_lib::json::to_json(&value)),
        Some(json!({"today": 3, "chat": 1, "user": "plain"}))
    );
    assert_eq!(
        counts("other").map(|value| moqui_lib::json::to_json(&value)),
        Some(json!({"other": true}))
    );
    {
        let _authz = ctx.disable_authz();
        let mut theme = world.facade.make_value("moqui.security.UserScreenTheme").unwrap();
        theme.set("userId", Value::string("admin"));
        theme.set("screenThemeTypeEnumId", Value::string("STT_INTERNAL_QUASAR"));
        theme.set("screenThemeId", Value::string("EMPTY_QUASAR"));
        theme.create(&world.facade).unwrap();
    }
    let lines = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(crate::testing::LogSubscriber(Arc::clone(&lines)), || {
        assert_eq!(counts("admin"), None, "no counts service in the theme");
        // a failing service and a missing one: no counts, each logged once however often it is asked
        for _ in 0..3 {
            assert_eq!(counts("grouped"), None);
        }
        let mut theme = world.facade.make_value("moqui.security.UserScreenTheme").unwrap();
        theme.set("userId", Value::string("plain"));
        theme.set("screenThemeTypeEnumId", Value::string("STT_INTERNAL_QUASAR"));
        theme.set("screenThemeId", Value::string("MISSING_QUASAR"));
        let _authz = ctx.disable_authz();
        theme.create(&world.facade).unwrap();
        assert_eq!(counts("plain"), None);
        assert_eq!(counts("plain"), None);
    });
    // (the service call logs the error of its body each time, as any failing service's)
    let text = logged(&lines);
    let shell_line = |service: &str| format!("The shell counts service {service}");
    assert_eq!(
        text.matches(&shell_line("test.Shell.get#BadCounts")).count(),
        1,
        "{text}"
    );
    assert!(text.contains("the counts are down"), "{text}");
    assert_eq!(
        text.matches(&shell_line("test.Shell.get#NoSuchCounts")).count(),
        1,
        "{text}"
    );
    assert!(
        ctx.message().errors().is_empty(),
        "the failures are not the caller's errors"
    );

    // the service: the current user by default, another user's counts only for internal work
    crate::user::internal_login_user("other").unwrap();
    let out = crate::service::sync("org.moqui.impl.ShellServices.get#ShellCounts", &BTreeMap::new()).unwrap();
    assert_eq!(
        out.get("counts").map(moqui_lib::json::to_json),
        Some(json!({"other": true}))
    );
    let theirs = BTreeMap::from([("userId".to_string(), Value::string("grouped"))]);
    let out = crate::service::sync("org.moqui.impl.ShellServices.get#ShellCounts", &theirs).unwrap();
    assert!(!out.contains_key("counts"), "{out:?}");
    assert!(
        ctx.message().errors().join("\n").contains("another user's counts"),
        "{:?}",
        ctx.message().errors()
    );
    ctx.message().clear_errors();
    let _authz = ctx.disable_authz();
    let out = crate::service::sync("org.moqui.impl.ShellServices.get#ShellCounts", &theirs).unwrap();
    assert_eq!(out.get("counts"), Some(&Value::Null), "the group's service fails");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `shellCounts` transition of webroot.xml (the shell's poll while the socket is down), as that file writes it, under
/// the test root: 401 without a user, else `{counts}` of the user's theme service, null without one.
#[test]
fn the_shell_counts_transition_answers_the_counts_of_the_current_user() {
    let dir = tree("counts-transition");
    let webroot = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/webroot/screen/webroot.xml"),
    )
    .unwrap();
    let start = webroot
        .find("<transition name=\"shellCounts\"")
        .expect("webroot.xml has shellCounts");
    let end = start + webroot[start..].find("</transition>").unwrap() + "</transition>".len();
    let root_text = ROOT.replacen(
        "<subscreens default-item",
        &format!("{}\n    <subscreens default-item", &webroot[start..end]),
        1,
    );
    std::fs::write(dir.join("Root.xml"), root_text).unwrap();
    let world = world(&dir);
    let root = root_of(&dir);
    let (status, _) = get(&world, &root, None, "/shellCounts", &[]);
    assert_eq!(status, 401);
    let (status, answer) = get(&world, &root, Some("plain"), "/shellCounts", &[]);
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer, json!({"counts": {"today": 3, "chat": 1, "user": "plain"}}));
    let (_, answer) = get(&world, &root, Some("other"), "/shellCounts", &[]);
    assert_eq!(answer, json!({"counts": {"other": true}}));
    {
        let ctx = world.factory.new_context();
        let _entered = ctx.enter();
        let _authz = ctx.disable_authz();
        row(
            &world,
            "moqui.security.UserScreenTheme",
            &[
                ("userId", Value::string("admin")),
                ("screenThemeTypeEnumId", Value::string("STT_INTERNAL_QUASAR")),
                ("screenThemeId", Value::string("EMPTY_QUASAR")),
            ],
        );
    }
    let (status, answer) = get(&world, &root, Some("admin"), "/shellCounts", &[]);
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer, json!({"counts": null}), "no counts service in the theme");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A user whose qapps theme names `test.Shell.get#ZonedCounts`, with a time zone and a locale of their own.
fn zoned_user(world: &World, user: &str) {
    world.user(user, None);
    let ctx = world.factory.new_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let mut account = world.facade.make_value("moqui.security.UserAccount").unwrap();
    account.set("userId", Value::string(user));
    account.set("timeZone", Value::string("America/Chicago"));
    account.set("locale", Value::string("de"));
    account.update(&world.facade).unwrap();
    row(
        world,
        "moqui.security.UserScreenTheme",
        &[
            ("userId", Value::string(user)),
            ("screenThemeTypeEnumId", Value::string("STT_INTERNAL_QUASAR")),
            ("screenThemeId", Value::string("ZONED_QUASAR")),
        ],
    );
}

/// The counts service a theme names is trusted data and reads only that user's rows, and no app's authorization covers
/// it (a Mantle service under the webroot transitions): it runs for the current user without service authorization, in
/// shellData and the shellCounts transition, and the user's own time zone and locale apply. Another user's counts stay
/// refused to a caller with authorization on.
#[test]
fn the_counts_service_runs_for_the_current_user_without_service_authorization_in_the_users_zone() {
    let dir = tree("counts-authz");
    let webroot = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/webroot/screen/webroot.xml"),
    )
    .unwrap();
    let start = webroot.find("<transition name=\"shellCounts\"").unwrap();
    let end = start + webroot[start..].find("</transition>").unwrap() + "</transition>".len();
    let root_text = ROOT.replacen(
        "<subscreens default-item",
        &format!("{}\n    <subscreens default-item", &webroot[start..end]),
        1,
    );
    std::fs::write(dir.join("Root.xml"), root_text).unwrap();
    let world = service_authz_world(&dir);
    zoned_user(&world, "zoned");
    let root = root_of(&dir);
    let expected = json!({"zone": "America/Chicago", "locale": "de", "user": "zoned"});
    let (status, answer) = get(&world, &root, Some("zoned"), "/shellCounts", &[]);
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer, json!({"counts": expected}));
    let (status, data) = get(&world, &root, Some("zoned"), "/shellData", &[]);
    assert_eq!(status, 200, "{data}");
    assert_eq!(data["counts"], expected, "{data}");

    // the service itself is refused to the user outside the shell, and get#ShellCounts still refuses another user's counts
    let ctx = world.factory.new_context();
    let _entered = ctx.enter();
    crate::user::internal_login_user("zoned").unwrap();
    let direct = BTreeMap::from([("userId".to_string(), Value::string("zoned"))]);
    let err = crate::service::sync("test.Shell.get#ZonedCounts", &direct).unwrap_err();
    assert!(err.to_string().contains("not authorized"), "{err}");
    let theirs = BTreeMap::from([("userId".to_string(), Value::string("plain"))]);
    let err = crate::service::sync("org.moqui.impl.ShellServices.get#ShellCounts", &theirs).unwrap_err();
    assert!(err.to_string().contains("not authorized"), "{err}");
    ctx.message().clear_errors();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The shellCounts messages for `user` the topic carried so far.
fn sent_counts(seen: &Arc<Mutex<Vec<serde_json::Value>>>, user: &str) -> Vec<serde_json::Value> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|body| {
            body["topic"] == "shellCounts" && body["userIds"].as_array().is_some_and(|ids| ids.contains(&json!(user)))
        })
        .cloned()
        .collect()
}

fn wait_for(seen: &Arc<Mutex<Vec<serde_json::Value>>>, user: &str, count: usize, limit: Duration) -> usize {
    let start = Instant::now();
    loop {
        let found = sent_counts(seen, user).len();
        if found >= count || start.elapsed() > limit {
            return found;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn publish_sends_the_counts_on_the_shell_counts_topic_once_per_window_and_after_the_commit() {
    let dir = tree("publish");
    let world = world(&dir);
    let user = format!("counted{}", std::process::id());
    world.user(&user, None);
    crate::service::shell::set_window(&world.factory, Duration::from_millis(400));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&seen);
    let _subscription = crate::notify::subscribe(
        crate::notify::NOTIFICATION_MESSAGE,
        Arc::new(move |message| keep.lock().unwrap().push(message.body.clone())),
    )
    .unwrap();
    let ctx = world.factory.new_context();
    let _entered = ctx.enter();
    let _authz = ctx.disable_authz();
    let publish = || {
        let parameters = BTreeMap::from([(
            "userIdList".to_string(),
            Value::List(vec![Value::string(&user), Value::string(&user)]),
        )]);
        let started = Instant::now();
        crate::service::sync("org.moqui.impl.ShellServices.publish#ShellCounts", &parameters).unwrap();
        // it does not wait for the counts
        assert!(started.elapsed() < Duration::from_millis(300));
    };

    publish();
    assert_eq!(wait_for(&seen, &user, 1, Duration::from_secs(5)), 1);
    let first = &sent_counts(&seen, &user)[0];
    assert_eq!(
        first["message"],
        json!({"today": 3, "chat": 1, "user": user}),
        "{first}"
    );
    assert_eq!(first["showAlert"], false, "{first}");
    assert_eq!(
        first["notificationMessageId"],
        serde_json::Value::Null,
        "not persisted: {first}"
    );
    // three more inside the window are one send at its end
    publish();
    publish();
    publish();
    assert_eq!(wait_for(&seen, &user, 2, Duration::from_secs(5)), 2);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(sent_counts(&seen, &user).len(), 2, "the folded requests are one send");

    // in a transaction it waits for the commit, and a rollback sends nothing
    let tx = world.facade.default_transactions().unwrap();
    assert!(tx.begin(None).unwrap());
    publish();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sent_counts(&seen, &user).len(), 2, "nothing before the commit");
    tx.rollback().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(sent_counts(&seen, &user).len(), 2, "nothing after a rollback");
    assert!(tx.begin(None).unwrap());
    publish();
    tx.commit().unwrap();
    assert_eq!(wait_for(&seen, &user, 3, Duration::from_secs(5)), 3);

    // the sender has no session: the user's time zone and locale are set from the account, so "today" is the user's day
    let zoned = format!("zoned{}", std::process::id());
    zoned_user(&world, &zoned);
    let parameters = BTreeMap::from([("userIdList".to_string(), Value::string(&zoned))]);
    crate::service::sync("org.moqui.impl.ShellServices.publish#ShellCounts", &parameters).unwrap();
    assert_eq!(wait_for(&seen, &zoned, 1, Duration::from_secs(5)), 1);
    assert_eq!(
        sent_counts(&seen, &zoned)[0]["message"],
        json!({"zone": "America/Chicago", "locale": "de", "user": zoned})
    );
    let _ = std::fs::remove_dir_all(&dir);
}
