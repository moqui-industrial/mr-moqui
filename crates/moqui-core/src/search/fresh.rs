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

//! Read-your-writes for a search that asks for it (`SearchRequest::fresh_by`).
//!
//! A user's entity writes become `DataFeedPending` rows, and indexing them is eventual. The ids of
//! the rows a session's transactions inserted are remembered in that session. A fresh search
//! first waits, until its deadline, for those rows to be delivered (the indexing committed). The
//! Postgres backend then waits for its read replica to replay the primary, or runs on the primary.

use std::time::{Duration, Instant};

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use crate::context::ExecutionContext;
use crate::entity::Condition;

const PENDING: &str = "moqui.entity.feed.DataFeedPending";
/// The session attribute that holds the remembered row ids, comma-separated.
const ATTRIBUTE: &str = "moqui.search.pendingIds";
/// Only the newest ids are kept.
const KEEP: usize = 200;
const POLL: Duration = Duration::from_millis(50);

/// Remember pending row ids a transaction of this session inserted. Without a session, nothing.
pub fn remember_pending(ids: &[String]) {
    if ids.is_empty() || !crate::user::context_entered() {
        return;
    }
    let mut kept: Vec<String> = remembered();
    kept.extend(ids.iter().cloned());
    if kept.len() > KEEP {
        kept.drain(..kept.len() - KEEP);
    }
    crate::user::session::set_session_attribute(ATTRIBUTE, Some(&kept.join(",")));
}

fn remembered() -> Vec<String> {
    if !crate::user::context_entered() {
        return Vec::new();
    }
    crate::user::session::session_attribute(ATTRIBUTE)
        .map(|text| {
            text.split(',')
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Wait until none of this session's remembered rows remain, or `deadline`. Returns true when
/// they are all delivered. Delivered ids are forgotten.
///
/// # Errors
///
/// Returns an error when the pending rows cannot be read.
pub fn wait_for_pending(deadline: Instant) -> Result<bool, Error> {
    let ids = remembered();
    if ids.is_empty() {
        return Ok(true);
    }
    let facade = ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("read-your-writes needs an entity facade"))?;
    let _authz = ExecutionContext::with_current(|ec| ec.disable_authz());
    loop {
        let left: Vec<String> = facade
            .find(PENDING)?
            .condition(Condition::is_in(
                "dataFeedPendingId",
                ids.iter().map(Value::string).collect(),
            ))
            .list()?
            .iter()
            .filter_map(|row| row.get("dataFeedPendingId").map(moqui_lib::object::to_plain_string))
            .collect();
        if left.is_empty() {
            crate::user::session::set_session_attribute(ATTRIBUTE, None);
            return Ok(true);
        }
        if Instant::now() >= deadline {
            crate::user::session::set_session_attribute(ATTRIBUTE, Some(&left.join(",")));
            return Ok(false);
        }
        std::thread::sleep(POLL);
    }
}
