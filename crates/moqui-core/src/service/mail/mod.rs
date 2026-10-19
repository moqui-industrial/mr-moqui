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

//! Outbound SMTP and inbound IMAP/POP3 for the email services. The bodies live in [`implementation`] when the `mail`
//! feature is on, with the mail client's sync engine beside it: [`sync`] (`sync#EmailAccount` and the job), [`remote`]
//! (local changes onto the server), [`idle`] (IMAP IDLE workers), [`store`] (a received message as a row),
//! [`discover`] (autodiscover), [`import`] (.eml and .mbox), [`oauth`] (sign-in with Google or Microsoft, the token
//! store and XOAUTH2) and [`inbound`] (mail the host's SMTP listener received for hosted and shared accounts). These
//! functions are the registry entries either way, so a host built without mail still resolves the locations and reports
//! that mail is not built in.

use std::collections::BTreeMap;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use crate::entity::EntityFacade;
use crate::service::rust_services::RustServiceFn;

mod contacts;
#[cfg(feature = "mail")]
mod discover;
#[cfg(feature = "mail")]
mod export;
#[cfg(feature = "mail")]
mod idle;
#[cfg(feature = "mail")]
mod implementation;
#[cfg(feature = "mail")]
mod import;
#[cfg(feature = "mail")]
pub mod inbound;
#[cfg(feature = "mail")]
mod oauth;
#[cfg(feature = "mail")]
mod remote;
#[cfg(feature = "mail")]
mod store;
#[cfg(feature = "mail")]
mod sync;

#[cfg(all(test, feature = "mail"))]
pub(crate) use implementation::{SharedSinkGuard, SinkGuard};

#[cfg(all(test, feature = "mail"))]
mod client_tests;
#[cfg(all(test, feature = "mail"))]
mod fake_servers;
#[cfg(all(test, feature = "mail"))]
mod inbound_tests;
#[cfg(all(test, feature = "mail"))]
mod oauth_tests;
#[cfg(all(test, feature = "mail"))]
mod settings_tests;
#[cfg(all(test, feature = "mail"))]
mod sync_tests;

pub(in crate::service) fn register(map: &mut BTreeMap<&'static str, RustServiceFn>) -> Result<(), String> {
    rust_service!(map, send_template)?;
    rust_service!(map, send_message)?;
    rust_service!(map, poll_server)?;
    rust_service!(map, test_server)?;
    rust_service!(map, sync_account)?;
    rust_service!(map, sync_due_accounts)?;
    rust_service!(map, push_remote_changes)?;
    rust_service!(map, discover_server)?;
    rust_service!(map, import_messages)?;
    rust_service!(map, export_message)?;
    rust_service!(map, start_oauth_sign_in)?;
    rust_service!(map, complete_oauth_sign_in)?;
    rust_service!(map, get_oauth_status)?;
    rust_service!(map, get_oauth_providers)?;
    rust_service!(map, find_oauth_provider)?;
    rust_service!(map, revoke_oauth_sign_in)?;
    rust_service!(map, revoke_oauth_token)?;
    rust_service!(map, filter_contact_addresses)?;
    Ok(())
}

/// `filter#ContactAddresses`, with or without mail built in: the app's hook says which senders are a user's contacts
/// ([`contacts`]).
fn filter_contact_addresses(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    contacts::filter_contact_addresses(parameters, facade)
}

/// Start the IMAP IDLE workers on the scheduler's tick (`email_idle_enabled`). The host calls it once after boot.
#[cfg(feature = "mail")]
pub(crate) fn start_idle(factory: &std::sync::Arc<crate::context::ExecutionContextFactory>) {
    idle::start(factory);
}

#[cfg(not(feature = "mail"))]
pub(crate) fn start_idle(_factory: &std::sync::Arc<crate::context::ExecutionContextFactory>) {}

/// Stop the IMAP IDLE workers; the factory's shutdown calls it.
#[cfg(feature = "mail")]
pub(crate) fn stop_idle(factory: &crate::context::ExecutionContextFactory) {
    idle::stop_all(factory);
}

#[cfg(not(feature = "mail"))]
pub(crate) fn stop_idle(_factory: &crate::context::ExecutionContextFactory) {}

/// A mail service body: the implementation with mail built in, else an error that says it is not.
macro_rules! mail_service {
    ($name:ident, $service:literal, $($body:tt)+) => {
        #[cfg(feature = "mail")]
        fn $name(
            parameters: &BTreeMap<String, Value>,
            facade: &EntityFacade,
        ) -> Result<BTreeMap<String, Value>, LibError> {
            $($body)+(parameters, facade)
        }

        #[cfg(not(feature = "mail"))]
        fn $name(
            _parameters: &BTreeMap<String, Value>,
            _facade: &EntityFacade,
        ) -> Result<BTreeMap<String, Value>, LibError> {
            unavailable($service)
        }
    };
}

mail_service!(
    send_template,
    "org.moqui.impl.EmailServices.send#EmailTemplate",
    implementation::send_template
);
mail_service!(
    send_message,
    "org.moqui.impl.EmailServices.send#EmailMessage",
    implementation::send_message
);
mail_service!(
    poll_server,
    "org.moqui.impl.EmailServices.poll#EmailServer",
    implementation::poll_server
);
mail_service!(
    test_server,
    "org.moqui.impl.EmailServices.test#EmailServer",
    implementation::test_server
);
mail_service!(
    sync_account,
    "org.moqui.impl.EmailClientServices.sync#EmailAccount",
    sync::sync_account_service
);
mail_service!(
    sync_due_accounts,
    "org.moqui.impl.EmailClientServices.sync#DueEmailAccounts",
    sync::sync_due_accounts
);
mail_service!(
    push_remote_changes,
    "org.moqui.impl.EmailClientServices.push#RemoteChanges",
    remote::push_changes
);
mail_service!(
    discover_server,
    "org.moqui.impl.EmailClientServices.discover#EmailServer",
    discover::discover_server
);
mail_service!(
    import_messages,
    "org.moqui.impl.EmailClientServices.import#EmailMessages",
    import::import_messages
);
mail_service!(
    export_message,
    "org.moqui.impl.EmailClientServices.export#EmailMessage",
    export::export_message
);
mail_service!(
    start_oauth_sign_in,
    "org.moqui.impl.EmailClientServices.start#OAuthSignIn",
    oauth::start_sign_in
);
mail_service!(
    complete_oauth_sign_in,
    "org.moqui.impl.EmailClientServices.complete#OAuthSignIn",
    oauth::complete_sign_in
);
mail_service!(
    get_oauth_status,
    "org.moqui.impl.EmailClientServices.get#OAuthStatus",
    oauth::sign_in_status
);
mail_service!(
    get_oauth_providers,
    "org.moqui.impl.EmailClientServices.get#OAuthProviders",
    oauth::providers_service
);
mail_service!(
    find_oauth_provider,
    "org.moqui.impl.EmailClientServices.find#OAuthProvider",
    oauth::find_provider_service
);
mail_service!(
    revoke_oauth_sign_in,
    "org.moqui.impl.EmailClientServices.revoke#OAuthSignIn",
    oauth::sign_out_service
);
mail_service!(
    revoke_oauth_token,
    "org.moqui.impl.EmailClientServices.revoke#OAuthToken",
    oauth::revoke_token_service
);

#[cfg(not(feature = "mail"))]
fn unavailable(name: &str) -> Result<BTreeMap<String, Value>, LibError> {
    Err(LibError::new(format!(
        "service {name}: mail is not built into this host"
    )))
}

#[cfg(test)]
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
            (std::any::type_name_of_val(&super::export_message), "export_message"),
            (
                std::any::type_name_of_val(&super::start_oauth_sign_in),
                "start_oauth_sign_in",
            ),
            (
                std::any::type_name_of_val(&super::complete_oauth_sign_in),
                "complete_oauth_sign_in",
            ),
            (std::any::type_name_of_val(&super::get_oauth_status), "get_oauth_status"),
            (
                std::any::type_name_of_val(&super::get_oauth_providers),
                "get_oauth_providers",
            ),
            (
                std::any::type_name_of_val(&super::find_oauth_provider),
                "find_oauth_provider",
            ),
            (
                std::any::type_name_of_val(&super::revoke_oauth_sign_in),
                "revoke_oauth_sign_in",
            ),
            (
                std::any::type_name_of_val(&super::revoke_oauth_token),
                "revoke_oauth_token",
            ),
            (
                std::any::type_name_of_val(&super::filter_contact_addresses),
                "filter_contact_addresses",
            ),
        ] {
            assert_eq!(function, format!("moqui_core::service::mail::{path}"));
        }
    }
}
