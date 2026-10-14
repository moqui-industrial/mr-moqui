# Performance implementation plan

Companion to [Performance.md](Performance.md). Define outcomes, not file layout or implementation steps; the implementing agent chooses the integration. Excludes application-specific features and webroot changes.

Rust below is illustrative pseudocode: helper names are not promised APIs and these are not runnable tests. All required goals, algorithms, test examples, recorded numbers, and measurement context are inline here or in the companion Performance.md. Original experiment-report filenames below identify provenance only; those reports, benchmark scripts, and implementation files are not required to read or implement this plan. Measurements have not been newly verified here.

## Performance criteria and recorded before/after

Preserve durability, authorization, required initialization, and business/seed transaction boundaries. A faster incomplete startup is not a pass.

| Measurement | Before | After | Acceptance criterion |
|---|---:|---:|---|
| Existing DB: process launch → authenticated response, median | 867.929 ms | 372.403 ms | Maximum ≤500 ms, including database startup |
| Empty DB: process launch → authenticated response, median | 17,207.475 ms | 11,402.169 ms | Maximum ≤500 ms, including schema and seeds |
| Application SQL statements before readiness probe, existing DB | 1,004 | 7 | Reduce repeated inspection without skipping live schema checks |
| Application SQL statements before readiness probe, empty DB | 3,443 | 2,904 | Preserve complete schema/seed initialization |
| Unchanged PostgreSQL schema-check batch | Per-object inspection; isolated count not recorded | 3 catalog queries, no DDL | Constant catalog-query count per batch, live repair still works |
| Clean development build | 98.66 s | 56.98 s | No clean-build target; report separately |
| Representative warm Rust edits, median / p95 | Not measured with valid actual edits | 3.27 s / 3.38 s | Whole edit → verified readiness ≤2 s |

**Startup samples:** baseline empty DB `17,326.603 / 17,207.475 / 17,042.457 ms`; baseline existing DB `895.244 / 737.973 / 867.929 ms`. Combined optimized implementation empty DB `12,399.365 / 11,402.169 / 10,139.491 ms`; existing DB `373.169 / 362.409 / 372.403 ms`. Each pair used a fresh database followed by a restart, with application and PostgreSQL stopped between launches.

These are warm-file, release-build, real-corpus measurements on an AMD Ryzen AI 7 350 (16 logical CPUs, 30 GiB RAM), Linux 7.2.5-3-omarchy, rustc 1.97.1. The startup counts/fixtures describe the measured corpus, not every optional component. The improvements combine bulk schema inspection, macro caching, and atomic DDL; **individual timing contributions were not isolated**. A separate existing-DB series had a `555.238 ms` outlier, so maximum acceptance is not established. Empty-DB startup fails; fresh embedded PostgreSQL bootstrap alone measured `1,587.006 / 1,531.388 / 1,511.921 ms`.

Build clean timings are full-debug versus `debug = 0`; source snapshots differed, so they are not a controlled before/after comparison. Original timestamp-only rebuilds took `6.45–10.67 s`, but are **not valid actual-edit baselines**. The after edit result uses 20 actual logic edits; source changes, compilation, and linking only, not shutdown/startup/readiness. It fails the ≤2 s target even before those extra stages.

### Original sources (provenance only; not required files)

- **Startup before:** `plans/Performance-results.md`, section “Real database and network startup (baseline)” — startup samples/medians and pre-probe SQL counts.
- **Startup after:** `plans/Performance-results.md`, section “Startup optimizations” — final three paired runs after atomic DDL, SQL counts, three-query schema recheck, and the separate 555.238 ms outlier.
- **Database bootstrap:** `plans/Performance-results.md`, section “Empty-database limitation” — the three bootstrap-only samples.
- **Build before/after:** `plans/Build-performance.md`, section “Results” — original full-debug clean build, reduced-debug clean build, timestamp-only rebuilds, and 20 actual logic-edit median/p95.
- **Parser/model diagnostics:** `plans/Performance-results.md`, sections “Environment and reproduction” and “XML and entity model”.
- **SQL-generation diagnostics:** `plans/Performance-results.md`, section “SQL generation”.
- **Acceptance targets:** `plans/Performance.md`, sections “XML: minimum work and startup budget”, “Incremental builds and development feedback”, and “Target: 100,000 commits / reads of transactional business data per second”.

The original sources were local experiment reports, not raw timing artifacts or independent verification. Their relevant data and limitations are reproduced inline. No per-optimization timings are inferred where the reports only measured combined changes.

### Inline measurement context

**Recorded startup experiment:** the normal application server bound to loopback and used a disk-backed disposable runtime under `$HOME/.cache`, with normal base components plus a tiny readiness seed component. The normal timed first-start loader installed a public demo-user fixture and an entity-read authorization grant. Readiness required HTTP 200 with seeded data from `/rest/e1/moqui.basic.EnumerationType`; an anonymous request required HTTP 401. A preliminary metrics request was included in the timer; a post-response metrics request verified increased process-wide SQL counters outside that timer. These counters are corroboration, not request-scoped tracing. Direct post-response database checks verified durability and 167 public tables, 419 public indexes, 186 enumerations, 37 enumeration types, and 10 user-group permissions. First starts loaded 1,346 records; restarts loaded zero. Executables/definition files were warm, builds and shutdown between samples were excluded, and no application/database process remained running between launches. Required schema and seed work was not moved outside startup timing. The baseline reported 243 service expressions not yet converted to JavaScript, so the successful entity probe does not establish readiness of all services.

**Recorded build experiment:** `cargo +stable build --locked -p moqui`, development profile/default features, Cargo 1.97.1, default compiler/linker settings. Isolated snapshots and target output were on `/tmp` tmpfs; clean means empty build artifacts, not an empty dependency-download cache. Each actual logic edit changed a numeric-flag branch in core entity REST handling and rebuilt the application. Incremental compilation remained enabled, release settings were unchanged, and the machine was a live desktop without CPU isolation. The clean-build comparison used different snapshots; the 20-edit after series reported a separate clean build of 60.20 s. Restore source-level development debug information with `CARGO_PROFILE_DEV_DEBUG=2`; that setting changes the build cache. No full loop, reload, public-API, or release-rebuild acceptance was measured.

### Example of the end-to-end acceptance test

Run the real server against a disposable runtime, not a reduced benchmark startup. Use a corpus of 300,000 lines/15–45 MB for full acceptance, and report smaller real-corpus diagnostics separately. Test empty and existing databases independently; cold-file and warm-file runs are separate results.

```rust
// Illustrative test structure; subprocess launch is inside the timer.
for mode in [EmptyDatabase, ExistingDatabase] {
    for _ in 0..20 {
        let runtime = disposable_runtime(mode); // No app or DB process running.
        let started = Instant::now();
        let server = launch_real_application(&runtime);
        let response = authenticated_database_read(&server); // Includes body arrival.
        let elapsed = started.elapsed();
        assert_eq!(response.status(), 200);
        assert!(response.contains_expected_seed_data());
        record_sample(mode, elapsed);
        assert_eq!(anonymous_read(&server).status(), 401);
        assert_durable_database_and_complete_schema(&server);
        stop_application_and_database(server);
        assert!(elapsed <= Duration::from_millis(500));
    }
}
```

Create first-start fixtures through normal timed seed loading; existing-DB samples reuse a previously seeded, stopped disposable database. Verify SQL execution during the probe, `fsync=on`, `synchronous_commit=on`, expected schema/seed contents, and no restart seed replay. Clean up even on failure. Report every sample, median, nearest-rank p95, maximum, environment, corpus, and failures; do not discard outliers or expose secrets.

## 1. Inspect schema in bulk and install it atomically

**What needs to happen:** replace per-object PostgreSQL metadata round trips with an indexed live snapshot per datasource batch. Compare the model against that snapshot, create missing objects, and refresh after table creation. Keep non-PostgreSQL behavior compatible. Use one durable schema transaction where supported, recovering tolerated DDL errors through savepoints and rolling back fatal failures.

**Criterion:** unchanged schema requires three catalog queries and no DDL, independent of entity count. All missing columns/indexes/foreign keys/FTS objects remain repairable. No partial schema installation after a fatal batch failure. Timing evidence is the combined startup comparison above; no isolated DDL-only before/after milliseconds were recorded.

```rust
// Core algorithm: bounded round trips, indexed comparison, atomic installation.
with_schema_transaction_if_supported(connection, |connection| {
    let mut catalog = read_live_catalog(connection)?; // Three bulk queries.
    let created = create_missing_tables(connection, model, &mut catalog)?;
    if created {
        catalog = read_live_catalog(connection)?;
    }
    for ddl in missing_schema_objects(model, &catalog)? {
        // Recover an allowed duplicate/lenient error before continuing.
        execute_with_savepoint_when_supported(connection, ddl)?;
    }
    Ok(()) // Commit only on success; fatal error rolls back the batch.
})?;
```

**Example regression/performance test:** check a fixture schema, reset a connection-scoped SQL recorder, check again, and assert exactly three reads/no DDL. Drop a column, ordinary/FK/FTS indexes, and foreign keys; recheck and assert the expected objects return. Force a later table-creation failure and assert earlier creation rolled back. Verify nontransactional drivers retain their existing behavior.

```rust
check_schema(&db, &model)?;
let trace = record_sql(|| check_schema(&db, &model))?;
assert_eq!(trace.catalog_queries(), 3);
assert_eq!(trace.ddl_statements(), 0);
drop_fixture_schema_objects(&db)?;
check_schema(&db, &model)?;
assert_fixture_schema_complete(&db);
assert_fatal_batch_leaves_no_partial_schema(&db);
```

## 2. Reuse XML-action macro compilation

**What needs to happen:** retain reusable macro closures within the owning JavaScript engine instead of compiling the same support modules for every service. Keep plain/debug options separate, preserve action validation, and respect engine lifetime/reload boundaries.

**Criterion:** one successful macro construction per engine per mode, unchanged generated behavior, and safe teardown. There is **no isolated before/after timing**; the startup numbers above include this optimization alongside schema changes.

```rust
// Core algorithm: cache engine-owned values, never cross engine/context boundaries.
let macros = match engine.macros.get(&debug_mode) {
    Some(saved) => restore_in_owning_context(saved)?,
    None => {
        let macros = compile_action_macros(debug_mode)?;
        engine.macros.insert(debug_mode, persist(&macros));
        macros
    }
};
generate_and_validate_actions(xml, macros)?; // Still runs for each definition.
```

**Example test:** generate the same actions repeatedly with modes off/on/off/on. Assert exact equivalence to uncached output, one construction per mode, unchanged validation failures, and independent caches in separate engines. Use construction counts, not flaky wall-clock assertions, for the regression; measure startup separately.

## 3. Load component definitions as one coherent model

**What needs to happen:** include discovered component entities/services in normal loading and startup. Resolve framework/component references together, derive schema tables from the loaded model, and load embedded entity seeds before dependent ordinary data. Preserve dependency order, configuration/filter behavior, and existing-database seed policy.

**Criterion:** no repeated framework loading or per-request definition rebuild. Startup with enabled components still owes the same ≤500 ms budget. **No isolated component-loading before/after timing was recorded**, and enabling components changes the measured corpus; do not attribute an invented speedup to this work.

```rust
// Core algorithm: discover once, resolve together, reuse the model.
let components = discover_in_dependency_order(configuration)?;
let definitions = read_framework_and_component_definitions(&components)?;
let model = merge_and_resolve(definitions)?;
check_schema(model.concrete_tables_by_datasource())?;
if normal_startup_requires_seeds() {
    load_embedded_entity_seeds(&components)?;
    load_selected_component_data(&components)?;
}
publish_shared_model(model);
```

**Example test:** a synthetic component references a framework entity, declares a view and nested service, and has ordinary data depending on an embedded seed. Start the actual host, invoke the service/read the data, then restart and verify no seed replay. Include it in the authenticated startup test, reporting the changed corpus rather than comparing unlike fixtures.

## 4. Reduce development feedback cost

**What needs to happen:** reduce avoidable debug/codegen/linking overhead and unnecessary downstream recompilation while retaining incremental builds and existing architecture. Evaluate reduced development debug information and optional fast linkers; let measurements decide. Preserve a documented full-debug option and unchanged release behavior.

**Criterion:** a small actual Rust implementation edit reaches verified updated behavior within two seconds, including build, shutdown, startup, and request. Recorded build-only numbers and their baseline limitations are above; no full edit-to-ready before/after result exists.

```rust
// Core measurement algorithm: real edits, isolated source tree, full feedback loop.
for variant in 0..20 {
    let started = Instant::now();
    save_distinct_compiled_logic_edit(&snapshot, variant)?;
    build_development_binary(&snapshot)?;
    stop_previous_application_and_database()?;
    let server = launch_development_application(&snapshot)?;
    assert_updated_authenticated_behavior(&server, variant)?;
    let elapsed = started.elapsed();
    record_sample(variant, elapsed);
    assert!(elapsed <= Duration::from_secs(2));
}
```

**Example test:** use 20 different compiled core logic edits in an isolated workspace. Compare identical snapshots with full and reduced debug information; record build and whole-loop times separately. Timestamp/comment/no-op edits do not qualify. Measure public API changes and release rebuilds separately. Reloadable XML/JavaScript/templates retain the separate ≤100 ms edit-to-updated-behavior criterion from Performance.md.

## Diagnostic benchmarks and remaining acceptance

Measure parser/model and SQL generation independently to locate costs; these do not replace startup or durable transaction tests. Recorded diagnostics used 20 release samples on the environment above:

| Diagnostic | Recorded median | Recorded p95 | Interpretation |
|---|---:|---:|---|
| Resident parse/tree construction, 21,134,945 bytes / 323,128 lines | 134.819 ms | 149.873 ms | 156.77 MB/s; inside illustrative 100–500 MB/s range |
| Warm reads plus parse/tree construction, same repeated corpus | 144.751 ms | 158.415 ms | Not cold-file startup or unique-model resolution |
| Real framework entity model, 217 entities | 4.856 ms | 5.495 ms | Includes reads/parse/merge/resolve |
| INSERT generation per statement | 0.219 µs | 0.224 µs | Text generation only |
| UPDATE generation per statement | 0.264 µs | 0.272 µs | Includes fresh predicate/value allocation |
| DELETE generation per statement | 0.179 µs | 0.182 µs | Includes fresh predicate/value allocation |
| Opt-in UPSERT generation per statement | 0.388 µs | 0.423 µs | Benchmark-local opt-in only |
| Primary-key SELECT generation per statement | 0.597 µs | 0.654 µs | Includes fresh predicate/value allocation |

**Recorded corpus/workload:** 192 XML documents, 1,625,765 bytes, 24,856 lines, 16,180 nodes, and 26,186 attributes, excluding `framework-catalog.xml` because its DOCTYPE was unsupported. Parser-scale samples used 13 copies: 2,496 documents, 21,134,945 bytes, 323,128 lines, 210,340 nodes, and 340,418 attributes. Trees remained alive until timing stopped; destruction was excluded. The real model measurement used 217 framework entities, not the repeated corpus. SQL samples used PostgreSQL `moqui.basic.Enumeration` (12 fields), three insert fields, two update fields, a primary-key predicate, and all fields for SELECT, at 100,000 generated statements per sample. INSERT/UPSERT timed text generation only; UPDATE/DELETE/SELECT included fresh predicate/value allocation. UPSERT used a benchmark-local dialect clone; production configuration was unchanged. Diagnostic medians used the upper middle of 20 sorted samples, while build medians used the conventional median; p95 used nearest rank.

These are baselines, **not before/after optimization comparisons**. Example benchmark: parse resident input while retaining trees until the timer stops; generate 100,000 statements per sample with `black_box`. Report corpus bytes/node counts, allocation scope, and exclusions. Repeating documents tests parser scale, not unique-reference resolution. SQL text generation excludes database execution, authorization, preparation, and durable commits; it establishes no production cache speedup or transactional throughput.

The separate target of **100,000 durable commits/completed business reads per second** remains unmeasured before and after. Verify it with a declared read/write mix, completed-operation counts, durable acknowledgments, full configured framework semantics, errors, and tail latency. Never substitute generated statements or row counts for that criterion.
