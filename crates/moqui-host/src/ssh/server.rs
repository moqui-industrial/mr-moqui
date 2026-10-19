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

//! The built-in SSH server (`plans/archive/Ssh.md`): off unless `ssh_port` is set. It listens on `ssh_host` (default
//! `127.0.0.1`) with the host key `ssh_host_key`, a secret property that is the base64 seed of an Ed25519 key
//! (generated into `runtime/.env` like the other `CHANGEME` secrets) or an OpenSSH private key.
//!
//! Login is the user facade: a password through `loginUser` (failures count toward the lockout, and an account that
//! needs a second factor cannot log in here), or a public key whose fingerprint is an active `UserSshKey` of that
//! user. The `sftp` subsystem needs the `SSH_SFTP` permission and serves `ssh_sftp_root`; `exec` and `shell` need
//! `SSH_SHELL` and run [`super::shell`]. At most `ssh_max_sessions` logged-in sessions are open at once. A connection
//! that has not logged in has [`PREAUTH_TIMEOUT`] to do so, and at most [`MAX_PREAUTH`] such connections wait at once.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use moqui_core::context::{Error as CoreError, ExecutionContextFactory};
use moqui_lib::error::Error;
use moqui_lib::object::Value;
use russh::keys::{HashAlg, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId, ChannelOpenFailure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{on_pool, sftp_fs::SftpFs, shell};

const DEFAULT_MAX_SESSIONS: usize = 8;
const DEFAULT_MAX_FILE: usize = 256 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(600);
/// How long a connection has from accept to a successful login.
const PREAUTH_TIMEOUT: Duration = Duration::from_secs(30);
/// Connections that have not logged in yet, at once; one more is closed. They do not count toward `ssh_max_sessions`.
const MAX_PREAUTH: usize = 16;
/// The longest admin shell line, and the longest `js` block.
const MAX_LINE: usize = 1024 * 1024;
const PROMPT: &str = "moqui> ";

/// The address the server listens on, kept on the factory for `Server::ssh_address`.
#[derive(Default)]
pub struct SshAddress(pub std::sync::Mutex<Option<SocketAddr>>);

/// A running server. Drop stops the listener and disconnects every session.
pub struct SshServer {
    address: SocketAddr,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl SshServer {
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for SshServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

struct Shared {
    factory: std::sync::Weak<ExecutionContextFactory>,
    root: String,
    max_file: usize,
    /// The cluster topics received, shared with the HTTPS cluster listener.
    cluster_posts: Arc<std::sync::Mutex<Vec<String>>>,
}

/// The host key from `ssh_host_key`: an OpenSSH private key, or the base64 of a 32-byte Ed25519 seed.
///
/// # Errors
///
/// Returns an error when the property is empty, still the placeholder, or does not decode.
pub fn host_key(text: &str) -> Result<PrivateKey, Error> {
    let text = text.trim();
    if text.is_empty() || text.ends_with("CHANGEME") {
        return Err(Error::new("ssh_host_key is not set; the SSH server needs a host key"));
    }
    if text.starts_with("-----BEGIN") {
        return russh::keys::decode_secret_key(text, None)
            .map_err(|err| Error::new(format!("ssh_host_key does not decode: {err}")));
    }
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|err| Error::new(format!("ssh_host_key is not base64: {err}")))?;
    let seed: [u8; 32] = seed
        .try_into()
        .map_err(|_| Error::new("ssh_host_key must be the base64 of 32 bytes, or an OpenSSH private key"))?;
    let keypair = russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&seed);
    Ok(PrivateKey::from(keypair))
}

/// Start the server when `ssh_port` is set (`0` binds an ephemeral port).
///
/// # Errors
///
/// Returns an error when the host key, the address, or the bind fails.
pub fn start(
    factory: &Arc<ExecutionContextFactory>,
    runtime_dir: &std::path::Path,
    cluster_posts: Arc<std::sync::Mutex<Vec<String>>>,
) -> Result<Option<SshServer>, Error> {
    moqui_lib::env::set_property(moqui_core::cluster::SSH_FINGERPRINT_PROPERTY, "");
    moqui_lib::env::set_property(moqui_core::cluster::SSH_ADDRESS_PROPERTY, "");
    let port_text = moqui_lib::env::get_prop_or_env("ssh_port");
    let port_text = port_text.trim();
    if port_text.is_empty() {
        return Ok(None);
    }
    let port: u16 = port_text
        .parse()
        .map_err(|_| Error::new(format!("ssh_port {port_text} is not a port number")))?;
    let host = match moqui_lib::env::get_prop_or_env("ssh_host").trim() {
        "" => "127.0.0.1".to_string(),
        other => other.to_string(),
    };
    let key = host_key(&moqui_lib::env::get_prop_or_env("ssh_host_key"))?;
    let root = match moqui_lib::env::get_prop_or_env("ssh_sftp_root").trim() {
        "" => "runtime://sftp".to_string(),
        other => other.to_string(),
    };
    if root.starts_with("runtime:") {
        let relative = root.trim_start_matches("runtime://").trim_start_matches("runtime:");
        let _ = std::fs::create_dir_all(runtime_dir.join(relative.trim_start_matches('/')));
    }
    let max_sessions = number_property("ssh_max_sessions", DEFAULT_MAX_SESSIONS);
    let max_file = number_property("ssh_sftp_max_file", DEFAULT_MAX_FILE);
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(russh::server::Config {
        keys: vec![key],
        inactivity_timeout: Some(IDLE),
        auth_rejection_time: Duration::from_millis(500),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        max_auth_attempts: 6,
        nodelay: true,
        ..russh::server::Config::default()
    });
    let bind = format!("{host}:{port}");
    let listener = crate::edge::block_on(async move { TcpListener::bind(bind).await })?
        .map_err(|err| Error::new(format!("could not bind the SSH server to {host}:{port}: {err}")))?;
    let address = listener
        .local_addr()
        .map_err(|err| Error::new("the SSH listener has no address").with_source(err))?;
    let (shutdown, watch) = tokio::sync::watch::channel(false);
    let shared = Arc::new(Shared {
        factory: Arc::downgrade(factory),
        root,
        max_file,
        cluster_posts,
    });
    crate::edge::handle().spawn(accept_loop(listener, config, shared, watch, max_sessions));
    *factory
        .extension(|_| SshAddress::default())
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(address);
    tracing::info!("SSH server listening on {address}");
    // For the cluster member row: peers pin this key, and the ssh:// base URL defaults to this address.
    moqui_lib::env::set_property(moqui_core::cluster::SSH_FINGERPRINT_PROPERTY, fingerprint);
    moqui_lib::env::set_property(moqui_core::cluster::SSH_ADDRESS_PROPERTY, address.to_string());
    Ok(Some(SshServer { address, shutdown }))
}

fn number_property(name: &str, default: usize) -> usize {
    moqui_lib::env::get_prop_or_env(name)
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// The connections counted against the two limits.
struct Counts {
    preauth: AtomicUsize,
    sessions: AtomicUsize,
    max_sessions: usize,
}

/// One connection's place in [`Counts`]: a pre-auth slot from accept, traded for a session at login. Drop gives back
/// whichever it holds.
struct Slot {
    counts: Arc<Counts>,
    preauth: AtomicBool,
    session: AtomicBool,
}

impl Slot {
    /// A pre-auth slot, or `None` when [`MAX_PREAUTH`] connections are already waiting to log in.
    fn preauth(counts: &Arc<Counts>) -> Option<Self> {
        if counts.preauth.fetch_add(1, Ordering::AcqRel) >= MAX_PREAUTH {
            counts.preauth.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self {
            counts: Arc::clone(counts),
            preauth: AtomicBool::new(true),
            session: AtomicBool::new(false),
        })
    }

    /// Trade the pre-auth slot for a session. False when `ssh_max_sessions` are open.
    fn log_in(&self) -> bool {
        if self.session.load(Ordering::Acquire) {
            return true;
        }
        if self.counts.sessions.fetch_add(1, Ordering::AcqRel) >= self.counts.max_sessions {
            self.counts.sessions.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        self.session.store(true, Ordering::Release);
        if self.preauth.swap(false, Ordering::AcqRel) {
            self.counts.preauth.fetch_sub(1, Ordering::AcqRel);
        }
        true
    }

    fn logged_in(&self) -> bool {
        self.session.load(Ordering::Acquire)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.preauth.swap(false, Ordering::AcqRel) {
            self.counts.preauth.fetch_sub(1, Ordering::AcqRel);
        }
        if self.session.swap(false, Ordering::AcqRel) {
            self.counts.sessions.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// Resolves [`PREAUTH_TIMEOUT`] after `since` when the connection has not logged in by then, and never otherwise.
async fn preauth_expired(slot: &Slot, since: tokio::time::Instant) {
    tokio::time::sleep_until(since + PREAUTH_TIMEOUT).await;
    if slot.logged_in() {
        std::future::pending::<()>().await;
    }
}

async fn accept_loop(
    listener: TcpListener,
    config: Arc<russh::server::Config>,
    shared: Arc<Shared>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    max_sessions: usize,
) {
    let counts = Arc::new(Counts {
        preauth: AtomicUsize::new(0),
        sessions: AtomicUsize::new(0),
        max_sessions,
    });
    loop {
        let accepted = tokio::select! {
            _ = shutdown.changed() => return,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, peer)) = accepted else {
            continue;
        };
        let Some(slot) = Slot::preauth(&counts) else {
            tracing::warn!("SSH connection from {peer} refused: {MAX_PREAUTH} connections are waiting to log in");
            drop(stream);
            continue;
        };
        let slot = Arc::new(slot);
        let since = tokio::time::Instant::now();
        let handler = Connection {
            shared: Arc::clone(&shared),
            slot: Arc::clone(&slot),
            peer,
            username: None,
            cluster: false,
            can_sftp: false,
            can_shell: false,
            can_git: false,
            channels: HashMap::new(),
            ptys: HashSet::new(),
            git_protocol: HashMap::new(),
        };
        let config = Arc::clone(&config);
        let mut stop = shutdown.clone();
        tokio::spawn(async move {
            let _ = stream.set_nodelay(true);
            // The version exchange waits on the client too, so the login deadline covers it.
            let deadline = since + PREAUTH_TIMEOUT;
            match tokio::time::timeout_at(deadline, russh::server::run_stream(config, stream, handler)).await {
                Ok(Ok(session)) => {
                    let handle = session.handle();
                    tokio::select! {
                        _ = stop.changed() => {
                            let _ = handle.disconnect(russh::Disconnect::ByApplication, "server stopping".into(), String::new()).await;
                        }
                        () = preauth_expired(&slot, since) => {
                            tracing::debug!("SSH connection from {peer} did not log in within {PREAUTH_TIMEOUT:?}");
                            let _ = handle.disconnect(russh::Disconnect::ByApplication, "login timed out".into(), String::new()).await;
                        }
                        result = session => {
                            if let Err(err) = result {
                                tracing::debug!("SSH session from {peer} ended: {err:?}");
                            }
                        }
                    }
                }
                Ok(Err(err)) => tracing::debug!("SSH connection from {peer} failed: {err:?}"),
                Err(_) => tracing::debug!("SSH connection from {peer} sent no version within {PREAUTH_TIMEOUT:?}"),
            }
            drop(slot);
        });
    }
}

/// One client connection.
struct Connection {
    shared: Arc<Shared>,
    /// This connection's place in the pre-auth and session counts.
    slot: Arc<Slot>,
    peer: SocketAddr,
    username: Option<String>,
    /// A cluster peer logged in with the cluster key: it may open `cluster <topic>` channels and nothing else.
    cluster: bool,
    can_sftp: bool,
    can_shell: bool,
    /// `git-upload-pack` and `git-receive-pack` (`SSH_GIT`); each repository is still checked.
    can_git: bool,
    channels: HashMap<ChannelId, Channel<Msg>>,
    ptys: HashSet<ChannelId>,
    /// `GIT_PROTOCOL` of each channel's `env` request.
    git_protocol: HashMap<ChannelId, String>,
}

/// The SSH permissions of the user now logged in.
#[derive(Clone, Copy, Debug)]
struct Permissions {
    sftp: bool,
    shell: bool,
    git: bool,
}

fn ssh_permissions() -> Result<Permissions, CoreError> {
    Ok(Permissions {
        sftp: moqui_core::user::has_permission("SSH_SFTP")?,
        shell: moqui_core::user::has_permission("SSH_SHELL")?,
        git: moqui_core::user::has_permission("SSH_GIT")?,
    })
}

impl Connection {
    /// Let the user in when a permission allows something. A user who may only use git logs in by key, unless
    /// `ssh_git_password_auth` is true: git hosting opens the server to every developer, and keys are not guessed.
    fn accept(&mut self, username: &str, permissions: Permissions, by_password: bool) -> Auth {
        let Permissions { sftp, shell, git } = permissions;
        if !sftp && !shell && !git {
            tracing::warn!(
                "SSH login of {username} from {}: no SSH_SFTP, SSH_SHELL or SSH_GIT permission",
                self.peer
            );
            return Auth::reject();
        }
        if !sftp
            && !shell
            && by_password
            && moqui_lib::env::get_prop_or_env("ssh_git_password_auth").trim() != "true"
        {
            tracing::warn!(
                "SSH password login of {username} from {} refused: git over SSH takes a key (ssh_git_password_auth)",
                self.peer
            );
            return Auth::reject();
        }
        if !self.slot.log_in() {
            tracing::warn!(
                "SSH login of {username} from {} refused: ssh_max_sessions {} are open",
                self.peer,
                self.slot.counts.max_sessions
            );
            return Auth::reject();
        }
        tracing::info!("SSH login of {username} from {}", self.peer);
        self.username = Some(username.to_string());
        self.can_sftp = sftp;
        self.can_shell = shell;
        self.can_git = git;
        Auth::Accept
    }
}

impl russh::server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let (username, secret) = (user.to_string(), password.to_string());
        let checked = on_pool(&self.shared.factory, move |_| {
            moqui_core::user::login_user(&username, &secret)?;
            if moqui_core::user::current_user_id()?.is_none_or(|id| id.is_empty()) {
                return Err(CoreError::new("a second factor is required, which SSH does not take"));
            }
            ssh_permissions()
        })
        .await;
        match checked {
            Ok(permissions) => Ok(self.accept(user, permissions, true)),
            Err(err) => {
                tracing::warn!(
                    "SSH password login of {user} from {} failed: {}",
                    self.peer,
                    err.message()
                );
                Ok(Auth::reject())
            }
        }
    }

    async fn auth_publickey(&mut self, user: &str, public_key: &PublicKey) -> Result<Auth, Self::Error> {
        if user == super::cluster::CLUSTER_USER {
            if super::cluster::is_cluster_key(public_key) {
                if !self.slot.log_in() {
                    tracing::warn!(
                        "cluster peer from {} refused: ssh_max_sessions {} are open",
                        self.peer,
                        self.slot.counts.max_sessions
                    );
                    return Ok(Auth::reject());
                }
                tracing::info!("cluster peer connected from {}", self.peer);
                self.cluster = true;
                self.username = Some(user.to_string());
                return Ok(Auth::Accept);
            }
            tracing::warn!(
                "cluster login from {} with a key that is not the cluster key",
                self.peer
            );
            return Ok(Auth::reject());
        }
        let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
        let username = user.to_string();
        let checked = on_pool(&self.shared.factory, move |factory| {
            if !key_belongs_to(factory, &fingerprint, &username)? {
                return Err(CoreError::new(format!(
                    "no active UserSshKey {fingerprint} for {username}"
                )));
            }
            moqui_core::user::internal_login_user(&username)?;
            ssh_permissions()
        })
        .await;
        match checked {
            Ok(permissions) => Ok(self.accept(user, permissions, false)),
            Err(err) => {
                tracing::warn!("SSH key login of {user} from {} failed: {}", self.peer, err.message());
                Ok(Auth::reject())
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.username.is_none() {
            reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        }
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.ptys.insert(channel);
        session.channel_success(channel)
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" || !self.can_sftp {
            return session.channel_failure(channel);
        }
        let Some(open) = self.channels.remove(&channel) else {
            return session.channel_failure(channel);
        };
        session.channel_success(channel)?;
        let fs = SftpFs::new(
            std::sync::Weak::clone(&self.shared.factory),
            self.shared.root.clone(),
            self.shared.max_file,
        );
        russh_sftp::server::run(open.into_stream(), fs).await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.cluster {
            let command = String::from_utf8_lossy(data).into_owned();
            let topic = command
                .strip_prefix("cluster ")
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let Some(open) = self.channels.remove(&channel).filter(|_| !topic.is_empty()) else {
                return session.channel_failure(channel);
            };
            session.channel_success(channel)?;
            let factory = std::sync::Weak::clone(&self.shared.factory);
            let posts = Arc::clone(&self.shared.cluster_posts);
            tokio::spawn(super::cluster::serve_channel(open.into_stream(), factory, posts, topic));
            return Ok(());
        }
        let line = String::from_utf8_lossy(data).into_owned();
        if moqui_core::resource::git::server::parse_ssh_command(&line).is_some()
            || moqui_core::resource::git::server::lfs::parse_ssh_command(&line).is_some()
        {
            let (Some(username), true) = (self.username.clone(), self.can_git) else {
                return session.channel_failure(channel);
            };
            let Some(open) = self.channels.remove(&channel) else {
                return session.channel_failure(channel);
            };
            session.channel_success(channel)?;
            tokio::spawn(super::git::serve_channel(
                open,
                channel,
                session.handle(),
                std::sync::Weak::clone(&self.shared.factory),
                username,
                line,
                self.git_protocol.remove(&channel),
            ));
            return Ok(());
        }
        let (Some(username), true) = (self.username.clone(), self.can_shell) else {
            return session.channel_failure(channel);
        };
        let Some(open) = self.channels.remove(&channel) else {
            return session.channel_failure(channel);
        };
        session.channel_success(channel)?;
        let factory = std::sync::Weak::clone(&self.shared.factory);
        let handle = session.handle();
        let newline = if self.ptys.contains(&channel) { "\r\n" } else { "\n" };
        tokio::spawn(async move {
            let mut stream = open.into_stream();
            let outcome = shell::run(&factory, &username, &line).await;
            let text = with_newlines(&outcome.text, newline);
            if !text.is_empty() {
                let _ = stream.write_all(text.as_bytes()).await;
                let _ = stream.write_all(newline.as_bytes()).await;
            }
            let _ = stream.flush().await;
            let _ = handle.exit_status_request(channel, outcome.status).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
        Ok(())
    }

    /// `GIT_PROTOCOL` (protocol v2 for git) is kept for the channel's exec; other variables are refused, as
    /// OpenSSH's default `AcceptEnv` does.
    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if variable_name == "GIT_PROTOCOL" && self.username.is_some() {
            self.git_protocol.insert(channel, variable_value.to_string());
            return session.channel_success(channel);
        }
        session.channel_failure(channel)
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        let (Some(username), true) = (self.username.clone(), self.can_shell) else {
            return session.channel_failure(channel);
        };
        let Some(open) = self.channels.remove(&channel) else {
            return session.channel_failure(channel);
        };
        session.channel_success(channel)?;
        let factory = std::sync::Weak::clone(&self.shared.factory);
        let handle = session.handle();
        let pty = self.ptys.contains(&channel);
        tokio::spawn(async move {
            let status = interactive(open.into_stream(), &factory, &username, pty).await;
            let _ = handle.exit_status_request(channel, status).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
        Ok(())
    }
}

/// Whether an active `UserSshKey` with this fingerprint belongs to the account named `username`.
fn key_belongs_to(factory: &ExecutionContextFactory, fingerprint: &str, username: &str) -> Result<bool, CoreError> {
    let _authz =
        moqui_core::context::ExecutionContext::with_current(moqui_core::context::ExecutionContext::disable_authz);
    let facade = factory
        .entity()
        .ok_or_else(|| CoreError::new("the entity facade is not installed"))?;
    let Some(account) = facade
        .find("moqui.security.UserAccount")?
        .eq("username", Value::string(username))
        .one()?
    else {
        return Ok(false);
    };
    let user_id = account
        .get("userId")
        .map(moqui_lib::object::to_plain_string)
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(i64::MAX);
    let keys = facade
        .find("moqui.security.UserSshKey")?
        .eq("userId", Value::string(&user_id))
        .list()?;
    Ok(keys.values().iter().any(|key| {
        let stored = key
            .get("fingerprint")
            .map(moqui_lib::object::to_plain_string)
            .unwrap_or_default();
        let from = key.get("fromDate").and_then(millis);
        let thru = key.get("thruDate").and_then(millis);
        super::client::same_fingerprint(&stored, fingerprint)
            && from.is_none_or(|from| from <= now)
            && thru.is_none_or(|thru| thru > now)
    }))
}

fn millis(value: &Value) -> Option<i64> {
    match moqui_lib::object::basic_convert(value, "Timestamp").value {
        Value::Timestamp(stamp) => Some(stamp.as_millisecond()),
        _ => None,
    }
}

fn with_newlines(text: &str, newline: &str) -> String {
    if newline == "\n" {
        text.to_string()
    } else {
        text.replace('\n', newline)
    }
}

/// The interactive loop: a prompt, a line, its output. With a PTY the client does not echo, so this echoes and
/// handles backspace, ctrl-C (drop the line) and ctrl-D (end on an empty line). Returns the exit status.
async fn interactive<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    mut stream: S,
    factory: &std::sync::Weak<ExecutionContextFactory>,
    username: &str,
    pty: bool,
) -> u32 {
    let newline = if pty { "\r\n" } else { "\n" };
    let greeting = format!("MR-Moqui admin shell for {username}; type help{newline}{PROMPT}");
    if stream.write_all(greeting.as_bytes()).await.is_err() {
        return 1;
    }
    let mut line: Vec<u8> = Vec::new();
    let mut block: Option<Vec<String>> = None;
    // Bytes past MAX_LINE are not kept; the line is refused when it ends.
    let mut overflow = false;
    let mut block_len = 0_usize;
    let mut buffer = [0u8; 1024];
    loop {
        let read = match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return 0,
            Ok(read) => read,
        };
        for &byte in &buffer[..read] {
            let mut echo: Vec<u8> = Vec::new();
            let mut finished = None;
            match byte {
                b'\r' | b'\n' => {
                    if byte == b'\n' && pty {
                        continue;
                    }
                    if pty {
                        echo.extend_from_slice(b"\r\n");
                    }
                    finished = Some(String::from_utf8_lossy(&line).into_owned());
                    line.clear();
                }
                0x7f | 0x08 => {
                    if line.pop().is_some() && pty {
                        echo.extend_from_slice(b"\x08 \x08");
                    }
                }
                0x03 => {
                    line.clear();
                    overflow = false;
                    block = None;
                    if pty {
                        echo.extend_from_slice(format!("^C{newline}{PROMPT}").as_bytes());
                    }
                }
                0x04 if line.is_empty() => return 0,
                byte if byte >= 0x20 || byte == b'\t' => {
                    if line.len() < MAX_LINE {
                        line.push(byte);
                        if pty {
                            echo.push(byte);
                        }
                    } else {
                        overflow = true;
                    }
                }
                _ => {}
            }
            if !echo.is_empty() && stream.write_all(&echo).await.is_err() {
                return 1;
            }
            let Some(text) = finished else {
                continue;
            };
            if std::mem::take(&mut overflow) {
                block = None;
                let message = format!("the line is longer than {MAX_LINE} bytes and was dropped");
                if refuse(&mut stream, &message, newline).await.is_err() {
                    return 1;
                }
                continue;
            }
            // A `js` block gathers lines until a line with only `.`.
            if let Some(lines) = block.as_mut() {
                block_len = block_len.saturating_add(text.len() + 1);
                if block_len > MAX_LINE {
                    block = None;
                    let message = format!("the js block is longer than {MAX_LINE} bytes and was dropped");
                    if refuse(&mut stream, &message, newline).await.is_err() {
                        return 1;
                    }
                    continue;
                }
                if text.trim() == "." {
                    let code = lines.join("\n");
                    block = None;
                    let outcome = shell::run(factory, username, &format!("js {code}")).await;
                    if write_outcome(&mut stream, &outcome.text, newline).await.is_err() {
                        return 1;
                    }
                } else {
                    lines.push(text);
                    if stream.write_all(b"... ").await.is_err() {
                        return 1;
                    }
                    continue;
                }
            } else if shell::starts_block(&text) {
                block = Some(Vec::new());
                block_len = 0;
                if stream
                    .write_all(format!("end with a line of .{newline}... ").as_bytes())
                    .await
                    .is_err()
                {
                    return 1;
                }
                continue;
            } else {
                let outcome = shell::run(factory, username, &text).await;
                if outcome.exit {
                    let _ = stream.write_all(format!("bye{newline}").as_bytes()).await;
                    return 0;
                }
                if write_outcome(&mut stream, &outcome.text, newline).await.is_err() {
                    return 1;
                }
            }
            if stream.write_all(PROMPT.as_bytes()).await.is_err() {
                return 1;
            }
        }
    }
}

/// An input the shell refuses: the message, then a fresh prompt.
async fn refuse<W: AsyncWriteExt + Unpin>(stream: &mut W, message: &str, newline: &str) -> std::io::Result<()> {
    stream
        .write_all(format!("error: {message}{newline}{PROMPT}").as_bytes())
        .await
}

async fn write_outcome<W: AsyncWriteExt + Unpin>(stream: &mut W, text: &str, newline: &str) -> std::io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    stream.write_all(with_newlines(text, newline).as_bytes()).await?;
    stream.write_all(newline.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{interactive, Counts, Slot, MAX_LINE, MAX_PREAUTH};

    #[tokio::test]
    async fn an_overlong_shell_line_or_js_block_is_refused() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let shell = tokio::spawn(async move { interactive(server, &std::sync::Weak::new(), "tester", false).await });
        let (mut reader, mut writer) = tokio::io::split(client);
        let send = tokio::spawn(async move {
            writer.write_all(&vec![b'a'; MAX_LINE + 10]).await.unwrap();
            writer.write_all(b"\njs\n").await.unwrap();
            let chunk = vec![b'b'; MAX_LINE / 2];
            for _ in 0..3 {
                writer.write_all(&chunk).await.unwrap();
                writer.write_all(b"\n").await.unwrap();
            }
            writer.write_all(b"exit\n").await.unwrap();
            writer
        });
        let mut seen = Vec::new();
        reader.read_to_end(&mut seen).await.unwrap();
        let seen = String::from_utf8_lossy(&seen);
        assert!(seen.contains("error: the line is longer than"), "{seen}");
        assert!(seen.contains("error: the js block is longer than"), "{seen}");
        assert!(seen.ends_with("bye\n"), "{seen}");
        drop(send.await.unwrap());
        assert_eq!(shell.await.unwrap(), 0);
    }

    #[test]
    fn a_login_trades_the_preauth_slot_for_a_session() {
        let counts = Arc::new(Counts {
            preauth: AtomicUsize::new(0),
            sessions: AtomicUsize::new(0),
            max_sessions: 1,
        });
        let waiting: Vec<Slot> = (0..MAX_PREAUTH).map(|_| Slot::preauth(&counts).unwrap()).collect();
        assert!(Slot::preauth(&counts).is_none(), "past MAX_PREAUTH");
        assert!(waiting[0].log_in());
        // The login gave its pre-auth slot back, and took the one session.
        let next = Slot::preauth(&counts).unwrap();
        assert!(!next.log_in(), "past ssh_max_sessions");
        drop(waiting);
        assert!(next.log_in());
        drop(next);
        assert_eq!(counts.preauth.load(Ordering::Acquire), 0);
        assert_eq!(counts.sessions.load(Ordering::Acquire), 0);
    }
}
