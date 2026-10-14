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

//! Entity facade. Field encryption stays in [`crypto`] and is not called from this spine.

pub mod cache;
pub mod crypto;
pub mod document;
pub mod field_rw;
pub mod load_feed;
pub mod query_rest;

mod dbresource;
mod etl;
pub(crate) mod exec;
mod facade;
mod find;
mod fk;
mod meta;
mod params;
mod schema;
mod search_form;
mod sequence;
mod share;
mod value;

pub use dbresource::{DbResourceChild, DbResourceInfo, DbResourceStat};
pub use etl::EtlLoader;
pub use exec::statement_stats;
pub use facade::{EntityFacade, EntityFlags, EntityGroup};
pub use find::{EntityFind, EntityListIterator};
pub use meta::EntityDbMeta;
/// The condition tree `EntityFind::condition` takes, so callers need not depend on `moqui-db`.
pub use moqui_db::sql::{CompareOp, Condition};
pub(crate) use schema::document as schema_document;
pub use search_form::SearchFormResult;
pub(crate) use sequence::uuid_v4;
pub use sequence::{fill_bank, stagger_step, take_id, SequenceBank};
pub use value::{EntityList, EntityValue};

#[cfg(test)]
mod tests;
