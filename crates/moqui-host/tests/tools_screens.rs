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

//! Every tools screen, System and Tools, renders in `html` for an admin user on the seeded embedded Postgres: each
//! screen file under the two apps with no parameters, then the detail screens with the records they show. A screen
//! whose `render-modes` leave out `html` answers that html is not supported, as Java does. The screens that wait for
//! a later phase or are not ported show their message: the Groovy shell, and the log viewer on the Postgres search
//! backend.

#![cfg(all(unix, feature = "embedded-postgres", feature = "postgres", feature = "axum"))]

mod common;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use common::{basic, exchange, http_request, start_with_data};

/// The records the detail screens show.
const DATA: &str = r#"<entity-facade-xml type="seed">
    <moqui.security.UserAccount userId="GATE_TOTP_USER" username="gate.totp" userFullName="Gate Totp"/>
    <moqui.basic.print.NetworkPrinter networkPrinterId="GATE_PRINTER" serverHost="localhost" printerName="gate" description="Gate printer"/>
    <!-- A created date ahead, so the seed job clean_PrintJobData_daily, which runs on the first scheduler pass, keeps it. -->
    <moqui.basic.print.PrintJob printJobId="GATE_PJ" networkPrinterId="GATE_PRINTER" statusId="PtjNotSent" jobName="Gate job"
        contentType="text/plain" createdDate="2999-01-02 03:04:05"/>
    <moqui.service.job.ServiceJobRun jobRunId="GATE_RUN" jobName="clean_ArtifactData_daily" hasError="N"
        startTime="2026-01-02 03:04:05" endTime="2026-01-02 03:04:06"/>
    <moqui.service.message.SystemMessageType systemMessageTypeId="GateSmsg" description="Gate message type"/>
    <moqui.service.message.SystemMessageRemote systemMessageRemoteId="GateRemote" description="Gate remote"/>
    <moqui.service.message.SystemMessage systemMessageId="GATE_MSG" systemMessageTypeId="GateSmsg" statusId="SmsgProduced"
        isOutgoing="Y" messageText="{}"/>
    <moqui.entity.sync.EntitySync entitySyncId="GATE_SYNC" description="Gate sync" statusId="EsNotStarted" forPull="N"/>
    <moqui.server.Visit visitId="GATE_VISIT" userId="EX_JOHN_DOE" clientIpAddress="127.0.0.1" fromDate="2026-01-02 03:04:05"/>
    <moqui.server.instance.AppInstance appInstanceId="GATE_AI" instanceName="gate" hostName="gate.example"/>
    <moqui.entity.view.DbViewEntity dbViewEntityName="GateView" packageName="gate.view" isDataView="Y"/>
    <moqui.entity.view.DbViewEntityMember dbViewEntityName="GateView" entityAlias="MASTER" entityName="moqui.basic.Enumeration"/>
    <moqui.entity.document.DataDocument dataDocumentId="GateDoc" documentName="Gate Document" primaryEntityName="moqui.basic.Enumeration"
        indexName="gate"/>
    <moqui.entity.document.DataDocumentField dataDocumentId="GateDoc" fieldSeqId="01" fieldPath="enumId"/>
    <moqui.entity.document.DataDocumentField dataDocumentId="GateDoc" fieldSeqId="02" fieldPath="description"/>
</entity-facade-xml>"#;

/// Detail screens with the parameters of a record in [`DATA`] or the seed data.
const DETAILS: &[&str] = &[
    "/System/Cache/CacheElements?cacheName=l10n.message",
    "/System/Print/PrintJob/PrintJobDetail?printJobId=GATE_PJ",
    "/System/ServiceJob/JobRuns/JobRunDetail?jobRunId=GATE_RUN",
    "/System/ServiceJob/Jobs/ServiceJobDetail?jobName=clean_ArtifactData_daily",
    "/System/Security/UserAccount/UserAccountDetail?userId=EX_JOHN_DOE",
    // Its render sets up a TOTP factor for the user, so it is not the user the test signs in as.
    "/System/Security/UserAccount/UserAccountDetail/VerifyTotp?userId=GATE_TOTP_USER",
    "/System/Security/UserGroup/UserGroupDetail?userGroupId=ADMIN",
    "/System/Security/UserGroup/GroupUsers?userGroupId=ADMIN",
    "/System/Security/ArtifactGroup/ArtifactGroupDetail?artifactGroupId=SYSTEM_APP",
    "/System/SystemMessage/Type/MessageTypeDetail?systemMessageTypeId=GateSmsg",
    "/System/SystemMessage/Remote/MessageRemoteDetail?systemMessageRemoteId=GateRemote",
    "/System/SystemMessage/Message/SystemMessageDetail?systemMessageId=GATE_MSG",
    "/System/SystemMessage/Message/SystemMessageDetail/EditMessageText?systemMessageId=GATE_MSG",
    "/System/EntitySync/EntitySyncDetail?entitySyncId=GATE_SYNC",
    "/System/EntitySync/EntitySyncHistory?entitySyncId=GATE_SYNC",
    "/System/Visit/VisitDetail?visitId=GATE_VISIT",
    "/System/Instance/InstanceDetail?appInstanceId=GATE_AI",
    "/System/DataDocument/Edit/EditDataDocument?dataDocumentId=GateDoc",
    "/System/DataDocument/Edit/ViewDataDocument?dataDocumentId=GateDoc",
    "/System/DataDocument/Edit/DataDocFields?dataDocumentId=GateDoc",
    "/System/DataDocument/Edit/DataDocFields/FieldList?dataDocumentId=GateDoc",
    "/System/Cache/CacheList?filterRegexp=l10n",
    "/Tools/Entity/DataEdit/EntityDetail?selectedEntity=moqui.basic.Enumeration",
    "/Tools/Entity/DataEdit/EntityDataFind?selectedEntity=moqui.basic.Enumeration",
    "/Tools/Entity/DataEdit/EntityDataEdit?selectedEntity=moqui.basic.Enumeration&enumId=PtjNotSent",
    "/Tools/Entity/DataEdit/EntityList?viewOption=master",
    "/Tools/Entity/SpeedTest?baseCalls=10",
    "/Tools/Entity/TableStats?viewOption=non-empty&filterRegexp=moqui.basic",
    "/Tools/AutoScreen/AutoFind?aen=moqui.basic.Enumeration",
    "/Tools/AutoScreen/AutoEdit/AutoEditMaster?aen=moqui.basic.StatusType&statusTypeId=PrintJob",
    "/Tools/AutoScreen/AutoEdit/AutoEditDetail?aen=moqui.basic.StatusType&den=moqui.basic.StatusItem&statusTypeId=PrintJob",
    "/Tools/DataView/ViewDbView?dbViewEntityName=GateView",
    "/Tools/DataView/EditDbView?dbViewEntityName=GateView",
    "/Tools/Service/ServiceDetail?serviceName=org.moqui.impl.BasicServices.convert%23Uom",
    "/Tools/Service/ServiceRun?serviceName=org.moqui.impl.BasicServices.echo%23Data",
    "/Tools/Service/ServiceReference?serviceName=PrintServices",
    "/Tools/ArtifactStats?levels=1",
];

/// The screen paths under `dir`, the subscreen directories walked as the screen facade walks them, with whether the
/// screen renders in html (`render-modes` leaves it out).
fn screen_paths(dir: &Path, prefix: &str, out: &mut Vec<(String, bool)>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if path.extension().is_some_and(|ext| ext == "xml") {
            let text = std::fs::read_to_string(&path).unwrap();
            let html = text.split_once("render-modes=\"").is_none_or(|(_, rest)| {
                rest.split('"')
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .any(|mode| mode == "html")
            });
            out.push((format!("{prefix}/{name}"), html));
            let sub = path.with_extension("");
            if sub.is_dir() {
                screen_paths(&sub, &format!("{prefix}/{name}"), out);
            }
        }
    }
}

fn get(address: SocketAddr, auth: &str, path: &str) -> (u16, String) {
    let (base, query) = path.split_once('?').map_or((path, ""), |(base, query)| (base, query));
    let target = if query.is_empty() {
        format!("{base}.html")
    } else {
        format!("{base}.html?{query}")
    };
    let (status, _, page) = exchange(address, &http_request("GET", &target, &[("Authorization", auth)], ""));
    (status, page)
}

fn failure(path: &str, status: u16, page: &str) -> String {
    let snippet: String = page.find("Error rendering").map_or_else(
        || page.chars().take(400).collect(),
        |at| page[at..].chars().take(400).collect(),
    );
    format!("{path}: {status}: {snippet}")
}

#[test]
fn every_tools_screen_renders_in_html_for_an_admin() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/tools/screen");
    let mut paths = Vec::new();
    for app in ["System", "Tools"] {
        paths.push((format!("/{app}"), true));
        screen_paths(&root.join(app), &format!("/{app}"), &mut paths);
    }

    let gate = start_with_data("tools-screens", DATA);
    let address = gate.server.address();
    let john = basic("john.doe", "moqui");
    let mut failures = Vec::new();
    let mut checked = 0;
    for (path, html) in &paths {
        // The speed test runs its whole set at render; the parameter case below runs a small one.
        if path.ends_with("/SpeedTest") {
            continue;
        }
        let (status, page) = get(address, &john, path);
        checked += 1;
        if !html {
            if status != 404 || !page.contains("render mode html is not supported") {
                failures.push(failure(path, status, &page));
            }
            continue;
        }
        if status != 200 || page.contains("Error rendering") {
            failures.push(failure(path, status, &page));
        }
    }
    for path in DETAILS {
        let (status, page) = get(address, &john, path);
        checked += 1;
        if status != 200 || page.contains("Error rendering") {
            failures.push(failure(path, status, &page));
        }
    }
    assert!(checked > 140, "{checked} screens");
    assert!(
        failures.is_empty(),
        "{} of {checked} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );

    // Screens of a later phase or not ported show why.
    let (_, shell) = get(address, &john, "/Tools/GroovyShell");
    assert!(shell.contains("The Groovy Shell is not available"), "{shell}");
    let (_, logs) = get(address, &john, "/System/LogViewer");
    assert!(logs.contains("has no log index moqui_logs"), "{logs}");

    // The detail screens show their records.
    let (_, entity) = get(
        address,
        &john,
        "/Tools/Entity/DataEdit/EntityDetail?selectedEntity=moqui.basic.Enumeration",
    );
    assert!(
        entity.contains("Entity Detail for Enumeration (moqui.basic.Enumeration)"),
        "{entity}"
    );
    assert!(
        entity.contains("enumTypeId") && entity.contains("moqui.basic.EnumerationType"),
        "{entity}"
    );
    // The conf caches, the screen definitions, and the entity list caches are listed together.
    let (_, cache) = get(address, &john, "/System/Cache/CacheList?filterRegexp=screen.def");
    assert!(cache.contains("cacheName=screen.definition"), "{cache}");
    let (_, cache) = get(
        address,
        &john,
        "/System/Cache/CacheList?filterRegexp=list.moqui.basic.Enum",
    );
    assert!(
        cache.contains("cacheName=entity.record.list.moqui.basic.Enumeration"),
        "{cache}"
    );
    let (_, job) = get(
        address,
        &john,
        "/System/Print/PrintJob/PrintJobDetail?printJobId=GATE_PJ",
    );
    assert!(job.contains("Gate job") && job.contains("Not Sent"), "{job}");
}
