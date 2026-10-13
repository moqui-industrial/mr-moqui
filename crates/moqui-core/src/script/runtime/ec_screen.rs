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

//! `ec.screen.makeRender()` and the calls around it, for services that render a screen to text:
//! the scheduled screens and email bodies. `ec.screen.render(options)` is the host call and
//! `makeRender` is a chain of setters over it. A render mode named `xsl-fo` is `html-print`.

use super::*;

const BUILDER: &str = r#"(function (ec) {
    ec.screen.makeRender = () => {
        const o = {};
        const b = {
            webappName: (v) => { o.webappName = v; return b; },
            rootScreen: (v) => { o.rootScreen = v; return b; },
            rootScreenFromHost: (v) => { o.rootScreenFromHost = v; return b; },
            screenPath: (v) => { o.screenPath = v; return b; },
            renderMode: (v) => { o.renderMode = v; return b; },
            baseLinkUrl: (v) => { o.baseLinkUrl = v; return b; },
            lastStandalone: (v) => { o.lastStandalone = String(v); return b; },
            parameters: (v) => { o.parameters = v; return b; },
            render: () => ec.screen.render(o).text
        };
        return b;
    };
})"#;

/// `ec.screen.render(options)`: `{screenPath, renderMode, rootScreen, baseLinkUrl, lastStandalone,
/// parameters}` to `{status, contentType, text}`. The render runs in this execution context.
fn screen_render<'js>(ctx: Ctx<'js>, options: JsValue<'js>) -> rquickjs::Result<Object<'js>> {
    host_guard(&ctx, || {
        let Value::Map(options) = js_to_host(&ctx, &options)? else {
            return Err(Exception::throw_message(&ctx, "ec.screen.render needs an options map"));
        };
        let text = |name: &str| {
            options
                .get(name)
                .map(moqui_lib::object::to_plain_string)
                .unwrap_or_default()
        };
        let mut parameters = std::collections::BTreeMap::new();
        if let Some(Value::Map(given)) = options.get("parameters") {
            parameters.extend(given.iter().map(|(key, value)| (key.clone(), value.clone())));
        }
        if !text("lastStandalone").is_empty() {
            parameters.insert("lastStandalone".to_string(), Value::String(text("lastStandalone")));
        }
        let factory = with_ec(&ctx, |ec| Arc::clone(ec.factory()))?;
        let mut render = crate::screen::make_render(&factory)
            .screen_path(text("screenPath"))
            .parameters(parameters)
            .base_url(text("baseLinkUrl"));
        if !text("rootScreen").is_empty() {
            render = render.root_screen(text("rootScreen"));
        }
        if !text("renderMode").is_empty() {
            render = render.render_mode(text("renderMode"));
        }
        let output = render
            .render()
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        let result = Object::new(ctx.clone())?;
        result.set("status", i32::from(output.status))?;
        result.set("contentType", output.content_type().unwrap_or_default())?;
        result.set("text", output.body_text())?;
        Ok(result)
    })
}

fn screen_mime_type_by_mode(ctx: Ctx<'_>, mode: String) -> rquickjs::Result<Option<String>> {
    host_guard(&ctx, || {
        with_ec(&ctx, |ec| {
            crate::screen::ScreenFacade::of(ec.factory())
                .mime_type(&mode)
                .map(str::to_string)
        })
    })
}

/// A mode that writes text is every configured mode: the print mode is HTML until PDF exists.
fn screen_is_render_mode_text(ctx: Ctx<'_>, mode: String) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        with_ec(&ctx, |ec| {
            crate::screen::ScreenFacade::of(ec.factory())
                .mime_type(&mode)
                .is_some_and(|mime| mime.starts_with("text/") || mime == "application/javascript")
        })
    })
}

/// `ec.resource.xslFoTransform(...)`: PDF bytes, which arrive with phase 12.
pub(super) fn resource_xsl_fo_transform(ctx: Ctx<'_>, _rest: Rest<JsValue<'_>>) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        crate::screen::PdfRenderer::render("")
            .map(|_| ())
            .map_err(|err| Exception::throw_message(&ctx, err.message()))
    })
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, screen: &Object<'js>) -> Result<(), Error> {
    let render = caught(ctx, "screen.render", Function::new(ctx.clone(), screen_render))?;
    caught(ctx, "screen.render", screen.set("render", render))?;
    let mime = caught(
        ctx,
        "screen.getMimeTypeByMode",
        Function::new(ctx.clone(), screen_mime_type_by_mode),
    )?;
    caught(ctx, "screen.getMimeTypeByMode", screen.set("getMimeTypeByMode", mime))?;
    let text = caught(
        ctx,
        "screen.isRenderModeText",
        Function::new(ctx.clone(), screen_is_render_mode_text),
    )?;
    caught(ctx, "screen.isRenderModeText", screen.set("isRenderModeText", text))
}

/// Define the `makeRender` chain once `ec` is a global.
pub(super) fn install_builder(ctx: &Ctx<'_>) -> Result<(), Error> {
    let define: Function = eval_js(ctx, BUILDER)?;
    let ec: Object = caught(ctx, "ec", ctx.globals().get("ec"))?;
    caught(ctx, "ec.screen.makeRender", define.call::<_, ()>((ec,)))
}
