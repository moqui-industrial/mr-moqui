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

//! The Vue SFC dialect (plans/LLM-A2A-Assist.md 4.10): a registered placeholder until phase 13.8 builds its sandbox
//! frame (`/assist/sandbox`, its CSP, the activation-gated submit and the watchdog). It is not offered ([`Dialect::available`]
//! is false), and a `write_ui` of kind `vue-sfc` is answered with the reason and the dialect to use instead.

use super::{Dialect, Prepared, Refused, WriteInput};
use crate::context::ExecutionContextFactory;

#[derive(Default)]
pub struct VueSfc;

impl Dialect for VueSfc {
    fn name(&self) -> &'static str {
        "vue-sfc"
    }

    fn label(&self) -> &'static str {
        "Vue SFC"
    }

    fn skill(&self) -> &'static str {
        "vue-sfc"
    }

    fn available(&self) -> bool {
        false
    }

    fn prepare(
        &self,
        _factory: &ExecutionContextFactory,
        _previous: Option<&str>,
        _input: &WriteInput,
        _paths: &dyn super::openui::Paths,
    ) -> Result<Prepared, Refused> {
        Err(Refused {
            error: crate::llm::tools::format::ToolError::new(
                "dialect_unavailable",
                "Vue SFC canvases are not available on this server, so nothing was shown. Write the view with kind \
                 openui (skill openui-lang).",
            )
            .to_text(),
            problems: Vec::new(),
        })
    }
}

crate::submit_dialect!(VueSfc);
