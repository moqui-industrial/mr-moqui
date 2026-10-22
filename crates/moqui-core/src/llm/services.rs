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

//! The bodies of the `type="rust"` services of `org.moqui.agent.AgentAdminServices`: the effective configuration, the
//! admin's capability verdicts, the endpoint test and the capability probe, the usage rollup, the content cleanup, the
//! quota check and the dashboard totals.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use moqui_lib::error::Error as LibError;
use moqui_lib::object::{is_empty, to_plain_string, Value};

use moqui_db::sql::Condition;

use super::conf::{mask_secret, ReasoningLevel};
use super::provider::{self, CallOptions, Finish, Message, ModelRequest, Recorder, ToolSpec};
use super::registry::{self, Verdict, VerdictSource};
use super::run::RunState;
use super::usage;
use crate::context::ExecutionContext;
use crate::entity::EntityFacade;
use crate::service::rust_services::{rust_service, RustServiceFn};

pub(crate) fn register(map: &mut BTreeMap<&'static str, RustServiceFn>) -> Result<(), String> {
    rust_service!(map, get_effective_config)?;
    rust_service!(map, test_endpoint)?;
    rust_service!(map, probe_model)?;
    rust_service!(map, set_model_capability)?;
    rust_service!(map, rollup_usage)?;
    rust_service!(map, clean_call_content)?;
    rust_service!(map, check_quota)?;
    rust_service!(map, get_dashboard_totals)?;
    rust_service!(map, get_call_content)?;
    Ok(())
}

type Parameters = BTreeMap<String, Value>;
type Out = Result<BTreeMap<String, Value>, LibError>;

fn text(parameters: &Parameters, key: &str) -> Option<String> {
    parameters
        .get(key)
        .filter(|value| !is_empty(value))
        .map(to_plain_string)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn add_error(text: impl Into<String>) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_error(text));
}

fn out(entries: impl IntoIterator<Item = (&'static str, Value)>) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<IndexMap<_, _>>(),
    )
}

fn opt<T: ToString>(value: Option<T>) -> Value {
    value.map_or(Value::Null, |value| Value::string(value.to_string()))
}

fn number(value: Option<u32>) -> Value {
    value.map_or(Value::Null, |value| Value::I64(i64::from(value)))
}

fn count(value: u64) -> Value {
    Value::I64(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The AgentAdmin permission, else an error message and false.
fn admin() -> bool {
    match crate::user::has_permission("AgentAdmin") {
        Ok(true) => true,
        _ => {
            add_error("this needs the AgentAdmin permission");
            false
        }
    }
}

fn llm() -> Option<std::sync::Arc<super::LlmFacade>> {
    ExecutionContext::with_current(|ec| super::facade(ec.factory()))
}

fn now_millis() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// `get#EffectiveConfig`: the endpoints, models and bindings in effect (the conf with the rows over it), keys masked,
/// with where each came from, whether it can take a call, and the problems of the conf and the rows.
fn get_effective_config(_parameters: &Parameters, facade: &EntityFacade) -> Out {
    if !admin() {
        return Ok(BTreeMap::new());
    }
    let Some(llm) = llm() else {
        return Ok(BTreeMap::new());
    };
    let registry = llm.registry(facade);
    let endpoint_list = registry
        .endpoints
        .values()
        .map(|endpoint| {
            let conf = &endpoint.conf;
            map([
                ("endpointId", Value::string(&conf.id)),
                ("protocol", Value::string(conf.protocol.as_str())),
                ("baseUrl", Value::string(&conf.url)),
                ("requestUrl", Value::string(conf.request_url())),
                ("apiKeyMasked", Value::string(mask_secret(&conf.resolved_key().unwrap_or_default()))),
                ("apiKeyEnv", opt(conf.api_key_env.as_ref())),
                ("maxConcurrent", number(conf.max_concurrent)),
                ("isLocal", Value::string(if conf.local { "Y" } else { "N" })),
                ("disabled", Value::string(if conf.disabled { "Y" } else { "N" })),
                ("source", Value::string(endpoint.source.as_str())),
            ])
        })
        .collect();
    let model_list = registry
        .models
        .values()
        .map(|model| {
            let conf = &model.conf;
            let price = |price: Option<rust_decimal::Decimal>| price.map_or(Value::Null, Value::Decimal);
            map([
                ("modelId", Value::string(&conf.id)),
                ("endpointId", Value::string(&conf.endpoint)),
                ("providerModel", Value::string(&conf.name)),
                ("tier", Value::string(conf.tier.as_str())),
                ("contextWindow", number(conf.context_window)),
                ("maxOutput", number(conf.max_output)),
                ("priceInput", price(conf.prices.input)),
                ("priceCachedInput", price(conf.prices.cached_input)),
                ("priceCacheWrite", price(conf.prices.cache_write)),
                ("priceOutput", price(conf.prices.output)),
                ("currencyUomId", Value::string(&conf.prices.currency)),
                (
                    "reasoningLevels",
                    Value::string(
                        conf.reasoning
                            .keys()
                            .map(|level| level.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ),
                ("disabled", Value::string(if conf.disabled { "Y" } else { "N" })),
                ("source", Value::string(model.source.as_str())),
                (
                    "usable",
                    Value::string(match registry.usable(&conf.id, false) {
                        Ok(_) => "yes".to_string(),
                        Err(reason) => reason,
                    }),
                ),
            ])
        })
        .collect();
    let binding_list = registry
        .bindings
        .values()
        .map(|binding| {
            let resolved = registry.resolve_feature(&binding.conf.feature);
            map([
                ("featureId", Value::string(&binding.conf.feature)),
                ("modelId", Value::string(&binding.conf.model)),
                ("fallbackModelId", opt(binding.conf.fallback.as_ref())),
                ("reasoningLevel", Value::string(binding.conf.reasoning.as_str())),
                ("source", Value::string(binding.source.as_str())),
                (
                    "resolvedModelId",
                    resolved.as_ref().map_or(Value::Null, |target| Value::string(&target.model.id)),
                ),
                (
                    "resolveError",
                    resolved.err().map_or(Value::Null, |error| Value::string(error.to_string())),
                ),
            ])
        })
        .collect();
    let conf = llm.conf();
    let problems = conf
        .problems
        .iter()
        .chain(registry.problems.iter())
        .map(Value::string)
        .collect();
    let limits = &conf.limits;
    Ok(out([
        ("enabled", Value::Bool(conf.enabled)),
        ("endpointList", Value::List(endpoint_list)),
        ("modelList", Value::List(model_list)),
        ("bindingList", Value::List(binding_list)),
        ("problemList", Value::List(problems)),
        (
            "limits",
            map([
                ("maxConcurrentRuns", number(Some(limits.max_concurrent_runs))),
                ("runQueue", number(Some(limits.run_queue))),
                ("maxSteps", number(Some(limits.max_steps))),
                ("maxRunSeconds", count(limits.max_run_seconds)),
                ("userDailyTokens", count(limits.user_daily_tokens)),
                ("userDailyCost", limits.user_daily_cost.map_or(Value::Null, Value::Decimal)),
            ]),
        ),
        (
            "contentLog",
            map([
                ("enabled", Value::Bool(conf.content_log.enabled)),
                ("redact", Value::Bool(conf.content_log.redact)),
                ("retentionDays", number(Some(conf.content_log.retention_days))),
            ]),
        ),
    ]))
}

/// The target of `modelId` (or of `endpointId`'s first model), or an error message and `None`.
fn target_of(parameters: &Parameters, facade: &EntityFacade) -> Option<registry::Target> {
    let llm = llm()?;
    let registry = llm.registry(facade);
    let model_id = match (text(parameters, "modelId"), text(parameters, "endpointId")) {
        (Some(model_id), _) => model_id,
        (None, Some(endpoint_id)) => match registry.models.values().find(|model| model.conf.endpoint == endpoint_id) {
            Some(model) => model.conf.id.clone(),
            None => {
                add_error(format!("endpoint {endpoint_id} has no model to test with"));
                return None;
            }
        },
        (None, None) => {
            add_error("modelId or endpointId is required");
            return None;
        }
    };
    match registry.resolve_model(&model_id, ReasoningLevel::None) {
        Ok(target) => Some(target),
        Err(error) => {
            add_error(error.to_string());
            None
        }
    }
}

/// The limit of one admin test or probe call.
const ADMIN_CALL_SECONDS: u64 = 90;

/// The options of an admin call: the current user, a deadline, no re-ask at the output limit, and the call log.
fn admin_options<'a>(llm: &'a super::LlmFacade, facade: &'a EntityFacade, source: usage::CallSource) -> CallOptions<'a> {
    let mut options = CallOptions::new(source);
    options.user_id = crate::user::current_user_id().ok().flatten();
    options.deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(ADMIN_CALL_SECONDS));
    options.length_retry = false;
    options.recorder = Some(Recorder { llm, facade });
    options
}

fn message(text: impl Into<String>, kind: &str) {
    let text = text.into();
    ExecutionContext::with_current(|ctx| ctx.message().add_message_typed(text, kind));
}

/// Whether a reply is an answer of some kind: not an error, a cancel, an overflow, a filter or a refusal.
fn answered(finish: &Finish) -> bool {
    matches!(finish, Finish::Stop | Finish::ToolCalls | Finish::Length)
}

/// `test#Endpoint`: one tiny call to a model, recorded with source `test`. A failure is the answer (`ok` false and
/// the message), not a service error, so its `LlmCall` row stays.
fn test_endpoint(parameters: &Parameters, facade: &EntityFacade) -> Out {
    if !admin() {
        return Ok(BTreeMap::new());
    }
    let (Some(target), Some(llm)) = (target_of(parameters, facade), llm()) else {
        return Ok(BTreeMap::new());
    };
    let request = ModelRequest {
        history: vec![Message::user("Reply with the single word: ok")],
        reasoning: Some(ReasoningLevel::None),
        max_output: Some(64),
        ..ModelRequest::default()
    };
    let mut options = admin_options(&llm, facade, usage::CallSource::Test);
    let started = std::time::Instant::now();
    let reply = provider::call(&target, &request, &mut options);
    let latency = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let ok = answered(&reply.finish);
    let error_text = reply.finish.error().map(ToString::to_string);
    if ok {
        message(format!("Model {} answered in {latency} ms: {}", target.model.id, reply.text.trim()), "success");
    } else {
        message(
            format!(
                "Model {} did not answer ({}): {}",
                target.model.id,
                reply.finish.as_str(),
                error_text.clone().unwrap_or_default()
            ),
            "danger",
        );
    }
    Ok(out([
        ("ok", Value::Bool(ok)),
        ("httpStatus", reply.http_status.map_or(Value::Null, |status| Value::I64(i64::from(status)))),
        ("latencyMs", Value::I64(latency)),
        ("replyText", Value::string(reply.text.trim())),
        ("finish", Value::string(reply.finish.as_str())),
        ("errorText", opt(error_text)),
        ("modelId", Value::string(&target.model.id)),
        ("callId", opt(reply.call_ids.last())),
    ]))
}

/// The prefix of the prompt-caching probe: about 1,500 tokens, over the smallest prefix the hosted caches keep.
fn caching_prefix() -> String {
    let mut text = String::from("You answer questions about this reference list. ");
    for index in 0..300 {
        text.push_str(&format!("Item {index} is the number {} written out. ", index * 7));
    }
    text
}

/// One probe: its verdict and evidence, from the calls it made.
struct Probe {
    capability: &'static str,
    /// `None` when a call failed for a reason that says nothing of the capability (a timeout, a network error, a
    /// rate limit, a cancel): nothing is stored, so an earlier verdict stands.
    verdict: Option<Verdict>,
    evidence: String,
}

/// Whether a reply failed for a reason that says nothing of what the model can do. A bad request does say: the server
/// refused the feature.
fn transient(finish: &Finish) -> bool {
    match finish {
        Finish::Canceled => true,
        Finish::Error(error) => error.class != provider::ErrorClass::BadRequest,
        _ => false,
    }
}

fn probe_one(capability: &'static str, target: &registry::Target, send: &mut dyn FnMut(&ModelRequest) -> provider::Reply) -> Probe {
    let mut failure: Option<String> = None;
    let mut call = |request: &ModelRequest| {
        let reply = send(request);
        if failure.is_none() && transient(&reply.finish) {
            failure = Some(match reply.finish.error() {
                Some(error) => error.to_string(),
                None => reply.finish.as_str().to_string(),
            });
        }
        reply
    };
    let get_time = ToolSpec::new("get_time", "The current time.", serde_json::json!({"type": "object", "properties": {}}));
    let get_date = ToolSpec::new("get_date", "The current date.", serde_json::json!({"type": "object", "properties": {}}));
    let failed = |reply: &provider::Reply| match reply.finish.error() {
        Some(error) => format!("{} ({error})", reply.finish.as_str()),
        None => reply.finish.as_str().to_string(),
    };
    let (verdict, evidence) = match capability {
        "tools" => {
            let reply = call(&ModelRequest {
                tools: vec![get_time],
                history: vec![Message::user("Call the get_time tool now.")],
                reasoning: Some(ReasoningLevel::None),
                max_output: Some(256),
                ..ModelRequest::default()
            });
            let names: Vec<&str> = reply.tool_calls.iter().map(|call| call.name.as_str()).collect();
            if names.contains(&"get_time") {
                (Verdict::Yes, format!("called {}", names.join(", ")))
            } else if !reply.bad_tool_calls.is_empty() {
                (Verdict::Partial, format!("a tool call with bad arguments: {}", reply.bad_tool_calls[0].error))
            } else {
                (Verdict::No, format!("no tool call; finish {}", failed(&reply)))
            }
        }
        "parallel-tools" => {
            let reply = call(&ModelRequest {
                tools: vec![get_time, get_date],
                history: vec![Message::user("Call get_time and get_date at the same time, in one reply.")],
                reasoning: Some(ReasoningLevel::None),
                max_output: Some(256),
                ..ModelRequest::default()
            });
            match reply.tool_calls.len() {
                0 => (Verdict::No, format!("no tool call; finish {}", failed(&reply))),
                1 => (Verdict::Partial, format!("one call in the reply ({})", reply.tool_calls[0].name)),
                count => (Verdict::Yes, format!("{count} calls in one reply")),
            }
        }
        "json-schema" => {
            let schema = serde_json::json!({"type": "object", "properties": {"answer": {"type": "string"}},
                "required": ["answer"], "additionalProperties": false});
            let reply = call(&ModelRequest {
                history: vec![Message::user("What is the capital of France? Answer in the JSON asked for.")],
                response_schema: Some(provider::ResponseSchema {
                    name: "answer".to_string(),
                    schema: schema.clone(),
                    strict: true,
                }),
                reasoning: Some(ReasoningLevel::None),
                max_output: Some(256),
                ..ModelRequest::default()
            });
            if !answered(&reply.finish) {
                (Verdict::No, format!("the schema request failed: {}", failed(&reply)))
            } else {
                match provider::reply_json(&reply.text) {
                    Ok(value) => match provider::check_schema(&value, &schema) {
                        Ok(()) => (Verdict::Yes, format!("the reply follows the schema: {value}")),
                        Err(problem) => (Verdict::Partial, format!("JSON that does not follow the schema: {problem}")),
                    },
                    Err(_) => (Verdict::No, format!("no JSON in the reply: {}", reply.text.chars().take(120).collect::<String>())),
                }
            }
        }
        "streaming" => {
            let reply = call(&ModelRequest {
                history: vec![Message::user("Count from 1 to 20, separated by spaces.")],
                reasoning: Some(ReasoningLevel::None),
                max_output: Some(256),
                ..ModelRequest::default()
            });
            match (answered(&reply.finish), reply.text_deltas) {
                (false, _) => (Verdict::No, format!("the call failed: {}", failed(&reply))),
                (true, 0 | 1) => (Verdict::Partial, format!("the reply came in {} piece", reply.text_deltas)),
                (true, pieces) => (Verdict::Yes, format!("the reply came in {pieces} pieces")),
            }
        }
        "reasoning-replay" => {
            let level = target
                .model
                .reasoning
                .keys()
                .copied()
                .find(|level| *level != ReasoningLevel::None)
                .unwrap_or(ReasoningLevel::Medium);
            let mut request = ModelRequest {
                tools: vec![get_time],
                history: vec![Message::user("Think briefly, then call the get_time tool.")],
                reasoning: Some(level),
                max_output: Some(2048),
                ..ModelRequest::default()
            };
            let first = call(&request);
            match (&first.replay, first.tool_calls.first()) {
                (None, _) => (Verdict::No, format!("nothing to replay at reasoning {}; finish {}", level.as_str(), failed(&first))),
                (Some(_), None) => (Verdict::Partial, "a replay but no tool call to continue from".to_string()),
                (Some(bag), Some(tool_call)) => {
                    let kinds: Vec<String> = bag
                        .items
                        .iter()
                        .map(|item| serde_json::to_value(item).ok().and_then(|value| value.get("kind").and_then(|kind| kind.as_str().map(str::to_string))).unwrap_or_default())
                        .collect();
                    request.history.push(first.assistant_message());
                    request.history.push(Message::tool_result(&tool_call.id, &tool_call.name, "12:00"));
                    let second = call(&request);
                    if answered(&second.finish) {
                        (Verdict::Yes, format!("{} sent back and the next call answered", kinds.join(", ")))
                    } else {
                        (Verdict::No, format!("the call with {} sent back failed: {}", kinds.join(", "), failed(&second)))
                    }
                }
            }
        }
        "prompt-caching" => {
            let request = ModelRequest {
                system: vec![caching_prefix()],
                history: vec![Message::user("Which number is item 3? Answer with the number only.")],
                reasoning: Some(ReasoningLevel::None),
                max_output: Some(32),
                cache_key: Some(format!("probe-{}", target.model.id)),
                ..ModelRequest::default()
            };
            let first = call(&request);
            let second = call(&request);
            let cached = second.usage.cached_input.max(first.usage.cached_input);
            if !answered(&second.finish) {
                (Verdict::No, format!("the second call failed: {}", failed(&second)))
            } else if cached > 0 {
                (
                    Verdict::Yes,
                    format!("{cached} of {} input tokens cached on the second call", second.usage.input),
                )
            } else if first.usage.cache_write > 0 {
                (Verdict::Partial, format!("{} tokens written to the cache, none read", first.usage.cache_write))
            } else {
                (Verdict::No, format!("no cached tokens reported ({} input)", second.usage.input))
            }
        }
        other => (Verdict::No, format!("no probe for {other}")),
    };
    match failure {
        Some(failure) => Probe {
            capability,
            verdict: None,
            evidence: format!("no verdict: a call failed ({failure})"),
        },
        None => Probe {
            capability,
            verdict: Some(verdict),
            evidence,
        },
    }
}

/// `probe#Model`: tiny live requests for each capability, each verdict stored with its evidence unless an admin's is
/// there; a probe that fails is a verdict of no, and the others go on. Out: `capabilityList`, each with `stored`.
fn probe_model(parameters: &Parameters, facade: &EntityFacade) -> Out {
    if !admin() {
        return Ok(BTreeMap::new());
    }
    let (Some(target), Some(llm)) = (target_of(parameters, facade), llm()) else {
        return Ok(BTreeMap::new());
    };
    let mut options = admin_options(&llm, facade, usage::CallSource::Probe);
    let mut list = Vec::new();
    let mut yes = 0;
    for capability in registry::CAPABILITIES {
        // each call gets its own deadline, so one slow call does not use up the next one's time
        let probe = probe_one(capability, &target, &mut |request| {
            options.deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(ADMIN_CALL_SECONDS));
            provider::call(&target, request, &mut options)
        });
        let stored = match probe.verdict {
            None => false,
            Some(verdict) => {
                let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
                match registry::store_verdict(
                    facade,
                    &target.model.id,
                    probe.capability,
                    verdict,
                    VerdictSource::Probe,
                    Some(&probe.evidence),
                ) {
                    Ok(stored) => stored,
                    Err(error) => {
                        tracing::error!("the {} verdict of model {} was not stored: {error}", probe.capability, target.model.id);
                        false
                    }
                }
            }
        };
        if probe.verdict == Some(Verdict::Yes) {
            yes += 1;
        }
        list.push(map([
            ("capability", Value::string(probe.capability)),
            ("verdict", Value::string(probe.verdict.map_or("unknown", Verdict::as_str))),
            ("evidence", Value::string(&probe.evidence)),
            ("stored", Value::Bool(stored)),
        ]));
    }
    message(
        format!("Model {}: {yes} of {} capabilities probed yes", target.model.id, registry::CAPABILITIES.len()),
        "info",
    );
    Ok(out([("capabilityList", Value::List(list))]))
}

/// `set#ModelCapability`: an admin's verdict on a capability, which a probe never replaces; an empty verdict removes
/// the admin's so the next probe decides.
fn set_model_capability(parameters: &Parameters, facade: &EntityFacade) -> Out {
    if !admin() {
        return Ok(BTreeMap::new());
    }
    let (Some(model_id), Some(capability)) = (text(parameters, "modelId"), text(parameters, "capability")) else {
        add_error("modelId and capability are required");
        return Ok(BTreeMap::new());
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    match text(parameters, "verdict") {
        None => {
            facade
                .find("moqui.llm.LlmModelCapability")?
                .eq("modelId", Value::string(&model_id))
                .eq("capability", Value::string(&capability))
                .delete_all()?;
        }
        Some(verdict_text) => {
            let Some(verdict) = Verdict::parse(&verdict_text) else {
                add_error(format!("verdict {verdict_text} is not yes, no or partial"));
                return Ok(BTreeMap::new());
            };
            let evidence = text(parameters, "evidence")
                .unwrap_or_else(|| format!("set by {}", crate::user::current_user_id().ok().flatten().unwrap_or_default()));
            registry::store_verdict(facade, &model_id, &capability, verdict, VerdictSource::Admin, Some(&evidence))?;
        }
    }
    Ok(BTreeMap::new())
}

/// `rollup#LlmUsage`: recompute `LlmUsageDaily` for `usageDate`, or for today and the `days` before it (default 1),
/// each day in its own transaction. A day that fails is logged and rolled back, and the others go on. Out: `rowCount`.
fn rollup_usage(parameters: &Parameters, facade: &EntityFacade) -> Out {
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let dates: Vec<jiff::civil::Date> = match text(parameters, "usageDate") {
        Some(date_text) => match date_text.get(..10).unwrap_or(&date_text).parse::<jiff::civil::Date>() {
            Ok(date) => vec![date],
            Err(error) => {
                add_error(format!("usageDate {date_text} is not a date: {error}"));
                return Ok(BTreeMap::new());
            }
        },
        None => {
            let days = text(parameters, "days").and_then(|days| days.parse::<i64>().ok()).unwrap_or(1).clamp(0, 366);
            let today = usage::day_of(now_millis())?;
            (0..=days)
                .filter_map(|back| today.checked_sub(jiff::Span::new().days(back)).ok())
                .collect()
        }
    };
    let mut rows = 0;
    for date in dates {
        // each day in a transaction of its own: one that fails rolls back alone
        let rolled = facade.transactions_for("moqui.llm.LlmUsageDaily").and_then(|transactions| {
            transactions.run(crate::transaction::TransactionAttribute::ForceNew, None, || usage::rollup_day(facade, date))
        });
        match rolled {
            Ok(written) => rows += written,
            Err(error) => tracing::error!("the LLM usage of {date} was not rolled up: {error}"),
        }
    }
    Ok(out([("rowCount", Value::I64(i64::try_from(rows).unwrap_or(i64::MAX)))]))
}

/// `clean#LlmCallContent`: clear the content of calls older than `retentionDays` (default the conf's). Out:
/// `clearedCount`.
fn clean_call_content(parameters: &Parameters, facade: &EntityFacade) -> Out {
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let days = match text(parameters, "retentionDays").and_then(|days| days.parse::<u32>().ok()) {
        Some(days) => days,
        None => llm().map_or(14, |llm| llm.conf().content_log.retention_days),
    };
    let cleared = usage::clean_call_content(facade, days, now_millis())?;
    Ok(out([("clearedCount", Value::I64(cleared))]))
}

/// `check#LlmQuota`: whether a user (default the current one) may make more calls in a scope (default all). Another
/// user's quota needs AgentAdmin.
fn check_quota(parameters: &Parameters, facade: &EntityFacade) -> Out {
    let current = crate::user::current_user_id().ok().flatten().unwrap_or_default();
    let user_id = text(parameters, "userId").unwrap_or_else(|| current.clone());
    if user_id.is_empty() {
        add_error("no user to check the quota of");
        return Ok(BTreeMap::new());
    }
    if user_id != current && !admin() {
        return Ok(BTreeMap::new());
    }
    let scope = text(parameters, "scope").unwrap_or_else(|| "all".to_string());
    let Some(llm) = llm() else {
        return Ok(BTreeMap::new());
    };
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let status = usage::check_quota(facade, &llm.conf().limits, &user_id, &scope, now_millis())?;
    Ok(out([
        ("withinQuota", Value::Bool(status.is_within())),
        ("reason", opt(status.exceeded.as_ref())),
        ("period", Value::string(&status.limit.period)),
        ("usedTokens", count(status.used.usage.total())),
        ("usedCost", Value::Decimal(status.used.cost)),
        ("maxTokens", status.limit.max_tokens.map_or(Value::Null, count)),
        ("maxCost", status.limit.max_cost.map_or(Value::Null, Value::Decimal)),
        ("limitFrom", Value::string(&status.limit.from)),
    ]))
}

/// `get#DashboardTotals`: the calls of today (from `LlmCall`) and of this month (the rollup of the earlier days plus
/// today), for all users, and the runs submitted today, waiting for input and failed today.
fn get_dashboard_totals(_parameters: &Parameters, facade: &EntityFacade) -> Out {
    if !admin() {
        return Ok(BTreeMap::new());
    }
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let today = usage::day_of(now_millis())?;
    let (start, end) = usage::day_bounds(today)?;
    let today_totals = usage::totals_between(facade, start, end)?;
    let mut month = usage::daily_totals_between(facade, today.first_of_month(), today)?;
    month.calls += today_totals.calls;
    month.errors += today_totals.errors;
    month.usage.add(&today_totals.usage);
    month.cost += today_totals.cost;
    let totals = |totals: &usage::Totals| {
        map([
            ("callCount", count(totals.calls)),
            ("errorCount", count(totals.errors)),
            ("inputTokens", count(totals.usage.input)),
            ("cachedInputTokens", count(totals.usage.cached_input)),
            ("outputTokens", count(totals.usage.output)),
            ("totalTokens", count(totals.usage.total())),
            ("cost", Value::Decimal(totals.cost.normalize())),
            ("currencyUomId", opt(totals.currency.as_ref())),
        ])
    };
    let stamp = crate::user::stamp_value;
    let runs = || -> Result<crate::entity::EntityFind, LibError> { facade.find("moqui.agent.AgentRun") };
    let since_today = || Condition::greater_equal("submittedDate", stamp(start));
    let runs_today = runs()?.condition(since_today()).count()?;
    let runs_failed = runs()?
        .condition(since_today())
        .eq("statusId", Value::string(RunState::Failed.status_id()))
        .count()?;
    let runs_waiting = runs()?
        .condition(Condition::is_in(
            "statusId",
            vec![
                Value::string(RunState::InputRequired.status_id()),
                Value::string(RunState::AuthRequired.status_id()),
            ],
        ))
        .count()?;
    Ok(out([
        ("today", totals(&today_totals)),
        ("month", totals(&month)),
        ("runsToday", Value::I64(runs_today)),
        ("runsWaiting", Value::I64(runs_waiting)),
        ("runsFailedToday", Value::I64(runs_failed)),
    ]))
}

/// `get#LlmCallContent`: the request and response of a call for its own user, or for a user with AgentAudit, whose
/// read is logged in `AgentAuditLog` (D12): AgentAdmin alone does not show another user's content.
fn get_call_content(parameters: &Parameters, facade: &EntityFacade) -> Out {
    let Some(call_id) = text(parameters, "callId") else {
        add_error("callId is required");
        return Ok(BTreeMap::new());
    };
    let viewer = crate::user::current_user_id().ok().flatten().filter(|id| !id.is_empty());
    let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
    let Some(row) = facade.find("moqui.llm.LlmCall")?.eq("callId", Value::string(&call_id)).one()? else {
        return Ok(out([
            ("contentShown", Value::Bool(false)),
            ("reason", Value::string(format!("there is no call {call_id}"))),
        ]));
    };
    let owner = super::text_of(&row, "userId");
    let own = viewer.is_some() && owner == viewer;
    let audit = !own && viewer.is_some() && crate::user::has_permission("AgentAudit").unwrap_or(false);
    if !own && !audit {
        return Ok(out([
            ("contentShown", Value::Bool(false)),
            ("audited", Value::Bool(false)),
            ("reason", Value::string("The content of another user's call needs the AgentAudit permission.")),
        ]));
    }
    if audit {
        let mut log = facade.make_value("moqui.agent.AgentAuditLog")?;
        log.set_sequenced_id_primary(facade)?;
        log.set("auditorUserId", opt(viewer.as_ref()));
        log.set("targetUserId", opt(owner.as_ref()));
        log.set("callId", Value::string(&call_id));
        log.set("runId", opt(super::text_of(&row, "runId")));
        log.set(
            "reason",
            Value::string(text(parameters, "reason").unwrap_or_else(|| "the content of an LLM call".to_string())),
        );
        log.set("auditDate", crate::user::now_value());
        log.create(facade)?;
    }
    Ok(out([
        ("contentShown", Value::Bool(true)),
        ("audited", Value::Bool(audit)),
        ("requestJson", opt(super::text_of(&row, "requestJson"))),
        ("responseJson", opt(super::text_of(&row, "responseJson"))),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::conf::{EndpointConf, ModelConf, ProtocolId};
    use crate::llm::provider::fake;

    fn target(id: &str) -> registry::Target {
        fake::clear(id);
        registry::Target {
            endpoint: EndpointConf::new(id, ProtocolId::Fake, ""),
            model: ModelConf::new(format!("{id}-model"), id, "fake-1"),
            reasoning: ReasoningLevel::None,
            feature: None,
            fell_back_from: None,
        }
    }

    fn probe(capability: &'static str, target: &registry::Target) -> Probe {
        let mut options = CallOptions::new(usage::CallSource::Probe);
        options.max_attempts = 1;
        probe_one(capability, target, &mut |request| provider::call(target, request, &mut options))
    }

    #[test]
    fn a_probe_whose_call_fails_gives_no_verdict_and_a_refused_feature_is_no() {
        let target = target("probe-fails");
        assert_eq!(probe("tools", &target).verdict, Some(Verdict::Yes));
        fake::script("probe-fails", vec![fake::Reply::status(503, "")]);
        let failed = probe("tools", &target);
        assert_eq!(failed.verdict, None);
        assert!(failed.evidence.starts_with("no verdict"), "{}", failed.evidence);
        // the second call of the cache probe is rate limited: still no verdict
        fake::script(
            "probe-fails",
            vec![fake::Reply::text("7"), fake::Reply::status(429, r#"{"error":{"message":"slow down"}}"#)],
        );
        assert_eq!(probe("prompt-caching", &target).verdict, None);
        // a server that refuses the schema says no
        fake::script(
            "probe-fails",
            vec![fake::Reply::status(400, r#"{"error":{"message":"response_format is not supported"}}"#)],
        );
        assert_eq!(probe("json-schema", &target).verdict, Some(Verdict::No));
    }
}
