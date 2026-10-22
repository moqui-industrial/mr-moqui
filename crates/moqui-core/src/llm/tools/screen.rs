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

//! The `screen` tool (plans/LLM-A2A-Assist.md 7.3, 7.4, 7.7): the apps as the user, through the run's screen session
//! ([`crate::llm::screen`]). One call runs its steps in order, `go`, `set`, `find`, `do` and `key`, and a failed step
//! stops the rest; with none it looks again. The answer is the screen as text, `--- ok` or `--- error <why>` last, with
//! a `did you mean:` clause for a name that finds no element.
//!
//! Every render of the session is fenced and its writes are held: a press that writes answers `approval pending`, and
//! the held request travels as the result's data ([`held_outcome`], the seam phase 13.4 turns into an approval and a
//! pause). When the user's page follows (the drawer's `screen.follow`), the session's follow events are stored events
//! of the run. The session's state is kept on the conversation after each call, so a later run or another node opens
//! where this one stood.

use std::sync::Arc;

use serde_json::{json, Value as Json};

use super::schema::{object, Property};
use super::{caps, AgentTool, Classification, ToolOutcome, ToolResult, ToolSpec};
use crate::context::{ExecutionContext, ExecutionContextFactory};
use crate::llm::run::context::{ScreenHandle, TurnContext};
use crate::llm::screen::{
    self, OpenSpec, PendingWrite, ScreenAnswer, ScreenCommand, ScreenError, ScreenSessions, StateWriter,
};

const NAME: &str = "screen";

pub struct ScreenTool(ToolSpec);

impl Default for ScreenTool {
    fn default() -> Self {
        let mut spec = ToolSpec::new(
            NAME,
            "Use the apps as the user: open a screen, fill fields, press buttons, links and rows. Steps run in order go, \
             set, find, do; with none it looks again. The answer is the screen as text: path, title, focus, overlay, \
             message, the screen with the focused element in [[ ]], then --- ok or --- error and why. A press that \
             writes waits for the user's approval. To filter a list: do Find, set its find fields, do Find; a dialog's \
             fields are set after do on its button.",
            object(&[
                Property::optional("go", "string", "A screen path, with ?query"),
                Property::optional("set", "object", "{element: value}; an element is its label, id or #n"),
                Property::optional("find", "string", "Text to move to, a list row's included"),
                Property::optional("do", "string", "The element to press: a button, link, list row or menu"),
                Property::optional("key", "string", "Keys such as Enter or F10, only when no element fits"),
                Property::optional("elements", "boolean", "Add the list of elements"),
                Property::optional("rows", "integer", "Screen rows, default 30, at most 100"),
            ]),
            Classification::Read,
        )
        .needs(&[caps::SCREEN]);
        // A held write becomes an approval the run waits for (phase 13.4), so it never runs beside other reads.
        spec.may_pause = true;
        Self(spec)
    }
}

impl AgentTool for ScreenTool {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    fn call(&self, ctx: &mut TurnContext<'_>, args: Json) -> ToolOutcome {
        let Some(sessions) = screen::sessions() else {
            return ToolOutcome::Failed("screens are not available on this server".to_string());
        };
        let Some(factory) = ExecutionContext::with_current(|ec| Arc::clone(ec.factory())) else {
            return ToolOutcome::Failed("the screen tool runs only in a run's context".to_string());
        };
        let mut command = match command_of(&args) {
            Ok(command) => command,
            Err(problem) => return ToolOutcome::Failed(problem),
        };
        command.agent_id.clone_from(&ctx.agent.agent_id);
        command.run_id = ctx.run.map(|run| run.run_id.clone());
        // a go that is not a screen the user may open runs nothing: the screen as it is, and why
        let refused = command
            .go
            .as_deref()
            .and_then(|path| screen::check_screen_path(&factory, path).err());
        if refused.is_some() {
            command = ScreenCommand {
                agent_id: command.agent_id,
                run_id: command.run_id,
                rows: command.rows,
                ..ScreenCommand::default()
            };
        }
        let follow = ctx.caps.has(caps::SCREEN_FOLLOW);
        let Some(handle) = ctx.screen.as_deref_mut() else {
            return ToolOutcome::Failed("this run has no screen session".to_string());
        };
        let mut answer = match run_command(
            &factory,
            sessions.as_ref(),
            handle,
            ctx.user,
            ctx.agent,
            follow,
            &command,
        ) {
            Ok(answer) => answer,
            Err(error) => return ToolOutcome::Failed(error.to_string()),
        };
        if let Some(problem) = refused {
            answer.ok = false;
            answer.text = replace_outcome(&answer.text, &format!("--- error {problem}"));
        }
        if let Some(facade) = factory.entity() {
            // a step writes it only while its node holds the run's lease; a call with no run, only for its own user
            let writer = match ctx.run {
                Some(run) => StateWriter::Run {
                    run_id: &run.run_id,
                    node: ctx.events.node(),
                },
                None => StateWriter::Owner(&ctx.user.user_id),
            };
            if let Err(error) = screen::store_state(&facade, &handle.conversation_id, writer, &answer.state) {
                tracing::error!(
                    "the screen state of conversation {} was not kept: {error}",
                    handle.conversation_id
                );
            }
        }
        if follow {
            for event in &answer.follow {
                ctx.events.emit(&event.event_type, event.payload.clone());
            }
        }
        match answer.held.take() {
            Some(write) => held_outcome(&answer, write),
            None => ToolOutcome::Done(ToolResult {
                text: answer.text.clone(),
                data: Some(json!({"ok": answer.ok, "path": answer.path, "parameters": answer.parameters})),
            }),
        }
    }
}

crate::submit_tool!(ScreenTool);

/// The command of a call's arguments.
fn command_of(args: &Json) -> Result<ScreenCommand, String> {
    let text = |name: &str| {
        args.get(name)
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let mut set = Vec::new();
    match args.get("set") {
        None | Some(Json::Null) => {}
        Some(Json::Object(values)) => {
            for (element, value) in values {
                let value = match value {
                    Json::String(text) => text.clone(),
                    Json::Null => String::new(),
                    Json::Bool(flag) => if *flag { "Y" } else { "N" }.to_string(),
                    other => other.to_string(),
                };
                set.push((element.clone(), value));
            }
        }
        Some(_) => {
            return Err("set is an object of element names and their values, as {\"Name\": \"Ann\"}".to_string())
        }
    }
    let rows = match args.get("rows") {
        None | Some(Json::Null) => None,
        Some(value) => {
            let rows = value
                .as_u64()
                .and_then(|rows| u16::try_from(rows).ok())
                .filter(|rows| (10..=screen::MAX_ROWS).contains(rows))
                .ok_or_else(|| format!("rows is a number from 10 to {}", screen::MAX_ROWS))?;
            Some(rows)
        }
    };
    Ok(ScreenCommand {
        go: text("go"),
        set,
        find: text("find"),
        press: text("do"),
        key: text("key"),
        elements: args.get("elements").and_then(Json::as_bool).unwrap_or(false),
        rows,
        ..ScreenCommand::default()
    })
}

/// Run `command` in the conversation's session, opening it (or opening it again, when this node no longer has it)
/// from the state the conversation kept.
fn run_command(
    factory: &Arc<ExecutionContextFactory>,
    sessions: &dyn ScreenSessions,
    handle: &mut ScreenHandle,
    user: &crate::llm::run::context::UserRef,
    agent: &crate::llm::run::context::AgentDef,
    follow: bool,
    command: &ScreenCommand,
) -> Result<ScreenAnswer, ScreenError> {
    for attempt in 0..2 {
        if handle.session_id.is_empty() {
            handle.session_id = match sessions.find(&user.user_id, &handle.conversation_id) {
                Some(id) => id,
                None => sessions.open(&open_spec(factory, handle, user, agent, follow, command))?,
            };
        }
        match sessions.command(&handle.session_id, &user.user_id, command) {
            Err(ScreenError::NotFound) if attempt == 0 => handle.session_id.clear(),
            other => return other,
        }
    }
    Err(ScreenError::NotFound)
}

fn open_spec(
    factory: &Arc<ExecutionContextFactory>,
    handle: &ScreenHandle,
    user: &crate::llm::run::context::UserRef,
    agent: &crate::llm::run::context::AgentDef,
    follow: bool,
    command: &ScreenCommand,
) -> OpenSpec {
    let state = factory
        .entity()
        .and_then(|facade| match screen::load_state(&facade, &handle.conversation_id) {
            Ok(state) => state,
            Err(error) => {
                tracing::error!(
                    "the screen state of conversation {} could not be read: {error}",
                    handle.conversation_id
                );
                None
            }
        });
    OpenSpec {
        user_id: user.user_id.clone(),
        username: user.username.clone(),
        conversation_id: handle.conversation_id.clone(),
        agent_id: agent.agent_id.clone(),
        run_id: command.run_id.clone(),
        start_path: agent
            .start_path
            .clone()
            .filter(|path| screen::path_problem(path).is_none())
            .unwrap_or_else(|| screen::DEFAULT_START_PATH.to_string()),
        columns: screen::DEFAULT_COLUMNS,
        rows: command.rows.unwrap_or(screen::DEFAULT_ROWS),
        follow,
        state,
    }
}

/// `text` with its last line, the outcome, replaced by `outcome`.
fn replace_outcome(text: &str, outcome: &str) -> String {
    let kept = match text.rfind("\n--- ") {
        Some(at) => &text[..at],
        None if text.starts_with("--- ") => "",
        None => text,
    };
    if kept.is_empty() {
        outcome.to_string()
    } else {
        format!("{kept}\n{outcome}")
    }
}

/// What a held write comes to. Until approvals (phase 13.4) this is a result that says the write waits for the user's
/// approval, with the held request as its data (`pendingWrite`, a [`PendingWrite`]); 13.4 makes this the approval's
/// precheck, snapshot and row, and a [`ToolOutcome::Pause`] that [`AgentTool::resume`] answers by running it through
/// [`ScreenSessions::execute_approved`].
pub fn held_outcome(answer: &ScreenAnswer, write: PendingWrite) -> ToolOutcome {
    let transition = write.transition().to_string();
    let changes: Vec<String> = write
        .changes
        .iter()
        .map(|change| {
            format!(
                "{}: {} → {}",
                if change.label.is_empty() {
                    &change.field
                } else {
                    &change.label
                },
                if change.original.is_empty() {
                    "(empty)"
                } else {
                    &change.original
                },
                change.proposed
            )
        })
        .collect();
    let mut line = format!("approval pending: {transition} waits for the user's approval; nothing was saved");
    if !changes.is_empty() {
        line.push_str(&format!(" ({})", changes.join(", ")));
    }
    let text = replace_outcome(&answer.text, &format!("{line}\n--- ok"));
    ToolOutcome::Done(ToolResult {
        text,
        data: Some(json!({
            "ok": answer.ok,
            "path": answer.path,
            "parameters": answer.parameters,
            "approval": "pending",
            "pendingWrite": serde_json::to_value(&write).unwrap_or(Json::Null),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arguments_become_one_command_in_order() {
        let command = command_of(
            &json!({"go": " /apps/x?a=1 ", "set": {"Name": "Ann", "Count": 3, "Done": true},
            "do": "Save", "elements": true, "rows": 40}),
        )
        .unwrap();
        assert_eq!(command.go.as_deref(), Some("/apps/x?a=1"));
        assert_eq!(
            command.set,
            [
                ("Count".to_string(), "3".to_string()),
                ("Done".to_string(), "Y".to_string()),
                ("Name".to_string(), "Ann".to_string())
            ]
        );
        assert_eq!(command.press.as_deref(), Some("Save"));
        assert!(command.elements);
        assert_eq!(command.rows, Some(40));
        assert!(command_of(&json!({"rows": 500})).is_err());
        assert!(command_of(&json!({"set": ["Name", "Ann"]})).is_err());
        assert_eq!(command_of(&json!({})).unwrap(), ScreenCommand::default());
    }

    #[test]
    fn an_outcome_line_is_replaced_and_a_held_write_says_it_waits() {
        assert_eq!(
            replace_outcome("path /a\n--- ok", "--- error no"),
            "path /a\n--- error no"
        );
        assert_eq!(replace_outcome("--- ok", "--- error no"), "--- error no");
        let answer = ScreenAnswer {
            ok: true,
            text: "path /apps/x\nerror held\n--- ok".to_string(),
            path: "/apps/x".to_string(),
            ..ScreenAnswer::default()
        };
        let write = PendingWrite {
            held: json!({"transition": "updateThing", "argsHash": "h"}),
            args_hash: "h".to_string(),
            changes: vec![crate::llm::screen::FieldChange {
                field: "description".to_string(),
                label: "Description".to_string(),
                original: "Old".to_string(),
                proposed: "New".to_string(),
            }],
            ..PendingWrite::default()
        };
        let ToolOutcome::Done(result) = held_outcome(&answer, write) else {
            panic!("a held write is a result");
        };
        assert!(
            result.text.ends_with(
                "approval pending: updateThing waits for the user's approval; nothing was saved (Description: Old → New)\n--- ok"
            ),
            "{}",
            result.text
        );
        let data = result.data.unwrap();
        assert_eq!(data["approval"], "pending");
        assert_eq!(data["pendingWrite"]["argsHash"], "h");
    }
}
