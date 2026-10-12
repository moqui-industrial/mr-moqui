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

//! Entity ECA, data-file services, and feed receive. None of these call a service yet.

use std::collections::BTreeMap;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use crate::entity::load_feed::HeldService;
use crate::entity::EntityFacade;

/// Run matched ECA rules. `before` selects run-before; otherwise run-after. This calls nothing.
pub fn run_matched_eca(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    before: bool,
    values: &BTreeMap<String, Value>,
) -> Result<(), Error> {
    let _ = (facade, entity_name, operation, before, values);
    Ok(())
}

/// Run one data-file service element. This calls nothing.
pub fn run_held_service(facade: &EntityFacade, service: &HeldService) -> Result<(), Error> {
    let _ = (facade, service);
    Ok(())
}

/// Schedule feed receive after commit. This calls nothing and does not use a connection.
pub fn schedule_feed_receive(facade: &EntityFacade) -> Result<(), Error> {
    let _ = facade;
    Ok(())
}
