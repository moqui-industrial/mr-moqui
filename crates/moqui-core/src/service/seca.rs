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

//! Service ECA. Rules are loaded from XML. `sync` does not call [`run_seca_rules`] yet.

use std::cell::RefCell;
use std::collections::BTreeMap;

#[cfg(test)]
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use crate::context::{Error, ExecutionContext};

#[derive(Clone, Debug)]
struct SecaRule {
    service_plain: String,
    when_name: String,
    name_is_pattern: bool,
    run_on_error: bool,
    priority: i32,
    condition: String,
    actions: String,
}

thread_local! {
    static RULES: RefCell<Vec<SecaRule>> = const { RefCell::new(Vec::new()) };
}

/// Replace the thread-local SECA rules with those in `xml`.
#[cfg(test)]
pub fn load_seca_rules(xml: &str) -> Result<(), Error> {
    let root = MNode::parse_text("secas", xml)?;
    let nodes = if root.name() == "seca" {
        vec![root]
    } else {
        root.children_named("seca")
    };
    let mut rules = nodes.iter().map(rule_from).collect::<Vec<_>>();
    rules.sort_by_key(|rule| rule.priority);
    RULES.with(|slot| *slot.borrow_mut() = rules);
    Ok(())
}

/// Run SECA actions whose service and `when` match. A false condition skips the rule.
pub fn run_seca_rules(
    service: &str,
    when: &str,
    parameters: &BTreeMap<String, Value>,
    results: Option<&BTreeMap<String, Value>>,
) -> Result<(), Error> {
    let mut rules = RULES.with(|slot| slot.borrow().clone());
    rules.sort_by_key(|rule| rule.priority);
    for rule in rules {
        if rule.when_name != when || !service_matches(&rule, service) {
            continue;
        }
        if !rule.run_on_error && message_has_error() {
            continue;
        }
        let _frame = super::callback::publish_names(parameters)?;
        let _results_frame = match results {
            Some(results) => Some(super::callback::publish_names(results)?),
            None => None,
        };
        if !super::callback::condition_allows(&rule.condition)? {
            continue;
        }
        super::callback::run_stored_actions(&rule.actions)?;
    }
    Ok(())
}

#[cfg(test)]
fn rule_from(node: &MNode) -> SecaRule {
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

    use super::{load_seca_rules, run_seca_rules};

    #[test]
    fn seca_for_a_test_service_runs_its_action() {
        load_seca_rules(
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
        )
        .unwrap();
        let context = crate::context::ExecutionContextFactory::test_context();
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
