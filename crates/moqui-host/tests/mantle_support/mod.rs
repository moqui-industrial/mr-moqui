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

//! Shared fixture for the Mantle flow tests (`mantle_flows.rs`): one scratch runtime and embedded Postgres per test
//! binary, loaded once with every data type (seed, seed-initial, install, demo) of `mantle-udm` and `mantle-usl`,
//! plus the call, login and data-check helpers the ported Spock flows use.
//!
//! The flows share one database, as the Spock specs share one JVM, so [`with_flow`] runs them one at a time. The
//! Spock specs also pin sequenced ids with `tempSetSequencedIdPrimary` and a clock with `setEffectiveTime`; neither
//! exists here, so a port reads generated ids from the service results and leaves dates out of its checks.

#![allow(dead_code, unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moqui_core::context::{ExecutionContext, ExecutionContextFactory};
use moqui_core::driver::DbDriver;
use moqui_core::entity::EntityFacade;
use moqui_core::transaction::PoolConfig;
use moqui_host::db::embedded::EmbeddedCluster;
use moqui_host::{load_data, LoadFlags};
use moqui_lib::node::MNode;
use moqui_lib::object::{to_plain_string, Value};

pub type Map = BTreeMap<String, Value>;

struct Fixture {
    // Field order is drop order: the factory and facade go before the cluster they talk to.
    factory: Arc<ExecutionContextFactory>,
    facade: EntityFacade,
    _driver: Arc<dyn DbDriver>,
    _cluster: EmbeddedCluster,
    runtime: PathBuf,
}

// SAFETY: the fixture is only touched while holding `STATE`, one thread at a time.
unsafe impl Send for Fixture {}

static STATE: Mutex<Option<Fixture>> = Mutex::new(None);
static LOADED: OnceLock<Result<(), String>> = OnceLock::new();

extern "C" {
    fn atexit(callback: extern "C" fn()) -> i32;
}

extern "C" fn stop_fixture() {
    if let Ok(mut state) = STATE.lock() {
        if let Some(fixture) = state.take() {
            let runtime = fixture.runtime.clone();
            drop(fixture);
            let _ = std::fs::remove_dir_all(runtime);
        }
    }
}

/// Whether the Mantle components are in `runtime/component`.
pub fn mantle_present() -> bool {
    let components = repo().join("runtime/component");
    components.join("mantle-udm").is_dir() && components.join("mantle-usl").is_dir()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn start() -> Result<Fixture, String> {
    let repo = repo();
    let components = repo.join("runtime/component");
    let framework = repo.join("runtime/base-component/framework");
    // Short path: the Postgres socket path must stay under 108 bytes.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let runtime = PathBuf::from("/tmp").join(format!("mfl-{}-{nanos}", std::process::id()));
    let cluster_dir = runtime.join("db");
    std::fs::create_dir_all(runtime.join("component")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cluster_dir).map_err(|e| e.to_string())?;
    copy_dir(&repo.join("runtime/conf"), &runtime.join("conf"));
    let link = |from: PathBuf, to: PathBuf| std::os::unix::fs::symlink(from, to).map_err(|e| e.to_string());
    link(repo.join("runtime/base-component"), runtime.join("base-component"))?;
    for name in ["mantle-udm", "mantle-usl"] {
        link(components.join(name), runtime.join("component").join(name))?;
    }
    let flags = LoadFlags {
        data_types: Some(vec!["all".to_string()]),
        ..LoadFlags::default()
    };
    load_data(&runtime, &framework, &cluster_dir, &flags).map_err(|e| format!("load: {e}"))?;
    let (cluster, driver) = EmbeddedCluster::start(&cluster_dir, 30).map_err(|e| e.to_string())?;
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let factory = ExecutionContextFactory::boot_with_framework(&runtime, &framework).map_err(|e| e.to_string())?;
    let entity_dirs = moqui_host::db::entity_facade::component_entity_dirs(&factory);
    let facade = moqui_host::db::entity_facade::open_entity_facade_components(
        &framework,
        &entity_dirs,
        Arc::clone(&driver),
        PoolConfig {
            min: 0,
            max: 12,
            reserve: 2,
            acquire_timeout: Duration::from_secs(30),
            ..Default::default()
        },
        "postgres",
    )
    .map_err(|e| e.to_string())?;
    moqui_host::db::entity_facade::attach(&factory, facade.clone());
    Ok(Fixture {
        factory,
        facade,
        _driver: driver,
        _cluster: cluster,
        runtime,
    })
}

/// A flow's view of the fixture: the facade, with the execution context entered and authorization disabled, as the
/// Spock `setup()` does.
#[derive(Clone)]
pub struct Flow {
    pub facade: EntityFacade,
    pub factory: Arc<ExecutionContextFactory>,
}

/// Run `body` as one flow against the shared, once-loaded database. Returns without running when the Mantle
/// components are absent. Panics (failing the test) when the load fails or a step panics.
pub fn with_flow(name: &str, body: impl FnOnce(&Flow)) {
    if !mantle_present() {
        eprintln!("skipping {name}: runtime/component/mantle-udm and mantle-usl are not present");
        return;
    }
    let mut state: MutexGuard<'_, Option<Fixture>> = match STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if state.is_none() {
        if let Some(Err(err)) = LOADED.get() {
            panic!("the fixture failed to load earlier: {err}");
        }
        match start() {
            Ok(fixture) => {
                let _ = LOADED.set(Ok(()));
                // SAFETY: registering a plain extern "C" fn with the C runtime.
                unsafe { atexit(stop_fixture) };
                *state = Some(fixture);
            }
            Err(err) => {
                let _ = LOADED.set(Err(err.clone()));
                panic!("the fixture failed to load: {err}");
            }
        }
    }
    let fixture = state.as_ref().expect("fixture");
    let context = fixture.factory.new_context();
    let _entered = context.enter();
    let _authz = context.disable_authz();
    let flow = Flow {
        facade: fixture.facade.clone(),
        factory: Arc::clone(&fixture.factory),
    };
    eprintln!("== flow {name}");
    body(&flow);
    let _ = moqui_core::user::logout();
}

/// Build a parameter map: `p(&[("orderId", s("X")), ...])`.
pub fn p(parameters: &[(&str, Value)]) -> Map {
    parameters
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

pub fn s(text: &str) -> Value {
    Value::string(text)
}

/// A decimal or integer parameter given as text, so no float rounding creeps in.
pub fn n(text: &str) -> Value {
    moqui_lib::object::basic_convert(&Value::string(text), "BigDecimal").value
}

pub fn login(username: &str) {
    moqui_core::user::logout().ok();
    moqui_core::user::login_user(username, "moqui").unwrap_or_else(|err| panic!("login {username}: {err}"));
}

pub fn logout() {
    moqui_core::user::logout().unwrap();
}

/// `ec.service.sync().name(name).parameters(..).call()`. A thrown error or a message-facade error is a panic that
/// names the service, which is how a flow reports its first failing service.
pub fn call(name: &str, parameters: &[(&str, Value)]) -> Map {
    eprintln!("   call {name}");
    let out = moqui_core::service::sync(name, &p(parameters)).unwrap_or_else(|err| {
        let text = err.to_string();
        if text.contains("no service named") {
            // the service file is there but the service did not register: say why
            let why = service_status(name).err().unwrap_or_default();
            panic!("SERVICE {name}: {text} ({why})");
        }
        panic!("SERVICE {name}: {err}")
    });
    let errors = ExecutionContext::with_current(|ec| {
        let errors = ec.message().errors().to_vec();
        ec.message().clear_errors();
        errors
    })
    .unwrap_or_default();
    assert!(errors.is_empty(), "SERVICE {name}: {errors:?}");
    out
}

/// Whether a service registers: `Ok(())`, or why not (a Groovy expression left in it, or no such service).
pub fn service_status(name: &str) -> Result<(), String> {
    let (package, verb_noun) = name.rsplit_once('.').ok_or("no package")?;
    let (verb, noun) = verb_noun.split_once('#').ok_or("no verb#noun")?;
    let relative = format!("service/{}.xml", package.replace('.', "/"));
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
    let candidates = [
        root.join("component/mantle-usl").join(&relative),
        root.join("component/mantle-udm").join(&relative),
        root.join("base-component/framework").join(&relative),
    ];
    let path = candidates
        .iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format!("no file {relative}"))?;
    let file = moqui_lib::node::MNode::parse_file(path).map_err(|e| e.to_string())?;
    let defined = file.children().iter().any(|c| {
        c.name() == "service"
            && c.attribute("verb").as_deref() == Some(verb)
            && c.attribute("noun").as_deref() == Some(noun)
    });
    if !defined {
        return Err(format!("{verb_noun} is not defined in {relative}"));
    }
    let failures = moqui_core::service::check(std::slice::from_ref(path)).map_err(|e| e.to_string())?;
    let prefix = format!("service {name} ");
    let mine: Vec<String> = failures.into_iter().filter(|f| f.starts_with(&prefix)).collect();
    if mine.is_empty() {
        Ok(())
    } else {
        Err(mine.join("; "))
    }
}

/// A result field as text; empty when absent.
pub fn text(out: &Map, name: &str) -> String {
    out.get(name).map(to_plain_string).unwrap_or_default()
}

/// A result field that must be there.
pub fn need(out: &Map, name: &str) -> String {
    let value = text(out, name);
    assert!(!value.is_empty(), "result has no {name}: {out:?}");
    value
}

pub fn number(value: Option<&Value>) -> f64 {
    value
        .map(to_plain_string)
        .and_then(|t| t.parse().ok())
        .unwrap_or(f64::NAN)
}

/// `ec.entity.find(entity).condition(..).one()`'s fields, or `None`.
pub fn one(flow: &Flow, entity: &str, key: &[(&str, &str)]) -> Option<Map> {
    let mut find = flow.facade.find(entity).unwrap();
    for (field, value) in key {
        find = find.eq(field, s(value));
    }
    find.one()
        .unwrap_or_else(|err| panic!("find {entity}: {err}"))
        .map(|v| v.fields().clone())
}

pub fn list(flow: &Flow, entity: &str, key: &[(&str, &str)]) -> Vec<Map> {
    let mut find = flow.facade.find(entity).unwrap();
    for (field, value) in key {
        find = find.eq(field, s(value));
    }
    find.list()
        .unwrap_or_else(|err| panic!("find {entity}: {err}"))
        .iter()
        .map(|v| v.fields().clone())
        .collect()
}

/// `ec.entity.makeDataLoader().xmlText(xml).check()`: for each record, rows of the entity that match the record's
/// id fields (names ending in `Id`) are compared on every other attribute, numbers by value and everything else as
/// text; an empty attribute matches null. A record with no matching row is an error. Returns the error lines.
pub fn check(flow: &Flow, xml: &str) -> Vec<String> {
    let root = MNode::parse_text("check", xml).unwrap_or_else(|err| panic!("check xml: {err}"));
    let mut errors = Vec::new();
    for record in root.children() {
        check_record(flow, &record, &mut errors);
    }
    errors
}

fn check_record(flow: &Flow, record: &MNode, errors: &mut Vec<String>) {
    let entity = record.name();
    if !entity.contains('.') {
        // a relationship element (`<items ...>`): not checked, the Spock files use them for nesting
        return;
    }
    let attributes = record.attributes();
    let mut find = flow
        .facade
        .find(&entity)
        .unwrap_or_else(|err| panic!("check {entity}: {err}"));
    for (name, value) in &attributes {
        if name.ends_with("Id") && !value.is_empty() {
            find = find.eq(name, s(value));
        }
    }
    let rows = find.list().unwrap_or_else(|err| panic!("check {entity}: {err}"));
    if rows.is_empty() {
        errors.push(format!("no {entity} row for {attributes:?}"));
    } else {
        let mut best: Vec<String> = Vec::new();
        let mut matched = false;
        for row in rows.iter() {
            let mut row_errors = Vec::new();
            for (name, expected) in &attributes {
                let actual = row.get(name).map(to_plain_string).unwrap_or_default();
                if !same(expected, &actual) {
                    row_errors.push(format!("{entity} {name}: expected [{expected}] got [{actual}]"));
                }
            }
            if row_errors.is_empty() {
                matched = true;
                break;
            }
            if best.is_empty() || row_errors.len() < best.len() {
                best = row_errors;
            }
        }
        if !matched {
            errors.extend(best);
        }
    }
    for child in record.children() {
        // nested records carry their parent's key in the Spock files only by position; check them standalone
        check_record(flow, &child, errors);
    }
}

fn same(expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    match (expected.parse::<f64>(), actual.parse::<f64>()) {
        (Ok(a), Ok(b)) => (a - b).abs() < 0.000_000_5,
        _ => false,
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
