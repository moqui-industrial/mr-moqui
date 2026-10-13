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

//! In-process notifications. The host WebSocket and cluster fan-out subscribe later.

mod message;
mod topic;

pub(crate) use message::service_job_error;
pub use message::{mark_sent, mark_viewed, NotificationMessage};
pub use topic::{
    install, origin_id, publish, subscribe, Listener, Subscription, TopicMessage, ENTITY_CACHE_INVALIDATE,
    NOTIFICATION_MESSAGE,
};

#[cfg(test)]
mod tests;
