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

//! Run one statement. The span records SQL text and `duration_ms`.

use std::time::Instant;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::cache;
use super::facade::EntityFacade;
use super::load_feed;
use crate::driver::{DbConnection, DbRow};
use crate::transaction::TransactionManager;

const SLOW_MS: u64 = 200;

pub(crate) fn query_tx(tx: &TransactionManager, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
    tx.with_connection(|conn| query_conn(conn, sql, params))
}

pub(crate) fn execute_tx(tx: &TransactionManager, sql: &str, params: &[Value]) -> Result<u64, Error> {
    tx.with_connection(|conn| execute_conn(conn, sql, params))
}

pub(crate) fn query_conn(conn: &mut dyn DbConnection, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
    trace(sql, || conn.query(sql, params))
}

pub(crate) fn execute_conn(conn: &mut dyn DbConnection, sql: &str, params: &[Value]) -> Result<u64, Error> {
    trace(sql, || conn.execute(sql, params))
}

pub(crate) fn execute_script(conn: &mut dyn DbConnection, sql: &str) -> Result<(), Error> {
    for statement in split_sql(sql) {
        execute_conn(conn, statement, &[])?;
    }
    Ok(())
}

/// Begin and commit on a reserve connection. An error rolls that connection back.
pub(crate) fn with_reserve_tx<T, F>(tx: &TransactionManager, body: F) -> Result<T, Error>
where
    F: FnOnce(&mut dyn DbConnection) -> Result<T, Error>,
{
    tx.with_reserve(|conn| {
        conn.begin()?;
        match body(conn) {
            Ok(value) => {
                if let Err(err) = conn.commit() {
                    let _ = conn.rollback();
                    return Err(err);
                }
                Ok(value)
            }
            Err(err) => {
                let _ = conn.rollback();
                Err(err)
            }
        }
    })
}

/// Register the cache and feed hooks once for the open frame.
pub(crate) fn after_statement(facade: &EntityFacade, tx: &TransactionManager, feed: bool) {
    if !tx.in_transaction() {
        return;
    }
    if !tx.has_cache_flush() && cache::register_cache_flush(facade, tx).is_err() {
        return;
    }
    // One registration per frame: the pre-commit insert, the after-commit receive, and the
    // rollback cleanup are added together, whatever other commit hooks the frame has.
    if feed && !tx.has_feed_insert() {
        let _ = load_feed::register_feed_insert(facade, tx);
    }
}

pub(crate) fn already_exists(err: &Error) -> bool {
    let mut current = Some(err as &dyn std::error::Error);
    while let Some(item) = current {
        if existence_text(&item.to_string()) || existence_text(&format!("{item:?}")) {
            return true;
        }
        current = item.source();
    }
    false
}

fn existence_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("already exists") || text.contains("duplicate")
}

pub(crate) fn split_sql(sql: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    let mut chars = sql.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if in_quote {
            if ch == '\'' {
                if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                    chars.next();
                } else {
                    in_quote = false;
                }
            }
        } else if ch == '\'' {
            in_quote = true;
        } else if ch == ';' {
            let piece = sql[start..index].trim();
            if !piece.is_empty() {
                out.push(piece);
            }
            start = index + ch.len_utf8();
        }
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn trace<T>(sql: &str, body: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    let started = Instant::now();
    let span = tracing::info_span!("sql", sql = %sql, duration_ms = tracing::field::Empty).entered();
    let result = body();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    span.record("duration_ms", duration_ms);
    if duration_ms >= SLOW_MS {
        tracing::warn!(sql = %sql, duration_ms, "slow statement");
    }
    result
}
