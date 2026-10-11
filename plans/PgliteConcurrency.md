# PGLite concurrency

How the embedded database serves many `ExecutionContext` threads, and where a sim overlay lives.

This follows `plans/Architecture-Proposal.md` (Database, Platform notes) and `plans/Crate-Layout.md` (`DbDriver` in `moqui-host`). HOLD and FLUSH rules are a port of the Java framework at the time of the port. They are not specified here. The database lifecycle is.

`pglite-rs` 0.2 is the crate. The API this plan uses is `open_multi_process`, `connection_uri`, `socket_path`, and `MultiProcessOptions`. The in-process API is `open` and `open_temp`. Those two stay in the engine limits below. The driver does not call them.

## Engine limits

These are properties of `pglite-rs` 0.2.7, and the layout below is built around them.

- `open_multi_process` starts a child postmaster and returns a connection URI. Many of those clusters can run in one process. The feature is compiled for Unix only. Windows builds fail if the `multiple-process` feature is turned on.
- The in-process engine (`open`, `open_temp`) can be opened once per process. A second open returns `AlreadyOpen` while the first is alive, and `ReopenUnsupported` after it closes. Closing it does not allow another open.
- `open_temp` writes a directory under the system temp dir and deletes that directory when the instance is dropped. It is not an in-memory database. The published footprint of about 23 MB RSS is this in-process engine. A multi-process pool is a child postmaster and is larger.
- In-process calls take the crate's own per-call lock. A second lock around the same engine deadlocks if a call re-enters the database while the first lock is held.
- The socket path for a child postmaster has to stay under 96 bytes. If the host process is killed with SIGKILL, the child is not signaled. The next open on that data directory depends on Postgres handling a stale `postmaster.pid`. A normal shutdown uses the crate's shutdown path: SIGINT, then a wait.

`moqui-host`'s `db` module owns this. `DbDriver` stays a blocking trait. Core does not call `pglite-rs`. `pglite-rs` is lifecycle only: `open_multi_process` at startup, `connection_uri` or `socket_path` once to build the sync pool, and `close` at shutdown. On a multi-process instance `unix_uri()` returns `connection_uri()` and does not start the in-process socket gateway. Per-query traffic never touches the `pglite-rs` async API.

## Shared database

On Unix the default entity database is one cluster, opened with `PGlite::open_multi_process`. Each framework thread checks out its own connection, and Postgres provides the concurrency. `force-new` is a second connection on the calling thread, the same shape as the JDBC pool today. A savepoint cannot commit on its own, so `force-new` does not use one.

After open, the driver reads `connection_uri()` or `socket_path()`. On `pglite-rs` 0.2.7 the multi-process URI is `postgresql://{user}@localhost/{db}?host={socket_dir}`. The authority host is `localhost`. The query `host` is the socket directory, and `socket_path()` is the file `.s.PGSQL.{port}` inside it. `postgres::Config`'s URL parser records the authority and then the query host, and tries them in that order, so a parsed URI connects to TCP 5432 before the child. The URI also omits the port. `MultiProcessOptions.port` defaults to 5432, and the client looks up `.s.PGSQL.{port}` in the directory it is given.

The driver builds `postgres::Config` from the username, the database, that port, and the socket directory (`host` of the directory, or `host_path` on `socket_path().parent()`), with `NoTls`. It does not parse the URI. Each pooled `postgres::Client` owns a `current_thread` runtime and drives it with `block_on` on connect, on every query, and on drop. The host does not pass in a Tokio handle and does not call `block_on`. Connect, query, and pool drop run on factory `std` threads. `Runtime` construction panics on a thread that is already inside a runtime, so the bridge that awaited `open_multi_process` has exited before `PostgresConnectionManager` connects, and `close` is not run on a thread that still holds the pool. Pool size is also a runtime count: one runtime per pooled connection, with memory and speed testing to follow.

A transaction pins one checked-out client to the worker thread until commit or rollback. A call outside a transaction checks out and returns. `force-new` checks out a second client on that thread. A thread that already holds a client must not block in `r2d2::Pool::get()` for a connection another holder still has. Each pool's max size is at least the number of worker threads that can hold a connection from it, plus one spare, so one `force-new` does not wait on a holder. A second concurrent `force-new` on that same pool waits. The entity cluster and the overlay cluster are two pools, each with its own spare.

Pool sizes and timeouts start at the Java datasource values, and the spare above is added when that value does not already cover it. `MultiProcessOptions.max_connections` is the crate's own pool, and the API treats a value below 1 as 1. Postgres is started with `max_connections` equal to that pool size, plus 2, plus `extra_connections`. The host uses `r2d2`, so the crate's `max_connections` stays 1 and `extra_connections` is the host pool size, spare included. A checkout is then not rejected while the host pool still has a free slot.

Embedded search uses the `search` schema on this same cluster. A second postmaster starts only when the search store is actually a different database: search config names another Postgres, or the entity datasource is MySQL (or another non-Postgres) and search stays embedded. That second cluster is Unix only. On Windows the same configuration needs an external Postgres or OpenSearch for search. A production configuration that names an external database for a store does not start a PGLite postmaster for that store.

## Overlay

`TransactionCacheDb` in the Java framework is a private H2 database per `ExecutionContext`. `startTxCacheDb` creates one, and `close` shuts it down. Several can exist at the same time, one per sim context, including nested `enter_sim` agents. The per-transaction `TransactionCache` is a map, not a database, and it stays a map.

`open_temp` cannot stand in for that. The process can open the in-process engine only once, and it cannot open it again after close.

On Unix the overlay is a second multi-process cluster, started on the first `startTxCacheDb` and kept for the life of the process. Each overlay is a schema on that cluster. `close` drops the schema. Concurrent sims are concurrent schemas and concurrent connections. The data directory is a temp directory. One long-lived cluster avoids an initdb on every `enter_sim`. The overlay is not the entity cluster, so a sim schema is not stored in the entity data directory. The schema starts empty. The driver does not copy entity tables into it. How a sim read sees entity rows stays with the Java HOLD/FLUSH port.

## Windows

PGLite is Unix only. `open_multi_process` is compiled for Unix, and the in-process engine is not a Windows stand-in. Windows is a lower priority platform. The `multiple-process` feature stays off on Windows builds.

Startup fails when entities or search would need a PGLite postmaster. The error names that store and says to configure an external Postgres for it. OpenSearch satisfies search without a Postgres, so a Windows process with external Postgres or MySQL entities and an OpenSearch search store starts. A MySQL-only configuration is asked for an external Postgres only for the store that still needs one. Sim has no non-PGLite path. Entering sim on Windows fails, because the overlay cluster cannot start. Process startup does not fail only because sim exists in the product.

## Startup, shutdown, and tests

`open_multi_process` and `close` are async. They are bridged once during factory startup and shutdown. That bridge's runtime has exited before the pool connects, and `close` runs where dropping the pool is safe. `connection_uri` and `socket_path` are sync reads. Per-query `DbDriver` methods are the checked-out `postgres::Client` driving its own runtime. The host does not call `open` or `open_temp`.

A host test that needs more than one database uses multi-process clusters.

## What this does not change

- HOLD, FLUSH, and `enter_sim` follow the Java framework at the time of the port. This plan is the lifecycle of the database under `TransactionCacheDb`.
- The per-transaction `TransactionCache` stays an in-memory map.
- External Postgres uses the same sync `postgres::Client` pool as a PGLite cluster socket. External MySQL uses the sync `mysql::Pool`. There is no SQLx dependency.
- Search still has its own logical store. On the default embedded cluster that store is the `search` schema. Config can place it on another Postgres.
