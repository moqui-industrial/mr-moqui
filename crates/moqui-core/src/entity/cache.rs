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

use std::collections::BTreeMap;

use moqui_db::sql::{Condition, OrderBy};
use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::facade::EntityFacade;
use super::value::EntityList;
use crate::transaction::TransactionManager;

/// The find `list` and `iterator` ask the cache about. Owned so a cache can store the key.
pub struct FindLookup {
    pub entity_name: String,
    pub condition: Option<Condition>,
    pub order: Vec<OrderBy>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Whether the caller should run the write on the enlisted connection.
///
/// `Handled` means this module stored the write or ran it itself. Return that only when the write
/// will not be lost: a transaction cache inside an open frame, or a sim write already sent to the
/// overlay. `on_write` still runs after either variant and is the one-cache invalidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteGate {
    Proceed,
    Handled,
}

/// Look up a cached list. This spine always misses.
pub fn before_find(facade: &EntityFacade, lookup: &FindLookup) -> Option<EntityList> {
    let _ = (
        facade,
        &lookup.entity_name,
        &lookup.condition,
        &lookup.order,
        lookup.limit,
        lookup.offset,
    );
    None
}

/// Look up a cached count. This spine always misses. The value is the count, not a list length.
pub fn before_count(facade: &EntityFacade, entity_name: &str, condition: Option<&Condition>) -> Option<i64> {
    let _ = (facade, entity_name, condition);
    None
}

/// Store a find result. Rows are already decrypted and localized.
pub fn after_find(facade: &EntityFacade, lookup: &FindLookup, rows: &EntityList) -> Result<(), Error> {
    let _ = (
        facade,
        &lookup.entity_name,
        &lookup.condition,
        &lookup.order,
        lookup.limit,
        lookup.offset,
        rows,
    );
    Ok(())
}

/// Decide whether SQL on the enlisted connection should run. Values are normalized and not yet encrypted.
pub fn gate_write(
    facade: &EntityFacade,
    entity_name: &str,
    operation: &str,
    values: &BTreeMap<String, Value>,
) -> Result<WriteGate, Error> {
    let _ = (facade, entity_name, operation, values);
    Ok(WriteGate::Proceed)
}

/// Note a create, update, or delete. This spine does not invalidate a cache.
pub fn on_write(facade: &EntityFacade, entity_name: &str, operation: &str) -> Result<(), Error> {
    let _ = (facade, entity_name, operation);
    Ok(())
}

/// Register a no-op `on_cache_flush` so the transaction slot exists.
pub fn register_cache_flush(facade: &EntityFacade, tx: &TransactionManager) -> Result<(), Error> {
    let _ = facade;
    tx.on_cache_flush(|_| Ok(()))
}

#[cfg(test)]
mod tests {
    use super::WriteGate;

    #[test]
    fn handled_is_a_write_gate() {
        assert_eq!(WriteGate::Handled, WriteGate::Handled);
        assert_ne!(WriteGate::Proceed, WriteGate::Handled);
    }
}
