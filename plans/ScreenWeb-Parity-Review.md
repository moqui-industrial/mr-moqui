# Screen and Web Parity Review

Status: the nine phases were worked on 2026-10-19 and 2026-10-20. 96 findings are open and none is
High: the three High findings (P5-01, P7-01, P8-01) and P8-02 were fixed on 2026-10-20. The shape
follows [EntityService-Parity-Review](archive/EntityService-Parity-Review.md).

Scope: the screen and web parts of the framework — `crates/moqui-core/src/screen/`,
`crates/moqui-core/src/web/`, the screen macro modules under
`runtime/base-component/framework/template/screen/`, and the HTTP host in `crates/moqui-host/src/`
— compared against the Moqui Java / Groovy / FreeMarker / XML sources they were ported from.

This is the screen-and-web half of the gap review named in
[Remaining-Work, Parity gap review](Remaining-Work.md#parity-gap-review); the entity and service
half is [EntityService-Parity-Review](archive/EntityService-Parity-Review.md).

## How to read a row

The review asks a different question from the entity and service review. That one asked "does this
line mean the same thing?". This one asks **"is the feature there?"** — the port replaces FreeMarker
macros with JavaScript modules, Groovy with Rust, and the servlet API with axum, so the code cannot
look alike and byte-identical output is not the goal. A finding is:

- a behavior Java has and the port does not (a missing widget, a missing attribute, a missing branch);
- a behavior both have that decides differently (a changed default, a different fallback, a
  different order of operations);
- a message, a status code, a URL shape, or an authorization rule that differs.

A difference that is deliberate and already recorded in [Parity-Diffs](Parity-Diffs.md) is not a
finding: it is listed under "Checked and found equivalent" with a pointer to its row there. A
difference that looks deliberate but is not recorded is a finding with `Intentional?` = **Unclear**.

Severity: **High** = wrong result, data loss, authorization gap, crash, or a security-relevant
divergence; **Med** = missing branch, changed default, edge-case divergence; **Low** = cosmetic,
message text, or provably equivalent ordering.

Paths: Rust locations are relative to `crates/` unless they start with `runtime/`; Java locations
are file names under `../moqui/framework/src/main/{groovy,java}/org/moqui/impl/` unless they start
with `framework/` or `runtime/`. Line numbers are as of 2026-10-19.

## Summary

All nine phases were worked on 2026-10-19. 100 findings were found; the three High ones and P8-02
were fixed on 2026-10-20, so 96 are open and none is High.

| Phase | Open | High | Unclear |
|---|---|---|---|
| 1 Screen definition / form model | 12 | — | 4 |
| 2 Screen render / SRI surface | 13 | — | 1 |
| 3 Macro libraries: html, csv, text, xml, plain | 6 | — | 2 |
| 4 qvt render mode | 9 | — | — |
| 5 Server-static qvt | 8 | — | 1 |
| 6 Print and spreadsheet | 6 | — | 5 |
| 7 Screen URLs / tree / menu / static files / facade | 10 | — | 8 |
| 8 Web facade | 17 | — | 3 |
| 9 HTTP host and web app endpoints | 15 | — | 3 |
| **Total** | **96** | **0** | **27** |

Severity of the open findings: 48 Med and 48 Low; the three that were High, and the Med that was
fixed with them, are gone. Intentional: 69 plain divergences (`No`) and 27 that look deliberate but
are not recorded in
[Parity-Diffs](Parity-Diffs.md) (`Unclear`); the 27 are the ones that need a decision from the port
owner before they are fixed.

Two findings were removed on verification: **P2-01** was a duplicate of **P1-04** (the same code
path, found from two phases), and **P4-01** was invalid (see
[Verification pass](#verification-pass)). The numbers keep their original values, so the gaps in the
numbering are expected.

### The findings that were High, and are fixed

| ID | Area | What it was | Fixed |
|---|---|---|---|
| P5-01 | Server-static qvt | The static template's number rule called a free identifier `moqui`; the server compiles the template with Vue's `prefixIdentifiers`, which rewrote it to `_ctx.moqui`, which is undefined — the rule threw on every non-empty value and the form never submitted | 2026-10-20: the rule calls `$mq`, the name the static scope has and the Vue global property `app.config.globalProperties.$mq`, so one rule text works in both qvt modes |
| P7-01 | Screen schema | The REST schema documents (`entity.json`, `master.json`, `service.swagger`) were served to any logged-in user; Java refuses with 403 without the `REST_SCHEMA` permission | 2026-10-20: the permission is asked for, as Java does |
| P8-01 | Web facade | An absolute redirect target was sent as the `Location`; Java reduces an `http(s)` target to its path when the webapp has no `http-host` (the default), so Java has no open redirect there and the port did | 2026-10-20: Java's rule is in `http_redirect_location`, and a target that is not a path of this site goes to the root |

## Verification pass

Every phase was written by a subagent and then checked by hand. The High findings, the duplicates
and a sample of the Med findings were re-derived from the Java source, not taken on trust:

| Finding | Checked against | Result |
|---|---|---|
| P1-04 / P2-01 | `ScreenDefinition.groovy:1000-1008` | **Confirmed.** Java runs the `service-call` only when there is no error but always runs the `<actions>`, with a comment saying the older rule "doesn't allow actions to handle errors". The port runs one merged list under Java's skip rule. |
| P4-01 | `DefaultScreenMacros.qvt.ftl:377`, `ScreenQvt.js:927`, `ScreenQvt.js:112` | **Invalid, removed.** The port's condition is the same as Java's (escape when `encode="true"`), and the port's `A()` helper escapes every attribute value — the `?xml` of the FreeMarker macro. The two render the same bytes in both the `encode="true"` and the default case. The agent had read the `label` macro's other rule (`encode != "false"`). |
| P5-01 | `ScreenQvtStatic.js:370`, `ScreenQvt.js:258`, `vue_check.rs:136` | **Confirmed.** The static template writes `moqui.isStringNumber(value)` into a `:rules` array; the server compiles with `prefixIdentifiers: true`, which rewrites the free identifier to `_ctx.moqui`, and the static scope has no `moqui`. The dynamic mode asks the server for the rules (`sri.getFieldValidationJsRules`) and localizes the messages server-side, so it does not have the problem. |
| P7-01 | `RestSchemaUtil.groovy:570-582`, `screen/schema.rs:60-70` | **Confirmed.** Java's `requireSchemaAccess` checks `hasPermission("REST_SCHEMA")` and answers 403; the port's `logged_in` checks only that a user is logged in. The permission and its ADMIN grant are in the port's seed data, so the port has a permission that no code reads. |
| P8-01 | `ScreenRenderImpl.groovy:228-251`, `web/mod.rs:844`, `render.rs:1371-1384` | **Confirmed.** Java's `sendHttpRedirect` rewrites an absolute target to its path when no `http-host` is configured; the port has no such rule. (The port's `screen-last` path is separately safe: it accepts only a path that starts with `/` and not `//`, which is stricter than Java's `isSameOriginRedirect`.) |
| P8-02 | `WebUtilities.java:456-466`, `moqui-host/src/http.rs:897-912` | **Confirmed, severity Med.** Java drops CR/LF/NUL from a redirect location. The port does not, and axum rejects the header value, so the answer is a 500 ("could not build the response") where Java redirects. |
| P2-08 | `ScreenDefinition.groovy:212`, `ScreenRenderImpl.groovy:2460` | **Confirmed, but already recorded.** Java only reports the screen's `render-modes` to the client; the port refuses a mode the screen does not name. That difference is already in [Parity-Diffs](Parity-Diffs.md#mail-and-email-eca) (the P8b-09 row names `screen/render.rs:410`), so P2-08 is a pointer, not a new finding. |

The remaining findings are cited with a file and a line on both sides and were not re-checked
one by one. A reader who wants the same assurance on a Med finding can follow the citation.

The three High findings and P8-02 were fixed on 2026-10-20, on the strength of this pass: the
permission check, Java's redirect rule and the `$mq` rule text. The fixes are in
[Parity-Diffs](../plans/Parity-Diffs.md) where they differ from Java on purpose.

---

## Phase 1 — Screen definition, form model and saved finds

Compared `crates/moqui-core/src/screen/definition.rs`, `crates/moqui-core/src/screen/form.rs` and
`crates/moqui-core/src/screen/saved_finds.rs` (with the parts of `screen/render.rs`,
`screen/sri/forms.rs`, `screen/sri/sections.rs` and `screen/generate.rs` they drive) against
`ScreenDefinition.groovy`, `ScreenSection.groovy`, `ScreenWidgets.groovy` and `ScreenForm.groovy`,
and the port's `runtime/base-component/framework/xsd/xml-screen-3.xsd` against the Java
`framework/xsd/xml-screen-3.xsd`.

Worked 2026-10-19: 12 findings — the auto-field drop-down of a related entity, the `fromDate`
default, the aggregate `show-total`, the transition actions under an error, the saved-find
transition contract, the dynamic form's saved-find key, and six smaller ones.

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P1-01 | `screen/form.rs:1088-1097`, `:1099-1109`, `:1118-1141` | `ScreenForm.groovy:770-792`, `:832-840`, `:854-857`, `:903-908`, `:944-986` | An auto field whose column is the one key of a `one` relationship is a `text-line`/`text-find` in the port; Java makes it a drop-down of the related entity's records | Med | No |
| P1-02 | `screen/form.rs:1078-1080` | `ScreenForm.groovy:819-821` | The auto `fromDate` field has no `default-value` (today); Java sets it | Med | No |
| P1-03 | `screen/form.rs:1127-1131` | `ScreenForm.groovy:893-901` | `show-total` is always `sum`; Java uses the `function` of the field (a view alias's `min`/`max`/`avg`) | Med | No |
| P1-04 | `screen/render.rs:1135-1138` (with `screen/definition.rs:327-365`) | `ScreenDefinition.groovy:1000-1008` | A transition's `actions` do not run when there is an error; Java runs the service-call only without an error, but always runs the actions | Med | No |
| P1-05 | `screen/definition.rs:281-296` | `ScreenDefinition.groovy:1109-1138` | The implicit `formSaveFind` and `formSelectColumns` take only a POST with a session token; Java takes any method and no token | Med | Unclear |
| P1-06 | `screen/saved_finds.rs:111-124` | `ScreenForm.groovy:278-297` | A dynamic form's saved finds are not keyed by the entity or service the form is for; Java adds the extension from `auto-fields-service`/`auto-fields-entity` | Med | No |
| P1-07 | `screen/saved_finds.rs:588-599` | `ScreenForm.groovy:2400` | The scheduled email subject escapes `${` in the find's description; Java expands it (template injection) | Low | Unclear |
| P1-08 | `screen/saved_finds.rs:548-563`, `:617-633` | `ScreenForm.groovy:2389-2410`, `:2446-2461` | `ScheduleFind` and `MakeDefault` refuse a find the user does not own; Java allows any find id | Low | Unclear |
| P1-09 | `screen/form.rs:132-197` | `ScreenForm.groovy:2033-2139` | The form-list `entity-find` selects the definition's columns; Java selects the columns the user saved | Low | Unclear |
| P1-10 | (absent) | `ScreenForm.groovy:93-96`, `:465-481` | `formDisplayOnly` (and `formDisplayOnly_<form>`) is not implemented: the form stays editable | Low | No |
| P1-11 | `screen/definition.rs:1466-1471`, `:938-944` | `xml-screen-3.xsd:660-699`, `:700-711` | The port reads `subscreens-item/@include-in-menu` and `@disable-when` and accepts `subscreens-menu/@type` of `wizard`/`stack`; the port's own XSD rejects all three | Low | No |
| P1-12 | `screen/form.rs:729-735` | `ScreenForm.groovy:1160-1182` | A new field with a `layoutSequenceNum` is appended to the `field-layout`; Java rebuilds the layout and puts the field at that sequence | Low | No |

#### P1-01 — An auto field of a related entity is a text box, not a drop-down

`add_auto_entity_field` in `screen/form.rs` finds the `one` relationship a field is the single key
of (`shape.one_relations`, read from the entity model at `screen/form.rs:936-946`), but it only uses
that relation for the `display` and `find-display` default fields, where it writes a
`display-entity` (`screen/form.rs:1053-1067`, `:1113-1116`, `:1138-1141`). The `edit` branch writes
`text-line` with a size (`screen/form.rs:1088-1097`), the `find` branch writes `text-find`
(`:1099-1109`) and the `find-display` header field writes `text-find` (`:1132-1136`).

Java does the same lookup (`ScreenForm.groovy:775-796`) and then, in the `edit`, `find` and
`find-display` branches, calls `addEntityFieldDropDown` (`ScreenForm.groovy:832-840`, `:854-857`,
`:903-908`). That builds a `drop-down` with an `entity-options`/`entity-find` of the related entity
(limit 200, the enumeration or status type filtered by the relationship's `title`), with the
related entity's description field in the text; when the related entity has no rows or more than
200, Java falls back to a `text-line` (`ScreenForm.groovy:944-986`).

A user sees the difference on every auto-generated form: the foreign key of an edit form is a free
text box to type an id into, instead of a picker of the related records; and a find field filters
the id column with free text, so a search for "Open" matches no row. The port would need the
relationship's related entity, its key field and its description field in `EntityShape` (the first
two are there) and the `drop-down`/`entity-options`/`entity-find` widget it builds.

#### P1-02 — The auto `fromDate` field has no default value

Java's `addAutoEntityField` writes a `date-time` widget for a date or time field and, when the
field is named `fromDate`, gives it the default value
`${ec.l10n.format(ec.user.nowTimestamp, 'yyyy-MM-dd HH:mm')}` (`ScreenForm.groovy:819-821`).

The port writes the `date-time` widget with its `type` and nothing else
(`screen/form.rs:1078-1080`), so the from-date of an auto-generated edit form is empty. A user has
to pick the current date by hand before saving, and a form that relies on "from today" searches
nothing until they do.

#### P1-03 — The aggregate total of a form-list is always the sum

For a number or currency field of an auto-generated form-list, Java sets the field's `show-total`
to the `function` attribute of the field node when it is `min`, `max` or `avg`, and to `sum`
otherwise (`ScreenForm.groovy:893-901`). For a view entity that node is the alias, so an alias
with `function="min"` (or `max`, `avg`) gets that total.

The port always writes `show-total="sum"` (`screen/form.rs:1127-1131`), and `EntityShape` does not
carry the function at all, though the entity model has it for an alias
(`crates/moqui-db/src/model/view.rs:121`). A form-list over a view with an aggregate alias (a
lowest or highest balance, an average) shows the sum of that column where Java shows the
aggregate. The fix is to pass the alias's `function` through `EntityShape` to
`add_auto_entity_field`.

#### P1-04 — A transition's actions are skipped when there is an error

Java runs the transition's `service-call` only when the message facade has no error, but it always
runs the transition's `actions`; the Java comment says so explicitly ("prior code also required
`!ec.messageFacade.hasError()` which doesn't allow actions to handle errors",
`ScreenDefinition.groovy:1000-1008`).

The port merges the `service-call` and the `actions` of a transition into one actions blob
(`screen/definition.rs:327-365`, the service-call first) and runs it only when there is no error
(`screen/render.rs:1135-1138`). A transition whose service reports a validation error and whose
actions handle that error (set a message, choose another response, clean up) therefore does
nothing in the port: the error response is used and the actions never run. The same happens when
an earlier screen of the path left an error in the message facade — Java would still run the
transition's actions.

The two `hasError()` rules do not line up either: Java's `MessageFacadeImpl.hasError()` (`:124`)
counts the validation errors as well as the error list, and the port's `has_error()`
(`service/util.rs:30-32`) counts only the error list. So a transition whose service wrote a
validation error is skipped in the port and runs in Java, and the port's rule is the wider of the two
only for the error list. The fix is to keep the `service-call` and the `<actions>` apart in
`TransitionDef` (as Java's `TransitionItem` does), to run the actions unconditionally, and to skip
only the `service-call` when there are errors.

#### P1-05 — The implicit saved-find transitions need a POST and a session token

Java's `FormSelectColumnsTransitionItem` and `FormSavedFindsTransitionItem` are `method = "any"`
with `requireSessionToken = false` (`ScreenDefinition.groovy:1109-1138`); the two transitions write
rows, so they are the ones Java's CSRF hole was in.

The port makes them `method = "post"` with `require_session_token = Some(true)`
(`screen/definition.rs:281-296`), and the code comment there records the difference ("Java takes
any method and no token"). The port's own macros post with the token
(`runtime/base-component/framework/template/screen/ScreenHtml.js:1010`, `:1021`, `:1061`), so the
normal flow works, but a screen that reaches `formSaveFind` with a link (a GET) is refused in the
port and answered in Java. The archived Code-Quality-Review (H14) records the intent, but
Parity-Diffs does not carry the row, so it is reported here as an open question rather than an
accepted difference.

#### P1-06 — A dynamic form's saved finds are not keyed by the entity or service

Java's `getSavedFindFullLocation` (`ScreenForm.groovy:278-297`) builds the key of a dynamic form's
saved finds as the form location plus `#` and an extension: the context's `formLocationExtension`
if there is one, else the service name of the form's `auto-fields-service`, else the entity name
of its `auto-fields-entity`.

The port's `saved_find_location` (`screen/saved_finds.rs:111-124`) uses only the context's
`formLocationExtension`, and the port's macros post the bare form name, so the extension is never
there (`runtime/base-component/framework/template/screen/ScreenHtml.js:1010`). Every dynamic form
of a screen therefore shares one list of saved finds and one column choice, and the rows Java
wrote for a dynamic form (keyed `…form_list$Name#EntityName`) are never found. The fix is to
read the form's `auto-fields-service`/`auto-fields-entity` name in `saved_find_location` and to
append it as Java does.

#### P1-07 — The scheduled email subject does not run the find's description as a template

Java builds the `ScreenScheduled.emailSubject` as the find's description followed by
`' ${ec.l10n.format(ec.user.nowTimestamp, null)}'` (`ScreenForm.groovy:2400`), and the subject is
expanded as a template when the scheduled screen is sent, so a description holding `${…}` is
executed as template source.

The port escapes `${` in the description before it goes into the subject
(`screen/saved_finds.rs:588-599`, `template_literal`), so the subject shows the description as it
is. This is the third part of the archived Code-Quality-Review's H14 fix, and it is not in
Parity-Diffs.

#### P1-08 — A find the user does not own cannot be scheduled or made the default

Java's `ScheduleFind` (`ScreenForm.groovy:2389-2410`) checks only that the find exists, and
`MakeDefault` (`ScreenForm.groovy:2446-2461`) does not check the ownership either (there is a
"FUTURE: consider some sort of check" comment). The port refuses both when the user is neither the
owner nor in a group that has the find (`screen/saved_finds.rs:548-563`, `:617-633`), which the
doc comment there records. A user cannot make a shared find of their group the default of a screen
in the port, where Java allows it. This is the second part of H14 and is not in Parity-Diffs.

#### P1-09 — The form-list `entity-find` selects the definition's columns

Java's `FormListRenderInfo.getListObject` (`ScreenForm.groovy:2033-2139`) selects the fields of
the columns the user has chosen (the `FormConfig` of the active saved find or of the user), plus
the hidden fields that have a `default-field`; the order-by fields that are not displayed are
dropped.

The port's `form_select_fields` (`screen/form.rs:132-197`) takes the columns of the form
definition, not the user's saved columns, and says so in its doc comment ("here the definition's
columns are a superset of it"). The rows carry a few more fields than the user chose, which no
screen shows; the visible columns are the user's, because the user's columns are applied when the
form is assembled for the render (`screen/sri/forms.rs:119-161`).

#### P1-10 — `formDisplayOnly` is not implemented

Java's `ScreenForm.isDisplayOnly` (`ScreenForm.groovy:93-96`) looks for `formDisplayOnly` and
`formDisplayOnly_<formName>` in the context; when one of them is `"true"` the form is rebuilt with
every non-display widget replaced by a `display` (`ScreenForm.groovy:465-481`, `fieldSubNodeToDisplay`
at `:509-532`), so the form is read-only.

Nothing in the port reads either name (no occurrence of `formDisplayOnly` in `crates/`), so a
screen that puts `formDisplayOnly` in the context still shows editable widgets and submits the
form. No screen in either tree uses it today (in Java only `ScreenForm` and `ScreenUrlInfo` name
it), so the gap shows only in an application that relies on it.

#### P1-11 — The port's code reads three things the port's own XSD rejects

The port's XSD is its own dialect and is a superset of Java's for the screen elements, but three
things the port's server code accepts are not in it:

- `subscreens-item/@include-in-menu` is read as an alias of `menu-include`
  (`screen/definition.rs:1465-1469`); the XSD declares only `menu-include`
  (`xml-screen-3.xsd:688`), and Java reads only `menu-include`.
- `subscreens-item/@disable-when` is read (`screen/definition.rs:1471`) and used when the menu is
  built, but the XSD has the attribute commented out (`xml-screen-3.xsd:693-698`). Java reads the
  attribute too, so the port matches Java here; the XSD hole is inherited.
- `subscreens-menu/@type` of `wizard` or `stack` is accepted by the code
  (`screen/definition.rs:938-944`, which treats a wizard or a stack panel as a tab menu, as
  Parity-Diffs records), but the XSD's `subscreens-menu` allows only `tab` and `popup`
  (`xml-screen-3.xsd:700-711`); the wizard and stack types are declared for `subscreens-panel`
  only (`xml-screen-3.xsd:715-751`).

Nothing of the other kind was found: every element the port's XSD adds to the screen vocabulary
(`shell-providers` and its kinds, `container-split`, `context`, `chatter`, `record-files`,
`html-view`, `markdown-view`, `status-bar`, `dashboard`, `number-card`) is implemented, in the
server or in the port's JavaScript macro modules.

#### P1-12 — A new field with a layout sequence is appended to the field layout

Java's `addFieldToFieldLayout` (`ScreenForm.groovy:1160-1182`) rebuilds the `field-layout` and
puts a new field at its `layoutSequenceNum`, or at the end when it has none. The port's
`add_field_to_layout` (`screen/form.rs:729-735`) always appends the `field-ref`, so the order of
the fields of a form with a `field-layout` can differ from Java's when a field has a
`layoutSequenceNum` (the `DbForm` fields carry one).

#### Phase 1 checked and found equivalent

- **The screen model** (`screen/definition.rs`): the element vocabulary the port recognizes —
  `parameter`, `transition`, `transition-include`, `subscreens` with `subscreens-item` and
  `conditional-default`, `section`, `section-iterate`, `section-include`, `form-single`,
  `form-list`, `tree`, `macro-template`, `web-settings`, `always-actions`, `pre-actions`,
  `widgets`/`fail-widgets` — matches Java's `ScreenDefinition` constructor
  (`ScreenDefinition.groovy:196-341`) and `ScreenSection`/`ScreenWidgets`
  (`ScreenSection.groovy:41-72`, `ScreenWidgets.groovy:28-40`); the port also registers the
  sections and forms inside a `fail-widgets`, which Java does not (a superset).
- **`extends` (the screen-extend merge)**: the attributes are copied over the base screen, and
  `parameter`, `transition` and `transition-include` are merged by name, `subscreens` by
  `subscreens-item` name, `actions-extend` before/after/replace and `widgets-extend` before/after
  by name or id (`screen/definition.rs:1061-1130`), as Java does
  (`ScreenDefinition.groovy:120-192`). The port finds the extend file by the path after the
  scheme and by the path after `/screen/`, as Java does
  (`ScreenDefinition.groovy:91-118`).
- **`transition-include`**: the transition is copied from the other screen and runs under the
  including screen (`screen/definition.rs:836-852`), as Java does
  (`ScreenDefinition.groovy:232-239`); a transition that cannot be found fails the screen in
  both.
- **`section-include`**: the sections, forms and trees inside the included section come along, a
  nested include is pulled the same way, and a name or location with `${` is resolved when the
  screen renders (`screen/definition.rs:1175-1259`, `screen/sri/sections.rs:132-153`), as Java's
  `pullSectionInclude` does (`ScreenDefinition.groovy:343-391`).
- **The sections of an extended form**: Java copies the sections of the form a form `extends` into
  the extending screen (`ScreenForm.groovy:119-129`); the port resolves them at render time from
  the screen the extended form is in (`screen/sri/sections.rs:106-125`, `:155-170`), so a form that
  extends a form of another screen renders its sections in both.
- **The transition lookup**: a method-specific transition wins over an `any` one, and a name that
  matches a transition name as a whole regular expression is matched for a request with a method
  (`screen/definition.rs:692-719`), as Java's `getTransitionItem` does
  (`ScreenDefinition.groovy:503-517`).
- **The authorization action of a transition**: the `authz-action` attribute, else the action of
  the transition's single service (the service's own, else by its verb), else `view` for a
  read-only transition and `update` otherwise (`screen/definition.rs:372-377` and
  `screen/render.rs:1147-1153`), as Java's `resolveAuthzAction` does
  (`ScreenDefinition.groovy:891-907`).
- **`read-only`, `begin-transaction` and `require-session-token`**: a transition with no actions
  and no service-call is read-only, `require-session-token` defaults to the webapp default
  (`true`), as Java's `TransitionItem` does (`ScreenDefinition.groovy:821-867`).
- **The responses**: `none`, `url`, `screen-last`, `screen-last-noparam`, the `parameter` children,
  the `parameter-map`, `save-current-screen` and `save-parameters`
  (`screen/definition.rs:176-234`, `:378-390`), as Java's `ResponseItem`
  (`ScreenDefinition.groovy:1195-1253`); the conditional responses are all checked and the last one
  that holds wins, and the error response wins over them (`screen/render.rs:1289-1308`), as Java
  does (`ScreenDefinition.groovy:1010-1021`).
- **The parameters**: a screen's and a transition's `parameter` elements are evaluated (`from` as an
  expression, `value` as an expansion, else the context, else the web parameter) and put into the
  context, and the path parameters are bound to the extra path names
  (`screen/render.rs:1310-1332`), as Java's `setAllParameters` does
  (`ScreenDefinition.groovy:911-941`).
- **The conditions**: the `condition` attribute is an expression and the `condition` element's
  children are an action condition (`screen/definition.rs:82-116`, `:526-534`), as Java's
  `ScreenSection`/`TransitionItem` read the attribute and the first child of the element
  (`ScreenSection.groovy:45-53`, `ScreenDefinition.groovy:848-852`).
- **The automatic transitions**: `actions`, `formSelectColumns`, `formSaveFind` and `screenDoc` are
  added when the screen has none of that name, and a screen's own transition of the name wins
  (`screen/definition.rs:854-864`), as Java does (`ScreenDefinition.groovy:241-245`).
- **The subscreens**: the directory of the screen holds its subscreens, the `subscreens-item`
  elements and the `moqui.screen.SubscreensItem` rows add and replace them in that order, and the
  conf `screen-facade` entries come before the rows (`screen/definition.rs:1284-1425`), as Java's
  `populateSubscreens` does (`ScreenDefinition.groovy:393-454`); the menu order is by index (no
  index first) then by the upper-case title (`screen/definition.rs:461-477`), as Java's
  `SubscreensItemComparator` does (`ScreenDefinition.groovy:1326-1343`); a user-group item is only
  for the members of the group (`screen/definition.rs:435-441`), as Java's `isValidInCurrentContext`
  does (`ScreenDefinition.groovy:1317-1323`).
- **A placeholder screen is server static**: a screen whose widgets are only the subscreens
  placeholders is server static in every render mode (`screen/definition.rs:1162-1172`), as Java
  does (`ScreenDefinition.groovy:329-337`).
- **`hasTabMenu`**: a tab (and, in the port, a wizard or a stack) panel or menu makes the screen
  report a tab menu; the wizard and stack are the port's own, and Parity-Diffs records them
  ("screen menu data", Layout panels section).
- **The form model** (`screen/form.rs`): `extends` of a form of the same screen, of another screen
  or of a `DbForm` (`screen/form.rs:599-621`), as Java's `initForm` does
  (`ScreenForm.groovy:99-136`); the merge of the `entity-find`, `row-actions`, `row-selection`,
  `hidden-parameters`, `field-layout`, `form-list-column` and `columns` of an overriding form
  (`screen/form.rs:638-700`), as Java's `mergeFormNodes` does (`ScreenForm.groovy:1084-1135`); the
  merge of a field of the same name and of the `conditional-field` by its condition
  (`screen/form.rs:702-727`), as Java's `mergeFieldNode` does (`ScreenForm.groovy:1137-1158`); the
  hidden fields a `field-layout` does not reference are added to it
  (`screen/form.rs:578-591`), as Java does (`ScreenForm.groovy:221-229`).
- **The auto fields**: `auto-fields-entity` with `include` of `all`, `pk` or `nonpk`, the
  `default-display` of a view alias deciding a form-list field, the `pq-expression` alias with
  `default-display="true"` as a display field, and `auto-fields-service` with the service's in and
  out parameters (`screen/form.rs:979-1030`, `:1189-1216`), as Java's `addEntityFields` and
  `addServiceFields` do (`ScreenForm.groovy:643-698`, `:618-641`); the `auto-columns` grouping of
  the displayed fields (id and date first, then numbers, short text and long text, about six to a
  column) (`screen/form.rs:1325-1399`), as Java's `addAutoEntityColumns` does
  (`ScreenForm.groovy:700-768`).
- **The auto widgets**: `auto-widget-entity` and `auto-widget-service` are expanded into the
  widget of the field, and a `widget-template-include` is replaced by the template's children
  (`screen/form.rs:737-846`), as Java's `expandFieldNode`/`expandFieldSubNode` do
  (`ScreenForm.groovy:988-1082`).
- **The `DbForm` forms**: the form is built from the `DbForm`, its `DbFormField`, the field
  attributes, the plain and the entity options with their conditions and orders, and the layout
  sequence (`screen/form.rs:320-447`), as Java's `getDbFormNode` does
  (`ScreenForm.groovy:358-452`); the forms of the user's groups are found through the
  `DbFormLookup` view (`screen/form.rs:255-293`), as Java does (`ScreenForm.groovy:73-80`,
  `:334-356`); a form with such an extension is assembled for each render
  (`screen/generate.rs:276-284`).
- **The validation of a field**: a field that is one of the in-parameters of the service the form's
  transition calls gets `validate-service`, or `validate-entity` for an implicit entity-auto
  service (`screen/form.rs:1152-1186`), as Java's `setSubFieldValidateAttrs` does
  (`ScreenForm.groovy:246-276`).
- **The saved finds** (`screen/saved_finds.rs`): the request parameters a saved find stands for
  (`makeFormListFindParameters`, `screen/saved_finds.rs:126-164` vs `ScreenForm.groovy:2288-2317`);
  the finds of the user and of the user's groups with their description, parameters and
  `isByUserId` (`screen/saved_finds.rs:240-302` vs `ScreenForm.groovy:298-323`); the user's default
  find of a screen (`screen/saved_finds.rs:304-318` vs `ScreenForm.groovy:324-332`); the delete of
  a find with its fields once no user and no group has it (`screen/saved_finds.rs:506-545` vs
  `ScreenForm.groovy:2359-2387`); the save of a find, of a group's find copied for the user, of
  its `FormConfig` and of its fields (`screen/saved_finds.rs:666-806` vs
  `ScreenForm.groovy:2475-2558`); the fields a find records (`makeFormListFindFields`,
  `screen/saved_finds.rs:343-406` vs `ScreenForm.groovy:1862-1930`); the user's columns of a
  form-list and their reset (`screen/saved_finds.rs:878-1088` vs `ScreenForm.groovy:1721-1860`,
  `:2561-2682`).
- **The screen documents**: the list of a screen's documents in the user's locale with the title of
  the file when there is none (`screen/saved_finds.rs:1090-1128` vs
  `ScreenDefinition.groovy:731-760`), and the document of a `docIndex` rendered as a template
  (`screen/saved_finds.rs:1130-1161` vs `ScreenDefinition.groovy:1164-1193`).
- **Java's field validation helpers** (`getFieldValidationClasses`,
  `getFieldValidationJsRules`, `getFieldValidationRegexpInfo`,
  `ScreenForm.groovy:1509-1644`) are not used anywhere in the Java tree, not even in the macros,
  so their absence in the port is not a gap.

#### Out of scope

- **The FreeMarker macro libraries** (`DefaultScreenMacros.*.ftl`, `ScreenHtmlMacros.ftl`): the port
  replaces them with the JavaScript modules under
  `runtime/base-component/framework/template/screen/` (Parity-Mapping, Macro libraries; the FreeMarker
  engine is in "Excluded on purpose").
- **The `js` and `vuet` render modes** and `/vapps` (Parity-Mapping, Excluded on purpose); the port's
  modes are `text`, `html`, `html-print`, `xml`, `csv`, `qvt`, `qjs` and `qvue`.
- **Groovy expression evaluation** of `from`, `value`, `condition` and `disable-when`: the port
  evaluates JavaScript (Parity-Mapping, `script/runtime.rs`, Redesign).
- **`ScreenWidgetRender` and `ScreenWidgetRenderFtl`**: the port generates the code of a screen
  from its JavaScript macro module (`screen/generate.rs`), a different phase of this review.
- **The screen URL resolution** (`ScreenUrlInfo`, `findSubscreenPath`, the screen history and the
  saved parameters): `screen/url.rs` and `screen/urls.rs`, a later phase.

#### Open questions

- Should the saved-find hardening (P1-05, P1-07, P1-08) be carried into Parity-Diffs? The
  archived Code-Quality-Review (H14) records the rationale — "formSaveFind and formSelectColumns
  take only a POST with the session token. A saved find can be scheduled or made the default only
  by its owner or one of its groups. A description is never template source" — but Parity-Diffs
  has no row for it, so the next reader has to find the archive to know it is deliberate.
- Should a dynamic form's saved finds and column choice be keyed per entity or service, as Java
  does (P1-06)? The port's macros would then have to post the extension with the form location.
- Should the port's XSD declare what its code reads (P1-11), and should `moqui lint --screens`
  check the attributes against the XSD (the open gallery finding F24)?

## Phase 2 — Screen render and the SRI surface

Compared `crates/moqui-core/src/screen/render.rs`, `crates/moqui-core/src/screen/sri/{mod,fields,forms,options,sections,layout,menu_items,theme}.rs` and
`crates/moqui-core/src/screen/aggregate.rs` against `../moqui/framework/src/main/groovy/org/moqui/impl/screen/ScreenRenderImpl.groovy`,
`../moqui/framework/src/main/groovy/org/moqui/impl/screen/WebFacadeStub.groovy` and
`../moqui/framework/src/main/groovy/org/moqui/impl/entity/AggregationUtil.java` (with `ScreenDefinition.groovy`, `ScreenForm.groovy`,
`ScreenUrlInfo.groovy`, `ResourceFacadeImpl.groovy`, `WebFacadeImpl.groovy` and `MessageFacadeImpl.groovy` read for the branches they decide).

Worked 2026-10-19: 13 findings (0 High, 7 Med, 6 Low) — `save-current-screen` records
the wrong thing, the grouped average is wrong, the screen's `set` element decides differently, the form-list does not set the two shared-map
counters, the screen's `render-modes` is enforced where Java only advises it, the text-mode column widths are laid out differently, and six
smaller ones; the sri surface itself is complete.

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P2-02 | `screen/render.rs:1340-1342` | `ScreenRenderImpl.groovy:519-523` | `save-current-screen="true"` pushes the screen onto the screen history; Java saves it as the `screen-last` target with its parameters | Med | No |
| P2-03 | `screen/aggregate.rs:156-198`, `:304-330` | `AggregationUtil.java:212-234`, `:339-359` | The average of a group leaves out its first row and its count is one short; Java throws a `NullPointerException` on the second row of the group and the render fails | Med | No |
| P2-04 | `screen/aggregate.rs:143-147` | `AggregationUtil.java:202-210`, `ScreenForm.groovy:2038` | The group key uses the group-by field even when the field is not displayed or hidden; Java leaves it out of the key, so all rows fall in one group | Low | No |
| P2-05 | `screen/aggregate.rs:86-104` | `ScreenForm.groovy:1403-1407`, `:46-50` | An `aggregate` or `show-total` that is not a function name is logged and ignored; Java's `AggregateFunction.valueOf` throws and the screen fails to load | Low | No |
| P2-06 | `screen/sri/fields.rs:336-360` | `ScreenRenderImpl.groovy:1459-1465`, `ResourceFacadeImpl.groovy:439-451` | `sri.setInContext` skips a set when the field already has a value, always sets an empty value, and localizes the `value` and `default-value`; Java sets a truthy value, keeps the old value otherwise, and expands without l10n | Med | No |
| P2-07 | `screen/sri/forms.rs:194-257` | `ScreenForm.groovy:2131-2138` | The form-list render does not put `_formListRendered` and `_formListResultCount` in the shared context map | Med | No |
| P2-08 | `screen/render.rs:432-437` | `ScreenDefinition.groovy:212`, `ScreenUrlInfo.groovy:500` | A render mode the screen does not name in `render-modes` is a 404; Java renders it and only reports the modes to the client | Med | Unclear |
| P2-09 | `screen/render.rs:560-565` | `WebFacadeImpl.groovy:262-273` | The sign-in pages are left out of the screen history; Java puts every screen there | Low | No |
| P2-10 | `screen/render.rs:191`, `:969-978` | `ScreenRenderImpl.groovy:443-476`, `WebFacadeStub.groovy:126` | A render with no socket (the `make_render` / `ScreenTest` seam) skips the session-token and the insecure-request checks; Java's stub goes through them | Low | No |
| P2-11 | `screen/render.rs:1355-1358` | `ScreenRenderImpl.groovy:573`, `:281-310` | A JSON transition answer keeps the messages out of the session; Java saves them to the session and returns them in the JSON as well | Low | No |
| P2-12 | `screen/render.rs:1408-1462` | `ScreenRenderImpl.groovy:609-610`, `ScreenUrlInfo.groovy:1194-1202` | The saved parameters (`pageNoLimit`, `lastStandalone`, `renderMode`) are not copied into the URL of a redirect; Java copies them | Low | No |
| P2-13 | `screen/render.rs:750-763` | `ScreenRenderImpl.groovy:998-1009` | `allow-web-request="false"` answers 403; Java throws a `BaseArtifactException`, so 500 | Low | No |
| P2-14 | `screen/sri/layout.rs:102-104`, `:129-135` | `ScreenForm.groovy:2247`, `:2273` | A column with no `print-width` gets a share of the line in the port and no width at all in Java (an integer division); the leftover characters are given out in the port and dropped in Java | Med | No |

P2-01 was found here first and is kept as **P1-04** in Phase 1: the two phases found the same code path
(`screen/render.rs:1135-1139`) and the same Java rule. The extra detail it carried — that Java's
`hasError()` also counts the validation errors while the port's `has_error()` (`service/util.rs:30-32`)
counts only the error list — is in P1-04.


#### P2-02 — `save-current-screen` saves the wrong thing

A response with `save-current-screen="true"` calls `user::session::push_screen_history` in the port
(`screen/render.rs:1340-1342`). Java calls `saveScreenLastInfo(screenPath, null)`
(`ScreenRenderImpl.groovy:519-523`), which puts the path in `moqui.screen.last.path` and the request parameters in
`moqui.screen.last.parameters` — the pair a later `screen-last` response goes back to
(`WebFacadeImpl.groovy:1323-1341`).

The two are different stores. In Java the transition names the screen a `screen-last` returns to; in the port the transition only
adds a line to the screen history, and a `screen-last` response falls back to the newest history entry — a screen the user
happened to visit, not the one the transition named. A user who saves the current screen from a transition and then comes back
with `screen-last` lands on the wrong page in the port, and the parameters Java keeps with the saved screen are lost. The fix is
to call `user::session::save_screen_last(path, &parameters)` (the same call `redirect_to_login` makes at
`screen/render.rs:816`) instead of pushing the history.

#### P2-03 — The average of a group leaves out its first row

`Aggregation::apply` handles the first row of a group apart (`screen/aggregate.rs:156-198`): a field with a function other than
`count` is copied into the row as it is, so an `aggregate="avg"` field gets the row's value and **no** `nCount` and `nTotal`
counters. The second row of the group goes to `apply_function`, whose `Avg` branch reads those two counters
(`screen/aggregate.rs:304-330`); they are missing, so it takes the "first value" arm, writes the counter as 1 and the total as
that row's value, and the first row's value is gone.

For a group of three rows of 10, 20 and 30 the port shows 20 (the average of 20 and 30) with a count of 1.

Java does not get that far. Its first row of a group copies the value the same way (`AggregationUtil.java:230-234`), so the
counters `doFunction(AVG)` needs are never seeded, and the second row reads them as null:
`BigDecimal count = (BigDecimal) resultMap.get(fieldCountName); ... BigDecimal avgTotal = total.add(newNumBd);`
(`AggregationUtil.java:351-353`) — a `NullPointerException`, so the render of such a form-list fails with a 500 in Java. (The
average of the *totals* row, `show-total="avg"`, is fine in both: there `doFunction` is called for every row, so the first call
seeds the counters.)

So the port does not crash where Java does, but it shows a number that is not the average. The fix is to run the `avg` function
for the first row of a group as well, so the counters are there and the average is the average of every row of the group.

No screen in `runtime/` uses `aggregate=` today (only `show-total=`), so nothing shipped shows the wrong number yet; the port's own
test (`screen/tests/forms.rs:661-716`) covers `sum`, `group-by` and `sub-list` but not `avg`.

#### P2-04 — The group key ignores the displayed-field rule

Java builds the key of a group from the group-by fields that are in `includeFields` — the displayed fields of the form plus its
hidden fields (`ScreenForm.groovy:2038`, `:2066`) — and skips the rest (`AggregationUtil.java:202-210`). The port always uses
every group-by field (`screen/aggregate.rs:143-147`).

A group-by field that is neither displayed nor hidden is therefore the key of one group in Java (the key map is empty, so every
row matches every other row) and the key of several groups in the port. A form-list that groups by a field the user cannot see
shows one row in Java and one row per value in the port. The fix is to pass the displayed and hidden field names of the form to
`Aggregation::apply` and to leave a group-by field out of the key when it is not among them.

#### P2-05 — An unknown aggregate function is ignored instead of failing

`Aggregation::of` reads `aggregate` and `show-total` and, when the name is not a function, logs an error and carries on
(`screen/aggregate.rs:86-104`). Java builds the function with `AggregateFunction.valueOf(aggregate.toUpperCase())`
(`ScreenForm.groovy:1403-1407`, `:46-50`), which throws `IllegalArgumentException` for a name that is not a function — the
`if (af == null)` check that follows it can never be true. A screen with a typo in `aggregate="summs"` loads and renders in the
port and fails to load in Java.

The port's behaviour is the one the Java code meant to have (the log message says "Ignoring aggregate ... not a valid function"),
so this is a case where the port is more forgiving than the Java it copies. It should either keep the throw or be recorded as a
deliberate difference.

#### P2-06 — `sri.setInContext` decides differently from Java in three ways

`sri.setInContext` is the `set` element of a field's sub-node (`ScreenRenderImpl.groovy:1459-1465` calls
`ResourceFacadeImpl.setInContext`). The port's version (`screen/sri/fields.rs:336-360`) differs from Java's
(`ResourceFacadeImpl.groovy:439-451`) in three ways:

- **The skip rule.** The port skips the set when `set-if-empty="false"` and the field already holds a value. Java sets the field
  whenever the new value is truthy; `set-if-empty` only decides what happens to an *empty* value. So a
  `<set field="status" from="newStatus" set-if-empty="false"/>` updates the field in Java and leaves the old value in the port.
  The XSD (`xml-actions-3.xsd:228-230`) says the same as Java: "If an empty String or null value is found, set that in the field.
  Defaults to true, set to false to do nothing to the field."
- **The default.** The port always sets the field, so a `<set field="x" from="emptyThing"/>` clears `x`. Java only sets a truthy
  value, so `x` keeps the value it had. (Java's default is the opposite of the XSD's "true" default, but the two runtimes should
  at least agree with each other.)
- **The l10n.** The port expands the `value` and the `default-value` with `expand_localized`; Java expands them with
  `expand(..., localize=false)`. A value that names a label is translated in the port and not in Java.

The port's own actions runner (`runtime/base-component/framework/template/XmlActions.js:336-345`) uses a third rule again
(`set-if-empty="false"` → do not set an empty new value), so the same attribute means three things in the port. Picking one rule —
Java's, or the XSD's — and using it in all three places is the fix.

#### P2-07 — The form-list does not set `_formListRendered` and `_formListResultCount`

Java's `getListObject` puts `_formListRendered` and `_formListResultCount` in the shared context map after the list is built
(`ScreenForm.groovy:2131-2138`), so that code running later can tell that a list was rendered and how many rows it had. The
port's `open_form_list` / `form_rows` (`screen/sri/forms.rs:194-257`) do not set them, although the port has a shared map
(`context/factory.rs:672`).

The effect is already visible in the port's own runtime: `runtime/base-component/framework/service/org/moqui/impl/ScreenServices.xml:138-140`
says "Java reads `_formListRendered` and `_formListResultCount` from the shared context map; the port's form-list render does not
set them, so noResultsAbort does not stop a send". A scheduled screen with `noResultsAbort="Y"` whose form-list found no rows
sends the email in the port and does not in Java. Setting the two values in `form_rows` (the count is the number of rows the
aggregation produced) closes it.

#### P2-08 — The screen's `render-modes` is enforced

The port refuses a render whose mode the target screen does not name: `rendered.allows_render_mode(&mode)` gives a 404
("render mode csv is not supported by ..."), `screen/render.rs:432-437`. In Java `render-modes` is a set the definition keeps
(`ScreenDefinition.groovy:212`) and reports to the client in the menu data; nothing checks it before the render, so a screen that
names `render-modes="html"` is still exported as CSV and rendered as `text` (`ScreenUrlInfo.groovy:500` only uses the mode to
decide whether a path extension is a render mode at all).

Two things follow. A screen that lists some modes cannot be asked for another one in the port, where Java renders it — a CSV
export of a `render-modes="html"` screen is a 404. And the port's own screens that name the vuet modes, which the port does not
have, cannot be rendered at all: `runtime/base-component/example/screen/Gallery/Board.xml:19` and `Calendar.xml:19` say
`render-modes="vue,qvue"`, so in the port an html or a qvt request for them is a 404 (the gallery spec asks for `qvue`, which the
mode list allows). Java renders all of them.

The check is not recorded as a difference; [Parity-Diffs](../Parity-Diffs.md) mentions the refusal in passing in the rationale of
the P8b-09 row ("This port's screen renderer refuses a mode the screen does not list (`screen/render.rs:410`)"), but not as a
difference of its own. Either the check goes, or the port's screens that name the vuet modes are changed and the difference is
recorded.

#### P2-09 — The sign-in pages are left out of the screen history

`record_history` skips a screen whose first path element is `Login`, `SecondFactor`, `ChangePassword`, `sso`, `error`, `rest` or
`rpc` (`screen/render.rs:560-565`). Java's `saveScreenHistory` has no such rule: it stores every screen that is not standalone,
has no `history="false"` and was not a fragment (`WebFacadeImpl.groovy:262-273`).

The history is the fallback a `screen-last` response uses when no screen was saved (`ScreenRenderImpl.groovy:558-568`), so in
Java a `screen-last` after a login can land on the login page and in the port it lands on the page before it. The rule is
defensible — a sign-in page is not a place to come back to — but it is a difference and should be recorded.

#### P2-10 — The render with no socket skips the request checks

`make_render` and `ScreenTest` set `enforce_request_checks: false` (`screen/render.rs:191`), which skips the redirect to https,
the redirect to the login screen (`screen/render.rs:391`) and the session-token check (`screen/render.rs:969-978`).
Java's `ScreenTest` goes through the same `internalRender` with a `WebFacadeStub`, so the checks run there: the stub's
`getSessionToken()` answers `TestSessionToken` (`WebFacadeStub.groovy:126`) and a POST of a transition with actions therefore
fails with "Session token required (in X-CSRF-Token)" unless the test passes that token.

A screen test (or a service that renders a screen) that posts a transition runs in the port and fails in Java. The `dynamic-options`
and `getFieldTransitionValue` calls are not affected: they render as a GET, which Java does not check either.

#### P2-11 — A JSON answer keeps the messages out of the session

`respond` saves the messages to the session only when the answer is not JSON (`screen/render.rs:1355-1358`), with the comment
"A JSON answer carries the messages itself, so they are not kept for the next page as well". Java calls `saveMessagesToSession()`
for every redirect, JSON or not (`ScreenRenderImpl.groovy:573`), and `getBasicResponseMap` puts the same messages in the JSON
(`:281-310`), so in Java the messages are shown twice: once in the JSON answer and once on the page the client goes to next.

In the port the next page shows nothing. For a background form that posts and then navigates, the message is lost. The fix is to
call `save_messages()` for the JSON case as well.

#### P2-12 — The saved parameters are not copied into the redirect URL

Java copies three parameters out of the saved parameters (`moqui.saved.parameters`) into the URL of a redirect:
`pageNoLimit`, `lastStandalone` and `renderMode` (`ScreenRenderImpl.groovy:609-610`, `ScreenUrlInfo.groovy:1194-1202`). The
port uses the saved parameters only for a `screen-last` response (`screen/render.rs:1402-1406`); a normal `url` response does not
get them.

A response with `save-parameters="true"` (the runtime uses it in `webroot.xml:45`, `Tools/Service/ServiceRun.xml:35` and
`Tools/Entity/DataImport.xml:68`) therefore loses the "show all rows" (`pageNoLimit`) and the render mode of the request that
saved them: the list is paged again after the form post, where Java keeps it unpaged.

#### P2-13 — `allow-web-request="false"` answers 403 instead of 500

`check_web_settings` refuses a screen with `allow-web-request="false"` with an `Unauthorized` error
(`screen/render.rs:750-763`), which the web layer answers with 403. Java throws a `BaseArtifactException`
(`ScreenRenderImpl.groovy:1002-1003`), which is a general error and answers 500.

The port's status is the better one (the screen is not allowed in a web request, the request is refused), but the two differ and
a client that watches for 500 sees a change.

#### P2-14 — The text-mode column widths are laid out differently

`form_list_column_char_widths` gives a column with no `print-width` a share of the line — `100 / percentColsCount` as a decimal
(`screen/sri/layout.rs:102-104`) — and hands the characters that are left over to the percent columns
(`screen/sri/layout.rs:129-135`, with a decimal division for the per-column adjustment).

Java computes both with an integer division: the missing percent width is `(1 / percentColsCount) * 100`, which is `0` for more
than one percent column (`ScreenForm.groovy:2247`), and the adjustment per column is
`(diffRemaining / percentColsCount).setScale(0, RoundingMode.UP)`, which is `0` as well (`ScreenForm.groovy:2273`) — so the
adjustment never moves a character.

A form-list in the `text` mode (and the `csv` mode, which uses the same widths) is therefore laid out differently. Two columns,
one with `print-width="20"` (a percent width, the default type) and one with no `print-width`: Java gives the first column all
130 characters and the second none, the port gives them 48 and 82. And when no column has a percent width at all, Java divides
`100 / percentTotal` by zero (`ScreenForm.groovy:2255`) and the render fails, where the port spreads the line over the columns.

The fixed widths, the percent widths, the zero width (which hides a column and gives its separator back) and the error when the
fixed widths exceed the line are the same in both. The difference is in the two places where Java's integer division drops the
share of a column that has no width of its own. The port's behaviour is the one the Java code looks like it meant to have, so
this is a case for a recorded difference rather than a change.

#### Phase 2 checked and found equivalent

- **The sri surface, method by method.** Java's `sri` is the `ScreenRenderImpl` itself (`internalRender` puts it in the context
  stack, `ScreenRenderImpl.groovy:314`), so every public method of that class is a method a macro may call. The port builds the
  same object in `script/runtime/screen_host.rs:992-1068` and `script/runtime/urls.rs:274-284`, and the port's own macro modules
  use the same names (`runtime/base-component/framework/template/screen/*.js`). Compared name by name:
  - the render control — `doBoundaryComments`, `getActiveScreenDef`, `getScreenUrlInfo`, `getScreenUrlInstance`,
    `getCurrentScreenUrl`, `getRenderMode`, `pushContext`, `popContext`, `setInContext`, `renderSection`,
    `renderSectionInclude`, `renderSubscreen`, `renderIncludeScreen`, `renderText`, `renderDynamicForm`,
    `renderUserColumnsForm`, `appendToAfterScreenWriter`, `getAfterScreenWriterText`, `appendToScriptWriter`,
    `getScriptWriterText`, `encodeHtml` — all present with the same meaning (`sri/mod.rs`, `sri/sections.rs`,
    `script/runtime/screen_host.rs`).
  - the field values — `getFieldValue`, `getFieldValueString` (both overloads), `getFieldValuePlainString`,
    `getFieldValueClass`, `getNamedValuePlain`, `getFieldEntityValue`, `getFieldOptions`, `getFieldTransitionValue`,
    `getContextValue` (through `node_parameters`) — present (`sri/fields.rs`, `sri/options.rs`); the widget overload of
    `getFieldValueString` reads the widget's `format` and its `text-format` in the `text` and `csv` modes as Java does
    (`ScreenRenderImpl.groovy:1540-1555`), and the three-argument overload takes the field node as Java's does.
  - the forms — `getFormNode`, `getFormInstance`, `getFormHiddenParameters`, `getUserFormListFinds`,
    `getUserDefaultFormListFindId`, `pushSingleFormMapContext`, `getSingleFormMap`, `openFormList`, `startFormListRow`,
    `endFormListRow`, `startFormListSubRow`, `endFormListSubRow`, `safeCloseList`, `formListColumnCharWidths` — present
    (`sri/forms.rs`, `sri/layout.rs`).
  - the URLs — `buildUrl`, `buildUrlInfo`, `buildUrlFromTarget`, `makeUrlByType`, `isAnchorLink`, `activeInCurrentMenu` — present
    (`screen/urls.rs`).
  - the theme and the menu — `getThemeValues`, `getThemeIconClass`, `getMenuData` — present (`sri/theme.rs`,
    `screen/menu.rs`); `getCurrentThemeId` is not on the port's `sri` object, but the Java macros do not call it either (they
    call `getThemeValues`, which the port has with the same one- and two-argument shape).
  - the five that Java's qvt macro calls and the port's `sri` does not have — `getFormFieldValues`,
    `getFormListHeaderValues`, `getFormListRowValues`, `makeFormListSingleMap`, `makeFormListMultiMap` — are done by the port's
    qvt macro instead (`ScreenQvt.js:386-414`, `singleValues` / `headerValues` / `rowValues`), with the same values: the
    pass-through parameters and `moquiFormName` and `lastUpdatedStamp` of a form-single, the hidden parameters and `orderByField`
    and `formListFindId` and `pageSize` of a form-list header, the URL parameters and the row values and `pageIndex` of a row.
    The same holds for `transformFormListRow` and `addFormFieldValue`, which the port's macro does in `addValue`
    (`ScreenQvt.js:286-360`) with the same per-widget values Java's `addFormFieldValue` puts out
    (`ScreenRenderImpl.groovy:1793-2003`). A macro written by hand against Java's `sri` still cannot call those five, though.
  - the ones Java has that no macro calls and the port does not need — `getWriter`, `getOutputStream`, `getRootScreenDef`,
    `getNextScreenDef`, `getActiveScreenPathName`, `getNextScreenPathName`, `getActiveScreenPath`,
    `getScreenPathHasTransition`, `isInCurrentScreenPath`, `getTemplate`, `getScreenWidgetRender`,
    `getSectionIncludeName`, `getBaseLinkUri`, `getDefaultText`, `addNodeParameters`, `buildUrlInfo`, `sendHttpRedirect`,
    `sendJsonRedirect`, `getBasicResponseMap` — the servlet types are gone with the servlet, and the rest are the internals of the
    path and the URL (`screen/url.rs`, `screen/urls.rs`). `getBaseLinkUri` is the one with a known gap: the port keeps
    `baseLinkUrl` on the render (`screen/render.rs:255-258`) and nothing reads it, which is the open item in
    [Remaining-Work](../Remaining-Work.md) ("`sri.baseLinkUrl(url)` is kept on the render but nothing reads it"), so this phase
    confirms it rather than reports it.
- **The render mode.** Java takes the mode from the builder, then the `renderMode` request parameter, then the extension of the
  target screen, and falls back to `html` (`ScreenRenderImpl.groovy:416-431`); the port does the same in that order
  (`screen/render.rs:708-729`), and both map `xsl-fo` to the print mode. The content type and the character encoding come from
  the `web-settings` of the screens in the render path, else from the mode (`ScreenRenderImpl.groovy:1005-1008`, `:423-431`); the
  port does the same (`screen/render.rs:733-748`), with `require-encryption` defaulting to true when there is no `web-settings`
  element, as Java's `checkWebappSettings` treats it (`screen/definition.rs:810-812`).
- **The always-actions and pre-actions.** Java runs the `always-actions` of every screen in the path from the root, then the
  `pre-actions` from the render start (or from the third screen for a mode that is always standalone), in two separate passes with
  the artifact frames of the screens above the render start kept on the stack so the screens below inherit
  (`ScreenRenderImpl.groovy:842-940`); the port does the same in `render_screens` (`screen/render.rs:579-683`), including the
  always-standalone start index (`:628-633`) and the inherited frames (`:664-680`).
- **The required parameters of a screen.** Java checks the required parameters of the screens to render after the actions have
  run, adds the error "Required parameter missing (name)" and leaves the screen out of the render
  (`ScreenRenderImpl.groovy:945-959`); the port does the same (`screen/render.rs:650-661`) and the screen is skipped in
  `render_subscreen` (`sri/sections.rs:388-395`) as Java skips it in `renderSubscreen` (`ScreenRenderImpl.groovy:1189`).
- **The transition responses.** Java picks the `error-response` when there are errors, else the last `conditional-response` whose
  condition holds, else the `default-response` (`ScreenDefinition.groovy:1010-1021`); the port picks the same
  (`screen/render.rs:1291-1308`). A response of type `none` saves the parameters and the error parameters and sends nothing
  (`ScreenRenderImpl.groovy:529-535`); the port does the same (`screen/render.rs:1343-1351`). A `screen-last` response goes back
  to the saved screen, else to the newest history entry, else to the root (`ScreenRenderImpl.groovy:541-570`); the port does the
  same (`screen/render.rs:1365-1401`), with the same-origin check on a saved absolute URL.
- **The transition parameters.** Java binds the extra path names to the `path-parameter` names and then the screen and transition
  `parameter` entries into the context (`ScreenDefinition.groovy:912-937`); the port does the same
  (`screen/render.rs:1310-1332`), and a missing required parameter of the transition is a 400 in the port
  (`screen/render.rs:1052-1056`) as it is a `BaseArtifactException` in Java (`ScreenDefinition.groovy:963-975`).
- **The authorization of the screens and the transition.** Java pushes a screen frame with the authz required for the target
  screen only when the screen requires authentication, and a transition frame with the authz action of the transition (the
  `authz-action`, else the single service's own, else view for a read-only transition)
  (`ScreenDefinition.groovy:894-896`, `:943-947`); the port does the same (`screen/render.rs:907-929`, `:1147-1154`).
  `anonymous-all` and `anonymous-view` are applied the same way (`ScreenRenderImpl.groovy:674-681`; the port at
  `screen/render.rs:918-929`).
- **The artifact hits.** Java counts a screen hit with the content type, and a transition hit named `screen#transition` with the
  response type (`ScreenRenderImpl.groovy:505-511`, `:664-670`); the port counts the same two (`screen/render.rs:1026-1049`,
  `sri/sections.rs:88-93`).
- **The login and the https redirects.** Java redirects to the last `login-path` of the path (or `/Login`) when a screen that
  requires authentication is asked for with no user, and to https when the webapp has https on
  (`ScreenRenderImpl.groovy:1010-1106`); the port does the same (`screen/render.rs:767-834`), with the 401 for a fragment or a
  transition in both.
- **The screen history.** Java stores every screen that is not standalone, has no `history="false"` and was not a fragment
  (`WebFacadeImpl.groovy:262-273`); the port stores the same set, apart from the sign-in pages (P2-09).
- **The saved find.** Java reads the user's default saved find for the target screen, honours `_clear`, and adds the saved find's
  parameters when the request has none of its own (`ScreenRenderImpl.groovy:340-378`); the port does the same
  (`screen/saved_finds.rs:170-238`), with the `pageSize` preference handled in the same place.
- **The messages.** The messages, the errors and the validation errors ride the session to the next screen
  (`WebFacadeImpl.groovy:1343-1358`); the port saves them the same way (`screen/render.rs:1619-1646`), and the validation error
  of a field is shown by that field and not also as an error line in both.
- **`WebUtilities.encodeHtml`.** The port's `encode_html` (`sri/mod.rs:157-170`) escapes the same five characters as Java's
  (`ScreenRenderImpl.groovy:1169`, `:1197`, `:1239`).
- **The aggregation.** The functions are the same seven (`min`, `max`, `sum`, `avg`, `count`, `first`, `last`; Java's
  `AggregateFunction` at `AggregationUtil.java:37`), `show-total="true"` means `sum` and `show-total="false"` means none in both
  (`ScreenForm.groovy:1394-1396`; the port at `screen/aggregate.rs:86-90`), a form with aggregate fields and no group-by field is
  an error in both (`ScreenForm.groovy:1425-1430`; the port at `screen/aggregate.rs:118-123`), the totals row is one more row at
  the end marked `_moquiRowType: "total"` in both (`AggregationUtil.java:145-150`; the port at `screen/aggregate.rs:244-248`), the
  sub-list of a group is the list of the original rows with the sub-list totals last in both
  (`AggregationUtil.java:134-144`; the port at `screen/aggregate.rs:234-243`), and `count`, `min`, `max`, `sum`, `first` and `last`
  behave the same over the rows of a group (apart from the average, P2-03). The row actions and the `from` expressions of the
  fields run per row with the entry, the index and the has-next in the context under the same names in both
  (`AggregationUtil.java:175-198`; the port at `screen/sri/forms.rs:216-247`).
- **The form-list column widths.** `formListColumnCharWidths` reads the same `print-width` and `print-width-type` of each field
  of each column as Java's `getFormListColumnCharWidths` (`ScreenForm.groovy:2180-2234`), the fixed widths, the percent widths
  and the zero width (which hides the column and gives its separator back) come out the same, and so does the error when the
  fixed widths exceed the line — the port's own tests at `sri/mod.rs:251-294`. The two places where Java's integer division
  drops a share are P2-14.

#### Out of scope

- The vuet, vue and js render modes and `DefaultScreenMacros.vuet.ftl`: excluded on purpose (Parity-Mapping, "Excluded on
  purpose"). The port's `qvue` mode is its own thing.
- FreeMarker as the template engine, and with it `FtlTemplateRenderer` and the `ftl` files: excluded on purpose. The port's
  `renderText` refuses a `.ftl` file with a clear message (`sri/sections.rs:444-448`), which is the port's own decision, not a
  Java one.
- The servlet types of `sri.getWriter` and `sri.getOutputStream`: the port has no servlet.
- `ScreenTestImpl` as a whole is `screen/screen_test.rs`, which is outside this phase; only the seam it shares with
  `make_render` was compared (P2-10).

#### Open questions

- Should the port enforce the screen's `render-modes` (P2-08)? If it does, the screens that name the vuet modes
  (`Gallery/Board.xml`, `Gallery/Calendar.xml`, `Tools/Service/ServiceLoadRunner.xml`) have to change, because the port has no
  `vue` mode; if it does not, the check in `screen/render.rs:432-437` goes and Java's advisory meaning is kept.
- Which rule for the `set` element's `set-if-empty` is the port's: Java's (a truthy value is always set), the XSD's ("set to false
  to do nothing to the field"), or the one the actions runner already uses (P2-06)? Whichever it is, the three places should
  agree.
- Should `_formListRendered` and `_formListResultCount` be set by the form-list render (P2-07), or is the workaround in
  `ScreenServices.xml` the answer the port wants? The workaround is in the runtime, so it is a port decision, not a gap in the
  framework.

## Phase 3 — Macro libraries: html, csv, text, xml, plain

Compared `runtime/base-component/framework/template/screen/ScreenHtml.js`, `ScreenCsv.js`,
`ScreenText.js`, `ScreenXml.js`, `ScreenPlain.js`, `ScreenSupport.js` and
`crates/moqui-core/src/screen/generate.rs` against `DefaultScreenMacros.html.ftl`,
`DefaultScreenMacros.csv.ftl`, `DefaultScreenMacros.text.ftl`, `DefaultScreenMacros.xml.ftl`,
`DefaultScreenMacros.plain.ftl`, `DefaultScreenMacros.any.ftl`, `ScreenHtmlMacros.ftl`,
`ScreenWidgetRender.java` and `ScreenWidgetRenderFtl.groovy`.

Worked 2026-10-19: 6 findings — an element no mode knows fails the render instead of the
placeholder FreeMarker writes, the PDF export lost its page-layout dialog, an anchor link is
decided by read-only instead of by actions, the text export modes read `hide` as a condition, the
export modes ignore the typed columns, and the html display lost its value class.

| ID | Rust/JS location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P3-01 | `ScreenSupport.js:31-55`, `crates/moqui-core/src/script/walker.rs:161`, `screen/generate.rs:500` | `DefaultScreenMacros.html.ftl:17`, `DefaultScreenMacros.csv.ftl:23`, `DefaultScreenMacros.text.ftl:30`, `DefaultScreenMacros.xml.ftl:16`, `DefaultScreenMacros.plain.ftl:16` | An element no mode names is an error ("no screen macro in html, element X") and fails the render; Java's `@element` fallback writes a placeholder and the rest of the page renders | Med | Unclear |
| P3-02 | `ScreenHtml.js:978-981` | `DefaultScreenMacros.html.ftl:1114-1160`, `:1286-1289` | The `show-pdf-button` is a plain link to `html-print`; Java opens a "Generate PDF" dialog with a page-layout choice (10 layouts) and a file name, so the page layout cannot be chosen from the screen | Med | Unclear |
| P3-03 | `screen/urls.rs:788-796`, `ScreenHtml.js:562`, `:576-597` | `DefaultScreenMacros.html.ftl:421-423`, `:456-458`, `ScreenUrlInfo.groovy:993` | A link is an anchor when the URL is read-only; Java makes it an anchor when the transition has no actions, so a transition with actions and `read-only="true"` is a GET link here and a POST button in Java | Low | No |
| P3-04 | `ScreenCsv.js:94-98`, `ScreenText.js:175`, `ScreenXml.js:71` (with `ScreenSupport.js:148-156`) | `DefaultScreenMacros.csv.ftl:147`, `DefaultScreenMacros.text.ftl:124`, `:240`, `DefaultScreenMacros.xml.ftl:102` | The csv, text and xml modes evaluate a field's `hide` as a condition; Java compares it with the literal `"true"`, so a field hidden by an expression is dropped from the export here and kept in Java | Low | No |
| P3-05 | `ScreenCsv.js:92`, `ScreenText.js:185`, `ScreenXml.js:71` | `DefaultScreenMacros.csv.ftl:97`, `DefaultScreenMacros.text.ftl:167`, `DefaultScreenMacros.xml.ftl:68`, `ScreenForm.groovy:1743-1780` | The csv, text and xml modes always use the untyped columns; Java's form-list column info picks the `columns type="…"` set that matches the request's `_uiType` | Low | No |
| P3-06 | `ScreenHtml.js:497-499` | `DefaultScreenMacros.html.ftl:1960` | The display span has no field-value class; Java writes `sri.getFieldValueClass(...)` (the value's type name) on it | Low | No |

#### P3-01 — An element no mode names fails the render, where FreeMarker writes a placeholder

Every mode file in Java starts with a catch-all macro: `<#macro @element>` in
`DefaultScreenMacros.html.ftl:17` (which writes `<p>=== Doing nothing for element ${.node?node_name},
not yet implemented. ===</p>`), and an empty one in `DefaultScreenMacros.csv.ftl:23`,
`DefaultScreenMacros.text.ftl:30`, `DefaultScreenMacros.xml.ftl:16` and
`DefaultScreenMacros.plain.ftl:16`. FreeMarker falls back to it for any element that has no macro of
its own (verified against FreeMarker 2.3.35: a `<#visit>` of an element with no macro runs
`@element`), so a screen that uses an element the mode does not know still renders — the element
becomes a placeholder and the rest of the page is there.

The port has no such fallback. `ScreenSupport.js:34-55` lists every element a macro table must
name, `noops(S.allElements)` builds a macro for each of them, and the walker refuses anything else:
`crates/moqui-core/src/script/walker.rs:161` returns `no screen macro in {mode}, element {name}`
(the message is built in `screen/generate.rs:500`). The error comes out of the code generation, so it
is a render error, not a build error: for a named section it is caught by `guarded` in
`screen/sri/sections.rs:44-68` and the section's output is replaced by an error message, but for the
root screen it propagates out of `render_body` (`screen/render.rs:687-689`) and the request fails.

The XSD allows this on purpose: `runtime/base-component/framework/xsd/xml-screen-3.xsd:31` has
`<xs:any minOccurs="0" maxOccurs="unbounded" processContents="skip"/>` in the `widget-elements`
group, "to facilitate extension by adding only FTL macros". A screen with a custom widget element
(a theme's own macro, or an element from a newer Moqui) therefore renders in Java and breaks in the
port.

The port's rule is deliberate and documented in `ScreenSupport.js:31-33` ("an element that is missing
is an error and an element a mode does not render is a decision in that mode's file"), which is a
good way to catch a typo — but it is not in Parity-Diffs, and the cost is a page that does not render
at all. The fix is a fallback macro for the unknown element (empty in the text modes, the Java
placeholder paragraph in html) or, at least, keeping the rest of the section when the code cannot be
made.

#### P3-02 — The PDF button lost the page-layout dialog

Java's form-list navigation writes two things for the PDF: a "PDF" button
(`DefaultScreenMacros.html.ftl:1286-1289`) and, in `paginationHeaderModals`, a "Generate PDF" modal
(`DefaultScreenMacros.html.ftl:1114-1160`) with a `layoutMaster` select offering ten page layouts
(letter/legal/tabloid/A4/A3, portrait and landscape), a `saveFilename` field, and a form that posts
to `${webappRoot}/fop${screenPath}`. The layout reaches the print renderer as the `layoutMaster`
context field (`DefaultScreenMacros.xsl-fo.ftl:21-23`).

The port writes one link (`ScreenHtml.js:978-981`):

```js
if (at(n, "show-pdf-button") === "true") {
    const print = sri.getScreenUrlInstance().cloneUrlInstance().addParameter("renderMode", "html-print").addParameter("pageNoLimit", "true");
    out += '<a data-btn="default" target="_blank" href="' + e(print.getUrlWithParams()) + '">' + e(L("Print")) + "</a>";
}
```

The port's print mode still understands the layout: `ScreenHtmlPrint.js:33` reads the `layoutMaster`
context field (`$masters[layoutMaster] ? layoutMaster : "letter-portrait"`), and the request
parameters are in the context (`screen/render.rs:412-415`), so `?layoutMaster=a4-landscape` on the
print URL works. What is gone is the way to choose it: there is no dialog, so the page layout of a
PDF export cannot be picked from the screen, and the file name cannot be given either (the browser
names the download). The html-print pipeline itself is a redesign (README redesign 5) and the
`/fop` path is kept, but the button is the html macro's own feature and the layout choice is still
supported by the mode, so the missing piece is only the dialog.

#### P3-03 — An anchor link is decided by read-only, not by actions

Java writes a link as an anchor (a plain GET link) when it is `link-type="anchor"` or
`"anchor-button"`, or when it has no `link-type` (or `auto`) and either the `url-type` is not
`transition` or the target transition has no actions
(`DefaultScreenMacros.html.ftl:421-423`, and the same test again for the form at `:456-458`).
"Has actions" is `UrlInstance.getHasActions()`: the target transition has an `<actions>` or a
`<service-call>` (`ScreenUrlInfo.groovy:993`). A transition with actions is rendered as a POST form
with a button, because a GET to it is refused by the render
(`ScreenRenderImpl.groovy:441-451`).

The port decides the same thing with `read_only`
(`crates/moqui-core/src/screen/urls.rs:788-796`):

```rust
link_type == "anchor"
    || link_type == "anchor-button"
    || ((link_type.is_empty() || link_type == "auto")
        && ((!url_type.is_empty() && url_type != "transition") || url.read_only))
```

`read_only` is true when the transition has no actions *and* no service-call, or when it declares
`read-only="true"` (`screen/definition.rs:345-371`). The two rules differ for exactly one case: a
transition that has `<actions>` and also declares `read-only="true"`. Java renders it as a POST
button (it has actions, so `!hasActions` is false); the port renders it as a GET anchor, so the
transition's parameters go into the URL and the request is a GET. The port's own guard
(`screen/render.rs:1158-1190`) allows the GET, because the transition is read-only, so nothing
breaks — but the method and where the parameters sit differ, and Java's form carries a session
token where the port's link does not.

#### P3-04 — The export modes read `hide` as a condition, Java compares it with `"true"`

A form-list field may be hidden with a `hide` attribute that is a condition
(`xml-form-3.xsd:626-633`: "If true field will always be hidden … If empty (default) will guess").
Java's html mode evaluates it (`DefaultScreenMacros.html.ftl:757` and `:1691`:
`ec.getResource().condition(fieldSubParent["@hide"]!, "")`), but the csv, text and xml modes compare
it with the literal string `"true"` (`DefaultScreenMacros.csv.ftl:147`,
`DefaultScreenMacros.text.ftl:124` and `:240`, `DefaultScreenMacros.xml.ftl:102`), so a field hidden
by an expression is still written to those exports.

The port evaluates the attribute in every mode: the csv mode guards each header and cell with
`S.hideGuard` (`ScreenCsv.js:94-98`, `ScreenSupport.js:148-156`), the text mode gives each column
field a `hide` closure that is called with the expression (`ScreenText.js:175`), and the xml mode
guards each attribute the same way (`ScreenXml.js:71`). The port's rule is the one Java's html mode
uses and the one the `hide` attribute documents, so the port is the more consistent of the two — but
the exports differ: a column that Java's CSV export writes (with the field's value) is missing from
the port's, and a column that Java's text export writes at the wrong width is dropped from the
port's.

#### P3-05 — The export modes ignore the typed columns

Java builds the columns of a form-list from the `columns` elements, choosing the set whose `type`
matches the request's `_uiType` first and the untyped set otherwise
(`ScreenForm.groovy:1743-1780`), and the csv, text and xml macros all use that list
(`DefaultScreenMacros.csv.ftl:97`, `DefaultScreenMacros.text.ftl:167`,
`DefaultScreenMacros.xml.ftl:68`). A request with `_uiType=mobile` therefore exports the mobile
columns in Java.

The port's html mode makes the same choice at run time
(`ScreenHtml.js:1824-1829`, the `_uiType` test), but the export modes call
`S.formListColumns(node)` without a type (`ScreenCsv.js:92`, `ScreenText.js:185`,
`ScreenXml.js:71`), which always takes the untyped `columns` element. An export requested with
`_uiType=mobile` in the URL (the parameter is kept in the screen URL, so the CSV link of a mobile
page carries it) has the default columns in the port and the mobile columns in Java.

#### P3-06 — The display span lost the field-value class

Java's display widget writes the value's type as a class on the span
(`DefaultScreenMacros.html.ftl:1960`): `class="text-inline ${sri.getFieldValueClass(dispFieldNode)}
…"`, where `getFieldValueClass` is the simple name of the value's class
(`ScreenRenderImpl.groovy:1646-1649`, e.g. `BigDecimal`, `Date`, `String`). A theme's CSS can use
it to style a value by its type.

The port's display writes `class="display …"` with no value class (`ScreenHtml.js:497-499`). The
port's `sri` still has `getFieldValueClass` (`crates/moqui-core/src/script/runtime/screen_host.rs`,
the `sri_get_field_value_class` method), so the html mode simply does not ask for it. A theme that
styled a display by value type loses that hook.

#### Phase 3 checked and found equivalent

- **The macro list of `DefaultScreenMacros.html.ftl`**: every macro it defines has an equivalent in
  `ScreenHtml.js` — `screen`/`widgets`/`fail-widgets` (`:1973-1975`), `subscreens-menu`/
  `subscreens-panel` (`:1977-1978`), `section`/`section-iterate`/`section-include`
  (`ScreenSupport.js:240-241`, `ScreenHtml.js:1979`), `container`/`container-box`/`container-row`/
  `container-panel`/`container-dialog`/`dynamic-container`/`dynamic-dialog`/`button-menu`
  (`:1981-2001`), `tree` (`:2003`), `link` (`:2004`), `image`/`label`/`editable` (`:2005-2007`),
  `form-single`/`form-list` (`:2018-2019`), the field layout elements `field-ref`,
  `fields-not-referenced`, `field-row`, `field-row-big`, `field-group`, `field-accordion`,
  `field-col-row` (the `layoutCode` at `:1581-1637`), and every field widget `check`, `date-find`,
  `date-period`, `date-time`, `display`, `display-entity`, `drop-down`, `file`, `hidden`, `lookup`,
  `password`, `radio`, `range-find`, `reset`, `submit`, `text-area`, `text-find`, `text-line`
  (`:2028-2045`). The helper macros (`nodeId`, `fieldName`, `fieldId`, `fieldTitle`,
  `fieldIdByName`, `widgetTextValue`, `linkFormForm`, `linkFormLink`, `paginationHeader`,
  `paginationHeaderModals`, `formSingleSubField`, `formSingleWidget`, `formListHeaderField`,
  `formListSubField`, `formListWidget`) are the runtime functions of `ScreenHtml.js` (`nodeId`,
  `titleCode`, `fid`, `fnm`, `valueText`, `linkForm`, `linkElement`, `nav`, `findDialog`,
  `singleFieldCode`, `cellCode`, `columnField`).
- **The macro list of the other modes**: `DefaultScreenMacros.csv.ftl` maps to `ScreenCsv.js`
  (`form-list` at `:95`, `link` at `:68`, the option widgets at `:151-153`, `display` at `:156`,
  `text-area`/`text-line`/`text-find` at `:159-161`, `submit` at `:158`); `DefaultScreenMacros.text.ftl`
  maps to `ScreenText.js` (`form-single` at `:194`, `form-list` at `:184`, the option widgets at
  `:304-306`, `image`/`label` at `:266-267`); `DefaultScreenMacros.xml.ftl` maps to `ScreenXml.js`
  (`form-list` at `:69`, `link` at `:127`, the option widgets at `:135-137`, the `widgets` root
  element at `:96-107`); `DefaultScreenMacros.plain.ftl` maps to `ScreenPlain.js` (the section and
  subscreen calls at `:51-55`, the text and render-mode at `:43-50`). The elements Java leaves as
  empty macros (`file`, `hidden`, `ignored`, `password`, `range-find`, `reset`, `date-find`,
  `row-actions`, `parameter`, `subscreens-menu`) are no-ops in the port, as they are in Java.
- **The shared `.any.ftl` macros**: `include-screen`, `render-mode` and `text` are in
  `ScreenSupport.js:212-243` (`anyMacros`), and they choose the text node the same way Java does
  (the last `type` that matches the mode, else the one with no type or `any`), read the location or
  the inline text, and encode when `@encode="true"`. Parity-Mapping lists
  `DefaultScreenMacros.any.ftl` as excluded because the port has no separate module for it; the
  macros themselves are in `ScreenSupport.js`.
- **How the macros get the sri**: Java's data model is the `ContextStack` with `sri` and `ec` put
  into it (`ScreenRenderImpl.groovy:314`), and the macros call `sri.xxx` and `ec.getResource()`.
  The port installs `sri` and `ec` as globals of the script engine
  (`crates/moqui-core/src/script/runtime/screen_host.rs:992`, `sri_object`) and the generated code
  calls `sri.xxx(...)` and `ec.resource.…`, with the same methods for every widget
  (`getFieldOptions`, `getFieldValueString`, `getFieldValuePlainString`, `getFieldEntityValue`,
  `getFieldValidation`, `getFieldTransitionValue`, `getFormHiddenParameters`, `openFormList`,
  `startFormListRow`, `pushSingleFormMapContext`, `setInContext`, `makeUrlByType`, `buildUrl`,
  `getMenuItems`, `getThemeIconClass`).
- **How a widget's script is emitted**: Java writes an inline `<script>` per widget (jQuery validate,
  select2, datetimepicker, typeahead, jeditable, jstree, sortableLists) and moves the menus with
  `$("#…").append(…)`. The port writes no script at all: it writes the markup and a few `data-*`
  attributes and one shared script, `runtime/base-component/webroot/screen/webroot/js/moqui-html.js`,
  binds them (the tree at `:386`, the editable at `:423`, the remote options and the autocomplete
  select at `:262-335`, the dialogs at `:574`, the sortable columns at `:559`). The libraries a widget
  needs are added to the `footer_scripts`/`footer_stylesheets` sets of the context
  (`ScreenHtml.js:189-197`, set by `runtime/base-component/webroot/screen/webroot.xml:19-24`). This is
  the documented change of the port (no jQuery), not a gap in the widgets themselves.
- **How a section's code is generated and called**: Java keeps one FreeMarker template per render
  mode (`ScreenFacadeImpl.groovy:243-266` makes `<#include macro><#visit widgetsNode>`) and
  `ScreenWidgetRenderFtl.groovy:24-35` pushes a context frame, puts `sri` and `widgetsNode` in it
  and processes the template for one section's widgets. The port keeps the same shape in
  JavaScript: `screen/generate.rs:247-326` builds one function per (mode, macro location, section,
  widgets/fail-widgets) and caches it with the definition (`:199-225`), the mode module's `macros`
  table is the FreeMarker macro set (`screen/generate.rs:462-491`), the walker turns the element tree
  into the statement list the same way `<#visit>` does, and the unit is run with the nesting depth
  (`crates/moqui-core/src/script/runtime/screen_host.rs:60-100`) so a section inside a form-list cell
  gives the state of the unit around it back. A section's widgets are the same tree in both
  (`ScreenSection.groovy:44-58` against `screen/sri/sections.rs:155-202`), the condition and the
  actions of a section run before its widgets, and the fail-widgets are used when the condition does
  not hold.
- **The form map and the field chain**: `pushSingleFormMapContext`/`popContext` around a form and
  around each field (`ScreenHtml.js:1657-1668`, `:1682-1691`) are Java's
  `sri.pushSingleFormMapContext` (`DefaultScreenMacros.html.ftl:583`, `:786`/`:817`); the
  conditional-field chain with the default-field as the fallback is Java's `formSingleSubField`
  (`DefaultScreenMacros.html.ftl:742-753`) and `formListSubField` (`:1651-1663`), and the port's
  `fieldChain` (`ScreenSupport.js:125-145`) takes the first conditional-field whose condition holds,
  else the default-field; the `<set>` elements of a field run before its widgets in both
  (`ScreenHtml.js:1562`, `:1686`; Java `:788`).
- **The form-list rows, the hidden fields and the multi form**: the row forms outside the table
  with the hidden fields of each row, the header form, the first/second/last-row forms, the multi
  form with `_isMulti` and the final row of submit widgets are written the same way
  (`ScreenHtml.js:1143-1307` against `DefaultScreenMacros.html.ftl:1359-1602`), and the row actions
  and the field `from` expressions run per row (`screen/sri/forms.rs:205-257`, Java's
  `startFormListRow`).
- **The pagination**: the nav row with the first/previous/next/last links, the page numbers, the
  "go to page" form, the "show all" link, and the CSV/XLS/text/print buttons are all written
  (`ScreenHtml.js:925-983` against `DefaultScreenMacros.html.ftl:1162-1329`); the page text is the
  localized `PaginationTemplate` with the same variables (`curPageIndex`, `curPageMaxIndex`,
  `listName`, `context[listName + 'PageRangeLow']`), which the port expands with the same map.
- **The saved finds and the column dialog**: the find-options dialog with the saved finds, the
  "save new find" form, the "update to current", the delete and the default buttons, the order-by
  select, and the sortable column dialog that posts `columnsTree` to `formSelectColumns`
  (`ScreenHtml.js:987-1065`) are the port's version of Java's `paginationHeaderModals`
  (`DefaultScreenMacros.html.ftl:837-1161`); the port's `formLocation` is the form's name and the
  server makes the full location of it (`crates/moqui-core/src/screen/saved_finds.rs:322-341`),
  which is what Java's `getSavedFindFullLocation` names.
- **The option widgets**: a drop-down, a check and a radio show the text of the option that is the
  current value, with the `no-current-selected-key` as the fallback, in the html, csv, text and xml
  modes alike (`ScreenHtml.js:455-474`, `ScreenCsv.js:54-66`, `ScreenText.js:128-140`,
  `ScreenXml.js:33-44` against Java's `check`/`radio`/`drop-down` macros).
- **The text mode's column widths**: `sri.formListColumnCharWidths`
  (`crates/moqui-core/src/screen/sri/layout.rs:27-156`) is Java's
  `getFormListColumnCharWidths` (`ScreenForm.groovy:2180-2270`), step for step: the fixed widths
  win over the percent ones, the percent widths are scaled to 100 and the remainder is given out
  column by column, and a set of fixed widths that does not fit the line is an error. The port also
  gives the width back after a container-box or a form-single, where FreeMarker kept the last
  assigned width in a global (`ScreenText.js:18-20`).
- **The csv escaping and the rows**: a value with a comma, a quote or a line break is quoted and its
  quotes doubled (`ScreenCsv.js:31-37`, Java's `csvValue` at `DefaultScreenMacros.csv.ftl:16-21`).
  The port writes one row per line; FreeMarker wrote the newline only after some of the field
  macros, so a Java CSV row ran onto the next line. A label writes nothing in the port — that is the
  recorded difference in [Parity-Diffs, Gallery fixes (2026-10-18)](../Parity-Diffs.md#gallery-fixes-2026-10-18)
  ("csv render"), and the port names it in the header comment of `ScreenCsv.js:18-20`.
- **The xml mode**: the screen's name is the root element (`ScreenXml.js:96-107`, Java's `widgets`
  macro at `DefaultScreenMacros.xml.ftl:18-22`), a form-list is an element of `…Entry` elements
  whose attributes are the displayed fields, encoded with `encodeForXmlAttribute`
  (`ScreenXml.js:28-31`, `:59-90`).
- **The plain mode**: only the text of the screen's own `render-mode`/`text` elements is written, and
  the subscreen and section calls are kept so text inside a section still shows
  (`ScreenPlain.js:37-58` against `DefaultScreenMacros.plain.ftl:16-29`).
- **`lookup`**: Java's `lookup` macro is commented out (`DefaultScreenMacros.html.ftl:2164-2179`),
  so Java writes the "not yet implemented" placeholder for it; the port writes a text line
  (`ScreenHtml.js:2037`) and records the difference in the header comment of the file
  (`ScreenHtml.js:34`).
- **The `date-time` of the export modes**: Java's csv and xml macros format the value only when the
  widget has a `format` (`DefaultScreenMacros.csv.ftl:176-181`), while the port always formats it
  with the pattern or the type's default (`ScreenCsv.js:154-155`, `ScreenXml.js:138-139`); the
  difference is named in the header comments of `ScreenCsv.js:18` and `ScreenXml.js:19`.
- **The link's background attributes and the dialog's tooltip**: the port's link form takes
  `background-submit`, `background-reload-id`, `background-hide-id` and `background-message`
  (`ScreenHtml.js:604-610`) and the container-dialog's button takes the `tooltip`
  (`ScreenHtml.js:672-679`); Java's macros have neither — they are the recorded additions in
  [Parity-Diffs, Framework gaps from the IronStone screens (2026-10-19)](../Parity-Diffs.md#framework-gaps-from-the-ironstone-screens-2026-10-19).
- **The wizard and stack subscreens-panel**: Java writes "LATER … not yet supported"
  (`DefaultScreenMacros.html.ftl:125-128`); the port writes the step list and the Back/Next links
  (`ScreenHtml.js:782-804`), which is the recorded difference in
  [Parity-Diffs, Layout panels and the calendar and contact tools (2026-10-19)](../Parity-Diffs.md#layout-panels-and-the-calendar-and-contact-tools-2026-10-19).

#### Out of scope

- `DefaultScreenMacros.vuet.ftl` and the vuet/js render modes and `/vapps`: excluded on purpose
  (Parity-Mapping, "Excluded on purpose"; the Macro libraries table lists
  `DefaultScreenMacros.vuet.ftl` as excluded).
- `DefaultScreenMacros.any.ftl` as a separate module: Parity-Mapping's Macro libraries table lists
  it as excluded; its macros are in `ScreenSupport.js` (`anyMacros`) and were compared (above).
- `TestScreenHtmlMacros.ftl`: excluded on purpose (Parity-Mapping, Macro libraries table).
- FreeMarker itself, and the `?interpret` of an inline text template: the fancy dialect replaces it
  (Parity-Mapping, "Script, XML actions, and templates"; README redesign 4).
- `ScreenHtmlMacros.ftl`: an example of overriding the `container` macro, not a framework feature;
  the port's `macro-template-location` override covers the same need
  (`screen/generate.rs:445-460`).
- The `xsl-fo` mode and `moqui-fop`: the print pipeline is `html-print` (README redesign 5), and
  `ScreenHtmlPrint.js` is not in this phase's scope.
- The qvt and qvt-static modes (`ScreenQvt.js`, `ScreenQvtStatic.js`): other phases.

#### Open questions

- Should an element that no mode names still fail the render (P3-01)? The port's rule catches a
  typo in a screen, which is worth something, but the XSD allows an extension element and the cost
  is a page that does not render. A fallback macro that writes the Java placeholder (or nothing in
  the text modes) would keep the page; the alternative is to make the rule a lint
  (`moqui lint --screens`) instead of a render error.
- Should the csv, text and xml modes keep evaluating `hide` as a condition (P3-04), or match Java's
  literal compare? The port's rule agrees with the port's own html mode and with the attribute's
  documentation; Java's three export modes disagree with Java's own html mode. Whichever is chosen,
  it should be the same in all four modes.
- Should the html mode offer the page-layout (and file name) choice for the PDF export (P3-02)? The
  print mode already reads `layoutMaster`, so only the dialog is missing.

## Phase 4 — The qvt render mode

Compared `runtime/base-component/framework/template/screen/ScreenQvt.js` (2579 lines) against
`../moqui/runtime/template/screen-macro/DefaultScreenMacros.qvt.ftl` (2253 lines), with
`runtime/base-component/framework/template/screen/ScreenSupport.js` and the qvt client library
(`runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.js`) for the shared macros and the
`m-*` component props.

Worked 2026-10-19: 9 findings (0 High, 3 Med, 6 Low) — P4-01 was found here and removed as invalid on
verification (the port's `A()` helper escapes every attribute value, which is the `?xml` of the FreeMarker
macro, so the qvt `editable` widget matches Java; the `#### P4-01` section is gone and the number is a gap).
All 80 FreeMarker macros of the qvt FTL have a port equivalent, so the component set
is complete; 1 High, 3 Med and 6 Low findings, mostly in the form-list nav row (saved finds, PDF export)
and in the `editable` widget's `encode` handling.

| ID | JS location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P4-02 | `ScreenQvt.js:1241-1278` | `DefaultScreenMacros.qvt.ftl:716-741` | The qvt form-list has no "Select Find" drop-down and no "Clear Current Find" link, so a user who applied a saved find cannot clear it from the list | Med | No |
| P4-03 | `ScreenQvt.js:1204-1216` | `DefaultScreenMacros.qvt.ftl:977-1016` | `show-pdf-button` writes a "Print" item that opens the `html-print` view in a new tab; Java wrote a PDF dialog that posted to `/fop/<path>` and let the user pick the page layout, so no PDF file and no page-layout choice | Med | No |
| P4-04 | `ScreenQvt.js:2463` | `DefaultScreenMacros.qvt.ftl:411,425` | A `button-menu` item is wrapped in `v-close-popup`, which closes an enclosing `m-container-dialog` as well as the menu; Java writes the item without it, on purpose | Med | No |
| P4-05 | `ScreenQvt.js:1241-1278` | `DefaultScreenMacros.qvt.ftl:840-856` | The Saved Finds dialog has no schedule action (the mode and cron drop-downs and the Schedule button Java writes for the active find) | Low | No |
| P4-06 | `ScreenQvt.js:1393-1620` | `DefaultScreenMacros.qvt.ftl:1471-1473` | A form-list's `focus-field` is ignored; Java writes a script that focuses that field of the first row | Low | No |
| P4-07 | `ScreenQvt.js:1548` | `DefaultScreenMacros.qvt.ftl:1349` | A row-selection check box calls `clickCheckbox` with an empty event, so the shift-click range selection of Java is lost | Low | No |
| P4-08 | `ScreenQvt.js:724` | `DefaultScreenMacros.qvt.ftl:1814-1817` | A `display` with a `dynamic-transition` outside a form writes an empty hidden input value; Java writes the value the transition returns for the field | Low | No |
| P4-09 | `ScreenQvt.js:1633` | `DefaultScreenMacros.qvt.ftl:2157,2238-2248` | The find-options filter summary leaves out the value of a `display` or `display-entity` widget, which Java's `widgetTextValue` includes | Low | No |
| P4-10 | `ScreenQvt.js:2412-2434` | `DefaultScreenMacros.qvt.ftl:125-145` | A `container-panel` writes no id on its header, left, center, right and footer parts; Java writes `<panelId>-header`, `-left`, `-center`, `-right`, `-footer` | Low | No |


#### P4-02 — No way to clear the current saved find

Java's list nav row has a "Select Find" drop-down (`DefaultScreenMacros.qvt.ftl:716-741`) that lists the
user's finds and, at the top, a "Clear Current Find" link that goes to the screen with
`formListFindId=_clear`.

The port keeps the finds in their own "Saved Finds" dialog (`ScreenQvt.js:1192-1197`, body at
`:1241-1278`) and lists them there, each with a link that applies it
(`base + "?formListFindId=" + id`). There is no `_clear` link anywhere in the qvt modules (nor in
`ScreenQvtStatic.js`). The server side does support it (`crates/moqui-core/src/screen/saved_finds.rs:194`,
the `_clear` case), so the port can clear a find; the qvt render mode just never offers it.

What a user sees: a user who applies a saved find cannot go back to the plain search. The "Reset to
Default" button of the find-options summary (`ScreenQvt.js:1233`) posts only the hidden parameters, so
the `formListFindId` in the URL stays and the server puts the find's parameters back on the next render
(`saved_finds.rs:179-210`); the list keeps the find's filter no matter how often the user resets. In
Java the same list has "Clear Current Find" in the nav row and the find is gone in one click.

The fix is to add the `formListFindId=_clear` link to the Saved Finds dialog (a `m-link` to
`path + "?formListFindId=_clear"`), and optionally the "Select Find" drop-down of Java in the nav row.

#### P4-03 — The PDF export of a form-list is an HTML page, not a PDF

Java's `show-pdf-button` writes a "Generate PDF" dialog (`DefaultScreenMacros.qvt.ftl:977-1016`): a plain
form that posts to `<webappRoot>/fop<screenPath>` with a `layoutMaster` page-layout select (letter,
legal, tabloid, A4, A3, portrait and landscape) and a file name, so the browser receives a PDF file.

The port writes one "Print" item in the export menu
(`ScreenQvt.js:1204`, `:1213-1216`): `renderMode=html-print&pageNoLimit=true` opened with
`target="_blank"`. That is the HTML print view; the user gets a print-preview page and has to print it
from the browser, and the page layout cannot be chosen. The port does have a PDF route
(`/fop/<path>` renders the screen as PDF, `crates/moqui-core/src/screen/mod.rs`) and a PDF renderer
(`screen/pdf.rs`), so the PDF itself is available — the qvt export menu just does not use it.

What a user sees: Java's "PDF" button downloads a PDF of the list in the chosen page size; the port's
"Print" item opens an HTML page in a new tab with no page-size choice. `layoutMaster` is read by the
print renderer (`ScreenHtmlPrint.js:33`), so the layout would still work if the export item pointed at
the PDF route with a `layoutMaster` parameter.

The fix is to give the export menu a PDF item that goes to the `/fop/<path>` route (with
`pageNoLimit=true` and a `layoutMaster`), and to keep the Java page-layout dialog for the choice.

#### P4-04 — A button-menu item closes the dialog it is in

Java writes the items of a `button-menu` as plain `<q-item><q-item-section>` and says why
(`DefaultScreenMacros.qvt.ftl:411`): "do not use auto-close or v-close-popup because it closes dialogs as
well as the menu!".

The port wraps every item in `v-close-popup` (`ScreenQvt.js:2463`:
`'<q-item clickable v-close-popup><q-item-section>'`). A `button-menu` inside a `container-dialog` (a
common screen pattern) therefore closes the dialog when an item is chosen, and the screen the user asked
for is thrown away. In Java the dialog stays open and the item's transition renders behind it.

The fix is to drop `v-close-popup` from the `button-menu` items (the export and page-size menus in the
nav row are not inside a dialog and may keep it).

#### P4-05 — The Saved Finds dialog cannot schedule a find

Java's Saved Finds dialog writes, for the active find, the find's description, the "My Default" mark and
a schedule form (`DefaultScreenMacros.qvt.ftl:840-856`): a mode drop-down (XLSX, CSV, PDF), a cron
drop-down (Monday-Friday, Every Day, Monday Only, Monthly) and a "Schedule" button that posts
`ScheduleFind` to `formSaveFind`.

The port's dialog (`ScreenQvt.js:1241-1278`) lists the finds with "Update to Current", "Make Default",
"Clear Default" and "Delete" and offers "Save New Find", but there is no schedule action and no
active-find block. The port does have the server side (`crates/moqui-core/src/screen/saved_finds.rs:547`,
`ScheduleFind`) and the screens (`MrSimpleScreens/screen/MyAccount/User/ScheduledFinds.xml`), so a user
can still schedule a find, but only by leaving the list and using the My Account screen; in Java the
schedule is one click away from the list.

The fix is to write the active-find block with the two drop-downs and the Schedule button in the port's
Saved Finds dialog.

#### P4-06 — A form-list's focus-field is ignored

Java's form-list ends with a script that focuses the field named by `focus-field` in the first row
(`DefaultScreenMacros.qvt.ftl:1471-1473`, adding the `default-focus` class the dialog also looks for,
`WebrootVue.qvt.js:1166`).

The port's `formList` (`ScreenQvt.js:1393-1620`) never reads `focus-field`; only the form-single does
(`ScreenQvt.js:1078`, the `focus-field` prop of `m-form`). A list screen that names a `focus-field` opens
with the focus on the page's first control instead of on that field.

The fix is to pass `focus-field` on the first row's `m-form` (the client `m-form` already has the prop)
or to write the same `m-script` that Java writes.

#### P4-07 — Shift-click no longer selects a range of rows

Java's row check box is `v-model="formProps.checkboxStates[i]"` with
`@click="formProps.clickCheckbox($event, i)"` (`DefaultScreenMacros.qvt.ftl:1349`); the DOM event is what
carries `shiftKey`, and `clickCheckbox` uses it to select every row between the last one and this one
(`WebrootVue.qvt.js:1566-1574`).

The port's check box is `:model-value` with
`@update:model-value="(value, event) => { formProps.clickCheckbox(event || {}, i); ... }"`
(`ScreenQvt.js:1548`). A `update:model-value` event carries only the value, so `event` is always
`undefined` and `clickCheckbox` gets `{}` — `shiftKey` is never read. The single-row check still works
(the port sets `checkboxStates[i] = value` itself), but the shift-click range is gone.

The fix is to bind the click (`@click`) as well as the model change, so the DOM event reaches
`clickCheckbox`.

#### P4-08 — A dynamic display posts an empty value

Java's `display` macro, when it writes the hidden input for the field (`also-hidden`, and the widget is
not inside a form), takes the value from the dynamic transition:
`<#assign hiddenValue><@widgetTextValue .node true "value"/></#assign>`
(`DefaultScreenMacros.qvt.ftl:1814-1817`), so the form posts the id the transition returned for the
field's value.

The port writes `value: dyn ? "" : sri.getFieldValuePlainString(field, "")` (`ScreenQvt.js:724`): for a
`display` with a `dynamic-transition` the hidden input is empty, and the form posts nothing for that
field. The visible text is the same (the `m-display` component asks the transition for the label), so the
divergence is only in what the form posts.

The fix is to ask the transition for its value the way Java does (`sri.getFieldTransitionValue` with the
`value` field), which the port already calls for the drop-down.

#### P4-09 — The filter summary leaves out the display widgets

Java's `widgetTextValue` has a `noDisplay` list of the widgets that show nothing
(`DefaultScreenMacros.qvt.ftl:2157`: hidden, ignored, password, reset, submit, text-area, link, label) and
then a branch for `display` (the field value, or the dynamic transition's value) and for `display-entity`
(the entity value). The find-options summary (`curFindSummary`, `DefaultScreenMacros.qvt.ftl:699-707`) is
built from it, so a header field with a `display` shows its value in the summary line.

The port's `valueText` (`ScreenQvt.js:1633`) adds `display` and `display-entity` to the list of widgets
that show nothing, so the summary line of the find options (and the "Reset to Default" summary,
`ScreenQvt.js:1227-1231`) leaves out those values. A user with a date range or a looked-up value in the
find options sees a shorter summary than in Java.

The fix is to give `valueText` the `display` and `display-entity` branches Java has.

#### P4-10 — The container-panel parts have no ids

Java's `container-panel` writes an id on every part: `<panelId>-header`, `<panelId>-left`,
`<panelId>-center`, `<panelId>-right`, `<panelId>-footer` (`DefaultScreenMacros.qvt.ftl:125-145`).

The port writes only the id of the outer element (`ScreenQvt.js:2412-2434`); the header, left, center,
right and footer divs have their class but no id. A screen that links to a panel part, or a script that
finds `#<panelId>-center`, finds nothing. The `m-container-dialog` registers itself with the root by its
id (`WebrootVue.qvt.js:1191`), so a `background-hide-id` naming a panel part would not work either.

The fix is to add the same suffixed ids to the five parts.

#### Phase 4 checked and found equivalent

- **The component set.** Every one of the 80 `<#macro ` of `DefaultScreenMacros.qvt.ftl` has an
  equivalent in the port: the layout macros (`screen`, `widgets`, `fail-widgets`, `section`,
  `section-iterate`, `section-include`, `include-screen`), the containers (`container`, `container-box`
  and its `box-header`/`box-toolbar`/`box-body`/`box-body-nopad`, `container-row`/`row-col`,
  `container-panel` and its five parts, `container-dialog`, `dynamic-container`, `dynamic-dialog`), the
  standalone widgets (`link` with its `linkFormForm`/`linkFormLink` pair, `image`, `label`, `editable`,
  `button-menu`, `tree`, `parameter`), the forms (`form-single`, `field-ref`, `fields-not-referenced`,
  `field-row`, `field-row-big`, `field-group`, `field-accordion`, `field-col-row`), the list
  (`form-list`, `paginationHeader`, `formListSelectedRowCard`, `formListHeaderField`, the four
  `formListSub*` macros), the field widgets (`check`, `date-find`, `date-period`, `date-time`, `display`,
  `display-entity`, `drop-down`, `file`, `hidden`, `ignored`, `password`, `radio`, `range-find`, `reset`,
  `submit`, `text-area`, `text-line`, `text-find`, `lookup`) and the helpers (`nodeId`, `fieldName`,
  `fieldId`, `fieldIdByName`, `fieldTitle`, `widgetTextValue`, `getQuasarColor`, `getMomentDateFormat`).
  The port has no widget Java cannot draw; the elements Java's qvt FTL does not define at all
  (`dashboard`, `context`, `number-card`, `chatter`, `record-files`, `html-view`, `markdown-view`,
  `status-bar`, `container-split`) are the port's own additions and are in
  [Parity-Diffs, Framework gaps from the IronStone screens].
- **The shared macros.** `include-screen`, `render-mode`, `text`, `render-dynamic-form`,
  `form-list-user-columns`, `section`, `section-iterate` and `subscreens-*` come from the port's
  `ScreenSupport.js` `anyMacros` and `ScreenQvt.js`'s own table, and match the FTL's
  `DefaultScreenMacros.any.ftl` and the qvt FTL's `subscreens-*` macros (the wizard and stack panel types
  are the recorded redesign in Parity-Diffs).
- **The `m-*` component props.** Each component the macros write declares the props the macro sets:
  `m-form` (`action`, `method`, `read-only`, `focus-field`, `body-parameter-names`, `submit-message`,
  `submit-reload-id`, `submit-hide-id`, `exclude-empty-fields`, `parent-checkbox-set`, `fields-initial`,
  `validation-errors-initial`, `WebrootVue.qvt.js:1644`), `m-form-link`, `m-form-paginate`
  (`paginate`, `:1885`), `m-form-go-page` (`id-val`, `max-index`), `m-form-column-config`
  (`columns-initial`, `find-parameters`, `form-location`), `m-text-line` (`type`, `prefix`, `size`,
  `maxlength`, `mask`, `fill-mask`, `reverse-fill-mask`, `rules`, `default-url`, `default-parameters`,
  `depends-on`, `:2511`), `m-drop-down` (`options`, `options-url`, `value-field`, `label-field`,
  `server-search`, `server-delay`, `server-min-length`, `multiple`, `allow-empty`, `combo`,
  `required-manual-select`, `submit-on-select`, `show-not` slot, `:2374`), `m-date-time` (`type`,
  `format`, `auto-year`, `minute-step`, `rules`), `m-date-period` (`allow-empty`, `from-thru-type`,
  `fields`), `m-display` (`display`, `value-url`, `depends-on`, `label-wrapper`), `m-check`/`m-radio`
  (`options`, `no-wrapper`, `true-value`, `false-value`, `single-label`, `toggle`), `m-file`, `m-text-area`
  (`editor-type`, `rows`, `cols`, `readonly`, `autogrow`, `mention-url`), `m-container-box` (`type`,
  `title`, `initial-open`; the port omits the attribute when the box is not closed and the component
  default is `true`, which is what Java's `:initial-open="true"` means), `m-container-dialog` (`color`,
  `width`, `button-text`, `button-icon`, `button-class`, `title`), `m-editable` (`label-type`,
  `label-value`, `url`, `url-parameters`, `parameter-name`, `widget-type`, `load-url`, `load-parameters`),
  `m-tree-top` (`items`, `open-path`, `parameters`), `m-markdown`, `m-html-view`, `m-status-bar`,
  `m-number-card`, `m-list-views` and `m-subscreens-*`.
- **The field binding.** Java writes `v-model="${fieldsJsName}.${name}"` and `:fields` for a widget in a
  form and a plain `value` outside one; the port's `bind`/`bindOnly`
  (`ScreenQvt.js:242-244`) do the same, and the value the model holds is the one the `fields-initial`
  of the form seeds.
- **The `fields-initial` values.** The port's `addValue` (`ScreenQvt.js:293-360`) names the same values
  as Java's `ScreenRenderImpl.addFormFieldValue`: the plain value, the `_display` value of a display and
  a display-entity, the `_op`/`_not` of a drop-down and a text-find, the `_from`/`_thru` of a date-find
  and a range-find, the `_poffset`/`_period`/`_pdate` of a date-period, the `_isMulti` flag of a multi
  list, `moquiFormName`, `lastUpdatedStamp` and the pass-through parameters. (The data these carry is
  Phase 5's subject.)
- **The list structure.** The row forms, the multi form with its final submit row, the first, second and
  last rows with their own forms and maps, the sub-rows of an aggregated list, the row-selection
  check boxes with their `checkbox-parameter`, `checkbox-list-mode` and `checkbox-values`, the header
  form and the header dialog, the column layout (`columns` with a `type`, `form-list-column`, the
  per-field columns) and the `columns type` choice by `_uiType` all follow
  `DefaultScreenMacros.qvt.ftl:1113-1478` and Java's `ScreenForm` column rules.
- **The nav row.** The find-options dialog with the order-by drop-down and the "Clear Parameters" button,
  the column-config dialog with the "Do Not Display" column, the saved-finds dialog, the page-size menu
  with the sizes 10/20/50/100/200/500, the "go to page" control, the `m-form-paginate` control, the
  "No results found" and "Find Options required to view results" messages and the reset-to-default
  summary all follow `DefaultScreenMacros.qvt.ftl:668-1078`. The nav row is a `div` above the table and
  not a `tr` (the port's own documented choice in the module comment).
- **The export and text dialogs.** The CSV and XLS items (`renderMode=csv|xlsx`, `pageNoLimit=true`,
  `lastStandalone=true`, `saveFilename`) and the fixed-width text dialog (the line characters, page
  lines, line wrap and file name fields) follow `DefaultScreenMacros.qvt.ftl:924-976`; the XLS item is
  written only when `ec.screen.isRenderModeValid("xlsx")` in both.
- **The links and buttons.** The link text (an entity value, a `text-map` expansion, an image, an icon
  from the theme), the `link-type` variants (anchor, anchor-button, hidden-form-link), the URL forms (a
  screen path, a transition path, a full URL), `url-noparam`, `target-window`, `dynamic-load-id`,
  `pass-through-parameters`, `badge`, `tooltip`, the confirmation (a `data-confirm` attribute the shell
  answers instead of Java's inline `onclick`), the disabled link and the `m-form`/`m-form-link` choice
  follow `DefaultScreenMacros.qvt.ftl:220-345`. A link in a field is written to the after-screen writer
  (`sri.appendToAfterScreenWriter`) in both.
- **The dates and the find widgets.** The default date patterns (`HH:mm`, `yyyy-MM-dd`,
  `yyyy-MM-dd HH:mm`), the Java-to-moment pattern change (`a`→`A`, `d`→`D`, `y`→`Y`), the from/thru
  pair of a date-find, the period widget with its offset and period fields, the range-find pair, the
  text-find with its operator, not and ignore-case options, and the drop-down with its dynamic options,
  its `value-field`/`label-field`, its `depends-on` map and its current-value label follow
  `DefaultScreenMacros.qvt.ftl:1656-1701, 1859-1920, 1975-1994, 2114-2151`.
- **The containers.** The `container` with its type and style, the `container-box` with its header,
  toolbar, body and no-pad body, the `container-row` with its column sizes (the `col-md-12`/`col-sm-12`/
  `col-xs-12` fallbacks), the `container-panel` with its left/right widths and its center class, the
  `container-dialog` with its condition, its title, its button and the `_openDialog` flag, the
  `dynamic-container` with its `_dynamic_container_id`, the `dynamic-dialog` and its disabled button, and
  the `tree` with its items URL and parameters follow `DefaultScreenMacros.qvt.ftl:88-215`.
- **The `m-editable` extras.** The port writes the widget when the text is empty and
  `display-if-empty="true"` is set (with the "Click to edit" placeholder), which Java's qvt macro does not
  do at all, and it supports the `select` and `date` widget types. That is a superset of Java and is
  not a finding.
- **The `text-line` with `ac-transition`.** Java writes an error message ("text-line with @ac-transition
  is not supported", `DefaultScreenMacros.qvt.ftl:2064`); the port writes a server-search drop-down over
  the same transition (`ScreenQvt.js:434-445`). The port's is a superset, recorded in the module comment
  of `ScreenQvt.js`; see the open question below.

#### Out of scope

- **The `vuet` and `js` render modes and `/vapps`**: not ported (Parity-Mapping, "Excluded on
  purpose"). The `DefaultScreenMacros.vuet.ftl` file has no port counterpart.
- **`DefaultScreenMacros.any.ftl` itself**: the file is the source of the `include-screen`, `render-mode`
  and `text` macros, which the port shares through `ScreenSupport.js` `anyMacros`; the inline
  `?interpret` template of the FTL is the port's fancy template dialect (`script/template.rs`, README
  redesign 4), not a qvt question.
- **FreeMarker**: the engine is excluded (Parity-Mapping, "Excluded on purpose"); only the macros the
  qvt FTL defines are compared here.
- **The data the qvt template gets**: the `data` prop, the actions and the server-static variant are
  Phase 5's subject; the `fields-initial` values and the actions are named here only as far as a widget
  writes them.
- **The dropped client libraries**: CKEditor (`m-ck-editor`) and SimpleMDE (`m-simple-mde`) of Java's
  `text-area` with `editor-type="html"` or `"md"` are replaced by `QEditor` and a markdown preview in
  `m-text-area` (Parity-Mapping, "the client libraries the port drops or replaces"), and the rich
  editors (`rich`, `rich-light`) and the `m-rich-text` mention search are the port's own
  (Parity-Diffs, "Rich text").
- **`m-form-list` and the server-static qvt**: the `m-form-list` branch of Java's `form-list` macro
  (`DefaultScreenMacros.qvt.ftl:1143-1196`) is the server-static variant, which is Phase 5.

#### Open questions

- The deliberate qvt differences listed in the top comment of `ScreenQvt.js` (the `v-pre` encoding, the
  `ac-transition` drop-down, the `data-confirm` confirmation, the background-message as text, the mask
  tokens, the nav row above the table, the display/link text in `fields-initial`, the list views) are
  documented in the module but are not in [Parity-Diffs](../Parity-Diffs.md). They should be added there
  so that the next review does not read them as findings.
- The `editable` rule of P4-01 is the one the html mode and the qvt mode now share, and the two Java
  macros disagree with each other (`DefaultScreenMacros.html.ftl:521` escapes on `encode="true"`,
  `DefaultScreenMacros.qvt.ftl:377` escapes unless `encode="true"`). The port owner has to pick the
  rule; the qvt macro is the one this phase compares against, and it is the one that keeps the value
  inert.
- The `m-container-dialog` open flag is written as a static attribute (`open-dialog="true"`,
  `ScreenQvt.js:951`) where Java binds it (`:openDialog="true"`). The value is the string `"true"` for a
  Boolean prop, so the dialog opens but Vue's prop type check warns. It was left out of the findings
  because the behaviour is the same; it would be tidier as `":open-dialog": "true"`.

## Phase 5 — Server-static qvt

Compared `crates/moqui-core/src/screen/statics.rs`, `crates/moqui-core/src/screen/static_ast.rs`,
`crates/moqui-core/src/screen/statics/report.rs`, `crates/moqui-core/src/screen/vue_check.rs`,
`runtime/base-component/framework/template/screen/ScreenQvtStatic.js` and
`crates/moqui-host/src/qvt_report.rs` against the server-merged qvt module
`runtime/base-component/framework/template/screen/ScreenQvt.js`, the qvt render path
(`crates/moqui-core/src/screen/render.rs`, `crates/moqui-core/src/screen/sri/`), the client host
(`runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.js`) and the Java sources
(`framework/src/main/groovy/org/moqui/impl/screen/ScreenRenderImpl.groovy`,
`ScreenForm.groovy`, `runtime/template/screen-macro/DefaultScreenMacros.qvt.ftl`). There is no Java
counterpart for the static mode: Java's `server-static` was only a `Cache-Control` hint, so the
comparison is "does the static form still deliver what the merged qvt mode delivers, and is every
gap recorded".

Worked 2026-10-19: nine findings, one of them High and fixed on 2026-10-20 (the static template's number rule calls the
global `moqui`, which the server's `prefixIdentifiers` compile rewrites to `_ctx.moqui`), four
features the merged mode has and the static module drops without a note (the form's
`<hidden-parameters>`, the `validate-service` JS rules, `pass-through-parameters`, the disabled
state of a form the user may not use), one report gap (a "clean" verdict that never ran the full
check), and three smaller ones (the screen history, `all-checked`, `hideNav`). P5-01 fixed 2026-10-20: the rule calls `$mq`, the name the static
scope has and the Vue global property `app.config.globalProperties.$mq`, so one rule text works in both qvt modes.

| ID | Rust/JS location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P5-02 | `ScreenQvtStatic.js:1249` (row form), `:1095` (form-single) | `ScreenQvt.js:383,396,407`; Java `ScreenRenderImpl.groovy:1716,2134,2154,2176` | The merged mode puts the form's `<hidden-parameters>` into the fields-initial of every row form, of the header find form and of a single form; the static module builds its fields-initial without them, so no row action of a static form-list posts them | Med | No |
| P5-03 | `ScreenQvtStatic.js:367-373` | `ScreenQvt.js:253-261`; Java `ScreenForm.groovy:1557` | The merged mode adds `sri.getFieldValidationJsRules(sub)` (the `validate-service` parameter's browser checks, and the whole-number check of a numeric `validate-entity` field) to the `:rules` of a field; the static module keeps only the `number`/`digits` classes it reads from `getFieldValidation` | Med | No |
| P5-04 | `ScreenQvtStatic.js:283-287` (the state), `:533,653` | `ScreenQvt.js:180,449,528,1071,1419`; Java `ScreenRenderImpl.groovy:2441` | The merged mode sets `st.disabled` from the transition's `disableLink` and disables every widget and the submit button of a form or list the user may not use; the static module has no `disabled` state, so the same form is rendered editable and the user only learns of the refusal from the server | Med | Unclear |
| P5-05 | `ScreenQvtStatic.js:1095`, `:918` (a link) | `ScreenQvt.js:388,796`; Java `ScreenRenderImpl.groovy:1696` | The merged mode seeds a form with `pass-through-parameters="true"` from the screen URL's pass-through map, and adds them to a link's URL; the static module reads neither attribute, so the form and the link lose those parameters | Med | No |
| P5-06 | `crates/moqui-core/src/screen/statics/report.rs:73-75`, `:576-604`; `crates/moqui-host/src/qvt_report.rs:112-123` | `plans/archive/QvtServerStatic.md:140` | `StaticReport::is_clean()` ignores `full_check`, and `moqui lint --screens --qvt-static` runs the static half only (`full = false`), so the report prints "verdict clean" for a screen whose widget expressions were never evaluated against the wrapped `ec` — the clean the plan asks for before a screen is marked | Med | No |
| P5-07 | `crates/moqui-core/src/screen/render.rs:442-444` (against `:548-577`) | Java `ScreenRenderImpl.groovy:982` (`saveScreenHistory`) | The static path returns from the render before `record_history`, so a server-static screen is never pushed onto the user's screen history; the merged qvt screen is | Med | No |
| P5-08 | `ScreenQvtStatic.js:428-429` | `ScreenQvt.js:311,680-684`; Java `ScreenRenderImpl.groovy:1909` | A `check` widget with `all-checked="true"` starts with every option checked in the merged mode; the static module seeds the field with the field's own value, so the group starts empty | Med | No |
| P5-09 | `ScreenQvtStatic.js:1042-1064`, `:1513-1514` | `ScreenQvt.js:997,1012,1168,1176`; Java `DefaultScreenMacros.qvt.ftl:32,713` | The merged mode reads the `hideNav` context flag and writes no subscreen tabs and no form-list nav row when it is set; the static module reads it nowhere, so both appear | Low | No |

#### P5-02 — the static form-list drops the form's `<hidden-parameters>`

The merged mode reads the form's hidden parameters and puts them into the fields-initial of every
form it writes: `ScreenQvt.js:383` (`const hiddenParams = (n) => sri.getFormHiddenParameters(n)`),
used at `:396` for the header find form and `:407` for each row form. Java does the same
(`ScreenRenderImpl.groovy:1716, 2134, 2154` call `getFormHiddenParameters(formNode)`, defined at
`:2176`): the `<hidden-parameters>` element's `<parameter>` elements are read from the context
(`from`, else the parameter's own name) and posted with the form.

The static module never calls `sri.getFormHiddenParameters`. `ScreenQvtStatic.js:1249` builds the
row form's starting values as

```js
const initial = initialOf(fields, "r", [["moquiFormName", sq(name)], ["pageIndex", "$mq.str(" + scopeRef("pageIndex") + "||0)"]]);
```

and `:1095` builds the form-single's as `initialOf(fields, "r", [["moquiFormName", sq(at(n, "name"))]])`.
No hidden parameter is in either, and the form-list's own `<hidden-parameters>` element is never
read at all.

The result is that in a server-static screen every row action of a form-list posts without the
parameters the form declared. `<hidden-parameters><parameter name="partyId"/></hidden-parameters>`
(`runtime/component/MrSimpleScreens/screen/MyAccount/User/Messages/FindMessage.xml:74`) is the
common shape: the merged row action receives `partyId`; the static one does not, so the transition
either fails on a missing required parameter or acts without the scope it was given. The same
applies to the list's own find form and to a `form-single` with a hidden parameter.

Nothing records the gap. The compatibility report scans the definition for `always-actions` and
`pre-actions` side effects, for a `section-iterate` over a form-list's list and for a label outside
the list that reads the page keys (`report.rs:423-490`); it has no check for hidden parameters, and
the static module writes no `note()` for them, so a report for such a screen is "clean".

The fix is to read the hidden parameters on the server when the package is made (the values are
context values, so they belong in the package) and to fold them into the template's
`:fields-initial`, or to have the client post them with the row form.

#### P5-03 — the static mode drops the `validate-service` and `validate-entity` browser rules

`ScreenQvt.js:253-261` builds a field's `:rules` from three sources: the widget's own `rules`
attribute, and `sri.getFieldValidationJsRules(sub)` — the browser checks of the field's
`validate-service` parameter, or the whole-number check of a numeric `validate-entity` field
(`crates/moqui-core/src/screen/sri/fields.rs:412-451`, Java `ScreenForm.groovy:1557`). Each rule
becomes `value => (<expr>) || "<message>"`.

The static module's `ruleList` (`ScreenQvtStatic.js:367-373`) keeps only the widget's own `rules`
attribute and adds two checks of its own, derived from the *classes* `getFieldValidation` returns
(`number`, `digits`). Everything `getFieldValidationJsRules` knows is gone: a `validate-service`
with `text-email`, `text-url`, `text-letters`, `text-digits` or a `matches` pattern, and the
"whole number" check of an integer entity field.

The user sees a field that accepts what it should refuse. The server still validates and still
refuses the save, so nothing is written that should not be — but the error arrives as a server
message after the submit instead of as a field error while typing, which is what the merged mode
does. It was also the second half of P5-01: the one rule the static module did keep for a number
field was the broken one (fixed on 2026-10-20, the rule now calls the scope's `$mq`).

The fix is to call `sri.getFieldValidationJsRules` in the static module as well and write its rules
into the template — the same way the merged mode does — and to make sure the expressions they use
name only scope helpers (the rule text the server writes does: `$mq.isStringNumber` and the rest).

#### P5-04 — a form or list the user may not use is rendered enabled in the static mode

The merged module keeps a `disabled` flag in its per-unit state (`ScreenQvt.js:180`). `formOpen`
sets it from the transition's `disableLink` (`:1071`) and `formList` from the list's own transition
(`:1419`); every widget then writes `disable: st.disabled || null` (`:449` for a text line, `:487`
for a password, `:637` for a drop-down, `:687` for a check, and the submit button at `:528`). A user
who may not use the transition sees a greyed-out form.

The static module's state (`ScreenQvtStatic.js:283-287`) has no `disabled` key, and the only
`disable` it ever writes is the widget's own `disabled` attribute (`:533`, `:653`). So the same form
in a server-static screen is fully editable: the user fills it in, presses Save, and the server
refuses the transition. The refusal is the same one the merged mode avoids by greying the form out.

`plans/archive/QvtServerStatic.md:69` records the deliberate part — "This pass does not bake
`disableLink` into the template, and it does not add a permission map to the package. A link the
user may not use is still rendered. The server rejects the request." — but it speaks of *links*. The
state of the *widgets* of a disabled form is a second thing, and it is not recorded anywhere, which
is why this is filed as "Unclear" rather than as an accepted difference.

The fix is either to carry the flag into the package (the server knows `disableLink` when it makes
the package) and bind `:disable` over the scope, or to record the form-wide disabled state in the
plan next to the link case.

#### P5-05 — `pass-through-parameters` is dropped from forms and links

`ScreenQvt.js:388` seeds a form-single with the screen URL's pass-through parameter map when the
form says `pass-through-parameters="true"`, and `:796` adds the same map to a link's URL. Java does
the same for the form (`ScreenRenderImpl.groovy:1696`).

The static module reads neither attribute. `ScreenQvtStatic.js:1095` builds the form-single's
fields-initial with only the form's own name, and the link's parameters (`:918`, `allParams`) come
from the target's parameters, the `parameter-map` and the `<parameter>` elements — the screen's
pass-through parameters are not among them.

A screen whose form passes parameters through (the pattern is common in `runtime/component/MrSimpleScreens`,
for example `SimpleScreens/Accounting/TimePeriod/FindTimePeriod.xml:134` and `:142`) loses them in
the static mode: the transition is reached without the parameter the screen was carrying, so it
either fails on a missing required parameter or works on the wrong scope. Nothing in the report
notices.

#### P5-06 — the report's "clean" verdict does not mean the plan's "clean"

The plan splits the check in two (plans/archive/QvtServerStatic.md:140): "**Report.** `moqui lint
--screens --qvt-static [report-file]` runs the static half (eligibility, the `oxc_parser` walk of
each written expression, the macros' notes, the `always-actions` and `pre-actions` scan) over every
screen file with an entity model and no database. `screen::statics::static_report(..., full = true)`
adds the render in `html` with the wrapped `ec`, in an execution context that supplies the user and
data." And the decision rule above it: "A clean report is necessary before a screen is marked
`server-static="qvt"`".

The code does not keep that apart. `StaticReport::is_clean()` (`report.rs:73-75`) is
`self.eligible && self.violations.is_empty()` and says nothing about `full_check`, which the report
prints on its own line (`report.rs:99`) but does not weigh. `moqui lint --screens --qvt-static`
calls `static_reports_for_files` (`qvt_report.rs:122`), which calls `static_report(..., false)`
(`report.rs:640`) — the static half only. `render_reports` (`qvt_report.rs:126-153`) then counts
such a screen as *clean* and prints "verdict clean".

So the tool the owner is told to run before marking a screen prints exactly the verdict the plan
says is the gate, on a report that never evaluated a single widget expression. The screen's
`ec.entity.find` in a label is caught by the AST walk, but a widget expression that only fails at
run time (P5-01's `_ctx.moqui` was a compile-clean, run-time-broken case) is not, because the full
check is the half that runs the expressions.

The fix is to make `is_clean()` require `full_check` (and have the lint command say so in its
summary line, e.g. "clean (static half only)"), or to run the full check in the lint command when
a database is available. The port's own test `a_screen_with_nothing_for_the_browser_to_refuse_has_a_clean_report`
(`crates/moqui-core/src/screen/tests/statics.rs:497-508`) uses `full = false` and asserts
`report.is_clean()`, so the current behaviour is what the tests expect — the test is the place the
decision has to change.

#### P5-07 — a server-static screen is not recorded in the user's screen history

`render.rs:441-444` serves the static template and returns:

```rust
if state.render_mode == "qvt" && super::statics::serve_template(&state, job.request, job.path)? {
    return Ok(());
}
```

`record_history` (`:548-577`, Java `ScreenRenderImpl.groovy:982`) is called further down, after the
render, so the early return skips it. `record_history` is what pushes the screen onto the session's
screen history (`user::session::push_screen_history`, `crates/moqui-core/src/user/session.rs:905`),
which the "return to" after a sign-in reads (`render.rs:1389`) and which `ec.web.getScreenHistory`
exposes.

A merged qvt screen is recorded — the port counts "a qvt screen the `/qapps` shell loads, as Java
counted the vapps fetches" (`render.rs:547`). A server-static screen is not: the user sees it, but
the history has no entry, so the back path after a login and any screen list built from the history
miss it. The client keeps no history of its own (`rg screenHistory runtime/base-component/webroot/screen/webroot/js`
finds nothing), so the server's is the only one.

The fix is to call `record_history` before the early return, or to move it above the static branch.

#### P5-08 — `all-checked` on a `check` widget is dropped

`ScreenQvt.js:311` seeds a `check` field with every option key when the widget says
`all-checked="true"`; `:680-684` does the same for the widget's own model value. Java does the same
(`ScreenRenderImpl.groovy:1909`).

`ScreenQvtStatic.js:428-429` seeds the field with `$mq.cur(<the field's value>, true, <none>)` —
the field's own comma list, which is empty for a new record. A `check` group declared
`all-checked="true"` therefore starts empty in a static screen and starts with every box ticked in
the merged one, and the value posted on save differs (`runtime/component/MrSimpleScreens/screen/SimpleScreens/Shipping/PackShipment/PackSummary/PackCompleted.xml:172`
is an example of the attribute in the tree).

#### P5-09 — `hideNav` is not honored in the static mode

The merged module reads the `hideNav` context flag in four places: the subscreen menu
(`ScreenQvt.js:997`), the tab panel (`:1012`), the form-list's bottom nav (`:1168`) and its nav row
(`:1176`). Java's macros do the same (`runtime/template/screen-macro/DefaultScreenMacros.qvt.ftl:32, 713`).

The static module reads it nowhere: `subscreensTabs` (`ScreenQvtStatic.js:1061-1064`) always writes
`<m-subscreens-tabs>`, `subscreensPanel` (`:1042-1060`) always writes the tab panel, and the
form-list's `navSlot` (`:1513-1514`) always writes the find/saved-finds/export tools. `hideNav` is
a context value set by the screen's actions, so it *is* in the screen package — the template just
does not read it. A screen that sets `hideNav` gets its subscreen tabs and its list tools in the
static mode, which the screen asked to hide.

#### Phase 5 checked and found equivalent

- **The data split itself**: `GET {screen}/actions` answers the screen package and
  `GET {screen}/actions/{formName}` the form-list package, and the two are separate requests with
  `Cache-Control: private, no-store` (`statics.rs:450-461, 802-828, 844-1035`). This matches
  `plans/archive/QvtServerStatic.md:19-20, 26-51`; the screen package drops the form-list's list
  and its six page keys (`statics.rs:818-823`, `PAGE_SUFFIXES` at `:50-57`), the credentials are
  stripped at any depth (`web::strip_credential_parameters`, `:825` and `:1032`), a decimal is a
  string, and a find or a file is left out of the package (`unwrap_context`, `:464-476`). The
  form request answers the list, the page keys, the header field values, the show-total row
  (`:923-926`), the first/second/last rows (`:938-960`), the saved finds and the user's columns
  (`:994-1013`) — the same set the plan lists.
- **The template holds no request data and needs no `with`**: the unit runs with no actions
  (`build_template`, `statics.rs:311-349`, `UnitRequest { section: "" }`), the response is a script
  that registers a render function under the screen path (`wrap_script`, `:269-274`), and the
  compile uses `prefixIdentifiers: true` so the page needs no `unsafe-eval`. The port's own test
  (`crates/moqui-core/src/screen/tests/statics.rs:76-112`) checks that the template and the script
  hold none of the request's values.
- **The `always-actions` / `pre-actions` double side effect**: the form request runs the path's
  actions again, so a write, a send or a message in those blocks happens twice. This is recorded in
  the plan (`QvtServerStatic.md:49`) *and* reported by the scan (`report.rs:366-415`, kinds
  `side-effect`), and the port's test asserts it
  (`tests/statics.rs:530-531`). The form request itself does not require the screen request to have
  run first (`run_actions_transition`, `statics.rs:1193-1231`), as the plan says.
- **A `section-iterate` over a form-list's list, and a label outside the list that reads its page
  keys**: both are reported (`report.rs:453-489`, kind `form-list-key`) and the port's test asserts
  both (`tests/statics.rs:533-534`).
- **The eligibility rules**: a dynamic form, a form that extends an expression, a form that takes
  its fields from an expression, a form that says `server-static` without qvt, `allow-extra-path`, a
  `render-modes` without qvt and a qvt macro module that is not the built-in one all keep the
  screen a server merge (`structure_reasons`, `statics.rs:92-179`; the report's reasons are the
  same list, `report.rs:328-337`), as the plan's "Eligibility" paragraph says.
- **`display-entity`, `entity-options`, `list-options` and `dynamic-options` without `server-search`
  are resolved on the server** and kept in the package as `{name}_display` and
  `mqOpt_{form}_{field}_{sub}_{widget}` (`statics.rs:657-799`, `option_key` at `:659-661`), and the
  template reads exactly those keys (`ScreenQvtStatic.js:377-395`, `:973`). The port's test checks
  both the extra map of a form-single and the option list (`tests/statics.rs:442-457`).
- **The `ec` the browser has, and only that**: the client host is `ec.l10n` (localize, format,
  formatCurrency, formatCurrencyNoSymbol), `ec.user` (the session snapshot and `getPreference`),
  `ec.factory.moquiVersion` and `ec.resource.expand`/`expandNoL10n` (`WebrootVue.qvt.js:633-699`),
  and the plan's table lists the same members (`QvtServerStatic.md:81-92`). The static AST walk
  flags every other `ec` member (`static_ast.rs:33-51`), and the port's test shows an
  `ec.entity.find` caught (`tests/statics.rs:523`).
- **The `oxc_parser` walk of the expressions the static macros write**: `record_ast`
  (`report.rs:209-222`) checks each expression the macros hand over (`ScreenQvtStatic.js:131-135`,
  `sri.staticCheck`) with `check_expression` (`static_ast.rs:71-93`) and names `ec`, `sri`,
  `context`, `decimal`, `utilities` and `server-global` uses. The port's test asserts the `ec`,
  `decimal` and `sri` kinds (`tests/statics.rs:523-525`). The walk's own limit — a method chosen at
  run time is not seen — is stated in the module doc (`static_ast.rs:15-18`) and in the plan
  (`QvtServerStatic.md:106`).
- **The full check's wrapped `ec`**: `WidgetCheck` (`report.rs:310-326`) wraps the execution context
  while widget expressions run, so each use of an area the browser does not have is recorded and
  the render goes on (`report.rs:576-604`); the port's test asserts the `wrapped-ec` kind and that
  the render itself did not fail (`tests/statics.rs:526-528`).
- **The Vue check**: `vue_check.rs` runs the vendored `compiler-dom` bundle in a QuickJS runtime of
  its own, with a memory limit, a stack limit and a 20-second deadline per compile
  (`:36-70`, `:121-161`), and the server compiles the static template with the same compiler and
  the same options it uses for the precompiled client components (`:205-236`). A template the Vue
  compiler rejects is a build failure, not a broken page: `template_for` keeps the failure with the
  definition (`statics.rs:283-301`) and `serve_template` logs it and leaves the screen a server
  merge (`:397-404`).
- **`disableLink` on a link**: the static mode writes the link and lets the server refuse the
  request, which the plan records (`QvtServerStatic.md:69`). See P5-04 for the form-wide state,
  which the plan does not name.
- **The tree**: a `tree` element's JSON is answered by the same `/actions/{name}` request
  (`statics.rs:846-855`), as the plan says (`QvtServerStatic.md:148`).

#### Out of scope

- **The qvt client** (`runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.js`, the
  `m-static-form-list` component, the browser `ec`, `m-scope`): read here only as the other side of
  the data flow and of the scope the template is rendered against. The client components are the
  qvt phase's own scope.
- **The server-merged qvt module as a parity target in its own right** (`ScreenQvt.js`): it is the
  comparison side here, not the subject; its own divergences from Java are the qvt phase's.
- **The other render modes** (html, html-print, csv, text, xml, plain) and the print and spreadsheet
  modes: other phases of this review.
- **Java's `server-static`**: it was only a `Cache-Control` hint
  (`ScreenRenderImpl`/`ScreenDefinition.serverStatic`), so the static mode has no Java counterpart
  to compare against; the comparison is with the port's own merged qvt mode.
- **The `screen-extend` and DbForm cache identity**: the plan requires the cache identity to
  include the resolved definition (`QvtServerStatic.md:73`) and `identity` (`statics.rs:220-234`)
  hashes the merged screen XML, each form with its extensions and the files it was read from, which
  covers it; checked, not a finding.

#### Open questions

- **A section's XML condition is evaluated twice, by two implementations.** The server evaluates it
  when it prepares the package (`prepare_section`, `statics.rs:578-647`, through the actions
  runner) and the browser evaluates it again for the `v-if` (`ScreenQvtStatic.js:1318-1343`,
  `$mq.compare`). The two read the same value from different places — the context on the server,
  the package in the browser — and the package changes a value's *type* on the way (a timestamp
  becomes epoch-millis text, a decimal a string; see Parity-Diffs, "Time zones (2026-10-19)" and
  "Framework gaps from the IronStone screens"). A `<compare>` on a date field can therefore hold
  on the server and fail in the browser, and the branch the browser shows would be the one whose
  data was not prepared. Does the port want the section condition evaluated once (on the server,
  with the branch baked into the package) instead of twice?
- **Should the compatibility report know what the static module drops?** The report checks the
  expressions, the notes, the actions and the definition, but not the features the static
  transcription does not carry (P5-02, P5-03, P5-05, P5-08, P5-09). A "clean" report therefore
  means "the expressions are safe", not "the static mode delivers what the merged mode delivers".
  A list of the features the static module does not implement, checked by the report, would make
  the report mean what the plan says it means.
- **Should `moqui lint --screens --qvt-static` run the full check?** It needs a database and real
  data, which is why it does not today; the alternative is to have the lint command print the
  static half's verdict under a name that cannot be read as the plan's "clean" (see P5-06).

## Phase 6 — Print and spreadsheet

Compared `runtime/base-component/framework/template/screen/ScreenHtmlPrint.js`, `runtime/base-component/framework/template/screen/ScreenXlsx.js`, `crates/moqui-core/src/screen/pdf.rs`, `crates/moqui-core/src/screen/xlsx.rs`, `crates/moqui-core/src/screen/xlsx_off.rs`, `crates/moqui-host/src/pdf.rs` (with `crates/moqui-host/src/pdf/cdp.rs`) against `../moqui/runtime/template/screen-macro/DefaultScreenMacros.xsl-fo.ftl`, `../moqui/runtime/base-component/webroot/screen/includes/Header.xsl-fo.ftl`, `../moqui/runtime/component/moqui-poi/src/main/groovy/org/moqui/poi/FormListExcelRender.groovy`, `../moqui/runtime/component/moqui-poi/src/main/groovy/org/moqui/poi/ScreenWidgetRenderExcel.groovy` and `../moqui/runtime/component/moqui-fop/src/main/groovy/org/moqui/fop/HtmlRenderer.groovy` (the FOP engine and its servlet's other content types are the behavior reference only).

Worked 2026-10-19: every macro of the XSL-FO library has a print-mode equivalent, the page masters, the form-list widths and the PDF image inlining match, and the spreadsheet is close to moqui-poi; six divergences found — the xlsx cell of a conditional field, the xlsx header title, the xlsx sheet set, the print document's running title and page number, the hidden field of a printed form, and the `.xsl-fo` request's content type.

| ID | Rust/JS location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P6-01 | `runtime/base-component/framework/template/screen/ScreenXlsx.js:78-97` (`subCellCode`, `cellCode`) | `../moqui/runtime/component/moqui-poi/src/main/groovy/org/moqui/poi/FormListExcelRender.groovy:162-163` | The xlsx cell of a field with a `conditional-field` shows the conditional field's widgets when its condition holds; Java's xlsx render reads only the `default-field`'s widgets (and throws an NPE on a field that has no `default-field`) | Med | Unclear |
| P6-02 | `runtime/base-component/framework/template/screen/ScreenXlsx.js:126-148` (the container and section macros) | `../moqui/runtime/component/moqui-poi/src/main/groovy/org/moqui/poi/ScreenWidgetRenderExcel.groovy:89-120` (`renderSubNodes`) | The xlsx render makes a sheet for a form-list inside a `container-panel`, a `container-split` or a `section-iterate`; Java's widget render walks only `form-list`, `section`, `container-box` (`box-body`, `box-body-nopad`) and `container-row` (`row-col`) and skips `section-iterate` on purpose | Low | Unclear |
| P6-03 | `runtime/base-component/framework/template/screen/ScreenXlsx.js:39-49`, `ScreenSupport.js:160-164` (`titleText`) | `FormListExcelRender.groovy:124-129`, `../moqui/framework/src/main/java/org/moqui/util/StringUtilities.java:179-200` (`camelCaseToPretty`) | The xlsx header title of a field with no `title` attribute is the field name split on capitals (`item1Name` → "Item1 Name"); Java's xlsx uses `camelCaseToPretty` (`item1Name` → "Item 1 Name") | Low | Unclear |
| P6-04 | `runtime/base-component/framework/template/screen/ScreenHtmlPrint.js:92-96` (the `@page` rule) | `../moqui/runtime/base-component/webroot/screen/includes/Header.xsl-fo.ftl:57-67` | The default print document asks for a running title (`@top-center`) and a page number (`@bottom-center`, "- N -") on every page; Java's default FOP document has neither, because the `xsl-region-before`/`xsl-region-after` static content in `Header.xsl-fo.ftl` is commented out | Low | Unclear |
| P6-05 | `runtime/base-component/framework/template/screen/ScreenSupport.js:120-121` (`widgetless`), `ScreenHtmlPrint.js:224-232` (`singleFieldCode`) | `../moqui/runtime/template/screen-macro/DefaultScreenMacros.xsl-fo.ftl:238-249` (`formSingleWidget`) | A `form-single` field whose widget is `hidden` prints nothing in the port; Java prints the field title and an empty value cell | Low | No |
| P6-06 | `crates/moqui-core/src/screen/cache.rs:143-161` (`canonical_render_mode`, `BUILT_IN_MODES`), `runtime/base-component/framework/conf/MoquiDefaultConf.xml:684-685` | `../moqui/framework/src/main/resources/MoquiDefaultConf.xml:429-430` | A direct `.xsl-fo` request answers `text/html` (the `html-print` document); Java's answers `text/xml` (the FO document). `plans/Parity-Mapping.md:361` says a direct `.xsl-fo` request still returns FO XML | Low | Unclear |

#### P6-01 — The xlsx cell of a conditional field shows a different value than Java's

Java's `FormListExcelRender.renderSheet` takes the widgets of a list field from one place only: `MNode defaultField = fieldNode.first("default-field"); ArrayList<MNode> childList = defaultField.getChildren()` (FormListExcelRender.groovy:162-163). A `conditional-field` is never consulted, so an exported cell always holds the default field's value, and a field that has only conditional fields makes the render fail with a null-pointer exception on `defaultField.getChildren()`.

The port walks the field the other way round. `ScreenXlsx.js:91` calls `S.fieldChain(field, subCellCode)`, which emits the first `conditional-field` whose `condition` holds and falls back to the `default-field`; `cellCode` (`:90-97`) then makes sure a field that wrote no cell still writes an empty one, so the columns stay under their headers. The module's own header comment records this ("a conditional-field whose condition holds is used, as in the csv mode, where Java read only the default-field", ScreenXlsx.js:24-25), and it is right that Java's `DefaultScreenMacros.csv.ftl:133-144` does pick the conditional field — but the xlsx render is a different class with a different rule, and Parity-Diffs records no such difference.

A user sees it in the export: a list field whose value is shown differently per row (a `conditional-field` on a status, a type, a date) exports the default field's value in Java and the row's own value in the port, so the spreadsheet no longer matches the list on the screen. The port's behaviour is the more useful one and matches its own csv and html modes, so the fix is a decision, not a change: either keep it and record it in Parity-Diffs, or read the `default-field` only, as Java does, and keep the empty-cell guard for a field that has none.

#### P6-02 — The xlsx render makes sheets Java's widget render never reaches

Java's xlsx output is not a template render: the mode is registered as a `screen-output` with `widget-render-class="org.moqui.poi.ScreenWidgetRenderExcel"` (`../moqui/runtime/component/moqui-poi/MoquiConf.xml:6-7`), and `ScreenWidgetRenderExcel.renderSubNodes` walks the widgets by hand (ScreenWidgetRenderExcel.groovy:89-120). It descends into `form-list` (a sheet), `section` (through `sri.renderSection`, so the section's conditions and actions run), `container-box` (`box-body`, `box-body-nopad`) and `container-row` (`row-col`). Everything else is ignored, and the comment says so: "NOTE: other elements ignored, including section-iterate (outside intended use case for Excel render".

The port implements the mode as a full macro render (`ScreenXlsx.js:126-148`), so it reaches everything the screen tree holds: `container-panel` (`panel-header`, `panel-left`, `panel-center`, `panel-right`, `panel-footer`), `container-split` (`split-side`, `split-main`), and `section-iterate`, which renders the section once per entry (`crates/moqui-core/src/screen/sri/sections.rs:273-289`). A form-list inside a panel becomes an extra sheet, and a form-list inside a `section-iterate` becomes one sheet per entry, the later ones renamed "Name (2)", "Name (3)" by `unique_sheet_name` (`crates/moqui-core/src/screen/xlsx.rs:300-316`).

The port exports more than Java, never less, so nothing is lost; but the workbook of a screen with a form-list in a panel has a sheet Java's has not, and a screen whose only form-list sits in a `section-iterate` fails in Java ("No sheets rendered", FormListExcelRender's caller) and succeeds in the port. If the intent is to match Java's sheet set, the xlsx macro module should walk only the containers Java walks; if the intent is the fuller export, Parity-Diffs should say so.

#### P6-03 — The xlsx header title of a field with no title

Java's xlsx header takes the title from the `header-field`, else the `default-field`, else `StringUtilities.camelCaseToPretty(fieldNode.attribute("name"))` (FormListExcelRender.groovy:124-129). `camelCaseToPretty` splits the name on `(?=[A-Z0-9\.#])`, capitalizes each part, and turns a part that is "id" into "ID" (StringUtilities.java:179-200), so `item1Name` becomes "Item 1 Name" and `order.id` becomes "Order Id".

The port uses the other title rule, the one the `fieldTitle` macro of the FTL libraries uses: `S.titleText` splits the name on `(?=[A-Z])` only and replaces "Id" with "ID" (`ScreenSupport.js:160-164`), and `ScreenXlsx.js:39-49` applies it when the chosen sub-node has no `title` attribute. So `item1Name` becomes "Item1 Name" and a name with a dot keeps the dot.

The port's rule matches Java's csv, print and html modes, so the xlsx header is the odd one out; a user comparing the export with the screen sees a header that reads "Item1 Name" where Java's export reads "Item 1 Name". Fixing it is a one-line change in `ScreenXlsx.js` (a `camelCaseToPretty` of the field name) if the xlsx header is to match Java's xlsx, or a decision to keep the field-title rule in every mode.

#### P6-04 — The default print document has a running title and a page number Java's does not have

Java's print document is the `Header.xsl-fo.ftl` included by the webroot screen: a `fo:root` with ten `fo:simple-page-master`s and a `fo:page-sequence`. Its `fo:static-content` for `xsl-region-before` (the document title) and `xsl-region-after` (the page number, "- N -") is present in the file but **commented out** (Header.xsl-fo.ftl:58-67), so a FOP document from the default header has no running header and no page number. The twelve application print templates carry their own footers, and those are in both runtimes.

The port's default print document asks for both: the `@page` rule writes `@top-center { content: "<title>" }` when `documentTitle` is set and always writes `@bottom-center { content: "- " counter(page) " -" }` (ScreenHtmlPrint.js:92-96), and the port's own review found the margin box printing ("the margin box printed 'Page 2 of 2'", `plans/archive/Markdown-PDF-Review.md:51`). A port PDF therefore carries the document title at the top of every page and a page number at the bottom, where a Java PDF carries neither.

Parity-Diffs, [Print fixes (2026-10-18)](../../plans/Parity-Diffs.md), the "PDF footers" row records the running footer as an `@page` margin box and the page numbering of a document printed several times to a render; what it does not record is that the footer and the running title exist at all in the default document, where Java's has none. The port follows the design the commented-out FOP block describes, so this may well be intended; the reviewer should decide whether the extra title line and page numbers are wanted on every printed screen, and record the answer.

#### P6-05 — A hidden field of a printed form loses its title

Java's `formSingleWidget` (DefaultScreenMacros.xsl-fo.ftl:238-249) writes the field as a two-cell table — the title cell and the value cell — and then recurses into the field's widgets. A field whose widget is `hidden` takes the second branch (`<#recurse fieldSubNode/>`), so the table is written, the title is in it, and the value cell is empty (the `hidden` macro itself writes nothing). The same holds for a `hidden` widget in a field with `hide="false"`.

The port treats a field with a `hidden`, `ignored` or `submit` widget as a field with nothing to print: `widgetless` (ScreenSupport.js:120-121) makes `singleFieldCode` return an empty string (ScreenHtmlPrint.js:224-232), so neither the title cell nor the value cell is written. `ignored` and `submit` agree with Java (both return before the table is written), but `hidden` does not.

The effect is small but visible: a printed form in Java shows the label of every hidden field with an empty value beside it, and in the port those labels are gone, so the printed form has fewer lines than the form on the screen. The fix is in `widgetless`: a field whose only widget is `hidden` should still print its title with an empty value, as Java does.

#### Phase 6 checked and found equivalent

- **The macro set of the XSL-FO library.** Every `<#macro` of `DefaultScreenMacros.xsl-fo.ftl` has a counterpart in `ScreenHtmlPrint.js`, and the macros Java leaves empty have no-ops in the port. Enumerated against the FTL: `screen`, `widgets`, `fail-widgets` (ScreenHtmlPrint.js:336-339); `subscreens-menu` (empty in Java, a no-op in the port), `subscreens-active` and `subscreens-panel` (`sri.renderSubscreen()`, from `ScreenSupport.js:238-239`); `section` and `section-iterate` (`sri.renderSection`, `ScreenSupport.js:240-241`); `section-include` (`sri.renderSectionInclude`, ScreenHtmlPrint.js:386 — the port passes the node's name, attributes and text, which is all the Rust side reads, `screen/sri/sections.rs:132-153`); `include-screen` (`ScreenSupport.js:216-217`); `container`, `container-box` (with `box-header`, `box-body`, `box-body-nopad`), `container-row`, `row-col`, `container-panel` (`panel-header`, `panel-left`, `panel-center`, `panel-right`, `panel-footer`) and `container-split` (ScreenHtmlPrint.js:341-385); `link` and its inner `linkFormLink` (ScreenHtmlPrint.js:145-168), `label` (:188-203), `image` (:205-217), `parameter` (empty in Java, a no-op in the port); `form-single` with `field-ref`, `fields-not-referenced`, `field-row`, `field-row-big`, `field-group`, `field-accordion` (ScreenHtmlPrint.js:224-299), `field`, `conditional-field`, `default-field`, `header-field`; `form-list` with `formListHeaderField`, `formListSubField` and `formListWidget` (ScreenHtmlPrint.js:320-332 and `ScreenSupport.js:125-145`); the field widgets `check`, `drop-down`, `radio` (ScreenHtmlPrint.js:131-142), `date-time` (:422-423), `display` (:424), `display-entity` (:425), `submit` (:426), `text-area`, `text-line`, `text-find` (:427-429); and the empty ones `date-find`, `date-period`, `file`, `hidden`, `ignored`, `password`, `range-find`, `reset`, `row-actions`, `set`.
- **The attributes of the widgets.** The port reads the same attributes as the FreeMarker macros: the `condition` of a link, a label and a status bar (Java's `ec.getResource().condition`, the port's `raw(condition)`); the `text-map` of a link and a label (`ec.resource.expand(text, "", textMap)` in both); `no-current-selected-key` on a check, a drop-down and a radio (Java's `.node["@no-current-selected-key"]!`, the port's `attr(node, "no-current-selected-key")`); `default-value` and `format`/`type` on a date-time (the port's `S.dateTimeFormat` gives the same `yyyy-MM-dd`, `HH:mm` and `yyyy-MM-dd HH:mm` defaults, `ScreenSupport.js:276-281`); `currency-unit-field` and `currency-hide-symbol` on a display (`S.displayValueExpr`, `ScreenSupport.js:248-264`); `align` on a form-list field (the port writes it as the cell's `text-align`, Java as `fo:block text-align`); `print-width` and `print-width-type` on a field (see the widths below); `entity-name`, `key-field-name`, `entity-key-name` and `use-cache` on a display-entity or a link with an entity (`screen/sri/fields.rs:209-258` follows Java's `getFieldEntityValue`: the default text when the field is empty or the entity is not found, the entity's map for a `text` attribute, the entity's default description field otherwise).
- **The page layout.** The ten page masters of `Header.xsl-fo.ftl:1-55` (letter, legal, tabloid, A4 and A3, portrait and landscape) with their page sizes and margins are the port's `$masters` (ScreenHtmlPrint.js:26-34); the line width for the form-list columns is Java's `layoutWidthIn * 72 / 6` (DefaultScreenMacros.xsl-fo.ftl:33-36) as the port's `$lineChars` (ScreenHtmlPrint.js:35); the column width algorithm is Java's `getFormListColumnCharWidths` (ScreenForm.groovy:2180+) in `screen/sri/layout.rs:27-156` — the fixed character widths, the percent widths, the `print-width="0"` column that takes no room and the rounding of the rest are the same. The `@page` size and margin the PDF renderer uses are the document's own (`preferCSSPageSize`, `moqui-host/src/pdf.rs:477`).
- **The form-list of the print document.** The columns come from the assembled form, as in Java: the macro module is generated from the assembled form node (`screen/generate.rs:262-300`), which is Java's `sri.getFormInstance(name)` and `getAllColInfo()`, and a form-list whose columns the user chose is rendered through `sri.renderUserColumnsForm` (`screen/sri/forms.rs:119-161`), as Java's `form-list-user-columns` does. The page range line ("low - high / count", Java's `context[listName + "PageRangeLow"]` etc., the port's `page-range` div, ScreenHtmlPrint.js:43-44), the header row that repeats on each page (`display: table-header-group`), the row that does not split (`break-inside: avoid`), the grey band on every other row (Java's `background-color="#EEEEEE"` on the even index, the port's `tbody tr:nth-child(odd)`, the same rows) and the cell padding of 2pt are the same. A list with no rows writes no table in both.
- **The PDF images.** `PdfRenderer::render` inlines every `<img>` whose source is a resource location as a `data:` URL (`screen/pdf.rs:96-251`), which is what Java's FOP resolver did through the resource facade (`FopToolFactory.groovy:97-123`, the `LocalResourceResolver`); an `http`/`https` source is not fetched and a source that is not a png, jpeg, gif, webp, bmp or svg is left as written — the recorded difference in Parity-Diffs, [Print fixes (2026-10-18)](../../plans/Parity-Diffs.md), "PDF images". The barcode is an inline SVG from `ec.resource.code128Svg` where Java had barcode4j's FOP extension; the difference in the quiet zone and the height is recorded in the same table, "barcodes".
- **The print pipeline around the document.** `/fop/<path>` renders the screen at that path as the print mode and sends it as a PDF, with `filename` or `saveFilename` making it a download (`screen/mod.rs:241-305`), as Java's `MoquiFopServlet` did with its FOP transform; the difference that the port makes only PDF is recorded in Parity-Diffs, "Print fixes (2026-10-18)", `/fop`, and P8a-17 for `print#ScreenDocument` (`service/tools.rs:179-221`, which renders `html-print` and hands it to `PdfRenderer`). The email attachment of a screen rendered in `xsl-fo` becomes a PDF (`service/mail/implementation.rs:926-991`, as Java's `sendEmailTemplate.groovy:305-313`), and the scheduled report of `run#ScreenScheduled` does the same, with `layoutMaster` set to `letter-landscape` and the full render path (`runtime/base-component/framework/service/org/moqui/impl/ScreenServices.xml:84-134`, the same lines as `../moqui/framework/service/org/moqui/impl/ScreenServices.xml:86-143`).
- **The xlsx sheet layout.** One sheet per form-list, named for the form (`ScreenXlsx.js:106-107`; Java's `wb.createSheet(formName)`, FormListExcelRender.groovy:60), a header row of the field titles, a row per list entry, a frozen header row (`set_freeze_panes(1, 0)` for Java's `createFreezePane(0, 1)`), the columns fitted to their text (`autofit` for Java's `autoSizeColumn`), and the row height left to the application (Java's `setHeight((short) -1)`) — `screen/xlsx.rs:150-206`.
- **The xlsx styles.** The header is bold, centred and filled with `GREY_40_PERCENT` (the port's `0x0096_9696`, `screen/xlsx.rs:37` and `:224-229`; Java's `IndexedColors.GREY_40_PERCENT`, FormListExcelRender.groovy:330-341); a string is left-aligned and wrapped, a number centred, a number of a field with `align="right"` right-aligned, a currency or a `.00` format right-aligned with the negatives in red, and the dates, times and stamps carry the `yyyy-MM-dd`, `HH:mm:ss` and `yyyy-MM-dd HH:mm` formats — the same number formats, the same currency format `#,##0.00_);[Red](#,##0.00)` and the same alignments as Java's `makeRow*Style` methods (FormListExcelRender.groovy:343-420).
- **The xlsx cell values.** One value keeps its type (a number is a number, a date is an Excel serial number with a date format, a time and a stamp likewise, a boolean and any other object are text), and several values are joined with line breaks and formatted with their widget's format (the port's `XlsxBook::cell`, `screen/xlsx.rs:103-126`; Java's two branches, FormListExcelRender.groovy:168-237). A value that is null is an empty cell, and the widgets Java ignores (`ignored`, `hidden`, `submit`, `image`, `date-find`, `file`, `password`, `range-find`, `reset`, FormListExcelRender.groovy:41-42) are the port's `IGNORED` list (ScreenXlsx.js:35). The port's cell never fails the whole export: a text over Excel's 32 767 characters is cut with an ellipsis (`screen/xlsx.rs:284-293`).
- **The xlsx response.** The mode is a binary mode with the spreadsheet's media type and is always standalone (`screen/cache.rs:137-152`), the file name comes from `saveFilename` as in Java's XLS link (`ScreenHtml.js:971-973`; Java's `DefaultScreenMacros.html.ftl:1277-1280`), and a screen with no form-list fails with "No sheets rendered" in both (`screen/xlsx.rs:150-153`; Java's `BaseArtifactException("No sheets rendered")`, ScreenWidgetRenderExcel.groovy:69-76). The port's test `a_form_list_in_xlsx_is_a_sheet_with_typed_cells` (`crates/moqui-core/src/screen/tests/widgets.rs:179-217`) checks the sheet name, the header row, the typed cells and the empty cell of a submit field.
- **The `xlsx` feature gate.** `screen/xlsx_off.rs` keeps the same call shape as `screen/xlsx.rs` (a sheet, a header, a cell, a row end, a cell count) and counts what the render asks for, and `to_bytes` reports that xlsx is not built into this host; the mode is still registered, so a request for it fails with that message where Java, without the moqui-poi component, has no xlsx mode at all and the render fails for the missing render mode.
- **The PDF engine's own behaviour.** One long-lived Chromium, one page per document, every request the page makes failed so the document reaches no network, no server and no session, the page size from the document's `@page` (`preferCSSPageSize`), the backgrounds printed (`printBackground`), the load and the fonts awaited before the print, `pdf_page_limit` pages at a time and `pdf_timeout` seconds for each (`moqui-host/src/pdf.rs:344-503`). The browser is found at startup, the reason PDF is unavailable is logged and reported to every caller (`moqui-host/src/pdf.rs:73-175`, `screen/pdf.rs:32-34`), and a browser that died is started again.

#### Out of scope

- The FOP engine and the FO to PDF transform (`FopToolFactory`, `FopConfParser`, the `fop.xconf`): Parity-Mapping, [Excluded on purpose](../Parity-Mapping.md) — "FreeMarker, GString, and FOP as engines"; print is `html-print` (README redesign 5). Only the behaviour of `FopToolFactory`'s resource resolver was used, as the reference for the image inlining.
- The FO document itself as an output format: the port's print mode is HTML, and the `/fop` path makes a PDF (Parity-Diffs, "Print fixes (2026-10-18)", `/fop`).
- The FOP servlet's other content types (`HtmlRenderServlet`'s `svg`, `png` and `fopPdf` outputs, its `pageFormat`, `scale`, `mediaType` and `includeImages` parameters, the CSSBox `HtmlRenderer`): Parity-Mapping, Excluded on purpose — the FOP servlet's other content types; the port's `/fop/<path>` makes only PDF.
- FreeMarker, GString and the FTL template renderers: Parity-Mapping, Excluded on purpose.
- The vuet and js render modes and `/vapps`: Parity-Mapping, Excluded on purpose. This is why the print mode does not rewrite a link's `/apps/` to `/vapps/` as the XSL-FO `linkFormLink` macro does (`DefaultScreenMacros.xsl-fo.ftl:132`): in this port `/apps/` is the right target.
- The barcode4j FOP extension (`<barcode:code128>`): the port draws the barcode as inline SVG; the difference in the quiet zone and the height is recorded in Parity-Diffs, "Print fixes (2026-10-18)", "barcodes".

#### Open questions

- Should the xlsx mode keep the port's rule (the `conditional-field` whose condition holds) or Java's (the `default-field` only, with the NPE on a field that has none)? P6-01 is a decision, not a defect: the port's rule matches the csv, text and html modes and the screen the user sees, but the xlsx export is not what Java's xlsx export gives.
- Is the running title and the page number wanted on every printed screen? P6-04 follows the design the commented-out `xsl-region-before`/`xsl-region-after` block of `Header.xsl-fo.ftl` describes, and Parity-Diffs records the `@page` margin box as the port of the XSL-FO running footer, but a Java PDF has neither and the answer is not written down.
- `plans/Parity-Mapping.md:361` says "A direct `.xsl-fo` request still returns FO XML", but the port rewrites `xsl-fo` to `html-print` everywhere (`screen/cache.rs:154-161`), answers `text/html` and its own conf registers `xsl-fo` as `text/html` (`runtime/base-component/framework/conf/MoquiDefaultConf.xml:684-685`). Either the mapping sentence is stale or the port is missing the FO mode; the mapping's Screens section ("`xsl-fo` as a stored mode name selects this") agrees with the code, so the sentence in the Macro libraries section should be corrected.

## Phase 7 — Screen URLs, tree, menu, static files, facade and cache

Compared `crates/moqui-core/src/screen/{url,urls,tree,menu,static_files,cache,info,schema,screen_test,dispatch,mod}.rs`
against `../moqui/framework/src/main/groovy/org/moqui/impl/screen/{ScreenUrlInfo,ScreenTree,ScreenFacadeImpl,ScreenTestImpl}.groovy`,
with `ScreenRenderImpl.groovy` (the file branch, `checkWebappSettings`, `getMenuData`), `ScreenDefinition.groovy`
(the subscreens, the path search, the transition lookup) and `RestSchemaUtil.groovy` (the schema documents) for the
parts the phase names.

Worked 2026-10-19: 1 High, 2 Med and 8 Low findings — the REST schema documents do not check the `REST_SCHEMA`
permission, the link permission check does not enforce the transition's own authorization, the static file response
ignores the webapp's `response-header` conf, and the file set, the cache check interval, the tree root, the inline
query parameters, the plain URL type, the `skip-actions` render-mode attribute, the schema media type and the
method-specific transition lookup each decide differently.

P7-01 fixed 2026-10-20: the schema documents ask for the `REST_SCHEMA` permission, as Java's
`RestSchemaUtil.requireSchemaAccess` does.

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P7-02 | `moqui-core/src/screen/urls.rs:181-213` | `screen/ScreenUrlInfo.groovy:281-354` | The link permission check never enforces the last screen's own view permission when the path ends on a transition, and does not enforce the transition's authorization for a screen with `require-authentication="false"` | Med | No |
| P7-03 | `moqui-core/src/screen/static_files.rs:41-42`, `:233-243` | `screen/ScreenRenderImpl.groovy:666-720`, `framework/src/main/resources/MoquiDefaultConf.xml:305-311` | The file response's `Cache-Control` is hardcoded (1 hour, or 1 year for a versioned file); Java takes it from the webapp's `screen-resource-text` / `screen-resource-binary` / `screen-resource-template` response header (24 hours by default) | Med | Unclear |
| P7-04 | `moqui-core/src/screen/static_files.rs:29-39`, `:78-85`, `moqui-core/src/screen/render.rs:397-410` | `screen/ScreenUrlInfo.groovy:575-581`, `screen/ScreenRenderImpl.groovy:656-712` | The file branch serves a narrower set: a non-screen `.xml` (and `.properties`, `.sql`, `.groovy`) is refused, and a template file (`.md`, `.html.fancy`) under a screen without `include-child-content` is sent raw instead of being rendered | Low | Unclear |
| P7-05 | `moqui-core/src/screen/cache.rs:481-503`, `:612-623` | `screen/ScreenFacadeImpl.groovy:160-209` | A changed screen file is picked up at most once per `screen_check_interval_ms` (2 s by default) and never in production; Java reloads it on the next request | Low | Unclear |
| P7-06 | `moqui-core/src/screen/tree.rs:146-161` | `screen/ScreenTree.groovy:61-72` | A request with no `treeNodeId` gets the tree's root sub-nodes; Java gets the sub-nodes of the first `tree-node` in the definition | Low | Unclear |
| P7-07 | `moqui-core/src/screen/url.rs:519-525` | `screen/ScreenUrlInfo.groovy:872-883` | A `?` parameter whose value holds an `=` (or is empty) is kept; Java's `split("=")` drops the pair | Low | Unclear |
| P7-08 | `moqui-core/src/screen/urls.rs:78-99` | `screen/ScreenUrlInfo.groovy:361-393` | A plain `url-type` is prefixed with the base URL (a `mailto:` is mangled) and `path()` returns the plain text, where Java returns the plain URL as it is and an empty path | Low | Unclear |
| P7-09 | `moqui-core/src/screen/cache.rs:221-245` | `screen/ScreenFacadeImpl.groovy:86`, `:232`, `screen/ScreenSection.groovy:141-152` | The `skip-actions` attribute of a `screen-text-output` / `screen-output` is not read; a render mode that says it runs no section actions runs them | Low | No |
| P7-10 | `moqui-core/src/screen/schema.rs:33-39` | `util/RestSchemaUtil.groovy:630`, `:657` | The schema document is sent as `application/json` with the message facade's messages merged into it; Java sends `application/schema+json` as a file named `<entity>.schema.json` | Low | Unclear |
| P7-11 | `moqui-core/src/screen/urls.rs:62-65`, `moqui-core/src/screen/definition.rs:692-712` | `screen/ScreenDefinition.groovy:503-517` | A URL instance finds a method-specific transition without a request method; Java's `getTransitionItem(name, method)` only matches the transition of that method | Low | Unclear |

#### P7-02 — The link permission check does not enforce the transition's authorization

`ScreenUrlInfo.isPermitted` (`screen/ScreenUrlInfo.groovy:281-354`) walks the screens of the path and pushes each
one with the authorization required only for the last screen
(`isLast ? (!requireAuthentication || "true".equals(requireAuthentication)) : false`), then checks the transition
with `isPermitted(aeii, lastAeii, true, ...)` — always with the authorization required — unless the screen is
`anonymous-view` or `anonymous-all`. It also checks the transition's one service separately.

`UrlInstance::permitted` (`moqui-core/src/screen/urls.rs:181-213`) pushes every screen of the path with
`requires_authz = false` (`:193`) and only the last screen alone gets a real check when the path ends on a screen
(`:190-192`). When the path ends on a transition, the transition's frame is pushed with
`required = target.authentication == Authentication::Required` (`:201-211`), so the transition's own
`AT_XML_SCREEN_TRANS` record is enforced only for a screen that requires authentication.

Two consequences. For a screen with `require-authentication="false"` (the port's `Authentication::Optional`), Java
still checks the transition with the authorization required — a user with no `AT_XML_SCREEN_TRANS` record gets a
disabled link — while the port pushes the frame with `requires_authz = false`, which by the port's own rule
("`requires_authz` false pushes the frame whatever the records say",
`moqui-core/src/user/authz.rs:337`, `:355`) does not enforce a deny and does not require an allow. The link is
enabled. And for a screen that does require authentication, Java checks the screen's own view record for the last
screen of the path; the port does not, so a user with a record for the transition but none for the screen keeps an
enabled link.

What a user sees: a link to a transition the user may not use is drawn as an ordinary link in the port and is
greyed out in Java; clicking it in the port gives the authorization error page instead of a disabled link. The
request itself is still refused by the render's own check, so this is a link that should have been disabled, not an
open door. The fix is to push the screens of the path with the last one's `requires_authz` as Java does (the
`Optional` case included: Java requires the authorization for a `require-authentication="false"` screen, and only
`anonymous-view` / `anonymous-all` skip it), and to check the transition with the authorization required.

#### P7-03 — The file response ignores the webapp's response-header conf

Java's file branch takes its cache header from the webapp's conf: `webappInfo.addHeaders("screen-resource-binary",
response)` for a binary file, `"screen-resource-text"` for a text file and `"screen-resource-template"` for a
template (`screen/ScreenRenderImpl.groovy:666-720`). The default conf gives all three
`Cache-Control: max-age=86400, must-revalidate, public` and the template
`no-cache, no-store, must-revalidate, private` (`framework/src/main/resources/MoquiDefaultConf.xml:305-311`).

The port hardcodes the header (`moqui-core/src/screen/static_files.rs:233-243`): `public, max-age=3600,
must-revalidate` for a plain file and `public, max-age=31536000, immutable` for a file with a `.min.` in its name or
a `v` parameter (`:41-42`). The conf's `response-header` entries are parsed and merged
(`moqui-core/src/context/conf.rs:242-245`) but never applied by type anywhere in the port.

What a user sees: a screen's static file is revalidated every hour instead of every day, and a deployment that
tightens or loosens the cache rule in the conf has no effect on the files. The same gap covers the other header
types (`screen-render`, `screen-server-static`, `screen-secure`, `cors-actual`, `cors-preflight`), which the port
hardcodes in `web::finish_response` (`moqui-core/src/web/mod.rs:1429-1454`) — that part belongs to the web phase,
but the file branch is the one that reads a `screen-resource-*` header in Java.

The fix is to read the `response-header` entries of the webapp's conf by type and use the `screen-resource-text` /
`screen-resource-binary` value for a file response, as Java does.

#### P7-04 — The file branch serves fewer files, and a template file is sent raw

Java's file branch accepts any file under the screen's directory that exists, is not a directory and is not a
screen (`screen/ScreenUrlInfo.groovy:575-581`), and a file that has a template renderer is rendered as a template
(`screen/ScreenRenderImpl.groovy:656-712`).

The port refuses a file whose extension is in `BLOCKED_EXTENSIONS` — `xml`, `ftl`, `fancy`, `fancy-locked`,
`groovy`, `java`, `kt`, `properties`, `sql` — unless it is a template under a screen with `include-child-content`
(`moqui-core/src/screen/static_files.rs:29-39`, `:78-85`). So a data file such as `Note.xml` next to a screen is a
404 in the port and a download in Java, and a `.md` or `.html.fancy` file under a screen without
`include-child-content` is sent as it is (the raw markdown) where Java renders it.

The port's own test records the narrower set as intended (`moqui-core/src/screen/tests/files.rs:124-159`), so the
divergence is deliberate but not recorded in [Parity-Diffs]. A deployment that keeps a non-screen XML file under a
screen directory (a data file, a config fragment) loses it in the port. The fix, if the narrower set is kept, is a
row in Parity-Diffs; if not, the blocked list should hold only the source files the port has no renderer for.

#### P7-05 — A changed screen file is reloaded at most once every two seconds

Java's `makeScreenDefinition` (`screen/ScreenFacadeImpl.groovy:160-209`) compares the file's last-modified time
with the definition's load time on every request (and the `extends` dependencies with theirs), so an edited screen
is the new screen on the next request.

The port checks the same files, but at most once per `screen_check_interval_ms` (2 s by default, and never when
`instance_purpose` is `production`) (`moqui-core/src/screen/cache.rs:481-503`, `:612-623`). Within that window an
edited screen keeps serving the old definition, and the resolved paths that hold it keep serving the old walk.

What a user sees: a developer who edits a screen and reloads within two seconds sees the old screen. The rule
itself is a sensible one (it keeps a hot screen from re-parsing on every request) and the module doc says so
(`moqui-core/src/screen/cache.rs:15-24`), but it is a changed default that Parity-Diffs does not record, and the
production case (never) is a bigger difference than Java's.

#### P7-06 — A tree request with no node id gets the root's nodes

Java's `sendSubNodeJson` (`screen/ScreenTree.groovy:61-72`) uses the root's sub-nodes only when `treeNodeId` is
`"#"`; with no `treeNodeId` it looks up `treeNodeName`, and when that names nothing it warns and falls back to the
sub-nodes of the **first** `tree-node` of the definition.

The port treats an empty `treeNodeId` as the root (`moqui-core/src/screen/tree.rs:146-161`:
`if node_id == "#" || node_id.is_empty()`), so it answers with the root's sub-nodes.

What a user sees: the tree clients always send `"#"` for the root (`runtime/base-component/webroot/screen/webroot/js/moqui-html.js:407`,
`WebrootVue.qvt.js:1295`), so nothing in the shipped clients sees it; a direct call to the tree endpoint without
`treeNodeId` gets the root's nodes in the port and the first node's nodes in Java. The fix is to drop the
`is_empty()` from the root test and let the "no name" branch fall through to the first node, as Java does.

#### P7-07 — A `?` parameter with an `=` in its value is kept

Java splits the query tail on `&` and then on `=`, and keeps a parameter only when the split gives exactly two
parts (`screen/ScreenUrlInfo.groovy:872-883`). So `?a=b=c` and `?a=` are both dropped.

The port parses the tail with `web::form_pairs` (`moqui-core/src/screen/url.rs:519-525`), which splits on the first
`=` only: `?a=b=c` gives `a=b=c` and `?a=` gives `a` with an empty value (`moqui-core/src/web/mod.rs:1215-1225`).
The `~name=value` form of the same rule *is* Java's in the port (`cleanup_path_names`,
`moqui-core/src/screen/url.rs:631-640`, with a test for it in `moqui-core/src/screen/tests/walk.rs:284-290`), so the
two forms of the same convention now decide differently.

What a user sees: an inline parameter whose value holds an `=` (a filter expression, a base64 value) reaches the
screen in the port and is silently dropped in Java. The port's behavior is the useful one; the finding is that the
two halves of the convention disagree and that neither is recorded in Parity-Diffs.

#### P7-08 — A plain URL type is prefixed with the base URL

Java's plain-URL stub returns the URL as it is: `getBaseUrl` returns `plainUrl` when it is set
(`screen/ScreenUrlInfo.groovy:361-377`) and `getUrlWithBase` then appends the (empty) path list
(`:379-393`); `getPath()` for a plain URL is the empty string, because the stub has no path names.

The port's `UrlInstance::url` (`moqui-core/src/screen/urls.rs:78-87`) returns the plain text unchanged only when
it holds `"://"` or starts with `"//"`; anything else is prefixed with the base URL, and `path()`
(`:68-76`) returns the plain text.

What a user sees: with a base URL set — a screen test, a scheduled email body, a `baseLinkUrl` render — a
`url-type="plain"` link of `mailto:someone@example.com` becomes `http://hostmailto:someone@example.com`, and a
plain `/some/path` becomes `http://host/some/path` where Java leaves it a path. In an ordinary web request the
base URL is empty, so the plain text passes through unchanged and nothing is seen; the difference shows in the
renders that set one. The fix is to return the plain URL as it is, as Java does, and to keep `path()` empty for a
plain URL.

#### P7-09 — The `skip-actions` attribute of a render mode is not read

Java's `ScreenFacadeImpl` reads `skip-actions` from every `screen-text-output` and `screen-output` node
(`screen/ScreenFacadeImpl.groovy:86`) and exposes it as `isRenderModeSkipActions` (`:232`); `ScreenSection` uses it
to run no section actions in that mode (`screen/ScreenSection.groovy:141-152`).

The port's `TextOutput` (`moqui-core/src/screen/cache.rs:95-101`) has the mime type, the macro module, the
`always-standalone` flag and the binary flag, and the conf parse (`:221-245`) reads `type`, `mime-type`,
`macro-template-location` and `always-standalone` — but not `skip-actions`. No mode in the shipped conf sets it,
so nothing in the port's own configuration shows the difference; a deployment that adds a mode with
`skip-actions="true"` gets its section actions run anyway.

The fix is one more field on `TextOutput` and one branch in the section render, as Java has.

#### P7-10 — The schema document is `application/json`, not `application/schema+json`

Java sends the schema document as `application/schema+json` with a file name — `MoquiEntities.schema.json` for all
the entities, `<entityName>.schema.json` for one (`util/RestSchemaUtil.groovy:630`, `:657`) — through
`sendTextResponse`, so the body is the document and nothing is added to it.

The port sends the value through `web::send_json_response` (`moqui-core/src/screen/schema.rs:33-39`), which sets
`application/json` and merges the message facade's messages into the document as a `messages` key
(`moqui-core/src/web/mod.rs:611-653`); no file name is sent.

What a user sees: a client that asks for the schema and checks the media type sees `application/json` where Java
sends `application/schema+json`, and a schema document can carry a `messages` key that Java's never has. The
document's shape is a recorded difference ([Parity-Diffs](../Parity-Diffs.md), the `P2-12` row of the entity and
service review: `$defs`/draft 2020-12 instead of `definitions`/draft-04); the media type and the file name are
not.

#### P7-11 — A URL instance finds a method-specific transition without a request method

Java's `getTransitionItem(name, method)` (`screen/ScreenDefinition.groovy:503-517`) looks up `name#method` first
(a method-specific transition), then the bare `name` (the `any` transition), and only then tries the name as a
regular expression — and that last step needs a non-empty method. A transition declared `method="post"` is
registered under `save#post` (`:229`), so a GET request never finds it.

The port's `ScreenDef::transition` (`moqui-core/src/screen/definition.rs:692-712`) falls back, for an empty
method, to "the first transition with that name" — which is the method-specific one — and `UrlInstance::transition`
(`moqui-core/src/screen/urls.rs:62-65`) always asks with `None`.

What a user sees: a link to a `method="post"` transition is a POST button in the port (the transition is found, so
it is not read-only) and a plain anchor in Java, where the target of a GET page is not a transition at all. The
port's behavior is the more useful one, but the two decide differently, and the same lookup feeds `is_screen_url`
and `disable_link`.

#### Phase 7 checked and found equivalent

- **The URL walk** (`moqui-core/src/screen/url.rs:210-382` against `screen/ScreenUrlInfo.groovy:449-787`): the
  path names, the render-mode extension (`pathNamePreDot`, `rmExtension`, the `always-standalone` switch), the
  no-sub-path wrapper (the wrapper's name joins the path and the walk continues under it), the transition branch
  (the name is taken off the extra path), the file branch, the `default-item` loop, the `conditional-default`
  elements, the `moqui.screen.SubscreensDefault` rows (the last row whose condition holds wins, the newest
  `defaultSeqId` first), `always-use-full-path`, the `begin-transaction` and `transaction-timeout` merge, the
  standalone rules (a positive `lastStandalone` keeps the last screens, a negative one drops from the front) and
  the menu-image scan with its default-subscreen limit all decide the same way.
- **The not-found branch**: a name that is neither a subscreen nor a transition and matches no file is a 404
  (`ErrorKind::NotFound`, `moqui-core/src/screen/url.rs:288-293`, `:348`), as Java's `targetExists = false` and
  `checkExists()` (`screen/ScreenUrlInfo.groovy:605-614`, `:789-792`); `allow-extra-path` keeps the path
  (`:289-291`).
- **The path cleanup** (`moqui-core/src/screen/url.rs:618-645` against `screen/ScreenUrlInfo.groovy:920-948`):
  the empty names, the `.` and `..`, and the `~name=value` inline parameter, with Java's `split("=")` rule that
  keeps a pair only when it has two parts (tested in `moqui-core/src/screen/tests/walk.rs:266-291`).
- **`ScreenPathAlias`** (`moqui-core/src/screen/url.rs:577-614` against `screen/ScreenUrlInfo.groovy:835-859`):
  the alias is read with the authorization off, the rows outside their `fromDate`/`thruDate` window are dropped, the
  newest `fromDate` wins, and a read failure leaves the path uncached so it is resolved again.
- **The `//` name search** (`moqui-core/src/screen/url.rs:649-728` against
  `screen/ScreenDefinition.groovy:523-614`): the direct child, the transition under the screen, the grandchild,
  the deep recursion and the file under the screen, in that order, and a direct child whose rest does not match is
  a miss rather than a deeper search. The port adds a depth cap where Java has none.
- **The URL cache** (`moqui-core/src/screen/url.rs:134-139`, `:170-172` against
  `screen/ScreenUrlInfo.groovy:136-165`): the key is the root screen, the path names and the `lastStandalone`, a
  walk that took a conditional default is not reused (`:322-328`, `:441-452`), and a cached walk is dropped when
  one of the screens it holds is no longer current (`moqui-core/src/screen/cache.rs:506-514`).
- **The screen-definition cache** (`moqui-core/src/screen/cache.rs:419-503` against
  `screen/ScreenFacadeImpl.groovy:152-209`): the definition is keyed by its location, the files it was read from
  (and the screens it extends) decide whether it is current, and `isScreen` reads only the root element of an
  XML file (`moqui-core/src/screen/cache.rs:556-576`).
- **The facade conf** (`moqui-core/src/screen/cache.rs:163-263` against `screen/ScreenFacadeImpl.groovy:74-88`):
  the render modes, their mime types, their macro modules, `always-standalone`, the binary `screen-output` modes,
  the `boundary-comments` flag and the `screen`/`subscreens-item` entries of the conf. `xsl-fo` is the print mode
  and `html-print` its canonical name in both.
- **The subscreens** (`moqui-core/src/screen/definition.rs:1290-1424` against
  `screen/ScreenDefinition.groovy:395-454`): the directory by convention, the `subscreens/subscreens-item`
  elements, the conf's `screen-facade/screen/subscreens-item` and its `default-subscreen`, and the
  `moqui.screen.SubscreensItem` rows with their `makeDefault`, in that order of override.
- **The root screen from the host** (`moqui-core/src/screen/info.rs:53-86` against
  `screen/ScreenFacadeImpl.groovy:350-365`): the first host pattern that matches the whole host wins, `.*` is the
  fallback even when it is listed first, and the port of the host is stripped first.
- **The screen info list** (`moqui-core/src/screen/info.rs:114-319` against
  `screen/ScreenFacadeImpl.groovy:368-484`): the counts of the screen and of all the screens under it, the
  non-placeholder rule (forms, sections, or more than four transitions), the transitions with actions (the port
  merges a transition's `service-call` into its actions, so it counts as Java's
  `hasActionsOrSingleService` does), the `indentedName` the tools screen reads (added in the script wrapper,
  `moqui-core/src/script/runtime/tools_host.rs:351-355`), and the walk that stops at a screen mounted twice.
- **The screen test** (`moqui-core/src/screen/screen_test.rs` against `screen/ScreenTestImpl.groovy`): the
  `rootScreen`, `baseScreenPath`, `renderMode`, `baseLinkUrl` and the request headers; the paths of the screens
  with no required parameter (`no_required_parameter_paths`, against
  `screen/ScreenDefinition.groovy:616-634`); the error strings (`[Template Error`, `FTL stack trace`,
  `Could not find subscreen or transition`); the counts of the renders, the errors and the characters; and the
  whole `ScreenTest` surface, which the script runtime exposes as `ec.screen.makeTest()`
  (`moqui-core/src/script/runtime/ec_screen.rs:41-86`).
- **The screen tree** (`moqui-core/src/screen/tree.rs:144-272` against `screen/ScreenTree.groovy:53-179`): the
  JSON of the jstree nodes (`id`, `text`, `li_attr.treeNodeName`, `a_attr` with the href, the load id and the URL
  text, `state.opened`/`selected`, `children`), the condition and the actions of the sub-node and of the node, the
  `nodeList` list with `nodeList_entry`, `nodeList_index` and `nodeList_has_next` in the context, the expansion of
  the id and the localized text, and the `javascript:` href of a `dynamic-load-id`.
- **The menu data** (`moqui-core/src/screen/menu.rs:150-351` against
  `screen/ScreenRenderImpl.groovy:2358-2520`): the 401 without a user, the 404 for a path that names no screen,
  the 403 for a screen the user may not view, the pre-actions of the screens below the first two, the entries of
  each level with their paths, the parameter strings, the images and their type, the `hasTabMenu` and
  `renderModes`, the last entry with the screen documents, the extra path list and the user's saved finds. The
  difference Java's `getMenuData` records for a broken subscreen — it throws and the whole menu answers 500 — is
  recorded in [Parity-Diffs](../Parity-Diffs.md) (the `menus` row of the gallery fixes, 2026-10-18), and the port
  leaves the entry out and logs it (`moqui-core/src/screen/menu.rs:76-87`).
- **The static file response** (`moqui-core/src/screen/static_files.rs:217-313`): the media type by extension, the
  `ETag` and the 304 for a matching `If-None-Match`, the one byte range of RFC 9110 (and the 416), the `HEAD`
  without a body, the `405` for another method, and the refusal of a path that leaves the screen's directory, of a
  hidden file and of a percent-encoded traversal (`:46-58`, `:87-103`).
- **The dispatch transitions** (`moqui-core/src/screen/dispatch.rs` against the `rest.xml` of both runtimes): the
  `login` with its second factor, the `sendOtp` and `verifyOtp`, the `logout`, the `e1`/`v1`/`m1`/`s1` entity and
  service calls, and the `sm` system message; the `X-HTTP-Method-Override` of a POST and the `X-Page-*` headers
  of a find. The bodies are the port's own Rust, not a port of the Groovy, so only the behavior was compared.
- **The schema documents** (`moqui-core/src/screen/schema.rs:25-40` against
  `util/RestSchemaUtil.groovy:584-663`): the entity name and the master name out of the path, the `.json`
  suffix, the master name `default` when none is given, and the 400 for an entity that does not exist. The shape
  of the document is a recorded difference ([Parity-Diffs](../Parity-Diffs.md), the `P2-12` row).

#### Out of scope

- **Swagger 2.0 and RAML**: the `entity.swagger`, `master.swagger`, `service.swagger`, `entity.raml`,
  `master.raml` and `service.raml` transitions answer 404 "Swagger is not built" / "RAML is not built"
  (`moqui-core/src/screen/dispatch.rs:57-64`); Parity-Mapping, *Excluded on purpose*, names both formats.
- **FreeMarker as an engine**: a `.ftl` file under a screen cannot be rendered in the port, so it is refused
  rather than sent; Parity-Mapping, *Excluded on purpose*, drops `FtlTemplateRenderer`.
- **The `vuet`, `vue` and `js` render modes**: not in the port's conf, so a path that ends in `.vuet` is a 404
  (`moqui-core/src/screen/tests/walk.rs:150-153`); Parity-Mapping, *Excluded on purpose*.
- **The entity and service REST bodies** (`e1`, `m1`, `v1`, `s1`): the shapes of the answers and the paging are
  the entity and service phases (P5 and P8a of the entity and service review), not this one.

#### Open questions

- ~~Should the `REST_SCHEMA` permission also be required for the other REST documents the port serves?~~ —
  answered on 2026-10-20: no, the schema documents only, as Java's `requireSchemaAccess` guards. The
  entity REST endpoints keep the entity's own authorization.
- Should the file response take its `Cache-Control` from the webapp's conf (P7-03), and with it the other
  `response-header` types the port hardcodes? That is one change in one place for all of them, and it belongs with
  the web phase.

## Phase 8 — The web facade

Compared `crates/moqui-core/src/web/{mod,multipart,address,error_info}.rs` (with the callers the reverse
index names: `entity/sensitive.rs`, `service/system_sync/message.rs`, `screen/sri/mod.rs`,
`script/runtime/ec_extra.rs`, and the script facade `script/runtime/facades.rs`) against
`../moqui/framework/src/main/groovy/org/moqui/impl/context/WebFacadeImpl.groovy`,
`../moqui/framework/src/main/java/org/moqui/context/WebFacade.java`,
`../moqui/framework/src/main/java/org/moqui/context/WebMediaTypeException.java` and
`../moqui/framework/src/main/java/org/moqui/util/WebUtilities.java`.

Worked 2026-10-19: the facade surface was enumerated method by method (every public method of Java's
`WebFacade` and `WebFacadeImpl` against the port's `web` module and its script facade); 19 findings —
1 High (an open redirect the port's redirect helper allows where Java refuses the target), 11 Med, 7 Low.

P8-01 and P8-02 fixed 2026-10-20: Java's `sendHttpRedirect` rule is in `http_redirect_location` in
`screen/render.rs` — the location is sanitized, an `http(s)` target is its path and query when the
webapp names no `http-host`, and a target that is not a path of this site goes to the root.

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P8-03 | `web/mod.rs:1161-1187` (`parse_body`) | `WebFacadeImpl.groovy:400-422` (`getRequestParameters`), `ContextStack.getByString` | The request body wins over the query string in the port (a JSON field overwrites a query parameter; a multipart field becomes a second value of a query parameter); in Java the servlet parameters are pushed above the JSON and multipart maps, so the query value wins | Med | No |
| P8-04 | `web/mod.rs:1196` (`pairs_to_parameters`), `multipart.rs:570` | `WebUtilities.java:148-158`, `WebFacadeImpl.groovy:225` | A single non-breaking space (`\u00a0`) is Java's "no parameter" value and becomes `null`; the port keeps the text, in a query value and in a multipart field alike | Med | No |
| P8-05 | `web/mod.rs:1447-1453` (`finish_response`) | `MoquiServlet.groovy:226-232` (`handleCors`) | An origin that is not allowed is answered 401 "Origin not allowed" in Java and the request is not run; the port runs the request and only leaves the CORS headers off | Med | No |
| P8-06 | `web/mod.rs:1371-1389` | `ExecutionContextFactoryImpl.groovy:1866-1872` | `require-session-token`, `handle-cors`, `allow-origins` and `upload-executable-allow` are read from the properties only; Java reads the webapp element's attribute (which defaults to the property) | Med | No |
| P8-07 | `script/runtime/facades.rs:683-955` (no `sendError`) | `WebFacade.java:90`, `WebFacadeImpl.groovy:883-941` | `ec.web.sendError(code, message, throwable)` is not ported; the port's own `Tools/Entity/DataSnapshot.xml` uses `sendJsonError` instead, so a browser gets a JSON body where Java sends an HTML error page | Med | No |
| P8-08 | `script/runtime/facades.rs:932-941` (`getWebappRootUrl`), `web/mod.rs:501` | `WebFacadeImpl.groovy:543-580` | The port answers the webapp root URL with the render's base URL, which is empty for a real request; Java answers the full URL (`requireFullUrl`) or the context path | Med | No |
| P8-09 | `web/mod.rs:674-689` (`send_text_response_with`) | `WebFacadeImpl.groovy:773-784` (`sendTextResponseInternal`) | When the message facade has an error, Java sends the errors text with a 500; the port sends the text with a 200 | Med | No |
| P8-10 | `web/mod.rs:245-252` (`parameters`), `script/runtime/facades.rs:776-779` | `WebFacadeImpl.groovy:372-391` (`getParameters`) | Java's `ec.web.parameters` is the live `ContextStack` (the request parameters over the session and application attributes, writable); the port hands a copy of the request parameters, so a session value read through it is gone and a write to it is lost | Med | No |
| P8-11 | `web/mod.rs:64`, `web/mod.rs:1271` (`cookie_value`) | the servlet's `getHeader` / `getCookies` | The port keeps one value per header name, so a header sent twice is joined with `", "`; two `Cookie` headers then make the session cookie read as `"abc, moqui.session=def"` and the session is not found | Med | No |
| P8-12 | `web/mod.rs:1166` (`parse_body`) | `WebFacadeImpl.groovy:133-160`, `ResourceReference.java:448-470` (`isTextContentType`) | The port puts the body text in `_requestBodyText` for any body that is not a form, a binary one included; Java reads the body only for a text content type, so a binary body is no parameter at all | Med | No |
| P8-13 | `web/mod.rs:1167` | `WebFacadeImpl.groovy:146` | The port parses a `+json` body (`application/merge-patch+json`) into parameters; Java parses only `application/json` and `text/json` | Low | No |
| P8-14 | `multipart.rs:499`, `multipart.rs:591` | `WebFacadeImpl.groovy:171-207` | The port strips the directories from the uploaded file's name and gives a part without a `Content-Type` the type `application/octet-stream`; Java keeps the client's name (`getName()`) and a part without a type has a null type | Low | Unclear |
| P8-15 | `script/runtime/error_info.rs:61-82`, `web/error_info.rs:30-54` | `ArtifactExecutionInfoImpl.java:504-510` | The artifact info's `toString` is `"Screen <name> (View)"` in the port and Java's is the bracketed `[name:..., type:..., action:..., ...]` form; `getArtifactStack()` is an empty list in the port | Low | No |
| P8-16 | `screen/sri/mod.rs:157` (`encode_html`) | `WebUtilities.java:266-287` | `encodeHtml` replaces the C0 control characters Java replaces (0x05, 0x12-0x14, 0x16, 0x17, 0x19); the port writes them into the HTML as they are | Low | No |
| P8-17 | `web/mod.rs:302-316` (`full_url`) | `WebFacadeImpl.groovy:357-365` (`getRequestUrl`) | The port rebuilds the URL from the decoded query pairs, so a value with a space is not encoded again, and it keeps the `Host` header's `:port` where Java drops `:80` and `:443` | Low | No |
| P8-18 | `web/mod.rs:764-774` (`send_resource_response`) | `WebFacadeImpl.groovy:823-834`, `MoquiDefaultConf.xml:305` | Java caches every inline resource response publicly for a day; the port does that only for the app's own files (`component://`, `classpath://`) and answers `private, no-store` for the rest | Low | Unclear |
| P8-19 | `web/mod.rs:1516-1525` (`strip_credential_parameters`) | `WebUtilities.java:731-740` (`stripCredentialParameters`) | The port strips the credential names at every depth of a JSON value; Java strips only the top level of the parameter map | Low | Unclear |

#### P8-03 — The request body wins over the query string

Java's `getRequestParameters` (`WebFacadeImpl.groovy:400-422`) pushes the maps in this order:
saved parameters, multipart parameters, JSON parameters, the declared path parameters, the servlet's
own parameter map (the query string and the form body), the path-info parameters, and an empty map.
`ContextStack.getByString` reads from the top down, so the servlet's parameters win over the JSON
parameters and over the multipart parameters, and the path-info parameters win over all of them.

The port builds one flat map instead (`web/mod.rs:1100-1193`): the query pairs first, then the form
pairs, then the multipart fields, and finally the JSON object's fields, which are written over the
map with `parameters.insert(key, value)` (`web/mod.rs:1169-1173`). So:

- `POST /rest/s1/x?orderId=1` with the JSON body `{"orderId": 2}` reads `orderId` as 2 in the port
  and as "1" in Java.
- A multipart upload with `?name=x` in the query and a field `name` in the body reads a list of both
  in the port and the query value alone in Java.

A REST call that means to update one record and is given the id in the query string updates a
different record in the port. The fix is to keep the layers apart as Java does: the query and form
values first, the body values only for the names the query does not carry.

#### P8-04 — The "no parameter" value (a non-breaking space) is kept as text

Java uses `\u00a0` as the placeholder for "no value": `simplifyRequestParameters`
(`WebUtilities.java:148-158`) maps a single value of `"\u00a0"` to `null`, and
`addValueToMultipartParameterMap` (`WebFacadeImpl.groovy:223-237`) does the same for a multipart form
field. The comment says it is "used as a placeholder when empty string doesn't work".

The port never does this: `pairs_to_parameters` (`web/mod.rs:1196-1212`) makes every single value a
string, and the multipart fields (`multipart.rs:570`) keep their text. A form that posts `&nbsp;` for
a field that must be empty shows a non-breaking space in the port and an empty field in Java, and a
script that tests `if (param == null)` takes the other branch in the port.

#### P8-05 — A CORS request from a disallowed origin is answered 401 in Java and runs in the port

`MoquiServlet.handleCors` (`MoquiServlet.groovy:181-249`) computes the allowed origin and, when the
origin is in none of the allowed sets, logs a warning and answers 401 "Origin not allowed" through
`WebFacadeImpl.sendError`, and the servlet returns without running the request. The same-origin case
(the origin's domain equals the request's host) and the `*` case are answered with the CORS headers.

The port's `finish_response` (`web/mod.rs:1447-1453`) asks `origin_allowed_for` and, when the origin
is not allowed, simply does not add the CORS headers; the request is served as usual. A browser still
blocks the response, so the difference is invisible to the browser, but the request itself has run: a
cross-origin `POST` from a page on a site that is not on the allow list still changes data in the port
and is refused in Java. Java's 401 is one of the two layers in front of a cross-site request (the
other is the session token), and the port drops it.

#### P8-06 — The webapp's CORS, session-token and upload settings are read from the properties only

`ExecutionContextFactoryImpl.WebappInfo` (`ExecutionContextFactoryImpl.groovy:1857-1872`) reads
`require-session-token`, `handle-cors`, `allow-origins` and `upload-executable-allow` from the
`<webapp>` element of the conf. The default conf points them at the properties
(`webapp_require_session_token`, `webapp_handle_cors`, `webapp_allow_origins`,
`webapp_upload_executable_allow`), so a deployment that leaves them alone gets the same result in the
port. A deployment that sets the attribute on its own `<webapp>` element gets a different one: the
port reads the property instead (`web/mod.rs:1371-1389`), so the CORS allow-list of the conf is empty
(no origin is allowed but the one of the request itself), `handle-cors="false"` in the conf does not
stop the CORS headers, `require-session-token="false"` still refuses a POST without a token, and
`upload-executable-allow="true"` still refuses an executable upload. The port already has the machinery for this: `ExecutionContextFactory::webapp_setting`
(`context/factory.rs:550`) reads a webapp attribute, as `web/address.rs:147-160` does for
`client-ip-header`.

#### P8-07 — `ec.web.sendError` is not ported

`WebFacade.sendError(int, String, Throwable)` (`WebFacade.java:90`) sends an error with the shape the
`Accept` header asks for: an HTML page for a browser, a JSON body with `message`, `errorName`, `error`
and `path` for a JSON client, and `text/plain` otherwise (`WebFacadeImpl.groovy:883-941`). The port's
`HostWeb` (`script/runtime/facades.rs:683-955`) has `sendJsonError` but no `sendError`, so the method
is gone from the facade.

The port's own `Tools/Entity/DataSnapshot.xml` had to be changed for it: it now calls
`ec.web.sendJsonError(404, "Snapshot not found")` where Java calls
`ec.web.sendError(404, "Snapshot not found", null)`. A user who asks the Tools screen for a snapshot
that is not there gets a JSON body in the browser in the port and the HTML error page in Java. The
fix is a `sendError` on the facade that writes the HTML, JSON or plain body as Java does.

#### P8-08 — `ec.web.getWebappRootUrl` answers an empty string for a real request

Java's `getWebappRootUrl` (`WebFacadeImpl.groovy:543-580`) builds the URL from the webapp's
`http-host`/`https-host` and the ports, or from the request's own host when the webapp names none; it
answers the full URL when `requireFullUrl` is true and the context path otherwise.

The port's script method (`script/runtime/facades.rs:932-941`) ignores its arguments and answers
`web::base_url()`, which is the base URL of a render that was made without a socket. The host installs
the web slot with an empty base URL for a real request (`screen/mod.rs:178`), so the method answers
`""` for every request. Java answers `/` for `requireFullUrl` false (the same as the port, so the FOP
links in the ported screens are right) but the full URL for `requireFullUrl` true, which the ported
`Tools/Entity/SpeedTest.xml` and the gallery screens use to build an absolute URI. The port has the
pieces: `ExecutionContextFactory::webapp_root_url` (`context/factory.rs:511`) and the request's own
host.

#### P8-09 — A text response does not become the errors text

Java's `sendTextResponseInternal` (`WebFacadeImpl.groovy:773-784`) checks the message facade first:
when it has an error, the response is the errors string with a 500, not the text with a 200. The port's
`send_text_response_with` (`web/mod.rs:674-689`) always sends the text with a 200.

The JSON answer has the rule (`web/mod.rs:632-647`), so the two answers of one request disagree in the
port. A screen that adds an error message and then sends a text response (the ported Tools screens
send XML and text) shows the text and a 200 where Java shows the error and a 500.

#### P8-10 — `ec.web.parameters` is a copy of the request parameters, not the live stack

Java's `getParameters` (`WebFacadeImpl.groovy:372-391`) returns a `ContextStack` of the request
parameters, the application attributes, the session attributes, the request attributes and an empty map
on top. A read falls through to the session and the application attributes, and a write lands in the
empty map at the top, so a script can add a parameter that the rest of the request sees.

The port's `web::parameters()` (`web/mod.rs:245-252`) clones the request parameter map, and the script
facade hands the script that copy (`script/runtime/facades.rs:776-779`). A ported script that reads a
session attribute through `ec.web.parameters` gets nothing, and a script that writes one changes only
its own copy. `ec.web.sessionAttributes` and `ec.web.requestParameters` are there for the two common
cases, but the fall-through and the write are not.

#### P8-11 — A header sent twice is joined, and the cookie parser then reads a wrong value

The port keeps one value per header name (`web/mod.rs:64`), and the host builds that map by joining the
values of a repeated header with `", "` (`moqui-host/src/http.rs:853-867`). The servlet's
`getHeader` gives the first value and `getCookies` gives the cookies of every `Cookie` header.

`web::cookie_value` (`web/mod.rs:1271-1281`) splits the header on `;`, so a client that sends two
`Cookie` headers (`moqui.session=abc` and `moqui.session=def`, which HTTP/2 and some proxies do) makes
the port read the session cookie as `abc, moqui.session=def`. The session is not found, so the user is
logged out on every such request, and the visitor cookie has the same problem. The fix is to keep the
values of a header as a list (or to split the joined `Cookie` value on `, ` as well), and to read the
first value as the servlet does.

#### P8-12 — A binary body becomes the `_requestBodyText` parameter

Java reads the request body in the `WebFacadeImpl` constructor (`WebFacadeImpl.groovy:133-160`) only
when the content type is a text type (`ResourceReference.isTextContentType`,
`ResourceReference.java:448-470`: `text/*`, `application/javascript`, `application/json`,
`application/jwt`, `+json`, `application/rtf`, `application/xml*`, `+xml`, `application/yaml*`,
`+yaml`). A body of any other type is not read at all: `getRequestBodyText()` is null and there is no
`_requestBodyText` parameter.

The port reads every body that is not a form (`web/mod.rs:1165-1166`) and puts the text in
`_requestBodyText`, a binary body included, decoded with `from_utf8_lossy`. A binary upload of a few
megabytes therefore becomes a lossy string in the parameter map, which the request details and the
artifact hit write out, and a service that reads `_requestBodyText` gets replacement characters where
Java gives it nothing.

#### P8-13 — A `+json` body is parsed into parameters

Java parses the body as JSON when the content type contains `application/json` or `text/json`
(`WebFacadeImpl.groovy:146`); a `+json` type is a text type, so the body text is read but the JSON is
not parsed and no field becomes a parameter. The port also parses `application/merge-patch+json` and
the other `+json` types (`web/mod.rs:1167`), so their fields become request parameters and can set
entity fields of a REST call.

#### P8-14 — The uploaded file's name and content type

Java puts the `FileItem` in the multipart map as it is: `getName()` is the file name the client sent,
with any path in it, and a part without a `Content-Type` header has a null type
(`WebFacadeImpl.groovy:171-207`). The port strips the directories from the name (`multipart.rs:591`,
`base_name`) and gives a part without a type the type `application/octet-stream`
(`multipart.rs:492-499`).

The name change is a hardening (a client cannot make the server believe a file is at
`/etc/passwd`), but it is a visible difference: a screen that shows the name of an uploaded file shows
the last segment in the port and the whole name in Java, and a script that tests the content type
against null sees `application/octet-stream` in the port.

#### P8-15 — The artifact info of an error

Java's `errorThrowable.getArtifactInfo()` gives an `ArtifactExecutionInfoImpl`, whose `toString` is
`[name:'...', type:'...', action:'...', required: ..., granted:..., user:'...', authz:'...',
authAction:'...', inheritable:..., runningTime:...]` (`ArtifactExecutionInfoImpl.java:504-510`), and
`getArtifactStack()` gives the stack of artifacts that were being checked. The port's artifact info
(`script/runtime/error_info.rs:61-82`) has the same `name`, `actionDescription` and `typeDescription`
as Java, but its `toString` is `"Screen <name> (View)"` and `getArtifactStack()` is an empty list.

The port's own `error/Forbidden.xml` shows only the name, the action and the type, so nothing the
shipped screens show changes; a ported screen that shows the detail modal of Java's `Forbidden.xml`
shows a different "Current:" line and no stack.

#### P8-16 — `encodeHtml` leaves the C0 control characters in

`WebUtilities.encodeHtml` (`WebUtilities.java:266-287`) replaces 0x05 with "...", 0x12 with "&apos;",
0x13 and 0x14 with "&quot;", 0x16 and 0x17 with "-" and 0x19 with "tm", beside the usual HTML
escapes. The port's `encode_html` (`screen/sri/mod.rs:157-170`) does the HTML escapes only, so a value
with one of those control characters is written into the page as the character itself. The
JavaScript-safe variant of the same helper does handle them (`script/runtime/ec_extra.rs:613-636`),
with the backslash and the U+2028/U+2029 escapes added.

#### P8-17 — The request URL is rebuilt from the decoded query pairs

Java's `getRequestUrl` (`WebFacadeImpl.groovy:357-365`) uses the scheme, the server name, the server
port (dropping 80 and 443) and the raw query string. The port's `full_url` (`web/mod.rs:302-316`)
uses the `Host` header as it came (so the port stays) and rebuilds the query from the decoded pairs,
so a value that held a space or a `&` comes out unencoded: the URL of the artifact hit and of the
visit record reads `?note=a b` where Java reads `?note=a%20b`.

#### P8-18 — An inline resource response is not publicly cacheable in the port

Java's `sendResourceResponseInternal` (`WebFacadeImpl.groovy:823-834`) sets the webapp's
`web-resource-inline` headers for every inline response, which is
`Cache-Control: max-age=86400, must-revalidate, public` in the default conf
(`MoquiDefaultConf.xml:305`). The port (`web/mod.rs:764-774`) sends that only for a file of the
application itself (`component://`, `classpath://`) and answers `private, no-store` for anything else
(a drive's file, a record's file, a temporary zip), so that a shared cache keeps no copy of what a
user was allowed to read. The code says so; the difference is not in `Parity-Diffs`.

#### P8-19 — The credential names are stripped at every depth

`WebUtilities.stripCredentialParameters` (`WebUtilities.java:731-740`) copies a parameter map and
leaves out the credential names at the top level only. The port's `strip_credential_parameters`
(`web/mod.rs:1516-1525`) walks the whole JSON value, so a nested map loses a key named `password` or
`api_key` too. The port says why (a `UserAccount` value in a screen's data holds the same names as
columns), but the two answers differ for a screen whose data holds a nested `password` field.

#### Phase 8 checked and found equivalent

The facade surface, method by method. Java's `WebFacade` interface and the public methods of
`WebFacadeImpl` against the port's `web` module and its script facade
(`script/runtime/facades.rs`, the `HostWeb` class):

| Java | Port | Result |
|---|---|---|
| `getRequestUrl` | `web::request_url()` (internal) | the same URL for the artifact hit; the script facade has no `getRequestUrl` (P8-17 for the encoding) |
| `getParameters` | `web::parameters()`, `ec.web.parameters` | present, but a copy (P8-10) |
| `getRequest` | — | the servlet type; out of scope |
| `getRequestAttributes` | — | the port has no request attributes; the only Java uses of them are `moquiRequestStartTime`, `moqui.webapp.httpHost`, `moqui.session.token.created` and `moqui.login.error`, which the port keeps elsewhere (the request start time in the render, the http host in the conf, the new session in the response headers, the login error in the render job) |
| `getRequestParameters` | `web::parameters()`, `ec.web.requestParameters` | the same set of names; the precedence differs (P8-03) |
| `getSecureRequestParameters` | `web::secure_parameters()` | the same rule: the names in the query string are left out, the body, the multipart and the JSON fields stay (`web/mod.rs:375-385`); the SQL runner reads it (`script/runtime/tools_host.rs:1602`) |
| `getHostName(boolean)` | `web::host_name()`, `ec.web.getHostName` | the host of the `Host` header; Java adds the server port when asked and the port does not add the default 80 or 443 (Low, same shape as P8-17) |
| `getPathInfo`, `getPathInfoList` | `screen/url.rs` | the port splits the path itself; the decoded segments are the same list |
| `getRequestBodyText` | the `_requestBodyText` parameter | present; the port also sets it for a binary body (P8-12) |
| `getResourceDistinctValue` | `web::resource_distinct_value()`, `ec.web.getResourceDistinctValue` | one value per process, as Java's `initStartHex` |
| `getResponse` | the web slot's output | the servlet type; out of scope |
| `getSession` | `user::session` | the port's own session (README redesign 7) |
| `getSessionAttributes` | `ec.web.sessionAttributes` | the same map, read and written |
| `getSessionToken` | `ec.web.sessionToken` | the same value; the response carries `moquiSessionToken` and `X-CSRF-Token` when it is new (`screen/mod.rs:468-474`), as Java's constructor does |
| `getServletContext`, `getApplicationAttributes` | — | the servlet context; out of scope |
| `getWebappRootUrl(boolean, Boolean)` | `ec.web.getWebappRootUrl` | present, but empty for a real request (P8-08) |
| `getErrorParameters` | `ec.web.errorParameters` | the same saved map, taken once per request |
| `getSavedMessages`, `getSavedErrors`, `getSavedValidationErrors` | `ec.web.savedMessages`, `savedErrors`, `savedValidationErrors` | the same lists, taken from the session once (`web/mod.rs:265-277`) |
| `getSavedPublicMessages` | — | the port has no public messages; no Java screen reads them |
| `getFieldValidationErrors(String)` | — | not ported; no Java screen reads it, and Java's own `WebFacadeStub` answers null for it |
| `getScreenHistory` | `ec.web.getScreenHistory` | the same list, newest first; the port keeps the paths and rebuilds the map, so the `name` is the last path segment where Java has the menu name and the first two parameter values, and there is no `screenLocation` |
| `sendJsonResponse` | `ec.web.sendJsonResponse` | the same rules: a string is sent as it is, the messages become `messages`, the errors become `errors` and the status 500, a JSON-RPC answer keeps its own errors (`web/mod.rs:611-653`) |
| `sendJsonError(int, String, Throwable)` | `ec.web.sendJsonError` | the same `{"errorCode","errors"}` body and status; the throwable is not used in either |
| `sendTextResponse(String)` / `(String, String, String)` | `ec.web.sendTextResponse` | the same content type and the same attachment disposition with the ASCII file name; the port adds `;charset=UTF-8` where Java does not, sends no `Content-Disposition: inline` when there is no file name (Java sends one), and it does not have Java's error rule (P8-09) |
| `sendResourceResponse(String)` / `(String, boolean)` | `ec.web.sendResourceResponse(location, inline, fileName)` | the same 404 for a location that is not there, the same content type from the file name, the same inline and attachment dispositions; the port adds a file name argument and its own cache rule (P8-18) |
| `sendError(int, String, Throwable)` | — | missing (P8-07) |
| `handleJsonRpcServiceCall` | — | excluded on purpose (JSON-RPC) |
| `handleEntityRestCall` | `screen/dispatch.rs` (`e1`, `v1`, `m1`) | the same: the JSON parse error is a 400, no user is a 401, `X-HTTP-Method-Override` on a POST, the JSON list of objects is a batch, the page and run-time headers |
| `handleServiceRestCall` | `screen/dispatch.rs` (`s1`) | the same: the login error is a 401, the other errors a 500, the JSON parse error a 400, the batch list |
| `handleSystemMessage` | `service/system_sync/message.rs` | the same: the type and the remote are checked, the four auth modes (`SmatLogin`, `SmatHmacSha256`, `SmatHmacSha256Timestamp` with its five minute window and replay cache, `SmatNone`), the same HMAC values (`WebUtilities.hmacSha256Base64` and `hmacSha256Hex`, `service/system_sync/message.rs:378-401`) |
| `sendQzSignedResponse` (impl) | `ec.web.sendQzSignedResponse` | the same: no user is a 401, the Base64 `SHA1withRSA` signature as `text/plain`, a key that cannot be read a 500 |
| `saveScreenLastInfo` (impl) | `ec.web.saveScreenLastInfo`, `user::session::save_screen_last` | the same path and parameters in the session |
| `saveScreenHistory` (impl) | `user::session::push_screen_history` | the same list, newest first, without repeats, 40 entries |
| `addDeclaredPathParameter` (impl) | the screen's `~name=value` path parameters | the same: the name is not decoded, the value is (`screen/url.rs:616-645` against `ScreenUrlInfo.cleanupPathNameList:920-948`) |
| `getRequestDetails` (impl) | `web::request_details()` | the same shape: the request line, the scheme, the remote address, every header, every parameter; the port hides the `Authorization` and `Cookie` headers and leaves out the credential parameters |
| `makeNewSession` (impl) | the login rotation in `user/session.rs` | the same: a new session id, the attributes copied, a new token |
| `saveMessagesToSession`, `saveParametersToSession`, `saveRequestParametersToSession`, `saveErrorParametersToSession` (impl) | `user::session` | the same maps, taken once by the next request |
| `makeDiskFileItemFactory` (impl) | `multipart::Spool` | the same idea: the parts over 256 KiB go to a file under `runtime/tmp/upload`; the port adds limits Java has not (2000 parts, 16 KiB of part headers, 1 MiB of a text field), which is the hardening the web-protection design asks for |
| `viewEmailMessage` (impl) | — | the tracking pixel; Java says it is not an HTTP entry point |
| `runFirstHitInVisitActions`, `runBeforeRequestActions`, `runAfterRequestActions`, `runAfterLoginActions`, `runBeforeLogoutActions` (impl) | `screen/render.rs` | the webapp's lifecycle actions; the screens phase |

Other checks of this phase:

- `WebUtilities.isSameOriginRedirect` (`WebUtilities.java:423-453` against `web/mod.rs:463-499`): the
  same rules — empty, a backslash, a control character, `javascript:`, `data:`, `vbscript:` and a
  `//` target are refused, a path is accepted, an absolute URL is accepted only when it names the
  configured host, and a user info (`a@b`) is refused. The port falls back to the render's base URL
  when no configured host is given, where Java uses the webapp's `http-host` and so refuses every
  absolute URL when it is empty; for a real request the base URL is empty, so the two agree. The port's
  script binding takes the configured host where Java takes the request
  (`script/runtime/ec_extra.rs:691-708`), which the comment there says.
- `WebUtilities.ipMatches` (`WebUtilities.java:385-405` against `web/mod.rs:1327-1356`): the same
  comma separated patterns, the `*` and `*.*.*.*` wildcards, the four dot separated parts, the
  `low-high` ranges and the exact IPv6 match.
- `WebUtilities.canonicalizeClientIp` (`WebUtilities.java:330-355`): the port's `parse_hop`
  (`web/address.rs:107-128`) drops the brackets, the port and the zone, and reads a `Forwarded` style
  `for=` pair, as Java does.
- `getClientIp` (`UserFacadeImpl.groovy:967-1019` against `web/mod.rs:1303-1317` and
  `web/address.rs:130-216`): the same header of the conf, the same first hop when no proxy list is
  set, the same fall back to the socket peer. The port adds `trusted-proxies` with CIDR ranges and
  takes the rightmost hop that is not a trusted proxy; that difference is named in
  [Parity-Mapping, Web and the HTTP host](../Parity-Mapping.md#web-and-the-http-host) (`web/address.rs`,
  "Partial") and is a hardening.
- `WebUtilities.isExecutable` (`WebUtilities.java:752-779` against `multipart.rs:199-203`): the same
  headers — Windows PE, Linux ELF, Java class, macOS Mach-O and its byte swap, and the `#!` shebang —
  and a ZIP is not a program in either.
- `WebMediaTypeException` (`WebMediaTypeException.java:20-23`): the port has no exception type of its
  own for this; the executable upload is refused with a 415 and the same message
  (`screen/mod.rs:114-116`), which is the status Java's `MoquiServlet` gives it
  (`MoquiServlet.groovy:135-137`). The body shape differs, because the port has no `sendError`
  (P8-07).
- `WebUtilities.encodeHtmlJsSafe` and `fieldValuesEncodeHtmlJsSafe`
  (`WebUtilities.java:179-264` against `script/runtime/ec_extra.rs:610-658`): the same escapes, with
  the backslash and the U+2028/U+2029 escapes added, and a null argument is empty text in both.
- `WebUtilities.encodeHtml` (`WebUtilities.java:266-287` against `screen/sri/mod.rs:157-170`): the
  same HTML escapes, except for the control characters (P8-16).
- `WebUtilities.stripUserAccountSecrets`, `removeUserAccountSecretsFromMap`,
  `removeUserAccountIdentityFromMap`, `isUserAccountEntity`, `isIdentityAdminEntity`,
  `isSecretConfigEntity` and `isRestrictedGenericEntity` (`WebUtilities.java:496-551` against
  `entity/sensitive.rs`): the same field names, the same entity name classification, the same
  "restricted generic entity" set, and the script binding the Tools screens use
  (`script/runtime/tools_host.rs:806`). The port strips the secrets from a nested map as well, which
  Java does not (P8-19 is about the credential parameters, the same shape).
- `WebUtilities.hmacSha256`, `hmacSha256Base64`, `hmacSha256Hex` and `hmacSha256TimestampHeader`
  (`WebUtilities.java:667-695` against `service/system_sync/message.rs:378-401`): the same values, and
  the same header format `t=<seconds>,v1=<hex>` for the timestamped one.
- `WebUtilities.lastPathSegment` and `isSafeSinglePathSegment` (`WebUtilities.java:468-486` against
  `script/runtime/ec_extra.rs:687-717`): the same rules, including the backslash as a separator and
  the control characters.
- The multipart map (`WebFacadeImpl.groovy:161-237` against `web/mod.rs:1109-1145` and
  `multipart.rs`): the same shape — the fields by name, the files by name, `_fileUploadList` with all
  the files, a repeated name becomes a list — and the same part headers (`Content-Disposition` with
  the name and the file name, the quoted values with the escapes, `Content-Type`). A file input that
  was left empty posts nothing in both.
- The session cookie: the port has its own cookie names (`moqui.session`, `moqui.visitor`) and its own
  session row; that is README redesign 7, named in
  [Parity-Mapping, User and session](../Parity-Mapping.md#user-and-session) (`user/session.rs`,
  "Redesign").

#### Out of scope

- `getRequest`, `getResponse`, `getSession` and `getServletContext` (the servlet types): the port has
  its own request and response ([Parity-Mapping, Web and the HTTP host](../Parity-Mapping.md#web-and-the-http-host),
  "No servlet types").
- `handleJsonRpcServiceCall` and the JSON-RPC dispatcher: excluded on purpose
  ([Parity-Mapping, Excluded on purpose](../Parity-Mapping.md#excluded-on-purpose), "JSON-RPC").
- The FOP servlet's other content types and the `viewEmailMessage` tracking pixel: Java says the
  pixel is not an HTTP entry point, and the FOP path is a screens item.
- The servlet container's own body and header limits, and the `MoquiSessionListener` session bytes:
  excluded on purpose (Hazelcast and the session bytes) and the cluster design.

#### Open questions

- ~~Should the port refuse an absolute redirect target the way Java does when the webapp names no
  `http-host`?~~ — answered on 2026-10-20: yes, and more: a target that is not a path of this site
  goes to the root. The difference from Java is in
  [Parity-Diffs](Parity-Diffs.md#screen-and-web-review-2026-10-20).
- Should the CORS settings, the session-token setting and the executable-upload setting be read from
  the webapp element of the conf (P8-06)? The factory can read them
  (`ExecutionContextFactory::webapp_setting`), but the CORS headers themselves are hard-coded in
  `web/mod.rs` where Java reads the `cors-actual` and `cors-preflight` response headers of the conf.
- Should `ec.web.getWebappRootUrl(true, ...)` answer the request's own host, as Java does (P8-08)?
  The port's `ExecutionContextFactory::webapp_root_url` answers the conf's `http-host` and falls back
  to `localhost`, which is what the SSO callback URL is built from
  (`service/sso.rs:100-104`); with no `http-host` configured Java would answer the request's host.

## Phase 9 — The HTTP host and the web app endpoints

Compared `crates/moqui-host/src/{http,notws,search_proxy,logging,guard,tls,edge,lib}.rs` and
`crates/moqui-core/src/screen/mod.rs` against `MoquiServlet.groovy`, `MoquiAuthFilter.groovy`,
`MoquiContextListener.groovy`, `NotificationEndpoint.groovy`, `NotificationWebSocketListener.groovy`,
`MoquiAbstractEndpoint.groovy`, `ElasticRequestLogFilter.groovy`, `MoquiFopServlet.groovy`,
`ScreenResourceNotFoundException.groovy`, `framework/src/main/webapp/WEB-INF/web.xml`,
`framework/src/main/resources/MoquiDefaultConf.xml` and `log4j2.xml`.

Worked 2026-10-19: the route table was walked servlet by servlet (every URL pattern in
`MoquiDefaultConf.xml` and `web.xml` was checked against the axum router), then the request pipeline,
the auth filter rules, the websocket endpoint, the log filters and the startup; 15 findings, none High.

| ID | Rust location | Java/XML location | Divergence | Severity | Intentional? |
|---|---|---|---|---|---|
| P9-01 | `moqui-core/src/web/error_info.rs:96-104`, `moqui-core/src/screen/mod.rs:404` | `MoquiDefaultConf.xml:227-231`, `MoquiServlet.groovy:282-298` | The error page is chosen by a hardcoded table (`error/Unauthorized`, `error/Forbidden`, `error/NotFound`, `error/TooMany`, `error/InternalError`); the `<error-screen>` entries of the webapp node in the conf are merged but never read | Med | No |
| P9-02 | `moqui-core/src/context/conf.rs:164-174` | `ExecutionContextImpl.java:199,259`, `UserFacadeImpl.groovy:685,736`, `MoquiContextListener.groovy:199-204,222-227` | The webapp lifecycle actions (`before-request`, `after-request`, `after-login`, `before-logout`, `after-startup`, `before-shutdown`) are merged into the conf and never run | Med | No |
| P9-03 | `moqui-host/src/http.rs:208-221` | `MoquiContextListener.groovy:78-198`, `MoquiDefaultConf.xml:242-302` | The route table is fixed in code; the `<servlet>`, `<filter>` and `<endpoint>` declarations of the webapp node are merged (`conf.rs:180-188`) but nothing reads them, so a deployment that adds or re-paths a servlet gets a 404 where Java served it | Med | No |
| P9-04 | `moqui-core/src/web/mod.rs:1447-1453` | `MoquiServlet.groovy:227-232` | A request with an `Origin` that is not allowed is served normally (the CORS headers are simply left off); Java refuses it with 401 "Origin not allowed" | Med | No |
| P9-05 | `moqui-core/src/web/mod.rs:1378-1389,1371-1376,1095-1097` | `ExecutionContextFactoryImpl.groovy:1857-1874`, `MoquiDefaultConf.xml:220-224` | `handle-cors`, `allow-origins` and the other webapp settings are read as the `webapp_*` properties, not from the webapp element of the conf, so a component that overrides the element (not the property) has no effect | Med | No |
| P9-06 | `moqui-core/src/screen/mod.rs:107-109` | `MoquiServlet.groovy:52-178` | Every `OPTIONS` request answers 204 with an empty body, before the path is resolved; Java renders the screen (200) or answers 404 for a path that does not exist, and a CORS preflight answers 200 | Med | No |
| P9-07 | `moqui-core/src/user/authz.rs:518-523`, `moqui-core/src/screen/mod.rs:270-282` | `MoquiServlet.groovy:130-134` | The tarpit answer is 429 with the seconds in the message text, but no `Retry-After` header; Java adds the header | Med | No |
| P9-08 | `moqui-core/src/screen/render.rs:1167-1193` | `ScreenRenderImpl.groovy:443-457` | A transition that writes may take its `~name=value` parameters from the URL; Java refuses such a transition outright (a `BaseArtifactException`) because URL parameters are not protected | Med | No |
| P9-09 | `moqui-host/src/http.rs:585-596,763-828` | `ElasticRequestLogFilter.groovy:91-120,153-159`, `MoquiDefaultConf.xml:243-246` | The `moqui_http_log` entry has no `session`, `visitor_id` or `remote_user`, and only the screen requests are logged; Java's filter is on `/*` and logs every request with those fields | Med | No |
| P9-10 | `moqui-host/src/logging.rs:556-572` | `ElasticSearchLogger.groovy:100-116` | The `moqui_logs` event has no `user_id` and no `visitor_id` (the MDC fields) and no `thrown`; the port's own mapping declares `user_id` and `visitor_id` | Med | No |
| P9-11 | `moqui-host/src/search_proxy.rs:138-144` | `MoquiDefaultConf.xml:289-295` (Jetty `ProxyServlet$Transparent`, `prefix=/kibana`) | The `/kibana` proxy keeps the `/kibana` prefix in the path it forwards; Jetty strips the prefix, so Java sends `/app/dashboards` and the port sends `/kibana/app/dashboards` | Med | Unclear |
| P9-12 | `moqui-host/src/logging.rs:436-441` | `log4j2.xml:27-28` | The error log file is `moqui-error.log`; Java's is `error.log` | Low | No |
| P9-13 | `moqui-core/src/web/mod.rs:1429-1455` | `MoquiDefaultConf.xml:324` | The `X-XSS-Protection: 1; mode=block` header of a screen render is not sent | Low | No |
| P9-14 | `moqui-host/src/search_proxy.rs:182-188` | Jetty `ProxyServlet` (the client's headers are forwarded) | The proxy does not forward the client's `Authorization` or `Cookie` headers; Java forwards them (and with them the `moqui.session` cookie) | Low | Unclear |
| P9-15 | `moqui-host/src/search_proxy.rs:125-132` | `MoquiAuthFilter.groovy:80-90` | The 401 for the proxies carries a `WWW-Authenticate: Basic realm="Moqui"` challenge; Java sends no challenge, and its body is the message-facade text, not "login required" | Low | Unclear |

#### P9-01 — The error page is a hardcoded table, not the conf's `<error-screen>`

Java looks the error screen up in the webapp info of the conf: `MoquiServlet.sendErrorResponse` asks
`ecfi.getWebappInfo(moquiWebappName)?.getErrorScreenNode(errorType)` for the `error` kind
(`unauthorized`, `forbidden`, `not-found`, `too-many`, `internal-error`) and renders the screen at
its `screen-path`, with `errorCode`, `errorType`, `errorMessage`, `errorThrowable` and
`showErrorDetail` in the context (`MoquiServlet.groovy:282-298`). The default conf names the five
paths (`MoquiDefaultConf.xml:227-231`), and a deployment may point them elsewhere.

The port has the same five paths in a `match` in `web::error_info::error_screen(status)`
(`moqui-core/src/web/error_info.rs:96-104`), used by `screen::error_page`
(`moqui-core/src/screen/mod.rs:404`). The conf's `<error-screen>` elements are merged
(`context/conf.rs:161`) and never read, so a deployment that overrides one of them gets the stock
`error/Unauthorized` page in the port and its own page in Java. The fix is to read the element
from the webapp node (the conf cache already holds the webapp children) and fall back to the
default path.

#### P9-02 — The webapp lifecycle actions never run

Java runs six webapp-scoped action sets: `before-request` and `after-request` around every request
(`ExecutionContextImpl.java:199` and `:259`), `after-login` and `before-logout` around a login and a
logout (`UserFacadeImpl.groovy:685,736`), and `after-startup` and `before-shutdown` around the
webapp's life (`MoquiContextListener.groovy:199-204` and `:222-227`). The elements live in the
webapp node of the conf.

The port merges exactly those elements into the conf (`context/conf.rs:164-174`, so the names are
known) and then never runs them: there is no read of `before-request` or any of the others anywhere
in `crates/`. A deployment that uses `<before-request>` to set a request-scoped value, or
`<after-startup>` to warm something, gets nothing in the port and no warning that the hook is
missing. The fix is to run the `<actions>` of the element in the request worker (before and after
the render) and at boot and shutdown.

#### P9-03 — The route table is fixed in code; the conf's servlets and filters are inert

Java's `MoquiContextListener` builds the whole route table from the webapp node of the conf: the
filters (`MoquiContextListener.groovy:78-112`), the listeners (`:114-125`), the servlets with their
`url-pattern`s and `init-param`s (`:127-157`) and the websocket endpoints (`:170-197`). The default
conf declares `MoquiServlet` on `/*`, `MoquiFopServlet` on `/fop/*`, the `ElasticSearchProxy` on
`/elastic/*`, the `KibanaProxy` on `/kibana/*`, the A2A and LLM servlets and the `/notws` and
`/groovysh` endpoints (`MoquiDefaultConf.xml:266-302`).

The port's router is a literal list (`moqui-host/src/http.rs:208-221`): `/health`, `/metrics`,
`/notws`, `/elastic`, `/kibana`, `/git` (feature) and a fallback for everything else, which is the
screen dispatcher and carries the `/fop/*` path (`moqui-core/src/screen/mod.rs:241-254,316-320`).
The conf's `<servlet>`, `<filter>` and `<endpoint>` elements are merged
(`context/conf.rs:180-188`) but no code reads them. So a deployment that mounts the FOP servlet at
`/print/*`, or adds a servlet of its own, serves that URL in Java and gets the screen tree's 404 in
the port. The tree's own extra servlets (`H2Console`, `HtmlRenderServlet`) are excluded features,
so this bites only a deployment that customises the framework's own mappings; the fix is to read
the `url-pattern`s of the known servlets from the conf and build the routes from them.

#### P9-04 — A disallowed `Origin` is served, not refused

Java answers a cross-origin request whose origin is not allowed with 401 "Origin not allowed"
(`MoquiServlet.groovy:227-232`, inside `handleCors`, which runs before the screen render and before
the error screen). The same-origin case is allowed through Groovy's `==` (`originDomain == serverName`,
`:199`), so a same-origin POST is not refused.

The port's `finish_response` only adds the CORS headers when the origin is allowed
(`moqui-core/src/web/mod.rs:1447-1453`); when it is not, the request goes on and is answered
normally, with the body. A browser still hides the answer from the page (there is no
`Access-Control-Allow-Origin`), but a non-browser client — a script, a native app, `curl` — gets the
full content with a 200 where Java gets a 401. The fix is to answer 401 for a request with an
`Origin` header that is not allowed, as `handleCors` does, before the screen runs.

#### P9-05 — The webapp element's settings are read as properties

Java reads the CORS and token settings from the webapp element of the conf
(`ExecutionContextFactoryImpl.groovy:1857-1874`: `allow-origins` into `allowOriginSet`,
`handle-cors`, `require-session-token`, `client-ip-header`, `upload-executable-allow`), and the
default conf fills those attributes from the `webapp_*` properties
(`MoquiDefaultConf.xml:220-224`). A component that overrides the element in its own
`MoquiConf.xml` therefore changes the behaviour in Java.

The port reads the properties instead: `webapp_handle_cors` and `webapp_allow_origins`
(`moqui-core/src/web/mod.rs:1378-1389`), `webapp_require_session_token` (`:1371-1376`) and
`webapp_upload_executable_allow` (`:1095-1097`). The factory already has the webapp element's
attributes cached and readable (`context/factory.rs:550-556`, `webapp_setting`), and
`address.rs:149` uses it for the client-ip header, so the other three are a one-line change each.
With the default conf the two agree, so this only bites a deployment that overrides the element.

#### P9-06 — Every `OPTIONS` request answers 204

Java has no generic `OPTIONS` handling: `MoquiServlet` renders the screen for the method, so an
`OPTIONS` on a screen path answers 200 with the screen, and one on a path that does not exist
answers 404. A CORS preflight is answered 200 with the preflight headers
(`MoquiServlet.groovy:236-245`).

The port answers 204 with an empty body for every `OPTIONS`, before the body is parsed and before
the path is resolved (`moqui-core/src/screen/mod.rs:107-109`). A client that uses `OPTIONS` to
discover what a path supports learns nothing, and a path that does not exist cannot be told apart
from one that does. The preflight status is 204 where Java sends 200 (a browser accepts both). The
fix is to answer 200 for a preflight (an `OPTIONS` with `Access-Control-Request-Method`) and to
resolve the path for the other `OPTIONS` requests.

#### P9-07 — The tarpit answer has no `Retry-After`

Java catches `ArtifactTarpitException` in `MoquiServlet.service` and answers 429 with the
`Retry-After` header set to the exception's retry seconds (`MoquiServlet.groovy:130-134`); the FOP
servlet does the same (`MoquiFopServlet.groovy:127-131`).

The port raises the tarpit as an error of kind `Tarpit` with the seconds in the message
(`moqui-core/src/user/authz.rs:518-523`) and maps the kind to 429
(`moqui-core/src/web/mod.rs:1543-1554`), but the answer carries no `Retry-After` header: the error
page path writes the render's headers and the JSON path writes only `Content-Type`
(`moqui-core/src/screen/mod.rs:270-282`, `moqui-core/src/web/mod.rs:1527-1536`). A client that is
refused for hitting an artifact too often cannot tell when to try again and has to parse the
message. The fix is to carry the retry seconds on the error and add the header for a 429, as the
guard already does for its own rate limits (`moqui-host/src/guard.rs:707,837-841`).

#### P9-08 — A write transition may take its parameters from the URL

Java refuses to run a transition that has actions (or a single service) and is not `read-only`
when the request is not secure while https is enabled, **or** when the URL carries a query string,
**or** when it carries a `~name=value` path parameter
(`ScreenRenderImpl.groovy:443-457`; the path parameters are parsed by
`WebUtilities.getPathInfoParameterMap`). The reason is in the message: the parameters are in the
URL, so they are not encrypted and cannot be checked as coming from the application.

The port's `insecure_transition` checks the https rule and the query string, but not the
`~name=value` parameters (`moqui-core/src/screen/render.rs:1167-1193`), even though the port parses
them into `ScreenUrlInfo::inline_parameters`
(`moqui-core/src/screen/url.rs:616-645`, `:66-67`). It also allows `lastStandalone` and
`renderMode` in the query, which Java does not. The consequence is a cross-origin trigger: a link
or an `<img>` to `/Some/Transition/~orderId=123` runs the write in the port and is refused by
Java. The fix is to refuse the transition when `inline_parameters` is not empty, as Java does.

#### P9-09 — The HTTP request log has no session, visitor or user, and misses the other routes

Java's `ElasticRequestLogFilter` is registered on `/*` (`MoquiDefaultConf.xml:243-246`), so it sees
every request the container serves, and each entry carries `remote_user`, `session` (the HTTP
session id) and `visitor_id` (`ElasticRequestLogFilter.groovy:153-159`; the mapping is at `:80-88`).

The port's `RequestLog` (`moqui-host/src/http.rs:763-828`) writes `@timestamp`, `remote_ip`,
`server_ip`, `content_type`, `request_method`, `request_scheme`, `request_host`, `request_path`,
`request_query`, `http_version`, `response`, `time_initial_ms`, `time_final_ms`, `bytes`,
`referrer` and `agent` — and never `remote_user`, `session` or `visitor_id`, although the port's
own index mapping declares all three (`moqui-core/src/search/logs.rs:55-72`). The entry is also
only built in `dispatch_screen` (`moqui-host/src/http.rs:585-596,641`), so the `/notws`,
`/elastic`, `/kibana`, `/health` and `/metrics` requests are not logged at all. An operator cannot
tie a request in `moqui_http_log` to the user or the session that made it, and cannot see the
proxy or websocket traffic. The fix is to fill the three fields from the bound session and to build
the entry for every route, not only the screen ones.

#### P9-10 — The application log event has no user, visitor or throwable

Java's `ElasticSearchLogger` writes the MDC's `moqui_userId` and `moqui_visitorId` as `user_id`
and `visitor_id` and the exception as `thrown`
(`ElasticSearchLogger.groovy:100-116`); the MDC is set per request in
`ExecutionContextImpl.java:202-204`.

The port's log event carries `@timestamp`, `level`, `thread_name`, `thread_id`, `logger_name`,
`message`, `source_host` and the `mdc` map of the tracing fields
(`moqui-host/src/logging.rs:556-572`); there is no `user_id`, no `visitor_id` and no `thrown`. The
port's index mapping declares `user_id` and `visitor_id`
(`moqui-core/src/search/logs.rs:45-46`), so a search over `moqui_logs` for a user finds nothing.
The fix is to add the user and the visitor of the current context to the event (the port has no
MDC, so the fields have to be carried on the event).

#### P9-11 — The `/kibana` proxy keeps the prefix that Jetty strips

Both proxy servlets are Jetty's `ProxyServlet$Transparent` with a `prefix` init-param
(`MoquiDefaultConf.xml:283-295`: `prefix=/elastic` and `prefix=/kibana`). Jetty's transparent
delegate rewrites the target as `proxyTo + requestURI.substring(prefix.length())`, so the prefix is
stripped: Java sends `/kibana/app/dashboards` to Kibana as `/app/dashboards`.

The port strips the prefix for `/elastic` and keeps it for `/kibana`
(`moqui-host/src/search_proxy.rs:138-144`), which the module doc presents as "as the servlets'
prefix settings do" (`search_proxy.rs:20-22`) — they do not: Jetty strips both. The two therefore
reach the upstream with different paths. Which one works depends on the Dashboards configuration:
the Java comment says to set `server.basePath: "/kibana"` (`MoquiDefaultConf.xml:291`), which the
port's form satisfies and Java's does not; a Dashboards without a base path is the other way
round. The fix is to strip the prefix for `/kibana` as Jetty does (or to keep it and drop the
base-path instruction), and to correct the module doc.

#### P9-12 — The error log file has a different name

Java's log4j2 writes `moqui.log` and `error.log` into `${moqui.runtime}/log` (`log4j2.xml:18-28`).
The port writes `moqui.log` and `moqui-error.log` into the directory `moqui_log_dir` names
(`moqui-host/src/logging.rs:436-441`, the default `runtime://log` in
`runtime/base-component/framework/conf/MoquiDefaultConf.xml:306`). The directory and the all-events
file match; the error file does not, so a deployment or a log shipper that looks for
`runtime/log/error.log` finds nothing. The fix is to name the second file `error`.

#### P9-13 — `X-XSS-Protection` is not sent

Java's conf sets `X-XSS-Protection: 1; mode=block` on every screen render
(`MoquiDefaultConf.xml:324`), next to the CSP and the `X-Frame-Options` the port does send. The
port's `finish_response` sends `Cache-Control`, `Content-Security-Policy`, `X-Frame-Options`,
`X-Content-Type-Options` and (over https) `Strict-Transport-Security`
(`moqui-core/src/web/mod.rs:1429-1455`) but not the XSS header. The header is ignored by current
browsers, so the effect is on scanners only. The fix is one line in `finish_response`, or a
decision to leave it out on purpose and record it in Parity-Diffs.

#### P9-14 — The proxy does not forward the client's credentials

Java's proxy forwards the client's headers, so a request to `/elastic` or `/kibana` arrives at the
upstream with the client's `Authorization` header and the client's cookies — including the
`moqui.session` cookie. The port drops both (`moqui-host/src/search_proxy.rs:182-188`) and, for
Elasticsearch, adds the configured `elasticsearch_user`/`elasticsearch_password` instead
(`:189-195`).

Dropping the session cookie is a good thing (Java hands its own session cookie to a third-party
service), but a Dashboards behind a proxy that authenticates with the forwarded header, or an
Elasticsearch that expects the client's own credentials, cannot be reached through the port. The
fix is to forward the `Authorization` header (and only that) when no configured credential is set.

#### P9-15 — The proxy's 401 carries a Basic challenge Java does not send

Java's `MoquiAuthFilter` answers an unauthenticated request with `sendError(401, message)` and no
challenge (`MoquiAuthFilter.groovy:80-85`); there is no `WWW-Authenticate` header anywhere in the
Java framework. The port answers the same case with `WWW-Authenticate: Basic realm="Moqui"` and
the body "login required" (`moqui-host/src/search_proxy.rs:125-132`), and the port's
`check_permission` does accept the Basic credentials that the challenge invites
(`moqui-core/src/screen/mod.rs:485-527`).

The difference is visible: a browser that hits `/elastic` without a login gets a 401 error page in
Java and a username/password prompt in the port. The port's flow is the more useful one, but it
is not recorded anywhere, so it should be either kept and recorded or matched to Java.

#### Phase 9 checked and found equivalent

- **The route table.** Every URL pattern the Java side serves was checked against the port's
  router (`moqui-host/src/http.rs:208-221`): `/*` → the screen dispatcher (the fallback,
  `http.rs:220-221` → `moqui-core/src/screen/mod.rs:89-135`), `/fop/*` → the same dispatcher
  through `fop_screen_path` (`screen/mod.rs:241-254,316-320`, Java's `MoquiFopServlet`), `/notws` →
  `notws::upgrade` (`http.rs:211`), `/elastic/*` and `/kibana/*` → the proxy
  (`http.rs:212-217`, Java's `ElasticSearchProxy`/`KibanaProxy`). The port adds `/health`,
  `/metrics` and `/git/*`, which have no Java counterpart. The LLM, A2A and `groovysh` routes are
  excluded (see below).
- **The screen request pipeline.** Java's servlet thread does the render inside
  `MoquiServlet.service` (`MoquiServlet.groovy:52-178`); the port does it on the factory's request
  pool (`http.rs:597-647`, `moqui-core/src/service/request_pool.rs`), with the screen's own
  transaction begun and ended by the render (`moqui-core/src/screen/render.rs:1740-1759`, Java's
  `ScreenRenderImpl` `beginTransaction`/`commit`).
- **The error page of a failed screen.** Java's `sendErrorResponse` renders the `error/*` screen
  with `errorCode`, `errorType`, `errorMessage`, `errorThrowable` and `showErrorDetail`
  (`MoquiServlet.groovy:279-298`); the port renders the same screen with the same context
  (`moqui-core/src/screen/mod.rs:385-449`), and only for a client that takes html and a path that
  is not an API path, as Java's `Accept: text/html` check does.
- **The status of a failed screen.** Java maps `AuthenticationRequiredException` to 401,
  `ArtifactAuthorizationException` to 403, `ScreenResourceNotFoundException` to 404,
  `ArtifactTarpitException` to 429 and `WebMediaTypeException` to 415
  (`MoquiServlet.groovy:119-137`); the port maps its error kinds the same way
  (`moqui-core/src/web/mod.rs:1543-1554`), with the 415 for an executable upload
  (`moqui-core/src/screen/mod.rs:114-116`).
- **The `moquiSession` cookie.** Java's session cookie is the container's, with `HttpOnly` and
  `SameSite=Lax` set by `MoquiContextListener` (`MoquiContextListener.groovy:161-168`) and `Secure`
  on a secure request; the port's `moqui.session` cookie carries `HttpOnly; SameSite=Lax; Path=/`
  and `Secure` over https (`moqui-core/src/user/session.rs:351-366`). The `moqui.visitor` cookie
  has its one-year `Max-Age` in both (`UserFacadeImpl.groovy:223-228`).
- **The session token.** Java creates `moqui.session.token` on the first request of a session and
  sends it as the `moquiSessionToken` and `X-CSRF-Token` headers
  (`WebFacadeImpl.groovy:210-218`); the port sends the same two headers when the session is new or
  the token changed (`moqui-core/src/screen/mod.rs:451-476`). The CSRF check of a write transition
  matches Java's rule (a non-GET method, a transition that runs something, the token from the
  parameter or the header, and the header-authenticated requests are exempt):
  `moqui-core/src/screen/render.rs:1778-1811` against `ScreenRenderImpl.groovy:458-475`.
- **`MoquiAuthFilter`.** Java's rules are: an `OPTIONS` request is answered by `handleCors` before
  the user is read (`MoquiAuthFilter.groovy:64-68`), the session, the Basic header, the `api_key`
  and `login_key` headers and the `authUsername`/`authPassword` parameters log a user in
  (`UserFacadeImpl.groovy:96-184`), no user is 401 and a missing permission is 403
  (`MoquiAuthFilter.groovy:80-90`). The port's `check_permission`
  (`moqui-core/src/screen/mod.rs:546-579`) binds the session, runs the same `authenticate`
  (`screen/mod.rs:485-527`) and answers 401 without a user and 403 without the permission
  (`moqui-host/src/search_proxy.rs:123-136`); the `ElasticRemote` and `KibanaRemote` permissions
  are the ones Java's filter declares (`MoquiDefaultConf.xml:248-255`).
- **The screen auth rules.** Java's `checkWebappSettings` (`ScreenRenderImpl.groovy:998-1040`)
  refuses a screen with `allow-web-request="false"`, redirects a screen that needs a user to the
  last `login-path` of the path, and answers 401 for a `lastStandalone` or a transition; the port
  has the same three rules (`moqui-core/src/screen/render.rs:750-763,798-834,839-853`), takes the
  last `login-path` of the path as Java does, and sends the same 401.
- **The websocket endpoint.** The path is `/notws` in both (`MoquiDefaultConf.xml:301`); the
  handshake checks the `Origin` against the allow list and the `Host`
  (`MoquiContextListener.groovy:254-282`, Java's `WebUtilities.webSocketOriginAllowed`), which the
  port does with the same rules (`moqui-host/src/notws.rs:85-91`,
  `moqui-core/src/web/mod.rs:1394-1415`); the user comes from the session of the handshake
  (Java's `initFromHttpSession`, `MoquiAbstractEndpoint.groovy:81-96`; the port reads the session
  row, `notws.rs:122-154`); the `subscribe:` and `unsubscribe:` commands and the `ALL` topic are
  the same (`NotificationEndpoint.groovy:44-62`, `NotificationWebSocketListener.groovy:69-83`; the
  port's `notws.rs:242-278,328-363`); the wrapped message has the same fields
  (`NotificationMessageImpl.groovy:458-463`; the port's `notify/message.rs:594-658`); the idle
  timeout is the endpoint's hour (`MoquiDefaultConf.xml:301`, `notws.rs:52-53,179`); and
  `markSent` is called for the user that got the message.
- **The log files.** `moqui.log` in `runtime/log`, rolled by size and by day, with the warn and
  error lines in the second file (`moqui-host/src/logging.rs:419-469`, Java's `log4j2.xml:18-36`).
  The event log ships to `moqui_logs` and the request log to `moqui_http_log` through the search
  backend, as Java's `ElasticSearchLogger` and `ElasticRequestLogFilter` do
  (`moqui-host/src/logging.rs:114-158`, `moqui-core/src/search/logs.rs:26-28`).
- **The startup.** Java's `MoquiContextListener.contextInitialized` builds the factory, the webapp
  filters, listeners and servlets, the session cookie config, the websocket endpoints and the
  after-startup actions, and `contextDestroyed` runs the before-shutdown actions and destroys the
  factory (`MoquiContextListener.groovy:57-235`). The port's `boot_server_factory`
  (`moqui-host/src/lib.rs:985-1169`) builds the factory, the datasource, the entity facade, the
  search, the tools, the request pool, the cluster and the notification push, and the serve thread
  flushes the artifact hits and the sessions and stops the log shipping at the end
  (`lib.rs:942-961`), which is Java's `ecfi.destroy()`.

#### Out of scope

- `LlmServlet`, `A2AServlet`, `A2ACardServlet` and the `LlmAuthFilter`: the LLM and A2A endpoints
  are deferred (Parity-Mapping, "Excluded on purpose"; Remaining-Work, "Deferred on purpose").
- `GroovyShellEndpoint` at `/groovysh`: the SSH admin shell replaces it (Parity-Mapping, "Excluded
  on purpose").
- `MoquiSessionListener`: the port does not copy session bytes (Parity-Mapping, "Hazelcast and
  session-byte replication").
- `MoquiFopServlet`'s other content types: the port's `/fop` makes only PDF, recorded in
  Parity-Diffs, "Print fixes (2026-10-18)", `/fop`.
- The `H2Console` and `HtmlRenderServlet` servlets of the Java runtime conf: H2 and the FOP
  component are excluded (Parity-Mapping, "Databases not in this tree" and "FreeMarker, GString,
  and FOP as engines").
- The `web.xml` `JakartaFileCleaner` listener and the `session-timeout` of the container session:
  the port keeps its own sessions (README redesign 7).

#### Open questions

- Should the port read the webapp's `<servlet>`, `<filter>` and `<endpoint>` declarations from the
  conf (P9-03), or is the fixed route table the intended design? If it is the design, the conf
  merge of those elements (`context/conf.rs:180-188`) should stop, so that a deployment does not
  believe a servlet it never registered.
- The `/kibana` prefix (P9-11): should the port keep the prefix it keeps now (which matches the
  `server.basePath: "/kibana"` the Java conf comment asks for) or strip it as Jetty does? Either
  way the module doc of `search_proxy.rs` has to match.
- The `X-XSS-Protection` header (P9-13): send it as Java does, or leave it out and record the
  decision in Parity-Diffs?

## Cross-cutting threads

These are the threads the phases found, in the order the work should take them. They are the
screen-and-web half of [Parity-Mapping, Cross-cutting threads](Parity-Mapping.md#cross-cutting-threads).

1. **The webapp conf the port parses but does not read.** P9-01 (the error screen), P9-02 (the
   webapp lifecycle actions), P9-03 (the servlets and filters the conf declares), P9-05 (the
   webapp element's settings), P7-03 (the `screen-resource-*` response headers) and P8-06 (the CORS,
   session-token and upload settings) are all the same thing: the port merges the Java webapp conf
   and then reads only part of it. One pass over the `<webapp>` element would close six findings.
2. **The redirect path.** P8-01 and P8-02 are fixed (2026-10-20: Java's `sendHttpRedirect` rule is
   in `http_redirect_location`). P7-02 (the link permission check) and P7-08 (a plain `url-type` is
   prefixed with the base URL) are what is left of the thread: where a location comes from and what
   may be done with it.
3. **The render-mode policy.** P2-08 (a mode the screen does not name is refused), P3-01 (an
   element no mode names fails the render), P6-06 (a direct `.xsl-fo` request answers HTML) and
   P7-09 (`skip-actions` is not read) are the port's render-mode rules meeting Java's, which has
   none. The port should decide the policy once and record it in
   [Parity-Diffs](Parity-Diffs.md), then make the four findings point at that decision.
4. **The saved finds.** P1-05 (the implicit saved-find transitions need a POST and a session
   token), P1-06 (a dynamic form's saved finds are not keyed by the entity or service), P4-05 (the
   Saved Finds dialog has no schedule action) and P4-02 (no way to clear the current find) are one
   feature: the saved find as a user-visible object. Phase 1 compared the statics; the UI side is
   in Phases 4 and 5.
5. **The static qvt's drift from the dynamic qvt.** P5-02 (the form's `hidden-parameters` are
   dropped), P5-03 (the browser validation rules are dropped), P5-04 (a form the user may not use
   is rendered enabled), P5-05 (`pass-through-parameters` is dropped), P5-06 (the report's "clean"
   verdict ignores the full check), P5-07 (a server-static screen is not in the screen history) and
   P5-08 (`all-checked` is dropped) are the static mode losing behavior the dynamic mode has. The
   report does not know about them, so the linter cannot catch the next one.
6. **The form fields.** P1-01 (an auto field of a related entity is a text box), P1-02 (the auto
   `fromDate` field has no default), P1-12 (a new field with a layout sequence is appended),
   P3-06 (the display span lost the field-value class) and P6-01 (the xlsx cell of a conditional
   field) are the auto-fields and the field attributes.

## What this review does not cover

- The **screen XML dialect** itself: the port's XML screen language is its own (the `runtime/`
  `xml-screen-3.xsd`), and Phase 1 compared the two XSDs only for the elements the port's code
  reads. A screen that works in Java and not in the port is often a dialect difference, not a
  finding, and the IronStone screens are the port's dialect already.
- The **client library** (`runtime/base-component/webroot/screen/webroot/js/`), the Quasar widgets
  and the CSS. The review compared the server's output to the client's contract where a finding
  turned on it (P4-01 was one such), but the client was not reviewed on its own.
- The **screen services** the framework XML declares (`ScreenServices.xml`, `WikiServices.xml`):
  the entity and service review covered the service layer, and the screens that call them are in
  scope here only where the call is made.
- The **Mantle and other application screens**. They are screens, but they are the port's dialect
  and they are IronStone's work, not the framework's.

## Next steps

1. ~~Decide the three High findings~~ — done on 2026-10-20: P5-01, P7-01 and P8-01 are fixed, with
   P8-02. The plan and the record of the decisions are in
   [ScreenWeb-High-Findings](ScreenWeb-High-Findings.md).
2. Sort the 27 `Unclear` findings into "deliberate, record it in Parity-Diffs" and "a gap". The
   `Unclear` ones are the ones that will otherwise be re-found.
3. Work the cross-cutting threads above, in the order given. P8-01 and P8-02 are in; P7-02 and
   P7-08 remain on the redirect path. The webapp conf and the render-mode policy follow.
4. When the findings are fixed, move the accepted differences into
   [Parity-Diffs](Parity-Diffs.md) with their `P<N>-<n>` IDs, leave a pointer here, and archive
   this file under `plans/archive/` the way
   [EntityService-Parity-Review](archive/EntityService-Parity-Review.md) was archived.
