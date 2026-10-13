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

//! `ec.entity.conditionFactory`: conditions a script builds for an entity find.

use super::*;

/// A condition node a script passes to `EntityFind.condition`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostCondition {
    #[qjs(skip_trace)]
    pub(super) inner: Condition,
}

pub(super) fn condition_js<'js>(ctx: &Ctx<'js>, condition: Option<Condition>) -> rquickjs::Result<JsValue<'js>> {
    match condition {
        Some(inner) => Class::instance(ctx.clone(), HostCondition { inner }).and_then(|class| class.into_js(ctx)),
        None => Null.into_js(ctx),
    }
}

pub(super) fn condition_of(value: &JsValue<'_>) -> Option<Condition> {
    let object = value.as_object()?;
    let class = object.as_class::<HostCondition>()?;
    let inner = class.borrow().inner.clone();
    Some(inner)
}

pub(super) fn list_values(value: Value) -> Vec<Value> {
    match value {
        Value::List(items) => items,
        Value::String(text) => text.split(',').map(|part| Value::string(part.trim())).collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// One comparison from an operator name. Equality with null is `IS NULL`, as the Java factory does.
pub(super) fn comparison(field: &str, operator: &str, value: Value) -> Result<Condition, moqui_lib::error::Error> {
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

pub(super) fn join(conditions: Vec<Condition>, operator: &str) -> Option<Condition> {
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
pub(super) fn make_condition<'js>(
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

pub(super) fn truthy_text(value: &str) -> bool {
    value.eq_ignore_ascii_case("true")
}

/// From and thru date columns that cover `validDate`, or now: `(from is null or from <= d) and (thru is null or thru > d)`.
pub(super) fn make_condition_date<'js>(
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
pub(super) fn make_action_condition<'js>(ctx: Ctx<'js>, args: Rest<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
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
