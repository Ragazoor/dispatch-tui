//! tmux composition for the board's split-pane feature: pulling an agent's
//! window into the board window as a pane ("pin"), and swapping a different
//! task's window into an already-pinned pane.
//!
//! These live here rather than in `src/runtime/split.rs` because they are domain
//! logic — multi-step sequences carrying policy, not raw tmux verbs — while the
//! runtime's job is `spawn_blocking` and message emission (the Message→Command
//! split in docs/architecture.md). And here rather than in `src/tmux.rs` because
//! the swap path calls `resync_agent_tree_pane`; putting them in `tmux.rs` would
//! invert that dependency.
//!
//! Being reachable from tests/tmux_lifecycle.rs falls out of the same placement,
//! and matters because the correctness of these sequences is entirely about
//! tmux's own pane semantics, which the mock layer cannot observe.
//!
//! See docs/specs/split-pane.allium and docs/specs/agent-tree.allium's
//! `ToggleVsSplitPaneInteraction`.

use anyhow::{Context, Result};

use crate::models::TmuxWindow;
use crate::process::ProcessRunner;
use crate::tmux;

use super::agents::companion_pane_ids;

/// Move `window`'s agent pane into `target_pane`'s window as a right-hand pane,
/// discarding every companion pane left behind — the agent-tree pane and any
/// editor pane opened from it.
///
/// Returns the joined pane's id (tmux preserves pane ids across a move).
pub fn join_task_window_into_pane(
    window: &TmuxWindow,
    target_pane: &str,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    // Capture the companion panes *before* the join. `join_pane` moves only the
    // agent's own (active) pane out of the window, so anything left behind is
    // orphaned in a window nothing owns: a lone companion pane is
    // indistinguishable from "hidden" to the agent-tree toggle (see
    // docs/specs/agent-tree.allium: ToggleVsSplitPaneInteraction), and an editor
    // pane has no owner at all once the tree that opened it is gone. Checking
    // after the join would be too late to tell any of them apart.
    //
    // Plural, and not "the window's inactive pane": with an editor pane open
    // that lookup was ambiguous and drained neither.
    let companion_panes = companion_panes_or_none(window, runner);

    let pane_id = tmux::join_pane(window, target_pane, runner)?;
    drain_pinned_from_window(window, &pane_id, companion_panes, runner);
    Ok(pane_id)
}

/// The companion panes of `window`, or none when the check fails.
///
/// Best-effort: a failed check degrades to "no companions" rather than failing
/// the pin or swap, which is the user's actual intent.
fn companion_panes_or_none(window: &TmuxWindow, runner: &dyn ProcessRunner) -> Vec<String> {
    companion_pane_ids(window, runner).unwrap_or_else(|e| {
        tracing::warn!(
            %window,
            error = %e,
            "failed to check for companion panes before moving the agent pane"
        );
        Vec::new()
    })
}

/// Tidy up `window` after `pinned_pane`, its agent, has moved into the split
/// pane: kill the `companions` captured before the move, then tag the window as
/// awaiting `pinned_pane` if a pane dispatch did not mark still keeps it alive.
///
/// The tag is what lets the pane come back to this window later rather than
/// being refused as a duplicate (`restore_pinned_pane`). See PinTaskInSplitPane's
/// "Leftover window" guidance in docs/specs/split-pane.allium. Best-effort
/// throughout: a failure is logged, and an untagged leftover only means a later
/// exit is refused, as it was before tags existed.
fn drain_pinned_from_window(
    window: &TmuxWindow,
    pinned_pane: &str,
    companions: Vec<String>,
    runner: &dyn ProcessRunner,
) {
    for companion_id in companions {
        if let Err(e) = tmux::kill_pane(&companion_id, runner) {
            tracing::warn!(
                %window,
                %companion_id,
                error = %e,
                "failed to kill leftover companion pane after moving the agent pane"
            );
        }
    }

    match tmux::has_window(window, runner) {
        Ok(false) => {}
        Ok(true) => {
            if let Err(e) = tmux::set_window_awaits_pane(window, pinned_pane, runner) {
                tracing::warn!(
                    %window,
                    %pinned_pane,
                    error = %e,
                    "failed to tag the leftover window; exiting the split will refuse to restore the pane"
                );
            }
        }
        Err(e) => tracing::warn!(
            %window,
            error = %e,
            "failed to check for a leftover window after moving the agent pane"
        ),
    }
}

/// Return the pinned `pane_id` to a standalone window named `window`.
///
/// Breaks the pane out into a new window when the name is free. When a window
/// already holds it, the pane rejoins that window only if it is the one the pane
/// was pinned from (tagged by `drain_pinned_from_window`); any other window under
/// the name is a duplicate and the break's refusal is returned unchanged. See
/// ExitSplitMode and RefuseExitSplitModeOntoLiveWindow in
/// docs/specs/split-pane.allium.
pub fn restore_pinned_pane(
    pane_id: &str,
    window: &TmuxWindow,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let refused = match tmux::break_pane_to_window(pane_id, window, runner) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    // A failed lookup reads as "not awaiting": the refusal is then reported,
    // which leaves the pane where it is.
    if tmux::window_awaits_pane(window, pane_id, runner).unwrap_or(false) {
        tmux::rejoin_pane(pane_id, window, runner).context("rejoin pane failed")
    } else {
        Err(refused)
    }
}

/// Swap `new_window`'s agent pane into `right_pane` (the currently pinned pane),
/// then dispose of the standalone window now holding the previous occupant:
/// renamed back to the outgoing task's window when `old_task` is given, killed
/// outright when it is not. When the outgoing task's window is still alive as
/// the leftover it was pinned from, the outgoing pane rejoins it instead and the
/// incoming task's window is drained as a pin drains it. Any other live window
/// under the outgoing name refuses the swap before a pane moves.
///
/// `old_task`, when the outgoing pane belonged to a task, is `(window_name,
/// worktree_path)` — the two travel together because they name the same task
/// and are only ever known or unknown together, never independently; a tuple
/// keeps "window known, worktree unknown" unrepresentable rather than guarded
/// at runtime. The worktree half rewrites the renamed window's `@dispatch_dir`
/// (see the call site below) so the companion resync reads a value that
/// matches the window's new identity rather than the incoming task's, which is
/// what the window option still says immediately after a rename (rename-window
/// does not touch window options).
///
/// Returns the pane id of the task swapped in.
pub fn swap_task_window_into_pane(
    new_window: &TmuxWindow,
    right_pane: &str,
    old_task: Option<(&TmuxWindow, &str)>,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    // 1. Resolve the incoming task's pane before swapping.
    let new_pane_id = tmux::pane_id_for_window(new_window, runner).context("cannot get pane ID")?;

    // 2. Decide where the outgoing pane goes before anything moves, so a
    // refusal fails the swap with nothing rearranged. A live window under the
    // outgoing name is either the leftover the pane was pinned from (rejoin it)
    // or a duplicate (refuse). See SwapSplitPane in docs/specs/split-pane.allium.
    let rejoin = match old_task {
        Some((old_name, _)) => {
            let taken = tmux::has_window(old_name, runner).context("rename window failed")?;
            // A failed tag lookup reads as "not awaiting", which refuses.
            if taken && !tmux::window_awaits_pane(old_name, right_pane, runner).unwrap_or(false) {
                return Err(tmux::duplicate_window_name_error(old_name))
                    .context("rename window failed");
            }
            taken
        }
        None => false,
    };

    // 3. Atomically swap pane contents, addressing the source by pane id.
    //
    // Not `<window>.0`: that form is an *index*, and no pane has index 0 when the
    // user sets `pane-base-index 1` — tmux fails with "can't find pane: 0" and
    // the swap silently does nothing. A `-b` split also renumbers indices, so the
    // index of "the window's own pane" is not stable even at the default base
    // index. The pane id resolved above is exactly the pane meant here and is
    // immune to both. See learning #324 and tests/tmux_lifecycle.rs's
    // `swap_works_when_pane_base_index_is_1`.
    let companions = if rejoin {
        companion_panes_or_none(new_window, runner)
    } else {
        Vec::new()
    };
    tmux::swap_pane(&new_pane_id, right_pane, runner).context("swap pane failed")?;

    // 4. Rejoin, rename or kill the standalone window that now holds the old
    // content.
    match old_task {
        Some((old_name, _)) if rejoin => {
            // The outgoing pane goes home, leaving the incoming task's window
            // with only what sat beside its agent: drain it as a pin would.
            tmux::rejoin_pane(right_pane, old_name, runner).context("rejoin pane failed")?;
            drain_pinned_from_window(new_window, &new_pane_id, companions, runner);
        }
        Some((old_name, old_worktree)) => {
            tmux::rename_window(new_window.as_str(), old_name, runner)
                .context("rename window failed")?;
            // The renamed window's @dispatch_dir still names the incoming
            // task's worktree — a rename does not touch window options, only
            // the window's name. Rewrite it to the outgoing task's worktree
            // before resyncing, so the resync's read of @dispatch_dir (below)
            // reflects the window's new identity instead of racing whatever
            // was there before. Targeted by `old_name` (the window's new
            // name), not `new_pane_id`: swap-pane moves pane *objects* between
            // windows (`new_pane_id` now lives wherever `right_pane` used to
            // be, not in this window any more), so only the window name still
            // identifies the right target here. Best-effort like the resync
            // itself: the swap and rename have already succeeded by this
            // point, so a failure here is logged, never propagated.
            if let Err(e) = tmux::set_window_dispatch_dir(old_name, old_worktree, runner) {
                tracing::warn!(
                    window = %old_name,
                    error = %e,
                    "failed to rewrite @dispatch_dir after swap; companion may resync into the wrong worktree"
                );
            }
            // swap-pane exchanged only the agent panes — the renamed window's
            // companion pane (if any) still renders the previous occupant's
            // tree. Resync it to the task the window's new name implies.
            super::resync_agent_tree_pane(old_name, runner);
        }
        None => {
            tmux::kill_window(new_window, runner).context("kill window failed")?;
        }
    }

    Ok(new_pane_id)
}
