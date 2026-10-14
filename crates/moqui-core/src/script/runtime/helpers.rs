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

//! Global helper functions: `compare`, `basicConvert`, list ordering and filtering.

use super::*;

pub(super) const HELPERS_JS: &str = r#"
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

pub(super) fn install_helpers(ctx: &Ctx<'_>) -> Result<(), Error> {
    let globals = ctx.globals();
    let compare = Function::new(ctx.clone(), js_compare).map_err(js_error)?;
    globals.set("compare", compare).map_err(js_error)?;
    let convert = Function::new(ctx.clone(), js_basic_convert).map_err(js_error)?;
    globals.set("basicConvert", convert).map_err(js_error)?;
    let to_decimal = Function::new(ctx.clone(), js_to_decimal).map_err(js_error)?;
    globals.set("toDecimal", to_decimal).map_err(js_error)?;
    let order = Function::new(ctx.clone(), js_compare_order).map_err(js_error)?;
    globals.set("__compareOrder", order).map_err(js_error)?;
    let in_range = Function::new(ctx.clone(), js_in_date_range).map_err(js_error)?;
    globals.set("__inDateRange", in_range).map_err(js_error)?;
    caught(ctx, "helpers", ctx.eval::<(), _>(HELPERS_JS))
}

/// Java type name of a value, so a string on the other side of a compare converts to it.
pub(super) fn java_type_of(value: &Value) -> &'static str {
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

pub(super) fn js_compare<'js>(
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

pub(super) fn js_basic_convert<'js>(
    ctx: Ctx<'js>,
    value: JsValue<'js>,
    kind: String,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let host = js_to_host(&ctx, &value)?;
        host_to_js(&ctx, moqui_lib::object::basic_convert(&host, &kind).value)
    })
}

/// `toDecimal(x)`: a decimal from a decimal, a number or numeric text, else an error.
pub(super) fn js_to_decimal<'js>(ctx: Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let host = js_to_host(&ctx, &value)?;
        match super::decimal::decimal_of(&host) {
            Some(decimal) => host_to_js(&ctx, decimal),
            None => Err(Exception::throw_message(
                &ctx,
                &format!(
                    "toDecimal takes a decimal, a number or numeric text, not {}",
                    moqui_lib::object::to_plain_string(&host)
                ),
            )),
        }
    })
}

pub(super) fn js_compare_order<'js>(ctx: Ctx<'js>, left: JsValue<'js>, right: JsValue<'js>) -> rquickjs::Result<i32> {
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

pub(super) fn timestamp_of(value: &Value) -> Value {
    moqui_lib::object::basic_convert(value, "java.sql.Timestamp").value
}

pub(super) fn now_timestamp() -> Value {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    timestamp_of(&Value::I64(millis))
}

pub(super) fn js_in_date_range<'js>(
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
