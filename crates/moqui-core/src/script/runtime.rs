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

// Stack and template entry points stay `pub` for a later re-export. Until then
// a lib build does not call them, so their helpers look unused.
#![allow(dead_code)]

use std::any::Any;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;

use moqui_lib::context_stack::{ContextStack, Lookup};
use moqui_lib::object::Value;
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
use crate::entity::{EntityFacade, EntityFind, EntityList, EntityValue};

const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const VALUE_OF_MESSAGE: &str =
    "decimal valueOf is not supported; use add, subtract, multiply, divide, eq, compare, or isZero";

thread_local! {
    static STACK: RefCell<ContextStack> = RefCell::new(ContextStack::new());
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

/// Persistents drop before the context, and the context drops before the runtime.
struct Engine {
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

fn with_context<R>(body: impl FnOnce(&Ctx) -> Result<R, Error>) -> Result<R, Error> {
    ensure_engine()?;
    ENGINE.with(|slot| {
        let engine = slot.borrow();
        let engine = engine.as_ref().ok_or_else(|| Error::new("quickjs runtime missing"))?;
        engine.context.with(|ctx| body(&ctx))
    })
}

fn ensure_engine() -> Result<(), Error> {
    ENGINE.with(|slot| {
        if slot.borrow().is_some() {
            return Ok(());
        }
        let engine = Engine::start()?;
        *slot.borrow_mut() = Some(engine);
        Ok(())
    })
}

impl Engine {
    fn start() -> Result<Self, Error> {
        let runtime = Runtime::new().map_err(js_error)?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        // No public path reaches TransactionManager::poll_deadline: transactions_for
        // needs an entity name, and the facade does not list groups.
        runtime.set_interrupt_handler(Some(Box::new(|| false)));
        let context = Context::full(&runtime).map_err(js_error)?;
        let helpers = context.with(|ctx| install_context(&ctx))?;
        Ok(Self {
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
    if property.is_symbol() {
        return Undefined.into_js(&ctx);
    }
    let name = property.to_string()?;
    match stack_get(&name) {
        Lookup::Value(value) => host_to_js(&ctx, value),
        Lookup::Context | Lookup::Missing => match helper_object(&ctx, &name)? {
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
    if property.is_symbol() {
        return Ok(true);
    }
    let ctx = target.0.ctx().clone();
    let name = property.to_string()?;
    let host = js_to_host(&ctx, &value)?;
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
    let persistent = ENGINE.with(|slot| {
        let engine = slot.borrow();
        let engine = engine.as_ref()?;
        Some(match kind {
            HelperKind::Object => engine.object_utilities.clone(),
            HelperKind::Collection => engine.collection_utilities.clone(),
            HelperKind::String => engine.string_utilities.clone(),
        })
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
        Value::Timestamp(_) | Value::Date(_) | Value::Time(_) | Value::Bytes(_) | Value::List(_) | Value::Map(_) => {
            Err(Exception::throw_message(ctx, "cannot convert this value"))
        }
    }
}

fn i64_to_js<'js>(ctx: &Ctx<'js>, number: i64) -> rquickjs::Result<JsValue<'js>> {
    if let Ok(small) = i32::try_from(number) {
        return small.into_js(ctx);
    }
    #[allow(clippy::cast_precision_loss)]
    (number as f64).into_js(ctx)
}

fn js_to_host(ctx: &Ctx<'_>, value: &JsValue<'_>) -> rquickjs::Result<Value> {
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
        return Ok(Value::F64(number));
    }
    if let Some(text) = value.as_string() {
        return Ok(Value::String(text.to_string()?));
    }
    if let Some(object) = value.as_object() {
        if let Some(class) = object.as_class::<HostDecimal>() {
            return Ok(class.borrow().inner.clone());
        }
    }
    Err(Exception::throw_message(ctx, "cannot store this value"))
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
    let persistent = ENGINE.with(|slot| {
        let engine = slot.borrow();
        let engine = engine.as_ref()?;
        let guard = engine.templates.borrow();
        guard.get(template).cloned()
    });
    let Some(persistent) = persistent else {
        return Ok(None);
    };
    persistent.restore(ctx).map(Some).map_err(js_error)
}

fn save_template<'js>(ctx: &Ctx<'js>, template: &str, function: &Function<'js>) {
    let saved = Persistent::save(ctx, function.clone());
    ENGINE.with(|slot| {
        let engine = slot.borrow();
        let Some(engine) = engine.as_ref() else {
            return;
        };
        engine.templates.borrow_mut().insert(template.to_string(), saved);
    });
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
    let make_condition = Function::new(ctx.clone(), condition_unimplemented).map_err(js_error)?;
    factory.set("makeCondition", make_condition).map_err(js_error)?;
    let make_date = Function::new(ctx.clone(), condition_unimplemented).map_err(js_error)?;
    factory.set("makeConditionDate", make_date).map_err(js_error)?;
    let make_direct = Function::new(ctx.clone(), condition_unimplemented).map_err(js_error)?;
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
        Class::instance(
            ctx.clone(),
            HostEntity {
                inner: RefCell::new(value),
            },
        )
    })
}

fn condition_unimplemented(ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        Err(Exception::throw_message(&ctx, "conditionFactory is not implemented"))
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
    host_guard(&ctx, || fresh_service(&ctx))
}

fn service_async<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
    host_guard(&ctx, || fresh_service(&ctx))
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

fn fresh_service<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
    Class::instance(
        ctx.clone(),
        HostService {
            call: ServiceCall { name: String::new() },
        },
    )
}

fn not_implemented(ctx: &Ctx<'_>, what: &str) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("{what} is not implemented"))
}

fn value_to_js<'js>(ctx: &Ctx<'js>, value: &Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::Timestamp(_) | Value::Date(_) | Value::Time(_) | Value::Bytes(_) => {
            moqui_lib::object::to_plain_string(value).into_js(ctx)
        }
        Value::List(_) | Value::Map(_) => Err(Exception::throw_message(ctx, "cannot convert this value")),
        other => host_to_js(ctx, other.clone()),
    }
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
            if let Some(value) = second.0 {
                let field = js_string(&ctx, &first)?;
                let host = js_to_host(&ctx, &value)?;
                find = find.eq(&field, host);
                return wrap_find(&ctx, find);
            }
            let Some(object) = first.as_object() else {
                return Err(Exception::throw_message(
                    &ctx,
                    "entity condition needs a field and a value",
                ));
            };
            for key in object.keys::<String>() {
                let key = key?;
                let property: JsValue = object.get(key.as_str())?;
                let host = js_to_host(&ctx, &property)?;
                find = find.eq(&key, host);
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

    /// EntityFind has no cache, lock, distinct, clone, or column list. The chain keeps this find.
    fn use_cache<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn for_update<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
    }

    fn distinct<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostFind>> {
        host_guard(&ctx, || wrap_find(&ctx, take_find(&ctx, &self.inner)?))
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
            Class::instance(ctx.clone(), HostList { inner: list })
        })
    }

    fn one<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let found = take_find(&ctx, &self.inner)?
                .one()
                .map_err(|err| throw_lib(&ctx, &err))?;
            match found {
                Some(value) => {
                    let class = Class::instance(
                        ctx.clone(),
                        HostEntity {
                            inner: RefCell::new(value),
                        },
                    )?;
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

    fn iterator(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "iterator")))
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
    inner: EntityList,
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
                    let class = Class::instance(
                        ctx.clone(),
                        HostEntity {
                            inner: RefCell::new(value.clone()),
                        },
                    )?;
                    class.into_js(&ctx)
                }
                None => Null.into_js(&ctx),
            }
        })
    }

    fn filter_by_date(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "filterByDate")))
    }
}

/// One entity value. Field storage stays on this object, not on the context stack.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostEntity {
    #[qjs(skip_trace)]
    inner: RefCell<EntityValue>,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostEntity {
    fn get<'js>(&self, ctx: Ctx<'js>, field: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match self.inner.borrow().get(&field) {
            Some(value) => value_to_js(&ctx, value),
            None => Null.into_js(&ctx),
        })
    }

    fn set(&self, ctx: Ctx<'_>, field: String, value: JsValue<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let host = js_to_host(&ctx, &value)?;
            self.inner.borrow_mut().set(field, host);
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
            let entity_name = self.inner.borrow().entity_name().to_string();
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
                self.inner.borrow_mut().set(name, host);
            }
            Ok(())
        })
    }

    fn create(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.inner.borrow().create(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn update(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.inner.borrow().update(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn delete(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.inner.borrow().delete(&facade).map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn create_or_update(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.inner
                .borrow()
                .create_or_update(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn set_sequenced_id_primary(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.inner
                .borrow_mut()
                .set_sequenced_id_primary(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn set_sequenced_id_secondary(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let facade = facade_of(&ctx)?;
            self.inner
                .borrow_mut()
                .set_sequenced_id_secondary(&facade)
                .map_err(|err| throw_lib(&ctx, &err))
        })
    }

    fn find_related(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "findRelated")))
    }

    fn find_related_one(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "findRelatedOne")))
    }

    fn delete_related(&self, ctx: Ctx<'_>, _args: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
        host_guard(&ctx, || Err(not_implemented(&ctx, "deleteRelated")))
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
}

/// Service call builder. `call` does not touch `ServiceFacade`. There is no `distribute` method.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct HostService {
    #[qjs(skip_trace)]
    call: ServiceCall,
}

impl HostService {
    fn duplicate<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        Class::instance(
            ctx.clone(),
            HostService {
                call: self.call.clone(),
            },
        )
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostService {
    fn name<'js>(&self, ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || {
            let mut call = self.call.clone();
            call.name = name;
            Class::instance(ctx.clone(), HostService { call })
        })
    }

    fn disable_authz<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn multi<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn ignore_transaction<'js>(&self, ctx: Ctx<'js>, _flag: JsValue<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn require_new_transaction<'js>(
        &self,
        ctx: Ctx<'js>,
        _flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn transaction_timeout<'js>(
        &self,
        ctx: Ctx<'js>,
        _seconds: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn use_transaction_cache<'js>(
        &self,
        ctx: Ctx<'js>,
        _flag: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn parameters<'js>(&self, ctx: Ctx<'js>, _map: JsValue<'js>) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn parameter<'js>(
        &self,
        ctx: Ctx<'js>,
        _name: String,
        _value: JsValue<'js>,
    ) -> rquickjs::Result<Class<'js, HostService>> {
        host_guard(&ctx, || self.duplicate(&ctx))
    }

    fn call(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let _ = &self.call.name;
            Err(Exception::throw_message(
                &ctx,
                "service call is not executed by the XML action host",
            ))
        })
    }
}

fn js_error(err: impl ToString) -> Error {
    Error::new(err.to_string())
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
