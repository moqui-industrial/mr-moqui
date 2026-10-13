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

//! The decimal class scripts see, and its checked arithmetic.

use super::*;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostDecimal {
    #[qjs(skip_trace)]
    pub(super) inner: Value,
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

pub(super) enum Arith {
    Add,
    Subtract,
    Multiply,
    Divide,
}

pub(super) fn host_arith(ctx: &Ctx<'_>, left: &Value, right: &Value, op: Arith) -> rquickjs::Result<HostDecimal> {
    host_guard(ctx, || {
        let inner = arith(left, right, op).map_err(|err| throw_lib(ctx, &err))?;
        Ok(HostDecimal { inner })
    })
}

pub(super) fn arith(left: &Value, right: &Value, op: Arith) -> Result<Value, moqui_lib::error::Error> {
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

pub(super) fn decimals_equal(left: &Value, right: &Value) -> Result<bool, moqui_lib::error::Error> {
    let Some(left) = left.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    let Some(right) = right.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    Ok(moqui_lib::object::decimal_eq(left, right))
}

pub(super) fn decimals_compare(left: &Value, right: &Value) -> Result<Ordering, moqui_lib::error::Error> {
    let Some(left) = left.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    let Some(right) = right.as_decimal() else {
        return Err(moqui_lib::error::Error::new("not a decimal"));
    };
    Ok(moqui_lib::object::decimal_compare(left, right))
}
