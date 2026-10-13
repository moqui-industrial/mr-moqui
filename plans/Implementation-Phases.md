# Implementation Phases

Bottom-up build order for the MR-Moqui port, and the four gates that mark it.

This follows `plans/Architecture-Proposal.md` and `plans/Crate-Layout.md`. Search, server-static qvt, embedded Postgres, context objects, and cluster coordination stay in their own plans. This document says when each body of code lands. It does not restate them.

The entity facade and the service facade are whole parts. Both are finished before the first gate. That gate is the integration proof that they load a data file. It is not the line that cuts those facades down.

Mantle and the other application components stay outside this repository. They are the first consumer after a gate, not a phase. There are no dates or pull-request cuts here.

## Gates

1. **Data file load.** The entity facade and the service facade are already complete. Startup creates tables, framework seed XML loads, and a data-file element that names a service runs that service.
2. **REST API request.** The HTTP edge and a basic screen facade run `rest.xml`. Entity REST and service REST are the facade methods from gate 1, called through a transition.
3. **Screen rendering.** One real screen renders in every kept mode: `text`, `csv`, `xml`, `html-print`, `html`, `qvt`, plus the `qjs` and `qvue` companions. The name `xsl-fo` selects `html-print`. `vuet` is absent. Server-static `qvt` is the last slice of this gate.
4. **Tools and utilities.** The other facades and tool services: search, mail transport, notifications, LLM and A2A, ETL, and the tool components. This is not a second pass over entity or service.

## Decisions

- Build bottom-up. A facade is finished, including the features that would otherwise be left as a remainder, before any gate that depends on it. Gate 1 depends on the whole entity facade and the whole service facade.
- The entity facade is one phase: definitions, SQL, CRUD, database metadata, view-entity, dynamic view, master definitions, entity cache, transaction cache, sim mode, `text-fts` (column, index, and `matches`), audit log, field encryption, localized fields, ECAs, the data loader, the data writer, and `EntityFacade.rest`. Document assembly and feed rules live here too. The index those feeds write is `SearchFacade`, which stays gate 4. Until that facade exists, feed dispatch inserts the pending row and the receive service records that search is not up. It leaves that row. It does not grow a second, temporary index.
- The service facade is the next phase, after the entity facade and after the user and artifact-authz checks those calls enforce. It includes definitions, entity-auto, inline, script, the full XML action library, QuickJS, SECAs, email-ECA rules, `service-group`, local `async="true"`, service `callFuture()`, the scheduler and scheduled jobs, parameter validation, `RestApi`, and the remote REST runner. `async="distribute"` stays dropped, as the architecture proposal already decided. `callFuture()` is the same factory thread and new `ExecutionContext` as `async="true"`, plus a oneshot the caller waits on.
- JSON-RPC is not included. Moqui's generic service endpoint (`rpc.xml`, `handleJsonRpcServiceCall`, service type `remote-json-rpc`, `callJsonRpc`, `send#SystemMessageJsonRpc`) was not used seriously, and its authorization semantics are a reason XML-configured REST exists. There is no dispatcher, no remote JSON-RPC runner, and no `/rpc` route. A service of type `remote-json-rpc` fails registration. Copied Java XML that names it stays in the tree and is not wired. The A2A protocol binding is a different wire format and stays in the phase 11 plan.
- Tool services are not the service facade. Send-mail, search indexing, LLM, print, and system-message handling are gate 4 service bodies on top of a facade that can already call them. Email-ECA rules are the service facade. The `lettre` transport they eventually call is gate 4.
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
11 LLM, A2A, and the other tools
12 PDF, later database drivers, and the tool components
        Gate 4 — tools and utilities
```

Inside phase 4 the entity code is still ordered bottom-up: definitions and SQL, then metadata, then the write-path features (cache, sim, full-text, ECA matching, loader and writer). ECA actions and data-file service elements call the service facade, so they are turned on at the end of phase 6, still before gate 1. That is the one cycle between the two facades. Entity is built first, with those callbacks stored and not yet run. Service is built second. The gate runs only after both sides of the callback exist.

Phase 9 depends on gate 1 and can run beside phases 7 and 8. Phase 10's mail depends on the service facade. The notification WebSocket also needs the phase 7 session cookie. Neither waits on gate 3. An email body that is a screen uses the phase 8 renderers. Phase 11 needs the HTTP client from phase 6, the screen facade, and SearchFacade where a tool calls search. Phase 12's PDF needs the `html-print` render from gate 3. Later database drivers in phase 12 need only the `DbDriver` trait from phase 3.

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

- `TransactionFacade` and the transaction manager, in core, calling `DbDriver`: begin-or-join, commit, rollback, savepoint, `setRollbackOnly`, timeout, `ignore`, `use-or-begin`, `force-new`, `cache`, `force-cache`, and the synchronization list (transaction cache, `runOnCommit` / `runOnRollback`, service ECAs, data feed). `begin` records the deadline and does not check out. The first statement enlists one connection. The feed hook follows `plans/SearchFacade.md` and the fixed phases in the Transactions section: after the cache flush and before commit, the feed inserts the pending row on the enlisted connection. Phase 3 registers that insert. Phase 6 points the after-commit slot at the receive service, which records that search is not up and leaves the pending row. Phase 9 consumes the row with the index write. `suspend` parks that client on the transaction, off the free list. `resume` checks the same client out. `force-new` is that suspend followed by begin, on the same thread and the same `ExecutionContext`. The park budget is 1. The thread holds one ordinary connection per pool. The parked client still counts against the pool max. A reserve inside the pool (default 8) serves side-channel work. Startup fails when `(request cap + service cap) × 2` exceeds `pool_max - reserve`. One deadline per transaction, default 60 seconds. The timer marks rollback-only and cancels the active statement. `destroyAllInThread` runs in a `finally` block on every request, job, and async-service thread. No JTA types and no XA two-phase commit. The rules are the Transactions section of `plans/Architecture-Proposal.md`. The transaction-cache hooks used in phase 4 sit on these same calls.
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

Appended slice by slice.

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
  line (the Java macro was commented out); a form-list with `group-by` sub-lists renders every column in the main row; the
  saved-find and column dialogs show only when the screen has `formSaveFind` and `formSelectColumns`, which no screen has
  yet; `show-pdf-button` links to the `html-print` page. No unit has a `<script>`.
- **Page shell.** `Header`, `Footer`, `navbar`, `Messages`, `Login` and `SecondFactor` are `.html.fancy` files, and the
  `webroot.xml`, `apps.xml`, `Login.xml`, `SecondFactor.xml`, error and `AppList` screens use them. `.ftl` locations are
  refused when rendered, so the `.ftl` files left are `WebrootVue.qvt.ftl` (qapps) and `AutoScreenMenu.qvt.ftl`, which
  belong to the qvt slice. The `vuet` text types are gone. The default theme links Font Awesome, `moqui-html.css` and
  `moqui-html.js` (with `defer`); Tom Select and SortableJS are added to `footer_scripts` and `footer_stylesheets` by the
  widgets that use them, and the client loads them itself for a fragment that has no footer. The component data files
  (theme, localization, security) are not read by `moqui load`; the gate loads them by location.
- **Static files.** The path walk ends at a file under a screen's directory when a name is neither a subscreen nor a
  transition (`screen/static_files.rs`). A transition named `favicon.ico` would hide the file, so `webroot.xml` has none.
  Weak `ETag` from the modification time and size, 304, HEAD, a year of `immutable` for `*.min.*` or a `v` parameter and an
  hour with revalidation otherwise. The host adds tower-http's gzip layer to the whole router; brotli is not enabled, since
  its feature adds a crate for a saving gzip mostly gives.
- **Multipart.** axum's `multipart` feature needs `multer`, which is not in the tree, and the request pool takes an owned
  request, so the host reads the body (up to `webapp_max_upload`, default 100 MiB, while it reads; other bodies stop at
  1 MiB) and `web/multipart.rs` parses it. Text fields join the parameters. A file is an `UploadedFile` host object
  (`name`, `contentType`, `size`, `getString()`, `write("dbresource://...")`) in the parameters and in
  `ec.web.multiPartParameters`, with `_fileUploadList`. A file that starts like a program is a 415 unless
  `webapp_upload_executable_allow` is `true`.
- **Messages and forms.** A response that redirects saves its messages and errors in the session, and its request
  parameters when it had errors (not passwords or the token). The next request that asks gets them once: `ec.web.getPageMessages()`
  for the shell, `ec.web.errorParameters` and the field values of a form. Pages the user got in `html` go on the screen
  history, newest first, for `screen-last` and the navbar menu. A failure for a browser (`Accept: text/html`, not an API path) renders
  `error/*` with `errorCode`, `errorType`, `errorMessage`, `errorThrowable` and `showErrorDetail`.

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

## Phase 11 — LLM, A2A, and the other tools

Depends on the HTTP client, the user facade, and the screen facade for screen-search hints. Search-backed tools wait on phase 9. The LLM client sets the sim-allow exception on `RestClient`.

- LLM client, conversation, protocols, and the tool loop, as the Java framework stands when the port begins. Host routes for the LLM endpoints. A stream stays in the host.
- A2A routes in the host, following `../moqui/framework/plans/A2A-Server.md`. This document does not restate that plan. Streams do not hold an `ExecutionContext`.
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
- LLM and A2A cover the same cases as the current Java tests for those facades.
- When `pdf-chromium-path` names a Chromium binary, one `html-print` document comes back as PDF bytes and a print-job record can hold them. When the binary is absent, the same call reports that PDF is not available, and the gate still passes.
- Every entry in the tool-component list is either implemented or recorded as staying outside. `moqui-fop` is recorded as not ported.
- A later database driver, when one is included in the build under test, runs the same metadata and insert smoke test as MySQL. A build that omits them still passes the gate.
- Type generation emits declarations for a sample of framework and tools services and screens, and the output passes `tsc --noEmit`.
- `/health` reports ready and `/metrics` reports the pool gauges.

Scheduled jobs are a phase 6 test. Sim mode is a phase 4 test. They are not repeated on this checklist.

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
