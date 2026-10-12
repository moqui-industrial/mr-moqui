# SSH and SFTP

The crates for an SSH client, an SSH server, and SFTP on both sides. The listener, the SFTP calls, and the admin shell are not built in this pass. The crates are linked from `moqui-host` so the resolve and the license graph are real.

This follows Tool components and print in `plans/Architecture-Proposal.md`. The cluster default stays the HTTPS POST in `plans/Cluster.md`.

## Decision

`russh` 0.64 is the SSH client and the SSH server. `russh-sftp` 3 is SFTP version 3 for both directions. It does not depend on `russh`. The host opens a subsystem channel and feeds that crate the bytes.

Both crates are Apache-2.0. They live in `moqui-host`, with the other process-edge dependencies. `russh` is built with `default-features = false` and with `aws-lc-rs`, `flate2`, and `rsa`. `aws-lc-rs` is the same crypto backend as embedded Postgres, external Postgres TLS, and `lettre`. `rsa` stays because real servers still present RSA host keys. `flate2` is SSH compression. The enabled backend is `miniz_oxide`, already in the graph. `zlib-rs` and `libz-sys` stay off. `ring`, `des`, and `dsa` stay off.

`russh-sftp` has no crypto feature. Its `async-trait` feature stays off. It pulls `chrono` 0.4 for SFTP timestamps. Moqui time stays `jiff`. Nothing else in the framework uses `chrono`.

`russh` 0.64 depends on `rsa` 0.10.0-rc and `ssh-key` 0.7.0-rc. Those are the upstream pins this version ships. `libssh` stays out. It is LGPL. `ssh2` and `libssh2` stay out. They are a C client with no server.

Nothing in this pass opens a port or transfers a file. The dependency is the choice, recorded before the server exists.

## Later: SFTP and the admin server

An SFTP client covers what `moqui-sftp` does today. A built-in sshd in the host covers server-to-server file and command exchange, and it replaces the Groovy shell. There is still no web eval endpoint.

Authentication goes through the user facade and artifact authz. `russh` supplies the channel and the PTY. The command loop is Moqui: a small command set, plus a JavaScript runner. The SFTP server root is a resource location, not a view of the host disk. The host key is a database row, the same idea as the cluster certificate in `plans/Cluster.md`.

## Later: remotefs-ssh

`remotefs-ssh` is not a dependency. When it is reviewed, two uses are in scope. An SFTP server can use `DbResource` as its backend. An SFTP client can sit behind `ResourceFacade`. Either one is a separate pass over that crate's license and its SSH backend, which would be the `russh` feature rather than `libssh2` or `libssh`.

## Later: cluster channel

An optional cluster sender can be a `russh` session to each peer. One channel carries cache invalidation and another carries notifications, using the same topic frames as the HTTPS POST. It is not an SSH tunnel with HTTPS inside it. A pooled HTTPS POST stays the default, as `plans/Cluster.md` specifies. A bulk update that invalidates one cache many times collapses to one message, on either sender. `NOTIFY` remains the unnamed fallback for peers that cannot open a direct connection.

## Out of scope

- A listener, an auth handler, or a shell command, in this pass.
- Linking `remotefs-ssh`.
- Changing the cluster default away from HTTPS.
- Using `chrono` as a Moqui clock.
