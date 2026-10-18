use super::*;

// -- agents A4 (task #90118): the part the in-memory store cannot run -----------
//
// `Table::delete(row)` on `task_subagents` (no primary key) is a raw host call
// the in-memory store of `spacetime/module-tests/` does not serve, so this case,
// which deletes an existing entry, stays on the live store. The rest of the
// agent cases are in `spacetime/module-tests/tests/agents.rs`.

/// `BackfillCurrentAgents`, A4: a store published before A4 holds the facts on
/// its Task rows. The backfill moves them onto the current agent, points the
/// task's subagent entries at it, and blanks the Task. A second run changes
/// nothing, and the agent's own newer fact is not overwritten.
#[test]
fn the_backfill_moves_a_pre_a4_tasks_session_facts_onto_its_agent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    let mut row = task_json(1, "t", "running", 0, "host-a");
    row["owner"] = serde_json::json!("user-me");
    row["worktree"] = serde_json::json!("/wt/1-t");
    row["tmux_window"] = serde_json::json!("task-1");
    row["last_pre_tool_use_at"] = serde_json::json!("2026-10-16 11:00:00");
    row["last_notification_at"] = serde_json::json!("2026-10-16 11:30:00");
    row["live_subagents"] = serde_json::json!(2);
    row["stop_pending"] = serde_json::json!(true);
    row["stop_pending_at"] = serde_json::json!("2026-10-16 11:45:00.000");
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let entries = serde_json::json!([
        {"task_id": 1, "agent_id": "a1", "session_id": "s1", "started_at": "t", "agent_row": 0},
        {"task_id": 1, "agent_id": "a2", "session_id": "s1", "started_at": "t", "agent_row": 0},
    ]);
    let seeded = instance.call("seed_task_subagents", &[&entries.to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));

    backfill_agents(&instance);

    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        "2026-10-16 11:00:00"
    );
    assert_eq!(
        current_agent_column(&instance, "last_notification_at", 1),
        "2026-10-16 11:30:00"
    );
    assert_eq!(current_agent_column(&instance, "live_subagents", 1), "2");
    assert_eq!(current_agent_column(&instance, "stop_pending", 1), "true");
    assert_eq!(
        current_agent_column(&instance, "stop_pending_at", 1),
        "2026-10-16 11:45:00.000"
    );
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "live_subagents", 1), "0");
    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM task_subagents WHERE agent_row = 0"
        ),
        "0"
    );

    // A newer agent fact survives a second run: the Task is blank now.
    let tool = instance.call(
        "record_pre_tool_use",
        &["1", "active", "2026-10-16 12:00:00"],
    );
    assert!(tool.status.success(), "{}", describe(&tool));
    backfill_agents(&instance);
    assert_eq!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        "2026-10-16 12:00:00"
    );
}
