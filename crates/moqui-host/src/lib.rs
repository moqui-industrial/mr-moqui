//! Candidate process edge. The server, drivers, and PDF renderer are not implemented yet.

/// Process entry used by the `moqui` binary.
pub fn run() -> &'static str {
    let _ = moqui_core::candidate();
    "moqui candidate build; server not implemented"
}
