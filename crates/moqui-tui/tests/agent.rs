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

//! The agent mode: one command line in, one settled answer out, with elements found by id, number or label.

use std::collections::BTreeMap;

use moqui_tui::{
    gallery_link, Agent, Choice, FakeLink, LinkCall, MenuItem, ShellInfo, GALLERY_DETAIL_PATH, GALLERY_LIST_PATH,
};

const GEO_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/geo", "title": "Geo" },
  "c": [ { "t": "form", "id": "Geo", "name": "Geo", "transitionUrl": "/geo/save", "c": [
    { "t": "field", "id": "countryGeoId", "name": "countryGeoId", "title": "Country",
      "w": { "kind": "dropDown", "value": "USA", "options": [ { "value": "USA", "label": "USA" }, { "value": "CAN", "label": "Canada" } ] } },
    { "t": "field", "id": "state", "name": "stateProvinceGeoId", "title": "State",
      "w": { "kind": "dropDown", "value": "CA", "options": [ { "value": "CA", "label": "California" } ],
             "optionsUrl": "/states", "dependsOn": { "countryGeoId": "countryGeoId" } } },
    { "t": "field", "id": "city", "name": "city", "title": "City", "w": { "kind": "textLine", "value": "" } }
  ] } ]
}"#;

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn an_answer_names_the_place_and_focus_and_ends_with_its_outcome() {
    let mut agent = Agent::open(gallery_link(), GALLERY_LIST_PATH, 100, 40);
    let answer = agent.run("look");
    assert!(answer.ok);
    let text = lines(&answer.text);
    assert_eq!(text[0], format!("path {GALLERY_LIST_PATH}"));
    assert!(text[1].starts_with("title "), "{}", answer.text);
    assert!(text[2].starts_with("focus "), "{}", answer.text);
    assert_eq!(text[3], "overlay none");
    assert_eq!(text.last(), Some(&"--- ok"));
    // The focused element is drawn in reverse video, so it is between [[ and ]].
    assert!(answer.text.contains("[["), "{}", answer.text);
}

#[test]
fn elements_are_found_by_label_id_and_number() {
    let mut agent = Agent::open(gallery_link(), GALLERY_LIST_PATH, 100, 40);
    let listed = agent.run("elements");
    assert!(listed.ok);
    let line = listed
        .text
        .lines()
        .find(|line| line.contains("openDetail"))
        .unwrap_or_else(|| panic!("{}", listed.text));
    assert!(line.contains("link") && line.contains("Open detail"), "{line}");
    let number: String = line[2..].chars().take_while(char::is_ascii_digit).collect();

    let focused = agent.run(&format!("focus #{number}"));
    assert!(focused.ok, "{}", focused.text);
    assert!(focused.text.contains("focus link openDetail"), "{}", focused.text);
    assert!(agent.run("focus openDetail").ok);
    let done = agent.run("do open detail");
    assert!(done.ok, "{}", done.text);
    assert_eq!(agent.session().path(), GALLERY_DETAIL_PATH);
    assert!(done.text.starts_with(&format!("path {GALLERY_DETAIL_PATH}")));
}

#[test]
fn set_types_a_field_and_chooses_a_drop_down_the_way_a_person_does() {
    let link = FakeLink::from_json(&[GEO_JSON]).unwrap();
    link.answer_options(
        "/states",
        BTreeMap::from([("countryGeoId".to_string(), "CAN".to_string())]),
        vec![Choice {
            value: "ON".to_string(),
            label: "Ontario".to_string(),
        }],
    );
    let mut agent = Agent::open(link.clone(), "/geo", 100, 30);
    let city = agent.run("set City Ottawa East");
    assert!(city.ok, "{}", city.text);
    assert_eq!(agent.session().buffer("city"), "Ottawa East");
    // By label; the dependent state's options are asked for and settled before the answer.
    let country = agent.run("set Country Canada");
    assert!(country.ok, "{}", country.text);
    assert_eq!(agent.session().buffer("countryGeoId"), "CAN");
    assert!(link
        .calls()
        .iter()
        .any(|call| matches!(call, LinkCall::Options { url, .. } if url == "/states")));
    // By value.
    let state = agent.run("set state ON");
    assert!(state.ok, "{}", state.text);
    assert_eq!(agent.session().buffer("state"), "ON");
    let elements = agent.run("elements");
    assert!(elements.text.contains("= \"ON\""), "{}", elements.text);
    assert!(elements.text.contains("[changed]"), "{}", elements.text);
    // A choice that is not there is an error, and the field keeps its value.
    let wrong = agent.run("set Country Atlantis");
    assert!(!wrong.ok);
    assert!(
        wrong
            .text
            .ends_with("--- error no choice of countryGeoId matches \"Atlantis\""),
        "{}",
        wrong.text
    );
    assert_eq!(agent.session().buffer("countryGeoId"), "CAN");
}

#[test]
fn a_palette_search_is_settled_before_the_answer() {
    let link = gallery_link();
    link.answer_shell(ShellInfo {
        menu: vec![MenuItem {
            title: "Gallery".to_string(),
            url: "/Gallery".to_string(),
            children: Vec::new(),
        }],
        ..ShellInfo::default()
    });
    link.answer_json(
        "/shellSearch",
        BTreeMap::from([
            ("text".to_string(), "so-1".to_string()),
            ("limit".to_string(), "5".to_string()),
        ]),
        serde_json::json!({ "groups": [{ "label": "Orders", "rows": [{ "title": "SO-10023", "url": "/o?orderId=SO-10023" }] }] }),
    );
    let mut agent = Agent::open(link, GALLERY_LIST_PATH, 100, 40);
    assert!(agent.run("key Ctrl+K").ok);
    let typed = agent.run("type so-1");
    assert!(typed.text.contains("SO-10023"), "{}", typed.text);
    assert!(typed.text.contains("overlay palette \"so-1\""), "{}", typed.text);
}

#[test]
fn json_answers_parse_and_errors_leave_the_session() {
    let mut agent = Agent::open(gallery_link(), GALLERY_LIST_PATH, 100, 40);
    let answer = agent.run("look --json");
    let value: serde_json::Value = serde_json::from_str(&answer.text).unwrap();
    assert_eq!(value["ok"], serde_json::json!(true));
    assert_eq!(value["path"], serde_json::json!(GALLERY_LIST_PATH));
    assert!(value["elements"]
        .as_array()
        .unwrap()
        .iter()
        .any(|element| element["id"] == "openDetail"));
    assert!(value["screen"].as_array().unwrap().len() > 3);

    let unknown = agent.run("dance");
    assert!(!unknown.ok);
    assert!(unknown.text.ends_with("--- error unknown command dance; try help"));
    let missing = agent.run("do Nowhere at all");
    assert!(!missing.ok);
    assert!(missing.text.contains("no element named"), "{}", missing.text);
    assert!(!agent.run("key NotAKey").ok);
    assert!(agent.run("help").text.contains("Keys on this screen:"));
    assert!(agent.run("size 160x50").ok);
    assert!(!agent.run("size big").ok);

    assert!(agent.run("close").ok);
    assert!(agent.closed());
    assert!(!agent.run("look").ok);
}

const CONFIRM_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/c", "title": "Confirm" },
  "c": [ { "t": "form", "id": "C", "name": "C", "transitionUrl": "/c/save", "c": [
    { "t": "field", "id": "C.note", "name": "note", "title": "Note", "w": { "kind": "textLine", "value": "" } },
    { "t": "field", "id": "C.go", "name": "go", "title": "Go", "w": { "kind": "submit", "text": "Go", "confirm": "Really?" } }
  ] } ]
}"#;

#[test]
fn a_confirm_is_answered_by_its_choice_and_empty_rows_are_counted() {
    let link = FakeLink::from_json(&[CONFIRM_JSON]).unwrap();
    link.answer_transition("/c/save", moqui_tui::TransitionOutcome::saved("Saved"));
    let mut agent = Agent::open(link.clone(), "/c", 100, 40);
    let asked = agent.run("do Go");
    assert!(
        asked
            .text
            .contains("overlay confirm \"Really?\", choices No|Yes, at \"No\""),
        "{}",
        asked.text
    );
    let wrong = agent.run("do Maybe");
    assert!(
        wrong.text.ends_with("--- error the confirm's choices are No, Yes"),
        "{}",
        wrong.text
    );
    let saved = agent.run("do yes");
    assert!(saved.ok, "{}", saved.text);
    assert!(saved.text.contains("message Saved"), "{}", saved.text);
    assert!(link
        .calls()
        .iter()
        .any(|call| matches!(call, LinkCall::Transition { url, .. } if url == "/c/save")));
    // The rows between the form and the key line are one counted line, not thirty empty ones.
    assert!(saved.text.contains("empty rows)"), "{}", saved.text);
    assert!(!saved.text.contains("\n\n\n"), "{}", saved.text);
}

/// The login dialog's fields are elements while it is on top: `set` fills them, `do Log in` sends it, and the
/// password is never in an answer.
#[test]
fn the_login_dialog_is_filled_by_set_and_sent_by_do() {
    let link = gallery_link();
    link.require_login("secret-pw");
    link.require_code("123456", Vec::new());
    let mut agent = Agent::open(link.clone(), GALLERY_LIST_PATH, 100, 40);
    let listed = agent.run("elements");
    assert!(listed.text.contains("login.password"), "{}", listed.text);
    assert!(agent.run("set Username john").ok);
    let answer = agent.run("set Password secret-pw");
    assert!(answer.ok, "{}", answer.text);
    assert!(!answer.text.contains("secret-pw"), "{}", answer.text);
    let json = agent.run("elements --json");
    assert!(!json.text.contains("secret-pw"), "{}", json.text);
    assert!(json.text.contains("•••••••••"), "{}", json.text);
    // The right password wants the code: an error answer that says so, the dialog still open.
    let wanted = agent.run("do Log in");
    assert!(!wanted.ok, "{}", wanted.text);
    assert!(wanted.text.contains("Enter the code"), "{}", wanted.text);
    assert!(agent.run("set Code 123456").ok);
    let done = agent.run("do Log in");
    assert!(done.ok, "{}", done.text);
    assert!(done.text.contains("overlay none"), "{}", done.text);
    assert!(link
        .calls()
        .iter()
        .any(|call| matches!(call, LinkCall::Login { username, code, .. } if username == "john" && code == "123456")));
}
