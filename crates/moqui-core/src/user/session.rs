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

//! Server session. The fields are phase 5.

use crate::context::Error;

pub fn current_username() -> Result<Option<String>, Error> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn current_username_is_a_stub() {
        assert!(super::current_username().unwrap().is_none());
    }
}
