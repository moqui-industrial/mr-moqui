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

//! `ec.llm`, for app code (plans/LLM-A2A-Assist.md 4.14), with the `llm` feature.
//!
//! ```text
//! ec.llm.complete({prompt, system, messages, model, feature, tier, schema, schemaName, reasoning, maxOutput,
//!         timeoutSeconds})
//!     -> {text, json, modelId, finish, inputTokens, cachedInputTokens, outputTokens, cost, callIds}
//! ec.llm.isEnabled() -> boolean
//! ```
//!
//! `messages` are earlier turns, each `{role: "user" | "assistant", text}`; `schema` is a JSON schema (a map or its
//! text), and with it `json` is the reply parsed and checked. The model is `model`, else the binding of `feature`, else
//! a model of `tier`. A call that ends without an answer throws.

use super::*;
use crate::llm::conf::{ReasoningLevel, Tier};
use crate::llm::provider::Message;
use crate::llm::CompleteRequest;

pub(super) fn llm_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, Error> {
    let object = Object::new(ctx.clone()).map_err(js_error)?;
    let complete = Function::new(ctx.clone(), llm_complete).map_err(js_error)?;
    object.set("complete", complete).map_err(js_error)?;
    let enabled = Function::new(ctx.clone(), |ctx: Ctx<'_>| {
        host_guard(&ctx, || {
            let factory = with_ec(&ctx, |ec| ec.factory().clone())?;
            Ok(crate::llm::facade(&factory).enabled())
        })
    })
    .map_err(js_error)?;
    object.set("isEnabled", enabled).map_err(js_error)?;
    Ok(object)
}

fn text_option(map: &indexmap::IndexMap<String, moqui_lib::object::Value>, key: &str) -> Option<String> {
    map.get(key)
        .filter(|value| !value.is_null())
        .map(moqui_lib::object::to_plain_string)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn llm_complete<'js>(ctx: Ctx<'js>, args: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let moqui_lib::object::Value::Map(map) = js_to_host(&ctx, &args)? else {
            return Err(Exception::throw_message(&ctx, "ec.llm.complete takes an options map"));
        };
        let mut ask = CompleteRequest {
            model: text_option(&map, "model"),
            feature: text_option(&map, "feature"),
            system: text_option(&map, "system"),
            prompt: text_option(&map, "prompt"),
            schema_name: text_option(&map, "schemaName"),
            ..CompleteRequest::default()
        };
        if let Some(tier) = text_option(&map, "tier") {
            ask.tier = Some(Tier::parse(&tier).ok_or_else(|| {
                Exception::throw_message(&ctx, &format!("tier {tier} is not fast, balanced or thorough"))
            })?);
        }
        if let Some(level) = text_option(&map, "reasoning") {
            ask.reasoning = Some(ReasoningLevel::parse(&level).ok_or_else(|| {
                Exception::throw_message(
                    &ctx,
                    &format!("reasoning {level} is not none, low, medium, high or max"),
                )
            })?);
        }
        ask.max_output = text_option(&map, "maxOutput").and_then(|text| text.parse().ok());
        ask.timeout = text_option(&map, "timeoutSeconds")
            .and_then(|text| text.parse::<u64>().ok())
            .map(std::time::Duration::from_secs);
        ask.schema = match map.get("schema") {
            None | Some(moqui_lib::object::Value::Null) => None,
            Some(moqui_lib::object::Value::String(text)) => Some(
                serde_json::from_str(text)
                    .map_err(|error| Exception::throw_message(&ctx, &format!("the schema is not JSON: {error}")))?,
            ),
            Some(value) => Some(moqui_lib::json::to_json(value)),
        };
        if let Some(moqui_lib::object::Value::List(messages)) = map.get("messages") {
            for message in messages {
                let moqui_lib::object::Value::Map(message) = message else {
                    continue;
                };
                let text = text_option(message, "text")
                    .or_else(|| text_option(message, "content"))
                    .unwrap_or_default();
                ask.messages.push(match text_option(message, "role").as_deref() {
                    Some("assistant") => Message::assistant(text),
                    _ => Message::user(text),
                });
            }
        }
        let factory = with_ec(&ctx, |ec| ec.factory().clone())?;
        let facade = facade_of(&ctx)?;
        let completion = crate::llm::facade(&factory)
            .complete(&facade, &ask)
            .map_err(|error| throw_lib(&ctx, &error))?;
        let mut out = indexmap::IndexMap::new();
        let count = |value: u64| moqui_lib::object::Value::I64(i64::try_from(value).unwrap_or(i64::MAX));
        out.insert("text".to_string(), moqui_lib::object::Value::String(completion.text));
        out.insert(
            "json".to_string(),
            completion
                .json
                .as_ref()
                .map_or(moqui_lib::object::Value::Null, moqui_lib::json::from_json),
        );
        out.insert(
            "modelId".to_string(),
            moqui_lib::object::Value::String(completion.model_id),
        );
        out.insert(
            "finish".to_string(),
            moqui_lib::object::Value::String(completion.finish),
        );
        out.insert("inputTokens".to_string(), count(completion.usage.input));
        out.insert("cachedInputTokens".to_string(), count(completion.usage.cached_input));
        out.insert("outputTokens".to_string(), count(completion.usage.output));
        out.insert(
            "cost".to_string(),
            completion
                .cost
                .map_or(moqui_lib::object::Value::Null, moqui_lib::object::Value::Decimal),
        );
        out.insert(
            "callIds".to_string(),
            moqui_lib::object::Value::List(
                completion
                    .call_ids
                    .into_iter()
                    .map(moqui_lib::object::Value::String)
                    .collect(),
            ),
        );
        host_to_js(&ctx, moqui_lib::object::Value::Map(out))
    })
}

#[cfg(test)]
mod tests {
    use crate::context::ExecutionContextFactory;
    use crate::script::{clear_context_stack, eval_condition, run_script};

    const CONF: &str = r#"<moqui-conf><llm-facade>
        <endpoint id="script-fake" protocol="fake"/>
        <model id="script-model" endpoint="script-fake" name="fake-1"/>
        <bind feature="classify" model="script-model" reasoning="none"/>
    </llm-facade></moqui-conf>"#;

    #[test]
    fn ec_llm_complete_answers_with_the_checked_json_of_a_schema() {
        clear_context_stack();
        let context = ExecutionContextFactory::test_context_with_conf(CONF);
        context.factory().set_entity(crate::testing::open());
        let _entered = context.enter();
        crate::llm::provider::fake::clear("script-fake");
        let holds = |expression: &str| assert!(eval_condition(expression).unwrap(), "{expression}");
        holds("ec.llm.isEnabled() === true");
        holds(
            r#"(function () {
                const answer = ec.llm.complete({feature: "classify", prompt: "Invoice #12",
                    schema: {type: "object", required: ["label"], properties: {label: {enum: ["invoice", "other"]}}}});
                return answer.json.label === "invoice" && answer.modelId === "script-model" && answer.finish === "stop"
                    && answer.callIds.length === 1 && answer.inputTokens > 0;
            })()"#,
        );
        holds(r#"ec.llm.complete({model: "script-model", prompt: "Say hi"}).text === "Fake reply to: Say hi""#);
        for (script, expected) in [
            (
                r#"ec.llm.complete({model: "script-model"})"#,
                "needs a prompt or messages",
            ),
            (r#"ec.llm.complete({tier: "huge", prompt: "x"})"#, "tier huge"),
            (
                r#"ec.llm.complete({model: "nope", prompt: "x"})"#,
                "model nope is not configured",
            ),
        ] {
            let error = run_script(script).unwrap_err().to_string();
            assert!(error.contains(expected), "{script}: {error}");
        }
    }
}
