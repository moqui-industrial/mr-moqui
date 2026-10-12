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
//! `EntityModel` contains `MNode`, which is `Rc` and not `Send`. The facade never
//! reads `entity_condition` after load.

use moqui_db::model::EntityModel;

/// Loaded entity model. Immutable after `EntityModel::load`.
#[allow(clippy::non_send_fields_in_send_ty)]
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

// SAFETY: after load the model is immutable. `entity_condition` holds `MNode`
// (`Rc<RefCell<_>>`). This facade never reads, clones, or drops those nodes on
// another thread. Callers read plain definition fields only. The surrounding
// `Arc` drops the model on one thread when the last facade goes away.
unsafe impl Send for SharedModel {}
unsafe impl Sync for SharedModel {}
