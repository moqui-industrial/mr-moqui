# Remaining work

Summary derived from the archived plans on 2026-10-14, extended 2026-10-16 with the parity and code quality review leftovers, and trimmed to open items on 2026-10-18. Phases 0–12 and gates 1–4 of [Implementation-Phases](archive/Implementation-Phases.md) are built. This file lists what is not. Details stay in the linked archived sections; this file does not replace them.

What is not ported on purpose (JSON-RPC, the Groovy and Java runners, the database drivers not in this tree, `service-group`, Swagger 2.0 and RAML, Hazelcast, the embedded SMTP server, vuet and `/vapps`) is in [Parity-Mapping, Excluded on purpose](Parity-Mapping.md#excluded-on-purpose); the non-SQL entity datasources (`ElasticDatasourceFactory`, OrientDB) are a proposal in [Future-Plans](Future-Plans.md). Differences from Java that are kept on purpose are in [Parity-Diffs](Parity-Diffs.md).

## Deferred on purpose

- **Phase 13: LLM, A2A and the assist panel.** Ported as the Java framework stands when the phase begins, because the Java side is not yet stable and community feedback iterations are still running. Scope: LLM client, conversation, protocols, tool loop, host routes for LLM endpoints, A2A routes (design in `../moqui/framework/plans/A2A-Server.md`), and the screens' assist panel (`js/assist`) with the other agent and assist parts. `LlmServices.xml`, `A2AServices.xml` and the tools component's `llm` REST resource were removed on 2026-10-14; take fresh copies from moqui-framework when the phase begins and wire them then. The LLM and A2A entities and `LlmTypeData.xml` stay. The LLM client sets the sim-allow exception on `RestClient`. Check: same cases as the Java tests at the time of the port. See [Implementation-Phases, Phase 13 and Phase 8](archive/Implementation-Phases.md).

## Known open items

| Area | Item | Source |
|---|---|---|
| Wasm browser host | No login flow: calls run with authorization disabled, except renders (`crates/moqui-browser/src/host.rs`) | [Wasm, Status](archive/Wasm.md) |
| Wasm | No lock between tabs: without pthreads the WasmFS OPFS backend opens a file through its async access handle, not the exclusive sync access handle, so a second tab opens and writes the database the first tab has open (`scripts/wasm-chrome-smoke.mjs` shows it and warns), and two writers can corrupt the SQLite file. A Web Locks (`navigator.locks`) lock per database name in the worker would make the second tab wait or refuse | found 2026-10-18; [Wasm, Status](archive/Wasm.md) |
| Wasm | The WebSocket check runs against a Node mock of `/notws`, not moqui-host. A live-host mode needs the package served from moqui-host's origin, a signed-in session, and checks of what the real `/notws` does (the mock's `echo:`, `drop` and `Ack` replies are its own) | same |
| Parity | Java vs MR-Moqui parity harness and its diff report (a gate artifact) has no code; only search-backend parity tests exist. Java runtime must stay runnable | [Implementation-Phases, Tests](archive/Implementation-Phases.md) |
| Search | Universal search result links go to the generic entity edit screens until IronStone has record screens; the per-document hook predicate is not built | [SearchFacade, REST service and UI](archive/SearchFacade.md) |
| Search | Semantic retrieval has the `Embedder` trait and a development hash embedder only; no local model (`semantic` cargo feature) | [Implementation-Phases, Phase 9](archive/Implementation-Phases.md) |
| Search | Postgres broad queries (common words: rank, facets, proximity) take tens to hundreds of ms at 50k documents; candidates are a stored per-field `tsvector`, a ranked-row cap, and measuring cover density on a real corpus. Hybrid `k` and weights wait on a relevance set | [Performance, Search](Performance.md) |
| IronStone | The 16 app roots (Commons, Comms and the 14 business apps) hold placeholder screens. The screens moved there from the first SimpleScreens port are starting points, not finished ports; since M0 (Oct 18, 2026) they all render in qvt and pass `moqui lint --screens`, and `test/AppStructure.spec.js` renders each with a demo record. `moqui lint --screens` still flags 11 sites in BOSS: 5 uses of the `StringUtilities`/`WebUtilities` globals, as the tools screens have, and 6 in Sales/Reports (`SalesAnalysis`, `SalesSummary`), which no spec renders yet | ironstone-boss `plan/design-drawers-commons.md`, `plan/initial-design.md`; the build plan for the shell, Commons and Comms is `plan/impl-drawers-commons.md` (milestones M0–M11) |
| MarbleERP | Acceptance work after the migration: wiki fixtures, exhaustive role and organization isolation, CSRF, every mounted QVT interaction, physical checks and scanners | [MarbleERPMigration, Limits](archive/MarbleERPMigration.md#limits) |
| Tests | `cargo test -p moqui-host --test gate3_browser`: in the server-merged qvt ArtifactGroupDetail a form-list drop-down shows its title, not the selected option (`Always Allow (overrides Deny)` is not on the page); and the static-qvt tests first run the compatibility report over every screen in `runtime/`, which is slow with the mantle components present: `qvt_report` in `cargo test-slow` passed but took 3 hours on the debug build (with another test run on the machine), and `moqui lint --screens --qvt-static` on the release build had not finished after 10 minutes | found 2026-10-18 |
| Tests | `moqui test mantle-usl SimpleScreens`: 15 of the SimpleScreensRender features fail, the same with or without the 2026-10-18 changes. Among them, the print screens are asked for with `renderMode=xsl-fo` in the path, which the screen test does not apply, so they render nothing; with `makeTest().renderMode("html-print")` they render | found 2026-10-18 |

## Revision management follow-ups (2026-10-18)

Built: [Revision-Management](../docs/Revision-Management.md) (dbresource reverse deltas, the version diff API, `git://` resources, the gallery's Editor archetype).

| Area | Item | Source |
|---|---|---|
| Git | No push to another server: gitoxide has none yet. A `db` repository is published by `export#GitRepository` and the git command line. Add `push#GitRepository` when gix supports it (pushes *to* Moqui are served: [Git-Hosting](../docs/Git-Hosting.md)) | `resource/git/fs.rs` |
| Git hosting | No repack of `local` repositories: each push adds a pack, and gix's object database slots fill over many pushes (`gc#GitRepositories` repacks `db` ones with our delta codec; a `local` one could be repacked through a pack of its own objects) | `resource/git/server/pack_in.rs` |
| Git storage | A `db` delta chain is read a level of bases per query; on Postgres a recursive CTE could read a cold chain in one. A `git://` read of an old version of a large text is fine; GC keeps chains under `git_db_delta_depth` | `resource/git/db.rs` |
| Git storage | Content blobs are in the database only (`ContentBlob.storageKind` `db`); a file system or S3-like backend behind the same store for the chunks | `resource/blob.rs` |
| Git storage | A `remote` mirror does not fetch LFS objects; and LFS transfer adapters other than basic (`git-lfs-transfer` over SSH, multipart) | `resource/git/server/lfs.rs` |
| Git storage | `GitObject` of a `local` repository written through `git://` over 8 MiB is a loose object of its size (only a `db` repository keeps large objects as content blobs); keep large files in LFS | `resource/git/fs.rs` |
| Git hosting | Not served yet: partial clone (`filter blob:none`), `report-status-v2`, `push-cert`, the dumb HTTP protocol, anonymous read of a public repository | `resource/git/server/` |
| Git hosting | `GitUpdate` rows (the push log, and one per `git://` write a gitECA rule listens to, such as a wiki save into a repository Mantle's rules watch) are never purged; a job that removes done ones past an age | `resource/git/events.rs` |
| Git hosting | `SSH_GIT` is granted to ADMIN only: `has_permission` does not count ALL_USERS, so a grant for every user needs a group of its own | `framework/data/SecurityTypeData.xml` |
| Git | SSH and `file://` fetches run the system `ssh` and `git-upload-pack`; an in-process SSH transport over `russh` (already linked for the SSH server) would drop that | same |
| Git | File history walks the first-parent line from the head on each call; a cache of each path's versions by head commit would serve large repositories | `resource/git/ops.rs` `history` |
| Git | SHA-1 repositories only; no submodules, LFS, tags as versions, or signed commits | same |
| MrSimpleScreens | `Project/ProjectSummary.xml` logs "decimal valueOf is not supported" in `ActiveProjectSection` for a project with no time or cost figures (seen 2026-10-19, before the git boxes were added) | found 2026-10-19 |
| Screens | The Editor archetype uses `text-area editor-type="md"`; the co-editing component of the style guide (§8) is not built | `example/screen/Gallery/Editor.xml` |
| Screens | `sri.baseLinkUrl(url)` is kept on the render (`RenderState.base_link_url`) but nothing reads it: Java's markdown renderers resolve a page's relative links against `getBaseLinkUri()`, and the port's markdown leaves them as written, so a relative link in a wiki page is relative to the screen URL | `screen/render.rs`; Java `ScreenRenderImpl.getBaseLinkUri` |
| MrSimpleScreens | Groovy `take()` is still called in `Order/FindOrder.xml`, `Shipment/FindShipment.xml` and `Accounting/Reports/AccountLedger.xml` (no JS shim); the wiki screens' calls were changed to `substring` | found 2026-10-18 |
| Tests | `cargo lint-min` fails: `tests/common/mod.rs` calls `moqui_host::pdf::downloaded_chromium`, which the `pdf` feature leaves out (from the chromium-download commit) | found 2026-10-18 |

## Found by the screen archetype gallery (2026-10-18)

The gallery (`runtime/base-component/example/screen/Gallery/`, [style-guide §10](../runtime/component/ironstone-boss/plan/style-guide.md)) exercised the qvt render mode and the script runtime; these are the findings not fixed that day (the fixed ones are in the gallery commit, F4 and F28 after it, and, where they differ from Java on purpose, in [Parity-Diffs](Parity-Diffs.md#gallery-fixes-2026-10-18)).

| Area | Item | Source |
|---|---|---|
| Screens | `form-list paginate="true"` over an in-memory context list (an unpaged `entity-find` into a list) renders every row and no nav row; `sections.rs paginate_items` exists but is not applied. `search-form-inputs paginate="false"` still pages when `pageIndex`/`pageSize` are in the parameters (`entity/search_form.rs:52`). Check both against Java | gallery finding F31 |
| Screens | `columns type="mobile"` is chosen by the Quasar platform (user agent, `_uiType=mobile`), not by viewport width; the style guide defines phone by width | gallery finding F22 |
| Screens | A server-static `qjs`/`qvue` component is cached and not remounted on a query-only navigation (`m-subscreens-active`); a component whose state is in the URL must watch `$root.currentParameters`. Decide: remount on a search change, or keep the documented watcher | gallery finding F30 |
| Screens | A `type="none"` transition response to a JSON request carries no messages (as Java), so a background form relies on `background-message`; answering the messages JSON on a `none` response would let the transition's own `<message>` show | gallery finding F3 |
| Lint | `moqui lint --screens` does not check attributes against the XSD, so an unknown attribute (`display default-text`) passes silently | gallery finding F24 |
| Screens | The html render mode writes an `icon` attribute (`ScreenHtml.js` `icon()`) and an icon menu image of `subscreens-menu` (`menuLink`) as CSS classes, so a `sym_o_*` name shows nothing there; the qvt mode and the html app list draw it (Material Symbols, style-guide §5.6) | found 2026-10-18 |
| Entity | `moqui.basic.EnumerationAndType` (alias-all view) returns a null `lastUpdatedStamp`; check the alias-all handling of the stamp fields against Java | gallery finding F23, unverified |
| REST | The ADMIN demo user gets 403 on `/rest/e1/moqui.basic.EnumerationType` (`AT_ENTITY … view`); check the REST entity authorization seed | gallery finding F27, unverified |

## Found during the IronStone build (2026-10-19)

Found while building the suite shell, the notifications and the Mantle services of ironstone-boss `plan/impl-drawers-commons.md`.
The child-list dates of the Postgres search, condition objects in a script's context and the shell breadcrumbs were fixed on
2026-10-19 (M2 framework). Open:

| Area | Item | Source |
|---|---|---|
| Search | Dates in child lists are now stored as ISO-8601 text when indexed (`ObjectMapping::type_dates`); a document indexed before keeps its epoch-millis text, so a child-date filter on it still fails on Postgres until its DataFeed is indexed again (`index#DataFeedDocuments`). mr-mantle-usl `search#Universal` can now limit to current party rows (its TODO, `parties.partyThruDate`) | M2 framework |
| Notifications | `approve#` and `refuse#NotificationMessage` mark only the deciding user's `NotificationMessageUser` row viewed; the other recipients of the same approval keep it waiting until they open it (their approve service call answers with the record's state) | M2 framework |
| Notifications | The daily digest is one list per user, newest last; no grouping by topic or app yet, and no per-user hour (the job runs at 06:00 server time) | M2 framework |
| Screens | In a server-static screen a link url with `${}` is bound as text (`exp`), so a relative path in it is not resolved against the screen; a plain url and its query are | M2 framework (number-card links) |

## Parity gap review

The entity and service modules (Parity-Mapping's Entity and Service sections) were reviewed phase by phase against their Java sources on 2026-10-16: [EntityService-Parity-Review](archive/EntityService-Parity-Review.md), 8 phases, with the accepted differences in [Parity-Diffs](Parity-Diffs.md). The rest of [Parity-Mapping](Parity-Mapping.md) (moqui-lib, moqui-db, context and resources, user and session, transactions and drivers, script and templates, screens and macros, web and the HTTP host, search, notifications and cluster, mail and the tool components, tooling, browser host) has had no phase review yet; the threads to carry through one are [Parity-Mapping, Cross-cutting threads](Parity-Mapping.md#cross-cutting-threads).

The review's open findings (P1-14, P1-15, P3-20, P3-21, P4a-18, P5-19) were fixed on 2026-10-18; what differs from Java in the fixes is in [Parity-Diffs, Remaining-Work fixes](Parity-Diffs.md#remaining-work-fixes-2026-10-18).

## Code quality review leftovers

The [Code-Quality-Review](archive/Code-Quality-Review.md) of 2026-10-14 was resolved on 2026-10-15 (six fix phases, about 170 new tests), and the Lows, live checks and limits it left were worked on 2026-10-18. Still open from its [Resolution](archive/Code-Quality-Review.md#not-done-or-left-as-noted):

- **Static files:** a range is read alone (`Range` is answered), but a whole-file response is still read into memory: the response body is bytes, not a stream.
- **Process-wide global state:** the notify bus and its origin id, the transaction deadline slots and lock tracking, the QZ signer, and `LAST_BOOTED`. One factory runs per process outside tests; `LAST_BOOTED` and the single deadline timer are process-level by design, and the notify bus and lock tracking have no factory handle at their call sites.

## Mantle conversion follow-up

mantle-usl services are hand ported to JavaScript, and `moqui lint --groovy` is at 0 flagged sites ([ConvertProcess](archive/ConvertProcess.md)). The specs run under `moqui test mantle-usl` ([MantleFlowTests](archive/MantleFlowTests.md)): 344 features in 43 specs, 751 of 762 services called (`moqui test mantle-usl --coverage`).

- **The 11 services never called:**
  - three call Google (`GooglePartyServices`) and one the reCAPTCHA service (`create#ContactRequestReCaptcha`), which need a network;
  - four are `TenantSubscriptionServices`, left for the deprecated multi-tenant mode (the `fulfill#ProductSubscriptionResource` it builds on is covered);
  - `TaxServices.calculate#InvoiceSalesTax` is an empty TODO upstream;
  - `ReturnServices.returnAndResell#Shipment` never places its new order upstream (a spec documents where it stops);
  - `PartyServices.send#DataDocumentNotifications` needs the data feed of a search backend.
- **Unfinished upstream, kept as they are:** `generate#Efw2File` writes no RT, RU, RV or RF records and never sets `fileText`; `autoApply#Payment`, `get#VariantProductsInfo`, `calculate#SalesTaxLocal` and `post#WorkEffortCost` are stubs.

Migrating other components' scripts is outside this repository's gates.

## Related documents
[Future-Plans](Future-Plans.md) (digest of the unbuilt [proposals](proposals/)), [Performance-Summary](Performance-Summary.md) (the next phase), [Parity-Mapping](Parity-Mapping.md) and [Parity-Diffs](Parity-Diffs.md) (reference for the gap review). The index is [README](README.md).
