//! Flat telemetry mappings shared by the local SDK exporters and the OTLP receiver.

use super::mapping::{DocumentMapping, FieldKind, FieldMapping, ObjectMapping};

pub const SPANS: &str = "otel_spans";
pub const LOGS: &str = "otel_logs";
pub const METRICS: &str = "otel_metrics";
pub const ERRORS: &str = "otel_errors";
pub const ANNOTATIONS: &str = "monitor_annotations";
pub const TELEMETRY_INDEXES: &[&str] = &[SPANS, LOGS, METRICS, ERRORS, ANNOTATIONS];

pub fn is_telemetry_index(name: &str) -> bool {
    TELEMETRY_INDEXES.contains(&name)
}

pub fn telemetry_mapping(name: &str) -> Option<DocumentMapping> {
    use FieldKind::{Boolean, Date, Double, Keyword, Long, Text};
    let fields: &[(&str, FieldKind)] = match name {
        SPANS => &[
            ("trace_id", Keyword),
            ("span_id", Keyword),
            ("parent_span_id", Keyword),
            ("name", Keyword),
            ("kind", Keyword),
            ("start", Date),
            ("start_nano", Long),
            ("duration_nano", Long),
            ("status_code", Keyword),
            ("sampled", Boolean),
            ("moqui.artifact.type", Keyword),
            ("moqui.artifact.name", Keyword),
            ("moqui.artifact.action", Keyword),
            ("moqui.artifact.sub_type", Keyword),
            ("moqui.artifact.slow", Boolean),
            ("user.id", Keyword),
            ("moqui.visit.id", Keyword),
            ("http.request.method", Keyword),
            ("http.route", Keyword),
            ("http.response.status_code", Long),
            ("url.path", Keyword),
            ("url.query", Text),
            ("url.scheme", Keyword),
            ("client.address", Keyword),
            ("server.address", Keyword),
            ("session.id", Keyword),
            ("moqui.visitor.id", Keyword),
            ("network.protocol.version", Keyword),
            ("http.response.body.size", Long),
            ("moqui.http.time_to_first_byte", Double),
            ("http.request.header.referer", Text),
            ("user_agent.original", Text),
            ("http.response.header.content-type", Keyword),
            ("db.system.name", Keyword),
            ("db.query.text", Text),
            ("moqui.db.system.uncertain", Boolean),
            ("url.full", Text),
            ("moqui.job.root", Boolean),
            ("moqui.job.name", Keyword),
            ("moqui.job.run.id", Keyword),
            ("gen_ai.operation.name", Keyword),
            ("gen_ai.provider.name", Keyword),
            ("gen_ai.request.model", Keyword),
            ("gen_ai.response.model", Keyword),
            ("gen_ai.usage.input_tokens", Long),
            ("gen_ai.usage.output_tokens", Long),
            ("gen_ai.response.finish_reasons", Keyword),
            ("exception.type", Keyword),
            ("exception.message", Text),
            ("exception.stacktrace", Text),
        ],
        LOGS => &[
            ("timestamp", Date),
            ("severity_number", Long),
            ("severity_text", Keyword),
            ("body", Text),
            ("trace_id", Keyword),
            ("span_id", Keyword),
            ("scope_name", Keyword),
            ("thread.name", Keyword),
            ("thread.id", Long),
            ("user.id", Keyword),
            ("moqui.visitor.id", Keyword),
            ("exception.type", Keyword),
            ("exception.message", Text),
            ("exception.stacktrace", Text),
        ],
        METRICS => &[
            ("name", Keyword),
            ("series_fingerprint", Keyword),
            ("bucket_start", Date),
            ("bucket_end", Date),
            ("granularity", Keyword),
            ("aggregation", Keyword),
            ("unit", Keyword),
            ("count", Long),
            ("sum", Double),
            ("min", Double),
            ("max", Double),
            ("last", Double),
            ("last_time", Date),
            ("http.route", Keyword),
            ("http.request.method", Keyword),
            ("http.response.status_code", Long),
            ("db.system.name", Keyword),
            ("outcome", Keyword),
            ("moqui.artifact.type", Keyword),
            ("moqui.artifact.name", Keyword),
            ("moqui.artifact.action", Keyword),
            ("moqui.artifact.sub_type", Keyword),
        ],
        ERRORS => &[
            ("fingerprint", Keyword),
            ("first_seen", Date),
            ("last_seen", Date),
            ("count", Long),
            ("exception.type", Keyword),
            ("message", Text),
            ("stacktrace", Text),
        ],
        ANNOTATIONS => &[
            ("time", Date),
            ("time_end", Date),
            ("source", Keyword),
            ("tags", Keyword),
            ("text", Text),
            ("scope", Keyword),
            ("dashboard", Keyword),
            ("panel", Keyword),
            ("created_by", Keyword),
            ("rule_id", Keyword),
            ("series_id", Keyword),
            ("state", Keyword),
        ],
        _ => return None,
    };
    let mut root = ObjectMapping::default();
    for (field, kind) in fields.iter().copied().chain([
        ("service.name", Keyword),
        ("service.instance.id", Keyword),
        ("service.version", Keyword),
        ("vcs.ref.head.revision", Keyword),
        ("host.name", Keyword),
        ("source.name", Keyword),
        ("source.id", Keyword),
        ("moqui.identity.uncertain", Boolean),
    ]) {
        if name == ANNOTATIONS && matches!(field, "source.name" | "source.id") {
            continue;
        }
        root.fields
            .insert(field.to_string(), FieldMapping { kind, sortable: true });
    }
    Some(DocumentMapping {
        document_id: name.to_string(),
        index_name: None,
        physical_name: name.to_string(),
        root,
        manual_mapping_service: None,
        display: Default::default(),
    })
}
