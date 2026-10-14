# Performance Plan

Source discussion: https://chatgpt.com/share/6ace4f8e-ffd4-83e8-8648-6014b19ade13

## Architecture: one-node and maximum-scale setups

Support two deployment setups with the same application features and transaction semantics:

- **One node:** Run everything on one server: web/application services, PostgreSQL, reporting, search/indexing, background jobs, and all other features. Priorities are full feature coverage first, keeping everything on the server to avoid third-party service dependencies second, and maximum performance within those constraints third.
- **Maximum scale:** Separate work across five node types: three application-server types and two database types.
  1. **Writing application servers:** Handle many concurrent client connections and internal business logic only; offload third-party data conversion and heavy processing to processing servers.
  2. **Processing application servers:** Handle third-party data conversion and heavy processing, keeping that work off writing servers.
  3. **Batch application servers (optional):** Writing-type nodes dedicated to asynchronous internal business work; they do not accept client connections. Separate batch nodes may not be required.
  4. **Write database:** A PostgreSQL node dedicated to transactional writes to maximize concurrent durable transaction throughput. Keep reports, search, and derived-index computation off this node; reads required for transaction correctness still execute here.
  5. **Read database:** A PostgreSQL node for read workloads, report calculations, search indexes, and other derived computations, fed from committed write-database changes. Use a writable read model when derived tables/indexes must be maintained locally; a physical read-only standby cannot perform those writes. Define replication lag and read-after-write behavior explicitly.

Measure the setups separately. The local PostgreSQL paths and shared-machine resource budgets below describe the one-node setup; maximum-scale measurements must include inter-node networking, replication, and derived-data freshness without weakening write durability.

## TPC-C single-node comparison

The historical audited non-clustered TPC-C record is [Oracle on SPARC T5-8](https://www.tpc.org/1792): 8,552,523 tpmC, or approximately 142,542 New-Order transactions/second (128 database cores, 2013; external storage and separate load generators).
Each successful New-Order transaction commits durably after reading customer/tax data, advancing a district order counter, inserting an order and queue entry, and updating stock and inserting order lines for 5–15 items (10 on average).
The metric counts New-Order transactions only; other transaction types run alongside them. These are durable commits, but the transaction mix and work per commit must match before throughput is directly comparable.

## Target: 100,000 commits / reads of transactional business data per second

The existing plan has not demonstrated this target; reproducible measurements must be included before making measured-capacity claims. [Executed durable-WAL follow-up](../experiments/mantle-execution/WAL-FOLLOWUP.md) reaches approximately 26k/s on the faithful extracted 100-line invoice path, 35.8k/s over 15 minutes on a reduced mixed 100-line fused-SQL path, and a repeated median 53.5k/s on a lighter one-line diagnostic. These reduced, write-only experiments omit full framework semantics and do not pass this target; complete startup/reload/edit-to-ready acceptance remains unmeasured.

Aim for 100,000 durable commits / completed reads of transactional business data per second through the Rust framework and PostgreSQL. Specify the commit/read mix and report both rates separately; internal SQL reads, row writes, accepted messages, and business processes are not this metric. Include validation, configured authorization/ECA/audit/feed behavior, indexes, and successful durable commit before acknowledging a write. Use independent keys for the initial capacity test; hot-row contention is a separate limit.

Report rows/second separately. A transaction containing 100 writes counts as one commit. Keep PostgreSQL `fsync` and synchronous commit enabled, with storage that honors flushes. This is a target, not a demonstrated capability. The single-row write estimates below are a write-only baseline, not the definition of the target.

## XML: minimum work and startup budget

Load definitions once into factory-level shared memory, not once per request. The Rust path is:

```text
read XML bytes
  -> parse and allocate XML trees
  -> merge extensions and resolve entity/service references
  -> build indexed runtime metadata
  -> generate service action code and SQL templates as needed
```

If XML itself must be generated before serving requests, include generation and required file output in the startup budget. Existing files require no XML-generation step. Required QuickJS service compilation also counts toward startup, although it is not part of the native Rust XML-parser floor.

For a planning corpus of 300,000 lines at 50–150 bytes/line, input is approximately 15–45 MB. Measure the actual corpus bytes and node/attribute counts; line count does not determine runtime cost.

Illustrative resource floors, not benchmark results:

| Operation | Assumption | Time for 15–45 MB |
|---|---|---|
| Cold sequential file read | Effective 1–3 GB/s | 5–45 ms, excluding file-open latency |
| One scan of resident bytes | Effective 10–30 GB/s | 0.5–4.5 ms |
| XML parsing/tree construction | Effective 100–500 MB/s | 30–450 ms |

The scan is only a bandwidth floor; it does not include syntax handling, string copies, allocation, cache misses, or relationship resolution. Those bandwidth assumptions must be measured on the target machine. Multiple small files can cost more than one equivalent sequential read. Streaming can overlap reading and parsing, so these are not necessarily additive.

Set a **500-millisecond maximum cold-start budget, from process launch to ready to serve requests**, for this 300,000-line corpus. Include cold-file reads, XML parsing, native Rust model construction, required action/SQL generation and JavaScript compilation, database startup and connections, required schema checks, worker initialization, and listener readiness. Readiness must mean an authenticated database-backed request can execute successfully, not merely that the listener is open. This is an aggressive acceptance target, not a measured result or physical upper bound.

Every step required for readiness counts toward the same 500 ms total; there is no separate excluded initialization budget. Embedded PostgreSQL startup counts when that mode is used. On an empty database, required schema creation and seed/data loading count too. Report existing-database and empty-database results separately without changing the target. Start without a pre-running application, embedded database process, or warmed definition cache; distinguish cold-file from warm-file runs.

The required startup checklist is:

1. Process launch, executable/shared-library loading, and Rust initialization.
2. Locate runtime assets; load environment, configuration, and deployment secrets; generate and persist unset secrets when required.
3. Discover components and order dependencies; generate any required XML/assets that do not already exist.
4. Read and parse configuration and the 300,000-line definition corpus into memory.
5. Merge entity extensions; resolve entities, relationships, views, services, parameters, and required screen/route definitions; populate shared metadata caches.
6. Generate required XML-action code and SQL templates; validate expressions; initialize QuickJS runtimes and compile code required for readiness.
7. Start the configured database, establish required connections, and initialize transaction pools and reserves.
8. Check database metadata; create or migrate required schema and indexes; load required seed/data; commit initialization transactions durably.
9. Initialize authorization, users/sessions, facades, caches, bounded request/service workers, scheduler, and configured tools whose initialization is required before serving.
10. Bind listeners and install routes; complete any required proxy/TLS startup when those processes are part of the measured deployment launch.
11. Successfully execute an authenticated database-backed readiness request, then declare ready. Include this probe in the elapsed time.

Run independent steps concurrently only where dependencies and correctness permit. Time every applicable step and the end-to-end interval using a monotonic clock. Do not defer required work behind a successful readiness response or exclude child-process startup to meet the target. Optional features that are disabled need not start; enabled features that gate readiness must fit the same budget.

Rust implementation requirements: release builds, bounded parsing passes, indexed name/reference lookup, minimal repeated string copies, and shared immutable definitions. Do not generate every possible query at startup. Cache generated templates and prepare statements on each database connection when first used.

## Incremental builds and development feedback

The 500 ms startup target does not imply a 500 ms build. Compilation, linking, shutdown, startup, and any verification request are separate parts of the edit-to-ready interval.

Prioritize these development targets on a specified fast development machine with warm build caches. These are acceptance goals, not measured results:

| Change | Target |
|---|---|
| Reloadable XML, JavaScript, or templates | No Rust build; at most 100 ms from saved edit to updated behavior, with 10–100 ms as the planning range |
| Small Rust implementation change | At most 2 seconds edit-to-ready, including rebuild, shutdown, startup, and a successful verification request |

For small Rust changes, 0.5–2 seconds for compilation and linking is an optimistic planning range; allow 1–5 seconds when estimating before measurements exist. Meeting the 2-second edit-to-ready target with a 500 ms restart leaves less than 1.5 seconds for the build, shutdown, and verification. Sub-500-ms native rebuilds are a stretch goal, not a promise. Public API or widely used generic changes may take several to tens of seconds; optimized release rebuilds are a separate measurement, not the normal edit loop.

Keep application XML, JavaScript, and templates outside Rust compilation. Support targeted development reloads that invalidate affected definitions, compiled scripts, and dependent caches rather than reloading the entire corpus. Do not assume schema or startup-only configuration changes can reload safely; measure their required migration or restart path separately.

Use incremental compilation and a fast linker such as `lld`, or `wild` on supported Linux targets. Avoid LTO in the development profile, reduce debug-info overhead where acceptable, and keep unchanged heavyweight dependencies cached. Any development-profile optimization must preserve correct behavior; separately verify that the chosen profile can meet its startup budget rather than assuming release-build startup numbers apply.

Benchmark representative implementation edits in `moqui-core`, not only the small binary crate. Report compilation/linking time and end-to-end edit-to-ready time separately, including median and p95 over repeated edits. Record hardware, toolchain, profile, linker, cache state, and changed code. Measure reloads, public API changes, and release rebuilds separately; a no-op Cargo invocation is not an incremental-change benchmark.

## SQL and transaction preparation

Use parameterized SQL and the planned per-connection prepared-statement cache. Bind new values rather than reparse XML or regenerate identical SQL for each write. Rust can generate SQL text; PostgreSQL still performs its own parse/plan preparation and execution.

For a simple write, the minimum database exchange can be one prepared statement in autocommit, which also commits that statement's transaction. Use `RETURNING` only when its result is needed. Multi-statement entity/service work requires its actual transaction boundaries; do not remove them to meet the target.

Avoid an existence query plus write plus readback when one correct statement suffices. Preserve entity semantics, related writes, and pending-feed registration. Count all resulting SQL statements, index changes, and WAL bytes against the write budget.

## Approximate physical capacity limits

At 100,000 writes/second, the system completes one write every **10 microseconds in aggregate**, not necessarily with 10-microsecond request latency.

```text
CPU ceiling       = available CPU-seconds/second / total CPU-seconds/write
WAL ceiling       = sustainable WAL bytes/second / WAL bytes/write
commit ceiling    = average transactions/group / durable group interval
network ceiling   = usable bytes/second / total wire bytes/write
concurrency limit = in-flight writes / average end-to-end latency
```

Sustainable throughput cannot exceed the tightest relevant limit. These are approximate resource bounds, not a prediction: locks, scheduling, random I/O, checkpoints, and queueing can lower capacity further. Rust does not eliminate PostgreSQL IPC, WAL, or storage latency.

What the target requires under illustrative assumptions:

| Resource | Requirement at 100,000 writes/s |
|---|---|
| CPU: 16 cores at 80% useful utilization | At most 128 microseconds total CPU/write across application, database, and proxy on that machine |
| WAL: 1–4 KiB generated/write | Approximately 102–410 MB/s sustained WAL output, plus checkpoint/data-file traffic |
| Durable group interval: 100 microseconds | Average at least 10 transactions per group |
| Durable group interval: 1 millisecond | Average at least 100 transactions per group |
| Mean end-to-end latency: 1 millisecond | At least 100 writes in flight |
| Mean end-to-end latency: 10 milliseconds | At least 1,000 writes in flight |
| Total client request/response traffic: 2 KiB/write | Approximately 205 MB/s before additional protocol overhead |

One serialized durability barrier per independent write would require a barrier every 10 microseconds. Do not assume storage can do that. PostgreSQL group commit must amortize barriers across independent transactions without acknowledging them before durability. Measured flush latency and group size determine whether this target is achievable.

Use enough bounded Rust workers and database connections to cover the measured latency, within the existing pool reserve/parking constraints. More connections are not automatically faster. If transaction latency requires impractical concurrency, reduce the work or latency rather than assuming a larger pool solves it.

## Minimum syscalls and physical operations

Reuse client connections, database connections, and prepared statements. Socket creation, connection establishment, TLS handshakes, and statement preparation are first-use costs, not requirements for every write.

Representative steady-state local PostgreSQL path, with TLS terminated at the deployment proxy:

```text
proxy: receive client ciphertext; decrypt
Rust host: receive HTTP request
Rust worker: validate; bind cached SQL
Rust driver: send PostgreSQL protocol message
PostgreSQL: receive; execute; update row/index buffers; append WAL
PostgreSQL: write WAL; complete durable flush/group commit
PostgreSQL: send successful transaction result
Rust driver: receive result
Rust host: send HTTP response
proxy: encrypt; send client response
```

Representative Linux syscall families, not a guaranteed one-call-per-stage trace:

| Process/work | Typical calls |
|---|---|
| Socket receive/send | `recvfrom`/`recvmsg`/`read`, `sendto`/`sendmsg`/`write`/`writev` |
| Network readiness | `epoll_wait`/`epoll_pwait`, with interest registration as needed |
| Worker notification/parking | `futex`, event notification, or runtime-specific mechanisms when needed |
| PostgreSQL WAL output | `write`/`pwrite64`/`writev`, depending on implementation |
| PostgreSQL durability | `fdatasync`/`fsync`, or configured synchronous-write method |
| Data/index cache miss | File reads such as `pread64`; absent for resident pages |
| Background checkpoint | Data-file writes and required synchronization |

The minimum logical work is: request transfer, database command transfer, required row/index changes, WAL persistence, commit confirmation transfer, and response transfer. Hot data can avoid foreground data-page reads; checkpoint writes remain. TLS, parsing, SQL binding, and cached index traversal are CPU work and do not inherently require individual syscalls.

Batch independent socket/WAL operations and let group commit share flushes. A syscall, a WAL write, a storage-device command, and a durability barrier are different units; minimizing one does not prove the others were minimized. Handle short reads/writes and wait for successful completion before dependent work.

With a ring-aware host, socket operations can be submitted as `IORING_OP_RECV` and `IORING_OP_SEND` via `io_uring_enter`, potentially in batches. This does not put PostgreSQL WAL writes on the application's ring. Keep the planned Axum/Tokio baseline unless an equivalent measured ring implementation reduces actual CPU or physical work.

To establish the achievable maximum, measure the XML stages and run the write target with syscall counts across application/proxy/database, total CPU/write, WAL bytes/write, flush latency/group size, completed transactions/s, errors, and tail latency. Use syscall tracing for diagnostic runs rather than treating instrumented throughput as the production ceiling. Keep durability and entity semantics unchanged.
