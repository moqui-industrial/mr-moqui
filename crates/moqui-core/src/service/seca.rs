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

//! Service ECA. Rules come from the factory's `*.secas.xml` files (see `eca_rules`).

use std::collections::BTreeMap;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::eca_rules;
use crate::context::{Error, ExecutionContext};

#[derive(Clone, Debug)]
pub(super) struct SecaRule {
    service_plain: String,
    when_name: String,
    name_is_pattern: bool,
    run_on_error: bool,
    pub(super) priority: i32,
    condition: String,
    actions: String,
}

/// Run SECA actions whose service and `when` match. A false condition skips the rule.
///
/// Each rule runs in its own context frame holding the parameters, `parameters`, and, for the
/// rules that run after the service, the results and `results`.
pub fn run_seca_rules(
    service: &str,
    when: &str,
    parameters: &BTreeMap<String, Value>,
    results: Option<&BTreeMap<String, Value>>,
) -> Result<(), Error> {
    let Some(rules) = eca_rules::current() else {
        return Ok(());
    };
    for rule in &rules.seca {
        if rule.when_name != when || !service_matches(rule, service) {
            continue;
        }
        if !rule.run_on_error && message_has_error() {
            continue;
        }
        let _frame = super::rule_actions::publish_names(parameters)?;
        crate::script::set_context_name("parameters", Value::Map(to_index_map(parameters)))?;
        if let Some(results) = results {
            for (name, value) in results {
                crate::script::set_context_name(name, value.clone())?;
            }
            crate::script::set_context_name("results", Value::Map(to_index_map(results)))?;
        }
        if !super::rule_actions::condition_allows(&rule.condition)? {
            continue;
        }
        super::rule_actions::run_stored_actions(&rule.actions)?;
    }
    Ok(())
}

fn to_index_map(values: &BTreeMap<String, Value>) -> indexmap::IndexMap<String, Value> {
    values
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(super) fn seca_rule_from(node: &MNode) -> SecaRule {
    let service = node.attribute("service").unwrap_or_default();
    SecaRule {
        service_plain: service.replace('#', ""),
        when_name: node.attribute("when").unwrap_or_default(),
        name_is_pattern: node.attribute("name-is-pattern").as_deref() == Some("true"),
        run_on_error: node.attribute("run-on-error").as_deref() == Some("true"),
        priority: node
            .attribute("priority")
            .and_then(|text| text.parse().ok())
            .unwrap_or(5),
        condition: node
            .first_named("condition")
            .map(|child| child.write_string())
            .unwrap_or_default(),
        actions: node
            .first_named("actions")
            .map(|child| child.write_string())
            .unwrap_or_default(),
    }
}

impl SecaRule {
    /// Whether the rule runs for `service`, at any `when`.
    pub(super) fn applies_to(&self, service: &str) -> bool {
        service_matches(self, service)
    }

    /// The rule as XML, for the tools screens that list rules.
    pub(super) fn describe(&self) -> String {
        let mut text = format!(
            "<seca service=\"{}\" when=\"{}\" priority=\"{}\"{}{}>",
            self.service_plain,
            self.when_name,
            self.priority,
            if self.name_is_pattern { " name-is-pattern=\"true\"" } else { "" },
            if self.run_on_error { " run-on-error=\"true\"" } else { "" },
        );
        text.push_str(&self.condition);
        text.push_str(&self.actions);
        text.push_str("</seca>");
        text
    }
}

fn service_matches(rule: &SecaRule, service: &str) -> bool {
    let plain = service.replace('#', "");
    if !rule.name_is_pattern {
        return rule.service_plain == plain;
    }
    moqui_lib::object::compare(
        &Value::string(&plain),
        "matches",
        &Value::string(&rule.service_plain),
        "String",
    )
    .unwrap_or(false)
}

fn message_has_error() -> bool {
    ExecutionContext::with_current(|ctx| !ctx.message().errors().is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use moqui_lib::object::Value;

    use super::run_seca_rules;
    use crate::service::eca_rules;

    #[test]
    fn seca_for_a_test_service_runs_its_action() {
        let context = crate::context::ExecutionContextFactory::test_context();
        eca_rules::install(
            context.factory(),
            eca_rules::from_xml(
                r#"<secas>
                <seca service="test.example#Run" when="pre-service" priority="8">
                    <actions><script>ec.message.addMessage("late");</script></actions>
                </seca>
                <seca service="test.example#Run" when="pre-service" priority="1">
                    <condition><expression>false</expression></condition>
                    <actions><script>ec.message.addMessage("skipped");</script></actions>
                </seca>
                <seca service="test.example#Run" when="pre-service" priority="2">
                    <actions><script>ec.message.addMessage("early");</script></actions>
                </seca>
            </secas>"#,
                "",
            ),
        );
        let _entered = context.enter();
        let mut parameters = BTreeMap::new();
        parameters.insert("note".to_string(), Value::string("go"));
        run_seca_rules("test.example#Run", "pre-service", &parameters, None).unwrap();
        // Priority order, and the false condition skips its rule.
        assert_eq!(context.message().messages(), ["early", "late"]);
        run_seca_rules("test.example#Run", "post-service", &parameters, None).unwrap();
        assert_eq!(context.message().messages(), ["early", "late"]);
    }
}
