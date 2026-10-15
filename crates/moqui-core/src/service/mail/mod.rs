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
//! feature is on. These three functions are the registry entries either way, so a host built without mail still
//! resolves the locations and reports that mail is not built in.

use std::collections::BTreeMap;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use crate::entity::EntityFacade;
use crate::service::rust_services::RustServiceFn;

#[cfg(feature = "mail")]
mod implementation;

#[cfg(all(test, feature = "mail"))]
pub(crate) use implementation::{SharedSinkGuard, SinkGuard};

pub(in crate::service) fn register(map: &mut BTreeMap<&'static str, RustServiceFn>) -> Result<(), String> {
    rust_service!(map, send_template)?;
    rust_service!(map, send_message)?;
    rust_service!(map, poll_server)?;
    Ok(())
}

#[cfg(feature = "mail")]
fn send_template(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    implementation::send_template(parameters, facade)
}

#[cfg(not(feature = "mail"))]
fn send_template(
    _parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    unavailable("org.moqui.impl.EmailServices.send#EmailTemplate")
}

#[cfg(feature = "mail")]
fn send_message(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    implementation::send_message(parameters, facade)
}

#[cfg(not(feature = "mail"))]
fn send_message(
    _parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    unavailable("org.moqui.impl.EmailServices.send#EmailMessage")
}

#[cfg(feature = "mail")]
fn poll_server(
    parameters: &BTreeMap<String, Value>,
    facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    implementation::poll_server(parameters, facade)
}

#[cfg(not(feature = "mail"))]
fn poll_server(
    _parameters: &BTreeMap<String, Value>,
    _facade: &EntityFacade,
) -> Result<BTreeMap<String, Value>, LibError> {
    unavailable("org.moqui.impl.EmailServices.poll#EmailServer")
}

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
        assert_eq!(
            std::any::type_name_of_val(&super::send_template),
            "moqui_core::service::mail::send_template"
        );
        assert_eq!(
            std::any::type_name_of_val(&super::send_message),
            "moqui_core::service::mail::send_message"
        );
        assert_eq!(
            std::any::type_name_of_val(&super::poll_server),
            "moqui_core::service::mail::poll_server"
        );
    }
}
