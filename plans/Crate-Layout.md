# Crate and Source Layout

How the MR-Moqui repository is split into Rust crates and source trees.

This follows `plans/Architecture-Proposal.md`. It replaces the Gradle source sets in `moqui/framework` (`src/main/java`, `src/main/groovy`, `src/start/java`, `src/test/groovy`).

## Decision

Four crates, one workspace, and two data directories.

```
mr-moqui/
  Cargo.toml                 # workspace
  crates/
    moqui-lib/               # no ExecutionContext; usable on its own
    moqui-core/              # the framework; synchronous, multi-threaded
    moqui-host/              # process edge: HTTP server, drivers, process spawn
    moqui/                   # binary: CLI and process entry
  framework/                 # framework component: XML, XSD, templates, default conf
  runtime/                   # instance directory: conf, webroot, tools, logs
```

Dependency direction is one way:

```
moqui (bin) → moqui-host → moqui-core → moqui-lib
```

`moqui-core` is the main framework crate. `moqui-host` is the crate that sits on top of it. `moqui-lib` sits under both and does not know the framework exists.

There is no `src/start` tree and no interface-only tree. Each crate has one `src/` tree. `pub` is the supported API. Extension points that have a second implementation, or that the host must supply, are traits in the same module as the rest of that code.

## Why these cuts and no others

The current trees are language and packaging boundaries:

- `src/main/java` holds the public interfaces plus utilities (`org.moqui.*`, `org.moqui.util`, `org.moqui.resource`, `org.moqui.etl`, `org.moqui.jcache`).
- `src/main/groovy` holds `org.moqui.impl.*`.
- `src/start/java` is `MoquiStart`, a classloader whose job is to run an executable WAR (nested jars, then Jetty).
- `src/test/groovy` is the boot-the-framework suite.

Rust has one language, and dependencies are linked into the binary. Those two source sets do not come forward as directories.

Entity, service, and screen call each other through `ExecutionContext` (entity ECAs call services, services call entities, screens call both). Splitting them into crates makes a cycle, which Cargo will not allow. A traits-only crate plus impl crates would recreate the Java/Groovy split and still leave the cycle. They are modules inside `moqui-core`.

The cuts that are real:

1. Code with no framework type in its API or body can be used without starting Moqui. That is `moqui-lib`.
2. The process edge stays above the framework. That is `moqui-host`: the HTTP server, database driver connections, and process spawn.

Everything else stays in `moqui-core`, including the facades, XML Actions, screens, the entity engine, services, LLM and A2A logic, the JavaScript engine, and the framework's own threads.

## Threads, and why the host is separate

There is no WASM target. `wasm32-wasip1` and a browser build are out of scope. The constraints that target imposed (no threads, and waiting calls as futures) are not worth it. Moqui uses threads in many places: the request, async service calls, scheduled jobs, and other background workers. Those stay threads. The execution model in `plans/Architecture-Proposal.md` is the rule: one `ExecutionContext` per thread, blocking calls. Entity, screen, and HTTP APIs return values. The one wait handle is service-facade `callFuture()`, a oneshot from a worker-pool thread.

`moqui-core` uses `std::thread`. The factory starts the worker threads. "Which jobs are due" and the thread that sleeps and runs them both live in core. An async service call, and `callFuture()`, run on the service worker pool (bounded, with a bounded queue) and get another context. Core may use `std::fs` and `std::time`.

`moqui-core` does not use Tokio, axum, the sync DB clients, or `postgresql_embedded`. Calling async clients from a facade would turn entity finds, service calls, and screen renders into futures, and a QuickJS host callback cannot suspend. The transaction manager and its pool live in core and call `DbDriver`. The host implements that trait with `postgres::Client` for embedded and external Postgres, and with the `mysql` client for MySQL. Each `postgres::Client` owns a `current_thread` runtime and drives it with `block_on` on connect, on every query, and on drop. The host does not own a Tokio handle. Those calls run on factory `std` threads. Building that runtime panics on a thread already inside one, so the embedded-Postgres startup bridge has exited before the pool connects, and the pool is not dropped on the axum runtime. Startup and shutdown go through `postgresql_embedded`. The socket and the settings are `plans/EmbeddedPostgres.md`. The `http` feature on `moqui-lib` links blocking `reqwest`, which starts a runtime of its own. That runtime is not the axum runtime, and core does not call the axum handle for outbound HTTP.

The axum handler accepts the connection on Tokio and hands the request to a thread from the factory's worker pool. It waits for the response. It does not await a framework future, and it does not use Tokio's `spawn_blocking` pool. That pool's cap is independent of the datasource pool.

## moqui-lib

Standalone utilities. No `ExecutionContext`, no facade, no `Moqui` static lookup. A program can depend on this crate and never link the framework.

The one place the current code cheats is `RestClient.refuseIfSim()`, which calls `Moqui.getExecutionContextFactory()`. That check moves to the framework caller (the LLM client and the remote service runners already know sim mode). The client itself stays unaware of the framework. Callers that want a policy pass a predicate into the client. The predicate is not a Moqui type. It default-denies while the context is in sim. Java `RestClient.allowInSim` is the LLM exception: provider calls set it, and the predicate allows that path. Every other caller stays denied.

| Module | Comes from | Notes |
|---|---|---|
| `error` | `BaseException` | Plain error type. Artifact exceptions stay in core. |
| `node` | `MNode` | XML tree, location, merge, and the writer. Parsing is filled in against the corpus. No XML crate. |
| `resource` | `ResourceReference`, file and memory refs | Trait plus file and memory implementations. `component://` and `db://` stay in core. `content://` is an error that names the location. Classpath goes away; the framework component is a directory. |
| `object` | `ObjectUtilities` | Field coercion: decimals, timestamps, booleans, strings. `rust_decimal` and `time` 0.3 live here. `liquid-core` already depends on `time` ^0.3.37. |
| `string` | `StringUtilities` | Encode, decode, pretty-print, masks. Groovy class-name cleanup is not ported. |
| `collection` | `CollectionUtilities` | Map and list helpers the XML and entity code actually use. |
| `context_stack` | `ContextStack` | Stack of maps for variable scope. No engine trait. What each environment sees is `plans/ContextObjects.md`. |
| `env` | `SystemBinding.getPropOrEnv` and `expand` | Property and environment lookup, and `${name}` expansion against those. This is string replacement, not a script engine. Context expansion (`ResourceFacade.expand`) stays in core. |
| `etl` | `SimpleEtl`, `FlatXmlExtractor` | Extract, transform, load pipeline. The entity data loader in core uses it. |
| `topic` | `SimpleTopic` | In-process publish. |
| `cache` | `MCache`, `MCacheManager` | Local cache. The core cache facade uses this directly. No JSR-107 surface. |
| `signer` | `SimpleSigner` | Signing helpers on ordinary crypto crates. |
| `sgml` | `SimpleSgmlReader` | OFX-style SGML, 159 lines, no attributes. Its own function onto `Node`. It is not the framework XML parser. |
| `http` | new | `HttpClient` trait. |
| `rest` | `RestClient` | Blocking client. Feature `http`. |

`node` is the tree the rest of the framework calls. The FreeMarker half of `MNode` is not ported, and neither is the JDK SAX handler. Parsing is whatever the corpus needs, filled in when a file fails to load. The behavior to match, from the handler the corpus already goes through: UTF-8, an optional XML declaration, comments and processing instructions skipped, the element name stored raw (`getNodeNamespace` returns null today), attributes kept as written (`xmlns:xsi` and `xsi:noNamespaceSchemaLocation` are ordinary attributes), no schema fetch, no external entities, no DOCTYPE, character data including CDATA trimmed into the node's text, the five predefined entities and numeric character references expanded, and any other entity an error. The writer emits child text as CDATA. Line and column are tracked. `EntityValue`'s W3C DOM export is not ported. Acceptance is the corpus: framework conf, entity, service, and screen XML, and `mantle-udm` when it is loaded. Nothing else this runtime parses XML (`lettre`, `axum`, `postgres`, `reqwest`, MiniJinja 3 with serde off, `rquickjs`, `rustls`), so there is no XML crate. `sgml` stays its own reader. Gate 3 `xsl-fo` emits the FO document and does not transform it. The deferred XSLT-plus-FOP step would bring its own engine with `PdfRenderer`.

`cache` carries the `MCache` design. Rust is where the same algorithms get faster. The store is a concurrent map of entries. A get checks expiry using the access, creation, and update durations. `get(key, expireBeforeTime)` drops an entry whose last update is older than a known resource change, which is how screens and resources avoid a stale parse. Stats stay: gets, hits, misses, expires, evictions. `maxEntries` is enforced by a scheduled pass (default `maxCheckSeconds` 30), not on put. The pass removes the overflow, ordered by lowest `accessCount`, then oldest `lastAccessTime`. `MCacheManager` is a name-to-cache map, enough to use as a factory. Listeners, entry processors, write-through, and MX beans stay out.

`RestClient` is the proof that lib is useful alone: URI building, headers, JSON, retry, timeout, streaming, multipart. The `http` feature turns on `reqwest` with rustls, using the blocking client. `reqwest::blocking` runs requests on a Tokio runtime that client starts for itself. A call from a framework worker does not use the axum handle. A lib-only program can leave the feature off. `moqui-core` enables it and uses `RestClient` by default. A test, or a deployment with outbound HTTP disabled, passes another `HttpClient`. The sim-mode check stays at the framework caller, as the predicate described above: default-deny, with the LLM provider path allowed.

URL helpers, query-string handling, HMAC, and content-type sniffing from `WebUtilities` live in lib. The servlet containers, Jetty client, and session objects in that class are not ported.

Not ported, on purpose:

- `MClassLoader`. There is no classpath and no nested-jar loader.
- `ContextBinding`. It exists to wrap a Groovy `Binding`. The script context is the context stack plus the QuickJS context in core.
- `LiteStringMap`. Use `indexmap` until a profile says a special map is worth it.
- The JSR-107 surface on top of `MCache` (listeners, entry processors, write-through, MX beans). `MCache` and `MCacheManager` are ported. The cache facade in core uses them directly.

## moqui-core

The framework. One `src/` tree, modules by facade. Public types are `pub`. Helpers are private. There is no `impl` tree.

Built-in behavior is a struct with methods. The Java interfaces on `EntityFind`, `ServiceCallSync`, `ScreenRender`, and the facades existed so a Groovy class could sit behind a Java type. A struct is the API.

A trait is used when conf selects an implementation, or when the implementation cannot live in this crate:

| Trait | Why it is a trait |
|---|---|
| `DbDriver` | Replaces `EntityDatasourceFactory` and the JDBC half of `TransactionInternal`. Embedded Postgres, external Postgres, and MySQL implement it in `moqui-host`. The transaction manager and its pool live in core and call this trait. The methods block and return rows. Later databases use the same trait. |
| `HttpClient` | Defined in lib. Core uses the blocking `RestClient` unless a test or conf supplies another. Outbound HTTP with no client returns a clear error. |
| `ServiceRunner` | Entity-auto, script, and remote runners are built in. Remote runners call `HttpClient`. |
| `SearchBackend` | Postgres SQL for the query and aggregation trees is in core. An OpenSearch mapping that only builds HTTP bodies can live here and use `HttpClient`. |
| `LlmProtocol`, `LlmTool` | More than one protocol and many tools. |
| `PdfRenderer` | Optional, and later. Core emits the XSL-FO document without this trait. A PDF engine implements it, with the tool components. See the architecture proposal. |

The cache facade uses the `MCache` port directly. There is no second cache implementation. Outbound mail uses `lettre` directly. Neither is a trait. Distributed invalidation, when it is added, is a listener on the in-process topic in `moqui-lib`. It is not a Redis client and not a second cache facade. Entity cache, this local cache, and the `screen-server-static` response header stay separate. Inbound SMTP is a listening server and stays a host feature. Where Moqui already has an implementation because the general library was the wrong shape, that implementation is ported. `MNode`, `MCache`, and the transaction manager are the cases. An external crate earns its place when it is the engine (Postgres, QuickJS, MiniJinja, Liquid, axum) or when another crate already needed pulls it in.

QuickJS-NG (`rquickjs`) is the script runner. There is one scripting language, so conf does not select a runner. MiniJinja expands framework XML into JavaScript or HTML. Liquid renders user-authored text and is the sandbox boundary. Those are two engines in `template`, not one trait an operator swaps. Tests call QuickJS, MiniJinja, and Liquid directly. The templating split is `plans/Architecture-Proposal.md`. What each environment sees is `plans/ContextObjects.md`.

`ExecutionContext` and `ExecutionContextFactory` are structs. The host builds the factory and passes the host implementations. `DbDriver` is a map: one entry per entity group, plus the search store when that store is a different database. It is not one trait object. `PdfRenderer` is passed when a renderer exists. Facades are fields. There is no `Moqui.getExecutionContextFactory()` global inside lib or core. The binary may keep a process-wide factory for the CLI. The factory starts the framework's worker threads.

`WebFacade` is a struct filled from an owned request value (method, path, query, headers, body) and it writes an owned response. It does not refer to axum or to a socket. `initWebFacade(HttpServletRequest, HttpServletResponse)` becomes `handle` on the request value. The host translates axum types at the edge.

### Modules

`context` holds the factory and the facades that are not large enough to be their own directory: artifact execution, cache (the facade over `MCache`), l10n, logger (over `tracing`), message, notification, resource (the `component://` scheme and the facade), transaction (the facade, the manager, and the pool), user, web, conf loading, and component discovery.

`script` is the QuickJS runner. `template` is MiniJinja 3.0.0 and Liquid. The values each one sees are `plans/ContextObjects.md`.

`entity` holds definitions, values, the condition tree, find, SQL rendering, the dialect profiles, the `DbDriver` trait, ECAs, the data loader and writer, the entity cache, feeds, and dynamic views. SQL text is core. Opening a connection is host.

`service` holds definitions, the call builders, runners, SECAs, email ECAs, REST API metadata, JSON-RPC dispatch, and job definitions, including the thread that sleeps and runs due jobs.

`screen` holds definitions, forms, trees, URL info, and the MiniJinja expansion to HTML and to client JavaScript.

`search` is the new facade from the architecture proposal, not a port of `ElasticFacade`. The API is `plans/SearchFacade.md`. Postgres query and aggregation SQL is in core. OpenSearch request bodies live here when they are just HTTP. Starting the OpenSearch process does not. When the entity datasource is Postgres, embedded or external, default search tables are the `search` schema on that database. Search is its own embedded cluster when the entity datasource is not Postgres, and an external Postgres when a search URL names one. Embedded search starts on Windows too.

`llm` holds the client, conversation, protocol encoding, tool loop, and `a2a`. Provider calls go through `HttpClient`.

Other current classes, placed by the same rule:

| Current | Where |
|---|---|
| `EdiHandler` | `moqui-core`. It holds an `ExecutionContext`. |
| `RestSchemaUtil` | `moqui-core` (`service`). |
| `UserSecurityChecks` | `moqui-core` (`context` / user). |
| `JdbcExtractor` | The ETL interfaces stay in lib. JDBC extraction is host, next to `DbDriver`. |
| `ElasticSearchLogger` | Host, beside the OpenSearch process. |
| `ElFinderConnector` | Host, if the file API stays. It is an HTTP endpoint. |
| `MoquiShiroRealm`, `MoquiWebSecurityManager` | Not ported. User auth is the core user facade on `argon2` 0.6 with the `password-hash` feature. |
| `TransactionInternalBitronix` | Moqui's own transaction manager in core, against `DbDriver`, with this class's behavior as the reference: local one-phase commit, the pool, synchronizations, and suspend that parks the enlisted client on the transaction. The XA journal, crash recovery, and JMS are not ported. |
| `H2ServerToolFactory`, `JackrabbitRunToolFactory`, `JCSCacheToolFactory` | Not ported. `content://` resolves as an error. |
| `MCacheToolFactory` | The cache facade on the `MCache` port. |
| `SubEthaSmtpToolFactory` | Host, behind a feature, if inbound SMTP stays. |
| Servlet and filter classes in `impl/webapp` | Host HTTP layer. The behavior they delegate (screen render, auth, REST, LLM, A2A, notifications) is core. |
| `src/main/resources/org/moqui/impl/*.groovy` (send and poll email) | Core service logic. Outbound mail uses `lettre` in core. The Groovy scripts are not kept as scripts. |

`rquickjs` links QuickJS-NG, which is C. It belongs in core because XML Actions and expressions are the framework. Each `ExecutionContext` owns the runtime for its thread. Functions called from JavaScript are synchronous and return values. Do not create a `moqui-js` crate.

Core can load components, parse XML, expand actions and screens, run JavaScript, evaluate entity conditions, and render SQL with no database driver and no listen socket. Opening a connection, accepting a request, and spawning an external process are host. That is what keeps a core unit test free of those.

## Databases

The entity engine is database-agnostic in the same way the Java framework is. `moqui-core` owns the dialect and the SQL. `moqui-host` owns connections. Adding a database is a dialect profile plus a driver. It is not a fork of the entity facade, and it is not a new crate.

The dialect profile is the `<database>` block Moqui already has in `MoquiDefaultConf.xml`: SQL type names, join style, offset style, lateral style, full-text style, null handling, and the related flags. The SQL renderer in core takes a dialect. It does not import a driver. Core renders SQL text at runtime, as it is now. There is no SQLx dependency and no `query!` compile-time macros.

`DbDriver` is the execution boundary: connect, checkout, begin, commit, rollback, savepoint, execute, query, and release. `query` returns rows; a streaming arm serves `entity.find.iterator`, the ETL JDBC extract, entity sync, and large exports (Postgres named cursor inside the transaction, MySQL unbuffered). An open stream pins its connection until it closes, and outside a transaction that pins a pooled checkout. The pool in core checks a connection out and enlists one for the life of the transaction. A call outside a transaction checks out and returns. `suspend` parks that client on the transaction, off the free list and off the thread. `resume` checks the same client out again. `force-new` is that suspend followed by begin, so the thread holds one connection. The parked client still counts against the pool max. Checkout is `plans/EmbeddedPostgres.md`. The transaction rules are the Transactions section of `plans/Architecture-Proposal.md`. Rows come back as the plain values `moqui-lib`'s `object` module already uses (`rust_decimal` and `time`), not JSON. `postgres` decodes timestamps with `with-time-0_3`, and `mysql` with its `time` feature. The `with-chrono-0_4` and `mysql` `chrono` features stay off. The driver does not know entity definitions. The datasource conf selects the driver at runtime. The factory holds the group map described above.

Search stays the path from the architecture proposal. When the entity datasource is Postgres, embedded or external, default search tables are the `search` schema on that database. A search URL is how search points at a different Postgres. When the entity datasource is not Postgres, search is its own embedded cluster. Naming an external database for a store does not also start an embedded postmaster for that store.

Initial drivers, all in `moqui-host` and all enabled in the default binary:

| Driver | Role | Dialect profile |
|---|---|---|
| Embedded Postgres (`postgresql_embedded` 0.21) | Default entity cluster when no external database is configured. Lifecycle only: start and stop, with `bundled`, `blocking`, `theseus`, and `tls-rustls-aws-lc-rs` (default features off), and `POSTGRESQL_VERSION` pinned to 18.6.0. Per-query traffic is `postgres::Client` on the Unix socket with `NoTls`. The pool is the core transaction manager. Embedded search is the `search` schema on that cluster. The sim overlay is a second cluster, one schema per `TransactionCacheDb`. See `plans/EmbeddedPostgres.md`. | `postgres` |
| Sync Postgres (`postgres` 0.19) | External Postgres, same client and same core pool | `postgres` |
| Sync MySQL (`mysql` 28) | External MySQL and MariaDB. The core pool checks connections out. `mysql::Pool` is not the pooling design. | `mysql8`, and the older `mysql` profile |

Each `postgres::Client` owns a `current_thread` Tokio runtime and calls `block_on` on it for connect, query, and drop. The host does not own that runtime. `DbDriver` methods run only on factory worker threads, never on an axum task. Pool sizes and timeouts start at the Java datasource values. Memory and speed testing follow. MySQL is a first target alongside Postgres, not a follow-up milestone. A prepared-statement cache waits on a profile. `postgres::Client` already prepares statements.

Embedded Postgres socket traffic uses `NoTls`. External Postgres TLS is `tokio-postgres-rustls` (`MakeRustlsConnect`) with its `aws-lc-rs` feature. External MySQL TLS is the `mysql` feature `rustls-tls` (aws-lc-rs). The `native-tls` feature stays off. On `mysql` 28 that feature is opt-in, not part of the default set. That is the same rustls provider family as `reqwest`'s `rustls-tls` feature. Database traffic does not also link native-tls or OpenSSL. `postgresql_embedded` uses `tls-rustls-aws-lc-rs` for the same reason, and its `native-tls` feature stays off.

The Java conf also defines databases to add later, still in `moqui-host`'s `db` module, as another dialect plus a `DbDriver`. Each one needs a synchronous `DbDriver` implementation; there is no SQLx dependency. SQLite is not a Moqui database and is not a target. Of the remaining Moqui databases, SQL Server, Oracle, DB2, and the rest each wait on a sync driver that implements the same trait. H2 is not on the list. Embedded Postgres replaces it. OrientDB appears only as a comment in the Java conf.

| Conf name | Database | When |
|---|---|---|
| `mssql` | SQL Server | Later, via a sync `DbDriver` |
| `oracle` | Oracle | Later, via a sync `DbDriver` |
| `db2` | DB2 | Later, same |
| `db2i` | DB2 for i | Later, same |
| `derby` | Derby | Later, same |
| `hsql` | HSQLDB | Later, same |

Cargo features may trim a driver out of a custom build. The default feature set includes embedded Postgres, Postgres, and MySQL. A MySQL entity database still needs Postgres or OpenSearch for search, and sim still needs the overlay cluster, so the default binary includes `postgresql_embedded`. A custom build may omit it only when search is an external Postgres or OpenSearch and the deployment does not use sim. The binary-size note in `plans/Architecture-Proposal.md` is the same limit.

## moqui-host

The process edge. Depends on `moqui-core` and on `moqui-lib` with the `http` feature. This is the only crate that may use axum, Tokio, the sync DB clients (`postgres`, `mysql`), `postgresql_embedded`, and process spawn. `lettre` lives in core. `MCache` lives in lib. There is no `moka`, no `r2d2`, and no XML crate.

| Module | Role |
|---|---|
| `http` | axum routes and WebSocket upgrade. `extract::ws` needs axum's `ws` feature, which is not in the default set. Builds the core request value, calls core on a worker thread, writes the response. Replaces `MoquiServlet`, the auth filter, the LLM and A2A servlets, FOP servlet entry, and the notification endpoint. |
| `db` | `DbDriver` implementations: embedded Postgres lifecycle, external Postgres, and external MySQL. The pool is the core transaction manager. Each method runs on the checked-out client and returns rows. Later databases from the table above land here. Embedded Postgres concurrency is this module, as in `plans/EmbeddedPostgres.md`. |
| `mail` | Inbound SMTP, behind a feature, if it stays. Outbound mail is core. |
| `search` | OpenSearch process supervisor. Query construction stays in core. |
| `pdf` | Empty placeholder. PDF rendering is a later effort, along with `moqui-fop` and the other tool components. |

Features on this crate, not more crates: `opensearch`, `inbound-smtp`, `pdf`. Default features are embedded Postgres, external Postgres, MySQL, and axum.

A `moqui-db` crate split out of host is a later option, and only if something needs the drivers without HTTP. It is not part of this layout. The binary always wants the HTTP server and the initial drivers together.

## moqui binary

`crates/moqui` is a binary crate. It replaces `MoquiStart` and the `main` path on `org.moqui.Moqui`.

`MoquiStart` does three things: load jars from inside a WAR, parse CLI arguments, and start embedded Jetty. The first has no Rust equivalent. The second and third are `main`.

Commands match the current CLI shape: run the server, and `load` with the same flags the current `Moqui.loadData` accepts (`types`, `components`, `location`, `timeout`, `raw`, and the disable flags). The binary constructs the host stack, builds an `ExecutionContextFactory`, and either serves or loads.

The binary stays thin. Framework behavior does not accumulate here. Extra in-process service functions that must be Rust (there is no Java service runner) are registered from the binary or from host into a core registry.

Dev layout is a checkout plus `framework/` and `runtime/` on disk, found from the working directory, which is how Moqui finds its runtime today. Embedding those directories in the binary is an optional later feature. It is not a source set.

## Data directories

These are not crates.

`framework/` is the framework component. It absorbs what is now on the framework classpath and what is framework-owned template source:

- `component.xml`
- `data/`, `entity/`, `service/`, `screen/`
- `xsd/`
- `template/` for the MiniJinja macro libraries (XML Actions, screens, forms) and the Liquid snippets that replace framework FreeMarker
- default `conf/MoquiDefaultConf.xml`

`runtime/` is the instance directory, as it is today:

- `conf/` for `MoquiDevConf.xml`, production conf, and local overrides
- `base-component/webroot/` for the browser assets (Vue, Quasar, `moqui-html.js`)
- `base-component/tools/` for the tools component XML
- `component/` for add-ons such as mantle, which stay outside this repository
- log, sessions, and other runtime state, gitignored as they are now

Screen macros that are framework code live under `framework/template/`. The runtime can still override a template by location. Instance conf stays in `runtime/conf/`.

## Tests

Unit tests live next to the code in the crate that owns it, so lib tests do not boot the framework and core tests do not open a socket.

Tests that start a factory, load `framework/`, and hit HTTP live under `crates/moqui/tests` (or `crates/moqui-host/tests` when they do not need the CLI). That replaces `src/test/groovy`.

## What this does not add

- A `moqui-api` crate, or any other interface-only crate.
- Per-facade crates (`moqui-entity`, `moqui-service`, `moqui-screen`).
- A `start` or `execWar` source tree.
- A port of the JSR-107 cache API, Shiro, the Bitronix XA journal, Jackrabbit, H2, JCS, or the servlet types. `MCache` is ported. The transaction manager is Moqui's own, against `DbDriver`, with Bitronix's behavior as the reference.
- A WASM target, a `moqui-js` crate, a `moqui-db` crate, or a crate per database.
- The tool components (`moqui-fop`, `moqui-poi`, `moqui-image`, `moqui-cups`, `moqui-sso`, `moqui-sftp`). Whether any of them move into this repository is a later decision, limited by binary size.
