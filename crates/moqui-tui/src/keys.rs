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

//! Key names and the default keymap. Theme overrides merge on top. Esc, Enter, and Tab stay fixed.

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

    pub fn with_alt(key: Key) -> Self {
        Self {
            alt: true,
            ..Self::plain(key)
        }
    }
}

/// Actions the default map binds. Palette, quick create, numbering, and on-screen find are absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Next,
    Prev,
    Activate,
    Back,
    Cancel,
    Prompt,
    Reload,
    PagePrev,
    PageNext,
    Sort,
    Find,
    Quit,
}

impl Action {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "next" => Self::Next,
            "prev" => Self::Prev,
            "activate" => Self::Activate,
            "back" => Self::Back,
            "cancel" => Self::Cancel,
            "prompt" => Self::Prompt,
            "reload" => Self::Reload,
            "page-prev" => Self::PagePrev,
            "page-next" => Self::PageNext,
            "sort" => Self::Sort,
            "find" => Self::Find,
            "quit" => Self::Quit,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMap {
    bindings: Vec<(KeyPress, Action)>,
}

impl KeyMap {
    pub fn baseline() -> Self {
        Self {
            bindings: vec![
                (KeyPress::plain(Key::Tab), Action::Next),
                (KeyPress::plain(Key::Down), Action::Next),
                (KeyPress::plain(Key::BackTab), Action::Prev),
                (KeyPress::plain(Key::Up), Action::Prev),
                (KeyPress::plain(Key::Enter), Action::Activate),
                (KeyPress::plain(Key::Right), Action::Activate),
                (KeyPress::plain(Key::Left), Action::Back),
                (KeyPress::plain(Key::F(3)), Action::Back),
                (KeyPress::plain(Key::Escape), Action::Cancel),
                (KeyPress::plain(Key::F(12)), Action::Cancel),
                (KeyPress::plain(Key::F(4)), Action::Prompt),
                (KeyPress::plain(Key::F(5)), Action::Reload),
                (KeyPress::plain(Key::F(7)), Action::PagePrev),
                (KeyPress::plain(Key::PageUp), Action::PagePrev),
                (KeyPress::plain(Key::F(8)), Action::PageNext),
                (KeyPress::plain(Key::PageDown), Action::PageNext),
                (KeyPress::plain(Key::Char('s')), Action::Sort),
                (KeyPress::plain(Key::Char('f')), Action::Find),
                (KeyPress::plain(Key::Char('q')), Action::Quit),
            ],
        }
    }

    pub fn action(&self, press: &KeyPress) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(bound, _)| bound == press)
            .map(|(_, action)| *action)
    }

    fn retain_action(&mut self, action: Action) {
        self.bindings.retain(|(_, bound)| *bound != action);
    }

    fn bind(&mut self, press: KeyPress, action: Action) {
        self.bindings.retain(|(bound, _)| bound != &press);
        self.bindings.push((press, action));
    }
}

/// Why one theme entry was not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeSkip {
    pub action: String,
    pub chord: String,
    pub reason: &'static str,
}

/// Theme rows replace the default chords of a known action. A bad chord or an unknown action is
/// skipped. A chord of Esc, Enter, or Tab is refused, and that action keeps its default.
pub fn merge_keys(base: &KeyMap, overrides: &[(&str, &str)]) -> (KeyMap, Vec<MergeSkip>) {
    let mut map = base.clone();
    let mut skipped = Vec::new();
    for (action_name, chords) in overrides {
        let Some(action) = Action::parse(action_name) else {
            skipped.push(MergeSkip {
                action: (*action_name).to_string(),
                chord: (*chords).to_string(),
                reason: "unknown action",
            });
            continue;
        };
        let mut parsed = Vec::new();
        let mut bad = false;
        for chord in chords.split('|') {
            let chord = chord.trim();
            if chord.is_empty() {
                continue;
            }
            match parse_chord(chord) {
                Some(press) if fixed_key(&press) => {
                    skipped.push(MergeSkip {
                        action: (*action_name).to_string(),
                        chord: chord.to_string(),
                        reason: "fixed key",
                    });
                    bad = true;
                }
                Some(press) => parsed.push(press),
                None => {
                    skipped.push(MergeSkip {
                        action: (*action_name).to_string(),
                        chord: chord.to_string(),
                        reason: "bad chord",
                    });
                    bad = true;
                }
            }
        }
        if bad {
            continue;
        }
        map.retain_action(action);
        for press in parsed {
            map.bind(press, action);
        }
    }
    (map, skipped)
}

fn fixed_key(press: &KeyPress) -> bool {
    matches!(press.key, Key::Escape | Key::Enter | Key::Tab | Key::BackTab)
}

fn parse_chord(chord: &str) -> Option<KeyPress> {
    let mut alt = false;
    let mut ctrl = false;
    let mut shift = false;
    let mut key = None;
    for part in chord.split('+') {
        let part = part.trim();
        if part.is_empty() {
            return None;
        }
        match part.to_ascii_lowercase().as_str() {
            "alt" => alt = true,
            "ctrl" => ctrl = true,
            "shift" => shift = true,
            other => {
                if key.is_some() {
                    return None;
                }
                key = Some(parse_key(other)?);
            }
        }
    }
    Some(KeyPress {
        key: key?,
        alt,
        ctrl,
        shift,
    })
}

fn parse_key(name: &str) -> Option<Key> {
    if name.len() == 1 {
        return name.chars().next().map(Key::Char);
    }
    Some(match name {
        "esc" | "escape" => Key::Escape,
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backtab" => Key::BackTab,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pgup" | "pageup" => Key::PageUp,
        "pgdn" | "pagedown" => Key::PageDown,
        other => {
            let number = other.strip_prefix('f')?.parse::<u8>().ok()?;
            if (1..=12).contains(&number) {
                Key::F(number)
            } else {
                return None;
            }
        }
    })
}
