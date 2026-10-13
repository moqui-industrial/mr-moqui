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
