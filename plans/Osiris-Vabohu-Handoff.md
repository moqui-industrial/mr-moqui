# OSIRIS → mr-vabohu handoff

## Authoritative goal and repositories

Port the full original OSIRIS functionality into **mr-vabohu** and publish the
component to https://github.com/moquilabs/mr-vabohu, not `moquilabs/vabohu`.
Required framework changes belong on **feat/osiris-vabohu** in
https://github.com/moqui/mr-moqui, updating existing draft PR #10.
The component is not part of the framework Git repository. Do not change the
framework's `origin` to the component upstream. Commit/push are authorized by the
user's latest instruction; earlier local-only / PR-only instructions are superseded.

Local framework worktree: `/home/user/moquilabs/mr-moqui`.
Local component repository: `runtime/component/mr-vabohu` (`main`, its own `origin`).
Framework remote: `origin`, `git@github.com:moqui/mr-moqui`.
No frontend, alternative component name, or work in unrelated worktrees.
Read AGENTS.md and `.agents/skills/building/SKILL.md` before builds/worktree work.

## Source baseline

Original OSIRIS: `/home/user/labs/moqui/runtime/component/osiris`, baseline commit
`4db80cb24a1ab045c4c2aee85f9ff692d204b754` plus local modified/untracked source.
Use the frozen **44-file working-tree snapshot** in the component's
`reference/osiris/`; verify `reference/osiris.sha256.json`. Never replace it with
HEAD-only source or copy JVM build output/hooks/credentials into the component.
Original checkouts and the other worktrees have not been edited. The initial
checkout's untracked gap-analysis plan remains untouched.

## Current tested milestone — not the full port

- Framework adapters: master read/flatten, scoped errors, service input/output
  metadata, REST service resolution and screen transition metadata.
- Fixed real-runtime blockers: component metadata calls now use the actual APIs;
  REST/transition metadata includes path parameters; ambiguous screen mounts fail
  closed; Login declares optional username/password/code inputs without changing
  its MFA continuation behavior; script master reads retain child primary keys
  (REST export's key suppression is deliberately unchanged).
- Component: entities, framework-only discovery seed, actor-scoped/state-gated
  discovery, process projection, deterministic UTF-16 helpers and identity checks.
- Deterministic graph validation, including legacy entity/field records, exact
  input bindings, field metadata and producer/consumer dependency inference.
- ADMIN-only graph upsert and explicit deterministic action application, normal
  artifact authorization, process-row locks, expected-source checks, sanitized
  mutable graph persistence, soft deletion and rollback.
- Fixed subtype persistence: `one-nofk` subtype relations need `mutable="true"`.
  Canonicalize persisted variable references because master relationships can
  expose the same field row both flat and nested; do not relax duplicate-ID
  checks on caller-supplied proposals.

`apply#BusinessProcessActions` is a deterministic administrative service, not the
original queued `update#BusinessProcessDiff` or its frontend completion contract.
Upsert does not delete omitted graph records or change conversation identity.
`apply#BusinessProcessDiff` now validates raw semantic mappings at the locked
write boundary, revalidates advisory same-process deleted/just-replaced identity
selections, restores phrase/reference IDs and remaps surviving inputs. Obsolete
restored reference trees are cleaned child-first after repointing retained refs.
No text-similarity inference or model selection is implemented. Do not claim
ordinary-user edit policy, model orchestration or full ERP parity.

Current gates:

- `node --test runtime/component/mr-vabohu/test/*.test.mjs`: **89 passed**;
  includes all 44 snapshot hash checks.
- Real XML/QuickJS + disposable PostgreSQL: **11 features passed**. Tests include
  discovery, canonical graph reload, stale updates, concurrent locked edits,
  soft deletion, foreign identities, ADMIN checks with artifact authz disabled,
  and rollback after a subtype insert fails after its parent rows were written.
- `cargo test -p moqui-core --lib --no-default-features --features http`:
  **824 passed, 0 failed, 1 ignored**.
- Native-component public API integration suite: **11 passed**.
- Library Clippy with the same features and `-D warnings`: passed.
- Merged current `origin/master` (`d761f57`, including native-component PR #13
  and SQL-name PR #12) and reran these gates on the combined framework branch.

Use isolated incremental `CARGO_TARGET_DIR=/home/user/.cache/mr-vabohu-target`,
compiler `rustc 1.98.1`, unless intentionally selecting a new consistent toolchain.
Logs for this iteration: `/tmp/mr-vabohu-{node-tests,runtime-tests,core-tests,clippy}.log`.

## Remaining implementation, not optional reduced scope

1. Port LLM profiles/conversations/tools: validated terminal acceptance, retries,
   browse policy, latest-edit injection before each request and advisory identity
   selection. Raw HTTP is not the full tool runtime; do not fabricate success.
2. Connect model-based advisory identity selection to the now-ported deterministic
   restoration/reference-remapping write boundary; port normal-user edit policy.
   Enforce ADMIN-only relationship and artifact-authorization changes again at
   the write boundary. Keep full-source stale checks and atomic writes.
3. Implement shared per-process queue/workers, supersession, owner-scoped replay
   and retention, and failure cleanup. No JS-global shared state. Keep network I/O
   outside caller transactions and isolated persistence in named services.
4. Implement authenticated completion SSE preserving original GET-only URL,
   owner checks/status codes, capacity limits, heartbeat/disconnect behavior and
   `completed`/`failed` events. Do not silently substitute polling.
5. For native shared-state implementations, use a component-owned crate. The
   committed native API is now merged into this branch; read local
   `plans/Native-Components.md` and `plans/Native-Components-Usage.md` first.
   Do not edit/copy another session's unfinished core files or invent a dynamic
   loader. Native code remains selected through authoritative service XML and
   normal authentication/transaction/authorization processing.
6. Port/install actual MarbleERP/SimpleScreens/Mantle operations and original
   seed/demo processes. The small framework seed lacks full entity annotations
   and ERP functionality; it is not user-approved reduced scope.
7. Translate remaining upstream tests and run security/MFA/session/CSRF,
   concurrency/supersession, rollback/identity, LLM failures, SSE isolation/replay
   and actual backend/browser qualification. Keep framework PR #10 draft.

See `plans/Osiris-Vabohu-Implementation.md` for framework adapter history and the
component README for install/test commands and current limitations.
