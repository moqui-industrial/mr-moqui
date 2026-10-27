//! Optional OTLP/HTTP JSON export over the framework's existing reqwest 0.13 transport.
//! No collector, async runtime, second HTTP stack, or tracing bridge is required.

use std::sync::Arc;
use std::time::Duration;

use moqui_lib::error::Error;
use moqui_lib::object::Value;
use serde_json::{Value as Json, json};

use super::Exporter;
use crate::entity::document::Document;

pub struct OtlpExporter {
    endpoint: String,
    headers: Vec<(String, String)>,
    client: moqui_lib::http::RestClient,
}

impl OtlpExporter {
    pub fn new(endpoint: &str, headers: Vec<(String, String)>) -> Result<Self, Error> {
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(Error::new("OTLP endpoint must use http or https"));
        }
        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            headers,
            client: moqui_lib::http::RestClient::new(),
        })
    }
}

impl Exporter for OtlpExporter {
    fn export(&self, index: &str, documents: Vec<Document>) -> Result<usize, Error> {
        let (path, body) = encode(index, &documents)?;
        let mut call = self
            .client
            .post(format!("{}{path}", self.endpoint))
            .timeout(Duration::from_secs(5));
        for (name, value) in &self.headers {
            call = call.header(name.clone(), value.clone());
        }
        let response = call.text("application/json", body.to_string()).send(false)?;
        if !(200..300).contains(&response.status()) {
            return Err(Error::new(format!("OTLP export returned HTTP {}", response.status())));
        }
        let rejected = serde_json::from_str::<Json>(response.body())
            .ok()
            .and_then(|body| {
                body.get("partialSuccess").and_then(|partial| {
                    ["rejectedSpans", "rejectedLogRecords", "rejectedDataPoints"]
                        .iter()
                        .find_map(|key| {
                            partial.get(*key).and_then(|value| {
                                value
                                    .as_u64()
                                    .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                            })
                        })
                })
            })
            .unwrap_or(0);
        Ok(documents.len().saturating_sub(rejected as usize))
    }
}

/// Local persistence and optional external export fail independently. Local success is the
/// returned count; an unavailable collector never prevents local investigation.
pub struct TeeExporter {
    pub local: Arc<dyn Exporter>,
    pub remote: Arc<dyn Exporter>,
    failures: std::sync::atomic::AtomicU64,
    telemetry: std::sync::Weak<super::Telemetry>,
}
impl Exporter for TeeExporter {
    fn export(&self, index: &str, documents: Vec<Document>) -> Result<usize, Error> {
        let local = self.local.export(index, documents.clone());
        if matches!(index, "otel_spans" | "otel_logs" | "otel_metrics") {
            let count = documents.len() as u64;
            match self.remote.export(index, documents) {
                Ok(exported) => {
                    let dropped = count.saturating_sub(exported as u64);
                    if let Some(telemetry) = self.telemetry.upgrade() {
                        telemetry.note_remote_export(exported as u64, dropped, false);
                    }
                    if dropped > 0 {
                        let failures = self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if failures == 0 || failures.is_power_of_two() {
                            tracing::warn!(
                                index,
                                rejected = dropped,
                                "external collector partially rejected telemetry"
                            );
                        }
                    }
                }
                Err(error) => {
                    if let Some(telemetry) = self.telemetry.upgrade() {
                        telemetry.note_remote_export(0, count, true);
                    }
                    let failures = self.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if failures == 0 || failures.is_power_of_two() {
                        tracing::warn!(
                            index,
                            failures = failures + 1,
                            "external telemetry export failed: {error}"
                        );
                    }
                }
            }
        }
        local
    }
    fn export_received(&self, index: &str, documents: Vec<Document>) -> Result<usize, Error> {
        self.local.export_received(index, documents)
    }
    fn upsert_error(&self, document: Document) -> Result<(), Error> {
        self.local.upsert_error(document)
    }
}

/// Wrap a local exporter when a configured endpoint exists. Header values are never logged.
pub fn configured(factory: &crate::context::ExecutionContextFactory, local: Arc<dyn Exporter>) -> Arc<dyn Exporter> {
    let endpoint = factory.conf_setting("telemetry", "otlp-endpoint");
    if endpoint.is_empty() {
        return local;
    }
    let headers = factory
        .conf_setting("telemetry", "otlp-headers")
        .split(',')
        .filter_map(|item| {
            item.split_once('=')
                .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    match OtlpExporter::new(&endpoint, headers) {
        Ok(remote) => Arc::new(TeeExporter {
            local,
            remote: Arc::new(remote),
            failures: std::sync::atomic::AtomicU64::new(0),
            telemetry: std::sync::Arc::downgrade(&super::for_factory(factory)),
        }),
        Err(error) => {
            tracing::warn!("external telemetry exporter is disabled: {error}");
            local
        }
    }
}

fn encode(index: &str, documents: &[Document]) -> Result<(&'static str, Json), Error> {
    let mut resources = Vec::new();
    for document in documents {
        let resource = json!({"attributes": resource_attributes(document)});
        let scope = json!({"name":"moqui"});
        match index {
            "otel_spans" => {
                let start = integer(document, "start_nano");
                let mut span = json!({"traceId":super::text(document,"trace_id"),"spanId":super::text(document,"span_id"),
                    "name":super::text(document,"name"),"kind":match super::text(document,"kind").as_str(){"SERVER"=>2,"CLIENT"=>3,"PRODUCER"=>4,"CONSUMER"=>5,_=>1},
                    "startTimeUnixNano":start.to_string(),"endTimeUnixNano":start.saturating_add(integer(document,"duration_nano")).to_string(),
                    "attributes": attributes(document),"status":{"code":if super::text(document,"status_code")=="ERROR"{2}else{1}}});
                let parent = super::text(document, "parent_span_id");
                if !parent.is_empty() {
                    span["parentSpanId"] = json!(parent);
                }
                if let Some(Value::List(events)) = document.get("events") {
                    span["events"] = Json::Array(events.iter().filter_map(|event| if let Value::Map(event) = event {
                        Some(json!({"name":super::text(event,"name"),"timeUnixNano":integer(event,"timestamp").saturating_mul(1_000_000).to_string(),
                            "attributes":event.iter().filter(|(key,_)|key.as_str()!="name"&&key.as_str()!="timestamp").map(|(key,value)|kv(key,value)).collect::<Vec<_>>()}))
                    } else { None }).collect());
                }
                resources.push(json!({"resource":resource,"scopeSpans":[{"scope":scope,"spans":[span]}]}));
            }
            "otel_logs" => {
                let mut log = json!({"timeUnixNano":integer(document,"timestamp").saturating_mul(1_000_000).to_string(),
                    "severityNumber":integer(document,"severity_number"),"severityText":super::text(document,"severity_text"),
                    "body":any(document.get("body").unwrap_or(&Value::Null)),"attributes":attributes(document)});
                for (from, to) in [("trace_id", "traceId"), ("span_id", "spanId")] {
                    let value = super::text(document, from);
                    if !value.is_empty() {
                        log[to] = json!(value);
                    }
                }
                resources.push(json!({"resource":resource,"scopeLogs":[{"scope":scope,"logRecords":[log]}]}));
            }
            "otel_metrics" => {
                if super::text(document, "granularity") != "minute" {
                    continue;
                }
                let mut point = json!({"startTimeUnixNano":integer(document,"bucket_start").saturating_mul(1_000_000).to_string(),
                    "timeUnixNano":integer(document,"bucket_end").saturating_mul(1_000_000).to_string(),"attributes":attributes(document)});
                let mut metric = json!({"name":super::text(document,"name"),"unit":super::text(document,"unit")});
                if let (Some(Value::List(bounds)), Some(Value::List(buckets))) =
                    (document.get("bounds"), document.get("buckets"))
                {
                    if !buckets.is_empty() {
                        point["count"] = json!(integer(document, "count").to_string());
                        if document.contains_key("sum") {
                            point["sum"] = json!(super::number(document, "sum"));
                        }
                        point["explicitBounds"] =
                            json!(bounds.iter().map(moqui_lib::json::to_json).collect::<Vec<_>>());
                        point["bucketCounts"] = json!(
                            buckets
                                .iter()
                                .map(moqui_lib::object::to_plain_string)
                                .collect::<Vec<_>>()
                        );
                        for key in ["min", "max"] {
                            if document.contains_key(key) {
                                point[key] = json!(super::number(document, key));
                            }
                        }
                        metric["histogram"] = json!({"aggregationTemporality":1,"dataPoints":[point]});
                    } else {
                        point["asDouble"] = json!(super::number(document, "sum"));
                        metric["sum"] = json!({"aggregationTemporality":1,"isMonotonic":true,"dataPoints":[point]});
                    }
                } else {
                    point["asDouble"] = json!(super::number(document, "sum"));
                    metric["sum"] = json!({"aggregationTemporality":1,"isMonotonic":true,"dataPoints":[point]});
                }
                if super::text(document, "aggregation") == "gauge" {
                    let points = metric["sum"]["dataPoints"].clone();
                    if let Some(values) = metric.as_object_mut() {
                        values.remove("sum");
                    }
                    metric["gauge"] = json!({"dataPoints":points});
                }
                if let Some(raw) = document.get("exponential_histogram") {
                    let mut point = moqui_lib::json::to_json(raw);
                    point["startTimeUnixNano"] =
                        json!(integer(document, "bucket_start").saturating_mul(1_000_000).to_string());
                    point["timeUnixNano"] =
                        json!(integer(document, "bucket_end").saturating_mul(1_000_000).to_string());
                    point["attributes"] = json!(attributes(document));
                    if let Some(values) = metric.as_object_mut() {
                        values.remove("sum");
                        values.remove("histogram");
                    }
                    let temporality = super::text(document, "aggregation_temporality");
                    metric["exponentialHistogram"] = json!({"aggregationTemporality":if temporality=="2"||temporality=="AGGREGATION_TEMPORALITY_CUMULATIVE"{2}else{1},"dataPoints":[point]});
                }
                resources.push(json!({"resource":resource,"scopeMetrics":[{"scope":scope,"metrics":[metric]}]}));
            }
            _ => return Err(Error::new("this index is not an OTLP signal")),
        }
    }
    Ok(match index {
        "otel_spans" => ("/v1/traces", json!({"resourceSpans":resources})),
        "otel_logs" => ("/v1/logs", json!({"resourceLogs":resources})),
        _ => ("/v1/metrics", json!({"resourceMetrics":resources})),
    })
}

fn integer(document: &Document, key: &str) -> i64 {
    match document.get(key) {
        Some(Value::I64(value)) => *value,
        Some(Value::I32(value)) => i64::from(*value),
        Some(value) => moqui_lib::object::to_plain_string(value).parse().unwrap_or(0),
        None => 0,
    }
}
fn resource_attributes(document: &Document) -> Vec<Json> {
    if let Some(Value::Map(resource)) = document.get("resource") {
        return resource.iter().map(|(key, value)| kv(key, value)).collect();
    }
    [
        "service.name",
        "service.instance.id",
        "service.version",
        "vcs.ref.head.revision",
    ]
    .iter()
    .filter_map(|key| document.get(*key).map(|value| kv(key, value)))
    .collect()
}
fn attributes(document: &Document) -> Vec<Json> {
    match document.get("attributes") {
        Some(Value::Map(values)) => values
            .iter()
            .filter(|(key, _)| !key.starts_with("moqui.head."))
            .map(|(key, value)| kv(key, value))
            .collect(),
        _ => Vec::new(),
    }
}
fn kv(key: &str, value: &Value) -> Json {
    json!({"key":key,"value":any(value)})
}
fn any(value: &Value) -> Json {
    match value {
        Value::Bool(value) => json!({"boolValue":value}),
        Value::I32(value) => json!({"intValue":value.to_string()}),
        Value::I64(value) => json!({"intValue":value.to_string()}),
        Value::F64(value) => json!({"doubleValue":value}),
        Value::List(values) => json!({"arrayValue":{"values":values.iter().map(any).collect::<Vec<_>>()}}),
        Value::Map(values) => {
            json!({"kvlistValue":{"values":values.iter().map(|(key,value)|kv(key,value)).collect::<Vec<_>>()}})
        }
        _ => json!({"stringValue":moqui_lib::object::to_plain_string(value)}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_json_round_trips_through_the_receiver() {
        let mut document = Document::new();
        for (key, value) in [
            ("trace_id", Value::string("0123456789abcdef0123456789abcdef")),
            ("span_id", Value::string("0123456789abcdef")),
            ("name", Value::string("test")),
            ("kind", Value::string("SERVER")),
            ("start_nano", Value::I64(1_000_000_000)),
            ("duration_nano", Value::I64(100_000_000)),
            ("service.name", Value::string("demo")),
        ] {
            document.insert(key.into(), value);
        }
        let (path, json) = encode("otel_spans", &[document]).unwrap();
        assert_eq!(path, "/v1/traces");
        let decoded = crate::telemetry::ingest::Decoder::default()
            .decode(
                crate::telemetry::ingest::Signal::Traces,
                "application/json",
                &serde_json::to_vec(&json).unwrap(),
                "source",
            )
            .unwrap();
        assert_eq!(decoded.documents.len(), 1);
        assert_eq!(super::super::text(&decoded.documents[0], "service.name"), "demo");
    }

    #[test]
    fn received_signals_never_forward_and_partial_external_loss_is_separate() {
        struct Count {
            calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
            partial: bool,
        }
        impl Exporter for Count {
            fn export(&self, _: &str, documents: Vec<Document>) -> Result<usize, Error> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(if self.partial {
                    documents.len() / 2
                } else {
                    documents.len()
                })
            }
        }
        let telemetry = std::sync::Arc::new(super::super::Telemetry::from_conf(
            "<moqui-conf><telemetry enabled=\"false\"/></moqui-conf>",
            super::super::Resource::default(),
        ));
        let local_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let remote_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let exporter = TeeExporter {
            local: std::sync::Arc::new(Count {
                calls: local_calls.clone(),
                partial: false,
            }),
            remote: std::sync::Arc::new(Count {
                calls: remote_calls.clone(),
                partial: true,
            }),
            failures: std::sync::atomic::AtomicU64::new(0),
            telemetry: std::sync::Arc::downgrade(&telemetry),
        };
        let document = Document::from_iter([("source".into(), Value::string("local"))]);
        assert_eq!(
            exporter.export_received("otel_logs", vec![document.clone()]).unwrap(),
            1
        );
        assert_eq!(remote_calls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(exporter.export("otel_logs", vec![document; 4]).unwrap(), 4);
        assert_eq!(local_calls.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(remote_calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let counters = telemetry.counters();
        assert_eq!(counters.remote_exported, 2);
        assert_eq!(counters.remote_dropped, 2);
        assert_eq!(counters.dropped, 0);
        assert_eq!(counters.remote_failed, 0);
    }
}
