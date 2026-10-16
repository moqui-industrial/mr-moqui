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

//! The screen statistics of the tools artifact stats screen: `ScreenFacadeImpl.getAllRootScreenLocations` and
//! `getScreenInfoList`. Each screen counts its own forms, trees, sections, and transitions, and the totals of all
//! the screens under it. The whole tree is walked; the list stops at `levels`.

use indexmap::IndexMap;
use moqui_lib::node::MNode;
use moqui_lib::object::Value;

use super::cache::ScreenFacade;
use super::definition::ScreenDef;
use crate::context::{Error, ExecutionContextFactory};

/// How deep the walk goes before it stops, against a screen that mounts an ancestor.
const MAX_DEPTH: usize = 24;

/// The `root-screen` locations of every `webapp` in the conf, in conf order.
pub fn all_root_screen_locations(factory: &ExecutionContextFactory) -> Vec<String> {
    let Ok(conf) = MNode::parse_text("moqui-conf", factory.conf_text()) else {
        return Vec::new();
    };
    let Some(list) = conf.first_named("webapp-list") else {
        return Vec::new();
    };
    list.children_named("webapp")
        .iter()
        .flat_map(|webapp| webapp.children_named("root-screen"))
        .filter_map(|root| root.attribute("location"))
        .collect()
}

#[derive(Default)]
struct Totals {
    subscreens: i64,
    non_placeholder: i64,
    forms: i64,
    trees: i64,
    sections: i64,
    transitions: i64,
    with_actions: i64,
}

struct Info {
    row: IndexMap<String, Value>,
    level: usize,
    own: Totals,
    all: Totals,
    children: Vec<Info>,
}

/// One row for each screen under `root_location`, down to `levels`, in menu order. Each row has the counts of the
/// screen (`forms`, `transitions`, ...) and of all the screens under it (`allSubscreensForms`, ...), its `level`, its
/// `name`, and `ssi`, the subscreens item that mounts it (null for the root).
///
/// # Errors
///
/// Returns an error when the root screen does not load. A subscreen that does not load is left out, as Java logs it.
pub fn screen_info_list(
    factory: &ExecutionContextFactory,
    root_location: &str,
    levels: usize,
) -> Result<Vec<IndexMap<String, Value>>, Error> {
    let facade = ScreenFacade::of(factory);
    let root = facade.screen_def(factory, root_location)?;
    let mut path = vec![root.location.clone()];
    let info = walk(&facade, factory, &root, None, 0, &mut path);
    let mut out = Vec::new();
    flatten(info, levels, &mut out);
    Ok(out)
}

fn walk(
    facade: &ScreenFacade,
    factory: &ExecutionContextFactory,
    def: &ScreenDef,
    item: Option<(&str, String, Option<i64>)>,
    level: usize,
    path: &mut Vec<String>,
) -> Info {
    let own = Totals {
        subscreens: count(def.subscreens.items.len()),
        non_placeholder: 0,
        forms: count(def.forms.len()),
        trees: count(def.trees.len()),
        sections: count(def.sections.len()),
        transitions: count(def.transitions.len()),
        with_actions: count(
            def.transitions
                .iter()
                .filter(|transition| transition.actions.is_some())
                .count(),
        ),
    };
    let is_non_placeholder = own.forms > 0 || own.sections > 0 || own.transitions > 4;
    let mut items: Vec<_> = def.subscreens.items.iter().collect();
    items.sort_by(|left, right| left.name.cmp(&right.name));
    // Java sorts by menu index, a missing index first, keeping the name order otherwise.
    items.sort_by_key(|item| item.menu_index);
    let mut children = Vec::new();
    if level < MAX_DEPTH {
        for child in items {
            if path.contains(&child.location) {
                continue;
            }
            let Ok(child_def) = facade.screen_def(factory, &child.location) else {
                tracing::info!(
                    "While getting ScreenInfo screen not found for {} at: {}",
                    child.name,
                    child.location
                );
                continue;
            };
            path.push(child.location.clone());
            children.push(walk(
                facade,
                factory,
                &child_def,
                Some((child.name.as_str(), child.menu_title.clone(), child.menu_index)),
                level + 1,
                path,
            ));
            path.pop();
        }
    }
    let mut all = Totals::default();
    for child in &children {
        all.subscreens += 1 + child.all.subscreens;
        all.non_placeholder += i64::from(child_is_non_placeholder(child)) + child.all.non_placeholder;
        all.forms += child.own.forms + child.all.forms;
        all.trees += child.own.trees + child.all.trees;
        all.sections += child.own.sections + child.all.sections;
        all.transitions += child.own.transitions + child.all.transitions;
        all.with_actions += child.own.with_actions + child.all.with_actions;
    }
    let mut row = IndexMap::new();
    let name = item
        .as_ref()
        .map_or_else(|| def.screen_name.clone(), |(name, _, _)| (*name).to_string());
    row.insert("name".to_string(), Value::string(&name));
    row.insert("level".to_string(), Value::I64(count(level)));
    row.insert("location".to_string(), Value::string(&def.location));
    row.insert(
        "ssi".to_string(),
        match &item {
            Some((name, title, index)) => {
                let mut ssi = IndexMap::new();
                ssi.insert("name".to_string(), Value::string(*name));
                ssi.insert("menuTitle".to_string(), Value::string(title));
                ssi.insert("menuIndex".to_string(), index.map_or(Value::Null, Value::I64));
                Value::Map(ssi)
            }
            None => Value::Null,
        },
    );
    row.insert("isNonPlaceholder".to_string(), Value::Bool(is_non_placeholder));
    Info {
        row,
        level,
        own,
        all,
        children,
    }
}

fn child_is_non_placeholder(info: &Info) -> bool {
    info.row.get("isNonPlaceholder") == Some(&Value::Bool(true))
}

fn flatten(info: Info, levels: usize, out: &mut Vec<IndexMap<String, Value>>) {
    let mut row = info.row;
    let pairs = [
        ("subscreens", info.own.subscreens),
        ("allSubscreens", info.all.subscreens),
        // Java does not count the non-placeholder subscreens of one level, only of all levels.
        ("subscreensNonPlaceholder", info.own.non_placeholder),
        ("allSubscreensNonPlaceholder", info.all.non_placeholder),
        ("forms", info.own.forms),
        ("allSubscreensForms", info.all.forms),
        ("trees", info.own.trees),
        ("allSubscreensTrees", info.all.trees),
        ("sections", info.own.sections),
        ("allSubscreensSections", info.all.sections),
        ("transitions", info.own.transitions),
        ("allSubscreensTransitions", info.all.transitions),
        ("transitionsWithActions", info.own.with_actions),
        ("allSubscreensTransitionsWithActions", info.all.with_actions),
    ];
    for (name, number) in pairs {
        row.insert(name.to_string(), Value::I64(number));
    }
    out.push(row);
    // `addChildrenToList`: the root lists its children, and each lists its own while the list goes deeper.
    for child in info.children {
        if child.level <= levels.max(1) {
            flatten(child, levels, out);
        }
    }
}

fn count(number: usize) -> i64 {
    i64::try_from(number).unwrap_or(i64::MAX)
}
