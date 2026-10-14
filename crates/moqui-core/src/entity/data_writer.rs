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

//! `EntityDataWriterImpl`: write the records of a set of entities as entity XML, JSON, or CSV, to text, one file, a
//! directory with a file per entity, or a zip of either. The record shapes are those of the entity REST writer in
//! `query_rest`. A file that is there already is a message error and writes nothing, as in Java.
//!
//! Each entity is read as one list, not through a cursor. The zip entries are stored, not deflated.

use std::collections::BTreeMap;
use std::path::Path;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::facade::EntityFacade;
use super::query_rest::{csv_cell, data_body};
use crate::context::ExecutionContext;
use moqui_db::sql::Condition;

/// The output format, `EntityDataWriter.FileType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataFileType {
    Xml,
    Json,
    Csv,
}

impl DataFileType {
    /// `FileType.valueOf`: XML, JSON, or CSV, in any case.
    ///
    /// # Errors
    ///
    /// Returns an error for any other name.
    pub fn parse(text: &str) -> Result<Self, Error> {
        match text.trim().to_ascii_uppercase().as_str() {
            "XML" => Ok(Self::Xml),
            "JSON" => Ok(Self::Json),
            "CSV" => Ok(Self::Csv),
            other => Err(Error::new(format!("file type {other} is not XML, JSON, or CSV"))),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Xml => "xml",
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        if name.ends_with(".json") {
            Some(Self::Json)
        } else if name.ends_with(".xml") {
            Some(Self::Xml)
        } else if name.ends_with(".csv") {
            Some(Self::Csv)
        } else {
            None
        }
    }
}

/// A data writer: the Java builder's settings, then one of the write methods.
#[derive(Clone)]
pub struct EntityDataWriter {
    facade: EntityFacade,
    file_type: DataFileType,
    entity_names: Vec<String>,
    skip_entity_names: Vec<String>,
    dependent_levels: i64,
    master: Option<String>,
    prefix: String,
    filter_map: BTreeMap<String, Value>,
    order_by: Vec<String>,
    from_date: Option<Value>,
    thru_date: Option<Value>,
    iso_date_time: bool,
    table_column_names: bool,
}

impl EntityDataWriter {
    pub fn new(facade: &EntityFacade) -> Self {
        Self {
            facade: facade.clone(),
            file_type: DataFileType::Xml,
            entity_names: Vec::new(),
            skip_entity_names: Vec::new(),
            dependent_levels: 0,
            master: None,
            prefix: String::new(),
            filter_map: BTreeMap::new(),
            order_by: Vec::new(),
            from_date: None,
            thru_date: None,
            iso_date_time: false,
            table_column_names: false,
        }
    }

    pub fn file_type(&mut self, file_type: DataFileType) -> &mut Self {
        self.file_type = file_type;
        self
    }

    pub fn entity_name(&mut self, name: &str) -> &mut Self {
        if !self.entity_names.iter().any(|known| known == name) {
            self.entity_names.push(name.to_string());
        }
        self
    }

    pub fn skip_entity_name(&mut self, name: &str) -> &mut Self {
        if !self.skip_entity_names.iter().any(|known| known == name) {
            self.skip_entity_names.push(name.to_string());
        }
        self
    }

    /// Every entity that is not a view, in model order, without the names added before (Java's `allEntities`).
    pub fn all_entities(&mut self) -> &mut Self {
        let added = std::mem::take(&mut self.entity_names);
        self.entity_names = self.facade.with_model(|model| {
            model
                .entities()
                .iter()
                .filter(|entity| !entity.is_view && !added.contains(&entity.full_name))
                .map(|entity| entity.full_name.clone())
                .collect()
        });
        self
    }

    pub fn dependent_levels(&mut self, levels: i64) -> &mut Self {
        self.dependent_levels = levels;
        self
    }

    pub fn master(&mut self, name: &str) -> &mut Self {
        self.master = Some(name.to_string()).filter(|name| !name.is_empty());
        self
    }

    pub fn prefix(&mut self, prefix: &str) -> &mut Self {
        prefix.clone_into(&mut self.prefix);
        self
    }

    pub fn filter_map(&mut self, map: BTreeMap<String, Value>) -> &mut Self {
        self.filter_map.extend(map);
        self
    }

    pub fn order_by(&mut self, field: &str) -> &mut Self {
        self.order_by.push(field.to_string());
        self
    }

    pub fn from_date(&mut self, stamp: Value) -> &mut Self {
        self.from_date = Some(stamp).filter(|value| !value.is_null());
        self
    }

    pub fn thru_date(&mut self, stamp: Value) -> &mut Self {
        self.thru_date = Some(stamp).filter(|value| !value.is_null());
        self
    }

    pub fn iso_date_time(&mut self, iso: bool) -> &mut Self {
        self.iso_date_time = iso;
        self
    }

    pub fn table_column_names(&mut self, table_columns: bool) -> &mut Self {
        self.table_column_names = table_columns;
        self
    }

    fn active_names(&self) -> Vec<String> {
        self.entity_names
            .iter()
            .filter(|name| !self.skip_entity_names.contains(name))
            .cloned()
            .collect()
    }

    /// `writer`: every record in one document. Returns the text and how many records it has.
    ///
    /// # Errors
    ///
    /// Returns an error when an entity is not defined, a find fails, or CSV is asked for more than one entity.
    pub fn text(&self) -> Result<(String, u64), Error> {
        let names = self.active_names();
        if self.file_type == DataFileType::Csv && names.len() != 1 {
            return Err(Error::new("Cannot write to single CSV file with multiple entity names"));
        }
        let mut out = start_file(self.file_type);
        let mut written = 0;
        for name in &names {
            let (body, count) = self.entity_text(name)?;
            if self.file_type == DataFileType::Csv {
                out.push_str(&self.csv_header(name)?);
            }
            out.push_str(&body);
            written += count;
        }
        out.push_str(end_file(self.file_type));
        Ok((out, written))
    }

    /// `file`: one file. The extension of the name sets the type.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written or the records cannot be read.
    pub fn file(&mut self, filename: &str) -> Result<u64, Error> {
        if Path::new(filename).exists() {
            add_error(format!("File {filename} already exists."));
            return Ok(0);
        }
        if let Some(kind) = DataFileType::from_name(filename) {
            self.file_type = kind;
        }
        if self.file_type == DataFileType::Csv && self.active_names().len() > 1 {
            add_error("Cannot write to single CSV file with multiple entity names");
            return Ok(0);
        }
        let (text, written) = self.text()?;
        write_new(Path::new(filename), text.as_bytes())?;
        add_message(format!("Wrote {written} records to file {filename}"));
        Ok(written)
    }

    /// `zipFile`: one file inside a new zip. The extension of the name in the zip sets the type.
    ///
    /// # Errors
    ///
    /// Returns an error when the zip cannot be written or the records cannot be read.
    pub fn zip_file(&mut self, filename_within_zip: &str, zip_filename: &str) -> Result<u64, Error> {
        if Path::new(zip_filename).exists() {
            add_error(format!("File {zip_filename} already exists."));
            return Ok(0);
        }
        if let Some(kind) = DataFileType::from_name(filename_within_zip) {
            self.file_type = kind;
        }
        if self.file_type == DataFileType::Csv && self.active_names().len() > 1 {
            add_error("Cannot write to single CSV file with multiple entity names");
            return Ok(0);
        }
        let (text, written) = self.text()?;
        let zip = stored_zip(&[(filename_within_zip.to_string(), text.into_bytes())]);
        write_new(Path::new(zip_filename), &zip)?;
        add_message(format!("Wrote {written} records to file {zip_filename}"));
        Ok(written)
    }

    /// `directory`: a file per entity that has records, named for the entity. A file that is there already is
    /// skipped with a message error.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be written or the records cannot be read.
    pub fn directory(&self, path: &str) -> Result<u64, Error> {
        let dir = Path::new(path);
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|err| Error::new(format!("could not create {path}")).with_source(err))?;
        }
        if !dir.is_dir() {
            add_error(format!("Path {path} is not a directory."));
            return Ok(0);
        }
        let mut written = 0;
        for (name, text, count) in self.entity_files()? {
            let filename = format!("{path}/{name}.{}", self.file_type.extension());
            if Path::new(&filename).exists() {
                add_error(format!("File {filename} already exists, skipping entity {name}."));
                continue;
            }
            write_new(Path::new(&filename), text.as_bytes())?;
            add_message(format!("Wrote {count} records to file {filename}"));
            written += count;
        }
        Ok(written)
    }

    /// `zipDirectory`: a new zip with an entry per entity that has records, under `path_within_zip`.
    ///
    /// # Errors
    ///
    /// Returns an error when the zip cannot be written or the records cannot be read.
    pub fn zip_directory(&self, path_within_zip: &str, zip_filename: &str) -> Result<u64, Error> {
        if Path::new(zip_filename).exists() {
            add_error(format!("File {zip_filename} already exists."));
            return Ok(0);
        }
        let mut entries = Vec::new();
        let mut written = 0;
        for (name, text, count) in self.entity_files()? {
            let base = if self.table_column_names {
                self.facade.with_entity(&name, |entity, _, _| Ok(entity.table_name.clone()))?
            } else {
                name
            };
            let within = if path_within_zip.is_empty() {
                format!("{base}.{}", self.file_type.extension())
            } else {
                format!("{path_within_zip}/{base}.{}", self.file_type.extension())
            };
            add_message(format!("Wrote {count} records to {within}"));
            entries.push((within, text.into_bytes()));
            written += count;
        }
        write_new(Path::new(zip_filename), &stored_zip(&entries))?;
        Ok(written)
    }

    /// Each entity with records as a whole file: its name, the text, and the record count.
    fn entity_files(&self) -> Result<Vec<(String, String, u64)>, Error> {
        let mut files = Vec::new();
        for name in self.active_names() {
            let (body, count) = self.entity_text(&name)?;
            if count == 0 {
                continue;
            }
            let mut text = start_file(self.file_type);
            if self.file_type == DataFileType::Csv {
                text.push_str(&self.csv_header(&name)?);
            }
            text.push_str(&body);
            text.push_str(end_file(self.file_type));
            files.push((name, text, count));
        }
        Ok(files)
    }

    /// The records of one entity, as `makeEntityFind`: the filter map, the order, and the update stamp range.
    fn entity_text(&self, name: &str) -> Result<(String, u64), Error> {
        let (has_stamp, full_name, masters) = self.facade.with_entity(name, |entity, _, _| {
            Ok((
                entity.has_field("lastUpdatedStamp"),
                entity.full_name.clone(),
                entity.masters.iter().map(|master| master.name.clone()).collect::<Vec<_>>(),
            ))
        })?;
        let mut find = self.facade.find(&full_name)?;
        for (field, value) in &self.filter_map {
            find = find.condition(Condition::equals(field.as_str(), value.clone()));
        }
        for entry in &self.order_by {
            find = find.order_by_text(entry);
        }
        if has_stamp {
            if let Some(from) = &self.from_date {
                find = find.condition(Condition::greater_equal("lastUpdatedStamp", from.clone()));
            }
            if let Some(thru) = &self.thru_date {
                find = find.condition(Condition::less("lastUpdatedStamp", thru.clone()));
            }
        }
        let rows: Vec<BTreeMap<String, Value>> = find.list()?.values().iter().map(|value| value.fields().clone()).collect();
        let count = u64::try_from(rows.len()).unwrap_or(u64::MAX);
        if self.file_type == DataFileType::Csv {
            return Ok((self.csv_rows(&full_name, &rows)?, count));
        }
        let master = self.master.as_deref().filter(|master| masters.iter().any(|known| known == master));
        let body = data_body(
            &self.facade,
            &full_name,
            &rows,
            self.file_type == DataFileType::Json,
            self.dependent_levels,
            master,
            &self.prefix,
            self.iso_date_time,
        )?;
        Ok((body, count))
    }

    /// The two CSV header lines: the entity (or table) name, `export`, and the primary key fields; then every field.
    fn csv_header(&self, name: &str) -> Result<String, Error> {
        let table_columns = self.table_column_names;
        self.facade.with_entity(name, |entity, _, _| {
            let label = |field: &moqui_db::model::FieldDefinition| {
                if table_columns {
                    field.column_name.clone()
                } else {
                    field.name.clone()
                }
            };
            let mut out = if table_columns {
                entity.table_name.clone()
            } else {
                entity.full_name.clone()
            };
            out.push_str(",export");
            for field in entity.fields.iter().filter(|field| field.is_pk) {
                out.push(',');
                out.push_str(&label(field));
            }
            out.push('\n');
            out.push_str(&entity.fields.iter().map(label).collect::<Vec<_>>().join(","));
            out.push('\n');
            Ok(out)
        })
    }

    fn csv_rows(&self, name: &str, rows: &[BTreeMap<String, Value>]) -> Result<String, Error> {
        let iso = self.iso_date_time;
        self.facade.with_entity(name, |entity, _, _| {
            let mut out = String::new();
            for row in rows {
                for (index, field) in entity.fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    csv_cell(&mut out, row.get(&field.name), iso);
                }
                out.push('\n');
            }
            Ok(out)
        })
    }
}

fn start_file(kind: DataFileType) -> String {
    match kind {
        DataFileType::Json => "[\n".to_string(),
        DataFileType::Xml => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<entity-facade-xml>\n".to_string(),
        DataFileType::Csv => String::new(),
    }
}

fn end_file(kind: DataFileType) -> &'static str {
    match kind {
        DataFileType::Json => "]\n\n",
        DataFileType::Xml => "</entity-facade-xml>\n\n",
        DataFileType::Csv => "",
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|err| Error::new(format!("could not create {}", parent.display())).with_source(err))?;
    }
    std::fs::write(path, bytes).map_err(|err| Error::new(format!("could not write {}", path.display())).with_source(err))
}

fn add_message(text: String) {
    ExecutionContext::with_current(|ctx| ctx.message().add_message(text));
}

fn add_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

/// A zip of stored (not compressed) entries, which the data loader reads back.
pub(crate) fn stored_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let crc = crc32(data);
        let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);
        // Local file header: version 2.0, UTF-8 names (bit 11), stored, no time.
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes());
        out.extend_from_slice(&0x0800_u16.to_le_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0x0800_u16.to_le_bytes());
        central.extend_from_slice(&[0; 6]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_at = u32::try_from(out.len()).unwrap_or(u32::MAX);
    let central_len = u32::try_from(central.len()).unwrap_or(u32::MAX);
    let count = u16::try_from(entries.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_at.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc32_matches_the_check_value() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
}
