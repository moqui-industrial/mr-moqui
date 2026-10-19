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

//! The `xlsx` render mode's workbook in a build without the `xlsx` feature: the same calls as [`super::xlsx`] when
//! it is built in, keeping only the counts the render asks for, and [`XlsxBook::to_bytes`] reports that xlsx is not
//! built into this host.

use moqui_lib::object::Value;

use crate::context::Error;

/// How a number cell is shown: the field's `align` and the widget's currency or `.00` format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellHints {
    pub currency: bool,
    pub align_right: bool,
}

/// The workbook a render fills, as counts only.
#[derive(Debug, Default)]
pub struct XlsxBook {
    sheets: usize,
    row_cells: usize,
}

impl XlsxBook {
    pub fn start_sheet(&mut self, _name: &str) {
        self.sheets += 1;
        self.row_cells = 0;
    }

    pub fn header(&mut self, _title: &str) {}

    /// # Errors
    ///
    /// Never; the signature is the built-in one's.
    pub fn cell(&mut self, _values: &[(Value, String)], _hints: CellHints) -> Result<(), Error> {
        if self.sheets > 0 {
            self.row_cells += 1;
        }
        Ok(())
    }

    pub fn end_row(&mut self) {
        self.row_cells = 0;
    }

    pub fn cell_count(&self) -> usize {
        self.row_cells
    }

    pub fn is_empty(&self) -> bool {
        self.sheets == 0
    }

    /// # Errors
    ///
    /// Always: this build has no xlsx writer.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        Err(Error::new("xlsx is not built into this host"))
    }
}
