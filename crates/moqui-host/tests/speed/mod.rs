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

//! What the speed tests share: an allocator that counts this thread's allocations, the timed rows, and the table.
//!
//! The counts are of the thread that runs the operation, so a tokio worker or a cache evictor does not show in them.
//! They do not change from run to run the way times do, which makes them the steadier measure of a change.
//!
//! - **allocs/call**: allocations and reallocations. The allocator's work, and what fragments the heap.
//! - **bytes/call**: bytes those asked for, freed or not. The memory traffic ("pressure").
//! - **kept/call**: bytes allocated and not freed on this thread by the end of the batch, a cache or a log that grows.
//! - **sql/call**: statements the entity layer ran (`statement_stats`), on any thread. Begin and commit are not counted.

#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

fn note_alloc(size: usize) {
    let size = i64::try_from(size).unwrap_or(i64::MAX);
    let _ = ALLOCS.try_with(|count| count.set(count.get() + 1));
    let _ = BYTES.try_with(|bytes| bytes.set(bytes.get() + size.unsigned_abs()));
    let _ = LIVE.try_with(|live| live.set(live.get() + size));
}

fn note_free(size: usize) {
    let size = i64::try_from(size).unwrap_or(i64::MAX);
    let _ = LIVE.try_with(|live| live.set(live.get() - size));
}

// SAFETY: every call goes to `System` with the caller's arguments. The counters are const-initialized thread locals
// without destructors, so touching them does not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_alloc(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_alloc(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_free(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_free(layout.size());
        note_alloc(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub allocs: u64,
    pub bytes: u64,
    pub live: i64,
}

pub fn counts() -> Counts {
    Counts {
        allocs: ALLOCS.with(Cell::get),
        bytes: BYTES.with(Cell::get),
        live: LIVE.with(Cell::get),
    }
}

pub struct Measured {
    pub elapsed: Duration,
    pub allocs: u64,
    pub bytes: u64,
    pub kept: i64,
    pub statements: u64,
}

/// Time `body` and count what this thread allocated in it.
pub fn measure<T>(body: impl FnOnce() -> T) -> (Measured, T) {
    let (statements_before, _) = moqui_core::entity::statement_stats();
    let before = counts();
    let started = Instant::now();
    let out = body();
    let elapsed = started.elapsed();
    let after = counts();
    let (statements_after, _) = moqui_core::entity::statement_stats();
    let measured = Measured {
        elapsed,
        allocs: after.allocs - before.allocs,
        bytes: after.bytes - before.bytes,
        kept: after.live - before.live,
        statements: statements_after - statements_before,
    };
    (measured, out)
}

pub struct Row {
    pub operation: String,
    pub calls: u64,
    pub batches: Vec<Measured>,
}

/// The rows of one mode. A warmup batch is run with `record` off, so its first-call costs are not kept.
#[derive(Default)]
pub struct Rows {
    pub record: bool,
    pub rows: Vec<Row>,
}

impl Rows {
    /// Run `body`, which makes `calls` calls, and keep its measure under `operation`.
    pub fn run<T>(&mut self, operation: &str, calls: usize, body: impl FnOnce() -> T) -> T {
        let (measured, out) = measure(body);
        self.note(operation, calls, measured);
        out
    }

    pub fn note(&mut self, operation: &str, calls: usize, measured: Measured) {
        if !self.record {
            return;
        }
        if let Some(row) = self.rows.iter_mut().find(|row| row.operation == operation) {
            row.batches.push(measured);
            return;
        }
        self.rows.push(Row {
            operation: operation.to_string(),
            calls: u64::try_from(calls).unwrap_or(u64::MAX),
            batches: vec![measured],
        });
    }

    /// Per call, from the upper median batch by time. The allocation columns are of that batch too.
    pub fn print(&self) {
        println!(
            "  {:<46} {:>7} {:>10} {:>9} {:>10} {:>9} {:>10} {:>9} {:>6} {:>17}",
            "operation", "calls", "median ms", "us/call", "calls/s", "allocs", "bytes", "kept", "sql", "min-max ms"
        );
        for row in &self.rows {
            let mut order: Vec<usize> = (0..row.batches.len()).collect();
            order.sort_by_key(|index| row.batches[*index].elapsed);
            let median = &row.batches[order[order.len() / 2]];
            let calls = row.calls.max(1) as f64;
            let seconds = median.elapsed.as_secs_f64();
            let per_second = if seconds == 0.0 { 0.0 } else { calls / seconds };
            let fastest = row.batches[order[0]].elapsed.as_secs_f64() * 1.0e3;
            let slowest = row.batches[order[order.len() - 1]].elapsed.as_secs_f64() * 1.0e3;
            println!(
                "  {:<46} {:>7} {:>10.3} {:>9.2} {:>10.0} {:>9.1} {:>10.0} {:>9.0} {:>6.2} {:>8.3}-{:<8.3}",
                row.operation,
                row.calls,
                seconds * 1.0e3,
                seconds * 1.0e6 / calls,
                per_second,
                median.allocs as f64 / calls,
                median.bytes as f64 / calls,
                median.kept as f64 / calls,
                median.statements as f64 / calls,
                fastest,
                slowest,
            );
        }
    }
}

pub fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|text| text.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// Removes the database directory when the test ends, also on a panic.
pub struct DirGuard(pub PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Under `/tmp`, not `TMPDIR`: the Postgres socket path has to fit a short limit.
pub fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    PathBuf::from("/tmp").join(format!("{prefix}-{}-{nanos}", std::process::id()))
}

pub fn warn_if_debug(name: &str) {
    if cfg!(debug_assertions) {
        eprintln!(
            "{name}: this binary has debug assertions, so it is not a release profile. \
             The numbers are not comparable to ./moq build."
        );
    }
}
