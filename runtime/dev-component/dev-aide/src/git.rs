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

//! The git layer (plans/design-dev-aide.md 4.2, D5): the developer's own `git` (the conf's `git-command`) on working
//! trees, with their config, hooks, signing and credentials. Never through a shell: arguments are argv, paths after `--`,
//! `-c core.quotepath=off -c color.ui=false`, `LC_ALL=C` for parsed output, `GIT_TERMINAL_PROMPT=0` so nothing prompts,
//! and `GIT_OPTIONAL_LOCKS=0` for reads so status polling never takes the index lock from the developer's terminal. One
//! write at a time per repository; reads in parallel; reads time out after 15 s and network commands after 300 s.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use moqui_lib::error::Error;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Read,
    Write,
    Network,
}

#[derive(Clone, Debug)]
pub struct Git {
    pub command: String,
    pub read_timeout: Duration,
    pub network_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// The error of a command that did not succeed: git's own message.
    ///
    /// # Errors
    ///
    /// Returns an error with git's message when the command failed.
    pub fn checked(self, what: &str) -> Result<Self, Error> {
        if self.success() {
            Ok(self)
        } else {
            let message = Some(self.stderr.trim())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| self.stdout.trim());
            Err(Error::new(format!("git {what}: {message}")))
        }
    }
}

fn write_lock(repo: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().unwrap_or_else(|err| err.into_inner());
    Arc::clone(locks.entry(repo.to_path_buf()).or_default())
}

impl Git {
    /// Whether the git command runs at all.
    #[must_use]
    pub fn available(&self) -> bool {
        Command::new(&self.command)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Run `git args` in `repo`, with `env` added and `stdin` written to it.
    ///
    /// # Errors
    ///
    /// Returns an error when git cannot start or runs past its timeout (it is killed).
    pub fn run(
        &self,
        repo: &Path,
        mode: Mode,
        args: &[&str],
        env: &[(&str, String)],
        stdin: Option<&str>,
    ) -> Result<Output, Error> {
        let lock = (mode != Mode::Read).then(|| write_lock(repo));
        let _held = lock
            .as_ref()
            .map(|lock| lock.lock().unwrap_or_else(|err| err.into_inner()));
        let mut command = Command::new(&self.command);
        command
            .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
            .args(args)
            .current_dir(repo)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("SSH_ASKPASS", "")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if mode == Mode::Read {
            command.env("GIT_OPTIONAL_LOCKS", "0");
        }
        for (name, value) in env {
            command.env(name, value);
        }
        let mut child = command
            .spawn()
            .map_err(|err| Error::new(format!("could not run {}: {err}", self.command)))?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let text = text.to_string();
            std::thread::spawn(move || {
                let _ = pipe.write_all(text.as_bytes());
            });
        }
        let reader = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut bytes);
                }
                String::from_utf8_lossy(&bytes).into_owned()
            })
        };
        let stdout = reader(child.stdout.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>));
        let stderr = reader(child.stderr.take().map(|pipe| Box::new(pipe) as Box<dyn Read + Send>));
        // a write may run hooks (pre-commit), so it has longer than a read
        let timeout = match mode {
            Mode::Read => self.read_timeout,
            Mode::Write => Duration::from_secs(120),
            Mode::Network => self.network_timeout,
        };
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|err| Error::new(format!("git: {err}")))? {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::new(format!(
                    "git {} ran past {} s and was stopped",
                    args.first().copied().unwrap_or(""),
                    timeout.as_secs()
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        Ok(Output {
            code: status.code(),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        })
    }

    /// A read whose failure is an error.
    ///
    /// # Errors
    ///
    /// Returns git's message when it fails.
    pub fn read(&self, repo: &Path, args: &[&str]) -> Result<String, Error> {
        Ok(self
            .run(repo, Mode::Read, args, &[], None)?
            .checked(args.first().copied().unwrap_or(""))?
            .stdout)
    }
}

/// One changed path of `git status --porcelain=v2`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    /// The index (staged) and work tree letters, `.` for unchanged; `?` both for untracked, `!` for ignored.
    pub index: char,
    pub worktree: char,
    pub conflicted: bool,
}

impl StatusEntry {
    /// The one letter the tree shows: U for a conflict, ? untracked, ! ignored, else the work tree's change or the
    /// index's (M, A, D, R).
    #[must_use]
    pub fn letter(&self) -> char {
        if self.conflicted {
            return 'U';
        }
        let pick = if self.worktree != '.' {
            self.worktree
        } else {
            self.index
        };
        match pick {
            'T' => 'M',
            'C' => 'A',
            other => other,
        }
    }

    #[must_use]
    pub fn staged(&self) -> bool {
        !self.conflicted && self.index != '.' && self.index != '?' && self.index != '!'
    }

    #[must_use]
    pub fn unstaged(&self) -> bool {
        !self.conflicted && self.worktree != '.' && self.worktree != '?' && self.worktree != '!'
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub oid: Option<String>,
    /// The branch, or None when detached.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<StatusEntry>,
}

/// Parse `git status --porcelain=v2 -z --branch` output.
#[must_use]
pub fn parse_status(out: &str) -> Status {
    let mut status = Status::default();
    let mut records = out.split('\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.oid = Some(value.to_string()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.to_string()),
                "branch.upstream" => status.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for part in value.split_whitespace() {
                        if let Some(ahead) = part.strip_prefix('+') {
                            status.ahead = ahead.parse().unwrap_or(0);
                        } else if let Some(behind) = part.strip_prefix('-') {
                            status.behind = behind.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = record.as_bytes()[0];
        match kind {
            b'1' | b'2' | b'u' => {
                // 1 XY sub mH mI mW hH hI path / 2 XY sub mH mI mW hH hI Xscore path NUL orig / u XY sub m1 m2 m3 mW h1 h2 h3 path
                let fields = match kind {
                    b'1' => 8,
                    b'2' => 9,
                    _ => 10,
                };
                let parts: Vec<&str> = record.splitn(fields + 1, ' ').collect();
                let Some(path) = parts.get(fields) else {
                    continue;
                };
                let xy: Vec<char> = parts.get(1).map_or_else(Vec::new, |xy| xy.chars().collect());
                let original = if kind == b'2' {
                    records.next().map(str::to_string)
                } else {
                    None
                };
                status.entries.push(StatusEntry {
                    path: (*path).to_string(),
                    original,
                    index: xy.first().copied().unwrap_or('.'),
                    worktree: xy.get(1).copied().unwrap_or('.'),
                    conflicted: kind == b'u',
                });
            }
            b'?' | b'!' => {
                let mark = char::from(kind);
                status.entries.push(StatusEntry {
                    path: record[2..].to_string(),
                    original: None,
                    index: mark,
                    worktree: mark,
                    conflicted: false,
                });
            }
            _ => {}
        }
    }
    status
}

/// One commit of a log.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub id: String,
    pub short_id: String,
    pub author_name: String,
    pub author_email: String,
    pub date: String,
    pub subject: String,
}

/// The `--format` that [`parse_log`] reads: fields split by US, commits by RS.
pub const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%an%x1f%ae%x1f%aI%x1f%s%x1e";

#[must_use]
pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\u{1e}')
        .map(|record| record.trim_start_matches(['\n', '\r']))
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let fields: Vec<&str> = record.split('\u{1f}').collect();
            (fields.len() >= 6).then(|| Commit {
                id: fields[0].to_string(),
                short_id: fields[1].to_string(),
                author_name: fields[2].to_string(),
                author_email: fields[3].to_string(),
                date: fields[4].to_string(),
                subject: fields[5].to_string(),
            })
        })
        .collect()
}

/// A local branch with its upstream, from `for-each-ref` with [`BRANCH_FORMAT`].
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub current: bool,
}

pub const BRANCH_FORMAT: &str = "--format=%(refname:short)%1f%(upstream:short)%1f%(HEAD)";

#[must_use]
pub fn parse_branches(out: &str) -> Vec<Branch> {
    out.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\u{1f}').collect();
            (fields.len() >= 3 && !fields[0].is_empty()).then(|| Branch {
                name: fields[0].to_string(),
                upstream: Some(fields[1].to_string()).filter(|upstream| !upstream.is_empty()),
                current: fields[2] == "*",
            })
        })
        .collect()
}

/// `git remote -v` as each remote's name and fetch URL.
#[must_use]
pub fn parse_remotes(out: &str) -> Vec<(String, String)> {
    let mut remotes: Vec<(String, String)> = Vec::new();
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        let (Some(name), Some(url), kind) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if kind != Some("(push)") && !remotes.iter().any(|(have, _)| have == name) {
            remotes.push((name.to_string(), url.to_string()));
        }
    }
    remotes
}

/// A commit message with a task trailer (`Task: PROJ-12`) after a blank line, unless the message has it already.
#[must_use]
pub fn with_trailer(message: &str, task: Option<&str>) -> String {
    let message = message.trim_end();
    match task.map(str::trim).filter(|task| !task.is_empty()) {
        Some(task) if !message.lines().any(|line| line.trim() == format!("Task: {task}")) => {
            format!("{message}\n\nTask: {task}\n")
        }
        _ => format!("{message}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2_with_every_kind_of_entry() {
        let out = "# branch.oid 1234abcd\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
            1 .M N... 100644 100644 100644 aaa aaa service/A.xml\0\
            1 A. N... 000000 100644 100644 000 bbb entity/New Entities.xml\0\
            2 R. N... 100644 100644 100644 ccc ccc R100 screen/B.xml\0screen/Old.xml\0\
            u UU N... 100644 100644 100644 100644 d1 d2 d3 data/Seed.xml\0\
            ? notes/todo.md\0! target/x\0";
        let status = parse_status(out);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        let summary: Vec<(&str, char, bool, bool)> = status
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.letter(), entry.staged(), entry.unstaged()))
            .collect();
        assert_eq!(
            summary,
            [
                ("service/A.xml", 'M', false, true),
                ("entity/New Entities.xml", 'A', true, false),
                ("screen/B.xml", 'R', true, false),
                ("data/Seed.xml", 'U', false, false),
                ("notes/todo.md", '?', false, false),
                ("target/x", '!', false, false),
            ]
        );
        assert_eq!(status.entries[2].original.as_deref(), Some("screen/Old.xml"));
        let detached = parse_status("# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!((detached.oid, detached.branch), (None, None));
    }

    #[test]
    fn log_branches_and_remotes() {
        let log = parse_log(
            "a1\u{1f}a\u{1f}Ann\u{1f}ann@x.com\u{1f}2026-10-24T10:00:00+00:00\u{1f}First, with a comma\u{1e}\n\
            b2\u{1f}b\u{1f}Bo\u{1f}bo@x.com\u{1f}2026-10-23T09:00:00+00:00\u{1f}Second\u{1e}\n",
        );
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "First, with a comma");
        assert_eq!(log[1].author_email, "bo@x.com");
        let branches = parse_branches("main\u{1f}origin/main\u{1f}*\nfeature\u{1f}\u{1f} \n");
        assert!(branches[0].current);
        assert_eq!(branches[1].upstream, None);
        let remotes = parse_remotes(
            "origin\tgit@host:me/site.git (fetch)\norigin\tgit@host:me/site.git (push)\nup\thttps://x/y (fetch)\n",
        );
        assert_eq!(
            remotes,
            [
                ("origin".into(), "git@host:me/site.git".into()),
                ("up".into(), "https://x/y".into())
            ]
        );
    }

    #[test]
    fn a_trailer_once() {
        assert_eq!(
            with_trailer("Add a field", Some("PROJ-12")),
            "Add a field\n\nTask: PROJ-12\n"
        );
        assert_eq!(
            with_trailer("Add\n\nTask: PROJ-12", Some("PROJ-12")),
            "Add\n\nTask: PROJ-12\n"
        );
        assert_eq!(with_trailer("Add  \n", None), "Add\n");
    }
}
