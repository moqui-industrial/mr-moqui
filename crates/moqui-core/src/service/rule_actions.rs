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

//! Running action XML for rules: the frame that holds a rule's parameters, its condition, and its
//! actions. SECA, email ECA, and entity ECA share it.

use std::collections::BTreeMap;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

/// Push a script frame holding `values`. The frame pops when the guard drops.
pub(crate) fn publish_names(
    values: &BTreeMap<String, Value>,
) -> Result<crate::script::ContextFrame, crate::context::Error> {
    let frame = crate::script::ContextFrame::push();
    for (name, value) in values {
        crate::script::set_context_name(name, value.clone())?;
    }
    Ok(frame)
}

/// An empty condition passes. XML is parsed so only the expression text is evaluated.
pub(crate) fn condition_allows(condition: &str) -> Result<bool, crate::context::Error> {
    let trimmed = condition.trim();
    if trimmed.is_empty() {
        return Ok(true);
    }
    let expression = if trimmed.starts_with('<') {
        expression_text(trimmed)?
    } else {
        trimmed.to_string()
    };
    let expression = expression.trim();
    if expression.is_empty() {
        return Ok(true);
    }
    crate::script::eval_condition(expression)
}

fn expression_text(xml: &str) -> Result<String, crate::context::Error> {
    let root = MNode::parse_text("condition", xml)?;
    let mut parts = Vec::new();
    for child in root.children() {
        if child.name() != "expression" && child.name() != "script" {
            continue;
        }
        let Some(text) = child.text() else {
            continue;
        };
        let text = text.trim();
        if !text.is_empty() {
            parts.push(text.to_string());
        }
    }
    if parts.len() <= 1 {
        return Ok(parts.pop().unwrap_or_default());
    }
    Ok(parts
        .into_iter()
        .map(|part| format!("({part})"))
        .collect::<Vec<_>>()
        .join(" && "))
}

/// Run action XML on the shared action runner. A bare element is wrapped in `<actions>`.
pub(crate) fn run_stored_actions(actions: &str) -> Result<(), crate::context::Error> {
    let trimmed = actions.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.starts_with("<actions") {
        return crate::script::run_actions(trimmed);
    }
    crate::script::run_actions(&format!("<actions>{trimmed}</actions>"))
}
