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

//! The Postgres frontend/backend protocol (version 3) over a blocking socket, for [`super::postgres`].
//!
//! Messages are encoded into one write buffer and sent with one write; replies are read into one read buffer. A
//! round trip is then a write and a read, where an async client adds readiness polls and wakeups. Every request ends
//! with `Sync`, so each has its own `ReadyForQuery` and a failed one does not take the next down with it, and a
//! request can be left unanswered ([`Wire::queue`]): its reply is read, and its failure kept, by the next call that
//! waits. The encoding and the parsing of messages, and SCRAM, are `postgres-protocol`'s.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use fallible_iterator::FallibleIterator;
use moqui_core::driver::DbRow;
use moqui_lib::error::Error;
use postgres_protocol::authentication::sasl::{ChannelBinding, ScramSha256, SCRAM_SHA_256};
use postgres_protocol::message::backend::{DataRowBody, ErrorFields, Message, ParameterDescriptionBody, RowDescriptionBody};
use postgres_protocol::message::frontend;
use postgres_types::{Kind, Type};

/// Requests sent and not waited for that may be unanswered. A queue past this waits for the older half.
const PIPELINE_DEPTH: usize = 256;
/// Bytes asked of the socket at a time.
const READ_CHUNK: usize = 16 * 1024;
/// Every result column comes in binary.
const BINARY: i16 = 1;
const TEXT: i16 = 0;

/// Where a connection goes. TLS is for TCP only.
#[derive(Clone)]
pub(super) enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    Tcp {
        host: String,
        port: u16,
        tls: Option<Arc<rustls::ClientConfig>>,
    },
}

/// A result column of a statement.
pub(super) struct Column {
    /// Shared by the rows of the statement.
    pub(super) name: Arc<str>,
    pub(super) ty: Type,
}

/// What a prepared statement takes and gives.
pub(super) struct Described {
    pub(super) params: Vec<Type>,
    pub(super) columns: Arc<[Column]>,
}

/// Parameter `index`'s bytes, written into the message in the format the request names for it; `false` for a NULL.
pub(super) type Encode<'a> = &'a mut dyn FnMut(usize, &mut BytesMut) -> Result<bool, Error>;
/// One result cell from its type and bytes (`None` for a SQL NULL).
pub(super) type Decode<'a> = &'a dyn Fn(&Type, Option<&[u8]>) -> Result<moqui_lib::object::Value, Error>;

/// Which `Describe` a request carries.
enum Describe {
    None,
    /// Of the statement it parses: its parameter types and columns, for later runs.
    Statement,
    /// Of the portal it binds: the columns of its rows.
    Portal,
}

/// What the reply to an unanswered request must hold.
enum Expect {
    /// A simple query, its rows unread.
    Simple { context: &'static str },
    /// An execute: a failure, or `zero_rows` as the error when it changed no row.
    Execute { zero_rows: Option<String> },
    /// The close of a prepared statement the cache let go.
    Close,
}

pub(super) struct Wire {
    stream: Stream,
    read: BytesMut,
    write: BytesMut,
    /// Oldest first. Replies come in the order the requests were sent.
    pending: VecDeque<Expect>,
    /// The first failure among `pending`, for the next call that waits.
    failed: Option<Error>,
    notifications: VecDeque<String>,
    /// The connection failed or ended.
    dead: bool,
    process_id: i32,
    secret_key: i32,
}

/// What failed a request: the server, or this side while reading the reply (a cell that does not decode).
enum Failure {
    Db(DbError),
    Local(Error),
}

impl Failure {
    fn into_error(self, context: &str) -> Error {
        match self {
            Failure::Db(db) => db.into_error(context),
            Failure::Local(err) => err,
        }
    }
}

/// The parts of the server's `ErrorResponse` a message shows, as `tokio_postgres`'s `DbError` showed them.
#[derive(Debug)]
pub(super) struct DbError {
    severity: String,
    message: String,
    detail: Option<String>,
    hint: Option<String>,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "db error: {}: {}", self.severity, self.message)?;
        if let Some(detail) = &self.detail {
            write!(f, "\nDETAIL: {detail}")?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\nHINT: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DbError {}

impl DbError {
    fn parse(mut fields: ErrorFields<'_>) -> Self {
        let mut error = DbError {
            severity: String::new(),
            message: String::new(),
            detail: None,
            hint: None,
        };
        while let Ok(Some(field)) = fields.next() {
            let value = || String::from_utf8_lossy(field.value_bytes()).into_owned();
            match field.type_() {
                b'S' => error.severity = value(),
                b'M' => error.message = value(),
                b'D' => error.detail = Some(value()),
                b'H' => error.hint = Some(value()),
                _ => {}
            }
        }
        error
    }

    /// `postgres {context}: {message}`, the server's message as the text and the whole of it as the source.
    fn into_error(self, context: &str) -> Error {
        Error::new(format!("postgres {context}: {}", self.message)).with_source(self)
    }
}

enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn open(endpoint: &Endpoint) -> Result<Self, Error> {
        match endpoint {
            #[cfg(unix)]
            Endpoint::Unix(path) => UnixStream::connect(path)
                .map(Stream::Unix)
                .map_err(|err| io_error(&format!("connect {}", path.display()), err)),
            Endpoint::Tcp { host, port, tls } => {
                let tcp = TcpStream::connect((host.as_str(), *port))
                    .map_err(|err| io_error(&format!("connect {host}:{port}"), err))?;
                tcp.set_nodelay(true).map_err(|err| io_error("connect", err))?;
                match tls {
                    None => Ok(Stream::Tcp(tcp)),
                    Some(config) => negotiate_tls(tcp, host, config),
                }
            }
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(stream) => stream.set_read_timeout(timeout),
            Stream::Tcp(stream) => stream.set_read_timeout(timeout),
            Stream::Tls(stream) => stream.sock.set_read_timeout(timeout),
        }
    }

    fn set_nonblocking(&self, on: bool) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(stream) => stream.set_nonblocking(on),
            Stream::Tcp(stream) => stream.set_nonblocking(on),
            Stream::Tls(stream) => stream.sock.set_nonblocking(on),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(stream) => stream.read(buf),
            Stream::Tcp(stream) => stream.read(buf),
            Stream::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(stream) => stream.write(buf),
            Stream::Tcp(stream) => stream.write(buf),
            Stream::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(stream) => stream.flush(),
            Stream::Tcp(stream) => stream.flush(),
            Stream::Tls(stream) => stream.flush(),
        }
    }
}

/// `SSLRequest`, then the TLS handshake (rustls runs it on the first write).
fn negotiate_tls(mut tcp: TcpStream, host: &str, config: &Arc<rustls::ClientConfig>) -> Result<Stream, Error> {
    let mut request = BytesMut::new();
    frontend::ssl_request(&mut request);
    tcp.write_all(&request).map_err(|err| io_error("TLS request", err))?;
    let mut answer = [0u8; 1];
    tcp.read_exact(&mut answer)
        .map_err(|err| io_error("TLS request", err))?;
    if answer[0] != b'S' {
        return Err(Error::new("postgres connect: the server does not accept TLS"));
    }
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|err| Error::new(format!("postgres connect: {host} is not a TLS server name")).with_source(err))?;
    let connection = rustls::ClientConnection::new(Arc::clone(config), name)
        .map_err(|err| Error::new("postgres connect: TLS setup").with_source(err))?;
    Ok(Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp))))
}

fn io_error(context: &str, err: std::io::Error) -> Error {
    Error::new(format!("postgres {context}: {err}")).with_source(err)
}

fn protocol_error(what: &str) -> Error {
    Error::new(format!("postgres: unexpected {what} from the server"))
}

/// Rows a command tag counts: the last word of `INSERT 0 5`, `UPDATE 3`, `SELECT 4`; none for other commands.
fn tag_rows(tag: &str) -> u64 {
    tag.rsplit(' ').next().and_then(|count| count.parse().ok()).unwrap_or(0)
}

/// A type the server named by oid. One this build does not know is decoded as text, or hex.
fn type_of(oid: u32) -> Type {
    Type::from_oid(oid).unwrap_or_else(|| Type::new(format!("oid {oid}"), oid, Kind::Simple, String::new()))
}

impl Wire {
    /// Connect, authenticate (cleartext, MD5 or SCRAM-SHA-256) and wait for the server to be ready.
    pub(super) fn connect(
        endpoint: &Endpoint,
        user: &str,
        database: &str,
        password: Option<&str>,
        options: Option<&str>,
    ) -> Result<Self, Error> {
        let mut wire = Wire {
            stream: Stream::open(endpoint)?,
            read: BytesMut::with_capacity(READ_CHUNK),
            write: BytesMut::with_capacity(1024),
            pending: VecDeque::new(),
            failed: None,
            notifications: VecDeque::new(),
            dead: false,
            process_id: 0,
            secret_key: 0,
        };
        let mut parameters = vec![("user", user), ("database", database), ("client_encoding", "UTF8")];
        if let Some(options) = options {
            parameters.push(("options", options));
        }
        frontend::startup_message(parameters, &mut wire.write).map_err(|err| io_error("startup", err))?;
        wire.flush()?;
        wire.authenticate(user, password)?;
        loop {
            match wire.read_message()? {
                Message::BackendKeyData(body) => {
                    wire.process_id = body.process_id();
                    wire.secret_key = body.secret_key();
                }
                Message::ReadyForQuery(_) => break,
                Message::ErrorResponse(body) => return Err(DbError::parse(body.fields()).into_error("connect")),
                _ => {}
            }
        }
        Ok(wire)
    }

    fn authenticate(&mut self, user: &str, password: Option<&str>) -> Result<(), Error> {
        let need_password = || Error::new("postgres connect: the server asks for a password and none is set");
        loop {
            match self.read_message()? {
                Message::AuthenticationOk => return Ok(()),
                Message::AuthenticationCleartextPassword => {
                    let password = password.ok_or_else(need_password)?;
                    frontend::password_message(password.as_bytes(), &mut self.write)
                        .map_err(|err| io_error("password", err))?;
                    self.flush()?;
                }
                Message::AuthenticationMd5Password(body) => {
                    let password = password.ok_or_else(need_password)?;
                    let hash =
                        postgres_protocol::authentication::md5_hash(user.as_bytes(), password.as_bytes(), body.salt());
                    frontend::password_message(hash.as_bytes(), &mut self.write)
                        .map_err(|err| io_error("password", err))?;
                    self.flush()?;
                }
                Message::AuthenticationSasl(body) => {
                    let password = password.ok_or_else(need_password)?;
                    let mut mechanisms = body.mechanisms();
                    let mut scram = false;
                    while let Some(mechanism) = mechanisms.next().map_err(|err| io_error("SASL", err))? {
                        scram |= mechanism == SCRAM_SHA_256;
                    }
                    if !scram {
                        return Err(Error::new(
                            "postgres connect: the server offers no SASL mechanism this client has",
                        ));
                    }
                    let mut sasl = ScramSha256::new(password.as_bytes(), ChannelBinding::unsupported());
                    frontend::sasl_initial_response(SCRAM_SHA_256, sasl.message(), &mut self.write)
                        .map_err(|err| io_error("SASL", err))?;
                    self.flush()?;
                    match self.read_message()? {
                        Message::AuthenticationSaslContinue(body) => {
                            sasl.update(body.data()).map_err(|err| io_error("SASL", err))?;
                        }
                        Message::ErrorResponse(body) => return Err(DbError::parse(body.fields()).into_error("connect")),
                        _ => return Err(protocol_error("SASL reply")),
                    }
                    frontend::sasl_response(sasl.message(), &mut self.write).map_err(|err| io_error("SASL", err))?;
                    self.flush()?;
                    match self.read_message()? {
                        Message::AuthenticationSaslFinal(body) => {
                            sasl.finish(body.data()).map_err(|err| io_error("SASL", err))?;
                        }
                        Message::ErrorResponse(body) => return Err(DbError::parse(body.fields()).into_error("connect")),
                        _ => return Err(protocol_error("SASL reply")),
                    }
                }
                Message::ErrorResponse(body) => return Err(DbError::parse(body.fields()).into_error("connect")),
                _ => {
                    return Err(Error::new(
                        "postgres connect: the server asks for an authentication this client has not",
                    ))
                }
            }
        }
    }

    pub(super) fn is_dead(&self) -> bool {
        self.dead
    }

    /// The handle a cancel request names this connection's backend by.
    pub(super) fn cancel_key(&self) -> (i32, i32) {
        (self.process_id, self.secret_key)
    }

    fn flush(&mut self) -> Result<(), Error> {
        if self.write.is_empty() {
            return Ok(());
        }
        let written = self.stream.write_all(&self.write).and_then(|()| self.stream.flush());
        self.write.clear();
        written.map_err(|err| {
            self.dead = true;
            io_error("write", err)
        })
    }

    /// More bytes from the socket. An end of stream is a closed connection.
    fn fill(&mut self) -> Result<(), Error> {
        let mut chunk = [0u8; READ_CHUNK];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.dead = true;
                    return Err(Error::new("postgres: the server closed the connection"));
                }
                Ok(count) => {
                    self.read.extend_from_slice(&chunk[..count]);
                    return Ok(());
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) => {
                    if !matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
                        self.dead = true;
                    }
                    return Err(io_error("read", err));
                }
            }
        }
    }

    /// The next message that is not a notice, a notification (kept) or a parameter status.
    fn read_message(&mut self) -> Result<Message, Error> {
        loop {
            if let Some(message) = self.parse_buffered()? {
                return Ok(message);
            }
            self.fill()?;
        }
    }

    fn parse_buffered(&mut self) -> Result<Option<Message>, Error> {
        loop {
            let message = match Message::parse(&mut self.read) {
                Ok(Some(message)) => message,
                Ok(None) => return Ok(None),
                Err(err) => {
                    self.dead = true;
                    return Err(io_error("reply", err));
                }
            };
            match message {
                Message::NotificationResponse(body) => {
                    if let Ok(payload) = body.message() {
                        self.notifications.push_back(payload.to_string());
                    }
                }
                Message::NoticeResponse(body) => {
                    let notice = DbError::parse(body.fields());
                    tracing::info!("postgres {}: {}", notice.severity, notice.message);
                }
                Message::ParameterStatus(_) => {}
                other => return Ok(Some(other)),
            }
        }
    }

    /// Read one request's reply through its `ReadyForQuery`, giving `each` every other message. The server's error is
    /// returned, not raised; `each`'s first error is kept the same way, and the reply is still read to its end, so
    /// the next reply starts where it should.
    fn read_reply(&mut self, mut each: impl FnMut(Message) -> Result<(), Error>) -> Result<Option<Failure>, Error> {
        let mut failure: Option<Failure> = None;
        loop {
            match self.read_message()? {
                Message::ReadyForQuery(_) => return Ok(failure),
                Message::ErrorResponse(body) => {
                    failure.get_or_insert_with(|| Failure::Db(DbError::parse(body.fields())));
                }
                message => {
                    if failure.is_none() {
                        if let Err(err) = each(message) {
                            failure = Some(Failure::Local(err));
                        }
                    }
                }
            }
        }
    }

    /// Read the replies of the unanswered requests until no more than `keep` are left. Their first failure is kept.
    fn settle_to(&mut self, keep: usize) -> Result<(), Error> {
        if self.pending.len() <= keep {
            return Ok(());
        }
        self.flush()?;
        while self.pending.len() > keep {
            let Some(expect) = self.pending.pop_front() else {
                break;
            };
            let mut rows = 0;
            let failure = self.read_reply(|message| {
                if let Message::CommandComplete(body) = message {
                    rows = body.tag().map(tag_rows).unwrap_or(0);
                }
                Ok(())
            })?;
            let failure = match expect {
                Expect::Simple { context } => failure.map(|failure| failure.into_error(context)),
                Expect::Execute { zero_rows } => match failure {
                    Some(failure) => Some(failure.into_error("execute")),
                    None if rows == 0 => zero_rows.map(Error::new),
                    None => None,
                },
                Expect::Close => failure.map(|failure| failure.into_error("close")),
            };
            if let Some(err) = failure {
                self.failed.get_or_insert(err);
            }
        }
        Ok(())
    }

    /// Every reply in, and the first failure among them.
    pub(super) fn finish(&mut self) -> Result<(), Error> {
        self.settle_to(0)?;
        self.failed.take().map_or(Ok(()), Err)
    }

    /// Every reply in, or the connection lost, and their failures dropped.
    pub(super) fn abandon(&mut self) {
        let _ = self.settle_to(0);
        self.pending.clear();
        self.failed = None;
    }

    fn check_open(&self) -> Result<(), Error> {
        if self.dead {
            Err(Error::new("postgres: the connection is closed"))
        } else {
            Ok(())
        }
    }

    /// Write a request with `encode`; on a failure to encode it, nothing of it stays in the buffer.
    fn encode(&mut self, encode: impl FnOnce(&mut BytesMut) -> Result<(), Error>) -> Result<(), Error> {
        self.check_open()?;
        let mark = self.write.len();
        encode(&mut self.write).inspect_err(|_| self.write.truncate(mark))
    }

    /// Send a request (already encoded) and read the replies before it, then its own. The first failure of an earlier
    /// request is the error: in a transaction it is what made this one fail.
    fn round(&mut self, context: &str, each: impl FnMut(Message) -> Result<(), Error>) -> Result<(), Error> {
        self.flush()?;
        self.settle_to(0)?;
        let failure = self.read_reply(each)?;
        if let Some(err) = self.failed.take() {
            return Err(err);
        }
        failure.map_or(Ok(()), |failure| Err(failure.into_error(context)))
    }

    /// Keep the request just encoded unanswered. With `write` it goes out now, so the server runs it while this thread
    /// goes on; without, it goes out with the next request that is written.
    fn leave(&mut self, expect: Expect, write: bool) -> Result<(), Error> {
        self.pending.push_back(expect);
        if write {
            self.flush()?;
        }
        if self.pending.len() >= PIPELINE_DEPTH {
            self.settle_to(PIPELINE_DEPTH / 2)?;
        }
        Ok(())
    }

    /// One or more statements by the simple query protocol, waited for.
    pub(super) fn simple(&mut self, sql: &str, context: &str) -> Result<(), Error> {
        self.encode(|buf| frontend::query(sql, buf).map_err(|err| io_error(context, err)))?;
        self.round(context, |_| Ok(()))
    }

    /// [`Self::simple`] unanswered, written with the next request (a `BEGIN`, which the server has nothing to do for,
    /// goes out with the transaction's first statement).
    pub(super) fn queue_simple(&mut self, sql: &str, context: &'static str) -> Result<(), Error> {
        self.encode(|buf| frontend::query(sql, buf).map_err(|err| io_error(context, err)))?;
        self.leave(Expect::Simple { context }, false)
    }

    /// Prepare `sql` as statement `name` (`""` for the unnamed one) and describe it.
    pub(super) fn prepare(&mut self, name: &str, sql: &str) -> Result<Described, Error> {
        self.encode(|buf| {
            frontend::parse(name, sql, std::iter::empty(), buf)
                .and_then(|()| frontend::describe(b'S', name, buf))
                .map_err(|err| io_error("prepare", err))?;
            frontend::sync(buf);
            Ok(())
        })?;
        let mut params = Vec::new();
        let mut columns: Arc<[Column]> = Arc::new([]);
        self.round("prepare", |message| {
            match message {
                Message::ParameterDescription(body) => params = params_of(&body)?,
                Message::RowDescription(body) => columns = columns_of(&body)?,
                _ => {}
            }
            Ok(())
        })?;
        Ok(Described { params, columns })
    }

    /// Close prepared statement `name`, with the next request.
    pub(super) fn close_statement(&mut self, name: &str) -> Result<(), Error> {
        self.encode(|buf| {
            frontend::close(b'S', name, buf).map_err(|err| io_error("close", err))?;
            frontend::sync(buf);
            Ok(())
        })?;
        self.leave(Expect::Close, false)
    }

    /// One request: with `parse`, a `Parse` of that SQL as `statement` first; with [`Describe::Statement`], a
    /// `Describe` of the statement after it; then a `Bind` of `statement` to `count` parameters, each written by `encode`
    /// in the format `formats` gives, with every result column in binary; with [`Describe::Portal`], a `Describe` of
    /// the portal; then `Execute` and `Sync`. Nothing of it stays in the buffer when a parameter does not encode.
    fn bind_execute(
        &mut self,
        parse: Option<&str>,
        statement: &str,
        describe: Describe,
        formats: &[i16],
        count: usize,
        encode: Encode<'_>,
    ) -> Result<(), Error> {
        self.encode(|buf| {
            if let Some(sql) = parse {
                frontend::parse(statement, sql, std::iter::empty(), buf).map_err(|err| io_error("parse", err))?;
            }
            if matches!(describe, Describe::Statement) {
                frontend::describe(b'S', statement, buf).map_err(|err| io_error("describe", err))?;
            }
            let mut failure = None;
            let bound = frontend::bind(
                "",
                statement,
                formats.iter().copied(),
                0..count,
                |index, buf| match encode(index, buf) {
                    Ok(true) => Ok(postgres_protocol::IsNull::No),
                    Ok(false) => Ok(postgres_protocol::IsNull::Yes),
                    Err(err) => {
                        let message = err.message().to_string();
                        failure = Some(err);
                        Err(message.into())
                    }
                },
                Some(BINARY),
                buf,
            );
            if let Some(err) = failure {
                return Err(err);
            }
            bound.map_err(|err| match err {
                frontend::BindError::Conversion(err) => Error::new(format!("postgres bind: {err}")),
                frontend::BindError::Serialization(err) => io_error("bind", err),
            })?;
            if matches!(describe, Describe::Portal) {
                frontend::describe(b'P', "", buf).map_err(|err| io_error("describe", err))?;
            }
            frontend::execute("", 0, buf).map_err(|err| io_error("execute", err))?;
            frontend::sync(buf);
            Ok(())
        })
    }

    /// Run prepared `statement`, waited for; the rows it changed.
    pub(super) fn execute(
        &mut self,
        statement: &str,
        formats: &[i16],
        count: usize,
        encode: Encode<'_>,
    ) -> Result<u64, Error> {
        self.bind_execute(None, statement, Describe::None, formats, count, encode)?;
        let mut rows = 0;
        self.round("execute", |message| {
            if let Message::CommandComplete(body) = message {
                rows = body.tag().map(tag_rows).unwrap_or(0);
            }
            Ok(())
        })?;
        Ok(rows)
    }

    /// [`Self::execute`] unanswered, and written now. `zero_rows` is the error when it changes no row.
    pub(super) fn send_execute(
        &mut self,
        statement: &str,
        formats: &[i16],
        count: usize,
        encode: Encode<'_>,
        zero_rows: Option<String>,
    ) -> Result<(), Error> {
        self.bind_execute(None, statement, Describe::None, formats, count, encode)?;
        self.leave(Expect::Execute { zero_rows }, true)
    }

    /// Run prepared `statement` with its `columns`, waited for; its rows, each cell made by `decode`.
    pub(super) fn query(
        &mut self,
        statement: &str,
        columns: &Arc<[Column]>,
        formats: &[i16],
        count: usize,
        encode: Encode<'_>,
        decode: Decode<'_>,
    ) -> Result<Vec<DbRow>, Error> {
        self.bind_execute(None, statement, Describe::None, formats, count, encode)?;
        let mut rows = Vec::new();
        self.round("query", |message| {
            if let Message::DataRow(body) = message {
                rows.push(row_of(&body, columns, decode)?);
            }
            Ok(())
        })?;
        Ok(rows)
    }

    /// `sql` as the unnamed statement in one round trip: parsed, bound with every parameter in text (the server
    /// takes each as the type it infers), described and run. Its rows and the rows it changed.
    pub(super) fn query_unnamed(
        &mut self,
        sql: &str,
        count: usize,
        encode: Encode<'_>,
        decode: Decode<'_>,
        context: &str,
    ) -> Result<(Vec<DbRow>, u64), Error> {
        self.bind_execute(Some(sql), "", Describe::Portal, &[TEXT], count, encode)?;
        let mut columns: Arc<[Column]> = Arc::new([]);
        let mut rows = Vec::new();
        let mut changed = 0;
        self.round(context, |message| {
            match message {
                Message::RowDescription(body) => columns = columns_of(&body)?,
                Message::DataRow(body) => rows.push(row_of(&body, &columns, decode)?),
                Message::CommandComplete(body) => changed = body.tag().map(tag_rows).unwrap_or(0),
                _ => {}
            }
            Ok(())
        })?;
        Ok((rows, changed))
    }

    /// Prepare `sql` as statement `name`, describe it and run it, in one round trip where [`Self::prepare`] then a run
    /// take two: every parameter in text, as the server infers its type. The statement as described, its rows and the
    /// rows it changed. When the run fails after the statement parsed, the statement is closed, so a later use
    /// prepares it again.
    pub(super) fn prepare_run(
        &mut self,
        name: &str,
        sql: &str,
        count: usize,
        encode: Encode<'_>,
        decode: Decode<'_>,
        context: &str,
    ) -> Result<(Described, Vec<DbRow>, u64), Error> {
        self.bind_execute(Some(sql), name, Describe::Statement, &[TEXT], count, encode)?;
        let mut parsed = false;
        let mut params = Vec::new();
        let mut columns: Arc<[Column]> = Arc::new([]);
        let mut rows = Vec::new();
        let mut changed = 0;
        let ran = self.round(context, |message| {
            match message {
                Message::ParseComplete => parsed = true,
                Message::ParameterDescription(body) => params = params_of(&body)?,
                Message::RowDescription(body) => columns = columns_of(&body)?,
                Message::DataRow(body) => rows.push(row_of(&body, &columns, decode)?),
                Message::CommandComplete(body) => changed = body.tag().map(tag_rows).unwrap_or(0),
                _ => {}
            }
            Ok(())
        });
        if let Err(err) = ran {
            if parsed && !self.dead {
                self.close_statement(name)?;
            }
            return Err(err);
        }
        Ok((Described { params, columns }, rows, changed))
    }

    /// The payloads of the notifications that arrive within `timeout` of the call, and any already waiting behind the
    /// first. Empty when none came; an error when the connection is lost.
    pub(super) fn notifications(&mut self, timeout: Duration) -> Result<Vec<String>, Error> {
        self.check_open()?;
        self.parse_buffered()?;
        if self.notifications.is_empty() {
            self.stream
                .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
                .map_err(|err| io_error("notification", err))?;
            let waited = self.fill();
            let _ = self.stream.set_read_timeout(None);
            match waited {
                Ok(()) => {}
                Err(_) if !self.dead => {}
                Err(err) => return Err(err),
            }
            self.parse_buffered()?;
        }
        if !self.notifications.is_empty() {
            // Those right behind the first, already on the socket.
            self.stream
                .set_nonblocking(true)
                .map_err(|err| io_error("notification", err))?;
            let more = self.fill();
            let _ = self.stream.set_nonblocking(false);
            if let Err(err) = more {
                if self.dead {
                    return Err(err);
                }
            }
            self.parse_buffered()?;
        }
        Ok(self.notifications.drain(..).collect())
    }
}

impl Drop for Wire {
    /// `Terminate`, so the server ends the session at once rather than on a broken socket.
    fn drop(&mut self) {
        if self.dead {
            return;
        }
        self.write.clear();
        frontend::terminate(&mut self.write);
        let _ = self.stream.write_all(&self.write);
        let _ = self.stream.flush();
    }
}

fn params_of(body: &ParameterDescriptionBody) -> Result<Vec<Type>, Error> {
    let mut oids = body.parameters();
    let mut params = Vec::new();
    while let Some(oid) = oids.next().map_err(|err| io_error("describe", err))? {
        params.push(type_of(oid));
    }
    Ok(params)
}

fn columns_of(body: &RowDescriptionBody) -> Result<Arc<[Column]>, Error> {
    let mut fields = body.fields();
    let mut columns = Vec::new();
    while let Some(field) = fields.next().map_err(|err| io_error("describe", err))? {
        columns.push(Column {
            name: Arc::from(field.name()),
            ty: type_of(field.type_oid()),
        });
    }
    Ok(columns.into())
}

fn row_of(body: &DataRowBody, columns: &Arc<[Column]>, decode: Decode<'_>) -> Result<DbRow, Error> {
    let buffer = body.buffer();
    let mut ranges = body.ranges();
    let mut cells = Vec::with_capacity(columns.len());
    let mut index = 0;
    while let Some(range) = ranges.next().map_err(|err| io_error("row", err))? {
        let column = columns.get(index).ok_or_else(|| protocol_error("row column"))?;
        let value = decode(&column.ty, range.map(|range| &buffer[range]))
            .map_err(|err| Error::new(format!("postgres column {index}")).with_source(err))?;
        cells.push((Arc::clone(&column.name), value));
        index += 1;
    }
    Ok(DbRow { columns: cells })
}

/// A cancel of a running statement, on a connection of its own, as the protocol has it.
pub(super) fn cancel(endpoint: &Endpoint, process_id: i32, secret_key: i32) -> Result<(), Error> {
    let mut stream = Stream::open(endpoint)?;
    let mut request = BytesMut::new();
    frontend::cancel_request(process_id, secret_key, &mut request);
    stream
        .write_all(&request)
        .and_then(|()| stream.flush())
        .map_err(|err| io_error("cancel", err))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    use super::{tag_rows, Endpoint, Wire};

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::aws_lc_rs::default_provider())
    }

    /// One backend message: its tag, then its length (which counts itself) and body.
    fn message(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend_from_slice(&(u32::try_from(body.len() + 4).unwrap()).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    /// A frontend message after startup: its tag and body.
    fn read_message(stream: &mut impl Read) -> (u8, Vec<u8>) {
        let mut head = [0u8; 5];
        stream.read_exact(&mut head).unwrap();
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        let mut body = vec![0u8; len - 4];
        stream.read_exact(&mut body).unwrap();
        (head[0], body)
    }

    /// A server that takes the client through `SSLRequest` and a TLS handshake, trusts it, and answers one query.
    #[test]
    fn a_tls_connection_negotiates_starts_and_answers_a_query() {
        let issued = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert = issued.cert.der().clone();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(issued.signing_key.serialize_der().into());
        let server_config = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut tcp, _) = listener.accept().unwrap();
            let mut request = [0u8; 8];
            tcp.read_exact(&mut request).unwrap();
            assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47], "SSLRequest");
            tcp.write_all(b"S").unwrap();
            let connection = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, tcp);
            let mut len = [0u8; 4];
            tls.read_exact(&mut len).unwrap();
            let mut startup = vec![0u8; u32::from_be_bytes(len) as usize - 4];
            tls.read_exact(&mut startup).unwrap();
            assert!(
                String::from_utf8_lossy(&startup).contains("moqui"),
                "the database in the startup"
            );
            let mut reply = message(b'R', &0_i32.to_be_bytes());
            reply.extend(message(b'K', &[0, 0, 0, 7, 0, 0, 0, 9]));
            reply.extend(message(b'Z', b"I"));
            tls.write_all(&reply).unwrap();
            let (tag, body) = read_message(&mut tls);
            assert_eq!(tag, b'Q');
            assert!(body.starts_with(b"SELECT 1"));
            let mut reply = message(b'C', b"SELECT 1\0");
            reply.extend(message(b'Z', b"I"));
            tls.write_all(&reply).unwrap();
            let (tag, _) = read_message(&mut tls);
            assert_eq!(tag, b'X', "Terminate");
        });
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let client_config = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let endpoint = Endpoint::Tcp {
            host: "localhost".to_string(),
            port,
            tls: Some(Arc::new(client_config)),
        };
        let mut wire = Wire::connect(&endpoint, "moqui", "moqui", None, None).unwrap();
        assert_eq!(wire.cancel_key(), (7, 9));
        wire.simple("SELECT 1", "query").unwrap();
        drop(wire);
        server.join().unwrap();
    }

    #[test]
    fn a_server_that_refuses_tls_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut tcp, _): (TcpStream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8];
            tcp.read_exact(&mut request).unwrap();
            tcp.write_all(b"N").unwrap();
        });
        let config = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let endpoint = Endpoint::Tcp {
            host: "localhost".to_string(),
            port,
            tls: Some(Arc::new(config)),
        };
        let err = Wire::connect(&endpoint, "moqui", "moqui", None, None).err().unwrap();
        assert!(err.message().contains("does not accept TLS"), "{err}");
        server.join().unwrap();
    }

    #[test]
    fn a_command_tag_counts_its_rows() {
        assert_eq!(tag_rows("INSERT 0 5"), 5);
        assert_eq!(tag_rows("UPDATE 3"), 3);
        assert_eq!(tag_rows("SELECT 4"), 4);
        assert_eq!(tag_rows("BEGIN"), 0);
    }
}
