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
| `.editable` | `data-url`, `data-params`, `data-name`, `data-widget`, `data-load-url`, `data-load-params`, `data-options`, `data-value`, `data-empty` | click to edit, save by POST; `data-widget` `select` (the options, or those `data-load-url` answers), `date` and `date-time` (a date input) |
| `.status-bar` | `form.status-form` | the `status-bar` element: the stage chips (`data-state`, `data-color`) and a form per transition; nothing to bind beyond `data-confirm` |
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
| `MoquiLib.js` | the `moqui` object: type checks, `deepCopy`, field comparison, `objToSearch` / `searchToObj` / `parseHref` / `makeHref`, `request` (fetch with the session token; it hands a response's `X-CSRF-Token` to `moqui.onSessionToken` and its `X-Moqui-Preferences` to `moqui.onPreferences`), `loadScript` / `loadStylesheet`, `parseVue` and `evalComponentJs` (the `qvue` and `qjs` text), `NotificationClient`, `LruMap`. Runs under Node. `loadScript(src, callback, validate)` loads a script once (`moqui.scriptLoads`, one promise per src): a call while it loads waits for it; when it fails every waiter is rejected (and the callback gets the error) and the element is removed, so a later call loads it again; a `validate` that never sees its library rejects after its retries (about 20 s). |
| `MoquiL10n.js` | `moqui.l10n`: `format`, `formatCurrency`, `formatCurrencyNoSymbol` and `localize`. A copy of `crates/moqui-lib` `format.rs` and `datefmt.rs`: Java `DecimalFormat` and date pattern subset, HALF_EVEN on decimal text. `MoquiL10n.test.js` runs `crates/moqui-lib/testdata/format-vectors.json`. |
| `WebrootVue.qvt.js` | the `m-*` components and the application. `moqui.qvt.components` holds the definitions. |
| `WebrootPublic.qvt.js` | the public shell of the anonymous token routes (`/s/<token>`; `docs/Revision-Management.md`, Token routes): on the page `webroot/PublicShell.xml` makes, with its own settings block `moqui-public-config` (`route {name, basePath, title, renderModes}`, `context`, the locale and the theme's Quasar configuration), it mounts the route's screen (loaded as `basePath.qvt`, `.qvue` or `.qjs`) with the `m-*` components of `WebrootVue.qvt.js`, which loads before it and starts nothing there. The root offers the parts of `m-qapps-root` the components call; a link under the route stays on the page, any other leaves it. |
| `WebrootChat.js` | `moqui.chat`: the chat store and presence of the shell (below, Chat store and presence), and the account menu's status components, in `moqui.qvtComponents`, which `WebrootVue.qvt.js` registers. Loads before it. Runs under Node. |
| `MoquiQzComponent.js` | QZ Tray 2.3 printing, loaded after `/libs/qz-tray/qz-tray.js` and only when the user preference `qz.print.enabled` is `true`. |
| `../css/WebrootVue.qvt.css` | the styles of the shell, the div tables and the widgets |

The tests are `MoquiLib.test.js`, `MoquiL10n.test.js`, `WebrootVue.qvt.test.js`, `WebrootVue.qvt.shell.test.js` (the palette, the key map, the
screen context's forms and the root's shell data), `WebrootVue.qvt.records.test.js` (the HTML of the rich-text editor, the record panel URL,
avatars and record cards), `WebrootVue.qvt.layout.test.js` (the split's layout and the wizard's way after an answer), `WebrootVue.qvt.htmlview.test.js` (m-html-view's cleaning), `WebrootPublic.qvt.test.js` (the public shell's screen load and links), `WebrootChat.test.js` (the chat store: presence,
typing, read markers, unread counts, a pushed event merged into a timeline, do not disturb, the status components) and `WebrootVue.qvt.static.test.js` (the script and package
the server answers for `crates/moqui-core` `testdata/Static/Items` run on the real Vue runtime over a fake DOM; every component template compiles with the vendored
Vue compiler; `loadComponent` is checked with a stub for the request). `cargo test` runs them when `node` is on the path. What needs a
browser (mounting, Quasar menus and popups, dialogs, posts, the lazy libraries, static lists) is the Chromium smoke of Gate 3
(`crates/moqui-host/tests/gate3_browser.rs`); downloads and the notification WebSocket are not driven there yet.

## Page shell and settings

`/qapps/**` is one HTML page, `includes/WebrootVue.qvt.html.fancy`, made by the pre-actions of `qapps.xml`. The libraries load at the end
of the body in this order: `vue/vue.global.prod.js`, `quasar/quasar.umd.prod.js`, `quasar/lang/<pack>.umd.prod.js`, `MoquiLib.js`,
`MoquiL10n.js`, `WebrootChat.js`, `WebrootVue.qvt.render.js`, `WebrootVue.qvt.js`. Compiling templates in the browser needs `unsafe-eval` in a content security policy; the page has no
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
| `density`, `startApp` | `SHELL_DENSITY` (`comfortable` or `compact`) and `SHELL_START_APP`, the app the user may open that `/qapps` with no path opens (empty for the app list) |
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
primary (`../css/WebrootVue.qvt.css`); a minor one (`reset`, `hidden-form-link`, page size, the export menu, icon buttons) is `flat`, and a
`hidden-form-link` keeps its text at the left as a text link's (`m-link-left`: Quasar centers a button's content). A link that posts
through a hidden form takes a form-single's `background-reload-id`, `background-hide-id` and `background-message` (the `m-form`'s
`submit-reload-id`, `submit-hide-id` and `submit-message`; the html mode posts it in the background with any of them or `background-submit`). A screen
link with the default link-type inside a form-list cell, and `link-type="anchor"` anywhere, is text (`m-link-text`); `anchor-button` keeps the
button in a cell. A `container-dialog` or `dynamic-dialog` button inside a form-list cell is flat and dense like the other in-cell links: the
container-dialog's button comes through the component's `button` slot (`q-btn dense flat class="m-link"`), the dynamic-dialog gets
`button-class="m-btn-flat"`; outside a cell both are the component's tonal button with the `icon` attribute as `button-icon` (default
`open_in_new`). The exports of a list (csv, xlsx, text, print) are the items of one `q-btn-dropdown`, and the page buttons are a
`q-pagination`, in a `form-list-nav-row` above the table (with the tools) and another below it.

The `icon` attribute of `link`, `submit`, `button-menu` and the dialogs is a Quasar icon name, Material Symbols outlined (`sym_o_add`, the
style guide's names) or Material Icons (`add`, `edit`), written as `<q-icon name="…" size="sm">`, or CSS classes when the value has a space or
an `fa` prefix (`fa fa-plus`), written as `<i class="… q-icon">`; the dialogs give either form to the `icon` prop of their `q-btn`. A menu
image of `menu-image-type="icon"` follows the same rule (`moqui.isIconClass`): `m-menu-icon` draws it as a `q-icon` in the drawer, the
breadcrumbs' subscreen menus and the screen history, and as an `i` with the classes for `fa fa-*`. Quasar's own icons (select arrows,
pagination, expansion items) are its `material-symbols-outlined` icon set, set when the app is created. The cells of a `field-row` are `col-xs-12 col-sm-6`
(two fields side by side from 600px, stacked below), `col-sm-4` for three fields and `col-sm-3` for four.

Two small classes of the style guide are in the same sheet: `.m-muted` for a caption (`grey-7`, and `grey-5` under `body--dark`), and `.m-tonal` on a
`q-chip` or `q-badge` for the tint of `.m-btn-tonal` (a status chip is `q-chip dense square class="m-tonal text-positive"`: the tint follows the text
color, so a `text-<color>` class picks it, not `color=`, whose `bg-*` class would cover it; primary without one). A list's row selection column is
`m-sel` (as narrow as its checkbox), and a list wider than its card scrolls inside it (`overflow-x` on `.q-table__container`).

| Component | For |
|---|---|
| `m-form` | form-single and rows of a form-list: `fields-initial`, `v-slot:default="formProps"` (`fields`, `hasFieldsChanged`, `fieldChanged`, checkbox set), submit by `fetch` (JSON answer, or a download), `submit-message`, `submit-reload-id`, `submit-hide-id`, `focus-field`, the `submitted` event (with the answer, the JSON or the text) after a submit whose answer has no errors, for a `.qvue` host (`<m-form @submitted="...">`), `exclude-empty-fields`, `parent-checkbox-set`, `validation-errors-initial`; the validation errors of its own fields go in `fieldErrors`, which the field wrappers show on the Quasar field |
| `m-form-link` | a form that goes to a screen: the fields become the query. Slot `fields`, `clearForm` |
| `m-checkbox-set` | selected rows: `checkbox-count`, `checkbox-parameter`, `checkbox-list-mode`, `checkbox-values` |
| `m-form-list` | a list drawn by the client from rows it fetches, with sort, pages, selection (the `m-sel` column) and sub rows (see below); the page buttons are written again under a list of more than one page, after the table |
| `m-form-paginate`, `m-form-go-page`, `m-form-column-config` | page buttons, jump to a page, the column picker |
| `m-scope`, `m-static-form-list` | written by `ScreenQvtStatic.js` only: a scope over a form-single's map, and the form-list of a server-static screen (see Server-static screens) |
| `m-text-line`, `m-text-area` | text input. `m-text-area` with `editor-type` `html` is `QEditor`, `md` is a text area with a Marked preview, and `rich` and `rich-light` are `m-rich-text` (with `mention-url` and `mention-parameters`, from the text-area's `mention-transition`) |
| `m-rich-text` | the Tiptap editor (see Rich text below): the `m-*` model contract (`v-model` or `model-value`, `name`, the change tint and field errors in an `m-form`, a hidden input), `light`, `placeholder`, `rows`, `enter` (`newline`, or `submit`: Enter submits the form and emits `submit`, Shift+Enter is a line break; Ctrl+Enter always submits), `mention-url`, `mention-parameters`, `plain` (`v-model:plain`, the plain-text toggle), `plain-toggle`, `autofocus`, `disable`, `readonly`; emits `mention` with the `{id, label}` picked |
| `m-drop-down` | `options`, `options-url`, `options-parameters`, `depends-on`, `server-search`, `multiple`, `allow-empty`, `combo`, `submit-on-select`, `options-load-init` |
| `m-date-time`, `m-date-period` | `QDate` and `QTime` in a popup; period and from-thru. In period mode the field is the base date (`_pdate`) and the period and offset selects (`_period`, `_poffset`) are in the menu of the `event_repeat` button in its append slot, which reads the choice ("Last Month") once made; the `date_range` button switches to from-thru (`_from`, `_thru`) |
| `m-display` | text or a read-only field (`label-wrapper`), `display`, `value-url` |
| `m-check`, `m-radio`, `m-file`, `m-field`, `m-text-find`, `m-range-find` | the other widgets |
| `m-editable` | click to edit, `QPopupEdit`, `url`, `url-parameters`, `parameter-name`, `widget-type` (`textarea`, `text-line`, `select`, `date`, `date-time`), `load-url`, `options` (a select's `{value, label}` list, from the `option`, `list-options` and `entity-options` of the element; else what `load-url` answers: a list, or a map of key to label with `selected`, `moqui.editableOptions`), `value` (a select's current key; by default the option whose label is the text), `placeholder` (an empty text with `display-if-empty` shows it, so there is something to click). A select shows the chosen option's label after a save; a date edits in `QDate` (and `QTime`) as `yyyy-MM-dd` (`yyyy-MM-dd HH:mm`) |
| `m-link` | a screen link: `href`, `load-id`, `confirmation`; an external `href` (another scheme or host, `moqui.isExternalLink`) is a plain anchor the browser follows. A `data-confirm` attribute on any button or link is asked first |
| `m-container-box`, `m-box-body`, `m-container-dialog`, `m-dialog` | boxes and dialogs. `m-container-dialog` (`container-dialog`) registers with the root by its `id` while mounted (`addContainer`, taken back with `removeContainer` when unmounted), so a form's `background-hide-id` (`submit-hide-id`) and `$root.hideContainer(id)` close it; its `reload` and `load` do nothing (the content is the page's). `tooltip` (the `container-dialog` attribute) is a `q-tooltip` on its button, the slot button of a list cell too |
| `m-dynamic-container`, `m-dynamic-dialog` | a screen loaded into a container, or a dialog, with `load`, `reload` and `hide`; the dialog's button takes `button-text`, `button-class`, `color` and `button-icon` (default `open_in_new`), as `m-container-dialog` does |
| `m-tree-top`, `m-tree-item` | tree: `items` is the node list or its URL (`<screen>/actions/<tree>`, posted `treeNodeId`, `treeNodeName`, `treeOpenPath`). A node's `children` is a list when it came inline (on `open-path`), `true` to load from the URL when the node is opened, `false` for a leaf |
| `m-subscreens-tabs` (also `m-subscreens-menu`), `m-subscreens-active`, `m-subscreens-panel` | the subscreen menu and the place of the active subscreen |
| `m-subscreens-wizard` | `subscreens-panel type="wizard"`: a header-only `q-stepper` with a step per subscreen of the menu data (titles, menu order; a done step's header goes back to it), the `m-subscreens-active` below it, and Back and Next (`finish-text` on the last step); `id`, `finish-text`, `next-text`, `back-text`. It provides `mWizard` to the step's forms; while the step form posts a spinner covers the step (see Layout panels below) |
| `m-subscreens-stack` | `subscreens-panel type="stack"`: the first subscreen in menu order under the others, with a `q-breadcrumbs` back to it when another is active; no tabs |
| `m-container-split` | `container-split`: a `q-splitter` of the `side` and `main` slots; `id`, `side-width` (320), `min-main` (480), `side-position` (`left`, `right`), `select-parameter`, `back-text`, `height` (by default the viewport below the split, so each pane scrolls in itself; see Layout panels below) |
| `m-menu-nav-item`, `m-menu-subscreen-item`, `m-menu-item-content`, `m-menu-icon` | the navigation of the shell; `m-menu-icon` is a menu image of type icon (`name`, `size`) |
| `m-screen-context` | the `context` element: while mounted it registers `recordEntity`, `recordId`, `title` and `selectionForm` with the root (`setScreenContext`), again when one changes, and takes them back when unmounted (see Shell data and providers) |
| `m-status-bar` | the `status-bar` element (style-guide §3.3, §5.2): `bar` (what `org.moqui.impl.StatusServices.get#StatusBar` answers: `current`, `stages` with `state` done, current or todo and `color`, `transitions` with `isNext` and `isDanger`), else `status-id`, `status-type-id`, `status-flow-id`, `stages` and `allowed-status-ids` (lists or comma text) and `read-only`, which it asks of `bar-url` (`/statusBar`, the root screen's transition) and again when one changes, as a server-static screen does; `action` (the transition), `parameters`, `parameter-name` (`statusId`), `title`, `confirm` (true). A stage is a `q-chip dense square`: the current one filled in its color, a done one `m-tonal` with a check, one to come outlined; a transition is a button in an `m-form` of its own posting `action` with the new status: the next stage filled primary, the others tonal (negative for those that end badly), each asking "Label: title?" (`moqui.statusBarView`) |
| `m-number-card` | the `number-card` element: a `q-card flat bordered` with the `title` as `text-overline`, the `value` (`text-h5`; `size` `sm` is `text-h6`, `lg` `text-h4`), the `delta` with an arrow, green or red by its sign against `delta-good` (`up`, the default, or `down`; no sign is muted: `moqui.numberCardDelta`), and the `icon`; with `href` the whole card is an `m-link` (`external`: a plain anchor). The `dashboard` element around cards is no component: a `div.m-dashboard` CSS grid, columns of at least its `min-width` (`--m-dashboard-min`, 200 px), each child in a `div.m-dashboard-item` (an empty one hidden, a `section-iterate`'s rows each a cell) |
| `m-chatter`, `m-record-files` | the `chatter` and `record-files` elements: `record-entity`, `record-id`, `default-tab`, `hide`, `id` (of the `m-dynamic-container` the panel loads into, for `submit-reload-id`). See Record panels below |
| `m-avatar` | a `q-avatar`: `image` (a face image URL; initials when it does not load), else the initials of `name` on a color of the name (`moqui.avatar`), else a person icon; `size` (32px), `color`, `text-color`, `square`, `rounded`, `tooltip` (the name); `presence` (`online`, `away`, `dnd`, `offline`) draws a dot, and `user-id` the dot of that user's presence from the chat store (asked for once mounted), nothing without either |
| `m-record-card` | a compact card of a record: `record-entity`, `record-id`, and `card-url` (asked once per page for each record, GET with `recordEntity` and `recordId` and `card-parameters`) or `card` (the JSON itself); `dense`. See Record cards below |
| `m-html-view` | an email or other untrusted HTML body in a frame sandboxed without scripts: `html` (or `text`, a text/plain body), `allow-remote` (a trusted sender: remote images at once), `cid-map` (`{contentId: url}` of the inline images), `collapse-quotes` (true), `images-bar` (true: the "Show images" bar), `dark` (`auto`, `on`, `off`), `title`; emits `remote-images` (how many were hidden), `show-images` (the bar's button was pressed), `loaded` (`{height, blocked}`). In XML the `html-view` element. See Email bodies below |
| `m-chart`, `m-mermaid` | Chart.js 4 and Mermaid 11, loaded on first use. `m-chart` takes `config`; `m-mermaid` takes the diagram in its `text` attribute (the compiler condenses the line breaks of slot content), `height` and `config` |
| `m-script`, `m-stylesheet` | `<script>` and `<link>` of a screen's text |

`m-form-list` takes `rows` (a list, or the URL of the rows, else `<current screen>/actions/<name>`), `search`, `list-name`, `action`, `multi`,
`skip-form`, `skip-header`, `header-form`, `selection` and the buttons, and slots `rowForm` (`fields`, `rowIndex`), `row` (`fields`, `rowIndex`,
`selected`), `subRow` (`row`, `subRow`), `header` (`search`, `sortBy`, `orderBy`), `headerForm`, `nav`. The answer to a rows request is a list, or a map with the
list under the name of the form and `Count`, `PageIndex`, `PageSize`, `PageMaxIndex`, `PageRangeLow`, `PageRangeHigh` keys. The server-merged
macros write the rows themselves; this component is for the screens that fetch.

The shell is a `q-layout` with a `q-header` (menu button, the launcher, the current app's title, the screen path as `q-breadcrumbs`, Search,
the `+` menu, the Today, Chat, Inbox and Notifications buttons, the documentation, the notify history when there is no Notifications tab, and
the account menu: see Shell data and providers), a `q-drawer` with the screen menu (`QUASAR_LEFT_OPEN`; an overlay below 1024px), the page, and a `q-footer` with the
theme's footer items. The `LoadingBar` plugin runs while the root's `loading` count is above zero. `config.quasar` (brand, notify, loadingBar) is the
JSON of the theme's `STRT_QUASAR_CONFIG` resource, merged over the defaults.
The shell gives Quasar a stand-in for `vue-router` (`$router.resolve`, `push`, `replace`, and `$route`), so `q-btn to="..."` and `q-item to="..."` navigate with `setUrl`.
`moqui.webrootVue` is the root: `setUrl(url, bodyParameters)`, `setParameters(obj)`, `reloadContainer(id)`, `loadContainer(id, url)`,
`hideContainer(id)`, `addNotify`, `getLinkPath`, the screen context: `setScreenContext(key, value)`, `removeScreenContext(key)`,
`screenContext` and `assistReach`, and the suite shell: `shellData`, `shellCounts`, `openPalette()`, `openTray(key)`, `closeTray()`,
`openQuickCreate(item)` (below).

## Shell data and providers

The suite shell (IronStone BOSS design-drawers-commons §1, impl-drawers-commons M1) reads three things from the server beyond the menu
data of the current path. Every screen location in them is a link of the page, under `linkBasePath` (`/qapps/...`); to load one as a
component (an `m-dynamic-container` tab or an `m-dynamic-dialog`) put `basePath` in place of `linkBasePath` (`/apps/...`). A link is
there only when the user may view its screen, and a shell element whose value is null or whose list is empty is hidden (R27).

**`GET /shellData`** (the root screen's transition, read-only, 401 without a user; `crates/moqui-core/src/screen/shell.rs`), fetched
once per page load:

```
{ apps: [ { name, title, image, imageType, path,                       // an app root, as the app list shows it; path /qapps/<name>
            menu: [ { title, path, image, imageType, depth } ],        // its menu, depth 1 and 2, each entry followed by its children
            providers: {
              quickCreate:      [ { kind, label, icon, url, requiresRecord, sequence } ],
              todaySources:     [ { service, label } ],
              paletteActions:   [ { label, icon, url, sequence } ],
              paletteTypes:     [ docType ],
              paletteSources:   [ { group, label, icon, prefix, idPattern, sequence } ],   // no service name
              notificationTopics: [ topic ],
              keymaps:          [ { scope, keys: [ { keys, label, url, action } ] } ],
              accountMenuItems: [ { label, icon, url, sequence } ] },
            recordScreens: { chatter: { url, renderModes } | null,     // the theme's record panels this app root mounts (R5)
                             recordFiles: { url, renderModes } | null } } ],
  shell: { trayToday, trayChat, trayNotifications, quickCreate, mailScreen, messagesScreen,   // links, or null
           chatterScreen, recordFilesScreen },                                     // subscreen names under the current app root, or null
  counts: { ... } | null }
```

- `apps` are the subscreens of the apps root (`/apps`, `webroot/apps.xml`) in menu order that are in the menu, take no parameters and that
  the user may view, each authorized as the menu data authorizes a subscreen (a grant on a screen reaches the screens below it). An app
  root that does not load, or whose providers fail, is logged by name and location and left out; the others are answered. `menu` lists
  what the drawer would to depth two, with the menu data's checks (`menu-include`, authorization, a required parameter left out); a
  disabled entry is left out. The menu entries are the palette's Screens group.
- `providers` are the app root's `<shell-providers>` (`xml-screen-3.xsd`): labels localized, each list in `sequence` order (none counts as
  0), absent text as null. A quick-create kind with no `screen` opens the theme's `STRT_QUICK_CREATE` screen with `kind` added to its query;
  with neither it is left out. `parameter` children are added to a link's query. A provider whose screen the user may not view is left out,
  and one whose screen does not exist is logged and left out. `requiresRecord` kinds are offered only while `screenContext.record` is set.
  `action` of a key is a client action of the shell (`palette`, `quickCreate`) for a key with no `url`.
- `shell` are theme values of the user's theme for `STT_INTERNAL_QUASAR`, found as a render finds a theme (the user's `UserScreenTheme`, a
  group's, the type's `enumCode`, a `DEFAULT` theme of the type) and `DEFAULT_QUASAR` when there is none. They are sent here and not in
  `moqui-config`. `STRT_TRAY_TODAY`, `STRT_TRAY_CHAT`, `STRT_TRAY_NOTIFICATIONS`, `STRT_QUICK_CREATE`, `STRT_MAIL_SCREEN` and
  `STRT_MESSAGES_SCREEN` (the screens the header's Mail and Messages counts link to) are screen paths under the apps root; `STRT_CHATTER_SCREEN` and `STRT_RECORD_FILES_SCREEN` are subscreen
  names that every app root mounts (`Chatter`, `RecordFiles`), loaded as `<current app root>/<name>?recordEntity=&recordId=` so the app's
  authorization applies (R5).
- `recordScreens` are, for `chatterScreen` and `recordFilesScreen` of the theme, that subscreen of the app root (`/qapps/<app>/<name>`)
  with its `render-modes` as written (`["all"]` without the attribute), when the app root mounts it and the user may view it there;
  null otherwise, which is no error (the tools have no Chatter). A subscreen that does not load is logged and null.
- `counts` is what `org.moqui.impl.ShellServices.get#ShellCounts` answers for the user: the `counts` map of the service the theme names in
  `STRT_SHELL_COUNTS_SERVICE`, called with `userId`, with authorization disabled (the theme names it, it reads only that user's rows, and
  no app's authorization covers it), in the user's time zone and locale. The keys the shell reads, each a number or a map with a `total`:
  `today`, `chat`, `email` (external mail unread), `messages` (internal messages and the mailbox posts delivered to the Inbox),
  `notifications` (unviewed in the Notifications inbox, approvals included), `approvals` and `updates` (unviewed in the Updates inbox,
  which no header badge shows). Null when the theme names none, or the service is missing or fails (logged once per service name).

**`sri.getShellProviders(kind)`**, for a screen that only reads the providers (the tray's Today panel reads `todaySources`, the
Notifications panel `notificationTopics`): `{apps: [{name, title, image, imageType, path, providers}]}`, the apps and providers of
`getShellData` without the menus, record screens, theme values and counts; with `kind` (`quickCreate`, `todaySources`,
`paletteActions`, `paletteTypes`, `paletteSources`, `notificationTopics`, `keymaps`, `accountMenuItems`) each app's `providers` holds
that list alone, and another kind is an error. It is kept per session (visit), user, locale and theme while the app root screens are
current (a screen file changed in development, an app added) for up to ten minutes, and dropped on a login or logout in the session;
no user gives no apps. `getShellData` is unchanged.

**`GET /shellSearch?text=&groups=&limit=`** (read-only, 401 without a user): the palette's provider groups.

```
{ groups: [ { group, label, icon, prefix, rows: [ { title, subtitle, icon, url } ], moreUrl } ] }
```

The server runs the `palette-source` services of the apps the user may open, and only those: a request names groups (`groups`, a comma
list of group names, to narrow the search, as the client's prefixes do), never a service. The groups are in source `sequence` order across
the apps; the same group and service registered by two apps runs once. An empty `text` runs nothing. A source with an `id-pattern` runs only
when the whole text matches it. A source that fails is logged by group and service and left out, and so is one that answers no rows.
`limit` is the rows per group, 5 by default (at most 50).

**A palette source service** takes:

| In | Type | |
|---|---|---|
| `text` | String | the search text, trimmed |
| `limit` | Integer | the rows wanted |
| `docTypes` | List | the `palette-type` document types of the apps the user may open (for `search#Universal`) |
| `idMatch` | Boolean | true for a source with an `id-pattern` (the text is an id); absent otherwise |

and answers `rows`, a List of Maps (`title`, `subtitle`, `icon`, `url`, a `/qapps` link; text or empty), of which the first `limit` are used,
and optionally `moreUrl`, the link of the full search for the text. The service runs as the user, with the user's authorization and
entity filters, so it shows only what the user may see. `example.ExampleServices.search#GalleryEnumerations` is a small one.

**The `shellCounts` topic.** `org.moqui.impl.ShellServices.publish#ShellCounts(userIdList)` sends each user a NotificationMessage on the topic
`shellCounts`, not persisted and with `showAlert` false, whose `message` is the counts map (as `counts` above). A user gets at most one every
two seconds: requests inside that window fold into one send at its end. Inside a transaction the request waits for the commit, and the
sends are made on a server thread, so the caller never waits for the counts. The EECAs and SECAs on the records the counts read call it.

**`GET /shellCounts`** (read-only, 401 without a user): `{counts}`, what `get#ShellCounts` answers for the current user (null without a counts
service). The shell asks it every five minutes while the notification socket is down.

**Notifications.** A notification on the socket (topic `notifications` of the client, the wrapped message of `NotificationMessage`) has,
beside Java's keys (`topic`, `subTopic`, `sentDate`, `notificationMessageId`, `topicDescription`, `message`, `title`, `link`, `type`,
`persistOnSend`, `showAlert`, `alertNoAutoHide`), `reasonEnumId` (`NotificationReason`: `NrMention`, `NrAssignment`, `NrApproval`,
`NrDateAlert`, `NrReminder`, `NrComment`, `NrSystem`), `recordEntityName`, `recordId`, `actorUserId` (the sender unless set), `appName` and
`inboxEnumId` (the topic's `NotificationInbox`: `NinNotifications`, the default, or `NinUpdates`). A sender sets them with
`ec.makeNotificationMessage().reason('mention').record(entityName, id).actorUserId(id).appName(name)`. The screens read the current user's
notifications only through `org.moqui.impl.NotificationServices`: `get#UserNotifications` (by inbox, reason, app, topic, read state, dates,
text, approvals waiting; paged; with the unread of each inbox and the approvals waiting), `mark#NotificationsViewed` (ids, or all of an
inbox, which leaves the approvals waiting: an approval is decided, not read), and `approve#NotificationMessage` and
`refuse#NotificationMessage`, whose decision is final for the message (every recipient's row is marked viewed, so the others no longer
see it waiting; an approval that each of several users decides is a message per user), which call the services the topic names
(`approveServiceName`, `refuseServiceName`) with `recordEntityName`, `recordId`, `message` (the messageJson map) and, for a refuse,
`reason` (required when the topic has `refuseReasonRequired`).

**Screen context.** The `context` element (`<context record-entity record-id title selection-form/>`) writes an `m-screen-context`; in a
server-static screen its attributes are bound over the screen's actions data. The root keeps the registrations of the mounted ones in
`screenContexts` and exposes the latest as:

```
screenContext: { path, parameters,                         // currentPath (/apps/...) and currentParameters
                 record: { entity, id, title } | null,     // with an entity and an id (also screenRecord)
                 selection: { form, ids } | null,          // the selection-form list and the values of its selected rows
                 filters: [ { form, parameters } ] | null, // the current parameters that are find fields of a list or find form
                 forms: [ { name, kind, fields, findFields?, rows? } ] }   // kind form, find or list
assistReach: ["m-canvas", "m-tray"?, "m-quick-create"?]   // ids Assist may reach: the q-page-container, the tray while open, the
                                                           // quick-create dialog while open; never the header, drawer or palette
```

`forms` comes from the forms mounted now (`moqui.screenForms`: each `m-form`, `m-form-link` and `m-form-list`, and the `m-checkbox-set`
of a list's selection; `moqui.screenFormsContext` gathers them): a single form by its name with its field names, a find form (`m-form-link`)
as `find`, and a list as `list`. A list is known by its `<list>_table` element: its row forms (`<list>_<n>`) fold into it (`fields`, `rows`)
and its header find form (`<list>_header`) gives `findFields`; an `m-form-list` and a multi form are lists too. `filters` are, for each list
or find form, the current URL parameters with a value that are its find fields (with their `_op`, `_not`, `_ic`, `_from`, `_thru` and period
companions). `selection.ids` are the values of the checked rows of the selection-form list (`rows` for an `m-form-list`). A dialog's
screen with a context registers after the page's and is the latest until it is unmounted.

### The shell in the client

`m-qapps-root` fetches `/shellData` once when the page starts and again after a re-login, and keeps it as `shellData` (`shellCounts` holds the
latest counts). The header, left to right (IronStone style-guide §2.2, design §1.2):

- **Launcher**: the theme's logo as a button whose `q-menu` is a grid of `shellData.apps` (their `m-menu-icon` icons), the current app
  outlined, and "All applications" to the AppList. The title is the current app's (`currentApp`, the first path name), the theme's outside one.
- **Search** opens the palette; a label and the key hint (Ctrl K, ⌘K on a Mac) from 1024 px, the icon alone below.
- **`+`**: the `quickCreate` kinds of every app, the current app's first, once per link; a `requiresRecord` kind only while there is a record
  in the screen context. A kind opens its link (as `/apps/...`) in `m-shell-quick-create`, an `m-dialog` with an `m-dynamic-container`, with
  `recordEntity`, `recordId` and `recordTitle` from the screen context. An `m-form` in it with no `submit-hide-id` closes it when the answer has
  no errors (it injects `mSubmitHide`); going to another screen closes it too. A quick-create screen answers its form with JSON (messages in
  `messageInfos`) and no `screenUrl` to stay on the page (example `Gallery/QuickNote.xml`), or with a `screenUrl` to open the new record. A
  message info may carry a `link` and `linkText`: `{messageInfos: [{message, type, link, linkText}]}` notifies with a button (`linkText`, "Open"
  by default) that follows it (`moqui.openLink`: an app link through `$root.setUrl`, an external one in a new window), so a quick create can
  offer the record it made and stay. A transition's `<message>` has no link; `moqui.notifyMessages` takes texts and maps alike.
- **Today, Chat, Mail, Messages, Notifications**: each only when its theme value is set (R27). Today, Chat and Notifications open the tray on
  their tab; Mail (`m-counter-email`) links to `shell.mailScreen` and Messages (`m-counter-messages`) to `shell.messagesScreen`. Below 600 px
  Mail and Messages are one button (`m-counter-mail`) to the mail screen whose badge is their sum, the tray tabs one tray button
  (`m-counter-tray`, the Notifications icon) whose badge is their sum and that opens the tray on its last tab (its tabs switch), and the app
  title gives way to the launcher, so a phone header holds menu, launcher, Search, `+`, the tray, Mail, the theme plugins and the account. Badges are `shellCounts.today`, `chat`,
  `email`, `messages` and `notifications` (a number, or a map's `total`); `updates` has no badge. Without a Notifications tab the
  notify-history menu stays. The screen history is the palette's Recent.
- **Account menu**: the user's avatar with their presence dot as its button; in the menu, the status (`m-user-status-menu`, below) after the user's
  name, then the `accountMenuItems` of every app. The Chat button has the user's own dot too. The right `q-drawer` slot stays free for Assist (R1).

**Tray** (`m-shell-tray`, id `m-tray`): a `q-page-sticky` at the right of the canvas, inside the `q-page-container`, 400 px from 600 px and the
full width below, over the screen. A tab per tray screen of the theme; each is an `m-dynamic-container` of its link (as `/apps/...`) mounted on
its first opening and then kept, so its state lasts across navigation. The root keeps the open state, the tab and each tab's scroll (also in
`sessionStorage`, for a reload). Esc and the close button close it. On the notification socket, `shellCounts` sets the badges and reloads the
open tab; `todayItems` reloads Today, a `chatRoom:` topic Chat, and `notifications` or an app's `notification-topic` Notifications. A reload keeps
the tab's content until the new one is there (`m-dynamic-container.reload`). While the socket is down the counts are fetched every five
minutes (`/shellCounts`) and the open tab reloaded.

**Palette** (`m-shell-palette`, id `m-palette`): a `q-dialog` at the top, Ctrl+K or Cmd+K anywhere but in a rich-text editor (which keeps
Ctrl+K). Groups (`moqui.shellPalette.groups`): Recent (the screens of this page before the current one) before any text; then Screens (the
apps' menus by title), Saved finds (the current menu data's), Actions (the apps' `paletteActions`), and the `shellSearch` groups, asked
250 ms after the text stops changing, each up to five rows with a "more" row to its `moreUrl`. `>` shows the actions alone, `:` the screens
alone, and a source's prefix (`@`, `#`, `~`) only its sources' groups (`groups` of the request). The arrow keys move, Enter opens the selected
row (the first by default), Ctrl+Enter or Cmd+Enter opens it in a new tab.

**Keys** (`moqui.keymap`, design §1.7): `registerScope(scope, bindings)` returns the function that takes them back; a binding is
`{keys, label, run(event), action, when(), notInEditor}`, keys like `Mod+K` (Ctrl or Cmd), `Ctrl+Shift+A`, `Esc`, `?` or the chord `g c`
(1.5 s). A screen's or `.qvue` component's scope is tried before `global`; a text input, select or editable element keeps the single keys it
types; while a dialog is open single keys wait. The shell's own: Mod+K the palette, Esc closes the tray when no dialog or menu is open (Quasar
closes those), `?` the overlay of the keys on now, `c` the current app's first `+` kind. The apps' `keymap` providers: an empty or `global`
scope everywhere, another while its app is the current one. The `KEYMAP` user preference (`config.preferences`) changes keys:
`{"global": {"palette": "Mod+P", "g c": "g x"}}`, by scope, the binding's `action` or else its keys.

## Preferences

The user's preferences in the page are `config.preferences` (the keys the user has set, read through `ec.user.getPreference`),
`config.density` and `config.startApp`. **`moqui.applyPreference(key, value)`** keeps a new value there and applies at once one
the shell reads: `QUASAR_DARK` (`$q.dark`), `QUASAR_LEFT_OPEN` (the drawer), `KEYMAP` (`moqui.keymap.setOverrides`) and
`SHELL_DENSITY` (`moqui.applyDensity`: `compact` puts `m-density-compact` on the body, whose rules in `../css/WebrootVue.qvt.css`
tighten the page, boxes, list rows, fields and menu items; anything else is the comfortable default). Another key is only kept.
`moqui.applyPreferences(map)` applies a map. **The server tells the page**: a response to a request that set preferences
(`ec.user.setPreference`, `set#Preference`) has the header `X-Moqui-Preferences`, the JSON map of what it set (null for a removed
key) percent-encoded as `encodeURIComponent` encodes; `moqui.request` decodes it and calls `moqui.onPreferences`, which is
`moqui.applyPreferences`. So a Preferences screen that saves through its own transition, whatever it answers, changes the page
without a reload; one that saves in the browser alone calls `moqui.applyPreference`. `SHELL_START_APP` applies to the next
`/qapps` page (`moqui.startPath`): the server sends `startApp` only for an app the user may open (`sri.getShellProviders`).

## Record panels, rich text, avatars and record cards

IronStone impl-drawers-commons M3: the framework pieces Chatter and record files are built from. The framework knows no data model
behind them; BOSS supplies the screens and the transitions.

**Record panels.** `<chatter record-entity record-id default-tab hide id/>` and `<record-files record-entity record-id id/>`
(`xml-screen-3.xsd`) are an `m-chatter` and an `m-record-files`; in a server-static screen their attributes are bound over the
actions data. Each loads, into an `m-dynamic-container`, the subscreen of the current app root (`currentAppName`, the first name of
the path) that the theme names (`shell.chatterScreen`, `shell.recordFilesScreen`) as that app's `recordScreens` entry gives it,
as its own request, so the app's authorization and entity filters apply (R5):

```
/apps/<app>/<name>[.qvue|.qjs]?recordEntity=<entity>&recordId=<id>&appRoot=<app>[&defaultTab=<tab>][&hide=<list>]
```

`.qvue` (or `.qjs`) when the screen's render modes name it, as the menu data's `renderModes` choose for a screen. A `.qvue` panel
does not see the query of its own request, so it reads the same values with `inject: ['mRecordPanel']`:
`{kind: 'chatter' | 'recordFiles', recordEntity, recordId, appRoot, defaultTab, hide, containerId}`. `defaultTab` is the
compose tab to open on (`message`, `log`, `activity`; design §15 per-app defaults) and `hide` a comma list of what to leave out
(`message`, `activity`), both for the panel to read. The element shows nothing, and says why once in the console (`console.info`),
when the theme names no screen, the page is in no app, or the app does not mount it; without an entity and an id it shows nothing.
A new record id loads the panel again (`m-dynamic-container.load`). `moqui.recordPanelUrl(shellData, appName, kind, record, basePath,
linkBasePath)` makes the URL. In html the element is a plain link to the screen when the html theme names one (`ScreenHtml.js`),
else nothing; the other modes write nothing.

**Rich text** (`m-rich-text`). Tiptap 3 (`libs/tiptap/tiptap.min.js`, the global `Tiptap`, built by `libs/refresh.mjs`), loaded
once for the page when the first editor mounts. The full toolbar has the paragraph style (paragraph, headings 1 to 3), bold,
italic, strike, the two lists, link and image (an address asked in a line under the toolbar; Ctrl+K for a link), code, code block,
quote, undo, redo and the plain-text toggle; `light` has bold, italic, link, list and code, for notes and chat. Markdown-style typing
(`**bold**`, `- `, `> `, `` ` ``) works in both. The value is HTML, `""` for an empty document, made only of what a service
parameter with `allow-html="safe"` keeps (`crates/moqui-core/src/service/validate.rs`, ammonia's defaults plus `data-mention-id`):
`p`, `h1`-`h3`, `strong`, `em`, `s`, `u`, `code`, `pre`, `blockquote`, `ul`, `ol` (`start`), `li`, `br`, `hr`, `a` (`href`, `title`;
no `target` or `class`, and the cleaning adds `rel="noopener noreferrer"`), `img` (`src` http, https or a path, `alt`, `title`; no
`data:`) and the mention:

```html
<span data-mention-id="EX_JOHN_DOE">@John Doe</span>
```

The id is what the mention search answered, for the server to notify (`example.ExampleServices.clean#GalleryNote` reads them back);
the label is shown. **The mention search**: typing `@` asks `mention-url` (GET) with `term` (the text after the @, maybe empty) and
the `mention-parameters`, 150 ms after typing stops, and takes a JSON list of `{id, label}` (also `value` for id, `title` or
`name` for label, an optional `image` for the avatar, or the list under `options`); the first ten are offered, up and down arrows
move, Enter or Tab or a click picks. **The plain-text toggle** turns the value into plain text (`moqui.richText.toPlain`: blocks
apart by a blank line, list items with `- ` or their number, quotes with `> `, a link as its text and address) and back into
paragraphs (`fromPlain`); a mention becomes its `@Label` text. A host that sends mail as text reads `v-model:plain`. When the
library does not load, the field is `m-text-area` with `editor-type="html"`. The pure parts are `moqui.richText` (`mentionHtml`,
`mentions`, `isEmpty`, `value`, `toPlain`, `fromPlain`, `mentionItems`, `linkHref`, `toolbar`), tested in
`WebrootVue.qvt.records.test.js`. In XML: `text-area editor-type="rich"` or `"rich-light"` with `mention-transition`.

**Record cards** (`m-record-card`). The card JSON a `card-url` answers, or the `card` prop (BOSS answers it from
`mantle.GeneralServices.lookup#ById`):

```
{ title, typeLabel, icon, id, entity, url,                  // url a /qapps link; title falls back to the id, icon to sym_o_description
  status: { label, color } | "label",                        // a q-chip m-tonal text-<color> (primary without one)
  fields: [ { label, value } ] }                             // the first two with a value are shown
```

or that object under `card`. A failure or an answer that is not a map shows the entity and id (logged with `console.info`).
`moqui.recordCard.normalize` reads it.

**Email bodies** (`m-html-view`, IronStone design-drawers-commons §12 reading pane). DOMPurify (`libs/dompurify`, loaded with the
first view) cleans the HTML: the HTML profile only (no SVG or MathML), and no script, style element (the message's style blocks
are taken out first and cleaned as CSS), form or control, frame, object or embed, base, link, meta, media or template, and no
`srcset`, `poster`, `ping`, `formaction` or data- attribute of the sender. `moqui.htmlView.process` then walks the result:

- **Images.** `data:` images stay; `cid:` ones come from `cid-map`, whose URLs (this origin only, with the session) are read into
  `data:` URLs first, since the frame shows no other; a remote (`http`, `https`, `//`) image is a placeholder of its size with its
  alt text until "Show images" (the bar, or `allow-remote` set by the host for a trusted sender), and a tracking pixel (at most
  2 by 2) is dropped; a relative or `javascript:` source is dropped. `remote-images` reports the count, CSS backgrounds included.
- **Links.** `http`, `https`, `mailto`, `tel` and anchors stay, with `target="_blank"` and `rel="noopener noreferrer"`; any other
  `href` is removed (the text stays). A `target` elsewhere is removed.
- **CSS.** In style attributes and style blocks: remote `url()` only when images are allowed, no `@import`, `expression()`,
  `behavior` or `-moz-binding`, no fixed positions; backslash escapes are read first, so none hides a `url()`.
- **Reply history.** The quoted part at the end (Gmail `gmail_quote`, Apple Mail and Thunderbird `blockquote type=cite`,
  Outlook `#divRplyFwdMsg` and what follows it, Yahoo, a "-----Original Message-----" line, or a last `blockquote`) goes into a
  `<details class="m-hv-quote">` behind a "•••" summary, which opens without a script. A quote with a reply after it stays.
  A text body is escaped, its URLs and addresses made links, and its trailing `>` lines with the "On ... wrote:" line before
  them collapsed the same way.

The frame is `<iframe sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox" referrerpolicy="no-referrer"
srcdoc="...">`: no `allow-scripts`, so nothing in the message runs even if the cleaning missed it; `allow-same-origin` only so the
host can measure the content (with no script in the frame it cannot use it); links open in a normal window. The document's
policy (`moqui.htmlView.csp`) is `default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; base-uri 'none';
form-action 'none'; frame-src 'none'`, with `data: https: http:` for images and fonts once images are allowed, and a
`<meta name="referrer" content="no-referrer">`.
The host sizes the frame to `#m-hv-root` after it loads and again when the frame's width or content changes (a
`ResizeObserver` from the host), an image loads or a quote opens: the frame is as tall as the message and the page scrolls,
not the frame. Dark mode: a plain message (no colors or backgrounds of its own) takes light text on the dark card; a designed
one stays on a light card (`m-html-view-card`). Without DOMPurify (the script blocked) the view shows the message's text. The
pure parts (`imageSource`, `linkHref`, `css`, `isDesigned`, `autolink`, `textToHtml`, `quoteStart`, `quoteKind`, `collapse`,
`process`, `splitStyles`, `csp`, `srcdoc`) are tested in `WebrootVue.qvt.htmlview.test.js` over a small fake DOM; the sandbox,
the policy, Show images, the quote toggle, the height and dark mode are `gate3_browser.rs`
`the_html_view_cleans_sandboxes_and_sizes_a_message_in_chromium` on the gallery's Message screen.

In XML: `<html-view html="${message.body}" text="${message.bodyText}" allow-remote="trustedSender" cid-map="cidMap"
collapse-quotes="true" dark="auto"/>` (with `id`, `condition` and `style`). The HTML and text are attribute values, never
template text, so nothing in a message is compiled; a server-static screen binds them over its data. The html mode cleans the
HTML on the server (`StringUtilities.cleanHtml`, the safe set without styles) into a frame sandboxed without scripts that loads
no image but `data:`; the text and print modes write the text (or the HTML's text).

## Layout panels: split, wizard and stack

**`m-container-split`** (style-guide §3.8). From 1024 px (`$q.screen.gt.sm`) a `q-splitter` with `unit="px"`: the side at
`side-width`, draggable from 200 px to the container's width less `min-main` (`moqui.splitLayout`, which a `ResizeObserver` on the
container feeds). A dragged width is kept in `sessionStorage` (`moqui.SPLIT_WIDTH_<id>`) at once and in the user preference
`SPLIT_WIDTH_<id>` once the dragging stops; a new page reads the session value, then the preference snapshot of the page
(`config.preferences`), then the attribute. Below 1024 px, or in a container narrower than 200 px plus `min-main`, the same
splitter is stacked by the style sheet (`m-split-stacked`), so the panes keep their content: with a `select-parameter`, a value of
that parameter in `$root.currentParameters` shows the main pane with a back button (it goes to the screen without the parameter)
and none shows the side; without one both panes show, the side first. The selection is read in the browser, so a server-static
screen uses it as it is. `data-stacked` on the element says which layout is shown. Side by side, the splitter is as high as the
`height` attribute, else the viewport below the split's top (less 16 px, at least 360 px; `moqui.splitHeight`, measured on mount,
on a resize of the window and of the container), so a long side list scrolls in its own pane and the page does not; stacked, the
page scrolls.

**`m-subscreens-wizard`** and the step protocol (style-guide §3.10). The steps are the subscreens of the wizard's screen in
the menu data (`navMenuList[pathIndex].subscreens`, the index read before its `m-subscreens-active` registers), so the current
step is the subscreen in the URL. Each `m-form` registers with the `mWizard` it injects; the **step form** is the first of them in
document order that is inside the step and not in a table, a form-list or a dialog. A step screen's form posts to its own
transition, and normally has no submit button of its own (one it has does the same as Next).

1. Next with a step form submits it (`submitForm`: the Quasar rules first; a refused validation or a failed request leaves the
   wizard where it is). A step without a form goes to the next step at once.
2. The answer of the step form goes to the wizard instead of `m-form`'s own `screenUrl` handling (`moqui.wizardAfterAnswer`):
   errors or validation errors stay (the field errors show on their fields); `redirectUrl` is followed; a `screenPathList` that is
   not the current step (the transition's `default-response` named another screen, such as `../Review` to skip a step or the
   record the wizard made) is followed with `screenUrl`; otherwise, on a step before the last, the wizard goes to the next step
   with the current parameters, the step's own and the answer's `screenParameters` (`moqui.wizardStepUrl`), so a transition passes
   the draft record's id with `<default-response url="."><parameter name="importId"/></default-response>`. The last step's answer
   is followed when it names a screen and stays when it does not. A step form's answer adds no "Submit successful" toast.
3. Back, and the header of a done step, go to that step without submitting; a step shows what it saved before, so a wizard that
   can be abandoned keeps a draft (a record, or the session as the gallery's does).
4. While the step form posts (by Next, by Enter or by a submit button of its own: the form tells the wizard, `stepSubmitting`) a
   `q-inner-loading` spinner covers the step and Back and Next wait; it stays until the answer says to stay, or the screen it goes
   to (the next step, or the last step's own result) is there (`setUrl`'s completion), so a long last step (an import, a run)
   shows that it is working.

**`m-subscreens-stack`**: the first subscreen in menu order is the bottom of the stack; any other active one shows a breadcrumb
back to it. The html mode writes the wizard's steps as a numbered list of links with Back and Next links, and the stack's
breadcrumb (`moqui-html.css` `.wizard`, `.stack-crumbs`); a split is two columns (`.split`) stacked below 1024 px with
`data-selected` and a back link.

## Chat store and presence

IronStone impl-drawers-commons M10 (design-drawers-commons §1.8, §13): `WebrootChat.js` is `moqui.chat`, the page's store over the
notification socket, started by the shell root once mounted for a signed-in user. The server side is `org.moqui.impl.ChatServices`
(`framework/service/org/moqui/impl/ChatServices.xml`, which describes the rooms, the access rules and the pushes); BOSS's Chat screens use
the store for the live parts and their own transitions over those services for the rest.

**The pushes** are NotificationMessages that are not persisted and show no alert, routed by the socket to their users only:

```
chatRoom:<chatRoomId>  {kind: 'event', chatRoomId, eventSeq, event}           // to the room's members: a new event, as get#Timeline lists one
                       {kind: 'typing', chatRoomId, userId, typing}            // to the other members (signal#Typing; nothing stored)
                       {kind: 'read', chatRoomId, userId, lastReadEventSeq}    // a member's read marker (read receipts)
chatUser:<userId>      {kind: 'room', action: 'joined'|'left'|'changed'|'role'|'settings', chatRoomId}
                       {kind: 'read', chatRoomId, lastReadEventSeq} | {kind: 'read', all: true}   // read in another tab
userStatus             {userId, presence, statusText, emoji, clearAtDate, inCall}   // to the user's room members; self and status to the user
```

`presence` is `online`, `away`, `dnd` or `offline` (an invisible user is offline to others). The shell's own `shellCounts` topic carries the
header's Chat badge (`counts.chat`, the app's counts service adding `get#UnreadCounts`).

**The store** (`moqui.chat`; its `state` is a Vue reactive object):

| | |
|---|---|
| `state` | `me` (the user's status: `status` as chosen, `presence`, `statusText`, `emoji`, `clearAtDate`, `autoAwayMinutes`), `presence` (userId to `{presence, statusText, emoji, clearAtDate, inCall}`), `unread` (chatRoomId to `{unread, mentions}`), `typing` (chatRoomId to userId to the time it ends), `reads` (chatRoomId to userId to lastReadEventSeq), `openRooms` |
| `on(kind, fn)` | a listener of `event`, `typing`, `read`, `room` or `presence`; returns the function that removes it. While a screen listens to `event` or `room`, a chat push no longer reloads the tray's Chat tab (`trayReloads`); a typing or read signal never does |
| `openRoom(id)`, `closeRoom(id)` | a room shown now: its events do not count as unread, and the socket is told of its topic |
| `typing(id, false?)` | tell the room the user is typing, at most every 3 seconds, and once when it stops; `typingIn(id)` the users typing there |
| `presenceOf(userId)`, `statusOf(userId)` | the dot (`""` while not known) and the status; `watchUsers(ids)` asks for them, the asks of the moment in one request, each user once in five minutes |
| `myPresence()`, `quiet()` | the user's own dot (invisible is grey) and do not disturb: while it is on the page's toasts are off (`notificationClient.disableDisplay()`) and a screen plays no chat sound; the notifications are still recorded and counted |
| `setStatus({status, statusText, emoji, clearAt, clearStatus})` | `clearAt` is minutes, `today` or `week` (the end of today or of Sunday, in the browser's zone) |
| `applyEvent(list, event, me, threadRoot)` | merge a pushed event into a timeline list as `get#Timeline` answers it: a message added in its place once (a thread reply counts on its root unless the list is that thread), an edit, a reaction, a redaction (of a message or a reaction) and a pin applied |
| `reduce(state, jsonObj, me, now)` | the socket message on a state (pure; the store's listener) |

**Auto-away.** The store notes the user's input (keys, pointer, wheel, touch, focus, the tab coming back) and posts a heartbeat every minute
with the seconds since the last input; the server sets away after the user's `CHAT_AUTO_AWAY_MINUTES` preference (10 by default) and
offline 3 minutes after the last heartbeat (`expire#UserStatuses`, each minute, publishes those changes and clears a status at its clear-at
time). Input after an idle beat past the auto-away minutes beats at once, so the others see the user back.

**The webroot transitions** of the store, each as the current user (401 without one): `GET /userStatus` `{myStatus}`, `POST /setUserStatus`
(`status`, `statusText`, `emoji`, `clearAtDate` in epoch millis, `clearAfterMinutes`, `clearStatus`), `POST /userStatusHeartbeat`
(`idleSeconds`), `GET /userStatuses?userIds=a,b` `{statusMap}`, `POST /chatTyping` (`chatRoomId`, `typing`) and `GET /chatUnread`
`{total, unread, mentions, roomCounts}`.

**The account menu's status**: `m-user-status-menu` (the current status and its message, the four statuses, "Set a status message", "Clear status
message") and `m-user-status-dialog` (emoji, text up to 100 characters, clear after: don't clear, 30 minutes, 1 hour, 4 hours, today, this
week), outside the menu so it outlives it.

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
  date or time from the pattern for a string; epoch millis text is a time stamp with any date pattern, and with no pattern, as `ec.l10n.format`
  on the server), `merge`, `url` / `qs` (path and query from a parameter map), `link` (the url a `${}` expression of a link or number-card gave:
  a relative path resolved against the screen's own path, as the server render resolves a link's url; an absolute path or an external URL as
  it is), `shown` (`visible-when`), `btnKind` and `btnText` (the color and classes of a link or menu button whose `btn-type` is an expression, as
  `ScreenQvt.js` writes them), `cols` (the classes of a `row-col` whose sizes are expressions).
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
