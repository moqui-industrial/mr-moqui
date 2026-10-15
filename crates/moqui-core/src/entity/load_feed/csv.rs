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

//! Entity CSV data files: the entity or service name and type line, the header line, then a record per line.

use std::collections::BTreeMap;
use std::io::Read;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::loader::{call_data_file_service, entity_defined, skip_type, store_record, Record};
use super::{DataFileService, LoadOptions};
use crate::entity::facade::EntityFacade;

pub(super) fn load_csv_reader(
    facade: &EntityFacade,
    location: &str,
    reader: impl Read,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b',')
        .comment(Some(b'#'))
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(reader);
    let mut records = reader.records();
    let first = loop {
        match records.next() {
            Some(Ok(record)) if !record_blank(&record) => break record,
            Some(Ok(_)) => {}
            Some(Err(err)) => return Err(Error::new(format!("could not read csv {location}")).with_source(err)),
            None => return Err(Error::new(format!("csv {location} has no data"))),
        }
    };
    let entity_name = first.get(0).unwrap_or("").trim().to_string();
    if entity_name.is_empty() {
        return Err(Error::new(format!("csv {location} has no entity name")));
    }
    let file_type = first.get(1).map(str::trim).filter(|value| !value.is_empty());
    if skip_type(file_type, &options.data_types) {
        return Ok(0);
    }
    let header = loop {
        match records.next() {
            Some(Ok(record)) if !record_blank(&record) => break record,
            Some(Ok(_)) => {}
            Some(Err(err)) => return Err(Error::new(format!("could not read csv {location}")).with_source(err)),
            None => return Err(Error::new(format!("csv {location} has no header"))),
        }
    };
    let mut headers = Vec::new();
    for (index, field) in header.iter().enumerate() {
        if field.is_empty() || headers.iter().any(|(name, _)| name == field) {
            continue;
        }
        headers.push((field.to_string(), index));
    }
    let is_entity = entity_defined(facade, &entity_name);
    if !is_entity && !entity_name.contains('#') {
        return Err(Error::new(format!(
            "csv {location} names {entity_name}, which is not an entity or a service"
        )));
    }
    let mut total = 0_u64;
    for record in records {
        let record = record.map_err(|err| Error::new(format!("could not read csv {location}")).with_source(err))?;
        if record_blank(&record) {
            continue;
        }
        if is_entity {
            let mut fields = BTreeMap::new();
            for (name, index) in &headers {
                let Some(text) = record.get(*index) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                fields.insert(name.clone(), Value::string(text));
            }
            let stored = store_record(
                facade,
                options,
                &Record {
                    entity: entity_name.clone(),
                    fields,
                    children: Vec::new(),
                },
            )?;
            total = total.saturating_add(stored);
        } else {
            let mut parameters = BTreeMap::new();
            for (name, index) in &headers {
                if let Some(text) = record.get(*index).filter(|text| !text.is_empty()) {
                    parameters.insert(name.clone(), text.to_string());
                }
            }
            call_data_file_service(
                facade,
                options,
                DataFileService {
                    name: entity_name.clone(),
                    location: location.to_string(),
                    parameters,
                },
            )?;
        }
    }
    Ok(total)
}

fn record_blank(record: &csv::StringRecord) -> bool {
    record.iter().all(|field| field.is_empty())
}
