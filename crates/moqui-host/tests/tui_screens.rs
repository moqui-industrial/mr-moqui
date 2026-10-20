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

//! Scripted Gallery and Party screens through the in-process client.
//!
//! One server: every Gallery screen renders a model, then the keyboard flows on Gallery List and SimpleScreens Party.
//! Mantle is linked into this runtime only.

#![cfg(all(unix, feature = "embedded-postgres", feature = "axum"))]

mod common;

use example as _;

use std::path::PathBuf;
use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_host::tui::HostLink;
use moqui_lib::object::Value;
use moqui_tui::{frame, outline, AppView, FormList, Key, KeyPress, Node, ScreenLink, ScreenModel};

const GALLERY: &[&str] = &[
    "Landing",
    "List",
    "Detail",
    "Record360",
    "Task",
    "Board",
    "Calendar",
    "Split",
    "Settings",
    "Wizard",
    "Dialog",
    "Report",
    "Panel",
    "Widgets",
    "Editor",
    "Message",
    "Views",
    "QuickNote",
    "QuickTask",
    "Chatter",
    "RecordFiles",
];

const SIMPLE_SCREENS: &str = r#"<screen-facade>
    <screen location="component://webroot/screen/webroot/apps.xml">
        <subscreens-item name="SimpleScreens" menu-title="Simple Screens"
            location="component://SimpleScreens/screen/SimpleScreens.xml"/>
    </screen>
</screen-facade>"#;

#[test]
fn gallery_and_party_screens() {
    let gate = start_gate();
    let factory = Arc::clone(gate.server.factory());
    grant_gate(&factory);
    let link = HostLink::new(factory, "john.doe");
    render_gallery(&link);
    drive_gallery(&link);
    drive_party(&link);
}

fn start_gate() -> common::Gate {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let udm = std::fs::canonicalize(repo.join("runtime/component/mr-mantle-udm")).expect("mantle-udm");
    let usl = std::fs::canonicalize(repo.join("runtime/component/mr-mantle-usl")).expect("mantle-usl");
    let screens = std::fs::canonicalize(repo.join("runtime/component/MrSimpleScreens")).expect("SimpleScreens");
    let links = [
        (udm.to_str().expect("udm path"), "component/mantle-udm"),
        (usl.to_str().expect("usl path"), "component/mantle-usl"),
        (screens.to_str().expect("screens path"), "component/SimpleScreens"),
    ];
    // The Editor screen reads the EXAMPLE wiki space. That file is demo data, so the seed load leaves it out.
    let wiki = std::fs::read_to_string(repo.join("runtime/base-component/example/data/ExampleWikiData.xml"))
        .expect("example wiki data");
    common::start_with_links("tui-screens", SIMPLE_SCREENS, Some(&wiki), Some(&["demo"]), &links)
}

fn render_gallery(link: &HostLink) {
    let mut failures = Vec::new();
    for name in GALLERY {
        let path = format!("/apps/gallery/{name}");
        match link.level(&path, &std::collections::BTreeMap::new()) {
            Ok(model) => {
                if let Some(text) = lone_error(&model) {
                    failures.push(format!("{path} rendered only an error: {text}"));
                }
            }
            Err(err) => failures.push(format!("{path}: {err}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn lone_error(model: &ScreenModel) -> Option<&str> {
    match model.c.as_slice() {
        [Node::Error { text, .. }] => Some(text.as_str()),
        _ => None,
    }
}

fn drive_gallery(link: &HostLink) {
    let mut view = open_screen(link, "/apps/gallery/List");
    paint_frame(&mut view);
    page_enumerations(&mut view);
    sort_enumerations(&mut view);
    toggle_find(&mut view);
    open_detail(&mut view);
    view.on_key(&press(Key::F(3)));
    assert!(
        view.session().path().contains("gallery/List"),
        "{} {}",
        view.session().path(),
        view.session().message()
    );
    submit_new_enum(&mut view);
    find_enumerations(&mut view);
}

fn find_enumerations(view: &mut AppView<HostLink>) {
    focus(view, "EnumFind.description");
    type_text(view, "e");
    view.on_key(&press(Key::Enter));
    assert!(
        view.session().path().contains("gallery/List"),
        "{}",
        view.session().message()
    );
}

fn page_enumerations(view: &mut AppView<HostLink>) {
    let count = find_list(&view.session().model().c, "EnumList")
        .map(list_count)
        .unwrap_or(0);
    assert!(
        count > 20,
        "EnumList count {count}; {}",
        list_summary(view.session().model())
    );
    let before = first_cell(view);
    focus(view, "EnumList");
    view.on_key(&press(Key::F(8)));
    assert_eq!(view.session().page_index("EnumList"), 1, "{}", view.session().message());
    assert_ne!(before, first_cell(view), "page did not change the first row");
}

fn sort_enumerations(view: &mut AppView<HostLink>) {
    focus(view, "EnumList");
    view.on_key(&press(Key::Char('s')));
    let order = view
        .session()
        .model()
        .parameters
        .get("orderByField")
        .cloned()
        .unwrap_or_default();
    assert_eq!(order, "enumId", "{}", view.session().message());
}

fn toggle_find(view: &mut AppView<HostLink>) {
    focus(view, "EnumList");
    view.on_key(&press(Key::Char('f')));
    assert!(view.session().find_open());
    let text = view.screen_text(100, 40);
    assert!(text.contains("Find"), "{text}");
    view.on_key(&press(Key::Escape));
    assert!(!view.session().find_open());
}

fn open_detail(view: &mut AppView<HostLink>) {
    focus(view, "EnumList");
    view.on_key(&press(Key::Enter));
    assert!(
        view.session().path().contains("/Detail"),
        "{} {}",
        view.session().path(),
        view.session().message()
    );
    assert_side_by_side(view.session().model(), frame(180).canvas);
    assert_stacked(view.session().model(), frame(100).canvas);
}

fn submit_new_enum(view: &mut AppView<HostLink>) {
    focus(view, "NewEnumDialog");
    view.on_key(&press(Key::Enter));
    focus(view, "NewEnumForm.description");
    view.on_key(&press(Key::Enter));
    assert!(
        view.session().message().contains("nothing is written"),
        "{}",
        view.session().message()
    );
    assert!(
        view.session().dialogs_open().is_empty(),
        "{:?}",
        view.session().dialogs_open()
    );
    assert!(
        view.session().path().contains("gallery/List"),
        "{}",
        view.session().path()
    );
}

fn drive_party(link: &HostLink) {
    let mut view = open_screen(link, "/apps/SimpleScreens/Party/FindParty");
    paint_frame(&mut view);
    assert_stacked(view.session().model(), frame(180).canvas);
    let note = note_search(&mut view);
    eprintln!("party search: {note}");
    // A search that the index cannot answer replaces the screen with an error. Open the find again.
    view = open_screen(link, "/apps/SimpleScreens/Party/FindParty");
    check_country_states(&mut view);
    create_person(&mut view, link);
}

fn note_search(view: &mut AppView<HostLink>) -> String {
    if !focus_at(view, "OuterSearchForm.anyField") {
        return format!("field missing at {}", view.session().focus_id());
    }
    type_text(view, "zzznomatch");
    view.on_key(&press(Key::Enter));
    let error = lone_error(view.session().model()).unwrap_or("");
    format!(
        "path {} message {} error {}",
        view.session().path(),
        view.session().message(),
        error
    )
}

fn check_country_states(view: &mut AppView<HostLink>) {
    focus(view, "CreatePersonCustDialog");
    view.on_key(&press(Key::Enter));
    focus(view, "CreatePersonCustForm.stateProvinceGeoId");
    choose_value(view, "CA");
    assert_eq!(view.session().buffer("CreatePersonCustForm.stateProvinceGeoId"), "CA");
    focus(view, "CreatePersonCustForm.countryGeoId");
    choose_value(view, "CAN");
    assert_ne!(view.session().buffer("CreatePersonCustForm.stateProvinceGeoId"), "CA");
    focus(view, "CreatePersonCustForm.stateProvinceGeoId");
    view.on_key(&press(Key::F(4)));
    let choices = view.session().focused_choices();
    assert!(choices.iter().any(|choice| choice.value == "ON"), "{choices:?}");
    assert!(choices.iter().all(|choice| choice.value != "CA"), "{choices:?}");
    view.on_key(&press(Key::Escape));
    view.on_key(&press(Key::Escape));
    assert!(
        view.session().dialogs_open().is_empty(),
        "{:?}",
        view.session().dialogs_open()
    );
}

fn create_person(view: &mut AppView<HostLink>, link: &HostLink) {
    focus(view, "CreatePersonDialog");
    view.on_key(&press(Key::Enter));
    focus(view, "CreatePersonForm.firstName");
    type_text(view, "Tui");
    view.on_key(&press(Key::Tab));
    assert_eq!(view.session().focus_id(), "CreatePersonForm.lastName");
    type_text(view, "Gate");
    view.on_key(&press(Key::Enter));
    // The country test leaves another form dirty, so the redirect asks before it leaves.
    if view.session().confirm_choice().is_some() {
        // Down moves the confirm. Right would type into the text field.
        view.on_key(&press(Key::Down));
        view.on_key(&press(Key::Enter));
    }
    assert!(
        view.session().path().contains("/EditParty"),
        "{} {}",
        view.session().path(),
        view.session().message()
    );
    require_menu(view, link);
    paint_frame(view);
    assert_side_by_side(view.session().model(), frame(180).canvas);
    assert_stacked(view.session().model(), frame(100).canvas);
    focus(view, "EditPersonForm.firstName");
    type_text(view, "x");
    view.on_key(&press(Key::Enter));
    let saved = view.session().buffer("EditPersonForm.firstName").contains('x')
        || !view.session().message().is_empty()
        || view.session().mark("EditPersonForm.firstName").is_some();
    assert!(
        saved,
        "buffer {} message {} focus {}",
        view.session().buffer("EditPersonForm.firstName"),
        view.session().message(),
        view.session().focus_id()
    );
}

fn open_screen(link: &HostLink, path: &str) -> AppView<HostLink> {
    let model = link
        .level(path, &std::collections::BTreeMap::new())
        .unwrap_or_else(|err| panic!("{path}: {err}"));
    let mut view = AppView::new(model, link.clone());
    require_menu(&mut view, link);
    view
}

fn require_menu(view: &mut AppView<HostLink>, link: &HostLink) {
    let path = view.session().path().to_string();
    let menu = link.menu(&path).unwrap_or_else(|err| panic!("menu {path}: {err}"));
    assert!(!menu.is_empty(), "menu {path}");
    view.session_mut().set_menu(menu);
}

fn paint_frame(view: &mut AppView<HostLink>) {
    let narrow = view.screen_text(100, 40);
    assert!(
        narrow.contains(view.session().path().rsplit('/').next().unwrap_or("")),
        "{narrow}"
    );
    let wide = view.screen_text(180, 50);
    assert!(wide.contains('│'), "{wide}");
    let title = drawer_title(view);
    assert!(!title.is_empty(), "menu has no title");
    assert!(wide.contains(&title), "missing {title}\n{wide}");
}

fn drawer_title(view: &AppView<HostLink>) -> String {
    let mut titles = Vec::new();
    for item in view.session().menu() {
        if !item.title.is_empty() {
            titles.push(item.title.clone());
        }
        for child in &item.children {
            if !child.title.is_empty() {
                titles.push(child.title.clone());
            }
        }
    }
    titles
        .into_iter()
        .min_by_key(|title| title.chars().count())
        .unwrap_or_default()
}

fn focus(view: &mut AppView<HostLink>, id: &str) {
    assert!(
        focus_at(view, id),
        "focus is {}, wanted {id}",
        view.session().focus_id()
    );
}

fn focus_at(view: &mut AppView<HostLink>, id: &str) -> bool {
    if view.session().focus_id() == id {
        return true;
    }
    for _ in 0..200 {
        view.on_key(&press(Key::Tab));
        if view.session().focus_id() == id {
            return true;
        }
    }
    false
}

fn type_text(view: &mut AppView<HostLink>, text: &str) {
    for ch in text.chars() {
        view.on_key(&press(Key::Char(ch)));
    }
}

fn choose_value(view: &mut AppView<HostLink>, value: &str) {
    view.on_key(&press(Key::F(4)));
    let choices = view.session().focused_choices();
    let current = view.session().buffer(view.session().focus_id()).to_string();
    let from = choices.iter().position(|choice| choice.value == current).unwrap_or(0);
    let Some(to) = choices.iter().position(|choice| choice.value == value) else {
        let shown: Vec<_> = choices.iter().take(8).map(|choice| choice.value.as_str()).collect();
        panic!(
            "no {value} among {} choices, first {shown:?}; {}",
            choices.len(),
            field_debug(view)
        );
    };
    let key = if to >= from { Key::Down } else { Key::Up };
    for _ in 0..from.abs_diff(to) {
        view.on_key(&press(key.clone()));
    }
    view.on_key(&press(Key::Enter));
}

fn field_debug(view: &AppView<HostLink>) -> String {
    let id = view.session().focus_id().to_string();
    let buffer = view.session().buffer(&id).to_string();
    let mut found = String::new();
    describe_field(&view.session().model().c, &id, &mut found);
    if found.is_empty() {
        format!("focus {id} buffer {buffer}")
    } else {
        format!("{found} buffer {buffer}")
    }
}

fn describe_field(nodes: &[Node], id: &str, found: &mut String) {
    for node in nodes {
        if let Node::Field { id: field_id, w, .. } = node {
            if field_id == id {
                *found = format!("{field_id} {w:?}");
            }
        }
        describe_field(node.children(), id, found);
    }
}

fn press(key: Key) -> KeyPress {
    KeyPress::plain(key)
}

fn first_cell(view: &AppView<HostLink>) -> String {
    view.session()
        .list_page("EnumList")
        .first()
        .and_then(|row| row.cells.first())
        .map(|cell| cell.text().to_string())
        .unwrap_or_default()
}

fn list_count(list: &FormList) -> u32 {
    if list.paging.count > 0 {
        list.paging.count
    } else {
        u32::try_from(list.rows.len()).unwrap_or(u32::MAX)
    }
}

fn list_summary(model: &ScreenModel) -> String {
    let mut lines = Vec::new();
    summarize_lists(&model.c, &mut lines);
    if lines.is_empty() {
        "no form-list".to_string()
    } else {
        lines.join("; ")
    }
}

fn summarize_lists(nodes: &[Node], lines: &mut Vec<String>) {
    for node in nodes {
        if let Node::FormList { list } = node {
            lines.push(format!(
                "{} count {} size {} rows {}",
                list.id,
                list.paging.count,
                list.paging.size,
                list.rows.len()
            ));
        }
        summarize_lists(node.children(), lines);
    }
}

fn find_list<'a>(nodes: &'a [Node], id: &str) -> Option<&'a FormList> {
    for node in nodes {
        if let Node::FormList { list } = node {
            if list.id == id {
                return Some(list);
            }
        }
        if let Some(found) = find_list(node.children(), id) {
            return Some(found);
        }
    }
    None
}

fn assert_side_by_side(model: &ScreenModel, canvas: u16) {
    let xs = column_xs(model, canvas);
    assert!(xs.len() >= 2, "columns {xs:?}\n{}", outline(model, canvas));
}

fn assert_stacked(model: &ScreenModel, canvas: u16) {
    let xs = column_xs(model, canvas);
    assert!(xs.len() < 2, "columns {xs:?}\n{}", outline(model, canvas));
}

fn column_xs(model: &ScreenModel, canvas: u16) -> Vec<u16> {
    let mut xs = Vec::new();
    for line in outline(model, canvas).lines() {
        let Some(rest) = line.trim().strip_prefix("col x=") else {
            continue;
        };
        let Some(token) = rest.split_whitespace().next() else {
            continue;
        };
        let Ok(x) = token.parse::<u16>() else {
            continue;
        };
        if !xs.contains(&x) {
            xs.push(x);
        }
    }
    xs
}

fn grant_gate(factory: &Arc<ExecutionContextFactory>) {
    let context = factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let facade = factory.entity().expect("entity facade");
    let mut group = facade
        .make_value("moqui.security.ArtifactGroup")
        .expect("ArtifactGroup");
    group.set("artifactGroupId", Value::string("TUI_GATE"));
    group.set("description", Value::string("Screen TUI gate"));
    group.create(&facade).expect("TUI_GATE");
    grant_member(&facade, ".*", "AT_SERVICE");
    grant_member(&facade, "component://SimpleScreens/.*", "AT_XML_SCREEN");
    grant_member(&facade, "component://SimpleScreens/.*", "AT_XML_SCREEN_TRANS");
    let mut authz = facade
        .make_value("moqui.security.ArtifactAuthz")
        .expect("ArtifactAuthz");
    authz.set("artifactAuthzId", Value::string("TUI_GATE_ADMIN"));
    authz.set("userGroupId", Value::string("ADMIN"));
    authz.set("artifactGroupId", Value::string("TUI_GATE"));
    authz.set("authzTypeEnumId", Value::string("AUTHZT_ALWAYS"));
    authz.set("authzActionEnumId", Value::string("AUTHZA_ALL"));
    authz.create(&facade).expect("TUI_GATE_ADMIN");
    grant_state(&facade, "CA", "California", "USA");
    grant_state(&facade, "ON", "Ontario", "CAN");
    grant_enum(&facade, "PartyType", "Party Type", "PtyPerson", "Person");
    let mut na = facade.make_value("mantle.party.Party").expect("Party");
    na.set("partyId", Value::string("_NA_"));
    na.set("pseudoId", Value::string("_NA_"));
    na.set("partyTypeEnumId", Value::string("PtyPerson"));
    na.set("ownerPartyId", Value::string("_NA_"));
    na.create(&facade).unwrap_or_else(|err| panic!("_NA_ party: {err}"));
}

fn grant_enum(
    facade: &moqui_core::entity::EntityFacade,
    type_id: &str,
    type_desc: &str,
    enum_id: &str,
    enum_desc: &str,
) {
    let mut kind = facade
        .make_value("moqui.basic.EnumerationType")
        .expect("EnumerationType");
    kind.set("enumTypeId", Value::string(type_id));
    kind.set("description", Value::string(type_desc));
    kind.create(facade)
        .unwrap_or_else(|err| panic!("enum type {type_id}: {err}"));
    let mut item = facade.make_value("moqui.basic.Enumeration").expect("Enumeration");
    item.set("enumId", Value::string(enum_id));
    item.set("enumTypeId", Value::string(type_id));
    item.set("description", Value::string(enum_desc));
    item.create(facade)
        .unwrap_or_else(|err| panic!("enum {enum_id}: {err}"));
}

fn grant_member(facade: &moqui_core::entity::EntityFacade, name: &str, artifact_type: &str) {
    let mut member = facade
        .make_value("moqui.security.ArtifactGroupMember")
        .expect("ArtifactGroupMember");
    member.set("artifactGroupId", Value::string("TUI_GATE"));
    member.set("artifactName", Value::string(name));
    member.set("artifactTypeEnumId", Value::string(artifact_type));
    member.set("nameIsPattern", Value::string("Y"));
    member.set("inheritAuthz", Value::string("Y"));
    member
        .create(facade)
        .unwrap_or_else(|err| panic!("{artifact_type} {name}: {err}"));
}

fn grant_state(facade: &moqui_core::entity::EntityFacade, geo_id: &str, geo_name: &str, parent: &str) {
    let mut geo = facade.make_value("moqui.basic.Geo").expect("Geo");
    geo.set("geoId", Value::string(geo_id));
    geo.set("geoTypeEnumId", Value::string("GEOT_STATE"));
    geo.set("geoName", Value::string(geo_name));
    geo.set("geoCodeAlpha2", Value::string(geo_id));
    geo.create(facade).unwrap_or_else(|err| panic!("geo {geo_id}: {err}"));
    let mut assoc = facade.make_value("moqui.basic.GeoAssoc").expect("GeoAssoc");
    assoc.set("geoId", Value::string(parent));
    assoc.set("toGeoId", Value::string(geo_id));
    assoc.set("geoAssocTypeEnumId", Value::string("GAT_REGIONS"));
    assoc
        .create(facade)
        .unwrap_or_else(|err| panic!("assoc {parent}->{geo_id}: {err}"));
}
