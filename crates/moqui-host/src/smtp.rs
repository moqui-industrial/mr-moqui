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

//! The inbound SMTP listener for hosted and shared mail accounts, behind the edge gateway (the `inbound-smtp` feature;
//! docs/Mail-Inbound.md). Off unless `smtp_listen_port` is set; it listens on `smtp_listen_host` (default `127.0.0.1`).
//!
//! It is the last hop of a gateway (Postfix with Rspamd, or another MTA) that does the internet side: TLS, SPF, DKIM,
//! DMARC, spam and virus filtering, queueing and retries. So it is a small RFC 5321 server for that one client: EHLO or
//! HELO, MAIL FROM (SIZE, BODY=8BITMIME, SMTPUTF8), RCPT TO, DATA, RSET, NOOP, QUIT, VRFY (252), HELP, with PIPELINING
//! and ENHANCEDSTATUSCODES; no AUTH, no STARTTLS, no relay. A connection from an address outside `smtp_allowed_addresses`
//! (IPs and CIDRs) gets 554 at connect and is closed; with `smtp_proxy_protocol` each connection starts with a PROXY v1
//! or v2 header and the address it names must be allowed too. An empty allow-list does not start the listener.
//!
//! `RCPT TO` is answered from the accounts ([`moqui_core::service::mail_inbound::find_recipient`]): an address of a
//! hosted or shared account is accepted, any other refused with 550 there, so nothing is accepted that would bounce
//! later. After `DATA` the message is stored once per account ([`moqui_core::service::mail_inbound::deliver`]); a store
//! that fails, or no request worker free, answers 451 so the gateway tries again, and the accounts that took it the
//! first time do not store it twice. Both run on a request worker with authorization disabled.
//!
//! Limits: `smtp_max_message_size` bytes (advertised as SIZE; a larger message is read to its end and refused with
//! 552), `smtp_max_recipients` a message (452 after), `smtp_max_connections` at once (421 after), `smtp_command_timeout`
//! seconds for each command line or data chunk, `smtp_session_timeout` seconds for a whole connection. Each session's
//! outcome is logged, with its addresses and sizes, never the message.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use moqui_core::context::{Error as CoreError, ExecutionContextFactory};
use moqui_core::service::mail_inbound::{self as inbound, Envelope, Options, Outcome, Recipients};
use moqui_lib::error::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// The longest command line, with its CRLF. RFC 5321 asks for 512; extensions make it longer.
const MAX_COMMAND: usize = 4096;
/// A data line is read in chunks of at most this many bytes.
const DATA_CHUNK: usize = 64 * 1024;
/// Commands answered with a syntax or sequence error before the connection is closed (an unknown recipient is not one).
const MAX_ERRORS: usize = 20;
/// The PROXY v2 signature.
const PROXY_V2: [u8; 12] = [0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A];

/// The address the listener is on, kept on the factory for `Server::smtp_address`.
#[derive(Default)]
pub struct SmtpAddress(pub std::sync::Mutex<Option<SocketAddr>>);

/// An allowed address or network: `10.0.0.5`, `10.0.0.0/8`, `fd00::/8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllowedNet {
    network: IpAddr,
    prefix: u8,
}

impl AllowedNet {
    /// # Errors
    ///
    /// Returns why when `text` is not an address or a network.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let network: IpAddr = address
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .map_err(|_| format!("{text} is not an IP address or network"))?;
        let network = network.to_canonical();
        let width = if network.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= width)
                .ok_or_else(|| format!("{text} has no valid prefix length"))?,
            None => width,
        };
        Ok(Self { network, prefix })
    }

    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.network, address.to_canonical()) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(self.prefix)).unwrap_or(0);
                u32::from(network) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = u128::MAX.checked_shl(128 - u32::from(self.prefix)).unwrap_or(0);
                u128::from(network) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

/// The listener's settings, from the `smtp_*` properties.
#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    /// The name in the greeting, EHLO and the Received field.
    pub hostname: String,
    pub allowed: Vec<AllowedNet>,
    pub max_message: usize,
    pub max_recipients: usize,
    pub max_connections: usize,
    pub command_timeout: Duration,
    pub session_timeout: Duration,
    pub proxy_protocol: bool,
    /// The plus-addressing characters; empty for none.
    pub plus_separator: String,
    pub spam_headers: Vec<String>,
    pub authserv_id: String,
}

fn property(name: &str) -> String {
    moqui_lib::env::get_prop_or_env(name).trim().to_string()
}

fn number(name: &str, default: u64) -> u64 {
    property(name)
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn count(name: &str, default: usize) -> usize {
    usize::try_from(number(name, default as u64)).unwrap_or(default)
}

impl SmtpConfig {
    /// The settings when `smtp_listen_port` is set, else none.
    ///
    /// # Errors
    ///
    /// Returns why the listener cannot start: a port that does not parse, an allow-list that is empty or does not
    /// parse.
    pub fn from_properties() -> Result<Option<Self>, Error> {
        let port_text = property("smtp_listen_port");
        if port_text.is_empty() {
            return Ok(None);
        }
        let port: u16 = port_text
            .parse()
            .map_err(|_| Error::new(format!("smtp_listen_port {port_text} is not a port number")))?;
        let allowed_text = property("smtp_allowed_addresses");
        let allowed = allowed_text
            .split([',', ' '])
            .filter(|entry| !entry.trim().is_empty())
            .map(AllowedNet::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|text| Error::new(format!("smtp_allowed_addresses: {text}")))?;
        if allowed.is_empty() {
            return Err(Error::new(
                "smtp_allowed_addresses is empty: name the edge gateway's addresses (127.0.0.1 for a gateway on this host)",
            ));
        }
        let hostname = match property("smtp_hostname") {
            name if !name.is_empty() => name,
            _ => match property("webapp_http_host") {
                name if !name.is_empty() => name,
                _ => "localhost".to_string(),
            },
        };
        let host = match property("smtp_listen_host") {
            host if !host.is_empty() => host,
            _ => "127.0.0.1".to_string(),
        };
        let list = |name: &str| -> Vec<String> {
            property(name)
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect()
        };
        Ok(Some(Self {
            host,
            port,
            hostname,
            allowed,
            max_message: count("smtp_max_message_size", 25 * 1024 * 1024),
            max_recipients: count("smtp_max_recipients", 100),
            max_connections: count("smtp_max_connections", 20),
            command_timeout: Duration::from_secs(number("smtp_command_timeout", 300)),
            session_timeout: Duration::from_secs(number("smtp_session_timeout", 1800)),
            proxy_protocol: matches!(
                property("smtp_proxy_protocol").to_ascii_lowercase().as_str(),
                "true" | "y" | "yes" | "1"
            ),
            plus_separator: match property("smtp_plus_separator") {
                none if none.eq_ignore_ascii_case("none") => String::new(),
                characters => characters
                    .chars()
                    .filter(|ch| !ch.is_whitespace() && *ch != '@')
                    .collect(),
            },
            spam_headers: list("smtp_spam_headers"),
            authserv_id: property("smtp_authserv_id"),
        }))
    }

    fn allows(&self, address: IpAddr) -> bool {
        self.allowed.iter().any(|net| net.contains(address))
    }
}

/// A running listener. Drop stops it; a session in progress is answered 421 at its next command.
pub struct SmtpServer {
    address: SocketAddr,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl SmtpServer {
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for SmtpServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Start the listener when `smtp_listen_port` is set (`0` binds an ephemeral port).
///
/// # Errors
///
/// Returns an error when the settings or the bind fail; the host logs it and starts without the listener.
pub fn start(factory: &Arc<ExecutionContextFactory>) -> Result<Option<SmtpServer>, Error> {
    let Some(config) = SmtpConfig::from_properties()? else {
        return Ok(None);
    };
    let bind = if config.host.contains(':') && !config.host.starts_with('[') {
        format!("[{}]:{}", config.host, config.port)
    } else {
        format!("{}:{}", config.host, config.port)
    };
    let bind_text = bind.clone();
    let listener = crate::edge::block_on(async move { TcpListener::bind(bind).await })?
        .map_err(|err| Error::new(format!("could not bind the SMTP listener to {bind_text}: {err}")))?;
    let address = listener
        .local_addr()
        .map_err(|err| Error::new("the SMTP listener has no address").with_source(err))?;
    if config.authserv_id.is_empty() {
        tracing::warn!(
            "smtp_authserv_id is empty: Authentication-Results fields are kept as they came, the gateway's not told \
             from a forged one"
        );
    }
    let backend = FactoryBackend {
        factory: Arc::downgrade(factory),
        delimiters: config.plus_separator.clone(),
        options: Options {
            spam_headers: config.spam_headers.clone(),
            authserv_id: config.authserv_id.clone(),
        },
    };
    let (shutdown, watch) = tokio::sync::watch::channel(false);
    tracing::info!(
        "SMTP listener on {address} for hosted and shared accounts, from {}{}",
        config
            .allowed
            .iter()
            .map(|net| format!("{}/{}", net.network, net.prefix))
            .collect::<Vec<_>>()
            .join(", "),
        if config.proxy_protocol { " through PROXY" } else { "" }
    );
    crate::edge::handle().spawn(serve(listener, config, backend, watch));
    *factory
        .extension(|_| SmtpAddress::default())
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(address);
    Ok(Some(SmtpServer { address, shutdown }))
}

/// The answer to `RCPT TO`.
pub(crate) enum RcptAnswer {
    /// The account it goes to.
    Local(String),
    Unknown,
    TempFail(String),
}

/// The answer after `DATA`.
pub(crate) enum DataAnswer {
    /// Stored; the text is for the session log.
    Stored(String),
    TempFail(String),
    Rejected(String),
}

/// A message after `DATA`.
pub(crate) struct Message {
    pub(crate) raw: Vec<u8>,
    pub(crate) envelope: Envelope,
    pub(crate) recipients: Vec<Recipients>,
}

/// Where the listener finds accounts and stores mail; the factory's, or a test's.
pub(crate) trait Backend: Send + Sync + 'static {
    fn recipient(&self, address: String) -> impl Future<Output = RcptAnswer> + Send;
    fn deliver(&self, message: Message) -> impl Future<Output = DataAnswer> + Send;
}

struct FactoryBackend {
    factory: Weak<ExecutionContextFactory>,
    delimiters: String,
    options: Options,
}

impl Backend for FactoryBackend {
    async fn recipient(&self, address: String) -> RcptAnswer {
        let delimiters = self.delimiters.clone();
        let found = crate::ssh::on_pool_without_authz(&self.factory, move |factory| {
            let facade = factory
                .entity()
                .ok_or_else(|| CoreError::new("the entity facade is not ready"))?;
            inbound::find_recipient(&facade, &address, &delimiters).map_err(CoreError::from)
        })
        .await;
        match found {
            Ok(Some(account)) => RcptAnswer::Local(account),
            Ok(None) => RcptAnswer::Unknown,
            Err(err) => RcptAnswer::TempFail(err.message().to_string()),
        }
    }

    async fn deliver(&self, message: Message) -> DataAnswer {
        let options = self.options.clone();
        let delivered = crate::ssh::on_pool_without_authz(&self.factory, move |factory| {
            let facade = factory
                .entity()
                .ok_or_else(|| CoreError::new("the entity facade is not ready"))?;
            Ok(inbound::deliver(
                &facade,
                &message.raw,
                &message.envelope,
                &message.recipients,
                &options,
            ))
        })
        .await;
        match delivered {
            Err(err) => DataAnswer::TempFail(err.message().to_string()),
            Ok(Err(text)) => DataAnswer::Rejected(text),
            Ok(Ok(report)) => {
                let accounts: Vec<String> = report
                    .deliveries
                    .iter()
                    .map(|delivery| match &delivery.result {
                        Ok(Outcome::Stored { email_message_id, junk }) => format!(
                            "account {} stored it as {email_message_id}{}",
                            delivery.email_account_id,
                            if *junk { " in Junk" } else { "" }
                        ),
                        Ok(Outcome::Duplicate) => format!("account {} had it", delivery.email_account_id),
                        Ok(Outcome::Gone) => format!("account {} is gone", delivery.email_account_id),
                        Err(_) => format!("account {} failed", delivery.email_account_id),
                    })
                    .collect();
                let summary = format!(
                    "{}; spam {}; {}",
                    accounts.join(", "),
                    if report.spam { "yes" } else { "no" },
                    if report.authentication.is_empty() {
                        "no Authentication-Results"
                    } else {
                        report.authentication.as_str()
                    }
                );
                if report.store_failed() {
                    DataAnswer::TempFail(summary)
                } else {
                    DataAnswer::Stored(summary)
                }
            }
        }
    }
}

struct Shared<B> {
    config: SmtpConfig,
    backend: B,
    active: AtomicUsize,
    sessions: AtomicU64,
}

/// A place among `smtp_max_connections`, given back on drop.
struct Slot<'a>(&'a AtomicUsize);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Accept connections until `shutdown` says stop.
pub(crate) async fn serve<B: Backend>(
    listener: TcpListener,
    config: SmtpConfig,
    backend: B,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let shared = Arc::new(Shared {
        config,
        backend,
        active: AtomicUsize::new(0),
        sessions: AtomicU64::new(0),
    });
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let _ = stream.set_nodelay(true);
                    let shared = Arc::clone(&shared);
                    let shutdown = shutdown.clone();
                    tokio::spawn(async move {
                        let (reader, writer) = stream.into_split();
                        connection(reader, writer, peer, &shared, shutdown).await;
                    });
                }
                Err(err) => {
                    tracing::warn!("SMTP accept failed: {err}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
    tracing::info!("SMTP listener stopped");
}

fn session_id(shared: &Shared<impl Backend>) -> String {
    let number = shared.sessions.fetch_add(1, Ordering::Relaxed);
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    format!("{seconds:X}{:04X}", number & 0xFFFF)
}

async fn connection<B: Backend, R, W>(
    reader: R,
    mut writer: W,
    peer: SocketAddr,
    shared: &Shared<B>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let id = session_id(shared);
    let peer_ip = peer.ip().to_canonical();
    let config = &shared.config;
    if !config.allows(peer_ip) {
        tracing::warn!("SMTP connection {id} from {peer_ip} refused: not in smtp_allowed_addresses");
        let _ = reply_now(
            &mut writer,
            config.command_timeout,
            &format!("554 5.7.1 {} does not take mail from this address", config.hostname),
        )
        .await;
        let _ = writer.shutdown().await;
        return;
    }
    if shared.active.fetch_add(1, Ordering::SeqCst) >= config.max_connections {
        shared.active.fetch_sub(1, Ordering::SeqCst);
        tracing::warn!("SMTP connection {id} from {peer_ip} refused: smtp_max_connections are open");
        let _ = reply_now(
            &mut writer,
            config.command_timeout,
            "421 4.7.0 Too many connections, try again later",
        )
        .await;
        let _ = writer.shutdown().await;
        return;
    }
    let _slot = Slot(&shared.active);
    let started = Instant::now();
    let mut session = Session {
        shared,
        id,
        client_ip: peer_ip,
        reader: BufReader::new(reader),
        writer,
        shutdown,
        deadline: started + config.session_timeout,
        helo: None,
        esmtp: false,
        transaction: None,
        errors: 0,
        messages: 0,
        accepted: 0,
        refused: 0,
    };
    let end = session.run().await;
    let _ = session.writer.shutdown().await;
    let level_error = matches!(end, End::Io(_));
    let text = format!(
        "SMTP session {} from {}{} ({}): {} messages, {} recipients accepted, {} refused, {} in {}ms",
        session.id,
        session.client_ip,
        if session.client_ip == peer_ip {
            String::new()
        } else {
            format!(" through {peer_ip}")
        },
        session.helo.as_deref().unwrap_or("no EHLO"),
        session.messages,
        session.accepted,
        session.refused,
        end.text(),
        started.elapsed().as_millis()
    );
    if level_error {
        tracing::warn!("{text}");
    } else {
        tracing::info!("{text}");
    }
}

/// Write one reply line now, within `wait`.
async fn reply_now<W: AsyncWrite + Unpin>(writer: &mut W, wait: Duration, line: &str) -> Result<(), End> {
    let mut bytes = Vec::with_capacity(line.len() + 2);
    bytes.extend_from_slice(line.as_bytes());
    bytes.extend_from_slice(b"\r\n");
    match tokio::time::timeout(wait, writer.write_all(&bytes)).await {
        Err(_) => Err(End::Timeout),
        Ok(Err(err)) => Err(End::Io(err.to_string())),
        Ok(Ok(())) => Ok(()),
    }
}

/// Why a session ended.
enum End {
    Quit,
    Closed,
    Timeout,
    Shutdown,
    TooLong,
    TooManyErrors,
    Proxy(String),
    Io(String),
}

impl End {
    fn text(&self) -> String {
        match self {
            Self::Quit => "quit".to_string(),
            Self::Closed => "closed by the client".to_string(),
            Self::Timeout => "timed out".to_string(),
            Self::Shutdown => "server stopping".to_string(),
            Self::TooLong => "line too long".to_string(),
            Self::TooManyErrors => "too many errors".to_string(),
            Self::Proxy(text) => format!("refused: {text}"),
            Self::Io(text) => format!("connection error: {text}"),
        }
    }
}

/// A mail transaction, from `MAIL FROM`.
struct Transaction {
    mail_from: String,
    utf8: bool,
    /// Each accepted `RCPT TO` and its account.
    recipients: Vec<(String, String)>,
}

struct Session<'a, B, R, W> {
    shared: &'a Shared<B>,
    id: String,
    client_ip: IpAddr,
    reader: BufReader<R>,
    writer: W,
    shutdown: tokio::sync::watch::Receiver<bool>,
    deadline: Instant,
    helo: Option<String>,
    esmtp: bool,
    transaction: Option<Transaction>,
    errors: usize,
    messages: usize,
    accepted: usize,
    refused: usize,
}

impl<B, R, W> Session<'_, B, R, W>
where
    B: Backend,
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    fn config(&self) -> &SmtpConfig {
        &self.shared.config
    }

    /// The time left for one read: the command timeout, within the session's.
    fn wait(&self) -> Result<Duration, End> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(End::Timeout);
        }
        Ok(left.min(self.config().command_timeout))
    }

    async fn reply(&mut self, line: &str) -> Result<(), End> {
        let wait = self.wait()?;
        reply_now(&mut self.writer, wait, line).await
    }

    /// A reply that counts as an error; too many of them close the connection.
    async fn error(&mut self, line: &str) -> Result<(), End> {
        self.errors += 1;
        if self.errors >= MAX_ERRORS {
            self.reply("421 4.7.0 Too many errors, closing the connection").await?;
            return Err(End::TooManyErrors);
        }
        self.reply(line).await
    }

    /// Read up to and with the next LF, at most `limit` bytes, into `buf`.
    async fn read_line(&mut self, limit: usize, buf: &mut Vec<u8>) -> Result<(), End> {
        buf.clear();
        let wait = self.wait()?;
        let mut limited = (&mut self.reader).take(limit as u64);
        let read = limited.read_until(b'\n', buf);
        tokio::select! {
            changed = self.shutdown.changed() => {
                let _ = changed;
                Err(End::Shutdown)
            }
            result = tokio::time::timeout(wait, read) => match result {
                Err(_) => Err(End::Timeout),
                Ok(Err(err)) => Err(End::Io(err.to_string())),
                Ok(Ok(0)) => Err(End::Closed),
                Ok(Ok(_)) => Ok(()),
            }
        }
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), End> {
        let wait = self.wait()?;
        match tokio::time::timeout(wait, self.reader.read_exact(buf)).await {
            Err(_) => Err(End::Timeout),
            Ok(Err(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => Err(End::Closed),
            Ok(Err(err)) => Err(End::Io(err.to_string())),
            Ok(Ok(_)) => Ok(()),
        }
    }

    async fn run(&mut self) -> End {
        match self.session().await {
            Ok(()) => End::Quit,
            Err(End::Timeout) => {
                let _ = reply_now(
                    &mut self.writer,
                    Duration::from_secs(5),
                    "421 4.4.2 Timeout, closing the connection",
                )
                .await;
                End::Timeout
            }
            Err(End::Shutdown) => {
                let _ = reply_now(
                    &mut self.writer,
                    Duration::from_secs(5),
                    "421 4.3.2 Server stopping, try again later",
                )
                .await;
                End::Shutdown
            }
            Err(end) => end,
        }
    }

    async fn session(&mut self) -> Result<(), End> {
        if self.config().proxy_protocol {
            let source = self.read_proxy().await?;
            if let Some(source) = source {
                let source = source.to_canonical();
                if !self.config().allows(source) {
                    let _ = self
                        .reply(&format!(
                            "554 5.7.1 {} does not take mail from this address",
                            self.config().hostname
                        ))
                        .await;
                    return Err(End::Proxy(format!("{source} is not in smtp_allowed_addresses")));
                }
                self.client_ip = source;
            }
        }
        let greeting = format!("220 {} ESMTP mr-moqui", self.config().hostname);
        self.reply(&greeting).await?;
        let mut buf = Vec::with_capacity(512);
        loop {
            self.read_line(MAX_COMMAND, &mut buf).await?;
            if !buf.ends_with(b"\n") {
                self.reply("500 5.5.2 Line too long").await?;
                return Err(End::TooLong);
            }
            let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
            let (verb, rest) = match line.split_once(' ') {
                Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.trim().to_string()),
                None => (line.trim().to_ascii_uppercase(), String::new()),
            };
            match verb.as_str() {
                "EHLO" | "HELO" => self.hello(&verb, &rest).await?,
                "MAIL" => self.mail(&rest).await?,
                "RCPT" => self.rcpt(&rest).await?,
                "DATA" => self.data(&rest).await?,
                "RSET" => {
                    self.transaction = None;
                    self.reply("250 2.0.0 Ok").await?;
                }
                "NOOP" => self.reply("250 2.0.0 Ok").await?,
                "QUIT" => {
                    self.reply("221 2.0.0 Bye").await?;
                    return Ok(());
                }
                "VRFY" => self.reply("252 2.5.0 Cannot VRFY user").await?,
                "HELP" => self.reply("214 2.0.0 See RFC 5321").await?,
                "STARTTLS" | "AUTH" | "BDAT" | "EXPN" | "ETRN" | "TURN" | "ATRN" => {
                    self.error("502 5.5.1 Command not implemented").await?;
                }
                _ => self.error("500 5.5.2 Command not recognized").await?,
            }
        }
    }

    /// The PROXY v1 or v2 header: the client's address, or none for a proxy's own connection (LOCAL, UNKNOWN).
    async fn read_proxy(&mut self) -> Result<Option<IpAddr>, End> {
        let mut start = [0_u8; 6];
        self.read_exact(&mut start).await?;
        if &start == b"PROXY " {
            let mut line = Vec::with_capacity(108);
            self.read_line(102, &mut line).await?;
            if !line.ends_with(b"\r\n") {
                return Err(End::Proxy("a PROXY v1 header too long or not ended".to_string()));
            }
            let text = String::from_utf8_lossy(&line);
            let parts: Vec<&str> = text.split_whitespace().collect();
            return match parts.as_slice() {
                ["UNKNOWN", ..] => Ok(None),
                ["TCP4" | "TCP6", source, _, _, _] => source
                    .parse::<IpAddr>()
                    .map(Some)
                    .map_err(|_| End::Proxy(format!("PROXY v1 source {source} is not an address"))),
                _ => Err(End::Proxy("a PROXY v1 header that does not parse".to_string())),
            };
        }
        if start != PROXY_V2[..6] {
            return Err(End::Proxy("no PROXY header".to_string()));
        }
        let mut rest = [0_u8; 10];
        self.read_exact(&mut rest).await?;
        if rest[..6] != PROXY_V2[6..] || rest[6] >> 4 != 2 {
            return Err(End::Proxy("a PROXY v2 header that does not parse".to_string()));
        }
        let command = rest[6] & 0x0F;
        let family = rest[7] >> 4;
        let length = usize::from(u16::from_be_bytes([rest[8], rest[9]]));
        if length > 4096 {
            return Err(End::Proxy("a PROXY v2 header too long".to_string()));
        }
        let mut addresses = vec![0_u8; length];
        self.read_exact(&mut addresses).await?;
        if command == 0 {
            return Ok(None);
        }
        match family {
            1 if length >= 12 => Ok(Some(IpAddr::from([
                addresses[0],
                addresses[1],
                addresses[2],
                addresses[3],
            ]))),
            2 if length >= 36 => {
                let mut octets = [0_u8; 16];
                octets.copy_from_slice(&addresses[..16]);
                Ok(Some(IpAddr::from(octets)))
            }
            1 | 2 => Err(End::Proxy("a PROXY v2 address block too short".to_string())),
            _ => Ok(None),
        }
    }

    async fn hello(&mut self, verb: &str, rest: &str) -> Result<(), End> {
        let name = rest.split_whitespace().next().unwrap_or_default();
        if name.is_empty() {
            return self.error(&format!("501 5.5.4 Syntax: {verb} hostname")).await;
        }
        self.helo = Some(name.chars().take(255).collect());
        self.esmtp = verb == "EHLO";
        self.transaction = None;
        let hostname = self.config().hostname.clone();
        if !self.esmtp {
            return self.reply(&format!("250 {hostname}")).await;
        }
        let size = self.config().max_message;
        let lines = [
            format!("250-{hostname}"),
            "250-PIPELINING".to_string(),
            format!("250-SIZE {size}"),
            "250-8BITMIME".to_string(),
            "250-SMTPUTF8".to_string(),
            "250 ENHANCEDSTATUSCODES".to_string(),
        ];
        self.reply(&lines.join("\r\n")).await
    }

    async fn mail(&mut self, rest: &str) -> Result<(), End> {
        if self.helo.is_none() {
            return self.error("503 5.5.1 Send EHLO first").await;
        }
        if self.transaction.is_some() {
            return self.error("503 5.5.1 Nested MAIL command").await;
        }
        let Some(path) = strip_keyword(rest, "FROM:") else {
            return self.error("501 5.5.4 Syntax: MAIL FROM:<address>").await;
        };
        let Some((address, parameters)) = parse_path(path) else {
            return self.error("501 5.1.7 Bad sender address syntax").await;
        };
        let mut utf8 = false;
        for parameter in parameters.split_whitespace() {
            let (key, value) = parameter.split_once('=').unwrap_or((parameter, ""));
            match key.to_ascii_uppercase().as_str() {
                "SIZE" if self.esmtp => {
                    let Ok(size) = value.parse::<u64>() else {
                        return self.error("501 5.5.4 Bad SIZE parameter").await;
                    };
                    if size > self.config().max_message as u64 {
                        return self
                            .error("552 5.3.4 Message size exceeds fixed maximum message size")
                            .await;
                    }
                }
                "BODY" if self.esmtp && matches!(value.to_ascii_uppercase().as_str(), "7BIT" | "8BITMIME") => {}
                "SMTPUTF8" if self.esmtp && value.is_empty() => utf8 = true,
                _ => {
                    return self
                        .error(&format!("555 5.5.4 Unsupported option: {}", clip(parameter)))
                        .await
                }
            }
        }
        self.transaction = Some(Transaction {
            mail_from: address,
            utf8,
            recipients: Vec::new(),
        });
        self.reply("250 2.1.0 Ok").await
    }

    async fn rcpt(&mut self, rest: &str) -> Result<(), End> {
        let Some(count) = self
            .transaction
            .as_ref()
            .map(|transaction| transaction.recipients.len())
        else {
            return self.error("503 5.5.1 Need MAIL command").await;
        };
        let Some(path) = strip_keyword(rest, "TO:") else {
            return self.error("501 5.5.4 Syntax: RCPT TO:<address>").await;
        };
        let Some((address, parameters)) = parse_path(path) else {
            return self.error("501 5.1.3 Bad recipient address syntax").await;
        };
        if !parameters.trim().is_empty() {
            return self
                .error(&format!("555 5.5.4 Unsupported option: {}", clip(parameters.trim())))
                .await;
        }
        let well_formed = address
            .rsplit_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty());
        if !well_formed && !address.eq_ignore_ascii_case("postmaster") {
            return self.error("501 5.1.3 Bad recipient address syntax").await;
        }
        if count >= self.config().max_recipients {
            self.refused += 1;
            return self.reply("452 4.5.3 Too many recipients").await;
        }
        let answer = if well_formed {
            self.shared.backend.recipient(address.clone()).await
        } else {
            RcptAnswer::Unknown
        };
        match answer {
            RcptAnswer::Local(account) => {
                if let Some(transaction) = self.transaction.as_mut() {
                    transaction.recipients.push((address, account));
                }
                self.accepted += 1;
                self.reply("250 2.1.5 Ok").await
            }
            RcptAnswer::Unknown => {
                self.refused += 1;
                tracing::info!(
                    "SMTP session {} refused recipient {}: no hosted or shared account has it",
                    self.id,
                    clip(&address)
                );
                self.reply(&format!(
                    "550 5.1.1 <{}>: Recipient address rejected: User unknown",
                    clip(&address)
                ))
                .await
            }
            RcptAnswer::TempFail(text) => {
                self.refused += 1;
                tracing::error!(
                    "SMTP session {} could not look up recipient {}: {text}",
                    self.id,
                    clip(&address)
                );
                self.reply(&format!(
                    "451 4.3.0 <{}>: Temporary lookup failure, try again later",
                    clip(&address)
                ))
                .await
            }
        }
    }

    async fn data(&mut self, rest: &str) -> Result<(), End> {
        if !rest.is_empty() {
            return self.error("501 5.5.4 Syntax: DATA").await;
        }
        let Some(recipients) = self
            .transaction
            .as_ref()
            .map(|transaction| transaction.recipients.len())
        else {
            return self.error("503 5.5.1 Need MAIL command").await;
        };
        if recipients == 0 {
            return self.error("554 5.5.1 No valid recipients").await;
        }
        self.reply("354 End data with <CR><LF>.<CR><LF>").await?;
        let raw = self.read_data().await?;
        let Some(transaction) = self.transaction.take() else {
            return self.reply("503 5.5.1 Need MAIL command").await;
        };
        let Some(raw) = raw else {
            tracing::warn!(
                "SMTP session {} refused a message from <{}>: larger than smtp_max_message_size ({} bytes)",
                self.id,
                clip(&transaction.mail_from),
                self.config().max_message
            );
            return self
                .reply("552 5.3.4 Message size exceeds fixed maximum message size")
                .await;
        };
        let size = raw.len();
        let mut grouped: Vec<Recipients> = Vec::new();
        for (address, account) in &transaction.recipients {
            match grouped.iter_mut().find(|group| group.email_account_id == *account) {
                Some(group) => {
                    if !group.addresses.contains(address) {
                        group.addresses.push(address.clone());
                    }
                }
                None => grouped.push(Recipients {
                    email_account_id: account.clone(),
                    addresses: vec![address.clone()],
                }),
            }
        }
        let protocol = match (self.esmtp, transaction.utf8) {
            (true, true) => "UTF8ESMTP",
            (true, false) => "ESMTP",
            _ => "SMTP",
        };
        let message = Message {
            raw,
            envelope: Envelope {
                mail_from: transaction.mail_from.clone(),
                helo: self.helo.clone().unwrap_or_default(),
                client_ip: self.client_ip.to_string(),
                protocol: protocol.to_string(),
                session_id: self.id.clone(),
                hostname: self.config().hostname.clone(),
            },
            recipients: grouped,
        };
        let answer = self.shared.backend.deliver(message).await;
        let recipients: Vec<&str> = transaction
            .recipients
            .iter()
            .map(|(address, _)| address.as_str())
            .collect();
        match answer {
            DataAnswer::Stored(summary) => {
                self.messages += 1;
                tracing::info!(
                    "SMTP session {} stored a message of {size} bytes from <{}> to {}: {summary}",
                    self.id,
                    clip(&transaction.mail_from),
                    recipients.join(", ")
                );
                self.reply(&format!("250 2.0.0 Ok: stored as {}.{}", self.id, self.messages))
                    .await
            }
            DataAnswer::TempFail(text) => {
                tracing::error!(
                    "SMTP session {} could not store a message of {size} bytes from <{}> to {}, answered 451: {text}",
                    self.id,
                    clip(&transaction.mail_from),
                    recipients.join(", ")
                );
                self.reply("451 4.3.0 Message not stored, try again later").await
            }
            DataAnswer::Rejected(text) => {
                tracing::warn!(
                    "SMTP session {} refused a message of {size} bytes from <{}>: {text}",
                    self.id,
                    clip(&transaction.mail_from)
                );
                self.reply(&format!("554 5.6.0 {}", clip(&text))).await
            }
        }
    }

    /// The message up to `CRLF.CRLF`, dot-unstuffed; none when it is larger than the limit (read to its end anyway).
    /// Only a `.` line after a CRLF ends it, so a bare LF cannot end the data early.
    async fn read_data(&mut self) -> Result<Option<Vec<u8>>, End> {
        let max = self.config().max_message;
        let mut out: Vec<u8> = Vec::new();
        let mut too_big = false;
        let mut line = Vec::with_capacity(1024);
        let mut line_start = true;
        loop {
            self.read_line(DATA_CHUNK, &mut line).await?;
            if line_start && line == b".\r\n" {
                break;
            }
            let content = if line_start && line.first() == Some(&b'.') {
                &line[1..]
            } else {
                &line[..]
            };
            if !too_big {
                if out.len() + content.len() > max {
                    too_big = true;
                    out = Vec::new();
                } else {
                    out.extend_from_slice(content);
                }
            }
            line_start = line.ends_with(b"\r\n");
        }
        Ok((!too_big).then_some(out))
    }
}

/// `rest` after a keyword such as `FROM:`, in any case, with the spaces some clients put after the colon.
fn strip_keyword<'a>(rest: &'a str, keyword: &str) -> Option<&'a str> {
    let head = rest.get(..keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    Some(rest[keyword.len()..].trim_start())
}

/// A `<path>` and the parameters after it. The source route (`@a,@b:`) is dropped; a quoted local part may hold `>`.
/// `<>` is the null path. Control characters are not allowed.
fn parse_path(text: &str) -> Option<(String, &str)> {
    let inner = text.strip_prefix('<')?;
    let mut quoted = false;
    let mut escaped = false;
    let mut end = None;
    for (index, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '>' if !quoted => {
                end = Some(index);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    let mut address = &inner[..end];
    if address.starts_with('@') {
        address = address.split_once(':').map(|(_, rest)| rest)?;
    }
    if address
        .chars()
        .any(|ch| ch.is_control() || (ch == ' ' && !address.contains('"')))
    {
        return None;
    }
    let rest = &inner[end + 1..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((address.to_string(), rest))
}

/// Text from the client for a reply or the log: at most 200 characters.
fn clip(text: &str) -> String {
    text.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    /// Accounts by address, and every message delivered; a recipient `fail@` makes the store fail.
    #[derive(Default)]
    struct Fake {
        delivered: Mutex<Vec<Message>>,
    }

    impl Backend for Arc<Fake> {
        async fn recipient(&self, address: String) -> RcptAnswer {
            match address.to_ascii_lowercase().as_str() {
                "sales@example.com" | "fail@example.com" => RcptAnswer::Local(address.to_ascii_lowercase()),
                "busy@example.com" => RcptAnswer::TempFail("database down".to_string()),
                _ => RcptAnswer::Unknown,
            }
        }

        async fn deliver(&self, message: Message) -> DataAnswer {
            let fail = message
                .recipients
                .iter()
                .any(|group| group.email_account_id == "fail@example.com");
            let bad = !message.raw.windows(2).any(|pair| pair == b": ");
            self.delivered.lock().unwrap().push(message);
            if bad {
                DataAnswer::Rejected("the message has no header fields".to_string())
            } else if fail {
                DataAnswer::TempFail("store failed".to_string())
            } else {
                DataAnswer::Stored("ok".to_string())
            }
        }
    }

    fn config(allowed: &str) -> SmtpConfig {
        SmtpConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            hostname: "mx.test".to_string(),
            allowed: allowed.split(',').map(|net| AllowedNet::parse(net).unwrap()).collect(),
            max_message: 1000,
            max_recipients: 2,
            max_connections: 2,
            command_timeout: Duration::from_secs(5),
            session_timeout: Duration::from_secs(30),
            proxy_protocol: false,
            plus_separator: "+".to_string(),
            spam_headers: Vec::new(),
            authserv_id: String::new(),
        }
    }

    async fn start(config: SmtpConfig) -> (SocketAddr, Arc<Fake>, tokio::sync::watch::Sender<bool>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let fake = Arc::new(Fake::default());
        let (stop, watch) = tokio::sync::watch::channel(false);
        tokio::spawn(serve(listener, config, Arc::clone(&fake), watch));
        (address, fake, stop)
    }

    struct Client {
        reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
        writer: tokio::net::tcp::OwnedWriteHalf,
    }

    impl Client {
        async fn connect(address: SocketAddr) -> Self {
            Self::from(TcpStream::connect(address).await.unwrap())
        }

        fn from(stream: TcpStream) -> Self {
            let (reader, writer) = stream.into_split();
            Self {
                reader: BufReader::new(reader),
                writer,
            }
        }

        /// One reply, its lines joined with `|`; empty at the end of the stream.
        async fn reply(&mut self) -> String {
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                if self.reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let line = line.trim_end().to_string();
                let last = line.as_bytes().get(3) != Some(&b'-');
                lines.push(line);
                if last {
                    break;
                }
            }
            lines.join("|")
        }

        async fn send(&mut self, text: &str) {
            self.writer.write_all(text.as_bytes()).await.unwrap();
        }

        async fn command(&mut self, line: &str) -> String {
            self.send(&format!("{line}\r\n")).await;
            self.reply().await
        }
    }

    #[tokio::test]
    async fn a_session_takes_mail_for_known_recipients_only() {
        let (address, fake, _stop) = start(config("127.0.0.1")).await;
        let mut client = Client::connect(address).await;
        assert_eq!(client.reply().await, "220 mx.test ESMTP mr-moqui");
        assert_eq!(client.command("MAIL FROM:<a@x.org>").await, "503 5.5.1 Send EHLO first");
        let ehlo = client.command("EHLO gw.example.com").await;
        assert_eq!(
            ehlo,
            "250-mx.test|250-PIPELINING|250-SIZE 1000|250-8BITMIME|250-SMTPUTF8|250 ENHANCEDSTATUSCODES"
        );
        assert_eq!(
            client.command("RCPT TO:<sales@example.com>").await,
            "503 5.5.1 Need MAIL command"
        );
        assert_eq!(
            client.command("MAIL FROM:<a@x.org> SIZE=2000").await,
            "552 5.3.4 Message size exceeds fixed maximum message size"
        );
        assert_eq!(
            client.command("MAIL FROM:<a@x.org> BODY=8BITMIME SIZE=100").await,
            "250 2.1.0 Ok"
        );
        assert_eq!(
            client.command("RCPT TO:<nobody@example.com>").await,
            "550 5.1.1 <nobody@example.com>: Recipient address rejected: User unknown"
        );
        assert_eq!(
            client.command("RCPT TO:<busy@example.com>").await,
            "451 4.3.0 <busy@example.com>: Temporary lookup failure, try again later"
        );
        assert_eq!(client.command("RCPT TO:<Sales@example.com>").await, "250 2.1.5 Ok");
        assert_eq!(
            client.command("RCPT TO:<@relay.example:sales@example.com>").await,
            "250 2.1.5 Ok"
        );
        assert_eq!(
            client.command("RCPT TO:<sales@example.com>").await,
            "452 4.5.3 Too many recipients"
        );
        assert_eq!(
            client.command("RCPT TO:<sales@example.com> NOTIFY=NEVER").await,
            "555 5.5.4 Unsupported option: NOTIFY=NEVER"
        );
        assert_eq!(client.command("DATA").await, "354 End data with <CR><LF>.<CR><LF>");
        client
            .send("Subject: Hi\r\n\r\n..leading dot\r\nbare\nline\n.\nstill data\r\n.\r\n")
            .await;
        assert!(client.reply().await.starts_with("250 2.0.0 Ok: stored as "));
        {
            let delivered = fake.delivered.lock().unwrap();
            assert_eq!(delivered.len(), 1);
            let message = &delivered[0];
            assert_eq!(
                String::from_utf8_lossy(&message.raw),
                "Subject: Hi\r\n\r\n.leading dot\r\nbare\nline\n.\nstill data\r\n"
            );
            assert_eq!(message.envelope.mail_from, "a@x.org");
            assert_eq!(message.envelope.helo, "gw.example.com");
            assert_eq!(message.envelope.client_ip, "127.0.0.1");
            assert_eq!(message.envelope.protocol, "ESMTP");
            assert_eq!(message.recipients.len(), 1, "one account");
            assert_eq!(message.recipients[0].email_account_id, "sales@example.com");
            assert_eq!(
                message.recipients[0].addresses,
                ["Sales@example.com", "sales@example.com"]
            );
        }
        // pipelined, a store that fails, then too large, then no header fields
        client
            .send("MAIL FROM:<>\r\nRCPT TO:<fail@example.com>\r\nDATA\r\n")
            .await;
        assert_eq!(client.reply().await, "250 2.1.0 Ok");
        assert_eq!(client.reply().await, "250 2.1.5 Ok");
        assert_eq!(client.reply().await, "354 End data with <CR><LF>.<CR><LF>");
        client.send("Subject: x\r\n\r\nbody\r\n.\r\n").await;
        assert_eq!(client.reply().await, "451 4.3.0 Message not stored, try again later");
        assert_eq!(client.command("MAIL FROM:<a@x.org>").await, "250 2.1.0 Ok");
        assert_eq!(client.command("RCPT TO:<sales@example.com>").await, "250 2.1.5 Ok");
        assert_eq!(client.command("DATA").await, "354 End data with <CR><LF>.<CR><LF>");
        client
            .send(&format!("Subject: big\r\n\r\n{}\r\n.\r\n", "x".repeat(2000)))
            .await;
        assert_eq!(
            client.reply().await,
            "552 5.3.4 Message size exceeds fixed maximum message size"
        );
        assert_eq!(client.command("DATA").await, "503 5.5.1 Need MAIL command");
        assert_eq!(client.command("MAIL FROM:<a@x.org>").await, "250 2.1.0 Ok");
        assert_eq!(client.command("RSET").await, "250 2.0.0 Ok");
        assert_eq!(client.command("DATA").await, "503 5.5.1 Need MAIL command");
        assert_eq!(client.command("MAIL FROM:<a@x.org>").await, "250 2.1.0 Ok");
        assert_eq!(client.command("DATA").await, "554 5.5.1 No valid recipients");
        assert_eq!(client.command("RCPT TO:<sales@example.com>").await, "250 2.1.5 Ok");
        assert_eq!(client.command("DATA").await, "354 End data with <CR><LF>.<CR><LF>");
        client.send("no header\r\n.\r\n").await;
        assert_eq!(client.reply().await, "554 5.6.0 the message has no header fields");
        assert_eq!(
            client.command("AUTH PLAIN abc").await,
            "502 5.5.1 Command not implemented"
        );
        assert_eq!(client.command("STARTTLS").await, "502 5.5.1 Command not implemented");
        assert_eq!(client.command("VRFY sales").await, "252 2.5.0 Cannot VRFY user");
        assert_eq!(client.command("NOOP").await, "250 2.0.0 Ok");
        assert_eq!(client.command("QUIT").await, "221 2.0.0 Bye");
        assert_eq!(client.reply().await, "");
        assert_eq!(
            fake.delivered.lock().unwrap().len(),
            3,
            "the 552 message never reached the store"
        );
    }

    #[tokio::test]
    async fn addresses_outside_the_allow_list_are_refused_at_connect() {
        let (address, fake, _stop) = start(config("10.0.0.0/8, ::1")).await;
        let mut client = Client::connect(address).await;
        assert_eq!(
            client.reply().await,
            "554 5.7.1 mx.test does not take mail from this address"
        );
        assert_eq!(client.reply().await, "");
        assert!(fake.delivered.lock().unwrap().is_empty());
        // 127.0.0.2 is not 127.0.0.1; macOS routes only 127.0.0.1 unless lo0 has an alias
        let (address, _fake, _stop) = start(config("127.0.0.1/32")).await;
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        match socket.bind("127.0.0.2:0".parse().unwrap()) {
            Err(err) if err.kind() == std::io::ErrorKind::AddrNotAvailable => {
                eprintln!("skipping the 127.0.0.2 connect: this host does not route 127.0.0.2 ({err})");
            }
            bound => {
                bound.unwrap();
                let mut other = Client::from(socket.connect(address).await.unwrap());
                assert_eq!(
                    other.reply().await,
                    "554 5.7.1 mx.test does not take mail from this address"
                );
            }
        }
        let mut allowed = Client::connect(address).await;
        assert_eq!(allowed.reply().await, "220 mx.test ESMTP mr-moqui");
    }

    #[tokio::test]
    async fn connections_over_the_limit_wait_and_idle_ones_time_out() {
        let mut short = config("127.0.0.0/8");
        short.command_timeout = Duration::from_millis(300);
        let (address, _fake, stop) = start(short).await;
        let mut one = Client::connect(address).await;
        let mut two = Client::connect(address).await;
        assert_eq!(one.reply().await, "220 mx.test ESMTP mr-moqui");
        assert_eq!(two.reply().await, "220 mx.test ESMTP mr-moqui");
        let mut three = Client::connect(address).await;
        assert_eq!(three.reply().await, "421 4.7.0 Too many connections, try again later");
        assert_eq!(one.reply().await, "421 4.4.2 Timeout, closing the connection");
        assert_eq!(one.reply().await, "");
        drop(two);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut four = Client::connect(address).await;
        assert_eq!(four.reply().await, "220 mx.test ESMTP mr-moqui");
        // stopping answers the session in progress
        stop.send(true).unwrap();
        assert_eq!(four.command("NOOP").await, "421 4.3.2 Server stopping, try again later");
    }

    #[tokio::test]
    async fn a_proxy_header_names_the_client_which_must_be_allowed_too() {
        let mut proxied = config("127.0.0.1, 192.0.2.10");
        proxied.proxy_protocol = true;
        proxied.max_connections = 10;
        let (address, fake, _stop) = start(proxied).await;
        let mut v1 = Client::connect(address).await;
        v1.send("PROXY TCP4 192.0.2.10 127.0.0.1 40000 25\r\n").await;
        assert_eq!(v1.reply().await, "220 mx.test ESMTP mr-moqui");
        v1.command("EHLO gw").await;
        v1.command("MAIL FROM:<a@x.org>").await;
        v1.command("RCPT TO:<sales@example.com>").await;
        v1.command("DATA").await;
        v1.send("Subject: proxied\r\n\r\nhi\r\n.\r\n").await;
        assert!(v1.reply().await.starts_with("250 2.0.0"));
        assert_eq!(fake.delivered.lock().unwrap()[0].envelope.client_ip, "192.0.2.10");
        let mut stranger = Client::connect(address).await;
        stranger.send("PROXY TCP4 198.51.100.7 127.0.0.1 40000 25\r\n").await;
        assert_eq!(
            stranger.reply().await,
            "554 5.7.1 mx.test does not take mail from this address"
        );
        let mut v2 = Client::connect(address).await;
        let mut header = PROXY_V2.to_vec();
        header.extend_from_slice(&[0x21, 0x11, 0, 12, 192, 0, 2, 10, 127, 0, 0, 1, 0x9C, 0x40, 0, 25]);
        v2.writer.write_all(&header).await.unwrap();
        assert_eq!(v2.reply().await, "220 mx.test ESMTP mr-moqui");
        let mut local = Client::connect(address).await;
        let mut header = PROXY_V2.to_vec();
        header.extend_from_slice(&[0x20, 0x00, 0, 0]);
        local.writer.write_all(&header).await.unwrap();
        assert_eq!(local.reply().await, "220 mx.test ESMTP mr-moqui");
        let mut none = Client::connect(address).await;
        none.send("EHLO gw\r\n").await;
        assert_eq!(none.reply().await, "");
    }

    #[test]
    fn allow_list_entries_and_paths_parse() {
        let net = AllowedNet::parse("10.1.0.0/16").unwrap();
        assert!(net.contains("10.1.200.3".parse().unwrap()));
        assert!(net.contains("::ffff:10.1.0.9".parse().unwrap()));
        assert!(!net.contains("10.2.0.1".parse().unwrap()));
        assert!(AllowedNet::parse("0.0.0.0/0")
            .unwrap()
            .contains("8.8.8.8".parse().unwrap()));
        assert!(AllowedNet::parse("fd00::/8")
            .unwrap()
            .contains("fd12::1".parse().unwrap()));
        assert!(AllowedNet::parse("[::1]").unwrap().contains("::1".parse().unwrap()));
        assert!(AllowedNet::parse("10.0.0.0/33").is_err());
        assert!(AllowedNet::parse("gateway.example.com").is_err());
        assert_eq!(parse_path("<a@x.org> SIZE=1"), Some(("a@x.org".to_string(), " SIZE=1")));
        assert_eq!(parse_path("<>"), Some((String::new(), "")));
        assert_eq!(
            parse_path("<\"odd>name\"@x.org>"),
            Some(("\"odd>name\"@x.org".to_string(), ""))
        );
        assert_eq!(parse_path("a@x.org"), None);
        assert_eq!(parse_path("<a@x.org>junk"), None);
        assert_eq!(strip_keyword("from: <a@x.org>", "FROM:"), Some("<a@x.org>"));
    }
}
