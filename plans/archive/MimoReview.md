# Mimo Review — MR-Moqui Plans

Review of the plan set from an application architecture and tool/task fitment perspective, with the review decisions recorded. Reviewed 2026-10-12: every file in `plans/`, the workspace `Cargo.toml` / `Cargo.lock`, and the Java framework at `../moqui` (framework/src surveyed for subsystem inventory and known pain points).

This builds on `plans/archive/SonnetReview.md`. The current plans already absorb that review's recommendations (checked items in `plans/archive/SonnetRecommendations.md`): the `setPrototypeOf` proxy recipe, checked decimal ops with documented semantics, `croner` 4, `jiff`, the reserve connection tier, fixed synchronization phases, statement timeouts, savepoints around handled failures, the spikes, the parity harness, and verify-legacy password rehashing. This review does not repeat those.

Sections: [Verdict](#verdict), [How the pieces fit](#how-the-pieces-fit), [Decisions](#decisions) (configuration keys, `runtime/.env`, generated secrets, xlsx tooling), [Tool and task fitment](#tool-and-task-fitment), [Remaining risks](#remaining-risks), [What else is worth including](#what-else-is-worth-including-from-moqui), [Follow-ups](#follow-ups).

## Verdict

The architecture is coherent and the tool choices are, with few exceptions, the right ones. The design has a consistent spine: operational state lives in the database, evaluation lives in one engine (QuickJS), the process edge lives in the host, and Moqui-own code exists only where the library ecosystem is genuinely wrong-shaped (MNode, MCache, the transaction manager). That spine resolves most of the Java framework's structural debt (JTA thread-local guess-and-repair, FTL-to-Groovy compilation, reflection-loaded plugins, Hazelcast) instead of porting it. Residual risk concentrates in behavioral parity (decimal semantics, transaction-cache overlay and iterator merge) and unmeasured resource costs (per-connection runtimes, per-thread compiled JavaScript, embedded Postgres RSS). Both are cheap to probe early; the spike list is the right place.

## How the pieces fit

The JavaScript-everywhere decision is load-bearing across five documents and they agree: `ContextObjects.md` defines four trust environments over one runtime, `Architecture-Proposal.md` defines the renderers and code generation, `QvtServerStatic.md` consumes the unwrap projection, `TypeGeneration.md` types it, and `Implementation-Phases.md` adds `moqui lint` (oxc) plus a compatibility report as gate checks. JS-in-screens is not a bolt-on; it is what makes server-static qvt, client expression evaluation, and the locked-down template sandbox possible at all.

The transaction story fits the same way. Fixed-phase synchronizations, the connection stash, the reserve, and the pending-feed-row-before-commit form one design — a transactional outbox that replaces Java's empty `DataFeedSynchronization.beforeCompletion` plus fire-and-forget `afterCompletion` (`EntityDataFeed.groovy`, `ServiceEcaRule.groovy:157-170`), and gives the "no XA" claim teeth:

```mermaid
sequenceDiagram
    participant S as Service/entity write
    participant TX as Tx manager (core)
    participant DB as Enlisted connection
    participant P as Service worker pool
    participant Q as SearchBackend
    S->>TX: begin (no checkout, deadline recorded)
    S->>DB: first statement enlists + stashes connection
    TX->>TX: phase 1: transaction-cache flush
    TX->>DB: phase 2: insert DataFeedPending (same tx)
    TX->>DB: commit → connection returns to pool
    TX->>P: phase 3: afterCompletion submissions
    P->>Q: assemble document, index, delete pending row
    Note over P,Q: index failure leaves the row;<br/>IndexDataFeedDocuments drains it
```

Cross-resource work is an outbox or a SECA; `PREPARE TRANSACTION` never appears. The cluster design (DB membership plus post-commit fan-out) and DB sessions (`sessionRevision` optimistic merge on a reserve connection) extend the same rule: everything that must survive a crash is a row — with the exception of key material, which this review moves to configuration (Decision 1).

One interaction to enforce in code, not just at startup: `force-new` park budget 1 + reserve tier + `(request cap + service cap) × 2 <= pool_max - reserve` is what makes lazy enlistment safe. The Java hot paths that nest checkouts (sequence refill, semaphores, login counters, job rows — `EntityFacadeImpl:2001` and others) all move to the reserve in the plan. Assert the invariant at pool-checkout time; runtime-added threads (jobs, WebSockets) must not be able to violate it silently.

## Decisions

### 1. Cluster and session keys are configuration, not database rows

Key material moves out of the database and into the known `entity_ds_crypt_pass` pattern: a `<default-property>` in `MoquiDefaultConf.xml` with a `CHANGEME` default and `is-secret="true"`, referenced as `${name}` from the conf, settable by config file or environment. Rationale: keys stored separate from the data, blast radius of database read access reduced, and one established pattern instead of a new key-distribution mechanism (`../moqui/framework/src/main/resources/MoquiDefaultConf.xml:67-69` is the reference; resolution order today is system property, then env var, then the conf `value` — `ExecutionContextFactoryImpl.groovy:407-442`, `SystemBinding.getPropOrEnv`).

What this replaces in the plans: the `moqui.server.HttpSession` HMAC key row, the cluster client-certificate row (the pair that `plans/Cluster.md` had the first instance generate into the database), and the SSH host key row in `plans/Ssh.md`. The keys stay distinct from each other.

Proposed property names (settle on final names at implementation; follow the existing prefix convention):

| Property | Purpose | Shipped default | When unset |
|---|---|---|---|
| `session_cookie_key` | HMAC-SHA256 key for the session-cookie signature | `CHANGEME` sentinel | Generated (Decision 3) |
| `cluster_tls_cert` | Client certificate for the cluster endpoint (PEM, base64 DER on one line) | `CHANGEME` sentinel | Generated with `cluster_tls_key` |
| `cluster_tls_key` | Private key for that certificate | `CHANGEME` sentinel | Generated with `cluster_tls_cert` |
| `entity_ds_crypt_pass` | Field encryption key (existing) | `MoquiDefaultPassword:CHANGEME` | Generated (Decision 3) |
| `entity_ds_crypt_pass_old` | Rotation input via `decrypt-alt` (existing) | unchanged | Left alone; migration input only |
| `ssh_host_key` | SSH server host key (when the `plans/Ssh.md` server is built) | `CHANGEME` sentinel | Generated (Decision 3) |

Detail decisions:

- **PEM storage.** `.env` values are single-line (see Decision 2), so PEM material is stored base64-encoded DER (PKCS#8 key, X.509 certificate) and decoded at load. A deployment that prefers files can point the properties at content loaded from its own secret mount; the properties hold values either way.
- **Cluster consistency.** Session cookies and cluster mTLS are shared across instances, so clustered deployments must provision identical values on every instance (config management, a mounted `runtime/.env`, or environment injection from a secret store). Per-instance auto-generation (Decision 3) is for single-instance and development use. To keep the database's coordination value without storing keys: the member row gains key fingerprints (SHA-256 of the HMAC key, certificate fingerprint) — not the keys — and a member whose fingerprint differs from the live peer set fails startup with a clear message. Recommended, cheap, and it catches the copy-paste provisioning failure that would otherwise show up as mysteriously invalid cookies.
- **`is-secret` carries forward.** Explicit `is-secret="true"` on each property, and the Java name heuristic (`pass`, `pw`, `key`) as a backstop. Secret values are never logged (the Java log lines already say "Setting secret property X"), never included in conf dumps or `/actions` responses (`../moqui/framework/test/test_a01_more.py:375-417` asserts `crypt-pass` never appears in responses — keep that test pattern).
- **Rotation.** `entity_ds_crypt_pass_old` plus `decrypt-alt` remains the key-migration path; auto-generation fills only the primary. The session-cookie key can rotate freely (unknown MAC = unknown session = logout). The cluster cert rotates by provisioning the new pair on all members (the fingerprint check makes a partial rotation fail loudly).

### 2. `runtime/.env` support

A `.env` file in the root of the runtime directory — a known filesystem location — is loaded at startup and treated like environment variables. It supports any property defined in `MoquiDefaultConf.xml` or the other conf files (effectively any name resolvable through `${...}` expansion).

- **Format.** `NAME=VALUE` one per line, `#` comments, optional surrounding quotes stripped, single-line values. A minimal parser in `moqui-lib`'s `env` module. Not a general dotenv library dependency.
- **Precedence.** Real environment variable beats `.env` entry beats conf `value` (the standard dotenv rule; matches the Java order of system property / env var / default with `.env` sitting alongside env vars and below them). Any property can come from any of the three.
- **File hygiene.** Unix permissions 0600; gitignored with the rest of runtime-local files; never included in `save-db` dumps. **Backups:** if `entity_ds_crypt_pass` is lost, encrypted field values are unrecoverable, so `runtime/.env` (or the provisioned secret store) is part of the instance backup set. This goes in the deployment doc with the properties below.
- **Generated writes.** Auto-generation (Decision 3) appends to this file and never rewrites existing entries, with a header comment noting the file holds secrets and must be kept.

### 3. Generate unset secrets instead of shipping them

When a secret property (the `CHANGEME` sentinel values above) has not been set — still the shipped default after environment, `.env`, and config resolution — generate a secure random value at startup and save it to `runtime/.env`, then use it. This replaces the Java behavior of running with `MoquiDefaultPassword:CHANGEME` and telling operators to override it (`SecurityCryptoTests.groovy:80-84` records that shipped default).

- **Sentinel rule.** A shipped default whose value ends in `CHANGEME` marks the property as generate-on-unset. Non-secret defaults (for example `entity_ds_user=sa`) are never generated.
- **Scope: generated keys vs external credentials.** Auto-generation applies only to keys where Moqui defines both ends: field-encryption pass(es), the session-cookie key, the cluster certificate pair, the future SSH host key. Credentials of external systems — database password, SMTP mailbox password (seed data), OpenSearch password — cannot be generated (the other end would not match) and belong in the always-set documentation below with fail-fast validation where checkable at startup.
- **Failure behavior.** If a value must be generated and `runtime/.env` is not writable, fail startup with an actionable error. Never run on a regenerating key: it would silently invalidate sessions and encrypted data on every restart.
- **Generation.** 32 bytes from the OS CSPRNG for symmetric keys, base64-encoded; a self-signed certificate with a long validity and the cluster member identity for `cluster_tls_*`. First startup on an empty runtime therefore "just works" for single-instance and dev, and produces a `.env` an operator can copy to sibling instances.

### 4. xlsx tooling: `calamine` + `rust_xlsxwriter`

`moqui-poi` is small (~1,140 lines) but its feature is heavily used by real applications (spreadsheet import feeds and export links in mantle and SimpleScreens). The Rust ecosystem here is mature and permissively licensed: `calamine` for reading (xlsx/xls/csv/ods) and `rust_xlsxwriter` for writing (formats, autofilter, the pieces export links need). Recommendation: pull xlsx import/export into scope as an ordinary `moqui-host` (read) / `moqui-lib` (write) tool on these crates rather than leaving it to the phase 12 size-weighing — with these dependencies the binary-size argument that kept `moqui-poi` out of `moqui.war` does not apply. The same logic recommends `image` (resize/thumbnail) for `moqui-image` when that area is reached.

### Properties to always set for a deployment

The configuration documentation (deployment guide when it exists; this table until then) lists, by config file, env var, or `runtime/.env`:

| Property / area | Why | If unset |
|---|---|---|
| `entity_ds_db_conf`, `entity_ds_host`, `entity_ds_port`, `entity_ds_database`, `entity_ds_user`, `entity_ds_password` | External database credentials (when not using embedded Postgres) | Embedded Postgres default for dev |
| `entity_ds_crypt_pass`, `entity_ds_crypt_pass_old` | Field encryption; **back up with the instance** | Generated to `runtime/.env` |
| `session_cookie_key` | Cookie signing; identical on all cluster members | Generated to `runtime/.env` |
| `cluster_tls_cert`, `cluster_tls_key` | Cluster mTLS; identical on all cluster members | Generated to `runtime/.env` |
| `webapp_http_port`, `webapp_https_*`, `webapp_allow_origins`, `webapp_client_ip_header`, `webapp_status_ips` | Edge behavior behind the reverse proxy | Conf defaults |
| `search-facade` cluster credentials | When the OpenSearch backend is selected | Postgres default backend needs none |
| Email mailbox credentials | `EmailServer` seed data (not conf): change `EMAIL_CHANGEME` | Seeded dev values |
| `ssh_host_key` | When the SSH server is built | Generated to `runtime/.env` |

## Tool and task fitment

| Choice | Verdict | Note |
|---|---|---|
| Own transaction manager, Bitronix as behavior reference | ✅ Right | No Rust crate is this thing (the pools are connection lifecycle only; the distributed-transaction crates are network coordinators). Take the behavior: transaction state machine, `Scheduler` phases, timeout task — roughly 3K Java lines. Skip the XA journal, crash recovery, and JMS as planned. |
| Own pool (not r2d2/deadpool) | ✅ Right | The justification is correct as written now (reserve tier, parked/wait gauges, `DbDriver`), not "parking is impossible" — an r2d2 guard is `Send` and parking works. ~150 lines. |
| `MNode`, `MCache` ports | ✅ Right, two caveats | Same pattern as Java: wrong-shaped libraries. Do not hand-roll the concurrent map under MCache — `dashmap` or `scc` back it while the port covers the semantics. The XML tokenizer is security-sensitive (entity references, CDATA, encodings); `quick-xml` under the `MNode` API is worth a second look, or corpus round-trip plus fuzz tests before phase 2. |
| rquickjs / QuickJS-NG | ✅ Right | Correct size/startup tradeoff against V8; V8 as a later replacement (not a second engine) is the right shape. See risk 2. |
| `postgresql_embedded` as the default database | ✅ Right | Bigger win than it looks: dev and CI run the production dialect, so `text-fts`, `date_bin` buckets, `pg_trgm`, and the `search` schema are exercised before deploy. The pglite rejection reasoning is sound. |
| Sync `DbDriver` boundary | ⚠️ Acceptable — measure | Each pooled `postgres::Client` carries its own `current_thread` runtime and pays a `block_on` pump per query; a pool of 90 is 90 runtimes. It contains async to the host and keeps core futures-free, which is worth it, but it is the least-validated cost in the design. "Panics if built on a thread already inside a runtime" is a `DbDriver` contract invariant — document it and debug-assert it. |
| SearchFacade trees over Elastic JSON | ✅ Best decision in the set | Query/aggregation trees let Postgres be Postgres (`date_trunc`/`date_bin`, `EXISTS` over `jsonb_array_elements`, exact `count(distinct)`) instead of emulating Lucene. The two parity risks are named (quarter/week bucket offsets, `CountDistinct` exact vs approximate). |
| axum edge + blocking core | ✅ Right | But see risk 1 (response streaming). |
| `chromiumoxide` + external Chromium | ✅ Right | Print is a batch job; one long-lived browser with a page per document fits. `pdf-servo` carries real maintenance weight for an edge case — consider demoting it from planned feature to "investigation if a customer needs it". |
| `jiff` 0.2 | ✅ Right | IANA database included, `with-jiff-0_2` works, one clock story across core, `croner`, and the drivers. |
| `rust_decimal` | ⚠️ Acceptable | Methods plus lint plus golden semantics is workable, but Java `BigDecimal` is arbitrary-precision and never overflows. See risk 3. |
| `croner` 4, `oxc_parser`, `argon2`, `lettre`, `totp-rs`, `pulldown-cmark`, `csv` | ✅ Right | All fit the task; oxc pin churn is the only maintenance note. |
| `huskarl` (SSO) | ✅ Fine | Niche crate, but staged ahead of the feature with `oauth2`/`jsonwebtoken` documented as fallback. Re-verify crate health when the flow is built. |
| `russh` / `russh-sftp` | ✅ Fine | Watch the `rsa` 0.10.0-rc upstream pin. |
| Hand-rolled Java-pattern formatter | ✅ Acceptable | No crate matches the Java pattern subset; shared test vectors across server and browser (already planned) are the safeguard. |
| `calamine` + `rust_xlsxwriter` (Decision 4) | ✅ Add | Replaces `moqui-poi`; mature, permissive, and the size argument for keeping it out does not hold in Rust. |

## Remaining risks

1. **Owned responses vs large output.** `sendTextResponse`/owned response buffers CSV/XLSX exports, file downloads, and PDF bytes in memory. Specify now: spool-to-temp-file or a bounded streaming arm in `WebFacade`. Same for multipart uploads — Java's temp cleanup is already broken (`WebFacadeImpl.groovy:1436`), and retrofitting streaming bodies into an owned-body design later is painful.

2. **Per-thread JavaScript compilation duplication.** Each `ExecutionContext` thread owns a QuickJS runtime; generated action/template functions compile per runtime. With ~100 request and worker threads and the whole mantle-usl definition set that is compile CPU at first touch per thread plus duplicated function memory. QuickJS-NG can serialize compiled bytecode — a shared compiled-function cache (compile once, load bytecode per context) is a natural optimization; put it in the plan as an explicit later step and measure it in the phase 6 spike.

3. **Decimal overflow becomes a script exception.** Checked operations returning errors is right for safety, but it is a semantic divergence: intermediate products that worked in Java's `BigDecimal` will now throw. Put big-number cases (multi-digit multiplication, `setScale` up, `divide` at scale 10) in the parity harness so diffs are found, and keep `bigdecimal` 0.4.x as the named fallback if the oracle shows widespread breakage.

4. **Cross-request state in the per-thread runtime.** Strict mode plus the global proxy keeps assignments on the context stack, but explicit `globalThis.foo = 1` or module-level state persists in the reused runtime across requests — a ThreadLocal-leak class of bug with no parity test to catch it. Define the rule (scripts must not set own globals, or `destroyAllInThread` snapshots/deletes non-builtin own globals) and add a lint check.

5. **Transaction-cache overlay and entity iterator semantics are unspecified.** "HOLD/FLUSH are a port of the Java code" inherits Java's broken edges: `EntityListIteratorImpl.java:170-177` throws on `absolute`/`relative`/`getPartialList` when tx-cache creates exist, `set`/`add` are unimplemented, and `EntityFindBase.groovy:911` silently prefers the cache over the database. The plans specify that the iterator streams and releases the connection but not how the overlay merges into a stream. Decide this explicitly — it is the sim-mode correctness story.

6. **Key sharing across a cluster** (was: keys in the database). Resolved by Decision 1: configuration-managed values with fingerprint mismatch detection. The residual is operational — provisioning identical values — which the properties-to-always-set table and the member fingerprint check cover.

## What else is worth including (from Moqui)

From the `../moqui` survey — places where Java is already known-broken or acknowledged debt, where the rewrite is the cheapest moment to fix.

**High value, low cost — do it in the phase that touches the area:**

- **EECA original-value round trips** (`EntityEcaRule.groovy:60-105`): each `get-original-value`/`get-entire-entity` rule fires a `getDbValue` find per rule per operation. Batch/prefetch the original row once per operation. Keep the rule model; fix the fetch pattern.
- **DataFeed registration** (`EntityDataFeed.groovy`): "if an entity is added to a DataDocument at runtime it won't pick it up!!!!" plus a global synchronized 5s-throttle rebuild. Replace with per-DataDocument registration invalidated on definition change. Fits the pending-row design already planned.
- **Do not port the double-execution bug**: `XmlAction.checkCondition()` runs the action twice (`XmlAction.java:78-80`). Add it to the parity harness's intentional-differences list so it is not "fixed" back in.
- **Silent stubs to define or drop**: entity-auto `update-expire`/`delete-expire`/`find`/`find-one` empty bodies (`EntityAutoServiceRunner.groovy:89-96`), the `if (false && …)` error-localization dead code (`EntityValueBase.java:1455`), the `entity-data` XML action that is still a TODO in `XmlActions.groovy.ftl`, and the `makeSearchFilter` TODOs (`FieldToFieldCondition.groovy:81`). Implement each deliberately or delete it from the XSD/API; a rewrite that preserves silent no-ops inherits the worst kind of debt.
- **NotificationMessage** (`NotificationMessageImpl.groovy`): 4+ queries per send for recipient resolution (`getNotifyUserIds`), template expansion in getters, transient global re-lookup. Split message from delivery; one recipient query. Phase 10 says "no redesign" — this specific fix is cheap and worth the exception.
- **Batch REST body semantics** (`WebFacadeImpl.groovy:975-976`): today each row runs in its own transaction as a loop accident. Make per-row transactions plus per-row errors an explicit documented `RestApi` decision.

**Medium value, aligned with work already planned:**

- **Server-side screen render cache** (`ScreenRenderImpl.groovy:839`, "server-side render cache never done"). Server-static qvt plus the template compile cache make this natural: cached rendered static fragments slot into the `screen-server-static` story.
- **Screen URL machinery untangling** (~6.5K lines across `ScreenRenderImpl`/`ScreenUrlInfo`/`ScreenForm` conflating URL parsing, default-path resolution, authz, transaction begin, and rendering — with a "lost work, best guess" comment at `ScreenRenderImpl.groovy:1147`). Layer these explicitly in the port; `ScreenUrlInfo.groovy:450` (expand-before-cache) is a per-request cost the design can remove.
- **Postgres-backed log search**: the plans make the log viewer OpenSearch-only. Logs are `tracing` spans and the default search store is a Postgres schema — indexing log events as one more document type would make LogViewer work in the default no-OpenSearch deployment. Tradeoff is write volume and retention; make it a decision rather than silence.
- **SECA tx-commit delivery**: Java runs tx-commit SECAs fire-and-forget (`ServiceEcaRule.groovy:157-170`, failures only logged). The afterCompletion-submission plus job-spill design already fixes this; name it in the README redesign list so the behavior change is tracked.

**Scope adjustments:** per Decision 4, xlsx (and likely image) tooling moves earlier and inside the framework. Dropping JCR, `MClassLoader`, the Groovy shell, `InstanceServices`, `async="distribute"`, and the Elastic entity group all remain correct.

## Follow-ups

Plan updates made with this review: `Cluster.md`, `Architecture-Proposal.md` (Sessions and login, Summary of Decisions), `Implementation-Phases.md` (phase 1 `env`, phase 2 conf/secrets, phase 7 session key, phase 10 cluster cert), `Ssh.md` (host key), `Crate-Layout.md` (`env` module), `README.md` (cluster bullet and the redesigns list) now describe configuration-held keys, `runtime/.env`, and generated secrets.

Still open:

1. Add the four measure-early probes to the spike list: per-connection runtime footprint, per-thread JS compile cost, embedded Postgres RSS under load, `date_bin` quarter/week bucket parity against the Elastic histograms.
2. Write the two undecided semantics into the plans: iterator-over-tx-cache merge (risk 5) and streaming/spooling for exports and uploads (risk 1).
3. Extend the parity-harness intentional-differences list with the Java bugs not to replicate: double-run conditions, iterator stubs, silent entity-auto stubs, EECA fetch patterns.
4. Finalize the property names in Decision 1 and move the properties-to-always-set table into the deployment documentation when that exists.
