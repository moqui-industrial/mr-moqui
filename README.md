# Modern Rust Moqui Framework - MR Moqui

## Goals

- Keep XML as the spec language for configuration, entities, services, screens, SECAs, and data files, with only the adaptations required by a Rust runtime.
- Move the heavy lifting (entity facade, service facade, screen rendering, tooling, and performance-sensitive service implementations) into Rust.
- Use one language (JavaScript) for scripts and templates on the server, and for inline expressions on the client, so that XML Screens can become client-renderable. The engine is QuickJS.
- Default to an embedded Postgres (`postgresql_embedded`, theseus) when no external database is configured, and use an external Postgres when one is. The same engine backs the sim-mode overlay.
- Make OpenSearch optional from day one by planning a search interface that can be satisfied by Postgres full-text search and JSON operators.
- Preserve the ExecutionContext / facade shape that makes Moqui’s artifact model work.

## Significant Redesigns

Most of MR-Moqui is a port of the Java framework as it exists when the port begins. These are the places that change on purpose.

1. **SearchFacade replaces ElasticFacade.** Postgres full-text and JSON are the default backend. OpenSearch is optional. Plan: [`plans/archive/SearchFacade.md`](plans/archive/SearchFacade.md).
2. **qvt becomes server-static.** With `server-static` set for `qvt`, the server sends a data-free Vue template and the client merges data from `/actions` and from a separate form-list request. The attribute stays opt-in for this pass. Plan: [`plans/archive/QvtServerStatic.md`](plans/archive/QvtServerStatic.md).
3. **Feed registration commits with the entity change.** Entity create, update, and delete register document keys during the transaction. After the transaction-cache flush and before commit, on the enlisted connection, the feed inserts a pending row. Rollback drops it. Assembly and the index write run after commit on the service worker pool, for same-database Postgres, a second Postgres, and OpenSearch. An index failure leaves the pending row and does not roll the entity change back. Plan: [`plans/archive/SearchFacade.md`](plans/archive/SearchFacade.md).
4. **JavaScript for scripts and templates.** Scripts, XML action and screen macros, and text templates are JavaScript, initially on QuickJS. `ec.resource.expand()` is a template literal. Structured text uses a second renderer: `if`, `list`, and `assign`, compiled to one cached JavaScript function. A locked-down variant of that renderer is the sandbox for less-trusted templates. Plan: [`plans/archive/Architecture-Proposal.md`](plans/archive/Architecture-Proposal.md) (Templating).
5. **Print HTML is its own render mode.** Server-side page layout is `html-print` (print CSS and `@page`). The old `xsl-fo` name selects that mode. One long-lived external Chromium process prints it to PDF, through `chromiumoxide`. An off-by-default Servo build is the in-process alternate where a child process is forbidden. Plan: [`plans/archive/Architecture-Proposal.md`](plans/archive/Architecture-Proposal.md) (Tool components and print).
6. **Cluster coordination uses the shared database.** Instances find each other in a member table. Cache invalidation and notifications fan out over HTTPS, and that endpoint requires a client certificate whose key material is deployment configuration (member rows hold only key fingerprints). Service jobs stay on the database. There is no Hazelcast, Redis, or session-byte replication. Plan: [`plans/archive/Cluster.md`](plans/archive/Cluster.md).
7. **Sessions live in the database.** The session is `moqui.server.HttpSession`, cached through the entity one-cache and invalidated on the existing cluster topic. A flush overlays dirty fields onto the latest row and retries on `sessionRevision`. The default flush is eventually consistent. Guarantee mode reads the primary and flushes at login, logout, and the end of the request, so a load balancer without stickiness can fail over. Plan: [`plans/archive/Architecture-Proposal.md`](plans/archive/Architecture-Proposal.md) (Sessions and login) and [`plans/archive/Cluster.md`](plans/archive/Cluster.md).
8. **Keys are configuration, and unset secrets are generated.** Secret key values (field encryption, session-cookie signing, the cluster certificate pair, later the SSH host key) are `default-property`s in the `entity_ds_crypt_pass` pattern: a `CHANGEME` value in `MoquiDefaultConf.xml` set by config file or environment, never database rows. A `runtime/.env` file in the root of the runtime directory is loaded at startup like environment variables and supports any conf property. A secret still at its `CHANGEME` default is generated with the OS CSPRNG and saved to `runtime/.env`, instead of running on the shipped default. Plan: [`plans/archive/MimoReview.md`](plans/archive/MimoReview.md) (Decisions 1–3).

## Build, Test, and Run

Very basic instructions from a fresh checkout. A Run & Deploy guide will come later.

Prerequisites: Rust 1.94 or newer (`rustc --version`). The first build downloads the
embedded-Postgres 18.6.0 archive. Node.js is optional; the JS client tests run under
`cargo test` only when `node` is on the path.

```sh
# Development build, binary at target/debug/moqui
cargo build -p moqui

# Production build, binary at target/release/moqui. Strip psql (it links GPL readline)
# from the Postgres archive the binary embeds first, and check the result.
scripts/pg-payload.sh strip
cargo build --release -p moqui
scripts/pg-payload.sh check

# Tests (whole workspace) and lints
cargo test
cargo lint

# The Gate 3 tests, left out of the default run (embedded Postgres per test, Chromium)
cargo test -p moqui-host --test gate3 --test gate3_browser
```

Run from the repo root (paths default to `MOQUI_RUNTIME=runtime`,
`MOQUI_FRAMEWORK=<runtime>/../framework`, embedded cluster in `runtime/db/embedded`).
Setting `entity_ds_db_conf` to `postgres`, `mysql` or `mysql8`, with the other `entity_ds_*`
properties, uses that server instead. The examples are in `runtime/conf/MoquiDevConf.xml`.


```sh
cargo run -p moqui
# or: ./target/debug/moqui
```

With no arguments the binary starts the HTTP server (port `8080` by default, cluster
HTTPS on `8443`) using `framework/conf/MoquiDefaultConf.xml` merged with
`runtime/conf/MoquiDevConf.xml` when present. Optional data load before or after
starting (no arguments loads the default `seed`, `seed-initial`, `install` set):

```sh
./target/debug/moqui load
```

For production packaging, deploy the release binary together with the `framework/`
and `runtime/` directories and run `./target/release/moqui` from the directory that
holds them (or point `MOQUI_RUNTIME` / `MOQUI_FRAMEWORK` at them). The production
settings live in `runtime/conf/MoquiProductionConf.xml`, but right now the server
only auto-loads `MoquiDevConf.xml`, so for a production copy put the production
contents in place of `runtime/conf/MoquiDevConf.xml`.

## Implementation Order

The port is bottom-up. The entity facade and the service facade are finished before the first gate, a data file load. Plan: [`plans/archive/Implementation-Phases.md`](plans/archive/Implementation-Phases.md).

The architecture direction is [`plans/archive/Architecture-Proposal.md`](plans/archive/Architecture-Proposal.md). The crate cut is [`plans/archive/Crate-Layout.md`](plans/archive/Crate-Layout.md). Context objects are [`plans/archive/ContextObjects.md`](plans/archive/ContextObjects.md). Embedded Postgres is [`plans/archive/EmbeddedPostgres.md`](plans/archive/EmbeddedPostgres.md). TypeScript generation from definitions is [`plans/archive/TypeGeneration.md`](plans/archive/TypeGeneration.md). Cluster coordination is [`plans/archive/Cluster.md`](plans/archive/Cluster.md). SSH and SFTP crates are [`plans/archive/Ssh.md`](plans/archive/Ssh.md). The SSO relying party is [`plans/archive/Sso.md`](plans/archive/Sso.md). The architecture and tool-fit review of this plan set, with the deployment-configuration decisions, is [`plans/archive/MimoReview.md`](plans/archive/MimoReview.md).

## AI/LLM Policy

Moqui was originally designed and built largely to reduce the human pain and suffering involved with building and maintaining enterprise applications. The same philosophy applies to the world of AI, from LLMs to harnesses, and from harnesses used to make Moqui to those built into it.

This project is an AI-assisted rebuild of the manually written Moqui Framework in Rust. It would likely never be done without AI assistance migrating and filling in code. AI assistance to migrate existing components made for Moqui is also recommended. Without that being available, migration from Groovy to JavaScript would be a much more expensive thing to do and it is much less likely we'd ever do it.

LLMs are considered tools used by people. The person using the tool is just as responsible as with a text editor and keyboard. LLMs are the functional and moral equivalent of a hammer: they do not shake themselves, and are not a substitute or replacement for a responsible party.

Don't trust, verify. Use AI to help verify, but remember who is responsible, and who is a who.
