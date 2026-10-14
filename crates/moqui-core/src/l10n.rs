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

//! `ec.l10n`: localized messages and formatting in the session locale and time zone.
//!
//! The formatters are in `moqui_lib::format`. The locale is the session locale, else `en_US`.
//! The time zone is the session time zone, else UTC.

use jiff::tz::TimeZone;
use moqui_lib::format;
use moqui_lib::format::Decimal;
use moqui_lib::object::{to_plain_string, Value};

use crate::context::{Error, ExecutionContext};

const LOCALIZED_MESSAGE: &str = "moqui.basic.LocalizedMessage";
const MAX_ORIGINAL: usize = 255;
const DEFAULT_LOCALE: &str = "en_US";

/// The session locale such as `en_US`.
pub fn locale() -> String {
    crate::user::locale()
        .ok()
        .flatten()
        .filter(|text| !text.is_empty())
        .map(|text| text.replace('-', "_"))
        .unwrap_or_else(|| DEFAULT_LOCALE.to_string())
}

/// The session time zone, UTC when unset or unknown.
pub fn time_zone() -> TimeZone {
    crate::user::time_zone()
        .ok()
        .flatten()
        .and_then(|name| TimeZone::get(&name).ok())
        .unwrap_or(TimeZone::UTC)
}

/// The text of `original` in the session locale. A message longer than 255 characters is an
/// error, as in Java. With no `LocalizedMessage` row the text is returned as is, minus any
/// `##` suffix used to tell two uses of the same words apart.
///
/// # Errors
///
/// Returns an error for an `original` longer than 255 characters.
pub fn localize(original: &str) -> Result<String, Error> {
    localize_in(original, &locale())
}

/// [`localize`] in `locale`.
///
/// # Errors
///
/// Same as [`localize`].
pub fn localize_in(original: &str, locale: &str) -> Result<String, Error> {
    if original.is_empty() {
        return Ok(String::new());
    }
    if original.chars().count() > MAX_ORIGINAL {
        return Err(Error::new(format!(
            "Original String cannot be more than 255 characters long, passed in string was {} characters long",
            original.chars().count()
        )));
    }
    let key = format!("{original}::{locale}");
    let cache = ExecutionContext::with_current(|ec| ec.factory().cache().get("l10n.message"))
        .and_then(Result::ok)
        .flatten();
    if let Some(cache) = &cache {
        if let Some(found) = cache.get(&key)? {
            return Ok(moqui_lib::object::to_plain_string(&found));
        }
    }
    let (result, store) = lookup(original, locale);
    if store {
        if let Some(cache) = &cache {
            cache.put(key, Value::string(&result))?;
        }
    }
    Ok(result)
}

/// The message, and whether it is safe to cache (false when there was no entity facade).
fn lookup(original: &str, locale: &str) -> (String, bool) {
    let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() else {
        return (original.to_string(), false);
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let find = |text: &str, locale: &str| -> Option<String> {
        facade
            .find(LOCALIZED_MESSAGE)
            .ok()?
            .eq("original", Value::string(text))
            .eq("locale", Value::string(locale))
            .one()
            .ok()
            .flatten()
            .and_then(|row| row.get("localized").map(to_plain_string))
    };
    let chain = |text: &str| -> Option<String> {
        find(text, locale)
            .or_else(|| locale.split_once('_').and_then(|(language, _)| find(text, language)))
            .or_else(|| find(text, "default"))
    };
    if let Some(found) = chain(original) {
        return (found, true);
    }
    // Words that precede a `##` suffix are the message when the full text has no row.
    let close_curly = original.rfind('}');
    if let Some(hash) = original
        .rfind("##")
        .filter(|hash| *hash > 0 && close_curly.is_none_or(|close| *hash > close))
    {
        let base = &original[..hash];
        return (chain(base).unwrap_or_else(|| base.to_string()), true);
    }
    (original.to_string(), true)
}

fn decimal_of(value: &Value) -> Option<Decimal> {
    match value {
        Value::Decimal(number) => Some(*number),
        Value::I32(number) => Some(Decimal::from(*number)),
        Value::I64(number) => Some(Decimal::from(*number)),
        Value::F64(number) => format!("{number}").parse().ok(),
        Value::String(text) => text.trim().replace(',', "").parse().ok(),
        _ => None,
    }
}

/// `ec.l10n.format`: a value as text. A string is itself. Dates use the zone, numbers use the
/// pattern or the default of grouping with up to twelve fraction digits, and a map or list is JSON.
///
/// # Errors
///
/// Returns an error for a pattern that is not supported.
pub fn format_value(value: &Value, pattern: Option<&str>) -> Result<String, Error> {
    let locale = locale();
    let pattern = pattern.filter(|text| !text.is_empty());
    Ok(match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Timestamp(stamp) => format::format_timestamp(*stamp, pattern, &time_zone())?,
        Value::Date(date) => format::format_date(*date, pattern)?,
        Value::Time(time) => format::format_time(*time, pattern)?,
        Value::F64(number) => format::format_float(*number, pattern, &locale)?,
        Value::I32(_) | Value::I64(_) | Value::Decimal(_) => {
            let number = decimal_of(value).unwrap_or_default();
            format::format_number(number, pattern, &locale)?
        }
        Value::List(_) | Value::Map(_) => {
            let json = moqui_lib::json::to_json(value).to_string();
            if json.len() > 128 {
                serde_json::to_string_pretty(&moqui_lib::json::to_json(value)).unwrap_or(json)
            } else {
                json
            }
        }
        other => to_plain_string(other),
    })
}

struct CurrencyInfo {
    symbol: String,
    fraction_digits: Option<usize>,
}

/// ISO currencies the framework data uses. Others print their code and two fraction digits.
fn iso_currency(code: &str) -> Option<(&'static str, usize)> {
    Some(match code {
        "USD" => ("$", 2),
        "EUR" => ("\u{20ac}", 2),
        "GBP" => ("\u{a3}", 2),
        "JPY" => ("\u{a5}", 0),
        "CNY" => ("CN\u{a5}", 2),
        "INR" => ("\u{20b9}", 2),
        "CAD" => ("CA$", 2),
        "AUD" => ("A$", 2),
        "MXN" => ("MX$", 2),
        "BRL" => ("R$", 2),
        "CHF" => ("CHF", 2),
        "KRW" => ("\u{20a9}", 0),
        _ => return None,
    })
}

fn currency_info(uom_id: Option<&str>) -> CurrencyInfo {
    let mut info = CurrencyInfo {
        symbol: String::new(),
        fraction_digits: None,
    };
    let Some(uom_id) = uom_id.filter(|text| !text.is_empty()) else {
        return info;
    };
    let mut symbol = None;
    if let Some(facade) = ExecutionContext::with_current(|ec| ec.factory().entity()).flatten() {
        let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
        let row = facade.find("moqui.basic.Uom").ok().and_then(|find| {
            find.eq("uomId", Value::string(uom_id))
                .eq("uomTypeEnumId", Value::string("UT_CURRENCY_MEASURE"))
                .one()
                .ok()
                .flatten()
        });
        if let Some(row) = row {
            symbol = row.get("symbol").map(to_plain_string).filter(|text| !text.is_empty());
            info.fraction_digits = row
                .get("fractionDigits")
                .and_then(decimal_of)
                .and_then(|digits| usize::try_from(digits.trunc().mantissa()).ok());
        }
    }
    let iso = iso_currency(uom_id);
    info.symbol = symbol.unwrap_or_else(|| iso.map_or_else(|| uom_id.to_string(), |(symbol, _)| symbol.to_string()));
    if info.fraction_digits.is_none() {
        info.fraction_digits = iso.map(|(_, digits)| digits);
    }
    info
}

/// `ec.l10n.formatCurrency`. A string amount is parsed, an empty one is empty text. The symbol and
/// fraction digits come from the `Uom` row of type currency, else the ISO table, else two digits.
///
/// # Errors
///
/// Returns an error for an amount that is not a number.
pub fn format_currency(
    amount: &Value,
    uom_id: Option<&str>,
    fraction_digits: Option<usize>,
    hide_symbol: bool,
) -> Result<String, Error> {
    if amount.is_null() || amount.as_str().is_some_and(str::is_empty) {
        return Ok(String::new());
    }
    let number = decimal_of(amount)
        .ok_or_else(|| Error::new(format!("cannot format {} as currency", to_plain_string(amount))))?;
    let info = currency_info(uom_id);
    let digits = fraction_digits.or(info.fraction_digits).unwrap_or(2);
    let symbol = if hide_symbol { "" } else { info.symbol.as_str() };
    Ok(format::format_currency(number, symbol, digits, &locale()))
}

/// `ec.l10n.parseTimestamp(input, format)` in the session time zone. None for text that is not a
/// time stamp.
pub fn parse_timestamp(input: &str) -> Option<Value> {
    moqui_lib::object::parse_timestamp(input, &time_zone()).map(Value::Timestamp)
}

#[cfg(test)]
mod tests {
    use moqui_lib::object::Value;

    use super::{format_currency, format_value, localize, localize_in};
    use crate::context::ExecutionContextFactory;

    fn entity_context() -> crate::context::ExecutionContext {
        let context = ExecutionContextFactory::test_context();
        context.factory().set_entity(crate::testing::open());
        context
    }

    fn message(context: &crate::context::ExecutionContext, original: &str, locale: &str, localized: &str) {
        let facade = context.factory().entity().expect("facade");
        let mut row = facade.make_value("moqui.basic.LocalizedMessage").expect("value");
        row.set("original", Value::string(original));
        row.set("locale", Value::string(locale));
        row.set("localized", Value::string(localized));
        row.create(&facade).expect("create");
    }

    #[test]
    fn localize_walks_the_locale_chain_then_default() {
        let context = entity_context();
        let _entered = context.enter();
        message(&context, "Hello", "default", "Hello (default)");
        message(&context, "Hello", "fr", "Bonjour");
        message(&context, "Hello", "fr_CA", "Allo");
        message(&context, "Thing##a", "default", "Thing A");
        assert_eq!(localize_in("Hello", "fr_CA").unwrap(), "Allo");
        assert_eq!(localize_in("Hello", "fr_FR").unwrap(), "Bonjour");
        assert_eq!(localize_in("Hello", "de_DE").unwrap(), "Hello (default)");
        assert_eq!(localize_in("No Row", "de_DE").unwrap(), "No Row");
        assert_eq!(localize_in("Thing##b", "en_US").unwrap(), "Thing");
        assert_eq!(localize_in("", "en_US").unwrap(), "");
        assert_eq!(localize("Hello").unwrap(), "Hello (default)");
    }

    #[test]
    fn a_message_over_255_characters_is_an_error() {
        let context = entity_context();
        let _entered = context.enter();
        assert!(localize(&"x".repeat(255)).is_ok());
        let err = localize(&"x".repeat(256)).unwrap_err();
        assert!(err.to_string().contains("255"), "{err}");
    }

    #[test]
    fn format_covers_numbers_dates_and_currency() {
        let context = entity_context();
        let _entered = context.enter();
        let amount = Value::Decimal("1234.125".parse().unwrap());
        assert_eq!(format_value(&amount, None).unwrap(), "1,234.125");
        assert_eq!(format_value(&amount, Some("#,##0.00")).unwrap(), "1,234.12");
        assert_eq!(format_value(&Value::Null, None).unwrap(), "");
        assert_eq!(format_value(&Value::string("as is"), Some("0")).unwrap(), "as is");
        let stamp = Value::Timestamp("2024-03-05T14:07:09Z".parse().unwrap());
        assert_eq!(format_value(&stamp, None).unwrap(), "2024-03-05 14:07");
        assert_eq!(format_currency(&amount, Some("USD"), None, false).unwrap(), "$1,234.12");
        assert_eq!(format_currency(&amount, Some("USD"), None, true).unwrap(), "1,234.12");
        assert_eq!(
            format_currency(&Value::string(""), Some("USD"), None, false).unwrap(),
            ""
        );
        assert_eq!(
            format_currency(&Value::I32(1234), Some("JPY"), None, false).unwrap(),
            "\u{a5}1,234"
        );
    }
}
