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

//! `moqui check` and `moqui refs` (plans/design-dev-aide.md 13.6, F13): the checks of `moqui_core::devtools::check` and
//! the references of `moqui_core::devtools::refs` from the command line, on a model-only boot as `moqui types` boots,
//! for a developer, a git hook or a coding agent outside the server.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use moqui_core::context::ExecutionContextFactory;
use moqui_core::devtools::refs::{self, RefKind, Reference};
use moqui_core::devtools::{check, Problem, Severity};
use moqui_lib::error::Error;

struct CheckArgs {
    paths: Vec<PathBuf>,
    json: bool,
    extensions: bool,
}

fn parse_check(args: &[String]) -> Result<CheckArgs, Error> {
    let mut parsed = CheckArgs {
        paths: Vec::new(),
        json: false,
        extensions: false,
    };
    for arg in args {
        match arg.as_str() {
            "--json" => parsed.json = true,
            "--extensions" => parsed.extensions = true,
            "--server" => {
                return Err(Error::new(
                    "moqui check --server is not built yet; run moqui check where the files are",
                ))
            }
            other if other.starts_with('-') => return Err(crate::usage::command_usage("check")),
            other => parsed.paths.push(PathBuf::from(other)),
        }
    }
    Ok(parsed)
}

/// `moqui check [paths] [--extensions] [--json]`: the problems of each file, as `path:line:col: severity: message [rule,
/// skill#section]` or one JSON object per line; with no paths, the XML files git shows as changed under the current
/// directory. Exit status 1 when there is an error, 0 otherwise (warnings do not fail it).
pub(crate) fn dispatch_check(args: &[String]) -> Result<(), Error> {
    let args = parse_check(args)?;
    crate::logging::install_with_console(Some(tracing::Level::ERROR));
    let paths = if args.paths.is_empty() && !args.extensions {
        changed_files()?
    } else {
        args.paths.clone()
    };
    let factory = model_only()?;
    let mut problems: Vec<Problem> = Vec::new();
    for path in &paths {
        if path.is_dir() {
            let mut files = Vec::new();
            collect_xml(path, &mut files);
            for file in files {
                problems.extend(check::check_file(&factory, &file));
            }
        } else {
            problems.extend(check::check_file(&factory, path));
        }
    }
    if args.extensions {
        problems.extend(check::check_extensions(&factory));
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    for problem in &problems {
        if args.json {
            println!("{}", serde_json::to_string(problem).unwrap_or_default());
        } else {
            println!("{}", line_of(problem, &cwd));
        }
    }
    let errors = problems
        .iter()
        .filter(|problem| problem.severity == Severity::Error)
        .count();
    if !args.json {
        let files = if args.paths.is_empty() && !args.extensions {
            format!(
                "{} changed file{}",
                paths.len(),
                if paths.len() == 1 { "" } else { "s" }
            )
        } else {
            format!("{} path{}", paths.len(), if paths.len() == 1 { "" } else { "s" })
        };
        eprintln!(
            "moqui check: {files}, {errors} error{}, {} warning{}",
            if errors == 1 { "" } else { "s" },
            problems.len() - errors,
            if problems.len() - errors == 1 { "" } else { "s" }
        );
    }
    if errors > 0 {
        return Err(Error::new(format!(
            "moqui check found {errors} error{}",
            if errors == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

fn line_of(problem: &Problem, cwd: &Path) -> String {
    let path = Path::new(&problem.path);
    let shown = path.strip_prefix(cwd).unwrap_or(path).display();
    let severity = match problem.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    };
    let skill = problem
        .skill
        .as_deref()
        .map(|skill| format!(", {skill}"))
        .unwrap_or_default();
    format!(
        "{shown}:{}:{}: {severity}: {} [{}{skill}]",
        problem.line, problem.column, problem.message, problem.rule
    )
}

/// The XML files git shows as changed (modified, added, renamed, untracked) under the current directory.
fn changed_files() -> Result<Vec<PathBuf>, Error> {
    let top = git(&["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(top.trim());
    let status = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all", "--", "."])?;
    let mut paths = Vec::new();
    let mut entries = status.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        // A rename lists the old path next; it is gone.
        if code.starts_with('R') || code.starts_with('C') {
            entries.next();
        }
        if code.contains('D') {
            continue;
        }
        let path = top.join(path);
        if path.extension().is_some_and(|ext| ext == "xml") && path.is_file() {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// Run `git` with `args` for its output, as the dev tools run it: argv, no prompt, no optional locks.
fn git(args: &[&str]) -> Result<String, Error> {
    let output = std::process::Command::new("git")
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|err| Error::new("moqui check with no paths runs git, which could not start").with_source(err))?;
    if !output.status.success() {
        return Err(Error::new(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn collect_xml(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        // The XSDs' own catalog is not a Moqui file.
        let skipped = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.') || matches!(name, "target" | "node_modules" | "xsd"));
        if skipped {
            continue;
        }
        if path.is_dir() {
            collect_xml(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "xml") {
            into.push(path);
        }
    }
}

/// A factory booted on the runtime with an entity facade of the framework's and the components' entities that opens no
/// database, as `moqui types` boots.
fn model_only() -> Result<Arc<ExecutionContextFactory>, Error> {
    let (runtime, framework) = crate::runtime_framework_dirs();
    let factory = ExecutionContextFactory::boot_with_framework(&runtime, &framework)
        .map_err(|err| Error::new(err.to_string()))?;
    let pool = moqui_core::transaction::PoolConfig {
        min: 0,
        max: 1,
        reserve: 0,
        acquire_timeout: std::time::Duration::from_millis(10),
        ..Default::default()
    };
    let facade = crate::db::entity_facade::open_entity_facade_for_factory(
        &factory,
        Arc::new(crate::qvt_report::ModelOnly),
        pool,
        "postgres",
    )?;
    facade.set_model_only();
    crate::db::entity_facade::attach(&factory, facade);
    Ok(factory)
}

/// `moqui refs <name> [--in | --out] [--kind entity|service|field|parameter|location]`: what uses an artifact (`--in`,
/// the default) or what it uses (`--out`), from the references in every component's entity, service, screen and
/// screen-extend files.
pub(crate) fn dispatch_refs(args: &[String]) -> Result<(), Error> {
    let mut name = None;
    let mut outgoing = false;
    let mut kind = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--in" => outgoing = false,
            "--out" => outgoing = true,
            "--kind" => kind = rest.next().cloned(),
            other if other.starts_with("--kind=") => kind = Some(other["--kind=".len()..].to_string()),
            other if other.starts_with('-') => return Err(crate::usage::command_usage("refs")),
            other if name.is_none() => name = Some(other.to_string()),
            _ => return Err(crate::usage::command_usage("refs")),
        }
    }
    let name = name.ok_or_else(|| crate::usage::command_usage("refs"))?;
    let kind = kind.map(|kind| kind_named(&kind)).transpose()?;
    crate::logging::install_with_console(Some(tracing::Level::ERROR));
    let factory = model_only()?;
    let mut found: Vec<Reference> = Vec::new();
    for file in refs::component_xml_files(&factory) {
        found.extend(refs::scan_file(&file));
    }
    let target = Target::of(&factory, &name);
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut count = 0;
    for reference in found {
        if kind.is_some_and(|kind| kind != reference.kind) {
            continue;
        }
        let matches = if outgoing {
            target.is_from(&reference)
        } else {
            target.is_named_by(&factory, &reference)
        };
        if !matches {
            continue;
        }
        count += 1;
        let path = Path::new(&reference.path);
        println!(
            "{}:{}: {} {}@{} {} (in {})",
            path.strip_prefix(&cwd).unwrap_or(path).display(),
            reference.line,
            kind_name(reference.kind),
            reference.element,
            reference.attribute,
            reference.value,
            reference.from
        );
    }
    eprintln!(
        "moqui refs: {count} reference{} {} {name}",
        if count == 1 { "" } else { "s" },
        if outgoing { "from" } else { "to" }
    );
    Ok(())
}

fn kind_named(name: &str) -> Result<RefKind, Error> {
    Ok(match name {
        "entity" => RefKind::Entity,
        "service" => RefKind::Service,
        "field" => RefKind::FindField,
        "parameter" => RefKind::ServiceParameter,
        "location" => RefKind::Location,
        other => {
            return Err(Error::new(format!(
                "--kind {other}: one of entity, service, field, parameter, location"
            )))
        }
    })
}

fn kind_name(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Entity => "entity",
        RefKind::Service => "service",
        RefKind::FindField => "field",
        RefKind::ServiceParameter => "parameter",
        RefKind::Location => "location",
    }
}

/// What `moqui refs` was asked about: a service (a name with `#`), a file or location, or an entity.
enum Target {
    Service(String),
    Location(String),
    Entity(String),
}

impl Target {
    fn of(factory: &ExecutionContextFactory, name: &str) -> Self {
        if name.contains('#') {
            return Self::Service(name.to_string());
        }
        if name.contains("://") || name.ends_with(".xml") || Path::new(name).exists() {
            let path = std::fs::canonicalize(name).map_or_else(|_| name.to_string(), |path| path.display().to_string());
            return Self::Location(path);
        }
        Self::Entity(full_entity_name(factory, name).unwrap_or_else(|| name.to_string()))
    }

    /// The reference names this target.
    fn is_named_by(&self, factory: &ExecutionContextFactory, reference: &Reference) -> bool {
        match (self, reference.kind) {
            (Self::Service(name), RefKind::Service) => {
                reference.value == *name || reference.value.replace('#', "") == name.replace('#', "")
            }
            (Self::Entity(name), RefKind::Entity) => {
                full_entity_name(factory, &reference.value).as_deref() == Some(name.as_str())
                    || reference.value == *name
            }
            (Self::Service(name), RefKind::Entity) => {
                // entity-auto: create#moqui.basic.Enumeration is used by name only.
                name.split_once('#').is_some_and(|(_, noun)| reference.value == noun)
            }
            (Self::Location(location), RefKind::Location) => {
                reference.value == *location
                    || factory
                        .resource()
                        .get(&reference.value)
                        .ok()
                        .and_then(|resource| resource.file_path().map(Path::to_path_buf))
                        .and_then(|path| std::fs::canonicalize(path).ok())
                        .is_some_and(|path| path.display().to_string() == *location)
            }
            _ => false,
        }
    }

    /// The reference is made inside this target's definition.
    fn is_from(&self, reference: &Reference) -> bool {
        match self {
            Self::Service(name) => reference.from == format!("service:{name}"),
            Self::Entity(name) => {
                reference.from == format!("entity:{name}")
                    || reference
                        .from
                        .rsplit_once('.')
                        .is_some_and(|(_, short)| format!("entity:{short}") == format!("entity:{name}"))
            }
            Self::Location(location) => {
                std::fs::canonicalize(&reference.path).is_ok_and(|path| path.display().to_string() == *location)
            }
        }
    }
}

/// The full name of an entity named in full or short, when the model has it.
fn full_entity_name(factory: &ExecutionContextFactory, name: &str) -> Option<String> {
    let facade = factory.entity()?;
    moqui_core::entity::info::entity_full_name(&facade, name)
}
