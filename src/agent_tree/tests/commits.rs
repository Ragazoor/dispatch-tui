//! The commits section: its list, its keys and the selected source.

use super::*;

// ---- commits section (docs/specs/agent-tree.allium: Commits Section) ----

/// A 40-hex commit id whose first seven characters are unique per `n`, so a
/// short id names exactly one listed commit.
fn commit_id(n: u32) -> String {
    format!("{:07x}{}", 0x0abc_0000 + n, "e".repeat(33))
}

fn commit(n: u32) -> AgentCommit {
    AgentCommit {
        id: commit_id(n),
        subject: format!("commit {n}"),
    }
}

/// Commits in the order given — newest first, as git lists them.
fn commits(ns: &[u32]) -> Vec<AgentCommit> {
    ns.iter().map(|n| commit(*n)).collect()
}

impl KeyRig {
    fn with_commits(changes: &[GitFileChange], listed: Vec<AgentCommit>) -> Self {
        let mut rig = Self::new(changes);
        rig.state.adopt_commit_list(Ok(listed));
        rig.draw();
        rig
    }

    fn cursor_commit(&self) -> Option<String> {
        self.state.commits.cursor_commit().map(|c| c.id.clone())
    }
}

/// The commits section's config: re-read every second, at most six rows on
/// screen, at most fifty commits listed.
#[test]
fn the_commits_sections_config_defaults() {
    use crate::agent_tree::{changes::MAX_LISTED, run::COMMITS_REFRESH_INTERVAL};
    assert_eq!(COMMITS_REFRESH_INTERVAL, std::time::Duration::from_secs(1));
    assert_eq!(COMMITS_MAX_ROWS, 6);
    assert_eq!(MAX_LISTED, 50);
}

/// ShowAgentTreePane / SplitAgentTreePaneOnAgentLaunch: a fresh renderer
/// starts on unstaged work, with nothing listed yet.
#[test]
fn the_pane_starts_on_unstaged_work_with_no_commits_listed() {
    let state = RenderState::new();
    assert_eq!(state.selected_commit, None);
    assert!(state.commits.commits().is_empty());
}

// -- RefreshAgentTreeCommitList / AgentTreeCommitListFailureKeepsLastList --

#[test]
fn a_working_commit_read_replaces_the_list() {
    let mut state = RenderState::new();
    state.adopt_commit_list(Ok(commits(&[2, 1])));
    assert_eq!(state.commits.commits(), commits(&[2, 1]).as_slice());

    state.adopt_commit_list(Ok(commits(&[3, 2, 1])));
    assert_eq!(state.commits.commits(), commits(&[3, 2, 1]).as_slice());
}

/// The selection is keyed by commit id, so a new commit arriving above it
/// leaves it on the same commit.
#[test]
fn a_new_commit_above_the_selection_does_not_move_it() {
    let mut state = RenderState::new();
    state.adopt_commit_list(Ok(commits(&[2, 1])));
    state.selected_commit = Some(commit_id(1));

    state.adopt_commit_list(Ok(commits(&[3, 2, 1])));

    assert_eq!(state.selected_commit, Some(commit_id(1)));
}

/// A selected commit the agent rebased, amended or reset away falls back to
/// unstaged work — silently: the list shows what happened.
#[test]
fn a_selected_commit_that_vanishes_falls_back_to_unstaged_work_silently() {
    let mut state = RenderState::new();
    state.adopt_commit_list(Ok(commits(&[2, 1])));
    state.selected_commit = Some(commit_id(2));

    state.adopt_commit_list(Ok(commits(&[9, 1])));

    assert_eq!(state.selected_commit, None);
    assert_eq!(state.notice, None, "the fallback raises no notice");
}

/// A stale list that says it is stale beats an empty one: a failed read
/// keeps the list AND the selection, and says why.
#[test]
fn a_failed_commit_read_keeps_the_list_and_the_selection_and_says_why() {
    let mut state = RenderState::new();
    state.adopt_commit_list(Ok(commits(&[2, 1])));
    state.selected_commit = Some(commit_id(1));

    state.adopt_commit_list(Err("fatal: Not a valid object name main".to_string()));

    assert_eq!(state.commits.commits(), commits(&[2, 1]).as_slice());
    assert_eq!(state.selected_commit, Some(commit_id(1)));
    assert_eq!(
        state.notice,
        Some(Notice::commit_list("fatal: Not a valid object name main"))
    );
}

/// A working read clears its own stale notice and nothing else.
#[test]
fn a_working_commit_read_clears_only_its_own_notice() {
    let mut state = RenderState::new();
    state.notice = Some(Notice::commit_list("no base branch"));
    state.adopt_commit_list(Ok(vec![]));
    assert_eq!(state.notice, None);

    for other in [
        Notice::git("index.lock"),
        Notice::diff("split failed"),
        Notice::agent_list("db locked"),
        Notice::agent_jump("no window"),
    ] {
        state.notice = Some(other.clone());
        state.adopt_commit_list(Ok(vec![]));
        assert_eq!(state.notice, Some(other));
    }
}

/// Row 0 is "unstaged work"; the cursor starts there.
#[test]
fn the_commits_cursor_starts_on_unstaged_work() {
    let mut section = CommitsSection::new();
    section.set_commits(commits(&[2, 1]));
    assert_eq!(section.cursor_commit(), None);
}

/// The cursor follows its commit by id, not by index.
#[test]
fn the_commits_cursor_follows_its_commit_when_a_new_one_arrives() {
    let mut section = CommitsSection::new();
    section.set_commits(commits(&[2, 1]));
    section.down();
    section.down();
    assert_eq!(section.cursor_commit(), Some(&commit(1)));

    section.set_commits(commits(&[3, 2, 1]));

    assert_eq!(section.cursor_commit(), Some(&commit(1)));
}

/// ...and clamps to the last row when that commit has gone.
#[test]
fn the_commits_cursor_clamps_to_the_last_row_when_its_commit_has_gone() {
    let mut section = CommitsSection::new();
    section.set_commits(commits(&[3, 2, 1]));
    section.bottom();
    assert_eq!(section.cursor_commit(), Some(&commit(1)));

    section.set_commits(commits(&[3, 2]));

    assert_eq!(section.cursor_commit(), Some(&commit(2)));
}

// -- keys with the commits section focused (AgentKeysFollowFocus) --------

#[test]
fn with_the_commits_section_focused_j_and_k_move_its_cursor_not_the_trees() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs"), modified("b.rs")], commits(&[2, 1]));
    let tree_before = rig.selected();
    rig.state.focus = Focus::Commits;

    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.cursor_commit(), Some(commit_id(2)));
    rig.press(KeyCode::Down);
    assert_eq!(rig.cursor_commit(), Some(commit_id(1)));
    rig.press(KeyCode::Char('k'));
    assert_eq!(rig.cursor_commit(), Some(commit_id(2)));
    rig.press(KeyCode::Up);
    assert_eq!(rig.cursor_commit(), None, "back on unstaged work");
    assert_eq!(rig.selected(), tree_before);
}

#[test]
fn with_the_commits_section_focused_gg_capital_g_and_half_pages_jump_its_cursor() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[3, 2, 1]));
    rig.state.focus = Focus::Commits;

    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.cursor_commit(), Some(commit_id(1)));
    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.cursor_commit(), None);
    rig.press_ctrl(KeyCode::Char('d'));
    assert!(
        rig.cursor_commit().is_some(),
        "Ctrl-D moved off the top row"
    );
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.cursor_commit(), None);
}

/// The cursor and the selection are separate: moving selects nothing.
#[test]
fn moving_the_commits_cursor_selects_nothing() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[2, 1]));
    rig.state.focus = Focus::Commits;
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.state.selected_commit, None);
}

/// SelectAgentTreeSource: Space or Enter on a commit row selects that
/// commit, and the loop is told the source changed.
#[test]
fn space_and_enter_on_a_commit_row_select_it_as_the_source() {
    for (code, detail) in [(KeyCode::Char(' '), " "), (KeyCode::Enter, "Enter")] {
        let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[2, 1]));
        rig.state.focus = Focus::Commits;
        rig.press(KeyCode::Char('j'));
        rig.state.usage.clear();

        assert_eq!(rig.press(code), KeyAction::SourceChanged, "{detail}");
        assert_eq!(rig.state.selected_commit, Some(commit_id(2)), "{detail}");
        assert_eq!(
            recorded_usage(&rig.state),
            vec![("select_source".to_string(), Some(detail.to_string()))]
        );
    }
}

/// The "unstaged work" row selects unstaged work.
#[test]
fn space_on_the_unstaged_work_row_selects_unstaged_work_again() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[2, 1]));
    rig.state.focus = Focus::Commits;
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    assert_eq!(rig.state.selected_commit, Some(commit_id(2)));

    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::SourceChanged);
    assert_eq!(rig.state.selected_commit, None);
}

/// Selecting what is already selected is a no-op, not a rebuild, and
/// records no usage.
#[test]
fn selecting_the_row_already_selected_does_nothing_and_records_nothing() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[2, 1]));
    rig.state.focus = Focus::Commits;
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    rig.state.usage.clear();

    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Continue);
    assert_eq!(rig.press(KeyCode::Enter), KeyAction::Continue);
    assert_eq!(rig.state.selected_commit, Some(commit_id(2)));
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
}

/// The open set is left alone (a path open under one source stays open
/// under the next), and focus stays in the commits section so stepping
/// through commits is a loop of motion and Space.
#[test]
fn selecting_a_source_leaves_the_open_set_and_the_focus_alone() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[1]));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    assert!(rig.state.is_diff_open(Path::new("a.rs")));
    assert_eq!(
        rig.state.selected_commit, None,
        "Space selects a source only with the commits section focused"
    );

    rig.state.focus = Focus::Commits;
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));

    assert_eq!(rig.state.selected_commit, Some(commit_id(1)));
    assert!(rig.state.is_diff_open(Path::new("a.rs")));
    assert_eq!(rig.state.focus, Focus::Commits);
}

#[test]
fn h_l_left_and_right_do_nothing_with_the_commits_section_focused() {
    let mut rig = KeyRig::with_commits(&[modified("src/a.rs")], commits(&[2, 1]));
    let opened = rig.state.tree_state.opened().clone();
    rig.press(KeyCode::Char('j'));
    let selected = rig.selected();
    rig.state.focus = Focus::Commits;
    rig.press(KeyCode::Char('j'));
    for code in [
        KeyCode::Char('h'),
        KeyCode::Left,
        KeyCode::Char('l'),
        KeyCode::Right,
    ] {
        assert_eq!(rig.press(code), KeyAction::Continue);
    }
    assert_eq!(rig.state.tree_state.opened(), &opened);
    assert_eq!(rig.selected(), selected);
    assert_eq!(rig.cursor_commit(), Some(commit_id(2)));
}

/// The pane-wide keys act from the commits section too.
#[test]
fn a_and_q_still_act_with_the_commits_section_focused() {
    let mut rig = KeyRig::with_commits(&[modified("a.rs")], commits(&[1]));
    rig.state.focus = Focus::Commits;
    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);
    assert!(rig.state.is_diff_open(Path::new("a.rs")));
    assert_eq!(rig.press(KeyCode::Char('q')), KeyAction::Exit);
}

/// The tree switches now, not on the next tick, and starts from nothing:
/// the old source's tree is never drawn under the new source's name. The
/// selection is published where the diff pane reads it.
#[test]
fn selecting_a_source_empties_the_tree_at_once_and_publishes_the_selection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (worktree, _) = crate::worktree_admin::tests::make_linked_worktree(dir.path(), "task");
    let root = PathBuf::from(&worktree);
    let mut tree = build_tree(&root, &[modified("a.rs")]);
    let mut state = RenderState::new();
    state.selected_commit = Some(commit_id(1));

    adopt_selected_source(&root, &mut tree, &mut state);

    assert_eq!(tree, build_tree(&root, &[]));
    assert_eq!(
        crate::agent_tree::open_set::read_selected_source(&worktree),
        Some(commit_id(1))
    );
}

/// press_every_row_key / RecordedActionMatchesRow for the commits section.
/// The namespace's rows are exactly the pane-wide keys plus select_source:
/// q, Ctrl+C, Tab, a, j, Down, k, Up, gg, G, Ctrl+D, Ctrl+U, Space, Enter.
#[test]
fn pressing_each_key_of_each_commits_row_records_the_rows_action() {
    let mut pressed = 0;
    for binding in bindings_in(KeyNamespace::AgentTreeCommits) {
        assert!(!binding.keys.contains(&ANY_OTHER_KEY));
        for key in binding.keys {
            let mut rig = KeyRig::with_commits(&three_node_changes(), commits(&[2, 1]));
            rig.state.focus = Focus::Commits;
            // On a commit that is not selected, so select_source takes effect.
            rig.press(KeyCode::Char('j'));
            rig.state.usage.clear();
            if *key == "gg" {
                rig.press(KeyCode::Char('g'));
                assert!(rig.state.usage.is_empty(), "first g is pending input");
                rig.press(KeyCode::Char('g'));
            } else {
                let ev = crate::agent_tree::pane::test_key_event(key);
                handle_key(&mut rig.state, &rig.tree, ev);
            }
            pressed += 1;
            let detail = match *key {
                "Space" => " ".to_string(),
                k if k.starts_with("Ctrl+") => k[5..].to_lowercase(),
                k => k.to_string(),
            };
            assert_eq!(
                recorded_usage(&rig.state),
                vec![(binding.action.to_string(), Some(detail))],
                "agent_tree.commits {key}"
            );
        }
    }
    assert_eq!(pressed, 14);
}

// -- CommitsSectionSitsBetweenTreeAndAgents / CommitRowShowsShortIdAndSubject

fn render_pane_buffer(
    state: &mut RenderState,
    tree: &TreeNode,
    height: u16,
) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(40, height)).expect("terminal");
    terminal
        .draw(|frame| render_pane(frame, frame.area(), tree, state, "wt"))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn line_index(out: &str, needle: &str) -> usize {
    out.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?}:\n{out}"))
}

/// The y of the first row containing `needle`.
fn row_of(buffer: &ratatui::buffer::Buffer, needle: &str) -> u16 {
    let out = buffer_to_string(buffer);
    u16::try_from(line_index(&out, needle)).expect("row fits")
}

/// Every cell of row `y`, symbol and style — what the user sees, colour and
/// emphasis included.
fn row_cells(buffer: &ratatui::buffer::Buffer, y: u16) -> Vec<(String, ratatui::style::Style)> {
    let area = *buffer.area();
    (area.left()..area.right())
        .map(|x| (buffer[(x, y)].symbol().to_string(), buffer[(x, y)].style()))
        .collect()
}

#[test]
fn the_commits_section_sits_between_the_tree_and_the_agents_section() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    state.adopt_agent_list(Ok(vec![agent(4941, false)]));

    let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 16));

    let tree_row = line_index(&out, "a.rs");
    let header = line_index(&out, "Commits (Tab)");
    let unstaged = line_index(&out, "unstaged work");
    let agents = line_index(&out, "Agents (Tab)");
    assert!(
        tree_row < header && header < unstaged && unstaged < agents,
        "{out}"
    );
}

/// "unstaged work" is always the top row, so the section is never empty.
#[test]
fn the_commits_section_shows_unstaged_work_with_no_commits() {
    let tree = build_tree(&root(), &[]);
    let mut state = RenderState::new();
    let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 16));
    assert!(out.contains("unstaged work"), "{out}");
}

/// Each commit row reads `<short id> <subject>`, newest first beneath
/// "unstaged work".
#[test]
fn a_commit_row_reads_its_short_id_then_its_subject_newest_first() {
    let tree = build_tree(&root(), &[]);
    let mut state = RenderState::new();
    state.adopt_commit_list(Ok(commits(&[2, 1])));

    let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 20));

    let unstaged = line_index(&out, "unstaged work");
    let newer = line_index(&out, "commit 2");
    let older = line_index(&out, "commit 1");
    assert!(unstaged < newer && newer < older, "{out}");
    let row = out.lines().nth(newer).expect("row");
    let id_at = row.find(&commit_id(2)[..7]).expect(row);
    let subject_at = row.find("commit 2").expect(row);
    assert!(id_at < subject_at, "{row}");
}

/// As tall as its rows — "unstaged work" plus one per commit — capped at
/// config.agent_tree_commits_max_rows.
#[test]
fn the_commits_section_grows_with_its_commits_up_to_its_cap() {
    for (listed, rows) in [(0u32, 1usize), (1, 2), (10, COMMITS_MAX_ROWS)] {
        let tree = build_tree(&root(), &[]);
        let mut state = RenderState::new();
        if listed > 0 {
            state.adopt_commit_list(Ok((1..=listed).rev().map(commit).collect()));
        }
        let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 30));
        let header = line_index(&out, "Commits (Tab)");
        let agents = line_index(&out, "Agents (Tab)");
        assert_eq!(agents - header - 2, rows, "{listed} commits:\n{out}");
    }
}

/// Past the cap the section scrolls to keep its cursor in view.
#[test]
fn the_commits_section_scrolls_to_keep_its_cursor_in_view() {
    let tree = build_tree(&root(), &[]);
    let mut state = RenderState::new();
    state.adopt_commit_list(Ok((1..=10).rev().map(commit).collect()));
    state.focus = Focus::Commits;
    render_pane_buffer(&mut state, &tree, 30);
    handle_key(
        &mut state,
        &tree,
        KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE),
    );

    let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 30));
    assert!(
        out.contains("commit 1"),
        "the oldest commit must scroll into view:\n{out}"
    );
}

/// The SELECTED row carries a marker in every state — here with focus on
/// the tree, where the commits section draws no cursor at all.
#[test]
fn the_selected_source_is_marked_whatever_has_focus() {
    let tree = build_tree(&root(), &[]);
    let draw = |selected: Option<String>| {
        let mut state = RenderState::new();
        state.adopt_commit_list(Ok(commits(&[1])));
        state.selected_commit = selected;
        let buffer = render_pane_buffer(&mut state, &tree, 20);
        (
            row_cells(&buffer, row_of(&buffer, "unstaged work")),
            row_cells(&buffer, row_of(&buffer, "commit 1")),
        )
    };
    let (unstaged_selected, commit_unselected) = draw(None);
    let (unstaged_unselected, commit_selected) = draw(Some(commit_id(1)));

    assert_ne!(unstaged_selected, unstaged_unselected, "unstaged work row");
    assert_ne!(commit_selected, commit_unselected, "commit row");
}

/// TreeTitleNamesItsSource: the root alone for unstaged work, the selected
/// commit's short id beside it for a commit.
#[test]
fn the_tree_title_names_the_selected_commit() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let short = &commit_id(1)[..7];

    let mut state = RenderState::new();
    let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 16));
    let title = out.lines().next().expect("title row");
    assert!(title.contains("wt") && !title.contains(short), "{title}");

    let mut state = RenderState::new();
    state.selected_commit = Some(commit_id(1));
    let out = buffer_to_string(&render_pane_buffer(&mut state, &tree, 16));
    let title = out.lines().next().expect("title row");
    assert!(title.contains("wt") && title.contains(short), "{title}");
}

/// Drawn and focused exactly as the agents section is: the focus colour on
/// its border when focused, red when a notice is up (AgentTreeNoticeRedensBorder
/// — every section's border).
#[test]
fn the_commits_sections_border_shows_focus_and_reddens_with_a_notice() {
    use crate::palette::CYAN;
    let tree = build_tree(&root(), &[]);

    let mut state = RenderState::new();
    state.focus = Focus::Commits;
    let buffer = render_pane_buffer(&mut state, &tree, 20);
    let y = row_of(&buffer, "Commits (Tab)");
    assert_eq!(buffer[(0, y)].style().fg, Some(CYAN));

    let mut state = RenderState::new();
    state.notice = Some(Notice::commit_list("no base branch"));
    let buffer = render_pane_buffer(&mut state, &tree, 20);
    let y = row_of(&buffer, "Commits (Tab)");
    assert_eq!(buffer[(0, y)].style().fg, Some(RED));
}

// -- SelectedCommitIsListed, as a property over key and read sequences ----

mod property_tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        /// A working read listing the pool's commits where the mask is set.
        Read(Vec<bool>),
        FailedRead,
        Down,
        Up,
        Top,
        Bottom,
        Select,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            proptest::collection::vec(any::<bool>(), 5).prop_map(Op::Read),
            Just(Op::FailedRead),
            Just(Op::Down),
            Just(Op::Up),
            Just(Op::Top),
            Just(Op::Bottom),
            Just(Op::Select),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Whatever the user presses and whatever git reads back, the
        /// selection is unstaged work or a commit the section lists.
        #[test]
        fn the_selection_is_always_unstaged_work_or_a_listed_commit(
            ops in proptest::collection::vec(op(), 1..24)
        ) {
            let pool = commits(&[5, 4, 3, 2, 1]);
            let mut rig = KeyRig::new(&[]);
            rig.state.focus = Focus::Commits;
            for op in ops {
                match op {
                    Op::Read(mask) => {
                        let listed = pool
                            .iter()
                            .zip(&mask)
                            .filter(|(_, keep)| **keep)
                            .map(|(c, _)| c.clone())
                            .collect();
                        rig.state.adopt_commit_list(Ok(listed));
                    }
                    Op::FailedRead => rig.state.adopt_commit_list(Err("git failed".into())),
                    Op::Down => {
                        rig.press(KeyCode::Char('j'));
                    }
                    Op::Up => {
                        rig.press(KeyCode::Char('k'));
                    }
                    Op::Top => {
                        rig.press(KeyCode::Char('g'));
                        rig.press(KeyCode::Char('g'));
                    }
                    Op::Bottom => {
                        rig.press(KeyCode::Char('G'));
                    }
                    Op::Select => {
                        rig.press(KeyCode::Char(' '));
                    }
                }
                let listed = rig.state.commits.commits();
                let holds = match &rig.state.selected_commit {
                    None => true,
                    Some(id) => listed.iter().any(|c| &c.id == id),
                };
                prop_assert!(
                    holds,
                    "selected {:?} not in {:?}",
                    rig.state.selected_commit,
                    listed
                );
            }
        }
    }
}
