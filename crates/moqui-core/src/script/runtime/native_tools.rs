//! Explicit JavaScript wrappers for component tools; no Rust reflection or opaque values.

use rquickjs::function::Rest;

use super::*;

fn get_tool<'js>(ctx: Ctx<'js>, name: String, parameters: Rest<JsValue<'js>>) -> rquickjs::Result<Object<'js>> {
    host_guard(&ctx, || {
        let parameters = parameters
            .0
            .iter()
            .map(|value| js_to_host(&ctx, value))
            .collect::<rquickjs::Result<Vec<_>>>()?;
        let tool = with_ec(&ctx, |ec| ec.factory().get_tool(&name, &parameters))?
            .map_err(|error| Exception::throw_message(&ctx, error.message()))?;
        let object = Object::new(ctx.clone())?;
        for &method in tool.methods() {
            let tool = Arc::clone(&tool);
            let function = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
                host_guard(&ctx, || {
                    let args = args
                        .0
                        .iter()
                        .map(|value| js_to_host(&ctx, value))
                        .collect::<rquickjs::Result<Vec<_>>>()?;
                    let value = with_ec(&ctx, |ec| tool.call(ec, method, &args))?
                        .map_err(|error| Exception::throw_message(&ctx, error.message()))?;
                    host_to_js(&ctx, value)
                })
            })?;
            object.set(method, function)?;
        }
        Ok(object)
    })
}

fn get_factory<'js>(ctx: Ctx<'js>, name: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let tool = with_ec(&ctx, |ec| ec.factory().get_tool_factory(&name))?;
        let Some(tool) = tool else {
            return Ok(JsValue::new_null(ctx.clone()));
        };
        let object = Object::new(ctx.clone())?;
        object.set("name", tool.name())?;
        let get = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<JsValue<'js>>| {
            get_tool(ctx, name.clone(), args)
        })?;
        object.set("getInstance", get)?;
        Ok(object.into_value())
    })
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, ec: &Object<'js>, factory: &Object<'js>) -> Result<(), Error> {
    let get = Function::new(ctx.clone(), get_tool).map_err(js_error)?;
    ec.set("getTool", get.clone()).map_err(js_error)?;
    factory.set("getTool", get).map_err(js_error)?;
    factory
        .set(
            "getToolFactory",
            Function::new(ctx.clone(), get_factory).map_err(js_error)?,
        )
        .map_err(js_error)
}
