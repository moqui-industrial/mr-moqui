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

//! In-memory session state, owned by the [`ExecutionContext`] it belongs to.
//!
//! There is no database row and no cookie here. The web layer loads this state into a new
//! context and stores it back, so one request never sees another request's user. With no
//! context entered, reads see an empty session and writes are dropped.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::authz::AuthzCheck;
use crate::context::{Error, ExecutionContext};

/// What the artifact authorization checks cached for the logged-in user, and when.
#[derive(Clone, Debug, Default)]
pub(crate) struct AuthzCache {
    pub(crate) generation: u64,
    pub(crate) groups: Option<Arc<std::collections::HashSet<String>>>,
    pub(crate) checks: Option<Arc<Vec<Arc<AuthzCheck>>>>,
    pub(crate) tarpit: BTreeMap<String, Arc<Vec<TarpitCheck>>>,
}

/// One tarpit rule for an artifact type, for the user's groups.
#[derive(Clone, Debug)]
pub(crate) struct TarpitCheck {
    pub(crate) artifact_name: String,
    pub(crate) name_is_pattern: bool,
    pub(crate) max_hits_count: i64,
    pub(crate) max_hits_duration: i64,
    pub(crate) tarpit_duration: i64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Memory {
    pub(crate) authz: AuthzCache,
    user_id: Option<String>,
    username: Option<String>,
    visit_id: Option<String>,
    visitor_id: Option<String>,
    locale: Option<String>,
    time_zone: Option<String>,
    csrf_token: Option<String>,
    anonymous: bool,
    flash: Vec<String>,
    saved: BTreeMap<String, String>,
    errors: BTreeMap<String, String>,
    screen_history: Vec<String>,
}

/// The logged-in username. `Ok(None)` when nobody is logged in.
pub fn current_username() -> Result<Option<String>, Error> {
    Ok(with_session(|session| session.username.clone()))
}

pub fn current_user_id() -> Result<Option<String>, Error> {
    Ok(with_session(|session| session.user_id.clone()))
}

pub fn is_anonymous() -> Result<bool, Error> {
    Ok(with_session(|session| session.anonymous))
}

/// CSRF token for this thread. Generated from `getrandom` the first time it is read.
pub fn csrf_token() -> Result<String, Error> {
    let existing = with_session(|session| session.csrf_token.clone());
    if let Some(token) = existing {
        return Ok(token);
    }
    let token = random_hex(32)?;
    with_session(|session| {
        if session.csrf_token.is_none() {
            session.csrf_token = Some(token.clone());
        }
    });
    Ok(with_session(|session| session.csrf_token.clone().unwrap_or(token)))
}

pub fn visit_id() -> Result<Option<String>, Error> {
    Ok(with_session(|session| session.visit_id.clone()))
}

pub fn visitor_id() -> Result<Option<String>, Error> {
    Ok(with_session(|session| session.visitor_id.clone()))
}

pub fn locale() -> Result<Option<String>, Error> {
    Ok(with_session(|session| session.locale.clone()))
}

pub fn time_zone() -> Result<Option<String>, Error> {
    Ok(with_session(|session| session.time_zone.clone()))
}

pub fn flash_messages() -> Result<Vec<String>, Error> {
    Ok(with_session(|session| session.flash.clone()))
}

pub fn saved_parameters() -> Result<BTreeMap<String, String>, Error> {
    Ok(with_session(|session| session.saved.clone()))
}

pub fn error_parameters() -> Result<BTreeMap<String, String>, Error> {
    Ok(with_session(|session| session.errors.clone()))
}

pub fn screen_history() -> Result<Vec<String>, Error> {
    Ok(with_session(|session| session.screen_history.clone()))
}

pub fn set_locale(locale: Option<&str>) {
    with_session(|session| session.locale = locale.map(str::to_string));
}

pub fn set_time_zone(time_zone: Option<&str>) {
    with_session(|session| session.time_zone = time_zone.map(str::to_string));
}

#[allow(dead_code)]
pub fn add_flash_message(message: &str) {
    with_session(|session| session.flash.push(message.to_string()));
}

#[allow(dead_code)]
pub fn save_parameter(name: &str, value: &str) {
    with_session(|session| {
        session.saved.insert(name.to_string(), value.to_string());
    });
}

#[allow(dead_code)]
pub fn set_error_parameter(name: &str, value: &str) {
    with_session(|session| {
        session.errors.insert(name.to_string(), value.to_string());
    });
}

#[allow(dead_code)]
pub fn push_screen_history(screen: &str) {
    with_session(|session| session.screen_history.push(screen.to_string()));
}

/// A new login starts a new session: the CSRF token and cached authorization data are dropped.
pub(crate) fn set_user(user_id: &str, username: &str) {
    with_session(|session| {
        session.user_id = Some(user_id.to_string());
        session.username = Some(username.to_string());
        session.anonymous = false;
        session.csrf_token = None;
        session.authz = AuthzCache::default();
    });
}

pub(crate) fn set_anonymous(anonymous: bool) {
    with_session(|session| session.anonymous = anonymous);
}

pub(crate) fn set_visit(visit_id: &str, visitor_id: &str) {
    with_session(|session| {
        session.visit_id = Some(visit_id.to_string());
        session.visitor_id = Some(visitor_id.to_string());
    });
}

/// Clears the user and the anonymous flag. Visit ids and screen state stay. The CSRF token and
/// cached authorization data go, so a later login cannot reuse them.
pub(crate) fn clear_login() {
    with_session(|session| {
        session.user_id = None;
        session.username = None;
        session.anonymous = false;
        session.csrf_token = None;
        session.authz = AuthzCache::default();
    });
}

#[allow(dead_code)]
pub(crate) fn reset_session() {
    with_session(|session| *session = Memory::default());
}

pub(crate) fn with_session<T>(body: impl FnOnce(&mut Memory) -> T) -> T {
    let mut pending = Some(body);
    let ran = ExecutionContext::with_current(|ec| pending.take().map(|run| run(&mut ec.session())));
    match ran.flatten() {
        Some(value) => value,
        None => match pending.take() {
            Some(run) => run(&mut Memory::default()),
            None => unreachable!("the session body runs once"),
        },
    }
}

fn random_hex(size: usize) -> Result<String, Error> {
    let mut bytes = vec![0_u8; size];
    getrandom::fill(&mut bytes).map_err(|err| Error::new(format!("could not read random bytes: {err}")))?;
    Ok(encode_hex(&bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionContextFactory;

    #[test]
    fn a_session_belongs_to_its_context_and_never_leaks_to_the_next() {
        let first = ExecutionContextFactory::test_context();
        {
            let _entered = first.enter();
            set_user("U1", "ada");
            assert_eq!(current_username().unwrap().as_deref(), Some("ada"));
        }
        // No context entered: nobody is logged in.
        assert!(current_username().unwrap().is_none());
        let second = ExecutionContextFactory::test_context();
        let _entered = second.enter();
        assert!(
            current_username().unwrap().is_none(),
            "a new context starts with no user"
        );
    }

    #[test]
    fn login_and_logout_rotate_the_csrf_token() {
        let ctx = ExecutionContextFactory::test_context();
        let _entered = ctx.enter();
        let anonymous = csrf_token().unwrap();
        set_user("U1", "ada");
        let after_login = csrf_token().unwrap();
        assert_ne!(anonymous, after_login);
        clear_login();
        assert_ne!(after_login, csrf_token().unwrap());
    }

    #[test]
    fn current_username_is_none_until_a_user_is_set() {
        let ctx = ExecutionContextFactory::test_context();
        let _entered = ctx.enter();
        reset_session();
        assert!(current_username().unwrap().is_none());
        assert!(current_user_id().unwrap().is_none());
        assert!(!is_anonymous().unwrap());
        set_user("U1", "ada");
        set_locale(Some("en"));
        set_time_zone(Some("UTC"));
        set_visit("100000", "visitor-1");
        add_flash_message("saved-flash");
        save_parameter("q", "1");
        set_error_parameter("password", "required");
        push_screen_history("Login");
        assert_eq!(current_username().unwrap().as_deref(), Some("ada"));
        assert_eq!(current_user_id().unwrap().as_deref(), Some("U1"));
        assert_eq!(locale().unwrap().as_deref(), Some("en"));
        assert_eq!(time_zone().unwrap().as_deref(), Some("UTC"));
        assert_eq!(visit_id().unwrap().as_deref(), Some("100000"));
        assert_eq!(visitor_id().unwrap().as_deref(), Some("visitor-1"));
        assert_eq!(flash_messages().unwrap(), vec!["saved-flash".to_string()]);
        assert_eq!(saved_parameters().unwrap().get("q").map(String::as_str), Some("1"));
        assert_eq!(
            error_parameters().unwrap().get("password").map(String::as_str),
            Some("required")
        );
        assert_eq!(screen_history().unwrap(), vec!["Login".to_string()]);
        let token = csrf_token().unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(csrf_token().unwrap(), token);
        assert!(!is_anonymous().unwrap());
        clear_login();
        assert!(current_username().unwrap().is_none());
        assert!(!is_anonymous().unwrap());
        assert_eq!(visit_id().unwrap().as_deref(), Some("100000"));
        reset_session();
    }
}
