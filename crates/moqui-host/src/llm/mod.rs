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

//! The host routes of the agents (plans/LLM-A2A-Assist.md 4.5), under the `llm` feature: the event stream of a run
//! and its catch-up ([`sse`]). The drawer's JSON calls are transitions of the webroot `assist` subscreen (phase 13.5);
//! the A2A, MCP and gateway adapters join here in their phases.

pub(crate) mod sse;

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::http::HttpState;

/// Add `GET /assist/runs/{runId}/events` (SSE) and `GET /assist/runs/{runId}/events.json` to `app`.
pub(crate) fn routes(app: Router<Arc<HttpState>>) -> Router<Arc<HttpState>> {
    app.route("/assist/runs/{run_id}/events", get(sse::events))
        .route("/assist/runs/{run_id}/events.json", get(sse::events_json))
}
