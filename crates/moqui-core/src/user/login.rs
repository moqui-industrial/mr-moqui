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

//! Login. The user facade is not installed in this slice.

use crate::context::Error;

pub fn login_user(username: &str, password: &str) -> Result<(), Error> {
    let _ = (username, password);
    Err(Error::new("the user facade is not installed"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn login_reports_that_the_user_facade_is_not_installed() {
        let err = super::login_user("john", "secret").unwrap_err();
        let shown = err.to_string();
        assert!(shown.contains("user facade"), "{shown}");
        assert!(shown.contains("not installed"), "{shown}");
    }
}
