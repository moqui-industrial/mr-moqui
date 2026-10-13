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

//! QuickJS evaluation for server expressions and XML actions.
//!
//! One runtime per thread. Bare names resolve through a proxy on `globalThis`
//! that reads the thread-local context stack. `ec` is an own property of
//! `globalThis` and hosts the calls XML actions emit.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use moqui_lib::context_stack::{ContextStack, Lookup};
use moqui_lib::object::{HostObject, Value};
use rquickjs::class::Trace;
use rquickjs::context::EvalOptions;
use rquickjs::function::{Opt, Rest, This};
use rquickjs::proxy::{Proxy, ProxyHandler, ProxyProperty, ProxyReceiver, ProxyTarget};
use rquickjs::Error as JsError;
use rquickjs::{
    CatchResultExt, Class, Context, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime, Null, Object, Persistent,
    Runtime, Undefined, Value as JsValue,
};

mod conditions;
mod convert;
mod decimal;
mod entities;
mod facades;
mod helpers;
mod services;

use conditions::*;
use convert::*;
use decimal::*;
use entities::*;
pub use facades::run_script_location;
use facades::*;
use helpers::*;
use services::*;

use crate::context::{Error, ExecutionContext};
use moqui_db::sql::{CompareOp, Condition};

use crate::entity::{EntityFacade, EntityFind, EntityList, EntityValue};
use crate::service::call::CallOptions;

const MAX_CACHED_TEMPLATES: usize = 4096;
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const VALUE_OF_MESSAGE: &str =
    "decimal valueOf is not supported; use add, subtract, multiply, divide, eq, compare, or isZero";

thread_local! {
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
    static STACK: RefCell<ContextStack> = RefCell::new(ContextStack::new());
    static ENGINES: RefCell<Vec<Rc<Engine>>> = const { RefCell::new(Vec::new()) };
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Bounds script time on this thread to a transaction's remaining time.
///
/// The QuickJS interrupt handler reads the deadline, so a loop that never returns
/// ends with an error instead of holding the worker. Nested guards keep the earlier deadline.
pub struct DeadlineGuard {
    previous: Option<Instant>,
}

impl DeadlineGuard {
    pub fn new(remaining: Option<Duration>) -> Self {
        let previous = DEADLINE.with(Cell::get);
        if let Some(remaining) = remaining {
            let next = Instant::now() + remaining;
            let chosen = previous.map_or(next, |existing| existing.min(next));
            DEADLINE.with(|deadline| deadline.set(Some(chosen)));
        }
        Self { previous }
    }
}

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        DEADLINE.with(|deadline| deadline.set(self.previous));
    }
}

/// Persistents drop before the context, and the context drops before the runtime.
struct Engine {
    /// Lists and maps a running script read by bare name. The script mutates these objects, so
    /// they are written back to the stack at the end of the run and before a host call.
    tracked: RefCell<HashMap<String, Persistent<JsValue<'static>>>>,
    templates: RefCell<HashMap<String, Persistent<Function<'static>>>>,
    object_utilities: Persistent<Object<'static>>,
    collection_utilities: Persistent<Object<'static>>,
    string_utilities: Persistent<Object<'static>>,
    context: Context,
    // The context and its persistents borrow this runtime. Drop order frees them first.
    #[allow(dead_code)]
    runtime: Runtime,
}

#[derive(Clone, Copy)]
enum HelperKind {
    Object,
    Collection,
    String,
}

/// Push an empty frame on the thread-local context stack.
pub fn push_context_frame() {
    STACK.with(|stack| stack.borrow_mut().push());
}

/// A context-stack frame that pops when dropped.
pub struct ContextFrame;

impl ContextFrame {
    pub fn push() -> Self {
        push_context_frame();
        Self
    }
}

impl Drop for ContextFrame {
    fn drop(&mut self) {
        STACK.with(|stack| {
            let _ = stack.borrow_mut().pop();
        });
    }
}

/// Put `value` in the top frame.
pub fn set_context_name(name: &str, value: Value) -> Result<(), Error> {
    STACK.with(|stack| stack.borrow_mut().put(name, value).map(|_| ()).map_err(Error::from))
}

/// Replace the thread-local stack with an empty one. The runtime stays.
pub fn clear_context_stack() {
    STACK.with(|stack| *stack.borrow_mut() = ContextStack::new());
}

/// Run one script statement in the server runtime. The script is not strict.
pub fn run_script(source: &str) -> Result<(), Error> {
    if source.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    eval_script(source)
}

/// Evaluate a strict expression. Null, false, and a zero number are false. A decimal is true.
pub fn eval_condition(expression: &str) -> Result<bool, Error> {
    if expression.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    with_context(|ctx| {
        let value = eval_js::<JsValue>(ctx, expression)?;
        Ok(js_truthy(&value))
    })
}

/// Expand a template. `${expression}` runs in this runtime. No `$`, or a trailing `$`, is unchanged.
pub fn expand(template: &str) -> Result<String, Error> {
    if !template.contains('$') || template.ends_with('$') {
        return Ok(template.to_string());
    }
    if template.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    with_context(|ctx| expand_in(ctx, template))
}

pub(super) fn with_ctx<R>(body: impl FnOnce(&Ctx) -> Result<R, Error>) -> Result<R, Error> {
    with_context(body)
}

pub(super) fn eval_source<'js, T>(ctx: &Ctx<'js>, source: &str) -> Result<T, Error>
where
    T: FromJs<'js>,
{
    eval_js(ctx, source)
}

/// Run one generated action script.
///
/// The script is not strict. A strict function would reject bare assignment, and
/// that assignment is how a plain value reaches the context stack.
pub(super) fn eval_script(source: &str) -> Result<(), Error> {
    with_context(|ctx| {
        let mut options = EvalOptions::default();
        options.strict = false;
        options.global = true;
        ctx.eval_with_options::<JsValue, _>(source, options)
            .catch(ctx)
            .map(|_| ())
            .map_err(|caught| Error::new(caught.to_string()))
    })
}

/// Same prefix rules as [`expand`], on the callback's context.
///
/// [`expand`] enters `Context::with` again, and that lock is already held here.
fn expand_here(ctx: &Ctx<'_>, template: &str) -> Result<String, Error> {
    if !template.contains('$') || template.ends_with('$') {
        return Ok(template.to_string());
    }
    if template.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    expand_in(ctx, template)
}

/// A script that runs a service that runs a script needs its own runtime: a `Context::with`
/// cannot be entered again while it is held. Each nesting level gets one, kept for reuse.
const MAX_SCRIPT_DEPTH: usize = 16;

struct DepthGuard;

impl Drop for DepthGuard {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn with_context<R>(body: impl FnOnce(&Ctx) -> Result<R, Error>) -> Result<R, Error> {
    let depth = DEPTH.with(Cell::get);
    if depth >= MAX_SCRIPT_DEPTH {
        return Err(Error::new("scripts are nested too deeply"));
    }
    let existing = ENGINES.with(|engines| engines.borrow().get(depth).cloned());
    let engine = match existing {
        Some(engine) => engine,
        None => {
            let started = Rc::new(Engine::start()?);
            ENGINES.with(|engines| engines.borrow_mut().push(Rc::clone(&started)));
            started
        }
    };
    DEPTH.with(|current| current.set(depth + 1));
    let _guard = DepthGuard;
    engine.context.with(|ctx| {
        let result = body(&ctx);
        let flushed = flush_tracked(&ctx, &engine, true);
        match (result, flushed) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(err), _) | (Ok(_), Err(err)) => Err(err),
        }
    })
}

/// Write each tracked list and map back to the context stack. `clear` also forgets them, so the
/// next bare read converts the stack's value again.
fn flush_tracked(ctx: &Ctx<'_>, engine: &Engine, clear: bool) -> Result<(), Error> {
    let entries: Vec<(String, Persistent<JsValue<'static>>)> = if clear {
        engine.tracked.borrow_mut().drain().collect()
    } else {
        engine
            .tracked
            .borrow()
            .iter()
            .map(|(name, saved)| (name.clone(), saved.clone()))
            .collect()
    };
    let mut first = None;
    for (name, saved) in entries {
        let outcome = saved
            .restore(ctx)
            .map_err(|err| js_caught(ctx, err))
            .and_then(|value| js_value_to_host(ctx, &value));
        match outcome {
            Ok(host) => {
                STACK.with(|stack| {
                    let mut stack = stack.borrow_mut();
                    if !stack.put_existing(&name, host.clone()) {
                        let _ = stack.put(name.clone(), host);
                    }
                });
            }
            Err(err) => {
                first.get_or_insert(err);
            }
        }
    }
    match first {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Flush the innermost script's tracked values, for a host call that reads the stack.
fn flush_current(ctx: &Ctx<'_>, clear: bool) -> rquickjs::Result<()> {
    match current_engine() {
        Some(engine) => flush_tracked(ctx, &engine, clear).map_err(|err| Exception::throw_message(ctx, err.message())),
        None => Ok(()),
    }
}

/// The engine of the innermost running script.
fn current_engine() -> Option<Rc<Engine>> {
    let depth = DEPTH.with(Cell::get);
    ENGINES.with(|engines| engines.borrow().get(depth.checked_sub(1)?).cloned())
}

impl Engine {
    fn start() -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(js_error)?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_interrupt_handler(Some(Box::new(|| {
            DEADLINE
                .with(Cell::get)
                .is_some_and(|deadline| Instant::now() >= deadline)
        })));
        let context = Context::full(&runtime).map_err(js_error)?;
        let helpers = context.with(|ctx| install_context(&ctx))?;
        Ok(Self {
            tracked: RefCell::new(HashMap::new()),
            templates: RefCell::new(HashMap::new()),
            object_utilities: helpers.object_utilities,
            collection_utilities: helpers.collection_utilities,
            string_utilities: helpers.string_utilities,
            context,
            runtime,
        })
    }
}

struct Helpers {
    object_utilities: Persistent<Object<'static>>,
    collection_utilities: Persistent<Object<'static>>,
    string_utilities: Persistent<Object<'static>>,
}

fn install_context(ctx: &Ctx<'_>) -> Result<Helpers, Error> {
    let object_utilities = Persistent::save(ctx, Object::new(ctx.clone()).map_err(js_error)?);
    let collection_utilities = Persistent::save(ctx, Object::new(ctx.clone()).map_err(js_error)?);
    let string_utilities = Persistent::save(ctx, Object::new(ctx.clone()).map_err(js_error)?);
    // Own property before the proxy. A later set walks the prototype and the proxy refuses objects.
    install_ec(ctx)?;
    install_helpers(ctx)?;
    install_proxy(ctx)?;
    Ok(Helpers {
        object_utilities,
        collection_utilities,
        string_utilities,
    })
}

fn install_proxy(ctx: &Ctx<'_>) -> Result<(), Error> {
    let target = Object::new(ctx.clone()).map_err(js_error)?;
    let handler = ProxyHandler::new(ctx.clone())
        .map_err(js_error)?
        .with_has(proxy_has)
        .map_err(js_error)?
        .with_getter(proxy_get)
        .map_err(js_error)?
        .with_setter(proxy_set)
        .map_err(js_error)?;
    let proxy = Proxy::new(ctx.clone(), target, handler).map_err(js_error)?;
    ctx.globals().set_prototype(Some(&proxy)).map_err(js_error)?;
    Ok(())
}

fn proxy_has(target: ProxyTarget<'_>, property: ProxyProperty<'_>) -> rquickjs::Result<bool> {
    let ctx = target.0.ctx().clone();
    host_guard(&ctx, || proxy_has_inner(&target, &property))
}

fn proxy_has_inner(target: &ProxyTarget<'_>, property: &ProxyProperty<'_>) -> rquickjs::Result<bool> {
    if property.is_symbol() {
        return Ok(false);
    }
    let name = property.to_string()?;
    let ctx = target.0.ctx().clone();
    Ok(!global_has_own(&ctx, &name)?)
}

fn proxy_get<'js>(
    target: ProxyTarget<'js>,
    property: ProxyProperty<'js>,
    _receiver: ProxyReceiver<'js>,
) -> rquickjs::Result<JsValue<'js>> {
    let ctx = target.0.ctx().clone();
    host_guard(&ctx, || proxy_get_inner(&ctx, &property))
}

fn proxy_get_inner<'js>(ctx: &Ctx<'js>, property: &ProxyProperty<'js>) -> rquickjs::Result<JsValue<'js>> {
    let ctx = ctx.clone();
    if property.is_symbol() {
        return Undefined.into_js(&ctx);
    }
    let name = property.to_string()?;
    if let Some(engine) = current_engine() {
        let saved = engine.tracked.borrow().get(&name).cloned();
        if let Some(saved) = saved {
            return saved.restore(&ctx);
        }
    }
    match stack_get(&name) {
        Lookup::Value(value @ (Value::List(_) | Value::Map(_))) => {
            let js = host_to_js(&ctx, value)?;
            if let Some(engine) = current_engine() {
                engine
                    .tracked
                    .borrow_mut()
                    .insert(name, Persistent::save(&ctx, js.clone()));
            }
            Ok(js)
        }
        Lookup::Value(value) => host_to_js(&ctx, value),
        Lookup::Context => context_snapshot(&ctx),
        Lookup::Missing => match helper_object(&ctx, &name)? {
            Some(helper) => helper.into_js(&ctx),
            None => Null.into_js(&ctx),
        },
    }
}

fn proxy_set<'js>(
    target: ProxyTarget<'js>,
    property: ProxyProperty<'js>,
    value: JsValue<'js>,
    _receiver: ProxyReceiver<'js>,
) -> rquickjs::Result<bool> {
    let ctx = target.0.ctx().clone();
    host_guard(&ctx, || proxy_set_inner(&ctx, &property, &value))
}

fn proxy_set_inner<'js>(ctx: &Ctx<'js>, property: &ProxyProperty<'js>, value: &JsValue<'js>) -> rquickjs::Result<bool> {
    let ctx = ctx.clone();
    if property.is_symbol() {
        return Ok(true);
    }
    let name = property.to_string()?;
    if let Some(engine) = current_engine() {
        engine.tracked.borrow_mut().remove(&name);
    }
    let host = js_to_host(&ctx, value)?;
    STACK.with(|stack| {
        stack
            .borrow_mut()
            .put(name, host)
            .map(|_| ())
            .map_err(|err| Exception::throw_message(&ctx, err.message()))
    })?;
    Ok(true)
}

fn global_has_own(ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<bool> {
    let globals = ctx.globals();
    let object: Object = globals.get("Object")?;
    let prototype: Object = object.get("prototype")?;
    let has_own: Function = prototype.get("hasOwnProperty")?;
    has_own.call((This(globals), name))
}

/// `context` reads as a plain object of every visible name. A write to it does not reach the stack.
fn context_snapshot<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    flush_current(ctx, false)?;
    let mut visible = STACK.with(|stack| stack.borrow().combined_map());
    visible.retain(|_, value| !matches!(value, Value::Bytes(_)));
    collection_to_js(ctx, Value::Map(visible))
}

/// Set a name on the stack from host code, forgetting any list or map the script already read
/// under that name so its next read sees the new value.
fn put_name(ctx: &Ctx<'_>, name: &str, value: Value) -> rquickjs::Result<()> {
    if let Some(engine) = current_engine() {
        engine.tracked.borrow_mut().remove(name);
    }
    STACK.with(|stack| {
        stack
            .borrow_mut()
            .put(name, value)
            .map(|_| ())
            .map_err(|err| Exception::throw_message(ctx, err.message()))
    })
}

fn stack_get(name: &str) -> Lookup {
    STACK.with(|stack| stack.borrow().get(name))
}

fn helper_kind(name: &str) -> Option<HelperKind> {
    match name {
        "ObjectUtilities" => Some(HelperKind::Object),
        "CollectionUtilities" => Some(HelperKind::Collection),
        "StringUtilities" => Some(HelperKind::String),
        _ => None,
    }
}

fn helper_object<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Option<Object<'js>>> {
    let Some(kind) = helper_kind(name) else {
        return Ok(None);
    };
    let persistent = current_engine().map(|engine| match kind {
        HelperKind::Object => engine.object_utilities.clone(),
        HelperKind::Collection => engine.collection_utilities.clone(),
        HelperKind::String => engine.string_utilities.clone(),
    });
    let Some(persistent) = persistent else {
        return Ok(None);
    };
    persistent.restore(ctx).map(Some)
}

fn eval_js<'js, T>(ctx: &Ctx<'js>, source: &str) -> Result<T, Error>
where
    T: FromJs<'js>,
{
    let mut options = EvalOptions::default();
    options.strict = true;
    options.global = true;
    ctx.eval_with_options::<T, _>(source, options)
        .catch(ctx)
        .map_err(|caught| Error::new(caught.to_string()))
}

fn expand_in(ctx: &Ctx<'_>, template: &str) -> Result<String, Error> {
    if let Some(function) = restore_template(ctx, template)? {
        return call_template(ctx, function);
    }
    let function = eval_js::<Function>(ctx, &template_source(template))?;
    save_template(ctx, template, &function);
    call_template(ctx, function)
}

fn restore_template<'js>(ctx: &Ctx<'js>, template: &str) -> Result<Option<Function<'js>>, Error> {
    let persistent = current_engine().and_then(|engine| engine.templates.borrow().get(template).cloned());
    let Some(persistent) = persistent else {
        return Ok(None);
    };
    persistent.restore(ctx).map(Some).map_err(js_error)
}

fn save_template<'js>(ctx: &Ctx<'js>, template: &str, function: &Function<'js>) {
    let saved = Persistent::save(ctx, function.clone());
    if let Some(engine) = current_engine() {
        let mut templates = engine.templates.borrow_mut();
        if templates.len() >= MAX_CACHED_TEMPLATES {
            templates.clear();
        }
        templates.insert(template.to_string(), saved);
    }
}

fn call_template(ctx: &Ctx<'_>, function: Function<'_>) -> Result<String, Error> {
    function
        .call(())
        .catch(ctx)
        .map_err(|caught| Error::new(caught.to_string()))
}

fn template_source(template: &str) -> String {
    let mut source = String::from("(function(){return `");
    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if bytes.get(index + 1) == Some(&b'$') && bytes.get(index + 2) == Some(&b'{') {
                source.push_str("\\${");
                index += 3;
            } else {
                source.push_str("\\\\");
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'`' {
            source.push_str("\\`");
            index += 1;
            continue;
        }
        let ch = template[index..].chars().next().unwrap_or('\u{FFFD}');
        source.push(ch);
        index += ch.len_utf8();
    }
    source.push_str("`;})");
    source
}

fn host_guard<'js, T>(ctx: &Ctx<'js>, body: impl FnOnce() -> rquickjs::Result<T>) -> rquickjs::Result<T> {
    match std::panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => Err(Exception::throw_message(ctx, &panic_message(payload))),
    }
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "host callback panicked".to_string()
}

fn throw_lib(ctx: &Ctx<'_>, err: &moqui_lib::error::Error) -> JsError {
    Exception::throw_message(ctx, err.message())
}

/// `ec` is an own property of `globalThis`, so the bare-name proxy does not hide it.
fn install_ec(ctx: &Ctx<'_>) -> Result<(), Error> {
    let ec = caught(ctx, "ec", Object::new(ctx.clone()))?;
    ec.set("entity", entity_object(ctx)?).map_err(js_error)?;
    ec.set("resource", resource_object(ctx)?).map_err(js_error)?;
    let message = caught(ctx, "message", Class::instance(ctx.clone(), HostMessage {}))?;
    ec.set("message", message).map_err(js_error)?;
    ec.set("logger", logger_object(ctx)?).map_err(js_error)?;
    ec.set("service", service_object(ctx)?).map_err(js_error)?;
    let user = caught(ctx, "user", Class::instance(ctx.clone(), HostUser {}))?;
    ec.set("user", user).map_err(js_error)?;
    let web = caught(ctx, "web", Class::instance(ctx.clone(), HostWeb {}))?;
    ec.set("web", web).map_err(js_error)?;
    ec.set("cache", cache_object(ctx)?).map_err(js_error)?;
    caught(ctx, "globals", ctx.globals().set("ec", ec))?;
    Ok(())
}

fn caught<'js, T>(ctx: &Ctx<'js>, step: &str, result: rquickjs::Result<T>) -> Result<T, Error> {
    result.catch(ctx).map_err(|err| Error::new(format!("{step}: {err}")))
}

fn entity_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, Error> {
    let entity = Object::new(ctx.clone()).map_err(js_error)?;
    let find = caught(ctx, "entity.find", Function::new(ctx.clone(), entity_find))?;
    entity.set("find", find).map_err(js_error)?;
    let make = caught(ctx, "entity.makeValue", Function::new(ctx.clone(), entity_make_value))?;
    entity.set("makeValue", make).map_err(js_error)?;
    let factory = Object::new(ctx.clone()).map_err(js_error)?;
    let make_condition = Function::new(ctx.clone(), make_condition).map_err(js_error)?;
    factory.set("makeCondition", make_condition).map_err(js_error)?;
    let make_date = Function::new(ctx.clone(), make_condition_date).map_err(js_error)?;
    factory.set("makeConditionDate", make_date).map_err(js_error)?;
    let make_direct = Function::new(ctx.clone(), make_action_condition).map_err(js_error)?;
    factory
        .set("makeActionConditionDirect", make_direct)
        .map_err(js_error)?;
    entity.set("conditionFactory", factory).map_err(js_error)?;
    Ok(entity)
}

fn cache_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, Error> {
    let cache = Object::new(ctx.clone()).map_err(js_error)?;
    let get_cache = Function::new(ctx.clone(), cache_get_cache).map_err(js_error)?;
    cache.set("getCache", get_cache).map_err(js_error)?;
    Ok(cache)
}

fn resource_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, Error> {
    let resource = Object::new(ctx.clone()).map_err(js_error)?;
    let expand = Function::new(ctx.clone(), resource_expand).map_err(js_error)?;
    resource.set("expand", expand).map_err(js_error)?;
    let script = Function::new(ctx.clone(), resource_script).map_err(js_error)?;
    resource.set("script", script).map_err(js_error)?;
    Ok(resource)
}

fn logger_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, Error> {
    let logger = Object::new(ctx.clone()).map_err(js_error)?;
    let log = Function::new(ctx.clone(), logger_log).map_err(js_error)?;
    logger.set("log", log).map_err(js_error)?;
    let warn = Function::new(ctx.clone(), logger_warn).map_err(js_error)?;
    logger.set("warn", warn).map_err(js_error)?;
    Ok(logger)
}

fn service_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, Error> {
    let service = Object::new(ctx.clone()).map_err(js_error)?;
    let sync = Function::new(ctx.clone(), service_sync).map_err(js_error)?;
    service.set("sync", sync).map_err(js_error)?;
    let async_call = Function::new(ctx.clone(), service_async).map_err(js_error)?;
    service.set("async", async_call).map_err(js_error)?;
    Ok(service)
}

fn facade_of(ctx: &Ctx<'_>) -> rquickjs::Result<EntityFacade> {
    let installed = with_ec(ctx, |ec| ec.factory().entity())?;
    match installed {
        Some(facade) => Ok(facade),
        None => Err(Exception::throw_message(ctx, "no entity facade is installed")),
    }
}

fn with_ec<T>(ctx: &Ctx<'_>, body: impl FnOnce(&ExecutionContext) -> T) -> rquickjs::Result<T> {
    ExecutionContext::with_current(body).ok_or_else(|| Exception::throw_message(ctx, "no execution context is entered"))
}

fn not_implemented(ctx: &Ctx<'_>, what: &str) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("{what} is not implemented"))
}

fn js_error(err: impl ToString) -> Error {
    Error::new(err.to_string())
}

/// Read one name from the thread-local stack. `None` when the name is missing.
pub fn context_value(name: &str) -> Option<Value> {
    match stack_get(name) {
        Lookup::Value(value) => Some(value),
        Lookup::Missing | Lookup::Context => None,
    }
}

/// Evaluate a JavaScript expression to a host value.
///
/// # Errors
///
/// Returns an error when the expression does not parse, throws, or gives a value that cannot be held.
pub fn eval_value(expression: &str) -> Result<Value, Error> {
    if expression.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    with_context(|ctx| {
        let evaluated = eval_js::<JsValue>(ctx, expression)?;
        js_value_to_host(ctx, &evaluated)
    })
}

/// Evaluate a JavaScript object expression into a map. A non-object is an error.
pub fn eval_map(expression: &str) -> Result<std::collections::BTreeMap<String, Value>, Error> {
    if expression.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    with_context(|ctx| {
        let evaluated = eval_js::<JsValue>(ctx, expression)?;
        object_map(ctx, &evaluated)
    })
}

#[cfg(test)]
mod tests {
    use super::{clear_context_stack, eval_condition, expand, push_context_frame, set_context_name};
    use crate::context::Error;
    use moqui_lib::object::{basic_convert, Value};

    const MAX_COEFFICIENT: &str = "79228162514264337593543950335";

    fn decimal(text: &str) -> Value {
        let converted = basic_convert(&Value::string(text), "BigDecimal");
        assert!(converted.changed, "expected a decimal from {text}");
        converted.value
    }

    #[test]
    fn unset_bare_name_is_null() -> Result<(), Error> {
        clear_context_stack();
        assert!(!eval_condition("unsetBareName")?);
        assert!(eval_condition("unsetBareName === null")?);
        assert!(eval_condition("typeof unsetBareName === \"object\"")?);
        assert!(!eval_condition("0")?);
        assert!(!eval_condition("false")?);
        Ok(())
    }

    #[test]
    fn stack_name_evaluates_to_its_value() -> Result<(), Error> {
        clear_context_stack();
        set_context_name("orderId", Value::string("1000"))?;
        push_context_frame();
        set_context_name("orderId", Value::string("2000"))?;
        assert!(eval_condition("orderId === \"2000\"")?);
        assert!(eval_condition("(assignedName = \"x\") === \"x\"")?);
        set_context_name("assignedName", Value::string("y"))?;
        assert!(eval_condition("assignedName === \"y\"")?);
        set_context_name("Array", Value::string("hidden"))?;
        assert!(eval_condition("typeof Array === \"function\"")?);
        set_context_name("ObjectUtilities", Value::string("mine"))?;
        assert!(eval_condition("ObjectUtilities === \"mine\"")?);
        clear_context_stack();
        assert!(eval_condition(
            "typeof ObjectUtilities === \"object\" && ObjectUtilities !== null"
        )?);
        Ok(())
    }

    #[test]
    fn decimal_overflow_is_a_js_exception() -> Result<(), Error> {
        clear_context_stack();
        set_context_name("big", decimal(MAX_COEFFICIENT))?;
        set_context_name("zero", decimal("0"))?;
        assert!(eval_condition("zero")?);
        assert!(eval_condition("zero.isZero()")?);
        assert!(!eval_condition("big.isZero()")?);
        assert!(!eval_condition("big == big")?);
        let overflow = eval_condition("big.add(big)").expect_err("overflow");
        assert!(overflow.to_string().contains("overflow"), "{overflow}");
        let value_of = eval_condition("big + 1").expect_err("valueOf");
        assert!(value_of.to_string().contains("valueOf"), "{value_of}");
        Ok(())
    }

    #[test]
    fn expand_substitutes_a_context_field() -> Result<(), Error> {
        clear_context_stack();
        set_context_name("name", Value::string("Ada"))?;
        set_context_name("big", decimal(MAX_COEFFICIENT))?;
        assert_eq!(expand("Hello ${name}")?, "Hello Ada");
        assert_eq!(expand("Hello ${name}")?, "Hello Ada");
        assert_eq!(expand("no dollars")?, "no dollars");
        assert_eq!(expand("trailing$")?, "trailing$");
        assert_eq!(expand("${big}")?, MAX_COEFFICIENT);
        Ok(())
    }
}
