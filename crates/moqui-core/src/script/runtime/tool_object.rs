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

//! The object `ec.factory.getTool(name, ...parameters)` returns: a component's [`Tool`](crate::context::Tool) as a
//! script sees it. A name in the tool's `methods()` reads as a function that converts its arguments and result as
//! service parameters are; any other name reads through the tool's `property()`. The object holds the tool, not data,
//! so it is the same tool after the context stack keeps it (`<set field="tool" from="ec.factory.getTool('X')"/>`).

use rquickjs::class::{PropertyDescriptor, PropertyName};
use rquickjs::Atom;

use super::*;
use crate::context::ToolHandle;

#[derive(Trace, JsLifetime)]
#[rquickjs::class(exotic)]
pub(super) struct HostTool {
    #[qjs(skip_trace)]
    pub(super) inner: Arc<ToolHandle>,
}

/// `tool` as a script value.
pub(super) fn tool_to_js<'js>(ctx: &Ctx<'js>, tool: Arc<ToolHandle>) -> rquickjs::Result<JsValue<'js>> {
    Class::instance(ctx.clone(), HostTool { inner: tool }).and_then(|class| class.into_js(ctx))
}

/// `ec.factory.getTool(name, ...parameters)`.
pub(super) fn factory_get_tool<'js>(
    ctx: Ctx<'js>,
    name: String,
    parameters: Rest<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let parameters = parameters
            .0
            .iter()
            .map(|value| js_to_host(&ctx, value))
            .collect::<rquickjs::Result<Vec<_>>>()?;
        let factory = with_ec(&ctx, |ec| Arc::clone(ec.factory()))?;
        let tool = factory
            .tool(&name, &parameters)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        tool_to_js(&ctx, Arc::new(ToolHandle(tool)))
    })
}

impl HostTool {
    fn is_method(&self, name: &str) -> bool {
        self.inner.0.methods().contains(&name)
    }

    /// A function that calls `method` on this tool.
    fn method<'js>(&self, ctx: &Ctx<'js>, method: String) -> rquickjs::Result<JsValue<'js>> {
        let tool = Arc::clone(&self.inner);
        let function = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| -> rquickjs::Result<JsValue<'js>> {
                host_guard(&ctx, || {
                    let args = args
                        .0
                        .iter()
                        .map(|value| js_to_host(&ctx, value))
                        .collect::<rquickjs::Result<Vec<_>>>()?;
                    match tool.0.call(&method, args) {
                        Ok(result) => host_to_js(&ctx, result),
                        Err(err) => Err(Exception::throw_message(&ctx, err.message())),
                    }
                })
            },
        )?;
        function.into_js(ctx)
    }

    /// A method of the tool first, then the class prototype and `Object.prototype`, then the tool's own property.
    fn property<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> rquickjs::Result<JsValue<'js>> {
        host_guard(ctx, || {
            let name = atom_name(&atom)?;
            if let Some(name) = name.as_deref().filter(|name| self.is_method(name)) {
                return self.method(ctx, name.to_string());
            }
            if let Some(found) = prototype_member::<HostTool>(ctx, &atom)? {
                return Ok(found);
            }
            let Some(name) = name else {
                return Undefined.into_js(ctx);
            };
            match self.inner.0.property(&name) {
                Ok(Some(value)) => host_to_js(ctx, value),
                Ok(None) => Undefined.into_js(ctx),
                Err(err) => Err(Exception::throw_message(ctx, err.message())),
            }
        })
    }

    fn own_property<'js>(&self, ctx: &Ctx<'js>, atom: Atom<'js>) -> rquickjs::Result<Option<PropertyDescriptor<'js>>> {
        match atom_name(&atom)? {
            Some(name) if self.is_method(&name) => Ok(Some(PropertyDescriptor::new_value(
                self.method(ctx, name)?,
                false,
                false,
                false,
            ))),
            _ => Ok(None),
        }
    }

    fn own_names<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Vec<PropertyName<'js>>> {
        self.inner
            .0
            .methods()
            .iter()
            .map(|name| {
                Ok(PropertyName {
                    atom: Atom::from_str(ctx.clone(), name)?,
                    is_enumerable: false,
                })
            })
            .collect()
    }
}

/// The exotic methods `#[rquickjs::class(exotic)]` calls. A tool is read-only: a set or delete is refused, which a
/// strict-mode script sees as a TypeError.
#[allow(non_snake_case)]
mod __impl_exotic_HostTool__ {
    use super::*;
    use rquickjs::class::JsCell;

    pub(crate) struct ExoticImpl;

    impl ExoticImpl {
        pub fn exotic_get_property<'js>(
            this: &JsCell<'js, HostTool>,
            ctx: &Ctx<'js>,
            atom: Atom<'js>,
            _receiver: JsValue<'js>,
        ) -> rquickjs::Result<JsValue<'js>> {
            this.borrow().property(ctx, atom)
        }

        pub fn exotic_set_property<'js>(
            _this: &JsCell<'js, HostTool>,
            _ctx: &Ctx<'js>,
            _atom: Atom<'js>,
            _receiver: JsValue<'js>,
            value: JsValue<'js>,
        ) -> rquickjs::Result<bool> {
            keep_assigned(value);
            Ok(false)
        }

        pub fn exotic_delete_property<'js>(
            _this: &JsCell<'js, HostTool>,
            _ctx: &Ctx<'js>,
            _atom: Atom<'js>,
        ) -> rquickjs::Result<bool> {
            Ok(false)
        }

        pub fn exotic_has_property<'js>(
            this: &JsCell<'js, HostTool>,
            ctx: &Ctx<'js>,
            atom: Atom<'js>,
        ) -> rquickjs::Result<bool> {
            if atom_name(&atom)?.is_some_and(|name| this.borrow().is_method(&name)) {
                return Ok(true);
            }
            prototype_has::<HostTool>(ctx, &atom)
        }

        pub fn exotic_get_own_property<'js>(
            this: &JsCell<'js, HostTool>,
            ctx: &Ctx<'js>,
            atom: Atom<'js>,
        ) -> rquickjs::Result<Option<PropertyDescriptor<'js>>> {
            this.borrow().own_property(ctx, atom)
        }

        pub fn exotic_get_own_property_names<'js>(
            this: &JsCell<'js, HostTool>,
            ctx: &Ctx<'js>,
        ) -> rquickjs::Result<Vec<PropertyName<'js>>> {
            this.borrow().own_names(ctx)
        }
    }
}
