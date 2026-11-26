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

//! The Workbench's sandbox started through the container driver (`moqui_lib::docker`), with
//! `moqui-agent-workbench@sandbox` `driver` (moqui-agent-workbench, plan/design-workbench.md 9.4): what the component's
//! `scripts/sandbox.sh start` does, on Docker's or Podman's API. One container per user (`moqui-workbench-<userId>`):
//! a read-only root with a tmpfs `/tmp`, no capability, `no-new-privileges`, memory, CPU and process limits, its
//! app-server port on 127.0.0.1 alone (a port the engine picks), the user's volume for Codex's home, the workspace of
//! the runtime, Moqui's components, docs and skills read only. The MCP and git keys are secret files under
//! `/run/secrets` (never in the environment, labels or command); the capability token stays in the runtime
//! (`workbench/<userId>/token`), only its SHA-256 goes into the container.

use std::path::{Path, PathBuf};
use std::time::Duration;

use moqui_core::context::conf_env::setting;
use moqui_lib::docker::{ContainerSpec, DockerHost, Hardening, Mount, MountKind, PortBinding, SecretFile};
use sha2::Digest;
use zeroize::Zeroizing;

use super::{GitAccess, Sandbox};

/// The user the image runs as (the node image's), who owns the secret files and the files mounted for it.
const IMAGE_UID: u32 = 1000;
/// The app-server's port in the container.
const APP_PORT: u16 = 4500;
/// How long the engine has to publish the port of a container just started.
const PORT_WAIT: Duration = Duration::from_secs(10);

fn workbench_setting(name: &str) -> String {
    setting(&format!("moqui-agent-workbench@{name}"))
}

/// The engine of this machine: `engine-socket`, else rootless Podman's socket of this user, else Docker's.
fn engine() -> Result<DockerHost, String> {
    let configured = workbench_setting("engine-socket");
    let mut candidates: Vec<PathBuf> = Vec::new();
    if configured.is_empty() {
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            candidates.push(PathBuf::from(runtime).join("podman/podman.sock"));
        }
        candidates.push(PathBuf::from("/run/podman/podman.sock"));
        candidates.push(PathBuf::from("/var/run/docker.sock"));
    } else {
        candidates.push(PathBuf::from(configured.trim_start_matches("unix://")));
    }
    candidates
        .into_iter()
        .find(|socket| socket.exists())
        .map(DockerHost::unix)
        .ok_or_else(|| "no container engine answers on this machine (set moqui-agent-workbench@engine-socket)".to_string())
}

/// A number setting, or its default.
fn number(name: &str, default: i64) -> i64 {
    workbench_setting(name).parse().ok().filter(|value| *value > 0).unwrap_or(default)
}

/// A directory to mount read only, when it is there.
fn existing_dir(path: &Path) -> Option<String> {
    path.canonicalize()
        .ok()
        .filter(|path| path.is_dir())
        .map(|path| path.to_string_lossy().into_owned())
}

/// A new capability token for the app-server, and its SHA-256 for the container.
fn new_token() -> Result<(String, String), String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    aws_lc_rs::rand::fill(bytes.as_mut()).map_err(|_| "no random bytes for the sandbox's token".to_string())?;
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let sha: String = sha2::Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((token, sha))
}

/// Write `text` to `path`, readable by this process's user alone.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| format!("{} was not written: {err}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|err| format!("{} was not written: {err}", path.display()))
}

/// The container of `user_id`'s sandbox, as the driver makes it.
pub(super) fn sandbox_spec(
    runtime: &Path,
    user_id: &str,
    token_sha: &str,
    mcp: Option<(String, String)>,
    git: Option<GitAccess>,
) -> Result<ContainerSpec, String> {
    let name = format!("moqui-workbench-{user_id}");
    let state = Sandbox::dir(runtime, user_id);
    let workspace = state.join("workspace");
    std::fs::create_dir_all(&workspace).map_err(|err| format!("the workspace was not made: {err}"))?;
    let bind = |source: String, target: &str, read_only: bool| Mount {
        kind: MountKind::Bind,
        source,
        target: target.to_string(),
        read_only,
    };
    let mut mounts = vec![
        Mount {
            kind: MountKind::Volume,
            source: format!("{name}-home"),
            target: "/home/node/.codex".to_string(),
            read_only: false,
        },
        bind(
            existing_dir(&workspace).ok_or_else(|| "the workspace is not a directory".to_string())?,
            "/workspace",
            false,
        ),
    ];
    if let Some(components) = existing_dir(&runtime.join("components")) {
        mounts.push(bind(components, "/moqui/components", true));
    }
    let docs = Some(workbench_setting("docs-dir"))
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| runtime.join("../docs"), PathBuf::from);
    if let Some(docs) = existing_dir(&docs) {
        mounts.push(bind(docs, "/moqui/docs", true));
    }
    if let Some(skills) = Some(workbench_setting("skills-dir"))
        .filter(|dir| !dir.is_empty())
        .and_then(|dir| existing_dir(Path::new(&dir)))
    {
        mounts.push(bind(skills, "/etc/codex/skills", true));
    }
    let mut env = vec![("CODEX_WS_TOKEN_SHA256".to_string(), token_sha.to_string())];
    let mut secrets = Vec::new();
    let reaches_moqui = mcp.is_some() || git.is_some();
    if let Some((url, key)) = mcp {
        env.push(("MOQUI_MCP_URL".to_string(), url));
        secrets.push(SecretFile::new("moqui-mcp-token", Zeroizing::new(key.into_bytes()), IMAGE_UID, IMAGE_UID));
    }
    if let Some(git) = git {
        env.push(("MOQUI_GIT_URL".to_string(), git.url));
        env.push(("MOQUI_GIT_USER".to_string(), git.username));
        env.push(("MOQUI_GIT_NAME".to_string(), git.name));
        env.push(("MOQUI_GIT_EMAIL".to_string(), git.email));
        secrets.push(SecretFile::new("moqui-git-token", Zeroizing::new(git.key.into_bytes()), IMAGE_UID, IMAGE_UID));
    }
    let mut extra_hosts = Vec::new();
    if reaches_moqui {
        extra_hosts.push("host.docker.internal:host-gateway".to_string());
        // the CA of Moqui's certificate when it is not a public one: ca-file, else dev-cert.sh's
        let ca = Some(workbench_setting("ca-file"))
            .filter(|file| !file.is_empty())
            .map_or_else(|| runtime.join("workbench/ca.crt"), PathBuf::from);
        if let Some(ca) = ca.canonicalize().ok().filter(|path| path.is_file()) {
            mounts.push(bind(ca.to_string_lossy().into_owned(), "/etc/workbench/moqui-ca.crt", true));
            env.push(("MOQUI_CA_CERT".to_string(), "/etc/workbench/moqui-ca.crt".to_string()));
        }
    }
    Ok(ContainerSpec {
        name,
        image: Some(workbench_setting("sandbox-image"))
            .filter(|image| !image.is_empty())
            .unwrap_or_else(|| "moqui-workbench-sandbox:0.169.0".to_string()),
        env,
        labels: vec![("moqui.workbench.user".to_string(), user_id.to_string())],
        mounts,
        ports: vec![PortBinding {
            container_port: APP_PORT,
            protocol: "tcp".to_string(),
            host_port: None,
            host_ip: Some("127.0.0.1".to_string()),
        }],
        secrets,
        hardening: Hardening {
            read_only: true,
            tmpfs: vec![("/tmp".to_string(), "rw,nosuid,nodev,size=1g".to_string())],
            cap_drop_all: true,
            no_new_privileges: true,
            memory_bytes: Some(number("memory-mb", 4096) << 20),
            nano_cpus: Some(number("cpus", 2) * 1_000_000_000),
            pids_limit: Some(number("pids", 1024)),
            extra_hosts,
            podman_userns: Some(format!("keep-id:uid={IMAGE_UID},gid={IMAGE_UID}")),
        },
        ..ContainerSpec::default()
    })
}

/// Start `user_id`'s sandbox through the engine: a container that runs already is made again (its keys are new at each
/// start), the token written, the port the engine picked read back into `workbench/<userId>/url`.
pub(super) fn start(
    runtime: &Path,
    user_id: &str,
    mcp: Option<(String, String)>,
    git: Option<GitAccess>,
) -> Result<Sandbox, String> {
    let host = engine()?;
    start_on(&host, runtime, user_id, mcp, git)
}

pub(super) fn start_on(
    host: &DockerHost,
    runtime: &Path,
    user_id: &str,
    mcp: Option<(String, String)>,
    git: Option<GitAccess>,
) -> Result<Sandbox, String> {
    let state = Sandbox::dir(runtime, user_id);
    std::fs::create_dir_all(&state).map_err(|err| format!("{} was not made: {err}", state.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700));
    }
    let (token, sha) = new_token()?;
    let spec = sandbox_spec(runtime, user_id, &sha, mcp, git)?;
    if let Some(info) = host.inspect(&spec.name).map_err(|err| err.to_string())? {
        host.remove_container(&info.id, true).map_err(|err| err.to_string())?;
    }
    let id = host.create_spec(&spec).map_err(|err| err.to_string())?;
    let started = host.start(&id).map_err(|err| err.to_string()).and_then(|()| {
        let waited = std::time::Instant::now();
        loop {
            if let Some(port) = host
                .published_port(&id, &format!("{APP_PORT}/tcp"), Some("127.0.0.1"))
                .map_err(|err| err.to_string())?
            {
                return Ok(port);
            }
            if waited.elapsed() > PORT_WAIT {
                return Err("the sandbox published no port".to_string());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    let port = match started {
        Ok(port) => port,
        Err(why) => {
            if let Err(err) = host.remove_container(&id, true) {
                tracing::error!(user_id, "the sandbox that did not start was not removed: {err}");
            }
            return Err(why);
        }
    };
    write_private(&state.join("token"), &token)?;
    let url = format!("ws://127.0.0.1:{port}");
    write_private(&state.join("url"), &url)?;
    Ok(Sandbox { url, token })
}

#[cfg(test)]
#[path = "codexws_driver_tests.rs"]
mod tests;
