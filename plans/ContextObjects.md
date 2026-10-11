# Context objects

Which values MiniJinja, QuickJS, Liquid, and the browser can see, and which Rust types those environments share.

This follows Templating in `plans/Architecture-Proposal.md` and the browser host in `plans/QvtServerStatic.md`. The method list on server `ec` facades and on `sri` stays a later plan.

## Decision

There are four places a value is visible. Three of them are Rust engines. The fourth is the browser.

| Environment | When it runs | What it is given |
|---|---|---|
| MiniJinja | Once, when an XML action or screen definition is compiled. The generated text is cached. | The element being compiled: attribute strings, child element text, and the already-rendered output of its children. No `ExecutionContext`, no context stack, no entity value. |
| QuickJS | Every server evaluation: XML actions, `<script>`, attribute expressions, and a server screen render (html, text, csv, xml, xsl-fo, and qvt when it is not server-static). | The live context stack, `ec`, and during a screen render `sri`. Entity values and lists stay host objects. |
| Liquid | Every send of an email, notification, or other user- or admin-authored template. | A plain snapshot of the map the caller passed. Filters for format and localize. No `ec`, no `sri`, no live entity. |
| Browser | A server-static qvt screen, after the template is cached. | The JSON package from `plans/QvtServerStatic.md`, plus the restricted browser `ec` that plan already lists. |

String expansion (`ResourceFacade.expand`, and the `${name}` expansion in `moqui-lib`) is not a fifth engine. It looks up names in the context stack and evaluates embedded expressions with QuickJS on the server, or with the restricted browser host. Property expansion in lib stays string replacement against the environment and does not see the context stack.

MiniJinja does not run on the request path. A screen macro that today calls `sri` or `ec` while FreeMarker is rendering emits a JavaScript call instead. That call runs later, in QuickJS or in the browser. Internal framework text that is not an XML macro and not user-authored may use MiniJinja with a plain map. That map is the same snapshot Liquid gets.

## What Java does, and what changes

`MNode` implements the FreeMarker node interfaces and caches the wrappers on the node. That pays off because screen FreeMarker runs on every render and looks up `@attr` and child elements through those interfaces. Everything else reaches FreeMarker through `BeansWrapper` reflection. `ContextStack` is only a `Map`.

The two FreeMarker jobs already have different data models:

- `XmlActions.groovy.ftl` is compiled once. Its data model is one entry, `xmlActionsRoot`, the `MNode`. `<#visit>` walks the node. The names `ec`, `context`, and `result` appear in the generated Groovy. They are not in the FreeMarker data model. The generated script header says so.
- Screen macros run on every render. `ScreenWidgetRenderFtl` passes the whole `ContextStack` as the data model, after putting `sri` and the widgets node on it. `ec` is already on the stack. Macros both walk `.node` and call `sri` and `ec` while they write output.

MiniJinja 3.0.0 has no `<#visit>` and no node type. Its `Object` trait also takes ownership: `Value::from_object` stores an `Arc`, and the object must be `Send + Sync`. That is unchanged from 2.x. The context stack is confined to one `ExecutionContext` thread, so it is the wrong type to hand to MiniJinja even before the orphan rule. The `serde` feature stays off. Macros see attribute strings, child text, and already-rendered children.

The Rust walker owns the visit. For each element it calls the macro named for that element and passes attributes, text, and rendered children. Child output is produced first, then the parent macro wraps it, including the case where the parent emits a loop and the child script sits inside the loop. Attribute strings are copied into that macro call. The copy happens once per definition, and the cached result is the generated script. That replaces the `MNode` wrapper cache. `Node` does not implement an engine trait.

## One stack, three projections

`ContextStack` stays in `moqui-lib`. It is a stack of maps. It implements no MiniJinja, Liquid, or QuickJS trait. `moqui-lib` does not depend on those crates. A type in lib cannot implement their traits anyway: the orphan rule requires the crate to own the type or the trait.

QuickJS is the live view. Bare names in an expression (`orderId`, `statusId`) resolve on the stack, the way the Groovy binding and the FreeMarker data model do. `context` is that stack. `ec` is the host object for the execution context and is also the `ec` entry on the stack. Assignments write the top map. A service `result` map is present when the current actions provide one, as today. The QuickJS runtime is the one already owned by that `ExecutionContext`. Values created for a call die with it. They are not stored on a process-wide runtime.

### Codegen rule: bare names

This is the contract for macros and for migrated scripts. It is decided before macros are written.

Groovy returns null for an unset bare name. `ContextBinding.hasVariable` always returns true (`ContextBinding.java:41-45`), so the get falls through to `contextStack.getByString` and yields null instead of `MissingPropertyException`. `if (someUnsetParam)` is false. In JavaScript the same line throws `ReferenceError` at runtime. Identifier lookup calls `HasBinding` before `Get`, so a get-only trap never runs for an unset name.

The server runtime installs a `Proxy` on `globalThis` for that `ExecutionContext`. It closes over that thread's `ContextStack`. It is not installed on the browser `window`. `with` is not used. Strict mode rejects it, and the proxy is the mechanism that works.

- `has` returns true for a stack key, for a real global property, and for every other name. An unknown name still has a binding, so the read does not throw.
- `get` returns the stack value when any map on the stack contains the key, including an explicit null. Otherwise it returns the real global (`Array`, `Math`, `ObjectUtilities`, and the rest). Otherwise it returns null.
- `set` writes the top map, as `ContextBinding.setVariable` does. It does not create a real global.

Macros emit bare names. They do not rewrite reads to `context.foo`. `context.foo` still works because `context` is on the stack. Server-static qvt expressions are Vue template expressions against the JSON package. A missing key renders empty there. A new client script that mentions an unset name can still throw. That is acceptable. The proxy is not how the browser works.

Expression hazards that change what a macro emits, including Elvis versus `??`, empty collections, `==`, and Groovy ranges, are the table in `plans/Architecture-Proposal.md` (What changes in expressions). Macros follow that table. `??` is only for a null check. An empty string or a zero uses logical or. An empty list or map tests length. `==` stays `==`. A Groovy inclusive range becomes `slice` with an exclusive end.

Liquid and the browser see a snapshot, not the stack. The snapshot is the same projection as `ContextJavaUtil.unwrapMap`:

- Strings, booleans, numbers, and timestamps pass through. Timestamps become ISO-8601 text in JSON and in Liquid.
- An entity value becomes the plain field map. An entity list becomes a list of those maps.
- `EntityFind`, the execution context, `sri`, and the context stack object are dropped.
- Anything else is dropped and logged, as `unwrap` does today.

Liquid's `Value` / `ObjectView` and the browser's JSON are two sinks of that one projection. A trusted internal MiniJinja data template is a third sink. The projection is the shared code. The engine value types are not.

A live entity value inside Liquid would let a template walk a relationship and read through to the database. The snapshot is the sandbox, matching the decision that Liquid is the boundary. Callers pass the map the template is allowed to see. Filters may format and localize. They call the same functions as `ec.l10n` on the server. They do not take an `ExecutionContext`.

## Host objects in QuickJS

These are the types a script can hold. Plain maps and lists that are already plain data are ordinary JavaScript objects, copied in at the boundary. The types below stay Rust values behind getters so a screen or a service does not clone the working set into the engine.

| Name | Visible as | Rust value |
|---|---|---|
| context keys | Bare names, and `context.name`. On the server an unset name is null, via the proxy above | The `ContextStack` |
| `ec` | Host object | The `ExecutionContext` and its facades |
| `sri` | Host object, only while a server screen render is on the stack | The screen renderer. Absent in actions that are not a render, and absent in the browser |
| entity value | Host object, field get and set | The entity value. `unwrap` is what Liquid, JSON, and the browser receive instead |
| entity list | Host object, index, length, iterable, `map`, `filter` | The list. `map` returns a JavaScript array so `list.map(x => x.field)` works without copying first |
| decimal, timestamp | Host values | `rust_decimal` and `time` from `moqui-lib`. Not JavaScript numbers. `+` is not overloaded. Comparison and arithmetic are methods or `ObjectUtilities` helpers, on the server only |
| `ObjectUtilities`, `CollectionUtilities`, `StringUtilities` | The same helpers the generated script header calls today | Functions in lib, bound once per runtime |

`ec.entity`, `ec.service`, and the rest of the facade surface are the server host plan. They exist only on this QuickJS object. The browser `ec` is the table in `plans/QvtServerStatic.md` and does not grow to match the server object. `sri` is not added to the browser.

Generated screen JavaScript for a server render calls `sri` and writes to the render writer. Generated server-static qvt does not. It transcribes expressions into the Vue template, and the browser evaluates them against the package and the restricted `ec`.

## What is reused

- `Node` and `ContextStack` in lib. One tree, one stack. Engines do not own copies of them.
- The plain projection (`unwrap`). Liquid, browser JSON, and internal MiniJinja data templates all consume it.
- Format and localize functions. QuickJS `ec.l10n`, Liquid filters, and the browser `ec.l10n` call them. The browser implementation is the subset already specified and talks to the locale catalog, not to the server function.

## What is not reused

- A single Rust type that implements `minijinja::value::Object`, `liquid_core::ValueView`, and an rquickjs class. The engines disagree on ownership (`Arc` and `Send` versus a thread-bound context versus an owned snapshot), and lib cannot name their traits.
- `MNode`'s FreeMarker wrapper cache. Codegen runs once. The walker passes strings.
- A reflection wrapper in the style of `BeansWrapper`. Each host type above is written out.
- Live entity values, `ec`, or `sri` inside Liquid or inside the cached qvt template.

## Out of scope

- The method list on server `ec` facades and on `sri`.
- QuickJS operator overloading for decimals. The browser cannot share it, and the build flag is not a dependency.
- Running MiniJinja per request so a screen macro can read the context stack. Request data is read by the generated script.
