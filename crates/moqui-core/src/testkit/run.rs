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

//! Run one spec file in the entered execution context: rewrite it, evaluate it so its `spec()` and `feature()` calls
//! register, then drive `setupSpec`, each feature (each `where` row), and `cleanupSpec` from here, one script call
//! each, so timing, timeouts, `ec.message` errors and the state left between features are handled in one place.

use std::sync::Arc;
use std::time::{Duration, Instant};

use moqui_lib::object::to_plain_string;
use rquickjs::{Ctx, Exception, Function, Object};
use serde::Deserialize;

use super::rewrite::rewrite_spec;
use crate::context::{Error, ExecutionContext, ExecutionContextFactory};

const PRELUDE: &str = include_str!("prelude.js");
const PRELUDE_NAME: &str = "moqui-test-prelude.js";

/// A spec file to run.
#[derive(Clone, Debug)]
pub struct SpecFile {
    /// The spec name, the file name without `.spec.js`.
    pub name: String,
    /// The file as reports show it, such as `mantle-usl/test/OrderToCashBasicFlow.spec.js`; stack traces name it.
    pub display: String,
    pub source: String,
}

/// Why a feature, or a spec phase, did not pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// `assertion` (a condition was false), `error` (a block threw), `messages` (`ec.message` errors were left), or
    /// `registration` (the file did not load, or a `spec()` or `feature()` is wrong).
    pub kind: String,
    /// The block that failed, such as `when` or `setupSpec`.
    pub block: String,
    /// The power-assert diagram, or the error.
    pub message: String,
    /// `file:line` of the failure in the spec file, when known.
    pub location: Option<String>,
    /// The stack of an error, for the full report.
    pub stack: String,
    /// The `ec.message` errors left by the feature and the data-check messages, for the report.
    pub details: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed(Failure),
    /// `ignore: "reason"` on the feature.
    Ignored(String),
    /// Not run: a stepwise spec after a failure, or a failed `setupSpec`.
    Skipped(String),
}

/// One feature, or one `where` row of it.
#[derive(Clone, Debug)]
pub struct CaseResult {
    pub name: String,
    pub outcome: Outcome,
    pub duration: Duration,
}

#[derive(Clone, Debug)]
pub struct SpecResult {
    pub name: String,
    pub display: String,
    pub cases: Vec<CaseResult>,
    /// The file did not load, or `setupSpec` failed.
    pub setup_failure: Option<Failure>,
    pub cleanup_failure: Option<Failure>,
    pub duration: Duration,
}

impl SpecResult {
    pub fn failed(&self) -> bool {
        self.setup_failure.is_some()
            || self.cleanup_failure.is_some()
            || self.cases.iter().any(|case| matches!(case.outcome, Outcome::Failed(_)))
    }
}

/// What to run of a spec.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// Run only the features whose name contains this text.
    pub feature_filter: Option<String>,
}

#[derive(Deserialize)]
struct Plan {
    errors: Vec<String>,
    spec: SpecPlan,
    cases: Vec<CasePlan>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecPlan {
    stepwise: bool,
    allow_message_errors: bool,
    timeout: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CasePlan {
    name: String,
    ignore: Option<String>,
    timeout: Option<f64>,
    allow_message_errors: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptFailure {
    kind: String,
    block: String,
    message: String,
    line: Option<u32>,
    #[serde(default)]
    stack: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockResult {
    failure: Option<ScriptFailure>,
    data_errors: Vec<String>,
}

/// Run a spec in the execution context entered on this thread. `on_case` hears each feature as it finishes.
///
/// The context is the spec's own: the effective time, the login and the authorization the spec leaves are reset
/// after it.
pub fn run_spec(spec: &SpecFile, options: &RunOptions, on_case: &mut dyn FnMut(&CaseResult)) -> SpecResult {
    let started = Instant::now();
    let mut result = SpecResult {
        name: spec.name.clone(),
        display: spec.display.clone(),
        cases: Vec::new(),
        setup_failure: None,
        cleanup_failure: None,
        duration: Duration::ZERO,
    };
    let plan = match load(spec) {
        Ok(plan) => plan,
        Err(failure) => {
            result.setup_failure = Some(*failure);
            reset_context();
            result.duration = started.elapsed();
            return result;
        }
    };
    let setup = block(
        spec,
        "__moquiTest.runSpecPhase('setupSpec')",
        plan.spec.timeout,
        "setupSpec",
    );
    clear_messages();
    let mut skip_rest = setup.as_ref().err().map(|_| "setupSpec failed".to_string());
    if let Err(failure) = setup {
        result.setup_failure = Some(*failure);
    }
    for (index, case) in plan.cases.iter().enumerate() {
        if let Some(filter) = &options.feature_filter {
            if !case.name.contains(filter.as_str()) {
                continue;
            }
        }
        let case_started = Instant::now();
        let outcome = if let Some(reason) = &skip_rest {
            Outcome::Skipped(reason.clone())
        } else if let Some(reason) = &case.ignore {
            Outcome::Ignored(reason.clone())
        } else {
            let timeout = case.timeout.or(plan.spec.timeout);
            let ran = block(spec, &format!("__moquiTest.runCase({index})"), timeout, "feature");
            let errors = take_message_errors();
            let allow = case.allow_message_errors || plan.spec.allow_message_errors;
            match ran {
                Ok(()) if errors.is_empty() || allow => Outcome::Passed,
                Ok(()) => Outcome::Failed(Failure {
                    kind: "messages".to_string(),
                    block: "feature".to_string(),
                    message: format!("the feature left {} error message(s) in ec.message", errors.len()),
                    location: None,
                    stack: String::new(),
                    details: errors,
                }),
                Err(mut failure) => {
                    failure.details.extend(errors);
                    Outcome::Failed(*failure)
                }
            }
        };
        if plan.spec.stepwise && matches!(outcome, Outcome::Failed(_)) {
            skip_rest = Some("an earlier feature of this stepwise spec failed".to_string());
        }
        let case_result = CaseResult {
            name: case.name.clone(),
            outcome,
            duration: case_started.elapsed(),
        };
        on_case(&case_result);
        result.cases.push(case_result);
    }
    if let Err(failure) = block(
        spec,
        "__moquiTest.runSpecPhase('cleanupSpec')",
        plan.spec.timeout,
        "cleanupSpec",
    ) {
        result.cleanup_failure = Some(*failure);
    }
    reset_context();
    result.duration = started.elapsed();
    result
}

/// The `dependsOn` names of a spec, read from its `spec({ dependsOn: [...] })` call without running it.
pub fn spec_depends_on(source: &str) -> Vec<String> {
    super::rewrite::spec_depends_on(source)
}

fn registration(message: impl Into<String>) -> Box<Failure> {
    Box::new(Failure {
        kind: "registration".to_string(),
        block: "load".to_string(),
        message: message.into(),
        location: None,
        stack: String::new(),
        details: Vec::new(),
    })
}

fn load(spec: &SpecFile) -> Result<Plan, Box<Failure>> {
    crate::script::clear_context_stack();
    install_prelude().map_err(|err| registration(format!("the test prelude did not load: {err}")))?;
    let rewritten = rewrite_spec(&spec.source).map_err(|err| registration(format!("{}: {err}", spec.display)))?;
    let file = serde_json::to_string(&spec.display).unwrap_or_default();
    crate::script::eval_named_value(&format!("__moquiTest.reset({file})"), PRELUDE_NAME)
        .map_err(|err| registration(err.to_string()))?;
    // The wrapper opens on the file's first line, so a line in an error is the file's.
    let wrapped = format!("(function(){{{}\n}})();", rewritten.source);
    crate::script::eval_named_value(&wrapped, &spec.display).map_err(|err| registration(err.to_string()))?;
    let conditions = serde_json::to_string(&rewritten.conditions).unwrap_or_else(|_| "[]".to_string());
    crate::script::eval_named_value(&format!("__moquiTest.setConditions({conditions})"), PRELUDE_NAME)
        .map_err(|err| registration(err.to_string()))?;
    let plan = crate::script::eval_named_value("__moquiTest.plan()", PRELUDE_NAME)
        .map_err(|err| registration(err.to_string()))?;
    let plan: Plan = serde_json::from_str(&to_plain_string(&plan))
        .map_err(|err| registration(format!("the spec plan did not read: {err}")))?;
    if !plan.errors.is_empty() {
        return Err(registration(plan.errors.join("\n")));
    }
    Ok(plan)
}

/// The prelude and the host calls it uses, in this thread's outermost engine.
fn install_prelude() -> Result<(), Error> {
    crate::script::eval_named_value(PRELUDE, PRELUDE_NAME)?;
    crate::script::with_ctx(|ctx| install_host(ctx).map_err(|err| Error::new(err.to_string())))
}

fn install_host(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let host = Object::new(ctx.clone())?;
    host.set("parallel", Function::new(ctx.clone(), parallel_host)?)?;
    // A plain set of a global goes through the context stack's proxy, which takes only data.
    let define: Function = ctx.eval(
        "(host) => Object.defineProperty(globalThis, '__moquiTestHost', { value: host, writable: true, configurable: true })",
    )?;
    define.call::<_, rquickjs::Value>((host,)).map(|_| ())
}

/// Run one block call and read its result. The deadline, when given, stops a block that runs too long.
fn block(spec: &SpecFile, call: &str, timeout_seconds: Option<f64>, block_name: &str) -> Result<(), Box<Failure>> {
    let _deadline = timeout_seconds
        .filter(|seconds| *seconds > 0.0)
        .map(|seconds| crate::script::DeadlineGuard::new(Some(Duration::from_secs_f64(seconds))));
    let value = crate::script::eval_named_value(call, PRELUDE_NAME).map_err(|err| {
        Box::new(Failure {
            kind: "error".to_string(),
            block: block_name.to_string(),
            message: err.to_string(),
            location: None,
            stack: String::new(),
            details: Vec::new(),
        })
    })?;
    let parsed: BlockResult = serde_json::from_str(&to_plain_string(&value)).map_err(|err| {
        Box::new(Failure {
            kind: "error".to_string(),
            block: block_name.to_string(),
            message: format!("the block result did not read: {err}"),
            location: None,
            stack: String::new(),
            details: Vec::new(),
        })
    })?;
    match parsed.failure {
        None => Ok(()),
        Some(failure) => Err(Box::new(Failure {
            kind: failure.kind,
            block: failure.block,
            message: failure.message.replace(PRELUDE_NAME, "harness"),
            location: failure.line.map(|line| format!("{}:{line}", spec.display)),
            stack: failure.stack,
            details: parsed
                .data_errors
                .into_iter()
                .map(|error| format!("data check: {error}"))
                .collect(),
        })),
    }
}

fn take_message_errors() -> Vec<String> {
    ExecutionContext::with_current(|ec| {
        let errors = ec.message().errors().to_vec();
        ec.message().clear();
        errors
    })
    .unwrap_or_default()
}

fn clear_messages() {
    ExecutionContext::with_current(|ec| ec.message().clear());
}

/// What a spec leaves on its context does not reach the next: the effective time, the login, a script's
/// `disableAuthz`, messages and queued async work.
fn reset_context() {
    ExecutionContext::with_current(|ec| {
        ec.set_effective_time(None);
        ec.script_enable_authz();
        ec.message().clear();
    });
    if let Err(err) = crate::user::logout() {
        tracing::debug!("logout after a spec: {err}");
    }
    let factory = ExecutionContext::with_current(|ec| Arc::clone(ec.factory()));
    if let Some(factory) = factory {
        if !factory.wait_worker_pool_empty(50) {
            tracing::warn!("Async service work is still running 5 seconds after the spec");
        }
    }
    crate::script::clear_context_stack();
}

/// `__moquiTestHost.parallel(n, source, argsJson, transaction, timeoutSeconds)`: run the function source on `n`
/// threads, each with its own execution context and engine, and return `{results, errors}` as JSON.
fn parallel_host(
    ctx: Ctx<'_>,
    count: u32,
    source: String,
    args: String,
    transaction: bool,
    timeout_seconds: f64,
) -> rquickjs::Result<String> {
    let factory = ExecutionContext::with_current(|ec| Arc::clone(ec.factory()))
        .ok_or_else(|| Exception::throw_message(&ctx, "parallel needs an execution context"))?;
    let timeout = Duration::from_secs_f64(timeout_seconds.max(1.0));
    Ok(run_parallel(&factory, count, &source, &args, transaction, timeout))
}

fn run_parallel(
    factory: &Arc<ExecutionContextFactory>,
    count: u32,
    source: &str,
    args: &str,
    transaction: bool,
    timeout: Duration,
) -> String {
    let (sender, receiver) = std::sync::mpsc::channel();
    for thread_num in 0..count {
        let factory = Arc::clone(factory);
        let thread_sender = sender.clone();
        let source = source.to_string();
        let args = args.to_string();
        let spawned = std::thread::Builder::new()
            .name(format!("moqui-test-parallel-{thread_num}"))
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let out = run_thread(&factory, thread_num, &source, &args, transaction);
                let _ = thread_sender.send((thread_num, out));
            });
        if let Err(err) = spawned {
            let _ = sender.send((thread_num, Err(format!("thread {thread_num} did not start: {err}"))));
        }
    }
    drop(sender);
    let deadline = Instant::now() + timeout;
    let mut results: Vec<Option<serde_json::Value>> = vec![None; count as usize];
    let mut errors = Vec::new();
    for _ in 0..count {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok((thread_num, Ok(json))) => {
                results[thread_num as usize] = Some(serde_json::from_str(&json).unwrap_or(serde_json::Value::Null));
            }
            Ok((thread_num, Err(err))) => errors.push(format!("thread {thread_num}: {err}")),
            Err(_) => {
                errors.push(format!("threads still running after {}s", timeout.as_secs()));
                break;
            }
        }
    }
    serde_json::json!({
        "results": results.into_iter().map(|result| result.unwrap_or(serde_json::Value::Null)).collect::<Vec<_>>(),
        "errors": errors,
    })
    .to_string()
}

fn run_thread(
    factory: &Arc<ExecutionContextFactory>,
    thread_num: u32,
    source: &str,
    args: &str,
    transaction: bool,
) -> Result<String, String> {
    let context = factory.new_context();
    let _entered = context.enter();
    crate::script::clear_context_stack();
    let body = || -> Result<String, moqui_lib::error::Error> {
        install_prelude()?;
        let code = format!("JSON.stringify((({source}))({thread_num}, {args}) ?? null)");
        let value = crate::script::eval_named_value(&code, "parallel")?;
        Ok(to_plain_string(&value))
    };
    let out = if transaction {
        match factory.entity() {
            Some(facade) => match facade.inner.groups.get(&facade.inner.default_group) {
                Some(group) => group
                    .transactions
                    .run(crate::transaction::TransactionAttribute::UseOrBegin, None, body),
                None => body(),
            },
            None => body(),
        }
    } else {
        body()
    };
    let out = out.map_err(|err| err.to_string());
    // A thread's own messages are its errors, as a service call's would be.
    let errors = context.message().errors().to_vec();
    if !errors.is_empty() && out.is_ok() {
        return Err(errors.join("; "));
    }
    if let Err(err) = crate::user::logout() {
        tracing::debug!("logout after a parallel thread: {err}");
    }
    out
}
