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

//! Database driver boundary. No Postgres or MySQL types.
//!
//! SQL text uses `?` placeholders outside quotes. A driver rewrites those for
//! its client. `query` and `execute` may cache a prepared statement, cap 100,
//! keyed by SQL text. [`DbConnection::query_once`] must not take a cache slot.
//! An open stream pins the connection until [`DbConnection::close_stream`].

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use moqui_lib::error::Error;
use moqui_lib::object::Value;

/// One result row. Column order is the select list order.
#[derive(Clone, Debug, PartialEq)]
pub struct DbRow {
    pub columns: Vec<(String, Value)>,
}

impl DbRow {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.columns
            .iter()
            .find(|(column, _)| column == name)
            .map(|(_, value)| value)
    }
}

/// Opens connections. The pool calls this. One connection is one client.
pub trait DbDriver: Send + Sync {
    fn connect(&self) -> Result<Box<dyn DbConnection>, Error>;
}

/// One checked-out client. `Send` so a transaction can park it off the thread.
///
/// `begin` starts a transaction on this client. It does not check a client out
/// of a pool. Commit and rollback end that transaction. Savepoint names are
/// chosen by the caller.
pub trait DbConnection: Send {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error>;

    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error>;

    fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error>;

    fn open_stream(&mut self, sql: &str, params: &[Value]) -> Result<(), Error>;

    fn next_row(&mut self) -> Result<Option<DbRow>, Error>;

    fn close_stream(&mut self) -> Result<(), Error>;

    fn stream_open(&self) -> bool;

    fn begin(&mut self) -> Result<(), Error>;

    fn commit(&mut self) -> Result<(), Error>;

    fn rollback(&mut self) -> Result<(), Error>;

    fn savepoint(&mut self, name: &str) -> Result<(), Error>;

    fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), Error>;

    fn release_savepoint(&mut self, name: &str) -> Result<(), Error>;

    /// A handle the deadline thread can call while this connection is borrowed.
    fn canceller(&self) -> Box<dyn DbCancel>;

    /// Remaining statement time. `None` clears it. The default does nothing.
    ///
    /// Postgres sets `statement_timeout`. MySQL sets `max_execution_time`. The manager calls this
    /// only when `use-statement-timeout` is on.
    fn set_statement_timeout(&mut self, remaining: Option<Duration>) -> Result<(), Error> {
        let _ = remaining;
        Ok(())
    }
}

/// A database named by its `database-list` conf name and its address, as Java's `getConfConnection` takes from the
/// `entity_ds_*` entries of a map. The connection is its own, not one from the entity pool.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ConfDb {
    /// The `database-list` name, such as `postgres` or `mysql8`.
    pub db_type: String,
    pub host: String,
    /// Empty is the database's default port.
    pub port: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

impl std::fmt::Debug for ConfDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfDb")
            .field("db_type", &self.db_type)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Makes a driver for a [`ConfDb`]. The host installs one; it knows the database clients.
pub type ConfDriverFactory = dyn Fn(&ConfDb) -> Result<Arc<dyn DbDriver>, Error> + Send + Sync;

fn conf_driver_slot() -> &'static RwLock<Option<Arc<ConfDriverFactory>>> {
    static SLOT: RwLock<Option<Arc<ConfDriverFactory>>> = RwLock::new(None);
    &SLOT
}

/// Use `factory` for every [`conf_driver`] in this process.
pub fn install_conf_driver_factory(factory: Arc<ConfDriverFactory>) {
    *conf_driver_slot().write().unwrap_or_else(PoisonError::into_inner) = Some(factory);
}

/// A driver for the database `conf` names.
///
/// # Errors
///
/// When the host installed no factory, or the factory cannot make a driver for the conf name.
pub fn conf_driver(conf: &ConfDb) -> Result<Arc<dyn DbDriver>, Error> {
    let factory = conf_driver_slot()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| Error::new("no database client for conf connections is installed in this host"))?;
    factory(conf)
}

/// Cancels the statement running on another thread. A connection with no
/// active statement returns success.
pub trait DbCancel: Send + Sync {
    fn cancel(&self) -> Result<(), Error>;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;

    #[test]
    fn conf_db_debug_leaves_out_the_password() {
        let conf = ConfDb {
            db_type: "postgres".to_string(),
            user: "app".to_string(),
            password: "s3cret-pass".to_string(),
            ..ConfDb::default()
        };
        let text = format!("{conf:?}");
        assert!(!text.contains("s3cret-pass"), "{text}");
        assert!(text.contains("postgres") && text.contains("app"), "{text}");
    }

    struct FakeDriver;

    struct FakeConn {
        cancelled: Arc<AtomicBool>,
        streaming: bool,
        rows_left: i64,
    }

    impl DbConnection for FakeConn {
        fn execute(&mut self, _sql: &str, _params: &[Value]) -> Result<u64, Error> {
            Ok(1)
        }

        fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            self.query_once(sql, params)
        }

        fn query_once(&mut self, _sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            Ok(vec![DbRow {
                columns: vec![("n".to_string(), params.first().cloned().unwrap_or(Value::Null))],
            }])
        }

        fn open_stream(&mut self, _sql: &str, _params: &[Value]) -> Result<(), Error> {
            self.streaming = true;
            self.rows_left = 2;
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
            if !self.streaming || self.rows_left == 0 {
                return Ok(None);
            }
            self.rows_left -= 1;
            Ok(Some(DbRow {
                columns: vec![("n".to_string(), Value::I64(self.rows_left))],
            }))
        }

        fn close_stream(&mut self) -> Result<(), Error> {
            self.streaming = false;
            self.rows_left = 0;
            Ok(())
        }

        fn stream_open(&self) -> bool {
            self.streaming
        }

        fn begin(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn commit(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn savepoint(&mut self, _name: &str) -> Result<(), Error> {
            Ok(())
        }

        fn rollback_to_savepoint(&mut self, _name: &str) -> Result<(), Error> {
            Ok(())
        }

        fn release_savepoint(&mut self, _name: &str) -> Result<(), Error> {
            Ok(())
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            let flag = Arc::clone(&self.cancelled);
            Box::new(FlagCancel(flag))
        }
    }

    struct FlagCancel(Arc<AtomicBool>);

    impl DbCancel for FlagCancel {
        fn cancel(&self) -> Result<(), Error> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    impl DbDriver for FakeDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
            Ok(Box::new(FakeConn {
                cancelled: Arc::new(AtomicBool::new(false)),
                streaming: false,
                rows_left: 0,
            }))
        }
    }

    #[test]
    fn trait_objects_query_stream_and_cancel() {
        let driver: Arc<dyn DbDriver> = Arc::new(FakeDriver);
        let mut conn = driver.connect().unwrap();
        let cancel = conn.canceller();
        cancel.cancel().unwrap();
        let rows = conn.query_once("select ?", &[Value::I64(3)]).unwrap();
        assert_eq!(rows[0].get("n"), Some(&Value::I64(3)));
        conn.open_stream("select n", &[]).unwrap();
        assert!(conn.stream_open());
        assert!(conn.next_row().unwrap().is_some());
        assert!(conn.next_row().unwrap().is_some());
        assert!(conn.next_row().unwrap().is_none());
        conn.close_stream().unwrap();
        assert!(!conn.stream_open());
    }
}
