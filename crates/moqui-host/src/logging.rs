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

//! Server log output, log files, and shipping logs to the search backend's log indexes.
//!
//! [`install`] sets the process's `tracing` subscriber: one line per event on stderr, at
//! `moqui_log_level` (default `info`) and above. Spans are not tracked; artifact hits are counted
//! when a frame pops, not here.
//!
//! [`attach_files`] opens `moqui.log` and `moqui-error.log` once the runtime directory and the
//! default properties are known, writes the lines logged before that, and rolls the files over by
//! size and by day. Without it lines go to stderr only.
//!
//! When the search backend keeps log indexes (OpenSearch or Postgres), [`ship_to_search`] starts a
//! thread that bulk-indexes queued events to `moqui_logs` and HTTP requests to `moqui_http_log`
//! through the SearchFacade, as Java's `ElasticSearchLogger` and `ElasticRequestLogFilter` do.
//! Events are queued from [`install`] on, so the boot lines ship once search attaches. DEBUG and
//! TRACE are not shipped, a full queue drops new entries, and events logged while shipping are not
//! queued again. [`stop_shipping`] sends what is left and ends the thread.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
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
/// The shipper checks for a stop this often while it waits to flush.
const POLL: Duration = Duration::from_millis(100);

static APP_QUEUE: Mutex<VecDeque<Document>> = Mutex::new(VecDeque::new());
static HTTP_QUEUE: Mutex<VecDeque<Document>> = Mutex::new(VecDeque::new());
static SHIPPING: AtomicBool = AtomicBool::new(false);
static STOP_SHIPPING: AtomicBool = AtomicBool::new(false);
static SHIPPER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
/// Lines logged before [`attach_files`] and the sinks after it.
static FILES: Mutex<FileState> = Mutex::new(FileState::Pending(Vec::new()));
/// Lines held for the files before they attach. A failed boot never attaches, so this is capped.
const PENDING_LINE_LIMIT: usize = 20000;

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
/// Returns false and starts nothing for a backend that does not.
pub fn ship_to_search(factory: &Arc<ExecutionContextFactory>) -> bool {
    let Some(facade) = moqui_core::search::installed(factory) else {
        return false;
    };
    if !facade.backend().keeps_log_indexes() || SHIPPING.swap(true, Ordering::SeqCst) {
        return false;
    }
    STOP_SHIPPING.store(false, Ordering::SeqCst);
    if let Err(err) = facade.ensure_log_indexes() {
        tracing::warn!("Could not create the log indexes, the first flush tries again: {err}");
    }
    let backend_name = facade.backend().name();
    let shipper_facade = Arc::clone(&facade);
    let spawned = std::thread::Builder::new()
        .name("moqui-log-ship".into())
        .spawn(move || {
            IN_SHIPPER.with(|flag| flag.set(true));
            loop {
                let mut waited = Duration::ZERO;
                while waited < FLUSH_EVERY && !STOP_SHIPPING.load(Ordering::SeqCst) {
                    std::thread::sleep(POLL);
                    waited += POLL;
                }
                flush(&shipper_facade);
                if STOP_SHIPPING.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
    match spawned {
        Ok(handle) => {
            *SHIPPER.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
            tracing::info!(
                "Shipping log events to the {backend_name} search backend indexes {} and {}",
                moqui_core::search::logs::APP_LOG,
                moqui_core::search::logs::HTTP_LOG
            );
            true
        }
        Err(_) => {
            SHIPPING.store(false, Ordering::SeqCst);
            false
        }
    }
}

/// Send what is queued and end the shipper thread, so it no longer holds the search facade and its
/// database connections. Does nothing when shipping never started.
pub fn stop_shipping() {
    let handle = SHIPPER.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
    if let Some(handle) = handle {
        STOP_SHIPPING.store(true, Ordering::SeqCst);
        let _ = handle.join();
        SHIPPING.store(false, Ordering::SeqCst);
    }
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

enum FileState {
    /// Before [`attach_files`]: lines wait here.
    Pending(Vec<(Level, String)>),
    Attached(Box<FileSinks>),
    /// No directory, or it could not be opened.
    Off,
}

struct FileSinks {
    all: Rolling,
    errors: Rolling,
}

/// One log file that rolls over by size and by day.
struct Rolling {
    dir: PathBuf,
    base: &'static str,
    file: Option<std::fs::File>,
    size: u64,
    day: String,
    max_bytes: u64,
    max_files: usize,
}

impl Rolling {
    fn open(dir: &Path, base: &'static str, max_bytes: u64, max_files: usize) -> std::io::Result<Self> {
        let mut rolling = Self {
            dir: dir.to_path_buf(),
            base,
            file: None,
            size: 0,
            day: String::new(),
            max_bytes,
            max_files,
        };
        let path = rolling.path();
        let modified = std::fs::metadata(&path).ok().and_then(|meta| meta.modified().ok());
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        rolling.size = file.metadata().map_or(0, |meta| meta.len());
        // An existing file belongs to the day it was last written, so a restart on a later day rolls it first.
        rolling.day = modified
            .and_then(|time| jiff::Timestamp::try_from(time).ok())
            .map_or_else(today, |at| at.strftime("%Y-%m-%d").to_string());
        rolling.file = Some(file);
        Ok(rolling)
    }

    fn path(&self) -> PathBuf {
        self.dir.join(format!("{}.log", self.base))
    }

    fn write(&mut self, line: &str) {
        let day = today();
        let incoming = line.len() as u64 + 1;
        if self.size > 0 && (day != self.day || self.size + incoming > self.max_bytes) {
            self.roll();
        }
        if self.size == 0 {
            self.day = day;
        }
        if let Some(file) = self.file.as_mut() {
            if writeln!(file, "{line}").is_ok() {
                self.size += incoming;
            }
        }
    }

    /// Move the current file to `base.yyyy-MM-dd.N.log`, start a new one, and drop the oldest rolled files.
    fn roll(&mut self) {
        self.file = None;
        let mut index = 1;
        let rolled = loop {
            let candidate = self.dir.join(format!("{}.{}.{index}.log", self.base, self.day));
            if !candidate.exists() {
                break candidate;
            }
            index += 1;
        };
        if std::fs::rename(self.path(), &rolled).is_err() {
            let _ = std::fs::remove_file(self.path());
        }
        self.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
            .ok();
        self.size = 0;
        self.prune();
    }

    fn prune(&self) {
        let prefix = format!("{}.", self.base);
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut rolled: Vec<(std::time::SystemTime, PathBuf)> = read
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with(&prefix) && name.ends_with(".log") && *name != format!("{}.log", self.base)
            })
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        rolled.sort();
        let excess = rolled.len().saturating_sub(self.max_files);
        for (_, path) in rolled.into_iter().take(excess) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn today() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%d").to_string()
}

impl FileSinks {
    fn write(&mut self, level: Level, line: &str) {
        self.all.write(line);
        if level <= Level::WARN {
            self.errors.write(line);
        }
    }
}

fn write_file_line(level: Level, line: &str) {
    let mut state = FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match &mut *state {
        FileState::Pending(lines) => {
            if lines.len() < PENDING_LINE_LIMIT {
                lines.push((level, line.to_string()));
            }
        }
        FileState::Attached(sinks) => sinks.write(level, line),
        FileState::Off => {}
    }
}

/// Open the log files named by `moqui_log_dir` (a path, or a `runtime://` location; empty means no files),
/// `moqui_log_max_size_mb` and `moqui_log_max_files`, and write the lines logged so far. The first call
/// decides; later calls change nothing. Returns the directory when files are being written.
pub fn attach_files(runtime_dir: &Path) -> Option<PathBuf> {
    let mut state = FILES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let FileState::Pending(pending) = &mut *state else {
        return None;
    };
    let pending = std::mem::take(pending);
    let dir = file_dir(runtime_dir, &moqui_lib::env::get_prop_or_env("moqui_log_dir"));
    let max_mb = moqui_lib::env::get_prop_or_env("moqui_log_max_size_mb")
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|mb| *mb > 0)
        .unwrap_or(100);
    let max_files = moqui_lib::env::get_prop_or_env("moqui_log_max_files")
        .trim()
        .parse::<usize>()
        .unwrap_or(30);
    let opened = dir.as_ref().and_then(|dir| {
        let sinks = std::fs::create_dir_all(dir).and_then(|()| {
            Ok(FileSinks {
                all: Rolling::open(dir, "moqui", max_mb * 1024 * 1024, max_files)?,
                errors: Rolling::open(dir, "moqui-error", max_mb * 1024 * 1024, max_files)?,
            })
        });
        match sinks {
            Ok(sinks) => Some((dir.clone(), sinks)),
            Err(err) => {
                // stderr, not tracing: this runs under the state lock
                let _ = writeln!(
                    std::io::stderr(),
                    "could not open log files in {}: {err}",
                    dir.display()
                );
                None
            }
        }
    });
    match opened {
        Some((dir, mut sinks)) => {
            for (level, line) in &pending {
                sinks.write(*level, line);
            }
            *state = FileState::Attached(Box::new(sinks));
            Some(dir)
        }
        None => {
            *state = FileState::Off;
            None
        }
    }
}

fn file_dir(runtime_dir: &Path, configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if configured.is_empty() {
        return None;
    }
    let path = match configured.strip_prefix("runtime://") {
        Some(rest) => runtime_dir.join(rest.trim_start_matches('/')),
        None => PathBuf::from(configured),
    };
    Some(if path.is_absolute() {
        path
    } else {
        runtime_dir.join(path)
    })
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
        write_file_line(level, &line);

        // Queued from the start, so the boot lines ship once search attaches.
        if level == Level::DEBUG || level == Level::TRACE || IN_SHIPPER.with(Cell::get) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moqui-logging-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rolled(dir: &Path, base: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(&format!("{base}.")) && name != &format!("{base}.log"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_file_rolls_by_size_and_keeps_the_newest() {
        let dir = scratch("size");
        let mut log = Rolling::open(&dir, "moqui", 100, 2).unwrap();
        for index in 0..20 {
            log.write(&format!("line {index:02} {}", "x".repeat(30)));
            // distinct modified times, so the oldest is well defined
            std::thread::sleep(Duration::from_millis(15));
        }
        let names = rolled(&dir, "moqui");
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(
            names
                .iter()
                .all(|name| name.ends_with(".log") && name.contains(&today())),
            "{names:?}"
        );
        let current = std::fs::read_to_string(dir.join("moqui.log")).unwrap();
        assert!(current.contains("line 19"), "{current}");
        assert!(!std::fs::read_to_string(dir.join(&names[0]))
            .unwrap()
            .contains("line 00"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_from_an_earlier_day_rolls_before_the_first_write() {
        let dir = scratch("day");
        let mut log = Rolling::open(&dir, "moqui", 1_000_000, 5).unwrap();
        log.write("first");
        log.day = "2000-01-01".to_string();
        log.write("second");
        let names = rolled(&dir, "moqui");
        assert_eq!(names, vec!["moqui.2000-01-01.1.log".to_string()]);
        assert_eq!(std::fs::read_to_string(dir.join("moqui.log")).unwrap(), "second\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warnings_and_errors_go_to_the_error_file_too() {
        let dir = scratch("errors");
        let mut sinks = FileSinks {
            all: Rolling::open(&dir, "moqui", 1_000_000, 5).unwrap(),
            errors: Rolling::open(&dir, "moqui-error", 1_000_000, 5).unwrap(),
        };
        sinks.write(Level::INFO, "info line");
        sinks.write(Level::WARN, "warn line");
        sinks.write(Level::ERROR, "error line");
        assert_eq!(
            std::fs::read_to_string(dir.join("moqui.log")).unwrap().lines().count(),
            3
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("moqui-error.log")).unwrap(),
            "warn line\nerror line\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_dir_is_a_runtime_location_a_path_or_empty() {
        let runtime = Path::new("/srv/moqui/runtime");
        assert_eq!(
            file_dir(runtime, "runtime://log"),
            Some(PathBuf::from("/srv/moqui/runtime/log"))
        );
        assert_eq!(
            file_dir(runtime, "/var/log/moqui"),
            Some(PathBuf::from("/var/log/moqui"))
        );
        assert_eq!(
            file_dir(runtime, "logs"),
            Some(PathBuf::from("/srv/moqui/runtime/logs"))
        );
        assert_eq!(file_dir(runtime, "  "), None);
    }
}
