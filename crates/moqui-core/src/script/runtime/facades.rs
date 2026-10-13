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

//! Smaller `ec` facades: cache, message, logger, resource, user, and web.

use super::*;
use moqui_lib::object::Value;

/// A named framework cache. A cache the conf does not define reads as empty and ignores writes.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostCache {
    #[qjs(skip_trace)]
    pub(super) name: String,
}

pub(super) fn cache_get_cache<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<Class<'js, HostCache>> {
    host_guard(&ctx, || Class::instance(ctx.clone(), HostCache { name }))
}

impl HostCache {
    fn with_cache<T>(
        &self,
        ctx: &Ctx<'_>,
        fallback: T,
        body: impl FnOnce(&moqui_lib::cache::MCache<String, String>) -> Result<T, moqui_lib::error::Error>,
    ) -> rquickjs::Result<T> {
        let found = with_ec(ctx, |ec| ec.factory().cache().get(&self.name))?;
        match found {
            Ok(Some(cache)) => body(&cache).map_err(|err| throw_lib(ctx, &err)),
            Ok(None) => Ok(fallback),
            Err(err) => Err(Exception::throw_message(ctx, err.message())),
        }
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostCache {
    fn get<'js>(&self, ctx: Ctx<'js>, key: String) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let found = self.with_cache(&ctx, None, |cache| cache.get(&key))?;
            match found {
                Some(text) => text.into_js(&ctx),
                None => Null.into_js(&ctx),
            }
        })
    }

    fn put(&self, ctx: Ctx<'_>, key: String, value: JsValue<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let text = moqui_lib::object::to_plain_string(&js_to_host(&ctx, &value)?);
            self.with_cache(&ctx, (), |cache| cache.put(key, text))
        })
    }

    fn remove(&self, ctx: Ctx<'_>, key: String) -> rquickjs::Result<bool> {
        host_guard(&ctx, || self.with_cache(&ctx, false, |cache| cache.remove(&key)))
    }

    fn contains_key(&self, ctx: Ctx<'_>, key: String) -> rquickjs::Result<bool> {
        host_guard(&ctx, || self.with_cache(&ctx, false, |cache| cache.contains_key(&key)))
    }

    fn clear(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || self.with_cache(&ctx, (), |cache| cache.clear()))
    }
}

pub(super) fn resource_expand(ctx: Ctx<'_>, template: String) -> rquickjs::Result<String> {
    host_guard(&ctx, || {
        expand_here(&ctx, &template).map_err(|err| Exception::throw_message(&ctx, err.message()))
    })
}

/// `ec.resource.script(location)`: run the file in the script's own names.
pub(super) fn resource_script(ctx: Ctx<'_>, location: String) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        let source = read_script(&location).map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        let mut options = EvalOptions::default();
        options.strict = false;
        options.global = true;
        ctx.eval_with_options::<JsValue, _>(source, options).map(|_| ())
    })
}

pub(super) fn read_script(location: &str) -> Result<String, Error> {
    let path = crate::context::locate(location)?;
    std::fs::read_to_string(&path)
        .map_err(|err| Error::new(format!("could not read script {location}")).with_source(err))
}

/// Run a script file as a service body does: in the current context frame, with bare names.
///
/// # Errors
///
/// Returns an error when the file cannot be read or the script throws.
pub fn run_script_location(location: &str) -> Result<(), Error> {
    let source = read_script(location)?;
    eval_script(&source)
}

pub(super) fn logger_log(
    ctx: Ctx<'_>,
    level: String,
    message: String,
    _extra: Opt<JsValue<'_>>,
) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        match level.as_str() {
            "warn" | "warning" | "error" => tracing::warn!("{message}"),
            _ => tracing::info!("{message}"),
        }
        Ok(())
    })
}

pub(super) fn logger_warn(ctx: Ctx<'_>, message: String) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        tracing::warn!("{message}");
        Ok(())
    })
}

/// Message facade for the entered execution context. There is no separate public-message list.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostMessage {}

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

/// `ec.user`. `loginUser` records a failure and returns false instead of throwing.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostUser {}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostUser {
    fn login_user(&self, ctx: Ctx<'_>, username: String, password: String) -> rquickjs::Result<bool> {
        host_guard(&ctx, || match crate::user::login_user(&username, &password) {
            Ok(()) => Ok(true),
            Err(err) => {
                with_ec(&ctx, |ec| ec.message().add_error(err.message().to_string()))?;
                Ok(false)
            }
        })
    }

    fn logout_user(&self, ctx: Ctx<'_>) -> rquickjs::Result<bool> {
        host_guard(&ctx, || match crate::user::logout() {
            Ok(()) => Ok(true),
            Err(err) => {
                with_ec(&ctx, |ec| ec.message().add_error(err.message().to_string()))?;
                Ok(false)
            }
        })
    }

    fn internal_login_user(&self, ctx: Ctx<'_>, username: String) -> rquickjs::Result<bool> {
        host_guard(&ctx, || match crate::user::internal_login_user(&username) {
            Ok(()) => Ok(true),
            Err(err) => {
                with_ec(&ctx, |ec| ec.message().add_error(err.message().to_string()))?;
                Ok(false)
            }
        })
    }

    #[qjs(get)]
    fn username<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match crate::user::current_username() {
            Ok(Some(name)) => name.into_js(&ctx),
            Ok(None) => Null.into_js(&ctx),
            Err(err) => Err(Exception::throw_message(&ctx, err.message())),
        })
    }

    #[qjs(get)]
    fn user_id<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || match crate::user::current_user_id() {
            Ok(Some(name)) => name.into_js(&ctx),
            Ok(None) => Null.into_js(&ctx),
            Err(err) => Err(Exception::throw_message(&ctx, err.message())),
        })
    }
}

/// `ec.web` for the request installed on this thread.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(super) struct HostWeb {}

#[rquickjs::methods(rename_all = "camelCase")]
impl HostWeb {
    fn send_json_response(&self, ctx: Ctx<'_>, value: JsValue<'_>) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            crate::web::send_json_response(js_to_host(&ctx, &value)?);
            Ok(())
        })
    }

    fn send_json_error(
        &self,
        ctx: Ctx<'_>,
        status: i32,
        message: String,
        _extra: Opt<JsValue<'_>>,
    ) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            let status = u16::try_from(status).unwrap_or(500);
            crate::web::send_json_error(status, &message);
            Ok(())
        })
    }

    fn send_text_response(&self, ctx: Ctx<'_>, text: String) -> rquickjs::Result<()> {
        host_guard(&ctx, || {
            crate::web::send_text_response(&text);
            Ok(())
        })
    }

    #[qjs(get)]
    fn parameters<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || host_to_js(&ctx, map_value(crate::web::parameters())))
    }

    #[qjs(get)]
    fn session_attributes<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(&ctx, || {
            let mut map = indexmap::IndexMap::new();
            for (key, value) in crate::user::session::session_attributes() {
                map.insert(key, Value::string(value));
            }
            host_to_js(&ctx, Value::Map(map))
        })
    }

    #[qjs(get)]
    fn session_token(&self, ctx: Ctx<'_>) -> rquickjs::Result<String> {
        host_guard(&ctx, || {
            crate::user::csrf_token().map_err(|err| Exception::throw_message(&ctx, err.message()))
        })
    }
}

fn map_value(entries: std::collections::BTreeMap<String, Value>) -> Value {
    Value::Map(entries.into_iter().collect())
}
