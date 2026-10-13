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

//! Sync Postgres client behind [`moqui_core::driver::DbDriver`].
//!
//! `postgres::Client` owns the `current_thread` runtime and `block_on`s connect, query, and drop.

#[cfg(feature = "postgres")]
mod driver {
    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;

    use moqui_core::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use moqui_lib::error::Error;
    use moqui_lib::object::{basic_convert, Value};
    use postgres::config::SslMode;
    use postgres::types::private::BytesMut;
    use postgres::types::{Format, FromSql, IsNull, ToSql, Type};
    use postgres::{Client, Config, NoTls, Statement};
    use tokio_postgres_rustls::MakeRustlsConnect;

    const CACHE_CAP: usize = 100;
    const FETCH_BATCH: usize = 50;
    const STREAM_CURSOR: &str = "moqui_stream_cursor";
    /// Seconds from the Unix epoch to 2000-01-01 00:00:00 UTC.
    const PG_EPOCH_SECS: i64 = 946_684_800;
    const PG_EPOCH_USEC: i64 = PG_EPOCH_SECS * 1_000_000;
    /// Days from 1970-01-01 to 2000-01-01.
    const PG_EPOCH_DAYS: i64 = 10_957;

    /// Username, database, and either a Unix socket directory or a TCP host.
    pub struct PostgresDriver {
        user: String,
        database: String,
        password: Option<String>,
        target: Target,
    }

    enum Target {
        #[cfg(unix)]
        Unix { socket_dir: PathBuf },
        /// Loopback TCP with [`NoTls`]. Embedded Postgres on Windows uses this.
        TcpNoTls { host: String, port: u16 },
        Tcp {
            host: String,
            port: u16,
            tls: MakeRustlsConnect,
        },
    }

    #[derive(Clone)]
    enum TlsMode {
        None,
        Rustls(MakeRustlsConnect),
    }

    /// One `postgres::Client`. Statement LRU is keyed by the original SQL text.
    pub struct PostgresConn {
        client: Client,
        cache: StatementCache,
        in_transaction: bool,
        streaming: bool,
        stream_exhausted: bool,
        stream_buf: VecDeque<DbRow>,
        tls: TlsMode,
    }

    struct StatementCache {
        statements: HashMap<String, Statement>,
        order: VecDeque<String>,
    }

    struct PgCancel {
        token: postgres::CancelToken,
        tls: TlsMode,
    }

    struct Param<'a>(&'a Value);

    struct PgValue(Value);

    impl PostgresDriver {
        /// Unix socket directory and [`NoTls`]. The port is the server default, 5432.
        #[cfg(unix)]
        pub fn unix_socket(
            user: impl Into<String>,
            database: impl Into<String>,
            socket_dir: impl Into<PathBuf>,
        ) -> Self {
            Self {
                user: user.into(),
                database: database.into(),
                password: None,
                target: Target::Unix {
                    socket_dir: socket_dir.into(),
                },
            }
        }

        /// TCP host and [`NoTls`]. Embedded Windows uses `127.0.0.1` and the ephemeral port.
        /// External servers stay on [`Self::external`].
        #[must_use]
        pub fn tcp_no_tls(
            user: impl Into<String>,
            database: impl Into<String>,
            host: impl Into<String>,
            port: u16,
        ) -> Self {
            Self {
                user: user.into(),
                database: database.into(),
                password: None,
                target: Target::TcpNoTls {
                    host: host.into(),
                    port,
                },
            }
        }

        /// TCP host and rustls. `MakeRustlsConnect` implements `MakeTlsConnect` for the sync client.
        ///
        /// # Errors
        ///
        /// Returns an error when the rustls provider cannot be installed.
        pub fn external(
            user: impl Into<String>,
            database: impl Into<String>,
            host: impl Into<String>,
            port: u16,
        ) -> Result<Self, Error> {
            let tls = std::panic::catch_unwind(MakeRustlsConnect::with_webpki_roots)
                .map_err(|_| Error::new("external TLS is not available on the sync client"))?;
            Ok(Self {
                user: user.into(),
                database: database.into(),
                password: None,
                target: Target::Tcp {
                    host: host.into(),
                    port,
                    tls,
                },
            })
        }

        /// Password for the role. Embedded initdb uses password auth.
        #[must_use]
        pub fn password(mut self, password: impl Into<String>) -> Self {
            self.password = Some(password.into());
            self
        }

        /// Open one client. The returned connection is the concrete type tests cache-count.
        ///
        /// # Errors
        ///
        /// Returns an error when the server rejects the connection or `SET TIME ZONE`.
        pub fn connect_postgres(&self) -> Result<PostgresConn, Error> {
            let mut config = Config::new();
            config.user(&self.user).dbname(&self.database);
            if let Some(password) = &self.password {
                config.password(password);
            }
            let (mut client, tls) = match &self.target {
                #[cfg(unix)]
                Target::Unix { socket_dir } => {
                    config.host_path(socket_dir);
                    config.ssl_mode(SslMode::Disable);
                    let client = config.connect(NoTls).map_err(|err| pg("connect", err))?;
                    (client, TlsMode::None)
                }
                Target::TcpNoTls { host, port } => {
                    config.host(host);
                    config.port(*port);
                    config.ssl_mode(SslMode::Disable);
                    let client = config.connect(NoTls).map_err(|err| pg("connect", err))?;
                    (client, TlsMode::None)
                }
                Target::Tcp { host, port, tls } => {
                    config.host(host);
                    config.port(*port);
                    config.ssl_mode(SslMode::Require);
                    let client = config.connect(tls.clone()).map_err(|err| pg("connect", err))?;
                    (client, TlsMode::Rustls(tls.clone()))
                }
            };
            client
                .batch_execute("SET TIME ZONE 'UTC'")
                .map_err(|err| pg("set time zone", err))?;
            Ok(PostgresConn {
                client,
                cache: StatementCache::new(),
                in_transaction: false,
                streaming: false,
                stream_exhausted: false,
                stream_buf: VecDeque::new(),
                tls,
            })
        }
    }

    impl DbDriver for PostgresDriver {
        fn connect(&self) -> Result<Box<dyn DbConnection>, Error> {
            Ok(Box::new(self.connect_postgres()?))
        }
    }

    impl PostgresConn {
        /// Prepared statements stored for `execute` and `query`. `query_once` does not count.
        #[must_use]
        pub fn statement_cache_len(&self) -> usize {
            self.cache.len()
        }

        fn prepared(&mut self, sql: &str) -> Result<Statement, Error> {
            if let Some(statement) = self.cache.get(sql) {
                return Ok(statement);
            }
            let rewritten = rewrite_placeholders(sql);
            let statement = self.client.prepare(&rewritten).map_err(|err| pg("prepare", err))?;
            self.cache.insert(sql.to_string(), statement.clone());
            Ok(statement)
        }

        fn finish_transaction(&mut self, sql: &str) -> Result<(), Error> {
            if self.streaming {
                self.close_stream()?;
            }
            self.client.batch_execute(sql).map_err(|err| pg(sql, err))?;
            self.in_transaction = false;
            Ok(())
        }

        fn fetch_stream(&mut self) -> Result<(), Error> {
            let sql = format!("FETCH {FETCH_BATCH} FROM {STREAM_CURSOR}");
            let rows = self.client.query(&sql, &[]).map_err(|err| pg("fetch cursor", err))?;
            if rows.len() < FETCH_BATCH {
                self.stream_exhausted = true;
            }
            for row in &rows {
                self.stream_buf.push_back(map_row(row)?);
            }
            Ok(())
        }

        fn clear_stream(&mut self) {
            self.streaming = false;
            self.stream_exhausted = false;
            self.stream_buf.clear();
        }
    }

    impl DbConnection for PostgresConn {
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
            let bound = bind(params)?;
            let refs = param_refs(&bound);
            let statement = self.prepared(sql)?;
            self.client.execute(&statement, &refs).map_err(|err| pg("execute", err))
        }

        fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            let bound = bind(params)?;
            let refs = param_refs(&bound);
            let statement = self.prepared(sql)?;
            let rows = self.client.query(&statement, &refs).map_err(|err| pg("query", err))?;
            rows.iter().map(map_row).collect()
        }

        fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            let bound = bind(params)?;
            let refs = param_refs(&bound);
            let rewritten = rewrite_placeholders(sql);
            let rows = self.client.query(&rewritten, &refs).map_err(|err| pg("query", err))?;
            rows.iter().map(map_row).collect()
        }

        fn open_stream(&mut self, sql: &str, params: &[Value]) -> Result<(), Error> {
            if !self.in_transaction {
                return Err(Error::new("a transaction is required"));
            }
            if self.streaming {
                return Err(Error::new("a stream is already open"));
            }
            let bound = bind(params)?;
            let refs = param_refs(&bound);
            let rewritten = rewrite_placeholders(sql);
            let declare = format!("DECLARE {STREAM_CURSOR} NO SCROLL CURSOR FOR {rewritten}");
            self.client
                .execute(&declare, &refs)
                .map_err(|err| pg("declare cursor", err))?;
            self.streaming = true;
            self.stream_exhausted = false;
            self.stream_buf.clear();
            Ok(())
        }

        fn next_row(&mut self) -> Result<Option<DbRow>, Error> {
            if !self.streaming {
                return Ok(None);
            }
            if self.stream_buf.is_empty() {
                if self.stream_exhausted {
                    return Ok(None);
                }
                self.fetch_stream()?;
            }
            Ok(self.stream_buf.pop_front())
        }

        fn close_stream(&mut self) -> Result<(), Error> {
            if !self.streaming {
                return Ok(());
            }
            self.client
                .batch_execute(&format!("CLOSE {STREAM_CURSOR}"))
                .map_err(|err| pg("close cursor", err))?;
            self.clear_stream();
            Ok(())
        }

        fn stream_open(&self) -> bool {
            self.streaming
        }

        fn begin(&mut self) -> Result<(), Error> {
            self.client.batch_execute("BEGIN").map_err(|err| pg("begin", err))?;
            self.in_transaction = true;
            Ok(())
        }

        fn commit(&mut self) -> Result<(), Error> {
            self.finish_transaction("COMMIT")
        }

        fn rollback(&mut self) -> Result<(), Error> {
            self.finish_transaction("ROLLBACK")
        }

        fn savepoint(&mut self, name: &str) -> Result<(), Error> {
            let sql = format!("SAVEPOINT {}", quote_ident(name));
            self.client.batch_execute(&sql).map_err(|err| pg("savepoint", err))
        }

        fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), Error> {
            let sql = format!("ROLLBACK TO SAVEPOINT {}", quote_ident(name));
            self.client
                .batch_execute(&sql)
                .map_err(|err| pg("rollback to savepoint", err))
        }

        fn release_savepoint(&mut self, name: &str) -> Result<(), Error> {
            let sql = format!("RELEASE SAVEPOINT {}", quote_ident(name));
            self.client
                .batch_execute(&sql)
                .map_err(|err| pg("release savepoint", err))
        }

        /// `SET LOCAL` lasts until the transaction ends, so clearing is a no-op.
        fn set_statement_timeout(&mut self, remaining: Option<std::time::Duration>) -> Result<(), Error> {
            let Some(remaining) = remaining else {
                return Ok(());
            };
            let ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
            self.client
                .batch_execute(&format!("SET LOCAL statement_timeout = {ms}"))
                .map_err(|err| pg("statement_timeout", err))?;
            Ok(())
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(PgCancel {
                token: self.client.cancel_token(),
                tls: self.tls.clone(),
            })
        }
    }

    impl DbCancel for PgCancel {
        fn cancel(&self) -> Result<(), Error> {
            let result = match &self.tls {
                TlsMode::None => self.token.cancel_query(NoTls),
                TlsMode::Rustls(tls) => self.token.cancel_query(tls.clone()),
            };
            result.map_err(|err| pg("cancel", err))
        }
    }

    impl StatementCache {
        fn new() -> Self {
            Self {
                statements: HashMap::new(),
                order: VecDeque::new(),
            }
        }

        fn len(&self) -> usize {
            self.statements.len()
        }

        fn get(&mut self, sql: &str) -> Option<Statement> {
            let statement = self.statements.get(sql)?.clone();
            self.touch(sql);
            Some(statement)
        }

        fn insert(&mut self, sql: String, statement: Statement) {
            if self.statements.contains_key(&sql) {
                self.touch(&sql);
                self.statements.insert(sql, statement);
                return;
            }
            if self.order.len() >= CACHE_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.statements.remove(&old);
                }
            }
            self.order.push_back(sql.clone());
            self.statements.insert(sql, statement);
        }

        fn touch(&mut self, sql: &str) {
            if let Some(index) = self.order.iter().position(|key| key == sql) {
                if let Some(key) = self.order.remove(index) {
                    self.order.push_back(key);
                }
            }
        }
    }

    impl std::fmt::Debug for Param<'_> {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Debug::fmt(self.0, formatter)
        }
    }

    impl ToSql for Param<'_> {
        fn to_sql(&self, _ty: &Type, out: &mut BytesMut) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
            match value_text(self.0)? {
                Some(text) => {
                    out.extend_from_slice(text.as_bytes());
                    Ok(IsNull::No)
                }
                None => Ok(IsNull::Yes),
            }
        }

        fn accepts(_ty: &Type) -> bool {
            true
        }

        fn encode_format(&self, _ty: &Type) -> Format {
            Format::Text
        }

        postgres::types::to_sql_checked!();
    }

    impl<'a> FromSql<'a> for PgValue {
        fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
            Ok(PgValue(decode_value(ty, raw)?))
        }

        fn accepts(_ty: &Type) -> bool {
            true
        }
    }

    fn pg(context: &str, err: postgres::Error) -> Error {
        // `Display` for a server error is the text "db error". The server message is the useful part.
        let detail = match err.as_db_error() {
            Some(db) => db.message().to_string(),
            None => err.to_string(),
        };
        Error::new(format!("postgres {context}: {detail}")).with_source(err)
    }

    fn bind(params: &[Value]) -> Result<Vec<Param<'_>>, Error> {
        if params
            .iter()
            .any(|value| matches!(value, Value::List(_) | Value::Map(_)))
        {
            return Err(Error::new("postgres parameters must be scalar values"));
        }
        Ok(params.iter().map(Param).collect())
    }

    fn param_refs<'a>(params: &'a [Param<'a>]) -> Vec<&'a (dyn ToSql + Sync)> {
        params.iter().map(|param| param as &(dyn ToSql + Sync)).collect()
    }

    fn rewrite_placeholders(sql: &str) -> String {
        let mut out = String::with_capacity(sql.len() + 8);
        let mut chars = sql.chars().peekable();
        let mut in_quote = false;
        let mut index = 0u32;
        while let Some(ch) = chars.next() {
            if in_quote {
                out.push(ch);
                if ch == '\'' {
                    if chars.peek() == Some(&'\'') {
                        out.push(chars.next().unwrap_or('\''));
                    } else {
                        in_quote = false;
                    }
                }
            } else if ch == '\'' {
                in_quote = true;
                out.push(ch);
            } else if ch == '?' {
                index += 1;
                out.push('$');
                out.push_str(&index.to_string());
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn quote_ident(name: &str) -> String {
        let mut quoted = String::with_capacity(name.len() + 2);
        quoted.push('"');
        for ch in name.chars() {
            if ch == '"' {
                quoted.push_str("\"\"");
            } else {
                quoted.push(ch);
            }
        }
        quoted.push('"');
        quoted
    }

    fn value_text(value: &Value) -> Result<Option<String>, Box<dyn std::error::Error + Sync + Send>> {
        let text = match value {
            Value::Null => return Ok(None),
            Value::String(text) => text.clone(),
            Value::Bool(true) => "true".to_string(),
            Value::Bool(false) => "false".to_string(),
            Value::I32(number) => number.to_string(),
            Value::I64(number) => number.to_string(),
            Value::F64(number) => format_f64(*number),
            Value::Decimal(number) => number.to_string(),
            Value::Timestamp(timestamp) => timestamp.to_string(),
            Value::Date(date) => date.to_string(),
            Value::Time(time) => time.to_string(),
            Value::Bytes(bytes) => format_bytea(bytes),
            Value::List(_) | Value::Map(_) | Value::Object(_) => {
                return Err("postgres parameters must be scalar values".into());
            }
        };
        Ok(Some(text))
    }

    fn format_f64(number: f64) -> String {
        if number.is_nan() {
            "NaN".to_string()
        } else if number.is_infinite() && number.is_sign_negative() {
            "-Infinity".to_string()
        } else if number.is_infinite() {
            "Infinity".to_string()
        } else {
            number.to_string()
        }
    }

    fn format_bytea(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(2 + bytes.len() * 2);
        text.push_str("\\x");
        text.push_str(&hex_encode(bytes));
        text
    }

    fn hex_encode(raw: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(raw.len() * 2);
        for byte in raw {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        out
    }

    fn map_row(row: &postgres::Row) -> Result<DbRow, Error> {
        let mut columns = Vec::with_capacity(row.len());
        for (index, column) in row.columns().iter().enumerate() {
            let value = row
                .try_get::<_, Option<PgValue>>(index)
                .map_err(|err| Error::new(format!("postgres column {index}")).with_source(err))?;
            columns.push((
                column.name().to_string(),
                value.map(|cell| cell.0).unwrap_or(Value::Null),
            ));
        }
        Ok(DbRow { columns })
    }

    fn decode_value(ty: &Type, raw: &[u8]) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        if ty == &Type::BOOL {
            decode_bool(raw)
        } else if ty == &Type::INT2 {
            Ok(Value::I32(i32::from(i16::from_be_bytes(read_array(raw)?))))
        } else if ty == &Type::INT4 {
            Ok(Value::I32(i32::from_be_bytes(read_array(raw)?)))
        } else if ty == &Type::INT8 {
            Ok(Value::I64(i64::from_be_bytes(read_array(raw)?)))
        } else if ty == &Type::FLOAT4 {
            Ok(Value::F64(f64::from(f32::from_be_bytes(read_array(raw)?))))
        } else if ty == &Type::FLOAT8 {
            Ok(Value::F64(f64::from_be_bytes(read_array(raw)?)))
        } else if is_text(ty) {
            decode_text(raw)
        } else if ty == &Type::JSONB {
            // Binary jsonb is a version byte, 1, then the JSON text.
            match raw.split_first() {
                Some((1, text)) => decode_text(text),
                _ => Err(bad("bad jsonb")),
            }
        } else if ty == &Type::BYTEA {
            Ok(Value::Bytes(raw.to_vec()))
        } else if ty == &Type::NUMERIC {
            expect_value(&numeric_to_text(raw)?, "BigDecimal")
        } else if ty == &Type::TIMESTAMPTZ || ty == &Type::TIMESTAMP {
            decode_timestamp(raw)
        } else if ty == &Type::DATE {
            decode_date(raw)
        } else if ty == &Type::TIME {
            decode_time(raw)
        } else {
            Ok(decode_unknown(ty, raw))
        }
    }

    fn is_text(ty: &Type) -> bool {
        ty == &Type::TEXT
            || ty == &Type::VARCHAR
            || ty == &Type::BPCHAR
            || ty == &Type::NAME
            || ty == &Type::UNKNOWN
            || ty == &Type::JSON
            || ty == &Type::XML
    }

    fn decode_bool(raw: &[u8]) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        match raw {
            [0] => Ok(Value::Bool(false)),
            [1] => Ok(Value::Bool(true)),
            _ => Err(bad("bad bool")),
        }
    }

    fn decode_text(raw: &[u8]) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        let text = std::str::from_utf8(raw).map_err(|_| bad("invalid utf8"))?;
        Ok(Value::string(text))
    }

    fn decode_unknown(ty: &Type, raw: &[u8]) -> Value {
        match std::str::from_utf8(raw) {
            Ok(text) => Value::string(text),
            Err(_) => Value::string(format!("{ty}:{}", hex_encode(raw))),
        }
    }

    fn decode_timestamp(raw: &[u8]) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        let micros = i64::from_be_bytes(read_array(raw)?);
        if micros == i64::MAX {
            return Ok(Value::string("infinity"));
        }
        if micros == i64::MIN {
            return Ok(Value::string("-infinity"));
        }
        expect_value(&format_pg_timestamp(micros)?, "Timestamp")
    }

    fn decode_date(raw: &[u8]) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        let days = i32::from_be_bytes(read_array(raw)?);
        if days == i32::MAX {
            return Ok(Value::string("infinity"));
        }
        if days == i32::MIN {
            return Ok(Value::string("-infinity"));
        }
        let unix_days = i64::from(days) + PG_EPOCH_DAYS;
        let (year, month, day) = civil_from_unix_days(unix_days).ok_or_else(|| bad("date"))?;
        expect_value(&format!("{year:04}-{month:02}-{day:02}"), "Date")
    }

    fn decode_time(raw: &[u8]) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        let micros = i64::from_be_bytes(read_array(raw)?);
        if micros < 0 {
            return Err(bad("time"));
        }
        let secs = micros / 1_000_000;
        let frac = u32::try_from(micros % 1_000_000).map_err(|_| bad("time"))?;
        if secs > 86_400 {
            return Err(bad("time"));
        }
        let hour = secs / 3_600;
        let minute = (secs % 3_600) / 60;
        let second = secs % 60;
        expect_value(&format!("{hour:02}:{minute:02}:{second:02}.{frac:06}"), "Time")
    }

    fn format_pg_timestamp(micros: i64) -> Result<String, Box<dyn std::error::Error + Sync + Send>> {
        let unix_us = micros
            .checked_add(PG_EPOCH_USEC)
            .ok_or_else(|| bad("timestamp overflow"))?;
        let secs = unix_us.div_euclid(1_000_000);
        let frac = u32::try_from(unix_us.rem_euclid(1_000_000)).map_err(|_| bad("timestamp fraction"))?;
        let days = secs.div_euclid(86_400);
        let tod = secs.rem_euclid(86_400);
        let (year, month, day) = civil_from_unix_days(days).ok_or_else(|| bad("timestamp date"))?;
        let hour = tod / 3_600;
        let minute = (tod % 3_600) / 60;
        let second = tod % 60;
        Ok(format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{frac:06}"
        ))
    }

    fn expect_value(text: &str, kind: &str) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        let converted = basic_convert(&Value::string(text), kind);
        if kind_matches(&converted.value, kind) {
            Ok(converted.value)
        } else {
            Err(bad(&format!("could not decode {kind} from {text}")))
        }
    }

    fn kind_matches(value: &Value, kind: &str) -> bool {
        matches!(
            (value, kind),
            (Value::Decimal(_), "BigDecimal")
                | (Value::Timestamp(_), "Timestamp")
                | (Value::Date(_), "Date")
                | (Value::Time(_), "Time")
        )
    }

    fn read_array<const N: usize>(raw: &[u8]) -> Result<[u8; N], Box<dyn std::error::Error + Sync + Send>> {
        raw.try_into().map_err(|_| bad("short postgres value"))
    }

    fn bad(message: &str) -> Box<dyn std::error::Error + Sync + Send> {
        message.into()
    }

    fn numeric_to_text(raw: &[u8]) -> Result<String, Box<dyn std::error::Error + Sync + Send>> {
        if raw.len() < 8 {
            return Err(bad("short numeric"));
        }
        let ndigits = i16::from_be_bytes([raw[0], raw[1]]);
        let weight = i32::from(i16::from_be_bytes([raw[2], raw[3]]));
        let sign = u16::from_be_bytes([raw[4], raw[5]]);
        let dscale_raw = i16::from_be_bytes([raw[6], raw[7]]);
        if ndigits < 0 || dscale_raw < 0 {
            return Err(bad("bad numeric header"));
        }
        match sign {
            0x0000 | 0x4000 => {}
            0xC000 => return Err(bad("numeric NaN")),
            0xD000 => return Err(bad("numeric infinity")),
            0xF000 => return Err(bad("numeric -infinity")),
            _ => return Err(bad("numeric sign")),
        }
        let ndigits = usize::from(u16::try_from(ndigits).map_err(|_| bad("bad numeric digits"))?);
        let dscale = usize::from(u16::try_from(dscale_raw).map_err(|_| bad("bad numeric scale"))?);
        if ndigits > 2_000 || dscale > 8_000 || !(-2_000..=2_000).contains(&weight) {
            return Err(bad("numeric too large"));
        }
        if raw.len() < 8 + ndigits * 2 {
            return Err(bad("short numeric"));
        }
        let mut digits = Vec::with_capacity(ndigits);
        for index in 0..ndigits {
            let offset = 8 + index * 2;
            let digit = u16::from_be_bytes([raw[offset], raw[offset + 1]]);
            if digit >= 10_000 {
                return Err(bad("numeric digit"));
            }
            digits.push(digit);
        }
        let mut integer = String::new();
        let mut fraction = String::new();
        if weight >= 0 {
            let groups = usize::try_from(weight + 1).map_err(|_| bad("numeric weight"))?;
            for index in 0..groups {
                let digit = digits.get(index).copied().unwrap_or(0);
                if index == 0 {
                    integer.push_str(&digit.to_string());
                } else {
                    integer.push_str(&format!("{digit:04}"));
                }
            }
            for digit in digits.iter().skip(groups) {
                fraction.push_str(&format!("{digit:04}"));
            }
        } else {
            integer.push('0');
            let leading = usize::try_from(-weight - 1).map_err(|_| bad("numeric weight"))?;
            if leading > 2_000 {
                return Err(bad("numeric too large"));
            }
            for _ in 0..leading {
                fraction.push_str("0000");
            }
            for digit in &digits {
                fraction.push_str(&format!("{digit:04}"));
            }
        }
        if fraction.len() < dscale {
            fraction.push_str(&"0".repeat(dscale - fraction.len()));
        } else {
            fraction.truncate(dscale);
        }
        let mut text = String::new();
        if sign == 0x4000 {
            text.push('-');
        }
        if integer.is_empty() {
            text.push('0');
        } else {
            text.push_str(&integer);
        }
        if dscale > 0 {
            text.push('.');
            text.push_str(&fraction);
        }
        Ok(text)
    }

    /// Howard Hinnant's `civil_from_days`. `days` is the Unix day count.
    fn civil_from_unix_days(days: i64) -> Option<(i32, u32, u32)> {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let mut year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        if month <= 2 {
            year += 1;
        }
        Some((
            i32::try_from(year).ok()?,
            u32::try_from(month).ok()?,
            u32::try_from(day).ok()?,
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn placeholders_skip_quoted_question_marks() {
            assert_eq!(rewrite_placeholders("SELECT ?"), "SELECT $1");
            assert_eq!(rewrite_placeholders("SELECT ?, ?"), "SELECT $1, $2");
            assert_eq!(rewrite_placeholders("SELECT '?'"), "SELECT '?'");
            assert_eq!(rewrite_placeholders("SELECT 'it''s ?', ?"), "SELECT 'it''s ?', $1");
            assert_eq!(rewrite_placeholders("SELECT 'a''?''b' || ?"), "SELECT 'a''?''b' || $1");
        }

        #[test]
        fn numeric_binary_decodes_base_10000_digits() {
            let raw = [
                0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x0F, 0x6E, 0x04, 0xD2, 0x15, 0xE0,
            ];
            assert_eq!(numeric_to_text(&raw).unwrap(), "3950.123456");
            let negative = [
                0x00, 0x03, 0x00, 0x00, 0x40, 0x00, 0x00, 0x06, 0x0F, 0x6E, 0x04, 0xD2, 0x15, 0xE0,
            ];
            assert_eq!(numeric_to_text(&negative).unwrap(), "-3950.123456");
            assert_eq!(
                numeric_to_text(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap(),
                "0"
            );
        }

        #[test]
        fn civil_dates_match_the_unix_epoch() {
            assert_eq!(civil_from_unix_days(0), Some((1970, 1, 1)));
            assert_eq!(civil_from_unix_days(10_957), Some((2000, 1, 1)));
            assert_eq!(civil_from_unix_days(18_262), Some((2020, 1, 1)));
        }

        #[test]
        fn external_builds_a_rustls_connector_or_reports_it() {
            let driver = PostgresDriver::external("user", "db", "localhost", 5432).expect("rustls connector");
            assert!(matches!(driver.target, Target::Tcp { port: 5432, .. }));
        }

        #[test]
        fn tcp_no_tls_builds_a_loopback_target() {
            let driver = PostgresDriver::tcp_no_tls("postgres", "moqui", "127.0.0.1", 5432);
            assert!(matches!(
                driver.target,
                Target::TcpNoTls {
                    ref host,
                    port: 5432
                } if host == "127.0.0.1"
            ));
        }
    }
}

#[cfg(feature = "postgres")]
pub use driver::{PostgresConn, PostgresDriver};
