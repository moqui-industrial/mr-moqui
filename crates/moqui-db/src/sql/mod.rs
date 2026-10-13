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

//! Condition tree and SQL text. No connection and no driver.
//!
//! Identifiers are the model field and table names. `name-replace` runs before quoting.
//! Postgres quotes with `"`. MySQL quotes with backticks. Placeholders stay `?`.

mod condition;
mod ident;
mod statement;

pub use condition::{render_condition, CompareOp, Condition, FieldRef, LimitOffset, Nulls, OrderBy};
pub use ident::{is_mysql, quote_ident};
pub use statement::{
    add_column, count, create_foreign_key, create_index, create_table, delete, insert, insert_or_update, select,
    select_view, select_view_with, select_with, update, SelectOptions,
};

#[cfg(test)]
mod tests;
