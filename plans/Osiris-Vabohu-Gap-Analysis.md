# OSIRIS / Vabohu → MR-Moqui API gap analysis

Baseline audit before the migration branch. Current implementation status: [Osiris-Vabohu-Implementation.md](Osiris-Vabohu-Implementation.md).

## Verdict and scope

**Cannot run as-is.** A JavaScript port alone is insufficient: the LLM runtime, REST/screen introspection contracts, entity graph helpers, shared diff queue, and completion transport also need implementation or replacement.

Inspected working trees:

- **OSIRIS:** `/home/user/labs/moqui/runtime/component/osiris`, HEAD `4db80cb24a1ab045c4c2aee85f9ff692d204b754`, **including its uncommitted changes and new `BusinessProcessServices.xml` / `BusinessProcessState.groovy`**.
- **Vabohu frontend:** `/home/user/labs/vabohu/www`.
- **Target:** `/home/user/moqui/mr-moqui`, HEAD `d2e89b816cc7176f5fab7039e45fdbba54cce667`.

Below, OSIRIS paths are relative to its component directory; target paths are relative to this repository. Line numbers refer to these inspected files, not the committed-only OSIRIS version. This is a source/API audit plus a parser check, **not a successful deployment or end-to-end execution test**. No application code was changed.

## 1. Actual missing runtime/API contracts

These remain blockers **after** translating Groovy syntax to JavaScript. “Missing” means missing from the callable script/HTTP surface, not necessarily that no underlying Rust capability exists.

| OSIRIS source lines | API used / gap | Required change |
|---|---|---|
| `service/GanedenServices.xml:1034-1075`; `src/main/groovy/osiris/BusinessProcessIdentityResolver.groovy:83-107` | `ec.llm.client(...)`; fluent `system`, `user`, `conversation`, `tool`, `timeout`, `maxIterations`, `beforeRequest`, `call`; `LlmTool.service()`, `LlmTool.browse()`, custom server tools, terminal results, `LlmConversation.appendUser()` | **Implement the LLM/tool/conversation runtime or replace it with an application-owned equivalent.** Preserve validated tool execution, retry/error feedback, terminal acceptance, identity selection, and latest-edit injection before each model request. A raw HTTP call to an LLM is not equivalent. |
| `src/main/groovy/osiris/BusinessProcessDiff.groovy:51,53,128,150`; `service/GanedenServices.xml:871-876,882-891,917-926,932-949,954-962,970-980`; `src/main/groovy/osiris/BusinessProcessIdentityResolver.groovy:100` | `LlmConversationImpl.checkCanView()`, `persistIsolated()`, `LlmGateway.withoutCallerTx()` are Java implementation APIs, not MR script APIs | Replace owner/access checks explicitly; move isolated DB operations into services with a new transaction; keep external model I/O outside the caller transaction. The existing `requireNewTransaction(true)` service option is usable, but does not supply these Java callback APIs. Do not remove process-row locks or merge these boundaries into one long transaction. |
| `MoquiConf.xml:22-30` | Two `<llm-facade><profile>` definitions have no executing LLM facade behind them | Port profile loading and behavior along with the LLM runtime: provider settings, system prompt, client-system restriction, browse permissions, and logging policy. Config merge support and LLM entity definitions already exist; neither implements `ec.llm`. |
| `src/main/groovy/osiris/BusinessProcessDiff.groovy:10-12,94-113,126-133`; `service/GanedenServices.xml:875,884-890,909-980` | JVM-static concurrent `requests`, `pendingRequests`, `workers` shared across requests and async service workers | Replace with an explicitly shared Rust queue/state extension or database-backed queue. Ordinary JS module/global maps are not an equivalent cross-worker shared store. Preserve per-process exclusion, supersession, owner scoping, terminal replay, and one-hour retention. Database storage is an option, not a requirement for matching the current single-process implementation. |
| `MoquiConf.xml:10-19`; `src/main/groovy/osiris/BusinessProcessDiffEventsServlet.groovy:9-11,15-77`; `service/GanedenServices.xml:901` | Jakarta `HttpServlet`, `MoquiAuthFilter`, servlet context/current EC, streaming `response.writer`, `Semaphore`, `Thread.sleep()`; custom servlet registered at `/ganeden/business-process/events/*` | Implement a native authenticated SSE route at the same URL, or change the frontend to a new completion protocol. Preserve GET-only handling, owner checks, 401/403/404 behavior, capacity limit/503, heartbeats, disconnect handling, `completed`/`failed` events and JSON payloads. MR's XML servlet-config merge does not load Java servlet classes. |
| `service/GanedenServices.xml:251-278,597-625`; `src/main/groovy/osiris/BusinessProcessContext.groovy:17-24` | `ec.serviceFacade.restApi.rootResourceCache`, resource traversal via `resourceMap`, `idNode`, `methodMap`, `instanceof RestApi.MethodService`, `handler.serviceName` | Add a public **REST path + HTTP method → backing service metadata** resolver, or expose a complete equivalent tree. Replace both XML traversal copies and the helper. Existing `ec.service.restApi.getRootResources()` is dashboard metadata, not this resolver: it omits method handlers and ID nodes. Must preserve named-resource/ID fallback and non-service rejection. |
| `service/GanedenServices.xml:211-217,559-565,688-689`; `service/GanedenServices.xml:145-148` | `ec.screenFacade.getScreenDefinition(...).getTransitionItem(...)`; transition `getParameterMap()`, `parameterMap`, `getSingleServiceName()` | Expose screen/transition metadata through a public script API or replace these calls with a dedicated metadata service. Rendering a screen is supported, but the Java definition/transition objects are not exposed. Needed for reference validation and activity input discovery. |
| `service/GanedenServices.xml:221-236,569-584` | `ec.web.request.servletContext.getInitParameter('moqui-name')`, `request.serverName`, `ec.screenFacade.rootScreenFromHost(...)`, Java `ScreenUrlInfo.getScreenUrlInfo(...).getUrlWithBase('')` | Replace servlet request access and Java static URL generation with a native root-screen/path-resolution helper. Existing host/base URL access can supply host/domain information, but not this complete screen-location → mounted transition URL calculation. |
| `service/GanedenServices.xml:224-234,572-582` | `getScreenInfoList(...).find { it.sd.location == ... }` and `parentInfo` chain | Adapt to MR metadata **and supply missing ancestry/path information**. `ec.screen.getScreenInfoList()` exists, but returns flat rows with `location`, `name`, `level`, counts and `ssi`, not Java `sd`/`parentInfo` objects. Merely renaming `screenFacade` to `screen` does not fix this. |
| `service/GanedenServices.xml:147-148,156,278,693-694` | `getServiceDefinition()` exists, but consumers require `inParameterInfoMap`, `inParameterNames`, `getInParameter(name).attribute('required')` | Extend service metadata wrappers or rewrite callers to inspect `serviceNode` parameter nodes. Current metadata exposes scalar service settings plus `serviceXml`/`serviceNode`, not these Java parameter maps/methods. Preserve required-parameter and backing-service fallback logic. |
| `service/GanedenServices.xml:471-473,796-798,866,937-938,1186-1187`; `service/BusinessProcessServices.xml:264` | `EntityFind.oneMaster('default')` | Add a callable master-graph read helper or explicitly assemble the same graph in an application service. Script `EntityFind` has `one()`/`list()`, not `oneMaster()`. Master definitions and REST/data export expansion exist internally, but that does not make this method callable. A plain `one()` is **not** equivalent: consumers require nested phrases, subtype metadata, references and relationships. |
| `service/GanedenServices.xml:748,1199` | `EntityFacadeImpl.getValueListFromPlainMap(businessProcess, 'ganeden.BusinessProcess')` | Implement recursive plain-graph → typed entity-value flattening or replace both calls with explicit typed graph traversal. Preserve entity identity, keys, relationship propagation, insertion ordering and reverse deletion ordering. Not supplied by `makeValue()` alone. |
| `service/GanedenServices.xml:1044,1052-1054` | `toolEc.message.pushErrors()` / `popErrors()` | Add scoped error-stack handling or isolate validation service errors explicitly. `clearErrors()` exists; push/pop do not. Clearing the current context without restoring prior errors would change retry/validation behavior. |

### Target implementation evidence for section 1

- **LLM intentionally deferred:** `plans/Remaining-Work.md:7` explicitly assigns LLM client, conversations, tool loop and host routes to Phase 13. `crates/moqui-core/src/context/conf.rs:299-311` only merges LLM config. `runtime/base-component/framework/entity/LlmEntities.xml` contains the entities. No `ec.llm` installation or LLM client/tool implementation was found in Rust source.
- **Native HTTP:** `crates/moqui-host/src/http.rs:156-165` registers Axum routes. There is no OSIRIS SSE route or Java servlet loader. `crates/moqui-core/src/context/conf.rs:173-177` merges servlet XML only.
- **REST introspection:** `crates/moqui-core/src/script/runtime/tools_host.rs:590`; `crates/moqui-core/src/service/rest.rs:66-109`. The internal routing tree exists, but exported `resource_info` omits `methodMap` and `idNode` and only includes one level of named children.
- **Screen introspection:** `crates/moqui-core/src/script/runtime/ec_screen.rs` exposes rendering/testing helpers, not Java screen definitions; `crates/moqui-core/src/script/runtime/tools_host.rs:280-286` exports screen info. Its actual row shape is built in `crates/moqui-core/src/screen/info.rs:150-168,182-210`.
- **Service metadata:** `crates/moqui-core/src/script/runtime/tools_host.rs:253-258`; `crates/moqui-core/src/service/info.rs:106-203`. `serviceXml` is reconstructed; nested parameter detail is not fully preserved. If expanding the metadata API for LLM tool schemas, use authoritative definitions, not this lossy reconstruction alone.
- **Entity script bindings:** `crates/moqui-core/src/script/runtime/entities.rs:441-471,1091-1093,1222-1242,1341-1347`. No `oneMaster()` or `getValueListFromPlainMap()` binding was found. Existing graph expansion lives in `crates/moqui-core/src/entity/query_rest.rs`.
- **Async/new transactions:** `crates/moqui-core/src/script/runtime/services.rs:23-25,126-134,237-270`; `crates/moqui-core/src/script/runtime/ec_extra.rs:839-846` creates the factory script object. No `ec.runAsync` or `ec.factory.executionContext` script binding was found.
- **Message facade:** `crates/moqui-core/src/script/runtime/facades.rs:281-350` exports error operations, but no push/pop stack methods.

## 2. Existing capabilities with incompatible call shapes

These are component rewrites, not reasons to implement an entirely new subsystem.

| OSIRIS source lines | Incompatible call | Equivalent/change |
|---|---|---|
| `service/BusinessProcessServices.xml:191-206` | `ec.runAsync { ... }`, `ec.factory.executionContext`, JVM synchronized collections | Rewrite fan-out as `ec.service.async().name('BusinessProcessServices.resolve#PhraseIdentity').parameters(...).callFuture()` per proposed phrase, then join each future with `.get()` and collect returned `matches` in the caller. **Result-returning parallel services already exist** (`crates/moqui-core/src/script/runtime/services.rs:219-239,278-317`); no new fan-out subsystem is required. Preserve error propagation, caller identity/authz and duplicate-ID rejection; do not share mutable JS objects between workers. |
| `service/GanedenServices.xml:929-950,991,999,1064` | A Groovy closure is passed as the `nextMessage` service parameter and invoked by the callee | Refactor the queue-fetch callback into a named service/helper with serializable process/request IDs, or keep the orchestration inside one JS/Rust implementation. MR service parameters cross the JS boundary as `moqui_lib::Value` data (`crates/moqui-core/src/script/runtime/services.rs:192-216`), not JVM closures or callable JS references. A syntax-only closure-to-function rewrite is insufficient. |
| `service/GanedenServices.xml:39` | `ec.user.userGroupIdSet` relies on Groovy bean-property resolution | Use `ec.user.getUserGroupIdSet()`; binding at `script/runtime/user_host.rs:112` under `crates/moqui-core/src`. |
| `service/GanedenServices.xml:147,211,222-224,251,278,559,570-572,597,688,693`; `src/main/groovy/osiris/BusinessProcessContext.groovy:17` | `ec.serviceFacade`, `ec.screenFacade`, Java implementation-class casts | Use `ec.service` / `ec.screen` where the corresponding method actually exists. No such facade aliases are installed. The missing metadata methods in section 1 still need implementation; removing casts is not sufficient. |
| `src/main/groovy/osiris/BusinessProcessDiff.groovy:44-46,64` | Fluent `.makeValue(...).setSequencedIdPrimary()` and `.setAll(...).create()/update()` | `setSequencedIdPrimary()` exists but returns void; `setAll()` is not exposed. Keep the value in a variable, call sequence assignment separately, assign fields or use supported `setFields()`, then call `create()`/`update()`. Preserve explicit null writes when detaching pending tool calls. |
| `service/GanedenServices.xml:750-751,1200,1365,1378,1382-1389,1402` | `EntityValue.resolveEntityName()` | Use supported `getEntityName()` and verify canonical names for the persistence/delete-rank comparisons. Binding: `crates/moqui-core/src/script/runtime/entities.rs:1091-1093`. |
| `service/GanedenServices.xml:952,1048,1079`; `service/BusinessProcessServices.xml:199`; `src/main/groovy/osiris/BusinessProcessContext.groovy:10`; `BusinessProcessDiff.groovy:144,147`; `BusinessProcessIdentityResolver.groovy:93` (same directory) | `ec.message.errorsString` / `worker.message.errorsString` | Call `getErrorsString()`. MR exposes a method, not Groovy's automatic bean property. Binding: `crates/moqui-core/src/script/runtime/facades.rs:336-338`. |
| `service/GanedenServices.xml:194,276,620,654,693,748,1199`; `src/main/groovy/osiris/BusinessProcessContext.groovy:24` | `instanceof org.moqui...` and Java implementation casts | Remove JVM type dependencies. Use explicit supported API contracts/discriminators; for REST methods, the resolver must report whether a handler is service-backed. Do not replace typed validation with unconditional acceptance. `ec.entity.isEntityDefined()` itself already exists. |
| `service/GanedenServices.xml:368` | `org.moqui.util.StringUtilities.camelCaseToPretty()` | Use `StringUtilities.camelCaseToPretty()` exposed by MR (`crates/moqui-core/src/script/runtime/tools_host.rs:657`); drop Java package lookup. |

`ec.web.getWebappRootUrl()` is present (`crates/moqui-core/src/script/runtime/facades.rs:772-781`). Its arguments are ignored and it uses the current base URL, so verify proxy/HTTPS output rather than assuming Java's secure-URL selection semantics are preserved.

## 3. Language / helper loading: separate from API gaps

The XML artifact model is retained, but expressions and scripts must be JavaScript. None of the following JVM classes becomes available merely by copying OSIRIS's jar or source directory:

- `src/main/groovy/osiris/BusinessProcessChanges.groovy:1-60`: ChangeSet application and range mapping; port algorithm to JS/Rust.
- `src/main/groovy/osiris/BusinessProcessDiffActions.groovy:1-162`: semantic action validation, reference remapping, deep copies and generated IDs; port algorithm to JS/Rust.
- `src/main/groovy/osiris/BusinessProcessContext.groovy:1-26`: service-backed spans plus REST introspection; port and address section 1.
- `src/main/groovy/osiris/BusinessProcessDiff.groovy:1-153`: queue/transactions; port and address section 1.
- `src/main/groovy/osiris/BusinessProcessIdentityResolver.groovy:1-110`: identity validation and model tools; port and address section 1.
- `src/main/groovy/osiris/BusinessProcessState.groovy:1-39`: state projection; ordinary entity/service APIs exist, but the Groovy class and `JsonSlurper` must be replaced. Preserve fail-closed state gating.
- `src/main/groovy/osiris/BusinessProcessDiffEventsServlet.groovy:1-79`: native HTTP rewrite, not a JS transliteration of servlet calls.
- `src/main/groovy/start/osiris.groovy:16-38`: JVM startup class; currently only logs startup, so it is not a substantive business-function blocker.

All XML calls to `osiris.BusinessProcess*` need an explicit JS/Rust loading/call mechanism after these ports. Java `ConcurrentHashMap`, `ConcurrentLinkedQueue`, `Runnable`, `Consumer`, exception classes and JSON helpers need appropriate replacements; `JSON.parse/stringify` only replaces JSON conversion, not threading or transaction behavior.

Parser check performed:

```sh
target/release/moqui lint --groovy /home/user/labs/moqui/runtime/component/osiris
```

Result: **641 expression sites; 207 parse failures (189 site failures + 18 generated-action/template-body failures); 232 sites with a Groovy idiom or failure**. The scan reported 10 files and is not a full Groovy-class compiler or semantic compatibility test. This report mode exited 0 despite reporting failures. Full output is at `/tmp/osiris-mr-moqui-lint.txt` (temporary local artifact).

Examples: `service/BusinessProcessServices.xml:16-38` contains static imports, casts, closures and Groovy map literals; `service/GanedenServices.xml:909-980,998-1080` contains embedded Groovy orchestration. These are execution blockers before most API calls can even be reached.

`template/BusinessProcessDiff.ftl` is currently **plain prompt text**, read with `getLocationText()` at `service/GanedenServices.xml:1061`; its `.ftl` extension alone is not an active FreeMarker-rendering blocker for that call.

## 4. Deployment dependencies and frontend impact

- `component.xml:4` depends on **MarbleERP**. This target's `runtime/component` has neither MarbleERP nor SimpleScreens. Install/port the actual dependency chain or deliberately narrow OSIRIS's dependency and seed/demo scope; do not assume the stock MR runtime supplies it.
- `data/GanedenDemoData.xml:47,49,69,89,106` references `component://SimpleScreens/screen/MyAccount/User/TimeEntries.xml` transitions. Those demo activities cannot execute against this target as installed. Port/install the screens or retarget/remove those demo references.
- `data/GanedenSeedData.xml:30,61` references tools' `createUserGroupMember` and webroot REST `login`. These must be validated through the new transition metadata/URL adapter; they are not evidence that REST dispatch itself is missing.
- The component's `screen/App.xml` is only a placeholder. The useful UI is Vabohu, not that screen.
- Existing frontend backend contract: `www/do.html:57`, `www/activity/[phraseId].html:33-34` use `/rest/s1/ganeden/activity...`; `www/business-process-diff.js:17,39-75,145-154` POSTs edits, consumes the returned `eventsUrl`, and expects credentialed `completed`/`failed` SSE events. These frontend files are under `/home/user/labs/vabohu`.
- **Frontend changes are avoidable** if MR preserves those JSON/URL/SSE/authentication contracts. Switching to polling or another event channel requires changing `www/business-process-diff.js:39-75,145-154`, not just changing a backend service.
- Verify CORS, session cookies, CSRF and proxy/HTTPS behavior in integration tests. This audit did not prove a missing CORS/auth subsystem. Vabohu's existing cross-origin credentialed usage still needs deployment configuration.

## 5. What is already there / recommended implementation boundary

Do **not** classify ordinary entity CRUD/find/conditions, `forUpdate(true)`, `useCache(false)`, service sync/async, new-transaction service calls, entity definition/field lookup, user group checks, password/MFA login methods, sessions, base URL access, resource text reads, REST XML dispatch or XML artifact definitions as wholly unimplemented. Their underlying capabilities exist; the table identifies the missing entry points/object shapes actually used by OSIRIS.

Smallest sensible path to equivalent functionality:

1. Port expressions/helper algorithms; adapt the existing-capability call shapes in section 2.
2. Supply master graph read/flatten helpers plus REST/transition/parameter/URL metadata adapters. Consolidate duplicated reference resolution rather than porting Java internals verbatim.
3. Port/install required screens/components and make read-only activity discovery, process retrieval, reference validation and state gating work first. These can be exercised without the LLM editor, but are not currently drop-in compatible either.
4. Supply the LLM/tool/conversation implementation, isolated transaction services and shared queue. Replace closure-based parallel identity resolution with existing service `callFuture()`/`get()` calls.
5. Add the authenticated completion SSE route, keeping Vabohu's contract unchanged.
6. Run translated OSIRIS tests plus frontend tests and real MR integration tests: master graph round-trip, referenced transition parameters, owner-denied event replay, concurrent edits/supersession, stale-source rejection, rollback, LLM retry/terminal tool behavior, ADMIN-only relationship changes, MFA and fail-closed activity state.

**Bottom line:** existing MR entity/service/REST infrastructure is reusable. The principal new runtime work is LLM orchestration and native completion/queue transport; the principal non-LLM API gaps are metadata introspection and master-graph conversion. It is not just a Groovy syntax conversion.
