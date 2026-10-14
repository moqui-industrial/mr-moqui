# SSH and SFTP

An SFTP client with the `moqui-sftp` services, an SSH server with an SFTP subsystem and an admin shell, and an optional SSH cluster channel. Built in phase 12.

This follows Tool components and print in `plans/Architecture-Proposal.md`. The cluster default stays the HTTPS POST in `plans/Cluster.md`.

## Decision

`russh` 0.64 is the SSH client and the SSH server. `russh-sftp` 3 is SFTP version 3 for both directions. It does not depend on `russh`. The host opens a subsystem channel and feeds that crate the bytes.

Both crates are Apache-2.0. They live in `moqui-host`, with the other process-edge dependencies. `russh` is built with `default-features = false` and with `aws-lc-rs`, `flate2`, and `rsa`. `aws-lc-rs` is the same crypto backend as embedded Postgres, external Postgres TLS, and `lettre`. `rsa` stays because real servers still present RSA host keys. `flate2` is SSH compression. The enabled backend is `miniz_oxide`, already in the graph. `zlib-rs` and `libz-sys` stay off. `ring`, `des`, and `dsa` stay off.

`russh-sftp` has no crypto feature. Its `async-trait` feature stays off. It pulls `chrono` 0.4 for SFTP timestamps. Moqui time stays `jiff`. Nothing else in the framework uses `chrono`.

`russh` 0.64 depends on `rsa` 0.10.0-rc and `ssh-key` 0.7.0-rc. Those are the upstream pins this version ships. `libssh` stays out. It is LGPL. `ssh2` and `libssh2` stay out. They are a C client with no server.

The async work runs on the host's edge runtime (`crates/moqui-host/src/edge.rs`); a blocking caller waits on a channel.

## SFTP client: `moqui-sftp` in the framework

Core has the client logic (`crates/moqui-core/src/sftp.rs`) and the host installs the protocol calls (`crates/moqui-host/src/ssh/client.rs`). The behavior is the Java `SftpClient`:

- `SftpClient(host, username, port)`: a `host:port` string wins; without a port it is 115, Java's `SFTP_PORT`. Most servers listen on 22, so a remote names its port.
- `password`, `publicKeyPkcs8` (PEM: PKCS#8, PKCS#1, OpenSSH, or PuTTY; callable more than once), `preserveAttributes`, `connect`, and Java's messages for a missing host, username, or credential, and for a call before `connect`.
- `lsFiles`, `ls` (`name`, `path`, `isFile`, `size`, `lastAccess`, `lastModified`), `getBinary`, `getText(path, charset)`, `put` (to `path/filename` when `path` is a directory, else to `path`; `createDir` makes the parent of `path`), `openStream`, `putStream`, `moveFile`, `renameFile`, `rename`, `rm`, `close`. Charsets are `encoding_rs`. With `preserveAttributes` a put sets the mode, as SSHJ's preserve does.
- Scripts get a global `SftpClient`. Bytes are `Uint8Array`. `publicKey(PrivateKey, PublicKey)`, `publicKey(KeyProvider...)` and `getInternalClient()` take Java objects and throw.

`org.moqui.sftp.SftpMessageServices.send#SystemMessageSftp` and `poll#SystemMessageSftp` keep the Java names, parameters, and logic, as Rust bodies. A type whose `sendServiceName` is the SFTP send goes through the normal queue, send, receive, and consume cycle; the poll runs as a service job. Paths are the port's JavaScript `${}` expansion, so a Groovy `?:` is an error that says to use `??`.

Host keys: Java accepted any key. `SystemMessageRemote.sshHostKeyFingerprint` pins one (`SHA256:...`). Without a pin the key is trusted on first use and recorded in `runtime/conf/ssh_known_hosts` (OpenSSH format, never `~/.ssh`), and a changed key is refused with both fingerprints in the error.

## SSH server

Off unless `ssh_port` is set (`0` binds an ephemeral port). `ssh_host` defaults to `127.0.0.1`. The host key is `ssh_host_key`, a secret property: the base64 of a 32-byte Ed25519 seed, which the `CHANGEME` generator writes into `runtime/.env` (`plans/MimoReview.md` Decision 1), or an OpenSSH private key. It is not a database row. At most `ssh_max_sessions` (8) connections are open at once.

Authentication goes through the user facade: a password through `loginUser` (failures count toward the lockout; an account that needs a second factor cannot log in here), or a public key that is an active `moqui.security.UserSshKey` of that user, matched by fingerprint.

- `SSH_SFTP` (granted to ADMIN) allows the `sftp` subsystem. The root is the resource location `ssh_sftp_root` (default `runtime://sftp`), not a view of the host disk: `runtime`, `file`, `component`, or `dbresource`. A path that climbs out of the root is refused. A file is read or written whole (a `dbresource` file is a row), up to `ssh_sftp_max_file` bytes.
- `SSH_SHELL` (granted to ADMIN_ADV, and sealed) allows `exec` and an interactive shell. It replaces the Groovy shell. There is still no web eval endpoint. Commands: `help`, `status`, `service <name> [json]`, `entity <name> [json]`, `cache clear [name]`, `js <code>` (a block until a line of `.` when alone), `exit`. Each command runs in a context logged in as the SSH user, so artifact authorization applies to every service and entity call.

## Cluster channel

`cluster_transport=ssh` makes the cluster sender an SSH client. A member advertises `ssh://host:port` (its SSH server; the default when `cluster_base_url` is empty) and records its host key fingerprint on its `ClusterMember` row (`sshHostKeyFingerprint`), which peers pin. A peer logs in as `moqui-cluster` with an Ed25519 key derived from the shared `cluster_tls_key`, keeps one session to each peer and one `exec` channel for each topic, and writes length-prefixed frames of the same topic JSON as the HTTPS body. The receiver applies each frame through the same path as `POST /cluster/{topic}` and answers with that status. It is not an SSH tunnel with HTTPS inside it. A peer whose base URL is `https://` still gets the pooled HTTPS POST, which stays the default, as `plans/Cluster.md` specifies. A bulk update that invalidates one cache many times collapses to one message on either sender. Peers that cannot open a direct connection use the `NOTIFY` transport in `plans/Cluster.md`.

## `remotefs-ssh`

Not a dependency. The SFTP server already serves `DbResource` through `ResourceFacade`. An `sftp://` scheme behind `ResourceFacade` is not added; if it is, it is the client above, not `remotefs-ssh` with `libssh2` or `libssh`.

## Out of scope

- An `sftp://` resource scheme.
- Linking `remotefs-ssh`.
- Changing the cluster default away from HTTPS.
- Port forwarding, agent forwarding, and X11 on the server.
- Using `chrono` as a Moqui clock.
