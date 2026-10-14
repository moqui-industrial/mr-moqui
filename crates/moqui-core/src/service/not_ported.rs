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

//! Framework services that are defined but not run yet: LLM and A2A wait for phase 13, and the Docker app instance
//! provisioning of `InstanceServices` is not ported (Decisions in `plans/Implementation-Phases.md`). The definitions
//! keep their parameters; a call fails with a message that names the service, rather than running nothing.

const LLM: &str = "org.moqui.impl.LlmServices.";
const A2A: &str = "org.moqui.impl.A2AServices.";
const INSTANCE: &str = "org.moqui.impl.InstanceServices.";

/// True for a service that is defined but not available in this port.
pub(super) fn runs(name: &str) -> bool {
    reason(name).is_some()
}

/// What a call of `name` reports.
pub(super) fn reason(name: &str) -> Option<String> {
    if name.starts_with(LLM) {
        Some(format!("service {name} is not available: LLM is not available until phase 13"))
    } else if name.starts_with(A2A) {
        Some(format!("service {name} is not available: A2A is not available until phase 13"))
    } else if name.starts_with(INSTANCE) {
        Some(format!(
            "service {name} is not available: Docker app instance provisioning is not ported"
        ))
    } else {
        None
    }
}
