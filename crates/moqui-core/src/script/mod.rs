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

//! Script facade. Expressions and XML actions are no-ops until phase 6.

mod actions;
mod parse;
mod runtime;

pub use actions::run_actions;
pub use parse::parse_js;
pub use runtime::eval_condition;

#[cfg(test)]
mod tests {
    use super::{parse_js, run_actions};

    #[test]
    fn parse_js_accepts_arithmetic_and_actions_return_ok() {
        parse_js("1 + 1").unwrap();
        run_actions("<actions/>").unwrap();
    }
}
