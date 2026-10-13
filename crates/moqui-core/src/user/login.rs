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

//! Login against `moqui.security.UserAccount`. A bound web session rotates on password success.

use std::time::{SystemTime, UNIX_EPOCH};

use moqui_db::sql::{select as sql_select, update as sql_update, Condition, FieldRef, LimitOffset};
use moqui_lib::object::{basic_convert, Value};

use super::password::{self, hash_password};
use super::policy::{self, password_acceptable};
use super::session;
use crate::context::{Error, ExecutionContext};
use crate::driver::{DbConnection, DbRow};
use crate::entity::{EntityFacade, EntityValue};

const USER_ACCOUNT: &str = "moqui.security.UserAccount";
const USER_LOGIN_KEY: &str = "moqui.security.UserLoginKey";
#[allow(dead_code)] // Used by change_password, which the library reaches only from tests.
const PASSWORD_HISTORY: &str = "moqui.security.UserPasswordHistory";
const INVALID_LOGIN: &str = "The username or password is not valid";
const WEEK_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Check the password and, on success, set the in-memory user.
///
/// Unknown users, an empty username, and an empty password share one message.
/// A bound web session rotates its id only after this password check succeeds.
pub fn login_user(username: &str, password: &str) -> Result<(), Error> {
    if username.is_empty() || password.is_empty() {
        return Err(Error::new(INVALID_LOGIN));
    }
    // The login reads and writes the user's own account whatever the caller may do.
    let _authz = without_authz();
    let facade = entity_facade()?;
    let Some(account) = find_username(&facade, username)? else {
        password::burn_verify(password);
        return Err(Error::new(INVALID_LOGIN));
    };
    let now = now_millis();
    if let Err(err) = gate_account(&facade, &account, now) {
        password::burn_verify(password);
        return Err(err);
    }
    if !password_ok(&account, password)? {
        let user_id = text_field(&account, "userId");
        if !user_id.is_empty() {
            increment_failed(&facade, &user_id)?;
        }
        return Err(Error::new(INVALID_LOGIN));
    }
    post_password(&account, now)?;
    // A request repeating Basic credentials for the user its session already holds keeps its id.
    let same_user = session::current_username()?.as_deref() == Some(username);
    finish_login(&facade, &account, Some(password), true)?;
    if same_user {
        return Ok(());
    }
    session::rotate_on_password_login()
}

/// Authorization is off while the login code reads and writes the user tables.
fn without_authz() -> Option<crate::context::AuthzGuard> {
    ExecutionContext::with_current(|ctx| ctx.disable_authz())
}

/// Set the in-memory user without checking the password. Does not rotate a bound session.
///
/// # Errors
///
/// Returns an error when the account is unknown, disabled, terminated, or needs a password change.
pub fn internal_login_user(username: &str) -> Result<(), Error> {
    if username.is_empty() {
        return Err(Error::new("No username specified"));
    }
    let _authz = without_authz();
    let facade = entity_facade()?;
    let Some(account) = find_username(&facade, username)? else {
        return Err(Error::new(INVALID_LOGIN));
    };
    let now = now_millis();
    gate_account(&facade, &account, now)?;
    post_password(&account, now)?;
    finish_login(&facade, &account, None, false)
}

/// Log in with a `UserLoginKey`. The stored `loginKey` is [`super::password::hash_login_key`].
///
/// A `thruDate` in the past is rejected. A null `thruDate` stays valid. Does not rotate the session id.
///
/// # Errors
///
/// Returns [`INVALID_LOGIN`](Error) when the key is empty, unknown, or expired, or the account cannot log in.
pub fn login_user_key(key: &str) -> Result<(), Error> {
    if key.is_empty() {
        return Err(Error::new(INVALID_LOGIN));
    }
    let hashed = password::hash_login_key(key)?;
    let _authz = without_authz();
    let facade = entity_facade()?;
    let Some(row) = facade
        .find(USER_LOGIN_KEY)?
        .eq("loginKey", Value::string(&hashed))
        .one()?
    else {
        return Err(Error::new(INVALID_LOGIN));
    };
    if let Some(thru) = field_millis(&row, "thruDate") {
        if now_millis() > thru {
            return Err(Error::new(INVALID_LOGIN));
        }
    }
    let user_id = text_field(&row, "userId");
    let Some(account) = find_user_id(&facade, &user_id)? else {
        return Err(Error::new(INVALID_LOGIN));
    };
    internal_login_user(&text_field(&account, "username"))
}

/// Header login. Uses `api_key` when it is present, otherwise `login_key`.
///
/// Does not read a query string. An existing username returns without a lookup.
///
/// # Errors
///
/// Returns the login error when the chosen key is rejected.
pub fn apply_header_keys(api_key: Option<&str>, login_key: Option<&str>) -> Result<(), Error> {
    if session::current_username()?.is_some() {
        return Ok(());
    }
    let Some(key) = present_key(api_key).or_else(|| present_key(login_key)) else {
        return Ok(());
    };
    login_user_key(key)
}

fn present_key(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}

/// Mark the session anonymous when nobody is logged in. Returns true when it did, so the caller
/// can undo it with [`logout_anonymous_only`]. This does not use the `_NA_` account.
///
/// # Errors
///
/// Returns an error only if the session cannot be read.
pub fn login_anonymous() -> Result<bool, Error> {
    if session::current_username()?.is_some() || session::is_anonymous()? {
        return Ok(false);
    }
    session::set_anonymous(true);
    Ok(true)
}

/// Undo [`login_anonymous`] without touching a real login.
pub fn logout_anonymous_only() {
    if session::current_username().ok().flatten().is_none() {
        session::set_anonymous(false);
    }
}

/// Clear the in-memory user and anonymous flag. Sets `hasLoggedOut` on the account.
///
/// Guarantee mode flushes the bound session after the user is cleared.
///
/// # Errors
///
/// Returns an error when the account row cannot be updated. The session is cleared either way.
pub fn logout() -> Result<(), Error> {
    let _authz = without_authz();
    let update = if let Some(user_id) = session::current_user_id()? {
        match entity_facade() {
            Ok(facade) => mark_logged_out(&facade, &user_id),
            Err(_) => Ok(()),
        }
    } else {
        Ok(())
    };
    session::clear_login();
    update.and(session::flush_after_logout())
}

/// Replace the password with argon2 and push the old hash into `UserPasswordHistory`.
///
/// The history rows and the new password are one transaction, so a failure part way leaves both
/// as they were.
///
/// # Errors
///
/// Returns an error when the policy rejects the password, it was used recently, or a write fails.
pub fn change_password(user_id: &str, new_password: &str) -> Result<(), Error> {
    password_acceptable(new_password)?;
    let _authz = without_authz();
    let facade = entity_facade()?;
    let account = find_user_id(&facade, user_id)?
        .ok_or_else(|| Error::new(format!("Cannot update password, account not found with ID {user_id}")))?;
    if password_ok(&account, new_password)? {
        return Err(Error::new("New password is same as current password"));
    }
    reject_history(&facade, &account, user_id, new_password)?;
    let hashed = hash_password(new_password)?;
    let tx = facade.transactions_for(USER_ACCOUNT)?;
    tx.run(crate::transaction::TransactionAttribute::UseOrBegin, None, || {
        remember_history(&facade, &account, user_id)?;
        trim_history(&facade, user_id)?;
        Ok(update_password_fields(&facade, user_id, &hashed)?)
    })?;
    Ok(())
}

fn update_password_fields(facade: &EntityFacade, user_id: &str, hashed: &str) -> Result<(), Error> {
    update_user(
        facade,
        user_id,
        &[
            ("currentPassword", Value::string(hashed)),
            ("passwordHashType", Value::string("argon2")),
            ("passwordSalt", Value::string("")),
            ("passwordBase64", Value::string("N")),
            ("passwordSetDate", now_value()),
            ("requirePasswordChange", Value::string("N")),
            ("disabled", Value::string("N")),
            ("resetPassword", Value::Null),
            ("resetPasswordSetDate", Value::Null),
        ],
    )
}

pub(crate) fn entity_facade() -> Result<EntityFacade, Error> {
    match ExecutionContext::with_current(|ctx| ctx.factory().entity()) {
        None => Err(Error::new("no execution context")),
        Some(None) => Err(Error::new("no entity facade")),
        Some(Some(facade)) => Ok(facade),
    }
}

pub(crate) fn now_value() -> Value {
    stamp_value(now_millis())
}

pub(crate) fn stamp_value(millis: i64) -> Value {
    basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn gate_account(facade: &EntityFacade, account: &EntityValue, now: i64) -> Result<(), Error> {
    let user_id = text_field(account, "userId");
    if still_disabled(account, now) {
        if !user_id.is_empty() {
            increment_failed(facade, &user_id)?;
        }
        return Err(Error::new(INVALID_LOGIN));
    }
    if terminated(account, now) {
        return Err(Error::new(INVALID_LOGIN));
    }
    Ok(())
}

fn post_password(account: &EntityValue, now: i64) -> Result<(), Error> {
    if text_field(account, "requirePasswordChange") == "Y" {
        return Err(Error::new(
            "Authenticate failed because the account requires a password change",
        ));
    }
    if password_expired(account, now) {
        return Err(Error::new("Authenticate failed because the password must be changed"));
    }
    Ok(())
}

fn finish_login(facade: &EntityFacade, account: &EntityValue, plain: Option<&str>, rehash: bool) -> Result<(), Error> {
    let user_id = text_field(account, "userId");
    let username = text_field(account, "username");
    if user_id.is_empty() || username.is_empty() {
        return Err(Error::new(INVALID_LOGIN));
    }
    let mut fields = Vec::new();
    if rehash && legacy_hash(account) {
        if let Some(plain) = plain {
            let hashed = hash_password(plain)?;
            fields.push(("currentPassword", Value::string(hashed)));
            fields.push(("passwordHashType", Value::string("argon2")));
            fields.push(("passwordSalt", Value::string("")));
            fields.push(("passwordBase64", Value::string("N")));
        }
    }
    if needs_reset(account) {
        fields.push(("successiveFailedLogins", Value::I64(0)));
        fields.push(("disabled", Value::string("N")));
        fields.push(("disabledDateTime", Value::Null));
        fields.push(("hasLoggedOut", Value::string("N")));
    }
    if !fields.is_empty() {
        update_user(facade, &user_id, &fields)?;
    }
    session::set_user(&user_id, &username);
    let locale = nonempty(text_field(account, "locale"));
    let zone = nonempty(text_field(account, "timeZone"));
    session::set_locale(locale.as_deref());
    session::set_time_zone(zone.as_deref());
    link_visit(facade, &user_id)
}

/// The visit that began before login now belongs to the user.
fn link_visit(facade: &EntityFacade, user_id: &str) -> Result<(), Error> {
    let Some(visit_id) = session::visit_id()?.filter(|id| !id.is_empty()) else {
        return Ok(());
    };
    let mut visit = facade.make_value("moqui.server.Visit")?;
    visit.set("visitId", Value::string(visit_id));
    visit.set("userId", Value::string(user_id));
    visit.update(facade)?;
    Ok(())
}

fn legacy_hash(account: &EntityValue) -> bool {
    !password::is_argon2(
        &text_field(account, "passwordHashType"),
        &text_field(account, "currentPassword"),
    )
}

fn needs_reset(account: &EntityValue) -> bool {
    field_i64(account, "successiveFailedLogins").unwrap_or(0) != 0
        || text_field(account, "disabled") == "Y"
        || field_millis(account, "disabledDateTime").is_some()
        || text_field(account, "hasLoggedOut") == "Y"
}

fn still_disabled(account: &EntityValue, now: i64) -> bool {
    if text_field(account, "disabled") != "Y" {
        return false;
    }
    let Some(disabled_at) = field_millis(account, "disabledDateTime") else {
        return true;
    };
    let window_ms = policy::disable_minutes().saturating_mul(60_000);
    disabled_at.saturating_add(window_ms) > now
}

fn terminated(account: &EntityValue, now: i64) -> bool {
    field_millis(account, "terminateDate").is_some_and(|term| now > term)
}

fn password_expired(account: &EntityValue, now: i64) -> bool {
    let Some(set_at) = field_millis(account, "passwordSetDate") else {
        return false;
    };
    let weeks = policy::change_weeks();
    if weeks <= 0 {
        return false;
    }
    now.saturating_sub(set_at) / WEEK_MILLIS > weeks
}

fn password_ok(account: &EntityValue, password: &str) -> Result<bool, Error> {
    password::passwords_match(
        password,
        &text_field(account, "currentPassword"),
        &text_field(account, "passwordHashType"),
        &text_field(account, "passwordSalt"),
        text_field(account, "passwordBase64") == "Y",
    )
}

#[allow(dead_code)]
fn reject_history(
    facade: &EntityFacade,
    account: &EntityValue,
    user_id: &str,
    new_password: &str,
) -> Result<(), Error> {
    let base64 = text_field(account, "passwordBase64") == "Y";
    let rows = facade
        .find(PASSWORD_HISTORY)?
        .eq("userId", Value::string(user_id))
        .list()?;
    for row in rows.iter() {
        let matched = password::passwords_match(
            new_password,
            &text_field(row, "password"),
            &text_field(row, "passwordHashType"),
            &text_field(row, "passwordSalt"),
            base64,
        )?;
        if matched {
            let limit = policy::history_limit();
            return Err(Error::new(format!("Password was used in last {limit} passwords")));
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn remember_history(facade: &EntityFacade, account: &EntityValue, user_id: &str) -> Result<(), Error> {
    let mut row = facade.make_value(PASSWORD_HISTORY)?;
    row.set("userId", Value::string(user_id));
    row.set("fromDate", now_value());
    row.set("password", Value::string(text_field(account, "currentPassword")));
    row.set("passwordSalt", Value::string(text_field(account, "passwordSalt")));
    row.set(
        "passwordHashType",
        Value::string(text_field(account, "passwordHashType")),
    );
    row.create(facade)?;
    Ok(())
}

#[allow(dead_code)]
fn trim_history(facade: &EntityFacade, user_id: &str) -> Result<(), Error> {
    let rows = facade
        .find(PASSWORD_HISTORY)?
        .eq("userId", Value::string(user_id))
        .order_by("fromDate", false)
        .list()?;
    let limit = usize::try_from(policy::history_limit().max(0)).unwrap_or(0);
    let overflow = rows.len().saturating_sub(limit);
    for row in rows.iter().take(overflow) {
        row.delete(facade)?;
    }
    Ok(())
}

fn find_username(facade: &EntityFacade, username: &str) -> Result<Option<EntityValue>, Error> {
    Ok(facade
        .find(USER_ACCOUNT)?
        .eq("username", Value::string(username))
        .one()?)
}

fn find_user_id(facade: &EntityFacade, user_id: &str) -> Result<Option<EntityValue>, Error> {
    Ok(facade.find(USER_ACCOUNT)?.eq("userId", Value::string(user_id)).one()?)
}

fn update_user(facade: &EntityFacade, user_id: &str, fields: &[(&str, Value)]) -> Result<(), Error> {
    let mut value = facade.make_value(USER_ACCOUNT)?;
    value.set("userId", Value::string(user_id));
    for (name, field) in fields {
        value.set(*name, field.clone());
    }
    value.update(facade)?;
    Ok(())
}

#[allow(dead_code)]
fn mark_logged_out(facade: &EntityFacade, user_id: &str) -> Result<(), Error> {
    update_user(facade, user_id, &[("hasLoggedOut", Value::string("Y"))])
}

/// Failed-login count commits on a reserve connection, so the caller's rollback does not drop it.
///
/// The update only applies to the count that was read, and a lost race reads again, so parallel
/// guesses cannot count as one.
fn increment_failed(facade: &EntityFacade, user_id: &str) -> Result<(), Error> {
    let statements = facade.with_entity(USER_ACCOUNT, |entity, dialect, _| {
        let by_user = Condition::equals("userId", Value::string(user_id));
        let fields = [
            FieldRef::new("successiveFailedLogins"),
            FieldRef::new("disabled"),
            FieldRef::new("disabledDateTime"),
        ];
        let set = ["successiveFailedLogins", "disabled", "disabledDateTime"];
        let select_sql = sql_select(entity, dialect, &fields, Some(&by_user), &[], LimitOffset::default())?.sql;
        let counted = Condition::and(vec![
            by_user.clone(),
            Condition::equals("successiveFailedLogins", Value::I64(0)),
        ]);
        let uncounted = Condition::and(vec![by_user.clone(), Condition::is_null("successiveFailedLogins")]);
        Ok(IncrementSql {
            select: select_sql,
            update_counted: sql_update(entity, dialect, &set, Some(&counted))?.sql,
            update_uncounted: sql_update(entity, dialect, &set, Some(&uncounted))?.sql,
        })
    })?;
    let tx = facade.transactions_for(USER_ACCOUNT)?;
    let user_id = user_id.to_string();
    tx.with_reserve(|conn| apply_failed_increment(conn, &statements, &user_id))?;
    Ok(())
}

struct IncrementSql {
    select: String,
    update_counted: String,
    update_uncounted: String,
}

const INCREMENT_ATTEMPTS: usize = 8;

fn apply_failed_increment(
    conn: &mut dyn DbConnection,
    statements: &IncrementSql,
    user_id: &str,
) -> Result<(), moqui_lib::error::Error> {
    conn.begin()?;
    let outcome = write_failed_increment(conn, statements, user_id);
    match outcome {
        Ok(()) => conn.commit(),
        Err(err) => {
            let _ = conn.rollback();
            Err(err)
        }
    }
}

fn write_failed_increment(
    conn: &mut dyn DbConnection,
    statements: &IncrementSql,
    user_id: &str,
) -> Result<(), moqui_lib::error::Error> {
    let key = [Value::string(user_id)];
    for _ in 0..INCREMENT_ATTEMPTS {
        let rows = conn.query(&statements.select, &key)?;
        let Some(row) = rows.first() else {
            return Err(moqui_lib::error::Error::new(format!("no UserAccount {user_id}")));
        };
        let previous = row.get("successiveFailedLogins").filter(|value| !value.is_null());
        let next = row_i64(row, "successiveFailedLogins").saturating_add(1);
        let disable = next > policy::max_failures();
        let disabled = if disable {
            Value::string("Y")
        } else {
            row_value(row, "disabled")
        };
        let when = if disable {
            now_value()
        } else {
            row_value(row, "disabledDateTime")
        };
        let mut params = vec![Value::I64(next), disabled, when, Value::string(user_id)];
        let sql = match previous {
            Some(_) => {
                params.push(Value::I64(row_i64(row, "successiveFailedLogins")));
                &statements.update_counted
            }
            None => &statements.update_uncounted,
        };
        if conn.execute(sql, &params)? > 0 {
            return Ok(());
        }
    }
    Err(moqui_lib::error::Error::new(format!(
        "could not count a failed login for {user_id}: the account kept changing"
    )))
}

fn row_value(row: &DbRow, name: &str) -> Value {
    row.get(name).cloned().unwrap_or(Value::Null)
}

fn row_i64(row: &DbRow, name: &str) -> i64 {
    match row.get(name) {
        Some(Value::I64(number)) => *number,
        Some(Value::I32(number)) => i64::from(*number),
        _ => 0,
    }
}

fn text_field(value: &EntityValue, name: &str) -> String {
    value.get(name).and_then(Value::as_str).unwrap_or("").to_string()
}

fn field_i64(value: &EntityValue, name: &str) -> Option<i64> {
    value.get(name).and_then(value_i64)
}

fn field_millis(value: &EntityValue, name: &str) -> Option<i64> {
    value.get(name).and_then(value_millis)
}

fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::I64(number) => Some(*number),
        Value::I32(number) => Some(i64::from(*number)),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub(crate) fn value_millis(value: &Value) -> Option<i64> {
    match value {
        Value::Timestamp(timestamp) => Some(timestamp.as_millisecond()),
        other => value_i64(other),
    }
}

fn nonempty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use moqui_lib::object::{basic_convert, Value};

    use super::super::password::{passwords_match, simple_hash};
    use super::super::session;

    use super::*;
    use crate::context::ExecutionContextFactory;
    use crate::entity::EntityFacade;

    #[test]
    fn login_user_sets_current_username_and_logout_clears_it() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        session::add_flash_message("keep");
        assert_eq!(login_user("", "Secret1!").unwrap_err().to_string(), INVALID_LOGIN);
        assert_eq!(login_user("ada", "").unwrap_err().to_string(), INVALID_LOGIN);
        assert_eq!(
            login_user("missing", "Secret1!").unwrap_err().to_string(),
            INVALID_LOGIN
        );
        assert_eq!(session::flash_messages().unwrap(), vec!["keep".to_string()]);
        assert!(session::current_username().unwrap().is_none());

        login_anonymous().unwrap();
        assert!(session::current_username().unwrap().is_none());
        assert!(session::is_anonymous().unwrap());

        create_user(&facade, "ada", "Secret1!", |value| {
            value.set("locale", Value::string("en"));
            value.set("timeZone", Value::string("UTC"));
        });
        login_user("ada", "Secret1!").unwrap();
        assert_eq!(session::current_username().unwrap().as_deref(), Some("ada"));
        assert!(!session::is_anonymous().unwrap());
        assert_eq!(session::locale().unwrap().as_deref(), Some("en"));
        assert_eq!(session::time_zone().unwrap().as_deref(), Some("UTC"));
        assert_eq!(session::csrf_token().unwrap().len(), 40);
        assert_eq!(session::flash_messages().unwrap(), vec!["keep".to_string()]);

        logout().unwrap();
        assert!(session::current_username().unwrap().is_none());
        assert!(!session::is_anonymous().unwrap());
        let row = load_user(&facade, "ada");
        assert_eq!(text(&row, "hasLoggedOut"), "Y");
        session::reset_session();
    }

    #[test]
    fn a_legacy_hash_verifies_and_the_stored_value_becomes_an_argon2_string() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        create_user(&facade, "ada", "Secret1!", |_| {});
        login_user("ada", "Secret1!").unwrap();
        let row = load_user(&facade, "ada");
        let stored = text(&row, "currentPassword");
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert_eq!(text(&row, "passwordHashType"), "argon2");
        assert!(passwords_match("Secret1!", &stored, "argon2", "", false).unwrap());
        logout().unwrap();
        session::reset_session();
    }

    #[test]
    fn the_fourth_failed_login_disables_the_account_the_third_does_not_and_it_survives_rollback() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        session::add_flash_message("keep");
        create_user(&facade, "ada", "Secret1!", |_| {});
        for _ in 0..3 {
            assert_eq!(login_user("ada", "wrong").unwrap_err().to_string(), INVALID_LOGIN);
        }
        let before = load_user(&facade, "ada");
        assert_eq!(i64_field(&before, "successiveFailedLogins"), 3);
        assert_ne!(text(&before, "disabled"), "Y");

        facade.begin(USER_ACCOUNT, None).unwrap();
        let mut hinted = load_user(&facade, "ada");
        hinted.set("passwordHint", Value::string("changed"));
        hinted.update(&facade).unwrap();
        let failed = login_user("ada", "wrong");
        facade.rollback(USER_ACCOUNT).unwrap();
        assert_eq!(failed.unwrap_err().to_string(), INVALID_LOGIN);

        let after = load_user(&facade, "ada");
        assert_eq!(i64_field(&after, "successiveFailedLogins"), 4);
        assert_eq!(text(&after, "disabled"), "Y");
        assert!(!after.get("disabledDateTime").is_none_or(Value::is_null));
        assert_eq!(text(&after, "passwordHint"), "original");
        assert!(session::current_username().unwrap().is_none());
        assert_eq!(session::flash_messages().unwrap(), vec!["keep".to_string()]);
        session::reset_session();
    }

    #[test]
    fn disabled_without_a_timestamp_stays_disabled_and_terminate_does_not_increment() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        create_user(&facade, "stuck", "Secret1!", |value| {
            value.set("disabled", Value::string("Y"));
        });
        assert_eq!(login_user("stuck", "Secret1!").unwrap_err().to_string(), INVALID_LOGIN);
        let stuck = load_user(&facade, "stuck");
        assert_eq!(text(&stuck, "disabled"), "Y");
        assert_eq!(i64_field(&stuck, "successiveFailedLogins"), 1);
        assert!(!text(&stuck, "currentPassword").starts_with("$argon2"));

        create_user(&facade, "gone", "Secret1!", |value| {
            value.set("terminateDate", stamp(1));
        });
        assert_eq!(login_user("gone", "Secret1!").unwrap_err().to_string(), INVALID_LOGIN);
        let gone = load_user(&facade, "gone");
        assert_eq!(i64_field(&gone, "successiveFailedLogins"), 0);
        assert!(!text(&gone, "currentPassword").starts_with("$argon2"));
        session::reset_session();
    }

    #[test]
    fn a_correct_password_logs_in_after_the_disable_window() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        let opened = now_millis() - 6 * 60_000;
        create_user(&facade, "ada", "Secret1!", |value| {
            value.set("disabled", Value::string("Y"));
            value.set("disabledDateTime", stamp(opened));
            value.set("successiveFailedLogins", Value::I64(4));
        });
        login_user("ada", "Secret1!").unwrap();
        assert_eq!(session::current_username().unwrap().as_deref(), Some("ada"));
        let row = load_user(&facade, "ada");
        assert_eq!(text(&row, "disabled"), "N");
        assert!(row.get("disabledDateTime").is_none_or(Value::is_null));
        assert_eq!(i64_field(&row, "successiveFailedLogins"), 0);
        assert!(text(&row, "currentPassword").starts_with("$argon2id$"));
        logout().unwrap();
        session::reset_session();
    }

    #[test]
    fn require_password_change_and_an_old_password_fail_without_rehash() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        create_user(&facade, "must", "Secret1!", |value| {
            value.set("requirePasswordChange", Value::string("Y"));
        });
        let err = login_user("must", "Secret1!").unwrap_err();
        assert!(err.to_string().contains("password change"), "{err}");
        assert_eq!(i64_field(&load_user(&facade, "must"), "successiveFailedLogins"), 0);
        assert!(!text(&load_user(&facade, "must"), "currentPassword").starts_with("$argon2"));

        let old = now_millis() - 13 * WEEK_MILLIS;
        create_user(&facade, "stale", "Secret1!", |value| {
            value.set("passwordSetDate", stamp(old));
        });
        let err = login_user("stale", "Secret1!").unwrap_err();
        assert!(err.to_string().contains("must be changed"), "{err}");
        assert!(!text(&load_user(&facade, "stale"), "currentPassword").starts_with("$argon2"));
        assert!(session::current_username().unwrap().is_none());
        session::reset_session();
    }

    #[test]
    fn a_password_below_the_current_policy_still_logs_in() {
        // The policy applies when a password is set. Java does not check it at login, so a user
        // whose password predates the policy is not locked out.
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        create_user(&facade, "ada", "secret", |_| {});
        login_user("ada", "secret").unwrap();
        assert_eq!(session::current_username().unwrap().as_deref(), Some("ada"));
        assert!(text(&load_user(&facade, "ada"), "currentPassword").starts_with("$argon2"));
        session::reset_session();
    }

    #[test]
    fn internal_login_skips_the_password_and_anonymous_does_not_set_a_username() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        create_user(&facade, "ada", "Secret1!", |_| {});
        internal_login_user("ada").unwrap();
        assert_eq!(session::current_username().unwrap().as_deref(), Some("ada"));
        assert!(!text(&load_user(&facade, "ada"), "currentPassword").starts_with("$argon2"));
        login_anonymous().unwrap();
        assert_eq!(session::current_username().unwrap().as_deref(), Some("ada"));
        logout().unwrap();
        login_anonymous().unwrap();
        assert!(session::current_username().unwrap().is_none());
        assert!(session::is_anonymous().unwrap());
        assert_eq!(
            internal_login_user("").unwrap_err().to_string(),
            "No username specified"
        );
        session::reset_session();
    }

    #[test]
    fn change_password_stores_argon2_and_rejects_history() {
        let (ctx, facade) = open_ctx();
        let _guard = ctx.enter();
        create_user(&facade, "ada", "Secret1!", |_| {});
        for millis in 1..=5 {
            let mut row = facade.make_value(PASSWORD_HISTORY).unwrap();
            row.set("userId", Value::string("ada"));
            row.set("fromDate", stamp(millis));
            row.set("password", Value::string("not-a-real-hash"));
            row.set("passwordSalt", Value::string("s4lt"));
            row.set("passwordHashType", Value::string("SHA-256"));
            row.create(&facade).unwrap();
        }
        change_password("ada", "Secret2!").unwrap();
        let account = load_user(&facade, "ada");
        let stored = text(&account, "currentPassword");
        assert!(stored.starts_with("$argon2id$"), "{stored}");
        assert_eq!(text(&account, "passwordHashType"), "argon2");
        let history = facade
            .find(PASSWORD_HISTORY)
            .unwrap()
            .eq("userId", Value::string("ada"))
            .order_by("fromDate", false)
            .list()
            .unwrap();
        assert_eq!(history.len(), 5);
        assert!(history.iter().all(|row| field_millis(row, "fromDate") != Some(1)));
        let current = change_password("ada", "Secret2!").unwrap_err();
        assert!(current.to_string().contains("current"), "{current}");
        let reused = change_password("ada", "Secret1!").unwrap_err();
        assert!(reused.to_string().contains("last"), "{reused}");
        session::reset_session();
    }

    fn open_ctx() -> (crate::context::ExecutionContext, EntityFacade) {
        session::reset_session();
        let ctx = ExecutionContextFactory::test_context();
        let facade = crate::testing::open();
        ctx.factory().set_entity(facade.clone());
        (ctx, facade)
    }

    fn create_user(facade: &EntityFacade, username: &str, password: &str, fill: impl FnOnce(&mut EntityValue)) {
        let stored = simple_hash(password, "s4lt", "SHA-256", false).unwrap();
        let mut value = facade.make_value(USER_ACCOUNT).unwrap();
        value.set("userId", Value::string(username));
        value.set("username", Value::string(username));
        value.set("currentPassword", Value::string(stored));
        value.set("passwordSalt", Value::string("s4lt"));
        value.set("passwordHashType", Value::string("SHA-256"));
        value.set("passwordBase64", Value::string("N"));
        value.set("passwordHint", Value::string("original"));
        fill(&mut value);
        value.create(facade).unwrap();
    }

    fn load_user(facade: &EntityFacade, username: &str) -> EntityValue {
        facade
            .find(USER_ACCOUNT)
            .unwrap()
            .eq("username", Value::string(username))
            .one()
            .unwrap()
            .unwrap_or_else(|| panic!("missing {username}"))
    }

    fn text(value: &EntityValue, field: &str) -> String {
        value.get(field).and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn i64_field(value: &EntityValue, field: &str) -> i64 {
        match value.get(field) {
            Some(Value::I64(number)) => *number,
            Some(Value::I32(number)) => i64::from(*number),
            _ => 0,
        }
    }

    fn stamp(millis: i64) -> Value {
        basic_convert(&Value::I64(millis), "java.sql.Timestamp").value
    }

    fn now_millis() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }
}
