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

//! Artifact stack. Push and pop are tracing spans. Authorization is phase 5.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use super::error::Error;

struct Frame {
    entered: tracing::span::EnteredSpan,
    started: Instant,
}

#[derive(Default)]
pub(super) struct ArtifactStack {
    frames: Vec<Frame>,
    authz_depth: Rc<Cell<u32>>,
}

impl ArtifactStack {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn push(&mut self, artifact_type: &str, name: &str, action: &str) {
        let artifact_type = artifact_type.to_string();
        let name = name.to_string();
        let action = action.to_string();
        let entered = tracing::info_span!(
            "artifact",
            artifact_type = %artifact_type,
            name = %name,
            action = %action,
            duration_ms = tracing::field::Empty,
        )
        .entered();
        self.frames.push(Frame {
            entered,
            started: Instant::now(),
        });
    }

    pub(super) fn pop(&mut self) -> Result<(), Error> {
        let frame = self.frames.pop().ok_or_else(|| Error::new("artifact stack is empty"))?;
        let duration_ms = u64::try_from(frame.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        frame.entered.record("duration_ms", duration_ms);
        drop(frame);
        Ok(())
    }

    pub(super) fn disable_authz(&self) -> AuthzGuard {
        self.authz_depth.set(self.authz_depth.get().saturating_add(1));
        AuthzGuard {
            depth: Rc::clone(&self.authz_depth),
        }
    }

    pub(super) fn authz_disabled(&self) -> bool {
        self.authz_depth.get() > 0
    }

    pub(super) fn allows(&self) -> bool {
        // Phase 5 decides when the depth is zero. Until then the call is allowed.
        let _disabled = self.authz_disabled();
        true
    }
}

/// Decrements the authorization-disabled depth when dropped.
#[must_use = "dropping the guard re-enables authorization checks"]
pub struct AuthzGuard {
    depth: Rc<Cell<u32>>,
}

impl Drop for AuthzGuard {
    fn drop(&mut self) {
        self.depth.set(self.depth.get().saturating_sub(1));
    }
}
