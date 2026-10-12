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

//! Insert, update, delete, select, count, view-entity select, and DDL text.

use std::collections::HashSet;

use moqui_lib::error::Error;

use crate::dialect::Dialect;
use crate::model::{EntityDefinition, EntityModel, FieldDefinition, IndexDefinition, Relationship};

use super::condition::{fts_config, render_condition, write_field, Condition, FieldRef, LimitOffset, Nulls, OrderBy};
use super::ident::{clip_name, is_mysql, is_postgres, placeholders, quote_ident, quote_list};

/// `INSERT INTO table (columns) VALUES (?, ?)`. The caller chooses the columns. None are skipped.
pub fn insert(entity: &EntityDefinition, dialect: &Dialect, columns: &[&str]) -> Result<String, Error> {
    if columns.is_empty() {
        return Err(Error::new("insert has no columns"));
    }
    let mut sql = String::new();
    sql.push_str("INSERT INTO ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    sql.push_str(" (");
    sql.push_str(&quote_list(dialect, columns));
    sql.push_str(") VALUES (");
    sql.push_str(&placeholders(dialect, columns.len()));
    sql.push(')');
    Ok(sql)
}

/// `UPDATE table SET col = ? WHERE ...`. An empty condition omits `WHERE`.
pub fn update(
    entity: &EntityDefinition,
    dialect: &Dialect,
    columns: &[&str],
    condition: Option<&Condition>,
) -> Result<String, Error> {
    if columns.is_empty() {
        return Err(Error::new("update has no columns"));
    }
    let mut sql = String::new();
    sql.push_str("UPDATE ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    sql.push_str(" SET ");
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&quote_ident(dialect, column));
        sql.push_str(" = ");
        sql.push_str(dialect.placeholder);
    }
    append_where(&mut sql, dialect, condition)?;
    Ok(sql)
}

/// `DELETE FROM table WHERE ...`. An empty condition omits `WHERE`.
pub fn delete(entity: &EntityDefinition, dialect: &Dialect, condition: Option<&Condition>) -> Result<String, Error> {
    let mut sql = String::new();
    sql.push_str("DELETE FROM ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    append_where(&mut sql, dialect, condition)?;
    Ok(sql)
}

/// `SELECT fields FROM table WHERE ... ORDER BY ...` plus limit and offset.
/// An empty `fields` list selects `*`. Schema is omitted.
pub fn select(
    entity: &EntityDefinition,
    dialect: &Dialect,
    fields: &[FieldRef],
    condition: Option<&Condition>,
    order_by: &[OrderBy],
    page: LimitOffset,
) -> Result<String, Error> {
    let mut sql = String::from("SELECT ");
    if fields.is_empty() {
        sql.push('*');
    } else {
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                sql.push_str(", ");
            }
            write_field(&mut sql, dialect, field);
        }
    }
    sql.push_str(" FROM ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    append_where(&mut sql, dialect, condition)?;
    append_order(&mut sql, dialect, order_by);
    append_page(&mut sql, dialect, page);
    Ok(sql)
}

/// `SELECT COUNT(*) FROM table WHERE ...`. No `ORDER BY` and no limit.
pub fn count(entity: &EntityDefinition, dialect: &Dialect, condition: Option<&Condition>) -> Result<String, Error> {
    let mut sql = String::from("SELECT COUNT(*) FROM ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    append_where(&mut sql, dialect, condition)?;
    Ok(sql)
}

/// View-entity select. Members are joined in document order, including `member-relationship`.
///
/// The first member is the driving table. A later member with `join_optional` is `LEFT JOIN`,
/// otherwise `JOIN`. A `member-relationship` uses that relationship's key map. An empty
/// `member-entity` key map is `ON 1=1`. A relationship with no key map joins the related
/// primary key, as `EntityDefinition.groovy` does.
/// Selected columns are the explicit aliases (`alias.field`, or `alias.name` when `field` is
/// absent) followed by each `alias_all` expanded to that member's fields except `exclude`.
/// A prefix is prepended, with the field's first letter uppercased, and written as `AS`.
pub fn select_view(
    view: &EntityDefinition,
    model: &EntityModel,
    dialect: &Dialect,
    condition: Option<&Condition>,
    order_by: &[OrderBy],
    page: LimitOffset,
) -> Result<String, Error> {
    let mut sql = String::from("SELECT ");
    sql.push_str(&view_select_list(view, model, dialect)?);
    sql.push_str(&view_from(view, model, dialect)?);
    append_where(&mut sql, dialect, condition)?;
    append_order(&mut sql, dialect, order_by);
    append_page(&mut sql, dialect, page);
    Ok(sql)
}

/// Insert, then update on a key conflict.
///
/// When `never_try_insert` is set, this returns an error that says try-insert is off.
/// The caller uses create-or-update instead. Postgres writes `ON CONFLICT (pk) DO UPDATE SET`.
/// MySQL writes `ON DUPLICATE KEY UPDATE`.
pub fn insert_or_update(entity: &EntityDefinition, dialect: &Dialect, columns: &[&str]) -> Result<String, Error> {
    if dialect.never_try_insert {
        return Err(Error::new(format!("try-insert is off for database {}", dialect.name)));
    }
    if columns.is_empty() {
        return Err(Error::new("insert_or_update has no columns"));
    }
    let mut sql = insert(entity, dialect, columns)?;
    let pk = entity.pk_field_names();
    let pk_owned: HashSet<&str> = pk.iter().copied().collect();
    let updates: Vec<&str> = columns
        .iter()
        .copied()
        .filter(|column| !pk_owned.contains(column))
        .collect();
    if is_mysql(dialect) {
        sql.push_str(" ON DUPLICATE KEY UPDATE ");
        let targets: Vec<&str> = if updates.is_empty() { vec![columns[0]] } else { updates };
        for (index, column) in targets.iter().enumerate() {
            if index > 0 {
                sql.push_str(", ");
            }
            let quoted = quote_ident(dialect, column);
            sql.push_str(&quoted);
            sql.push_str(" = VALUES(");
            sql.push_str(&quoted);
            sql.push(')');
        }
        return Ok(sql);
    }
    if pk.is_empty() {
        return Err(Error::new(format!(
            "insert_or_update needs a primary key on {}",
            entity.full_name
        )));
    }
    sql.push_str(" ON CONFLICT (");
    sql.push_str(&quote_list(dialect, &pk));
    sql.push(')');
    if updates.is_empty() {
        sql.push_str(" DO NOTHING");
        return Ok(sql);
    }
    sql.push_str(" DO UPDATE SET ");
    for (index, column) in updates.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        let quoted = quote_ident(dialect, column);
        sql.push_str(&quoted);
        sql.push_str(" = EXCLUDED.");
        sql.push_str(&quoted);
    }
    Ok(sql)
}

/// `CREATE TABLE`, plus one `CREATE INDEX ... USING gin (to_tsvector(...))` per `text-fts`
/// column when the profile is `postgres-tsvector`. MySQL puts `FULLTEXT` on the table and
/// appends `ENGINE`, character set, and collate when the profile sets them.
pub fn create_table(entity: &EntityDefinition, model: &EntityModel, dialect: &Dialect) -> Result<String, Error> {
    if entity.is_view {
        return Err(Error::new(format!(
            "cannot create a table for view-entity {}",
            entity.full_name
        )));
    }
    if entity.fields.is_empty() {
        return Err(Error::new(format!("entity {} has no fields", entity.full_name)));
    }
    let mut sql = String::from("CREATE TABLE ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    sql.push_str(" (");
    for (index, field) in entity.fields.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        write_column(&mut sql, model, dialect, field)?;
    }
    let pk = entity.pk_field_names();
    if !pk.is_empty() {
        sql.push_str(", CONSTRAINT ");
        let pk_name = clip_name(
            &format!("PK_{}", entity.table_name),
            dialect.constraint_name_clip_length,
        );
        sql.push_str(&quote_ident(dialect, &pk_name));
        sql.push_str(" PRIMARY KEY (");
        sql.push_str(&quote_list(dialect, &pk));
        sql.push(')');
    }
    if is_mysql(dialect) && dialect.fts_style.as_deref() == Some("mysql-fulltext") {
        for field in entity.fields.iter().filter(|field| field.field_type == "text-fts") {
            sql.push_str(", FULLTEXT INDEX ");
            sql.push_str(&quote_ident(dialect, &fts_index_name(entity, &field.name, dialect)));
            sql.push_str(" (");
            sql.push_str(&quote_ident(dialect, &field.name));
            sql.push(')');
        }
    }
    sql.push(')');
    if let Some(engine) = dialect.table_engine.as_deref() {
        sql.push_str(" ENGINE=");
        sql.push_str(engine);
    }
    if let Some(character_set) = dialect.character_set.as_deref() {
        sql.push_str(" CHARACTER SET ");
        sql.push_str(character_set);
    }
    if let Some(collate) = dialect.collate.as_deref() {
        sql.push_str(" COLLATE ");
        sql.push_str(collate);
    }
    if dialect.fts_style.as_deref() == Some("postgres-tsvector") {
        let config = fts_config(dialect)?;
        for field in entity.fields.iter().filter(|field| field.field_type == "text-fts") {
            sql.push_str(";\nCREATE INDEX ");
            sql.push_str(&quote_ident(dialect, &fts_index_name(entity, &field.name, dialect)));
            sql.push_str(" ON ");
            sql.push_str(&quote_ident(dialect, &entity.table_name));
            sql.push_str(" USING gin (to_tsvector('");
            sql.push_str(&config);
            sql.push_str("', ");
            sql.push_str(&quote_ident(dialect, &field.name));
            sql.push_str("))");
        }
    }
    Ok(sql)
}

/// `CREATE [UNIQUE] INDEX name ON table (fields)`.
pub fn create_index(entity: &EntityDefinition, index: &IndexDefinition, dialect: &Dialect) -> Result<String, Error> {
    if index.fields.is_empty() {
        return Err(Error::new(format!("index {} has no fields", index.name)));
    }
    let mut sql = String::from("CREATE ");
    if index.unique {
        sql.push_str("UNIQUE ");
    }
    sql.push_str("INDEX ");
    sql.push_str(&quote_ident(
        dialect,
        &clip_name(&index.name, dialect.constraint_name_clip_length),
    ));
    sql.push_str(" ON ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    sql.push_str(" (");
    sql.push_str(&quote_list(dialect, &index.fields));
    sql.push(')');
    Ok(sql)
}

/// `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY ... REFERENCES ...`.
///
/// Only a `type=one` relationship that is not `is-auto-reverse` and not `one-nofk`.
/// An empty `key_map` uses the related entity's primary keys under the same names.
/// The constraint name is clipped to `constraint_name_clip_length` when that is set.
pub fn create_foreign_key(
    entity: &EntityDefinition,
    relationship: &Relationship,
    related: &EntityDefinition,
    dialect: &Dialect,
) -> Result<String, Error> {
    if relationship.is_auto_reverse || relationship.relationship_type != "one" {
        return Err(Error::new(format!(
            "foreign key requires a type=one relationship that is not auto-reverse, found {} on {}",
            relationship.relationship_type, entity.full_name
        )));
    }
    let pairs = foreign_key_pairs(relationship, related)?;
    let mut sql = String::from("ALTER TABLE ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    sql.push_str(" ADD CONSTRAINT ");
    sql.push_str(&quote_ident(dialect, &foreign_key_name(entity, relationship, dialect)));
    sql.push_str(" FOREIGN KEY (");
    let sources: Vec<&str> = pairs.iter().map(|(source, _)| source.as_str()).collect();
    let targets: Vec<&str> = pairs.iter().map(|(_, target)| target.as_str()).collect();
    sql.push_str(&quote_list(dialect, &sources));
    sql.push_str(") REFERENCES ");
    sql.push_str(&quote_ident(dialect, &related.table_name));
    sql.push_str(" (");
    sql.push_str(&quote_list(dialect, &targets));
    sql.push(')');
    Ok(sql)
}

/// `ALTER TABLE ... ADD column type`.
pub fn add_column(
    entity: &EntityDefinition,
    field: &FieldDefinition,
    model: &EntityModel,
    dialect: &Dialect,
) -> Result<String, Error> {
    if entity.is_view {
        return Err(Error::new(format!(
            "cannot add a column on view-entity {}",
            entity.full_name
        )));
    }
    let mut sql = String::from("ALTER TABLE ");
    sql.push_str(&quote_ident(dialect, &entity.table_name));
    sql.push_str(" ADD ");
    write_column(&mut sql, model, dialect, field)?;
    Ok(sql)
}

fn append_where(sql: &mut String, dialect: &Dialect, condition: Option<&Condition>) -> Result<(), Error> {
    let Some(condition) = condition else {
        return Ok(());
    };
    let rendered = render_condition(condition, dialect)?;
    if rendered.is_empty() {
        return Ok(());
    }
    sql.push_str(" WHERE ");
    sql.push_str(&rendered);
    Ok(())
}

fn append_order(sql: &mut String, dialect: &Dialect, order_by: &[OrderBy]) {
    if order_by.is_empty() {
        return;
    }
    sql.push_str(" ORDER BY ");
    let write_nulls = is_postgres(dialect) && !dialect.never_nulls;
    for (index, entry) in order_by.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        write_field(sql, dialect, &entry.field);
        sql.push_str(if entry.descending { " DESC" } else { " ASC" });
        if write_nulls {
            match entry.nulls {
                Some(Nulls::First) => sql.push_str(" NULLS FIRST"),
                Some(Nulls::Last) => sql.push_str(" NULLS LAST"),
                None => {}
            }
        }
    }
}

fn append_page(sql: &mut String, dialect: &Dialect, page: LimitOffset) {
    let ph = dialect.placeholder;
    let limit_style = dialect.offset_style.as_deref() == Some("limit");
    match (limit_style, page.limit.is_some(), page.offset.is_some()) {
        (false, true, _) => {
            sql.push_str(" LIMIT ");
            sql.push_str(ph);
            sql.push_str(" OFFSET ");
            sql.push_str(ph);
        }
        (false, false, true) => {
            sql.push_str(" OFFSET ");
            sql.push_str(ph);
        }
        (true, true, false) => {
            sql.push_str(" LIMIT ");
            sql.push_str(ph);
        }
        (true, _, true) => {
            sql.push_str(" LIMIT ");
            sql.push_str(ph);
            sql.push_str(", ");
            sql.push_str(ph);
        }
        (_, false, false) => {}
    }
}

fn view_select_list(view: &EntityDefinition, model: &EntityModel, dialect: &Dialect) -> Result<String, Error> {
    let mut parts = Vec::new();
    let mut used = HashSet::new();
    for alias in &view.aliases {
        let source = alias.field.as_deref().unwrap_or(alias.name.as_str());
        let mut expr = String::new();
        if let Some(entity_alias) = alias.entity_alias.as_deref() {
            expr.push_str(&quote_ident(dialect, entity_alias));
            expr.push('.');
        }
        expr.push_str(&quote_ident(dialect, source));
        if alias.field.as_deref().is_some_and(|field| field != alias.name) {
            expr.push_str(" AS ");
            expr.push_str(&quote_ident(dialect, &alias.name));
        }
        used.insert(alias.name.clone());
        parts.push(expr);
    }
    for alias_all in &view.alias_alls {
        let member = view.member(&alias_all.entity_alias).ok_or_else(|| {
            Error::new(format!(
                "view {} alias-all {} has no member-entity",
                view.full_name, alias_all.entity_alias
            ))
        })?;
        let member_entity = model.entity(&member.entity_name)?;
        let excluded: HashSet<&str> = alias_all.exclude.iter().map(String::as_str).collect();
        for field in &member_entity.fields {
            if excluded.contains(field.name.as_str()) {
                continue;
            }
            let out_name = match alias_all.prefix.as_deref() {
                Some(prefix) => prefixed_name(prefix, &field.name),
                None => field.name.clone(),
            };
            if !used.insert(out_name.clone()) {
                continue;
            }
            let mut expr = String::new();
            expr.push_str(&quote_ident(dialect, &alias_all.entity_alias));
            expr.push('.');
            expr.push_str(&quote_ident(dialect, &field.name));
            if alias_all.prefix.is_some() {
                expr.push_str(" AS ");
                expr.push_str(&quote_ident(dialect, &out_name));
            }
            parts.push(expr);
        }
    }
    if parts.is_empty() {
        Ok("*".to_string())
    } else {
        Ok(parts.join(", "))
    }
}

fn prefixed_name(prefix: &str, field: &str) -> String {
    let mut name = String::with_capacity(prefix.len() + field.len());
    name.push_str(prefix);
    let mut chars = field.chars();
    if let Some(first) = chars.next() {
        name.push(first.to_ascii_uppercase());
        name.push_str(chars.as_str());
    }
    name
}

fn view_from(view: &EntityDefinition, model: &EntityModel, dialect: &Dialect) -> Result<String, Error> {
    let parts = view_parts(view);
    if parts.is_empty() {
        return Ok(String::new());
    }
    let mut sql = String::from(" FROM ");
    for (index, part) in parts.iter().enumerate() {
        let (table_entity, alias_name, optional, keys, join_from) = match part {
            ViewPart::Entity(member) => (
                model.entity(&member.entity_name)?,
                member.entity_alias.as_str(),
                member.join_optional,
                member_keys(member),
                member.join_from_alias.clone(),
            ),
            ViewPart::Relationship(member) => {
                let from = alias_entity(view, model, &member.join_from_alias)?;
                let relationship = from.relationship(&member.relationship).ok_or_else(|| {
                    Error::new(format!(
                        "view {} member {} relationship {} was not found on {}",
                        view.full_name, member.entity_alias, member.relationship, from.full_name
                    ))
                })?;
                let related = model.entity(&relationship.related)?;
                (
                    related,
                    member.entity_alias.as_str(),
                    member.join_optional,
                    relationship_keys(relationship, related),
                    Some(member.join_from_alias.clone()),
                )
            }
        };
        let table = quote_ident(dialect, &table_entity.table_name);
        let alias = quote_ident(dialect, alias_name);
        if index == 0 {
            sql.push_str(&table);
            sql.push(' ');
            sql.push_str(&alias);
            continue;
        }
        if optional {
            sql.push_str(" LEFT JOIN ");
        } else {
            sql.push_str(" JOIN ");
        }
        sql.push_str(&table);
        sql.push(' ');
        sql.push_str(&alias);
        sql.push_str(" ON ");
        if keys.is_empty() {
            sql.push_str("1=1");
            continue;
        }
        let fallback = match parts.first() {
            Some(ViewPart::Entity(member)) => member.entity_alias.as_str(),
            Some(ViewPart::Relationship(member)) => member.entity_alias.as_str(),
            None => alias_name,
        };
        let left_alias = join_from.as_deref().unwrap_or(fallback);
        let left = quote_ident(dialect, left_alias);
        for (key_index, (field_name, related)) in keys.iter().enumerate() {
            if key_index > 0 {
                sql.push_str(" AND ");
            }
            sql.push_str(&left);
            sql.push('.');
            sql.push_str(&quote_ident(dialect, field_name));
            sql.push_str(" = ");
            sql.push_str(&alias);
            sql.push('.');
            sql.push_str(&quote_ident(dialect, related));
        }
    }
    Ok(sql)
}

enum ViewPart<'a> {
    Entity(&'a crate::model::MemberEntity),
    Relationship(&'a crate::model::MemberRelationship),
}

fn view_parts(view: &EntityDefinition) -> Vec<ViewPart<'_>> {
    if !view.view_members.is_empty() {
        return view
            .view_members
            .iter()
            .map(|member| match member {
                crate::model::ViewMember::Entity(member) => ViewPart::Entity(member),
                crate::model::ViewMember::Relationship(member) => ViewPart::Relationship(member),
            })
            .collect();
    }
    view.member_entities.iter().map(ViewPart::Entity).collect()
}

fn member_keys(member: &crate::model::MemberEntity) -> Vec<(String, String)> {
    member
        .key_map
        .iter()
        .map(|key| {
            let related = key
                .related
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| key.field_name.clone());
            (key.field_name.clone(), related)
        })
        .collect()
}

fn relationship_keys(relationship: &crate::model::Relationship, related: &EntityDefinition) -> Vec<(String, String)> {
    if relationship.key_map.is_empty() {
        return related
            .pk_field_names()
            .into_iter()
            .map(|pk| (pk.to_string(), pk.to_string()))
            .collect();
    }
    let pks = related.pk_field_names();
    relationship
        .key_map
        .iter()
        .map(|key| {
            let related_field = match key.related.as_deref().filter(|value| !value.is_empty()) {
                Some(name) => name.to_string(),
                None if relationship.relationship_type.starts_with("one") && pks.len() == 1 => pks[0].to_string(),
                None => key.field_name.clone(),
            };
            (key.field_name.clone(), related_field)
        })
        .collect()
}

fn alias_entity<'a>(
    view: &'a EntityDefinition,
    model: &'a EntityModel,
    alias: &str,
) -> Result<&'a EntityDefinition, Error> {
    alias_entity_at(view, model, alias, 0)
}

/// A `member-relationship` chain longer than the member count has to loop.
fn alias_entity_at<'a>(
    view: &'a EntityDefinition,
    model: &'a EntityModel,
    alias: &str,
    depth: usize,
) -> Result<&'a EntityDefinition, Error> {
    if depth > view.member_relationships.len() {
        return Err(Error::new(format!(
            "view {} member alias {alias} joins from itself",
            view.full_name
        )));
    }
    for member in &view.member_entities {
        if member.entity_alias == alias {
            return model.entity(&member.entity_name);
        }
    }
    for member in &view.member_relationships {
        if member.entity_alias != alias {
            continue;
        }
        let from = alias_entity_at(view, model, &member.join_from_alias, depth + 1)?;
        let relationship = from.relationship(&member.relationship).ok_or_else(|| {
            Error::new(format!(
                "view {} alias {} relationship {} was not found",
                view.full_name, alias, member.relationship
            ))
        })?;
        return model.entity(&relationship.related);
    }
    Err(Error::new(format!(
        "view {} has no member alias {alias}",
        view.full_name
    )))
}

fn write_column(
    sql: &mut String,
    model: &EntityModel,
    dialect: &Dialect,
    field: &FieldDefinition,
) -> Result<(), Error> {
    let (sql_type, java_type) = column_type(model, dialect, field)?;
    sql.push_str(&quote_ident(dialect, &field.name));
    sql.push(' ');
    sql.push_str(&sql_type);
    if java_type.ends_with("String") {
        if let Some(character_set) = dialect.character_set.as_deref() {
            sql.push_str(" CHARACTER SET ");
            sql.push_str(character_set);
        }
        if let Some(collate) = dialect.collate.as_deref() {
            sql.push_str(" COLLATE ");
            sql.push_str(collate);
        }
    }
    if field.not_null {
        sql.push_str(" NOT NULL");
    }
    Ok(())
}

fn column_type(model: &EntityModel, dialect: &Dialect, field: &FieldDefinition) -> Result<(String, String), Error> {
    let dictionary = model.dictionary_type(&field.field_type).ok_or_else(|| {
        Error::new(format!(
            "no dictionary type {} for field {}",
            field.field_type, field.name
        ))
    })?;
    let sql_type = dialect
        .sql_type(&field.field_type)
        .unwrap_or(dictionary.default_sql_type.as_str())
        .to_string();
    Ok((sql_type, dictionary.java_type.clone()))
}

fn fts_index_name(entity: &EntityDefinition, field: &str, dialect: &Dialect) -> String {
    clip_name(
        &format!("{}_{}_FTS", entity.table_name, field),
        dialect.constraint_name_clip_length,
    )
}

fn foreign_key_name(entity: &EntityDefinition, relationship: &Relationship, dialect: &Dialect) -> String {
    let mut name = entity.entity_name.clone();
    if let Some(title) = relationship.title.as_deref() {
        name.push_str(title);
    }
    name.push_str(&relationship.related_entity_name);
    clip_name(&name, dialect.constraint_name_clip_length)
}

fn foreign_key_pairs(relationship: &Relationship, related: &EntityDefinition) -> Result<Vec<(String, String)>, Error> {
    if relationship.key_map.is_empty() {
        let pairs: Vec<(String, String)> = related
            .pk_field_names()
            .into_iter()
            .map(|name| (name.to_string(), name.to_string()))
            .collect();
        if pairs.is_empty() {
            return Err(Error::new(format!(
                "relationship {} has no key-map and {} has no primary key",
                relationship.name, related.full_name
            )));
        }
        return Ok(pairs);
    }
    let mut pairs = Vec::with_capacity(relationship.key_map.len());
    let related_fields: HashSet<&str> = related.fields.iter().map(|field| field.name.as_str()).collect();
    let related_pk = related.pk_field_names();
    for key in &relationship.key_map {
        let mut target = key
            .related
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| key.field_name.clone());
        let related_missing = key.related.as_deref().unwrap_or("").is_empty();
        let use_single_pk = related_pk.len() == 1
            && ((relationship.key_map.len() == 1 && related_missing) || !related_fields.contains(target.as_str()));
        if use_single_pk {
            target = related_pk[0].to_string();
        }
        pairs.push((key.field_name.clone(), target));
    }
    Ok(pairs)
}
