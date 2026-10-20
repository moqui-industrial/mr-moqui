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

//! Terminal client for the `tui` screen model.
//!
//! The `model` feature is serde types, layout, and key decoding. Revue is behind `ui`.
//! This crate does no server I/O. Phase 1 is a spike: two hand-built Gallery fixtures
//! and a decision on how an SSH session can draw without owning the process terminal.

mod decode;
mod keys;
mod layout;
mod link;
mod model;
mod nav;
mod session;
mod state;
#[cfg(feature = "ui")]
mod ui;

pub use decode::{Decoded, Decoder};
pub use keys::{merge_keys, Action, Key, KeyMap, KeyPress, MergeSkip};
pub use layout::{
    display_width, dotted_label, frame, label_column_width, layout, outline, Frame, LabelMode, LaidOut, WidthClass,
    DRAWER_DOCK_AT, DRAWER_WIDTH, PAGE_ROWS,
};
pub use link::{
    gallery_link, FakeLink, LinkCall, LinkError, ScreenLink, TransitionOutcome, GALLERY_DETAIL_JSON,
    GALLERY_DETAIL_PATH, GALLERY_LIST_JSON, GALLERY_LIST_PATH,
};
pub use model::{
    Cell, Choice, Column, FormList, Messages, ModelError, Node, Paging, ScreenMeta, ScreenModel, ValidationError,
    Widget,
};
pub use nav::Nav;
pub use session::{MenuItem, Session};
pub use state::{Confirm, OptionLane, OptionRequest};
#[cfg(feature = "ui")]
pub use ui::{render_text, run_key_session, run_local, run_session, AppView, DrawProbe};
