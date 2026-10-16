//! Deterministic connection closures may not start invisible transactions or ask the DB clock.
use crate::driver::{DbCancel, DbConnection, DbRow};
use moqui_lib::error::Error;
use moqui_lib::object::Value;
use std::time::Duration;

pub(super) struct CheckedConnection<'a>(pub &'a mut dyn DbConnection);

fn check_sql(sql: &str) -> Result<(), Error> {
    let first = sql.split_whitespace().next().unwrap_or("").trim_end_matches(';');
    if ["begin", "commit", "rollback", "start", "end"]
        .iter()
        .any(|word| first.eq_ignore_ascii_case(word))
    {
        return boundary();
    }
    const CLOCK_WORDS: &[&str] = &[
        "now",
        "clock_timestamp",
        "statement_timestamp",
        "transaction_timestamp",
        "timeofday",
        "current_timestamp",
        "localtimestamp",
        "current_date",
        "current_time",
        "localtime",
    ];
    // Deliberately conservative, not a dialect SQL parser. Bind literals instead of embedding
    // clock words in custom SQL (including quoted identifiers/literals).
    if sql
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| CLOCK_WORDS.iter().any(|clock| word.eq_ignore_ascii_case(clock)))
    {
        return Err(Error::new(
            "database wall-clock expressions are unsupported in deterministic mode; bind the framework logical timestamp",
        ));
    }
    Ok(())
}

fn boundary() -> Result<(), Error> {
    Err(Error::new(
        "raw connection transaction boundaries are unsupported in deterministic mode; use TransactionManager",
    ))
}

impl DbConnection for CheckedConnection<'_> {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
        check_sql(sql)?;
        self.0.execute(sql, params)
    }
    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        check_sql(sql)?;
        self.0.query(sql, params)
    }
    fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
        check_sql(sql)?;
        self.0.query_once(sql, params)
    }
    fn open_stream(&mut self, sql: &str, params: &[Value]) -> Result<(), Error> {
        check_sql(sql)?;
        self.0.open_stream(sql, params)
    }
    fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
        self.0.next_row()
    }
    fn close_stream(&mut self) -> Result<(), Error> {
        self.0.close_stream()
    }
    fn stream_open(&self) -> bool {
        self.0.stream_open()
    }
    fn begin(&mut self) -> Result<(), Error> {
        boundary()
    }
    fn commit(&mut self) -> Result<(), Error> {
        boundary()
    }
    fn rollback(&mut self) -> Result<(), Error> {
        boundary()
    }
    fn savepoint(&mut self, name: &str) -> Result<(), Error> {
        self.0.savepoint(name)
    }
    fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), Error> {
        self.0.rollback_to_savepoint(name)
    }
    fn release_savepoint(&mut self, name: &str) -> Result<(), Error> {
        self.0.release_savepoint(name)
    }
    fn canceller(&self) -> Box<dyn DbCancel> {
        self.0.canceller()
    }
    fn set_statement_timeout(&mut self, remaining: Option<Duration>) -> Result<(), Error> {
        self.0.set_statement_timeout(remaining)
    }
}
