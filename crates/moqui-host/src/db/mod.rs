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

//! `DbDriver` implementations. The pool stays in `moqui-core`.

pub mod embedded;
pub mod entity_db;
pub mod entity_facade;
pub mod jdbc_extract;
pub mod mysql;
pub mod postgres;
pub mod sim;
#[cfg(any(feature = "mysql", feature = "postgres"))]
mod statement_lru;

/// `remaining` in whole milliseconds for a server statement timeout. Both servers read 0 as no limit, so a remainder
/// under a millisecond is 1, and a remainder of zero, a deadline that has already passed, is an error.
#[cfg(any(feature = "mysql", feature = "postgres"))]
fn timeout_millis(remaining: std::time::Duration) -> Result<u64, moqui_lib::error::Error> {
    if remaining.is_zero() {
        return Err(moqui_lib::error::Error::new(
            "the transaction timed out before the statement timeout was set",
        ));
    }
    Ok(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX).max(1))
}
