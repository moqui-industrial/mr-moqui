# Remaining work

Summary derived from the archived plans on 2026-10-14. Phases 0–12 and gates 1–4 of [Implementation-Phases](archive/Implementation-Phases.md) are built. This file lists what is not. Details stay in the linked archived sections; this file does not replace them.

## Deferred on purpose

- **Phase 13: LLM and A2A.** Ported as the Java framework stands when the phase begins, because the Java side is not yet stable and community feedback iterations are still running. Scope: LLM client, conversation, protocols, tool loop, host routes for LLM endpoints, A2A routes (design in `../moqui/framework/plans/A2A-Server.md`). Until then `LlmServices.xml`, `A2AServices.xml` and the LLM/A2A parts of the tools component stay in the tree, unwired; a call fails with an error naming the service. The LLM client sets the sim-allow exception on `RestClient`. Check: same cases as the Java tests at the time of the port. See [Implementation-Phases, Phase 13](archive/Implementation-Phases.md).
- **Further database drivers** (SQL Server, Oracle, DB2, DB2 for i, Derby, HSQLDB). Build stays on Postgres, MySQL, and SQLite (browser). A missing driver is a configuration error naming the database. Phase 12 in the same file.
- **Assist panel** and the other agent/assist parts: part of this phase.
- **Deliberately not ported:** JSON-RPC, Groovy shell and web eval, Java/Groovy service runners, `vuet`/`/vapps`, `service-group`, `InstanceServices` Docker provisioning, Swagger 2.0/RAML output, `ElasticDatasourceFactory`, Hazelcast/session-byte replication.

## Known open items

| Area | Item | Source |
|---|---|---|
| Wasm browser host | No login flow (calls run with authz disabled except renders); Groovy services skipped (`crates/moqui-browser/src/host.rs`) | [Wasm, Status](archive/Wasm.md) |
| Wasm | Second-tab OPFS failure untested; `/opfs` must not be touched via the Emscripten FS JS API; WebSocket check uses a Node mock of `/notws`, not moqui-host | same |
| Screens | Assist panel (`js/assist`) not ported; it belongs with the Phase 13 assist parts | [Implementation-Phases, Phase 8](archive/Implementation-Phases.md) |
| Mail | Embedded SMTP server: no code; stays disabled behind a feature if ever built | [Crate-Layout](archive/Crate-Layout.md) |
| Parity | Java vs MR-Moqui parity harness and its diff report (a gate artifact) has no code; only search-backend parity tests exist. Java runtime must stay runnable | [Implementation-Phases, Tests](archive/Implementation-Phases.md) |
| Search | Wiki manual data feed stays pending (framework wiki services not ported) | [ConvertProcess](archive/ConvertProcess.md) |

## Checked and done (removed from the list)

- Postgres `NOTIFY` cluster channel: `cluster_transport=notify`, `moqui-host/src/cluster_notify.rs`, with an integration test.
- The russh SSH cluster sender: `moqui-host/src/ssh/cluster.rs`.
- `ScreenServices.xml` bodies: JavaScript or Rust under `ServiceKind::Tools` (Phase 11).
- Static `qvt` template double download: the handler loads the script with the response's ETag/Cache-Control serving the second request, so it is a cache hit, not a second download. Not verified in a browser.
- `lookup` widget as a text line: matches Java, where the macro was commented out. Not a gap.
- `WASM_BIGINT` link flag: no longer present in the repo.

## Mantle conversion follow-up

[ConvertProcess](archive/ConvertProcess.md) left a decision, not more mechanical rewrites: Java collections used as collections in Groovy scripts (`new HashSet`/`TreeSet`/`ArrayList` ~230 sites, `.add(` 366, `.put(` 204, `.contains(` 86, `*.` spreads 174). Either keep plain JavaScript (current ports) or add script globals `HashSet`, `TreeSet`, `ArrayList`, `HashMap` with Java's methods, as [Architecture-Proposal](archive/Architecture-Proposal.md) suggests. After that, 503 service expressions (mostly whole Groovy script blocks) still fail to parse. Application migration of mantle-usl scripts is outside this repository's gates.

## Related active plans
[Future-Plans](Future-Plans.md) (unbuilt proposals) and [Performance-Summary](Performance-Summary.md).
