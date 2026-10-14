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

//! Forms as the macros see them: the form element with `extends`, `widget-template-include`,
//! `auto-fields-entity`, and `screen-extend` forms merged in, as `ScreenForm.initForm` does.
//!
//! Not ported yet: `auto-fields-service` and `auto-widget-service` (the form is built without
//! those fields and a warning is logged), `DbForm` forms, `dynamic="true"` re-assembly per render,
//! and the drop-down an `auto-fields-entity` field gets for a related entity (it gets a text
//! widget instead). A form is assembled once per definition and kept with it.

use std::sync::Arc;

use moqui_lib::node::MNode;

use super::cache::ScreenFacade;
use super::definition::{Actions, ScreenDef};
use crate::context::{Error, ExecutionContext, ExecutionContextFactory};
use crate::script;

const MAX_EXTENDS_DEPTH: usize = 8;

/// An assembled form. `xml` is the form element the macros walk.
#[derive(Debug)]
pub struct FormInstance {
    pub name: String,
    pub list: bool,
    pub xml: Arc<str>,
    /// `form-list` only: the list name, and `list-entry` when given.
    pub list_name: String,
    pub list_entry: String,
    /// The form's `entity-find` as actions that fill `list_name`.
    pub entity_find: Option<Actions>,
    pub row_actions: Option<Actions>,
    /// Fields with a `from` expression, evaluated for each row after the row actions.
    pub field_from: Vec<(String, String)>,
}

impl FormInstance {
    /// Parse the assembled form on this thread.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored XML does not parse.
    pub fn node(&self) -> Result<MNode, Error> {
        MNode::parse_text("assembled-form", &self.xml).map_err(Error::from)
    }

    fn of(def: &ScreenDef, node: &MNode) -> Result<Self, Error> {
        let name = node.attribute("name").unwrap_or_default();
        let list = node.name() == "form-list";
        let label = format!("{} form {name}", def.location);
        let list_name = node.attribute("list").unwrap_or_default();
        let entity_find = match node.first_named("entity-find") {
            Some(find) if list => {
                let copy = find.deep_copy();
                copy.set_attribute("list", list_name.clone());
                copy.set_attribute("form-select-fields", form_select_fields(node).join(","));
                let actions = MNode::new("actions");
                actions.append_node(copy);
                Some(Actions::from_xml(
                    actions.write_string(),
                    &format!("{label} entity-find"),
                )?)
            }
            _ => None,
        };
        let row_actions = match node.first_named("row-actions") {
            Some(actions) if list => Some(Actions::from_node(&actions, &format!("{label} row-actions"))?),
            _ => None,
        };
        let mut field_from = Vec::new();
        if list {
            for field in node.children_named("field") {
                if let Some(from) = field.attribute("from").filter(|text| !text.is_empty()) {
                    crate::script::parse_js(&from)
                        .map_err(|err| Error::new(format!("{label}: field from `{from}`: {}", err.message())))?;
                    field_from.push((field.attribute("name").unwrap_or_default(), from));
                }
            }
        }
        Ok(Self {
            name,
            list,
            xml: Arc::from(node.write_string()),
            list_name,
            list_entry: node.attribute("list-entry").unwrap_or_default(),
            entity_find,
            row_actions,
            field_from,
        })
    }
}

/// The fields a form-list's `entity-find` selects, as Java's `FormListRenderInfo`: the fields of the
/// columns (the `form-list-column` or `columns` field refs, otherwise each field whose widget is not
/// hidden), then the hidden fields that have a `default-field`. Java takes the columns from the
/// user's saved column selection when there is one; here the definition's columns are a superset of it.
fn form_select_fields(form: &MNode) -> Vec<String> {
    let fields = form.children_named("field");
    let mut names: Vec<String> = Vec::new();
    let mut push = |name: String| {
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    };
    let column_refs: Vec<String> = form
        .children()
        .into_iter()
        .filter(|child| child.name() == "form-list-column" || child.name() == "columns")
        .flat_map(|child| descendants_named(&child, "field-ref"))
        .filter_map(|field_ref| field_ref.attribute("name"))
        .collect();
    if column_refs.is_empty() {
        for field in &fields {
            if !list_field_hidden_widget(field) && field.attribute("hide").as_deref() != Some("true") {
                push(field.attribute("name").unwrap_or_default());
            }
        }
    } else {
        column_refs.into_iter().for_each(&mut push);
    }
    for field in &fields {
        if list_field_hidden_widget(field) && field.first_named("default-field").is_some() {
            push(field.attribute("name").unwrap_or_default());
        }
    }
    names
}

/// Java's `isListFieldHiddenWidget`: the default field and each conditional field are hidden or ignored.
fn list_field_hidden_widget(field: &MNode) -> bool {
    let shown = |sub: &MNode| sub.first_named("hidden").is_none() && sub.first_named("ignored").is_none();
    if field.first_named("default-field").is_some_and(|sub| shown(&sub)) {
        return false;
    }
    !field.children_named("conditional-field").iter().any(shown)
}

/// The assembled form `name` of `def`, from the definition's cache when it was assembled before.
///
/// # Errors
///
/// Returns an error when the form is not in the screen, an `extends` target is missing, or an
/// expression in the form is not JavaScript.
pub fn assemble(
    facade: &ScreenFacade,
    factory: &ExecutionContextFactory,
    def: &ScreenDef,
    name: &str,
) -> Result<Arc<FormInstance>, Error> {
    if let Some(found) = def.generated.form(name) {
        return Ok(found);
    }
    let node = assemble_node(facade, factory, def, name, 0, false)?;
    let instance = Arc::new(FormInstance::of(def, &node)?);
    def.generated.put_form(name, Arc::clone(&instance));
    Ok(instance)
}

/// A `dynamic="true"` form assembled from the context as it is now: `${}` in `extends` and in the
/// `entity-name` of `auto-fields-entity` is expanded. Nothing is cached, since the next render may differ.
///
/// # Errors
///
/// Same as [`assemble`].
pub fn assemble_dynamic(
    facade: &ScreenFacade,
    factory: &ExecutionContextFactory,
    def: &ScreenDef,
    name: &str,
) -> Result<Arc<FormInstance>, Error> {
    let node = assemble_node(facade, factory, def, name, 0, true)?;
    Ok(Arc::new(FormInstance::of(def, &node)?))
}

fn assemble_node(
    facade: &ScreenFacade,
    factory: &ExecutionContextFactory,
    def: &ScreenDef,
    name: &str,
    depth: usize,
    dynamic: bool,
) -> Result<MNode, Error> {
    if depth > MAX_EXTENDS_DEPTH {
        return Err(Error::new(format!(
            "form extends are nested too deeply at {}#{name}",
            def.location
        )));
    }
    let form = def
        .forms
        .get(name)
        .ok_or_else(|| Error::new(format!("Could not find form {name} in screen {}", def.location)))?;
    let base = form.node.node()?;
    if form.dynamic && !dynamic {
        return Err(Error::new(format!(
            "form {name} in {} is dynamic (dynamic=\"true\"), and dynamic forms are not rendered yet",
            def.location
        )));
    }
    let extensions: Vec<MNode> = form.extensions.iter().map(|ext| ext.node()).collect::<Result<_, _>>()?;
    let assembled = MNode::new(base.name());
    let mut column_info: Vec<(String, Vec<String>)> = Vec::new();

    if let Some(mut extends) = base.attribute("extends").filter(|text| !text.is_empty()) {
        if dynamic {
            extends = script::expand(&extends)?;
        }
        let found = extends_node(facade, factory, def, &extends, depth, dynamic)?;
        merge_form_nodes(&assembled, &found, true, true);
    }
    let is_list = base.name() == "form-list";
    for sub in base.children() {
        match sub.name().as_str() {
            "field" => {
                let copy = sub.deep_copy();
                expand_field_node(facade, factory, &assembled, &copy)?;
                merge_field_node(&assembled, &copy, false);
            }
            "auto-fields-entity" => {
                let mut entity_name = sub.attribute("entity-name").unwrap_or_default();
                if dynamic {
                    entity_name = script::expand(&entity_name)?;
                    if entity_name.is_empty() || entity_name == "null" {
                        entity_name = script::context_value("formLocationExtension")
                            .map(|value| moqui_lib::object::to_plain_string(&value))
                            .unwrap_or_default();
                    }
                }
                let shape = entity_shape(&entity_name)?;
                let excludes: Vec<String> = sub
                    .children_named("exclude")
                    .iter()
                    .filter_map(|exclude| exclude.attribute("field-name"))
                    .collect();
                let include = sub
                    .attribute("include")
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| "all".to_string());
                let field_type = sub
                    .attribute("field-type")
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| "find-display".to_string());
                let displayed = add_entity_fields(&shape, &include, &field_type, &excludes, &assembled);
                if is_list && sub.attribute("auto-columns").as_deref() != Some("false") {
                    column_info.push((entity_name, displayed));
                }
            }
            "auto-fields-service" => {
                tracing::warn!(
                    "form {name} in {}: auto-fields-service is not supported, its fields are left out",
                    def.location
                );
            }
            _ => {}
        }
    }
    merge_form_nodes(&assembled, &base, false, false);
    for extension in &extensions {
        merge_form_nodes(&assembled, extension, true, true);
    }

    // A field-layout with fields it does not reference still needs the hidden ones, so they submit.
    if let Some(layout) = assembled.first_named("field-layout") {
        if !has_descendant(&layout, "fields-not-referenced") {
            for field in assembled.children_named("field") {
                let field_name = field.attribute("name").unwrap_or_default();
                let referenced = descendants_named(&layout, "field-ref")
                    .iter()
                    .any(|reference| reference.attribute("name").as_deref() == Some(field_name.as_str()));
                if !referenced && has_descendant(&field, "hidden") {
                    add_field_to_layout(&assembled, &field);
                }
            }
        }
    }
    for (entity_name, displayed) in &column_info {
        let shape = entity_shape(entity_name)?;
        add_auto_entity_columns(&assembled, &base, &shape, displayed);
    }
    Ok(assembled)
}

fn extends_node(
    facade: &ScreenFacade,
    factory: &ExecutionContextFactory,
    def: &ScreenDef,
    extends: &str,
    depth: usize,
    dynamic: bool,
) -> Result<MNode, Error> {
    let (location, form_name) = match extends.split_once('#') {
        Some((location, form_name)) => (location, form_name),
        None => (def.location.as_str(), extends),
    };
    if location == def.location {
        return assemble_node(facade, factory, def, form_name, depth + 1, dynamic);
    }
    if location == "moqui.screen.form.DbForm" || location == "DbForm" {
        return Err(Error::new("form extends of a DbForm is not supported"));
    }
    let other = facade.screen_def(factory, location)?;
    // The assembled form is kept with this definition, so a change to the other screen makes this one stale.
    def.generated.add_dependencies(&other.sources);
    assemble_node(facade, factory, &other, form_name, depth + 1, dynamic)
}

fn has_descendant(node: &MNode, name: &str) -> bool {
    !descendants_named(node, name).is_empty()
}

fn descendants_named(node: &MNode, name: &str) -> Vec<MNode> {
    let mut out = Vec::new();
    for child in node.children() {
        if child.name() == name {
            out.push(child.clone());
        }
        out.extend(descendants_named(&child, name));
    }
    out
}

/// `ScreenForm.mergeFormNodes`.
fn merge_form_nodes(base: &MNode, over: &MNode, deep_copy: bool, copy_fields: bool) {
    base.copy_attributes_from(over);
    if let Some(find) = over.first_named("entity-find") {
        match base
            .first_named("entity-find")
            .and_then(|existing| base.index_of(&existing))
        {
            Some(index) => {
                let _ = base.replace_child(index, find.deep_copy());
            }
            None => base.insert_node(0, find.deep_copy()),
        }
    }
    if let Some(actions) = over.first_named("row-actions") {
        let target = base
            .first_named("row-actions")
            .unwrap_or_else(|| base.append("row-actions"));
        for action in actions.children() {
            target.append_node(action.deep_copy());
        }
    }
    if let Some(selection) = over.first_named("row-selection") {
        let target = base
            .first_named("row-selection")
            .unwrap_or_else(|| base.append("row-selection"));
        target.copy_attributes_from(&selection);
        for action in selection.children() {
            target.append_node(action.deep_copy());
        }
    }
    if let Some(hidden) = over.first_named("hidden-parameters") {
        base.remove_where(|child| child.name() == "hidden-parameters");
        base.append_node(hidden.deep_copy());
    }
    if copy_fields {
        for field in over.children_named("field") {
            merge_field_node(base, &field, deep_copy);
        }
    }
    if let Some(layout) = over.first_named("field-layout") {
        base.remove_where(|child| child.name() == "field-layout");
        base.append_node(layout.deep_copy());
    }
    if over.first_named("form-list-column").is_some() {
        base.remove_where(|child| child.name() == "form-list-column");
        for column in over.children_named("form-list-column") {
            base.append_node(column.deep_copy());
        }
    }
    for columns in over.children_named("columns") {
        let kind = columns.attribute("type");
        base.remove_where(|child| child.name() == "columns" && child.attribute("type") == kind);
        base.append_node(columns.deep_copy());
    }
}

/// `ScreenForm.mergeFieldNode`: a field of the same name is merged in place, else appended.
fn merge_field_node(base: &MNode, over: &MNode, deep_copy: bool) {
    let name = over.attribute("name");
    let existing = base
        .children()
        .into_iter()
        .find(|child| child.name() == "field" && child.attribute("name") == name);
    match existing {
        Some(field) => {
            field.copy_attributes_from(over);
            for single in ["header-field", "first-row-field", "second-row-field"] {
                field.merge_single_child(over, single);
            }
            field.merge_children_by_keys(over, "conditional-field", &["condition"], None);
            for single in ["default-field", "last-row-field"] {
                field.merge_single_child(over, single);
            }
        }
        None => {
            base.append_node(if deep_copy { over.deep_copy() } else { over.clone() });
            if base.first_named("field-layout").is_some() {
                add_field_to_layout(base, over);
            }
        }
    }
}

fn add_field_to_layout(form: &MNode, field: &MNode) {
    let Some(layout) = form.first_named("field-layout") else {
        return;
    };
    let reference = layout.append("field-ref");
    reference.set_attribute("name", field.attribute("name").unwrap_or_default());
}

/// Replace `widget-template-include` and `auto-widget-entity` in the sub-nodes of a field.
fn expand_field_node(
    facade: &ScreenFacade,
    factory: &ExecutionContextFactory,
    form: &MNode,
    field: &MNode,
) -> Result<(), Error> {
    let _ = facade;
    for sub in field.children() {
        if !matches!(
            sub.name().as_str(),
            "header-field"
                | "first-row-field"
                | "second-row-field"
                | "conditional-field"
                | "default-field"
                | "last-row-field"
        ) {
            continue;
        }
        expand_field_sub_node(factory, form, field, &sub)?;
    }
    Ok(())
}

fn expand_field_sub_node(
    factory: &ExecutionContextFactory,
    form: &MNode,
    field: &MNode,
    sub: &MNode,
) -> Result<(), Error> {
    let Some(widget) = sub.first_child() else {
        return Ok(());
    };
    match widget.name().as_str() {
        "widget-template-include" => {
            let location = widget.attribute("location").unwrap_or_default();
            let Some((file, template_name)) = location.split_once('#') else {
                return Err(Error::new(format!(
                    "widget-template-include.@location must contain a hash/pound sign to separate the file location and widget-template.@name: [{location}]"
                )));
            };
            let text = factory.resource().get(file)?.read_text()?;
            let templates = MNode::parse_text(file, &text)?;
            let template = templates
                .children()
                .into_iter()
                .find(|node| node.attribute("name").as_deref() == Some(template_name))
                .ok_or_else(|| Error::new(format!("Could not find widget-template [{template_name}] in [{file}]")))?;
            let others: Vec<MNode> = sub.children().into_iter().skip(1).collect();
            let sets = widget.children_named("set");
            sub.clear_children();
            for node in template.children() {
                sub.append_node(node.deep_copy());
            }
            for node in others {
                sub.append_node(node.deep_copy());
            }
            for set in sets {
                sub.append_node(set.deep_copy());
            }
        }
        "auto-widget-entity" => {
            let entity_name = widget.attribute("entity-name").unwrap_or_default();
            let shape = entity_shape(&entity_name)?;
            let field_name = widget
                .attribute("field-name")
                .filter(|text| !text.is_empty())
                .or_else(|| field.attribute("name"))
                .unwrap_or_default();
            let field_type = widget
                .attribute("field-type")
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "find-display".to_string());
            sub.clear_children();
            add_auto_entity_field(&shape, &field_name, &field_type, field, Some(sub), form);
        }
        "auto-widget-service" => {
            tracing::warn!(
                "auto-widget-service is not supported, the widget of field {:?} is left out",
                field.attribute("name")
            );
            sub.clear_children();
        }
        _ => {}
    }
    Ok(())
}

/// What the form builder needs of an entity definition.
pub(super) struct EntityShape {
    pub full_name: String,
    /// Name, type, and whether it is a primary key, in definition order.
    pub fields: Vec<(String, String, bool)>,
    pub pk_fields: Vec<String>,
    /// Related `one` entities reached by a single key field: the field, the related entity, its key field.
    pub one_relations: Vec<(String, String, String)>,
    pub description_field: String,
}

pub(super) fn entity_shape(entity_name: &str) -> Result<EntityShape, Error> {
    let facade = ExecutionContext::with_current(|ec| ec.factory().entity())
        .flatten()
        .ok_or_else(|| Error::new("no entity facade is installed"))?;
    facade
        .with_model(|model| -> Result<EntityShape, moqui_lib::error::Error> {
            let entity = model.entity(entity_name)?;
            let fields: Vec<(String, String, bool)> = entity
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.field_type.clone(), field.is_pk))
                .collect();
            let mut one_relations = Vec::new();
            for relationship in &entity.relationships {
                // The expanded keys, so a relationship with no key-map (the related entity's primary key of the
                // same name) counts, as in Java.
                if relationship.relationship_type != "one"
                    || relationship.is_auto_reverse
                    || relationship.key_pairs.len() != 1
                {
                    continue;
                }
                let (field, related_key) = relationship.key_pairs[0].clone();
                one_relations.push((field, relationship.related.clone(), related_key));
            }
            let description_field = entity
                .fields
                .iter()
                .filter(|field| !field.is_pk)
                .find(|field| field.name.ends_with("Name"))
                .map(|field| field.name.clone())
                .or_else(|| entity.field("description").map(|field| field.name.clone()))
                .unwrap_or_default();
            Ok(EntityShape {
                full_name: entity.full_name.clone(),
                pk_fields: fields
                    .iter()
                    .filter(|(_, _, is_pk)| *is_pk)
                    .map(|(name, _, _)| name.clone())
                    .collect(),
                fields,
                one_relations,
                description_field,
            })
        })
        .map_err(Error::from)
}

fn add_entity_fields(
    shape: &EntityShape,
    include: &str,
    field_type: &str,
    excludes: &[String],
    form: &MNode,
) -> Vec<String> {
    let mut displayed = Vec::new();
    let want_pk = include == "all" || include == "pk";
    let want_non_pk = include == "all" || include == "nonpk";
    for (name, entity_type, is_pk) in &shape.fields {
        if (*is_pk && !want_pk) || (!*is_pk && !want_non_pk) {
            continue;
        }
        if excludes.contains(name) || name == "lastUpdatedStamp" {
            continue;
        }
        let mut make_default = true;
        if form.name() == "form-list"
            && matches!(
                entity_type.as_str(),
                "text-long" | "text-very-long" | "text-fts" | "binary-very-long"
            )
        {
            if entity_type != "text-long" {
                continue;
            }
            make_default = true;
        }
        displayed.push(name.clone());
        let field = MNode::new("field");
        field.set_attribute("name", name.clone());
        let sub = make_default.then(|| {
            let sub = field.append("default-field");
            sub.set_attribute("validate-entity", shape.full_name.clone());
            sub.set_attribute("validate-field", name.clone());
            sub
        });
        add_auto_entity_field(shape, name, field_type, &field, sub.as_ref(), form);
        merge_field_node(form, &field, false);
    }
    displayed
}

fn add_auto_entity_field(
    shape: &EntityShape,
    field_name: &str,
    field_type: &str,
    field: &MNode,
    sub: Option<&MNode>,
    form: &MNode,
) {
    let entity_type = shape
        .fields
        .iter()
        .find(|(name, _, _)| name == field_name)
        .map_or("text-long", |(_, field_type, _)| field_type.as_str());
    let relation = shape.one_relations.iter().find(|(key, _, _)| key == field_name);
    let is_list = form.name() == "form-list";
    let header = |order: &str| {
        if is_list && field.first_named("header-field").is_none() {
            let header = field.append("header-field");
            header.set_attribute("show-order-by", order);
        }
    };
    let display_entity = |target: &MNode, relation: &(String, String, String)| {
        // The text is over the related entity's value, so it is that entity's description field (Java's
        // `getDefaultDescriptionField` of the related definition), not the form entity's.
        let description = entity_shape(&relation.1)
            .map(|related| related.description_field)
            .unwrap_or_default();
        let text = if description.is_empty() {
            format!("[${{{}}}]", relation.2)
        } else {
            format!("${{{description} || ''}} [${{{}}}]", relation.2)
        };
        let node = target.append("display-entity");
        node.set_attribute("entity-name", relation.1.clone());
        node.set_attribute("text", text);
    };
    match field_type {
        "edit" => {
            if field_name == "lastUpdatedStamp" {
                if let Some(sub) = sub {
                    sub.append("hidden");
                }
                return;
            }
            header("true");
            let Some(sub) = sub else { return };
            if entity_type.starts_with("date") || entity_type.starts_with("time") {
                let node = sub.append("date-time");
                node.set_attribute("type", entity_type);
            } else if matches!(entity_type, "text-long" | "text-very-long" | "text-fts") {
                sub.append("text-area");
            } else if entity_type == "text-indicator" {
                let drop = sub.append("drop-down");
                drop.set_attribute("allow-empty", "true");
                drop.append("option").set_attribute("key", "Y");
                drop.append("option").set_attribute("key", "N");
            } else if entity_type == "binary-very-long" {
                sub.append("display");
            } else {
                let size = if entity_type.starts_with("number-") || entity_type.starts_with("currency-") {
                    "10"
                } else {
                    "30"
                };
                sub.append("text-line").set_attribute("size", size);
            }
        }
        "find" => {
            header("case-insensitive");
            let Some(sub) = sub else { return };
            if entity_type.starts_with("date") || entity_type.starts_with("time") {
                sub.append("date-find").set_attribute("type", entity_type);
            } else if entity_type.starts_with("number-") || entity_type.starts_with("currency-") {
                sub.append("range-find");
            } else {
                sub.append("text-find");
            }
        }
        "display" => {
            header("case-insensitive");
            let Some(sub) = sub else { return };
            match relation {
                Some(relation) => display_entity(sub, relation),
                None => display_with_format(sub, entity_type),
            }
        }
        "find-display" => {
            header("case-insensitive");
            let header_field = field
                .first_named("header-field")
                .unwrap_or_else(|| field.append("header-field"));
            if entity_type == "date" || entity_type == "time" {
                header_field.append("date-find").set_attribute("type", entity_type);
            } else if entity_type == "date-time" {
                header_field.append("date-period").set_attribute("time", "true");
            } else if entity_type.starts_with("number-") || entity_type.starts_with("currency-") {
                header_field.append("range-find").set_attribute("size", "10");
                field.set_attribute("align", "right");
                field.set_attribute("show-total", "sum");
            } else {
                let find = header_field.append("text-find");
                find.set_attribute("size", "30");
                find.set_attribute("default-operator", "begins");
                find.set_attribute("ignore-case", "false");
            }
            let Some(sub) = sub else { return };
            match relation {
                Some(relation) => display_entity(sub, relation),
                None => display_with_format(sub, entity_type),
            }
        }
        "hidden" => {
            if let Some(sub) = sub {
                sub.append("hidden");
            }
        }
        _ => {}
    }
}

fn display_with_format(sub: &MNode, entity_type: &str) {
    let node = sub.append("display");
    match entity_type {
        "currency-amount" => node.set_attribute("format", "#,##0.00"),
        "currency-precise" => node.set_attribute("format", "#,##0.000"),
        _ => {}
    }
}

/// `addAutoEntityColumns`: the displayed fields in columns, id and date fields first, then
/// numbers, short text, and long text, about six to a column.
fn add_auto_entity_columns(form: &MNode, initial: &MNode, shape: &EntityShape, displayed: &[String]) {
    if displayed.is_empty() {
        return;
    }
    let per_column = {
        let count = displayed.len();
        // BigDecimal ROUND_HALF_UP of count / 6.0
        (count * 2 + 6) / 12
    };
    let kind = |name: &str| {
        shape
            .fields
            .iter()
            .find(|(field, _, _)| field == name)
            .map_or("text-long", |(_, field_type, _)| field_type.as_str())
    };
    let mut groups: [Vec<&String>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for name in displayed {
        let field_type = kind(name);
        let group = if field_type.starts_with("id") || field_type.starts_with("date") || field_type == "time" {
            0
        } else if field_type.starts_with("number")
            || field_type.starts_with("currency")
            || field_type == "text-indicator"
        {
            1
        } else if field_type == "text-short" || field_type == "text-medium" {
            2
        } else {
            3
        };
        groups[group].push(name);
    }
    let sorted: Vec<&String> = groups.into_iter().flatten().collect();
    let columns = match form.first_named("columns") {
        Some(columns) => columns,
        None => {
            let columns = form.append("columns");
            for field in initial.children_named("field") {
                let name = field.attribute("name").unwrap_or_default();
                let mut subs = field.children_named("default-field");
                subs.extend(field.children_named("conditional-field"));
                let mut used = false;
                for sub in subs {
                    if used {
                        continue;
                    }
                    if sub.children().len() == 1
                        && (sub.first_named("hidden").is_some() || sub.first_named("ignored").is_some())
                    {
                        continue;
                    }
                    columns
                        .append("column")
                        .append("field-ref")
                        .set_attribute("name", name.clone());
                    used = true;
                }
            }
            columns
        }
    };
    let mut current: Vec<&String> = Vec::new();
    for (index, name) in sorted.iter().enumerate() {
        current.push(name);
        if current.len() == per_column || index + 1 == sorted.len() {
            let column = columns.append("column");
            for field in current.drain(..) {
                column.append("field-ref").set_attribute("name", field.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use moqui_lib::node::MNode;

    use super::{merge_field_node, merge_form_nodes};

    fn parse(xml: &str) -> MNode {
        MNode::parse_text("test", xml).unwrap()
    }

    #[test]
    fn an_override_form_replaces_attributes_the_entity_find_and_the_layout() {
        let base = parse(
            r#"<form-list name="A" list="one"><entity-find entity-name="X"/><row-actions><set field="a"/></row-actions>
               <field-layout><field-ref name="f"/></field-layout><field name="f"><default-field title="F"><display/></default-field></field></form-list>"#,
        );
        let over = parse(
            r#"<form-list name="A" list="two"><entity-find entity-name="Y"/><row-actions><set field="b"/></row-actions>
               <field-layout><field-ref name="g"/></field-layout>
               <field name="f"><default-field title="Changed"/></field><field name="g"><default-field><text-line/></default-field></field></form-list>"#,
        );
        merge_form_nodes(&base, &over, true, true);
        assert_eq!(base.attribute("list").as_deref(), Some("two"));
        let finds = base.children_named("entity-find");
        assert_eq!(finds.len(), 1);
        assert_eq!(finds[0].attribute("entity-name").as_deref(), Some("Y"));
        // row-actions of both run
        let actions: Vec<String> = base
            .first_named("row-actions")
            .unwrap()
            .children()
            .iter()
            .filter_map(|node| node.attribute("field"))
            .collect();
        assert_eq!(actions, ["a", "b"]);
        // the layout is the override's, with no field-ref added for a field it already names
        let layout = base.first_named("field-layout").unwrap();
        assert_eq!(
            layout
                .children()
                .iter()
                .filter_map(|node| node.attribute("name"))
                .collect::<Vec<_>>(),
            ["g"]
        );
        // the field of the same name is merged in place, the new one is appended
        let fields = base.children_named("field");
        assert_eq!(fields.len(), 2);
        let default = fields[0].first_named("default-field").unwrap();
        assert_eq!(default.attribute("title").as_deref(), Some("Changed"));
        assert!(
            default.first_named("display").is_some(),
            "a default-field without children keeps the base widgets"
        );
    }

    #[test]
    fn conditional_fields_merge_by_condition() {
        let base = parse(
            r#"<form-single name="A"><field name="f"><conditional-field condition="a"><display/></conditional-field>
               <default-field><display/></default-field></field></form-single>"#,
        );
        let over = parse(
            r#"<field name="f"><conditional-field condition="a" title="T"><text-line/></conditional-field>
               <conditional-field condition="b"><hidden/></conditional-field></field>"#,
        );
        merge_field_node(&base, &over, false);
        let field = base.first_named("field").unwrap();
        let conditions: Vec<String> = field
            .children_named("conditional-field")
            .iter()
            .filter_map(|node| node.attribute("condition"))
            .collect();
        assert_eq!(conditions, ["a", "b"]);
        let first = &field.children_named("conditional-field")[0];
        assert_eq!(first.attribute("title").as_deref(), Some("T"));
        assert!(first.first_named("text-line").is_some() && first.first_named("display").is_none());
    }
}
