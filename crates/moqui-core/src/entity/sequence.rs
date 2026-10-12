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

//! In-memory sequenced-id bank. Refill uses a reserve connection.

use std::fs::File;
use std::io::Read;

use moqui_db::sql::{insert, select, update, Condition, FieldRef, LimitOffset};
use moqui_lib::error::Error;
use moqui_lib::object::{basic_convert, Value};

use super::exec::{query_conn, with_reserve_tx};
use super::facade::EntityFacade;
use crate::driver::DbConnection;

const SEQUENCE_START: i64 = 100_000;
const SEQUENCE_ENTITY: &str = "moqui.entity.SequenceValueItem";

/// One bank. `end` is the last reserved id, inclusive. The next id to issue is `current`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceBank {
    pub current: i64,
    pub end: i64,
}

/// Fill a bank. No stored row starts at 100000.
///
/// A stored `seqNum` is the last id reserved for this name. The next id is one past it.
/// The bank reserves `bank_size + 1` ids, matching `EntityFacadeImpl.dbSequencedIdPrimary`.
pub fn fill_bank(stored_last: Option<i64>, bank_size: u64, current_hint: i64) -> SequenceBank {
    let size = i64::try_from(bank_size.max(1)).unwrap_or(i64::MAX);
    let start = match stored_last {
        Some(stored) => stored.saturating_add(1).max(current_hint),
        None => SEQUENCE_START.max(current_hint),
    };
    SequenceBank {
        current: start,
        end: start.saturating_add(size),
    }
}

/// How many ids to skip. `roll` is in `0.0..=1.0`. The step is at least 1.
pub fn stagger_step(stagger_max: u64, roll: f64) -> i64 {
    let max = u64::max(stagger_max, 1);
    let scaled = roll.clamp(0.0, 1.0) * (max as f64);
    let rounded = scaled.round();
    let step = if rounded.is_sign_negative() || rounded > (i64::MAX as f64) {
        1
    } else {
        rounded as i64
    };
    step.max(1)
}

/// Issue one id, or `None` when `current` is past `end`.
pub fn take_id(bank: &mut SequenceBank, stagger_max: u64, roll: f64) -> Option<i64> {
    if bank.current > bank.end {
        return None;
    }
    let id = bank.current;
    bank.current = bank.current.saturating_add(stagger_step(stagger_max, roll));
    Some(id)
}

struct PrimarySpec {
    seq_name: String,
    bank_size: u64,
    stagger: u64,
    use_uuid: bool,
    prefix: Option<String>,
    pk_field: String,
}

struct SeqSql {
    select_sql: String,
    insert_sql: String,
    update_sql: String,
}

pub(crate) fn assign_primary(facade: &EntityFacade, entity_name: &str) -> Result<(String, String), Error> {
    let spec = facade.with_entity(entity_name, |entity, _, _| {
        let pk_field = entity
            .pk_field_names()
            .first()
            .copied()
            .ok_or_else(|| Error::new(format!("{} has no primary key", entity.full_name)))?
            .to_string();
        Ok(PrimarySpec {
            seq_name: entity.full_name.clone(),
            bank_size: entity.sequence_bank_size,
            stagger: entity.sequence_primary_stagger,
            use_uuid: entity.sequence_primary_use_uuid,
            prefix: entity.sequence_primary_prefix.clone(),
            pk_field,
        })
    })?;
    if spec.use_uuid {
        return Ok((spec.pk_field, apply_prefix(spec.prefix.as_deref(), &uuid_v4()?)));
    }
    let prepared = facade.with_entity(SEQUENCE_ENTITY, |entity, dialect, _| seq_sql(entity, dialect))?;
    let tx = facade.transactions_for(SEQUENCE_ENTITY)?;
    let mut banks = facade.inner.banks.lock().unwrap_or_else(|err| err.into_inner());
    loop {
        let bank = banks
            .entry(spec.seq_name.clone())
            .or_insert(SequenceBank { current: 0, end: 0 });
        let roll = roll_for(spec.stagger)?;
        if let Some(id) = take_id(bank, spec.stagger, roll) {
            return Ok((spec.pk_field, apply_prefix(spec.prefix.as_deref(), &id.to_string())));
        }
        let hint = bank.current;
        let filled = refill(&tx, &prepared, &spec.seq_name, spec.bank_size, hint)?;
        banks.insert(spec.seq_name.clone(), filled);
    }
}

fn seq_sql(entity: &moqui_db::model::EntityDefinition, dialect: &moqui_db::dialect::Dialect) -> Result<SeqSql, Error> {
    let condition = Condition::equals("seqName", Value::Null);
    Ok(SeqSql {
        select_sql: select(
            entity,
            dialect,
            &[FieldRef::new("seqNum")],
            Some(&condition),
            &[],
            LimitOffset::default(),
        )?,
        insert_sql: insert(entity, dialect, &["seqName", "seqNum"])?,
        update_sql: update(entity, dialect, &["seqNum"], Some(&condition))?,
    })
}

fn refill(
    tx: &crate::transaction::TransactionManager,
    prepared: &SeqSql,
    seq_name: &str,
    bank_size: u64,
    hint: i64,
) -> Result<SequenceBank, Error> {
    let select_sql = prepared.select_sql.clone();
    let insert_sql = prepared.insert_sql.clone();
    let update_sql = prepared.update_sql.clone();
    let seq_name = seq_name.to_string();
    with_reserve_tx(tx, move |conn| {
        let (exists, stored) = read_seq(conn, &select_sql, &seq_name)?;
        let bank = fill_bank(stored, bank_size, hint);
        write_seq(conn, &insert_sql, &update_sql, &seq_name, bank.end, exists)?;
        Ok(bank)
    })
}

fn read_seq(conn: &mut dyn DbConnection, sql: &str, seq_name: &str) -> Result<(bool, Option<i64>), Error> {
    let name = basic_convert(&Value::string(seq_name), "String").value;
    let rows = query_conn(conn, sql, &[name])?;
    let Some(row) = rows.first() else {
        return Ok((false, None));
    };
    let number = row.columns.first().and_then(|(_, value)| as_i64(value));
    Ok((true, number))
}

fn as_i64(value: &Value) -> Option<i64> {
    match basic_convert(value, "Long").value {
        Value::I64(number) => Some(number),
        _ => None,
    }
}

fn write_seq(
    conn: &mut dyn DbConnection,
    insert_sql: &str,
    update_sql: &str,
    seq_name: &str,
    next_id: i64,
    exists: bool,
) -> Result<(), Error> {
    let name = basic_convert(&Value::string(seq_name), "String").value;
    let number = basic_convert(&Value::I64(next_id), "Long").value;
    if exists {
        super::exec::execute_conn(conn, update_sql, &[number, name])?;
        return Ok(());
    }
    match super::exec::execute_conn(conn, insert_sql, &[name.clone(), number.clone()]) {
        Ok(_) => Ok(()),
        Err(_) => {
            super::exec::execute_conn(conn, update_sql, &[number, name])?;
            Ok(())
        }
    }
}

fn apply_prefix(prefix: Option<&str>, id: &str) -> String {
    match prefix {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}{id}"),
        _ => id.to_string(),
    }
}

fn roll_for(stagger: u64) -> Result<f64, Error> {
    if stagger <= 1 {
        return Ok(0.0);
    }
    unit_roll()
}

fn unit_roll() -> Result<f64, Error> {
    let mut bytes = [0_u8; 8];
    let mut file =
        File::open("/dev/urandom").map_err(|err| Error::new("could not read /dev/urandom").with_source(err))?;
    file.read_exact(&mut bytes)
        .map_err(|err| Error::new("could not read /dev/urandom").with_source(err))?;
    let number = u64::from_le_bytes(bytes);
    Ok((number as f64) / (u64::MAX as f64))
}

pub(crate) fn uuid_v4() -> Result<String, Error> {
    let mut bytes = [0_u8; 16];
    let mut file =
        File::open("/dev/urandom").map_err(|err| Error::new("could not read /dev/urandom").with_source(err))?;
    file.read_exact(&mut bytes)
        .map_err(|err| Error::new("could not read /dev/urandom").with_source(err))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format_uuid(&bytes))
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if index == 4 || index == 6 || index == 8 || index == 10 {
            out.push('-');
        }
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_starts_at_100000_and_end_is_inclusive() {
        let bank = fill_bank(None, 50, 0);
        assert_eq!(bank.current, 100_000);
        assert_eq!(bank.end, 100_050);
        let mut bank = bank;
        assert_eq!(take_id(&mut bank, 1, 0.0), Some(100_000));
        assert_eq!(bank.current, 100_001);
        bank.current = bank.end;
        assert_eq!(take_id(&mut bank, 1, 0.0), Some(100_050));
        assert_eq!(take_id(&mut bank, 1, 0.0), None);
    }

    #[test]
    fn refill_starts_one_past_the_stored_last_id() {
        let bank = fill_bank(Some(100_050), 50, 0);
        assert_eq!(bank.current, 100_051);
        assert_eq!(bank.end, 100_101);
        let jumped = fill_bank(Some(100_050), 50, 100_053);
        assert_eq!(jumped.current, 100_053);
        assert_eq!(jumped.end, 100_103);
    }

    #[test]
    fn stagger_step_rounds_and_is_at_least_one() {
        assert_eq!(stagger_step(4, 0.9), 4);
        assert_eq!(stagger_step(4, 0.0), 1);
        assert_eq!(stagger_step(1, 0.2), 1);
    }
}
