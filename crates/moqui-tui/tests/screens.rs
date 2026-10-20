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

//! Scripted keys, painted Gallery frames, and the in-memory draw probe.
//!
//! `UPDATE_GOLDEN=1` rewrites `tests/snapshots/`.

use std::path::PathBuf;

use moqui_tui::{
    gallery_link, render_text, run_key_session, AppView, Key, KeyPress, MenuItem, Node, ScreenLink, ScreenModel,
    Session, GALLERY_DETAIL_PATH, GALLERY_LIST_JSON, GALLERY_LIST_PATH,
};

fn press(view: &mut AppView, key: Key) -> bool {
    view.on_key(&KeyPress::plain(key))
}

fn list_view() -> (AppView, moqui_tui::FakeLink) {
    let link = gallery_link();
    let model = link
        .level(GALLERY_LIST_PATH, &std::collections::BTreeMap::new())
        .unwrap();
    (AppView::new(model, link.clone()), link)
}

#[test]
fn session_pages_and_a_screen_link_loads_detail() {
    let link = gallery_link();
    let mut session = Session::new(
        link.level(GALLERY_LIST_PATH, &std::collections::BTreeMap::new())
            .unwrap(),
    );
    assert_eq!(session.focus_id(), "enumTypeId");
    assert!(session.on_key(&KeyPress::plain(Key::Char('q')), &link));

    let mut session = Session::new(
        link.level(GALLERY_LIST_PATH, &std::collections::BTreeMap::new())
            .unwrap(),
    );
    session.on_key(&KeyPress::plain(Key::Tab), &link);
    assert_eq!(session.focus_id(), "description");
    assert!(!session.on_key(&KeyPress::plain(Key::Char('q')), &link));
    assert_eq!(session.buffer("description"), "q");
    session.on_key(&KeyPress::plain(Key::F(8)), &link);
    assert_eq!(session.page_index("EnumList"), 1);
    assert_eq!(session.list_page("EnumList")[0].cells[0].text(), "ENUM_10");
    session.on_key(&KeyPress::plain(Key::F(7)), &link);
    assert_eq!(session.page_index("EnumList"), 0);
    assert_eq!(session.list_page("EnumList")[0].cells[0].text(), "ENUM_00");

    let mut session = Session::new(
        link.level(GALLERY_LIST_PATH, &std::collections::BTreeMap::new())
            .unwrap(),
    );
    for _ in 0..4 {
        session.on_key(&KeyPress::plain(Key::Tab), &link);
    }
    assert_eq!(session.focus_id(), "openDetail");
    session.on_key(&KeyPress::plain(Key::Enter), &link);
    assert_eq!(session.path(), GALLERY_DETAIL_PATH);
    assert_eq!(session.focus_id(), "updated");
}

#[test]
fn app_view_types_through_input_and_picks_from_the_option_list() {
    let (mut view, _) = list_view();
    press(&mut view, Key::Enter);
    assert!(view.session().dropdown_open());
    press(&mut view, Key::Down);
    let open = view.screen_text(100, 40);
    assert!(open.contains("Screen Theme Type"), "{open}");
    press(&mut view, Key::Enter);
    assert!(!view.session().dropdown_open());
    assert_eq!(view.session().buffer("enumTypeId"), "ScreenThemeType");
    let text = view.screen_text(100, 40);
    assert!(text.contains("Screen Theme Type"), "{text}");

    press(&mut view, Key::Tab);
    assert_eq!(view.session().focus_id(), "description");
    press(&mut view, Key::Char('a'));
    press(&mut view, Key::Backspace);
    press(&mut view, Key::Char('z'));
    assert_eq!(view.session().buffer("description"), "z");
    assert!(!press(&mut view, Key::Char('q')));
    assert_eq!(view.session().buffer("description"), "zq");
}

#[test]
fn three_tabs_then_enter_opens_detail_from_the_view() {
    let (mut view, _) = list_view();
    for _ in 0..3 {
        press(&mut view, Key::Tab);
    }
    assert_eq!(view.session().focus_id(), "EnumList");
    press(&mut view, Key::Enter);
    assert_eq!(view.session().path(), GALLERY_DETAIL_PATH);
}

#[test]
fn painted_list_keeps_unicode_and_caps_the_long_description() {
    let link = gallery_link();
    let model = ScreenModel::from_json(GALLERY_LIST_JSON).unwrap();
    let Node::FormList { list } = &model.c[1] else {
        panic!("form list");
    };
    let long = list.rows[3].cells[1].text();
    assert_eq!(long.chars().count(), 45);
    let wide = render_text(&model, &link, 100, 40);
    assert!(wide.contains("東京"), "{wide}");
    assert!(wide.contains("🎉"), "{wide}");
    // Revue's draw skips zero-width joiners, so the painted family is three emoji.
    assert!(wide.contains("👨👩👧"), "{wide}");
    assert!(!wide.contains('\u{200D}'), "{wide}");
    assert!(!wide.contains('\0'), "{wide}");
    assert!(wide.contains('…'), "{wide}");
    assert!(!wide.contains(long), "{wide}");
    assert!(wide.contains("Page 1/2"), "{wide}");
    assert!(wide.contains("> Type"), "{wide}");
    assert!(wide.starts_with("≡ Gallery ▸ List ▸ Enumerations\n"), "{wide}");
    assert!(wide.lines().any(|line| line.contains("F3=Back")), "{wide}");
    assert!(!wide.contains('│'), "{wide}");

    let narrow = render_text(&model, &link, 72, 24);
    assert!(narrow.lines().any(|line| line.ends_with('>')), "{narrow}");

    let detail = link
        .level(GALLERY_DETAIL_PATH, &std::collections::BTreeMap::new())
        .unwrap();
    let stacked = render_text(&detail, &link, 72, 24);
    assert!(stacked.contains("+ Record\n"), "{stacked}");
    assert!(
        !stacked
            .lines()
            .any(|line| line.contains("+ Record") && line.contains("+ Type")),
        "{stacked}"
    );
    let medium = render_text(&detail, &link, 100, 40);
    assert!(
        !medium
            .lines()
            .any(|line| line.contains("+ Record") && line.contains("+ Type")),
        "{medium}"
    );
    let detail_text = render_text(&detail, &link, 180, 50);
    assert!(
        detail_text
            .lines()
            .any(|line| line.contains("+ Record") && line.contains("+ Type")),
        "{detail_text}"
    );
    // draw_text skips the combining acute, same as it skips a ZWJ.
    assert!(detail_text.contains("cafe"), "{detail_text}");
    assert!(!detail_text.contains('\u{0301}'), "{detail_text}");
    assert!(detail_text.contains("STT_INTERNAL"), "{detail_text}");
    assert!(detail_text.contains("+ Record"), "{detail_text}");
    assert!(detail_text.contains("+ Type"), "{detail_text}");
}

#[test]
fn pipeline_harness_bytes_only_grow() {
    let link = gallery_link();
    let model = link
        .level(GALLERY_LIST_PATH, &std::collections::BTreeMap::new())
        .unwrap();
    let mut once = Vec::new();
    let first = run_key_session(&model, &link, &[], (180, 50), &mut once);
    assert!(first.harness_len > 0, "a frame writes terminal bytes");
    assert_eq!(first.harness_len, first.copied_len);
    assert_eq!(first.copied_len, once.len());
    assert_eq!(first.frames, 1);

    let keys = [KeyPress::plain(Key::Tab), KeyPress::plain(Key::Char('a'))];
    let mut grown_out = Vec::new();
    let grown = run_key_session(&model, &link, &keys, (180, 50), &mut grown_out);
    assert!(
        grown.harness_len > first.harness_len,
        "the harness keeps earlier frames: {} then {}",
        first.harness_len,
        grown.harness_len
    );
    assert_eq!(grown.harness_len, grown.copied_len);
    assert_eq!(grown.copied_len, grown_out.len());
    assert_eq!(grown.frames, 3);
    println!(
        "route2 frames={} bytes={} ms={}",
        grown.frames, grown.harness_len, grown.millis
    );
}

const DIALOG_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/dialog", "title": "Dialog" },
  "c": [
    {
      "t": "dialog",
      "id": "New",
      "title": "New enumeration",
      "button": "New",
      "c": [
        {
          "t": "form",
          "id": "NewForm",
          "name": "NewForm",
          "transitionUrl": "/new",
          "c": [
            {
              "t": "field",
              "id": "NewForm.name",
              "name": "name",
              "title": "Name",
              "required": true,
              "w": { "kind": "textLine", "value": "" }
            }
          ]
        }
      ]
    }
  ]
}"#;

#[test]
fn an_open_dialog_and_a_validation_mark_are_on_the_frame() {
    let link = moqui_tui::FakeLink::from_json(&[DIALOG_JSON]).unwrap();
    let model = ScreenModel::from_json(DIALOG_JSON).unwrap();
    let mut view = AppView::new(model, link);
    press(&mut view, Key::Enter);
    assert_eq!(view.session().dialogs_open(), &["New".to_string()]);
    let dialog = view.screen_text(100, 40);
    assert!(dialog.contains("+ New enumeration"), "{dialog}");
    assert!(dialog.contains("Name"), "{dialog}");
    golden("dialog-100x40", &dialog);

    let (edit, edit_link) = validation_view();
    let mut view = edit;
    press(&mut view, Key::Enter);
    assert_eq!(view.session().mark("Edit.name"), Some("required"));
    let marked = view.screen_text(100, 40);
    assert!(marked.contains("! required"), "{marked}");
    assert!(marked.lines().any(|line| line == "required"), "{marked}");
    let _ = edit_link;
    golden("validation-100x40", &marked);
}

#[test]
fn a_docked_drawer_shows_two_menu_levels_and_a_busy_mark() {
    let (mut view, _) = list_view();
    view.session_mut().set_menu(vec![MenuItem {
        title: "Gallery".to_string(),
        url: "/Gallery".to_string(),
        children: vec![MenuItem {
            title: "List".to_string(),
            url: GALLERY_LIST_PATH.to_string(),
            children: Vec::new(),
        }],
    }]);
    view.session_mut().set_busy(true);
    let wide = view.screen_text(180, 40);
    assert!(wide.contains("Gallery ▸ List ▸ Enumerations *"), "{wide}");
    assert!(wide.contains("▾ Gallery"), "{wide}");
    assert!(wide.contains("▶ List"), "{wide}");
    assert!(wide.contains('│'), "{wide}");
    let narrow = view.screen_text(100, 40);
    assert!(narrow.starts_with('≡'), "{narrow}");
    assert!(!narrow.contains("▾ Gallery"), "{narrow}");
}

fn validation_view() -> (AppView, moqui_tui::FakeLink) {
    const EDIT: &str = r#"{
      "v": 1,
      "screen": { "path": "/edit", "title": "Edit" },
      "c": [
        {
          "t": "form",
          "id": "Edit",
          "name": "Edit",
          "transitionUrl": "/edit/update",
          "c": [
            {
              "t": "field",
              "id": "Edit.name",
              "name": "name",
              "title": "Name",
              "required": true,
              "w": { "kind": "textLine", "value": "" }
            }
          ]
        }
      ]
    }"#;
    let link = moqui_tui::FakeLink::from_json(&[EDIT]).unwrap();
    let model = ScreenModel::from_json(EDIT).unwrap();
    (AppView::new(model, link.clone()), link)
}

#[test]
fn gallery_snapshots() {
    let link = gallery_link();
    let list = link
        .level(GALLERY_LIST_PATH, &std::collections::BTreeMap::new())
        .unwrap();
    let detail = link
        .level(GALLERY_DETAIL_PATH, &std::collections::BTreeMap::new())
        .unwrap();
    let frames = [
        ("list-72x24", &list, 72u16, 24u16),
        ("list-100x40", &list, 100, 40),
        ("list-180x50", &list, 180, 50),
        ("detail-72x24", &detail, 72, 24),
        ("detail-100x40", &detail, 100, 40),
        ("detail-180x50", &detail, 180, 50),
    ];
    for (name, model, width, height) in frames {
        golden(name, &render_text(model, &link, width, height));
    }
}

fn golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("snapshots dir")).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}. Set UPDATE_GOLDEN=1", path.display()));
    assert_eq!(expected, actual, "{}", path.display());
}
