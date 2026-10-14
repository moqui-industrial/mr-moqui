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


//! Server log output, and shipping logs to the search backend's log indexes.
//!
//! [`install`] sets the process's `tracing` subscriber: one line per event on stderr, at
//! `moqui_log_level` (default `info`) and above. Spans are not tracked; artifact hits are counted
//! when a frame pops, not here.
//!
//! When the search backend keeps log indexes (OpenSearch), [`ship_to_search`] starts a thread that
//! bulk-indexes queued events to `moqui_logs` and HTTP requests to `moqui_http_log`, as Java's
//! `ElasticSearchLogger` and `ElasticRequestLogFilter` do: DEBUG and TRACE are not shipped, a full
//! queue drops new entries, and events logged while shipping are not queued again.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::entity::document::Document;
use moqui_lib::object::Value;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// Entries waiting for the shipper, per index. Java's `QUEUE_LIMIT`.
const QUEUE_LIMIT: usize = 16384;
/// Entries per bulk request.
const BATCH: usize = 1000;
const FLUSH_EVERY: Duration = Duration::from_secs(2);

static APP_QUEUE: Mutex<VecDeque<Document>> = Mutex::new(VecDeque::new());
static HTTP_QUEUE: Mutex<VecDeque<Document>> = Mutex::new(VecDeque::new());
static SHIPPING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Set on the shipper thread, so the client's own events are not shipped.
    static IN_SHIPPER: Cell<bool> = const { Cell::new(false) };
}

/// Install the stderr subscriber once per process. Later calls, and a process that already has a
/// global subscriber, change nothing.
pub fn install() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let level = match moqui_lib::env::get_prop_or_env("moqui_log_level")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "trace" => Level::TRACE,
            "debug" => Level::DEBUG,
            "warn" | "warning" => Level::WARN,
            "error" => Level::ERROR,
            _ => Level::INFO,
        };
        let _ = tracing::subscriber::set_global_default(LineSubscriber {
            level,
            next_span: AtomicU64::new(1),
        });
    });
}

/// Start shipping log events and HTTP requests to the backend's log indexes, when it keeps them.
/// Returns false and starts nothing for a backend that does not (Postgres).
pub fn ship_to_search(factory: &Arc<ExecutionContextFactory>) -> bool {
    let Some(facade) = moqui_core::search::installed(factory) else {
        return false;
    };
    if !facade.backend().keeps_log_indexes() || SHIPPING.swap(true, Ordering::SeqCst) {
        return false;
    }
    let spawned = std::thread::Builder::new()
        .name("moqui-log-ship".into())
        .spawn(move || {
            IN_SHIPPER.with(|flag| flag.set(true));
            loop {
                std::thread::sleep(FLUSH_EVERY);
                flush(&facade);
            }
        });
    if spawned.is_err() {
        SHIPPING.store(false, Ordering::SeqCst);
        return false;
    }
    true
}

/// Index what is queued now. The shipper thread calls this; tests call it to avoid waiting.
pub fn flush(facade: &moqui_core::search::SearchFacade) {
    for (queue, index) in [
        (&APP_QUEUE, moqui_core::search::logs::APP_LOG),
        (&HTTP_QUEUE, moqui_core::search::logs::HTTP_LOG),
    ] {
        loop {
            let batch: Vec<Document> = {
                let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let count = queue.len().min(BATCH);
                queue.drain(..count).collect()
            };
            if batch.is_empty() {
                break;
            }
            let full = batch.len() == BATCH;
            if let Err(err) = facade.index_log_entries(index, batch) {
                // stderr, not tracing: an event here would be queued for the store that just failed
                let _ = writeln!(std::io::stderr(), "could not ship log entries to {index}: {err}");
                break;
            }
            if !full {
                break;
            }
        }
    }
}

/// Queue one HTTP request for `moqui_http_log`. Nothing is queued unless shipping has started.
pub fn queue_http_request(entry: Document) {
    if SHIPPING.load(Ordering::Relaxed) {
        push(&HTTP_QUEUE, entry);
    }
}

fn push(queue: &Mutex<VecDeque<Document>>, entry: Document) {
    let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if queue.len() < QUEUE_LIMIT {
        queue.push_back(entry);
    }
}

/// The time now in epoch millis.
pub fn now_millis() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// This host's name, as Java's `InetAddress.getLocalHost().hostName`, read once.
pub fn host_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .or_else(|| std::env::var("HOSTNAME").ok())
            .or_else(|| std::env::var("COMPUTERNAME").ok())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "localhost".to_string())
    })
}

struct LineSubscriber {
    level: Level,
    next_span: AtomicU64,
}

impl Subscriber for LineSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= self.level
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed).max(1))
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed").to_string();
        let millis = now_millis();
        let mut line = String::with_capacity(96 + fields.message.len());
        let stamp = jiff::Timestamp::from_millisecond(millis)
            .map(|at| at.strftime("%Y-%m-%d %H:%M:%S%.3f").to_string())
            .unwrap_or_default();
        let _ = write!(
            line,
            "{stamp} {:<5} [{thread_name}] {} - {}",
            metadata.level(),
            metadata.target(),
            fields.message
        );
        for (name, value) in &fields.extra {
            let _ = write!(line, " {name}={value}");
        }
        let _ = writeln!(std::io::stderr(), "{line}");

        let level = *metadata.level();
        if !SHIPPING.load(Ordering::Relaxed)
            || level == Level::DEBUG
            || level == Level::TRACE
            || IN_SHIPPER.with(Cell::get)
        {
            return;
        }
        let mut entry = Document::new();
        entry.insert("@timestamp".into(), Value::I64(millis));
        entry.insert("level".into(), Value::string(level.as_str()));
        entry.insert("thread_name".into(), Value::String(thread_name));
        entry.insert("thread_id".into(), Value::I64(thread_number(&thread)));
        entry.insert("logger_name".into(), Value::string(metadata.target()));
        entry.insert("message".into(), Value::String(fields.message));
        entry.insert("source_host".into(), Value::string(host_name()));
        if !fields.extra.is_empty() {
            let mdc = fields
                .extra
                .into_iter()
                .map(|(name, value)| (name, Value::String(value)))
                .collect();
            entry.insert("mdc".into(), Value::Map(mdc));
        }
        push(&APP_QUEUE, entry);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// The number in `ThreadId(n)`; `ThreadId::as_u64` is not stable.
fn thread_number(thread: &std::thread::Thread) -> i64 {
    let text = format!("{:?}", thread.id());
    text.trim_start_matches("ThreadId(")
        .trim_end_matches(')')
        .parse()
        .unwrap_or(0)
}

#[derive(Default)]
struct Fields {
    message: String,
    extra: Vec<(String, String)>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.extra.push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.extra.push((field.name().to_string(), format!("{value:?}")));
        }
    }
}
