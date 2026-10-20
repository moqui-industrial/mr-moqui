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
mod session;
#[cfg(feature = "ui")]
mod ui;

pub use decode::{Decoded, Decoder};
pub use keys::{Key, KeyPress};
pub use layout::{display_width, layout, outline, LabelMode, LaidOut, WidthClass, PAGE_ROWS};
pub use link::{
    gallery_link, FakeLink, ScreenLink, GALLERY_DETAIL_JSON, GALLERY_DETAIL_PATH, GALLERY_LIST_JSON, GALLERY_LIST_PATH,
};
pub use model::{Cell, Choice, Column, FormList, ModelError, Node, Paging, ScreenMeta, ScreenModel, Widget};
pub use session::Session;
#[cfg(feature = "ui")]
pub use ui::{render_text, run_key_session, run_local, run_session, AppView, DrawProbe};
