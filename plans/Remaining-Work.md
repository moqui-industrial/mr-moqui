# Remaining work

Summary derived from the archived plans on 2026-10-14, extended 2026-10-16 with the parity and code quality review leftovers, and trimmed to open items on 2026-10-18. Phases 0–12 and gates 1–4 of [Implementation-Phases](archive/Implementation-Phases.md) are built. This file lists what is not. Details stay in the linked archived sections; this file does not replace them.

What is not ported on purpose (JSON-RPC, the Groovy and Java runners, the database drivers not in this tree, `service-group`, Swagger 2.0 and RAML, Hazelcast, the embedded SMTP server, vuet and `/vapps`) is in [Parity-Mapping, Excluded on purpose](Parity-Mapping.md#excluded-on-purpose); the non-SQL entity datasources (`ElasticDatasourceFactory`, OrientDB) are a proposal in [Future-Plans](Future-Plans.md). Differences from Java that are kept on purpose are in [Parity-Diffs](Parity-Diffs.md).

## Deferred on purpose

- **Phase 13: LLM, A2A and the assist panel.** Ported as the Java framework stands when the phase begins, because the Java side is not yet stable and community feedback iterations are still running. Scope: LLM client, conversation, protocols, tool loop, host routes for LLM endpoints, A2A routes (design in `../moqui/framework/plans/A2A-Server.md`), and the screens' assist panel (`js/assist`) with the other agent and assist parts. `LlmServices.xml`, `A2AServices.xml` and the tools component's `llm` REST resource were removed on 2026-10-14; take fresh copies from moqui-framework when the phase begins and wire them then. The LLM and A2A entities and `LlmTypeData.xml` stay. The LLM client sets the sim-allow exception on `RestClient`. Check: same cases as the Java tests at the time of the port. See [Implementation-Phases, Phase 13 and Phase 8](archive/Implementation-Phases.md).

## Known open items

| Area | Item | Source |
|---|---|---|
| Wasm browser host | No login flow: calls run with authorization disabled, except renders (`crates/moqui-browser/src/host.rs`) | [Wasm, Status](archive/Wasm.md) |
| Wasm | No lock between tabs: without pthreads the WasmFS OPFS backend opens a file through its async access handle, not the exclusive sync access handle, so a second tab opens and writes the database the first tab has open (`scripts/wasm-chrome-smoke.mjs` shows it and warns), and two writers can corrupt the SQLite file. A Web Locks (`navigator.locks`) lock per database name in the worker would make the second tab wait or refuse | found 2026-10-18; [Wasm, Status](archive/Wasm.md) |
| Wasm | The WebSocket check runs against a Node mock of `/notws`, not moqui-host. A live-host mode needs the package served from moqui-host's origin, a signed-in session, and checks of what the real `/notws` does (the mock's `echo:`, `drop` and `Ack` replies are its own) | same |
| Parity | Java vs MR-Moqui parity harness and its diff report (a gate artifact) has no code; only search-backend parity tests exist. Java runtime must stay runnable | [Implementation-Phases, Tests](archive/Implementation-Phases.md) |
| Search | Universal search result links go to the generic entity edit screens until IronStone has record screens; the per-document hook predicate is not built | [SearchFacade, REST service and UI](archive/SearchFacade.md) |
| Search | Semantic retrieval has the `Embedder` trait and a development hash embedder only; no local model (`semantic` cargo feature) | [Implementation-Phases, Phase 9](archive/Implementation-Phases.md) |
| Search | Postgres broad queries (common words: rank, facets, proximity) take tens to hundreds of ms at 50k documents; candidates are a stored per-field `tsvector`, a ranked-row cap, and measuring cover density on a real corpus. Hybrid `k` and weights wait on a relevance set | [Performance, Search](Performance.md) |
| IronStone | The `boss` menu has Search, QuickSearch and SalesSummary only; SalesAnalysis, SimpleReport and the User screens are not converted | ironstone-boss `mr-moqui` branch |
| MarbleERP | Acceptance work after the migration: wiki fixtures, exhaustive role and organization isolation, CSRF, every mounted QVT interaction, physical checks and scanners | [MarbleERPMigration, Limits](archive/MarbleERPMigration.md#limits) |
| Tests | `cargo test -p moqui-host --test gate3_browser`: in the server-merged qvt ArtifactGroupDetail a form-list drop-down shows its title, not the selected option (`Always Allow (overrides Deny)` is not on the page); and the static-qvt tests first run the compatibility report over every screen in `runtime/`, which is slow with the mantle components present: `qvt_report` in `cargo test-slow` passed but took 3 hours on the debug build (with another test run on the machine), and `moqui lint --screens --qvt-static` on the release build had not finished after 10 minutes | found 2026-10-18 |
| Tests | `moqui test mantle-usl SimpleScreens`: 15 of the SimpleScreensRender features fail, the same with or without the 2026-10-18 changes. Among them, the print screens are asked for with `renderMode=xsl-fo` in the path, which the screen test does not apply, so they render nothing; with `makeTest().renderMode("html-print")` they render | found 2026-10-18 |

## Parity gap review

The entity and service modules (Parity-Mapping's Entity and Service sections) were reviewed phase by phase against their Java sources on 2026-10-16: [EntityService-Parity-Review](archive/EntityService-Parity-Review.md), 8 phases, with the accepted differences in [Parity-Diffs](Parity-Diffs.md). The rest of [Parity-Mapping](Parity-Mapping.md) (moqui-lib, moqui-db, context and resources, user and session, transactions and drivers, script and templates, screens and macros, web and the HTTP host, search, notifications and cluster, mail and the tool components, tooling, browser host) has had no phase review yet; the threads to carry through one are [Parity-Mapping, Cross-cutting threads](Parity-Mapping.md#cross-cutting-threads).

The review's open findings (P1-14, P1-15, P3-20, P3-21, P4a-18, P5-19) were fixed on 2026-10-18; what differs from Java in the fixes is in [Parity-Diffs, Remaining-Work fixes](Parity-Diffs.md#remaining-work-fixes-2026-10-18).

## Code quality review leftovers

The [Code-Quality-Review](archive/Code-Quality-Review.md) of 2026-10-14 was resolved on 2026-10-15 (six fix phases, about 170 new tests), and the Lows, live checks and limits it left were worked on 2026-10-18. Still open from its [Resolution](archive/Code-Quality-Review.md#not-done-or-left-as-noted):

- **Static files:** a range is read alone (`Range` is answered), but a whole-file response is still read into memory: the response body is bytes, not a stream.
- **Process-wide global state:** the notify bus and its origin id, the transaction deadline slots and lock tracking, the QZ signer, and `LAST_BOOTED`. One factory runs per process outside tests; `LAST_BOOTED` and the single deadline timer are process-level by design, and the notify bus and lock tracking have no factory handle at their call sites.

## Mantle conversion follow-up

mantle-usl services are hand ported to JavaScript, and `moqui lint --groovy` is at 0 flagged sites ([ConvertProcess](archive/ConvertProcess.md)). The specs run under `moqui test mantle-usl` ([MantleFlowTests](archive/MantleFlowTests.md)): 344 features in 43 specs, 751 of 762 services called (`moqui test mantle-usl --coverage`).

- **The 11 services never called:**
  - three call Google (`GooglePartyServices`) and one the reCAPTCHA service (`create#ContactRequestReCaptcha`), which need a network;
  - four are `TenantSubscriptionServices`, left for the deprecated multi-tenant mode (the `fulfill#ProductSubscriptionResource` it builds on is covered);
  - `TaxServices.calculate#InvoiceSalesTax` is an empty TODO upstream;
  - `ReturnServices.returnAndResell#Shipment` never places its new order upstream (a spec documents where it stops);
  - `PartyServices.send#DataDocumentNotifications` needs the data feed of a search backend.
- **Unfinished upstream, kept as they are:** `generate#Efw2File` writes no RT, RU, RV or RF records and never sets `fileText`; `autoApply#Payment`, `get#VariantProductsInfo`, `calculate#SalesTaxLocal` and `post#WorkEffortCost` are stubs.
- **ironstone-boss:** 18 `moqui lint --screens` failures.

Migrating other components' scripts is outside this repository's gates.

## Related documents
[Future-Plans](Future-Plans.md) (digest of the unbuilt [proposals](proposals/)), [Performance-Summary](Performance-Summary.md) (the next phase), [Parity-Mapping](Parity-Mapping.md) and [Parity-Diffs](Parity-Diffs.md) (reference for the gap review). The index is [README](README.md).
