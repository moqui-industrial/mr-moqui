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

//! Cluster membership, the shared certificate, and fan-out of the two in-process topics.
//! The HTTPS listener and the peer client are installed later. This wave posts through [`Sender`].
//!
//! Fan-out is not done on the thread that published: the message goes on a queue, and one sender thread posts it to
//! the live peers, as `plans/Cluster.md` says. Invalidations waiting together are sent once per entity, since a peer
//! clears the entity's whole cache for one. The peer list is kept and read again on the heartbeat tick.

#[cfg(all(test, feature = "cluster-tls"))]
mod tests;

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use moqui_db::sql::{
    delete as sql_delete, insert as sql_insert, select as sql_select, update as sql_update, Condition, FieldRef,
    LimitOffset,
};
use moqui_lib::env;
use moqui_lib::object::Value;
use sha2::{Digest, Sha256};

use crate::context::{Error, ExecutionContext, ExecutionContextFactory};
use crate::driver::DbRow;
use crate::entity::exec::with_reserve_tx;
use crate::entity::EntityFacade;
use crate::notify::{self, Listener, TopicMessage, ENTITY_CACHE_INVALIDATE, NOTIFICATION_MESSAGE};
use crate::user::{now_millis, now_value, value_millis};

const MEMBER: &str = "moqui.server.ClusterMember";
const MEMBER_COLUMNS: &[&str] = &[
    "instanceId",
    "clusterBaseUrl",
    "heartbeat",
    "sessionKeyFingerprint",
    "certFingerprint",
];
const DEFAULT_PORT: u16 = 8443;
const RETRIES: u32 = 3;
/// Waits between tries of one post: a busy peer answers 503 and is given a moment.
const RETRY_WAITS: [Duration; 2] = [Duration::from_millis(100), Duration::from_millis(500)];
/// Messages waiting for the sender thread. Past this a message is dropped and counted; an invalidation is safe to
/// lose this way, since every peer still drops its own cache on its own writes and on the next invalidation.
const OUTBOX_LIMIT: usize = 10_000;
/// A peer whose post failed every try is skipped this long, so a peer that is down does not slow every message.
const PEER_BACKOFF: Duration = Duration::from_secs(5);
/// A member row with no heartbeat in this many liveness windows is deleted.
const DEAD_WINDOWS: i64 = 10;

thread_local! {
    static APPLYING: Cell<bool> = const { Cell::new(false) };
}

static SENDER: Mutex<Option<Arc<dyn Sender>>> = Mutex::new(None);
static NO_SENDER_LOGGED: AtomicBool = AtomicBool::new(false);
static FACTORY: Mutex<Weak<ExecutionContextFactory>> = Mutex::new(Weak::new());
static LISTENERS: OnceLock<Listeners> = OnceLock::new();
static OUTBOX: OnceLock<Outbox> = OnceLock::new();
static DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Batches the sender thread has taken and not finished, so a test can wait for the posts.
static IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The messages waiting for the sender thread, and the peers it posts to.
struct Outbox {
    queue: Mutex<std::collections::VecDeque<(&'static str, TopicMessage)>>,
    ready: std::sync::Condvar,
    peers: Mutex<PeerCache>,
}

#[derive(Default)]
struct PeerCache {
    urls: Vec<String>,
    /// When the list was read, by the clock of `now_millis`; zero is never.
    read_at: i64,
    /// Peers to skip until the time given, after every try of a post failed.
    down_until: std::collections::HashMap<String, std::time::Instant>,
}

struct Listeners {
    _cache: notify::Subscription,
    _notice: notify::Subscription,
}

struct Applying;

impl Applying {
    fn enter() -> Self {
        APPLYING.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for Applying {
    fn drop(&mut self) {
        APPLYING.with(|flag| flag.set(false));
    }
}

fn applying() -> bool {
    APPLYING.with(Cell::get)
}

/// Posts one topic message to a peer. The host installs the HTTPS client. Tests install a recorder.
pub trait Sender: Send + Sync {
    /// `base_url` has no trailing slash. `topic` is the topic name, not a path.
    fn post(&self, base_url: &str, topic: &str, payload: &[u8]) -> Result<(), Error>;
}

/// Replace the process sender. A later call replaces the previous one.
pub fn install_sender(sender: Arc<dyn Sender>) {
    *sender_slot() = Some(sender);
    NO_SENDER_LOGGED.store(false, Ordering::Relaxed);
}

/// Register this instance, check live peers, and subscribe once to both topics.
///
/// The host calls this after [`ExecutionContextFactory::set_entity`]. A second call refreshes the
/// member row and does not add another listener or heartbeat hook.
///
/// # Errors
///
/// Returns an error when the entity facade is missing, the certificate pair does not match, or a
/// live peer's fingerprint differs.
pub fn start(factory: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
    let facade = factory
        .entity()
        .ok_or_else(|| Error::new("the entity facade is not installed"))?;
    let instance_id = ensure_instance_id(factory)?;
    let fingerprints = configured_fingerprints()?;
    let base_url = cluster_base_url()?;
    reject_peer_mismatch(&facade, factory, &instance_id, &fingerprints)?;
    write_member(
        &facade,
        &Member {
            instance_id: instance_id.clone(),
            base_url,
            heartbeat: millis_value(db_now(&facade)),
            session_fp: fingerprints.session,
            cert_fp: fingerprints.cert,
        },
    )?;
    remember_factory(factory);
    // The member list is read again on the next send: this start may have joined a cluster or left one.
    *outbox().peers.lock().unwrap_or_else(|err| err.into_inner()) = PeerCache::default();
    install_heartbeat(factory, instance_id);
    install_listeners()
}

/// Wait until the sender thread has posted everything queued, for tests.
#[cfg(all(test, feature = "cluster-tls"))]
pub(crate) fn wait_for_sends(timeout: Duration) -> bool {
    let until = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < until {
        let empty = outbox().queue.lock().unwrap_or_else(|err| err.into_inner()).is_empty();
        if empty && IN_FLIGHT.load(Ordering::SeqCst) == 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Clear the named entity's one-cache for `entity-cache-invalidate`, then publish `message`.
/// The fan-out listener does not post this publish onward.
///
/// # Errors
///
/// Returns an error when `topic` is unknown, or the cache body has no entity name.
pub fn apply_remote(facade: &EntityFacade, topic: &str, message: &TopicMessage) -> Result<(), Error> {
    if topic == ENTITY_CACHE_INVALIDATE {
        let name = message
            .body
            .get("entityName")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| Error::new("entity-cache-invalidate body has no entityName"))?;
        crate::entity::cache::clear_one_cache(facade, name).map_err(Error::from)?;
    } else if topic != NOTIFICATION_MESSAGE {
        return Err(Error::new(format!("unknown topic {topic}")));
    }
    let _applying = Applying::enter();
    notify::publish(topic, message)
}

/// Publish a cache invalidation after the local clear. An open transaction waits for commit.
pub(crate) fn note_write(
    facade: &EntityFacade,
    entity_name: &str,
    pk_fields: &[String],
    values: &std::collections::BTreeMap<String, Value>,
) -> Result<(), Error> {
    let message = TopicMessage {
        origin_id: notify::origin_id().to_string(),
        message_id: crate::entity::uuid_v4().map_err(Error::from)?,
        body: invalidation_body(entity_name, pk_fields, values),
    };
    let tx = facade.transactions_for(entity_name).map_err(Error::from)?;
    if tx.in_transaction() {
        tx.on_commit(move || {
            if let Err(err) = notify::publish(ENTITY_CACHE_INVALIDATE, &message) {
                tracing::warn!("cluster cache publish failed: {err}");
            }
        })
        .map_err(Error::from)?;
        return Ok(());
    }
    notify::publish(ENTITY_CACHE_INVALIDATE, &message)
}

/// One self-signed pair. Both values are standard base64 of DER, one line each.
#[cfg(feature = "cluster-tls")]
pub(crate) fn generate_cert_pair() -> Result<(String, String), Error> {
    let key = rcgen::KeyPair::generate().map_err(cert_err)?;
    let mut params = rcgen::CertificateParams::new(vec!["moqui-cluster".to_string()]).map_err(cert_err)?;
    let mut name = rcgen::DistinguishedName::new();
    name.push(rcgen::DnType::CommonName, "moqui-cluster");
    params.distinguished_name = name;
    let today = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let until = today
        .checked_add(jiff::Span::new().years(10))
        .map_err(|err| Error::new(format!("could not generate cluster tls material: {err}")))?;
    params.not_before = rcgen::date_time_ymd(ymd_year(today), ymd_month(today), ymd_day(today));
    params.not_after = rcgen::date_time_ymd(ymd_year(until), ymd_month(until), ymd_day(until));
    let cert = params.self_signed(&key).map_err(cert_err)?;
    let cert_b64 = crate::context::standard_base64(cert.der().as_ref());
    let key_b64 = crate::context::standard_base64(key.serialized_der());
    Ok((cert_b64, key_b64))
}

#[cfg(feature = "cluster-tls")]
fn ymd_year(date: jiff::civil::Date) -> i32 {
    i32::from(date.year())
}

#[cfg(feature = "cluster-tls")]
fn ymd_month(date: jiff::civil::Date) -> u8 {
    u8::try_from(date.month()).unwrap_or(1)
}

#[cfg(feature = "cluster-tls")]
fn ymd_day(date: jiff::civil::Date) -> u8 {
    u8::try_from(date.day()).unwrap_or(1)
}

#[cfg(feature = "cluster-tls")]
fn cert_err(err: impl std::fmt::Display) -> Error {
    Error::new(format!("could not generate cluster tls material: {err}"))
}

struct Fingerprints {
    session: String,
    cert: String,
}

struct Member {
    instance_id: String,
    base_url: String,
    heartbeat: Value,
    session_fp: String,
    cert_fp: String,
}

/// `instance_id` when it is set, else the id in `runtime/db/instance_id`, else a new one written there. It is not
/// written to `runtime/.env`, which `plans/Cluster.md` says to copy between members: two members with one id would
/// each take the other's member row for its own.
fn ensure_instance_id(factory: &ExecutionContextFactory) -> Result<String, Error> {
    let current = env::get_prop_or_env("instance_id");
    if !current.is_empty() {
        return Ok(current);
    }
    let file = factory.runtime_dir().join("db").join("instance_id");
    if let Ok(text) = std::fs::read_to_string(&file) {
        let kept = text.trim().to_string();
        if !kept.is_empty() {
            env::set_property("instance_id", kept.clone());
            return Ok(kept);
        }
    }
    let id = crate::entity::uuid_v4().map_err(Error::from)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| Error::new(format!("could not make {}: {err}", parent.display())))?;
    }
    std::fs::write(&file, format!("{id}\n"))
        .map_err(|err| Error::new(format!("could not write {}: {err}", file.display())))?;
    env::set_property("instance_id", id.clone());
    Ok(id)
}

fn configured_fingerprints() -> Result<Fingerprints, Error> {
    let session_key = require_configured("session_cookie_key")?;
    let cert_b64 = require_configured("cluster_tls_cert")?;
    let _key = require_configured("cluster_tls_key")?;
    let der = decode_der("cluster_tls_cert", &cert_b64)?;
    Ok(Fingerprints {
        session: sha256_hex(session_key.as_bytes()),
        cert: sha256_hex(&der),
    })
}

fn require_configured(name: &str) -> Result<String, Error> {
    let value = env::get_prop_or_env(name);
    if value.is_empty() || value.ends_with("CHANGEME") {
        return Err(Error::new(format!("{name} is not configured")));
    }
    Ok(value)
}

fn decode_der(name: &str, text: &str) -> Result<Vec<u8>, Error> {
    STANDARD
        .decode(text.trim())
        .map_err(|_| Error::new(format!("{name} is not base64 der")))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let hex = b"0123456789abcdef";
        out.push(hex[(byte >> 4) as usize] as char);
        out.push(hex[(byte & 0x0f) as usize] as char);
    }
    out
}

fn cluster_base_url() -> Result<String, Error> {
    let configured = env::get_prop_or_env("cluster_base_url");
    let url = if configured.is_empty() {
        format!("https://127.0.0.1:{}", cluster_port()?)
    } else {
        configured
    };
    Ok(trim_base(&url))
}

fn cluster_port() -> Result<u16, Error> {
    let raw = env::get_prop_or_env("cluster_https_port");
    if raw.is_empty() {
        return Ok(DEFAULT_PORT);
    }
    raw.parse().map_err(|_| Error::new("cluster_https_port is not a port"))
}

fn trim_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn window_millis(factory: &ExecutionContextFactory) -> i64 {
    let millis = i64::try_from(factory.service_pools().check_interval().as_millis()).unwrap_or(60_000);
    millis.saturating_mul(3)
}

fn is_live(heartbeat: Option<i64>, now: i64, window: i64) -> bool {
    let Some(stamp) = heartbeat else {
        return false;
    };
    now.saturating_sub(stamp) <= window
}

fn reject_peer_mismatch(
    facade: &EntityFacade,
    factory: &ExecutionContextFactory,
    instance_id: &str,
    fingerprints: &Fingerprints,
) -> Result<(), Error> {
    let now = db_now(facade);
    let window = window_millis(factory);
    for member in load_members(facade)? {
        if member.instance_id == instance_id || !is_live(member.heartbeat, now, window) {
            continue;
        }
        if member.session_fp != fingerprints.session {
            return Err(Error::new("session_cookie_key does not match a live cluster member"));
        }
        if member.cert_fp != fingerprints.cert {
            return Err(Error::new("cluster_tls_cert does not match a live cluster member"));
        }
    }
    Ok(())
}

fn remember_factory(factory: &Arc<ExecutionContextFactory>) {
    *factory_slot() = Arc::downgrade(factory);
}

fn install_heartbeat(factory: &Arc<ExecutionContextFactory>, instance_id: String) {
    if factory.cluster_heartbeat.swap(true, Ordering::AcqRel) {
        return;
    }
    let weak = Arc::downgrade(factory);
    factory.service_pools().add_tick_hook(move || {
        let Some(factory) = weak.upgrade() else {
            return;
        };
        let Some(facade) = factory.entity() else {
            tracing::warn!("cluster heartbeat has no entity facade");
            return;
        };
        // The database's clock, so members whose clocks differ still see each other as live.
        let now = db_now(&facade);
        if let Err(err) = touch_member(&facade, &instance_id, now) {
            tracing::warn!("cluster heartbeat failed: {err}");
        }
        if let Err(err) = delete_dead_members(&facade, now.saturating_sub(window_millis(&factory) * DEAD_WINDOWS)) {
            tracing::warn!("cluster member cleanup failed: {err}");
        }
        refresh_peers(&factory);
        warn_loopback_peers(&factory);
    });
}

fn install_listeners() -> Result<(), Error> {
    if LISTENERS.get().is_some() {
        return Ok(());
    }
    let installed = Listeners {
        _cache: notify::subscribe(ENTITY_CACHE_INVALIDATE, listen(ENTITY_CACHE_INVALIDATE))?,
        _notice: notify::subscribe(NOTIFICATION_MESSAGE, listen(NOTIFICATION_MESSAGE))?,
    };
    let _ = LISTENERS.set(installed);
    Ok(())
}

fn listen(topic: &'static str) -> Listener {
    Arc::new(move |message| fan_out(topic, message))
}

fn fan_out(topic: &'static str, message: &TopicMessage) {
    // Remote messages carry the sender's origin and are not posted again. apply_remote republishes
    // on this process, so the applying flag stops that same-process re-post.
    if applying() || message.origin_id != notify::origin_id() {
        return;
    }
    let outbox = outbox();
    {
        // A single member has no one to tell: with no peers at the last read, nothing is queued.
        let peers = outbox.peers.lock().unwrap_or_else(|err| err.into_inner());
        if peers.read_at != 0 && peers.urls.is_empty() {
            return;
        }
    }
    let mut queue = outbox.queue.lock().unwrap_or_else(|err| err.into_inner());
    if queue.len() >= OUTBOX_LIMIT {
        let dropped = DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
        if dropped.is_power_of_two() {
            tracing::warn!("cluster fan-out queue is full; {dropped} messages dropped");
        }
        return;
    }
    queue.push_back((topic, message.clone()));
    outbox.ready.notify_one();
}

fn outbox() -> &'static Outbox {
    OUTBOX.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("moqui-cluster-send".to_string())
            .spawn(send_loop);
        if let Err(err) = spawned {
            tracing::error!("cluster sender thread did not start: {err}");
        }
        Outbox {
            queue: Mutex::new(std::collections::VecDeque::new()),
            ready: std::sync::Condvar::new(),
            peers: Mutex::new(PeerCache::default()),
        }
    })
}

/// The sender thread: take what is waiting, collapse the invalidations, and post each to every live peer.
fn send_loop() {
    loop {
        let batch: Vec<(&'static str, TopicMessage)> = {
            let outbox = outbox();
            let mut queue = outbox.queue.lock().unwrap_or_else(|err| err.into_inner());
            while queue.is_empty() {
                queue = outbox.ready.wait(queue).unwrap_or_else(|err| err.into_inner());
            }
            IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
            queue.drain(..).collect()
        };
        // A panic in one batch must not end the thread that every later message waits on.
        if std::panic::catch_unwind(|| send_batch(coalesce(batch))).is_err() {
            tracing::error!("cluster fan-out batch panicked");
        }
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn send_batch(batch: Vec<(&'static str, TopicMessage)>) {
    let Some(factory) = factory_slot().upgrade() else {
        return;
    };
    let peers = current_peers(&factory);
    if peers.is_empty() {
        return;
    }
    let Some(sender) = sender_slot().clone() else {
        if !NO_SENDER_LOGGED.swap(true, Ordering::Relaxed) {
            tracing::warn!("cluster fan-out has no sender");
        }
        return;
    };
    for (topic, message) in &batch {
        let payload = match payload_bytes(message) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!("cluster fan-out could not encode a message: {err}");
                continue;
            }
        };
        for base_url in &peers {
            if peer_down(base_url) {
                continue;
            }
            if !post_with_retry(&sender, base_url, topic, &payload) {
                mark_peer_down(base_url);
            }
        }
    }
}

/// One invalidation for each entity, in the place of the first: a peer clears the entity's cache whatever the key.
/// Notifications are all kept, in order.
fn coalesce(batch: Vec<(&'static str, TopicMessage)>) -> Vec<(&'static str, TopicMessage)> {
    let mut seen = std::collections::HashSet::new();
    batch
        .into_iter()
        .filter(|(topic, message)| {
            *topic != ENTITY_CACHE_INVALIDATE
                || seen.insert(
                    message
                        .body
                        .get("entityName")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                )
        })
        .collect()
}

/// The live peers, read again when the heartbeat tick has not refreshed them within a liveness window.
fn current_peers(factory: &ExecutionContextFactory) -> Vec<String> {
    let window = window_millis(factory);
    {
        let peers = outbox().peers.lock().unwrap_or_else(|err| err.into_inner());
        if peers.read_at != 0 && now_millis().saturating_sub(peers.read_at) < window {
            return peers.urls.clone();
        }
    }
    refresh_peers(factory);
    outbox().peers.lock().unwrap_or_else(|err| err.into_inner()).urls.clone()
}

fn refresh_peers(factory: &ExecutionContextFactory) {
    let Some(facade) = factory.entity() else {
        return;
    };
    match live_peer_urls(factory, &facade) {
        Ok(urls) => {
            let mut peers = outbox().peers.lock().unwrap_or_else(|err| err.into_inner());
            peers.urls = urls;
            peers.read_at = now_millis();
        }
        Err(err) => tracing::warn!("cluster fan-out could not load members: {err}"),
    }
}

fn peer_down(base_url: &str) -> bool {
    let mut peers = outbox().peers.lock().unwrap_or_else(|err| err.into_inner());
    match peers.down_until.get(base_url) {
        Some(until) if std::time::Instant::now() < *until => true,
        Some(_) => {
            peers.down_until.remove(base_url);
            false
        }
        None => false,
    }
}

fn mark_peer_down(base_url: &str) {
    outbox()
        .peers
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .down_until
        .insert(base_url.to_string(), std::time::Instant::now() + PEER_BACKOFF);
}

fn live_peer_urls(factory: &ExecutionContextFactory, facade: &EntityFacade) -> Result<Vec<String>, Error> {
    let instance_id = env::get_prop_or_env("instance_id");
    let now = db_now(facade);
    let window = window_millis(factory);
    let mut bases = Vec::new();
    for member in load_members(facade)? {
        if member.instance_id == instance_id || !is_live(member.heartbeat, now, window) {
            continue;
        }
        let base = trim_base(&member.base_url);
        if !base.is_empty() {
            bases.push(base);
        }
    }
    Ok(bases)
}

fn payload_bytes(message: &TopicMessage) -> Result<Vec<u8>, Error> {
    let value = serde_json::json!({
        "origin_id": message.origin_id,
        "message_id": message.message_id,
        "body": message.body,
    });
    serde_json::to_vec(&value).map_err(|err| Error::new(format!("could not encode cluster message: {err}")))
}

/// Post, trying again after a short and then a longer wait. False when every try failed.
fn post_with_retry(sender: &Arc<dyn Sender>, base_url: &str, topic: &str, payload: &[u8]) -> bool {
    for attempt in 0..RETRIES {
        match sender.post(base_url, topic, payload) {
            Ok(()) => return true,
            Err(err) if attempt + 1 == RETRIES => {
                tracing::warn!("cluster post to {base_url} failed for topic {topic}: {err}");
            }
            Err(_) => std::thread::sleep(RETRY_WAITS[usize::try_from(attempt).unwrap_or(0).min(RETRY_WAITS.len() - 1)]),
        }
    }
    false
}

fn invalidation_body(
    entity_name: &str,
    pk_fields: &[String],
    values: &std::collections::BTreeMap<String, Value>,
) -> serde_json::Value {
    let mut primary_key = serde_json::Map::new();
    for name in pk_fields {
        if let Some(value) = values.get(name).filter(|value| !value.is_null()) {
            primary_key.insert(name.clone(), moqui_lib::json::to_json(value));
        }
    }
    serde_json::json!({
        "entityName": entity_name,
        "primaryKey": primary_key,
    })
}

struct LoadedMember {
    instance_id: String,
    base_url: String,
    heartbeat: Option<i64>,
    session_fp: String,
    cert_fp: String,
}

fn load_members(facade: &EntityFacade) -> Result<Vec<LoadedMember>, Error> {
    let fields: Vec<FieldRef> = MEMBER_COLUMNS.iter().copied().map(FieldRef::new).collect();
    let sql = facade
        .with_entity(MEMBER, |entity, dialect, _| {
            sql_select(entity, dialect, &fields, None, &[], LimitOffset::default()).map(|statement| statement.sql)
        })
        .map_err(Error::from)?;
    let tx = facade.transactions_for(MEMBER).map_err(Error::from)?;
    let rows = with_reserve_tx(&tx, |conn| conn.query(&sql, &[])).map_err(Error::from)?;
    Ok(rows.iter().map(member_from_row).collect())
}

fn member_from_row(row: &DbRow) -> LoadedMember {
    LoadedMember {
        instance_id: row_text(row, "instanceId"),
        base_url: row_text(row, "clusterBaseUrl"),
        heartbeat: row.get("heartbeat").and_then(value_millis),
        session_fp: row_text(row, "sessionKeyFingerprint"),
        cert_fp: row_text(row, "certFingerprint"),
    }
}

fn row_text(row: &DbRow, name: &str) -> String {
    match row.get(name) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => moqui_lib::object::to_plain_string(other),
    }
}

fn write_member(facade: &EntityFacade, member: &Member) -> Result<(), Error> {
    let _authz = entered_authz();
    let fields: Vec<FieldRef> = MEMBER_COLUMNS.iter().copied().map(FieldRef::new).collect();
    let by_id = Condition::equals("instanceId", Value::string(&member.instance_id));
    let (select_sql, insert_sql, update_sql) = facade
        .with_entity(MEMBER, |entity, dialect, _| {
            let select_sql = sql_select(entity, dialect, &fields, Some(&by_id), &[], LimitOffset::default())?.sql;
            let insert_sql = sql_insert(entity, dialect, MEMBER_COLUMNS)?;
            let update_sql = sql_update(
                entity,
                dialect,
                &[
                    "clusterBaseUrl",
                    "heartbeat",
                    "sessionKeyFingerprint",
                    "certFingerprint",
                ],
                Some(&by_id),
            )?
            .sql;
            Ok((select_sql, insert_sql, update_sql))
        })
        .map_err(Error::from)?;
    let instance = Value::string(&member.instance_id);
    let insert_params = vec![
        instance.clone(),
        Value::string(&member.base_url),
        member.heartbeat.clone(),
        Value::string(&member.session_fp),
        Value::string(&member.cert_fp),
    ];
    let update_params = vec![
        Value::string(&member.base_url),
        member.heartbeat.clone(),
        Value::string(&member.session_fp),
        Value::string(&member.cert_fp),
        instance.clone(),
    ];
    let tx = facade.transactions_for(MEMBER).map_err(Error::from)?;
    with_reserve_tx(&tx, |conn| {
        let rows = conn.query(&select_sql, &[instance])?;
        if rows.is_empty() {
            conn.execute(&insert_sql, &insert_params)?;
        } else {
            conn.execute(&update_sql, &update_params)?;
        }
        Ok(())
    })
    .map_err(Error::from)
}

/// The database server's time in epoch milliseconds, else this host's.
fn db_now(facade: &EntityFacade) -> i64 {
    let read = facade.transactions_for(MEMBER).map_err(Error::from).and_then(|tx| {
        with_reserve_tx(&tx, |conn| conn.query("SELECT CURRENT_TIMESTAMP", &[])).map_err(Error::from)
    });
    read.ok()
        .and_then(|rows| rows.first().and_then(|row| row.columns.first().map(|(_, value)| value.clone())))
        .and_then(|value| value_millis(&value))
        .unwrap_or_else(now_millis)
}

fn millis_value(millis: i64) -> Value {
    jiff::Timestamp::from_millisecond(millis).map_or_else(|_| now_value(), Value::Timestamp)
}

/// Remove the rows of members that stopped long ago, so the table does not keep every instance that ever ran.
fn delete_dead_members(facade: &EntityFacade, before: i64) -> Result<(), Error> {
    let _authz = entered_authz();
    let condition = Condition::less("heartbeat", millis_value(before));
    let statement = facade
        .with_entity(MEMBER, |entity, dialect, _| sql_delete(entity, dialect, Some(&condition)))
        .map_err(Error::from)?;
    let tx = facade.transactions_for(MEMBER).map_err(Error::from)?;
    with_reserve_tx(&tx, |conn| {
        conn.execute(&statement.sql, &statement.params)?;
        Ok(())
    })
    .map_err(Error::from)
}

/// A live member whose base URL is a loopback address cannot be reached from another host: say so once, since the
/// fan-out to it fails without a sign otherwise.
fn warn_loopback_peers(factory: &ExecutionContextFactory) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let peers = outbox().peers.lock().unwrap_or_else(|err| err.into_inner()).urls.clone();
    if peers.is_empty() || WARNED.load(Ordering::Relaxed) {
        return;
    }
    let own = cluster_base_url().unwrap_or_default();
    let loopback = |url: &str| url.contains("://127.") || url.contains("://localhost") || url.contains("://[::1]");
    if loopback(&own) || peers.iter().any(|url| loopback(url)) {
        WARNED.store(true, Ordering::Relaxed);
        let _ = factory;
        tracing::error!(
            "a cluster member has a loopback cluster_base_url ({own}, peers {peers:?}); set cluster_base_url on each \
             member to an address the others can reach"
        );
    }
}

fn touch_member(facade: &EntityFacade, instance_id: &str, now: i64) -> Result<(), Error> {
    let _authz = entered_authz();
    let condition = Condition::equals("instanceId", Value::string(instance_id));
    let sql = facade
        .with_entity(MEMBER, |entity, dialect, _| {
            sql_update(entity, dialect, &["heartbeat"], Some(&condition)).map(|statement| statement.sql)
        })
        .map_err(Error::from)?;
    let params = [millis_value(now), Value::string(instance_id)];
    let tx = facade.transactions_for(MEMBER).map_err(Error::from)?;
    with_reserve_tx(&tx, |conn| {
        conn.execute(&sql, &params)?;
        Ok(())
    })
    .map_err(Error::from)
}

fn entered_authz() -> Option<crate::context::AuthzGuard> {
    ExecutionContext::with_current(ExecutionContext::disable_authz)
}

fn sender_slot() -> std::sync::MutexGuard<'static, Option<Arc<dyn Sender>>> {
    SENDER.lock().unwrap_or_else(|err| err.into_inner())
}

fn factory_slot() -> std::sync::MutexGuard<'static, Weak<ExecutionContextFactory>> {
    FACTORY.lock().unwrap_or_else(|err| err.into_inner())
}
