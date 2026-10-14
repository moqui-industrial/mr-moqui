# Implementation Phases

Bottom-up build order for the MR-Moqui port, and the four gates that mark it.

This follows `plans/Architecture-Proposal.md` and `plans/Crate-Layout.md`. Search, server-static qvt, embedded Postgres, context objects, and cluster coordination stay in their own plans. This document says when each body of code lands. It does not restate them.

The entity facade and the service facade are whole parts. Both are finished before the first gate. That gate is the integration proof that they load a data file. It is not the line that cuts those facades down.

Mantle and the other application components stay outside this repository. They are the first consumer after a gate, not a phase. There are no dates or pull-request cuts here.

## Gates

1. **Data file load.** The entity facade and the service facade are already complete. Startup creates tables, framework seed XML loads, and a data-file element that names a service runs that service.
2. **REST API request.** The HTTP edge and a basic screen facade run `rest.xml`. Entity REST and service REST are the facade methods from gate 1, called through a transition.
3. **Screen rendering.** One real screen renders in every kept mode: `text`, `csv`, `xml`, `html-print`, `html`, `qvt`, plus the `qjs` and `qvue` companions. The name `xsl-fo` selects `html-print`. `vuet` is absent. Server-static `qvt` is the last slice of this gate.
4. **Tools and utilities.** The other facades and tool services: search, mail transport, notifications, system messages, entity sync, ETL, and the tool components. This is not a second pass over entity or service. LLM and A2A are phase 13, after this gate, and do not block it.

## Decisions

- Build bottom-up. A facade is finished, including the features that would otherwise be left as a remainder, before any gate that depends on it. Gate 1 depends on the whole entity facade and the whole service facade.
- The entity facade is one phase: definitions, SQL, CRUD, database metadata, view-entity, dynamic view, master definitions, entity cache, transaction cache, sim mode, `text-fts` (column, index, and `matches`), audit log, field encryption, localized fields, ECAs, the data loader, the data writer, and `EntityFacade.rest`. Document assembly and feed rules live here too. The index those feeds write is `SearchFacade`, which stays gate 4. Until that facade exists, feed dispatch inserts the pending row and the receive service records that search is not up. It leaves that row. It does not grow a second, temporary index.
- The service facade is the next phase, after the entity facade and after the user and artifact-authz checks those calls enforce. It includes definitions, entity-auto, inline, script, the full XML action library, QuickJS, SECAs, email-ECA rules, `service-group`, local `async="true"`, service `callFuture()`, the scheduler and scheduled jobs, parameter validation, `RestApi`, and the remote REST runner. `async="distribute"` stays dropped, as the architecture proposal already decided. `callFuture()` is the same factory thread and new `ExecutionContext` as `async="true"`, plus a oneshot the caller waits on.
- JSON-RPC is not included. Moqui's generic service endpoint (`rpc.xml`, `handleJsonRpcServiceCall`, service type `remote-json-rpc`, `callJsonRpc`, `send#SystemMessageJsonRpc`) was not used seriously, and its authorization semantics are a reason XML-configured REST exists. There is no dispatcher, no remote JSON-RPC runner, and no `/rpc` route. A service of type `remote-json-rpc` fails registration. Copied Java XML that names it stays in the tree and is not wired. The A2A protocol binding is a different wire format and stays in the phase 13 plan.
- Tool services are not the service facade. Send-mail, search indexing, print, and system-message handling are gate 4 service bodies on top of a facade that can already call them. LLM service bodies are phase 13. Email-ECA rules are the service facade. The `lettre` transport they eventually call is gate 4.
- XML actions are one JavaScript macro module and one QuickJS runner, inside the service facade, before gate 1. `JsTemplateLiteral` is part of that runner, because action text calls `ec.resource.expand()`. Screen transitions reuse the runner. There is no smaller action subset built only for the load gate. The fancy renderer and its locked-down variant land with screens.
- The script host at the end of the service facade exposes `ec.entity`, `ec.service`, `ec.message`, `ec.logger`, `ec.resource`, `ec.user`, and the static helpers from `plans/ContextObjects.md`. `ec.web` and `sri` grow with the web and screen phases. The method list is the Server host methods section of `plans/ContextObjects.md`. This document says which facade must answer before which gate.
- Before any Groovy `<script>` body or attribute expression is rewritten, and before screen macros are written, a mechanical pre-scan extracts those texts and classifies Elvis, empty collections, Groovy ranges, decimal arithmetic and `BigDecimal` method calls, decimal `==` and truthiness, decimal literals (Groovy decimal literals are `BigDecimal`; their scale and rounding are not JavaScript number behavior), closure idioms (`each`, `collect`, `find`, `any`, `every`), GString, the regex operators (`=~`, `==~`), Elvis on a method chain, and `catch` around an entity or service call, per file. The same toolkit extracts the translatable string call sites so the localization catalogs survive the migration, and its report is a per-expression before/after report that feeds the AI pass in `plans/Architecture-Proposal.md`. It is not a compatibility shim. It runs on Groovy source. Text that is already JavaScript is parsed by `oxc_parser` at definition load, in phase 6. mantle-usl is outside this repository. The same pass covers it before that migration is treated as ready. Counts already recorded as a review's counts are re-measured by that pass.
- User login and artifact authorization are in place before the service facade is called complete, because entity operations and service calls consult them. The load path still disables authorization, as `ExecutionContextFactoryImpl.checkEmptyDb` does. The signed session cookie and HTTP basic auth are the web edge at gate 2. `UserFacade.loginUser` does not need a socket.
- `HttpClient` and `RestClient` land in `moqui-lib` with the service facade, because the remote runners are part of that facade. The sim predicate default-denies. Sim itself is already implemented in the entity and transaction work. The LLM provider exception is wired when the LLM client lands.
- MySQL is in the same driver phase as embedded Postgres and external Postgres. Gate 1 is demonstrated on embedded Postgres and repeated as a metadata, insert, and select smoke test on MySQL. Further databases (SQL Server, Oracle, DB2, Derby, HSQL) are new `DbDriver` implementations under gate 4. The facade is already written against the trait. H2 has no driver.
- `signer` is `SimpleSigner` (RSA `SHA1withRSA` over a PEM) and lands in phase 8 with QZ Tray. Field encryption is the PBE in phase 4 and does not use it. `etl` and `sgml` wait for the tools phase.
- Security ECAs run during the load gate. `Security.eecas.xml` fires on `UserGroupMember` and `UserGroupPermission`, and framework seed writes `UserGroupPermission`. `UserSecurityChecks` are Rust functions on the script host.
- Second factor, password reset, and API-key admin wait for the tools phase. Gate 2 uses HTTP basic auth and the login transition for a user with no second factor.
- The OpenAPI 3.1 export and the JSON Schema transitions (`entity.json` and the service schemas) on `rest.xml` close the REST phase. Gate 2 does not depend on them. `RestApi.run` itself already exists from the service facade. Swagger 2.0 is a candidate to leave out and stays out unless a concrete consumer needs it; RAML is left out.
- Basic `ec.l10n` format and localize land with screen macros. Localization admin screens are ordinary screens.
- Server-static `qvt` closes gate 3, after server-merged `qvt` renders. It follows `plans/QvtServerStatic.md` and stays opt-in.
- Search is implemented under gate 4. The design is `plans/SearchFacade.md`.
- The SSO relying party is framework code. The crate is chosen in `plans/Sso.md`. The redirect is not part of gate 2 and does not sit in the tool-component tail. PDF and the tool components (`moqui-poi`, `moqui-image`, `moqui-cups`) are the tail of gate 4. Gate 3 `html-print` is the print document, and the name `xsl-fo` selects that mode. Email attachments, scheduled screen PDFs, and print rendering report that PDF is not available until phase 12. The engine is the long-lived Chromium in Tool components and print of `plans/Architecture-Proposal.md`. `moqui-fop` is not ported.
- The Groovy shell endpoint is dropped. There is no Java service runner and no Groovy runner. Framework script services become JavaScript or Rust. The replacement crates are chosen in `plans/Ssh.md`. The sshd is still not part of the port.
- `InstanceServices` Docker provisioning (init, start, stop, remove, logs, and create database) is not ported. `callFuture()` still reserves a worker when the caller is already a pool thread. The Java joined sites stay named in `plans/Architecture-Proposal.md`. They are not a body of work in this repository.

## Phase order

```
0  workspace and XML trees
1  moqui-lib, without HTTP yet
2  factory, conf, components, cache, messages, artifact stack
3  transactions and database drivers
4  entity facade, the whole facade
5  user and artifact authorization
6  service facade, the whole facade (lib HTTP lands here)
        Gate 1 — data file load
7  web host and basic screen facade, rest.xml on the wire
        Gate 2 — REST API request
8  screen model and every kept render mode
        Gate 3 — screen rendering
9  SearchFacade, and the feed starts indexing
10 mail transport, notifications
11 system messages, entity sync, user remainder, and the other tools
12 PDF, later database drivers, and the tool components
        Gate 4 — tools and utilities
13 LLM and A2A, once the Java side settles
```

Inside phase 4 the entity code is still ordered bottom-up: definitions and SQL, then metadata, then the write-path features (cache, sim, full-text, ECA matching, loader and writer). ECA actions and data-file service elements call the service facade, so they are turned on at the end of phase 6, still before gate 1. That is the one cycle between the two facades. Entity is built first, with those callbacks stored and not yet run. Service is built second. The gate runs only after both sides of the callback exist.

Phase 9 depends on gate 1 and can run beside phases 7 and 8. Phase 10's mail depends on the service facade. The notification WebSocket also needs the phase 7 session cookie. Neither waits on gate 3. An email body that is a screen uses the phase 8 renderers. Phase 11 needs the HTTP client from phase 6 for system messages and entity sync, and the phase 7 transitions (`/rest/sm`, `sendOtp`, `verifyOtp`). Phase 12's PDF needs the `html-print` render from gate 3. Later database drivers in phase 12 need only the `DbDriver` trait from phase 3. Phase 13 needs the HTTP client, the user facade, the screen facade for screen-search hints, and SearchFacade. It does not hold up gate 4.

## Phase 0 — Workspace and XML trees

Depends on nothing.

- Cargo workspace from `plans/Crate-Layout.md`: `moqui-lib`, `moqui-db`, `moqui-core`, `moqui-host`, binary `moqui`. Direction is `moqui` → `moqui-host` → `moqui-core` → `moqui-db` → `moqui-lib`. `moqui-db` is the entity model and SQL text. It has no `ExecutionContext` and no driver.
- `framework/` and `runtime/` in this repository: entity, service, screen, and data XML, XSD, default conf, `component.xml`, webroot screens, and the tools component XML. Macro templates and browser libraries are added by the phase that owns them.
- A parse test for `MoquiDefaultConf.xml` plus a dev conf overlay.

Mantle, SimpleScreens, and the other components stay outside the repo, as they do today under `../moqui/runtime/component`.

## Phase 1 — `moqui-lib`

Depends on phase 0. No `ExecutionContext`.

Modules: `error`, `node` (the `MNode` port; parsing is filled in as real files fail to load; no XML crate), `cache` (`MCache` and `MCacheManager`), `object` (decimals, timestamps, booleans, strings), `string`, `collection`, `context_stack`, `env` (property lookup and `${name}` expansion against the environment, including the `runtime/.env` file loaded at startup like environment variables), and the resource trait with file and memory implementations.

`signer` is added in phase 8 with QZ Tray. It is not used for field encryption. `http` / `RestClient` is added at the start of phase 6, when the remote runners need it. `etl` and `sgml` wait for phase 11. `context_stack` lives here because actions and screens share it, and the stack must not know the framework.

## Phase 2 — Factory, components, cache, messages

Depends on phase 1.

- `ExecutionContextFactory` and `ExecutionContext`. Facades are fields. A facade that does not exist yet is not called.
- Conf XML, `component.xml` discovery, and component dependency order. `default-property` resolution follows the Java order — environment (including `runtime/.env`, real environment winning) then the conf `value` — with `is-secret` values never logged. A secret property still at its `CHANGEME` default is generated (CSPRNG) and appended to `runtime/.env` at startup; an unwritable `.env` fails startup rather than running on a regenerated key. The properties and the always-set deployment list are Decision 1 of `plans/MimoReview.md`.
- `ResourceFacade`. Schemes are `file`, `runtime` (the runtime directory), and `component`. The framework component is a directory. There is no classpath scheme and no jar scheme. `getChildren` and `findChildFile` exist on those three. Directory subscreens and wiki spaces use that walk. `dbresource://` lands with the entity facade in phase 4. `http://` and `https://` locations wait for the blocking client in phase 6 and until then report that HTTP resources are not available. `content://` is an error that names the location.
- Cache facade on the `MCache` port from phase 1. Entity-definition and service-location caches use it. There is no second cache implementation.
- Logger on `tracing`.
- `MessageFacade` for errors and messages.
- Artifact execution stack with push, pop, and `disableAuthz`. Push opens a `tracing` span and pop closes it. The span carries the artifact type, name, action, and duration. Phase 11's hit-bin flush is a subscriber on those spans and still writes `ArtifactHit` and `ArtifactHitBin`. The authorization decision itself is phase 5. Until then the stack allows the call when authorization is disabled, which is the load path and the early entity tests.

## Spikes

Throwaway programs, run before phases 3 and 6 are treated as settled. They are not gates, not phase tests, and they do not stay in the tree.

1. Load mantle-udm's 422 entity definitions onto embedded Postgres.
2. AI-migrate about 30 representative mantle-usl services and run them.
3. Stress the pool with nested `force-new` plus sequence refill.
4. Render one real SimpleScreens screen as static `qvt`, including decimals. This run is also the proof that `@vue/compiler-dom`'s Node build loads in QuickJS, as `plans/QvtServerStatic.md` describes.

## Phase 3 — Transactions and database drivers

Depends on phase 2. `DbDriver` is a trait in core. Connections are host. Dialect profiles and SQL text are `moqui-db`. The SQL text is written in phase 4. There is no SQLx dependency.

- `TransactionFacade` and the transaction manager, in core, calling `DbDriver`: begin-or-join, commit, rollback, savepoint, `setRollbackOnly`, timeout, `ignore`, `use-or-begin`, `force-new`, `cache`, `force-cache`, and the synchronization list (transaction cache, `runOnCommit` / `runOnRollback`, service ECAs, data feed). `begin` records the deadline and does not check out. The first statement enlists one connection. The feed hook follows `plans/SearchFacade.md` and the fixed phases in the Transactions section: after the cache flush and before commit, the feed inserts the pending row on the enlisted connection. Phase 3 registers that insert. Phase 6 points the after-commit slot at the receive service, which records that search is not up and leaves the pending row. Phase 9 consumes the row with the index write. `suspend` parks that client on the transaction, off the free list. `resume` checks the same client out. `force-new` is that suspend followed by begin, on the same thread and the same `ExecutionContext`. The park budget is counted per thread and per pool: `max - reserve - 1` parked levels, so `force-new` nests as in Java. The startup check below still guarantees one parked level for every thread at once, and a deeper level that finds the pool exhausted fails within `pool-time-wait` instead of waiting on a parked connection. The thread holds one active ordinary connection per pool. The parked client still counts against the pool max. A reserve inside the pool (default 8) serves side-channel work. Startup fails when `(request cap + service cap) × 2` exceeds `pool_max - reserve`. One deadline per transaction, default 60 seconds. The timer marks rollback-only and cancels the active statement. `destroyAllInThread` runs in a `finally` block on every request, job, and async-service thread. No JTA types and no XA two-phase commit. The rules are the Transactions section of `plans/Architecture-Proposal.md`. The transaction-cache hooks used in phase 4 sit on these same calls.
- Per-connection prepared-statement LRU, cap 100, keyed by SQL text, on the Postgres and MySQL drivers. Entity SQL uses it. `query_once` is the one-shot path.
- Dialect profiles from the `<database>` block, in `moqui-db`. Postgres and MySQL in this phase.
- Instrumentation from the start: gauges for checked-out, parked (suspended), and waiting connections, pool wait time, and service worker pool depth and queue wait, reported through `tracing` metrics. A pool that starves is the failure mode this port can create, and it must be visible.
- Host `db`: embedded Postgres via `postgresql_embedded` (start, stop, `NoTls`), external Postgres with rustls, and the MySQL client with rustls. Unix embedded traffic uses the socket directory. Windows uses TCP to `127.0.0.1` on an ephemeral port. The pool is the core manager. Settings, `save-db` / `load-save`, the major-version upgrade path, and orphaned-postmaster handling are `plans/EmbeddedPostgres.md`.
- A test round-trips a raw statement through each driver, and a streaming query round-trip per driver covers the cursor arm (`DbDriver` streaming in `plans/Crate-Layout.md`).

The facade is complete against this trait. Adding SQL Server, Oracle, DB2, Derby, or HSQL later is a new driver in phase 12.

## Phase 4 — Entity facade

Depends on phase 3. This phase finishes the entity facade.

Bottom-up inside the phase:

1. Entity XML in `moqui-db`: dictionary types, fields, relationships, reverse-many, `extend-entity`, master definitions, view-entity structure.
2. `EntityValue`, `EntityList`, find, create, update, delete, refresh, sequenced primary and secondary ids, `createOrUpdate`, foreign-key check, and dummy foreign keys, in core. The condition tree is `moqui-db`, and `EntityFind` builds it. The sequence bank stays in memory (default size 50, stagger, start at 100000). Refill runs on a reserve connection, autocommit, against `SequenceValueItem`, as the Transactions section specifies. It does not open a nested transaction on the caller's connection.
3. SQL and DDL renderer in `moqui-db` for insert, update, delete, select, and count, including joins, offset, and view-entity select, for the postgres and mysql profiles. Core calls it. Statement execution is a `tracing` span with the SQL text and duration, and a slow-statement threshold logs the slow ones.
4. `EntityDbMeta`: check and create tables, columns, indexes, foreign keys, and sequences. Startup runs the check before any load. Runtime DDL uses a reserve connection, not the caller's enlisted connection. `text-fts` columns and their indexes are part of this metadata. The `matches` operator is part of the condition tree in this same phase.
5. Field encryption on write and read, a port of `EntityJavaUtil.enDeCrypt`. Conf `crypt-algo` defaults to `PBEWithHmacSHA256AndAES_128`, with `crypt-pass`, `crypt-salt`, `crypt-iter`, the constant IV, and URL-safe Base64 stored as `IV:ciphertext`. An empty IV prefix means that constant IV. A value with no colon is hex and is decrypt-only. `decrypt-alt` includes `PBEWithMD5AndDES`. Existing rows must decrypt. New writes use the default algorithm and the colon form. The oracle is `SecurityCryptoTests.groovy` plus one fixture of Java ciphertext for the default algorithm and one for the alt. The implementation is RustCrypto when those vectors pass. It does not use `SimpleSigner`. Audit-log rows when a field has `enable-audit-log`.
6. A field with `enable-localization="true"` reads through `moqui.basic.LocalizedEntityField` for the session locale, then the locale chain, then the stored value. Framework fields that set the flag include `Enumeration.description`, status item and transition text, and `Uom`. This is the entity read path. `ec.l10n.localize` stays the message catalog in phase 8.
7. Entity cache, cleared on write. The local cache stays the only cache. Invalidation across instances is the cluster topic in `plans/Cluster.md`, landed in phase 10, and is not a second cache. Transaction cache as an in-memory map. Sim mode: a schema per `TransactionCacheDb` on the overlay cluster from `plans/EmbeddedPostgres.md`. HOLD, FLUSH, and `enter_sim` are a port of the Java code at the time the port begins.
8. Dynamic view. Data writer. `EntityFacade.rest` (find, create, store, update, delete, path primary keys, relationship steps, dependent levels, master name), tested by calling the facade, with no HTTP server yet.
9. ECA rules loaded and matched on create, update, and delete. The action bodies run once phase 6 can run them.
10. Data loader for entity elements: streaming XML, CSV, and JSON, including a zip of those. Roots `entity-facade-xml` and `seed-data`. Type filter, component filter, conf `load-data` locations, nested relationships, `onlyCreate`, `useTryInsert`, dummy foreign keys. Service-named elements are parsed and held until phase 6 runs them. `never-try-insert="true"` on Postgres forces `useTryInsert` off, and the loader uses `createOrUpdate`. When try-insert is on, Postgres uses `INSERT ... ON CONFLICT` and MySQL uses `INSERT ... ON DUPLICATE KEY UPDATE`. An unhandled statement error marks the transaction rollback-only. Codegen emits a savepoint around a `try` whose body contains an entity or service call. `ignore-error="true"` stays a message-level flag. The rule is the Transactions section of `plans/Architecture-Proposal.md`.
11. Data-feed rules. Feed dispatch is stored here and calls the receive service once phase 6 can run it. That service does not index until phase 9. `disableEntityEca`, `disableAuditLog`, `disableFkCreate`, and `disableDataFeed` work. Document assembly (the `EntityDataDocument` port) did not land in this phase; it landed with phase 9, with the feed's delete rows and related-entity resolution.

The factory holds one `DbDriver` per entity group. The default framework group is one database. `dbresource://` resolves through this facade. `db://` is not a scheme.

Phase 4 tests cover the facade without a service call: create and read, a view-entity find, a dynamic view, an FTS `matches`, a sim write that does not land in the main schema, table check on an empty database, and `EntityFacade.rest` against an in-memory request map. An entity-find `iterator` streams a result set larger than one fetch and releases the connection. A hold or a sim schema uses the merged in-memory list, and the cursor opens only when there is no overlay.

## Phase 5 — User and artifact authorization

Depends on phase 4. The permission and user tables exist. Entity and service operations are not complete until they enforce these checks, so this phase sits between the two facades and still before gate 1.

- Passwords with `argon2` 0.6 (`password-hash` feature). `loginUser`, `logoutUser`, anonymous login, `internalLoginUser`. Shiro is not carried forward. A stored hash whose `passwordHashType` is not argon2 is verified with that algorithm and, on success, rehashed to argon2. An empty type uses the conf default, SHA-256. The legacy construction is Shiro `SimpleHash`: `MessageDigest` of the password plus salt, hex unless `passwordBase64` is `Y`. The name `SHA` is Java's SHA-1. `passwordHashType` and `passwordBase64` stay. Login-key hashes stay on their own conf type.
- Server-side session state: user, visit, visitor, locale, time zone, CSRF token, flash messages, saved and error parameters, screen history. These fields live in memory in this phase. `moqui.server.HttpSession` and the flush are phase 7, as Sessions and login in `plans/Architecture-Proposal.md` describes. The signed cookie that carries the session id is phase 7. Tests call `loginUser` on the facade directly.
- QuickJS expression runtime and context bindings for authorization filters, with the safety limits described in phase 6. Expression errors fail closed; phase 6 reuses this runtime.
- Artifact authorization against artifact groups, permissions, and authz records, including tarpit. Entity finds and writes, and service calls, consult it. `disableAuthz` still short-circuits, which is how load and internal work run.
- A matching `ArtifactAuthzFilter` also adds `EntityFilter` conditions to the find, the Java `filterFindForUser` path. View-entity finds map the filter field through the member alias. `EntityFilterSet.applyCond` is a JavaScript expression, evaluated when the find is built, with the same bare-name rules as other server expressions. Mantle's `MANTLE_USER_ORG` and `MANTLE_ACTIVE_ORG` sets are the oracle. A phase 5 test finds through a view-entity with one filter applied and one `applyCond` that skips the set.
- Password policy from `user-facade`: minimum length, minimum digits, history limit, change interval, `max-failures`, and `disable-minutes`. A failure increments the counter on the reserve connection. The account disables for `disable-minutes` after `max-failures`. The checks run on login and on password change. Password reset and second factor stay phase 11.
- Visit and visitor rows. The stats screens are ordinary screens later.

Second factor, password reset, and API-key admin are phase 11. A user with no second factor stays logged in from `loginUser`.

## Phase 6 — Service facade

Depends on phases 4 and 5. This phase finishes the service facade.

- `http` and `RestClient` in `moqui-lib`, blocking `reqwest`. The sim predicate default-denies. Sim is live from phase 4, so the predicate sees it. `http://` and `https://` resource locations use this client.
- Service XML, `service-include`, and `verb#noun` lookup. Implicit entity-auto: no path, verb one of `create`, `update`, `store`, `delete`, noun an entity name. A data element such as `create-moqui.security.UserAccount` resolves that way after `-` becomes `#`. `type="interface"` defines a service with no runner. `<implements>` copies that service's parameters onto the implementing service. Framework email, data-feed, print, and system-message services load through this. A service that implements an unknown name fails at load.
- `semaphore` is `wait` or `fail`. The default name is the service name. `semaphore-name` replaces it. `semaphore-parameter` scopes the row by that parameter's value. The row is `moqui.service.semaphore.ServiceParameterSemaphore`. The check and the clear each run on a reserve connection, before the service's own transaction. `fail` returns an error when the row is held. `wait` sleeps until the row is free or the semaphore timeout fires. `poll#EmailServer` is `semaphore="fail"`.
- Sync call: parameters, the full validation set (required, matches, ranges, entity-backed validators), and the transaction attributes `ignore`, `use-or-begin` (the default), `force-new`, `cache`, `force-cache`, and `transaction-timeout`. `cache` is `use-or-begin` with the transaction cache. `force-cache` is `force-new` with the transaction cache. `transaction-isolation` is not ported. The `authenticate` attribute is enforced with phase 5.
- Entity-auto runner, including nested maps. The plain-map load path calls the same store function.
- XML actions: a JavaScript module with a function per element in `xml-actions-3.xsd`, emitting JavaScript, run by QuickJS. The walker is the one in `plans/ContextObjects.md`. The function sees the element, not the execution context. Functions emit bare names, and the server proxy in that plan resolves an unset name to null. `JsTemplateLiteral` implements `ec.resource.expand()`. One QuickJS runtime per thread and per nesting depth, not per `ExecutionContext`: a script that calls a service that runs a script needs a second runtime, because a `Context::with` cannot be entered again. Each is created when first needed and reused. The shared context stack is per thread. Scripts run in sloppy mode, because a bare assignment is how a name reaches the stack. `destroyAllInThread` does not rebuild it. Host callbacks are `catch_unwind`, and the workspace is `panic = unwind`. Decimals stay host values and use checked arithmetic. The runtime's interrupt handler watches the transaction deadline, and `set_memory_limit` uses the conf byte cap.
- Script-type services whose body is JavaScript. The Groovy email scripts are converted in phase 10. There is no Java runner.
- `UserSecurityChecks` for the two security ECAs, as Rust functions on the host. ECA actions from phase 4 run through this runner. SECA rules run through it too.
- Email-ECA rules run through the same action runner and call a service. The transport behind send-mail is phase 10. Until then the send service reports that mail is not configured.
- `service-group` is not built: Moqui's own `xml-actions-3.xsd` has it commented out as tabled. `async="true"` submits to the service worker pool and gets another `ExecutionContext`. `callFuture()` is that submit plus a oneshot the caller waits on. The pool is bounded (cap, bounded queue, depth metric). A `callFuture` submit against a full queue fails with a clear error and does not block a thread that already holds a transaction. A `callFuture` whose caller is already a pool thread runs on a reserved worker, so the join cannot occupy every worker. Fire-and-forget that must survive a full queue, including commit and rollback synchronizations and any `afterCompletion` submit, is inserted as a one-shot service job. The scheduler tick submits that row, and a due job, to the service pool when a slot is free, and leaves the row when the pool is full. The tick does not run the body. `ServiceJobRun` stays the history row of a named job. It is not that queue. The joined sites are named in `plans/Architecture-Proposal.md`. The scheduler thread sleeps and dispatches, with the `croner` parser in `plans/Crate-Layout.md`. Both are core threads, as that file describes.
- Definition load parses every JavaScript expression with `oxc_parser` 0.139.0 (`rust-version` 1.94.0, the host floor). The version is named in the workspace `Cargo.toml` and no crate depends on it until this phase, so lib and core stay checkable on older rustc. 0.140 and later declare a higher MSRV. 0.153.0 declares Rust 1.97. A Groovy leftover fails that load. The same AST is what the phase 8 compatibility report walks. The binary subcommand is `moqui lint`. The decimal lint in `plans/TypeGeneration.md` is this parse.
- `RestApi`: load `*.rest.xml`, walk the path, run `<entity>` and `<service>` operations. Tested by calling `run` with a path list and a parameter map. The HTTP route is phase 7.
- Remote REST runner on `RestClient`. JSON-RPC is not included, as the Decisions section says.

### Where the code differs from the text above

These were decided while building phases 5 and 6 and are not defects.

- **Authorization.** `isPermitted` is ported: frames carry what was authorized, a child inherits from its parent when the record says `inheritAuthz`, an always-allow skips the checks, and a deny on the stack stops a plain allow. Entity reads need `view`. Tarpit counts hits for services and screens, not entities, as in Java. The per-user group and check lists are cached on the session and reload when an authorization table is written. `filterMap` and `authzServiceName` are honored. Session state lives on the `ExecutionContext`.
- **Errors carry a kind.** `ErrorKind` (unauthorized, authentication required, tarpit, not found, method not allowed) is what phase 7 maps to status codes.
- **Service calls.** A service goes on the artifact stack. Message errors left by the body roll the transaction back and the call still returns its results. `post-commit` rules run after a failure and skip unless `run-on-error`. Authentication is not required while authorization is disabled, which is how a data load calls services.
- **Parameters.** A value that cannot become the declared type is an error. `allow-html` defaults to `none`; `safe` cleans the value with `ammonia`'s default allow-list (disallowed tags and attributes are dropped, script and style content removed, unsafe URL schemes removed, links get `rel="noopener noreferrer"`), without Jsoup's removal messages. Text with no `<` is not touched. `default` is an expression and `default-value` is expanded text.
- **Jobs.** `ServiceJob` rows with a cron expression run from a scheduler pass every `scheduled-job-check-time` seconds, in the server time zone. The lock is taken with a conditional write on a reserve connection, not `FOR UPDATE`. A fire-and-forget call that finds the pool queue full is stored as a `ServiceJob` named `ASYNC_QUEUE_<id>`, its parameters as text, and submitted when a worker is free. A script's `ec.service.async()...callFuture()` returns a future with `get()`, `get(timeoutMs)` and `isDone()`; Rust code has `service::start_future` and `service::call_future`. Service definitions belong to the factory, so `lint` and `load_definitions` need an entered context and two factories in one process no longer share services.
- **XML actions.** `searchFormMap` (`search-form-inputs`), `requireSearchFormParameters`, `havingCondition`, `forUpdate(true)`, `distinct(true)`, `selectField`, `ignore-case` and `to-field-name` conditions are implemented, with SQL for view-entity aggregates (`GROUP BY`/`HAVING`), `DISTINCT`, `FOR UPDATE`, `UPPER()` comparison and field-to-field comparison. `cache` and `use-clone` hints are accepted and change nothing. Still not implemented: `multi` service calls, `distribute` (a load error), and `entity-data`. Deliberate gaps: `get` on a future has no default timeout (as in Java), the script runtime is per thread and depth rather than per execution context, and scripts are not strict-mode.
- **Remote.** `remote-rest` sends GET, POST, PUT, PATCH, and DELETE with a typed JSON body. `RestClient` follows redirects only to the same host, refuses a response over 16 MiB, and can refuse internal addresses (`deny_internal_addresses`), which the caller turns on for URLs a user supplied. `http(s)://` resources are fetched on first read.

Phase 6 then turns on the two callbacks phase 4 stored: ECA action execution, and data-file service elements (`sync()` on the mapped name). It also points the after-commit feed slot at the receive service. That service records that search is not up and leaves the pending row until phase 9.

Phase 6 tests cover a sync call, an entity-auto store, an inline action with an entity-find, a SECA, a job run, `RestApi.run` for `basic/enumTypes`, and a remote runner against a stub HTTP server.

### Gate 1 — data file load

The facades are already complete. This gate proves them together.

- Fresh embedded Postgres data directory.
- Startup check creates the framework tables, including `text-fts` indexes.
- Load of framework `seed` and `seed-initial` finishes, and an `Enumeration` count is greater than zero.
- `UserGroupPermission` rows from that seed are stored, which means the security ECAs ran and `UserSecurityChecks` allowed the seed write under disabled authorization.
- A fixture file with one entity element and one service element loads. The service element is an implicit entity-auto create, plus one inline service whose actions do an entity-find and an entity-create. Both resulting rows read back.
- MySQL: create a table through metadata, insert, select.
- Binary `load` accepts the Java flags: `types`, `components`, `location`, `timeout`, `raw`, and the disable flags. Empty-db load on startup uses the same path.

After the gate, loading `mantle-udm` entity XML is the first migration check. It is not part of the gate. Expressions in mantle services stay Groovy until that component is migrated, which is outside this repository.

## Phase 7 — Web layer and `rest.xml`

Depends on gate 1. `EntityFacade.rest` and `RestApi` already run. This phase is the HTTP edge and enough of the screen facade to reach a transition.

- Host `http`: axum accepts the connection, builds the owned request (method, path, query, headers, body), runs core on a factory request-pool thread, and writes the owned response. The request pool has a bounded queue. When the queue is full the host rejects the request. The framework does not see axum. The host does not use Tokio `spawn_blocking` for this.
- The SSO callback is specified in `plans/Sso.md`. It is not a gate 2 requirement. It lands after the session cookie and `loginUser` exist.
- Signed session cookie from the phase 5 session. The signature is HMAC-SHA256 over the session id. The key is the `session_cookie_key` secret property (`plans/MimoReview.md` Decision 1), generated into `runtime/.env` when left at the `CHANGEME` default. A bad MAC is an unknown session. The cookie is `HttpOnly`, `SameSite=Lax`, and `Secure` when the request is HTTPS. Login replaces the session id, copies the map, sets a new `sessionToken`, removes the previous row, and invalidates both one-cache keys. This phase creates `moqui.server.HttpSession` and flushes it as Sessions and login in `plans/Architecture-Proposal.md` describes: dirty-field merge, `sessionRevision`, and the entity one-cache. Restart reads that row back. The snapshot is the struct fields only: user, visit, visitor, locale, time zone, CSRF token, flash messages, saved and error parameters, and screen history. It does not include the QuickJS runtime, the transaction, or a connection. HTTP basic authentication, and the API-key and login-key headers, call `loginUser` before the transition.
- The public listener is plain HTTP on `webapp_http_port`. TLS for that listener is not in this pass. A reverse proxy is the deployment that terminates TLS. `client-ip-header` is the client address when the proxy sets it, and the peer address otherwise.
- Response headers follow the Java `response-header` table. `screen-render` is private and no-store, plus `Content-Security-Policy` `frame-ancestors 'none'; form-action 'self';`, `X-Frame-Options: sameorigin`, and `X-Content-Type-Options: nosniff`. HSTS is sent only when the request is HTTPS.
- CORS applies when the request `Origin` is in `webapp.@allow-origins`. Preflight answers the Java allow-headers and methods. `Access-Control-Allow-Credentials` is true. A missing allow-list sends no allow-origin header.
- `WebFacade`: parameters, a JSON object or JSON list body, `sendJsonResponse`, `sendJsonError`, `sendTextResponse`, session attributes, and the response headers entity REST already sets (`X-Total-Count`, the page headers, `X-Run-Time-ms`). Multipart waits for the form file widget in phase 8. `ec.web` is added to the script host here.
- Basic screen facade: parse screen XML, directory subscreens and explicit `subscreens-item`, `ScreenUrlInfo`, transition match on name and method, run the transition actions on the phase 6 runner, and honor `default-response` of `none` or a redirect. Widget rendering is phase 8. `webroot.xml` already treats `rest` as a subscreen by file name.
- `rest.xml` transitions this phase wires: `e1` to `EntityFacade.rest`, `s1` to `RestApi.run`, `login`, and `logout`. `sendOtp` and `verifyOtp` parse, and until phase 11 they return a clear error instead of sending a code. `/rest/sm` parses and returns a clear error that system messages are not available until phase 11. The JSON-RPC route is not built.
- Screen `require-authentication` stays false on `rest.xml`, so the transition decides. The REST handlers require a logged-in user.
- Schema transitions (`entity.json`, the service schemas, and the OpenAPI 3.1 export) close this phase. Gate 2 does not call them. Swagger and RAML output is not built.
- `/health` and `/metrics` on the HTTP edge: readiness reports the factory and pool state, and `/metrics` exposes the phase 3 gauges and the statement timings.

### Gate 2 — REST API request

- Binary serving HTTP, embedded Postgres, seed loaded.
- `GET /rest/s1/moqui/basic/enumTypes` with HTTP basic auth returns JSON of the seeded enumeration types. That resource is an `<entity>` list operation in `tools/service/moqui.rest.xml`.
- `GET /rest/e1/...` for that same entity, by alias or full name, returns JSON.
- `POST /rest/login` for the seed user returns the logged-in JSON and a session cookie. A following request with that cookie and no `Authorization` header succeeds.
- An anonymous request receives 401. A user without permission receives 403.

The response is JSON from the transition. Widget HTML is not part of this gate.

Review follow-ups landed after gate 2: a request authenticated by its own credentials (Basic, `api_key`, `login_key`, `authUsername`) skips the session-token check as in Java; web sessions expire after `webapp_session_timeout` minutes (default 60, 0 never) and idle `HttpSession` rows are purged from the scheduler tick; repeated query or form names become list parameters; `X-Forwarded-Proto` is believed only when `webapp_client_ip_header` marks a proxy; a full request queue is `ErrorKind::Unavailable` (503). Still open: `session.rs` keeps its own small JSON codec because it must preserve number text and key order, and `userInfo` stays commented out in `rest.xml` as in Java.

## Phase 8 — Screen model and render modes

Depends on the screen URL and transition runner from phase 7, and on the action runner from phase 6.

- The rest of the screen model: sections, `section-iterate`, containers, trees, `form-single`, `form-list`, conditional and error responses, `include-screen`, `screen-extend`, and database forms.
- `ScreenFacade.makeRender()` runs with no socket. The same owned `WebFacade` is filled from a parameter map, a session map, a base URL, and a writer. Email bodies, `render#ScheduledScreen`, and `ScreenTest` (the dynamic-options path) use that fill. They do not build a servlet stub.
- `render_ScheduledScreens_frequent` stays a service job. It logs in as the scheduled user when `userId` is set, renders with `makeRender()`, and emails or saves the output. `renderMode` `xsl-fo` still means `html-print`, and the PDF bytes wait for phase 12. The inner call that is `async="distribute"` in `ScreenServices.xml` becomes local `async="true"`. That is the only `async="distribute"` in the tree. Distribute stays dropped.
- `always-actions`, `pre-actions`, section actions, and `row-actions` through the phase 6 runner.
- JavaScript screen and form macro modules. The walker passes attributes, child text, and already-rendered children. Server HTML and client JavaScript are module targets. A native widget-tree renderer is not the path.
- The fancy template renderer and the locked-down variant. The dialect is `if`, `list`, and `assign`, compiled to one cached JavaScript function. No Mustache. Inline `<text template="true">` bodies are rewritten from `<#if>`, `<#list>`, and `<#assign>` into that dialect. `ScreenRenderEmail.xml` is the fixture. The trusted renderer sees the live context. The locked-down renderer is the same compiler and the unwrap snapshot from `plans/ContextObjects.md`. `JsTemplateLiteral` is already in place from phase 6.
- `ec.l10n` format, formatCurrency, and localize, using the session locale and time zone and the seeded localization messages. `format` implements the Java pattern subset the screens use (`#,##0.00`, `yyyy-MM-dd`, `yyyy-MM-dd HH:mm`, and the currency patterns), with `HALF_EVEN`. `0.125` at two places is `0.12`. The server function lives in `moqui-lib`. The browser copy is checked against the same vectors. QuickJS `Intl` and `toFixed` are not the formatter. There is no number-format crate. `sri` gains the methods the macros call. The method list is the Server host methods section of `plans/ContextObjects.md`. This phase implements that list, and records any method it adds.
- Modes, in this order. Each one is a JavaScript macro module on the same screen model.

| Order | Mode | Output |
|---|---|---|
| 1 | `text` | Plain text, server merge |
| 2 | `csv` | CSV, server merge |
| 3 | `xml` | XML, server merge |
| 4 | `html-print` | Print HTML, server merge. `xsl-fo` selects this mode. PDF is phase 12 |
| 5 | `html` | Terse server HTML, shared stylesheet, `moqui-html.js` |
| 6 | `qvt` | Server-merged Vue component object |
| 7 | `qjs`, `qvue` | Companions, kept |
| 8 | `qvt` with `server-static` | Data-free template plus `/actions` JSON |

`vuet`, and the `js` and `vue` companions that feed it, are not built. `/vapps` is absent. `/apps` stays HTML. `/qapps` stays the default.

HTML follows the widget table in the architecture proposal: semantic elements, one shared script, `fetch`, native date inputs, Tom Select and SortableJS loaded only on a page that has them, Font Awesome 4. jQuery, Bootstrap, and Moment are not in this client.

Server-merged `qvt` follows the library section of the architecture proposal: Vue 3.5 and Quasar 2.35 by script tag, `m-*` wrappers, and no jQuery, Moment, or `http-vue-loader`. `QEditor` replaces CKEditor 4. A textarea plus Marked replaces SimpleMDE. Chart.js 4, a current Mermaid build that still loads from a script tag, the Assist libraries, and QZ Tray 2.3 when printing is enabled. `signer` lands in `moqui-lib` with that client: `SimpleSigner`, RSA `SHA1withRSA` over a PEM, the Java caller in `WebFacadeImpl`. The `rsa` crate is chosen then.

Server-static `qvt` is last, and follows `plans/QvtServerStatic.md`. The attribute stays opt-in. The response uses the `screen-server-static` cache header. The template is compiled once when the screen definition is cached, with Vue's `prefixIdentifiers` option, and the cached body is a render function. A page whose qvt output, including fragments, is entirely server-static loads `vue.runtime.global.prod.js`. Server-merged `qvt`, a `merged-fragment` response, and a server-static shell that can receive a merged fragment load `vue.global.prod.js` and compile in the browser. `GET {screen}/actions` and `GET {screen}/actions/{formName}` are private and uncached. The browser `ec` is the restricted one in that plan. `dynamic-container` stays a client fetch. The response header `X-Moqui-Qvt` is `static-template` or `merged-fragment`, as `plans/QvtServerStatic.md` specifies. The compatibility report walks the `oxc_parser` AST from phase 6. It does not flip the default.

### Gate 3 — screen rendering

- One existing tools or framework screen that contains a form-single, a form-list, and a section is rendered through the server in `text`, `csv`, `xml`, `html-print`, `html`, and `qvt`.
- The screen's actions are entity finds and inline services. A screen whose actions call search, mail, or an LLM is the wrong screen for this gate.
- The HTML response includes the shared stylesheet and `moqui-html.js`. A form post runs the transition and returns its response.
- The `qvt` response is a component object the Vue 3 compiler can compile. With `server-static` set for `qvt`, the template contains no row data, and `/actions` returns the JSON package from `plans/QvtServerStatic.md`.
- A headless Chromium smoke runs for each kept mode. The document renders, a form is present, and the console has no exception. The same run executes the compatibility report. The binary is the one Tool components and print already names: `pdf-chromium-path`, else `chromium`, else `chromium-browser`. This smoke does not use `PdfRenderer`. A missing binary fails the smoke. A gate run that claims gate 3 complete does not skip it.
- `qjs` and `qvue` return the companion bodies for that screen.
- A `/vapps` path has no screen.
- The `html-print` body is HTML and does not include `moqui-html.js`. A request with extension `.xsl-fo` returns that same body.

### Where the code differs from the text above

A summary of slices 0 to 8 first, then the notes appended slice by slice.

- **Shape.** The plan held: one screen model (slice 1: definitions, cache, `ScreenUrlInfo`, `makeRender()` with no socket), one macro walker
  (slice 2), and a JavaScript macro module for each mode (`ScreenText.js`, `ScreenCsv.js`, `ScreenXml.js`, `ScreenHtmlPrint.js`, `ScreenHtml.js`,
  `ScreenQvt.js`, `ScreenPlain.js` for `qjs`/`qvue`, `ScreenQvtStatic.js`). Screen XML expressions were migrated from Groovy to JavaScript and are
  linted (`moqui lint --screens`, slice 0); 3 Groovy screens still do not load and are named by the lint and the report. Templates are the `fancy`
  dialect (slice 3), not FreeMarker; a `.ftl` location is refused.
- **Deviations that stay.** `html-print` writes its own document wrapper; `xsl-fo` is only an alias. The body of a `.qvt` response is a
  template the client wraps, not a component object (the Java shape). `/qapps` is html and loads `.qvt` from `/apps`. Request text in qvt uses
  `&#123;` instead of `v-pre`. Server-static `include-screen` is inlined and tree JSON is a 501. The macro-level differences are listed at the top
  of `ScreenHtml.js` and `ScreenQvt.js`.
- **Fixed in slice 8 (found by the Gate 3 harness).** A transition's `service-call` (default `out-map="context"`) failed with `cannot put with key
  'context'`, so every form post through a transition shortcut was a 500: `XmlActions.js` now puts each result on the context by name. A
  `default-response` to a screen did not carry the target screen's `parameter` values (Java's `UrlInstance.getParameterMap`), so
  `updateArtifactGroup` redirected to `ArtifactGroupDetail` without `artifactGroupId`. A transition requested with `Accept: application/json`
  (the qvt `m-form`) got a 302 to the page html; it now gets Java's `sendJsonRedirect` JSON (`screenUrl`, `screenPathList`,
  `screenParameters`, `messageInfos`, `currentParameters`), and the messages are not also saved for the next page. `entity-find` with
  `search-form-inputs` did not set `<list>Count`, `PageIndex`, `PageSize`, `PageMaxIndex`, `PageRangeLow` and `PageRangeHigh`, so no
  form-list in any mode had pages (the find now has `getLimit`, `getOffset` and `copy` for it). `Header.html.fancy` wrote a script or style
  sheet twice when the theme and the page both named it (every error page loaded `moqui-html.js` twice). In qvt `fields-initial`, a `link` after a
  `display-entity` in one field replaced the entity text with the id (Java does the same). The `m-*` field wrappers passed `id` to
  `q-input`/`q-select`/`q-file`, which drop it; they now pass `for`, so the native input has the id. `m-static-form-list` reused the row
  components of the last page, and an `m-form` copies its `fields-initial` once, so page 2 showed the rows of page 1.
- **Deferred.** PDF bytes (phase 12); `ScreenServices.xml` service bodies (mail screen bodies were done after slice 8, see phase 10's
  "Screen mail after phase 8"); `vuet` and `/vapps` (dropped); the Assist panel; in a
  static template, row selection, first/second/last rows, aggregation, the column picker and the csv/text/pdf buttons; precompiled `m-*`
  templates, which `webapp_qvt_static_only=true` waits for; the `lookup` widget (a text line, as the Java macro was commented out). A static
  template is fetched once to read `X-Moqui-Qvt` and then loaded again by a script element (a URL without the query), so it downloads twice.
- **Deferred after the Phase 8 and 10 review against Java.** Each is a Java feature no screen in this tree needs yet; the Java reference is
  in `ScreenRenderImpl` (SRI), `ScreenDefinition` (SD), `ScreenForm` (SF) and `ScreenUrlInfo` (SUI):
  - tree node JSON (`sendSubNodeJson`, SD:1085-1088), `include-child-content` and template files under a screen (SRI:650-747),
    transition names that are patterns (SD:509-515), and `section-include` copying only what the section holds (SD:366-390).
- **Ported after the review.** These came off the list above:
  - `auto-fields-service` and `auto-widget-service` build fields from a service's parameters, with `auto-parameters` expanded, an entity
    field's widget when the parameter is one and a widget by the parameter type otherwise; `Tools/Service/ServiceRun.xml` shows its
    parameter form. `setSubFieldValidateAttrs` puts `validate-service` (or `validate-entity` for an implicit entity-auto service) on the
    fields of a form whose transition calls one service, so the client checks follow the service.
  - `extends="DbForm#id"` builds the form from the `DbForm` records (fields, attributes, options and entity options), and a form a
    `DbForm` modifies for some group (`DbFormLookup`, read as `DbForm` and `DbFormUserGroup`) is assembled for each render with the
    current user's `DbForm` fields merged in. The `DbForm` node is not cached as Java's `dbFormNodeByIdCache` is; the entity reads use
    the entity cache. `formDisplayOnly` is not read.
  - Saved finds and columns (`screen/saved_finds.rs`). Every screen has the implicit `formSaveFind`, `formSelectColumns` and `screenDoc`
    transitions (`TransitionDef.implicit`), and a screen's own transition of the name wins. A render applies the request's
    `formListFindId`, else the user's `FormListFindUserDefault` for the target screen, and `_clear`; the saved find's parameters and
    `orderByField` apply when the request has none of its own. `pageSize` is kept as the `screen.user.page.size` preference and used
    when a request has none. `formSaveFind` saves, updates, deletes, makes or clears the default and schedules (`ScreenScheduled`), as
    `processFormSavedFind`; `formSelectColumns` keeps the column dialog's choice in a `FormConfig` of the user's own (or by `_uiType`),
    and `ResetColumns` removes it. Form locations are Java's (`{screen}.form_list${name}`, with `#extension` for a dynamic form), so
    records written by Java apply; the macros post the form name and the server makes the location. A form-list with `select-columns` or
    `saved-finds` is generated inside `form-list-user-columns`: at render `sri.renderUserColumnsForm` assembles it with the user's columns
    (the active saved find's `FormConfig`, else the user's, else a group's) when there are any, and the generated code runs otherwise,
    in every server-merged mode. The find dialog lists the user's saved finds in html and qvt (`sri.getUserFormListFinds`,
    `sri.getUserDefaultFormListFindId`), and the menu data has them and the target's screen documents. Differences: the
    `DbFormLookup` and `FormConfigUserGroupView` views are read as their entities; a saved find's description defaults to the username and
    the server time.
  - Form-list aggregation (`screen/aggregate.rs`, a port of `AggregationUtil`): `aggregate="group-by"` merges the rows with the same
    values, a function (`min`, `max`, `sum`, `avg`, `count`, `first`, `last`) aggregates over the group, `aggregate="sub-list"` fields
    become the group's `aggregateSubList`, and `show-total` (`true` is `sum`) adds a last row marked `_moquiRowType: "total"` (and sub-list
    totals). It runs in `sri.openFormList`, so every mode gets the grouped rows. html and qvt draw the sub-list fields in rows under the
    main row (`sri.formListSubRowCount`, `sri.startFormListSubRow`, `sri.endFormListSubRow`) and mark the total row; the other modes show the
    main fields. The group key is every `group-by` field, where Java takes only the displayed ones. A static template leaves out the
    sub-list fields and the total row, which the `/actions/{form}` package does not carry. The select limited to the displayed and
    hidden fields was done in phase 11.
- **Differences that stay.** `disable-when` on a `subscreens-item` blocks the screen below the first level; Java defines `getDisable`
  but reads it nowhere. The `default-display` attribute of an entity field is not read (no entity in this tree or the Java components
  sets it).

- **Slice 3, fancy renderers.** `script/template.rs` compiles the dialect in Rust to one JavaScript function, cached by
  source text, and the trusted renderer runs it on the script runtime. The locked-down renderer runs the same function in
  a separate QuickJS runtime (isolation is a runtime, not a scope trick). `fancy`, `html.fancy`, `fancy-locked` and
  `html.fancy-locked` are registered in `MoquiDefaultConf.xml`. `moqui lint --screens` compiles every template body, so
  leftover FreeMarker is a lint failure. The 51 bodies that already compiled and the 10 that did not (the error pages,
  `AppList`, the `apps.xml` footer, `Login` and `GroovyShell`) are in the dialect. The ten `.ftl` locations are listed in
  the lint report and are slice 4 work (`Header.html`, `Footer.html`, `navbar.html`, `Login`, `SecondFactor`,
  `AutoScreenMenu`, `WebrootVue.qvt`), except `Header.xsl-fo.ftl`, which `ScreenHtmlPrint.js` replaces.
- **html-print.** The document wrapper (`@page` rules for the ten paper and orientation names, a page number in the
  footer, `documentTitle` in the header) is written by the first `widgets` of the render, not by `webroot.xml`, because
  the mode is always standalone. `layoutMaster` selects the page. `xsl-fo` is `html-print` for the request extension, the
  `renderMode` parameter, the builder, a screen's `render-modes` and a `<text type>` list.
- **`ScreenServices.xml`.** Only the `async="distribute"` attribute changed. Service bodies are still Groovy and wait for
  the service migration; `ec.screen.makeRender()`, `getMimeTypeByMode` and `isRenderModeText` exist so that migration is a
  rewrite of the script, and PDF bytes report that PDF is not available until phase 12.
- `BasicWidgetTemplates.xml` is in `runtime/base-component/webroot/template/screen/` with its Elvis expressions as `||`.
- **Slice 4, html.** `ScreenHtml.js` writes the markup of every widget the Java html macros handled (containers, box,
  panel, native `dialog`, dynamic container and dialog, button menu, tree, links with confirmation and link type, images,
  labels, editable, subscreen menus and panels, form-single with field layout, groups and accordion, form-list with row
  forms, header sort, pagination, find dialog, column picker, CSV and print links, and every field widget). The widget
  markup is in a runtime object that each script engine runs once (a module's `runtime` property, run before the first
  unit that names it), so a unit holds only the structure of its form and calls `$h.dropDown(...)` and the like. The
  attributes the client binds are listed in `webroot/js/README.md`. Differences: option text and labels are always
  encoded; a text-line `ac-transition` is a `select` that Tom Select fills, so `name_ac` is not posted; `lookup` is a text
  line (the Java macro was commented out); `show-pdf-button` links to the `html-print` page. No unit has a `<script>`.
- **Page shell.** `Header`, `Footer`, `navbar`, `Messages`, `Login` and `SecondFactor` are `.html.fancy` files, and the
  `webroot.xml`, `apps.xml`, `Login.xml`, `SecondFactor.xml`, error and `AppList` screens use them. `.ftl` locations are
  refused when rendered, so the `.ftl` files left are `WebrootVue.qvt.ftl` (qapps) and `AutoScreenMenu.qvt.ftl`, which
  belong to the qvt slice. The `vuet` text types are gone. The default theme links Font Awesome, `moqui-html.css` and
  `moqui-html.js` (with `defer`); Tom Select and SortableJS are added to `footer_scripts` and `footer_stylesheets` by the
  widgets that use them, and the client loads them itself for a fragment that has no footer. The component data files
  (theme, localization, security) were not read by `moqui load` then; since the Phase 10 merge `moqui load` and the empty-database
  load read each component's `data` directory, and the gate no longer loads them by location (slice 8).
- **Static files.** The path walk ends at a file under a screen's directory when a name is neither a subscreen nor a
  transition (`screen/static_files.rs`). A transition named `favicon.ico` would hide the file, so `webroot.xml` has none.
  Weak `ETag` from the modification time and size, 304, HEAD, a year of `immutable` for `*.min.*` or a `v` parameter and an
  hour with revalidation otherwise. The host adds tower-http's gzip layer to the whole router; brotli is not enabled, since
  its feature adds a crate for a saving gzip mostly gives.
- **Multipart.** axum's `multipart` feature needs `multer`, which is not in the tree, and the request pool takes an owned
  request, so the host reads the body (up to `webapp_max_upload`, default 100 MiB, while it reads; other bodies stop at
  `webapp_max_body`, default 10 MiB) and `web/multipart.rs` parses it. Text fields join the parameters. A file is an `UploadedFile` host object
  (`name`, `contentType`, `size`, `getString()`, `write("dbresource://...")`) in the parameters and in
  `ec.web.multiPartParameters`, with `_fileUploadList`. A file that starts like a program is a 415 unless
  `webapp_upload_executable_allow` is `true`.
- **Messages and forms.** A response that redirects saves its messages and errors in the session, and its request
  parameters when it had errors (not passwords or the token). The next request that asks gets them once: `ec.web.getPageMessages()`
  for the shell, `ec.web.errorParameters` and the field values of a form. A validation error of a form's own field is shown by that field in every client, which goes beyond Java (its qvt client only
  notified): the html page after a redirect, an html background submit (it asks for JSON and draws the same `.field-error` markup), and a qvt
  `m-form`, whose `fieldErrors` the `m-*` wrappers put on the Quasar field until its value changes (from the JSON answer, or from
  `validation-errors-initial` after a redirect). An error placed on a field is not also a notification. The `validationErrors` maps carry Java's
  `formPretty`, `fieldPretty` and `serviceNamePretty`. Pages the user got in `html` go on the screen
  history, newest first, for `screen-last` and the navbar menu. A failure for a browser (`Accept: text/html`, not an API path) renders
  `error/*` with `errorCode`, `errorType`, `errorMessage`, `errorThrowable` and `showErrorDetail`.
- **Slice 6, qvt server-merged, qjs, qvue.** `ScreenQvt.js` writes the Vue template of the target screen, as the html module does: a runtime
  object that each engine runs once (`$mqQvt`) holds the markup of each widget and a unit holds the structure of its form. The body of a `.qvt`
  response is the template, not a component object: the client wraps it as `{template: '<div id="...">' + body + '</div>'}` as the Java client
  did, so `qjs` (the script object) and `qvue` (a component file) stay companions of the same screen. The widgets are the `m-*` wrappers
  of `js/WebrootVue.qvt.js` (list in `webroot/js/README.md`); a macro writes no raw `q-input`, `q-select`, `q-date` or `q-field`. Form fields
  start from `fields-initial` (the port of `addFormFieldValue` is in the runtime, so a form-list row gets its values from the row it
  renders), a widget binds `v-model` inside a form and `model-value` outside one, and every opening brace of request text is the character
  reference `&#123;`, which keeps data from being read as an interpolation (the Java macros used `v-pre`). Differences from the FreeMarker macros are
  at the top of `ScreenQvt.js`: `text-line` with `ac-transition` is an `m-drop-down` with `server-search` (the Java macro wrote an error),
  a confirmation is a `data-confirm` attribute the shell answers, the `background-message` is text and not evaluated, `editor-type` `html`
  and `md` are `m-text-area` (`QEditor`; a text area with a Marked preview), `lookup` is a text line, the Inputmask `mask` is converted to Quasar's
  tokens, and the navigation bar of a form-list sits above the table. The column dialog is `m-form-column-config`. `ScreenPlain.js` serves `qjs` and `qvue`: the text of the screen's own `render-mode`
  elements and the subscreen and section calls. The mode a unit is made for reaches a module that serves more than one mode as
  `T.options.mode`.
- **Page shell and host.** `/qapps/**` is `webroot/screen/webroot/qapps.xml` (`allow-extra-path`) and renders `includes/WebrootVue.qvt.html.fancy`
  in the html mode; the screens it shows are fetched from `/apps/**` as `.qvt` (`.qjs`, `.qvue`) and are always standalone, so only the target
  screen renders and `pre-actions` run from the third screen of the path on. The default mode of `/qapps` is therefore html, and qvt
  is the mode of what it loads. The pre-actions of `qapps.xml` add the scripts to `footer_scripts` and build `qvtConfig`, which the shell writes
  as the one JSON block `<script type="application/json" id="moqui-config">` (shape in `js/README.md`). The `AutoScreenMenu.qvt.ftl` of the tools
  component is `AutoScreenMenu.qvt.fancy`, and `AutoEdit.xml` names it. A qvt-family response (`qvt`, `qjs`, `qvue`) that is not server-static says
  `Cache-Control: private, no-store`, and `qvt` says `X-Moqui-Qvt: merged-fragment` (`render.rs`, `add_client_component_headers`).
  `sri.getMenuData` (`screen/menu.rs`) is the `menuData` transition of `webroot.xml`: the pre-actions of the screens on the path from the third on run
  (an error in one is logged), each screen on the path has its subscreens in menu order with the ones the user may not view or that miss a required
  parameter dropped, and the target comes last with its extra path and render modes; 401 for nobody, 404 for a path that is no screen and 403 for one
  the user may not view. A grant is read with the parents of the item on the artifact stack, so it reaches the screens below. The `apps.xml` transition `l10nCatalog`
  answers the messages of a locale for `moqui.l10n.localize`. `ec.user.timeZone` and a `ec.user.userAccount` without its password columns are new host
  properties; `sri.getScreenUrlInfo().extraPathNameList` and the url methods `isScreenUrl`, `isTransition` and `getTransitionMethod` too. The
  emailed link of `ScreenRenderEmail.xml` and the `ServiceJobError` link point to `/qapps` and not to the dropped `/vapps`. The FreeMarker and GString
  renderers and caches of the Java conf are out of `MoquiDefaultConf.xml` and `MoquiDevConf.xml`.
- **The client.** `MoquiLib.js`, `MoquiL10n.js`, `WebrootVue.qvt.js` and `MoquiQzComponent.js` in `webroot/js` are plain ES2020 on Vue 3.5 and Quasar 2.35
  (the vendored libs), with `fetch` for requests, one JSON settings block, `QPopupEdit` for `m-editable`, `QEditor`, Chart.js 4, Mermaid 11
  (`mermaid.run`), QZ Tray 2.3 on `QDialog`, a vue-router stand-in for Quasar's `to`, and `loadComponent` choosing by the `X-Moqui-Qvt` header (only
  `merged-fragment` is handled; `static-template` is an error until the server-static slice installs its handler in `moqui.qvtHandlers`). The Assist
  panel (`js/assist`) is not ported. `MoquiL10n.js` checks against `crates/moqui-lib/testdata/format-vectors.json`.
- **Tests.** Golden code for `qvt`, `qjs`, `qvue` (`script/testdata/screen/<mode>/`), the rendered templates in `qvt/render/*.qvt` (each compiled with the vendored
  Vue compiler in QuickJS, `screen::vue_check`), menu data with permissions (`tests/menu.rs`), the Node tests of the client (run by `cargo test` when `node` is on the
  path), and `gate3.rs` for `ArtifactGroupDetail` as `.qvt`, `.qjs` and `.qvue`, the `/qapps` shell and `/menuData` over HTTP.
- **Slice 7, server-static qvt.** `server-static` names `qvt` (list or `all`), or the screen has only subscreens placeholders (or no widgets), and
  `statics::eligibility_of` finds nothing that needs request data: a `dynamic="true"` form, an `extends` or `auto-fields-*` named by an expression, a form that says
  its own `server-static` without qvt, a `section-include` or `include-screen` named by an expression or of a screen with forms, `allow-extra-path`, a qvt macro module other
  than the built-in one, `render-modes` without qvt. Such a screen stays a server merge, with the reason in the report and no static header. The template is `ScreenQvtStatic.js`
  (unit mode `qvt-static`), run once with no actions: `section` is `v-if`, `section-iterate` is `v-for`, a form-single is an `m-scope` over its map and an `m-form` whose
  `fields-initial` are expressions over the scope, a form-list is `m-static-form-list`. Text is `v-text` of a localized call, and `${}` text is
  `ec.resource.expand(pattern, {expr: () => expr})`. The Vue compiler (`prefixIdentifiers`, in QuickJS, read from `libs/vue/compiler-dom.cjs.js`) compiles it when the definition
  is cached; the response is the script `moqui.qvtStatic[path] = function (Vue) {...}` that a script element loads, `X-Moqui-Qvt: static-template`, `Cache-Control` of
  `screen-server-static`, a weak `ETag` over the script and the resolved definition (screen XML with `screen-extend`, each form and its extensions, the source files, the macro
  module), 304. Every screen has the implicit `actions` transition: `GET {screen}/actions` runs `always-actions` from the first screen, `pre-actions` from the third, the root
  actions and the actions of the sections it reaches, and answers the package; `GET {screen}/actions/{form}` answers a form-list. `display-entity`, `entity-options`,
  `list-options` and `dynamic-options` without `server-search` are looked up on the server (`list-options` too, which the plan left in the template), and a `set` in a field runs
  there. Differences from the plan: credential names are stripped at any depth; `include-screen` is written inline in the including template (and its actions run in the
  package) instead of a child component; tree node JSON is a 501 and form-list aggregation is not done (the report names screens that need them); first/second/last rows, row
  selection, saved finds, the column picker, the csv, text and pdf buttons, a multi form and `button-menu` are not written in a static template; the non-qvt modes get the cache header
  when the screen names them, and the automatic subscreens-only case gets it for qvt only (a page of html holds the navbar of the user). `webapp_qvt_static_only=true` loads
  `vue.runtime.global.prod.js`, and waits for the `m-*` components and the shell to be precompiled before it can be used: they still have string templates.
  The report is `moqui lint --screens --qvt-static [report-file]` (one block for each screen file: location, mode, verdict, reasons and violations; it does not fail the
  process) and `screen::statics::static_report` with `full` for the render with the wrapped `ec`. Over this tree 128 screens: 73 clean, 38 with violations, 14 stay a server merge,
  3 (the Groovy screens) do not load. `ArtifactGroupDetail` is clean, and Gate 3 marks it with a `screen-extend` in its own runtime.
- **Slice 8, the Gate 3 harness.** `tests/gate3.rs` and `tests/gate3_browser.rs` share the seeded server in `tests/common/mod.rs`: one embedded
  Postgres, the seed load (framework files and each component's `data` directory, so the themes, pagination text and the tools app groups are
  rows without a by-location load), the demo users, and three screens of the test runtime (`GateCompanion`, `GateWidgets` with click to edit,
  a background submit with a date, a drop-down and a multiple drop-down, a form-list with the column picker and 186 rows, and Chart.js and
  Mermaid in qvt text; `GateStaticList`, server-static with the same list). `gate3.rs` adds a form post through `updateArtifactGroup` (the
  session cookie and the page's token; refused without the token; the JSON answer for `Accept: application/json`), that `html-print` names no
  `moqui-html.js` and no script, and that the seed load wrote the tools and webroot data. `crates/moqui/tests/lint_cli.rs` runs the binary's
  `lint --screens` and `lint --screens --qvt-static` and checks both exit 0. The browser smoke is a small Chrome DevTools Protocol client over
  `tungstenite` (`tests/common/cdp.rs`), not `chromiumoxide` and not `PdfRenderer`: Chromium with `--headless=new`, a temporary profile, a page
  attached with a flat session, `Runtime.evaluate`, mouse and key events through `Input`, and every `Runtime.exceptionThrown`,
  `Runtime.consoleAPICalled` error or warning and `Log.entryAdded` error or warning collected as a problem.

### Gate 3 status

Every item of Gate 3 has a test, and they pass. The browser items, in `tests/gate3_browser.rs`, with the console clean on every page:

- `html` (under `/apps`, with the navbar): `moqui-html.js` once, the form values and rows, the menus moved into the navbar, a native dialog opened
  and closed, a form post through the transition that comes back to the page with the new value; click to edit; a background submit (date,
  drop-down, the multiple drop-down through Tom Select loaded on use) with its message and no navigation; the column picker with SortableJS
  loaded on use and the tree in its input; page 2 and a descending sort of the form-list.
- `html-print`: no `window.moqui`, no script, the three form-list tables. `text`, `csv` and `xml`: fetched by the page with their content types, and
  the xml parses with `DOMParser`.
- `qvt` through `/qapps`, server-merged: the shell mounts, the form-single is a `QForm` with its value, the rows show the display-entity text, a
  container dialog opens with a drop-down whose `QMenu` lists the server's options, a row's `m-drop-down` is changed through its menu and the row's
  `m-form` posts with the session token (stored in the database); Chart.js and Mermaid are absent on that page and load and draw on the
  widgets page; `m-editable` saves through `QPopupEdit`; `m-date-time` takes a day from the `QDate` popup; the form posts in the background with
  its message; the column dialog opens; page 2 of the list.
- `qvt` server-static (the fixture marked by `screen-extend`): `moqui.qvtStatic[path]` is a function, the script, `/actions` and
  `/actions/ArtifactAuthzList` were requested, the form value and rows come from the packages, there is no literal `{{`, `${` or `ec.` in the
  rendered text, the dialog and the row post work as above; on `GateStaticList` page 2 and the descending sort fetch `/actions/EnumList` and
  redraw. The same run executes the compatibility report and checks the fixture is clean.

The notification WebSocket is a separate browser test (Phase 10, below): a notification sent through the core API shows as a toast in
`html` and as a Quasar notify in `/qapps`.

Not run in the browser: file downloads (the `Content-Disposition` path of `m-form`), QZ Tray, dynamic containers and trees, `m-form-link` header find forms, the dark-mode toggle, and
the Login second factor. The `qjs` and `qvue` companions are checked over HTTP only. The smoke checks content, behavior and the console; it does
not judge layout.

## Phase 9 — SearchFacade

Depends on gate 1. Entity documents and the feed rules already assemble, and the service facade can call the receive service. The HTTP client already exists. The API is `plans/SearchFacade.md`.

- Query tree, aggregation tree, and Postgres SQL in core. When the entity datasource is Postgres, embedded or external, documents live in the `search` schema on that database.
- A separate embedded Postgres cluster when the entity datasource is not Postgres, and an external Postgres when a search URL names one. MySQL is not a search backend. Embedded search starts on Windows too, over TCP to `127.0.0.1` as `plans/EmbeddedPostgres.md` specifies.
- Indexing consumes `DataFeedPending` rows on the service pool after commit, then deletes the row. This replaces the phase 6 record that search is not up. Same-database Postgres, a second Postgres, and OpenSearch use that path.
- The query-string parser for the subset existing screens emit.
- One active backend. The OpenSearch backend builds HTTP bodies and sends them through `HttpClient`. The host `search` module starts the process. `/elastic` and `/kibana` proxies, and the log indexes, exist only when that backend is selected.
- `CountDistinct` is exact on Postgres and may be approximate on OpenSearch.

### Where the code differs from the text above

Phase 9 was checked against `SalesSummary.xml` through `SalesReportServices`, and against `SearchServices.xml` and its callers. The changes to the API are in `plans/SearchFacade.md`.

- **Document assembly.** `entity::document` is the `EntityDataDocument` port: one outer-join query per document from the field paths, rows merged into nested maps (type-one flattened, type-many as lists keyed by the relationship alias), expression fields in JavaScript after the query, query and post-query conditions, `manualDataServiceName`, and batched feed reads that page on the primary key.
- **Feed.** A write records its values and its transaction frame. Before commit the feed resolves writes to document ids, a related write by walking back along the relationship path, and inserts one pending row per document with `index` or `delete`. After commit `process#DataFeedPending` runs on the pool and calls each real-time feed's receive or delete service. The `process_DataFeedPending_frequent` job retries what is left. The Postgres `xmin` probe remains only for writes recorded without values.
- **Text** uses the `simple` configuration, not `english`, to match Elastic's `standard` analyzer.
- **Services.** `SearchServices.xml` bodies are JavaScript on `ec.search`. `search#DataDocuments` reads the Elastic JSON `nestedQueryMap` that unmigrated callers send. A `<script>` body may end with a top-level `return`.
- **Store placement** is the host `search` module: the entity Postgres `search` schema, a `postgres` url, or a dedicated embedded cluster under `runtime/db/search` when the entity database is not Postgres. The data load attaches search when it can.
- **OpenSearch** request bodies are built by the compiler in core. Starting the OpenSearch process, the `/elastic` and `/kibana` proxies, and the log indexes wait for the web phases.

`get#InventoryProjectedInfo` is a mantle rewrite onto `aggregate`. `get#PriceHistoryInfo` stays unimplemented because it has no callers. Both are named here so they are not ported as `ElasticFacade` clients. The inventory rewrite happens with mantle, outside this gate.

## Phase 10 — Mail and notifications

Depends on the service facade. The notification WebSocket also needs the phase 7 session cookie. Email-ECA rules already fire and currently stop at the unconfigured send service. This phase provides that service. Email bodies are screens from phase 8. Subject and from address go through `JsTemplateLiteral`.

- Outbound mail with `lettre` in core. The Groovy send scripts become Rust service logic. The send service from phase 6 then sends.
- The inbound path is `poll#EmailServer`: IMAP and POP3 against `EmailServer`, rustls, no `native-tls`. The crate is chosen when the dependency is added and goes through `plans/License-Review.md`. Each new message runs the email-ECA rules. The seed job stays paused. `semaphore="fail"` is the phase 6 semaphore. The embedded SMTP server stays a disabled host feature. It is not this path.
- Notification messages, the in-process topic, and the host WebSocket. The socket uses the phase 5 session and the phase 7 cookie. A message runs on the service pool with its own `ExecutionContext`, after the same handoff as HTTP. The connection does not keep one for its lifetime.
- The cluster sender from `plans/Cluster.md`: the `cluster_tls_cert` / `cluster_tls_key` configuration and the member-row key fingerprints, and HTTPS fan-out of the cache-invalidation topic and the notification topic. The local cache drop stays on the writer. The peer send is one host thread after commit, after the enlisted client is back in the pool. Member heartbeat runs on the scheduler tick. One live member means no fan-out. The same-session WebSocket test stays the single-node case.
- Markdown for screen and wiki text that asks for it.

### This pass (Phase 8 still open)

Phase 8 owns screen rendering. This pass builds the Phase 10 slices that do not call `ScreenFacade.makeRender()`.

In this pass:

- Outbound SMTP with `lettre` in core, and a test sink. `send#EmailMessage` sends a stored row. `send#EmailTemplate` expands subject and from through `JsTemplateLiteral`, writes the `EmailMessage` row, and sends when `body` and `bodyText` are already parameters. `email_allowed_hosts` and `allowedToDomains` apply. Sim mode returns `emailMessageId` `SIM` and does not connect. An empty SMTP host logs and does not send.
- `poll#EmailServer` for IMAP and POP3, rustls, no `native-tls`, if a blocking crate passes `plans/License-Review.md`. Each new message runs the email-ECA rules. The seed job stays paused. The poll loop is tested with a fake store.
- Notification messages and the in-process topic. The host WebSocket `/notws` uses the phase 7 session cookie. A text frame runs on the service pool with its own `ExecutionContext`. Job errors notify topic `ServiceJobError`. Recipient resolution is one query per source, not one query per user.
- Cluster membership, generated `cluster_tls_cert` / `cluster_tls_key`, key fingerprints, heartbeat on the scheduler tick, and HTTPS fan-out of `entity-cache-invalidate` and `notification-message`. One live member does not fan out. The peer send is `on_commit`, after the enlisted client is back in the pool.
- `pulldown-cmark` as `moqui_lib::markdown::to_html`.

Still deferred (to phase 12 where named):

- The embedded SMTP server, the russh cluster sender, and Postgres `NOTIFY`.
- PDF bytes for an `xsl-fo` attachment (phase 12).

### Screen mail after phase 8

Done once Gate 3 passed. `service/mail.rs` follows `sendEmailTemplate.groovy`; tests are in `service/mail.rs`,
`service/email_eca.rs` and `notify/tests.rs`, with fixture screens in `service/testdata/mail`.

- **Body.** A template with `bodyScreenLocation` renders that screen with `make_render` (no socket; the render installs its own web slot
  and restores the one of a request in progress, sends no session map and records no history) in `html` for `body` and in `text` for
  `bodyText`. The context is a frame with the service parameters, the `bodyParameters` over them, and the script's values (`emailTypeEnumId`,
  `ccAddresses`, `bccAddresses`, `fromAddress`, `fromName`, `subject`, `webappName`, `webHostName`, `emailMessageId`); the `bodyParameters`
  are also the render's request parameters. Links start from the conf `webapp` named by `webappName` (default `webroot`), as Java's
  `makeWebappHost` with no request: `https` and the https host and port when `https-enabled`, else `http`, an empty host is `localhost`
  (`ExecutionContextFactory::webapp_root_url`). The render runs before the `EmailMessage` row goes to `ES_READY`, as in Java.
- **`send#ScreenRenderEmail`** works end to end: its two Groovy map literals are JavaScript, and the seed templates' `classpath://screen/...`
  locations resolve (`ResourceFacade` reads `classpath://` from the framework directory, as `locate` already did). It requires a user, as in
  Java.
- **Attachments.** `EmailTemplateAttachment` rows: `attachmentCondition` (JavaScript) skips a row, `screenRenderMode` renders
  `attachmentLocation` as the root screen or `screenPath` under the root screen as last standalone, `forEachIn` renders one attachment for each
  entry (a map's fields, or `forEachEntry`, in a pushed frame), the file name is expanded in the context and defaults to the screen name with
  the mode as extension, the content type is the mode's (`text/csv`, `text/plain`, `text/xml`, `text/html`), and blank output is no attachment.
  The `attachments` parameter takes `contentText`, `contentBytes`, a screen render, or a location. A row with no mode sends the bytes at its
  location.
- **`xsl-fo`.** The attachment is skipped with a logged warning that says `PDF is not available until phase 12`, and the email goes out
  without it. That is what Java does when the PDF transform throws (`logger.warn`, no attachment, send continues). No error is added.
- **Failures.** A body or attachment render that fails, or an `attachmentCondition`/`forEachIn` that throws, stops the send: logged, and the
  message (not error) `Error sending email: ...`, as the Groovy outer catch. A created `EmailMessage` stays in draft.
- **Notifications.** `emailNotifications` Y (topic user, else topic) sends `send#EmailTemplate` with the wrapped message as `bodyParameters`,
  so `NotificationEmail.xml` renders. A saved `EmailMessage` id is returned by user (`email_message_id_by_user_id`) and, for a persisted
  notification, stored with the sent date on its `NotificationMessageUser` row, as Java's `store#NotificationMessageUser`. A failed email
  is logged, and the caller's messages and errors are restored, so the in-app send still succeeds.
- **Gate 4 check.** `screen_body_renders_html_and_text_and_sends_with_attachments`: a screen body in both modes, a `JsTemplateLiteral`
  subject, the MIME text through the test sink (`multipart/mixed` holding `multipart/alternative` with `text/plain` and `text/html`), csv
  attachments from `forEachIn`, a conditional text attachment, a skipped one, a plain file, an html attachment from the parameter, and the
  `xsl-fo` one left out. The email-ECA test sends through the same path.

Where it differs from Java:

- A `body` or `bodyText` the caller passes is used and only the missing part is rendered (the Groovy script declares the parameters but
  always renders both). With no `bodyScreenLocation` and neither part, the send is the error `email has no body`.
- The text part is rendered only when the screen's `render-modes` allow `text`; Java rendered text unconditionally. Blank output is no part.
- `EmailTemplate` has no locale field: the body renders in the locale of the user the service runs as, which is Java's behavior too.
- There is one root screen, so `screenPath` attachments render under it; `webHostName` does not pick a root screen by host. It is still in
  the context, where `ScreenRenderEmail.xml` and `NotificationEmail.xml` use it for links.
- A screen render accepts an anonymous login (an `anonymous-view` service such as `send#EmailTemplate`) for a screen that requires
  authentication, as Java's `getLoggedInAnonymous()` check does; before, only a named user passed (`screen/render.rs`).
- An `attachments` entry that has no content, screen, or location is logged, as in Java, not added as an error.

### After Phase 8: markdown and the browser notification client

- **Markdown.** The template registry has the renderer classes `markdown` (`.md`, `.markdown`) and `markdown-fancy` (`.md.fancy`,
  `.markdown.fancy`, the port of Java's `.md.ftl`), in `MoquiDefaultConf.xml` and in the fallback list of a factory with no conf. So
  `ec.resource.template` and a screen `<text location>` (`sri.renderText`) render a markdown file, which is how a wiki page is shown.
  The markdown is CommonMark plus GitHub tables, and raw HTML passes through as with Java's flexmark: a markdown file is a trusted
  template file, not sanitized. The HTML of a file is kept by location and modified time, as Java's `resource.markdown.location` cache.
  `ec.resource.expand` does not render markdown, in Java or here. Differences: the flexmark `[TOC]` block is not rendered; in
  `.md.fancy` the tags, comments, `${}` and escapes are set aside before the markdown pass and put back after it (Java ran markdown
  over the FreeMarker source, so markdown could escape an expression), a tag alone in a paragraph loses the `<p>`, and `${}` output is
  HTML escaped as in `.html.fancy`. A `<text>` picks its mode by `type` as in Java: a wiki-style `type="html,qvt"` renders the HTML in
  both modes (in qvt it is part of the Vue template, as in Java, so the file is trusted there too), and a text mode `<text template="false">`
  writes the source. The wiki screens are not in this tree (they are in HiveMind); `WikiServices.xml` renders no markdown. Tests: the
  renderer unit tests in `script/template/tests.rs` and `screen/tests/markdown.rs` (a `.md` and a `.md.fancy` text in html, in qvt
  compiled by the Vue compiler, and the source in text).
- **Notification client.** `webapp_notification_endpoint` defaults to `/notws`, the Java endpoint. The html header names it
  (`meta[name=moqui-notify]`) and the `/qapps` settings carry it only for a signed in user, so a page with no user opens no socket.
  `moqui-html.js` subscribes to `ALL` when the socket opens, shows a notification with a title and `showAlert` as a toast (with its
  link), and reopens a closed socket after a growing delay, at most ten times in a row. The qvt client is the Java one: `ALL`, a desktop
  notification when the browser allows it, else a Quasar notify, and six reopen tries 30 seconds apart. Two server changes came with it:
  every socket of a session is registered (the first port kept one per session, so a second tab got nothing; Java keys its endpoints by
  the WebSocket session, one per connection), and the upgrade reads the user from the live session of the node before the `HttpSession`
  row, because the eventual flush has not written a fresh login when the page opens its socket. Test: `gate3_browser.rs`
  `a_notification_sent_on_the_server_shows_in_the_html_and_qapps_pages` (the login page has no socket; after sign in a
  `NotificationMessage` sent through the core API shows in `html` and in `/qapps`, and the console stays clean).

## Phase 11 — System messages, entity sync, user remainder, and the other tools

Depends on the HTTP client and the user facade. LLM and A2A were split out of this phase into phase 13.

- ElFinder on the host, when the file API stays.
- Artifact hit bins and the flush that writes them. The flush is a subscriber on the phase 2 artifact spans and runs as reserve-connection work on the scheduler tick. It still writes `ArtifactHit` and `ArtifactHitBin`, and the tools screens still read those tables.
- System messages, a port of the Java services, no redesign. `queue#SystemMessage` inserts `SmsgProduced` in a new transaction (`force-new`), then `sendNow` (the default) submits `send#ProducedSystemMessage` as local async. Senders are the type's `sendServiceName`: REST or direct local. `send#SystemMessageJsonRpc` is not ported. Status moves `SmsgProduced` to `SmsgSending`, then `SmsgSent`, or back to the prior status on failure. `receive#IncomingSystemMessage` is the `/rest/sm` transition from phase 7. Consume runs the type's consume service. Ack uses `produceAckServiceName`. `reset#SystemMessageInError` puts an error row back to produced or received. The paused jobs `send_AllProducedSystemMessages_frequent` and `consume_AllReceivedSystemMessages_frequent` stay paused.
- Entity sync, a port of `EntitySyncServices`, no redesign. `run#EntitySyncAll` finds sync rows that are not running and submits `run#EntitySync` as local async. One sync holds its `EntitySync` row. Includes select records since the last stamp. Push calls `put#EntitySyncData` on the remote. Pull calls `get#EntitySyncData`. Both ride `RestClient`. The Java push uses `callJsonRpc`. That client is not ported. A history row records the run. The seed job `run_EntitySyncAll_frequent` stays paused. This is not the cluster sender. Cluster still moves cache invalidation and notifications only. The streaming cursor that serves the include query is the `DbDriver` arm from phase 3.
- ETL: `SimpleEtl` and `FlatXmlExtractor` in lib. `sgml` in lib. JDBC extract beside `DbDriver` in the host.
- EDI handler in core.
- `dbresource://` is the phase 4 scheme. JCR is dropped. Resolving a `content://` location is an error that names the location. It is not remapped onto blob storage. Filesystem and S3 are their own resource implementations when a deployment asks for one.
- Cache admin and the other tools services that wrap facades already built.
- Print-job records. Producing the PDF is phase 12. Until then a print or email path that asks for PDF reports that PDF is not available.
- User remainder: preferences, password reset, TOTP, email second factor, and the admin side of API keys and login keys. The `sendOtp` and `verifyOtp` transitions start working here.
- The Groovy shell is not replaced with a script console, and there is no web eval endpoint even in development: the hardened shell is still too risky. The crates are `russh` and `russh-sftp` (`plans/Ssh.md`). An SSH server in the host, authenticated through the user facade and artifact authz, is the later shape for admin access and pairs with `moqui-sftp`. The server is not part of the port.
- Definition type generation (`plans/TypeGeneration.md`): TypeScript declarations from entity, service, and screen definitions, emitted by a CLI subcommand after definitions load.

### Where the code differs from the text above

Phase 11 was built in slices on branch `phase11`, then reviewed against the Java code. Tests are named in each slice's commit.

- **User remainder.**
  - `user/preference.rs` covers preferences: property, then `UserPreference`, then the user's `UserGroupPreference` rows by `groupPriority`, with `ALL_USERS` as the default group.
  - `user/factor.rs` covers TOTP (`totp-rs`: SHA1, 6 digits, 30 s, skew 1), single-use codes and the `getLoginKey` port.
  - `UserServices.xml` is JavaScript.
  - A login that needs a factor rotates the session and sets the pre-auth attributes. `login`, `sendOtp` and `verifyOtp` on `rest.xml` finish it.
  - Whether a factor is required is decided in Rust and not by calling `get#UserAuthcFactorRequired`, so a boot without `UserServices` cannot skip it. An override of that service does not change login.
  - Passwords and reset passwords are argon2.
  - `setup#UserAuthcFactorTotp` returns the `otpauth://` URI and no QR image, because `totp-rs` is linked without `qr`.
- **System messages and entity sync.**
  - `SystemMessageServices.xml` is JavaScript.
  - `/rest/sm` (`service/system_sync/message.rs`) follows `WebFacadeImpl.handleSystemMessage`. The HMAC is compared in constant time.
  - `send#SystemMessageJsonRpc` is an error that names `send#SystemMessageRest`.
  - Entity sync runs over REST (`/rest/s1/moqui/entity/syncs/data/{get|put}`). `targetServerUrl` is the remote server root, so rows that name `/rpc/json` need a new URL.
  - `get#EntitySyncData` applies each entry's `includeFilterList`. The Java loop never did, so this changes which records sync.
  - Inline and script services get an isolated context, as Java's `pushContext` does.
  - A `service-call` name with `${}` is expanded.
- **ETL, sgml, EDI.**
  - `moqui_lib::etl` and `moqui_lib::sgml`.
  - `EntityFacade::make_etl_loader` and `EntityFind::extract` in core.
  - `JdbcExtractor` in the host, on its own driver connection.
  - `EdiHandler` in core, with a script global.
  - The `EtlLoader` try-insert is an upsert or `create_or_update`, as in the data loader, because a failed statement marks the transaction rollback-only.
  - EDI structure files are JavaScript expressions, so mantle's `X12*.groovy` grammars need that conversion. Script code calls `EdiHandler.makeAk3(segmentError)`.
  - The Groovy pre-scan for a tree outside this repository is `moqui lint --groovy <dir> ...` (screens, forms, services, ECA rules and data files; it lists the `.groovy` files and fails nothing). `moqui convert --groovy [--write] <dir> ...` makes the mechanical rewrites in the expression places of XML: Groovy map literals become objects, `context + [...]` becomes `Object.assign({}, context, {...})`, Elvis becomes `||` (not before a ternary at its depth), and each grammar file gets a `.js` copy that the locations then name. A map after any other `+` is left, since in JavaScript it would join text. Without `--write` it prints each change. The six mantle-edi grammars, converted, are fixtures in `edi/testdata/mantle` and load in `EdiHandler`.
  - Measured on 2026-10-14 against the mantle repositories, which were not changed. Lint sites (files): mantle-usl `context-plus` 85 (28), Elvis 629 (53), map literal 1598 (63); SimpleScreens `context-plus` 26 (18), Elvis 798 (164), map literal 976 (214); mantle-udm Elvis 41 (2); mantle-edi Elvis 16 (4), map literal 121 (4), and the 6 grammars. A dry `convert --groovy` over the four makes 2513 map-literal, 111 `context-plus` and 1504 Elvis rewrites in 326 files, rewrites 15 grammar locations, and leaves 72 places for review.
  - In Java, a configuration added with `addConfiguration` never runs. Here it runs.
- **Type generation.**
  - `moqui types [--out dir]` sits beside `load` and `lint`, because there is no `run` command.
  - Members the runtime does not have are left out, for example `ScheduledJobRunner`.
  - Entity fields are typed as properties, as well as with `get` and `set`.
  - The `tsc` checks skip with a logged message when `tsc` is not on PATH.
- **Entity value property access.** This was not in the bullets above.
  - `value.field` reads and writes in scripts and trusted templates, and so do `value['field']` and a relationship name.
  - Entity lists index, have `length`, and support `for...of`, `map`, `filter` and `forEach`.
  - A method name wins over a field name.
  - Reading an unknown name gives `undefined`, where Java throws. Writing one throws, and so does `.set`.
  - `list[0]` is the row of the list, so `list[0].x = y` and a `<set>` on an `<iterate>` entry change the list, as in Groovy. `plans/ContextObjects.md` has the rules.
- **Artifact hits.**
  - Hits are counted when an artifact frame pops (`context/hits.rs`), not by a `tracing` subscriber.
  - The scheduler tick writes `ArtifactHit` and `ArtifactHitBin` on a reserve connection, and shutdown flushes what is left.
  - A bin whose length has passed is written on the tick, without waiting for the next hit.
  - The parameter string leaves out credential names, ignoring case.
- **ElFinder.**
  - `service/elfinder.rs` implements the commands Java implements, over `dbresource://` and the resolved file roots, with Java's hashes.
  - `ec.web.sendResourceResponse` is a port of Java's.
  - The elFinder client, jQuery and jQuery UI load from cdnjs as in Java. They are not vendored, because elFinder is not on npm and jQuery was removed (`plans/License-Review.md`).
- **Tools.**
  - The `ServerServices`, `BasicServices`, `EntityServices`, `ServiceServices` and `ScreenServices` bodies are JavaScript, or Rust under `ServiceKind::Tools`.
  - Cache admin runs on `MCacheManager`.
  - `EntityDataWriter` writes text, files and stored zips.
  - `tools_host.rs` is the script host the tools screens call. `plans/ContextObjects.md` has it under "Added by phase 11 (tools)".
  - The JVM parts of the dashboard, cache and thread screens show process figures from `/proc` instead.
  - Every tools screen renders in `html` (`tools_screens.rs`). GroovyShell and the log viewer say why they have nothing to show. `ViewDbView` says that DbViewEntity views are not in the entity model.
- **Print jobs.**
  - `print#Document` stores the `PrintJob` and marks the send `PtjSendFailed`, with "Network printing is not available until phase 12 (moqui-cups)".
  - `print#ScreenDocument` reports that PDF is not available until phase 12.
- **Not ported, with an error that says so.** These are `ServiceKind::NotPorted`:
  - LLM and A2A (phase 13).
  - `InstanceServices`.
  - The wiki services, whose screens are in HiveMind.
- **Framework gaps closed along the way.**
  - A form-list `entity-find` selects the displayed and hidden fields, as Java's `getListObject` does.
  - `entity-find-one` with a full primary key finds by the primary key only.
  - `select-field` takes a comma list.
  - Text `List` and `Set` parameters are split.
  - An uploaded file passed into a service stays a file.
  - The Groovy idioms `context + {...}` and Elvis are gone from the seed data and the tools screens.

Fixed after the review against Java. Several of these predate phase 11:

- **Transactions.**
  - The `force-new` park budget is per thread, and nesting works (see phase 3).
  - Rows held by the transaction cache belong to their frame, so an inner `force-new` commit or rollback no longer writes or drops the outer frame's rows.
  - Holding is on only for `cache` and `force-cache`. `use-transaction-cache="false"` turns it off.
  - `in_transaction()` is Java's `isTransactionInPlace`.
  - The ETL loader begins a transaction on every entity group it writes.
  - `ignore` runs inside the transaction in place, as in Java, and an error in it marks that transaction rollback-only. `ignore` plus `force-new` (`TransactionAttribute::IgnoreSuspend`) suspends it and runs with none.
  - `no-tx-cache="true"` writes the frame's held rows and stops holding before the service runs, as Java's `flushAndDisableTransactionCache`. It acts on the manager of the service's entity group.
- **Authorization.**
  - Service and entity REST push an `AT_REST_PATH` frame for each path segment, as `RestApi` does. The `/rest/s1/moqui` ADMIN rule and the grants it passes down now apply.
  - Login checks `ipAllowed` on the account and its groups.
- **Hardening beyond Java.**
  - A sync load with restricted entities refuses a restricted record at any depth, and refuses service elements.
  - A second-factor code stands in for the old password only for the pre-authenticated user's own account.
  - A wrong code counts as a failed login.
  - The `/rest/sm` replay check is an atomic `put_if_absent`.
  - The artifact-hit parameter string leaves out credentials, codes, `sql`, secrets and tokens, and the in-memory hits are capped.
- **ElFinder.** A target that does not decode, or that leaves the volume, is `errFileNotFound`, never the root. `rm`, `rename`, `put` and upload are refused on the root.
- **SQL runner.** It reads a query through a cursor to one row past the limit. `ec.logger.log` takes Java's numeric levels.
- **Script runtime.**
  - A list or map a script holds keeps its JS identity across a host call that leaves the stack value unchanged. Before, `list.push(ec.service.sync()...call())` and an alias kept across a call lost their writes.
  - When the host call changed the value, the host wins.
  - An entity-value property write through the prototype chain lands on the receiver.

## Phase 12 — PDF, later drivers, and the tool components

Depends on the `html-print` render from gate 3, and on the `DbDriver` trait from phase 3.

- `PdfRenderer` in the host, as Tool components and print in `plans/Architecture-Proposal.md` describes. One long-lived Chromium, `chromiumoxide` 0.9, the external-binary recipe, and `pdf-servo` left off. The wait does not hold the enlisted connection.
- Rewrite the SimpleScreens `*.xsl-fo.ftl` documents to `html-print` templates. Callers that store render mode `xsl-fo` to mean PDF render `html-print` and pass it to `PdfRenderer`. The data-prep services stay. The screen `<text type="xsl-fo">` elements that include those templates stay valid through the alias.
- `moqui-fop` is not ported. `moqui-poi`, `moqui-image`, `moqui-cups`, and `moqui-sftp` are each weighed against binary size the way `moqui.war` size kept them out of the Java framework. A component that stays outside is listed as outside, with that reason. `moqui-cups` consumes the PDF bytes. When `moqui-sftp` is built, the client is `russh-sftp` on `russh`, not a new stack.
- Further sync drivers, each a dialect plus a `DbDriver`: SQL Server, Oracle, DB2, DB2 for i, Derby, HSQLDB. A missing driver is a configuration error that names the database. None of them block the other gate 4 work.

### Gate 4 — tools and utilities

The gate is a checklist. Each item is its own test. Entity and service behavior is covered by the earlier phase tests. This checklist covers the tools that call them.

- SearchFacade indexes one data document on Postgres and a query returns it. The feed from phase 4 is the writer. An OpenSearch run is a second test. The default remains Postgres.
- An email whose body is a screen renders in `text` and `html`, the subject is a `JsTemplateLiteral` expansion, and the send path accepts it. A test sink is enough. An email ECA that calls send uses that path.
- A notification sent from a service arrives on the WebSocket for the same session the HTTP request used.
- A system message is queued, sent, received, and consumed. An entity sync push and pull each complete with a history row.
- A user logs in with TOTP and with an email second factor, and a password reset email goes through the test sink.
- The flush writes artifact hit bins, and the tools screen reads them.
- An ETL flat-XML extract loads entity rows, and an EDI document round-trips.
- When `pdf-chromium-path` names a Chromium binary, one `html-print` document comes back as PDF bytes and a print-job record can hold them. When the binary is absent, the same call reports that PDF is not available, and the gate still passes.
- Every entry in the tool-component list is either implemented or recorded as staying outside. `moqui-fop` is recorded as not ported.
- A later database driver, when one is included in the build under test, runs the same metadata and insert smoke test as MySQL. A build that omits them still passes the gate.
- Type generation emits declarations for a sample of framework and tools services and screens, and the output passes `tsc --noEmit`.
- `/health` reports ready and `/metrics` reports the pool gauges.

Scheduled jobs are a phase 6 test. Sim mode is a phase 4 test. They are not repeated on this checklist.

## Phase 13 — LLM and A2A

The LLM client, the agent and assist parts, and A2A are new in the Java framework and not yet stable. They are ported as the Java framework stands when this phase begins, not as it stood when the port began. They come after gate 4 and do not block it.

Depends on the HTTP client, the user facade, the screen facade for screen-search hints, and SearchFacade. The LLM client sets the sim-allow exception on `RestClient`.

- LLM client, conversation, protocols, and the tool loop. Host routes for the LLM endpoints. A stream stays in the host.
- A2A routes in the host, following `../moqui/framework/plans/A2A-Server.md`. This document does not restate that plan. Streams do not hold an `ExecutionContext`.

Until this phase, the copied XML (`LlmServices.xml`, `A2AServices.xml`, and the LLM and A2A parts of the tools component) stays in the tree and is not wired, as was done for JSON-RPC. A call to one of those services fails with an error that names the service. It does not quietly run nothing.

Check: LLM and A2A cover the same cases as the Java tests for those facades at the time of the port.

## Tests

Unit tests live in the crate that owns the code. Lib tests do not start a factory. Each facade phase has its own tests before the next phase starts. Phase 4 tests do not call a service. Phase 6 tests do not open a socket. Phase 7 tests do not render a widget.

Generated code is pinned by golden tests: one fixture XML per action and screen element, with the expected JavaScript checked in. The macros are the framework's compiler backend (`plans/Architecture-Proposal.md`, Templating), and these tests are what make an edit to them safe.

The ported tests pin intended behavior. A parity harness covers the rest: the Java framework and MR-Moqui run the same fixtures and requests against the same data, and the harness diffs the observable output (REST JSON, screen output in each kept mode, and the generated SQL). It runs at the gates and its diff report is a gate artifact: what still differs, and why, with the intentional redesigns from the README named. The Java runtime stays runnable for the life of the port. mantle-usl's Spock tests are the first application oracle. A review counted 10 of them. They are diffed, not rewritten first. That component is outside this repository.

The gates are integration tests on the binary or on `moqui-host`:

| Gate | Test |
|---|---|
| 1 | Load framework seed, read it back, run the service-element fixture, MySQL smoke test |
| 2 | The `enumTypes` service REST request, the entity REST request, login, and the 401 and 403 cases |
| 3 | One real screen in each kept mode, plus one server-static `qvt` pair (template and `/actions`) |
| 4 | The checklist above, one test each |

The Java Spock suite and the Python tests under `../moqui/framework/test` are ported as the area they cover lands. The HTTP security tests start after gate 2 and extend through gate 3. They are not a separate phase.

## Scope

This plan does not restate search, server-static `qvt`, embedded Postgres, context objects, or the crate cut. It does not split the entity facade or the service facade across a gate. It does not migrate `mantle-usl` from Groovy to JavaScript. XML actions in that component stay XML. Expressions and `<script>` bodies there are application migration after the gates that load data and render screens.
