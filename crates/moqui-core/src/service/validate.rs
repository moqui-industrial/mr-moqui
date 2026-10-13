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

//! In-parameter checks: required, matches, ranges, and entity existence.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use moqui_lib::object::{
    basic_convert, decimal_compare, decimal_eq, is_empty, regex_full_match, to_plain_string, Value,
};

use super::define::{AutoParams, Check, Include, ParamDef, Required, ServiceDef, ServiceKind};
use crate::context::Error;
use crate::entity::EntityFacade;

pub fn validate(name: &str, parameters: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>, Error> {
    let def = super::define::find(name).ok_or_else(|| Error::new(format!("no service named {name}")))?;
    let facade = current_facade();
    clean(&def, parameters, facade.as_ref())
}

pub(crate) fn clean(
    def: &ServiceDef,
    input: &BTreeMap<String, Value>,
    facade: Option<&EntityFacade>,
) -> Result<BTreeMap<String, Value>, Error> {
    let declared = expand_params(def, &def.in_params, facade)?;
    let mut params = input.clone();
    let mut known = BTreeSet::new();
    for param in &declared {
        if param.name.is_empty() || param.required == Required::Disabled {
            continue;
        }
        known.insert(param.name.clone());
        if is_blank(params.get(&param.name)) {
            if let Some(literal) = &param.default_value {
                let converted = basic_convert(&Value::string(literal), &param.java_type).value;
                params.insert(param.name.clone(), converted);
            }
        }
        if param.required == Required::True && is_blank(params.get(&param.name)) {
            return Err(Error::new(format!("parameter {} is required", param.name)));
        }
        let Some(value) = params.get(&param.name).cloned() else {
            continue;
        };
        if is_empty(&value) {
            continue;
        }
        check_param(def, param, &value, facade)?;
        if !matches!(value, Value::Map(_) | Value::List(_)) {
            let converted = basic_convert(&value, &param.java_type).value;
            params.insert(param.name.clone(), converted);
        }
    }
    if def.validate_input {
        retain_known(def, &mut params, &known, facade)?;
    }
    Ok(params)
}

fn expand_params(def: &ServiceDef, params: &[ParamDef], facade: Option<&EntityFacade>) -> Result<Vec<ParamDef>, Error> {
    let mut out = Vec::new();
    for param in params {
        if param.name.is_empty() {
            if let Some(auto) = &param.auto {
                out.extend(expand_auto(def, auto, facade)?);
                continue;
            }
        }
        out.push(param.clone());
    }
    Ok(out)
}

fn expand_auto(def: &ServiceDef, auto: &AutoParams, facade: Option<&EntityFacade>) -> Result<Vec<ParamDef>, Error> {
    let Some(facade) = facade else {
        return Err(Error::new("no entity facade is installed"));
    };
    let entity_name = auto
        .entity_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| def.noun.clone());
    if entity_name.is_empty() {
        return Err(Error::new(format!(
            "service {} auto-parameters has no entity",
            def.name
        )));
    }
    facade
        .with_entity(&entity_name, |entity, _, model| {
            let mut params = Vec::new();
            for field in &entity.fields {
                let take = match auto.include {
                    Include::Pk => field.is_pk,
                    Include::NonPk => !field.is_pk,
                    Include::All => true,
                };
                if !take || auto.exclude.iter().any(|name| name == &field.name) {
                    continue;
                }
                let java = model
                    .dictionary_type(&field.field_type)
                    .map(|entry| entry.java_type.clone())
                    .unwrap_or_else(|| "String".to_string());
                params.push(ParamDef {
                    name: field.name.clone(),
                    java_type: java,
                    required: auto.required,
                    default_value: field.default_value.clone(),
                    entity_name: Some(entity.full_name.clone()),
                    field_name: Some(field.name.clone()),
                    checks: Vec::new(),
                    auto: None,
                });
            }
            Ok(params)
        })
        .map_err(Error::from)
}

fn check_param(def: &ServiceDef, param: &ParamDef, value: &Value, facade: Option<&EntityFacade>) -> Result<(), Error> {
    for check in &param.checks {
        apply_check(&param.name, check, value)?;
    }
    if matches!(def.verb.as_str(), "update" | "delete") {
        if let Some(entity_name) = &param.entity_name {
            let Some(facade) = facade else {
                return Err(Error::new("no entity facade is installed"));
            };
            let field = param.field_name.clone().unwrap_or_else(|| param.name.clone());
            let found = facade
                .find(entity_name)
                .map_err(Error::from)?
                .eq(&field, value.clone())
                .one()
                .map_err(Error::from)?;
            if found.is_none() {
                return Err(Error::new(format!(
                    "parameter {} was not found on {entity_name}",
                    param.name
                )));
            }
        }
    }
    Ok(())
}

fn apply_check(name: &str, check: &Check, value: &Value) -> Result<(), Error> {
    match check {
        Check::Matches { regexp, message } => {
            let text = to_plain_string(value);
            let matched = regex_full_match(&text, regexp)?;
            if !matched {
                let detail = if message.is_empty() {
                    format!("parameter {name} does not match {regexp}")
                } else {
                    format!("parameter {name} {message}")
                };
                return Err(Error::new(detail));
            }
        }
        Check::NumberRange {
            min,
            min_eq,
            max,
            max_eq,
            message,
        } => {
            if !is_decimal(value) {
                return Err(Error::new(format!("parameter {name} is not a number")));
            }
            if let Some(min) = min {
                let ordering = decimal_against(value, min)
                    .ok_or_else(|| Error::new(format!("parameter {name} min is not a number")))?;
                if ordering.is_lt() || (ordering.is_eq() && !min_eq) {
                    return Err(range_error(name, message));
                }
            }
            if let Some(max) = max {
                let ordering = decimal_against(value, max)
                    .ok_or_else(|| Error::new(format!("parameter {name} max is not a number")))?;
                if ordering.is_gt() || (ordering.is_eq() && !max_eq) {
                    return Err(range_error(name, message));
                }
            }
        }
        Check::NumberInteger => {
            if !is_whole(value) {
                return Err(Error::new(format!("parameter {name} is not an integer")));
            }
        }
        Check::NumberDecimal => {
            if !is_decimal(value) {
                return Err(Error::new(format!("parameter {name} is not a decimal")));
            }
        }
        Check::TextLength { min, max } => {
            let len = to_plain_string(value).chars().count();
            if min.is_some_and(|min| len < min) || max.is_some_and(|max| len > max) {
                return Err(Error::new(format!("parameter {name} length is out of range")));
            }
        }
        Check::TextEmail => require_match(name, value, r"^[^@\s]+@[^@\s]+\.[^@\s]+$", "is not an email address")?,
        Check::TextUrl => require_match(name, value, r"^https?://\S+$", "is not a URL")?,
        Check::TextLetters => require_match(name, value, "^[A-Za-z]+$", "is not letters")?,
        Check::TextDigits => require_match(name, value, "^[0-9]+$", "is not digits")?,
        Check::TimeRange { after, before } => {
            let stamp = stamp_millis(value).ok_or_else(|| Error::new(format!("parameter {name} is not a date")))?;
            if let Some(after) = after {
                let bound = bound_millis(after)?;
                if stamp <= bound {
                    return Err(Error::new(format!("parameter {name} is not after {after}")));
                }
            }
            if let Some(before) = before {
                let bound = bound_millis(before)?;
                if stamp >= bound {
                    return Err(Error::new(format!("parameter {name} is not before {before}")));
                }
            }
        }
        Check::CreditCard { types } => {
            let digits: String = to_plain_string(value)
                .chars()
                .filter(|ch| ch.is_ascii_digit())
                .collect();
            if !luhn(&digits) || !card_type_ok(&digits, types.as_deref()) {
                return Err(Error::new(format!("parameter {name} is not a credit card number")));
            }
        }
        Check::And(checks) => {
            for child in checks {
                apply_check(name, child, value)?;
            }
        }
        Check::Or(checks) => {
            if checks.is_empty() {
                return Err(Error::new(format!("parameter {name} failed validation")));
            }
            let mut passed = false;
            for child in checks {
                if apply_check(name, child, value).is_ok() {
                    passed = true;
                    break;
                }
            }
            if !passed {
                return Err(Error::new(format!("parameter {name} failed validation")));
            }
        }
        Check::Not(checks) => {
            for child in checks {
                if apply_check(name, child, value).is_ok() {
                    return Err(Error::new(format!("parameter {name} failed validation")));
                }
            }
        }
    }
    Ok(())
}

fn range_error(name: &str, message: &Option<String>) -> Error {
    match message {
        Some(message) if !message.is_empty() => Error::new(format!("parameter {name} {message}")),
        _ => Error::new(format!("parameter {name} is out of range")),
    }
}

fn require_match(name: &str, value: &Value, pattern: &str, detail: &str) -> Result<(), Error> {
    let text = to_plain_string(value);
    if !regex_full_match(&text, pattern)? {
        return Err(Error::new(format!("parameter {name} {detail}")));
    }
    Ok(())
}

fn is_decimal(value: &Value) -> bool {
    basic_convert(value, "BigDecimal").value.as_decimal().is_some()
}

fn is_whole(value: &Value) -> bool {
    let Some(number) = basic_convert(value, "BigDecimal").value.as_decimal() else {
        return false;
    };
    decimal_eq(number, number.trunc())
}

fn decimal_against(value: &Value, bound: &str) -> Option<std::cmp::Ordering> {
    let left = basic_convert(value, "BigDecimal").value.as_decimal()?;
    let right = basic_convert(&Value::string(bound), "BigDecimal").value.as_decimal()?;
    Some(decimal_compare(left, right))
}

fn retain_known(
    def: &ServiceDef,
    params: &mut BTreeMap<String, Value>,
    known: &BTreeSet<String>,
    facade: Option<&EntityFacade>,
) -> Result<(), Error> {
    if def.kind == ServiceKind::EntityAuto {
        let Some(facade) = facade else {
            return Err(Error::new("no entity facade is installed"));
        };
        let fields = facade
            .with_entity(&def.noun, |entity, _, _| {
                Ok(entity
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .collect::<BTreeSet<_>>())
            })
            .map_err(Error::from)?;
        params.retain(|name, value| {
            known.contains(name) || fields.contains(name) || matches!(value, Value::Map(_) | Value::List(_))
        });
        return Ok(());
    }
    params.retain(|name, _| known.contains(name));
    Ok(())
}

fn is_blank(value: Option<&Value>) -> bool {
    value.is_none_or(is_empty)
}

fn current_facade() -> Option<EntityFacade> {
    crate::context::ExecutionContext::with_current(|ctx| ctx.factory().entity()).flatten()
}

fn stamp_millis(value: &Value) -> Option<i64> {
    if value.is_null() {
        return None;
    }
    to_plain_string(value).parse().ok()
}

fn bound_millis(text: &str) -> Result<i64, Error> {
    if text == "now" {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        return Ok(millis);
    }
    let converted = basic_convert(&Value::string(text), "Timestamp").value;
    stamp_millis(&converted).ok_or_else(|| Error::new(format!("time {text} is not a timestamp")))
}

fn luhn(digits: &str) -> bool {
    let chars: Vec<char> = digits.chars().collect();
    if chars.len() < 12 {
        return false;
    }
    let mut sum = 0;
    let mut alternate = false;
    for ch in chars.iter().rev() {
        let mut digit = ch.to_digit(10).unwrap_or(0);
        if alternate {
            digit *= 2;
            if digit > 9 {
                digit -= 9;
            }
        }
        sum += digit;
        alternate = !alternate;
    }
    sum % 10 == 0
}

fn card_type_ok(digits: &str, types: Option<&str>) -> bool {
    let Some(types) = types.filter(|text| !text.is_empty()) else {
        return true;
    };
    let kind = if digits.starts_with('4') {
        "visa"
    } else if digits.starts_with("34") || digits.starts_with("37") {
        "amex"
    } else if digits.starts_with("6011") || digits.starts_with("65") {
        "discover"
    } else if digits.starts_with("36") || digits.starts_with("38") {
        "dinersclub"
    } else {
        "mastercard"
    };
    types
        .split(',')
        .any(|allowed| allowed.trim().eq_ignore_ascii_case(kind))
}
