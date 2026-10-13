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

//! Service and email ECA rules, loaded once per factory from every component's `service/` directory
//! (`*.secas.xml` and `*.emecas.xml`), as `ServiceFacadeImpl` does.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use moqui_lib::node::MNode;

use super::email_eca::{email_rule_from, EmailRule};
use super::seca::{seca_rule_from, SecaRule};
use crate::context::{ExecutionContext, ExecutionContextFactory};

#[derive(Default)]
pub(super) struct EcaRules {
    /// Rules in priority order, a later rule with the same id having replaced an earlier one.
    pub(super) seca: Vec<Arc<SecaRule>>,
    pub(super) email: Vec<Arc<EmailRule>>,
}

type Shared = RwLock<Arc<EcaRules>>;

/// The rules of the entered context's factory. `None` when no context is entered.
pub(super) fn current() -> Option<Arc<EcaRules>> {
    ExecutionContext::with_current(|ctx| rules_of(ctx.factory()))
}

fn rules_of(factory: &Arc<ExecutionContextFactory>) -> Arc<EcaRules> {
    let shared = factory.extension::<Shared>(|factory| RwLock::new(Arc::new(load(factory))));
    let guard = shared.read().unwrap_or_else(|err| err.into_inner());
    Arc::clone(&guard)
}

/// Replace a factory's rules, for tests that bring their own XML.
#[cfg(test)]
pub(super) fn install(factory: &Arc<ExecutionContextFactory>, rules: EcaRules) {
    let shared = factory.extension::<Shared>(|_| RwLock::new(Arc::new(EcaRules::default())));
    *shared.write().unwrap_or_else(|err| err.into_inner()) = Arc::new(rules);
}

fn load(factory: &ExecutionContextFactory) -> EcaRules {
    let mut seca_by_id: HashMap<String, Arc<SecaRule>> = HashMap::new();
    let mut seca: Vec<Arc<SecaRule>> = Vec::new();
    let mut email = Vec::new();
    for component in factory.components() {
        let dir = component.location.join("service");
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<_> = read.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            let is_seca = name.ends_with(".secas.xml");
            if !is_seca && !name.ends_with(".emecas.xml") {
                continue;
            }
            let root = match MNode::parse_file(&path) {
                Ok(root) => root,
                Err(err) => {
                    tracing::error!(file = %path.display(), error = %err, "could not read ECA rules");
                    continue;
                }
            };
            if is_seca {
                for node in root.children_named("seca") {
                    let rule = Arc::new(seca_rule_from(&node));
                    match node.attribute("id").filter(|id| !id.is_empty()) {
                        Some(id) => {
                            seca_by_id.insert(id, rule);
                        }
                        None => seca.push(rule),
                    }
                }
            } else {
                for node in root.children_named("emeca") {
                    email.push(Arc::new(email_rule_from(&node)));
                }
            }
        }
    }
    seca.extend(seca_by_id.into_values());
    seca.sort_by_key(|rule| rule.priority);
    tracing::info!(seca = seca.len(), email = email.len(), "loaded service ECA rules");
    EcaRules { seca, email }
}

/// Parse rules from XML text, for tests.
#[cfg(test)]
pub(super) fn from_xml(seca_xml: &str, email_xml: &str) -> EcaRules {
    let mut rules = EcaRules::default();
    if !seca_xml.is_empty() {
        let root = MNode::parse_text("secas", seca_xml).expect("secas xml");
        rules.seca = root
            .children_named("seca")
            .iter()
            .map(|node| Arc::new(seca_rule_from(node)))
            .collect();
        rules.seca.sort_by_key(|rule| rule.priority);
    }
    if !email_xml.is_empty() {
        let root = MNode::parse_text("emecas", email_xml).expect("emecas xml");
        rules.email = root
            .children_named("emeca")
            .iter()
            .map(|node| Arc::new(email_rule_from(node)))
            .collect();
    }
    rules
}
