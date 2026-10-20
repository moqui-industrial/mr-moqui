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

//! Rendered SQL for a stable statement shape. The driver still prepares by the text.
//!
//! A hit returns the text from the first render. Parameters are bound by the caller, so a different
//! primary key does not need a new string. The cap drops the oldest shapes when a varying column list
//! would otherwise grow the map without limit.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use moqui_lib::error::Error;

use super::facade::EntityFacade;

const CAP: usize = 256;

#[derive(Default)]
struct SqlText {
    entries: HashMap<Arc<str>, Arc<str>>,
    order: VecDeque<Arc<str>>,
}

impl SqlText {
    fn get(&self, key: &str) -> Option<Arc<str>> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: &str, sql: Arc<str>) {
        if self.entries.contains_key(key) {
            return;
        }
        let key: Arc<str> = Arc::from(key);
        self.order.push_back(Arc::clone(&key));
        self.entries.insert(Arc::clone(&key), sql);
        while self.entries.len() > CAP {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            // An older queue item for a key that was inserted again is skipped: insert refuses a second copy.
            self.entries.remove(old.as_ref());
        }
    }
}

/// The SQL for `key`, rendering it once.
pub(crate) fn cached(
    facade: &EntityFacade,
    key: &str,
    render: impl FnOnce() -> Result<String, Error>,
) -> Result<Arc<str>, Error> {
    if let Some(hit) = facade.with_slot(SqlText::default, |cache| cache.get(key)) {
        return Ok(hit);
    }
    let sql: Arc<str> = Arc::from(render()?);
    let stored = Arc::clone(&sql);
    facade.with_slot(SqlText::default, |cache| cache.insert(key, stored));
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::cached;
    use crate::testing;

    #[test]
    fn the_second_call_reuses_the_rendered_text_and_a_different_key_does_not() {
        let facade = testing::open();
        let mut renders = 0;
        let first = cached(&facade, "entity|insert|testId,", || {
            renders += 1;
            Ok("INSERT INTO TEST".to_string())
        })
        .unwrap();
        let second = cached(&facade, "entity|insert|testId,", || {
            renders += 1;
            Ok("SHOULD NOT RENDER".to_string())
        })
        .unwrap();
        assert_eq!(renders, 1);
        assert_eq!(&*first, "INSERT INTO TEST");
        assert_eq!(first, second);
        let other = cached(&facade, "entity|insert|testId,testMedium,", || {
            renders += 1;
            Ok("INSERT INTO TEST (two)".to_string())
        })
        .unwrap();
        assert_eq!(renders, 2);
        assert_eq!(&*other, "INSERT INTO TEST (two)");
    }
}
