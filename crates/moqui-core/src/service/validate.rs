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

//! Parameter validation. The checks are phase 6.

use crate::context::Error;

pub fn validate(name: &str) -> Result<(), Error> {
    let _ = name;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn validate_is_a_stub() {
        super::validate("create#Example").unwrap();
    }
}
