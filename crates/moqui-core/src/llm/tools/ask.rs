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

//! `ask` (plans/LLM-A2A-Assist.md 7.3): the agent asks the user before it goes on, with 2 to 4 short options, the one it
//! recommends first, or a few fields to fill; the user may also type an answer. It is offered where someone can answer
//! (`question.inline` in the drawer and a chat room, `question.data` for A2A, where the question is the
//! `input-required` data part); a job has no `ask`. The call stores an `AgentQuestion` (the one write of this read tool,
//! with the fence lifted for it: run bookkeeping, as the budget question is) and pauses the run with a
//! `question.asked` event; the answer (`submit#AgentInput` of kind answer, or a message) resumes it, and the model reads
//! what the user chose. The fields are plain text fields named and labeled; the typed forms of `request_form` come in
//! phase 13.4 behind [`Field`].

use moqui_lib::object::Value;
use serde_json::{json, Value as Json};

use super::format::ToolError;
use super::schema::{object, Property};
use super::{caps, AgentTool, Classification, Pause, PauseKind, ToolInput, ToolOutcome, ToolSpec};
use crate::entity::fence;
use crate::llm::run::context::TurnContext;

const QUESTION: &str = "moqui.agent.AgentQuestion";
const MAX_OPTIONS: usize = 4;
const MAX_FIELDS: usize = 6;
const OPTION_CAP: usize = 80;

pub struct Ask(ToolSpec);

impl Default for Ask {
    fn default() -> Self {
        let mut schema = object(&[
            Property::required("question", "string", ""),
            Property::optional("options", "array", "2 to 4 short answers, the one you recommend first"),
            Property::optional("multi", "boolean", "More than one may be chosen"),
            Property::optional("fields", "array", "Names of fields to fill instead"),
        ]);
        schema["properties"]["options"]["items"] = json!({"type": "string"});
        schema["properties"]["fields"]["items"] = json!({"type": "string"});
        Self(ToolSpec {
            may_pause: true,
            ..ToolSpec::new(
                "ask",
                "Ask the user before you go on: 2 to 4 short options (they may also type an answer), or a few fields to fill. Your last action in the turn.",
                schema,
                Classification::Read,
            )
            .needs_any(&[caps::QUESTION_INLINE, caps::QUESTION_DATA])
        })
    }
}

/// A field the user fills: its name and its label. Phase 13.4's forms add the type, choices and checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub label: String,
}

impl Field {
    /// A field from what the model wrote: `name`, `Label`, or `name: Label`.
    pub fn of(text: &str) -> Self {
        let (name, label) = match text.split_once(':') {
            Some((name, label)) => (name.trim(), label.trim()),
            None => (text.trim(), text.trim()),
        };
        let name: String = name
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .enumerate()
            .map(|(at, word)| {
                if at == 0 {
                    word.to_lowercase()
                } else {
                    let mut chars = word.chars();
                    chars.next().map_or_else(String::new, |first| {
                        first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect()
                    })
                }
            })
            .collect();
        Self {
            name,
            label: label.to_string(),
        }
    }
}

/// A question as the clients get it.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub text: String,
    pub options: Vec<String>,
    pub multi: bool,
    pub fields: Vec<Field>,
}

impl Question {
    /// The question of a call's arguments.
    ///
    /// # Errors
    ///
    /// Returns an error the model reads: no question, one option or more than four, too many fields.
    pub fn of(args: &Json) -> Result<Self, ToolError> {
        let text = args
            .get("question")
            .and_then(Json::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if text.is_empty() {
            return Err(ToolError::new(
                "no_question",
                "Call ask with the question and 2 to 4 short options, the one you recommend first.",
            ));
        }
        let list = |key: &str| -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for item in args.get(key).and_then(Json::as_array).into_iter().flatten() {
                let item = match item {
                    Json::String(text) => text.trim().to_string(),
                    Json::Object(map) => map
                        .get("label")
                        .or_else(|| map.get("name"))
                        .and_then(Json::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    other => other.to_string(),
                };
                if !item.is_empty() && !out.iter().any(|known| known.eq_ignore_ascii_case(&item)) {
                    out.push(item);
                }
            }
            out
        };
        let options: Vec<String> = list("options")
            .into_iter()
            .map(|option| super::format::cut(&option, OPTION_CAP))
            .collect();
        let fields: Vec<Field> = list("fields").iter().map(|field| Field::of(field)).collect();
        if fields.is_empty() && !(2..=MAX_OPTIONS).contains(&options.len()) {
            return Err(ToolError::new(
                "options",
                format!(
                    "ask takes 2 to 4 options, and this call has {}. Call ask again with 2 to 4 short options, the one you recommend first, or with fields.",
                    options.len()
                ),
            ));
        }
        if fields.len() > MAX_FIELDS {
            return Err(ToolError::new(
                "fields",
                format!("ask takes at most {MAX_FIELDS} fields. Ask for fewer, or open the screen that has them."),
            ));
        }
        Ok(Self {
            text,
            options,
            multi: args.get("multi").and_then(Json::as_bool).unwrap_or(false),
            fields,
        })
    }

    /// The event's data: the question, its options (the first recommended) and its fields.
    pub fn to_json(&self) -> Json {
        json!({
            "text": self.text,
            "options": self.options.iter().enumerate().map(|(at, label)| json!({"label": label, "recommended": at == 0}))
                .collect::<Vec<_>>(),
            "multi": self.multi,
            "fields": self.fields.iter().map(|field| json!({"name": field.name, "label": field.label, "type": "text"}))
                .collect::<Vec<_>>(),
            "freeText": true,
        })
    }

    /// What the model reads of an answer: the options chosen (by label, by number, or by data `choice` or `choices`),
    /// the fields filled (data `values`, or the data object itself), else the text the user typed.
    pub fn answer_text(&self, input: &ToolInput) -> (String, Json) {
        let data = input.data.clone().unwrap_or(Json::Null);
        let mut picked: Vec<String> = Vec::new();
        let named = |text: &str| -> Option<String> {
            let text = text.trim();
            if let Ok(number) = text.parse::<usize>() {
                if (1..=self.options.len()).contains(&number) {
                    return Some(self.options[number - 1].clone());
                }
            }
            self.options
                .iter()
                .find(|option| option.eq_ignore_ascii_case(text))
                .cloned()
        };
        for key in ["choice", "choices"] {
            match data.get(key) {
                Some(Json::String(text)) => picked.extend(named(text)),
                Some(Json::Array(items)) => picked.extend(items.iter().filter_map(Json::as_str).filter_map(named)),
                Some(Json::Number(number)) => picked.extend(named(&number.to_string())),
                _ => {}
            }
        }
        if picked.is_empty() && !self.options.is_empty() {
            let parts: Vec<&str> = if self.multi {
                input.text.split(',').collect()
            } else {
                vec![input.text.as_str()]
            };
            let all: Vec<Option<String>> = parts.iter().map(|part| named(part)).collect();
            if !all.is_empty() && all.iter().all(Option::is_some) {
                picked = all.into_iter().flatten().collect();
            }
        }
        if !picked.is_empty() {
            let chosen: Vec<String> = picked.iter().map(|label| format!("\"{label}\"")).collect();
            return (
                format!("The user chose {}.", chosen.join(", ")),
                json!({"choices": picked}),
            );
        }
        if !self.fields.is_empty() {
            let values = data.get("values").cloned().unwrap_or_else(|| data.clone());
            if let Some(values) = values.as_object().filter(|values| !values.is_empty()) {
                let filled: Vec<String> = self
                    .fields
                    .iter()
                    .map(|field| {
                        let value = values.get(&field.name).map_or_else(String::new, |value| match value {
                            Json::String(text) => text.clone(),
                            other => other.to_string(),
                        });
                        format!("{}: {value}", field.name)
                    })
                    .collect();
                return (
                    format!("The user filled: {}.", filled.join(", ")),
                    json!({"values": values}),
                );
            }
        }
        let typed = input.text.trim();
        if typed.is_empty() {
            return ("The user answered nothing.".to_string(), json!({"text": ""}));
        }
        (format!("The user answered: {typed}"), json!({"text": typed}))
    }
}

impl AgentTool for Ask {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn call(&self, ctx: &mut TurnContext<'_>, args: Json) -> ToolOutcome {
        let question = match Question::of(&args) {
            Ok(question) => question,
            Err(error) => return error.outcome(),
        };
        let Some(run) = ctx.run else {
            return ToolError::new(
                "no_run",
                "There is no one to ask here. Go on with what you have, and say what you assumed.",
            )
            .outcome();
        };
        let Some(facade) = super::apps::factory().ok().and_then(|factory| factory.entity()) else {
            return ToolError::new("unavailable", "The question could not be asked now.").outcome();
        };
        let data = question.to_json();
        // the question row is run bookkeeping the runtime writes, as the budget question is, so the fence lifts for it
        // under the run's lease, in one transaction: a node that lost the run leaves no open question behind
        let node = ctx.events.node().to_string();
        let stored = {
            let _lifted = fence::lift();
            crate::llm::run::quietly(|| {
                crate::llm::run::in_tx(&facade, || {
                    crate::llm::run::held(&facade, &run.run_id, &node, crate::llm::run::Hold::Lease)?;
                    let mut row = facade.make_value(QUESTION)?;
                    row.set_sequenced_id_primary(&facade)?;
                    row.set("runId", Value::string(&run.run_id));
                    row.set("toolCallId", Value::string(&ctx.call.tool_call_id));
                    row.set("userId", Value::string(&ctx.user.user_id));
                    row.set("questionText", Value::string(&question.text));
                    row.set("choicesJson", Value::string(data["options"].to_string()));
                    if !question.fields.is_empty() {
                        row.set("schemaJson", Value::string(data["fields"].to_string()));
                    }
                    row.set("statusId", Value::string("AgqOpen"));
                    row.set("createdDate", crate::llm::run::stamp(crate::llm::run::now_millis()));
                    row.create(&facade)?;
                    Ok(crate::llm::text_of(&row, "questionId").unwrap_or_default())
                })
            })
        };
        let question_id = match stored {
            Ok(id) => id,
            Err(error) => {
                tracing::error!("the question of agent run {} could not be stored: {error}", run.run_id);
                return ToolError::new(
                    "unavailable",
                    "The question could not be asked now. Go on, and say what you assumed.",
                )
                .outcome();
            }
        };
        let mut payload = data;
        payload["questionId"] = json!(question_id);
        payload["toolCallId"] = json!(ctx.call.tool_call_id);
        payload["surface"] = json!(ctx.surface.as_str());
        ToolOutcome::Pause(Pause {
            kind: PauseKind::Question,
            reference: question_id,
            expires: None,
            event: Some(("question.asked".to_string(), payload)),
        })
    }

    fn resume(&self, ctx: &mut TurnContext<'_>, args: Json, input: &ToolInput) -> ToolOutcome {
        let question = match Question::of(&args) {
            Ok(question) => question,
            Err(error) => return error.outcome(),
        };
        let (text, answer) = question.answer_text(input);
        if let (Some(run), Some(facade)) = (
            ctx.run,
            super::apps::factory().ok().and_then(|factory| factory.entity()),
        ) {
            let _lifted = fence::lift();
            let node = ctx.events.node().to_string();
            let answered = crate::llm::run::quietly(|| {
                crate::llm::run::in_tx(&facade, || {
                    crate::llm::run::held(&facade, &run.run_id, &node, crate::llm::run::Hold::Lease)?;
                    facade
                        .find(QUESTION)?
                        .eq("runId", Value::string(&run.run_id))
                        .eq("toolCallId", Value::string(&ctx.call.tool_call_id))
                        .eq("statusId", Value::string("AgqOpen"))
                        .update_all([
                            ("statusId".to_string(), Value::string("AgqAnswered")),
                            ("answerJson".to_string(), Value::string(answer.to_string())),
                            (
                                "answeredDate".to_string(),
                                crate::llm::run::stamp(crate::llm::run::now_millis()),
                            ),
                        ])
                })
            });
            if let Err(error) = answered {
                tracing::error!("the answer of agent run {} could not be stored: {error}", run.run_id);
            }
        }
        ctx.events.emit(
            "question.answered",
            json!({"toolCallId": ctx.call.tool_call_id, "answer": answer}),
        );
        ToolOutcome::text(text)
    }
}

crate::submit_tool!(Ask);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_takes_two_to_four_options_or_fields() {
        let question =
            Question::of(&json!({"question": "Which one?", "options": ["Acme Corp", "Acme Inc", "acme corp"]}))
                .unwrap();
        assert_eq!(question.options, ["Acme Corp", "Acme Inc"]);
        assert_eq!(
            question.to_json()["options"][0],
            json!({"label": "Acme Corp", "recommended": true})
        );
        let one = Question::of(&json!({"question": "Sure?", "options": ["Yes"]}))
            .unwrap_err()
            .to_text();
        assert!(
            one.starts_with("{\"error\":\"options\"") && one.contains("has 1"),
            "{one}"
        );
        assert!(Question::of(&json!({"options": ["a", "b"]})).is_err());
        let fields =
            Question::of(&json!({"question": "Who?", "fields": ["First name", "email: Email address"]})).unwrap();
        assert_eq!(
            fields.fields,
            [
                Field {
                    name: "firstName".to_string(),
                    label: "First name".to_string()
                },
                Field {
                    name: "email".to_string(),
                    label: "Email address".to_string()
                },
            ]
        );
    }

    #[test]
    fn an_answer_is_an_option_by_label_number_or_data_else_the_text() {
        let question = Question::of(&json!({"question": "Which?", "options": ["Acme Corp", "Acme Inc"]})).unwrap();
        let input = |text: &str, data: Option<Json>| ToolInput {
            kind: "answer".to_string(),
            text: text.to_string(),
            data,
        };
        assert_eq!(
            question.answer_text(&input("acme inc", None)).0,
            "The user chose \"Acme Inc\"."
        );
        assert_eq!(
            question.answer_text(&input("1", None)).0,
            "The user chose \"Acme Corp\"."
        );
        assert_eq!(
            question.answer_text(&input("", Some(json!({"choice": "Acme Inc"})))).0,
            "The user chose \"Acme Inc\"."
        );
        assert_eq!(
            question.answer_text(&input("Neither, use Beta LLC", None)).0,
            "The user answered: Neither, use Beta LLC"
        );
        let multi = Question::of(&json!({"question": "Which?", "options": ["A", "B", "C"], "multi": true})).unwrap();
        assert_eq!(
            multi.answer_text(&input("a, c", None)).0,
            "The user chose \"A\", \"C\"."
        );
        let form = Question::of(&json!({"question": "Who?", "fields": ["First name"]})).unwrap();
        assert_eq!(
            form.answer_text(&input("", Some(json!({"values": {"firstName": "Jo"}}))))
                .0,
            "The user filled: firstName: Jo."
        );
    }
}
