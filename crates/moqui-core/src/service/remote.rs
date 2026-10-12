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

//! Remote runners. HTTP resources are not available in this slice.

use crate::context::Error;

pub fn http_resource_error(location: &str) -> Error {
    let _ = location;
    Error::new("HTTP resources are not available")
}

#[cfg(test)]
mod tests {
    #[test]
    fn http_resource_error_names_http_and_not_available() {
        let err = super::http_resource_error("https://example.com/");
        let shown = err.to_string();
        assert!(shown.contains("HTTP"), "{shown}");
        assert!(shown.contains("not available"), "{shown}");
    }
}
