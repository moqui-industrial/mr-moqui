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

use moqui_tui::{
    dotted_label, frame, label_column_width, outline, Node, ScreenMeta, ScreenModel, WidthClass, GALLERY_DETAIL_JSON,
    GALLERY_LIST_JSON,
};

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
            ..Default::default()
        },
        c: (0..3)
            .map(|index| Node::Box {
                id: format!("b{index}"),
                title: format!("Box{index}"),
                kind: String::new(),
                initial: String::new(),
                c: Vec::new(),
            })
            .collect(),
        ..Default::default()
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

#[test]
fn frame_docks_a_drawer_only_when_the_leftover_canvas_is_at_least_120() {
    let narrow = frame(100);
    assert_eq!(narrow.drawer, 0);
    assert_eq!(narrow.canvas, 100);
    assert_eq!(narrow.class, WidthClass::Sm);

    let wide = frame(180);
    assert_eq!(wide.drawer, 26);
    assert_eq!(wide.canvas, 154);
    assert_eq!(wide.class, WidthClass::Md);

    let docked = frame(150);
    assert_eq!(docked.drawer, 26);
    assert_eq!(docked.canvas, 124);
    assert_eq!(docked.class, WidthClass::Md);

    let hidden = frame(149);
    assert_eq!(hidden.drawer, 0);
    assert_eq!(hidden.canvas, 149);
    assert_eq!(hidden.class, WidthClass::Md);
}

#[test]
fn labels_pad_to_the_longest_title_and_sit_above_on_xs() {
    assert_eq!(label_column_width(&["Type", "Description"], WidthClass::Sm), 11);
    assert_eq!(label_column_width(&["Type", "Description"], WidthClass::Xs), 0);
    assert_eq!(dotted_label("Type", 11), "Type.......");
    assert_eq!(dotted_label("Type", 0), "Type");
}

#[test]
fn a_docked_180_column_frame_places_detail_boxes_side_by_side() {
    let model = ScreenModel::from_json(GALLERY_DETAIL_JSON).unwrap();
    let wide = outline(&model, frame(180).canvas);
    assert!(wide.contains("class=md canvas=154\n"), "{wide}");
    assert!(wide.contains("row\n"), "{wide}");
    assert!(wide.contains("box Record x=0 w=76\n"), "{wide}");
    assert!(wide.contains("box Type x=77 w=77\n"), "{wide}");

    let narrow = outline(&model, frame(100).canvas);
    assert!(narrow.starts_with("class=sm canvas=100\n"), "{narrow}");
    assert!(!narrow.contains("\nrow\n"), "{narrow}");
}

#[test]
fn row_columns_past_twelve_spans_wrap_to_a_new_line() {
    let cols: Vec<String> = (0..4)
        .map(|index| format!(r#"{{"t":"col","id":"c{index}","md":"6","c":[{{"t":"text","text":"Col{index}"}}]}}"#))
        .collect();
    let json = format!(
        r#"{{"v":1,"screen":{{"path":"/grid","title":"Grid"}},"c":[{{"t":"row","id":"r","c":[{}]}}]}}"#,
        cols.join(",")
    );
    let model = ScreenModel::from_json(&json).unwrap();
    assert_eq!(
        outline(&model, 154),
        "\
class=md canvas=154
row
col x=0 w=76
  text Col0
col x=77 w=76
  text Col1
row
col x=0 w=76
  text Col2
col x=77 w=76
  text Col3
"
    );
}
