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

//! Service worker pool. The factory stores this guard. No threads yet.

/// Factory-owned pool guard.
///
/// The later worker implementation's Drop stops and joins its own workers when the factory value
/// drops. [`ServicePools::idle`] stays thread-free.
pub struct ServicePools {
    _private: (),
}

impl ServicePools {
    /// No workers. test_context stores this. Dropping it must not stop another factory's pool.
    pub fn idle() -> Self {
        Self { _private: () }
    }
}

/// Read tools.@worker-pool-max and tools.@worker-queue from conf_text later.
/// This stub ignores the text and returns an idle guard. The factory stores it.
pub fn start_pools(conf_text: &str) -> Result<ServicePools, crate::context::Error> {
    let _ = conf_text;
    Ok(ServicePools::idle())
}
