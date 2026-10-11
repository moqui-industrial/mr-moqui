# PGLite concurrency

How the embedded database serves many `ExecutionContext` threads, and where a sim overlay lives.

This follows `plans/Architecture-Proposal.md` (Database, Platform notes) and `plans/Crate-Layout.md` (`DbDriver` in `moqui-host`). HOLD and FLUSH rules are a port of the Java framework at the time of the port. They are not specified here. The database lifecycle is.

`pglite-rs` 0.2 is the crate. The API this plan uses is `open_multi_process`, `connection_uri`, and `MultiProcessOptions`. The in-process API is `open` and `open_temp`.

## Engine limits

These are properties of `pglite-rs` 0.2.7, and the layout below is built around them.

- `open_multi_process` starts a child postmaster and returns a connection URI. Many of those clusters can run in one process. The feature is compiled for Unix only. Windows builds fail if the `multiple-process` feature is turned on.
- The in-process engine (`open`, `open_temp`) can be opened once per process. A second open returns `AlreadyOpen` while the first is alive, and `ReopenUnsupported` after it closes. Closing it does not allow another open.
- `open_temp` writes a directory under the system temp dir and deletes that directory when the instance is dropped. It is not an in-memory database. The published footprint of about 23 MB RSS is this in-process engine. A multi-process pool is a child postmaster and is larger.
- In-process calls take the crate's own per-call lock. A second lock around the same engine deadlocks if a call re-enters the database while the first lock is held.
- The socket path for a child postmaster has to stay under 96 bytes. If the host process is killed with SIGKILL, the child is not signaled. The next open on that data directory depends on Postgres handling a stale `postmaster.pid`. A normal shutdown uses the crate's shutdown path: SIGINT, then a wait.

`moqui-host`'s `db` module owns this. `DbDriver` stays a blocking trait. Core does not call `pglite-rs`.

## Shared database

On Unix the default entity database is one cluster, opened with `PGlite::open_multi_process`. Each framework thread checks out its own connection, and Postgres provides the concurrency. `force-new` is a second connection on the calling thread, the same shape as the JDBC pool today.

The driver pools ordinary Postgres connections against `connection_uri` and blocks the calling thread for each `DbDriver` call. The future is driven with the host Tokio handle's `block_on` from that worker thread, the same way an external Postgres socket is driven. `futures::executor::block_on` does not drive those sockets. It is only appropriate for an in-process call, which completes when it is polled.

Pool size follows the datasource pool settings. `MultiProcessOptions.max_connections` is the crate's own pool, and the API treats a value below 1 as 1. Postgres is started with `max_connections` equal to that pool size, plus 2, plus `extra_connections`. The host uses its own pool, so `max_connections` stays 1 and `extra_connections` is the host pool size. A checkout is then not rejected while the host pool still has a free slot.

Embedded search uses the `search` schema on this same cluster. A second postmaster starts only when the search store is actually a different database: search config names another Postgres, or the entity datasource is MySQL (or another non-Postgres) and search stays embedded. A production configuration that names an external database for a store does not start a PGLite postmaster for that store.

## Overlay

`TransactionCacheDb` in the Java framework is a private H2 database per `ExecutionContext`. `startTxCacheDb` creates one, and `close` shuts it down. Several can exist at the same time, one per sim context, including nested `enter_sim` agents. The per-transaction `TransactionCache` is a map, not a database, and it stays a map.

`open_temp` cannot stand in for that. The process can open the in-process engine only once, and it cannot open it again after close.

On Unix the overlay is a second multi-process cluster, started on the first `startTxCacheDb` and kept for the life of the process. Each overlay is a schema on that cluster. `close` drops the schema. Concurrent sims are concurrent schemas and concurrent connections. The data directory is a temp directory. One long-lived cluster avoids an initdb on every `enter_sim`. The overlay is not the entity cluster, so a sim schema is not stored in the entity data directory.

## Windows

Windows development still has to start with no external database installed, and `open_multi_process` is not available there.

The Windows driver uses the one in-process engine for the whole embedded store: entity tables, the `search` schema, and overlay schemas. The crate's per-call lock serializes use. The host does not add a mutex, and it does not hold a lock across a transaction. Concurrent transactions on that database run one at a time. `force-new` is a savepoint on the single session, because a second connection cannot run beside the first.

A second in-process open does not succeed, so the Windows driver never tries to start a second engine. A workload that needs real concurrency, or a sim whose files are isolated from the entity data directory, uses an external Postgres. If a later `pglite-rs` supports multi-process on Windows, the Unix path becomes the Windows path.

## Blocking and tests

Shared and overlay clusters are sockets, so their `DbDriver` methods use the Tokio handle. The in-process engine, used on Windows, is polled with `futures::executor::block_on` because the call completes when polled.

A host test that needs more than one database uses multi-process clusters. The in-process engine opens once per process and cannot be reopened.

## What this does not change

- HOLD, FLUSH, and `enter_sim` follow the Java framework at the time of the port. This plan is the lifecycle of the database under `TransactionCacheDb`.
- The per-transaction `TransactionCache` stays an in-memory map.
- External Postgres and MySQL drivers are SQLx pools. An external Postgres `DbDriver` blocks with the same Tokio handle as a PGLite cluster.
- Search still has its own logical store. On the default embedded cluster that store is the `search` schema. Config can place it on another Postgres.
