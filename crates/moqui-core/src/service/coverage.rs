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

//! Per-service call counts for `moqui test --coverage`: which services a test run reached. Off until
//! [`enable`], so a normal run pays one atomic load per call. Every call passes `call::sync_with`, which records
//! the canonical name of the service it resolved, so nested calls, ECA and SECA calls, entity-auto calls and the
//! threads of `parallel` are all counted. A call whose name does not resolve is not.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTS: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// Start counting calls, from zero.
pub fn enable() {
    if let Ok(mut counts) = COUNTS.lock() {
        counts.clear();
    }
    ENABLED.store(true, Ordering::Release);
}

/// Stop counting and return the calls so far, by service name.
pub fn finish() -> BTreeMap<String, u64> {
    ENABLED.store(false, Ordering::Release);
    COUNTS
        .lock()
        .map(|mut counts| std::mem::take(&mut *counts))
        .unwrap_or_default()
}

pub(crate) fn record(name: &str) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut counts) = COUNTS.lock() {
        *counts.entry(name.to_string()).or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_counted_only_between_enable_and_finish() {
        record("a.B.c#D");
        enable();
        record("a.B.c#D");
        record("a.B.c#D");
        record("a.B.e#F");
        let counts = finish();
        record("a.B.c#D");
        assert_eq!(counts.get("a.B.c#D"), Some(&2));
        assert_eq!(counts.get("a.B.e#F"), Some(&1));
        assert!(finish().is_empty());
    }
}
