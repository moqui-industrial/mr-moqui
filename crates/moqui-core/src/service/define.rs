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

//! Service definition load. One bad service stays out of the registry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use moqui_lib::node::MNode;

use crate::context::{Error, ExecutionContext, ExecutionContextFactory};
use crate::transaction::TransactionAttribute;

const SEMAPHORE_ENTITY: &str = "moqui.service.semaphore.ServiceParameterSemaphore";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceKind {
    Inline,
    EntityAuto,
    Script,
    Interface,
    RemoteRest,
    Mail,
    /// A system message or entity sync service whose body is Rust, in `system_sync`.
    SystemSync,
    /// The elFinder connector service, whose body is Rust, in `elfinder`.
    ElFinder,
    /// A tools service (print jobs, entity data snapshots, data document notifications) whose body is Rust, in `tools`.
    Tools,
    /// A service defined but not available in this port, in `not_ported`: LLM, A2A, Docker instances.
    NotPorted,
    /// A script service whose language this port does not run, such as Groovy.
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Authenticate {
    Required,
    Off,
    AnonymousAll,
    AnonymousView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SemaphoreMode {
    None,
    Fail,
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Required {
    True,
    False,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Include {
    Pk,
    NonPk,
    All,
}

/// `allow-html`. The default, `none`, rejects text with a `<`. `safe` cleans the markup, as Jsoup
/// does in Java, and `any` leaves it alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AllowHtml {
    #[default]
    None,
    Safe,
    Any,
}

fn allow_html_of(node: &MNode) -> AllowHtml {
    match node.attribute("allow-html").as_deref() {
        Some("any") => AllowHtml::Any,
        Some("safe") => AllowHtml::Safe,
        _ => AllowHtml::None,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AutoParams {
    pub allow_html: AllowHtml,
    pub entity_name: Option<String>,
    pub include: Include,
    pub exclude: Vec<String>,
    pub required: Required,
}

#[derive(Clone, Debug)]
pub(crate) enum Check {
    Matches {
        regexp: String,
        message: String,
    },
    NumberRange {
        min: Option<String>,
        min_eq: bool,
        max: Option<String>,
        max_eq: bool,
        message: Option<String>,
    },
    NumberInteger,
    NumberDecimal,
    TextLength {
        min: Option<usize>,
        max: Option<usize>,
    },
    TextEmail,
    TextUrl,
    TextLetters,
    TextDigits,
    TimeRange {
        after: Option<String>,
        before: Option<String>,
    },
    CreditCard {
        types: Option<String>,
    },
    And(Vec<Check>),
    Or(Vec<Check>),
    Not(Vec<Check>),
}

#[derive(Clone, Debug)]
pub(crate) struct ParamDef {
    pub name: String,
    pub java_type: String,
    pub required: Required,
    /// `default-value`: text with `${}` expanded against the other parameters, then converted.
    pub default_value: Option<String>,
    /// `default`: a JavaScript expression over the other parameters.
    pub default_expr: Option<String>,
    pub allow_html: AllowHtml,
    pub checks: Vec<Check>,
    pub auto: Option<AutoParams>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImplementRef {
    pub service: String,
    pub required: Option<bool>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceDef {
    pub name: String,
    pub verb: String,
    pub noun: String,
    pub kind: ServiceKind,
    /// The `type` attribute, the sub-type of the service's artifact hits.
    pub service_type: String,
    /// `no-remember-parameters`: the hit does not keep the parameters. An implemented service passes it on.
    pub no_remember_parameters: bool,
    pub location: String,
    /// `<actions>` XML, or empty when the service has none.
    pub actions: String,
    pub authenticate: Authenticate,
    pub authz_action: Option<String>,
    pub validate_input: bool,
    pub transaction: TransactionAttribute,
    pub timeout: Option<Duration>,
    pub semaphore: SemaphoreMode,
    pub semaphore_name: Option<String>,
    pub semaphore_timeout: Duration,
    pub semaphore_sleep: Duration,
    pub semaphore_ignore: Duration,
    pub semaphore_parameter: Option<String>,
    pub in_params: Vec<ParamDef>,
    pub out_params: Vec<ParamDef>,
    pub implements: Vec<ImplementRef>,
}

impl ServiceDef {
    pub(crate) fn implicit(name: &str, verb: &str, noun: &str) -> Self {
        Self {
            name: name.to_string(),
            verb: verb.to_string(),
            noun: noun.to_string(),
            kind: ServiceKind::EntityAuto,
            service_type: "entity-implicit".to_string(),
            no_remember_parameters: false,
            location: String::new(),
            actions: String::new(),
            authenticate: Authenticate::Required,
            authz_action: None,
            validate_input: true,
            transaction: TransactionAttribute::UseOrBegin,
            timeout: None,
            semaphore: SemaphoreMode::None,
            semaphore_name: None,
            semaphore_timeout: Duration::from_secs(120),
            semaphore_sleep: Duration::from_secs(5),
            semaphore_ignore: Duration::from_secs(3600),
            semaphore_parameter: None,
            in_params: Vec::new(),
            out_params: Vec::new(),
            implements: Vec::new(),
        }
    }

    pub(crate) fn semaphore_entity() -> &'static str {
        SEMAPHORE_ENTITY
    }
}

#[derive(Default)]
struct Registry {
    services: BTreeMap<String, ServiceDef>,
    failures: Vec<String>,
}

struct ServiceInclude {
    prefix: String,
    verb: String,
    noun: String,
    location: String,
}

type Shared = RwLock<Arc<Registry>>;

/// The entered context's factory and its registry, or an error when no context is entered.
fn entered() -> Result<Arc<Shared>, Error> {
    ExecutionContext::with_current(|ctx| registry_of(ctx.factory()))
        .ok_or_else(|| Error::new("no execution context is entered; service definitions belong to its factory"))
}

fn registry_of(factory: &Arc<ExecutionContextFactory>) -> Arc<Shared> {
    factory.extension::<Shared>(|_| RwLock::new(Arc::new(Registry::default())))
}

fn snapshot(shared: &Shared) -> Arc<Registry> {
    Arc::clone(&shared.read().unwrap_or_else(|err| err.into_inner()))
}

#[cfg(test)]
pub(crate) fn is_registered(name: &str) -> bool {
    find(name).is_some()
}

/// What a form field shows of an input parameter's rules: the classes the macros turn into input attributes.
pub(crate) struct ParamHints {
    pub classes: Vec<&'static str>,
    pub regexp: Option<(String, String)>,
}

/// The validation hints of input parameter `parameter` of `service`, as `getFieldValidationClasses` reads them.
pub(crate) fn input_hints(service: &str, parameter: &str) -> Option<ParamHints> {
    let def = find(service)?;
    let param = def.in_params.iter().find(|param| param.name == parameter)?;
    let mut classes = Vec::new();
    if param.required == Required::True {
        classes.push("required");
    }
    let mut regexp = None;
    for check in &param.checks {
        match check {
            Check::NumberInteger | Check::NumberDecimal if !classes.contains(&"number") => classes.push("number"),
            Check::TextEmail => classes.push("email"),
            Check::TextUrl => classes.push("url"),
            Check::TextDigits => classes.push("digits"),
            Check::CreditCard { .. } => classes.push("creditcard"),
            Check::Matches {
                regexp: pattern,
                message,
            } if regexp.is_none() => {
                regexp = Some((pattern.clone(), message.clone()));
            }
            _ => {}
        }
    }
    let java_type = param.java_type.as_str();
    if [
        "BigDecimal",
        "BigInteger",
        "Long",
        "Integer",
        "Double",
        "Float",
        "Number",
    ]
    .iter()
    .any(|suffix| java_type.ends_with(suffix))
        && !classes.contains(&"number")
    {
        classes.push("number");
    }
    Some(ParamHints { classes, regexp })
}

/// A service of the entered context's factory. `None` when it is not defined or no context is entered.
pub(crate) fn find(name: &str) -> Option<ServiceDef> {
    let shared = entered().ok()?;
    snapshot(&shared).services.get(name).cloned()
}

/// Every service registered with the entered context's factory, by name. Type generation reads these.
///
/// # Errors
///
/// Returns an error when no context is entered.
pub(crate) fn registered() -> Result<Vec<ServiceDef>, Error> {
    Ok(snapshot(&*entered()?).services.values().cloned().collect())
}

/// The services `paths` define, and the registration failures, without registering them.
///
/// # Errors
///
/// Returns an error when a file cannot be read.
#[cfg(test)]
pub(crate) fn definitions_in(paths: &[PathBuf]) -> Result<(Vec<ServiceDef>, Vec<String>), Error> {
    let registry = build(paths)?;
    Ok((registry.services.into_values().collect(), registry.failures))
}

/// Register the configured service files with the entered context's factory.
///
/// # Errors
///
/// Returns an error when no context is entered or a file cannot be read.
pub fn load_definitions() -> Result<(), Error> {
    lint(&[])?;
    Ok(())
}

/// Register `paths`, or the configured service files when `paths` is empty, with the entered
/// context's factory, replacing what it had. Returns the registration failures.
///
/// # Errors
///
/// Returns an error when no context is entered or a file cannot be read.
pub fn lint(paths: &[PathBuf]) -> Result<Vec<String>, Error> {
    if paths.is_empty() {
        let configured = configured_paths()?;
        install(&configured)?;
    } else {
        install(paths)?;
    }
    Ok(snapshot(&*entered()?).failures.clone())
}

/// Registration failures for `paths`, or the configured service files when `paths` is empty,
/// without changing the services that are registered. This is what `moqui lint` runs.
///
/// # Errors
///
/// Returns an error when a file cannot be read.
pub fn check(paths: &[PathBuf]) -> Result<Vec<String>, Error> {
    let configured;
    let paths = if paths.is_empty() {
        configured = configured_paths()?;
        &configured[..]
    } else {
        paths
    };
    Ok(build(paths)?.failures)
}

fn install(paths: &[PathBuf]) -> Result<(), Error> {
    let registry = build(paths)?;
    let shared = entered()?;
    *shared.write().unwrap_or_else(|err| err.into_inner()) = Arc::new(registry);
    Ok(())
}

fn build(paths: &[PathBuf]) -> Result<Registry, Error> {
    let mut services = BTreeMap::new();
    let mut failures = Vec::new();
    let mut includes = Vec::new();
    let mut loaded = BTreeSet::new();
    let mut pending: Vec<PathBuf> = paths.to_vec();
    while let Some(path) = pending.pop() {
        load_file(
            &path,
            &mut loaded,
            &mut services,
            &mut failures,
            &mut includes,
            &mut pending,
        )?;
    }
    // Includes first, so a service can implement a name that is only an include alias.
    resolve_includes(&includes, &mut services, &mut failures);
    resolve_implements(&mut services, &mut failures);
    Ok(Registry { services, failures })
}

fn load_file(
    path: &Path,
    loaded: &mut BTreeSet<PathBuf>,
    services: &mut BTreeMap<String, ServiceDef>,
    failures: &mut Vec<String>,
    includes: &mut Vec<ServiceInclude>,
    pending: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    let path = normalize(path);
    if !loaded.insert(path.clone()) {
        return Ok(());
    }
    let root = MNode::parse_file(&path)?;
    if root.name() != "services" {
        failures.push(format!("{} root is {} and not services", path.display(), root.name()));
        return Ok(());
    }
    let prefix = service_prefix(&path);
    for child in root.children() {
        match child.name().as_str() {
            "service" => load_service(&prefix, &child, services, failures),
            "service-include" => {
                let verb = child.attribute("verb").unwrap_or_default();
                let noun = child.attribute("noun").unwrap_or_default();
                let location = child.attribute("location").unwrap_or_default();
                if location.is_empty() {
                    failures.push(format!(
                        "service-include {}.{} is missing a location",
                        prefix,
                        service_tail(&verb, &noun)
                    ));
                    continue;
                }
                let included = resolve_location(&location)?;
                pending.push(included);
                includes.push(ServiceInclude {
                    prefix: prefix.clone(),
                    verb,
                    noun,
                    location,
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn load_service(prefix: &str, node: &MNode, services: &mut BTreeMap<String, ServiceDef>, failures: &mut Vec<String>) {
    let Some(verb) = node.attribute("verb") else {
        failures.push(format!("service in {prefix} is missing verb"));
        return;
    };
    let noun = node.attribute("noun").unwrap_or_default();
    let name = service_name(prefix, &verb, &noun);
    let type_name = node.attribute("type").unwrap_or_else(|| "inline".to_string());
    if type_name == "remote-json-rpc" {
        failures.push(format!("service {name} JSON-RPC is not included"));
        return;
    }
    if type_name == "java" {
        failures.push(format!("service {name} has no Java runner"));
        return;
    }
    if type_name == "camel" {
        failures.push(format!("service {name} type camel is not included"));
        return;
    }
    if !matches!(
        type_name.as_str(),
        "inline" | "entity-auto" | "script" | "interface" | "remote-rest"
    ) {
        failures.push(format!("service {name} has unknown type {type_name}"));
        return;
    }
    // A service that is not available keeps its definition, so a call names why; its body is not compiled.
    let not_ported = type_name != "interface" && super::not_ported::runs(&name);
    let mut expression_errors = Vec::new();
    if !not_ported && !expressions_ok(&name, node, &mut expression_errors) {
        failures.extend(expression_errors);
        return;
    }
    // The generated script is what runs, so it is what must parse. This reaches every attribute
    // that becomes JavaScript, not only the ones named in `is_expression_attr`.
    if let Some(actions) = node.first_named("actions").filter(|_| !not_ported) {
        if let Err(err) = crate::script::check_actions(&actions.write_string()) {
            failures.push(format!("service {name} {err}"));
            return;
        }
    }
    let location = node.attribute("location").unwrap_or_default();
    let kind = if not_ported {
        ServiceKind::NotPorted
    } else if super::tools::runs(&name) {
        ServiceKind::Tools
    } else if super::system_sync::runs(&name) {
        ServiceKind::SystemSync
    } else if super::elfinder::runs(&name) {
        ServiceKind::ElFinder
    } else {
        kind_of(&type_name, &location)
    };
    let def = ServiceDef {
        name: name.clone(),
        verb,
        noun,
        kind,
        service_type: type_name,
        no_remember_parameters: bool_attr(node, "no-remember-parameters", false),
        location,
        actions: node
            .first_named("actions")
            .map(|node| node.write_string())
            .unwrap_or_default(),
        authenticate: authenticate_of(node),
        authz_action: node.attribute("authz-action"),
        validate_input: bool_attr(node, "validate", true),
        transaction: transaction_of(node),
        timeout: timeout_of(node),
        semaphore: semaphore_of(node),
        semaphore_name: node.attribute("semaphore-name"),
        semaphore_timeout: seconds_attr(node, "semaphore-timeout", 120),
        semaphore_sleep: seconds_attr(node, "semaphore-sleep", 5),
        semaphore_ignore: seconds_attr(node, "semaphore-ignore", 3600),
        semaphore_parameter: node.attribute("semaphore-parameter"),
        in_params: node
            .first_named("in-parameters")
            .map(|node| parse_param_list(&node))
            .unwrap_or_default(),
        out_params: node
            .first_named("out-parameters")
            .map(|node| parse_param_list(&node))
            .unwrap_or_default(),
        implements: node
            .children_named("implements")
            .iter()
            .filter_map(|child| {
                let service = child.attribute("service")?;
                let required = match child.attribute("required").as_deref() {
                    Some(value) if value.eq_ignore_ascii_case("true") => Some(true),
                    Some(value) if value.eq_ignore_ascii_case("false") => Some(false),
                    _ => None,
                };
                Some(ImplementRef { service, required })
            })
            .collect(),
    };
    services.insert(name, def);
}

fn kind_of(type_name: &str, location: &str) -> ServiceKind {
    match type_name {
        "entity-auto" => ServiceKind::EntityAuto,
        "script" if is_groovy_location(location) && is_mail_location(location) => ServiceKind::Mail,
        "script" if is_groovy_location(location) => ServiceKind::Unsupported,
        "script" => ServiceKind::Script,
        "interface" => ServiceKind::Interface,
        "remote-rest" => ServiceKind::RemoteRest,
        _ => ServiceKind::Inline,
    }
}

/// The framework's email scripts are Groovy until the mail transport lands; they report that mail
/// is not configured. Any other Groovy script is simply not runnable.
fn is_mail_location(location: &str) -> bool {
    location.to_ascii_lowercase().contains("email")
}

fn is_groovy_location(location: &str) -> bool {
    let path = location.split(['?', '#']).next().unwrap_or(location);
    path.to_ascii_lowercase().ends_with(".groovy")
}

fn authenticate_of(node: &MNode) -> Authenticate {
    match node.attribute("authenticate").as_deref() {
        Some("false") => Authenticate::Off,
        Some("anonymous-all") => Authenticate::AnonymousAll,
        Some("anonymous-view") => Authenticate::AnonymousView,
        _ => Authenticate::Required,
    }
}

fn transaction_of(node: &MNode) -> TransactionAttribute {
    match node.attribute("transaction").as_deref() {
        Some("ignore") => TransactionAttribute::Ignore,
        Some("force-new") => TransactionAttribute::ForceNew,
        Some("cache") => TransactionAttribute::Cache,
        Some("force-cache") => TransactionAttribute::ForceCache,
        _ => TransactionAttribute::UseOrBegin,
    }
}

fn timeout_of(node: &MNode) -> Option<Duration> {
    let text = node.attribute("transaction-timeout")?;
    let seconds = text.parse::<u64>().ok()?;
    if seconds == 0 {
        None
    } else {
        Some(Duration::from_secs(seconds))
    }
}

fn semaphore_of(node: &MNode) -> SemaphoreMode {
    match node.attribute("semaphore").as_deref() {
        Some("fail") => SemaphoreMode::Fail,
        Some("wait") => SemaphoreMode::Wait,
        _ => SemaphoreMode::None,
    }
}

fn seconds_attr(node: &MNode, name: &str, default: u64) -> Duration {
    let seconds = node
        .attribute(name)
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(default);
    Duration::from_secs(seconds)
}

fn bool_attr(node: &MNode, name: &str, default: bool) -> bool {
    match node.attribute(name).as_deref() {
        Some(value) if value.eq_ignore_ascii_case("true") => true,
        Some(value) if value.eq_ignore_ascii_case("false") => false,
        _ => default,
    }
}

fn expressions_ok(service: &str, node: &MNode, errors: &mut Vec<String>) -> bool {
    let mut ok = true;
    if node.name() == "script" {
        if let Some(text) = node.text() {
            if let Err(err) = check_javascript(service, &text) {
                errors.push(err);
                ok = false;
            }
        }
    }
    for (key, value) in node.attributes() {
        if is_expression_attr(&key) {
            if let Err(err) = check_javascript(service, &value) {
                errors.push(err);
                ok = false;
            }
        }
    }
    for child in node.children() {
        if !expressions_ok(service, &child, errors) {
            ok = false;
        }
    }
    ok
}

fn is_expression_attr(key: &str) -> bool {
    matches!(key, "from" | "condition" | "default" | "in-map")
}

fn check_javascript(service: &str, text: &str) -> Result<(), String> {
    crate::script::parse_js(text).map_err(|err| format!("service {service} {err}"))
}

fn parse_param_list(node: &MNode) -> Vec<ParamDef> {
    let mut params = Vec::new();
    for child in node.children() {
        match child.name().as_str() {
            "parameter" => params.push(parse_parameter(&child)),
            "auto-parameters" => params.push(parse_auto(&child)),
            _ => {}
        }
    }
    params
}

fn parse_parameter(node: &MNode) -> ParamDef {
    ParamDef {
        name: node.attribute("name").unwrap_or_default(),
        java_type: node.attribute("type").unwrap_or_else(|| "String".to_string()),
        required: required_of(node, Required::False),
        default_value: node.attribute("default-value"),
        default_expr: node.attribute("default").filter(|text| !text.is_empty()),
        allow_html: allow_html_of(node),
        checks: parse_checks(node),
        auto: None,
    }
}

fn parse_auto(node: &MNode) -> ParamDef {
    let include = match node.attribute("include").as_deref() {
        Some("pk") => Include::Pk,
        Some("nonpk") => Include::NonPk,
        _ => Include::All,
    };
    let exclude = node
        .children_named("exclude")
        .iter()
        .filter_map(|child| child.attribute("field-name"))
        .collect();
    let required = required_of(node, Required::False);
    ParamDef {
        name: String::new(),
        java_type: "String".to_string(),
        required,
        default_value: None,
        default_expr: None,
        allow_html: AllowHtml::None,
        checks: Vec::new(),
        auto: Some(AutoParams {
            allow_html: allow_html_of(node),
            entity_name: node.attribute("entity-name"),
            include,
            exclude,
            required,
        }),
    }
}

fn required_of(node: &MNode, default: Required) -> Required {
    match node.attribute("required").as_deref() {
        Some("true") => Required::True,
        Some("false") => Required::False,
        Some("disabled") => Required::Disabled,
        _ => default,
    }
}

fn parse_checks(node: &MNode) -> Vec<Check> {
    let mut checks = Vec::new();
    for child in node.children() {
        if let Some(check) = parse_check(&child) {
            checks.push(check);
        }
    }
    checks
}

fn parse_check(node: &MNode) -> Option<Check> {
    let check = match node.name().as_str() {
        "matches" => Check::Matches {
            regexp: node.attribute("regexp").unwrap_or_default(),
            message: node.attribute("message").unwrap_or_default(),
        },
        "number-range" => Check::NumberRange {
            min: node.attribute("min"),
            min_eq: bool_attr(node, "min-include-equals", true),
            max: node.attribute("max"),
            max_eq: bool_attr(node, "max-include-equals", false),
            message: node.attribute("message"),
        },
        "number-integer" => Check::NumberInteger,
        "number-decimal" => Check::NumberDecimal,
        "text-length" => Check::TextLength {
            min: usize_attr(node, "min"),
            max: usize_attr(node, "max"),
        },
        "text-email" => Check::TextEmail,
        "text-url" => Check::TextUrl,
        "text-letters" => Check::TextLetters,
        "text-digits" => Check::TextDigits,
        "time-range" => Check::TimeRange {
            after: node.attribute("after"),
            before: node.attribute("before"),
        },
        "credit-card" => Check::CreditCard {
            types: node.attribute("types"),
        },
        "val-or" => Check::Or(parse_checks(node)),
        "val-and" => Check::And(parse_checks(node)),
        "val-not" => Check::Not(parse_checks(node)),
        _ => return None,
    };
    Some(check)
}

fn usize_attr(node: &MNode, name: &str) -> Option<usize> {
    node.attribute(name).and_then(|text| text.parse().ok())
}

fn resolve_implements(services: &mut BTreeMap<String, ServiceDef>, failures: &mut Vec<String>) {
    let names: Vec<String> = services.keys().cloned().collect();
    let mut done = BTreeSet::new();
    for name in names {
        let mut stack = Vec::new();
        resolve_one(&name, services, failures, &mut done, &mut stack);
    }
}

fn resolve_one(
    name: &str,
    services: &mut BTreeMap<String, ServiceDef>,
    failures: &mut Vec<String>,
    done: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
) {
    if done.contains(name) || !services.contains_key(name) {
        return;
    }
    if stack.iter().any(|item| item == name) {
        failures.push(format!("service {name} implements itself"));
        services.remove(name);
        return;
    }
    stack.push(name.to_string());
    let implements = services.get(name).map(|def| def.implements.clone()).unwrap_or_default();
    for implement in implements {
        if !services.contains_key(&implement.service) {
            failures.push(format!(
                "service {name} implements unknown service {}",
                implement.service
            ));
            services.remove(name);
            stack.pop();
            return;
        }
        resolve_one(&implement.service, services, failures, done, stack);
        if !services.contains_key(name) || !services.contains_key(&implement.service) {
            if services.contains_key(name) {
                failures.push(format!(
                    "service {name} implements unknown service {}",
                    implement.service
                ));
                services.remove(name);
            }
            stack.pop();
            return;
        }
        let (inherited_in, inherited_out, no_remember) = {
            let target = services.get(&implement.service).expect("implemented service");
            (
                target.in_params.clone(),
                target.out_params.clone(),
                target.no_remember_parameters,
            )
        };
        let def = services.get_mut(name).expect("service being resolved");
        def.no_remember_parameters |= no_remember;
        merge_params(&mut def.in_params, inherited_in, implement.required);
        merge_params(&mut def.out_params, inherited_out, implement.required);
    }
    done.insert(name.to_string());
    stack.pop();
}

fn merge_params(dest: &mut Vec<ParamDef>, extra: Vec<ParamDef>, required: Option<bool>) {
    for mut param in extra {
        if param.required == Required::Disabled {
            continue;
        }
        if let Some(flag) = required {
            param.required = if flag { Required::True } else { Required::False };
        }
        let present = !param.name.is_empty() && dest.iter().any(|have| have.name == param.name);
        if !present {
            dest.push(param);
        }
    }
}

fn resolve_includes(
    includes: &[ServiceInclude],
    services: &mut BTreeMap<String, ServiceDef>,
    failures: &mut Vec<String>,
) {
    let mut aliases = Vec::new();
    for include in includes {
        let local = service_name(&include.prefix, &include.verb, &include.noun);
        match follow_include(include, &[], 0) {
            Ok(target) => {
                if services.contains_key(&target) {
                    aliases.push((local, target));
                } else {
                    failures.push(format!("service {local} include did not resolve to {target}"));
                }
            }
            Err(err) => failures.push(err),
        }
    }
    for (local, target) in aliases {
        if services.contains_key(&local) {
            continue;
        }
        let Some(def) = services.get(&target).cloned() else {
            continue;
        };
        let mut alias = def;
        alias.name = local.clone();
        services.insert(local, alias);
    }
}

fn follow_include(include: &ServiceInclude, stack: &[PathBuf], depth: usize) -> Result<String, String> {
    if depth > 32 {
        return Err(format!(
            "service-include {} is too deep",
            service_name(&include.prefix, &include.verb, &include.noun)
        ));
    }
    let path = resolve_location(&include.location).map_err(|err| err.to_string())?;
    let path = normalize(&path);
    if stack.iter().any(|seen| seen == &path) {
        return Err(format!(
            "service-include {} is circular",
            service_name(&include.prefix, &include.verb, &include.noun)
        ));
    }
    let root = MNode::parse_file(&path).map_err(|err| err.to_string())?;
    let Some(node) = find_member(&root, &include.verb, &include.noun) else {
        return Err(format!(
            "service-include {} was not found in {}",
            service_name(&include.prefix, &include.verb, &include.noun),
            path.display()
        ));
    };
    if node.name() == "service-include" {
        let location = node.attribute("location").unwrap_or_default();
        let next = ServiceInclude {
            prefix: service_prefix(&path),
            verb: include.verb.clone(),
            noun: include.noun.clone(),
            location,
        };
        let mut stack = stack.to_vec();
        stack.push(path);
        return follow_include(&next, &stack, depth + 1);
    }
    let verb = node.attribute("verb").unwrap_or_else(|| include.verb.clone());
    let noun = node.attribute("noun").unwrap_or_default();
    Ok(service_name(&service_prefix(&path), &verb, &noun))
}

fn find_member(root: &MNode, verb: &str, noun: &str) -> Option<MNode> {
    root.children().into_iter().find(|child| {
        let name = child.name();
        if name != "service" && name != "service-include" {
            return false;
        }
        let child_verb = child.attribute("verb").unwrap_or_default();
        let child_noun = child.attribute("noun").unwrap_or_default();
        if noun.is_empty() {
            format!("{child_verb}{child_noun}") == verb || (child_verb == verb && child_noun.is_empty())
        } else {
            child_verb == verb && child_noun == noun
        }
    })
}

fn service_prefix(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let Some(index) = text.rfind("/service/") else {
        return String::new();
    };
    let rest = &text[index + "/service/".len()..];
    let rest = rest.strip_suffix(".xml").unwrap_or(rest);
    if rest.is_empty() {
        String::new()
    } else {
        rest.replace('/', ".")
    }
}

fn service_tail(verb: &str, noun: &str) -> String {
    if noun.is_empty() {
        verb.to_string()
    } else {
        format!("{verb}#{noun}")
    }
}

fn service_name(prefix: &str, verb: &str, noun: &str) -> String {
    let tail = service_tail(verb, noun);
    if prefix.is_empty() {
        tail
    } else {
        format!("{prefix}.{tail}")
    }
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn configured_paths() -> Result<Vec<PathBuf>, Error> {
    let conf = framework_dir().join("conf/MoquiDefaultConf.xml");
    let root = MNode::parse_file(&conf)?;
    let facade = root
        .first_named("service-facade")
        .ok_or_else(|| Error::new("service-facade is missing"))?;
    let mut paths = Vec::new();
    for node in facade.children_named("service-file") {
        let Some(location) = node.attribute("location") else {
            continue;
        };
        if location.is_empty() {
            continue;
        }
        paths.push(resolve_location(&location)?);
    }
    Ok(paths)
}

/// `classpath://` and `component://` locations. Copied from the private entity loader.
fn resolve_location(location: &str) -> Result<PathBuf, Error> {
    crate::context::locate(location).map_err(|err| Error::new(err.message().to_string()))
}

fn framework_dir() -> PathBuf {
    crate::context::framework_dir()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn groovy_leftover_fails_that_service_and_keeps_the_neighbor() {
        let ctx = crate::context::ExecutionContextFactory::test_context();
        let _entered = ctx.enter();
        let path = write_fixture(
            "groovy",
            r#"<services>
                <service verb="groovy" noun="Broken">
                    <actions><set field="x" from="a ?: b"/></actions>
                </service>
                <service verb="plain" noun="Neighbor">
                    <in-parameters><parameter name="ok" default-value="yes"/></in-parameters>
                </service>
            </services>"#,
        );
        let failures = super::lint(&[path]).unwrap();
        let text = failures.join("\n");
        assert!(text.contains("groovy#Broken"), "{text}");
        assert!(text.contains("?:"), "{text}");
        assert!(super::is_registered("plain#Neighbor"), "{text}");
        assert!(!super::is_registered("groovy#Broken"), "{text}");
    }

    #[test]
    fn remote_json_rpc_is_not_registered() {
        let ctx = crate::context::ExecutionContextFactory::test_context();
        let _entered = ctx.enter();
        let path = write_fixture(
            "rpc",
            r#"<services>
                <service verb="remote" noun="Rpc" type="remote-json-rpc"/>
                <service verb="plain" noun="Neighbor"/>
            </services>"#,
        );
        let failures = super::lint(&[path]).unwrap();
        let text = failures.join("\n");
        assert!(text.contains("JSON-RPC"), "{text}");
        assert!(!super::is_registered("remote#Rpc"));
        assert!(super::is_registered("plain#Neighbor"));
    }

    #[test]
    fn load_definitions_registers_email_services() {
        let ctx = crate::context::ExecutionContextFactory::test_context_in(
            "",
            crate::service::ServicePools::idle(),
            crate::context::framework_dir(),
        );
        let _entered = ctx.enter();
        super::load_definitions().unwrap();
        assert!(
            super::is_registered("org.moqui.EmailServices.send#EmailTemplate"),
            "{}",
            explain("Email")
        );
        assert!(
            super::is_registered("org.moqui.impl.EmailServices.poll#EmailServer"),
            "{}",
            explain("poll")
        );
    }

    fn explain(needle: &str) -> String {
        super::snapshot(&super::entered().unwrap())
            .failures
            .iter()
            .filter(|line| line.contains(needle))
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn write_fixture(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("moqui-svc-{name}-{}.xml", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }
}
