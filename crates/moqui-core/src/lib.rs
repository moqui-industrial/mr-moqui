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

//! Framework. Synchronous and multi-threaded. No listen socket and no database driver.

pub mod context;
pub mod entity;

/// Marker so the host links this crate.
pub fn candidate() -> &'static str {
    let _ = moqui_db::candidate();
    moqui_lib::candidate()
}
