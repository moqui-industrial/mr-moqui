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

//! The tools services on the seeded embedded Postgres, called through the Service Run screen's `runJson` transition as
//! an admin: a print job is created and its status moves to a failed send, a screen document reports that PDF is not
//! available, the clean services delete by a time basis, an entity data snapshot goes
//! out to a zip and back in, and the cache admin screen clears a cache.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

mod common;

use std::net::SocketAddr;

use common::{basic, exchange, http_request, start_with_data};

const DATA: &str = r#"<entity-facade-xml type="seed">
    <moqui.basic.print.NetworkPrinter networkPrinterId="GATE_PRINTER" serverHost="localhost" printerName="gate"/>
    <moqui.basic.print.PrintJob printJobId="GATE_OLD_PJ" networkPrinterId="GATE_PRINTER" statusId="PtjNotSent"
        createdDate="2020-01-02 03:04:05"/>
    <moqui.basic.Uom uomId="GATE_A" description="Gate A" uomTypeEnumId="UT_LENGTH_MEASURE"/>
    <moqui.basic.Uom uomId="GATE_B" description="Gate B" uomTypeEnumId="UT_LENGTH_MEASURE"/>
    <moqui.basic.UomConversion uomConversionId="GATE_AB" uomId="GATE_A" toUomId="GATE_B" conversionFactor="2.5"
        conversionOffset="1" fromDate="2020-01-01 00:00:00"/>
</entity-facade-xml>"#;

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// Run a service through `ServiceRun/runJson`. Returns the status and the body.
fn run(address: SocketAddr, auth: &str, service: &str, pairs: &[(&str, &str)]) -> (u16, String) {
    let mut body = format!("serviceName={}", encode(service));
    for (key, value) in pairs {
        body.push_str(&format!("&{}={}", encode(key), encode(value)));
    }
    let request = http_request(
        "POST",
        "/Tools/Service/ServiceRun/runJson",
        &[("Authorization", auth), ("Content-Type", "application/x-www-form-urlencoded")],
        &body,
    );
    let (status, _, text) = exchange(address, &request);
    (status, text)
}

fn page(address: SocketAddr, auth: &str, path: &str) -> String {
    let (status, _, text) = exchange(address, &http_request("GET", path, &[("Authorization", auth)], ""));
    assert_eq!(status, 200, "{path}: {text}");
    text
}

#[test]
fn tools_services_run_on_postgres() {
    let gate = start_with_data("tools-services", DATA);
    let address = gate.server.address();
    let john = basic("john.doe", "moqui");

    // print#Document: the job is created not sent, then the send records that network printing is phase 12.
    let (_, printed) = run(
        address,
        &john,
        "org.moqui.impl.PrintServices.print#Document",
        &[("networkPrinterId", "GATE_PRINTER"), ("inputStream", "hello printer"), ("contentType", "text/plain"),
            ("jobName", "Gate print")],
    );
    assert!(printed.contains("Network printing is not available until phase 12 (moqui-cups)"), "{printed}");
    let jobs = page(address, &john, "/System/Print/PrintJob/PrintJobList.html?jobName=Gate%20print");
    assert!(jobs.contains("Gate print") && jobs.contains("Send Failed"), "{jobs}");

    // A screen document asks for PDF.
    let (_, pdf) = run(
        address,
        &john,
        "org.moqui.impl.PrintServices.print#ScreenDocument",
        &[("networkPrinterId", "GATE_PRINTER"), ("screenLocation", "component://tools/screen/System/dashboard.xml")],
    );
    assert!(pdf.contains("PDF is not available until phase 12"), "{pdf}");

    // The clean services compare a time basis with the stamp columns.
    let (status, cleaned) = run(address, &john, "org.moqui.impl.ServerServices.clean#PrintJobData", &[("daysToKeep", "30")]);
    assert_eq!(status, 200, "{cleaned}");
    assert!(cleaned.contains("\"printJobsRemoved\": 1"), "{cleaned}");
    let (status, cleaned) = run(address, &john, "org.moqui.impl.ServerServices.clean#ArtifactData", &[("daysToKeep", "90")]);
    assert_eq!(status, 200, "{cleaned}");
    assert!(cleaned.contains("artifactHitsRemoved"), "{cleaned}");

    // Decimal arithmetic, both directions.
    let (_, converted) = run(
        address,
        &john,
        "org.moqui.impl.BasicServices.convert#Uom",
        &[("uomId", "GATE_A"), ("toUomId", "GATE_B"), ("amount", "4")],
    );
    assert!(converted.contains("\"convertedAmount\": 11"), "{converted}");
    let (_, back) = run(
        address,
        &john,
        "org.moqui.impl.BasicServices.convert#Uom",
        &[("uomId", "GATE_B"), ("toUomId", "GATE_A"), ("amount", "11")],
    );
    assert!(back.contains("\"convertedAmount\": 4"), "{back}");

    // An entity data snapshot out to a zip in runtime/db/snapshot, then back in.
    let (status, exported) = run(
        address,
        &john,
        "org.moqui.impl.EntityServices.export#EntityDataSnapshot",
        &[("entitiesToInclude", "moqui.basic.Uom"), ("baseFilename", "GateSnapshot")],
    );
    assert_eq!(status, 200, "{exported}");
    assert!(exported.contains("\"recordsWritten\"") && !exported.contains("\"recordsWritten\": 0"), "{exported}");
    let snapshots = page(address, &john, "/Tools/Entity/DataSnapshot.html");
    assert!(snapshots.contains("GateSnapshot.zip"), "{snapshots}");
    let (status, imported) = run(
        address,
        &john,
        "org.moqui.impl.EntityServices.import#EntityDataSnapshot",
        &[("zipFilename", "GateSnapshot.zip")],
    );
    assert_eq!(status, 200, "{imported}");
    assert!(imported.contains("\"recordsLoaded\"") && !imported.contains("\"recordsLoaded\": 0"), "{imported}");

    // Cache admin: the conversion found above is in the entity list cache, and Clear All empties it.
    let size = |text: &str| -> String {
        let at = text.find(">entity.record.list.moqui.basic.UomConversion<").unwrap_or_else(|| panic!("{text}"));
        let cells: Vec<&str> = text[at..].split("<span").skip(1).take(1).collect();
        cells.first().and_then(|cell| cell.split('>').nth(1)).and_then(|cell| cell.split('<').next()).unwrap_or("").to_string()
    };
    let caches = page(address, &john, "/System/Cache/CacheList.html?filterRegexp=UomConversion");
    assert_ne!(size(&caches), "0", "{caches}");
    let request = http_request(
        "POST",
        "/System/Cache/CacheList/clearAllCaches",
        &[("Authorization", &john), ("Content-Type", "application/x-www-form-urlencoded")],
        "",
    );
    let (status, _, cleared) = exchange(address, &request);
    assert!(status == 200 || status == 302, "{status}: {cleared}");
    let caches = page(address, &john, "/System/Cache/CacheList.html?filterRegexp=UomConversion");
    assert_eq!(size(&caches), "0", "{caches}");
}
