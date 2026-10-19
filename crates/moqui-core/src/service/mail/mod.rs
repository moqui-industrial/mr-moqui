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

//! Outbound SMTP and inbound IMAP/POP3 for the email services. The send path lives in [`implementation`] (the `smtp`
//! feature), the receive path in [`inbound`] (the `imap` feature, which needs `smtp`), with the mail client's sync
//! engine beside it: [`sync`] (`sync#EmailAccount` and the job), [`remote`] (local changes onto the server), [`idle`]
//! (IMAP IDLE workers), [`store`] (a received message as a row), [`discover`] (autodiscover) and [`import`] (.eml and
//! .mbox). The functions below are the registry entries. In a build without a feature its services still load, and a
//! call reports that the feature is not built in (`rust_services::missing_feature`).

use std::collections::BTreeMap;

#[cfg(feature = "smtp")]
use moqui_lib::error::Error as LibError;
#[cfg(feature = "smtp")]
use moqui_lib::object::Value;

#[cfg(feature = "smtp")]
use crate::entity::EntityFacade;
use crate::service::rust_services::RustServiceFn;

#[cfg(feature = "imap")]
mod discover;
#[cfg(feature = "imap")]
mod idle;
#[cfg(feature = "smtp")]
mod implementation;
#[cfg(feature = "imap")]
mod import;
#[cfg(feature = "imap")]
mod inbound;
#[cfg(feature = "imap")]
mod remote;
#[cfg(feature = "imap")]
mod store;
#[cfg(feature = "imap")]
mod sync;

#[cfg(all(test, feature = "smtp"))]
pub(crate) use implementation::{SharedSinkGuard, SinkGuard};

#[cfg(all(test, feature = "imap"))]
mod client_tests;
#[cfg(all(test, feature = "imap"))]
mod fake_servers;
#[cfg(all(test, feature = "imap"))]
mod sync_tests;

#[cfg_attr(not(feature = "smtp"), allow(unused_variables))]
pub(in crate::service) fn register(map: &mut BTreeMap<&'static str, RustServiceFn>) -> Result<(), String> {
    #[cfg(feature = "smtp")]
    {
        rust_service!(map, send_template)?;
        rust_service!(map, send_message)?;
        rust_service!(map, test_server)?;
        rust_service!(map, push_remote_changes)?;
    }
    #[cfg(feature = "imap")]
    {
        rust_service!(map, poll_server)?;
        rust_service!(map, sync_account)?;
        rust_service!(map, sync_due_accounts)?;
        rust_service!(map, discover_server)?;
        rust_service!(map, import_messages)?;
    }
    Ok(())
}

/// Start the IMAP IDLE workers on the scheduler's tick (`email_idle_enabled`). The host calls it once after boot.
#[cfg(feature = "imap")]
pub(crate) fn start_idle(factory: &std::sync::Arc<crate::context::ExecutionContextFactory>) {
    idle::start(factory);
}

#[cfg(not(feature = "imap"))]
pub(crate) fn start_idle(_factory: &std::sync::Arc<crate::context::ExecutionContextFactory>) {}

/// Stop the IMAP IDLE workers; the factory's shutdown calls it.
#[cfg(feature = "imap")]
pub(crate) fn stop_idle(factory: &crate::context::ExecutionContextFactory) {
    idle::stop_all(factory);
}

#[cfg(not(feature = "imap"))]
pub(crate) fn stop_idle(_factory: &crate::context::ExecutionContextFactory) {}

/// A mail service body, registered under this module's path when `$feature` is built in.
macro_rules! mail_service {
    ($feature:literal, $name:ident, $($body:tt)+) => {
        #[cfg(feature = $feature)]
        fn $name(
            parameters: &BTreeMap<String, Value>,
            facade: &EntityFacade,
        ) -> Result<BTreeMap<String, Value>, LibError> {
            $($body)+(parameters, facade)
        }
    };
}

mail_service!("smtp", send_template, implementation::send_template);
mail_service!("smtp", send_message, implementation::send_message);
mail_service!("smtp", test_server, implementation::test_server);
mail_service!("imap", poll_server, inbound::poll_server);
mail_service!("imap", sync_account, sync::sync_account_service);
mail_service!("imap", sync_due_accounts, sync::sync_due_accounts);
mail_service!("imap", push_remote_changes, remote::push_changes);

/// `push#RemoteChanges` without `imap`: no account reaches an IMAP server from this build, so a local change of the mail
/// client has no server side to queue, as for an account with no IMAP server. The filing, draft and send services of
/// internal accounts call it, so it succeeds rather than reporting the feature missing.
#[cfg(all(feature = "smtp", not(feature = "imap")))]
fn push_remote_changes(
    _parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    Ok(BTreeMap::new())
}
mail_service!("imap", discover_server, discover::discover_server);
mail_service!("imap", import_messages, import::import_messages);

#[cfg(all(test, feature = "imap"))]
mod tests {
    #[test]
    fn mail_paths_match_the_function_types() {
        for (function, path) in [
            (std::any::type_name_of_val(&super::send_template), "send_template"),
            (std::any::type_name_of_val(&super::send_message), "send_message"),
            (std::any::type_name_of_val(&super::poll_server), "poll_server"),
            (std::any::type_name_of_val(&super::test_server), "test_server"),
            (std::any::type_name_of_val(&super::sync_account), "sync_account"),
            (
                std::any::type_name_of_val(&super::sync_due_accounts),
                "sync_due_accounts",
            ),
            (
                std::any::type_name_of_val(&super::push_remote_changes),
                "push_remote_changes",
            ),
            (std::any::type_name_of_val(&super::discover_server), "discover_server"),
            (std::any::type_name_of_val(&super::import_messages), "import_messages"),
        ] {
            assert_eq!(function, format!("moqui_core::service::mail::{path}"));
        }
    }
}
