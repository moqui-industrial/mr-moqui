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

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::eca_rules;
use super::util::has_error;
use crate::context::Error;

#[derive(Clone, Debug)]
pub(super) struct SecaRule {
    id: String,
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
    for &position in rules.seca_index.matching(&rules.seca, service, when).iter() {
        let rule = &rules.seca[position];
        if !rule.run_on_error && has_error() {
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
        let in_rule = |err: Error| {
            Error::new(format!(
                "service ECA {} on {when} of {service}: {}",
                rule.id,
                err.message()
            ))
        };
        if !super::rule_actions::condition_allows(&rule.condition).map_err(in_rule)? {
            continue;
        }
        super::rule_actions::run_stored_actions(&rule.actions).map_err(in_rule)?;
    }
    Ok(())
}

/// The SECA rules by `when` and service, made once when the rules load, so a call does not scan every rule at each of
/// its phases. Each list holds positions in the priority order.
#[derive(Default)]
pub(super) struct SecaIndex {
    /// By `when`, then by the service name without `#`.
    exact: HashMap<String, HashMap<String, Vec<usize>>>,
    /// The rules whose name is a pattern, by `when`.
    patterns: HashMap<String, Vec<usize>>,
}

impl SecaIndex {
    pub(super) fn new(rules: &[Arc<SecaRule>]) -> Self {
        let mut index = Self::default();
        for (position, rule) in rules.iter().enumerate() {
            let when = rule.when_name.clone();
            if rule.name_is_pattern {
                index.patterns.entry(when).or_default().push(position);
            } else {
                index
                    .exact
                    .entry(when)
                    .or_default()
                    .entry(rule.service_plain.clone())
                    .or_default()
                    .push(position);
            }
        }
        index
    }

    /// The positions of the rules of `rules`, the list this index was made from, that run for `service` at `when`,
    /// in priority order.
    fn matching<'a>(&'a self, rules: &[Arc<SecaRule>], service: &str, when: &str) -> Cow<'a, [usize]> {
        let patterns = self.patterns.get(when).map_or(&[][..], Vec::as_slice);
        let exact = self.exact.get(when);
        if patterns.is_empty() && exact.is_none() {
            return Cow::Borrowed(&[]);
        }
        let plain = service.replace('#', "");
        let named = exact
            .and_then(|by_service| by_service.get(&plain))
            .map_or(&[][..], Vec::as_slice);
        let mut positions: Vec<usize> = patterns
            .iter()
            .copied()
            .filter(|position| pattern_matches(&rules[*position], &plain))
            .collect();
        if positions.is_empty() {
            return Cow::Borrowed(named);
        }
        positions.extend_from_slice(named);
        positions.sort_unstable();
        Cow::Owned(positions)
    }
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
        id: node.attribute("id").unwrap_or_default(),
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
            if self.name_is_pattern {
                " name-is-pattern=\"true\""
            } else {
                ""
            },
            if self.run_on_error {
                " run-on-error=\"true\""
            } else {
                ""
            },
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
    pattern_matches(rule, &plain)
}

/// Whether the rule's name pattern matches `plain`, a service name without `#`.
fn pattern_matches(rule: &SecaRule, plain: &str) -> bool {
    moqui_lib::object::compare(
        &Value::string(plain),
        "matches",
        &Value::string(&rule.service_plain),
        "String",
    )
    .unwrap_or(false)
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

    #[test]
    fn pattern_and_named_rules_run_together_in_priority_order() {
        let context = crate::context::ExecutionContextFactory::test_context();
        eca_rules::install(
            context.factory(),
            eca_rules::from_xml(
                r#"<secas>
                <seca service="test.example.Thing.run#Run" when="pre-service" priority="3">
                    <actions><script>ec.message.addMessage("named 3");</script></actions>
                </seca>
                <seca service="test\.example\..*" name-is-pattern="true" when="pre-service" priority="2">
                    <actions><script>ec.message.addMessage("pattern 2");</script></actions>
                </seca>
                <seca service="test.example.Thing.run#Run" when="pre-service" priority="1">
                    <actions><script>ec.message.addMessage("named 1");</script></actions>
                </seca>
                <seca service="test.example.Thing.run#Other" when="pre-service" priority="1">
                    <actions><script>ec.message.addMessage("other");</script></actions>
                </seca>
                <seca service="test\.example\..*" name-is-pattern="true" when="post-service" priority="1">
                    <actions><script>ec.message.addMessage("post");</script></actions>
                </seca>
            </secas>"#,
                "",
            ),
        );
        let _entered = context.enter();
        let parameters = BTreeMap::new();
        let run = |service: &str, when: &str| {
            context.message().clear();
            run_seca_rules(service, when, &parameters, None).unwrap();
            context.message().messages().to_vec()
        };
        assert_eq!(
            run("test.example.Thing.run#Run", "pre-service"),
            ["named 1", "pattern 2", "named 3"]
        );
        assert_eq!(
            run("test.example.Thing.run#Other", "pre-service"),
            ["other", "pattern 2"]
        );
        assert_eq!(run("test.other.Thing.run#Run", "pre-service"), Vec::<String>::new());
        assert_eq!(run("test.example.Thing.run#Run", "post-service"), ["post"]);
        assert!(run("test.example.Thing.run#Run", "pre-validate").is_empty());
    }

    #[test]
    fn compare_and_combined_conditions_decide_whether_a_rule_runs() {
        let context = crate::context::ExecutionContextFactory::test_context();
        eca_rules::install(
            context.factory(),
            eca_rules::from_xml(
                r#"<secas>
                <seca service="test.example#Cond" when="pre-service" priority="1">
                    <condition><compare field="note" operator="equals" value="go"/></condition>
                    <actions><script>ec.message.addMessage("compare");</script></actions>
                </seca>
                <seca service="test.example#Cond" when="pre-service" priority="2">
                    <condition><and>
                        <compare field="note" operator="equals" value="go"/>
                        <expression>count &gt; 2</expression>
                    </and></condition>
                    <actions><script>ec.message.addMessage("and");</script></actions>
                </seca>
                <seca service="test.example#Cond" when="pre-service" priority="3">
                    <condition><or>
                        <compare field="note" operator="equals" value="stop"/>
                        <not><expression>count &gt; 2</expression></not>
                    </or></condition>
                    <actions><script>ec.message.addMessage("or");</script></actions>
                </seca>
            </secas>"#,
                "",
            ),
        );
        let _entered = context.enter();
        let run = |note: &str, count: i64| {
            context.message().clear();
            let mut parameters = BTreeMap::new();
            parameters.insert("note".to_string(), Value::string(note));
            parameters.insert("count".to_string(), Value::I64(count));
            run_seca_rules("test.example#Cond", "pre-service", &parameters, None).unwrap();
            context.message().messages().to_vec()
        };
        assert_eq!(run("go", 5), ["compare", "and"]);
        assert_eq!(run("go", 1), ["compare", "or"]);
        assert_eq!(run("stop", 5), ["or"]);
        assert!(run("other", 5).is_empty());
    }

    #[test]
    fn a_condition_element_that_is_not_a_condition_fails_the_rule() {
        let context = crate::context::ExecutionContextFactory::test_context();
        eca_rules::install(
            context.factory(),
            eca_rules::from_xml(
                r#"<secas>
                <seca service="test.example#Odd" when="pre-service">
                    <condition><unknown-check field="note"/></condition>
                    <actions><script>ec.message.addMessage("ran");</script></actions>
                </seca>
            </secas>"#,
                "",
            ),
        );
        let _entered = context.enter();
        let parameters = BTreeMap::new();
        assert!(run_seca_rules("test.example#Odd", "pre-service", &parameters, None).is_err());
        assert!(context.message().messages().is_empty(), "the rule did not run");
    }
}
