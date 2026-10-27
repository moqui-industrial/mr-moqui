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

//! Detach `moqui serve` from the terminal.
//!
//! The process the user launched spawns this binary again with the same arguments and
//! [`DETACHED`] set, then waits for one status line. The child serves. A restart execs the same
//! arguments and inherits [`DETACHED`], so it stays this process and does not spawn again.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use moqui_lib::error::Error;

/// Set on the detached child. Presence means this process is already the server.
pub(crate) const DETACHED: &str = "MOQUI_DETACHED";

/// This process was spawned by [`spawn_and_wait`], or exec'd from that child.
pub(crate) fn already_detached() -> bool {
    std::env::var_os(DETACHED).is_some_and(|value| !value.is_empty())
}

/// Spawn the server and return when it prints its listening line, or when it fails.
///
/// On success the child keeps running and this process is the one that exits. The working
/// directory is unchanged.
///
/// # Errors
///
/// Returns an error when this binary cannot be spawned, or when the child reports a failure
/// instead of a listening line.
pub(crate) fn spawn_and_wait() -> Result<(), Error> {
    // Leave the child running. Dropping `Child` does not stop it, and this process exits next.
    let _running = finish_detach(spawn_child()?)?;
    Ok(())
}

fn spawn_child() -> Result<Child, Error> {
    let exe = std::env::current_exe()
        .map_err(|err| Error::new("could not find this binary to start it in the background").with_source(err))?;
    let mut command = Command::new(exe);
    command.args(std::env::args_os().skip(1));
    command.env(DETACHED, "1");
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // SAFETY: `setsid` is async-signal-safe. `pre_exec` runs in the forked child before exec,
        // and the closure calls nothing else.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // DETACHED_PROCESS, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW.
        command.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    command
        .spawn()
        .map_err(|err| Error::new("could not start the server in the background").with_source(err))
}

/// Read the child's one status line. A listening line leaves `child` running. Anything else waits
/// for the child and returns that text.
///
/// # Errors
///
/// Returns an error when the status pipe cannot be read, or when the child does not report that it
/// is listening.
pub(crate) fn finish_detach(mut child: Child) -> Result<Child, Error> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("moqui serve: no status pipe"))?;
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .map_err(|err| Error::new("moqui serve: could not read the server status").with_source(err))?;
    let line = line.trim_end_matches(['\r', '\n']).trim();
    if let Some(rest) = line.strip_prefix("Listening on ") {
        if !rest.is_empty() {
            println!("Listening on {rest}");
            return Ok(child);
        }
    }
    let status = child
        .wait()
        .map_err(|err| Error::new("moqui serve: could not wait for the server").with_source(err))?;
    let detail = if line.is_empty() {
        format!("moqui serve: the server stopped before it was listening ({status})")
    } else {
        line.to_string()
    };
    Err(Error::new(detail))
}

/// Point stdin, stdout, and stderr at `/dev/null` so the status pipe closes and the process holds
/// no terminal. Unix only: on Windows the status line already ended, and the parent has the text.
pub(crate) fn silence_stdio() {
    #[cfg(unix)]
    {
        let Ok(devnull) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/null") else {
            return;
        };
        let fd = std::os::unix::io::AsRawFd::as_raw_fd(&devnull);
        // SAFETY: `dup2` is async-signal-safe. The descriptors are this process's standard streams
        // and a `/dev/null` fd that stays open until the copies are in place.
        unsafe {
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::finish_detach;
    use std::process::{Command, Stdio};

    fn sh(script: &str) -> std::process::Child {
        Command::new("sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh")
    }

    #[test]
    fn a_listening_line_leaves_the_child_running() {
        let mut child = finish_detach(sh("echo 'Listening on 127.0.0.1:9 (pid 1)'; exec sleep 30")).expect("listening");
        let pid = child.id();
        assert!(child.try_wait().expect("child status").is_none(), "child {pid} exited");
        let pid = i32::try_from(pid).expect("pid");
        // SAFETY: `kill` with SIGTERM signals that child and does not share memory with it.
        unsafe { libc::kill(pid, libc::SIGTERM) };
        let _ = child.wait();
    }

    #[test]
    fn a_failed_child_reports_its_line() {
        let err = finish_detach(sh("echo 'could not bind the port'; exit 1")).expect_err("failure");
        assert!(err.to_string().contains("could not bind the port"), "{err}");
    }

    #[test]
    fn a_child_that_exits_quietly_names_the_stop() {
        let err = finish_detach(sh("exit 1")).expect_err("quiet exit");
        assert!(err.to_string().contains("stopped before it was listening"), "{err}");
    }
}
