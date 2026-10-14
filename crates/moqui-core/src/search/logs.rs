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

//! The log indexes, `moqui_logs` and `moqui_http_log`. The host writes them; they are not
//! DataDocuments. They exist only on a backend that keeps them (OpenSearch), where the facade
//! treats each as a flat document type whose id is the index name, so search,
//! `delete_matching` retention, and `get_document` work as they do for documents.
//!
//! The fields follow `ElasticSearchLogger` and `ElasticRequestLogFilter`. `@timestamp` is the
//! event time in epoch millis, a field of the body. `mdc` and `thrown` are left to dynamic mapping.

use super::mapping::{DocumentMapping, FieldKind, FieldMapping, ObjectMapping};

/// Application log events.
pub const APP_LOG: &str = "moqui_logs";
/// HTTP request log entries.
pub const HTTP_LOG: &str = "moqui_http_log";

/// True for `moqui_logs` and `moqui_http_log`.
pub fn is_log_index(name: &str) -> bool {
    name == APP_LOG || name == HTTP_LOG
}

/// The flat mapping of a log index, or `None` for any other name.
pub fn log_mapping(name: &str) -> Option<DocumentMapping> {
    use FieldKind::{Date, Double, Keyword, Long, Text};
    let fields: &[(&str, FieldKind)] = match name {
        APP_LOG => &[
            ("@timestamp", Date),
            ("level", Keyword),
            ("thread_name", Keyword),
            ("thread_id", Long),
            ("thread_priority", Long),
            ("user_id", Keyword),
            ("visitor_id", Keyword),
            ("logger_name", Text),
            ("name", Text),
            ("message", Text),
            ("source_host", Keyword),
        ],
        HTTP_LOG => &[
            ("@timestamp", Date),
            ("remote_ip", Keyword),
            ("remote_user", Keyword),
            ("server_ip", Keyword),
            ("content_type", Text),
            ("request_method", Keyword),
            ("request_scheme", Keyword),
            ("request_host", Keyword),
            ("request_path", Text),
            ("request_query", Text),
            ("http_version", Double),
            ("response", Long),
            ("time_initial_ms", Long),
            ("time_final_ms", Long),
            ("bytes", Long),
            ("referrer", Text),
            ("agent", Text),
            ("session", Keyword),
            ("visitor_id", Keyword),
        ],
        _ => return None,
    };
    let mut root = ObjectMapping::default();
    for (field, kind) in fields {
        root.fields.insert(
            (*field).to_string(),
            FieldMapping {
                kind: *kind,
                sortable: false,
            },
        );
    }
    Some(DocumentMapping {
        document_id: name.to_string(),
        // Found by name (`SearchFacade::target_documents`), so no alias: it would be the index's own name.
        index_name: None,
        physical_name: name.to_string(),
        root,
        manual_mapping_service: None,
        display: Default::default(),
    })
}
