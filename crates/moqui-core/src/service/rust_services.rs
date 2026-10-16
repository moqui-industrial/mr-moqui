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

//! Function registry for `type="rust"` services. Each function registers under its Rust path, down to the function
//! name (`moqui_core::service::mail::send_template`). The service element's `location` is that path.
//!
//! The built-in services are in a process-wide map. A component's tool factory registers its own on the execution
//! context factory ([`ExecutionContextFactory::register_service`]), and a service definition read in that factory's
//! context finds them there first.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;
use moqui_lib::sync::{read, write};

use crate::context::{Error, ExecutionContext, ExecutionContextFactory};
use crate::entity::EntityFacade;

type RustServiceBody =
    dyn Fn(&BTreeMap<String, Value>, &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> + Send + Sync;

/// A Rust service body. The service name stays on the definition; the function is found by its `location`.
#[derive(Clone)]
pub struct RustServiceFn(Arc<RustServiceBody>);

impl RustServiceFn {
    pub fn new(
        run: impl Fn(&BTreeMap<String, Value>, &EntityFacade) -> Result<BTreeMap<String, Value>, LibError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self(Arc::new(run))
    }

    pub(crate) fn run(
        &self,
        parameters: &BTreeMap<String, Value>,
        facade: &EntityFacade,
    ) -> Result<BTreeMap<String, Value>, LibError> {
        (self.0)(parameters, facade)
    }
}

impl std::fmt::Debug for RustServiceFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RustServiceFn")
    }
}

/// Register `$function` under `module_path!() + "::" + its name`. A second insert of that path is an error.
macro_rules! rust_service {
    ($map:expr, $function:ident) => {{
        let location: &'static str = concat!(module_path!(), "::", stringify!($function));
        debug_assert_eq!(location, ::std::any::type_name_of_val(&$function));
        if $map
            .insert(location, $crate::service::RustServiceFn::new($function))
            .is_some()
        {
            Err(format!("duplicate rust service {location}"))
        } else {
            Ok(())
        }
    }};
}

/// Register a `type="rust"` service function on an execution context factory under its Rust path,
/// `module_path!() + "::" + its name`, the `location` the service definition names. For a tool factory's
/// [`ToolFactory::pre_facade_init`](crate::context::ToolFactory::pre_facade_init):
///
/// ```ignore
/// moqui_core::register_service!(ecf, echo)?;
/// ```
#[macro_export]
macro_rules! register_service {
    ($ecf:expr, $function:ident) => {
        $ecf.register_service(concat!(module_path!(), "::", stringify!($function)), $function)
    };
}

static REGISTRY: OnceLock<BTreeMap<&'static str, RustServiceFn>> = OnceLock::new();

/// The services one execution context factory's tool factories registered.
#[derive(Default)]
struct Registered(RwLock<BTreeMap<String, RustServiceFn>>);

/// The function registered at `location`: the entered context's factory registrations first, then the built-ins. With no
/// entered context (`moqui lint` without a factory) only the built-ins.
pub(crate) fn get(location: &str) -> Option<RustServiceFn> {
    let registered = ExecutionContext::with_current(|ec| {
        read(&ec.factory().extension(|_| Registered::default()).0)
            .get(location)
            .cloned()
    })
    .flatten();
    registered.or_else(|| registry().get(location).cloned())
}

/// [`ExecutionContextFactory::register_service`].
pub(crate) fn register(factory: &ExecutionContextFactory, location: &str, run: RustServiceFn) -> Result<(), Error> {
    if location.trim().is_empty() {
        return Err(Error::new("a rust service location is empty"));
    }
    if registry().contains_key(location) {
        return Err(Error::new(format!(
            "rust service {location} is already registered as a built-in service"
        )));
    }
    if super::define::definitions_read(factory) {
        return Err(Error::new(format!(
            "rust service {location} is registered after the service definitions were read; register it in pre_facade_init"
        )));
    }
    let registered = factory.extension(|_| Registered::default());
    let mut map = write(&registered.0);
    if map.contains_key(location) {
        return Err(Error::new(format!("rust service {location} is already registered")));
    }
    map.insert(location.to_string(), run);
    Ok(())
}

fn registry() -> &'static BTreeMap<&'static str, RustServiceFn> {
    REGISTRY.get_or_init(build)
}

fn build() -> BTreeMap<&'static str, RustServiceFn> {
    let mut map = BTreeMap::new();
    let mut errors = Vec::new();
    for result in [
        super::mail::register(&mut map),
        super::system_sync::register(&mut map),
        super::elfinder::register(&mut map),
        super::sso::register(&mut map),
        super::tools::register(&mut map),
        super::images::register(&mut map),
        super::spreadsheet::register(&mut map),
    ] {
        if let Err(error) = result {
            errors.push(error);
        }
    }
    assert!(errors.is_empty(), "rust service registry: {}", errors.join("; "));
    map
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_keys_are_the_function_paths() {
        let map = super::build();
        let mut keys: Vec<&str> = map.keys().copied().collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "moqui_core::service::elfinder::run_command",
                "moqui_core::service::images::convert_image",
                "moqui_core::service::images::image_reader_file_suffixes",
                "moqui_core::service::images::image_reader_mime_types",
                "moqui_core::service::images::image_writer_file_suffixes",
                "moqui_core::service::images::image_writer_mime_types",
                "moqui_core::service::images::write_image_service",
                "moqui_core::service::mail::poll_server",
                "moqui_core::service::mail::send_message",
                "moqui_core::service::mail::send_template",
                "moqui_core::service::spreadsheet::load_spreadsheet_entity",
                "moqui_core::service::spreadsheet::read_spreadsheet",
                "moqui_core::service::sso::handle_callback",
                "moqui_core::service::sso::login_user",
                "moqui_core::service::sso::logout_user",
                "moqui_core::service::system_sync::entity_sync::get_data",
                "moqui_core::service::system_sync::entity_sync::include_list",
                "moqui_core::service::system_sync::entity_sync::internal_run",
                "moqui_core::service::system_sync::entity_sync::put_data",
                "moqui_core::service::system_sync::entity_sync::run_sync",
                "moqui_core::service::system_sync::message::send_rest",
                "moqui_core::service::system_sync::sftp::poll",
                "moqui_core::service::system_sync::sftp::send",
                "moqui_core::service::tools::cancel_print_job",
                "moqui_core::service::tools::get_print_job_details",
                "moqui_core::service::tools::get_server_printers",
                "moqui_core::service::tools::hold_print_job",
                "moqui_core::service::tools::print_document",
                "moqui_core::service::tools::print_resource_document",
                "moqui_core::service::tools::print_screen_document",
                "moqui_core::service::tools::release_print_job",
                "moqui_core::service::tools::rewrite_encrypted_fields",
                "moqui_core::service::tools::send_document_notifications",
                "moqui_core::service::tools::send_print_job_internal",
            ]
        );
    }
}
