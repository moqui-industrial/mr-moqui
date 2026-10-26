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

use super::*;
use crate::search::mapping::FieldMapping;
use crate::search::parse::parse_query_string;

fn sales() -> DocumentMapping {
    let field = |kind| FieldMapping { kind, sortable: false };
    let mut root = ObjectMapping::default();
    for (name, kind) in [
        ("orderId", FieldKind::Keyword),
        ("productId", FieldKind::Keyword),
        ("itemTypeEnumId", FieldKind::Keyword),
        ("placedDate", FieldKind::Date),
        ("itemSaleTotal", FieldKind::Double),
        ("statusId", FieldKind::Keyword),
    ] {
        root.fields.insert(name.into(), field(kind));
    }
    root.fields.insert(
        "combinedName".into(),
        FieldMapping {
            kind: FieldKind::Text,
            sortable: true,
        },
    );
    let mut issuances = ObjectMapping::default();
    issuances
        .fields
        .insert("issuedSaleTotal".into(), field(FieldKind::Double));
    root.children.insert("issuances".into(), issuances);
    DocumentMapping {
        document_id: "MantleSalesOrderItem".into(),
        index_name: Some("mantle_sales".into()),
        physical_name: "mantle_sales_order_item".into(),
        root,
        manual_mapping_service: None,
        display: Default::default(),
    }
}

#[test]
fn sales_order_summary_body_matches_the_java_service() {
    let mapping = sales();
    let aggs = Agg::map_from_value(&moqui_lib::json::from_json(
        &serde_json::from_str(
            r#"{"DateOuter":{"date_histogram":{"field":"placedDate","calendar_interval":"1w","time_zone":"UTC",
                    "format":"dd MMM yyyy","order":{"_key":"desc"},"offset":"-1d"},
                  "aggs":{"orderCount":{"cardinality":{"field":"orderId","precision_threshold":40000}},
                          "ItemProduct":{"filter":{"term":{"itemTypeEnumId":"ItemProduct"}},
                             "aggs":{"productIssuedTotal":{"sum":{"field":"issuances.issuedSaleTotal"}}}}}}}"#,
        )
        .unwrap(),
    ))
    .unwrap();
    let json = aggs_json(&mapping, &aggs).unwrap();
    assert_eq!(
        json,
        json!({"DateOuter": {
        "date_histogram": {"field": "placedDate", "calendar_interval": "1w", "time_zone": "UTC",
            "format": "dd MMM yyyy", "offset": "-1d", "order": {"_key": "desc"}, "min_doc_count": 0},
        "aggs": {
            "ItemProduct": {"filter": {"term": {"itemTypeEnumId": "ItemProduct"}},
                "aggs": {"productIssuedTotal": {"sum": {"field": "issuances.issuedSaleTotal"}}}},
            "orderCount": {"cardinality": {"field": "orderId", "precision_threshold": 40000}}
        }}})
    );
    let range = Query::range(
        "placedDate",
        Some(Value::string("2026-01-01")),
        Some(Value::I64(1_769_904_000_000)),
    );
    assert_eq!(
        query_json(&mapping, &range),
        json!({"range": {"placedDate": {"gte": 1_767_225_600_000_i64, "lt": 1_769_904_000_000_i64}}})
    );
}

#[test]
fn min_and_max_aggregations_round_trip() {
    let mapping = sales();
    let aggs = Agg::map_from_value(&moqui_lib::json::from_json(
        &serde_json::from_str(r#"{"lo":{"min":{"field":"itemSaleTotal"}},"hi":{"max":{"field":"itemSaleTotal"}}}"#)
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        aggs_json(&mapping, &aggs).unwrap(),
        json!({"lo": {"min": {"field": "itemSaleTotal"}}, "hi": {"max": {"field": "itemSaleTotal"}}})
    );
    let out = read_aggs(&aggs, &json!({"lo": {"value": 2.5}, "hi": {"value": null}}));
    assert!(matches!(out["lo"], Value::Decimal(_)));
    assert_eq!(out["hi"], Value::Null);
}

#[test]
fn query_strings_compile_to_query_string_and_nested() {
    let mapping = sales();
    let query = Query::all(vec![
        parse_query_string("*:jo* AND combinedName:\"jo smith\"").unwrap(),
        Query::child("issuances", Query::range("issuedSaleTotal", Some(Value::I64(1)), None)),
    ]);
    let json = query_json(&mapping, &query);
    let text = json.to_string();
    assert!(text.contains(r#""query":"jo*""#), "{text}");
    assert!(text.contains(r#""query":"\"jo smith\"""#), "{text}");
    assert!(
        text.contains(r#""nested":{"path":"issuances","query":{"range":{"issuances.issuedSaleTotal":{"gte":1}}}"#),
        "{text}"
    );
    let mut request = SearchRequest::new(Query::MatchAll);
    request.sort = vec![crate::search::SortField::parse("-combinedName").unwrap()];
    let body = search_json(&mapping, &request);
    assert_eq!(
        body["sort"],
        json!([{"combinedName.keyword": {"order": "desc", "missing": "_last", "mode": "max", "unmapped_type": "keyword"}}])
    );
    assert_eq!(body["track_total_hits"], json!(true));
}

#[test]
fn reads_bucket_responses() {
    let mapping = sales();
    let aggs = Agg::map_from_value(&moqui_lib::json::from_json(
        &serde_json::from_str(
            r#"{"T":{"terms":{"field":"productId"},"aggs":{"s":{"sum":{"field":"itemSaleTotal"}},
                    "F":{"filter":{"term":{"itemTypeEnumId":"ItemProduct"}}}}}}"#,
        )
        .unwrap(),
    ))
    .unwrap();
    let _ = aggs_json(&mapping, &aggs).unwrap();
    let response =
        json!({"T": {"buckets": [{"key": "P1", "doc_count": 2, "s": {"value": 130.5}, "F": {"doc_count": 1}}]}});
    let out = read_aggs(&aggs, &response);
    let Value::Map(terms) = &out["T"] else { panic!() };
    let Value::List(buckets) = &terms["buckets"] else {
        panic!()
    };
    let Value::Map(bucket) = &buckets[0] else { panic!() };
    assert_eq!(to_plain_string(&bucket["key"]), "P1");
    assert_eq!(bucket["doc_count"], Value::I64(2));
    assert_eq!(to_plain_string(&bucket["s"]), "130.5");
    let Value::Map(filter) = &bucket["F"] else { panic!() };
    assert_eq!(filter["doc_count"], Value::I64(1));
}

#[test]
fn a_sort_resolves_against_every_type_and_names_an_unmapped_type() {
    let sales = sales();
    let mut party = sales.clone();
    party.document_id = "MantleParty".into();
    party.physical_name = "mantle_party".into();
    party.root.fields.clear();
    party.root.fields.insert(
        "pseudoId".into(),
        FieldMapping {
            kind: FieldKind::Keyword,
            sortable: false,
        },
    );
    let mut request = SearchRequest::new(Query::MatchAll);
    request.sort = ["combinedName", "-placedDate", "pseudoId", "_score"]
        .iter()
        .map(|text| SortField::parse(text).unwrap())
        .collect();
    // The party type is first and has none of the sales fields.
    let body = search_body(&[&party, &sales], &request);
    assert_eq!(
        body["sort"],
        json!([
            {"combinedName.keyword": {"order": "asc", "missing": "_last", "mode": "min", "unmapped_type": "keyword"}},
            {"placedDate": {"order": "desc", "missing": "_last", "mode": "max", "unmapped_type": "date"}},
            {"pseudoId": {"order": "asc", "missing": "_last", "mode": "min", "unmapped_type": "keyword"}},
            {"_score": {"order": "asc"}}
        ])
    );
}

/// A one-request-per-connection HTTP server that answers `replies` in order, then stops.
/// Joining it gives the request lines it saw.
fn serve(replies: Vec<(u16, Json)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                if let Some(split) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= split + 4 + length {
                        break;
                    }
                }
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => buf.extend_from_slice(&chunk[..size]),
                }
            }
            let text = String::from_utf8_lossy(&buf);
            seen.push(text.lines().next().unwrap_or_default().to_string());
            let body = body.to_string();
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
        seen
    });
    (url, handle)
}

fn backend(url: &str) -> OpenSearchBackend {
    OpenSearchBackend::new(&ClusterConf {
        url: url.to_string(),
        user: None,
        password: None,
        index_prefix: None,
    })
}

#[test]
fn index_exists_is_false_only_for_a_404() {
    let (url, server) = serve(vec![
        (404, json!({"error": {"type": "index_not_found_exception"}})),
        (503, json!({"error": {"reason": "cluster_block_exception"}})),
        (401, json!({"error": {"reason": "unauthorized"}})),
        (200, json!({"mantle_sales_order_item": {"mappings": {}}})),
    ]);
    let backend = backend(&url);
    let mapping = sales();
    assert!(!backend.index_exists(&mapping).unwrap());
    let err = backend.index_exists(&mapping).unwrap_err();
    assert!(err.message().contains("503"), "{err}");
    assert!(backend.index_exists(&mapping).is_err());
    assert!(backend.index_exists(&mapping).unwrap());
    assert_eq!(server.join().unwrap().len(), 4);
}

#[test]
fn shard_failures_and_partial_deletes_are_errors() {
    let (url, server) = serve(vec![
        (
            200,
            json!({"_shards": {"total": 2, "successful": 1, "failed": 1,
                "failures": [{"reason": {"reason": "No mapping found for [x] in order to sort on"}}]},
                "hits": {"total": {"value": 1}, "hits": []}}),
        ),
        (
            200,
            json!({"_shards": {"total": 1, "successful": 0, "failed": 1}, "aggregations": {}}),
        ),
        (
            200,
            json!({"deleted": 3, "timed_out": false,
                "failures": [{"cause": {"reason": "version conflict"}}]}),
        ),
        (
            200,
            json!({"_shards": {"total": 2, "successful": 2, "failed": 0},
                "hits": {"total": {"value": 0}, "hits": []}}),
        ),
    ]);
    let backend = backend(&url);
    let mapping = Arc::new(sales());
    let err = backend
        .search(&[Arc::clone(&mapping)], &SearchRequest::new(Query::MatchAll))
        .unwrap_err();
    assert!(err.message().contains("No mapping found"), "{err}");
    let err = backend
        .aggregate(&mapping, &Query::MatchAll, &IndexMap::new())
        .unwrap_err();
    assert!(err.message().contains("1 failed"), "{err}");
    let err = backend.delete_matching(&mapping, &Query::MatchAll).unwrap_err();
    assert!(err.message().contains("deleted 3"), "{err}");
    assert!(err.message().contains("version conflict"), "{err}");
    let page = backend
        .search(&[Arc::clone(&mapping)], &SearchRequest::new(Query::MatchAll))
        .unwrap();
    assert_eq!(page.total, 0);
    server.join().unwrap();
}

#[test]
fn a_failing_manual_mapping_service_fails_the_ensure_and_is_tried_again() {
    let ctx = crate::context::ExecutionContextFactory::test_context();
    ctx.factory().set_entity(crate::testing::open());
    let _entered = ctx.enter();
    let missing = || (404, json!({"error": {"type": "index_not_found_exception"}}));
    let (url, server) = serve(vec![missing(), missing()]);
    let backend = backend(&url);
    let mut mapping = sales();
    mapping.manual_mapping_service = Some("moqui.test.NoSuchServices.make#Mapping".into());
    let err = backend.ensure_index(&mapping).unwrap_err();
    assert!(err.message().contains("manual mapping service"), "{err}");
    assert!(lock(&backend.known).is_empty());
    assert!(backend.ensure_index(&mapping).is_err());
    // Each ensure looked again and neither made the index.
    let seen = server.join().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(
        seen.iter()
            .all(|line| line.starts_with("GET /mantle_sales_order_item/_mapping")),
        "{seen:?}"
    );
    let errors = ctx.message().errors().to_vec();
    assert!(errors.is_empty(), "{errors:?}");
}
