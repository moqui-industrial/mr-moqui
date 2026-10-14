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

//! Whether Vue accepts a qvt template. The vendored compiler bundle (`webroot/libs/vue/compiler-dom.cjs.js`)
//! runs in a QuickJS runtime of its own and compiles each template as the browser build does, as the template of a
//! component. This is the check that "the qvt response is a component the Vue 3 compiler can compile" (Gate 3) runs
//! for the fixture screens, and the compiler the server-static work compiles with.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use rquickjs::{CatchResultExt, Context, Runtime};

use crate::context::Error;

const MEMORY_LIMIT: usize = 256 * 1024 * 1024;
const STACK_LIMIT: usize = 4 * 1024 * 1024;

/// Compile each template, wrapped in a `div` as the client wraps a screen, and return the compiler errors as
/// `"<index>: <code> <message> at <line>:<column>"`. An empty list means every template compiled.
///
/// # Errors
///
/// Returns an error when the compiler bundle does not load or the compile script itself fails.
pub fn compile_errors(compiler_source: &str, templates: &[String]) -> Result<Vec<String>, Error> {
    let runtime = Runtime::new().map_err(|err| Error::new(err.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    // A template that sends the compiler into a loop stops at the deadline instead of holding the worker.
    runtime.set_interrupt_handler(Some(Box::new(|| {
        COMPILE_DEADLINE.with(Cell::get).is_some_and(|at| Instant::now() >= at)
    })));
    let context = Context::full(&runtime).map_err(|err| Error::new(err.to_string()))?;
    let list = serde_json::to_string(templates).map_err(|err| Error::new(err.to_string()))?;
    context.with(|ctx| {
        ctx.eval::<(), _>(compiler_source)
            .catch(&ctx)
            .map_err(|err| Error::new(format!("the Vue compiler did not load: {err}")))?;
        let script = format!(
            "(function () {{ var errors = []; {list}.forEach(function (template, index) {{
                VueCompilerDOM.compile('<div>' + template + '</div>', {{ mode: 'function', hoistStatic: false,
                    onError: function (err) {{ errors.push(index + ': ' + err.code + ' ' + err.message + (err.loc ? ' at ' + err.loc.start.line + ':' + err.loc.start.column : '')); }} }});
            }}); return JSON.stringify(errors); }})()"
        );
        let json: String = ctx
            .eval(script)
            .catch(&ctx)
            .map_err(|err| Error::new(format!("the Vue compiler failed: {err}")))?;
        serde_json::from_str(&json).map_err(|err| Error::new(err.to_string()))
    })
}

/// A QuickJS runtime that has the compiler loaded, kept for the thread: loading the bundle costs more than a compile.
struct Compiler {
    context: Context,
    // The context borrows this runtime. Drop order frees the context first.
    #[allow(dead_code)]
    runtime: Runtime,
}

thread_local! {
    static COMPILER: RefCell<Option<(u64, Compiler)>> = const { RefCell::new(None) };
    static COMPILE_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// How long one template compile may run.
const COMPILE_TIME_LIMIT: Duration = Duration::from_secs(20);

fn compiler_for(source: &str) -> Result<Compiler, Error> {
    let runtime = Runtime::new().map_err(|err| Error::new(err.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    let context = Context::full(&runtime).map_err(|err| Error::new(err.to_string()))?;
    context.with(|ctx| {
        ctx.eval::<(), _>(source)
            .catch(&ctx)
            .map_err(|err| Error::new(format!("the Vue compiler did not load: {err}")))
    })?;
    Ok(Compiler { context, runtime })
}

/// The whole compiler text, so an edited bundle of the same length starts a new runtime. Compiles are rare: once per
/// screen definition that is server-static.
fn source_key(source: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

/// Compile a template to a render function, as the server-static qvt does when it caches a screen: `prefixIdentifiers`
/// output, so the code reads the scope as `_ctx.name` and has no `with`. The result is the body of a function of `Vue`
/// that returns `render(_ctx, _cache)`.
///
/// # Errors
///
/// Returns an error when the compiler does not load or reports any error for the template.
pub fn compile_render(compiler_source: &str, template: &str) -> Result<String, Error> {
    let key = source_key(compiler_source);
    COMPILER.with(|slot| {
        if slot.borrow().as_ref().is_none_or(|(held, _)| *held != key) {
            *slot.borrow_mut() = Some((key, compiler_for(compiler_source)?));
        }
        let guard = slot.borrow();
        let (_, compiler) = guard.as_ref().ok_or_else(|| Error::new("the Vue compiler did not start"))?;
        let input = serde_json::to_string(template).map_err(|err| Error::new(err.to_string()))?;
        let script = format!(
            "(function () {{ var errors = []; var result = VueCompilerDOM.compile({input}, {{ prefixIdentifiers: true, mode: 'function',
                onError: function (err) {{ errors.push(err.code + ' ' + err.message + (err.loc ? ' at ' + err.loc.start.line + ':' + err.loc.start.column : '')); }} }});
                return JSON.stringify({{ code: result.code, errors: errors }}); }})()"
        );
        COMPILE_DEADLINE.with(|deadline| deadline.set(Some(Instant::now() + COMPILE_TIME_LIMIT)));
        let json: Result<String, Error> = compiler.context.with(|ctx| {
            ctx.eval(script)
                .catch(&ctx)
                .map_err(|err| Error::new(format!("the Vue compiler failed: {err}")))
        });
        COMPILE_DEADLINE.with(|deadline| deadline.set(None));
        let json = json?;
        let parsed: serde_json::Value = serde_json::from_str(&json).map_err(|err| Error::new(err.to_string()))?;
        let errors: Vec<String> = parsed["errors"]
            .as_array()
            .map(|list| list.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if !errors.is_empty() {
            return Err(Error::new(format!("the Vue compiler rejected the template: {}", errors.join("; "))));
        }
        parsed["code"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| Error::new("the Vue compiler gave no code"))
    })
}
