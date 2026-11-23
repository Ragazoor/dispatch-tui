//! agents.allium: AgentNameResolvesToTask, OrchestratorNameIsNotATask,
//! OrchestratorNameResolvesToItsEpic; mcp-task-tools.allium:
//! TaskRefResolvesToTaskId.
//!
//! Every tool that takes a task id also takes the name of one of the task's
//! agents, current or ended, and every tool that takes an epic id also takes
//! the name of the epic's orchestrator. Ids are parsed by the argument
//! deserializers, which have no store to ask, so the name is swapped for its
//! id here, once, before dispatch — no handler needs to know names exist.

use serde_json::Value;

use crate::models::{Agent, EpicId};

/// The argument keys that name a task.
const TASK_REF_KEYS: [&str; 2] = ["task_id", "task_ids"];

/// The argument key that names an epic.
const EPIC_REF_KEY: &str = "epic_id";

/// An orchestrator as the resolver needs it: its name and its epic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OrchestratorRef {
    pub(super) name: String,
    pub(super) epic: EpicId,
    pub(super) epic_title: String,
}

/// A word in a task or epic argument that names nothing it can stand for.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum UnknownTaskRef {
    /// No agent has the name.
    NoAgent(String),
    /// The name is an orchestrator's, which is no task.
    Orchestrator(OrchestratorRef),
    /// No orchestrator has the name given as an epic.
    NoOrchestrator(String),
}

impl UnknownTaskRef {
    /// What the caller is told.
    pub(super) fn message(&self) -> String {
        match self {
            Self::NoAgent(name) => format!("No task has the agent name '{name}'"),
            Self::Orchestrator(o) => format!(
                "'{}' is the orchestrator of epic {} ({}), not a task",
                o.name, o.epic.0, o.epic_title
            ),
            Self::NoOrchestrator(name) => {
                format!("No epic has the orchestrator name '{name}'")
            }
        }
    }
}

/// Is `s` an id, as the id deserializers read one (digits)?
fn is_id(s: &str) -> bool {
    s.trim().parse::<i64>().is_ok()
}

/// Does any task-naming argument hold something other than an id? Lets the
/// dispatcher skip loading the task and agent lists for the common all-numeric
/// call.
pub(super) fn holds_name(args: &Value) -> bool {
    let Value::Object(map) = args else {
        return false;
    };
    let is_name = |v: &Value| v.as_str().is_some_and(|s| !is_id(s));
    TASK_REF_KEYS
        .iter()
        .chain(std::iter::once(&EPIC_REF_KEY))
        .any(|key| match map.get(*key) {
            Some(Value::Array(items)) => items.iter().any(is_name),
            Some(value) => is_name(value),
            None => false,
        })
}

fn resolve_one(
    value: &mut Value,
    agents: &[Agent],
    orchestrators: &[OrchestratorRef],
) -> Result<(), UnknownTaskRef> {
    let Value::String(s) = value else {
        return Ok(());
    };
    if is_id(s) {
        return Ok(());
    }
    if let Some(agent) = agents.iter().find(|a| a.name == *s) {
        *value = Value::from(agent.task_id.0);
        return Ok(());
    }
    match orchestrators.iter().find(|o| o.name == *s) {
        Some(o) => Err(UnknownTaskRef::Orchestrator(o.clone())),
        None => Err(UnknownTaskRef::NoAgent(std::mem::take(s))),
    }
}

/// Every orchestrator the board holds, with its epic's title.
pub(super) async fn orchestrator_refs(state: &crate::mcp::McpState) -> Vec<OrchestratorRef> {
    let mut refs = Vec::new();
    for o in state.db.list_orchestrators().await {
        let epic_title = state
            .db
            .get_epic(o.epic_id)
            .await
            .map(|e| e.title)
            .unwrap_or_default();
        refs.push(OrchestratorRef {
            name: o.name,
            epic: o.epic_id,
            epic_title,
        });
    }
    refs
}

fn resolve_epic(
    value: &mut Value,
    orchestrators: &[OrchestratorRef],
) -> Result<(), UnknownTaskRef> {
    let Value::String(s) = value else {
        return Ok(());
    };
    if is_id(s) {
        return Ok(());
    }
    match orchestrators.iter().find(|o| o.name == *s) {
        Some(o) => {
            *value = Value::from(o.epic.0);
            Ok(())
        }
        None => Err(UnknownTaskRef::NoOrchestrator(std::mem::take(s))),
    }
}

/// Replace every agent name in the task-naming arguments with its task's id,
/// and an orchestrator's name in `epic_id` with its epic's id. Digits stay
/// ids, so an unknown id still reaches the handler and fails there as it
/// always did; a word that is no such name fails here the same way.
pub(super) fn resolve_task_refs(
    args: &mut Value,
    agents: &[Agent],
    orchestrators: &[OrchestratorRef],
) -> Result<(), UnknownTaskRef> {
    let Value::Object(map) = args else {
        return Ok(());
    };
    for key in TASK_REF_KEYS {
        match map.get_mut(key) {
            Some(Value::Array(items)) => {
                for item in items {
                    resolve_one(item, agents, orchestrators)?;
                }
            }
            Some(value) => resolve_one(value, agents, orchestrators)?,
            None => {}
        }
    }
    if let Some(value) = map.get_mut(EPIC_REF_KEY) {
        resolve_epic(value, orchestrators)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentEndReason, AgentId, AgentState, DispatchMode, TaskId};
    use serde_json::json;

    fn agents() -> Vec<Agent> {
        vec![
            Agent {
                id: AgentId(1),
                task_id: TaskId(65542),
                name: "ada".into(),
                host: String::new(),
                start_mode: Some(DispatchMode::Dispatch),
                state: AgentState::Ended,
                started_at: Some(chrono::Utc::now()),
                ended_at: Some(chrono::Utc::now()),
                end_reason: Some(AgentEndReason::Replaced),
                tmux_window: None,
                last_pre_tool_use_at: None,
                last_notification_at: None,
                live_subagents: 0,
                stop_pending: false,
                stop_pending_at: None,
                background_work: crate::models::BackgroundWork::default(),
            },
            Agent {
                id: AgentId(2),
                task_id: TaskId(65542),
                name: "linus".into(),
                host: String::new(),
                start_mode: Some(DispatchMode::Dispatch),
                state: AgentState::Current,
                started_at: Some(chrono::Utc::now()),
                ended_at: None,
                end_reason: None,
                tmux_window: None,
                last_pre_tool_use_at: None,
                last_notification_at: None,
                live_subagents: 0,
                stop_pending: false,
                stop_pending_at: None,
                background_work: crate::models::BackgroundWork::default(),
            },
            Agent {
                id: AgentId(3),
                task_id: TaskId(7),
                name: "grace".into(),
                host: String::new(),
                start_mode: Some(DispatchMode::Dispatch),
                state: AgentState::Current,
                started_at: Some(chrono::Utc::now()),
                ended_at: None,
                end_reason: None,
                tmux_window: None,
                last_pre_tool_use_at: None,
                last_notification_at: None,
                live_subagents: 0,
                stop_pending: false,
                stop_pending_at: None,
                background_work: crate::models::BackgroundWork::default(),
            },
        ]
    }

    fn orchestrators() -> Vec<OrchestratorRef> {
        vec![OrchestratorRef {
            name: "zlata".into(),
            epic: EpicId(69708),
            epic_title: "Epic orchestrators".into(),
        }]
    }

    #[test]
    fn an_orchestrators_name_as_a_task_is_answered_as_what_it_is() {
        let mut args = json!({"task_id": "zlata"});
        let err = resolve_task_refs(&mut args, &agents(), &orchestrators()).unwrap_err();
        assert_eq!(
            err.message(),
            "'zlata' is the orchestrator of epic 69708 (Epic orchestrators), not a task"
        );
    }

    #[test]
    fn an_orchestrators_name_as_an_epic_becomes_its_epics_id() {
        let mut args = json!({"epic_id": "zlata", "query": "x"});
        assert!(holds_name(&args));
        resolve_task_refs(&mut args, &agents(), &orchestrators()).unwrap();
        assert_eq!(args, json!({"epic_id": 69708, "query": "x"}));
    }

    #[test]
    fn an_epic_word_that_is_no_orchestrators_name_is_refused() {
        let mut args = json!({"epic_id": "grace"});
        let err = resolve_task_refs(&mut args, &agents(), &orchestrators()).unwrap_err();
        assert_eq!(err.message(), "No epic has the orchestrator name 'grace'");
    }

    #[test]
    fn a_numeric_epic_id_is_left_alone() {
        let mut args = json!({"epic_id": "12"});
        assert!(!holds_name(&args));
        resolve_task_refs(&mut args, &agents(), &orchestrators()).unwrap();
        assert_eq!(args, json!({"epic_id": "12"}));
    }

    #[test]
    fn a_current_agent_name_becomes_its_tasks_id() {
        let mut args = json!({"task_id": "grace", "title": "x"});
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, json!({"task_id": 7, "title": "x"}));
    }

    #[test]
    fn an_ended_agent_name_still_resolves_to_its_task() {
        let mut args = json!({"task_ids": ["ada", "linus", "grace"]});
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, json!({"task_ids": [65542, 65542, 7]}));
    }

    #[test]
    fn agent_names_inside_task_ids_resolve_alongside_numbers_and_numeric_strings() {
        let mut args = json!({"task_ids": ["linus", 7, "9"]});
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, json!({"task_ids": [65542, 7, "9"]}));
    }

    #[test]
    fn an_unknown_name_is_an_error_naming_it() {
        let mut args = json!({"task_id": "nope"});
        assert_eq!(
            resolve_task_refs(&mut args, &agents(), &[]),
            Err(UnknownTaskRef::NoAgent("nope".into()))
        );
    }

    #[test]
    fn numbers_and_numeric_strings_are_left_alone_even_when_unknown() {
        let mut args = json!({"task_id": 999999});
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, json!({"task_id": 999999}));
        let mut args = json!({"task_id": "999999"});
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, json!({"task_id": "999999"}));
    }

    #[test]
    fn other_argument_keys_are_not_touched() {
        let mut args = json!({"title": "scar", "description": "nope"});
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, json!({"title": "scar", "description": "nope"}));
    }

    #[test]
    fn holds_name_is_true_only_for_a_non_numeric_task_ref() {
        assert!(holds_name(&json!({"task_id": "scar"})));
        assert!(holds_name(&json!({"task_ids": [1, "scar"]})));
        assert!(!holds_name(&json!({"task_id": 5, "title": "scar"})));
        assert!(!holds_name(&json!({"task_ids": [1, "2"]})));
        assert!(!holds_name(&Value::Null));
    }

    #[test]
    fn non_object_arguments_pass_through() {
        let mut args = Value::Null;
        resolve_task_refs(&mut args, &agents(), &[]).unwrap();
        assert_eq!(args, Value::Null);
    }
}
