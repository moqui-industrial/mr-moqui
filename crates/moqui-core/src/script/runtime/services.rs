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

//! The service call builder scripts get from `ec.service`.

use super::*;

pub(super) fn service_sync<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
    host_guard(&ctx, || fresh_service(&ctx, false))
}

pub(super) fn service_async<'js>(ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, HostService>> {
    host_guard(&ctx, || fresh_service(&ctx, true))
}

pub(super) fn fresh_service<'js>(ctx: &Ctx<'js>, is_async: bool) -> rquickjs::Result<Class<'js, HostService>> {
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

#[derive(Clone)]
pub(super) struct ServiceCall {
    pub(super) name: String,
    pub(super) parameters: std::collections::BTreeMap<String, Value>,
    pub(super) options: CallOptions,
    pub(super) is_async: bool,
}

/// Service call builder. Each method returns a new builder. There is no `distribute` method.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostService {
    #[qjs(skip_trace)]
    pub(super) call: ServiceCall,
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
            // The callee may read or change the caller's names, so they are written back first.
            flush_current(&ctx, true)?;
            if self.call.is_async {
                if let Err(err) = crate::service::call::async_call(&self.call.name, &self.call.parameters) {
                    let message = err.message().to_string();
                    with_ec(&ctx, |ec| ec.message().add_error(message))?;
                }
                return Null.into_js(&ctx);
            }
            match crate::service::call::sync_with(&self.call.name, &self.call.parameters, &self.call.options) {
                Ok(results) => host_to_js(&ctx, Value::Map(results.into_iter().collect())),
                Err(err) => {
                    let message = err.message().to_string();
                    // The call already recorded its own failure as a message.
                    with_ec(&ctx, |ec| {
                        let known = ec.message().errors().iter().any(|text| text == &message);
                        if !known {
                            ec.message().add_error(message);
                        }
                    })?;
                    Null.into_js(&ctx)
                }
            }
        })
    }
}
