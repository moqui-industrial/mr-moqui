# Modern Rust Moqui Framework - MR Moqui

## Goals

- Keep XML as the spec language for configuration, entities, services, screens, SECAs, and data files, with only the adaptations required by a Rust runtime.
- Move the heavy lifting (entity facade, service facade, screen rendering, tooling, and performance-sensitive service implementations) into Rust.
- Use one language (JavaScript) for inline expressions on both server and client so that XML Screens can become client-renderable.
- Default to an embedded Postgres (`postgresql_embedded`, theseus) when no external database is configured, and use an external Postgres when one is. The same engine backs the sim-mode overlay.
- Make OpenSearch optional from day one by planning a search interface that can be satisfied by Postgres full-text search and JSON operators.
- Preserve the ExecutionContext / facade shape that makes Moqui’s artifact model work.

## AI/LLM Policy

Moqui was originally designed and built largely to reduce the human pain and suffering involved with building and maintaining enterprise applications. The same philosophy applies to the world of AI, from LLMs to harnesses, and from harnesses used to make Moqui to those built into it.

This project is an AI-assisted rebuild of the manually written Moqui Framework in Rust. It would likely never be done without AI assistance migrating and filling in code. AI assistance to migrate existing components made for Moqui is also recommended. Without that being available, migration from Groovy to JavaScript would be a much more expensive thing to do and it is much less likely we'd ever do it.

LLMs are considered tools used by people. The person using the tool is just as responsible as with a text editor and keyboard. LLMs are the functional and moral equivalent of a hammer: they do not shake themselves, and are not a substitute or replacement for a responsible party.

Don't trust, verify. Use AI to help verify, but remember who is responsible, and who is a who.

## Significant redesigns

Most of MR-Moqui is a port of the Java framework as it exists when the port begins. These are the places that change on purpose.

1. **SearchFacade replaces ElasticFacade.** Postgres full-text and JSON are the default backend. OpenSearch is optional. Plan: [`plans/SearchFacade.md`](plans/SearchFacade.md).
2. **qvt becomes server-static.** With `server-static` set for `qvt`, the server sends a data-free Vue template and the client merges data from `/actions` and from a separate form-list request. The attribute stays opt-in for this pass. Plan: [`plans/QvtServerStatic.md`](plans/QvtServerStatic.md).

## Build order

The port is bottom-up. The entity facade and the service facade are finished before the first gate, a data file load. Plan: [`plans/Implementation-Phases.md`](plans/Implementation-Phases.md).
