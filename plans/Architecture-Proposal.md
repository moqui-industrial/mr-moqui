# MR-Moqui Architecture Proposal

**Modern Rust Moqui** (pronounced “Mister Moqui”)

This document captures the initial architecture direction for a Rust rewrite of Moqui Framework. It is the starting proposal for the new repository.

**STATUS NOTE**: Templating, screen render modes, print and PDF, the crate cut, search, qvt server-static, embedded Postgres, context objects, and the build order are specified. Templating is below: QuickJS runs scripts and templates. JavaScript modules expand XML Actions and screen/form XML, `JsTemplateLiteral` is `ec.resource.expand()`, and a fancy renderer (dialect still open) covers structured text, with a locked-down variant for less-trusted templates. Screen render modes are in Screen Render Modes. The crate cut is `plans/Crate-Layout.md`. Search is `plans/SearchFacade.md`. Server-static qvt is `plans/QvtServerStatic.md`. Embedded-database concurrency, the transaction manager's checkout, and the sim overlay's database lifecycle are `plans/EmbeddedPostgres.md`. What each environment can see, which types are shared, and the server host method list are `plans/ContextObjects.md`. The build order is `plans/Implementation-Phases.md`. The proposal will still move as the first components are built.

## Repository

- New repository: `moqui/mr-moqui`
- Combines what currently lives in `moqui-framework` and `moqui-runtime`. The split has had minimal payoff and high cost; the new repo is the single home for the framework and the default runtime.
- Higher-level components (`mantle-udm`, `mantle-usl`, applications) stay separate and continue to be defined primarily in XML.

## Goals

- Keep XML as the spec language for configuration, entities, services, screens, SECAs, and data files, with only the adaptations required by a Rust runtime.
- Move the heavy lifting (entity facade, service facade, screen rendering, tooling, and performance-sensitive service implementations) into Rust.
- Use one language (JavaScript) for scripts, templates, and inline expressions, on the server and in the browser, so that XML Screens can become client-renderable.
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

XML Actions (`entity-find`, `service-call`, `if`, `iterate`, `set`, and the rest) and screen/form XML are expanded by JavaScript modules into JavaScript and run by the embedded engine. An in-memory interpreter remains possible as a later optimization, but it is not the primary path. Expressions inside attributes, `<script>` bodies, and text templates are JavaScript. Details are in Templating.

Components remain directories with a `component.xml` descriptor, discovered and loaded at startup. Resource locations keep a scheme-based model (`component://`, and equivalents).

The framework is synchronous and multi-threaded, as Moqui is today. An `ExecutionContext` belongs to one thread. An HTTP request, an async service call, a scheduled job, and the other background workers each run on a thread with their own context. A call that waits (a database query, outbound HTTP, mail) blocks that thread and returns a value. Entity, screen, and HTTP APIs return values. `RestClient` stays blocking. The one wait handle is service-facade `callFuture()`: a worker-pool thread, a new `ExecutionContext`, and a oneshot the caller waits on. Framework code does not await on Tokio. `RestClient`'s blocking `reqwest` and each `postgres::Client` start a private runtime, as `plans/Crate-Layout.md` describes.

Worker threads that belong to the framework are ordinary threads started by the factory. The HTTP server lives in the host crate. It accepts connections on Tokio and hands each request to a thread from that pool, which calls the framework and waits for the response. The host does not use Tokio's `spawn_blocking` pool for this. That pool's cap is independent of the datasource pool. Each request, job, and async-service thread calls `destroyAllInThread` when it finishes. Transactions specifies what that closes. Shutdown drains the worker pools and the in-flight requests and jobs first, then closes open iterators and transactions and returns connections, then stops the postmasters, with a hard timeout on the drain. Transaction attributes run on the calling thread: begin, commit, rollback, a savepoint, or, for `force-new`, suspend then begin. The attributes are `ignore`, `use-or-begin` (the default), `force-new`, `cache`, `force-cache`, and `transaction-timeout`. How `suspend` parks the client is in Transactions. Details of the crate cut are in `plans/Crate-Layout.md`.

`service-call` with `async="true"` runs on the service worker pool and gets another `ExecutionContext`, as it does now. `callFuture()` is that same submit, and the caller waits on the oneshot. The pool is bounded: a configurable thread cap, a bounded queue, and a depth and wait metric. A `callFuture` submit against a full queue fails with a clear error and does not block a thread that may already hold a transaction. A `callFuture` whose caller is already a pool thread runs on a reserved worker, so the join cannot occupy every worker. Fire-and-forget that must survive a full queue is inserted as a one-shot service job, and the scheduler thread runs it. That includes commit and rollback synchronizations and any `afterCompletion` submit. `ServiceJobRun` stays the history row of a named job. Java's `worker-queue` of 65535 is the conf capacity of the in-memory queue, not the overflow policy. The joined sites are `org.moqui.impl.InstanceServices.check#AppInstance` (three checks) and `mantle.party.DuplicateServices.check#AllPartyDuplicates` (batches of ten). `init#AppInstance` starts work it does not join, and that stays `async().call()`, with the same spill when the queue is full. `async="distribute"` (run on any node in a cluster) is dropped. It is rarely used, and there is no cluster transport for it. The service-facade `distributed-factory` goes with it. Distributed cache invalidation stays a later listener on the in-process topic. It is not a cache trait and not a Redis dependency.

QuickJS is one runtime per `ExecutionContext`, used only on that context's thread. A function the script calls returns a value before the script continues. The workspace stays `panic = unwind`. Every host function the runtime calls is wrapped in `catch_unwind`, and a panic becomes a JS exception. A panic that crosses that boundary aborts the process. Decimal methods use checked arithmetic, so an overflow is that exception. `set_interrupt_handler` watches the transaction deadline from Transactions, and `set_memory_limit` is set so a runaway allocation is a script exception. The byte cap is a conf value filled in when phase 6 creates the runtime.

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
- Heavier options (V8 via `rusty_v8`) were considered and set aside. Binary size, build complexity, and the fact that most script work is business logic rather than tight loops made the lighter engine the better fit. The same engine runs templates. For the short strings `expand()` handles, and for a macro call that runs once per definition, V8 would cost more than it returns. If generated JavaScript or large templates later dominate a profile, switching this runtime to V8 can be reconsidered. It would replace QuickJS, not sit beside it. Pure-Rust engines (Boa and others) are not yet the right primary runtime for a large body of existing service code.

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

- An unset bare name is null in Groovy. `ContextBinding.hasVariable` always returns true (`ContextBinding.java:41-45`), so the get falls through to the context stack instead of raising `MissingPropertyException`. `if (someUnsetParam)` is false. In JavaScript the same line throws `ReferenceError` at runtime unless the server runtime resolves the name. That is the common migration failure: it fails when the script runs, not when it is compiled. The server mechanism is `plans/ContextObjects.md`. Measured 2026-10-11 in `runtime/component/mantle-usl`: 687 `<script>` blocks, 1,084 `<entity-find>`, 1,044 `<entity-find-one>`, and 38 `<entity-find-count>` (2,166 find elements). The framework service XML has the same shape.
- Elvis (`?:`) is a different operator from nullish coalescing. `''` and `0` are falsy in both Groovy and JavaScript, so `if ('')` and `if (0)` already match. `'' ?: 'x'` is `'x'` in Groovy, `'' ?? 'x'` is `''` in JavaScript, and `'' || 'x'` is `'x'`. Use `||` when the left side is a string or number that may be empty or zero. Use `??` only where the old expression was a null check. Do not mechanically rewrite `?:` to `??`.
- An empty list or empty map is falsy in Groovy and truthy in JavaScript. Neither `??` nor `||` reproduces that (`[] || 'x'` is `[]`). Conditions that mean “has rows” or “has entries” test length.
- Groovy `==` and JavaScript `==` both treat `'1' == 1` as true. The migration keeps `==`. The breaks are list and map equality (content in Groovy, reference in JavaScript) and host decimals: `==` between two decimals is reference equality and does not throw, `==` against a number throws, and a zero decimal is truthy. `+` is not overloaded.
- Groovy `fields[3..size-1]` is an inclusive range. `fields.slice(3, size)` is the JavaScript when `size` is the length, because `slice` excludes the end. Left as Groovy it is a syntax error, so it fails when the script is compiled. The review counted 23 in mantle-usl. The framework has one in `WikiServices.xml` (`extraPathNameList[1..size-1]`). `ScreenRenderImpl.groovy` has another and is a Rust port, not a script.

The QuickJS context mimics the Groovy and FreeMarker context, so expression and script migration stays small. A script starts with the same injected objects the Java runtime puts there: `ec`, `context`, a service `result` where the current actions provide one, and the other objects that entry point injects by default. A screen template sees what the FreeMarker macros see today. The static helpers those scripts and generated actions call are there too, including `ObjectUtilities`, `CollectionUtilities`, and `StringUtilities`.

Host values (`EntityValue`, `EntityList`, decimals, timestamps) expose the methods those scripts call. JDK types such as `BigDecimal` and `TreeSet` are small host helpers with the same call shape. Decimals stay `rust_decimal` host values and do not become JavaScript numbers. `NUMERIC(26,6)`, `(25,5)`, and `(24,4)` fit. mantle-usl has 903 `BigDecimal` tokens and 48 `setScale` calls in xml and groovy, measured 2026-10-11. Operators are not overloaded. `valueOf` throws an error naming the method to use, and `toString` works, so `` `${decimal}` `` interpolation works. `+`, relational operators, and `==` against a number or a string throw. `==` between two decimals does not call `valueOf` and returns false. A decimal is always truthy, including zero, so `if (amount)` and `amount || fallback` do not match Groovy `0.00G`. Scripts call `add`, `subtract`, `multiply`, `divide`, `eq`, `compare`, and `isZero`. `divide` uses scale `max(left, right, 10)`, and `1/3` is `0.3333333333`. The halfway rounding mode is measured from the Groovy in the Java tree when the host method is written. Checked operations return a script error when the 28-digit coefficient overflows. `bigdecimal` stays unused unless a migrated expression needs more than those digits. In the browser package a decimal is a string, so `"10.00" < "9.00"` is true and `"1.00"+"2.00"` concatenates. That case is a compatibility-report violation, as `plans/QvtServerStatic.md` specifies. Timestamps follow the same rule where coercion would be wrong. The pre-scan in `plans/Implementation-Phases.md` counts decimal arithmetic, `==`, and truthiness. The methods the generated script calls are `plans/ContextObjects.md`. A tighter sandbox on which framework calls a script may make is a later effort.

### Migration approach

A general Groovy-to-JavaScript transpiler is not planned, and a long-lived compatibility shim for Groovy syntax is not desired. The preferred path is AI-assisted update of expressions and scripts, after the pre-scan in `plans/Implementation-Phases.md`. A temporary, narrowly scoped rewrite of the common attribute-expression idioms above may be used during the transition if it proves helpful, but it should not become something that ships and is relied on in production. The cost of manual migration is no longer a reason to keep risky compatibility layers.

`mantle-udm` (entity definitions and seed data) should load with little or no change once the entity model and data loader exist. `mantle-usl` requires migration of Groovy expressions and `<script>` bodies to JavaScript. XML Action elements stay as XML. The macro library that expands them changes from FreeMarker (emitting Groovy) to a JavaScript module (emitting JavaScript).

## Templating

FreeMarker currently does two jobs: it expands XML Actions and screen XML into code, and it renders text against the live context. Both become JavaScript on the QuickJS runtime that already runs scripts. Scripts and templates share a language, a context, and the host objects in `plans/ContextObjects.md`.

`GStringTemplateRenderer` was an earlier experiment along this line. It rendered a file as a Groovy `GString` through `groovy.text.GStringTemplateEngine`, and `MoquiDefaultConf.xml` registers it for the `gstring` and `html.gstring` extensions. It was viable. Rewriting the FreeMarker macro library was not worth it at the time, so the FTL set stayed. This port picks that experiment back up, with JavaScript instead of Groovy, and includes the macro libraries.

QuickJS-NG through `rquickjs` is the engine for scripts and for every template renderer below. V8 stays the later option described under Why rquickjs / QuickJS-NG. It is not a second engine in this port.

### Template renderers

`TemplateRenderer` is a trait, the same shape as `org.moqui.context.TemplateRenderer`. Conf maps a file extension to an implementation, as `<template-renderer>` does today. The set stays open. A MiniJinja or Liquid renderer can be registered later if a concrete template set wants that syntax. Neither is part of this port.

Three implementations are planned. All three run on the `ExecutionContext`'s QuickJS runtime. A template is compiled once and the function is cached, by location and last modified for a file, and by source text for a string. That is the cache `GStringTemplateRenderer` keeps for its Groovy template.

**`JsTemplateLiteral`.** The file or string is the body of an ES6 template literal. The renderer escapes backslashes and backticks, wraps the text in a function that returns the literal, and compiles it. `${` stays an expression, so an author writes `\${` for a literal dollar brace. This is the renderer `ec.resource.expand()` uses, in place of the Groovy `"""..."""` script `ResourceFacadeImpl.expand` compiles today. Localization of an input shorter than 256 characters still happens before that compile. Bare names resolve through the server proxy. The live context is visible, including `ec` on a request. An expression inside `${}` is JavaScript, so this is the same trust level as a script. It fits the short component-authored strings the framework already expands: messages, subjects, from addresses. It has no statements. A loop is an expression, or the template belongs on the fancy renderer.

**Fancy renderer.** Control flow, iteration, and partials, for a template that is mostly text with structure. Inline `<text template="true">` is this case today. `DefaultScreenMacros.any.ftl` compiles the body with FreeMarker's `?interpret`, and bodies such as `framework/screen/ScreenRenderEmail.xml` use `<#if>`, `<#list>`, and `<#assign>`. Those bodies are rewritten into the fancy dialect. The dialect and the JavaScript library are not chosen yet. Mustache.js is the candidate: small, plain JavaScript, no Node APIs, run inside QuickJS. If that candidate is kept, the renderer is named `JsMustache`. Another library gets its own name and conf extension when it is chosen. The trusted fancy renderer sees the same live context as `JsTemplateLiteral`. The library is loaded once per runtime. Each template is compiled once and cached.

**Locked-down fancy renderer.** The same dialect and the same library, with a different context. If the fancy renderer is `JsMustache`, this one is `JsMustacheRestricted`. It receives the unwrap snapshot from `plans/ContextObjects.md`: plain values, decimals as strings, no `ec`, no `sri`, no live entity value. Format and localize are plain functions and do not take an `ExecutionContext`. This is new. Moqui's sandbox options have been minimal, and untrusted FreeMarker, GString, and script templates have not been accepted. Component-authored and framework templates stay on the trusted renderers. A template that may be edited by someone who is not a component author uses this renderer, selected by its own conf extension. `expand()` does not use it. A stored string that must not run as a script is rendered here, not passed to `expand()`.

Email bodies stay screens, rendered in the `html` and `text` modes. Subject, from address, and the other `ec.resource.expand` call sites use `JsTemplateLiteral`.

### Code generation

XML Actions and screen/form XML are expanded by JavaScript functions, one per element name. Today the FreeMarker macros emit Groovy. These emit JavaScript, run on the same QuickJS runtime.

The macro file is a JavaScript module. `macro-template-location` points at that file, so a theme still overrides a render mode by replacing the file, with no framework rebuild. Functions correspond to the XML elements (`entityFind`, `serviceCall`, `formSingle`, `field`, and so on). The Rust walker dispatches with `macros[elementName](node, children)`. The node is a plain object of attribute strings and child element text. `children` is the already-rendered output of the child elements, and the function places that string, including inside a loop it emits. A function may instead take `visit` and call it where the child output belongs. The function returns a string. It does not see the request `ExecutionContext`. Request data is read later, by the generated script.

The functions build that string with template literals. Attribute and text values reach the generated JavaScript only through one `jsStr()` helper, and a property test runs values containing quotes, backslashes, newlines, and `${` through every function. Autoescape stays off: the output is code, and value escaping happens at runtime through the host helpers. Dates are formatted by `ec.l10n` in the generated script.

Server HTML and client-renderable screen JavaScript are two modules, or one module that takes the mode. `qvt` emits a component object. A server-static template is compiled on the server, as `plans/QvtServerStatic.md` describes. Server-merged `qvt` is compiled by the compiler in `vue.global`.

This keeps the properties that matter:

- The modules are ordinary text. They can be edited without rebuilding the framework.
- The thing that runs is generated JavaScript, so behavior is visible and ordinary script tooling applies.
- Location information from the original XML is written into the generated script as comments or a source map, so stack traces and debugging point back at the service or screen file.
- Golden tests pin the generated JavaScript for a fixture per element. The modules are the framework's compiler backend, and these tests are what make an edit to them safe.
- Composition is a function call.

An in-memory interpreter for XML Actions remains possible as a later optimization or for elements that are awkward to generate, but it is not the primary path. Generated JavaScript is preferred for lower execution overhead, well-known behavior, and easier debugging.

### Summary

| Job | Implementation | Notes |
|---|---|---|
| XML Actions → JavaScript | QuickJS macro module | Replaces FTL → Groovy. Emits debuggable JS. Sees the element, not the request. |
| Screen/form XML → JavaScript or HTML | QuickJS macro module | One module per mode, or one module that takes the mode. `macro-template-location` still selects the file. |
| `ec.resource.expand()`, short component text | `JsTemplateLiteral` | Template-literal body, live context. Successor of the Groovy string `expand()` compiles. |
| Structured text, including `<text template="true">` | Fancy renderer | Dialect and library are open. Mustache.js is the candidate (`JsMustache`). Live context. |
| Less-trusted templates | Locked-down fancy renderer | Same library, unwrap snapshot, no `ec`. New. `expand()` does not use it. |

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

The default embedded database is one cluster, so each thread has its own connection. Embedded search is the `search` schema on that cluster. The sim overlay is a second cluster, one schema per `TransactionCacheDb`. Details are in `plans/EmbeddedPostgres.md`. External Postgres remains the production database when one is configured, and default search tables stay on that same database. External Postgres and MySQL are `DbDriver` implementations in the host. The pool and the transaction manager live in core and call that trait. Each Postgres client drives its own runtime, as `plans/Crate-Layout.md` and `plans/EmbeddedPostgres.md` describe. Pool sizes, the reserve, and timeouts are the Transactions section.

### Transactions

There are no JTA types and no XA two-phase commit. Transactions are local to the single transactional datasource. The transaction manager is Moqui's own, written against `DbDriver`: `TransactionFacade` semantics, a pool, and a synchronization list. There is no Rust JTA-style manager to adopt (the existing pools are connection lifecycle only, and the distributed-transaction crates are network coordinators), so this is Moqui code maintained here, with Bitronix as the behavior reference.

The manager covers the local one-phase path and the facade behavior. When the conf has no `xa-ds-class`, `TransactionInternalBitronix` wraps a plain JDBC connection in `LrcXADataSource` and commits it in one phase. Postgres and MySQL take that path. The pool settings come from the datasource conf: min, max, idle time, acquisition timeout, a test on checkout, and `pool-reserve` (default 8) held inside the max. The code fallback when `pool-maxsize` is omitted is min 5, max 50 (`TransactionInternalBitronix.groovy`). The postgres `<database>` sets max 90, under a default `max_connections` of 100. A production comment in the Java conf sizes a busy server at about 120 for 100 request threads and 16 workers. That is an example, not the default. Startup fails when the request-thread cap plus the service worker cap exceeds `pool_max - reserve`.

`begin` records the transaction and its deadline and does not check out a connection. The first statement checks one out and enlists it. A call outside a transaction checks out and returns. One enlisted connection is then shared for the rest of the transaction. That is the connection stash, and it is always on. `use-connection-stash="false"` is a startup error. `TransactionFacadeImpl` is the API on top: begin-or-join, commit, rollback, `setRollbackOnly`, timeout, and synchronizations.

Synchronizations run in fixed phases, not registration order. Bitronix runs `beforeCompletion` in reverse registration order, all at one position (`BitronixTransaction.fireBeforeCompletionEvent`), and `afterCompletion` forward. Java's data-feed `beforeCompletion` is empty (`EntityDataFeed.groovy`), so that reversal does nothing today. The port's phases are: transaction-cache flush, then the same-database data feed on the enlisted connection, then commit or rollback, then `afterCompletion` forward for `runOnCommit` / `runOnRollback`, service ECAs, and the data feed when search is a different database or OpenSearch. The feed rule is `plans/SearchFacade.md`.

In Java, `suspend` closes the Moqui connection handles and then calls `tm.suspend()` (`TransactionFacadeImpl.groovy:544-548`). Bitronix LRC keeps the physical session with the suspended transaction and returns that session on resume. A Postgres or MySQL local transaction is that session. `PREPARE TRANSACTION` ends it, so the port does not use it. `suspend` parks the enlisted client on the transaction. The client leaves the thread and stays off the free list. `resume` checks that same client back out. Uncommitted work stays on that session. `runRequireNew` in Java always starts a thread (`requireNewThread = true`) and may reuse the `ExecutionContext` on that thread. The port does not. `force-new` is same-thread suspend, then begin.

`begin` joins an active transaction and returns false. Commit and rollback run only when this frame began the transaction. A failed commit rolls back to clean up. `setRollbackOnly` stores the cause message, the cause error, and the callsite (`RollbackInfo`). A later commit becomes a rollback and logs all three. `force-new` parks the current client, when one is checked out, and begins a new transaction on the same thread. The calling thread holds one connection. The parked client still counts against the pool max. Ordinary checkouts cannot take a reserve connection, and they fail when only reserve connections are idle. `pool-time-wait` is that fail-fast. There is no idle spare kept for a parked client.

Side-channel work runs on a reserve connection in its own short transaction. It does not nest a second enlisted connection inside the caller. That work is sequence-bank refill, runtime DDL, service semaphores, failed-login counters, login keys, job-run rows, notification saves, and email status updates. Sequence banks stay on `SequenceValueItem`, including the in-memory bank (default size 50), stagger, and the start at 100000. The refill runs when the bank is exhausted, inside `sequencedIdPrimary` on the create path. Postgres uses `INSERT ... ON CONFLICT DO UPDATE ... RETURNING`. MySQL uses a short autocommit `SELECT ... FOR UPDATE` and `UPDATE` on the reserve connection. A native sequence is not the mechanism.

The deadline comes from `transaction-timeout`, default 60 seconds (`TxStackInfo`, and Bitronix's default). A timer thread marks the transaction rollback-only at the deadline and cancels the active statement, so the connection comes back. Postgres uses `CancelToken`. MySQL uses the driver's cancel. `use-statement-timeout` defaults false, as `entity_statement_timeout` does. When it is true, each statement's server timeout is the time remaining until that deadline (`statement_timeout`, MySQL `max_execution_time`). Java submits the JDBC call to another thread and waits with `Future.get` (`EntityQueryBuilder`). That side thread is not ported. The QuickJS interrupt handler watches the same deadline.

The service attributes preserved are `ignore`, `use-or-begin` (the default), `force-new`, `cache`, `force-cache`, and `transaction-timeout`. `cache` is `use-or-begin` with the transaction cache on. `force-cache` is `force-new` with the transaction cache on. A call-site `requireNewTransaction` is `force-new`. A call-site `useTransactionCache` is `cache`. `transaction-isolation` is schema-only in Java and is not ported. No service XML in this tree sets `transaction="cache"` or `transaction="force-cache"`. The attributes are still parsed.

`use-lock-track` defaults false (`entity_lock_track`). When on, an in-memory map warns if two transactions touch the same entity primary key. It is not a database lock. Every request, job, and async-service thread calls `destroyAllInThread` in a `finally` block. It commits a transaction left open, resumes and commits suspended transactions, clears that stack's lock-track entries, and returns parked connections.

An unhandled statement error marks the transaction rollback-only, and no further statement runs on that connection. Postgres aborts the transaction on any error until rollback or rollback-to-savepoint. `never-try-insert="true"` stays on the postgres `<database>`. `useTryInsert` is forced off then, and the loader uses `createOrUpdate`. When try-insert is on, Postgres uses `INSERT ... ON CONFLICT` and MySQL uses `INSERT ... ON DUPLICATE KEY UPDATE`. `ignore-error="true"` clears service messages after a normal return (`XmlActions.groovy.ftl`). It does not catch a thrown SQL error. Codegen emits a savepoint around a `try` whose body contains an entity or service call, and rolls back to it when the `catch` continues. mantle-usl has 11 such `catch` sites and 5 `ignore-error="true"` attributes. The phase list is `plans/Implementation-Phases.md`.

The manager and the pool live in core, next to the transaction facade. The manager calls `DbDriver` and does not import `postgres` or `mysql`. Embedded Postgres, external Postgres, and MySQL are the driver implementations in the host. `plans/Crate-Layout.md` places them.

At most one writable connection is enlisted. A second writable group whose database is not the primary is a startup configuration error. `logging` in the default conf is `ElasticDatasourceFactory`, not a second SQL connection. A group marked nontransactional is never enlisted. Its connections autocommit. The commented-out `nontransactional` example stays documentation. `transactional#clone1` is disabled by default (`entity_ds_c1_disabled`). `useClone` checks out the clone, or the primary when no clone is enabled, and does not enlist it. Clone reads are not part of the transaction. Java enlists that XA connection. Two-phase commit would add resources at the `DbDriver` boundary (Postgres has `PREPARE TRANSACTION` for it) without changing the facade, and nothing is built for that case.

The pool is Moqui's own. An r2d2 guard is `Send`, and holding it on the transaction parks the connection, so parking alone does not require a new pool. The core pool exists for the reserve, gauges for checked-out, parked, and waiting connections, lazy checkout, and one implementation behind `DbDriver` for Postgres and MySQL.

The Java library does not carry forward, and the manager does not link `moqui/bitronix`. That fork (4.0.1, January 2026) is the behavior reference. Call it the transaction manager, not a Bitronix port: what ships is facade semantics, a pool, and synchronizations. Bitronix is about 17,800 lines, most of it the XA journal, crash recovery, and JMS. Those stay out until a datasource is configured with a real XA resource. No current Rust crate is this in-process manager: the async pools are the same layer as a generic connection pool, and the distributed-transaction crates are network coordinators. Cross-resource work uses an outbox or a SECA rather than two-phase commit. Each Postgres and MySQL connection keeps an LRU of prepared statements, cap 100, keyed by SQL text, matching `setPreparedStatementCacheSize(100)`. `query(&str)` prepares a named statement and closes it on drop (`tokio-postgres` 0.7.18). It is not a cache. Entity SQL uses the LRU. `query_typed` is the one-shot path for a statement that should not take a cache slot.

## Search

OpenSearch-free deployments are a day-one goal. Postgres full-text search and JSON operators cover the common cases that ElasticFacade is used for today (full-text and parameterized document-style queries).

The search interface should be designed so that it is not locked to the current ElasticFacade shape. If a cleaner interface falls out of supporting both a Postgres implementation and an optional OpenSearch implementation, that interface is part of the MR-Moqui migration, not a later retrofit. OpenSearch remains available as an optional backend for deployments that want it. It is no longer required.

When the entity datasource is Postgres, embedded or external, default search tables are the `search` schema on that database. A search `<postgres>` URL is how search points at a different Postgres. When the entity datasource is not Postgres, search is its own embedded cluster. Naming an external database for a store does not also start an embedded postmaster for that store. See `plans/SearchFacade.md` and `plans/EmbeddedPostgres.md`.

The interface, the feature set taken from current callers, and the two backends are specified in `plans/SearchFacade.md`.

## Other Infrastructure

| Area | Direction |
|---|---|
| HTTP server | `axum` + `tower` in the host. The framework sees an owned request and blocks until the response is ready. WebSocket via axum `extract::ws`. That module needs the `ws` feature, which is not in axum's default set. The feature pulls `tokio-tungstenite`, so the host does not depend on it directly. `tower-http` serves compression and an `ETag` on static assets and on the server-static qvt template. `Cache-Control` for that template stays as `plans/QvtServerStatic.md` specifies. HTTP client via blocking `reqwest`. No servlet container, no JNDI. |
| Screens and XML Actions | JavaScript modules expand XML to JavaScript. Screen and form XML can also emit server HTML. No FreeMarker, and no native widget-tree renderer as the primary path. See Templating and Screen Render Modes. |
| Text templates | `JsTemplateLiteral` for `ec.resource.expand()` and short text. A fancy renderer, dialect open (Mustache.js is the candidate), for structured text. A locked-down variant of that renderer for less-trusted templates. All on QuickJS. |
| Security | Do not port Shiro. Reimplement UserFacade on `argon2` 0.6. The `password-hash` feature is optional and part of the crate's default set; it pulls `password-hash` 0.6. A dependency that turns default features off still enables `password-hash`. The framework owns the session. Artifact authorization stays Moqui’s own model and tables. See Sessions and login. |
| Cache | A port of `MCache` and `MCacheManager`. No second cache implementation. Distributed invalidation, later, is a listener on the in-process topic in `moqui-lib`. Hazelcast does not carry forward. |
| Logging | `tracing`. Metrics are the same instrumentation: pool and worker-pool gauges, statement spans and timings, and artifact-stack spans, exposed on `/metrics`, with `/health` at the edge. Hit bins are a subscriber on those spans. |
| JSON / XML | `serde_json`. XML is the `MNode` port in `moqui-lib`. Conf is XML. The REST schema export is OpenAPI 3.1, written as JSON by the `RestSchemaUtil` port and JSON Schema-aligned, so the `entity.json` and service schema transitions stay. Swagger 2.0 is a candidate to leave out and is ported only when a concrete consumer needs it; RAML is left out. `serde_yaml` is not a dependency. |
| CSV, mail, markdown, cron, TOTP | Ordinary crates (`csv`, `lettre`, `pulldown-cmark` / `comrak`, `croner` 4, `totp-rs`). The cron parser is the service module in `plans/Crate-Layout.md`. |
| Content repository (JCR) | Dropped. `content://` is not remapped onto blob storage. Resolving a `content://` location is an error that names the location. `db://` stays. Filesystem and S3 are their own resource implementations when a deployment asks for one. |

JDBC drivers, H2, the Bitronix library, Shiro, FreeMarker, Groovy, and the servlet stack do not carry forward. The transaction behavior Moqui relies on is reimplemented in Moqui's own manager on `DbDriver`, as Transactions describes. FreeMarker's two jobs move to JavaScript on QuickJS: macro modules for code generation, and the template renderers for text. Where Moqui already has an implementation because the general library was the wrong shape, that implementation is ported. `MNode`, `MCache`, and the transaction manager are the cases. An external crate earns its place when it is the engine, or when another crate this runtime already needs pulls it in.

## Sessions and login

The servlet session and Shiro do not carry forward. The framework keeps a server-side session of its own. A signed cookie carries the session id. The factory keeps the session struct in memory and writes a snapshot of it under `runtime/sessions` on change, so a restart reads it back. A clustered deployment is sticky-session until a deployment asks for a shared store, and that store lands behind the same session struct. The session holds what the Java session holds: the authenticated user, visit and visitor ids, locale and time zone, the CSRF session token (`moqui.session.token`), flash messages and errors, saved and error parameters, and screen history. The snapshot is those fields. It does not include the QuickJS runtime, the transaction, or a connection. The notification WebSocket uses the same session.

Login, logout, and the check that the user has logged out elsewhere live on this session. A successful login replaces the session id and removes the previous snapshot. A stored password whose `passwordHashType` is not argon2 is verified with that algorithm and then rehashed. The construction is phase 5 of `plans/Implementation-Phases.md`. A second factor in progress (email or TOTP) stays on the session until authentication completes. An API key or a login key on the request can authenticate without a password in the session. The long-lived visitor cookie stays separate from the session cookie. Artifact authorization is unchanged and reads the user from this session.

## Tool components and print

Server-side page layout is the `html-print` render mode: HTML and CSS, with `@media print` and `@page`. An open `html` or `qvt` screen still prints from the browser dialog through the rules already in `webroot-layout.css` and `WebrootVue.qvt.css`. A document that is emailed as a PDF, sent to a printer, or opened for print is `html-print`. The dedicated documents (SimpleScreens invoices, checks, statements, pick and pack) are rewritten from `*.xsl-fo.ftl` to `html-print` templates in phase 12. Their data-prep services stay. The screen XML that includes them (`<text type="xsl-fo" location="...">`) keeps that type, because the type maps to this mode. Strict page layout is not a goal. Flowing text, a repeating table, and a header and footer are. A check or print form on preprinted stock stays absolute placement, in CSS.

`html-print` is its own JavaScript macro module, a sibling of `html`. It emits the print document: `@page`, page header and footer, breaks, and flowing tables. It does not load `moqui-html.js`. Conf `screen-text-output` for type `html-print` is mime `text/html` and `always-standalone="true"`, the standalone flag the old `xsl-fo` entry had, so the print document keeps the full render path for its header and footer. The name `xsl-fo` is an alias of this mode, rewritten to `html-print` before macro lookup. A request extension `.xsl-fo` or `.html-print`, a stored `renderMode` or `screenRenderMode` of `xsl-fo`, and a link parameter `renderMode=xsl-fo` all select that one module. A screen `render-modes` list that names `xsl-fo` still allows `html-print`. A `<text type="xsl-fo">` matches the mode, as does `<text type="html-print">`. The active mode the macros see is `html-print`. `DefaultScreenMacros.xsl-fo.ftl` and the page masters in `webroot/screen/includes/Header.xsl-fo.ftl` (letter, legal, tabloid, A4, A3, portrait and landscape) are the behavior this module replaces, as `@page` sizes. They are not ported as FO output. FOP is not ported. `moqui-fop` already has an HTML attempt beside FOP: `HtmlRenderer` on CSSBox, exposed as `/htmlr/*`. That renderer is not carried forward, and neither is the `/fop/*` servlet. Java's `xslFoTransform` is an identity XSLT into the FOP handler, and every in-tree caller passes a null stylesheet. That transform is not ported. Callers that store `xsl-fo` to mean a PDF still get one. Email attachments (`screenRenderMode`), scheduled screens (`ScreenScheduled.renderMode`, including the schedule form whose option key is `xsl-fo` and whose label is PDF), and print jobs render `html-print` and pass it to `PdfRenderer`. The file they attach or store stays `application/pdf`, and the filename extension stays `pdf`, as `ScreenServices` does today when `renderMode` is `xsl-fo`. A direct `.html-print` or `.xsl-fo` request returns the print HTML. Existing stored rows keep the value `xsl-fo`.

`PdfRenderer` is implemented in the host `pdf` module. The engine is one long-lived Chromium process. The crate is `chromiumoxide` 0.9 (MIT OR Apache-2.0). It speaks the DevTools Protocol and attaches to a browser the host already started (`Browser::connect`). `html2pdf` is a command-line wrapper over `headless_chrome` and starts a browser per run. `headless_chrome` is the older library this crate grew out of. `chromiumoxide` is the candidate because an instance that prints (email attachments, warehouse printers that run all day) keeps one browser and opens a page per document. The crate can be revisited when the module is built.

The browser binary is an external dependency. It is Chromium, and it is not vendored and not downloaded. Chromium is BSD. Google Chrome, and the `chrome-headless-shell` build Google ships with Chrome for Testing, are acceptable on a developer machine and are not the deployment binary. `chromiumoxide_fetcher` stays out. Conf `pdf-chromium-path` names the executable. When it is empty the host looks for `chromium` and then `chromium-browser` on `PATH`. A developer may point the setting at Google Chrome. When no binary is found the server still starts, and a caller that asks for PDF gets the report that PDF is not available.

When a binary is found, the host starts it once at factory startup and keeps it for the life of the process. The arguments are `--headless=new`, `--remote-debugging-address=127.0.0.1`, `--remote-debugging-port=0`, `--user-data-dir` under the runtime temp directory, `--no-first-run`, `--disable-gpu`, and `--disable-extensions`. The sandbox stays on. The host reads the DevTools WebSocket URL from the process output, or from `http://127.0.0.1:<port>/json/version`, and connects. The `chromiumoxide` handler runs on its own thread and its own runtime, not on an axum task and not on the `ExecutionContext` thread. `PdfRenderer` blocks that worker until the bytes come back, the same shape as a database call.

Each document opens a page, sets the `html-print` document on it, and closes the page after `Page.printToPDF`. The browser does not navigate to a caller-supplied URL, and the page does not receive the session cookie or `ec`. `preferCSSPageSize` is set so the document's `@page` rule chooses the paper. A conf cap limits concurrent pages, default 4. Further renders wait. A conf timeout closes a page that does not finish, default 60 seconds. If the process exits, the host starts it again. A render that was in flight fails with that error.

QuickJS stays the script engine. Chromium's V8 runs only if the finished document contains a script. The two runtimes are not interchangeable.

A build feature `pdf-servo`, off by default, is the alternate for a deployment that cannot spawn a process. It links Servo and renders in process. Servo's print path is not at parity with Chromium, so templates are written against the Chromium engine, and this feature is the build to try where a child process is forbidden. The default host feature `pdf` links `chromiumoxide` with `tokio-runtime` and `rustls`. `native-tls`, `async-std-runtime`, and the browser fetcher stay off. A build may enable either feature or both. With neither, `PdfRenderer` is the stub that reports PDF unavailable. Until phase 12 the module is that stub.

The other tool components stay a size decision, as the `moqui.war` size kept them out of Moqui Java: `moqui-poi`, `moqui-image`, `moqui-cups`, `moqui-sso`, and `moqui-sftp`. `moqui-cups` consumes the PDF bytes. It does not render them. Some of them may move into the framework. Admin access is not a script console and not a web eval endpoint: an SSH server in the host (exec and file access, authenticated through the user facade and artifact authz) is the better shape, and it pairs with `moqui-sftp`. That is an idea for a later pass, not part of the port. The size of the base `moqui.war` was a reason they stayed outside Moqui Java, and the size of the MR-Moqui binary is the same kind of limit. The default binary still includes the embedded Postgres distribution, including for a deployment whose entity database is MySQL, because search and sim use it. A custom build may omit `postgresql_embedded` only when search is an external Postgres or OpenSearch and sim is unused. Chromium is not part of that binary.

## Screen Render Modes

A screen is requested with a render-mode extension (`.html`, `.html-print`, `.qvt`, `.text`, `.csv`, `.xml`, `.xsl-fo`). `.xsl-fo` selects `html-print`. A JavaScript macro module replaces the FreeMarker macro set for every mode. The modes themselves change as follows. `/qapps` stays the default UI. `/apps` stays the server-rendered HTML UI. `/vapps` goes away.

### Modes that stay

`text`, `csv`, `xml`, and `html-print` stay server-rendered output formats. `html-print` is the print document in Tool components and print, and `xsl-fo` is the old name of that mode. These modes load no browser libraries. Until phase 12, a job or print that asks for a PDF reports that PDF is not available.

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

### html-print

`html-print` is the print document. The macro module and the `xsl-fo` alias are in Tool components and print. The output is HTML with print CSS. A browser can open `.html-print` and use the print dialog. `PdfRenderer` consumes this output. The mode does not load `moqui-html.js`.

### qvt

`qvt` stays the primary UI, on Vue 3.5 and Quasar 2.35. Delivery stays script tags, plus `quasar.umd.prod.js`. A page whose qvt output, including fragments, is entirely server-static loads `vue.runtime.global.prod.js`. Server-merged `qvt`, and a server-static shell that can receive a merged fragment, load `vue.global.prod.js` (the build that includes the template compiler). Which page gets which build is `plans/QvtServerStatic.md`. A tree-shaken bundle does not fit a renderer where the screen XML decides which Quasar components appear. Quasar 2 is a component and prop break from 1.22 (`modelValue` instead of `value` / `input`, and the matching changes in the `m-*` wrappers). It is the same pass as the library removals below.

When `server-static` includes `qvt`, that response is a data-free Vue template and the client merges the data. The attribute stays opt-in for this pass. The plan is [`plans/QvtServerStatic.md`](QvtServerStatic.md). The library choices in the rest of this section stand either way.

jQuery and Moment are removed. The qvt macros keep emitting the `m-*` wrappers defined in `WebrootVue.qvt.js`. `m-drop-down` wraps `q-select` and carries options-url, server-search, and depends-on. `m-date-time` wraps `q-input` with `q-date` and `q-time`. The macros do not emit a raw `QSelect` or `QInput`. Masks stay on that `q-input`. `QForm` rules cover validation, `$q.notify` covers toasts, and `m-tree-*` covers trees. `text-line[@ac-transition]` is already unsupported in qvt. Authors use a drop-down with server search.

What jQuery still does is helper work, and each call has a direct replacement:

- `$.ajax` becomes `fetch`. Form submit already uses `XMLHttpRequest` for file downloads.
- `$.extend`, `$.each`, and the `$.is*` checks become `Object.assign`, ordinary loops, and the `moqui.is*` helpers.
- The `#conf*` hidden inputs that the root app scrapes with `$().val()` become one JSON config object in the root template (session token, paths, user, locale, dark mode, plugin URLs).
- Focus today is jQuery `.focus()` plus the `.default-focus` class. The port keeps that with a template ref. The submit control is `buttonClicked`, set on click, or `document.activeElement` when that is unset (`m-form` and `m-form-link`).
- `m-editable` never calls Jeditable. Click-to-edit in qvt becomes Quasar's `QPopupEdit`. The current component is a stub: the jeditable call is a TODO and does not run. html mode does call jeditable, from `DefaultScreenMacros.html.ftl`, and the html shared script covers that behavior.

Moment is loaded (67 KB gzip, with every locale) and assigned to `Vue.prototype.moment`. The only `moment()` calls are inside the commented-out datetimepicker in `m-date-time`. Quasar's date utils use the same tokens the macros already emit (`YYYY-MM-DD`, `HH:mm`). The locale map that renames `zh` to `zh-cn` exists for Moment's locale files. Quasar language packs cover `QDate` and `QTime`.

`http-vue-loader` goes away with Vue 2. It splits a fetched `.qvt` into template, script, and style. The qvt macro module emits a component object. A server-static template is compiled on the server. Server-merged `qvt` is compiled by the compiler in `vue.global`. Vue 3 filters (`decodeHtml`, `format`) become functions.

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
- `mantle-usl`: service XML stays. Groovy expressions and scripts are migrated to JavaScript (AI-assisted). XML Actions stay as XML and are expanded to JavaScript by the macro module.
- Screens: XML screen and form definitions stay. JavaScript macro modules expand them. Render modes are in Screen Render Modes. Server-static `qvt` is [`plans/QvtServerStatic.md`](QvtServerStatic.md): the attribute stays opt-in, and expressions are JavaScript so the client can evaluate them. Inline `<text template="true">` bodies move to the fancy renderer. Email bodies stay screens. `ec.resource.expand()` is `JsTemplateLiteral`.
- Applications built on Mantle follow the same pattern.

## Summary of Decisions

| Topic | Decision |
|---|---|
| Name | MR-Moqui (Modern Rust Moqui, “Mister Moqui”) |
| Repository | `moqui/mr-moqui`, combining framework and runtime |
| Spec language | XML (conf, entities, services, screens, SECAs, data) |
| Scripting | JavaScript via `rquickjs` (QuickJS-NG), for scripts and templates. V8 is a later replacement only if a profile shows the JavaScript is the heavy part. |
| Groovy migration | AI-assisted, after the pre-scan in `plans/Implementation-Phases.md`. No long-lived compatibility layer. Decimal `+`, comparison, `==`, and truthiness become method calls. |
| Embedded database | `postgresql_embedded` 0.21 (theseus) as the default configuration |
| Production database | External Postgres, MySQL, etc when configured |
| Sim overlay | An embedded Postgres schema per `TransactionCacheDb`, on its own cluster. Behavior is a port of the Java framework at the time of the port. |
| Search | Postgres FTS + JSON as a first-class, OpenSearch-free path. Interface designed day one, not required to match ElasticFacade. |
| Transactions | Local one-phase commit. Moqui's own transaction manager (facade semantics, pool, synchronizations) on `DbDriver`, with Bitronix as the behavior reference. No XA two-phase commit. Checkout is lazy. A reserve inside the pool serves side-channel work. Synchronizations run in fixed phases. `suspend` parks the enlisted client on the transaction. |
| HTTP | axum in the host. Framework calls are synchronous. `tower-http` compression and `ETag` on static assets and the server-static qvt template. |
| Threads | One `ExecutionContext` per thread. Framework workers are real threads. No `Future` outside service `callFuture()`. |
| Async services | The bounded service worker pool, and `callFuture()` when the caller waits. A full queue spills fire-and-forget work to a one-shot service job. `async="distribute"` is dropped. |
| Sessions | Server-side session. A signed cookie carries the id. A file snapshot under `runtime/sessions` survives a restart. Same session for HTTP and the notification WebSocket. |
| Print and PDF | `html-print` (print CSS and `@page`), rendered by one long-lived Chromium process through `chromiumoxide` 0.9. The binary is external Chromium. `pdf-servo` is an off-by-default in-process alternate where a child process is forbidden. The name `xsl-fo` selects `html-print`. The other tool components stay outside until binary size is weighed. |
| WASM | Not a target. |
| XML Actions | JavaScript macro modules emit JavaScript, run by QuickJS. In-memory interpretation is not the primary path. |
| Screens | JavaScript macro modules emit server HTML and/or client-renderable JavaScript |
| Render modes | `qvt` is the primary UI (Vue 3.5, Quasar 2.35, no jQuery or Moment). `html` stays as terse server HTML with a small shared script. `html-print` is the print document PDF prints. `text`, `csv`, and `xml` stay as they are. `xsl-fo` is the old name of `html-print`. `vuet` is dropped. |
| Text templates | QuickJS. `JsTemplateLiteral` for `expand()` and short text. Fancy renderer (dialect open; Mustache.js is the candidate) for structured text. Locked-down variant of that renderer for less-trusted text. |
| Security | Moqui artifact authz on Rust primitives. No Shiro. |
| Date, time, and cron | `jiff` 0.2 for timestamps and IANA time zones. Scheduled jobs use `croner` 4 on that type. Details are `plans/Crate-Layout.md`. |

The server JavaScript host method list is in `plans/ContextObjects.md`. The build order is `plans/Implementation-Phases.md`: the entity facade and the service facade are whole, and both are finished before gate 1, a data file load. Screen rendering is gate 3.
