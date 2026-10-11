# Recommended Changes to the MR-Moqui Plans

Summary of actions from `TempReview.md`, grouped by priority. Each item is a change to a plan document, a decision to make, or work to add to the phase list.

## 1. Correct the plan text (cheap, before implementation)

- [x] `ContextObjects.md`, global-Proxy recipe: the mechanism is `Object.setPrototypeOf(globalThis, new Proxy({}, {has,get,set}))`, not installing a Proxy on `globalThis` (which still throws `ReferenceError` for unset names). Document that own globals win over stack keys, that `typeof unset` returns `"object"`, and put host helpers behind one namespace object so they never shadow parameters.
- [x] `Architecture-Proposal.md`, "a missed one must not fail quietly": correct to what the tests show. `+`, `<`, `== 0` throw on host decimals; `a == b` between two decimals silently returns false; a zero decimal is truthy in JS while Groovy treats `0.00G` as falsy (`zero || 5` returns the zero). In the browser, decimals are strings: `"10.00" < "9.00"` is true, `"1.00"+"2.00"` is `"1.002.00"`.
- [x] `TypeGeneration.md`: a branded `string` type does not make `amount + tax` a compile error in tsc (only `*` errors). Use an object brand (catches `+` and `*`) and add a lint rule for relational operators and truthiness on `Decimal`.
- [x] `EmbeddedPostgres.md`: "No extra extensions" is wrong for `SearchFacade`. `gin_trgm_ops` needs `CREATE EXTENSION pg_trgm` (present in the theseus 18.6.0 archive; the plan's search tables depend on it).
- [x] `README.md` redesigns: add the pre-commit data feed. Java's `DataFeedSynchronization.beforeCompletion()` is empty; the feed runs in `afterCompletion`.
- [x] `Architecture-Proposal.md`: re-measure the migration counts. 687 script blocks and 23 ranges reproduce in mantle-usl; "4,063 `<entity-find>`" does not (1,084 `<entity-find`, 2,166 with `-one` and `-count`).
- [x] `Crate-Layout.md`/`EmbeddedPostgres.md`: drop "postgres::Client already prepares statements". `query(&str)` prepares and closes a named statement per call — tokio-postgres documents this as three round trips. Java's pool caches 100 statements per connection; keep a per-connection LRU keyed on SQL or use `query_typed`.
- [x] `Implementation-Phases.md`: state the failed-statement policy. PG poisons a transaction after any error (Java sets `never-try-insert="true"` for Postgres; mantle-usl has `catch(` and `ignore-error="true"` sites). Use savepoints around handled failures or `ON CONFLICT` upserts.

## 2. Transaction manager design

- [x] Eliminate nested checkouts on hot paths. Java forces new transactions for sequence banks (every create without a PK), runtime DDL, two service semaphores, failed-login counters, login keys, job-run creation, notifications and email status. With "parked counts against max, no spare" this stalls the pool. Take sequences from a native PG sequence or an autocommit `UPDATE…RETURNING`, add a reserve tier for side-channel work, check out lazily at the first statement, and enforce `request_threads ≤ pool_max − reserve` (Java sizes ~120 connections for 100 request threads plus workers; the plan starts at the default 50).
- [x] Specify sync ordering explicitly. Bitronix runs `beforeCompletion` in reverse registration order. `TransactionCache` registers before the feed, so a pre-commit feed would run before the cache flush. Use `Scheduler`-style positions/phases instead of registration order.
- [x] Define timeout enforcement: one per-transaction deadline fed to QuickJS `set_interrupt_handler` (verified it stops `while(true)`; `set_memory_limit` stops runaway allocation), to `statement_timeout`, and to a `CancelToken` fired from the timer thread. The plans currently keep `transaction-timeout` with no mechanism.
- [x] Replace "assert exactly one enlisted resource" with a startup config error for two writable groups on different databases, and make clone (`useClone`) reads non-enlisted. Conf allows `transactional#clone1`, `logging`, `nontransactional`.
- [x] Fix the pool rationale. Stock pools do model parking (an r2d2 guard is `Send`; parking is storing the guard in the Tx record — verified with a live test). Justify an own pool by the reserve tier, `parked`/wait gauges, and `DbDriver`, not by parking.
- [x] Carry these Java facade behaviors deliberately: `use-connection-stash`, `use-lock-track`, `destroyAllInThread`, `RollbackInfo` (cause + location).
- [x] Add a panic boundary: every host function wrapped in `catch_unwind`, converting panics to JS exceptions. A panic in an rquickjs callback aborts the process (verified, SIGABRT); `rust_decimal` multiplication panics on overflow even with overflow checks off. Use checked decimal ops throughout; evaluate `bigdecimal` 0.4.11 as the host decimal for Java scale/rounding parity (Groovy `1G/3G` is `0.3333333333`, scale-10 rule; mantle-usl uses 920 `BigDecimal` references and 48 `setScale` calls).

## 3. Templating and scripts

- [x] Assign an engine to FreeMarker's actual surface. 57 non-macro FTL files (40 call live `ec.*`, 549 `ec.l10n` references) and 27 inline `<text template="true">` blocks (7 xsl-fo) have no runner under the plan. Make `<text template="true">` a QuickJS template evaluation with the live context, matching what `template="true"` means today.
- [x] Make Liquid optional, not the email/print engine. Email bodies are screens (`sendEmailTemplate.groovy` renders `bodyScreenLocation` in html and text modes); `liquid` last shipped 2025-02. Keep the snapshot-based Liquid path only if admin-authored templates actually appear.
- [x] Constrain `ec.resource.expand` on database-stored strings (`EmailTemplate.subject`, `fromAddress`) to path-lookup only (`${a.b.c}`); full JS stays for component-authored XML. This is code execution on data today and the plan carries it forward.

## 4. Time, format, and scheduler

- [x] Decide the time-zone crate before Phase 1, because `moqui-lib`'s `object` module bakes it in. `time` 0.3 has no IANA database (chosen because `liquid-core` depends on it); `jiff` 0.2 works with `postgres-types` (`with-jiff-0_2`).
- [ ] Specify the Java pattern dialect once (`#,##0.00` appears 161 times, `yyyy-MM-dd` 84 times) with test vectors run against both the Rust and browser formatters. Ties differ today: JS `(0.125).toFixed(2)` is 0.13, Java HALF_EVEN gives 0.12. QuickJS has no `Intl`, so the server formatter is Rust work either way.
- [x] Swap `cron` for `croner` 4. `cron` parses all 5 distinct seeded expressions but rejects Quartz `L`/`W`/`#`, which Moqui jobs use, and has no `describe()` for `getCronDescription`. `croner` has both; normalize `0/15` to `0-59/15` (the seed data uses `0 0/15 * * * ?`).

## 5. Decisions to make now

- [x] PDF strategy. 24 SimpleScreens files reference xsl-fo (invoices, checks, statements, pick and pack). The Rust `fop` crates are 0.1.2 (~600 downloads, not evaluated). Choose: FOP sidecar (exact, keeps templates), Typst (template rewrites), or headless Chromium. This is Phase 12 in the plan but gates the FO template migration.
- [x] Client precompilation for `qvt`. Static templates are cacheable; compile them server-side and ship the runtime-only Vue build (41 KB gzip vs 61 KB), removing the `new Function`/`with(_ctx)` need for `unsafe-eval`. Merged qvt keeps browser compilation.
- [x] Replace the string scan with `oxc_parser`-based checks in both the compat report and a permanent `moqui lint` that parses every expression at load (Groovy leftovers then fail at load, not at run).

## 6. Work to add to the phase list

- [x] Throwaway spikes before committing Phases 3 and 6: load mantle-udm's 422 entity definitions on embedded PG; AI-migrate ~30 representative mantle-usl services and run them; pool stress test with nested force-new plus sequences; render one real SimpleScreens screen as static qvt with decimals.
- [x] Gate 3: add a headless-browser smoke test per mode. The gate currently checks that the Vue compiler accepts the output, not that it renders. The same harness can run the compat report.
- [x] Migration oracle: keep Java Moqui runnable for parity diffs (mantle-usl's 10 Spock tests).
- [x] Legacy password hashes: verify-then-rehash on login. Seed data has 16 users with `passwordHashType="SHA"` (Shiro `SimpleHash`, salted SHA-256 in default conf); the plan is argon2 only, so no existing database can log in.
- [x] Session persistence: keep a snapshot behind the session struct. Jetty's `FileSessionDataStore` survives restarts today; in-memory sessions log everyone out on every deploy.
- [x] Async overflow: Java's `worker-queue` is 65,535. A bounded queue that rejects on full loses work in `afterCompletion`, and `callFuture` joins from pool threads can starve it. Spill fire-and-forget work to a DB-backed queue (`ServiceJobRun` exists).
- [x] Dev/ops: a `saveDb`/`loadSave` equivalent (the theseus archive ships `pg_dump`, `pg_restore`, `pg_upgrade`, `pg_basebackup`), a major-version upgrade path for the embedded cluster, and orphaned-postmaster handling after a hard kill.
- [x] Optional, low cost: extract the entity model/SQL renderer/dialects into their own crate (they meet the plan's own "no framework type in the API" rule) and map the artifact stack onto `tracing` spans so hit bins and metrics become subscribers.
