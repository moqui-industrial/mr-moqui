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
//! Each entity is read through a cursor and written as it is read, a chunk of records at a time, so neither a table
//! nor the whole export is held in memory. Records written with their dependents are the exception: one entity's
//! records are read first. Zip entries are deflated, with zip64 past 4 GiB.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use moqui_lib::error::Error;
use moqui_lib::object::Value;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use super::facade::EntityFacade;
use super::find::EntityFind;
use super::query_rest::{csv_cell, data_body};
use crate::context::ExecutionContext;
use crate::transaction::TransactionAttribute;
use moqui_db::sql::Condition;

/// Records rendered and written at a time.
const CHUNK: usize = 500;
/// Transaction timeout for reading one entity, as `EntityDataWriterImpl`'s `txTimeout`.
const READ_TIMEOUT: Duration = Duration::from_secs(3600);

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
        let mut out = Vec::new();
        let written = self.write_document(&names, &mut out)?;
        let text = String::from_utf8(out).map_err(|err| Error::new("data text is not utf-8").with_source(err))?;
        Ok((text, written))
    }

    /// `file`: one file. The extension of the name sets the type.
    ///
    /// A file that cannot be written, or records that cannot be read, are logged and a message error, and 0 is
    /// returned, as in Java.
    pub fn file(&mut self, filename: &str) -> Result<u64, Error> {
        if Path::new(filename).exists() {
            add_error(format!("File {filename} already exists."));
            return Ok(0);
        }
        if let Some(kind) = DataFileType::from_name(filename) {
            self.file_type = kind;
        }
        let names = self.active_names();
        if self.file_type == DataFileType::Csv && names.len() > 1 {
            add_error("Cannot write to single CSV file with multiple entity names");
            return Ok(0);
        }
        let written = match write_new(Path::new(filename), |out| self.write_document(&names, out)) {
            Ok(written) => written,
            Err(err) => return Ok(write_failed(filename, &err)),
        };
        add_message(format!("Wrote {written} records to file {filename}"));
        Ok(written)
    }

    /// `zipFile`: one file inside a new zip. The extension of the name in the zip sets the type.
    ///
    /// A zip that cannot be written, or records that cannot be read, are logged and a message error, and 0 is
    /// returned, as in Java.
    pub fn zip_file(&mut self, filename_within_zip: &str, zip_filename: &str) -> Result<u64, Error> {
        if Path::new(zip_filename).exists() {
            add_error(format!("File {zip_filename} already exists."));
            return Ok(0);
        }
        if let Some(kind) = DataFileType::from_name(filename_within_zip) {
            self.file_type = kind;
        }
        let names = self.active_names();
        if self.file_type == DataFileType::Csv && names.len() > 1 {
            add_error("Cannot write to single CSV file with multiple entity names");
            return Ok(0);
        }
        let written = write_new(Path::new(zip_filename), |out| {
            let mut zip = ZipWriter::new(out);
            zip.start_file(filename_within_zip, entry_options())
                .map_err(|err| zip_error(zip_filename, err))?;
            let written = self.write_document(&names, &mut zip)?;
            zip.finish().map_err(|err| zip_error(zip_filename, err))?;
            Ok(written)
        });
        let written = match written {
            Ok(written) => written,
            Err(err) => return Ok(write_failed(zip_filename, &err)),
        };
        add_message(format!("Wrote {written} records to file {zip_filename}"));
        Ok(written)
    }

    /// `directory`: a file per entity that has records, named for the entity. A file that is there already is
    /// skipped with a message error.
    ///
    /// An entity whose file cannot be written is logged and a message error, and the other entities are still written.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created.
    pub fn directory(&self, path: &str) -> Result<u64, Error> {
        let dir = Path::new(path);
        if !dir.exists() {
            std::fs::create_dir_all(dir)
                .map_err(|err| Error::new(format!("could not create {path}")).with_source(err))?;
        }
        if !dir.is_dir() {
            add_error(format!("Path {path} is not a directory."));
            return Ok(0);
        }
        let mut written = 0;
        // An entity that cannot be written is logged and a message error; the other entities are still written.
        for name in self.active_names() {
            match self.has_records(&name) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(err) => {
                    write_failed(&format!("{path} for entity {name}"), &err);
                    continue;
                }
            }
            let filename = format!("{path}/{name}.{}", self.file_type.extension());
            if Path::new(&filename).exists() {
                add_error(format!("File {filename} already exists, skipping entity {name}."));
                continue;
            }
            match write_new(Path::new(&filename), |out| self.write_entity_file(&name, out)) {
                Ok(count) => {
                    add_message(format!("Wrote {count} records to file {filename}"));
                    written += count;
                }
                Err(err) => {
                    write_failed(&filename, &err);
                }
            }
        }
        Ok(written)
    }

    /// `zipDirectory`: a new zip with an entry per entity that has records, under `path_within_zip`. Each entry is
    /// written as its records are read.
    ///
    /// A zip that cannot be written, or records that cannot be read, are logged and a message error, and 0 is
    /// returned, as in Java.
    pub fn zip_directory(&self, path_within_zip: &str, zip_filename: &str) -> Result<u64, Error> {
        if Path::new(zip_filename).exists() {
            add_error(format!("File {zip_filename} already exists."));
            return Ok(0);
        }
        let written = write_new(Path::new(zip_filename), |out| {
            let mut zip = ZipWriter::new(out);
            let mut written = 0;
            for name in self.active_names() {
                if !self.has_records(&name)? {
                    continue;
                }
                let base = if self.table_column_names {
                    self.facade
                        .with_entity(&name, |entity, _, _| Ok(entity.table_name.clone()))?
                } else {
                    name.clone()
                };
                let within = if path_within_zip.is_empty() {
                    format!("{base}.{}", self.file_type.extension())
                } else {
                    format!("{path_within_zip}/{base}.{}", self.file_type.extension())
                };
                zip.start_file(within.as_str(), entry_options())
                    .map_err(|err| zip_error(zip_filename, err))?;
                let count = self.write_entity_file(&name, &mut zip)?;
                add_message(format!("Wrote {count} records to {within}"));
                written += count;
            }
            zip.finish().map_err(|err| zip_error(zip_filename, err))?;
            Ok(written)
        });
        Ok(written.unwrap_or_else(|err| write_failed(zip_filename, &err)))
    }

    /// The file start, each entity's records (after its CSV header), and the file end.
    fn write_document(&self, names: &[String], out: &mut impl Write) -> Result<u64, Error> {
        write_text(out, start_file(self.file_type))?;
        let mut written = 0;
        for name in names {
            if self.file_type == DataFileType::Csv {
                write_text(out, &self.csv_header(name)?)?;
            }
            written += self.write_records(name, out)?;
        }
        write_text(out, end_file(self.file_type))?;
        Ok(written)
    }

    /// One entity as a whole file.
    fn write_entity_file(&self, name: &str, out: &mut impl Write) -> Result<u64, Error> {
        self.write_document(&[name.to_string()], out)
    }

    /// Whether the find of this entity has a record.
    fn has_records(&self, name: &str) -> Result<bool, Error> {
        let (find, _, _) = self.entity_find(name)?;
        Ok(!find.limit(1).list()?.is_empty())
    }

    /// The find of one entity, as `makeEntityFind`: the filter map, the order, and the update stamp range. Also the
    /// full name and the master definition this writer renders with, if the entity has it.
    fn entity_find(&self, name: &str) -> Result<(EntityFind, String, Option<&str>), Error> {
        let (has_stamp, full_name, masters) = self.facade.with_entity(name, |entity, _, _| {
            Ok((
                entity.has_field("lastUpdatedStamp"),
                entity.full_name.clone(),
                entity
                    .masters
                    .iter()
                    .map(|master| master.name.clone())
                    .collect::<Vec<_>>(),
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
        let master = self
            .master
            .as_deref()
            .filter(|master| masters.iter().any(|known| known == master));
        Ok((find, full_name, master))
    }

    /// Write the records of one entity as they are read from a cursor, [`CHUNK`] at a time. Records with dependents
    /// are read first, because a dependent query needs the connection the cursor holds. Returns the record count.
    fn write_records(&self, name: &str, out: &mut impl Write) -> Result<u64, Error> {
        let (find, full_name, master) = self.entity_find(name)?;
        let dependents = self.file_type != DataFileType::Csv && (master.is_some() || self.dependent_levels > 0);
        let tx = self.facade.transactions_for(&full_name)?;
        tx.run(TransactionAttribute::UseOrBegin, Some(READ_TIMEOUT), || {
            let mut cursor = find.iterator()?;
            let mut rows = Vec::new();
            let mut count = 0_u64;
            while let Some(value) = cursor.next_value()? {
                count += 1;
                rows.push(value.fields().clone());
                if !dependents && rows.len() >= CHUNK {
                    write_text(out, &self.render(&full_name, &rows, master)?)?;
                    rows.clear();
                }
            }
            cursor.close()?;
            for chunk in rows.chunks(CHUNK) {
                write_text(out, &self.render(&full_name, chunk, master)?)?;
            }
            Ok(count)
        })
    }

    fn render(&self, full_name: &str, rows: &[BTreeMap<String, Value>], master: Option<&str>) -> Result<String, Error> {
        if self.file_type == DataFileType::Csv {
            return self.csv_rows(full_name, rows);
        }
        data_body(
            &self.facade,
            full_name,
            rows,
            self.file_type == DataFileType::Json,
            self.dependent_levels,
            master,
            &self.prefix,
            self.iso_date_time,
        )
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

fn start_file(kind: DataFileType) -> &'static str {
    match kind {
        DataFileType::Json => "[\n",
        DataFileType::Xml => "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<entity-facade-xml>\n",
        DataFileType::Csv => "",
    }
}

fn end_file(kind: DataFileType) -> &'static str {
    match kind {
        DataFileType::Json => "]\n\n",
        DataFileType::Xml => "</entity-facade-xml>\n\n",
        DataFileType::Csv => "",
    }
}

fn write_text(out: &mut impl Write, text: &str) -> Result<(), Error> {
    out.write_all(text.as_bytes())
        .map_err(|err| Error::new("could not write data").with_source(err))
}

/// Create the new file `path` and write it with `body`. When `body` fails the file is removed, so a failed write
/// leaves no partial file.
fn write_new<T>(path: &Path, body: impl FnOnce(&mut BufWriter<File>) -> Result<T, Error>) -> Result<T, Error> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|err| Error::new(format!("could not create {}", parent.display())).with_source(err))?;
    }
    let file = File::create_new(path)
        .map_err(|err| Error::new(format!("could not write {}", path.display())).with_source(err))?;
    let mut out = BufWriter::new(file);
    let result = body(&mut out).and_then(|value| {
        out.flush()
            .map_err(|err| Error::new(format!("could not write {}", path.display())).with_source(err))?;
        Ok(value)
    });
    if result.is_err() {
        drop(out);
        if let Err(err) = std::fs::remove_file(path) {
            tracing::warn!(error = %err, path = %path.display(), "could not remove a partly written data file");
        }
    }
    result
}

/// Deflated, stamped now, and zip64 when an entry or the archive passes 4 GiB, as `ZipOutputStream` writes them.
fn entry_options() -> SimpleFileOptions {
    let now = jiff::Zoned::now().datetime();
    let stamp = (|| {
        DateTime::from_date_and_time(
            u16::try_from(now.year()).ok()?,
            u8::try_from(now.month()).ok()?,
            u8::try_from(now.day()).ok()?,
            u8::try_from(now.hour()).ok()?,
            u8::try_from(now.minute()).ok()?,
            u8::try_from(now.second()).ok()?,
        )
        .ok()
    })()
    .unwrap_or_default();
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(stamp)
        .large_file(true)
}

fn zip_error(zip_filename: &str, err: ZipError) -> Error {
    Error::new(format!("could not write zip {zip_filename}")).with_source(err)
}

/// As Java's `EntityDataWriterImpl`, a write that fails is logged and a message error, and nothing is counted for it.
fn write_failed(target: &str, err: &Error) -> u64 {
    tracing::warn!(error = %err, "Error writing data to {target}");
    add_error(format!("Error writing data to {target}: {}", err.message()));
    0
}

fn add_message(text: String) {
    ExecutionContext::with_current(|ctx| ctx.message().add_message(text));
}

fn add_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use moqui_lib::object::Value;

    use super::{DataFileType, EntityDataWriter};
    use crate::entity::load_feed::{load_entity_data, LoadOptions};
    use crate::entity::EntityFacade;

    struct DirGuard(PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> (PathBuf, DirGuard) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moqui-writer-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        (path.clone(), DirGuard(path))
    }

    /// `rows` test entity rows, more than one chunk, and two `Foo` rows.
    fn seeded(rows: usize) -> EntityFacade {
        let facade = crate::testing::open();
        for index in 0..rows {
            let mut row = facade.make_value("moqui.test.TestEntity").unwrap();
            row.set("testId", Value::string(format!("T{index:04}")));
            row.set("testMedium", Value::string(format!("medium <{index}>")));
            row.create(&facade).unwrap();
        }
        for id in ["F1", "F2"] {
            let mut row = facade.make_value("moqui.test.Foo").unwrap();
            row.set("fooId", Value::string(id));
            row.create(&facade).unwrap();
        }
        facade
    }

    fn count(facade: &EntityFacade, entity: &str) -> i64 {
        facade.find(entity).unwrap().count().unwrap()
    }

    #[test]
    fn zip_directory_streams_each_entity_and_loads_back() {
        let source = seeded(1203);
        let (dir, _guard) = temp_dir();
        let zip_path = dir.join("export.zip");
        let zip_name = zip_path.to_str().unwrap();
        let mut writer = EntityDataWriter::new(&source);
        writer
            .entity_name("moqui.test.TestEntity")
            .entity_name("moqui.test.Foo")
            .entity_name("moqui.basic.Enumeration");
        assert_eq!(writer.zip_directory("data", zip_name).unwrap(), 1205);

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        assert_eq!(
            names,
            ["data/moqui.test.TestEntity.xml", "data/moqui.test.Foo.xml"],
            "no entry for an entity without records"
        );
        assert_eq!(
            archive.by_index(0).unwrap().compression(),
            zip::CompressionMethod::Deflated
        );

        let target = crate::testing::open();
        let loaded = load_entity_data(&target, zip_name, &LoadOptions::default()).unwrap();
        assert_eq!(loaded.records, 1205);
        assert_eq!(count(&target, "moqui.test.TestEntity"), 1203);
        assert_eq!(count(&target, "moqui.test.Foo"), 2);
        let last = target
            .find("moqui.test.TestEntity")
            .unwrap()
            .eq("testId", Value::string("T1202"))
            .one()
            .unwrap()
            .expect("last row");
        assert_eq!(last.get("testMedium").and_then(Value::as_str), Some("medium <1202>"));

        // A zip that is there already is a message error and is left as it was.
        let before = std::fs::read(&zip_path).unwrap();
        assert_eq!(writer.zip_directory("data", zip_name).unwrap(), 0);
        assert_eq!(std::fs::read(&zip_path).unwrap(), before);
    }

    #[test]
    fn text_files_and_a_zip_file_hold_every_record() {
        let source = seeded(600);
        let (dir, _guard) = temp_dir();

        let mut writer = EntityDataWriter::new(&source);
        writer.entity_name("moqui.test.TestEntity").file_type(DataFileType::Csv);
        let (csv, written) = writer.text().unwrap();
        assert_eq!(written, 600);
        assert_eq!(csv.lines().count(), 602, "two header lines, then a line per record");

        let json_path = dir.join("rows.json");
        let mut writer = EntityDataWriter::new(&source);
        writer
            .entity_name("moqui.test.TestEntity")
            .entity_name("moqui.test.Foo");
        assert_eq!(writer.file(json_path.to_str().unwrap()).unwrap(), 602);
        let target = crate::testing::open();
        let loaded = load_entity_data(&target, json_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(loaded.records, 602);

        let tables = dir.join("tables");
        let mut writer = EntityDataWriter::new(&source);
        writer
            .entity_name("moqui.test.Foo")
            .entity_name("moqui.basic.Enumeration");
        assert_eq!(writer.directory(tables.to_str().unwrap()).unwrap(), 2);
        assert!(tables.join("moqui.test.Foo.xml").exists());
        assert!(!tables.join("moqui.basic.Enumeration.xml").exists());

        let zip_path = dir.join("one.zip");
        let mut writer = EntityDataWriter::new(&source);
        writer.entity_name("moqui.test.TestEntity");
        assert_eq!(writer.zip_file("rows.xml", zip_path.to_str().unwrap()).unwrap(), 600);
        let target = crate::testing::open();
        let loaded = load_entity_data(&target, zip_path.to_str().unwrap(), &LoadOptions::default()).unwrap();
        assert_eq!(loaded.records, 600);
    }
}
