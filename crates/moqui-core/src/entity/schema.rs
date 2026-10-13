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

//! JSON Schema for one entity, or for every entity when no name is given.

use indexmap::IndexMap;

use moqui_db::model::{EntityDefinition, EntityModel, MasterDefinition, MasterDetail};
use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::EntityFacade;

const SCHEMA: &str = "https://json-schema.org/draft/2020-12/schema";

/// JSON Schema for `names`. An empty list is every non-view entity, or every master when `master` is set.
///
/// A master request with no master name uses `default`.
pub(crate) fn document(facade: &EntityFacade, names: &[String], master: bool) -> Result<Value, Error> {
    facade.with_model(|model| {
        if names.is_empty() {
            return Ok(catalog(model, master));
        }
        one_entity(
            model,
            &names[0],
            master.then(|| names.get(1).cloned().unwrap_or_else(|| "default".to_string())),
        )
    })
}

fn one_entity(model: &EntityModel, name: &str, master_name: Option<String>) -> Result<Value, Error> {
    let entity = match model.entity(name) {
        Ok(entity) => entity,
        Err(err) if err.message().contains("not unique") => return Err(err),
        Err(_) => return Err(Error::new(format!("No entity found with name or alias {name}"))),
    };
    let master =
        match master_name.as_deref() {
            Some(master_name) => Some(entity.masters.iter().find(|item| item.name == master_name).ok_or_else(
                || {
                    Error::new(format!(
                        "Master name {master_name} not valid for entity {}",
                        entity.full_name
                    ))
                },
            )?),
            None => None,
        };
    Ok(standalone(entity, master))
}

fn catalog(model: &EntityModel, master: bool) -> Value {
    let mut entities: Vec<&EntityDefinition> = model.entities().iter().collect();
    entities.sort_by(|left, right| left.full_name.cmp(&right.full_name));
    let mut defs = IndexMap::new();
    for entity in entities {
        if master {
            for master_def in &entity.masters {
                defs.insert(
                    format!("{}.{}", entity.full_name, master_def.name),
                    object_schema(entity, Some(master_def)),
                );
            }
        } else if !entity.is_view {
            defs.insert(entity.full_name.clone(), object_schema(entity, None));
        }
    }
    let mut root = IndexMap::new();
    root.insert("$schema".to_string(), Value::string(SCHEMA));
    root.insert("title".to_string(), Value::string("Moqui Entity REST API"));
    root.insert("$defs".to_string(), Value::Map(defs));
    Value::Map(root)
}

fn standalone(entity: &EntityDefinition, master: Option<&MasterDefinition>) -> Value {
    let mut schema = object_map(entity, master);
    let (id, title) = match master {
        Some(master) => (
            format!("{}.{}", entity.full_name, master.name),
            format!("{} ({})", entity.entity_name, master.name),
        ),
        None => (entity.full_name.clone(), entity.entity_name.clone()),
    };
    schema.insert("$schema".to_string(), Value::string(SCHEMA));
    schema.insert("$id".to_string(), Value::string(id));
    schema.insert("title".to_string(), Value::string(title));
    Value::Map(schema)
}

fn object_schema(entity: &EntityDefinition, master: Option<&MasterDefinition>) -> Value {
    Value::Map(object_map(entity, master))
}

fn object_map(entity: &EntityDefinition, master: Option<&MasterDefinition>) -> IndexMap<String, Value> {
    let mut properties = IndexMap::new();
    for field in &entity.fields {
        properties.insert(field.name.clone(), field_schema(&field.field_type));
    }
    if let Some(master) = master {
        add_details(&master.details, &mut properties);
    }
    let required: Vec<Value> = entity.pk_field_names().into_iter().map(Value::string).collect();
    let mut map = IndexMap::new();
    map.insert("type".to_string(), Value::string("object"));
    map.insert("properties".to_string(), Value::Map(properties));
    if !required.is_empty() {
        map.insert("required".to_string(), Value::List(required));
    }
    map
}

fn add_details(details: &[MasterDetail], properties: &mut IndexMap<String, Value>) {
    for detail in details {
        if properties.contains_key(&detail.relationship) {
            continue;
        }
        let mut items = IndexMap::new();
        items.insert("type".to_string(), Value::string("object"));
        let mut array = IndexMap::new();
        array.insert("type".to_string(), Value::string("array"));
        array.insert("items".to_string(), Value::Map(items));
        properties.insert(detail.relationship.clone(), Value::Map(array));
    }
}

fn field_schema(field_type: &str) -> Value {
    let json_type = match field_type {
        "number-integer" | "number-float" | "number-decimal" | "currency-amount" | "currency-precise" => "number",
        _ => "string",
    };
    let mut map = IndexMap::new();
    map.insert("type".to_string(), Value::string(json_type));
    let format = match field_type {
        "date-time" => Some("date-time"),
        "date" => Some("date"),
        "number-integer" => Some("int64"),
        "number-float" => Some("double"),
        _ => None,
    };
    if let Some(format) = format {
        map.insert("format".to_string(), Value::string(format));
    }
    Value::Map(map)
}
