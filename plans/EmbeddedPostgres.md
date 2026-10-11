# Embedded Postgres

How the embedded database serves many `ExecutionContext` threads, and where a sim overlay lives.

This follows `plans/Architecture-Proposal.md` (Database, Platform notes, Transactions) and `plans/Crate-Layout.md` (`DbDriver` in `moqui-host`, the transaction manager and pool in `moqui-core`). HOLD and FLUSH rules are a port of the Java framework at the time of the port. They are not specified here. The database lifecycle is.

`postgresql_embedded` 0.21 (theseus-rs) is the crate. It starts a real Postgres postmaster as a child process. The server is not linked into the framework binary. `pglite-rs` is not used.

## Engine

`moqui-host`'s `db` module owns startup and shutdown. Default features of `postgresql_embedded` are off. The features that are on are `bundled`, `blocking`, `theseus`, and `tls-rustls-aws-lc-rs`. `native-tls` and `zonky` stay off. `POSTGRESQL_VERSION` is pinned at build to 18.6.0, so a bundled binary does not call GitHub at runtime. The crate's MSRV is Rust 1.94, and that is the floor for the host crate.

The install lives under the runtime directory, not `~/.theseus`. One install serves every cluster. Two data directories: the entity cluster, and a temp directory for the sim cluster.

Measured theseus Postgres 18.6.0 archive sizes, 2026-10-11. The compressed figure is what a bundled build adds. The extracted figure is the on-disk install.

| Target | Compressed | Extracted |
|---|---|---|
| x86_64 linux gnu | 11.6 MB | 34.0 MB (1,441 files) |
| aarch64 linux gnu | 11.4 MB | not unpacked; same order |
| linux musl | about 25 MB | not unpacked |
| macOS | 12.5–12.8 MB | not unpacked |
| Windows x86_64 | 51.6 MB | not unpacked; only the Windows build pays this |

Runtime RSS was not measured. `shared_buffers` dominates it, so the settings below start small, and the first driver test records RSS.

Startup settings:

- The Unix socket directory is the data directory, kept short. `listen_addresses` is empty.
- The client is built from the username, the database, the port the postmaster bound, and that socket directory, with `NoTls`. It does not parse a URI.
- `shared_buffers` is 16 MB on the entity postmaster and 8 MB on sim. `max_connections` is the pool max plus the postmaster's superuser slots. The application reserve is inside the pool max. `logging_collector` is off. `fsync` stays on for the entity cluster and for sim. Sim sets `synchronous_commit` off. That is the only durability relaxation. Search schema init runs `CREATE EXTENSION IF NOT EXISTS pg_trgm` before any `gin_trgm_ops` index (`plans/SearchFacade.md`). Full-text and plpgsql are already in the server.

Start and stop go through the crate's blocking API, bridged once during factory startup and shutdown. That bridge's runtime has exited before the pool connects. Per-query traffic is `postgres::Client`. Each client owns a `current_thread` runtime and drives it with `block_on` on connect, on every query, and on drop. Those calls run on factory `std` threads. The host does not pass in a Tokio handle.

## Checkout

Checkout goes through the transaction manager in `moqui-core`. The manager calls `DbDriver` and does not import `postgres` or `mysql`. The manager is specified with the transaction facade in `plans/Crate-Layout.md` and in the Transactions section of `plans/Architecture-Proposal.md`.

`begin` does not check out a connection. The first statement enlists one and shares it until commit, rollback, or suspend. A call outside a transaction checks a connection out and returns it. `suspend` parks that client on the transaction. The client leaves the thread and stays off the free list. `resume` checks that same client out again. `force-new` is that suspend followed by begin, on the same thread. The calling thread holds one connection on that pool. The parked client still counts against the pool max. Ordinary checkouts cannot take the reserve. The rules are the Transactions section of `plans/Architecture-Proposal.md`. A savepoint cannot commit on its own, so `force-new` does not use one.

Pool sizes come from the datasource conf. The code fallback when `pool-maxsize` is omitted is min 5, max 50. The postgres `<database>` sets max 90. `pool-reserve` defaults to 8 and sits inside that max. Idle time and the acquisition timeout come from `pool-time-wait`. A normal checkout fails when only reserve connections are idle. The acquisition timeout is the fail-fast. A checkout runs the configured test query. The host has one pool implementation, the core transaction manager. Another database is another instance of that pool. The sim overlay is one. It is not an entity-group `DbDriver` map entry. The one-connection rule is per pool, so a sim thread can hold the entity transaction's client and one overlay client.

When the entity datasource is Postgres, embedded or external, search uses the `search` schema on that database. A second postmaster starts only when the search store is a different database: search config names another Postgres, or the entity datasource is MySQL (or another non-Postgres) and search stays embedded. Naming an external database for a store does not also start an embedded postmaster for that store.

## Overlay

`TransactionCacheDb` in the Java framework is a private H2 database per `ExecutionContext`. `startTxCacheDb` creates one, and `close` shuts it down. Several can exist at the same time, one per sim context, including nested `enter_sim` agents. The per-transaction `TransactionCache` is a map, not a database, and it stays a map.

The overlay is a second cluster, started on the first `startTxCacheDb` and kept for the life of the process. Each overlay is a schema on that cluster. `close` drops the schema. Concurrent sims are concurrent schemas and concurrent connections. The data directory is a temp directory. One long-lived cluster avoids an initdb on every `enter_sim`. The overlay is not the entity cluster, so a sim schema is not stored in the entity data directory. The schema starts empty. The driver does not copy entity tables into it. How a sim read sees entity rows stays with the Java HOLD/FLUSH port.

## Windows

theseus runs on Windows. Embedded entity, embedded search, and the sim overlay start there. Windows stays a lower-priority test target. The 51.6 MB archive is only in the Windows build.

## Startup, shutdown, and tests

The blocking start and stop are bridged once. That runtime has exited before the pool connects. Per-query `DbDriver` methods are the checked-out `postgres::Client` driving its own runtime.

A host test that needs more than one database uses two clusters.

## Dev snapshots and upgrades

`save-db` and `load-save` are binary commands, the Gradle tasks of those names. They run `pg_dump` and `pg_restore` from the theseus archive against the entity cluster, custom format. When the `search` schema lives on that cluster, the dump includes it. `plans/License-Review.md` walked every ELF in the 18.6.0 archive. `psql` is the only binary that needs `libreadline`, and it stays stripped. `pg_dump`, `pg_restore`, `pg_upgrade`, and `pg_basebackup` stay in the payload. Each theseus bump rechecks `NEEDED` the same way.

`PG_VERSION` in the data directory is the installed major version. A minor bump starts the new postmaster on the same directory. A major bump cannot open the old directory. The previous server binaries stay beside the new install, and startup runs `pg_upgrade`, or dumps and restores.

After a hard kill, startup reads `postmaster.pid`. A live process on our data directory is adopted when the version matches, and refused with that pid when it does not. Startup does not kill an unrelated postmaster and does not start a second one on the same directory.

## What this does not change

- HOLD, FLUSH, and `enter_sim` follow the Java framework at the time of the port. This plan is the lifecycle of the database under `TransactionCacheDb`.
- The per-transaction `TransactionCache` stays an in-memory map.
- External Postgres uses the same `postgres::Client` and the same core pool as an embedded cluster socket. External MySQL is a `DbDriver` on that same manager. There is no SQLx dependency.
- Search still has its own logical store. When the entity datasource is Postgres, embedded or external, that store is the `search` schema on that database. A search URL can place it on another Postgres.
