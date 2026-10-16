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

//! Tool factories: the registry, the lifecycle on a booted factory, rust services they register, and the tool object
//! scripts get from `ec.factory.getTool`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use moqui_lib::error::Error as LibError;
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::component::ComponentInfo;
use super::tool::{tool_value, ComponentTools, Tool, ToolFactory, ToolRegistry};
use super::{BootOptions, Error, ExecutionContextFactory, Executor};
use crate::entity::EntityFacade;
use crate::script::{clear_context_stack, eval_condition, run_script, set_context_name};

/// A tool factory that does nothing but have a name and a priority.
struct Named(&'static str, i32);

impl ToolFactory for Named {
    fn name(&self) -> &str {
        self.0
    }

    fn init_priority(&self) -> i32 {
        self.1
    }
}

fn three() -> Vec<Box<dyn ToolFactory>> {
    vec![
        Box::new(Named("C", 5)),
        Box::new(Named("A", 20)),
        Box::new(Named("B", 20)),
    ]
}

fn off() -> Vec<Box<dyn ToolFactory>> {
    vec![Box::new(Named("Off", 1))]
}

fn absent() -> Vec<Box<dyn ToolFactory>> {
    vec![Box::new(Named("Absent", 1))]
}

fn twice() -> Vec<Box<dyn ToolFactory>> {
    vec![Box::new(Named("A", 1))]
}

fn component(name: &str) -> ComponentInfo {
    ComponentInfo {
        name: name.to_string(),
        version: String::new(),
        location: PathBuf::new(),
        depends_on: Vec::new(),
    }
}

fn conf(text: &str) -> MNode {
    MNode::parse_text("moqui-conf", text).unwrap()
}

#[test]
fn registry_orders_by_priority_then_name_and_applies_the_conf() {
    let makers: &[(&str, ComponentTools)] = &[("one", three), ("one", off), ("gone", absent)];
    let components = [component("one")];
    let registry = ToolRegistry::build(makers, &components, &conf("<moqui-conf/>")).unwrap();
    assert_eq!(registry.names(), ["Off", "C", "A", "B"]);
    // The conf reorders by name and leaves one out; a Java class element is not a Rust tool factory.
    let registry = ToolRegistry::build(
        makers,
        &components,
        &conf(
            r#"<moqui-conf><tools>
                <tool-factory class="org.moqui.impl.tools.MCacheToolFactory" init-priority="03"/>
                <tool-factory name="B" init-priority="1"/>
                <tool-factory name="Off" disabled="true"/>
            </tools></moqui-conf>"#,
        ),
    )
    .unwrap();
    assert_eq!(registry.names(), ["B", "C", "A"]);
    // The component of `Absent` is not loaded, so it is never made.
    assert!(registry.instance("Absent", &[]).is_err());
}

#[test]
fn registry_refuses_a_duplicate_name_and_a_bad_priority() {
    let components = [component("one"), component("two")];
    let duplicate: &[(&str, ComponentTools)] = &[("one", three), ("two", twice)];
    let err = ToolRegistry::build(duplicate, &components, &conf("<moqui-conf/>"))
        .err()
        .expect("duplicate");
    assert!(
        err.to_string().contains("tool factory A is defined more than once"),
        "{err}"
    );
    let err = ToolRegistry::build(
        &[("one", three)],
        &components,
        &conf(r#"<moqui-conf><tools><tool-factory name="A" init-priority="soon"/></tools></moqui-conf>"#),
    )
    .err()
    .expect("bad priority");
    assert!(err.to_string().contains("init-priority soon"), "{err}");
}

/// What the lifecycle factories did, in order. Only the lifecycle test makes them.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn events() -> Vec<String> {
    EVENTS.lock().unwrap().clone()
}

fn record(event: String) {
    EVENTS.lock().unwrap().push(event);
}

/// A `type="rust"` service a tool factory registers: its parameters back, with `ran`.
fn echo(parameters: &BTreeMap<String, Value>, _facade: &EntityFacade) -> Result<BTreeMap<String, Value>, LibError> {
    let mut out = parameters.clone();
    out.insert("ran".to_string(), Value::Bool(true));
    Ok(out)
}

struct Lifecycle {
    name: &'static str,
    priority: i32,
}

impl ToolFactory for Lifecycle {
    fn name(&self) -> &str {
        self.name
    }

    fn init_priority(&self) -> i32 {
        self.priority
    }

    fn pre_facade_init(&self, ecf: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
        record(format!("pre {}", self.name));
        assert!(ecf.entity().is_none(), "no entity facade before init");
        if self.name == "First" {
            crate::register_service!(ecf, echo)?;
        }
        Ok(())
    }

    fn init(&self, ecf: &Arc<ExecutionContextFactory>) -> Result<(), Error> {
        record(format!("init {}", self.name));
        assert!(ecf.entity().is_some(), "the entity facade is attached before init");
        Ok(())
    }

    fn instance(&self, parameters: &[Value]) -> Result<Arc<dyn Tool>, Error> {
        let label = match parameters.first() {
            Some(Value::String(text)) => text.clone(),
            _ => self.name.to_lowercase(),
        };
        Ok(Arc::new(Echo { label }))
    }

    fn destroy(&self) {
        record(format!("destroy {}", self.name));
    }
}

fn lifecycle() -> Vec<Box<dyn ToolFactory>> {
    vec![
        Box::new(Lifecycle {
            name: "Second",
            priority: 2,
        }),
        Box::new(Lifecycle {
            name: "First",
            priority: 1,
        }),
    ]
}

struct Echo {
    label: String,
}

impl Tool for Echo {
    fn methods(&self) -> &[&str] {
        &["echo", "child", "fail"]
    }

    fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, Error> {
        match method {
            "echo" => Ok(Value::List(args)),
            "child" => Ok(tool_value(Arc::new(Echo {
                label: format!("{} child", self.label),
            }))),
            "fail" => Err(Error::new("failed on purpose")),
            _ => unreachable!("not a method: {method}"),
        }
    }

    fn property(&self, name: &str) -> Result<Option<Value>, Error> {
        Ok((name == "label").then(|| Value::String(self.label.clone())))
    }
}

const RUST_SERVICES: &str = r#"<services>
    <service verb="echo" noun="Rust" type="rust" location="moqui_core::context::tool_tests::echo" authenticate="false">
        <in-parameters><parameter name="note"/></in-parameters>
        <out-parameters><parameter name="note"/><parameter name="ran"/></out-parameters>
    </service>
    <service verb="missing" noun="Rust" type="rust" location="moqui_core::context::tool_tests::not_registered"/>
</services>"#;

fn holds(expression: &str) {
    assert!(eval_condition(expression).unwrap(), "{expression}");
}

#[test]
fn a_booted_factory_runs_the_lifecycle_and_scripts_reach_the_tools() {
    let runtime = std::env::temp_dir().join(format!("moqui-tool-boot-{}", std::process::id()));
    std::fs::create_dir_all(&runtime).unwrap();
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/base-component/framework");
    static MAKERS: [(&str, ComponentTools); 1] = [("framework", lifecycle)];
    let factory = {
        let _lock = crate::user::session::cookie_property_lock();
        ExecutionContextFactory::boot_with_options(
            &runtime,
            &framework,
            BootOptions {
                executor: Executor::Inline,
                tool_factories: Some(&MAKERS),
            },
        )
        .expect("boot")
    };
    assert_eq!(events(), ["pre First", "pre Second"]);
    assert_eq!(factory.tool_names(), ["First", "Second"]);

    factory.set_entity(crate::testing::open());
    factory.post_facade_init().unwrap();
    factory.post_facade_init().unwrap();
    assert_eq!(events()[2..], ["init First", "init Second"], "init runs once, in order");

    // The typed tool factory, for Rust code.
    assert!(factory.tool_factory::<Lifecycle>("First").is_some());
    assert!(factory.tool_factory::<Named>("First").is_none());
    assert!(factory.tool_factory::<Lifecycle>("Nope").is_none());

    // A built-in location and a second registration of a location are refused.
    let err = factory
        .register_service("moqui_core::service::mail::send_template", echo)
        .expect_err("built-in");
    assert!(err.to_string().contains("built-in"), "{err}");
    let err = factory
        .register_service("moqui_core::context::tool_tests::echo", echo)
        .expect_err("twice");
    assert!(err.to_string().contains("already registered"), "{err}");

    let path = runtime.join("ToolServices.xml");
    std::fs::write(&path, RUST_SERVICES).unwrap();
    let ctx = factory.new_context();
    {
        let _entered = ctx.enter();
        let failures = crate::service::lint(std::slice::from_ref(&path)).unwrap();
        assert_eq!(
            failures,
            ["service missing#Rust rust location moqui_core::context::tool_tests::not_registered is not registered"]
        );
        let mut parameters = BTreeMap::new();
        parameters.insert("note".to_string(), Value::string("hello"));
        let out = crate::service::sync("echo#Rust", &parameters).unwrap();
        assert_eq!(out.get("note").and_then(Value::as_str), Some("hello"));
        assert_eq!(out.get("ran"), Some(&Value::Bool(true)));
        // The definitions are read now, so a later registration would not be seen by them.
        let err = factory
            .register_service("moqui_core::context::tool_tests::late", echo)
            .expect_err("late");
        assert!(
            err.to_string().contains("after the service definitions were read"),
            "{err}"
        );

        clear_context_stack();
        holds("JSON.stringify(ec.factory.getTool('First').echo(1, 'a', null)) === '[1,\"a\",null]'");
        holds("ec.factory.getTool('First').label === 'first'");
        holds("ec.factory.getTool('First', 'given').label === 'given'");
        holds("ec.factory.getTool('Second').missing === undefined");
        holds("'echo' in ec.factory.getTool('First') && !('nope' in ec.factory.getTool('First'))");
        holds("Object.getOwnPropertyNames(ec.factory.getTool('First')).join() === 'echo,child,fail'");
        // A method's tool result is a tool, with methods of its own.
        holds("ec.factory.getTool('First').child().label === 'first child'");
        holds("ec.factory.getTool('First').child().echo('x')[0] === 'x'");
        // A tool is read-only.
        holds("(() => { const t = ec.factory.getTool('First'); try { t.label = 'x' } catch (e) {} return t.label === 'first' })()");
        // Errors from the tool and from getTool are script exceptions.
        holds("(() => { try { ec.factory.getTool('First').fail(); return false } catch (e) { return String(e.message).includes('failed on purpose') } })()");
        holds("(() => { try { ec.factory.getTool('Nope'); return false } catch (e) { return String(e.message).includes('no tool factory named Nope') } })()");

        // Kept on the context stack and read back, it is still the tool, as `<set field="tool" from="..."/>` keeps it.
        run_script("held = ec.factory.getTool('First', 'kept')").unwrap();
        holds("held.label === 'kept' && held.echo(2)[0] === 2");
        set_context_name(
            "given",
            tool_value(Arc::new(Echo {
                label: "from rust".to_string(),
            })),
        )
        .unwrap();
        holds("given.label === 'from rust' && given.child().label === 'from rust child'");
        clear_context_stack();
    }
    drop(ctx);

    factory.destroy_tools();
    factory.destroy_tools();
    drop(factory);
    assert_eq!(
        events()[4..],
        ["destroy Second", "destroy First"],
        "destroy runs once, in reverse"
    );
    let _ = std::fs::remove_dir_all(&runtime);
}
