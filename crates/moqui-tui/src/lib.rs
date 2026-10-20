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

//! The terminal client of the `tui` render mode (plans/Screen-TUI-Impl.md section 3).
//!
//! It does no I/O of its own (D4): the session queues calls, and the host's [`ScreenLink`] answers them. The `model`
//! feature is everything but the Revue view: the model, the link, navigation, state, layout, keys and theme, which
//! `moqui-core`'s tests use without compiling Revue (D3). The `ui` feature adds the Revue view and the local loop.

mod decode;
mod edit;
mod forms;
mod keys;
mod layout;
mod link;
mod model;
mod nav;
mod session;
mod state;
mod theme;
#[cfg(feature = "ui")]
mod ui;

pub use decode::{Decoded, Decoder};
pub use edit::{row_col, Edited, Limits};
pub use forms::{FieldInfo, FocusItem, FocusKind, FormInfo, Index, LinkInfo, Scope, TabInfo};
pub use keys::{merge_keys, Action, Key, KeyMap, KeyPress, MergeSkip, ACTIONS};
pub use layout::{
    display_width, dotted_label, frame, frame_with, label_column_width, layout, layout_levels, layout_nodes, outline,
    outline_of, truncate_width, width_class, wrap, Breakpoints, Frame, LabelMode, LaidOut, Piece, WidthClass,
    DRAWER_DOCK_AT, DRAWER_WIDTH, PAGE_ROWS,
};
pub use link::{
    gallery_link, row_link, FakeLink, LinkCall, LinkError, MenuItem, Reply, ScreenLink, ShellInfo, TransitionOutcome,
    GALLERY_DETAIL_JSON, GALLERY_DETAIL_PATH, GALLERY_LIST_JSON, GALLERY_LIST_PATH,
};
pub use model::{
    clean_text, walk, Cell, Choice, Column, FormList, HtmlBlock, HtmlLink, ListRow, Messages, ModelError, Node, Paging,
    RowSelection, Rule, ScreenMeta, ScreenModel, ValidationError, VisibleWhen, Widget, MODEL_VERSION,
};
pub use nav::{decode as decode_query, level_path, query_parameters, Location, Nav, LEVEL_FLOOR};
pub use session::{drawer_rows, DropDown, Login, Message, Overlay, Picker, Session};
pub use state::{Confirm, ConfirmKind, Edits, Finished, Lanes, Purpose, Request, Then};
pub use theme::{Depth, Glyph, Role, Theme};
#[cfg(feature = "ui")]
pub use ui::{from_revue, render_text, run_local, snapshot, AppView, LocalOptions};
