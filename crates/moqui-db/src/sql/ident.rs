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

//! Identifier quoting. `name-replace` runs before the quotes.

use crate::dialect::Dialect;

pub(crate) fn is_mysql(dialect: &Dialect) -> bool {
    starts_with_ignore(&dialect.name, "mysql")
}

pub(crate) fn is_postgres(dialect: &Dialect) -> bool {
    starts_with_ignore(&dialect.name, "postgres")
}

fn starts_with_ignore(name: &str, prefix: &str) -> bool {
    name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn apply_name_replace<'a>(dialect: &'a Dialect, name: &'a str) -> &'a str {
    for pair in &dialect.name_replaces {
        if pair.original.eq_ignore_ascii_case(name) {
            return pair.replace.as_str();
        }
    }
    name
}

/// Quote one identifier. MySQL uses backticks. Every other profile uses Postgres double quotes.
/// `name-replace` is applied first (`CONDITION` becomes `THE_CONDITION` on MySQL).
pub fn quote_ident(dialect: &Dialect, name: &str) -> String {
    let name = apply_name_replace(dialect, name);
    let quote = if is_mysql(dialect) { '`' } else { '"' };
    let mut out = String::with_capacity(name.len() + 2);
    out.push(quote);
    for ch in name.chars() {
        if ch == quote {
            out.push(quote);
        }
        out.push(ch);
    }
    out.push(quote);
    out
}

pub(crate) fn quote_list(dialect: &Dialect, names: &[impl AsRef<str>]) -> String {
    let mut out = String::new();
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&quote_ident(dialect, name.as_ref()));
    }
    out
}

pub(crate) fn placeholders(dialect: &Dialect, count: usize) -> String {
    let mut out = String::new();
    for index in 0..count {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(dialect.placeholder);
    }
    out
}

pub(crate) fn clip_name(name: &str, limit: Option<u32>) -> String {
    let Some(limit) = limit else {
        return name.to_string();
    };
    let limit = limit as usize;
    if name.len() <= limit {
        return name.to_string();
    }
    let mut end = limit;
    while end > 0 && !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}
