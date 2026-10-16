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

use moqui_core::context::ComponentTools;

/// The component crates linked into this binary, by component name. `./moq build` generates a binary with the same
/// list plus the add-on component crates in runtime/component (target/moqui-app).
static COMPONENT_TOOLS: [(&str, ComponentTools); 1] = [("example", example::tool_factories)];

fn main() {
    if let Err(err) = moqui_core::context::register_component_tools(&COMPONENT_TOOLS) {
        eprintln!("{err}");
        std::process::exit(1);
    }
    moqui_host::main();
}
