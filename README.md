# Modern Rust Moqui Framework - MR Moqui

## Goals

- Keep XML as the spec language for configuration, entities, services, screens, SECAs, and data files, with only the adaptations required by a Rust runtime.
- Move the heavy lifting (entity facade, service facade, screen rendering, tooling, and performance-sensitive service implementations) into Rust.
- Use one language (JavaScript) for scripts and templates on the server, and for inline expressions on the client, so that XML Screens can become client-renderable. The engine is QuickJS.
- Default to an embedded Postgres (`postgresql_embedded`, theseus) when no external database is configured, and use an external Postgres when one is. The same engine backs the sim-mode overlay.
- Make OpenSearch optional from day one by planning a search interface that can be satisfied by Postgres full-text search and JSON operators.
- Preserve the ExecutionContext / facade shape that makes Moqui’s artifact model work.

## Significant redesigns

Most of MR-Moqui is a port of the Java framework as it exists when the port begins. These are the places that change on purpose.

1. **SearchFacade replaces ElasticFacade.** Postgres full-text and JSON are the default backend. OpenSearch is optional. Plan: [`plans/SearchFacade.md`](plans/SearchFacade.md).
2. **qvt becomes server-static.** With `server-static` set for `qvt`, the server sends a data-free Vue template and the client merges data from `/actions` and from a separate form-list request. The attribute stays opt-in for this pass. Plan: [`plans/QvtServerStatic.md`](plans/QvtServerStatic.md).
3. **Same-database data feed commits with the entity change.** When search tables are on the entity database, the feed writes after the transaction-cache flush and before commit, on the enlisted connection. In Java, `DataFeedSynchronization.beforeCompletion()` is empty and the feed runs in `afterCompletion` on the worker pool, only after the commit. A different database or OpenSearch stays after commit. Plan: [`plans/SearchFacade.md`](plans/SearchFacade.md).
4. **JavaScript for scripts and templates.** Scripts, XML action and screen macros, and text templates are JavaScript, initially on QuickJS. `ec.resource.expand()` is a template literal. Structured text uses a second renderer whose dialect is still open, and a locked-down variant of that renderer is the sandbox for less-trusted templates. Plan: [`plans/Architecture-Proposal.md`](plans/Architecture-Proposal.md) (Templating).
5. **Print HTML is its own render mode.** Server-side page layout is `html-print` (print CSS and `@page`). The old `xsl-fo` name selects that mode. One long-lived external Chromium process prints it to PDF, through `chromiumoxide`. An off-by-default Servo build is the in-process alternate where a child process is forbidden. Plan: [`plans/Architecture-Proposal.md`](plans/Architecture-Proposal.md) (Tool components and print).
6. **Cluster coordination uses the shared database.** Instances find each other in a member table. Cache invalidation and notifications fan out over HTTPS, and that endpoint requires a client certificate stored in the database. Service jobs stay on the database. There is no Hazelcast, Redis, or session-byte replication. Plan: [`plans/Cluster.md`](plans/Cluster.md).
7. **Sessions live in the database.** The snapshot is a row from the first session implementation, not a file under `runtime/sessions` and not a replicated in-memory session. The default flush is eventually consistent. An optional mode commits on each change or at login, logout, and the end of the request, so a load balancer without stickiness can fail over. Plan: [`plans/Architecture-Proposal.md`](plans/Architecture-Proposal.md) (Sessions and login) and [`plans/Cluster.md`](plans/Cluster.md).

## Build order

The port is bottom-up. The entity facade and the service facade are finished before the first gate, a data file load. Plan: [`plans/Implementation-Phases.md`](plans/Implementation-Phases.md).

The architecture direction is [`plans/Architecture-Proposal.md`](plans/Architecture-Proposal.md). The crate cut is [`plans/Crate-Layout.md`](plans/Crate-Layout.md). Context objects are [`plans/ContextObjects.md`](plans/ContextObjects.md). Embedded Postgres is [`plans/EmbeddedPostgres.md`](plans/EmbeddedPostgres.md). TypeScript generation from definitions is [`plans/TypeGeneration.md`](plans/TypeGeneration.md). Cluster coordination is [`plans/Cluster.md`](plans/Cluster.md). SSH and SFTP crates are [`plans/Ssh.md`](plans/Ssh.md).

## AI/LLM Policy

Moqui was originally designed and built largely to reduce the human pain and suffering involved with building and maintaining enterprise applications. The same philosophy applies to the world of AI, from LLMs to harnesses, and from harnesses used to make Moqui to those built into it.

This project is an AI-assisted rebuild of the manually written Moqui Framework in Rust. It would likely never be done without AI assistance migrating and filling in code. AI assistance to migrate existing components made for Moqui is also recommended. Without that being available, migration from Groovy to JavaScript would be a much more expensive thing to do and it is much less likely we'd ever do it.

LLMs are considered tools used by people. The person using the tool is just as responsible as with a text editor and keyboard. LLMs are the functional and moral equivalent of a hammer: they do not shake themselves, and are not a substitute or replacement for a responsible party.

Don't trust, verify. Use AI to help verify, but remember who is responsible, and who is a who.
