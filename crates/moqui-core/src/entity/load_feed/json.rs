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

//! Entity JSON data files, read one array element at a time.

use std::collections::BTreeMap;
use std::io::BufRead;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

use super::loader::{
    apply_key_pairs, call_data_file_service, decode_binary, entity_defined, inherit_pk_fields, shape_of, shared_pks,
    skip_type, store_record, Record,
};
use super::pull::Pull;
use super::{DataFileService, LoadOptions};
use crate::entity::facade::EntityFacade;

pub(super) fn load_json_text(
    facade: &EntityFacade,
    location: &str,
    text: &str,
    options: &LoadOptions,
) -> Result<u64, Error> {
    load_json_reader(facade, location, std::io::Cursor::new(text.as_bytes()), options)
}

pub(super) fn load_json_reader(
    facade: &EntityFacade,
    location: &str,
    reader: impl BufRead,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let mut input = Pull::new(reader);
    input.skip_bom()?;
    let kind = input.take_with(json_start, "json root")?;
    match kind {
        JsonStart::Object => {
            let text = input.take_text(json_value_end, "json value")?;
            let loaded = load_json_text_value(facade, location, &text, options)?;
            input.consume_match(json_ws, "json space")?;
            if !input.exhausted()? {
                return Err(Error::new(format!("{location} json has trailing data")));
            }
            Ok(loaded)
        }
        JsonStart::Array => {
            let mut total = 0_u64;
            let mut file_type = None;
            let mut first = true;
            loop {
                input.consume_match(json_ws, "json space")?;
                if input.starts_with("]")? {
                    input.consume(1);
                    break;
                }
                if input.exhausted()? {
                    return Err(Error::new(format!("{location} json array ended early")));
                }
                let text = input.take_text(json_value_end, "json value")?;
                let value = parse_json(&text)?;
                let mut marker = false;
                if first {
                    first = false;
                    if let Json::Object(pairs) = &value {
                        if let Some(kind) = json_object_string(pairs, "_dataType") {
                            file_type = Some(kind);
                            marker = true;
                        }
                    }
                }
                if !marker && !skip_type(file_type.as_deref(), &options.data_types) {
                    total = total.saturating_add(load_json_document(facade, location, value, options)?);
                }
                let end = input.take_with(json_after_value, "json separator")?;
                if end {
                    break;
                }
            }
            Ok(total)
        }
    }
}

fn load_json_text_value(
    facade: &EntityFacade,
    location: &str,
    text: &str,
    options: &LoadOptions,
) -> Result<u64, Error> {
    let root = parse_json(text)?;
    let (file_type, values) = json_documents(root)?;
    if skip_type(file_type.as_deref(), &options.data_types) {
        return Ok(0);
    }
    let mut total = 0_u64;
    for value in values {
        total = total.saturating_add(load_json_document(facade, location, value, options)?);
    }
    Ok(total)
}

fn load_json_document(facade: &EntityFacade, location: &str, value: Json, options: &LoadOptions) -> Result<u64, Error> {
    let Json::Object(pairs) = value else {
        tracing::warn!(location, "json value is not an object");
        return Ok(0);
    };
    let entity_name =
        json_object_string(&pairs, "_entity").ok_or_else(|| Error::new(format!("json {location} has no _entity")))?;
    if entity_defined(facade, &entity_name) {
        let record = json_record(facade, &entity_name, &pairs)?;
        store_record(facade, options, &record)
    } else if entity_name.contains('#') {
        call_data_file_service(
            facade,
            options,
            DataFileService {
                name: entity_name,
                location: location.to_string(),
                parameters: json_service_parameters(&pairs)?,
            },
        )?;
        Ok(0)
    } else {
        Err(Error::new(format!(
            "json {location} _entity {entity_name} is not an entity or a service"
        )))
    }
}

fn json_documents(root: Json) -> Result<(Option<String>, Vec<Json>), Error> {
    match root {
        Json::Object(pairs) => {
            let file_type = json_object_string(&pairs, "_dataType");
            Ok((file_type, vec![Json::Object(pairs)]))
        }
        Json::Array(mut values) => {
            let file_type = values.first().and_then(|value| {
                if let Json::Object(pairs) = value {
                    json_object_string(pairs, "_dataType")
                } else {
                    None
                }
            });
            // A leading array object that carries `_dataType` is the file type and is not a record.
            if file_type.is_some() {
                values.remove(0);
            }
            Ok((file_type, values))
        }
        _ => Err(Error::new("json root must be an object or an array")),
    }
}

fn json_record(facade: &EntityFacade, entity_name: &str, pairs: &[(String, Json)]) -> Result<Record, Error> {
    json_record_in(facade, entity_name, pairs, &BTreeMap::new())
}

fn json_record_in(
    facade: &EntityFacade,
    entity_name: &str,
    pairs: &[(String, Json)],
    ancestor: &BTreeMap<String, Value>,
) -> Result<Record, Error> {
    let shape = shape_of(facade, entity_name)?;
    let mut fields = BTreeMap::new();
    let mut nested_values = Vec::new();
    for (key, value) in pairs {
        if key == "_entity" || key == "_dataType" {
            continue;
        }
        match value {
            Json::Object(_) | Json::Array(_) => nested_values.push((key, value)),
            _ => {
                if !shape.fields.contains_key(key) {
                    tracing::warn!(entity = %shape.full_name, field = %key, "json field is not on the entity");
                    continue;
                }
                if let Some(converted) =
                    json_field_value(shape.fields.get(key).map(String::as_str).unwrap_or(""), value)?
                {
                    fields.insert(key.clone(), converted);
                }
            }
        }
    }
    let shared = shared_pks(&shape, &fields, ancestor);
    let mut children = Vec::new();
    for (key, value) in nested_values {
        let Some(rel) = shape.relations.get(key) else {
            if entity_defined(facade, key) {
                let mut nested = json_children(facade, key, value, &shared)?;
                for child in &mut nested {
                    inherit_pk_fields(facade, &child.entity, &shared, &mut child.fields)?;
                }
                children.extend(nested);
            } else {
                tracing::warn!(entity = %shape.full_name, field = %key, "json nested value is not a relationship");
            }
            continue;
        };
        let mut nested = json_children(facade, &rel.related, value, &shared)?;
        for child in &mut nested {
            apply_key_pairs(&fields, rel, &mut child.fields);
            inherit_pk_fields(facade, &child.entity, &shared, &mut child.fields)?;
        }
        children.extend(nested);
    }
    Ok(Record {
        entity: shape.full_name,
        fields,
        children,
    })
}

fn json_children(
    facade: &EntityFacade,
    entity_name: &str,
    value: &Json,
    ancestor: &BTreeMap<String, Value>,
) -> Result<Vec<Record>, Error> {
    match value {
        Json::Object(pairs) => Ok(vec![json_record_in(facade, entity_name, pairs, ancestor)?]),
        Json::Array(values) => {
            let mut records = Vec::new();
            for value in values {
                let Json::Object(pairs) = value else {
                    return Err(Error::new(format!("nested {entity_name} value is not an object")));
                };
                records.push(json_record_in(facade, entity_name, pairs, ancestor)?);
            }
            Ok(records)
        }
        _ => Err(Error::new(format!("nested {entity_name} value is not an object"))),
    }
}

fn json_service_parameters(pairs: &[(String, Json)]) -> Result<BTreeMap<String, String>, Error> {
    let mut parameters = BTreeMap::new();
    for (key, value) in pairs {
        if key == "_entity" || key == "_dataType" {
            continue;
        }
        match value {
            Json::Object(_) | Json::Array(_) => {
                return Err(Error::new(format!("service parameter {key} is not a scalar")));
            }
            Json::Null => {}
            Json::String(text) if text.is_empty() => {}
            other => {
                if let Some(text) = json_scalar_string(other) {
                    parameters.insert(key.clone(), text);
                }
            }
        }
    }
    Ok(parameters)
}

fn json_object_string(pairs: &[(String, Json)], key: &str) -> Option<String> {
    pairs.iter().find_map(|(name, value)| {
        if name == key {
            json_scalar_string(value).filter(|text| !text.is_empty())
        } else {
            None
        }
    })
}

fn json_scalar_string(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(text) => Some(text.clone()),
        Json::String(text) => Some(text.clone()),
        Json::Array(_) | Json::Object(_) => None,
    }
}

fn json_field_value(field_type: &str, value: &Json) -> Result<Option<Value>, Error> {
    match value {
        Json::Null => Ok(None),
        Json::String(text) if text.is_empty() => Ok(None),
        Json::String(text) if field_type == "binary-very-long" => Ok(Some(Value::Bytes(decode_binary(text)))),
        Json::String(text) => Ok(Some(Value::string(text))),
        Json::Bool(flag) => Ok(Some(Value::Bool(*flag))),
        Json::Number(text) => Ok(Some(parse_json_number(text))),
        Json::Array(_) | Json::Object(_) => Ok(None),
    }
}

fn parse_json_number(text: &str) -> Value {
    if text.contains(['.', 'e', 'E']) {
        text.parse::<f64>()
            .map(Value::F64)
            .unwrap_or_else(|_| Value::string(text))
    } else {
        text.parse::<i64>()
            .map(Value::I64)
            .unwrap_or_else(|_| Value::string(text))
    }
}

enum JsonStart {
    Object,
    Array,
}

fn json_ws(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    let n = text.bytes().take_while(|byte| is_json_ws(*byte)).count();
    if n == text.len() && !eof {
        return Ok(None);
    }
    Ok(Some(n))
}

fn json_start(text: &str, eof: bool) -> Result<Option<(usize, JsonStart)>, Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_json_ws(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("json ended early"))
        } else {
            Ok(None)
        };
    }
    match bytes[i] {
        b'{' => Ok(Some((i, JsonStart::Object))),
        b'[' => Ok(Some((i + 1, JsonStart::Array))),
        found => Err(Error::new(format!("unexpected json character {}", char::from(found)))),
    }
}

pub(super) fn json_value_end(text: &str, eof: bool) -> Result<Option<usize>, Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_json_ws(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("json ended early"))
        } else {
            Ok(None)
        };
    }
    match bytes[i] {
        b'"' => scan_string(bytes, i, eof),
        b'{' | b'[' => scan_container(bytes, i, eof),
        b't' => scan_lit(bytes, i, b"true", eof),
        b'f' => scan_lit(bytes, i, b"false", eof),
        b'n' => scan_lit(bytes, i, b"null", eof),
        b'-' | b'0'..=b'9' => scan_number(bytes, i, eof),
        found => Err(Error::new(format!("unexpected json character {}", char::from(found)))),
    }
}

fn json_after_value(text: &str, eof: bool) -> Result<Option<(usize, bool)>, Error> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_json_ws(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return if eof {
            Err(Error::new("json separator ended early"))
        } else {
            Ok(None)
        };
    }
    match bytes[i] {
        b',' => Ok(Some((i + 1, false))),
        b']' => Ok(Some((i + 1, true))),
        found => Err(Error::new(format!("unexpected json character {}", char::from(found)))),
    }
}

fn scan_string(bytes: &[u8], start: usize, eof: bool) -> Result<Option<usize>, Error> {
    let mut i = start + 1;
    let mut escape = false;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if escape {
            escape = false;
            continue;
        }
        if byte == b'\\' {
            escape = true;
            continue;
        }
        if byte == b'"' {
            return Ok(Some(i));
        }
    }
    if eof {
        Err(Error::new("json string ended early"))
    } else {
        Ok(None)
    }
}

fn scan_container(bytes: &[u8], start: usize, eof: bool) -> Result<Option<usize>, Error> {
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escape = false;
    let mut i = start;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if byte == b'\\' {
                escape = true;
                continue;
            }
            if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(i));
                }
                if depth < 0 {
                    return Err(Error::new("json container is not closed"));
                }
            }
            _ => {}
        }
    }
    if eof {
        Err(Error::new("json ended early"))
    } else {
        Ok(None)
    }
}

fn scan_lit(bytes: &[u8], start: usize, lit: &[u8], eof: bool) -> Result<Option<usize>, Error> {
    let have = bytes.len().saturating_sub(start);
    if have < lit.len() {
        if bytes[start..].iter().zip(lit.iter()).all(|(got, want)| got == want) {
            return if eof {
                Err(Error::new("json literal is incomplete"))
            } else {
                Ok(None)
            };
        }
        return Err(Error::new("json literal is incomplete"));
    }
    if &bytes[start..start + lit.len()] != lit {
        return Err(Error::new("json literal is incomplete"));
    }
    if start + lit.len() == bytes.len() && !eof {
        return Ok(None);
    }
    Ok(Some(start + lit.len()))
}

fn scan_number(bytes: &[u8], start: usize, eof: bool) -> Result<Option<usize>, Error> {
    let mut i = start;
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    if i >= bytes.len() {
        return number_end(i, eof, false);
    }
    if bytes[i] == b'0' {
        i += 1;
    } else if bytes[i].is_ascii_digit() {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        return Err(Error::new("json number is incomplete"));
    }
    if i >= bytes.len() {
        return number_end(i, eof, true);
    }
    if bytes[i] == b'.' {
        i += 1;
        if i >= bytes.len() {
            return number_end(i, eof, false);
        }
        if !bytes[i].is_ascii_digit() {
            return Err(Error::new("json fraction is incomplete"));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i >= bytes.len() {
            return number_end(i, eof, true);
        }
    }
    if bytes[i] == b'e' || bytes[i] == b'E' {
        i += 1;
        if i >= bytes.len() {
            return number_end(i, eof, false);
        }
        if bytes[i] == b'+' || bytes[i] == b'-' {
            i += 1;
        }
        if i >= bytes.len() {
            return number_end(i, eof, false);
        }
        if !bytes[i].is_ascii_digit() {
            return Err(Error::new("json exponent is incomplete"));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i >= bytes.len() {
            return number_end(i, eof, true);
        }
    }
    Ok(Some(i))
}

fn number_end(i: usize, eof: bool, complete: bool) -> Result<Option<usize>, Error> {
    if !eof {
        Ok(None)
    } else if complete {
        Ok(Some(i))
    } else {
        Err(Error::new("json number is incomplete"))
    }
}

fn is_json_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

pub(super) enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

struct JsonParser<'a> {
    text: &'a str,
    index: usize,
}

pub(super) fn parse_json(text: &str) -> Result<Json, Error> {
    let mut parser = JsonParser { text, index: 0 };
    parser.skip();
    let value = parser.value()?;
    parser.skip();
    if parser.index != parser.text.len() {
        return Err(Error::new("json has trailing data"));
    }
    Ok(value)
}

impl JsonParser<'_> {
    fn skip(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.index..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn value(&mut self) -> Result<Json, Error> {
        self.skip();
        match self.peek() {
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('"') => Ok(Json::String(self.string()?)),
            Some('[') => self.array(),
            Some('{') => self.object(),
            Some('-') | Some('0'..='9') => Ok(Json::Number(self.number()?)),
            Some(ch) => Err(Error::new(format!("unexpected json character {ch}"))),
            None => Err(Error::new("json ended early")),
        }
    }

    fn literal(&mut self, expected: &str, value: Json) -> Result<Json, Error> {
        for ch in expected.chars() {
            if self.bump() != Some(ch) {
                return Err(Error::new(format!("json literal {expected} is incomplete")));
            }
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, Error> {
        if self.bump() != Some('"') {
            return Err(Error::new("json string does not start with a quote"));
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                Some('"') => return Ok(out),
                Some('\\') => out.push(self.escape()?),
                Some(ch) => out.push(ch),
                None => return Err(Error::new("json string ended early")),
            }
        }
    }

    fn escape(&mut self) -> Result<char, Error> {
        match self.bump() {
            Some('"') => Ok('"'),
            Some('\\') => Ok('\\'),
            Some('/') => Ok('/'),
            Some('b') => Ok('\u{0008}'),
            Some('f') => Ok('\u{000c}'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('u') => self.hex_char(),
            Some(ch) => Err(Error::new(format!("json escape {ch} is not valid"))),
            None => Err(Error::new("json escape ended early")),
        }
    }

    fn hex_char(&mut self) -> Result<char, Error> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let Some(ch) = self.bump() else {
                return Err(Error::new("json hex escape ended early"));
            };
            let digit = ch
                .to_digit(16)
                .ok_or_else(|| Error::new("json hex escape is not hex"))?;
            value = (value << 4) | digit;
        }
        char::from_u32(value).ok_or_else(|| Error::new("json hex escape is not a character"))
    }

    fn number(&mut self) -> Result<String, Error> {
        let start = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        if self.peek() == Some('0') {
            self.bump();
        } else if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.bump();
            }
        } else {
            return Err(Error::new("json number is incomplete"));
        }
        if self.peek() == Some('.') {
            self.bump();
            if !self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                return Err(Error::new("json fraction is incomplete"));
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.bump();
            }
        }
        if self.peek().is_some_and(|ch| ch == 'e' || ch == 'E') {
            self.bump();
            if self.peek().is_some_and(|ch| ch == '+' || ch == '-') {
                self.bump();
            }
            if !self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                return Err(Error::new("json exponent is incomplete"));
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.bump();
            }
        }
        Ok(self.text[start..self.index].to_string())
    }

    fn array(&mut self) -> Result<Json, Error> {
        self.bump();
        let mut values = Vec::new();
        self.skip();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.skip();
            match self.bump() {
                Some(',') => self.skip(),
                Some(']') => return Ok(Json::Array(values)),
                _ => return Err(Error::new("json array is not closed")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, Error> {
        self.bump();
        let mut pairs = Vec::new();
        self.skip();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Json::Object(pairs));
        }
        loop {
            self.skip();
            if self.peek() != Some('"') {
                return Err(Error::new("json object key is not a string"));
            }
            let key = self.string()?;
            self.skip();
            if self.bump() != Some(':') {
                return Err(Error::new("json object is missing a colon"));
            }
            pairs.push((key, self.value()?));
            self.skip();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Json::Object(pairs)),
                _ => return Err(Error::new("json object is not closed")),
            }
        }
    }
}
