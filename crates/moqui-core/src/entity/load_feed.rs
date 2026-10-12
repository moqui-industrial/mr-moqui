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

//! Entity ECA match and the data-feed insert hook. This spine runs no actions.

use moqui_lib::error::Error;

use super::facade::EntityFacade;
use crate::transaction::TransactionManager;

/// Match ECA rules for a write. This spine returns no rules.
///
/// Callers discard the returned names. Record matched rules on the facade from inside this function.
pub fn match_rules(facade: &EntityFacade, entity_name: &str, operation: &str) -> Vec<String> {
    let _ = (facade, entity_name, operation);
    Vec::new()
}

/// Register a no-op `on_feed_insert`. This spine does not insert `DataFeedPending`.
pub fn register_feed_insert(facade: &EntityFacade, tx: &TransactionManager) -> Result<(), Error> {
    let _ = facade;
    tx.on_feed_insert(|_| Ok(()))
}
