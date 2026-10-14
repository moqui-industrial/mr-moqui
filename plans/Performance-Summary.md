# Performance summary

Digest of [Performance.md](Performance.md), [Performance-Implementation.md](Performance-Implementation.md), [MantleUSL-Performance-Audit.md](MantleUSL-Performance-Audit.md) and [MantleUSL-Execution-Benchmark-Plan.md](MantleUSL-Execution-Benchmark-Plan.md). The first two are self-contained plans with recorded numbers; the audit is a static source review of mantle-usl; the benchmark plan defines the proofs.

## Target
**100,000 durable commits / completed reads of transactional business data per second**, one-node first, then a maximum-scale split (writing, processing and optional batch app servers; a write database and a read database). Reference point: TPC-C record ~142,542 New-Order/s (128 cores, 2013). **Not demonstrated.** Never substitute generated statements or row counts for it; use a declared read/write mix, durable acknowledgments, full framework semantics and tail latency.

## Recorded so far (local experiments, not full acceptance)
- Durable-WAL follow-ups: ~26k/s faithful extracted 100-line invoice path, 35.8k/s over 15 min on a reduced mixed fused-SQL path, median 53.5k/s on a lighter one-line diagnostic.
- XML parse/tree: 156.77 MB/s resident (21.1 MB corpus, 134.8 ms median); real framework entity model (217 entities) 4.856 ms median; warm read+parse of 21 MB ≈ 32% of a 500 ms startup budget.
- SQL text generation: 0.18–0.60 µs per statement (insert, update, delete, upsert, PK select). Excludes execution, auth, commit.

## Implementation goals ([Performance-Implementation](Performance-Implementation.md))
1. Inspect schema in bulk and install atomically (the browser build already does this in one transaction: 18 s → 2.0 s).
2. Reuse XML-action macro compilation across loads.
3. Load component definitions as one coherent model.
4. Cut development feedback cost (incremental builds).
Also in [Performance.md](Performance.md): SQL/transaction preparation, approximate physical capacity limits, minimum syscalls.

## Mantle USL audit findings
Avoidable work amplification comes before engine speed: per-item total recalculation on orders/invoices (triangular), synchronous ECA/service cascades, historical summary scans, duplicate reads, quadratic collection loops, and JS host-boundary/decimal costs. QuickJS may stay but is unproven for financial arithmetic and collection-heavy paths. Recommended: fix workload shape, add indexed algorithms, then build a real generated-JS + facade spike (party create, bounded order import, invoice finalization, reservation, payment) and compare native orchestration vs QuickJS through the same facades and commit boundaries. Optional ahead-of-time Rust backend from a shared validated action IR is a proposal only.

## Next steps
Run Stages A–E of the benchmark plan (inventory/oracle → minimum real QuickJS path → cheapest improvement → execution-method comparison → capacity and UX) with the correctness gate before any performance claim.

Note: the audit and benchmark plan link to `../experiments/mantle-execution/README.md`, which is not in this repository (the results came from a separate experiment branch).
