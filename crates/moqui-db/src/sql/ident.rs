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

pub fn is_mysql(dialect: &Dialect) -> bool {
    starts_with_ignore(&dialect.name, "mysql")
}

/// The browser build's local database (`plans/archive/Wasm.md`).
pub fn is_sqlite(dialect: &Dialect) -> bool {
    starts_with_ignore(&dialect.name, "sqlite")
}

fn starts_with_ignore(name: &str, prefix: &str) -> bool {
    name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Column replacements are ordered, case-insensitive, and may chain, as in Java FieldInfo.
/// They do not apply to table names or logical result aliases.
pub fn physical_column_name(dialect: &Dialect, name: &str) -> String {
    if dialect.legacy_names {
        return legacy_name(dialect, name);
    }
    let mut name = name;
    for pair in &dialect.name_replaces {
        if pair.original.eq_ignore_ascii_case(name) {
            name = &pair.replace;
        }
    }
    physical_name(dialect, name)
}

/// The old Rust renderer replaced all identifiers (even tables), stopping at the first match.
pub fn legacy_name(dialect: &Dialect, name: &str) -> String {
    dialect
        .name_replaces
        .iter()
        .find(|pair| pair.original.eq_ignore_ascii_case(name))
        .map_or_else(|| name.to_string(), |pair| pair.replace.clone())
}

/// Java writes physical names unquoted. Keep quoting for safety, but emulate the
/// database's unquoted folding. Logical SQL/result aliases must not pass through here.
pub fn physical_name(dialect: &Dialect, name: &str) -> String {
    if dialect.legacy_names {
        legacy_name(dialect, name)
    } else if starts_with_ignore(&dialect.name, "postgres") {
        name.to_ascii_lowercase()
    } else if ["h2", "hsql", "oracle", "db2", "derby"]
        .iter()
        .any(|prefix| starts_with_ignore(&dialect.name, prefix))
    {
        name.to_ascii_uppercase()
    } else {
        name.to_string()
    }
}

pub fn quote_physical_ident(dialect: &Dialect, name: &str) -> String {
    quote_ident(dialect, &physical_name(dialect, name))
}

pub fn quote_column(dialect: &Dialect, name: &str) -> String {
    quote_ident(dialect, &physical_column_name(dialect, name))
}

/// Quote an identifier verbatim, including logical aliases. MySQL uses backticks.
/// For physical table/constraint names use [`quote_physical_ident`]; for columns [`quote_column`].
pub fn quote_ident(dialect: &Dialect, name: &str) -> String {
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
        out.push_str(&quote_column(dialect, name.as_ref()));
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

/// `constraint-name-clip-length`, 30 when absent, as Java defaults it.
pub(crate) fn clip_length(dialect: &Dialect) -> usize {
    dialect.constraint_name_clip_length.map_or(30, |length| length as usize)
}

/// The first `limit` characters.
pub(crate) fn clip_name(name: &str, limit: usize) -> String {
    name.chars().take(limit).collect()
}

/// Java `EntityDbMeta.shrinkName`: drop vowels from the end toward the start until the name fits,
/// then clip to one less than `limit`.
pub(crate) fn shrink_name(name: &str, limit: usize) -> String {
    let mut chars: Vec<char> = name.chars().collect();
    if chars.len() <= limit {
        return name.to_string();
    }
    let mut index = chars.len();
    while index > 0 && chars.len() > limit {
        index -= 1;
        if "AEIOUaeiou".contains(chars[index]) {
            chars.remove(index);
        }
    }
    if chars.len() > limit {
        chars.truncate(limit.saturating_sub(1));
    }
    chars.into_iter().collect()
}
