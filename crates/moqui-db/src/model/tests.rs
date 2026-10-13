use super::*;

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn framework_model() -> EntityModel {
    let root = workspace_root();
    let conf = MNode::parse_file(root.join("framework/conf/MoquiDefaultConf.xml")).unwrap();
    EntityModel::load_directory(&conf, &root.join("framework/entity")).unwrap()
}

#[test]
fn framework_entities_and_dictionary_load() {
    let model = framework_model();
    assert!(!model.full_names().is_empty());
    let id = model.dictionary_type("id").unwrap();
    assert_eq!(id.java_type, "java.lang.String");
    assert_eq!(id.default_sql_type, "VARCHAR(40)");
    assert!(model.dictionary_type("text-fts").is_some());

    let enumeration = model.entity("moqui.basic.Enumeration").unwrap();
    assert_eq!(model.entity("Enumeration").unwrap().full_name, enumeration.full_name);
    assert_eq!(enumeration.table_name, "Enumeration");
    assert_eq!(enumeration.entity_use, "configuration");
    assert_eq!(enumeration.cache, "true");
    assert_eq!(enumeration.sequence_bank_size, 50);
    assert_eq!(enumeration.sequence_primary_stagger, 1);
    assert_eq!(enumeration.sequence_secondary_padded_length, 2);
    assert!(enumeration.sequence_primary_prefix.is_none());
    assert!(!enumeration.sequence_primary_use_uuid);
    let stamp = enumeration.field("lastUpdatedStamp").unwrap();
    assert_eq!(stamp.field_type, "date-time");
    assert!(!stamp.is_pk);
    assert!(!stamp.not_null);
    assert_eq!(enumeration.masters[0].name, "default");
    assert_eq!(enumeration.masters[0].details[0].relationship, "type");

    let factor = model.entity("moqui.security.UserAuthcFactor").unwrap();
    let factor_id = factor.field("factorId").unwrap();
    assert!(factor_id.is_pk);
    assert!(!factor_id.not_null);
    assert!(factor.field("userId").unwrap().not_null);

    let audit = model.entity("moqui.entity.EntityAuditLog").unwrap();
    assert!(audit.create_only);
    assert!(audit.field("changedEntityName").unwrap().create_only);
    assert!(audit.field("lastUpdatedStamp").unwrap().create_only);
    let index = audit
        .indexes
        .iter()
        .find(|index| index.name == "ENTAUDLOG_PKPRIM")
        .unwrap();
    assert!(!index.unique);
    assert_eq!(index.fields, vec!["pkPrimaryValue".to_string()]);

    let detail = model.entity("moqui.basic.StatusFlowTransitionToDetail").unwrap();
    let condition = detail.entity_condition.as_ref().unwrap();
    assert_eq!(condition.order_by, vec!["sequenceNum".to_string()]);
    assert!(condition.conditions.items.is_empty());
    assert!(!condition.distinct);

    let by_alias = model.entity("artifactGroups").unwrap();
    assert_eq!(by_alias.full_name, "moqui.security.ArtifactGroup");

    let missing = model.entity("moqui.missing.NoSuch").unwrap_err();
    assert!(missing.to_string().contains("NoSuch"));
    assert!(model.entity_full("moqui.missing.NoSuch").is_none());
}

#[test]
fn artifact_authz_check_view_keeps_members() {
    let model = framework_model();
    let view = model.entity("moqui.security.ArtifactAuthzCheckView").unwrap();
    assert!(view.is_view);
    assert_eq!(view.full_name, "moqui.security.ArtifactAuthzCheckView");
    let member = view.member("AAZ").unwrap();
    assert_eq!(member.entity_name, "moqui.security.ArtifactAuthz");
    let group_member = view.member("AGM").unwrap();
    assert_eq!(group_member.relationship.as_deref(), Some("groupMembers"));
    assert_eq!(group_member.entity_name, "moqui.security.ArtifactGroupMember");
    assert_eq!(group_member.join_from_alias.as_deref(), Some("AAZ"));
    assert_eq!(
        group_member.key_map,
        vec![KeyMap {
            field_name: "artifactGroupId".to_string(),
            related: Some("artifactGroupId".to_string()),
        }]
    );
    let alias = view.alias("artifactName").unwrap();
    assert_eq!(alias.entity_alias.as_deref(), Some("AGM"));
    assert!(alias.field.is_none());
    assert_eq!(
        alias.member_entity.as_deref(),
        Some("moqui.security.ArtifactGroupMember")
    );
    assert_eq!(alias.field_type.as_deref(), Some("text-medium"));
    assert_eq!(alias.java_type, "java.lang.String");
    assert!(alias.is_pk);
    assert_eq!(
        view.pk_field_names(),
        vec![
            "artifactAuthzId",
            "artifactGroupId",
            "artifactName",
            "artifactTypeEnumId"
        ]
    );
}

#[test]
fn extend_entity_adds_a_field() {
    let root = workspace_root();
    let conf = MNode::parse_file(root.join("framework/conf/MoquiDefaultConf.xml")).unwrap();
    let mut documents = entity_documents(&root.join("framework/entity")).unwrap();
    documents.push(
        MNode::parse_text(
            "inline-extend.xml",
            r#"<entities>
                <extend-entity entity-name="Geo" package="moqui.basic">
                    <field name="extraNote" type="text-short"/>
                </extend-entity>
            </entities>"#,
        )
        .unwrap(),
    );
    let model = EntityModel::load(&conf, &documents).unwrap();
    let geo = model.entity("moqui.basic.Geo").unwrap();
    let field = geo.field("extraNote").unwrap();
    assert_eq!(field.field_type, "text-short");
    assert!(!field.is_pk);
    assert!(!field.not_null);
    assert!(geo.field("geoId").unwrap().is_pk);
}

#[test]
fn auto_reverse_swaps_the_uom_conversion_key_map() {
    let model = framework_model();
    let conversion = model.entity("moqui.basic.UomConversion").unwrap();
    let forward = conversion.relationship("To#moqui.basic.Uom").unwrap();
    assert_eq!(forward.relationship_type, "one");
    assert!(!forward.is_auto_reverse);
    assert_eq!(forward.key_map.len(), 1);
    assert_eq!(forward.key_map[0].field_name, "toUomId");
    assert_eq!(forward.key_map[0].related.as_deref(), Some("uomId"));

    let uom = model.entity("moqui.basic.Uom").unwrap();
    let reverse = uom.relationship("To#moqui.basic.UomConversion").unwrap();
    assert!(reverse.is_auto_reverse);
    assert_eq!(reverse.relationship_type, "many");
    assert_eq!(reverse.related, "moqui.basic.UomConversion");
    assert!(reverse.mutable);
    assert_eq!(reverse.key_map.len(), 1);
    assert_eq!(reverse.key_map[0].field_name, "uomId");
    assert_eq!(reverse.key_map[0].related.as_deref(), Some("toUomId"));
    assert!(uom.has_dependents);
    assert!(uom.relationship("toConversions").is_some());
}

#[test]
fn short_entity_name_must_be_unique() {
    let conf = MNode::parse_text(
        "conf.xml",
        r#"<database-list>
            <dictionary-type type="id" java-type="java.lang.String" default-sql-type="VARCHAR(40)"/>
        </database-list>"#,
    )
    .unwrap();
    let document = MNode::parse_text(
        "entities.xml",
        r#"<entities>
            <entity entity-name="Same" package="one.a" no-update-stamp="true">
                <field name="id" type="id" is-pk="true"/>
            </entity>
            <entity entity-name="Same" package="two.b" no-update-stamp="true">
                <field name="id" type="id" is-pk="true"/>
            </entity>
        </entities>"#,
    )
    .unwrap();
    let model = EntityModel::load(&conf, &[document]).unwrap();
    assert!(model.entity("one.a.Same").is_ok());
    assert!(model.entity("two.b.Same").is_ok());
    let err = model.entity("Same").unwrap_err();
    assert!(err.to_string().contains("not unique"));
    assert_eq!(model.full_names().len(), 2);
    assert!(model.entity("one.a.Same").unwrap().field("lastUpdatedStamp").is_none());
}
