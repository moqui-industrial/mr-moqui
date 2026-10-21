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

//! Postgres behind [`moqui_core::driver::DbDriver`], on `tokio_postgres`.
//!
//! Each connection runs its client on its own `current_thread` runtime, and a call blocks on that runtime until its
//! reply is in, as the sync `postgres` crate does. A `BEGIN`, a `SET LOCAL statement_timeout` and a deferred write
//! ([`DbConnection::execute_deferred`]) are sent without waiting for their replies: they share the round trip of the
//! next call that waits, which takes those replies first and reports the first failure among them.

#[cfg(feature = "postgres")]
mod driver {
    use std::collections::VecDeque;
    use std::future::{poll_fn, Future};
    use std::path::PathBuf;
    use std::pin::{pin, Pin};
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    use moqui_core::driver::{DbCancel, DbConnection, DbDriver, DbRow};
    use moqui_lib::codec::hex_encode;
    use moqui_lib::error::Error;
    use moqui_lib::object::{basic_convert, Value};
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio::runtime::Runtime;
    use tokio_postgres::config::SslMode;
    use tokio_postgres::tls::MakeTlsConnect;
    use tokio_postgres::types::private::BytesMut;
    use tokio_postgres::types::{Format, FromSql, IsNull, ToSql, Type};
    use tokio_postgres::{AsyncMessage, Client, Config, NoTls, Notification, Socket, Statement};
    use tokio_postgres_rustls::MakeRustlsConnect;

    use crate::db::statement_lru::StatementLru;

    const CACHE_CAP: usize = 100;
    /// Requests sent without waiting that may be in flight. A send past this waits for the older half of them.
    const PIPELINE_DEPTH: usize = 256;
    const FETCH_BATCH: usize = 50;
    const STREAM_CURSOR: &str = "moqui_stream_cursor";
    /// Seconds from the Unix epoch to 2000-01-01 00:00:00 UTC.
    const PG_EPOCH_SECS: i64 = 946_684_800;
    const PG_EPOCH_USEC: i64 = PG_EPOCH_SECS * 1_000_000;
    /// Days from 1970-01-01 to 2000-01-01.
    const PG_EPOCH_DAYS: i64 = 10_957;

    /// Username, database, and either a Unix socket directory or a TCP host.
    #[derive(Clone)]
    pub struct PostgresDriver {
        user: String,
        database: String,
        password: Option<String>,
        target: Target,
        /// libpq `options`: server settings for each session, such as `-c statement_timeout=30000`.
        options: Option<String>,
    }

    #[derive(Clone)]
    enum Target {
        #[cfg(unix)]
        Unix { socket_dir: PathBuf, port: u16 },
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

    /// One `tokio_postgres` client and its connection. Statement LRU is keyed by the original SQL text.
    pub struct PostgresConn {
        /// Taken when the connection drops, so the connection sees its last client go and sends `Terminate`.
        client: Option<Arc<Client>>,
        cancel: tokio_postgres::CancelToken,
        wire: Wire,
        cache: StatementLru<Prepared>,
        in_transaction: bool,
        streaming: bool,
        stream_exhausted: bool,
        stream_buf: VecDeque<DbRow>,
        tls: TlsMode,
    }

    /// A prepared statement and its column names, which every row it reads shares.
    #[derive(Clone)]
    struct Prepared {
        statement: Statement,
        names: Arc<[Arc<str>]>,
    }

    /// The connection's runtime, and what drives the socket.
    struct Wire {
        runtime: Runtime,
        link: Link,
    }

    /// The socket, the replies owed to requests that did not wait, and the notifications read in passing.
    struct Link {
        connection: Box<dyn Drive>,
        /// Oldest first. Replies come in the order the requests were sent.
        pending: VecDeque<Pending>,
        /// The first failure among `pending`, for the next call that waits.
        failed: Option<Error>,
        notifications: VecDeque<Notification>,
        /// The connection failed or ended.
        dead: bool,
    }

    /// A request sent and not waited for. It ends when its reply is read.
    type Pending = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

    /// A `tokio_postgres::Connection`, whatever its socket and TLS stream.
    trait Drive: Send {
        fn poll_message(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<AsyncMessage, tokio_postgres::Error>>>;
    }

    impl<S, T> Drive for tokio_postgres::Connection<S, T>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
        T: AsyncRead + AsyncWrite + Unpin + Send,
    {
        fn poll_message(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<AsyncMessage, tokio_postgres::Error>>> {
            tokio_postgres::Connection::poll_message(self, cx)
        }
    }

    struct PgCancel {
        token: tokio_postgres::CancelToken,
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
                options: None,
                target: Target::Unix {
                    socket_dir: socket_dir.into(),
                    port: 5432,
                },
            }
        }

        /// The port that names the socket file (`.s.PGSQL.<port>`) of a [`Self::unix_socket`]
        /// driver. A TCP driver keeps the port it was made with.
        #[cfg(unix)]
        #[must_use]
        pub fn port(mut self, value: u16) -> Self {
            if let Target::Unix { port, .. } = &mut self.target {
                *port = value;
            }
            self
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
                options: None,
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
                options: None,
                target: Target::Tcp {
                    host: host.into(),
                    port,
                    tls,
                },
            })
        }

        /// Server settings sent with each new session, the libpq `options` string
        /// (`-c statement_timeout=30000 -c default_transaction_read_only=on`).
        #[must_use]
        pub fn options(mut self, options: impl Into<String>) -> Self {
            self.options = Some(options.into());
            self
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
            if let Some(options) = &self.options {
                config.options(options);
            }
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| Error::new("could not start the postgres connection runtime").with_source(err))?;
            let (client, connection, tls) = match &self.target {
                #[cfg(unix)]
                Target::Unix { socket_dir, port } => {
                    config.host_path(socket_dir);
                    config.port(*port);
                    config.ssl_mode(SslMode::Disable);
                    let (client, connection) = open(&runtime, &config, NoTls)?;
                    (client, connection, TlsMode::None)
                }
                Target::TcpNoTls { host, port } => {
                    config.host(host);
                    config.port(*port);
                    config.ssl_mode(SslMode::Disable);
                    let (client, connection) = open(&runtime, &config, NoTls)?;
                    (client, connection, TlsMode::None)
                }
                Target::Tcp { host, port, tls } => {
                    config.host(host);
                    config.port(*port);
                    config.ssl_mode(SslMode::Require);
                    let (client, connection) = open(&runtime, &config, tls.clone())?;
                    (client, connection, TlsMode::Rustls(tls.clone()))
                }
            };
            let mut conn = PostgresConn {
                cancel: client.cancel_token(),
                client: Some(Arc::new(client)),
                wire: Wire {
                    runtime,
                    link: Link {
                        connection,
                        pending: VecDeque::new(),
                        failed: None,
                        notifications: VecDeque::new(),
                        dead: false,
                    },
                },
                cache: StatementLru::new(CACHE_CAP),
                in_transaction: false,
                streaming: false,
                stream_exhausted: false,
                stream_buf: VecDeque::new(),
                tls,
            };
            conn.batch("SET TIME ZONE 'UTC'", "set time zone")?;
            Ok(conn)
        }
    }

    /// Connect on `runtime`. The connection is driven by each call that waits, not by a spawned task.
    fn open<T>(runtime: &Runtime, config: &Config, tls: T) -> Result<(Client, Box<dyn Drive>), Error>
    where
        T: MakeTlsConnect<Socket>,
        T::Stream: Send + 'static,
    {
        let (client, connection) = runtime.block_on(config.connect(tls)).map_err(|err| pg("connect", err))?;
        Ok((client, Box::new(connection)))
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

        /// `LISTEN` on `channel`, an identifier the caller controls.
        ///
        /// # Errors
        ///
        /// Returns an error when the channel is not a plain identifier or the server refuses.
        pub fn listen(&mut self, channel: &str) -> Result<(), Error> {
            if channel.is_empty() || !channel.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                return Err(Error::new(format!("{channel:?} is not a channel name")));
            }
            self.batch(&format!("LISTEN {channel}"), "listen")
        }

        /// The payloads of the notifications that arrive within `timeout` of the call, and any already
        /// waiting behind the first. Empty when none came.
        ///
        /// # Errors
        ///
        /// Returns an error when the connection is lost.
        pub fn notifications(&mut self, timeout: std::time::Duration) -> Result<Vec<String>, Error> {
            let Wire { runtime, link } = &mut self.wire;
            let first = runtime.block_on(async {
                let arrived = poll_fn(|cx| match link.drive(cx) {
                    Err(err) => Poll::Ready(Err(err)),
                    Ok(()) if !link.notifications.is_empty() || link.dead => Poll::Ready(Ok(())),
                    Ok(()) => Poll::Pending,
                });
                tokio::time::timeout(timeout, arrived).await.unwrap_or(Ok(()))
            });
            first?;
            // Those right behind the first.
            runtime.block_on(poll_fn(|cx| Poll::Ready(link.drive(cx))))?;
            if link.dead && link.notifications.is_empty() {
                return Err(Error::new("postgres notification: the connection is closed"));
            }
            Ok(link
                .notifications
                .drain(..)
                .map(|notification| notification.payload().to_string())
                .collect())
        }

        /// True when the server has closed this connection.
        #[must_use]
        pub fn is_closed(&self) -> bool {
            self.wire.link.dead || self.client.as_ref().is_none_or(|client| client.is_closed())
        }

        /// A statement or several, by the simple query protocol, waited for.
        fn batch(&mut self, sql: &str, context: &str) -> Result<(), Error> {
            let client = client(&self.client)?;
            self.wire.wait(client.batch_execute(sql), context)
        }

        /// A statement by the simple query protocol, sent without waiting for its reply, with the next call that waits.
        fn send_batch(&mut self, sql: String, context: &'static str) -> Result<(), Error> {
            let client = Arc::clone(client(&self.client)?);
            self.wire.send(
                Box::pin(async move { client.batch_execute(&sql).await.map_err(|err| pg(context, err)) }),
                false,
            )
        }

        fn prepared(&mut self, sql: &str) -> Result<Prepared, Error> {
            if let Some(prepared) = self.cache.get(sql) {
                return Ok(prepared.clone());
            }
            let rewritten = rewrite_placeholders(sql);
            let client = client(&self.client)?;
            let statement = self.wire.wait(client.prepare(&rewritten), "prepare")?;
            let prepared = Prepared {
                names: column_names(statement.columns()),
                statement,
            };
            self.cache.insert(sql, prepared.clone());
            Ok(prepared)
        }

        /// `COMMIT` waits for the deferred requests first and fails with the first of them that failed: the transaction
        /// is then aborted, and a `COMMIT` would roll it back without an error. `ROLLBACK` drops their failures. `COMMIT`
        /// closes an open stream's cursor first. `ROLLBACK` does not, since it closes the cursor itself and a `CLOSE` in
        /// an aborted transaction would fail before the rollback is sent. Either way the stream is gone.
        fn finish_transaction(&mut self, sql: &str) -> Result<(), Error> {
            let commit = sql == "COMMIT";
            if commit {
                self.wire.finish()?;
            } else {
                self.wire.abandon();
            }
            let close = self.streaming && commit;
            let closed = if close { self.close_stream() } else { Ok(()) };
            self.clear_stream();
            closed?;
            self.batch(sql, sql)?;
            self.in_transaction = false;
            Ok(())
        }

        fn fetch_stream(&mut self) -> Result<(), Error> {
            let sql = format!("FETCH {FETCH_BATCH} FROM {STREAM_CURSOR}");
            let client = client(&self.client)?;
            let rows = self.wire.wait(client.query(sql.as_str(), &[]), "fetch cursor")?;
            if rows.len() < FETCH_BATCH {
                self.stream_exhausted = true;
            }
            let names = rows.first().map(|row| column_names(row.columns())).unwrap_or_default();
            for row in &rows {
                self.stream_buf.push_back(map_row(row, &names)?);
            }
            Ok(())
        }

        fn clear_stream(&mut self) {
            self.streaming = false;
            self.stream_exhausted = false;
            self.stream_buf.clear();
        }
    }

    impl Drop for PostgresConn {
        /// As the sync client closes: with the last client gone (the pending requests hold it too) the connection sends
        /// `Terminate` and ends.
        fn drop(&mut self) {
            self.wire.link.pending.clear();
            self.client = None;
            let Wire { runtime, link } = &mut self.wire;
            if !link.dead {
                runtime.block_on(poll_fn(|cx| match link.drive(cx) {
                    Ok(()) if !link.dead => Poll::Pending,
                    _ => Poll::Ready(()),
                }));
            }
        }
    }

    impl Wire {
        /// Run `request` until its reply is in. The requests sent before it are answered first, and the first of them
        /// that failed is the error returned: in a transaction, that failure is what made the later statements fail.
        fn wait<T>(
            &mut self,
            request: impl Future<Output = Result<T, tokio_postgres::Error>>,
            context: &str,
        ) -> Result<T, Error> {
            let Wire { runtime, link } = self;
            let mut request = pin!(request);
            let result = runtime.block_on(poll_fn(|cx| {
                // The first poll queues the request; the connection then writes it and reads the replies.
                if let Poll::Ready(result) = request.as_mut().poll(cx) {
                    return Poll::Ready(Ok(result));
                }
                if let Err(err) = link.drive(cx) {
                    return Poll::Ready(Err(err));
                }
                link.settle(cx);
                request.as_mut().poll(cx).map(Ok)
            }));
            // The replies of the requests before it came first: take them.
            if !link.pending.is_empty() {
                runtime.block_on(poll_fn(|cx| {
                    link.settle(cx);
                    Poll::Ready(())
                }));
            }
            if let Some(err) = link.failed.take() {
                return Err(err);
            }
            match result {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(err)) => Err(pg(context, err)),
                Err(err) => Err(err),
            }
        }

        /// Send `request` without waiting for its reply, which a later call takes. With `write` it goes out now, so
        /// the server runs it while this thread goes on; without, it goes out with the next call that waits, in the
        /// same write (a `BEGIN`, which the server has nothing to do for, rides with the first statement).
        fn send(&mut self, request: Pending, write: bool) -> Result<(), Error> {
            let Wire { runtime, link } = self;
            if link.dead {
                return Err(Error::new("postgres: the connection is closed"));
            }
            link.pending.push_back(request);
            if write {
                runtime.block_on(poll_fn(|cx| {
                    link.queue_last(cx);
                    let driven = link.drive(cx);
                    link.settle(cx);
                    Poll::Ready(driven)
                }))?;
            } else {
                link.queue_last(&mut Context::from_waker(Waker::noop()));
            }
            if self.link.pending.len() >= PIPELINE_DEPTH {
                self.settle_to(PIPELINE_DEPTH / 2)?;
            }
            Ok(())
        }

        /// Wait until no more than `keep` requests are unanswered.
        fn settle_to(&mut self, keep: usize) -> Result<(), Error> {
            let Wire { runtime, link } = self;
            if link.pending.len() <= keep {
                return Ok(());
            }
            runtime.block_on(poll_fn(|cx| {
                if let Err(err) = link.drive(cx) {
                    return Poll::Ready(Err(err));
                }
                link.settle(cx);
                if link.pending.len() <= keep {
                    Poll::Ready(Ok(()))
                } else {
                    Poll::Pending
                }
            }))
        }

        /// Every reply in, and the first failure among them.
        fn finish(&mut self) -> Result<(), Error> {
            self.settle_to(0)?;
            self.link.failed.take().map_or(Ok(()), Err)
        }

        /// Every reply in, or the connection lost, and their failures dropped.
        fn abandon(&mut self) {
            let _ = self.settle_to(0);
            self.link.pending.clear();
            self.link.failed = None;
        }
    }

    impl Link {
        /// The first poll of the request just sent, which queues it for the connection. A request that ends there (a
        /// parameter that does not encode) failed.
        fn queue_last(&mut self, cx: &mut Context<'_>) {
            if let Some(request) = self.pending.back_mut() {
                if let Poll::Ready(result) = request.as_mut().poll(cx) {
                    self.pending.pop_back();
                    if let Err(err) = result {
                        self.failed.get_or_insert(err);
                    }
                }
            }
        }

        /// Write and read what the socket allows now. A notification is kept for [`PostgresConn::notifications`], a
        /// notice is logged.
        fn drive(&mut self, cx: &mut Context<'_>) -> Result<(), Error> {
            while !self.dead {
                match self.connection.poll_message(cx) {
                    Poll::Ready(Some(Ok(AsyncMessage::Notification(notification)))) => {
                        self.notifications.push_back(notification);
                    }
                    Poll::Ready(Some(Ok(AsyncMessage::Notice(notice)))) => {
                        tracing::info!("postgres {}: {}", notice.severity(), notice.message());
                    }
                    Poll::Ready(Some(Ok(_))) => {}
                    Poll::Ready(Some(Err(err))) => {
                        self.dead = true;
                        return Err(pg("connection", err));
                    }
                    Poll::Ready(None) => self.dead = true,
                    Poll::Pending => break,
                }
            }
            Ok(())
        }

        /// Take the replies that are in, oldest first, up to the first request still waiting for its own.
        fn settle(&mut self, cx: &mut Context<'_>) {
            while let Some(request) = self.pending.front_mut() {
                let Poll::Ready(result) = request.as_mut().poll(cx) else {
                    break;
                };
                self.pending.pop_front();
                if let Err(err) = result {
                    self.failed.get_or_insert(err);
                }
            }
        }
    }

    fn client(slot: &Option<Arc<Client>>) -> Result<&Arc<Client>, Error> {
        slot.as_ref()
            .ok_or_else(|| Error::new("postgres: the connection is closed"))
    }

    impl DbConnection for PostgresConn {
        fn is_closed(&self) -> bool {
            PostgresConn::is_closed(self)
        }

        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Error> {
            check_scalar(params)?;
            let prepared = self.prepared(sql)?;
            let client = client(&self.client)?;
            self.wire
                .wait(client.execute_raw(&prepared.statement, params.iter().map(Param)), "execute")
        }

        /// In a transaction the statement is sent and its reply left for the next call that waits, or the commit. Out
        /// of one it runs as [`Self::execute`]: the connection goes back to the pool after it.
        fn execute_deferred(&mut self, sql: &str, params: Vec<Value>, zero_rows: Option<String>) -> Result<(), Error> {
            if !self.in_transaction {
                let count = self.execute(sql, &params)?;
                return match zero_rows {
                    Some(message) if count == 0 => Err(Error::new(message)),
                    _ => Ok(()),
                };
            }
            check_scalar(&params)?;
            let statement = self.prepared(sql)?.statement;
            let client = Arc::clone(client(&self.client)?);
            self.wire.send(Box::pin(async move {
                let count = client
                    .execute_raw(&statement, params.iter().map(Param))
                    .await
                    .map_err(|err| pg("execute", err))?;
                match zero_rows {
                    Some(message) if count == 0 => Err(Error::new(message)),
                    _ => Ok(()),
                }
            }), true)
        }

        fn finish_deferred(&mut self) -> Result<(), Error> {
            self.wire.finish()
        }

        fn query(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            let bound = bind(params)?;
            let refs = param_refs(&bound);
            let prepared = self.prepared(sql)?;
            let client = client(&self.client)?;
            let rows = self.wire.wait(client.query(&prepared.statement, &refs), "query")?;
            rows.iter().map(|row| map_row(row, &prepared.names)).collect()
        }

        fn returns_rows(&mut self, sql: &str) -> Result<Option<bool>, Error> {
            // A deferred failure is reported, not taken for this statement's.
            self.wire.finish()?;
            // Not kept in the statement cache: a statement typed in the SQL runner runs once. One that does not prepare
            // is left to run, so its own error is the one reported.
            let rewritten = rewrite_placeholders(sql);
            let client = client(&self.client)?;
            match self.wire.wait(client.prepare(&rewritten), "prepare") {
                Ok(statement) => Ok(Some(!statement.columns().is_empty())),
                Err(_) => Ok(None),
            }
        }

        fn query_once(&mut self, sql: &str, params: &[Value]) -> Result<Vec<DbRow>, Error> {
            let bound = bind(params)?;
            let refs = param_refs(&bound);
            let rewritten = rewrite_placeholders(sql);
            let client = client(&self.client)?;
            let rows = self.wire.wait(client.query(rewritten.as_str(), &refs), "query")?;
            let names = rows.first().map(|row| column_names(row.columns())).unwrap_or_default();
            rows.iter().map(|row| map_row(row, &names)).collect()
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
            let client = client(&self.client)?;
            self.wire
                .wait(client.execute(declare.as_str(), &refs), "declare cursor")?;
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
            self.batch(&format!("CLOSE {STREAM_CURSOR}"), "close cursor")?;
            self.clear_stream();
            Ok(())
        }

        fn stream_open(&self) -> bool {
            self.streaming
        }

        /// Sent without waiting: the first statement of the transaction takes its reply, in the same round trip.
        fn begin(&mut self) -> Result<(), Error> {
            self.send_batch("BEGIN".to_string(), "begin")?;
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
            self.batch(&format!("SAVEPOINT {}", quote_ident(name)), "savepoint")
        }

        fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), Error> {
            self.batch(
                &format!("ROLLBACK TO SAVEPOINT {}", quote_ident(name)),
                "rollback to savepoint",
            )
        }

        fn release_savepoint(&mut self, name: &str) -> Result<(), Error> {
            self.batch(&format!("RELEASE SAVEPOINT {}", quote_ident(name)), "release savepoint")
        }

        /// `SET LOCAL` lasts until the transaction ends, so clearing is a no-op. Sent without waiting, as `BEGIN` is.
        fn set_statement_timeout(&mut self, remaining: Option<std::time::Duration>) -> Result<(), Error> {
            let Some(remaining) = remaining else {
                return Ok(());
            };
            let ms = crate::db::timeout_millis(remaining)?;
            self.send_batch(format!("SET LOCAL statement_timeout = {ms}"), "statement_timeout")
        }

        fn canceller(&self) -> Box<dyn DbCancel> {
            Box::new(PgCancel {
                token: self.cancel.clone(),
                tls: self.tls.clone(),
            })
        }
    }

    impl DbCancel for PgCancel {
        /// On the calling thread, the deadline thread, with a runtime of its own as the sync client's cancel had.
        fn cancel(&self) -> Result<(), Error> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| Error::new("could not start the postgres cancel runtime").with_source(err))?;
            let result = match &self.tls {
                TlsMode::None => runtime.block_on(self.token.cancel_query(NoTls)),
                TlsMode::Rustls(tls) => runtime.block_on(self.token.cancel_query(tls.clone())),
            };
            result.map_err(|err| pg("cancel", err))
        }
    }

    impl std::fmt::Debug for Param<'_> {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Debug::fmt(self.0, formatter)
        }
    }

    impl ToSql for Param<'_> {
        fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
            if let (Value::Bytes(bytes), true) = (self.0, ty == &Type::BYTEA) {
                out.extend_from_slice(bytes);
                return Ok(IsNull::No);
            }
            // The common kinds go straight into the buffer, as `value_text` writes them.
            use std::fmt::Write as _;
            let written = match self.0 {
                Value::Null => return Ok(IsNull::Yes),
                Value::String(text) => {
                    out.extend_from_slice(text.as_bytes());
                    Ok(())
                }
                Value::Bool(flag) => {
                    out.extend_from_slice(if *flag { b"true" } else { b"false" });
                    Ok(())
                }
                Value::I32(number) => write!(out, "{number}"),
                Value::I64(number) => write!(out, "{number}"),
                Value::Decimal(number) => write!(out, "{number}"),
                Value::Timestamp(timestamp) => write!(out, "{timestamp}"),
                Value::Date(date) => write!(out, "{date}"),
                Value::Time(time) => write!(out, "{time}"),
                _ => return self.to_sql_text(out),
            };
            written.map_err(|_| bad("could not write a postgres parameter"))?;
            Ok(IsNull::No)
        }

        fn accepts(_ty: &Type) -> bool {
            true
        }

        /// Text, so the server parses each value as its column's type, except bytes for a `bytea`: sent as they are
        /// rather than as hex text twice their size (which also overflows the 1 GB field limit at half of it).
        fn encode_format(&self, ty: &Type) -> Format {
            if ty == &Type::BYTEA && matches!(self.0, Value::Bytes(_)) {
                Format::Binary
            } else {
                Format::Text
            }
        }

        tokio_postgres::types::to_sql_checked!();
    }

    impl Param<'_> {
        fn to_sql_text(&self, out: &mut BytesMut) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
            match value_text(self.0)? {
                Some(text) => {
                    out.extend_from_slice(text.as_bytes());
                    Ok(IsNull::No)
                }
                None => Ok(IsNull::Yes),
            }
        }
    }

    impl<'a> FromSql<'a> for PgValue {
        fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
            Ok(PgValue(decode_value(ty, raw)?))
        }

        fn accepts(_ty: &Type) -> bool {
            true
        }
    }

    fn pg(context: &str, err: tokio_postgres::Error) -> Error {
        // `Display` for a server error is the text "db error". The server message is the useful part.
        let detail = match err.as_db_error() {
            Some(db) => db.message().to_string(),
            None => err.to_string(),
        };
        Error::new(format!("postgres {context}: {detail}")).with_source(err)
    }

    fn bind(params: &[Value]) -> Result<Vec<Param<'_>>, Error> {
        check_scalar(params)?;
        Ok(params.iter().map(Param).collect())
    }

    fn check_scalar(params: &[Value]) -> Result<(), Error> {
        if params
            .iter()
            .any(|value| matches!(value, Value::List(_) | Value::Map(_)))
        {
            return Err(Error::new("postgres parameters must be scalar values"));
        }
        Ok(())
    }

    fn param_refs<'a>(params: &'a [Param<'a>]) -> Vec<&'a (dyn ToSql + Sync)> {
        params.iter().map(|param| param as &(dyn ToSql + Sync)).collect()
    }

    /// JDBC `?` placeholders as `$1`, `$2`, ... A `?` inside a string (`'..'`, `E'..'`, `$tag$..$tag$`), a quoted
    /// identifier or a comment is left as it is.
    fn rewrite_placeholders(sql: &str) -> String {
        let bytes = sql.as_bytes();
        let mut out = String::with_capacity(sql.len() + 8);
        let mut index = 0u32;
        let mut copied = 0;
        let mut at = 0;
        while let Some(&byte) = bytes.get(at) {
            let next = bytes.get(at + 1).copied();
            at = match byte {
                b'\'' => quoted_end(bytes, at + 1, b'\'', escape_string(bytes, at)),
                b'"' => quoted_end(bytes, at + 1, b'"', false),
                b'-' if next == Some(b'-') => bytes[at..]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(bytes.len(), |offset| at + offset + 1),
                b'/' if next == Some(b'*') => block_comment_end(bytes, at + 2),
                b'$' => dollar_quote_end(sql, at).unwrap_or(at + 1),
                b'?' => {
                    index += 1;
                    out.push_str(&sql[copied..at]);
                    out.push('$');
                    out.push_str(&index.to_string());
                    copied = at + 1;
                    at + 1
                }
                _ => at + 1,
            };
        }
        out.push_str(&sql[copied..]);
        out
    }

    /// True when the quote at `at` opens an `E'..'` string, in which a backslash escapes the next character.
    fn escape_string(bytes: &[u8], at: usize) -> bool {
        let Some(prefix) = at.checked_sub(1).and_then(|before| bytes.get(before)) else {
            return false;
        };
        let word_before = at
            .checked_sub(2)
            .and_then(|before| bytes.get(before))
            .is_some_and(|&byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$');
        matches!(prefix, b'E' | b'e') && !word_before
    }

    /// The index past the `quote` that closes a string or identifier whose body starts at `from`. A doubled quote, or
    /// with `backslash` a backslash, escapes.
    fn quoted_end(bytes: &[u8], from: usize, quote: u8, backslash: bool) -> usize {
        let mut at = from;
        while let Some(&byte) = bytes.get(at) {
            if backslash && byte == b'\\' {
                at += 2;
            } else if byte == quote {
                if bytes.get(at + 1) != Some(&quote) {
                    return at + 1;
                }
                at += 2;
            } else {
                at += 1;
            }
        }
        bytes.len()
    }

    /// The index past the `*/` that closes a block comment whose body starts at `from`. Postgres comments nest.
    fn block_comment_end(bytes: &[u8], from: usize) -> usize {
        let mut depth = 1usize;
        let mut at = from;
        while let Some(&byte) = bytes.get(at) {
            let next = bytes.get(at + 1).copied();
            if byte == b'/' && next == Some(b'*') {
                depth += 1;
                at += 2;
            } else if byte == b'*' && next == Some(b'/') {
                depth -= 1;
                at += 2;
                if depth == 0 {
                    return at;
                }
            } else {
                at += 1;
            }
        }
        bytes.len()
    }

    /// For a `$tag$` (or `$$`) at `at` that opens a dollar-quoted string, the index past its closing tag. `None` when
    /// the `$` opens no tag, as in a `$1` parameter, or follows an identifier character.
    fn dollar_quote_end(sql: &str, at: usize) -> Option<usize> {
        let bytes = sql.as_bytes();
        let after_word = at
            .checked_sub(1)
            .and_then(|before| bytes.get(before))
            .is_some_and(|&byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if after_word {
            return None;
        }
        let rest = sql.get(at + 1..)?;
        let tag_len = rest.find('$')?;
        let tag = &rest[..tag_len];
        let valid = tag.bytes().enumerate().all(|(position, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (position > 0 && byte.is_ascii_digit())
        });
        if !valid {
            return None;
        }
        let delimiter = &sql[at..at + tag_len + 2];
        let body = at + tag_len + 2;
        Some(
            sql[body..]
                .find(delimiter)
                .map_or(sql.len(), |offset| body + offset + delimiter.len()),
        )
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

    fn column_names(columns: &[tokio_postgres::Column]) -> Arc<[Arc<str>]> {
        columns.iter().map(|column| Arc::from(column.name())).collect()
    }

    /// `names` are the row's column names, shared by the rows of its statement.
    fn map_row(row: &tokio_postgres::Row, names: &[Arc<str>]) -> Result<DbRow, Error> {
        let mut columns = Vec::with_capacity(row.len());
        for (index, column) in row.columns().iter().enumerate() {
            let value = row
                .try_get::<_, Option<PgValue>>(index)
                .map_err(|err| Error::new(format!("postgres column {index}")).with_source(err))?;
            let name = names.get(index).map_or_else(|| Arc::from(column.name()), Arc::clone);
            columns.push((name, value.map(|cell| cell.0).unwrap_or(Value::Null)));
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
            expect_value(numeric_to_text(raw)?, "BigDecimal")
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
        // The moment the text below names (UTC, no offset), without writing and parsing that text. Outside jiff's range
        // the text decides.
        if let Some(stamp) = micros
            .checked_add(PG_EPOCH_USEC)
            .and_then(|unix_us| jiff::Timestamp::from_microsecond(unix_us).ok())
        {
            return Ok(Value::Timestamp(stamp));
        }
        expect_value(format_pg_timestamp(micros)?, "Timestamp")
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
        expect_value(format!("{year:04}-{month:02}-{day:02}"), "Date")
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
        expect_value(format!("{hour:02}:{minute:02}:{second:02}.{frac:06}"), "Time")
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

    fn expect_value(text: String, kind: &str) -> Result<Value, Box<dyn std::error::Error + Sync + Send>> {
        let text = Value::String(text);
        let converted = basic_convert(&text, kind);
        if kind_matches(&converted.value, kind) {
            Ok(converted.value)
        } else {
            Err(bad(&format!("could not decode {kind} from {}", text.as_str().unwrap_or_default())))
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
        let digit_at = |index: usize| {
            let offset = 8 + index * 2;
            u16::from_be_bytes([raw[offset], raw[offset + 1]])
        };
        if (0..ndigits).any(|index| digit_at(index) >= 10_000) {
            return Err(bad("numeric digit"));
        }
        let leading = if weight >= 0 {
            0
        } else {
            let leading = usize::try_from(-weight - 1).map_err(|_| bad("numeric weight"))?;
            if leading > 2_000 {
                return Err(bad("numeric too large"));
            }
            leading
        };
        // Written into one string: base-10000 digit groups, the first without leading zeros, then the fraction to
        // `dscale` digits.
        use std::fmt::Write as _;
        let mut text = String::with_capacity(ndigits * 4 + leading * 4 + dscale + 8);
        if sign == 0x4000 {
            text.push('-');
        }
        let groups = if weight >= 0 {
            let groups = usize::try_from(weight + 1).map_err(|_| bad("numeric weight"))?;
            for index in 0..groups {
                let digit = if index < ndigits { digit_at(index) } else { 0 };
                let _ = if index == 0 {
                    write!(text, "{digit}")
                } else {
                    write!(text, "{digit:04}")
                };
            }
            groups
        } else {
            text.push('0');
            0
        };
        if dscale > 0 {
            text.push('.');
            let fraction_start = text.len();
            for _ in 0..leading {
                text.push_str("0000");
            }
            for index in groups.min(ndigits)..ndigits {
                let _ = write!(text, "{:04}", digit_at(index));
            }
            let fraction_end = fraction_start + dscale;
            if text.len() < fraction_end {
                let pad = fraction_end - text.len();
                text.extend(std::iter::repeat_n('0', pad));
            } else {
                text.truncate(fraction_end);
            }
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
        fn a_parameter_writes_the_text_value_text_gives() {
            let values = [
                Value::Null,
                Value::string("it's"),
                Value::Bool(true),
                Value::Bool(false),
                Value::I32(-5),
                Value::I64(1 << 40),
                basic_convert(&Value::string("12.340"), "BigDecimal").value,
                Value::Timestamp(jiff::Timestamp::from_microsecond(1_700_000_000_123_456).unwrap()),
                Value::Date(jiff::civil::date(2024, 2, 29)),
                Value::Time(jiff::civil::time(13, 4, 5, 123_000_000)),
                Value::F64(1.5),
                Value::F64(f64::NEG_INFINITY),
            ];
            for value in &values {
                let mut out = BytesMut::new();
                let null = Param(value).to_sql(&Type::TEXT, &mut out).unwrap();
                match value_text(value).unwrap() {
                    Some(text) => {
                        assert!(matches!(null, IsNull::No), "{value:?}");
                        assert_eq!(&out[..], text.as_bytes(), "{value:?}");
                    }
                    None => assert!(matches!(null, IsNull::Yes), "{value:?}"),
                }
            }
        }

        #[test]
        fn a_binary_timestamp_decodes_to_the_moment_its_text_names() {
            for micros in [0_i64, 1, -1, 999_999, 813_456_789_012_345, -946_684_800_000_000, 2_000_000_000_000_000] {
                let raw = micros.to_be_bytes();
                let direct = decode_timestamp(&raw).unwrap();
                let text = expect_value(format_pg_timestamp(micros).unwrap(), "Timestamp").unwrap();
                assert_eq!(direct, text, "{micros}");
            }
        }

        #[test]
        fn placeholders_skip_quoted_question_marks() {
            assert_eq!(rewrite_placeholders("SELECT ?"), "SELECT $1");
            assert_eq!(rewrite_placeholders("SELECT ?, ?"), "SELECT $1, $2");
            assert_eq!(rewrite_placeholders("SELECT '?'"), "SELECT '?'");
            assert_eq!(rewrite_placeholders("SELECT 'it''s ?', ?"), "SELECT 'it''s ?', $1");
            assert_eq!(rewrite_placeholders("SELECT 'a''?''b' || ?"), "SELECT 'a''?''b' || $1");
        }

        #[test]
        fn placeholders_skip_identifiers_comments_and_other_strings() {
            assert_eq!(rewrite_placeholders(r#"SELECT "a?b", ?"#), r#"SELECT "a?b", $1"#);
            assert_eq!(rewrite_placeholders(r#"SELECT "a""?", ?"#), r#"SELECT "a""?", $1"#);
            assert_eq!(rewrite_placeholders("SELECT ? -- why?\n, ?"), "SELECT $1 -- why?\n, $2");
            assert_eq!(rewrite_placeholders("SELECT ? -- why?"), "SELECT $1 -- why?");
            assert_eq!(
                rewrite_placeholders("SELECT /* a? /* b? */ c? */ ?"),
                "SELECT /* a? /* b? */ c? */ $1"
            );
            assert_eq!(rewrite_placeholders("SELECT $$a?b$$, ?"), "SELECT $$a?b$$, $1");
            assert_eq!(
                rewrite_placeholders("SELECT $fn$ ? $x$ ? $fn$, ?"),
                "SELECT $fn$ ? $x$ ? $fn$, $1"
            );
            assert_eq!(rewrite_placeholders(r"SELECT E'a\'?', ?"), r"SELECT E'a\'?', $1");
            assert_eq!(rewrite_placeholders(r"SELECT e'\\', ?"), r"SELECT e'\\', $1");
            // A backslash in a standard string is a plain character, and `name'..'` is no E-string.
            assert_eq!(rewrite_placeholders(r"SELECT '\', ?"), r"SELECT '\', $1");
            assert_eq!(rewrite_placeholders(r"SELECT some'\', ?"), r"SELECT some'\', $1");
            // `$1` and `a$b` open no dollar quote.
            assert_eq!(rewrite_placeholders("SELECT $1, a$b, ?"), "SELECT $1, a$b, $1");
            assert_eq!(rewrite_placeholders("SELECT 'é?', ?"), "SELECT 'é?', $1");
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
