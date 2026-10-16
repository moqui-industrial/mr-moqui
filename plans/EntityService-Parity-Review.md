# Entity and Service Parity Review

Scope: Rust modules in `crates/moqui-core/src/entity/` and `crates/moqui-core/src/service/` (with the
`moqui-db` SQL/model layer they delegate to), compared against the Moqui Java / Groovy / XML code they
were ported from.

Revalidated 2026-10-16 against `d9d5f9f`. Findings fixed by 956954e, 48e9d36, 62a3def and 2ce6ecd, and
findings that proved invalid or duplicated, have been removed; the IDs that remain keep their original
numbers, so gaps in the numbering are expected. Deliberate, documented differences are collected in
[Accepted differences](#accepted-differences) rather than the phase tables.

Paths: Rust locations are relative to `crates/moqui-core/src/` unless they start with `moqui-db/`,
`moqui-lib/` or `moqui-host/` (siblings under `crates/`), or `runtime/`. Java locations are file names
under `moqui/framework/src/main/{groovy,java}/org/moqui/impl/` or framework service XML. Line numbers
are as of the revalidation commit.

Severity: **High** = wrong result, data loss, authorization gap, or crash; **Med** = missing branch,
changed default, or edge-case divergence; **Low** = cosmetic, message text, or provably equivalent
ordering.

## Summary

97 open findings and 32 accepted differences. Open by phase:

| Phase | Open | High |
|---|---|---|
| 1 Entity value / field / condition | 2 | — |
| 2 Entity find / query | 0 | — |
| 3 Facade / metadata / DDL / cache | 3 | — |
| 4 Data loader / feed / ECA / writer / document / ETL | 18 | P4a-01, P4a-02 |
| 5 Entity query / REST | 5 | P5-01 |
| 6 Service definition / validation / call | 20 | P6b-01 (decision) |
| 7 ECA / SECA / pools / jobs | 19 | — |
| 8 REST / remote / tools / mail / components | 30 | P8a-03 |

| ID | Area | One-line |
|---|---|---|
| P4a-01 | data loader | CSV BLOB stored as Base64 text, not decoded bytes |
| P4a-02 | data loader | CSV rows use entity-auto `store` (auto key/status) vs Java `createOrUpdate` |
| P5-01 | entity REST | REST find of a **view-entity** returns only `_entity` (no alias values) |
| P6b-01 | service call | Service-body failure returns `Err`; Java returns the result map — **open design decision** |
| P8a-03 | service REST | `/rest/s1` service results not run through `stripUserAccountSecrets` |

### Cross-cutting threads

1. **Access control** (P8a-03, P7b-07, P7b-08, P7a-05): service-REST results are not stripped of
   UserAccount secrets; async calls skip the pre-queue authz check and then run with authz enabled;
   the permission check omits ALL_USERS group permissions.
2. **Loader contract** (P4a-01, P4a-02, P4a-05, P4a-08, P4a-09, P4a-11): CSV typing, the CSV write
   path, empty/short cells and bad-type handling diverge from Java's accept/reject rules.
3. **Full-text search and model sources** (P3-20, P3-14, P3-21): a `LIKE` on a `text-fts` field is not rewritten
   to a full-text search (the DDL now follows `FtsDdl`), DB view entities are not built from their rows, and a
   foreign key is recognized only by its computed name.
4. **Error contract** (P6b-01, P7a-10): a failing service body propagates `Err` where Java records the
   error and returns; `post-commit` SECA gets `None` instead of an empty map.
5. **View REST output** (P5-01): view finds now filter on aliases but still render no alias values.

### Open decisions

- **P6b-01** — keep Rust's `Err` on a service-body failure (the Rust tests assert it) as a documented
  redesign, or switch to Java's record-and-return contract. Everything that calls `sync(...).call()`
  depends on the answer.
- **Spreadsheet/print references**: `org.moqui.impl.SpreadsheetServices` and some component Groovy are
  absent from these checkouts, so those comparisons rely on the XML descriptor only.

### Stale code comments (small to-dos)

- `entity/load_feed/loader.rs:435` says the loader uses the entity-auto store path; true for XML/JSON
  but not CSV (Java CSV uses `createOrUpdate`), which hides P4a-02.

---

## Phase 1 — Entity value / field / condition core

Compared `entity/{value,field_rw,crypto,fk}.rs` (plus `moqui-db/src/sql/condition.rs`) against
`EntityValueBase.java`, `EntityValueImpl.java`, `EntityList(Impl).java`, `FieldInfo.java`,
`EntityJavaUtil.java` and `condition/*`.

Worked 2026-10-16: P1-02, P1-03, P1-06 and P1-10 fixed; P1-07 and P1-13 moved to
[Accepted differences](#accepted-differences); P1-14 and P1-15 found along the way. Line numbers in this
section are as of that change.

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P1-14 | `entity/value.rs:750` | `EntityValueImpl.java:142-143` | An update that matches no row returns `Ok`; Java's `updateExtended` throws "Tried to update a value that does not exist" | Med | No |
| P1-15 | `entity/value.rs:464-534` | `EntityValueBase.java:1324-1343`, `1654-1658` | A `lastUpdatedStamp` the caller sets on a found value is replaced unless the update reads the row; Java keeps it, since `put` records the old value in `dbValueMap` | Low | No |

#### P1-14 — Update of a missing row is silent

`finish_write` takes the affected-row count from `execute_tx` and only the create-or-update probe looks at
it. Java throws when `executeUpdate()` returns 0, so `value.update()` and the entity-auto `update#` of a
row that is not there fail in Java and do nothing in Rust. An entity whose update reads the row first
(see the equivalences below) returns early in both, since Java then compares the value with itself.
Making the zero count an error needs a pass over the Rust callers that update rows that may be gone.

#### P1-15 — Caller-set stamp without a read

Java's `putKnownField` keeps the old value of a field changed on a value read from the database, and
`update` counts `lastUpdatedStamp` as modified, so keeps the caller's stamp, when that old value is
non-null and differs. Rust's `EntityValue` does not track what it read, so it can tell only against a row
the update reads (audit-logged, create-only or defaulted fields, optimistic locking, or a data feed). It
matters only to a caller that sets the stamp on purpose on an entity with none of those.

#### Phase 1 checked and found equivalent

- `update` (P1-02, fixed). Java reads the stored row first only for `needsAuditLog`, `createOnlyAny`,
  `optimisticLock`, `hasFieldDefaults` or a data feed document; then it merges the stored fields into the
  value, writes only those that differ, and returns after the run-before EECA rules when none do. With no
  read it writes every field the value holds (`isFieldModified` is true for any field `dbValueMap` lacks),
  as Rust always did. Rust now reads the row in the same cases (`update_reads_row` plus
  `feed_wants_old`; the create-or-update probe is skipped for them), reuses the row create-or-update just
  read, and writes, skips and refuses as Java: a stamp alone is no update, a changed create-only field is
  an error, and optimistic locking refuses a stale stamp. Only the changed fields and the key are sealed,
  so an unchanged field that failed to decrypt is not written back.
- `lastUpdatedStamp` (P1-03, fixed): the transaction start time (`TransactionManager::start_time_millis`),
  else the wall clock; on create only when unset; on update kept when it differs from the stored one (see
  P1-15 for the case with no read).
- Field defaults (P1-06, fixed): any expression runs in the script runtime with the stored fields and then
  the value's fields in the context, as Java pushes `dbValueMap` and the value map; quoted literals,
  numbers, field names and `ec.user.nowTimestamp`/`userId` skip the script. `""` is empty; an unknown name
  is null, as `ContextBinding`. Primary-key defaults are filled before an update's read and the others
  after it. The one Groovy-only default in the runtime (`DataDocumentField.sequenceNum`) is converted, and
  `moqui convert --groovy` now treats an entity field's `default` as an expression.
- Audit rows (P1-10, fixed) go through `create#moqui.entity.EntityAuditLog` with authorization off when the
  context's facade is the one writing, so that service's SECA rules run; with no context they are created
  directly. An update's audit compares with the row it already read.
- Field encryption: constant IV, salt padding, iteration default, algorithm, base64-vs-hex decode,
  `decrypt-alt` retry — test vectors pass. Decrypt failure uses `_DECRYPT_FAILED_` and refuses to
  re-encrypt it (`entity/field_rw.rs:116-121,153-157`).
- Localization: localized text is cached per locale and never written back to the stored field;
  single-PK only and `"default"` locale fallback, as Java.
- FK existence check only under `dummy_fks` (loader), as Java (`entity/fk.rs:15-17`).
- Audit: field selection for `true`/`update`, `{field}_changeReason`, `artifactStack`.
- Null forms of equal/not-equal/like/between → `IS (NOT) NULL`; empty `IN`/`NOT IN` → constant
  false/true; `ignore-case` upper-casing keyed off a String column.
- `order-by` parsing (`OrderBy::parse`) incl. `NULLS FIRST/LAST`, `DESC/ASC`, `UPPER()/LOWER()`,
  `never-nulls`; `create_or_update` applies PK defaults before the existence check.

---

## Phase 2 — Entity find / query

Compared `entity/{find,search_form,schema}.rs` (with `moqui-db/src/sql/` and `screen/aggregate.rs`)
against `EntityFindBase.groovy`, `EntityFindImpl.java`, `EntityFindBuilder.java`,
`EntityQueryBuilder.java`, `AggregationUtil.java`, `FtsDdl.java`, `FtsSql.java` and
`RestSchemaUtil.groovy`.

Worked 2026-10-16: P2-02, P2-03, P2-04, P2-06, P2-07 and P2-11 fixed; P2-08 moved to
[Accepted differences](#accepted-differences); P2-13 and P2-15 found and fixed, and P2-14 found and accepted,
along the way. No Phase 2 finding is open. Line numbers in this section are as of that change.

#### Phase 2 checked and found equivalent

- Repeated equality (P2-03, fixed): `eq` and `condition` keep one equality (or `IS NULL`) per plain field, as
  Java's `simpleAndMap`, so a second one for the field replaces the first in place, and an `AND` is unrolled so
  its equalities do the same (`add_condition`, `entity/find.rs:723`). A search-form condition goes the same way,
  so a form value replaces the find's own equality on that field, as in Java.
- Search-form cache (P2-02, fixed): `search_form_map` sets `useCache(false)` when the find did not choose, and a
  find that asked for the cache (Java's `shouldCache`) is not paged (`entity/search_form.rs:66-69`, `:98`).
- Search-form `_ic` (P2-04, fixed) applies to `equals`/`like`/`contains`/`begins` only, not `in`.
- Search-form `_not` (P2-13, fixed): on `equals` and `in` it also finds the rows with no value, Java's `orNull`
  (`or_null`, `entity/search_form.rs:314`); `like`/`contains`/`begins` stay a plain `NOT LIKE`. Before, Rust
  left out the null rows.
- `pageNoLimit` (P2-15, fixed) removes paging only for `true` or the text `"true"`, as Java; Rust took any
  non-empty value but `false`, so `Y` removed paging.
- JSON schema relationships (P2-06, fixed): an entity's dependent relationships, or a master's details (with
  `use-master`), are properties named by short alias or relationship name with a `$ref` to the related schema,
  an array for `many`; the related schemas go in `$defs`, nested as Java nests them, and a reference back to a
  standalone document's own entity is `#` (`entity/schema.rs:129-243`). The catalog also holds any view entity
  a relationship refers to, so every `$ref` resolves; Java refers to a definition it never writes.
- JSON schema `enum` (P2-07, fixed): the first non-auto-reverse `one` relationship keyed by the field alone,
  titled and to `Enumeration` or `StatusItem`, lists that type's ids by `sequenceNum` (`field_enums`,
  `entity/schema.rs:248`); a read that fails is logged and the field has no `enum`.
- Distinct count (P2-11, fixed): with no field selected it counts `SELECT DISTINCT *` in a sub-select, as Java
  wraps all fields (`moqui-db/src/sql/statement.rs:262`).
- `one()` with no condition or an empty PK returns `None` without querying (`entity/find.rs:368-406`).
- `select_field` of an unknown field is skipped with a warning (`entity/find.rs:622-634`); its doc comment now
  says so.
- Search-form operators (`equals`/`like`/`contains`/`begins`/`empty`/`in`, `_from`/`_thru`, `_period`),
  date-range widening, `_period`/`getPeriodRange` windows, unknown op → no condition.
- Pagination defaults (page size 20, offset `index*size`, `pageNoLimit`, negative clamp); paged and
  no-condition finds are not cached.
- `narrow_to_full_pk`, `shouldCache` core rules, `FOR UPDATE` requiring a transaction.
- Order-by (`ORDER BY 1` when paging, `NULLS LAST` unless `never-nulls`, `UPPER`/`LOWER`), distinct
  order/select inclusion.
- View query building (member trimming, join paths, `entity-condition`, group-by/having,
  `COUNT(*) FROM (…)`) and aggregation (`min`/`max`/`sum`/`avg`/`count`/`first`/`last`, total row).
- Full-text conditions (`FtsSql`); find authorization push and find filters (fail-closed on a missing
  view alias); `requireSearchFormParameters`.

**Not ported (scope, not findings):** `updateAll`/`deleteAll`/`extract`/`listMaster`/`oneMaster` and
JDBC-only options (`fetchSize`, `maxRows`, `resultSetType/Concurrency`, cursor `offset-style`).

---

## Phase 3 — Facade / metadata / sequence / datasource / cache

Reviewed the entity facade, model/DDL metadata, sequenced-id, datasource, tools-info, cache and
dbresource modules. Physical naming, DDL text and the model loader live in `moqui-db`; those findings
cite their `moqui-db` location.

Worked 2026-10-16: P3-01, P3-02, P3-07, P3-09, P3-10, P3-12, P3-13, P3-16, P3-17, P3-18, P3-19, P3b-03,
P3b-04, P3b-07, P3b-08 and P3b-09 fixed; P3-06 and P3-11 moved to [Accepted differences](#accepted-differences);
P3-08 removed as invalid (H2, SQL Server and Oracle have no driver in this port and `datasource_conf` refuses
them, so no full-text DDL can run for those styles); P3-14 left open; P3-22 found and fixed, and P3-20 and P3-21
found and left open, along the way. Line numbers in this section are as of that change.

### Findings — 3a facade / metadata / sequence / datasource

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P3-14 | `entity/share.rs` (immutable model); `runtime/base-component/tools/screen/Tools/DataView/ViewDbView.xml:62` | `EntityFacadeImpl.groovy:529-555,675-727` | DB view entities (`DbViewEntity` rows) are never made into view-entities; the DataView screen says so | Med | Unsure |
| P3-20 | `moqui-db/src/sql/condition.rs:811` (`matches` only) | `FtsSql.java:81-123`; `FieldValueCondition.java:73` | A `LIKE`/`NOT LIKE` on a `text-fts` field stays a SQL `LIKE`; Java rewrites it to the full-text predicate plus a `LOWER(col) LIKE` per word, and falls back to `LIKE` when the group's full-text setup failed | Med | No |
| P3-21 | `entity/meta.rs:733` | `EntityDbMeta.groovy:881`, `1047`, `1132` | A foreign key counts as there only by its computed name; Java matches the key columns, so a key Java made under another name gets a second one | Low | No |

No 3b (cache / dbresource) finding is open.

#### P3-14 — DB view entities not loaded

Java looks a `DbViewEntity` name up in the database when the definition is first asked for, builds a
`view-entity` node from its `DbViewEntityMember`, `DbViewEntityKeyMap` and `DbViewEntityAlias` rows, and caches
it until the tools screens clear it. Rust's model is loaded once from files and is immutable (`SharedModel`);
every lookup (`with_entity`, `with_model`, the SQL builders) borrows from it. Supporting these needs a second,
mutable set of definitions behind a lock that every name lookup consults after the file model (or a model that
can be swapped), a build from the rows through `EntityModel::complete_view` that reads them without recursing
into the lookup, and invalidation when those rows change (the DataView edit screens and the data loader). That
reaches every model lookup site, so it is left for a change of its own; until then the DataView screen tells the
user the view is not available (`ViewDbView.xml:62`).

#### P3-20 — `LIKE` on a `text-fts` field is not a full-text search

Java's `FieldValueCondition` hands a `LIKE`/`NOT LIKE` on a `text-fts` field to `FtsSql.appendIfRewritten`,
which (unless the pattern is a prefix-only one, the words are all shorter than three letters, a hold overlay is
active, or the group's FTS is disabled) writes the `fts-style` predicate (`{col}_tsv @@ plainto_tsquery(...)`,
`MATCH ... AGAINST`, ...) and a `LOWER(col) LIKE ?` filter per word. Rust renders every `LIKE` as SQL `LIKE`;
the full-text predicate is reachable only through `EntityFind::matches`, which no component calls. The
Phase 2 note on `FtsSql` covered only that predicate's text. Rust also has no `FtsSql.disable`: when the
`_tsv` column could not be added (P3-07), `matches` on Postgres fails rather than falling back to `LIKE`.

#### P3-21 — Foreign key existence by name

`ensure_fks` skips a foreign key when the schema has a constraint of the name `foreign_key_name` gives. Java's
`foreignKeyExists` reads the imported keys and matches the key columns. A database whose keys Java created under
another name (MySQL's `fk-style="name_fk"` on MySQL 8.0.16 and later names them `{table}_ibfk_N`, see P3-11)
gets a second, identically keyed constraint from the first Rust check. Matching by key columns needs a catalog
read of the foreign key columns per group, like `existing_constraints`.

#### Phase 3 checked and found equivalent

- Global `sequenced-id-prefix` (P3-01, fixed): read once with the conf (`sequenced_id_prefix`,
  `entity/facade.rs:601`) and put in front of every id a bank issues, for an entity (inside its own
  `sequence-primary-prefix`) and for `sequencedIdPrimary`; not on a UUID or a sim id, as Java
  (`entity/sequence.rs:211`, `:257`).
- Entity `sequence-primary-prefix` (P3-22, found and fixed): `${}` runs with the value's fields in the context,
  as Java's `setSequencedIdPrimary` expands it (`expand_prefix`, `entity/sequence.rs:216`). Before, the text was
  used as written.
- Datasource `sequence-primary-use-uuid` (P3-16, fixed): the entity's attribute or its group's datasource (the
  default group's when it has none) gives a UUID, for `setSequencedIdPrimary` and `sequencedIdPrimary`
  (`datasource_uses_uuid`, `entity/facade.rs:307`).
- `needsAuditLog` in the tools info (P3-02, fixed): a field (the entity's attribute as its default) with `true`
  or `update` (`entity/info.rs:572`).
- Full-text DDL (P3-07, fixed): on Postgres the generated column `{column}_tsv` (`to_tsvector(config,
  coalesce(col, ''))`, stored) and a GIN index `fts_{table}_{column}` on it; on MySQL a `FULLTEXT` index
  `{table}_{column}_FTS`; names with characters other than letters, digits and `_` as `_`, clipped to 60, as
  `FtsDdl` (`moqui-db/src/sql/statement.rs:439-520`). `matches` on a `text-fts` field searches the `_tsv` column
  (`moqui-db/src/sql/condition.rs:817-825`); a view alias still computes `to_tsvector(...)`, which is right but
  not indexed. A plain entity with a `text-fts` field no longer reads `SELECT *`, which would carry the generated
  column. **Upgrade:** a database this port set up before has an expression index `{table}_{field}_FTS` clipped
  to 30 and no `_tsv` column. The next table check (startup, or the runtime check) adds the column and its index
  and then drops that old index (`ensure_fts`, `entity/meta.rs:667`); on MySQL a `FULLTEXT` index under the old
  name is dropped after the new one is added. On Postgres adding the stored column rewrites the table once, so a
  large `text-fts` table makes that start slow; set `fts-ddl="false"` and add the column and index by hand to
  avoid it. Checked against embedded Postgres, including the upgrade.
- `fts-ddl="false"` (P3-09, fixed): no full-text column or index, in create table or in the check, and no old
  index is dropped. The attribute is now in the runtime's `moqui-conf-3.xsd`.
- `use-fk-initially-deferred` (P3-10, fixed): `INITIALLY DEFERRED` after the references
  (`moqui-db/src/sql/statement.rs:599-601`). Only new foreign keys get it; an existing one is not altered.
- `always-use-constraint-keyword` (P3-12, fixed): `CONSTRAINT NOT NULL` for a not-null column of a create table
  (`moqui-db/src/sql/statement.rs:737-739`). No shipped profile sets it.
- `<load-entity location>` (P3-13, fixed): the conf's files are read after the framework's `entity` directory and
  before the components', as Java's `getAllEntityFileLocations`; a file already in a model directory (the
  framework's own list) is read once, and `*.eecas.xml` ones join the EECA rules (`load_entity_files`,
  `entity/facade.rs:624`; `EntityModel::load_paths`, `moqui-db/src/model/mod.rs:399`). `component://` is the
  framework or a component whose directory has that name; a location that cannot be found is logged and skipped.
- Repeated entity names (P3-17, fixed): a full name defined again is a warning naming both locations and the
  later definition is used; an entity name in more than one package is a warning at load and means the
  framework's (`moqui.`) definition, else the full name that sorts last, as Java's definition cache ends up after
  `loadAllEntityDefinitions` (`preferred`, `moqui-db/src/model/mod.rs:497`).
- `extend-entity` with no matching entity or another package (P3-18, fixed): skipped with a warning naming it
  (`moqui-db/src/model/mod.rs:586`); Java skips it silently.
- JDBC URI parameters (P3-19, fixed): those with an equivalent in this port's drivers are read (`sslmode`/
  `sslMode`, MySQL `useSSL`, Postgres `currentSchema` when there is no `schema-name`); any other is logged by
  name and left out, since the Postgres and MySQL clients here are not JDBC drivers (`jdbc_uri`,
  `entity/datasource.rs:295`).
- Sim rollback poison (P3b-03, fixed): the first sim write of a transaction adds a rollback hook that ends the
  sim, as Java's `noteWrite`/`poison`; afterwards every find and sim write fails with Java's message until the
  sim is exited (`note_sim_write`, `entity/cache.rs:211`; `sim_check_find`, `:183`). A transaction that commits,
  one that never wrote to the sim, and a write with no transaction are not tracked, and the sim's own records on
  production (`SIM_PRODUCTION_ENTITIES`) are still read. Checked against an embedded overlay.
- dbresource write across a file/dir boundary (P3b-04, fixed): the parent directories go through
  `directory_id`, so a file in the path is Java's "found file instead" error, and a directory at the location is
  an error rather than becoming a file (`write_db_resource`, `entity/dbresource.rs:120`).
- `DbResourceFile.mimeType` (P3b-07, fixed): set on the first write from the file name
  (`content_type_for_name`, `entity/dbresource.rs:366`); that list is shorter than Java's MIME map, so a rarer
  extension gets none.
- Directory children (P3b-08, fixed) are listed by `filename` (`entity/dbresource.rs:313`).
- `getNextVersions` (P3b-09, fixed): the history rows whose `previousVersionName` is the version
  (`db_resource_next_versions`, `entity/dbresource.rs:466`), and `ResourceReference.getNextVersions` in scripts.
- `<database use-indexes>`, `use-indexes-unique(-where-not-null)`, `use-foreign-keys`
  (`entity/meta.rs:609,725`, `moqui-db/src/dialect.rs:217-223`); unknown entity group falls back to
  the default group (`entity/facade.rs:536-546`).
- Entity-operation authorization (`AT_ENTITY` for create/update/delete/view, `authorize-skip`
  semantics); facade `begin`/`commit`/`rollback`/`transactions_for` per group.
- Sequenced-id bank start 100000, bank size 50, stagger, `tempSet/tempReset`, per-name locking,
  per-entity `sequence-primary-prefix`.
- `enable-localization`, `encrypt`, `not-null`, PK `CONSTRAINT` naming, constraint-name clip 30;
  `use-foreign-key-indexes`, `never-nulls`, `never-try-insert`, `use-binary-type-for-blob`,
  `table-engine`, `character-set`, `collate`, `for-update`; FK/index naming; dictionary type mapping.
- Cache: one/list keying, `entity.record.list.{entity}`, LFU no-TTL, `useCache`/`neverCache`; member
  writes evict cached views over that member, nested views included (`entity/cache.rs:584-586,705-746`, coarser
  than Java but safe); count cache; limit/offset finds not cached.
- Per-transaction hold cache: create-after-delete collapse, delete-after-create, chronological flush,
  rollback drop, last-value-wins update; `flush_and_disable`; `stopTxCache`.
- Sim: view reads answered from the overlay (`sim_view.rs`, used by list/count/iterator); production
  allow-list matches `TransactionCacheDb.java:128-131`; overlay lifecycle.
- dbresource versioning, delete, move, missing-path `exists:false`, `stat` last-modified.
- Write-hook ordering (cache clear, feed registration, audit after the DB write).

---

## Phase 4 — Data loader / feed / ECA / writer / document / ETL

Reviewed `entity/load_feed/*`, `entity/data_writer.rs`, `entity/document.rs` and `entity/etl.rs`
against `EntityDataLoaderImpl`, `EntityDataFeed`, `EntityEcaRule`, `EntityDataWriterImpl`,
`EntityDataDocument`, `SimpleEtl` and the relevant `EntityFacadeImpl`/`EntityValueBase` methods.
`pull.rs` has no Java counterpart.

### Findings — 4a data loader

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P4a-01 | `entity/load_feed/csv.rs:125`; `moqui-lib/src/object.rs:577` | `EntityDataLoaderImpl.groovy:1022-1030` | CSV BLOB cells stored as raw Base64 text; Java Base64-decodes (`typeValue==12`) before `setBytes` | High | No |
| P4a-02 | `entity/load_feed/csv.rs:127` → `entity/load_feed/loader.rs:436` | `EntityDataLoaderImpl.groovy:579,1036-1042` | CSV rows go through entity-auto `store_entity` (auto `fromDate`, secondary sequencing, status-flow check); Java CSV uses `createOrUpdate` and throws on an incomplete multi-key PK | High | No |
| P4a-04 | `entity/load_feed/json.rs:289` | `EntityDataLoaderImpl.groovy:1130`; `FieldInfo.java:235-241` | JSON BLOB: Rust Base64-decodes; Java stores raw UTF-8 bytes | Med | Unsure |
| P4a-05 | `entity/load_feed/loader.rs:537-548` | `EntityJavaUtil.java:522-536`; `FieldInfo.java:217-219` | Bad type text: Java records a validation error and the record fails (XML/JSON) or throws (CSV); Rust keeps the original string and stores it | Med | No |
| P4a-08 | `entity/load_feed/csv.rs:122-124`; `entity/load_feed/loader.rs:233-235`, `:655` | `EntityDataLoaderImpl.groovy:813-817,1016-1018` | Empty CSV cell / XML attribute skips the field; Java sets null, so an update clears the column and a `defaultValues` entry does not survive | Med | No |
| P4a-09 | `entity/load_feed/csv.rs:102`; `entity/load_feed/json.rs:120` | `EntityDataLoaderImpl.groovy:968-974,1117-1123` | CSV/JSON service name without `#` errors; Java calls any defined service (Rust XML uses `defined_name`, `entity/load_feed/loader.rs:191-192`) | Med | No |
| P4a-10 | `entity/load_feed/check.rs:96-101` | `EntityDataLoaderImpl.groovy:69,169-217,428-430` | `list()`/`checkInfo()` unported; "-- Checked" line only when messages exist (Java `messageNoActionFiles` defaults true) | Med | No |
| P4a-11 | `entity/load_feed/csv.rs:119-121` | `EntityDataLoaderImpl.groovy:1013-1014` | Short CSV row: Rust skips missing columns; Java throws | Med | No |
| P4a-12 | `entity/load_feed/loader.rs:59`, `:749-757` | `EntityDataLoaderImpl.groovy:249-255,330-336` | `options.components` filters explicit `component://` locations; Java only filters auto-discovery | Low | No |
| P4a-13 | `entity/load_feed/loader.rs:244`, `:288` | `EntityDataLoaderImpl.groovy:859,865` | Child-element and service-param text `.trim()`ed; Java keeps raw SAX data | Low | No |
| P4a-14 | `entity/load_feed/xml.rs:102-109` | `EntityDataLoaderImpl.groovy:692-699` | Unknown root element errors; Java ignores any root other than the known types | Low | No |
| P4a-15 | `moqui-lib/src/object.rs:484` | `FieldInfo.java:233` | `"y"/"Y"` → true (matches `ObjectUtilities.java:124`, not `FieldInfo`'s `Boolean.valueOf`) | Low | No |

### Findings — 4b feed / ECA / writer / document / ETL

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P4b-01 | `entity/load_feed/eca.rs:96-97`, `:184` | `EntityFacadeImpl.groovy:968-969,986-990` | EECA rule `entity` not canonicalized; undefined entities not skipped | Med | No |
| P4b-02 | `entity/load_feed/eca.rs:203-212` | `EntityFacadeImpl.groovy:935-936,966-977` | Same-entity rule order: single file-order list vs no-id-first then id'd (Java's id'd order is `HashMap`-undefined) | Low | No |
| P4b-03 | `entity/document.rs:834-857` | `EntityDataDocument.groovy:136,163-169,363-427` | Rows merged by `doc_id` even when not all PKs are aliased; Java emits one doc per row | Med | No |
| P4b-05 | `entity/data_writer.rs:452-465` | `EntityValueBase.java:1128,1181,1187`; `EntityDataWriterImpl.groovy:391-397` | XML record count/message excludes nested dependents | Low | No |
| P4b-06 | `entity/data_writer.rs:303-307`, `:555-557` | `EntityDataWriterImpl.groovy:91-96,152-154` | `file()`/`directory()` create parent dirs; Java errors if the parent is missing (zip path matches) | Low | No |
| P4b-08 | `entity/etl.rs:92-97` | `EntityFacadeImpl.groovy:2151-2174` | `useTryInsert` fallback has an extra `never_try_insert` guard; Java does create then update | Low | No |

#### P4a-01 — CSV BLOB cells are not Base64-decoded

Java's CSV handler special-cases `fi.typeValue == 12`, Base64-decodes the cell and calls `setBytes`.
Rust inserts `Value::string(text)` for every column and `to_bytes` returns its UTF-8 bytes, so a CSV
BLOB column stores the literal Base64 text (and Java errors on invalid Base64 where Rust stores it).

#### P4a-02 — CSV entity rows use the entity-auto store path

Rust CSV rows go through `prepare_fields` (auto `fromDate`, secondary sequence) and
`service::call::store_entity`. Java's CSV reader does its own PK check and then
`value.createOrUpdate()`: a row with an incomplete multi-key PK throws in Java, whereas Rust auto-fills
the key and inserts, and a CSV row that changes `statusId` triggers Rust's status-flow check that the
Java CSV path never runs. (The XML/JSON paths use entity-auto store in both.)

#### P4a-04 — JSON BLOB values decoded that Java stores raw

Rust `json_field_value` Base64-decodes a string for a `binary-very-long` field. Java's JSON path
converts through `FieldInfo.convertFromString`, where typeValue 12 does
`new SerialBlob(value.getBytes())` — raw UTF-8, no decode.

#### P4a-05 — Bad type text stored verbatim

Java's `EntityInfo.setFieldsEv` catches conversion failures and records a validation error, leaving the
field unset, and the error makes the record fail; CSV conversion throws and fails the file. Rust
`basic_convert` returns the original value on failure, so the offending string is written.

#### P4a-08 / P4a-11 — Empty and short CSV rows

Java's empty-cell branch sets the field to null and an empty XML attribute is put as null; Rust skips
these values, so on an update Java clears the column and Rust leaves it, and a `defaultValues` entry
survives where Java nulls it. Java also does `record.get(i)` unguarded so a short row throws; Rust
loads a partial record.

#### P4a-09 — CSV/JSON service name requires `#`

Java calls any name `sfi.isServiceDefined` accepts. Rust's CSV and JSON readers treat a non-entity name
as a service only when it contains `#`; the XML path uses the broader `defined_name` check, so Rust is
internally inconsistent too.

#### P4b-01 — EECA rule entity name not canonicalized

Java resolves a rule's `entity` via `getEntityDefinition` and keys it by `getFullEntityName()`, warning
and skipping an undefined entity. Rust `EcaIndex::ingest` stores the raw attribute and `rule_matches`
compares it literally, so a rule written with a short name silently never fires.

#### P4b-03 — Data documents merged even when not all PKs are aliased

Java's `hasAllPrimaryPks` decides whether to merge rows by doc id; otherwise it emits one document per
query row. Rust `merge_rows` always groups by `doc_id`, so such a DataDocument returns fewer, merged
documents.

#### Phase 4 checked and found equivalent

- `defaultValues` merged into XML, JSON and CSV records (`entity/load_feed/loader.rs:647-666`).
- Per-location error isolation (log and continue) and one transaction per file (`entity/load_feed/loader.rs:94-112,
  166-177`).
- `ec.entity.getEntityDataDocument()` + `writeDocumentsTo*` bound for the shipped DataDocument screens
  (`script/runtime/tools_host.rs:253-266`).
- XML `binary-very-long` child-element Base64 decode with UTF-8 fallback; XML entity/field/relationship
  parsing, nested-record key inheritance, `<seed-data>`/`entity-facade-xml` type skipping.
- JSON array-vs-object roots, `_dataType` marker, per-element `_entity` dispatch; zip entry dispatch.
- `data_types` skip semantics; `only_create`; restricted-entity list; `fromDate` default and PK
  sequencing for XML/JSON; `check` messages, `lastUpdatedStamp` exclusion, PK display format.
- `SimpleEtl` skip rules and error accounting; feed pending registration, delete handling and
  reassembly; `get#DataFeedLatestDocuments`.
- Document control fields, `_index` lowercased, `manualDataServiceName`, expression fields,
  `postQuery`; writer CSV/JSON/XML framing; ETL `dummy_fks`, `extract`, `store_entity` fallback.

The feed's pending-table vs in-memory `DataFeedSynchronization` architecture is a documented
intentional difference and is not counted.

---

## Phase 5 — Entity query / REST

Reviewed `entity/query_rest.rs` (dynamic view build, REST find/list/create/store/update/delete,
path→PK resolution, master/dependent expansion, JSON/XML/CSV writers) with `entity/search_form.rs`,
`entity/sensitive.rs`, `entity/schema.rs`, `screen/dispatch.rs`, `web/mod.rs` and `moqui-db`
`model/{mod,view}.rs` against `EntityFacadeImpl.rest`, `EntityAutoServiceRunner`,
`EntityFindBase.searchFormMap`/`processInputFields`, `EntityDynamicViewImpl`, `EntityDefinition`,
`WebUtilities`, `ContextJavaUtil.jacksonMapper` and `RestSchemaUtil`. JSON-schema differences are
tracked once under P2-06/P2-07/P2-12.

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P5-01 | `entity/query_rest.rs:300`, `:739` | `EntityFacadeImpl.groovy:1730,1754-1755` | REST find of a **view-entity** filters on aliases now but renders only `_entity` (`EntitySnap` copies `entity.fields`, empty for views); PK lookup on a view never happens | High | No |
| P5-11 | `web/mod.rs:514` → `entity/query_rest.rs:1763` | `ContextJavaUtil.java:591-597` | REST JSON serializes `Timestamp`/`Date`/`Time` as quoted strings; Java emits unquoted millis (`WRITE_DATES_AS_TIMESTAMPS`) | Med | No |
| P5-13 | `entity/query_rest.rs:1238-1247` | `EntityDynamicViewImpl.groovy:104-110` | `default-display` alias attribute dropped (unused in `runtime/`) | Low | Unsure |
| P5-15 | `web/mod.rs:514` | `WebFacadeImpl.groovy:721`; `ContextJavaUtil.java:595` | Entity REST JSON compact vs Java `INDENT_OUTPUT` pretty-print | Low | Unsure |
| P5-16 | `moqui-db/src/model/view.rs:267` | `EntityDefinition.groovy:180,205-210`; `EntityValueBase.java:215-251` | `pq-expression` aliases unsupported (fail the `alias X has no entity-alias` check) and unevaluated (unused in `runtime/`) | Low | Unsure |

#### P5-01 — REST find of a view-entity returns only `_entity`

The model stores view aliases in `EntityDefinition.aliases` and leaves `fields` empty for a view.
Conditions now go through `search_form_map`, which reads aliases (`entity/search_form.rs:125-144`), but
`EntitySnap::from_entity` still copies only `entity.fields` and `render` iterates `snap.fields`, so the
response rows carry only `_entity` and `pk_complete` is always false. Java iterates
`ed.allFieldInfoList` (aliases for a view) and returns alias values.

#### P5-11 — REST timestamps quoted instead of millis numbers

Java serializes with `ContextJavaUtil.jacksonMapper`, which enables `WRITE_DATES_AS_TIMESTAMPS`, so
`Timestamp`/`Date`/`Time` becomes an unquoted millis number. Rust's JSON writer has no arm for those
variants, so they fall through to a quoted string. Decimals match.

#### Phase 5 checked and found equivalent

- Identity-admin/secret-config read and write bans and `stripUserAccountSecrets` on every response
  (`entity/query_rest.rs:186-211`, `entity/sensitive.rs` mirrors `WebUtilities`).
- REST writes go through entity-auto (`write_rest`, `entity/query_rest.rs:234-255`), so status-flow checks
  and `oldStatusId`/`statusChanged` apply.
- REST field conditions via `search_form_map`: `_ic`, `_period`/`_poffset`/`_pdate`, date-only `_thru`
  padding, aggregate aliases to HAVING, unknown `_op` adds nothing.
- Multi-step relationship paths (`entity/query_rest.rs:417-438`); path→entity+PK resolution,
  `masterNameInPath` fallback, verb→operation mapping.
- Dynamic view `<complex-alias>`/`case`/`when`/`else`, alias attributes, member `<field>`, `alias-all`
  expansion; view `group-by`/`having` inference; `<order-by>`/`<entity-condition>`.
- Body nested-map/child recursion and parent-PK propagation; response shape, `X-Total-Count`/`X-Page-*`
  headers, error JSON and status codes; decimal formatting; REST authorization frame.
- Out of scope (not findings): Swagger/RAML, Elasticsearch/DataDocument `documentList`, JSON-RPC,
  runners, service-group, Hazelcast, LLM/A2A, extra drivers.

---

## Phase 6 — Service definition / validation / call / semaphore

Reviewed `service/{define,validate,info,call,semaphore}.rs` against `ServiceDefinition.java`,
`ParameterInfo.java`, `ServiceFacadeImpl.groovy`, `ServiceCallImpl.java`, `ServiceCallSyncImpl.java`,
`ServiceCallSpecialImpl.groovy` and the inline/entity-auto/script runners. Async is in Phase 7.

### Findings — 6a definition / validation / info

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P6a-01 | `service/validate.rs:141`, `:167`, `:168` | `ServiceDefinition.java:212,492,519-545,556-558` | `validate="false"` gates only unknown-parameter retention (`:78`) and nesting (`:97`); Rust still enforces `required`, `allow-html` and declared checks (only `noop` uses `validate="false"` in `runtime/`) | Med | No |
| P6a-03 | `service/validate.rs:393-394` | `ParameterInfo.java:154-193` | Number conversion ignores the parameter `format` (date/time/timestamp now use it) | Med | No |
| P6a-04 | `service/info.rs:26-34`; `service/define.rs:572-578` | `ServiceFacadeImpl.groovy:336-359,380-413` | `known_service_names` returns only loaded services; Java scans every component `service/` dir | Med | Unsure |
| P6a-06 | `service/define.rs:1211-1224`, `:1032` | `ServiceDefinition.java:106-152` | Implements parameters appended after own params (Java places them first); an own param of the same name replaces the interface one instead of merging attributes, so an untyped own param becomes `String` | Med | No |
| P6a-07 | `service/validate.rs:586-608` | `ServiceDefinition.java:738-782` | `time-range` bounds exclusive vs Java inclusive; unparseable value errors in Rust but is skipped in Java | Med | No |
| P6a-08 | `service/validate.rs:578-583` | `ServiceDefinition.java:41,698-715` | Email/letters/digits checks are ASCII regexes; Java uses `EmailValidator` and `Character.isLetter`/`isDigit` | Low-Med | No |
| P6a-09 | `service/info.rs:37-39` | `ServiceFacadeImpl.groovy:492-497,530-534` | `getSecaRuleCount` counts distinct rules; Java counts per-service expansions | Low | Unsure |
| P6a-10 | `service/validate.rs:368` | `ServiceDefinition.java:265-272,296-304` | Auto-expanded params omit the `field-name` Java sets | Low | No |
| P6a-11 | `service/define.rs:1034` | `ParameterInfo.java:95-98` | Empty `default-value=""` counts as a default in Rust, not in Java | Low | No |
| P6a-12 | `service/validate.rs:49` | `ParameterInfo.java:101-102`; `ServiceDefinition.java:396-553` | `required="disabled"` params skipped before `known.insert`, so `retain_known` drops them; Java still converts a passed disabled param | Low | Unsure |
| P6a-13 | `service/validate.rs:376-381`, `:625-640` | `ServiceDefinition.java:531-543,568-579` | Stops at the first failing check per param; Java collects all failures, including failing OR branches | Low | No |
| P6a-14 | `service/validate.rs:73`, `:142` | `ServiceDefinition.java:492-493,594,656` | Error text differs (`X is required (in service …)` vs `Field cannot be empty`) | Low | No |
| P6a-15 | `service/define.rs:554-565` | `ServiceFacadeImpl.groovy:275-298` | File lookup takes the first existing file; Java lets later components override | Low | Unsure |
| P6a-16 | `service/define.rs:949-957` | `ServiceDefinition.java:190-195` | Non-numeric `transaction-timeout` silently ignored vs Java `Integer.valueOf` error (`"0"` → unset is equivalent) | Low | No |
| P6a-17 | `service/validate.rs:692-708` | `ServiceCallSyncImpl.java:558-606` | Implicit entity-auto calls filtered to fields + Map/List values; Java passes the raw map | Low | Unsure |

### Findings — 6b call / semaphore

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P6b-01 | `service/call.rs:382-397` | `ServiceCallSyncImpl.java:320-356,391` | A service-body failure is returned as `Err` after `add_error`; Java catches it, records the message/cause chain, rolls back, and returns the result map (only `ArtifactAuthorizationException` rethrown) | High | Decision pending |
| P6b-10 | `service/semaphore.rs:196-201` | `ServiceCallSyncImpl.java:461-466` | Empty-string `semaphore-parameter` maps to `_NULL_` in Rust but `""` in Java | Low | No |
| P6b-11 | `service/semaphore.rs:63` | `ServiceCallSyncImpl.java:497-517` | Semaphore-conflict error text differs | Low | No |
| P6b-12 | `service/call.rs:1109-1113` | `EntityAutoServiceRunner.groovy:487-491` | Invalid status transition error hardcoded; Java expands `StatusFlowTransitionNotFoundTemplate` | Low | No |
| P6b-14 | `service/call.rs:827-838`, `:865-879` | `ServiceCallSyncImpl.java:322,391`; `InlineServiceRunner.java:56-66`; `ScriptServiceRunner.java:56-83` | Rust collects only declared out-parameters; Java returns the runner map, so undeclared `result` keys / a body-returned Map are dropped in Rust | Low | Unsure |

#### P6a-01 — `validate="false"` still enforces required/HTML/checks

Java's `nestedParameterClean` gates the required-field error (`parameterIsEmpty && validate &&
parameterInfo.required`), the `allow-html` check and every `ParameterValidation` sub-element behind
`validate`; type conversion is not gated. Rust always runs `required` (`service/validate.rs:141`), `clean_html`
(`:167`) and `check_param` (`:168`); only `retain_known` and nesting look at `validate_input`. No
shipped `runtime/` service with parameters uses `validate="false"` today, so the impact is on
component services that do.

#### P6a-06 — Implements parameter ordering and merge

Java inserts deep-copied `implements` parameters first, then merges own params by name, so an own param
that only overrides one attribute keeps the interface's `type`, `required`, etc. Rust appends
implemented params after own params, and a same-named own param replaces the interface one entirely;
an own param with no `type` then defaults to `String` (`service/define.rs:1032`).

#### P6a-07 / P6a-08 — `time-range` and character-class validation

Java `time-range` compares with `compareTo(...) < 0` / `> 0`, so equality passes and an unparseable
value is skipped; Rust uses exclusive bounds and errors on an unparseable formatted value. Java's email
check uses `EmailValidator` and letters/digits use `Character.isLetter`/`isDigit`; Rust uses ASCII
regexes, so non-ASCII letters/digits and some emails validate differently.

#### P6b-01 — A service-body failure throws in Rust but returns a map in Java

Java's `callSingle` wraps the runner in `try { … } catch (Throwable t)`, adds `t`'s message and cause
chain to the message facade, rolls back, then returns the result map. Rust maps any `Err` from
`execute` to `add_error(...)` and then returns `Err(err)`. A failing service body therefore makes Rust
`sync(...).call()` return `Err` where Java returns normally with the error in the message facade. The
Rust tests assert this `Err`; see [Open decisions](#open-decisions).

#### Phase 6 checked and found equivalent

- Nested `<parameter>` children parsed and validated (`service/define.rs:1042`, `service/validate.rs:84-125`).
- Implicit entity-auto skips authentication (`Authenticate::Off`, `service/define.rs:215-217`); an `AT_SERVICE`
  frame is still pushed but with `requires_authz=false`.
- Early "errors already in the facade" abort with `ErrorsAside` push/pop, and the rollback-only guard
  (`service/call.rs:271-291,402-416`).
- `verb_action` maps `get`/`search` to `view` (`service/call.rs:760`); entity-auto `delete` requires a full PK
  or `*` wildcard (`delete_entity`, `service/call.rs:972-1015`) and returns an empty map.
- Service `type` default, `authenticate` values, `transaction`/`no-tx-cache` defaults,
  `no-remember-parameters`, interface pass-through; semaphore attribute defaults and none/fail/wait.
- `auto-parameters` include/exclude/required/allow-html; `default` precedence, requirement-after-
  defaults, empty-value handling; `min/max-include-equals`, `matches`/`number-*`/`text-length`/
  `credit-card`, any-scheme `text-url`, `allow-html` modes.
- Service-name construction and combined-camel-case resolution; transaction attribute mapping and
  timeout selection; input-validation no-throw/empty result; SECA phases and tx-hook registration.
- `multi(true)` row gathering; `authUsername`/`authPassword` login/logout; semaphore fail/wait,
  staleness, insert-race retry, keying by `semaphore-name`.
- Entity-auto `checkFromDate`, sequencing, `oldStatusId`/`statusChanged`, `StatusFlowTransition`;
  `store` create-vs-createOrUpdate; nested recursion and parent-PK propagation.
- Out of scope (not findings): JSON-RPC, Groovy/Java runners, `service-group`, Swagger/RAML, XML-RPC.

---

## Phase 7 — ECA / SECA / email ECA / pools / jobs / scheduler

Reviewed `service/{eca,eca_rules,seca,rule_actions,email_eca,security_checks}.rs` and
`service/{pool,request_pool,jobs,schedule}.rs` against `EntityEcaRule.groovy`, `ServiceEcaRule.groovy`,
`EmailEcaRule.groovy`, `ServiceFacadeImpl.groovy`, `ServiceCallAsyncImpl`/`ServiceCallJobImpl`/
`ScheduledJobRunner.groovy`, `ExecutionContextFactoryImpl`, `UserSecurityChecks.groovy` +
`Security.eecas.xml`, and `MoquiDefaultConf.xml`.

### Findings — 7a ECA / SECA / email / security

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P7a-03 | `service/call.rs:293`, `:316` | `ServiceCallSyncImpl.java:173,215,239,565` | Implicit entity-auto: `pre-validate` SECA fires once in Rust, twice in Java; and Rust runs `pre-auth` SECA, which Java never reaches (it returns at `:215`) | Med | No |
| P7a-05 | `service/security_checks.rs:142-188` | `UserFacadeImpl.groovy:852-858` | `user_in_group` omits the `userGroupId == "ALL_USERS"` short-circuit, and `user_has_permission` omits ALL_USERS group permissions that Java `hasPermission` includes | Med | No |
| P7a-06 | `entity/load_feed/eca.rs:191-194`; `service/email_eca.rs:34-37`; `script/actions.rs:76-81` | `EmailEcaRule.groovy:44-47`; `EntityEcaRule.groovy:44-47` | EECA and EMECA AND every `<condition>` child; Java wraps only the first (SECA now matches) | Low | Unsure |
| P7a-07 | `script/runtime/convert.rs:123-137` | `actions/XmlAction.java:77-81` | `eval_condition` JS truthiness (empty array/object true) vs Groovy `as Boolean` (empty collection false) | Low | Unsure |
| P7a-08 | `service/seca.rs:132-168` | `ServiceFacadeImpl.groovy:415-508` | SECA patterns matched at runtime; Java expands patterns to defined services at load | Low | Yes |
| P7a-10 | `service/call.rs:395` | `ServiceCallSyncImpl.java:388` | `post-commit` on a failed call passes `results = None`; Java passes an empty map | Low | No |
| P7a-11 | `service/seca.rs:65-66` | `ServiceEcaRule.groovy:82-101`; `ServiceCallSyncImpl.java:307-322` | Rust publishes a copy of `parameters`; Java puts the live `currentParameters` map, so a pre-* SECA can change the call's inputs | Low-Med | Unsure |
| P7a-12 | `service/call.rs:363`, `:524-559` | `ServiceEcaRule.groovy:104,143-173` | Tx rules: Java's `registerTx` never fires for pattern rules, Rust's `has_rules` includes them; Rust registers `tx-rollback` before the run with no results, Java after | Low | No |

### Findings — 7b pools / async jobs / scheduler

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P7b-01 | `service/pool.rs:715-719`, `:786` | `ServiceFacadeImpl.groovy:134-144` | `scheduled-job-check-time=0` disables the runner in Java; Rust clamps to 1s and always runs the scheduler | Med | No |
| P7b-02 | `service/schedule.rs:34`, `:49` | `ServiceFacadeImpl.groovy:135-140` | No 120s initial delay; Rust's first pass is at `check_interval` | Med | Unsure |
| P7b-05 | `service/pool.rs:592-599`, `:1057-1071` | `ExecutionContextFactoryImpl.groovy:848-861` | Shutdown fails queued async calls; Java `workerPool.shutdown()` drains them (Java never shuts down `jobWorkerPool`, so jobs match) | Med | Unsure |
| P7b-07 | `service/call.rs:87-90`; `service/pool.rs:519-544` | `ServiceCallImpl.java:116-129`; `ServiceCallAsyncImpl.groovy:199` | Async params not validated/converted at queue time, no authz pre-check before queueing, `authUsername` not added | Med | No |
| P7b-08 | `service/pool.rs:930-942` | `ServiceCallAsyncImpl.groovy:199` | Async worker calls `sync` with authz enabled; Java disables authz (it was checked before queueing) | Med | No |
| P7b-10 | `service/jobs.rs:300-305` | `ScheduledJobRunner.groovy:240-250` | Pool full at submit: Rust deletes the `ServiceJobRun` row; Java keeps it as a failed run | Low | No |
| P7b-11 | `service/jobs.rs:801` | `EntityListImpl.java:95,102` | `thruDate` boundary inclusive (Java) vs exclusive (Rust) | Low | No |
| P7b-12 | `service/jobs.rs:805-807` | `ScheduledJobRunner.groovy:125` | `paused=""` treated as paused by Java, active by Rust | Low | No |
| P7b-13 | `service/pool.rs:43` | `ExecutionContextFactoryImpl.groovy:467` | Default `worker-queue` 65535 vs 65536 | Low | No |
| P7b-14 | `service/jobs.rs:446-451` | `ServiceCallJobImpl.groovy:233-238` | `ServiceJobRun` omits `hostAddress`/`hostName`; `runThread` name differs | Low | No |
| P7b-15 | `service/jobs.rs:99-101` | `ScheduledJobRunner.groovy:261-266` | Runner stats not updated on a failed pass; Java still updates | Low | No |

#### P7a-03 — Implicit entity-auto SECA phases

Java runs `pre-validate` SECA in the main body and again inside `runImplicitEntityAuto`, and returns
from `runImplicitEntityAuto` at `:215` before the `pre-auth` SECA at `:239`. Rust's single `sync_with`
runs `pre-validate` once and then runs `pre-auth`, so a non-idempotent pre-validate rule fires half as
often and a pre-auth rule fires where Java never runs it.

#### P7a-05 — ALL_USERS group handling

Java `isInGroup` returns true for `ALL_USERS` without a lookup, and `hasPermission` includes
permissions granted to the ALL_USERS group. Rust `user_in_group` and `user_has_permission` only consult
the user's own date-ranged memberships, so a permission granted to ALL_USERS is denied in Rust.

#### P7a-11 — SECA parameters are a copy

Java's SECA context `parameters` is the live `currentParameters` map, so a `pre-validate`/`pre-auth`/
`pre-service` rule that sets a parameter changes what the service receives. Rust publishes a copy, so
such a rule's changes are lost.

#### P7b-01 / P7b-02 — Scheduler enablement and initial delay

Java starts the `ScheduledJobRunner` only when `scheduled-job-check-time > 0`. Rust applies `.max(1)`
and always spawns the scheduler, so a deployment that sets `0` to disable scheduling gets a pass every
second. Java also waits a 120s initial delay; Rust's first pass is after `check_interval` (default
60s).

#### P7b-05 — Shutdown drops queued async calls

Java's `destroy` calls `workerPool.shutdown()` then `awaitTermination(30s)`, letting queued async calls
finish. Rust `ServicePools::shutdown` → `fail_pending` empties the queue and sends
`Err("service worker pool is not running")` to async callers. Java never shuts down `jobWorkerPool`,
so abandoning queued jobs is not a divergence.

#### P7b-07 / P7b-08 — Async queue-time validation and authz

Java's `ServiceCallAsyncImpl.call()` runs `validateCall` before queueing: the authz check
(`ServiceCallImpl.java:122-127`), `convertValidateCleanParameters`, and adding `authUsername`; the
worker then logs the user in and calls `.disableAuthz()`. Rust passes the raw map to the pool, validates
inside the worker, never checks authz before queueing, and calls `call::sync` with authz enabled — so a
caller without authz can queue work, validation errors surface later, and an async call touching
authz-controlled artifacts can fail where Java succeeds.

#### Phase 7 checked and found equivalent

- SECA/EMECA discovery per component, id-overwrite, priority ordering; SECA rules for undefined
  services skipped at load (`service/eca_rules.rs:104-112`, `service/seca.rs:267-278`); SECA uses only the first
  `<condition>` child (`service/seca.rs:193-197`).
- Runtime name matching; `run-on-error` gating for SECA; phase set/ordering and tx hooks; tx rules run
  on a worker in a new transaction and skip sim sessions (`service/call.rs:524-559`); condition-false skip.
  SECA context carries `parameters`/`results` (as a copy — see P7a-11).
- Action dispatch `service-call`/`entity-*`/`script`/`in-map="context"`; email ECA context fields.
- Security-check logic: empty-id, `authzDisabled`, privileged/sealed membership, date-ranged lookups,
  error strings.
- Registered callbacks run (`call_callbacks`, `service/call.rs:393`).
- Jobs run on a separate job queue sized by `job-pool-core` (`service/pool.rs:249-252,806-833`); job topic
  notification plus `ServiceJobError` to the user and `ServiceJobUser` recipients (`service/jobs.rs:578-647`).
- Cron parsing, lock-claim concurrency, `expireLockTime`/`minRetryTime`/`transactionTimeout` defaults,
  expired-lock run, `repeatCount` pause, `lastSchedule < lastRunTime` skip, min-retry-after-error;
  run-history write and clipping; `run_job_now`; parallel-count accounting.
- Out of scope (not findings): JSON-RPC, Groovy/Java runners, `service-group`, Swagger/RAML,
  Elasticsearch, Hazelcast cluster job coordination, LLM/A2A, extra drivers. `request_pool.rs` has no
  Java counterpart.

---

## Phase 8 — REST/remote/tools, mail, and component services

Reviewed `service/{rest,remote,tools}.rs`, `service/mail/{implementation,mod}.rs`, and
`service/{sso,elfinder,images,spreadsheet}.rs` + `service/system_sync/*` against `RestApi.groovy`,
`ServiceServices.xml`, `RemoteRestServiceRunner.groovy`, `PrintServices.xml`, `EntityServices.xml`, the
mail Groovy resources and `EmailServices.xml`, `ElFinderConnector.groovy`, `EntitySyncServices.xml`,
`SystemMessageServices.xml`/`WebFacadeImpl.handleSystemMessage`, `SftpMessageServices.xml`,
`AuthServices.xml`/`AuthenticationFlow.groovy`/`MoquiSecurityGrantedAccessAdapter.groovy`, and the
image/spreadsheet service XML. The `*.rest.xml` HTTP route/CORS layer lives in `screen/dispatch.rs` /
`web/mod.rs`.

### Findings — 8a REST / remote REST / tools / print

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P8a-03 | `service/rest.rs:564` | `RestApi.groovy:833` | `run_service` results on `/rest/s1` not passed through `strip_user_account_secrets` (entity results are, `service/rest.rs:614`); Java strips every `RestResult` | High | No |
| P8a-04 | `screen/dispatch.rs:154` | `RestApi.groovy:648-662` | `X-HTTP-Method-Override` honored for `/rest/e1` (`screen/dispatch.rs:332-343`) but not `/rest/s1` | Med | No |
| P8a-06 | `service/rest.rs:327-331` | `RestApi.groovy:605` | Resource `description` read from a child element; real `*.rest.xml` put it on the attribute | Med | No |
| P8a-08 | `service/remote.rs:169-180` | `RemoteRestServiceRunner.groovy:77` | 2xx non-JSON body yields `{response:<text>}` and an empty body `{}`; Java `jsonObject()` throws and fails the call | Med | Unsure |
| P8a-11 | `service/rest.rs:564` | `RestApi.groovy:252` | Null-valued out-parameters not removed (`nestedRemoveNullsFromResultMap`) | Low | Unsure |
| P8a-12 | `service/remote.rs:87` | `RemoteRestServiceRunner.groovy:71-75` | Error message omits the reason phrase | Low | No |
| P8a-13 | `service/remote.rs:143-156` | `RestClient.java:941-952` | GET query omits list/map and empty values | Low | Unsure |
| P8a-15 | `outbound.rs:29-35` | `RestClient.java:89-90` | `HEAD`/`OPTIONS` dropped, falling back to POST | Low | No |
| P8a-17 | `service/tools.rs:203-212` | `PrintServices.xml:100,121-123` | `print#ScreenDocument` rejects non-PDF and ignores `webappName` | Low | Unsure |
| P8a-18 | `service/rest.rs:296-298`, `:343-345` | `RestApi.groovy:192,614` | Empty `require-authentication=""` inherits the parent in Java, required in Rust | Low | No |
| P8a-19 | `service/rest.rs:310` | `RestApi.groovy:618` | Rust lowercases `method type`; Java keys by the verbatim attribute | Low | No |
| P8a-20 | `service/rest.rs:375`, `:224` | `RestApi.groovy:616-632` | Unsupported method child fails the whole file; Java registers no handler | Low | No |
| P8a-21 | `service/tools.rs:618`, `:664-668` | `EntityServices.xml:176-192` | Empty-string userIds dropped; DataDocument read without `useCache(true)` | Low | Unsure |
| P8a-22 | `service/rest.rs:132-161` | `RestApi.groovy:100-106,597,634-644` | `resource_info` omits `childPaths`/`methods` (the dashboard uses only `childMethods`, which is recursive on both sides) | Low | Unsure |
| P8a-24 | `service/rest.rs:729` (`openapi_document`) | `RestApi.groovy:471`; `RestSchemaUtil.groovy:517` | Service-parameter schemas omit the `enum` lists Java derives from `Enumeration`/`StatusItem` (entity schemas have them since P2-07) | Low | Unsure |

### Findings — 8b mail / email services

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P8b-03 | `runtime/…/org/moqui/impl/EmailServices.xml:82-83` | `EmailServices.xml:72-73` | `save#EcaEmailMessage` stores addresses as `join(', ')` vs Java `List.toString()` (`[a, b]`) | Med | Unsure |
| P8b-04 | `service/mail/implementation.rs:1658-1666` | `pollEmailServer.groovy:111` | Expunge only when this poll flagged deletes; Java `close(true)` always expunges | Low | Unsure |
| P8b-06 | `service/mail/implementation.rs:780-791` | `sendEmailTemplate.groovy:283-286` | Default attachment filename differs (Java strips 4 chars / NPEs) | Low | Unsure |
| P8b-07 | `service/mail/implementation.rs:2073-2100` | `sendEmailTemplate.groovy:161-165` | Recipient split respects quotes/`<>`; Java splits on every `,` | Low | Unsure |
| P8b-09 | `service/mail/implementation.rs:384` | `sendEmailTemplate.groovy:88-90` | Text alternative rendered only when `allows_text`; Java always renders it | Low | Unsure |
| P8b-10 | `service/mail/implementation.rs:981-983` | `sendEmailTemplate.groovy:141` | Empty expanded subject omits the `Subject` header; Java sets it unconditionally | Low | Unsure |

### Findings — 8c SSO / elFinder / system sync / images

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P8c-02 | `service/elfinder.rs:319-322` | `ElFinderConnector.groovy:152` | `info.write` from `writes_restricted()` only, not `supportsWrite()` (Rust roots are always writable, so near-moot) | Low | No |
| P8c-03 | `service/elfinder.rs:44` | `ElFinderConnector.groovy:257` | Root-refused error text differs | Low | No |
| P8c-04 | `service/elfinder.rs:621-641` | `ElFinderConnector.groovy:316,326` | `mkdir`/`mkfile` omit the `supportsWrite()` check (near-moot, as P8c-02) | Low | No |
| P8c-05 | `service/sso.rs:255` | `AuthenticationFlow.groovy:89-91` | Callback ignores `RelayState` (moot without SAML) | Low | Unsure |
| P8c-06 | `service/sso.rs:204-207` | `AuthenticationFlow.groovy:50-53` | `login` clears a stored `returnTo` when none given | Low | Unsure |
| P8c-07 | `service/sso.rs:347` | `MoquiSecurityGrantedAccessAdapter.groovy:35` | `map_fields` keeps numeric `0`; Groovy truthiness treats `0` false | Low | No |
| P8c-11 | `service/images.rs:317` | impl `ImageServices.xml:109` | "Original" output format uses detected lowercase suffix; Java usually writes `outputImageFormat` | Low | Unsure |
| P8c-16 | `service/elfinder.rs:753-755` | `ElFinderConnector.groovy:75-93` | A hash of exactly the 3-char volume id decodes to invalid; Java yields the root | Low | No |
| P8c-17 | `service/sso.rs:286-290` | `AuthenticationFlow.groovy:139-155` | `logout` always ends local login; Java only if pac4j `perform` did not throw | Low | Unsure |

#### P8a-03 — Service-REST service results not stripped of secrets

Java wraps every Service-REST response in `WebUtilities.stripUserAccountSecrets` (`RestApi.groovy:833`),
removing `currentPassword`/`resetPassword`/`passwordSalt`/`passwordHashType`/`passwordBase64` wherever
they appear. Rust strips entity results (`service/rest.rs:614`) but returns `run_service` results as produced
(`service/rest.rs:564`), so a service method on `/rest/s1` whose out-parameters include a UserAccount map leaks
password material. The fix is to run the `entity/sensitive.rs` stripper over service results too.

#### P8a-04 / P8a-06 — REST verb override and resource description

Java resolves the effective verb through `PathNode.effectiveRequestMethod` (honoring
`X-HTTP-Method-Override`) for both service and entity dispatch; Rust does this only for `/rest/e1` and
passes the raw method for `/rest/s1`, so an override POST gets 405. Java reads resource `description`
from the attribute, which real `*.rest.xml` files use; Rust reads a child element, so descriptions are
always empty.

#### P8a-08 — Remote-REST response handling

On a 2xx response Java parses JSON and throws on a non-JSON or empty body. Rust tolerates a non-JSON
body as `{"response": "<text>"}` and an empty one as `{}`, changing success/failure and the value
shape.

#### P8b-03 — Email address formatting

The port's `save#EcaEmailMessage` stores `toAddresses` (and cc/bcc) as `join(', ')`, where Java uses
Groovy `List.toString()` (`[a, b]`), so stored values differ for anything that reads them back.

#### Phase 8 checked and found equivalent

- 8a: identity-admin read ban and identity/secret write bans with UserAccount input stripping
  (`service/rest.rs:591-606`); entity results stripped and returned without `_entity` (`service/rest.rs:614,632-642`;
  master path keeps it, as Java); `count` via `search_form_map` with `X-Total-Count`; remote-REST
  `method` attribute honored (`service/call.rs:800-808`); `rewrite#EntityEncryptedFields` logs and skips
  records that won't decrypt or update (`service/tools.rs:778-800`).
- 8a: root/path 404s and error kinds; `*.rest.xml` discovery and root replacement; named-child
  precedence; `allow-extra-path`; `AT_REST_PATH` frame/authz; `anonymous-all`/`anonymous-view`; entity
  operation set and `{operation}#{entity}` dispatch; find-one-not-found → null; default list limit 100
  and pagination headers; `masterName`; remote-REST location/error/GET-vs-body handling; print
  services; `send#DataDocumentNotifications`; `rewrite#EntityEncryptedFieldsAll`.
- 8b: draft lifecycle; cc/bcc merge; allowed-host DNS-suffix; SMTP auth/TLS/port; attachment branches;
  IMAP/POP3 protocol set and config; Email-ECA field mapping; SIM-session skip.
- 8c: elFinder hash/unhash, response fields and command set; SSO service names/params/flows, account
  sync, profile mapping, OIDC/OAuth config, failure redirect; `login` refusing disabled/inbound flows
  matches Java's callback `buildAll()` filter; entity-sync lock/guards/status/history — including the
  early return when `internalRun` hits an error, which Java's XML `service-call` also does
  (`XmlActions.groovy.ftl:69`); `get#EntitySyncData` split loop, include-list expansion, put options;
  system-message REST send/receive and missing-HMAC 403; SFTP send/poll; image convert/write and
  spreadsheet read/load (XML descriptor).
- Out of scope (not findings): JSON-RPC, `RemoteJsonRpcServiceRunner`, Groovy/Java runners,
  `service-group`, Swagger/RAML (Rust's `openapi_document` is an addition), Elasticsearch, Hazelcast,
  LLM/A2A, extra drivers.

---

## Accepted differences

Deliberate differences that are documented in code or follow project policy (AGENTS.md failure
tolerance). Not part of the worklist; revisit only if the rationale changes.

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P1-07 | entity condition | A raw `Where` fragment is wrapped in `(...)`; Java appends it verbatim, so boolean precedence can differ (`moqui-db/src/sql/condition.rs:659-665`) | Safer: an `OR` in the fragment cannot bind across the conditions around it; comment at the site |
| P1-13 | entity list | Rust `EntityList` has only len/get/iter/values; Java has `filterByAnd`, `orderByFields`, `getFirst` and more (`entity/value.rs:243-277`) | Rust callers use slice/iterator methods; the script runtime's list has the helpers scripts call (`filterByDate`, `getPartialList`, `first`), and no converted component uses the others; doc comment at the type |
| P2-08 | entity find | Distinct and select-field finds bypass the entity cache; Java caches them by reading every field, so a cached entity answers with whole, non-distinct rows (`entity/find.rs:238-244`) | Rust returns the rows and fields asked for; comment at `bypasses_cache` |
| P2-10 | entity find | `iterator()` under `requireSearchFormParameters` returns an empty iterator; Java returns `null` (`entity/find.rs:524-530`) | Rust has no null iterator; empty is the safe equivalent |
| P2-12 | entity schema | JSON schema envelope redesigned: `$defs`/draft 2020-12, PK `required`, keyed by full name vs Java `definitions`/`anyOf`/pagination, draft-04, `_entity`+`default`, short-or-full name (`entity/schema.rs:27,89-128`) | Deliberate modernization; relationships and `enum` now follow Java inside it (P2-06/P2-07 fixed) |
| P2-14 | search form | `_not` with `_ic` on `equals` compares ignoring case; Java throws, since `ignoreCase()` on the `orNull` join condition is unsupported (`entity/search_form.rs:269-272`) | A search a user can type should not fail; comment at the site |
| P3-06 | datasource / DDL | No table, constraint or index name is schema-qualified: Postgres sets the session `search_path` to `schema-name` (`moqui-host/src/db/entity_db.rs:202-207`), `use-schema-for-all` (only the DB2 profiles) is unread, and a MySQL `schema-name` is logged and not used (`entity/datasource.rs:222-229`) | Same placement on Postgres without qualifying each statement; no DB2 driver; on MySQL Java's `schema.TABLE` is another database, which the jdbc-uri names. `use-schemas="false"` is now honored (no search path, `entity/datasource.rs:178-184`); comments at both sites |
| P3-11 | DDL | `fk-style="name_fk"` (the MySQL profiles) is not followed: always `ADD CONSTRAINT name FOREIGN KEY` (`moqui-db/src/sql/statement.rs:562-571`) | MySQL 8.0.16 and later ignore the `name_fk` name and call the key `{table}_ibfk_N`, so the check that goes by name (P3-21) would add it again on every check; doc comment at `create_foreign_key` |
| P3b-10 | feed cache | Feed-info cache invalidated on write vs Java 900s TTL (`entity/cache.rs:838-845`) | Rust is more current |
| P4b-07 | data feed | `get#DataFeedDocuments` honors `thruUpdateStamp`; Java always passes null | Bug fix; comment at runtime `EntityServices.xml:35-36` |
| P4b-09 | data writer | `isoDateTime(true)` formats dates in JSON and XML; Java ignores it (`TODO`) (`entity/query_rest.rs:1555-1578,1704-1721`) | Implements Java's TODO |
| P5-14 | entity REST | Request `pageSize` capped at `entity_rest_max_page_size` (1000); Java caps nothing | Documented `entity/query_rest.rs:534-553` |
| P6a-05 | service tools | Service detail view is a rebuilt node (`method` null, no `groovyString`, some attributes dropped) | Documented `service/info.rs:183-184` |
| P6b-08 | semaphore | Row cleared only while `lockThread` equals Rust's owner label | Documented `service/semaphore.rs:72-73,204-205` |
| P7a-01 | EECA | Security-check EECAs run even when the message facade already has errors (Java skips them) | Stricter by design; `service/eca.rs:74-75`, test at `service/eca.rs:327` |
| P7a-04 | security | `SSH_SHELL` sealed in Rust conf/constants | Rust-only artifact (`service/security_checks.rs:31-32`, `MoquiDefaultConf.xml:63-64`) |
| P7a-09 | email ECA | A failing email rule is logged and the rest run; Java aborts | AGENTS.md failure-tolerance policy |
| P7b-04 | pools | Workers = `worker-pool-max` − job core; core/alive ignored (Java effectively runs `worker-pool-core`) | Comment at `service/pool.rs:741-747` |
| P7b-06 | jobs | Lock row with a run id but null `lastRunTime` holds the job | Comment at `service/jobs.rs:224-227`; Java always writes both |
| P8a-14 | remote REST | Substituted `${param}` values percent-encoded; Java does not URL-encode | Doc comment `service/remote.rs:107-108` |
| P8a-16 | service REST | No-user denial returns 401; Java throws (403) | Comment `service/rest.rs:497-498` |
| P8a-23 | service REST | Authc message says "operation"; Java has the "operaton" typo | Don't copy the typo |
| P8b-01 | mail | `allowedToDomains` dot-boundary match on the extracted address vs Java `endsWith` on the raw address | Hardening; doc comment `service/mail/implementation.rs:545-564` |
| P8b-02 | mail poll | Per-message `ForceNew` transaction and continue-on-error vs Java one poll transaction aborting on first throw | AGENTS.md policy; comment `service/mail/implementation.rs:1303-1307` |
| P8b-05 | email ECA | `message` context is a Map, not the live `MimeMessage` | Comment `service/mail/implementation.rs:1402` |
| P8b-08 | mail | `send#Email`/`send#EmailTemplate` gain `body`/`bodyText` overrides | Module doc `service/mail/implementation.rs:23`, `:83-84` |
| P8c-08 | SSO | Default group added only when non-empty | `service/sso.rs:479-481` |
| P8c-10 | elFinder | `rm` validates all targets before deleting; Java deletes then returns on first bad | Comment `service/elfinder.rs:655` |
| P8c-12 | images | Suffix/MIME lists hardcoded; Java uses live `ImageIO` lists | `service/images.rs:46-55` |
| P8c-13 | system message | Replay-cache TTL check on received messages (Rust-only) | Hardening; `service/system_sync/message.rs:430-446` |
| P8c-14 | SSO | SAML flows error as unavailable; Java builds a `SAML2Client` | No SAML client in the port (`service/sso.rs:180-184`) |
| P8c-15 | elFinder | Symlink-to-directory deleted as a link; Java follows and deletes contents | Comment `service/elfinder.rs:425-431` |
