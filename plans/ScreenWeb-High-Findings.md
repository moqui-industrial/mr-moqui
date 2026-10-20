# Screen and Web High Findings — Fix Plan

Status: worked on 2026-10-20. The three High findings of the review — P5-01, P7-01 and P8-01 — and
P8-02 are fixed; the plan below is the record of what was decided and done. The decisions section
is what shipped. Phase B's earlier small fix (leave the server rule text as `moqui.…`) is not: the
server emits `$mq.…`.

Scope: the three High findings of the screen and web parity review — **P5-01** (the server-static
qvt's number rule), **P7-01** (the REST schema documents) and **P8-01** (an open redirect) — plus
**P8-02** (the redirect location is not sanitized), which the same change closes. They are in the
review's summary table, [the findings that were High, and are fixed](ScreenWeb-Parity-Review.md#the-findings-that-were-high-and-are-fixed).

The full review is [ScreenWeb-Parity-Review](ScreenWeb-Parity-Review.md). The 96 other findings are
not in this plan.

Each fix below is small and independent. Do them in the order given: P7-01 first (it is the
simplest and it touches an authorization rule), then P5-01, then P8-01/P8-02 (the redirect change
touches the most call sites).

---

## Phase A — P7-01: the REST schema documents need the `REST_SCHEMA` permission

### The finding

`crates/moqui-core/src/screen/schema.rs:60-70` checks only that a user is logged in. Java's
`RestSchemaUtil.requireSchemaAccess` (`RestSchemaUtil.groovy:570-582`) checks the login **and** the
`REST_SCHEMA` permission, and answers 403 without it.

The permission already exists in the port's data — `runtime/base-component/framework/data/MoquiSetupData.xml:183-184`
has the `UserPermission` row and the `ADMIN` group grant, and
`runtime/base-component/framework/conf/MoquiDefaultConf.xml:79` seals it — so no code reads it.

### The fix

`crates/moqui-core/src/screen/schema.rs`:

- Rename `logged_in` to `require_schema_access` (Java's name) and add the permission check:

  ```rust
  const SCHEMA_PERMISSION: &str = "REST_SCHEMA";
  const SCHEMA_FORBIDDEN: &str = "Not authorized for REST schema";

  /// Java's `RestSchemaUtil.requireSchemaAccess`: a user, and the `REST_SCHEMA` permission.
  fn require_schema_access(login_error: Option<&str>) -> Result<bool, Error> {
      if let Some(error) = login_error {
          web::send_json_error(401, error);
          return Ok(false);
      }
      if user::current_username()?.is_none() {
          web::send_json_error(401, SCHEMA_AUTH);
          return Ok(false);
      }
      if !user::has_permission(SCHEMA_PERMISSION)? {
          web::send_json_error(403, SCHEMA_FORBIDDEN);
          return Ok(false);
      }
      Ok(true)
  }
  ```

- Call it from `entity_json` and `service_swagger` in place of `logged_in`.

`user::has_permission` (`crates/moqui-core/src/user/preference.rs:195`) is the right helper: it reads
`moqui.security.UserGroupMember` and `moqui.security.UserGroupPermission`, honours the dates, and
runs with the authorization check off (Java's `UserPermissionCheck` view does the same).

No change is needed in the dispatch (`screen/dispatch.rs:65-67`): `entity.json`, `master.json` and
`service.swagger` all go through `schema.rs`.

### The tests

`crates/moqui-core/src/screen/tests.rs`:

- The check is skipped when the context's authorization is off, the way the port's other permission
  checks are (`user::filter.rs`), so the tests use the pattern the other tests use: enter a context
  and disable the authorization in advance (`let _entered = context.enter(); let _authz =
  context.disable_authz();`, as in `screen/tests/widgets.rs:302-304`). The permission rows are then
  not needed.
- The existing test `entity_json_has_the_primary_key_and_the_service_document_lists_enum_types`
  (`tests.rs:446`) uses the `admin` user and expects 200, so it gets that pattern.
- A new test, `the_rest_schema_documents_need_the_rest_schema_permission`, leaves the authorization
  on: a user without the permission gets 403 and the message `Not authorized for REST schema`, and
  nobody gets 401. (The test world's conf has no `artifact-execution` entries, so the screen
  transition itself is not refused before the schema's own check.)

### What to watch

- A deployment that relied on the schema documents being open to every logged-in user will now get
  403 for the users that are not in `ADMIN`. That is Java's rule, but it is a behavior change for
  the port; say so in the release notes.
- Only the schema documents get the check. The entity REST endpoints (`/rest/e1/...`) are governed by
  the entity's own `AT_ENTITY` authorization in both runtimes — do not add the check there.
- The `Unclear` question in the review ("should the other REST documents need it too?") is answered
  here: no. Java's `requireSchemaAccess` guards the schema documents only.

---

## Phase B — P5-01: the server-static qvt number rule calls a name that is not in the scope

### The finding

`runtime/base-component/framework/template/screen/ScreenQvtStatic.js:370` writes

```js
rules.push("value => !value || moqui.isStringNumber(value) || ec.l10n.localize('Please enter a valid number')");
```

The server-static template is compiled on the server with Vue's `prefixIdentifiers`
(`crates/moqui-core/src/screen/vue_check.rs:136`), so every free identifier in it becomes `_ctx.<name>`.
The static scope is the client's scope proxy, whose host is
`{ ec, $mq, StringUtilities }` (`runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.js:704`).
So `_ctx.ec` and `_ctx.$mq` resolve and **`_ctx.moqui` does not**: the rule throws a `TypeError` on
every non-empty value and a server-static form with a number field never submits.

The merged (server-merged) qvt mode is not affected: its template is compiled in the browser without
`prefixIdentifiers`, so `moqui` is the browser global there.

### The fix

Two files, and no change to the server's rule text:

1. `runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.js` — add the two number checks
   to `qvt.helpers` (the `$mq` set, around line 469). They are thin wrappers over the client's own
   `moqui` library, so they keep its locale separators:

   ```js
   // The number checks of a field's browser rules. The merged mode has the moqui global; a server-static
   // template has only the scope, so the rule calls them through $mq.
   isStringNumber: (value) => moqui.isStringNumber(value),
   isStringInteger: (value) => moqui.isStringInteger(value),
   ```

2. `runtime/base-component/framework/template/screen/ScreenQvtStatic.js:370` — call them through the
   scope:

   ```js
   if (check.number) rules.push("value => !value || $mq.isStringNumber(value) || ec.l10n.localize('Please enter a valid number')");
   ```

   The digits rule on the next line needs no change: its regex is inline and `ec.l10n.localize`
   resolves to `_ctx.ec.l10n.localize`, which the restricted browser host has.

### Why not change the server's rule text

`crates/moqui-core/src/service/define.rs:376-377` holds `VALIDATE_NUMBER` and `VALIDATE_NUMBER_INT`
as `!value || moqui.isStringNumber(value)`. They are used only by the qvt path
(`input_js_rules` → `field_validation_js_rules` → `sri.getFieldValidationJsRules` →
`ScreenQvt.js`), and there the merged mode's browser global is what makes them work.

The alternative is to make the server emit `$mq.isStringNumber(value)` for both modes and give the
merged mode a browser global `$mq` (`root.$mq = moqui` in `MoquiLib.js`). That is one rule text for
both modes and it would make a later fix of P5-03 (the static mode drops the `validate-service`
rules) safe, because those rules come from the same server text.

**Decide this before the change.** The recommendation is the alternative — one rule text, one
name, no chance of the same bug in the next rule — but it adds a browser global and touches the
merged mode's rules, so it is a bigger blast radius. If the plan takes the small fix above, leave a
note in `ScreenQvtStatic.js` that a rule text written by the server must not be pasted into a static
template, and put the one-text change on the follow-up list.

### The tests

- `crates/moqui-core/src/screen/tests/statics.rs`: a new fixture screen with a form field that has a
  number check. The "number" class comes from `validate-service` or `validate-entity`
  (`crates/moqui-core/src/screen/sri/fields.rs:377-404`), so the fixture needs a service with a
  number parameter — install one with `crate::service::install(&factory, &[<the fixture path>])`,
  the way `screen/tests.rs:93-95` installs `moqui.rest.xml`. Add the fixture service XML under
  `crates/moqui-core/src/screen/testdata/Static/`.
- The test renders the screen static and asserts the compiled script has **no** `_ctx.moqui` and has
  the number rule through `_ctx.$mq`. That is the regression: the bug is a name in the compiled
  output, so the compiled output is what to assert on.
- The existing golden files (`testdata/Static/golden/Items.script.js` and the rest) do not change:
  no fixture has a number field today, so no rule is emitted. Check them when the fixture is added.
- `runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.static.test.js`: assert that
  `qvt.helpers.isStringNumber` and `qvt.helpers.isStringInteger` agree with `moqui.isStringNumber`
  and `moqui.isStringInteger`.

### What to watch

- The static template is cached and shared, so a fix in the module needs the cache to be dropped.
  The static template cache is per screen definition; a restart clears it, and the definition cache
  has a check interval (P7-05 in the review) so a changed file is picked up in development.
- The number check honours the user's locale separators through `moqui.thousandSeparator` and
  `moqui.decimalSeparator`, so the wrapper must call the client's function and not re-implement the
  test.

---

## Phase C — P8-01 and P8-02: the redirect target

### The finding

`web::send_redirect` (`crates/moqui-core/src/web/mod.rs:844`) writes the location it is given into
the `Location` header. Java's `ScreenRenderImpl.sendHttpRedirect` (`ScreenRenderImpl.groovy:228-251`)
does three things the port does not:

1. `WebUtilities.sanitizeRedirectLocation` (`WebUtilities.java:456-466`): drop CR, LF and NUL, trim,
   and an empty location becomes `/`. (P8-02.)
2. When the webapp names no `http-host` (the default: `webapp_http_host` is empty, and
   `runtime/base-component/framework/conf/MoquiDefaultConf.xml:431` gives the webapp
   `http-host="${webapp_http_host}"`), an `http://` or `https://` target is reduced to its path and
   query. The browser therefore stays on this site, and Java has no open redirect there. (P8-01.)
3. A location that starts with `/` and not `//` is sent as a `Location` header; anything else goes
   through `response.sendRedirect`.

The port sends an absolute target as it is, so a screen whose response URL is expanded from a
request parameter sends the client to that URL.

### The fix

`crates/moqui-core/src/screen/render.rs` — where Java has it (`ScreenRenderImpl`), a helper for the
location and the call sites that use it:

```rust
/// Java's `ScreenRenderImpl.sendHttpRedirect`: a location is sanitized, and an absolute `http(s)`
/// target is reduced to its path and query when the webapp names no `http-host`, so a redirect
/// cannot leave the site. Java's `WebUtilities.sanitizeRedirectLocation` drops CR, LF and NUL.
fn http_redirect_location(factory: &Arc<ExecutionContextFactory>, location: &str) -> String {
    let mut clean = String::with_capacity(location.len());
    for ch in location.chars() {
        if ch != '\r' && ch != '\n' && ch != '\0' {
            clean.push(ch);
        }
    }
    let clean = clean.trim();
    if clean.is_empty() {
        return "/".to_string();
    }
    let host = factory.webapp_setting(WEBAPP, "http-host");
    if host.is_empty() && (clean.starts_with("http://") || clean.starts_with("https://")) {
        // Java's URI.create: the authority is dropped, the path (or "/" when there is none) and the query stay.
        let after = clean.split_once("://").map_or("", |(_, rest)| rest);
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let rest = &after[end..];
        let (path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, Some(query.split('#').next().unwrap_or(""))),
            None => (rest.split('#').next().unwrap_or(""), None),
        };
        let path = if path.is_empty() { "/".to_string() } else { path.to_string() };
        return match query {
            Some(query) if !query.is_empty() => format!("{path}?{query}"),
            _ => path,
        };
    }
    clean.to_string()
}
```

(`moqui-core` has no URL crate — the parse above is the small hand-rolled one Java's `java.net.URI`
does for this case. A location that is not a valid URI must become `/`, as in Java's catch block;
add that check for a space or a control character.)

Then use it at the three call sites that correspond to Java's `sendHttpRedirect`:

| Port site | Java site |
|---|---|
| `screen/render.rs:360` (`send_redirect_and_stop_render`, the 302 branch) | `ScreenRenderImpl.groovy:220` |
| `screen/render.rs:832` (the login redirect) | `ScreenRenderImpl.groovy:1091` |
| `screen/render.rs:1443` and `:1466` (the response URL redirect, plain and screen-path) | `ScreenRenderImpl.groovy:598` and `:635` |

`send_redirect_and_stop_render` is `pub` and has no factory argument. Get the factory from the
current execution context, the way `screen/schema.rs:88-91` does
(`ExecutionContext::with_current(|ec| ec.factory())`).

### The call sites that must NOT change

- `screen/render.rs:791` — the https redirect. Java sends it with `response.sendRedirect`
  (`ScreenRenderImpl.groovy:1101`), not with `sendHttpRedirect`, so the absolute URL stays.
- `service/sso.rs:82`, `:213`, `:296` — the SSO redirects. Java's `AuthenticationFlow` uses
  `ec.web.response.sendRedirect` directly (`AuthenticationFlow.groovy:70`, `:119`, `:154`), so the
  identity provider's absolute URL stays. Breaking this would break every OIDC login.

### The tests

`crates/moqui-core/src/screen/tests/render.rs`:

- A new transition in the fixture `crates/moqui-core/src/screen/testdata/Root.xml`:
  `<transition name="goExternal"><default-response url="https://evil.example/steal"/></transition>`
  (the fixture already has `goUrl`, `goAbsolute`, `goPlain` and `goParams`).
- With the default conf (no `http-host`), the `Location` is `/steal`.
- With a conf that names a host —
  `ExecutionContextFactory::test_context_with_conf("<moqui-conf><webapp-list><webapp name=\"webroot\" http-host=\"app.example\"/></webapp-list></moqui-conf>")`
  — the `Location` is the absolute URL, as in Java.
- A location with a CR or LF is redirected, not a 500 (P8-02). The port's axum layer rejects a
  header value with a control character (`moqui-host/src/http.rs:897-912`), so this is the test that
  pins the sanitize.

### What to watch

- Java's rule only reduces an `http(s)` target. A protocol-relative target (`//evil.example/x`) is
  **not** reduced by Java, and Java sends it — an open redirect Java has. The port's `screen-last`
  path already refuses a `//` target (`screen/render.rs:1380-1384`), so the port is stricter there.
  **Decide** whether the new helper also refuses `//` and any other scheme. Refusing is safer and
  matches the port's own style; it is a difference from Java and it goes in
  [Parity-Diffs](Parity-Diffs.md) as a deliberate one.
- The rule reads the webapp's `http-host` from the conf, so a deployment that names a host keeps
  the absolute redirect. That is Java's rule, not a new hole.

---

## The work, in order

| # | Change | Files | Test |
|---|---|---|---|
| A1 | The `REST_SCHEMA` check | `crates/moqui-core/src/screen/schema.rs` | `crates/moqui-core/src/screen/tests.rs` |
| A2 | The authorization-off pattern in the tests, and the new 403 case | `crates/moqui-core/src/screen/tests.rs` | the same |
| B1 | The `$mq` global and the scope's number helpers | `runtime/base-component/webroot/screen/webroot/js/MoquiLib.js`, `…/WebrootVue.qvt.js` | the client tests |
| B2 | The rule text calls `$mq` | `crates/moqui-core/src/service/define.rs`, `…/ScreenQvtStatic.js` | `crates/moqui-core/src/screen/tests/statics.rs`, `…/forms.rs` |
| B3 | The fixture screen and service | `crates/moqui-core/src/screen/testdata/Static/…` | the same |
| C1 | The redirect location helper | `crates/moqui-core/src/screen/render.rs` | `crates/moqui-core/src/screen/tests/render.rs` |
| C2 | The call sites, and the JSON answer's `redirectUrl` | the same | the same |
| C3 | The fixture transition | `crates/moqui-core/src/screen/testdata/Root.xml` | the same |

The three phases do not touch each other and can be three commits.

The work is done: `cargo test -p moqui-core --lib` (1371 tests), `cargo test -p moqui-host --test gate3`
(10 tests, the real HTTP stack) and `cargo lint` are green. The 4 failures in `moqui-db` are the ones
that were there before this work (the database profiles and the full-text DDL).

## Running the tests

```sh
cargo test -p moqui-core --lib screen::           # the screen unit tests
cargo test -p moqui-core --lib screen::tests::statics
cargo test -p moqui-core --lib screen::tests::render
cargo test -p moqui-core --lib service::           # the rule text of B (if the alternative is taken)
node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.static.test.js
```

The `gate3` and `gate3_browser` suites are the slow confirmation; they need an embedded Postgres and
Chromium, and they are not required for these three fixes.

## The documentation to update with the fixes

- `plans/ScreenWeb-Parity-Review.md`: mark P5-01, P7-01, P8-01 and P8-02 as fixed, with the commit.
- `plans/Parity-Diffs.md`: add the rows for the differences the fixes keep on purpose:
  - the strictness decision of Phase C (a `//` target or another scheme, if the port refuses it);
  - the change in the `REST_SCHEMA` behavior, if the port's answer differs from Java's message text.
- `plans/Remaining-Work.md`: the "Parity gap review" section still says the rest of the framework
  has had no phase review; point it at the screen and web review.

## Out of scope

- The 97 other findings of the review, including the Med ones in the same areas (P5-02, P5-03,
  P5-04, P5-05, P8-05, P8-06).
- The systemic guard for this class of bug: the static compatibility report
  (`crates/moqui-core/src/screen/statics/report.rs`) could flag a compiled template that names a
  scope member the static scope does not have, which is what would have caught P5-01. That is
  P5-06's area, and it is a separate piece of work.
- P7-02, the link permission check, and P7-03, the static file response headers: Med findings in the
  same files.

## The decisions

1. **Phase B: one rule text.** Decided 2026-10-20: the alternative. The server's rule text calls the
   helpers as `$mq.…`, the merged qvt mode sets `app.config.globalProperties.$mq`, and the
   server-static scope has `$mq` in its host. One text, both modes, and a later fix of P5-03 is safe.
2. **Phase C: how strict.** Decided 2026-10-20: refuse. A target that is not a path of this site and
   not an `http(s)` URL goes to the root, in the `Location` of a redirect and in the `redirectUrl` of
   a JSON answer. An `http(s)` URL reduced to a path stays only when that path is a single slash and
   holds no backslash; `//` and `\` become `/`. The difference from Java is in Parity-Diffs.
3. **Phase A: the permission.** `has_permission("REST_SCHEMA")` runs even when artifact authorization
   is off, as Java's `requireSchemaAccess` always asks. A user in `ADMIN` with the grant is allowed.
