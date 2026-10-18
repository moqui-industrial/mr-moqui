# moqui-html.js

The client of the `html` render mode, with `../css/moqui-html.css`. It has no dependencies and no build step: the
file is plain ES2020 and is served as it is. About 8 KB gzipped. `ScreenHtml.js` (`framework/template/screen/`)
writes the markup; this file binds the data attributes below. A page without JavaScript still works: forms post,
links follow, and a select shows its current option.

Tests of the pure functions run under Node:
`node --test runtime/base-component/webroot/screen/webroot/js/moqui-html.test.js`. The cargo test
`screen::tests::files::the_html_client_passes_its_node_tests` runs them when `node` is on the path. Behavior in a
browser is the Chromium smoke of Gate 3 (`crates/moqui-host/tests/gate3_browser.rs`).

## Attributes

| Element | Attribute | What the client does |
|---|---|---|
| `button`, `a` | `data-dialog="id"` | `showModal()` of the `dialog` with that id |
| `dialog` | `data-open` | opens when the page loads |
| `dialog` | `data-load="url"` | loads its `.body` from the URL when opened, empties it when closed |
| any | `data-close` | closes the dialog around it |
| `div` | `data-load="url"` | loads the URL into the element when the page loads. `moqui.load(el)` loads it again |
| `a` | `data-load-into="id"` | loads the link into the element instead of following it |
| `nav.tabs` | `data-dynamic="id"` | links with `data-load-url` load into the element and mark their tab active |
| `nav` | `data-tab-hash` | Login tabs: the `.tab-panel` sections follow the URL hash |
| any | `data-move-to="id"` | moves the element into `#id` (a `ul` gets a `li` around it): popup menus and crumbs go to the navbar |
| `button`, `a` | `data-confirm="text"` | `confirm()` before the click does anything |
| `button` | `data-clear="formId"` | clears the form's fields |
| `button` | `data-theme-toggle`, `data-pref-url` | switches dark and light, and posts `OUTER_STYLE` |
| `button` | `data-nav-toggle` | opens the navbar on a narrow screen |
| `a` | `data-history-back` | `history.back()` |
| `.alert` | `button[data-dismiss]` | removes the message |
| `form` | `data-bg`, `data-reload`, `data-msg`, `data-hide` | background submit; then reload the container with that id, toast a message, close the dialog with that id |
| `form` | `data-focus="name"` | focuses the first field whose name starts with it |
| `form` | `data-gopage="n"` | the page number box is 1-based and posts a 0-based `pageIndex` |
| `input[type=datetime-local]` | | the `T` becomes a space in the posted value (`yyyy-MM-dd HH:mm`) |
| `select` | `data-ts="select\|remote\|ac"` | Tom Select, loaded on first use. `data-combo` allows new values, `data-orderby="inputId"` writes the order-by text there |
| `select`, `input`, `span` | `data-url`, `data-params`, `data-depends`, `data-optional` | transition URL, fixed parameters (JSON), `[[fieldId, parameter], ...]` read when the options load or the field changes |
| `select` | `data-search`, `data-min`, `data-delay`, `data-label`, `data-value`, `data-empty`, `data-use-actual` | search as you type, minimum length, delay, label and value field names, empty first option, the typed text is the value |
| `input` | `data-default` | text-line `default-transition`: fills the value from the transition |
| `span[data-dynamic="fieldId"]` | `data-hidden` | display with `dynamic-transition`: sets `fieldId_display` and the hidden input |
| `.tree` | `data-url`, `data-params` | fetches nodes (POST, `treeNodeId`, `treeNodeName`), draws a nested list, opens children on click |
| `.editable` | `data-url`, `data-params`, `data-name`, `data-widget`, `data-load-url`, `data-load-params` | click to edit, save by POST |
| `ul` | `data-sortable-tree="inputId"` | form-list column picker with SortableJS; the hierarchy goes to the input as JSON |
| any | `data-btn`, `data-size`, `data-type`, `data-align`, `data-wide`, `data-submit` | style only |
| `meta[name=moquiSessionToken]` | | the CSRF token, sent as the `X-CSRF-Token` header and `moquiSessionToken` parameter on every request |
| `meta[name=moqui-notify]` | | the path of the notification WebSocket. Nothing connects when it is absent |

`window.moqui` has `bind(element)` (call it after you insert html), `notify(text, type)`, `load(element, url)`,
`request(url, fetchOptions)` and `post(url, params)`.

---

# The qvt client

The client of the `qvt` render mode and of the `/qapps` shell: Vue 3.5 (the build with the template compiler) and Quasar 2.35 from
`../libs`. There is no build step and no other library: no jQuery, Moment, Select2, Bootstrap Notify, jQuery Validate or
`http-vue-loader`. The pages of the `html` mode load `moqui-html.js` instead, and the two never share a page (both define `window.moqui`).

| File | What |
|---|---|
| `MoquiLib.js` | the `moqui` object: type checks, `deepCopy`, field comparison, `objToSearch` / `searchToObj` / `parseHref` / `makeHref`, `request` (fetch with the session token), `loadScript` / `loadStylesheet`, `parseVue` and `evalComponentJs` (the `qvue` and `qjs` text), `NotificationClient`, `LruMap`. Runs under Node. |
| `MoquiL10n.js` | `moqui.l10n`: `format`, `formatCurrency`, `formatCurrencyNoSymbol` and `localize`. A copy of `crates/moqui-lib` `format.rs` and `datefmt.rs`: Java `DecimalFormat` and date pattern subset, HALF_EVEN on decimal text. `MoquiL10n.test.js` runs `crates/moqui-lib/testdata/format-vectors.json`. |
| `WebrootVue.qvt.js` | the `m-*` components and the application. `moqui.qvt.components` holds the definitions. |
| `MoquiQzComponent.js` | QZ Tray 2.3 printing, loaded after `/libs/qz-tray/qz-tray.js` and only when the user preference `qz.print.enabled` is `true`. |
| `../css/WebrootVue.qvt.css` | the styles of the shell, the div tables and the widgets |

The tests are `MoquiLib.test.js`, `MoquiL10n.test.js`, `WebrootVue.qvt.test.js` and `WebrootVue.qvt.static.test.js` (the script and package
the server answers for `crates/moqui-core` `testdata/Static/Items` run on the real Vue runtime over a fake DOM; every component template compiles with the vendored
Vue compiler; `loadComponent` is checked with a stub for the request). `cargo test` runs them when `node` is on the path. What needs a
browser (mounting, Quasar menus and popups, dialogs, posts, the lazy libraries, static lists) is the Chromium smoke of Gate 3
(`crates/moqui-host/tests/gate3_browser.rs`); downloads and the notification WebSocket are not driven there yet.

## Page shell and settings

`/qapps/**` is one HTML page, `includes/WebrootVue.qvt.html.fancy`, made by the pre-actions of `qapps.xml`. The libraries load at the end
of the body in this order: `vue/vue.global.prod.js`, `quasar/quasar.umd.prod.js`, `quasar/lang/<pack>.umd.prod.js`, `MoquiLib.js`,
`MoquiL10n.js`, `WebrootVue.qvt.js`. Compiling templates in the browser needs `unsafe-eval` in a content security policy; the page has no
inline script and no inline event handler, and the settings are a data block:

```html
<script type="application/json" id="moqui-config">{ ... }</script>
```

| Key | Meaning |
|---|---|
| `sessionToken` | the CSRF token. `moqui.request` sends it as `X-CSRF-Token` on writes and a form adds `moquiSessionToken`. A response header `X-CSRF-Token` replaces it, and the other tabs are told through a `BroadcastChannel`. |
| `appHost`, `appRootPath` | host with port, and the context path (empty here) |
| `basePath`, `linkBasePath` | `/apps` and `/qapps`: screens are fetched under the first and the address of the page is under the second. `getLinkPath` converts. |
| `userId`, `username`, `userFullName` | the signed-in user |
| `locale`, `timeZone`, `quasarLang` | `en-US` style tag, zone name, and the key of the language pack in `Quasar.Lang` (`enUS`, `ptBR`, `zhCN`, `de`, ...) |
| `darkMode`, `leftOpen` | the preferences `QUASAR_DARK` and `QUASAR_LEFT_OPEN` |
| `notificationEndpoint` | the path of the notification WebSocket, from `webapp_notification_endpoint`. Empty means no socket is opened. |
| `navPlugins`, `accountPlugins` | theme values `STRT_HEADER_NAVBAR_COMP` and `STRT_HEADER_ACCOUNT_COMP`: URLs of components for the header |
| `qz` | `{enabled}`: print through QZ Tray |
| `qvt` | `{build, handles}`: the Vue build this page loaded (`compiler`, or `runtime` with `webapp_qvt_static_only=true`) and the `X-Moqui-Qvt` responses it handles (`merged-fragment`, `static-template`) |
| `userAccount`, `preferences` | the snapshot of the user for the `ec` of a server-static screen: `{partyId, emailAddress, userFullName}` (no password column) and the user's preferences by key |
| `labels` | text the shell needs localized |
| `quasar`, `currencies` | optional: Quasar configuration (brand, notify, loading bar), and `{USD: {symbol, fractionDigits}}` for `formatCurrency` |

The catalog of messages for `localize` is fetched from `/apps/l10nCatalog?locale=` once the page starts: `{locale, messages: {original: localized}}`
with the `default`, language and locale entries merged.

## Loading a screen

`moqui.loadComponent(urlInfo, callback, divId)` requests `<path>.qvt` (or `.qjs` / `.qvue` when the menu data lists that render mode for the
screen) with the extra path and query, and gives `callback` a component. It returns an object with `abort()`.

- `205` and `X-Redirect-To`: the shell goes to that address (a redirect of a render that is always standalone).
- `.qvue`: the text is a component file: `<template>`, `<script>` (`module.exports =`, `export default` or `define(...)`) and `<style>`.
- `.qjs`: an object expression or `define(...)`. Without a `template` the template is the `.qvt` of the same screen.
- `.qvt`: the response header `X-Moqui-Qvt` chooses the handler in `moqui.qvtHandlers`:
  `merged-fragment` (the default) wraps the text as the template `<div id="divId">text</div>`, and `<script>` and `<link>` in it become
  `m-script` and `m-stylesheet`; `static-template` is the script of a server-static screen (below). A handler may return a promise.
- A response with `Cache-Control: max-age` is kept by screen path in `moqui.componentCache` (50 entries). A merged response says
  `private, no-store` and is never kept. For a static template the cache holds the means to make a component, and each use makes its own, since
  each use fetches its own data.

## The components the macros write

`ScreenQvt.js` writes only these names for widgets. A widget binds with `v-model` (`modelValue`) inside a form, and with `model-value`
outside one. The wrappers add `dense outlined stack-label hide-bottom-space` themselves (Quasar floats the label inside the outline; a group of
checkboxes or radios is a borderless `m-field` over a `q-option-group`, and a `display` with a label is a borderless read-only field), tint a field
that differs from its form's `fields-initial`, and take `fields` where they read the fields they depend on (`depends-on`). The `id` a macro gives a
field wrapper is the id of its native input (the `for` of the Quasar field), so `label for`, `focus-field` and `document.getElementById` find the
input. Raw `q-input`, `q-select`, `q-date` and `q-field` are not written by a macro; `q-btn`, `q-tooltip`, `q-badge`, `q-list`, `q-btn-dropdown`,
`q-expansion-item`, `q-checkbox` and the layout classes are.

Buttons are of three kinds, so a screen reads the same everywhere: a `submit` is `unelevated color="primary"` once a field of its form changed
(`outline` until then), and a `link` or `button-menu` with an explicit `btn-type="primary"` is the same filled primary button (the one main
action of a title row); a secondary action (a `link` outside a form-list cell, a transition link, a `button-menu`, a dialog's button, the Find
Options / Columns / Saved Finds tools of a list) is tonal, `unelevated class="m-btn-tonal"` with `text-<color>` for a `btn-type` other than
primary (`../css/WebrootVue.qvt.css`); a minor one (`reset`, `hidden-form-link`, page size, the export menu, icon buttons) is `flat`. A screen
link with the default link-type inside a form-list cell, and `link-type="anchor"` anywhere, is text (`m-link-text`); `anchor-button` keeps the
button in a cell. A `container-dialog` or `dynamic-dialog` button inside a form-list cell is flat and dense like the other in-cell links: the
container-dialog's button comes through the component's `button` slot (`q-btn dense flat class="m-link"`), the dynamic-dialog gets
`button-class="m-btn-flat"`; outside a cell both are the component's tonal button with the `icon` attribute as `button-icon` (default
`open_in_new`). The exports of a list (csv, xlsx, text, print) are the items of one `q-btn-dropdown`, and the page buttons are a
`q-pagination`, in a `form-list-nav-row` above the table (with the tools) and another below it.

The `icon` attribute of `link`, `submit`, `button-menu` and the dialogs is a Material Icons name (`add`, `edit`; a `sym_o_*` name later),
written as `<q-icon name="…" size="sm">`, or CSS classes when the value has a space or an `fa` prefix (`fa fa-plus`), written as
`<i class="… q-icon">`; the dialogs give either form to the `icon` prop of their `q-btn`. The cells of a `field-row` are `col-xs-12 col-sm-6`
(two fields side by side from 600px, stacked below), `col-sm-4` for three fields and `col-sm-3` for four.

Two small classes of the style guide are in the same sheet: `.m-muted` for a caption (`grey-7`, and `grey-5` under `body--dark`), and `.m-tonal` on a
`q-chip` or `q-badge` for the tint of `.m-btn-tonal` (a status chip is `q-chip dense square class="m-tonal text-positive"`: the tint follows the text
color, so a `text-<color>` class picks it, not `color=`, whose `bg-*` class would cover it; primary without one). A list's row selection column is
`m-sel` (as narrow as its checkbox), and a list wider than its card scrolls inside it (`overflow-x` on `.q-table__container`).

| Component | For |
|---|---|
| `m-form` | form-single and rows of a form-list: `fields-initial`, `v-slot:default="formProps"` (`fields`, `hasFieldsChanged`, `fieldChanged`, checkbox set), submit by `fetch` (JSON answer, or a download), `submit-message`, `submit-reload-id`, `submit-hide-id`, `focus-field`, `exclude-empty-fields`, `parent-checkbox-set`, `validation-errors-initial`; the validation errors of its own fields go in `fieldErrors`, which the field wrappers show on the Quasar field |
| `m-form-link` | a form that goes to a screen: the fields become the query. Slot `fields`, `clearForm` |
| `m-checkbox-set` | selected rows: `checkbox-count`, `checkbox-parameter`, `checkbox-list-mode`, `checkbox-values` |
| `m-form-list` | a list drawn by the client from rows it fetches, with sort, pages, selection (the `m-sel` column) and sub rows (see below); the page buttons are written again under a list of more than one page, after the table |
| `m-form-paginate`, `m-form-go-page`, `m-form-column-config` | page buttons, jump to a page, the column picker |
| `m-scope`, `m-static-form-list` | written by `ScreenQvtStatic.js` only: a scope over a form-single's map, and the form-list of a server-static screen (see Server-static screens) |
| `m-text-line`, `m-text-area` | text input. `m-text-area` with `editor-type` `html` is `QEditor`, and `md` is a text area with a Marked preview |
| `m-drop-down` | `options`, `options-url`, `options-parameters`, `depends-on`, `server-search`, `multiple`, `allow-empty`, `combo`, `submit-on-select`, `options-load-init` |
| `m-date-time`, `m-date-period` | `QDate` and `QTime` in a popup; period and from-thru. In period mode the field is the base date (`_pdate`) and the period and offset selects (`_period`, `_poffset`) are in the menu of the `event_repeat` button in its append slot, which reads the choice ("Last Month") once made; the `date_range` button switches to from-thru (`_from`, `_thru`) |
| `m-display` | text or a read-only field (`label-wrapper`), `display`, `value-url` |
| `m-check`, `m-radio`, `m-file`, `m-field`, `m-text-find`, `m-range-find` | the other widgets |
| `m-editable` | click to edit, `QPopupEdit`, `url`, `url-parameters`, `parameter-name`, `widget-type`, `load-url` |
| `m-link` | a screen link: `href`, `load-id`, `confirmation`. A `data-confirm` attribute on any button or link is asked first |
| `m-container-box`, `m-box-body`, `m-container-dialog`, `m-dialog` | boxes and dialogs |
| `m-dynamic-container`, `m-dynamic-dialog` | a screen loaded into a container, or a dialog, with `load`, `reload` and `hide`; the dialog's button takes `button-text`, `button-class`, `color` and `button-icon` (default `open_in_new`), as `m-container-dialog` does |
| `m-tree-top`, `m-tree-item` | tree: `items` is the node list or its URL (`<screen>/actions/<tree>`, posted `treeNodeId`, `treeNodeName`, `treeOpenPath`). A node's `children` is a list when it came inline (on `open-path`), `true` to load from the URL when the node is opened, `false` for a leaf |
| `m-subscreens-tabs` (also `m-subscreens-menu`), `m-subscreens-active`, `m-subscreens-panel` | the subscreen menu and the place of the active subscreen |
| `m-menu-nav-item`, `m-menu-subscreen-item`, `m-menu-item-content` | the navigation of the shell |
| `m-chart`, `m-mermaid` | Chart.js 4 and Mermaid 11, loaded on first use. `m-chart` takes `config`; `m-mermaid` takes the diagram in its `text` attribute (the compiler condenses the line breaks of slot content), `height` and `config` |
| `m-script`, `m-stylesheet` | `<script>` and `<link>` of a screen's text |

`m-form-list` takes `rows` (a list, or the URL of the rows, else `<current screen>/actions/<name>`), `search`, `list-name`, `action`, `multi`,
`skip-form`, `skip-header`, `header-form`, `selection` and the buttons, and slots `rowForm` (`fields`, `rowIndex`), `row` (`fields`, `rowIndex`,
`selected`), `subRow` (`row`, `subRow`), `header` (`search`, `sortBy`, `orderBy`), `headerForm`, `nav`. The answer to a rows request is a list, or a map with the
list under the name of the form and `Count`, `PageIndex`, `PageSize`, `PageMaxIndex`, `PageRangeLow`, `PageRangeHigh` keys. The server-merged
macros write the rows themselves; this component is for the screens that fetch.

The shell is a `q-layout` with a `q-header` (menu button, logo, title, the screen path as `q-breadcrumbs`, the documentation, notification,
history and account menus), a `q-drawer` with the screen menu (`QUASAR_LEFT_OPEN`; an overlay below 1024px), the page, and a `q-footer` with the
theme's footer items. The `LoadingBar` plugin runs while the root's `loading` count is above zero. `config.quasar` (brand, notify, loadingBar) is the
JSON of the theme's `STRT_QUASAR_CONFIG` resource, merged over the defaults.
The shell gives Quasar a stand-in for `vue-router` (`$router.resolve`, `push`, `replace`, and `$route`), so `q-btn to="..."` and `q-item to="..."` navigate with `setUrl`.
`moqui.webrootVue` is the root: `setUrl(url, bodyParameters)`, `setParameters(obj)`, `reloadContainer(id)`, `loadContainer(id, url)`,
`hideContainer(id)`, `addNotify`, `getLinkPath`.

## Server-static screens

`plans/archive/QvtServerStatic.md` is the plan. A screen whose `server-static` names `qvt` (or that has only subscreens placeholders) answers `<path>.qvt` with
a script and `X-Moqui-Qvt: static-template`, `Cache-Control: max-age=86400, must-revalidate, public` and an `ETag`. The script is
`(m.qvtStatic = m.qvtStatic || {})[path] = function (Vue) { ...; return function render(_ctx, _cache) { ... } }`, the Vue compiler's
`prefixIdentifiers` output of a template that holds no request data (no `with`, no `new Function`). The handler loads it with a script element
(a CSP without `unsafe-eval` allows that and not `fetch` and `new Function`), and the component it makes fetches `<path>/actions` (the
screen package, `private, no-store`) and shows the render function over it.

- **Scope chain.** Every free name in a template expression is `_ctx.name`. The root `_ctx` is `qvt.makeScope(package, ...)`. `m-scope` (form-single
  map, with `mqExtra_{form}` on top) and the `row` slot of `m-static-form-list` declare their own `_ctx` (`v-slot="_ctx"`), so the innermost map wins,
  as the context stack does on the server. A `section-iterate` is a `v-for` whose entry is a local. `$scope` is the scope itself, `ec` the host below.
- **`ec`.** `qvt.makeEc(config)` is a Proxy: `ec.l10n` (`localize`, `format`, `formatCurrency`, `formatCurrencyNoSymbol`), `ec.user` (`userId`, `username`,
  `locale`, `timeZone`, `userAccount`, `getPreference`) and `ec.resource.expand` / `expandNoL10n`. Any other member throws
  `ec.X is not available in a server-static qvt screen`. `expand(pattern, table)` localizes the pattern (under 256 characters) and replaces each `${expr}` with
  `table[expr]()`, the table of functions the compiled template carries for the expressions of that text, so nothing is evaluated from text.
  An expression that is not in the table throws.
- **`$mq`** (`qvt.helpers`): `str`, `dflt`, `nul`, `cur` (current value of a drop-down, check or radio), `fmt` (a value as text with a Java pattern, picking number,
  date or time from the pattern for a string), `merge`, `url` / `qs` (path and query from a parameter map), `shown` (`visible-when`).
- **`m-static-form-list`** fetches `<path>/actions/<name>` with the request's parameters and the page and sort it has changed (`setPageIndex`, `sortBy`,
  `orderBy`), draws `m-form-paginate` and `m-form-go-page`, the `header` slot (`sortBy`, `orderBy`, `action`, `orderByField`) and the `row` slot once for each row with
  the row on the scope of the screen (`rowIndex` is a name of it). The rows of each answer are new components (an `m-form` copies its
`fields-initial` once), so another page or order is not patched into the rows of the last one.

## Not here

The Assist panel (`js/assist`, the chat and canvas, `confAssist`) is not ported: it needs the vendored Chart.js, Marked, DOMPurify and highlight.js paths
of `../README.md` (`chartjs/chart.umd.min.js`, `highlightjs/highlight.min.js`, `highlightjs/styles/github-dark.min.css`) when it is. The `m-*` components
and the shell root keep string templates, so only the compiler build of Vue runs them: the runtime build waits for those to be precompiled. A static
form-list does not write row selection, saved finds, the column picker, first/second/last rows, aggregation, or the csv, text and pdf buttons yet (the
compatibility report says which screens use them).
