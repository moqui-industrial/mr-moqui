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

//! Field values and coercion. Decimals are `rust_decimal`. Timestamps are `jiff`.
//!
//! `divide` uses scale `max(left, right, 10)` and half-up, so `1/3` is `0.3333333333`.
//! That is the Groovy rule. The half-even screen formatter is a later phase.
//!
//! A timestamp or date with no zone is read in the zone passed to the parser.
//! [`basic_convert`] uses UTC, which is the framework default before a user is logged in.
//! `java.sql` date and time values are civil dates and times. Their plain form is the
//! UTC epoch millis of that civil value, which matches `getTime` when the JVM zone is UTC.

use std::cmp::Ordering;
use std::str::FromStr;

use indexmap::IndexMap;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, ToSpan};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::error::Error;

#[path = "regex_match.rs"]
mod regex_match;

/// A value that can cross a facade without an `ExecutionContext`.
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    String(String),
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Decimal(Decimal),
    Timestamp(Timestamp),
    Date(Date),
    Time(Time),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(IndexMap<String, Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        java_equals(self, other)
    }
}

/// Period units named by the `TF_*` UOM ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodUnit {
    Millis,
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

/// Result of [`basic_convert`]. A failed conversion keeps the original value.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    pub value: Value,
    pub changed: bool,
}

impl Value {
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            Self::Decimal(value) => Some(*value),
            _ => None,
        }
    }
}

pub fn period_unit(uom_id: &str) -> Result<PeriodUnit, Error> {
    match uom_id {
        "TF_ms" => Ok(PeriodUnit::Millis),
        "TF_s" => Ok(PeriodUnit::Seconds),
        "TF_min" => Ok(PeriodUnit::Minutes),
        "TF_hr" => Ok(PeriodUnit::Hours),
        "TF_day" => Ok(PeriodUnit::Days),
        "TF_wk" => Ok(PeriodUnit::Weeks),
        "TF_mon" => Ok(PeriodUnit::Months),
        "TF_yr" => Ok(PeriodUnit::Years),
        _ => Err(Error::new(format!("no period unit for UOM id {uom_id}"))),
    }
}

/// Add `count` of `unit` to a timestamp. Months and years use calendar arithmetic.
pub fn add_period(timestamp: Timestamp, count: i64, unit: PeriodUnit) -> Result<Timestamp, Error> {
    let zoned = timestamp.to_zoned(TimeZone::UTC);
    let shifted = match unit {
        PeriodUnit::Millis => zoned.checked_add(count.milliseconds()),
        PeriodUnit::Seconds => zoned.checked_add(count.seconds()),
        PeriodUnit::Minutes => zoned.checked_add(count.minutes()),
        PeriodUnit::Hours => zoned.checked_add(count.hours()),
        PeriodUnit::Days => zoned.checked_add(count.days()),
        PeriodUnit::Weeks => zoned.checked_add(count.weeks()),
        PeriodUnit::Months => zoned.checked_add(count.months()),
        PeriodUnit::Years => zoned.checked_add(count.years()),
    }
    .map_err(|err| Error::new("could not add a period").with_source(err))?;
    Ok(shifted.timestamp())
}

pub fn decimal_add(left: Decimal, right: Decimal) -> Result<Decimal, Error> {
    left.checked_add(right)
        .ok_or_else(|| Error::new("decimal add overflowed"))
}

pub fn decimal_subtract(left: Decimal, right: Decimal) -> Result<Decimal, Error> {
    left.checked_sub(right)
        .ok_or_else(|| Error::new("decimal subtract overflowed"))
}

pub fn decimal_multiply(left: Decimal, right: Decimal) -> Result<Decimal, Error> {
    left.checked_mul(right)
        .ok_or_else(|| Error::new("decimal multiply overflowed"))
}

pub fn decimal_divide(left: Decimal, right: Decimal) -> Result<Decimal, Error> {
    if right.is_zero() {
        return Err(Error::new("decimal divide by zero"));
    }
    let scale = left.scale().max(right.scale()).max(10);
    let quotient = left
        .checked_div(right)
        .ok_or_else(|| Error::new("decimal divide overflowed"))?;
    Ok(quotient.round_dp_with_strategy(scale, RoundingStrategy::MidpointAwayFromZero))
}

/// Numerical equality. `1.0` and `1.00` are equal. Script `==` is not this method.
pub fn decimal_eq(left: Decimal, right: Decimal) -> bool {
    left == right
}

pub fn decimal_compare(left: Decimal, right: Decimal) -> Ordering {
    left.cmp(&right)
}

pub fn decimal_is_zero(value: Decimal) -> bool {
    value.is_zero()
}

/// Null, empty string, empty list, or empty map. `0` and `false` are not empty.
pub fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::List(items) => items.is_empty(),
        Value::Map(entries) => entries.is_empty(),
        _ => false,
    }
}

pub fn to_plain_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::I32(number) => number.to_string(),
        Value::I64(number) => number.to_string(),
        Value::F64(number) => number.to_string(),
        Value::Decimal(number) => number.to_string(),
        Value::Timestamp(timestamp) => timestamp.as_millisecond().to_string(),
        Value::Date(date) => date_epoch_millis(*date).to_string(),
        Value::Time(time) => time_epoch_millis(*time).to_string(),
        Value::Bytes(bytes) => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        Value::List(items) => {
            let mut out = String::new();
            for item in items {
                if item.is_null() {
                    continue;
                }
                if !out.is_empty() {
                    out.push(',');
                }
                out.push_str(&to_plain_string(item));
            }
            out
        }
        Value::Map(entries) => {
            let mut out = String::from("{");
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                out.push_str(key);
                out.push('=');
                out.push_str(&to_plain_string(item));
            }
            out.push('}');
            out
        }
    }
}

/// SQL `LIKE`. The pattern is case-insensitive. `%` and `_` are the wildcards.
/// A null pattern matches. A null value does not.
pub fn compare_like(value: &Value, pattern: &Value) -> bool {
    if pattern.is_null() {
        return true;
    }
    let (Some(text), Some(like)) = (value.as_str(), pattern.as_str()) else {
        return false;
    };
    like_match(text, like)
}

/// Operators from `ObjectUtilities.compare`. Ordered compares error when the values
/// cannot be ordered. `equals` uses Java `equals`: decimal scale matters.
pub fn compare(field: &Value, operator: &str, to_field: &Value, java_type: &str) -> Result<bool, Error> {
    let field = basic_convert(field, java_type).value;
    let to_field = if is_empty(to_field) {
        to_field.clone()
    } else {
        basic_convert(to_field, java_type).value
    };
    let result = match operator {
        "less" => compare_obj(&field, &to_field)? == Ordering::Less,
        "greater" => compare_obj(&field, &to_field)? == Ordering::Greater,
        "less-equals" => compare_obj(&field, &to_field)? != Ordering::Greater,
        "greater-equals" => compare_obj(&field, &to_field)? != Ordering::Less,
        "contains" => to_plain_string(&field).contains(&to_plain_string(&to_field)),
        "not-contains" => !to_plain_string(&field).contains(&to_plain_string(&to_field)),
        "empty" => is_empty(&field),
        "not-empty" => !is_empty(&field),
        "matches" => regex_matches(&to_plain_string(&field), &to_plain_string(&to_field))?,
        "not-matches" => !regex_matches(&to_plain_string(&field), &to_plain_string(&to_field))?,
        "not-equals" => !java_equals(&field, &to_field),
        _ => java_equals(&field, &to_field),
    };
    Ok(result)
}

/// Null on the left sorts after a value. Null on the right is an error, as `compareTo` would be.
pub fn compare_obj(left: &Value, right: &Value) -> Result<Ordering, Error> {
    if left.is_null() {
        return Ok(if right.is_null() {
            Ordering::Equal
        } else {
            Ordering::Greater
        });
    }
    if right.is_null() {
        return Err(Error::new("cannot compare a value to null"));
    }
    match (left, right) {
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
        (Value::I32(a), Value::I32(b)) => Ok(a.cmp(b)),
        (Value::I64(a), Value::I64(b)) => Ok(a.cmp(b)),
        (Value::F64(a), Value::F64(b)) => a.partial_cmp(b).ok_or_else(|| Error::new("cannot compare NaN")),
        (Value::Decimal(a), Value::Decimal(b)) => Ok(a.cmp(b)),
        (Value::Timestamp(a), Value::Timestamp(b)) => Ok(a.cmp(b)),
        (Value::Date(a), Value::Date(b)) => Ok(a.cmp(b)),
        (Value::Time(a), Value::Time(b)) => Ok(a.cmp(b)),
        (Value::Bytes(a), Value::Bytes(b)) => Ok(a.cmp(b)),
        _ => Err(Error::new("values are not the same comparable type")),
    }
}

pub fn add_numbers(left: &Value, right: &Value) -> Result<Value, Error> {
    if left.is_null() {
        return Ok(right.clone());
    }
    if right.is_null() {
        return Ok(left.clone());
    }
    if let (Some(a), Some(b)) = (as_decimal(left), as_decimal(right)) {
        if matches!(left, Value::Decimal(_)) || matches!(right, Value::Decimal(_)) {
            return Ok(Value::Decimal(decimal_add(a, b)?));
        }
    }
    if let (Some(a), Some(b)) = (as_f64(left), as_f64(right)) {
        if matches!(left, Value::F64(_)) || matches!(right, Value::F64(_)) {
            return Ok(Value::F64(a + b));
        }
    }
    if let (Some(a), Some(b)) = (as_i64(left), as_i64(right)) {
        if matches!(left, Value::I64(_)) || matches!(right, Value::I64(_)) {
            return Ok(Value::I64(a.wrapping_add(b)));
        }
    }
    if let (Some(a), Some(b)) = (as_i32(left), as_i32(right)) {
        return Ok(Value::I32(a.wrapping_add(b)));
    }
    Err(Error::new("add_numbers needs numeric values"))
}

/// Coerce `value` to the Java type name used on an entity field.
///
/// Boolean strings are only `true` and `y`, case-insensitive. A failed conversion
/// returns the original value and `changed == false`.
pub fn basic_convert(value: &Value, java_type: &str) -> Converted {
    if value.is_null() || java_type.is_empty() {
        return Converted {
            value: value.clone(),
            changed: false,
        };
    }
    let target = simple_type_name(java_type);
    if matches_target(value, target) {
        return Converted {
            value: value.clone(),
            changed: false,
        };
    }
    let converted = convert_to(value, target);
    match converted {
        Some(next) => Converted {
            value: next,
            changed: true,
        },
        None => Converted {
            value: value.clone(),
            changed: false,
        },
    }
}

pub fn parse_timestamp(input: &str, zone: &TimeZone) -> Option<Timestamp> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if is_integer_text(input) {
        if let Ok(millis) = input.parse::<i64>() {
            return Timestamp::from_millisecond(millis).ok();
        }
    }
    if let Some(parts) = parse_civil(input) {
        return parts_to_timestamp(parts, zone);
    }
    Timestamp::from_str(input).ok()
}

pub fn parse_date(input: &str) -> Option<Date> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Some(date) = parse_ymd(input) {
        return Some(date);
    }
    if let Some(date) = parse_mdy(input) {
        return Some(date);
    }
    if let Some(parts) = parse_civil(input) {
        return Date::new(parts.year, parts.month, parts.day).ok();
    }
    if is_integer_text(input) {
        let millis = input.parse::<i64>().ok()?;
        let timestamp = Timestamp::from_millisecond(millis).ok()?;
        return Some(timestamp.to_zoned(TimeZone::UTC).date());
    }
    None
}

pub fn parse_time(input: &str) -> Option<Time> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Some(time) = parse_hms(input, true) {
        return Some(time);
    }
    if let Some(time) = parse_clock(input, true) {
        return Some(time);
    }
    if let Some(time) = parse_ampm(input) {
        return Some(time);
    }
    if let Some(parts) = parse_civil(input) {
        return Time::new(parts.hour, parts.minute, parts.second, parts.nano).ok();
    }
    if is_integer_text(input) {
        let millis = input.parse::<i64>().ok()?;
        let timestamp = Timestamp::from_millisecond(millis).ok()?;
        return Some(timestamp.to_zoned(TimeZone::UTC).time());
    }
    None
}

fn simple_type_name(java_type: &str) -> &str {
    match java_type {
        "byte[]" | "[B" => "byte[]",
        other => other.rsplit('.').next().unwrap_or(other),
    }
}

fn matches_target(value: &Value, target: &str) -> bool {
    matches!(
        (value, target),
        (Value::String(_), "String")
            | (Value::Bool(_), "Boolean")
            | (Value::I32(_), "Integer")
            | (Value::I64(_), "Long")
            | (Value::F64(_), "Double" | "Float")
            | (Value::Decimal(_), "BigDecimal")
            | (Value::Timestamp(_), "Timestamp")
            | (Value::Date(_), "Date")
            | (Value::Time(_), "Time")
            | (Value::Bytes(_), "byte[]" | "Blob")
    )
}

fn convert_to(value: &Value, target: &str) -> Option<Value> {
    match target {
        "String" => Some(Value::String(to_plain_string(value))),
        "Boolean" => Some(Value::Bool(to_bool(value)?)),
        "Integer" => Some(Value::I32(to_i32(value)?)),
        "Long" => Some(Value::I64(to_i64(value)?)),
        "Double" | "Float" => Some(Value::F64(to_f64(value)?)),
        "BigDecimal" => Some(Value::Decimal(to_decimal(value)?)),
        "Timestamp" => Some(Value::Timestamp(to_timestamp(value)?)),
        "Date" => Some(Value::Date(to_date(value)?)),
        "Time" => Some(Value::Time(to_time(value)?)),
        "byte[]" | "Blob" => Some(Value::Bytes(to_bytes(value)?)),
        _ => None,
    }
}

fn to_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) => Some(text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("y")),
        Value::I32(number) => Some(*number != 0),
        Value::I64(number) => Some(*number != 0),
        Value::F64(number) => Some(*number != 0.0),
        Value::Decimal(number) => Some(!number.is_zero()),
        _ => None,
    }
}

fn to_i32(value: &Value) -> Option<i32> {
    match value {
        Value::I32(number) => Some(*number),
        Value::I64(number) => i32::try_from(*number).ok(),
        Value::String(text) => text.parse().ok(),
        Value::Decimal(number) => i32::try_from(number.trunc()).ok(),
        Value::F64(number) if number.fract() == 0.0 => i32::try_from(*number as i64).ok(),
        _ => None,
    }
}

fn to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::I64(number) => Some(*number),
        Value::I32(number) => Some(i64::from(*number)),
        Value::String(text) => text.parse().ok(),
        Value::Decimal(number) => i64::try_from(number.trunc()).ok(),
        Value::F64(number) if number.fract() == 0.0 => Some(*number as i64),
        _ => None,
    }
}

fn to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::F64(number) => Some(*number),
        Value::I32(number) => Some(f64::from(*number)),
        Value::I64(number) => Some(*number as f64),
        Value::String(text) => text.parse().ok(),
        Value::Decimal(number) => number.to_string().parse().ok(),
        _ => None,
    }
}

fn to_decimal(value: &Value) -> Option<Decimal> {
    match value {
        Value::Decimal(number) => Some(*number),
        Value::I32(number) => Some(Decimal::from(*number)),
        Value::I64(number) => Some(Decimal::from(*number)),
        Value::String(text) => Decimal::from_str(text).ok(),
        Value::F64(number) => Decimal::from_str(&number.to_string()).ok(),
        _ => None,
    }
}

fn to_timestamp(value: &Value) -> Option<Timestamp> {
    match value {
        Value::Timestamp(timestamp) => Some(*timestamp),
        Value::String(text) => parse_timestamp(text, &TimeZone::UTC),
        Value::I64(millis) => Timestamp::from_millisecond(*millis).ok(),
        Value::I32(millis) => Timestamp::from_millisecond(i64::from(*millis)).ok(),
        Value::Date(date) => date_to_timestamp(*date),
        _ => None,
    }
}

fn to_date(value: &Value) -> Option<Date> {
    match value {
        Value::Date(date) => Some(*date),
        Value::String(text) => parse_date(text),
        Value::Timestamp(timestamp) => Some(timestamp.to_zoned(TimeZone::UTC).date()),
        _ => None,
    }
}

fn to_time(value: &Value) -> Option<Time> {
    match value {
        Value::Time(time) => Some(*time),
        Value::String(text) => parse_time(text),
        Value::Timestamp(timestamp) => Some(timestamp.to_zoned(TimeZone::UTC).time()),
        _ => None,
    }
}

fn to_bytes(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Bytes(bytes) => Some(bytes.clone()),
        Value::String(text) => Some(text.as_bytes().to_vec()),
        _ => None,
    }
}

fn as_decimal(value: &Value) -> Option<Decimal> {
    to_decimal(value)
}

fn as_f64(value: &Value) -> Option<f64> {
    to_f64(value)
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::I64(number) => Some(*number),
        Value::I32(number) => Some(i64::from(*number)),
        _ => None,
    }
}

fn as_i32(value: &Value) -> Option<i32> {
    match value {
        Value::I32(number) => Some(*number),
        _ => None,
    }
}

/// Java `Objects.equals` for these values. Decimal scale is significant.
pub fn java_equals(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Decimal(a), Value::Decimal(b)) => a.mantissa() == b.mantissa() && a.scale() == b.scale(),
        (Value::F64(a), Value::F64(b)) => a.to_bits() == b.to_bits(),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::I32(a), Value::I32(b)) => a == b,
        (Value::I64(a), Value::I64(b)) => a == b,
        (Value::Timestamp(a), Value::Timestamp(b)) => a == b,
        (Value::Date(a), Value::Date(b)) => a == b,
        (Value::Time(a), Value::Time(b)) => a == b,
        (Value::Bytes(a), Value::Bytes(b)) => a == b,
        (Value::List(a), Value::List(b)) => a == b,
        (Value::Map(a), Value::Map(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| value == other))
        }
        _ => false,
    }
}

struct CivilParts {
    year: i16,
    month: i8,
    day: i8,
    hour: i8,
    minute: i8,
    second: i8,
    nano: i32,
    offset_seconds: Option<i32>,
    zone_name: Option<String>,
}

fn parse_civil(input: &str) -> Option<CivilParts> {
    // Tried as whole-string matches, so a shorter pattern cannot steal a longer one.
    let patterns: &[fn(&str) -> Option<CivilParts>] = &[
        parse_date_time_frac_zone,
        parse_date_time_frac_offset,
        parse_date_time_offset,
        parse_date_time_frac,
        parse_date_time_seconds,
        parse_date_time_minutes,
        parse_date_t_seconds,
        parse_date_only,
    ];
    patterns.iter().find_map(|parse| parse(input))
}

fn parse_date_only(input: &str) -> Option<CivilParts> {
    let date = parse_ymd(input)?;
    Some(parts_from_date(date))
}

fn parse_date_time_minutes(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, ' ')?;
    let (hour, minute, rest) = take_hm(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(stamp(date, hour, minute, 0, 0, None, None))
}

fn parse_date_time_seconds(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, ' ')?;
    let (hour, minute, second, rest) = take_hms(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(stamp(date, hour, minute, second, 0, None, None))
}

fn parse_date_time_frac(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, ' ')?;
    let (hour, minute, second, nano, rest) = take_hms_frac(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(stamp(date, hour, minute, second, nano, None, None))
}

fn parse_date_t_seconds(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, 'T')?;
    let (hour, minute, second, rest) = take_hms(rest)?;
    if !rest.is_empty() {
        return None;
    }
    Some(stamp(date, hour, minute, second, 0, None, None))
}

fn parse_date_time_offset(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, 'T')?;
    let (hour, minute, second, rest) = take_hms(rest)?;
    let offset = parse_offset(rest)?;
    Some(stamp(date, hour, minute, second, 0, Some(offset), None))
}

fn parse_date_time_frac_offset(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, 'T')?;
    let (hour, minute, second, nano, rest) = take_hms_frac(rest)?;
    let offset = parse_offset(rest)?;
    Some(stamp(date, hour, minute, second, nano, Some(offset), None))
}

fn parse_date_time_frac_zone(input: &str) -> Option<CivilParts> {
    let (date, rest) = split_date(input, ' ')?;
    let (hour, minute, second, nano, rest) = take_hms_frac(rest)?;
    let zone = rest
        .strip_prefix(' ')
        .filter(|name| !name.is_empty() && name.chars().all(is_zone_char))?;
    Some(stamp(date, hour, minute, second, nano, None, Some(zone.to_string())))
}

fn parts_from_date(date: Date) -> CivilParts {
    stamp(date, 0, 0, 0, 0, None, None)
}

fn stamp(
    date: Date,
    hour: i8,
    minute: i8,
    second: i8,
    nano: i32,
    offset_seconds: Option<i32>,
    zone_name: Option<String>,
) -> CivilParts {
    CivilParts {
        year: date.year(),
        month: date.month(),
        day: date.day(),
        hour,
        minute,
        second,
        nano,
        offset_seconds,
        zone_name,
    }
}

fn parts_to_timestamp(parts: CivilParts, default_zone: &TimeZone) -> Option<Timestamp> {
    let date = Date::new(parts.year, parts.month, parts.day).ok()?;
    let time = Time::new(parts.hour, parts.minute, parts.second, parts.nano).ok()?;
    let date_time = DateTime::from_parts(date, time);
    let zone = if let Some(seconds) = parts.offset_seconds {
        TimeZone::fixed(Offset::from_seconds(seconds).ok()?)
    } else if let Some(name) = parts.zone_name.as_deref() {
        resolve_zone(name)?
    } else {
        default_zone.clone()
    };
    Some(date_time.to_zoned(zone).ok()?.timestamp())
}

fn resolve_zone(name: &str) -> Option<TimeZone> {
    if name.eq_ignore_ascii_case("GMT") || name.eq_ignore_ascii_case("UTC") || name == "Z" {
        return Some(TimeZone::UTC);
    }
    TimeZone::get(name).ok()
}

fn split_date(input: &str, sep: char) -> Option<(Date, &str)> {
    let (date_text, rest) = input.split_once(sep)?;
    Some((parse_ymd(date_text)?, rest))
}

fn parse_ymd(input: &str) -> Option<Date> {
    let mut parts = input.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::new(year, month, day).ok()
}

fn parse_mdy(input: &str) -> Option<Date> {
    let mut parts = input.split('/');
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    let year = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::new(year, month, day).ok()
}

fn take_hm(input: &str) -> Option<(i8, i8, &str)> {
    let (hour, rest) = take_n_digits(input, 2)?;
    let rest = rest.strip_prefix(':')?;
    let (minute, rest) = take_n_digits(rest, 2)?;
    Some((hour as i8, minute as i8, rest))
}

fn take_hms(input: &str) -> Option<(i8, i8, i8, &str)> {
    let (hour, minute, rest) = take_hm(input)?;
    let rest = rest.strip_prefix(':')?;
    let (second, rest) = take_n_digits(rest, 2)?;
    Some((hour, minute, second as i8, rest))
}

fn take_hms_frac(input: &str) -> Option<(i8, i8, i8, i32, &str)> {
    let (hour, minute, second, rest) = take_hms(input)?;
    let rest = rest.strip_prefix('.')?;
    let digits = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect::<String>();
    if digits.is_empty() || digits.len() > 9 {
        return None;
    }
    let nano = frac_to_nano(&digits)?;
    Some((hour, minute, second, nano, &rest[digits.len()..]))
}

fn parse_hms(input: &str, allow_short: bool) -> Option<Time> {
    if let Some((hour, minute, second, nano, rest)) = take_hms_frac(input) {
        if rest.is_empty() {
            return Time::new(hour, minute, second, nano).ok();
        }
    }
    if allow_short {
        parse_clock(input, false)
    } else {
        None
    }
}

fn parse_clock(input: &str, with_seconds: bool) -> Option<Time> {
    if with_seconds {
        if let Some((hour, minute, second, rest)) = take_hms(input) {
            if rest.is_empty() {
                return Time::new(hour, minute, second, 0).ok();
            }
        }
    }
    let (hour, minute, rest) = take_hm(input)?;
    if !rest.is_empty() {
        return None;
    }
    Time::new(hour, minute, 0, 0).ok()
}

fn parse_ampm(input: &str) -> Option<Time> {
    let (clock, marker) = input.rsplit_once(' ')?;
    let pm = match marker.to_ascii_lowercase().as_str() {
        "am" => false,
        "pm" => true,
        _ => return None,
    };
    let (mut hour, minute, second) = if let Some((hour, minute, second, rest)) = take_hms(clock) {
        if !rest.is_empty() {
            return None;
        }
        (hour, minute, second)
    } else {
        let (hour, minute, rest) = take_hm(clock)?;
        if !rest.is_empty() {
            return None;
        }
        (hour, minute, 0)
    };
    if !(1..=12).contains(&hour) {
        return None;
    }
    if hour == 12 {
        hour = 0;
    }
    if pm {
        hour += 12;
    }
    Time::new(hour, minute, second, 0).ok()
}

fn parse_offset(input: &str) -> Option<i32> {
    if input == "Z" || input.eq_ignore_ascii_case("GMT") || input.eq_ignore_ascii_case("UTC") {
        return Some(0);
    }
    let (sign, rest) = match input.chars().next()? {
        '+' => (1, &input[1..]),
        '-' => (-1, &input[1..]),
        _ => return None,
    };
    if rest.len() == 4 && rest.chars().all(|ch| ch.is_ascii_digit()) {
        let hours: i32 = rest[..2].parse().ok()?;
        let minutes: i32 = rest[2..].parse().ok()?;
        return Some(sign * (hours * 3600 + minutes * 60));
    }
    if rest.len() == 5 && rest.as_bytes().get(2) == Some(&b':') {
        let hours: i32 = rest[..2].parse().ok()?;
        let minutes: i32 = rest[3..].parse().ok()?;
        return Some(sign * (hours * 3600 + minutes * 60));
    }
    None
}

fn take_n_digits(input: &str, n: usize) -> Option<(u32, &str)> {
    if input.len() < n || !input[..n].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((input[..n].parse().ok()?, &input[n..]))
}

fn frac_to_nano(digits: &str) -> Option<i32> {
    let mut padded = digits.to_string();
    while padded.len() < 9 {
        padded.push('0');
    }
    padded.parse().ok()
}

fn is_zone_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '/' || ch == '_' || ch == '-' || ch == '+'
}

fn is_integer_text(input: &str) -> bool {
    let digits = input.strip_prefix('-').unwrap_or(input);
    !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
}

fn date_to_timestamp(date: Date) -> Option<Timestamp> {
    let date_time = DateTime::from_parts(date, Time::midnight());
    Some(date_time.to_zoned(TimeZone::UTC).ok()?.timestamp())
}

fn date_epoch_millis(date: Date) -> i64 {
    date_to_timestamp(date)
        .map(|timestamp| timestamp.as_millisecond())
        .unwrap_or(0)
}

fn time_epoch_millis(time: Time) -> i64 {
    let date_time = DateTime::from_parts(Date::new(1970, 1, 1).expect("epoch date"), time);
    date_time
        .to_zoned(TimeZone::UTC)
        .ok()
        .map(|zoned| zoned.timestamp().as_millisecond())
        .unwrap_or(0)
}

fn like_match(text: &str, pattern: &str) -> bool {
    let text: Vec<char> = text.chars().flat_map(|ch| ch.to_lowercase()).collect();
    let pattern: Vec<char> = pattern.chars().flat_map(|ch| ch.to_lowercase()).collect();
    like_at(&text, &pattern, 0, 0)
}

fn like_at(text: &[char], pattern: &[char], ti: usize, pi: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }
    if pattern[pi] == '%' {
        if like_at(text, pattern, ti, pi + 1) {
            return true;
        }
        if ti < text.len() && like_at(text, pattern, ti + 1, pi) {
            return true;
        }
        return false;
    }
    if ti < text.len() && (pattern[pi] == '_' || pattern[pi] == text[ti]) {
        return like_at(text, pattern, ti + 1, pi + 1);
    }
    false
}

fn regex_matches(text: &str, pattern: &str) -> Result<bool, Error> {
    regex_match::is_full_match(text, pattern).map_err(Error::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_divide_uses_groovy_scale() {
        let quotient = decimal_divide(Decimal::from(1), Decimal::from(3)).unwrap();
        assert_eq!(quotient.to_string(), "0.3333333333");
        let sixth = decimal_divide(Decimal::from(1), Decimal::from(6)).unwrap();
        assert_eq!(sixth.to_string(), "0.1666666667");
        assert!(decimal_multiply(Decimal::MAX, Decimal::from(2)).is_err());
        assert!(decimal_eq(
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("1.00").unwrap()
        ));
        assert!(
            !decimal_is_zero(Decimal::from_str("0.00").unwrap()) || decimal_is_zero(Decimal::from_str("0.00").unwrap())
        );
        assert!(decimal_is_zero(Decimal::from_str("0.00").unwrap()));
    }

    #[test]
    fn boolean_and_empty_follow_object_utilities() {
        let yes = basic_convert(&Value::string("Y"), "Boolean");
        assert_eq!(yes.value, Value::Bool(true));
        assert!(yes.changed);
        assert_eq!(
            basic_convert(&Value::string("yes"), "Boolean").value,
            Value::Bool(false)
        );
        assert_eq!(
            basic_convert(&Value::string("false"), "java.lang.Boolean").value,
            Value::Bool(false)
        );
        assert!(!is_empty(&Value::I32(0)));
        assert!(!is_empty(&Value::Bool(false)));
        assert!(is_empty(&Value::string("")));
        assert!(is_empty(&Value::Null));
        let bad = basic_convert(&Value::string("nope"), "Integer");
        assert!(!bad.changed);
        assert_eq!(bad.value, Value::string("nope"));
    }

    #[test]
    fn timestamps_dates_and_plain_millis() {
        let converted = basic_convert(&Value::string("2020-01-02 03:04:05"), "java.sql.Timestamp");
        let Value::Timestamp(timestamp) = converted.value else {
            panic!("not a timestamp");
        };
        let expected = parse_timestamp("2020-01-02T03:04:05Z", &TimeZone::UTC).unwrap();
        assert_eq!(timestamp, expected);
        assert_eq!(
            to_plain_string(&Value::Timestamp(timestamp)),
            expected.as_millisecond().to_string()
        );
        assert_eq!(parse_date("01/02/2020").unwrap(), Date::new(2020, 1, 2).unwrap());
        assert_eq!(parse_time("13:05").unwrap(), Time::new(13, 5, 0, 0).unwrap());
        assert_eq!(
            to_plain_string(&Value::Date(Date::new(1970, 1, 2).unwrap())),
            "86400000"
        );
    }

    #[test]
    fn like_and_compare() {
        assert!(compare_like(&Value::string("Hello"), &Value::string("h%")));
        assert!(compare_like(&Value::string("Hello"), &Value::string("h_llo")));
        assert!(!compare_like(&Value::string("Hello"), &Value::string("x%")));
        assert!(compare_like(&Value::string("a.b"), &Value::string("a.b")));
        assert!(compare_like(&Value::Null, &Value::Null) || compare_like(&Value::string("a"), &Value::Null));
        assert!(compare(&Value::string("10"), "greater", &Value::string("2"), "Integer").unwrap());
        assert!(compare(&Value::string(""), "empty", &Value::Null, "String").unwrap());
        assert!(compare(&Value::string("abc"), "matches", &Value::string("a.*"), "").unwrap());
        assert!(compare(&Value::string("abc"), "matches", &Value::string("a.*c"), "").unwrap());
        assert!(compare(&Value::string("abc"), "matches", &Value::string("(ab|a)c"), "").unwrap());
        assert!(compare(&Value::string("abc"), "matches", &Value::string("[a-c]+"), "").unwrap());
        assert!(compare(&Value::string("12"), "matches", &Value::string(r"\d+"), "").unwrap());
        assert!(!compare(&Value::string("abc"), "matches", &Value::string("a.*d"), "").unwrap());
        let one = Decimal::from_str("1.0").unwrap();
        let one_two = Decimal::from_str("1.00").unwrap();
        assert!(!java_equals(&Value::Decimal(one), &Value::Decimal(one_two)));
        assert_eq!(decimal_compare(one, one_two), Ordering::Equal);
    }

    #[test]
    fn period_units_move_a_timestamp() {
        let start = parse_timestamp("2020-01-31T00:00:00Z", &TimeZone::UTC).unwrap();
        let next = add_period(start, 1, period_unit("TF_mon").unwrap()).unwrap();
        assert_eq!(next.to_zoned(TimeZone::UTC).date(), Date::new(2020, 2, 29).unwrap());
    }
}
