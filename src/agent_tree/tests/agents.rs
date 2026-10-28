//! The agents section and its read through the board.

use super::*;

// ---- agents section (docs/specs/agent-tree.allium: Agents Section) ----

#[test]
fn the_pane_starts_on_the_agents_view() {
    let state = RenderState::new();
    assert_eq!(state.focus, Focus::Agents);
    assert_eq!(state.browse_focus, Focus::Tree);
}

/// SwitchAgentTreeView: Tab flips from the browse view to the agents view and
/// back, landing on the browse section the user left.
#[test]
fn tab_flips_between_the_browse_view_and_the_agents_view() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(1, false)]);
    assert_eq!(rig.press(KeyCode::Tab), KeyAction::Continue);
    assert_eq!(rig.state.focus, Focus::Agents);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// SwitchAgentTreeView: with two views the two keys are the same flip.
#[test]
fn back_tab_flips_the_view_like_tab() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press_with(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(rig.state.focus, Focus::Agents);
    rig.press_with(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// An empty agents list is a real place to be: Tab never skips it.
#[test]
fn tab_works_with_no_agents_listed() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Agents);
}

/// SwitchAgentTreeBrowseFocus: `s` moves between tree and commits.
#[test]
fn s_moves_between_the_tree_and_the_commits_section() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Commits);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// Tab returns to the browse section that was focused when the user left.
#[test]
fn flipping_back_returns_to_the_commits_section_when_that_is_where_you_were() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Char('s'));
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Agents);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Commits);
}

/// In the agents view there is only one section, so `s` has nowhere to go.
#[test]
fn s_does_nothing_in_the_agents_view() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Tab);
    rig.press(KeyCode::Char('s'));
    assert_eq!(rig.state.focus, Focus::Agents);
}

#[test]
fn tab_clears_a_notice() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.state.notice = Some(Notice::agent_list("db locked"));
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.notice, None);
}

// The agents-section tests below set focus directly: they are about the
// section, not about how Tab reaches it (SwitchAgentTreeView, above).

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
    assert!(out.contains("Agents (Tab: browse)"), "{out}");
    assert!(out.contains("#4941") && out.contains("#4942"), "{out}");
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
    assert!(out.contains("Commits (s, Tab: agents)"), "{out}");
}

// ---- PanesReadThroughTheBoard: the agents section's read (task #4982)

struct FakeBoard(std::result::Result<crate::hooks::wire::PaneView, String>);

#[async_trait::async_trait]
impl crate::agent_tree::pane::PaneViewSource for FakeBoard {
    async fn pane_view(&self, _task_id: TaskId) -> anyhow::Result<crate::hooks::wire::PaneView> {
        self.0.clone().map_err(|e| anyhow::anyhow!(e))
    }
    fn board_address(&self) -> String {
        "127.0.0.1:8899".into()
    }
}

fn pane_agent(id: i64) -> crate::hooks::wire::PaneAgent {
    crate::hooks::wire::PaneAgent {
        id: TaskId(id),
        title: format!("task {id}"),
        tmux_window: format!("task-{id}"),
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

    let rows = read_agent_rows(&board, TaskId(1)).await.unwrap();

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
        live_agents: vec![crate::hooks::wire::PaneAgent {
            agent_name: "scar".into(),
            ..pane_agent(9)
        }],
    }));

    let rows = read_agent_rows(&board, TaskId(7)).await.unwrap();

    assert_eq!(rows[0].agent_name, "scar");
}

/// The board's live agents become the section's rows: ordered by id, the
/// pane's own task marked. The board already filtered them
/// (LiveIsTheBoardsOwnDefinition), so nothing it sends is dropped.
#[tokio::test]
async fn the_agent_list_is_the_boards_live_agents_by_id_with_own_marked() {
    let board = FakeBoard(Ok(crate::hooks::wire::PaneView {
        task: None,
        live_agents: vec![pane_agent(9), pane_agent(4), pane_agent(7)],
    }));

    let rows = read_agent_rows(&board, TaskId(7)).await;

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

    let read = read_agent_rows(&board, TaskId(7)).await;

    let reason = read.expect_err("an unreachable board must not read as an empty list");
    assert!(reason.contains("127.0.0.1:8899"), "{reason}");
}
