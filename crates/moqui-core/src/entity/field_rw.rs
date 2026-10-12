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

//! Field read and write hooks. Later: encryption, audit log, and localization.

use std::collections::BTreeMap;

use moqui_db::model::EntityDefinition;
use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::facade::EntityFacade;

/// Prepare field values before SQL. This spine returns them unchanged.
pub fn before_write(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let _ = (facade, entity);
    Ok(values)
}

/// Adjust a row after SQL. This spine returns it unchanged.
pub fn after_read(
    facade: &EntityFacade,
    entity: &EntityDefinition,
    values: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, Error> {
    let _ = (facade, entity);
    Ok(values)
}
