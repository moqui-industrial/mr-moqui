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

//! Edit buffers are owned by the session. This module holds the marks, the leave prompt,
//! and the one-in-flight options lane.

use std::collections::BTreeMap;

/// One options request. A newer `push` while this generation is in flight replaces the pending one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionRequest {
    pub generation: u64,
    pub field_id: String,
    pub url: String,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct OptionLane {
    next: u64,
    inflight: Option<u64>,
    pending: Option<OptionRequest>,
}

impl OptionLane {
    /// The request to send now, or nothing when one is already in flight.
    pub fn push(
        &mut self,
        field_id: String,
        url: String,
        parameters: BTreeMap<String, String>,
    ) -> Option<OptionRequest> {
        self.next += 1;
        let request = OptionRequest {
            generation: self.next,
            field_id,
            url,
            parameters,
        };
        if self.inflight.is_some() {
            self.pending = Some(request);
            None
        } else {
            self.inflight = Some(request.generation);
            Some(request)
        }
    }

    /// `apply` is true when `generation` is still the latest. The second value is the pending
    /// request that should be sent next, which means the finished result is dropped.
    pub fn finish(&mut self, generation: u64) -> (bool, Option<OptionRequest>) {
        if self.inflight != Some(generation) {
            return (false, None);
        }
        self.inflight = None;
        if let Some(next) = self.pending.take() {
            self.inflight = Some(next.generation);
            (false, Some(next))
        } else {
            (true, None)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    Leave { url: String },
}

/// Save, Discard, or Stay. The selected choice starts at Stay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub choices: Vec<&'static str>,
    pub at: usize,
    pub kind: ConfirmKind,
}

impl Confirm {
    pub fn leave(url: String) -> Self {
        Self {
            choices: vec!["Stay", "Discard", "Save"],
            at: 0,
            kind: ConfirmKind::Leave { url },
        }
    }

    pub fn choice(&self) -> &str {
        self.choices.get(self.at).copied().unwrap_or("Stay")
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.choices.len() as isize;
        if len == 0 {
            return;
        }
        self.at = (self.at as isize + delta).rem_euclid(len) as usize;
    }
}
