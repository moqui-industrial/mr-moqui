# Testing components with `moqui test`

`moqui test` runs the JavaScript spec files in a component's `test/` directory. They run in the same script engine as
the component's services, on a database loaded with every data type, with Spock's conveniences: spec and feature
setup and cleanup, ordered features with shared state, `given`/`when`/`then`/`expect` blocks whose conditions fail with
the values of their parts, `where` tables, a suite order, data checks, pinned sequence ids, a pinned clock, screen and
REST renders, and features that run on several threads.

The specs of `runtime/component/mr-mantle-usl/test` are the largest set, ported from mantle-usl's Spock specs; its
`README.md` maps Groovy to spec files line by line. The design record is `plans/archive/TestHarness.md`.

## Running

```sh
./moq test mantle-usl                                  # the release binary and the dev conf
moqui test mantle-usl                                  # every spec, fresh database
moqui test mantle-usl --db snapshot                    # load once, restore the saved dump on later runs
moqui test mantle-usl --spec OrderToCashBasicFlow      # one spec and the specs its dependsOn names
moqui test mantle-usl --spec OrderToCashBasicFlow --feature "ship Sales Order"
moqui test mantle-usl --junit build/test-results       # JUnit XML for CI
moqui test mantle-usl --coverage                       # the services no spec called, per service file
moqui test mantle-usl --keep --log-level info          # keep the scratch runtime, more on the console
```

With no component, every component that has `test/*.spec.js` runs, in load order. `./moq test --debug` uses the debug
binary.

| Option | |
|---|---|
| `--spec Name[,Name]` | The specs to run, by file name without `.spec.js`, with the specs their `dependsOn` names, in suite order. `dependsOn` is resolved within the spec's own component |
| `--feature text` | Only the features whose names contain the text |
| `--db fresh` | The default: load every data type into a scratch runtime, removed at the end unless `--keep` |
| `--db snapshot` | As fresh, but save the load under `runtime/save` and restore it while the entity and data files and the binary are unchanged |
| `--db existing` | The runtime's embedded database as it is. The specs change its data |
| `--junit dir` | `TEST-<component>.<Spec>.xml` per spec |
| `--coverage`, `--coverage-report file` | The services of the components' `service/*.xml` that no spec called, with the total; the report adds call counts |
| `--log-level level` | Console level, `warn` by default. Everything goes to `runtime/log/moqui.log`, whose path the run prints |

The console has a line per feature, and the failures in full at the end. Any failed feature exits 1.

## Files and order

A spec is `<component>/test/<Name>.spec.js`. Its name is the file name. `test/suite.js`, `suite(["SpecA", "SpecB"])`,
gives the order; the other spec files run after it, by name. Each spec runs in its own execution context, on the one
database of the run, so a spec sees what the specs before it wrote.

## A spec file

```js
const effectiveTime = Date.now()
let cartOrderId                                     // shared between features (Spock's @Shared)

spec({
    dependsOn: ["OrderProcureToPayBasicFlow"],      // run first when this spec is picked with --spec
    tempSequences: { start: 55500, banks: { "mantle.order.OrderHeader": 10 } },   // reset after cleanupSpec
    setupSpec() { ec.user.setEffectiveTime(effectiveTime) },                      // reset after the spec
    setup() { ec.artifactExecution.disableAuthz() },
    cleanup() { ec.artifactExecution.enableAuthz() },
})

feature("create Sales Order", {
    when() {
        login("joe@public.com")
        const priceMap = call("mantle.product.PriceServices.get#ProductPrice", { productId: "DEMO_1_1" })
        cartOrderId = call("mantle.order.OrderServices.add#OrderProductQuantity", { productId: "DEMO_1_1", quantity: 1 }).orderId
        const dataCheckErrors = checkData(`<entity-facade-xml>
            <mantle.order.OrderHeader orderId="${cartOrderId}" entryDate="${effectiveTime}" grandTotal="200.68"/>
        </entity-facade-xml>`)
        return { priceMap, dataCheckErrors }         // what then() reads
    },
    then({ priceMap, dataCheckErrors }) {
        priceMap.price == 16.99
        dataCheckErrors.length == 0
    },
})

feature("call the REST API (#requestMethod, #screenPath)", {
    where: [
        { requestMethod: "post", screenPath: "s1/mantle/orders/55800/place", containsTextList: ['"statusChanged" : true'] },
    ],
    expect({ requestMethod, screenPath, containsTextList }) {
        const str = screenTest("rest").render(screenPath, null, requestMethod)
        containsTextList.every((text) => str.assertContains(text))
    },
})
```

### `spec({...})`

Once per file:

- `dependsOn`: the specs that run first when this one is picked with `--spec`.
- `stepwise`: after a failed feature, skip the rest (Spock's `@Stepwise`). Off by default.
- `tempSequences: { start, banks }`: sequenced ids from `start`, with the bank size for each entity, set before
  `setupSpec` and reset after `cleanupSpec`.
- `allowMessageErrors`, and `timeout` in seconds per block.
- `setupSpec`, `cleanupSpec`, `setup` and `cleanup`, as in Spock. Cleanups always run. These get no `ctx`.

### `feature(name, {...})`

A statement at the top level of the file; features run in the order they are written.

- Blocks: `given`, then `when` with `then`, or `expect` alone, and `cleanup`, which always runs. Each block gets `ctx`:
  the fields of the `where` row and what the earlier blocks returned.
- `where`: an array of row objects, or a function that returns one. Each row is its own case, and `#field` in the name is
  the row's value.
- `ignore: "reason"`, `timeout`, `allowMessageErrors`.
- A feature with `then` and no `when`, or a block name that is not one of these, is an error when the file loads.

## Conditions

Each expression statement directly in `then` or `expect` is a condition, as in a Spock `then:` block. A false one fails
the feature and shows the value of each part:

```
Condition not satisfied:

priceMap.price == 16.99
|        |     |
|        16.98 false
{"price":"16.98","priceUomId":"USD"}
```

- `==`, `!=`, `<`, `<=`, `>` and `>=` compare as Groovy does: decimals, numbers and number text by value
  (`16.990 == 16.99`), and lists and maps by their items. `===` stays strict.
- `!x` and a bare value use Groovy truth: an empty list, map or string, and zero, are false.
- An assignment is not a condition, and a call that returns `undefined` passes.
- Inside an `if`, a loop or a callback, use `assert(condition, message)`.
- A feature also fails when it leaves errors in `ec.message`, unless `allowMessageErrors` is set. Messages are cleared
  between features.

The source is rewritten for this before it runs (`crates/moqui-core/src/testkit/rewrite.rs`), with no new lines, so error
line numbers are the file's. A `then` or `expect` the rewrite cannot see, such as a feature object built in a helper
function, is refused when the file loads rather than run with conditions that cannot fail. Use `where` rows instead.

## Helpers

`ec` is the full script `ec`, as in a service. The spec engine adds:

| Helper | |
|---|---|
| `call(name, params)` | A service call. It throws, naming the service and its `ec.message` errors, where `ec.service.sync()` would return null |
| `checkData(xml)` | `ec.entity.makeDataLoader().xmlText(xml).check()`: the messages of what did not match, each logged |
| `login(username, password = "moqui")`, `logout()` | `login` throws when the login fails |
| `screenTest(baseScreenPath)` | `ec.screen.makeTest().baseScreenPath(...)`. `render(path, params, method)` gives `output`, `errorMessages`, `status`, `renderTime` and `assertContains(text)`; `renderMode("html-print")` and the other builder methods set the mode and root screen |
| `parallel(n, fn, args, opts)` | Run `fn(threadNum, args)` on `n` threads, each with its own `ec`, and return the results. `opts`: `transaction` (default true), `timeout` (default 120 s) |
| `uploadedFile(fileName, text, contentType = "text/plain")` | The `FileItem` that services taking an upload expect |
| `assert(condition, message)` | A condition outside the top level of a block |
| `sum(list, field)`, `dec(value)` | A decimal sum of a field, and a value as a decimal |
| `log` | `ec.logger` |

`ec.user.setEffectiveTime(millis)` pins the clock for `ec.user.nowTimestamp`, date filters with no date, and the
`fromDate` defaults; the runner clears it after the spec. `ec.entity.tempSetSequencedIdPrimary` and
`tempResetSequencedIdPrimary` are what `tempSequences` calls.

## Mail

A spec checks a send that goes, with no mail server, through the mail capture: while it is on, `send#EmailMessage`,
and so `send#EmailTemplate` and the mail client's `send#Draft`, does all of a real send (the `EmailMessage` row, its
`ES_SENT` status and `sentDate`, the mail client's Sent folder) except connect, and keeps the message in memory. The
`EmailServer` must still have an SMTP host, as for a real send (an empty one sends nothing); it is never connected. A
spec turns it on for itself, and the conf property `mail_capture_sends=true` turns it on for a whole run
(`mail_capture_sends=true ./moq test ...`) or a server that must not send mail; it is off otherwise, so a spec that
checks a send that cannot connect sees one. The tool reads and clears what was captured:

```js
const capture = ec.factory.getTool("MailCapture")
spec({ setupSpec() { capture.enable() }, cleanupSpec() { capture.reset() } })
// in a feature
capture.clear()
const sent = call("org.moqui.impl.EmailClientServices.send#Draft", { emailMessageId: draftId })
capture.last().toList            // ["customer@example.com"]
```

| Method | |
|---|---|
| `getMessages()` | The captured messages, oldest first, each `{subject, fromAddress, fromName, toList, ccList, bccList, replyToList, body, bodyText, messageId, inReplyTo, references, attachmentNames, capturedDate}` (the 500 most recent) |
| `last()`, `count()`, `clear()` | The newest one (or null), how many, and forget them |
| `enable()`, `disable()`, `reset()` | Capture on or off for this server whatever the property says, and back to the property; reset it when the spec ends, as the switch outlives the spec |
| `isEnabled()` | Whether a send now is captured |

The capture is a tool of the framework (`moqui_core::service::mail_capture`, in `MoquiDefaultConf.xml`).

## Notifications

A `moqui test` run has no notification socket, and what is sent only to someone listening (the shell counts of
`org.moqui.impl.ShellServices.publish#ShellCounts`) is not sent at all. A spec that checks those notifications turns on
the notification capture, which listens on the notification topic as the server's socket does and keeps each message in
memory:

```js
const capture = ec.factory.getTool("NotificationCapture")
capture.enable()
try {
    // ... what publishes, then (the shell counts go after the commit, on a thread of the server) wait for them
    capture.getMessages("shellCounts").filter((row) => row.userIds.includes(userId)).length
} finally { capture.disable(); capture.clear() }
```

| Method | |
|---|---|
| `getMessages(topic?)` | The captured messages, oldest first, all or of one topic, each `{topic, subTopic, userIds, title, link, message, capturedDate}` (the 500 most recent) |
| `last(topic?)`, `count(topic?)`, `clear()` | The newest one (or null), how many, and forget them |
| `enable()`, `disable()`, `isEnabled()` | Listen or stop; disable it when the spec ends, as the listener outlives the spec |

The capture is a tool of the framework (`moqui_core::notify::capture`, in `MoquiDefaultConf.xml`).

## Pitfalls

- Outside a `then` or `expect` block, a line that starts with `[`, `(`, `/` or a template literal continues the line
  before it, since JavaScript inserts no semicolon there: `x = 1` then a line `[a, b].forEach(f)` is one expression.
  Start such a line with `;`, or put the list in a variable. In a `then` or `expect` block each line is its own
  condition, as in Spock: the rewrite ends the line before such a line when it ended an expression, so
  `/^Today · \d+$/.test(label)` on its own line is a regular expression, not a division.
- Decimals from services are objects: `+`, `<` and `Number()` on them throw outside a condition. Use their methods
  (`add`, `subtract`, `multiply`, `negate`, `compare`), or `dec`.
- `parallel` runs its function from its source text on other threads, so it cannot use the spec's variables. Pass what it
  needs in `args`, which is copied as JSON, and return values JSON can hold.
- Time stamps reach scripts and data as epoch millis text: write `entryDate="${effectiveTime}"`, and
  `new Date(effectiveTime + n)` converts to a time stamp.
- A data-dependent service on a product another spec uses can see that spec's inventory and reservations when it runs
  after it; give a new spec its own stock or its own product.
- Generic JavaScript linters report `no-unused-expressions` in `then` and `expect`; those expressions are the conditions.
