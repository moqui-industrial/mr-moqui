# Deterministic transaction mode (protocol v1)

Opt in before boot with `MOQUI_DETERMINISTIC=true` or the host CLI flag `--deterministic`.
Required environment:

```sh
export MOQUI_DETERMINISTIC=true
export MOQUI_DETERMINISTIC_HOOK=http://127.0.0.1:8765/transaction
export MOQUI_DETERMINISTIC_ENGINE=rust
export MOQUI_DETERMINISTIC_RUN=my-run
export MOQUI_DETERMINISTIC_LOG=/absolute/path/deterministic.log
```

`MOQUI_DETERMINISTIC_TIMEOUT_SECONDS` defaults to 120, accepts 1..3600. Connect timeout is
5 real seconds; response timeout is bounded real time. No redirects, proxies or automatic
retries. Hook URLs cannot contain credentials, query strings or fragments. Responses require
HTTP 200, JSON with nonempty `operation_id` and an in-range integer `time_epoch_ms`, or
`{"ok":true}` for completion. Bodies are capped at 64 KiB. Errors do not include response
bodies or URL contents. The append-only JSONL log is flushed after each acknowledged event;
new files have mode 0600 on Unix. Sequence and provider are process-wide, including factory
restarts. A new process starts at sequence 1; sequence is not an inter-engine matching key.

## Boundaries, time and execution

- Every newly begun framework transaction obtains an assignment synchronously before work
  (connections are lazily enlisted). Joins reuse it. Force-new/reserve transactions obtain a
  child assignment, suspend the caller and restore its timestamp on resume.
- Nontransactional manager connection work becomes an explicit transaction. Entity writes
  begin before generating defaults/stamps. Reserve connection work uses an independent
  managed transaction; the normal executor retains its previous reserve/autocommit behavior.
- Bootstrap, outer execution contexts, shutdown flushes and post-transaction hooks without
  another assigned operation use explicit coordinator operations, not a clock fallback.
  Synthetic operations enclose nontransactional work, not one atomic database transaction.
  Their normal scope exit means `committed`, unwinding means `rolled_back`.
- Artifact metadata contains only type/subtype/name, innermost first; `artifact` is stack[0].
  Empty context uses the singleton AT_OTHER/transaction/bootstrap stack. Parameters, user
  credentials, SQL and error text are not included in protocol events. Real context metadata
  is not normalized to hide different transaction topology.
- `moqui_lib::clock` supplies framework/user timestamps, entity `lastUpdatedStamp`, login,
  audit/default times, notifications, document/feed stamps and framework JavaScript `Date`
  no-argument construction/`Date.now()`. Explicit context effective-time overrides do not
  override coordinator time. Reading a logical clock without an assignment panics; it never
  returns host time in deterministic mode.
- `Instant` deadlines, elapsed/profiling time, cache eviction clocks, file modification times
  and diagnostic console/file log rotation remain real. Search-log timestamps are assigned
  at their explicit flush operation, not taken from runtime-thread diagnostic wall time.
- A process-wide reentrant lane encloses operations and outer execution contexts. Inline
  service/job queues drain FIFO only with no active or suspended transaction. Host HTTP
  dispatch runs synchronous core code on blocking tasks, then takes this lane. External
  request-pool workers also take it when entering a context; there is one request worker.
- PostgreSQL search DDL uses managed independent transactions, savepoints for the existing
  tolerated extension/duplicate-object failures, and bound logical `created_at` on registry
  writes. Entity schema DDL uses savepoints for the existing tolerated duplicate/FK errors.
  These are not new seed-data exceptions.
- Failure poisons future work. An uncertain database commit/rollback is left incomplete,
  rather than inventing a rollback acknowledgement. Completion-hook failure cannot undo an
  already committed database transaction. Destructor-only completion failures are logged
  and poison the clock/lane's future operations; they cannot return an error to their caller.

## Application entropy

The host installs `moqui_lib::random` alongside the deterministic clock provider.
Random draws call the coordinator with `version`, `run_id`, `engine`, `event="random"`,
`domain` and `length`; the response is exactly `{"bytes":"<lowercase hex>"}`.
The coordinator owns the replayable 256-bit seed and independent semantic-domain and
loading/activity streams. Requests do not open or pair transactions. Missing providers,
transport/log failures and malformed responses fail closed, without OS-entropy fallback.
Logs retain request metadata, not generated tokens.

Audited UUIDs, sequence staggering, strings, session ids/tokens, login keys, password
salts, factor/TOTP codes, generated configuration secrets, notifications, cluster ids,
data-feed ids, search document ids and persisted email Message-IDs use this source.
QuickJS service and locked-template contexts replace server-side `Math.random`.
UUID v4 bits, URL-base64 strings, 53-bit doubles and bounded integers use shared vectors.
Normal mode retains ordinary entropy. **Never enable test mode in production or a shared
deployment: generated credentials and secrets are predictable.**

Transport cryptography, temporary-directory isolation, MIME multipart boundaries,
browser/vendor randomness, arbitrary components and external providers remain outside
this adapter. Seeding is not a parity guarantee or a production replay mechanism.
The companion harness documents the algorithm and full audit in DETERMINISTIC-RANDOMNESS.md.

## Explicit limitations

- The autonomous scheduled-job timer does not run with the inline executor; startup reports
  this. Invoke named jobs explicitly with `run_job_now`. Scheduled rows are retained, not
  claimed/completed silently. `callFuture` inside a transaction is rejected instead of
  starting a concurrent transaction or joining it accidentally. Use queued async work.
- Artifact hits and sessions use explicit shutdown flushes, no background writes. Hit/log
  queues are not capped/dropped in this mode and can consume unbounded memory. Log shipping
  to supported search backends is queued until shutdown, without a shipper thread. Cluster
  sends are synchronous; heartbeats depend on the unsupported automatic scheduler.
- Default primary PostgreSQL startup is supported. OpenSearch and search read replicas are
  rejected, not silently replaced. Simulation overlays are rejected. Direct driver/client
  use outside TransactionManager (including JDBC extraction), browser hosts without a host
  provider, and third-party autonomous/native threads are not supported by this protocol.
- Managed connection closures reject raw begin/commit/rollback and conservatively reject
  SQL clock words (`now`, `CURRENT_TIMESTAMP`, `clock_timestamp`, etc.). This is a safety
  check, not a full SQL parser: even quoted clock words are rejected. Bind logical timestamp
  values. Arbitrary custom SQL, stored procedures, defaults/triggers with hidden server
  clocks and raw SQL transaction-control tricks are outside the guarantee.
- Only audited application entropy is controlled as described above; transport keys,
  host identity, external authentication providers and other external I/O are not made
  deterministic. Persisted elapsed/profiling fields remain real. Nested operations must
  end in LIFO order on their originating thread.
- Optional `MOQUI_DETERMINISTIC_OFFLINE=true` explicitly disables search/cluster/SSH and
  reports it. It is **not required** and is not used by the default-server smoke test.
- Transaction topology is unchanged where possible, but deterministic autocommit wrapping,
  Rust per-record loader transactions and independent DDL/reserve operations do not promise
  Java's topology. A strict coordinator may reject startup. No unmatched-operation, seed
  equivalence or database-copy exceptions are added.

## Evidence / reproduction

Use the stable compiler pinned by the parent harness (1.94.0) and a separate target directory:

```sh
export PATH=/home/user/.local/share/mise/installs/rust/1.94.0:$PATH
export CARGO_TARGET_DIR=/home/user/moqui/mr-moqui-deterministic-target
cargo build --locked -p moqui
cargo test --locked -p moqui-host --test deterministic
cargo test --locked -p moqui-core --lib
python scripts/test-deterministic-server.py \
  --binary "$CARGO_TARGET_DIR/debug/moqui" \
  --output "$CARGO_TARGET_DIR/server-smoke-new"
```

The focused integration test forks isolated processes and uses a real HTTP mock coordinator:
joins, nesting/suspend/resume, commit/rollback/rollback-only/drop/begin failure, uncertain
commit, timestamp binds, JavaScript dates, metadata, FIFO async drain, lane serialization,
factory restarts, append preservation, malformed replies/status/ack, bounded timeout,
missing configuration, SQL clock/boundary rejection and normal mode.

The Python smoke uses a fresh disposable PostgreSQL 18 container and a copy of **this
checkout's** runtime. It starts the actual binary with default search/cluster behavior,
loads startup data, requires `/health` HTTP 200, records the `/status` screen response and
shuts down. The current `/status` screen returns HTTP 500 for an unimplemented
`ec.user.getClientIp()` script method; the smoke records this, not a successful screen render.
It retains hook/log/database evidence
under the output directory. Its single-engine mock assigns times only: this is **not**
cross-engine equivalence evidence. Use the parent harness's strict coordinator for that.

Clock audit: production direct `Timestamp::now`, `Zoned::now` and `SystemTime::now` reads in
core were routed through the logical clock; remaining core occurrences are test-only.
Host temp-directory names and diagnostic logging remain real. SQL search found registry
`DEFAULT now()`/upsert `now()` (removed/bound when enabled), cluster `SELECT CURRENT_TIMESTAMP`
(logical branch when enabled), and replica lag `now()` (replicas unsupported). All timeout/
profiling `Instant::now()` paths remain untouched.
