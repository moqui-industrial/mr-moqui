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

//! Email ECA. Rules come from the factory's `*.emecas.xml` files. Actions run through the same
//! XML action host as service ECA.

use std::collections::BTreeMap;

use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::eca_rules;
use crate::context::Error;

#[derive(Clone, Debug)]
pub(super) struct EmailRule {
    condition: String,
    actions: String,
}

pub(super) fn email_rule_from(node: &MNode) -> EmailRule {
    EmailRule {
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

/// Run email ECA actions whose condition passes. `fields` are placed on the script context.
pub fn run_email_rules(fields: &BTreeMap<String, Value>) -> Result<(), Error> {
    let Some(rules) = eca_rules::current() else {
        return Ok(());
    };
    for rule in &rules.email {
        let _frame = super::rule_actions::publish_names(fields)?;
        if !super::rule_actions::condition_allows(&rule.condition)? {
            continue;
        }
        super::rule_actions::run_stored_actions(&rule.actions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::run_email_rules;
    use crate::context::ExecutionContextFactory;
    use crate::service::eca_rules;

    use crate::entity::EntityFacade;

    #[test]
    fn email_eca_runs_its_action_and_mail_is_not_configured() {
        let facade = open();
        let context = ExecutionContextFactory::test_context_in(
            "",
            crate::service::ServicePools::idle(),
            crate::context::framework_dir(),
        );
        eca_rules::install(
            context.factory(),
            eca_rules::from_xml(
                "",
                r#"<emecas>
                <emeca rule-name="SendNotice">
                    <condition><expression>true</expression></condition>
                    <actions>
                        <script>ec.message.addMessage("emailEcaRan");</script>
                        <service-call name="org.moqui.impl.EmailServices.send#EmailTemplate">
                            <field-map field-name="emailTemplateId" value="T1"/>
                            <field-map field-name="toAddresses" value="a@example.com"/>
                        </service-call>
                    </actions>
                </emeca>
            </emecas>"#,
            ),
        );
        context.factory().set_entity(facade);
        let _entered = context.enter();
        crate::service::load_definitions().unwrap();
        // A failed service-call adds an error and the rule's generated code returns, as in Java.
        run_email_rules(&BTreeMap::new()).unwrap();
        let errors = context.message().errors().join("\n");
        assert!(
            errors.contains("No EmailTemplate record found") || errors.contains("screen mail body is not available"),
            "{errors}"
        );
        assert!(!errors.contains("mail is not configured"), "{errors}");
        assert!(context.message().messages().iter().any(|text| text == "emailEcaRan"));
    }

    fn open() -> EntityFacade {
        crate::testing::open()
    }
}
