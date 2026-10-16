# Writing a native component

A component is still a directory with `component.xml`, optional `MoquiConf.xml`, and service/screen/entity XML. Add a Cargo library for native code. The application depends on that library and links it:

```rust
use my_component as _; // retain the component's linked catalogue entry

fn main() {
    if let Err(error) = moqui_host::dispatch(std::env::args().skip(1)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
```

The application owns its `Cargo.toml` and lockfile. Component and application crates may be in their own workspace; they do not need to be added to the framework workspace. Use path or Git dependencies on the same MR revision. Build the application binary with Cargo; `moq build` continues to build the stock framework binary, which does not link third-party component crates.

## XML-defined native service

In the component library:

```rust
use moqui_core::context::{Error, ExecutionContext};
use moqui_core::native::{NativeComponent, NativeRegistry, Parameters, ServiceInfo, submit_component};
use moqui_lib::error::Error as LibError;

submit_component! {
    NativeComponent { name: "my-component", register: implementations }
}

fn implementations(registry: &mut NativeRegistry) -> Result<(), Error> {
    registry.service_function("my_component::echo", echo)
}

fn echo(_ec: &ExecutionContext, _service: &ServiceInfo<'_>, parameters: &Parameters)
    -> Result<Parameters, LibError>
{
    Ok(parameters.clone())
}
```

`component.xml` must name `my-component`. Its normal runtime discovery is what enables this crate's implementations. Merely linking the crate does not enable its code for another runtime.

In `service/Example.xml`:

```xml
<services>
    <service verb="echo" type="rust" location="my_component::echo">
        <in-parameters><parameter name="message" required="true"/></in-parameters>
        <out-parameters><parameter name="message"/></out-parameters>
    </service>
</services>
```

Call `Example.echo` through `ec.service` as usual. The implementation binding does **not** create this service. XML defines authentication, validation, transaction policy, outputs and ECAs. A native function receives the current EC and can use its factory, entities and tools. Native locations are explicit identifiers, not dynamically loaded Rust symbols.

## Tools

Register a constructor with `registry.tool("my_component::ClientFactory", constructor)`. Implement `native::ToolFactory`, returning `Arc<dyn Tool>` from `get_instance`. Select it in the component's `MoquiConf.xml`:

```xml
<moqui-conf>
    <tools><tool-factory class="my_component::ClientFactory" init-priority="20"/></tools>
</moqui-conf>
```

- `pre_facade_init` runs during core boot, before entity attachment. Lower numeric priorities run first, ties sort by implementation identifier.
- `init` runs after host entity/search setup, before startup data loading. Direct core embedders call `factory.init_tools()` after attaching their facades. Initialization is serialized and idempotent; it must not recursively initialize or shut down tools.
- `factory.shutdown()` stops request/service workers before closing tools; direct embedders should call it after stopping listeners. `destroy` runs in reverse order on shutdown or failed initialization; `post_facade_destroy` runs in reverse order when the factory's facade fields have been released. Cleanup must tolerate partial initialization and must not panic.
- Never retain a strong `Arc<ExecutionContextFactory>` in a tool/provider/renderer: it would form a cycle. Use a weak reference or independent factory-local state.
- Rust: `factory.get_tool_typed::<MyTool>("Client", &parameters)`.
- JS: `ec.getTool("Client", ...parameters)` or `ec.factory.getToolFactory("Client").getInstance(...)`. A missing factory returns `null`. No Java class argument is used.
- Implement `Tool::methods()` and `Tool::call()` to expose explicit JS methods accepting/returning plain `Value` data. Neither reflection nor arbitrary Rust objects cross the boundary. Stateful/session tools may return a fresh instance each time. The instance must reject calls after shutdown if outstanding references remain.

## Other implementation bindings

- `registry.resource(class, constructor)` + `<resource-reference scheme="aws3" class="..."/>`: implement `ResourceProvider`. Constructors can obtain factory-local tools/state. Avoid I/O during construction when it requires a tool's normal initialization. Reads, stat, listing and mutations route through the same provider; unsupported mutations return errors. Providers are responsible for sim/outbound policy.
- `registry.template(class, Arc<dyn TemplateRenderer>)` + `<template-renderer extension="cwiki" class="..."/>`: reuse the existing renderer interface. No service or tool is required. Registry renderer objects are created per factory.
- `registry.runner(class, function)` + `<service-type name="custom" runner-class="..."/>`: XML services with `type="custom"` call this runner. `ServiceInfo` contains the XML service name/type/location/method. Keep runner state in a tool or factory extension. Built-in service types cannot be overridden.

Existing built-in XLSX/image/SFTP/IPP support is unchanged. This API does not load JARs, Java tool classes, Java widget renderers, servlet filters, FOP, Hazelcast or Drools. Port component code and explicitly select its Rust implementation identifiers. Rebuild the application whenever native code/dependencies change.

Executable public-API examples and tests: `crates/moqui-core/tests/native_components.rs`.
