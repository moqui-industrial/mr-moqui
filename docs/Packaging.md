# Build Packages

A moqui binary is built as one of four packages, plus the browser build. They differ in the Cargo features they turn
on (what the binary can do), the Cargo profile they build with (how it is compiled), and where the binary lands.
`./moq build` picks one with a flag; every task that runs moqui (`run`, `load`, `test`, `save-db`, ...) takes the same
flag and uses that binary. `moqui version` prints a binary's package and features.

| Package | `./moq` flag | Profile | Binary | What it has |
|---|---|---|---|---|
| Default | (none) | `lite` | `target/lite/moqui` | The web server on embedded Postgres, PDF (through Chromium), outbound mail (SMTP) |
| Debug | `--debug` | `dev` | `target/debug/moqui` | Every feature, unoptimized framework code with debug info |
| Maximal | `--release` | `release` | `target/release/moqui` | Every feature, Postgres embedded |
| Enterprise | `--enterprise` | `enterprise` | `target/enterprise/moqui` | Every feature, with the databases external (Postgres and MySQL drivers, no embedded Postgres) |
| Browser | `scripts/wasm-build.sh [--release]` | `dev` / `release` | `target/browser/wasm-pkg/<profile>` | The framework on SQLite in a Web Worker; none of the optional features |

Plain `cargo build -p moqui`, `cargo test` and `cargo lint` use the Maximal feature set, so they cover everything.

## Features

The framework features are in `crates/moqui-core`; the host features, and the packages, are in `crates/moqui-host`;
`crates/moqui` (and the crate `./moq build` generates in `target/moqui-app` for add-on component crates) forwards the
package features `lite`, `enterprise` and `maximal`.

| Feature | Crate | What it gates | Default | Enterprise | Maximal |
|---|---|---|---|---|---|
| `axum` | host | The HTTP server, its listeners and routes | yes | yes | yes |
| `postgres` | host | The Postgres driver | yes | yes | yes |
| `embedded-postgres` | host | The embedded Postgres clusters (entity, search, sim), `save-db`, `load-save`, backups, `moqui test` | yes | | yes |
| `mysql` | host | The MySQL driver | | yes | yes |
| `pdf` | host | The DevTools client of the external Chromium that prints `html-print` to PDF | yes | yes | yes |
| `ssh` | host | The SFTP client, the built-in SSH server (SFTP, admin shell, git over SSH), the SSH cluster channel | | yes | yes |
| `sso` | host | OIDC and OAuth 2 logins (`org.moqui.sso.AuthServices`) | | yes | yes |
| `print` | host | Network printing over IPP (`moqui-cups`) | | yes | yes |
| `smtp` | core | Outbound mail: `send#EmailMessage`, `send#EmailTemplate`, password reset and notification mail | yes | yes | yes |
| `imap` | core | Inbound mail: `poll#EmailServer`, the mail client's sync, IDLE, rules and import (includes `smtp`) | | yes | yes |
| `git` | core | `git://` locations, git drives, `org.moqui.impl.GitServices`, the git server over HTTP and SSH, LFS | | yes | yes |
| `image` | core | `org.moqui.impl.ImageServices` (`convert#Image`) and elFinder thumbnails | | yes | yes |
| `xlsx` | core | The `xlsx` render mode and `org.moqui.impl.SpreadsheetServices` | | yes | yes |
| `http`, `cluster-tls` | core | Remote REST, `http` locations, the OpenSearch client; the cluster certificate | yes | yes | yes |

`mail` is `smtp` and `imap` together. The browser build has none of the core features.

### When a feature is left out

The definitions still load, so components, screens and service files are the same in every package:

- A service whose Rust implementation belongs to a missing feature fails when called, with
  `service <name>: <feature> is not built into this host` in the message facade (`rust_services::missing_feature`).
- The host integrations report through the framework's existing messages: SFTP and SSO calls say no client is
  installed, network printing says it is not installed, and `cluster_transport=ssh` cannot start without the SSH
  server. A cluster peer that advertises `ssh://` gets an error naming the missing feature.
- An `xlsx` render fails with `xlsx is not built into this host`; an elFinder thumbnail is left out; a `git://`
  location has no provider, and `/git/...` is not routed.
- gitECA rules are still read and checked; nothing runs them, since there are no git events.
- Without `embedded-postgres`, `entity_ds_db_conf` must name an external `postgres` or `mysql` database, the sim
  overlay is not available, and `save-db`, `load-save` and `moqui test` (which makes scratch embedded databases) report
  that this build has no embedded Postgres.

A spec file that needs a feature says so, and `moqui test` reports it as skipped in a binary without it:

```js
spec({
    requires: ["git"],
    ...
})
```

## Profiles

The profiles are in `.cargo/config.toml`, so the generated `target/moqui-app` crate builds the same way. Each package
has its own, so the artifacts of one never replace another's and switching packages does not rebuild.

- `dev` (Debug): the workspace crates at opt-level 0 with debug info, the dependencies at opt-level 3.
- `lite` (Default): `release` with `incremental = true` and 256 codegen units for the workspace and path crates
  (Cargo applies `incremental` only to those), so a rebuild after an edit to the framework is quick; the dependencies
  keep the release settings and are built once.
- `release` (Maximal) and `enterprise`: `panic = "unwind"` (services unwind across the service boundary), otherwise
  Cargo's release defaults.

## Embedded Postgres archive

The `embedded-postgres` feature embeds the theseus Postgres archive (the version `.cargo/config.toml` forces through
`POSTGRESQL_VERSION`) and unpacks it once into `runtime/db/install/<version>`, shared by the entity, search and sim
clusters. `crates/moqui-host/build.rs` stages it from `~/.theseus/postgresql/postgresql-<version>-<target>.tar.gz`, or
the file `MOQUI_PG_ARCHIVE` names, and downloads nothing. It refuses an archive that still holds `bin/psql` or a
readline library (GPL, plans/archive/License-Review.md).

`scripts/pg-payload.sh strip` downloads the archive when it is missing (checking its sha256) and repacks it without
psql; `./moq build` runs it the first time. A plain `cargo build` of a package with embedded Postgres asks for it.
`scripts/pg-payload.sh check` checks an archive, and every one staged under `target/`, for psql and readline.

## Checks

`scripts/check-packages.sh` runs Clippy with warnings denied on each feature set a build uses: the workspace
(Maximal, as `cargo lint`), the Default and Enterprise packages, the host with no features (`cargo lint-min`), and the
framework with none (what the browser links). A gate that only one package sees still has to compile there.

## Build cost

Measured on Oct 19, 2026, on one 12-thread Linux machine, Rust 1.99: a clean build of each package in its own target
directory, the binary, and two run-time checks. Before the packages there was one feature set, every build carrying
everything, and postgresql_embedded's `bundled` build script compiled a second HTTP and TLS stack.

| Build | Clean build (wall) | Compile CPU | Crates | Binary | Stripped | `.text` |
|---|---|---|---|---|---|---|
| Before (release, everything) | 171 s | 1394 s | 623 | 84 MB | 67.5 MB | 38.1 MiB |
| Maximal (`release`) | 151 s | 1091 s | 545 | 82.9 MB | 66.8 MB | 37.7 MiB |
| Enterprise | 141 s | 971 s | 522 | 68.4 MB | 53.1 MB | 36.4 MiB |
| Default (`lite`) | 101 s | 608 s | 354 | 57.1 MB | 47.4 MB | 24.5 MiB |

The embedded Postgres archive is 11.9 MB of each binary that has it.

| Build | Rebuild after a one-line edit in moqui-core | Load seed, seed-initial, install (8141 records) |
|---|---|---|
| Maximal (`release`) | 60 s | 8.4 to 10.0 s |
| Default (`lite`, opt-level 3) | 11.6 s | 8.6 s |
| Default at opt-level 2 / 1 | 11.6 s / 10.7 s | 8.3 s / 9.1 s |
| Debug (`dev`) | 8.8 s | 18.1 s |

The Default profile keeps opt-level 3: a lower one saves no rebuild time. Thin LTO on the release profile made the
binary larger (84.1 MB, 67.6 MB stripped) for a seed load of 8.0 to 8.3 s, within the noise of a load that waits on the
database, so it is off. Stripping symbols saves about 16 MB a binary; it is not done, so a backtrace in a log keeps its
function names.

What a clean build still spends: aws-lc-sys (the TLS crypto every package links) is the first link of the longest
chain, then rustls, reqwest and moqui-core itself; QuickJS is compiled twice because rquickjs's `macro` proc-macro links
its own copy; postgresql_embedded's build script still compiles reqwest and tokio (without TLS) though it no longer
uses them. `cargo build --timings` writes the chart to `target/cargo-timings/`.
