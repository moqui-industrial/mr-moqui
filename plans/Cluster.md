# Cluster coordination

How instances that share a database find each other, and how cache invalidation and notifications cross the process boundary. Session bytes do not. The session struct and its flush modes are Sessions and login in `plans/Architecture-Proposal.md`.

This follows that section and the cache row in the same proposal. `moqui-hazelcast` is the Java behavior being replaced. It is an add-on component, not part of the framework tree.

## Decision

A cluster exists when more than one instance uses the same database. Membership is a table in that database. Each instance upserts its own row on startup and refreshes a heartbeat. Peers are the other rows whose heartbeat is newer than a few intervals. There is no seed list and no multicast.

Peers call each other over HTTPS. The cluster endpoint requires a client certificate. The certificate and its private key are secret configuration, not database rows: `cluster_tls_cert` and `cluster_tls_key` `default-property`s in `MoquiDefaultConf.xml`, the `entity_ds_crypt_pass` pattern, set by config file or environment and generated into `runtime/.env` when left at the `CHANGEME` default (Decision 1 of `plans/MimoReview.md`). Clustered deployments provision the same pair on every instance; the pair is base64 DER on single-line `.env` values. The connection between instances carries a TLS signature, and does not carry the key. The session-cookie HMAC key (`session_cookie_key`) is a different property. The two keys stay distinct.

The local publish stays a `SimpleTopic`. The local `MCache` drop runs on the writer. The peer message is the cache key, queued at commit and sent by one host thread after the enlisted client is back in the pool. The topic does not carry the entity body or the session map. The receiver applies the message locally and does not send it on. One live member means no fan-out.

The member row holds the cluster base URL, the address peers call. Only that endpoint requires the client certificate. Public application routes do not.

`moqui-host` owns the HTTPS listener and the client calls. The member table, the key-material loading, and the topic listener are in `moqui-core`, beside the cache and the session. There is no new crate, no Redis, no gossip library, no Raft, and no Hazelcast. `rustls` is already the TLS stack.

Operational state that instances must share, or that must survive a restart, goes in the database. Secrets do not. Key material is configuration (`runtime/.env` or the deployment's environment), never a row. Logs, the embedded Postgres data directory, and temp files stay on disk. The database files themselves stay files.

## What moqui-hazelcast wires

The component turns on five uses. Two stay, as topics. Three do not.

| Java use | Disposition |
|---|---|
| `entity-cache-invalidate` topic | Stays a `SimpleTopic`. The cluster listener fans it out. The local `MCache` drops the named entry. Cache entries are not stored in a distributed map. |
| `notification-message` topic | Stays a `SimpleTopic`. The cluster listener fans it out. The instance that holds that user's WebSocket applies it. The others ignore it. |
| Web session filter | Not ported. The session snapshot is a database row. That row is how instances share a session. Peers do not copy session bytes. |
| `service-executor` | Not ported. Service jobs already coordinate through the database. |
| `javax.cache` distributed cache | Not ported. There is one local cache. Invalidation is the cluster behavior. |

## Membership

The member row is the instance id, the cluster base URL, the heartbeat timestamp, and key fingerprints (SHA-256 of the session-cookie HMAC key, and the certificate fingerprint). Startup upserts it. A row that stops refreshing drops out of the peer set after a few intervals. The keys themselves are configuration, shared by provisioning. A member whose fingerprints differ from the live peer set fails startup with a clear message, so a copy-paste provisioning mistake is caught instead of surfacing as invalid cookies.

## Transport

The sender is a small trait. The default implementation is an HTTPS POST to `{base}/cluster/{topic}` with the shared client certificate. The HTTP client keeps that connection pooled. The send runs on one host thread, after commit, and only after the enlisted client has been returned. The message is the cache key and carries an origin id. An invalidation is safe to apply twice. A failed send is retried briefly. A later optional sender is a `russh` session with two channels, one for each topic, specified in `plans/Ssh.md`. The default remains this POST.

A notification carries a message id so a retry does not deliver it twice.

Postgres `NOTIFY` can be another sender later, for peers that share the database and cannot open HTTPS to each other. It is not the default, and it is not specified further.

## Sessions

The share is the `moqui.server.HttpSession` row through the entity one-cache. A second instance loads that row by id. Invalidation uses the entity-cache topic, which drops the one-cache entry. The receiver does not apply a copy of the attribute map. Session bytes still do not cross the cluster connection. The cookie signature is HMAC-SHA256 over the session id. That key is its own secret property (`session_cookie_key`), generated into `runtime/.env` when left at the `CHANGEME` default. It is not the cluster certificate. A bad MAC is an unknown session. The merge, the reserve connection, and the flush modes are Sessions and login in `plans/Architecture-Proposal.md`.

## When it lands

The session row is written in phase 7 of `plans/Implementation-Phases.md`, with the web edge. The member heartbeat, the key fingerprints, and the topic fan-out land in phase 10, with notifications. A single-node gate does not need a second instance.

## Out of scope

- A distributed service executor.
- A replicated in-memory map or other Hazelcast primitive beyond the two topics.
- Sticky sessions as a requirement. A load balancer may be sticky. Failover without stickiness uses the session row and the guarantee flush.
- A file snapshot under `runtime/sessions`.
- A seed list, multicast, or any discovery that does not read the member table.
- A pre-shared key sent on the request. The credential on the wire is the TLS client certificate.
- Key material as database rows. Keys are configuration (`runtime/.env` or environment).
