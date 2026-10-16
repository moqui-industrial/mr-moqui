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

//! `type="rust"` services. `register_service!` names each function by its Rust path, which is the `location` in
//! `service/example/ExampleServices.xml`: `example::services::echo`.

use std::collections::BTreeMap;

use moqui_core::context::{Error, ExecutionContextFactory};
use moqui_core::entity::EntityFacade;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

/// Register this module's services. Called from the tool factory's `pre_facade_init`, before any service definition is
/// read.
pub(crate) fn register(ecf: &ExecutionContextFactory) -> Result<(), Error> {
    moqui_core::register_service!(ecf, echo)?;
    Ok(())
}

/// `example.ExampleServices.echo#Message`: the message back, with the user the call runs as. The parameters are
/// already validated against the definition, and the call's execution context is the entered one, so the user and the
/// transaction are the caller's.
///
/// # Errors
///
/// Returns an error when the session cannot be read.
pub fn echo(parameters: &BTreeMap<String, Value>, _facade: &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> {
    let message = parameters.get("message").cloned().unwrap_or(Value::Null);
    let username = moqui_core::user::current_username()?.map_or(Value::Null, Value::String);
    let mut out = BTreeMap::new();
    out.insert("message".to_string(), message);
    out.insert("username".to_string(), username);
    Ok(out)
}
