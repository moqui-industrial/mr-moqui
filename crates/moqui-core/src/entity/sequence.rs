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
//!
//! Postgres claims the next bank with one `INSERT ... ON CONFLICT ... RETURNING`.
//! MySQL runs one `INSERT ... ON DUPLICATE KEY UPDATE`, then reads the row it now holds with
//! `SELECT ... FOR UPDATE`. A bare locking read on a missing row would take a gap lock.
//! `seqNum` stays the last reserved id. There is no native sequence.

use moqui_db::sql::{is_mysql, is_sqlite, quote_ident};
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

#[derive(Clone)]
struct SeqSql {
    mysql: bool,
    lock_sql: String,
    mysql_upsert_sql: String,
    upsert_sql: String,
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
            .or_insert(SequenceBank { current: 0, end: -1 });
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
    let table = quote_ident(dialect, &entity.table_name);
    let name_col = quote_ident(dialect, "seqName");
    let num_col = quote_ident(dialect, "seqNum");
    let mysql = is_mysql(dialect);
    // SQLite has the same upsert and RETURNING, but its two-argument max() is GREATEST.
    let greatest = if is_sqlite(dialect) { "max" } else { "GREATEST" };
    Ok(SeqSql {
        mysql,
        lock_sql: format!("SELECT {num_col} FROM {table} WHERE {name_col} = ? FOR UPDATE"),
        mysql_upsert_sql: format!(
            "INSERT INTO {table} ({name_col}, {num_col}) VALUES (?, ?) ON DUPLICATE KEY UPDATE {num_col} = GREATEST({num_col} + ?, ?)"
        ),
        upsert_sql: format!(
            "INSERT INTO {table} ({name_col}, {num_col}) VALUES (?, ?) ON CONFLICT ({name_col}) DO UPDATE SET {num_col} = {greatest}({table}.{num_col} + ?, ?) RETURNING {num_col}"
        ),
    })
}

fn refill(
    tx: &crate::transaction::TransactionManager,
    prepared: &SeqSql,
    seq_name: &str,
    bank_size: u64,
    hint: i64,
) -> Result<SequenceBank, Error> {
    let prepared = prepared.clone();
    let seq_name = seq_name.to_string();
    let size = bank_width(bank_size);
    with_reserve_tx(tx, move |conn| {
        let (first_end, increment, hint_end) = refill_targets(size, hint);
        let end = if prepared.mysql {
            mysql_end(conn, &prepared, &seq_name, first_end, increment, hint_end)?
        } else {
            postgres_end(conn, &prepared, &seq_name, first_end, increment, hint_end)?
        };
        Ok(bank_from_end(end, size))
    })
}

/// `end` is the stored `seqNum`. The bank starts `size` below it, matching [`fill_bank`].
fn bank_from_end(end: i64, size: i64) -> SequenceBank {
    SequenceBank {
        current: end.saturating_sub(size),
        end,
    }
}

fn bank_width(bank_size: u64) -> i64 {
    i64::try_from(bank_size.max(1)).unwrap_or(i64::MAX)
}

/// First insert stores `first_end`. A conflict stores `GREATEST(old + increment, hint_end)`.
fn refill_targets(size: i64, hint: i64) -> (i64, i64, i64) {
    let first_end = SEQUENCE_START.max(hint).saturating_add(size);
    let increment = size.saturating_add(1);
    let hint_end = hint.saturating_add(size);
    (first_end, increment, hint_end)
}

fn postgres_end(
    conn: &mut dyn DbConnection,
    prepared: &SeqSql,
    seq_name: &str,
    first_end: i64,
    increment: i64,
    hint_end: i64,
) -> Result<i64, Error> {
    let rows = query_conn(
        conn,
        &prepared.upsert_sql,
        &[
            string_param(seq_name),
            long_param(first_end),
            long_param(increment),
            long_param(hint_end),
        ],
    )?;
    one_seq_num(&rows, seq_name)
}

fn mysql_end(
    conn: &mut dyn DbConnection,
    prepared: &SeqSql,
    seq_name: &str,
    first_end: i64,
    increment: i64,
    hint_end: i64,
) -> Result<i64, Error> {
    let name = string_param(seq_name);
    super::exec::execute_conn(
        conn,
        &prepared.mysql_upsert_sql,
        &[
            name.clone(),
            long_param(first_end),
            long_param(increment),
            long_param(hint_end),
        ],
    )?;
    let rows = query_conn(conn, &prepared.lock_sql, std::slice::from_ref(&name))?;
    one_seq_num(&rows, seq_name)
}

fn one_seq_num(rows: &[crate::driver::DbRow], seq_name: &str) -> Result<i64, Error> {
    rows.first()
        .and_then(|row| row.columns.first())
        .and_then(|(_, value)| as_i64(value))
        .ok_or_else(|| Error::new(format!("sequence refill for {seq_name} returned no seqNum")))
}

fn string_param(seq_name: &str) -> Value {
    basic_convert(&Value::string(seq_name), "String").value
}

fn long_param(number: i64) -> Value {
    basic_convert(&Value::I64(number), "Long").value
}

fn as_i64(value: &Value) -> Option<i64> {
    match basic_convert(value, "Long").value {
        Value::I64(number) => Some(number),
        _ => None,
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
    fill_random(&mut bytes)?;
    let number = u64::from_le_bytes(bytes);
    Ok((number as f64) / (u64::MAX as f64))
}

pub(crate) fn uuid_v4() -> Result<String, Error> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format_uuid(&bytes))
}

fn fill_random(bytes: &mut [u8]) -> Result<(), Error> {
    getrandom::fill(bytes).map_err(|err| Error::new(format!("could not read random bytes: {err}")))
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

    #[test]
    fn sql_end_matches_fill_bank() {
        let (first_end, _, _) = refill_targets(50, 0);
        assert_eq!(bank_from_end(first_end, 50), fill_bank(None, 50, 0));
        let old = 100_050_i64;
        let (_, increment, hint_end) = refill_targets(50, 0);
        let bumped = old.saturating_add(increment).max(hint_end);
        assert_eq!(bank_from_end(bumped, 50), fill_bank(Some(old), 50, 0));
        let hint = 100_090;
        let (_, increment, hint_end) = refill_targets(50, hint);
        let jumped = old.saturating_add(increment).max(hint_end);
        assert_eq!(bank_from_end(jumped, 50), fill_bank(Some(old), 50, hint));
    }

    #[test]
    fn refill_sql_is_insert_on_conflict_or_for_update() {
        let framework = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../framework");
        let conf = moqui_lib::node::MNode::parse_file(framework.join("conf/MoquiDefaultConf.xml")).unwrap();
        let model = moqui_db::model::EntityModel::load_directory(&conf, &framework.join("entity")).unwrap();
        let dialects = moqui_db::dialect::Dialects::load(&conf).unwrap();
        let entity = model.entity("moqui.entity.SequenceValueItem").unwrap();
        let postgres = seq_sql(entity, dialects.get("postgres").unwrap()).unwrap();
        assert!(!postgres.mysql);
        assert_eq!(
            postgres.upsert_sql,
            "INSERT INTO \"SequenceValueItem\" (\"seqName\", \"seqNum\") VALUES (?, ?) ON CONFLICT (\"seqName\") DO UPDATE SET \"seqNum\" = GREATEST(\"SequenceValueItem\".\"seqNum\" + ?, ?) RETURNING \"seqNum\""
        );
        let mysql = seq_sql(entity, dialects.get("mysql").unwrap()).unwrap();
        assert!(mysql.mysql);
        assert_eq!(
            mysql.lock_sql,
            "SELECT `seqNum` FROM `SequenceValueItem` WHERE `seqName` = ? FOR UPDATE"
        );
        assert_eq!(
            mysql.mysql_upsert_sql,
            "INSERT INTO `SequenceValueItem` (`seqName`, `seqNum`) VALUES (?, ?) ON DUPLICATE KEY UPDATE `seqNum` = GREATEST(`seqNum` + ?, ?)"
        );
    }
}
