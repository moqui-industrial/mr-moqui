# Implementation Phases

Bottom-up build order for the MR-Moqui port, and the four gates that mark it.

This follows `plans/Architecture-Proposal.md` and `plans/Crate-Layout.md`. Search, server-static qvt, embedded Postgres, and context objects stay in their own plans. This document says when each body of code lands. It does not restate them.

The entity facade and the service facade are whole parts. Both are finished before the first gate. That gate is the integration proof that they load a data file. It is not the line that cuts those facades down.

Mantle and the other application components stay outside this repository. They are the first consumer after a gate, not a phase. There are no dates or pull-request cuts here.

## Gates

1. **Data file load.** The entity facade and the service facade are already complete. Startup creates tables, framework seed XML loads, and a data-file element that names a service runs that service.
2. **REST API request.** The HTTP edge and a basic screen facade run `rest.xml`. Entity REST and service REST are the facade methods from gate 1, called through a transition.
3. **Screen rendering.** One real screen renders in every kept mode: `text`, `csv`, `xml`, `xsl-fo`, `html`, `qvt`, plus the `qjs` and `qvue` companions. `vuet` is absent. Server-static `qvt` is the last slice of this gate.
4. **Tools and utilities.** The other facades and tool services: search, mail transport, notifications, LLM and A2A, ETL, and the tool components. This is not a second pass over entity or service.

## Decisions

- Build bottom-up. A facade is finished, including the features that would otherwise be left as a remainder, before any gate that depends on it. Gate 1 depends on the whole entity facade and the whole service facade.
- The entity facade is one phase: definitions, SQL, CRUD, database metadata, view-entity, dynamic view, master definitions, entity cache, transaction cache, sim mode, `text-fts` (column, index, and `matches`), audit log, field encryption, ECAs, the data loader, the data writer, entity sync, and `EntityFacade.rest`. Document assembly and feed rules live here too. The index those feeds write is `SearchFacade`, which stays gate 4. Until that facade exists, feed dispatch calls the receive service and the service records that search is not up. It does not grow a second, temporary index.
- The service facade is the next phase, after the entity facade and after the user and artifact-authz checks those calls enforce. It includes definitions, entity-auto, inline, script, the full XML action library, QuickJS, SECAs, email-ECA rules, `service-group`, local `async="true"`, the scheduler and scheduled jobs, parameter validation, `RestApi`, the JSON-RPC dispatcher, and the remote REST and remote JSON-RPC runners. `async="distribute"` stays dropped, as the architecture proposal already decided.
- Tool services are not the service facade. Send-mail, search indexing, LLM, print, and system-message handling are gate 4 service bodies on top of a facade that can already call them. Email-ECA rules are the service facade. The `lettre` transport they eventually call is gate 4.
- XML actions are one MiniJinja macro set and one QuickJS runner, inside the service facade, before gate 1. Screen transitions reuse that runner. There is no smaller action subset built only for the load gate.
- The script host at the end of the service facade exposes `ec.entity`, `ec.service`, `ec.message`, `ec.logger`, `ec.resource`, `ec.user`, and the static helpers from `plans/ContextObjects.md`. `ec.web` and `sri` grow with the web and screen phases. The method list on those objects is still the open plan in `ContextObjects.md`. This document says which facade must answer before which gate.
- User login and artifact authorization are in place before the service facade is called complete, because entity operations and service calls consult them. The load path still disables authorization, as `ExecutionContextFactoryImpl.checkEmptyDb` does. The signed session cookie and HTTP basic auth are the web edge at gate 2. `UserFacade.loginUser` does not need a socket.
- `HttpClient` and `RestClient` land in `moqui-lib` with the service facade, because the remote runners are part of that facade. The sim predicate default-denies. Sim itself is already implemented in the entity and transaction work. The LLM provider exception is wired when the LLM client lands.
- MySQL is in the same driver phase as embedded Postgres and external Postgres. Gate 1 is demonstrated on embedded Postgres and repeated as a metadata, insert, and select smoke test on MySQL. Further databases (SQL Server, Oracle, DB2, Derby, HSQL) are new `DbDriver` implementations under gate 4. The facade is already written against the trait. H2 has no driver.
- `signer` lands in `moqui-lib` with the entity facade, because field encryption uses it. `etl` and `sgml` wait for the tools phase.
- Security ECAs run during the load gate. `Security.eecas.xml` fires on `UserGroupMember` and `UserGroupPermission`, and framework seed writes `UserGroupPermission`. `UserSecurityChecks` are Rust functions on the script host.
- Second factor, password reset, and API-key admin wait for the tools phase. Gate 2 uses HTTP basic auth and the login transition for a user with no second factor.
- Swagger, RAML, and JSON Schema transitions on `rest.xml` close the REST phase. Gate 2 does not depend on them. `RestApi.run` itself already exists from the service facade.
- Basic `ec.l10n` format and localize land with screen macros. Localization admin screens are ordinary screens.
- Server-static `qvt` closes gate 3, after server-merged `qvt` renders. It follows `plans/QvtServerStatic.md` and stays opt-in.
- Search is implemented under gate 4. The design is `plans/SearchFacade.md`.
- PDF and the tool components (`moqui-fop`, `moqui-poi`, `moqui-image`, `moqui-cups`, `moqui-sso`) are the tail of gate 4. `xsl-fo` output exists at gate 3. `PdfRenderer` stays the empty host hook until an engine is chosen against binary size.
- The Groovy shell endpoint is dropped. There is no Java service runner and no Groovy runner. Framework script services become JavaScript or Rust.

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
10 mail transport, Liquid, notifications
11 LLM, A2A, and the other tools
12 PDF, later database drivers, and the tool components
        Gate 4 — tools and utilities
```

Inside phase 4 the entity code is still ordered bottom-up: definitions and SQL, then metadata, then the write-path features (cache, sim, full-text, ECA matching, loader and writer). ECA actions and data-file service elements call the service facade, so they are turned on at the end of phase 6, still before gate 1. That is the one cycle between the two facades. Entity is built first, with those callbacks stored and not yet run. Service is built second. The gate runs only after both sides of the callback exist.

Phases 9 and 10 can proceed in parallel after gate 3. Phase 11 needs the HTTP client from phase 6, and SearchFacade where a tool calls search. Phase 12 needs `xsl-fo` from gate 3. Later database drivers in phase 12 need only the `DbDriver` trait from phase 3.

## Phase 0 — Workspace and XML trees

Depends on nothing.

- Cargo workspace from `plans/Crate-Layout.md`: `moqui-lib`, `moqui-core`, `moqui-host`, binary `moqui`. Direction is `moqui` → `moqui-host` → `moqui-core` → `moqui-lib`.
- `framework/` and `runtime/` in this repository: entity, service, screen, and data XML, XSD, default conf, `component.xml`, webroot screens, and the tools component XML. Macro templates and browser libraries are added by the phase that owns them.
- A parse test for `MoquiDefaultConf.xml` plus a dev conf overlay.

Mantle, SimpleScreens, and the other components stay outside the repo, as they do today under `../moqui/runtime/component`.

## Phase 1 — `moqui-lib`

Depends on phase 0. No `ExecutionContext`.

Modules: `error`, `node` (the `MNode` port; parsing is filled in as real files fail to load; no XML crate), `cache` (`MCache` and `MCacheManager`), `object` (decimals, timestamps, booleans, strings), `string`, `collection`, `context_stack`, `env` (property lookup and `${name}` expansion against the environment), and the resource trait with file and memory implementations.

`signer` is added at the start of phase 4, when field encryption needs it. `http` / `RestClient` is added at the start of phase 6, when the remote runners need it. `etl` and `sgml` wait for phase 11. `context_stack` lives here because actions and screens share it, and the stack must not know the framework.

## Phase 2 — Factory, components, cache, messages

Depends on phase 1.

- `ExecutionContextFactory` and `ExecutionContext`. Facades are fields. A facade that does not exist yet is not called.
- Conf XML, `component.xml` discovery, and component dependency order.
- `ResourceFacade`: `component://` on the lib resource trait. The framework component is a directory. There is no classpath scheme.
- Cache facade on the `MCache` port from phase 1. Entity-definition and service-location caches use it. There is no second cache implementation.
- Logger on `tracing`.
- `MessageFacade` for errors and messages.
- Artifact execution stack with push, pop, and `disableAuthz`. The authorization decision itself is phase 5. Until then the stack allows the call when authorization is disabled, which is the load path and the early entity tests.

## Phase 3 — Transactions and database drivers

Depends on phase 2. `DbDriver` is a trait in core. Connections are host. SQL text is core, written in phase 4. There is no SQLx dependency.

- `TransactionFacade` and the transaction manager, in core, calling `DbDriver`: begin-or-join, commit, rollback, savepoint, `setRollbackOnly`, timeout, `ignore`, `force-new`, and the synchronization list (transaction cache, `runOnCommit` / `runOnRollback`, service ECAs, data feed). One connection enlisted for the transaction. `suspend` returns it before the transaction is parked, and `force-new` is that suspend followed by begin. No JTA types and no XA two-phase commit. The transaction-cache hooks used in phase 4 sit on these same calls.
- Dialect profiles from the `<database>` block. Postgres and MySQL in this phase.
- Host `db`: embedded Postgres via `postgresql_embedded` (start, stop, Unix socket, `NoTls`), external Postgres with rustls, and the MySQL client with rustls. The pool is the core manager. Settings are `plans/EmbeddedPostgres.md`.
- A test round-trips a raw statement through each driver.

The facade is complete against this trait. Adding SQL Server, Oracle, DB2, Derby, or HSQL later is a new driver in phase 12.

## Phase 4 — Entity facade

Depends on phase 3. This phase finishes the entity facade.

Bottom-up inside the phase:

1. Entity XML: dictionary types, fields, relationships, reverse-many, `extend-entity`, master definitions, view-entity structure. `signer` moves into `moqui-lib` here.
2. `EntityValue`, `EntityList`, the condition tree, find, create, update, delete, refresh, sequenced primary and secondary ids, `createOrUpdate`, foreign-key check, and dummy foreign keys.
3. SQL renderer for insert, update, delete, select, and count, including joins, offset, and view-entity select, for the postgres and mysql profiles.
4. `EntityDbMeta`: check and create tables, columns, indexes, foreign keys, and sequences. Startup runs the check before any load. `text-fts` columns and their indexes are part of this metadata. The `matches` operator is part of the condition tree in this same phase.
5. Field encryption on write and read. Audit-log rows when a field has `enable-audit-log`.
6. Entity cache, cleared on write. Transaction cache as an in-memory map. Sim mode: a schema per `TransactionCacheDb` on the overlay cluster from `plans/EmbeddedPostgres.md`. HOLD, FLUSH, and `enter_sim` are a port of the Java code at the time the port begins. A distributed cache-invalidation listener is still later, on the lib topic, and is not a second cache.
7. Dynamic view. Entity sync. Data writer. `EntityFacade.rest` (find, create, store, update, delete, path primary keys, relationship steps, dependent levels, master name), tested by calling the facade, with no HTTP server yet.
8. ECA rules loaded and matched on create, update, and delete. The action bodies run once phase 6 can run them.
9. Data loader for entity elements: streaming XML, CSV, and JSON, including a zip of those. Roots `entity-facade-xml` and `seed-data`. Type filter, component filter, conf `load-data` locations, nested relationships, `onlyCreate`, `useTryInsert`, dummy foreign keys. Service-named elements are parsed and held until phase 6 runs them.
10. Data-document assembly and data-feed rules. Feed dispatch calls the receive service. That service does not index until phase 9. `disableEntityEca`, `disableAuditLog`, `disableFkCreate`, and `disableDataFeed` work.

The factory holds one `DbDriver` per entity group. The default framework group is one database.

Phase 4 tests cover the facade without a service call: create and read, a view-entity find, a dynamic view, an FTS `matches`, a sim write that does not land in the main schema, table check on an empty database, and `EntityFacade.rest` against an in-memory request map.

## Phase 5 — User and artifact authorization

Depends on phase 4. The permission and user tables exist. Entity and service operations are not complete until they enforce these checks, so this phase sits between the two facades and still before gate 1.

- Passwords with `argon2`. `loginUser`, `logoutUser`, anonymous login, `internalLoginUser`. Shiro is not carried forward.
- Server-side session state: user, visit, visitor, locale, time zone, CSRF token, flash messages, saved and error parameters, screen history. The signed cookie that carries the session id is phase 7. Tests call `loginUser` on the facade directly.
- Artifact authorization against artifact groups, permissions, and authz records, including tarpit. Entity finds and writes, and service calls, consult it. `disableAuthz` still short-circuits, which is how load and internal work run.
- Visit and visitor rows. The stats screens are ordinary screens later.

Second factor, password reset, and API-key admin are phase 11. A user with no second factor stays logged in from `loginUser`.

## Phase 6 — Service facade

Depends on phases 4 and 5. This phase finishes the service facade.

- `http` and `RestClient` in `moqui-lib`, blocking `reqwest`. The sim predicate default-denies. Sim is live from phase 4, so the predicate sees it.
- Service XML, `service-include`, and `verb#noun` lookup. Implicit entity-auto: no path, verb one of `create`, `update`, `store`, `delete`, noun an entity name. A data element such as `create-moqui.security.UserAccount` resolves that way after `-` becomes `#`.
- Sync call: parameters, the full validation set (required, matches, ranges, entity-backed validators), and the transaction attributes `use-transaction`, `ignore`, and `force-new`. The `authenticate` attribute is enforced with phase 5.
- Entity-auto runner, including nested maps. The plain-map load path calls the same store function.
- XML actions: MiniJinja 3.0.0 macros named for the elements in `xml-actions-3.xsd`, emitting JavaScript, run by QuickJS. The walker is the one in `plans/ContextObjects.md`. MiniJinja sees the element, not the execution context. Macros emit bare names, and the server proxy in that plan resolves an unset name to null. One QuickJS runtime per `ExecutionContext`, on that thread. Decimals stay host values.
- Script-type services whose body is JavaScript. The Groovy email scripts are converted in phase 10. There is no Java runner.
- `UserSecurityChecks` for the two security ECAs, as Rust functions on the host. ECA actions from phase 4 run through this runner. SECA rules run through it too.
- Email-ECA rules run through the same action runner and call a service. The transport behind send-mail is phase 10. Until then the send service reports that mail is not configured.
- `service-group`. `async="true"` starts another thread and another `ExecutionContext`. The scheduler thread sleeps and runs due jobs. Both are core threads, as `plans/Crate-Layout.md` describes.
- `RestApi`: load `*.rest.xml`, walk the path, run `<entity>` and `<service>` operations. Tested by calling `run` with a path list and a parameter map. The HTTP route is phase 7.
- JSON-RPC dispatcher, tested the same way, with a request body and no socket.
- Remote REST and remote JSON-RPC runners on `RestClient`.

Phase 6 then turns on the two callbacks phase 4 stored: ECA action execution, and data-file service elements (`sync()` on the mapped name).

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

- Host `http`: axum accepts the connection, builds the owned request (method, path, query, headers, body), runs core on a factory worker thread, and writes the owned response. The framework does not see axum. The host does not use Tokio `spawn_blocking` for this.
- Signed session cookie from the phase 5 session. Login replaces the session id. HTTP basic authentication, and the API-key and login-key headers, call `loginUser` before the transition.
- `WebFacade`: parameters, a JSON object or JSON list body, `sendJsonResponse`, `sendJsonError`, `sendTextResponse`, session attributes, and the response headers entity REST already sets (`X-Total-Count`, the page headers, `X-Run-Time-ms`). Multipart waits for the form file widget in phase 8. `ec.web` is added to the script host here.
- Basic screen facade: parse screen XML, directory subscreens and explicit `subscreens-item`, `ScreenUrlInfo`, transition match on name and method, run the transition actions on the phase 6 runner, and honor `default-response` of `none` or a redirect. Widget rendering is phase 8. `webroot.xml` already treats `rest` as a subscreen by file name.
- `rest.xml` transitions this phase wires: `e1` to `EntityFacade.rest`, `s1` to `RestApi.run`, `login`, and `logout`. `sendOtp` and `verifyOtp` parse, and until phase 11 they return a clear error instead of sending a code. The JSON-RPC HTTP route calls the dispatcher from phase 6.
- Screen `require-authentication` stays false on `rest.xml`, so the transition decides. The REST handlers require a logged-in user.
- Schema transitions (`entity.json`, `service.swagger`, and the RAML siblings) close this phase. Gate 2 does not call them.

### Gate 2 — REST API request

- Binary serving HTTP, embedded Postgres, seed loaded.
- `GET /rest/s1/moqui/basic/enumTypes` with HTTP basic auth returns JSON of the seeded enumeration types. That resource is an `<entity>` list operation in `tools/service/moqui.rest.xml`.
- `GET /rest/e1/...` for that same entity, by alias or full name, returns JSON.
- `POST /rest/login` for the seed user returns the logged-in JSON and a session cookie. A following request with that cookie and no `Authorization` header succeeds.
- An anonymous request receives 401. A user without permission receives 403.

The response is JSON from the transition. Widget HTML is not part of this gate.

## Phase 8 — Screen model and render modes

Depends on the screen URL and transition runner from phase 7, and on the action runner from phase 6.

- The rest of the screen model: sections, `section-iterate`, containers, trees, `form-single`, `form-list`, conditional and error responses, `include-screen`, `screen-extend`, and database forms.
- `always-actions`, `pre-actions`, section actions, and `row-actions` through the phase 6 runner.
- MiniJinja screen and form macros. The walker passes attributes, child text, and already-rendered children. Server HTML and client JavaScript are macro targets. A native widget-tree renderer is not the path.
- `ec.l10n` format, formatCurrency, and localize, using the session locale and time zone and the seeded localization messages. `sri` gains the methods the macros call. The method list remains the open host-API plan. This phase implements the calls the current macros make, and records any it adds.
- Modes, in this order. Each one is a macro library on the same screen model.

| Order | Mode | Output |
|---|---|---|
| 1 | `text` | Plain text, server merge |
| 2 | `csv` | CSV, server merge |
| 3 | `xml` | XML, server merge |
| 4 | `xsl-fo` | XSL-FO document, server merge. PDF is phase 12 |
| 5 | `html` | Terse server HTML, shared stylesheet, `moqui-html.js` |
| 6 | `qvt` | Server-merged Vue component object |
| 7 | `qjs`, `qvue` | Companions, kept |
| 8 | `qvt` with `server-static` | Data-free template plus `/actions` JSON |

`vuet`, and the `js` and `vue` companions that feed it, are not built. `/vapps` is absent. `/apps` stays HTML. `/qapps` stays the default.

HTML follows the widget table in the architecture proposal: semantic elements, one shared script, `fetch`, native date inputs, Tom Select and SortableJS loaded only on a page that has them, Font Awesome 4. jQuery, Bootstrap, and Moment are not in this client.

Server-merged `qvt` follows the library section of the architecture proposal: Vue 3.5 and Quasar 2.35 by script tag, `m-*` wrappers, and no jQuery, Moment, or `http-vue-loader`. `QEditor` replaces CKEditor 4. A textarea plus Marked replaces SimpleMDE. Chart.js 4, a current Mermaid build that still loads from a script tag, the Assist libraries, and QZ Tray 2.3 when printing is enabled.

Server-static `qvt` is last, and follows `plans/QvtServerStatic.md`. The attribute stays opt-in. The response uses the `screen-server-static` cache header. `GET {screen}/actions` and `GET {screen}/actions/{formName}` are private and uncached. The browser `ec` is the restricted one in that plan. `dynamic-container` stays server HTML. The compatibility report runs. It does not flip the default.

### Gate 3 — screen rendering

- One existing tools or framework screen that contains a form-single, a form-list, and a section is rendered through the server in `text`, `csv`, `xml`, `xsl-fo`, `html`, and `qvt`.
- The screen's actions are entity finds and inline services. A screen whose actions call search, mail, or an LLM is the wrong screen for this gate.
- The HTML response includes the shared stylesheet and `moqui-html.js`. A form post runs the transition and returns its response.
- The `qvt` response is a component object the Vue 3 compiler can compile. With `server-static` set for `qvt`, the template contains no row data, and `/actions` returns the JSON package from `plans/QvtServerStatic.md`.
- `qjs` and `qvue` return the companion bodies for that screen.
- A `/vapps` path has no screen.
- The `xsl-fo` body is well-formed XML.

## Phase 9 — SearchFacade

Depends on gate 1. Entity documents and the feed rules already assemble, and the service facade can call the receive service. The HTTP client already exists. The API is `plans/SearchFacade.md`.

- Query tree, aggregation tree, and Postgres SQL in core. Documents live in the `search` schema on the entity cluster.
- A separate embedded Postgres cluster, or an external Postgres, when config places search somewhere else, including when the entity datasource is MySQL. MySQL is not a search backend. Embedded search starts on Windows too.
- The feed joins the entity transaction when both stores are the same database, and runs after commit when they are not. This replaces the phase 4 record that search is not up.
- The query-string parser for the subset existing screens emit.
- One active backend. The OpenSearch backend builds HTTP bodies and sends them through `HttpClient`. The host `search` module starts the process. `/elastic` and `/kibana` proxies, and the log indexes, exist only when that backend is selected.
- `CountDistinct` is exact on Postgres and may be approximate on OpenSearch.

`get#InventoryProjectedInfo` is a mantle rewrite onto `aggregate`. It is named here so it is not ported as an `ElasticFacade` client. The rewrite happens with mantle, outside this gate.

## Phase 10 — Mail, Liquid, and notifications

Depends on the service facade. Email-ECA rules already fire and currently stop at the unconfigured send service. This phase provides that service.

- Liquid for email, notifications, and other user-authored text. Standard library plus the small Moqui filter set in `plans/ContextObjects.md`. The snapshot is the unwrap projection. Filters format and localize and do not receive an `ExecutionContext`.
- Outbound mail with `lettre` in core. The Groovy send scripts become Rust service logic. The send service from phase 6 then sends.
- Inbound SMTP in the host, behind a feature.
- Notification messages, the in-process topic, and the host WebSocket. The socket uses the phase 5 session and the phase 7 cookie. A message runs on a worker with its own `ExecutionContext`. The connection does not keep one for its lifetime.
- Markdown for screen and wiki text that asks for it.

## Phase 11 — LLM, A2A, and the other tools

Depends on the HTTP client, the user facade, and the screen facade for screen-search hints. Search-backed tools wait on phase 9. The LLM client sets the sim-allow exception on `RestClient`.

- LLM client, conversation, protocols, and the tool loop, as the Java framework stands when the port begins. Host routes for the LLM endpoints. A stream stays in the host.
- A2A routes in the host, following `../moqui/framework/plans/A2A-Server.md`. This document does not restate that plan. Streams do not hold an `ExecutionContext`.
- ElFinder on the host, when the file API stays.
- Artifact hit bins and the flush that writes them.
- System message receive, record, and consume. The `/rest/sm` transition on `rest.xml` starts calling `receive#IncomingSystemMessage` here.
- ETL: `SimpleEtl` and `FlatXmlExtractor` in lib. `sgml` in lib. JDBC extract beside `DbDriver` in the host.
- EDI handler in core.
- `db://` and content resources. JCR is dropped. Blob storage is the filesystem, with S3 as a resource implementation when a deployment asks for it.
- Cache admin and the other tools services that wrap facades already built.
- Print-job records. Turning `xsl-fo` into PDF is phase 12.
- User remainder: preferences, password reset, TOTP, email second factor, and the admin side of API keys and login keys. The `sendOtp` and `verifyOtp` transitions start working here.
- The Groovy shell is not replaced with a script console.
- A distributed cache-invalidation listener on the lib topic, when a deployment asks for one. It is not a Redis client and not a second cache facade.

## Phase 12 — PDF, later drivers, and the tool components

Depends on `xsl-fo` from gate 3, and on the `DbDriver` trait from phase 3.

- An implementation of `PdfRenderer`. The host `pdf` module is the placeholder until then.
- `moqui-fop`, `moqui-poi`, `moqui-image`, `moqui-cups`, and `moqui-sso`, each weighed against binary size the way `moqui.war` size kept them out of the Java framework. A component that stays outside is listed as outside, with that reason.
- Further sync drivers, each a dialect plus a `DbDriver`: SQL Server, Oracle, DB2, DB2 for i, Derby, HSQLDB. A missing driver is a configuration error that names the database. None of them block the other gate 4 work.

### Gate 4 — tools and utilities

The gate is a checklist. Each item is its own test. Entity and service behavior is covered by the earlier phase tests. This checklist covers the tools that call them.

- SearchFacade indexes one data document on Postgres and a query returns it. The feed from phase 4 is the writer. An OpenSearch run is a second test. The default remains Postgres.
- An email template renders through Liquid, and the send path accepts it. A test sink is enough. An email ECA that calls send uses that path.
- A notification sent from a service arrives on the WebSocket for the same session the HTTP request used.
- LLM and A2A cover the same cases as the current Java tests for those facades.
- Every entry in the tool-component list is either implemented or recorded as staying outside.
- A later database driver, when one is included in the build under test, runs the same metadata and insert smoke test as MySQL. A build that omits them still passes the gate.

Scheduled jobs are a phase 6 test. Sim mode is a phase 4 test. They are not repeated on this checklist.

## Tests

Unit tests live in the crate that owns the code. Lib tests do not start a factory. Each facade phase has its own tests before the next phase starts. Phase 4 tests do not call a service. Phase 6 tests do not open a socket. Phase 7 tests do not render a widget.

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
