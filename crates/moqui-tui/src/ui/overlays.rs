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

//! What draws over the canvas: dialogs and the find dialog (90% wide, full screen at xs), the drop-down list under its
//! field, the date picker, the confirmation, the help, the info box and the login.

use revue::layout::Rect;

use crate::keys::ACTIONS;
use crate::layout::{display_width, layout_nodes, truncate_width, Breakpoints, WidthClass};
use crate::model::{walk, Node};
use crate::session::{Overlay, Session};
use crate::state::ConfirmKind;
use crate::theme::Role;

use super::draw::draw_piece;
use super::paint::{Look, Painter};

pub fn draw(paint: &mut Painter, session: &Session, breakpoints: &Breakpoints, canvas: Rect, class: WidthClass) {
    for overlay in session.overlays() {
        match overlay {
            Overlay::Dialog(id) => {
                let title = session.index().dialogs.get(id).cloned().unwrap_or_default();
                if let Some(nodes) = dialog_nodes(session, id) {
                    body_box(paint, session, breakpoints, canvas, class, &title, &nodes);
                }
            }
            Overlay::Find(list) => {
                if let Some(found) = session.list(list) {
                    body_box(
                        paint,
                        session,
                        breakpoints,
                        canvas,
                        class,
                        "Find",
                        &found.header_fields.clone(),
                    );
                }
            }
            Overlay::DropDown(_) => dropdown(paint, session, canvas),
            Overlay::Picker(picker) => date_picker(paint, picker, canvas, paint_spot(paint, &picker.key)),
            Overlay::Confirm(confirm) => {
                let text = match &confirm.kind {
                    ConfirmKind::Leave => "Leave without saving the changes?".to_string(),
                    ConfirmKind::Ask(text) => text.clone(),
                };
                let choices: Vec<String> = confirm
                    .choices()
                    .iter()
                    .map(|choice| {
                        if *choice == confirm.choice() {
                            format!("< {choice} >")
                        } else {
                            format!("[ {choice} ]")
                        }
                    })
                    .collect();
                small_box(paint, canvas, "Confirm", &[text, String::new(), choices.join("  ")]);
            }
            Overlay::Help => help(paint, session, canvas),
            Overlay::Info => {
                let link = session
                    .focus()
                    .and_then(|item| session.index().links.get(&item.key))
                    .map(|link| format!("Link: {}", link.url));
                let mut lines = vec![
                    format!("Screen: {}", session.path()),
                    format!("Address: {}", session.location_url()),
                    format!("Title: {}", session.model().screen.title),
                ];
                lines.extend(link);
                small_box(paint, canvas, "Info", &lines);
            }
            Overlay::Login(login) => {
                let mark = |at: usize| if login.at == at { ">" } else { " " };
                let dots = "•".repeat(login.password.chars().count());
                let mut lines = vec![
                    format!("{} Username . : [{:<20}]", mark(0), login.username),
                    format!("{} Password . : [{:<20}]", mark(1), dots),
                    format!("{} Code . . . : [{:<20}]", mark(2), login.code),
                    String::new(),
                    "Enter=Log in  Tab=Next  Esc=Cancel".to_string(),
                ];
                if !login.error.is_empty() {
                    lines.insert(0, format!("! {}", login.error));
                }
                small_box(paint, canvas, "Log in", &lines);
            }
            Overlay::Drawer(_) => {}
        }
    }
}

fn paint_spot(paint: &Painter, key: &str) -> Option<(u16, u16)> {
    paint.spots.get(key).copied()
}

fn dialog_nodes(session: &Session, id: &str) -> Option<Vec<Node>> {
    let mut found = None;
    for model in session.nav().models() {
        walk(&model.c, &mut |node| {
            if let Node::Dialog { id: dialog, c, .. } = node {
                if dialog == id {
                    found = Some(c.clone());
                }
            }
        });
    }
    found
}

/// A dialog's body, laid out at its own width and drawn in a box over the canvas.
fn body_box(
    paint: &mut Painter,
    session: &Session,
    breakpoints: &Breakpoints,
    canvas: Rect,
    class: WidthClass,
    title: &str,
    nodes: &[Node],
) {
    let width = if class == WidthClass::Xs {
        canvas.width
    } else {
        canvas.width * 9 / 10
    };
    let laid = layout_nodes(nodes, width.saturating_sub(4), breakpoints);
    let height = u16::try_from(laid.height() + 2).unwrap_or(u16::MAX).min(canvas.height);
    let x = canvas.x + (canvas.width - width) / 2;
    let y = if class == WidthClass::Xs {
        canvas.y
    } else {
        canvas.y + (canvas.height - height) / 2
    };
    let rect = Rect::new(x, y, width, height);
    paint.clear(rect);
    paint.fixed_frame(rect, &format!(" {title} "));
    let saved = (paint.top, paint.bottom, paint.scroll, paint.left, paint.right);
    paint.top = y + 1;
    paint.bottom = y + height.saturating_sub(1);
    paint.scroll = 0;
    paint.left = x + 1;
    paint.right = x + width.saturating_sub(1);
    let mut at = 0;
    for piece in &laid.pieces {
        draw_piece(paint, session, piece, x + 2, at);
        at += piece.height();
    }
    (paint.top, paint.bottom, paint.scroll, paint.left, paint.right) = saved;
}

/// A small centered box of text lines.
fn small_box(paint: &mut Painter, canvas: Rect, title: &str, lines: &[String]) {
    let widest = lines
        .iter()
        .map(|line| display_width(line))
        .max()
        .unwrap_or(0)
        .max(display_width(title) + 4);
    let width = u16::try_from(widest + 4).unwrap_or(u16::MAX).min(canvas.width);
    let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX).min(canvas.height);
    let rect = Rect::new(
        canvas.x + (canvas.width - width) / 2,
        canvas.y + (canvas.height - height) / 2,
        width,
        height,
    );
    paint.clear(rect);
    paint.fixed_frame(rect, &format!(" {title} "));
    for (at, line) in lines.iter().enumerate() {
        let row = rect.y + 1 + u16::try_from(at).unwrap_or(0);
        if row + 1 >= rect.y + rect.height {
            break;
        }
        let look = if line.starts_with('!') {
            Look::role(Role::Negative).bold()
        } else {
            Look::role(Role::Text)
        };
        paint.fixed(rect.x + 2, row, line, look, usize::from(width.saturating_sub(4)));
    }
}

/// The open drop-down: its entries under the field, the typed filter at the top, the highlighted one reversed.
fn dropdown(paint: &mut Painter, session: &Session, canvas: Rect) {
    let Some(drop) = session.dropdown() else {
        return;
    };
    let choices = session.dropdown_choices();
    let current = session.buffer(&drop.field).to_string();
    let widest = choices
        .iter()
        .map(|choice| display_width(&choice.label))
        .max()
        .unwrap_or(10)
        .clamp(12, 50);
    let width = u16::try_from(widest + 4).unwrap_or(40);
    let visible = choices.len().clamp(1, 10);
    let height = u16::try_from(visible + 2 + usize::from(!drop.filter.is_empty())).unwrap_or(12);
    let (sx, sy) = paint_spot(paint, &drop.field).unwrap_or((canvas.x + 2, canvas.y));
    let below = sy + 1;
    let y = if below + height <= canvas.y + canvas.height {
        below
    } else {
        (sy).saturating_sub(height).max(canvas.y)
    };
    let x = sx.min((canvas.x + canvas.width).saturating_sub(width));
    let rect = Rect::new(x, y, width, height);
    paint.clear(rect);
    paint.fixed_frame(rect, "");
    let mut row = y + 1;
    if !drop.filter.is_empty() {
        paint.fixed(
            x + 1,
            row,
            &format!("/{}", drop.filter),
            Look::role(Role::Info),
            usize::from(width - 2),
        );
        row += 1;
    }
    let start = drop.at.saturating_sub(visible - 1);
    for (at, choice) in choices.iter().enumerate().skip(start).take(visible) {
        let multiple = matches!(
            session.field_widget(&drop.field),
            Some(crate::model::Widget::DropDown { multiple: true, .. })
        );
        let on = if multiple {
            current.split(',').any(|part| part == choice.value)
        } else {
            current == choice.value
        };
        let mark = if multiple {
            if on {
                "[x] "
            } else {
                "[ ] "
            }
        } else if on {
            "• "
        } else {
            "  "
        };
        let label = if choice.label.is_empty() {
            "(none)"
        } else {
            &choice.label
        };
        let text = format!("{mark}{}", truncate_width(label, widest));
        let look = Look::role(Role::Text).focused(at == drop.at);
        paint.fill_row(x + 1, row, usize::from(width - 2), look);
        paint.fixed(x + 1, row, &text, look, usize::from(width - 2));
        row += 1;
    }
    if choices.is_empty() {
        paint.fixed(
            x + 1,
            row,
            "no matches",
            Look::role(Role::Muted).italic(),
            usize::from(width - 2),
        );
    }
}

/// A month grid at the field: arrows move a day or a week, PgUp and PgDn a month, Home today, Enter picks.
fn date_picker(paint: &mut Painter, picker: &crate::session::Picker, canvas: Rect, spot: Option<(u16, u16)>) {
    let width = 24;
    let height = 10;
    let (sx, sy) = spot.unwrap_or((canvas.x + 2, canvas.y));
    let y = if sy + 1 + height <= canvas.y + canvas.height {
        sy + 1
    } else {
        sy.saturating_sub(height).max(canvas.y)
    };
    let x = sx.min((canvas.x + canvas.width).saturating_sub(width));
    let rect = Rect::new(x, y, width, height);
    paint.clear(rect);
    let title = picker.date.strftime("%B %Y").to_string();
    paint.fixed_frame(rect, &title);
    paint.fixed(x + 1, y + 1, "Mo Tu We Th Fr Sa Su", Look::role(Role::Muted), 22);
    let first = picker.date.first_of_month();
    let lead = usize::try_from(first.weekday().to_monday_zero_offset()).unwrap_or(0);
    let days = usize::try_from(picker.date.days_in_month()).unwrap_or(30);
    let today = jiff::Zoned::now().date();
    for day in 1..=days {
        let cell = lead + day - 1;
        let row = y + 2 + u16::try_from(cell / 7).unwrap_or(0);
        let col = x + 1 + u16::try_from((cell % 7) * 3).unwrap_or(0);
        let date = first
            .checked_add(jiff::Span::new().days(i64::try_from(day - 1).unwrap_or(0)))
            .unwrap_or(first);
        let look = if date == picker.date {
            Look::role(Role::Primary).bold().reverse()
        } else if date == today {
            Look::role(Role::Primary).underline()
        } else {
            Look::role(Role::Text)
        };
        paint.fixed(col, row, &format!("{day:>2}"), look, 2);
    }
    paint.fixed(
        x + 1,
        y + height - 2,
        "Enter=Pick  Home=Today",
        Look::role(Role::Muted),
        22,
    );
}

/// F1: every action with its keys, from the merged keymap, so a theme's keys show.
fn help(paint: &mut Painter, session: &Session, canvas: Rect) {
    let keys = session.keys();
    let mut lines = Vec::new();
    for action in ACTIONS {
        let chords: Vec<String> = keys.chords(action).iter().map(|press| press.label()).collect();
        if chords.is_empty() {
            continue;
        }
        lines.push(format!("{:<16} {}", action.title(), chords.join(", ")));
    }
    lines.push(format!("{:<16} Alt+1…Alt+9", "Tab 1 to 9"));
    lines.push(String::new());
    lines.push("Shift+Left/Right scrolls a wide list. Any key closes.".to_string());
    let room = usize::from(canvas.height.saturating_sub(2));
    if lines.len() > room {
        lines.truncate(room);
    }
    small_box(paint, canvas, "Keys", &lines);
}
