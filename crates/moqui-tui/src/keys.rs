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

//! Keys the spike handles. The full scheme in section 1.5 is Phase 2.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: Key,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl KeyPress {
    pub fn plain(key: Key) -> Self {
        Self {
            key,
            alt: false,
            ctrl: false,
            shift: false,
        }
    }
}
