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

//! `moqui test-client [component | path ...]` (plans/ClientTestHarness.md): run the spec files of a component's
//! `test/client` directory against a server that is already running, over HTTP and WebSocket. The specs have the
//! format, the runner and the report of `moqui test`; they run under [`ClientHost`], a [`SpecHost`] with no execution
//! context.
//!
//! A pure client: no execution context factory, database, listener, conf, `runtime/.env`, `runtime/log/moqui.log` or
//! service workers. It reads the spec files and its components' files, and writes only under `--out` and `--junit`,
//! so it runs beside a server in the same runtime directory, or from another machine.

mod http;
mod raw;
mod ws;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use moqui_core::testkit::{run_spec, spec_missing_features, CaseExtras, RunOptions, SpecHost};
use moqui_lib::error::Error;
use moqui_lib::sync::lock;
use serde_json::{json, Value};

use crate::test_cmd::{component_name, component_specs, discover, report, select_specs, skipped_spec, TestComponent};
use http::{Ask, Server, Session};

const PRELUDE: &str = include_str!("prelude.js");

/// The server when neither `--server` nor `MOQUI_TEST_SERVER` names one.
const DEFAULT_SERVER: &str = "http://localhost:8080";

/// A request's limit when its own `timeout` does not set one.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// The response bodies kept for `save`, the latest.
const KEPT_BODIES: usize = 32;

/// What `moqui test-client` was asked to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientArgs {
    /// Component names, `test/client` directories or `.spec.js` files.
    pub targets: Vec<String>,
    pub server: Option<String>,
    pub specs: Vec<String>,
    pub feature: Option<String>,
    /// `--param name=value`, in order; a later one wins.
    pub params: Vec<(String, String)>,
    pub junit: Option<PathBuf>,
    pub out: Option<PathBuf>,
    /// `--wait seconds`: poll until the server answers.
    pub wait: Option<f64>,
    /// `--timeout seconds`: what `waitUntil` and a socket wait by default.
    pub timeout: f64,
    pub insecure: bool,
    pub allow_http: bool,
    pub fail_on_skip: bool,
    pub log_level: Option<String>,
}

fn seconds_option(name: &str, text: &str) -> Result<f64, Error> {
    text.parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .ok_or_else(|| Error::new(format!("{name} is a number of seconds, not '{text}'")))
}

pub fn parse_client_args(args: &[String]) -> Result<ClientArgs, Error> {
    let mut parsed = ClientArgs {
        timeout: 10.0,
        ..ClientArgs::default()
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next().cloned().ok_or_else(|| {
                Error::new(format!(
                    "{name} needs a value\nRun 'moqui help test-client' for details."
                ))
            })
        };
        match arg.as_str() {
            "--server" => parsed.server = Some(value("--server")?),
            "--spec" => parsed.specs.extend(
                value("--spec")?
                    .split(',')
                    .filter(|name| !name.is_empty())
                    .map(str::to_string),
            ),
            "--feature" => parsed.feature = Some(value("--feature")?),
            "--param" => {
                let pair = value("--param")?;
                let (name, text) = pair
                    .split_once('=')
                    .filter(|(name, _)| !name.trim().is_empty())
                    .ok_or_else(|| Error::new(format!("--param needs name=value, not '{pair}'")))?;
                parsed.params.push((name.trim().to_string(), text.to_string()));
            }
            "--junit" => parsed.junit = Some(PathBuf::from(value("--junit")?)),
            "--out" => parsed.out = Some(PathBuf::from(value("--out")?)),
            "--wait" => parsed.wait = Some(seconds_option("--wait", &value("--wait")?)?),
            "--timeout" => {
                parsed.timeout = seconds_option("--timeout", &value("--timeout")?)?;
                if parsed.timeout <= 0.0 {
                    return Err(Error::new("--timeout is more than 0 seconds"));
                }
            }
            "--log-level" => parsed.log_level = Some(value("--log-level")?),
            "--insecure" => parsed.insecure = true,
            "--allow-http" => parsed.allow_http = true,
            "--fail-on-skip" => parsed.fail_on_skip = true,
            "--serve" => {
                return Err(Error::new(
                    "--serve is not built yet (plans/ClientTestHarness.md P2): start a server (./moq run) and pass --server",
                ))
            }
            "--chromium" | "--headed" => {
                return Err(Error::new(format!(
                    "{arg} is for the browser, which is not built yet (plans/ClientTestHarness.md P5)"
                )))
            }
            other if other.starts_with("--") => {
                return Err(Error::new(format!(
                    "unknown option {other}\nRun 'moqui help test-client' for details."
                )));
            }
            target => parsed.targets.push(target.to_string()),
        }
    }
    Ok(parsed)
}

/// Run `moqui test-client` with console logging. An error when the server does not answer or anything failed, so
/// the exit status is 1.
pub fn dispatch_test_client(args: &[String]) -> Result<(), Error> {
    let parsed = parse_client_args(args)?;
    let console = match parsed.log_level.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("trace") => tracing::Level::TRACE,
        Some("debug") => tracing::Level::DEBUG,
        Some("info") => tracing::Level::INFO,
        Some("error") => tracing::Level::ERROR,
        _ => tracing::Level::WARN,
    };
    // Console only: no log file is attached, so `runtime/log/moqui.log` is never opened.
    crate::logging::install_with_console(Some(console));
    run(&parsed, &mut |line| println!("{line}"))
}

/// Run the client specs `args` selects, each line of the report to `print`. Logging is the caller's.
///
/// # Errors
///
/// Returns an error for a bad target or server, when nothing answers at the server, or when a spec or feature failed
/// (or, with `--fail-on-skip`, was skipped).
pub fn run(args: &ClientArgs, print: &mut dyn FnMut(&str)) -> Result<(), Error> {
    let (runtime, _framework) = crate::runtime_framework_dirs();
    let server_text = args
        .server
        .clone()
        .or_else(|| {
            std::env::var("MOQUI_TEST_SERVER")
                .ok()
                .filter(|text| !text.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_SERVER.to_string());
    let server = Server::parse(&server_text, args.allow_http)?;
    let components = select(&runtime, args)?;
    if args.insecure {
        tracing::warn!("--insecure: the certificate of {} is not checked", server.url);
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(args.insecure)
        .user_agent(concat!("moqui-test-client/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| Error::new(format!("HTTP client: {err}")))?;
    check_server(&client, &server, args.wait)?;
    let out_dir = args.out.clone().unwrap_or_else(|| default_out_dir(&runtime));
    let host = Arc::new(ClientHost {
        server,
        params: params(args),
        client,
        insecure: args.insecure,
        timeout: Duration::from_secs_f64(args.timeout),
        out_dir: out_dir.clone(),
        component_dir: Mutex::new(PathBuf::new()),
        next_id: AtomicU64::new(1),
        sessions: Mutex::new(HashMap::new()),
        sockets: Mutex::new(HashMap::new()),
        bodies: Mutex::new(VecDeque::new()),
    });
    let host_dyn: Arc<dyn SpecHost> = host.clone();
    let options = RunOptions {
        feature_filter: args.feature.clone(),
        host: Some(host_dyn),
    };
    let started = Instant::now();
    let mut results = Vec::new();
    for component in &components {
        *lock(&host.component_dir) = component.dir.clone();
        for spec in &component.specs {
            let missing = spec_missing_features(&spec.source);
            let result = if missing.is_empty() {
                let result = run_spec(spec, &options, &mut |case| print(&report::case_line(&spec.name, case)));
                if let Some(failure) = &result.setup_failure {
                    print(&format!(
                        "{} setup FAILED: {}",
                        spec.name,
                        failure.message.lines().next().unwrap_or("")
                    ));
                }
                result
            } else {
                let result = skipped_spec(spec, &missing);
                for case in &result.cases {
                    print(&report::case_line(&spec.name, case));
                }
                result
            };
            results.push(report::ComponentSpec {
                component: component.name.clone(),
                result,
            });
        }
    }
    if let Some(dir) = &args.junit {
        report::write_junit(dir, &results, ".client")?;
    }
    let (text, failed) = report::summary(&results, started.elapsed());
    for line in text.trim_end_matches('\n').split('\n') {
        print(line);
    }
    if out_dir.is_dir() {
        print(&format!("Output: {}", out_dir.display()));
    }
    if failed > 0 {
        return Err(Error::new(format!("{failed} test failure(s)")));
    }
    let skipped = results
        .iter()
        .flat_map(|spec| &spec.result.cases)
        .filter(|case| matches!(case.outcome, moqui_core::testkit::Outcome::Skipped(_)))
        .count();
    if args.fail_on_skip && skipped > 0 {
        return Err(Error::new(format!("{skipped} skipped, and --fail-on-skip")));
    }
    Ok(())
}

/// The components and specs to run: those named (a component, a `test/client` directory or a spec file), or every
/// component with client specs; then `--spec` with the specs their `dependsOn` names.
fn select(runtime: &Path, args: &ClientArgs) -> Result<Vec<TestComponent>, Error> {
    let add = |component: TestComponent, components: &mut Vec<TestComponent>| match components
        .iter_mut()
        .find(|known| known.name == component.name)
    {
        Some(known) => {
            for spec in component.specs {
                if !known.specs.iter().any(|have| have.name == spec.name) {
                    known.specs.push(spec);
                }
            }
        }
        None => components.push(component),
    };
    let discovered = if args.targets.iter().any(|target| !is_path(target)) || args.targets.is_empty() {
        discover(runtime, "test/client")?
    } else {
        Vec::new()
    };
    let mut components: Vec<TestComponent> = Vec::new();
    if args.targets.is_empty() {
        components = discovered;
    } else {
        for target in &args.targets {
            if is_path(target) {
                add(path_component(Path::new(target))?, &mut components);
            } else {
                let component = discovered
                    .iter()
                    .find(|component| &component.name == target)
                    .cloned()
                    .ok_or_else(|| Error::new(format!("component {target} has no test/client/*.spec.js files")))?;
                add(component, &mut components);
            }
        }
    }
    order_by_depends_on(&mut components);
    if !args.specs.is_empty() {
        let mut picked = Vec::new();
        for mut component in components {
            let names: Vec<String> = args
                .specs
                .iter()
                .filter(|name| component.specs.iter().any(|spec| &spec.name == *name))
                .cloned()
                .collect();
            if names.is_empty() {
                continue;
            }
            component.specs = select_specs(&component, &names)?;
            picked.push(component);
        }
        for name in &args.specs {
            if !picked
                .iter()
                .any(|component| component.specs.iter().any(|spec| &spec.name == name))
            {
                return Err(Error::new(format!("no component has a client spec {name}")));
            }
        }
        components = picked;
    }
    if components.is_empty() {
        return Err(Error::new(format!(
            "no test/client/*.spec.js files under {}/base-component, {}/component or {}/dev-component",
            runtime.display(),
            runtime.display(),
            runtime.display()
        )));
    }
    Ok(components)
}

fn is_path(target: &str) -> bool {
    target.contains('/') || target.ends_with(".spec.js") || target == "." || target == ".."
}

/// The component of a `test/client` directory, a component directory, or one spec file in `test/client`: the directory
/// above `test`. A file brings its own spec only, with what its `dependsOn` names.
fn path_component(path: &Path) -> Result<TestComponent, Error> {
    let missing = || {
        Error::new(format!(
            "{} is not a test/client directory or a client .spec.js file",
            path.display()
        ))
    };
    let path = std::fs::canonicalize(path).map_err(|_| missing())?;
    let (client_dir, file) = if path.is_file() {
        (path.parent().ok_or_else(missing)?.to_path_buf(), Some(path.clone()))
    } else if path.join("test/client").is_dir() {
        (path.join("test/client"), None)
    } else {
        (path.clone(), None)
    };
    if client_dir.file_name().and_then(|name| name.to_str()) != Some("client")
        || client_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("test")
    {
        return Err(missing());
    }
    let dir = client_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(missing)?
        .to_path_buf();
    let name = component_name(&dir);
    let mut component = TestComponent {
        specs: component_specs(&name, &client_dir, "test/client")?,
        name,
        dir,
    };
    if let Some(file) = file {
        let spec_name = file
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".spec.js"))
            .ok_or_else(missing)?
            .to_string();
        component.specs = select_specs(&component, &[spec_name])?;
    }
    Ok(component)
}

/// Each component after the components its `component.xml` `depends-on` names, otherwise in the order found.
fn order_by_depends_on(components: &mut Vec<TestComponent>) {
    let depends: Vec<Vec<String>> = components
        .iter()
        .map(|component| {
            moqui_lib::node::MNode::parse_file(component.dir.join("component.xml"))
                .map(|node| {
                    node.children_named("depends-on")
                        .iter()
                        .filter_map(|child| child.attribute("name"))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    let names: Vec<String> = components.iter().map(|component| component.name.clone()).collect();
    let mut placed: Vec<usize> = Vec::new();
    while placed.len() < components.len() {
        let next = (0..components.len()).find(|index| {
            !placed.contains(index)
                && depends[*index].iter().all(|name| {
                    names
                        .iter()
                        .position(|known| known == name)
                        .is_none_or(|at| placed.contains(&at) || at == *index)
                })
        });
        // A cycle keeps the order found for what is left.
        let next = next.unwrap_or_else(|| {
            (0..components.len())
                .find(|index| !placed.contains(index))
                .unwrap_or_default()
        });
        placed.push(next);
    }
    let mut taken: Vec<Option<TestComponent>> = components.drain(..).map(Some).collect();
    components.extend(placed.into_iter().filter_map(|index| taken[index].take()));
}

/// `GET /` with redirects off until anything answers, for `--wait` seconds when given.
fn check_server(client: &reqwest::blocking::Client, server: &Server, wait: Option<f64>) -> Result<(), Error> {
    let deadline = Instant::now() + Duration::from_secs_f64(wait.unwrap_or(0.0));
    let url = server.join("/");
    loop {
        match client.get(&url).timeout(Duration::from_secs(10)).send() {
            Ok(_) => return Ok(()),
            Err(err) => {
                if Instant::now() >= deadline {
                    tracing::debug!("{url}: {}", http::error_chain(&err));
                    return Err(Error::new(format!(
                        "nothing answers at {}: start a server (./moq run), or pass --server or --serve",
                        server.url
                    )));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// `runtime/tmp/test-client/<yyyyMMdd-HHmmss>-<pid>`, or `test-client-out/<run>` in the working directory without a
/// runtime. Made when something is written to it.
fn default_out_dir(runtime: &Path) -> PathBuf {
    let run = format!(
        "{}-{}",
        jiff::Zoned::now().strftime("%Y%m%d-%H%M%S"),
        std::process::id()
    );
    if runtime.join("base-component").is_dir() {
        runtime.join("tmp/test-client").join(run)
    } else {
        PathBuf::from("test-client-out").join(run)
    }
}

/// The `MOQUI_TEST_<NAME>` variables (`MOQUI_TEST_LOAD_PASSWORD` is `params.loadPassword`), then `--param`.
fn params(args: &ClientArgs) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = std::env::vars()
        .filter_map(|(name, value)| {
            let rest = name.strip_prefix("MOQUI_TEST_")?;
            (!rest.is_empty() && rest != "SERVER").then(|| (camel_case(rest), value))
        })
        .collect();
    for (name, value) in &args.params {
        out.insert(name.clone(), value.clone());
    }
    out
}

fn camel_case(name: &str) -> String {
    let mut out = String::new();
    for (index, part) in name.split('_').filter(|part| !part.is_empty()).enumerate() {
        let lower = part.to_ascii_lowercase();
        if index == 0 {
            out.push_str(&lower);
        } else {
            let mut chars = lower.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.extend(chars);
            }
        }
    }
    out
}

/// The client host of a run: the server, the params, the HTTP client, and what the current spec opened.
pub(crate) struct ClientHost {
    server: Server,
    params: BTreeMap<String, String>,
    client: reqwest::blocking::Client,
    insecure: bool,
    /// `--timeout`.
    timeout: Duration,
    out_dir: PathBuf,
    /// The component of the spec running, the root of `files`.
    component_dir: Mutex<PathBuf>,
    next_id: AtomicU64,
    sessions: Mutex<HashMap<u64, Arc<Mutex<Session>>>>,
    sockets: Mutex<HashMap<u64, Arc<Mutex<ws::Socket>>>>,
    bodies: Mutex<VecDeque<(u64, Arc<Vec<u8>>)>>,
}

fn field<'a>(args: &'a Value, name: &str) -> &'a Value {
    args.get(name).unwrap_or(&Value::Null)
}

fn text_field(args: &Value, name: &str) -> String {
    field(args, name).as_str().unwrap_or("").to_string()
}

fn pairs(value: &Value) -> Vec<(String, String)> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(name, value)| {
                    let text = value.as_str().map_or_else(|| value.to_string(), str::to_string);
                    (name.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

impl ClientHost {
    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn session(&self, args: &Value) -> Result<Arc<Mutex<Session>>, String> {
        let id = field(args, "session").as_u64().unwrap_or(0);
        lock(&self.sessions)
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("session {id} is closed: a session lives for its spec"))
    }

    fn socket(&self, args: &Value) -> Result<Arc<Mutex<ws::Socket>>, String> {
        let id = field(args, "id").as_u64().unwrap_or(0);
        lock(&self.sockets)
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("socket {id} is closed"))
    }

    fn seconds(&self, args: &Value, name: &str, default: Duration) -> Duration {
        field(args, name)
            .as_f64()
            .filter(|seconds| *seconds >= 0.0)
            .map_or(default, Duration::from_secs_f64)
    }

    /// Keep a body for `save` and answer the response JSON.
    fn answer(&self, answer: http::Answer) -> Value {
        let id = self.id();
        let json = answer.to_json(id);
        let mut bodies = lock(&self.bodies);
        bodies.push_back((id, Arc::new(answer.body)));
        while bodies.len() > KEPT_BODIES {
            bodies.pop_front();
        }
        json
    }

    fn request(&self, args: &Value) -> Result<Value, String> {
        let session = self.session(args)?;
        let ask = Ask {
            method: text_field(args, "method"),
            url: self.server.join(&text_field(args, "path")),
            opts: field(args, "opts").as_object().cloned().unwrap_or_default(),
        };
        let answer = http::send(&self.client, &mut lock(&session), &ask, REQUEST_TIMEOUT)?;
        Ok(self.answer(answer))
    }

    fn raw(&self, args: &Value) -> Result<Value, String> {
        let session = self.session(args)?;
        let opts = field(args, "opts");
        let cookies = field(opts, "cookies").as_bool().unwrap_or(true);
        let timeout = self.seconds(opts, "timeout", self.timeout);
        let answer = {
            let held = lock(&session);
            raw::send(
                &self.server,
                &held,
                &text_field(args, "text"),
                cookies,
                self.insecure,
                timeout,
            )?
        };
        let content_type = answer.header("content-type").unwrap_or("").to_string();
        lock(&session).absorb(&answer.url, &answer.headers, &content_type, &answer.body);
        Ok(self.answer(answer))
    }

    fn connect(&self, args: &Value) -> Result<Value, String> {
        let session = self.session(args)?;
        let origin = field(args, "origin").as_str().map(str::to_string);
        let connected = {
            let held = lock(&session);
            ws::connect(
                &self.server,
                &held,
                &text_field(args, "path"),
                &pairs(field(args, "headers")),
                origin.as_deref(),
                self.insecure,
                self.seconds(args, "timeout", self.timeout),
            )?
        };
        let id = connected.socket.is_some().then(|| self.id());
        let json = connected.to_json(id);
        if let (Some(id), Some(socket)) = (id, connected.socket) {
            lock(&self.sockets).insert(id, Arc::new(Mutex::new(socket)));
        }
        Ok(json)
    }

    /// Save a kept body into the output directory under `name`, a plain file name.
    fn save(&self, args: &Value) -> Result<Value, String> {
        let id = field(args, "id").as_u64().unwrap_or(0);
        let name = text_field(args, "name");
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            return Err(format!("save: '{name}' is not a plain file name"));
        }
        let body = lock(&self.bodies)
            .iter()
            .find(|(kept, _)| *kept == id)
            .map(|(_, body)| Arc::clone(body))
            .ok_or_else(|| format!("save: the body of response {id} is no longer kept (the last {KEPT_BODIES} are)"))?;
        std::fs::create_dir_all(&self.out_dir).map_err(|err| format!("{}: {err}", self.out_dir.display()))?;
        let path = self.out_dir.join(&name);
        std::fs::write(&path, body.as_slice()).map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(Value::String(path.display().to_string()))
    }

    /// A path within the spec's component directory: relative, with no `..`, and not out through a link.
    fn confined(&self, relative: &str) -> Result<PathBuf, String> {
        let root = lock(&self.component_dir).clone();
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir | std::path::Component::Prefix(_)))
        {
            return Err(format!("files: {relative} is not a path within the component"));
        }
        let root = std::fs::canonicalize(&root).map_err(|err| format!("files: {}: {err}", root.display()))?;
        let full = std::fs::canonicalize(root.join(path)).map_err(|err| format!("files: {relative}: {err}"))?;
        if !full.starts_with(&root) {
            return Err(format!("files: {relative} is outside the component"));
        }
        Ok(full)
    }

    fn list(&self, args: &Value) -> Result<Value, String> {
        let dir = text_field(args, "dir");
        let root = self.confined(".")?;
        let start = self.confined(if dir.is_empty() { "." } else { &dir })?;
        let mut found = Vec::new();
        let mut pending = vec![start];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    pending.push(path);
                } else if kind.is_file() {
                    if let Ok(relative) = path.strip_prefix(&root) {
                        found.push(relative.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        found.sort();
        Ok(json!(found))
    }

    fn dispatch(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "config" => Ok(json!({
                "server": self.server.to_json(),
                "params": self.params,
                "timeout": self.timeout.as_secs_f64(),
            })),
            "session.new" => {
                let id = self.id();
                let session = Session {
                    headers: pairs(field(args, "headers")),
                    ..Session::default()
                };
                lock(&self.sessions).insert(id, Arc::new(Mutex::new(session)));
                Ok(json!(id))
            }
            "session.cookies" => {
                let session = self.session(args)?;
                let cookies = lock(&session).jar.to_json();
                Ok(cookies)
            }
            "session.clearCookies" => {
                let session = self.session(args)?;
                lock(&session).jar.clear();
                Ok(Value::Null)
            }
            "session.token" => {
                let session = self.session(args)?;
                let token = lock(&session).token.clone();
                Ok(json!(token))
            }
            "http.request" => self.request(args),
            "http.raw" => self.raw(args),
            "response.save" => self.save(args),
            "ws.connect" => self.connect(args),
            "ws.send" => {
                let socket = self.socket(args)?;
                lock(&socket).send(&text_field(args, "text"))?;
                Ok(Value::Null)
            }
            "ws.receive" => {
                let timeout = self.seconds(args, "timeout", self.timeout);
                let socket = self.socket(args)?;
                let frame = lock(&socket).receive(timeout)?;
                Ok(json!(frame))
            }
            "ws.close" => {
                let id = field(args, "id").as_u64().unwrap_or(0);
                if let Some(socket) = lock(&self.sockets).remove(&id) {
                    lock(&socket).close();
                }
                Ok(Value::Null)
            }
            "files.list" => self.list(args),
            "files.read" => {
                let path = self.confined(&text_field(args, "path"))?;
                std::fs::read_to_string(&path)
                    .map(Value::String)
                    .map_err(|err| format!("files: {}: {err}", path.display()))
            }
            "sleep" => {
                let ms = field(args, "ms").as_f64().unwrap_or(0.0).clamp(0.0, 3_600_000.0);
                std::thread::sleep(Duration::from_secs_f64(ms / 1000.0));
                Ok(Value::Null)
            }
            "log" => {
                let message = text_field(args, "message");
                match text_field(args, "level").as_str() {
                    "trace" => tracing::trace!("{message}"),
                    "debug" => tracing::debug!("{message}"),
                    "warn" => tracing::warn!("{message}"),
                    "error" => tracing::error!("{message}"),
                    _ => tracing::info!("{message}"),
                }
                Ok(Value::Null)
            }
            other => Err(format!("{other} is not a call of the client host")),
        }
    }
}

impl SpecHost for ClientHost {
    fn prelude(&self) -> &str {
        PRELUDE
    }

    fn call(&self, name: &str, args: &str) -> Result<String, String> {
        let args: Value =
            serde_json::from_str(args).map_err(|err| format!("{name}: the arguments did not read: {err}"))?;
        self.dispatch(name, &args).map(|value| value.to_string())
    }

    fn after_case(&self, _failed: bool) -> CaseExtras {
        CaseExtras::default()
    }

    /// Close the spec's sockets and drop its sessions and kept bodies.
    fn after_spec(&self) {
        let sockets: Vec<_> = lock(&self.sockets).drain().map(|(_, socket)| socket).collect();
        for socket in sockets {
            lock(&socket).close();
        }
        lock(&self.sessions).clear();
        lock(&self.bodies).clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn options_parse_and_errors_name_the_option() {
        let parsed = parse_client_args(&args(&[
            "framework",
            "path/to/test/client",
            "--server",
            "https://test.example.com",
            "--spec",
            "A,B",
            "--param",
            "users=50",
            "--param",
            "note=a=b",
            "--junit",
            "out",
            "--wait",
            "30",
            "--timeout",
            "2.5",
            "--insecure",
            "--fail-on-skip",
        ]))
        .unwrap();
        assert_eq!(
            parsed.targets,
            vec!["framework".to_string(), "path/to/test/client".to_string()]
        );
        assert_eq!(parsed.server.as_deref(), Some("https://test.example.com"));
        assert_eq!(parsed.specs, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(
            parsed.params,
            vec![
                ("users".to_string(), "50".to_string()),
                ("note".to_string(), "a=b".to_string())
            ]
        );
        assert_eq!((parsed.wait, parsed.timeout), (Some(30.0), 2.5));
        assert!(parsed.insecure && parsed.fail_on_skip && !parsed.allow_http);
        for (bad, says) in [
            (&["--param", "novalue"][..], "--param needs name=value"),
            (&["--wait", "soon"][..], "--wait is a number of seconds"),
            (&["--timeout", "0"][..], "--timeout is more than 0"),
            (&["--server"][..], "--server needs a value"),
            (&["--bogus"][..], "unknown option --bogus"),
            (&["--serve"][..], "not built yet"),
            (&["--headed"][..], "for the browser"),
        ] {
            let err = parse_client_args(&args(bad)).unwrap_err().to_string();
            assert!(err.contains(says), "{bad:?}: {err}");
        }
    }

    #[test]
    fn params_take_the_environment_names_in_camel_case() {
        assert_eq!(camel_case("LOAD_PASSWORD"), "loadPassword");
        assert_eq!(camel_case("USER"), "user");
        assert_eq!(camel_case("MAX_P95"), "maxP95");
    }

    #[test]
    fn components_follow_depends_on() {
        let dir = std::env::temp_dir().join(format!("moqui-test-client-order-{}", std::process::id()));
        let make = |name: &str, depends: &[&str]| {
            let component = dir.join(name);
            std::fs::create_dir_all(&component).unwrap();
            let children: String = depends
                .iter()
                .map(|depend| format!("<depends-on name=\"{depend}\"/>"))
                .collect();
            std::fs::write(
                component.join("component.xml"),
                format!("<component name=\"{name}\">{children}</component>"),
            )
            .unwrap();
            TestComponent {
                name: name.to_string(),
                dir: component,
                specs: Vec::new(),
            }
        };
        let mut components = vec![make("a", &["c"]), make("b", &[]), make("c", &["missing"])];
        order_by_depends_on(&mut components);
        let names: Vec<&str> = components.iter().map(|component| component.name.as_str()).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_target_is_its_component_and_one_file_its_spec() {
        let dir = std::env::temp_dir().join(format!("moqui-test-client-path-{}", std::process::id()));
        let client = dir.join("comp/test/client");
        std::fs::create_dir_all(&client).unwrap();
        std::fs::write(dir.join("comp/component.xml"), "<component name=\"my-comp\"/>").unwrap();
        std::fs::write(client.join("A.spec.js"), "spec({})").unwrap();
        std::fs::write(client.join("B.spec.js"), "spec({ dependsOn: ['A'] })").unwrap();
        let whole = path_component(&client).unwrap();
        assert_eq!(whole.name, "my-comp");
        assert_eq!(whole.specs.len(), 2);
        assert_eq!(whole.specs[0].display, "my-comp/test/client/A.spec.js");
        assert_eq!(path_component(&dir.join("comp")).unwrap().specs.len(), 2);
        let one = path_component(&client.join("B.spec.js")).unwrap();
        assert_eq!(
            one.specs.iter().map(|spec| spec.name.as_str()).collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert!(path_component(&dir).is_err());
        assert!(path_component(&dir.join("nowhere")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
