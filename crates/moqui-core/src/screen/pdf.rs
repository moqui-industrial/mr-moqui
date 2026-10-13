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

//! PDF output for the `html-print` mode. The renderer arrives with phase 12 (print and PDF through a
//! headless browser). Until then a request for PDF bytes reports that it is not available, and the
//! `html-print` document (the older name `xsl-fo`) is what the render gives.

use crate::context::Error;

/// What a caller sees when it asks for PDF bytes.
pub const PDF_UNAVAILABLE: &str = "PDF is not available until phase 12";

/// The PDF renderer. This is the stub.
pub struct PdfRenderer;

impl PdfRenderer {
    /// PDF bytes for an `html-print` document.
    ///
    /// # Errors
    ///
    /// Always returns [`PDF_UNAVAILABLE`] until phase 12.
    pub fn render(_html_print_document: &str) -> Result<Vec<u8>, Error> {
        Err(Error::new(PDF_UNAVAILABLE))
    }
}
