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

//! Examples of Rust in a component. The binary registers [`tool_factories`] under the component name `example`
//! (`moqui_core::context::register_component_tools`), and each factory boot that loads the component makes them.

mod example_tool;
pub mod services;

use moqui_core::context::ToolFactory;

/// The tool factories of this component, made again for each execution context factory.
#[must_use]
pub fn tool_factories() -> Vec<Box<dyn ToolFactory>> {
    vec![Box::new(example_tool::ExampleToolFactory::default())]
}
