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

//! Artifact hits and hit bins. A popped artifact frame is counted here in memory, as Java's
//! `countArtifactHit`. The scheduler tick writes `moqui.server.ArtifactHit` and
//! `moqui.server.ArtifactHitBin` on a reserve connection, so a request never writes them.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use moqui_db::sql::insert as sql_insert;
use moqui_lib::node::MNode;
use moqui_lib::object::{to_plain_string, Value};
use moqui_lib::sync::lock;

use super::error::Error;
use super::factory::ExecutionContextFactory;
use crate::entity::EntityFacade;
use crate::user::{now_millis, stamp_value};

const HIT: &str = "moqui.server.ArtifactHit";
const HIT_BIN: &str = "moqui.server.ArtifactHitBin";
/// Hits before a running time can be called slow.
const CHECK_SLOW_THRESHOLD: u64 = 50;
/// A slow hit longer than this is always written, whatever `persist-hit` says.
const USER_IMPACT_MIN_MILLIS: f64 = 1000.0;
/// Rows per transaction in a flush, Java's `maxCreates`.
const MAX_CREATES: usize = 1000;
const FLUSH_RETRIES: u32 = 5;
/// Hits held for the flush. Past this, until a tick takes them, a new hit is dropped and counted, so memory stays
/// bounded when no tick runs.
const MAX_QUEUED_HITS: usize = 10_000;
/// Queued hits at which [`ArtifactHits::count`] asks for a flush now, as the scheduler tick may not run for a long
/// time, such as during one long call. Half the limit, so the flush has room to run while hits keep coming.
const FLUSH_QUEUED_HITS: usize = MAX_QUEUED_HITS / 2;
/// Closed bins at which a flush is asked for, half the limit as for hits.
const FLUSH_CLOSED_BINS: usize = MAX_CLOSED_BINS / 2;
/// Closed bins held for the flush, dropped and counted past this as hits are.
const MAX_CLOSED_BINS: usize = 20_000;
/// Distinct artifacts with counts. A new artifact past this is not binned, and is counted as dropped.
const MAX_STATS: usize = 20_000;
/// Exact parameter names, in lower case, left out of a hit's `parameterString` besides the web credential names.
const SECRET_PARAMETER_NAMES: &[&str] = &["code", "moquisessiontoken", "sessiontoken", "sql", "sharedsecret"];

/// Entities whose operations are not counted, so counting does not count itself.
const ENTITIES_TO_SKIP: &[&str] = &[
    "moqui.server.ArtifactHit",
    "create#moqui.server.ArtifactHit",
    "moqui.server.ArtifactHitBin",
    "create#moqui.server.ArtifactHitBin",
    "moqui.entity.SequenceValueItem",
    "moqui.security.UserAccount",
    "moqui.entity.document.DataDocument",
    "moqui.entity.document.DataDocumentField",
    "moqui.entity.document.DataDocumentCondition",
    "moqui.entity.feed.DataFeedAndDocument",
    "moqui.entity.view.DbViewEntity",
    "moqui.entity.view.DbViewEntityMember",
    "moqui.entity.view.DbViewEntityKeyMap",
    "moqui.entity.view.DbViewEntityAlias",
];

/// One hit as the artifact stack or a screen reports it.
pub(crate) struct Hit<'a> {
    pub artifact_type: &'a str,
    pub sub_type: Option<&'a str>,
    pub name: &'a str,
    pub parameters: Option<&'a BTreeMap<String, Value>>,
    pub start_ms: i64,
    pub running_ms: f64,
    pub output_size: Option<i64>,
}

/// What a written hit takes from the context: read only when a hit row is made.
#[derive(Default)]
pub(crate) struct HitContext {
    pub visit_id: Option<String>,
    pub user_id: Option<String>,
    pub errors: Vec<String>,
    pub request_url: Option<String>,
    pub referrer_url: Option<String>,
}

/// The `server-stats` settings and the counts since start. One per factory.
pub(crate) struct ArtifactHits {
    bin_length_ms: i64,
    /// `artifact-stats` per type: `(persist-hit, persist-bin)`.
    by_type: HashMap<String, (bool, bool)>,
    state: Mutex<State>,
    /// True while a background flush started by [`spawn_flush`] runs, so only one runs at a time.
    flushing: AtomicBool,
    /// The background flush threads not yet seen to finish, for shutdown to wait on before the database closes.
    flush_threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

#[derive(Default)]
struct State {
    stats: HashMap<String, StatsInfo>,
    /// Insertion number of the next new artifact, for [`java_hash_map_order`].
    next_order: u64,
    /// Bins that ended, waiting for the flush.
    closed: Vec<BinRow>,
    /// Hits waiting for the flush, Java's `deferredHitInfoQueue`.
    hits: Vec<HitRow>,
    /// Hits, closed bins, and new artifacts dropped by the limits since the last flush.
    dropped: u64,
}

impl State {
    /// Count one thing dropped by a limit. The first, and every thousandth after, is logged.
    fn drop_one(&mut self, what: &str) {
        self.dropped += 1;
        if self.dropped == 1 || self.dropped.is_multiple_of(1000) {
            tracing::warn!(
                dropped = self.dropped,
                "Artifact hit {what} dropped, the in-memory limit is reached and no flush has run"
            );
        }
    }
}

/// Java's `ArtifactStatsInfo`: counts since start for one artifact, and its current bin.
struct StatsInfo {
    /// When the artifact was first counted since start, see [`java_hash_map_order`].
    order: u64,
    artifact_type: String,
    sub_type: Option<String>,
    name: String,
    cur_bin: Option<BinInfo>,
    hit_count: u64,
    total_time: f64,
    total_squared: f64,
}

/// Java's `ArtifactBinInfo`.
struct BinInfo {
    start_ms: i64,
    hit_count: u64,
    slow_hit_count: u64,
    total_time: f64,
    total_squared: f64,
    min_time: f64,
    max_time: f64,
}

struct BinRow {
    artifact_type: String,
    sub_type: Option<String>,
    name: String,
    end_ms: i64,
    bin: BinInfo,
}

struct HitRow {
    context: HitContext,
    is_slow: bool,
    artifact_type: String,
    sub_type: Option<String>,
    name: String,
    start_ms: i64,
    running_ms: f64,
    parameter_string: Option<String>,
    output_size: Option<i64>,
}

impl ArtifactHits {
    /// Read `server-stats` from the merged conf. No conf counts nothing.
    pub(crate) fn from_conf(conf_text: &str) -> Self {
        let mut bin_length_ms = 900_000;
        let mut by_type = HashMap::new();
        let stats = (!conf_text.trim().is_empty())
            .then(|| MNode::parse_shared("moqui-conf", conf_text).ok())
            .flatten()
            .and_then(|root| root.first_named("server-stats"));
        if let Some(stats) = stats {
            if let Some(seconds) = stats
                .attribute("bin-length-seconds")
                .and_then(|text| text.trim().parse::<i64>().ok())
            {
                bin_length_ms = seconds.saturating_mul(1000);
            }
            for node in stats.named_iter("artifact-stats") {
                let Some(artifact_type) = node.attribute("type") else {
                    continue;
                };
                // The first entry for a type wins, as Java's getArtifactStatsNode with no sub-type.
                by_type.entry(artifact_type).or_insert((
                    node.attribute_is("persist-hit", "true"),
                    node.attribute_is("persist-bin", "true"),
                ));
            }
        }
        Self {
            bin_length_ms,
            by_type,
            state: Mutex::new(State::default()),
            flushing: AtomicBool::new(false),
            flush_threads: Mutex::new(Vec::new()),
        }
    }

    fn persist(&self, artifact_type: &str) -> (bool, bool) {
        self.by_type.get(artifact_type).copied().unwrap_or((false, false))
    }

    /// Whether a hit of this type is counted at all, so a caller need not keep its parameters.
    pub(crate) fn counts(&self, artifact_type: &str) -> bool {
        let (hit, bin) = self.persist(artifact_type);
        hit || bin
    }

    /// Java's `countArtifactHit`. `context` is read only when the hit is written. Returns true when the queued hits
    /// or closed bins have reached the flush threshold, so the caller should start a flush rather than wait for the
    /// scheduler tick, see [`spawn_flush`]. The limits stay as the safety net when no flush runs.
    pub(crate) fn count(&self, hit: &Hit<'_>, context: impl FnOnce() -> HitContext) -> bool {
        let (persist_hit, persist_bin) = self.persist(hit.artifact_type);
        if !persist_hit && !persist_bin {
            return false;
        }
        self.count_counted(hit, persist_hit, persist_bin, context);
        let state = lock(&self.state);
        state.hits.len() >= FLUSH_QUEUED_HITS || state.closed.len() >= FLUSH_CLOSED_BINS
    }

    fn count_counted(&self, hit: &Hit<'_>, persist_hit: bool, persist_bin: bool, context: impl FnOnce() -> HitContext) {
        let is_entity =
            hit.artifact_type == "AT_ENTITY" || hit.sub_type.is_some_and(|sub_type| sub_type.starts_with("entity"));
        if is_entity && ENTITIES_TO_SKIP.contains(&hit.name) {
            return;
        }
        let mut state = lock(&self.state);
        let mut is_slow = false;
        if persist_bin {
            // Artifact names are unique enough that the type is not in the key.
            let key = match hit.sub_type {
                Some(sub_type) if hit.artifact_type != "AT_SERVICE" => format!("{}{sub_type}", hit.name),
                _ => hit.name.to_string(),
            };
            if state.stats.len() >= MAX_STATS && !state.stats.contains_key(&key) {
                state.drop_one("artifact");
                return;
            }
            let State {
                stats,
                closed,
                next_order,
                ..
            } = &mut *state;
            let full = closed.len() >= MAX_CLOSED_BINS;
            let info = stats.entry(key).or_insert_with(|| {
                *next_order += 1;
                StatsInfo {
                    order: *next_order,
                    artifact_type: hit.artifact_type.to_string(),
                    sub_type: hit.sub_type.map(str::to_string),
                    name: hit.name.to_string(),
                    cur_bin: None,
                    hit_count: 0,
                    total_time: 0.0,
                    total_squared: 0.0,
                }
            });
            // The current bin ended since the last hit: it waits for the flush, and a new one starts.
            let expired = info
                .cur_bin
                .as_ref()
                .is_some_and(|bin| hit.start_ms > bin.start_ms + self.bin_length_ms);
            let mut dropped_bin = false;
            if expired {
                if let Some(row) = info.close_bin(self.bin_length_ms, None) {
                    if full {
                        dropped_bin = true;
                    } else {
                        closed.push(row);
                    }
                }
                info.cur_bin = Some(BinInfo::new(hit.start_ms));
            }
            is_slow = info.count_hit(hit.start_ms, hit.running_ms);
            if dropped_bin {
                state.drop_one("bin");
            }
        }
        // Single hits of an entity are never written: too many, and they would count themselves.
        if is_entity || !((is_slow && hit.running_ms > USER_IMPACT_MIN_MILLIS) || persist_hit) {
            return;
        }
        drop(state);
        let row = HitRow {
            context: context(),
            is_slow,
            artifact_type: hit.artifact_type.to_string(),
            sub_type: hit.sub_type.map(str::to_string),
            name: hit.name.to_string(),
            start_ms: hit.start_ms,
            running_ms: hit.running_ms,
            parameter_string: hit.parameters.and_then(parameter_string),
            output_size: hit.output_size,
        };
        let mut state = lock(&self.state);
        if state.hits.len() >= MAX_QUEUED_HITS {
            state.drop_one("row");
        } else {
            state.hits.push(row);
        }
    }

    /// Wait up to `limit` for a background flush that is still running. Shutdown calls this after the scheduler has
    /// stopped and before the database closes, so the flush does not fail on a closed database.
    pub(crate) fn wait_for_flush(&self, limit: std::time::Duration) {
        let handles = std::mem::take(&mut *lock(&self.flush_threads));
        let deadline = std::time::Instant::now() + limit;
        for handle in handles {
            while !handle.is_finished() {
                if std::time::Instant::now() >= deadline {
                    tracing::warn!(
                        "Artifact hit flush still running after {}s; closing the database anyway",
                        limit.as_secs()
                    );
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _ = handle.join();
        }
    }

    /// Claim the one background flush. False when one is already running.
    fn begin_flush(&self) -> bool {
        self.flushing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Take what the flush writes: the waiting hits, the closed bins, and each current bin whose
    /// time is over. `all` also takes the current bins that are still open, as Java's `destroy`.
    ///
    /// Current bins are taken in the iteration order of Java's `artifactStatsInfoByType` `HashMap`, which is the
    /// order Java's `destroy` gives them their sequenced ids. Bins of Rust-only sub types (see
    /// [`crate::entity::RustOnlyEntityHits`]) have no place in Java's map: they are left out of its order and taken
    /// after it, so the bins Java also has get the same ids.
    fn take(&self, all: bool) -> (Vec<HitRow>, Vec<BinRow>) {
        let now = now_millis();
        let mut state = lock(&self.state);
        let State {
            stats,
            closed,
            hits,
            dropped,
            ..
        } = &mut *state;
        let (rust_only, java): (Vec<_>, Vec<_>) = stats
            .iter()
            .map(|(key, info)| (key.as_str(), info.order, info.sub_type.as_deref()))
            .partition(|(_, _, sub_type)| crate::entity::is_rust_only_sub_type(*sub_type));
        let mut keys = java_hash_map_order(java.into_iter().map(|(key, order, _)| (key, order)));
        let mut rust_only: Vec<(&str, u64)> = rust_only.into_iter().map(|(key, order, _)| (key, order)).collect();
        rust_only.sort_by_key(|(_, order)| *order);
        keys.extend(rust_only.into_iter().map(|(key, _)| key.to_string()));
        for key in keys {
            let Some(info) = stats.get_mut(&key) else {
                continue;
            };
            let Some(start) = info.cur_bin.as_ref().map(|bin| bin.start_ms) else {
                continue;
            };
            if now > start + self.bin_length_ms {
                closed.extend(info.close_bin(self.bin_length_ms, None));
            } else if all {
                closed.extend(info.close_bin(self.bin_length_ms, Some(now)));
            }
        }
        if *dropped > 0 {
            tracing::warn!(
                dropped = *dropped,
                "Artifact hits, bins, or artifacts were dropped since the last flush"
            );
            *dropped = 0;
        }
        (std::mem::take(hits), std::mem::take(closed))
    }
}

/// Keys in the iteration order of a Java `HashMap<String, _>` the keys were put into in `order`, with nothing
/// removed: by bucket, `(h ^ (h >>> 16)) & (capacity - 1)` of the Java `String.hashCode`, and in each bucket by
/// insertion, which a resize keeps. The capacity starts at 16 and doubles while the size is over 3/4 of it. A bucket
/// of 8 or more keys, which Java turns into a tree once the capacity is 64, is not modelled.
fn java_hash_map_order<'a>(keys: impl Iterator<Item = (&'a str, u64)>) -> Vec<String> {
    let mut keyed: Vec<(u32, u64, &str)> = Vec::new();
    for (key, order) in keys {
        let hash = key
            .encode_utf16()
            .fold(0_u32, |hash, unit| hash.wrapping_mul(31).wrapping_add(u32::from(unit)));
        keyed.push((hash ^ (hash >> 16), order, key));
    }
    let mut capacity = 16_usize;
    while keyed.len() > capacity / 4 * 3 {
        capacity *= 2;
    }
    let mask = u32::try_from(capacity - 1).unwrap_or(u32::MAX);
    keyed.sort_by_key(|(spread, order, _)| (spread & mask, *order));
    keyed.into_iter().map(|(_, _, key)| key.to_string()).collect()
}

impl StatsInfo {
    /// Count a hit and say whether it was slow: more than 2.6 standard deviations above the average.
    fn count_hit(&mut self, start_ms: i64, running: f64) -> bool {
        self.hit_count += 1;
        let is_slow = self.is_hit_slow(running);
        // The first hit is almost always much slower, so when it is over 3 times the second the second counts twice.
        if self.hit_count == 2 && self.total_time > running * 3.0 {
            self.total_time = running * 2.0;
            self.total_squared = running * running * 2.0;
        } else {
            self.total_time += running;
            self.total_squared += running * running;
        }
        self.cur_bin
            .get_or_insert_with(|| BinInfo::new(start_ms))
            .count_hit(running, is_slow);
        is_slow
    }

    fn is_hit_slow(&self, running: f64) -> bool {
        if self.hit_count < CHECK_SLOW_THRESHOLD {
            return false;
        }
        let count = self.hit_count as f64;
        let average = self.total_time / count;
        let std_dev = ((self.total_squared - (self.total_time * self.total_time) / count).abs() / (count - 1.0)).sqrt();
        let slow_time = average + std_dev * 2.6;
        if slow_time != 0.0 && running > slow_time {
            if running > USER_IMPACT_MIN_MILLIS {
                tracing::warn!(
                    "Slow hit to {}:{}:{} running time {running} is greater than average {average} plus 2.6 standard deviations {std_dev}",
                    self.artifact_type,
                    self.sub_type.as_deref().unwrap_or("null"),
                    self.name
                );
            }
            return true;
        }
        false
    }

    /// End the current bin at its length, or at `end_ms` when given.
    fn close_bin(&mut self, bin_length_ms: i64, end_ms: Option<i64>) -> Option<BinRow> {
        let bin = self.cur_bin.take()?;
        Some(BinRow {
            artifact_type: self.artifact_type.clone(),
            sub_type: self.sub_type.clone(),
            name: self.name.clone(),
            end_ms: end_ms.unwrap_or(bin.start_ms + bin_length_ms),
            bin,
        })
    }
}

impl BinInfo {
    fn new(start_ms: i64) -> Self {
        Self {
            start_ms,
            hit_count: 0,
            slow_hit_count: 0,
            total_time: 0.0,
            total_squared: 0.0,
            min_time: f64::MAX,
            max_time: 0.0,
        }
    }

    fn count_hit(&mut self, running: f64, is_slow: bool) {
        self.hit_count += 1;
        if is_slow {
            self.slow_hit_count += 1;
        }
        if self.hit_count == 2 && self.total_time > running * 3.0 {
            self.total_time = running * 2.0;
            self.total_squared = running * running * 2.0;
        } else {
            self.total_time += running;
            self.total_squared += running * running;
        }
        self.min_time = self.min_time.min(running);
        self.max_time = self.max_time.max(running);
    }
}

/// `key=value` pairs, without maps, lists, empty values and [`secret_parameter`] names.
/// Values are cut at 80 characters and the whole at 1000.
fn parameter_string(parameters: &BTreeMap<String, Value>) -> Option<String> {
    if parameters.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (key, value) in parameters {
        if matches!(value, Value::Null | Value::Map(_) | Value::List(_) | Value::Object(_)) || secret_parameter(key) {
            continue;
        }
        let text = to_plain_string(value);
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(key);
        out.push('=');
        out.push_str(&truncate(&text, 80));
    }
    Some(truncate(&out, 1000))
}

/// A parameter a hit row must not keep, compared without case: the web credential names, a second-factor `code`,
/// session tokens, SQL text, a shared secret, and any name with `password`, `secret`, or `token` in it or ending in
/// `_key`, `apikey`, or `loginkey`. Ids such as `primaryKeyId` or `monkeyId` are kept.
/// Java left out only names with `password`.
pub(crate) fn secret_parameter(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    crate::web::CREDENTIAL_PARAMETER_NAMES.contains(&lower.as_str())
        || SECRET_PARAMETER_NAMES.contains(&lower.as_str())
        || ["password", "secret", "token"].iter().any(|part| lower.contains(part))
        || ["_key", "apikey", "loginkey"].iter().any(|end| lower.ends_with(end))
}

fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// The host address and name, as Java's `InetAddress.getLocalHost()`, found once.
pub(crate) fn server_address() -> &'static (String, String) {
    static ADDRESS: OnceLock<(String, String)> = OnceLock::new();
    ADDRESS.get_or_init(|| {
        let name = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .or_else(|| std::env::var("HOSTNAME").ok())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        let Some(name) = name else {
            return ("127.0.0.1".to_string(), "localhost".to_string());
        };
        // Java orders the resolved addresses IPv4 first (java.net.preferIPv6Addresses defaults to false), keeping
        // the resolver's order within each family, and takes the first.
        let addresses: Vec<std::net::IpAddr> = std::net::ToSocketAddrs::to_socket_addrs(&(name.as_str(), 0))
            .map(|found| found.map(|address| address.ip()).collect())
            .unwrap_or_default();
        let ip = addresses
            .iter()
            .find(|address| address.is_ipv4())
            .or_else(|| addresses.first())
            .map_or_else(|| "127.0.0.1".to_string(), ToString::to_string);
        (ip, name)
    })
}

/// Write the waiting hits and the bins whose time is over. The scheduler tick calls this with
/// `all` false. Shutdown calls it with `all` true, which also writes the bins still open.
///
/// # Errors
///
/// Returns an error when a chunk of rows cannot be written after the retries.
pub(crate) fn flush(factory: &ExecutionContextFactory, all: bool) -> Result<(), Error> {
    let (hits, bins) = factory.artifact_hits().take(all);
    if hits.is_empty() && bins.is_empty() {
        return Ok(());
    }
    let Some(facade) = factory.entity() else {
        return Ok(());
    };
    let (ip, host) = server_address();
    let mut rows: Vec<(&str, BTreeMap<String, Value>)> = Vec::with_capacity(hits.len() + bins.len());
    for bin in bins {
        rows.push((HIT_BIN, bin_fields(bin, ip, host)));
    }
    for hit in hits {
        rows.push((HIT, hit_fields(hit, ip, host)));
    }
    let mut failed = None;
    for chunk in rows.chunks(MAX_CREATES) {
        let mut last = None;
        for _ in 0..FLUSH_RETRIES {
            match write_chunk(&facade, chunk) {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(err) => {
                    tracing::error!("Error saving ArtifactHits, retrying: {err}");
                    last = Some(err);
                }
            }
        }
        failed = failed.or(last);
    }
    failed.map_or(Ok(()), Err)
}

/// One transaction on a reserve connection for a chunk of rows. Ids come from the sequence first,
/// which takes its own reserve connection.
fn write_chunk(facade: &EntityFacade, chunk: &[(&str, BTreeMap<String, Value>)]) -> Result<(), Error> {
    let mut statements = Vec::with_capacity(chunk.len());
    for (entity_name, fields) in chunk {
        let mut value = facade.make_value(entity_name)?;
        for (name, field) in fields {
            value.set(name.clone(), field.clone());
        }
        value.set_sequenced_id_primary(facade)?;
        let plain = facade.normalize_values(entity_name, value.fields().clone(), false)?;
        let names: Vec<&str> = plain.keys().map(|name| &**name).collect();
        let sql = facade.with_entity(entity_name, |entity, dialect, _| sql_insert(entity, dialect, &names))?;
        statements.push((sql, plain.into_values().collect::<Vec<_>>()));
    }
    let tx = facade.transactions_for(HIT_BIN)?;
    crate::entity::exec::with_reserve_tx(&tx, |conn| {
        for (sql, params) in &statements {
            conn.execute(sql, params)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// A decimal with 6 digits after the point, as Java's `setScale(6, HALF_UP)`.
fn decimal(value: f64) -> Value {
    moqui_lib::object::basic_convert(&Value::string(format!("{value:.6}")), "java.math.BigDecimal").value
}

fn optional(text: Option<String>) -> Value {
    text.map_or(Value::Null, Value::string)
}

fn bin_fields(row: BinRow, ip: &str, host: &str) -> BTreeMap<String, Value> {
    let bin = row.bin;
    let mut fields = BTreeMap::new();
    fields.insert("artifactType".to_string(), Value::string(row.artifact_type));
    fields.insert("artifactSubType".to_string(), optional(row.sub_type));
    fields.insert("artifactName".to_string(), Value::string(row.name));
    fields.insert("binStartDateTime".to_string(), stamp_value(bin.start_ms));
    fields.insert("binEndDateTime".to_string(), stamp_value(row.end_ms));
    fields.insert("hitCount".to_string(), Value::I64(bin.hit_count as i64));
    fields.insert("totalTimeMillis".to_string(), decimal(bin.total_time));
    fields.insert("totalSquaredTime".to_string(), decimal(bin.total_squared));
    fields.insert("minTimeMillis".to_string(), decimal(bin.min_time));
    fields.insert("maxTimeMillis".to_string(), decimal(bin.max_time));
    fields.insert("slowHitCount".to_string(), Value::I64(bin.slow_hit_count as i64));
    fields.insert("serverIpAddress".to_string(), Value::string(ip));
    fields.insert("serverHostName".to_string(), Value::string(host));
    fields
}

fn hit_fields(row: HitRow, ip: &str, host: &str) -> BTreeMap<String, Value> {
    let HitContext {
        visit_id,
        user_id,
        errors,
        request_url,
        referrer_url,
    } = row.context;
    let mut fields = BTreeMap::new();
    fields.insert("visitId".to_string(), optional(visit_id));
    fields.insert("userId".to_string(), optional(user_id));
    fields.insert(
        "isSlowHit".to_string(),
        Value::string(if row.is_slow { "Y" } else { "N" }),
    );
    fields.insert("artifactType".to_string(), Value::string(row.artifact_type));
    fields.insert("artifactSubType".to_string(), optional(row.sub_type));
    fields.insert("artifactName".to_string(), Value::string(row.name));
    fields.insert("startDateTime".to_string(), stamp_value(row.start_ms));
    fields.insert("runningTimeMillis".to_string(), decimal(row.running_ms));
    if let Some(parameters) = row.parameter_string {
        fields.insert("parameterString".to_string(), Value::string(parameters));
    }
    if let Some(size) = row.output_size {
        fields.insert("outputSize".to_string(), Value::I64(size));
    }
    if errors.is_empty() {
        fields.insert("wasError".to_string(), Value::string("N"));
    } else {
        let joined: String = errors.iter().map(|error| format!("{error};")).collect();
        fields.insert("wasError".to_string(), Value::string("Y"));
        fields.insert("errorMessage".to_string(), Value::string(truncate(&joined, 255)));
    }
    if let Some(url) = request_url.filter(|url| !url.is_empty()) {
        fields.insert("requestUrl".to_string(), Value::string(truncate(&url, 255)));
    }
    if let Some(url) = referrer_url.filter(|url| !url.is_empty()) {
        fields.insert("referrerUrl".to_string(), Value::string(url));
    }
    fields.insert("serverIpAddress".to_string(), Value::string(ip));
    fields.insert("serverHostName".to_string(), Value::string(host));
    fields
}

/// Releases the single-flight claim when the flush thread ends, also by a panic.
struct FlushClaim(Arc<ExecutionContextFactory>);

impl Drop for FlushClaim {
    fn drop(&mut self) {
        self.0.artifact_hits().flushing.store(false, Ordering::Release);
    }
}

/// Start a flush on a background thread when none is running, for [`ArtifactHits::count`] reporting a full queue.
/// It writes as the scheduler tick does, on the reserve connection. The thread holds the factory only while it
/// flushes, and a failure or panic is logged, not raised. None starts once shutdown began: its final flush writes
/// what is waiting.
pub(crate) fn spawn_flush(factory: &Arc<ExecutionContextFactory>) {
    if factory.is_shutting_down() || !factory.artifact_hits().begin_flush() {
        return;
    }
    let weak = Arc::downgrade(factory);
    let spawned = std::thread::Builder::new()
        .name("artifact-hit-flush".to_string())
        .spawn(move || {
            let Some(factory) = weak.upgrade() else {
                return;
            };
            let claim = FlushClaim(factory);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| flush(&claim.0, false)));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::warn!("artifact hit flush failed: {err}"),
                Err(_) => tracing::warn!("artifact hit flush panicked"),
            }
        });
    match spawned {
        Ok(handle) => {
            let mut threads = lock(&factory.artifact_hits().flush_threads);
            threads.retain(|thread| !thread.is_finished());
            threads.push(handle);
        }
        Err(err) => {
            // The closure never ran, so its claim was never made: release ours.
            factory.artifact_hits().flushing.store(false, Ordering::Release);
            tracing::warn!("artifact hit flush thread did not start: {err}");
        }
    }
}

#[cfg(test)]
#[path = "hits_tests.rs"]
mod tests;
