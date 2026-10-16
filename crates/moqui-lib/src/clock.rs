//! Framework logical time. Durations and deadlines must continue to use `Instant`.
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

static DETERMINISTIC: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
thread_local! { static ASSIGNED: Cell<Option<i64>> = const { Cell::new(None) }; }

pub fn deterministic() -> bool {
    DETERMINISTIC.load(Ordering::SeqCst)
}

/// Called by the transaction coordinator, never by application code.
pub fn enable_deterministic() {
    DETERMINISTIC.store(true, Ordering::SeqCst);
}

pub fn fail() {
    FAILED.store(true, Ordering::SeqCst);
}

pub fn assign(millis: Option<i64>) {
    ASSIGNED.with(|time| time.set(millis));
}

pub fn now_millis() -> i64 {
    if deterministic() || std::env::var("MOQUI_DETERMINISTIC").as_deref() == Ok("true") {
        assert!(
            !FAILED.load(Ordering::SeqCst),
            "deterministic coordinator failed; restart required"
        );
        return ASSIGNED.with(|time| {
            time.get()
                .expect("deterministic clock requires a coordinator operation")
        });
    }
    jiff::Timestamp::now().as_millisecond()
}

pub fn now() -> jiff::Timestamp {
    jiff::Timestamp::from_millisecond(now_millis()).expect("validated logical timestamp")
}

pub fn zoned_now() -> jiff::Zoned {
    now().to_zoned(jiff::tz::TimeZone::system())
}
