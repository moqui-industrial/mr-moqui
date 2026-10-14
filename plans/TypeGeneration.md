# Definition TypeScript Generation

What TypeScript declarations the framework generates from the XML definitions, and where they land.

This follows `plans/Architecture-Proposal.md` (Scripting Language: JavaScript via rquickjs, and the optional TypeScript authoring layer) and `plans/ContextObjects.md` (the host method lists and the plain projection). It is a tool. It does not change the request path, the runtime, or the XML schemas.

## Decision

A `types` command on the binary, beside `run` and `load`, walks the loaded definitions and writes `.d.ts` files plus one small shape module. It runs after the definitions load, against the same parsed model the runtime uses. It is not on the request path and not part of startup. A dev watch mode is optional and later.

The output directory is a CLI flag (`--out`), defaulting to `runtime/types/`. An application component may commit the output so its screens get completion without a build step.

## What is generated

| Declaration | Source |
|---|---|
| Entity value and entity list types | Entity XML and the dictionary types, matching the field get and set the QuickJS host exposes |
| Service input and output maps | The service definitions, including the implicit entity-auto verbs, with required and optional matching the validation set |
| The unwrapped projection | `ContextJavaUtil.unwrapMap`: plain maps, decimals as strings, timestamps as ISO-8601 strings, entity values as field maps. This is the JSON, locked-down template, and browser shape |
| Browser `ec` | The restricted host in `plans/QvtServerStatic.md`, as an interface: `l10n`, the `user` snapshot, `resource.expand` |
| Server `ec` and `sri` | The Server host methods section of `plans/ContextObjects.md`, split by when the method exists (facade surface, screen render) |
| Static helpers | `ObjectUtilities`, `CollectionUtilities`, `StringUtilities`, and `ScheduledJobRunner` methods the generated script calls |

The type mapping follows `plans/ContextObjects.md`. On the server, a decimal is an object type whose methods are the host value's (`add`, `subtract`, `multiply`, `divide`, `eq`, `compare`, `isZero`, `toNumber`, `toString`). That object type makes `amount + tax` and `amount * tax` errors in authored TypeScript. `tsc` still allows `==` and `if (amount)`. The phase 6 definition-load parse (`moqui lint`, `oxc_parser` 0.139.0) flags `==`, relational operators, and truthiness on that server type. That is the expression lint. `oxc` as a TypeScript transpile path, below, is a different use. A timestamp is a branded ISO-8601 string (`Timestamp`). In the unwrapped projection, in the locked-down template, and in the browser, a decimal stays a string, as the row above says. Ids and strings are `string`, integers and floating numbers are `number`, booleans are `boolean`.

Screen context typing is partial and honest about it: action scripts are dynamic, so a screen context interface lists the keys the generator can prove (form maps, `<set>` names, section and `section-iterate` lists) and an index signature for the rest. A `server-static` `qvt` screen gets the package shape from `plans/QvtServerStatic.md` as its prop type, minus the keys that plan strips.

## What it does not do

- No runtime enforcement. The QuickJS host is unchanged; these are declarations for editors and `tsc`.
- No JavaScript codegen and no transpilation. If a longer script is authored as TypeScript, SWC or oxc transpiles it and these declarations are what make that useful.
- No checking of names chosen at runtime, and no declarations for a component whose definitions are not loaded (mantle types arrive when mantle loads).
- No change to the XML schemas. The generator reads the definitions as they are.

## Where it lands

Phase 11 in `plans/Implementation-Phases.md`, gate 4 acceptance: the declarations for a sample of framework and tools services and screens pass `tsc --noEmit`. The generator is a core tool on the definition caches; the binary command is the entry point.
