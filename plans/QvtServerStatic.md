# qvt Server-Static

How a screen marked server-static for `qvt` is sent as a cacheable Vue template, and how the browser fills it. The XML screen schema stays. The `server-static` attribute stays the switch.

This follows Screen Render Modes in `plans/Architecture-Proposal.md`. The library choices there stand: Vue 3.5, Quasar 2.35, the qvt JavaScript macro module emitting a component object, no jQuery, no Moment, no `http-vue-loader`. This plan is the data split, the browser host, and the compatibility check.

The Java behavior this is built on lives in `../moqui`: `ScreenDefinition` (`serverStatic`, `ActionsTransitionItem`), `ScreenRenderImpl.doActualRender`, `ScreenForm.FormInstance.isServerStatic`, `DefaultScreenMacros.qvt.ftl`, and `WebrootVue.qvt.js`.

## Decision

`server-static` remains a list of render modes, or `all`. A screen is server-static for `qvt` only when the attribute includes `qvt` or `all`, or when the screen is the existing automatic case (widgets are only `subscreens-panel`, `subscreens-menu`, and `subscreens-active`). Forms inherit the screen value and can override it. This pass does not make `qvt` static by default. A later pass can invert that so `qvt` is server-static unless the attribute says otherwise. That waits on the compatibility reports, read with the limit in Compatibility check.

When the active mode is in the set, the qvt macros emit a Vue template with no request data in it. The response uses the existing `screen-server-static` cache header (`max-age=86400, must-revalidate, public`). The client caches that component by screen path, as `loadComponent` already does when `Cache-Control` contains `max-age`. The cache entry is the template. It does not vary by user, locale, or parameters. Literal strings are localized in the browser. `GET {screen}/actions` and `GET {screen}/actions/{formName}` are per user. Their responses are `Cache-Control: private, no-store`.

When the active mode is outside the set, qvt stays a server merge, as the current macros do: values expanded into the template, form fields seeded from `fields-initial`. `html`, `html-print`, `text`, `csv`, and `xml` stay server merges. The name `xsl-fo` is `html-print`. Marking one of those modes server-static still sends the cache header, and it is only valid for output that does not vary. The automatic subscreens shell is that case. A content screen marked `server-static="html"` is a stale-cache bug, and the compatibility report calls it out.

The client loads two kinds of JSON after the template.

- `GET {screen}/actions` is the screen package. It is the context after data prep, with every form-list's list left out.
- `GET {screen}/actions/{formName}` is that form-list. Every form-list on a static screen uses this request, paginated or not.

The context object keeps the shape expressions already walk. The browser adds a separate `ec` host beside that tree. `ec.l10n` formatting, a small `ec.user` snapshot, and `ec.resource.expand` are implemented in the browser. Any other property of `ec`, and any use of `sri`, is absent and throws when an expression touches it.

The schema does not grow a second template language and does not grow attributes that mark an expression as client-safe. Compatibility is a static scan of each transcribed expression, plus a render that evaluates every widget expression against real actions. A clean report is necessary before a screen is marked `server-static="qvt"`. It is not sufficient to invert the default: the render only sees the rows that run produced.

## Screen package and form-list package

Both requests walk the screen path. `always-actions` run for every screen in the path, from index 0. `pre-actions` start at `renderPathDifference`, except when the render mode is `always-standalone` and the path has more than two screens: qvt is that mode, and then `pre-actions` start at index 2 (webroot and apps are skipped). That is the fork in `ScreenRenderImpl.doActualRender`. Parent shells such as `PopCommerceAdmin` put organization and app fields into the context there. Each request is self-contained. The form request does not require the screen request to have run first.

`GET {screen}/actions` then runs the root section actions and the actions of named sections and of `section-iterate`. Section-iterate runs per entry on the server, and the list in the package is the list after those per-entry actions. The response body is that context, unwrapped the way `ContextJavaUtil.unwrapMap` unwraps it: entity values become plain maps, and `EntityFind`, the execution context, the screen renderer, and the context stack are dropped. Credential parameters are stripped with `stripCredentialParameters`, the same helper the JSON redirect and the basic response map already use. Java's `/actions` transition sends `unwrapMap` and does not strip. Stripping here is a behavior change, and this plan keeps it.

Before the body is written, every form-list on the screen is removed from it. The removed keys are the form's `@list` name and the pagination keys the renderer would have set: `{list}Count`, `{list}PageIndex`, `{list}PageSize`, `{list}PageMaxIndex`, `{list}PageRangeLow`, `{list}PageRangeHigh`. A list that screen actions prepared for a form-list is still prepared, and then dropped from this response. A `section-iterate` whose list name is not a form-list `@list` stays in the package. A `section-iterate` whose list name is a form-list `@list` loses that list from the screen package, and the compatibility report records it, the same way it records a label outside the form-list that reads the list. Tree node JSON stays on `GET {screen}/actions/{treeName}`.

`GET {screen}/actions/{formName}` builds that one list.

- A form with its own `entity-find` runs that find, including `search-form-inputs` pagination.
- A form whose `@list` is a context name runs the screen actions that populate it, then reads the list. This is the current Java fallback, kept.
- `row-actions`, `field.@from`, and aggregation run on the server, as `getListObject` does today. The rows in the response are those maps, plain values, in the same shape a row has when it is pushed onto the context. The response does not use the `name_display` projection that `getFormListRowValues` builds for the current qvt field object.

The body is one map: the list under the `@list` name, the pagination keys under the names above, the header field values, and the saved-find and column-selection records when those features are on. Search, page, and order parameters arrive on the query string, as they do now.

Widget lookups that the browser host cannot do are resolved while building the package, and stored next to the author's fields.

- `display-entity` is resolved on the server. The display text is stored on the row or the form map as `{name}_display`. The author's field value stays as it is.
- `entity-options` are resolved on the server. The option list is stored under a key the macro reserves for that field. `list-options` that name a list already in the context are left as that list. The template evaluates `@key` and `@text`.
- `dynamic-options` has two paths, and the port keeps both. When `server-search` is not set, Java renders the named transition with `ScreenTest` during the screen render and fills the option map. Server-static does that resolve while building the package and stores the options next to the field. When `server-search` is set, the macro already emits `options-url` as the transition URL, and the client fetches it. That stays a client fetch.
- `<set>` inside a field runs on the server during the request that owns the scope (the screen package for a form-single or a loose widget, the form package for a form-list row or header). The template receives the map after those sets.

Running the screen actions for both requests means a screen whose actions fill a form-list pays for that work twice. This pass accepts that cost. The second run of `always-actions` and `pre-actions` is introduced by this split. Today's qvt form-list is one server render. Java's form `/actions` branch does not run those blocks again; the render path already ran them once before the transition. A write, a send, or a message appended from `always-actions` or `pre-actions` happens on the screen request and again on the form request, and the form request does not require the screen request to have run first. A screen with those side effects is unsafe to mark `server-static="qvt"` until the form request stops repeating that work. The compatibility report records actions in those blocks that write, send, or append a message. A later pass can skip list-producing work on the screen request, and that is the pass that removes the double side effect.

The form-list header, order-by, page size, and the CSV, text, and PDF links bind to the form package. A label outside the form-list that reads `{list}Count` or the list itself is reported by the compatibility check. Those keys exist on the form scope.

## What the template evaluates

Static qvt macros transcribe the screen. They do not call `expand` or `condition` on request data.

Expressions are JavaScript, the same language the server evaluator uses. A decimal in the package is a string, as `plans/ContextObjects.md` specifies. An expression that does decimal arithmetic, or that calls `ObjectUtilities` for it, is a compatibility-report violation. Formatting goes through `ec.l10n`. In the template, expressions run against a scope chain that matches the server context stack:

- The screen template sees the screen package.
- `form-single` pushes `@map` (default `fieldValues`).
- `section-iterate` pushes `@entry` and, when set, `@key`.
- Each form-list row slot pushes that row on top of the screen package plus the form package keys.
- `ec` is in scope at every level and is not a key in the package.

Conditions that choose structure become `v-if` on that scope: `section.@condition`, `conditional-field.@condition`, `field.@hide`, `link.@condition`, `label.@condition`, `container-dialog.@condition`, `image.@condition`. Every branch is in the template. `section-iterate` becomes `v-for` over `@list`. `visible-when` stays a client expression, transcribed from `@field`, `@from`, and `@value` without evaluating them on the server.

Formatting that the server applies inside `expand` and inside display widgets is a call on `ec.l10n` in the template. `@format`, currency attributes, and an author expression that already calls `ec.l10n.format` all go through that host. The locale catalog for `localize` is loaded once per session locale, not written into the template.

Links are transcribed as a path plus the `<parameter>` elements, evaluated against the scope. The server still enforces artifact authz on navigation and on the transition. This pass does not bake `disableLink` into the template, and it does not add a permission map to the package. A link the user may not use is still rendered. The server rejects the request.

`dynamic-container` stays a client fetch. The element in the static template is the URL. The type of the fragment is the response header `X-Moqui-Qvt: static-template` or `X-Moqui-Qvt: merged-fragment`. `Cache-Control` stays the caching header (`screen-server-static` for a template, the existing no-store header for a merged fragment) and is not the type signal. A server-static inner screen sends `static-template`, and the client treats the body as a template and loads that screen's `/actions`. An inner screen that is not server-static sends `merged-fragment`, and the client compiles the body as a server-merged qvt fragment. It does not re-evaluate that fragment's expressions as a static template. `include-screen` of another static screen is a child component. That child loads its own template and its own `/actions`. `share-scope="true"` passes the parent package into the child scope. The child's own package is pushed on top: a child key hides a parent key of the same name, and parent keys the child does not set stay visible.

`dynamic="true"`, and a form whose `extends`, service name, or entity name is an expression, rebuild the widget tree from request data. There is no single template to cache. The qvt response for that screen stays a server merge, the static cache header is withheld, and the compatibility report says why. DbForm and `screen-extend` are configuration. They are part of the resolved definition, and the cache identity includes that resolved definition so a form edited in the database is not served as the previous template.

`m-form-list` in `WebrootVue.qvt.js` is a stub (Bootstrap markup, no sub-rows, no row selection, no saved finds). The static form-list is a new Quasar component that fetches `/actions/{formName}`, pushes each row, and paginates by changing that request. It does not reuse the stub.

## Browser host

The browser `ec` is a small object implemented in the qvt client runtime. It is populated from the session, including the JSON config that replaces the `#conf*` inputs. It is not sent inside the screen package.

| Member | Behavior in the browser |
|---|---|
| `ec.l10n.localize` | Looks up the string in the locale catalog. A missing entry returns the original string, as the server does. |
| `ec.l10n.format` | Formats a date, time, or number with the session locale and time zone. |
| `ec.l10n.formatCurrency` | Formats a currency amount, with the currency symbol. |
| `ec.l10n.formatCurrencyNoSymbol` | Formats a currency amount, without the symbol. |
| `ec.user.userId`, `username`, `locale`, `timeZone` | The logged-in session. |
| `ec.user.userAccount` | `partyId`, `emailAddress`, `userFullName`, plus the session locale and time zone. Credential columns (`currentPassword` and the other password fields) stay on the server. |
| `ec.user.getPreference` | Reads a preference from the snapshot loaded with the session. |
| `ec.resource.expand` | Localizes a string under the server's length rule, then evaluates its expressions against the current scope plus the extra map argument. Those expressions run under this same restricted host. A string that calls `ec.entity` or `sri` throws the way a widget expression does. |

Anything else read from `ec` throws in the browser. That includes `ec.entity`, `ec.web`, `ec.context`, `ec.resource` methods other than `expand`, `ec.user` methods other than the reads above, and `sri`. An expression that calls `ec.entity.find`, `ec.web.getRequest`, or `sri.buildUrl` is a screen bug for server-static mode. The fix is to move that work into actions, or into the package prep above, so the template reads a value that is already on the context.

`ec.resource.expand` expands strings under the restricted host. It does not load resources, and it does not call the server to evaluate an expression. `ec.resource.getLocationText` stays unsupported.

## Compatibility check

The screen schema allows expressions the browser host rejects. The check finds those before a user hits them.

The execution context used for widget evaluation can be wrapped. The wrapper's `ec` returns a facade per area. The methods in the table above delegate to the real implementation. Every other operation throws, with the screen location, the element, and the operation. The wrapper is installed only while widget expressions run (`expand`, `condition`, and `expression` from widgets and from `<set>` in a field). `<actions>`, services, and entity finds inside actions use the real context. Data prep is allowed to do that work.

Two places use the wrapper.

The qvt macros, while emitting a server-static template, inspect each expression they transcribe. A call they can see that is outside the host is recorded. This pass is static. It misses a method chosen at runtime.

The full check renders the screen in a mode that walks the whole widget tree and evaluates every expression. `html`, or `qvt` with server-static forced off, both do that. The run is a screen-facade tool, not part of an ordinary user request. It uses real actions so lists, conditions, and rows actually execute, and the wrapper around widget evaluation. Each throw is caught, recorded, and the render continues. Macros also record server-only work they still perform on that path when the result would have to be baked into a static template and is not covered by package prep: `disableLink`, a `display-entity` or `entity-options` lookup that the package step does not handle, a URL that can only be built by `sri` from request state, a `section-iterate` whose list name is a form-list `@list`, and decimal arithmetic or an `ObjectUtilities` call that does it. The same tool reads `always-actions` and `pre-actions` for `service-call`, entity create, update, and delete, and message elements. Those are recorded as side effects. The wrapper stays off for them, because data prep is allowed to do that work. The record is what keeps that screen off server-static. The tool logs one report per screen: location, mode, and each violation.

A clean report means this run found no violation, so the screen can be marked `server-static="qvt"` for the data that was rendered. The render only executes rows the actions produced, and the static scan misses a method chosen at runtime, so a clean report is not a proof that every future row is safe. Inverting the default waits on reports for the screens we care about, read with that limit. The report also names screens whose `always-actions` or `pre-actions` write, send, or append a message. Those stay off server-static until the form request stops repeating that work.

## Current Java behavior the port keeps

- The attribute, the automatic subscreens-only case, form inheritance, and the `screen-server-static` header.
- The path walk that runs `always-actions` and `pre-actions` before the target actions.
- `unwrapMap` as the plain-data rule for the screen package.
- Form-list `entity-find`, `search-form-inputs` pagination, `row-actions`, `field.@from`, and aggregation, on the form request.
- Tree JSON on `actions/{treeName}`. `dynamic-options` without `server-search` is a server-side `ScreenTest` of the transition. `server-search` is a client fetch of that transition URL.
- `/menuData` as the user-specific menu fetch. The template does not include the menu. `getMenuData` already runs ancestor `pre-actions` and drops subscreens the user may not view.
- qvt `always-standalone`, so the template is the target screen and the shell is the SPA.

Java today still merges data into a template it then marks cacheable if the attribute is set. The static macros close that gap for `qvt`. Java's screen `/actions` returns the whole context, including lists. This plan removes form-list keys from that response. Java's form `/actions/{formName}` returns the display projection as a bare array, with pagination only in `X-Total-*` headers, and it logs the body at info. This plan returns the context-shaped map described above and does not log the body. The `m-form-list` stub and the `form-list` server-static branch in the qvt macros are the starting point for the client fetch and are replaced rather than ported.

## Later

Inverting the default, so `qvt` is server-static unless the attribute excludes it, waits on compatibility reports for the screens we care about, read with the limit above. A greyed-out link for `disableLink` waits on the same reports. If they show that the visual state matters, the form or screen package gains an explicit permitted flag per link. It is not part of this pass. Skipping the repeated screen actions on the form request is the same kind of later pass.
