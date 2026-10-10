# MR-Moqui Architecture Proposal

**Modern Rust Moqui** (pronounced “Mister Moqui”)

This document captures the initial architecture direction for a Rust rewrite of Moqui Framework. It is the starting proposal for the new repository.

**STATUS NOTE**: This is a very early proposal. Templating and XML Action handling are specified below: MiniJinja expands XML Actions and screen/form XML into JavaScript, and Liquid renders user-authored text. Screen render modes (what stays, what is dropped, and the html and qvt library plans) are specified in Screen Render Modes. The proposal is expected to be refined as the first components are built.

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

XML Actions (`entity-find`, `service-call`, `if`, `iterate`, `set`, and the rest) and screen/form XML are expanded by MiniJinja macros into JavaScript and run by the embedded engine. An in-memory interpreter remains possible as a later optimization, but it is not the primary path. Expressions inside attributes, and `<script>` bodies, are JavaScript. Details are in Templating.

Components remain directories with a `component.xml` descriptor, discovered and loaded at startup. Resource locations keep a scheme-based model (`component://`, and equivalents).

The framework is synchronous and multi-threaded, as Moqui is today. An `ExecutionContext` belongs to one thread. An HTTP request, an async service call, a scheduled job, and the other background workers each run on a thread with their own context. A call that waits (a database query, outbound HTTP, mail) blocks that thread and returns a value. The framework does not use `Future`, and it does not depend on Tokio.

Worker threads that belong to the framework are ordinary threads started by the factory. The HTTP server lives in the host crate. It accepts connections on Tokio and hands each request to a worker thread that calls the framework and waits for the response. Transaction attributes (`ignore`, `force-new`, timeout) run on the calling thread: begin, commit, rollback, a savepoint, or a second connection. Details of the crate cut are in `plans/Crate-Layout.md`.

`service-call` with `async="true"` starts another thread and another `ExecutionContext`, as it does now. `async="distribute"` (run on any node in a cluster) is dropped. It is rarely used, and there is no cluster transport for it. The service-facade `distributed-factory` goes with it. Distributed cache invalidation stays a later hook.

QuickJS is one runtime per `ExecutionContext`, used only on that context's thread. A function the script calls returns a value before the script continues.

## Scripting Language: JavaScript via rquickjs

JavaScript is the scripting language. The engine is QuickJS-NG embedded through the `rquickjs` crate.

### Why JavaScript

- The same language can run on the server and in the browser. Screen expressions become client-evaluable without a second expression language.
- Modern ECMAScript already covers the features that made Groovy preferable to Java for Moqui expressions: dot access on objects, optional chaining, nullish coalescing, object literals, array indexing, template literals, and array methods.
- Service libraries such as mantle-usl are primarily higher-level business logic. The hot paths and infrastructure stay in Rust. An interpreter without a JIT is acceptable for this workload.

### Why rquickjs / QuickJS-NG

- Small, embeddable, fast startup, and a clean Rust integration. Host functions are synchronous: the script calls, the call blocks, and it returns a value. Module loaders stay available.
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

The QuickJS context mimics the Groovy and FreeMarker context, so expression and script migration stays small. A script starts with the same injected objects the Java runtime puts there: `ec`, `context`, a service `result` where the current actions provide one, and the other objects that entry point injects by default. A screen template sees what the FreeMarker macros see today. The static helpers those scripts and generated actions call are there too, including `ObjectUtilities`, `CollectionUtilities`, and `StringUtilities`.

Host values (`EntityValue`, `EntityList`, decimals, timestamps) expose the methods those scripts call. JDK types such as `BigDecimal` and `TreeSet` are small host helpers with the same call shape. A tighter sandbox on which framework calls a script may make is a later effort.

### Migration approach

A general Groovy-to-JavaScript transpiler is not planned, and a long-lived compatibility shim for Groovy syntax is not desired. The preferred path is AI-assisted update of expressions and scripts. A temporary, narrowly scoped rewrite of the common attribute-expression idioms above may be used during the transition if it proves helpful, but it should not become something that ships and is relied on in production. The cost of manual migration is no longer a reason to keep risky compatibility layers.

`mantle-udm` (entity definitions and seed data) should load with little or no change once the entity model and data loader exist. `mantle-usl` requires migration of Groovy expressions and `<script>` bodies to JavaScript. XML Action elements stay as XML. The macro library that expands them changes from FreeMarker (emitting Groovy) to MiniJinja (emitting JavaScript).

## Templating

FreeMarker currently serves two different purposes. MR-Moqui splits them across two engines.

### Code generation with MiniJinja

XML Actions and screen/form XML are expanded by a library of macros (one per element name) into executable code. Today the target is Groovy. In MR-Moqui the target is JavaScript, run on the embedded QuickJS-NG engine described above.

MiniJinja is the engine for this library. It is a Jinja2-compatible template engine written in Rust, with macros, `{% import %}` / `{% from %}`, inheritance, and blocks. A file or a directory of macros, named to match the XML element names (`entity-find`, `service-call`, `form-single`, `field`, and so on), replaces the current FTL macro set. The generator loads the matching macro, passes the element attributes and already-rendered children, and emits JavaScript.

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

Reusable pieces are snippets rather than FreeMarker-style macros. That is sufficient for this surface and safer. Custom filters provide Moqui-specific formatting, localization, and any limited context access that is explicitly allowed.

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
- If not, use PGLite (`pglite-rs`) as the embedded database (just as Moqui does now with H2; ie this is the default config).
- In either case, use PGLite for the sim-mode overlay database (TransactionCacheDb).

### Why PGLite

PGLite (`pglite-rs`) is an in-process Postgres fork. It links into the binary, needs no separate server install, and exposes both a direct Rust API and (on Unix) a socket that ordinary Postgres clients can use. It is the closest equivalent to H2’s “just start” experience while speaking real Postgres.

For sim mode, `PGlite::open_temp()` creates a true in-memory database. Data is not written to disk and disappears when the instance is dropped. Measured footprint is on the order of 23 MB RSS. PGLite replaces H2 as the engine under the transaction cache and the sim-mode overlay. The behavior of both is a port of the Java framework as it stands when the port begins. The `llm-client` branch is expected to keep changing until then, so this proposal does not freeze those rules.

The side-process alternative (`postgresql_embedded`) was considered. It runs a real postmaster and is operationally similar to the existing OpenSearch sidecar. It does not offer a true in-memory mode (only an ephemeral data directory). Given that sim mode wants an in-memory overlay, and that a single engine for both the default embedded database and the overlay is simpler, PGLite is the choice. External Postgres remains the production path when configured.

### Platform notes

Deployments are overwhelmingly Linux. macOS and Windows matter primarily for development, and builds should work on all three. There is no WASM target. A `wasm32` build would have required the framework to give up threads, and Moqui uses threads for requests, async services, scheduled jobs, and other background work. That model carries forward. PGLite is the in-process engine for the embedded database and the in-memory sim overlay.

Shared embedded databases (the default entity database, and the default search database when it is also embedded) use PGLite's multi-process mode so each thread has its own connection. A sim overlay is a separate in-memory instance. Windows, where that multi-process mode is not available, is covered in `plans/PgliteConcurrency.md`. External Postgres remains the production database when one is configured.

### Transactions

There is no JTA, no XA, and no Bitronix equivalent. Transactions are local to the single transactional datasource. The TransactionFacade preserves the useful service attributes (`ignore`, `force-new`, timeout) using ordinary begin / commit / rollback (and savepoints or a second connection where required). Cross-resource work uses an outbox or SECA rather than two-phase commit.

## Search

OpenSearch-free deployments are a day-one goal. Postgres full-text search and JSON operators cover the common cases that ElasticFacade is used for today (full-text and parameterized document-style queries).

The search interface should be designed so that it is not locked to the current ElasticFacade shape. If a cleaner interface falls out of supporting both a Postgres implementation and an optional OpenSearch implementation, that interface is part of the MR-Moqui migration, not a later retrofit. OpenSearch remains available as an optional backend for deployments that want it. It is no longer required.

For Postgres-backed search, search-specific tables will use the embedded PGLite database by default, and maybe configured to use an external Postgres instance (different config from EntityFacade datasource, may point to same or different pgsql instances).

The interface, the feature set taken from current callers, and the two backends are specified in `plans/SearchFacade.md`.

## Other Infrastructure

| Area | Direction |
|---|---|
| HTTP server | `axum` + `tower` in the host. The framework sees an owned request and blocks until the response is ready. WebSocket via `tokio-tungstenite` / axum. HTTP client via blocking `reqwest`. No servlet container, no JNDI. |
| Screens and XML Actions | MiniJinja macros expand XML to JavaScript. Screen and form XML can also emit server HTML. No FreeMarker, and no native widget-tree renderer as the primary path. See Templating and Screen Render Modes. |
| User-authored templates | Liquid (`liquid` crate) for email, notifications, and custom text/HTML. Sandboxed by design. |
| Security | Do not port Shiro. Reimplement UserFacade on Rust primitives (`argon2` / `password-hash`). The framework owns the session. Artifact authorization stays Moqui’s own model and tables. See Sessions and login. |
| Cache | `moka` for local cache. Distributed invalidation is a replaceable hook (Redis or similar), not a framework dependency on Hazelcast. |
| Logging | `tracing`. |
| JSON / YAML / XML | `serde_json`, `serde_yaml`, `quick-xml`. |
| CSV, mail, markdown, cron, TOTP | Ordinary crates (`csv`, `lettre`, `pulldown-cmark` / `comrak`, `cron`, `totp-rs`). |
| Content repository (JCR) | Dropped. ResourceFacade plus blob storage (filesystem or S3). |

JDBC drivers, H2, Bitronix, Shiro, FreeMarker, Groovy, and the servlet stack do not carry forward. FreeMarker’s two jobs move to MiniJinja (code generation) and Liquid (user-authored text).

## Sessions and login

The servlet session and Shiro do not carry forward. The framework keeps a server-side session of its own. A signed cookie carries the session id. The session holds what the Java session holds: the authenticated user, visit and visitor ids, locale and time zone, the CSRF session token (`moqui.session.token`), flash messages and errors, saved and error parameters, and screen history. The notification WebSocket uses the same session.

Login, logout, and the check that the user has logged out elsewhere live on this session. A successful login replaces the session id. A second factor in progress (email or TOTP) stays on the session until authentication completes. An API key or a login key on the request can authenticate without a password in the session. The long-lived visitor cookie stays separate from the session cookie. Artifact authorization is unchanged and reads the user from this session.

## Tool components and print

`xsl-fo` stays a server render mode. Turning that output into PDF, and the tool components that live outside the framework today, are a later effort. That set includes `moqui-fop`, `moqui-poi`, `moqui-image`, `moqui-cups`, and `moqui-sso`. Some of them may move into the framework. The size of the base `moqui.war` was a reason they stayed outside Moqui Java, and the size of the MR-Moqui binary is the same kind of limit. Until that effort, the host `pdf` module is an empty placeholder and `PdfRenderer` is the trait an engine will implement.

## Screen Render Modes

A screen is requested with a render-mode extension (`.html`, `.qvt`, `.text`, `.csv`, `.xml`, `.xsl-fo`). MiniJinja replaces the FreeMarker macro set for every mode. The modes themselves change as follows. `/qapps` stays the default UI. `/apps` stays the server-rendered HTML UI. `/vapps` goes away.

### Modes that stay

`text`, `csv`, `xml`, and `xsl-fo` stay server-rendered output formats with the same output shape. They load no browser libraries. Scheduled jobs, exports, and print already depend on that output.

### vuet

`vuet` (`/vapps`), and the `.js` and `.vue` companion modes that exist to feed it, are dropped. That UI loads the HTML-mode jQuery stack and then Vue 2 on top of it. qvt is the client-rendered UI.

### html

`html` stays because the document can be small and the feature set is the basic one: forms, form-lists, drop-downs, dates, a tree that navigates on click, background submit into a dynamic container. The current page works against that. Every screen loads `CombinedBase.min.js` (about 205 KB gzip): jQuery, Bootstrap 3, Moment with every locale, Select2, Selectivity, Typeahead, Inputmask, jQuery Validate, the Bootstrap datetimepicker, jsTree, and a set of plugins that are no longer maintained. The macros then append an inline `<script>` for most widgets, and Bootstrap class names (`form-control`, `col-sm-*`, `btn btn-primary`) on every field.

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
| QZ Tray's RSVP dependency in the base bundle | Not loaded. Current QZ Tray uses native promises. See qvt. |

Bootstrap 5, htmx, and Alpine were set aside. Each one puts utility or behavior attributes on the elements, which is the size this mode exists to avoid.

### qvt

`qvt` stays the primary UI, on Vue 3.5 and Quasar 2.35. Delivery stays script tags: `vue.global.prod.js` (the build that includes the template compiler) and `quasar.umd.prod.js`. A tree-shaken bundle does not fit a renderer where the screen XML decides which Quasar components appear. Quasar 2 is a component and prop break from 1.22 (`modelValue` instead of `value` / `input`, and the matching changes in the `m-*` wrappers). It is the same pass as the library removals below.

Moving `qvt` to server-static output, with all data merge done in the client, is a separate redesign. The plan for it is pending. See Significant redesigns in the repository README. The library choices in the rest of this section stand either way.

jQuery and Moment are removed. The widgets are already Quasar: `QSelect` for drop-downs (including remote search), `QInput` with `QDate` and `QTime` for dates, `QInput` `mask` for masks, `QForm` rules for validation, `$q.notify` for toasts, and custom `m-tree-*` components for trees. `text-line[@ac-transition]` is already unsupported in qvt. Authors use a drop-down with server search.

What jQuery still does is helper work, and each call has a direct replacement:

- `$.ajax` becomes `fetch`. Form submit already uses `XMLHttpRequest` for file downloads.
- `$.extend`, `$.each`, and the `$.is*` checks become `Object.assign`, ordinary loops, and the `moqui.is*` helpers.
- The `#conf*` hidden inputs that the root app scrapes with `$().val()` become one JSON config object in the root template (session token, paths, user, locale, dark mode, plugin URLs).
- Focus uses a template ref. The submit button comes from `SubmitEvent.submitter`.
- `m-editable` never actually calls Jeditable. Click-to-edit is `QPopupEdit`.

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
| QZ Tray 2.0.7 | Updated to 2.3 when printing is enabled. 2.3 uses native promises, so RSVP and the standalone SHA-256 script go away. `MoquiQzComponent` is rewritten onto `QDialog` and `QBtn`. The current component is Bootstrap markup inside the Quasar shell. |

## Sim Mode

An ExecutionContext can be placed in sim mode so that nothing permanent is written. Settings on the context control this, as they do in the Java framework. The overlay database is an in-memory PGLite instance (`open_temp`) in place of the in-memory H2 overlay. Transaction-cache and sim-mode behavior, including `enter_sim` and the other `llm-client` work, are ported from the Java code at the time the port begins. How the PGLite engine is shared across threads is `plans/PgliteConcurrency.md`.

## Component and Application Migration

- `mantle-udm`: entity XML and seed/install data. Expected to load as-is once the entity loader and data loader exist. Expression defaults or function expressions, if any, go through the JavaScript evaluator.
- `mantle-usl`: service XML stays. Groovy expressions and scripts are migrated to JavaScript (AI-assisted). XML Actions stay as XML and are expanded to JavaScript by MiniJinja macros.
- Screens: XML screen and form definitions stay. MiniJinja macros expand them. Render modes are in Screen Render Modes. Moving `qvt` to server-static text, with data merge in the client, is a pending redesign (see the README). Expressions are JavaScript, which is what makes that client merge possible. FreeMarker templates for notifications, email, and print move to Liquid.
- Applications built on Mantle follow the same pattern.

## Summary of Decisions

| Topic | Decision |
|---|---|
| Name | MR-Moqui (Modern Rust Moqui, “Mister Moqui”) |
| Repository | `moqui/mr-moqui`, combining framework and runtime |
| Spec language | XML (conf, entities, services, screens, SECAs, data) |
| Scripting | JavaScript via `rquickjs` (QuickJS-NG) |
| Groovy migration | AI-assisted. No long-lived compatibility layer. |
| Embedded database | PGLite (`pglite-rs`) as default configuration |
| Production database | External Postgres, MySQL, etc when configured |
| Sim overlay | In-memory PGLite (`open_temp`) in place of H2. Behavior is a port of the Java framework at the time of the port. |
| Search | Postgres FTS + JSON as a first-class, OpenSearch-free path. Interface designed day one, not required to match ElasticFacade. |
| Transactions | Local only. No XA. |
| HTTP | axum in the host. Framework calls are synchronous. |
| Threads | One `ExecutionContext` per thread. Framework workers are real threads. No `Future` in the framework. |
| Async services | Local thread. `async="distribute"` is dropped. |
| Sessions | Server-side session. A signed cookie carries the id. Same session for HTTP and the notification WebSocket. |
| Print and tool components | Later. `xsl-fo` stays. PDF and the tool components stay outside until binary size is weighed. |
| WASM | Not a target. |
| XML Actions | MiniJinja macros emit JavaScript. In-memory interpretation is not the primary path. |
| Screens | MiniJinja macros emit server HTML and/or client-renderable JavaScript |
| Render modes | `qvt` is the primary UI (Vue 3.5, Quasar 2.35, no jQuery or Moment). `html` stays as terse server HTML with a small shared script. `text`, `csv`, `xml`, and `xsl-fo` stay as they are. `vuet` is dropped. |
| User-authored templates | Liquid (`liquid` crate), sandboxed by design |
| Security | Moqui artifact authz on Rust primitives. No Shiro. |

This is the initial proposal. It is expected to be refined as the first components (entity loader, JavaScript context bridge, PGLite-backed EntityFacade, and a minimal screen renderer) are built.
