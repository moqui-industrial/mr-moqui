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

//! The data feed's receiving side: `DataFeedPending` rows become calls to each feed's services.
//!
//! A commit inserts one pending row per touched document (the primary record id, and `index` or
//! `delete`). After commit a service worker runs [`process_pending`] for those rows: it assembles
//! the documents, calls the receive service of every real-time feed that has the document with
//! `documentList`, calls the delete service for a document that no longer assembles, and deletes
//! the rows that were delivered. A row that cannot be delivered stays, and a later call with no
//! ids (the `IndexDataFeedDocuments` drain) tries it again.
//!
//! A feed's receive service defaults to `index#DataDocuments` and its delete service to
//! `delete#DataDocument`. Other feeds, such as notifications, receive through their own service.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use jiff::Timestamp;
use moqui_lib::error::Error;
use moqui_lib::object::{to_plain_string, Value};

use crate::context::ExecutionContext;
use crate::entity::document::{self, Document};
use crate::entity::{Condition, EntityFacade};
use crate::transaction::TransactionAttribute;

pub const DEFAULT_RECEIVE: &str = "org.moqui.search.SearchServices.index#DataDocuments";
pub const DEFAULT_DELETE: &str = "org.moqui.search.SearchServices.delete#DataDocument";
const PENDING: &str = "moqui.entity.feed.DataFeedPending";
const REALTIME_PUSH: &str = "DTFDTP_RT_PUSH";
/// Pending rows one drain call reads.
const DRAIN_BATCH: i64 = 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeedReport {
    pub documents_indexed: usize,
    pub documents_deleted: usize,
    pub rows_done: usize,
    pub rows_left: usize,
}

struct Feed {
    data_feed_id: String,
    receive: String,
    delete: String,
}

/// Deliver pending rows: the given ids, or a batch of every row when `ids` is `None`.
///
/// # Errors
///
/// Returns an error when no context is entered or the pending rows cannot be read. A failed
/// delivery is logged and its rows stay.
pub fn process_pending(ids: Option<&[String]>) -> Result<FeedReport, Error> {
    let facade = ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("data feed processing needs an entity facade"))?;
    let _authz = ExecutionContext::with_current(|ec| ec.disable_authz());
    let mut find = facade.find(PENDING)?;
    match ids {
        Some([]) => return Ok(FeedReport::default()),
        Some(ids) => {
            find = find.condition(Condition::is_in(
                "dataFeedPendingId",
                ids.iter().map(Value::string).collect(),
            ));
        }
        None => find = find.limit(DRAIN_BATCH),
    }
    let rows = find.list()?;
    // Document id, then record id to the pending row ids for it.
    let mut by_document: IndexMap<String, IndexMap<String, Vec<String>>> = IndexMap::new();
    for row in rows.iter() {
        let text = |name: &str| row.get(name).map(to_plain_string).unwrap_or_default();
        let (pending_id, document_id, record_id) =
            (text("dataFeedPendingId"), text("dataDocumentId"), text("primaryKey"));
        if document_id.is_empty() || record_id.is_empty() {
            continue;
        }
        by_document
            .entry(document_id)
            .or_default()
            .entry(record_id)
            .or_default()
            .push(pending_id);
    }
    let feeds = realtime_feeds(&facade)?;
    let search_up = ExecutionContext::with_current(|ec| super::installed(ec.factory()).is_some()).unwrap_or(false);
    let mut report = FeedReport::default();
    for (document_id, records) in by_document {
        let row_count: usize = records.values().map(Vec::len).sum();
        let document_feeds: Vec<&Feed> = feeds
            .get(&document_id)
            .map(|list| list.iter().collect())
            .unwrap_or_default();
        let deliverable = document_feeds
            .iter()
            .all(|feed| search_up || (feed.receive != DEFAULT_RECEIVE && feed.delete != DEFAULT_DELETE));
        if !deliverable {
            tracing::debug!(document = %document_id, "search is not configured; data feed rows stay");
            report.rows_left += row_count;
            continue;
        }
        let record_ids: Vec<String> = records.keys().cloned().collect();
        // Each document type in its own transaction: a type that fails (a manual data service that errors, say) rolls
        // back only its own delivery and keeps only its own rows, not the other types' in this batch.
        let delivered = facade.transactions_for(PENDING).and_then(|tx| {
            tx.run(TransactionAttribute::ForceNew, None, || {
                let (indexed, deleted) = deliver(&facade, &document_id, &record_ids, &document_feeds)?;
                for pending_ids in records.values() {
                    for pending_id in pending_ids {
                        delete_pending(&facade, pending_id)?;
                    }
                }
                Ok((indexed, deleted))
            })
        });
        match delivered {
            Ok((indexed, deleted)) => {
                report.documents_indexed += indexed;
                report.documents_deleted += deleted;
                report.rows_done += row_count;
            }
            Err(err) => {
                tracing::warn!(document = %document_id, error = %err, "data feed delivery failed; pending rows stay");
                // The failure is logged; its message errors would stop the next document type's services.
                ExecutionContext::with_current(|ec| ec.message().clear_errors());
                report.rows_left += row_count;
            }
        }
    }
    Ok(report)
}

fn deliver(
    facade: &EntityFacade,
    document_id: &str,
    record_ids: &[String],
    feeds: &[&Feed],
) -> Result<(usize, usize), Error> {
    let documents = document::documents_by_ids(facade, document_id, record_ids)?;
    let found: std::collections::HashSet<String> = documents
        .iter()
        .filter_map(|doc| doc.get("_id").map(to_plain_string))
        .collect();
    let gone: Vec<&String> = record_ids.iter().filter(|id| !found.contains(*id)).collect();
    let stamp = Value::Timestamp(Timestamp::now());
    for feed in feeds {
        if !documents.is_empty() {
            let mut params = BTreeMap::new();
            params.insert("dataFeedId".to_string(), Value::string(&feed.data_feed_id));
            params.insert("feedStamp".to_string(), stamp.clone());
            params.insert(
                "documentList".to_string(),
                Value::List(documents.iter().cloned().map(Value::Map).collect()),
            );
            call_checked(&feed.receive, &params)?;
        }
        for id in &gone {
            let mut params = BTreeMap::new();
            params.insert("dataFeedId".to_string(), Value::string(&feed.data_feed_id));
            params.insert("feedStamp".to_string(), stamp.clone());
            params.insert("dataDocumentId".to_string(), Value::string(document_id));
            params.insert("documentId".to_string(), Value::string(*id));
            call_checked(&feed.delete, &params)?;
        }
    }
    Ok((documents.len(), gone.len()))
}

/// Call a service; an error message it leaves is a failure, and is cleared so it does not fail
/// the caller.
fn call_checked(name: &str, params: &BTreeMap<String, Value>) -> Result<(), Error> {
    let before = ExecutionContext::with_current(|ec| ec.message().errors().len()).unwrap_or(0);
    let result = crate::service::call::sync(name, params);
    let errors: Vec<String> = ExecutionContext::with_current(|ec| {
        let mut messages = ec.message();
        let errors = messages.errors()[before.min(messages.errors().len())..].to_vec();
        if !errors.is_empty() {
            messages.clear();
        }
        errors
    })
    .unwrap_or_default();
    match result {
        Err(err) => Err(Error::new(format!("{name}: {}", err.message()))),
        Ok(_) if !errors.is_empty() => Err(Error::new(format!("{name}: {}", errors.join("; ")))),
        Ok(_) => Ok(()),
    }
}

fn delete_pending(facade: &EntityFacade, pending_id: &str) -> Result<(), Error> {
    let mut value = facade.make_value(PENDING)?;
    value.set("dataFeedPendingId", Value::string(pending_id));
    value.delete(facade)
}

/// Real-time push feeds, by the document ids they contain.
fn realtime_feeds(facade: &EntityFacade) -> Result<IndexMap<String, Vec<Feed>>, Error> {
    let feeds = facade
        .find("moqui.entity.feed.DataFeed")?
        .eq("dataFeedTypeEnumId", Value::string(REALTIME_PUSH))
        .list()?;
    let links = facade.find("moqui.entity.feed.DataFeedDocument")?.list()?;
    let mut out: IndexMap<String, Vec<Feed>> = IndexMap::new();
    for feed in feeds.iter() {
        let text = |name: &str| feed.get(name).map(to_plain_string).filter(|text| !text.is_empty());
        let Some(data_feed_id) = text("dataFeedId") else {
            continue;
        };
        for link in links.iter() {
            if link.get("dataFeedId").map(to_plain_string).as_deref() != Some(data_feed_id.as_str()) {
                continue;
            }
            let Some(document_id) = link.get("dataDocumentId").map(to_plain_string) else {
                continue;
            };
            out.entry(document_id).or_default().push(Feed {
                data_feed_id: data_feed_id.clone(),
                receive: text("feedReceiveServiceName").unwrap_or_else(|| DEFAULT_RECEIVE.to_string()),
                delete: text("feedDeleteServiceName").unwrap_or_else(|| DEFAULT_DELETE.to_string()),
            });
        }
    }
    Ok(out)
}

/// `index#DataFeedDocuments`: rebuild every document of a feed, or one document of it, in
/// batches sent to the feed's receive service. Returns how many documents were sent.
///
/// # Errors
///
/// Returns an error when the feed has no documents, a definition is bad, or a receive call fails.
pub fn rebuild_feed(
    data_feed_id: &str,
    data_document_id: Option<&str>,
    from: Option<Timestamp>,
    thru: Option<Timestamp>,
    batch_size: usize,
) -> Result<usize, Error> {
    let facade = ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("data feed rebuild needs an entity facade"))?;
    let _authz = ExecutionContext::with_current(|ec| ec.disable_authz());
    let feed = facade
        .find("moqui.entity.feed.DataFeed")?
        .eq("dataFeedId", Value::string(data_feed_id))
        .one()?
        .ok_or_else(|| Error::new(format!("No DataFeed found for ID {data_feed_id}")))?;
    let receive = feed
        .get("feedReceiveServiceName")
        .map(to_plain_string)
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| DEFAULT_RECEIVE.to_string());
    let mut find = facade
        .find("moqui.entity.feed.DataFeedDocument")?
        .eq("dataFeedId", Value::string(data_feed_id));
    if let Some(id) = data_document_id {
        find = find.eq("dataDocumentId", Value::string(id));
    }
    let links = find.list()?;
    if links.is_empty() {
        return Err(Error::new(format!(
            "No DataDocuments found for DataFeed ID {data_feed_id}{}",
            data_document_id
                .map(|id| format!(" and DataDocument ID {id}"))
                .unwrap_or_default()
        )));
    }
    let mut total = 0;
    for link in links.iter() {
        let Some(document_id) = link.get("dataDocumentId").map(to_plain_string) else {
            continue;
        };
        if receive == DEFAULT_RECEIVE {
            if let Ok(search) = super::current() {
                search.ensure_index(&document_id)?;
            }
        }
        total += document::feed_documents(&facade, &document_id, from, thru, batch_size, |batch: Vec<Document>| {
            let mut params = BTreeMap::new();
            params.insert("dataFeedId".to_string(), Value::string(data_feed_id));
            params.insert("feedStamp".to_string(), Value::Timestamp(Timestamp::now()));
            params.insert(
                "documentList".to_string(),
                Value::List(batch.into_iter().map(Value::Map).collect()),
            );
            call_checked(&receive, &params)?;
            Ok(true)
        })?;
    }
    Ok(total)
}
