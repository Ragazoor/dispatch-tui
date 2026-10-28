//! The diff pane's lifecycle as seen from the tree that owns it: split it when
//! something is open, kill it when nothing is (see
//! `docs/specs/agent-tree.allium`'s `SplitAgentTreeDiffPane` and
//! `CloseAgentTreeDiffPaneWhenEmpty`).
//!
//! Sibling of [`crate::agent_tree::model`], which owns tree *building*, and of
//! [`crate::agent_tree::diff_viewer`], which owns what the pane draws once it
//! exists. This
//! module owns only the tmux effect, so the renderer's key handling can stay a
//! pure function of the keys pressed.
//!
//! Replaces the editor pane this feature removed, and inherits its pane-role
//! marker and its accepted marker gap — the differences are that the new pane
//! spans the window's full width, and that it is killed when nothing is open.
//! It is also reused to show the editor (`show_in_editor`). The pane shows one
//! file at a time: opening a file replaces what it shows, an editor included,
//! and moves focus to it (`FocusFollowsTheOpenedFile`). Only emptying the open
//! set leaves an editor running (`EditorPaneOutlivesItsTree`).

use std::path::Path;

use anyhow::{Context, Result};

use crate::models::TaskId;
use crate::process::ProcessRunner;
use crate::tmux::{
    self, PANE_CONTENT_DIFF, PANE_CONTENT_EDITOR, PANE_CONTENT_OPTION, PANE_ROLE_DIFF,
    PANE_ROLE_OPTION,
};

/// Height of the lower pane as a percentage of the AGENT WINDOW, spanning its
/// full width — matches `config.agent_tree_diff_pane_percent` in
/// docs/specs/agent-tree.allium.
const DIFF_PANE_PERCENT: u8 = 50;

/// The pane this process is running in, from `$TMUX_PANE`.
///
/// tmux exports it into every pane it creates, so it is present whenever the
/// renderer runs where it is meant to. Its absence means the renderer was
/// started outside tmux, which is a real (if unusual) way to run it — hence an
/// error the caller can show, not a panic.
pub fn current_pane_from_env() -> Result<String> {
    std::env::var("TMUX_PANE")
        .ok()
        .filter(|p| !p.is_empty())
        .context("not running inside tmux ($TMUX_PANE is unset)")
}

/// This agent window's lower pane, if it has one, and whether it runs the
/// editor.
///
/// Value-matched, not presence-matched: the tree pane this is called from
/// carries the same option under a different role, and mistaking one for the
/// other would let a close take the tree away.
fn existing_lower_pane(
    my_pane: &str,
    runner: &dyn ProcessRunner,
) -> Result<Option<(String, bool)>> {
    let Some(pane) =
        tmux::pane_ids_with_option_value(my_pane, PANE_ROLE_OPTION, PANE_ROLE_DIFF, runner)
            .context("failed to look for an existing diff pane")?
            .into_iter()
            .next()
    else {
        return Ok(None);
    };
    let editor =
        tmux::pane_ids_with_option_value(my_pane, PANE_CONTENT_OPTION, PANE_CONTENT_EDITOR, runner)
            .context("failed to tell whether the lower pane runs the editor")?
            .contains(&pane);
    Ok(Some((pane, editor)))
}

/// Make the open set and the panes agree: split a diff pane when something is
/// open and there is none, kill it when nothing is open and there is one.
///
/// Called on every change to the open set, which is what makes a pane the user
/// killed themselves come back on their next toggle rather than leaving them
/// with a set of open files and nowhere to see them. Deliberately NOT called on
/// the refresh tick: a pane the user closed a second ago must stay closed until
/// they ask again.
///
/// A pane running the editor is replaced by the diff renderer when something
/// is open (`ReplaceAgentTreeEditorWithDiff`) and left running when nothing is
/// (`EditorPaneOutlivesItsTree`). Whenever something is open, focus ends in the
/// lower pane (`FocusFollowsTheOpenedFile`).
///
/// `my_pane` is the calling renderer's own pane id, used both as the split
/// target and to identify the window to look in — every pane involved is in the
/// agent's own window.
pub fn reconcile_diff_pane(
    my_pane: &str,
    data_dir: &Path,
    task_id: TaskId,
    worktree: &Path,
    anything_open: bool,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let pane = match (anything_open, existing_lower_pane(my_pane, runner)?) {
        (true, None) => split_diff_pane(my_pane, data_dir, task_id, worktree, runner)?,
        (true, Some((pane, true))) => {
            let command = diff_command(data_dir, task_id, runner);
            run_in(worktree, &command, |cwd, argv| {
                tmux::respawn_pane_running(&pane, cwd, argv, runner)
            })?;
            mark_content(&pane, PANE_CONTENT_DIFF, runner);
            pane
        }
        // The diff renderer already there reads the new open set by itself.
        (true, Some((pane, false))) => pane,
        (false, Some((pane, false))) => return tmux::kill_pane(&pane, runner),
        // Nothing is open and there is no pane, or an editor pane is left be.
        (false, None) | (false, Some((_, true))) => return Ok(()),
    };
    focus(&pane, runner);
    Ok(())
}

/// Show `relative` — a path below `worktree` — in the user's editor in the
/// lower pane (`OpenAgentTreeFileInEditor`): respawn the pane we created, or
/// split a fresh one when there is none. Never touches a pane we did not
/// create. A pane already running the editor is replaced too, losing any
/// unsaved edits: the user chose that over having to quit it first (task
/// 98620). A path with no working-tree file is refused, because an editor
/// handed a missing path opens a misleading empty buffer. Focus ends in the
/// pane.
pub fn show_in_editor(
    my_pane: &str,
    worktree: &Path,
    relative: &Path,
    editor: &[String],
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let file = worktree.join(relative);
    if !file.exists() {
        anyhow::bail!(
            "{} no longer exists; Space shows its diff",
            relative.display()
        );
    }
    let cwd = worktree.to_string_lossy().into_owned();
    let file_arg = file.to_string_lossy().into_owned();
    let mut command: Vec<&str> = editor.iter().map(String::as_str).collect();
    command.push(file_arg.as_str());

    let pane = match existing_lower_pane(my_pane, runner)? {
        Some((pane, _)) => {
            tmux::respawn_pane_running(&pane, &cwd, &command, runner)?;
            pane
        }
        None => {
            let pane = tmux::split_window_below_running(
                my_pane,
                DIFF_PANE_PERCENT,
                &cwd,
                &command,
                runner,
            )?;
            mark_diff_pane(&pane, runner);
            pane
        }
    };
    mark_content(&pane, PANE_CONTENT_EDITOR, runner);
    focus(&pane, runner);
    Ok(())
}

/// Move tmux focus to the lower pane the user just opened a file in
/// (`FocusFollowsTheOpenedFile`). The pane is already showing the file, so a
/// failure is logged rather than reported as a failed open.
fn focus(pane: &str, runner: &dyn ProcessRunner) {
    if let Err(e) = tmux::select_pane(pane, runner) {
        tracing::warn!(%pane, error = %e, "failed to move focus to the lower pane");
    }
}

/// Hand a tmux helper the worktree as its working directory and `command` as
/// borrowed argv, the shape every pane-running helper takes.
fn run_in<T>(worktree: &Path, command: &[String], f: impl FnOnce(&str, &[&str]) -> T) -> T {
    let cwd = worktree.to_string_lossy();
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    f(&cwd, &argv)
}

/// The diff renderer's argv. `--data-dir` is named explicitly rather than left
/// to the default: this process was given a database path and the pane below
/// it must read the SAME open set and the same task; inheriting a default that
/// happened to match would be a coincidence, not a guarantee.
fn diff_command(data_dir: &Path, task_id: TaskId, runner: &dyn ProcessRunner) -> Vec<String> {
    vec![
        runner.agent_binaries().dispatch,
        "--data-dir".to_string(),
        data_dir.to_string_lossy().into_owned(),
        "agent-diff".to_string(),
        task_id.to_string(),
    ]
}

/// The pane is open and showing something; the marker only matters to the NEXT
/// reconcile, so a failure here is logged rather than reported as a failed
/// open. Worst case a later toggle splits a second pane — the same accepted
/// gap the editor pane's marker had, and the reason OneDiffPanePerAgentWindow
/// is held by construction rather than enforced.
fn mark_diff_pane(pane: &str, runner: &dyn ProcessRunner) {
    if let Err(e) = tmux::set_pane_option(pane, PANE_ROLE_OPTION, PANE_ROLE_DIFF, runner) {
        tracing::warn!(%pane, error = %e, "failed to mark the diff pane with its role");
    }
}

/// Record what the pane runs, for the next renderer to read back. Logged on
/// failure like [`mark_diff_pane`]: the pane is already open. The cost of a
/// missed editor mark is that the pane reads as a diff, so keep this write
/// right after the pane is created.
fn mark_content(pane: &str, content: &str, runner: &dyn ProcessRunner) {
    if let Err(e) = tmux::set_pane_option(pane, PANE_CONTENT_OPTION, content, runner) {
        tracing::warn!(%pane, content, error = %e, "failed to mark what the lower pane runs");
    }
}

fn split_diff_pane(
    my_pane: &str,
    data_dir: &Path,
    task_id: TaskId,
    worktree: &Path,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    let command = diff_command(data_dir, task_id, runner);
    let pane = run_in(worktree, &command, |cwd, argv| {
        tmux::split_window_below_running(my_pane, DIFF_PANE_PERCENT, cwd, argv, runner)
    })?;
    mark_diff_pane(&pane, runner);
    mark_content(&pane, PANE_CONTENT_DIFF, runner);
    Ok(pane)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::MockProcessRunner;

    fn db() -> &'static Path {
        Path::new("/data")
    }

    fn worktree() -> &'static Path {
        Path::new("/work/wt")
    }

    /// No pane carries the diff role, then the split, the two markers (role,
    /// content) and the focus move succeed.
    fn no_pane_then_split() -> MockProcessRunner {
        MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n"),
            MockProcessRunner::ok_with_stdout(b"%9\n"),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ])
    }

    fn assert_focus_moved_to_the_lower_pane(runner: &MockProcessRunner) {
        let calls = runner.flattened_calls();
        assert_eq!(
            calls.last().map(String::as_str),
            Some("tmux select-pane -t %9"),
            "opening a file must move focus to it; got {calls:?}"
        );
    }

    /// A diff pane already exists, running the diff renderer, then whatever the
    /// caller does with it.
    fn existing_pane(rest: Vec<anyhow::Result<std::process::Output>>) -> MockProcessRunner {
        pane_running(b"%9 diff\n", rest)
    }

    /// A lower pane already exists, running the editor.
    fn existing_editor_pane(rest: Vec<anyhow::Result<std::process::Output>>) -> MockProcessRunner {
        pane_running(b"%9 editor\n", rest)
    }

    /// Role lookup finds `%9`; the content lookup answers `content_rows`.
    fn pane_running(
        content_rows: &[u8],
        rest: Vec<anyhow::Result<std::process::Output>>,
    ) -> MockProcessRunner {
        let mut queued = vec![
            MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n%9 diff\n"),
            MockProcessRunner::ok_with_stdout(content_rows),
        ];
        queued.extend(rest);
        MockProcessRunner::new(queued)
    }

    #[test]
    fn opening_the_first_diff_splits_a_pane_below_the_tree() {
        let runner = no_pane_then_split();

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap();

        let calls = runner.flattened_calls();
        assert!(
            calls[1].contains("split-window") && calls[1].contains("-t %2"),
            "must split from the tree's own pane; got {calls:?}"
        );
        assert_focus_moved_to_the_lower_pane(&runner);
    }

    /// The pane must read the SAME database as the tree, so it sees the same
    /// open set and the same task. Inheriting a default that happened to match
    /// would be a coincidence, not a guarantee.
    #[test]
    fn the_diff_pane_is_told_which_database_and_task_to_read() {
        let runner = no_pane_then_split();

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap();

        let split = &runner.flattened_calls()[1];
        assert!(split.contains("--data-dir /data"), "got {split}");
        assert!(split.contains("agent-diff 42"), "got {split}");
    }

    #[test]
    fn the_new_pane_is_marked_with_the_diff_role() {
        let runner = no_pane_then_split();

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap();

        let mark = &runner.flattened_calls()[2];
        assert!(
            mark.contains("set-option") && mark.contains("%9") && mark.ends_with("diff"),
            "got {mark}"
        );
    }

    /// Splitting again on every toggle would subdivide the column until the
    /// window ran out of room. The renderer redraws the new file by itself;
    /// only focus moves (FocusFollowsTheOpenedFile).
    #[test]
    fn opening_another_diff_reuses_the_pane_and_moves_focus_to_it() {
        let runner = existing_pane(vec![MockProcessRunner::ok()]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap();

        assert_eq!(runner.flattened_calls().len(), 3, "lookups and focus only");
        assert_focus_moved_to_the_lower_pane(&runner);
    }

    /// Focus is a convenience: the pane is open and showing the file, so a
    /// failed move is logged, not reported as a failed open.
    #[test]
    fn a_failed_focus_move_still_counts_as_opened() {
        let runner = existing_pane(vec![MockProcessRunner::fail("no such pane")]);

        assert!(reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).is_ok());
    }

    #[test]
    fn closing_the_last_diff_kills_the_pane() {
        let runner = existing_pane(vec![MockProcessRunner::ok()]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, &runner).unwrap();

        assert_eq!(
            runner.flattened_calls()[2],
            "tmux kill-pane -t %9".to_string()
        );
    }

    /// The resting state, and by far the commonest call: nothing is open and
    /// there is no pane. It must cost one lookup and no tmux mutation.
    #[test]
    fn nothing_open_and_no_pane_changes_nothing() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"%1 \n%2 agent_tree\n",
        )]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, &runner).unwrap();

        assert_eq!(runner.flattened_calls().len(), 1);
    }

    /// The role lookup is matched on its exact VALUE. The tree pane carries the
    /// same option under a different role, and mistaking one for the other
    /// would let a close take the tree away.
    #[test]
    fn the_trees_own_pane_is_never_mistaken_for_the_diff_pane() {
        let runner = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%2 agent_tree\n"),
            MockProcessRunner::ok_with_stdout(b"%9\n"),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap();

        let calls = runner.flattened_calls();
        assert!(
            !calls.iter().any(|c| c.contains("kill-pane")),
            "got {calls:?}"
        );
        assert!(calls[1].contains("split-window"), "got {calls:?}");
    }

    /// A failed lookup must not degrade to "there is no diff pane": that would
    /// split a fresh one on every toggle until the column ran out of room.
    #[test]
    fn a_failed_role_lookup_is_an_error_rather_than_a_second_pane() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("no server running")]);

        let err =
            reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap_err();

        assert!(
            format!("{err:#}").contains("existing diff pane"),
            "got {err:#}"
        );
    }

    /// The pane is open and rendering; the marker only matters to the next
    /// reconcile, so failing to write it must not report a failed open.
    #[test]
    fn a_pane_that_opened_but_could_not_be_marked_still_counts_as_opened() {
        let runner = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%2 agent_tree\n"),
            MockProcessRunner::ok_with_stdout(b"%9\n"),
            MockProcessRunner::fail("unknown option"),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ]);

        assert!(reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).is_ok());
    }

    // ---- OpenAgentTreeFileInEditor / ShowDiffInLowerPane ------------------

    fn editor() -> Vec<String> {
        vec!["vim".to_string(), "-p".to_string()]
    }

    /// A real worktree directory holding `a.rs`, so the existence check passes.
    fn worktree_with_file() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[test]
    fn v_with_no_lower_pane_splits_one_running_the_editor_on_the_file() {
        let wt = worktree_with_file();
        let runner = no_pane_then_split();

        show_in_editor("%2", wt.path(), Path::new("a.rs"), &editor(), &runner).unwrap();

        let calls = runner.flattened_calls();
        assert!(
            calls[1].contains("split-window") && calls[1].contains("-f"),
            "{calls:?}"
        );
        assert!(
            calls[1].ends_with(&format!("vim -p {}", wt.path().join("a.rs").display())),
            "{calls:?}"
        );
        assert!(
            calls[2].contains("set-option") && calls[2].ends_with("diff"),
            "{calls:?}"
        );
        assert!(
            calls[3].contains("@dispatch_pane_content") && calls[3].ends_with("editor"),
            "the pane must be marked as running the editor; {calls:?}"
        );
        assert_focus_moved_to_the_lower_pane(&runner);
    }

    /// The pane we created is reused, never split again.
    #[test]
    fn v_with_a_lower_pane_respawns_it_with_the_editor() {
        let wt = worktree_with_file();
        let runner = existing_pane(vec![
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ]);

        show_in_editor("%2", wt.path(), Path::new("a.rs"), &editor(), &runner).unwrap();

        let calls = runner.flattened_calls();
        assert_eq!(calls.len(), 5, "{calls:?}");
        assert!(
            calls[2].contains("respawn-pane -k") && calls[2].contains("-t %9"),
            "{calls:?}"
        );
        assert!(calls[2].contains("vim -p"), "{calls:?}");
        assert!(
            calls[3].contains("@dispatch_pane_content") && calls[3].ends_with("editor"),
            "{calls:?}"
        );
        assert_focus_moved_to_the_lower_pane(&runner);
    }

    #[test]
    fn v_on_a_file_with_no_working_tree_copy_is_refused_without_touching_tmux() {
        let wt = worktree_with_file();
        let runner = MockProcessRunner::new(vec![]);

        let err =
            show_in_editor("%2", wt.path(), Path::new("gone.rs"), &editor(), &runner).unwrap_err();

        assert!(format!("{err}").contains("no longer exists"), "{err}");
        assert!(runner.flattened_calls().is_empty());
    }

    // ---- ReplaceAgentTreeEditorWithDiff / EditorPaneOutlivesItsTree ------

    fn assert_no_pane_mutation(runner: &MockProcessRunner) {
        let calls = runner.flattened_calls();
        assert!(
            !calls.iter().any(|c| {
                c.contains("kill-pane") || c.contains("respawn-pane") || c.contains("split-window")
            }),
            "an editor pane must be left alone; got {calls:?}"
        );
    }

    /// Opening a diff while the editor shows replaces the editor in the same
    /// pane: the user chose that over "quit the editor first" (task 98620).
    #[test]
    fn opening_a_diff_while_the_editor_shows_replaces_it_with_the_diff() {
        let runner = existing_editor_pane(vec![
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, &runner).unwrap();

        let calls = runner.flattened_calls();
        assert!(
            calls[2].contains("respawn-pane -k")
                && calls[2].contains("-t %9")
                && calls[2].contains("agent-diff 42"),
            "{calls:?}"
        );
        assert!(
            calls[3].contains("@dispatch_pane_content") && calls[3].ends_with("diff"),
            "the pane must be re-marked as running the diff; {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.contains("split-window")),
            "{calls:?}"
        );
        assert_focus_moved_to_the_lower_pane(&runner);
    }

    /// Space closing the last diff, `q` and hiding the tree all empty the
    /// set; the editor must survive each (EditorPaneOutlivesItsTree).
    #[test]
    fn emptying_the_open_set_while_the_editor_shows_does_not_kill_it() {
        let runner = existing_editor_pane(vec![]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, &runner).unwrap();

        assert_no_pane_mutation(&runner);
    }

    /// `v` while the editor shows another file replaces it.
    #[test]
    fn v_while_the_editor_shows_replaces_it_with_the_new_file() {
        let wt = worktree_with_file();
        let runner = existing_editor_pane(vec![
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
            MockProcessRunner::ok(),
        ]);

        show_in_editor("%2", wt.path(), Path::new("a.rs"), &editor(), &runner).unwrap();

        let calls = runner.flattened_calls();
        assert!(
            calls[2].contains("respawn-pane -k")
                && calls[2].ends_with(&format!("vim -p {}", wt.path().join("a.rs").display())),
            "{calls:?}"
        );
        assert_focus_moved_to_the_lower_pane(&runner);
    }

    /// The flag used to live in the renderer's memory: a restarted renderer
    /// read it as "diff" and let the next press take the editor. It is read from
    /// the pane now, so a fresh call (no state carried) still sees the editor.
    #[test]
    fn a_pane_without_a_content_mark_counts_as_the_diff() {
        let runner = pane_running(b"", vec![MockProcessRunner::ok()]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, &runner).unwrap();

        assert_eq!(
            runner.flattened_calls()[2],
            "tmux kill-pane -t %9".to_string()
        );
    }

    #[test]
    fn a_failed_content_lookup_is_an_error_rather_than_a_kill() {
        let runner = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%2 agent_tree\n%9 diff\n"),
            MockProcessRunner::fail("no server running"),
        ]);

        let err =
            reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, &runner).unwrap_err();

        assert!(format!("{err:#}").contains("runs the editor"), "{err:#}");
        assert_no_pane_mutation(&runner);
    }
}
