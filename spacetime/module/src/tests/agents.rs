use super::*;

const NOW: &str = "2026-10-16 12:00:00.000";

/// A plain `AgentStore` over vectors: the smallest store the generic rules
/// can run against, so each rule is pinned without a `ReducerContext`.
#[derive(Default)]
struct Rows {
    agents: Vec<Agent>,
    ledger: Vec<HandedOutAgentName>,
    next_id: i64,
    seed: u64,
    /// `(task_id, agent_row)` of every SubagentEntry.
    entries: Vec<(i64, i64)>,
    /// Task ids whose Task row was blanked of session facts.
    blanked: Vec<i64>,
}

impl AgentStore for Rows {
    fn agents_of_task(&self, task_id: i64) -> Vec<Agent> {
        self.agents
            .iter()
            .filter(|a| a.task_id == task_id)
            .cloned()
            .collect()
    }
    fn name_is_handed_out(&self, name: &str) -> bool {
        self.ledger.iter().any(|r| r.name == name)
    }
    fn name_held_by_agent(&self, name: &str) -> bool {
        self.agents.iter().any(|a| a.name == name)
    }
    fn insert_agent(&mut self, row: Agent) {
        self.next_id += 1;
        self.agents.push(Agent {
            id: self.next_id,
            ..row
        });
    }
    fn update_agent(&mut self, row: Agent) {
        let slot = self.agents.iter_mut().find(|a| a.id == row.id).unwrap();
        *slot = row;
    }
    fn delete_agent(&mut self, id: i64) {
        self.agents.retain(|a| a.id != id);
    }
    fn delete_subagent_entries_of(&mut self, task_id: i64) {
        self.entries.retain(|(t, _)| *t != task_id);
    }
    fn adopt_subagent_entries(&mut self, task_id: i64, agent_id: i64) {
        for e in self
            .entries
            .iter_mut()
            .filter(|(t, a)| *t == task_id && *a == 0)
        {
            e.1 = agent_id;
        }
    }
    fn blank_task_session_facts(&mut self, task_id: i64) {
        self.blanked.push(task_id);
    }
    fn insert_handed_out_name(&mut self, row: HandedOutAgentName) {
        assert!(
            !self.name_is_handed_out(&row.name),
            "{} handed out twice",
            row.name
        );
        self.ledger.push(row);
    }
    fn now(&self) -> String {
        NOW.into()
    }
    fn name_seed(&self) -> u64 {
        self.seed
    }
}

fn task(id: i64) -> Task {
    Task { id, ..blank_task() }
}

fn with_worktree(id: i64, window: &str) -> Task {
    Task {
        worktree: format!("/repo/.worktrees/{id}-x"),
        host: "host-a".into(),
        tmux_window: window.into(),
        ..task(id)
    }
}

/// A task as a store published before A4 left it: a live session's facts on
/// the Task row.
fn pre_a4_live(id: i64) -> Task {
    Task {
        last_pre_tool_use_at: "2026-10-16 11:00:00".into(),
        last_notification_at: "2026-10-16 11:30:00".into(),
        live_subagents: 2,
        stop_pending: true,
        stop_pending_at: "2026-10-16 11:45:00.000".into(),
        ..with_worktree(id, "task-1")
    }
}

fn current(rows: &Rows, task_id: i64) -> Vec<&Agent> {
    rows.agents
        .iter()
        .filter(|a| a.task_id == task_id && a.state == AGENT_CURRENT)
        .collect()
}

// -- the name list (decision A11) ---------------------------------------------

#[test]
fn every_agent_name_is_short_and_unique() {
    let mut seen = std::collections::HashSet::new();
    for name in AGENT_NAMES {
        assert!(
            (3..=7).contains(&name.len()) && name.chars().all(|c| c.is_ascii_lowercase()),
            "{name}"
        );
        assert!(seen.insert(*name), "duplicate name {name}");
        assert!(validate_agent_name(name).is_ok(), "{name}");
    }
    assert!(AGENT_NAMES.len() >= 500, "{}", AGENT_NAMES.len());
    for kept in ["ada", "linus", "grace"] {
        assert!(AGENT_NAMES.contains(&kept), "{kept}");
    }
}

#[test]
fn the_name_list_leaves_out_tool_names() {
    for tool in [
        "git", "gh", "tmux", "claude", "cargo", "rust", "ruby", "perl", "julia",
    ] {
        assert!(!AGENT_NAMES.contains(&tool), "{tool}");
    }
}

#[test]
fn the_picker_reads_the_ledger_and_falls_back_to_a_number() {
    assert_eq!(pick_agent_name(0, |_| false), AGENT_NAMES[0]);
    assert_eq!(pick_agent_name(0, |n| n == AGENT_NAMES[0]), AGENT_NAMES[1]);
    let all: std::collections::HashSet<&str> = AGENT_NAMES.iter().copied().collect();
    assert_eq!(
        pick_agent_name(3, |n| all.contains(n)),
        format!("{}2", AGENT_NAMES[3])
    );
}

#[test]
fn a_picked_name_is_never_only_digits() {
    // AgentNameIsNotAnId.
    for seed in [0u64, 7, u64::MAX] {
        let name = pick_agent_name(seed, |_| false);
        assert!(!name.chars().all(|c| c.is_ascii_digit()), "{name}");
    }
}

// -- StartAgentOnFreshLaunch --------------------------------------------------

#[test]
fn a_fresh_launch_starts_a_current_agent_and_hands_out_its_name() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "research");
    let agent = &rows.agents[0];
    assert_eq!(agent.task_id, 1);
    assert_eq!(agent.state, AGENT_CURRENT);
    assert_eq!(agent.host, "host-a");
    assert_eq!(agent.start_mode, "research");
    assert_eq!(agent.started_at, NOW);
    assert_eq!(rows.ledger.len(), 1);
    assert_eq!(rows.ledger[0].name, agent.name);
}

#[test]
fn a_second_fresh_launch_ends_the_first_agent_as_replaced() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &with_worktree(1, "task-1"), "host-a", "dispatch");
    start_agent_on_fresh_launch(&mut rows, &with_worktree(1, ""), "host-a", "dispatch");
    assert_eq!(current(&rows, 1).len(), 1, "OneCurrentAgentPerTask");
    let old = &rows.agents[0];
    assert_eq!(old.state, AGENT_ENDED);
    assert_eq!(old.end_reason, AGENT_END_REPLACED);
    assert_eq!(old.ended_at, NOW);
    assert_eq!(old.tmux_window, "", "EndedAgentHoldsNothing");
    assert_ne!(rows.agents[0].name, rows.agents[1].name);
}

#[test]
fn the_new_agent_starts_without_a_window_and_stamped_at_the_claim() {
    // StartAgentOnFreshLaunch: no window until the host reports one; the
    // activity stamp is the claim's, so the watchdog measures from it.
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &with_worktree(1, "task-1"), "host-a", "dispatch");
    let agent = &rows.agents[0];
    assert_eq!(agent.tmux_window, "");
    assert_eq!(agent.last_pre_tool_use_at, NOW);
    assert_eq!(agent.last_notification_at, "");
    assert_eq!(agent.live_subagents, 0);
    assert!(!agent.stop_pending);
}

#[test]
fn a_replaced_agent_takes_its_subagent_entries_and_pending_stop_with_it() {
    // EndedAgentHoldsNothing, A16.
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    update_current_agent(&mut rows, 1, |a| Agent {
        tmux_window: "task-1".into(),
        live_subagents: 2,
        stop_pending: true,
        ..a
    });
    rows.entries.push((1, 1));
    rows.entries.push((1, 1));
    rows.entries.push((2, 9));
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    let old = &rows.agents[0];
    assert_eq!(old.state, AGENT_ENDED);
    assert_eq!(old.live_subagents, 0);
    assert!(!old.stop_pending);
    assert_eq!(
        rows.entries,
        vec![(2, 9)],
        "only the other task's entry is left"
    );
}

#[test]
fn a_name_freed_by_a_deleted_task_is_never_handed_out_again() {
    // AgentNameIsNeverReused: the picker reads the ledger, which a delete
    // does not touch.
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    let first = rows.agents[0].name.clone();
    delete_agents_of(&mut rows, 1);
    assert!(rows.agents.is_empty());
    start_agent_on_fresh_launch(&mut rows, &task(2), "host-a", "dispatch");
    assert_ne!(rows.agents[0].name, first);
    assert_eq!(rows.ledger.len(), 2);
}

// -- session facts live on the agent (A4) ----------------------------------------

#[test]
fn a_session_fact_is_written_on_the_current_agent_only() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    assert!(update_current_agent(&mut rows, 1, |a| Agent {
        tmux_window: "task-1".into(),
        ..a
    }));
    assert_eq!(rows.agents[0].tmux_window, "task-1");
    // A pause clears it; the agent stays current (ResumeKeepsTheAgent: a
    // resume's window write brings the same agent back).
    close_agent_session(&mut rows, 1);
    assert_eq!(rows.agents[0].tmux_window, "");
    assert_eq!(rows.agents[0].state, AGENT_CURRENT);
    update_current_agent(&mut rows, 1, |a| Agent {
        tmux_window: "task-1".into(),
        ..a
    });
    assert_eq!(rows.agents.len(), 1);
}

#[test]
fn a_session_fact_for_a_task_without_an_agent_writes_nothing() {
    // Decision A15.
    let mut rows = Rows::default();
    assert!(!update_current_agent(&mut rows, 1, |a| Agent {
        tmux_window: "task-1".into(),
        ..a
    }));
    assert!(rows.agents.is_empty());
}

#[test]
fn a_close_clears_the_window_and_the_pending_stop() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    update_current_agent(&mut rows, 1, |a| Agent {
        tmux_window: "task-1".into(),
        stop_pending: true,
        live_subagents: 1,
        ..a
    });
    close_agent_session(&mut rows, 1);
    assert_eq!(session_window(&rows, 1), "");
    assert!(!rows.agents[0].stop_pending);
    assert_eq!(
        rows.agents[0].live_subagents, 1,
        "only the drain clears the count"
    );
}

fn named(patch: TaskPatch) -> TaskPatch {
    patch
}

#[test]
fn a_patch_names_session_facts_for_the_agent() {
    let patch = named(TaskPatch {
        tmux_window: Some("task-1".into()),
        last_pre_tool_use_at: Some("2026-10-16 12:00:00".into()),
        live_subagents: Some(3),
        stop_pending: Some(true),
        ..TaskPatch::default()
    });
    assert!(patch_names_session_facts(&patch));
    assert!(!patch_names_session_facts(&TaskPatch {
        title: Some("x".into()),
        ..TaskPatch::default()
    }));
    let applied = apply_session_patch(blank_agent(), &patch);
    assert_eq!(applied.tmux_window, "task-1");
    assert_eq!(applied.last_pre_tool_use_at, "2026-10-16 12:00:00");
    assert_eq!(applied.live_subagents, 3);
    assert!(applied.stop_pending);
    assert_eq!(
        applied.last_notification_at, "",
        "an unnamed fact is left alone"
    );
}

#[test]
fn a_patch_that_sets_a_fact_needs_an_agent_but_one_that_clears_does_not() {
    let sets = TaskPatch {
        tmux_window: Some("task-1".into()),
        ..TaskPatch::default()
    };
    assert!(check_session_patch(false, &sets).is_err());
    assert!(check_session_patch(true, &sets).is_ok());
    let clears = TaskPatch {
        tmux_window: Some(String::new()),
        live_subagents: Some(0),
        stop_pending: Some(false),
        ..TaskPatch::default()
    };
    assert!(check_session_patch(false, &clears).is_ok());
}

#[test]
fn clearing_the_worktree_ends_the_agent_as_worktree_gone() {
    let mut rows = Rows::default();
    let live = with_worktree(1, "task-1");
    start_agent_on_fresh_launch(&mut rows, &live, "host-a", "dispatch");
    update_current_agent(&mut rows, 1, |a| Agent {
        tmux_window: "task-1".into(),
        ..a
    });
    rows.entries.push((1, 1));
    follow_task_write(&mut rows, Some(&live), &task(1));
    let agent = &rows.agents[0];
    assert_eq!(agent.state, AGENT_ENDED);
    assert_eq!(agent.end_reason, AGENT_END_WORKTREE_GONE);
    assert_eq!(agent.tmux_window, "");
    assert!(rows.entries.is_empty(), "an ended agent holds no subagents");
}

#[test]
fn a_write_that_moves_nothing_an_agent_follows_changes_nothing() {
    let live = with_worktree(1, "task-1");
    let retitled = Task {
        title: "new".into(),
        ..live.clone()
    };
    assert_eq!(
        agent_follow_up(Some(&live), &retitled),
        AgentFollowUp::Nothing
    );
    assert_eq!(agent_follow_up(None, &live), AgentFollowUp::Nothing);
}

// -- EndAgentWhenLaunchFails --------------------------------------------------

#[test]
fn a_failed_launch_ends_the_agent_as_launch_failed() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    end_current_agent(&mut rows, 1, AGENT_END_LAUNCH_FAILED);
    assert_eq!(rows.agents[0].state, AGENT_ENDED);
    assert_eq!(rows.agents[0].end_reason, AGENT_END_LAUNCH_FAILED);
    assert!(current(&rows, 1).is_empty());
}

// -- AgentsGoWithTheirTask ----------------------------------------------------

#[test]
fn deleting_a_task_removes_every_agent_it_had_and_keeps_the_names() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    start_agent_on_fresh_launch(&mut rows, &task(2), "host-a", "dispatch");
    delete_agents_of(&mut rows, 1);
    assert!(rows.agents.iter().all(|a| a.task_id == 2));
    assert_eq!(rows.ledger.len(), 3);
}

// -- BackfillCurrentAgents ----------------------------------------------------

#[test]
fn the_backfill_names_every_task_with_a_worktree_and_no_current_agent() {
    let mut rows = Rows::default();
    let has_agent = with_worktree(3, "task-3");
    start_agent_on_fresh_launch(&mut rows, &has_agent, "host-a", "dispatch");
    let tasks = vec![
        with_worktree(1, "task-1"),
        task(2),
        has_agent,
        Task {
            status: DONE.into(),
            ..with_worktree(4, "")
        },
    ];
    backfill_agents_into(&mut rows, tasks.clone());
    for id in [1, 3, 4] {
        assert_eq!(current(&rows, id).len(), 1, "task {id}");
    }
    assert!(current(&rows, 2).is_empty(), "no worktree, no agent");
    let backfilled = current(&rows, 1)[0];
    assert_eq!(backfilled.host, "host-a");
    assert_eq!(backfilled.start_mode, AGENT_BACKFILL_START_MODE);
    assert_eq!(backfilled.tmux_window, "task-1");
    // Idempotent.
    let before = rows.agents.len();
    backfill_agents_into(&mut rows, tasks);
    assert_eq!(rows.agents.len(), before);
}

#[test]
fn the_backfill_moves_a_pre_a4_tasks_session_facts_onto_its_agent() {
    // BackfillCurrentAgents, A4: the facts, the entries, then the blanking.
    let mut rows = Rows::default();
    let live = pre_a4_live(1);
    start_agent_on_fresh_launch(&mut rows, &live, "host-a", "dispatch");
    rows.entries.extend([(1, 0), (1, 0)]);
    backfill_agents_into(&mut rows, vec![live]);
    let agent = current(&rows, 1)[0];
    assert_eq!(agent.tmux_window, "task-1");
    assert_eq!(agent.last_pre_tool_use_at, "2026-10-16 11:00:00");
    assert_eq!(agent.last_notification_at, "2026-10-16 11:30:00");
    assert_eq!(agent.live_subagents, 2);
    assert!(agent.stop_pending);
    assert_eq!(agent.stop_pending_at, "2026-10-16 11:45:00.000");
    let agent_id = agent.id;
    assert_eq!(rows.entries, vec![(1, agent_id), (1, agent_id)]);
    assert_eq!(rows.blanked, vec![1]);
}

#[test]
fn a_second_backfill_cannot_overwrite_a_newer_agent_fact() {
    // The Task facts are blank after the first pass, so a rerun copies nothing.
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &task(1), "host-a", "dispatch");
    update_current_agent(&mut rows, 1, |a| Agent {
        live_subagents: 4,
        ..a
    });
    backfill_agents_into(&mut rows, vec![with_worktree(1, "")]);
    assert_eq!(rows.agents[0].live_subagents, 4);
    assert!(rows.blanked.is_empty());
}

#[test]
fn the_backfill_drops_the_session_facts_of_a_task_with_no_agent_to_hold_them() {
    let mut rows = Rows::default();
    rows.entries.push((1, 0));
    let stale = Task {
        stop_pending: true,
        ..task(1)
    };
    backfill_agents_into(&mut rows, vec![stale]);
    assert!(rows.agents.is_empty());
    assert!(rows.entries.is_empty());
    assert_eq!(rows.blanked, vec![1]);
}

#[test]
fn a_task_row_holds_no_session_fact_after_a_write() {
    let written = without_session_facts(pre_a4_live(1));
    assert!(!task_holds_session_facts(&written));
    assert_eq!(written.worktree, pre_a4_live(1).worktree);
}

/// A5 retires the task alias from `write_task`, which bumped the version to 9.
/// Later rule changes bump it further, so this pins only the floor. The
/// board's own copy is pinned equal to this one by `src/sync/rules_gate.rs`'s
/// test.
#[test]
fn the_rules_version_is_at_least_nine_since_agents_a5() {
    assert!(RULES_VERSION >= 9);
}

// -- seed_agents' row checks ----------------------------------------------------

fn seeded(state: &str, end_reason: &str, window: &str, name: &str) -> Agent {
    Agent {
        id: 1,
        task_id: 1,
        name: name.into(),
        start_mode: "dispatch".into(),
        state: state.into(),
        end_reason: end_reason.into(),
        tmux_window: window.into(),
        ..blank_agent()
    }
}

#[test]
fn a_seeded_agent_must_keep_the_row_invariants() {
    assert!(validate_seeded_agent(&seeded("current", "", "task-1", "ada")).is_ok());
    assert!(validate_seeded_agent(&seeded("ended", "replaced", "", "ada")).is_ok());
    for bad in [
        seeded("ended", "replaced", "task-1", "ada"),
        seeded("ended", "", "", "ada"),
        seeded("current", "replaced", "", "ada"),
        seeded("current", "", "", ""),
        seeded("current", "", "", "123"),
        seeded("paused", "", "", "ada"),
        Agent {
            live_subagents: 1,
            ..seeded("ended", "replaced", "", "ada")
        },
        Agent {
            stop_pending: true,
            ..seeded("ended", "replaced", "", "ada")
        },
        Agent {
            start_mode: "resume".into(),
            ..seeded("current", "", "", "ada")
        },
    ] {
        assert!(validate_seeded_agent(&bad).is_err(), "{bad:?}");
    }
}
