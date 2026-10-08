//! The agent-tree pane's tests, one file per module they exercise. Shared
//! fixtures live here.

use super::changes::*;
use super::keys::*;
use super::list_cursor::ListCursor;
use super::model::tests::nul_stream;
use super::model::*;
use super::pane::buffer_to_string;
use super::render::agents::AgentRow;
use super::render::commits::MAX_ROWS as COMMITS_MAX_ROWS;
use super::render::commits::*;
use super::render::*;
use super::run::*;
use super::state::*;
use super::test_repo::TestRepo;
use crate::keybindings::{bindings_in, KeyContext, KeyNamespace, ANY_OTHER_KEY};
use crate::models::{test_tmux_window, TaskId};
use crate::palette::{FG, MUTED, RED};
use crate::process::MockProcessRunner;
use crate::process::RealProcessRunner;
use anyhow::Result;
use crossterm::event::KeyModifiers;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::path::PathBuf;

mod agents;
mod changes;
mod commits;
mod keys;
mod render;
mod run;

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

/// A commit the user has selected in the commits section.
const COMMIT: &str = "3333333333333333333333333333333333333333";

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
        MockProcessRunner::ok_with_stdout(nul_stream(diff).as_bytes()),
        MockProcessRunner::ok_with_stdout(nul_stream(numstat).as_bytes()),
    ])
}

fn recorded_usage(state: &RenderState) -> Vec<(String, Option<String>)> {
    state
        .usage
        .iter()
        .map(|e| (e.action.clone(), e.detail.clone()))
        .collect()
}

fn agent(id: i64, own: bool) -> AgentRow {
    crate::agent_tree::render::agents::test_row(id, own)
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
        MockProcessRunner::ok_with_stdout(nul_stream(diff).as_bytes()),
        MockProcessRunner::ok_with_stdout(nul_stream(numstat).as_bytes()),
        MockProcessRunner::ok_with_stdout(nul_stream(untracked).as_bytes()),
    ]
}

impl KeyRig {
    fn with_agents(changes: &[GitFileChange], agents: Vec<AgentRow>) -> Self {
        let mut rig = Self::new(changes);
        rig.state.adopt_agent_list(Ok(agents));
        rig.draw();
        rig
    }
}
