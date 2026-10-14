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

//! A map with a size cap for the compiled-code caches. When it is full, the quarter of the
//! entries used longest ago go, so the code in use stays compiled instead of all of it being
//! thrown out at once.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

pub(crate) struct BoundedMap<K, V> {
    entries: HashMap<K, (V, u64)>,
    tick: u64,
    cap: usize,
}

impl<K: Hash + Eq + Clone, V: Clone> BoundedMap<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            tick: 0,
            cap: cap.max(4),
        }
    }

    pub(crate) fn get<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(value, used)| {
            *used = tick;
            value.clone()
        })
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        if self.entries.len() >= self.cap && !self.entries.contains_key(&key) {
            self.evict();
        }
        self.tick += 1;
        self.entries.insert(key, (value, self.tick));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    fn evict(&mut self) {
        let mut ticks: Vec<u64> = self.entries.values().map(|(_, used)| *used).collect();
        ticks.sort_unstable();
        let cutoff = ticks[(ticks.len() / 4).max(1) - 1];
        self.entries.retain(|_, (_, used)| *used > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::BoundedMap;

    #[test]
    fn a_full_map_drops_the_entries_used_longest_ago() {
        let mut map = BoundedMap::new(8);
        for index in 0..8 {
            map.insert(index, index);
        }
        // 0 and 1 are used again, so 2 and 3 are the oldest quarter.
        assert_eq!(map.get(&0), Some(0));
        assert_eq!(map.get(&1), Some(1));
        map.insert(8, 8);
        assert_eq!(map.len(), 7);
        assert_eq!(map.get(&2), None);
        assert_eq!(map.get(&3), None);
        assert_eq!(map.get(&4), Some(4));
        assert_eq!(map.get(&0), Some(0));
        assert_eq!(map.get(&8), Some(8));
    }
}
