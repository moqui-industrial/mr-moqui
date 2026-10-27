//! Real alert persistence, concurrency and delivery gates. Every evaluation uses a fresh
//! authenticated context; restart also discards the factory and all in-memory alert state.
#![cfg(all(unix, feature = "embedded-postgres", feature = "axum"))]
mod common;
use example as _;
use moqui_core::{
    context::{ExecutionContext, ExecutionContextFactory},
    entity::document::Document,
    native::{Parameters, ServiceInfo},
    report, search,
    transaction::TransactionAttribute,
};
use moqui_lib::object::Value;
use serde_json::{json, Value as Json};
use std::sync::Arc;

const TOPIC: &str = "M6AlertFixture";
fn document(value: Json) -> Document {
    match moqui_lib::json::from_json(&value) {
        Value::Map(map) => map,
        _ => panic!("object required"),
    }
}
fn parameters(value: Json) -> Parameters {
    document(value).into_iter().collect()
}
fn result(value: Parameters) -> Json {
    moqui_lib::json::to_json(&Value::Map(value.into_iter().collect()))
}
fn service() -> ServiceInfo<'static> {
    ServiceInfo {
        name: "org.moqui.report.ReportServices.test#AlertFixture",
        service_type: "rust",
        location: "test",
        method: "",
    }
}
fn with_owner<T>(factory: &Arc<ExecutionContextFactory>, body: impl FnOnce(&ExecutionContext) -> T) -> T {
    let ec = factory.new_context();
    let _entered = ec.enter();
    moqui_core::user::internal_login_user("john.doe").unwrap();
    body(&ec)
}
fn save(factory: &Arc<ExecutionContextFactory>, id: &str, body: &Json) {
    with_owner(factory, |ec| {
        let entity = factory.entity().unwrap();
        let tx = entity.transactions_for("moqui.report.Definition").unwrap();
        tx.run(TransactionAttribute::UseOrBegin, None, || {
            report::save_definition(
                ec,
                &service(),
                &parameters(json!({"definition":{"id":id,"type":"alert","title":id,"body":body}})),
            )
        })
        .unwrap();
    });
}
fn evaluate(factory: &Arc<ExecutionContextFactory>, id: &str, clock: i64) -> Json {
    with_owner(factory, |ec| {
        ec.set_effective_time(Some(clock));
        result(report::evaluate_rules(ec, &service(), &parameters(json!({"definitionId":id}))).unwrap())
    })
}
fn concurrent(factory: &Arc<ExecutionContextFactory>, id: &str, clock: i64) {
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let threads = (0..4)
        .map(|_| {
            let factory = factory.clone();
            let id = id.to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                evaluate(&factory, &id, clock)
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        assert_eq!(thread.join().unwrap()["failed"], 0);
    }
}
fn rows(factory: &Arc<ExecutionContextFactory>, entity_name: &str, rule: Option<&str>) -> Vec<Json> {
    with_owner(factory, |ec| {
        let _inspection = ec.disable_authz();
        let entity = factory.entity().unwrap();
        let mut find = entity.find(entity_name).unwrap();
        if let Some(rule) = rule {
            find = find.eq("definitionId", Value::string(rule));
        }
        find.list()
            .unwrap()
            .iter()
            .map(|row| {
                Json::Object(
                    row.fields()
                        .iter()
                        .map(|(name, value)| (name.to_string(), moqui_lib::json::to_json(value)))
                        .collect(),
                )
            })
            .collect()
    })
}
fn states(factory: &Arc<ExecutionContextFactory>, rule: &str) -> Vec<String> {
    rows(factory, "moqui.report.AlertInstance", Some(rule))
        .iter()
        .map(|row| row["state"].as_str().unwrap().to_owned())
        .collect()
}
fn notification_count(factory: &Arc<ExecutionContextFactory>, rule: &str) -> usize {
    with_owner(factory, |ec| {
        let _inspection = ec.disable_authz();
        let entity = factory.entity().unwrap();
        entity
            .find("moqui.security.user.NotificationMessage")
            .unwrap()
            .eq("topic", Value::string(TOPIC))
            .list()
            .unwrap()
            .iter()
            .filter(|row| {
                row.get("messageJson")
                    .and_then(Value::as_str)
                    .and_then(|body| serde_json::from_str::<Json>(body).ok())
                    .is_some_and(|body| body["ruleId"] == rule)
            })
            .count()
    })
}
fn points(factory: &Arc<ExecutionContextFactory>, at: i64, value: f64) {
    let facade = search::installed(factory).unwrap();
    let docs=["one","two"].into_iter().map(|series|document(json!({"_id":format!("m6-{series}"),"bucket_start":at,"name":"m6-latency","service.name":series,"granularity":"minute","sum":value,"count":1}))).collect();
    assert_eq!(facade.index_entries("otel_metrics", docs).unwrap(), 2);
}
fn rule(at: i64) -> Json {
    json!({"query":{"datasetId":"otel_metrics","from":(at-86_400_000).to_string(),"thru":(at+30*86_400_000_i64).to_string(),"queryString":"name:m6-latency","aggs":{"service":{"terms":{"field":"service.name","size":20},"aggs":{"value":{"sum":{"field":"sum"}}}}}},"field":"value","labelFields":["service"],"labels":{"team":"operations"},"condition":{"type":"above","threshold":10,"recoveryThreshold":7},"pendingSeconds":120,"keepFiringSeconds":120,"intervalSeconds":60,"contactPoints":[{"id":"header","type":"notification","topic":TOPIC,"userId":"EX_JOHN_DOE","groupBy":["team"],"groupWaitSeconds":120,"repeatSeconds":180}]})
}

#[test]
fn alerts_persist_timers_serialize_workers_and_route_only_complete_observations() {
    moqui_host::logging::install();
    let fixture = r#"<entity-facade-xml type="seed">
      <moqui.service.job.ServiceJob jobName="evaluate_ReportAlerts_minute" paused="Y"/>
      <moqui.security.user.NotificationTopic topic="M6AlertFixture" description="Alert acceptance" isPrivate="Y" receiveNotifications="Y" persistOnSend="Y"/>
      <moqui.security.user.NotificationTopicUser topic="M6AlertFixture" userId="EX_JOHN_DOE" receiveNotifications="Y"/>
    </entity-facade-xml>"#;
    let mut gate = common::start_with_data("report-alerts", fixture);
    let at = "2026-10-26T12:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_millisecond();
    let body = rule(at);
    let id = "persisted-alert";
    points(gate.server.factory(), at, 12.0);
    save(gate.server.factory(), id, &body);
    concurrent(gate.server.factory(), id, at);
    assert_eq!(states(gate.server.factory(), id), ["Pending", "Pending"]);
    assert_eq!(notification_count(gate.server.factory(), id), 0);
    assert_eq!(
        rows(gate.server.factory(), "moqui.report.AlertNotification", Some(id)).len(),
        1,
        "two series are one persisted notification group"
    );
    evaluate(gate.server.factory(), id, at + 60_000);
    let pending = rows(gate.server.factory(), "moqui.report.AlertInstance", Some(id));
    let notification = rows(gate.server.factory(), "moqui.report.AlertNotification", Some(id));
    let old_factory = Arc::downgrade(gate.server.factory());
    gate = gate.restart();
    assert!(
        old_factory.upgrade().is_none(),
        "restart must release the original factory"
    );
    assert_eq!(
        rows(gate.server.factory(), "moqui.report.AlertInstance", Some(id)),
        pending
    );
    assert_eq!(
        rows(gate.server.factory(), "moqui.report.AlertNotification", Some(id)),
        notification
    );
    concurrent(gate.server.factory(), id, at + 120_000);
    assert_eq!(states(gate.server.factory(), id), ["Firing", "Firing"]);
    evaluate(gate.server.factory(), id, at + 180_000);
    assert_eq!(
        notification_count(gate.server.factory(), id),
        0,
        "group wait survives restart and does not send early"
    );
    concurrent(gate.server.factory(), id, at + 240_000);
    assert_eq!(
        notification_count(gate.server.factory(), id),
        1,
        "concurrent evaluators send one grouped notification"
    );
    points(gate.server.factory(), at, 8.0);
    evaluate(gate.server.factory(), id, at + 300_000);
    assert_eq!(
        states(gate.server.factory(), id),
        ["Firing", "Firing"],
        "recovery threshold retains firing between 7 and 10"
    );
    assert_eq!(
        notification_count(gate.server.factory(), id),
        1,
        "repeat interval is persisted"
    );
    concurrent(gate.server.factory(), id, at + 420_000);
    assert_eq!(notification_count(gate.server.factory(), id), 2);
    points(gate.server.factory(), at, 6.0);
    evaluate(gate.server.factory(), id, at + 480_000);
    assert_eq!(states(gate.server.factory(), id), ["Recovering", "Recovering"]);
    evaluate(gate.server.factory(), id, at + 540_000);
    assert_eq!(states(gate.server.factory(), id), ["Recovering", "Recovering"]);
    concurrent(gate.server.factory(), id, at + 600_000);
    assert_eq!(states(gate.server.factory(), id), ["Normal", "Normal"]);
    assert_eq!(
        notification_count(gate.server.factory(), id),
        3,
        "resolution sends once without waiting for the firing repeat interval"
    );
    let history = search::installed(gate.server.factory())
        .unwrap()
        .search(
            &search::Target::Index("monitor_annotations".into()),
            &search::SearchRequest::new(search::Query::eq("rule_id", Value::string(id))),
        )
        .unwrap();
    assert_eq!(
        history.total, 8,
        "one transition annotation per instance, never duplicated by workers"
    );

    points(gate.server.factory(), at, 12.0);
    for (rule_id, policy) in [
        (
            "silenced-alert",
            json!({"silences":[{"from":(at-1000).to_string(),"thru":(at+86_400_000).to_string(),"matchers":{"team":"operations"}}]}),
        ),
        (
            "muted-alert",
            json!({"muteTimings":[{"weekdays":[0,1,2,3,4,5,6],"start":"00:00","end":"24:00"}]}),
        ),
        ("unmatched-alert", json!({"contactMatchers":{"team":"other-team"}})),
    ] {
        let mut muted = body.clone();
        muted["pendingSeconds"] = json!(0);
        muted["contactPoints"][0]["groupWaitSeconds"] = json!(0);
        for (key, value) in policy.as_object().unwrap() {
            if key == "contactMatchers" {
                muted["contactPoints"][0]["matchers"] = value.clone();
            } else {
                muted[key] = value.clone();
            }
        }
        save(gate.server.factory(), rule_id, &muted);
        evaluate(gate.server.factory(), rule_id, at);
        assert_eq!(states(gate.server.factory(), rule_id), ["Firing", "Firing"]);
        assert_eq!(
            notification_count(gate.server.factory(), rule_id),
            0,
            "{rule_id} must retain state while suppressing delivery"
        );
        assert!(rows(gate.server.factory(), "moqui.report.AlertNotification", Some(rule_id)).is_empty());
    }
    for (rule_id, query, state) in [
        ("error-alert", json!({"queryString":"("}), "Error"),
        ("incomplete-alert", json!({"maxBuckets":1}), "Error"),
        (
            "missing-alert",
            json!({"queryString":"name:no-matching-metric"}),
            "NoData",
        ),
    ] {
        let mut bad = body.clone();
        bad["contactPoints"] = json!([]);
        for (key, value) in query.as_object().unwrap() {
            bad["query"][key] = value.clone();
        }
        if rule_id == "incomplete-alert" {
            with_owner(gate.server.factory(), |ec| {
                let result = report::execute_query(ec, &bad["query"]).unwrap();
                assert!(
                    result["notices"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|notice| notice["complete"] == false),
                    "backend cap must produce an explicit incomplete result"
                );
            });
        }
        save(gate.server.factory(), rule_id, &bad);
        evaluate(gate.server.factory(), rule_id, at);
        assert_eq!(states(gate.server.factory(), rule_id), [state]);
    }
}
