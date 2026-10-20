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

//! Key merge, level stack, submit, paging, and the options lane. No Revue.

use std::collections::BTreeMap;

use moqui_tui::{
    gallery_link, merge_keys, Action, Choice, FakeLink, Key, KeyMap, KeyPress, LinkCall, Messages, OptionLane,
    ScreenLink, ScreenModel, Session, TransitionOutcome, ValidationError, GALLERY_DETAIL_PATH, GALLERY_LIST_PATH,
};

const EDIT_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/edit", "title": "Edit" },
  "c": [
    {
      "t": "form",
      "id": "Edit",
      "name": "Edit",
      "transitionUrl": "/edit/update",
      "submitMessage": "Kept",
      "hidden": [{ "name": "partyId", "value": "P1" }],
      "c": [
        {
          "t": "field",
          "id": "Edit.name",
          "name": "name",
          "title": "Name",
          "required": true,
          "maxlength": 4,
          "w": { "kind": "textLine", "value": "" }
        },
        {
          "t": "field",
          "id": "Edit.password",
          "name": "password",
          "title": "Password",
          "w": { "kind": "password" }
        },
        {
          "t": "field",
          "id": "Edit.note",
          "name": "note",
          "title": "Note",
          "w": { "kind": "textLine", "value": "keep" }
        },
        {
          "t": "field",
          "id": "Edit.save",
          "name": "save",
          "title": "Save",
          "w": { "kind": "submit", "text": "Save" }
        }
      ]
    },
    {
      "t": "form",
      "id": "Other",
      "name": "Other",
      "transitionUrl": "/other",
      "c": [
        {
          "t": "field",
          "id": "Other.extra",
          "name": "extra",
          "title": "Extra",
          "w": { "kind": "textLine", "value": "" }
        }
      ]
    }
  ]
}"#;

const LEAVE_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/leave", "title": "Leave" },
  "c": [
    {
      "t": "form",
      "id": "Edit",
      "name": "Edit",
      "transitionUrl": "/leave/update",
      "c": [
        {
          "t": "field",
          "id": "Edit.note",
          "name": "note",
          "title": "Note",
          "w": { "kind": "textLine", "value": "" }
        }
      ]
    },
    { "t": "link", "id": "away", "text": "Away", "url": "/away" }
  ]
}"#;

const AWAY_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/away", "title": "Away" },
  "c": []
}"#;

const PARTY_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/Party", "title": "Party" },
  "c": [{ "t": "link", "id": "find", "text": "Find", "url": "/Party/FindParty" }]
}"#;

const FIND_PARTY_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/Party/FindParty", "title": "Find" },
  "c": [{ "t": "link", "id": "edit", "text": "Edit", "url": "/Party/EditParty" }]
}"#;

const EDIT_PARTY_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/Party/EditParty", "title": "Edit" },
  "c": []
}"#;

const GEO_JSON: &str = r#"{
  "v": 1,
  "screen": { "path": "/geo", "title": "Geo" },
  "c": [
    {
      "t": "form",
      "id": "Geo",
      "name": "Geo",
      "c": [
        {
          "t": "field",
          "id": "countryGeoId",
          "name": "countryGeoId",
          "title": "Country",
          "w": {
            "kind": "dropDown",
            "value": "USA",
            "options": [
              { "value": "USA", "label": "USA" },
              { "value": "CAN", "label": "Canada" }
            ]
          }
        },
        {
          "t": "field",
          "id": "state",
          "name": "stateProvinceGeoId",
          "title": "State",
          "w": {
            "kind": "dropDown",
            "value": "CA",
            "options": [{ "value": "CA", "label": "California" }],
            "optionsUrl": "/states",
            "dependsOn": ["countryGeoId"],
            "optionsParameters": ["countryGeoId"]
          }
        }
      ]
    }
  ]
}"#;

fn press(session: &mut Session, link: &FakeLink, key: Key) -> bool {
    session.on_key(&KeyPress::plain(key), link)
}

#[test]
fn a_busy_submit_does_not_send_again() {
    let (mut session, link) = edit_session();
    session.set_buffer("Edit.name", "Ann".to_string());
    session.set_busy(true);
    press(&mut session, &link, Key::Enter);
    assert!(link.calls().is_empty());
}

fn edit_session() -> (Session, FakeLink) {
    let model = ScreenModel::from_json(EDIT_JSON).unwrap();
    let link = FakeLink::from_json(&[EDIT_JSON]).unwrap();
    (Session::new(model), link)
}

#[test]
fn merge_keys_skips_a_bad_chord_an_unknown_action_and_a_fixed_key() {
    let (map, skipped) = merge_keys(
        &KeyMap::baseline(),
        &[
            ("fly", "f2"),
            ("reload", "not a chord"),
            ("reload", "esc"),
            ("back", "f2"),
        ],
    );
    assert_eq!(skipped[0].reason, "unknown action");
    assert_eq!(skipped[1].reason, "bad chord");
    assert_eq!(skipped[1].chord, "not a chord");
    assert_eq!(skipped[2].reason, "fixed key");
    assert_eq!(map.action(&KeyPress::plain(Key::F(5))), Some(Action::Reload));
    assert_eq!(map.action(&KeyPress::plain(Key::Escape)), Some(Action::Cancel));
    assert_eq!(map.action(&KeyPress::plain(Key::F(2))), Some(Action::Back));
    assert_eq!(map.action(&KeyPress::plain(Key::F(3))), None);
    assert_eq!(map.action(&KeyPress::plain(Key::Enter)), Some(Action::Activate));
    assert_eq!(map.action(&KeyPress::plain(Key::Tab)), Some(Action::Next));
}

#[test]
fn a_parent_level_stays_when_the_next_path_is_still_under_it() {
    let link = FakeLink::from_json(&[PARTY_JSON, FIND_PARTY_JSON, EDIT_PARTY_JSON]).unwrap();
    let mut session = Session::new(ScreenModel::from_json(PARTY_JSON).unwrap());
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.paths(), vec!["/Party", "/Party/FindParty"]);
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.paths(), vec!["/Party", "/Party/EditParty"]);
    let level_paths: Vec<_> = link
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            LinkCall::Level { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(
        level_paths,
        vec!["/Party/FindParty".to_string(), "/Party/EditParty".to_string()]
    );
}

#[test]
fn gallery_list_to_detail_replaces_the_only_level() {
    let link = gallery_link();
    let mut session = Session::new(link.level(GALLERY_LIST_PATH, &BTreeMap::new()).unwrap());
    for _ in 0..4 {
        press(&mut session, &link, Key::Tab);
    }
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.paths(), vec![GALLERY_DETAIL_PATH]);
}

#[test]
fn an_empty_required_field_is_marked_and_not_sent() {
    let (mut session, link) = edit_session();
    assert_eq!(session.focus_id(), "Edit.name");
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.mark("Edit.name"), Some("required"));
    assert!(link.calls().is_empty());

    session.set_buffer("Edit.name", "abcde".to_string());
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.mark("Edit.name"), Some("maxlength"));
    assert!(link.calls().is_empty());
}

#[test]
fn a_valid_submit_clears_that_form_and_keeps_another_forms_edit() {
    let (mut session, link) = edit_session();
    link.answer_transition("/edit/update", TransitionOutcome::saved("Saved"));
    session.set_buffer("Edit.name", "Ann".to_string());
    session.set_buffer("Edit.password", "s3cret".to_string());
    session.set_buffer("Other.extra", "x".to_string());
    press(&mut session, &link, Key::Enter);

    let mut calls = link.calls();
    // Staying on the screen reloads it, so the form shows what the server saved.
    assert!(matches!(calls.pop(), Some(LinkCall::Level { path, .. }) if path == "/edit"));
    let Some(LinkCall::Transition { url, method, fields }) = calls.pop() else {
        panic!("transition");
    };
    assert_eq!(url, "/edit/update");
    assert_eq!(method, "post");
    assert_eq!(fields.get("name").map(String::as_str), Some("Ann"));
    assert_eq!(fields.get("note").map(String::as_str), Some("keep"));
    assert_eq!(fields.get("partyId").map(String::as_str), Some("P1"));
    assert!(!fields.contains_key("password"));
    assert!(!fields.contains_key("save"));
    assert!(!fields.contains_key("extra"));
    assert!(!session.is_dirty("Edit.name"));
    assert_eq!(session.buffer("Edit.name"), "");
    assert!(session.is_dirty("Other.extra"));
    assert_eq!(session.buffer("Other.extra"), "x");
    assert_eq!(session.message(), "Saved");
}

#[test]
fn a_service_error_keeps_the_edits_and_stays() {
    let (mut session, link) = edit_session();
    link.answer_transition(
        "/edit/update",
        TransitionOutcome {
            screen_url: String::new(),
            messages: Messages {
                errors: vec!["Party is locked".to_string()],
                ..Messages::default()
            },
            parameters: BTreeMap::new(),
        },
    );
    session.set_buffer("Edit.name", "Ann".to_string());
    press(&mut session, &link, Key::Enter);
    assert!(session.is_dirty("Edit.name"));
    assert_eq!(session.buffer("Edit.name"), "Ann");
    assert_eq!(session.message(), "Party is locked");
    assert!(!link.calls().iter().any(|call| matches!(call, LinkCall::Level { .. })));
}

#[test]
fn a_cleared_optional_field_posts_empty() {
    let (mut session, link) = edit_session();
    session.set_buffer("Edit.name", "Ann".to_string());
    session.set_buffer("Edit.note", String::new());
    press(&mut session, &link, Key::Enter);
    let Some(LinkCall::Transition { fields, .. }) = link
        .calls()
        .into_iter()
        .find(|call| matches!(call, LinkCall::Transition { .. }))
    else {
        panic!("transition");
    };
    assert_eq!(fields.get("note").map(String::as_str), Some(""));
}

#[test]
fn save_on_a_leave_from_a_link_posts_the_edited_form() {
    let model = ScreenModel::from_json(LEAVE_JSON).unwrap();
    let link = FakeLink::from_json(&[LEAVE_JSON, AWAY_JSON]).unwrap();
    let mut session = Session::new(model);
    session.set_buffer("Edit.note", "hello".to_string());
    press(&mut session, &link, Key::Tab);
    assert_eq!(session.focus_id(), "away");
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.confirm_choice(), Some("Stay"));
    press(&mut session, &link, Key::Right);
    press(&mut session, &link, Key::Right);
    assert_eq!(session.confirm_choice(), Some("Save"));
    press(&mut session, &link, Key::Enter);

    let posted = link.calls().into_iter().find_map(|call| match call {
        LinkCall::Transition { url, fields, .. } => Some((url, fields)),
        _ => None,
    });
    let (url, fields) = posted.expect("transition");
    assert_eq!(url, "/leave/update");
    assert_eq!(fields.get("note").map(String::as_str), Some("hello"));
    assert_eq!(session.path(), "/away");
}

#[test]
fn server_validation_focuses_the_first_bad_field() {
    let (mut session, link) = edit_session();
    link.answer_transition(
        "/edit/update",
        TransitionOutcome::invalid(vec![ValidationError {
            form: "Edit".to_string(),
            field: "name".to_string(),
            message: "taken".to_string(),
        }]),
    );
    session.set_buffer("Edit.name", "Ann".to_string());
    press(&mut session, &link, Key::Tab);
    assert_eq!(session.focus_id(), "Edit.password");
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.focus_id(), "Edit.name");
    assert_eq!(session.mark("Edit.name"), Some("taken"));
    assert_eq!(session.message(), "");
}

#[test]
fn paging_sends_page_index_and_sort_keeps_it() {
    let link = gallery_link();
    let mut session = Session::new(link.level(GALLERY_LIST_PATH, &BTreeMap::new()).unwrap());
    press(&mut session, &link, Key::Tab);
    press(&mut session, &link, Key::F(8));
    assert_eq!(session.page_index("EnumList"), 1);
    assert_eq!(session.list_page("EnumList")[0].cells[0].text(), "ENUM_10");

    press(&mut session, &link, Key::Tab);
    press(&mut session, &link, Key::Tab);
    assert_eq!(session.focus_id(), "EnumList");
    press(&mut session, &link, Key::Char('s'));
    press(&mut session, &link, Key::Char('s'));
    press(&mut session, &link, Key::Char('s'));

    let mut sorts = Vec::new();
    for call in link.calls() {
        if let LinkCall::Level { parameters, .. } = call {
            if parameters.contains_key("pageIndex") || parameters.contains_key("orderByField") {
                sorts.push(parameters);
            }
        }
    }
    assert_eq!(sorts[0].get("pageIndex").map(String::as_str), Some("1"));
    assert_eq!(sorts[1].get("orderByField").map(String::as_str), Some("enumId"));
    assert_eq!(sorts[1].get("pageIndex").map(String::as_str), Some("1"));
    assert_eq!(sorts[2].get("orderByField").map(String::as_str), Some("-enumId"));
    assert_eq!(sorts[3].get("orderByField").map(String::as_str), Some("description"));
    assert_eq!(sorts[3].get("pageIndex").map(String::as_str), Some("1"));
}

#[test]
fn find_opens_on_f_and_closes_on_escape() {
    let link = gallery_link();
    let mut session = Session::new(link.level(GALLERY_LIST_PATH, &BTreeMap::new()).unwrap());
    for _ in 0..3 {
        press(&mut session, &link, Key::Tab);
    }
    press(&mut session, &link, Key::Char('f'));
    assert!(session.find_open());
    press(&mut session, &link, Key::Escape);
    assert!(!session.find_open());
}

#[test]
fn an_unsaved_leave_stays_until_discard() {
    let link = gallery_link();
    let mut session = Session::new(link.level(GALLERY_LIST_PATH, &BTreeMap::new()).unwrap());
    press(&mut session, &link, Key::Tab);
    press(&mut session, &link, Key::Char('z'));
    assert!(session.is_dirty("description"));
    for _ in 0..3 {
        press(&mut session, &link, Key::Tab);
    }
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.confirm_choice(), Some("Stay"));
    assert_eq!(session.path(), GALLERY_LIST_PATH);
    press(&mut session, &link, Key::Escape);
    assert_eq!(session.confirm_choice(), None);
    assert!(session.is_dirty("description"));

    press(&mut session, &link, Key::Enter);
    press(&mut session, &link, Key::Right);
    assert_eq!(session.confirm_choice(), Some("Discard"));
    press(&mut session, &link, Key::Enter);
    assert_eq!(session.path(), GALLERY_DETAIL_PATH);
    assert!(!session.is_dirty("description"));
}

#[test]
fn a_superseded_options_request_is_dropped() {
    let mut lane = OptionLane::default();
    let mut first_params = BTreeMap::new();
    first_params.insert("countryGeoId".to_string(), "USA".to_string());
    let mut next_params = BTreeMap::new();
    next_params.insert("countryGeoId".to_string(), "CAN".to_string());
    let first = lane
        .push("state".to_string(), "/states".to_string(), first_params)
        .expect("idle lane sends");
    assert!(lane
        .push("state".to_string(), "/states".to_string(), next_params.clone())
        .is_none());
    let (apply, pending) = lane.finish(first.generation);
    assert!(!apply);
    let pending = pending.expect("pending request");
    assert_eq!(pending.parameters, next_params);
    let (apply, pending) = lane.finish(pending.generation);
    assert!(apply);
    assert!(pending.is_none());
}

#[test]
fn one_country_change_fetches_states_once_and_clears_a_missing_value() {
    let model = ScreenModel::from_json(GEO_JSON).unwrap();
    let link = FakeLink::from_json(&[GEO_JSON]).unwrap();
    let mut canada = BTreeMap::new();
    canada.insert("countryGeoId".to_string(), "CAN".to_string());
    link.answer_options(
        "/states",
        canada.clone(),
        vec![Choice {
            value: "ON".to_string(),
            label: "Ontario".to_string(),
        }],
    );
    let mut session = Session::new(model);
    assert_eq!(session.buffer("state"), "CA");
    session.set_buffer("countryGeoId", "CAN".to_string());
    session.changed("countryGeoId", &link);

    let options: Vec<_> = link
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            LinkCall::Options { url, parameters } => Some((url, parameters)),
            _ => None,
        })
        .collect();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].0, "/states");
    assert_eq!(options[0].1, canada);
    assert_eq!(session.buffer("state"), "");
    let widget = session.field_widget("state").unwrap();
    let moqui_tui::Widget::DropDown { options, .. } = widget else {
        panic!("drop-down");
    };
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].value, "ON");
}
