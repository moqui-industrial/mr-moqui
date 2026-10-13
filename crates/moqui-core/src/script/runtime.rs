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

//! QuickJS evaluation for server expressions.
//!
//! One runtime per thread. Bare names resolve through a proxy on `globalThis`
//! that reads the thread-local context stack.

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
use rquickjs::function::This;
use rquickjs::proxy::{Proxy, ProxyHandler, ProxyProperty, ProxyReceiver, ProxyTarget};
use rquickjs::Error as JsError;
use rquickjs::{
    CatchResultExt, Class, Context, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime, Null, Object, Persistent,
    Runtime, Undefined, Value as JsValue,
};

use crate::context::Error;

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
///
/// List and map context values are installed as own properties for the call.
/// The proxy cannot convert those, and an own property is read first.
pub fn eval_map(expression: &str) -> Result<std::collections::BTreeMap<String, Value>, Error> {
    if expression.as_bytes().contains(&0) {
        return Err(Error::new("script source contains a null byte"));
    }
    with_context(|ctx| {
        let bound = bind_collections(ctx, expression)?;
        let evaluated = eval_js::<JsValue>(ctx, expression);
        unbind_collections(ctx, &bound);
        object_map(ctx, &evaluated?)
    })
}

fn bind_collections(ctx: &Ctx<'_>, source: &str) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    for name in expression_names(source) {
        if names.iter().any(|existing| existing == &name) || reserved_global(&name) {
            continue;
        }
        let Some(value) = context_value(&name) else {
            continue;
        };
        if !matches!(value, Value::List(_) | Value::Map(_)) {
            continue;
        }
        let owned = global_has_own(ctx, &name).map_err(|err| js_caught(ctx, err))?;
        if owned {
            continue;
        }
        let js = collection_to_js(ctx, value).map_err(|err| js_caught(ctx, err))?;
        // `set` walks the prototype proxy, which cannot store a list. Define an own property.
        let defined = ctx.globals().prop(
            name.as_str(),
            rquickjs::object::Property::from(js).writable().configurable(),
        );
        if let Err(err) = defined {
            unbind_collections(ctx, &names);
            return Err(js_caught(ctx, err));
        }
        names.push(name);
    }
    Ok(names)
}

fn unbind_collections(ctx: &Ctx<'_>, names: &[String]) {
    for name in names {
        let _ = ctx.globals().remove(name.as_str());
    }
}

fn reserved_global(name: &str) -> bool {
    matches!(
        name,
        "Object"
            | "Array"
            | "undefined"
            | "NaN"
            | "Infinity"
            | "globalThis"
            | "Math"
            | "JSON"
            | "Number"
            | "String"
            | "Boolean"
    )
}

fn expression_names(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut names = Vec::new();
    let mut index = 0;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(open) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == open {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' || byte == b'"' || byte == b'`' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if is_name_start(byte) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_name_continue(bytes[index]) {
                index += 1;
            }
            if !name_is_member(source, start) {
                names.push(source[start..index].to_string());
            }
            continue;
        }
        index += 1;
    }
    names
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn is_name_continue(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
}

fn name_is_member(source: &str, start: usize) -> bool {
    source[..start].trim_end().ends_with('.')
}

fn collection_to_js<'js>(ctx: &Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::List(items) => {
            let array = rquickjs::Array::new(ctx.clone())?;
            for (index, item) in items.into_iter().enumerate() {
                array.set(index, host_any_to_js(ctx, item)?)?;
            }
            Ok(array.into_value())
        }
        Value::Map(entries) => {
            let object = Object::new(ctx.clone())?;
            for (key, item) in entries {
                object.set(key, host_any_to_js(ctx, item)?)?;
            }
            Ok(object.into_value())
        }
        other => host_to_js(ctx, other),
    }
}

fn host_any_to_js<'js>(ctx: &Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    match value {
        Value::List(_) | Value::Map(_) => collection_to_js(ctx, value),
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
        let mut map = match Value::Map(Default::default()) {
            Value::Map(map) => map,
            _ => return Err(Error::new("cannot convert this value")),
        };
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
