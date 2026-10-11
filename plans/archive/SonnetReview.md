# MR-Moqui Plan Review: Architecture and Tool Fit

The architecture fits Moqui. A synchronous thread per `ExecutionContext`, a one-way crate cut, and owning `MNode`, `MCache` and the transaction manager are all sound. The risks are mostly not library choices.

- **Plan sentences that fail when run.** Several statements about Java or Rust behavior don't hold up under a test.
- **FreeMarker's real surface has no engine.** The templates that use it are unassigned.
- **Late proof.** The first end-to-end proof is Gate 1, after Phase 6.

I read all plans, `../moqui`, and the Bitronix checkout at `~/work/bitronix`. I also ran probes with rquickjs 0.14, rust_decimal, croner, r2d2, Groovy 5.1, QuickJS, tsc and Vue's compiler. Nothing in the repo was changed.

## Glaring issues

1. **A Rust panic in any host function kills the process.**
   - A panicking rquickjs callback ends in "panic in a function that cannot unwind" and SIGABRT (exit 134).
   - `rust_decimal` operators panic on overflow even with overflow checks off. `25-digit × 25-digit` gives "Multiplication overflowed", while `checked_mul` returns `None`. Moqui's decimal types are NUMERIC(26,6), (25,5) and (24,4).
   - Groovy `BigDecimal` division differs. `1G/3G` is `0.3333333333` (scale-10 rule) in Groovy and 28 digits in rust_decimal. mantle-usl has 920 `BigDecimal` mentions and 48 `setScale` calls.
   - **Fix:** wrap every host function in `catch_unwind` and turn the panic into a JS exception. Use checked decimal ops. Evaluate `bigdecimal` 0.4.11 as the host decimal for Java parity.

2. **"Decimals fail loudly" has holes.** I tested the exact design in QuickJS-NG.
   - `+`, `<` and `== 0` throw, as intended.
   - `a == b` between two decimals silently returns false.
   - `if (zeroDecimal)` is truthy, but Groovy treats `0.00G` as falsy. `zero || 5` returns the zero.
   - In the browser, decimals are plain strings. `"10.00" < "9.00"` is true and `"1.00"+"2.00"` is `"1.002.00"`. The compat scan only looks for arithmetic.
   - TypeGeneration's branded `string` type does not make `amount + tax` a compile error. Only `*` errors. An object brand catches `+` but not `<`.
   - **Fix:** revive package decimals as the same throwing class on the client, add `.isZero()` and `.eq()`, and lint `==` and truthiness on decimal-typed names.

3. **The global-Proxy recipe in `ContextObjects.md` doesn't work as worded.**
   - `globalThis = new Proxy(...)` still throws `ReferenceError` for unset names.
   - `Object.setPrototypeOf(globalThis, new Proxy({}, {has,get,set}))` does work. Unset names give null, `if (unset)` is falsy, and assignments land on the stack.
   - Own globals win over stack keys, not the reverse as the plan says. Helpers like `ObjectUtilities` will shadow same-named parameters.
   - `typeof unset` returns `"object"`.
   - **Fix:** put the helpers behind one namespace object.

4. **FreeMarker's real surface has no assigned engine.**
   - 57 FTL files sit outside the screen macros. 40 of them call live `ec.*`, including 549 `ec.l10n` references, `ec.entity.find`, `findRelatedOne` and `ec.service.sync`.
   - 27 inline `<text template="true">` blocks exist in components, 7 of them xsl-fo.
   - Liquid gets only a snapshot, and MiniJinja is banned on the request path, so nothing can run these.
   - Email bodies are screens. `sendEmailTemplate.groovy` renders `bodyScreenLocation` in html and text modes, and the subject and from fields use `ec.resource.expand`. I found no admin-authored FreeMarker surface for Liquid to replace.
   - `liquid` was last released in 2025-02.
   - **Fix:** `<text template="true">` becomes a QuickJS template evaluation with the live context. Make Liquid optional.

5. **No time-zone or format story.**
   - `time` 0.3 has no IANA database. It was chosen because `liquid-core` depends on it. `jiff` 0.2 works with `postgres-types` (`with-jiff-0_2`).
   - QuickJS has no `Intl`. `typeof Intl` is undefined, and `toLocaleString('de-DE')` returns `"1234.5"`.
   - Screens use Java patterns: `#,##0.00` appears 161 times and `yyyy-MM-dd` 84 times. Ties round differently: JS `(0.125).toFixed(2)` is 0.13 and Java HALF_EVEN gives 0.12.
   - **Fix:** decide the tz crate before Phase 1, because `object` bakes it in. Write one pattern spec with test vectors that run against both the Rust and browser formatters.

6. **Use `croner`, not `cron`.**
   - All 5 distinct seeded expressions parse in `cron`, but it rejects Quartz `L`, `W` and `#`.
   - Moqui uses Quartz syntax and needs `getCronDescription`.
   - `croner` 4 handles L/W/# and has `describe()`, for example "At 02:00, on Monday." It rejects `0/15`, so normalize `0/15` to `0-59/15`. The seed data uses `0 0/15 * * * ?`.

7. **Inconsistencies.**
   - EmbeddedPostgres says "no extra extensions", but SearchFacade needs `gin_trgm_ops` (`pg_trgm`). The extension is in the theseus 18.6.0 archive and needs `CREATE EXTENSION`.
   - The pre-commit data feed changes Java behavior. `DataFeedSynchronization.beforeCompletion()` is empty and the feed runs in `afterCompletion`. Add it to the README's redesign list.
   - Of the migration counts, 687 script blocks and 23 ranges reproduce in mantle-usl. "4,063 entity-find" does not. I get 1,084 `<entity-find` and 2,166 with `-one` and `-count`.

## Transaction manager

I agree with the scope: one-phase, local, with your own pool. Bitronix is 23.2k lines in `~/work/bitronix` (14.0k excluding blanks and comments). That checkout's pom says 3.0.0-SNAPSHOT, so I can't confirm the plan's 4.0.1 or 17,800 figures. Code lines per package:

| Port | Skip |
|---|---|
| root, tx state machine and sync scheduler (1,648) | twopc 663, journal 979, recovery 595 |
| `resource/common`, pool and holder states (1,193) | gui 1,016, jms 1,405, jndi 186, osgi 117 |
| `timer`, timeout task and pool shrink (327) | jdbc 927, jdbc/proxy 1,321, lrc 345 |
| `utils`, `Scheduler` and clock only | rest of utils |

Roughly 3k Java lines carry the behavior you want [INFERENCE].

**Specific gaps:**

- **Nested checkouts are on hot paths.**
  - Java call sites: sequence banks on every create without a PK (`EntityFacadeImpl:2001`), runtime DDL, two service semaphores, three failed-login counters, login keys, job-run create, notifications, and email status updates.
  - With "parked still counts against max, no spare", N threads in transactions that each need a second connection stall the pool until `pool-time-wait`.
  - The Java conf comment sizes the pool at about 120 for 100 request threads, 16 workers and a few spare. The plan starts from the default of 50.
  - **Fixes:**
    1. Take sequences from a native PG sequence or an autocommit `UPDATE…RETURNING`. This also removes the per-`seqName` process lock and the TODOs in `dbSequencedIdPrimary`.
    2. Add a reserve tier for side-channel work.
    3. Check out the connection lazily at the first statement, not at `begin`.
    4. Enforce `request_threads ≤ pool_max − reserve`.
  - Java's `runRequireNew` actually spawns a thread (`requireNewThread = true`). The plan's same-thread suspend is right, since an `!Send` EC removes `useExecutionContextInThread`, which Java's own comment calls dangerous.
- **Sync ordering.**
  - Bitronix runs `beforeCompletion` in reverse registration order and `afterCompletion` forward, all at one position.
  - `TransactionCache` registers lazily, and the feed registers on the first write. With a pre-commit feed, `[cache, feed]` runs reversed, so the feed runs before the cache flush.
  - Use explicit phases. `Scheduler` already supports positions.
- **Timeouts have no enforcement mechanism.**
  - Bitronix has a `TransactionTimeoutTask`. Java adds `use-statement-timeout`, which derives a per-statement timeout from the remaining time.
  - Add one per-transaction deadline. Feed it to QuickJS `set_interrupt_handler` (tested: it stops `while(true)`, and `set_memory_limit` stops runaway allocation). Also feed it to `statement_timeout`, and to a `CancelToken` fired from the timer thread.
- **"Exactly one resource enlisted" would be a runtime assert.** Conf allows several groups: `transactional#clone1`, `logging`, `nontransactional`, and `useClone` is in the host method list. Make reads on a clone non-enlisted, and make two writable groups on different databases a startup config error.
- **The pool justification is wrong.** An r2d2 guard is `Send`, and parking means moving the guard into the Tx record. I tested it: with 2 parked and max 2, a third checkout times out and `state()` reports idle and total. An own pool is still fine. The reason is a reserve tier, `parked` and wait gauges, and `DbDriver`, and it should take about 150 lines.
- **A statement-cache regression versus Java.**
  - Java sets `setPreparedStatementCacheSize(100)`.
  - The plan says "`postgres::Client` already prepares". `query(&str)` prepares a fresh named statement and closes it each call. tokio-postgres's own docs for `query_typed` call this "three round trips".
  - This is harmless on a Unix socket and a regression on external Postgres. Use a per-connection LRU keyed on SQL, or `query_typed`, which exists on the sync client.
- **Failed statements poison a PG transaction.**
  - Java conf sets `never-try-insert="true"` for Postgres. mantle-usl has 11 `catch(` and 5 `ignore-error="true"`.
  - Phase 4 lists `useTryInsert` but no policy. Use a savepoint around statements whose failure is handled, or `ON CONFLICT`.
- **Carry these Java facade behaviors on purpose:** `use-connection-stash` (what "one connection shared" reinvents), `use-lock-track`, `destroyAllInThread`, and `RollbackInfo` (cause plus location).

## Fit by piece

| Piece | Verdict |
|---|---|
| rquickjs / QuickJS-NG | Keep. Runtime plus full context costs 77 µs, and compiling a 25-line function in a warm context takes 15.7 µs. Watch panic=abort, the missing `Intl`, and recompiling per EC. |
| MiniJinja codegen | Keep. Themes override macro sets by `macro-template-location`, so a text layer matters. |
| Liquid | Demote to optional (issue 4). |
| Embedded PG | Keep. Add the dev snapshot workflow (Java has `saveDb` and `loadSave`, and the archive ships `pg_dump`, `pg_restore`, `pg_upgrade` and `pg_basebackup`). Add a major-version upgrade path and orphaned-postmaster handling after SIGKILL. |
| `rust_decimal`, `time`, `cron` | Replace or augment (issues 1, 5, 6). |
| axum | Keep. An "owned response" buffers CSV and XLSX exports, so add a bounded stream or spool. |
| PDF | Needs a decision now. 24 SimpleScreens files reference xsl-fo (invoices, checks, statements, pick and pack). The Rust `fop` crates are 0.1.2 with about 600 downloads, and I did not evaluate their quality. Options are a FOP sidecar (exact, keeps templates), Typst (needs template rewrites), or headless Chromium. |
| `moqui-poi` | Small, 1,140 lines. `calamine` and `rust_xlsxwriter` are mature. |
| Entity model, SQL renderer, dialects | These meet your own cut rule (no framework type in the API). Extracting them from `moqui-core` would cut compile time and isolate the SQL-diff-testable part [INFERENCE]. |

## Making JS-in-screens real

1. **Precompile static qvt.**
   - The plan ships `vue.global.prod.js` (168 KB, 61 KB gzipped). Compiling in the browser uses `new Function` and `with(_ctx)`, which needs `unsafe-eval` and blocks a strict CSP later. The runtime-only build is 109 KB, 41 KB gzipped.
   - A static template is cacheable, so compile it once on the server. The Vue compiler runs in QuickJS (3.5 ms for a small template). `prefixIdentifiers` output has no `with` in the Node build, but the browser build rejects it with error 48.
   - I did not run the Node build inside QuickJS. Merged qvt keeps browser compile.
2. **Add a permanent `moqui lint` with `oxc_parser`.**
   - Parse every expression at load. Groovy leftovers (ranges, `?:`, GStrings, closures) then fail at load, not at run.
   - The same AST powers the static-screen compat checks (no `ec.entity`, `sri`, or decimal operators) instead of a string scan.
   - Until Phase 6 can run JS, this is the only early migration feedback.
3. **Port the client to Vue 3 and Quasar 2 now, against Java Moqui.** The client is independent of the server. Today the UI path stacks four changes: the Rust server, JS expressions, Vue 2→3 and Quasar 1→2 (`WebrootVue.qvt.js` is 2,762 lines, `qvt.ftl` 2,253, `MoquiLib` 644), and server-static.
4. **Add a headless-browser smoke test per mode.** Gate 3 only checks that the Vue compiler can compile the output, not that it renders. The same harness can run the compat report.

## Worth adding

- **Spikes before committing Phases 3 and 6.** These are throwaway, not gates:
  1. Load the mantle-udm entity definitions (422 entities) onto embedded PG.
  2. AI-migrate about 30 representative mantle-usl services and run them.
  3. Run a pool stress test with nested force-new plus sequences.
  4. Render one real SimpleScreens screen as static qvt with decimals.
- **Migration oracle.** mantle-usl has 10 Spock tests. Keep Java runnable for parity diffs.
- **Legacy password hashes.** The seed data has 16 users with `passwordHashType="SHA"`, and the default conf is SHA-256 salted via Shiro `SimpleHash`. The plan is argon2 only, so no existing database can log in. Add verify-legacy then rehash on login, keeping `passwordHashType` and `passwordBase64`.
- **`expand()` on database-stored strings is code execution.** `EmailTemplate.subject` and `fromAddress` go through `ec.resource.expand`. Make data-sourced strings path-lookup only (`${a.b.c}`), and keep full JS for component-authored XML. Audit seed data first.
- **Session persistence.** Jetty's `FileSessionDataStore` keeps sessions across restarts, and in-memory sessions would log everyone out on every restart or deploy. Persist a snapshot behind the same struct.
- **Async overflow.** Java's queue is 65,535 (`worker-queue`). A bounded queue that rejects on full inside `afterCompletion` loses work, and `callFuture` joins from pool threads can starve it. Spill to a DB-backed queue (`ServiceJobRun` exists) for fire-and-forget calls.
- **Artifact stack as `tracing` spans.** Hit bins, slow-statement logs and pool metrics then become subscribers rather than custom code. This is a design suggestion.

## Not verified

- `bigdecimal` behavior.
- The Node Vue compiler inside QuickJS.
- PG refusing to run as root in containers.
- PG 17 `transaction_timeout`.
- rquickjs bytecode caching.
- `Intl` `roundingMode` support in browsers.
- The plan's dependency claim that `liquid-core` pulls in `time`.