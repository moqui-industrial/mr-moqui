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

//! Find and write cache hooks. Later: entity cache, transaction cache, and sim overlay.

use moqui_db::sql::Condition;
use moqui_lib::error::Error;

use super::facade::EntityFacade;
use super::value::EntityList;
use crate::transaction::TransactionManager;

/// Look up a cached find. This spine always misses.
pub fn before_find(facade: &EntityFacade, entity_name: &str, condition: Option<&Condition>) -> Option<EntityList> {
    let _ = (facade, entity_name, condition);
    None
}

/// Store a find result. This spine does not cache.
pub fn after_find(facade: &EntityFacade, entity_name: &str, rows: &EntityList) -> Result<(), Error> {
    let _ = (facade, entity_name, rows);
    Ok(())
}

/// Note a create, update, or delete. This spine does not invalidate a cache.
pub fn on_write(facade: &EntityFacade, entity_name: &str, operation: &str) -> Result<(), Error> {
    let _ = (facade, entity_name, operation);
    Ok(())
}

/// Register a no-op `on_cache_flush` so the transaction slot exists.
pub fn register_cache_flush(tx: &TransactionManager) -> Result<(), Error> {
    tx.on_cache_flush(|_| Ok(()))
}
