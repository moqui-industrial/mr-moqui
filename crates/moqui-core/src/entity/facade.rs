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

//! One entity facade. One [`TransactionManager`] per entity group.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use moqui_db::dialect::{Dialect, Dialects};
use moqui_db::model::{EntityDefinition, EntityModel};
use moqui_lib::error::Error;
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::crypto::CryptConfig;
use super::find::EntityFind;
use super::query_rest;
use super::sequence::SequenceBank;
use super::share::SharedModel;
use super::value::EntityValue;
use crate::transaction::TransactionManager;

/// Flags stored for later hooks. `disable_fk_create` and `dummy_fks` are enforced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityFlags {
    pub disable_entity_eca: bool,
    pub disable_audit_log: bool,
    pub disable_fk_create: bool,
    pub disable_data_feed: bool,
    pub dummy_fks: bool,
}

/// One entity group: a dialect name and the transaction manager the host registered.
pub struct EntityGroup {
    name: String,
    dialect_name: String,
    transactions: TransactionManager,
}

impl EntityGroup {
    pub fn new(name: impl Into<String>, dialect_name: impl Into<String>, transactions: TransactionManager) -> Self {
        Self {
            name: name.into(),
            dialect_name: dialect_name.into(),
            transactions,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dialect_name(&self) -> &str {
        &self.dialect_name
    }

    pub fn transactions(&self) -> &TransactionManager {
        &self.transactions
    }
}

pub(crate) struct GroupRuntime {
    pub(crate) dialect: Dialect,
    pub(crate) transactions: TransactionManager,
}

pub(crate) struct Inner {
    pub(crate) model: SharedModel,
    pub(crate) default_group: String,
    pub(crate) groups: HashMap<String, GroupRuntime>,
    pub(crate) banks: Mutex<HashMap<String, SequenceBank>>,
    pub(crate) flags: RwLock<EntityFlags>,
    pub(crate) db_ready: AtomicBool,
    locale: RwLock<String>,
    crypt: RwLock<Option<(CryptConfig, Vec<CryptConfig>)>>,
}

/// Entity facade shared by the factory. Clone shares the model, banks, and pools.
#[derive(Clone)]
pub struct EntityFacade {
    pub(crate) inner: Arc<Inner>,
}

impl EntityFacade {
    /// Load the framework entity model and attach one transaction manager per group.
    pub fn open(framework_dir: &Path, default_group: &str, groups: Vec<EntityGroup>) -> Result<Self, Error> {
        if groups.is_empty() {
            return Err(Error::new("entity facade needs a transaction manager"));
        }
        let conf_path = framework_dir.join("conf/MoquiDefaultConf.xml");
        let conf = MNode::parse_file(&conf_path)?;
        let model = EntityModel::load_directory(&conf, &framework_dir.join("entity"))?;
        let dialects = Dialects::load(&conf)?;
        let mut runtimes = HashMap::new();
        for group in groups {
            if runtimes.contains_key(&group.name) {
                return Err(Error::new(format!("entity group {} is duplicated", group.name)));
            }
            let dialect = dialects
                .get(&group.dialect_name)
                .cloned()
                .ok_or_else(|| Error::new(format!("no database named {}", group.dialect_name)))?;
            runtimes.insert(
                group.name,
                GroupRuntime {
                    dialect,
                    transactions: group.transactions,
                },
            );
        }
        Ok(Self {
            inner: Arc::new(Inner {
                model: SharedModel::new(model),
                default_group: default_group.to_string(),
                groups: runtimes,
                banks: Mutex::new(HashMap::new()),
                flags: RwLock::new(EntityFlags::default()),
                db_ready: AtomicBool::new(false),
                locale: RwLock::new("en".to_string()),
                crypt: RwLock::new(None),
            }),
        })
    }

    pub fn set_locale(&self, locale: impl Into<String>) {
        let mut slot = self.inner.locale.write().unwrap_or_else(|err| err.into_inner());
        *slot = locale.into();
    }

    pub fn locale(&self) -> String {
        self.inner.locale.read().unwrap_or_else(|err| err.into_inner()).clone()
    }

    /// Primary field cipher plus decrypt-only alternates. `None` until the caller sets them.
    pub fn set_crypt(&self, primary: CryptConfig, alts: Vec<CryptConfig>) {
        let mut slot = self.inner.crypt.write().unwrap_or_else(|err| err.into_inner());
        *slot = Some((primary, alts));
    }

    pub fn crypt(&self) -> Option<(CryptConfig, Vec<CryptConfig>)> {
        self.inner.crypt.read().unwrap_or_else(|err| err.into_inner()).clone()
    }

    pub fn flags(&self) -> EntityFlags {
        *self.inner.flags.read().unwrap_or_else(|err| err.into_inner())
    }

    pub fn set_disable_entity_eca(&self, value: bool) {
        self.edit_flags(|flags| flags.disable_entity_eca = value);
    }

    pub fn set_disable_audit_log(&self, value: bool) {
        self.edit_flags(|flags| flags.disable_audit_log = value);
    }

    pub fn set_disable_fk_create(&self, value: bool) {
        self.edit_flags(|flags| flags.disable_fk_create = value);
    }

    pub fn set_disable_data_feed(&self, value: bool) {
        self.edit_flags(|flags| flags.disable_data_feed = value);
    }

    pub fn set_dummy_fks(&self, value: bool) {
        self.edit_flags(|flags| flags.dummy_fks = value);
    }

    /// Transaction manager for the entity's group. The group attribute falls back to the default group.
    pub fn transactions_for(&self, entity_name: &str) -> Result<TransactionManager, Error> {
        self.with_group(entity_name, |_, _, _, tx| Ok(tx.clone()))
    }

    pub fn begin(&self, entity_name: &str, timeout: Option<Duration>) -> Result<bool, Error> {
        self.transactions_for(entity_name)?.begin(timeout)
    }

    pub fn commit(&self, entity_name: &str) -> Result<(), Error> {
        self.transactions_for(entity_name)?.commit()
    }

    pub fn rollback(&self, entity_name: &str) -> Result<(), Error> {
        self.transactions_for(entity_name)?.rollback()
    }

    pub fn make_value(&self, entity_name: &str) -> Result<EntityValue, Error> {
        EntityValue::make(self, entity_name)
    }

    pub fn find(&self, entity_name: &str) -> Result<EntityFind, Error> {
        let full_name = self.with_entity(entity_name, |entity, _, _| Ok(entity.full_name.clone()))?;
        Ok(EntityFind::new(self.clone(), full_name))
    }

    /// `EntityFacade.rest`. Still the not-implemented stub.
    pub fn rest(
        &self,
        operation: &str,
        entity_name: &str,
        parameters: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Value, Error> {
        query_rest::rest(self, operation, entity_name, parameters)
    }

    pub fn dynamic_view(
        &self,
        entity_name: &str,
        parameters: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Vec<std::collections::BTreeMap<String, Value>>, Error> {
        query_rest::dynamic_view(self, entity_name, parameters)
    }

    pub fn write_data(
        &self,
        entity_name: &str,
        records: &[std::collections::BTreeMap<String, Value>],
    ) -> Result<(), Error> {
        query_rest::write_data(self, entity_name, records)
    }

    pub(crate) fn with_entity<T>(
        &self,
        name: &str,
        body: impl FnOnce(&EntityDefinition, &Dialect, &EntityModel) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let model = self.inner.model.model();
        let entity = model.entity(name)?;
        let group_name = entity.group.as_deref().unwrap_or(self.inner.default_group.as_str());
        let runtime = self
            .inner
            .groups
            .get(group_name)
            .ok_or_else(|| Error::new(format!("no transaction manager for entity group {group_name}")))?;
        body(entity, &runtime.dialect, model)
    }

    pub(crate) fn with_group<T>(
        &self,
        name: &str,
        body: impl FnOnce(&EntityDefinition, &Dialect, &EntityModel, &TransactionManager) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let model = self.inner.model.model();
        let entity = model.entity(name)?;
        let group_name = entity.group.as_deref().unwrap_or(self.inner.default_group.as_str());
        let runtime = self
            .inner
            .groups
            .get(group_name)
            .ok_or_else(|| Error::new(format!("no transaction manager for entity group {group_name}")))?;
        body(entity, &runtime.dialect, model, &runtime.transactions)
    }

    fn edit_flags(&self, edit: impl FnOnce(&mut EntityFlags)) {
        let mut flags = self.inner.flags.write().unwrap_or_else(|err| err.into_inner());
        edit(&mut flags);
    }
}
