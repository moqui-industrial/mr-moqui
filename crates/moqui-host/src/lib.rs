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

//! Process edge. HTTP, PDF, SSH, and SSO are not implemented yet.
//! `load` reads framework data files. `lint` checks service definitions.
//! Database drivers live in [`db`]. The search store and backend are placed by [`search`].

pub mod db;
pub mod search;

/// Process entry used by the `moqui` binary when it is started with no arguments.
pub fn run() -> &'static str {
    let _ = moqui_core::candidate();
    "moqui candidate build; server not implemented"
}

/// `pg_dump --format=custom` of the embedded `moqui` database at `dir`.
///
/// # Errors
///
/// Returns an error when the cluster does not start or `pg_dump` fails. A postmaster this call started is
/// stopped before it returns. A postmaster that was already live keeps running.
pub fn save_db(
    dir: impl AsRef<std::path::Path>,
    dump: impl AsRef<std::path::Path>,
) -> Result<(), moqui_lib::error::Error> {
    db::embedded::save_db(dir.as_ref(), dump.as_ref())
}

/// `pg_restore --format=custom` into the embedded `moqui` database at `dir`.
///
/// # Errors
///
/// Returns an error when the cluster does not start or `pg_restore` fails. A postmaster this call started is
/// stopped before it returns. A postmaster that was already live keeps running.
pub fn load_save(
    dir: impl AsRef<std::path::Path>,
    dump: impl AsRef<std::path::Path>,
) -> Result<(), moqui_lib::error::Error> {
    db::embedded::load_save(dir.as_ref(), dump.as_ref())
}

const USAGE: &str = "usage: moqui [save-db <cluster-dir> <dump-file> | load-save <cluster-dir> <dump-file> | load [name=value ...] | lint [file]]";

/// No arguments print [`run`]. `save-db` and `load-save` take a cluster directory and a dump file.
/// `load` takes `name=value` pairs. `lint` takes an optional service file.
///
/// # Errors
///
/// Returns an error for any other argument list, an unknown load flag, or when a command fails.
pub fn dispatch<I, S>(args: I) -> Result<Option<&'static str>, moqui_lib::error::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|arg| arg.as_ref().to_string()).collect();
    match args.as_slice() {
        [] => Ok(Some(run())),
        [command, dir, file] if command == "save-db" => {
            save_db(dir, file)?;
            Ok(None)
        }
        [command, dir, file] if command == "load-save" => {
            load_save(dir, file)?;
            Ok(None)
        }
        [command, rest @ ..] if command == "load" => {
            let flags = parse_load_args(rest)?;
            dispatch_load(&flags)?;
            Ok(None)
        }
        [command] if command == "lint" => {
            dispatch_lint(None)?;
            Ok(None)
        }
        [command, path] if command == "lint" => {
            dispatch_lint(Some(path))?;
            Ok(None)
        }
        _ => Err(moqui_lib::error::Error::new(USAGE)),
    }
}

fn dispatch_load(flags: &LoadFlags) -> Result<LoadReport, moqui_lib::error::Error> {
    // Like Moqui's `moqui.runtime`: an install runs from its own directory unless told otherwise.
    let runtime = std::env::var_os("MOQUI_RUNTIME").map_or_else(|| std::path::PathBuf::from("runtime"), Into::into);
    let framework = std::env::var_os("MOQUI_FRAMEWORK").map_or_else(|| runtime.join("../framework"), Into::into);
    let cluster = runtime.join("db/embedded");
    load_data(&runtime, &framework, &cluster, flags)
}

fn dispatch_lint(path: Option<&str>) -> Result<(), moqui_lib::error::Error> {
    let owned = path.map(std::path::PathBuf::from);
    let paths: Vec<std::path::PathBuf> = owned.into_iter().collect();
    let failures = moqui_core::service::check(&paths).map_err(|err| moqui_lib::error::Error::new(err.to_string()))?;
    for failure in &failures {
        println!("{failure}");
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(moqui_lib::error::Error::new(format!(
        "{} service expression(s) failed lint",
        failures.len()
    )))
}

/// `name=value` arguments for [`load_data`], matching `Moqui.loadData`.
///
/// `data_types` is `None` when `types` was omitted. That selects `seed`, `seed-initial`, and `install`.
/// An empty list, or a list containing `all`, selects every data type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadFlags {
    pub data_types: Option<Vec<String>>,
    pub components: Vec<String>,
    pub location: Option<String>,
    /// `timeout` seconds. Each loaded record runs under this transaction timeout.
    pub timeout_seconds: Option<u64>,
    pub raw: bool,
    pub disable_entity_eca: bool,
    pub disable_audit_log: bool,
    pub disable_fk_create: bool,
    pub disable_data_feed: bool,
    /// Extra service files registered before the load. Not a CLI flag. The path must not contain `/service/`.
    pub service_files: Vec<std::path::PathBuf>,
}

/// Counts from one [`load_data`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadReport {
    pub records: u64,
    pub enumeration: i64,
    pub user_group_permission: i64,
    /// `moqui.test.TestFts` existed after `check_tables` and before any data file.
    pub test_fts_before_load: bool,
    /// How many times `UserSecurityChecks` ran during the load. The security ECAs fire on each
    /// `UserGroupMember` and `UserGroupPermission` write, so seed data makes this positive.
    pub security_checks: u64,
}

/// Parse `name=value` load arguments. This does not open a database.
///
/// # Errors
///
/// Returns an error when a token is not `name=value`, the name is unknown, or the value does not parse.
pub fn parse_load_args(args: &[impl AsRef<str>]) -> Result<LoadFlags, moqui_lib::error::Error> {
    let mut flags = LoadFlags::default();
    let mut seen = std::collections::BTreeSet::new();
    for arg in args {
        let arg = arg.as_ref();
        let Some((name, value)) = arg.split_once('=') else {
            return Err(moqui_lib::error::Error::new(format!(
                "load flag '{arg}' is not name=value"
            )));
        };
        if name.is_empty() {
            return Err(moqui_lib::error::Error::new("load flag name is empty"));
        }
        if !seen.insert(name.to_string()) {
            return Err(moqui_lib::error::Error::new(format!("duplicate load flag {name}")));
        }
        match name {
            "types" => flags.data_types = Some(split_csv(value)),
            "components" => flags.components = split_csv(value),
            "location" => {
                if value.is_empty() {
                    return Err(moqui_lib::error::Error::new("location is empty"));
                }
                flags.location = Some(value.to_string());
            }
            "timeout" => flags.timeout_seconds = Some(parse_timeout(value)?),
            "raw" => flags.raw = parse_bool(name, value)?,
            "disableEntityEca" => flags.disable_entity_eca = parse_bool(name, value)?,
            "disableAuditLog" => flags.disable_audit_log = parse_bool(name, value)?,
            "disableFkCreate" => flags.disable_fk_create = parse_bool(name, value)?,
            "disableDataFeed" => flags.disable_data_feed = parse_bool(name, value)?,
            _ => return Err(moqui_lib::error::Error::new(format!("unknown load flag {name}"))),
        }
    }
    Ok(flags)
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_bool(name: &str, value: &str) -> Result<bool, moqui_lib::error::Error> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "y" | "yes" | "1" => Ok(true),
        "false" | "n" | "no" | "0" => Ok(false),
        _ => Err(moqui_lib::error::Error::new(format!(
            "load flag {name} value '{value}' is not a boolean"
        ))),
    }
}

fn parse_timeout(value: &str) -> Result<u64, moqui_lib::error::Error> {
    value.trim().parse::<u64>().map_err(|_| {
        moqui_lib::error::Error::new(format!("timeout value '{value}' is not an integer number of seconds"))
    })
}

fn load_options(flags: &LoadFlags) -> moqui_core::entity::load_feed::LoadOptions {
    let data_types = match &flags.data_types {
        None => ["seed", "seed-initial", "install"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        Some(types) if types.is_empty() || types.iter().any(|kind| kind == "all") => Vec::new(),
        Some(types) => types.clone(),
    };
    moqui_core::entity::load_feed::LoadOptions {
        data_types,
        components: flags.components.clone(),
        only_create: false,
        use_try_insert: flags.raw,
        dummy_fks: false,
        disable_fk_create: flags.raw || flags.disable_fk_create,
        timeout: flags.timeout_seconds.map(std::time::Duration::from_secs),
    }
}

/// Boot, create framework tables, and load data into a fresh embedded cluster at `cluster_dir`.
///
/// The same path serves startup empty-db load and `moqui load`. `location` replaces the conf list.
/// With no `types` and no `location`, an empty `moqui.basic.Enumeration` table loads `seed`,
/// `seed-initial`, and `install`. The factory is dropped, and its worker pool joined, before the
/// postmaster this call started is stopped.
///
/// # Errors
///
/// Returns an error when the build has no embedded Postgres, boot fails, a table cannot be created,
/// or a data file fails.
pub fn load_data(
    runtime_dir: impl AsRef<std::path::Path>,
    framework_dir: impl AsRef<std::path::Path>,
    cluster_dir: impl AsRef<std::path::Path>,
    flags: &LoadFlags,
) -> Result<LoadReport, moqui_lib::error::Error> {
    #[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
    {
        load_embedded(
            runtime_dir.as_ref(),
            framework_dir.as_ref(),
            cluster_dir.as_ref(),
            flags,
        )
    }
    #[cfg(not(all(feature = "embedded-postgres", feature = "postgres")))]
    {
        let _ = (runtime_dir, framework_dir, cluster_dir, flags);
        Err(moqui_lib::error::Error::new("embedded postgres is not in this build"))
    }
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn load_embedded(
    runtime_dir: &std::path::Path,
    framework_dir: &std::path::Path,
    cluster_dir: &std::path::Path,
    flags: &LoadFlags,
) -> Result<LoadReport, moqui_lib::error::Error> {
    use std::sync::Arc;

    use moqui_core::driver::DbDriver;

    use crate::db::embedded::EmbeddedCluster;

    std::fs::create_dir_all(runtime_dir).map_err(|err| {
        moqui_lib::error::Error::new(format!("could not create {}", runtime_dir.display())).with_source(err)
    })?;
    std::fs::create_dir_all(cluster_dir).map_err(|err| {
        moqui_lib::error::Error::new(format!("could not create {}", cluster_dir.display())).with_source(err)
    })?;
    let runtime_dir = std::fs::canonicalize(runtime_dir).map_err(|err| {
        moqui_lib::error::Error::new(format!("could not resolve {}", runtime_dir.display())).with_source(err)
    })?;
    let framework_dir = std::fs::canonicalize(framework_dir).map_err(|err| {
        moqui_lib::error::Error::new(format!("could not resolve {}", framework_dir.display())).with_source(err)
    })?;
    // The cluster is declared outside the factory so the worker pool joins before the postmaster stops.
    xmlshim::provide_libxml2_soname(cluster_dir)?;
    let (cluster, driver) = EmbeddedCluster::start(cluster_dir, 20)?;
    let driver: Arc<dyn DbDriver> = Arc::new(driver);
    let result = load_after_cluster(&runtime_dir, &framework_dir, &driver, flags);
    drop(driver);
    drop(cluster);
    result
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn load_after_cluster(
    runtime_dir: &std::path::Path,
    framework_dir: &std::path::Path,
    driver: &std::sync::Arc<dyn moqui_core::driver::DbDriver>,
    flags: &LoadFlags,
) -> Result<LoadReport, moqui_lib::error::Error> {
    let factory = moqui_core::context::ExecutionContextFactory::boot_with_framework(runtime_dir, framework_dir)
        .map_err(|err| moqui_lib::error::Error::new(err.to_string()))?;
    let outcome = load_with_factory(&factory, framework_dir, driver, flags);
    drop(factory);
    outcome
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn load_with_factory(
    factory: &std::sync::Arc<moqui_core::context::ExecutionContextFactory>,
    framework_dir: &std::path::Path,
    driver: &std::sync::Arc<dyn moqui_core::driver::DbDriver>,
    flags: &LoadFlags,
) -> Result<LoadReport, moqui_lib::error::Error> {
    let facade = crate::db::entity_facade::open_entity_facade(framework_dir, std::sync::Arc::clone(driver), pool())?;
    crate::db::entity_facade::attach(factory, facade.clone());
    // Feeds index through search when it is up; a load still runs when it is not.
    let _search = match crate::search::attach_search(factory, &facade, true) {
        Ok(store) => Some(store),
        Err(err) => {
            eprintln!("search is not available during this load: {err}");
            None
        }
    };
    let context = factory.new_context();
    let entered = context.enter();
    let authz = context.disable_authz();
    let outcome = load_in_context(framework_dir, &facade, flags);
    drop(authz);
    drop(entered);
    drop(context);
    drop(facade);
    outcome
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn pool() -> moqui_core::transaction::PoolConfig {
    moqui_core::transaction::PoolConfig {
        min: 0,
        max: 4,
        reserve: 1,
        acquire_timeout: std::time::Duration::from_secs(30),
    }
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn load_in_context(
    framework_dir: &std::path::Path,
    facade: &moqui_core::entity::EntityFacade,
    flags: &LoadFlags,
) -> Result<LoadReport, moqui_lib::error::Error> {
    use moqui_core::entity::load_feed::{conf_data_locations, load_entity_data};

    let checks_before = moqui_core::service::security_checks_run();
    apply_facade_flags(facade, flags);
    register_services(framework_dir, &flags.service_files)?;
    let entities = framework_entity_names(framework_dir)?;
    if !entities.iter().any(|name| name == "moqui.test.TestFts") {
        return Err(moqui_lib::error::Error::new(
            "framework entity list has no moqui.test.TestFts",
        ));
    }
    let refs: Vec<&str> = entities.iter().map(String::as_str).collect();
    facade.check_tables(&refs)?;
    let test_fts_before_load = facade.table_exists("moqui.test.TestFts")?;
    if !test_fts_before_load {
        return Err(moqui_lib::error::Error::new(
            "moqui.test.TestFts was not created before the seed load",
        ));
    }
    let options = load_options(flags);
    let mut records = 0_u64;
    if let Some(location) = &flags.location {
        eprintln!("load {location}");
        records = records.saturating_add(load_entity_data(facade, location, &options)?.records);
    } else if entity_count(facade, "moqui.basic.Enumeration")? == 0 {
        for path in framework_xml_paths(framework_dir)? {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            if name.ends_with(".eecas.xml") {
                continue;
            }
            let location = path.display().to_string();
            eprintln!("load {location}");
            records = records.saturating_add(load_entity_data(facade, &location, &options)?.records);
        }
        for location in conf_data_locations(facade)? {
            eprintln!("load {location}");
            records = records.saturating_add(load_entity_data(facade, &location, &options)?.records);
        }
    }
    let enumeration = entity_count(facade, "moqui.basic.Enumeration")?;
    let user_group_permission = entity_count(facade, "moqui.security.UserGroupPermission")?;
    eprintln!("loaded {records} records; enumeration {enumeration}; user group permission {user_group_permission}");
    Ok(LoadReport {
        records,
        enumeration,
        user_group_permission,
        test_fts_before_load,
        security_checks: moqui_core::service::security_checks_run().saturating_sub(checks_before),
    })
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn apply_facade_flags(facade: &moqui_core::entity::EntityFacade, flags: &LoadFlags) {
    if flags.raw || flags.disable_entity_eca {
        facade.set_disable_entity_eca(true);
    }
    if flags.raw || flags.disable_audit_log {
        facade.set_disable_audit_log(true);
    }
    if flags.raw || flags.disable_fk_create {
        facade.set_disable_fk_create(true);
    }
    if flags.raw || flags.disable_data_feed {
        facade.set_disable_data_feed(true);
    }
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn register_services(
    framework_dir: &std::path::Path,
    extra: &[std::path::PathBuf],
) -> Result<(), moqui_lib::error::Error> {
    let lint_error = |err: moqui_core::context::Error| moqui_lib::error::Error::new(err.to_string());
    // Explicitly supplied service files must register. A failure there stops the load.
    if !extra.is_empty() {
        let own = moqui_core::service::check(extra).map_err(lint_error)?;
        if !own.is_empty() {
            return Err(moqui_lib::error::Error::new(format!(
                "service definitions failed to register:\n{}",
                own.join("\n")
            )));
        }
    }
    let mut paths = configured_service_paths(framework_dir)?;
    paths.extend(extra.iter().cloned());
    // Framework services that still hold Groovy are left out and named by `moqui lint`.
    let skipped = moqui_core::service::lint(&paths).map_err(lint_error)?;
    if !skipped.is_empty() {
        eprintln!(
            "{} service expression(s) are not JavaScript yet; run `moqui lint`",
            skipped.len()
        );
    }
    Ok(())
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
fn entity_count(facade: &moqui_core::entity::EntityFacade, name: &str) -> Result<i64, moqui_lib::error::Error> {
    facade.find(name)?.count()
}

fn framework_xml_paths(framework_dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, moqui_lib::error::Error> {
    use moqui_lib::node::MNode;

    let conf = MNode::parse_file(framework_dir.join("conf/MoquiDefaultConf.xml"))?;
    let facade = conf
        .first_named("entity-facade")
        .ok_or_else(|| moqui_lib::error::Error::new("conf has no entity-facade"))?;
    let mut paths = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for node in facade.children_named("load-entity") {
        let Some(location) = node.attribute("location") else {
            continue;
        };
        let path = resolve_framework_location(framework_dir, &location)?;
        if path.is_file() {
            push_unique(&mut paths, &mut seen, path);
        }
    }
    let mut extras = Vec::new();
    let dir = framework_dir.join("entity");
    let read = std::fs::read_dir(&dir)
        .map_err(|err| moqui_lib::error::Error::new(format!("could not read {}", dir.display())).with_source(err))?;
    for entry in read {
        let entry = entry.map_err(|err| {
            moqui_lib::error::Error::new(format!("could not read {}", dir.display())).with_source(err)
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("xml") {
            extras.push(path);
        }
    }
    extras.sort();
    for path in extras {
        push_unique(&mut paths, &mut seen, path);
    }
    Ok(paths)
}

fn push_unique(
    paths: &mut Vec<std::path::PathBuf>,
    seen: &mut std::collections::BTreeSet<std::path::PathBuf>,
    path: std::path::PathBuf,
) {
    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if seen.insert(key) {
        paths.push(path);
    }
}

fn framework_entity_names(framework_dir: &std::path::Path) -> Result<Vec<String>, moqui_lib::error::Error> {
    use moqui_lib::node::MNode;

    let mut names = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in framework_xml_paths(framework_dir)? {
        let root = MNode::parse_file(&path)?;
        collect_entities(&root, &mut names, &mut seen);
    }
    Ok(names)
}

fn collect_entities(
    node: &moqui_lib::node::MNode,
    names: &mut Vec<String>,
    seen: &mut std::collections::BTreeSet<String>,
) {
    if node.name() == "entity" {
        if transactional_entity(node) {
            if let Some(full) = full_entity_name(node) {
                if seen.insert(full.clone()) {
                    names.push(full);
                }
            }
        }
        return;
    }
    if node.name() == "seed-data" {
        return;
    }
    for child in node.children() {
        collect_entities(&child, names, seen);
    }
}

fn transactional_entity(node: &moqui_lib::node::MNode) -> bool {
    !matches!(
        node.attribute("group"),
        Some(group) if !group.is_empty() && group != "transactional"
    )
}

fn full_entity_name(node: &moqui_lib::node::MNode) -> Option<String> {
    let name = node.attribute("entity-name")?;
    if name.is_empty() {
        return None;
    }
    match node.attribute("package") {
        Some(package) if !package.is_empty() => Some(format!("{package}.{name}")),
        _ => Some(name),
    }
}

fn configured_service_paths(
    framework_dir: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, moqui_lib::error::Error> {
    use moqui_lib::node::MNode;

    let conf = MNode::parse_file(framework_dir.join("conf/MoquiDefaultConf.xml"))?;
    let facade = conf
        .first_named("service-facade")
        .ok_or_else(|| moqui_lib::error::Error::new("conf has no service-facade"))?;
    let mut paths = Vec::new();
    for node in facade.children_named("service-file") {
        let Some(location) = node.attribute("location") else {
            continue;
        };
        if location.is_empty() {
            continue;
        }
        paths.push(resolve_framework_location(framework_dir, &location)?);
    }
    Ok(paths)
}

fn resolve_framework_location(
    framework_dir: &std::path::Path,
    location: &str,
) -> Result<std::path::PathBuf, moqui_lib::error::Error> {
    if let Some(rest) = location
        .strip_prefix("classpath://")
        .or_else(|| location.strip_prefix("classpath:"))
    {
        return Ok(framework_dir.join(rest.trim_start_matches('/')));
    }
    if let Some(rest) = location.strip_prefix("file:") {
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        return Ok(std::path::PathBuf::from(rest));
    }
    if location.starts_with("component://") || location.starts_with("component:") {
        return Err(moqui_lib::error::Error::new(format!(
            "component location is not resolved by the host loader: {location}"
        )));
    }
    Ok(std::path::PathBuf::from(location))
}

/// theseus 18.6.0 needs `libxml2.so.2`. This host may only ship `libxml2.so.16`. A caller that starts
/// an embedded cluster itself calls [`xmlshim::provide_libxml2_soname`] first, as `load_data` does.
pub mod xmlshim {
    use std::path::{Path, PathBuf};

    use moqui_lib::error::Error;

    /// Make `libxml2.so.2` loadable for a cluster started under `dir`, when the host needs it.
    ///
    /// # Errors
    ///
    /// Returns an error when the shim cannot be built.
    pub fn provide_libxml2_soname(dir: &Path) -> Result<(), Error> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            provide_linux(dir)?;
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = dir;
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn provide_linux(dir: &Path) -> Result<(), Error> {
        let cache = ldconfig_cache();
        if cache.lines().any(|line| ldconfig_soname(line) == "libxml2.so.2") {
            return Ok(());
        }
        let Some(host) = libxml2_16_path(&cache) else {
            return Ok(());
        };
        compile_libxml2_shim(dir, &host)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn ldconfig_cache() -> String {
        std::process::Command::new("ldconfig")
            .arg("-p")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn ldconfig_soname(line: &str) -> &str {
        line.split_whitespace().next().unwrap_or("")
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn libxml2_16_path(cache: &str) -> Option<PathBuf> {
        for line in cache.lines() {
            if ldconfig_soname(line) == "libxml2.so.16" && line.contains("x86-64") {
                let path = PathBuf::from(line.split("=>").nth(1)?.trim());
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn compile_libxml2_shim(dir: &Path, host: &Path) -> Result<(), Error> {
        let shim_dir = dir.join("xmlshim");
        std::fs::create_dir_all(&shim_dir)
            .map_err(|err| Error::new("could not create libxml2 shim dir").with_source(err))?;
        let assembly = shim_dir.join("shim.s");
        let map = shim_dir.join("ver.map");
        let shared = shim_dir.join("libxml2.so.2");
        std::fs::write(&assembly, shim_assembly())
            .map_err(|err| Error::new("could not write libxml2 shim").with_source(err))?;
        std::fs::write(&map, version_script())
            .map_err(|err| Error::new("could not write libxml2 shim").with_source(err))?;
        let output = std::process::Command::new("gcc")
            .args(["-shared", "-fPIC", "-o"])
            .arg(&shared)
            .arg(&assembly)
            .arg("-Wl,-soname,libxml2.so.2")
            .arg(format!("-Wl,--version-script={}", map.display()))
            .arg(host)
            .output()
            .map_err(|err| Error::new("gcc could not build the libxml2 shim").with_source(err))?;
        if !output.status.success() {
            return Err(Error::new(format!(
                "libxml2 shim: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let shim = shim_dir.display().to_string();
        // Two loads in one process each add their directory. Read and write under one lock, or the
        // second write drops the first one's directory and its postmaster cannot find the library.
        static LIBRARY_PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = LIBRARY_PATH_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let library_path = if existing.split(':').any(|part| part == shim) {
            existing
        } else if existing.is_empty() {
            shim
        } else {
            format!("{shim}:{existing}")
        };
        // SAFETY: initdb and the postmaster inherit this. It is set before those processes spawn.
        unsafe { std::env::set_var("LD_LIBRARY_PATH", library_path) };
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn shim_assembly() -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        out.push_str(".text\n");
        for (index, (name, version)) in XML_SYMBOLS.iter().enumerate() {
            writeln!(
                out,
                ".globl shim_{index}\n.type shim_{index}, @function\nshim_{index}:\n    jmp {name}@PLT\n.size shim_{index}, .-shim_{index}\n.symver shim_{index}, {name}@{version}"
            )
            .unwrap();
        }
        out
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn version_script() -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        for (index, version) in XML_VERSIONS.iter().enumerate() {
            out.push_str(version);
            out.push_str(" {\n  global:\n");
            for (name, symbol_version) in XML_SYMBOLS {
                if symbol_version == version {
                    writeln!(out, "    {name};").unwrap();
                }
            }
            out.push_str("  local: *;\n}");
            if index == 0 {
                out.push_str(";\n");
            } else {
                writeln!(out, " {};", XML_VERSIONS[index - 1]).unwrap();
            }
        }
        out
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const XML_VERSIONS: &[&str] = &[
        "LIBXML2_2.4.30",
        "LIBXML2_2.6.0",
        "LIBXML2_2.6.5",
        "LIBXML2_2.6.8",
        "LIBXML2_2.6.23",
    ];

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    const XML_SYMBOLS: &[(&str, &str)] = &[
        ("xmlAddChildList", "LIBXML2_2.4.30"),
        ("xmlBufferContent", "LIBXML2_2.4.30"),
        ("xmlBufferCreate", "LIBXML2_2.4.30"),
        ("xmlBufferFree", "LIBXML2_2.4.30"),
        ("xmlBufferLength", "LIBXML2_2.4.30"),
        ("xmlCharInRange", "LIBXML2_2.6.0"),
        ("xmlCheckVersion", "LIBXML2_2.4.30"),
        ("xmlCopyNode", "LIBXML2_2.4.30"),
        ("xmlCtxtReadDoc", "LIBXML2_2.6.0"),
        ("xmlCtxtReadMemory", "LIBXML2_2.6.0"),
        ("xmlDocSetRootElement", "LIBXML2_2.4.30"),
        ("xmlEncodeSpecialChars", "LIBXML2_2.4.30"),
        ("xmlFreeDoc", "LIBXML2_2.4.30"),
        ("xmlFreeNode", "LIBXML2_2.4.30"),
        ("xmlFreeParserCtxt", "LIBXML2_2.4.30"),
        ("xmlFreeTextWriter", "LIBXML2_2.6.0"),
        ("xmlGetExternalEntityLoader", "LIBXML2_2.4.30"),
        ("xmlGetUTF8Char", "LIBXML2_2.4.30"),
        ("xmlInitParser", "LIBXML2_2.4.30"),
        ("xmlKeepBlanksDefault", "LIBXML2_2.4.30"),
        ("xmlNewDoc", "LIBXML2_2.4.30"),
        ("xmlNewDocText", "LIBXML2_2.4.30"),
        ("xmlNewNode", "LIBXML2_2.4.30"),
        ("xmlNewParserCtxt", "LIBXML2_2.4.30"),
        ("xmlNewStringInputStream", "LIBXML2_2.4.30"),
        ("xmlNewTextWriterMemory", "LIBXML2_2.6.0"),
        ("xmlNodeDump", "LIBXML2_2.4.30"),
        ("xmlParseBalancedChunkMemory", "LIBXML2_2.4.30"),
        ("xmlParserPrintFileContext", "LIBXML2_2.4.30"),
        ("xmlSaveClose", "LIBXML2_2.6.8"),
        ("xmlSaveDoc", "LIBXML2_2.6.8"),
        ("xmlSaveToBuffer", "LIBXML2_2.6.23"),
        ("xmlSaveTree", "LIBXML2_2.6.8"),
        ("xmlSetExternalEntityLoader", "LIBXML2_2.4.30"),
        ("xmlSetGenericErrorFunc", "LIBXML2_2.4.30"),
        ("xmlSetStructuredErrorFunc", "LIBXML2_2.6.0"),
        ("xmlStrchr", "LIBXML2_2.4.30"),
        ("xmlStrcmp", "LIBXML2_2.4.30"),
        ("xmlStrdup", "LIBXML2_2.4.30"),
        ("xmlStrlen", "LIBXML2_2.4.30"),
        ("xmlStrncmp", "LIBXML2_2.4.30"),
        ("xmlStrstr", "LIBXML2_2.4.30"),
        ("xmlTextWriterEndElement", "LIBXML2_2.6.0"),
        ("xmlTextWriterStartElement", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteAttribute", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteBase64", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteBinHex", "LIBXML2_2.6.0"),
        ("xmlTextWriterWriteRaw", "LIBXML2_2.6.0"),
        ("xmlXPathCastBooleanToNumber", "LIBXML2_2.4.30"),
        ("xmlXPathCastBooleanToString", "LIBXML2_2.4.30"),
        ("xmlXPathCastNodeSetToString", "LIBXML2_2.4.30"),
        ("xmlXPathCastNodeToString", "LIBXML2_2.4.30"),
        ("xmlXPathCastNumberToString", "LIBXML2_2.4.30"),
        ("xmlXPathCompiledEval", "LIBXML2_2.4.30"),
        ("xmlXPathCtxtCompile", "LIBXML2_2.6.5"),
        ("xmlXPathFreeCompExpr", "LIBXML2_2.4.30"),
        ("xmlXPathFreeContext", "LIBXML2_2.4.30"),
        ("xmlXPathFreeObject", "LIBXML2_2.4.30"),
        ("xmlXPathNewContext", "LIBXML2_2.4.30"),
        ("xmlXPathRegisterNs", "LIBXML2_2.4.30"),
    ];
}

#[cfg(test)]
mod tests {
    use super::{dispatch, load_options, parse_load_args, LoadFlags};

    #[test]
    fn no_args_returns_the_candidate_line() {
        let line = dispatch(std::iter::empty::<&str>()).unwrap();
        assert_eq!(line, Some("moqui candidate build; server not implemented"));
    }

    #[test]
    fn unknown_command_names_the_dump_commands() {
        let err = dispatch(["nope"]).unwrap_err();
        let message = err.message();
        assert!(message.contains("save-db"), "{err}");
        assert!(message.contains("load-save"), "{err}");
        assert!(message.contains("load [name=value"), "{err}");
        assert!(message.contains("lint"), "{err}");
    }

    #[test]
    fn unknown_load_flag_names_the_flag() {
        let err = dispatch(["load", "nope=1"]).unwrap_err();
        assert!(err.message().contains("unknown load flag nope"), "{err}");
    }

    #[test]
    fn timeout_must_be_an_integer() {
        let err = dispatch(["load", "timeout=soon"]).unwrap_err();
        assert!(err.message().contains("timeout"), "{err}");
    }

    #[test]
    fn missing_types_use_empty_db_defaults() {
        let options = load_options(&LoadFlags::default());
        assert_eq!(options.data_types, ["seed", "seed-initial", "install"]);
        assert!(!options.use_try_insert);
        assert!(!options.disable_fk_create);
    }

    #[test]
    fn raw_and_types_parse() {
        let flags = parse_load_args(&[
            "types=seed,seed-initial",
            "raw=true",
            "disableFkCreate=false",
            "components=framework,mantle",
            "timeout=15",
        ])
        .unwrap();
        assert!(flags.raw);
        assert!(!flags.disable_fk_create);
        assert_eq!(flags.timeout_seconds, Some(15));
        assert_eq!(flags.components, ["framework", "mantle"]);
        let options = load_options(&flags);
        assert_eq!(options.data_types, ["seed", "seed-initial"]);
        assert!(options.use_try_insert);
        assert!(options.disable_fk_create);
    }

    #[test]
    fn types_all_clears_the_filter() {
        let flags = parse_load_args(&["types=all"]).unwrap();
        assert!(load_options(&flags).data_types.is_empty());
        let empty = parse_load_args(&["types="]).unwrap();
        assert!(load_options(&empty).data_types.is_empty());
    }

    #[test]
    fn camel_case_flags_are_independent() {
        let flags = parse_load_args(&[
            "disableEntityEca=true",
            "disableAuditLog=yes",
            "disableFkCreate=1",
            "disableDataFeed=y",
        ])
        .unwrap();
        assert!(flags.disable_entity_eca);
        assert!(flags.disable_audit_log);
        assert!(flags.disable_fk_create);
        assert!(flags.disable_data_feed);
        assert!(!flags.raw);
        let options = load_options(&flags);
        assert!(options.disable_fk_create);
        assert!(!options.use_try_insert);
    }

    #[test]
    fn lint_groovy_leftover_is_a_process_error() {
        let dir = std::env::temp_dir().join(format!("moqui-lint-{}", std::process::id()));
        let _guard = DirGuard(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("groovy-leftover.xml");
        std::fs::write(
            &path,
            r#"<services>
                <service verb="groovy" noun="Broken">
                    <actions><set field="x" from="a ?: b"/></actions>
                </service>
            </services>"#,
        )
        .unwrap();
        let err = dispatch(["lint", &path.display().to_string()]).expect_err("lint fails the process");
        assert!(err.to_string().contains("failed lint"), "{err}");
    }

    #[test]
    fn lint_of_a_clean_file_succeeds() {
        let dir = std::env::temp_dir().join(format!("moqui-lint-clean-{}", std::process::id()));
        let _guard = DirGuard(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clean.xml");
        std::fs::write(
            &path,
            r#"<services><service verb="clean" noun="Fine"><actions><set field="x" from="a || b"/></actions></service></services>"#,
        )
        .unwrap();
        dispatch(["lint", &path.display().to_string()]).unwrap();
    }

    #[test]
    fn lint_with_no_path_loads_configured_files() {
        // Framework services are not all JavaScript yet, so this may fail. It must not panic.
        let _ = dispatch(["lint"]);
    }

    struct DirGuard(std::path::PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
