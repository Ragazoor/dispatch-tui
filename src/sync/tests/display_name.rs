//! The task's display name as the board reads it (`agents.allium`, A17 and
//! A18; `core.allium: Task.display_name`): the name of its assigned or current
//! agent, else of the agent that ended last. Only a current agent lends the
//! task its session facts.

use crate::models::{AgentState, TaskId};
use crate::spacetime::bindings;
use crate::sync::rows::SharedRows;

fn task() -> bindings::Task {
    bindings::Task {
        id: 9,
        title: "t".into(),
        repo_path: "/repo".into(),
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        labels: String::new(),
        owner: String::new(),
        ..dispatch_spacetime_module::blank_task().into()
    }
}

fn assigned(id: i64, name: &str) -> bindings::Agent {
    bindings::Agent {
        id,
        task_id: 9,
        name: name.into(),
        state: "assigned".into(),
        ..dispatch_spacetime_module::blank_agent().into()
    }
}

fn ended(id: i64, name: &str, ended_at: &str) -> bindings::Agent {
    bindings::Agent {
        id,
        task_id: 9,
        name: name.into(),
        host: "host-a".into(),
        start_mode: "dispatch".into(),
        state: "ended".into(),
        started_at: "2026-10-16 09:00:00.000".into(),
        ended_at: ended_at.into(),
        end_reason: "worktree_gone".into(),
        ..dispatch_spacetime_module::blank_agent().into()
    }
}

#[test]
fn an_assigned_agent_decodes_with_no_launch_facts() {
    let agent = crate::sync::decode::agent(&assigned(1, "ada")).unwrap();
    assert_eq!(agent.state, AgentState::Assigned);
    assert_eq!(agent.start_mode, None);
    assert_eq!(agent.started_at, None);
    assert!(!agent.is_current() && !agent.is_live());
}

#[test]
fn a_backlog_task_shows_its_assigned_agents_name_and_no_session() {
    let rows = SharedRows::new();
    rows.upsert_task(&task());
    rows.upsert_agent(&assigned(1, "ada"));
    let shown = rows.task(TaskId(9)).unwrap();
    assert_eq!(shown.agent_name.as_deref(), Some("ada"));
    assert!(shown.tmux_window.is_none());
    assert!(
        rows.current_agent(TaskId(9)).is_none(),
        "assigned is not current"
    );
}

#[test]
fn a_task_whose_agents_ended_shows_the_one_that_ended_last() {
    let rows = SharedRows::new();
    rows.upsert_task(&task());
    rows.upsert_agent(&ended(2, "linus", "2026-10-17 10:00:00.000"));
    rows.upsert_agent(&ended(1, "ada", "2026-10-16 10:00:00.000"));
    assert_eq!(
        rows.task(TaskId(9)).unwrap().agent_name.as_deref(),
        Some("linus")
    );
    rows.remove_agent(crate::models::AgentId(2));
    assert_eq!(
        rows.task(TaskId(9)).unwrap().agent_name.as_deref(),
        Some("ada")
    );
    rows.remove_agent(crate::models::AgentId(1));
    assert_eq!(rows.task(TaskId(9)).unwrap().agent_name, None);
}
