//! Candidate utilities crate. No `ExecutionContext`.
//! The modules in `plans/Crate-Layout.md` are not implemented yet.
//! Feature `http` links blocking `reqwest`. It is off unless a dependent enables it.

/// Marker so a downstream crate links this one.
pub fn candidate() -> &'static str {
    "moqui-lib"
}
