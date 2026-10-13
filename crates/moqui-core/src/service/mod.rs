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
mod define;
mod eca;
mod eca_rules;
mod email_eca;
mod jobs;
mod mail;
mod pool;
mod remote;
mod request_pool;
mod rest;
mod rule_actions;
mod schedule;
mod seca;
mod security_checks;
pub(crate) mod semaphore;
mod validate;

pub use call::{async_call, call, call_future, start_future, sync, ServiceFuture};
pub use define::load_definitions;
pub use define::{check, lint};
pub use eca::{run_held_service, run_matched_eca, schedule_feed_receive, search_not_up};
pub use email_eca::run_email_rules;
pub(crate) use jobs::send_job_error_notification;
pub use jobs::{run_job_now, PassReport};
pub use pool::{start_pools, ServicePools};
pub(crate) use remote::parse_json;
pub use remote::shared_http_client;
pub use request_pool::{caps, check_serve_caps, RequestPool};
#[cfg(test)]
pub(crate) use rest::install;
pub(crate) use rest::openapi_document;
pub use rest::run;
pub use seca::run_seca_rules;
pub use security_checks::security_checks_run;
pub use semaphore::acquire;
pub use validate::validate;
