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

//! The Revue view through its real draw pipeline: the Gallery fixtures at 72x24, 100x40 and 180x50 (and an ASCII,
//! monochrome set), the frame, the overlays, and drawing that follows the keys.
//!
//! `UPDATE_GOLDEN=1` rewrites `tests/snapshots/`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use moqui_tui::{
    gallery_link, AppView, FakeLink, Key, KeyPress, MenuItem, ScreenLink, ScreenModel, Session, ShellInfo, Theme,
    GALLERY_DETAIL_PATH, GALLERY_LIST_PATH,
};

fn press(view: &mut AppView, key: Key) -> bool {
    view.on_key(&KeyPress::plain(key))
}

fn view_of(model: ScreenModel, link: FakeLink) -> AppView {
    let mut view = AppView::new(model, link);
    view.set_clock("Tue 14:32");
    view
}

fn list_view() -> AppView {
    let link = gallery_link();
    let model = link.level(GALLERY_LIST_PATH, &BTreeMap::new()).unwrap();
    view_of(model, link)
}

fn ascii() -> Theme {
    Theme::detect(&|name| (name == "NO_COLOR").then(|| "1".to_string()), true, false)
}

#[test]
fn gallery_snapshots() {
    let link = gallery_link();
    for (name, path, width, height, plain) in [
        ("list-72x24", GALLERY_LIST_PATH, 72u16, 24u16, false),
        ("list-100x40", GALLERY_LIST_PATH, 100, 40, false),
        ("list-180x50", GALLERY_LIST_PATH, 180, 50, false),
        ("detail-72x24", GALLERY_DETAIL_PATH, 72, 24, false),
        ("detail-100x40", GALLERY_DETAIL_PATH, 100, 40, false),
        ("detail-180x50", GALLERY_DETAIL_PATH, 180, 50, false),
        ("list-100x40-ascii", GALLERY_LIST_PATH, 100, 40, true),
        ("detail-180x50-ascii", GALLERY_DETAIL_PATH, 180, 50, true),
    ] {
        let mut view = view_of(link.level(path, &BTreeMap::new()).unwrap(), link.clone());
        if plain {
            view.set_theme(ascii());
        }
        let text = view.screen_text(width, height);
        if plain {
            assert!(
                !text.contains('─') && !text.contains('▾') && !text.contains('≡'),
                "{name}: {text}"
            );
        }
        golden(name, &text);
    }
}

#[test]
fn a_painted_list_keeps_unicode_pages_and_caps_a_long_description() {
    let mut view = list_view();
    let wide = view.screen_text(180, 50);
    assert!(wide.contains("東京"), "{wide}");
    assert!(wide.contains("🎉"), "{wide}");
    assert!(wide.contains("Description text that should be capped …"), "{wide}");
    assert!(wide.contains("Rows 1–10 of 20 · Page 1/2"), "{wide}");
    // Labels with spaced leaders, as in the mockups.
    assert!(wide.contains("Type . . . . : ["), "{wide}");
    assert!(wide.contains("Description  : ["), "{wide}");
    for _ in 0..3 {
        press(&mut view, Key::Tab);
    }
    press(&mut view, Key::F(8));
    let next = view.screen_text(180, 50);
    assert!(next.contains("ENUM_10"), "{next}");
    assert!(next.contains("Rows 11–20 of 20 · Page 2/2"), "{next}");
    // The list wider than 72 scrolls: the first column stays, the rest move, with the marks.
    let narrow = view.screen_text(72, 24);
    assert!(narrow.contains('▸'), "{narrow}");
    view.on_key(&KeyPress {
        key: Key::Right,
        shift: true,
        alt: false,
        ctrl: false,
    });
    let scrolled = view.screen_text(72, 24);
    assert!(scrolled.contains('◂'), "{scrolled}");
    assert!(scrolled.contains("ENUM_10"), "{scrolled}");
}

#[test]
fn typing_shows_in_the_field_with_its_change_mark() {
    let mut view = list_view();
    press(&mut view, Key::Tab);
    for ch in "Enum".chars() {
        press(&mut view, Key::Char(ch));
    }
    let text = view.screen_text(100, 40);
    assert!(text.contains("[Enum"), "{text}");
    assert!(text.contains("]•"), "{text}");
}

const DIALOG_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/dialog", "title": "Dialog" },
  "c": [
    { "t": "dialog", "id": "New", "title": "New enumeration", "button": "New", "c": [
      { "t": "form", "id": "NewForm", "name": "NewForm", "transitionUrl": "/new", "c": [
        { "t": "field", "id": "NewForm.name", "name": "name", "title": "Name", "required": true,
          "w": { "kind": "textLine", "value": "" } },
        { "t": "field", "id": "NewForm.go", "name": "go", "title": "Create", "w": { "kind": "submit", "text": "Create" } }
      ]}
    ]}
  ]
}"#;

#[test]
fn an_open_dialog_and_a_validation_mark_are_on_the_frame() {
    let link = FakeLink::from_json(&[DIALOG_JSON]).unwrap();
    let mut view = view_of(ScreenModel::from_json(DIALOG_JSON).unwrap(), link);
    let closed = view.screen_text(100, 40);
    assert!(closed.contains("[ New… ]"), "{closed}");
    press(&mut view, Key::Enter);
    assert_eq!(view.session().dialogs_open(), ["New"]);
    assert_eq!(view.session().focus_id(), "NewForm.name");
    let dialog = view.screen_text(100, 40);
    assert!(dialog.contains("New enumeration"), "{dialog}");
    assert!(dialog.contains("* Name"), "{dialog}");
    golden("dialog-100x40", &dialog);
    press(&mut view, Key::Enter);
    assert_eq!(view.session().mark("NewForm.name"), Some("A value is required"));
    let marked = view.screen_text(100, 40);
    assert!(marked.contains("! A value is required"), "{marked}");
    assert!(marked.contains("]!"), "{marked}");
    golden("validation-100x40", &marked);
    // Esc closes the dialog; focus is back on its button.
    press(&mut view, Key::Escape);
    press(&mut view, Key::Escape);
    assert!(view.session().dialogs_open().is_empty());
    assert_eq!(view.session().focus_id(), "New");
}

fn menu() -> Vec<MenuItem> {
    vec![MenuItem {
        title: "Gallery".to_string(),
        url: "/Gallery".to_string(),
        children: vec![
            MenuItem {
                title: "List".to_string(),
                url: GALLERY_LIST_PATH.to_string(),
                children: Vec::new(),
            },
            MenuItem {
                title: "Detail".to_string(),
                url: GALLERY_DETAIL_PATH.to_string(),
                children: Vec::new(),
            },
        ],
    }]
}

#[test]
fn the_drawer_docks_when_wide_and_f9_shows_and_focuses_it() {
    let mut view = list_view();
    view.session_mut().set_menu(menu());
    let wide = view.screen_text(180, 40);
    assert!(wide.contains("▾ Gallery"), "{wide}");
    assert!(wide.contains("▶ List"), "{wide}");
    assert!(wide.contains("┬"), "{wide}");
    let narrow = view.screen_text(100, 40);
    assert!(narrow.starts_with(" ≡"), "{narrow}");
    assert!(!narrow.contains("▾ Gallery"), "{narrow}");
    press(&mut view, Key::F(9));
    let shown = view.screen_text(100, 40);
    assert!(shown.contains("▾ Gallery"), "{shown}");
    press(&mut view, Key::Down);
    press(&mut view, Key::Down);
    press(&mut view, Key::Enter);
    assert_eq!(view.session().path(), GALLERY_DETAIL_PATH);
}

#[test]
fn the_drop_down_list_opens_under_its_field() {
    let mut view = list_view();
    press(&mut view, Key::F(4));
    let text = view.screen_text(100, 40);
    assert!(text.contains("Screen Theme Type"), "{text}");
    assert!(text.contains("(none)"), "{text}");
    press(&mut view, Key::Down);
    press(&mut view, Key::Enter);
    let picked = view.screen_text(100, 40);
    assert!(picked.contains("[Screen Theme Type"), "{picked}");
}

#[test]
fn the_leave_confirmation_and_the_help_show_over_the_screen() {
    let mut view = list_view();
    press(&mut view, Key::Tab);
    press(&mut view, Key::Char('z'));
    view.on_key(&KeyPress::with_ctrl(Key::Char('q')));
    let ask = view.screen_text(100, 40);
    assert!(ask.contains("Leave without saving"), "{ask}");
    assert!(ask.contains("< Stay >"), "{ask}");
    press(&mut view, Key::Escape);
    // F1 lists the keymap the theme left: here back is on F2.
    let link = view.link().clone();
    link.answer_shell(ShellInfo {
        keys: BTreeMap::from([("back".to_string(), "f2".to_string())]),
        ..ShellInfo::default()
    });
    let mut themed = AppView::with_session(Session::start(GALLERY_LIST_PATH), link);
    themed.set_clock("Tue 14:32");
    themed.run_requests();
    press(&mut themed, Key::F(1));
    let help = themed.screen_text(100, 40);
    assert!(help.contains("Back             F2"), "{help}");
    assert!(!help.contains("F3"), "{help}");
    press(&mut themed, Key::Escape);
    let line = themed.screen_text(100, 40);
    assert!(line.contains("F2=Back"), "{line}");
}

#[test]
fn control_characters_in_a_value_never_reach_the_terminal() {
    let json = "{\"v\":1,\"screen\":{\"path\":\"/x\",\"title\":\"X\"},\"c\":[{\"t\":\"text\",\"id\":\"t\",\"text\":\"a\\u001b]52;c;Zm9v\\u0007b\\u202Ec\"}]}";
    let link = FakeLink::from_json(&[json]).unwrap();
    let mut view = view_of(ScreenModel::from_json(json).unwrap(), link);
    let text = view.screen_text(80, 20);
    assert!(text.contains("a]52;c;Zm9vbc"), "{text}");
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'), "{text}");
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
