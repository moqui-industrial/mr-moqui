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

//! Width classes and box placement from section 1.3, for the canvas only.
//!
//! No drawer, so the canvas width is the terminal width. The spike page size is
//! [`PAGE_ROWS`] (the bottom of the 10..=100 range) so the Gallery list stays two pages
//! on a tall screen. Sizing the page to the visible rows is Phase 2.

use crate::model::{walk, Node, ScreenModel};

/// Rows per list page in the spike.
pub const PAGE_ROWS: usize = 10;

const COL_CAP: usize = 40;
const BOX_MIN: u16 = 40;
const LABEL_CAP: usize = 24;
const LABEL_CAP_SM: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthClass {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl WidthClass {
    pub fn name(self) -> &'static str {
        match self {
            Self::Xs => "xs",
            Self::Sm => "sm",
            Self::Md => "md",
            Self::Lg => "lg",
            Self::Xl => "xl",
        }
    }

    fn stacks_boxes(self) -> bool {
        matches!(self, Self::Xs | Self::Sm)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelMode {
    Above,
    Beside,
}

impl LabelMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Above => "above",
            Self::Beside => "beside",
        }
    }
}

pub fn width_class(canvas: u16) -> WidthClass {
    match canvas {
        0..=79 => WidthClass::Xs,
        80..=119 => WidthClass::Sm,
        120..=179 => WidthClass::Md,
        180..=239 => WidthClass::Lg,
        _ => WidthClass::Xl,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidOut {
    pub class: WidthClass,
    pub canvas: u16,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Consecutive boxes, side by side when the class and the 40-column rule allow it.
    Boxes(Vec<LaidBox>),
    Form(LaidForm),
    List(LaidList),
    Link(LaidLink),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidBox {
    pub id: String,
    pub title: String,
    pub x: u16,
    pub w: u16,
    pub children: Vec<BoxChild>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoxChild {
    Field(LaidField),
    Link(LaidLink),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidField {
    pub id: String,
    pub title: String,
    pub mode: LabelMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidForm {
    pub id: String,
    pub name: String,
    pub fields: Vec<LaidField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidList {
    pub id: String,
    pub name: String,
    pub columns: Vec<LaidCol>,
    pub scroll: bool,
    pub page_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidCol {
    pub title: String,
    pub width: usize,
    pub fixed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidLink {
    pub id: String,
    pub text: String,
}

pub fn layout(model: &ScreenModel, canvas: u16) -> LaidOut {
    let class = width_class(canvas);
    LaidOut {
        class,
        canvas,
        blocks: place_nodes(&model.c, class, canvas),
    }
}

pub fn outline(model: &ScreenModel, canvas: u16) -> String {
    let laid = layout(model, canvas);
    let mut out = format!("class={} canvas={canvas}\n", laid.class.name());
    for block in &laid.blocks {
        match block {
            Block::Boxes(boxes) => {
                if boxes.len() > 1 && boxes.iter().any(|box_| box_.x > 0) {
                    out.push_str("row\n");
                }
                for box_ in boxes {
                    out.push_str(&format!("box {} x={} w={}\n", box_.title, box_.x, box_.w));
                    for child in &box_.children {
                        push_child(&mut out, child, "  ");
                    }
                }
            }
            Block::Form(form) => {
                out.push_str(&format!("form {}\n", form.name));
                for field in &form.fields {
                    out.push_str(&format!("  field {} {}\n", field.title, field.mode.name()));
                }
            }
            Block::List(list) => {
                out.push_str(&format!(
                    "list {} scroll={}\n",
                    list.name,
                    if list.scroll { "true" } else { "false" }
                ));
                for col in &list.columns {
                    let fixed = if col.fixed { " fixed" } else { "" };
                    out.push_str(&format!("  col {} w={}{fixed}\n", col.title, col.width));
                }
                out.push_str(&format!("  page_rows={}\n", list.page_rows));
            }
            Block::Link(link) => out.push_str(&format!("link {}\n", link.text)),
            Block::Text(text) => out.push_str(&format!("text {text}\n")),
        }
    }
    out
}

fn push_child(out: &mut String, child: &BoxChild, indent: &str) {
    match child {
        BoxChild::Field(field) => {
            out.push_str(&format!("{indent}field {} {}\n", field.title, field.mode.name()));
        }
        BoxChild::Link(link) => out.push_str(&format!("{indent}link {}\n", link.text)),
        BoxChild::Text(text) => out.push_str(&format!("{indent}text {text}\n")),
    }
}

fn place_nodes(nodes: &[Node], class: WidthClass, canvas: u16) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if matches!(nodes[index], Node::Box { .. }) {
            let start = index;
            while index < nodes.len() && matches!(nodes[index], Node::Box { .. }) {
                index += 1;
            }
            blocks.push(Block::Boxes(place_boxes(&nodes[start..index], class, canvas)));
        } else {
            if let Some(block) = place_one(&nodes[index], class, canvas) {
                blocks.push(block);
            }
            index += 1;
        }
    }
    blocks
}

fn place_one(node: &Node, class: WidthClass, canvas: u16) -> Option<Block> {
    let mode = label_mode(class);
    match node {
        Node::Form { id, name, c, .. } => Some(Block::Form(LaidForm {
            id: id.clone(),
            name: name.clone(),
            fields: fields_in(c, mode, canvas),
        })),
        Node::FormList { list } => Some(Block::List(lay_list(list, canvas))),
        Node::Link { id, text, .. } => Some(Block::Link(LaidLink {
            id: id.clone(),
            text: text.clone(),
        })),
        Node::Text { text, .. } => Some(Block::Text(text.clone())),
        Node::Field { .. } | Node::Box { .. } => None,
    }
}

fn place_boxes(nodes: &[Node], class: WidthClass, canvas: u16) -> Vec<LaidBox> {
    let slots = box_slots(nodes.len(), class, canvas);
    nodes
        .iter()
        .zip(slots)
        .filter_map(|(node, (x, w))| {
            let Node::Box { id, title, c } = node else {
                return None;
            };
            Some(LaidBox {
                id: id.clone(),
                title: title.clone(),
                x,
                w,
                children: box_children(c, class, canvas),
            })
        })
        .collect()
}

fn box_slots(count: usize, class: WidthClass, canvas: u16) -> Vec<(u16, u16)> {
    if count == 0 {
        return Vec::new();
    }
    let count_u = u16::try_from(count).unwrap_or(u16::MAX);
    let side_by_side = !class.stacks_boxes() && count > 1;
    if side_by_side {
        let gutters = count_u.saturating_sub(1);
        let inner = canvas.saturating_sub(gutters);
        let share = inner / count_u;
        if share >= BOX_MIN {
            let mut slots = Vec::with_capacity(count);
            let mut x: u16 = 0;
            let used = share * count_u;
            let remainder = inner - used;
            for index in 0..count_u {
                let w = if index + 1 == count_u { share + remainder } else { share };
                slots.push((x, w));
                x = x.saturating_add(w).saturating_add(1);
            }
            return slots;
        }
    }
    vec![(0, canvas); count]
}

fn box_children(nodes: &[Node], class: WidthClass, canvas: u16) -> Vec<BoxChild> {
    let mode = label_mode(class);
    let mut children = Vec::new();
    for node in nodes {
        match node {
            Node::Field { id, title, .. } => children.push(BoxChild::Field(fit_field(id, title, mode, canvas))),
            Node::Link { id, text, .. } => children.push(BoxChild::Link(LaidLink {
                id: id.clone(),
                text: text.clone(),
            })),
            Node::Text { text, .. } => children.push(BoxChild::Text(text.clone())),
            Node::Form { .. } | Node::FormList { .. } | Node::Box { .. } => {}
        }
    }
    children
}

fn fields_in(nodes: &[Node], mode: LabelMode, canvas: u16) -> Vec<LaidField> {
    let mut fields = Vec::new();
    walk(nodes, &mut |node| {
        if let Node::Field { id, title, .. } = node {
            fields.push(fit_field(id, title, mode, canvas));
        }
    });
    fields
}

fn fit_field(id: &str, title: &str, mode: LabelMode, canvas: u16) -> LaidField {
    let cap = match mode {
        LabelMode::Above => usize::from(canvas),
        LabelMode::Beside => {
            if width_class(canvas) == WidthClass::Sm {
                LABEL_CAP_SM
            } else {
                LABEL_CAP
            }
        }
    };
    LaidField {
        id: id.to_string(),
        title: truncate_width(title, cap),
        mode,
    }
}

fn label_mode(class: WidthClass) -> LabelMode {
    if class == WidthClass::Xs {
        LabelMode::Above
    } else {
        LabelMode::Beside
    }
}

fn lay_list(list: &crate::model::FormList, canvas: u16) -> LaidList {
    let mut columns = Vec::new();
    let mut total = 0usize;
    for (index, column) in list.columns.iter().enumerate() {
        let mut width = display_width(&column.title).min(COL_CAP);
        for row in &list.rows {
            if let Some(cell) = row.cells.get(index) {
                width = width.max(display_width(cell.text()).min(COL_CAP));
            }
        }
        if index > 0 {
            total += 1;
        }
        total += width;
        columns.push(LaidCol {
            title: column.title.clone(),
            width,
            fixed: index == 0,
        });
    }
    LaidList {
        id: list.id.clone(),
        name: list.name.clone(),
        columns,
        scroll: total > usize::from(canvas),
        page_rows: PAGE_ROWS,
    }
}

pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    let cp = c as u32;
    if (0x0300..=0x036F).contains(&cp) || cp == 0x200D || (0xFE00..=0xFE0F).contains(&cp) {
        return 0;
    }
    if is_wide(cp) {
        return 2;
    }
    1
}

fn is_wide(cp: u32) -> bool {
    (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE10..=0xFE19).contains(&cp)
        || (0xFE30..=0xFE6F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x1F300..=0x1F64F).contains(&cp)
        || (0x1F680..=0x1F6FF).contains(&cp)
        || (0x1F900..=0x1F9FF).contains(&cp)
        || (0x1FA00..=0x1FAFF).contains(&cp)
        || (0x20000..=0x3FFFD).contains(&cp)
}

pub fn truncate_width(text: &str, max: usize) -> String {
    if max == 0 || display_width(text) <= max {
        return if max == 0 { String::new() } else { text.to_string() };
    }
    let keep = max - 1;
    let mut width = 0;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let next = char_width(ch);
        if width + next > keep {
            break;
        }
        width += next;
        end = index + ch.len_utf8();
    }
    let mut out = text[..end].to_string();
    out.push('…');
    out
}

/// Column widths the painter uses, looked up from a laid-out list.
#[cfg_attr(not(feature = "ui"), allow(dead_code))]
pub fn list_column_widths(model: &ScreenModel, canvas: u16, list_id: &str) -> Vec<usize> {
    layout(model, canvas)
        .blocks
        .into_iter()
        .find_map(|block| match block {
            Block::List(list) if list.id == list_id => Some(list.columns.into_iter().map(|col| col.width).collect()),
            _ => None,
        })
        .unwrap_or_default()
}
