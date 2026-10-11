# SearchFacade

Replacement for `ElasticFacade`. The API is a document search and aggregation tree. Postgres implements the whole thing and is the default. OpenSearch is an optional backend for the same API, and the only backend that keeps OpenSearch Dashboards.

This follows `plans/Architecture-Proposal.md` (Search) and `plans/Crate-Layout.md` (`search` module, `SearchBackend`). The inventory below is from the Moqui tree at `../moqui`: framework, runtime, mantle-udm, mantle-usl, SimpleScreens, and the other components that call the same services.

## Decision

`SearchFacade` is not a port of `ElasticFacade`. `ElasticFacade` takes ElasticSearch JSON maps and posts them. Mapping that JSON onto Postgres would freeze the Elastic query language into the framework. The facade sits one layer above that. Callers pass a query tree and an aggregation tree. Each backend compiles those trees.

Postgres is the default backend. When the entity database is the embedded PGLite cluster, search documents live in the `search` schema on that same cluster. Config may name an external Postgres instead, the same instance as the entities or another one. When the entity datasource is not Postgres (MySQL, for example), embedded search is its own PGLite cluster. There is no MySQL search backend. The store is independent of a non-Postgres entity datasource.

OpenSearch is one optional HTTP backend. It speaks the shared `_search` and `_bulk` subset that current OpenSearch and ElasticSearch 7+ have in common. The runtime already ships OpenSearch and OpenSearch Dashboards 3.4. One client covers both products for this subset. The ElasticSearch 6 and pre-7 branches in `ElasticFacadeImpl` (`interval` versus `calendar_interval`, old point-in-time paths, document types) stop here.

One active backend. Applications and the built-in reports always call `SearchFacade`. When the backend is OpenSearch, the indexes stay available to OpenSearch Dashboards through the existing `/elastic` and `/kibana` proxies. When the backend is Postgres, those dashboards are absent. The facade does not write to two stores. External reporting is a property of the OpenSearch backend. In-app reports use the aggregation API on whichever backend is configured.

`moqui-core` owns the document model, the query and aggregation trees, the Postgres SQL, and the OpenSearch request bodies. Bodies go out through `HttpClient`. `SearchBackend` is the trait. Starting an OpenSearch process stays in `moqui-host`.

Column full-text (`text-fts` on an entity field, `../moqui/framework/plans/DbFullTextSearch.md`) stays an EntityFacade feature. Skills, wiki bodies, and similar one-table text are that feature. `SearchFacade` is denormalized documents, nested child constraints, and aggregations.

## What ElasticFacade is used for

`org.moqui.context.ElasticFacade` is a named-cluster HTTP client. `search`, `count`, `deleteByQuery`, and `validateQuery` accept a `Map` that is sent as JSON. `org.moqui.search.SearchServices.search#DataDocuments` builds that JSON for find screens. `mantle.sales.SalesReportServices` skips the service and calls `elasticClient.search` with aggregation JSON.

### Documents and feeds

The document model is already independent of the search engine. It stays.

Definitions are in `framework/entity/EntityEntities.xml`. `EntityDataDocument` builds the maps. `EntityDataFeed` pushes them.

- `DataDocument` names a primary entity, an `indexName` (a group name, today an alias), field paths, relationship aliases, conditions, links, and user groups. `manualDataServiceName` can add fields after the query. `manualMappingServiceName` has no callers and is dropped.
- A field path is `Rel:Rel:field`, or an expression with a required alias. Expressions run at index time. In MR-Moqui they are JavaScript, same as other expressions. `fieldType` overrides the dictionary type. `sortable=Y` on a text field keeps an unanalyzed copy for sort.
- A relationship of type one is flattened into the parent object. A relationship of type many becomes an array of objects. MantleParty nests roles, identifications, contact mechs, user accounts, classifications, and notes. MantleSalesOrderItem nests issuances, and also stores header expressions (`itemSaleTotal`, `customerClassId`) computed from those children.
- A `DataDocumentCondition` is either part of the entity query (`MantleSalesOrderItem` requires `vendorRoleTypeId = OrgInternal`) or a post-query check.
- Document id is the primary key values joined with `::`.
- `DataFeed` is real-time push (`DTFDTP_RT_PUSH`) on create, update, and primary-entity delete, or manual pull. The receive service defaults to `index#DataDocuments`. The delete service defaults to `delete#DataDocument`. `index#DataFeedDocuments` rebuilds a feed in batches of 1000. The `IndexDataFeedDocuments` job wraps that call.
- Seed documents in mantle-udm: party, communication event, product, facility, wiki page, project, task, event, request, four sales documents, five inventory documents. Feeds: Mantle General (`mantle`), Sales (`mantle_sales`), Inventory (`mantle_inventory`). PopcProduct is a clone of MantleProduct. HiveMind and PopCommerce add links.

The physical name of a document type is `dataDocumentId` converted from camel case to underscore (`MantleParty` to `mantle_party`). `indexName` groups those types. A search with no document type searches the group.

### Find screens

`search#DataDocuments` sends a `bool.must` containing a Lucene `query_string` (`lenient: true`, default time zone), plus an optional `nestedQueryMap`. Each entry with a path becomes a `nested` query (`score_mode: avg`). A null path is a top-level clause. Sort accepts a leading `+` or `-`. Highlight is a list of fields. Paging is `from` / `size` with `track_total_hits`. `pageNoLimit` caps at 10000. Hits can be flattened. Each hit is given `_index`, `_id`, `_type` (the document id), and `highlights` when highlight was requested.

The query strings that callers build, including `StringUtilities.elasticQueryAutoWildcard`, use this subset:

- bare terms, a trailing `*` for prefix, a leading `*:` to search every field
- `field:value`, `field:value*`, `field:*value*`
- `AND`, `OR`, `NOT`, and parentheses
- `field:(a OR b OR _NA_)`
- `_exists_:field` and `*:*`
- dotted names (`roles.roleTypeId`, `contactMechs.city`, `features.productFeatureId`)

A dotted name means any element of that child array has the value. That is what `include_in_root: true` does on the nested mapping. It does not require the constraints to match the same element.

The same-element case is `nestedQueryMap`, and only two callers use it:

- Party classification: `partyClassificationId` equals, `classFromDate` is at or before now, and `classThruDate` is at or after now or missing.
- Pop store category: the same shape on `categories` (category id, from date, thru date or missing).

Org limits (`MANTLE_USER_ORG`, `MANTLE_ACTIVE_ORG`) are appended as `ownerPartyId:(…)` by `mantle.GeneralServices.search#MantleFiltered` and by the party and product screens.

Callers:

| Caller | What it sends |
|---|---|
| `mantle.party.PartyServices.search#Party` | Auto-wildcard text, field prefixes, the classification `Child` query, highlight fields, sort on `combinedName.keyword` |
| SimpleScreens party autocomplete (`PartyForms.xml`) | Auto-wildcard text, role, party type, disabled, owner org |
| SimpleScreens catalog search | Field terms on product attributes, features, and categories, plus owner org |
| SimpleScreens task find | Field terms, then an entity find by the returned ids |
| SimpleScreens request find, `ProjectServices` search | Query string on the work documents |
| `search#MantleFiltered` | Auto-wildcard plus owner org, nested documents left intact |
| HiveMind quick search, PopCommerce product search | `search#DataDocuments` |
| PopRestStore product, content, and blog search | Query string. Product search adds the category `Child` query |
| Tools DataDocument Search | Free query string against an `indexName`. The screen links to the Elastic query-string manual |
| Tools LogViewer | Raw `query_string` against the `logger` cluster or `default` |

### Built-in reports

`mantle.sales.SalesReportServices` builds the JSON itself. SimpleScreens `Accounting/Reports/SalesSummary.xml` calls it. The MarbleERP sales skill describes the same aggregations.

The filter is a `bool.filter`: a date `range` (`gte` / `lt`, epoch millis) and `terms` on status, plus optional id lists (vendor, customer, store, classification, product).

The aggregation tree is:

- `date_histogram` on a date field. Calendar units are day, week, month, quarter, and year. Time zone, a display format, an optional offset (week start), newest bucket first. Buckets expose `key` and `key_as_string`.
- `terms` on `productId` or `customerClassId`, with a size and an order by a sub-aggregation.
- `sum` and `cardinality` (`precision_threshold` 40000) on `orderId` or `invoiceId`.
- a `filter` aggregation (`term` or `terms`) wrapping those metrics. Product lines are `itemTypeEnumId = ItemProduct`. Discounts are a fixed list of item types. New customers are `isNewCustomer = Y`.
- `sum` of a number on each element of a child array (`issuances.issuedSaleTotal`, `issuances.issuedCostTotal`). The mapping marks issuances `nested` and `include_in_root`, so the sum sees every child value. It is not a correlated nested aggregation.

The four services are `get#SalesOrderSummary`, `get#SalesInvoiceSummary`, `get#SalesTopProduct`, and `get#InvoicesByCustomerClass`.

`mantle.product.InventoryReportServices.get#InventoryProjectedInfo` does not run. The body imports `org.moqui.elasticsearch.ElasticSearchUtil`, and the block above it calls `search#CountBySource`, which `SearchServices.xml` records as removed. The inventory documents and the Mantle Inventory feed are real. The service is a rewrite onto `aggregate`: `terms` on `productId`, `sum` of the quantity fields, an optional `term` on `facilityId`, an optional range on a date. `ElasticSearchUtil` is not carried forward.

### Indexing, delete, and logs

Indexing creates an index whose mapping comes from the DataDocument fields. Dictionary types map to keyword, text, date, long, double, or binary. Text with `sortable=Y` adds a keyword subfield. Dates use an explicit format list. Many-relationships are `nested` with `include_in_root`. Bulk index strips `_index`, `_type`, `_id`, and `_timestamp` before the body is stored. Those four are control fields.

Delete removes one document by id, or every document matching a retention filter. The retention jobs call `delete#Documents` for `moqui_logs` (90 days) and `moqui_http_log` (180 days). Both jobs are paused in seed data.

`ElasticSearchLogger` bulk-indexes log events to `moqui_logs`. `ElasticRequestLogFilter` bulk-indexes HTTP requests to `moqui_http_log`. Each creates its own flat mapping. Point-in-time ids are used only by `ElasticEntityListIterator` for large entity scans. Find screens use `from` / `size`.

### Entity datasource

The `logging` datasource group uses `ElasticDatasourceFactory`. `ArtifactHit` is not in that group. The extend-entity is commented out. The only entity on the group is `moqui.test.TestNoSqlEntity`. `ElasticEntityFind` compiles entity conditions to `bool.filter` (`term`, `terms`, `wildcard`, `range`, `exists`, and, or) and rejects joins. That path is not ported. Logs are described under Dashboards and logs.

### Client methods with no application caller

`call`, `callFuture`, `makeRestClient`, `validateQuery`, multi-get, partial document update, and a separate alias-exists check have no application caller. Tools ViewDocument is `get` by index and id, then a pretty-print. That becomes get-by-id.

## API

The types live in `moqui-core` under `search`. The sketches below are the contract. Names can move to match neighboring modules. The trees do not.

A document definition is the current DataDocument: primary entity, field tree, index-time expressions, conditions, links, user groups. The stored document is one JSON object per primary key. Control fields are the document id (`MantleParty`), the record id (`partyId` or the `::`-joined key), and the index timestamp. They are not fields of the body.

### Search

```text
search(target, query, sort, page, highlight) -> SearchPage

target: Documents(document ids) | Index(index name)
```

`Index("mantle")` is every DataDocument whose `indexName` is `mantle`. `Documents(["MantleParty"])` is one type. A hit carries `document_id`, `id`, `score`, the body, and optional highlights.

The body is the nested document. Flattening for a screen is a helper over the map, the same idea as `CollectionUtilities.flattenNestedMap`. It is not a search option and it does not change what is stored.

`SearchPage` is the hits and the total count. Page index, page max, and the range labels stay in the service that faces the screen.

### Query

```text
Query =
    Text { terms, fields: All | Only(names), match: Prefix | Contains | Term }
  | Field { name, op, value }
  | Bool { all: [Query], any: [Query], none: [Query], min_any: number? }
  | Child { path, query: Query }

FieldOp = Eq | In | Prefix | Wildcard | Range { lt, lte, gt, gte } | Exists
          and the negation of each
```

`Text` is the scored clause. It replaces `elasticQueryAutoWildcard` and `*:`. An empty text query matches every document.

`Field` does not score. A name with a dot (`contactMechs.city`, `issuances.issuedSaleTotal` used as a filter) matches if any element of that child array matches. That is the dotted query-string behavior.

`Child` matches if one element of `path` satisfies the whole inner query. Party classification and the Pop store category are `Child`. Two feature ids AND-ed together are two `Field` clauses, because the document must have both features, not both on the same feature row.

`Bool.min_any` is `minimum_should_match`. The classification query sets it to 1.

### Query string

The tools search box and the existing screen expressions pass a string. The facade parses the subset below into a `Query`. Services that are touched for MR-Moqui build a `Query` directly. The parser exists so the box and unmigrated expressions still run.

```text
query     = or
or        = and ("OR" and)*
and       = not ("AND" not)*          juxtaposition is also AND
not       = "NOT" unary | unary
unary     = "(" or ")" | clause
clause    = "_exists_:" field
          | "*:*"
          | field ":" value
          | value
value     = term | quoted | "(" or ")" | range
range     = "[" bound "TO" bound "]" | "{" bound "TO" bound "}"
field     = name ("." name)*
term      = optional leading "*" , text , optional trailing "*"
```

`*:` immediately before a term searches every field. A trailing `*` is `Prefix`. A leading and trailing `*` is `Contains`. A quoted string is a phrase (those terms, that order). `field:[a TO b]` and `field:{a TO b}` are inclusive and exclusive ranges. `*` or an empty string matches all. `_NA_` is an ordinary term.

Anything else is an error. The current client sets `lenient: true` and drops illegal clauses. The parser reports the clause instead. The tools screen link points at this grammar. It stops pointing at the Elastic query-string manual.

The OpenSearch log viewer is the exception. It sends the string through as a `query_string` on the log index. That path is not this parser, and it exists only on the OpenSearch backend.

### Sort, page, highlight

Sort is a field and a direction. A text field sorts on its unanalyzed value, which is the `.keyword` subfield today. Sorting a field that has no unanalyzed value is an error.

Page is offset and limit. The limit cap is 10000, matching the current window. The total is exact. Keyset pagination waits until a caller needs past that cap.

With no sort, order is score descending, then id. `Field`, `Child`, and the `all` / `none` arms of `Bool` do not affect score. `Text` does.

Highlight is an optional field list. It does not change which rows match. Party search is the caller that asks for it. Postgres uses `ts_headline` where the field is in the `tsvector`, and a plain substring snippet otherwise. Whether a given screen renders `highlights` is checked when that screen is ported. The API still returns them.

### Aggregate

```text
aggregate(target, filter: Query, aggs: named Agg) -> named results

Agg =
    Filter { query: Query, aggs }
  | DateHistogram { field, unit, time_zone, format, offset, order, aggs }
  | Terms { field, size, order, aggs }
  | Sum { field }
  | CountDistinct { field }

unit = Day | Week | Month | Quarter | Year
```

`filter` is the top-level constraint (the sales date range and status lists). It does not score.

`DateHistogram` buckets have `key` (bucket start, as a timestamp) and `key_display` (the formatted label, today's `key_as_string`). `offset` is the week-start shift. Quarter is a three-month bucket, which is what the sales service already does by expanding quarter into months and using interval `1q` only when it really wants a quarter.

`Terms` orders by bucket key or by a named metric. `CountDistinct` is an integer. Postgres uses `count(distinct …)`, which is exact, so the API has no precision threshold. The OpenSearch compiler may send `precision_threshold` on its own. Callers do not. OpenSearch cardinality is then approximate, and the integer the caller sees can differ between backends. The sales summaries are the callers that notice.

A `Sum` or `CountDistinct` field may be a root field or a number on each element of a child array. The aggregate is over those numbers. `issuances.issuedSaleTotal` is a sum of child numbers. It is not a `Child` query and it is not a nested aggregation.

`Filter` inside the tree is how product lines, discounts, and new customers are separated. The inner query is the same `Query` type.

There is no pipeline aggregation, percentile, significant-terms, composite, numeric histogram, or query-time script.

### Sales summary, before and after

Today `get#SalesOrderSummary` builds a map and reads the response like this:

```text
query: bool.filter [
  range placedDate gte/lt,
  terms partStatusId, terms statusId, optional id lists
]
aggs DateOuter: date_histogram placedDate
  orderCount: cardinality orderId
  totalSales: sum itemSaleTotal
  ItemProduct: filter term itemTypeEnumId = ItemProduct
    productSaleTotal: sum itemSaleTotal
    productIssuedTotal: sum issuances.issuedSaleTotal
    productCostTotal: sum issuances.issuedCostTotal
    productQuantityTotal: sum orderQuantity
  ItemDiscount: filter terms itemTypeEnumId in discount types
    discountTotal: sum itemSaleTotal
  NewCustomer: filter term isNewCustomer = Y
    newCustomerOrderCount: cardinality orderId

read: aggregations.DateOuter.buckets[]
  .key_as_string, .orderCount.value, .ItemProduct.productSaleTotal.value, ...
```

The service builds the same tree as `Agg` values and reads `key_display` and metric numbers. Moving averages, "this / last / prior", and the reversal of newest-first buckets stay in the service. They are presentation. The screen's output fields stay the same.

`get#SalesInvoiceSummary` is the same shape on `MantleSalesInvoice` and `MantleSalesInvoiceItem`. `get#SalesTopProduct` and `get#InvoicesByCustomerClass` use `Terms` as the outer aggregation instead of `DateHistogram`.

### Index and delete

```text
ensure_index(document_id)
index_documents(document_id, documents)
delete_document(document_id, id)
delete_matching(target, filter: Query)
get_document(document_id, id) -> body
```

`ensure_index` creates the store from the DataDocument definition. There is no separate put-mapping call. A definition change rebuilds the mapping on ensure. Bulk index is the write path the feed uses. `delete_matching` is the retention job: a `Field` range on the index timestamp, and an optional extra filter.

The feed itself stays. Entity create, update, and delete still register documents. The receive service calls `index_documents`. The delete service calls `delete_document`. Rebuild stays a batched read through `EntityDataDocument`.

The feed writes on the same transaction as the entity change when the search store is that same database. That is the default embedded cluster, and it is an external Postgres that is also the entity database. Commit and rollback then cover the row and the document. When the search store is a different database (entity tables on MySQL, or a search URL that points at another Postgres), the feed runs after the entity transaction commits. There is no XA. A search failure after that commit leaves the document to be rebuilt.

## Postgres

Schema `search`, so the tables sit beside entity tables on the default embedded cluster and whenever both stores are the same Postgres. One table per DataDocument. The table name is the physical document name (`mantle_party`). When search is its own cluster, the schema is still `search`.

Each table has:

- `doc_id text` primary key
- `indexed_at timestamptz`
- `body jsonb`, the full nested document
- a typed column for every field used in a filter, a sort, or an aggregation
- a generated `tsvector` for every text field, default config `english`, with a GIN index

Child arrays stay inside `body`. A `Child` query is an `EXISTS` over `jsonb_array_elements` so one element has to satisfy every inner predicate. A dotted `Field` is the same extraction with a single predicate. A sum of a child number is a lateral sum of those elements, or a `jsonb` sum.

```sql
EXISTS (
  SELECT 1
  FROM jsonb_array_elements(body -> 'classifications') AS c
  WHERE c ->> 'partyClassificationId' = $1
    AND (c ->> 'classFromDate')::timestamptz <= $2
    AND (
      c ->> 'classThruDate' IS NULL
      OR (c ->> 'classThruDate')::timestamptz >= $2
    )
)
```

If a measured document is too slow in that form, that document gains a child table. The API does not change.

`DateHistogram` is `date_trunc` in the requested time zone. The week offset uses `date_bin`. `Terms` is `GROUP BY`, `ORDER BY`, and `LIMIT`. Index-time expressions are stored values. They are not re-evaluated in SQL.

Dates and timestamps in `body` are ISO-8601. Range comparisons in the API take timestamps. The current client serializes timestamps with a custom Jackson serializer because ElasticSearch rejects a negative epoch millis. Postgres stores a `timestamptz` and does not need that workaround. The OpenSearch compiler formats dates in the mapping's accepted formats.

Text rank is `ts_rank` on the vectors the `Text` clause names. Filters are SQL `WHERE` and do not affect rank.

The default is the `search` schema on the embedded entity cluster. When the entity datasource is MySQL or another non-Postgres, the default is a separate embedded PGLite cluster. Config may point the search store at an external Postgres, the same instance as the entities or another one. Placement and how many postmasters start are `plans/PgliteConcurrency.md`.

## OpenSearch

The compiler emits the JSON the current services emit.

- `Text` becomes a `query_string` (or a `multi_match` when the clause is only terms). It sits in `bool.must`, so it scores.
- `Field`, `Child`, and non-text `Bool` arms sit in `bool.filter`.
- `Child` becomes a `nested` query with the path and the inner query.
- `DateHistogram`, `Terms`, `Sum`, `CountDistinct`, and `Filter` become `date_histogram`, `terms`, `sum`, `cardinality`, and `filter`.

Mappings follow `makeElasticSearchMapping`. Many-relationships stay `type: nested` with `include_in_root: true`, so dashboards that already query dotted names keep working. Keyword subfields stay for sortable text. Document types, the pre-7 interval name, and the old point-in-time paths do not.

`CountDistinct` may send `precision_threshold` inside the HTTP body. The value returned to the caller is the integer from the response.

## Config

`search-facade` replaces `elastic-facade`.

```xml
<search-facade backend="postgres"/>

<search-facade backend="postgres">
    <postgres url="..." user="..." password="..."/>
</search-facade>

<search-facade backend="opensearch">
    <cluster url="${opensearch_url}" user="${opensearch_user}" password="${opensearch_password}"
             index-prefix="${opensearch_index_prefix}"/>
</search-facade>
```

`backend` defaults to `postgres`. With no `postgres` element, tables live in the `search` schema of the embedded entity cluster, or on a dedicated embedded cluster when the entity datasource is not that Postgres. With `backend="opensearch"` the cluster element is required and the Postgres search store is not opened.

Service parameters named `clusterName` go away. There is one backend. The `logger` cluster name goes away with the logging entity group.

## Dashboards and logs

OpenSearch Dashboards, the `/elastic` and `/kibana` proxies, and the `ElasticRemote` and `KibanaRemote` permissions stay. They do nothing when the backend is Postgres, because there is no cluster to proxy. People who build visualizations on the Mantle indexes keep doing that when they choose the OpenSearch backend. The documents they see are the same documents the facade indexes. The facade is not a report designer, and Postgres is not given a dashboard server.

Application logs and HTTP request logs are flat indexes, `moqui_logs` and `moqui_http_log`, written by the host. They are not DataDocuments. The log viewer and the two cleanup jobs run when the backend is OpenSearch. When the backend is Postgres, log events stay in the tracing output and the log viewer reports that the search backend has no log index. `Crate-Layout.md` already places the log shipper on the host, beside the OpenSearch process.

## Caller migration

| Caller | Becomes |
|---|---|
| `SearchServices.search#DataDocuments` | `SearchFacade.search`. A query string is still accepted and parsed. |
| `index#DataDocuments`, `index#DataFeedDocuments`, `delete#DataDocument`, `delete#Documents` | `index_documents`, the feed rebuild, `delete_document`, `delete_matching` |
| `put#DataDocumentMappings`, `delete#ElasticIndex` | `ensure_index`. Dropping a document store is an admin method on the backend. |
| `search#MantleFiltered` | Same service. The org limit is a `Field` `In` list. |
| Party, product, task, request, project, HiveMind, PopCommerce, PopRestStore | `Text`, `Field`, and `Child`. Auto-wildcard text stays `Text`. |
| The four `SalesReportServices` | `aggregate`. Screen fields unchanged. |
| `get#InventoryProjectedInfo` | Rewritten on `aggregate`, or left unimplemented until then. The current body has nothing to port. |
| Tools DataDocument Search, ViewDocument | Parsed query string. `get_document`. |
| Log viewer, `ElasticSearchLogger`, `ElasticRequestLogFilter`, cleanup jobs | OpenSearch backend only. |
| `ElasticDatasourceFactory`, `TestNoSqlEntity` | Not ported. |
| `ec.elastic` / `getElastic()` | `ec.search` |

DataDocument and DataFeed entity XML stay. Seed documents stay. Groovy expressions inside field paths are part of the mantle expression migration to JavaScript. This design does not change those expressions.

## Out of scope

- A MySQL implementation of `SearchBackend`. Search tables are Postgres even when entity tables are MySQL.
- Writing each document to Postgres and OpenSearch. One backend is active.
- An entity group stored in the search engine. `ElasticEntityFind` and the `logging` datasource group stop.
- `text-fts` column search. That remains the entity engine.
- Point-in-time and scroll. Paging is offset and limit.
- A raw REST escape hatch on the facade. The OpenSearch compiler is the only place that builds HTTP paths.
- Pipeline aggregations, percentiles, significant terms, composite aggregations, numeric histograms, and query-time scripts.
- ElasticSearch version forks older than the shared 7+ / OpenSearch subset.
- `manualMappingServiceName`. Nothing calls it.
