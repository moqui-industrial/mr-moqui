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
    moqui-core/              # the framework; WASM-compatible
    moqui-host/              # depends on moqui-core; native-only
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
2. Code that cannot be compiled for the WASM target has to live above the framework. That is `moqui-host`.

Everything else stays in `moqui-core`, including the facades, XML Actions, screens, the entity engine, services, LLM and A2A logic, and the JavaScript engine.

## WASM target

`moqui-lib` and `moqui-core` compile for `wasm32-wasip1`. That target has a filesystem and clocks, and it does not have a listen socket, a process spawn, or a multi-thread runtime. CI is `cargo check --target wasm32-wasip1 -p moqui-lib -p moqui-core` (lib with default features off; see HTTP below). Tests run on the native target.

`wasm32-unknown-unknown` (the browser) is a later port. It needs a virtual filesystem in place of `std::fs`. Do not add crates for it now. Core may use `std::fs` and `std::time`.

`moqui-core` does not depend on Tokio. Functions that wait return a `Future`. The host runtime polls them. Core does not call `tokio::spawn` or `tokio::time::sleep`. "What is due" for a cron job is a pure calculation in core. The thread that sleeps and fires is host.

## moqui-lib

Standalone utilities. No `ExecutionContext`, no facade, no `Moqui` static lookup. A program can depend on this crate and never link the framework.

The one place the current code cheats is `RestClient.refuseIfSim()`, which calls `Moqui.getExecutionContextFactory()`. That check moves to the framework caller (the LLM client and the remote service runners already know sim mode). The client itself stays unaware of the framework. Callers that want a policy can pass a predicate into the client. The predicate is not a Moqui type.

| Module | Comes from | Notes |
|---|---|---|
| `error` | `BaseException` | Plain error type. Artifact exceptions stay in core. |
| `node` | `MNode` | XML tree, location info, parse and write. FreeMarker `TemplateModel` adapters are not ported. |
| `resource` | `ResourceReference`, file and memory refs | Trait plus file and memory implementations. `component://` and `db://` stay in core. Classpath goes away; the framework component is a directory. |
| `object` | `ObjectUtilities` | Field coercion: decimals, timestamps, booleans, strings. `rust_decimal` and a date-time crate live here. |
| `string` | `StringUtilities` | Encode, decode, pretty-print, masks. Groovy class-name cleanup is not ported. |
| `collection` | `CollectionUtilities` | Map and list helpers the XML and entity code actually use. |
| `context_stack` | `ContextStack` | Stack of maps for variable scope. |
| `env` | `SystemBinding.getPropOrEnv` and `expand` | Property and environment lookup, and `${name}` expansion against those. This is string replacement, not a script engine. Context expansion (`ResourceFacade.expand`) stays in core. |
| `etl` | `SimpleEtl`, `FlatXmlExtractor` | Extract, transform, load pipeline. The entity data loader in core uses it. |
| `topic` | `SimpleTopic` | In-process publish. |
| `signer` | `SimpleSigner` | Signing helpers on ordinary crypto crates. |
| `sgml` | `SimpleSgmlReader` | Small reader, depends only on string helpers. |
| `http` | new | `HttpClient` trait. Always compiled, including for WASM. |
| `rest` | `RestClient` | Concrete client. Feature `http` only. |

`RestClient` is the proof that lib is useful alone: URI building, headers, JSON, retry, timeout, streaming, multipart. On native builds the feature `http` turns on `reqwest` with rustls. That stack does not compile for `wasm32-wasip1` (no sockets), so the feature is native-only. `moqui-core` depends on `moqui-lib` without `http` and talks to the `HttpClient` trait. `moqui-host` enables `http` and passes a `RestClient` into the factory. Core tests use a fake client.

URL helpers, query-string handling, HMAC, and content-type sniffing from `WebUtilities` live in lib. The servlet containers, Jetty client, and session objects in that class are not ported.

Not ported, on purpose:

- `MClassLoader`. There is no classpath and no nested-jar loader.
- `ContextBinding`. It exists to wrap a Groovy `Binding`. The script context is the context stack plus the QuickJS context in core.
- `LiteStringMap`. Use `indexmap` until a profile says a special map is worth it.
- `jcache` (`MCache`, `MCacheManager`, and the JSR-107 types). The cache facade in core replaces them.

## moqui-core

The framework. One `src/` tree, modules by facade. Public types are `pub`. Helpers are private. There is no `impl` tree.

Built-in behavior is a struct with methods. The Java interfaces on `EntityFind`, `ServiceCallSync`, `ScreenRender`, and the facades existed so a Groovy class could sit behind a Java type. A struct is the API.

A trait is used when conf selects an implementation, or when the implementation cannot live in this crate:

| Trait | Why it is a trait |
|---|---|
| `DbDriver` | Replaces `EntityDatasourceFactory` and the JDBC half of `TransactionInternal`. PGLite, external Postgres, and MySQL implement it in `moqui-host`. Later databases use the same trait. |
| `CacheStore` | Core ships a `Mutex` + map store so the crate runs alone. Host may substitute `moka`. |
| `HttpClient` | Defined in lib. Core holds `Arc<dyn HttpClient>` injected at startup. Absent client: outbound HTTP returns a clear error. |
| `ScriptRunner` | Conf can name a runner. The built-in one is QuickJS-NG via `rquickjs`. |
| `TemplateRenderer` | Conf can name a renderer. Built-ins are MiniJinja (code generation) and Liquid (user text). |
| `ServiceRunner` | Entity-auto, script, and remote runners are built in. Remote runners call `HttpClient`. |
| `Mailer` | Sending mail needs a socket. Host implements it. Message building stays here. |
| `SearchBackend` | Postgres full-text SQL is in core. An OpenSearch mapping that only builds HTTP bodies can live here and use `HttpClient`. |
| `LlmProtocol`, `LlmTool` | More than one protocol and many tools. |
| `PdfRenderer` | Optional. XSL-FO output is produced here. A PDF engine, when one exists, is host. |

`ExecutionContext` and `ExecutionContextFactory` are structs. The factory is built by the host and given the trait objects above. Facades are fields. There is no `Moqui.getExecutionContextFactory()` global inside lib or core. The binary may keep a process-wide factory for the CLI.

`WebFacade` is a struct filled from an owned request value (method, path, query, headers, body) and it writes an owned response. It does not refer to axum or to a socket. `initWebFacade(HttpServletRequest, HttpServletResponse)` becomes `handle` on the request value. The host translates axum types at the edge.

### Modules

`context` holds the factory and the facades that are not large enough to be their own directory: artifact execution, cache, l10n, logger (over `tracing`), message, notification, resource (the `component://` scheme and the facade), transaction, user, web, conf loading, and component discovery.

`script` is the QuickJS runner. `template` is MiniJinja and Liquid.

`entity` holds definitions, values, the condition tree, find, SQL rendering, the dialect profiles, the `DbDriver` trait, ECAs, the data loader and writer, the entity cache, feeds, and dynamic views. SQL text is core. Opening a connection is host.

`service` holds definitions, the call builders, runners, SECAs, email ECAs, REST API metadata, JSON-RPC dispatch, and job definitions including "which jobs are due".

`screen` holds definitions, forms, trees, URL info, and the MiniJinja expansion to HTML and to client JavaScript.

`search` is the new facade from the architecture proposal, not a port of `ElasticFacade`. The API is `plans/SearchFacade.md`. Postgres query and aggregation SQL is in core. OpenSearch request bodies live here when they are just HTTP. Starting the OpenSearch process does not. Search tables stay on PGLite or an external Postgres, including when the entity datasource is MySQL.

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
| `MoquiShiroRealm`, `MoquiWebSecurityManager` | Not ported. User auth is the core user facade on `argon2`. |
| `TransactionInternalBitronix` | Not ported. Transactions are the driver connection. |
| `H2ServerToolFactory`, `JackrabbitRunToolFactory`, `JCSCacheToolFactory` | Not ported. |
| `MCacheToolFactory` | Replaced by `CacheStore`. |
| `SubEthaSmtpToolFactory` | Host, behind a feature, if inbound SMTP stays. |
| Servlet and filter classes in `impl/webapp` | Host HTTP layer. The behavior they delegate (screen render, auth, REST, LLM, A2A, notifications) is core. |
| `src/main/resources/org/moqui/impl/*.groovy` (send and poll email) | Core service logic plus the `Mailer` trait. The Groovy scripts are not kept as scripts. |

`rquickjs` links QuickJS-NG, which is C. It belongs in core because XML Actions and expressions are the framework. If the WASI build of that crate fails, gate it with a default-on feature `js` and fix the build. Do not create a `moqui-js` crate to get past the failure.

A WASM build has no database driver and no listen socket. It can still load components from the WASI filesystem, parse XML, expand actions and screens, run JavaScript, evaluate entity conditions, and render SQL text. The host supplies drivers when they exist.

## Databases

The entity engine is database-agnostic in the same way the Java framework is. `moqui-core` owns the dialect and the SQL. `moqui-host` owns connections. Adding a database is a dialect profile plus a driver. It is not a fork of the entity facade, and it is not a new crate.

The dialect profile is the `<database>` block Moqui already has in `MoquiDefaultConf.xml`: SQL type names, join style, offset style, lateral style, full-text style, null handling, and the related flags. The SQL renderer in core takes a dialect. It does not import a driver. Core does not use SQLx's compile-time query macros. SQL is produced from entity XML at runtime, as it is now.

`DbDriver` is the execution boundary: connect, begin, commit, rollback, savepoint, execute, query. Rows come back as plain values. The driver does not know entity definitions. The datasource conf selects the driver at runtime.

Search stays the path from the architecture proposal. Search tables use embedded PGLite by default, or an external Postgres when configured. That is independent of which database holds entity data.

Initial drivers, all in `moqui-host` and all enabled in the default binary:

| Driver | Role | Dialect profile |
|---|---|---|
| PGLite (`pglite-rs`) | Default when no external database is configured, and the sim-mode overlay (`open_temp`) | `postgres` |
| SQLx Postgres | External Postgres | `postgres` |
| SQLx MySQL | External MySQL and MariaDB | `mysql8`, and the older `mysql` profile |

PGLite is in-process, so it is not a SQLx driver. External Postgres and MySQL are SQLx. Both are first targets. MySQL is not a follow-up milestone.

The Java conf also defines databases to add later, still in `moqui-host`'s `db` module, as another dialect plus a `DbDriver`. The intended driver path is SQLx. SQLx 0.9 maintains Postgres, MySQL/MariaDB, and SQLite. SQLite is not a Moqui database and is not a target. Of the remaining Moqui databases, only SQL Server has a SQLx story, and that built-in driver was removed in 0.7 pending a rewrite. The others wait on a SQLx driver, or on another driver that implements the same trait. H2 is not on the list. PGLite replaces it. OrientDB appears only as a comment in the Java conf.

| Conf name | Database | When |
|---|---|---|
| `mssql` | SQL Server | Later, via SQLx once that driver is usable |
| `oracle` | Oracle | Later, via a `DbDriver` (no SQLx driver today) |
| `db2` | DB2 | Later, same |
| `db2i` | DB2 for i | Later, same |
| `derby` | Derby | Later, same |
| `hsql` | HSQLDB | Later, same |

Cargo features may trim a driver out of a custom build. The default feature set includes PGLite, Postgres, and MySQL.

## moqui-host

Native-only. Depends on `moqui-core` and on `moqui-lib` with the `http` feature. This is the only crate that may use axum, Tokio, SQLx, `pglite-rs`, `lettre`, `moka`, and process spawn.

| Module | Role |
|---|---|
| `http` | axum routes and WebSocket upgrade. Builds the core request value, calls core, writes the response. Replaces `MoquiServlet`, the auth filter, the LLM and A2A servlets, FOP servlet entry, and the notification endpoint. |
| `db` | `DbDriver` implementations: PGLite, SQLx Postgres, and SQLx MySQL. Later databases from the table above land here. |
| `cache` | `moka` as a `CacheStore`. |
| `schedule` | Tokio interval that calls the core "run due jobs" entry. |
| `mail` | `lettre` implementation of `Mailer`. Inbound SMTP is a feature. |
| `search` | OpenSearch process supervisor. Query construction stays in core. |
| `pdf` | Placeholder for an XSL-FO to PDF tool. Empty until a renderer is chosen. |

Features on this crate, not more crates: `opensearch`, `inbound-smtp`, `pdf`. Default features are PGLite, external Postgres, MySQL, and axum.

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

WASM CI checks that lib and core compile. It does not run the suite under WASI until a test harness is worth it.

## What this does not add

- A `moqui-api` crate, or any other interface-only crate.
- Per-facade crates (`moqui-entity`, `moqui-service`, `moqui-screen`).
- A `start` or `execWar` source tree.
- A port of the JSR-107 cache API, Shiro, Bitronix, Jackrabbit, H2, JCS, or the servlet types.
- A browser-WASM crate, a `moqui-js` crate, a `moqui-db` crate, or a crate per database.
