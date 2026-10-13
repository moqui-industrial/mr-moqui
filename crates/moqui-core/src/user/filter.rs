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

use super::authz::{authz_enabled_for, context_entered, granting_ids};
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
    let ids = granting_ids("AT_ENTITY", entity_name, "view")?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
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
    facade
        .with_entity(entity_name, |entity, _, model| {
            conditions_for(entity, model, &filter_entity, &map, join_or, allow_missing)
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
    join_or: bool,
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
            parts.push(value_condition(&name, value));
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

fn value_condition(field: &str, value: &Value) -> Condition {
    match value {
        Value::List(items) => Condition::is_in(field, items.clone()),
        other => Condition::equals(field, other.clone()),
    }
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
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use moqui_db::sql::Condition;
    use moqui_lib::error::Error;
    use moqui_lib::object::Value;

    use super::super::session;
    use crate::context::ExecutionContextFactory;
    use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use crate::entity::{EntityFacade, EntityGroup};
    use crate::script::{clear_context_stack, set_context_name};
    use crate::transaction::{PoolConfig, TransactionManager};

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
        let factory = ExecutionContextFactory::boot_with_framework(runtime.path(), &framework).expect("boot");
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

        let incoming = Condition::equals("productId", Value::string("P1"));
        let filtered = super::apply_entity_filters(VIEW, Some(incoming)).expect("user org filter");
        assert!(has_equals(&filtered, "productId", "P1"), "{filtered:?}");
        assert!(has_in(&filtered, "ownerPartyId", &["ORG1", "ORG2"]), "{filtered:?}");
        assert!(!has_equals(&filtered, "ownerPartyId", "ORG9"), "{filtered:?}");

        set_context_name("activeOrgId", Value::string("ORG9")).unwrap();
        let both = super::apply_entity_filters(VIEW, None).expect("both filters");
        assert!(has_in(&both, "ownerPartyId", &["ORG1", "ORG2"]), "{both:?}");
        assert!(has_equals(&both, "ownerPartyId", "ORG9"), "{both:?}");

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
        let driver: Arc<dyn DbDriver> = Arc::new(MemDriver {
            store: Arc::new(Mutex::new(BTreeMap::new())),
        });
        let transactions = TransactionManager::new(
            driver,
            PoolConfig {
                min: 0,
                max: 4,
                reserve: 2,
                acquire_timeout: Duration::from_secs(5),
            },
        )
        .expect("memory pool");
        let facade = EntityFacade::open(
            framework_dir,
            "transactional",
            vec![EntityGroup::new("transactional", "postgres", transactions)],
        )
        .expect("entity facade");
        facade.set_disable_audit_log(true);
        facade.set_disable_entity_eca(true);
        facade.set_disable_fk_create(true);
        facade.set_disable_data_feed(true);
        facade
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

    type Row = BTreeMap<String, Value>;
    type Tables = BTreeMap<String, Vec<Row>>;

    struct MemDriver {
        store: Arc<Mutex<Tables>>,
    }

    struct MemConn {
        store: Arc<Mutex<Tables>>,
    }

    struct NoCancel;

    impl DbCancel for NoCancel {
        fn cancel(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl DbDriver for MemDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
            Ok(Box::new(MemConn {
                store: Arc::clone(&self.store),
            }))
        }
    }

    impl DbConnection for MemConn {
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
            Ok(self.apply(sql, params)?.0)
        }

        fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            Ok(self.apply(sql, params)?.1)
        }

        fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            self.query(sql, params)
        }

        fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
            Ok(None)
        }

        fn close_stream(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn stream_open(&self) -> bool {
            false
        }

        fn begin(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn commit(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn savepoint(&mut self, _name: &str) -> Result<(), Error> {
            Ok(())
        }

        fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), Error> {
            Ok(())
        }

        fn release_savepoint(&mut self, _name: &str) -> Result<(), Error> {
            Ok(())
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(NoCancel)
        }
    }

    impl MemConn {
        fn apply(&mut self, sql: &str, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
            let sql = sql.trim();
            if sql.eq_ignore_ascii_case("SELECT 1") {
                return Ok((
                    1,
                    vec![DbRow {
                        columns: vec![("1".to_string(), Value::I64(1))],
                    }],
                ));
            }
            let mut cur = Cur { sql, i: 0 };
            if cur.eat_keyword("INSERT") {
                return self.insert(&mut cur, params);
            }
            if cur.eat_keyword("SELECT") {
                return self.select(&mut cur);
            }
            Err(Error::new(format!("unsupported sql {sql}")))
        }

        fn insert(&mut self, cur: &mut Cur<'_>, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
            cur.expect_keyword("INTO")?;
            let table = cur.quoted()?;
            cur.expect_symbol(b'(')?;
            let mut columns = Vec::new();
            loop {
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            cur.expect_symbol(b')')?;
            cur.expect_keyword("VALUES")?;
            cur.expect_symbol(b'(')?;
            for index in 0..columns.len() {
                if index > 0 {
                    cur.expect_symbol(b',')?;
                }
                cur.expect_symbol(b'?')?;
            }
            cur.expect_symbol(b')')?;
            cur.expect_end()?;
            let mut row = BTreeMap::new();
            for (index, column) in columns.iter().enumerate() {
                let value = params
                    .get(index)
                    .cloned()
                    .ok_or_else(|| Error::new("insert is missing a parameter"))?;
                row.insert(column.clone(), value);
            }
            let mut tables = self.store.lock().unwrap_or_else(|err| err.into_inner());
            tables.entry(table).or_default().push(row);
            Ok((1, Vec::new()))
        }

        fn select(&mut self, cur: &mut Cur<'_>) -> Result<(u64, Vec<DbRow>), Error> {
            let mut columns = Vec::new();
            loop {
                if cur.eat_keyword("FROM") {
                    break;
                }
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            if columns.is_empty() {
                return Err(Error::new(format!("select has no columns in {}", cur.sql)));
            }
            if !cur.keyword_was_from() {
                cur.expect_keyword("FROM")?;
            }
            let table = cur.quoted()?;
            cur.expect_end()?;
            let tables = self.store.lock().unwrap_or_else(|err| err.into_inner());
            let rows = tables.get(&table).cloned().unwrap_or_default();
            let projected = rows
                .iter()
                .map(|row| DbRow {
                    columns: columns
                        .iter()
                        .map(|column| (column.clone(), row.get(column).cloned().unwrap_or(Value::Null)))
                        .collect(),
                })
                .collect::<Vec<_>>();
            let count = u64::try_from(projected.len()).unwrap_or(u64::MAX);
            Ok((count, projected))
        }
    }

    struct Cur<'a> {
        sql: &'a str,
        i: usize,
    }

    impl Cur<'_> {
        fn skip(&mut self) {
            while self.i < self.sql.len() && self.sql.as_bytes()[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }

        fn eat_keyword(&mut self, word: &str) -> bool {
            self.skip();
            let rest = &self.sql[self.i..];
            if rest.len() < word.len() || !rest[..word.len()].eq_ignore_ascii_case(word) {
                return false;
            }
            let boundary = rest.as_bytes().get(word.len()).copied();
            if boundary.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                return false;
            }
            self.i += word.len();
            true
        }

        fn keyword_was_from(&self) -> bool {
            let head = self.sql[..self.i].trim_end();
            head.len() >= 4 && head[head.len() - 4..].eq_ignore_ascii_case("FROM")
        }

        fn expect_keyword(&mut self, word: &str) -> Result<(), Error> {
            if self.eat_keyword(word) {
                Ok(())
            } else {
                Err(Error::new(format!("expected {word} in {}", self.sql)))
            }
        }

        fn quoted(&mut self) -> Result<String, Error> {
            self.skip();
            let bytes = self.sql.as_bytes();
            if self.i >= bytes.len() || bytes[self.i] != b'"' {
                return Err(Error::new(format!("expected ident in {}", self.sql)));
            }
            self.i += 1;
            let start = self.i;
            while self.i < bytes.len() && bytes[self.i] != b'"' {
                self.i += 1;
            }
            if self.i >= bytes.len() {
                return Err(Error::new("unterminated ident"));
            }
            let name = self.sql[start..self.i].to_string();
            self.i += 1;
            Ok(name)
        }

        fn eat_symbol(&mut self, symbol: u8) -> bool {
            self.skip();
            if self.sql.as_bytes().get(self.i) == Some(&symbol) {
                self.i += 1;
                true
            } else {
                false
            }
        }

        fn expect_symbol(&mut self, symbol: u8) -> Result<(), Error> {
            if self.eat_symbol(symbol) {
                Ok(())
            } else {
                Err(Error::new(format!("expected {} in {}", char::from(symbol), self.sql)))
            }
        }

        fn expect_end(&mut self) -> Result<(), Error> {
            self.skip();
            if self.i >= self.sql.len() {
                Ok(())
            } else {
                Err(Error::new(format!("unexpected sql tail in {}", self.sql)))
            }
        }
    }
}
