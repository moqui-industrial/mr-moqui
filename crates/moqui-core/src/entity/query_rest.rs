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

//! Dynamic view, data writer, and `EntityFacade.rest`. Not implemented in this spine.

use std::collections::BTreeMap;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::facade::EntityFacade;

/// Run a dynamic view. Not implemented.
pub fn dynamic_view(
    facade: &EntityFacade,
    entity_name: &str,
    parameters: &BTreeMap<String, Value>,
) -> Result<Vec<BTreeMap<String, Value>>, Error> {
    let _ = (facade, entity_name, parameters);
    Err(Error::new("dynamic view is not implemented"))
}

/// Write entity data records. Not implemented.
pub fn write_data(facade: &EntityFacade, entity_name: &str, records: &[BTreeMap<String, Value>]) -> Result<(), Error> {
    let _ = (facade, entity_name, records);
    Err(Error::new("data writer is not implemented"))
}

/// `EntityFacade.rest`. Not implemented.
pub fn rest(
    facade: &EntityFacade,
    operation: &str,
    entity_name: &str,
    parameters: &BTreeMap<String, Value>,
) -> Result<Value, Error> {
    let _ = (facade, operation, entity_name, parameters);
    Err(Error::new("entity rest is not implemented"))
}
