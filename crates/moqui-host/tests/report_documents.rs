//! Mixed-index reporting uses authorized document types and immutable row constraints before
//! retrieval, totals, paging and document expansion. Framework rows keep this fixture small.
#![cfg(all(unix, feature = "embedded-postgres", feature = "axum"))]
mod common;
use example as _;
use moqui_core::{
    context::{ExecutionContext, ExecutionContextFactory},
    native::{Parameters, ServiceInfo},
    report, search,
    transaction::TransactionAttribute,
};
use moqui_lib::object::Value;
use serde_json::{json, Value as Json};
use std::{collections::BTreeSet, sync::Arc};
fn parameters(value: Json) -> Parameters {
    match moqui_lib::json::from_json(&value) {
        Value::Map(map) => map.into_iter().collect(),
        _ => panic!("object"),
    }
}
fn service() -> ServiceInfo<'static> {
    ServiceInfo {
        name: "org.moqui.report.ReportServices.test#MixedDocuments",
        service_type: "rust",
        location: "test",
        method: "",
    }
}
fn viewer<T>(factory: &Arc<ExecutionContextFactory>, name: &str, body: impl FnOnce(&ExecutionContext) -> T) -> T {
    let ec = factory.new_context();
    let _entered = ec.enter();
    moqui_core::user::internal_login_user(name).unwrap();
    body(&ec)
}
fn options() -> Json {
    json!({"datasetId":"scoped-mixed","from":"now-1h","thru":"now+1h","pageSize":100,"sort":["amount"]})
}

#[test]
fn mixed_index_documents_keep_type_row_link_and_dialog_boundaries() {
    verify_documents(None);
}

#[test]
#[ignore = "requires MONITOR_READER_OPENSEARCH_URL, for example http://127.0.0.1:19200"]
fn mixed_index_documents_keep_type_row_link_and_dialog_boundaries_opensearch() {
    let url = std::env::var("MONITOR_READER_OPENSEARCH_URL").expect("set MONITOR_READER_OPENSEARCH_URL");
    verify_documents(Some(&url));
}

fn verify_documents(opensearch_url: Option<&str>) {
    moqui_host::logging::install();
    let mut fixture = String::from(
        r#"<entity-facade-xml type="seed">
      <moqui.security.UserAccount userId="DOC_BOTH" username="doc.both" currentPassword="16ac58bbfa332c1c55bd98b53e60720bfa90d394" passwordHashType="SHA" disabled="N"/>
      <moqui.security.UserAccount userId="DOC_PARTY" username="doc.party" currentPassword="16ac58bbfa332c1c55bd98b53e60720bfa90d394" passwordHashType="SHA" disabled="N"/>
      <moqui.security.UserGroup userGroupId="DOC_SHARED" description="Scoped document readers"/>
      <moqui.security.UserGroup userGroupId="DOC_BOTH" description="Both document types"/>
      <moqui.security.UserGroupMember userGroupId="DOC_SHARED" userId="DOC_BOTH" fromDate="0"/>
      <moqui.security.UserGroupMember userGroupId="DOC_SHARED" userId="DOC_PARTY" fromDate="0"/>
      <moqui.security.UserGroupMember userGroupId="DOC_BOTH" userId="DOC_BOTH" fromDate="0"/>
      <moqui.security.ArtifactGroup artifactGroupId="DOC_READ" description="Authorized reporting fixture"/>
      <moqui.security.ArtifactGroupMember artifactGroupId="DOC_READ" artifactName="org.moqui.report.dataset.scoped-mixed" artifactTypeEnumId="AT_SERVICE" inheritAuthz="N"/>
      <moqui.security.ArtifactGroupMember artifactGroupId="DOC_READ" artifactName="org.moqui.report.ReportServices.*" artifactTypeEnumId="AT_SERVICE" nameIsPattern="Y" inheritAuthz="Y"/>
      <moqui.security.ArtifactGroupMember artifactGroupId="DOC_READ" artifactName=".*" artifactTypeEnumId="AT_XML_SCREEN" nameIsPattern="Y" inheritAuthz="Y"/>
      <moqui.security.ArtifactAuthz artifactAuthzId="DOC_READ_SHARED" artifactGroupId="DOC_READ" userGroupId="DOC_SHARED" authzTypeEnumId="AUTHZT_ALLOW" authzActionEnumId="AUTHZA_ALL"/>
      <moqui.basic.EnumerationType enumTypeId="DOC_BOTH" description="Both-reader rows"/>
      <moqui.basic.EnumerationType enumTypeId="DOC_PARTY" description="Party-reader rows"/>
      <moqui.basic.EnumerationType enumTypeId="DOC_OTHER" description="Another owner's rows"/>
      <moqui.basic.Enumeration enumId="PARTY_A" enumTypeId="DOC_BOTH" enumCode="Visible party" description="Common business record" sequenceNum="3"/>
      <moqui.basic.Enumeration enumId="PARTY_NOLINK" enumTypeId="DOC_BOTH" enumCode="NoLink" description="Common business record" sequenceNum="7"/>
      <moqui.basic.Enumeration enumId="PARTY_BLOCKED" enumTypeId="DOC_BOTH" enumCode="Blocked same owner" description="BLOCKED_SAME_OWNER" sequenceNum="500"/>
      <moqui.basic.Enumeration enumId="PARTY_OTHER" enumTypeId="DOC_OTHER" enumCode="Visible secret" description="SECRET_OTHER_OWNER" sequenceNum="13"/>
      <moqui.basic.Enumeration enumId="PARTY_LIMITED" enumTypeId="DOC_PARTY" enumCode="Limited party" description="Own allowed type" sequenceNum="11"/>
      <moqui.basic.Enumeration enumId="PRODUCT_A" enumTypeId="DOC_BOTH" enumCode="Visible product" description="Common business record" sequenceNum="5"/>
      <moqui.basic.Enumeration enumId="PRODUCT_FORBIDDEN" enumTypeId="DOC_PARTY" enumCode="Forbidden product" description="SECRET_FORBIDDEN_TYPE" sequenceNum="50"/>
    "#,
    );
    for (id, title, group) in [
        ("ReportParty", "Party", "DOC_SHARED"),
        ("ReportProduct", "Product", "DOC_BOTH"),
    ] {
        fixture.push_str(&format!(r#"
          <moqui.entity.document.DataDocument dataDocumentId="{id}" indexName="report_mixed" primaryEntityName="moqui.basic.Enumeration" documentTitle="{title}: ${{enumCode}}"/>
          <moqui.entity.document.DataDocumentField dataDocumentId="{id}" fieldSeqId="01" fieldPath="enumId" fieldType="keyword"/>
          <moqui.entity.document.DataDocumentField dataDocumentId="{id}" fieldSeqId="02" fieldPath="enumCode" fieldType="text" searchRole="title" sortable="Y"/>
          <moqui.entity.document.DataDocumentField dataDocumentId="{id}" fieldSeqId="03" fieldPath="enumTypeId" fieldType="keyword"/>
          <moqui.entity.document.DataDocumentField dataDocumentId="{id}" fieldSeqId="04" fieldPath="description" fieldType="text"/>
          <moqui.entity.document.DataDocumentField dataDocumentId="{id}" fieldSeqId="05" fieldPath="sequenceNum" fieldNameAlias="amount" fieldType="long"/>
          <moqui.entity.document.DataDocumentField dataDocumentId="{id}" fieldSeqId="06" fieldPath="lastUpdatedStamp" fieldNameAlias="time" fieldType="date"/>
          <moqui.entity.document.DataDocumentUserGroup dataDocumentId="{id}" userGroupId="ADMIN"/>
          <moqui.entity.document.DataDocumentUserGroup dataDocumentId="{id}" userGroupId="{group}"/>
          <moqui.entity.document.DataDocumentLink dataDocumentId="{id}" linkSeqId="01" label="Open {title}" linkUrl="/fixture/{id}/${{enumId}}" urlType="plain" linkCondition="enumCode !== 'NoLink'"/>
        "#));
    }
    fixture.push_str("</entity-facade-xml>");
    let prefix = format!("report_documents_{}_", std::process::id());
    let gate = match opensearch_url {
        Some(url) => common::start_with_conf_properties(
            "report-documents-os",
            Some(&fixture),
            "<search-facade backend=\"opensearch\"/>",
            &[
                ("search_backend", "opensearch"),
                ("elasticsearch_url", url),
                ("elasticsearch_index_prefix", &prefix),
                ("opensearch_run", "false"),
            ],
        ),
        None => common::start_with_data("report-documents", &fixture),
    };
    let factory = gate.server.factory();
    let facade = search::installed(factory).unwrap();
    viewer(factory, "john.doe", |ec| {
        let entity = factory.entity().unwrap();
        {
            let _setup = ec.disable_authz();
            for (kind, ids) in [
                (
                    "ReportParty",
                    vec![
                        "PARTY_A",
                        "PARTY_NOLINK",
                        "PARTY_BLOCKED",
                        "PARTY_OTHER",
                        "PARTY_LIMITED",
                    ],
                ),
                ("ReportProduct", vec!["PRODUCT_A", "PRODUCT_FORBIDDEN"]),
            ] {
                let ids = ids.into_iter().map(str::to_owned).collect::<Vec<_>>();
                let docs = moqui_core::entity::document::documents_by_ids(&entity, kind, &ids).unwrap();
                assert_eq!(docs.len(), ids.len());
                assert_eq!(facade.index_documents(&docs).unwrap(), ids.len());
            }
        }
        let tx = entity.transactions_for("moqui.report.Definition").unwrap();
        tx.run(TransactionAttribute::UseOrBegin,None,||report::save_definition(ec,&service(),&parameters(json!({"definition":{"id":"scoped-mixed","type":"dataset","title":"Mixed scoped records","userGroupId":"DOC_SHARED","body":{"indexName":"report_mixed","timeField":"time","rowFilter":{"field":{"name":"amount","op":"range","lt":100}},"contextFilters":[{"field":"enumTypeId","contextKey":"userId"}]}}})))).unwrap();
        let raw = report::documents(
            ec,
            &json!({"datasetId":"index:report_mixed","from":"now-1h","thru":"now+1h","pageSize":100}),
        )
        .unwrap();
        assert_eq!(
            raw["total"], 7,
            "administrator's derived index covers both actual DataDocuments"
        );
    });
    viewer(factory, "doc.both", |ec| {
        ec.user_context().insert("userId".into(), Value::string("DOC_OTHER"));
        let page = report::documents(ec, &options()).unwrap();
        assert_eq!(page["total"], 3);
        let hits = page["hits"].as_array().unwrap();
        assert_eq!(
            hits.iter()
                .map(|hit| hit["documentId"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["ReportParty", "ReportProduct"])
        );
        let party = hits.iter().find(|hit| hit["id"] == "PARTY_A").unwrap();
        assert_eq!(party["documentTitle"], "Party: Visible party");
        assert_eq!(party["links"][0]["url"], "/fixture/ReportParty/PARTY_A");
        let product = hits.iter().find(|hit| hit["id"] == "PRODUCT_A").unwrap();
        assert_eq!(product["documentTitle"], "Product: Visible product");
        assert!(
            hits.iter().find(|hit| hit["id"] == "PARTY_NOLINK").unwrap()["links"]
                .as_array()
                .unwrap()
                .is_empty(),
            "link conditions are evaluated separately for each authorized document"
        );
        let mut query = options();
        query["pageSize"] = json!(1);
        let mut ids = BTreeSet::new();
        loop {
            let page = report::documents(ec, &query).unwrap();
            assert_eq!(page["total"], 3);
            let hits = page["hits"].as_array().unwrap();
            if hits.is_empty() {
                break;
            }
            assert!(ids.insert(hits[0]["id"].as_str().unwrap().to_owned()));
            query["cursor"] = page["nextCursor"].clone();
        }
        assert_eq!(ids.len(), 3, "pagination counts only authorized rows across both types");
        let aggregated=report::execute_query(ec,&json!({"datasetId":"scoped-mixed","from":"now-1h","thru":"now+1h","aggs":{"total":{"sum":{"field":"amount"}},"types":{"terms":{"field":"_document"},"aggs":{"amount":{"sum":{"field":"amount"}}}}}})).unwrap();
        let frame: report::frame::Frame = serde_json::from_value(aggregated["frames"][0].clone()).unwrap();
        let rows = frame.rows();
        assert_eq!(
            rows.len(),
            2,
            "both authorized document types must have a bucket alongside root metrics: {aggregated}"
        );
        let per_type = rows
            .iter()
            .map(|row| (row["types"].as_str().unwrap(), row["amount"].as_f64().unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(per_type["ReportParty"], 10.0, "{aggregated}");
        assert_eq!(per_type["ReportProduct"], 5.0, "{aggregated}");
        assert_eq!(per_type.values().sum::<f64>(), 15.0);
        assert!(
            rows.iter().all(|row| row["total"].as_f64() == Some(15.0)),
            "the ungrouped scalar total uses the same authorized population: {aggregated}"
        );
        assert_eq!(aggregated["sourceCount"], 3);
        for (id, total) in [("PARTY_A", 1), ("PARTY_OTHER", 0), ("PARTY_BLOCKED", 0)] {
            let mut one = options();
            one["query"] = json!({"field":{"name":"_id","op":"eq","value":id}});
            assert_eq!(
                report::documents(ec, &one).unwrap()["total"],
                total,
                "single-document expansion cannot bypass filters"
            );
        }
        let mut ranked = options();
        ranked["queryString"] = json!("Visible");
        ranked["retrievalMode"] = json!("hybrid");
        if facade.capabilities().contains(&search::Capability::Hybrid) {
            let page = report::documents(ec, &ranked).unwrap();
            assert_eq!(page["total"], 2);
            assert!(page["hits"]
                .as_array()
                .unwrap()
                .iter()
                .all(|hit| ids.contains(hit["id"].as_str().unwrap())));
        } else {
            assert!(report::documents(ec, &ranked).is_err());
        }
        ranked["retrievalMode"] = json!("semantic");
        if !facade.capabilities().contains(&search::Capability::Semantic) {
            assert!(
                report::documents(ec, &ranked).is_err(),
                "unsupported semantic mode must not silently become lexical"
            );
        }
        assert!(
            report::documents(
                ec,
                &json!({"datasetId":"index:report_mixed","from":"now-1h","thru":"now+1h"})
            )
            .is_err(),
            "derived index may not bypass the scoped alias"
        );
    });
    viewer(factory, "doc.party", |ec| {
        let page = report::documents(ec, &options()).unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["hits"][0]["id"], "PARTY_LIMITED");
        let total=report::execute_query(ec,&json!({"datasetId":"scoped-mixed","from":"now-1h","thru":"now+1h","aggs":{"amount":{"sum":{"field":"amount"}}}})).unwrap();
        assert_eq!(
            total["frames"][0]["fields"][0]["values"][0].as_f64(),
            Some(11.0),
            "forbidden Product type must be removed before aggregation"
        );
        let mut forced = options();
        forced["documentType"] = json!("ReportProduct");
        forced["query"] = json!({"field":{"name":"_id","op":"eq","value":"PRODUCT_FORBIDDEN"}});
        assert_eq!(report::documents(ec, &forced).unwrap()["total"], 0);
        assert!(report::documents(
            ec,
            &json!({"datasetId":"ReportProduct","from":"now-1h","thru":"now+1h"})
        )
        .is_err());
    });
    let auth = common::basic("doc.both", "moqui");
    let path =
        "/apps/dashboards/Explore.html?datasetId=scoped-mixed&from=now-1h&thru=now%2B1h&queryString=_id%3APARTY_A";
    let (status, _, page) = common::exchange(
        gate.server.address(),
        &common::http_request("GET", path, &[("Authorization", &auth)], ""),
    );
    assert_eq!(status, 200, "{page}");
    assert!(
        page.contains("Party: Visible party") && page.contains("View document") && page.contains("PARTY_A"),
        "{page}"
    );
    assert!(
        !page.contains("SECRET_OTHER_OWNER") && !page.contains("BLOCKED_SAME_OWNER"),
        "dialog data must come only from authorized result rows"
    );
}
