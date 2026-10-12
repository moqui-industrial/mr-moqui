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

//! User facade. Login, passwords, and authorization are no-ops until phase 5.

mod authz;
mod filter;
mod login;
mod password;
mod policy;
mod session;
mod visit;

pub use authz::{authorize, context_entered};
pub use filter::apply_entity_filters;
pub use login::login_user;
pub use password::hash_password;
pub use policy::password_acceptable;
pub use session::current_username;
pub use visit::record_visit;
