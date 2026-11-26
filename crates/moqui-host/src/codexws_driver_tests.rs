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

use moqui_lib::docker::fake::FakeEngine;

use super::{sandbox_spec, start_on};
use crate::codexws::GitAccess;

const MCP_KEY: &str = "mcp-key-of-the-user-0123456789abcdef";
const GIT_KEY: &str = "git-key-of-the-user-0123456789abcdef";

fn runtime(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("moqui-codexws-driver-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("components")).unwrap();
    dir
}

fn git() -> GitAccess {
    GitAccess {
        url: "https://host.docker.internal:8443/git".to_string(),
        key: GIT_KEY.to_string(),
        username: "john.doe".to_string(),
        name: "John Doe".to_string(),
        email: "john@example.com".to_string(),
    }
}

#[test]
fn the_sandbox_is_hardened_and_its_keys_are_files() {
    let dir = runtime("spec");
    let mcp = Some(("https://host.docker.internal:8443/mcp".to_string(), MCP_KEY.to_string()));
    let spec = sandbox_spec(&dir, "john", &"ab".repeat(32), mcp, Some(git())).unwrap();
    assert_eq!(spec.name, "moqui-workbench-john");
    let hardening = &spec.hardening;
    assert!(hardening.read_only && hardening.cap_drop_all && hardening.no_new_privileges);
    assert_eq!(hardening.memory_bytes, Some(4096 << 20));
    assert_eq!(hardening.nano_cpus, Some(2_000_000_000));
    assert_eq!(hardening.pids_limit, Some(1024));
    assert_eq!(hardening.extra_hosts, ["host.docker.internal:host-gateway"]);
    let secrets: Vec<&str> = spec.secrets.iter().map(|secret| secret.name.as_str()).collect();
    assert_eq!(secrets, ["moqui-mcp-token", "moqui-git-token"]);
    // no key in the environment, the labels or the command
    let settings = format!("{:?} {:?} {:?}", spec.env, spec.labels, spec.cmd);
    assert!(!settings.contains(MCP_KEY) && !settings.contains(GIT_KEY), "{settings}");
    assert!(spec.env.contains(&("MOQUI_GIT_USER".to_string(), "john.doe".to_string())));
    assert_eq!(spec.ports[0].host_ip.as_deref(), Some("127.0.0.1"));
    assert_eq!(spec.ports[0].host_port, None, "the engine picks it");
    let targets: Vec<(&str, bool)> = spec.mounts.iter().map(|mount| (mount.target.as_str(), mount.read_only)).collect();
    assert_eq!(targets, [("/home/node/.codex", false), ("/workspace", false), ("/moqui/components", true)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_driver_starts_the_sandbox_and_writes_where_it_listens() {
    let dir = runtime("start");
    let engine = FakeEngine::docker();
    let host = engine.host();
    let mcp = Some(("https://host.docker.internal:8443/mcp".to_string(), MCP_KEY.to_string()));
    let sandbox = start_on(&host, &dir, "john", mcp.clone(), Some(git())).unwrap();
    assert_eq!(sandbox.url, "ws://127.0.0.1:32768");
    assert_eq!(sandbox.token.len(), 64);
    let state = dir.join("workbench/john");
    assert_eq!(std::fs::read_to_string(state.join("url")).unwrap(), sandbox.url);
    assert_eq!(std::fs::read_to_string(state.join("token")).unwrap(), sandbox.token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(state.join("token")).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(engine.secret_outside_files(MCP_KEY.as_bytes()).is_empty());
    assert!(engine.secret_outside_files(GIT_KEY.as_bytes()).is_empty());
    assert!(engine.secret_outside_files(sandbox.token.as_bytes()).is_empty(), "only the token's hash goes in");
    let containers = engine.containers();
    assert_eq!(containers.len(), 1);
    let (id, container) = containers.iter().next().unwrap();
    assert_eq!(container.status, "running");
    let files: Vec<String> = engine.files(id).into_iter().map(|(_, file)| file.path).collect();
    assert_eq!(files, ["run/secrets/moqui-mcp-token", "run/secrets/moqui-git-token"]);
    // a start again makes it again, with new keys and a new token
    let again = start_on(&host, &dir, "john", mcp, None).unwrap();
    assert_ne!(again.token, sandbox.token);
    let containers = engine.containers();
    assert_eq!(containers.len(), 1, "the first is removed");
    assert!(!containers.contains_key(id));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The real sandbox on this machine's engine: started by the driver from the built image, its app-server answers the
/// bridge's `initialize`, and git in it signs in with the key's file. Opt-in: `MOQUI_WORKBENCH_REAL_SOCKET` (the engine's
/// socket) and the image built (`scripts/sandbox.sh build`); nothing runs without them.
#[test]
fn the_real_sandbox_starts_and_answers() {
    let Some(socket) = std::env::var("MOQUI_WORKBENCH_REAL_SOCKET").ok().filter(|socket| !socket.is_empty()) else {
        eprintln!("MOQUI_WORKBENCH_REAL_SOCKET is not set: nothing run");
        return;
    };
    let host = moqui_lib::docker::DockerHost::unix(socket.trim_start_matches("unix://"));
    let dir = runtime("real");
    let sandbox = start_on(&host, &dir, "wbprobe", None, Some(git())).unwrap();
    let name = "moqui-workbench-wbprobe";
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let answer = runtime.block_on(async {
            let mut last = String::new();
            for _ in 0..40 {
                match crate::codexws::connect(&sandbox).await {
                    Ok((_, answer)) => return Ok(answer),
                    Err(why) => last = why,
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            Err(last)
        });
        let answer = answer.unwrap();
        assert!(answer.get("userAgent").is_some(), "the app-server's initialize: {answer}");
        // git asks its credential helper, which reads the key's file
        let out = std::process::Command::new("docker")
            .args(["exec", name, "sh", "-c", "printf 'protocol=https\\nhost=host.docker.internal:8443\\npath=git/x.git\\n\\n' | GIT_CONFIG_GLOBAL=/tmp/gitconfig git credential fill"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(&format!("password={GIT_KEY}")) && text.contains("username=john.doe"), "{text}");
        let env = std::process::Command::new("docker").args(["exec", name, "env"]).output().unwrap();
        assert!(!String::from_utf8_lossy(&env.stdout).contains(GIT_KEY), "the key is not in the environment");
    }));
    if let Some(info) = host.inspect(name).unwrap() {
        host.remove_container(&info.id, true).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure);
    }
}
