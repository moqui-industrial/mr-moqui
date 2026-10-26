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

use super::field_path_to_list;

#[test]
fn a_post_query_condition_compares_by_the_document_value_type() {
    use moqui_lib::object::Value;
    assert!(super::typed_equals(&Value::Decimal("5.00".parse().unwrap()), "5"));
    assert!(super::typed_equals(&Value::I64(7), "7"));
    assert!(!super::typed_equals(&Value::I64(7), "8"));
    assert!(super::typed_equals(&Value::Bool(true), "true"));
    assert!(super::typed_equals(&Value::string("Y"), "Y"));
}

#[test]
fn a_date_expression_returning_epoch_millis_is_a_timestamp() {
    use moqui_lib::object::Value;
    let field = |field_type: &str| super::DocField {
        path_element: "(x)".into(),
        alias: "x".into(),
        field_type: Some(field_type.into()),
        entity_type: None,
        sortable: false,
        function: None,
        is_expression: true,
        search_role: None,
        search_weight: None,
        default_display: None,
    };
    let stamp = super::expression_value(&field("date"), Value::string("1791979200000"));
    assert!(matches!(stamp, Value::Timestamp(_)), "{stamp:?}");
    assert!(matches!(
        super::expression_value(&field("date"), Value::F64(1791979200000.0)),
        Value::Timestamp(_)
    ));
    assert_eq!(
        super::expression_value(&field("double"), Value::string("12")),
        Value::string("12")
    );
}

#[test]
fn field_paths_keep_expression_colons() {
    assert_eq!(field_path_to_list("orderId"), vec!["orderId"]);
    assert_eq!(
        field_path_to_list("part:vendorRoles:roleTypeId"),
        vec!["part", "vendorRoles", "roleTypeId"]
    );
    assert_eq!(
        field_path_to_list("issuances:(acquireCost ? issuedQuantity * acquireCost : null)"),
        vec!["issuances", "(acquireCost ? issuedQuantity * acquireCost : null)"]
    );
    assert_eq!(field_path_to_list("(a ?: b)"), vec!["(a ?: b)"]);
}

fn create_row(facade: &crate::entity::EntityFacade, entity: &str, pairs: &[(&str, &str)]) {
    let mut row = facade.make_value(entity).unwrap();
    for (name, value) in pairs {
        row.set(*name, moqui_lib::object::Value::string(*value));
    }
    row.create(facade).unwrap();
}

#[test]
fn feed_pages_by_key_so_rows_written_meanwhile_are_not_skipped_or_repeated() {
    let facade = crate::testing::open();
    create_row(
        &facade,
        super::DOCUMENT,
        &[
            ("dataDocumentId", "PAGED"),
            ("primaryEntityName", "moqui.test.TestEntity"),
        ],
    );
    for (seq, path) in [("01", "testId"), ("02", "testMedium")] {
        create_row(
            &facade,
            super::DOCUMENT_FIELD,
            &[("dataDocumentId", "PAGED"), ("fieldSeqId", seq), ("fieldPath", path)],
        );
    }
    for id in ["A", "C", "E", "G"] {
        create_row(&facade, "moqui.test.TestEntity", &[("testId", id), ("testMedium", id)]);
    }
    let mut seen = Vec::new();
    let mut pages = 0;
    let count = super::feed_documents(&facade, "PAGED", None, None, 2, |docs| {
        pages += 1;
        seen.extend(
            docs.iter()
                .map(|doc| doc.get("_id").map(super::to_plain_string).unwrap_or_default()),
        );
        if pages == 1 {
            // One row before the read position, which an offset would read again, and one after it.
            create_row(
                &facade,
                "moqui.test.TestEntity",
                &[("testId", "B"), ("testMedium", "B")],
            );
            create_row(
                &facade,
                "moqui.test.TestEntity",
                &[("testId", "F"), ("testMedium", "F")],
            );
        }
        Ok(true)
    })
    .unwrap();
    assert_eq!(seen, ["A", "C", "E", "F", "G"]);
    assert_eq!(count, 5);
}

/// Java's `hasAllPrimaryPks`: the rows of one primary record merge into one document only when every primary key
/// field is a field alias; otherwise each row is a document.
#[test]
fn rows_merge_only_when_every_primary_key_is_aliased() {
    let facade = crate::testing::open();
    create_row(
        &facade,
        "moqui.basic.EnumerationType",
        &[("enumTypeId", "T1"), ("description", "type")],
    );
    for id in ["E1", "E2"] {
        create_row(
            &facade,
            "moqui.basic.Enumeration",
            &[("enumId", id), ("enumTypeId", "T1")],
        );
    }
    for (id, paths) in [
        ("MERGED", &["enumTypeId", "description", "enums:enumId"][..]),
        ("ROWS", &["description", "enums:enumId"][..]),
    ] {
        create_row(
            &facade,
            super::DOCUMENT,
            &[
                ("dataDocumentId", id),
                ("primaryEntityName", "moqui.basic.EnumerationType"),
            ],
        );
        for (at, path) in paths.iter().enumerate() {
            let seq = format!("{:02}", at + 1);
            create_row(
                &facade,
                super::DOCUMENT_FIELD,
                &[("dataDocumentId", id), ("fieldSeqId", &seq), ("fieldPath", path)],
            );
        }
    }
    let merged = super::data_documents(&facade, "MERGED", None, None, None).unwrap();
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert!(matches!(merged[0].get("enums"), Some(moqui_lib::object::Value::List(list)) if list.len() == 2));
    assert_eq!(merged[0].get("_id"), Some(&moqui_lib::object::Value::string("T1")));
    let rows = super::data_documents(&facade, "ROWS", None, None, None).unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    // As Java's `makeDocId`, a one-field key that is not a field of the document gives no `_id`.
    assert!(rows.iter().all(|doc| doc.get("_id").is_none()), "{rows:?}");
    assert!(rows
        .iter()
        .all(|doc| matches!(doc.get("enums"), Some(moqui_lib::object::Value::List(list)) if list.len() == 1)));
}

#[test]
fn a_document_definition_write_drops_the_cached_info() {
    let facade = crate::testing::open();
    create_row(
        &facade,
        super::DOCUMENT,
        &[
            ("dataDocumentId", "CACHED"),
            ("primaryEntityName", "moqui.test.TestEntity"),
        ],
    );
    let before = super::document_info(&facade, "CACHED").unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &before,
        &super::document_info(&facade, "CACHED").unwrap()
    ));
    create_row(
        &facade,
        super::DOCUMENT_FIELD,
        &[
            ("dataDocumentId", "CACHED"),
            ("fieldSeqId", "01"),
            ("fieldPath", "testMedium"),
        ],
    );
    let after = super::document_info(&facade, "CACHED").unwrap();
    assert!(!std::sync::Arc::ptr_eq(&before, &after), "the field write was seen");
}
