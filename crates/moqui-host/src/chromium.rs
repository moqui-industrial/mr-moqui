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

//! Finding and starting Chromium, for the PDF engine ([`crate::pdf`]) and the browser of `moqui test-client`
//! (`test_client/browser.rs`): the places a binary may be, the launch arguments both use, and the DevTools address the
//! browser writes once it listens. Not vendored: `./moq chromium-download` puts one in `runtime/chromium`.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use moqui_lib::error::Error;

/// A Chromium or Chrome binary found, and where it was found.
struct Found {
    place: String,
    path: PathBuf,
    chrome: bool,
}

/// Every Chromium and Chrome there is, in order: `configured` when it names a file (`configured_name` says where it
/// came from, for the warning when it does not), the one of [`downloaded_chromium`], `chromium`, `chromium-browser`,
/// `google-chrome` and `google-chrome-stable` on the `PATH`, and on macOS those in `/Applications`. One binary under
/// several names (chromium-browser a link to chromium, google-chrome-stable to google-chrome) is there once.
fn found_binaries(runtime_dir: Option<&Path>, configured: &str, configured_name: &str, what: &str) -> Vec<Found> {
    let mut found = Vec::new();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_file() {
            found.push(Found {
                place: configured_name.to_string(),
                path,
                chrome: false,
            });
        } else {
            tracing::warn!("Chromium for {what}: {configured_name} names {configured}, which is not a file");
        }
    }
    if let Some(runtime_dir) = runtime_dir {
        if let Some(path) = downloaded_chromium(runtime_dir) {
            let place = match std::fs::read_to_string(runtime_dir.join("chromium/REVISION")) {
                Ok(text) => format!(
                    "runtime/chromium (snapshot {})",
                    text.split_whitespace().next().unwrap_or("?")
                ),
                Err(_) => "runtime/chromium".to_string(),
            };
            found.push(Found {
                place,
                path,
                chrome: false,
            });
        }
    }
    let search = std::env::var_os("PATH").unwrap_or_default();
    for (name, chrome) in [
        ("chromium", false),
        ("chromium-browser", false),
        ("google-chrome", true),
        ("google-chrome-stable", true),
    ] {
        if let Some(path) = std::env::split_paths(&search)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
        {
            found.push(Found {
                place: format!("{name} on the PATH"),
                path,
                chrome,
            });
        }
    }
    if cfg!(target_os = "macos") {
        for (app, chrome) in [
            ("/Applications/Chromium.app/Contents/MacOS/Chromium", false),
            ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", true),
        ] {
            let path = PathBuf::from(app);
            if path.is_file() {
                found.push(Found {
                    place: "/Applications".to_string(),
                    path,
                    chrome,
                });
            }
        }
    }
    let mut seen = Vec::new();
    found.retain(|one| {
        let real = std::fs::canonicalize(&one.path).unwrap_or_else(|_| one.path.clone());
        let new = !seen.contains(&real);
        seen.push(real);
        new
    });
    found
}

/// Look at every place a Chromium may be, log each one found, and choose one for PDF: `configured` (the `pdf`
/// `chromium-path`, `pdf_chromium_path`) when it names a file,
/// else the first Chromium of [`downloaded_chromium`], `chromium` then `chromium-browser` on the `PATH`, and on macOS
/// `/Applications/Chromium.app`. Google Chrome is logged when it is found but is used only when `pdf_chromium_path`
/// names it.
///
/// # Errors
///
/// Returns the reason PDF is not available, already logged, when no Chromium is chosen.
pub fn find_chromium(runtime_dir: &Path, configured: &str) -> Result<PathBuf, String> {
    let configured = configured.trim();
    let found = found_binaries(Some(runtime_dir), configured, "pdf_chromium_path", "PDF");
    for one in &found {
        let note = if one.chrome {
            ", Google Chrome: used only when pdf_chromium_path names it"
        } else {
            ""
        };
        tracing::info!("Chromium for PDF: found {} at {}{note}", one.place, one.path.display());
    }
    if let Some(chosen) = found.iter().find(|one| !one.chrome) {
        tracing::info!(
            "Chromium for PDF: using {} from {}",
            chosen.path.display(),
            chosen.place
        );
        if chosen.place.starts_with("runtime/chromium") {
            check_apparmor_profile(&chosen.path);
        }
        return Ok(chosen.path.clone());
    }
    let mut reason = if configured.is_empty() {
        moqui_core::screen::PDF_UNAVAILABLE.to_string()
    } else {
        format!("PDF is not available: pdf_chromium_path names {configured}, which is not a file, and no other Chromium was found")
    };
    if let Some(chrome) = found.first() {
        reason.push_str(&format!(
            "; Google Chrome at {} is used when pdf_chromium_path names it",
            chrome.path.display()
        ));
    }
    tracing::warn!("Chromium for PDF: {reason}");
    Err(reason)
}

/// The browser of `moqui test-client`: `configured` (`--chromium`, else `MOQUI_CHROMIUM`) when it names a file, else
/// the one in `runtime/chromium` (with a runtime), `chromium` and `chromium-browser` on the `PATH`, then Google Chrome,
/// which the browser tests may use as they always have.
///
/// # Errors
///
/// Returns why there is none, naming the places looked at.
pub fn find_test_browser(runtime_dir: Option<&Path>, configured: &str) -> Result<PathBuf, String> {
    let configured = configured.trim();
    if !configured.is_empty() && !Path::new(configured).is_file() {
        return Err(format!(
            "--chromium (or MOQUI_CHROMIUM) names {configured}, which is not a file"
        ));
    }
    let found = found_binaries(runtime_dir, configured, "--chromium", "the browser tests");
    let chosen = found.iter().find(|one| !one.chrome).or_else(|| found.first());
    match chosen {
        Some(one) => {
            tracing::debug!(
                "Chromium for the browser tests: {} from {}",
                one.path.display(),
                one.place
            );
            if one.place.starts_with("runtime/chromium") {
                check_apparmor_profile(&one.path);
            }
            Ok(one.path.clone())
        }
        None => Err(
            "no Chromium: pass --chromium or set MOQUI_CHROMIUM, run ./moq chromium-download, or put chromium, \
             chromium-browser or google-chrome on the PATH"
                .to_string(),
        ),
    }
}

/// The Chromium that `./moq chromium-download` put in `runtime/chromium`.
#[must_use]
pub fn downloaded_chromium(runtime_dir: &Path) -> Option<PathBuf> {
    ["chrome", "Chromium.app/Contents/MacOS/Chromium"]
        .iter()
        .map(|name| runtime_dir.join("chromium").join(name))
        .find(|candidate| candidate.is_file())
}

/// Whether AppArmor limits unprivileged user namespaces (Ubuntu 23.10 and later), which keeps a Chromium that has no
/// AppArmor profile from starting its sandbox.
#[must_use]
pub fn userns_restricted() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
        .is_ok_and(|text| text.trim() == "1")
}

/// Warn when the downloaded Chromium needs the `moqui-chromium` AppArmor profile that `./moq chromium-download`
/// describes and it is not there. Distro and Chrome packages install their own profiles.
fn check_apparmor_profile(binary: &Path) {
    if !userns_restricted() {
        return;
    }
    let real = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let named = std::fs::read_to_string("/etc/apparmor.d/moqui-chromium")
        .is_ok_and(|text| text.contains(&format!("\"{}\"", real.display())));
    if !named {
        tracing::warn!(
            "Chromium: AppArmor limits unprivileged user namespaces here and no moqui-chromium profile names {}, \
             so it cannot start its sandbox and PDF and the browser tests will fail; ./moq help chromium-download shows \
             how to add one",
            real.display()
        );
    }
}

/// The browser's arguments, apart from the profile and the window: headless, DevTools on an ephemeral loopback port,
/// and nothing that phones home. The sandbox stays on. The PDF engine adds its proxy arguments, which keep a document
/// off the network.
#[must_use]
pub fn launch_args() -> [&'static str; 13] {
    [
        "--headless=new",
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-gpu",
        "--disable-extensions",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-sync",
        "--metrics-recording-only",
        "--password-store=basic",
        "--use-mock-keychain",
    ]
}

/// The browser socket from `DevToolsActivePort` in the profile, which Chromium writes once it listens.
///
/// # Errors
///
/// Returns an error when the browser exits first or does not listen within `wait`.
pub fn devtools_url(profile: &Path, child: &mut Child, wait: Duration) -> Result<String, Error> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + wait;
    loop {
        if let Ok(text) = std::fs::read_to_string(&file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                return Ok(format!("ws://127.0.0.1:{}{}", port.trim(), path.trim()));
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(Error::new(format!("Chromium exited before it listened: {status}")));
        }
        if Instant::now() >= deadline {
            return Err(Error::new(format!("Chromium did not write {}", file.display())));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_browser_that_is_no_file_is_an_error_and_one_that_is_comes_first() {
        let err = find_test_browser(None, "/nowhere/chromium").unwrap_err();
        assert!(err.contains("names /nowhere/chromium, which is not a file"), "{err}");
        let dir = std::env::temp_dir().join(format!("moqui-chromium-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("my-chromium");
        std::fs::write(&fake, "").unwrap();
        assert_eq!(find_test_browser(Some(&dir), fake.to_str().unwrap()).unwrap(), fake);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
