# MR-Moqui Architecture Proposal

**Modern Rust Moqui** (pronounced “Mister Moqui”)

This document captures the initial architecture direction for a Rust rewrite of Moqui Framework. It is the starting proposal for the new repository.

**STATUS NOTE**: Templating, screen render modes, the crate cut, search, qvt server-static, embedded Postgres, context objects, and the build order are specified. Templating is below: MiniJinja 3 expands XML Actions and screen/form XML into JavaScript, and Liquid renders user-authored text. Screen render modes are in Screen Render Modes. The crate cut is `plans/Crate-Layout.md`. Search is `plans/SearchFacade.md`. Server-static qvt is `plans/QvtServerStatic.md`. Embedded-database concurrency, the transaction manager's checkout, and the sim overlay's database lifecycle are `plans/EmbeddedPostgres.md`. What each environment can see, which types are shared, and the server host method list are `plans/ContextObjects.md`. The build order is `plans/Implementation-Phases.md`. The proposal will still move as the first components are built.

## Repository

- New repository: `moqui/mr-moqui`
- Combines what currently lives in `moqui-framework` and `moqui-runtime`. The split has had minimal payoff and high cost; the new repo is the single home for the framework and the default runtime.
- Higher-level components (`mantle-udm`, `mantle-usl`, applications) stay separate and continue to be defined primarily in XML.

## Goals

- Keep XML as the spec language for configuration, entities, services, screens, SECAs, and data files, with only the adaptations required by a Rust runtime.
- Move the heavy lifting (entity facade, service facade, screen rendering, tooling, and performance-sensitive service implementations) into Rust.
- Use one language for inline expressions on both server and client so that XML Screens can become client-renderable.
- Default to an embedded Postgres (`postgresql_embedded`) when no external database is configured, and use an external Postgres when one is. The same engine backs the sim-mode overlay.
- Make OpenSearch optional from day one by planning a search interface that can be satisfied by Postgres full-text search and JSON operators.
- Preserve the ExecutionContext / facade shape that makes Moqui’s artifact model work.

## Execution Model

The runtime is organized around an `ExecutionContext` that owns the facades:

- EntityFacade
- ServiceFacade
- ScreenFacade
- ResourceFacade
- TransactionFacade
- CacheFacade
- UserFacade
- MessageFacade
- LoggerFacade
- SearchFacade (see below)

XML Actions (`entity-find`, `service-call`, `if`, `iterate`, `set`, and the rest) and screen/form XML are expanded by MiniJinja macros into JavaScript and run by the embedded engine. An in-memory interpreter remains possible as a later optimization, but it is not the primary path. Expressions inside attributes, and `<script>` bodies, are JavaScript. Details are in Templating.

Components remain directories with a `component.xml` descriptor, discovered and loaded at startup. Resource locations keep a scheme-based model (`component://`, and equivalents).

The framework is synchronous and multi-threaded, as Moqui is today. An `ExecutionContext` belongs to one thread. An HTTP request, an async service call, a scheduled job, and the other background workers each run on a thread with their own context. A call that waits (a database query, outbound HTTP, mail) blocks that thread and returns a value. Entity, screen, and HTTP APIs return values. `RestClient` stays blocking. The one wait handle is service-facade `callFuture()`: a factory thread, a new `ExecutionContext`, and a oneshot the caller waits on. Framework code does not await on Tokio. `RestClient`'s blocking `reqwest` and each `postgres::Client` start a private runtime, as `plans/Crate-Layout.md` describes.

Worker threads that belong to the framework are ordinary threads started by the factory. The HTTP server lives in the host crate. It accepts connections on Tokio and hands each request to a thread from that pool, which calls the framework and waits for the response. The host does not use Tokio's `spawn_blocking` pool for this. That pool's cap is independent of the datasource pool. Transaction attributes run on the calling thread: begin, commit, rollback, a savepoint, or, for `force-new`, suspend then begin. The attributes are `ignore`, `use-or-begin` (the default), `force-new`, `cache`, `force-cache`, and `transaction-timeout`. How `suspend` parks the client is in Transactions. Details of the crate cut are in `plans/Crate-Layout.md`.

`service-call` with `async="true"` starts another thread and another `ExecutionContext`, as it does now. `callFuture()` is that same start, and the caller waits on the oneshot. The joined sites are `org.moqui.impl.InstanceServices.check#AppInstance` (three checks) and `mantle.party.DuplicateServices.check#AllPartyDuplicates` (batches of ten). `init#AppInstance` starts work it does not join, and that stays `async().call()`. Commit and rollback synchronizations that fire a service also use `async().call()`. `async="distribute"` (run on any node in a cluster) is dropped. It is rarely used, and there is no cluster transport for it. The service-facade `distributed-factory` goes with it. Distributed cache invalidation stays a later listener on the in-process topic. It is not a cache trait and not a Redis dependency.

QuickJS is one runtime per `ExecutionContext`, used only on that context's thread. A function the script calls returns a value before the script continues.

A WebSocket, an LLM stream, and an A2A stream stay in the host. The host owns the connection. A message, or a call that is not a stream, runs on a worker thread with its own `ExecutionContext`, outside the transaction, and returns a value. The connection does not keep a context for its whole life.

## Scripting Language: JavaScript via rquickjs

JavaScript is the scripting language. The engine is QuickJS-NG embedded through the `rquickjs` crate.

### Why JavaScript

- The same language can run on the server and in the browser. Screen expressions become client-evaluable without a second expression language.
- Modern ECMAScript already covers the features that made Groovy preferable to Java for Moqui expressions: dot access on objects, optional chaining, nullish coalescing, object literals, array indexing, template literals, and array methods.
- Service libraries such as mantle-usl are primarily higher-level business logic. The hot paths and infrastructure stay in Rust. An interpreter without a JIT is acceptable for this workload.

### Why rquickjs / QuickJS-NG

- Small, embeddable, fast startup, and a clean Rust integration. Host functions are synchronous: the script calls, the call blocks, and it returns a value. Module loaders stay available.
- Supports modern ECMAScript (modules, proxies, BigInt, and a large fraction of current language features). Async functions exist in the language and are unused here. A host call returns before the script continues, and the framework has no event loop that would drain a promise. It is not required to track the absolute latest spec the week it ships. QuickJS-NG is adequate. TypeScript remains an optional authoring layer (transpiled by SWC or oxc) if longer scripts later want types. It is not required for expressions.
- Heavier options (V8 via `rusty_v8`) were considered and set aside. Binary size, build complexity, and the fact that most script work is business logic rather than tight loops made the lighter engine the better fit. Pure-Rust engines (Boa and others) are not yet the right primary runtime for a large body of existing service code.

### What changes in expressions

Most expressions used in service and screen XML map mechanically:

| Groovy (current) | JavaScript |
|---|---|
| `map.field` / `entityValue.field` | same (objects or a Proxy over EntityValue) |
| `list[0]` | same |
| `obj?.field` | same |
| `a ?: b` | `a ?? b` for a null check. Use logical or when an empty string or zero should fall through. See the note. |
| `[orderId:orderId, statusId:'X']` | `{orderId, statusId:'X'}` |
| `x in ['A','B']` | `['A','B'].includes(x)` |
| `list*.field` | `list.map(x => x.field)` |
| `"Hello ${name}"` | `` `Hello ${name}` `` |
| ternary `? :` | same |
| `fields[3..size-1]` | `fields.slice(3, size)` when `size` is the length |
| `orderItem.'Child#mantle.order.OrderItem'` | `orderItem['Child#mantle.order.OrderItem']` |
| unset bare name, as in `if (someUnsetParam)` | same text. On the server the name is null. See `plans/ContextObjects.md`. |

Semantic differences to watch during migration. The server proxy that makes an unset name null is the codegen rule in `plans/ContextObjects.md`. It is decided before macros are written, and macros emit bare names.

- An unset bare name is null in Groovy. `ContextBinding.hasVariable` always returns true (`ContextBinding.java:41-45`), so the get falls through to the context stack instead of raising `MissingPropertyException`. `if (someUnsetParam)` is false. In JavaScript the same line throws `ReferenceError` at runtime unless the server runtime resolves the name. That is the common migration failure: it fails when the script runs, not when it is compiled. The review counted 687 `<script>` blocks and 4,063 `<entity-find>` elements in mantle-usl alone. mantle-usl is not in this checkout, so those counts are the review's and will be re-measured before the migration pass. The framework service XML has the same shape.
- Elvis (`?:`) is a different operator from nullish coalescing. `''` and `0` are falsy in both Groovy and JavaScript, so `if ('')` and `if (0)` already match. `'' ?: 'x'` is `'x'` in Groovy, `'' ?? 'x'` is `''` in JavaScript, and `'' || 'x'` is `'x'`. Use `||` when the left side is a string or number that may be empty or zero. Use `??` only where the old expression was a null check. Do not mechanically rewrite `?:` to `??`.
- An empty list or empty map is falsy in Groovy and truthy in JavaScript. Neither `??` nor `||` reproduces that (`[] || 'x'` is `[]`). Conditions that mean “has rows” or “has entries” test length.
- Groovy `==` and JavaScript `==` both treat `'1' == 1` as true. The migration keeps `==`. The breaks are list and map equality (content in Groovy, reference in JavaScript) and a host decimal compared to a number (no coercion, and `+` is not overloaded either).
- Groovy `fields[3..size-1]` is an inclusive range. `fields.slice(3, size)` is the JavaScript when `size` is the length, because `slice` excludes the end. Left as Groovy it is a syntax error, so it fails when the script is compiled. The review counted 23 in mantle-usl. The framework has one in `WikiServices.xml` (`extraPathNameList[1..size-1]`). `ScreenRenderImpl.groovy` has another and is a Rust port, not a script.

The QuickJS context mimics the Groovy and FreeMarker context, so expression and script migration stays small. A script starts with the same injected objects the Java runtime puts there: `ec`, `context`, a service `result` where the current actions provide one, and the other objects that entry point injects by default. A screen template sees what the FreeMarker macros see today. The static helpers those scripts and generated actions call are there too, including `ObjectUtilities`, `CollectionUtilities`, and `StringUtilities`.

Host values (`EntityValue`, `EntityList`, decimals, timestamps) expose the methods those scripts call. JDK types such as `BigDecimal` and `TreeSet` are small host helpers with the same call shape. Decimals stay host values and do not become JavaScript numbers. Groovy `+` and comparison on a decimal become a method on the host value or an `ObjectUtilities` helper. Operators are not overloaded. The pre-scan in `plans/Implementation-Phases.md` counts those expressions. The methods the generated script calls are `plans/ContextObjects.md`. A tighter sandbox on which framework calls a script may make is a later effort.

### Migration approach

A general Groovy-to-JavaScript transpiler is not planned, and a long-lived compatibility shim for Groovy syntax is not desired. The preferred path is AI-assisted update of expressions and scripts, after the pre-scan in `plans/Implementation-Phases.md`. A temporary, narrowly scoped rewrite of the common attribute-expression idioms above may be used during the transition if it proves helpful, but it should not become something that ships and is relied on in production. The cost of manual migration is no longer a reason to keep risky compatibility layers.

`mantle-udm` (entity definitions and seed data) should load with little or no change once the entity model and data loader exist. `mantle-usl` requires migration of Groovy expressions and `<script>` bodies to JavaScript. XML Action elements stay as XML. The macro library that expands them changes from FreeMarker (emitting Groovy) to MiniJinja (emitting JavaScript).

## Templating

FreeMarker currently serves two different purposes. MR-Moqui splits them across two engines. What each engine is given, and what QuickJS and the browser see, is `plans/ContextObjects.md`.

### Code generation with MiniJinja

XML Actions and screen/form XML are expanded by a library of macros (one per element name) into executable code. Today the target is Groovy. In MR-Moqui the target is JavaScript, run on the embedded QuickJS-NG engine described above.

MiniJinja is the engine for this library. Pin `minijinja` 3.0.0 (released 2026-10-08). It is a Jinja2-compatible template engine written in Rust, with macros, `{% import %}` / `{% from %}`, inheritance, and blocks. A file or a directory of macros, named to match the XML element names (`entity-find`, `service-call`, `form-single`, `field`, and so on), replaces the current FTL macro set. The walker renders one cached template per element name (`{% from 'defs' import entity_find %}{{ entity_find(attrs, text) }}`). MiniJinja has no call of a macro through a variable name, and there is no per-mode template that branches on the element name. The generator passes the element attributes and already-rendered children, and emits JavaScript.

The `serde` feature stays off. Macros receive attribute strings, child text, and already-rendered children. `set_auto_reload(false)` is set on the macro environment: 3.0 stats the file on every lookup when auto-reload is left on, and the cached artifact is the generated JavaScript. `trim_blocks` is on, set through `SyntaxConfig`, so macro output is not padded with whitespace that becomes JavaScript. There is no `minijinja-contrib`. Dates are formatted by `ec.l10n`. Fuel and autoescape remain. The `Object` trait still takes `self: &Arc<Self>` (`Send + Sync`). A node is not handed to MiniJinja. 3.0 changes floor division, `round`, and undefined-handling inside MiniJinja templates. The macros are new, so those changes are not a migration, and they do not change the JavaScript the macros emit.

This keeps the properties that matter:

- The macros are ordinary text. They can be edited without rebuilding the framework.
- The thing that runs is generated JavaScript, so behavior is visible and ordinary script tooling applies.
- Location information from the original XML can be written into the generated script as comments or a source map, so stack traces and debugging point back at the service or screen file.
- Composition works the same way it does with FreeMarker macros.

An in-memory interpreter for XML Actions remains possible as a later optimization or for elements that are awkward to generate, but it is not the primary path. Generated JavaScript is preferred for lower execution overhead, well-known behavior, and easier debugging.

The same MiniJinja library can target different outputs. Server-side HTML rendering and client-renderable screen JavaScript can be two macro sets, or one set with a target parameter.

### Runtime text templates with Liquid

Emails, notifications, custom HTML fragments, and other user- or admin-authored text are rendered with Liquid, via the `liquid` crate (liquid-rust).

Liquid was designed so that untrusted or semi-trusted templates can be rendered safely. It is intentionally limited: output tags, a small set of logic tags, filters, and inclusion of other templates via `{% render %}`. There is no general scripting. liquid-rust aims at Shopify compatibility and lets the host register only the filters and tags it wants.

Reusable pieces are snippets rather than FreeMarker-style macros. That is sufficient for this surface and safer. Custom filters provide Moqui-specific formatting, localization, and any limited context access that is explicitly allowed. That access is the snapshot in `plans/ContextObjects.md`.

Liquid has a broad ecosystem (highlighters, editors, documentation, examples) because it is widely used beyond Shopify. That tool support is the reason to accept a second engine rather than using MiniJinja for everything.

### Sandboxing

MiniJinja is used for framework-authored code generation, where the authors are the component or framework developers. It is not the sandbox boundary.

Liquid is the sandbox boundary for templates that may be edited by admins or come from less-trusted sources. Its restriction is structural. MiniJinja’s fuel system and restricted environments are available if a MiniJinja template ever needs limits, but user-facing text defaults to Liquid.

### Summary

| Job | Engine | Notes |
|---|---|---|
| XML Actions → JavaScript | MiniJinja macros | Replaces FTL → Groovy. Emits debuggable JS. |
| Screen/form XML → JavaScript or HTML | MiniJinja macros | Same library, possibly different target. |
| Emails, notifications, custom text/HTML | Liquid (`liquid` crate) | Sandboxed by design. Custom filters for Moqui helpers. |
| Internal framework templates | MiniJinja | No second engine required when authors are the framework. |

Both crates are pure Rust and small. Liquid does not need to be a complete Shopify subset on day one. Start with the standard library plus a small set of Moqui filters, and add tags only when a real template requires them.

## Database

- If an external Postgres or other database is configured, use it (just as with Moqui now).
- If not, use embedded Postgres (`postgresql_embedded` 0.21, theseus-rs). This is the default configuration, the same role H2 has today, on Linux, macOS, and Windows.
- Use that same engine for the sim-mode overlay database (`TransactionCacheDb`), whether or not the entity database is external.

### Why embedded Postgres

`postgresql_embedded` starts a real Postgres postmaster as a child process and needs no separate server install. Ordinary `postgres::Client` connections use its Unix socket. It is the “just start” experience H2 provides, and the dialect is Postgres, matching production and backing search.

The theseus Postgres 18.6.0 archive is 11.6 MB compressed on x86_64 Linux and extracts to 34.0 MB. One install serves every cluster. The Windows archive is 51.6 MB and is only in the Windows build. Sizes, `shared_buffers`, and the socket settings are `plans/EmbeddedPostgres.md`. Runtime RSS was not measured. `shared_buffers` starts at 16 MB for the entity postmaster and 8 MB for sim, and the first driver test records RSS.

`pglite-rs` was the earlier candidate. Every release shipped in a 16-day window in June 2026, its multi-process API is Unix-only, and its in-process engine can be opened once per process. SQLite is a second dialect, and H2 was the second dialect this choice replaces. DuckDB is an analytical store. A WASM build of PGlite is one connection, and this process is not a WASM target. A system `postgres` already on `PATH` does not start on its own.

Embedded Postgres replaces H2 under `TransactionCacheDb`. The per-transaction `TransactionCache` stays an in-memory map, as in the Java framework. HOLD and FLUSH behavior are a port of the Java code at the time the port begins. The `llm-client` branch is expected to keep changing until then, so this proposal does not freeze those rules. How many postmasters start, and where an overlay lives, is `plans/EmbeddedPostgres.md`.

External Postgres remains the production path when configured.

### Platform notes

Deployments are overwhelmingly Linux. macOS matters for development, and builds should work on Linux and macOS. Windows is a lower-priority test target. Embedded Postgres runs there, so entity, search, and the sim overlay start on Windows too. There is no WASM target. A `wasm32` build would have required the framework to give up threads, and Moqui uses threads for requests, async services, scheduled jobs, and other background work. That model carries forward.

The default embedded database is one cluster, so each thread has its own connection. Embedded search is the `search` schema on that cluster. The sim overlay is a second cluster, one schema per `TransactionCacheDb`. Details are in `plans/EmbeddedPostgres.md`. External Postgres remains the production database when one is configured, and default search tables stay on that same database. External Postgres and MySQL are `DbDriver` implementations in the host. The pool and the transaction manager live in core and call that trait. Each Postgres client drives its own runtime, as `plans/Crate-Layout.md` and `plans/EmbeddedPostgres.md` describe. Pool sizes and timeouts start at the Java datasource values.

### Transactions

There are no JTA types and no XA two-phase commit. Transactions are local to the single transactional datasource. The portions of Bitronix that Moqui actually uses are ported to Rust and maintained here, written against `DbDriver`.

That port is the local one-phase path and the facade behavior. When the conf has no `xa-ds-class`, `TransactionInternalBitronix` wraps a plain JDBC connection in `LrcXADataSource` and commits it in one phase. Postgres and MySQL take that path. The pool settings come from the datasource conf: min, max, idle time, acquisition timeout, and a test on checkout. One connection is enlisted and shared for the life of the transaction. `TransactionFacadeImpl` is the API on top: begin-or-join, commit, rollback, `setRollbackOnly` with a cause, timeout, and synchronizations. `beforeCompletion` flushes the transaction cache. When the search store is that same database, the data feed then writes on the enlisted client, still before commit. `afterCompletion` runs `runOnCommit` and `runOnRollback`, service ECAs, and the data feed when the search store is a different database or OpenSearch. The feed rule is `plans/SearchFacade.md`. In Java, `suspend` closes the Moqui connection handles and then calls `tm.suspend()` (`TransactionFacadeImpl.groovy:544-548`). Bitronix LRC keeps the physical session with the suspended transaction and returns that session on resume. A Postgres or MySQL local transaction is that session. `PREPARE TRANSACTION` ends it, so the port does not use it. `suspend` parks the enlisted client on the transaction. The client leaves the thread and stays off the free list. `resume` checks that same client back out. Uncommitted work stays on that session.

`begin` joins an active transaction and returns false. Commit and rollback run only when this frame began the transaction. A failed commit rolls back to clean up. `setRollbackOnly` records the cause, and a later commit becomes a rollback. `force-new` parks the current client and begins on a new checkout. The calling thread holds one connection. The parked client still counts against the pool max. There is no idle spare. The acquisition timeout from `pool-time-wait` fails the inner checkout when the pool cannot supply it, including a nested `force-new`. Timeout comes from the service attribute. The service attributes preserved are `ignore`, `use-or-begin` (the default), `force-new`, `cache`, `force-cache`, and `transaction-timeout`. `cache` is `use-or-begin` with the transaction cache on. `force-cache` is `force-new` with the transaction cache on. A call-site `requireNewTransaction` is `force-new`. A call-site `useTransactionCache` is `cache`. `transaction-isolation` is schema-only in Java and is not ported. No service XML in this tree sets `transaction="cache"` or `transaction="force-cache"`. The attributes are still parsed.

The manager and the pool live in core, next to the transaction facade. The manager calls `DbDriver` and does not import `postgres` or `mysql`. Embedded Postgres, external Postgres, and MySQL are the driver implementations in the host. `plans/Crate-Layout.md` places them.

The Java library does not carry forward, and the port does not link `moqui/bitronix`. That fork (4.0.1, January 2026) is the behavior reference. Bitronix is about 17,800 lines, most of it the XA journal, crash recovery, and JMS. Those stay out until a datasource is configured with a real XA resource. No current Rust crate is this in-process manager: the async pools are the same layer as a generic connection pool, and the distributed-transaction crates are network coordinators. Cross-resource work uses an outbox or a SECA rather than two-phase commit. A prepared-statement cache waits on a profile. `postgres::Client` already prepares statements.

## Search

OpenSearch-free deployments are a day-one goal. Postgres full-text search and JSON operators cover the common cases that ElasticFacade is used for today (full-text and parameterized document-style queries).

The search interface should be designed so that it is not locked to the current ElasticFacade shape. If a cleaner interface falls out of supporting both a Postgres implementation and an optional OpenSearch implementation, that interface is part of the MR-Moqui migration, not a later retrofit. OpenSearch remains available as an optional backend for deployments that want it. It is no longer required.

When the entity datasource is Postgres, embedded or external, default search tables are the `search` schema on that database. A search `<postgres>` URL is how search points at a different Postgres. When the entity datasource is not Postgres, search is its own embedded cluster. Naming an external database for a store does not also start an embedded postmaster for that store. See `plans/SearchFacade.md` and `plans/EmbeddedPostgres.md`.

The interface, the feature set taken from current callers, and the two backends are specified in `plans/SearchFacade.md`.

## Other Infrastructure

| Area | Direction |
|---|---|
| HTTP server | `axum` + `tower` in the host. The framework sees an owned request and blocks until the response is ready. WebSocket via axum `extract::ws`. That module needs the `ws` feature, which is not in axum's default set. The feature pulls `tokio-tungstenite`, so the host does not depend on it directly. `tower-http` serves compression and an `ETag` on static assets and on the server-static qvt template. `Cache-Control` for that template stays as `plans/QvtServerStatic.md` specifies. HTTP client via blocking `reqwest`. No servlet container, no JNDI. |
| Screens and XML Actions | MiniJinja macros expand XML to JavaScript. Screen and form XML can also emit server HTML. No FreeMarker, and no native widget-tree renderer as the primary path. See Templating and Screen Render Modes. |
| User-authored templates | Liquid (`liquid` crate) for email, notifications, and custom text/HTML. Sandboxed by design. |
| Security | Do not port Shiro. Reimplement UserFacade on `argon2` 0.6. The `password-hash` feature is optional and part of the crate's default set; it pulls `password-hash` 0.6. A dependency that turns default features off still enables `password-hash`. The framework owns the session. Artifact authorization stays Moqui’s own model and tables. See Sessions and login. |
| Cache | A port of `MCache` and `MCacheManager`. No second cache implementation. Distributed invalidation, later, is a listener on the in-process topic in `moqui-lib`. Hazelcast does not carry forward. |
| Logging | `tracing`. |
| JSON / XML | `serde_json`. XML is the `MNode` port in `moqui-lib`. Conf is XML. RAML and Swagger YAML are written by `RestSchemaUtil` today. The crate that writes them is chosen when those transitions are ported. `serde_yaml` is not a dependency. |
| CSV, mail, markdown, cron, TOTP | Ordinary crates (`csv`, `lettre`, `pulldown-cmark` / `comrak`, `cron`, `totp-rs`). |
| Content repository (JCR) | Dropped. `content://` is not remapped onto blob storage. Resolving a `content://` location is an error that names the location. `db://` stays. Filesystem and S3 are their own resource implementations when a deployment asks for one. |

JDBC drivers, H2, the Bitronix library, Shiro, FreeMarker, Groovy, and the servlet stack do not carry forward. The transaction behavior Moqui uses from Bitronix is ported onto `DbDriver`, as Transactions describes. FreeMarker’s two jobs move to MiniJinja (code generation) and Liquid (user-authored text). Where Moqui already has an implementation because the general library was the wrong shape, that implementation is ported. `MNode`, `MCache`, and the transaction manager are the cases. An external crate earns its place when it is the engine, or when another crate this runtime already needs pulls it in.

## Sessions and login

The servlet session and Shiro do not carry forward. The framework keeps a server-side session of its own. A signed cookie carries the session id. The session holds what the Java session holds: the authenticated user, visit and visitor ids, locale and time zone, the CSRF session token (`moqui.session.token`), flash messages and errors, saved and error parameters, and screen history. The notification WebSocket uses the same session.

Login, logout, and the check that the user has logged out elsewhere live on this session. A successful login replaces the session id. A second factor in progress (email or TOTP) stays on the session until authentication completes. An API key or a login key on the request can authenticate without a password in the session. The long-lived visitor cookie stays separate from the session cookie. Artifact authorization is unchanged and reads the user from this session.

## Tool components and print

`xsl-fo` stays a server render mode and emits the FO document, as the Java mode does (`mime-type` `text/xml`). Turning that document into PDF, and the tool components that live outside the framework today, are phase 12. That set includes `moqui-fop`, `moqui-poi`, `moqui-image`, `moqui-cups`, and `moqui-sso`. Some of them may move into the framework. The size of the base `moqui.war` was a reason they stayed outside Moqui Java, and the size of the MR-Moqui binary is the same kind of limit. The default binary still includes the embedded Postgres distribution, including for a deployment whose entity database is MySQL, because search and sim use it. A custom build may omit `postgresql_embedded` only when search is an external Postgres or OpenSearch and sim is unused. Until `PdfRenderer` is implemented, the host `pdf` module is an empty placeholder, and `/fop/*`, an email attachment rendered with `screenRenderMode=xsl-fo`, a scheduled screen PDF attachment, and print rendering do not produce a PDF. They report that PDF is not available. Java's `xslFoTransform` runs an identity XSLT transform into the FOP handler. Every in-tree caller passes a null stylesheet. That transform and its engine land with the PDF effort.

## Screen Render Modes

A screen is requested with a render-mode extension (`.html`, `.qvt`, `.text`, `.csv`, `.xml`, `.xsl-fo`). MiniJinja replaces the FreeMarker macro set for every mode. The modes themselves change as follows. `/qapps` stays the default UI. `/apps` stays the server-rendered HTML UI. `/vapps` goes away.

### Modes that stay

`text`, `csv`, `xml`, and `xsl-fo` stay server-rendered output formats with the same output shape. `xsl-fo` is the FO document. PDF conversion is phase 12, as Tool components and print describes. They load no browser libraries. Scheduled jobs, exports, and print already depend on that output. Until PDF exists, a job or print that asks for a PDF reports that PDF is not available.

### vuet

`vuet` (`/vapps`), and the `.js` and `.vue` companion modes that exist to feed it, are dropped. That UI loads the HTML-mode jQuery stack and then Vue 2 on top of it. qvt is the client-rendered UI. Its companions stay: `qvt`, `qjs`, and `qvue` (`MoquiDefaultConf.xml`). `WebrootVue` maps `js` to `qjs`, `vue` to `qvue`, and `vuet` to `qvt`. Dropping `vuet` drops that last mapping. `qjs` and `qvue` stay.

### html

`html` stays because the document can be small and the feature set is the basic one: forms, form-lists, drop-downs, dates, a tree that navigates on click, background submit into a dynamic container. The current page works against that. Every screen loads `CombinedBase.min.js` (about 205 KB gzip). `webroot/build.gradle` builds that file from `baseJs`: rsvp, jQuery, Bootstrap 3, typeahead, inputmask, jQuery Validate and additional-methods, Moment with every locale, the Bootstrap datetimepicker, Select2, Selectivity, bootstrap-notify, jeditable, jquery-sortable-lists, and MoquiLib. jquery.form and jsTree are not in that file. The same Gradle file's `htmlJs` list, and `apps.xml`, load them as separate scripts. The new html client replaces those two with `fetch` and the tree renderer in `moqui-html.js`. The macros then append an inline `<script>` for most widgets, and Bootstrap class names (`form-control`, `col-sm-*`, `btn btn-primary`) on every field.

The html macros emit semantic elements and a few data attributes. One shared script binds those attributes, so a form-list of dates does not emit a script per field. The always-loaded client is a Moqui-owned stylesheet and `moqui-html.js`, on the order of 15 KB and 8 KB gzip, plus Font Awesome 4.

| Widget need | html approach |
|---|---|
| Layout, forms, tables, nav, dialogs | Element selectors in the Moqui stylesheet. Font Awesome 4 stays. Screen XML icons are `fa fa-*`, and Font Awesome 5+ changes that class. |
| Background submit, dynamic container | `fetch`. The server already returns HTML fragments. |
| Validation | Constraint validation (`required`, `pattern`, `type`). The server still validates. |
| Date, time, date-find, date-period | `<input type="date">`, `type="time"`, `type="datetime-local">`. Default formats are already `yyyy-MM-dd` and `yyyy-MM-dd HH:mm`. The `T` separator from `datetime-local` is normalized to the space-separated timestamp. |
| Static drop-down | `<select>`. |
| Remote drop-down, combo-box, order-by multi-select, text autocomplete | Tom Select, loaded only on a page that has one of these. No jQuery. |
| Tree | A short renderer in `moqui-html.js`: fetch JSON, draw a nested list, navigate on click. |
| Click-to-edit, notification toast, WebSocket client | The same shared script. |
| Input mask | `placeholder`, `inputmode`, and `pattern`. A live mask is a qvt behavior. |
| `text-area` with `editor-type` `html` or `md` | A textarea. Rich editing stays in qvt. |
| Form-list column picker | SortableJS, loaded only with that dialog. |
| RSVP, the first file of `CombinedBase` | General use today, not a QZ Tray dependency. Not loaded. QZ's own scripts are sha-256, qz-tray, and MoquiQzComponent, and only when printing is enabled. See qvt. |

Bootstrap 5, htmx, and Alpine were set aside. Each one puts utility or behavior attributes on the elements, which is the size this mode exists to avoid.

### qvt

`qvt` stays the primary UI, on Vue 3.5 and Quasar 2.35. Delivery stays script tags: `vue.global.prod.js` (the build that includes the template compiler) and `quasar.umd.prod.js`. A tree-shaken bundle does not fit a renderer where the screen XML decides which Quasar components appear. Quasar 2 is a component and prop break from 1.22 (`modelValue` instead of `value` / `input`, and the matching changes in the `m-*` wrappers). It is the same pass as the library removals below.

When `server-static` includes `qvt`, that response is a data-free Vue template and the client merges the data. The attribute stays opt-in for this pass. The plan is [`plans/QvtServerStatic.md`](QvtServerStatic.md). The library choices in the rest of this section stand either way.

jQuery and Moment are removed. The qvt macros keep emitting the `m-*` wrappers defined in `WebrootVue.qvt.js`. `m-drop-down` wraps `q-select` and carries options-url, server-search, and depends-on. `m-date-time` wraps `q-input` with `q-date` and `q-time`. The macros do not emit a raw `QSelect` or `QInput`. Masks stay on that `q-input`. `QForm` rules cover validation, `$q.notify` covers toasts, and `m-tree-*` covers trees. `text-line[@ac-transition]` is already unsupported in qvt. Authors use a drop-down with server search.

What jQuery still does is helper work, and each call has a direct replacement:

- `$.ajax` becomes `fetch`. Form submit already uses `XMLHttpRequest` for file downloads.
- `$.extend`, `$.each`, and the `$.is*` checks become `Object.assign`, ordinary loops, and the `moqui.is*` helpers.
- The `#conf*` hidden inputs that the root app scrapes with `$().val()` become one JSON config object in the root template (session token, paths, user, locale, dark mode, plugin URLs).
- Focus today is jQuery `.focus()` plus the `.default-focus` class. The port keeps that with a template ref. The submit control is `buttonClicked`, set on click, or `document.activeElement` when that is unset (`m-form` and `m-form-link`).
- `m-editable` never calls Jeditable. Click-to-edit in qvt becomes Quasar's `QPopupEdit`. The current component is a stub: the jeditable call is a TODO and does not run. html mode does call jeditable, from `DefaultScreenMacros.html.ftl`, and the html shared script covers that behavior.

Moment is loaded (67 KB gzip, with every locale) and assigned to `Vue.prototype.moment`. The only `moment()` calls are inside the commented-out datetimepicker in `m-date-time`. Quasar's date utils use the same tokens the macros already emit (`YYYY-MM-DD`, `HH:mm`). The locale map that renames `zh` to `zh-cn` exists for Moment's locale files. Quasar language packs cover `QDate` and `QTime`.

`http-vue-loader` goes away with Vue 2. It splits a fetched `.qvt` into template, script, and style. MiniJinja emits a component object, and the compiler in `vue.global` compiles the template string. Vue 3 filters (`decodeHtml`, `format`) become functions.

`MoquiLib` stays as the utility set and `NotificationClient`. The Select2, jQuery Validate, and Bootstrap Notify setup at the bottom of that file is part of the html stack and is deleted. qvt already overrides `notifyGrowl` with `$q.notify`.

The other qvt libraries:

| Library | Plan |
|---|---|
| Font Awesome 4 and Material Icons | Both stay. Screen XML uses `fa fa-*`. The runtime components use Material icon names (`event`, `menu`). |
| CKEditor 4 (`editor-type="html"`) | Replaced by `QEditor`. CKEditor 4 is end of life. 4.22.1 is the last release under the open MPL, LGPL, or GPL license. Later 4.x releases are commercial. CKEditor 5 is GPL-2.0-or-later, which does not sit beside CC0 the way the MPL option did. `QEditor` simplifies HTML it does not understand. Wiki pages and product or category content HTML are the screens that store that markup. |
| SimpleMDE (`editor-type="md"`) | Replaced by a textarea and a preview rendered with Marked. SimpleMDE is unmaintained. |
| Chart.js 2.9 | Updated to 4.x (`chart.umd.min.js`), still lazy. Quasar has no chart component. The 2.x config does not carry forward. |
| Mermaid 9.3 | Updated off `mermaid.init` (removed in Mermaid 10), still lazy. Quasar has no diagram component. Pin a build that still loads from a script tag. |
| Marked, DOMPurify, highlight.js | Stay for the Assist panel, at current patch versions. highlight.js loads the languages Assist highlights, rather than the full build. |
| QZ Tray 2.0.7 | Updated to 2.3 when printing is enabled. The current conditional scripts are sha-256, qz-tray 2.0.7, and MoquiQzComponent. 2.3 uses native promises, so the SHA-256 script goes away. RSVP was never part of that set. `MoquiQzComponent` is rewritten onto `QDialog` and `QBtn`. The current component is Bootstrap markup inside the Quasar shell. |

## Sim Mode

An ExecutionContext can be placed in sim mode so that nothing permanent is written. Settings on the context control this, as they do in the Java framework. `TransactionCacheDb` is an embedded Postgres schema created for that context and dropped on close, in place of the private in-memory H2 database. The schema is on the overlay cluster. The per-transaction `TransactionCache` stays a map. HOLD, FLUSH, and `enter_sim` are ported from the Java code at the time the port begins. The database lifecycle is `plans/EmbeddedPostgres.md`.

## Component and Application Migration

- `mantle-udm`: entity XML and seed/install data. Expected to load as-is once the entity loader and data loader exist. Expression defaults or function expressions, if any, go through the JavaScript evaluator.
- `mantle-usl`: service XML stays. Groovy expressions and scripts are migrated to JavaScript (AI-assisted). XML Actions stay as XML and are expanded to JavaScript by MiniJinja macros.
- Screens: XML screen and form definitions stay. MiniJinja macros expand them. Render modes are in Screen Render Modes. Server-static `qvt` is [`plans/QvtServerStatic.md`](QvtServerStatic.md): the attribute stays opt-in, and expressions are JavaScript so the client can evaluate them. FreeMarker templates for notifications, email, and print move to Liquid.
- Applications built on Mantle follow the same pattern.

## Summary of Decisions

| Topic | Decision |
|---|---|
| Name | MR-Moqui (Modern Rust Moqui, “Mister Moqui”) |
| Repository | `moqui/mr-moqui`, combining framework and runtime |
| Spec language | XML (conf, entities, services, screens, SECAs, data) |
| Scripting | JavaScript via `rquickjs` (QuickJS-NG) |
| Groovy migration | AI-assisted, after the pre-scan in `plans/Implementation-Phases.md`. No long-lived compatibility layer. Decimal `+` and comparison become method calls. |
| Embedded database | `postgresql_embedded` 0.21 (theseus) as the default configuration |
| Production database | External Postgres, MySQL, etc when configured |
| Sim overlay | An embedded Postgres schema per `TransactionCacheDb`, on its own cluster. Behavior is a port of the Java framework at the time of the port. |
| Search | Postgres FTS + JSON as a first-class, OpenSearch-free path. Interface designed day one, not required to match ElasticFacade. |
| Transactions | Local one-phase commit. The Bitronix portions Moqui uses, on `DbDriver`. No XA two-phase commit. `suspend` parks the enlisted client on the transaction. |
| HTTP | axum in the host. Framework calls are synchronous. `tower-http` compression and `ETag` on static assets and the server-static qvt template. |
| Threads | One `ExecutionContext` per thread. Framework workers are real threads. No `Future` outside service `callFuture()`. |
| Async services | Local thread, and `callFuture()` when the caller waits. `async="distribute"` is dropped. |
| Sessions | Server-side session. A signed cookie carries the id. Same session for HTTP and the notification WebSocket. |
| Print and tool components | Later. Gate 3 `xsl-fo` is the FO document. PDF waits on `PdfRenderer`. The tool components stay outside until binary size is weighed. |
| WASM | Not a target. |
| XML Actions | MiniJinja macros emit JavaScript. In-memory interpretation is not the primary path. |
| Screens | MiniJinja macros emit server HTML and/or client-renderable JavaScript |
| Render modes | `qvt` is the primary UI (Vue 3.5, Quasar 2.35, no jQuery or Moment). `html` stays as terse server HTML with a small shared script. `text`, `csv`, and `xml` stay as they are. `xsl-fo` stays as the FO document. `vuet` is dropped. |
| User-authored templates | Liquid (`liquid` crate), sandboxed by design |
| Security | Moqui artifact authz on Rust primitives. No Shiro. |

The server JavaScript host method list is in `plans/ContextObjects.md`. The build order is `plans/Implementation-Phases.md`: the entity facade and the service facade are whole, and both are finished before gate 1, a data file load. Screen rendering is gate 3.
