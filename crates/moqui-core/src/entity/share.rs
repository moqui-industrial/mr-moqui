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

//! Share a loaded [`EntityModel`] across threads.
//!
//! The model holds no `MNode`, so it is `Send` and `Sync` on its own; the assertion below keeps
//! it that way.

use moqui_db::model::EntityModel;

/// Loaded entity model. Immutable after `EntityModel::load`.
pub(crate) struct SharedModel {
    model: EntityModel,
}

impl SharedModel {
    pub(crate) fn new(model: EntityModel) -> Self {
        Self { model }
    }

    pub(crate) fn model(&self) -> &EntityModel {
        &self.model
    }
}

const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<SharedModel>();
};
