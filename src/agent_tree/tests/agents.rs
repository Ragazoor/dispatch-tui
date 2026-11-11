//! The agents section and its read through the board.

use super::*;

// ---- agents section (docs/specs/agent-tree.allium: Agents Section) ----

#[test]
fn the_pane_starts_on_the_agents_view() {
    let state = RenderState::new();
    assert_eq!(state.focus, Focus::Agents);
    assert_eq!(state.browse_focus, Focus::Tree);
}

/// SwitchAgentTreeView: `s` flips from the browse view to the agents view and
/// back, landing on the browse section the user left.
#[test]
fn s_flips_between_the_browse_view_and_the_agents_view() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(1, false)]);
    assert_eq!(rig.press(KeyCode::Char('s')), KeyAction::Continue);
    assert_eq!(rig.state.focus, Focus::Agents);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// An empty agents list is a real place to be: `s` never skips it.
#[test]
fn s_works_with_no_agents_listed() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Agents);
}

/// SwitchAgentTreeBrowseFocus: Tab moves between tree and commits.
#[test]
fn tab_moves_between_the_tree_and_the_commits_section() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    assert_eq!(rig.press(KeyCode::Tab), KeyAction::Continue);
    assert_eq!(rig.state.focus, Focus::Commits);
    assert_eq!(rig.state.browse_focus, Focus::Commits);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// SwitchAgentTreeBrowseFocus: with two sections the two keys are the same
/// move.
#[test]
fn back_tab_moves_between_the_sections_like_tab() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press_with(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(rig.state.focus, Focus::Commits);
    rig.press_with(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// `s` returns to the browse section that was focused when the user left.
#[test]
fn flipping_back_returns_to_the_commits_section_when_that_is_where_you_were() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Tab);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Agents);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Commits);
}

/// In the agents view there is only one section, so Tab and Shift+Tab have
/// nowhere to go, and record nothing.
#[test]
fn tab_does_nothing_in_the_agents_view() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Char('s'));
    rig.state.usage.clear();
    rig.press(KeyCode::Tab);
    rig.press_with(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(rig.state.focus, Focus::Agents);
    assert_eq!(rig.state.browse_focus, Focus::Tree);
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
}

#[test]
fn s_clears_a_notice() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.state.notice = Some(Notice::agent_list("db locked"));
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.notice, None);
}

// The agents-section tests below set focus directly: they are about the
// section, not about how `s` reaches it (SwitchAgentTreeView, above).

#[test]
fn with_the_agents_section_focused_j_and_k_move_its_cursor_not_the_trees() {
    let mut rig = KeyRig::with_agents(
        &[modified("a.rs"), modified("b.rs")],
        vec![agent(1, false), agent(2, false)],
    );
    let tree_before = rig.selected();
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.state.agents.selected().unwrap().id, TaskId(2));
    rig.press(KeyCode::Up);
    assert_eq!(rig.state.agents.selected().unwrap().id, TaskId(1));
    assert_eq!(rig.selected(), tree_before);
}

#[test]
fn with_the_agents_section_focused_gg_and_capital_g_jump_its_cursor() {
    let mut rig = KeyRig::with_agents(
        &[modified("a.rs")],
        vec![agent(1, false), agent(2, false), agent(3, false)],
    );
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.state.agents.selected().unwrap().id, TaskId(3));
    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.state.agents.selected().unwrap().id, TaskId(1));
    rig.press_ctrl(KeyCode::Char('d'));
    assert_ne!(rig.state.agents.selected().unwrap().id, TaskId(1));
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.state.agents.selected().unwrap().id, TaskId(1));
}

#[test]
fn space_on_another_agent_jumps_to_its_window() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(7, false)]);
    rig.state.focus = Focus::Agents;
    assert_eq!(
        rig.press(KeyCode::Char(' ')),
        KeyAction::JumpTo(test_tmux_window("task-7"))
    );
    assert_eq!(
        rig.press(KeyCode::Enter),
        KeyAction::JumpTo(test_tmux_window("task-7"))
    );
}

/// ResumeDetachedAgentFromThePane: Space or Enter on a detached row asks the
/// loop to resume it, not to jump.
#[test]
fn space_on_a_detached_agent_requests_a_resume() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![detached(7)]);
    rig.state.focus = Focus::Agents;
    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Resume(TaskId(7)));
    assert_eq!(rig.press(KeyCode::Enter), KeyAction::Resume(TaskId(7)));
}

// ---- The resume's answer and the jump that follows it ----------------

fn pane_with(rows: Vec<AgentRow>) -> RenderState {
    let mut state = RenderState::new();
    state.adopt_agent_list(Ok(rows));
    state
}

/// AgentTreeResumeAccepted: the pane waits for the agent's window, and says
/// nothing — the notice is for failures.
#[test]
fn an_accepted_resume_waits_for_the_window_without_a_notice() {
    let mut state = pane_with(vec![detached(7)]);
    state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
    );
    assert_eq!(state.pending_jump, Some(TaskId(7)));
    assert_eq!(state.notice, None);
}

/// AgentTreeResumeRefusedIsVisible: the board's reason is the notice, and
/// nothing waits.
#[test]
fn a_refused_resume_shows_the_boards_reason() {
    let mut state = pane_with(vec![detached(7)]);
    state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Refused {
            reason: "its worktree is on another machine".into(),
        }),
    );
    assert_eq!(state.pending_jump, None);
    assert_eq!(
        state.notice,
        Some(Notice::agent_jump("its worktree is on another machine"))
    );
}

/// A board that could not be reached is a refusal too, naming why.
#[test]
fn an_unreachable_board_on_resume_is_a_visible_notice() {
    let mut state = pane_with(vec![detached(7)]);
    state.adopt_resume_answer(TaskId(7), Err("could not reach the board".into()));
    assert_eq!(state.pending_jump, None);
    assert_eq!(
        state.notice,
        Some(Notice::agent_jump("could not reach the board"))
    );
}

/// RefreshAgentTreeAgentList: the read that shows the resumed agent attached
/// hands the loop its window to jump to, once.
#[test]
fn the_read_that_shows_the_resumed_agent_attached_jumps_to_it_once() {
    let mut state = pane_with(vec![detached(7)]);
    state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
    );

    assert_eq!(state.adopt_agent_list(Ok(vec![detached(7)])), None);
    assert_eq!(state.pending_jump, Some(TaskId(7)));

    assert_eq!(
        state.adopt_agent_list(Ok(vec![agent(7, false)])),
        Some(test_tmux_window("task-7"))
    );
    assert_eq!(state.pending_jump, None);
    assert_eq!(state.adopt_agent_list(Ok(vec![agent(7, false)])), None);
}

/// `ResumeJumpWaitEnds`: `Accepted` only means "handed to the board". A launch
/// that fails there leaves the row detached for ever, so the wait ends after
/// a bounded number of list reads and the pane says so.
#[test]
fn a_resume_that_never_attaches_ends_the_wait_with_a_notice() {
    use crate::agent_tree::state::RESUME_JUMP_MAX_READS;
    let mut state = pane_with(vec![detached(7)]);
    state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
    );

    for _ in 0..RESUME_JUMP_MAX_READS - 1 {
        assert_eq!(state.adopt_agent_list(Ok(vec![detached(7)])), None);
        assert_eq!(state.pending_jump, Some(TaskId(7)), "still waiting");
        assert_eq!(state.notice, None);
    }
    assert_eq!(state.adopt_agent_list(Ok(vec![detached(7)])), None);

    assert_eq!(state.pending_jump, None);
    let Some(Notice::AgentJump(text)) = &state.notice else {
        panic!("expected an agent-jump notice, got {:?}", state.notice);
    };
    assert!(text.contains("did not start"), "{text}");
}

/// A fresh accepted resume starts a fresh wait.
#[test]
fn a_new_accepted_resume_restarts_the_wait() {
    use crate::agent_tree::state::RESUME_JUMP_MAX_READS;
    let mut state = pane_with(vec![detached(7)]);
    let accept = |state: &mut RenderState| {
        state.adopt_resume_answer(
            TaskId(7),
            Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
        )
    };
    accept(&mut state);
    for _ in 0..RESUME_JUMP_MAX_READS - 1 {
        state.adopt_agent_list(Ok(vec![detached(7)]));
    }
    accept(&mut state);
    state.adopt_agent_list(Ok(vec![detached(7)]));

    assert_eq!(state.pending_jump, Some(TaskId(7)));
    assert_eq!(state.notice, None);
}

/// RefreshAgentTreeAgentList: a resumed task that left the list stops the wait.
#[test]
fn a_resumed_task_that_leaves_the_list_stops_the_wait() {
    let mut state = pane_with(vec![detached(7), agent(8, false)]);
    state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
    );
    assert_eq!(state.adopt_agent_list(Ok(vec![agent(8, false)])), None);
    assert_eq!(state.pending_jump, None);
}

/// ForgetPendingResumeJump: moving the cursor off the resumed row stops the
/// wait, so a later attach does not jump the user away.
#[test]
fn moving_the_cursor_off_the_resumed_row_stops_the_wait() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![detached(7), agent(8, false)]);
    rig.state.focus = Focus::Agents;
    rig.state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
    );
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.state.pending_jump, None);
    assert_eq!(
        rig.state
            .adopt_agent_list(Ok(vec![agent(7, false), agent(8, false)])),
        None
    );
}

/// AgentTreeResumeAccepted: an answer that arrives after the user moved off
/// the row does not start a wait — the answer outlives the keypress.
#[test]
fn an_accepted_resume_after_the_cursor_moved_on_does_not_wait() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![detached(7), agent(8, false)]);
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char('j'));
    rig.state.adopt_resume_answer(
        TaskId(7),
        Ok(crate::hooks::wire::PaneResumeAnswer::Accepted),
    );
    assert_eq!(rig.state.pending_jump, None);
}

#[test]
fn space_on_the_panes_own_task_does_nothing() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(7, true)]);
    rig.state.focus = Focus::Agents;
    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Continue);
    assert_eq!(rig.state.notice, None);
}

#[test]
fn space_with_the_agents_section_focused_never_toggles_a_diff() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(7, true)]);
    rig.press(KeyCode::Char('j'));
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char(' '));
    assert!(rig.state.open_diffs().is_empty());
}

#[test]
fn q_still_exits_with_the_agents_section_focused() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(7, false)]);
    rig.state.focus = Focus::Agents;
    assert_eq!(rig.press(KeyCode::Char('q')), KeyAction::Exit);
}

#[test]
fn h_and_l_do_nothing_with_the_agents_section_focused() {
    let mut rig = KeyRig::with_agents(&[modified("src/a.rs")], vec![agent(7, false)]);
    let opened = rig.state.tree_state.opened().clone();
    rig.press(KeyCode::Char('j'));
    let selected = rig.selected();
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char('h'));
    rig.press(KeyCode::Left);
    rig.press(KeyCode::Char('l'));
    assert_eq!(rig.state.tree_state.opened(), &opened);
    assert_eq!(rig.selected(), selected);
}

// ---- RefreshAgentTreeAgentList / AgentTreeAgentListFailureKeepsLastList

#[test]
fn a_failed_agent_read_keeps_the_last_list_and_says_why() {
    let mut state = browse_state();
    state.adopt_agent_list(Ok(vec![agent(1, false)]));
    state.adopt_agent_list(Err("database is locked".to_string()));
    assert_eq!(state.agents.rows().len(), 1);
    assert_eq!(state.notice, Some(Notice::agent_list("database is locked")));
}

#[test]
fn a_working_agent_read_clears_only_its_own_notice() {
    let mut state = browse_state();
    state.notice = Some(Notice::agent_list("database is locked"));
    state.adopt_agent_list(Ok(vec![]));
    assert_eq!(state.notice, None);

    for other in [
        Notice::git("index.lock"),
        Notice::diff("split failed"),
        Notice::agent_jump("no window"),
        Notice::commit_list("no base branch"),
    ] {
        state.notice = Some(other.clone());
        state.adopt_agent_list(Ok(vec![]));
        assert_eq!(state.notice, Some(other));
    }
}

#[test]
fn a_working_git_query_leaves_agent_notices_alone() {
    for other in [
        Notice::agent_list("locked"),
        Notice::agent_jump("gone"),
        Notice::commit_list("no base branch"),
    ] {
        let mut state = browse_state();
        state.notice = Some(other.clone());
        state.clear_git_notice();
        assert_eq!(state.notice, Some(other));
    }
}

// ---- AgentTreeAgentJumpFailureIsVisible ------------------------------

#[test]
fn a_failed_jump_leaves_a_notice() {
    use crate::process::MockProcessRunner;
    let runner = MockProcessRunner::new(vec![]).with_windows(&["dispatch"]);
    let mut state = browse_state();
    jump_to_agent(&test_tmux_window("task-7"), &mut state, &runner);
    let Some(Notice::AgentJump(text)) = &state.notice else {
        panic!("expected an agent-jump notice, got {:?}", state.notice);
    };
    assert!(text.contains("task-7"), "{text}");
}

#[test]
fn a_successful_jump_selects_the_window_and_leaves_no_notice() {
    use crate::process::MockProcessRunner;
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&["task-7"]);
    let mut state = browse_state();
    jump_to_agent(&test_tmux_window("task-7"), &mut state, &runner);
    assert_eq!(state.notice, None);
    assert!(runner
        .recorded_calls()
        .iter()
        .any(|(_, args)| args.first().map(String::as_str) == Some("select-window")));
}

// ---- TwoViewsShareThePane ---------------------------------------------

fn render_pane_to_string(state: &mut RenderState, tree: &TreeNode, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(40, height)).expect("terminal");
    terminal
        .draw(|frame| render_pane(frame, frame.area(), tree, state, "wt"))
        .expect("draw");
    buffer_to_string(terminal.backend().buffer())
}

#[test]
fn the_browse_view_shows_the_tree_and_commits_and_no_agents() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = browse_state();
    state.sync_expansion(&tree);
    state.adopt_agent_list(Ok(vec![agent(4941, false)]));
    let out = render_pane_to_string(&mut state, &tree, 12);
    assert!(out.contains("a.rs") && out.contains("Commits"), "{out}");
    assert!(!out.contains("Agents") && !out.contains("#4941"), "{out}");
}

#[test]
fn the_agents_view_fills_the_pane_with_agents_and_hides_the_tree() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = browse_state();
    state.sync_expansion(&tree);
    state.adopt_agent_list(Ok(vec![agent(4941, false), agent(4942, true)]));
    state.focus = Focus::Agents;
    let out = render_pane_to_string(&mut state, &tree, 12);
    assert!(out.contains("Agents (s: browse)"), "{out}");
    assert!(
        out.contains("task 4941") && out.contains("task 4942"),
        "{out}"
    );
    assert!(!out.contains("a.rs") && !out.contains("Commits"), "{out}");
}

#[test]
fn the_agents_view_shows_with_no_agents() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = browse_state();
    state.focus = Focus::Agents;
    let out = render_pane_to_string(&mut state, &tree, 12);
    assert!(out.contains("Agents"), "{out}");
}

#[test]
fn the_commits_border_names_the_keys_that_leave_it() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = browse_state();
    let out = render_pane_to_string(&mut state, &tree, 12);
    assert!(out.contains("Commits (Tab, s: agents)"), "{out}");
}

// ---- PanesReadThroughTheBoard: the agents section's read (task #4982)

struct FakeBoard(std::result::Result<crate::hooks::wire::PaneView, String>);

#[async_trait::async_trait]
impl crate::agent_tree::pane::PaneViewSource for FakeBoard {
    async fn pane_view(&self, _task_id: TaskId) -> anyhow::Result<crate::hooks::wire::PaneView> {
        self.0.clone().map_err(|e| anyhow::anyhow!(e))
    }
    async fn resume_agent(
        &self,
        _task_id: TaskId,
    ) -> anyhow::Result<crate::hooks::wire::PaneResumeAnswer> {
        anyhow::bail!("not asked in these tests")
    }
    fn board_address(&self) -> String {
        "127.0.0.1:8899".into()
    }
}

/// A detached agent as the board sends it: no window.
fn detached_pane_agent(id: i64) -> crate::hooks::wire::PaneAgent {
    crate::hooks::wire::PaneAgent {
        tmux_window: None,
        ..pane_agent(id)
    }
}

/// RefreshAgentTreeAgentList: the board's detached agents are listed beside
/// the attached ones, in the same order, without a window.
#[tokio::test]
async fn detached_agents_are_listed_beside_attached_ones_without_a_window() {
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        live_agents: vec![pane_agent(9), pane_agent(4)],
        detached_agents: vec![detached_pane_agent(6)],
        hidden_agents: 0,
    }));

    let rows = read_agent_list(&board, TaskId(4))
        .await
        .map(|list| list.rows);

    assert_eq!(rows, Ok(vec![agent(4, true), detached(6), agent(9, false)]));
}

fn pane_agent(id: i64) -> crate::hooks::wire::PaneAgent {
    crate::hooks::wire::PaneAgent {
        id: TaskId(id),
        title: format!("task {id}"),
        tmux_window: Some(format!("task-{id}")),
        agent_name: String::new(),
        epic: None,
    }
}

fn pane_epic(id: i64, breadcrumb: &[&str]) -> crate::hooks::wire::PaneEpic {
    crate::agent_tree::render::agents::test_pane_epic(id, breadcrumb)
}

/// `AgentListGroupsByEpic` / RefreshAgentTreeAgentList's order: groups by
/// epic id, agents by id inside a group, epicless agents last; each row keeps
/// the epic the board sent.
#[tokio::test]
async fn the_agent_list_groups_agents_by_epic_with_epicless_last() {
    let five = pane_epic(5, &["Five"]);
    let three = pane_epic(3, &["Root", "Three"]);
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        detached_agents: vec![],
        hidden_agents: 0,
        live_agents: vec![
            crate::hooks::wire::PaneAgent {
                epic: Some(five.clone()),
                ..pane_agent(9)
            },
            pane_agent(4),
            crate::hooks::wire::PaneAgent {
                epic: Some(three.clone()),
                ..pane_agent(7)
            },
            crate::hooks::wire::PaneAgent {
                epic: Some(five.clone()),
                ..pane_agent(2)
            },
        ],
    }));

    let rows = read_agent_list(&board, TaskId(1))
        .await
        .map(|list| list.rows)
        .unwrap();

    let order: Vec<(i64, Option<crate::models::EpicId>)> = rows
        .iter()
        .map(|row| (row.id.0, row.epic.as_ref().map(|e| e.id)))
        .collect();
    let (e3, e5) = (Some(three.id), Some(five.id));
    assert_eq!(order, vec![(7, e3), (2, e5), (9, e5), (4, None)]);
    assert_eq!(rows[0].epic, Some(three));
}

/// `AgentRowShowsAgentName`: the agent name the board sends reaches the row.
#[tokio::test]
async fn the_boards_agent_name_reaches_the_agent_row() {
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        detached_agents: vec![],
        hidden_agents: 0,
        live_agents: vec![crate::hooks::wire::PaneAgent {
            agent_name: "scar".into(),
            ..pane_agent(9)
        }],
    }));

    let rows = read_agent_list(&board, TaskId(7))
        .await
        .map(|list| list.rows)
        .unwrap();

    assert_eq!(rows[0].agent_name, "scar");
}

/// The board's live agents become the section's rows: ordered by id, the
/// pane's own task marked. The board already filtered them
/// (LiveIsTheBoardsOwnDefinition), so nothing it sends is dropped.
#[tokio::test]
async fn the_agent_list_is_the_boards_live_agents_by_id_with_own_marked() {
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        detached_agents: vec![],
        hidden_agents: 0,
        live_agents: vec![pane_agent(9), pane_agent(4), pane_agent(7)],
    }));

    let rows = read_agent_list(&board, TaskId(7))
        .await
        .map(|list| list.rows);

    assert_eq!(
        rows,
        Ok(vec![agent(4, false), agent(7, true), agent(9, false)])
    );
}

/// A board that cannot be reached is a failed read with a reason naming
/// the board, never an empty list: "no agents are running" and "the board
/// did not answer" must not read the same (BoardPaneView).
#[tokio::test]
async fn an_unreachable_board_is_a_failed_agent_read_naming_the_board() {
    let board = FakeBoard(Err("connection refused".into()));

    let read = read_agent_list(&board, TaskId(7))
        .await
        .map(|list| list.rows);

    let reason = read.expect_err("an unreachable board must not read as an empty list");
    assert!(reason.contains("127.0.0.1:8899"), "{reason}");
}

/// The groups come in compact tree order: a child epic's group right after
/// its parent's, and a top-level group with no drawn child before one with
/// a child, whatever the ids (RefreshAgentTreeAgentList's grouped_by_epic,
/// tasks #98901, #99033).
#[tokio::test]
async fn the_agent_list_lays_its_groups_out_as_the_epic_tree() {
    let ten = pane_epic(10, &["Ten"]);
    let twenty = pane_epic(20, &["Twenty"]);
    let mut thirty = pane_epic(30, &["Thirty"]);
    thirty.ancestors = vec![crate::hooks::wire::PaneEpicRef {
        id: crate::models::EpicId(10),
        title: "Ten".into(),
    }];
    let with = |id, epic: &crate::hooks::wire::PaneEpic| crate::hooks::wire::PaneAgent {
        epic: Some(epic.clone()),
        ..pane_agent(id)
    };
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        detached_agents: vec![],
        hidden_agents: 0,
        live_agents: vec![with(1, &twenty), with(2, &thirty), with(3, &ten)],
    }));

    let rows = read_agent_list(&board, TaskId(9))
        .await
        .map(|list| list.rows)
        .unwrap();

    let order: Vec<i64> = rows.iter().map(|row| row.id.0).collect();
    assert_eq!(order, vec![1, 3, 2]);
}

// ---- the board's filter (agent-tree.allium: ListsFollowTheBoardFilter) ----

/// The read carries the board's hidden-agents count beside the rows.
#[tokio::test]
async fn the_agent_read_carries_the_boards_hidden_count() {
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        live_agents: vec![pane_agent(4)],
        detached_agents: vec![],
        hidden_agents: 3,
    }));

    let list = read_agent_list(&board, TaskId(4)).await.unwrap();

    assert_eq!(list.hidden, 3);
    assert_eq!(list.rows.len(), 1);
}

/// A good read sets the count; a failed one keeps it with the last rows
/// (AgentTreeAgentListFailureKeepsLastList).
#[test]
fn a_failed_read_keeps_the_last_hidden_count() {
    use crate::agent_tree::render::agents::AgentList;
    let mut state = RenderState::new();
    state.adopt_agent_read(Ok(AgentList {
        rows: vec![agent(1, false)],
        hidden: 2,
    }));
    assert_eq!(state.agents.hidden(), 2);

    state.adopt_agent_read(Err("board gone".into()));

    assert_eq!(state.agents.hidden(), 2);
    assert_eq!(state.agents.rows().len(), 1);
}

/// ResumeJumpWaitEnds counts list reads, so the poller hands the loop every
/// read, not only one that changed the list: a resumed agent whose launch
/// failed leaves the list as it was, and the wait must still run out.
#[tokio::test]
async fn the_agent_list_poller_sends_an_unchanged_list_again() {
    let board = std::sync::Arc::new(FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        live_agents: vec![pane_agent(4)],
        detached_agents: vec![detached_pane_agent(6)],
        hidden_agents: 0,
    })));
    let (reads, poller) =
        spawn_agent_list_poller(board, TaskId(4), std::time::Duration::from_millis(1));

    let received = tokio::task::spawn_blocking(move || {
        (0..2)
            .map(|_| {
                reads
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .is_ok()
            })
            .collect::<Vec<_>>()
    })
    .await
    .expect("receiver thread");
    poller.abort();

    assert_eq!(received, vec![true, true]);
}
