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

//! Canvas outlines at 72, 100, and 180. The shell frame is Phase 2.

use moqui_tui::{outline, Node, ScreenMeta, ScreenModel, GALLERY_DETAIL_JSON, GALLERY_LIST_JSON};

#[test]
fn gallery_list_outlines() {
    let model = ScreenModel::from_json(GALLERY_LIST_JSON).unwrap();
    assert_eq!(
        outline(&model, 72),
        "\
class=xs canvas=72
form EnumFind
  field Type above
  field Description above
  field Find above
list EnumList scroll=true
  col Identifier w=10 fixed
  col Description w=40
  col Code w=15
  col Note w=11
  page_rows=10
link Open detail
"
    );
    assert_eq!(
        outline(&model, 100),
        "\
class=sm canvas=100
form EnumFind
  field Type beside
  field Description beside
  field Find beside
list EnumList scroll=false
  col Identifier w=10 fixed
  col Description w=40
  col Code w=15
  col Note w=11
  page_rows=10
link Open detail
"
    );
    assert_eq!(
        outline(&model, 180),
        "\
class=lg canvas=180
form EnumFind
  field Type beside
  field Description beside
  field Find beside
list EnumList scroll=false
  col Identifier w=10 fixed
  col Description w=40
  col Code w=15
  col Note w=11
  page_rows=10
link Open detail
"
    );
}

#[test]
fn gallery_detail_places_two_boxes_and_truncates_the_long_label() {
    let model = ScreenModel::from_json(GALLERY_DETAIL_JSON).unwrap();
    assert_eq!(
        outline(&model, 72),
        "\
class=xs canvas=72
box Record x=0 w=72
  field Enum ID above
  field Updated above
  field Status above
  field Enumeration description from seed data above
box Type x=0 w=72
  field Type above
  field Code above
  field Sample above
link Back to list
"
    );
    assert_eq!(
        outline(&model, 100),
        "\
class=sm canvas=100
box Record x=0 w=100
  field Enum ID beside
  field Updated beside
  field Status beside
  field Enumeration descr… beside
box Type x=0 w=100
  field Type beside
  field Code beside
  field Sample beside
link Back to list
"
    );
    assert_eq!(
        outline(&model, 180),
        "\
class=lg canvas=180
row
box Record x=0 w=89
  field Enum ID beside
  field Updated beside
  field Status beside
  field Enumeration description… beside
box Type x=90 w=90
  field Type beside
  field Code beside
  field Sample beside
link Back to list
"
    );
}

#[test]
fn three_boxes_stack_until_each_would_have_forty_columns() {
    let model = ScreenModel {
        v: 1,
        screen: ScreenMeta {
            path: "/boxes".to_string(),
            title: "Boxes".to_string(),
        },
        c: (0..3)
            .map(|index| Node::Box {
                id: format!("b{index}"),
                title: format!("Box{index}"),
                c: Vec::new(),
            })
            .collect(),
    };
    assert_eq!(
        outline(&model, 100),
        "\
class=sm canvas=100
box Box0 x=0 w=100
box Box1 x=0 w=100
box Box2 x=0 w=100
"
    );
    assert_eq!(
        outline(&model, 180),
        "\
class=lg canvas=180
row
box Box0 x=0 w=59
box Box1 x=60 w=59
box Box2 x=120 w=60
"
    );
}
