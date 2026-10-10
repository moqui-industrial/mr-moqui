# MR-Moqui Architecture Proposal

**Modern Rust Moqui** (pronounced “Mister Moqui”)

This document captures the initial architecture direction for a Rust rewrite of Moqui Framework. It is the starting proposal for the new repository.

**STATUS NOTE**: This is a very early proposal, templating approach needs some work for both XML Screen rendering and for XML Action transformation which also needs consideration (interpret with in-memory objects, or transform the whole thing to JS like we do now with Groovy).

## Repository

- New repository: `moqui/mr-moqui`
- Combines what currently lives in `moqui-framework` and `moqui-runtime`. The split has had minimal payoff and high cost; the new repo is the single home for the framework and the default runtime.
- Higher-level components (`mantle-udm`, `mantle-usl`, applications) stay separate and continue to be defined primarily in XML.

## Goals

- Keep XML as the spec language for configuration, entities, services, screens, SECAs, and data files, with only the adaptations required by a Rust runtime.
- Move the heavy lifting (entity facade, service facade, screen rendering, tooling, and performance-sensitive service implementations) into Rust.
- Use one language for inline expressions on both server and client so that XML Screens can become client-renderable.
- Default to an embedded Postgres (PGLite) when no external database is configured, and use an external Postgres when one is. The same engine backs the sim-mode overlay.
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

XML Actions (`entity-find`, `service-call`, `if`, `iterate`, `set`, and the rest) are interpreted natively. They are not compiled to a scripting language. Expressions inside attributes, and `<script>` bodies, are evaluated by the embedded JavaScript engine.

Components remain directories with a `component.xml` descriptor, discovered and loaded at startup. Resource locations keep a scheme-based model (`component://`, and equivalents).

## Scripting Language: JavaScript via rquickjs

JavaScript is the scripting language. The engine is QuickJS-NG embedded through the `rquickjs` crate.

### Why JavaScript

- The same language can run on the server and in the browser. Screen expressions become client-evaluable without a second expression language.
- Modern ECMAScript already covers the features that made Groovy preferable to Java for Moqui expressions: dot access on objects, optional chaining, nullish coalescing, object literals, array indexing, template literals, and array methods.
- Service libraries such as mantle-usl are primarily higher-level business logic. The hot paths and infrastructure stay in Rust. An interpreter without a JIT is acceptable for this workload.

### Why rquickjs / QuickJS-NG

- Small, embeddable, fast startup, and a clean Rust integration (promises ↔ futures, easy registration of host functions, module loaders).
- Supports modern ECMAScript (modules, async, proxies, BigInt, and a large fraction of current language features). It is not required to track the absolute latest spec the week it ships. QuickJS-NG is adequate. TypeScript remains an optional authoring layer (transpiled by SWC or oxc) if longer scripts later want types. It is not required for expressions.
- Heavier options (V8 via `rusty_v8`) were considered and set aside. Binary size, build complexity, and the fact that most script work is business logic rather than tight loops made the lighter engine the better fit. Pure-Rust engines (Boa and others) are not yet the right primary runtime for a large body of existing service code.

### What changes in expressions

Most expressions used in service and screen XML map mechanically:

| Groovy (current) | JavaScript |
|---|---|
| `map.field` / `entityValue.field` | same (objects or a Proxy over EntityValue) |
| `list[0]` | same |
| `obj?.field` | same |
| `a ?: b` | `a ?? b` (see note) |
| `[orderId:orderId, statusId:'X']` | `{orderId, statusId:'X'}` |
| `x in ['A','B']` | `['A','B'].includes(x)` |
| `list*.field` | `list.map(x => x.field)` |
| `"Hello ${name}"` | `` `Hello ${name}` `` |
| ternary `? :` | same |
| `orderItem.'Child#mantle.order.OrderItem'` | `orderItem['Child#mantle.order.OrderItem']` |

Two semantic differences to watch during migration:

- Elvis (`?:`) treats any falsy value as missing. Nullish coalescing (`??`) only substitutes for `null` / `undefined`. Prefer `??`. Use `||` only where the old falsy behavior was intentional.
- Empty arrays are truthy in JavaScript. Conditions that mean “has rows” should test length.

Host types (`EntityValue`, `EntityList`, decimals, timestamps) are exposed into the QuickJS context as objects with the methods scripts actually call. JDK types (`BigDecimal`, `TreeSet`, etc.) are replaced by small host helpers rather than emulated.

### Migration approach

A general Groovy-to-JavaScript transpiler is not planned, and a long-lived compatibility shim for Groovy syntax is not desired. The preferred path is AI-assisted update of expressions and scripts. A temporary, narrowly scoped rewrite of the common attribute-expression idioms above may be used during the transition if it proves helpful, but it should not become something that ships and is relied on in production. The cost of manual migration is no longer a reason to keep risky compatibility layers.

`mantle-udm` (entity definitions and seed data) should load with little or no change once the entity model and data loader exist. `mantle-usl` requires migration of Groovy expressions and `<script>` bodies to JavaScript. The bulk of each service file remains XML Actions and is unaffected.

## Database

- If an external Postgres is configured, use it.
- If not, use PGLite (`pglite-rs`) as the embedded database.
- In either case, use PGLite for the sim-mode overlay database.

### Why PGLite

PGLite (`pglite-rs`) is an in-process Postgres fork. It links into the binary, needs no separate server install, and exposes both a direct Rust API and (on Unix) a socket that ordinary Postgres clients can use. It is the closest equivalent to H2’s “just start” experience while speaking real Postgres.

For sim mode, `PGlite::open_temp()` creates a true in-memory database. Data is not written to disk and disappears when the instance is dropped. Measured footprint is on the order of 23 MB RSS. This replaces the in-memory H2 overlay and can simplify or replace the List/Map-based `TransactionCache` / `TransactionCacheDb` approach when the overlay needs real SQL (views, constraints, JSON). A lighter List/Map cache can still exist for cases that only intercept entity operations.

The side-process alternative (`postgresql_embedded`) was considered. It runs a real postmaster and is operationally similar to the existing OpenSearch sidecar. It does not offer a true in-memory mode (only an ephemeral data directory). Given that sim mode wants an in-memory overlay, and that a single engine for both the default embedded database and the overlay is simpler, PGLite is the choice. External Postgres remains the production path when configured.

### Platform notes

Deployments are overwhelmingly Linux. macOS and Windows matter primarily for development, but builds should work on all three. A future WASM compilation of a subset of the framework is more realistic with an in-process engine than with a spawned postmaster. PGLite’s lineage (the original is a WASM build) fits that direction better than a side-process Postgres.

### Transactions

There is no JTA, no XA, and no Bitronix equivalent. Transactions are local to the single transactional datasource (Postgres). The TransactionFacade preserves the useful service attributes (`ignore`, `force-new`, timeout) using ordinary begin / commit / rollback (and savepoints or a second connection where required). Cross-resource work uses an outbox or SECA rather than two-phase commit.

## Search

OpenSearch-free deployments are a day-one goal. Postgres full-text search and JSON operators cover the common cases that ElasticFacade is used for today (full-text and parameterized document-style queries).

The search interface should be designed so that it is not locked to the current ElasticFacade shape. If a cleaner interface falls out of supporting both a Postgres implementation and an optional OpenSearch implementation, that interface is part of the MR-Moqui migration, not a later retrofit. OpenSearch remains available as an optional backend for deployments that want it. It is no longer required.

## Other Infrastructure

| Area | Direction |
|---|---|
| HTTP server | `axum` + `tower`. WebSocket via `tokio-tungstenite` / axum. HTTP client via `reqwest`. No servlet container, no JNDI. |
| Screens | XML widgets compile to a native widget tree and render through a trait (HTML, CSV, XML, text, …). No FreeMarker. Email and other templates can use a runtime engine such as MiniJinja where a template language is still wanted. |
| Security | Do not port Shiro. Reimplement UserFacade on Rust primitives (`argon2` / `password-hash`, sessions, signed cookies). Artifact authorization stays Moqui’s own model and tables. |
| Cache | `moka` for local cache. Distributed invalidation is a replaceable hook (Redis or similar), not a framework dependency on Hazelcast. |
| Logging | `tracing`. |
| JSON / YAML / XML | `serde_json`, `serde_yaml`, `quick-xml`. |
| CSV, mail, markdown, cron, TOTP | Ordinary crates (`csv`, `lettre`, `pulldown-cmark` / `comrak`, `cron`, `totp-rs`). |
| Content repository (JCR) | Dropped. ResourceFacade plus blob storage (filesystem or S3). |

JDBC drivers, H2, Bitronix, Shiro, FreeMarker, Groovy, and the servlet stack do not carry forward.

## Sim Mode

An ExecutionContext can be placed in sim mode so that nothing permanent is written. Settings on the context control this. The overlay database is an in-memory PGLite instance (`open_temp`). Writes go to the overlay. Reads see the overlay first, then the underlying database. This is the evolution of the TransactionCache / TransactionCacheDb idea. Using a real Postgres for the overlay means view-entities, JSON operators, and constraints behave the same as they do against the primary database, which is the main simplification over a pure List/Map simulation.

## Component and Application Migration

- `mantle-udm`: entity XML and seed/install data. Expected to load as-is once the entity loader and data loader exist. Expression defaults or function expressions, if any, go through the JavaScript evaluator.
- `mantle-usl`: service XML stays. Groovy expressions and scripts are migrated to JavaScript (AI-assisted). XML Actions are unchanged.
- Screens: XML screen definitions stay. Expressions become JavaScript so the same screen can be evaluated on the server or sent to the client for client-side rendering. FreeMarker templates (notifications, print) are a separate, smaller migration.
- Applications built on Mantle follow the same pattern.

## Summary of Decisions

| Topic | Decision |
|---|---|
| Name | MR-Moqui (Modern Rust Moqui, “Mister Moqui”) |
| Repository | `moqui/mr-moqui`, combining framework and runtime |
| Spec language | XML (conf, entities, services, screens, SECAs, data) |
| Scripting | JavaScript via `rquickjs` (QuickJS-NG) |
| Groovy migration | AI-assisted. No long-lived compatibility layer. |
| Embedded database | PGLite (`pglite-rs`) when no external DB is configured |
| Production database | External Postgres when configured |
| Sim overlay | In-memory PGLite (`open_temp`) |
| Search | Postgres FTS + JSON as a first-class, OpenSearch-free path. Interface designed day one, not required to match ElasticFacade. |
| Transactions | Local only. No XA. |
| HTTP | axum |
| Screens | Native widget rendering |
| Security | Moqui artifact authz on Rust primitives. No Shiro. |

This is the initial proposal. It is expected to be refined as the first components (entity loader, JavaScript context bridge, PGLite-backed EntityFacade, and a minimal screen renderer) are built.
