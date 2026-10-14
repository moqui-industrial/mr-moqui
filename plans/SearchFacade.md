# SearchFacade

Replacement for `ElasticFacade`. The API is a document search and aggregation tree. Postgres implements the whole thing and is the default. OpenSearch is an optional backend for the same API, and the only backend that keeps OpenSearch Dashboards.

This follows `plans/Architecture-Proposal.md` (Search) and `plans/Crate-Layout.md` (`search` module, `SearchBackend`). The inventory below is from the Moqui tree at `../moqui`: framework, runtime, mantle-udm, mantle-usl, SimpleScreens, and the other components that call the same services.

## Decision

`SearchFacade` is not a port of `ElasticFacade`. `ElasticFacade` takes ElasticSearch JSON maps and posts them. Mapping that JSON onto Postgres would freeze the Elastic query language into the framework. The facade sits one layer above that. Callers pass a query tree and an aggregation tree. Each backend compiles those trees.

Postgres is the default backend. When the entity datasource is Postgres, embedded or external, default search tables are the `search` schema on that database. A `<postgres>` URL is how search points at a different Postgres. When the entity datasource is not Postgres (MySQL, for example), search is its own embedded cluster. There is no MySQL search backend. The store is independent of a non-Postgres entity datasource.

OpenSearch is one optional HTTP backend. It speaks the shared `_search` and `_bulk` subset that current OpenSearch and ElasticSearch 7+ have in common. The runtime already ships OpenSearch and OpenSearch Dashboards 3.4. One client covers both products for this subset. The ElasticSearch 6 and pre-7 branches in `ElasticFacadeImpl` (`interval` versus `calendar_interval`, old point-in-time paths, document types) stop here.

One active backend. Applications and the built-in reports always call `SearchFacade`. When the backend is OpenSearch, the indexes stay available to OpenSearch Dashboards through the existing `/elastic` and `/kibana` proxies. When the backend is Postgres, those dashboards are absent. The facade does not write to two stores. External reporting is a property of the OpenSearch backend. In-app reports use the aggregation API on whichever backend is configured.

`moqui-core` owns the document model, the query and aggregation trees, the Postgres SQL, and the OpenSearch request bodies. The SQL compiler stays in core and uses `moqui-db` for quoting and placeholders. Bodies go out through `HttpClient`. `SearchBackend` is the trait. Starting an OpenSearch process stays in `moqui-host`.

Column full-text (`text-fts` on an entity field, `../moqui/framework/plans/DbFullTextSearch.md`) stays an EntityFacade feature. Skills, wiki bodies, and similar one-table text are that feature. `SearchFacade` is denormalized documents, nested child constraints, and aggregations.

## What ElasticFacade is used for

`org.moqui.context.ElasticFacade` is a named-cluster HTTP client. `search`, `count`, `deleteByQuery`, and `validateQuery` accept a `Map` that is sent as JSON. `org.moqui.search.SearchServices.search#DataDocuments` builds that JSON for find screens. `mantle.sales.SalesReportServices` skips the service and calls `elasticClient.search` with aggregation JSON.

### Documents and feeds

The document model is already independent of the search engine. It stays.

Definitions are in `framework/entity/EntityEntities.xml`. `EntityDataDocument` builds the maps. `EntityDataFeed` pushes them.

- `DataDocument` names a primary entity, an `indexName` (a group name, today an alias), field paths, relationship aliases, conditions, links, and user groups. `manualDataServiceName` can add fields after the query. `manualMappingServiceName` stays. `ElasticFacadeImpl` calls it when the field is set and passes the generated mapping. Seed data does not set it.
- A field path is `Rel:Rel:field`, or an expression with a required alias. Expressions run at index time. In MR-Moqui they are JavaScript, same as other expressions. `fieldType` overrides the dictionary type. `sortable=Y` on a text field keeps an unanalyzed copy for sort.
- A relationship of type one is flattened into the parent object. A relationship of type many becomes an array of objects. MantleParty nests roles, identifications, contact mechs, user accounts, classifications, and notes. MantleSalesOrderItem nests issuances, and also stores header expressions (`itemSaleTotal`, `customerClassId`) computed from those children.
- A `DataDocumentCondition` is either part of the entity query (`MantleSalesOrderItem` requires `vendorRoleTypeId = OrgInternal`) or a post-query check.
- Document id is the primary key values joined with `::`.
- `DataFeed` is real-time push (`DTFDTP_RT_PUSH`) on create, update, and primary-entity delete, or manual pull. The receive service defaults to `index#DataDocuments`. The delete service defaults to `delete#DataDocument`. `index#DataFeedDocuments` rebuilds a feed in batches of 1000. The `IndexDataFeedDocuments` job wraps that call.
- Seed data is 19 `DataDocument` rows, 5 `DataFeed` rows, and 24 `DataFeedDocument` rows. Four of the feeds index through `index#DataDocuments`. `MantleSearch` (index `mantle`) is one feed split across `MantleDocumentData.xml` and `MantleDocumentWorkData.xml` (same `dataFeedId`, 9 documents: party, communication event, product, facility, wiki page, project, task, event, request). `MantleSales` (`mantle_sales`) has the four sales documents. `MantleInventoryData` (`mantle_inventory`) has the five inventory documents. `PopCommerceSearch` (index `popc`) indexes `PopcProduct`, a clone of `MantleProduct` with a post-query store condition. `MyAccountNotifications` sends task, event, communication event, request, and wiki page to `mantle.party.PartyServices.send#DataDocumentNotifications`. HiveMind and PopCommerce add screen links. The extra `MantleSearch` feed in HiveMind is inside a comment.
- `esTypeMap` in `ElasticFacadeImpl` maps `text-fts` to `text`. A DataDocument field of that entity type is indexed as text. Column full-text stays the EntityFacade feature in `../moqui/framework/plans/DbFullTextSearch.md`. This mapping is the document-field case that page leaves open.

The physical name of a document type is `dataDocumentId` converted from camel case to underscore (`MantleParty` to `mantle_party`). `indexName` groups those types. A search with no document type searches the group.

### Find screens

`search#DataDocuments` sends a `bool.must` containing a Lucene `query_string` (`lenient: true`, default time zone), plus an optional `nestedQueryMap`. Each entry with a path becomes a `nested` query (`score_mode: avg`). A null path is a top-level clause. Sort accepts a leading `+` or `-`. Highlight is a list of fields. Paging is `from` / `size` with `track_total_hits`. `pageNoLimit` caps at 10000. Hits can be flattened. Each hit is given `_index`, `_id` (the record id), `_type` (the DataDocument id, such as `MantleParty`; a hit type of `_doc` is converted from the index name), `_version`, and `highlights` when highlight was requested.

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
| Tools LogViewer | Raw `query_string` against the log index: sent as written on OpenSearch, read with the facade grammar on Postgres |

### Built-in reports

`mantle.sales.SalesReportServices` builds the JSON itself. SimpleScreens `Accounting/Reports/SalesSummary.xml` calls it. The MarbleERP sales skill describes the same aggregations.

The filter is a `bool.filter`: a date `range` (`gte` / `lt`, epoch millis) and `terms` on status, plus optional id lists (vendor, customer, store, classification, product).

The aggregation tree is:

- `date_histogram` on a date field. Calendar units are day, week, month, quarter, and year. Time zone, a display format, an optional offset (week start), newest bucket first. Buckets expose `key` and `key_as_string`.
- `terms` on `productId` or `customerClassId`. `get#SalesTopProduct` sends a size and an order by a sub-aggregation. `get#InvoicesByCustomerClass` sends a size and no order, then sorts the merged rows in the service.
- `sum` and `cardinality` (`precision_threshold` 40000) on `orderId` or `invoiceId`.
- a `filter` aggregation (`term` or `terms`) wrapping those metrics. Product lines are `itemTypeEnumId = ItemProduct`. Discounts are a fixed list of item types. New customers are `isNewCustomer = Y`.
- `sum` of a number on each element of a child array (`issuances.issuedSaleTotal`, `issuances.issuedCostTotal`). The mapping marks issuances `nested` and `include_in_root`, so the sum sees every child value. It is not a correlated nested aggregation.

The histogram interval sent for a quarter is `1q`. The display count for a quarter is `intervalCount * 3`, and the search window is that display count plus the moving-average period count. A week with no explicit offset uses `firstDayOfWeek` against Monday, which is the Elastic default week start. The invoice histogram sets `min_doc_count: 1`. Postgres `date_bin`, or an equivalent bucket expression, has to reproduce the week offset, quarter buckets, and the empty buckets the histogram keeps. Newest-first order, the reverse for display, and the moving average stay in the service, as below.

The four services are `get#SalesOrderSummary`, `get#SalesInvoiceSummary`, `get#SalesTopProduct`, and `get#InvoicesByCustomerClass`.

`mantle.product.InventoryReportServices.get#InventoryProjectedInfo` does not run. Its actions import `org.moqui.elasticsearch.ElasticSearchUtil`, and that class is not in the repository. The five live calls are `simpleAggSearch` for order item, production estimate, asset, run consume, and run produce. `get#PriceHistoryInfo` imports the same class, calls `aggregationSearch` once, and has no callers. `search#CountBySource` is inside an XML comment above those actions, and the service is commented out in `SearchServices.xml`. The inventory documents and the Mantle Inventory feed are real. `get#InventoryProjectedInfo` is a rewrite onto `aggregate`: `terms` on `productId`, `sum` of the quantity fields, an optional `term` on `facilityId`, an optional range on a date. `ElasticSearchUtil` is not carried forward.

### Indexing, delete, and logs

Indexing creates an index whose mapping comes from the DataDocument fields. Dictionary types map to keyword, text, date, long, double, or binary. `text-fts` maps to text, the same entry `esTypeMap` already has. Text with `sortable=Y` adds a keyword subfield. Dates use an explicit format list. Many-relationships are `nested` with `include_in_root`. Bulk index strips `_index`, `_type`, `_id`, and `_timestamp` before the body is stored. Those four are control fields.

Delete removes one document by id, or every document matching a retention filter. The retention jobs call `delete#Documents` for `moqui_logs` (90 days) and `moqui_http_log` (180 days). Both jobs are paused in seed data.

`ElasticSearchLogger` bulk-indexes log events to `moqui_logs`. `ElasticRequestLogFilter` bulk-indexes HTTP requests to `moqui_http_log`. Each creates its own flat mapping. Point-in-time ids are used only by `ElasticEntityListIterator` for large entity scans. Find screens use `from` / `size`.

### Entity datasource

The `logging` datasource group uses `ElasticDatasourceFactory`. `ArtifactHit` is not in that group. The extend-entity is commented out. The only entity on the group is `moqui.test.TestNoSqlEntity`. `ElasticEntityFind` compiles entity conditions to `bool.filter` (`term`, `terms`, `wildcard`, `range`, `exists`, and, or) and rejects joins. That path is not ported. Logs are described under Dashboards and logs.

### Client methods with no application caller

`call`, `callFuture`, `makeRestClient`, multi-get, partial document update, and a separate alias-exists check have no application caller. `validateQuery` does: `search#DataDocuments` calls it with explanations on and, on failure, adds the danger message `Invalid search: ${queryString}` and returns before searching. The explanations payload is discarded. The TODO in `SearchServices.xml` is to pull that Elastic message. The query and the response are written to the server log. Tools ViewDocument is `get` by index and id, then a pretty-print. That becomes get-by-id.

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

`SearchPage` is the hits and the total count. Page index, page max, and the range labels stay in the service that faces the screen. A hit also carries the physical store name, which `search#DataDocuments` returns as `_index`; `_type` is the document id and `_id` the record id. HiveMind QuickSearch and SimpleScreens `Search.xml` read `_type`. `_version` has no caller and is not returned.

### Query

```text
Query =
    MatchAll
  | Text { terms, fields: All | Only(names), match: Term | Prefix | Contains | Phrase | Wildcard }
  | Field { name, op, value, values, gt, gte, lt, lte, negate }
  | Bool { all: [Query], any: [Query], none: [Query], min_any: number? }
  | Child { path, query: Query }

FieldOp = Eq | In | Prefix | Wildcard | Range | Exists
```

Scripts pass the same tree as maps: `"match_all"`, `{text: {terms, fields: "all" | [names], match}}`, `{field: {name, op, value, values, gt, gte, lt, lte, negate}}`, `{bool: {all, any, none, min_any}}`, `{child: {path, query}}`. A string in that place is a query string.

`Text` is the scored clause. It replaces `elasticQueryAutoWildcard` and `*:`. An empty text query matches every document. `Phrase` preserves term order and adjacency after configured text analysis; quoted query-string input compiles to this variant. PostgreSQL compiles it with `phraseto_tsquery`, and OpenSearch uses an equivalent phrase query. Golden tests distinguish adjacent, reversed, and separated terms, including stop-word handling. `simple` keeps stop words, as Elastic's `standard` analyzer does. A phrase does not run from one stored value into the next, on the root, across fields, or across child elements: `jsonb_to_tsvector` leaves a position gap between values. Highlights mark a phrase's words only where they are adjacent.

How a clause matches depends on the field's kind. That is the Elastic behavior for `text` and `keyword` mappings, and the screens rely on it:

| Kind | `field:v` | `field:v*` | `field:*v*` |
|---|---|---|---|
| text | a token, case-insensitive | a token prefix | a substring, case-insensitive |
| keyword | the exact value, case-sensitive | a value prefix, case-sensitive | a substring, case-sensitive |
| number, date | equal after conversion; a bare date is that UTC day | no match | no match |

A bare term, or `*:term`, searches every string and number in the document, children included. A value that cannot be read for its field (text on a number field) matches nothing, as the Elastic `lenient` flag had it. A field the document does not have matches nothing.

`Field` does not score. A name with a dot (`contactMechs.city`, `issuances.issuedSaleTotal` used as a filter) matches if any element of that child array matches. That is the dotted query-string behavior.

`Child` matches if one element of `path` satisfies the whole inner query. Party classification and the Pop store category are `Child`. Two feature ids AND-ed together are two `Field` clauses, because the document must have both features, not both on the same feature row.

`Bool.min_any` is `minimum_should_match`. The classification query sets it to 1. With none given it is 1 when `all` is empty and `any` is not, and 0 otherwise, as an Elastic `bool` has it. PopRestStore's category query is a should-only `bool` and relies on that default. A `bool` with only `none` matches every document except those. A negated clause also matches a document that lacks the field, as `must_not` does.

### Query string

The tools search box and the existing screen expressions pass a string. The facade parses the subset below into a `Query`. Services that are touched for MR-Moqui build a `Query` directly. The parser exists so the box and unmigrated expressions still run.

```text
query     = clause*
clause    = [conj] [mod] (field ":")? unary
conj      = "AND" | "&&" | "OR" | "||"
mod       = "+" | "-" | "NOT" | "!"
unary     = "(" query ")" | range | quoted | term
range     = ("[" | "{") bound "TO" bound ("]" | "}")
field     = name ("." name)*
term      = characters, "\" escapes; "*" and "?" are wildcards
```

Clauses combine as Lucene's classic parser combines them, which is what `query_string` did. Juxtaposition is OR (the default operator). `AND` makes the clauses on both sides required. `+` requires a clause, and `-` and `NOT` prohibit one. A group with only optional clauses needs one of them. So `(*:jo* *:sm*) AND roles.roleTypeId:Customer` is "either term, and the role".

`*:` immediately before a term searches every field, as a bare term does. A trailing `*` is `Prefix`. A leading and trailing `*` is `Contains` (Postgres: trigram-indexed `ILIKE` on a text column, a substring of a token on all fields; OpenSearch: wildcard). Other `*` and `?` are `Wildcard`. A quoted string is a phrase (those terms, that order). `field:[a TO b]` and `field:{a TO b}` are inclusive and exclusive ranges, and `*` is an open bound. `field:*` and `_exists_:field` are exists. `*`, `*:*`, or an empty string matches all. `_NA_` is an ordinary term. A hyphen, `@`, or `.` inside a term is part of it, so `foo-bar` and `jo@example.com*` are single terms. A leading `-` is the prohibit operator, which PopRestStore's split user input can produce.

Fuzzy (`~`), boost (`^`), and regex (`/re/`) are errors that name the clause. `search#DataDocuments` turns a parse error into the danger message `Invalid search: ...` with that detail and an empty page, as it did for a failed validate. The tools screen link points at this grammar. It stops pointing at the Elastic query-string manual.

`search#DataDocuments` also still takes `nestedQueryMap` with Elastic query JSON (a map or JSON text), because `PartyServices.search#Party` and PopRestStore build it and both live outside this repository. The service reads the subset they use into a `Query`: `bool` (`must`, `filter`, `should`, `must_not`, `minimum_should_match`), `term`, `terms`, `match`, `range` (`gt`, `gte`, `lt`, `lte`, epoch millis or ISO), `exists`, `prefix`, `wildcard`, `nested`, and `query_string`. Each path key becomes a `Child`. Other Elastic query types are an error that names them. Callers that are migrated pass a `query` map instead.

The log viewer is the exception on OpenSearch. It sends the string through as a `query_string` on the log index. On Postgres the same string is read by this parser, which covers the viewer's forms: `@timestamp:[a TO b]` ranges, `field:term`, `AND`/`OR`/`NOT`, groups and phrases.

### Sort, page, highlight

Sort is a field and a direction. A text field sorts on its unanalyzed value, which is the `.keyword` subfield today. A trailing `.keyword` in a sort or query name is accepted and names that same field. Sorting a text field that is not `sortable` is an error. A field of a child array sorts by its smallest value ascending and its largest descending, as Elastic sorts a multi-valued field: `search#Party` sorts on `userAccounts.username.keyword`. Missing values sort last in either direction. `_score` and `_id` sort by rank and record id.

Page is offset and limit. The limit cap is 10000, matching the current window. The total is exact. Keyset pagination waits until a caller needs past that cap.

With no sort, order is score descending, then id. `Field`, `Child`, and the `all` / `none` arms of `Bool` do not affect score. `Text` does.

Highlight is an optional field list. It does not change which rows match. Party search is the caller that asks for it, and `FindParty.xml` renders it: the shape is `{field: [fragment, ...]}` and matched words are wrapped in `<em>` and `</em>`, which the screen rewrites to `<strong>`. Fragments are made in Rust from the stored body and the query's positive text clauses, so both backends return the same shape. Only fields with a match appear. The text is not HTML-escaped, as Elastic does not escape it.

### Aggregate

```text
aggregate(target, filter: Query, aggs: named Agg) -> named results

Agg =
    Filter { query: Query, aggs }
  | DateHistogram { field, unit, time_zone, format, offset, order, min_doc_count, aggs }
  | Terms { field, size, order, aggs }
  | Sum { field }
  | CountDistinct { field }

unit = Minute | Hour | Day | Week | Month | Quarter | Year
```

The map form is the Elastic aggregation JSON the services already build, so a migrated service changes little: `{date_histogram: {field, calendar_interval | unit, time_zone, format, offset, order: {_key: desc}, min_doc_count}, aggs}`, `{terms: {field, size, order: {name: desc}}, aggs}`, `{sum: {field}}`, `{cardinality | count_distinct: {field}}`, and `{filter: <query>, aggs}` where the query is an Elastic `term`/`terms`/`bool` map or `{query: <query map>}`. `precision_threshold` is accepted and ignored.

Results are keyed by aggregation name. A bucket aggregation is `{buckets: [{key, key_display, doc_count, <sub>...}]}`. A filter is `{doc_count, <sub>...}`. A metric is a number, where Elastic had `{value}`. `get#SalesInvoiceSummary` and `get#InvoicesByCustomerClass` read `doc_count` from buckets as the invoice count, so it is part of the shape.

`filter` is the top-level constraint (the sales date range and status lists). It does not score. An aggregation runs over one document type.

`DateHistogram` buckets have `key` (bucket start, as a timestamp) and `key_display` (the formatted label, today's `key_as_string`). `format` is a Java `DateTimeFormatter` pattern in the root locale, so `dd MMM yyyy` shows `04 Jan 2026`; the invoice summary merges its two histograms by that string. `time_zone` defaults to the server zone. `offset` is an Elastic duration (`+6d`, `-1d`, `+3h`) and applies to any unit: the bucket is `trunc(local - offset) + offset`. The weekly sales default sends the user's first day of the week against Monday, which is `-1d` for Sunday. Quarter is one three-month bucket. The sales service sends interval `1q` for that bucket. `intervalCount * 3` is the window length in months, not a rewrite of the bucket into month buckets. `order` is by key, ascending unless `desc`. With `min_doc_count` 0 (the default) every period between the first and last non-empty period has a bucket, as Elastic fills them; periods outside that span, though inside the filter's date range, have none. The invoice histogram sends `min_doc_count: 1`, which drops empty periods.

`Terms` orders by `_count` (the default, descending, then key ascending), `_key`, or a named metric, and keeps `size` buckets. A document with no value for the field is in no bucket, as Elastic has no missing bucket: discount items have no `productId`. A sum over nothing is 0, not null, as Elastic returns it, because the services add and divide the sums. `CountDistinct` over nothing is 0. `CountDistinct` is an integer. Postgres uses `count(distinct …)`, which is exact, so the API has no precision threshold. The OpenSearch compiler may send `precision_threshold` on its own. Callers do not. OpenSearch cardinality is then approximate, and the integer the caller sees can differ between backends. The sales summaries are the callers that notice.

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

`ensure_index` creates the store from the DataDocument definition. There is no separate put-mapping call. A definition change rebuilds the mapping on ensure. When `manualMappingServiceName` is set, `ensure_index` runs that service with the generated mapping, as `ElasticFacadeImpl` does. Seed data does not set it. Bulk index is the write path the feed uses. `delete_matching` is the retention job: a `Field` range on the index timestamp, and an optional extra filter. That range hits the `indexed_at` column. The index timestamp is a control field, not a field of the body.

The feed itself stays. Entity create, update, and delete still register documents. The receive service calls `index_documents`. The delete service calls `delete_document`. Rebuild stays a batched read through `EntityDataDocument`.

Entity create, update, and delete register the written values during the transaction, keyed by the transaction frame, so a `force-new` commit takes only its own. After the transaction-cache flush and before commit, on the enlisted connection, the feed resolves each write to the documents it touches and inserts `moqui.entity.feed.DataFeedPending`: `dataDocumentId`, the primary entity, the document's record id, and the operation (`index` or `delete`). A write to the primary entity keys itself; a delete is a `delete` row. A write to a related entity is walked back along the document's relationship path, from the written values (so a deleted child still finds its parent), to the primary records it reaches, each an `index` row. Rollback drops the writes. `suspend` parks that client as the Transactions section of `plans/Architecture-Proposal.md` specifies. After commit the inserted row ids go to `org.moqui.EntityServices.process#DataFeedPending` on the service pool. That service assembles each document, calls the receive service of every real-time feed that has the document (`documentList`, defaulting to `index#DataDocuments`), calls the feed's delete service for a document that no longer assembles (gone, or now outside the document's conditions), and deletes the rows it delivered. Feeds that receive through their own service, such as `MyAccountNotifications`, run the same way. A full pool stores the call as a one-shot job. The `process_DataFeedPending_frequent` job delivers rows that remain, such as those left while search was not configured. Same-database Postgres, a second Postgres, and OpenSearch use this path. There is no XA. An index failure leaves the pending row and does not roll the entity change back. A later search can see the entity row before the document.

## Universal Mantle search

One universal search box spans authorized [Mantle UDM](https://github.com/moqui/mantle-udm) records: Party, Product, Order, Invoice, Shipment, Asset, Work Effort, and other business entities. It is an application service over `SearchFacade`, not another search backend or business data model. Mantle remains the source of truth; full details come from its entities, services, and relationships.

### Documents, identity, and authorization

Use the existing DataDocument definitions, per-document stores, and `Index(index name)` grouping. Extend the Mantle search feed with missing document types as needed. There is no separate universal `udm_search` table and no replacement of the nested document bodies. Display metadata such as code, title, subtitle, category, attributes, quality, and business URL belongs in the relevant DataDocument definition. Preserve the entity type and original primary-key fields, including composite keys; the facade hit's `(document_id, id)` identifies the stored document.

The application derives identity, tenancy/organization scope, and permitted documents from the authenticated session. A scope or owner-org match alone is not record-level authorization. Apply trusted record-level permission predicates before candidate ranking, paging, counts, facets, and snippets. Use indexed permission relations or authorization-aware queries where appropriate; an allowed-ID array is suitable only for small populations. Do not fetch unauthorized candidates and hide them after ranking. Authorized links and fetched business details must enforce the same policy.

Feed registration is transactional; document assembly and indexing remain eventual, exactly as Index and delete specifies. Index failures leave pending rows and do not roll business data back. PostgreSQL search data participates in the search database's crash recovery and PITR, but recovery must also resume pending feeds and reconcile/rebuild derived documents when the business and search databases are restored independently.

### Retrieval features and facade boundary

The baseline `Query`, `Agg`, sorting, highlighting, exact totals, and offset/limit paging above remain authoritative. Universal search uses `Text`, `Field`, `Child`, and `aggregate` through that contract; it does not send raw SQL or PostgreSQL query strings around the facade parser. OpenSearch remains optional, with one active backend and no dual indexing.

Additional universal-search goals are typo recovery, proximity, relevance boosting, optional semantic/vector retrieval, hybrid reciprocal rank fusion (RRF), and multilingual analysis. These require explicit, opt-in facade capability extensions before an application exposes them. They are not silently new meanings for `Text` or query-time scripts. Define the typed inputs, ranking/filter semantics, and backend support for each extension; unsupported modes return a clear capability error, not a hidden fallback or a second backend. Existing lexical search works without embeddings or a hosted inference service. The `simple` text configuration and existing `ts_rank` behavior stay the defaults; language-specific and multilingual configuration waits on a corpus that needs it.

For the PostgreSQL extensions, use independent indexed retrieval branches:

- Exact identifiers and beginning-of-title prefixes use B-tree indexes on appropriately normalized fields. Escape literal `%` and `_` in user prefix input. Word-internal autocomplete needs a text-query prefix path or a separate suggestion projection, not just beginning-of-title matching.
- Lexical retrieval uses native GIN vectors. Phrase parsing must preserve the facade's quoted-string semantics; PostgreSQL phrase/proximity primitives such as `phraseto_tsquery` and `tsquery_phrase` are compiler implementation details. `websearch_to_tsquery` must not replace the facade's field-aware grammar.
- Fuzzy matching uses a measured GiST `pg_trgm` nearest-neighbor path on normalized display text, conditionally for typo recovery. It is not an unconditional branch on every request and does not replace the existing GIN trigram `Contains` path.
- Optional semantic retrieval uses a locally generated query embedding and the document's matching embedding model/dimensions. No query embedding means no semantic branch. Filter every branch by the same authorization and business predicates.
- Hybrid retrieval merges bounded branch candidates by full facade document identity, not record id alone. A reference RRF score is `sum(branch_weight / (60 + branch_rank))`; exact matches can receive a larger weight. Optional positive quality boosts apply before final top-K. Candidate limits, weights, and tie-breaking must be specified and benchmarked rather than treated as universal constants.

In the code, `SearchBackend::capabilities()` lists `Capability` values (`proximity`, `fuzzy`, `autocomplete`, `weightedRank`, `accent`, `semantic`, `hybrid`), and the facade refuses a search or aggregation that needs one the backend lacks with an error naming it. Scripts read the list from `ec.search.capabilities()`. `DataDocumentField.searchRole` (title, subtitle, code, category, quality) and `searchWeight` (A to D) on root fields fill `DocumentMapping.display`, and they are part of the stored shape.

- `{near: {terms, fields, distance}}`: the terms in order with at most `distance` other words between the first and last, inside one stored value. Postgres checks positions from `unnest(tsvector)` after a GIN-indexed `@@`, one text field at a time, because `all_text` leaves too small a gap between values for a window. OpenSearch uses `intervals` with `max_gaps` and `ordered`, one field at a time.
- `{fuzzy: {terms}}`: the `title` role only. Postgres matches `fold(terms) <% norm_title` (pg_trgm word similarity over `pg_trgm.word_similarity_threshold`, GiST-indexed) and scores by `word_similarity`. OpenSearch is a `match` with `fuzziness: AUTO` and `operator: and`. The two agree on plain typos and can differ at the edges. Without `pg_trgm` Postgres does not list `fuzzy`.
- `{suggest: {text}}`: the `title` role only. The title starts with the text (Postgres `norm_title LIKE fold(text)%` on a `text_pattern_ops` index; OpenSearch `prefix` on a `title.norm` keyword with a lowercase normalizer), or each word starts a title word (prefix `tsquery`; `match_bool_prefix`). A title start scores higher.
- `rank: "coverDensity"`: Postgres ranks with `ts_rank_cd` over a generated `ranked_text`: weighted fields at their weight, then all text at D. OpenSearch adds the positive text terms as an optional `multi_match` over the weighted fields, boosted A 4 to D 1, so the set of matches does not change.
- `norm_title` is a generated column, the title passed through the store's fold function: `lower`, or with `unaccent="true"` an immutable `moqui_fold` wrapper in the store's schema. `text-config` (default `simple`) and `unaccent` on the `postgres` element select the configuration; with `unaccent` a copy named `moqui_unaccent_<base>` puts `unaccent` before the base configuration's word dictionaries, and Postgres lists `accent`. Both are part of the registry signature, so a change rebuilds the tables. OpenSearch does not fold accents yet and does not list `accent`.
- A search over several document types on OpenSearch compiles each type's query against its own mapping and gates it on `_index`, so roles and weights that differ by type apply where they belong.

The optional lexical ranking extension may use weighted vectors and `ts_rank_cd` for cover-density ranking; it does not change baseline `ts_rank`. Native GIN cannot directly return the highest text ranks, so broad queries may score large matching populations. Measure relevance and p95 latency instead of promising Elasticsearch-equivalent performance or introducing a custom BM25 layer.

The facade pieces for this are in the code. `SearchRequest.filter` (script `filter`) is a trusted query the service adds. It must match too, it does not score or highlight, and both backends apply it before ranking, paging, and totals: Postgres in the `WHERE` clause only, OpenSearch as `bool.filter`. A `Field` on the pseudo-field `_document` is decided per document type when the query compiles (`TRUE`/`FALSE`, `match_all`/`match_none`), so per-type predicates are `bool.any` of `bool.all[_document = T, ...]`. An `aggregate` over several types accepts only `terms` on `_document` with no sub-aggregations: the facade counts each type's exact matching total under the filter and returns the usual bucket shape. Any other multi-type aggregation is still an error.

Hybrid and semantic retrieval run in the facade (`search::hybrid`), so hybrid works on both backends. `SearchRequest.retrieval` is `Lexical` (the default), `Hybrid(options)`, or `Semantic(options)`; scripts pass `retrieval: "hybrid"` or `"semantic"` with `text`, `k` (candidates per branch, default 50), `weights` (`lexical` 1, `prefix` 2, `fuzzy` 0.5, `semantic` 1), `fuzzyBelow` (default 5), and `qualityBoost` (default 0). The branches are the lexical query (the text as words when the query is `match_all`), `suggest` on the title, `fuzzy` on the title only when the lexical branch matched fewer than `fuzzyBelow`, and the nearest neighbors of the text's embedding when an embedder is installed and the backend lists `semantic`. Every branch gets the trusted filter. Fusion is `sum(weight / (60 + rank))` keyed by document type and id, then `qualityBoost` times the `quality` role's value (read as 0 to 1), ties by document type then id. The hybrid total is an exact count of the union of the non-semantic branches (`totalRelation: "eq"`), so a hit found only by meaning can be on the page and not in the total; a semantic-only search reports its candidate count (`totalRelation: "candidates"`). Highlights come from the lexical query.

`<semantic enabled="true" dimensions=".." representation="halfvec|vector" max-chars="2000" embedder="hash"/>` under `search-facade` turns on embedding storage when `CREATE EXTENSION vector` succeeds: `embedding`, `embedding_model`, and `embedding_indexed_at` columns and a partial HNSW index (`<representation>_cosine_ops`, `WHERE embedding IS NOT NULL`), part of the registry signature. `org.moqui.search.SearchServices.embed#PendingDocuments` (`ec.search.embedPending`) embeds documents with no embedding of the model, or one older than the stored body. The text is the title, subtitle, code, and category roles, then the document's other strings, up to `max-chars`. It stores with `WHERE indexed_at = <the value it read>`, so a stale vector never replaces a newer body's. An application schedules it; framework seed has no job, because the index to embed is the application's. No model is built in: `Embedder` is the trait, `SearchFacade::set_embedder` installs one, and `embedder="hash"` installs `HashEmbedder`, a word hash for development with no semantic quality. OpenSearch has no semantic search here and does not list it.

Fetch large business details or perform entity-specific relational joins only after authorized top-K, using the original entity keys. Exact totals and true facets require counting/aggregating the full authorized matching population, not only the returned page or bounded RRF candidates. Define that population explicitly for fuzzy/semantic/hybrid modes before claiming exact counts.

### REST service and UI

Expose a dedicated authenticated application service, for example `POST /api/search`. This is a proposed endpoint, not an existing Mantle endpoint. Its adapter always calls `SearchFacade` and maps entity types to DataDocument targets, filters to trusted query clauses, facets to aggregations, and display fields/highlights to results.

```json
{
  "q": "industrial water pump",
  "entityType": null,
  "filters": {"status": "Active"},
  "mode": "lexical",
  "offset": 0,
  "limit": 20,
  "facets": ["entityType", "category"],
  "highlight": true
}
```

`lexical` avoids embedding inference; `autocomplete` uses the prefix/suggestion path. `semantic` and `hybrid` are exposed only when their facade capabilities are implemented and enabled. Quoted input requests a phrase. An explicit proximity request such as `"near": {"terms": ["water", "pump"], "distance": 3}` also requires the proximity capability. Empty input is handled as an authorized browse operation.

The response includes the query, exact total (`totalRelation: "eq"`), results, and authorized facet counts. Each result includes entity type/key, title/subtitle, optional snippet, score, authorized URL, category, and attributes. It uses the facade's offset/limit paging and cap; cursor pagination stays deferred. Do not report page-only facets or approximate candidate counts as full result-set counts.

Call this endpoint on submission and on debounced autocomplete requests. Render each result with its entity type's display template and navigate to its authorized business URL. All types and enabled modes share one input.

## Postgres

Schema `search`, so the tables sit beside entity tables whenever both stores are the same Postgres, including the default embedded cluster and an external entity Postgres. One table per DataDocument. The table name is the physical document name (`mantle_party`). When search is its own cluster, the schema is still `search`.

Each table has:

- `doc_id text` primary key
- `indexed_at timestamptz`
- `body jsonb`, the full nested document, control fields removed, timestamps as ISO-8601 UTC and dates as `yyyy-MM-dd`
- a typed column `f_<name>` for every root field: `text` for keyword and text, `bigint`, `numeric` (exact, where the Elastic mapping was `double`), `timestamptz`, `boolean`. The indexer computes these from the body; they are not generated columns, because the date conversion must not depend on the session zone.
- `all_text`, a generated `jsonb_to_tsvector('simple', body, '["string","numeric"]')` with a GIN index, for bare terms
- per text column, a GIN index on `to_tsvector('simple', column)` and a `gin_trgm_ops` GIN index so a `Contains` clause compiles to index-supported `ILIKE` instead of a scan. Schema init runs `CREATE EXTENSION IF NOT EXISTS pg_trgm` before that index, and leaves the trigram indexes out when the extension cannot be created. Keyword, number, and date columns have a btree index.

The text configuration is `simple`: lower-cased tokens, no stemming, no stop words. That is the closest Postgres has to Elastic's `standard` analyzer, which the current mappings use (no analyzer is set), so a prefix or token match finds what it found before. A per-field configuration waits on a corpus that needs one. The trigram index cost is measured on the first corpus, and a document field can drop it when measurements say so.

`search.document_index` records each document's table and a signature of the stored shape. `ensure_index` creates the table, or drops and recreates it when the signature changed, and logs that the feed must be rebuilt. DDL runs on a reserve connection, autocommit. Reads run on the caller's connection, inside its transaction when one is open. Writes join the caller's transaction or begin their own.

A `DataDocumentField.fieldType` is a search mapping type name (`keyword`, `text`, `long`, `double`, `float`, `date`), not a dictionary type, as in the Mantle documents. Without one the kind comes from the entity field's dictionary type (the `esTypeMap` table), and an expression field is `double`, as `makePropertyMap` has it.

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

`DateHistogram` is `(date_trunc(unit, (col AT TIME ZONE tz) - offset) + offset) AT TIME ZONE tz`. Postgres weeks start Monday, as Elastic's do, so the offset alone moves the week start; `date_bin` is not used, because it cannot step by month. Each bucket level is one `GROUP BY` statement over the keys of the bucket aggregations above it. Metrics and filter counts in that level are columns, a `Filter` adding `FILTER (WHERE ..)`. A sum of a child number is a lateral per-row sum. `Terms` ordering and size, and the empty date buckets, are applied in Rust to the grouped rows. Index-time expressions are stored values. They are not re-evaluated in SQL.

Dates and timestamps in `body` are ISO-8601. Range comparisons in the API take timestamps. The current client serializes timestamps with a custom Jackson serializer because ElasticSearch rejects a negative epoch millis. Postgres stores a `timestamptz` and does not need that workaround. The OpenSearch compiler formats dates in the mapping's accepted formats.

Text rank is `ts_rank` of `all_text` against the query's positive term, prefix, and phrase clauses. Filters are SQL `WHERE` and do not affect rank.

When the entity datasource is Postgres, embedded or external, the default is the `search` schema on that database. When the entity datasource is MySQL or another non-Postgres, the default is a separate embedded Postgres cluster, including on Windows, where that cluster connects over TCP to `127.0.0.1` on an ephemeral port, as `plans/EmbeddedPostgres.md` specifies. A `<postgres>` URL points search at a different Postgres. Placement and how many postmasters start are `plans/EmbeddedPostgres.md`. All Postgres traffic, including search SQL, goes through the synchronous drivers in `plans/Crate-Layout.md`. The default binary includes embedded Postgres for the embedded search cluster. A custom build may omit it only when search is an external Postgres or OpenSearch.

### Optional analysis and vector indexes

Keep dependencies minimal: native PostgreSQL full-text/JSON operators, bundled `pg_trgm`, optional `unaccent` for accent folding, and optional community `pgvector` for semantic retrieval. No custom BM25 implementation or separate search engine is needed for the PostgreSQL path; the optional OpenSearch backend is unchanged.

Configure accent folding with a custom text-search configuration placing `unaccent` before the `simple` (or, later, stemming) dictionary; installing the extension alone does not activate it. Later multilingual support must store the selected language/configuration with the document metadata and use matching index/query analysis. Configured dictionaries or `ts_rewrite` can handle synonyms. Do not change the default `simple` behavior without corpus tests.

When semantic search is enabled, add embedding storage and indexes to the relevant per-DataDocument store through `ensure_index`, not another universal table. One candidate representation is `halfvec(1024)` with a partial HNSW index using `halfvec_cosine_ops` where the embedding is non-null; model choice determines the actual dimensions and representation. Omit the vector extension, storage, index, and worker when disabled. Provision compatible extensions for embedded and external databases before enabling the feature; do not assume `pgvector` is bundled.

The embedding worker runs locally outside PostgreSQL. It writes derived embeddings through the selected search backend's indexing path and must not overwrite a newer document with an embedding computed from stale content. Measure filtered exact-vector scans against ANN: selective authorization and attribute predicates can substantially affect HNSW recall and execution. Typed filter columns and measured JSONB containment indexes serve attributes; nested `Child` semantics remain unchanged.

### Read replicas and freshness

The default single-node PostgreSQL deployment runs business data, document maintenance, and search reads together when both stores share the database. Search reads may move to a physical replica of the configured search database without changing document schemas or the facade/application API. Use a separate read-only pool; keep the existing PostgreSQL URL as the writable search primary. The replica is not a second active backend or a dual-indexing destination.

| Operation | Single node | Physical-replica deployment |
|---|---|---|
| Document assembly/indexing and embeddings | Search primary | Search primary |
| Index creation and schema maintenance | Search primary | Search primary |
| Search and aggregation execution | Search primary | Search replica |
| Search data | Per-DataDocument tables | Same tables replicated through WAL |
| Freshness | After committed indexing | Indexing delay plus replication lag |

`CREATE INDEX`, projection/embedding writes, and `REFRESH MATERIALIZED VIEW` (including `CONCURRENTLY`) run on the primary. A physical replica cannot perform them. Materialized views are optional, benchmark-driven read models, not a search prerequisite; refresh adds primary cost and freshness delay. Where entity and search stores share PostgreSQL, business data and permission relations replicate with the search tables. A separate search database must not assume those relations are available locally.

Validate permissions against current authoritative state before returning hits, totals, facets, or snippets; stale replica ACLs alone are insufficient, especially after revocation. For read-your-writes, first wait for the relevant pending-feed indexing to complete, then wait for its search-commit WAL position on the replica with a bounded timeout or route to the search primary. Routing to the primary alone does not remove indexing delay. Permission-sensitive requests require equivalent authoritative checks on either route.

In the code, `PostgresBackend::with_read_replica` takes the replica's pool. Searches, aggregations, and `get_document` run there. Index and schema writes, the `document_index` registry, and any read inside a transaction already open on the primary stay on the primary. A request asks for read-your-writes with `SearchRequest::fresh_by`, a deadline; scripts pass `consistency: "readYourWrites"` and `freshTimeout` (milliseconds, default 5000). `schedule_feed_receive` remembers the pending row ids a session's transactions inserted (`search::fresh`, the newest 200). A fresh read first waits for those rows to be delivered. It then reads `pg_current_wal_lsn()` on the primary, which covers every commit it waited for, and polls the replica's `pg_last_wal_replay_lsn()` until it reaches that position. If the deadline passes first, or the replica fails, the read runs on the primary and a counter goes up. A rows-still-pending timeout still runs the search; the primary has whatever is indexed. Replica sessions are `default_transaction_read_only` with `statement_timeout` (`read-statement-timeout`, 30 seconds). `/metrics` reports `search_replica_lag_seconds` (`now() - pg_last_xact_replay_timestamp()`, -1 when the replica cannot be read) and `search_replica_primary_fallbacks`.

Set query timeouts and monitor replication lag and standby query cancellations. Expensive search queries must not silently block recovery. A separately writable derived database is the existing separate-PostgreSQL placement, not a physical replica; moving maintenance there needs explicit feed consistency, authorization, and recovery rules. A replica offloads reads only and does not satisfy a topology that requires derived-index maintenance off the business write node.

### Validation

Benchmark representative Mantle corpora, broad and selective lexical queries, typo recovery, phrase/proximity, autocomplete, configured languages, and enabled vector/hybrid modes. Record relevance, p95 latency, primary maintenance cost, replication lag, and end-to-end freshness. Test authorization before ranking and aggregation, revocations during replica lag, pending-feed failures/recovery, read-after-write timeouts, and exact totals/facets independently of top-K. PostgreSQL stores and retrieves derived documents; Mantle owns business semantics and links; any optional local model only generates embeddings.

## OpenSearch

The compiler emits the JSON the current services emit.

- `Text` becomes a `query_string` (or a `multi_match` when the clause is only terms). It sits in `bool.must`, so it scores.
- `Field`, `Child`, and non-text `Bool` arms sit in `bool.filter`.
- `Child` becomes a `nested` query with the path and the inner query.
- `DateHistogram`, `Terms`, `Sum`, `CountDistinct`, and `Filter` become `date_histogram`, `terms`, `sum`, `cardinality`, and `filter`. `DateHistogram` emits `calendar_interval`, the ElasticSearch 7+ and OpenSearch key. The pre-7 `interval` name stays dropped. Postgres uses `date_trunc` and `date_bin`, above.
- The page total is exact. The compiler sends `track_total_hits: true`. The limit cap of 10000 is unchanged.

Mappings follow `makeElasticSearchMapping`. Many-relationships stay `type: nested` with `include_in_root: true`, so dashboards that already query dotted names keep working. Keyword subfields stay for sortable text. Document types, the pre-7 interval name, and the old point-in-time paths do not.

`CountDistinct` may send `precision_threshold` inside the HTTP body. The value returned to the caller is the integer from the response.

Each stored document carries `_indexed_at` (epoch millis), which a retention `delete_matching` on `@timestamp` filters, and which `get_document` and hits leave out. The mapping's `_meta` carries the shape signature, so `ensure_index` rebuilds an index whose definition changed, and an index with an `indexName` gets that alias. Highlights are made from `_source` by the same Rust code as on Postgres. The compiler is in core (`search::opensearch`), and the same parity test runs against both backends; the OpenSearch run is ignored unless `OPENSEARCH_URL` is set.

## Config

`search-facade` replaces `elastic-facade`.

```xml
<search-facade backend="${search_backend}"/>   <!-- default property search_backend is postgres -->

<search-facade backend="postgres"/>

<search-facade backend="postgres">
    <postgres url="..." user="..." password="..."/>
</search-facade>

<!-- Optional read replica; url remains the writable search primary. -->
<search-facade backend="postgres">
    <postgres url="..." user="..." password="..." read-url="..."/>
</search-facade>

<search-facade backend="opensearch">
    <cluster url="${opensearch_url}" user="${opensearch_user}" password="${opensearch_password}"
             index-prefix="${opensearch_index_prefix}"/>
</search-facade>
```

`opensearch_run`, `opensearch_start_wait`, and `moqui_log_level` are default properties; Dashboards and logs describes them.

`backend` defaults to `postgres`. With no `postgres` element, tables live in the `search` schema of the entity Postgres, embedded or external. When the entity datasource is not Postgres, that default is a dedicated embedded cluster. With `backend="opensearch"` the cluster element is required and the Postgres search store is not opened.

A `postgres` url uses TLS unless it carries `?sslmode=disable`, the same rule as the entity datasource's `sslmode`. A host that is an absolute path is a Unix socket directory.

The optional `read-url` selects a physical replica of the search primary for reads only; it uses the configured credentials (the `postgres` element's `user` and `password`) and a separate read-only pool. `read-statement-timeout` is seconds, default 30. It never receives index/schema writes. Without it, reads use the primary. Primary fallback and bounded freshness waits follow Read replicas and freshness above.

Service parameters named `clusterName` go away. There is one backend. The `logger` cluster name goes away with the logging entity group.

## Dashboards and logs

OpenSearch Dashboards, the `/elastic` and `/kibana` proxies, and the `ElasticRemote` and `KibanaRemote` permissions stay. They do nothing when the backend is Postgres (the proxies answer 404 through `SearchBackend::has_elastic_api`), because there is no cluster to proxy. People who build visualizations on the Mantle indexes keep doing that when they choose the OpenSearch backend. The documents they see are the same documents the facade indexes. The facade is not a report designer, and Postgres is not given a dashboard server.

Application logs and HTTP request logs are flat indexes, `moqui_logs` and `moqui_http_log`, written by the host. They are not DataDocuments. Both backends keep them: the host ships to whichever backend `search_backend` selects, through `SearchFacade::index_log_entries`, so switching the backend moves the stream with no host change. On Postgres each log index is a table in the `search` schema like a document, with ids made on arrival (13 hex digits of epoch millis, then 12 random) and the event time in the `@timestamp` column; the log viewer and the two cleanup jobs run on both. `Crate-Layout.md` already places the log shipper on the host, beside the OpenSearch process.

On OpenSearch the facade treats each log index as a flat document type named for the index (`search::logs`), with the fields of `ElasticSearchLogger` and `ElasticRequestLogFilter` and the event time in its own `@timestamp`. So `search`, `delete_matching` retention, and `get_document` work as they do for documents, and `Index("moqui_logs")` names it. The log viewer's string is a `Query::Lucene`, sent as written on OpenSearch and read with the query-string grammar on Postgres. The host installs a `tracing` subscriber (lines on stderr at `moqui_log_level`, and in `runtime/log/moqui.log` and `moqui-error.log`, which roll over by size and day; see `moqui_log_dir`, `moqui_log_max_size_mb`, `moqui_log_max_files`). Events are queued from the start, so the boot lines ship once search attaches, and a shipper thread bulk-indexes them every two seconds without waiting for a refresh. A data load and a server both ship, and each sends what is left before it stops. As in Java, DEBUG and TRACE are not shipped, a full queue (16384) drops new entries, and the shipper's own events are not queued. Screen and REST requests go to `moqui_http_log`; proxied requests do not, as Java's filter covered only the webapp.

`/elastic/*` proxies to `elasticsearch_url` with the prefix removed, and `/kibana/*` to `http://${kibana_host}:${kibana_port}` with the prefix kept (Dashboards sets `server.basePath: "/kibana"`). Each needs a login (session, Basic, or `api_key`) and the `ElasticRemote` or `KibanaRemote` permission: 401 with a Basic challenge, else 403, as `MoquiAuthFilter` answers. The client's `Authorization` and `Cookie` headers are not forwarded; `elasticsearch_user` and `elasticsearch_password` are. On Postgres both paths are 404, as the backend has no Elastic API.

When the opensearch backend's cluster is this host and nothing answers, the search store starts `runtime/opensearch` (`bin/opensearch` in the foreground with its bundled JDK, as `su` to an owning `opensearch` user when that works) and waits up to `opensearch_start_wait` seconds. Dropping the store sends SIGTERM and then kills it. A `pid` file means the node is run outside the server (`bin/opensearch -d -p pid`), and it is left alone. `opensearch_run=false` never starts one.

## Caller migration

| Caller | Becomes |
|---|---|
| `SearchServices.search#DataDocuments` | `SearchFacade.search`. A query string is still accepted and parsed, `nestedQueryMap` Elastic JSON is read as `Child` queries, and a `query` map is AND-ed. The body is JavaScript on `ec.search`. |
| `index#WikiSpacePages` | Same service, `ec.entity.getDataDocuments` and `index#DataDocuments`. |
| `index#DataDocuments`, `index#DataFeedDocuments`, `delete#DataDocument`, `delete#Documents` | `index_documents`, the feed rebuild, `delete_document`, `delete_matching` |
| `put#DataDocumentMappings`, `delete#ElasticIndex` | `ensure_index`. Dropping a document store is an admin method on the backend. |
| `search#MantleFiltered` | Same service. The org limit is a `Field` `In` list. |
| Party, product, task, request, project, HiveMind, PopCommerce, PopRestStore | `Text`, `Field`, and `Child`. Auto-wildcard text stays `Text`. |
| The four `SalesReportServices` | `aggregate`. Screen fields unchanged. |
| `get#InventoryProjectedInfo` | Mantle rewrite onto `aggregate`. It calls the missing `ElasticSearchUtil`, and the inventory body has nothing to port. The rewrite happens with mantle. |
| `get#PriceHistoryInfo` | Left unimplemented. It calls the missing `ElasticSearchUtil` and has no callers. |
| Tools DataDocument Search, ViewDocument | Parsed query string. `get_document`. |
| Log viewer, `ElasticSearchLogger`, `ElasticRequestLogFilter`, cleanup jobs | Both backends, through `index_log_entries`. |
| `ElasticDatasourceFactory`, `TestNoSqlEntity` | Not ported. |
| `ec.elastic` / `getElastic()` | `ec.search`: `search`, `aggregate`, `parseQuery`, `elasticQuery`, `indexDocuments`, `deleteDocument`, `deleteMatching`, `getDocument`, `ensureIndex`, `dropIndex`, `indexExists`, `documentTypes`, `isAvailable`, `backendName`, `flatten`, `processFeedPending`, `rebuildFeed`. Each search call takes one options map with `documentType` or `indexName`. |

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
