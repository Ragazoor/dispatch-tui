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
//! marker, its focus rule and its accepted marker gap — the differences are
//! that the new pane spans the window's full width, and that it is killed when
//! nothing is open. It is also reused to show the editor (`show_in_editor`).

use std::path::Path;

use anyhow::{Context, Result};

use crate::models::TaskId;
use crate::process::ProcessRunner;
use crate::tmux::{self, PANE_ROLE_DIFF, PANE_ROLE_OPTION};

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

/// This agent window's diff pane, if it has one.
///
/// Value-matched, not presence-matched: the tree pane this is called from
/// carries the same option under a different role, and mistaking one for the
/// other would let a close take the tree away.
fn existing_diff_pane(my_pane: &str, runner: &dyn ProcessRunner) -> Result<Option<String>> {
    Ok(
        tmux::pane_ids_with_option_value(my_pane, PANE_ROLE_OPTION, PANE_ROLE_DIFF, runner)
            .context("failed to look for an existing diff pane")?
            .into_iter()
            .next(),
    )
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
/// `my_pane` is the calling renderer's own pane id, used both as the split
/// target and to identify the window to look in — every pane involved is in the
/// agent's own window.
pub fn reconcile_diff_pane(
    my_pane: &str,
    data_dir: &Path,
    task_id: TaskId,
    worktree: &Path,
    anything_open: bool,
    showing_editor: bool,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let existing = existing_diff_pane(my_pane, runner)?;

    match (anything_open, existing) {
        (true, None) => split_diff_pane(my_pane, data_dir, task_id, worktree, runner),
        (false, Some(pane)) => tmux::kill_pane(&pane, runner),
        // The editor is showing and a file was opened: the diff renderer
        // takes the same pane back (`ShowDiffInLowerPane`).
        (true, Some(pane)) if showing_editor => {
            let command = diff_command(data_dir, task_id, runner);
            run_in(worktree, &command, |cwd, argv| {
                tmux::respawn_pane_running(&pane, cwd, argv, runner)
            })
        }
        // Already agreeing: something is open and a pane is showing it, or
        // nothing is open and there is no pane. Both are the resting state.
        (true, Some(_)) | (false, None) => Ok(()),
    }
}

/// Show `relative` — a path below `worktree` — in the user's editor in the
/// lower pane (`OpenAgentTreeFileInEditor`): respawn the pane we created, or
/// split a fresh one when there is none. Never touches a pane we did not
/// create. A path with no working-tree file is refused, because an editor
/// handed a missing path opens a misleading empty buffer.
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

    match existing_diff_pane(my_pane, runner)? {
        Some(pane) => tmux::respawn_pane_running(&pane, &cwd, &command, runner),
        None => {
            let pane = tmux::split_window_below_running(
                my_pane,
                DIFF_PANE_PERCENT,
                &cwd,
                &command,
                runner,
            )?;
            mark_diff_pane(&pane, runner);
            Ok(())
        }
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

fn split_diff_pane(
    my_pane: &str,
    data_dir: &Path,
    task_id: TaskId,
    worktree: &Path,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let command = diff_command(data_dir, task_id, runner);
    let pane = run_in(worktree, &command, |cwd, argv| {
        tmux::split_window_below_running(my_pane, DIFF_PANE_PERCENT, cwd, argv, runner)
    })?;
    mark_diff_pane(&pane, runner);
    Ok(())
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

    /// No pane carries the diff role, then the split and the marker succeed.
    fn no_pane_then_split() -> MockProcessRunner {
        MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%1 \n%2 agent_tree\n"),
            MockProcessRunner::ok_with_stdout(b"%9\n"),
            MockProcessRunner::ok(),
        ])
    }

    /// A diff pane already exists, then whatever the caller does with it.
    fn existing_pane(rest: Vec<anyhow::Result<std::process::Output>>) -> MockProcessRunner {
        let mut queued = vec![MockProcessRunner::ok_with_stdout(
            b"%1 \n%2 agent_tree\n%9 diff\n",
        )];
        queued.extend(rest);
        MockProcessRunner::new(queued)
    }

    #[test]
    fn opening_the_first_diff_splits_a_pane_below_the_tree() {
        let runner = no_pane_then_split();

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner).unwrap();

        let calls = runner.flattened_calls();
        assert!(
            calls[1].contains("split-window") && calls[1].contains("-t %2"),
            "must split from the tree's own pane; got {calls:?}"
        );
    }

    /// The pane must read the SAME database as the tree, so it sees the same
    /// open set and the same task. Inheriting a default that happened to match
    /// would be a coincidence, not a guarantee.
    #[test]
    fn the_diff_pane_is_told_which_database_and_task_to_read() {
        let runner = no_pane_then_split();

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner).unwrap();

        let split = &runner.flattened_calls()[1];
        assert!(split.contains("--data-dir /data"), "got {split}");
        assert!(split.contains("agent-diff 42"), "got {split}");
    }

    #[test]
    fn the_new_pane_is_marked_with_the_diff_role() {
        let runner = no_pane_then_split();

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner).unwrap();

        let mark = &runner.flattened_calls()[2];
        assert!(
            mark.contains("set-option") && mark.contains("%9") && mark.ends_with("diff"),
            "got {mark}"
        );
    }

    /// Splitting again on every toggle would subdivide the column until the
    /// window ran out of room.
    #[test]
    fn opening_a_second_diff_reuses_the_pane_that_is_already_there() {
        let runner = existing_pane(vec![]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner).unwrap();

        assert_eq!(runner.flattened_calls().len(), 1, "lookup only");
    }

    #[test]
    fn closing_the_last_diff_kills_the_pane() {
        let runner = existing_pane(vec![MockProcessRunner::ok()]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, false, &runner).unwrap();

        assert_eq!(
            runner.flattened_calls()[1],
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

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), false, false, &runner).unwrap();

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
        ]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner).unwrap();

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

        let err = reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner)
            .unwrap_err();

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
        ]);

        assert!(
            reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, false, &runner).is_ok()
        );
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
    }

    /// The pane we created is reused, never split again.
    #[test]
    fn v_with_a_lower_pane_respawns_it_with_the_editor() {
        let wt = worktree_with_file();
        let runner = existing_pane(vec![MockProcessRunner::ok()]);

        show_in_editor("%2", wt.path(), Path::new("a.rs"), &editor(), &runner).unwrap();

        let calls = runner.flattened_calls();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(
            calls[1].contains("respawn-pane -k") && calls[1].contains("-t %9"),
            "{calls:?}"
        );
        assert!(calls[1].contains("vim -p"), "{calls:?}");
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

    /// ShowDiffInLowerPane: opening a file while the editor shows puts the diff
    /// renderer back in the same pane.
    #[test]
    fn opening_a_diff_while_the_editor_shows_respawns_the_diff_renderer() {
        let runner = existing_pane(vec![MockProcessRunner::ok()]);

        reconcile_diff_pane("%2", db(), TaskId(42), worktree(), true, true, &runner).unwrap();

        let respawn = &runner.flattened_calls()[1];
        assert!(
            respawn.contains("respawn-pane -k") && respawn.contains("agent-diff 42"),
            "{respawn}"
        );
    }
}
