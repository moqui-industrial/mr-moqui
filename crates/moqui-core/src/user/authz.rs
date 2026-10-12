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

//! Artifact authorization. The check allows every call until phase 5.

use crate::context::Error;

/// True while an [`crate::context::ExecutionContext::enter`] guard is live on this thread.
pub fn context_entered() -> bool {
    crate::context::ExecutionContext::current().is_some()
}

/// Allow the artifact. Phase 5 replaces this with the permission check.
pub fn authorize(artifact_type: &str, name: &str, action: &str) -> Result<(), Error> {
    let _ = (artifact_type, name, action);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::context::ExecutionContext;

    #[test]
    fn enter_marks_the_thread_until_the_guard_drops() {
        let context = crate::context::ExecutionContextFactory::test_context();
        assert!(!super::context_entered());
        assert!(ExecutionContext::current().is_none());
        assert!(context.allows());
        let outer = context.enter();
        assert!(super::context_entered());
        assert!(ExecutionContext::current().is_some());
        let inner = context.enter();
        assert!(super::context_entered());
        drop(inner);
        assert!(super::context_entered());
        drop(outer);
        assert!(!super::context_entered());
        assert!(ExecutionContext::current().is_none());
        assert!(context.allows());
    }
}
