use super::*;

// -- Agent session state (Phase 6b) ------------------------------------------
//
// A handful of scenarios a unit test with a recording caller cannot check,
// the same reason this file exists at all: whether the REAL module reproduces
// `docs/specs/agent-health.allium`'s guarantees, not whether a mock assumed it
// would. `src/store/tests/over_sync/writes.rs` covers the `Store`'s decoding far more
// exhaustively; these are chosen to hit the module's own logic that decoding
// can't reach, especially the no-primary-key dedupe hazard an adversarial
// review of this task's plan flagged (`docs/plans/
// 2026-09-21-phase-6b-agent-session-state-reducers.md`, decision 7).

/// `live_subagents` counts real entries, and a repeated start for the SAME
/// agent does not double it. `task_subagents` has no primary key in the
/// module (unlike SQLite's `PRIMARY KEY (task_id, agent_id)`), so this is the
/// one property that hazard could actually break: `subagent_start` finding
/// and deleting the prior row by value before inserting the fresh one, not
/// merely fencing the session.
#[test]
fn a_repeated_subagent_start_for_the_same_agent_does_not_duplicate() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let first = instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:00Z"],
    );
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:01Z"],
    );
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM task_subagents WHERE task_id = 1"
        ),
        "1",
        "a repeated start for the same agent must replace, not duplicate, its row"
    );
    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "1"
    );
}

/// A second, distinct agent DOES raise the count — the dedupe above is keyed
/// on `agent_id`, not on the task.
#[test]
fn two_different_subagents_both_count() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:00Z"],
    );
    instance.call(
        "subagent_start",
        &["1", "agent-2", "session-1", "2026-09-19T10:00:00Z"],
    );

    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "2"
    );

    let stopped = instance.call("subagent_stop", &["1", "agent-1", "session-1"]);
    assert!(stopped.status.success(), "{}", describe(&stopped));
    assert_eq!(
        column(&instance, "SELECT live_subagents FROM tasks WHERE id = 1"),
        "1"
    );
}

/// `HookStop`: nothing live, so the Stop flips the task straight to `review`.
#[test]
fn try_record_stop_flips_a_running_task_with_nothing_live() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let stopped = instance.call("try_record_stop", &["1", "2026-09-19T10:00:00.000"]);
    assert!(stopped.status.success(), "{}", describe(&stopped));

    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "review"
    );
}

/// ...and a live subagent withholds the flip until it drains, at which point
/// the DRAIN — not a second Stop — applies it. `HookStop`/`HookSubagentStop`
/// in `docs/specs/agent-health.allium`.
#[test]
fn try_record_stop_defers_while_a_subagent_is_live_and_the_drain_applies_it() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);
    instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-09-19T10:00:00Z"],
    );

    let stopped = instance.call("try_record_stop", &["1", "2026-09-19T10:00:01.000"]);
    assert!(stopped.status.success(), "{}", describe(&stopped));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running",
        "a live subagent must withhold the flip"
    );
    assert_eq!(
        column(&instance, "SELECT stop_pending FROM tasks WHERE id = 1"),
        "true"
    );

    let drained = instance.call("subagent_stop", &["1", "agent-1", "session-1"]);
    assert!(drained.status.success(), "{}", describe(&drained));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "review",
        "draining the last subagent must apply the deferred Stop"
    );
    assert_eq!(
        column(&instance, "SELECT stop_pending FROM tasks WHERE id = 1"),
        "false"
    );
}

/// A Stop against a task that is not `Running` is refused outright — the
/// precondition failing is what lets the `Store` read `NoOp` back
/// unambiguously (this task's plan doc, decision 3) — rather than a silent
/// no-op the caller could not tell apart from a real flip.
#[test]
fn try_record_stop_refuses_a_task_that_is_not_running() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let mut row = task_json(1, "t", "backlog", 0, "");
    row["owner"] = serde_json::json!("user-me");
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let refused = instance.call("try_record_stop", &["1", "2026-09-19T10:00:00.000"]);
    assert!(
        !refused.status.success(),
        "a Stop against a non-Running task must be refused, got {}",
        describe(&refused)
    );
}

/// The PR learnings gate fires EXACTLY once: the first call sets it and wins,
/// a second is refused. The `Store` reads that refusal as `false` via
/// `ReducerOutcome::won()` — the same shape as the dispatch claim.
#[test]
fn mark_pr_learnings_gate_shown_wins_exactly_once() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    running_task(&instance, 1);

    let first = instance.call(
        "mark_pr_learnings_gate_shown",
        &["1", "2026-09-19T10:00:00.000"],
    );
    assert!(first.status.success(), "{}", describe(&first));

    let second = instance.call(
        "mark_pr_learnings_gate_shown",
        &["1", "2026-09-19T10:00:01.000"],
    );
    assert!(
        !second.status.success(),
        "a second call must be refused, got {}",
        describe(&second)
    );
}
