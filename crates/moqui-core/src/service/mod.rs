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

//! Service facade: definitions, synchronous calls, ECAs, pools, jobs, and REST. JSON-RPC is not part of this port.

pub(crate) mod call;
mod callback;
mod define;
mod email_eca;
mod pool;
mod remote;
mod request_pool;
mod rest;
mod schedule;
mod seca;
mod semaphore;
mod validate;

pub use call::call;
pub use callback::{run_held_service, run_matched_eca, schedule_feed_receive, security_checks_run};
pub use define::lint;
pub use define::load_definitions;
pub use email_eca::run_email_rules;
pub use pool::{start_pools, ServicePools};
pub use remote::http_resource_error;
pub(crate) use remote::parse_json;
pub use request_pool::{caps, check_serve_caps, RequestPool};
pub(crate) use rest::openapi_document;
pub use rest::run;
#[cfg(test)]
pub(crate) use rest::{clear_files, load_file};
pub use schedule::schedule;
pub use seca::run_seca_rules;
pub use semaphore::acquire;
pub use validate::validate;
