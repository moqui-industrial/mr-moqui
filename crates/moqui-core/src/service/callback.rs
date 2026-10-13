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

//! Entity ECA actions, data-file service elements, and feed receive.
//!
//! Security scripts from `Security.eecas.xml` run as Rust checks. Other action
//! bodies run through `script::run_actions`. Feed receive records that search is
//! not up and leaves `DataFeedPending` rows in place.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use moqui_lib::error::Error;
use moqui_lib::node::MNode;
use moqui_lib::object::{basic_convert, Value};

use crate::context::ExecutionContext;
use crate::entity::load_feed::{self, HeldService};
use crate::entity::{EntityFacade, EntityValue};

const DEFAULT_PRIVILEGED_GROUPS: &str = "ADMIN,ADMIN_ADV";
const DEFAULT_SEALED_PERMISSIONS: &str =
    "GROOVY_SHELL_WEB,SQL_RUNNER_WEB,SERVICE_LOAD_RUNNER,ADMIN_LOGIN_AS,ADMIN_PASSWORD,ElasticRemote,KibanaRemote,REST_SCHEMA";

/// How many times `UserSecurityChecks` ran on this thread, for a load report to show the ECAs fired.
pub fn security_checks_run() -> u64 {
    SECURITY_CHECKS.with(Cell::get)
}

thread_local! {
    static SECURITY_CHECKS: Cell<u64> = const { Cell::new(0) };
    static SEALED_ENTERED: Cell<bool> = const { Cell::new(false) };
    static SEARCH_NOT_UP: Cell<bool> = const { Cell::new(false) };
}

/// True after [`schedule_feed_receive`] remembered at least one receive service.
pub fn search_not_up() -> bool {
    SEARCH_NOT_UP.with(|cell| cell.get())
}

#[cfg(test)]
fn reset_search_not_up() {
    SEARCH_NOT_UP.with(|cell| cell.set(false));
}

#[cfg(test)]
fn sealed_grant_entered() -> bool {
    SEALED_ENTERED.with(|cell| cell.get())
}

#[cfg(test)]
fn reset_sealed_grant() {
    SEALED_ENTERED.with(|cell| cell.set(false));
}

/// Run matched ECA rules. `before` selects run-before rules. Ids come from the last match.
pub fn run_matched_eca(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    before: bool,
    values: &BTreeMap<String, Value>,
) -> Result<(), Error> {
    tracing::debug!(entity = entity_name, operation, before, "running matched entity eca");
    let matched: BTreeSet<String> = load_feed::matched_eca_ids(facade).into_iter().collect();
    if matched.is_empty() {
        return Ok(());
    }
    for rule in load_feed::eca_rules(facade, entity_name) {
        if !matched.contains(&rule.id) || rule.run_before != before {
            continue;
        }
        let _frame = publish_names(values).map_err(to_lib)?;
        if !condition_allows(&rule.condition).map_err(to_lib)? {
            continue;
        }
        run_rule_actions(facade, &rule.actions, values)?;
    }
    Ok(())
}

/// Run one data-file service element through [`crate::service::call::sync`].
///
/// `sync` reads the facade on the current execution context. A load that has not
/// entered a context keeps the held service and does not call it.
pub fn run_held_service(facade: &EntityFacade, service: &HeldService) -> Result<(), Error> {
    tracing::debug!(
        service = %service.name,
        location = %service.location,
        feed_off = facade.flags().disable_data_feed,
        "running held data-file service"
    );
    let mut params = BTreeMap::new();
    for (name, value) in &service.parameters {
        params.insert(name.clone(), Value::string(value));
    }
    if !crate::user::context_entered() {
        return Ok(());
    }
    let _guard = ExecutionContext::with_current(|ctx| ctx.disable_authz());
    super::call::sync(&service.name, &params).map(|_| ()).map_err(to_lib)
}

/// Record that search is not up for each remembered receive service. Runs inline.
pub fn schedule_feed_receive(facade: &EntityFacade) -> Result<(), Error> {
    let services = load_feed::held_feed_services(facade);
    if services.is_empty() {
        return Ok(());
    }
    for service in &services {
        let registered = super::define::find(&service.receive_service).is_some();
        SEARCH_NOT_UP.with(|cell| cell.set(true));
        tracing::debug!(
            service = %service.receive_service,
            registered,
            search_down = search_not_up(),
            "feed receive left search down"
        );
    }
    Ok(())
}

fn run_rule_actions(facade: &EntityFacade, actions: &str, values: &BTreeMap<String, Value>) -> Result<(), Error> {
    if actions.contains("UserSecurityChecks.assertPrivilegedGroupMembership") {
        return assert_privileged_group_membership(facade, &map_text(values, "userGroupId"));
    }
    if actions.contains("UserSecurityChecks.assertSealedPermissionGrant") {
        return assert_sealed_permission_grant(facade, &map_text(values, "userPermissionId"));
    }
    run_stored_actions(actions).map_err(to_lib)
}

fn assert_privileged_group_membership(facade: &EntityFacade, user_group_id: &str) -> Result<(), Error> {
    SECURITY_CHECKS.with(|count| count.set(count.get() + 1));
    if user_group_id.is_empty() {
        return Ok(());
    }
    if authz_disabled() {
        return Ok(());
    }
    if !privileged_groups().contains(user_group_id) {
        return Ok(());
    }
    if without_authz(|| user_in_group(facade, user_group_id))? {
        return Ok(());
    }
    Err(Error::new(format!(
        "User is not authorized to change membership of group {user_group_id}"
    )))
}

fn assert_sealed_permission_grant(facade: &EntityFacade, user_permission_id: &str) -> Result<(), Error> {
    SECURITY_CHECKS.with(|count| count.set(count.get() + 1));
    SEALED_ENTERED.with(|entered| entered.set(true));
    if user_permission_id.is_empty() {
        return Ok(());
    }
    if authz_disabled() {
        return Ok(());
    }
    if !sealed_permissions().contains(user_permission_id) {
        return Ok(());
    }
    if without_authz(|| user_has_permission(facade, user_permission_id))? {
        return Ok(());
    }
    Err(Error::new(format!(
        "User is not authorized to grant permission {user_permission_id}"
    )))
}

fn privileged_groups() -> BTreeSet<String> {
    csv_ids("user_privileged_groups", DEFAULT_PRIVILEGED_GROUPS)
}

fn sealed_permissions() -> BTreeSet<String> {
    csv_ids("user_sealed_permissions", DEFAULT_SEALED_PERMISSIONS)
}

fn csv_ids(property: &str, default_ids: &str) -> BTreeSet<String> {
    let loaded = moqui_lib::env::get_prop_or_env(property);
    let source = if loaded.is_empty() {
        default_ids
    } else {
        loaded.as_str()
    };
    source
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn authz_disabled() -> bool {
    ExecutionContext::with_current(|ctx| ctx.authz_disabled()).unwrap_or(false)
}

fn without_authz<T>(body: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    let guard = ExecutionContext::with_current(|ctx| ctx.disable_authz());
    let result = body();
    drop(guard);
    result
}

fn user_in_group(facade: &EntityFacade, group_id: &str) -> Result<bool, Error> {
    let Some(user_id) = current_account_id(facade)? else {
        return Ok(false);
    };
    let now = now_stamp();
    let rows = facade
        .find("moqui.security.UserGroupMember")?
        .eq("userId", Value::string(&user_id))
        .eq("userGroupId", Value::string(group_id))
        .list()?;
    Ok(rows
        .values()
        .iter()
        .any(|row| membership_open(row.get("fromDate"), row.get("thruDate"), &now)))
}

fn user_has_permission(facade: &EntityFacade, permission_id: &str) -> Result<bool, Error> {
    let Some(user_id) = current_account_id(facade)? else {
        return Ok(false);
    };
    let now = now_stamp();
    let members = facade
        .find("moqui.security.UserGroupMember")?
        .eq("userId", Value::string(&user_id))
        .list()?;
    for member in members.values() {
        if !membership_open(member.get("fromDate"), member.get("thruDate"), &now) {
            continue;
        }
        let group_id = row_text(member, "userGroupId");
        if group_id.is_empty() {
            continue;
        }
        let grants = facade
            .find("moqui.security.UserGroupPermission")?
            .eq("userGroupId", Value::string(&group_id))
            .eq("userPermissionId", Value::string(permission_id))
            .list()?;
        if grants
            .values()
            .iter()
            .any(|row| membership_open(row.get("fromDate"), row.get("thruDate"), &now))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn current_account_id(facade: &EntityFacade) -> Result<Option<String>, Error> {
    let Some(username) = crate::user::current_username().map_err(to_lib)? else {
        return Ok(None);
    };
    if username.is_empty() {
        return Ok(None);
    }
    let Some(account) = facade
        .find("moqui.security.UserAccount")?
        .eq("username", Value::string(username))
        .one()?
    else {
        return Ok(None);
    };
    let user_id = row_text(&account, "userId");
    if user_id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(user_id))
    }
}

fn membership_open(from: Option<&Value>, thru: Option<&Value>, now: &Value) -> bool {
    let Some(from) = from else {
        return false;
    };
    if from.is_null() {
        return false;
    }
    let Ok(order) = moqui_lib::object::compare_obj(from, now) else {
        return false;
    };
    if order == std::cmp::Ordering::Greater {
        return false;
    }
    match thru {
        None | Some(Value::Null) => true,
        Some(thru) => moqui_lib::object::compare_obj(thru, now).ok() == Some(std::cmp::Ordering::Greater),
    }
}

fn now_stamp() -> Value {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}

fn map_text(values: &BTreeMap<String, Value>, field: &str) -> String {
    match values.get(field) {
        Some(value) if !value.is_null() => moqui_lib::object::to_plain_string(value),
        _ => String::new(),
    }
}

fn row_text(row: &EntityValue, field: &str) -> String {
    match row.get(field) {
        Some(value) if !value.is_null() => moqui_lib::object::to_plain_string(value),
        _ => String::new(),
    }
}

fn to_lib(err: crate::context::Error) -> Error {
    Error::new(err.message())
}

/// Push a script frame holding `values`. The frame pops when the guard drops.
pub(crate) fn publish_names(
    values: &BTreeMap<String, Value>,
) -> Result<crate::script::ContextFrame, crate::context::Error> {
    let frame = crate::script::ContextFrame::push();
    for (name, value) in values {
        crate::script::set_context_name(name, value.clone())?;
    }
    Ok(frame)
}

/// An empty condition passes. XML is parsed so only the expression text is evaluated.
pub(crate) fn condition_allows(condition: &str) -> Result<bool, crate::context::Error> {
    let trimmed = condition.trim();
    if trimmed.is_empty() {
        return Ok(true);
    }
    let expression = if trimmed.starts_with('<') {
        expression_text(trimmed)?
    } else {
        trimmed.to_string()
    };
    let expression = expression.trim();
    if expression.is_empty() {
        return Ok(true);
    }
    crate::script::eval_condition(expression)
}

fn expression_text(xml: &str) -> Result<String, crate::context::Error> {
    let root = MNode::parse_text("condition", xml)?;
    let mut parts = Vec::new();
    for child in root.children() {
        if child.name() != "expression" && child.name() != "script" {
            continue;
        }
        let Some(text) = child.text() else {
            continue;
        };
        let text = text.trim();
        if !text.is_empty() {
            parts.push(text.to_string());
        }
    }
    if parts.len() <= 1 {
        return Ok(parts.pop().unwrap_or_default());
    }
    Ok(parts
        .into_iter()
        .map(|part| format!("({part})"))
        .collect::<Vec<_>>()
        .join(" && "))
}

/// Run action XML on the shared action runner. A bare element is wrapped in `<actions>`.
pub(crate) fn run_stored_actions(actions: &str) -> Result<(), crate::context::Error> {
    let trimmed = actions.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.starts_with("<actions") {
        return crate::script::run_actions(trimmed);
    }
    crate::script::run_actions(&format!("<actions>{trimmed}</actions>"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use moqui_lib::object::{basic_convert, Value};

    use super::{reset_sealed_grant, reset_search_not_up, sealed_grant_entered, search_not_up};
    use crate::context::ExecutionContextFactory;
    use crate::entity::load_feed::{load_entity_data, LoadOptions};
    use crate::entity::{EntityFacade, EntityValue};
    use crate::user::{hash_password, login_user};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn sealed_permission_create_is_stored_when_authz_is_disabled() {
        let (facade, context, _runtime) = boot_security();
        let _entered = context.enter();
        let _authz = context.disable_authz();
        seed_entity_allow(&facade);
        reset_sealed_grant();
        permission_row(&facade, "ADMIN", "ADMIN_PASSWORD")
            .create(&facade)
            .expect("create with authz disabled");
        assert!(sealed_grant_entered());
        assert_eq!(permission_count(&facade, "ADMIN", "ADMIN_PASSWORD"), 1);
    }

    #[test]
    fn sealed_permission_create_is_denied_without_the_permission() {
        let (facade, context, _runtime) = boot_security();
        let _entered = context.enter();
        {
            let _authz = context.disable_authz();
            seed_entity_allow(&facade);
            seed_user(&facade, "plain", "Secret-12");
        }
        login_user("plain", "Secret-12").expect("login");
        let seen = crate::user::current_username().expect("username");
        assert_eq!(
            seen.as_deref(),
            Some("plain"),
            "current_username cannot see the logged-in user"
        );
        reset_sealed_grant();
        let err = permission_row(&facade, "ADMIN", "ADMIN_PASSWORD")
            .create(&facade)
            .expect_err("sealed permission");
        let text = err.to_string();
        assert!(text.contains("authorized"), "{text}");
        assert!(text.contains("ADMIN_PASSWORD"), "{text}");
        assert!(sealed_grant_entered());
        assert_eq!(permission_count(&facade, "ADMIN", "ADMIN_PASSWORD"), 0);
    }

    #[test]
    fn data_file_runs_entity_auto_and_inline_service() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        let service = write_temp(
            "inline",
            r#"<services>
                <service verb="copy" noun="Row">
                    <in-parameters><parameter name="seedId"/></in-parameters>
                    <actions>
                        <entity-find entity-name="moqui.test.TestEntity" list="found">
                            <econdition field-name="testId" from="seedId"/>
                        </entity-find>
                        <entity-make-value entity-name="moqui.test.TestEntity" value-field="made"/>
                        <script>made.set("testId", "B"); made.set("testMedium", "made");</script>
                        <entity-create value-field="made"/>
                    </actions>
                </service>
            </services>"#,
        );
        let failures = crate::service::define::lint(std::slice::from_ref(&service)).unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        let facade = mem::open(false, false);
        let context = ExecutionContextFactory::test_context();
        context.factory().set_entity(facade.clone());
        let _entered = context.enter();
        let data = write_temp(
            "data",
            r#"<entity-facade-xml>
                <moqui.test.TestEntity testId="E1" testMedium="entity"/>
                <create-moqui.test.TestEntity testId="A1" testMedium="auto"/>
                <copy-Row seedId="E1"/>
            </entity-facade-xml>"#,
        );
        load_entity_data(&facade, &data.display().to_string(), &LoadOptions::default()).unwrap();
        assert_eq!(field(&facade, "A1", "testMedium").as_deref(), Some("auto"));
        assert_eq!(field(&facade, "B", "testMedium").as_deref(), Some("made"));
    }

    #[test]
    fn feed_commit_keeps_pending_and_records_search_not_up() {
        reset_search_not_up();
        let facade = mem::open(false, true);
        let context = ExecutionContextFactory::test_context();
        context.factory().set_entity(facade.clone());
        let _entered = context.enter();
        create_row(
            &facade,
            "moqui.basic.EnumerationType",
            &[("enumTypeId", "DataFeedType")],
        );
        create_row(
            &facade,
            "moqui.basic.Enumeration",
            &[("enumId", "DTFDTP_RT_PUSH"), ("enumTypeId", "DataFeedType")],
        );
        create_row(
            &facade,
            "moqui.entity.document.DataDocument",
            &[("dataDocumentId", "DOC1"), ("primaryEntityName", "moqui.test.Foo")],
        );
        create_row(
            &facade,
            "moqui.entity.feed.DataFeed",
            &[
                ("dataFeedId", "FEED1"),
                ("dataFeedTypeEnumId", "DTFDTP_RT_PUSH"),
                (
                    "feedReceiveServiceName",
                    "org.moqui.search.SearchServices.index#DataDocuments",
                ),
                (
                    "feedDeleteServiceName",
                    "org.moqui.search.SearchServices.delete#DataDocument",
                ),
            ],
        );
        create_row(
            &facade,
            "moqui.entity.feed.DataFeedDocument",
            &[("dataFeedId", "FEED1"), ("dataDocumentId", "DOC1")],
        );
        facade.begin("moqui.test.Foo", None).unwrap();
        create_row(&facade, "moqui.test.Foo", &[("fooId", "F1"), ("fooText", "alpha")]);
        facade.commit("moqui.test.Foo").unwrap();
        let pending = facade
            .find("moqui.entity.feed.DataFeedPending")
            .unwrap()
            .list()
            .unwrap();
        assert!(
            pending.values().iter().any(|row| {
                text(row, "dataDocumentId") == "DOC1"
                    && text(row, "entityName") == "moqui.test.Foo"
                    && text(row, "primaryKey") == "F1"
                    && text(row, "operation") == "index"
            }),
            "{pending:?}"
        );
        assert!(search_not_up());
    }

    fn boot_security() -> (EntityFacade, crate::context::ExecutionContext, TempDir) {
        let runtime = temp_dir("seca-runtime");
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let factory = ExecutionContextFactory::boot_with_framework(runtime.path(), &framework).expect("boot");
        let facade = mem::open(true, false);
        factory.set_entity(facade.clone());
        let context = factory.new_context();
        (facade, context, runtime)
    }

    fn seed_entity_allow(facade: &EntityFacade) {
        create_row(
            facade,
            "moqui.security.ArtifactGroupMember",
            &[
                ("artifactGroupId", "SEALED_ENTITY"),
                ("artifactName", ".*"),
                ("artifactTypeEnumId", "AT_ENTITY"),
                ("nameIsPattern", "Y"),
            ],
        );
        create_row(
            facade,
            "moqui.security.ArtifactAuthz",
            &[
                ("artifactAuthzId", "SEALED_ALLOW"),
                ("userGroupId", "ALL_USERS"),
                ("artifactGroupId", "SEALED_ENTITY"),
                ("authzTypeEnumId", "AUTHZT_ALLOW"),
                ("authzActionEnumId", "AUTHZA_ALL"),
            ],
        );
    }

    fn seed_user(facade: &EntityFacade, username: &str, password: &str) {
        let hashed = hash_password(password).expect("hash");
        let mut row = facade.make_value("moqui.security.UserAccount").unwrap();
        row.set("userId", Value::string("U1"));
        row.set("username", Value::string(username));
        row.set("currentPassword", Value::string(hashed));
        row.set("passwordHashType", Value::string("argon2"));
        row.set("passwordSalt", Value::string(""));
        row.set("passwordBase64", Value::string("N"));
        row.set("disabled", Value::string("N"));
        row.set("requirePasswordChange", Value::string("N"));
        row.create(facade).unwrap();
    }

    fn permission_row(facade: &EntityFacade, group_id: &str, permission_id: &str) -> EntityValue {
        let mut row = facade.make_value("moqui.security.UserGroupPermission").unwrap();
        row.set("userGroupId", Value::string(group_id));
        row.set("userPermissionId", Value::string(permission_id));
        row.set("fromDate", basic_convert(&Value::I64(0), "java.sql.Timestamp").value);
        row
    }

    fn permission_count(facade: &EntityFacade, group_id: &str, permission_id: &str) -> usize {
        facade
            .find("moqui.security.UserGroupPermission")
            .unwrap()
            .eq("userGroupId", Value::string(group_id))
            .eq("userPermissionId", Value::string(permission_id))
            .list()
            .unwrap()
            .len()
    }

    fn field(facade: &EntityFacade, test_id: &str, name: &str) -> Option<String> {
        facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string(test_id))
            .one()
            .unwrap()
            .and_then(|row| row.get(name).and_then(Value::as_str).map(str::to_string))
    }

    fn create_row(facade: &EntityFacade, entity: &str, pairs: &[(&str, &str)]) {
        let mut row = facade.make_value(entity).unwrap();
        for (name, value) in pairs {
            row.set(*name, Value::string(*value));
        }
        row.create(facade).unwrap();
    }

    fn text(row: &EntityValue, field: &str) -> String {
        row.get(field).and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn write_temp(label: &str, body: &str) -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("moqui-cb-{label}-{seq}-{}.xml", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &std::path::Path {
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
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("moqui-{label}-{nanos}-{seq}"));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
    mod mem {
        use std::collections::{BTreeMap, BTreeSet};
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        use moqui_lib::error::Error;
        use moqui_lib::object::{compare_obj, Value};

        use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
        use crate::entity::{EntityFacade, EntityGroup};
        use crate::transaction::{PoolConfig, TransactionManager};

        type Row = BTreeMap<String, Value>;
        type Tables = BTreeMap<String, Vec<Row>>;

        pub(super) fn open(eca_on: bool, feed_on: bool) -> EntityFacade {
            let tape = Arc::new(Mutex::new(Vec::new()));
            let driver: Arc<dyn DbDriver> = Arc::new(MemDriver {
                store: Arc::new(Mutex::new(BTreeMap::new())),
                tape,
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
                &framework_dir(),
                "transactional",
                vec![EntityGroup::new("transactional", "postgres", transactions)],
            )
            .expect("entity facade");
            facade.set_disable_audit_log(true);
            facade.set_disable_entity_eca(!eca_on);
            facade.set_disable_fk_create(true);
            facade.set_disable_data_feed(!feed_on);
            facade
        }

        fn framework_dir() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework")
        }

        struct MemDriver {
            store: Arc<Mutex<Tables>>,
            tape: Arc<Mutex<Vec<String>>>,
        }

        struct MemConn {
            store: Arc<Mutex<Tables>>,
            tape: Arc<Mutex<Vec<String>>>,
            local: Option<Tables>,
            dirty: BTreeSet<String>,
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
                    tape: Arc::clone(&self.tape),
                    local: None,
                    dirty: BTreeSet::new(),
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
                if self.local.is_none() {
                    let committed = self.store.lock().unwrap_or_else(|err| err.into_inner()).clone();
                    self.local = Some(committed);
                    self.dirty.clear();
                }
                Ok(())
            }

            fn commit(&mut self) -> Result<(), Error> {
                let Some(local) = self.local.take() else {
                    return Ok(());
                };
                let dirty = std::mem::take(&mut self.dirty);
                let mut committed = self.store.lock().unwrap_or_else(|err| err.into_inner());
                for table in dirty {
                    let rows = local.get(&table).cloned().unwrap_or_default();
                    committed.insert(table, rows);
                }
                Ok(())
            }

            fn rollback(&mut self) -> Result<(), Error> {
                self.local = None;
                self.dirty.clear();
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
                // The pool checks out a connection with `SELECT 1`.
                if !sql.eq_ignore_ascii_case("SELECT 1") {
                    self.tape
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .push(sql.to_string());
                }
                if sql.eq_ignore_ascii_case("SELECT 1") {
                    return Ok((
                        1,
                        vec![DbRow {
                            columns: vec![("1".to_string(), Value::I64(1))],
                        }],
                    ));
                }
                if sql.contains("ON CONFLICT") && sql.contains("SequenceValueItem") {
                    return self.sequence(params);
                }
                if sql.contains("xmin") && sql.contains("txid_current") {
                    return self.xmin_rows(sql);
                }
                let stmt = parse(sql).map_err(|err| Error::new(format!("{err} sql={sql}")))?;
                self.exec(stmt, params)
            }

            fn sequence(&mut self, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                if params.len() < 4 {
                    return Err(Error::new("sequence refill is missing parameters"));
                }
                let name = params[0].clone();
                let first_end = as_i64(&params[1])?;
                let increment = as_i64(&params[2])?;
                let hint_end = as_i64(&params[3])?;
                let mut rows = self.read_table("SequenceValueItem");
                let end = if let Some(row) = rows.iter_mut().find(|row| row.get("seqName") == Some(&name)) {
                    let old = row.get("seqNum").and_then(|value| as_i64(value).ok()).unwrap_or(0);
                    let new_end = old.saturating_add(increment).max(hint_end);
                    row.insert("seqNum".to_string(), Value::I64(new_end));
                    new_end
                } else {
                    let mut row = BTreeMap::new();
                    row.insert("seqName".to_string(), name);
                    row.insert("seqNum".to_string(), Value::I64(first_end));
                    rows.push(row);
                    first_end
                };
                self.put_table("SequenceValueItem", rows);
                Ok((
                    1,
                    vec![DbRow {
                        columns: vec![("seqNum".to_string(), Value::I64(end))],
                    }],
                ))
            }

            fn xmin_rows(&self, sql: &str) -> Result<(u64, Vec<DbRow>), Error> {
                let Some(from_at) = sql.find(" FROM ") else {
                    return Err(Error::new("xmin query has no table"));
                };
                let mut columns = Vec::new();
                let mut rest = &sql[..from_at];
                while let Some(start) = rest.find('"') {
                    let after = &rest[start + 1..];
                    let Some(end) = after.find('"') else {
                        break;
                    };
                    columns.push(after[..end].to_string());
                    rest = &after[end + 1..];
                }
                let tail = sql[from_at + 6..].trim_start();
                let table = tail
                    .strip_prefix('"')
                    .and_then(|text| text.split('"').next())
                    .unwrap_or("")
                    .to_string();
                if table.is_empty() || columns.is_empty() {
                    return Err(Error::new("xmin query is missing columns"));
                }
                let rows = self.read_table(&table);
                let projected: Vec<DbRow> = rows.iter().map(|row| project(row, &columns)).collect();
                let count = u64::try_from(projected.len()).unwrap_or(u64::MAX);
                Ok((count, projected))
            }
            fn exec(&mut self, stmt: Stmt, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                match stmt {
                    Stmt::Insert { table, columns } => {
                        let mut row = BTreeMap::new();
                        for (index, column) in columns.iter().enumerate() {
                            let value = params
                                .get(index)
                                .cloned()
                                .ok_or_else(|| Error::new("insert is missing a parameter"))?;
                            row.insert(column.clone(), value);
                        }
                        let mut rows = self.read_table(&table);
                        rows.push(row);
                        self.put_table(&table, rows);
                        Ok((1, Vec::new()))
                    }
                    Stmt::Update { table, columns, pred } => {
                        let values = columns
                            .iter()
                            .enumerate()
                            .map(|(index, _)| {
                                params
                                    .get(index)
                                    .cloned()
                                    .ok_or_else(|| Error::new("update is missing a parameter"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let start = columns.len();
                        let mut rows = self.read_table(&table);
                        let mut count = 0_u64;
                        for row in &mut rows {
                            if keep(&pred, row, params, start)? {
                                for (column, value) in columns.iter().zip(&values) {
                                    row.insert(column.clone(), value.clone());
                                }
                                count += 1;
                            }
                        }
                        self.put_table(&table, rows);
                        Ok((count, Vec::new()))
                    }
                    Stmt::Delete { table, pred } => {
                        let rows = self.read_table(&table);
                        let mut kept = Vec::new();
                        let mut count = 0_u64;
                        for row in rows {
                            if keep(&pred, &row, params, 0)? {
                                count += 1;
                            } else {
                                kept.push(row);
                            }
                        }
                        self.put_table(&table, kept);
                        Ok((count, Vec::new()))
                    }
                    Stmt::Select(query) => self.exec_select(&query, params),
                }
            }

            fn exec_select(&mut self, query: &SelectQuery, params: &[Value]) -> Result<(u64, Vec<DbRow>), Error> {
                let mut matched = Vec::new();
                for row in self.read_table(&query.table) {
                    if keep(&query.pred, &row, params, 0)? {
                        matched.push(row);
                    }
                }
                if query.count {
                    let n = i64::try_from(matched.len()).unwrap_or(i64::MAX);
                    return Ok((
                        1,
                        vec![DbRow {
                            columns: vec![("count".to_string(), Value::I64(n))],
                        }],
                    ));
                }
                matched.sort_by(|left, right| sort_rows(left, right, &query.order));
                // Postgres writes `LIMIT ? OFFSET ?`, so the limit parameter is first.
                let mut idx = pred_slots(&query.pred);
                let take_n = if query.limit {
                    Some(as_i64(take_param(params, &mut idx)?)?)
                } else {
                    None
                };
                let skip = if query.offset {
                    as_i64(take_param(params, &mut idx)?)?
                } else {
                    0
                };
                let start = usize::try_from(skip.max(0)).unwrap_or(0);
                let window: Vec<Row> = match take_n {
                    Some(limit) => {
                        let len = usize::try_from(limit.max(0)).unwrap_or(0);
                        matched.into_iter().skip(start).take(len).collect()
                    }
                    None => matched.into_iter().skip(start).collect(),
                };
                let projected: Vec<DbRow> = window.iter().map(|row| project(row, &query.columns)).collect();
                let n = u64::try_from(projected.len()).unwrap_or(u64::MAX);
                Ok((n, projected))
            }

            fn read_table(&self, table: &str) -> Vec<Row> {
                if let Some(local) = &self.local {
                    return local.get(table).cloned().unwrap_or_default();
                }
                self.store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .get(table)
                    .cloned()
                    .unwrap_or_default()
            }

            fn put_table(&mut self, table: &str, rows: Vec<Row>) {
                if let Some(local) = self.local.as_mut() {
                    local.insert(table.to_string(), rows);
                    self.dirty.insert(table.to_string());
                    return;
                }
                self.store
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .insert(table.to_string(), rows);
            }
        }

        fn keep(pred: &Option<Pred>, row: &Row, params: &[Value], start: usize) -> Result<bool, Error> {
            let Some(pred) = pred else {
                return Ok(true);
            };
            let mut idx = start;
            eval_pred(pred, row, params, &mut idx)
        }

        fn pred_slots(pred: &Option<Pred>) -> usize {
            pred.as_ref().map(count_slots).unwrap_or(0)
        }

        fn count_slots(pred: &Pred) -> usize {
            match pred {
                Pred::Cmp { .. } => 1,
                Pred::IsNull { .. } => 0,
                Pred::And(parts) | Pred::Or(parts) => parts.iter().map(count_slots).sum(),
            }
        }

        fn eval_pred(pred: &Pred, row: &Row, params: &[Value], idx: &mut usize) -> Result<bool, Error> {
            match pred {
                Pred::Cmp { field, op } => {
                    let param = take_param(params, idx)?;
                    let left = row.get(field).cloned().unwrap_or(Value::Null);
                    Ok(compare(&left, *op, param))
                }
                Pred::IsNull { field, negated } => {
                    let is_null = row.get(field).is_none_or(Value::is_null);
                    Ok(if *negated { !is_null } else { is_null })
                }
                Pred::And(parts) => {
                    let mut all = true;
                    for part in parts {
                        if !eval_pred(part, row, params, idx)? {
                            all = false;
                        }
                    }
                    Ok(all)
                }
                Pred::Or(parts) => {
                    let mut any = false;
                    for part in parts {
                        if eval_pred(part, row, params, idx)? {
                            any = true;
                        }
                    }
                    Ok(any)
                }
            }
        }

        fn compare(left: &Value, op: Op, right: &Value) -> bool {
            let ordering = cmp_values(left, right);
            match op {
                Op::Eq => left == right,
                Op::Ne => left != right,
                Op::Lt => ordering == std::cmp::Ordering::Less,
                Op::Gt => ordering == std::cmp::Ordering::Greater,
                Op::Le => ordering != std::cmp::Ordering::Greater,
                Op::Ge => ordering != std::cmp::Ordering::Less,
            }
        }

        fn cmp_values(left: &Value, right: &Value) -> std::cmp::Ordering {
            if left.is_null() && right.is_null() {
                return std::cmp::Ordering::Equal;
            }
            if left.is_null() {
                return std::cmp::Ordering::Greater;
            }
            if right.is_null() {
                return std::cmp::Ordering::Less;
            }
            compare_obj(left, right).unwrap_or(std::cmp::Ordering::Equal)
        }

        fn sort_rows(left: &Row, right: &Row, order: &[OrderKey]) -> std::cmp::Ordering {
            for key in order {
                let left_value = left.get(&key.field).cloned().unwrap_or(Value::Null);
                let right_value = right.get(&key.field).cloned().unwrap_or(Value::Null);
                let mut ordering = cmp_values(&left_value, &right_value);
                if key.desc {
                    ordering = ordering.reverse();
                }
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        }

        fn project(row: &Row, columns: &[String]) -> DbRow {
            DbRow {
                columns: columns
                    .iter()
                    .map(|name| (name.clone(), row.get(name).cloned().unwrap_or(Value::Null)))
                    .collect(),
            }
        }

        fn take_param<'a>(params: &'a [Value], idx: &mut usize) -> Result<&'a Value, Error> {
            let value = params.get(*idx).ok_or_else(|| Error::new("sql parameter missing"))?;
            *idx += 1;
            Ok(value)
        }

        fn as_i64(value: &Value) -> Result<i64, Error> {
            match value {
                Value::I64(number) => Ok(*number),
                Value::I32(number) => Ok(i64::from(*number)),
                Value::String(text) => text.parse().map_err(|_| Error::new("parameter is not an integer")),
                _ => Err(Error::new("parameter is not an integer")),
            }
        }

        enum Stmt {
            Insert {
                table: String,
                columns: Vec<String>,
            },
            Update {
                table: String,
                columns: Vec<String>,
                pred: Option<Pred>,
            },
            Delete {
                table: String,
                pred: Option<Pred>,
            },
            Select(SelectQuery),
        }

        struct SelectQuery {
            count: bool,
            columns: Vec<String>,
            table: String,
            pred: Option<Pred>,
            order: Vec<OrderKey>,
            limit: bool,
            offset: bool,
        }

        enum Pred {
            Cmp { field: String, op: Op },
            IsNull { field: String, negated: bool },
            And(Vec<Pred>),
            Or(Vec<Pred>),
        }

        #[derive(Clone, Copy)]
        enum Op {
            Eq,
            Ne,
            Lt,
            Gt,
            Le,
            Ge,
        }

        struct OrderKey {
            field: String,
            desc: bool,
        }

        fn parse(sql: &str) -> Result<Stmt, Error> {
            let mut cur = Cur { sql, i: 0 };
            let stmt = if cur.eat_keyword("INSERT") {
                parse_insert(&mut cur)?
            } else if cur.eat_keyword("UPDATE") {
                parse_update(&mut cur)?
            } else if cur.eat_keyword("DELETE") {
                parse_delete(&mut cur)?
            } else if cur.eat_keyword("SELECT") {
                parse_select(&mut cur)?
            } else {
                return Err(cur.fail("unsupported sql"));
            };
            Ok(stmt)
        }

        fn parse_insert(cur: &mut Cur) -> Result<Stmt, Error> {
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
            Ok(Stmt::Insert { table, columns })
        }

        fn parse_update(cur: &mut Cur) -> Result<Stmt, Error> {
            let table = cur.quoted()?;
            cur.expect_keyword("SET")?;
            let mut columns = Vec::new();
            loop {
                columns.push(cur.quoted()?);
                cur.expect_symbol(b'=')?;
                cur.expect_symbol(b'?')?;
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            cur.expect_end()?;
            Ok(Stmt::Update { table, columns, pred })
        }

        fn parse_delete(cur: &mut Cur) -> Result<Stmt, Error> {
            cur.expect_keyword("FROM")?;
            let table = cur.quoted()?;
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            cur.expect_end()?;
            Ok(Stmt::Delete { table, pred })
        }

        fn parse_select(cur: &mut Cur) -> Result<Stmt, Error> {
            let count = if cur.eat_keyword("COUNT") {
                cur.expect_symbol(b'(')?;
                cur.expect_symbol(b'*')?;
                cur.expect_symbol(b')')?;
                true
            } else {
                false
            };
            let columns = if count { Vec::new() } else { parse_columns(cur)? };
            cur.expect_keyword("FROM")?;
            let table = cur.quoted()?;
            let pred = if cur.eat_keyword("WHERE") {
                Some(parse_or(cur)?)
            } else {
                None
            };
            let order = if cur.eat_keyword("ORDER") {
                cur.expect_keyword("BY")?;
                parse_order(cur)?
            } else {
                Vec::new()
            };
            let limit = if cur.eat_keyword("LIMIT") {
                cur.expect_symbol(b'?')?;
                true
            } else {
                false
            };
            let offset = if cur.eat_keyword("OFFSET") {
                cur.expect_symbol(b'?')?;
                true
            } else {
                false
            };
            cur.expect_end()?;
            Ok(Stmt::Select(SelectQuery {
                count,
                columns,
                table,
                pred,
                order,
                limit,
                offset,
            }))
        }

        fn parse_columns(cur: &mut Cur) -> Result<Vec<String>, Error> {
            let mut columns = Vec::new();
            loop {
                if cur.peek_keyword("FROM") {
                    break;
                }
                columns.push(cur.quoted()?);
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            if columns.is_empty() {
                return Err(cur.fail("select has no columns"));
            }
            Ok(columns)
        }

        fn parse_order(cur: &mut Cur) -> Result<Vec<OrderKey>, Error> {
            let mut order = Vec::new();
            loop {
                let field = cur.quoted()?;
                let desc = if cur.eat_keyword("DESC") {
                    true
                } else if cur.eat_keyword("ASC") {
                    false
                } else {
                    return Err(cur.fail("expected ASC or DESC"));
                };
                order.push(OrderKey { field, desc });
                if !cur.eat_symbol(b',') {
                    break;
                }
            }
            Ok(order)
        }

        fn parse_or(cur: &mut Cur) -> Result<Pred, Error> {
            let mut parts = vec![parse_and(cur)?];
            while cur.eat_keyword("OR") {
                parts.push(parse_and(cur)?);
            }
            Ok(fold(parts, false))
        }

        fn parse_and(cur: &mut Cur) -> Result<Pred, Error> {
            let mut parts = vec![parse_primary(cur)?];
            while cur.eat_keyword("AND") {
                parts.push(parse_primary(cur)?);
            }
            Ok(fold(parts, true))
        }

        fn parse_primary(cur: &mut Cur) -> Result<Pred, Error> {
            if cur.eat_symbol(b'(') {
                let pred = parse_or(cur)?;
                cur.expect_symbol(b')')?;
                return Ok(pred);
            }
            let field = cur.quoted()?;
            if cur.eat_keyword("IS") {
                let negated = cur.eat_keyword("NOT");
                cur.expect_keyword("NULL")?;
                return Ok(Pred::IsNull { field, negated });
            }
            let Some(op) = cur.eat_op() else {
                return Err(cur.fail("expected comparison"));
            };
            cur.expect_symbol(b'?')?;
            Ok(Pred::Cmp { field, op })
        }

        fn fold(mut parts: Vec<Pred>, and: bool) -> Pred {
            if parts.len() == 1 {
                return parts.remove(0);
            }
            if and {
                Pred::And(parts)
            } else {
                Pred::Or(parts)
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
                if rest.len() < word.len() || !rest.starts_with(word) {
                    return false;
                }
                let end = self.i + word.len();
                if end < self.sql.len() {
                    let next = self.sql.as_bytes()[end];
                    if next.is_ascii_alphanumeric() || next == b'_' {
                        return false;
                    }
                }
                self.i = end;
                true
            }

            fn peek_keyword(&mut self, word: &str) -> bool {
                let saved = self.i;
                let found = self.eat_keyword(word);
                self.i = saved;
                found
            }

            fn eat_symbol(&mut self, symbol: u8) -> bool {
                self.skip();
                if self.i < self.sql.len() && self.sql.as_bytes()[self.i] == symbol {
                    self.i += 1;
                    true
                } else {
                    false
                }
            }

            fn eat_op(&mut self) -> Option<Op> {
                self.skip();
                let rest = &self.sql[self.i..];
                let (op, len) = if rest.starts_with("<>") {
                    (Op::Ne, 2)
                } else if rest.starts_with("<=") {
                    (Op::Le, 2)
                } else if rest.starts_with(">=") {
                    (Op::Ge, 2)
                } else if rest.starts_with('<') {
                    (Op::Lt, 1)
                } else if rest.starts_with('>') {
                    (Op::Gt, 1)
                } else if rest.starts_with('=') {
                    (Op::Eq, 1)
                } else {
                    return None;
                };
                self.i += len;
                Some(op)
            }

            fn quoted(&mut self) -> Result<String, Error> {
                if !self.eat_symbol(b'"') {
                    return Err(self.fail("expected identifier"));
                }
                let start = self.i;
                while self.i < self.sql.len() && self.sql.as_bytes()[self.i] != b'"' {
                    self.i += 1;
                }
                if self.i >= self.sql.len() {
                    return Err(self.fail("unterminated identifier"));
                }
                let name = self.sql[start..self.i].to_string();
                self.i += 1;
                Ok(name)
            }

            fn expect_keyword(&mut self, word: &str) -> Result<(), Error> {
                if self.eat_keyword(word) {
                    Ok(())
                } else {
                    Err(self.fail(&format!("expected {word}")))
                }
            }

            fn expect_symbol(&mut self, symbol: u8) -> Result<(), Error> {
                if self.eat_symbol(symbol) {
                    Ok(())
                } else {
                    Err(self.fail(&format!("expected {}", char::from(symbol))))
                }
            }

            fn expect_end(&mut self) -> Result<(), Error> {
                self.skip();
                if self.i == self.sql.len() {
                    Ok(())
                } else {
                    Err(self.fail("unexpected sql"))
                }
            }

            fn fail(&self, message: &str) -> Error {
                let rest: String = self.sql[self.i..].chars().take(160).collect();
                Error::new(format!("{message} at '{rest}'"))
            }
        }
    }
}
