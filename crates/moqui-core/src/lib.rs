//! Candidate framework crate. Facades are not implemented yet.

/// Marker so the host links this crate.
pub fn candidate() -> &'static str {
    let _ = moqui_db::candidate();
    moqui_lib::candidate()
}
