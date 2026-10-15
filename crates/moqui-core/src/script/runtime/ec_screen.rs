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
            render: () => ec.screen.render(o).text,
            // The bytes of the render: a binary mode's document such as xlsx, or a text mode's UTF-8.
            renderBytes: () => ec.screen.render(o).bytes
        };
        return b;
    };
    // Java's ScreenTest: render paths under a base screen path as the current user, with the errors of each render
    // collected on the result instead of thrown, and counts over the renders.
    ec.screen.makeTest = () => {
        const o = {};
        const t = {
            rootScreen: (v) => { o.rootScreen = v; return t; },
            baseScreenPath: (v) => { o.baseScreenPath = v; return t; },
            renderMode: (v) => { o.renderMode = v; return t; },
            baseLinkUrl: (v) => { o.baseLinkUrl = v; return t; },
            renderCount: 0, errorCount: 0, renderTotalChars: 0, startTime: Date.now(),
            render: (screenPath, parameters, requestMethod) => {
                const r = ec.screen.testRender(Object.assign({}, o,
                    { screenPath: String(screenPath), parameters: parameters || {}, method: requestMethod || "" }));
                t.renderCount++;
                t.errorCount += r.errorMessages.length;
                t.renderTotalChars += r.output ? r.output.length : 0;
                const str = {
                    screenPath: String(screenPath), output: r.output, errorMessages: r.errorMessages,
                    renderTime: r.renderTime, status: r.status,
                    assertContains: (text) => !!r.output && r.output.includes(text),
                    assertNotContains: (text) => !r.output || !r.output.includes(text),
                    assertNoErrors: () => r.errorMessages.length === 0,
                    getOutput: () => r.output, getErrorMessages: () => r.errorMessages, getRenderTime: () => r.renderTime
                };
                // the response parsed as JSON, or null when it is not JSON
                Object.defineProperty(str, "jsonObject", { get: () => { try { return JSON.parse(r.output); } catch (e) { return null; } } });
                return str;
            },
            renderAll: (screenPaths, parameters, requestMethod) => {
                for (const screenPath of screenPaths) t.render(screenPath, parameters, requestMethod);
            },
            getRenderCount: () => t.renderCount, getErrorCount: () => t.errorCount,
            getRenderTotalChars: () => t.renderTotalChars, getStartTime: () => t.startTime
        };
        return t;
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
        result.set("bytes", host_to_js(&ctx, Value::Bytes(output.body))?)?;
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

/// `ec.screen.isRenderModeBinary(mode)`: a `screen-output` mode such as `xlsx`, whose render is bytes.
fn screen_is_render_mode_binary(ctx: Ctx<'_>, mode: String) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        with_ec(&ctx, |ec| {
            crate::screen::ScreenFacade::of(ec.factory()).is_render_mode_binary(&mode)
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

/// `ec.resource.xslFoTransform(document, xslt, out, contentType)`: the PDF bytes of an `html-print` document (the
/// old name `xsl-fo`), as a `Uint8Array`. Java writes to the `out` stream; a script has none, so the bytes are the
/// return value and `out` is ignored. Java's XSLT step was an identity transform every caller skipped with null, so a
/// stylesheet is an error, as is any content type but PDF.
pub(super) fn resource_xsl_fo_transform<'js>(
    ctx: Ctx<'js>,
    document: JsValue<'js>,
    xslt: Opt<JsValue<'js>>,
    _out: Opt<JsValue<'js>>,
    content_type: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        if xslt
            .0
            .as_ref()
            .is_some_and(|value| !value.is_null() && !value.is_undefined())
        {
            return Err(Exception::throw_message(
                &ctx,
                "xslFoTransform does not apply an XSLT stylesheet; pass null",
            ));
        }
        let content_type = match content_type.0.as_ref().and_then(JsValue::as_string) {
            Some(text) => text.to_string()?,
            None => String::new(),
        };
        if !content_type.is_empty() && content_type != "application/pdf" {
            return Err(Exception::throw_message(
                &ctx,
                &format!("xslFoTransform makes application/pdf, not {content_type}"),
            ));
        }
        let Some(text) = document.as_string() else {
            return Err(Exception::throw_message(
                &ctx,
                "xslFoTransform needs the html-print document text",
            ));
        };
        let text = text.to_string()?;
        let bytes =
            crate::screen::PdfRenderer::render(&text).map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        host_to_js(&ctx, Value::Bytes(bytes))
    })
}

/// `ec.screen.testRender({screenPath, parameters, method, rootScreen, baseScreenPath, renderMode, baseLinkUrl})`, the
/// host side of `makeTest().render()`: `{output, errorMessages, renderTime, status}`. It does not throw for a failed
/// render; the failure is in `errorMessages`.
fn screen_test_render<'js>(ctx: Ctx<'js>, options: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let Value::Map(options) = js_to_host(&ctx, &options)? else {
            return Err(Exception::throw_message(
                &ctx,
                "ec.screen.testRender needs an options map",
            ));
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
        let factory = with_ec(&ctx, |ec| Arc::clone(ec.factory()))?;
        let mut test = crate::screen::ScreenTest::new(&factory)
            .base_screen_path(text("baseScreenPath"))
            .base_url(text("baseLinkUrl"));
        if !text("rootScreen").is_empty() {
            test = test.root_screen(text("rootScreen"));
        }
        if !text("renderMode").is_empty() {
            test = test.render_mode(text("renderMode"));
        }
        let method = text("method");
        let rendered = test.test_render(&text("screenPath"), parameters, Some(method.as_str()));
        let mut out = indexmap::IndexMap::new();
        out.insert(
            "output".to_string(),
            if rendered.output.is_some() {
                Value::String(rendered.text())
            } else {
                Value::Null
            },
        );
        out.insert(
            "errorMessages".to_string(),
            Value::List(rendered.error_messages.into_iter().map(Value::String).collect()),
        );
        out.insert(
            "renderTime".to_string(),
            Value::I64(i64::try_from(rendered.render_time_ms).unwrap_or(i64::MAX)),
        );
        out.insert(
            "status".to_string(),
            rendered
                .output
                .as_ref()
                .map_or(Value::Null, |output| Value::I64(i64::from(output.status))),
        );
        host_to_js(&ctx, Value::Map(out))
    })
}

pub(super) fn install<'js>(ctx: &Ctx<'js>, screen: &Object<'js>) -> Result<(), Error> {
    let render = caught(ctx, "screen.render", Function::new(ctx.clone(), screen_render))?;
    caught(ctx, "screen.render", screen.set("render", render))?;
    let test_render = caught(ctx, "screen.testRender", Function::new(ctx.clone(), screen_test_render))?;
    caught(ctx, "screen.testRender", screen.set("testRender", test_render))?;
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
    caught(ctx, "screen.isRenderModeText", screen.set("isRenderModeText", text))?;
    let binary = caught(
        ctx,
        "screen.isRenderModeBinary",
        Function::new(ctx.clone(), screen_is_render_mode_binary),
    )?;
    caught(
        ctx,
        "screen.isRenderModeBinary",
        screen.set("isRenderModeBinary", binary),
    )
}

/// Define the `makeRender` chain once `ec` is a global.
pub(super) fn install_builder(ctx: &Ctx<'_>) -> Result<(), Error> {
    let define: Function = eval_js(ctx, BUILDER)?;
    let ec: Object = caught(ctx, "ec", ctx.globals().get("ec"))?;
    caught(ctx, "ec.screen.makeRender", define.call::<_, ()>((ec,)))
}
