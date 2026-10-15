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

//! Entity ECA match, the entity data loader, and the `DataFeedPending` insert.
//!
//! Matching ECA rules run their conditions and actions (`service::run_matched_eca`), and data-file
//! service elements are called through `service::run_data_file_service` and also recorded here.
//! Feed receive and delete service names are recorded, and `service::schedule_feed_receive` hands
//! the pending rows to a service worker after commit. Documents are indexed only when search is attached.

mod csv;
mod eca;
mod feed;
mod json;
mod loader;
mod pull;
mod xml;
mod zip;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use moqui_lib::error::Error;
use moqui_lib::node::MNode;

use super::facade::EntityFacade;

#[cfg(test)]
pub(crate) use eca::add_eca_rules;
pub use eca::{eca_rule_count, eca_rules, match_rules, EcaRule};
pub(crate) use eca::{match_rules_with, EcaIndex};
pub(super) use feed::clear_frame_touches;
pub(crate) use feed::note_touch;
pub use feed::register_feed_insert;
pub use loader::{is_restricted_entity, load_entity_data, load_entity_xml_text};

/// A data-file service element. It is called as it is read, and kept here for [`data_file_services`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataFileService {
    pub name: String,
    pub location: String,
    pub parameters: BTreeMap<String, String>,
}

/// A real-time feed's receive and delete service names, recorded as feed rows are inserted. A service worker calls them after commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedServiceNames {
    pub data_feed_id: String,
    pub receive_service: String,
    pub delete_service: String,
}

/// Filters and write mode for [`load_entity_data`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadOptions {
    pub data_types: Vec<String>,
    pub components: Vec<String>,
    pub only_create: bool,
    pub use_try_insert: bool,
    pub dummy_fks: bool,
    pub disable_fk_create: bool,
    /// Transaction timeout for each record. `None` uses the transaction manager default.
    pub timeout: Option<std::time::Duration>,
    /// Java `restrictSensitiveEntities`: refuse every record, root or nested, whose entity is a
    /// [`RESTRICTED_ENTITIES`] entity. Here service elements are refused too, a hardening beyond the Java, which ran
    /// them and checked only root and CSV entity names.
    pub restrict_sensitive_entities: bool,
}

/// The identity-admin and secret-config entities (`WebUtilities.isRestrictedGenericEntity`), by simple name in lower
/// case.
pub const RESTRICTED_ENTITIES: &[&str] = &[
    "userloginkey",
    "usergroupmember",
    "usergrouppermission",
    "userpermission",
    "artifactauthz",
    "artifactgroup",
    "artifactgroupmember",
    "artifacttarpit",
    "usergrouppermissions",
    "userpermissions",
    "artifactgroups",
    "userauthcfactor",
    "userpasswordhistory",
    "emailserver",
    "systemmessageremote",
];

/// How many entity records were written. Data-file service calls are not included.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadResult {
    pub records: u64,
}

#[derive(Default)]
struct LoadSlot {
    services: Vec<DataFileService>,
    /// By `data_feed_id`, so each commit replaces the names of the feeds it saw.
    feed_services: BTreeMap<String, FeedServiceNames>,
}

fn with_load_slot<R>(facade: &EntityFacade, body: impl FnOnce(&mut LoadSlot) -> R) -> R {
    facade.with_slot(LoadSlot::default, body)
}

/// Service elements the loader has read, in order. They were called as they were read.
pub fn data_file_services(facade: &EntityFacade) -> Vec<DataFileService> {
    with_load_slot(facade, |slot| slot.services.clone())
}

/// Receive and delete service names seen while inserting feed rows. A service worker calls them after commit, not the loader.
pub fn feed_service_names(facade: &EntityFacade) -> Vec<FeedServiceNames> {
    with_load_slot(facade, |slot| slot.feed_services.values().cloned().collect())
}

/// `load-data` locations from the framework conf. This does not load those files.
pub fn conf_data_locations(facade: &EntityFacade) -> Result<Vec<String>, Error> {
    let conf = MNode::parse_file(facade.framework_dir().join("conf/MoquiDefaultConf.xml"))?;
    let facade = conf
        .first_named("entity-facade")
        .ok_or_else(|| Error::new("conf has no entity-facade"))?;
    let mut locations = Vec::new();
    for node in facade.children_named("load-data") {
        if let Some(location) = node.attribute("location") {
            locations.push(location);
        }
    }
    Ok(locations)
}
