# OSIRIS / Vabohu migration status

**Incomplete. This branch does not yet make OSIRIS/Vabohu runnable on MR-Moqui.**
The gap analysis is a baseline audit of the original working trees, not a claim that this branch completes the port.
Keep the PR in draft until all acceptance gates below pass.

Worktree: `/home/user/moqui/mr-moqui-osiris`; branch: `feat/osiris-vabohu`.
The original OSIRIS checkout and its uncommitted changes have not been edited or committed here.

## Implemented framework adapters

- `ec.entity.find(...).oneMaster(name)`: use the existing entity REST master expansion instead of introducing a second graph reader. Returns a plain graph; the root find retains its locking/cache options. Related reads still pass through entity authorization.
- `ec.entity.getValueListFromPlainMap(graph, entityName)`: flatten mutable relationship maps/lists into typed values; inherit ancestor primary keys and mapped relationship keys, ignore read-only relationships/metadata, and bound traversal depth. Does not persist or bypass authorization; callers must order/persist/delete values in a transaction.
- `EntityValue.resolveEntityName()` aliases `getEntityName()` for existing persistence/deletion rank comparisons.
- `ec.service.restApi.resolveService(path, method)`: resolve a resource path and method to service name/path metadata, honoring named resources before ID nodes and `allow-extra-path`. Reject entity-backed handlers. This is introspection only, not authorization to execute the service.
- `ec.screen.getTransitionInfo(rootLocation, screenLocation, transitionName, method)`: mounted transition path, backing service and declared parameter metadata. Reject unmounted screens or missing methods; skip unreadable sibling screens during path lookup. The caller must select the correct webapp root and use existing URL/authorization APIs when executing a transition.
- `ec.service.getServiceDefinition(name)`: expose in/out parameter maps, input parameter names and `getInParameter(name).attribute(...)`. Input auto-parameters are expanded using the entity model. This is not a complete nested JSON Schema generator.
- `ec.message.pushErrors()` / `popErrors()`: isolate validation errors, then merge saved caller errors back as Java does. Expose `errorsString` alongside `getErrorsString()`.
- Correct `ec.message.clearErrors()` to retain informational messages instead of clearing the entire message facade.

## Regression coverage

`crates/moqui-core/src/script/runtime/osiris_tests.rs` exercises script bindings, graph read/flatten round-trips, mapped relationship keys, read-only relationships, missing masters, mounted/unmounted/method-specific transition metadata, expanded service input metadata and scoped errors. `service/rest.rs` covers named/ID/extra-path metadata resolution and rejects non-service handlers. `context/message.rs` covers nested error scopes and informational-message retention.

Reproduce:

```sh
cargo test -p moqui-core --lib --no-default-features --features http
```

Verified result: **804 passed, 0 failed, 1 ignored**. `git diff --check` also passed.

Library Clippy passed with `cargo clippy -p moqui-core --lib --no-default-features --features http -- -D warnings`.
Including tests exposes an existing `clippy::drop_non_drop` at `crates/moqui-core/src/entity/sequence.rs:524` (`drop(next)`); that file is unchanged by this branch. The same library+tests Clippy command passed with only that lint allowed (`-- -D warnings -A clippy::drop_non_drop`). No repository lint configuration was relaxed.

`http` is required for existing loopback-HTTP tests. A no-default-features run without it is not a valid full-core HTTP test gate.

## Still required before the requested runtime goal is met

- [ ] Port the OSIRIS Groovy helper classes and XML expressions/scripts to JavaScript/Rust, preserving the current local OSIRIS source rather than an older committed version.
- [ ] Adapt component call sites to the new metadata APIs. A framework API adapter alone does not rewrite Java implementation-class accesses.
- [ ] Implement LLM profiles, clients, conversations, tool schemas/validation, retry/terminal-tool behavior, browse policy and latest-edit injection. The existing Phase 13 work remains deferred; no fake successful model results are introduced here.
- [ ] Replace callback service parameters with serializable named service operations; use existing service futures for parallel identity resolution.
- [ ] Implement the shared per-process diff queue, supersession, owner checks, retention/replay and worker failure cleanup.
- [ ] Preserve process-row locks, isolated commits and network-I/O-outside-transaction boundaries; verify rollback and concurrent submissions against Postgres, not just the memory test driver.
- [ ] Implement the native authenticated SSE completion endpoint with capacity limits, heartbeats, disconnects and Vabohu's `completed`/`failed` event contract.
- [ ] Port/install the required MarbleERP/SimpleScreens/Mantle dependency surface or explicitly replace its referenced operations. Merely deleting `depends-on` is not an equivalent application.
- [ ] Run translated OSIRIS security/validation/identity/state tests and real MR backend + Vabohu browser integration tests, including login/MFA, CORS/session/CSRF, ADMIN-only relationships, stale-source rejection and LLM retry/supersession.

No end-to-end deployment, real-model call, live browser/backend session or production configuration change has been performed by this implementation.
