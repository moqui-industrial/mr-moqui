# Framework Parity Differences

Companion to [Parity-Mapping](Parity-Mapping.md). Where that file says which Java, Groovy, XML and FreeMarker source each Rust module was ported from, this one records the behaviour differences inside those ported features that are deliberate: each has a rationale and a comment in the code that records it. Revisit a row only if its rationale changes. This is not a worklist.

What is not here:

- **Whole features that are not ported** (JSON-RPC, Groovy runners, Hazelcast, Shiro, FreeMarker and the rest): [Parity-Mapping, Excluded on purpose](Parity-Mapping.md#excluded-on-purpose).
- **The redesigns** (SearchFacade, server-static qvt, feed registration, JavaScript, html-print, database cluster coordination and sessions, keys as configuration): the root [README, Significant Redesigns](../README.md#significant-redesigns-vs-moqui-java).
- **Divergences that are still open**: findings in [Remaining-Work](Remaining-Work.md), each linking to the review that found it.

Rows keep the ID of the review that recorded them, so the archived review has the context (`P1-07` is finding 7 of phase 1 of [EntityService-Parity-Review](archive/EntityService-Parity-Review.md); its phase sections say what else was compared). A later review adds its accepted differences here under the matching heading, with its own IDs, and leaves a pointer in its own `## Accepted differences` section when it is archived.

Paths are as in the originating review: Rust locations relative to `crates/moqui-core/src/` unless they start with `moqui-db/`, `moqui-lib/` or `moqui-host/`, or `runtime/`. Line numbers are as of that review's last commit (2026-10-16 for the entity and service review).

## Entity and service review (2026-10-16)

From [EntityService-Parity-Review](archive/EntityService-Parity-Review.md), phases 1 to 8: `crates/moqui-core/src/entity/`, `crates/moqui-core/src/service/` and the `moqui-db` layer under them. AGENTS.md failure tolerance is the policy that an independent piece which fails is logged and the rest goes on.

### Entity

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P1-07 | entity condition | A raw `Where` fragment is wrapped in `(...)`; Java appends it verbatim, so boolean precedence can differ (`moqui-db/src/sql/condition.rs:659-665`) | Safer: an `OR` in the fragment cannot bind across the conditions around it; comment at the site |
| P1-13 | entity list | Rust `EntityList` has only len/get/iter/values; Java has `filterByAnd`, `orderByFields`, `getFirst` and more (`entity/value.rs:243-277`) | Rust callers use slice/iterator methods; the script runtime's list has the helpers scripts call (`filterByDate`, `getPartialList`, `first`), and no converted component uses the others; doc comment at the type |
| P2-08 | entity find | Distinct and select-field finds bypass the entity cache; Java caches them by reading every field, so a cached entity answers with whole, non-distinct rows (`entity/find.rs:238-244`) | Rust returns the rows and fields asked for; comment at `bypasses_cache` |
| P2-10 | entity find | `iterator()` under `requireSearchFormParameters` returns an empty iterator; Java returns `null` (`entity/find.rs:524-530`) | Rust has no null iterator; empty is the safe equivalent |
| P2-12 | entity schema | JSON schema envelope redesigned: `$defs`/draft 2020-12, PK `required`, keyed by full name vs Java `definitions`/`anyOf`/pagination, draft-04, `_entity`+`default`, short-or-full name (`entity/schema.rs:27,86-125`) | Deliberate modernization; relationships and `enum` now follow Java inside it (P2-06/P2-07 fixed) |
| P2-14 | search form | `_not` with `_ic` on `equals` compares ignoring case; Java throws, since `ignoreCase()` on the `orNull` join condition is unsupported (`entity/search_form.rs:269-272`) | A search a user can type should not fail; comment at the site |
| P3-06 | datasource / DDL | No table, constraint or index name is schema-qualified: Postgres sets the session `search_path` to `schema-name` (`moqui-host/src/db/entity_db.rs:202-207`), `use-schema-for-all` (only the DB2 profiles) is unread, and a MySQL `schema-name` is logged and not used (`entity/datasource.rs:222-229`) | Same placement on Postgres without qualifying each statement; no DB2 driver; on MySQL Java's `schema.TABLE` is another database, which the jdbc-uri names. `use-schemas="false"` is now honored (no search path, `entity/datasource.rs:178-184`); comments at both sites |
| P3-11 | DDL | `fk-style="name_fk"` (the MySQL profiles) is not followed: always `ADD CONSTRAINT name FOREIGN KEY` (`moqui-db/src/sql/statement.rs:562-571`) | MySQL 8.0.16 and later ignore the `name_fk` name and call the key `{table}_ibfk_N`, so the check that goes by name (P3-21) would add it again on every check; doc comment at `create_foreign_key` |
| P3-14 | entity facade | A view built at runtime (a `DbViewEntity`, `DataDocument.<id>`) reads its rows outside the caller's transaction, which is suspended, and is never cached (`cache="never"`, though a `DbViewEntity` says `Y`); Java reads in the transaction and caches a `cache="Y"` view (`build`, `entity/derived.rs:154`) | A failed read (a database without the table) cannot mark the caller's transaction for rollback, as the loader asks whether each data file element is an entity inside its file transaction; rows that transaction wrote are seen once it commits, when the overlay is dropped again. The entity cache keeps no member-to-view map for views built here and would keep a dropped view's finds. Comments at the site and in the module doc |
| P3b-10 | feed cache | Feed-info cache invalidated on write vs Java 900s TTL (`entity/cache.rs:838-845`) | Rust is more current |
| P4a-04 | data loader | A JSON `binary-very-long` string is Base64-decoded (text that is not Base64 is its UTF-8 bytes); Java's JSON reader stores the text's UTF-8 bytes (`entity/load_feed/json.rs:290-292`) | This port's data writer writes bytes in JSON as Base64, so its writer and loader round-trip; Java's JSON writer has no text form for bytes (`JsonBuilder` of a `SerialBlob`), so its pair never did. Comment at the site |
| P4a-19 | data loader | XML and JSON `binary-very-long` text is trimmed before the Base64 decode; Java decodes it as it is, so Base64 with whitespace around it becomes the text's UTF-8 bytes (`entity/load_feed/loader.rs:327-331`) | Base64 laid out on lines of its own decodes to the bytes meant; Java's writer puts it in CDATA with no whitespace, so both read that the same. Doc comment at `decode_binary` |
| P4a-20 | data loader | An empty CSV header cell (a trailing delimiter) is no column; Java's field lookup fails the file on it (`entity/load_feed/csv.rs:94-100`) | A trailing delimiter is no reason to fail a file; a non-empty name that is not a field still fails it (P4a-16). Comment at the site |
| P4b-07 | data feed | `get#DataFeedDocuments` honors `thruUpdateStamp`; Java always passes null | Bug fix; comment at runtime `EntityServices.xml:35-36` |
| P4b-08 | ETL | `useTryInsert` stores with `createOrUpdate` where the database is `never-try-insert` (Postgres); Java's ETL loader tries the insert anyway (`entity/etl.rs:92-101`) | Safer: on Postgres the failed insert aborts the transaction, so Java's update after it fails; the data loader's `handleValue` has the same guard. Comment at the site |
| P4b-09 | data writer | `isoDateTime(true)` formats dates in JSON and XML; Java ignores it (`TODO`) (`entity/query_rest.rs:1550-1582,1715-1732`) | Implements Java's TODO |
| P5-14 | entity REST | Request `pageSize` capped at `entity_rest_max_page_size` (1000); Java caps nothing | Documented `entity/query_rest.rs:532-551` |
| P5-16 | entity value | A `pq-expression` alias whose expression fails is logged with the alias and entity names and reads as null; Java's `get` throws an `EntityException` (`eval_pq_alias`, `entity/value.rs:1202`) | AGENTS.md failure tolerance: one bad expression does not fail the read of a row, its map, or a form-list of such rows; doc comment at `eval_pq_alias` |

### Service definition, validation and call

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P6a-05 | service tools | Service detail view is a rebuilt node (`method` null, no `groovyString`, some attributes dropped) | Documented `service/info.rs:194-195` |
| P6a-12 | service validation | A `required="disabled"` parameter is skipped, so validating drops a value passed for it, and an implemented one is not copied (`service/validate.rs:48-52`, `service/define.rs:1434-1441`); Java only makes it not required and still converts the value | The XSD says it behaves "as if the parameter did not exist", for overriding an earlier parameter; dropping it keeps an undeclared value from the service. Comment at the site |
| P6a-19 | service validation | `val-or` passes when one child passes and `val-not` fails when one passes, with a message (`service/validate.rs:704-723`); Java's `validateParameterSingle` adds each failing child's error even when the `val-or` passes, and adds none for a failing `val-not` | Java's makes a `val-or` with any failing branch fail the call and a failing `val-not` pass it; unused in `runtime/`. Comment at the site |
| P6a-20 | service validation | `text-email` checks the top-level domain's form (letters first, two or more long), not the IANA list, and a Unicode host is not valid; `text-letters` takes Unicode alphabetic characters, a little more than `Character.isLetter` (combining vowel signs, letter numbers) (`service/validate.rs:684-688`, `:789-875`) | No list of top-level domains to keep current and no IDN library in moqui-core; doc comment at `is_email` |
| P6b-08 | semaphore | Row cleared only while `lockThread` equals Rust's owner label | Documented `service/semaphore.rs:82-83,247-248` |
| P6b-14 | service call | A call returns only the declared out-parameters (all when none are declared), and inline/script bodies have no `result` map (`service/call.rs:951-968`); Java returns the runner's map, the `result` map's entries included | Keeps a body's other values from REST and remote callers; a ported script uses `result` as a name of its own (`SearchServices.xml`). Doc comment at `project` |
| P6b-15 | service call | An authorization failure in a service's body rolls back the transaction the call began (`tx.run`, `service/call.rs:516-520`); Java's `callSingle` rethrows `ArtifactAuthorizationException` and its `finally` commits what the body wrote before the refusal | Safer: a refused call leaves nothing written. Comment at the site |
| P6b-16 | authorization | An `authzServiceName` service that leaves errors fails the authorization check (`authz_service_type`, `user/authz.rs:403-419`); Java goes on with the record's own type, which can be `AUTHZT_ALLOW` | AGENTS.md: stay strict where it protects security, so a failing authz service never authorizes. Doc comment at `authz_service_type` |

### Service REST, OpenAPI, remote REST and tools

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P8a-14 | remote REST | Substituted `${param}` values percent-encoded; Java does not URL-encode | Doc comment `service/remote.rs:113-114` |
| P8a-16 | service REST | No-user denial returns 401; Java throws (403) | Comment `service/rest.rs:523-524` |
| P8a-17 | print | `print#ScreenDocument` refuses a `contentType` other than `application/pdf` with an error, and renders in the one webapp, `webroot`, whatever `webappName` says (`service/tools.rs:207-214`); Java's FOP writes other output types and renders in the named webapp | The PDF renderer makes only PDF, so another type cannot be made, and this port serves one webapp, `webappName`'s default; comment at the site |
| P8a-23 | service REST | Authc message says "operation"; Java has the "operaton" typo | Don't copy the typo |
| P8a-25 | DataDocument notifications | An empty `userId`, `fromUserId` or `toUserId` in a document names no user (`find_nested`, `service/tools.rs:615-632`); Java's `findAllFieldsNestedMap` adds it | Java then writes a `NotificationMessageUser` for user `""`, which fails the send for every user of the document; doc comment at `find_nested` |
| P8a-27 | service REST | A `<method>` with no `type`, no operation element, or an operation with no `name` (or no `operation`) is logged and left out, and the rest of the file is served (`service/rest.rs:326-344`); Java keys one with no type by `null`, registers a service or entity handler with no name, which fails when called, and fails the whole file for a method with no child | AGENTS.md failure tolerance; comment at the site |
| P8a-28 | service OpenAPI | A REST service that is not defined, or whose parameters do not expand, is logged and documented without parameters, and a parameter whose `entity-name` or `field-name` is not defined is logged and has no `enum` (`service/rest.rs:910-923`, `:1049-1067`); Java's Swagger throws for the whole document | AGENTS.md failure tolerance; doc comments at `service_parameters` and `parameter_enums` |
| P8a-37 | print | The IPP wire differs from cups4j's: a job carries `document-format`, the job's `contentType`; the `printer-uri` is the one the server reported; a hold, release or cancel names the job by `printer-uri` and `job-id`; a CUPS-Get-Printers refused for any reason but not-found is an error; and port 443, or a host that names `https://`, is TLS (`moqui-host/src/ipp.rs:20-27`; `endpoint`, `:53-79`; `:292-302`). cups4j sends no `document-format`, so the server types the document; writes the server's host with no port and the path; posts a `job-uri` to `/jobs/<id>`; reads any refusal as no printers; and always connects `http://host:port` | The same server and printer are reached; the type a job was stored with is what the server is told, and a refusal is reported rather than read as an empty server. Module doc and a comment at `document-format` |
| P8a-38 | print | A `NetworkPrinter` with no `serverPort` is on port 631 (`server_of`, `service/tools.rs:279-292`); Java's `(int) networkPrinter.serverPort ?: 631` casts before the default, so its `make#CupsPrinter` fails on such a record | 631 is the field's documented default ("Defaults to 631"); doc comment at `server_of` |
| P8a-39 | print | A send whose printer cannot be had (no `NetworkPrinter`, no printer of that name, the server not reached, no print client) is recorded on the job as `PtjSendFailed` with the message (`send_job`, `service/tools.rs:379-386`); Java's `make#CupsPrinter` runs before the Groovy's `try`, so its job stays `PtjNotSent` with no `errorMessage` | The job shows why it was not printed, which the PrintJob screens list; `PtjSendFailed` to `PtjPending` is a Send transition, so it can be sent again. Comment at the site, and the module doc |
| P8a-40 | print | A number the server does not give is null: a refused job without a `job-id` is stored and returned with no `jobId`, and details without `job-k-octets` or `job-media-sheets-completed` give no `size` or `pagesPrinted` (`service/tools.rs:345-350`, `:536`); cups4j's ints give -1 and 0 | No job on the server has id -1, and a job without one is reported as not sent by the details, hold, release and cancel; comments at both sites |
| P8a-29 | service REST | A plain map out-parameter loses its null entries one level down too (`remove_result_nulls`, `service/rest.rs:596-615`); Java removes them only from an `EntityValue` | Entity values are maps in this port, so the two cannot be told apart; a missing key and a `null` read the same to a JSON client. Doc comment at the function |

### Rules: EECA, SECA, security and rule conditions

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P7a-01 | EECA | Security-check EECAs run even when the message facade already has errors (Java skips them) | Stricter by design; `service/eca.rs:74-75`, test at `service/eca.rs:327` |
| P7a-04 | security | `SSH_SHELL` sealed in Rust conf/constants | Rust-only artifact (`service/security_checks.rs:31-32`, `MoquiDefaultConf.xml:63-64`) |
| P7a-07 | rule conditions | A rule condition is JavaScript truth, as an `<if>` in the actions: an empty list or map is true (`eval_condition`, `script/runtime.rs:574-579`); Java's `checkCondition` takes Groovy truth (`as Boolean`), where it is false | The action language is JavaScript: a rule condition and an `<if>` agree, converted Groovy tests a list's length (`groovy_convert`), and the linter flags collection truth (`lint_hazards`). Doc comment at `eval_condition` |
| P7a-08 | SECA | A pattern rule is matched against each called name, once per name (`SecaIndex`, `service/seca.rs:118-121`); Java expands it at load to the services known then, so it never runs for one it did not know, as an implicit entity-auto service | The same rules run for every service known at load; a pattern written for a name also covers that name when the service comes later. Comment at the site |
| P7a-15 | SECA | A pattern rule with `when="tx-commit"` or `"tx-rollback"` runs (`register_tx_rules`, `service/call.rs:607-612`); Java's `registerTx` compares the rule's service name, the pattern text, to the called name, so it never registers one | A pattern rule runs at each `when` it names, as at every other phase (P7a-08); Java's skip comes from comparing a pattern as a name. Doc comment at `register_tx_rules` |

### Pools, jobs and async calls

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P7b-04 | pools | Workers = `worker-pool-max` − job core; core/alive ignored (Java effectively runs `worker-pool-core`) | Comment at `service/pool.rs:801-803` |
| P7b-06 | jobs | Lock row with a run id but null `lastRunTime` holds the job | Comment at `service/jobs.rs:224-227`; Java always writes both |
| P7b-16 | jobs | `ServiceJobRun.runThread` is the name of this port's job thread, `moqui-job-0` (`service/jobs.rs:463-475`); Java's is `MoquiJob-1` | The column names the thread that ran the job in both; the threads follow this port's `moqui-*` names, which its logs show. Comment at the site |
| P7b-18 | async | An async call whose parameters fail validation is not queued: the errors are in the caller's message facade, and a future gets an empty map (`prepare_async`, `service/call.rs:264-313`); Java queues it, skipping its authorization check, and its run fails the same validation | Safer: a call that skipped the authorization check is not queued, and its run would do nothing but its `pre-validate` rules. Doc comment at `prepare_async` |

### Mail and email ECA

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P7a-09 | email ECA | A failing email rule is logged and the rest run; Java aborts | AGENTS.md failure-tolerance policy |
| P8b-01 | mail | `allowedToDomains` dot-boundary match on the extracted address vs Java `endsWith` on the raw address | Hardening; doc comment `service/mail/implementation.rs:545-564` |
| P8b-02 | mail poll | Per-message `ForceNew` transaction and continue-on-error vs Java one poll transaction aborting on first throw | AGENTS.md policy; comment `service/mail/implementation.rs:1303-1307` |
| P8b-03 | email ECA | `save#EcaEmailMessage` stores the to, cc and bcc lists joined by `, ` (runtime `EmailServices.xml:79-86`); Java stores Groovy's `List.toString()`, `[a, b]` | The stored lists are read back by `send#EmailMessage`, which splits them on commas in both, so Java's form sends to `[a` and `b]`; `send#EmailTemplate` stores them comma separated too. Comment at the site |
| P8b-04 | mail poll | An IMAP poll expunges only when it flagged a message deleted (`ImapBox::close`, `service/mail/implementation.rs:1668-1671`); Java's `folder.close(true)` always expunges | Safer: a poll of a server not set to delete does not purge what another client flagged and could still undelete. Comment at the site |
| P8b-05 | email ECA | `message` context is a Map, not the live `MimeMessage` | Comment `service/mail/implementation.rs:1402` |
| P8b-06 | mail | A screen attachment with no `fileName` is named for its location (or `screenPath`) less the extension (`service/mail/implementation.rs:783-795`); Java drops the last 4 characters of the location | The same for a `.xml` screen; Java fails the send with an NPE for a `screenPath` attachment, which has no location. Comment at the site |
| P8b-07 | mail | A recipient list is split on commas outside quotes and `<>`, and empty items are left out (`split_addresses`, `service/mail/implementation.rs:2080-2082`); Java splits on every comma | Java breaks `"Doe, Jane" <jane@example.com>` into two bad addresses. Doc comment at the function |
| P8b-08 | mail | `send#Email`/`send#EmailTemplate` gain `body`/`bodyText` overrides | Module doc `service/mail/implementation.rs:23`, `:83-84` |
| P8b-09 | mail | The text alternative is rendered only when the body screen's `render-modes` takes `text` (`render_bodies`, `service/mail/implementation.rs:373-392`); Java always renders it | This port's screen renderer refuses a mode the screen does not list (`screen/render.rs:410`), which would fail the send; no email body screen in `runtime/` sets `render-modes`. Doc comment at the function |

### SSO

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P8c-05 | SSO | The callback takes no return URL from `RelayState` (`service/sso.rs:235-246`); Java prefers it to the session's `returnTo` | `RelayState` is SAML's, which the port has no client for (P8c-14), so a return URL never comes from the callback's query. Comment at the site |
| P8c-06 | SSO | `login#User` without a `returnTo` clears one stored by an earlier, unfinished login (`service/sso.rs:204-208`); Java keeps it | Stricter: a stale return URL is not followed by a later login. Comment at the site |
| P8c-08 | SSO | Default group added only when non-empty | `service/sso.rs:498-500` |
| P8c-14 | SSO | SAML flows error as unavailable; Java builds a `SAML2Client` | No SAML client in the port (`service/sso.rs:180-184`) |
| P8c-17 | SSO | `logout#User` always ends the local login, even when the provider's end-session URL cannot be had, which is then logged (`service/sso.rs:266-298`); Java skips `logoutUser()` when its pac4j logout throws | Safer: a provider failure does not leave the user logged in here. Doc comment at the function |

### elFinder, images and system messages

| ID | Area | Difference | Rationale / where documented |
|---|---|---|---|
| P8c-10 | elFinder | `rm` validates all targets before deleting; Java deletes then returns on first bad | Comment `service/elfinder.rs:657` |
| P8c-12 | images | Suffix/MIME lists hardcoded; Java uses live `ImageIO` lists | `service/images.rs:46-55` |
| P8c-13 | system message | Replay-cache TTL check on received messages (Rust-only) | Hardening; `service/system_sync/message.rs:430-446` |
| P8c-15 | elFinder | Symlink-to-directory deleted as a link; Java follows and deletes contents | Comment `service/elfinder.rs:426-431` |

## Code quality review (2026-10-15)

Behaviour changes made when the [Code-Quality-Review](archive/Code-Quality-Review.md#behaviour-changes) findings were fixed, where the result differs from Java on purpose. The review's "Java-parity design points" (REST `pageNoLimit`, item-by-item JSON-list commits, the implicit `actions` transition's whole context, attachments by any locatable file) were kept as Java has them and are not differences.

| Area | Difference | Rationale / where documented |
|---|---|---|
| transactions | A request or service worker rolls back a frame left open by a unit of work, with an error log; before, a request worker committed it. A joined `rollback()` marks the frame rollback-only | Nothing half-finished is committed by the thread that merely found the frame open; review phase 2 (`TxScope` and frame checks) |
| errors | The HTTP status comes only from `ErrorKind`, so some general errors that gave 404, 405 or 403 give 500; in production a 500 body and a failed section show an error id, not the detail | Typed errors instead of text matching; detail stays in the log under the id; review phase 5 |
| sessions | A client that sends no cookie gets no Visitor or Visit row until the cookie comes back or it logs in; live sessions are capped (`webapp_session_max`) | A crawler or probe cannot fill the session table; review phase 4 |
| authorization | A DENY whose `filterMap` or pattern cannot be evaluated denies; non-expression ECA conditions are evaluated and an unknown condition child is an error | Fail closed; review phase 5. `ArtifactAuthz` data with a Groovy-form `filterMap` (`[a: b]`) should be checked |
| scripts | A script file runs as a function body, so `ec.resource.script` returns undefined when its last statement is not an expression; assigning to a host global throws; a script outside a transaction on a request thread stops at `webapp_request_timeout` | Engine and deadline soundness; review phase 3 and 4 |
| SSO | The local account is linked by `externalUserId` and a mismatched account is refused; email is the username only when verified or when it is the provider's username field | Account take-over hardening; review phase 2. See also P8c rows above |
| search | An OpenSearch partial shard failure is an error; highlight fragments are HTML-escaped | A partial answer is not reported as whole; review phase 5 |
| entity REST | Request `pageSize` is capped at `entity_rest_max_page_size` (1000); `pageNoLimit` still lifts the cap, as in Java | Same as P5-14 above |
| SFTP | The default port is 22 | The protocol's port; review phase 6 |
| data writer | Zip entries are deflated | Smaller exports; review phase 6 |

## Gallery fixes (2026-10-18)

Defects the screen archetype gallery (`runtime/base-component/example/screen/Gallery/`) found in the qvt render mode and the script runtime; these fixes differ from Java on purpose. The last three rows are gallery findings F4 and F28, fixed after the gallery commit.

| Area | Difference | Rationale / where documented |
|---|---|---|
| csv render | A `label` writes nothing in the csv render mode; Java's `DefaultScreenMacros.csv.ftl` wrote the label text with no line break, so a screen's captions ran into the header row | An export is the table; the difference is named in the top comment of `runtime/base-component/framework/template/screen/ScreenCsv.js` |
| entity-find | `limit` and `offset` that expand to an empty or non-numeric text apply no limit or offset and log at debug; Java's `as Integer` threw | Failure tolerance (AGENTS.md); `find_count` in `script/runtime/entities.rs` |
| menus | F4: a subscreen whose screen does not load or parse, or whose menu title or `disable-when` fails, is logged with its location and left out of `sri.getMenuData` (the qvt shell's `menuData`) and `sri.getMenuItems` (the html menu, the app list); its siblings and the parent still show. Java's `getMenuData` throws, so the whole menu answers 500 | Failure tolerance (AGENTS.md); a security error leaves the item out quietly. Doc comment at `leave_out` in `screen/menu.rs` |
| qvt render | F28: when a section, form, subscreen or included screen fails in the qvt mode, what it wrote is dropped before the error text is written in its place, and a `{` in that text is `&#123;`; Java and the html mode keep the partial output | A qvt response is a Vue template, which does not compile with the elements the failed part left open, and the text is not an interpolation. Doc comment at `guarded` in `screen/sri/sections.rs` |
| qvt render | F28: the screen a `.qvt` request renders alone (always standalone, below the root) writes its error in its place as it does under its parents, and the response is 200; Java's render start throws, so the request is a 500 | The client shows the screen where its parents would; the html mode already shows a subscreen's error that way. `render_active_screen` in `screen/sri/sections.rs` |

## Print fixes (2026-10-18)

The print findings of [Markdown-PDF-Review](archive/Markdown-PDF-Review.md#repository-findings) and the decision on P8a-41, where the fix differs from Java on purpose.

| Area | Difference | Rationale / where documented |
|---|---|---|
| PDF images | An `<img>` source that is a resource location is read through the resource facade and inlined, as Java's FOP resolver read it; an `http` or `https` source is not fetched, where Java's default resolver fetched it, and a source that is not a png, jpeg, gif, webp, bmp or svg image is not inlined | The PDF page makes no request (no network, no server, no session); module doc of `screen/pdf.rs` |
| PDF footers | The XSL-FO running footer is an `@page` margin box; a document printed several to a render (payment detail, shipment pack) numbers its pages through the render, where XSL-FO restarted each payment at page 1 | Chromium ignores `counter-reset: page`; comments in `PaymentDetail.print.html.fancy` and `ShipmentPack-shipmentPackPageSequence.html.fancy` |
| `/fop` | `/fop/<path>` makes only PDF: a `contentType` of `application/postscript` gives a PDF, as P8a-17 records for `print#ScreenDocument` | The PDF engine makes only PDF; `fop_screen_path` in `screen/mod.rs` |
| barcodes | `ec.resource.code128Svg` draws Code 128 as barcode4j did (the same code sets, symbols and check symbol), but: the quiet zone is 10 modules of the given module width, where barcode4j's is 2.1mm whatever the width; `height` is the whole height with the text, so the barcode stays inside it; and text Code 128 cannot hold (not ASCII) is logged and gives no barcode, where barcode4j failed the document | A barcode fits the box the template gave it, and one bad value does not stop a pick list printing; doc comments of `moqui-lib/src/barcode.rs` and `resource_code128_svg` in `script/runtime/ec_extra.rs` |
| print jobs | P8a-41: `orientation-requested` and `print-color-mode` are sent only when the job's `orientation` or `colorMode` names them; Java's cups4j asked for portrait and a monochrome `output-mode` on every job | A color logo or a landscape page is not changed by a default nobody chose; module doc of `moqui-host/src/ipp.rs` |

## Remaining-Work fixes (2026-10-18)

Where the fixes of the open entity and service findings (P1-14, P1-15, P3-20, P3-21, P4a-18, P5-19) and the leftovers worked from [Remaining-Work](Remaining-Work.md) differ from Java on purpose.

| Area | Difference | Rationale / where documented |
|---|---|---|
| entity condition | P3-20: a `LIKE` on a `text-fts` field is Java's `FtsSql` predicate in parentheses; Java appends its `AND LOWER(col) LIKE ?` words bare. A view alias of a `text-fts` field stays SQL `LIKE`, where Java rewrites it through the alias's column; and with no `_tsv` column (its DDL failed) the find fails, where Java's `FtsSql.disable` falls back to `LIKE` | Parentheses keep an `OR` around it from binding into it, as P1-07; no view in `runtime/` has a `text-fts` alias; a failed full-text DDL is logged by the table check, and a search that quietly turns into a table scan would hide it. Doc comment at `fts_like` in `moqui-db/src/sql/condition.rs` |
| login | No more of one user's password checks run at once on a factory than the failures the account has left (`max-failures` + 1 − `successiveFailedLogins`, at least one); another waits for one to end and reads the account again (`LoginCheck`, `user/login.rs`). Java checks every attempt that passed the disabled gate, and counts a failure only after its check | Concurrent guesses cannot all pass the gate before the first failure is counted, and concurrent logins with the right password still all succeed; one at a time, the logins checked are Java's. Doc comment at `LoginsChecking` |
| `matches` | The regular expression of `matches` is a Thompson NFA matched by a set of states, in time proportional to the text times the pattern (`moqui-lib/src/regex_match.rs`); a pattern whose counted quantifiers would write out more than 100,000 instructions is an error. Java's backtracking `java.util.regex` takes such a pattern and can take exponential time on nested quantifiers | A pattern a user or data file supplies cannot stall a request; the supported syntax is unchanged. Module doc |
| user authc | `validate#ExternalUserAuthcCode` does not update the `UserAccountEmail` pseudo factor that a code sent to the account email names as its `fromFactorId` (runtime `UserServices.xml`); Java's entity-auto update of that missing row fails the verify | There is no row to mark validated; with P1-14 an update of a missing row is an error, as in Java, so the code could never be used. Comment at the site |

## Revision management (2026-10-18)

Text diffs of resource versions and the `git://` scheme ([Revision-Management](../docs/Revision-Management.md)). Java had the first as TODOs in `DbResourceReference` and has no git scheme.

| Area | Difference | Rationale / where documented |
|---|---|---|
| dbresource history | A replaced text version's `DbResourceFileHistory` row holds a reverse unified diff against the next version (`isDiff="Y"`), with the whole bytes kept for binary MIME types, text that is not UTF-8, every 20th version, and a diff over half the replaced size. Java writes every past version whole (`putText` has "TODO: use diff from last version for text") | Most wiki and content edits change a few lines; the file row keeps the newest text whole, so the current read is unchanged and the chain to rebuild a version is at most 19 deltas. Doc comment at `past_payload` in `moqui-core/src/entity/dbresource.rs` |
| dbresource history | `openStream(versionName)` and `getText(versionName)` of an `isDiff="Y"` version rebuild it from the nearest whole text after it; a delta that does not apply exactly is logged and reads as no version. Java returns null for any `isDiff="Y"` row ("TODO ... reconstruct from root merging in diffs"), so a Java server sharing the database sees these versions as missing | The port writes such rows; a broken chain affects only the versions behind it, per the failure tolerance of AGENTS.md. Doc comment at `read_db_resource_version` |
| resource versions | `ResourceReference.getVersionDiff(lhs, rhs)` (a line diff with an escaped HTML table) and `Version.getText()`, `getPreviousVersion()`, `getNextVersions()`, `getMap()` on script version objects; the version methods dispatch through the resource facade to `dbresource` or a provider that keeps versions (`ResourceProvider::supports_version` and the rest, defaulted to none) | Java has no diff API; its compare screens ran jsdiff from a CDN in the browser. Doc comments in `context/resource.rs` and `native.rs` |
| `#version` | `ec.resource.getLocationText("loc#v")` and a template or `render-mode text` location with `#v` read version `v` of a `dbresource` or provider file, as Java's `getLocationText`; before this the port read the current text. A `#` suffix on a `dbresource` or provider location no longer becomes part of the file name | Java parity; `split_version` in `context/resource.rs`, `prepared` in `script/template.rs` |
| git scheme | `git://<repositoryId>/<path>` over a `moqui.resource.GitRepository` (`remote` mirror, `local` bare repository, or `db` objects in `GitObject`/`GitRef`), with `org.moqui.impl.GitServices` and the `fetch_AllGitRepositories_frequent` job | New; module doc of `moqui-core/src/resource/git/mod.rs` |
| elFinder | A `git://` root is a volume, through the resource facade; a new directory there holds `.gitkeep`, a directory is reported removed when its last file goes, and `git://` roots take writes in production as `dbresource://` does | Git keeps no empty directories; doc comments in `service/elfinder.rs` |

## Time zones (2026-10-19)

Timestamp text without an offset is read in the session time zone wherever text becomes a timestamp, as Java's `ec.l10n.parseTimestamp` reads it: a service parameter, a field set from text, a data file, a data check, a search form and a view's `econdition` value. The session zone is the user's `UserAccount.timeZone`, else the server's, which `default_time_zone` names at start (MoquiDefaultConf's `UTC`) as Java's `TimeZone.setDefault` does, so a load with no user reads its data in that zone. Text with an offset or `Z`, epoch millis, and dates and times are the same in any zone; only text a database driver gives back is read in UTC (module doc of `moqui-core/src/l10n.rs`). Where the port converts text that Java hands on unconverted:

| Area | Difference | Rationale / where documented |
|---|---|---|
| entity value | A field set from text with `set` or `setFields` is converted when the value is written, a timestamp in the session time zone as `setString` and `setAll` read one; Java's `set` keeps the `String` and its JDBC driver hands it to the database, which reads it in the database's zone (Postgres refuses the `varchar` for a `timestamp` column) | One rule for every text that becomes a timestamp; comment at `normalize_values` in `entity/value.rs` |
| entity condition | A text value compared to a date, time or number field is converted to the field's type before it is bound, timestamp text in the session time zone (`Column::typed`, `moqui-db/src/sql/condition.rs`); Java binds the `String` and the database reads it | The same text is the same moment in a condition and in the data it finds; doc comment at `typed` |
| ServiceJob cron | A job's cron is read in the machine's time zone (`TimeZone::system()`), the zone the operating system reports. Java's `ScheduledJobRunner` uses `ZonedDateTime.now()` after `TimeZone.setDefault(default_time_zone)`, so it follows the server zone (UTC in MoquiDefaultConf). The runner still skips a job only when the previous occurrence is strictly before the last run (equal times still run), as Java's `!lastSchedule.isBefore(lastRunDt)`. `is_last_run_before_last_schedule` stays strict: equal times are not due | The machine is where the job runs; `default_time_zone` stays the zone of timestamp text with no offset. `job_time_zone` in `service/jobs.rs`, used by the pass, `is_last_run_before_last_schedule` and `next_cron_execution`. Test `a_six_am_job_is_due_in_the_zone_where_it_is_six` |
| notification digest hour | The digest seed cron is hourly (`0 0 * * * ?`). Each user is sent in the hour of `UserPreference` `NOTIFICATION_DIGEST_HOUR` (0–23, missing means 6) in `UserAccount.timeZone`, or the machine zone when that is empty. A database loaded with the old `0 0 6 * * ?` row keeps that cron until the row is updated; startup does not rewrite it. The email groups by app, then topic. Java sent one flat list at 06:00 server time | `send#UserNotificationDigest`, `NotificationDigestEmail.xml` |

## Rich text (2026-10-19)

IronStone impl-drawers-commons M3: what the safe HTML cleaning keeps for the qvt rich-text editor. The `chatter` and
`record-files` screen elements and the editor are new (js/README.md, "Record panels, rich text, avatars and record cards").

| Area | Difference | Rationale / where documented |
|---|---|---|
| `allow-html="safe"` | The cleaning keeps `data-mention-id`, `data-record-entity` and `data-record-id` on a `span`, beside ammonia's default allow-list (Java's Jsoup safelist drops every `data-` attribute). Tables are in that default list | The mention `<span data-mention-id="id">@Label</span>` and the record link `<span data-record-entity="entity" data-record-id="id">#Label</span>` of the qvt rich-text editor, so a cleaned note still says whom it mentions and which record it points at; the values are escaped text. A record span is not a mention. Doc comment at `safe_html` in `service/validate.rs`, test `the_rich_text_editor_output_survives_the_safe_cleaning` |

## Layout panels and the calendar and contact tools (2026-10-19)

IronStone impl-drawers-commons M4 and M7 (framework). `container-split` and the `VCard` and `ICalendar` tools are new
(js/README.md, "Layout panels: split, wizard and stack"; module doc of `moqui-core/src/pim/mod.rs`).

| Area | Difference | Rationale / where documented |
|---|---|---|
| `subscreens-panel` | `type="wizard"` (a stepper whose Next submits the step's form and follows its answer) and `type="stack"` (a breadcrumb back to the first subscreen) render; Java's XSD had them TABLED and its macros wrote a "LATER ... not yet supported" heading | The wizard archetype of the IronStone style guide (§3.10); XSD documentation of `subscreens-panel`, `m-subscreens-wizard` in `WebrootVue.qvt.js` |
| screen menu data | A screen with a wizard or stack panel reports `hasTabMenu` true, as a tab panel does, so the shell's breadcrumb does not add a menu of its steps | The stepper and the breadcrumb are the panel's own menu; `screen/definition.rs` |

## Framework gaps from the IronStone screens (2026-10-19)

Fixes of the gaps the BOSS screens found; where a fix differs from Java on purpose, or adds what Java has not.

| Area | Difference | Rationale / where documented |
|---|---|---|
| `ec.l10n.format` | Epoch millis text (12 or 13 digits, 2001 to 2099) with no pattern is formatted as a time stamp (`yyyy-MM-dd HH:mm`); Java returns a `String` as it is | A time stamp reaches a script and a server-static screen's data as its millis text (`temporal_text`), where Java's script holds the `Timestamp`, so `${ec.l10n.format(statusHistory.changedDate, null)}` showed the millis. Doc comment at `epoch_millis_text` in `l10n.rs`; the client's `$mq.fmt` does the same |
| date patterns | The week fields `Y`, `w`, `W` and `F` follow the `en_US` calendar (weeks from Sunday, week 1 holds January 1) whatever the locale; `u` is the year (`DateTimeFormatter`), not `SimpleDateFormat`'s day of the week | The formatter has no locale calendar data; module doc of `moqui-lib/src/datefmt.rs` |
| cache facade | `ec.cache.getCache(name)` with no conf entry makes the cache with the default settings on first use, as Java; `clear()` of a name also clears the screen or entity list cache of that name | Java parity; doc comment at `CacheFacade::get_or_create` |
| in-memory test db | Numbers of two types (an `I32` condition value, an `I64` column, a decimal) compare by value | As the database compares them; `numeric` in `testing.rs` |
| spec rewrite | In a `then` or `expect` block a line that starts with `/`, `` ` ``, `(` or `[` after a line that ended an expression is its own condition (a `;` is put before it), as each line is in Spock; JavaScript would join it to the line before | Module doc of `testkit/lines.rs`; docs/Testing.md, Pitfalls |
| notifications | `mark#NotificationsViewed` of all of an inbox leaves the approvals waiting, and an approve or refuse marks every recipient's row viewed | No Java counterpart (no approvals in Java); the service file's top comment |
| `link` | `background-submit`, `background-reload-id`, `background-hide-id` and `background-message` on a link that posts through a hidden form, as on a `form-single`; Java's `link` has none | xml-form-3.xsd; js/README.md |
| `container-dialog` | `tooltip`; the qvt `m-container-dialog` registers as a container of the root by its id, so `background-hide-id` closes it; Java's did not register | xml-screen-3.xsd; js/README.md |
| `messageInfos` | A message info may carry `link` and `linkText`, a button of its notification | js/README.md, "Shell data and providers" |
| preferences | A response to a request that set user preferences has `X-Moqui-Preferences` (the JSON of what it set), which the qvt client applies (`moqui.applyPreference`) | js/README.md, "Preferences"; `preferences_header` in `web/mod.rs` |
| Visit | `sessionId` is the web session id at the visit's start (it changes at a password login's rotation; the `HttpSession` row keeps the visit), and `clientHostName` is the client address (no name lookup) | Java's `request.getRemoteHost()` without lookups gives the address; `create_visit` in `user/session.rs` |
| sessions | `org.moqui.impl.UserServices.end#UserVisit` and `ec.user.endVisit(visitId)` end another session of the user; `ec.user.visitId` and `ec.user.visit` as Java's `getVisitId` and `getVisit` | No Java counterpart for ending another session; doc comment at `end_user_visit` |
| mail | With `mail_capture_sends=true`, or the `MailCapture` tool enabled by a spec, `send#EmailMessage` keeps the message in memory (`ec.factory.getTool('MailCapture')`) instead of connecting | docs/Testing.md, "Mail"; module doc of `service/mail_capture.rs` |
| shell | `sri.getShellProviders(kind)`, the providers of `getShellData` alone, kept per session | js/README.md, "Shell data and providers"; doc comment at `shell_providers` |

## Modified fields (2026-10-19)

An `EntityValue` read from the database (a find, the entity cache, a refresh), or written since, keeps what each field it sets held before, as Java's `dbValueMap`, and its update writes only the fields that changed (doc comment of `EntityValue` and `write_update` in `entity/value.rs`).

| Area | Difference | Rationale / where documented |
|---|---|---|
| entity value update | A found value's `update` (and `createOrUpdate`, `updateAll`, a script's `update()`) writes only the fields set to something other than what was read, plus `lastUpdatedStamp`, and nothing (no statement, no audit row, no after rule) when none changed. Java's find leaves `isFromDb` false, so Java's update of a found value writes every field it holds, or, when the entity reads the row first (audit log, create-only, field defaults, optimistic lock, data feed), every held field that differs from the row, a stale one too | A find, a set and an update no longer write back a field another transaction changed in between (a lost update); a value from `makeValue` still writes every field it holds, as data load and the entity-auto services use. The stamp a caller sets on a found value is kept without a row read (P1-15) |
| entity value | `clone` keeps what the value read and what it changed; Java's `cloneValue` copies `dbValueMap` and starts not modified | A copy of a found value updates as the value would |
| ECA rules, cache, data feed | When an update of a found value reads the stored row (the entity reads it first, or an after rule has `get-original-value`), a field the value did not change is given to the rules, the cache clear and the data feed as stored; Java's `entityValue` holds the value as read, which Java also writes | The rules see the row as the update leaves it, so a field another transaction changed does not look changed by this one; `take_unmodified` in `entity/value.rs` |
| transaction cache, sim | A held update holds, and its flush writes, only the fields the update writes | As the direct write |
