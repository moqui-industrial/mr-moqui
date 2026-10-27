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

//! The default Postgres backend: SDK log events read through authorized Explore, retained
//! HTTP root spans, telemetry retention, and the Elastic proxy staying closed. This binary
//! owns the process logger, so it has one test.

#![cfg(all(unix, feature = "embedded-postgres", feature = "axum"))]

mod common;

// the example component loads with the repository runtime; link its tool factory as the moqui binary does
use example as _;

use std::time::{Duration, Instant};

use common::{basic, exchange, http_request, start_with_data};

#[test]
fn postgres_backend_exports_telemetry_through_the_search_facade() {
    moqui_host::logging::install();
    let marker = format!("pglogmarker{}", std::process::id());
    // Before the server exists: the queue holds it until search attaches.
    tracing::warn!("early boot line {marker}");
    let gate = start_with_data("search-logs-postgres", "<entity-facade-xml type=\"seed\"/>");
    let address = gate.server.address();
    let john = basic("john.doe", "moqui");
    let facade = moqui_core::search::installed(gate.server.factory()).expect("search is installed");
    assert_eq!(facade.backend().name(), "postgres");

    tracing::warn!("postgres log shipping {marker}");
    moqui_core::telemetry::for_factory(gate.server.factory()).flush();
    let deadline = Instant::now() + Duration::from_secs(20);
    let page = loop {
        let path =
            format!("/apps/dashboards/Explore.html?datasetId=otel_logs&queryString={marker}&from=now-1h&thru=now");
        let (status, _, page) = exchange(address, &http_request("GET", &path, &[("Authorization", &john)], ""));
        assert_eq!(status, 200);
        if page.contains(&format!("postgres log shipping {marker}")) || Instant::now() > deadline {
            break page;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(page.contains(&format!("postgres log shipping {marker}")), "{page}");
    assert!(page.contains(&format!("early boot line {marker}")), "{page}");
    assert!(!page.contains("Error rendering"), "{page}");

    // Every HTTP request retains a SERVER root, including requests below the sampling threshold.
    let deadline = Instant::now() + Duration::from_secs(20);
    let total = loop {
        moqui_core::telemetry::for_factory(gate.server.factory()).flush();
        let page = facade
            .search(
                &moqui_core::search::Target::Index("otel_spans".to_string()),
                &moqui_core::search::SearchRequest::new(moqui_core::search::query::Query::eq(
                    "kind",
                    moqui_lib::object::Value::string("SERVER"),
                )),
            )
            .expect("request span search");
        if page.total > 0 || Instant::now() > deadline {
            break page.total;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(total > 0, "no retained HTTP root spans");

    // Retention deletes telemetry logs by their normalized timestamp.
    let target = moqui_core::search::Target::Index("otel_logs".to_string());
    let cutoff = moqui_lib::object::Value::I64(jiff::Timestamp::now().as_millisecond() + 60_000);
    let removed = facade
        .delete_matching(
            &target,
            &moqui_core::search::query::Query::range("timestamp", None, Some(cutoff)),
        )
        .expect("retention delete");
    assert!(removed > 0, "retention deleted nothing");

    // The proxy to an Elastic server stays closed: this backend has none.
    let (status, _, _) = exchange(
        address,
        &http_request("GET", "/search/", &[("Authorization", &john)], ""),
    );
    assert_eq!(status, 404);
    drop(gate);
}
