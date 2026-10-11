# Context objects

Which values QuickJS and the browser can see, and which Rust types those environments share. Server text goes through one of the template renderers in `plans/Architecture-Proposal.md`. The macro modules that expand XML are the same runtime, with a narrower input.

This follows Templating in `plans/Architecture-Proposal.md` and the browser host in `plans/QvtServerStatic.md`. The method list on server `ec` facades and on `sri` is Server host methods, below.

## Decision

There are four places a value is visible. The three server places are one QuickJS runtime. The fourth is the browser.

| Environment | When it runs | What it is given |
|---|---|---|
| QuickJS scripts, `JsTemplateLiteral`, and the trusted fancy renderer | Every server evaluation: XML actions, `<script>`, attribute expressions, `ec.resource.expand()`, a trusted fancy template, and a server screen render (html, text, csv, xml, xsl-fo, and qvt when it is not server-static). | The live context stack, `ec`, and during a screen render `sri`. Entity values and lists stay host objects. |
| XML action and screen macro modules | Once, when an action or screen definition is compiled. The generated text is cached. | The element being compiled: attribute strings, child element text, and the already-rendered output of its children (or a `visit` function). No `ExecutionContext`, no context stack, no entity value. |
| Locked-down fancy renderer | When a template is rendered with the restricted renderer. | A plain snapshot of the map the caller passed. Format and localize functions. No `ec`, no `sri`, no live entity. |
| Browser | A server-static qvt screen, after the template is cached. | The JSON package from `plans/QvtServerStatic.md`, plus the restricted browser `ec` that plan already lists. |

String expansion (`ResourceFacade.expand`) is `JsTemplateLiteral` on that runtime. A string with no `$`, or with `$` as the last character, is returned as itself. Localization of an input shorter than 256 characters still happens first, as `ResourceFacadeImpl.expand` does. The `${name}` expansion in `moqui-lib` stays string replacement against the environment and does not see the context stack. On the server, embedded expressions run in QuickJS. In the browser, `ec.resource.expand` uses the restricted host.

The macro module does not run on the request path. A screen function that today calls `sri` or `ec` while FreeMarker is rendering emits a JavaScript call instead. That call runs later, in QuickJS or in the browser. Short framework text uses `JsTemplateLiteral` and the live context. Structured component text uses the trusted fancy renderer. Text that must not be a script uses the locked-down renderer and the snapshot.

## What Java does, and what changes

`MNode` implements the FreeMarker node interfaces and caches the wrappers on the node. That pays off because screen FreeMarker runs on every render and looks up `@attr` and child elements through those interfaces. Everything else reaches FreeMarker through `BeansWrapper` reflection. `ContextStack` is only a `Map`.

The two FreeMarker jobs already have different data models:

- `XmlActions.groovy.ftl` is compiled once. Its data model is one entry, `xmlActionsRoot`, the `MNode`. `<#visit>` walks the node. The names `ec`, `context`, and `result` appear in the generated Groovy. They are not in the FreeMarker data model. The generated script header says so.
- Screen macros run on every render. `ScreenWidgetRenderFtl` passes the whole `ContextStack` as the data model, after putting `sri` and the widgets node on it. `ec` is already on the stack. Macros both walk `.node` and call `sri` and `ec` while they write output.

The macro module is JavaScript on the QuickJS runtime, so a function can walk a node when the walker passes one. The Rust walker owns the default visit. For each element it calls `macros[elementName](node, children)`. `children` is the already-rendered output of the child elements, and the function places that string, including inside a loop it emits. Attribute strings are copied into that call. The copy happens once per definition, and the cached result is the generated script. That replaces the `MNode` wrapper cache. `Node` does not implement an engine trait. The function is not given the request context. Dispatch by element name is an ordinary property lookup, which is why the library is a JavaScript module rather than a template that cannot call a macro through a variable.

## One stack, three projections

`ContextStack` stays in `moqui-lib`. It is a stack of maps. It implements no QuickJS trait. `moqui-lib` does not depend on `rquickjs`. A type in lib cannot implement that trait anyway: the orphan rule requires the crate to own the type or the trait. The core runtime reads the stack through the proxy below.

QuickJS is the live view. Bare names in an expression (`orderId`, `statusId`) resolve on the stack, the way the Groovy binding and the FreeMarker data model do. `context` is that stack. `ec` is the host object for the execution context and is also the `ec` entry on the stack. Assignments write the top map. A service `result` map is present when the current actions provide one, as today. The QuickJS runtime is the one already owned by that `ExecutionContext`. Values created for a call die with it. They are not stored on a process-wide runtime.

### Codegen rule: bare names

This is the contract for macros and for migrated scripts. It is decided before macros are written.

Groovy returns null for an unset bare name. `ContextBinding.hasVariable` always returns true (`ContextBinding.java:41-45`), so the get falls through to `contextStack.getByString` and yields null instead of `MissingPropertyException`. `if (someUnsetParam)` is false. In JavaScript the same line throws `ReferenceError` at runtime. Identifier lookup calls `HasBinding` before `Get`, so a get-only trap never runs for an unset name.

The server runtime sets the prototype of `globalThis` to a `Proxy` for that `ExecutionContext`: `Object.setPrototypeOf(globalThis, new Proxy({}, { has, get, set }))`. The proxy closes over that thread's `ContextStack`. Assigning a `Proxy` to `globalThis` does not change identifier lookup, and an unset name still throws `ReferenceError`. The proxy is not installed on the browser `window`. `with` is not used. Strict mode rejects `with`. Evaluated scripts are strict, so an assignment goes through the `set` trap instead of creating an own global.

- `has` returns true for every name that is not already an own property of the real global. An unknown name still has a binding, so the read does not throw.
- `get` returns the stack value when any map on the stack contains the key, including an explicit null. Otherwise it returns a host helper (`ObjectUtilities`, `CollectionUtilities`, `StringUtilities`). Otherwise it returns null. Those helpers are not own properties of `globalThis`. A context key of the same name wins.
- Own properties of the real global (`Array`, `Math`, `Object`, and the other built-ins) are resolved by the engine before the proxy. A context key cannot hide those names.
- `set` writes the top map, as `ContextBinding.setVariable` does. It does not create a real global.
- An unset name is null, so `if (unset)` is false. `typeof unset` is `"object"`, because the value is null. A presence check is `== null`.

Macros emit bare names. They do not rewrite reads to `context.foo`. `context.foo` still works because `context` is on the stack. Server-static qvt expressions are Vue template expressions against the JSON package. A missing key renders empty there. A new client script that mentions an unset name can still throw. That is acceptable. The proxy is not how the browser works.

Expression hazards that change what a macro emits, including Elvis versus `??`, empty collections, `==`, and Groovy ranges, are the table in `plans/Architecture-Proposal.md` (What changes in expressions). Macros follow that table. `??` is only for a null check. An empty string or a zero uses logical or. An empty list or map tests length. `==` stays `==`. A Groovy inclusive range becomes `slice` with an exclusive end.

The locked-down fancy renderer and the browser see a snapshot, not the stack. The snapshot is the same projection as `ContextJavaUtil.unwrapMap`:

- Strings, booleans, numbers, and timestamps pass through. Timestamps become ISO-8601 text in JSON and in the locked-down template. A decimal becomes a string in JSON and in the locked-down template. It does not become a JavaScript number. String order and `+` are then lexicographic and concatenation (`"10.00" < "9.00"`, `"1.00"+"2.00"`).
- An entity value becomes the plain field map. An entity list becomes a list of those maps.
- `EntityFind`, the execution context, `sri`, and the context stack object are dropped.
- Anything else is dropped and logged, as `unwrap` does today.

The locked-down template's plain values and the browser's JSON are two sinks of that one projection. The projection is the shared code. QuickJS host objects are not.

The browser package is that projection plus two filters from `plans/QvtServerStatic.md`: credential parameters are stripped, and form-list lists and pagination keys are removed. A server-static expression that does decimal arithmetic, or that calls `ObjectUtilities` for it, is a compatibility-report violation. Formatting goes through `ec.l10n`. On the server, a decimal stays a host value.

A live entity value inside the locked-down template would let it walk a relationship and read through to the database. The snapshot is that boundary. Callers pass the map the template is allowed to see. Format and localize call the same functions as `ec.l10n` on the server. They do not take an `ExecutionContext`. `JsTemplateLiteral`, the trusted fancy renderer, and scripts are not this path. They see the live context, which is the same trust as a component-authored script.

## Host objects in QuickJS

These are the types a script can hold. Plain maps and lists that are already plain data are ordinary JavaScript objects, copied in at the boundary. The types below stay Rust values behind getters so a screen or a service does not clone the working set into the engine.

| Name | Visible as | Rust value |
|---|---|---|
| context keys | Bare names, and `context.name`. On the server an unset name is null, via the proxy above | The `ContextStack` |
| `ec` | Host object | The `ExecutionContext` and its facades |
| `sri` | Host object, only while a server screen render is on the stack | The screen renderer. Absent in actions that are not a render, and absent in the browser |
| entity value | Host object, field get and set | The entity value. `unwrap` is what the locked-down template, JSON, and the browser receive instead |
| entity list | Host object, index, length, iterable, `map`, `filter` | The list. `map` returns a JavaScript array so `list.map(x => x.field)` works without copying first |
| decimal, timestamp | Host values | `rust_decimal` and `time` from `moqui-lib`. Not JavaScript numbers. Operators are not overloaded. `valueOf` throws an error naming the method to use, and `toString` prints, so `` `${decimal}` `` interpolation works. `+`, relational operators, and `==` against a number or a string throw. `==` between two decimals does not call `valueOf` and is reference equality, so it returns false. A decimal is always truthy, including zero, so `if (amount)` and `amount || fallback` do not match Groovy. The host methods are `add`, `subtract`, `multiply`, `divide`, `eq`, `compare`, and `isZero`. `divide` uses scale `max(left, right, 10)`. Checked operations return a script error when the 28-digit coefficient overflows. The browser package and the locked-down template still receive a decimal as a string. A server-static expression that does decimal arithmetic is a compatibility-report violation (`plans/QvtServerStatic.md`). That is the migration rule in `plans/Architecture-Proposal.md` |
| `ObjectUtilities`, `CollectionUtilities`, `StringUtilities` | The same helpers the generated script header calls today | Functions in lib, bound once per runtime. The proxy returns them when the stack has no such key. They are not own properties of `globalThis` |

`ec.entity`, `ec.service`, and the rest of the facade surface are the Server host methods section. They exist only on this QuickJS object. The browser `ec` is the table in `plans/QvtServerStatic.md` and does not grow to match the server object. `sri` is not added to the browser.

Generated screen JavaScript for a server render calls `sri` and writes to the render writer. Generated server-static qvt does not. It transcribes expressions into the Vue template, and the browser evaluates them against the package and the restricted `ec`.

## What is reused

- `Node` and `ContextStack` in lib. One tree, one stack. Engines do not own copies of them.
- The plain projection (`unwrap`). The locked-down template and browser JSON both consume it.
- Format and localize functions. QuickJS `ec.l10n`, the locked-down template, and the browser `ec.l10n` call them. The browser implementation is the subset already specified and talks to the locale catalog, not to the server function.

## What is not reused

- A separate Rust value trait per template syntax. The renderers and the macro modules share the QuickJS runtime. The locked-down renderer gets the unwrap snapshot as plain JavaScript values.
- `MNode`'s FreeMarker wrapper cache. Codegen runs once. The walker passes strings.
- A reflection wrapper in the style of `BeansWrapper`. Each host type above is written out.
- Live entity values, `ec`, or `sri` inside the locked-down template or inside the cached qvt template.

## Server host methods

Measured 2026-10-11 from `framework/template/XmlActions.groovy.ftl` and the kept screen macros in `runtime/template/screen-macro/` (`any`, `html`, `qvt`, `text`, `csv`, `xml`, `xsl-fo`, `plain`). `vuet` is excluded. These are the calls the generated JavaScript makes. The browser `ec` keeps `l10n` format and localize, `resource.expand`, and the user allowlist from `plans/QvtServerStatic.md`. It does not gain the rest of this list. `sri` stays off the browser.

### `sri`, during a server screen render

`doBoundaryComments`, `getFieldValueString`, `makeUrlByType`, `getScreenUrlInstance`, `getFieldOptions`, `getActiveScreenDef`, `popContext`, `buildUrl`, `getFieldEntityValue`, `getFieldValue`, `renderSection`, `pushSingleFormMapContext`, `renderSubscreen`, `getFieldValuePlainString`, `getFormInstance`, `getThemeIconClass`, `startFormListRow`, `endFormListRow`, `getFieldTransitionValue`, `renderSectionInclude`, `pushContext`, `setInContext`, `getFormHiddenParameters`, `getCurrentScreenUrl`, `safeCloseList`, `appendToAfterScreenWriter`, `renderIncludeScreen`, `makeFormListSingleMap`, `getRenderMode`, `getSingleFormMap`, `renderText`, `isAnchorLink`, `getFormListHeaderValues`, `startFormListSubRow`, `endFormListSubRow`, `getFieldValueClass`, `getThemeValues`, `appendToScriptWriter`, `getSectionIncludedNode`, `getFormFieldValues`, `makeFormListMultiMap`, `getNamedValuePlain`, `getFormNode`.

Property: `activeInCurrentMenu`.

The macros then read the objects those methods return. A screen URL has `path`, `url`, `getUrlWithParams`, `cloneUrlInstance`, `addParameter`, `removeParameter`, and `getPassThroughParameterMap`. A screen definition has `location`, `getMenuSubscreensItems`, `getDefaultMenuName`, and `getScreenName`. `makeUrlByType` returns `getUrlWithParams` and `addParameter`. `buildUrl` returns `path` and `url`.

### `ec` during a server screen render

- `resource` (`getResource`): `expand`, `expandNoL10n`, `expression`, `condition`.
- `l10n` (`getL10n`): `localize`, `format`, `formatCurrency`, `formatCurrencyNoSymbol`.
- `context` (`getContext`): `get`, plus the fields `formListFindId` and `orderByField`.
- `web` (`getWeb`): `sessionToken`, `parameters`, `requestParameters`, `getWebappRootUrl`.
- `user` (`getUser`): `locale`, `getLocale`, `getPeriodDescription`.
- `screen`: `isRenderModeValid`.

### Statics the screen macros call

- `WebUtilities.encodeHtmlJsSafe`, `WebUtilities.fieldValuesEncodeHtmlJsSafe`.
- `CollectionUtilities.paginateParameters`, `CollectionUtilities.getMapArrayListValues`.
- `StringUtilities.encodeForXmlAttribute`.
- `ScheduledJobRunner.getCronDescription`, one call.

### `ec` from XML actions

`XmlActions.groovy.ftl` emits these. There is no `sri` in an action run.

- `service.sync` and `service.async`, then `name`, `disableAuthz`, `multi`, `ignoreTransaction`, `requireNewTransaction`, `transactionTimeout`, `useTransactionCache`, `parameters`, `parameter`, `call`. The current template also emits `distribute(true)` for `async="distribute"`. That attribute stays rejected, so the host does not implement `distribute`.
- `entity.find`, then `useCache`, `forUpdate`, `useClone`, `distinct`, `offset`, `limit`, `selectField`, `orderBy`, `condition`, `havingCondition`, `requireSearchFormParameters`, `searchFormMap`, `one`, `list`, `count`, `iterator`, `deleteAll`, `getLimit`, `getPageIndex`, `getPageSize`.
- The list from a find: `getPartialList`, `filterByDate`, `filterByLimit`, `size`, `pageIndex`, `pageSize`.
- The value: `setSequencedIdPrimary`, `setSequencedIdSecondary`, `setFields`, `create`, `createOrUpdate`, `update`, `delete`, `deleteRelated`, `findRelated`, `findRelatedOne`.
- `entity.makeValue`.
- `entity.conditionFactory`: `makeConditionDate`, `makeActionConditionDirect`, `makeCondition`.
- `resource.expand`, `resource.expression`, `resource.script`.
- `message.addError`, `message.addPublic`, `message.addMessage`, `message.hasError`, `message.getErrorsString`, `message.clearErrors`, and the `errors` field.
- `logger.warn`, `logger.log`.
- `user.nowTimestamp`.
- `web.sendJsonResponse`, `web.requestParameters`.
- `context` as the default field map for `condition`, `parameters`, and `searchFormMap`.

`entity-data` is still a TODO in the Java template and adds no method here.

## Out of scope

- QuickJS operator overloading for decimals. QuickJS-NG ties it to the built-in `BigDecimal` and `BigFloat` classes (`CONFIG_BIGNUM`). It does not extend to a host class, so the build flag does not give a host decimal `+`. The browser cannot share that flag either.
- Running the macro module per request so a screen function can read the context stack. Request data is read by the generated script.
- A second script or template engine. V8 is the later option in `plans/Architecture-Proposal.md`, and only as a replacement for this runtime if a profile says the JavaScript is the heavy part.
