# Remaining work

Open items only, grouped by domain; trimmed and reorganized on 2026-10-25. Built: phases 0–12 and gates 1–4 of [Implementation-Phases](archive/Implementation-Phases.md), and Phase 13.1 to 13.14 of [LLM-A2A-Assist](LLM-A2A-Assist.md). Each item says what is missing and links to where the detail is; the build records are in the linked plans and guides, not here.

What is not ported on purpose (JSON-RPC, the Groovy and Java runners, the database drivers not in this tree, `service-group`, Swagger 2.0 and RAML, Hazelcast, the embedded SMTP server, vuet and `/vapps`) is in [Parity-Mapping, Excluded on purpose](Parity-Mapping.md#excluded-on-purpose); the non-SQL entity datasources (`ElasticDatasourceFactory`, OrientDB) are a proposal in [Future-Plans](Future-Plans.md). Differences from Java that are kept on purpose are in [Parity-Diffs](Parity-Diffs.md).

## Agents, Assist, A2A, MCP and the gateway

Phase 13 is built ([LLM-A2A-Assist](LLM-A2A-Assist.md); guides [Assist](../docs/Assist.md), [A2A](../docs/A2A.md), [MCP](../docs/MCP.md), [Gateway](../docs/Gateway.md)). Every check ran on the scripted fake provider, fixture or loopback servers only.

| Area | Item | Source |
|---|---|---|
| Live models | No check has run against a live model: the evaluation suites on Qwen 3.8 27B (at least 80%) and on an 8B model in JSON mode (60%, at most 10% wrong calls), with their `baseline-local-qwen.json` and 8B baselines; live embeddings, transcription and text actions. The provider stream fixtures (`llm/provider/testdata/`) are hand-written from the documented formats; replace them with recorded streams when an endpoint is at hand | 13.1, 13.3, 13.7, 13.13, 13.14 |
| Drawer | A record chip map from the server; "Let Assist save it" on a hand-off; Summarize earlier messages; Chromium smokes for the 13.14 drawer extras (they have node tests only); TUI decisions over `--server` (a key login cannot decide) | docs/Assist.md, docs/TUI.md "Assist" |
| Canvas | The Wide splitter, Page mode and phone tabs; a canvas `Lookup` against real option transitions in the Chromium smoke; an admin field for `AgentDefinition.canvasDialects`; the worker renderer for `vue-sfc`, required before enabling it for agents that read untrusted content (self-navigation and WebRTC put back by the component's own code are only mitigated) | 13.6, 13.8 |
| Window | Reading the context length the server reports; did-you-mean for screen paths (the `screen` tool refuses a bad path with no suggestion) | 13.7 |
| Learning | Lessons from fail-then-succeed and from repeated Adjust | 13.14 |
| Chat agents | Feedback from reactions; manager/worker agents (`parentRunId` is unused); form questions in rooms | 13.9 |
| A2A server | OIDC JWT auth; an `LlmApiClient` `a2a` scope; an A2A tasks admin screen; A2A in `clean#AgentData`; CORS for `/a2a` | docs/A2A.md "Not yet" |
| MCP server | OAuth 2.1 with Moqui as the authorization server and the `LlmApiClient` `mcp` scope; `subscriptions/listen` and `listChanged`; resources and prompts; SSE answers; legacy `initialize` clients; an approval path for clients without URL elicitation | docs/MCP.md "Not yet" |
| Remote clients | Long remote tasks as a RemoteTask pause resumed by push; per-user OAuth sign-in for MCP servers; form elicitations and legacy in-stream requests; remote resources and prompts | 13.12 |
| Gateway | Protocol translation (Chat Completions to Responses or Anthropic models); interrupting a call's blocking upstream request when its caller leaves; making a login key for a client in the admin screen (shown once); cluster-wide requests per minute (counted per node) | docs/Gateway.md "Not built" |
| Apps | The active organization and organizations as a Mantle user-facts provider, and Mantle skills in mr-mantle-usl, MarbleERP and SimpleScreens (no component has a `skill/` directory yet) | [LLM-A2A-Assist](LLM-A2A-Assist.md) §12 |

`allow-remote` on services is not parsed; no remote path calls services by name, so it is documentation only. Provider calls keep the sim-allow exception on the outbound client.

## Mail

Built through M5f: [Mail-Inbound](../docs/Mail-Inbound.md), [Mail-OAuth](../docs/Mail-OAuth.md); ironstone-boss `plan/impl-drawers-commons.md` §5 M5.

| Area | Item | Source |
|---|---|---|
| Replies | The automatic reply counts as internal a user's address, the domain of a hosted or shared account, or one of its `internalDomains`, and answers only mail with the account's or an identity's address in To or Cc (not mail an alias forwards) | M5c, `send#OutOfOfficeReplies` |
| Unsubscribe | One-click unsubscribe refuses private addresses and names written as such, not a public name that resolves to a private address | M5c, `unsubscribe#Message` |
| Sync | Not yet run against a real account (a Gmail app password). A folder gone from the server keeps its local messages (`remoteRemovedDate`); a label taken off on the server stays here (keywords only add); POP3 reads no sync window, newest first, and does no STLS; IMAP signs in in plain text when the server offers no STARTTLS (logged); an IDLE wake-up in the moment before an IDLE starts can wait for the renew | M5d, `service/mail/sync.rs`, `idle.rs` |
| OAuth | Not yet run against real Google and Microsoft clients (the Testing 7-day limit and Google verification) | M5e, docs/Mail-OAuth.md |
| Inbound SMTP | No TLS client certificates (the allow-list is the gateway's identity); no BINARYMIME; no failure or delay DSNs of its own; the delivered report waits on the outgoing server for up to 30 seconds inside the store, before the listener answers | M5f, `moqui-host/src/smtp.rs`, `service/mail/delivery.rs` |
| Sending | A held send goes at the job's next minute unless the client calls `send#DueDrafts` when its hold ends; a send stopped between its claim and its result stays ES_READY without an error and is not retried | `EmailClientServices` |
| Accounts | `delete#EmailAccount` with deleteLocalCopies runs in the request (a job for large accounts); two first calls of `ensure#InternalAccount` at once could make two | `EmailClientServices` |
| Capture | The mail capture records at the SMTP step of `send#EmailMessage`, so a send with no `EmailServer` or an empty SMTP host is not captured; IMAP and POP have no capture. mr-mantle-usl `InboxThreads.spec.js` still makes a send fail and calls `record#SendResult`, which `ec.factory.getTool("MailCapture").enable()` would replace | `service/mail_capture.rs` |

## Chat and presence

| Item | Source |
|---|---|
| Calls (`ChatRoomMember.inCall` is not set); a link preview's picture, through an image proxy so no reader's address leaks (previews are text only, `service/chat_link.rs`); BOSS's chat screen rendering `m-assist-card`, with a Chromium smoke of the card | ironstone-boss `plan/impl-drawers-commons.md` §5 M10; 13.9 |

## Knowledge and wiki

| Item | Source |
|---|---|
| Relative links in a wiki page are not resolved when the page renders: `sri.baseLinkUrl(url)` is kept on the render (`RenderState.base_link_url`) but the markdown renderer never reads it, so a relative link is relative to the screen URL (Java resolves against `getBaseLinkUri()`) | `screen/render.rs`; docs/Revision-Management.md (Wiki pages) |

## Files and drives

| Item | Source |
|---|---|
| Previews and extracted text of docx, pptx and odt (deferred); PDF fonts, cmaps and JPEG 2000 or JBIG2 images (pdf.js `standard_fonts`, `cmaps` and `wasm` are not vendored: License-Review) | ironstone-boss `plan/impl-drawers-commons.md` §5 M8 |
| WebDAV: locks are per server (a client stays on one, `dav/locks.rs`), dead properties are not kept, no quota properties | [Sync](../docs/Sync.md) |

## Calendar and contacts

| Item | Source |
|---|---|
| Not built in the `VCard` and `ICalendar` tools and the CalDAV and CardDAV servers (`crates/moqui-core/src/pim`, `dav/pim.rs`): sync-collection (clients keep in step by ctag and entity tags), scheduling (CalDAV `schedule-*`), free-busy-query, attendees written by a client, iTIP processing of a received REPLY or COUNTER, VJOURNAL and VFREEBUSY, `RANGE=THISANDFUTURE` overrides (read as one instance), RSCALE and SKIP (RFC 7529), vCard PHOTO and other binary values, IMPP, ANNIVERSARY, GENDER and related cards. A VTIMEZONE that no IANA zone matches keeps its rule as a POSIX TZ text in the event's `timeZone`, which `expand` reads back | [Sync](../docs/Sync.md); M4, M7 framework |

## Git and git hosting

Built: [Git-Hosting](../docs/Git-Hosting.md), [Revision-Management](../docs/Revision-Management.md).

| Area | Item | Source |
|---|---|---|
| Push | No push to another server: gitoxide has none yet. A `db` repository is published by `export#GitRepository` and the git command line; add `push#GitRepository` when gix supports it | `resource/git/fs.rs` |
| Transport | SSH and `file://` fetches run the system `ssh` and `git-upload-pack`; an in-process SSH transport over `russh` (already linked for the SSH server) would drop that | same |
| Formats | SHA-1 repositories only; no submodules, tags as versions, or signed commits | same |
| Hosting | Forks; merges reading `.gitattributes` (merge drivers); revision expressions (`main~1`); a reviewer's view right is not checked when added (`update#MergeRequestReviewers`); merge request notifications are the app's (Mantle and BOSS topics); no `GitUpdate` is spared for a `git://` write now that the framework's own rules listen to writes | `resource/git/hosting.rs`, `service/git_hosting.rs` |
| Hosting | Not served: partial clone (`filter blob:none`), `report-status-v2`, `push-cert`, the dumb HTTP protocol, anonymous read of a public repository | `resource/git/server/` |
| Hosting | `SSH_GIT` is granted to ADMIN only: `has_permission` does not count ALL_USERS, so a grant for every user needs a group of its own | `framework/data/SecurityTypeData.xml` |
| Storage | No repack of `local` repositories: each push adds a pack, and gix's object database slots fill over many pushes (`gc#GitRepositories` repacks `db` ones only; a `local` one could be repacked through a pack of its own objects) | `resource/git/server/pack_in.rs` |
| Storage | A `db` delta chain is read a level of bases per query; on Postgres a recursive CTE could read a cold chain in one | `resource/git/db.rs` |
| Storage | Content blobs are in the database only (`ContentBlob.storageKind` `db`); a file system or S3-like backend behind the same store for the chunks | `resource/blob.rs` |
| Storage | A `local` repository's object written through `git://` over 8 MiB is a loose object of its size (only a `db` repository keeps large objects as content blobs); keep large files in LFS | `resource/git/fs.rs` |
| LFS | A `remote` mirror does not fetch LFS objects; no transfer adapters other than basic (`git-lfs-transfer` over SSH, multipart) | `resource/git/server/lfs.rs` |

## Search

| Item | Source |
|---|---|
| Universal search result links go to the generic entity edit screens until the apps have record screens; the per-document hook predicate is not built | [SearchFacade, REST service and UI](archive/SearchFacade.md) |
| Postgres broad queries (common words: rank, facets, proximity) take tens to hundreds of ms at 50k documents; candidates are a stored per-field `tsvector`, a ranked-row cap, and measuring cover density on a real corpus. Hybrid `k` and weights wait on a relevance set | [Performance, Search](Performance.md) |
| Semantic retrieval has the development hash embedder and `LlmEmbedder` (`embedder="llm"`, a model server's `/v1/embeddings`); there is no built-in local model, and no live embedding model has been run | [Implementation-Phases, Phase 9](archive/Implementation-Phases.md); docs/Gateway.md |
| Dates in child lists are stored as ISO-8601 text since 2026-10-19; a document indexed before keeps epoch-millis text, so a child-date filter on it fails on Postgres until its DataFeed is indexed again (`index#DataFeedDocuments`). mr-mantle-usl `search#Universal` can now limit to current party rows (its TODO on `parties.partyThruDate`) | `ObjectMapping::type_dates`; `UniversalSearchServices.xml` |

## Screens, client and shell

| Item | Source |
|---|---|
| `columns type="mobile"` is chosen when the page loads (width at most 600 px, or the Quasar platform), not on resize; the style guide defines phone by width | gallery finding F22; `WebrootVue.qvt.js` |
| A server-static `qjs`/`qvue` component is cached and not remounted on a query-only navigation (`m-subscreens-active`); a component whose state is in the URL must watch `$root.currentParameters`. Decide: remount on a search change, or keep the documented watcher | gallery finding F30 |
| A `type="none"` transition response to a JSON request carries no messages (as Java), so a background form relies on `background-message`; answering the messages JSON on a `none` response would let the transition's own `<message>` show | gallery finding F3; `screen/render.rs` |
| The gallery's Editor archetype uses `text-area editor-type="md"`, and its label says the co-editing component is not built; `m-coedit` exists (IronStone Notes and Knowledge use it), so the Editor should use it | `example/screen/Gallery/Editor.xml` |
| `m-html-view` treats a `//` image as https and opens every link (a `mailto:` too) in a new window rather than handing it to the composer; the html render mode shows the HTML cleaned to the safe set (no styles, no images but `data:`) at a fixed height | M3 and M5 framework |
| `container-split` keeps its pane widths per id; it has no collapse of the side, no keyboard resize (Quasar's separator takes none) and no list-detail focus handling (Escape back to the list). The wizard's step form is found in the DOM (the first `m-form` of the step outside a list or a dialog), so a step with two forms that should both post has to use one; the wizard does not save a draft itself (each step's transition does) | M4 framework |
| `sri.getShellProviders` is kept per session for up to ten minutes: a grant changed by an admin, or a quick-create or key target screen that changes, shows in another session at once and in the same one after that time (the app root screens are checked every time) | `screen/cache.rs` |
| The 390 px header check (`gate3_browser` tray test) runs without theme plugins: MrSimpleScreens' `ActiveOrgNav.qvue` adds a button and its organization's pseudoId as a label, which a long id can still push past a phone's width | framework gaps 2026-10-19 |

## Terminal client

| Item | Source |
|---|---|
| Live checks: the browser terminal's keys and paste in Chrome, Firefox and Safari; the mouse (clicks, the wheel and drag to copy in kitty and GNOME Terminal over SSH, in local mode, and OSC 52 in the browser terminal in Chrome and Firefox); F-keys, Alt and paste in xterm, kitty, GNOME Terminal, tmux and the Linux console | [Screen-TUI-Impl](archive/Screen-TUI-Impl.md) §5 Phase 4 findings, §7 |
| Per-user key bindings and a screen for them (D11); a two-way rich-text editor (3.8; Markdown is styled source); a `diagram` component (Revue's Mermaid widget does not draw a status flow); board sums and the calendar's create and resize in list views; the upstream Revue asks of §7 | same |

## Tooling: lint and XSD

| Item | Source |
|---|---|
| `moqui lint --screens` does not check attributes against the XSD, so an unknown attribute (`display default-text`) passes silently (unknown elements are flagged) | gallery finding F24; `script/screen_lint.rs` |
| `xml-screen-3.xsd` does not compile in libxml2 (`xmllint --schema`): "the content model is not determinist" for the complex types at `container-dialog` and `dashboard`, so screens cannot be validated against it with xmllint (they still parse, and the server does not read the XSD) | found 2026-10-19 |

## Sessions, notifications and security

| Item | Source |
|---|---|
| A stored NotificationMessage keeps the message's own `showAlert` (`notify/message.rs`), but nothing reads the stored value: the live sockets get the in-memory message. A client or screen that lists stored notifications (unread alerts after a login, a notification history) should read it | PR #134 |
| Login key, app password and external (SSO) logins leave `UserLoginHistory` rows, at most one a minute per user on a node. If automation logging in with a key on every request makes high volumes, record automated logins differently (sampled, counted, or in their own entity) | PR #63 follow-up; `user/login.rs` |

## Apps: IronStone BOSS, MarbleERP and Mantle

| Area | Item | Source |
|---|---|---|
| IronStone | Commons, Comms and Projects are built; the other 13 business apps hold placeholder screens (they render in qvt, pass `moqui lint --screens`, and `test/AppStructure.spec.js` renders each with a demo record). For Projects, what remains is `impl-projects.md` §7 Later and the open findings of its §6 | ironstone-boss `plan/impl-drawers-commons.md`, `plan/impl-projects.md` |
| IronStone | Sales/Reports (`SalesSummary`, `SalesAnalysis`) has no spec that renders it. Marking the BOSS data screens outside Projects and Commons `server-static="qvt"` waits on their specs, which check data in the merged qvt output that a server-static screen leaves to `/actions` | same |
| Tools | The tools screens still call the `StringUtilities`/`WebUtilities` globals (`runtime/base-component/tools/screen`, e.g. `EntityDataFind.xml`) | — |
| MarbleERP | Acceptance work after the migration: wiki fixtures, exhaustive role and organization isolation, CSRF, every mounted QVT interaction, physical checks and scanners | [MarbleERPMigration, Limits](archive/MarbleERPMigration.md#limits) |
| Mantle | `moqui test mantle-usl` calls 751 of 762 services. Not called: three Google ones (`GooglePartyServices`) and `create#ContactRequestReCaptcha` (need a network); four `TenantSubscriptionServices` (deprecated multi-tenant mode); `TaxServices.calculate#InvoiceSalesTax` (empty upstream); `ReturnServices.returnAndResell#Shipment` (never places its order upstream); `PartyServices.send#DataDocumentNotifications` (needs a search backend's data feed). Unfinished upstream and kept as they are: `generate#Efw2File` (no RT, RU, RV or RF records, no `fileText`), `autoApply#Payment`, `get#VariantProductsInfo`, `calculate#SalesTaxLocal`, `post#WorkEffortCost`. Migrating other components' scripts is outside this repository's gates | [ConvertProcess](archive/ConvertProcess.md), [MantleFlowTests](archive/MantleFlowTests.md) |

## Wasm browser host

| Item | Source |
|---|---|
| No login flow: calls run with authorization disabled, except renders (`crates/moqui-browser/src/host.rs`) | [Wasm, Status](archive/Wasm.md) |
| No lock between tabs: without pthreads the WasmFS OPFS backend opens a file through its async access handle, so a second tab opens and writes the database the first has open (`scripts/wasm-chrome-smoke.mjs` warns), and two writers can corrupt the SQLite file. A Web Locks (`navigator.locks`) lock per database name in the worker would make the second tab wait or refuse | same |
| The WebSocket check runs against a Node mock of `/notws`, not moqui-host. A live-host mode needs the package served from moqui-host's origin, a signed-in session, and checks of what the real `/notws` does (the mock's `echo:`, `drop` and `Ack` replies are its own) | same |

## Tests

| Item | Source |
|---|---|
| The Java vs MR-Moqui comparison harness and its diff report (a gate artifact) has no code; only search-backend comparison tests exist. The Java runtime must stay runnable | [Implementation-Phases, Tests](archive/Implementation-Phases.md) |
| 90 of the 143 Java security proofs are pending under `moqui test-client` (34 ported, 19 not applicable; `runtime/base-component/framework/test/client/README.md`): multipart, HMAC (`/rest/sm`) and timing (the tarpit, on a server with the tarpits on) are the patterns still to build | [ClientTestHarness](ClientTestHarness.md) P3 |
| A full `moqui test` run (all components, 1540 features) runs out of memory in QuickJS about 930 features in: ironstone-boss screen renders then fail with `out of memory` (about 63 features), though the same specs pass on their own. Something held per run grows: find it (contexts, compiled screens, cached finds) or give each spec file its own runtime | `./moq test-all`, 2026-10-25 |
| `example/test/GitHosting.spec.js` "a merge request waits for its review and check, then merges" fails about 1 run in 10: `merge#MergeRequest` right after `create#GitCommitStatus` for the head commit still says "checks not passed: ci/build". Look at how `latest_statuses` (`service/git_hosting.rs`) sees the new row, and whether the merge request's head and the branch head can differ there | `./moq test-all`, 2026-10-25 |

## Framework internals

- **Process-wide global state:** the notify bus and its origin id, the transaction deadline slots and lock tracking, the QZ signer, and `LAST_BOOTED`. One factory runs per process outside tests; `LAST_BOOTED` and the single deadline timer are process-level by design, and the notify bus and lock tracking have no factory handle at their call sites. From the [Code-Quality-Review](archive/Code-Quality-Review.md#not-done-or-left-as-noted).

## Java functionality review

The entity and service modules ([EntityService-Parity-Review](archive/EntityService-Parity-Review.md)) and the screen and web parts ([ScreenWeb-Parity-Review](archive/ScreenWeb-Parity-Review.md)) were reviewed phase by phase against their Java sources, with the accepted differences in [Parity-Diffs](Parity-Diffs.md). The rest of [Parity-Mapping](Parity-Mapping.md) (moqui-lib, moqui-db, context and resources, user and session, transactions and drivers, script and templates, search, notifications and cluster, mail and the tool components, tooling, browser host) has had no phase review yet; the threads to carry through one are [Parity-Mapping, Cross-cutting threads](Parity-Mapping.md#cross-cutting-threads).

## Related documents
[Future-Plans](Future-Plans.md) (digest of the unbuilt [proposals](proposals/)), [Performance-Summary](Performance-Summary.md) (the next phase), [Parity-Mapping](Parity-Mapping.md) and [Parity-Diffs](Parity-Diffs.md) (reference for the review). The index is [README](README.md).
