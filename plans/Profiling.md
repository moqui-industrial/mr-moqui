# Profiling

Status 2026-10-21: four rounds of profiling and optimization of entity, service, data load and XML/definition work against embedded Postgres, built and committed. The next round tests against an external Postgres (see [Next steps](#next-steps)). Companion to [Performance-Summary](Performance-Summary.md), which digests the performance plan; this document records what was measured and changed, how to measure, and what is left.

## How to measure

Five opt-in speed tests in `crates/moqui-host/tests/` (each `test = false` and `#[ignore]`, so a default `cargo test` neither links nor runs them):

| Test | What it times | Needs |
|---|---|---|
| `entity_speed` | entity facade finds (cache, PK, list, count, iterate 100 rows) and writes (create, update, delete, `create_or_update`), in one transaction and in autocommit | embedded Postgres |
| `service_speed` | the `SpeedTest.xml` calls: entity-auto create/update/delete of `moqui.test.TestEntity` and `echo#Data`, plus the same entity calls direct, in one transaction and each in its own | embedded Postgres |
| `data_load_speed` | XML create and reload, CSV, JSON, nested records, and the framework seed data in `moqui load` order (first load and reload) | embedded Postgres |
| `xml_speed` | `MNode` parse, keep, read, lookup and drop over the 784 runtime XML files, and loader-style record elements | nothing |
| `definition_speed` | the framework entity model, the service definitions, and the base components' screen definitions | nothing |

Run one with `CARGO_PROFILE_RELEASE_DEBUG=1 cargo test --release -p moqui-host --test <name> -- --ignored --nocapture`; each file's header names its size variables. The shared harness (`tests/speed/mod.rs`) prints, per call: median time, allocations, bytes allocated, bytes kept after the batch (a cache or a leak), and SQL statements (`entity::statement_stats`, a round-trip count). The allocation columns come from a thread-local counting allocator and do not change from run to run, so they are the steady measure on a busy machine.

Tools, as used here:
- **samply** for CPU (`samply record --save-only --unstable-presymbolicate`); needs `/proc/sys/kernel/perf_event_paranoid` at 1 or lower, which resets at reboot.
- **heaptrack** for allocation sites: `heaptrack_print X.zst --flamegraph-cost-type allocations -F stacks.txt`, demangled with `c++filt`, filtered to the stacks under one benchmark row (its closure, or a function such as `ScreenFacade>::screen_def`). Identical row closures are folded into one symbol, so pick a row whose closure is unique.
- **perf stat** `-e instructions:u` with 1 and 6 batches, for a load-independent comparison (the difference over 5 is one batch, Postgres children included).
- **strace** `-ff -o DIR/t` for syscalls per round trip (`perf trace` has no tracefs access on the dev machine); the client thread is the file whose `sendto` carries `SET TIME ZONE`.
- **Comparing builds:** copy each build's test binaries aside under a tag, run them alternately, and take each row's best batch median. Wall times swing ±50–100% when other jobs run; allocation counts do not.
- **Keep profiling output on disk** (`target/profiling/`), not in `/tmp`: `/tmp` is a RAM-backed tmpfs on the dev machine, and 23 GB of traces and binary copies there ran it out of memory during a workspace test build.

## What changed

Round 1 (commit `1cb412f`, started by Grok, reviewed and extended): the entity speed test, cached finds and autocommit writes.

Round 2 (commit `2f33ebb`): service speed test; rows mapped by moving values instead of cloning; binary timestamp decode; per-entity read shapes (column list and select head built once); cached per-entity read facts and implicit entity-auto service definitions; ECA and encryption copies only when an entity has rules or encrypted fields; Postgres parameters written straight into the send buffer.

Round 3 (commit `8b09f43`):
- **Driver on `tokio_postgres`** with `BEGIN` and `SET LOCAL statement_timeout` sent with the first statement of the transaction.
- **Deferred writes** (`DbConnection::execute_deferred` / `finish_deferred`): in a data file's transaction, creates and updates are sent without waiting; the next statement that waits, the loader's end of file, or `COMMIT` reports the first failure (and `COMMIT` refuses after one, since Postgres would roll back silently). The default implementation runs the statement at once, so MySQL and SQLite are unchanged.
- **`create_or_update` of a new row**: an `UPDATE` probe that matches nothing goes straight to `INSERT`, without the refresh `SELECT`.
- **Shared field names**: entity field maps are `FieldMap = BTreeMap<Arc<str>, Value>`, `DbRow` column names are `Arc<str>` shared per statement, and `EntityValue::set` takes the entity's own name instead of allocating one.
- Loader: shape cached per entity, records moved instead of cloned, attributes read in place, same-type values not converted.

Round 4 (commit `50a7e4b`, then the first-run prepare, shared record keys and definition-builder changes after it):
- **Upsert replaces try-insert.** A data load, CSV or ETL store of a record with its whole primary key is one `INSERT ... ON CONFLICT DO UPDATE` (MySQL `ON DUPLICATE KEY UPDATE`, SQLite `ON CONFLICT`), pipelined in a data load, wherever the write cannot differ from create-or-update: no create or update ECA rule, no field an update reads the stored row for (audit log, create-only, field defaults, optimistic lock), no data feed that wants the old values, no transaction cache or simulation, authorization off, and no non-key `statusId` (the entity-auto store checks a status change). Anything else takes the read-first store as before. `useTryInsert`, Java's `never-try-insert` and `EtlLoader::use_try_insert` are gone; `useTryInsert` parameters are accepted and ignored. An upsert's hit is a create's, and an explicit null in the record is written so an empty attribute still clears the field.
- **MNode.** The parser scans bytes, takes names and plain values as slices of the source, and counts line and column only where a node starts or an error is reported. Element and attribute names are interned `Rc<str>`; attributes are a small `Vec` instead of an `IndexMap`; one file location is shared per document; lookups (`children_named`, `first_named`, `first_named_attrs`, `write_string`) no longer copy. New borrowing calls: `has_name`, `with_name`, `with_attribute`, `attribute_is`, `has_attribute`, `child_count`, `for_each_child`, `for_each_attribute`, and `parse_flat` (an element with no children, read without a node).
- **Data loader** reads a flat record element with `parse_flat`, keys records by the entity's shared field names, and hands an upsert the record's map with no copy.
- **Postgres wire client** (`crates/moqui-host/src/db/pgwire.rs`): the protocol over a blocking Unix or TCP socket, TLS by rustls, `postgres-protocol` for message encoding, parsing and SCRAM. `tokio-postgres` and `tokio-postgres-rustls` are removed. A round trip is one write and one read. Every request ends with `Sync`, so an unanswered one (BEGIN, `SET LOCAL`, a deferred write, a statement close) is read and checked by the next call that waits.
- **First run prepares the statement** in the same round trip (Parse, Describe, Bind, Execute, Sync) where a prepare and a run took two; a first run that fails after parsing closes the statement so a later use prepares it again.
- **Definition builders**: the service loader's JavaScript syntax check reuses one parser arena per thread; the entity model copies an entity's nodes only when an `extend-entity` changes them; screen and service builders read names and children without copying.

## Results

Times are microseconds per call on the dev machine with embedded Postgres over a Unix socket, from alternating runs; allocations are exact. "Earliest" is the first recorded number for the row: round 2's start for entity rows and allocations, round 3's start for service and data load times, round 4's start for XML and definitions.

| Operation | Earliest | Now | Allocations earliest → now |
|---|---|---|---|
| entity one PK find, database | ~28 | 19.7 | 162 → 28 |
| entity cache hit | 1.21 | 0.82 | 31 → 12 |
| entity list 100 rows | 555 | 232 | 9,800 → 1,132 |
| entity create / update / delete (one transaction) | ~32 / 33 / 25 | 27 / 30 / 22 | 96 / 86 / 62 → 38 / 43 / 24 |
| service create, own transaction | 83 | 60 | 196 → 128 |
| service update / delete, one transaction | 37 / 28 | 33 / 24 | 137 / 106 → 66 / 42 |
| data load, XML record, new | 109 | 13 | 406 → 48 |
| data load, XML record, reload | 56 | 14 | 327 → 48 |
| data load, framework seed data, first load / reload | 146 / 68 | 46 / 21 | 576 / 497 → 87 / 74 |
| MNode parse, per file (784 files, 12.6 MB) | 176 | 76 | 2,723 → 608 |
| MNode retained memory, whole corpus | 78 MB | 39 MB | |
| framework entity model | 5.9 ms | 5.3 ms | 99k → 79k |
| service definitions | 9.7 ms | 8.7 ms | 79k → 39k (34 MB → 6.5 MB allocated) |
| screen definition, per screen | 131 | 112 | 1,282 → 467 |

Syscalls on the client thread for the same entity run: 96k with `tokio_postgres` (46k epoll waits, about 4 per round trip), 49k with the wire client (one `sendto` and one `recvfrom` per round trip).

What the numbers say:
- **Single statements are round-trip bound.** A PK find is about 20 µs of which the client's share is now small; halving allocations moved times by single digits. Over a network the round trip dominates more, so pipelining (deferred writes, BEGIN with the first statement, prepare with the first run) is worth more against an external server than these numbers show.
- **Data loads are where the design changed the outcome:** a record went from three waited statements to one pipelined statement, and the client work per record from 406 allocations to 48.
- **Memory.** Read rows share their column names, values share their entity's names, and parsed XML trees hold half what they did.

## Behaviour changes to know

- An upserted data-load record records a `create` artifact hit whether or not the row existed, and runs no update ECA rules (it is only used where the entity has none).
- `create_or_update` of a new row no longer records a `refresh` hit.
- A deferred write's failure surfaces at the next statement of the transaction or at the end of the data file, not at the write itself. The message is the failed statement's.
- `EntityValue::fields()` returns `&FieldMap`, `set` takes `impl IntoFieldName`, `DbRow` names are `Arc<str>`, `run_matched_eca` takes a `FieldMap`; `string_map` and `field_map` convert. Add-on component crates outside the workspace need the same small changes.
- `echo#Data` returns `timestampOut` as millisecond text, as the script layer converts dates on purpose; Java returns a `Timestamp`. Undecided.

## Next steps

**External Postgres (next).** What the embedded tests cannot show:
1. **TLS against a real server.** The client's TLS is tested only against an in-process server (SSLRequest, rustls handshake, startup, a query, Terminate). Test certificate verification with real roots, a server that requires TLS, and `sslmode`-style choices if any are wanted beyond "required".
2. **SCRAM channel binding.** The client offers `SCRAM-SHA-256` without channel binding; a server set to require `SCRAM-SHA-256-PLUS` would refuse it. Add `tls-server-end-point` binding from the rustls peer certificate if that matters.
3. **Latency.** Re-run `entity_speed`, `service_speed` and `data_load_speed` against a server one network hop away. The round-trip savings should grow: BEGIN pipelining saves one round trip per own-transaction service call, upsert pipelining all but one per data file, first-run prepare one per new statement per connection.
4. **Connection failure.** A server restart or a dropped TCP connection mid-transaction, mid-pipeline and mid-stream: the pool should discard the connection (`is_closed`), the frame should roll back, and the error should name the cause. Also `cancel` over TLS and a `statement_timeout` that fires.
5. **Poolers.** Named prepared statements need session pooling, or PgBouncer 1.21+ with protocol-level prepared statements. Check, and document the setting.

**Further work, by expected payoff:**
1. **COPY for bulk loads.** An empty-database load (`raw`, or a new seed) could stream rows with `COPY ... FROM STDIN` instead of one statement each; with the wire client this is a small addition and the largest remaining win for big loads.
2. **Multi-threaded benchmark.** `with_slot` is one facade-wide mutex that finds and writes take several times; a single-thread benchmark cannot show contention. Add a row that runs N threads on separate connections.
3. **Deferred writes on a cold statement** still prepare with a waited round trip (once per statement per connection). Queue the prepare with the write.
4. **Binary parameters** for integers, timestamps and numerics: less text formatting here and less parsing on the server.
5. **Remaining copying MNode calls in definition builders.** `name()`, `attribute()` and `children()` still copy; screen, form and service builders use them a few times per node. The borrowing calls exist; moving the busiest callers (screen `form.rs`, `definition.rs`, `service/define.rs`, the moqui-db model) would cut definition load further. The entity model's remaining allocations are mostly its own `String` fields.
6. **Nested data records** still go through an `MNode` per top-level element; a streaming builder could read them the way `parse_flat` reads flat ones.
7. **Reads before write** for entities that cannot upsert (audit-logged, status flows, data-feed entities) still cost one waited round trip each in a load.
8. **QuickJS for trivial actions.** About half of `echo#Data` is QuickJS running four `<set>` actions through its proxy; a native path for trivial actions would change value types, so it needs a decision.
9. **Transaction bookkeeping.** A transaction keeps about 1.3 KB per write until commit, for the end-of-transaction cache eviction; a smaller per-write record would do.
10. **A Java comparison.** Run the same `SpeedTest.xml` calls and a seed load on Moqui Java against the same Postgres, so the numbers above have a reference.

Known flaky test, not from this work: `service::coverage::tests::calls_are_counted_only_between_enable_and_finish` counts into a process-wide recorder and can race with tests running in parallel.
