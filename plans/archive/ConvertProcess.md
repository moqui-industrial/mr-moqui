# Converting a Groovy component: the Mantle trial

This is how `mantle-udm` and `mantle-usl` were converted until all their data loads into MR-Moqui. The goal was every data type with entity ECA rules on. Services and screens that the load never reaches were not ported. The work happened on 2026-10-14, on the `mr-moqui-temp` branch of each repository in `runtime/component`. Each repository has two commits: the converter's output, then the hand ports on top, so the two can be reviewed separately.

## Result

- `moqui load types=all` with entity ECAs on exits 0. It loads 8040 records and creates 571 tables. It also runs the data-file service calls `initPartyAccountingConfiguration` (fiscal periods) and `createVariantProducts` (12 variants), and the entity ECA rules that set GL account flags and asset totals.
- The search feed indexes `MantleParty`, `MantleProduct`, `MantleFacility`, and `MantleInventoryAsset`. The 8 `MantleWikiPage` rows stay pending, because the framework's wiki services are deliberately not ported (`get#WikiPageManualDocumentData` says so).
- Not done:
  - 532 service expressions still fail `moqui lint` in 62 service files, and 52 `map + [..]` places are flagged by the converter.
  - `moqui lint --groovy` counts 2083 sites with a Groovy idiom: decimal literals 702, collection methods 676, `new` of a Java type 301, `*.` 174, `in` 143, and so on.
  - Those services run only when called. The load does not call them.

## Process

1. **Scratch runtime.** Make a runtime that links the components and keeps its database out of the real `runtime/db`. Put it under `/tmp`: a Postgres Unix socket path must be shorter than 108 bytes, and the scratchpad path is not.

   ```sh
   R=/tmp/mrm-rt; mkdir -p $R/component; cp -r runtime/conf $R/conf
   ln -s $PWD/runtime/base-component $R/base-component
   for c in mantle-udm mantle-usl ironstone-boss; do ln -s $PWD/runtime/component/$c $R/component/$c; done
   ```

2. **Mechanical conversion.** Run the converter, first without `--write` to see the report, then with it:

   ```sh
   moqui convert --groovy runtime/component/mantle-usl              # report only
   moqui convert --groovy --write runtime/component/mantle-usl
   ```

   - **mantle-udm:** 41 Elvis rewrites.
   - **mantle-usl:** 1462 map literals, 85 context-plus copies, 649 Elvis rewrites, 318 empty-list rewrites, and 52 `map + [..]` places left for review.
   - The converter is idempotent: running it again on converted files rewrites nothing that was already rewritten.

3. **Load, fix the first failure, repeat:**

   ```sh
   rm -rf /tmp/mrm-rt/db
   MOQUI_RUNTIME=/tmp/mrm-rt MOQUI_FRAMEWORK=$PWD/framework MOQUI_ACTIONS_DEBUG=1 moqui_log_level=info \
       moqui load types=all
   ```

   - The last `Loading entity data from` line names the file.
   - An action error names the service (`in service ...`). It also names the generated statement (`at generated line N: ...`), and with `MOQUI_ACTIONS_DEBUG`, the XML element before it (`after // <set field=...>`).
   - Errors left in the message facade by a data file are logged after that file (`Error messages loading entity data from ...`).
   - `ERROR [main]` lines are the load. `[moqui-worker-N]` lines are the search feed running after each commit.
   - To read the database after a load, `psql` is not bundled. Run `postgres --single` from `db/install/<version>/bin` with `LD_LIBRARY_PATH` set to `db/embedded/xmlshim`.

4. **Split the commits.** Back up the final files, reset the repository, run the converter, and commit that. Then restore the backup and commit the hand ports.

## What the converter does

These are rewrites whose meaning is the same in JavaScript, made only in expression places:

- A Groovy map literal becomes an object.
- `context + [..]` becomes `Object.assign({}, context, {..})`.
- Elvis `a ?: b` becomes `a || b`.
- EDI grammar files get a `.js` copy.
- String-literal `equals`, simple casts, `.contains(` on known lists and strings, and empty `ArrayList`/`HashMap` constructors (the second iteration, below).
- **New in this trial:** in a `condition` attribute, a name that is the `list` of an `entity-find` or `entity-find-related` in the same file becomes `(name?.length > 0)`, and `!name` becomes `!name?.length`.
  - An empty list is false in Groovy, but an object (always true) in JavaScript.
  - `?.length` works for an entity list, an array, and null.
  - Mantle has about 320 such conditions. Missing this produced the hardest failures: a branch taken on an empty find, then `list[0].x` on undefined, or an early `<return/>` with no result.

## Hand ports, by kind

Each kind below names where it came up. Port to plain JavaScript; do not add Java look-alike shims to the runtime.

- **Java collections.**
  - `new HashSet([x])` becomes `[x]`.
  - `set.addAll(other)` becomes a `for ... of` with `push`, or `list = list.concat(Array.from(other))`.
  - `list.contains(x)` becomes `includes`.
  - `List.add(x)` becomes `push`.
  - `new TreeSet(map.keySet())` becomes `Object.keys(map).sort()`.
  - `addToListInMap(k, v, map)` becomes `(map[k] ??= []).push(v)`.
  - A list or map read from the context and changed in place keeps the change (tested in `script/actions.rs`).
  - Seen in: `find#RootGlAccountClassEnum`, `find#ProductFeatures`, `create#VariantProducts`, and `get#ProductManualDocumentData`.
- **Equality.** `'DEBIT'.equals(x)` becomes `x === 'DEBIT'`.
- **Map plus.** `map + [k: v]` becomes `Object.assign({}, map, {k: v})`. The converter flags these and leaves them. An entity value spreads into the copy, since it is a proxy over its fields. Seen in: `init#PartyAccountingConfiguration`.
- **Casts.** `(x as int)` becomes `parseInt(x, 10)` or `Math.trunc(x)`, and `as String` becomes `String(x)`.
- **Decimals.** A `BigDecimal` field or parameter is a host decimal in scripts, and operators on it throw.
  - `(a ?: 0.0) + (b ?: 0.0)` becomes `toDecimal(a ?? 0).add(b ?? 0)`.
  - `a < 0` becomes `toDecimal(a ?? 0).compare(0) < 0`.
  - `-a` becomes `toDecimal(a).multiply(-1)`.
  - A decimal used as a JavaScript number goes through `toNumber()`.
  - Seen in: `update#AssetFromDetail` and `create#TimePeriod`.
- **Dates.**
  - A date, timestamp, or `ec.user.nowTimestamp` reaches a script as epoch-millis text; a `Date` field is UTC midnight. Text such as `yyyy-MM-dd` assigned back is converted on write and in conditions.
  - The date code in `create#TimePeriod` and `check#NextFiscalTimePeriods` uses small helpers, `dateOf`, `textOf`, `plusDays`, `plusMonths` (which clamps the day as `java.time` does), and `plusUnits` for `TF_day`/`TF_wk`/`TF_mon`/`TF_yr`. They work in UTC, so a step never crosses a day boundary.
  - `Calendar`, `java.time.*`, and `new java.sql.Date(x.time)` go away.
  - Compare dates through `dateOf(..)`, not as text: epoch-millis strings of different lengths compare wrongly as text.
  - A JavaScript `Date` does not survive `<set field>`: it converts to a host value. Keep `Date` objects in script locals.
- **Script bodies.**
  - `import` lines and Java types (`def`, `ExecutionContext ec = context.ec`) go away.
  - `def f(..)` becomes a function or arrow.
  - `for (X x in list)` becomes `for (const x of list)`.
  - `catch (Exception e)` becomes `catch (e)`.
  - `newValue.setAll(context)` becomes `setFields(context, true, null, null)`.
  - `ec.service.sync().name("create", "a.B")` becomes `name("create#a.B")`.
  - `EntityCondition.IN` and `conditionDate` become `ec.entity.conditionFactory.makeActionConditionDirect(..)` and `makeConditionDate(..)`, which are what compiled XML actions use.
  - Seen in: `create#VariantProduct`.
- **Data-file Groovy.** A `LocalizedMessage` template in `MantleUdmSetupData.xml` (`TimePeriodBaseNameTemplate`) used `padLeft`, `getYear()`, and a string regex split. It now uses `padStart`, `slice`, and a regex literal.

The second iteration turned the safe ones of these into converter rules. See below.

## Second iteration: more converter rules

These rules were added and run on `mantle-usl`, in its own commit on the `mr-moqui` branch:

- **`string-equals` (45):** `'lit'.equals(x)` becomes `'lit' === x`, and `chain.equals('lit')` becomes `chain === 'lit'`. Both are wrapped in parentheses unless they are already the whole of a parenthesized group, such as an `if (...)`.
  - A Groovy `x.equals(..)` on null throws, where `===` is false.
  - `.equals` with no string literal on either side is left alone.
- **`cast` (44):**
  - `x as String` becomes `x?.toString()`, which is null for null as in Groovy.
  - `x as BigDecimal` becomes `(x == null ? null : toDecimal(x))`, and `x as int` (or `Integer`, `long`, `Long`) becomes `(x == null ? null : Math.trunc(toDecimal(x).toNumber()))`. These repeat the operand, so only a name chain is rewritten.
  - Groovy's `as` binds more loosely than arithmetic and ordering (`a + b as String` casts the sum). So a cast is rewritten only when its operand begins after `(`, `,`, `[`, `{`, `?`, `:`, `=`, `==`, `!=`, `&&`, `||`, `;`, or `return`.
  - The rest are left for review. The first run of this rule cast only the `b` of `a + b as String`; the spot check of the diff caught it.
- **`list-includes` (0 in Mantle):** `.contains(` becomes `.includes(` on a string literal or a found list of the file. Mantle's `contains` calls are on sets and on strings held in variables, which this rule cannot see.
- **`empty-collection` (12):** `new ArrayList()` becomes `[]`, and `new HashMap()` or `new LinkedHashMap()` becomes `{}`, with or without generics.

The load is unchanged: all data, ECAs on, exit 0, 12 variants. Service expressions that fail to parse went from 532 to 503. Most of what remains are whole Groovy script blocks, each one failure. `moqui lint --groovy` idiom sites went from 2083 to 2050. The `equals` fixes do not show in either count: `'X'.equals(y)` parses as JavaScript and fails only when it runs.

What is left needs a decision, not more safe rewrites:

- Java collections used as collections: `new HashSet(..)` 90, `new TreeSet(..)` 42, `new ArrayList(..)` with contents, `.add(` 366, `.put(` 204, `.addAll(` 57, `.contains(` 86, and `*.` spreads 174.
- JavaScript arrays, JavaScript `Set`, and Java's `Set` have different method shapes:
  - `add`/`contains` on Java's `Set`; `add`/`has` and a `size` property on a JS `Set`; `push`/`includes` on an array.
  - So rewriting call sites one by one needs the type at each call.
- `plans/archive/Architecture-Proposal.md` says JDK types such as `BigDecimal` and `TreeSet` are small host helpers with the same call shape. Following it would mean script globals `HashSet`, `TreeSet`, `ArrayList`, and `HashMap` with Java's methods. The converter would then only strip generics, and `*.name` could become `.map((it) => it.name)`.
- That is a choice about the script environment, so it is noted here rather than made. The ports so far use plain JavaScript.

## Framework changes the trial needed

Each of these is Java behavior the port lacked. All are in the framework commit for this trial.

- **Script environment:**
  - `EntityValue.cloneValue()`.
  - `ec.transaction.isTransactionInPlace()` and `isTransactionCacheActive()` (the queries only; scripts do not begin or commit).
  - `ec.logger` `info`, `error`, `debug`, and `trace`.
  - `isText()`, `isBinary()`, and `getContentType()` on resource references.
- **Error messages:**
  - A service body's error names the service.
  - An actions error shows the generated line, and with `MOQUI_ACTIONS_DEBUG` the element before it.
- **Entity ECA rules:**
  - Every field of the entity is a name in the rule's context, null when not set, so an unset field does not read a caller's variable of the same name. Java's rule context is the whole value.
  - `find one` with a full primary key drops the other equalities, `IS NULL` ones included, as Java's `oneInternal` does with nulls from `condition(context)`.
- **Conditions:**
  - A text value compared to a date, time, or number field is converted to the field's type before it is bound, as Java's condition does. Epoch-millis text is read as a moment.
  - A map condition such as `condition(context)` on a view entity uses the view's aliases.
- **Search feed and load:**
  - Each document type is delivered in its own transaction, so one type that fails (the wiki manual data service) rolls back and leaves pending only its own rows. Its message errors are cleared.
  - A load waits, up to 120 seconds, for the service workers (the feed) to finish before the database closes.

The earlier load work (component entity directories, rules, and data; services by path; `fromDate` and secondary ids on nested records) is in `plans/archive/Implementation-Phases.md`, after the gate's note on loading `mantle-udm`.
