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

//! Web protection at the edge, the `web-protection` element of the `webroot` webapp: per-client connection caps,
//! rate limits and bans, and client, method, path and user-agent rules, checked on the connection's task before a
//! session, a request worker or the database is spent. In memory and per node. See
//! `plans/proposals/Web-Protection.md`.
//!
//! This is not the ArtifactTarpit, which counts a user's use of screens and services inside a worker, after login
//! and authorization, for business quotas. The two apply together, this one first.
//!
//! The per-client parts are on when `enabled` is `true`, or when it is empty and `trusted-proxies` names a proxy;
//! without a proxy list a deployment behind a proxy would see every client as the proxy. The global limits of
//! [`crate::http::ListenLimits`] and the overload answers apply either way.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use axum::response::Response;
use moqui_core::web::address::{masked, parse_cidrs, Cidr, ClientAddressing};
use moqui_lib::node::MNode;

const SHARDS: usize = 32;
const DEFAULT_MAX_PER_CLIENT: usize = 64;
const DEFAULT_MAX_TRACKED: usize = 100_000;
const DEFAULT_RETRY_AFTER: u64 = 5;
const DEFAULT_QUEUE_TIMEOUT: u64 = 10;
/// The shortest time an idle client stays in the table.
const MIN_IDLE: Duration = Duration::from_secs(60);

/// What can count toward a [`Ban`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// Credentials that were not valid: a login form, Basic auth, an API or login key, git.
    AuthFailure,
    /// A 429 from a rate limit.
    RateLimited,
    /// A client, method, path or user-agent rule refused the request.
    Denied,
}

impl Event {
    fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "auth-failure" => Some(Self::AuthFailure),
            "rate-limited" => Some(Self::RateLimited),
            "denied" => Some(Self::Denied),
            _ => None,
        }
    }
}

/// A `rate-limit`: `requests` per `seconds` sustained, `burst` at once, for each client, over the requests its
/// `path` and `methods` match (all of them when left out). GCRA: one arrival time per client and limit.
#[derive(Debug)]
struct RateLimit {
    name: String,
    paths: Vec<Glob>,
    methods: Vec<String>,
    /// Nanoseconds between requests at the sustained rate.
    interval: u64,
    /// How far ahead of now the arrival time may run: `interval * burst`.
    tolerance: u64,
}

impl RateLimit {
    fn applies(&self, method: &str, path: &str) -> bool {
        (self.methods.is_empty() || self.methods.iter().any(|allowed| allowed.eq_ignore_ascii_case(method)))
            && (self.paths.is_empty() || self.paths.iter().any(|glob| glob.matches(path)))
    }
}

/// A `ban`: `threshold` of its `events` within `window` bans the client for `duration`.
#[derive(Debug)]
struct Ban {
    name: String,
    events: Vec<Event>,
    threshold: u32,
    window: u64,
    duration: u64,
}

/// A `url-pattern` style path: `*` within a segment, `**` across any number of them. Matched without regard to ASCII
/// case, against the path with its `%XX` escapes decoded.
#[derive(Clone, Debug)]
struct Glob {
    segments: Vec<String>,
}

impl Glob {
    fn new(pattern: &str) -> Option<Self> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }
        Some(Self {
            segments: pattern
                .trim_start_matches('/')
                .split('/')
                .map(str::to_ascii_lowercase)
                .collect(),
        })
    }

    fn matches(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let segments: Vec<&str> = self.segments.iter().map(String::as_str).collect();
        segments_match(&segments, &parts)
    }
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments_match(rest, &path[skip..])),
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(part, others)| wildcard_match(first, part) && segments_match(rest, others)),
    }
}

/// `*` for any run of characters within one segment.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && pattern[p] != b'*' && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|byte| *byte == b'*')
}

/// `%XX` escapes decoded, a bad one kept as written; `+` is a plus in a path.
fn decode_path(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| char::from(*byte).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push(u8::try_from(high * 16 + low).unwrap_or(b'%'));
                index += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `web-protection` conf, read once when the listener starts.
#[derive(Debug)]
pub(crate) struct GuardConf {
    /// The per-client parts: rules, rate limits, bans, `max-per-client`.
    enabled: bool,
    addressing: Arc<ClientAddressing>,
    ipv6_prefix: u8,
    max_tracked: usize,
    max_per_client: usize,
    methods: Vec<String>,
    /// How long a request may wait in the request pool's queue before it is answered 503 instead of run.
    pub(crate) queue_timeout: Option<Duration>,
    /// The `Retry-After` seconds of a 503.
    pub(crate) retry_after: u64,
    rate_limits: Vec<RateLimit>,
    bans: Vec<Ban>,
    exempt: Vec<Cidr>,
    deny: Vec<Cidr>,
    deny_paths: Vec<Glob>,
    deny_agents: Vec<String>,
}

/// An attribute of `node`'s child `child` (of `node` itself when `child` is empty), `${}` expanded and trimmed, or
/// the property `property` when it is left out or expands to nothing, as `moqui_core::web::protection_setting`.
pub(crate) fn setting(node: Option<&MNode>, child: &str, name: &str, property: &str) -> String {
    let element = match node {
        Some(node) if child.is_empty() => Some(node.clone()),
        Some(node) => node.first_named(child),
        None => None,
    };
    let configured = element
        .and_then(|element| element.attribute(name))
        .map(|raw| moqui_lib::env::expand(&raw).unwrap_or(raw))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if !configured.is_empty() || property.is_empty() {
        return configured;
    }
    moqui_lib::env::get_prop_or_env(property).trim().to_string()
}

/// A positive number from `text`, else `default`; a value that is there and not a number is logged with `what`.
fn number<T: std::str::FromStr + PartialOrd + Default + Copy>(text: &str, default: T, what: &str) -> T {
    if text.is_empty() {
        return default;
    }
    match text.parse::<T>() {
        Ok(value) if value > T::default() => value,
        _ => {
            tracing::error!("web-protection {what}: '{text}' is not a positive number; it is left out");
            default
        }
    }
}

fn list(text: &str) -> impl Iterator<Item = &str> {
    text.split(',').map(str::trim).filter(|item| !item.is_empty())
}

/// `<web-protection>` of the `webroot` webapp in `conf_text`.
pub(crate) fn protection_node(conf_text: &str) -> Option<MNode> {
    let conf = MNode::parse_text("moqui-conf", conf_text).ok()?;
    conf.first_named("webapp-list")?
        .children_named("webapp")
        .into_iter()
        .find(|webapp| webapp.attribute("name").as_deref() == Some("webroot"))?
        .first_named("web-protection")
}

impl GuardConf {
    /// From a `web-protection` element; with none, the properties and defaults (the per-client parts off unless
    /// `addressing` has trusted proxies).
    pub(crate) fn from_node(node: Option<&MNode>, addressing: Arc<ClientAddressing>) -> Self {
        let enabled = match setting(node, "", "enabled", "webapp_web_protection").as_str() {
            "true" => true,
            "false" => false,
            "" => addressing.has_trusted(),
            other => {
                tracing::error!("web-protection enabled: '{other}' is not true or false; it follows trusted-proxies");
                addressing.has_trusted()
            }
        };
        let ipv6_prefix = number(&setting(node, "", "client-ipv6-prefix", ""), 64u8, "client-ipv6-prefix").min(128);
        let max_tracked = number(
            &setting(node, "", "max-tracked-clients", ""),
            DEFAULT_MAX_TRACKED,
            "max-tracked-clients",
        );
        let max_per_client = number(
            &setting(node, "connections", "max-per-client", ""),
            DEFAULT_MAX_PER_CLIENT,
            "connections max-per-client",
        );
        let methods = list(&setting(node, "requests", "methods", ""))
            .map(str::to_ascii_uppercase)
            .collect();
        let queue_timeout = match setting(node, "overload", "queue-timeout", "").as_str() {
            "0" => None,
            text => Some(Duration::from_secs(number(
                text,
                DEFAULT_QUEUE_TIMEOUT,
                "overload queue-timeout",
            ))),
        };
        let retry_after = number(
            &setting(node, "overload", "retry-after", ""),
            DEFAULT_RETRY_AFTER,
            "overload retry-after",
        );
        let mut conf = Self {
            enabled,
            addressing,
            ipv6_prefix,
            max_tracked,
            max_per_client,
            methods,
            queue_timeout,
            retry_after,
            rate_limits: Vec::new(),
            bans: Vec::new(),
            exempt: Vec::new(),
            deny: Vec::new(),
            deny_paths: Vec::new(),
            deny_agents: Vec::new(),
        };
        if let Some(node) = node {
            conf.read_lists(node);
        }
        conf
    }

    fn read_lists(&mut self, node: &MNode) {
        let attr = |element: &MNode, name: &str| {
            element
                .attribute(name)
                .map(|raw| moqui_lib::env::expand(&raw).unwrap_or(raw))
                .map(|value| value.trim().to_string())
                .unwrap_or_default()
        };
        for element in node.children_named("rate-limit") {
            let name = attr(&element, "name");
            if attr(&element, "enabled") == "false" {
                continue;
            }
            match rate_limit(&element, &attr) {
                Some(limit) => self.rate_limits.push(limit),
                None => tracing::error!("web-protection rate-limit '{name}' is not valid; it is left out"),
            }
        }
        for element in node.children_named("ban") {
            let name = attr(&element, "name");
            if attr(&element, "enabled") == "false" {
                continue;
            }
            match ban(&element, &attr) {
                Some(ban) => self.bans.push(ban),
                None => tracing::error!("web-protection ban '{name}' is not valid; it is left out"),
            }
        }
        let rules = |name: &str| {
            node.children_named(name)
                .into_iter()
                .filter(|element| attr(element, "enabled") != "false")
                .collect::<Vec<_>>()
        };
        for element in rules("client-rule") {
            let cidrs = parse_cidrs(&attr(&element, "cidr"), "web-protection client-rule");
            match attr(&element, "action").as_str() {
                "exempt" => self.exempt.extend(cidrs),
                "deny" => self.deny.extend(cidrs),
                other => tracing::error!("web-protection client-rule action '{other}' is not deny or exempt"),
            }
        }
        for element in rules("path-rule") {
            match attr(&element, "action").as_str() {
                "deny" => self
                    .deny_paths
                    .extend(list(&attr(&element, "path")).filter_map(Glob::new)),
                other => tracing::error!("web-protection path-rule action '{other}' is not deny"),
            }
        }
        for element in rules("user-agent-rule") {
            match attr(&element, "action").as_str() {
                "deny" => self
                    .deny_agents
                    .extend(list(&attr(&element, "contains")).map(str::to_ascii_lowercase)),
                other => tracing::error!("web-protection user-agent-rule action '{other}' is not deny"),
            }
        }
    }

    pub(crate) fn addressing(&self) -> &ClientAddressing {
        &self.addressing
    }

    /// The idle time after which a client is dropped from the table: its longest ban window or rate-limit refill.
    fn idle_after(&self) -> u64 {
        let longest_ban = self.bans.iter().map(|ban| ban.window).max().unwrap_or(0);
        let longest_refill = self.rate_limits.iter().map(|limit| limit.tolerance).max().unwrap_or(0);
        longest_ban.max(longest_refill).max(nanos(MIN_IDLE))
    }
}

fn rate_limit(element: &MNode, attr: &impl Fn(&MNode, &str) -> String) -> Option<RateLimit> {
    let requests = attr(element, "requests")
        .parse::<f64>()
        .ok()
        .filter(|count| *count > 0.0)?;
    let seconds = match attr(element, "seconds").as_str() {
        "" => 1.0,
        text => text.parse::<f64>().ok().filter(|seconds| *seconds > 0.0)?,
    };
    let burst = match attr(element, "burst").as_str() {
        "" => requests.ceil(),
        text => text.parse::<f64>().ok().filter(|burst| *burst >= 1.0)?,
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // positive and far below u64::MAX nanoseconds
    let interval = ((seconds * 1e9) / requests).max(1.0) as u64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tolerance = (interval as f64 * burst.floor()) as u64;
    Some(RateLimit {
        name: attr(element, "name"),
        paths: list(&attr(element, "path")).filter_map(Glob::new).collect(),
        methods: list(&attr(element, "methods")).map(str::to_ascii_uppercase).collect(),
        interval,
        tolerance,
    })
}

fn ban(element: &MNode, attr: &impl Fn(&MNode, &str) -> String) -> Option<Ban> {
    let name = attr(element, "name");
    let mut events = Vec::new();
    for text in list(&attr(element, "events")) {
        match Event::parse(text) {
            Some(event) => events.push(event),
            None => tracing::error!("web-protection ban '{name}': '{text}' is not an event; it is left out"),
        }
    }
    if events.is_empty() {
        return None;
    }
    let threshold = attr(element, "threshold")
        .parse::<u32>()
        .ok()
        .filter(|count| *count > 0)?;
    let window = attr(element, "seconds")
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)?;
    let duration = attr(element, "duration")
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)?;
    Some(Ban {
        name,
        events,
        threshold,
        window: nanos(Duration::from_secs(window)),
        duration: nanos(Duration::from_secs(duration)),
    })
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// One client in the table.
struct Client {
    connections: usize,
    /// GCRA arrival time of each rate limit, nanoseconds after [`Guard::epoch`].
    arrivals: Vec<u64>,
    /// Start and count of each ban's window.
    windows: Vec<(u64, u32)>,
    banned_until: u64,
    seen: u64,
}

/// Refusal counts and overload counts, for `/metrics`.
#[derive(Default)]
pub(crate) struct Counts {
    pub(crate) connections_refused: AtomicU64,
    pub(crate) denied: AtomicU64,
    pub(crate) rate_limited: AtomicU64,
    pub(crate) untracked: AtomicU64,
    pub(crate) queue_full: AtomicU64,
    pub(crate) queue_expired: AtomicU64,
    pub(crate) abandoned: AtomicU64,
}

/// The web protection state of one listener set: the conf and the clients seen.
pub(crate) struct Guard {
    pub(crate) conf: GuardConf,
    shards: Vec<Mutex<HashMap<IpAddr, Client>>>,
    tracked: AtomicUsize,
    epoch: Instant,
    pub(crate) counts: Counts,
}

/// A connection counted against its client's `max-per-client` until it is dropped.
pub(crate) struct ConnectionSlot {
    counted: Option<(Arc<Guard>, IpAddr)>,
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        if let Some((guard, key)) = self.counted.take() {
            guard.with_known(key, |client| client.connections = client.connections.saturating_sub(1));
        }
    }
}

/// The answer to a request [`Guard::check`] refused; a `Connection: close` header in it closes the connection.
pub(crate) struct Refused {
    pub(crate) response: Box<Response>,
}

impl Guard {
    pub(crate) fn new(conf: GuardConf) -> Self {
        Self {
            conf,
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            tracked: AtomicUsize::new(0),
            epoch: Instant::now(),
            counts: Counts::default(),
        }
    }

    /// Everything per client off: the global limits only.
    #[cfg(test)]
    pub(crate) fn disabled() -> Self {
        Self::new(GuardConf::from_node(None, Arc::new(ClientAddressing::default())))
    }

    fn now(&self) -> u64 {
        nanos(self.epoch.elapsed()).max(1)
    }

    /// The table key of `address`: itself for IPv4, its `client-ipv6-prefix` for IPv6.
    fn key(&self, address: IpAddr) -> IpAddr {
        let address = address.to_canonical();
        if address.is_ipv6() {
            masked(address, self.conf.ipv6_prefix)
        } else {
            address
        }
    }

    fn shard(&self, key: IpAddr) -> &Mutex<HashMap<IpAddr, Client>> {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match key {
            IpAddr::V4(v4) => hasher.write(&v4.octets()),
            IpAddr::V6(v6) => hasher.write(&v6.octets()),
        }
        let index = usize::try_from(hasher.finish() % SHARDS as u64).unwrap_or(0);
        &self.shards[index]
    }

    /// Run `body` on the client of `key`, adding it unless the table is full (then `None`, and it is counted).
    fn with_client<R>(&self, key: IpAddr, body: impl FnOnce(&mut Client, u64) -> R) -> Option<R> {
        let now = self.now();
        let mut shard = moqui_lib::sync::lock(self.shard(key));
        let client = match shard.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if self.tracked.load(Ordering::Relaxed) >= self.conf.max_tracked {
                    self.counts.untracked.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                self.tracked.fetch_add(1, Ordering::Relaxed);
                entry.insert(Client {
                    connections: 0,
                    arrivals: vec![0; self.conf.rate_limits.len()],
                    windows: vec![(0, 0); self.conf.bans.len()],
                    banned_until: 0,
                    seen: now,
                })
            }
        };
        client.seen = now;
        Some(body(client, now))
    }

    /// Run `body` on the client of `key` when it is in the table.
    fn with_known<R>(&self, key: IpAddr, body: impl FnOnce(&mut Client) -> R) -> Option<R> {
        moqui_lib::sync::lock(self.shard(key)).get_mut(&key).map(body)
    }

    fn exempt(&self, address: IpAddr) -> bool {
        self.conf.exempt.iter().any(|range| range.contains(address))
    }

    fn denied(&self, address: IpAddr) -> bool {
        self.conf.deny.iter().any(|range| range.contains(address))
    }

    fn banned(&self, key: IpAddr) -> bool {
        let now = self.now();
        self.with_known(key, |client| client.banned_until > now)
            .unwrap_or(false)
    }

    /// A connection from `peer`, before the TLS handshake: `None` closes it, for a denied or banned client or one
    /// over `max-per-client`. A trusted proxy and an exempt client are not counted.
    pub(crate) fn admit(self: &Arc<Self>, peer: IpAddr) -> Option<ConnectionSlot> {
        let peer = peer.to_canonical();
        let uncounted = ConnectionSlot { counted: None };
        if !self.conf.enabled || self.conf.addressing.is_trusted(peer) || self.exempt(peer) {
            return Some(uncounted);
        }
        let key = self.key(peer);
        if self.denied(peer) {
            self.counts.connections_refused.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let max = self.conf.max_per_client;
        let admitted = self.with_client(key, |client, now| {
            if client.banned_until > now || client.connections >= max {
                false
            } else {
                client.connections += 1;
                true
            }
        });
        match admitted {
            None => Some(uncounted),
            Some(true) => Some(ConnectionSlot {
                counted: Some((Arc::clone(self), key)),
            }),
            Some(false) => {
                self.counts.connections_refused.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("web protection: a connection from {peer} was refused");
                None
            }
        }
    }

    /// The client of a request from `peer` with `headers`.
    pub(crate) fn client_of(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let header = self.conf.addressing.header();
        let value = (!header.is_empty())
            .then(|| headers.get(header).and_then(|value| value.to_str().ok()))
            .flatten();
        self.conf.addressing.resolve(peer, value)
    }

    /// The rules and rate limits for a request from `client` (through `peer`), once its head is read. A refusal of a
    /// client that came straight to this listener closes the connection; through a trusted proxy it does not, since the
    /// proxy's connection carries other clients.
    pub(crate) fn check(
        &self,
        peer: IpAddr,
        client: IpAddr,
        method: &str,
        path: &str,
        headers: &HeaderMap,
    ) -> Result<(), Refused> {
        if !self.conf.enabled || self.exempt(client) {
            return Ok(());
        }
        let close = !self.conf.addressing.is_trusted(peer.to_canonical());
        let key = self.key(client);
        if self.denied(client) || self.banned(key) {
            self.counts.denied.fetch_add(1, Ordering::Relaxed);
            return Err(Refused {
                response: Box::new(refusal(403, "forbidden", close, None)),
            });
        }
        let agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !agent.is_empty() && self.conf.deny_agents.iter().any(|part| agent.contains(part.as_str())) {
            return Err(self.deny(key, client, refusal(403, "forbidden", close, None)));
        }
        if !self.conf.methods.is_empty() && !self.conf.methods.iter().any(|allowed| allowed == method) {
            let mut response = refusal(405, "method not allowed", false, None);
            if let Ok(allow) = self.conf.methods.join(", ").parse() {
                response.headers_mut().insert(axum::http::header::ALLOW, allow);
            }
            return Err(self.deny(key, client, response));
        }
        let decoded = decode_path(path);
        if self.conf.deny_paths.iter().any(|glob| glob.matches(&decoded)) {
            return Err(self.deny(key, client, refusal(404, "not found", false, None)));
        }
        if self.conf.rate_limits.is_empty() {
            return Ok(());
        }
        let limits = &self.conf.rate_limits;
        let waited = self.with_client(key, |client_state, now| {
            let mut next = Vec::with_capacity(limits.len());
            for (index, limit) in limits.iter().enumerate() {
                if !limit.applies(method, &decoded) {
                    next.push(None);
                    continue;
                }
                let arrival = client_state.arrivals[index].max(now).saturating_add(limit.interval);
                let ahead = arrival - now;
                if ahead > limit.tolerance {
                    return Some((ahead - limit.tolerance, limit.name.clone()));
                }
                next.push(Some(arrival));
            }
            for (index, arrival) in next.into_iter().enumerate() {
                if let Some(arrival) = arrival {
                    client_state.arrivals[index] = arrival;
                }
            }
            None
        });
        let Some(Some((wait, name))) = waited else {
            return Ok(());
        };
        self.counts.rate_limited.fetch_add(1, Ordering::Relaxed);
        tracing::debug!("web protection: rate limit {name} refused a request from {client}");
        self.record_key(key, client, Event::RateLimited);
        let seconds = wait.div_ceil(1_000_000_000).max(1);
        Err(Refused {
            response: Box::new(refusal(429, "too many requests", false, Some(seconds))),
        })
    }

    fn deny(&self, key: IpAddr, client: IpAddr, response: Response) -> Refused {
        self.counts.denied.fetch_add(1, Ordering::Relaxed);
        tracing::debug!("web protection: a request from {client} was denied by a rule");
        self.record_key(key, client, Event::Denied);
        Refused {
            response: Box::new(response),
        }
    }

    /// Count `event` for `client` toward the bans that count it. Called from the connection task and from request
    /// workers (an auth failure).
    pub(crate) fn record(&self, client: IpAddr, event: Event) {
        if !self.conf.enabled || self.exempt(client) {
            return;
        }
        self.record_key(self.key(client), client, event);
    }

    fn record_key(&self, key: IpAddr, client: IpAddr, event: Event) {
        let bans = &self.conf.bans;
        if !bans.iter().any(|ban| ban.events.contains(&event)) {
            return;
        }
        let started = self.with_client(key, |state, now| {
            let mut started = None;
            for (index, ban) in bans.iter().enumerate() {
                if !ban.events.contains(&event) {
                    continue;
                }
                let window = &mut state.windows[index];
                if window.1 == 0 || now.saturating_sub(window.0) > ban.window {
                    *window = (now, 0);
                }
                window.1 += 1;
                if window.1 >= ban.threshold {
                    *window = (now, 0);
                    let until = now.saturating_add(ban.duration);
                    if state.banned_until <= now {
                        started = Some((ban.name.clone(), ban.duration));
                    }
                    state.banned_until = state.banned_until.max(until);
                }
            }
            started
        });
        if let Some(Some((name, duration))) = started {
            tracing::warn!(
                "web protection: {client} is banned for {} seconds by ban {name}",
                duration / 1_000_000_000
            );
        }
    }

    /// Drop the clients idle past every window, with no open connection and no ban.
    pub(crate) fn sweep(&self) {
        let now = self.now();
        let idle = self.conf.idle_after();
        for shard in &self.shards {
            let mut shard = moqui_lib::sync::lock(shard);
            let before = shard.len();
            shard.retain(|_, client| {
                client.connections > 0 || client.banned_until > now || now.saturating_sub(client.seen) <= idle
            });
            self.tracked.fetch_sub(before - shard.len(), Ordering::Relaxed);
        }
    }

    /// A 503 for a busy server, with `Retry-After`.
    pub(crate) fn unavailable(&self, text: &str) -> Response {
        refusal(503, text, false, Some(self.conf.retry_after))
    }

    /// The `/metrics` lines.
    pub(crate) fn metrics(&self) -> String {
        let now = self.now();
        let banned: usize = self
            .shards
            .iter()
            .map(|shard| {
                moqui_lib::sync::lock(shard)
                    .values()
                    .filter(|client| client.banned_until > now)
                    .count()
            })
            .sum();
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        format!(
            "edge_connections_refused {}\nedge_denied {}\nedge_rate_limited {}\nedge_banned_clients {banned}\n\
             edge_tracked_clients {}\nedge_untracked {}\nrequest_queue_full {}\nrequest_queue_expired {}\n\
             request_abandoned {}\n",
            count(&self.counts.connections_refused),
            count(&self.counts.denied),
            count(&self.counts.rate_limited),
            self.tracked.load(Ordering::Relaxed),
            count(&self.counts.untracked),
            count(&self.counts.queue_full),
            count(&self.counts.queue_expired),
            count(&self.counts.abandoned),
        )
    }

    /// Log at startup what is on, and a forwarding header that is believed from anyone.
    pub(crate) fn log_start(&self) {
        let addressing = &self.conf.addressing;
        if !addressing.header().is_empty() && !addressing.has_trusted() {
            tracing::warn!(
                "client-ip-header {} is believed from any peer: set trusted-proxies (webapp_trusted_proxies) to the \
                 proxies in front so a client cannot name its own address",
                addressing.header()
            );
        }
        if self.conf.enabled {
            tracing::info!(
                "Web protection on: {} rate limits, {} bans, {} connections per client",
                self.conf.rate_limits.len(),
                self.conf.bans.len(),
                self.conf.max_per_client
            );
        } else {
            tracing::info!(
                "Web protection per client is off (no trusted-proxies and enabled is not true); global limits apply"
            );
        }
    }
}

/// A short plain response for a refusal: `Retry-After` when `retry` is given, `Connection: close` when `close`.
fn refusal(status: u16, text: &str, close: bool, retry: Option<u64>) -> Response {
    let mut response = crate::http::plain(status, text);
    let headers = response.headers_mut();
    if let Some(seconds) = retry {
        headers.insert(axum::http::header::RETRY_AFTER, axum::http::HeaderValue::from(seconds));
    }
    if close {
        headers.insert(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("close"),
        );
    }
    response
}

#[cfg(test)]
pub(crate) mod tests {
    use std::net::IpAddr;
    use std::sync::Arc;

    use axum::http::HeaderMap;
    use moqui_core::web::address::{parse_cidrs, ClientAddressing};
    use moqui_lib::node::MNode;

    use super::{decode_path, Event, Glob, Guard, GuardConf};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    pub(crate) fn guard(xml: &str, trusted: &str) -> Arc<Guard> {
        let node = MNode::parse_text("web-protection", xml).unwrap();
        let addressing = Arc::new(ClientAddressing::new("X-Forwarded-For", parse_cidrs(trusted, "test")));
        Arc::new(Guard::new(GuardConf::from_node(Some(&node), addressing)))
    }

    fn status(guard: &Guard, client: &str, method: &str, path: &str) -> u16 {
        let client = ip(client);
        match guard.check(client, client, method, path, &HeaderMap::new()) {
            Ok(()) => 200,
            Err(refused) => refused.response.status().as_u16(),
        }
    }

    #[test]
    fn globs_match_url_patterns() {
        let glob = |pattern: &str| Glob::new(pattern).unwrap();
        assert!(glob("/.git/**").matches("/.git/config"));
        assert!(glob("/.git/**").matches("/.git"));
        assert!(!glob("/.git/**").matches("/x/.git/config"));
        assert!(glob("**/*.php").matches("/wp/xmlrpc.php"));
        assert!(glob("**/*.php").matches("/index.PHP"));
        assert!(!glob("**/*.php").matches("/apps/php"));
        assert!(glob("**/*.asp*").matches("/a/b/default.aspx"));
        assert!(glob("/Login/**").matches("/login/login"));
        assert!(glob("/rest/login").matches("/rest/login"));
        assert!(!glob("/rest/login").matches("/rest/login/x"));
        assert_eq!(decode_path("/%2e%67it/config"), "/.git/config");
        assert_eq!(decode_path("/a+b%zz"), "/a+b%zz");
    }

    #[test]
    fn off_without_trusted_proxies_unless_enabled() {
        assert!(!guard(r#"<web-protection/>"#, "").conf.enabled);
        assert!(guard(r#"<web-protection/>"#, "10.0.0.0/8").conf.enabled);
        assert!(guard(r#"<web-protection enabled="true"/>"#, "").conf.enabled);
        assert!(!guard(r#"<web-protection enabled="false"/>"#, "10.0.0.0/8").conf.enabled);
        let off = guard(
            r#"<web-protection><rate-limit name="all" requests="1" burst="1"/></web-protection>"#,
            "",
        );
        for _ in 0..5 {
            assert_eq!(status(&off, "203.0.113.5", "GET", "/"), 200);
        }
    }

    #[test]
    fn a_rate_limit_answers_429_past_its_burst_and_only_where_it_applies() {
        let guard = guard(
            r#"<web-protection enabled="true">
                <rate-limit name="all" requests="1" seconds="60" burst="3"/>
                <rate-limit name="login" path="/Login/**" methods="POST" requests="1" seconds="60" burst="1"/>
            </web-protection>"#,
            "",
        );
        assert_eq!(status(&guard, "203.0.113.5", "POST", "/Login/login"), 200);
        // the login limit refuses the second POST, and its refusal takes no token from the other limit
        assert_eq!(status(&guard, "203.0.113.5", "POST", "/Login/login"), 429);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/Login/login"), 200);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/apps"), 200);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/apps"), 429);
        // another client has its own tokens
        assert_eq!(status(&guard, "203.0.113.6", "GET", "/apps"), 200);
        let client = ip("203.0.113.5");
        let refused = guard
            .check(client, client, "GET", "/", &HeaderMap::new())
            .err()
            .unwrap();
        let retry: u64 = refused.response.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=60).contains(&retry), "{retry}");
    }

    #[test]
    fn rules_deny_by_client_method_path_and_agent_and_exempt_skips_them() {
        let guard = guard(
            r#"<web-protection enabled="true">
                <requests methods="GET,POST"/>
                <client-rule action="exempt" cidr="127.0.0.0/8"/>
                <client-rule action="deny" cidr="198.51.100.0/24, nope"/>
                <path-rule action="deny" path="/.env,/.git/**,**/*.php"/>
                <user-agent-rule action="deny" contains="sqlmap"/>
            </web-protection>"#,
            "",
        );
        assert_eq!(status(&guard, "198.51.100.9", "GET", "/"), 403);
        assert_eq!(status(&guard, "203.0.113.5", "DELETE", "/"), 405);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/%2Eenv"), 404);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/x/index.php"), 404);
        assert_eq!(status(&guard, "127.0.0.1", "GET", "/x/index.php"), 200);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/apps"), 200);
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "sqlmap/1.7".parse().unwrap());
        let client = ip("203.0.113.5");
        let refused = guard.check(client, client, "GET", "/apps", &headers).err().unwrap();
        assert_eq!(refused.response.status(), 403);
        assert_eq!(refused.response.headers()["connection"], "close");
    }

    #[test]
    fn events_past_a_threshold_ban_the_client_and_its_connections() {
        let guard = guard(
            r#"<web-protection enabled="true">
                <ban name="auth-failure" events="auth-failure" threshold="3" seconds="600" duration="900"/>
            </web-protection>"#,
            "",
        );
        let client = ip("203.0.113.5");
        guard.record(client, Event::AuthFailure);
        guard.record(client, Event::AuthFailure);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/"), 200);
        assert!(guard.admit(client).is_some());
        guard.record(client, Event::AuthFailure);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/"), 403);
        assert!(guard.admit(client).is_none());
        // a rate-limited event does not count toward this ban
        assert_eq!(status(&guard, "203.0.113.6", "GET", "/"), 200);
        assert!(
            guard.metrics().contains("edge_banned_clients 1\n"),
            "{}",
            guard.metrics()
        );
    }

    #[test]
    fn connections_per_client_are_capped_and_given_back() {
        let guard = guard(
            r#"<web-protection enabled="true"><connections max-per-client="2"/></web-protection>"#,
            "10.0.0.0/8",
        );
        let client = ip("203.0.113.5");
        let first = guard.admit(client).unwrap();
        let _second = guard.admit(client).unwrap();
        assert!(guard.admit(client).is_none());
        drop(first);
        assert!(guard.admit(client).is_some());
        // a trusted proxy carries many clients and is not capped
        let proxy = ip("10.0.0.2");
        let held: Vec<_> = (0..5).map(|_| guard.admit(proxy).unwrap()).collect();
        assert_eq!(held.len(), 5);
    }

    #[test]
    fn ipv6_clients_share_their_prefix() {
        let guard = guard(
            r#"<web-protection enabled="true"><rate-limit name="all" requests="1" seconds="60" burst="1"/></web-protection>"#,
            "",
        );
        assert_eq!(status(&guard, "2001:db8:1:2::10", "GET", "/"), 200);
        assert_eq!(status(&guard, "2001:db8:1:2::99", "GET", "/"), 429);
        assert_eq!(status(&guard, "2001:db8:1:3::10", "GET", "/"), 200);
    }

    #[test]
    fn a_full_table_serves_new_clients_untracked() {
        let guard = guard(
            r#"<web-protection enabled="true" max-tracked-clients="1">
                <rate-limit name="all" requests="1" seconds="60" burst="1"/>
            </web-protection>"#,
            "",
        );
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/"), 200);
        for _ in 0..3 {
            assert_eq!(status(&guard, "203.0.113.6", "GET", "/"), 200);
        }
        assert!(guard.metrics().contains("edge_untracked 3\n"), "{}", guard.metrics());
    }

    #[test]
    fn the_default_conf_is_off_until_there_are_trusted_proxies() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../runtime/base-component/framework/conf/MoquiDefaultConf.xml"
        ))
        .unwrap();
        let node = super::protection_node(&text);
        assert!(node.is_some(), "MoquiDefaultConf.xml has a web-protection");
        let off = GuardConf::from_node(node.as_ref(), Arc::new(ClientAddressing::default()));
        assert!(!off.enabled);
        let behind = Arc::new(ClientAddressing::new("X-Real-IP", parse_cidrs("10.0.0.0/8", "test")));
        let on = GuardConf::from_node(node.as_ref(), behind);
        assert!(on.enabled);
        let names: Vec<&str> = on.rate_limits.iter().map(|limit| limit.name.as_str()).collect();
        assert_eq!(names, ["all", "login"]);
        assert_eq!(on.bans.len(), 2);
        assert!(on.exempt.iter().any(|range| range.contains(ip("127.0.0.1"))));
        assert!(on.exempt.iter().any(|range| range.contains(ip("::1"))));
        assert!(on.deny_paths.iter().any(|glob| glob.matches("/.env")));
        assert!(!on.deny_paths.iter().any(|glob| glob.matches("/apps/system")));
        assert_eq!(on.methods.len(), 7);
        assert_eq!(on.max_per_client, 64);
        assert_eq!(on.queue_timeout, Some(std::time::Duration::from_secs(10)));
        assert_eq!(on.retry_after, 5);
        let limits = crate::http::ListenLimits::from_node(node.as_ref());
        assert_eq!(limits.max_uri, 8192);
        assert_eq!(limits.max_header_bytes, 65536);
        assert_eq!(limits.max_headers, 100);
    }

    #[test]
    fn bad_entries_are_left_out_and_the_rest_apply() {
        let guard = guard(
            r#"<web-protection enabled="true">
                <rate-limit name="bad" requests="many"/>
                <rate-limit name="off" requests="1" burst="1" enabled="false"/>
                <rate-limit name="good" requests="1" seconds="60" burst="1"/>
                <ban name="bad" events="nonsense" threshold="1" seconds="1" duration="1"/>
            </web-protection>"#,
            "",
        );
        assert_eq!(guard.conf.rate_limits.len(), 1);
        assert!(guard.conf.bans.is_empty());
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/"), 200);
        assert_eq!(status(&guard, "203.0.113.5", "GET", "/"), 429);
    }
}
