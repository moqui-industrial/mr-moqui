# Cluster coordination

How instances that share a database find each other, and how cache invalidation and notifications cross the process boundary. Session bytes do not. The session struct and its flush modes are Sessions and login in `plans/Architecture-Proposal.md`.

This follows that section and the cache row in the same proposal. `moqui-hazelcast` is the Java behavior being replaced. It is an add-on component, not part of the framework tree.

## Decision

A cluster exists when more than one instance uses the same database. Membership is a table in that database. Each instance upserts its own row on startup and refreshes a heartbeat. Peers are the other rows whose heartbeat is newer than a few intervals. There is no seed list and no multicast.

Peers call each other over HTTPS. The cluster endpoint requires a client certificate. The certificate and its private key are one row in the same database. The first instance to start generates the keypair when that row is empty and stores it. Later instances load it. Each instance reads the key from the database. The connection between instances carries a TLS signature, and does not carry the key.

The local publish stays a `SimpleTopic`. A listener fans that publish out to live peers. The receiver applies the message locally and does not send it on. One live member means no fan-out.

The member row holds the cluster base URL, the address peers call. Only that endpoint requires the client certificate. Public application routes do not.

`moqui-host` owns the HTTPS listener and the client calls. The member table, the certificate row, and the topic listener are in `moqui-core`, beside the cache and the session. There is no new crate, no Redis, no gossip library, no Raft, and no Hazelcast. `rustls` is already the TLS stack.

Operational state that instances must share, or that must survive a restart, goes in the database. Logs, the embedded Postgres data directory, and temp files stay on disk. The database files themselves stay files.

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

The member row is the instance id, the cluster base URL, and the heartbeat timestamp. Startup upserts it. A row that stops refreshing drops out of the peer set after a few intervals. The certificate row is separate and singular. Every instance that can read the database can load the same keypair.

## Transport

The sender is a small trait. The default implementation is an HTTPS POST to `{base}/cluster/{topic}` with the shared client certificate. The message carries an origin id. A failed send is retried briefly.

Cache invalidation is safe to apply twice. A notification carries a message id so a retry does not deliver it twice.

Postgres `NOTIFY` can be another sender later, for peers that share the database and cannot open HTTPS to each other. It is not the default, and it is not specified further.

## Sessions

The snapshot row is the share. A second instance loads the session by id from the database. It does not receive the session over the cluster connection. Flush modes, the reserve connection, and the guarantee points are Sessions and login in `plans/Architecture-Proposal.md`.

## When it lands

The session row is written in phase 7 of `plans/Implementation-Phases.md`, with the web edge. The member heartbeat, the certificate row, and the topic fan-out land in phase 10, with notifications. A single-node gate does not need a second instance.

## Out of scope

- A distributed service executor.
- A replicated in-memory map or other Hazelcast primitive beyond the two topics.
- Sticky sessions as a requirement. A load balancer may be sticky. Failover without stickiness uses the session row and the guarantee flush.
- A file snapshot under `runtime/sessions`.
- A seed list, multicast, or any discovery that does not read the member table.
- A pre-shared key sent on the request. The credential on the wire is the TLS client certificate.
