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

//! User facade: login, passwords, the in-memory session, and artifact authorization.

mod authz;
mod filter;
mod login;
mod password;
mod policy;
mod session;
mod visit;

pub use authz::{authorize, context_entered};
pub(crate) use authz::{denied, is_permitted, note_entity_write, AuthInfo, Permit};
pub use filter::apply_entity_filters;
pub use login::{change_password, internal_login_user, login_anonymous, login_user, logout, logout_anonymous_only};
pub use password::hash_password;
pub use policy::password_acceptable;
pub(crate) use session::Memory;
pub use session::{
    csrf_token, current_user_id, current_username, error_parameters, flash_messages, is_anonymous, locale,
    saved_parameters, screen_history, time_zone, visit_id, visitor_id,
};
pub use visit::record_visit;
