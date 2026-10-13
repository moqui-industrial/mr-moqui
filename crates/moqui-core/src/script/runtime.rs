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

use crate::context::{Error, ExecutionContext};
use moqui_db::sql::Condition;

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

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostDecimal {
    #[qjs(skip_trace)]
    inner: Value,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostDecimal {
    fn value_of(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(Exception::throw_message(&ctx, VALUE_OF_MESSAGE)))
    }

    #[qjs(rename = "toString")]
    fn js_string(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || Ok(moqui_lib::object::to_plain_string(&self.inner)))
    }

    fn add<'js>(&self, ctx: Ctx<'js>, other: &HostDecimal) -> rquickjs::Result<HostDecimal> {
        host_arith(&ctx, &self.inner, &other.inner, Arith::Add)
    }

    fn subtract<'js>(&self, ctx: Ctx<'js>, other: &HostDecimal) -> rquickjs::Result<HostDecimal> {
        host_arith(&ctx, &self.inner, &other.inner, Arith::Subtract)
    }

    fn multiply<'js>(&self, ctx: Ctx<'js>, other: &HostDecimal) -> rquickjs::Result<HostDecimal> {
        host_arith(&ctx, &self.inner, &other.inner, Arith::Multiply)
    }

    fn divide<'js>(&self, ctx: Ctx<'js>, other: &HostDecimal) -> rquickjs::Result<HostDecimal> {
        host_arith(&ctx, &self.inner, &other.inner, Arith::Divide)
    }

    #[qjs(rename = "eq")]
    fn equal<'js>(&self, ctx: Ctx<'js>, other: &HostDecimal) -> rquickjs::Result<bool> {
        host_guard(&ctx, || {
            decimals_equal(&self.inner, &other.inner).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn compare<'js>(&self, ctx: Ctx<'js>, other: &HostDecimal) -> rquickjs::Result<i32> {
        host_guard(&ctx, || {
            let ordering = decimals_compare(&self.inner, &other.inner).map_err(|err| throw_lib(&ctx, &err))?;
            Ok(match ordering {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            })
        })
    }

    fn is_zero<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<bool> {
        host_guard(&ctx, || {
            let Some(number) = self.inner.as_decimal() else {
                return Err(Exception::throw_message(&ctx, "not a decimal"));
            };
            Ok(moqui_lib::object::decimal_is_zero(number))
        })
    }
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

fn host_to_js<'js>(ctx: &Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::Null => Null.into_js(ctx),
        Value::Bool(flag) => flag.into_js(ctx),
        Value::String(text) => text.into_js(ctx),
        Value::I32(number) => number.into_js(ctx),
        Value::I64(number) => i64_to_js(ctx, number),
        Value::F64(number) => number.into_js(ctx),
        Value::Decimal(_) => {
            let class = Class::instance(ctx.clone(), HostDecimal { inner: value })?;
            class.into_js(ctx)
        }
        Value::Timestamp(_) | Value::Date(_) | Value::Time(_) => temporal_text(&value).into_js(ctx),
        Value::List(_) | Value::Map(_) => collection_to_js(ctx, value),
        Value::Bytes(_) => Err(Exception::throw_message(
            ctx,
            "binary values cannot be read by a script",
        )),
        Value::Object(object) => object_to_js(ctx, &object),
    }
}

/// Dates reach a script as text, as Java `toString` would give. Assigning one back stores the text,
/// and the entity layer converts it to the field type on write.
fn temporal_text(value: &Value) -> String {
    let text = moqui_lib::object::basic_convert(value, "String").value;
    match text.as_str() {
        Some(text) => text.to_string(),
        None => moqui_lib::object::to_plain_string(value),
    }
}

fn object_to_js<'js>(ctx: &Ctx<'js>, object: &HostObject) -> rquickjs::Result<JsValue<'js>> {
    if let Some(inner) = object.downcast::<EntityCell>() {
        return Class::instance(ctx.clone(), HostEntity { inner }).and_then(|class| class.into_js(ctx));
    }
    if let Some(inner) = object.downcast::<EntityList>() {
        return Class::instance(ctx.clone(), HostList { inner }).and_then(|class| class.into_js(ctx));
    }
    Err(Exception::throw_message(ctx, "cannot convert this object"))
}

/// Largest integer a JavaScript number holds exactly.
const SAFE_INTEGER: i64 = 9_007_199_254_740_991;

fn i64_to_js<'js>(ctx: &Ctx<'js>, number: i64) -> rquickjs::Result<JsValue<'js>> {
    if let Ok(small) = i32::try_from(number) {
        return small.into_js(ctx);
    }
    if number.abs() > SAFE_INTEGER {
        return Err(Exception::throw_message(
            ctx,
            "integer is too large for a script number; it would lose precision",
        ));
    }
    #[allow(clippy::cast_precision_loss)]
    (number as f64).into_js(ctx)
}

fn js_to_host(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<Value> {
    js_value_to_host(ctx, value).map_err(|err| Exception::throw_message(ctx, err.message()))
}

fn js_truthy(value: &JsValue<'_>) -> bool {
    if value.is_null() || value.is_undefined() {
        return false;
    }
    if let Some(flag) = value.as_bool() {
        return flag;
    }
    if let Some(number) = value.as_number() {
        return number != 0.0 && !number.is_nan();
    }
    if let Some(text) = value.as_string() {
        return text.to_string().ok().is_some_and(|text| !text.is_empty());
    }
    true
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

enum Arith {
    Add,
    Subtract,
    Multiply,
    Divide,
}

fn host_arith(ctx: &Ctx<'_>, left: &Value, right: &Value, op: Arith) -> rquickjs::Result<HostDecimal> {
    host_guard(ctx, || {
        let inner = arith(left, right, op).map_err(|err| throw_lib(ctx, &err))?;
        Ok(HostDecimal { inner })
    })
}

fn arith(left: &Value, right: &Value, op: Arith) -> Result<Value, moqui_lib::error::Error> {
    let Some(left) = left.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    let Some(right) = right.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    let number = match op {
        Arith::Add => moqui_lib::object::decimal_add(left, right)?,
        Arith::Subtract => moqui_lib::object::decimal_subtract(left, right)?,
        Arith::Multiply => moqui_lib::object::decimal_multiply(left, right)?,
        Arith::Divide => moqui_lib::object::decimal_divide(left, right)?,
    };
    Ok(Value::Decimal(number))
}

fn decimals_equal(left: &Value, right: &Value) -> Result<bool, moqui_lib::error::Error> {
    let Some(left) = left.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    let Some(right) = right.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    Ok(moqui_lib::object::decimal_eq(left, right))
}

fn decimals_compare(left: &Value, right: &Value) -> Result<Ordering, moqui_lib::error::Error> {
    let Some(left) = left.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    let Some(right) = right.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    Ok(moqui_lib::object::decimal_compare(left, right))
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
    caught(ctx, "globals", ctx.globals().set("ec", ec))?;
    Ok(())
}

const HELPERS_JS: &str = r#"
(function () {
    function field(item, name) {
        if (item == null) return null;
        if (typeof item.get === "function") return item.get(name);
        return item[name];
    }
    globalThis.orderMapList = function (list, fields) {
        if (list == null) return;
        var specs = fields.map(function (text) {
            var name = String(text), desc = false;
            if (name.charAt(0) === "-") { desc = true; name = name.substring(1); }
            else if (name.charAt(0) === "+") name = name.substring(1);
            return { name: name, desc: desc };
        });
        list.sort(function (a, b) {
            for (var i = 0; i < specs.length; i++) {
                var c = __compareOrder(field(a, specs[i].name), field(b, specs[i].name));
                if (c !== 0) return specs[i].desc ? -c : c;
            }
            return 0;
        });
    };
    globalThis.filterMapList = function (list, values) {
        if (list == null) return;
        for (var i = list.length - 1; i >= 0; i--) {
            for (var key in values) {
                if (!compare(field(list[i], key), "equals", null, values[key], null, "Object")) {
                    list.splice(i, 1);
                    break;
                }
            }
        }
    };
    globalThis.filterMapListByDate = function (list, fromName, thruName, valid, ignoreIfEmpty) {
        if (list == null) return;
        if (ignoreIfEmpty && (valid == null || valid === "")) return;
        for (var i = list.length - 1; i >= 0; i--) {
            if (!__inDateRange(field(list[i], fromName), field(list[i], thruName), valid)) list.splice(i, 1);
        }
    };
})();
"#;

fn install_helpers(ctx: &Ctx<'_>) -> Result<(), Error> {
    let globals = ctx.globals();
    let compare = Function::new(ctx.clone(), js_compare).map_err(js_error)?;
    globals.set("compare", compare).map_err(js_error)?;
    let convert = Function::new(ctx.clone(), js_basic_convert).map_err(js_error)?;
    globals.set("basicConvert", convert).map_err(js_error)?;
    let order = Function::new(ctx.clone(), js_compare_order).map_err(js_error)?;
    globals.set("__compareOrder", order).map_err(js_error)?;
    let in_range = Function::new(ctx.clone(), js_in_date_range).map_err(js_error)?;
    globals.set("__inDateRange", in_range).map_err(js_error)?;
    caught(ctx, "helpers", ctx.eval::<(), _>(HELPERS_JS))
}

/// Java type name of a value, so a string on the other side of a compare converts to it.
fn java_type_of(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "String",
        Value::Bool(_) => "Boolean",
        Value::I32(_) => "Integer",
        Value::I64(_) => "Long",
        Value::F64(_) => "Double",
        Value::Decimal(_) => "BigDecimal",
        Value::Timestamp(_) => "java.sql.Timestamp",
        Value::Date(_) => "java.sql.Date",
        Value::Time(_) => "java.sql.Time",
        _ => "Object",
    }
}

fn js_compare<'js>(
    ctx: Ctx<'js>,
    field: JsValue<'js>,
    operator: String,
    value: JsValue<'js>,
    to_field: JsValue<'js>,
    _format: JsValue<'js>,
    kind: Opt<String>,
) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        let field = js_to_host(&ctx, &field)?;
        let value = js_to_host(&ctx, &value)?;
        let target = if value.is_null() {
            js_to_host(&ctx, &to_field)?
        } else {
            value
        };
        let kind = kind.0.filter(|name| !name.is_empty() && name != "Object");
        let kind = kind.unwrap_or_else(|| java_type_of(&field).to_string());
        moqui_lib::object::compare(&field, &operator, &target, &kind).map_err(|err| throw_lib(&ctx, &err))
    })
}

fn js_basic_convert<'js>(ctx: Ctx<'js>, value: JsValue<'js>, kind: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let host = js_to_host(&ctx, &value)?;
        host_to_js(&ctx, moqui_lib::object::basic_convert(&host, &kind).value)
    })
}

fn js_compare_order<'js>(ctx: Ctx<'js>, left: JsValue<'js>, right: JsValue<'js>) -> rquickjs::Result<i32> {
    host_guard(&ctx, || {
        let left = js_to_host(&ctx, &left)?;
        let right = js_to_host(&ctx, &right)?;
        let ordering = match moqui_lib::object::compare_obj(&left, &right) {
            Ok(ordering) => ordering,
            // A null on the right cannot be ordered, so it sorts after a value.
            Err(_) if right.is_null() => Ordering::Less,
            Err(_) => Ordering::Equal,
        };
        Ok(match ordering {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        })
    })
}

fn timestamp_of(value: &Value) -> Value {
    moqui_lib::object::basic_convert(value, "java.sql.Timestamp").value
}

fn now_timestamp() -> Value {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    timestamp_of(&Value::I64(millis))
}

fn js_in_date_range<'js>(
    ctx: Ctx<'js>,
    from: JsValue<'js>,
    thru: JsValue<'js>,
    valid: JsValue<'js>,
) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        let from = timestamp_of(&js_to_host(&ctx, &from)?);
        let thru = timestamp_of(&js_to_host(&ctx, &thru)?);
        let valid = js_to_host(&ctx, &valid)?;
        let valid = if moqui_lib::object::is_empty(&valid) {
            now_timestamp()
        } else {
            timestamp_of(&valid)
        };
        let started = from.is_null()
            || moqui_lib::object::compare_obj(&from, &valid).is_ok_and(|ordering| ordering != Ordering::Greater);
        let open = thru.is_null()
            || moqui_lib::object::compare_obj(&thru, &valid).is_ok_and(|ordering| ordering == Ordering::Greater);
        Ok(started && open)
    })
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

fn entity_find<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostFind>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let find = facade.find(&name).map_err(|err| throw_lib(&ctx, &err))?;
        wrap_find(&ctx, find)
    })
}

fn entity_make_value<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostEntity>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let value = facade.make_value(&name).map_err(|err| throw_lib(&ctx, &err))?;
        Class::instance(ctx.clone(), HostEntity::new(value))
    })
}

/// A condition node a script passes to `EntityFind.condition`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostCondition {
    #[qjs(skip_trace)]
    inner: Condition,
}

fn condition_js<'js>(ctx: &Ctx<'js>, condition: Option<Condition>) -> rquickjs::Result<JsValue<'js>> {
    match condition {
        Some(inner) => Class::instance(ctx.clone(), HostCondition { inner }).and_then(|class| class.into_js(ctx)),
        None => Null.into_js(ctx),
    }
}

fn condition_of(value: &JsValue<'_>) -> Option<Condition> {
    let object = value.as_object()?;
    let class = object.as_class::<HostCondition>()?;
    let inner = class.borrow().inner.clone();
    Some(inner)
}

fn list_values(value: Value) -> Vec<Value> {
    match value {
        Value::List(items) => items,
        Value::String(text) => text.split(',').map(|part| Value::string(part.trim())).collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// One comparison from an operator name. Equality with null is `IS NULL`, as the Java factory does.
fn comparison(field: &str, operator: &str, value: Value) -> Result<Condition, moqui_lib::error::Error> {
    let condition = match operator {
        "equals" | "=" if value.is_null() => Condition::is_null(field),
        "equals" | "=" => Condition::equals(field, value),
        "not-equals" | "!=" | "<>" if value.is_null() => Condition::is_not_null(field),
        "not-equals" | "!=" | "<>" => Condition::not_equals(field, value),
        "less" | "<" => Condition::less(field, value),
        "greater" | ">" => Condition::greater(field, value),
        "less-equals" | "<=" => Condition::less_equal(field, value),
        "greater-equals" | ">=" => Condition::greater_equal(field, value),
        "like" => Condition::like(field, value),
        "not-like" => Condition::not_like(field, value),
        "in" => Condition::is_in(field, list_values(value)),
        "not-in" => Condition::not_in(field, list_values(value)),
        "between" => {
            let mut bounds = list_values(value).into_iter();
            match (bounds.next(), bounds.next()) {
                (Some(low), Some(high)) => Condition::between(field, low, high),
                _ => return Err(moqui_lib::error::Error::new("between needs a low and a high value")),
            }
        }
        "is-null" => Condition::is_null(field),
        "is-not-null" => Condition::is_not_null(field),
        other => {
            return Err(moqui_lib::error::Error::new(format!(
                "unknown condition operator {other}"
            )))
        }
    };
    Ok(condition)
}

fn join(conditions: Vec<Condition>, operator: &str) -> Option<Condition> {
    let mut conditions = conditions;
    match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ if operator.eq_ignore_ascii_case("or") => Some(Condition::or(conditions)),
        _ => Some(Condition::and(conditions)),
    }
}

/// `makeCondition(field, operator, value)`, `makeCondition(listOfConditions, "and"|"or")`,
/// or `makeCondition(map)` as an AND of equalities.
fn make_condition<'js>(
    ctx: Ctx<'js>,
    first: JsValue<'js>,
    second: Opt<JsValue<'js>>,
    third: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let as_text = |value: &Option<JsValue<'js>>| -> rquickjs::Result<String> {
            match value {
                Some(value) if value.is_string() => js_string(&ctx, value),
                _ => Ok(String::new()),
            }
        };
        if let Some(items) = first.as_array() {
            let mut parts = Vec::new();
            for item in items.iter::<JsValue>() {
                if let Some(condition) = condition_of(&item?) {
                    parts.push(condition);
                }
            }
            let operator = as_text(&second.0)?;
            return condition_js(&ctx, join(parts, &operator));
        }
        if first.is_string() {
            let field = js_string(&ctx, &first)?;
            let operator = as_text(&second.0)?;
            let value = match &third.0 {
                Some(value) => js_to_host(&ctx, value)?,
                None => Value::Null,
            };
            let condition = comparison(&field, &operator, value).map_err(|err| throw_lib(&ctx, &err))?;
            return condition_js(&ctx, Some(condition));
        }
        if let Some(object) = first.as_object() {
            let mut parts = Vec::new();
            for key in object.keys::<String>() {
                let key = key?;
                let property: JsValue = object.get(key.as_str())?;
                let value = js_to_host(&ctx, &property)?;
                parts.push(comparison(&key, "equals", value).map_err(|err| throw_lib(&ctx, &err))?);
            }
            return condition_js(&ctx, join(parts, "and"));
        }
        Err(Exception::throw_message(
            &ctx,
            "makeCondition needs a field, a list, or a map",
        ))
    })
}

fn truthy_text(value: &str) -> bool {
    value.eq_ignore_ascii_case("true")
}

/// From and thru date columns that cover `validDate`, or now: `(from is null or from <= d) and (thru is null or thru > d)`.
fn make_condition_date<'js>(
    ctx: Ctx<'js>,
    from_name: String,
    thru_name: String,
    valid: JsValue<'js>,
    ignore_if_empty: Opt<bool>,
    ignore: Opt<String>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        if ignore.0.as_deref().is_some_and(truthy_text) {
            return Null.into_js(&ctx);
        }
        let valid = js_to_host(&ctx, &valid)?;
        let valid = if moqui_lib::object::is_empty(&valid) {
            if ignore_if_empty.0.unwrap_or(false) {
                return Null.into_js(&ctx);
            }
            now_timestamp()
        } else {
            timestamp_of(&valid)
        };
        let started = Condition::or(vec![
            Condition::is_null(from_name.as_str()),
            Condition::less_equal(from_name.as_str(), valid.clone()),
        ]);
        let open = Condition::or(vec![
            Condition::is_null(thru_name.as_str()),
            Condition::greater(thru_name.as_str(), valid),
        ]);
        condition_js(&ctx, Some(Condition::and(vec![started, open])))
    })
}

/// The condition one `econdition` element describes. A skipped condition is null.
///
/// Arguments: field, operator, from, value, to-field, ignore-case, ignore-if-empty, or-null, ignore.
fn make_action_condition<'js>(ctx: Ctx<'js>, args: Rest<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let arg = |index: usize| {
            args.0
                .get(index)
                .cloned()
                .unwrap_or_else(|| Undefined.into_value(ctx.clone()))
        };
        let flag = |index: usize| arg(index).as_bool().unwrap_or(false);
        let field = js_string(&ctx, &arg(0))?;
        let operator = js_string(&ctx, &arg(1))?;
        if arg(8)
            .as_string()
            .is_some_and(|text| text.to_string().is_ok_and(|text| truthy_text(&text)))
        {
            return Null.into_js(&ctx);
        }
        let to_field = arg(4);
        if !to_field.is_null() && !to_field.is_undefined() {
            return Err(Exception::throw_message(
                &ctx,
                "to-field-name conditions are not supported",
            ));
        }
        if flag(5) {
            return Err(Exception::throw_message(
                &ctx,
                "ignore-case conditions are not supported",
            ));
        }
        let from = js_to_host(&ctx, &arg(2))?;
        let operand = if from.is_null() {
            js_to_host(&ctx, &arg(3))?
        } else {
            from
        };
        if flag(6) && moqui_lib::object::is_empty(&operand) {
            return Null.into_js(&ctx);
        }
        let mut condition = comparison(&field, &operator, operand).map_err(|err| throw_lib(&ctx, &err))?;
        if flag(7) {
            condition = Condition::or(vec![condition, Condition::is_null(field.as_str())]);
        }
        condition_js(&ctx, Some(condition))
    })
}

fn resource_expand(ctx: Ctx<'_>, template: String) -> rquickjs::Result<String> {
    host_guard(&ctx, || {
        expand_here(&ctx, &template).map_err(|err| Exception::throw_message(&ctx, err.message()))
    })
}

fn resource_script(ctx: Ctx<'_>, _location: String) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        Err(Exception::throw_message(&ctx, "resource.script is not implemented"))
    })
}

fn logger_log(ctx: Ctx<'_>, level: String, message: String, _extra: Opt<JsValue<'_>>) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        match level.as_str() {
            "warn" | "warning" | "error" => tracing::warn!("{message}"),
            _ => tracing::info!("{message}"),
        }
        Ok(())
    })
}

fn logger_warn(ctx: Ctx<'_>, message: String) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        tracing::warn!("{message}");
        Ok(())
    })
}

fn service_sync<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
    host_guard(&ctx, || fresh_service(&ctx, false))
}

fn service_async<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
    host_guard(&ctx, || fresh_service(&ctx, true))
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

fn wrap_find<'js>(ctx: &Ctx<'js>, find: EntityFind) -> rquickjs::Result<Class<'js, HostFind>> {
    Class::instance(
        ctx.clone(),
        HostFind {
            inner: RefCell::new(Some(find)),
        },
    )
}

fn take_find(ctx: &Ctx<'_>, slot: &RefCell<Option<EntityFind>>) -> rquickjs::Result<EntityFind> {
    slot.borrow_mut()
        .take()
        .ok_or_else(|| Exception::throw_message(ctx, "entity find was already used"))
}

fn fresh_service<'js>(ctx: &Ctx<'js>, is_async: bool) -> rquickjs::Result<Class<'js, HostService>> {
    Class::instance(
        ctx.clone(),
        HostService {
            call: ServiceCall {
                name: String::new(),
                parameters: std::collections::BTreeMap::new(),
                options: CallOptions::default(),
                is_async,
            },
        },
    )
}

fn not_implemented(ctx: &Ctx<'_>, what: &str) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("{what} is not implemented"))
}

fn value_to_js<'js>(ctx: &Ctx<'js>, value: &Value) -> rquickjs::Result<JsValue<'js>> {
    host_to_js(ctx, value.clone())
}

fn js_string(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<String> {
    let Some(text) = value.as_string() else {
        return Err(Exception::throw_message(ctx, "expected a string"));
    };
    text.to_string()
}

fn js_i64(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<i64> {
    if let Some(number) = value.as_int() {
        return Ok(i64::from(number));
    }
    if value.as_float().is_some() {
        return Err(Exception::throw_message(ctx, "limit is not an integer"));
    }
    Err(Exception::throw_message(ctx, "limit is not a number"))
}

fn prefixed_field(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        return field.to_string();
    }
    let mut name = String::with_capacity(prefix.len() + field.len());
    name.push_str(prefix);
    let mut chars = field.chars();
    if let Some(first) = chars.next() {
        name.push(first.to_ascii_uppercase());
        name.push_str(chars.as_str());
    }
    name
}

enum FieldInclude {
    All,
    Pk,
    NonPk,
}

fn field_include(value: &JsValue<'_>) -> FieldInclude {
    if value.is_null() || value.is_undefined() {
        return FieldInclude::All;
    }
    match value.as_bool() {
        Some(true) => FieldInclude::Pk,
        Some(false) => FieldInclude::NonPk,
        None => FieldInclude::All,
    }
}

fn field_prefix(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<String> {
    if value.is_null() || value.is_undefined() {
        return Ok(String::new());
    }
    js_string(ctx, value)
}

/// A find builder. Chain methods take the inner find so a second use fails.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostFind {
    #[qjs(skip_trace)]
    inner: RefCell<Option<EntityFind>>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostFind {
    fn condition<'js>(
        &self,
        ctx: Ctx<'js>,
        first: JsValue<'js>,
        second: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let mut find = take_find(&ctx, &self.inner)?;
            if second.0.is_none() {
                if first.is_null() || first.is_undefined() {
                    return wrap_find(&ctx, find);
                }
                if let Some(condition) = condition_of(&first) {
                    return wrap_find(&ctx, find.condition(condition));
                }
            }
            if let Some(value) = second.0 {
                let field = js_string(&ctx, &first)?;
                let host = js_to_host(&ctx, &value)?;
                let condition = comparison(&field, "equals", host).map_err(|err| throw_lib(&ctx, &err))?;
                return wrap_find(&ctx, find.condition(condition));
            }
            let Some(object) = first.as_object() else {
                return Err(Exception::throw_message(
                    &ctx,
                    "entity condition needs a field and a value",
                ));
            };
            // A map such as `context` carries names that are not fields. Only fields apply.
            let facade = facade_of(&ctx)?;
            let known: std::collections::BTreeSet<String> = facade
                .with_entity(find.entity_name(), |entity, _, _| {
                    Ok(entity.fields.iter().map(|field| field.name.clone()).collect())
                })
                .map_err(|err| throw_lib(&ctx, &err))?;
            for key in object.keys::<String>() {
                let key = key?;
                if !known.contains(&key) {
                    continue;
                }
                let property: JsValue = object.get(key.as_str())?;
                let host = js_to_host(&ctx, &property)?;
                let condition = comparison(&key, "equals", host).map_err(|err| throw_lib(&ctx, &err))?;
                find = find.condition(condition);
            }
            wrap_find(&ctx, find)
        })
    }

    fn order_by<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let (name, descending) = match field.strip_prefix('-') {
                Some(rest) => (rest.to_string(), true),
                None => (field, false),
            };
            let find = take_find(&ctx, &self.inner)?.order_by(&name, descending);
            wrap_find(&ctx, find)
        })
    }

    fn limit<'js>(&self, ctx: Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let number = js_i64(&ctx, &value)?;
            let find = take_find(&ctx, &self.inner)?.limit(number);
            wrap_find(&ctx, find)
        })
    }

    fn offset<'js>(&self, ctx: Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            let number = js_i64(&ctx, &value)?;
            let find = take_find(&ctx, &self.inner)?.offset(number);
            wrap_find(&ctx, find)
        })
    }

    /// Reads go to the entity cache or the database as `EntityFind` decides, so the cache,
    /// clone, and column-list hints change nothing. A lock or distinct request would change
    /// the result, so those fail rather than run without it.
    fn use_cache<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn for_update<'js>(&self, ctx: Ctx<'js>, flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            if flag.as_bool() == Some(true) {
                return Err(not_implemented(&ctx, "forUpdate"));
            }
            wrap_find(&ctx, take_find(&ctx, &self.inner)?)
        })
    }

    fn distinct<'js>(&self, ctx: Ctx<'js>, flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || {
            if flag.as_bool() == Some(true) {
                return Err(not_implemented(&ctx, "distinct"));
            }
            wrap_find(&ctx, take_find(&ctx, &self.inner)?)
        })
    }

    fn use_clone<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn select_field<'js>(&self, ctx: Ctx<'js>, _field: String) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn list<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let list = take_find(&ctx, &self.inner)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            Class::instance(ctx.clone(), HostList { inner: Arc::new(list) })
        })
    }

    fn one<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let found = take_find(&ctx, &self.inner)?
                .one()
                .map_err(|err| throw_lib(&ctx, &err))?;
            match found {
                Some(value) => {
                    let class = Class::instance(ctx.clone(), HostEntity::new(value))?;
                    class.into_js(&ctx)
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn count<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let number = take_find(&ctx, &self.inner)?
                .count()
                .map_err(|err| throw_lib(&ctx, &err))?;
            i64_to_js(&ctx, number)
        })
    }

    fn delete_all(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            let list = take_find(&ctx, &self.inner)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            for value in list.values() {
                value.delete(&facade).map_err(|err| throw_lib(&ctx, &err))?;
            }
            Ok(())
        })
    }

    /// A script reads the whole result. A streaming cursor is the host's `EntityFind::iterator`.
    fn iterator<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostList>> {
        self.list(ctx)
    }

    fn having_condition(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "havingCondition")))
    }

    fn search_form_map(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "searchFormMap")))
    }

    fn require_search_form_parameters(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "requireSearchFormParameters")))
    }
}

/// A find result. `length` and `get` are what iterate and inline scripts use.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostList {
    #[qjs(skip_trace)]
    inner: Arc<EntityList>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostList {
    #[qjs(get)]
    fn length(&self, ctx: Ctx<'_>) -> rquickjs::Result<i32> {
        host_guard(&ctx, || {
            i32::try_from(self.inner.len()).map_err(|_| Exception::throw_message(&ctx, "list is too long"))
        })
    }

    fn get<'js>(&self, ctx: Ctx<'js>, index: i32) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            if index < 0 {
                return Null.into_js(&ctx);
            }
            let index =
                usize::try_from(index).map_err(|_| Exception::throw_message(&ctx, "list index is too large"))?;
            match self.inner.get(index) {
                Some(value) => {
                    let class = Class::instance(ctx.clone(), HostEntity::new(value.clone()))?;
                    class.into_js(&ctx)
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn get_partial_list<'js>(
        &self,
        ctx: Ctx<'js>,
        offset: i32,
        size: i32,
        _count: Opt<bool>,
    ) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let start = usize::try_from(offset.max(0)).unwrap_or(0);
            let take = usize::try_from(size.max(0)).unwrap_or(0);
            let values = self.inner.values().iter().skip(start).take(take).cloned().collect();
            Class::instance(
                ctx.clone(),
                HostList {
                    inner: Arc::new(EntityList::new(values)),
                },
            )
        })
    }

    fn filter_by_date<'js>(
        &self,
        ctx: Ctx<'js>,
        from_name: String,
        thru_name: String,
        valid: JsValue<'js>,
        ignore_if_empty: Opt<bool>,
    ) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            let valid = js_to_host(&ctx, &valid)?;
            let empty = moqui_lib::object::is_empty(&valid);
            let values = if empty && ignore_if_empty.0.unwrap_or(false) {
                self.inner.values().to_vec()
            } else {
                let valid = if empty { now_timestamp() } else { timestamp_of(&valid) };
                self.inner
                    .values()
                    .iter()
                    .filter(|row| {
                        let from = timestamp_of(row.get(&from_name).unwrap_or(&Value::Null));
                        let thru = timestamp_of(row.get(&thru_name).unwrap_or(&Value::Null));
                        let started = from.is_null()
                            || moqui_lib::object::compare_obj(&from, &valid)
                                .is_ok_and(|ordering| ordering != Ordering::Greater);
                        let open = thru.is_null()
                            || moqui_lib::object::compare_obj(&thru, &valid)
                                .is_ok_and(|ordering| ordering == Ordering::Greater);
                        started && open
                    })
                    .cloned()
                    .collect()
            };
            Class::instance(
                ctx.clone(),
                HostList {
                    inner: Arc::new(EntityList::new(values)),
                },
            )
        })
    }
}

/// One entity value. Field storage stays on this object, not on the context stack.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostEntity {
    #[qjs(skip_trace)]
    inner: Arc<EntityCell>,
}

/// An entity value a script and the context stack share by reference.
type EntityCell = Mutex<EntityValue>;

impl HostEntity {
    fn new(value: EntityValue) -> Self {
        Self {
            inner: Arc::new(Mutex::new(value)),
        }
    }

    fn val(&self) -> MutexGuard<'_, EntityValue> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostEntity {
    fn get<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match self.val().get(&field) {
            Some(value) => value_to_js(&ctx, value),
            None => Null.into_js(&ctx),
        })
    }

    fn set(&self, ctx: Ctx<'_>, field: String, value: JsValue<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let host = js_to_host(&ctx, &value)?;
            self.val().set(field, host);
            Ok(())
        })
    }

    fn set_fields(
        &self,
        ctx: Ctx<'_>,
        map: JsValue<'_>,
        set_if_empty: Opt<bool>,
        prefix: JsValue<'_>,
        include: JsValue<'_>,
    ) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let set_empty = set_if_empty.0.unwrap_or(false);
            let prefix = field_prefix(&ctx, &prefix)?;
            let include = field_include(&include);
            let Some(object) = map.as_object() else {
                return Err(Exception::throw_message(&ctx, "setFields needs an object"));
            };
            let facade = facade_of(&ctx)?;
            let entity_name = self.val().entity_name().to_string();
            let specs = facade
                .with_entity(&entity_name, |entity, _, _| {
                    Ok(entity
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.is_pk))
                        .collect::<Vec<_>>())
                })
                .map_err(|err| throw_lib(&ctx, &err))?;
            for (name, is_pk) in specs {
                let keep = match include {
                    FieldInclude::All => true,
                    FieldInclude::Pk => is_pk,
                    FieldInclude::NonPk => !is_pk,
                };
                if !keep {
                    continue;
                }
                let key = prefixed_field(&prefix, &name);
                if !object.contains_key(key.as_str())? {
                    continue;
                }
                let property: JsValue = object.get(key.as_str())?;
                let host = js_to_host(&ctx, &property)?;
                let empty = host.is_null() || matches!(&host, Value::String(text) if text.is_empty());
                if !set_empty && empty {
                    continue;
                }
                self.val().set(name, host);
            }
            Ok(())
        })
    }

    fn create(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val().create(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn update(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val().update(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn delete(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val().delete(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn create_or_update(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val()
                .create_or_update(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn set_sequenced_id_primary(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val()
                .set_sequenced_id_primary(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn set_sequenced_id_secondary(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.val()
                .set_sequenced_id_secondary(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn find_related<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        map: Opt<JsValue<'js>>,
        order: Opt<JsValue<'js>>,
        _cache: Opt<JsValue<'js>>,
        for_update: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<Class<'js, HostList>> {
        host_guard(&ctx, || {
            if for_update.0.as_ref().and_then(JsValue::as_bool) == Some(true) {
                return Err(not_implemented(&ctx, "forUpdate"));
            }
            let mut find = self.related_find(&ctx, &name)?;
            if let Some(Value::Map(entries)) = map.0.as_ref().map(|value| js_to_host(&ctx, value)).transpose()? {
                for (key, value) in entries {
                    find = find.eq(&key, value);
                }
            }
            if let Some(Value::List(fields)) = order.0.as_ref().map(|value| js_to_host(&ctx, value)).transpose()? {
                for field in fields {
                    let text = moqui_lib::object::to_plain_string(&field);
                    match text.strip_prefix('-') {
                        Some(rest) => find = find.order_by(rest, true),
                        None => find = find.order_by(text.trim_start_matches('+'), false),
                    }
                }
            }
            let list = find.list().map_err(|err| throw_lib(&ctx, &err))?;
            Class::instance(ctx.clone(), HostList { inner: Arc::new(list) })
        })
    }

    fn find_related_one<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        _cache: Opt<JsValue<'js>>,
        for_update: Opt<JsValue<'js>>,
    ) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            if for_update.0.as_ref().and_then(JsValue::as_bool) == Some(true) {
                return Err(not_implemented(&ctx, "forUpdate"));
            }
            let found = self
                .related_find(&ctx, &name)?
                .one()
                .map_err(|err| throw_lib(&ctx, &err))?;
            match found {
                Some(value) => {
                    Class::instance(ctx.clone(), HostEntity::new(value)).and_then(|class| class.into_js(&ctx))
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn delete_related(&self, ctx: Ctx<'_>, name: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            let list = self
                .related_find(&ctx, &name)?
                .list()
                .map_err(|err| throw_lib(&ctx, &err))?;
            for value in list.values() {
                value.delete(&facade).map_err(|err| throw_lib(&ctx, &err))?;
            }
            Ok(())
        })
    }
}

impl HostEntity {
    fn related_find(&self, ctx: &Ctx<'_>, relationship: &str) -> rquickjs::Result<EntityFind> {
        let facade = facade_of(ctx)?;
        let record = self.val().clone();
        let (entity, conditions) =
            crate::service::call::related_conditions(&facade, record.entity_name(), relationship, &record)
                .map_err(|err| throw_lib(ctx, &err))?;
        let mut find = facade.find(&entity).map_err(|err| throw_lib(ctx, &err))?;
        for (field, value) in conditions {
            find = find.eq(&field, value);
        }
        Ok(find)
    }
}

/// Message facade for the entered execution context. There is no separate public-message list.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostMessage {}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostMessage {
    fn add_error(&self, ctx: Ctx<'_>, message: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            with_ec(&ctx, |ec| ec.message().add_error(message))?;
            Ok(())
        })
    }

    fn add_message(&self, ctx: Ctx<'_>, message: String, _kind: Opt<String>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            with_ec(&ctx, |ec| ec.message().add_message(message))?;
            Ok(())
        })
    }

    fn add_public(&self, ctx: Ctx<'_>, message: String, _kind: Opt<String>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            with_ec(&ctx, |ec| ec.message().add_message(message))?;
            Ok(())
        })
    }

    fn has_error(&self, ctx: Ctx<'_>) -> rquickjs::Result<bool> {
        host_guard(&ctx, || with_ec(&ctx, |ec| !ec.message().errors().is_empty()))
    }

    fn clear_errors(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            with_ec(&ctx, |ec| ec.message().clear())?;
            Ok(())
        })
    }

    fn get_errors_string(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || with_ec(&ctx, |ec| ec.message().errors().join("\n")))
    }

    #[qjs(get)]
    fn errors<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let count = with_ec(&ctx, |ec| ec.message().errors().len())?;
            if count == 0 {
                return Null.into_js(&ctx);
            }
            let count = i32::try_from(count).map_err(|_| Exception::throw_message(&ctx, "error count is too large"))?;
            count.into_js(&ctx)
        })
    }
}

#[derive(Clone)]
struct ServiceCall {
    name: String,
    parameters: std::collections::BTreeMap<String, Value>,
    options: CallOptions,
    is_async: bool,
}

/// Service call builder. Each method returns a new builder. There is no `distribute` method.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostService {
    #[qjs(skip_trace)]
    call: ServiceCall,
}

impl HostService {
    fn changed<'js>(
        &self,
        ctx: &Ctx<'js>,
        change: impl FnOnce(&mut ServiceCall) -> rquickjs::Result<()>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        let mut call = self.call.clone();
        change(&mut call)?;
        Class::instance(ctx.clone(), HostService { call })
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostService {
    fn name<'js>(&self, ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            self.changed(&ctx, |call| {
                call.name = name;
                Ok(())
            })
        })
    }

    fn disable_authz<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            self.changed(&ctx, |call| {
                call.options.disable_authz = true;
                Ok(())
            })
        })
    }

    fn multi<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            self.changed(&ctx, |_| Err(not_implemented(&ctx, "multi service call")))
        })
    }

    fn ignore_transaction<'js>(&self, ctx: Ctx<'js>, flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            self.changed(&ctx, |call| {
                call.options.ignore_transaction = flag.as_bool();
                Ok(())
            })
        })
    }

    fn require_new_transaction<'js>(
        &self,
        ctx: Ctx<'js>,
        flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            self.changed(&ctx, |call| {
                call.options.require_new_transaction = flag.as_bool();
                Ok(())
            })
        })
    }

    fn transaction_timeout<'js>(
        &self,
        ctx: Ctx<'js>,
        seconds: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            let seconds = js_i64(&ctx, &seconds)?;
            self.changed(&ctx, |call| {
                call.options.timeout = u64::try_from(seconds).ok().map(std::time::Duration::from_secs);
                Ok(())
            })
        })
    }

    fn use_transaction_cache<'js>(
        &self,
        ctx: Ctx<'js>,
        flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            self.changed(&ctx, |call| {
                call.options.use_transaction_cache = flag.as_bool();
                Ok(())
            })
        })
    }

    fn parameters<'js>(&self, ctx: Ctx<'js>, map: JsValue<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            let given = js_to_host(&ctx, &map)?;
            self.changed(&ctx, |call| {
                if let Value::Map(entries) = given {
                    call.parameters.extend(entries);
                }
                Ok(())
            })
        })
    }

    fn parameter<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        value: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            let value = js_to_host(&ctx, &value)?;
            self.changed(&ctx, |call| {
                call.parameters.insert(name, value);
                Ok(())
            })
        })
    }

    /// Run the service. A failure is added to the message facade and the result is null, so the
    /// generated code's `hasError()` check decides, as in the Java call builder.
    fn call<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            if self.call.is_async {
                return Err(not_implemented(&ctx, "async service call"));
            }
            // The callee may read or change the caller's names, so they are written back first.
            flush_current(&ctx, true)?;
            match crate::service::call::sync_with(&self.call.name, &self.call.parameters, &self.call.options) {
                Ok(results) => host_to_js(&ctx, Value::Map(results.into_iter().collect())),
                Err(err) => {
                    let message = err.message().to_string();
                    with_ec(&ctx, |ec| ec.message().add_error(message))?;
                    Null.into_js(&ctx)
                }
            }
        })
    }
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

fn collection_to_js<'js>(ctx: &Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::List(items) => {
            let array = rquickjs::Array::new(ctx.clone())?;
            for (index, item) in items.into_iter().enumerate() {
                array.set(index, host_to_js(ctx, item)?)?;
            }
            Ok(array.into_value())
        }
        Value::Map(entries) => {
            let object = Object::new(ctx.clone())?;
            for (key, item) in entries {
                object.set(key, host_to_js(ctx, item)?)?;
            }
            Ok(object.into_value())
        }
        other => host_to_js(ctx, other),
    }
}

fn object_map(ctx: &Ctx<'_>, value: &JsValue<'_>) -> Result<std::collections::BTreeMap<String, Value>, Error> {
    if !value.is_object() || value.is_array() || value.is_function() {
        return Err(Error::new("script value is not an object"));
    }
    match js_value_to_host(ctx, value)? {
        Value::Map(map) => Ok(map.into_iter().collect()),
        _ => Err(Error::new("script value is not an object")),
    }
}

fn js_value_to_host(ctx: &Ctx<'_>, value: &JsValue<'_>) -> Result<Value, Error> {
    if value.is_null() || value.is_undefined() {
        return Ok(Value::Null);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(Value::Bool(flag));
    }
    if let Some(number) = value.as_int() {
        return Ok(Value::I32(number));
    }
    if let Some(number) = value.as_float() {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        if number.fract() == 0.0 && number.abs() <= SAFE_INTEGER as f64 {
            return Ok(Value::I64(number as i64));
        }
        return Ok(Value::F64(number));
    }
    if let Some(text) = value.as_string() {
        let text = text.to_string().map_err(|err| js_caught(ctx, err))?;
        return Ok(Value::String(text));
    }
    if let Some(array) = value.as_array() {
        let mut items = Vec::with_capacity(array.len());
        for item in array.iter::<JsValue>() {
            let item = item.map_err(|err| js_caught(ctx, err))?;
            items.push(js_value_to_host(ctx, &item)?);
        }
        return Ok(Value::List(items));
    }
    if value.is_function() {
        return Err(Error::new("cannot convert this value"));
    }
    if let Some(object) = value.as_object() {
        if let Some(class) = object.as_class::<HostDecimal>() {
            return Ok(class.borrow().inner.clone());
        }
        if let Some(class) = object.as_class::<HostEntity>() {
            return Ok(Value::Object(HostObject(class.borrow().inner.clone())));
        }
        if let Some(class) = object.as_class::<HostList>() {
            return Ok(Value::Object(HostObject(class.borrow().inner.clone())));
        }
        let mut map = indexmap::IndexMap::new();
        for entry in object.props::<String, JsValue>() {
            let (key, item) = entry.map_err(|err| js_caught(ctx, err))?;
            map.insert(key, js_value_to_host(ctx, &item)?);
        }
        return Ok(Value::Map(map));
    }
    Err(Error::new("cannot convert this value"))
}

fn js_caught(ctx: &Ctx<'_>, err: JsError) -> Error {
    Error::new(rquickjs::CaughtError::from_error(ctx, err).to_string())
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
