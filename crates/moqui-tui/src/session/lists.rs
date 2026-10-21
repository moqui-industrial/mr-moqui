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

//! A list's rows, selection, paging and sort.

use super::*;

impl Session {
    /// Up and Down move through a list's rows; past the first or last row they move focus on, as Lynx does.
    pub(super) fn list_key(&mut self, press: &KeyPress) -> bool {
        let Some(item) = self.focus().cloned().filter(|item| item.kind == FocusKind::List) else {
            return false;
        };
        let rows = self.list(&item.key).map_or(0, |list| list.rows.len());
        let at = self.row_at(&item.key);
        match (&press.key, press.shift) {
            (Key::Up, false) if at > 0 => {
                self.rows.insert(item.key, at - 1);
                true
            }
            (Key::Down, false) if at + 1 < rows => {
                self.rows.insert(item.key, at + 1);
                true
            }
            // Right follows the row's link as Lynx does; Enter edits a row that has fields.
            (Key::Right, false) if press.is_plain() => {
                let link = self.list_page(&item.key).get(at).and_then(row_link).map(str::to_string);
                match link {
                    Some(url) => {
                        self.follow_url(&url);
                        true
                    }
                    None => false,
                }
            }
            (Key::Right, true) => {
                *self.hscroll.entry(item.key).or_default() += 1;
                true
            }
            (Key::Left, true) => {
                let scroll = self.hscroll.entry(item.key).or_default();
                *scroll = scroll.saturating_sub(1);
                true
            }
            _ => false,
        }
    }

    pub(super) fn toggle_row(&mut self) {
        let Some(list_id) = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
        else {
            return;
        };
        let Some(id) = self.selectable_row(&list_id, self.row_at(&list_id)) else {
            return;
        };
        let set = self.selected.entry(list_id).or_default();
        if !set.remove(&id) {
            set.insert(id);
        }
    }

    pub(super) fn select_all(&mut self) {
        let Some(list_id) = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
        else {
            return;
        };
        let rows = self.list(&list_id).map_or(0, |list| list.rows.len());
        let ids: BTreeSet<String> = (0..rows).filter_map(|at| self.selectable_row(&list_id, at)).collect();
        let set = self.selected.entry(list_id).or_default();
        if set.len() == ids.len() {
            set.clear();
        } else {
            *set = ids;
        }
    }

    /// The fields of a list row, for its editor: each with its column's title when it has none of its own.
    pub fn row_nodes(&self, list_id: &str, row: usize) -> Vec<Node> {
        let Some(list) = self.list(list_id) else {
            return Vec::new();
        };
        let Some(cells) = list.rows.get(row) else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        for (at, cell) in cells.cells.iter().enumerate() {
            let column = list
                .columns
                .get(at)
                .map(|column| column.title.clone())
                .unwrap_or_default();
            for node in cell.nodes() {
                if let Node::Field { title, .. } = node {
                    let mut node = node.clone();
                    if title.is_empty() {
                        if let Node::Field { title, .. } = &mut node {
                            title.clone_from(&column);
                        }
                    }
                    nodes.push(node);
                }
            }
        }
        // The multi form's buttons save every row, so they are in each row's editor.
        if list.multi {
            nodes.extend(list.buttons.iter().cloned());
        }
        nodes
    }

    /// The selected rows of a list as its row action posts them (`m-checkbox-set`): `parameter=a,b` in list mode, else
    /// `parameter_N` for each with `_isMulti`. None when no row is selected.
    pub(super) fn selection_parameters(&self, list_id: &str) -> Option<BTreeMap<String, String>> {
        let list = self.list(list_id)?;
        let selection = list.row_selection.as_ref()?;
        let ids: Vec<&String> = self.selected.get(list_id)?.iter().collect();
        if ids.is_empty() {
            return None;
        }
        let name = if selection.parameter.is_empty() {
            selection.id_field.clone()
        } else {
            selection.parameter.clone()
        };
        let mut out = BTreeMap::new();
        if selection.list_mode == "true" {
            out.insert(name, ids.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(","));
        } else {
            for (at, id) in ids.iter().enumerate() {
                out.insert(format!("{name}_{at}"), (*id).clone());
            }
            out.insert("_isMulti".to_string(), "true".to_string());
        }
        Some(out)
    }

    pub(super) fn selectable_row(&self, list_id: &str, at: usize) -> Option<String> {
        let list = self.list(list_id)?;
        list.row_selection.as_ref()?;
        let row = list.rows.get(at)?;
        Some(if row.select_id.is_empty() {
            row.id.clone()
        } else {
            row.select_id.clone()
        })
    }

    pub(super) fn first_list(&self, with_find: bool) -> Option<String> {
        self.focus
            .iter()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
            .find(|id| !with_find || self.list(id).is_some_and(|list| !list.header_fields.is_empty()))
    }

    /// The focused list, else the first one on the screen.
    pub(super) fn target_list(&self) -> Option<String> {
        self.focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
            .or_else(|| self.first_list(false))
    }

    pub(super) fn has_paged_list(&self) -> bool {
        self.index.lists.values().any(|list| page_count_of(list) > 1)
    }

    pub(super) fn turn_page(&mut self, delta: isize) {
        let Some(list_id) = self.target_list() else {
            return;
        };
        let count = self.page_count(&list_id) as isize;
        let current = self.page_index(&list_id) as isize;
        let next = (current + delta).clamp(0, count - 1);
        if next == current {
            return;
        }
        let mut params = self.parameters().clone();
        params.insert("pageIndex".to_string(), next.to_string());
        self.rows.insert(list_id, 0);
        self.reload_with(params);
    }

    pub(super) fn cycle_sort(&mut self) {
        let Some(list_id) = self
            .focus()
            .filter(|item| item.kind == FocusKind::List)
            .map(|item| item.key.clone())
        else {
            return;
        };
        let fields: Vec<String> = self
            .list(&list_id)
            .map(|list| {
                list.columns
                    .iter()
                    .filter(|column| !column.order_by.is_empty())
                    .map(|column| column.order_by.clone())
                    .collect()
            })
            .unwrap_or_default();
        if fields.is_empty() {
            self.set_message("This list does not sort.".to_string(), false);
            return;
        }
        let next = next_order(&fields, self.order_by());
        let mut params = self.parameters().clone();
        if next.is_empty() {
            params.remove("orderByField");
        } else {
            params.insert("orderByField".to_string(), next);
        }
        params.remove("pageIndex");
        // The rows come back in another order, so the current row starts at the top again.
        self.rows.insert(list_id, 0);
        self.reload_with(params);
    }
}

pub(super) fn page_size_of(list: &FormList) -> usize {
    if list.paging.size == 0 {
        PAGE_ROWS
    } else {
        usize::try_from(list.paging.size).unwrap_or(PAGE_ROWS).max(1)
    }
}

pub fn page_count_of(list: &FormList) -> usize {
    if let Some(max) = list.paging.max_index {
        return usize::try_from(max).unwrap_or(0) + 1;
    }
    let size = page_size_of(list);
    let count = if list.paging.count == 0 {
        list.rows.len()
    } else {
        usize::try_from(list.paging.count).unwrap_or(list.rows.len())
    };
    count.div_ceil(size).max(1)
}

/// Ascending then descending on each sortable column in turn, then unsorted.
pub(super) fn next_order(fields: &[String], current: &str) -> String {
    let (descending, name) = match current.strip_prefix('-') {
        Some(name) => (true, name),
        None => (false, current),
    };
    let bare = |field: &str| field.trim_start_matches('^').to_string();
    match fields.iter().position(|field| bare(field) == bare(name)) {
        None => fields[0].clone(),
        Some(at) if !descending => format!("-{}", fields[at]),
        Some(at) => fields.get(at + 1).cloned().unwrap_or_default(),
    }
}
