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

//! Global code compiled once per engine. QuickJS compiles a script to a bytecode object
//! (`JS_EVAL_FLAG_COMPILE_ONLY`), and `JS_EvalFunction` runs that object as global code, with the
//! same semantics as evaluating the text: a bare assignment still reaches the context stack. Java
//! compiles XML actions and Groovy expressions to classes once; this is the same saving for the
//! action scripts, conditions, and expressions that run on every request and every form-list row.
//!
//! rquickjs does not wrap these two calls for scripts, so they are made through `rquickjs::qjs`.

use std::ffi::CString;

use rquickjs::{qjs, CatchResultExt, Ctx, FromJs, Persistent, Value as JsValue};

use super::{current_engine, Error};

/// Compiled scripts kept per engine.
pub(super) const MAX_COMPILED_SCRIPTS: usize = 4096;

/// Run `source` as global code, compiled on first use and kept by its text and strictness.
pub(super) fn eval_compiled<'js, T>(ctx: &Ctx<'js>, source: &str, strict: bool) -> Result<T, Error>
where
    T: FromJs<'js>,
{
    let key = if strict {
        format!("s{source}")
    } else {
        format!("l{source}")
    };
    let engine = current_engine();
    let saved = engine
        .as_ref()
        .and_then(|engine| engine.scripts.borrow_mut().get(key.as_str()));
    let compiled = match saved {
        Some(saved) => saved.restore(ctx).map_err(|err| Error::new(err.to_string()))?,
        None => {
            let compiled = compile_global(ctx, source, strict)?;
            if let Some(engine) = &engine {
                engine
                    .scripts
                    .borrow_mut()
                    .insert(key, Persistent::save(ctx, compiled.clone()));
            }
            compiled
        }
    };
    let value = run_compiled(ctx, &compiled)?;
    T::from_js(ctx, value)
        .catch(ctx)
        .map_err(|caught| Error::new(caught.to_string()))
}

fn compile_global<'js>(ctx: &Ctx<'js>, source: &str, strict: bool) -> Result<JsValue<'js>, Error> {
    let text = CString::new(source).map_err(|_| Error::new("script source contains a null byte"))?;
    let mut flags = qjs::JS_EVAL_TYPE_GLOBAL | qjs::JS_EVAL_FLAG_COMPILE_ONLY;
    if strict {
        flags |= qjs::JS_EVAL_FLAG_STRICT;
    }
    let flags = i32::try_from(flags).map_err(|_| Error::new("bad eval flags"))?;
    // SAFETY: the context pointer is the live context of `ctx`, the source is a NUL-terminated
    // copy whose length excludes the NUL, and the file name is a static C string.
    let raw = unsafe {
        qjs::JS_Eval(
            ctx.as_raw().as_ptr(),
            text.as_ptr(),
            text.as_bytes().len() as _,
            c"eval_script".as_ptr(),
            flags,
        )
    };
    owned_or_exception(ctx, raw)
}

fn run_compiled<'js>(ctx: &Ctx<'js>, compiled: &JsValue<'js>) -> Result<JsValue<'js>, Error> {
    // SAFETY: `JS_EvalFunction` takes ownership of the object it is given, so it gets a new
    // reference and the cached bytecode keeps its own.
    let raw = unsafe {
        let function = qjs::JS_DupValue(ctx.as_raw().as_ptr(), compiled.as_raw());
        qjs::JS_EvalFunction(ctx.as_raw().as_ptr(), function)
    };
    owned_or_exception(ctx, raw)
}

/// Wrap an owned value from the C API, or turn its pending exception into an error.
fn owned_or_exception<'js>(ctx: &Ctx<'js>, raw: qjs::JSValue) -> Result<JsValue<'js>, Error> {
    // SAFETY: `raw` is an owned value just returned by the C API for this context.
    if unsafe { qjs::JS_IsException(raw) } {
        return Err(Err::<(), _>(rquickjs::Error::Exception)
            .catch(ctx)
            .map_or_else(|caught| Error::new(caught.to_string()), |()| Error::new("script failed")));
    }
    // SAFETY: as above; the new value owns the reference.
    Ok(unsafe { JsValue::from_raw(ctx.clone(), raw) })
}
