# Wasm

Browser wasm implementation for server side code is tricky, but managable.

We need to compile the wasm with emscripten that will generate the js bindings for performing async, file storage (opfs), and other based on native primitives:

For a Rust implementation of Moqui targeting Emscripten, these are the relevant official documentation pages:

This pattern is based on how sqlite does it and [this thread](https://chatgpt.com/share/6acefc7e-2980-83e8-991e-34142727a944).

| Area                    | Documentation                                                                                                                                                                                          | Why it matters                                   |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------ |
| Rust target             | [Rust ](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-emscripten.html)[`wasm32-unknown-emscripten`](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-emscripten.html) | Compiling Rust to WASM using Emscripten          |
| Emscripten build        | [Emscripten compiler documentation](https://emscripten.org/docs/compiling/index.html)                                                                                                                  | Linking, configuration, and generated JavaScript |
| Multithreading          | [Pthreads support](https://emscripten.org/docs/porting/pthreads.html)                                                                                                                                  | Native-style threads mapped to Web Workers       |
| Async execution         | [Asyncify and JSPI](https://emscripten.org/docs/porting/asyncify.html)                                                                                                                                 | Suspending and resuming WASM execution           |
| Thread coordination     | [Proxying API](https://emscripten.org/docs/api_reference/proxying.h.html)                                                                                                                              | Dispatching work between threads                 |
| Filesystems             | [File System API](https://emscripten.org/docs/api_reference/Filesystem-API.html)                                                                                                                       | Browser filesystem integration                   |
| Native interoperability | [Connecting C/C++ and JavaScript](https://emscripten.org/docs/porting/connecting_cpp_and_javascript/)                                                                                                  | Host function calls and generated bindings       |
| Moqui execution context | [Moqui ExecutionContext API](https://www.moqui.org/javadoc/org/moqui/context/ExecutionContext.html)                                                                                                    | Execution context, facades, and async semantics  |
| Moqui transactions      | [Moqui TransactionFacade API](https://www.moqui.org/javadoc/org/moqui/context/TransactionFacade.html)                                                                                                  | Transaction propagation and lifecycle            |

The key finding from the documentation: Emscripten supports Rust compilation through its dedicated target, but it doesn't automatically turn Tokio spawning, thread-local transaction state, or native database drivers into portable browser implementations.

A Rust Moqui port would therefore need a portable framework core plus explicit host adapters for async scheduling, transactions, database access, and other external services.
