/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! Tool factories: Java's `ToolFactory` for component crates linked into the binary.
//!
//! A component crate exports `pub fn tool_factories() -> Vec<Box<dyn ToolFactory>>`, and the binary registers it under
//! the component's name with [`register_component_tools`] before it boots anything. Each factory boot makes the tool
//! factories of the components it loaded, so a linked crate whose component is absent or `disabled` adds nothing. A
//! tool factory registers its `type="rust"` service implementations with
//! [`ExecutionContextFactory::register_service`] in [`ToolFactory::pre_facade_init`], and gives the object
//! `ec.factory.getTool(name, ...parameters)` returns to a script through [`ToolFactory::instance`].
//!
//! A conf `tools/tool-factory` element with a `name` attribute can disable a tool factory or change its init priority.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use moqui_lib::env;
use moqui_lib::node::MNode;
use moqui_lib::object::{HostObject, Value};

use super::component::ComponentInfo;
use super::error::{Error, ErrorKind};
use super::factory::ExecutionContextFactory;

/// Java's `ToolFactory`. One instance per [`ExecutionContextFactory`]: each boot calls the component's maker again.
///
/// A tool factory that keeps the execution context factory should keep a `Weak`: the factory holds its tool factories,
/// so an `Arc` back would keep both alive.
pub trait ToolFactory: Any + Send + Sync {
    /// The name `ec.factory.getTool` takes, unique across the loaded components.
    fn name(&self) -> &str;

    /// Lower runs first. A conf `tool-factory` element's `init-priority` replaces it. Ties go by name.
    fn init_priority(&self) -> i32 {
        10
    }

    /// At the end of the factory boot: the conf, the components, and the resource and cache facades are up, and there
    /// is no entity facade yet. Register service implementations here, so `moqui lint` (which has no database) and
    /// the first service call both see them.
    ///
    /// # Errors
    ///
    /// An error fails the boot.
    fn pre_facade_init(&self, ecf: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
        let _ = ecf;
        Ok(())
    }

    /// After the entity facade is attached, before the empty database load and the first request. Runs once.
    ///
    /// # Errors
    ///
    /// An error fails the start.
    fn init(&self, ecf: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
        let _ = ecf;
        Ok(())
    }

    /// Java's `getInstance(parameters...)`: the object `ec.factory.getTool(name, ...parameters)` returns.
    ///
    /// # Errors
    ///
    /// The default is an error: a tool factory that only registers services has no instance.
    fn instance(&self, parameters: &[Value]) -> Result<Arc<dyn Tool>, Error> {
        let _ = parameters;
        Err(Error::new(format!("tool {} has no instance", self.name())))
    }

    /// The TypeScript type of the instance, such as `{ greet(name: string): string }`, for `moqui types`. None types it
    /// loosely.
    fn type_declaration(&self) -> Option<&'static str> {
        None
    }

    /// At shutdown, in reverse init order, while the entity facade is still attached. Runs once, and also runs for a
    /// boot that failed after the tool factories were made, so it must not assume `init` ran.
    fn destroy(&self) {}
}

/// A tool object as a script sees it. A script calls a name in [`Tool::methods`] as a method, and reads any other name
/// through [`Tool::property`]. Arguments and results are [`Value`]s, converted as service parameters are; a method can
/// return another tool with [`tool_value`].
///
/// A call runs on the script's thread, inside its execution context and transaction.
pub trait Tool: Any + Send + Sync {
    /// The names a script can call.
    fn methods(&self) -> &[&str];

    /// Run `method`, one of [`Tool::methods`], with the script's arguments.
    ///
    /// # Errors
    ///
    /// An error is thrown in the script.
    fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, Error>;

    /// A property that is not a method, as a Groovy getter would be. None reads as `undefined`.
    ///
    /// # Errors
    ///
    /// An error is thrown in the script.
    fn property(&self, name: &str) -> Result<Option<Value>, Error> {
        let _ = name;
        Ok(None)
    }
}

/// What a component crate exports: `pub fn tool_factories() -> Vec<Box<dyn ToolFactory>>`.
pub type ComponentTools = fn() -> Vec<Box<dyn ToolFactory>>;

static COMPONENT_TOOLS: OnceLock<&'static [(&'static str, ComponentTools)]> = OnceLock::new();

/// Register the linked component crates, as (component name, maker) pairs. The binary calls this once, before it
/// boots a factory.
///
/// # Errors
///
/// Returns an error when the component tools are already registered.
pub fn register_component_tools(makers: &'static [(&'static str, ComponentTools)]) -> Result<(), Error> {
    COMPONENT_TOOLS
        .set(makers)
        .map_err(|_| Error::new("component tools are already registered"))
}

/// The registered component crates, empty when the binary registered none.
pub(super) fn component_tools() -> &'static [(&'static str, ComponentTools)] {
    COMPONENT_TOOLS.get().copied().unwrap_or(&[])
}

/// The value a script gets for `tool`: a [`Value::Object`] the script runtime shows as the tool object.
pub fn tool_value(tool: Arc<dyn Tool>) -> Value {
    Value::Object(HostObject::new(ToolHandle(tool)))
}

/// What a tool [`Value`] holds.
pub(crate) struct ToolHandle(pub(crate) Arc<dyn Tool>);

struct ToolEntry {
    name: String,
    component: String,
    factory: Arc<dyn ToolFactory>,
}

/// The tool factories of one execution context factory, in init order.
#[derive(Default)]
pub(super) struct ToolRegistry {
    entries: Vec<ToolEntry>,
    initialized: AtomicBool,
    destroyed: AtomicBool,
}

impl ToolRegistry {
    /// Make the tool factories of the loaded `components` from `makers`, with the conf overrides applied.
    pub(super) fn build(
        makers: &[(&'static str, ComponentTools)],
        components: &[ComponentInfo],
        conf: &MNode,
    ) -> Result<Self, Error> {
        let overrides = conf_overrides(conf)?;
        let mut entries: Vec<(i32, ToolEntry)> = Vec::new();
        for (component, maker) in makers {
            if !components.iter().any(|info| info.name == *component) {
                tracing::debug!(component, "Component is not loaded, its tool factories are not made");
                continue;
            }
            for factory in maker() {
                let name = factory.name().to_string();
                if name.is_empty() {
                    return Err(Error::new(format!(
                        "a tool factory of component {component} has an empty name"
                    )));
                }
                if entries.iter().any(|(_, entry)| entry.name == name) {
                    return Err(Error::new(format!("tool factory {name} is defined more than once")));
                }
                let found = overrides.iter().find(|item| item.name == name);
                if found.is_some_and(|item| item.disabled) {
                    tracing::info!("Tool factory {name} of component {component} is disabled in the conf");
                    continue;
                }
                let priority = found
                    .and_then(|item| item.priority)
                    .unwrap_or_else(|| factory.init_priority());
                entries.push((
                    priority,
                    ToolEntry {
                        name,
                        component: (*component).to_string(),
                        factory: Arc::from(factory),
                    },
                ));
            }
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.name.cmp(&right.1.name)));
        Ok(Self {
            entries: entries.into_iter().map(|(_, entry)| entry).collect(),
            ..Self::default()
        })
    }

    pub(super) fn pre_facade_init(&self, ecf: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
        for entry in &self.entries {
            entry
                .factory
                .pre_facade_init(ecf)
                .map_err(|err| Error::new(format!("tool factory {} pre-facade init failed: {err}", entry.name)))?;
        }
        Ok(())
    }

    pub(super) fn init(&self, ecf: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
        if self.initialized.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        for entry in &self.entries {
            let started = std::time::Instant::now();
            entry
                .factory
                .init(ecf)
                .map_err(|err| Error::new(format!("tool factory {} init failed: {err}", entry.name)))?;
            tracing::info!(
                "Initialized tool factory {} of component {} in {}ms",
                entry.name,
                entry.component,
                started.elapsed().as_millis()
            );
        }
        Ok(())
    }

    /// Destroy in reverse order, once. A panic in one is logged and the others still run, since this also runs from
    /// the factory's drop.
    pub(super) fn destroy(&self) {
        if self.entries.is_empty() || self.destroyed.swap(true, Ordering::AcqRel) {
            return;
        }
        for entry in self.entries.iter().rev() {
            if std::panic::catch_unwind(AssertUnwindSafe(|| entry.factory.destroy())).is_err() {
                tracing::error!("Tool factory {} panicked in destroy", entry.name);
            } else {
                tracing::info!("Destroyed tool factory {}", entry.name);
            }
        }
    }

    fn entry(&self, name: &str) -> Option<&ToolEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    pub(super) fn instance(&self, name: &str, parameters: &[Value]) -> Result<Arc<dyn Tool>, Error> {
        let entry = self
            .entry(name)
            .ok_or_else(|| Error::of_kind(ErrorKind::NotFound, format!("no tool factory named {name}")))?;
        entry.factory.instance(parameters)
    }

    pub(super) fn factory(&self, name: &str) -> Option<Arc<dyn ToolFactory>> {
        self.entry(name).map(|entry| Arc::clone(&entry.factory))
    }

    pub(super) fn names(&self) -> Vec<String> {
        self.entries.iter().map(|entry| entry.name.clone()).collect()
    }

    pub(super) fn type_declarations(&self) -> Vec<(String, &'static str)> {
        self.entries
            .iter()
            .filter_map(|entry| entry.factory.type_declaration().map(|text| (entry.name.clone(), text)))
            .collect()
    }
}

/// A conf `tools/tool-factory` element that names a Rust tool factory. Java's `class` entries have no `name`.
struct ConfOverride {
    name: String,
    priority: Option<i32>,
    disabled: bool,
}

fn conf_overrides(conf: &MNode) -> Result<Vec<ConfOverride>, Error> {
    let Some(tools) = conf.first_named("tools") else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for node in tools.children_named("tool-factory") {
        let Some(name) = node.attribute("name").filter(|name| !name.is_empty()) else {
            continue;
        };
        let expanded = |attr: &str| -> Result<Option<String>, Error> {
            match node.attribute(attr) {
                Some(raw) => Ok(Some(env::expand(&raw)?.trim().to_string()).filter(|value| !value.is_empty())),
                None => Ok(None),
            }
        };
        let priority = match expanded("init-priority")? {
            Some(text) => Some(text.parse::<i32>().map_err(|_| {
                Error::new(format!(
                    "tool-factory {name} init-priority {text} is not a whole number"
                ))
            })?),
            None => None,
        };
        let disabled = expanded("disabled")?.is_some_and(|value| value == "true");
        found.push(ConfOverride {
            name,
            priority,
            disabled,
        });
    }
    Ok(found)
}

/// The tool factory as its own type, for Rust code that wants more than the script methods.
pub(super) fn downcast<T: ToolFactory>(factory: Arc<dyn ToolFactory>) -> Option<Arc<T>> {
    let any: Arc<dyn Any + Send + Sync> = factory;
    any.downcast::<T>().ok()
}
