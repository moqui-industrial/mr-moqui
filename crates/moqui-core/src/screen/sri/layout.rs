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

//! The column widths of a form-list in a text render.

use moqui_lib::format::{Decimal, RoundingStrategy, ToPrimitive};

use crate::context::Error;

/// `FormListRenderInfo.getFormListColumnCharWidths`: the width of each column in characters. `columns`
/// holds the `print-width` and `print-width-type` of each field of each column.
///
/// # Errors
///
/// Returns an error when fixed widths exceed the line, or a width is not a number.
pub fn form_list_column_char_widths(columns: &[Vec<(String, String)>], line_width: i64) -> Result<Vec<i64>, Error> {
    let columns_count = columns.len();
    if columns_count == 0 {
        return Ok(Vec::new());
    }
    let original = if line_width == 0 { 132 } else { line_width };
    let mut line = original - (i64::try_from(columns_count).unwrap_or(0) - 1);
    let mut char_widths: Vec<Option<i64>> = vec![None; columns_count];
    let mut percent_widths: Vec<Option<Decimal>> = vec![None; columns_count];
    let mut fixed_width = 0_i64;
    let mut fixed_count = 0_i64;
    let hundred = Decimal::from(100);
    for (index, column) in columns.iter().enumerate() {
        let mut char_width = -1_i64;
        let mut percent_width: Option<Decimal> = None;
        for (print_width, kind) in column {
            if print_width.is_empty() {
                continue;
            }
            let current: Decimal = print_width
                .trim()
                .parse()
                .map_err(|_| Error::new(format!("print-width {print_width} is not a number")))?;
            if current.is_zero() {
                char_width = 0;
                line += 1;
                continue;
            }
            if kind == "characters" {
                let whole = current.trunc().to_i64().unwrap_or(0);
                if whole > char_width {
                    char_width = whole;
                }
            } else if percent_width.is_none_or(|existing| current > existing) {
                percent_width = Some(current);
            }
        }
        if char_width >= 0 {
            match percent_width {
                Some(percent) => {
                    let percent_chars = (percent / hundred * Decimal::from(line)).trunc().to_i64().unwrap_or(0);
                    if percent_chars < char_width {
                        char_widths[index] = Some(char_width);
                        fixed_width += char_width;
                        fixed_count += 1;
                    } else {
                        percent_widths[index] = Some(percent);
                    }
                }
                None => {
                    char_widths[index] = Some(char_width);
                    fixed_width += char_width;
                    fixed_count += 1;
                }
            }
        } else if let Some(percent) = percent_width {
            percent_widths[index] = Some(percent);
        }
    }
    let for_percent = line - fixed_width;
    if for_percent < 0 {
        return Err(Error::new(format!(
            "fixed width columns exceeded total line characters {original} by {} characters",
            -for_percent
        )));
    }
    let percent_cols = i64::try_from(columns_count).unwrap_or(0) - fixed_count;
    let mut percent_total = Decimal::ZERO;
    for index in 0..columns_count {
        match percent_widths[index] {
            Some(percent) => percent_total += percent,
            None => {
                if char_widths[index].is_some() {
                    continue;
                }
                let share = (Decimal::ONE / Decimal::from(percent_cols.max(1)))
                    .round_dp_with_strategy(10, RoundingStrategy::MidpointAwayFromZero)
                    * hundred;
                percent_widths[index] = Some(share);
                percent_total += share;
            }
        }
    }
    let mut used = 0_i64;
    let scale = if percent_total.is_zero() {
        Decimal::ONE
    } else {
        (hundred / percent_total).round_dp_with_strategy(10, RoundingStrategy::MidpointAwayFromZero)
    };
    for index in 0..columns_count {
        let Some(percent) = percent_widths[index] else {
            continue;
        };
        let actual = percent * scale;
        percent_widths[index] = Some(actual);
        let chars = (actual / hundred * Decimal::from(for_percent))
            .round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven)
            .to_i64()
            .unwrap_or(0);
        char_widths[index] = Some(chars);
        used += chars;
    }
    if used != for_percent && percent_cols > 0 {
        let mut remaining = for_percent - used;
        let per_column = (Decimal::from(remaining) / Decimal::from(percent_cols))
            .round_dp_with_strategy(0, RoundingStrategy::AwayFromZero)
            .to_i64()
            .unwrap_or(0);
        for index in 0..columns_count {
            if percent_widths[index].is_none() {
                continue;
            }
            let current = char_widths[index].unwrap_or(0);
            let adjust = if remaining.abs() > per_column.abs() {
                per_column
            } else {
                remaining
            };
            let new_chars = current + adjust;
            if new_chars > 0 {
                char_widths[index] = Some(new_chars);
                remaining -= adjust;
                if remaining == 0 {
                    break;
                }
            }
        }
    }
    Ok(char_widths.into_iter().map(|width| width.unwrap_or(0)).collect())
}
