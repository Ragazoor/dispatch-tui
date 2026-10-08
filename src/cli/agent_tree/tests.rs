use super::*;
use crate::agent_tree::build_tree;
use crate::agent_tree::LineCounts;
use crossterm::event::KeyModifiers;
use ratatui::style::Color;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from("/repo")
}

fn changed(path: &str, change: FileChange) -> GitFileChange {
    GitFileChange {
        path: PathBuf::from(path),
        change,
        counts: None,
    }
}

fn modified(path: &str) -> GitFileChange {
    changed(path, FileChange::Modified)
}

fn deleted(path: &str) -> GitFileChange {
    changed(path, FileChange::Deleted)
}

fn added(path: &str) -> GitFileChange {
    changed(path, FileChange::Added)
}

#[test]
fn empty_tree_produces_no_items() {
    let tree = build_tree(&root(), &[]);
    let items = build_tree_items(&tree, &BTreeSet::new(), &HashSet::new());
    assert!(items.is_empty());
}

#[test]
fn changed_file_becomes_a_leaf_item_named_by_relative_path() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let items = build_tree_items(&tree, &BTreeSet::new(), &HashSet::new());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].identifier(), "a.rs");
    assert!(items[0].children().is_empty());
}

#[test]
fn changed_dir_becomes_a_non_leaf_item() {
    let tree = build_tree(&root(), &[modified("src/a.rs")]);
    let items = build_tree_items(&tree, &BTreeSet::new(), &HashSet::new());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].identifier(), "src");
    assert_eq!(items[0].children().len(), 1);
}

/// Each node is identified by its own name — the widget scopes lookups by
/// ancestor chain, so sibling-uniqueness is all it needs. Keeping the
/// identifier the node's `name` verbatim is what makes a node's widget key
/// and its path segments the same vector (see `sync_expansion_at`), merged
/// directory names included.
#[test]
fn node_identifier_is_its_own_name() {
    let tree = build_tree(&root(), &[modified("a/b.rs"), modified("a/c/d.rs")]);
    let items = build_tree_items(&tree, &BTreeSet::new(), &HashSet::new());
    let a = &items[0];
    assert_eq!(a.identifier(), "a");
    let names: Vec<&String> = a.children().iter().map(|c| c.identifier()).collect();
    assert_eq!(names, vec!["b.rs", "c"]);
}

/// docs/specs/agent-tree.allium's MergedDirectoryChainRows: the chain is
/// one item, named by the whole route, and the widget sees exactly one
/// level where it used to see two.
#[test]
fn a_merged_chain_renders_as_one_item_named_by_the_whole_route() {
    let tree = build_tree(&root(), &[modified("a/b/c.rs")]);
    let items = build_tree_items(&tree, &BTreeSet::new(), &HashSet::new());

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].identifier(), "a/b");
    assert_eq!(items[0].children().len(), 1);
    assert_eq!(items[0].children()[0].identifier(), "c.rs");
}

#[test]
fn two_changed_roots_produce_two_top_level_items_sorted_by_name() {
    let tree = build_tree(&root(), &[modified("z.rs"), modified("a.rs")]);
    let items = build_tree_items(&tree, &BTreeSet::new(), &HashSet::new());
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].identifier(), "a.rs");
    assert_eq!(items[1].identifier(), "z.rs");
}

#[test]
fn sync_expansion_opens_newly_changed_directory() {
    let tree = build_tree(&root(), &[modified("src/a.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    assert!(state.tree_state.opened().contains(&vec!["src".to_string()]));
}

/// `absorb_manual_expansion` reads the widget's open set as the record of
/// what the USER did, which is only sound because rendering never writes to
/// it. Nothing in the widget's API promises that, and a future version that
/// pruned stale identifiers while drawing would turn its own housekeeping
/// into recorded user collapses — silently, since the pane would still look
/// right for a tick. Pin it here.
#[test]
fn rendering_never_changes_the_widget_open_set() {
    let mut rig = KeyRig::new(&three_node_changes());
    let before = rig.state.tree_state.opened().clone();

    rig.draw();
    rig.draw();

    assert_eq!(rig.state.tree_state.opened(), &before);
}

/// A poll whose answer has not changed must not undo the collapse. It
/// cannot even try: `adopt_tree` compares the rebuilt tree against the one
/// on screen and skips the expansion sync entirely. The sibling test below
/// covers the case where the answer HAS changed and the sync really runs.
#[test]
fn a_poll_with_an_unchanged_answer_leaves_a_manual_collapse_alone() {
    let changes = [modified("src/a.rs")];
    let mut rig = KeyRig::new(&changes);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    rig.refresh(&changes);
    assert!(!rig.is_open(&["src"]));
}

#[test]
fn sync_expansion_opens_a_newly_changed_sibling_without_reopening_a_closed_one() {
    let mut rig = KeyRig::new(&[modified("src/a.rs")]);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    // A second poll picks up a brand-new change under a different directory.
    rig.refresh(&[modified("src/a.rs"), added("docs/b.md")]);

    assert!(
        !rig.is_open(&["src"]),
        "manually closed dir must stay closed"
    );
    assert!(rig.is_open(&["docs"]), "newly changed dir must auto-open");
}

/// The collapse is forgotten when the row itself goes. The agent reverts
/// its last edit under `src/`, the row disappears, and a later edit there
/// is news — so the row comes back OPEN rather than carrying forward a
/// collapse the user made about a different state of the worktree.
#[test]
fn a_directory_that_disappears_and_returns_opens_again() {
    let mut rig = KeyRig::new(&[modified("src/a.rs"), modified("top.rs")]);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    rig.refresh(&[modified("top.rs")]);
    rig.refresh(&[modified("src/a.rs"), modified("top.rs")]);

    assert!(
        rig.is_open(&["src"]),
        "a row that went away and came back is news; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

#[test]
/// `a` holds a file of its own, so it survives chain merging and the tree
/// really is two levels deep — which is the thing this test is about. A
/// bare `a/b/c.rs` would be one merged row and prove nothing about nesting.
fn sync_expansion_opens_nested_ancestor_directories() {
    let tree = build_tree(&root(), &[modified("a/x.rs"), modified("a/b/c.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let opened = state.tree_state.opened();
    assert!(opened.contains(&vec!["a".to_string()]));
    assert!(opened.contains(&vec!["a".to_string(), "b".to_string()]));
}

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::cli::buffer_to_string;

fn render_to_string(changes: &[GitFileChange], title: &str, width: u16, height: u16) -> String {
    let tree = build_tree(&root(), changes);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, title))
        .expect("draw");
    buffer_to_string(terminal.backend().buffer())
}

fn counted(path: &str, change: FileChange, added: u32, removed: u32) -> GitFileChange {
    GitFileChange {
        path: PathBuf::from(path),
        change,
        counts: Some(LineCounts { added, removed }),
    }
}

// ---- line counts on the rendered rows ---------------------------------

#[test]
fn a_file_row_shows_its_added_and_removed_line_counts() {
    let out = render_to_string(
        &[counted("a.rs", FileChange::Modified, 12, 3)],
        "task",
        60,
        8,
    );
    insta::assert_snapshot!("file_row_line_counts", out);
}

/// The foreground colour `offset` CELLS right of where `needle` starts, on
/// the first row that holds it. Used to check that a merged row's leading
/// segments are dimmed and its last segment is not.
///
/// Cell-wise rather than byte-wise on purpose: the row begins with the
/// pane border and an expansion arrow, both multi-byte, so a byte offset
/// into the joined line lands several columns past the name.
fn row_fg_at(buffer: &ratatui::buffer::Buffer, needle: &str, offset: u16) -> Option<Color> {
    let area = *buffer.area();
    for y in area.top()..area.bottom() {
        let cells: Vec<&str> = (area.left()..area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        for start in 0..cells.len() {
            // The needle is consumed cell by cell. Joining the row tail
            // instead would reallocate it once per start column.
            let mut rest = needle;
            for cell in &cells[start..] {
                let Some(tail) = rest.strip_prefix(*cell) else {
                    break;
                };
                rest = tail;
                if rest.is_empty() {
                    let x = area.left() + u16::try_from(start).ok()? + offset;
                    return buffer[(x, y)].style().fg;
                }
            }
        }
    }
    None
}

/// docs/specs/agent-tree.allium's CountsShowOnTheFolderNearestTheFiles: a
/// directory that holds another directory leaves the counts to the row
/// further in. Before this the same `+40 -10` appeared on every folder on
/// the way down, which is what made a deep tree unreadable in a narrow
/// pane.
#[test]
fn a_directory_holding_a_subdirectory_shows_no_counts_while_expanded() {
    let out = render_to_string(
        &[
            counted("src/main.rs", FileChange::Modified, 12, 0),
            counted("src/cli/a.rs", FileChange::Modified, 40, 10),
        ],
        "task",
        60,
        10,
    );

    let src_row = out
        .lines()
        .find(|l| l.contains("▼ src") && !l.contains("cli"))
        .unwrap_or_else(|| panic!("no src row in:\n{out}"));
    assert!(
        !src_row.contains('+') && !src_row.contains('-'),
        "src holds a subdirectory and must show no counts; row: {src_row:?}\n{out}"
    );
    assert!(
        out.lines().any(|l| l.contains("cli") && l.contains("+40")),
        "cli is nearest the files and must carry its counts:\n{out}"
    );
}

/// The exception, and the reason the sum is still derived for every
/// directory: a collapsed row summarises rows that are not on screen.
#[test]
fn a_collapsed_directory_shows_its_full_sum_however_deep() {
    let mut rig = KeyRig::new(&[
        counted("src/main.rs", FileChange::Modified, 12, 0),
        counted("src/cli/a.rs", FileChange::Modified, 40, 10),
    ]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
    rig.press(KeyCode::Char('h'));

    let out = rig.rendered();
    insta::assert_snapshot!("collapsed_directory_full_sum", out);
}

/// The merged row is the folder nearest the files, so it carries them.
#[test]
fn a_merged_row_shows_the_counts_of_the_files_it_holds() {
    let out = render_to_string(
        &[counted("a/b/c/d.rs", FileChange::Modified, 7, 2)],
        "task",
        60,
        10,
    );
    let row = out
        .lines()
        .find(|l| l.contains("a/b/c"))
        .unwrap_or_else(|| panic!("no merged row in:\n{out}"));
    assert!(row.contains("+7"), "expected +7 on the merged row: {row:?}");
    assert!(row.contains("-2"), "expected -2 on the merged row: {row:?}");
}

/// The dim half of MergedDirectoryChainRows: the route recedes, the folder
/// the files are actually in does not. Asserted on the styled buffer,
/// because the whole point is a colour difference the plain text cannot
/// carry.
#[test]
fn a_merged_row_dims_every_segment_but_the_last() {
    let tree = build_tree(&root(), &[modified("a/b/c/d.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "task"))
        .expect("draw");
    let buffer = terminal.backend().buffer();

    // "a/b/c" — the leading "a/b/" is the route, "c" is the folder.
    assert_eq!(
        row_fg_at(buffer, "a/b/c", 0),
        Some(MUTED),
        "the route must be dimmed"
    );
    assert_eq!(
        row_fg_at(buffer, "a/b/c", 4),
        Some(FG),
        "the last segment must keep the ordinary directory colour"
    );
}

/// The spec's `MergedRoutesClipAtThePaneEdge`. A route wider than the pane
/// is cut at the right edge — not wrapped onto a second row, which would
/// put one directory on two rows and break the one-row-per-node reading
/// the cursor and the expansion keys depend on.
///
/// Asserted on the rendered rows because that is the only place clipping
/// happens: the label is built at full width either way.
#[test]
fn a_route_wider_than_the_pane_is_clipped_not_wrapped() {
    let out = render_to_string(
        &[counted(
            "aaaaaaaa/bbbbbbbb/cccccccc/dddddddd/leaf.rs",
            FileChange::Modified,
            7,
            2,
        )],
        "task",
        24,
        8,
    );

    let route_rows = out.lines().filter(|l| l.contains("aaaaaaaa")).count();
    assert_eq!(
        route_rows, 1,
        "the route must occupy exactly one row:\n{out}"
    );
    assert!(
        out.lines().all(|l| l.chars().count() <= 24),
        "no row may exceed the pane width:\n{out}"
    );
    // The tail of the route, and the counts behind it, are what is lost.
    assert!(
        !out.contains("dddddddd"),
        "expected the route cut at the edge:\n{out}"
    );
}

/// An unmerged directory has no route to dim, so it is drawn exactly as
/// before — one span, ordinary colour.
#[test]
fn an_unmerged_directory_row_is_not_dimmed() {
    let tree = build_tree(&root(), &[modified("src/a.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "task"))
        .expect("draw");

    assert_eq!(
        row_fg_at(terminal.backend().buffer(), "src", 0),
        Some(FG),
        "an unmerged directory name is not a route"
    );
}

/// A collapsed directory has to say how much is inside it, or the counts
/// are only useful once the user has already opened everything.
#[test]
fn a_directory_row_shows_the_sum_of_its_descendants() {
    let out = render_to_string(
        &[
            counted("src/a.rs", FileChange::Modified, 12, 3),
            counted("src/b.rs", FileChange::Added, 5, 1),
        ],
        "task",
        60,
        10,
    );
    insta::assert_snapshot!("summed_line_counts", out);
}

/// An untracked file has no counts and must show none — not "+0 -0", which
/// would read as "nothing changed in there" about a file the agent just
/// wrote. See the spec's UntrackedFilesHaveNoLineCounts.
#[test]
fn an_untracked_file_row_shows_no_counts_at_all() {
    let out = render_to_string(&[added("brand_new.rs")], "task", 60, 8);
    assert!(out.contains("brand_new.rs"), "expected the row in:\n{out}");
    assert!(!out.contains("+0"), "expected no +0 in:\n{out}");
    assert!(!out.contains("-0"), "expected no -0 in:\n{out}");
}

/// Zero is a real answer for a TRACKED file — a permission-only change
/// moves no lines — and is shown, unlike the absent counts above.
#[test]
fn a_tracked_file_with_no_moved_lines_still_shows_zeroes() {
    let out = render_to_string(
        &[counted("mode.sh", FileChange::Modified, 0, 0)],
        "task",
        60,
        8,
    );
    assert!(out.contains("+0"), "expected +0 in:\n{out}");
}

/// The badge still renders beside the counts. The two answer different
/// questions — what happened, and how much of it — and a row needs both.
#[test]
fn counts_render_alongside_the_badge_not_instead_of_it() {
    let out = render_to_string(
        &[counted("a.rs", FileChange::Modified, 2, 1)],
        "task",
        60,
        8,
    );
    insta::assert_snapshot!("counts_alongside_badge", out);
}

/// A rendered companion pane: `TreeState`'s cursor movement resolves
/// against the identifiers captured by the *last render*, so a key test
/// has to draw at least once before pressing anything.
struct KeyRig {
    tree: TreeNode,
    state: RenderState,
    terminal: Terminal<TestBackend>,
}

impl KeyRig {
    fn new(changes: &[GitFileChange]) -> Self {
        Self::sized(changes, 12)
    }

    /// `height` is the whole pane, borders included, so the visible row
    /// count the half-page motions divide is `height - 2`.
    fn sized(changes: &[GitFileChange], height: u16) -> Self {
        let tree = build_tree(&root(), changes);
        let mut state = RenderState::new();
        state.sync_expansion(&tree);
        let terminal = Terminal::new(TestBackend::new(50, height)).expect("terminal");
        let mut rig = Self {
            tree,
            state,
            terminal,
        };
        rig.draw();
        rig
    }

    fn draw(&mut self) {
        let tree = &self.tree;
        let state = &mut self.state;
        self.terminal
            .draw(|frame| render(frame, frame.area(), tree, state, "dispatch"))
            .expect("draw");
    }

    /// Press a key, then redraw as the real loop does — so a following
    /// press sees the identifiers the new view actually rendered.
    fn press(&mut self, code: KeyCode) -> KeyAction {
        self.press_with(code, KeyModifiers::NONE)
    }

    /// Press a key with Ctrl held.
    fn press_ctrl(&mut self, code: KeyCode) -> KeyAction {
        self.press_with(code, KeyModifiers::CONTROL)
    }

    fn press_with(&mut self, code: KeyCode, modifiers: KeyModifiers) -> KeyAction {
        let action = handle_key(&mut self.state, &self.tree, KeyEvent::new(code, modifiers));
        self.draw();
        action
    }

    /// Rebuild the tree from a new change set and re-sync expansion, as
    /// the real loop's `refresh` does, then redraw. The only way to test
    /// what a refresh does to view state the user has already touched.
    fn refresh(&mut self, changes: &[GitFileChange]) {
        adopt_tree(
            build_tree(&root(), changes),
            &mut self.tree,
            &mut self.state,
        );
        self.draw();
    }

    fn rendered(&self) -> String {
        buffer_to_string(self.terminal.backend().buffer())
    }

    fn selected(&self) -> Vec<String> {
        self.state.tree_state.selected().to_vec()
    }

    /// The selected node's single name segment, for the flat-file trees the
    /// jump-motion tests use.
    fn selected_name(&self) -> String {
        self.selected().join("/")
    }

    fn is_open(&self, path: &[&str]) -> bool {
        let path: Vec<String> = path.iter().map(|s| (*s).to_string()).collect();
        self.state.tree_state.opened().contains(&path)
    }
}

/// Two top-level files plus a directory holding one file. Files sort ahead
/// of directories (`RowsPutAFoldersOwnFilesFirst`), so the flattened view
/// is: a.rs, z.rs, src, src/lib.rs.
fn three_node_changes() -> Vec<GitFileChange> {
    vec![added("a.rs"), modified("src/lib.rs"), modified("z.rs")]
}

#[test]
fn q_exits_the_renderer() {
    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press(KeyCode::Char('q')), KeyAction::Exit);
}

#[test]
fn ctrl_c_exits_the_renderer() {
    let mut state = RenderState::new();
    let tree = build_tree(&root(), &[]);
    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(handle_key(&mut state, &tree, key), KeyAction::Exit);
}

#[test]
fn a_bare_c_does_not_exit_the_renderer() {
    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press(KeyCode::Char('c')), KeyAction::Continue);
}

#[test]
fn down_and_j_both_move_the_cursor_down() {
    for code in [KeyCode::Down, KeyCode::Char('j')] {
        let mut rig = KeyRig::new(&three_node_changes());
        assert_eq!(rig.press(code), KeyAction::Continue);
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");
        rig.press(code);
        assert_eq!(rig.selected(), vec!["z.rs".to_string()], "{code:?}");
    }
}

#[test]
fn up_and_k_both_move_the_cursor_up() {
    for code in [KeyCode::Up, KeyCode::Char('k')] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        assert_eq!(rig.selected(), vec!["z.rs".to_string()], "{code:?}");
        assert_eq!(rig.press(code), KeyAction::Continue);
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");
    }
}

#[test]
fn right_and_l_both_expand_the_selected_directory() {
    for code in [KeyCode::Right, KeyCode::Char('l')] {
        let mut rig = KeyRig::new(&three_node_changes());
        // "src" auto-expanded on first sync; collapse it so expanding is
        // an observable change. Three rows down: a.rs, z.rs, src.
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        assert_eq!(rig.selected(), vec!["src".to_string()], "{code:?}");
        assert!(rig.state.tree_state.close(&["src".to_string()]));
        rig.draw();
        assert!(!rig.is_open(&["src"]), "{code:?}");

        assert_eq!(rig.press(code), KeyAction::Continue);
        assert!(rig.is_open(&["src"]), "{code:?}");
    }
}

#[test]
fn left_and_h_both_collapse_the_selected_directory() {
    for code in [KeyCode::Left, KeyCode::Char('h')] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        rig.press(KeyCode::Down);
        assert_eq!(rig.selected(), vec!["src".to_string()], "{code:?}");
        assert!(rig.is_open(&["src"]), "{code:?}");

        assert_eq!(rig.press(code), KeyAction::Continue);
        assert!(!rig.is_open(&["src"]), "{code:?}");
    }
}

#[test]
fn h_on_a_child_moves_the_cursor_to_its_parent() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    // A node is identified by its own name, so a child's selection path is
    // [parent, child] — see `build_tree_items`.
    assert_eq!(
        rig.selected(),
        vec!["src".to_string(), "lib.rs".to_string()]
    );

    rig.press(KeyCode::Char('h'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
}

#[test]
fn space_and_enter_both_toggle_the_selected_directory() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        rig.press(KeyCode::Char('j'));
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["src".to_string()], "{code:?}");

        assert_eq!(rig.press(code), KeyAction::Continue);
        assert!(!rig.is_open(&["src"]), "{code:?}");
        rig.press(code);
        assert!(rig.is_open(&["src"]), "{code:?}");
    }
}

/// No key expands a file — `tui_tree_widget`'s `open()` has no leaf guard, so
/// an unguarded press on a file inserts that file's path into `opened()`
/// (#3834). Nothing renders differently, which is exactly why this has to be
/// asserted on the open set.
///
/// Space/Enter now *open* a file rather than doing nothing, but that is an
/// action for the loop to perform; the pane's own expansion state must still
/// come out untouched, which is what this covers for all four keys. The
/// returned action differs per key and is asserted by the tests above.
///
/// Asserted over the *whole* set, not just the file's own path, so no
/// sibling directory's expansion is disturbed either. One press per rig:
/// `l` followed by Space on the same file would cancel out (open, then
/// toggle closed) and pass even unguarded.
#[test]
fn no_key_expands_a_file_in_the_open_set() {
    for code in [
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Char('l'),
        KeyCode::Right,
    ] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");
        let before = rig.state.tree_state.opened().clone();

        rig.press(code);

        assert_eq!(rig.state.tree_state.opened(), &before, "{code:?}");
    }
}

/// The user-visible regression: a phantom open on a file is consumed by the
/// next `h`, so step-out silently needed two presses (#3834).
#[test]
fn h_after_space_on_a_file_steps_out_to_the_parent_in_one_press() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(
        rig.selected(),
        vec!["src".to_string(), "lib.rs".to_string()]
    );
    rig.press(KeyCode::Char(' '));
    rig.press(KeyCode::Char('h'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
}

/// Space and Enter on a *file* open its diff. The action tells the loop
/// the set moved; `handle_key` stays pure, so splitting a pane and asking
/// git are the loop's job.
#[test]
fn space_and_enter_on_a_file_open_its_diff() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["a.rs".to_string()], "{code:?}");

        assert_eq!(rig.press(code), KeyAction::DiffSetChanged, "{code:?}");
        assert!(rig.state.is_diff_open(Path::new("a.rs")), "{code:?}");
    }
}

/// The same key both ways — which is what makes it a toggle rather than an
/// open with a separate close to remember.
#[test]
fn space_and_enter_on_an_open_file_close_its_diff() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        rig.press(code);
        assert!(rig.state.is_diff_open(Path::new("a.rs")), "{code:?}");

        assert_eq!(rig.press(code), KeyAction::DiffSetChanged, "{code:?}");
        assert!(!rig.state.is_diff_open(Path::new("a.rs")), "{code:?}");
    }
}

#[test]
fn opening_a_nested_file_records_its_whole_relative_path() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(
        rig.selected(),
        vec!["src".to_string(), "lib.rs".to_string()]
    );

    assert_eq!(rig.press(KeyCode::Enter), KeyAction::DiffSetChanged);
    assert!(rig.state.is_diff_open(Path::new("src/lib.rs")));
}

/// The reversal from the editor this replaced, and the resolution of the
/// spec's old ShowDeletedFileContent question. An editor given a deleted
/// path opens a misleading empty buffer; a DIFF of a deleted file is
/// exactly its former contents, so the deleted case is the one where
/// opening it is most useful.
#[test]
fn space_and_enter_on_a_deleted_file_open_its_diff_too() {
    for code in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut rig = KeyRig::new(&[deleted("gone.rs")]);
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.selected(), vec!["gone.rs".to_string()], "{code:?}");

        assert_eq!(rig.press(code), KeyAction::DiffSetChanged, "{code:?}");
        assert!(rig.state.is_diff_open(Path::new("gone.rs")), "{code:?}");
        assert!(rig.state.notice.is_none(), "{code:?}: no refusal expected");
    }
}

/// Every badge opens. There is no per-badge guard left anywhere in this
/// path — see OpenAgentTreeFileDiff in docs/specs/agent-tree.allium.
#[test]
fn every_badge_opens_a_diff() {
    for change in [FileChange::Added, FileChange::Modified, FileChange::Deleted] {
        let mut rig = KeyRig::new(&[changed("a.rs", change)]);
        rig.press(KeyCode::Char('j'));
        assert_eq!(
            rig.press(KeyCode::Enter),
            KeyAction::DiffSetChanged,
            "{change:?}"
        );
        assert!(rig.state.is_diff_open(Path::new("a.rs")), "{change:?}");
    }
}

/// The directory behaviour is unchanged: Space/Enter still toggles
/// expansion, and must not put a directory in the open set. See
/// OnlyFilesOpenDiffs in docs/specs/agent-tree.allium.
#[test]
fn space_on_a_directory_toggles_expansion_and_opens_no_diff() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);

    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Continue);
    assert!(!rig.is_open(&["src"]));
    assert!(rig.state.open_diffs().is_empty());
}

#[test]
fn space_with_nothing_selected_does_nothing() {
    let mut rig = KeyRig::new(&three_node_changes());
    assert!(rig.selected().is_empty());
    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::Continue);
    assert!(rig.state.open_diffs().is_empty());
}

// ---- the open marker on tree rows -------------------------------------

/// Render the tree with `state` as it stands, rather than fresh — the
/// marker is a function of the open set, which only a pressed key fills.
fn render_rig(rig: &mut KeyRig) -> String {
    rig.draw();
    buffer_to_string(rig.terminal.backend().buffer())
}

#[test]
fn an_open_files_row_carries_the_open_marker() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));

    let out = render_rig(&mut rig);
    assert!(
        out.contains("\u{25cf} a.rs"),
        "expected the marker on a.rs in:\n{out}"
    );
}

#[test]
fn a_closed_files_row_carries_no_marker() {
    let mut rig = KeyRig::new(&three_node_changes());
    let out = render_rig(&mut rig);
    assert!(
        !out.contains("\u{25cf}"),
        "expected no marker anywhere in:\n{out}"
    );
}

/// The marker follows the SET, so closing a file takes it away again —
/// which is what NodeDiffOpenMatchesOpenSet asks of the rendered row.
#[test]
fn closing_a_diff_removes_its_marker() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    rig.press(KeyCode::Char(' '));

    let out = render_rig(&mut rig);
    assert!(!out.contains("\u{25cf}"), "expected no marker in:\n{out}");
}

/// A directory can never be open, so it never carries the marker OR the
/// blank that keeps file names in one column.
#[test]
fn a_directory_row_carries_no_marker_and_no_blank() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('a'));

    let out = render_rig(&mut rig);
    assert!(
        !out.contains("\u{25cf} src"),
        "a directory must not be marked; got:\n{out}"
    );
    assert!(
        out.contains("\u{25cf} lib.rs"),
        "its file child must be; got:\n{out}"
    );
}

// ---- the all-files key ------------------------------------------------

#[test]
fn a_opens_every_changed_files_diff() {
    let mut rig = KeyRig::new(&three_node_changes());

    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);

    assert!(rig.state.is_diff_open(Path::new("a.rs")));
    assert!(rig.state.is_diff_open(Path::new("src/lib.rs")));
}

/// MergedDirectoryChainRows: merging shortens what is drawn, never what is
/// opened. A file under a merged row must reach the open set under the
/// path it would have had with every intermediate row present — the diff
/// pane resolves that path against the worktree, so a shortened one would
/// open nothing.
#[test]
fn opening_a_file_under_a_merged_row_records_its_whole_relative_path() {
    let mut rig = KeyRig::new(&[modified("a/b/c/d.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a/b/c".to_string()]);
    rig.press(KeyCode::Char('j'));

    assert_eq!(rig.press(KeyCode::Char(' ')), KeyAction::DiffSetChanged);
    assert!(
        rig.state.is_diff_open(Path::new("a/b/c/d.rs")),
        "open set: {:?}",
        rig.state.open_diffs()
    );
}

/// The bulk form walks the tree rather than the selection, so it is the
/// other place a merged name could truncate a path.
#[test]
fn a_records_whole_paths_through_merged_rows() {
    let mut rig = KeyRig::new(&[modified("a/b/c/d.rs"), modified("src/x.rs")]);
    rig.press(KeyCode::Char('a'));

    let open: Vec<&Path> = rig.state.open_diffs().iter().map(|p| p.as_path()).collect();
    assert_eq!(
        open,
        vec![Path::new("a/b/c/d.rs"), Path::new("src/x.rs")],
        "merged rows must not shorten the paths beneath them"
    );
}

/// The open marker is looked up by the same relative path, so a merged row
/// above a file must not cost it its marker.
#[test]
fn a_file_under_a_merged_row_still_gets_its_open_marker() {
    let mut open = BTreeSet::new();
    open.insert(PathBuf::from("a/b/c/d.rs"));
    let tree = build_tree(&root(), &[modified("a/b/c/d.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).expect("terminal");
    state.open_diffs = open;
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "task"))
        .expect("draw");
    let rendered = buffer_to_string(terminal.backend().buffer());

    let row = rendered
        .lines()
        .find(|l| l.contains("d.rs"))
        .unwrap_or_else(|| panic!("no file row in:\n{rendered}"));
    assert!(
        row.contains('●'),
        "expected the open marker on {row:?}\n{rendered}"
    );
}

/// What the tree publishes to the diff pane: the open paths in ROW order,
/// which is not their sorted order. `z.rs` is a top-level file so its row
/// is above `src/lib.rs` (`RowsPutAFoldersOwnFilesFirst`), and the diff
/// pane renders what it is handed — it cannot re-derive this.
#[test]
fn the_published_open_set_is_in_row_order_not_path_order() {
    let mut rig = KeyRig::new(&[modified("a.rs"), modified("z.rs"), modified("src/lib.rs")]);
    rig.press(KeyCode::Char('a'));

    assert_eq!(
        rig.state.open_diffs_in_tree_order(&rig.tree),
        vec![
            PathBuf::from("a.rs"),
            PathBuf::from("z.rs"),
            PathBuf::from("src/lib.rs"),
        ]
    );
}

/// A path the tree no longer knows about is appended, not dropped: the open
/// set outlives its files on purpose (`OpenDiffPathsMaySurviveTheirFiles`),
/// and dropping it here would silently un-open a file the user opened.
#[test]
fn a_published_path_the_tree_no_longer_knows_is_kept_at_the_end() {
    let mut rig = KeyRig::new(&[modified("a.rs"), modified("src/lib.rs")]);
    rig.press(KeyCode::Char('a'));
    rig.refresh(&[modified("src/lib.rs")]);

    assert_eq!(
        rig.state.open_diffs_in_tree_order(&rig.tree),
        vec![PathBuf::from("src/lib.rs"), PathBuf::from("a.rs")],
        "the reverted file keeps its place in the set, at the end"
    );
}

/// Directories are routes to files, not things with contents, so `a` must
/// not put one in the set — see OnlyFilesOpenDiffs in the spec.
#[test]
fn a_opens_no_directories() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('a'));
    assert!(!rig.state.is_diff_open(Path::new("src")));
}

/// Direction is decided by the SET, not by a remembered mode: one press
/// always empties a non-empty set, however it came to be non-empty.
#[test]
fn a_closes_everything_when_anything_is_open() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('a'));
    assert!(!rig.state.open_diffs().is_empty());

    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);
    assert!(rig.state.open_diffs().is_empty());
}

/// Even one file opened with Space is enough to make `a` mean "close",
/// because the set is what decides and the set is what the user can see.
#[test]
fn a_closes_a_set_that_space_filled() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char(' '));
    assert_eq!(rig.state.open_diffs().len(), 1);

    rig.press(KeyCode::Char('a'));
    assert!(rig.state.open_diffs().is_empty());
}

/// `a` does not dispatch on the selection at all, so it works with the
/// cursor parked on a directory or on nothing.
#[test]
fn a_acts_on_the_whole_tree_whatever_the_cursor_is_on() {
    let mut rig = KeyRig::new(&three_node_changes());
    assert!(rig.selected().is_empty());
    rig.press(KeyCode::Char('a'));
    assert!(rig.state.is_diff_open(Path::new("src/lib.rs")));

    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["src".to_string()]);
    rig.press(KeyCode::Char('a'));
    assert!(rig.state.is_diff_open(Path::new("a.rs")));
}

/// On an empty tree there is nothing to open, and the press is harmless.
#[test]
fn a_on_an_empty_tree_opens_nothing() {
    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);
    assert!(rig.state.open_diffs().is_empty());
}

/// `l`/`Right` are expansion keys only — they must NOT have picked up the
/// open behaviour along with Space/Enter (#3834's guard stays a guard).
#[test]
fn l_and_right_on_a_file_still_do_nothing() {
    for code in [KeyCode::Char('l'), KeyCode::Right] {
        let mut rig = KeyRig::new(&three_node_changes());
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.press(code), KeyAction::Continue, "{code:?}");
    }
}

#[test]
fn any_key_clears_a_pending_notice() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.state.notice = Some(Notice::diff("could not split the diff pane"));

    rig.press(KeyCode::Char('j'));

    assert!(rig.state.notice.is_none());
}

// ---- Jump motions: gg, G, Ctrl-D, Ctrl-U ------------------------------
//
// See the AgentTreeCompanionPane surface and the
// AgentTreeGgChordNeverExpires guarantee in docs/specs/agent-tree.allium.

/// `count` top-level files, `f01.rs`..`fNN.rs`. Flat and zero-padded, so
/// the flattened view is exactly the files in that order and a landing row
/// is nameable without counting directories.
fn flat_file_changes(count: usize) -> Vec<GitFileChange> {
    (1..=count)
        .map(|n| modified(&format!("f{n:02}.rs")))
        .collect()
}

#[test]
fn gg_jumps_to_the_first_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    for _ in 0..4 {
        rig.press(KeyCode::Char('j'));
    }
    assert_ne!(
        rig.selected_name(),
        "f01.rs",
        "precondition: moved off row 0"
    );

    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.press(KeyCode::Char('g')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f01.rs");
}

#[test]
fn a_lone_g_moves_nothing() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    let before = rig.selected();

    assert_eq!(rig.press(KeyCode::Char('g')), KeyAction::Continue);
    assert_eq!(rig.selected(), before, "a lone g must be swallowed");
}

/// The chord has no clock, so the only thing that can end it is another
/// key — and that key must still do its own job.
#[test]
fn a_key_between_the_two_gs_disarms_the_chord_and_still_acts() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));

    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected_name(), "f03.rs", "the j must still move down");

    rig.press(KeyCode::Char('g'));
    assert_eq!(
        rig.selected_name(),
        "f03.rs",
        "the disarmed chord must not complete on the next lone g"
    );
}

#[test]
fn g_then_a_second_g_after_many_other_keys_needs_a_fresh_pair() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));

    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.selected_name(), "f01.rs");
}

#[test]
fn shift_g_jumps_to_the_last_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    assert_eq!(rig.press(KeyCode::Char('G')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f06.rs");
}

/// A jump lands on a *visible* row. Collapsing `src` hides `lib.rs`, so
/// the last row becomes `src` itself — and the jump must not reopen it.
#[test]
fn shift_g_skips_rows_a_collapsed_directory_hides() {
    let mut rig = KeyRig::new(&[modified("a.rs"), modified("src/lib.rs")]);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.selected_name(), "src");
    assert!(!rig.is_open(&["src"]), "a jump must not expand anything");
}

#[test]
fn gg_lands_on_the_first_row_without_expanding_it() {
    // Both siblings are directories, so the first row is one: a top-level
    // FILE would sort ahead of `src` (`RowsPutAFoldersOwnFilesFirst`) and
    // there would be nothing collapsible to land on.
    let mut rig = KeyRig::new(&[modified("src/lib.rs"), modified("z/z.rs")]);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["src"]), "precondition: src is collapsed");

    rig.press(KeyCode::Char('G'));
    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.selected_name(), "src");
    assert!(!rig.is_open(&["src"]), "a jump must not expand anything");
}

/// A 12-row pane has 10 visible rows, so half a page is 5. The leading `j`
/// establishes a selection: with nothing selected a half-page motion just
/// selects the first row, exactly as `j`/`k` do.
#[test]
fn ctrl_d_moves_the_cursor_half_a_page_down() {
    let mut rig = KeyRig::new(&flat_file_changes(20));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.press_ctrl(KeyCode::Char('d')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f06.rs");
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.selected_name(), "f11.rs");
}

#[test]
fn ctrl_u_moves_the_cursor_half_a_page_up() {
    let mut rig = KeyRig::new(&flat_file_changes(20));
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.selected_name(), "f20.rs");

    assert_eq!(rig.press_ctrl(KeyCode::Char('u')), KeyAction::Continue);
    assert_eq!(rig.selected_name(), "f15.rs");
}

#[test]
fn ctrl_d_clamps_at_the_last_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('d'));
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.selected_name(), "f06.rs");
}

#[test]
fn ctrl_u_clamps_at_the_first_visible_node() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.selected_name(), "f01.rs");
}

/// The distance is half the pane's *current* height, not a constant: a
/// 20-row pane (18 visible) jumps 9, where the default 12-row one jumps 5.
#[test]
fn half_a_page_scales_with_the_pane_height() {
    let mut tall = KeyRig::sized(&flat_file_changes(20), 20);
    tall.press(KeyCode::Char('j'));
    tall.press_ctrl(KeyCode::Char('d'));
    assert_eq!(tall.selected_name(), "f10.rs");

    let mut short = KeyRig::sized(&flat_file_changes(20), 8);
    short.press(KeyCode::Char('j'));
    short.press_ctrl(KeyCode::Char('d'));
    assert_eq!(short.selected_name(), "f04.rs");
}

/// A pane with a single visible row halves to zero. Zero is not a motion,
/// so the floor is one row.
#[test]
fn a_pane_too_short_to_halve_still_moves_one_row() {
    let mut rig = KeyRig::sized(&flat_file_changes(6), 3);
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.selected_name(), "f02.rs");
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.selected_name(), "f01.rs");
}

#[test]
fn jump_motions_are_no_ops_on_an_empty_tree() {
    for keys in [vec!['g', 'g'], vec!['G']] {
        let mut rig = KeyRig::new(&[]);
        for key in keys {
            assert_eq!(rig.press(KeyCode::Char(key)), KeyAction::Continue);
        }
        assert!(rig.selected().is_empty());
    }

    let mut rig = KeyRig::new(&[]);
    assert_eq!(rig.press_ctrl(KeyCode::Char('d')), KeyAction::Continue);
    assert_eq!(rig.press_ctrl(KeyCode::Char('u')), KeyAction::Continue);
    assert!(rig.selected().is_empty());
}

/// `q` still exits with a chord armed — disarming must not swallow the key
/// that did the disarming.
#[test]
fn q_after_a_lone_g_still_exits() {
    let mut rig = KeyRig::new(&flat_file_changes(6));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.press(KeyCode::Char('q')), KeyAction::Exit);
}

// ---- git_changes: unstaged work (AgentTreeShowsUnstagedWorkOnly) ------

use crate::process::MockProcessRunner;

/// Fork point of HEAD with the LOCAL base branch, in the commit-list rigs.
const LOCAL_FORK: &str = "1111111111111111111111111111111111111111";
/// Fork point of HEAD with the REMOTE-TRACKING base ref.
const REMOTE_FORK: &str = "2222222222222222222222222222222222222222";
/// A commit the user has selected in the commits section.
const COMMIT: &str = "3333333333333333333333333333333333333333";

/// A `-z` stream: NUL after every field, exactly as git emits it.
fn nul(fields: &[&str]) -> String {
    fields.iter().map(|f| format!("{f}\0")).collect()
}

/// One `git merge-base` answer, newline-terminated as git writes it.
fn sha(commit: &str) -> Result<std::process::Output> {
    MockProcessRunner::ok_with_stdout(format!("{commit}\n").as_bytes())
}

/// The unstaged form's three answers, in call order: the diff against the
/// index, its line counts, and the untracked listing. `diff` alternates
/// status and path; `numstat` holds whole `added\tremoved\tpath` records;
/// `untracked` is bare paths.
fn unstaged_out_counted(
    diff: &[&str],
    numstat: &[&str],
    untracked: &[&str],
) -> Vec<Result<std::process::Output>> {
    vec![
        MockProcessRunner::ok_with_stdout(nul(diff).as_bytes()),
        MockProcessRunner::ok_with_stdout(nul(numstat).as_bytes()),
        MockProcessRunner::ok_with_stdout(nul(untracked).as_bytes()),
    ]
}

/// Every unstaged-work command succeeds, with no line counts queued. There
/// is nothing to resolve first — the baseline is the index — so a rig needs
/// no merge-base answers at all.
fn git_rig(diff: &[&str], untracked: &[&str]) -> MockProcessRunner {
    MockProcessRunner::new(unstaged_out_counted(diff, &[], untracked))
}

/// The first diff fails, so the query fails. Nothing is queued past it, so
/// a second call would panic — which is what
/// `a_failing_diff_does_not_run_the_untracked_listing` relies on.
fn failing_git_rig(stderr: &str) -> MockProcessRunner {
    MockProcessRunner::new(vec![MockProcessRunner::fail(stderr)])
}

/// The one-commit form's two answers: name-and-status, then counts.
fn commit_rig(diff: &[&str], numstat: &[&str]) -> MockProcessRunner {
    MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(nul(diff).as_bytes()),
        MockProcessRunner::ok_with_stdout(nul(numstat).as_bytes()),
    ])
}

/// The spec's AgentTreeGitQuery, unstaged form, in full: the working tree
/// against the INDEX — naming no revision — then its counts, then the
/// untracked listing. No branch is resolved, so no merge-base probe runs: a
/// stale or missing base branch cannot fail the tree any more.
#[test]
fn unstaged_work_diffs_the_working_tree_against_the_index_and_resolves_no_branch() {
    let runner = git_rig(&["M", "src/a.rs"], &[]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");

    assert_eq!(changes, vec![modified("src/a.rs")]);
    assert_eq!(
        runner.flattened_calls(),
        vec![
            "git -C /wt diff --name-status --no-renames -z".to_string(),
            "git -C /wt diff --numstat --no-renames -z".to_string(),
            "git -C /wt ls-files --others --exclude-standard -z".to_string(),
        ]
    );
}

/// The counts query is a SEPARATE ask against the SAME baseline (the index)
/// and the same rename setting. If the two ever drifted apart, a row's badge
/// and its numbers would be answering different questions.
#[test]
fn git_changes_counts_lines_against_the_same_baseline_as_the_badges() {
    let runner = MockProcessRunner::new(unstaged_out_counted(
        &["M", "src/a.rs"],
        &["12\t3\tsrc/a.rs"],
        &[],
    ));

    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");

    assert_eq!(
        changes[0].counts,
        Some(LineCounts {
            added: 12,
            removed: 3
        })
    );
}

/// An untracked file is invisible to a diff against the index, so it comes
/// back with no counts however the query went. The pane must not fill that
/// hole with a zero — see the spec's UntrackedFilesHaveNoLineCounts.
#[test]
fn an_untracked_path_comes_back_with_no_line_counts() {
    let runner = MockProcessRunner::new(unstaged_out_counted(&[], &[], &["brand_new.rs"]));

    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");

    assert_eq!(changes, vec![added("brand_new.rs")]);
    assert_eq!(changes[0].counts, None);
}

/// Git C-quotes any path with a non-ASCII byte, and separates the status
/// from the path with a tab, unless `-z` is passed — so `src/é.rs` would
/// arrive as the literal `"src/\303\251.rs"`. Every step of both forms asks
/// for NUL-delimited output.
#[test]
fn every_unstaged_query_asks_for_nul_delimited_output() {
    let runner = git_rig(&[], &[]);
    git_changes(Path::new("/wt"), None, &runner).expect("ok");
    for call in runner.flattened_calls() {
        assert!(
            call.split(' ').any(|arg| arg == "-z"),
            "without -z, quoting breaks non-ASCII names; got {call}"
        );
    }
}

/// The payoff of the flag above: a non-ASCII path survives end to end, from
/// git's stdout to a node the tree can name.
#[test]
fn a_non_ascii_path_survives_parsing_and_tree_building() {
    let runner = git_rig(&["M", "src/é.rs"], &["docs/naïve.md"]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(changes, vec![modified("src/é.rs"), added("docs/naïve.md")]);

    let tree = build_tree(&root(), &changes);
    assert_eq!(
        tree.node_at(&["src", "é.rs"]).expect("é.rs").badge,
        Some(FileChange::Modified)
    );
    assert_eq!(
        tree.node_at(&["docs", "naïve.md"]).expect("naïve.md").badge,
        Some(FileChange::Added)
    );
}

/// Every query is bounded, so a git blocked on an index lock the agent
/// itself holds cannot wedge the renderer's single-threaded loop. A tree
/// tick runs at most three commands (config.agent_tree_git_timeout).
#[test]
fn every_unstaged_query_is_bounded_by_a_timeout() {
    let runner = git_rig(&[], &[]);
    git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT); 3]);
}

#[test]
fn git_changes_reports_untracked_files_as_added() {
    let runner = git_rig(&["M", "a.rs"], &["new.rs", "docs/draft.md"]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(
        changes,
        vec![modified("a.rs"), added("new.rs"), added("docs/draft.md")]
    );
}

#[test]
fn git_changes_reports_deletions() {
    let runner = git_rig(&["D", "src/old.rs"], &[]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(changes, vec![deleted("src/old.rs")]);
}

#[test]
fn git_changes_on_a_clean_worktree_reports_nothing() {
    let runner = git_rig(&[], &[]);
    assert!(git_changes(Path::new("/wt"), None, &runner)
        .expect("ok")
        .is_empty());
}

/// A failing git surfaces its own first stderr line, because that line is
/// what reaches the user's border and has to say something actionable.
#[test]
fn git_changes_fails_with_gits_own_message() {
    let runner = failing_git_rig("fatal: unable to read index.lock\n");
    let err = git_changes(Path::new("/wt"), None, &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("index.lock"), "got {err}");
}

/// A failing diff short-circuits: the counts and the listing must not run
/// against a repo that just refused to diff.
#[test]
fn a_failing_diff_does_not_run_the_untracked_listing() {
    let runner = failing_git_rig("fatal: unable to read index.lock\n");
    let _ = git_changes(Path::new("/wt"), None, &runner);
    assert_eq!(runner.recorded_calls().len(), 1);
}

// ---- git_changes: one selected commit (AgentTreeSourceIsOneSelection) --

/// ONE COMMIT: two steps, both the commit against its parent with rename
/// detection off and NUL-delimited output, and no untracked listing — a
/// commit has no untracked files. No branch is resolved for this form
/// either.
#[test]
fn a_selected_commit_is_diffed_in_two_nul_delimited_rename_free_queries() {
    let runner = commit_rig(&["M", "a.rs"], &["3\t1\ta.rs"]);
    git_changes(Path::new("/wt"), Some(COMMIT), &runner).expect("ok");

    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 2, "two steps, no untracked listing: {calls:?}");
    for call in &calls {
        let args: Vec<&str> = call.split(' ').collect();
        assert!(args.contains(&"-z"), "{call}");
        assert!(args.contains(&"--no-renames"), "{call}");
        assert!(call.contains(COMMIT), "the query names the commit: {call}");
        assert!(!call.contains("ls-files"), "{call}");
        assert!(!call.contains("merge-base"), "{call}");
    }
    assert!(calls[0].contains("--name-status"), "{calls:?}");
    assert!(calls[1].contains("--numstat"), "{calls:?}");
}

/// In the commit form every text file carries counts — there is no
/// untracked path for which a count could be missing.
#[test]
fn a_selected_commits_files_all_carry_counts() {
    let runner = commit_rig(
        &["M", "a.rs", "A", "new.rs"],
        &["3\t1\ta.rs", "5\t0\tnew.rs"],
    );
    let changes = git_changes(Path::new("/wt"), Some(COMMIT), &runner).expect("ok");
    assert_eq!(
        changes,
        vec![
            counted("a.rs", FileChange::Modified, 3, 1),
            counted("new.rs", FileChange::Added, 5, 0),
        ]
    );
}

#[test]
fn every_commit_query_is_bounded_by_a_timeout() {
    let runner = commit_rig(&[], &[]);
    git_changes(Path::new("/wt"), Some(COMMIT), &runner).expect("ok");
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT); 2]);
}

/// A commit git cannot read fails the query with git's own words
/// (AgentTreeGitFailureKeepsLastGoodTree: "with a commit selected, git
/// could not read that commit").
#[test]
fn a_commit_git_cannot_read_fails_the_query() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: bad object 3333333333333333333333333333333333333333\n",
    )]);
    let err = git_changes(Path::new("/wt"), Some(COMMIT), &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("bad object"), "got {err}");
}

// ---- git_changes against a real repository ------------------------------

use super::test_repo::TestRepo;
use crate::process::RealProcessRunner;

fn real_changes(repo: &TestRepo, commit: Option<&str>) -> Vec<GitFileChange> {
    git_changes(repo.root(), commit, &RealProcessRunner::default()).expect("git_changes")
}

/// Staged changes are not shown: a file staged and not changed again is
/// absent (AgentTreeShowsUnstagedWorkOnly).
#[test]
fn a_staged_change_is_not_shown() {
    let repo = TestRepo::new();
    repo.write("seed.txt", "seed\nstaged\n");
    repo.git(&["add", "seed.txt"]);

    assert_eq!(real_changes(&repo, None), vec![]);
}

/// Against the index, a row counts only the UNSTAGED part of a file.
#[test]
fn only_the_unstaged_part_of_a_file_is_counted() {
    let repo = TestRepo::new();
    repo.write("seed.txt", "seed\nstaged\n");
    repo.git(&["add", "seed.txt"]);
    repo.append("seed.txt", "unstaged 1\nunstaged 2\n");

    assert_eq!(
        real_changes(&repo, None),
        vec![counted("seed.txt", FileChange::Modified, 2, 0)]
    );
}

/// Committing empties the default view of everything that went into the
/// commit — the reversal of the old "Committing does not empty the pane".
#[test]
fn committed_work_leaves_the_default_view() {
    let repo = TestRepo::new();
    repo.write("src/a.rs", "fn a() {}\n");
    repo.commit_all("add a");

    assert_eq!(real_changes(&repo, None), vec![]);
}

#[test]
fn an_untracked_file_is_shown_as_added_with_no_counts() {
    let repo = TestRepo::new();
    repo.write("new.rs", "one\ntwo\n");

    assert_eq!(real_changes(&repo, None), vec![added("new.rs")]);
}

/// `git rm --cached` takes a file out of the index and leaves it on disk:
/// to git it is now untracked, so it is badged added with no counts — and
/// named once, since the two listings are disjoint against the index
/// (ChangePrecedence).
#[test]
fn a_file_taken_out_of_the_index_is_untracked_not_deleted() {
    let repo = TestRepo::new();
    repo.git(&["rm", "-q", "--cached", "seed.txt"]);

    assert_eq!(real_changes(&repo, None), vec![added("seed.txt")]);
}

/// A new file staged and then edited again is badged modified, not added:
/// the index already holds it, and what is unstaged is an edit to that.
#[test]
fn a_new_file_staged_then_edited_again_is_modified() {
    let repo = TestRepo::new();
    repo.write("n.rs", "one\n");
    repo.git(&["add", "n.rs"]);
    repo.append("n.rs", "two\n");

    assert_eq!(
        real_changes(&repo, None),
        vec![counted("n.rs", FileChange::Modified, 1, 0)]
    );
}

/// A tracked file removed from disk is badged deleted until the deletion is
/// staged, and then disappears — a staged deletion is staged work.
#[test]
fn a_deletion_is_shown_until_it_is_staged() {
    let repo = TestRepo::new();
    std::fs::remove_file(repo.root().join("seed.txt")).expect("rm");
    assert_eq!(
        real_changes(&repo, None),
        vec![counted("seed.txt", FileChange::Deleted, 0, 1)]
    );

    repo.git(&["rm", "-q", "seed.txt"]);
    assert_eq!(real_changes(&repo, None), vec![]);
}

/// A selected commit shows exactly that commit against its parent, with
/// counts, whatever the working tree and the index hold meanwhile.
#[test]
fn a_selected_commit_shows_exactly_that_commit_against_its_parent() {
    let repo = TestRepo::new();
    repo.write("a.rs", "a1\n");
    repo.commit_all("first");
    repo.write("a.rs", "a1\na2\n");
    repo.write("b.rs", "b\n");
    let second = repo.commit_all("second");
    // Work in progress the commit view must not see.
    repo.write("a.rs", "changed again\n");
    repo.write("untracked.rs", "x\n");

    let status_before = repo.git(&["status", "--porcelain"]);

    assert_eq!(
        real_changes(&repo, Some(&second)),
        vec![
            counted("a.rs", FileChange::Modified, 1, 0),
            counted("b.rs", FileChange::Added, 1, 0),
        ]
    );
    // ReadOnlyObservation: showing a commit checks nothing out.
    assert_eq!(repo.head(), second);
    assert_eq!(repo.git(&["status", "--porcelain"]), status_before);
}

/// A commit with no parent is diffed against the empty tree, so every file
/// it holds is badged added.
#[test]
fn a_root_commit_is_diffed_against_the_empty_tree() {
    let repo = TestRepo::new();
    let root_commit = repo.git(&["rev-list", "--max-parents=0", "HEAD"]);

    assert_eq!(
        real_changes(&repo, Some(root_commit.trim())),
        vec![counted("seed.txt", FileChange::Added, 1, 0)]
    );
}

/// A merge commit is shown against its FIRST parent: what it brought onto
/// this branch.
#[test]
fn a_merge_commit_is_shown_against_its_first_parent() {
    let repo = TestRepo::new();
    repo.commit_on_main("from_main.txt", "upstream work");
    repo.write("t.rs", "task\n");
    repo.commit_all("task work");
    repo.git(&["merge", "-q", "--no-ff", "main", "-m", "merge main"]);
    let merge = repo.head();

    assert_eq!(
        real_changes(&repo, Some(&merge)),
        vec![counted("from_main.txt", FileChange::Added, 1, 0)]
    );
}

// ---- git_branch_commits: the commits section's read --------------------

/// The fork-point resolution is KEPT, for this list alone: both refs the
/// base branch can denote are probed, and the listing is taken from the
/// fork point they agree on (AgentTreeBaselineIsTaskBaseBranch).
#[test]
fn the_commit_list_resolves_the_fork_point_from_both_base_refs() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(LOCAL_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    let commits = git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");

    assert!(commits.is_empty());
    let calls = runner.flattened_calls();
    assert_eq!(
        &calls[..2],
        [
            "git -C /wt merge-base HEAD main".to_string(),
            "git -C /wt merge-base HEAD origin/main".to_string(),
        ]
    );
    assert_eq!(calls.len(), 3, "one listing after the probes: {calls:?}");
    assert!(calls[2].contains(LOCAL_FORK), "{calls:?}");
}

/// Both probes are built from the task's own base branch name.
#[test]
fn the_commit_list_uses_the_tasks_own_base_branch() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(LOCAL_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "develop", &runner).expect("ok");
    assert_eq!(
        &runner.flattened_calls()[..2],
        [
            "git -C /wt merge-base HEAD develop".to_string(),
            "git -C /wt merge-base HEAD origin/develop".to_string(),
        ]
    );
}

/// Neither ref resolving fails the read, with the LOCAL branch's message —
/// the name the user put on the task — and nothing further runs.
#[test]
fn neither_base_ref_resolving_fails_the_commit_list_with_the_local_branchs_message() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: Not a valid object name nosuchbranch\n"),
        MockProcessRunner::fail("fatal: Not a valid object name origin/nosuchbranch\n"),
    ]);
    let err = git_branch_commits(Path::new("/wt"), "nosuchbranch", &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("nosuchbranch"), "got {err}");
    assert!(!err.contains("origin/"), "got {err}");
    assert_eq!(runner.recorded_calls().len(), 2);
}

/// A ranking probe that could not answer fails the read rather than
/// silently keeping the local fork point.
#[test]
fn an_ancestry_probe_that_cannot_answer_fails_the_commit_list() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::fail_with_code(128, "fatal: unable to read pack\n"),
    ]);
    let err = git_branch_commits(Path::new("/wt"), "main", &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("unable to read pack"), "got {err}");
    assert_eq!(runner.recorded_calls().len(), 3);
}

/// The two probes disagree, and the local fork point is an ancestor of the
/// remote one: the remote one is nearer HEAD and wins.
#[test]
fn a_local_base_behind_its_remote_lists_from_the_remote_fork_point() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::ok(),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert_eq!(
        calls[2],
        format!("git -C /wt merge-base --is-ancestor {LOCAL_FORK} {REMOTE_FORK}")
    );
    assert!(calls[3].contains(REMOTE_FORK), "{calls:?}");
    assert!(!calls[3].contains(LOCAL_FORK), "{calls:?}");
}

/// The mirror image, and the truly-diverged case: the local ref wins.
#[test]
fn a_local_base_ahead_of_its_remote_lists_from_the_local_fork_point() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::fail_with_code(1, ""),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert!(calls[3].contains(LOCAL_FORK), "{calls:?}");
}

/// A base branch never checked out locally is ordinary: the read keeps
/// working on the remote ref alone, with no ranking probe.
#[test]
fn a_missing_local_base_branch_still_lists_from_the_remote_ref() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: Not a valid object name main\n"),
        sha(REMOTE_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[2].contains(REMOTE_FORK), "{calls:?}");
}

/// A repo with no remote-tracking ref lists from the local branch alone.
#[test]
fn a_missing_remote_base_ref_still_lists_from_the_local_branch() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        MockProcessRunner::fail("fatal: Not a valid object name origin/main\n"),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[2].contains(LOCAL_FORK), "{calls:?}");
}

/// A probe that exits zero but names no commit is not a candidate.
#[test]
fn a_probe_that_returns_no_commit_is_not_a_fork_point_candidate() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"),
        sha(REMOTE_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    assert!(runner.flattened_calls()[2].contains(REMOTE_FORK));
}

/// A commits-section read runs up to four commands, every one bounded.
#[test]
fn every_commit_list_query_is_bounded_by_a_timeout() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::ok(),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT); 4]);
}

/// A listing git refuses fails the read; it must not read as "no commits".
#[test]
fn a_failed_listing_fails_the_commit_list() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(LOCAL_FORK),
        MockProcessRunner::fail("fatal: your current branch appears to be broken\n"),
    ]);
    let err = git_branch_commits(Path::new("/wt"), "main", &runner)
        .expect_err("a failed listing must not read as an empty list")
        .to_string();
    assert!(err.contains("broken"), "got {err}");
}

fn real_commits(repo: &TestRepo) -> Vec<AgentCommit> {
    git_branch_commits(repo.root(), "main", &RealProcessRunner::default()).expect("commits")
}

fn agent_commit(id: &str, subject: &str) -> AgentCommit {
    AgentCommit {
        id: id.to_string(),
        subject: subject.to_string(),
    }
}

/// The agent's commits since the fork point, newest first, as (full id,
/// subject) pairs.
#[test]
fn the_commit_list_is_the_agents_commits_newest_first() {
    let repo = TestRepo::new();
    repo.write("a.rs", "a\n");
    let first = repo.commit_all("first change");
    repo.write("b.rs", "b\n");
    let second = repo.commit_all("second change: naïve résumé");

    assert_eq!(
        real_commits(&repo),
        vec![
            agent_commit(&second, "second change: naïve résumé"),
            agent_commit(&first, "first change"),
        ]
    );
    assert_eq!(second.len(), 40, "selection is keyed by the FULL id");
}

/// An agent that has not committed yet lists nothing — the honest answer.
#[test]
fn an_agent_with_no_commits_lists_none() {
    let repo = TestRepo::new();
    assert_eq!(real_commits(&repo), vec![]);
}

/// Other people's work on the base branch never appears: commits that land
/// on it after the fork are not the agent's.
#[test]
fn commits_landing_on_the_base_branch_after_the_fork_are_not_listed() {
    let repo = TestRepo::new();
    repo.commit_on_main("upstream.txt", "upstream work");
    repo.write("a.rs", "a\n");
    let mine = repo.commit_all("my work");

    assert_eq!(real_commits(&repo), vec![agent_commit(&mine, "my work")]);
}

/// Merging the base branch in moves the fork point forward, so the base
/// branch's commits never appear — while the merge commit the agent made
/// is the agent's own, and is listed.
#[test]
fn merging_the_base_branch_in_never_lists_its_commits() {
    let repo = TestRepo::new();
    repo.commit_on_main("upstream.txt", "upstream work");
    repo.write("a.rs", "a\n");
    let mine = repo.commit_all("my work");
    repo.git(&["merge", "-q", "--no-ff", "main", "-m", "merge main"]);
    let merge = repo.head();

    let ids: Vec<String> = real_commits(&repo).into_iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![merge, mine]);
}

/// Only the newest config.agent_tree_commits_max_listed commits are listed;
/// older ones are not, and no marker says so.
#[test]
fn only_the_newest_fifty_commits_are_listed() {
    let repo = TestRepo::new();
    for n in 1..=55 {
        repo.git(&["commit", "-q", "--allow-empty", "-m", &format!("c{n}")]);
    }

    let commits = real_commits(&repo);
    assert_eq!(commits.len(), crate::cli::agent_tree_commits::MAX_LISTED);
    assert_eq!(commits.first().map(|c| c.subject.as_str()), Some("c55"));
    assert_eq!(commits.last().map(|c| c.subject.as_str()), Some("c6"));
}

/// The stale-local-base case the two-ref resolution exists for: the
/// worktree was branched from the remote-tracking ref, which is ahead of a
/// local base the human never pulled. The upstream commit between the two
/// is not the agent's and must not be listed.
#[test]
fn a_remote_base_ahead_of_a_stale_local_one_moves_the_fork_point() {
    let repo = TestRepo::new();
    let upstream = repo.commit_on_main("upstream.txt", "upstream work");
    repo.git_main(&["update-ref", "refs/remotes/origin/main", &upstream]);
    repo.git_main(&["reset", "-q", "--hard", "HEAD~1"]);
    repo.git(&["reset", "-q", "--hard", "origin/main"]);
    repo.write("a.rs", "a\n");
    let mine = repo.commit_all("my work");

    assert_eq!(real_commits(&repo), vec![agent_commit(&mine, "my work")]);
}

// ---- refresh: failure keeps the last good tree ------------------------

/// The spec's AgentTreeGitFailureKeepsLastGoodTree: a failed query leaves
/// the tree exactly as it was and says so. Blanking on a transient index
/// lock — the commonest failure, taken by the agent's own git — would make
/// the pane flicker empty.
#[test]
fn a_failed_git_query_keeps_the_last_good_tree_and_sets_a_notice() {
    let mut state = RenderState::new();
    let mut tree = build_tree(&root(), &[]);

    let good = git_rig(&["M", "src/a.rs"], &[]);
    refresh(&root(), &good, &mut tree, &mut state);
    assert!(tree.node_at(&["src", "a.rs"]).is_some());
    assert!(state.notice.is_none());

    let bad = failing_git_rig("fatal: unable to read index.lock\n");
    refresh(&root(), &bad, &mut tree, &mut state);

    assert!(
        tree.node_at(&["src", "a.rs"]).is_some(),
        "the last good tree must survive"
    );
    let notice = state.notice.as_ref().expect("notice set");
    assert!(matches!(notice, Notice::Git(_)), "got {notice:?}");
    assert!(notice.text().contains("index.lock"), "got {notice:?}");
}

/// The warning must name WHY git failed. anyhow's plain Display prints only
/// the outermost context, so a timeout or a failed spawn logged as `could not
/// run git` and nothing else — task #4928's 2139 identical, undiagnosable cards.
#[tokio::test]
async fn a_git_query_that_could_not_run_logs_its_cause() {
    let log = crate::test_log::logged_during(|| async {
        let mut state = RenderState::new();
        let mut tree = build_tree(&root(), &[]);
        let timeout =
            || Err(anyhow::anyhow!("git timed out after 10s").context("could not run git"));
        let timed_out = MockProcessRunner::new(vec![timeout()]);
        refresh(&root(), &timed_out, &mut tree, &mut state);
    })
    .await;

    assert!(log.contains("git query failed"), "got {log}");
    assert!(log.contains("timed out after 10s"), "got {log}");
}

/// A working git retracts its own complaint on the next tick.
#[test]
fn a_recovering_git_query_clears_its_own_notice() {
    let mut state = RenderState::new();
    let mut tree = build_tree(&root(), &[]);

    let bad = failing_git_rig("fatal: unable to read index.lock\n");
    refresh(&root(), &bad, &mut tree, &mut state);
    assert!(state.notice.is_some());

    let good = git_rig(&["M", "a.rs"], &[]);
    refresh(&root(), &good, &mut tree, &mut state);
    assert!(state.notice.is_none());
}

/// ...but it must not swallow the answer to a keypress the user made half a
/// second ago, nor the commits section's own complaint. The notices share
/// one field and one line of border, so the source is what keeps them apart
/// — see NoticeSource in the spec.
#[test]
fn a_successful_git_query_leaves_other_writers_notices_alone() {
    for other in [
        Notice::diff("could not split the diff pane"),
        Notice::commit_list("not a valid object name main"),
    ] {
        let mut state = RenderState::new();
        let mut tree = build_tree(&root(), &[]);
        state.notice = Some(other.clone());

        let good = git_rig(&["M", "a.rs"], &[]);
        refresh(&root(), &good, &mut tree, &mut state);

        assert_eq!(state.notice, Some(other));
    }
}

/// A revert un-badges the file with no bookkeeping: git stops reporting it,
/// so the node goes. This is the second half of task #4408 — a file that is
/// not modified must not show as modified.
#[test]
fn a_reverted_file_disappears_from_the_tree() {
    let mut state = RenderState::new();
    let mut tree = build_tree(&root(), &[]);

    let dirty = git_rig(&["M", "a.rs"], &[]);
    refresh(&root(), &dirty, &mut tree, &mut state);
    assert!(tree.node_at(&["a.rs"]).is_some());

    let clean = git_rig(&[], &[]);
    refresh(&root(), &clean, &mut tree, &mut state);
    assert!(
        tree.node_at(&["a.rs"]).is_none(),
        "a reverted file must leave the tree"
    );
}

/// RefreshAgentTree asks about the SELECTED source: with a commit selected
/// the tick runs the one-commit form — naming that commit, with no
/// untracked listing — and the tree shows that commit's files.
#[test]
fn a_refresh_with_a_commit_selected_shows_that_commit() {
    let mut state = RenderState::new();
    state.selected_commit = Some(COMMIT.to_string());
    let mut tree = build_tree(&root(), &[]);

    let runner = commit_rig(&["A", "src/new.rs"], &["4\t0\tsrc/new.rs"]);
    refresh(&root(), &runner, &mut tree, &mut state);

    assert_eq!(
        tree.node_at(&["src", "new.rs"]).and_then(|n| n.badge),
        Some(FileChange::Added)
    );
    for call in runner.flattened_calls() {
        assert!(call.contains(COMMIT), "{call}");
        assert!(!call.contains("ls-files"), "{call}");
    }
}

// ---- Snapshots ---------------------------------------------------------

#[test]
fn snapshot_notice_is_shown_in_the_bottom_border() {
    let tree = build_tree(&root(), &three_node_changes());
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    state.notice = Some(Notice::diff("could not split the diff pane"));
    let mut terminal = Terminal::new(TestBackend::new(50, 12)).expect("terminal");
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "dispatch"))
        .expect("draw");
    let rendered = buffer_to_string(terminal.backend().buffer());

    assert!(
        rendered.contains("could not split"),
        "the notice must be visible; rendered:\n{rendered}"
    );
    insta::assert_snapshot!(rendered);
}

/// The border reddens with the notice — see AgentTreeNoticeRedensBorder.
/// Asserted on the styled buffer, not the plain text, because the whole
/// point is that the frame carries where a line of text does not.
#[test]
fn a_notice_reddens_the_whole_border() {
    let tree = build_tree(&root(), &three_node_changes());
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    let mut terminal = Terminal::new(TestBackend::new(50, 12)).expect("terminal");

    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "dispatch"))
        .expect("draw");
    let corner = terminal.backend().buffer()[(0, 0)].clone();
    assert_ne!(
        corner.style().fg,
        Some(RED),
        "no notice: the border must not be red"
    );

    state.notice = Some(Notice::git("git: fatal: unable to read index.lock"));
    terminal
        .draw(|frame| render(frame, frame.area(), &tree, &mut state, "dispatch"))
        .expect("draw");
    let buf = terminal.backend().buffer();
    for (x, y) in [(0u16, 0u16), (49, 0), (0, 11), (49, 11)] {
        assert_eq!(
            buf[(x, y)].style().fg,
            Some(RED),
            "corner ({x},{y}) must be red while a notice shows"
        );
    }
}

#[test]
fn snapshot_empty_tree_shows_bare_title() {
    let rendered = render_to_string(&[], "dispatch", 50, 10);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_added_modified_and_deleted_badges() {
    let changes = vec![
        added("src/new.rs"),
        modified("src/lib.rs"),
        deleted("README.md"),
    ];
    let rendered = render_to_string(&changes, "dispatch", 50, 12);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_nested_directories_auto_expanded() {
    let rendered = render_to_string(&[modified("a/b/c.rs")], "dispatch", 50, 12);
    insta::assert_snapshot!(rendered);
}

/// The shape task #4716 is about: merged routes on one row each, and the
/// counts on the folder nearest the files rather than on every folder
/// above it.
#[test]
fn snapshot_merged_routes_and_nearest_folder_counts() {
    let changes = vec![
        counted("docs/specs/agent-tree.allium", FileChange::Modified, 40, 3),
        counted("src/main.rs", FileChange::Modified, 12, 0),
        counted("src/cli/agent_tree.rs", FileChange::Modified, 80, 20),
        counted("src/tui/ui/kanban/columns.rs", FileChange::Modified, 40, 10),
    ];
    let rendered = render_to_string(&changes, "dispatch", 50, 12);
    insta::assert_snapshot!(rendered);
}

/// The only form that exercises the widget's own open-set lookup, and
/// so the only one that can catch a key-representation mismatch: an
/// assertion over `opened()` can encode a key that matches no node and
/// still pass, because `TreeState::open` reports success on it (#3811).
/// Every directory on the way to a changed file is expanded, so the
/// leaf is on screen with no keypresses.
#[test]
fn deeply_nested_changed_file_is_visible_without_manual_expansion() {
    let rendered = render_to_string(&[modified("a/b/c/d/leaf.rs")], "dispatch", 50, 12);
    assert!(
        rendered.contains("leaf.rs"),
        "the leaf must be visible unaided; rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains('▶'),
        "no directory may render collapsed; rendered:\n{rendered}"
    );
}

/// A change to a sibling can merge or unmerge a chain, which renames the
/// row and so changes the identifier the widget stores its expansion
/// under. The collapse must survive that: RefreshAgentTree says a manual
/// collapse is not overwritten by the next refresh, unconditionally.
///
/// Collapsed with the real key rather than by reaching into the widget,
/// because what the renderer remembers about a collapse is recorded from
/// the key press — a direct `close` records nothing and would test a path
/// no user can take.
#[test]
fn manual_collapse_survives_a_refresh_that_unmerges_the_chain() {
    let mut rig = KeyRig::new(&[modified("a/b/c.rs"), modified("a/b/d.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a/b".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a/b"]), "precondition: the row is collapsed");

    // `a/z.rs` gives `a` a file of its own, so `a/b` unmerges into
    // `a` -> `b` and the collapsed row is re-keyed.
    rig.refresh(&[
        modified("a/b/c.rs"),
        modified("a/b/d.rs"),
        modified("a/z.rs"),
    ]);

    assert!(
        !rig.is_open(&["a", "b"]),
        "re-keying the row must not re-open it; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The mirror: the agent reverts the sibling, the chain merges again, and
/// the merged row must not spring open either.
#[test]
fn manual_collapse_survives_a_refresh_that_merges_the_chain() {
    let mut rig = KeyRig::new(&[modified("a/b/c.rs"), modified("a/z.rs")]);
    // Rows under `a` are its own file first, then the subfolder.
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string(), "b".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a", "b"]), "precondition: b is collapsed");

    rig.refresh(&[modified("a/b/c.rs")]);

    assert!(
        !rig.is_open(&["a/b"]),
        "re-keying the row must not re-open it; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The OUTER link of a chain. The user collapses `a` while it has a file
/// of its own; the agent reverts that file; the chain merges into `a/b`.
/// The merged row stands for `a` as much as for `a/b`, so it must come up
/// collapsed — the row absorbed the one the user closed, it did not
/// replace it.
#[test]
fn manual_collapse_survives_the_row_being_absorbed_into_a_merged_one() {
    let mut rig = KeyRig::new(&[modified("a/x.rs"), modified("a/b/c.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a"]), "precondition: a is collapsed");

    rig.refresh(&[modified("a/b/c.rs")]);

    assert!(
        !rig.is_open(&["a/b"]),
        "the merged row absorbed the collapsed one; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The mirror of the test above. Having inherited a collapse, the row is
/// opened by hand — and that open must clear the collapse recorded on
/// every link the row stands for, or a later unmerge hands the collapse
/// back to the row it came from and discards what the user asked for. An
/// explicit open cannot be weaker than an explicit close.
#[test]
fn opening_a_merged_row_clears_the_collapse_it_inherited() {
    let mut rig = KeyRig::new(&[modified("a/x.rs"), modified("a/b/c.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string()]);
    rig.press(KeyCode::Char('h'));

    // The chain merges; the row inherits the collapse (the test above).
    rig.refresh(&[modified("a/b/c.rs")]);
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a/b".to_string()]);
    assert!(!rig.is_open(&["a/b"]), "precondition: inherited collapse");

    rig.press(KeyCode::Char('l'));
    assert!(rig.is_open(&["a/b"]), "precondition: the user opened it");

    // The sibling file comes back and the chain unmerges again.
    rig.refresh(&[modified("a/x.rs"), modified("a/b/c.rs")]);

    assert!(
        rig.is_open(&["a"]),
        "the open must not be discarded; opened: {:?}",
        rig.state.tree_state.opened()
    );
}

/// The other half of RefreshAgentTree's rule 3, and the mirror of the two
/// tests above: when NOBODY collapsed anything, a refresh that unmerges a
/// chain must leave the rows it creates expanded. A row the user never
/// touched, rendered collapsed, hides a changed file's badge on a refresh
/// the user did not ask for — which is the failure the whole pane exists
/// to prevent.
#[test]
fn a_refresh_that_unmerges_a_chain_leaves_the_new_rows_expanded() {
    let mut rig = KeyRig::new(&[modified("a/b/c.rs")]);
    rig.refresh(&[modified("a/b/c.rs"), modified("a/x.rs")]);

    assert!(
        rig.is_open(&["a"]),
        "opened: {:?}",
        rig.state.tree_state.opened()
    );
    assert!(
        rig.is_open(&["a", "b"]),
        "opened: {:?}",
        rig.state.tree_state.opened()
    );
    let rendered = rig.rendered();
    assert!(
        rendered.contains("c.rs"),
        "the badge below must be on screen unaided:\n{rendered}"
    );
}

#[test]
fn manually_collapsed_nested_directory_stays_collapsed_on_refresh() {
    // `a/x.rs` keeps `a` from merging into `b`, so there is a genuinely
    // nested directory to collapse. Rows: a, x.rs, b, c.rs.
    let changes = [modified("a/x.rs"), modified("a/b/c.rs")];
    let mut rig = KeyRig::new(&changes);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('j'));
    assert_eq!(rig.selected(), vec!["a".to_string(), "b".to_string()]);
    rig.press(KeyCode::Char('h'));
    assert!(!rig.is_open(&["a", "b"]));

    rig.refresh(&changes);
    assert!(!rig.is_open(&["a", "b"]));
}

// ---- agents section (docs/specs/agent-tree.allium: Agents Section) ----

use crate::cli::agent_tree_agents::AgentRow;
use crate::models::{test_tmux_window, TaskId};

fn agent(id: i64, own: bool) -> AgentRow {
    crate::cli::agent_tree_agents::test_row(id, own)
}

impl KeyRig {
    fn with_agents(changes: &[GitFileChange], agents: Vec<AgentRow>) -> Self {
        let mut rig = Self::new(changes);
        rig.state.adopt_agent_list(Ok(agents));
        rig.draw();
        rig
    }
}

#[test]
fn the_pane_starts_with_the_tree_focused() {
    assert_eq!(RenderState::new().focus, Focus::Tree);
}

/// SwitchAgentTreeFocus: a cycle in screen order — tree, then commits,
/// then agents, then tree again.
#[test]
fn tab_cycles_focus_tree_then_commits_then_agents_then_tree() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(1, false)]);
    assert_eq!(rig.press(KeyCode::Tab), KeyAction::Continue);
    assert_eq!(rig.state.focus, Focus::Commits);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Agents);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Tree);
}

/// An empty section is a real place to be: Tab never skips one.
#[test]
fn tab_works_with_no_commits_or_agents_listed() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Commits);
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.focus, Focus::Agents);
}

/// Tab is the ONLY focus key: Shift+Tab (BackTab) does not cycle backwards.
#[test]
fn back_tab_does_not_move_focus() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.press_with(KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(rig.state.focus, Focus::Tree);
}

#[test]
fn tab_clears_a_notice() {
    let mut rig = KeyRig::new(&[modified("a.rs")]);
    rig.state.notice = Some(Notice::agent_list("db locked"));
    rig.press(KeyCode::Tab);
    assert_eq!(rig.state.notice, None);
}

// The agents-section tests below set focus directly: they are about the
// section, not about how Tab reaches it (SwitchAgentTreeFocus, above).

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
fn a_still_toggles_every_diff_with_the_agents_section_focused() {
    let mut rig = KeyRig::with_agents(&[modified("a.rs")], vec![agent(7, false)]);
    rig.state.focus = Focus::Agents;
    assert_eq!(rig.press(KeyCode::Char('a')), KeyAction::DiffSetChanged);
    assert!(rig.state.is_diff_open(Path::new("a.rs")));
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
    let mut state = RenderState::new();
    state.adopt_agent_list(Ok(vec![agent(1, false)]));
    state.adopt_agent_list(Err("database is locked".to_string()));
    assert_eq!(state.agents.rows().len(), 1);
    assert_eq!(state.notice, Some(Notice::agent_list("database is locked")));
}

#[test]
fn a_working_agent_read_clears_only_its_own_notice() {
    let mut state = RenderState::new();
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
        let mut state = RenderState::new();
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
    let mut state = RenderState::new();
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
    let mut state = RenderState::new();
    jump_to_agent(&test_tmux_window("task-7"), &mut state, &runner);
    assert_eq!(state.notice, None);
    assert!(runner
        .recorded_calls()
        .iter()
        .any(|(_, args)| args.first().map(String::as_str) == Some("select-window")));
}

// ---- AgentsSectionSitsBelowTheTree -----------------------------------

fn render_pane_to_string(state: &mut RenderState, tree: &TreeNode, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(40, height)).expect("terminal");
    terminal
        .draw(|frame| render_pane(frame, frame.area(), tree, state, "wt"))
        .expect("draw");
    buffer_to_string(terminal.backend().buffer())
}

#[test]
fn the_agents_section_sits_below_the_tree() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = RenderState::new();
    state.sync_expansion(&tree);
    state.adopt_agent_list(Ok(vec![agent(4941, false), agent(4942, true)]));
    let out = render_pane_to_string(&mut state, &tree, 12);
    let lines: Vec<&str> = out.lines().collect();
    let tree_row = lines.iter().position(|l| l.contains("a.rs")).expect(&out);
    let header = lines.iter().position(|l| l.contains("Agents")).expect(&out);
    let first = lines.iter().position(|l| l.contains("#4941")).expect(&out);
    assert!(tree_row < header && header < first, "{out}");
    assert_eq!(
        first,
        lines.len() - 3,
        "the section is at the bottom:\n{out}"
    );
}

#[test]
fn the_agents_section_shows_with_no_agents() {
    let tree = build_tree(&root(), &[modified("a.rs")]);
    let mut state = RenderState::new();
    let out = render_pane_to_string(&mut state, &tree, 12);
    assert!(out.contains("Agents"), "{out}");
}

// ---- the keybinding table drives the pane (docs/specs/keybindings.allium) ----

use crate::keybindings::{bindings_in, KeyContext, KeyNamespace, ANY_OTHER_KEY};

fn recorded_usage(state: &RenderState) -> Vec<(String, Option<String>)> {
    state
        .usage
        .iter()
        .map(|e| (e.action.clone(), e.detail.clone()))
        .collect()
}

/// A pane set up so `binding`'s context holds and its action takes effect.
fn rig_for(binding: &crate::keybindings::KeyBinding) -> KeyRig {
    let mut rig = KeyRig::with_agents(&three_node_changes(), vec![agent(1, false)]);
    match binding.context {
        Some(KeyContext::OnFile) => {
            rig.press(KeyCode::Char('j'));
        }
        Some(KeyContext::OnDirectory) => {
            for _ in 0..3 {
                rig.press(KeyCode::Char('j'));
            }
        }
        None => {}
        Some(other) => panic!("the tree pane has no context {other:?}"),
    }
    if binding.namespace == KeyNamespace::AgentTreeAgents {
        rig.state.focus = Focus::Agents;
    }
    rig.state.usage.clear();
    rig
}

/// press_every_row_key / RecordedActionMatchesRow for the tree and Agents
/// sections: each key of each row records the row's action id and the key.
#[test]
fn pressing_each_key_of_each_pane_row_records_the_rows_action() {
    let mut pressed = 0;
    for ns in [KeyNamespace::AgentTreeTree, KeyNamespace::AgentTreeAgents] {
        for binding in bindings_in(ns) {
            assert!(!binding.keys.contains(&ANY_OTHER_KEY));
            for key in binding.keys {
                let mut rig = rig_for(binding);
                if *key == "gg" {
                    rig.press(KeyCode::Char('j'));
                    rig.state.usage.clear();
                    rig.press(KeyCode::Char('g'));
                    assert!(rig.state.usage.is_empty(), "first g is pending input");
                    rig.press(KeyCode::Char('g'));
                } else {
                    let ev = crate::cli::test_key_event(key);
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
                    "{} {key}",
                    ns.name()
                );
            }
        }
    }
    assert!(pressed > 30, "{pressed}");
}

/// A modified press with no row does nothing and records nothing, in either
/// section: Ctrl+j is not `j`, Ctrl+a is not `a`, Ctrl+q is not `q`.
#[test]
fn a_modified_press_with_no_row_does_nothing_in_the_pane() {
    for focus in [Focus::Tree, Focus::Commits, Focus::Agents] {
        for key in ["Ctrl+J", "Ctrl+A", "Ctrl+Q", "Ctrl+G", "Ctrl+L"] {
            let mut rig = KeyRig::with_agents(&three_node_changes(), vec![agent(1, false)]);
            rig.state.focus = focus;
            let before = rig.selected();
            let ev = crate::cli::test_key_event(key);
            assert_eq!(
                handle_key(&mut rig.state, &rig.tree, ev),
                KeyAction::Continue
            );
            assert!(rig.state.usage.is_empty(), "{key}");
            assert_eq!(rig.selected(), before, "{key}");
            assert!(rig.state.open_diffs().is_empty(), "{key}");
        }
    }
}

/// An unlisted key, and a press whose context fails, record nothing.
#[test]
fn unbound_and_contextless_presses_record_nothing() {
    let mut rig = KeyRig::new(&three_node_changes());
    rig.press(KeyCode::Char('x'));
    // Nothing selected: Space/Enter/l have no applicable row.
    for code in [KeyCode::Char(' '), KeyCode::Enter, KeyCode::Char('l')] {
        rig.press(code);
    }
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
    // h/l and Space do nothing in the Agents section with no agent.
    rig.state.focus = Focus::Agents;
    rig.press(KeyCode::Char('h'));
    rig.press(KeyCode::Char('l'));
    rig.press(KeyCode::Char(' '));
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
    // h/l/Left/Right have no row in the commits section, and Space/Enter on
    // the row already selected ("unstaged work", the default) does nothing.
    rig.state.focus = Focus::Commits;
    for code in [
        KeyCode::Char('h'),
        KeyCode::Char('l'),
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Char(' '),
        KeyCode::Enter,
    ] {
        rig.press(code);
    }
    assert!(rig.state.usage.is_empty(), "{:?}", rig.state.usage);
}

// ---- commits section (docs/specs/agent-tree.allium: Commits Section) ----

use crate::cli::agent_tree_commits::{CommitsSection, MAX_ROWS as COMMITS_MAX_ROWS};

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
    use crate::cli::agent_tree_commits::{COMMITS_REFRESH_INTERVAL, MAX_LISTED};
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
        crate::agent_tree_open_set::read_selected_source(&worktree),
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
                let ev = crate::cli::test_key_event(key);
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

// ---- PanesReadThroughTheBoard: the agents section's read (task #4982)

struct FakeBoard(std::result::Result<crate::hooks::wire::PaneView, String>);

#[async_trait::async_trait]
impl crate::cli::PaneViewSource for FakeBoard {
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
    }
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
