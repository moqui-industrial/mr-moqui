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

//! Sync MySQL client behind [`moqui_core::driver::DbDriver`].

#[cfg(feature = "mysql")]
mod driver {
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Arc;

    use moqui_core::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use moqui_lib::error::Error;
    use moqui_lib::object::{basic_convert, Value};
    use mysql::consts::{ColumnFlags, ColumnType};
    use mysql::prelude::Queryable;
    use mysql::Value as MysqlValue;

    const STATEMENT_CACHE_CAP: usize = 100;
    /// MySQL `character_set` 63 is `binary`.
    const CHARSET_BINARY: u16 = 63;
    /// `ER_UNSUPPORTED_PS`: the statement cannot be prepared.
    const ER_UNSUPPORTED_PS: u16 = 1295;

    #[derive(Clone)]
    struct Endpoint {
        host: String,
        port: u16,
        username: String,
        password: String,
        database: String,
        tls: bool,
    }

    impl Endpoint {
        fn opts(&self) -> mysql::OptsBuilder {
            let database = if self.database.is_empty() {
                None
            } else {
                Some(self.database.as_str())
            };
            mysql::OptsBuilder::new()
                .ip_or_hostname(Some(self.host.as_str()))
                .tcp_port(self.port)
                .user(Some(self.username.as_str()))
                .pass(Some(self.password.as_str()))
                .db_name(database)
                .prefer_socket(false)
                .stmt_cache_size(0usize)
                .ssl_opts(self.tls.then(mysql::SslOpts::default))
                .init(vec!["SET NAMES utf8mb4"])
        }
    }

    struct KillState {
        endpoint: Endpoint,
        connection_id: AtomicU32,
        busy: AtomicBool,
    }

    struct BusyFlag<'a> {
        flag: &'a AtomicBool,
        keep: bool,
    }

    impl<'a> BusyFlag<'a> {
        fn arm(flag: &'a AtomicBool) -> Self {
            flag.store(true, Ordering::Release);
            Self { flag, keep: false }
        }

        fn keep(&mut self) {
            self.keep = true;
        }
    }

    impl Drop for BusyFlag<'_> {
        fn drop(&mut self) {
            if !self.keep {
                self.flag.store(false, Ordering::Release);
            }
        }
    }

    #[derive(Default)]
    struct StatementCache {
        map: HashMap<String, mysql::Statement>,
        order: VecDeque<String>,
    }

    impl StatementCache {
        fn len(&self) -> usize {
            self.map.len()
        }

        fn contains(&self, sql: &str) -> bool {
            self.map.contains_key(sql)
        }

        fn touch(&mut self, sql: &str) -> Option<&mysql::Statement> {
            let index = self.order.iter().position(|item| item == sql)?;
            let key = self.order.remove(index)?;
            self.order.push_back(key);
            self.map.get(sql)
        }

        fn insert(&mut self, sql: String, statement: mysql::Statement) -> Option<mysql::Statement> {
            if self.map.contains_key(&sql) {
                self.order.retain(|item| item != &sql);
                self.order.push_back(sql.clone());
                return self.map.insert(sql, statement);
            }
            let evicted = if self.map.len() >= STATEMENT_CACHE_CAP {
                self.order.pop_front().and_then(|old| self.map.remove(&old))
            } else {
                None
            };
            self.order.push_back(sql.clone());
            self.map.insert(sql, statement);
            evicted
        }

        fn into_statements(self) -> impl Iterator<Item = mysql::Statement> {
            self.map.into_values()
        }
    }

    /// One MySQL or MariaDB server. Each [`DbDriver::connect`] opens one connection.
    pub struct MysqlDriver {
        endpoint: Endpoint,
    }

    impl MysqlDriver {
        pub fn new(
            host: impl Into<String>,
            port: u16,
            username: impl Into<String>,
            password: impl Into<String>,
            database: impl Into<String>,
            tls: bool,
        ) -> Self {
            Self {
                endpoint: Endpoint {
                    host: host.into(),
                    port,
                    username: username.into(),
                    password: password.into(),
                    database: database.into(),
                    tls,
                },
            }
        }

        /// Opens one [`mysql::Conn`]. The crate pool is not used.
        pub fn connect_mysql(&self) -> Result<MysqlConn, Error> {
            let conn = mysql::Conn::new(self.endpoint.opts()).map_err(mysql_err)?;
            let kill = Arc::new(KillState {
                endpoint: self.endpoint.clone(),
                connection_id: AtomicU32::new(conn.connection_id()),
                busy: AtomicBool::new(false),
            });
            Ok(MysqlConn {
                stream: None,
                statements: StatementCache::default(),
                kill,
                conn: Box::new(conn),
            })
        }
    }

    impl DbDriver for MysqlDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
            Ok(Box::new(self.connect_mysql()?))
        }
    }

    /// One MySQL connection. `stream` is dropped before `conn`.
    pub struct MysqlConn {
        stream: Option<mysql::QueryResult<'static, 'static, 'static, mysql::Binary>>,
        statements: StatementCache,
        kill: Arc<KillState>,
        conn: Box<mysql::Conn>,
    }

    impl MysqlConn {
        pub fn statement_cache_len(&self) -> usize {
            self.statements.len()
        }

        fn ensure_idle(&self) -> Result<(), Error> {
            if self.stream.is_some() {
                Err(Error::new("mysql stream is open"))
            } else {
                Ok(())
            }
        }

        fn prep_cached(&mut self, sql: &str) -> Result<(), mysql::Error> {
            if self.statements.contains(sql) {
                return Ok(());
            }
            let statement = self.conn.prep(sql)?;
            if let Some(old) = self.statements.insert(sql.to_owned(), statement) {
                self.conn.close(old)?;
            }
            Ok(())
        }

        fn cached_rows(&mut self, sql: &str, params: mysql::Params) -> Result<Vec<DbRow>, Error> {
            let MysqlConn { conn, statements, .. } = self;
            let Some(statement) = statements.touch(sql) else {
                return Err(Error::new("mysql statement cache missed"));
            };
            let result = conn.exec_iter(statement, params).map_err(mysql_err)?;
            collect_rows(result)
        }

        fn cached_drain(&mut self, sql: &str, params: mysql::Params) -> Result<(), Error> {
            let MysqlConn { conn, statements, .. } = self;
            let Some(statement) = statements.touch(sql) else {
                return Err(Error::new("mysql statement cache missed"));
            };
            let result = conn.exec_iter(statement, params).map_err(mysql_err)?;
            drain(result)
        }

        fn once_rows(&mut self, sql: &str, params_empty: bool, params: mysql::Params) -> Result<Vec<DbRow>, Error> {
            let statement = match self.conn.prep(sql) {
                Ok(statement) => statement,
                Err(err) if params_empty && unsupported_prepared(&err) => {
                    let result = self.conn.query_iter(sql).map_err(mysql_err)?;
                    return collect_rows(result);
                }
                Err(err) => return Err(mysql_err(err)),
            };
            let rows = match self.conn.exec_iter(&statement, params) {
                Ok(result) => collect_rows(result),
                Err(err) => Err(mysql_err(err)),
            };
            let closed = self.conn.close(statement);
            match (rows, closed) {
                (Ok(rows), Ok(())) => Ok(rows),
                (Err(err), _) => Err(err),
                (_, Err(err)) => Err(mysql_err(err)),
            }
        }

        fn rows(&mut self, sql: &str, params: &[Value], cache: bool) -> Result<Vec<DbRow>, Error> {
            self.ensure_idle()?;
            let mysql_params = to_params(params)?;
            let kill = Arc::clone(&self.kill);
            let _busy = BusyFlag::arm(&kill.busy);
            if !cache {
                return self.once_rows(sql, params.is_empty(), mysql_params);
            }
            match self.prep_cached(sql) {
                Ok(()) => self.cached_rows(sql, mysql_params),
                Err(err) if params.is_empty() && unsupported_prepared(&err) => {
                    let result = self.conn.query_iter(sql).map_err(mysql_err)?;
                    collect_rows(result)
                }
                Err(err) => Err(mysql_err(err)),
            }
        }

        fn stream_result(
            &mut self,
            sql: &str,
            params: mysql::Params,
        ) -> Result<mysql::QueryResult<'static, 'static, 'static, mysql::Binary>, Error> {
            let MysqlConn { conn, statements, .. } = self;
            let Some(statement) = statements.touch(sql) else {
                return Err(Error::new("mysql statement cache missed"));
            };
            let result = conn.exec_iter(statement, params).map_err(mysql_err)?;
            // SAFETY: `result` borrows the `Conn` in `self.conn`'s heap box. The
            // caller stores it in `self.stream` and does not use `conn` again until
            // that field is dropped. `Drop` drops `stream` before `conn`. Moving
            // `MysqlConn` does not move the boxed `Conn`.
            let result = unsafe {
                std::mem::transmute::<
                    mysql::QueryResult<'_, '_, '_, mysql::Binary>,
                    mysql::QueryResult<'static, 'static, 'static, mysql::Binary>,
                >(result)
            };
            Ok(result)
        }

        fn command(&mut self, sql: String) -> Result<(), Error> {
            self.ensure_idle()?;
            let kill = Arc::clone(&self.kill);
            let _busy = BusyFlag::arm(&kill.busy);
            self.conn.query_drop(sql).map_err(mysql_err)
        }
    }

    impl Drop for MysqlConn {
        fn drop(&mut self) {
            self.kill.busy.store(false, Ordering::Release);
            drop(self.stream.take());
            let statements = std::mem::take(&mut self.statements);
            for statement in statements.into_statements() {
                let _ = self.conn.close(statement);
            }
        }
    }

    impl DbConnection for MysqlConn {
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
            self.ensure_idle()?;
            let mysql_params = to_params(params)?;
            let kill = Arc::clone(&self.kill);
            let _busy = BusyFlag::arm(&kill.busy);
            match self.prep_cached(sql) {
                Ok(()) => {
                    self.cached_drain(sql, mysql_params)?;
                    Ok(self.conn.affected_rows())
                }
                Err(err) if params.is_empty() && unsupported_prepared(&err) => {
                    self.conn.query_drop(sql).map_err(mysql_err)?;
                    Ok(self.conn.affected_rows())
                }
                Err(err) => Err(mysql_err(err)),
            }
        }

        fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            self.rows(sql, params, true)
        }

        fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            self.rows(sql, params, false)
        }

        fn open_stream(&mut self, sql: &str, params: &[Value]) -> Result<(), Error> {
            self.ensure_idle()?;
            let mysql_params = to_params(params)?;
            let kill = Arc::clone(&self.kill);
            let mut busy = BusyFlag::arm(&kill.busy);
            self.prep_cached(sql).map_err(mysql_err)?;
            let result = self.stream_result(sql, mysql_params)?;
            self.stream = Some(result);
            busy.keep();
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
            let Some(stream) = self.stream.as_mut() else {
                return Ok(None);
            };
            match stream.next() {
                Some(Ok(row)) => Ok(Some(map_row(row)?)),
                Some(Err(err)) => Err(mysql_err(err)),
                None => Ok(None),
            }
        }

        fn close_stream(&mut self) -> Result<(), Error> {
            self.kill.busy.store(false, Ordering::Release);
            drop(self.stream.take());
            Ok(())
        }

        fn stream_open(&self) -> bool {
            self.stream.is_some()
        }

        fn begin(&mut self) -> Result<(), Error> {
            self.command("START TRANSACTION".to_owned())
        }

        fn commit(&mut self) -> Result<(), Error> {
            self.command("COMMIT".to_owned())
        }

        fn rollback(&mut self) -> Result<(), Error> {
            self.command("ROLLBACK".to_owned())
        }

        fn savepoint(&mut self, name: &str) -> Result<(), Error> {
            self.command(format!("SAVEPOINT {}", quote_ident(name)?))
        }

        fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), Error> {
            self.command(format!("ROLLBACK TO SAVEPOINT {}", quote_ident(name)?))
        }

        fn release_savepoint(&mut self, name: &str) -> Result<(), Error> {
            self.command(format!("RELEASE SAVEPOINT {}", quote_ident(name)?))
        }

        fn set_statement_timeout(&mut self, remaining: Option<std::time::Duration>) -> Result<(), Error> {
            let ms = remaining
                .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0);
            self.command(format!("SET SESSION max_execution_time = {ms}"))
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(MysqlCancel {
                state: Arc::clone(&self.kill),
            })
        }
    }

    struct MysqlCancel {
        state: Arc<KillState>,
    }

    impl DbCancel for MysqlCancel {
        fn cancel(&self) -> Result<(), Error> {
            if !self.state.busy.load(Ordering::Acquire) {
                return Ok(());
            }
            let id = self.state.connection_id.load(Ordering::Acquire);
            if id == 0 {
                return Ok(());
            }
            let mut conn = mysql::Conn::new(self.state.endpoint.opts()).map_err(mysql_err)?;
            conn.query_drop(format!("KILL QUERY {id}")).map_err(mysql_err)
        }
    }

    fn mysql_err(err: mysql::Error) -> Error {
        Error::new("mysql request failed").with_source(err)
    }

    fn unsupported_prepared(err: &mysql::Error) -> bool {
        matches!(err, mysql::Error::MySqlError(server) if server.code == ER_UNSUPPORTED_PS)
    }

    fn quote_ident(name: &str) -> Result<String, Error> {
        if name.is_empty() || name.contains('\0') {
            return Err(Error::new("mysql savepoint name is empty"));
        }
        let mut quoted = String::with_capacity(name.len() + 2);
        quoted.push('`');
        for ch in name.chars() {
            if ch == '`' {
                quoted.push('`');
            }
            quoted.push(ch);
        }
        quoted.push('`');
        Ok(quoted)
    }

    fn collect_rows(iter: impl Iterator<Item = mysql::Result<mysql::Row>>) -> Result<Vec<DbRow>, Error> {
        let mut rows = Vec::new();
        for row in iter {
            rows.push(map_row(row.map_err(mysql_err)?)?);
        }
        Ok(rows)
    }

    fn drain(iter: impl Iterator<Item = mysql::Result<mysql::Row>>) -> Result<(), Error> {
        for row in iter {
            row.map_err(mysql_err)?;
        }
        Ok(())
    }

    fn map_row(row: mysql::Row) -> Result<DbRow, Error> {
        let columns: Vec<mysql::Column> = row.columns_ref().to_vec();
        let values = row.unwrap();
        let mut out = Vec::with_capacity(columns.len());
        for (column, value) in columns.into_iter().zip(values) {
            let name = column.name_str().into_owned();
            let cell = match map_cell(&column, value) {
                Ok(cell) => cell,
                Err(err) => {
                    return Err(Error::new(format!("mysql column {name}")).with_source(err));
                }
            };
            out.push((name, cell));
        }
        Ok(DbRow { columns: out })
    }

    fn map_cell(column: &mysql::Column, value: MysqlValue) -> Result<Value, Error> {
        if matches!(value, MysqlValue::NULL) {
            return Ok(Value::Null);
        }
        let unsigned = column.flags().contains(ColumnFlags::UNSIGNED_FLAG);
        match column.column_type() {
            ColumnType::MYSQL_TYPE_TINY
            | ColumnType::MYSQL_TYPE_SHORT
            | ColumnType::MYSQL_TYPE_INT24
            | ColumnType::MYSQL_TYPE_LONG
            | ColumnType::MYSQL_TYPE_LONGLONG
            | ColumnType::MYSQL_TYPE_YEAR => map_int(column.column_type(), unsigned, value),
            ColumnType::MYSQL_TYPE_FLOAT | ColumnType::MYSQL_TYPE_DOUBLE => map_float(value),
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => map_decimal(value),
            ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => map_date(value),
            ColumnType::MYSQL_TYPE_TIME | ColumnType::MYSQL_TYPE_TIME2 => map_time(value),
            ColumnType::MYSQL_TYPE_DATETIME
            | ColumnType::MYSQL_TYPE_DATETIME2
            | ColumnType::MYSQL_TYPE_TIMESTAMP
            | ColumnType::MYSQL_TYPE_TIMESTAMP2 => map_datetime(value),
            ColumnType::MYSQL_TYPE_BIT => map_bit(value),
            ColumnType::MYSQL_TYPE_JSON | ColumnType::MYSQL_TYPE_ENUM | ColumnType::MYSQL_TYPE_SET => map_text(value),
            ColumnType::MYSQL_TYPE_GEOMETRY | ColumnType::MYSQL_TYPE_VECTOR => map_bytes(value),
            _ if column.character_set() == CHARSET_BINARY => map_bytes(value),
            _ => map_text(value),
        }
    }

    fn map_int(column_type: ColumnType, _unsigned: bool, value: MysqlValue) -> Result<Value, Error> {
        let number = match value {
            MysqlValue::Int(number) => i128::from(number),
            MysqlValue::UInt(number) => i128::from(number),
            MysqlValue::Bytes(bytes) => parse_integer(&bytes)?,
            other => return Err(Error::new(format!("mysql integer value was {other:?}"))),
        };
        let small = matches!(
            column_type,
            ColumnType::MYSQL_TYPE_TINY
                | ColumnType::MYSQL_TYPE_SHORT
                | ColumnType::MYSQL_TYPE_INT24
                | ColumnType::MYSQL_TYPE_LONG
                | ColumnType::MYSQL_TYPE_YEAR
        );
        if small {
            if let Ok(narrow) = i32::try_from(number) {
                return Ok(Value::I32(narrow));
            }
        }
        if let Ok(wide) = i64::try_from(number) {
            return Ok(Value::I64(wide));
        }
        convert_text(number.to_string(), "BigDecimal")
    }

    fn map_float(value: MysqlValue) -> Result<Value, Error> {
        let number = match value {
            MysqlValue::Float(number) => f64::from(number),
            MysqlValue::Double(number) => number,
            MysqlValue::Int(number) => number as f64,
            MysqlValue::UInt(number) => number as f64,
            MysqlValue::Bytes(bytes) => {
                let text = utf8(&bytes)?;
                text.parse::<f64>()
                    .map_err(|_| Error::new("mysql float was not a number"))?
            }
            other => return Err(Error::new(format!("mysql float value was {other:?}"))),
        };
        Ok(Value::F64(number))
    }

    fn map_decimal(value: MysqlValue) -> Result<Value, Error> {
        let text = match value {
            MysqlValue::Bytes(bytes) => utf8(&bytes)?,
            MysqlValue::Int(number) => number.to_string(),
            MysqlValue::UInt(number) => number.to_string(),
            other => return Err(Error::new(format!("mysql decimal value was {other:?}"))),
        };
        convert_text(text, "BigDecimal")
    }

    fn map_date(value: MysqlValue) -> Result<Value, Error> {
        let text = match value {
            MysqlValue::Date(year, month, day, _, _, _, _) => {
                if year == 0 || month == 0 || day == 0 {
                    return Err(Error::new("mysql date was zero"));
                }
                format!("{year:04}-{month:02}-{day:02}")
            }
            MysqlValue::Bytes(bytes) => utf8(&bytes)?,
            other => return Err(Error::new(format!("mysql date value was {other:?}"))),
        };
        convert_text(text, "Date")
    }

    fn map_time(value: MysqlValue) -> Result<Value, Error> {
        let text = match value {
            MysqlValue::Time(negative, days, hour, minute, second, micros) => {
                if negative || days != 0 {
                    return Err(Error::new("mysql time is outside a civil time of day"));
                }
                if micros == 0 {
                    format!("{hour:02}:{minute:02}:{second:02}")
                } else {
                    format!("{hour:02}:{minute:02}:{second:02}.{micros:06}")
                }
            }
            MysqlValue::Bytes(bytes) => utf8(&bytes)?,
            other => return Err(Error::new(format!("mysql time value was {other:?}"))),
        };
        convert_text(text, "Time")
    }

    fn map_datetime(value: MysqlValue) -> Result<Value, Error> {
        let text = match value {
            MysqlValue::Date(year, month, day, hour, minute, second, micros) => {
                if year == 0 || month == 0 || day == 0 {
                    return Err(Error::new("mysql datetime was zero"));
                }
                if micros == 0 {
                    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
                } else {
                    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{micros:06}")
                }
            }
            MysqlValue::Bytes(bytes) => utf8(&bytes)?,
            other => return Err(Error::new(format!("mysql datetime value was {other:?}"))),
        };
        convert_text(text, "Timestamp")
    }

    fn map_text(value: MysqlValue) -> Result<Value, Error> {
        match value {
            MysqlValue::Bytes(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Ok(Value::String(text)),
                Err(err) => Ok(Value::Bytes(err.into_bytes())),
            },
            MysqlValue::Int(number) => Ok(Value::String(number.to_string())),
            MysqlValue::UInt(number) => Ok(Value::String(number.to_string())),
            other => Err(Error::new(format!("mysql text value was {other:?}"))),
        }
    }

    fn map_bytes(value: MysqlValue) -> Result<Value, Error> {
        match value {
            MysqlValue::Bytes(bytes) => Ok(Value::Bytes(bytes)),
            other => Err(Error::new(format!("mysql binary value was {other:?}"))),
        }
    }

    fn map_bit(value: MysqlValue) -> Result<Value, Error> {
        let bytes = match value {
            MysqlValue::Bytes(bytes) => bytes,
            MysqlValue::Int(number) => return Ok(Value::I64(number)),
            other => return map_int(ColumnType::MYSQL_TYPE_LONGLONG, true, other),
        };
        if bytes.len() > 8 {
            return Ok(Value::Bytes(bytes));
        }
        let mut wide = 0u64;
        for byte in bytes {
            wide = (wide << 8) | u64::from(byte);
        }
        if wide > i64::MAX as u64 {
            map_int(ColumnType::MYSQL_TYPE_LONGLONG, true, MysqlValue::UInt(wide))
        } else {
            Ok(Value::I64(wide as i64))
        }
    }

    fn parse_integer(bytes: &[u8]) -> Result<i128, Error> {
        let text = utf8(bytes)?;
        text.parse::<i128>()
            .map_err(|_| Error::new("mysql integer was not a number"))
    }

    fn utf8(bytes: &[u8]) -> Result<String, Error> {
        String::from_utf8(bytes.to_vec()).map_err(|err| Error::new("mysql text was not utf-8").with_source(err))
    }

    fn convert_text(text: String, java_type: &str) -> Result<Value, Error> {
        let converted = basic_convert(&Value::String(text), java_type);
        if converted.changed {
            Ok(converted.value)
        } else {
            Err(Error::new(format!("could not convert mysql value to {java_type}")))
        }
    }

    fn to_params(params: &[Value]) -> Result<mysql::Params, Error> {
        if params.is_empty() {
            return Ok(mysql::Params::Empty);
        }
        let mut values = Vec::with_capacity(params.len());
        for value in params {
            values.push(to_mysql_value(value)?);
        }
        Ok(mysql::Params::Positional(values))
    }

    fn to_mysql_value(value: &Value) -> Result<MysqlValue, Error> {
        Ok(match value {
            Value::Null => MysqlValue::NULL,
            Value::Bool(flag) => MysqlValue::Int(i64::from(*flag)),
            Value::I32(number) => MysqlValue::Int(i64::from(*number)),
            Value::I64(number) => MysqlValue::Int(*number),
            Value::F64(number) => MysqlValue::Double(*number),
            Value::String(text) => MysqlValue::Bytes(text.as_bytes().to_vec()),
            Value::Bytes(bytes) => MysqlValue::Bytes(bytes.clone()),
            Value::Decimal(number) => MysqlValue::Bytes(number.to_string().into_bytes()),
            Value::Date(date) => MysqlValue::Bytes(date.strftime("%Y-%m-%d").to_string().into_bytes()),
            Value::Time(time) => MysqlValue::Bytes(time.strftime("%H:%M:%S%.6f").to_string().into_bytes()),
            Value::Timestamp(timestamp) => {
                MysqlValue::Bytes(timestamp.strftime("%Y-%m-%d %H:%M:%S%.6f").to_string().into_bytes())
            }
            Value::List(_) | Value::Map(_) | Value::Object(_) => {
                return Err(Error::new("mysql parameters must be scalar values"));
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn must<T>(result: Result<T, Error>) -> T {
            match result {
                Ok(value) => value,
                Err(err) => panic!("{err}"),
            }
        }

        fn column(column_type: ColumnType) -> mysql::Column {
            mysql::Column::new(column_type).with_character_set(45)
        }

        #[test]
        fn types_are_send() {
            fn assert_send<T: Send>() {}
            fn assert_sync<T: Send + Sync>() {}
            assert_send::<MysqlConn>();
            assert_sync::<MysqlCancel>();
            assert_sync::<MysqlDriver>();
        }

        #[test]
        fn maps_mysql_values() {
            assert_eq!(
                must(map_cell(&column(ColumnType::MYSQL_TYPE_LONG), MysqlValue::NULL)),
                Value::Null
            );
            assert_eq!(
                must(map_cell(&column(ColumnType::MYSQL_TYPE_LONG), MysqlValue::Int(7))),
                Value::I32(7)
            );
            assert_eq!(
                must(map_cell(&column(ColumnType::MYSQL_TYPE_LONGLONG), MysqlValue::Int(7))),
                Value::I64(7)
            );
            let unsigned_int = column(ColumnType::MYSQL_TYPE_LONG).with_flags(ColumnFlags::UNSIGNED_FLAG);
            assert_eq!(
                must(map_cell(&unsigned_int, MysqlValue::Int(3_000_000_000))),
                Value::I64(3_000_000_000)
            );
            let big = must(map_cell(
                &column(ColumnType::MYSQL_TYPE_LONGLONG).with_flags(ColumnFlags::UNSIGNED_FLAG),
                MysqlValue::UInt(u64::MAX),
            ));
            assert_eq!(
                big,
                basic_convert(&Value::String(u64::MAX.to_string()), "BigDecimal").value
            );
            assert_eq!(
                must(map_cell(
                    &column(ColumnType::MYSQL_TYPE_DOUBLE),
                    MysqlValue::Double(1.5)
                )),
                Value::F64(1.5)
            );
            let decimal = must(map_cell(
                &column(ColumnType::MYSQL_TYPE_NEWDECIMAL),
                MysqlValue::Bytes(b"12.50".to_vec()),
            ));
            assert_eq!(
                decimal,
                basic_convert(&Value::String("12.50".into()), "BigDecimal").value
            );
            let date = must(map_cell(
                &column(ColumnType::MYSQL_TYPE_DATE),
                MysqlValue::Date(2020, 1, 2, 0, 0, 0, 0),
            ));
            assert_eq!(date, basic_convert(&Value::String("2020-01-02".into()), "Date").value);
            let time = must(map_cell(
                &column(ColumnType::MYSQL_TYPE_TIME),
                MysqlValue::Time(false, 0, 3, 4, 5, 6),
            ));
            assert_eq!(
                time,
                basic_convert(&Value::String("03:04:05.000006".into()), "Time").value
            );
            let stamp = must(map_cell(
                &column(ColumnType::MYSQL_TYPE_DATETIME),
                MysqlValue::Date(2020, 1, 2, 3, 4, 5, 0),
            ));
            assert_eq!(
                stamp,
                basic_convert(&Value::String("2020-01-02 03:04:05".into()), "Timestamp").value
            );
            assert_eq!(
                must(map_cell(
                    &column(ColumnType::MYSQL_TYPE_VARCHAR),
                    MysqlValue::Bytes(b"one".to_vec())
                )),
                Value::string("one")
            );
            let binary = mysql::Column::new(ColumnType::MYSQL_TYPE_BLOB).with_character_set(CHARSET_BINARY);
            assert_eq!(
                must(map_cell(&binary, MysqlValue::Bytes(b"\0\xff".to_vec()))),
                Value::Bytes(vec![0, 255])
            );
            assert_eq!(
                must(map_cell(
                    &column(ColumnType::MYSQL_TYPE_BIT),
                    MysqlValue::Bytes(vec![0x01])
                )),
                Value::I64(1)
            );
            assert!(map_cell(
                &column(ColumnType::MYSQL_TYPE_DATE),
                MysqlValue::Date(0, 0, 0, 0, 0, 0, 0)
            )
            .is_err());
            assert!(to_mysql_value(&Value::List(vec![])).is_err());
            assert_eq!(must(quote_ident("a`b")), "`a``b`");
            assert!(quote_ident("").is_err());
        }

        fn parse_test_spec(spec: &str) -> Option<(&str, &str, &str, u16, &str)> {
            let spec = spec.trim();
            let (userinfo, rest) = spec.rsplit_once('@')?;
            let (username, password) = userinfo.split_once(':')?;
            let (hostport, database) = rest.split_once('/')?;
            let (host, port) = hostport.rsplit_once(':')?;
            let port = port.parse().ok()?;
            if host.is_empty() || database.is_empty() {
                return None;
            }
            Some((username, password, host, port, database))
        }

        fn test_driver() -> Option<MysqlDriver> {
            if let Ok(spec) = std::env::var("MOQUI_TEST_MYSQL") {
                let (username, password, host, port, database) =
                    parse_test_spec(&spec).expect("MOQUI_TEST_MYSQL must be user:pass@host:port/database");
                let driver = MysqlDriver::new(host, port, username, password, database, false);
                return driver.connect_mysql().ok().map(|_| driver);
            }
            let address = std::net::SocketAddr::from(([127, 0, 0, 1], 3306));
            if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(1)).is_err() {
                return None;
            }
            let driver = MysqlDriver::new("127.0.0.1", 3306, "root", "", "mysql", false);
            driver.connect_mysql().ok().map(|_| driver)
        }

        #[test]
        fn parses_mysql_test_target() {
            assert_eq!(
                parse_test_spec("root:secret@127.0.0.1:3307/app"),
                Some(("root", "secret", "127.0.0.1", 3307, "app"))
            );
            assert_eq!(
                parse_test_spec("root:@127.0.0.1:3306/mysql"),
                Some(("root", "", "127.0.0.1", 3306, "mysql"))
            );
            assert!(parse_test_spec("not a target").is_none());
        }

        #[test]
        fn round_trip_stream_and_statement_cache() {
            let Some(driver) = test_driver() else {
                tracing::debug!("MySQL was not reachable");
                return;
            };
            let mut conn = must(driver.connect_mysql());
            must(conn.execute(
                "CREATE TEMPORARY TABLE moqui_mysql_roundtrip (
                    id INT NOT NULL PRIMARY KEY,
                    name VARCHAR(64) NULL
                )",
                &[],
            ));
            let insert = "INSERT INTO moqui_mysql_roundtrip (id, name) VALUES (?, ?)";
            assert_eq!(must(conn.execute(insert, &[Value::I32(1), Value::string("one")])), 1);
            assert_eq!(must(conn.execute(insert, &[Value::I32(2), Value::string("two")])), 1);
            let select = "SELECT id, name FROM moqui_mysql_roundtrip ORDER BY id";
            let rows = must(conn.query(select, &[]));
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].get("id"), Some(&Value::I32(1)));
            assert_eq!(rows[0].get("name"), Some(&Value::string("one")));
            assert_eq!(rows[1].get("id"), Some(&Value::I32(2)));
            assert_eq!(rows[1].get("name"), Some(&Value::string("two")));

            must(conn.open_stream(select, &[]));
            assert!(conn.stream_open());
            assert!(conn.execute("SELECT 1", &[]).is_err());
            let first = must(conn.next_row()).expect("first streamed row");
            let second = must(conn.next_row()).expect("second streamed row");
            assert!(must(conn.next_row()).is_none());
            must(conn.close_stream());
            assert!(!conn.stream_open());
            assert_eq!(first.get("id"), Some(&Value::I32(1)));
            assert_eq!(second.get("name"), Some(&Value::string("two")));

            let mut cached = must(driver.connect_mysql());
            let probe = "SELECT ? AS id";
            let first = must(cached.query(probe, &[Value::I32(1)]));
            let second = must(cached.query(probe, &[Value::I32(2)]));
            assert_eq!(cached.statement_cache_len(), 1);
            assert!(matches!(first[0].columns[0].1, Value::I32(1) | Value::I64(1)));
            assert!(matches!(second[0].columns[0].1, Value::I32(2) | Value::I64(2)));
            let len = cached.statement_cache_len();
            let once = must(cached.query_once("SELECT ? AS extra", &[Value::string("z")]));
            assert_eq!(cached.statement_cache_len(), len);
            assert_eq!(once[0].columns[0].1, Value::string("z"));

            let mut boxed = must(DbDriver::connect(&driver));
            must(boxed.query_once("SELECT 1", &[]));
        }
    }
}

#[cfg(feature = "mysql")]
pub use driver::{MysqlConn, MysqlDriver};
