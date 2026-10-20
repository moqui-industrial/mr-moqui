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

//! Byte sequences from a pty into keys, paste, and a window report.
//!
//! crossterm 0.29 reads the process terminal from `EventReader` and does not export a
//! parser for an arbitrary byte stream, so this decoder is the one the spike keeps.
//! A lone ESC stays buffered until the next byte, because it may be the start of a sequence.
//! Resize on a real SSH session arrives as a window-change, not as bytes. The `8;rows;cols t`
//! report is accepted so a test can feed a size without a pty.

use crate::keys::{Key, KeyPress};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decoded {
    Key(KeyPress),
    /// Columns, then rows. The xterm report is `8 ; rows ; cols t`.
    Resize {
        columns: u16,
        rows: u16,
    },
    Paste(String),
}

#[derive(Debug)]
pub struct Decoder {
    pending: Vec<u8>,
    paste: Option<Vec<u8>>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            paste: None,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Decoded> {
        if let Some(paste) = &mut self.paste {
            paste.extend_from_slice(bytes);
        } else {
            self.pending.extend_from_slice(bytes);
        }
        let mut out = Vec::new();
        let paste_end: &[u8] = b"\x1b[201~";
        loop {
            if let Some(buf) = &self.paste {
                let Some(at) = find_sub(buf, paste_end) else {
                    break;
                };
                let text = String::from_utf8_lossy(&buf[..at]).into_owned();
                let rest = buf[at + paste_end.len()..].to_vec();
                self.paste = None;
                self.pending = rest;
                out.push(Decoded::Paste(text));
                continue;
            }
            match take_one(&self.pending) {
                Take::NeedMore => break,
                Take::Skip(len) | Take::PasteStart(len) => {
                    let start = matches!(take_one(&self.pending), Take::PasteStart(_));
                    self.pending.drain(..len);
                    if start {
                        self.paste = Some(std::mem::take(&mut self.pending));
                    }
                }
                Take::One(decoded, len) => {
                    self.pending.drain(..len);
                    out.push(decoded);
                }
            }
        }
        out
    }
}

enum Take {
    NeedMore,
    Skip(usize),
    One(Decoded, usize),
    PasteStart(usize),
}

fn take_one(buf: &[u8]) -> Take {
    if buf.is_empty() {
        return Take::NeedMore;
    }
    match buf[0] {
        b'\x1b' => take_esc(buf),
        b'\r' => Take::One(key(Key::Enter), 1 + usize::from(buf.get(1) == Some(&b'\n'))),
        b'\n' => Take::One(key(Key::Enter), 1),
        b'\t' => Take::One(key(Key::Tab), 1),
        0x7f | 0x08 => Take::One(key(Key::Backspace), 1),
        byte if byte.is_ascii_graphic() || byte == b' ' => Take::One(key(Key::Char(byte as char)), 1),
        byte if byte >= 0x80 => take_utf8(buf),
        _ => Take::Skip(1),
    }
}

fn take_esc(buf: &[u8]) -> Take {
    if buf.len() < 2 {
        return Take::NeedMore;
    }
    match buf[1] {
        b'[' => take_csi(buf),
        b'O' => take_ss3(buf),
        b'\x1b' => Take::One(key(Key::Escape), 1),
        byte if byte.is_ascii_graphic() => Take::One(press(Key::Char(byte as char), true, false, false), 2),
        _ => Take::One(key(Key::Escape), 1),
    }
}

fn take_ss3(buf: &[u8]) -> Take {
    if buf.len() < 3 {
        return Take::NeedMore;
    }
    let key = match buf[2] {
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        _ => return Take::Skip(3),
    };
    Take::One(Decoded::Key(KeyPress::plain(key)), 3)
}

fn take_csi(buf: &[u8]) -> Take {
    // Linux console F1–F5 is ESC [ [ A–E. '[' is in the CSI final range, so this
    // is matched before the generic final-byte scan, which would stop on it.
    if buf.len() >= 3 && buf[2] == b'[' {
        if buf.len() < 4 {
            return Take::NeedMore;
        }
        let key = match buf[3] {
            b'A' => Key::F(1),
            b'B' => Key::F(2),
            b'C' => Key::F(3),
            b'D' => Key::F(4),
            b'E' => Key::F(5),
            _ => return Take::Skip(4),
        };
        return Take::One(Decoded::Key(KeyPress::plain(key)), 4);
    }
    let Some(final_at) = buf.iter().skip(2).position(|byte| (0x40..=0x7e).contains(byte)) else {
        return if buf.len() > 64 { Take::Skip(1) } else { Take::NeedMore };
    };
    let final_at = final_at + 2;
    let final_byte = buf[final_at];
    let body = &buf[2..final_at];
    let len = final_at + 1;
    let body = std::str::from_utf8(body).unwrap_or("");
    if final_byte == b'~' {
        return take_tilde(body, len);
    }
    if final_byte == b'u' {
        return take_kitty(body, len);
    }
    if final_byte == b't' {
        let mut parts = body.split(';');
        if parts.next() == Some("8") {
            let rows = parts.next().and_then(|text| text.parse().ok()).unwrap_or(0);
            let columns = parts.next().and_then(|text| text.parse().ok()).unwrap_or(0);
            return Take::One(Decoded::Resize { columns, rows }, len);
        }
        return Take::Skip(len);
    }
    let (mods, _) = mods_and_first(body);
    let key = match final_byte {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'Z' => Key::BackTab,
        _ => return Take::Skip(len),
    };
    Take::One(Decoded::Key(apply_mods(key, mods)), len)
}

fn take_tilde(body: &str, len: usize) -> Take {
    let (mods, code) = mods_and_first(body);
    if code == "200" {
        return Take::PasteStart(len);
    }
    if code == "201" {
        return Take::Skip(len);
    }
    let key = match code {
        "3" => Key::Delete,
        "5" => Key::PageUp,
        "6" => Key::PageDown,
        "15" => Key::F(5),
        "17" => Key::F(6),
        "18" => Key::F(7),
        "19" => Key::F(8),
        "20" => Key::F(9),
        "21" => Key::F(10),
        "23" => Key::F(11),
        "24" => Key::F(12),
        _ => return Take::Skip(len),
    };
    Take::One(Decoded::Key(apply_mods(key, mods)), len)
}

fn take_kitty(body: &str, len: usize) -> Take {
    let mut parts = body.split(';');
    let code: u32 = parts.next().unwrap_or("").parse().unwrap_or(0);
    let mods = xterm_mods(parts.next().unwrap_or("1"));
    let key = match code {
        13 => Key::Enter,
        27 => Key::Escape,
        9 => Key::Tab,
        127 => Key::Backspace,
        32..=126 => Key::Char(char::from_u32(code).unwrap_or('\u{FFFD}')),
        _ => return Take::Skip(len),
    };
    Take::One(Decoded::Key(apply_mods(key, mods)), len)
}

fn mods_and_first(body: &str) -> (u8, &str) {
    let mut parts = body.split(';').filter(|part| !part.is_empty());
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    if let Some(mods) = second {
        (xterm_mods(mods), first)
    } else {
        (0, first)
    }
}

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn xterm_mods(text: &str) -> u8 {
    let value: u8 = text.parse().unwrap_or(1);
    value.saturating_sub(1)
}

fn apply_mods(key: Key, mods: u8) -> KeyPress {
    KeyPress {
        key,
        shift: mods & 1 != 0,
        alt: mods & 2 != 0,
        ctrl: mods & 4 != 0,
    }
}

fn take_utf8(buf: &[u8]) -> Take {
    let width = utf8_width(buf[0]);
    if buf.len() < width {
        return Take::NeedMore;
    }
    let text = std::str::from_utf8(&buf[..width]).unwrap_or("\u{FFFD}");
    let ch = text.chars().next().unwrap_or('\u{FFFD}');
    Take::One(key(Key::Char(ch)), width)
}

fn utf8_width(byte: u8) -> usize {
    if byte & 0b1110_0000 == 0b1100_0000 {
        2
    } else if byte & 0b1111_0000 == 0b1110_0000 {
        3
    } else if byte & 0b1111_1000 == 0b1111_0000 {
        4
    } else {
        1
    }
}

fn key(key: Key) -> Decoded {
    Decoded::Key(KeyPress::plain(key))
}

fn press(key: Key, alt: bool, ctrl: bool, shift: bool) -> Decoded {
    Decoded::Key(KeyPress { key, alt, ctrl, shift })
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Decoded> {
        Decoder::new().push(bytes)
    }

    #[test]
    fn xterm_and_gnome_function_keys() {
        assert_eq!(keys(b"\x1bOP"), vec![key(Key::F(1))]);
        assert_eq!(keys(b"\x1bOS"), vec![key(Key::F(4))]);
        assert_eq!(keys(b"\x1b[15~"), vec![key(Key::F(5))]);
        assert_eq!(keys(b"\x1b[18~"), vec![key(Key::F(7))]);
        assert_eq!(keys(b"\x1b[24~"), vec![key(Key::F(12))]);
        assert_eq!(keys(b"\x1b[A"), vec![key(Key::Up)]);
    }

    #[test]
    fn kitty_unicode_keys_and_alt() {
        assert_eq!(keys(b"\x1b[13u"), vec![key(Key::Enter)]);
        assert_eq!(keys(b"\x1b[97;3u"), vec![press(Key::Char('a'), true, false, false)]);
    }

    #[test]
    fn linux_console_function_key() {
        assert_eq!(keys(b"\x1b[[A"), vec![key(Key::F(1))]);
    }

    #[test]
    fn tmux_and_xterm_alt_is_an_escape_prefix() {
        assert_eq!(keys(b"\x1ba"), vec![press(Key::Char('a'), true, false, false)]);
        assert_eq!(keys(b"\x1b[1;3A"), vec![press(Key::Up, true, false, false)]);
    }

    #[test]
    fn paste_keeps_a_newline_as_one_event() {
        let decoded = keys(b"\x1b[200~hello\nworld\x1b[201~");
        assert_eq!(decoded, vec![Decoded::Paste("hello\nworld".to_string())]);
    }

    #[test]
    fn a_sequence_split_across_reads_still_parses() {
        let mut decoder = Decoder::new();
        assert!(decoder.push(b"\x1b[1").is_empty());
        assert_eq!(decoder.push(b"5~"), vec![key(Key::F(5))]);
    }

    #[test]
    fn resize_report_is_columns_then_rows() {
        assert_eq!(
            keys(b"\x1b[8;50;180t"),
            vec![Decoded::Resize { columns: 180, rows: 50 }]
        );
    }
}
