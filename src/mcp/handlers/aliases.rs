//! mcp-task-tools.allium: AliasResolvesToTaskId, agents.allium:
//! AgentNameResolvesToTask.
//!
//! Every tool that takes a task id also takes the task's alias or the name of
//! one of its agents, current or ended. Ids are parsed by the argument
//! deserializers, which have no store to ask, so the word is swapped for its
//! id here, once, before dispatch — no handler needs to know aliases or agent
//! names exist. Step A5 of agents.allium removes the alias half.

use serde_json::Value;

use crate::models::{Agent, Task};

/// The argument keys that name a task.
const TASK_REF_KEYS: [&str; 2] = ["task_id", "task_ids"];

/// A word that cannot be turned into one task id.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum BadTaskRef {
    /// Neither a task's alias nor an agent's name.
    Unknown(String),
    /// One task's alias and another task's agent name. The word lists start
    /// disjoint, but `update_tasks` lets a person set an alias to any word, so
    /// guessing would act on the wrong task.
    Ambiguous(String),
}

impl BadTaskRef {
    /// What the caller is told. An unknown word is a plain not-found.
    pub(super) fn message(&self) -> String {
        match self {
            BadTaskRef::Unknown(word) => {
                format!("No task has the alias or agent name '{word}'")
            }
            BadTaskRef::Ambiguous(word) => format!(
                "'{word}' is one task's alias and another task's agent name; use the task id"
            ),
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
    TASK_REF_KEYS.iter().any(|key| match map.get(*key) {
        Some(Value::Array(items)) => items.iter().any(is_name),
        Some(value) => is_name(value),
        None => false,
    })
}

fn resolve_one(value: &mut Value, tasks: &[Task], agents: &[Agent]) -> Result<(), BadTaskRef> {
    let Value::String(s) = value else {
        return Ok(());
    };
    if is_id(s) {
        return Ok(());
    }
    // The two word lists start disjoint (agents.allium, A11), but an alias can
    // be set to any word, so a word naming two different tasks is refused
    // rather than resolved by whichever list is asked first.
    let by_alias = tasks.iter().find(|t| t.alias == *s).map(|t| t.id);
    let by_agent = agents.iter().find(|a| a.name == *s).map(|a| a.task_id);
    match (by_alias, by_agent) {
        (Some(a), Some(b)) if a != b => Err(BadTaskRef::Ambiguous(std::mem::take(s))),
        (Some(id), _) | (None, Some(id)) => {
            *value = Value::from(id.0);
            Ok(())
        }
        (None, None) => Err(BadTaskRef::Unknown(std::mem::take(s))),
    }
}

/// Replace every alias or agent name in the task-naming arguments with the
/// task's id. Digits stay ids, so an unknown id still reaches the handler and
/// fails there as it always did; a word that is no alias and no agent's name
/// fails here the same way.
pub(super) fn resolve_task_refs(
    args: &mut Value,
    tasks: &[Task],
    agents: &[Agent],
) -> Result<(), BadTaskRef> {
    let Value::Object(map) = args else {
        return Ok(());
    };
    for key in TASK_REF_KEYS {
        match map.get_mut(key) {
            Some(Value::Array(items)) => {
                for item in items {
                    resolve_one(item, tasks, agents)?;
                }
            }
            Some(value) => resolve_one(value, tasks, agents)?,
            None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AgentEndReason, AgentId, AgentState, DispatchMode, TaskId};
    use serde_json::json;

    fn tasks() -> Vec<Task> {
        vec![
            Task {
                id: TaskId(65542),
                alias: "scar".into(),
                ..Default::default()
            },
            Task {
                id: TaskId(7),
                alias: "moss".into(),
                ..Default::default()
            },
        ]
    }

    fn agents() -> Vec<Agent> {
        vec![
            Agent {
                id: AgentId(1),
                task_id: TaskId(65542),
                name: "ada".into(),
                host: String::new(),
                start_mode: DispatchMode::Dispatch,
                state: AgentState::Ended,
                started_at: chrono::Utc::now(),
                ended_at: Some(chrono::Utc::now()),
                end_reason: Some(AgentEndReason::Replaced),
                tmux_window: None,
            },
            Agent {
                id: AgentId(2),
                task_id: TaskId(65542),
                name: "linus".into(),
                host: String::new(),
                start_mode: DispatchMode::Dispatch,
                state: AgentState::Current,
                started_at: chrono::Utc::now(),
                ended_at: None,
                end_reason: None,
                tmux_window: None,
            },
            Agent {
                id: AgentId(3),
                task_id: TaskId(7),
                name: "grace".into(),
                host: String::new(),
                start_mode: DispatchMode::Dispatch,
                state: AgentState::Current,
                started_at: chrono::Utc::now(),
                ended_at: None,
                end_reason: None,
                tmux_window: None,
            },
        ]
    }

    #[test]
    fn a_current_agent_name_becomes_its_tasks_id() {
        let mut args = json!({"task_id": "grace", "title": "x"});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, json!({"task_id": 7, "title": "x"}));
    }

    #[test]
    fn an_ended_agent_name_still_resolves_to_its_task() {
        let mut args = json!({"task_ids": ["ada", "linus", "moss"]});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, json!({"task_ids": [65542, 65542, 7]}));
    }

    #[test]
    fn a_word_that_is_one_tasks_alias_and_anothers_agent_name_is_ambiguous() {
        let mut tasks = tasks();
        tasks[1].alias = "ada".into(); // task 7's alias; "ada" is task 65542's agent
        let mut args = json!({"task_id": "ada"});
        assert_eq!(
            resolve_task_refs(&mut args, &tasks, &agents()),
            Err(BadTaskRef::Ambiguous("ada".into()))
        );
    }

    #[test]
    fn a_word_naming_the_same_task_twice_is_not_ambiguous() {
        let mut tasks = tasks();
        tasks[0].alias = "ada".into(); // task 65542's alias and its agent
        let mut args = json!({"task_id": "ada"});
        resolve_task_refs(&mut args, &tasks, &agents()).unwrap();
        assert_eq!(args, json!({"task_id": 65542}));
    }

    #[test]
    fn an_alias_becomes_its_id() {
        let mut args = json!({"task_id": "scar", "title": "x"});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, json!({"task_id": 65542, "title": "x"}));
    }

    #[test]
    fn aliases_inside_task_ids_resolve_alongside_numbers_and_numeric_strings() {
        let mut args = json!({"task_ids": ["scar", 7, "9"]});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, json!({"task_ids": [65542, 7, "9"]}));
    }

    #[test]
    fn an_unknown_alias_is_an_error_naming_it() {
        let mut args = json!({"task_id": "nope"});
        assert_eq!(
            resolve_task_refs(&mut args, &tasks(), &agents()),
            Err(BadTaskRef::Unknown("nope".into()))
        );
    }

    #[test]
    fn numbers_and_numeric_strings_are_left_alone_even_when_unknown() {
        let mut args = json!({"task_id": 999999});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, json!({"task_id": 999999}));
        let mut args = json!({"task_id": "999999"});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, json!({"task_id": "999999"}));
    }

    #[test]
    fn other_argument_keys_are_not_touched() {
        let mut args = json!({"title": "scar", "description": "nope"});
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
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
        resolve_task_refs(&mut args, &tasks(), &agents()).unwrap();
        assert_eq!(args, Value::Null);
    }
}
