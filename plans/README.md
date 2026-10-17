# Plans

Design and status documents for the MR-Moqui port. As of 2026-10-14 the build (phases 0–12, gates 1–4) is done except LLM/A2A.

**Start here**
- [Remaining-Work.md](Remaining-Work.md): what is still open from the build, including deferred LLM/A2A.
- [Future-Plans.md](Future-Plans.md): digest of unbuilt proposals (Messaging, Monitoring, Data, Usability).
- [TestHarness.md](TestHarness.md): design and build order for `moqui test`, the JavaScript Spock replacement, and the port of the mantle-usl specs to it.
- [MantleFlowTests.md](MantleFlowTests.md): the Mantle USL Spock flows as JavaScript specs run by `moqui test`: how to run them, status of each.
- [Performance-Summary.md](Performance-Summary.md): digest of the performance plan, recorded numbers and Mantle USL audit.
- [Code-Quality-Review.md](Code-Quality-Review.md): code quality and problematic-pattern review of `crates/` (2026-10-14), with 15 verified High findings, cross-cutting patterns and a fix order.

**Active detail documents:** `Performance.md`, `Performance-Implementation.md`, `MantleUSL-Performance-Audit.md`, `MantleUSL-Execution-Benchmark-Plan.md`, `Messaging.md`, `Monitoring.md`, `Data.md`, `Usability.md`, `PostgresBackup.md`.

**[archive/](archive/)**: built or complete designs, kept as reference: `Implementation-Phases` (phase and gate record), `Architecture-Proposal`, `Crate-Layout`, `ContextObjects`, `SearchFacade`, `QvtServerStatic`, `EmbeddedPostgres`, `Cluster`, `Ssh`, `Sso`, `Office-Image-Print`, `TypeGeneration`, `Wasm`, `ConvertProcess` (Mantle conversion trial), `License-Review`, plus the earlier reviews `MimoReview`, `SonnetReview`, `SonnetRecommendations`.
