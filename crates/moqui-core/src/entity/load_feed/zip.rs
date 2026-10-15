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

//! Zip data files: each XML, CSV, JSON, or nested zip entry is loaded in turn.

use std::path::Path;

use moqui_lib::error::Error;

use super::csv::load_csv_reader;
use super::json::load_json_text;
use super::loader::extension;
use super::xml::load_xml_text;
use super::LoadOptions;
use crate::entity::facade::EntityFacade;

const MAX_INFLATE: usize = 16 * 1024 * 1024;

pub(super) fn load_zip_bytes(
    facade: &EntityFacade,
    location: &str,
    data: &[u8],
    options: &LoadOptions,
    depth: u32,
) -> Result<u64, Error> {
    let index = zip_index(data)?;
    let mut total = 0_u64;
    for meta in index {
        if meta.directory {
            continue;
        }
        if unsafe_zip_name(&meta.name) {
            return Err(Error::new(format!("zip entry {} is not a safe path", meta.name)));
        }
        let compressed = slice_at(
            data,
            meta.data_at,
            usize::try_from(meta.comp_size).unwrap_or(usize::MAX),
        )?;
        let plain = match meta.method {
            0 => compressed.to_vec(),
            8 => inflate(compressed)?,
            _ => {
                return Err(Error::new(format!(
                    "zip entry {} uses an unsupported method",
                    meta.name
                )));
            }
        };
        if meta.method == 8
            && meta.uncomp_size != 0
            && plain.len() != usize::try_from(meta.uncomp_size).unwrap_or(usize::MAX)
        {
            return Err(Error::new(format!(
                "zip entry {} inflated to the wrong size",
                meta.name
            )));
        }
        let started = std::time::Instant::now();
        let records = load_entry_bytes(facade, location, &meta.name, &plain, options, depth)?;
        if !matches!(extension(Path::new(&meta.name)).as_str(), "zip") {
            tracing::info!(
                "Loaded {records} records from {} in zip file {location} in {:.2}s",
                meta.name,
                started.elapsed().as_secs_f64()
            );
        }
        total = total.saturating_add(records);
    }
    Ok(total)
}

fn load_entry_bytes(
    facade: &EntityFacade,
    location: &str,
    name: &str,
    bytes: &[u8],
    options: &LoadOptions,
    depth: u32,
) -> Result<u64, Error> {
    let entry_location = format!("{location}!{name}");
    match extension(Path::new(name)).as_str() {
        "xml" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|err| Error::new(format!("{entry_location} is not utf-8")).with_source(err))?;
            load_xml_text(facade, &entry_location, text, options)
        }
        "csv" => load_csv_reader(facade, &entry_location, bytes, options),
        "json" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|err| Error::new(format!("{entry_location} is not utf-8")).with_source(err))?;
            load_json_text(facade, &entry_location, text, options)
        }
        "zip" => load_zip_bytes(facade, &entry_location, bytes, options, depth + 1),
        _ => Ok(0),
    }
}

struct ZipMeta {
    name: String,
    method: u16,
    comp_size: u32,
    uncomp_size: u32,
    data_at: usize,
    directory: bool,
}

fn zip_index(data: &[u8]) -> Result<Vec<ZipMeta>, Error> {
    let eocd = find_eocd(data)?;
    let cd_size = read_u32(data, eocd + 12)?;
    let cd_offset = read_u32(data, eocd + 16)?;
    if cd_size == u32::MAX || cd_offset == u32::MAX {
        return Err(Error::new("zip64 is not supported"));
    }
    let entries = usize::from(read_u16(data, eocd + 10)?);
    let mut at = usize::try_from(cd_offset).map_err(|_| Error::new("zip directory offset is too large"))?;
    let mut metas = Vec::with_capacity(entries);
    for _ in 0..entries {
        if read_u32(data, at)? != 0x0201_4b50 {
            return Err(Error::new("zip directory entry is not valid"));
        }
        let flags = read_u16(data, at + 8)?;
        if flags & 1 != 0 {
            return Err(Error::new("encrypted zip entries are not supported"));
        }
        let method = read_u16(data, at + 10)?;
        let comp_size = read_u32(data, at + 20)?;
        let uncomp_size = read_u32(data, at + 24)?;
        let name_len = usize::from(read_u16(data, at + 28)?);
        let extra_len = usize::from(read_u16(data, at + 30)?);
        let comment_len = usize::from(read_u16(data, at + 32)?);
        let local_off = read_u32(data, at + 42)?;
        if comp_size == u32::MAX || uncomp_size == u32::MAX || local_off == u32::MAX {
            return Err(Error::new("zip64 is not supported"));
        }
        let name_bytes = slice_at(data, at + 46, name_len)?;
        let name = String::from_utf8(name_bytes.to_vec())
            .map_err(|err| Error::new("zip entry name is not utf-8").with_source(err))?;
        let local = usize::try_from(local_off).map_err(|_| Error::new("zip local offset is too large"))?;
        if read_u32(data, local)? != 0x0403_4b50 {
            return Err(Error::new(format!("zip local header for {name} is not valid")));
        }
        let local_name = usize::from(read_u16(data, local + 26)?);
        let local_extra = usize::from(read_u16(data, local + 28)?);
        let data_at = local
            .checked_add(30)
            .and_then(|value| value.checked_add(local_name))
            .and_then(|value| value.checked_add(local_extra))
            .ok_or_else(|| Error::new("zip local header offset overflow"))?;
        metas.push(ZipMeta {
            directory: name.ends_with('/'),
            name,
            method,
            comp_size,
            uncomp_size,
            data_at,
        });
        at = at
            .checked_add(46)
            .and_then(|value| value.checked_add(name_len))
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| Error::new("zip directory offset overflow"))?;
    }
    Ok(metas)
}

fn find_eocd(data: &[u8]) -> Result<usize, Error> {
    if data.len() < 22 {
        return Err(Error::new("zip is shorter than the end record"));
    }
    let earliest = data.len().saturating_sub(22 + 65_535);
    let mut index = data.len() - 22;
    loop {
        if data.get(index..index + 4) == Some(&[0x50, 0x4b, 0x05, 0x06][..]) {
            let comment = usize::from(u16::from_le_bytes([data[index + 20], data[index + 21]]));
            if index.checked_add(22).and_then(|value| value.checked_add(comment)) == Some(data.len()) {
                return Ok(index);
            }
        }
        if index == earliest {
            break;
        }
        index -= 1;
    }
    Err(Error::new("zip end record was not found"))
}

fn unsafe_zip_name(name: &str) -> bool {
    name.starts_with('/') || name.starts_with('\\') || name.split(['/', '\\']).any(|part| part == "..")
}

fn read_u16(data: &[u8], at: usize) -> Result<u16, Error> {
    let bytes = slice_at(data, at, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], at: usize) -> Result<u32, Error> {
    let bytes = slice_at(data, at, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn slice_at(data: &[u8], start: usize, len: usize) -> Result<&[u8], Error> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| Error::new("zip offset overflow"))?;
    data.get(start..end)
        .ok_or_else(|| Error::new("zip entry overruns the file"))
}

pub(super) fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut bits = Bits { data, index: 0, bit: 0 };
    let mut out = Vec::new();
    loop {
        let final_block = bits.bit()? == 1;
        match bits.bits(2)? {
            0 => inflate_stored(&mut bits, &mut out)?,
            1 => {
                let lit = Huffman::from_lengths(&fixed_lit_lengths())?;
                let dist = Huffman::from_lengths(&fixed_dist_lengths())?;
                inflate_codes(&mut bits, &mut out, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_trees(&mut bits)?;
                inflate_codes(&mut bits, &mut out, &lit, &dist)?;
            }
            _ => return Err(Error::new("deflate block type is invalid")),
        }
        if out.len() > MAX_INFLATE {
            return Err(Error::new("deflate output is too large"));
        }
        if final_block {
            break;
        }
    }
    Ok(out)
}

fn inflate_stored(bits: &mut Bits<'_>, out: &mut Vec<u8>) -> Result<(), Error> {
    bits.align();
    let len = bits.bits(16)?;
    let nlen = bits.bits(16)?;
    if len ^ 0xffff != nlen {
        return Err(Error::new("deflate stored length check failed"));
    }
    let bytes = bits.take_bytes(usize::try_from(len).unwrap_or(usize::MAX))?;
    if out.len().saturating_add(bytes.len()) > MAX_INFLATE {
        return Err(Error::new("deflate output is too large"));
    }
    out.extend_from_slice(&bytes);
    Ok(())
}

fn inflate_codes(bits: &mut Bits<'_>, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman) -> Result<(), Error> {
    loop {
        let symbol = lit.decode(bits)?;
        if symbol < 256 {
            if out.len() >= MAX_INFLATE {
                return Err(Error::new("deflate output is too large"));
            }
            out.push(u8::try_from(symbol).unwrap_or(0));
        } else if symbol == 256 {
            return Ok(());
        } else {
            let length = length_of(bits, symbol)?;
            let coded = dist.decode(bits)?;
            let distance = distance_of(bits, coded)?;
            copy_back(out, distance, length)?;
        }
    }
}

fn dynamic_trees(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), Error> {
    let lit_count = usize::try_from(bits.bits(5)? + 257).unwrap_or(0);
    let dist_count = usize::try_from(bits.bits(5)? + 1).unwrap_or(0);
    let code_count = usize::try_from(bits.bits(4)? + 4).unwrap_or(0);
    if lit_count > 286 || dist_count > 32 || code_count > 19 {
        return Err(Error::new("deflate dynamic header is invalid"));
    }
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let mut code_lengths = vec![0_u16; 19];
    for slot in ORDER.iter().take(code_count) {
        code_lengths[*slot] = u16::try_from(bits.bits(3)?).unwrap_or(0);
    }
    let coder = Huffman::from_lengths(&code_lengths)?;
    let all = read_code_lengths(bits, &coder, lit_count + dist_count)?;
    let lit = Huffman::from_lengths(&all[..lit_count])?;
    let dist = Huffman::from_lengths(&all[lit_count..])?;
    Ok((lit, dist))
}

fn read_code_lengths(bits: &mut Bits<'_>, coder: &Huffman, count: usize) -> Result<Vec<u16>, Error> {
    let mut lengths = Vec::with_capacity(count);
    while lengths.len() < count {
        match coder.decode(bits)? {
            symbol @ 0..=15 => lengths.push(symbol),
            16 => {
                let previous = *lengths
                    .last()
                    .ok_or_else(|| Error::new("deflate repeat has no previous length"))?;
                let repeat = 3 + usize::try_from(bits.bits(2)?).unwrap_or(0);
                push_repeat(&mut lengths, count, previous, repeat)?;
            }
            17 => {
                let repeat = 3 + usize::try_from(bits.bits(3)?).unwrap_or(0);
                push_repeat(&mut lengths, count, 0, repeat)?;
            }
            18 => {
                let repeat = 11 + usize::try_from(bits.bits(7)?).unwrap_or(0);
                push_repeat(&mut lengths, count, 0, repeat)?;
            }
            _ => return Err(Error::new("deflate code length symbol is invalid")),
        }
    }
    Ok(lengths)
}

fn push_repeat(lengths: &mut Vec<u16>, count: usize, value: u16, repeat: usize) -> Result<(), Error> {
    if lengths.len().saturating_add(repeat) > count {
        return Err(Error::new("deflate code length repeat overruns the alphabet"));
    }
    for _ in 0..repeat {
        lengths.push(value);
    }
    Ok(())
}

fn length_of(bits: &mut Bits<'_>, symbol: u16) -> Result<usize, Error> {
    const BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227,
        258,
    ];
    const EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    if !(257..=285).contains(&symbol) {
        return Err(Error::new("deflate length symbol is invalid"));
    }
    let index = usize::from(symbol - 257);
    let extra = if EXTRA[index] == 0 { 0 } else { bits.bits(EXTRA[index])? };
    Ok(usize::from(BASE[index]).saturating_add(usize::try_from(extra).unwrap_or(0)))
}

fn distance_of(bits: &mut Bits<'_>, symbol: u16) -> Result<usize, Error> {
    const BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097,
        6145, 8193, 12289, 16385, 24577,
    ];
    const EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
    ];
    let index = usize::from(symbol);
    if index >= BASE.len() {
        return Err(Error::new("deflate distance symbol is invalid"));
    }
    let extra = if EXTRA[index] == 0 { 0 } else { bits.bits(EXTRA[index])? };
    Ok(usize::from(BASE[index]).saturating_add(usize::try_from(extra).unwrap_or(0)))
}

fn copy_back(out: &mut Vec<u8>, distance: usize, length: usize) -> Result<(), Error> {
    if distance == 0 || distance > out.len() || length > MAX_INFLATE {
        return Err(Error::new("deflate distance is out of range"));
    }
    for _ in 0..length {
        if out.len() >= MAX_INFLATE {
            return Err(Error::new("deflate output is too large"));
        }
        let byte = out[out.len() - distance];
        out.push(byte);
    }
    Ok(())
}

fn fixed_lit_lengths() -> Vec<u16> {
    let mut lengths = vec![8_u16; 288];
    for length in &mut lengths[144..256] {
        *length = 9;
    }
    for length in &mut lengths[256..280] {
        *length = 7;
    }
    lengths
}

fn fixed_dist_lengths() -> Vec<u16> {
    vec![5_u16; 32]
}

struct Bits<'a> {
    data: &'a [u8],
    index: usize,
    bit: u8,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, Error> {
        let Some(byte) = self.data.get(self.index) else {
            return Err(Error::new("deflate input ended early"));
        };
        let value = u32::from((byte >> self.bit) & 1);
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.index += 1;
        }
        Ok(value)
    }

    fn bits(&mut self, count: u8) -> Result<u32, Error> {
        let mut value = 0_u32;
        for shift in 0..count {
            value |= self.bit()? << shift;
        }
        Ok(value)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.index += 1;
        }
    }

    fn take_bytes(&mut self, len: usize) -> Result<Vec<u8>, Error> {
        self.align();
        let end = self
            .index
            .checked_add(len)
            .ok_or_else(|| Error::new("deflate length overflow"))?;
        if end > self.data.len() {
            return Err(Error::new("deflate stored block overruns input"));
        }
        let out = self.data[self.index..end].to_vec();
        self.index = end;
        Ok(out)
    }
}

struct Huffman {
    entries: Vec<(u8, u16, u16)>,
}

impl Huffman {
    fn from_lengths(lengths: &[u16]) -> Result<Self, Error> {
        let mut counts = [0_u16; 16];
        for &length in lengths {
            if length > 15 {
                return Err(Error::new("huffman code is longer than 15 bits"));
            }
            if length > 0 {
                counts[usize::from(length)] = counts[usize::from(length)]
                    .checked_add(1)
                    .ok_or_else(|| Error::new("huffman length count overflow"))?;
            }
        }
        let mut next = [0_u16; 16];
        let mut code = 0_u16;
        for bits in 1..=15 {
            code = code
                .checked_add(counts[bits - 1])
                .ok_or_else(|| Error::new("huffman code overflow"))?
                .checked_shl(1)
                .ok_or_else(|| Error::new("huffman code overflow"))?;
            next[bits] = code;
        }
        let mut entries = Vec::new();
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let assigned = next[usize::from(length)];
            next[usize::from(length)] = assigned.saturating_add(1);
            let symbol = u16::try_from(symbol).map_err(|_| Error::new("huffman symbol does not fit"))?;
            let bits = u8::try_from(length).unwrap_or(0);
            entries.push((bits, assigned, symbol));
        }
        Ok(Self { entries })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        let mut code = 0_u16;
        for length in 1..=15 {
            let bit = u16::try_from(bits.bit()?).map_err(|_| Error::new("deflate bit does not fit"))?;
            code = code.checked_shl(1).ok_or_else(|| Error::new("deflate code overflow"))? | bit;
            if let Some((_, _, symbol)) = self
                .entries
                .iter()
                .find(|(entry_len, entry_code, _)| *entry_len == length && *entry_code == code)
            {
                return Ok(*symbol);
            }
        }
        Err(Error::new("deflate huffman code is not in the alphabet"))
    }
}
