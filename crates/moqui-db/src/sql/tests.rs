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

use std::path::Path;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use crate::dialect::{Dialect, Dialects};
use crate::model::EntityModel;

use super::{
    add_column, count, create_foreign_key, create_index, create_table, delete, insert, insert_or_update,
    render_condition, select, select_view, update, Condition, FieldRef, LimitOffset, Nulls, OrderBy,
};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn framework() -> (EntityModel, Dialects) {
    let root = workspace_root();
    let conf = MNode::parse_file(root.join("framework/conf/MoquiDefaultConf.xml")).expect("conf");
    let model = EntityModel::load_directory(&conf, &root.join("framework/entity")).expect("model");
    let dialects = Dialects::load(&conf).expect("dialects");
    (model, dialects)
}

fn unquote(sql: &str) -> String {
    sql.replace(['"', '`'], "")
}

#[test]
fn postgres_enumeration_insert_select_and_count() {
    let (model, dialects) = framework();
    let enumeration = model.entity("moqui.basic.Enumeration").unwrap();
    let postgres = dialects.get("postgres").unwrap();

    let inserted = insert(enumeration, postgres, &["enumId", "enumTypeId", "description"]).unwrap();
    assert!(inserted.contains("Enumeration"), "{inserted}");
    assert!(inserted.contains('?'), "{inserted}");
    assert_eq!(
        inserted,
        "INSERT INTO \"Enumeration\" (\"enumId\", \"enumTypeId\", \"description\") VALUES (?, ?, ?)"
    );

    let condition = Condition::equals("enumTypeId", Value::string("GEO"));
    let order = [OrderBy::asc("enumTypeId").nulls(Nulls::Last)];
    let page = LimitOffset {
        limit: Some(20),
        offset: Some(0),
    };
    let selected = select(enumeration, postgres, &[], Some(&condition), &order, page).unwrap();
    assert!(unquote(&selected).contains("enumTypeId = ?"), "{selected}");
    assert!(selected.contains("ORDER BY"), "{selected}");
    assert!(selected.contains("NULLS LAST"), "{selected}");
    assert!(selected.contains("LIMIT ? OFFSET ?"), "{selected}");
    assert!(
        selected.starts_with("SELECT * FROM \"Enumeration\" WHERE "),
        "{selected}"
    );

    let counted = count(enumeration, postgres, Some(&condition)).unwrap();
    assert!(counted.starts_with("SELECT COUNT(*) FROM \"Enumeration\""), "{counted}");
    assert!(unquote(&counted).contains("enumTypeId = ?"), "{counted}");
    assert!(!counted.contains("ORDER BY"), "{counted}");
    assert!(!counted.contains("LIMIT"), "{counted}");
}

#[test]
fn insert_or_update_follows_never_try_insert() {
    let (model, dialects) = framework();
    let enumeration = model.entity("moqui.basic.Enumeration").unwrap();
    let postgres = dialects.get("postgres").unwrap();
    let mysql = dialects.get("mysql").unwrap();

    let err = insert_or_update(enumeration, postgres, &["enumId", "description"]).unwrap_err();
    assert!(err.to_string().contains("try-insert is off"), "{err}");

    let mysql_sql = insert_or_update(enumeration, mysql, &["enumId", "description"]).unwrap();
    assert!(mysql_sql.contains("ON DUPLICATE KEY UPDATE"), "{mysql_sql}");
    assert!(
        mysql_sql.contains("`description` = VALUES(`description`)"),
        "{mysql_sql}"
    );

    let mut postgres_try = postgres.clone();
    postgres_try.never_try_insert = false;
    let conflict = insert_or_update(enumeration, &postgres_try, &["enumId", "description"]).unwrap();
    assert!(conflict.contains("ON CONFLICT"), "{conflict}");
    assert!(conflict.contains("DO UPDATE SET"), "{conflict}");
    assert!(conflict.contains("EXCLUDED.\"description\""), "{conflict}");
}

#[test]
fn matches_uses_the_profile_fts_style() {
    let (model, dialects) = framework();
    let fts = model.entity("moqui.test.TestFts").unwrap();
    let postgres = dialects.get("postgres").unwrap();
    let mysql = dialects.get("mysql").unwrap();
    let condition = Condition::matches("body", Value::string("warehouse"));

    let pg = select(fts, postgres, &[], Some(&condition), &[], LimitOffset::default()).unwrap();
    assert!(pg.contains("to_tsvector"), "{pg}");
    assert!(pg.contains("plainto_tsquery"), "{pg}");
    assert!(
        pg.contains("to_tsvector('english', \"body\") @@ plainto_tsquery('english', ?)"),
        "{pg}"
    );

    let my = select(fts, mysql, &[], Some(&condition), &[], LimitOffset::default()).unwrap();
    assert!(my.contains("MATCH"), "{my}");
    assert!(my.contains("AGAINST"), "{my}");
    assert!(
        my.contains("MATCH (`body`) AGAINST (? IN NATURAL LANGUAGE MODE)"),
        "{my}"
    );

    let mut simple = postgres.clone();
    simple.fts_config = Some("simple".to_string());
    simple.never_try_insert = true;
    let rendered = render_condition(&condition, &simple).unwrap();
    assert!(rendered.contains("to_tsvector('simple', \"body\")"), "{rendered}");
}

#[test]
fn mysql_limit_order_and_name_replace() {
    let (model, dialects) = framework();
    let enumeration = model.entity("moqui.basic.Enumeration").unwrap();
    let mysql = dialects.get("mysql").unwrap();
    let order = [OrderBy::desc("sequenceNum").nulls(Nulls::First)];
    let both = select(
        enumeration,
        mysql,
        &[FieldRef::new("enumId")],
        Some(&Condition::equals("enumTypeId", Value::string("X"))),
        &order,
        LimitOffset {
            limit: Some(10),
            offset: Some(5),
        },
    )
    .unwrap();
    assert!(both.contains("LIMIT ?, ?"), "{both}");
    assert!(!both.contains("NULLS"), "{both}");
    assert!(both.contains("DESC"), "{both}");
    assert!(unquote(&both).contains("enumTypeId = ?"), "{both}");

    let limit_only = select(
        enumeration,
        mysql,
        &[],
        None,
        &[],
        LimitOffset {
            limit: Some(10),
            offset: None,
        },
    )
    .unwrap();
    assert!(limit_only.ends_with(" LIMIT ?"), "{limit_only}");

    let renamed = insert(enumeration, mysql, &["condition"]).unwrap();
    assert!(renamed.contains("`THE_CONDITION`"), "{renamed}");
    assert!(!renamed.contains("condition"), "{renamed}");
}

#[test]
fn empty_conditions_are_omitted_and_empty_in_is_false() {
    let (model, dialects) = framework();
    let enumeration = model.entity("moqui.basic.Enumeration").unwrap();
    let postgres = dialects.get("postgres").unwrap();
    let page = LimitOffset::default();

    let omitted = select(enumeration, postgres, &[], Some(&Condition::and(vec![])), &[], page).unwrap();
    assert!(!omitted.contains("WHERE"), "{omitted}");
    let or_omitted = select(enumeration, postgres, &[], Some(&Condition::or(vec![])), &[], page).unwrap();
    assert!(!or_omitted.contains("WHERE"), "{or_omitted}");

    let empty_in = select(
        enumeration,
        postgres,
        &[],
        Some(&Condition::is_in("enumId", vec![])),
        &[],
        page,
    )
    .unwrap();
    assert!(empty_in.contains("WHERE 1 = 0"), "{empty_in}");

    let empty_not_in = render_condition(&Condition::not_in("enumId", vec![]), postgres).unwrap();
    assert_eq!(empty_not_in, "1 = 1");

    let nested = render_condition(
        &Condition::or(vec![
            Condition::and(vec![]),
            Condition::negate(Condition::equals("enumId", Value::string("A"))),
        ]),
        postgres,
    )
    .unwrap();
    assert_eq!(nested, "NOT (\"enumId\" = ?)");

    let offset_only = select(
        enumeration,
        postgres,
        &[],
        None,
        &[],
        LimitOffset {
            limit: None,
            offset: Some(3),
        },
    )
    .unwrap();
    assert!(offset_only.ends_with(" OFFSET ?"), "{offset_only}");
    assert!(!offset_only.contains("LIMIT"), "{offset_only}");
}

#[test]
fn view_joins_key_map_and_optional_member() {
    let (model, dialects) = framework();
    let postgres = dialects.get("postgres").unwrap();
    let page = LimitOffset::default();

    let parent = model.entity("moqui.basic.ParentEnum").unwrap();
    let joined = select_view(parent, &model, postgres, None, &[], page).unwrap();
    assert!(joined.contains(" JOIN "), "{joined}");
    assert!(joined.contains(" ON "), "{joined}");
    assert!(!joined.contains("LEFT JOIN"), "{joined}");
    assert!(
        unquote(&joined).contains("ENUM.parentEnumId = ENUMP.enumId"),
        "{joined}"
    );

    let optional = model.entity("moqui.basic.EnumAndParent").unwrap();
    let left = select_view(optional, &model, postgres, None, &[], page).unwrap();
    assert!(left.contains("LEFT JOIN"), "{left}");
    assert!(left.contains(" ON "), "{left}");

    let only_rel = model.entity("moqui.basic.EnumerationAndType").unwrap();
    let rendered = select_view(only_rel, &model, postgres, None, &[], page).unwrap();
    assert!(rendered.contains("\"Enumeration\" \"ENUM\""), "{rendered}");
    assert!(rendered.contains(" JOIN "), "{rendered}");
    assert!(!rendered.contains("LEFT JOIN"), "{rendered}");
    let plain = unquote(&rendered);
    assert!(plain.contains("EnumerationType"), "{rendered}");
    assert!(plain.contains("ENUM.enumTypeId = ETP.enumTypeId"), "{rendered}");
    assert!(!plain.contains("1=1"), "{rendered}");
}

#[test]
fn hand_built_view_optional_empty_key_and_prefix() {
    let conf = MNode::parse_text(
        "dict",
        r#"<database-list>
            <dictionary-type type="id" java-type="java.lang.String" default-sql-type="VARCHAR(40)"/>
            <dictionary-type type="date-time" java-type="java.sql.Timestamp" default-sql-type="TIMESTAMP"/>
        </database-list>"#,
    )
    .unwrap();
    let doc = MNode::parse_text(
        "entities",
        r#"<entities>
            <entity entity-name="Left" package="sample">
                <field name="leftId" type="id" is-pk="true"/>
                <field name="label" type="id"/>
            </entity>
            <entity entity-name="Right" package="sample">
                <field name="rightId" type="id" is-pk="true"/>
                <field name="leftId" type="id"/>
            </entity>
            <view-entity entity-name="LeftRight" package="sample">
                <member-entity entity-alias="L" entity-name="sample.Left"/>
                <member-entity entity-alias="R" entity-name="sample.Right" join-from-alias="L" join-optional="true">
                    <key-map field-name="leftId"/>
                </member-entity>
                <member-entity entity-alias="X" entity-name="sample.Right" join-from-alias="L"/>
                <alias-all entity-alias="L"><exclude field="label"/></alias-all>
                <alias entity-alias="R" name="rightPk" field="rightId"/>
            </view-entity>
            <view-entity entity-name="Prefixed" package="sample">
                <member-entity entity-alias="L" entity-name="sample.Left"/>
                <member-entity entity-alias="R" entity-name="sample.Right" join-from-alias="L" join-optional="true"/>
                <alias-all entity-alias="R" prefix="other"/>
            </view-entity>
            <view-entity entity-name="OnlyRel" package="sample">
                <member-relationship entity-alias="R" join-from-alias="L" relationship="missing"/>
                <alias name="leftId" entity-alias="L" field="leftId"/>
            </view-entity>
        </entities>"#,
    )
    .unwrap();
    let model = EntityModel::load(&conf, &[doc]).unwrap();
    let postgres = Dialect {
        name: "postgres".to_string(),
        placeholder: "?",
        fts_style: Some("postgres-tsvector".to_string()),
        fts_config: Some("english".to_string()),
        join_style: None,
        offset_style: None,
        from_lateral_style: None,
        never_nulls: false,
        never_try_insert: true,
        use_binary_type_for_blob: false,
        constraint_name_clip_length: Some(60),
        lb_name: None,
        table_engine: None,
        character_set: None,
        collate: None,
        fk_style: None,
        default_test_query: None,
        result_fetch_size: None,
        sql_types: Default::default(),
        name_replaces: Vec::new(),
    };

    let view = model.entity("sample.LeftRight").unwrap();
    let sql = select_view(view, &model, &postgres, None, &[], LimitOffset::default()).unwrap();
    assert!(sql.contains("LEFT JOIN"), "{sql}");
    assert!(sql.contains(" JOIN \"Right\" \"X\" ON 1=1"), "{sql}");
    assert!(unquote(&sql).contains("L.leftId = R.leftId"), "{sql}");
    assert!(sql.contains("\"L\".\"leftId\""), "{sql}");
    assert!(!unquote(&sql).contains(".label"), "{sql}");
    assert!(
        unquote(&sql).contains("R.rightId AS rightPk") || sql.contains("AS \"rightPk\""),
        "{sql}"
    );

    let prefixed = model.entity("sample.Prefixed").unwrap();
    let prefixed_sql = select_view(prefixed, &model, &postgres, None, &[], LimitOffset::default()).unwrap();
    assert!(prefixed_sql.contains("LEFT JOIN"), "{prefixed_sql}");
    assert!(prefixed_sql.contains("ON 1=1"), "{prefixed_sql}");
    assert!(prefixed_sql.contains("AS \"otherRightId\""), "{prefixed_sql}");

    let only_rel = model.entity("sample.OnlyRel").unwrap();
    let err = select_view(only_rel, &model, &postgres, None, &[], LimitOffset::default()).unwrap_err();
    assert!(err.to_string().contains("has no member alias L"), "{err}");
}

#[test]
fn ddl_table_index_column_and_foreign_key() {
    let (model, dialects) = framework();
    let postgres = dialects.get("postgres").unwrap();
    let mysql = dialects.get("mysql").unwrap();

    let fts = model.entity("moqui.test.TestFts").unwrap();
    let pg_table = create_table(fts, &model, postgres).unwrap();
    assert!(pg_table.contains("CREATE TABLE"), "{pg_table}");
    assert!(pg_table.contains("\"body\" TEXT"), "{pg_table}");
    assert!(pg_table.contains("USING gin"), "{pg_table}");
    assert!(pg_table.contains("to_tsvector"), "{pg_table}");
    assert!(pg_table.contains("PRIMARY KEY"), "{pg_table}");
    assert!(!pg_table.contains("gin_trgm"), "{pg_table}");

    let my_table = create_table(fts, &model, mysql).unwrap();
    assert!(my_table.contains("FULLTEXT"), "{my_table}");
    assert!(my_table.contains("ENGINE=InnoDB"), "{my_table}");
    assert!(my_table.contains("`body` LONGTEXT"), "{my_table}");

    let factor = model.entity("moqui.security.UserAuthcFactor").unwrap();
    let factor_sql = create_table(factor, &model, postgres).unwrap();
    assert!(factor_sql.contains("\"userId\" VARCHAR(40) NOT NULL"), "{factor_sql}");

    let body = fts.field("body").unwrap();
    let added = add_column(fts, body, &model, postgres).unwrap();
    assert_eq!(added, "ALTER TABLE \"TestFts\" ADD \"body\" TEXT");

    let account = model.entity("moqui.security.UserAccount").unwrap();
    let index = account
        .indexes
        .iter()
        .find(|index| index.name == "USERACCT_USERNAME")
        .unwrap();
    let index_sql = create_index(account, index, postgres).unwrap();
    assert!(index_sql.contains("CREATE UNIQUE INDEX"), "{index_sql}");
    assert!(index_sql.contains("USERACCT_USERNAME"), "{index_sql}");
    assert!(unquote(&index_sql).contains("username"), "{index_sql}");

    let enumeration = model.entity("moqui.basic.Enumeration").unwrap();
    let relationship = enumeration.relationship("type").unwrap();
    let related = model.entity(&relationship.related).unwrap();
    let fk = create_foreign_key(enumeration, relationship, related, postgres).unwrap();
    assert!(fk.contains("REFERENCES"), "{fk}");
    assert!(fk.contains("EnumerationType"), "{fk}");
    assert!(fk.contains("ALTER TABLE"), "{fk}");
    assert!(fk.contains("FOREIGN KEY"), "{fk}");
    assert!(unquote(&fk).contains("enumTypeId"), "{fk}");

    let parent = enumeration.relationship("parent").unwrap();
    let err = create_foreign_key(enumeration, parent, enumeration, postgres).unwrap_err();
    assert!(err.to_string().contains("type=one"), "{err}");

    let updated = update(
        enumeration,
        postgres,
        &["description"],
        Some(&Condition::equals("enumId", Value::string("A"))),
    )
    .unwrap();
    assert!(
        updated.starts_with("UPDATE \"Enumeration\" SET \"description\" = ? WHERE "),
        "{updated}"
    );
    let deleted = delete(enumeration, postgres, Some(&Condition::is_null("description"))).unwrap();
    assert!(unquote(&deleted).contains("description IS NULL"), "{deleted}");
    assert!(deleted.starts_with("DELETE FROM \"Enumeration\" WHERE "), "{deleted}");
}
