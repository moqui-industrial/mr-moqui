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

//! Screen model round trip. Unknown fields are ignored. A version other than 1 is refused.

use moqui_tui::{
    gallery_link, Cell, ModelError, Node, ScreenLink, ScreenModel, GALLERY_DETAIL_JSON, GALLERY_DETAIL_PATH,
    GALLERY_LIST_JSON, GALLERY_LIST_PATH,
};

#[test]
fn gallery_fixtures_round_trip() {
    for text in [GALLERY_LIST_JSON, GALLERY_DETAIL_JSON] {
        let model = ScreenModel::from_json(text).unwrap_or_else(|err| panic!("{err}"));
        let again = serde_json::to_string(&model).unwrap();
        let parsed = ScreenModel::from_json(&again).unwrap();
        assert_eq!(model, parsed);
    }
    let link = gallery_link();
    assert_eq!(link.level(GALLERY_LIST_PATH).unwrap().screen.title, "Enumerations");
    assert_eq!(
        link.level(GALLERY_DETAIL_PATH).unwrap().screen.path,
        GALLERY_DETAIL_PATH
    );
}

#[test]
fn list_rows_carry_the_unicode_cells() {
    let model = ScreenModel::from_json(GALLERY_LIST_JSON).unwrap();
    let Node::FormList { list } = &model.c[1] else {
        panic!("list fixture child 1 is the form list");
    };
    assert_eq!(list.rows.len(), 20);
    assert_eq!(list.paging.count, 20);
    assert_eq!(list.rows[0].cells[0].text(), "ENUM_00");
    assert_eq!(list.rows[0].cells[0].link_url(), Some(GALLERY_DETAIL_PATH));
    assert_eq!(list.rows[0].cells[3].text(), "東京");
    assert_eq!(list.rows[1].cells[3].text(), "🎉");
    assert_eq!(list.rows[2].cells[3].text(), "👨\u{200D}👩\u{200D}👧");
    assert_eq!(list.rows[3].cells[1].text().chars().count(), 45);
    assert_eq!(list.rows[4].cells[3].text(), "family-note");
    assert_eq!(list.rows[10].cells[0].text(), "ENUM_10");
    assert!(matches!(list.rows[10].cells[0], Cell::Text(_)));
}

#[test]
fn unknown_fields_are_ignored_and_a_new_version_is_refused() {
    let text = r#"{"v":1,"extra":true,"screen":{"path":"/t","title":"T","later":1},"c":[
        {"t":"field","id":"a","name":"a","title":"A","nope":2,"w":{"kind":"textLine","value":"v","extra":1}}
    ]}"#;
    let model = ScreenModel::from_json(text).unwrap();
    assert_eq!(model.screen.path, "/t");
    assert_eq!(model.c[0].id(), "a");
    let again = ScreenModel::from_json(&serde_json::to_string(&model).unwrap()).unwrap();
    assert_eq!(model, again);

    let err = ScreenModel::from_json(r#"{"v":2,"screen":{"path":"/t","title":""},"c":[]}"#).unwrap_err();
    assert_eq!(err, ModelError::Version(2));
}
