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

//! A container that stays (plans/archive/Vault-Instances.md §4.2): what a managed instance's container is made of, its secret
//! files, and the check that no secret is in its environment, labels or command, where `docker inspect` and
//! `/proc/*/environ` show them. A secret is a file: put into the created container before it starts on Docker
//! ([`super::archive`]), a Podman secret on Podman ([`super::podman`]).

use std::time::Duration;

use serde_json::json;
use zeroize::Zeroizing;

use crate::error::Error;

/// The label that names a container's Podman secrets, so its removal deletes them. It holds names, never a value.
pub const SECRETS_LABEL: &str = "moqui.secrets";
/// Where the secret files are, in the container.
pub const SECRETS_DIR: &str = "/run/secrets";
/// A secret this long or longer is refused anywhere inside a value; a shorter one only as a whole value, so a short
/// secret does not refuse every value it happens to be part of.
const SUBSTRING_MIN: usize = 8;
/// The engines' API sockets: a container that binds one, or a directory that holds one, commands the host.
const ENGINE_SOCKETS: [&str; 6] = [
    "/var/run/docker.sock",
    "/run/docker.sock",
    "/run/podman/podman.sock",
    "/var/run/podman/podman.sock",
    "/run/containerd/containerd.sock",
    "/var/run/containerd/containerd.sock",
];
/// The file names of those sockets, wherever they are (a rootless Podman's is under `/run/user/<uid>`).
const SOCKET_NAMES: [&str; 3] = ["docker.sock", "podman.sock", "containerd.sock"];

/// A secret given to a container as the file `/run/secrets/<name>`, owned by `uid`:`gid` with `mode`. Its value is
/// zeroized when dropped, and its `Debug` does not show it.
pub struct SecretFile {
    pub name: String,
    pub bytes: Zeroizing<Vec<u8>>,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

impl SecretFile {
    /// A secret file readable only by its owner (0400), owned by `uid`:`gid`.
    pub fn new(name: impl Into<String>, bytes: Zeroizing<Vec<u8>>, uid: u32, gid: u32) -> Self {
        Self {
            name: name.into(),
            bytes,
            uid,
            gid,
            mode: 0o400,
        }
    }

    /// Its path in the container.
    pub fn path(&self) -> String {
        format!("{SECRETS_DIR}/{}", self.name)
    }
}

impl std::fmt::Debug for SecretFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretFile")
            .field("name", &self.name)
            .field("bytes", &format_args!("({} bytes)", self.bytes.len()))
            .field("uid", &self.uid)
            .field("gid", &self.gid)
            .field("mode", &format_args!("{:o}", self.mode))
            .finish()
    }
}

/// What a mount's source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    /// A named volume of the host.
    Volume,
    /// A path of the host.
    Bind,
}

/// A volume or a host path in the container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub kind: MountKind,
    /// The volume's name, or the host's path.
    pub source: String,
    /// The path in the container.
    pub target: String,
    pub read_only: bool,
}

/// A port of the container published on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortBinding {
    pub container_port: u16,
    /// `tcp` or `udp`.
    pub protocol: String,
    /// The host's port; the engine picks one when `None`.
    pub host_port: Option<u16>,
    /// The host's address to listen on (`127.0.0.1`); every address when `None`.
    pub host_ip: Option<String>,
}

/// When the engine starts the container again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestartPolicy {
    #[default]
    No,
    Always,
    UnlessStopped,
    /// On a non-zero exit, at most this many times (0: no limit).
    OnFailure(u32),
}

impl RestartPolicy {
    /// The policy's name in both APIs.
    pub fn name(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::UnlessStopped => "unless-stopped",
            Self::OnFailure(_) => "on-failure",
        }
    }

    fn retries(self) -> u32 {
        match self {
            Self::OnFailure(retries) => retries,
            _ => 0,
        }
    }
}

/// The engine's health check of the container. `test` is in the engine's form: `["CMD", "pg_isready"]`,
/// `["CMD-SHELL", "pg_isready -U app"]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthCheck {
    pub test: Vec<String>,
    pub interval: Option<Duration>,
    pub timeout: Option<Duration>,
    pub retries: Option<u32>,
    pub start_period: Option<Duration>,
}

impl HealthCheck {
    fn json(&self) -> serde_json::Value {
        let nanos = |duration: Option<Duration>| duration.map_or(0, |duration| duration.as_nanos());
        json!({
            "Test": self.test,
            "Interval": u64::try_from(nanos(self.interval)).unwrap_or(u64::MAX),
            "Timeout": u64::try_from(nanos(self.timeout)).unwrap_or(u64::MAX),
            "Retries": self.retries.unwrap_or(0),
            "StartPeriod": u64::try_from(nanos(self.start_period)).unwrap_or(u64::MAX),
        })
    }
}

/// What a container may do to its host and itself, beyond the engine's defaults: for a container that runs what a user
/// or an agent asks (moqui-agent-workbench's sandbox), each limit of `docker run`'s hardening, for both engines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hardening {
    /// The root filesystem read only. On Docker a secret file then goes into an anonymous volume at [`SECRETS_DIR`]
    /// (Docker refuses an archive into a read-only root), deleted with the container's anonymous volumes.
    pub read_only: bool,
    /// Memory filesystems: (target, options such as `rw,nosuid,nodev,size=1g`).
    pub tmpfs: Vec<(String, String)>,
    /// Drop every capability.
    pub cap_drop_all: bool,
    /// No process gains privileges (setuid, file capabilities).
    pub no_new_privileges: bool,
    /// Memory at most, in bytes.
    pub memory_bytes: Option<i64>,
    /// CPUs at most, in billionths of a CPU (2 CPUs: 2,000,000,000).
    pub nano_cpus: Option<i64>,
    /// Processes at most.
    pub pids_limit: Option<i64>,
    /// Names the container resolves (`host.docker.internal:host-gateway`).
    pub extra_hosts: Vec<String>,
    /// Podman's user namespace (`keep-id:uid=1000,gid=1000`: the host user is the image's user, who owns the files
    /// mounted for it); Docker has none per container.
    pub podman_userns: Option<String>,
}

/// A container that stays: made by [`super::DockerHost::create_spec`], started, stopped and removed by its id.
#[derive(Debug, Default)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    /// Pull the image when the host does not have it.
    pub pull: bool,
    /// The command, when not the image's.
    pub cmd: Vec<String>,
    /// The entrypoint, when not the image's: a wrapper that reads a secret file into what an image that reads no
    /// `*_FILE` setting takes. Checked as the command is.
    pub entrypoint: Vec<String>,
    /// Settings that are not secret. A secret's value here is refused ([`Self::check`]): a secret is a file, read
    /// through the image's `*_FILE` setting (`POSTGRES_PASSWORD_FILE=/run/secrets/postgres-password`).
    pub env: Vec<(String, String)>,
    pub labels: Vec<(String, String)>,
    pub mounts: Vec<Mount>,
    /// The network it is on, when not the engine's default.
    pub network: Option<String>,
    pub ports: Vec<PortBinding>,
    pub restart: RestartPolicy,
    pub health: Option<HealthCheck>,
    /// The user it runs as (`uid:gid`), when not the image's.
    pub user: Option<String>,
    pub secrets: Vec<SecretFile>,
    /// Let the container bind an engine's API socket (or a directory holding one) or use the host's network
    /// ([`Self::check`] refuses both otherwise). The instance kinds never set it.
    pub allow_host_access: bool,
    /// Its limits beyond the engine's defaults.
    pub hardening: Hardening,
}

/// The parts of an absolute path, without empty parts and `.`; `None` when it is not absolute or has a `..`.
fn path_parts(path: &str) -> Option<Vec<&str>> {
    if !path.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    (!parts.contains(&"..")).then_some(parts)
}

/// Whether the path `a` is `b`, inside it, or holds it.
fn overlaps(a: &[&str], b: &[&str]) -> bool {
    let shorter = a.len().min(b.len());
    a[..shorter] == b[..shorter]
}

/// Whether a bind of the host's `source` would reach an engine's API socket: the socket, or a directory that holds
/// one.
fn reaches_engine_socket(source: &[&str]) -> bool {
    let named = source.last().is_some_and(|last| SOCKET_NAMES.contains(last));
    // a rootless Podman's socket is /run/user/<uid>/podman/podman.sock
    let rootless = match source {
        ["run", "user", rest @ ..] => rest.len() < 2 || rest[1] == "podman",
        _ => false,
    };
    let known = ENGINE_SOCKETS
        .iter()
        .filter_map(|socket| path_parts(socket))
        .any(|socket| source.len() <= socket.len() && overlaps(source, &socket));
    named || rootless || known
}

/// Whether `name` may be a container's, a volume's or a secret's name, and so a part of an API path or an archive's
/// entry: a letter or digit, then letters, digits, `_`, `.` and `-`.
pub fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 128
        && bytes.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// The network modes an engine reads as a keyword, not a network's name: the host's network, another container's,
/// and Podman's user-mode networks and namespace options, which a network named so would be taken as.
pub const NETWORK_KEYWORDS: [&str; 6] = ["host", "pasta", "slirp4netns", "private", "container", "ns"];

/// Whether `network` may name the network a container is on: `bridge`, `none`, or a network's name ([`valid_name`])
/// that is no engine keyword ([`NETWORK_KEYWORDS`]). A list of what is refused would miss a mode: on Podman
/// `ns:<path>` joins the network namespace at a path, the host's included (`ns:/proc/1/ns/net`), `slirp4netns` and
/// `pasta` (with or without options) and `private` are modes of their own, and `host` and `container:<id>` are the
/// host's network and another container's (SEC-GIT-93).
pub fn valid_network(network: &str) -> bool {
    valid_name(network)
        && !NETWORK_KEYWORDS
            .iter()
            .any(|keyword| network.eq_ignore_ascii_case(keyword))
}

/// Whether `secret` is in `value`: anywhere in it when the secret is [`SUBSTRING_MIN`] bytes or longer, else as the
/// whole value. Not constant time: both sides are this process's own, and nothing outside it times the check.
fn holds(value: &[u8], secret: &[u8]) -> bool {
    let one = |secret: &[u8]| {
        if secret.len() < SUBSTRING_MIN {
            value == secret
        } else {
            value.windows(secret.len()).any(|window| window == secret)
        }
    };
    // a value read from a file often ends in a newline that a setting would not carry
    let trimmed = secret.trim_ascii();
    one(secret) || (!trimmed.is_empty() && trimmed.len() != secret.len() && one(trimmed))
}

impl ContainerSpec {
    /// Refuse a spec that would put a secret where `docker inspect` shows it: a secret's value in an environment
    /// setting (name or value), a label (name or value), the command or the health check's command. Also refuse a
    /// name that is not a valid name, two secrets of one name, a secret with no value or a mode that is not a file
    /// mode, and a label of the driver's own ([`SECRETS_LABEL`]). A mount at, inside or holding `/run/secrets` is
    /// refused, so a secret file cannot land in a volume or a host path. Unless [`Self::allow_host_access`], a bind
    /// that reaches an engine's API socket, and a network that is not `bridge`, `none` or a network's name (the
    /// host's, another container's, a namespace path, a Podman user-mode network: [`valid_network`]), are refused too.
    ///
    /// # Errors
    ///
    /// The first thing refused, naming the secret and where it is but never its value.
    pub fn check(&self) -> Result<(), Error> {
        if !valid_name(&self.name) {
            return Err(Error::new(format!("{:?} is not a container name", self.name)));
        }
        if self.image.trim().is_empty() || self.image.contains(char::is_whitespace) {
            return Err(Error::new(format!(
                "container {}: {:?} is not an image",
                self.name, self.image
            )));
        }
        if self.labels.iter().any(|(name, _)| name == SECRETS_LABEL) {
            return Err(Error::new(format!(
                "container {}: the label {SECRETS_LABEL} is the driver's own",
                self.name
            )));
        }
        self.check_mounts()?;
        self.check_hardening()?;
        let network = self.network.as_deref().unwrap_or_default().trim();
        if !self.allow_host_access && !network.is_empty() && !valid_network(network) {
            return Err(Error::new(format!(
                "container {}: the network {network} is not bridge, none or a network's name but the host's or another \
                 container's, a namespace or an engine's own mode, which a container that stays does not use",
                self.name
            )));
        }
        for (index, secret) in self.secrets.iter().enumerate() {
            // 64 at most, so its tar path fits a ustar header
            if !valid_name(&secret.name) || secret.name.len() > 64 {
                return Err(Error::new(format!(
                    "container {}: {:?} is not a secret file's name",
                    self.name, secret.name
                )));
            }
            if self.secrets[..index].iter().any(|other| other.name == secret.name) {
                return Err(Error::new(format!(
                    "container {}: two secrets are named {}",
                    self.name, secret.name
                )));
            }
            if secret.bytes.is_empty() {
                return Err(Error::new(format!(
                    "container {}: the secret {} has no value",
                    self.name, secret.name
                )));
            }
            if secret.mode > 0o777 {
                return Err(Error::new(format!(
                    "container {}: the secret {} has mode {:o}, which is not a file's",
                    self.name, secret.name, secret.mode
                )));
            }
            if let Some(place) = self.place_of(&secret.bytes) {
                return Err(Error::new(format!(
                    "container {}: the value of the secret {} is in its {place}; a secret is a file, never in a \
                     container's environment, labels or command (plans/archive/Vault-Instances.md section 4.2)",
                    self.name, secret.name
                )));
            }
        }
        Ok(())
    }

    /// Refuse a mount that overlaps `/run/secrets`, and a bind that reaches an engine's socket unless allowed.
    fn check_hardening(&self) -> Result<(), Error> {
        let hardening = &self.hardening;
        for (target, options) in &hardening.tmpfs {
            if path_parts(target).is_none_or(|parts| parts.is_empty()) {
                return Err(Error::new(format!(
                    "container {}: the tmpfs target {target:?} is not an absolute path without ..",
                    self.name
                )));
            }
            if !options.chars().all(|c| c.is_ascii_alphanumeric() || ",=_-".contains(c)) {
                return Err(Error::new(format!(
                    "container {}: {options:?} are not tmpfs options",
                    self.name
                )));
            }
        }
        for host in &hardening.extra_hosts {
            let valid = host.split_once(':').is_some_and(|(name, address)| {
                !name.is_empty()
                    && !address.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
                    && address.chars().all(|c| c.is_ascii_alphanumeric() || ".:-".contains(c))
            });
            if !valid {
                return Err(Error::new(format!(
                    "container {}: {host:?} is not a host's name and address",
                    self.name
                )));
            }
        }
        if let Some(userns) = &hardening.podman_userns {
            if userns.is_empty() || !userns.chars().all(|c| c.is_ascii_alphanumeric() || ":=,-".contains(c)) {
                return Err(Error::new(format!(
                    "container {}: {userns:?} is not a user namespace",
                    self.name
                )));
            }
        }
        for (what, value) in [
            ("memory", hardening.memory_bytes),
            ("CPU", hardening.nano_cpus),
            ("process", hardening.pids_limit),
        ] {
            if value.is_some_and(|limit| limit <= 0) {
                return Err(Error::new(format!("container {}: a {what} limit is more than 0", self.name)));
            }
        }
        Ok(())
    }

    /// Whether the secret files go into an anonymous volume at [`SECRETS_DIR`]: a read-only root, on Docker.
    pub(crate) fn secrets_in_volume(&self) -> bool {
        self.hardening.read_only && !self.secrets.is_empty()
    }

    fn check_mounts(&self) -> Result<(), Error> {
        let secrets = path_parts(SECRETS_DIR).unwrap_or_default();
        for mount in &self.mounts {
            let Some(target) = path_parts(&mount.target) else {
                return Err(Error::new(format!(
                    "container {}: the mount target {:?} is not an absolute path without ..",
                    self.name, mount.target
                )));
            };
            if overlaps(&target, &secrets) {
                return Err(Error::new(format!(
                    "container {}: the mount at {} is at, inside or around {SECRETS_DIR}, where its secret files go",
                    self.name, mount.target
                )));
            }
            if mount.kind == MountKind::Bind && !self.allow_host_access {
                let Some(source) = path_parts(&mount.source) else {
                    return Err(Error::new(format!(
                        "container {}: the bind source {:?} is not an absolute path without ..",
                        self.name, mount.source
                    )));
                };
                if reaches_engine_socket(&source) {
                    return Err(Error::new(format!(
                        "container {}: the bind of {} reaches a container engine's API socket, which commands the \
                         host",
                        self.name, mount.source
                    )));
                }
            }
        }
        Ok(())
    }

    /// Where `secret` is in what `docker inspect` shows, if anywhere. A setting's or a label's name is told by its
    /// place, never shown: it may be the secret.
    fn place_of(&self, secret: &[u8]) -> Option<String> {
        for (index, (name, value)) in self.env.iter().enumerate() {
            if holds(name.as_bytes(), secret) {
                return Some(format!("environment (the name of setting #{index})"));
            }
            if holds(value.as_bytes(), secret) {
                return Some(format!("environment ({name})"));
            }
        }
        for (index, (name, value)) in self.labels.iter().enumerate() {
            if holds(name.as_bytes(), secret) {
                return Some(format!("labels (the name of label #{index})"));
            }
            if holds(value.as_bytes(), secret) {
                return Some(format!("labels ({name})"));
            }
        }
        if self.cmd.iter().any(|word| holds(word.as_bytes(), secret)) {
            return Some("command".to_string());
        }
        if self.entrypoint.iter().any(|word| holds(word.as_bytes(), secret)) {
            return Some("entrypoint".to_string());
        }
        let health = self.health.as_ref().map_or(&[][..], |health| &health.test[..]);
        if health.iter().any(|word| holds(word.as_bytes(), secret)) {
            return Some("health check".to_string());
        }
        None
    }

    fn env_list(&self) -> Vec<String> {
        self.env.iter().map(|(name, value)| format!("{name}={value}")).collect()
    }

    fn label_map(&self, extra: Option<(&str, String)>) -> serde_json::Map<String, serde_json::Value> {
        let mut labels: serde_json::Map<String, serde_json::Value> = self
            .labels
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        if let Some((name, value)) = extra {
            labels.insert(name.to_string(), json!(value));
        }
        labels
    }

    /// The body of Docker's `POST /containers/create`, which Podman's compatibility API takes too. No secret is in it:
    /// the secret files are put after it is made ([`super::archive`]).
    pub fn docker_body(&self) -> serde_json::Value {
        let mut exposed = serde_json::Map::new();
        let mut bindings = serde_json::Map::new();
        for port in &self.ports {
            let key = format!("{}/{}", port.container_port, port.protocol);
            exposed.insert(key.clone(), json!({}));
            let binding = json!({
                "HostIp": port.host_ip.clone().unwrap_or_default(),
                "HostPort": port.host_port.map(|port| port.to_string()).unwrap_or_default(),
            });
            match bindings.get_mut(&key) {
                Some(serde_json::Value::Array(list)) => list.push(binding),
                _ => {
                    bindings.insert(key, json!([binding]));
                }
            }
        }
        let mut mounts: Vec<serde_json::Value> = self
            .mounts
            .iter()
            .map(|mount| {
                json!({
                    "Type": match mount.kind { MountKind::Volume => "volume", MountKind::Bind => "bind" },
                    "Source": mount.source,
                    "Target": mount.target,
                    "ReadOnly": mount.read_only,
                })
            })
            .collect();
        if self.secrets_in_volume() {
            // an anonymous volume: the secret files go there before the start, as a read-only root takes none
            mounts.push(json!({ "Type": "volume", "Target": SECRETS_DIR, "ReadOnly": false }));
        }
        let mut host = json!({
            "Mounts": mounts,
            "PortBindings": bindings,
            "RestartPolicy": { "Name": self.restart.name(), "MaximumRetryCount": self.restart.retries() },
        });
        let hardening = &self.hardening;
        if hardening.read_only {
            host["ReadonlyRootfs"] = json!(true);
        }
        if !hardening.tmpfs.is_empty() {
            let tmpfs: serde_json::Map<String, serde_json::Value> = hardening
                .tmpfs
                .iter()
                .map(|(target, options)| (target.clone(), json!(options)))
                .collect();
            host["Tmpfs"] = serde_json::Value::Object(tmpfs);
        }
        if hardening.cap_drop_all {
            host["CapDrop"] = json!(["ALL"]);
        }
        if hardening.no_new_privileges {
            host["SecurityOpt"] = json!(["no-new-privileges"]);
        }
        if let Some(memory) = hardening.memory_bytes {
            host["Memory"] = json!(memory);
        }
        if let Some(cpus) = hardening.nano_cpus {
            host["NanoCpus"] = json!(cpus);
        }
        if let Some(pids) = hardening.pids_limit {
            host["PidsLimit"] = json!(pids);
        }
        if !hardening.extra_hosts.is_empty() {
            host["ExtraHosts"] = json!(hardening.extra_hosts);
        }
        if let Some(network) = &self.network {
            host["NetworkMode"] = json!(network);
        }
        let mut body = json!({
            "Image": self.image,
            "Env": self.env_list(),
            "Labels": self.label_map(None),
            "ExposedPorts": exposed,
            "HostConfig": host,
        });
        if !self.cmd.is_empty() {
            body["Cmd"] = json!(self.cmd);
        }
        if !self.entrypoint.is_empty() {
            body["Entrypoint"] = json!(self.entrypoint);
        }
        if let Some(user) = &self.user {
            body["User"] = json!(user);
        }
        if let Some(health) = &self.health {
            body["Healthcheck"] = health.json();
        }
        body
    }

    /// The body of Podman's `POST /libpod/containers/create` (its SpecGenerator), with each secret file as the Podman
    /// secret `sources[i]` mounted at `/run/secrets/<name>`, and the names of those secrets in [`SECRETS_LABEL`].
    pub fn podman_body(&self, sources: &[String]) -> serde_json::Value {
        let env: serde_json::Map<String, serde_json::Value> = self
            .env
            .iter()
            .map(|(name, value)| (name.clone(), json!(value)))
            .collect();
        let labels = self.label_map((!sources.is_empty()).then(|| (SECRETS_LABEL, sources.join(","))));
        let secrets: Vec<serde_json::Value> = self
            .secrets
            .iter()
            .zip(sources)
            .map(|(secret, source)| {
                // a target that is not a path is the file's name under /run/secrets, in every Podman that has secrets
                json!({
                    "source": source,
                    "target": secret.name,
                    "uid": secret.uid,
                    "gid": secret.gid,
                    "mode": secret.mode,
                })
            })
            .collect();
        let volumes: Vec<serde_json::Value> = self
            .mounts
            .iter()
            .filter(|mount| mount.kind == MountKind::Volume)
            .map(|mount| {
                let options: Vec<&str> = if mount.read_only { vec!["ro"] } else { Vec::new() };
                json!({ "Name": mount.source, "Dest": mount.target, "Options": options })
            })
            .collect();
        let mut binds: Vec<serde_json::Value> = self
            .mounts
            .iter()
            .filter(|mount| mount.kind == MountKind::Bind)
            .map(|mount| {
                let mut options = vec!["rbind"];
                if mount.read_only {
                    options.push("ro");
                }
                json!({
                    "destination": mount.target,
                    "type": "bind",
                    "source": mount.source,
                    "options": options,
                })
            })
            .collect();
        binds.extend(self.hardening.tmpfs.iter().map(|(target, options)| {
            json!({
                "destination": target,
                "type": "tmpfs",
                "source": "tmpfs",
                "options": options.split(',').filter(|option| !option.is_empty()).collect::<Vec<&str>>(),
            })
        }));
        let ports: Vec<serde_json::Value> = self
            .ports
            .iter()
            .map(|port| {
                let mut mapping = json!({
                    "container_port": port.container_port,
                    "protocol": port.protocol,
                });
                if let Some(host_port) = port.host_port {
                    mapping["host_port"] = json!(host_port);
                }
                if let Some(host_ip) = &port.host_ip {
                    mapping["host_ip"] = json!(host_ip);
                }
                mapping
            })
            .collect();
        let mut body = json!({
            "name": self.name,
            "image": self.image,
            "env": env,
            "labels": labels,
            "secrets": secrets,
            "volumes": volumes,
            "mounts": binds,
            "portmappings": ports,
            "restart_policy": self.restart.name(),
        });
        if let RestartPolicy::OnFailure(retries) = self.restart {
            body["restart_tries"] = json!(retries);
        }
        if !self.cmd.is_empty() {
            body["command"] = json!(self.cmd);
        }
        if !self.entrypoint.is_empty() {
            body["entrypoint"] = json!(self.entrypoint);
        }
        if let Some(user) = &self.user {
            body["user"] = json!(user);
        }
        if let Some(health) = &self.health {
            body["healthconfig"] = health.json();
        }
        let hardening = &self.hardening;
        if hardening.read_only {
            body["read_only_filesystem"] = json!(true);
        }
        if hardening.cap_drop_all {
            body["cap_drop"] = json!(["ALL"]);
        }
        if hardening.no_new_privileges {
            body["no_new_privileges"] = json!(true);
        }
        let mut limits = serde_json::Map::new();
        if let Some(memory) = hardening.memory_bytes {
            limits.insert("memory".into(), json!({ "limit": memory }));
        }
        if let Some(cpus) = hardening.nano_cpus {
            // a quota of the period's microseconds: 100 ms of each 100 ms per CPU
            limits.insert("cpu".into(), json!({ "quota": cpus / 10_000, "period": 100_000 }));
        }
        if let Some(pids) = hardening.pids_limit {
            limits.insert("pids".into(), json!({ "limit": pids }));
        }
        if !limits.is_empty() {
            body["resource_limits"] = serde_json::Value::Object(limits);
        }
        if !hardening.extra_hosts.is_empty() {
            body["hostadd"] = json!(hardening.extra_hosts);
        }
        if let Some(userns) = &hardening.podman_userns {
            let (mode, value) = userns.split_once(':').unwrap_or((userns.as_str(), ""));
            body["userns"] = if value.is_empty() {
                json!({ "nsmode": mode })
            } else {
                json!({ "nsmode": mode, "value": value })
            };
        }
        match self.network.as_deref() {
            Some(network) if network.eq_ignore_ascii_case("host") => body["netns"] = json!({ "nsmode": "host" }),
            Some(network) => {
                body["netns"] = json!({ "nsmode": "bridge" });
                body["Networks"] = json!({ network: {} });
            }
            None => {}
        }
        body
    }
}
