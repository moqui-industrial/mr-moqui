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

//! Artifact authorization from `ArtifactAuthz` and `ArtifactGroupMember`.

use std::collections::HashSet;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::login::entity_facade;
use super::session::current_user_id;
use crate::context::{Error, ExecutionContext};
use crate::entity::{EntityFacade, EntityValue};

/// True while an [`crate::context::ExecutionContext::enter`] guard is live on this thread.
pub fn context_entered() -> bool {
    ExecutionContext::current().is_some()
}

/// Allow the artifact when no context is entered, authz is disabled, or the conf flag is not true.
///
/// Otherwise a matching `AUTHZT_ALWAYS` allows immediately, `AUTHZT_DENY` beats `AUTHZT_ALLOW`,
/// and no matching allow is an error. `authzServiceName` is ignored.
pub fn authorize(artifact_type: &str, name: &str, action: &str) -> Result<(), Error> {
    if !context_entered() {
        return Ok(());
    }
    if ExecutionContext::with_current(|ctx| ctx.authz_disabled()).unwrap_or(false) {
        return Ok(());
    }
    if !authz_enabled_for(artifact_type) {
        return Ok(());
    }
    with_disabled_facade(|facade| decide(facade, artifact_type, name, action))
}

/// True when `artifact-execution` for this type has `authz-enabled="true"`.
pub(crate) fn authz_enabled_for(artifact_type: &str) -> bool {
    artifact_flag(artifact_type, "authz-enabled")
}

/// Artifact authz ids that grant `action` on `name`. Empty when a deny wins or nothing allows.
pub(crate) fn granting_ids(artifact_type: &str, name: &str, action: &str) -> Result<Vec<String>, Error> {
    with_disabled_facade(|facade| {
        let groups = user_groups(facade)?;
        let checks = load_checks(facade)?;
        let mut always = Vec::new();
        let mut allow = Vec::new();
        let mut deny = false;
        for check in checks {
            if !matches_check(&groups, &check, artifact_type, name, action) {
                continue;
            }
            match check.authz_type.as_str() {
                "AUTHZT_ALWAYS" => always.push(check.artifact_authz_id),
                "AUTHZT_DENY" => deny = true,
                "AUTHZT_ALLOW" => allow.push(check.artifact_authz_id),
                _ => {}
            }
        }
        if !always.is_empty() {
            return Ok(always);
        }
        if deny {
            return Ok(Vec::new());
        }
        Ok(allow)
    })
}

fn decide(facade: &EntityFacade, artifact_type: &str, name: &str, action: &str) -> Result<(), Error> {
    if tarpit_locked(facade, artifact_type, name)? {
        return Err(denied(artifact_type, name, action));
    }
    let groups = user_groups(facade)?;
    let checks = load_checks(facade)?;
    let mut denied_hit = false;
    let mut allowed = false;
    for check in &checks {
        if !matches_check(&groups, check, artifact_type, name, action) {
            continue;
        }
        match check.authz_type.as_str() {
            "AUTHZT_ALWAYS" => return Ok(()),
            "AUTHZT_DENY" => denied_hit = true,
            "AUTHZT_ALLOW" => allowed = true,
            _ => {}
        }
    }
    if denied_hit || !allowed {
        return Err(denied(artifact_type, name, action));
    }
    Ok(())
}

fn denied(artifact_type: &str, name: &str, action: &str) -> Error {
    let user = current_user_id().ok().flatten().unwrap_or_default();
    Error::new(format!(
        "User [{user}] is not authorized for {artifact_type} [{name}] action {action}"
    ))
}

fn with_disabled_facade<T>(body: impl FnOnce(&EntityFacade) -> Result<T, Error>) -> Result<T, Error> {
    let facade = match entity_facade() {
        Ok(facade) => facade,
        Err(err) => return Err(Error::new(format!("User is not authorized ({err})"))),
    };
    let _guard = ExecutionContext::with_current(|ctx| ctx.disable_authz())
        .ok_or_else(|| Error::new("User is not authorized (no execution context)"))?;
    body(&facade)
}

/// An existing lock blocks only when `tarpit-enabled="true"`. Hits are not recorded.
fn tarpit_locked(facade: &EntityFacade, artifact_type: &str, name: &str) -> Result<bool, Error> {
    if !artifact_flag(artifact_type, "tarpit-enabled") {
        return Ok(false);
    }
    let Some(user_id) = current_user_id()? else {
        return Ok(false);
    };
    if user_id.is_empty() {
        return Ok(false);
    }
    let now = super::login::now_value();
    let rows = facade.find("moqui.security.ArtifactTarpitLock")?.list()?;
    for row in rows.values() {
        if text(row, "userId") != user_id
            || text(row, "artifactTypeEnumId") != artifact_type
            || text(row, "artifactName") != name
        {
            continue;
        }
        let Some(release) = row.get("releaseDateTime") else {
            continue;
        };
        if release.is_null() {
            continue;
        }
        if moqui_lib::object::compare_obj(release, &now).ok() == Some(std::cmp::Ordering::Greater) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn artifact_flag(artifact_type: &str, flag: &str) -> bool {
    ExecutionContext::with_current(|ctx| flag_in(ctx.factory().conf_text(), artifact_type, flag)).unwrap_or(false)
}

fn flag_in(text: &str, artifact_type: &str, flag: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let Ok(root) = MNode::parse_text("moqui-conf", text) else {
        return false;
    };
    let Some(facade) = root.first_named("artifact-execution-facade") else {
        return false;
    };
    for node in facade.children_named("artifact-execution") {
        if node.attribute("type").as_deref() != Some(artifact_type) {
            continue;
        }
        return node.attribute(flag).as_deref() == Some("true");
    }
    false
}

struct Check {
    artifact_authz_id: String,
    user_group_id: String,
    authz_type: String,
    authz_action: String,
    artifact_name: String,
    artifact_type: String,
    name_is_pattern: bool,
}

fn load_checks(facade: &EntityFacade) -> Result<Vec<Check>, Error> {
    let authz_rows = facade.find("moqui.security.ArtifactAuthz")?.list()?;
    let member_rows = facade.find("moqui.security.ArtifactGroupMember")?.list()?;
    let mut checks = Vec::new();
    for authz in authz_rows.values() {
        let group_id = text(authz, "artifactGroupId");
        let user_group = text(authz, "userGroupId");
        let authz_id = text(authz, "artifactAuthzId");
        if group_id.is_empty() || user_group.is_empty() || authz_id.is_empty() {
            continue;
        }
        for member in member_rows.values() {
            if text(member, "artifactGroupId") != group_id {
                continue;
            }
            let artifact_name = text(member, "artifactName");
            let artifact_type = text(member, "artifactTypeEnumId");
            if artifact_name.is_empty() || artifact_type.is_empty() {
                continue;
            }
            checks.push(Check {
                artifact_authz_id: authz_id.clone(),
                user_group_id: user_group.clone(),
                authz_type: text(authz, "authzTypeEnumId"),
                authz_action: text(authz, "authzActionEnumId"),
                artifact_name,
                artifact_type,
                name_is_pattern: text(member, "nameIsPattern") == "Y",
            });
        }
    }
    Ok(checks)
}

fn user_groups(facade: &EntityFacade) -> Result<HashSet<String>, Error> {
    let mut groups = HashSet::new();
    groups.insert("ALL_USERS".to_string());
    let Some(user_id) = current_user_id()? else {
        return Ok(groups);
    };
    if user_id.is_empty() {
        return Ok(groups);
    }
    let now = super::login::now_value();
    let rows = facade.find("moqui.security.UserGroupMember")?.list()?;
    for row in rows.values() {
        if text(row, "userId") != user_id || !membership_open(row, &now) {
            continue;
        }
        let group = text(row, "userGroupId");
        if !group.is_empty() {
            groups.insert(group);
        }
    }
    Ok(groups)
}

fn membership_open(row: &EntityValue, now: &Value) -> bool {
    let Some(from) = row.get("fromDate") else {
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
    match row.get("thruDate") {
        None | Some(Value::Null) => true,
        Some(thru) => moqui_lib::object::compare_obj(thru, now).ok() == Some(std::cmp::Ordering::Greater),
    }
}

fn matches_check(groups: &HashSet<String>, check: &Check, artifact_type: &str, name: &str, action: &str) -> bool {
    groups.contains(&check.user_group_id)
        && check.artifact_type == artifact_type
        && action_matches(&check.authz_action, action)
        && name_matches(&check.artifact_name, check.name_is_pattern, name)
}

fn action_matches(record_action: &str, requested: &str) -> bool {
    if record_action == "AUTHZA_ALL" {
        return true;
    }
    let requested_enum = match requested {
        "view" => "AUTHZA_VIEW",
        "create" => "AUTHZA_CREATE",
        "update" => "AUTHZA_UPDATE",
        "delete" => "AUTHZA_DELETE",
        "all" => "AUTHZA_ALL",
        other => other,
    };
    record_action == requested || record_action == requested_enum
}

fn name_matches(member_name: &str, pattern: bool, artifact_name: &str) -> bool {
    if !pattern {
        return member_name == artifact_name;
    }
    moqui_lib::object::compare(
        &Value::string(artifact_name),
        "matches",
        &Value::string(member_name),
        "String",
    )
    .unwrap_or(false)
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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use moqui_lib::object::Value;

    use super::super::login::memdb;
    use super::super::session;
    use crate::context::{ExecutionContext, ExecutionContextFactory};
    use crate::entity::{EntityFacade, EntityValue};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn enter_marks_the_thread_until_the_guard_drops() {
        let context = ExecutionContextFactory::test_context();
        assert!(!super::context_entered());
        assert!(ExecutionContext::current().is_none());
        assert!(context.allows());
        let outer = context.enter();
        assert!(super::context_entered());
        assert!(ExecutionContext::current().is_some());
        let inner = context.enter();
        assert!(super::context_entered());
        drop(inner);
        assert!(super::context_entered());
        drop(outer);
        assert!(!super::context_entered());
        assert!(ExecutionContext::current().is_none());
        assert!(context.allows());
    }

    #[test]
    fn user_without_a_record_is_denied_an_entity_create() {
        session::reset_session();
        let runtime = temp_dir("authz-runtime");
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let factory = ExecutionContextFactory::boot_with_framework(runtime.path(), &framework).expect("boot");
        let facade = memdb::open();
        factory.set_entity(facade.clone());
        let context = factory.new_context();
        let _entered = context.enter();
        session::set_user("nobody", "nobody");

        let row = test_row(&facade);
        let err = row.create(&facade).expect_err("create without an allow");
        assert!(err.to_string().contains("authorized"), "{err}");

        let guard = context.disable_authz();
        row.create(&facade).expect("create with authz disabled");
        drop(guard);

        let found = facade
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("T-DENY"))
            .list()
            .unwrap();
        assert_eq!(found.len(), 1);
        session::reset_session();
    }

    fn test_row(facade: &EntityFacade) -> EntityValue {
        let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
        row.set("testId", Value::string("T-DENY"));
        row
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
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("moqui-{label}-{}-{nanos}-{seq}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}
