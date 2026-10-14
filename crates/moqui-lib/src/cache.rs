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

//! Local cache. No JSR-107 surface and no process-wide manager.
//!
//! A get treats an entry as expired when `last + duration < now`, so the exact
//! boundary is still valid. `max_entries == 0` means unlimited. The background
//! pass orders overflow by lowest access count, then oldest access time, then
//! insertion sequence. The evict thread holds a weak handle and sleeps in short
//! slices, so dropping the cache does not wait for `max_check_seconds`. Each pass
//! also drops expired entries, so they do not count toward `max_entries`.
//!
//! A get takes a read lock and bumps the access count and time with atomics. Values
//! are kept behind an `Arc` and cloned after the lock is released. The eviction and
//! expiry passes rank a snapshot without the lock and take the write lock only to
//! remove.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};
use std::time::Duration;

use indexmap::IndexMap;

use crate::error::Error;

/// Time source for cache expiry. [`ManualClock`] lets a test move time without sleeping.
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        jiff::Timestamp::now().as_millisecond()
    }
}

/// A clock tests can move without sleeping.
#[derive(Debug)]
pub struct ManualClock {
    millis: AtomicI64,
}

impl ManualClock {
    pub fn new(millis: i64) -> Arc<Self> {
        Arc::new(Self {
            millis: AtomicI64::new(millis),
        })
    }

    pub fn set(&self, millis: i64) {
        self.millis.store(millis, Ordering::SeqCst);
    }

    pub fn advance(&self, delta: i64) {
        self.millis.fetch_add(delta, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> i64 {
        self.millis.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub max_check_seconds: u64,
    pub expire_access_ms: Option<i64>,
    pub expire_creation_ms: Option<i64>,
    pub expire_update_ms: Option<i64>,
    pub clock: Option<Arc<dyn Clock>>,
    /// When false, call [`MCache::evict_overflow`] directly. The default starts
    /// a thread only when `max_entries` is greater than zero.
    pub evict_in_background: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 0,
            max_check_seconds: 30,
            expire_access_ms: None,
            expire_creation_ms: None,
            expire_update_ms: None,
            clock: None,
            evict_in_background: true,
        }
    }
}

impl CacheConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries;
        self
    }

    pub fn max_check_seconds(mut self, seconds: u64) -> Self {
        self.max_check_seconds = seconds;
        self
    }

    pub fn expire_access_ms(mut self, millis: i64) -> Self {
        self.expire_access_ms = Some(millis);
        self
    }

    pub fn expire_creation_ms(mut self, millis: i64) -> Self {
        self.expire_creation_ms = Some(millis);
        self
    }

    pub fn expire_update_ms(mut self, millis: i64) -> Self {
        self.expire_update_ms = Some(millis);
        self
    }

    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn background(mut self, enabled: bool) -> Self {
        self.evict_in_background = enabled;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub gets: u64,
    pub hits: u64,
    pub misses: u64,
    pub puts: u64,
    pub expires: u64,
    pub evictions: u64,
    pub removals: u64,
}

/// What the cache admin screen lists for one cache: `CacheFacadeImpl.getAllCachesInfo`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheInfo {
    pub name: String,
    pub size: usize,
    /// 0 is no limit.
    pub max_entries: usize,
    /// Milliseconds after the last access, 0 for none.
    pub expire_idle_ms: i64,
    /// Milliseconds after creation, 0 for none.
    pub expire_live_ms: i64,
    pub stats: CacheStats,
}

/// One entry for the cache elements screen: `MEntry` times and hits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryInfo<K, V> {
    pub key: K,
    pub value: V,
    pub hit_count: u64,
    pub created: i64,
    pub updated: i64,
    pub accessed: i64,
}

struct Entry<V> {
    value: Arc<V>,
    created: i64,
    updated: i64,
    accessed: AtomicI64,
    access_count: AtomicU64,
    seq: u64,
}

impl<V> Entry<V> {
    fn new(value: V, now: i64, seq: u64) -> Self {
        Self {
            value: Arc::new(value),
            created: now,
            updated: now,
            accessed: AtomicI64::new(now),
            access_count: AtomicU64::new(0),
            seq,
        }
    }
}

#[derive(Default)]
struct AtomicStats {
    gets: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    puts: AtomicU64,
    expires: AtomicU64,
    evictions: AtomicU64,
    removals: AtomicU64,
}

fn bump(counter: &AtomicU64, by: u64) {
    if by > 0 {
        counter.fetch_add(by, Ordering::Relaxed);
    }
}

struct CacheInner<K, V> {
    name: String,
    entries: RwLock<HashMap<K, Entry<V>>>,
    stats: AtomicStats,
    max_entries: AtomicU64,
    max_check_seconds: u64,
    evict_in_background: bool,
    evictor_started: AtomicBool,
    expire_access_ms: Option<i64>,
    expire_creation_ms: Option<i64>,
    expire_update_ms: Option<i64>,
    clock: Arc<dyn Clock>,
    closed: AtomicBool,
    stop: AtomicBool,
    seq: AtomicU64,
}

/// A cloneable handle. Clones share the entry map.
pub struct MCache<K, V> {
    inner: Arc<CacheInner<K, V>>,
}

impl<K, V> Clone for MCache<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V> MCache<K, V>
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn new(name: impl Into<String>, config: CacheConfig) -> Self {
        let clock = config.clock.unwrap_or_else(|| Arc::new(SystemClock));
        let inner = Arc::new(CacheInner {
            name: name.into(),
            entries: RwLock::new(HashMap::new()),
            stats: AtomicStats::default(),
            max_entries: AtomicU64::new(config.max_entries as u64),
            max_check_seconds: config.max_check_seconds,
            evict_in_background: config.evict_in_background,
            evictor_started: AtomicBool::new(false),
            expire_access_ms: config.expire_access_ms,
            expire_creation_ms: config.expire_creation_ms,
            expire_update_ms: config.expire_update_ms,
            clock,
            closed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            seq: AtomicU64::new(1),
        });
        let cache = Self { inner };
        cache.start_evictor_if_needed();
        cache
    }

    /// Start the background pass once, when it is enabled and there is a limit to enforce.
    fn start_evictor_if_needed(&self) {
        let inner = &self.inner;
        if inner.evict_in_background
            && inner.max_check_seconds > 0
            && inner.max_entries.load(Ordering::SeqCst) > 0
            && !inner.evictor_started.swap(true, Ordering::SeqCst)
        {
            spawn_evictor(inner, inner.max_check_seconds);
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    pub fn get(&self, key: &K) -> Result<Option<V>, Error> {
        self.get_internal(key, None)
    }

    /// Drop the entry when its last update is older than `expire_before`.
    pub fn get_if_fresh(&self, key: &K, expire_before: i64) -> Result<Option<V>, Error> {
        self.get_internal(key, Some(expire_before))
    }

    pub fn put(&self, key: K, value: V) -> Result<(), Error> {
        self.ensure_open()?;
        let now = self.inner.clock.now_millis();
        let mut expired = false;
        {
            let mut entries = write_map(&self.inner.entries);
            if entries.get(&key).is_some_and(|entry| self.entry_expired(entry, now)) {
                entries.remove(&key);
                expired = true;
            }
            if let Some(entry) = entries.get_mut(&key) {
                // Same rule as `MEntry.setValue`: a clock that did not move leaves the old value.
                if now > entry.updated {
                    entry.value = Arc::new(value);
                    entry.updated = now;
                }
            } else {
                let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
                entries.insert(key, Entry::new(value, now, seq));
            }
        }
        if expired {
            bump(&self.inner.stats.expires, 1);
        }
        bump(&self.inner.stats.puts, 1);
        Ok(())
    }

    pub fn contains_key(&self, key: &K) -> Result<bool, Error> {
        self.ensure_open()?;
        let now = self.inner.clock.now_millis();
        let stale = match read_map(&self.inner.entries).get(key) {
            None => return Ok(false),
            Some(entry) => self.entry_expired(entry, now),
        };
        if stale && self.remove_if_expired(key, now) {
            return Ok(false);
        }
        Ok(read_map(&self.inner.entries).contains_key(key))
    }

    pub fn remove(&self, key: &K) -> Result<bool, Error> {
        self.ensure_open()?;
        let now = self.inner.clock.now_millis();
        let mut entries = write_map(&self.inner.entries);
        if entries.get(key).is_some_and(|entry| self.entry_expired(entry, now)) {
            entries.remove(key);
            drop(entries);
            bump(&self.inner.stats.expires, 1);
            return Ok(false);
        }
        let removed = entries.remove(key).is_some();
        drop(entries);
        if removed {
            bump(&self.inner.stats.removals, 1);
        }
        Ok(removed)
    }

    pub fn clear(&self) -> Result<(), Error> {
        self.ensure_open()?;
        write_map(&self.inner.entries).clear();
        Ok(())
    }

    pub fn len(&self) -> usize {
        read_map(&self.inner.entries).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> CacheStats {
        let stats = &self.inner.stats;
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        CacheStats {
            gets: load(&stats.gets),
            hits: load(&stats.hits),
            misses: load(&stats.misses),
            puts: load(&stats.puts),
            expires: load(&stats.expires),
            evictions: load(&stats.evictions),
            removals: load(&stats.removals),
        }
    }

    pub fn max_entries(&self) -> usize {
        self.inner.max_entries.load(Ordering::SeqCst) as usize
    }

    pub fn info(&self) -> CacheInfo {
        CacheInfo {
            name: self.inner.name.clone(),
            size: self.len(),
            max_entries: self.max_entries(),
            expire_idle_ms: self.inner.expire_access_ms.unwrap_or(0),
            expire_live_ms: self.inner.expire_creation_ms.unwrap_or(0),
            stats: self.stats(),
        }
    }

    /// Every entry with its times and hit count, in no order. Reading this is not an access.
    pub fn entry_infos(&self) -> Vec<EntryInfo<K, V>> {
        read_map(&self.inner.entries)
            .iter()
            .map(|(key, entry)| EntryInfo {
                key: key.clone(),
                value: (*entry.value).clone(),
                hit_count: entry.access_count.load(Ordering::Relaxed),
                created: entry.created,
                updated: entry.updated,
                accessed: entry.accessed.load(Ordering::Relaxed),
            })
            .collect()
    }

    /// Going from unlimited to a limit starts the background pass if the config enabled it.
    pub fn set_max_entries(&self, max_entries: usize) {
        self.inner.max_entries.store(max_entries as u64, Ordering::SeqCst);
        self.start_evictor_if_needed();
    }

    /// Remove overflow now. Tests use this instead of waiting for the thread.
    pub fn evict_overflow(&self) -> usize {
        if self.is_closed() {
            return 0;
        }
        let max = self.max_entries();
        if max == 0 {
            return 0;
        }
        let mut ranked: Vec<(u64, i64, u64, K)> = {
            let entries = read_map(&self.inner.entries);
            if entries.len() <= max {
                return 0;
            }
            entries
                .iter()
                .map(|(key, entry)| {
                    (
                        entry.access_count.load(Ordering::Relaxed),
                        entry.accessed.load(Ordering::Relaxed),
                        entry.seq,
                        key.clone(),
                    )
                })
                .collect()
        };
        ranked.sort_unstable_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
        });
        let mut entries = write_map(&self.inner.entries);
        let remove_count = entries.len().saturating_sub(max);
        let mut removed = 0;
        for (_, _, seq, key) in ranked {
            if removed >= remove_count {
                break;
            }
            // An entry replaced since the snapshot has a new sequence and is left alone.
            if entries.get(&key).is_some_and(|entry| entry.seq == seq) {
                entries.remove(&key);
                removed += 1;
            }
        }
        drop(entries);
        bump(&self.inner.stats.evictions, removed as u64);
        removed
    }

    pub fn clear_expired(&self) -> Result<usize, Error> {
        self.ensure_open()?;
        if !self.has_expiry() {
            return Ok(0);
        }
        let now = self.inner.clock.now_millis();
        let stale: Vec<K> = read_map(&self.inner.entries)
            .iter()
            .filter(|(_, entry)| self.entry_expired(entry, now))
            .map(|(key, _)| key.clone())
            .collect();
        if stale.is_empty() {
            return Ok(0);
        }
        let mut entries = write_map(&self.inner.entries);
        let mut count = 0;
        for key in stale {
            // Checked again, since a put may have refreshed the entry after the snapshot.
            if entries.get(&key).is_some_and(|entry| self.entry_expired(entry, now)) {
                entries.remove(&key);
                count += 1;
            }
        }
        drop(entries);
        bump(&self.inner.stats.expires, count as u64);
        Ok(count)
    }

    pub fn close(&self) -> Result<(), Error> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Err(Error::new(format!("cache {} is already closed", self.inner.name)));
        }
        self.inner.stop.store(true, Ordering::SeqCst);
        write_map(&self.inner.entries).clear();
        Ok(())
    }

    fn get_internal(&self, key: &K, expire_before: Option<i64>) -> Result<Option<V>, Error> {
        self.ensure_open()?;
        let has_expiry = self.has_expiry();
        let now = if has_expiry { self.inner.clock.now_millis() } else { 0 };
        let stale = |entry: &Entry<V>| {
            (has_expiry && self.entry_expired(entry, now)) || expire_before.is_some_and(|cutoff| entry.updated < cutoff)
        };
        let touch = |entry: &Entry<V>| {
            entry.access_count.fetch_add(1, Ordering::Relaxed);
            if has_expiry {
                entry.accessed.fetch_max(now, Ordering::Relaxed);
            }
            Arc::clone(&entry.value)
        };
        let mut expired = false;
        let found = match read_map(&self.inner.entries).get(key) {
            Some(entry) if !stale(entry) => Some(touch(entry)),
            Some(_) => None,
            None => {
                bump(&self.inner.stats.gets, 1);
                bump(&self.inner.stats.misses, 1);
                return Ok(None);
            }
        };
        let found = match found {
            Some(value) => Some(value),
            None => {
                // Stale under the read lock. Checked again under the write lock, since a put
                // may have replaced it in between.
                let mut entries = write_map(&self.inner.entries);
                match entries.get(key) {
                    Some(entry) if stale(entry) => {
                        entries.remove(key);
                        expired = true;
                        None
                    }
                    Some(entry) => Some(touch(entry)),
                    None => None,
                }
            }
        };
        let stats = &self.inner.stats;
        bump(&stats.gets, 1);
        if expired {
            bump(&stats.expires, 1);
        }
        bump(if found.is_some() { &stats.hits } else { &stats.misses }, 1);
        Ok(found.map(|value| V::clone(&value)))
    }

    /// Remove `key` when it is still expired under the write lock. True when it was removed.
    fn remove_if_expired(&self, key: &K, now: i64) -> bool {
        let mut entries = write_map(&self.inner.entries);
        if !entries.get(key).is_some_and(|entry| self.entry_expired(entry, now)) {
            return false;
        }
        entries.remove(key);
        drop(entries);
        bump(&self.inner.stats.expires, 1);
        true
    }

    fn has_expiry(&self) -> bool {
        self.inner.expire_access_ms.is_some()
            || self.inner.expire_creation_ms.is_some()
            || self.inner.expire_update_ms.is_some()
    }

    fn entry_expired(&self, entry: &Entry<V>, now: i64) -> bool {
        duration_expired(entry.accessed.load(Ordering::Relaxed), self.inner.expire_access_ms, now)
            || duration_expired(entry.created, self.inner.expire_creation_ms, now)
            || duration_expired(entry.updated, self.inner.expire_update_ms, now)
    }

    fn ensure_open(&self) -> Result<(), Error> {
        if self.is_closed() {
            Err(Error::new(format!("cache {} is closed", self.inner.name)))
        } else {
            Ok(())
        }
    }
}

fn duration_expired(last: i64, duration_ms: Option<i64>, now: i64) -> bool {
    duration_ms.is_some_and(|duration| last.saturating_add(duration) < now)
}

fn lock_map<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn read_map<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|err| err.into_inner())
}

fn write_map<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|err| err.into_inner())
}

fn spawn_evictor<K, V>(inner: &Arc<CacheInner<K, V>>, seconds: u64)
where
    K: Clone + Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let weak: Weak<CacheInner<K, V>> = Arc::downgrade(inner);
    let period_ms = seconds.saturating_mul(1000);
    let _ = std::thread::Builder::new()
        .name("moqui-mcache-evict".to_string())
        .spawn(move || {
            let slice = Duration::from_millis(50);
            let mut waited = 0_u64;
            loop {
                std::thread::sleep(slice);
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                if inner.stop.load(Ordering::SeqCst) {
                    break;
                }
                waited = waited.saturating_add(50);
                if period_ms > 0 && waited >= period_ms {
                    waited = 0;
                    let cache = MCache { inner };
                    let _ = cache.clear_expired();
                    cache.evict_overflow();
                }
            }
        });
}

struct Slot {
    typed: Arc<dyn std::any::Any + Send + Sync>,
    close: Box<dyn Fn() + Send + Sync>,
    info: Box<dyn Fn() -> CacheInfo + Send + Sync>,
    clear: Box<dyn Fn() -> Result<(), Error> + Send + Sync>,
}

/// Name-to-cache map. `create` returns the existing cache when the name is already there.
pub struct MCacheManager {
    slots: Mutex<IndexMap<String, Slot>>,
    closed: AtomicBool,
}

impl MCacheManager {
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(IndexMap::new()),
            closed: AtomicBool::new(false),
        }
    }

    pub fn create<K, V>(&self, name: impl Into<String>, config: CacheConfig) -> Result<MCache<K, V>, Error>
    where
        K: Clone + Eq + Hash + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.ensure_open()?;
        let name = name.into();
        let mut slots = lock_map(&self.slots);
        if let Some(slot) = slots.get(&name) {
            return downcast_cache(&name, Arc::clone(&slot.typed));
        }
        let cache = MCache::<K, V>::new(name.clone(), config);
        let typed = cache.inner.clone() as Arc<dyn std::any::Any + Send + Sync>;
        let closing = cache.clone();
        let reading = cache.clone();
        let clearing = cache.clone();
        slots.insert(
            name,
            Slot {
                typed,
                close: Box::new(move || {
                    let _ = closing.close();
                }),
                info: Box::new(move || reading.info()),
                clear: Box::new(move || clearing.clear()),
            },
        );
        Ok(cache)
    }

    pub fn get<K, V>(&self, name: &str) -> Result<Option<MCache<K, V>>, Error>
    where
        K: Clone + Eq + Hash + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.ensure_open()?;
        let slots = lock_map(&self.slots);
        match slots.get(name) {
            Some(slot) => downcast_cache(name, Arc::clone(&slot.typed)).map(Some),
            None => Ok(None),
        }
    }

    pub fn cache_names(&self) -> Result<Vec<String>, Error> {
        self.ensure_open()?;
        Ok(lock_map(&self.slots).keys().cloned().collect())
    }

    /// The info of each cache, whatever its key and value types, in creation order.
    pub fn infos(&self) -> Result<Vec<CacheInfo>, Error> {
        self.ensure_open()?;
        Ok(lock_map(&self.slots).values().map(|slot| (slot.info)()).collect())
    }

    /// Clear one cache by name. `false` when there is no cache of that name.
    pub fn clear(&self, name: &str) -> Result<bool, Error> {
        self.ensure_open()?;
        let slots = lock_map(&self.slots);
        match slots.get(name) {
            Some(slot) => (slot.clear)().map(|()| true),
            None => Ok(false),
        }
    }

    /// Clear every cache, as `CacheFacade.clearAllCaches`.
    pub fn clear_all(&self) -> Result<(), Error> {
        self.ensure_open()?;
        for slot in lock_map(&self.slots).values() {
            (slot.clear)()?;
        }
        Ok(())
    }

    pub fn destroy(&self, name: &str) -> Result<(), Error> {
        self.ensure_open()?;
        let mut slots = lock_map(&self.slots);
        let Some(slot) = slots.shift_remove(name) else {
            return Err(Error::new(format!("cache {name} does not exist")));
        };
        drop(slots);
        (slot.close)();
        Ok(())
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let slots = std::mem::take(&mut *lock_map(&self.slots));
        for (_, slot) in slots {
            (slot.close)();
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn ensure_open(&self) -> Result<(), Error> {
        if self.is_closed() {
            Err(Error::new("cache manager is closed"))
        } else {
            Ok(())
        }
    }
}

impl Default for MCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

fn downcast_cache<K, V>(name: &str, typed: Arc<dyn std::any::Any + Send + Sync>) -> Result<MCache<K, V>, Error>
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
{
    typed
        .downcast::<CacheInner<K, V>>()
        .map(|inner| MCache { inner })
        .map_err(|_| Error::new(format!("cache {name} exists with a different key or value type")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_uses_a_strict_less_than_boundary() {
        let clock = ManualClock::new(1_000);
        let cache = MCache::<String, String>::new(
            "entity.definition",
            CacheConfig::new()
                .expire_access_ms(1_000)
                .clock(Arc::clone(&clock) as Arc<dyn Clock>)
                .background(false),
        );
        cache.put("a".to_string(), "one".to_string()).unwrap();
        // Same instant as the put, so the hit does not move last-access time.
        assert_eq!(cache.get(&"a".to_string()).unwrap().as_deref(), Some("one"));
        clock.set(2_000);
        assert!(cache.contains_key(&"a".to_string()).unwrap());
        clock.set(2_001);
        assert_eq!(cache.get(&"a".to_string()).unwrap(), None);
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.expires, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn fresh_get_and_eviction_order() {
        let clock = ManualClock::new(10);
        let cache = MCache::<&'static str, i32>::new(
            "screen",
            CacheConfig::new()
                .max_entries(2)
                .clock(Arc::clone(&clock) as Arc<dyn Clock>)
                .background(false),
        );
        cache.put("old", 1).unwrap();
        clock.advance(5);
        cache.put("new", 2).unwrap();
        assert_eq!(cache.get(&"old").unwrap(), Some(1));
        clock.advance(5);
        cache.put("third", 3).unwrap();
        assert_eq!(cache.evict_overflow(), 1);
        assert_eq!(cache.get(&"new").unwrap(), None);
        assert_eq!(cache.get(&"old").unwrap(), Some(1));
        assert_eq!(cache.get(&"third").unwrap(), Some(3));
        assert_eq!(cache.stats().evictions, 1);
        clock.set(100);
        cache.put("aged", 7).unwrap();
        clock.set(200);
        assert_eq!(cache.get_if_fresh(&"aged", 150).unwrap(), None);
        cache.close().unwrap();
        assert!(cache.get(&"new").is_err());
        assert!(cache.close().is_err());
    }

    #[test]
    fn concurrent_gets_while_evicting() {
        let cache = MCache::<u32, Vec<u32>>::new("lists", CacheConfig::new().max_entries(100).background(false));
        for key in 0..1_000 {
            cache.put(key, vec![key; 8]).unwrap();
        }
        std::thread::scope(|scope| {
            for reader in 0..4_u32 {
                let cache = cache.clone();
                scope.spawn(move || {
                    for round in 0..2_000_u32 {
                        let key = (round * 7 + reader) % 1_000;
                        if let Some(value) = cache.get(&key).unwrap() {
                            assert_eq!(value, vec![key; 8]);
                        }
                    }
                });
            }
            scope.spawn(|| {
                cache.evict_overflow();
            });
        });
        assert_eq!(cache.len(), 100);
        let stats = cache.stats();
        assert_eq!(stats.gets, 8_000);
        assert_eq!(stats.hits + stats.misses, stats.gets);
        assert_eq!(stats.evictions, 900);
    }

    #[test]
    fn expired_entries_do_not_count_toward_the_limit() {
        let clock = ManualClock::new(0);
        let cache = MCache::<u32, u32>::new(
            "ttl",
            CacheConfig::new()
                .max_entries(2)
                .expire_creation_ms(10)
                .clock(Arc::clone(&clock) as Arc<dyn Clock>)
                .background(false),
        );
        cache.put(1, 1).unwrap();
        cache.put(2, 2).unwrap();
        clock.set(100);
        cache.put(3, 3).unwrap();
        // What the background pass does: expired first, then overflow.
        assert_eq!(cache.clear_expired().unwrap(), 2);
        assert_eq!(cache.evict_overflow(), 0);
        assert_eq!(cache.get(&3).unwrap(), Some(3));
    }

    #[test]
    fn setting_a_limit_starts_the_background_pass() {
        let cache = MCache::<u32, u32>::new("late", CacheConfig::new().max_check_seconds(1));
        for key in 0..5 {
            cache.put(key, key).unwrap();
        }
        cache.set_max_entries(2);
        let started = std::time::Instant::now();
        while cache.len() > 2 && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn manager_returns_the_existing_cache_and_drop_does_not_wait() {
        let manager = MCacheManager::new();
        let clock = ManualClock::new(0);
        let first = manager
            .create::<String, i32>(
                "local",
                CacheConfig::new()
                    .max_entries(1)
                    .max_check_seconds(30)
                    .clock(clock)
                    .background(true),
            )
            .unwrap();
        let again = manager.get::<String, i32>("local").unwrap().unwrap();
        again.put("k".to_string(), 1).unwrap();
        assert_eq!(first.get(&"k".to_string()).unwrap(), Some(1));
        let started = std::time::Instant::now();
        drop(first);
        drop(again);
        manager.destroy("local").unwrap();
        assert!(started.elapsed() < Duration::from_millis(50));
        assert!(manager.destroy("local").is_err());
    }

    #[test]
    fn manager_lists_and_clears_caches_of_any_type() {
        let manager = MCacheManager::new();
        let text = manager
            .create::<String, String>(
                "l10n.message",
                CacheConfig::new().expire_access_ms(5_000).background(false),
            )
            .unwrap();
        let numbers = manager
            .create::<i32, i64>("other", CacheConfig::new().max_entries(10).background(false))
            .unwrap();
        text.put("a".to_string(), "one".to_string()).unwrap();
        assert_eq!(text.get(&"a".to_string()).unwrap().as_deref(), Some("one"));
        numbers.put(1, 2).unwrap();
        numbers.put(3, 4).unwrap();
        let infos = manager.infos().unwrap();
        assert_eq!(infos.len(), 2);
        assert_eq!(
            (infos[0].name.as_str(), infos[0].size, infos[0].expire_idle_ms),
            ("l10n.message", 1, 5_000)
        );
        assert_eq!(
            (infos[1].name.as_str(), infos[1].size, infos[1].max_entries),
            ("other", 2, 10)
        );
        assert_eq!(infos[0].stats.hits, 1);
        let entries = text.entry_infos();
        assert_eq!((entries[0].key.as_str(), entries[0].value.as_str()), ("a", "one"));
        assert!(manager.clear("other").unwrap());
        assert!(!manager.clear("missing").unwrap());
        assert_eq!(numbers.len(), 0);
        manager.clear_all().unwrap();
        assert_eq!(text.len(), 0);
    }
}
