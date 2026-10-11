//! Entity model, SQL text, and dialects. No `ExecutionContext` and no driver.
//! The modules in `plans/Crate-Layout.md` are not implemented yet.

/// Marker so a downstream crate links this one.
pub fn candidate() -> &'static str {
    let _ = moqui_lib::candidate();
    "moqui-db"
}
