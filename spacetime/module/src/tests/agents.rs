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
}

impl AgentStore for Rows {
    fn agents_of_task(&self, task_id: i64) -> Vec<Agent> {
        self.agents.iter().filter(|a| a.task_id == task_id).cloned().collect()
    }
    fn name_is_handed_out(&self, name: &str) -> bool {
        self.ledger.iter().any(|r| r.name == name)
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
    fn insert_handed_out_name(&mut self, row: HandedOutAgentName) {
        assert!(!self.name_is_handed_out(&row.name), "{} handed out twice", row.name);
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
    Task {
        id,
        ..blank_task()
    }
}

fn with_worktree(id: i64, window: &str) -> Task {
    Task {
        worktree: format!("/repo/.worktrees/{id}-x"),
        host: "host-a".into(),
        tmux_window: window.into(),
        ..task(id)
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
fn every_agent_name_is_short_unique_and_not_an_alias_word() {
    let mut seen = std::collections::HashSet::new();
    for name in AGENT_NAMES {
        assert!(
            (3..=7).contains(&name.len()) && name.chars().all(|c| c.is_ascii_lowercase()),
            "{name}"
        );
        assert!(seen.insert(*name), "duplicate name {name}");
        assert!(!ALIAS_WORDS.contains(name), "{name} is also an alias word");
        assert!(validate_alias(name).is_ok(), "{name}");
    }
    assert!(AGENT_NAMES.len() >= 500, "{}", AGENT_NAMES.len());
    for kept in ["ada", "linus", "grace"] {
        assert!(AGENT_NAMES.contains(&kept), "{kept}");
    }
}

#[test]
fn the_name_list_leaves_out_tool_names() {
    for tool in ["git", "gh", "tmux", "claude", "cargo", "rust", "ruby", "perl", "julia"] {
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
fn the_new_agent_takes_the_tasks_window() {
    let mut rows = Rows::default();
    start_agent_on_fresh_launch(&mut rows, &with_worktree(1, "task-1"), "host-a", "dispatch");
    assert_eq!(rows.agents[0].tmux_window, "task-1");
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

// -- task writes: CurrentAgentMirrorsTheWindow, EndAgentWhenWorktreeGoes ------

#[test]
fn a_window_write_mirrors_onto_the_current_agent() {
    let mut rows = Rows::default();
    let prior = task(1);
    start_agent_on_fresh_launch(&mut rows, &prior, "host-a", "dispatch");
    let provisioned = with_worktree(1, "task-1");
    follow_task_write(&mut rows, Some(&prior), &provisioned);
    assert_eq!(rows.agents[0].tmux_window, "task-1");
    // A pause (wrap-up, exit_session) clears it; the agent stays current.
    let paused = with_worktree(1, "");
    follow_task_write(&mut rows, Some(&provisioned), &paused);
    assert_eq!(rows.agents[0].tmux_window, "");
    assert_eq!(rows.agents[0].state, AGENT_CURRENT);
    // ResumeKeepsTheAgent: a resume's window write brings the same agent back.
    let resumed = with_worktree(1, "task-1");
    follow_task_write(&mut rows, Some(&paused), &resumed);
    assert_eq!(rows.agents.len(), 1);
    assert_eq!(rows.agents[0].tmux_window, "task-1");
}

#[test]
fn clearing_the_worktree_ends_the_agent_as_worktree_gone() {
    let mut rows = Rows::default();
    let live = with_worktree(1, "task-1");
    start_agent_on_fresh_launch(&mut rows, &live, "host-a", "dispatch");
    follow_task_write(&mut rows, Some(&live), &task(1));
    let agent = &rows.agents[0];
    assert_eq!(agent.state, AGENT_ENDED);
    assert_eq!(agent.end_reason, AGENT_END_WORKTREE_GONE);
    assert_eq!(agent.tmux_window, "");
}

#[test]
fn a_write_that_moves_nothing_an_agent_mirrors_changes_nothing() {
    let live = with_worktree(1, "task-1");
    let retitled = Task {
        title: "new".into(),
        ..live.clone()
    };
    assert_eq!(agent_follow_up(Some(&live), &retitled), AgentFollowUp::Nothing);
    assert_eq!(agent_follow_up(None, &live), AgentFollowUp::Nothing);
}

#[test]
fn a_window_write_on_a_task_without_an_agent_writes_nothing() {
    let mut rows = Rows::default();
    follow_task_write(&mut rows, Some(&task(1)), &with_worktree(1, "task-1"));
    assert!(rows.agents.is_empty());
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
