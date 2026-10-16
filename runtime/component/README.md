Put your add-on component directories in this directory to load them when Moqui runs.

## Rust in a component

A component can have a Rust crate, as a Java Moqui component could have a `build.gradle`: a `Cargo.toml` and `src/`
beside its `component.xml`. `./moq build` finds each one here and links it into the moqui binary with the framework,
through a crate it generates in `target/moqui-app`. The binary is still `target/release/moqui`, or `target/debug/moqui`
with `--debug`. Run `./moq build` again after adding or removing a crate.

The crate:

- depends on the framework by path: `moqui-core = { path = "../../../crates/moqui-core", default-features = false }`
  and `moqui-lib = { path = "../../../crates/moqui-lib" }`;
- exports `pub fn tool_factories() -> Vec<Box<dyn moqui_core::context::ToolFactory>>`. The binary registers it under the
  component's name, so its tool factories are made only when the component loads.

A `ToolFactory` is Java's `ToolFactory`:

- **`pre_facade_init`** runs at the end of the factory boot. Register `type="rust"` services here:
  `moqui_core::register_service!(ecf, my_function)?` names the function by its Rust path, such as
  `my_component::services::my_function`, which is the `location` of the service definition.
- **`init`** runs once the entity facade is attached, before data loads and the first request.
- **`instance`** gives the object `ec.factory.getTool("Name", ...parameters)` returns. That object is a
  `moqui_core::context::Tool`: a script calls the names in its `methods()`, and its arguments and results convert as
  service parameters do.
- **`destroy`** runs at shutdown, in reverse order.

A conf element `<tools><tool-factory name="Name" init-priority="20" disabled="true"/></tools>` reorders a tool factory
or leaves it out. A `component.xml` with `disabled="true"` (it can be a property: `${my_disabled ?: 'true'}`) is
skipped here unless a conf `component-list` names it.

`runtime/base-component/example` has an example of each part, with specs (`./moq test example -D
example_disabled=false`). Its `src/` works as a template; its `README.md` has the `Cargo.toml` changes for a crate
here, which is not a member of the repository's workspace.

When a crate brings new dependencies, check their licenses and advisories over the whole graph:
`cargo deny --manifest-path target/moqui-app/Cargo.toml check` (the repository's `deny.toml` applies from the
repository directory).
