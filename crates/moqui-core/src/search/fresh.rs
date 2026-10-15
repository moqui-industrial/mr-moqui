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
//!
//! Ids are waited for once. Rows still pending at the deadline are forgotten with a warning, so a
//! row that is never delivered (a failing feed, or search not configured) does not hold every
//! later fresh search of the session for its whole deadline.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use crate::context::ExecutionContext;
use crate::entity::Condition;
use crate::user::session::update_session_attribute;

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
    update_session_attribute(ATTRIBUTE, |current| {
        let mut kept = split(current.as_deref());
        kept.extend(ids.iter().cloned());
        if kept.len() > KEEP {
            kept.drain(..kept.len() - KEEP);
        }
        Some(kept.join(","))
    });
}

/// Forget `ids`, keeping any a concurrent request of the session remembered meanwhile.
fn forget(ids: &[String]) {
    let done: HashSet<&str> = ids.iter().map(String::as_str).collect();
    update_session_attribute(ATTRIBUTE, |current| {
        let kept: Vec<String> = split(current.as_deref())
            .into_iter()
            .filter(|id| !done.contains(id.as_str()))
            .collect();
        (!kept.is_empty()).then(|| kept.join(","))
    });
}

fn split(text: Option<&str>) -> Vec<String> {
    text.map(|text| {
        text.split(',')
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn remembered() -> Vec<String> {
    if !crate::user::context_entered() {
        return Vec::new();
    }
    split(crate::user::session::session_attribute(ATTRIBUTE).as_deref())
}

/// Wait until none of this session's remembered rows remain, or `deadline`. Returns true when
/// they are all delivered. The ids waited for are forgotten either way, so a timeout costs one
/// search its deadline, not every later one.
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
        let left = facade
            .find(PENDING)?
            .condition(Condition::is_in(
                "dataFeedPendingId",
                ids.iter().map(Value::string).collect(),
            ))
            .count()?;
        if left == 0 {
            forget(&ids);
            return Ok(true);
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                rows = left,
                "read-your-writes search timed out waiting for data feed rows; the search reads without them, and \
                 later searches do not wait for them"
            );
            forget(&ids);
            return Ok(false);
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionContextFactory;

    fn pending_row(facade: &crate::entity::EntityFacade, id: &str) {
        let mut row = facade.make_value(PENDING).unwrap();
        row.set("dataFeedPendingId", Value::string(id));
        row.set("dataDocumentId", Value::string("DOC1"));
        row.set("primaryKey", Value::string("R1"));
        row.create(facade).unwrap();
    }

    #[test]
    fn a_timed_out_row_is_waited_for_once() {
        let ctx = ExecutionContextFactory::test_context();
        let facade = crate::testing::open();
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        pending_row(&facade, "STUCK1");
        remember_pending(&["STUCK1".to_string()]);
        assert_eq!(remembered(), ["STUCK1"]);

        let started = Instant::now();
        assert!(!wait_for_pending(started + Duration::from_millis(120)).unwrap());
        assert!(started.elapsed() >= Duration::from_millis(120));
        assert!(remembered().is_empty());

        let started = Instant::now();
        assert!(wait_for_pending(started + Duration::from_secs(10)).unwrap());
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn delivered_ids_are_forgotten_and_newer_ones_kept() {
        let ctx = ExecutionContextFactory::test_context();
        let facade = crate::testing::open();
        ctx.factory().set_entity(facade.clone());
        let _entered = ctx.enter();
        remember_pending(&["GONE1".to_string(), "GONE2".to_string()]);
        assert!(wait_for_pending(Instant::now() + Duration::from_secs(10)).unwrap());
        assert!(remembered().is_empty());
        remember_pending(&["A".to_string()]);
        forget(&["B".to_string()]);
        assert_eq!(remembered(), ["A"]);
    }
}
