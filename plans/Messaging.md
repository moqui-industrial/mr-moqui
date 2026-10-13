# Implementation Plan: Runtime-Reflective FlatBuffers for Moqui

Architecture specification · Rust server · Browser client · Dynamic schemas · Binary WebSockets · Large-scale fan-out

## 1. Executive decision

Build a schema-driven binary document system for internal communication using FlatBuffers, with runtime schema reflection, immutable schema versions, and shared message buffers.

This is an internal messaging optimization, not a replacement for Moqui's REST APIs or existing serialization formats. REST APIs must remain supported, including JSON and other currently supported formats. Internal callers may use the binary protocol; public and integration clients must not require FlatBuffers or `.bfbs` reflection.

The application will support:

- New Moqui Data Document types created without redeploying the server or client.
- Arbitrarily deep, nested, strictly typed document structures, subject to configurable safety limits.
- Separate read and write schema compatibility policies.
- Direct, zero-copy reads of FlatBuffers from received binary buffers.
- Runtime schema synchronization between Rust and browser clients.
- 100,000 incoming document updates per second.
- Fan-out toward as many as 10 million concurrent browser connections.
- Minimal handwritten protocol code, using existing FlatBuffers tooling wherever practical.

### Proposed technology stack

| Component                | Decision                                                |
| ------------------------ | ------------------------------------------------------- |
| Application server       | Rust                                                    |
| Async networking         | Tokio                                                   |
| HTTP/REST/WebSocket      | Axum, initially; preserve existing Moqui REST contracts  |
| Binary representation    | FlatBuffers                                             |
| Runtime schema           | FlatBuffers `.bfbs`                                     |
| Schema compiler          | `flatc`                                                 |
| Rust reflection          | `flatbuffers-reflection`, extended only where necessary |
| Browser client           | TypeScript, `flatbuffers` npm package                   |
| Browser reflection       | Small shared runtime schema interpreter                 |
| Schema registry          | Authoritative database + immutable in-memory caches     |
| Fan-out payload          | `bytes::Bytes` with shared ownership                    |
| Document synchronization | Snapshots + revisioned patches                          |
| Schema synchronization   | Versioned publication and on-demand retrieval           |

The key architectural decision is to separate the stable networking protocol from the dynamically defined document schemas, and keep transport/format adapters separate from application operations.

FlatBuffers officially supports binary schemas as a runtime intermediate representation. Its Rust reflection support exists, but is still described as experimental, so production qualification of that dependency is a required milestone—not something to assume complete.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://flatbuffers.dev\&sz=32)

FlatBuffers Docs

+1



## 2. Non-negotiable architectural constraints

These requirements determine which implementations are acceptable.

| Constraint             | Required behavior                                            |
| ---------------------- | ------------------------------------------------------------ |
| REST compatibility    | Preserve REST APIs, JSON and other supported formats         |
| Zero-copy data access  | Read fields directly from internal binary buffers            |
| Runtime type creation  | New document schema available without recompilation          |
| Strict types           | Server verifies exact field types and constraints            |
| Schema consistency     | A schema ID always identifies the same immutable definition  |
| Browser compatibility  | No native browser extensions or required C++ runtime         |
| Production safety      | Untrusted messages fully verified before field access        |
| Bounded memory         | No unbounded message, subscription or write queues           |
| Backward compatibility | Version-aware reading and controlled write migration         |
| Large-scale fan-out    | Encode an update once per payload variant, not per recipient |

### Important limits

There are four constraints I would explicitly document.

First: zero-copy means zero-copy structured reads, not guaranteed zero-copy networking. Browser `ArrayBuffer` delivery, TLS encryption and kernel networking can involve copies.

Second: FlatBuffers are not mutable general-purpose object graphs. Some scalar fields can be changed in place, but adding nested objects or resizing strings usually requires rebuilding the message.

Third: schema reflection does not eliminate schema validation. A `.bfbs` descriptor describes a binary structure. Moqui constraints, authorization and write semantics still need enforcement.

Fourth: 100,000 incoming updates per second is fundamentally different from 100,000 updates delivered to millions of recipients. The latter can generate billions of network deliveries per second.

These constraints must remain visible throughout implementation.

## 3. Design the system as two planes

Control plane

Infrequent, consistency-critical

Schema definitions

Validation metadata

Version publication

Schema compatibility

Client readiness

Data plane

High-frequency, latency-critical

WebSocket messages

Document reads/writes

Subscriptions

Binary fan-out

Backpressure

Immutable schema IDs connect both planes

A schema update should never force the entire high-throughput data path to pause.

Conversely, the data plane must never process a message using a schema version that is missing, unverified, or ambiguous.

### System topology

Moqui metadata and document definitions

Authoritative field and relationship information

Schema publisher — Rust

Generate .fbs · Compile .bfbs · Validate · Publish

Immutable schema registry

Schema IDs, versions, fingerprints, constraints

Rust Gateway 1

Schema cache, routing, WebSocket fan-out

Rust Gateway 2

Schema cache, routing, WebSocket fan-out

Rust Gateway N

Schema cache, routing, WebSocket fan-out

Binary WebSockets

Browser clients

TypeScript reflection · Schema cache · In-place document views

Do not introduce a separate distributed schema-registry service initially unless the existing database cannot provide atomic schema publication. A versioned table and process-local immutable caches are sufficient to begin.

## 4. Repository and module design

I would use a Rust workspace with one browser package.

```

moqui-wire/
├── Cargo.toml
├── crates/
│   ├── wire/
│   │   ├── src/
│   │   │   ├── envelope.rs
│   │   │   ├── reader.rs
│   │   │   ├── writer.rs
│   │   │   └── errors.rs
│   │   └── build.rs
│   │
│   ├── schema/
│   │   └── src/
│   │       ├── registry.rs
│   │       ├── compile.rs
│   │       ├── reflect.rs
│   │       ├── validate.rs
│   │       └── compatibility.rs
│   │
│   ├── documents/
│   │   └── src/
│   │       ├── snapshot.rs
│   │       ├── patch.rs
│   │       ├── revision.rs
│   │       └── moqui.rs
│   │
│   ├── gateway/
│   │   └── src/
│   │       ├── connection.rs
│   │       ├── websocket.rs
│   │       ├── rest.rs
│   │       ├── subscriptions.rs
│   │       ├── fanout.rs
│   │       └── backpressure.rs
│   │
│   └── schema-publisher/
│       └── src/main.rs
│
├── browser/
│   └── src/
│       ├── schema-registry.ts
│       ├── reflected-reader.ts
│       ├── reflected-writer.ts
│       ├── validate.ts
│       ├── websocket.ts
│       └── document-store.ts
│
├── schemas/
│   ├── envelope.fbs
│   └── patches.fbs
│
├── tests/
│   ├── fixtures/
│   ├── cross-language/
│   └── evolution/
│
└── benches/
    ├── serialization/
    ├── reflection/
    └── fanout/

```

These should be logical modules, not independently deployed services by default.

The essential interfaces should be small:

```

pub struct SchemaId(pub u64);

pub struct DocumentRevision(pub u64);

pub trait SchemaRegistry {
    fn get(&self, id: SchemaId)
        -> Option<&RuntimeSchema>;
}

pub trait DocumentReader {
    fn get_field(&self, field_id: u16)
        -> Result<FieldView<'_>, ReadError>;
}

pub trait DocumentWriter {
    fn encode(
        &self,
        schema: &RuntimeSchema,
        value: &DocumentInput,
    ) -> Result<bytes::Bytes, WriteError>;
}

```

These are proposed interfaces, not existing FlatBuffers APIs. In practice, `DocumentReader` may need to be implemented on a table-view type rather than a whole document, and `SchemaRegistry` may use owned/shared handles to accommodate cache reloads.

Keep the application unaware of raw FlatBuffers offsets. Only the wire and reflection modules should manipulate them.

## 5. Schema representation: actual `.bfbs`, not generic document values

This is the most consequential decision.

Use dynamically generated, specialized FlatBuffers tables for Moqui documents.

Do not encode every field through a generic structure such as:

```
Field
  name
  type
  value
```

That would reintroduce per-field metadata, type dispatch and additional pointer traversal.

Instead, when Moqui defines:

```
Order
  orderId: String
  total: Decimal
  items: List<OrderItem>
```

Generate a genuine FlatBuffers schema:

```

namespace Moqui;

table Decimal {
  unscaled:long;
  scale:int;
}

table OrderItem {
  productId:string (required);
  quantity:long;
  amount:Decimal;
}

table Order {
  orderId:string (required);
  total:Decimal;
  items:[OrderItem];
}

root_type Order;

```

This gives every document a real binary layout.

### Type mappings

| Moqui semantic type         | FlatBuffers representation                              |
| --------------------------- | ------------------------------------------------------- |
| String / ID                 | `string`                                                |
| Integer                     | `int`, `long`, etc.                                     |
| Boolean                     | `bool`                                                  |
| Floating point              | `float` / `double`                                      |
| Exact decimal               | `long` with scale, or defined Decimal128 representation |
| Date                        | Signed integer days since epoch                         |
| Timestamp                   | Signed integer microseconds since Unix epoch            |
| Enum                        | FlatBuffers `enum`, if definitions are stable           |
| Nested object               | `table`                                                 |
| List of objects             | Vector of tables                                        |
| Optional scalar             | Optional scalar supported by chosen FlatBuffers version |
| Polymorphic object          | `union`                                                 |
| Arbitrary binary data       | `[ubyte]`                                               |
| Reference to another entity | Explicit typed reference field/table                    |

Dates, decimals and references need application-level semantic definitions, because their meaning is not fully captured by primitive FlatBuffers types.

For money, never default to `float` or `double` when exact decimal arithmetic is required.

### Moqui integration

Moqui stores Data Document field paths and relationship aliases. The source definitions can also contain expressions.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



I would map them as follows:

- A scalar field path maps to a schema field with an assigned persistent ID.
- A one-to-one relationship maps to a nested table.
- A one-to-many relationship maps to a vector of nested tables.
- Aliases affect display names, not the persistent field's identity.
- Computed expressions require an explicitly declared result type.
- Document fields unsupported by the runtime type system are rejected during schema publication.

Do not infer permanent type identities from display names or relationship aliases. Those may change while the field's semantic identity stays the same.

### Stable field IDs

Every schema publisher must preserve numeric field IDs.

FlatBuffers permits explicit field IDs and uses those IDs to support controlled reordering. Its schema-evolution documentation specifically recommends explicit IDs when concurrent schema changes are a concern.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://flatbuffers.dev\&sz=32)

FlatBuffers Docs



Maintain an ID registry so a deleted field's ID can never be recycled.

## 6. Schema publication and live evolution

### Authoritative registry model

Use two records conceptually:

```

CREATE TABLE document_schema (
    schema_id BIGINT PRIMARY KEY,
    document_type TEXT NOT NULL,
    version INTEGER NOT NULL,
    fingerprint BYTEA NOT NULL UNIQUE,
    bfbs BYTEA NOT NULL,
    constraints BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(document_type, version)
);

CREATE TABLE document_schema_policy (
    document_type TEXT PRIMARY KEY,
    active_read_schema BIGINT NOT NULL,
    active_write_schema BIGINT NOT NULL,
    policy_generation BIGINT NOT NULL
);

```

Use separate immutable compatibility-policy records or a versioned policy table for accepted historical versions, migrations and rollout state. The two tables above are only the core schema storage.

Store the `.fbs` source as well, for diagnostics, auditability and compatibility checks.

### Schema publication algorithm

1. Capture one consistent Moqui metadata snapshot.

   Resolve the document's fields, relationships, types, expressions and constraints before starting compilation.
2. Assign stable field identities.

   Preserve IDs from the previous version. Allocate new IDs transactionally for new fields.
3. Generate `.fbs` and compile with `flatc`.

   Produce a `.bfbs` descriptor using the pinned FlatBuffers compiler version.
4. Run compatibility checks.

   Use `flatc --conform` against the previous schema. Reject incompatible changes from the same compatibility lineage.
5. Validate the descriptor and application constraints.

   Check supported features, recursion limits, field counts, nesting, schema size and domain types.
6. Publish atomically.

   Persist the immutable `.bfbs`, associated constraints and fingerprint. Then atomically advance the active-schema policy.
7. Notify gateway caches.

   Gateways fetch and install the new descriptor. They do not regenerate code or restart.
8. Notify only relevant browsers.

   Subscribers load the descriptor, prepare the runtime view, then acknowledge readiness.

The compiler supports `.bfbs` emission and the `--conform` compatibility check directly.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://github.com\&sz=32)

GitHub



For incompatible changes, create a new schema lineage or explicit migration, instead of forcing the new schema through `--conform`.

### Schema version transitions

A safe live deployment looks like this:

Example: Order schema v12 → v13

Before publication

Read v12

Write v12

Transition period

Read v12, v13

Write v12, v13 with policy

After migration and compatibility window

Preferred read/write v13

v12 retained only while historical data or clients require it

For incompatible write versions, do not silently reinterpret the bytes. Reject the write with a version-conflict error or run an explicitly defined migration.

### Avoid per-connection schema maps

Ten million connections do not need ten million copies of schema definitions.

Use:

- One immutable descriptor cache per gateway process.
- A small active-schema generation marker per connection.
- Lazy retrieval of actual schema descriptors.
- Compact schema identifiers in ordinary data messages.

If a schema update affects 100,000 users, one gateway should perform one registry fetch, not 100,000 independent fetches.

## 7. Implementing the Rust reflective reader

Use `flatbuffers-reflection::SafeBuffer` as the initial baseline.

The current implementation validates the bytes against a supplied runtime schema and provides a safe root-table view.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://docs.rs\&sz=32)

Rust

+1



Conceptually:

```

use flatbuffers_reflection::SafeBuffer;

fn validate_and_open<'a>(
    schema: &'a RuntimeSchema,
    bytes: &'a [u8],
) -> Result<ValidatedDocument<'a>, Error> {
    // 1. Verify FlatBuffers structure using
    //    schema.bfbs_descriptor().
    //
    // 2. Check exact Moqui field types and
    //    constraints using schema.constraints().
    //
    // 3. Return a view borrowing the bytes.
    //
    // No full document materialization.
    todo!()
}

```

This is architectural pseudocode; `RuntimeSchema` and `ValidatedDocument` are application types.

### Reader responsibilities

Implement:

| Function                 | Behavior                                          |
| ------------------------ | ------------------------------------------------- |
| `open(schema, bytes)`    | Verify a binary buffer                            |
| `get(field_id)`          | Read the exact declared field                     |
| `get_table(field_id)`    | Return a borrowed nested table view               |
| `get_vector(field_id)`   | Return a borrowed vector view                     |
| `get_string(field_id)`   | Return a borrowed UTF-8 string slice              |
| `get_decimal(field_id)`  | Read exact decimal representation                 |
| `is_present(field_id)`   | Distinguish omitted fields where permitted        |
| `validate_constraints()` | Enforce Moqui-specific structural and value rules |

Avoid coercing values through generic integer or floating-point accessor methods.

The official reflection API contains coercing `get_any_*` functions and exact-type variants. Your validator should prefer the latter.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://docs.rs\&sz=32)

Rust



### Reflection cache

The first time a gateway loads a schema, transform its reflection metadata into a compact cached descriptor.

For example:

```
CompiledSchema
  root_type
  fields_by_id
  fields_by_name
  field_types
  vtable_offsets
  nested_type_indices
  constraints
```

Do this once per schema version, not once per document.

The cache should be immutable and reference-counted, so live schema updates do not invalidate readers using older versions.

### Important performance detail

The current Rust `SafeBuffer` implementation constructs a `HashMap` when verifying a buffer, tracking mappings from buffer locations to reflected object indices.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://docs.rs\&sz=32)

source



That means zero-copy is not equivalent to zero allocation.

The proposed plan should benchmark this cost early. It may be acceptable for inbound writes, where validation is mandatory, but expensive for very high-frequency small messages.

Do not bypass verification merely to hit a throughput number. If necessary, optimize or replace the verification implementation while preserving its safety properties.

## 8. Implementing the dynamic browser reader and writer

The browser package needs two distinct capabilities.

### Reader

Generate TypeScript bindings for `reflection.fbs` at build time. Those bindings decode the `.bfbs` descriptor, not the dynamic document itself.

The browser reader then performs these operations:

```
const schema = await registry.get(schemaId);
const document = runtime.open(schema, arrayBuffer);

// Direct views over the received buffer
const name = document.getString("name");
const items = document.getVector("items");

const quantity = items.getTable(0).getInt64("quantity");
```

This is the desired custom API.

Do not generate a JavaScript object graph during `open()`.

The runtime stores the buffer, table location and schema descriptor. A nested field access creates a lightweight view.

### Writer

A dynamic writer is more involved because FlatBuffers construction involves offsets and reverse-order buffer building.

I would implement a schema-driven recursive encoder:

```

encode(table_type, input):
    validate input against runtime schema
    encode child strings and nested tables
    encode vectors of children
    start FlatBuffers table
    write scalar fields
    write child offsets
    finish table
    return table offset

```

The official FlatBuffers builder should perform binary layout construction. The custom layer decides which field operations to invoke based on `.bfbs`.

### Limit the supported features deliberately

For the first production version, support:

- Tables, including nested tables and recursive references.
- UTF-8 strings.
- Boolean, signed/unsigned integers, floats and doubles.
- Optional values and explicitly defined null semantics.
- Vectors of scalars, strings and tables.
- Explicit decimal and timestamp conventions.
- Enumerations.
- Controlled unions for known polymorphic relationships.

Postpone rarely needed features such as arbitrary unions, specialized fixed-layout structs, arrays and 64-bit vector layouts until the real Moqui metadata requires them.

For large integer values, the browser must use `bigint`, not JavaScript `number`, to avoid precision loss.

### Runtime type validation

The browser and server must apply equivalent structural rules, but the Rust server remains authoritative.

Keep a shared, declarative constraint specification for such things as:

- Required versus optional fields.
- Nullable versus absent values.
- Numeric ranges and precision.
- String lengths and patterns.
- Maximum list lengths.
- Legal enum values.
- Allowed nested relationships.

Authorization, database integrity and transactional constraints belong on the server.

Do not try to treat a JavaScript runtime descriptor as a new compile-time TypeScript interface. The browser gets runtime-validated types, not automatically generated static types.

## 9. WebSocket protocol

Keep the WebSocket protocol much smaller and more stable than the document schemas.

Use one binary WebSocket message per application frame, with explicit batching for multiple logical operations.

### Recommended envelope

Use a fixed FlatBuffers envelope for control messages, but avoid copying arbitrary document payloads into a second FlatBuffer byte vector merely to wrap them.

For the high-throughput data path, a compact fixed header followed by a complete root FlatBuffer is simpler and avoids that extra payload-copy opportunity.

Application binary message

Kind

1 B

Flags

1 B

Schema

4 B

Revision

8 B

FlatBuffers root buffer

Variable length

Conceptual 14-byte data header. It can be reduced for message classes that do not need revisions. A full request identifier, document identifier or sequence must be included wherever the operation semantics require it.

The binary payload remains a normal FlatBuffer that can be sliced or viewed without creating a new copy.

Use a stable per-deployment numeric schema ID. Registry IDs must never be reused for different definitions. If multiple independent registries can issue IDs, add a registry namespace or negotiate per-connection IDs.

### Message types

| Message             | Direction        | Purpose                                       |
| ------------------- | ---------------- | --------------------------------------------- |
| `HELLO`             | Both             | Protocol negotiation and reconnect state      |
| `SCHEMA_AVAILABLE`  | Server → browser | Notify client that a new schema exists        |
| `SCHEMA_GET`        | Browser → server | Request missing immutable descriptor          |
| `SCHEMA_DEFINITION` | Server → browser | Deliver `.bfbs` + constraints                 |
| `SCHEMA_READY`      | Browser → server | Confirm validated schema installation         |
| `SUBSCRIBE`         | Browser → server | Subscribe to documents or topics              |
| `SNAPSHOT`          | Server → browser | Complete current document                     |
| `PATCH`             | Both             | Revisioned document changes                   |
| `ACK`               | Server → browser | Confirm committed write or accepted operation |
| `ERROR`             | Both             | Typed failure response                        |
| `RESYNC_REQUIRED`   | Server → browser | Recover after missed or incompatible updates  |

Keep the control messages in one statically generated schema, where normal FlatBuffers binding generation is reliable.

### Avoid duplicating application headers

For example, do not add a separate payload length when WebSocket framing already provides message boundaries.

Do not include a 64-bit revision in all message types just because snapshots and patches need one.

Use the smallest header that actually enforces the required semantics.

### REST API and format compatibility

REST and internal WebSocket messaging are adapters over the same authoritative Moqui services, not separate implementations of business logic.

- Preserve existing REST routes, methods, authentication, authorization, response shapes and supported media types. Continue accepting and returning JSON and other existing formats; do not force binary envelopes or schema handshakes on REST clients.
- Use HTTP `Content-Type` for request decoding and `Accept` for response negotiation, retaining existing defaults. Return `415` for unsupported request media types and `406` when no acceptable response format is available.
- Allow opt-in FlatBuffers over HTTP for internal callers using an explicitly documented media type and schema identifier. Binary HTTP requests require the same verification and immutable-schema rules as binary WebSocket messages.
- Decode each format into the shared document-operation model and apply the same semantic validation, authorization, transactional rules, revision checks and idempotency behavior. Preserve existing REST concurrency contracts; map new revision-aware operations explicitly to HTTP preconditions and conflict responses.
- Keep JSON conversion at REST/format boundaries, not in the internal binary fan-out path. JSON parsing and serialization allocate; zero-copy requirements apply to binary structured reads, not every supported API format.
- Define lossless JSON mappings for exact decimals, 64-bit integers, dates, timestamps, binary values, and absent versus null fields for new document APIs, without changing existing REST contracts.
- REST writes and internal message writes must publish through the same committed-update path so subscriptions observe changes regardless of entry transport. REST responses use HTTP status codes and the API's error representation, not WebSocket control frames.

## 10. Snapshots versus patches

For complicated nested documents, this is as important as choosing FlatBuffers.

### Snapshot

A snapshot is a complete document encoded under one exact immutable `.bfbs` schema version.

Send snapshots for initial subscriptions, reconnect recovery and major structural changes.

### Patch

A patch describes an operation against a known revision.

For example:

```
Document ID: Order-123
Base revision: 101
New revision: 102

Operations:
  SET field 7 = "Approved"
  SET items[2].quantity = 5
  REMOVE items[3]
```

The patch format should be a separate stable schema.

Use numeric field IDs and an explicit path representation rather than sending arbitrary string paths on every update.

Define an immutable patch model supporting:

- Set value.
- Clear optional value.
- Replace subtree.
- Insert/remove list element.
- Replace list.
- Delete document.

For lists whose items represent entities, prefer stable item IDs over positional indices. Otherwise, concurrent inserts and removes can make patches target the wrong element.

Critical design choice: A snapshot has a specialized FlatBuffers layout; a patch may need a generic typed-value representation because it targets arbitrary fields.

That is acceptable. Small generic patches are often much cheaper than rebuilding and transmitting a large specialized snapshot.

### Write conflicts and idempotency

Every modifying operation should include:

```
client_request_id
document_id
base_revision
schema_id
patch
```

On the server:

1. Verify the binary structure and schema.
2. Authorize the requested mutation.
3. Check the base revision and any idempotency key.
4. Apply the change transactionally through the authoritative document service.
5. Publish a new committed revision.
6. Acknowledge using the original request ID.

On reconnect, the client sends the last committed document revision it processed. The server resumes from retained history if available, otherwise sends a snapshot.

Do not treat WebSocket delivery as exactly-once database execution.

## 11. Rust fan-out architecture

For 10 million clients, the networking architecture needs to minimize per-connection allocations.

### Use a sharded ownership model

Each gateway manages a subset of connections.

Each connection has one owning task or event-loop actor that exclusively handles its socket writes. Other tasks send references to immutable messages to that owner.

Avoid multiple tasks writing directly to the same WebSocket.

One validated and encoded update

Shared immutable `Bytes`

One payload allocation for many recipients

Subscribers A

Shared byte references

Subscribers B

Shared byte references

Subscribers C

Shared byte references

Per-connection WebSocket/TLS writes

Still requires independent transport processing

Use `bytes::Bytes` for shareable immutable payloads.

Be aware that merely calling `.clone()` on `Bytes` avoids cloning the payload data but still increments reference counts and creates per-recipient metadata.

For enormous fan-out, batching references by gateway shard can reduce contention.

### Avoid per-connection message channels when idle

A conventional Tokio task with its own large channel and read/write buffers may be reasonable at 10,000 connections but expensive at 10 million.

Start with Axum and Tokio to establish correctness and baseline performance.

If profiling shows task/channel memory is excessive, replace the internal gateway transport implementation with a more compact readiness/event-loop design. Keep the schema and document layers unchanged.

### Subscriptions

Store subscriptions in two directions:

```
topic -> sharded connection IDs
connection -> compact topic IDs
```

The first supports fan-out; the second supports efficient disconnection cleanup.

Use numeric subscription IDs and compressed/segmented membership representations where they outperform hash sets.

Do not scan 10 million connections for every update.

### Backpressure

Apply hard limits to:

- Queued bytes per connection.
- Number of pending updates.
- Maximum message size.
- Maximum subscriptions.
- Maximum in-flight writes.
- Maximum age of undelivered state.

When a connection is slow, coalesce replaceable state updates. If a lossless event stream is required, use a durable cursor and force resynchronization rather than silently dropping events.

Do not let slow readers retain gigabytes of old shared buffers.

## 12. Realistic capacity model

The dominant scaling variable is not simply messages received per second.

It is:

\\[ \text{Outbound deliveries/sec} = \text{updates/sec} \times \text{average recipients/update} \\]

Assuming 100,000 updates/sec and an average 512-byte application payload:

| Average recipients per update | Outbound deliveries/s | Payload egress |
| ----------------------------- | --------------------- | -------------- |
| 1                             | 100K                  | 0.41 Gbps      |
| 10                            | 1M                    | 4.10 Gbps      |
| 100                           | 10M                   | 40.96 Gbps     |
| 1,000                         | 100M                  | 409.6 Gbps     |
| 10,000                        | 1B                    | 4.10 Tbps      |
| 1,000,000                     | 100B                  | 409.6 Tbps     |

These are payload-only figures; actual egress includes WebSocket/TCP/IP/TLS overhead, retransmission and protocol metadata.

### The 10-million-recipient extreme

If every one of the 100,000 updates/sec were delivered to 10 million clients, that would be one trillion deliveries per second.

At 512 bytes per delivery, payload traffic alone would be approximately 4.096 petabits per second.

That is not a practical direct fan-out design for a conventional application deployment.

For large subscriber groups, you must coalesce state and deliver periodic snapshots or batches rather than mechanically forwarding every intermediate mutation.

### Baseline gateway capacity

I would begin capacity tests with the following planning scenarios, not promises:

| Cluster scenario | Connections/node | Node count |
| ---------------- | ---------------- | ---------- |
| Conservative     | 25,000           | 400        |
| Intermediate     | 50,000           | 200        |
| Aggressive       | 100,000          | 100        |

The appropriate density depends on TLS, kernel memory, activity patterns and network load. These are starting points for experiments, not empirically proven capacities.

For example, 100,000 connections at 16 KiB of total attributable memory each use approximately 1.53 GiB before additional application overhead.

Actual Linux socket, TLS and process memory must be measured under realistic traffic and slow-client conditions.

## 13. Performance targets and acceptance gates

These should be treated as initial engineering targets, not FlatBuffers guarantees. Record observed baselines and revise them with production-like document distributions.

### A. Serialization and reflection

Run these benchmarks on a specified, pinned server CPU and recent Chromium browser. Use documents of 1 KiB, 10 KiB and 100 KiB, including deeply nested types and mixed scalar/vector fields.

| Metric                                      | Initial target             | What it measures                          |
| ------------------------------------------- | -------------------------- | ----------------------------------------- |
| Server reflective read of one cached scalar | p99 under 1 µs             | Access after verification                 |
| Browser reflective scalar access            | p99 under 2 µs             | Access after schema lookup and validation |
| Rust 1 KiB encode                           | p99 under 25 µs            | Runtime-schema-driven builder             |
| Rust 1 KiB verification                     | p99 under 25 µs            | Full structural verification              |
| Browser 1 KiB dynamic encode                | p99 under 50 µs            | Schema-driven builder                     |
| Full-document materialization on receive    | 0 by default               | Must remain a lazy view                   |
| Inbound validation                          | 100% of untrusted messages | No unchecked access path                  |
| Per-recipient payload re-encoding           | 0 for identical payloads   | Shared fan-out                            |

These targets are intentionally ambitious, and the reflection/verification paths may require optimization to achieve them.

A more useful result than a single microbenchmark is the performance multiplier against JSON for the same workload, including actual field access and validation.

I would require the chosen FlatBuffers implementation to beat a well-optimized JSON baseline in either end-to-end CPU usage or allocation rate on representative traffic. If it does not, investigate before investing further in custom reflection code.

### B. Server ingest

| Metric                               | Target                                           |
| ------------------------------------ | ------------------------------------------------ |
| Aggregate inbound traffic            | 100,000 document operations/sec                  |
| Binary validation errors             | Handled without crashes or process termination   |
| Ingest p99 latency                   | Under 10 ms through routing and validation       |
| End-to-end committed write p99       | Set separately after benchmarking Moqui/database |
| Queue growth under steady-state load | None                                             |
| Duplicate request behavior           | Idempotent where operations require it           |
| Invalid schema writes                | Deterministic rejection                          |

The 10 ms target excludes database commit time. You cannot responsibly assign a database p99 target before profiling actual Moqui transactions, indexing and business rules.

### C. Fan-out

| Metric                                                | Initial target                                           |
| ----------------------------------------------------- | -------------------------------------------------------- |
| Encode operations per logical update                  | 1 per required payload variant                           |
| Per-recipient full payload copies in application code | 0                                                        |
| Per-connection unbounded queues                       | 0                                                        |
| Gateway memory budget                                 | Configured and measured                                  |
| Normal internal dispatch p99                          | Under 5 ms at tested load                                |
| Subscription routing cost                             | Proportional to relevant recipients, not all connections |
| Slow-client handling                                  | Bounded queue, coalescing or resync                      |
| Recovery after gateway restart                        | Reconnect and state resynchronization                    |

No single per-node outbound-message target should be committed until fan-out size and message-size distributions are measured.

### D. Schema synchronization

| Metric                                         | Initial target                           |
| ---------------------------------------------- | ---------------------------------------- |
| New schema publication                         | p99 under 2 seconds for ordinary schemas |
| Cache installation after registry notification | p99 under 1 second per healthy gateway   |
| Descriptor bytes sent per normal data message  | 0                                        |
| Rebuilding client application for new type     | Never                                    |
| Restarting Rust gateways for new type          | Never                                    |
| Schema fingerprint mismatch accepted           | Never                                    |
| Data processed against unknown schema          | Never                                    |
| Incompatible write silently accepted           | Never                                    |

These are healthy-system targets and exclude offline clients. For 10 million connections, total rollout time is determined by subscriptions, network congestion and connected-client responsiveness.

### E. Memory

Set measured process budgets for:

- Connection metadata.
- Tokio tasks or gateway actors.
- WebSocket parsing state.
- TLS session state.
- Kernel TCP buffers.
- Subscription indexes.
- Pending writes and queued updates.
- Shared FlatBuffers.
- Schema caches.

I would target no significant steady-state heap growth during a multi-hour 100K/s load test.

An idle connection memory target of 16–32 KiB, including amortized server and kernel costs, is worth testing initially. If the stack cannot reach that range, do not assume custom serialization will fix it.

## 14. Benchmark workloads

Don't test only a single flat `Order` object.

Use at least these five representative document cases:

| Fixture         | Characteristics                                   | Main risk                    |
| --------------- | ------------------------------------------------- | ---------------------------- |
| Small update    | 3–10 changed scalar fields                        | Per-message overhead         |
| Nested order    | 20–100 fields, 10–50 line items                   | Pointer traversal            |
| Large document  | 100 KiB+, multiple nested collections             | Size, allocation, throughput |
| Sparse document | Hundreds of possible fields, few present          | Schema and default handling  |
| Dynamic schema  | New fields and nested types during active traffic | Version synchronization      |

For every fixture, benchmark:

1. Rust construction.
2. Rust verification.
3. Rust reflective field access.
4. Browser receipt and validation.
5. Browser reflective field access.
6. Browser dynamic construction.
7. Round-trip equality and compatibility.
8. Memory allocated per stage.
9. Delivery across many subscribers.

Benchmarks must include both successful and malformed payloads.

Add REST compatibility tests for existing JSON and other supported formats, content negotiation, authentication/authorization, errors and revision conflicts. Cross-transport tests must verify that REST writes reach subscribers and that REST reads observe committed internal-message writes. Test lossless JSON/binary round trips for the new document APIs.

Also test a heavy-tailed message-size distribution rather than assuming every message is exactly 1 KiB.

## 15. Security and correctness requirements

These are release blockers, not optional later work.

### Schema publication

Only trusted operators or authorized Moqui services may publish new definitions.

Run `flatc` out of the data-plane process in a restricted worker with filesystem, CPU, memory, execution-time and input-size limits.

Require the compiler and runtimes to recognize every `.bfbs` advanced feature flag they encounter. The FlatBuffers intermediate representation explicitly requires unknown advanced features to be rejected.&#x20;

[image](https://www.google.com/s2/favicons?domain=https://flatbuffers.dev\&sz=32)

FlatBuffers Docs



### Message verification

Enforce:

- Maximum message size.
- Maximum nesting depth.
- Maximum table count.
- Maximum vector length.
- Maximum string length.
- Valid offsets and alignment.
- UTF-8 requirements.
- Exact semantic field types.
- Supported schema identifier and fingerprint.
- Schema-dependent required fields.
- Resource budgets for recursive validation.

Avoid `unsafe` reflective reads on untrusted bytes until verification has completed successfully.

### Browser/client trust

A client claiming `SCHEMA_READY` is not evidence that it can be trusted.

The server still validates all writes, including schema versions, authorization and document revisions.

### Fuzz testing

Use `cargo-fuzz` or equivalent fuzzing against the Rust reflective decoder and verifier.

Fuzz malformed buffers, malformed `.bfbs` files, deeply recursive descriptors, vector lengths, invalid unions and schema/version mismatches.

Cross-language property tests should ensure that every valid document encoded in Rust can be read by TypeScript and vice versa.

## 16. Explicit alternatives and decision rationale

| Architecture                                      | Why it might win                                           | Why I'm not choosing it                                         |
| ------------------------------------------------- | ---------------------------------------------------------- | --------------------------------------------------------------- |
| FlatBuffers `.bfbs` reflection                    | Specialized zero-copy layouts and runtime-defined schemas  | Chosen, despite implementation work                             |
| FlatBuffers generic typed values                  | Simpler runtime evolution                                  | Additional type tags, generic lookup and larger messages        |
| FlexBuffers + schema registry                     | Simplest schema-less direct-access solution                | Does not specialize the binary layout for each document         |
| Cap’n Proto dynamic schemas                       | Excellent zero-copy format and reflection concepts         | More complex Rust/browser runtime integration                   |
| Protobuf dynamic descriptors                      | Very mature dynamic-schema tooling                         | Ordinary decoding typically constructs values                   |
| Apache Avro                                       | Schema evolution is a core design feature                  | Not a general zero-copy object-traversal format                 |
| Apache Arrow IPC                                  | Excellent columnar and batched zero-copy processing        | Poor fit for large numbers of small document mutations          |
| Custom binary protocol                            | Maximum control and possibly less wire overhead            | Too much custom implementation for nested runtime-defined types |
| Rust/WASM reflection shared by server and browser | One shared implementation of validation and interpretation | Copies into WASM memory and adds deployment complexity          |

### Why FlatBuffers wins here

Its primary advantage is the combination of specialized per-document binary layouts and runtime-readable schema descriptors.

FlexBuffers has the flexibility but not the specialized layout.

Protobuf and Avro have runtime schema facilities but normally require more deserialization work.

Cap’n Proto is technically appealing, but the end-to-end Rust/browser implementation is not as straightforward.

FlatBuffers is therefore a defensible architecture—not necessarily the shortest implementation, but a good match for your stated absolute requirements.

The key risk remains the completeness and performance of the reflection stack, especially dynamic writing and browser verification. Resolve that before committing to the full deployment.

## 17. Implementation roadmap

I would organize the work around demonstrable milestones rather than integrating everything at once.

Implementation checklist

Estimated single-developer engineering effort; external integration and testing can extend it.

0/6 phases

Phase 0 — Feasibility spike1–2 weeks

- Pin compatible FlatBuffers, Rust and TypeScript versions
- Compile a nested Moqui schema to .bfbs at runtime
- Verify and read a document reflectively in Rust
- Read the same buffer reflectively in TypeScript
- Create a compatible document from both languages
- Benchmark the safe verification and dynamic builder paths

Exit gate: New runtime-created nested type is encoded and read in both languages without recompilation.

Phase 1 — Schema compiler and registry2–3 weeks

Phase 2 — Production reflection runtime3–5 weeks

Phase 3 — Document synchronization and REST adapters2–3 weeks

- Share document services between REST and internal messaging
- Preserve existing JSON and other format contracts
- Verify content negotiation and cross-transport consistency

Phase 4 — Gateway and fan-out3–5 weeks

Phase 5 — Scale and production hardening3–6 weeks

&#x20;Copy implementation checklist

The total is roughly 14–24 engineering weeks for a prototype through production hardening, assuming one experienced engineer, existing Moqui integration access and no major surprises. A smaller team can parallelize later work, but the feasibility spike and shared format semantics are sequential dependencies.

A genuinely production-ready reflective protocol is not realistically a few hundred lines of handwritten code. The goal should instead be to keep all custom complexity inside two reusable components: a schema-driven reader and a schema-driven writer.

## 18. What not to do

| Avoid                                         | Because                                           | Do instead                         |
| --------------------------------------------- | ------------------------------------------------- | ---------------------------------- |
| Generate and hot-load new Rust binaries       | Complex deployment, unsafe operational boundary   | Load immutable `.bfbs` descriptors |
| Generate new browser JS per schema            | Code execution and cache complexity               | Use runtime reflection             |
| Embed schemas in every message                | Repeated bandwidth and allocations                | Send schema IDs                    |
| Convert internal binary fan-out to JSON        | Destroys zero-copy benefits                       | Convert only at format boundaries  |
| Decode entire document graphs eagerly         | Extra allocation and CPU                          | Lazy nested field access           |
| Bypass verification                           | Malformed offsets can become safety hazards       | Verify before access               |
| Rebuild message for every subscriber          | Scales encoder CPU with fan-out                   | Share immutable encoded payloads   |
| Reuse field IDs after deletion                | Old messages become ambiguous                     | Reserve old IDs permanently        |
| Treat old schema writes as automatically safe | Can overwrite newer state                         | Revision checks and migrations     |
| Broadcast every schema to 10M browsers        | Unnecessary network traffic                       | Target affected subscribers        |
| Allow unlimited outbound queues               | Slow clients can exhaust memory                   | Bounded queues and resync          |
| Start by optimizing the serializer alone      | Networking, fan-out and Moqui writes may dominate | Benchmark end-to-end               |

## 19. Critical go/no-go decisions

Before building the full registry and gateways, I would insist on four proof points.

Proof 1: A new type appears at runtime

A Rust process and an already-running browser successfully read a newly introduced nested document using a freshly published `.bfbs`, without recompilation.

Proof 2: Both sides can write

Rust and TypeScript independently encode a complex nested document that the other side can verify and read using the identical schema.

Proof 3: Schema versions coexist

A v12 browser can continue its session while v13 is published. Compatible changes work; incompatible changes are rejected or migrated deliberately.

Proof 4: Real throughput advantage

FlatBuffers beats your JSON or conventional binary baseline on the metrics that matter: allocation, CPU, browser access latency, and per-update fan-out cost.

If Proof 2 requires an excessive amount of custom writer implementation, reconsider the design before continuing. A stable generic FlatBuffers schema with external runtime descriptors is the fallback; it preserves strict typing and direct buffer reads while sacrificing specialized per-type layouts.

## 20. Final architectural decision

I would approve this architecture conditionally:

Selected production architecture

## FlatBuffers + `.bfbs` + Rust + TypeScript

Schema control plane

Moqui metadata → Schema publisher → Immutable `.bfbs` registry → Gateway/browser caches

Document data plane

Internal clients/browser runtime ↔ Binary WebSocket or opt-in binary HTTP ↔ Rust gateway ↔ Moqui services

REST API plane

REST clients ↔ HTTP (JSON and other supported formats) ↔ REST adapters ↔ Same Moqui services

Performance strategy

Reflective, verified zero-copy reads · Cached descriptors · Revisioned patches · Shared immutable fan-out buffers

The most important decision is to prove the dynamic writer and safe reflective reader early. Those are the two areas where FlatBuffers provides the binary foundations but not a complete plug-and-play Rust/TypeScript runtime-document solution.

Once those are demonstrated, the remaining work becomes conventional systems engineering: schema publication, validation, synchronization, routing, backpressure and performance testing.

### Primary technical references

- [FlatBuffers binary schema and reflection representation](https://flatbuffers.dev/intermediate_representation/)
- [FlatBuffers Rust reflection documentation](https://flatbuffers.dev/languages/rust/)
- [Rust reflection APIs](https://docs.rs/flatbuffers-reflection/latest/flatbuffers_reflection/)
- [FlatBuffers schema evolution and compatibility](https://flatbuffers.dev/evolution/)
- [Official reflection meta-schema](https://github.com/google/flatbuffers/blob/master/reflection/reflection.fbs)
- [WebSocket protocol specification](https://www.rfc-editor.org/rfc/rfc6455)

Implementation priority: Get one complicated, dynamically defined Moqui document working bidirectionally under runtime `.bfbs` reflection before building any of the 10-million-connection infrastructure. That is the highest-risk technical dependency, and validating it first prevents spending weeks optimizing a gateway around an unproven document format.