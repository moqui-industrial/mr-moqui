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

26 open findings and 50 accepted differences. Open by phase:

| Phase | Open | High |
|---|---|---|
| 1 Entity value / field / condition | 2 | — |
| 2 Entity find / query | 0 | — |
| 3 Facade / metadata / DDL / cache | 3 | — |
| 4 Data loader / feed / ECA / writer / document / ETL | 1 | — |
| 5 Entity query / REST | 3 | — |
| 6 Service definition / validation / call | 1 | P6b-01 (decision) |
| 7 ECA / SECA / pools / jobs | 0 | — |
| 8 REST / remote / tools / mail / components | 16 | — |

| ID | Area | One-line |
|---|---|---|
| P6b-01 | service call | Service-body failure returns `Err`; Java returns the result map — **open design decision** |

### Cross-cutting threads

1. **Access control** (none open): service-REST results are now stripped of UserAccount secrets at any depth (P8a-03
   fixed), and a `/rest/s1` path is authorized for the method an `X-HTTP-Method-Override` names (P8a-04 fixed). (Async
   calls now check authorization as the caller before they are queued and run with it disabled, P7b-07 and P7b-08
   fixed, and `ALL_USERS` membership follows Java, P7a-05 fixed.)
2. **Full-text search and model sources** (P3-20, P3-14, P3-21): a `LIKE` on a `text-fts` field is not rewritten
   to a full-text search (the DDL now follows `FtsDdl`), DB view entities are not built from their rows, and a
   foreign key is recognized only by its computed name.
3. **Error contract** (P6b-01): a failing service body propagates `Err` where Java records the
   error and returns. (The errors text now follows Java's `errorsString`, P6a-23 fixed, and the `post-commit` SECA rules
   of a failed call get an empty result map, P7a-10 fixed.)
4. **View fields** (P5-19, P5-16, P5-18): the entity JSON schema and the data writer's CSV of a view read no fields
   (REST finds and form auto fields now read the aliases), `pq-expression` aliases are not evaluated, and
   `DataDocument.<id>` is not an entity name.

### Open decisions

- **P6b-01** — keep Rust's `Err` on a service-body failure (the Rust tests assert it) as a documented
  redesign, or switch to Java's record-and-return contract. Everything that calls `sync(...).call()`
  depends on the answer. Notes for the decision (2026-10-16):
  - Callers: about 25 non-test call sites of `service::sync`, `call::sync`, `sync_with` and `sync_multi` in
    moqui-core, moqui-host and moqui-browser, and about 90 in tests. Some stop on the `Err` with `?`
    (`entity/query_rest.rs:243`, `entity/field_rw.rs:466`, `user/authz.rs:417`, `service/rest.rs:590,658`,
    `service/spreadsheet.rs:315`, `moqui-browser/src/host.rs:182`); some match it to record or log it
    (`service/jobs.rs:490` records the job's error, `service/eca.rs:206`, `entity/document.rs:970`,
    `notify/message.rs:601`, `service/tools.rs:68`, `service/system_sync/sftp.rs:242`); some read the message
    facade as well (`screen/dispatch.rs:249,264,295`, `search/feed.rs:248`, `service/sso.rs:301`,
    `service/system_sync/message.rs:336`, `moqui-host/src/ssh/shell.rs:163`). Under Java's contract the first two
    groups would need a `has_error()` check, or they go on with an empty result.
  - Tests that assert the `Err` of a body failure: in `service/call.rs`,
    `service_failures_identify_owning_actions_and_rust_origin` (reads the diagnostic from the `Err`),
    `post_commit_rules_run_after_a_failure_only_when_they_run_on_error`,
    `a_runaway_script_ends_at_the_transaction_deadline` and `remote_rest_in_sim_mode_errors_before_a_socket`; and
    `service/elfinder/tests.rs:417`. `script_ports_tests.rs:612-621` takes either. The authentication and semaphore
    failures (`native_components.rs:395`, `held_fail_semaphore_errors`) throw in Java too, before its `try`.
  - Scripts and XML actions: `ec.service.sync()…call()` (`script/runtime/services.rs:266-297`) already turns the
    `Err` into a message-facade error and returns `null`; Java returns the result map, the empty one it started
    with when the runner threw. A `<service-call>` compiles to that call, merges the result only when it is not
    null, and returns on `ec.message.hasError()`, so actions stop as Java's do; only an `out-map` differs (left as it
    was here, `{}` in Java).
  - A switch: `sync_with` returns the empty (or partial) map after `add_error` and the rollback
    (`service/call.rs:458-468`), keeping `Err` for what Java also throws: an authorization failure in the body (Java
    rethrows `ArtifactAuthorizationException`) and the failures before the run (unknown service, authentication,
    semaphore). The script host's `Err` arm would then be unused, the callers in the first two groups above move
    to a `has_error()` check, and the tests above read the message facade.
- **Spreadsheet/print references**: `org.moqui.impl.SpreadsheetServices` and some component Groovy are
  absent from these checkouts, so those comparisons rely on the XML descriptor only.

### Stale code comments (small to-dos)

None open. The last one, the loader's comment that every record uses the entity-auto store path, now says that CSV
rows do not (P4a-02, `entity/load_feed/loader.rs:466-467`).

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
  standalone document's own entity is `#` (`entity/schema.rs:126-240`). The catalog also holds any view entity
  a relationship refers to, so every `$ref` resolves; Java refers to a definition it never writes.
- JSON schema `enum` (P2-07, fixed): the first non-auto-reverse `one` relationship keyed by the field alone,
  titled and to `Enumeration` or `StatusItem`, lists that type's ids by `sequenceNum` (`FieldEnums::of`,
  `entity/schema.rs:260`, which the service OpenAPI also uses, P8a-24); a read that fails is logged and the field
  has no `enum`.
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

Worked 2026-10-16: P4a-01, P4a-02, P4a-05, P4a-08, P4a-09, P4a-10, P4a-11, P4a-12, P4a-13, P4a-14, P4a-15,
P4b-01, P4b-02, P4b-03, P4b-05 and P4b-06 fixed; P4a-04 and P4b-08 moved to
[Accepted differences](#accepted-differences); P4a-16, P4a-17 and P4a-21 found and fixed, P4a-19 and P4a-20 found
and accepted, and P4a-18 (the `list()` half of P4a-10) split off and left open, along the way. The shipped data
(`moqui load types=all` in a copy of `runtime/`: the framework, tools and webroot seed, install and demo files, 1389
records) loads with no failed file or error. Line numbers in this section are as of that change.

### Findings — 4a data loader

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P4a-18 | `script/runtime/tools_host.rs:220-262` (no `list`) | `EntityDataLoaderImpl.groovy:209-217,633-645` | `EntityDataLoader.list()` unported: the values a load would write, as an `EntityList`, with no service called | Low | No |

No 4b (feed / ECA / writer / document / ETL) finding is open.

#### P4a-18 — `EntityDataLoader.list()` not ported

Split from P4a-10 when `checkInfo` was ported. Java's `ListValueHandler` collects each record (nested records through
`addValuesFromPlainMapRecursive`) into an `EntityList` and logs each service element instead of calling it. No screen,
service or component in `runtime/` calls `list()`, and the DataImport screen uses `check`/`checkInfo`, so it is left
open for want of a caller. A port would divert records the way a check does (`entity/load_feed/check.rs`), turn each
`Record` tree into entity values in Java's order, and bind `l.list` in the script data loader.

#### Phase 4 checked and found equivalent

- CSV BLOB cells (P4a-01, fixed): a `binary-very-long` cell is Base64 (the padding optional, as Java's decoder), and
  text that is not Base64 fails the file (`entity/load_feed/csv.rs:156`; `decode_base64`,
  `entity/load_feed/loader.rs:334`).
- CSV write path (P4a-02, fixed): a CSV row gets Java's own primary-key check (`complete_csv_pk`,
  `entity/load_feed/loader.rs:508`): with the fields converted, a missing one-field key is sequenced and a missing key
  of a longer one fails the file. It is then stored as Java's `handleValue` (`store_value`, `:481`): created when
  missing under `onlyCreate`, an upsert under `useTryInsert` (not where the database is `never-try-insert`), and
  otherwise `createOrUpdate`; no entity-auto `fromDate` default, secondary sequence or status-flow check. XML and
  JSON records still use the entity-auto store, as Java's plain-map handler; the comment there now says so (`:466`).
- Bad type text (P4a-05, fixed): `convert_fields` (`entity/load_feed/loader.rs:618`) fails the record with Java's
  "The value [..] is not valid for type .. for field .." when the text is not of the field's type, for XML, JSON and
  CSV, and in a check. Number text is read as a BigDecimal first, as `L10nFacade.parseNumber`, so `118,500` is
  118500 and `1800.0` is a whole 1800 for a Long.
- Empty values (P4a-08, fixed): an empty CSV cell, an empty XML attribute, and a JSON `null` or `""` are null, so a
  store clears the column, and a `defaultValues` entry does not fill a field the record sets empty
  (`entity/load_feed/csv.rs:151`; `entity/load_feed/loader.rs:258`, `:770-784`; `entity/load_feed/json.rs:287`). An
  empty field element is still not a value, as Java's `if (currentFieldValue)`.
- Service names (P4a-09, fixed): XML, CSV and JSON all take any name `isServiceDefined` takes, or a `verb#noun`
  entity-auto name (`data_service_name`, `entity/load_feed/loader.rs:237`; `entity/load_feed/csv.rs:106`;
  `entity/load_feed/json.rs:120`).
- `checkInfo` and `messageNoActionFiles` (P4a-10, fixed): `check_info_with` (`entity/load_feed/check.rs:99`) gives
  Java's diff rows (`entity`, `pk`, `notFound`, `pkComplete`, `location`, with `createValues` or `field`, `value`,
  `dbValue`) and no message for them; the script data loader has `checkInfo()` and `checkInfo(diffInfoList,
  messages)`, which the DataImport screen calls. Each file a check reads is followed by `-- Checked data (N records)
  in {location}` unless `messageNoActionFiles(false)` and the file gave no messages (`check_file`, `:146`); XML, CSV
  and JSON text are named `xmlText`, `csvText`, `jsonText` and get no such line, as Java's.
- Short CSV rows (P4a-11, fixed): a row with fewer columns than the header fails the file
  (`entity/load_feed/csv.rs:143`).
- `options.components` (P4a-12, fixed): `load_entity_data` reads any location it is given; the host's conf
  `load-data` locations and the script loader's gathered conf locations are narrowed with `location_in_components`
  (`entity/load_feed/loader.rs:876`; `moqui-host/src/lib.rs:1257`; `script/runtime/tools_host.rs:1311`).
- Field element text (P4a-13, fixed): the XML reader parses each element with `MNode::parse_text_raw`
  (`moqui-lib/src/node/mod.rs:94`), so a field element's or service parameter's text is kept as written, whitespace
  included, as Java's SAX `characters` (`entity/load_feed/loader.rs:270`).
- Root elements (P4a-14, fixed): a root other than `entity-facade-xml` or `seed-data` is read as an `<entities>` file
  is: only the `<seed-data>` (type `seed`) and `<entity-facade-xml>` (its `type`) elements in it are data, and a file
  with none loads nothing, not an error (`load_embedded_document`, `entity/load_feed/xml.rs:41`).
- Boolean text (P4a-15, fixed): the loader's conversion takes only `true`, in any case, as true (`Boolean.valueOf`,
  `convert_text`, `entity/load_feed/loader.rs:655`); `moqui-lib`'s `to_bool` keeps `ObjectUtilities`' `y`. No shipped
  dictionary type is a Boolean.
- CSV column that is not a field (P4a-16, found and fixed): fails the file, as Java's field lookup does; it was
  dropped (`entity/load_feed/csv.rs:120`).
- The text `null` (P4a-17, found and fixed): is null for any field type, as `FieldInfo.convertFromString`
  (`entity/load_feed/loader.rs:655`).
- A CSV header name given twice (P4a-21, found and fixed): reads the later column, as Java's header map; the first
  was read (`entity/load_feed/csv.rs:94`).
- EECA entity names (P4b-01, fixed): a rule's `entity` is resolved to its full name, and a rule for an entity that is
  not defined is skipped with Java's warning (`EcaIndex::ingest`, `entity/load_feed/eca.rs:180`).
- EECA order (P4b-02, fixed): an entity's rules with no id run first, then those with an id, each in file order
  (`entity/load_feed/eca.rs:214-223`); Java's id'd rules are in `HashMap` order, which file order is one instance of.
- Data document merging (P4b-03, fixed): rows merge by document id only when every primary key field is a field alias
  (`has_all_primary_pks`, `entity/document.rs:60`, `:633`); otherwise each row is a document (`:863`), and its `_id`
  is made, as `makeDocId`, of the document's fields named for the key, so a one-field key that is not a field gives
  no `_id` (`doc_id`, `:127`). Before, the `_id` was always the full key, read from the query's own key aliases.
- XML writer count (P4b-05, fixed): an XML record counts its nested records too, as `plainMapXmlWriter`; JSON and CSV
  count each top record (`data_body`, `entity/query_rest.rs:1555`, `xml_record_count`, `:1583`;
  `write_rows`, `entity/data_writer.rs:472`).
- Writer directories (P4b-06, fixed): `file()` needs its parent directory and `directory()` creates only the last
  one (`mkdir`), else the write is a message error; the zip writes still create the parents, as Java's
  (`entity/data_writer.rs:254`, `:300-315`, `write_new`, `:575`).
- `defaultValues` merged into XML, JSON and CSV records (`entity/load_feed/loader.rs:770-791`).
- Per-location error isolation (log and continue) and one transaction per file (`entity/load_feed/loader.rs:104-134,
  188-199`).
- `ec.entity.getEntityDataDocument()` + `writeDocumentsTo*` bound for the shipped DataDocument screens
  (`script/runtime/tools_host.rs:266-281`).
- XML `binary-very-long` child-element Base64 decode with UTF-8 fallback (see P4a-19 for the trim); XML
  entity/field/relationship parsing, nested-record key inheritance, `<seed-data>`/`entity-facade-xml` type skipping.
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

Worked 2026-10-16: P5-01, P5-11, P5-13 and P5-15 fixed; P5-17 (form auto fields of a view) found and fixed;
P5-16 left open, though a `pq-expression` alias no longer stops the model loading; P5-18 and P5-19 found and left
open. The JSON wire change (P5-11, P5-15) has no consumer in this repository that reads the old text: the browser
code (`runtime/base-component/webroot/screen/webroot/js`, `crates/moqui-browser/js`, `scripts/*.mjs`) parses with
`JSON.parse`/`response.json()`, `MoquiL10n.toInstant` already takes a millis number (the old quoted millis text was
an invalid date to it), the qvt screen data and `crates/moqui-browser` use `moqui_lib::json::to_json`, not this
writer, the `moqui test` specs run in-process, and only some Rust tests matched `"name": value` text (now `"name" :
value`). Line numbers in this section are as of that change.

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P5-16 | `moqui-db/src/model/mod.rs:708-723` | `EntityDefinition.groovy:180,205-210`; `EntityValueBase.java:215-251` | `pq-expression` aliases are not evaluated; such an alias is left out of its view with a logged error (unused in `runtime/`) | Low | No |
| P5-18 | `moqui-db/src/model/mod.rs:432` (`entity`); `entity/document.rs:722-758` | `EntityFacadeImpl.groovy:1110-1111`; `EntityDataDocument.groovy:169-183,204-208` | `DataDocument.<dataDocumentId>` is not an entity name: Java builds the document's dynamic view as an entity definition for finds and form auto fields (unused in `runtime/`) | Low | No |
| P5-19 | `entity/schema.rs:145`; `entity/data_writer.rs:522-538` | `RestSchemaUtil.groovy:144-147`; `EntityDataWriterImpl.groovy:349-359` | The entity JSON schema and the data writer's CSV header and rows of a **view-entity** have no fields; Java uses the aliases (`getAllFieldNames`) | Med | No |

#### P5-16 — `pq-expression` aliases not evaluated

Java leaves an alias with `pq-expression` out of the view's fields and keeps it in `pqExpressionNodeMap`; `get` of
that name evaluates the expression with the row's values, and so does `getMap` (not `getPlainValueMap`, so REST
output has none); form auto fields show it with `default-display="true"`. Before this pass Rust failed the alias's
`entity-alias` check, and since `EntityModel::load` stops at the first entity that does not parse, one such view
stopped the whole model loading. It is now left out of the view with an error naming it, and the rest loads. Left
open: evaluating it needs the value layer to run a script expression (moqui-db has no script runtime, and Java does
it lazily in `get`), a design choice best made with P5-18, the DataDocument entity definitions it was made for; no
view in `runtime/` uses it.

#### P5-18 — `DataDocument.<id>` entity definitions not ported

Java's `getEntityDefinition` answers a name starting with `DataDocument.` with the document's dynamic view
(`makeEntityDefinition`): its aliases carry the fields' `fieldNameAlias`, `functionName` and `defaultDisplay`, and
expression fields become `pq-expression` aliases. A screen can then find on it or give a form-list its auto fields.
Rust builds the same view only inside `entity/document.rs` (`code_view`) to query a document, so the name is not
found. Left open for want of a caller: nothing in `runtime/` names a `DataDocument.` entity. A port would resolve
the name in the facade's model lookup and needs P5-16 for expression fields.

#### P5-19 — Entity schema and data-writer CSV of a view have no fields

The model keeps a view's fields as `aliases` and leaves `fields` empty. `entity/schema.rs` builds a schema's
`properties` from `entity.fields`, and the data writer's CSV header and rows (`entity/data_writer.rs`) from the same,
so a view's schema has only `_entity` and its CSV no columns. Java uses `getAllFieldNames`, the aliases. Found while
fixing P5-01 (the same cause) and left open as it belongs to the schema (P2-12) and writer (Phase 4) code; the fix
is to read `field_names`/`pk_field_names` and the alias types for a view, as `EntitySnap` and the form shape now do.
The CSV header with `tableColumnNames` would also need a column name for an alias.

#### Phase 5 checked and found equivalent

- REST find of a view-entity (P5-01, fixed): `EntitySnap` takes `field_names`/`pk_field_names`, the aliases for a
  view (`entity/query_rest.rs:282-311`), so a view row renders its alias values (`render`, `:737-747`), and a
  complete primary key of aliases, from the path or the parameters, finds one row as Java's `containsPrimaryKey`
  (`find_rest`, `:478`). The identity and secret stripping (`sensitive::strip_user_account_secrets`) goes by key at
  any depth, so view rows get it too. The query-REST CSV writer (`render_csv`) also gets a view's aliases.
- REST JSON dates (P5-11, fixed): `encode_json` (`entity/query_rest.rs:1739-1781`), which every web JSON response
  and JSON error goes through (`web/mod.rs:514`, `:525`, `:1263`), writes a `Timestamp` or `Date` as an unquoted
  epoch-millis number and a `Time` as `"HH:mm:ss"`, as Jackson with `WRITE_DATES_AS_TIMESTAMPS` and its
  `SqlTimeSerializer` (checked against jackson-databind 2.22.2). Before, all three were quoted millis text. A
  `Date` is midnight UTC, as everywhere in this port; Java's is midnight in the JVM zone. The data writer's JSON
  files are unchanged.
- REST JSON layout (P5-15, fixed): the same writer indents as Jackson's `DefaultPrettyPrinter`
  (`INDENT_OUTPUT`): object entries on their own lines two spaces deeper per enclosing object, `"name" : value`,
  arrays on one line as `[ a, b ]`, and `{ }`/`[ ]` when empty.
- `default-display` (P5-13, fixed): the model keeps it on an alias (`moqui-db/src/model/view.rs:134`, `:279`), as
  does a REST dynamic view's alias spec (`entity/query_rest.rs:1243`), and a form-list's auto fields honor it as
  `ScreenForm.addEntityFields` (`screen/form.rs:934-943`). As in Java, a dynamic view never reaches form auto fields,
  so on a REST dynamic view it has no effect.
- Form auto fields of a view-entity (P5-17, found and fixed): `read_entity_shape` (`screen/form.rs:862-915`) read
  `entity.fields`, so `auto-fields-entity` of a view (the Tools entity data screens with a view selected) gave no
  fields. A view's fields are now its aliases, with their types (`text-long` when none, as Java's `fi.type ?:
  "text-long"`) and primary-key flags.
- Identity-admin/secret-config read and write bans and `stripUserAccountSecrets` on every response
  (`entity/query_rest.rs:185-210`, `entity/sensitive.rs` mirrors `WebUtilities`).
- REST writes go through entity-auto (`write_rest`, `entity/query_rest.rs:233-254`), so status-flow checks
  and `oldStatusId`/`statusChanged` apply.
- REST field conditions via `search_form_map`: `_ic`, `_period`/`_poffset`/`_pdate`, date-only `_thru`
  padding, aggregate aliases to HAVING, unknown `_op` adds nothing.
- Multi-step relationship paths (`entity/query_rest.rs:415-436`); path→entity+PK resolution,
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

Worked 2026-10-16: P6a-01, P6a-03, P6a-04, P6a-06, P6a-07, P6a-08, P6a-09, P6a-10, P6a-11, P6a-13, P6a-14, P6a-15,
P6a-16, P6a-17, P6b-10, P6b-11 and P6b-12 fixed; P6a-12 and P6b-14 moved to [Accepted
differences](#accepted-differences); P6b-01 left open as a decision, with notes under [Open
decisions](#open-decisions). Found along the way: P6a-18, P6a-21 and P6a-22 (fixed), P6a-19 and P6a-20 (the parts of
P6a-13 and P6a-08 that stay, accepted) and P6a-23 (fixed: the errors text that P6a-14's messages left without a
field). Two changes reach shipped services: parameters of `implements` now come first (an order change only: a dump of
the 37 `runtime/` services with `implements` gives the same name, type, `required`, checks and default for each
parameter as before), and an `auto-parameters` field merges with a parameter of the same name, so
`EntityServices.create#DataDocumentField` and `update#DataDocumentField`, whose `fieldPath required="true"` comes
before `<auto-parameters include="nonpk"/>`, no longer require `fieldPath`, as in Java, where the later
`auto-parameters` sets `required="false"`; in the other 7 such services the merged parameter keeps the type,
`required`, default and checks of the two before. Line numbers in this section are as of that change.

### Findings — 6a definition / validation / info

None open.

### Findings — 6b call / semaphore

| ID | Rust location | Java location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P6b-01 | `service/call.rs:458-468` | `ServiceCallSyncImpl.java:320-356,391` | A service-body failure is returned as `Err` after `add_error`; Java catches it, records the message/cause chain, rolls back, and returns the result map (only `ArtifactAuthorizationException` rethrown) | High | Decision pending |

#### P6b-01 — A service-body failure throws in Rust but returns a map in Java

Java's `callSingle` wraps the runner in `try { … } catch (Throwable t)`, adds `t`'s message and cause
chain to the message facade, rolls back, then returns the result map. Rust maps any `Err` from
`execute` to `add_error(...)` and then returns `Err(err)`. A failing service body therefore makes Rust
`sync(...).call()` return `Err` where Java returns normally with the error in the message facade. The
Rust tests assert this `Err`; see [Open decisions](#open-decisions).

#### Phase 6 checked and found equivalent

- `validate="false"` (P6a-01, fixed): skips the `required` check, the `allow-html` check and the declared checks, as
  Java's `nestedParameterClean`, and still converts (`clean_param`, `service/validate.rs:124-170`); unknown
  parameters are kept and nested maps not trimmed, as before.
- Number `format` (P6a-03, fixed): a number parameter with a `format` is read by that `DecimalFormat` pattern for
  the session locale, as `BigDecimalValidator.validate(text, pattern, locale)` (`convert_text`,
  `service/validate.rs:436`; `l10n::parse_number_format`, `l10n.rs:365`; `format::parse_decimal_pattern`,
  `moqui-lib/src/format.rs:330`): its prefix and suffix or negative sub-pattern, grouping only when the pattern
  groups, `%` and per-mille scaled back. Text it does not read is Java's error naming the format.
- Known service names (P6a-04, fixed): `known_service_names` (`service/info.rs:27`) adds to the registered services
  the services of the `service-file` locations and of every service file under each component's `service`
  directory, read but not registered, as `getKnownServiceNames` (`service_names_in_files`,
  `service/define.rs:639-702`); a file that does not parse is logged and skipped. Like Java, the framework's own
  `service` directory is listed only through `service-file`.
- `implements` merge (P6a-06, fixed): the implemented services' parameters come first, then the service's own are
  merged in by name, as `ServiceDefinition`'s `mergeParameter`: an own parameter sets only the attributes it has (a
  parameter that adds a check keeps the interface's `type` and `required`), its checks are added and its nested
  parameters merged (`merge_param`, `service/define.rs:162-202`; `resolve_one`/`implemented`,
  `service/define.rs:1363-1455`). `implements required` also sets the `required` of an `auto-parameters` it copies.
  A `required="disabled"` implemented parameter is still left out (P6a-12), and of two implemented services with a
  parameter of the same name the first one's is kept (Java appends both and the later wins its map).
- `time-range` (P6a-07, fixed): text is read by the `format` and a value that does not read is not checked; the
  bounds are inclusive, and `now` is the effective time written and read back by the format, so `after="now"` with
  `MM/yyyy` takes this month (`time_range`, `service/validate.rs:735-775`; `bound_millis`, `:912-927`).
- Character classes (P6a-08, fixed): `text-letters` is Unicode alphabetic in the Basic Multilingual Plane, as
  `Character.isLetter` of a UTF-16 unit, `text-digits` the decimal digits there, as `Character.isDigit`
  (`is_java_digit`, `service/validate.rs:779`), and `text-email` follows `EmailValidator`'s user and host rules
  (`is_email`, `:794-875`); what stays different is P6a-20.
- SECA rule count (P6a-09, fixed): `getSecaRuleCount` counts a pattern rule once per known service it matches
  (`seca_rule_count`, `service/info.rs:39`; `SecaRule::service_count`, `service/seca.rs:299-310`). The startup log
  keeps Java's "SECA rules active" count of rules (`seca_rules_active`, `service/info.rs:48`), and a SECA load reads
  the known names at most once (`service_defined`, `service/seca.rs:282-297`).
- `field-name` (P6a-10, fixed): an `auto-parameters` field sets `field-name` (`auto_fields`,
  `service/validate.rs:424`).
- Empty `default-value` (P6a-11, fixed): no default, as `ParameterInfo` (`parse_parameter`,
  `service/define.rs:1237`); an empty `default-value` on an own parameter still clears an implemented one's.
- Each failed check (P6a-13, fixed): every check of a parameter that fails is its own validation error, and an HTML
  error does not stop the checks (`clean_param`, `service/validate.rs:161-170`; `apply_check`, `:576-731`); `val-and`
  reports each child; `val-or`/`val-not` are P6a-19.
- Message text (P6a-14, fixed): Java's localized messages: `Field cannot be empty`, `Value entered ({0}) could not be
  converted to a {1}` (with `using format [..]`), the HTML message, and each check's `Value entered …` text or its own
  `message` expanded with `pv`, `regexp`, `min`/`max` (`apply_check`, `service/validate.rs:576-731`). The field and
  service are on the validation error, not in its text; the errors text adds them (P6a-23).
- Errors text (P6a-23, found and fixed): the JSON `errors` of a REST or screen response (`web/mod.rs:778-781`,
  `screen/dispatch.rs:372-375`) and a script's `ec.message.errorsString` (`script/runtime/facades.rs:350-352`) are
  Java's `getErrorsString`: the errors, then each validation error as `toStringPretty` writes it, its message, `(`,
  `for field`, ` on form` and ` of service` with the pretty names, and no closing parenthesis, as Java
  (`MessageFacade::errors_text`, `ValidationError::to_string_pretty`, `context/message.rs:44-67`, `:132-156`); a
  validation error's message, which the errors list also holds, is written once. The lines are joined without
  Java's newline after the last. The browser still leaves a placed field error out of its toasts: `placeFieldErrors`
  in `WebrootVue.qvt.js` (`moqui.validationErrorText`) and `moqui-html.js` match the line in either form. Before,
  the line was the message alone, which since P6a-14 named no field.
- File override (P6a-15, fixed): reading a service file under a framework or component `service` directory reads
  the files of the same path in the others, and a service is defined by the last of them that has it, in the order
  framework then components, as `findServiceNode`; this holds for the `service-file` files read at startup too
  (`load_file`, `service/define.rs:844-946`; `service_roots`, `:620-634`). A file that does not parse loses only its
  own services.
- Numeric attributes (P6a-16, fixed; P6a-22, found and fixed): a `transaction-timeout`, `semaphore-timeout`,
  `semaphore-sleep` or `semaphore-ignore` that is not a whole number fails that service with a registration failure,
  where Java's `Integer.valueOf`/`Long.parseLong` fails its definition; before, the first was ignored and the others
  took their defaults (`service/define.rs:1006-1021`; `timeout_of`, `:1142`; `seconds_attr`, `:1164`).
- Implicit entity-auto (P6a-17, fixed): the implicit definition does not clean its parameters
  (`validate_input: false`, `service/define.rs:263-269`), so SECA rules and the runner get the map as it came, as
  `runImplicitEntityAuto`; the runner sets only the entity's fields, as `setFields` (`run_entity`,
  `service/call.rs:1001`).
- Declared entity-auto (P6a-18, found and fixed): a declared `type="entity-auto"` service kept every entity field
  and every map or list parameter beyond its declared parameters; it now keeps only those declared, as any
  validating service, so `create#UomConversion` (`auto-parameters include="nonpk"`) no longer takes a caller's
  `uomConversionId` (`clean`, `service/validate.rs:78-80`).
- `auto-parameters` merge (P6a-21, found and fixed): an `auto-parameters` field and a parameter of the same name were
  two parameters, each cleaned in turn, so an untyped parameter after a `Timestamp` field converted the value back to
  text; they are now one, merged by name in declaration order, as `mergeAutoParameters` (`expand_params`,
  `service/validate.rs:321-341`; the same in `typegen/services.rs:34-61`).
- Semaphore scope (P6b-10, fixed): an empty `semaphore-parameter` is `_NA_` and an empty value is `""`; only a missing
  or null value is `_NULL_` (`scope_value`, `service/semaphore.rs:237-245`).
- Semaphore text (P6b-11, fixed): Java's "An instance of service semaphore … is already running (thread [..], locked
  at ..) and it is setup to fail on semaphore conflict." and the wait form with its seconds, with the holder read
  from the row (`take`, `service/semaphore.rs:33-77`; `try_lock`, `:151-216`).
- Status transition text (P6b-12, fixed): `StatusFlowTransitionNotFoundTemplate` localized and expanded with the
  status ids and names, its default text when the seed data is not loaded (`status_transition_message`,
  `service/call.rs:1228-1243`).
- Nested `<parameter>` children parsed and validated (`service/define.rs:1245`, `service/validate.rs:84-118`).
- Implicit entity-auto skips authentication (`Authenticate::Off`, `service/define.rs:263-265`); an `AT_SERVICE`
  frame is still pushed but with `requires_authz=false`.
- Early "errors already in the facade" abort with `ErrorsAside` push/pop, and the rollback-only guard
  (`service/call.rs:330-350,484-498`).
- `verb_action` maps `get`/`search` to `view` (`service/call.rs:846`); entity-auto `delete` requires a full PK
  or `*` wildcard (`delete_entity`, `service/call.rs:1069-1112`) and returns an empty map.
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

Worked 2026-10-16: P7a-03, P7a-06, P7a-10, P7a-11, P7a-12, P7b-01, P7b-02, P7b-05, P7b-07, P7b-08, P7b-10, P7b-11,
P7b-12, P7b-13, P7b-14 and P7b-15 fixed; P7a-05 fixed for `ALL_USERS` membership, and its permission half removed as
invalid: Java's `hasPermission` reads the `UserPermissionCheck` view, a join of the user's own `UserGroupMember` rows,
so a permission granted to `ALL_USERS` counts only for a user with a membership row for it, as Rust already did (a test
now holds both). P7a-07 and P7a-08 moved to [Accepted differences](#accepted-differences). Found along the way: P7a-13,
P7a-14, P7a-16 and P7b-17 (fixed), P7a-15 and P7b-16 (the parts of P7a-12 and P7b-14 that stay, accepted) and P7b-18
(accepted). Changes a deployment may see: an async call (`async_call`, `call_future`, `start_future`, a script's
`ec.service.async()`) of an unknown service, or by a caller not authorized for the service, is now an error where it is
made (in a script, an error message, as the script host reports a failed call), and its run on a worker has
authorization disabled; an async call in a sim session is skipped; shutdown runs the
queued async calls for up to 30 seconds; and the first scheduled job pass is 120 seconds after start. No `runtime/`
component has a SECA rule, so the live `parameters` (P7a-11, P7a-13) and the SECA phase order (P7a-03, P7a-16) change
only rules a deployment adds. The scheduler tests run their passes with `ServicePools::tick()`, not the thread, so none
waits on the initial delay and no test override was needed. Line numbers in this section are as of that change.

### Findings — 7a ECA / SECA / email / security

None open.

### Findings — 7b pools / async jobs / scheduler

None open.

#### Phase 7 checked and found equivalent

- Implicit entity-auto SECA phases (P7a-03, fixed): an implicit entity-auto call runs its `pre-validate` rules a second
  time where Java's `runImplicitEntityAuto` does, after the authentication check, and runs no `pre-auth` rules, which
  Java never reaches for it (`sync_with`, `service/call.rs:403-410`).
- `pre-auth` order (P7a-16, found and fixed): the `pre-auth` rules ran right after validation, before the
  `authUsername` login and the authentication check; they now run after both, as Java's, so they see the user the call
  logged in and do not run for a call that fails either (`service/call.rs:403-410`).
- `ALL_USERS` (P7a-05, fixed): `user_in_group` is true for `ALL_USERS` with no lookup and no user, as `isInGroupById`
  (`service/security_checks.rs:142-146`); a permission granted to `ALL_USERS` counts only with a membership row for
  it, as Java's `UserPermissionCheck` view (`user_has_permission`, `:162-164`).
- Membership dates (P7a-14, found and fixed): a membership or grant whose `thruDate` is now was closed; both ends are
  now inclusive, as `filterByDate` (`membership_open`, `service/security_checks.rs:219-234`).
- EECA and EMECA conditions (P7a-06, fixed): only the first child of `<condition>` is the condition, as
  `EntityEcaRule` and `EmailEcaRule` (`first_condition`, `service/rule_actions.rs:33-40`; used at
  `entity/load_feed/eca.rs:198` and `service/email_eca.rs:34`); SECA already did.
- `post-commit` results (P7a-10, fixed): the `post-commit` rules of a call that failed get an empty result map, Java's
  `result` as it began, instead of none (`service/call.rs:470-477`). The call still returns `Err` (P6b-01).
- Live `parameters` and `results` (P7a-11, fixed; P7a-13, found and fixed): after each rule, the `parameters` and
  `results` maps in its frame are read back into the call's (`run_seca_rules`, `read_back`, `service/seca.rs:47-102`), so a
  `pre-validate` rule changes what is validated (the input is copied only when there are such rules,
  `service/call.rs:352-356`), a `pre-auth` or `pre-service` rule what the service runs with, and a `post-service` or
  `post-commit` rule what is returned. A rule that sets a plain name still sets it only in its own frame, as Java's.
- `tx-rollback` registration (P7a-12, fixed): the `tx-rollback` rules are registered once the runner ran, with its
  results (empty when it failed), as Java's `finally` after `runService`, so a call stopped before it runs registers
  none (`service/call.rs:436-442`). The `tx-commit` rules are still registered after the `post-service` rules, so a write there
  can still register the feed hook first, but with the parameters and results as they were before those rules, the
  copies Java registers (`service/call.rs:443-444`); they run only when the transaction commits, so Java registering
  them on an error as well changes nothing. Pattern rules are P7a-15.
- Scheduler (P7b-01, P7b-02, fixed): `scheduled-job-check-time="0"` starts no job runner, with Java's warning: no job
  passes run, also from `tick()`, and `runner_stats` is `None`, so the tools screens show "No Service Job Runner
  active" (`settings`, `service/pool.rs:765-778`; `runner_stats`, `:413-419`; `tick`, `:567-581`). The first job pass
  is 120 seconds after start, then one every check time, as `scheduleAtFixedRate(jobRunner, 120, rate)`; the tick
  hooks, this port's own (the cluster heartbeat), keep running every interval from the start (`Clock`,
  `service/schedule.rs:66-96`).
- Shutdown (P7b-05, fixed): as `workerPool.shutdown()` and `awaitTermination(30s)`, the pool takes no new work, the
  scheduler and job workers end, and the service workers run the queued calls for up to 30 seconds; what is left then
  fails as before, with Java's "NOT YET terminated" warning (`shutdown`, `service/pool.rs:616-635`; `drain`,
  `:1137-1160`; `next_work`, `:921-939`). On one of the pool's own workers it does not wait. Queued job runs are
  abandoned at once, as before (Java never shuts its `jobWorkerPool` down).
- Async queue-time check (P7b-07, P7b-08, fixed): before an async call is queued, as the caller, an unknown or
  interface service is an error, the parameters are converted and validated, the service is pushed and popped to
  check authorization (no artifact hit), and `authUsername` is added, as Java's `validateCall`; the worker logs in the
  caller and runs the call with authorization disabled, as `AsyncServiceInfo` (`prepare_async`,
  `service/call.rs:256-313`; `submit`, `service/pool.rs:529-565`; `submit_future`, `:672-701`; `invoke`, `:998-1015`;
  `call_now`, `:1019-1042`). Input that fails validation queues nothing (P7b-18).
- Async in a sim session (P7b-17, found and fixed): an async call in a sim session ran on a worker, outside the sim,
  against the real database; it is now skipped with Java's log line, and a future gets `simSkipped` and `serviceName`,
  as `ServiceCallAsyncImpl` (`prepare_async`, `service/call.rs:275-281`).
- A run that cannot be started (P7b-10, fixed): once the pass has taken the lock, a run whose parameters cannot be
  read or that the job queue does not take is a failed run, as Java's `ScheduledJobRunner`: the lock is cleared and
  keeps this pass's run time, and the `ServiceJobRun` row records the error with start and end times; before, the row
  was deleted, and a failure reading the parameters left the lock held (`dispatch`, `service/jobs.rs:261-322`).
- Job dates and `paused` (P7b-11, P7b-12, fixed): a job whose `thruDate` is now is still in date, as `filterByDate`
  (`within_dates`, `service/jobs.rs:822-827`), and a `paused` that is not null or `N`, an empty one too, pauses the job
  (`paused_ok`, `:829-835`).
- Queue default (P7b-13, fixed): with no `worker-queue`, the queue holds 65536, Java's code default
  (`service/pool.rs:44-45`); `MoquiDefaultConf.xml` sets 65535 in both.
- Run host (P7b-14, fixed): a job run records `hostAddress` and `hostName`, from the address Java's
  `InetAddress.getLocalHost()` would give (`server_address`, `context/hits.rs:520`), with `runThread` and `startTime`
  (`run_job`, `service/jobs.rs:463-475`). The thread's name is P7b-16.
- Runner counters (P7b-15, fixed): a pass that failed or panicked still counts in the runner's counters, as Java's
  (`run_scheduled`, `service/jobs.rs:89-113`).
- SECA/EMECA discovery per component, id-overwrite, priority ordering; SECA rules for undefined
  services skipped at load (`service/eca_rules.rs:105-113`, `service/seca.rs:282-297`); SECA uses only the first
  `<condition>` child (`service/seca.rs:209-213`).
- Runtime name matching; `run-on-error` gating for SECA; phase set/ordering and tx hooks; tx rules run
  on a worker in a new transaction and skip sim sessions (`service/call.rs:607-652`); condition-false skip.
  SECA context carries `parameters`/`results`, the call's own (P7a-11).
- Action dispatch `service-call`/`entity-*`/`script`/`in-map="context"`; email ECA context fields.
- Security-check logic: empty-id, `authzDisabled`, privileged/sealed membership, date-ranged lookups,
  error strings.
- Registered callbacks run (`call_callbacks`, `service/call.rs:469`).
- Jobs run on a separate job queue sized by `job-pool-core` (`service/pool.rs:260-263,867-894`); job topic
  notification plus `ServiceJobError` to the user and `ServiceJobUser` recipients (`service/jobs.rs:602-671`).
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

Worked 2026-10-16 (8a only; 8b and 8c are for a later round): P8a-03, P8a-04, P8a-06, P8a-08, P8a-11, P8a-12,
P8a-13, P8a-15, P8a-18, P8a-19, P8a-20, P8a-21, P8a-22 and P8a-24 fixed; P8a-17 moved to
[Accepted differences](#accepted-differences). Found along the way: P8a-25, P8a-27, P8a-28 and P8a-29 (the parts of
P8a-21, P8a-20, P8a-24 and P8a-11 that stay, accepted) and P8a-26 (open). For P8a-08 the shipped callers were checked
first: the only `remote-rest` service in the tree, the browser smoke `get#Echo`, is answered with a JSON object, so
none depends on the lenient body handling. Changes a deployment may see: a `/rest/s1` service result has no `null`
out-parameters, no `null` fields one map down, and no `UserAccount` password fields; a POST with
`X-HTTP-Method-Override` reaches the overriding method on `/rest/s1` and is authorized for it; a `remote-rest` call
whose 2xx body is not a JSON object or array, an empty one too, now fails; its GET query carries empty and list
values; a `method="HEAD"` or `"OPTIONS"` is sent as that method (and fails with parameters, which it cannot carry as
a body); a `<method type="GET">` (upper case, outside the XSD) is no longer served; and the service OpenAPI
(`/rest/service.swagger/<root>`) now documents each service method's parameters and response. Its entity methods
still carry only a summary, as before (Swagger and RAML are out of scope, below). Line numbers in this section are as
of that change.

### Findings — 8a REST / remote REST / tools / print

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|----|---|---|---|---|---|
| P8a-26 | `service/rest.rs:1011` (`param_schema`) | `RestSchemaUtil.groovy:479-480`; `RestApi.groovy:272,290` | The service OpenAPI's parameter schemas have no `description`; Java takes the parameter's `<description>` text | Low | No |

#### P8a-26 — Parameter descriptions in the service OpenAPI

`ParamDef` keeps no `<description>` text, so the parameter schemas P8a-24 added have none. Left open: carrying it
means a new `ParamDef` field and a decision on how `merge_param` (`service/define.rs:176`) merges it for an
`implements` or a redeclared parameter, which Java's `mergeParameter` does by copying the child elements; that is
service-definition work beyond this REST round.

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

#### P8b-03 — Email address formatting

The port's `save#EcaEmailMessage` stores `toAddresses` (and cc/bcc) as `join(', ')`, where Java uses
Groovy `List.toString()` (`[a, b]`), so stored values differ for anything that reads them back.

#### Phase 8 checked and found equivalent

- Service results stripped of secrets (P8a-03, fixed): a `/rest/s1` service result goes through
  `strip_user_account_secrets`, as Java's `RestResult`, so the `UserAccount` password fields are left out of every map
  in it at any depth: an out-parameter that is an account, a list of them, or one nested further (`run_service`,
  `service/rest.rs:585-594`).
- Method override (P8a-04, fixed): `/rest/s1` takes `X-HTTP-Method-Override` on a POST as `/rest/e1` does, and the
  method it names is the one the path frames are authorized for (`AT_REST_PATH` view, create, update or delete) as
  well as the one that runs, as Java's `effectiveRequestMethod` in both `getActionFromMethod` and `runByMethod`
  (`effective_verb`, `screen/dispatch.rs:333-345`; used at `:87`, `:155`). An override cannot reach a method the
  user may not run: a view-only user's POST with `PUT` is refused at the path.
- Resource description (P8a-06, fixed): read from the `description` attribute, as written in the real `*.rest.xml` and
  the XSD, as Java (`service/rest.rs:356`); the tools dashboard's tooltip and child links show it.
- Remote-REST body (P8a-08, fixed): a 2xx body that is a JSON object is the result and an array is `response`; any
  other, an empty body too, fails the call with Java's "Error parsing JSON response from request to ...", the URL
  redacted (`response_map`, `service/remote.rs:174-183`; `:100-101`).
- Result nulls (P8a-11, fixed): as `nestedRemoveNullsFromResultMap`, the null out-parameters, and the null fields of
  each map out-parameter and of the maps that lead a list, are left out (`remove_result_nulls`,
  `service/rest.rs:596-615`); the plain-map part that stays is P8a-29.
- Remote-REST error text (P8a-12, fixed): "Remote service error 503: Service Unavailable", with the standard reason
  phrase of the status (`service/remote.rs:92`; `reason_phrase`, `outbound.rs:183-216`, shared with the script
  `rest()` host, which already used it); the transport does not pass on the server's own phrase.
- GET query (P8a-13, fixed): as `parametersMapToString`, every parameter but a null one, an empty value as `name=` and
  a list as its items joined by commas (`encode_query`, `service/remote.rs:149-160`). Java's stray `&` for a null is
  not copied, and a space is `%20` where `URLEncoder` writes `+`, which servers read the same.
- HEAD and OPTIONS (P8a-15, fixed): sent as themselves, as Java's `RestClient.Method`, where they fell back to POST
  (`outbound.rs:28-72`; `moqui-lib/src/http.rs:57-107`); with parameters, which they cannot carry as a body, the call
  fails before it is sent, as `RestClient.text` refuses one (`service/remote.rs:71-75`). The script `rest()` host
  takes them too.
- `require-authentication=""` (P8a-18, fixed): an empty value inherits the parent's, as Java's `?:`, on a resource,
  an id and a method (`service/rest.rs:311-315`, `:368-371`).
- Method `type` (P8a-19, fixed): methods are keyed by `type` as written, as Java's `methodMap`, so a `type="GET"`,
  outside the XSD's lower-case list, is never matched (`service/rest.rs:326-344`).
- A method that does not load (P8a-20, fixed): an operation other than `service` or `entity` is logged and that method
  left out, and the rest of the file is served, as Java registers no handler for it (`service/rest.rs:326-344`); what
  else is left out where Java does otherwise is P8a-27.
- DataDocument notifications (P8a-21, fixed): the `DataDocument` and its links are read through the entity cache, as
  `useCache(true)`, and user ids are looked for in the maps of a list but not in a list in a list, as
  `findAllFieldsNestedMap` (`find_nested`, `service/tools.rs:615-632`; `:669-687`). The empty id is P8a-25.
- Tools dashboard counts (P8a-22, fixed): each resource has `childPaths`, Java's count of the resources and ids under
  it, beside `childMethods` (`path_count`, `service/rest.rs:163-169`). Java's `methodMap` of handler objects is read
  only by its LLM browse tool (out of scope) and is not built.
- Service OpenAPI parameters (P8a-24, fixed): a service method in `/rest/service.swagger/<root>` has its path
  parameters, its other in-parameters as query parameters of a method with no body (not objects or arrays) or as the
  JSON `requestBody` of a `post`, `put` or `patch`, and its out-parameters as the `200` response, each with Java's
  `getJsonType`/`getJsonFormat`, its default, nested parameters, and the `enum` Java's `addParameterEnums` gives a
  parameter with `entity-name` and `field-name`, through P2-07's `FieldEnums` (`service_parameters`,
  `service/rest.rs:900-979`; `param_schema`, `:1009-1047`; `parameter_enums`, `:1049-1067`). The schemas are inline
  in OpenAPI 3.1 form where Java's Swagger 2 refers to `definitions`; a `required="disabled"` parameter is left out,
  as validation drops it (P6a-12). P8a-26 and P8a-28 are what stays.
- 8a: identity-admin read ban and identity/secret write bans with UserAccount input stripping
  (`service/rest.rs:641-656`); entity results stripped and returned without `_entity` (`service/rest.rs:664,682-692`;
  master path keeps it, as Java); `count` via `search_form_map` with `X-Total-Count`; remote-REST
  `method` attribute honored (`service/call.rs:886-894`); `rewrite#EntityEncryptedFields` logs and skips
  records that won't decrypt or update (`service/tools.rs:785-807`).
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
| P2-12 | entity schema | JSON schema envelope redesigned: `$defs`/draft 2020-12, PK `required`, keyed by full name vs Java `definitions`/`anyOf`/pagination, draft-04, `_entity`+`default`, short-or-full name (`entity/schema.rs:27,86-125`) | Deliberate modernization; relationships and `enum` now follow Java inside it (P2-06/P2-07 fixed) |
| P2-14 | search form | `_not` with `_ic` on `equals` compares ignoring case; Java throws, since `ignoreCase()` on the `orNull` join condition is unsupported (`entity/search_form.rs:269-272`) | A search a user can type should not fail; comment at the site |
| P3-06 | datasource / DDL | No table, constraint or index name is schema-qualified: Postgres sets the session `search_path` to `schema-name` (`moqui-host/src/db/entity_db.rs:202-207`), `use-schema-for-all` (only the DB2 profiles) is unread, and a MySQL `schema-name` is logged and not used (`entity/datasource.rs:222-229`) | Same placement on Postgres without qualifying each statement; no DB2 driver; on MySQL Java's `schema.TABLE` is another database, which the jdbc-uri names. `use-schemas="false"` is now honored (no search path, `entity/datasource.rs:178-184`); comments at both sites |
| P3-11 | DDL | `fk-style="name_fk"` (the MySQL profiles) is not followed: always `ADD CONSTRAINT name FOREIGN KEY` (`moqui-db/src/sql/statement.rs:562-571`) | MySQL 8.0.16 and later ignore the `name_fk` name and call the key `{table}_ibfk_N`, so the check that goes by name (P3-21) would add it again on every check; doc comment at `create_foreign_key` |
| P3b-10 | feed cache | Feed-info cache invalidated on write vs Java 900s TTL (`entity/cache.rs:838-845`) | Rust is more current |
| P4a-04 | data loader | A JSON `binary-very-long` string is Base64-decoded (text that is not Base64 is its UTF-8 bytes); Java's JSON reader stores the text's UTF-8 bytes (`entity/load_feed/json.rs:290-292`) | This port's data writer writes bytes in JSON as Base64, so its writer and loader round-trip; Java's JSON writer has no text form for bytes (`JsonBuilder` of a `SerialBlob`), so its pair never did. Comment at the site |
| P4a-19 | data loader | XML and JSON `binary-very-long` text is trimmed before the Base64 decode; Java decodes it as it is, so Base64 with whitespace around it becomes the text's UTF-8 bytes (`entity/load_feed/loader.rs:327-331`) | Base64 laid out on lines of its own decodes to the bytes meant; Java's writer puts it in CDATA with no whitespace, so both read that the same. Doc comment at `decode_binary` |
| P4a-20 | data loader | An empty CSV header cell (a trailing delimiter) is no column; Java's field lookup fails the file on it (`entity/load_feed/csv.rs:94-100`) | A trailing delimiter is no reason to fail a file; a non-empty name that is not a field still fails it (P4a-16). Comment at the site |
| P4b-07 | data feed | `get#DataFeedDocuments` honors `thruUpdateStamp`; Java always passes null | Bug fix; comment at runtime `EntityServices.xml:35-36` |
| P4b-08 | ETL | `useTryInsert` stores with `createOrUpdate` where the database is `never-try-insert` (Postgres); Java's ETL loader tries the insert anyway (`entity/etl.rs:92-101`) | Safer: on Postgres the failed insert aborts the transaction, so Java's update after it fails; the data loader's `handleValue` has the same guard. Comment at the site |
| P4b-09 | data writer | `isoDateTime(true)` formats dates in JSON and XML; Java ignores it (`TODO`) (`entity/query_rest.rs:1550-1582,1715-1732`) | Implements Java's TODO |
| P5-14 | entity REST | Request `pageSize` capped at `entity_rest_max_page_size` (1000); Java caps nothing | Documented `entity/query_rest.rs:532-551` |
| P6a-05 | service tools | Service detail view is a rebuilt node (`method` null, no `groovyString`, some attributes dropped) | Documented `service/info.rs:194-195` |
| P6a-12 | service validation | A `required="disabled"` parameter is skipped, so validating drops a value passed for it, and an implemented one is not copied (`service/validate.rs:48-52`, `service/define.rs:1434-1441`); Java only makes it not required and still converts the value | The XSD says it behaves "as if the parameter did not exist", for overriding an earlier parameter; dropping it keeps an undeclared value from the service. Comment at the site |
| P6a-19 | service validation | `val-or` passes when one child passes and `val-not` fails when one passes, with a message (`service/validate.rs:704-723`); Java's `validateParameterSingle` adds each failing child's error even when the `val-or` passes, and adds none for a failing `val-not` | Java's makes a `val-or` with any failing branch fail the call and a failing `val-not` pass it; unused in `runtime/`. Comment at the site |
| P6a-20 | service validation | `text-email` checks the top-level domain's form (letters first, two or more long), not the IANA list, and a Unicode host is not valid; `text-letters` takes Unicode alphabetic characters, a little more than `Character.isLetter` (combining vowel signs, letter numbers) (`service/validate.rs:684-688`, `:789-875`) | No list of top-level domains to keep current and no IDN library in moqui-core; doc comment at `is_email` |
| P6b-08 | semaphore | Row cleared only while `lockThread` equals Rust's owner label | Documented `service/semaphore.rs:82-83,247-248` |
| P6b-14 | service call | A call returns only the declared out-parameters (all when none are declared), and inline/script bodies have no `result` map (`service/call.rs:951-968`); Java returns the runner's map, the `result` map's entries included | Keeps a body's other values from REST and remote callers; a ported script uses `result` as a name of its own (`SearchServices.xml`). Doc comment at `project` |
| P7a-01 | EECA | Security-check EECAs run even when the message facade already has errors (Java skips them) | Stricter by design; `service/eca.rs:74-75`, test at `service/eca.rs:327` |
| P7a-04 | security | `SSH_SHELL` sealed in Rust conf/constants | Rust-only artifact (`service/security_checks.rs:31-32`, `MoquiDefaultConf.xml:63-64`) |
| P7a-07 | rule conditions | A rule condition is JavaScript truth, as an `<if>` in the actions: an empty list or map is true (`eval_condition`, `script/runtime.rs:574-579`); Java's `checkCondition` takes Groovy truth (`as Boolean`), where it is false | The action language is JavaScript: a rule condition and an `<if>` agree, converted Groovy tests a list's length (`groovy_convert`), and the linter flags collection truth (`lint_hazards`). Doc comment at `eval_condition` |
| P7a-08 | SECA | A pattern rule is matched against each called name, once per name (`SecaIndex`, `service/seca.rs:118-121`); Java expands it at load to the services known then, so it never runs for one it did not know, as an implicit entity-auto service | The same rules run for every service known at load; a pattern written for a name also covers that name when the service comes later. Comment at the site |
| P7a-09 | email ECA | A failing email rule is logged and the rest run; Java aborts | AGENTS.md failure-tolerance policy |
| P7a-15 | SECA | A pattern rule with `when="tx-commit"` or `"tx-rollback"` runs (`register_tx_rules`, `service/call.rs:607-612`); Java's `registerTx` compares the rule's service name, the pattern text, to the called name, so it never registers one | A pattern rule runs at each `when` it names, as at every other phase (P7a-08); Java's skip comes from comparing a pattern as a name. Doc comment at `register_tx_rules` |
| P7b-04 | pools | Workers = `worker-pool-max` − job core; core/alive ignored (Java effectively runs `worker-pool-core`) | Comment at `service/pool.rs:801-803` |
| P7b-06 | jobs | Lock row with a run id but null `lastRunTime` holds the job | Comment at `service/jobs.rs:224-227`; Java always writes both |
| P7b-16 | jobs | `ServiceJobRun.runThread` is the name of this port's job thread, `moqui-job-0` (`service/jobs.rs:463-475`); Java's is `MoquiJob-1` | The column names the thread that ran the job in both; the threads follow this port's `moqui-*` names, which its logs show. Comment at the site |
| P7b-18 | async | An async call whose parameters fail validation is not queued: the errors are in the caller's message facade, and a future gets an empty map (`prepare_async`, `service/call.rs:264-313`); Java queues it, skipping its authorization check, and its run fails the same validation | Safer: a call that skipped the authorization check is not queued, and its run would do nothing but its `pre-validate` rules. Doc comment at `prepare_async` |
| P8a-14 | remote REST | Substituted `${param}` values percent-encoded; Java does not URL-encode | Doc comment `service/remote.rs:113-114` |
| P8a-16 | service REST | No-user denial returns 401; Java throws (403) | Comment `service/rest.rs:523-524` |
| P8a-17 | print | `print#ScreenDocument` refuses a `contentType` other than `application/pdf` with an error, and renders in the one webapp, `webroot`, whatever `webappName` says (`service/tools.rs:207-214`); Java's FOP writes other output types and renders in the named webapp | The PDF renderer makes only PDF, so another type cannot be made, and this port serves one webapp, `webappName`'s default; comment at the site |
| P8a-23 | service REST | Authc message says "operation"; Java has the "operaton" typo | Don't copy the typo |
| P8a-25 | DataDocument notifications | An empty `userId`, `fromUserId` or `toUserId` in a document names no user (`find_nested`, `service/tools.rs:615-632`); Java's `findAllFieldsNestedMap` adds it | Java then writes a `NotificationMessageUser` for user `""`, which fails the send for every user of the document; doc comment at `find_nested` |
| P8a-27 | service REST | A `<method>` with no `type`, no operation element, or an operation with no `name` (or no `operation`) is logged and left out, and the rest of the file is served (`service/rest.rs:326-344`); Java keys one with no type by `null`, registers a service or entity handler with no name, which fails when called, and fails the whole file for a method with no child | AGENTS.md failure tolerance; comment at the site |
| P8a-28 | service OpenAPI | A REST service that is not defined, or whose parameters do not expand, is logged and documented without parameters, and a parameter whose `entity-name` or `field-name` is not defined is logged and has no `enum` (`service/rest.rs:910-923`, `:1049-1067`); Java's Swagger throws for the whole document | AGENTS.md failure tolerance; doc comments at `service_parameters` and `parameter_enums` |
| P8a-29 | service REST | A plain map out-parameter loses its null entries one level down too (`remove_result_nulls`, `service/rest.rs:596-615`); Java removes them only from an `EntityValue` | Entity values are maps in this port, so the two cannot be told apart; a missing key and a `null` read the same to a JSON client. Doc comment at the function |
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
