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
//! [`frame`] docks a 26-column drawer when the terminal is at least 150 columns, and the
//! width class is the leftover canvas. [`PAGE_ROWS`] is only the fallback when a list
//! document has no page size. The server sends `pageIndex`; the client does not resize it.

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

/// Terminal width at which the drawer docks. The leftover canvas is then at least 120.
pub const DRAWER_DOCK_AT: u16 = 150;

/// Columns the docked drawer takes.
pub const DRAWER_WIDTH: u16 = 26;

/// Drawer and the canvas the width class is computed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub terminal: u16,
    pub drawer: u16,
    pub canvas: u16,
    pub class: WidthClass,
}

pub fn frame(terminal: u16) -> Frame {
    let drawer = if terminal >= DRAWER_DOCK_AT { DRAWER_WIDTH } else { 0 };
    let canvas = terminal.saturating_sub(drawer);
    Frame {
        terminal,
        drawer,
        canvas,
        class: width_class(canvas),
    }
}

/// Beside-label column. `xs` is 0 because the label sits above the value. `sm` caps at 18, other classes at 24.
pub fn label_column_width(titles: &[&str], class: WidthClass) -> usize {
    if class == WidthClass::Xs {
        return 0;
    }
    let cap = if class == WidthClass::Sm {
        LABEL_CAP_SM
    } else {
        LABEL_CAP
    };
    titles
        .iter()
        .map(|title| display_width(title).min(cap))
        .max()
        .unwrap_or(0)
}

/// Title padded with dot leaders out to `width` columns. A width of 0 returns the title.
pub fn dotted_label(title: &str, width: usize) -> String {
    if width == 0 {
        return title.to_string();
    }
    let shown = truncate_width(title, width);
    let gap = width.saturating_sub(display_width(&shown));
    let mut out = shown;
    out.push_str(&".".repeat(gap));
    out
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
    /// Columns of a `container-row`, side by side when a column starts past x 0.
    Columns(Vec<LaidColumn>),
    Form(LaidForm),
    List(LaidList),
    Link(LaidLink),
    Text(String),
}

/// One column of a row. `blocks` were placed at the parent width class, in `w` columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidColumn {
    pub x: u16,
    pub w: u16,
    pub blocks: Vec<Block>,
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
    write_blocks(&mut out, &laid.blocks, "");
    out
}

fn write_blocks(out: &mut String, blocks: &[Block], indent: &str) {
    for block in blocks {
        write_block(out, block, indent);
    }
}

fn write_block(out: &mut String, block: &Block, indent: &str) {
    match block {
        Block::Boxes(boxes) => {
            if boxes.len() > 1 && boxes.iter().any(|box_| box_.x > 0) {
                out.push_str(indent);
                out.push_str("row\n");
            }
            for box_ in boxes {
                out.push_str(&format!("{indent}box {} x={} w={}\n", box_.title, box_.x, box_.w));
                let child_indent = format!("{indent}  ");
                for child in &box_.children {
                    push_child(out, child, &child_indent);
                }
            }
        }
        Block::Columns(columns) => {
            if columns.iter().any(|column| column.x > 0) {
                out.push_str(indent);
                out.push_str("row\n");
            }
            let nested = format!("{indent}  ");
            for column in columns {
                out.push_str(&format!("{indent}col x={} w={}\n", column.x, column.w));
                write_blocks(out, &column.blocks, &nested);
            }
        }
        Block::Form(form) => {
            out.push_str(&format!("{indent}form {}\n", form.name));
            for field in &form.fields {
                out.push_str(&format!("{indent}  field {} {}\n", field.title, field.mode.name()));
            }
        }
        Block::List(list) => {
            out.push_str(&format!(
                "{indent}list {} scroll={}\n",
                list.name,
                if list.scroll { "true" } else { "false" }
            ));
            for col in &list.columns {
                let fixed = if col.fixed { " fixed" } else { "" };
                out.push_str(&format!("{indent}  col {} w={}{fixed}\n", col.title, col.width));
            }
            out.push_str(&format!("{indent}  page_rows={}\n", list.page_rows));
        }
        Block::Link(link) => out.push_str(&format!("{indent}link {}\n", link.text)),
        Block::Text(text) => out.push_str(&format!("{indent}text {text}\n")),
    }
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
            continue;
        }
        if let Node::Row { c, .. } = &nodes[index] {
            blocks.extend(place_row(c, class, canvas));
        } else if let Some(children) = transparent(&nodes[index]) {
            blocks.extend(place_nodes(children, class, canvas));
        } else if let Some(block) = place_one(&nodes[index], class, canvas) {
            blocks.push(block);
        }
        index += 1;
    }
    blocks
}

/// Section, container, and the other wrappers that are not themselves drawn.
fn transparent(node: &Node) -> Option<&[Node]> {
    match node {
        Node::Section { c, .. }
        | Node::Container { c, .. }
        | Node::Fragment { c, .. }
        | Node::Active { c, .. }
        | Node::Subscreen { c, .. }
        | Node::Panel { c, .. }
        | Node::Split { c, .. }
        | Node::Dashboard { c, .. }
        | Node::ButtonMenu { c, .. }
        | Node::Col { c, .. } => Some(c),
        _ => None,
    }
}

fn place_row(children: &[Node], class: WidthClass, canvas: u16) -> Vec<Block> {
    let columns: Vec<&Node> = children
        .iter()
        .filter(|node| matches!(node, Node::Col { .. }))
        .collect();
    let (spans, bodies): (Vec<Option<u16>>, Vec<&[Node]>) = if columns.is_empty() {
        (vec![None], vec![children])
    } else {
        (
            columns.iter().map(|node| span_at(node, class)).collect(),
            columns.iter().map(|node| node.children()).collect(),
        )
    };
    let mut blocks = Vec::new();
    for line in span_lines(&spans) {
        blocks.extend(place_line(&spans[line.clone()], &bodies[line], class, canvas));
    }
    blocks
}

/// Splits the columns into lines as Bootstrap wraps them: a column whose span would take its line
/// past 12 starts the next line.
fn span_lines(spans: &[Option<u16>]) -> Vec<std::ops::Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0u16;
    for (index, span) in spans.iter().enumerate() {
        let span = span.unwrap_or(0).min(12);
        if index > start && used + span > 12 {
            lines.push(start..index);
            start = index;
            used = 0;
        }
        used += span;
    }
    lines.push(start..spans.len());
    lines
}

fn place_line(spans: &[Option<u16>], bodies: &[&[Node]], class: WidthClass, canvas: u16) -> Vec<Block> {
    let slots = column_slots(spans, class, canvas);
    if slots.iter().all(|(x, _)| *x == 0) {
        let mut blocks = Vec::new();
        for body in bodies {
            blocks.extend(place_nodes(body, class, canvas));
        }
        return blocks;
    }
    let laid = slots
        .into_iter()
        .zip(bodies)
        .map(|((x, w), body)| LaidColumn {
            x,
            w,
            blocks: place_nodes(body, class, w),
        })
        .collect();
    vec![Block::Columns(laid)]
}

fn span_at(node: &Node, class: WidthClass) -> Option<u16> {
    let Node::Col { xs, sm, md, lg, .. } = node else {
        return None;
    };
    let text = match class {
        WidthClass::Xs => xs.as_str(),
        WidthClass::Sm => filled(sm).unwrap_or(xs),
        WidthClass::Md => filled(md).or_else(|| filled(sm)).unwrap_or(xs),
        WidthClass::Lg | WidthClass::Xl => filled(lg).or_else(|| filled(md)).or_else(|| filled(sm)).unwrap_or(xs),
    };
    let span = text.parse::<u16>().ok()?;
    (span > 0).then_some(span)
}

fn filled(text: &str) -> Option<&str> {
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn column_slots(spans: &[Option<u16>], class: WidthClass, canvas: u16) -> Vec<(u16, u16)> {
    let count = spans.len();
    if count == 0 {
        return Vec::new();
    }
    if class.stacks_boxes() {
        return vec![(0, canvas); count];
    }
    if spans.iter().all(Option::is_none) {
        return box_slots(count, class, canvas);
    }
    let widths = span_widths(spans, canvas);
    if widths.iter().any(|width| *width < BOX_MIN) {
        return vec![(0, canvas); count];
    }
    let mut slots = Vec::with_capacity(count);
    let mut x = 0u16;
    for width in widths {
        slots.push((x, width));
        x = x.saturating_add(width).saturating_add(1);
    }
    slots
}

fn span_widths(spans: &[Option<u16>], canvas: u16) -> Vec<u16> {
    let count = u16::try_from(spans.len()).unwrap_or(u16::MAX);
    let inner = canvas.saturating_sub(count.saturating_sub(1));
    let specified: u32 = spans.iter().copied().flatten().map(u32::from).sum();
    let flex_count = u16::try_from(spans.iter().filter(|span| span.is_none()).count()).unwrap_or(0);
    let specified_width = inner.saturating_mul(u16::try_from(specified.min(12)).unwrap_or(12)) / 12;
    let flex_total = inner.saturating_sub(specified_width);
    let flex_share = flex_total.checked_div(flex_count).unwrap_or(0);
    let mut flex_used = 0u16;
    let mut flex_seen = 0u16;
    spans
        .iter()
        .map(|span| match span {
            Some(span) => inner.saturating_mul((*span).min(12)) / 12,
            None => {
                flex_seen += 1;
                if flex_seen == flex_count {
                    flex_total.saturating_sub(flex_used)
                } else {
                    flex_used = flex_used.saturating_add(flex_share);
                    flex_share
                }
            }
        })
        .collect()
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
        _ => None,
    }
}

fn place_boxes(nodes: &[Node], class: WidthClass, canvas: u16) -> Vec<LaidBox> {
    let slots = box_slots(nodes.len(), class, canvas);
    nodes
        .iter()
        .zip(slots)
        .filter_map(|(node, (x, w))| {
            let Node::Box { id, title, c, .. } = node else {
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
            _ => {}
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
    list_widths(&layout(model, canvas).blocks, list_id).unwrap_or_default()
}

fn list_widths(blocks: &[Block], list_id: &str) -> Option<Vec<usize>> {
    for block in blocks {
        match block {
            Block::List(list) if list.id == list_id => {
                return Some(list.columns.iter().map(|col| col.width).collect());
            }
            Block::Columns(columns) => {
                for column in columns {
                    if let Some(found) = list_widths(&column.blocks, list_id) {
                        return Some(found);
                    }
                }
            }
            Block::Boxes(_) | Block::Form(_) | Block::List(_) | Block::Link(_) | Block::Text(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::outline;
    use crate::model::ScreenModel;

    const ROW: &str = r#"{"v":1,"screen":{"path":"/row","title":"Row"},"c":[{"t":"row","id":"r","c":[
        {"t":"col","id":"a","lg":"6","c":[{"t":"box","id":"ba","title":"Left","c":[]}]},
        {"t":"col","id":"b","lg":"6","c":[{"t":"box","id":"bb","title":"Right","c":[]}]}
    ]}]}"#;

    /// Distinct `col x=` values. A list header is `col Title`, which does not match.
    fn column_xs(text: &str) -> Vec<u16> {
        let mut xs = Vec::new();
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("col x=") else {
                continue;
            };
            let Some(token) = rest.split_whitespace().next() else {
                continue;
            };
            let Ok(x) = token.parse::<u16>() else {
                continue;
            };
            if !xs.contains(&x) {
                xs.push(x);
            }
        }
        xs
    }

    #[test]
    fn a_row_of_two_lg_columns_sits_side_by_side_at_md_and_stacks_at_sm() {
        let model = ScreenModel::from_json(ROW).unwrap();
        let wide = outline(&model, 154);
        assert!(column_xs(&wide).len() >= 2, "{wide}");
        let narrow = outline(&model, 100);
        assert!(column_xs(&narrow).len() < 2, "{narrow}");
    }
}
