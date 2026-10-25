# Monitoring

Planned Oct 25, 2026: requirements, architecture and the M1–M5 implementation plan; no code yet. Next after the performance work. A new Monitor app in the tools component, built on OpenTelemetry, replaces ArtifactHit/ArtifactHitBin and the `moqui_logs`/`moqui_http_log` shipping, and takes in OTLP from external systems.

## Objective and scope

Provide a unified, technology-independent system that connects software changes, execution context, telemetry, source-level diagnostics, user and business impact, and recovery decisions.

Support backend services, browser, mobile and desktop applications, background workers, operating-system processes, containers, virtual machines, physical infrastructure, databases, networks, and AI-powered applications. Monitoring, diagnosis, and recovery must continue throughout each deployed version's lifetime, not only during releases.

## 1. Identify software, changes, and runtime exposure

- Record application/component identity, owner, team, source repository and revision, software version, immutable build artifact identity, build configuration, dependencies, and debugging symbols/source maps.
- Record every deployment, configuration, dependency, infrastructure, feature flag, AI model, and prompt change, including when it took effect.
- Track deployment environment, previous and candidate versions, rollout strategy, version overlap, and applicable recovery policy.
- Identify runtime instances and their lifecycle, host/device, operating system, infrastructure, executable version, and effective configuration and dependencies.
- Track which requests, users, sessions, and traffic populations encounter each version where measurable.
- Maintain authoritative identities and relationships so observations can be attributed to the exact software, deployment, runtime, and infrastructure without duplicating all metadata on every signal. Explicitly identify attribution uncertainty.

## 2. Collect and correlate telemetry

- Collect metrics, structured logs, distributed traces, profiles, stack traces, crash diagnostics, application/infrastructure events, real-user monitoring, session replay, business/product analytics, AI/LLM telemetry, network flows, network packet data, synthetic checks, security signals, audit records, change records, resource/topology data, and physical/device telemetry where applicable and supported.
- Capture relevant timestamps, identities, and context; normalize timestamps and account for clock differences.
- Preserve request, transaction, session, and causal relationships across clients, services, dependencies, networks, and infrastructure, including asynchronous tasks and message propagation where available.
- Support real-time analysis and historical investigation, including version-to-version comparisons.
- Validate schema compatibility, instrumentation coverage, and data quality; detect missing telemetry and broken correlation links.
- Apply sensitive-data filtering/masking, access controls, retention policies, and auditability throughout collection, investigation, and recovery.
- Support correlation of high-cardinality identities without requiring them as labels on every metric.

## 3. Capture failures and code-level performance

- Capture handled and unhandled exceptions, nested causes, hangs, deadlocks, resource exhaustion, crashes, and process termination reasons.
- Preserve available function/frame/module/source locations, asynchronous context, thread/task states, crash artifacts, and fault state. Retain symbols and source maps needed to resolve executable addresses to source code.
- Collect continuous or periodic CPU, memory, allocation, contention, and other supported profiles to attribute resource consumption to functions, threads, and execution paths.
- Link failures to profiles, logs, traces, active requests, dependency behavior, runtime state, and host/resource health before, during, and after the incident where possible.
- Connect frontend failures to application builds, devices, user sessions, and interaction history; connect backend failures to upstream callers and downstream dependencies.
- Use external diagnostics when a failing process cannot report, including abrupt termination, native crashes, out-of-memory kills, and host failures. Preserve partial evidence and explicitly show missing diagnostics.

## 4. Detect and quantify regressions

- Compare candidate versions against historical baselines, stable versions, or suitable control groups, using equivalent traffic, user populations, environments, and infrastructure where possible.
- Evaluate reliability, latency, resource consumption, security, user experience, business outcomes, and AI quality, latency, cost, and failure rates.
- Determine regression magnitude, duration, affected versions/instances/populations, severity, confidence, and data quality.
- Distinguish localized runtime/infrastructure faults from widespread version regressions, accounting for concurrent traffic, dependency, configuration, and infrastructure changes.
- Detect insufficient sample sizes and missing telemetry rather than interpreting them as evidence of health.
- Detect significant regressions without requiring a confirmed root cause first.

## 5. Diagnose probable root causes

- Identify the first observed deviation and preceding changes.
- Compare representative failing and healthy transactions, stacks, execution paths, logs, profiles, dependencies, resource usage, and source/configuration differences.
- Determine whether a fault originates in the affected component or propagates from a dependency, and identify affected user journeys and business transactions.
- Rank probable causes with supporting and contradicting evidence, relevant source revisions/functions/frames, timelines, and impact.
- Distinguish confirmed causes from correlation-based hypotheses and show unresolved uncertainty or missing evidence.
- Recommend relevant investigation or recovery actions.

## 6. Execute safe recovery

- Support continued observation, promotion, rollout pause, alerting, instance isolation, feature/configuration disablement, restart, rollback, and escalation for human approval.
- Select the least disruptive authorized action based on impact, confidence, recovery policy, permissions, sample volume, blast radius, and dependencies. Prefer targeted recovery for isolated faults.
- Permit automatic rollback only when sufficiently supported by evidence and verified as compatible with database schemas, persisted data, external interfaces, and dependent services.
- Escalate when no safe, authorized automatic action is available.
- Prevent repeated reversals, cascading actions, and conflicts between controllers.
- Preserve incident evidence and record the decision, rationale, policy, actor, authorization, execution history, and resulting deployment/runtime state.
- Execute recovery through existing deployment or operational controls.

## 7. Verify recovery and retain outcomes

- Verify that the intended action took effect and technical, user, and business indicators returned to acceptable behavior.
- Detect incomplete, failed, or harmful recovery; reopen or escalate unresolved incidents.
- Record the incident timeline, recovery outcome, resolution, corrective code changes, and confirmed root cause or unresolved status.
- Retain evidence for investigation, improvement, and compliance.
- Use incident findings to improve instrumentation, baselines, detection conditions, and recovery policies.

## 8. Provide a unified investigation experience

- Allow investigations to start from a version, regression, error, or incident rather than a telemetry storage type.
- Present severity, preceding changes, affected versions and infrastructure, technical/business impact, ranked causes, code-level evidence, distributed execution history, user experience, safe recovery controls, and recovery verification together.
- Allow direct navigation between errors, stacks, source code, profiles, traces, hosts, and deployment differences while preserving investigation context.
- Avoid requiring developers to manually combine evidence from independent monitoring systems.

## 9. Measure effectiveness

- Track detection time, cause-identification time, verified recovery time, regression detection precision/recall, automatic recovery success, false rollback rate, attribution coverage, symbolication coverage, profiling coverage, investigation effort, and business impact avoided.

---

## Operator UX: a 3 a.m. incident

- Usually no wake-up: authorized, reversible failures are contained or recovered automatically and verified against business health. Deduplicate/group alerts, suppress transient noise, and send a morning summary; page for critical integrity/security failures, unresolved business impact, failed recovery, or unsafe/unauthorized actions. Missing telemetry is not evidence of recovery.
- The page states what failed, severity, affected ERP operations/tenants, business impact, onset, current status, and recovery attempted/blocked, with one authenticated incident link and no sensitive document contents.
- That link opens one timeline: preceding build/config changes, affected versus healthy instances, baseline comparison, transaction outcomes, errors/stacks, granular JS hot functions/blocks, database/host evidence, ranked causes with uncertainty, and missing telemetry. Trace/profile/source links retain the incident context.
- Optional AI-assisted search: the operator asks “what changed, where is the JS bottleneck, and what is safe to do?” AI searches only authorized, tenant-scoped, redacted incident evidence and approved runbooks/source excerpts; answers cite evidence and flag uncertainty. Enforce filtering before indexing/model access, not just in prompts; exclude document contents, replay/raw dumps, credentials, arguments/locals, and secret-bearing source/config. Use pseudonymous references, audit searches/access, and never send raw ERP data to external models. AI has read-only scoped tools, no recovery authority; ordinary search and deterministic diagnosis remain available without AI.
- Show the next safe action, approval/compatibility checks, blast radius, and why automation stopped. The operator can pause/isolate/rollback through existing controls, then watch technical and business verification; never offer automatic reversal of committed business transactions.
- Faster than manually joining logs, dashboards, deployment history, and source: identity and causal links are already assembled, comparisons narrow the fault scope, and source-mapped evidence identifies where to investigate. Measure page-to-diagnosis and verified recovery time; do not promise instant fixes or hide uncertainty.

## Architecture

Decisions, Oct 25, 2026:
- **SDK:** use the OpenTelemetry SDK directly, with spans made from the artifact stack and no `tracing` bridge.
- **Storage:** SearchFacade indexes, on the Postgres or OpenSearch backend.
- **Ingest:** OTLP over HTTP first, with gRPC deferred.
- **References:** SkyWalking (Apache-2.0) and SigNoz (MIT, apart from its proprietary `ee/` tree), cloned under `~/work/moquiwork/3p_other/`. We borrow ideas from them, not code, and never read `ee/`.

### Starting point

- **Logging.** [`LineSubscriber`](../crates/moqui-host/src/logging.rs) is a hand-written `tracing` subscriber.
  - It hands out span ids but keeps no span data.
  - Events go to stderr, the TUI and SSH taps, and rolling files.
  - `ship_to_search` also sends them every 2 s to `moqui_logs`, and the `log_http` middleware ([`http.rs`](../crates/moqui-host/src/http.rs)) sends to `moqui_http_log`. Both go through `SearchFacade::index_log_entries`.
  - There is no `tracing-subscriber`, and no OpenTelemetry, prost or tonic crate in `Cargo.lock`.
- **Spans.** There are two `tracing` spans:
  - `artifact`, opened at each artifact-stack push and closed at pop ([`context/artifact.rs`](../crates/moqui-core/src/context/artifact.rs)). Screens, transitions, services, entities, REST and LLM calls all push frames.
  - `sql`, in [`entity/exec.rs`](../crates/moqui-core/src/entity/exec.rs).
  - There is no request span and no job span.
- **Hits.** `ExecutionContext::pop` calls `count_artifact_hit` ([`context/factory.rs`](../crates/moqui-core/src/context/factory.rs)).
  - [`context/hits.rs`](../crates/moqui-core/src/context/hits.rs) queues ArtifactHit rows and 900 s ArtifactHitBin bins, and writes them on the scheduler tick and at shutdown.
  - `clean#ArtifactData` removes old rows.
  - `stats-skip-condition` is configured but never read.
- **Metrics.** There is a plain-text `/metrics` endpoint (pool, request queue, statements, guards, replica lag), plus `status_map` and `thread_list` in [`context/status.rs`](../crates/moqui-core/src/context/status.rs).
- **Identity.**
  - Instance: `cluster::instance_id`.
  - Version: `CARGO_PKG_VERSION` and [`build_info.rs`](../crates/moqui-core/src/build_info.rs). There is no git revision.
- **Hand-offs.**
  - `CallJob` carries `user_name` and `origin` across the service pool ([`service/pool.rs`](../crates/moqui-core/src/service/pool.rs)).
  - Jobs write `ServiceJobRun` in [`service/jobs.rs`](../crates/moqui-core/src/service/jobs.rs).
  - Outbound calls go through [`outbound.rs`](../crates/moqui-core/src/outbound.rs).
- **SearchFacade.**
  - Aggregations are `Filter`, `DateHistogram`, `Terms`, `Sum`, `Extreme` and `CountDistinct` ([`search/agg.rs`](../crates/moqui-core/src/search/agg.rs)). Retention is `delete_matching`.
  - The Postgres backend keeps one table per document type in the `search` schema, with typed root columns and a `jsonb` body.
- **UI.**
  - Chart.js 4.5.1 through `m-chart`, plus Mermaid, highlight.js through `m-code`, and `m-dynamic-container`.
  - Live data arrives through `/notws` topics (the `publish#ShellCounts` pattern) or by polling JSON transitions (the `Tools/Service/ServiceLoadRunner.xml` pattern).
  - Every qvt block needs a `tui` twin. The TUI already has `chart`, `timeline` and `tree` components.

### Shape

```
 business code ── moqui-core::telemetry (thin wrapper; no SDK types leak)
                        │  spans · metrics · logs
                  opentelemetry_sdk (batch processor, periodic reader, logger provider)
                   │                              │
          local exporter                  OTLP/HTTP exporter (optional)
                   │                              └──► external collector / backend
 OTLP/HTTP receiver ──► same documents
                   │
        SearchFacade indexes: otel_spans · otel_logs · otel_metrics · otel_errors
                   │
        Monitor app (tools/screen/Monitor.xml): reads only the indexes
```

Internal and external sources share the same documents and the same screens, told apart by the `service.name` and `service.instance.id` resource attributes. The client side (the SDK and the exporters) replaces ArtifactHit/ArtifactHitBin and the `moqui_logs`/`moqui_http_log` shipping. The server side (the receiver) brings in telemetry from other systems.

### Crates

| Crate | Use |
|---|---|
| `opentelemetry` | API: tracer, meter, logger, context, W3C `traceparent` propagator |
| `opentelemetry_sdk` | Batch span processor on its own thread, periodic metric reader, logger provider, resource, samplers |
| `opentelemetry-proto` | OTLP messages (prost) for the receiver, and for our own exporter if needed |
| `opentelemetry-otlp` (http-proto) | Optional external exporter, used only if its HTTP client resolves to our reqwest 0.13. Otherwise a small exporter encodes with `opentelemetry-proto` and POSTs through `moqui_lib::http::RestClient` |

- **Versions.** All `opentelemetry*` crates go on one minor version, pinned by `Cargo.lock`.
- **Not used:** `tracing-subscriber`, `tracing-opentelemetry` and `opentelemetry-appender-tracing`. Spans come from the artifact stack and logs from `LineSubscriber`, so neither needs a `Registry`. tonic is deferred along with gRPC.
- **Check before adding.** Run `cargo tree` and look for:
  - tonic or a gRPC transport pulled in by the proto features
  - a second hyper or reqwest
  - a required async runtime (moqui-core has no direct tokio dependency)
- **Licenses.** These crates are Apache-2.0, which [`deny.toml`](../deny.toml) already allows. Add a row to [License-Review](archive/License-Review.md).
- **Wasm.** Keep them behind a `telemetry` feature that is off in `moqui-browser`.

### Signals

**Spans**
- **Request:** a SERVER root span per HTTP request, opened in `log_http`. It carries route, status, user, visitor and peer attributes, and replaces `RequestLog` and `moqui_http_log`.
- **Artifact:** a span per artifact frame, opened at push and ended at pop where the `artifact` span is today.
  - Attributes: `moqui.artifact.type`, `.name`, `.action`, `.sub_type`, user and visit ids, and the error status from `ec.message` errors.
  - These spans replace ArtifactHit rows.
  - User and visit ids are span attributes, never metric labels.
- **SQL:** a CLIENT span per statement, with `db.system.name` and the parameterized `db.query.text`. The slow-statement warning and the statement counters stay.
- **Jobs:** a root span per `ServiceJobRun`, with the trace id saved on the run so the job screens can link to it.
- **Propagation.** W3C `traceparent`/`tracestate` is:
  - read on requests
  - carried in `CallJob` next to `origin`, so async services and pool work join the trace
  - injected by `outbound.rs` and remote services
  - written into the qvt page so `WebrootVue` fetches carry it (browser spans come later with RUM)
- **Sampling.** Head sampling is set in config. The local exporter keeps every trace that has an error or a slow root (a simple tail rule) and drops the rest of the unsampled traces.

**Metrics** (SDK instruments, with OTel semantic names where they exist)
- **Instruments:**
  - `http.server.request.duration`
  - `moqui.artifact.duration`, labelled by type and name (and action where there is one). It replaces ArtifactHitBin: artifact names are a bounded set, and the bin becomes one minute.
  - `db.client.operation.duration`
  - `moqui.job.runs`, counted by outcome
  - observable gauges for pool, request queue, guards and process, from the values `/metrics` and `status_map` already compute
- **Export:** the periodic reader runs every 60 s and writes one document per series per interval (count, sum, min, max and histogram buckets). This is SkyWalking's pre-aggregation done by the SDK, so RED charts never scan raw spans.
- **Rollups:** a job rolls minute documents up to hour and day documents.
- **`/metrics`** stays, and can also serve Prometheus text from the same readers.

**Logs**
- `LineSubscriber` keeps stderr, the files and the taps.
- Instead of `ship_to_search`, it sends each event to an OTel `Logger`, taking trace and span ids from the current artifact frame, OTel severity numbers, and its event fields as attributes.
- DEBUG and TRACE are still not exported by default, and the exporter's own thread is skipped.

### Indexes

`search/telemetry.rs` generalizes [`search/logs.rs`](../crates/moqui-core/src/search/logs.rs): flat mappings that are not DataDocuments and work on both backends. Field names follow OTel (and SigNoz): `trace_id`, `span_id`, `duration_nano`, `severity_number`, `resource.*`, `attributes.*`.

| Index | Key fields (typed columns; the rest stays in the body) |
|---|---|
| `otel_spans` | trace_id, span_id, parent_span_id, name, kind, start, duration_nano, status_code, service.name, service.instance.id, moqui.artifact.type/name, user and visit ids, attributes, events |
| `otel_logs` | timestamp, severity_number/text, body, trace_id, span_id, service.name, instance, attributes |
| `otel_metrics` | metric name, series fingerprint, bucket start, granularity (minute/hour/day), count, sum, min, max, buckets, attributes |
| `otel_errors` | fingerprint (hash of type, normalized message and top frames), first_seen, last_seen, count, sample trace ids, service, last message/stack |

- **Errors.** The local exporter upserts `otel_errors` when a span ends in error or an exception event is recorded. This is the SigNoz groupID idea.
- **Retention.** A daily ServiceJob replaces `clean#ArtifactData`. It deletes by time with `delete_matching`, with separate settings per index and per metric granularity: raw spans and logs for days, minute metrics for days, hour and day metrics for months.
- **Later:** a Postgres-backend option to partition these tables by day, so retention drops partitions instead of deleting rows (SkyWalking's day tables).
- **Telemetry config.** It replaces `artifact-stats`. The settings: per-type enable and sampling (keep the artifact-type list), retention, the exporters, and the receiver. `stats-skip-condition` either becomes a sampling rule or goes away.

**SearchFacade additions** for the screens:
- `Avg` (or sum and count read together)
- `Percentiles`: `percentile_cont` on Postgres, native on OpenSearch
- a histogram-bucket merge for metric documents, so p50/p95/p99 can come from the SDK buckets without reading raw spans

These go into `Agg` and both backends, with tests on each.

### Ingest

- **Endpoints:** `POST /v1/traces`, `/v1/logs` and `/v1/metrics` on the existing axum server, taking `application/x-protobuf` and `application/json`. Responses follow OTLP, including partial success.
- **Auth:** an API key tied to a user with a `MONITOR_INGEST` permission. Each key names the source, and the source is recorded on what it sends.
- **Limits:** body size, rate per key, and a bounded queue that rejects when full and counts what it rejects. Redaction rules run before indexing.
- **Decoding:** `opentelemetry-proto` decodes into the same documents the local exporter writes, so every screen works on external data unchanged.
- **gRPC** (tonic on 4317) is deferred behind a feature.
- **Pulling later:** a ServiceJob that scrapes Prometheus-text endpoints into `otel_metrics`, and synthetic checks recorded as spans.

### Failure tolerance

- Telemetry never blocks or fails business work. Exports are bounded, run off the request thread, and never write inside a business transaction.
- A failed batch, a document that will not index, or a bad OTLP record is logged once (rate-limited), counted, and skipped.
- When the search backend is down, telemetry is dropped and loss counters go up. Those counters show on `/metrics` and on the Monitor Sources screen.
- Missing telemetry shows as missing, never as healthy (requirement 2).

### Monitor app

`runtime/base-component/tools/screen/Monitor.xml` is added to `apps.xml` from `tools/MoquiConf.xml` at menu-index 97 (System is 98, Tools is 99). It uses icon `sym_o_monitoring` and a `MONITOR_APP` artifact group in `ToolsSecurityData.xml`.

| Screen | Content | Replaces |
|---|---|---|
| dashboard | RED per service, top error groups, slowest artifacts, loss counters | System dashboard hit panels |
| Services | Service, then endpoints/artifacts, then instances. Rate, errors and duration percentiles over time | |
| Traces | Search by service, name, status, duration, user and time, then TraceDetail (waterfall, span attributes, logs for the trace) | |
| Logs | Query, filter and live tail. Each row links to its trace and span | `System/LogViewer.xml` |
| Errors | Fingerprint groups, then detail (count over time, samples, stack in `m-code`, traces) | |
| Metrics | Pick a metric, filter by attributes, group by, then chart | |
| Artifacts | Artifact duration and count by type and name, from `moqui.artifact.duration` | `ArtifactHitBins`, `ArtifactHitSummary` |
| Sources | Ingest keys, the latest data from each source, exporter status, drops | |
| Later | Alerts, Releases, Incidents (requirements 1, 4–7) | |

**Alerts** (M5) use SkyWalking's rule shape: an expression over `otel_metrics`, a period, and a silence period. A ServiceJob evaluates the rules each minute and notifies through the existing email and NotificationMessage. Alertmanager is not used.

**JS and UI**
- **Reuse, with no new library to start:**
  - Chart.js (`m-chart`) for every time series
  - `m-code` for stacks
  - Mermaid for a simple service map
  - `/notws` topics for the log tail
  - polling JSON transitions for refreshing charts
- **New in-house component:** `m-trace-waterfall`, plain positioned divs with no library.
- **TUI twins:** the `timeline` component (spans and logs), `tree` (span tree), and `chart`.
- **Library candidates only if needed** (each added through `libs/refresh.mjs` and License-Review):
  - uPlot (MIT, as SigNoz uses) if Chart.js is too slow for dense series
  - an in-house flame graph when profiles arrive
  - `rrweb` for opt-in replay, as below

### Retired

Each of these is replaced in the same milestone, so nothing loses data in between:
- **Hits:** `context/hits.rs`, the ArtifactHit and ArtifactHitBin entities and the ArtifactHitReport view, their fence rows, the REST entries in `tools/service/moqui.rest.xml`, the `artifact-stats` config, and the `clean#ArtifactData` job.
- **Logs:** `ship_to_search`, the `moqui_logs` and `moqui_http_log` mappings, and LogViewer.
- **Readers moved to the telemetry indexes:** the System dashboard, the Visit screens, `example` Gallery `Report.xml` and `Record360.xml`, `Tools/Entity/SpeedTest.xml` and `DataSnapshot.xml`, and the Assist and openui references to ArtifactHit.
- **Kept:** Visit and Visitor, which are business data. Visit ids link to spans.

## Implementation plan

Light plan. Each milestone ends with tests and an overhead measurement against the [Profiling](Profiling.md) speed tests (artifact spans sit on the hottest path).

**M1: foundation and traces**
- Check the crates with `cargo tree`, add the License-Review row, and add the `telemetry` feature.
- Add the `moqui-core::telemetry` module and the telemetry config in `MoquiDefaultConf.xml`.
- Set the resource: service name, instance id, version, and a git revision emitted by `build.rs`.
- Add the request, artifact, SQL and job spans, `traceparent` propagation (HTTP in, `CallJob`, outbound, the qvt page), sampling, and the local exporter to `otel_spans`.
- Add the Monitor app root, the Traces and TraceDetail screens with `m-trace-waterfall`, and their TUI twins.
- Tests:
  - span tree shape for a screen, service and entity call
  - propagation across `CallJob`
  - Postgres-backend index and search (like `crates/moqui-host/tests/search_logs_postgres.rs`)
  - a failed export does not fail the request

**M2: metrics and logs; retire hits and log shipping**
- Add the SDK instruments, metric documents and rollups, the retention job, and the SearchFacade `Avg`, `Percentiles` and bucket merge.
- Move `LineSubscriber` export to the OTel logger.
- Add the dashboard, Services, Artifacts, Logs and Metrics screens.
- Move every reader listed under Retired, then remove the hit and log-ship code.
- Tests: metric documents per interval, rollup totals, percentiles on both backends, log trace ids, retention.

**M3: errors**
- Fingerprinting, the `otel_errors` upsert, and the Errors screens, linked from spans, logs and the dashboard.
- Test that fingerprints stay stable across message parameters and line shifts.

**M4: external sources**
- The OTLP/HTTP receiver (keys, limits, redaction, partial success), the optional OTLP exporter, and the Sources screen.
- Tests: a `moqui test-client` spec posting protobuf and JSON fixtures; a manual run with an OTel Collector or demo app sending to the server; export to a collector.

**M5: alerts and versions**
- Alert rules, evaluation and notification.
- Release records (from the resource version and git revision) with a simple version-to-version comparison of RED metrics. This is the first piece of requirements 1 and 4.

**Later, as the requirements call for them:**
- **Server-JS profiling.** Instrument generated JS at function boundaries and at selected block and loop boundaries. Handwritten JS goes through an AST transform (for example `@babel/parser`, `@babel/traverse` and `@babel/generator` at build time).
  - Record through bounded hooks: call counts, inclusive and self elapsed time, host-call time, and sampled block counts. Keep recursion, exceptions, returns and async semantics intact.
  - Map stable function and location ids to the JS/XML source and the build, and link selected spans and errors to the active trace. Never capture arguments or locals.
  - Gate detailed instrumentation by script or request, and by a time and overhead budget. These timings are wall time, not CPU samples.
  - Native `pprof` (Rust on Linux) cannot resolve interpreted JS frames. True JS CPU or line sampling needs a QuickJS-NG engine-hook prototype or a verified runtime profiler.
- **Browser RUM** through the same OTLP receiver, and `rrweb` replay that is opt-in, masked and bounded.
- **Business operations.** Stable operation names, with commit, rollback, retry and pending outcomes taken from the transaction hooks (`on_commit` and `on_rollback` in `transaction/manager.rs`). Use authorized correlation references, never document contents or metric labels.
- **Regressions and incidents.** Persist baselines, regressions, incidents and evidence. Use deterministic comparisons and cause ranking, with sample and data-quality gates and explicit uncertainty. Incident, release and operation screens keep the tenant, build, runtime and time context, with separate diagnostic and recovery permissions and audited access.
- **Recovery.** A durable state machine (policy, decision, execution, verification) with a deployment-controller adapter.
  - Persist each idempotent action before running it, with one active action per scope.
  - Require compatibility checks and approvals, with cooldowns, deadlines and an emergency stop.
  - Verify against business health.
  - Advisory by default. Never reverse committed business transactions automatically.
- **Partitions and gRPC.** Day partitions on the Postgres backend, and the gRPC receiver.

Completion of the later stages is gated on: correlated failures, tenant and privacy isolation, tolerance of a telemetry outage, measured overhead, regression detection, and safe, retried and failed recovery tests. Other signal types and specialized infrastructure wait until they are needed. No extra server is required by default, and an OpenTelemetry Collector stays optional.
