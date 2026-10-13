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

//! Entity find filters from `ArtifactAuthzFilter` and `EntityFilter`.

use std::collections::BTreeMap;

use moqui_db::model::{EntityDefinition, EntityModel};
use moqui_db::sql::Condition;
use moqui_lib::object::Value;

use super::authz::{authz_enabled_for, context_entered};
use super::login::entity_facade;
use crate::context::{Error, ExecutionContext};
use crate::entity::{EntityFacade, EntityValue};

/// Add `EntityFilter` terms when a context is entered and entity authz is on.
///
/// `moqui.*` finds are unchanged. No matching filter returns `condition` as given.
/// A missing view alias with `allowMissingAlias` not `Y` fails closed (`1 = 0`).
pub fn apply_entity_filters(entity_name: &str, condition: Option<Condition>) -> Option<Condition> {
    if !context_entered() || entity_name.starts_with("moqui.") {
        return condition;
    }
    if ExecutionContext::with_current(|ctx| ctx.authz_disabled()).unwrap_or(true) {
        return condition;
    }
    if !authz_enabled_for("AT_ENTITY") {
        return condition;
    }
    match extra_conditions(entity_name) {
        Ok(extra) => combine(condition, extra),
        Err(err) => {
            tracing::error!(error = %err, "entity filter failed closed");
            combine(condition, vec![false_constraint()])
        }
    }
}

fn extra_conditions(entity_name: &str) -> Result<Vec<Condition>, Error> {
    // The filters come from the record that authorized this find, the frame on top of the stack.
    // A find that was not authorized by a record, or was inherited from no record, has none.
    let Some(authz_id) = ExecutionContext::with_current(|ec| ec.top_authz_info())
        .flatten()
        .and_then(|info| info.aacv)
        .map(|check| check.artifact_authz_id.clone())
    else {
        return Ok(Vec::new());
    };
    let ids = [authz_id];
    let facade = entity_facade()?;
    let _guard =
        ExecutionContext::with_current(|ctx| ctx.disable_authz()).ok_or_else(|| Error::new("no execution context"))?;
    let authz_filters = facade.find("moqui.security.ArtifactAuthzFilter")?.list()?;
    let sets = facade.find("moqui.security.EntityFilterSet")?.list()?;
    let filters = facade.find("moqui.security.EntityFilter")?.list()?;
    let mut extra = Vec::new();
    for authz_filter in authz_filters.values() {
        let authz_id = text(authz_filter, "artifactAuthzId");
        if !ids.iter().any(|id| id == &authz_id) {
            continue;
        }
        let set_id = text(authz_filter, "entityFilterSetId");
        let Some(set) = sets
            .values()
            .iter()
            .find(|row| text(row, "entityFilterSetId") == set_id)
        else {
            continue;
        };
        if !condition_passes(&text(authz_filter, "applyCond"))? || !condition_passes(&text(set, "applyCond"))? {
            continue;
        }
        let allow_missing = text(set, "allowMissingAlias") == "Y";
        for filter in filters.values() {
            if text(filter, "entityFilterSetId") != set_id {
                continue;
            }
            if let Some(condition) = one_filter(&facade, entity_name, filter, allow_missing)? {
                extra.push(condition);
            }
        }
    }
    Ok(extra)
}

fn condition_passes(expression: &str) -> Result<bool, Error> {
    if expression.is_empty() {
        return Ok(true);
    }
    crate::script::eval_condition(expression)
}

fn one_filter(
    facade: &EntityFacade,
    entity_name: &str,
    filter: &EntityValue,
    allow_missing: bool,
) -> Result<Option<Condition>, Error> {
    let filter_entity = text(filter, "entityName");
    let expression = text(filter, "filterMap");
    if filter_entity.is_empty() || expression.is_empty() {
        return Ok(None);
    }
    let map = match crate::script::eval_map(&expression) {
        Ok(map) => map,
        Err(err) if allow_missing => {
            tracing::error!(error = %err, "skipping entity filter map");
            return Ok(None);
        }
        Err(err) => return Err(err),
    };
    let join_or = text(filter, "joinOr") == "Y";
    let comparison = text(filter, "comparisonEnumId");
    facade
        .with_entity(entity_name, |entity, _, model| {
            conditions_for(
                entity,
                model,
                &filter_entity,
                &map,
                (join_or, &comparison),
                allow_missing,
            )
        })
        .map_err(Error::from)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Relation {
    Direct,
    Member,
    None,
}

fn conditions_for(
    entity: &EntityDefinition,
    model: &EntityModel,
    filter_entity: &str,
    map: &BTreeMap<String, Value>,
    (join_or, comparison): (bool, &str),
    allow_missing: bool,
) -> Result<Option<Condition>, moqui_lib::error::Error> {
    let relation = if entity.full_name == filter_entity {
        Relation::Direct
    } else if entity.is_view
        && entity
            .member_entities
            .iter()
            .any(|member| member.entity_name == filter_entity)
    {
        Relation::Member
    } else {
        Relation::None
    };
    if relation == Relation::None {
        return Ok(None);
    }
    let mut parts = Vec::new();
    for (field, value) in map {
        let names = match relation {
            Relation::Direct => vec![field.clone()],
            Relation::Member => member_aliases(entity, model, filter_entity, field),
            Relation::None => Vec::new(),
        };
        if names.is_empty() {
            if allow_missing {
                continue;
            }
            return Err(moqui_lib::error::Error::new(format!(
                "authorized data filter is missing an alias for {field} on {}",
                entity.full_name
            )));
        }
        for name in names {
            parts.push(value_condition(&name, value, comparison)?);
        }
    }
    Ok(join_conditions(parts, join_or))
}

fn member_aliases(entity: &EntityDefinition, model: &EntityModel, member_name: &str, field: &str) -> Vec<String> {
    let mut names = Vec::new();
    for alias in &entity.aliases {
        let Some(entity_alias) = alias.entity_alias.as_deref() else {
            continue;
        };
        let Some(member) = entity.member(entity_alias) else {
            continue;
        };
        if member.entity_name != member_name {
            continue;
        }
        let source = alias.field.as_deref().unwrap_or(alias.name.as_str());
        if source == field {
            names.push(alias.name.clone());
        }
    }
    for alias_all in &entity.alias_alls {
        let Some(member) = entity.member(&alias_all.entity_alias) else {
            continue;
        };
        if member.entity_name != member_name || alias_all.exclude.iter().any(|item| item == field) {
            continue;
        }
        let Ok(member_entity) = model.entity(&member.entity_name) else {
            continue;
        };
        if member_entity.field(field).is_none() {
            continue;
        }
        let out_name = match alias_all.prefix.as_deref() {
            Some(prefix) => prefixed_name(prefix, field),
            None => field.to_string(),
        };
        if names.iter().any(|name| name == &out_name) || entity.alias(&out_name).is_some() {
            continue;
        }
        names.push(out_name);
    }
    names
}

fn prefixed_name(prefix: &str, field: &str) -> String {
    let mut name = String::with_capacity(prefix.len() + field.len());
    name.push_str(prefix);
    let mut chars = field.chars();
    if let Some(first) = chars.next() {
        name.push(first.to_ascii_uppercase());
        name.push_str(chars.as_str());
    }
    name
}

/// One filter entry. With no comparison a list is `IN` and anything else is equality, as Java
/// does; an enum id such as `ENTCO_LESS` picks the operator.
fn value_condition(field: &str, value: &Value, comparison: &str) -> Result<Condition, moqui_lib::error::Error> {
    let list = |value: &Value| match value {
        Value::List(items) => items.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    };
    let condition = match comparison {
        "" if matches!(value, Value::List(_)) => Condition::is_in(field, list(value)),
        "" | "ENTCO_EQUALS" if value.is_null() => Condition::is_null(field),
        "" | "ENTCO_EQUALS" => Condition::equals(field, value.clone()),
        "ENTCO_NOT_EQUALS" if value.is_null() => Condition::is_not_null(field),
        "ENTCO_NOT_EQUALS" => Condition::not_equals(field, value.clone()),
        "ENTCO_LESS" => Condition::less(field, value.clone()),
        "ENTCO_GREATER" => Condition::greater(field, value.clone()),
        "ENTCO_LESS_EQ" => Condition::less_equal(field, value.clone()),
        "ENTCO_GREATER_EQ" => Condition::greater_equal(field, value.clone()),
        "ENTCO_IN" => Condition::is_in(field, list(value)),
        "ENTCO_NOT_IN" => Condition::not_in(field, list(value)),
        "ENTCO_BETWEEN" | "ENTCO_NOT_BETWEEN" => {
            let mut bounds = list(value).into_iter();
            let (Some(low), Some(high)) = (bounds.next(), bounds.next()) else {
                return Err(moqui_lib::error::Error::new(format!(
                    "filter on {field} needs a low and a high value for between"
                )));
            };
            let between = Condition::between(field, low, high);
            if comparison == "ENTCO_NOT_BETWEEN" {
                Condition::negate(between)
            } else {
                between
            }
        }
        "ENTCO_LIKE" => Condition::like(field, value.clone()),
        "ENTCO_NOT_LIKE" => Condition::not_like(field, value.clone()),
        "ENTCO_IS_NULL" => Condition::is_null(field),
        "ENTCO_IS_NOT_NULL" => Condition::is_not_null(field),
        other => {
            return Err(moqui_lib::error::Error::new(format!(
                "unknown filter comparison {other} on {field}"
            )))
        }
    };
    Ok(condition)
}

fn join_conditions(parts: Vec<Condition>, join_or: bool) -> Option<Condition> {
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ if join_or => Some(Condition::or(parts)),
        _ => Some(Condition::and(parts)),
    }
}

fn combine(incoming: Option<Condition>, extra: Vec<Condition>) -> Option<Condition> {
    if extra.is_empty() {
        return incoming;
    }
    let filters = single(extra);
    match incoming {
        None => Some(filters),
        Some(condition) => Some(Condition::and(vec![condition, filters])),
    }
}

fn single(mut parts: Vec<Condition>) -> Condition {
    if parts.len() == 1 {
        return parts.remove(0);
    }
    Condition::and(parts)
}

fn false_constraint() -> Condition {
    Condition::is_in("_entityFilter", Vec::new())
}

fn text(row: &EntityValue, field: &str) -> String {
    match row.get(field) {
        Some(Value::String(value)) => value.clone(),
        Some(other) if !other.is_null() => moqui_lib::object::to_plain_string(other),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use moqui_db::sql::Condition;
    use moqui_lib::object::Value;

    use super::super::session;
    use crate::context::ExecutionContextFactory;
    use crate::entity::EntityFacade;
    use crate::script::{clear_context_stack, set_context_name};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    const VIEW: &str = "mantle.product.ProductOrg";
    const PRODUCT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<entities>
    <entity entity-name="Product" package="mantle.product">
        <field name="productId" type="id" is-pk="true"/>
        <field name="ownerPartyId" type="id"/>
    </entity>
    <view-entity entity-name="ProductOrg" package="mantle.product">
        <member-entity entity-alias="PROD" entity-name="mantle.product.Product"/>
        <alias name="productId" entity-alias="PROD" field="productId"/>
        <alias name="ownerPartyId" entity-alias="PROD" field="ownerPartyId"/>
    </view-entity>
</entities>
"#;

    #[test]
    fn apply_entity_filters_returns_the_same_condition() {
        let condition = Condition::equals("testId", Value::string("T1"));
        let filtered = super::apply_entity_filters("moqui.test.TestEntity", Some(condition.clone()));
        assert_eq!(filtered, Some(condition));
        assert!(super::apply_entity_filters("moqui.test.TestEntity", None).is_none());
    }

    #[test]
    fn view_find_applies_user_org_and_skips_active_org_until_set() {
        session::reset_session();
        clear_context_stack();
        let runtime = temp_dir("filter-runtime");
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let factory = {
            let _lock = crate::user::session::cookie_property_lock();
            ExecutionContextFactory::boot_with_framework(runtime.path(), &framework).expect("boot")
        };
        let (facade, _model_dir) = product_facade();
        factory.set_entity(facade.clone());
        let context = factory.new_context();
        let _entered = context.enter();
        session::set_user("org-user", "org-user");
        {
            let _guard = context.disable_authz();
            seed(&facade);
        }
        set_context_name(
            "filterOrgIds",
            Value::List(vec![Value::string("ORG1"), Value::string("ORG2")]),
        )
        .unwrap();

        // The filters come from the record that allowed this frame.
        context
            .push_checked("AT_ENTITY", VIEW, "view", true, false, None)
            .expect("the ALL_USERS allow covers the view");
        let incoming = Condition::equals("productId", Value::string("P1"));
        let filtered = super::apply_entity_filters(VIEW, Some(incoming)).expect("user org filter");
        assert!(has_equals(&filtered, "productId", "P1"), "{filtered:?}");
        assert!(has_in(&filtered, "ownerPartyId", &["ORG1", "ORG2"]), "{filtered:?}");
        assert!(!has_equals(&filtered, "ownerPartyId", "ORG9"), "{filtered:?}");

        set_context_name("activeOrgId", Value::string("ORG9")).unwrap();
        let both = super::apply_entity_filters(VIEW, None).expect("both filters");
        assert!(has_in(&both, "ownerPartyId", &["ORG1", "ORG2"]), "{both:?}");
        assert!(has_equals(&both, "ownerPartyId", "ORG9"), "{both:?}");

        context.pop().unwrap();

        // The same filters on a real find through the view: ORG9 is out until the active org is set.
        {
            let _guard = context.disable_authz();
            for (id, owner) in [("P1", "ORG1"), ("P2", "ORG9"), ("P3", "ORG2")] {
                create(
                    &facade,
                    "mantle.product.Product",
                    &[("productId", id), ("ownerPartyId", owner)],
                );
            }
        }
        clear_context_stack();
        set_context_name(
            "filterOrgIds",
            Value::List(vec![Value::string("ORG1"), Value::string("ORG2")]),
        )
        .unwrap();
        let ids = |facade: &EntityFacade| -> Vec<String> {
            let mut ids: Vec<String> = facade
                .find(VIEW)
                .unwrap()
                .list()
                .unwrap()
                .values()
                .iter()
                .filter_map(|row| row.get("productId").and_then(Value::as_str).map(str::to_string))
                .collect();
            ids.sort();
            ids
        };
        // applyCond on the active org set skips it while activeOrgId is unset.
        assert_eq!(ids(&facade), ["P1", "P3"]);
        set_context_name("activeOrgId", Value::string("ORG1")).unwrap();
        assert_eq!(ids(&facade), ["P1"]);

        clear_context_stack();
        session::reset_session();
    }

    fn seed(facade: &EntityFacade) {
        create(
            facade,
            "moqui.security.ArtifactGroupMember",
            &[
                ("artifactGroupId", "MANTLE_ORG"),
                ("artifactName", VIEW),
                ("artifactTypeEnumId", "AT_ENTITY"),
                ("nameIsPattern", "N"),
            ],
        );
        create(
            facade,
            "moqui.security.ArtifactAuthz",
            &[
                ("artifactAuthzId", "MANTLE_ORG_AUTHZ"),
                ("userGroupId", "ALL_USERS"),
                ("artifactGroupId", "MANTLE_ORG"),
                ("authzTypeEnumId", "AUTHZT_ALLOW"),
                ("authzActionEnumId", "AUTHZA_ALL"),
            ],
        );
        create(
            facade,
            "moqui.security.EntityFilterSet",
            &[
                ("entityFilterSetId", "MANTLE_USER_ORG"),
                ("description", "User Organization Filters"),
            ],
        );
        create(
            facade,
            "moqui.security.EntityFilter",
            &[
                ("entityFilterId", "MANTLE_USER_ORG_A"),
                ("entityFilterSetId", "MANTLE_USER_ORG"),
                ("entityName", "mantle.product.Product"),
                ("filterMap", "({ ownerPartyId: filterOrgIds })"),
            ],
        );
        create(
            facade,
            "moqui.security.EntityFilterSet",
            &[
                ("entityFilterSetId", "MANTLE_ACTIVE_ORG"),
                ("description", "Active Organization Filters"),
                ("applyCond", "activeOrgId"),
                ("allowMissingAlias", "Y"),
            ],
        );
        create(
            facade,
            "moqui.security.EntityFilter",
            &[
                ("entityFilterId", "MANTLE_ACTIVE_ORG_A"),
                ("entityFilterSetId", "MANTLE_ACTIVE_ORG"),
                ("entityName", "mantle.product.Product"),
                ("filterMap", "({ ownerPartyId: activeOrgId })"),
            ],
        );
        create(
            facade,
            "moqui.security.ArtifactAuthzFilter",
            &[
                ("artifactAuthzId", "MANTLE_ORG_AUTHZ"),
                ("entityFilterSetId", "MANTLE_USER_ORG"),
            ],
        );
        create(
            facade,
            "moqui.security.ArtifactAuthzFilter",
            &[
                ("artifactAuthzId", "MANTLE_ORG_AUTHZ"),
                ("entityFilterSetId", "MANTLE_ACTIVE_ORG"),
            ],
        );
    }

    fn create(facade: &EntityFacade, name: &str, fields: &[(&str, &str)]) {
        let mut value = facade.make_value(name).unwrap();
        for (field, text) in fields {
            value.set(*field, Value::string(*text));
        }
        value.create(facade).unwrap();
    }

    fn has_equals(condition: &Condition, field: &str, expected: &str) -> bool {
        match condition {
            Condition::Equals(field_ref, value) => field_ref.name == field && value.as_str() == Some(expected),
            Condition::And(parts) | Condition::Or(parts) => parts.iter().any(|part| has_equals(part, field, expected)),
            Condition::Not(inner) => has_equals(inner, field, expected),
            _ => false,
        }
    }

    fn has_in(condition: &Condition, field: &str, expected: &[&str]) -> bool {
        match condition {
            Condition::In(field_ref, values) => {
                field_ref.name == field
                    && values.len() == expected.len()
                    && values
                        .iter()
                        .zip(expected)
                        .all(|(value, want)| value.as_str() == Some(*want))
            }
            Condition::And(parts) | Condition::Or(parts) => parts.iter().any(|part| has_in(part, field, expected)),
            Condition::Not(inner) => has_in(inner, field, expected),
            _ => false,
        }
    }

    fn product_facade() -> (EntityFacade, TempDir) {
        let root = temp_dir("product-fw");
        let entity_dir = root.path().join("entity");
        let conf_dir = root.path().join("conf");
        std::fs::create_dir_all(&entity_dir).unwrap();
        std::fs::create_dir_all(&conf_dir).unwrap();
        let real = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        std::os::unix::fs::symlink(
            real.join("conf/MoquiDefaultConf.xml"),
            conf_dir.join("MoquiDefaultConf.xml"),
        )
        .unwrap();
        for entry in std::fs::read_dir(real.join("entity")).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            std::os::unix::fs::symlink(entry.path(), entity_dir.join(entry.file_name())).unwrap();
        }
        std::fs::write(entity_dir.join("MantleProductAuthz.xml"), PRODUCT_XML).unwrap();
        (open_mem(root.path()), root)
    }

    fn open_mem(framework_dir: &Path) -> EntityFacade {
        crate::testing::open_at(framework_dir)
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDir {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("moqui-{label}-{}-{nanos}-{seq}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}
