use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=MOQUI_BUILD_REVISION");
    let revision = std::env::var("MOQUI_BUILD_REVISION")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=MOQUI_GIT_REVISION={}", revision.trim());
    if let Ok(output) = Command::new("git").args(["rev-parse", "--git-path", "HEAD"]).output() {
        if output.status.success() {
            let head = String::from_utf8_lossy(&output.stdout);
            println!("cargo:rerun-if-changed={}", head.trim());
        }
    }
    if let Ok(output) = Command::new("git").args(["rev-parse", "--git-path", "refs"]).output() {
        if output.status.success() {
            println!(
                "cargo:rerun-if-changed={}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
    }
}
