# Dev AIDE

The assisted IDE of mr-moqui (`plans/design-dev-aide.md`), at `/qapps/dev` on a dev instance:

- **Home** has the site's unpublished changes with Publish, each repository and directory with its branch and changes,
  the reload log, the restart a change needs, and "No site component yet. Create one".
- **Workspace** is the IDE: the roots (the repositories and directories of the loaded components), the file tree with
  git status, files in the CodeMirror editor with schema completion and checks, Save that reloads what the file defines,
  Changes (stage, unstage, discard, commit with the task trailer), Commits, Fetch, Pull, Push and Publish.
- **Settings** has the site components, "Allow editing upstream components", the roots' labels, read-only flags and
  publish branches, more directories, and network git on a shared dev server.

It gives its users code execution as the server's OS user, so it loads only when `instance_purpose` is dev:
`runtime/dev-component` is a dev-only component directory, and its `component.xml` says `purpose="dev"`. Bind the server to
127.0.0.1 (`webapp_http_bind=127.0.0.1`) unless it is a team dev server (`dev-aide shared="true"`).

Who may use it (`data/DevAideSecurityData.xml`):

| Permission | Who has it | What it allows |
|---|---|---|
| `DEV_AIDE_VIEW` | | Browse, search and read files. |
| `DEV_AIDE_EXTEND` | `DEV_AIDE_EXTENDERS` | Write site components and publish them. |
| `DEV_AIDE` | `ADMIN`, `DEV_AIDE_USERS` | Everything: upstream edits with the toggle on, and every git command. |

Its Rust crate is linked into the Dev and Debug packages (the `devtools` feature). In another package the screens load
and the services say they are not built in.

- **Tests:** `./moq test dev-aide` (the screens), `cargo test -p dev-aide` (confinement, the git parsers),
  `moqui test-client runtime/dev-component/dev-aide --offline` (the client), `cargo test -p moqui-host --test it devaide` (saves, the lock and tiers, git and Publish on scratch
  repositories), and `cargo test -p moqui-host --test gate3_browser the_workspace` (Chromium).
- **Screenshots:** `MOQUI_DEV_SHOTS=<dir>` writes them in light, dark and 390 px.
