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

//! Map and list helpers from `CollectionUtilities`.
//!
//! Groovy and W3C node copies are not ported. `stdDevMaxFromMapField` is not
//! in this pass. Decimal division uses the dividend's scale and half-up, which
//! is `BigDecimal.divide(divisor, HALF_UP)`. A zero divisor leaves the map as it is.

use std::cmp::Ordering;

use indexmap::IndexMap;
use jiff::Timestamp;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::error::Error;
use crate::object::{decimal_add, is_empty, java_equals, parse_timestamp, to_plain_string, Value};

/// The `key` value of each map. A missing key is a null, as in Java, so the values line up
/// with the maps unless `exclude_null` drops them.
pub fn get_map_list_values(maps: &[IndexMap<String, Value>], key: &str, exclude_null: bool) -> Vec<Value> {
    let mut values = Vec::with_capacity(maps.len());
    for map in maps {
        let value = map.get(key).unwrap_or(&Value::Null);
        if exclude_null && value.is_null() {
            continue;
        }
        values.push(value.clone());
    }
    values
}

/// Keep rows that match every field, or drop them when `exclude` is set.
/// Comparison is Java `equals`, so decimal scale matters.
pub fn filter_map_list(maps: &mut Vec<IndexMap<String, Value>>, fields: &IndexMap<String, Value>, exclude: bool) {
    if maps.is_empty() || fields.is_empty() {
        return;
    }
    maps.retain(|map| row_matches(map, fields) != exclude);
}

fn row_matches(map: &IndexMap<String, Value>, fields: &IndexMap<String, Value>) -> bool {
    fields.iter().all(|(name, expected)| match map.get(name) {
        Some(actual) => java_equals(actual, expected),
        None => expected.is_null(),
    })
}

/// Drop rows whose `from` is after `compare`, or whose `thru` is at or before it.
/// A missing date is open. Names default to `fromDate` and `thruDate`.
pub fn filter_map_list_by_date(
    maps: &mut Vec<IndexMap<String, Value>>,
    from_name: Option<&str>,
    thru_name: Option<&str>,
    compare: Option<Timestamp>,
    ignore_if_empty: bool,
) {
    if maps.is_empty() {
        return;
    }
    if ignore_if_empty && compare.is_none() {
        return;
    }
    let compare = compare.unwrap_or_else(Timestamp::now);
    let from_name = from_name.filter(|name| !name.is_empty()).unwrap_or("fromDate");
    let thru_name = thru_name.filter(|name| !name.is_empty()).unwrap_or("thruDate");
    maps.retain(|map| {
        if let Some(from) = map.get(from_name).and_then(timestamp_of) {
            if compare < from {
                return false;
            }
        }
        if let Some(thru) = map.get(thru_name).and_then(timestamp_of) {
            if compare >= thru {
                return false;
            }
        }
        true
    });
}

fn timestamp_of(value: &Value) -> Option<Timestamp> {
    match value {
        Value::Timestamp(timestamp) => Some(*timestamp),
        Value::I64(millis) => Timestamp::from_millisecond(*millis).ok(),
        Value::I32(millis) => Timestamp::from_millisecond(i64::from(*millis)).ok(),
        Value::String(text) => parse_timestamp(text, &jiff::tz::TimeZone::UTC),
        _ => None,
    }
}

/// Order rows in place. A field may start with `-`, `+`, or `^` (ignore case),
/// and one string may list several fields separated by commas.
/// Nulls sort first when the field is ascending, unless `nulls_last` says otherwise.
pub fn order_map_list(
    maps: &mut [IndexMap<String, Value>],
    field_names: &[&str],
    nulls_last: Option<bool>,
) -> Result<(), Error> {
    let fields = split_order_fields(field_names)?;
    if fields.is_empty() {
        return Ok(());
    }
    // The keys are made and checked first, so the sort's comparator cannot fail: one that stops partway gives an
    // order that is not total, which the standard sort may panic on.
    let mut keyed: Vec<(Vec<SortKey>, usize)> = maps
        .iter()
        .enumerate()
        .map(|(index, map)| {
            let keys = fields
                .iter()
                .map(|field| SortKey::of(map.get(&field.name), field.ignore_case))
                .collect();
            (keys, index)
        })
        .collect();
    for column in 0..fields.len() {
        check_comparable(keyed.iter().map(|(keys, _)| &keys[column]))?;
    }
    keyed.sort_by(|(left, _), (right, _)| compare_keys(left, right, &fields, nulls_last));
    let mut taken: Vec<IndexMap<String, Value>> = maps.iter_mut().map(std::mem::take).collect();
    for (slot, (_, index)) in maps.iter_mut().zip(keyed) {
        *slot = std::mem::take(&mut taken[index]);
    }
    Ok(())
}

/// One field of one row, as the sort compares it.
enum SortKey {
    Null,
    Number(Decimal),
    /// A double with no decimal form, such as infinity. It orders with the numbers.
    Float(f64),
    Text(String),
    Bool(bool),
    Timestamp(Timestamp),
    Date(jiff::civil::Date),
    Time(jiff::civil::Time),
    Bytes(Vec<u8>),
    /// A value with no order, such as a map. Two of them in one field are an error.
    Unordered,
    /// NaN, which orders with nothing.
    NotANumber,
}

impl SortKey {
    fn of(value: Option<&Value>, ignore_case: bool) -> Self {
        let Some(value) = value.filter(|value| !value.is_null()) else {
            return Self::Null;
        };
        if let Some(number) = as_number(value) {
            return Self::Number(number);
        }
        match value {
            Value::F64(number) if number.is_nan() => Self::NotANumber,
            Value::F64(number) => Self::Float(*number),
            Value::String(text) if ignore_case => Self::Text(text.to_lowercase()),
            Value::String(text) => Self::Text(text.clone()),
            Value::Bool(flag) => Self::Bool(*flag),
            Value::Timestamp(stamp) => Self::Timestamp(*stamp),
            Value::Date(date) => Self::Date(*date),
            Value::Time(time) => Self::Time(*time),
            Value::Bytes(bytes) => Self::Bytes(bytes.clone()),
            _ => Self::Unordered,
        }
    }

    /// Keys of one group order against each other. None for a null.
    fn group(&self) -> Option<u8> {
        match self {
            Self::Null => None,
            Self::Number(_) | Self::Float(_) => Some(0),
            Self::Text(_) => Some(1),
            Self::Bool(_) => Some(2),
            Self::Timestamp(_) => Some(3),
            Self::Date(_) => Some(4),
            Self::Time(_) => Some(5),
            Self::Bytes(_) => Some(6),
            Self::Unordered => Some(7),
            Self::NotANumber => Some(8),
        }
    }
}

/// An error when two present keys of one field cannot be ordered against each other.
fn check_comparable<'a>(keys: impl Iterator<Item = &'a SortKey>) -> Result<(), Error> {
    let mut first: Option<&SortKey> = None;
    for key in keys {
        let Some(group) = key.group() else {
            continue;
        };
        let Some(seen) = first else {
            first = Some(key);
            continue;
        };
        if matches!(key, SortKey::NotANumber) || matches!(seen, SortKey::NotANumber) {
            return Err(Error::new("cannot compare NaN"));
        }
        if seen.group() != Some(group) || group == 7 {
            return Err(Error::new("values are not the same comparable type"));
        }
    }
    Ok(())
}

fn compare_keys(left: &[SortKey], right: &[SortKey], fields: &[OrderField], nulls_last: Option<bool>) -> Ordering {
    for ((left, right), field) in left.iter().zip(right).zip(fields) {
        let nulls_first = match nulls_last {
            Some(last) => !last,
            None => field.ascending,
        };
        let ordering = match (left, right) {
            (SortKey::Null, SortKey::Null) => Ordering::Equal,
            (SortKey::Null, _) if nulls_first => Ordering::Less,
            (SortKey::Null, _) => Ordering::Greater,
            (_, SortKey::Null) if nulls_first => Ordering::Greater,
            (_, SortKey::Null) => Ordering::Less,
            (left, right) if field.ascending => compare_present_keys(left, right),
            (left, right) => compare_present_keys(left, right).reverse(),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

/// Two present keys that [`check_comparable`] passed.
fn compare_present_keys(left: &SortKey, right: &SortKey) -> Ordering {
    match (left, right) {
        (SortKey::Number(a), SortKey::Number(b)) => a.cmp(b),
        (SortKey::Float(a), SortKey::Float(b)) => a.total_cmp(b),
        (SortKey::Number(a), SortKey::Float(b)) => decimal_f64(*a).total_cmp(b),
        (SortKey::Float(a), SortKey::Number(b)) => a.total_cmp(&decimal_f64(*b)),
        (SortKey::Text(a), SortKey::Text(b)) => a.cmp(b),
        (SortKey::Bool(a), SortKey::Bool(b)) => a.cmp(b),
        (SortKey::Timestamp(a), SortKey::Timestamp(b)) => a.cmp(b),
        (SortKey::Date(a), SortKey::Date(b)) => a.cmp(b),
        (SortKey::Time(a), SortKey::Time(b)) => a.cmp(b),
        (SortKey::Bytes(a), SortKey::Bytes(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}

fn decimal_f64(number: Decimal) -> f64 {
    number.to_string().parse().unwrap_or(0.0)
}

struct OrderField {
    name: String,
    ascending: bool,
    ignore_case: bool,
}

fn split_order_fields(field_names: &[&str]) -> Result<Vec<OrderField>, Error> {
    let mut fields = Vec::new();
    for raw in field_names {
        for piece in raw.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            fields.push(parse_order_field(piece)?);
        }
    }
    Ok(fields)
}

fn parse_order_field(field: &str) -> Result<OrderField, Error> {
    let mut name = field;
    let mut ascending = true;
    if let Some(rest) = name.strip_prefix('-') {
        ascending = false;
        name = rest;
    } else if let Some(rest) = name.strip_prefix('+') {
        name = rest;
    }
    let mut ignore_case = false;
    if let Some(rest) = name.strip_prefix('^') {
        ignore_case = true;
        name = rest;
    }
    if name.is_empty() {
        return Err(Error::new(format!("empty order field in '{field}'")));
    }
    Ok(OrderField {
        name: name.to_string(),
        ascending,
        ignore_case,
    })
}

fn as_number(value: &Value) -> Option<Decimal> {
    match value {
        Value::I32(number) => Some(Decimal::from(*number)),
        Value::I64(number) => Some(Decimal::from(*number)),
        Value::F64(number) => Decimal::from_str_exact(&number.to_string())
            .ok()
            .or_else(|| parse_loose_decimal(&number.to_string())),
        Value::Decimal(number) => Some(*number),
        _ => None,
    }
}

fn parse_loose_decimal(text: &str) -> Option<Decimal> {
    Decimal::from_str_exact(text).ok()
}

/// The row with the highest score. A null field on a row matches any value and
/// adds nothing. The first row wins a tie.
pub fn find_maximal_match<'a>(
    maps: &'a [IndexMap<String, Value>],
    fields: &IndexMap<String, Value>,
) -> Option<&'a IndexMap<String, Value>> {
    let names: Vec<&str> = fields.keys().map(String::as_str).collect();
    let wanted: Vec<&Value> = fields.values().collect();
    let num_fields = names.len();
    let mut high_score = -1;
    let mut high = None;
    for map in maps {
        let mut score = 0;
        let mut skip = false;
        for (index, name) in names.iter().enumerate() {
            let Some(actual) = map.get(*name).filter(|value| !value.is_null()) else {
                continue;
            };
            if !java_equals(actual, wanted[index]) {
                skip = true;
                break;
            }
            score += (num_fields - index) as i32 + num_fields as i32;
        }
        if skip {
            continue;
        }
        if score > high_score {
            high_score = score;
            high = Some(map);
        }
    }
    high
}

pub fn add_to_list_in_map(map: &mut IndexMap<String, Value>, key: &str, value: Value) {
    match map.get_mut(key) {
        Some(Value::List(items)) => items.push(value),
        _ => {
            map.insert(key.to_string(), Value::List(vec![value]));
        }
    }
}

/// A set is a list that keeps the first copy of a value.
pub fn add_to_set_in_map(map: &mut IndexMap<String, Value>, key: &str, value: Value) -> bool {
    match map.get_mut(key) {
        Some(Value::List(items)) => {
            if items.iter().any(|item| java_equals(item, &value)) {
                return false;
            }
            items.push(value);
            true
        }
        _ => {
            map.insert(key.to_string(), Value::List(vec![value]));
            true
        }
    }
}

pub fn add_to_map_in_map(map: &mut IndexMap<String, Value>, outer: &str, inner: &str, value: Value) {
    match map.get_mut(outer) {
        Some(Value::Map(inner_map)) => {
            inner_map.insert(inner.to_string(), value);
        }
        _ => {
            let mut inner_map = IndexMap::new();
            inner_map.insert(inner.to_string(), value);
            map.insert(outer.to_string(), Value::Map(inner_map));
        }
    }
}

pub fn add_to_decimal_in_map(
    map: &mut IndexMap<String, Value>,
    key: &str,
    value: Option<Decimal>,
) -> Result<(), Error> {
    let Some(value) = value else {
        return Ok(());
    };
    match map.get(key) {
        None | Some(Value::Null) => {
            map.insert(key.to_string(), Value::Decimal(value));
            Ok(())
        }
        Some(current) => {
            let current = decimal_from_value(current)?;
            map.insert(key.to_string(), Value::Decimal(decimal_add(current, value)?));
            Ok(())
        }
    }
}

fn decimal_from_value(value: &Value) -> Result<Decimal, Error> {
    if let Some(number) = as_number(value) {
        return Ok(number);
    }
    if let Value::String(text) = value {
        return Decimal::from_str_exact(text).map_err(|_| Error::new(format!("not a decimal: {text}")));
    }
    Err(Error::new("not a decimal"))
}

pub fn add_decimals_in_map(base: &mut IndexMap<String, Value>, add: &IndexMap<String, Value>) -> Result<(), Error> {
    for (key, value) in add {
        let Value::Decimal(add_value) = value else {
            continue;
        };
        let current = match base.get(key) {
            Some(Value::Decimal(number)) => *number,
            _ => Decimal::ZERO,
        };
        base.insert(key.clone(), Value::Decimal(decimal_add(current, *add_value)?));
    }
    Ok(())
}

pub fn divide_decimals_in_map(base: &mut IndexMap<String, Value>, divisor: Decimal) -> Result<(), Error> {
    if divisor.is_zero() {
        return Ok(());
    }
    for value in base.values_mut() {
        let Value::Decimal(number) = value else {
            continue;
        };
        *number = divide_half_up(*number, divisor)?;
    }
    Ok(())
}

fn divide_half_up(value: Decimal, divisor: Decimal) -> Result<Decimal, Error> {
    let scale = value.scale();
    let quotient = value
        .checked_div(divisor)
        .ok_or_else(|| Error::new("decimal divide overflowed"))?;
    Ok(quotient.round_dp_with_strategy(scale, RoundingStrategy::MidpointAwayFromZero))
}

/// The value of `key`, including an explicit null. Nested maps and lists of maps
/// are searched only when this map does not contain the key.
pub fn find_field_nested(key: &str, map: &IndexMap<String, Value>) -> Option<Value> {
    if let Some(value) = map.get(key) {
        return Some(value.clone());
    }
    for value in map.values() {
        if let Some(found) = find_in_value(key, value) {
            return Some(found);
        }
    }
    None
}

fn find_in_value(key: &str, value: &Value) -> Option<Value> {
    match value {
        Value::Map(map) => find_field_nested(key, map),
        Value::List(items) => {
            for item in items {
                if let Value::Map(map) = item {
                    if let Some(found) = find_field_nested(key, map) {
                        return Some(found);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Every non-null value of `key`, in walk order, without duplicates.
pub fn find_all_fields_nested(key: &str, map: &IndexMap<String, Value>) -> Vec<Value> {
    let mut found = Vec::new();
    collect_fields(key, map, &mut found);
    found
}

fn collect_fields(key: &str, map: &IndexMap<String, Value>, found: &mut Vec<Value>) {
    if let Some(value) = map.get(key) {
        if !value.is_null() && found.iter().all(|have| !java_equals(have, value)) {
            found.push(value.clone());
        }
    }
    for value in map.values() {
        match value {
            Value::Map(nested) => collect_fields(key, nested, found),
            Value::List(items) => {
                for item in items {
                    if let Value::Map(nested) = item {
                        collect_fields(key, nested, found);
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn flatten_nested_map(map: &IndexMap<String, Value>) -> IndexMap<String, Value> {
    let mut out = IndexMap::new();
    flatten_into(map, &mut out);
    out
}

fn flatten_into(map: &IndexMap<String, Value>, out: &mut IndexMap<String, Value>) {
    for (key, value) in map {
        match value {
            Value::Map(nested) => flatten_into(nested, out),
            Value::List(items) => {
                for item in items {
                    if let Value::Map(nested) = item {
                        flatten_into(nested, out);
                    }
                }
            }
            other => {
                out.insert(key.clone(), other.clone());
            }
        }
    }
}

pub fn flatten_nested_map_with_keys(map: &IndexMap<String, Value>) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    flatten_keys(map, "", &mut out);
    out
}

fn flatten_keys(map: &IndexMap<String, Value>, parent: &str, out: &mut IndexMap<String, String>) {
    for (key, value) in map {
        let new_key = if parent.is_empty() {
            key.clone()
        } else {
            format!("{parent}[{key}]")
        };
        match value {
            Value::Map(nested) => flatten_keys(nested, &new_key, out),
            Value::List(items) => {
                for (index, item) in items.iter().enumerate() {
                    let indexed = format!("{new_key}[{index}]");
                    if let Value::Map(nested) = item {
                        flatten_keys(nested, &indexed, out);
                    } else {
                        out.insert(indexed, plain_or_null(item));
                    }
                }
            }
            other => {
                out.insert(new_key, plain_or_null(other));
            }
        }
    }
}

fn plain_or_null(value: &Value) -> String {
    if value.is_null() {
        "null".to_string()
    } else {
        to_plain_string(value)
    }
}

pub fn merge_nested_map(
    base: &mut IndexMap<String, Value>,
    override_map: &IndexMap<String, Value>,
    override_empty: bool,
) {
    for (key, value) in override_map {
        if !base.contains_key(key) {
            base.insert(key.clone(), value.clone());
            continue;
        }
        if value.is_null() {
            if override_empty {
                base.insert(key.clone(), Value::Null);
            }
            continue;
        }
        if let Value::String(text) = value {
            if override_empty || !text.is_empty() {
                base.insert(key.clone(), value.clone());
            }
            continue;
        }
        if let Value::Map(override_nested) = value {
            match base.get_mut(key) {
                Some(Value::Map(base_nested)) => merge_nested_map(base_nested, override_nested, override_empty),
                _ => {
                    base.insert(key.clone(), value.clone());
                }
            }
            continue;
        }
        if let Value::List(override_items) = value {
            match base.get_mut(key) {
                Some(Value::List(base_items)) => {
                    for item in override_items {
                        if base_items.iter().all(|have| !java_equals(have, item)) {
                            base_items.push(item.clone());
                        }
                    }
                }
                _ => {
                    base.insert(key.clone(), value.clone());
                }
            }
            continue;
        }
        base.insert(key.clone(), value.clone());
    }
}

pub fn remove_nulls(map: &mut IndexMap<String, Value>) {
    map.retain(|_, value| !value.is_null());
}

pub fn map_matches_fields(base: &IndexMap<String, Value>, compare: &IndexMap<String, Value>) -> bool {
    row_matches(base, compare)
}

/// Page keys are `{name}Count`, `PageIndex`, `PageSize`, `PageMaxIndex`,
/// `PageRangeLow`, and `PageRangeHigh`. `pageIndex` defaults to 0 and `pageSize`
/// to 20. When `{name}Count` is already set, this only sets `{name}AlreadyPaginated`.
pub fn paginate_parameters(
    list_size: i32,
    page_list_name: &str,
    context: &mut IndexMap<String, Value>,
) -> Result<(), Error> {
    let mut page_index = context_int(context, "pageIndex").unwrap_or(0);
    if page_index < 0 {
        page_index = 0;
    }
    let mut page_size = context_int(context, "pageSize").unwrap_or(20);
    if page_size < 0 {
        page_size = 20;
    }
    if page_size == 0 {
        return Err(Error::new("pageSize is 0"));
    }
    let count_key = format!("{page_list_name}Count");
    if context.get(&count_key).is_some_and(|value| !value.is_null()) {
        context.insert(format!("{page_list_name}AlreadyPaginated"), Value::Bool(true));
        return Ok(());
    }
    let count = list_size.max(0);
    let max_index = if count == 0 { 0 } else { (count - 1) / page_size };
    let mut page_range_low = page_index
        .checked_mul(page_size)
        .and_then(|start| start.checked_add(1))
        .ok_or_else(|| Error::new("page index overflow"))?;
    if page_range_low > count {
        page_range_low = count + 1;
    }
    let mut page_range_high = page_index
        .checked_mul(page_size)
        .and_then(|start| start.checked_add(page_size))
        .ok_or_else(|| Error::new("page index overflow"))?;
    if page_range_high > count {
        page_range_high = count;
    }
    context.insert(count_key, Value::I32(count));
    context.insert(format!("{page_list_name}PageIndex"), Value::I32(page_index));
    context.insert(format!("{page_list_name}PageSize"), Value::I32(page_size));
    context.insert(format!("{page_list_name}PageMaxIndex"), Value::I32(max_index));
    context.insert(format!("{page_list_name}PageRangeLow"), Value::I32(page_range_low));
    context.insert(format!("{page_list_name}PageRangeHigh"), Value::I32(page_range_high));
    Ok(())
}

pub fn paginate_list(
    list: &[Value],
    page_list_name: &str,
    context: &mut IndexMap<String, Value>,
) -> Result<Vec<Value>, Error> {
    if context.contains_key(&format!("{page_list_name}AlreadyPaginated")) {
        return Ok(list.to_vec());
    }
    let low_key = format!("{page_list_name}PageRangeLow");
    let high_key = format!("{page_list_name}PageRangeHigh");
    if context.get(&low_key).is_none_or(|value| value.is_null())
        || context.get(&high_key).is_none_or(|value| value.is_null())
    {
        let size = i32::try_from(list.len()).map_err(|_| Error::new("list is too long to paginate"))?;
        paginate_parameters(size, page_list_name, context)?;
    }
    let low = context_int(context, &low_key).ok_or_else(|| Error::new("missing page range low"))?;
    let high = context_int(context, &high_key).ok_or_else(|| Error::new("missing page range high"))?;
    if low < 1 {
        return Err(Error::new("page range low is below 1"));
    }
    let start = (low - 1) as usize;
    let end = high.max(0) as usize;
    if start > list.len() || end > list.len() || start > end {
        return Err(Error::new(format!(
            "page range {low}..{high} is outside a list of {}",
            list.len()
        )));
    }
    Ok(list[start..end].to_vec())
}

/// Read `{list_name}` from the context, paginate it, and store the page.
pub fn paginate_named_list(
    list_name: &str,
    page_list_name: Option<&str>,
    context: &mut IndexMap<String, Value>,
) -> Result<(), Error> {
    let page_name = page_list_name.filter(|name| !name.is_empty()).unwrap_or(list_name);
    let list = match context.get(list_name) {
        Some(Value::List(items)) => items.clone(),
        None | Some(Value::Null) => Vec::new(),
        Some(_) => return Err(Error::new(format!("{list_name} is not a list"))),
    };
    let page = paginate_list(&list, page_name, context)?;
    context.insert(page_name.to_string(), Value::List(page));
    Ok(())
}

fn context_int(context: &IndexMap<String, Value>, key: &str) -> Option<i32> {
    let value = context.get(key)?;
    if is_empty(value) {
        return None;
    }
    match value {
        Value::I32(number) => Some(*number),
        Value::I64(number) => i32::try_from(*number).ok(),
        Value::Decimal(number) => i32::try_from(number.mantissa()).ok().filter(|_| number.scale() == 0),
        Value::String(text) => text.parse().ok(),
        _ => to_plain_string(value).parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::parse_timestamp;
    use jiff::tz::TimeZone;

    fn row(pairs: &[(&str, Value)]) -> IndexMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn map_list_values_keep_alignment() {
        let maps = vec![
            row(&[("a", Value::I32(1))]),
            row(&[("b", Value::I32(2))]),
            row(&[("a", Value::Null)]),
            row(&[("a", Value::I32(3))]),
        ];
        let all = get_map_list_values(&maps, "a", false);
        assert_eq!(all.len(), 4);
        assert!(all[1].is_null() && all[2].is_null());
        let present = get_map_list_values(&maps, "a", true);
        assert_eq!(present.len(), 2);
        assert!(java_equals(&present[1], &Value::I32(3)));
    }

    #[test]
    fn filter_order_and_maximal_match() {
        let mut maps = vec![
            row(&[("status", Value::string("open")), ("name", Value::string("B"))]),
            row(&[("status", Value::string("closed")), ("name", Value::string("A"))]),
            row(&[("status", Value::string("open")), ("name", Value::string("a"))]),
        ];
        let mut fields = IndexMap::new();
        fields.insert("status".to_string(), Value::string("open"));
        filter_map_list(&mut maps, &fields, false);
        assert_eq!(maps.len(), 2);
        order_map_list(&mut maps, &["^name"], None).unwrap();
        assert_eq!(maps[0].get("name"), Some(&Value::string("a")));
        let mut priority = IndexMap::new();
        priority.insert("name".to_string(), Value::string("B"));
        priority.insert("status".to_string(), Value::string("open"));
        let best = find_maximal_match(&maps, &priority).unwrap();
        assert_eq!(best.get("name"), Some(&Value::string("B")));
    }

    #[test]
    fn order_sorts_by_fields_and_nulls_and_refuses_mixed_types_before_sorting() {
        let names = |maps: &[IndexMap<String, Value>]| -> Vec<String> {
            maps.iter()
                .map(|map| map.get("id").map(to_plain_string).unwrap_or_default())
                .collect()
        };
        let mut maps = vec![
            row(&[
                ("id", Value::string("r1")),
                ("n", Value::I64(3)),
                ("s", Value::string("b")),
            ]),
            row(&[
                ("id", Value::string("r2")),
                ("n", Value::Null),
                ("s", Value::string("a")),
            ]),
            row(&[
                ("id", Value::string("r3")),
                ("n", Value::F64(1.5)),
                ("s", Value::string("a")),
            ]),
            row(&[
                ("id", Value::string("r4")),
                ("n", Value::Decimal(Decimal::from(3))),
                ("s", Value::string("a")),
            ]),
            row(&[("id", Value::string("r5")), ("n", Value::F64(f64::INFINITY))]),
        ];
        order_map_list(&mut maps, &["n", "s"], None).unwrap();
        assert_eq!(names(&maps), ["r2", "r3", "r4", "r1", "r5"]);
        order_map_list(&mut maps, &["-n"], None).unwrap();
        // Stable: r4 and r1 tie on n and keep their order.
        assert_eq!(names(&maps), ["r5", "r4", "r1", "r3", "r2"]);
        order_map_list(&mut maps, &["n"], Some(true)).unwrap();
        assert_eq!(names(&maps)[4], "r2");

        // Mixed types fail before any reordering, however many rows there are, and never panic the sort.
        let mut mixed: Vec<IndexMap<String, Value>> = (0..200)
            .map(|index| {
                let value = if index % 7 == 0 {
                    Value::string(format!("{index}"))
                } else {
                    Value::I32(200 - index)
                };
                row(&[("id", Value::I32(index)), ("n", value)])
            })
            .collect();
        let before = names(&mixed);
        let err = order_map_list(&mut mixed, &["n"], None).unwrap_err();
        assert!(err.to_string().contains("not the same comparable type"), "{err}");
        assert_eq!(names(&mixed), before);
        let mut nan = vec![row(&[("n", Value::F64(f64::NAN))]), row(&[("n", Value::F64(1.0))])];
        assert!(order_map_list(&mut nan, &["n"], None).is_err());
        let mut one_map = vec![row(&[("n", Value::Map(IndexMap::new()))]), row(&[("n", Value::Null)])];
        order_map_list(&mut one_map, &["n"], None).unwrap();
        assert!(one_map[0].get("n").is_some_and(Value::is_null));
    }

    #[test]
    fn dates_decimals_and_pagination() {
        let early = parse_timestamp("2020-01-01T00:00:00Z", &TimeZone::UTC).unwrap();
        let mid = parse_timestamp("2020-06-01T00:00:00Z", &TimeZone::UTC).unwrap();
        let late = parse_timestamp("2020-12-01T00:00:00Z", &TimeZone::UTC).unwrap();
        let mut maps = vec![row(&[
            ("fromDate", Value::Timestamp(early)),
            ("thruDate", Value::Timestamp(late)),
            ("amount", Value::Decimal(Decimal::from_str_exact("10.00").unwrap())),
        ])];
        filter_map_list_by_date(&mut maps, None, None, Some(mid), false);
        assert_eq!(maps.len(), 1);
        divide_decimals_in_map(&mut maps[0], Decimal::from(4)).unwrap();
        assert_eq!(
            maps[0].get("amount").and_then(Value::as_decimal).unwrap().to_string(),
            "2.50"
        );
        let mut one = IndexMap::new();
        one.insert("n".to_string(), Value::Decimal(Decimal::from(1)));
        divide_decimals_in_map(&mut one, Decimal::from(3)).unwrap();
        assert_eq!(one.get("n").and_then(Value::as_decimal).unwrap().to_string(), "0");

        let list: Vec<Value> = (0..25).map(Value::I32).collect();
        let mut context = IndexMap::new();
        context.insert("pageIndex".to_string(), Value::I32(1));
        context.insert("pageSize".to_string(), Value::I32(20));
        let page = paginate_list(&list, "rows", &mut context).unwrap();
        assert_eq!(page.len(), 5);
        assert_eq!(context.get("rowsPageRangeLow"), Some(&Value::I32(21)));
        assert_eq!(context.get("rowsPageMaxIndex"), Some(&Value::I32(1)));
    }

    #[test]
    fn nested_maps_merge_and_flatten() {
        let mut inner = IndexMap::new();
        inner.insert("id".to_string(), Value::string("1"));
        let mut root = IndexMap::new();
        root.insert("id".to_string(), Value::Null);
        root.insert("child".to_string(), Value::Map(inner));
        assert!(find_field_nested("id", &root).unwrap().is_null());
        let mut nested = IndexMap::new();
        nested.insert("id".to_string(), Value::string("2"));
        let mut root = IndexMap::new();
        root.insert("child".to_string(), Value::Map(nested));
        assert_eq!(find_field_nested("id", &root), Some(Value::string("2")));
        let flat = flatten_nested_map(&root);
        assert_eq!(flat.get("id"), Some(&Value::string("2")));
        root.insert("note".to_string(), Value::string("keep"));
        let mut over = IndexMap::new();
        over.insert("note".to_string(), Value::string(""));
        merge_nested_map(&mut root, &over, false);
        assert_eq!(root.get("note"), Some(&Value::string("keep")));
        merge_nested_map(&mut root, &over, true);
        assert_eq!(root.get("note"), Some(&Value::string("")));
    }
}
