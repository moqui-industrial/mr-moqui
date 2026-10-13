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

//! Email ECA. Actions run through the same XML action host as service ECA.

use std::cell::RefCell;
use std::collections::BTreeMap;

#[cfg(test)]
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use crate::context::Error;

#[derive(Clone, Debug)]
struct EmailRule {
    condition: String,
    actions: String,
}

thread_local! {
    static RULES: RefCell<Vec<EmailRule>> = const { RefCell::new(Vec::new()) };
}

/// Replace the thread-local email ECA rules with those in `xml`.
#[cfg(test)]
pub fn load_email_rules(xml: &str) -> Result<(), Error> {
    let root = MNode::parse_text("emecas", xml)?;
    let nodes = if root.name() == "emeca" {
        vec![root]
    } else {
        root.children_named("emeca")
    };
    let rules = nodes
        .iter()
        .map(|node| EmailRule {
            condition: node
                .first_named("condition")
                .map(|child| child.write_string())
                .unwrap_or_default(),
            actions: node
                .first_named("actions")
                .map(|child| child.write_string())
                .unwrap_or_default(),
        })
        .collect();
    RULES.with(|slot| *slot.borrow_mut() = rules);
    Ok(())
}

/// Run email ECA actions whose condition passes. `fields` are placed on the script context.
pub fn run_email_rules(fields: &BTreeMap<String, Value>) -> Result<(), Error> {
    let rules = RULES.with(|slot| slot.borrow().clone());
    for rule in rules {
        let _frame = super::callback::publish_names(fields)?;
        if !super::callback::condition_allows(&rule.condition)? {
            continue;
        }
        super::callback::run_stored_actions(&rule.actions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use moqui_lib::error::Error as LibError;
    use moqui_lib::object::Value;

    use super::{load_email_rules, run_email_rules};
    use crate::context::ExecutionContextFactory;
    use crate::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use crate::entity::{EntityFacade, EntityGroup};
    use crate::transaction::{PoolConfig, TransactionManager};

    #[test]
    fn email_eca_runs_its_action_and_mail_is_not_configured() {
        let _gate = crate::service::define::lock_tests();
        crate::service::define::reset();
        crate::service::load_definitions().unwrap();
        load_email_rules(
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
        )
        .unwrap();
        let facade = open();
        let context = ExecutionContextFactory::test_context();
        context.factory().set_entity(facade);
        let _entered = context.enter();
        // A failed service-call adds an error and the rule's generated code returns, as in Java.
        run_email_rules(&BTreeMap::new()).unwrap();
        let errors = context.message().errors().join("\n");
        assert!(errors.contains("mail is not configured"), "{errors}");
        assert!(context.message().messages().iter().any(|text| text == "emailEcaRan"));
    }

    fn open() -> EntityFacade {
        let driver: Arc<dyn DbDriver> = Arc::new(TinyDriver);
        let transactions = TransactionManager::new(
            driver,
            PoolConfig {
                min: 0,
                max: 4,
                reserve: 2,
                acquire_timeout: Duration::from_secs(5),
            },
        )
        .expect("memory pool");
        EntityFacade::open(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework"),
            "transactional",
            vec![EntityGroup::new("transactional", "postgres", transactions)],
        )
        .expect("entity facade")
    }

    struct TinyDriver;

    struct TinyConn;

    struct NoCancel;

    impl DbCancel for NoCancel {
        fn cancel(&self) -> Result<(), LibError> {
            Ok(())
        }
    }

    impl DbDriver for TinyDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, LibError> {
            Ok(Box::new(TinyConn))
        }
    }

    impl DbConnection for TinyConn {
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, LibError> {
            self.query(sql, params)
                .map(|rows| u64::try_from(rows.len()).unwrap_or(0))
        }

        fn query(&mut self, sql: &str, _params: &[Value]) -> Result<Vec<DbRow>, LibError> {
            if sql.trim().eq_ignore_ascii_case("SELECT 1") {
                return Ok(vec![DbRow {
                    columns: vec![("1".to_string(), Value::I64(1))],
                }]);
            }
            Ok(Vec::new())
        }

        fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, LibError> {
            self.query(sql, params)
        }

        fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), LibError> {
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, LibError> {
            Ok(None)
        }

        fn close_stream(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn stream_open(&self) -> bool {
            false
        }

        fn begin(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn commit(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), LibError> {
            Ok(())
        }

        fn savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn release_savepoint(&mut self, _name: &str) -> Result<(), LibError> {
            Ok(())
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(NoCancel)
        }
    }
}
