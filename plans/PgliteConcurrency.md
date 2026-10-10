# PGLite concurrency

How the embedded database serves many `ExecutionContext` threads.

This follows `plans/Architecture-Proposal.md` (Database, Platform notes) and `plans/Crate-Layout.md` (`DbDriver` in `moqui-host`). Transaction-cache and sim-mode behavior are a port of the Java framework at the time of the port. They are not specified here.

## Decision

A shared embedded database uses PGLite's multi-process mode. Each framework thread gets its own connection, and Postgres provides the concurrency. The in-process single-backend API is for a database that one thread owns.

`moqui-host`'s `db` module owns this. `DbDriver` stays a blocking trait. Core does not call `pglite-rs`.

## Shared databases

The default entity database, and the default search database when it is also embedded, are shared. Requests, async services, and scheduled jobs use them from different threads at the same time. `force-new` needs a second connection on the calling thread. That is the same shape as the JDBC pool today.

On Unix (Linux and macOS) the driver opens that database with `PGlite::open_multi_process`. That starts a child postmaster and exposes a connection URI. The driver pools ordinary Postgres connections against it and blocks the calling thread for each `DbDriver` call. Pool size follows the datasource pool settings. `extra_connections` on `MultiProcessOptions` is at least that pool size, so a checkout is not rejected while the pool still has a free slot.

Two embedded databases, entity and search, are two child postmasters. A configured external Postgres replaces the child for the store it is configured on. A production configuration that names an external database does not start a PGLite postmaster for that store.

In-process `PGlite::open` is one backend. A mutex around it would run the whole server one query at a time, and it can deadlock if a call re-enters the database while holding the lock. The shared databases do not use that mode on Unix.

`pglite-rs` 0.2 is the crate. Its multi-process API is the one this plan depends on (`open_multi_process`, `connection_uri`, `MultiProcessOptions`).

## One thread, one engine

`PGlite::open_temp` stays the in-memory engine. A sim overlay is a separate short-lived instance, not a connection on the shared database. The Java behavior being ported uses that overlay from the sim's `ExecutionContext`. One thread owns that instance, so the in-process engine fits. If the Java code at port time shares one overlay across threads, that instance follows the shared-database rule above. This plan does not change the overlay's read and write rules.

## Windows

`pglite-rs` 0.2 compiles `open_multi_process` for Unix only. Windows development still has to start with no external database installed.

The Windows driver uses the in-process engine behind a mutex. The default configuration starts. Concurrent transactions on that database run one at a time. External Postgres is how a Windows machine gets real concurrency. If a later `pglite-rs` supports multi-process on Windows, the Unix path becomes the Windows path and the mutex goes away.

## What this does not change

- Transaction cache, sim mode, and `enter_sim` follow the Java framework at the time of the port.
- External Postgres and MySQL drivers are SQLx pools.
- Search still has its own store. That store may be this embedded database or an external Postgres.
