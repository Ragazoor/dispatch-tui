//! mcp-task-tools.allium: AliasResolvesToTaskId.
//!
//! Every tool that takes a task id also takes the task's alias. Ids are parsed
//! by the argument deserializers, which have no store to ask, so the alias is
//! swapped for its id here, once, before dispatch — no handler needs to know
//! aliases exist.

use serde_json::Value;

use crate::models::Task;

/// The argument keys that name a task.
const TASK_REF_KEYS: [&str; 2] = ["task_id", "task_ids"];

/// An alias that names no task.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct UnknownAlias(pub String);

/// Is `s` an id, as the id deserializers read one (digits)?
fn is_id(s: &str) -> bool {
    s.trim().parse::<i64>().is_ok()
}

/// Does any task-naming argument hold something other than an id? Lets the
/// dispatcher skip loading the task list for the common all-numeric call.
pub(super) fn holds_alias(args: &Value) -> bool {
    let Value::Object(map) = args else {
        return false;
    };
    let is_alias = |v: &Value| v.as_str().is_some_and(|s| !is_id(s));
    TASK_REF_KEYS.iter().any(|key| match map.get(*key) {
        Some(Value::Array(items)) => items.iter().any(is_alias),
        Some(value) => is_alias(value),
        None => false,
    })
}

fn resolve_one(value: &mut Value, tasks: &[Task]) -> Result<(), UnknownAlias> {
    let Value::String(s) = value else {
        return Ok(());
    };
    if is_id(s) {
        return Ok(());
    }
    match tasks.iter().find(|t| t.alias == *s) {
        Some(task) => {
            *value = Value::from(task.id.0);
            Ok(())
        }
        None => Err(UnknownAlias(std::mem::take(s))),
    }
}

/// Replace every alias in the task-naming arguments with the task's id.
/// Digits stay ids, so an unknown id still reaches the handler and fails there
/// as it always did; a word that is no task's alias fails here the same way.
pub(super) fn resolve_task_aliases(args: &mut Value, tasks: &[Task]) -> Result<(), UnknownAlias> {
    let Value::Object(map) = args else {
        return Ok(());
    };
    for key in TASK_REF_KEYS {
        match map.get_mut(key) {
            Some(Value::Array(items)) => {
                for item in items {
                    resolve_one(item, tasks)?;
                }
            }
            Some(value) => resolve_one(value, tasks)?,
            None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskId;
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

    #[test]
    fn an_alias_becomes_its_id() {
        let mut args = json!({"task_id": "scar", "title": "x"});
        resolve_task_aliases(&mut args, &tasks()).unwrap();
        assert_eq!(args, json!({"task_id": 65542, "title": "x"}));
    }

    #[test]
    fn aliases_inside_task_ids_resolve_alongside_numbers_and_numeric_strings() {
        let mut args = json!({"task_ids": ["scar", 7, "9"]});
        resolve_task_aliases(&mut args, &tasks()).unwrap();
        assert_eq!(args, json!({"task_ids": [65542, 7, "9"]}));
    }

    #[test]
    fn an_unknown_alias_is_an_error_naming_it() {
        let mut args = json!({"task_id": "nope"});
        assert_eq!(
            resolve_task_aliases(&mut args, &tasks()),
            Err(UnknownAlias("nope".into()))
        );
    }

    #[test]
    fn numbers_and_numeric_strings_are_left_alone_even_when_unknown() {
        let mut args = json!({"task_id": 999999});
        resolve_task_aliases(&mut args, &tasks()).unwrap();
        assert_eq!(args, json!({"task_id": 999999}));
        let mut args = json!({"task_id": "999999"});
        resolve_task_aliases(&mut args, &tasks()).unwrap();
        assert_eq!(args, json!({"task_id": "999999"}));
    }

    #[test]
    fn other_argument_keys_are_not_touched() {
        let mut args = json!({"title": "scar", "description": "nope"});
        resolve_task_aliases(&mut args, &tasks()).unwrap();
        assert_eq!(args, json!({"title": "scar", "description": "nope"}));
    }

    #[test]
    fn holds_alias_is_true_only_for_a_non_numeric_task_ref() {
        assert!(holds_alias(&json!({"task_id": "scar"})));
        assert!(holds_alias(&json!({"task_ids": [1, "scar"]})));
        assert!(!holds_alias(&json!({"task_id": 5, "title": "scar"})));
        assert!(!holds_alias(&json!({"task_ids": [1, "2"]})));
        assert!(!holds_alias(&Value::Null));
    }

    #[test]
    fn non_object_arguments_pass_through() {
        let mut args = Value::Null;
        resolve_task_aliases(&mut args, &tasks()).unwrap();
        assert_eq!(args, Value::Null);
    }
}
