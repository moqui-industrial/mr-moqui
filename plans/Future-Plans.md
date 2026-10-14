# Future plans

Digest of the unbuilt proposals in this directory. None has code in the workspace beyond what is noted. Read the full document before starting any of them.

## Messaging — runtime-reflective FlatBuffers ([Messaging.md](Messaging.md), 1286 lines)
Binary request/response and WebSocket document subscriptions using FlatBuffers `.bfbs` schemas generated at runtime from service, screen-transition and Data Document definitions, with a Rust reflective reader, a browser reader/writer on Emscripten WASM, shared message buffers and large-scale fan-out. JSON and other contracts stay. Status: roadmap shows 0 of 6 phases done. Today `moqui-browser` speaks JSON ([`crates/moqui-browser/src/api.rs`](../crates/moqui-browser/src/api.rs)) and cites this plan for the binary forms. Prerequisites in place: Wasm browser build, cached QuickJS service functions, committed-update path. Go/no-go gates are in §19; capacity model in §12–13.

## Monitoring ([Monitoring.md](Monitoring.md), 110 lines)
Technology-independent requirements linking changes, telemetry, source-level diagnostics, business impact and recovery. Implementation delta: OpenTelemetry bridge over existing `tracing` spans, isolated telemetry schema, error fingerprints and profiles, server-JS profiling, regression baselines, incident screens, and an advisory-by-default recovery state machine. Status: requirements only; the artifact execution stack already opens `tracing` spans, which is the intended hook.

## Data load types ([Data.md](Data.md), 15 lines, "not fully verified")
Replace `seed`, `seed-initial`, `install`, `demo` with explicit ownership and sensitivity: seed (code-owned), configuration, secrets (deployment references only), transactions (PII needs fake replacements or exclusion), and a dev export that filters sensitive fields. Reuses the existing loader/writer and component ordering. Status: idea list; needs design before work.

## Cross-platform usability ([Usability.md](Usability.md), 15 lines)
Keep business behavior in `moqui-core`; platform hosts supply processes, storage, networking and lifecycle; desktop (native, webview or browser), embedded/POS, and browser-local WASM (built, see [Wasm](archive/Wasm.md)) with remote or local-only operation. Native-widget rendering would need a backend separate from the HTML/Vue macros. Status: direction only.
