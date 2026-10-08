//! The board's event loop: one [`LoopEvent`] per input source, applied to
//! `App`, then the resulting commands drained through `commands::dispatch`.

use anyhow::Result;
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{commands, TuiRuntime};
use crate::tui::{self, App, Command, Message};

/// Minimum time between rendered frames (~60 fps cap).  Rapid key-repeat events
/// that arrive faster than this are processed but coalesced into a single render.
pub(super) const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

// ---------------------------------------------------------------------------
// run_loop — select over key events, async messages, and tick timer
// ---------------------------------------------------------------------------

/// One input the TUI event loop reacts to, drawn from any of its four sources
/// (keys, async messages, MCP notifications, the periodic tick). Naming the
/// event explicitly lets the loop body — `apply_loop_event` — be unit-tested
/// without a running `select!` or a real terminal.
///
/// `Message` is the largest variant and stays unboxed here deliberately: it is
/// already passed by value throughout the TUI (it flows through an
/// `UnboundedReceiver<Message>` unboxed), so boxing at this hop would add an
/// allocation per event for no benefit. That only stays affordable while
/// `Message` itself stays small — see the `size_of` guard-rail tests in
/// `src/tui/types.rs`.
#[cfg_attr(test, derive(Debug))]
pub(super) enum LoopEvent {
    Key(crossterm::event::KeyEvent),
    Message(Message),
    Mcp(crate::board_event::BoardEvent),
    Tick,
}

/// Await the next event from any input source. The tick arm is always enabled,
/// so this never resolves on an all-channels-closed condition — the loop exits
/// via `App::should_quit`, not channel closure.
pub(super) async fn next_loop_event(
    key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    msg_rx: &mut mpsc::UnboundedReceiver<Message>,
    mcp_notify_rx: &mut mpsc::UnboundedReceiver<crate::board_event::BoardEvent>,
    tick_interval: &mut tokio::time::Interval,
) -> LoopEvent {
    tokio::select! {
        // Key events from the blocking poll thread.
        Some(key) = key_rx.recv() => LoopEvent::Key(key),
        // Async messages (e.g., from dispatch results).
        Some(msg) = msg_rx.recv() => LoopEvent::Message(msg),
        // MCP event notification.
        Some(event) = mcp_notify_rx.recv() => LoopEvent::Mcp(event),
        // Periodic tick for tmux capture and feed polling.
        _ = tick_interval.tick() => LoopEvent::Tick,
    }
}

/// Apply one loop event to `app`, returning the commands it produced. Mirrors
/// the per-arm `dirty` bookkeeping and MCP-event side effects (refresh spawns,
/// feed-cache invalidation) of the original `select!` body, kept as a separate
/// function so the routing is directly testable.
pub(super) fn apply_loop_event(app: &mut App, event: LoopEvent, rt: &TuiRuntime) -> Vec<Command> {
    match event {
        // handle_key sets app.dirty unconditionally, same as the Message/Mcp
        // arms below — see the render-dirty-flag section of docs/architecture.md.
        LoopEvent::Key(key) => app.handle_key(key),
        LoopEvent::Message(msg) => {
            // Async messages typically carry visible state changes.
            app.dirty = true;
            app.update(msg)
        }
        LoopEvent::Mcp(event) => {
            // Spawn DB work so this never blocks key-event processing. Results
            // arrive back via msg_rx and are applied on the next iteration.
            app.dirty = true;
            apply_board_event(app, event, rt)
        }
        // Handlers set app.dirty themselves when they detect visible changes.
        LoopEvent::Tick => app.update(Message::System(crate::tui::messages::SystemMessage::Tick)),
    }
}

/// The `LoopEvent::Mcp` arm of [`apply_loop_event`].
fn apply_board_event(
    app: &mut App,
    event: crate::board_event::BoardEvent,
    rt: &TuiRuntime,
) -> Vec<Command> {
    use crate::board_event::BoardEvent;
    match event {
        BoardEvent::Refresh => {
            // A broad refresh may follow a managed-feed config save
            // (set_managed_feed_config) that enabled a feed on a
            // previously feed-less instance. Invalidate the FeedRunner
            // cache so the next tick re-queries for feed commands and
            // starts polling the freshly-provisioned epics rather than
            // short-circuiting on a stale any_feed_cmds == Some(false).
            rt.invalidate_feed_cache();
            drop(rt.spawn_refresh_from_db());
            vec![]
        }
        BoardEvent::TaskChanged(task_id) => {
            drop(rt.spawn_refresh_task(task_id));
            vec![]
        }
        BoardEvent::EpicChanged(epic_id) => {
            // Invalidate the FeedRunner's cache so the next tick re-queries
            // for feed commands (e.g. a newly added feed_command becomes visible).
            rt.invalidate_feed_cache();
            drop(rt.spawn_refresh_epic(epic_id));
            vec![]
        }
        BoardEvent::BranchRebased { repo_path } => {
            // A rebase wrap-up pulled origin/<base> and fast-forwarded
            // local <base>, so the refs are current and no fetch is
            // needed. An unresolved repository measures nothing.
            if !repo_path.is_empty() {
                drop(rt.exec_refresh_repo_sync(repo_path, false));
            }
            vec![]
        }
        BoardEvent::AgentLaunched { repo_path } => {
            // RefreshRepoSyncStateAfterDispatch: provisioning the agent's
            // worktree already fetched origin/<base>, so this is a local
            // ref read at no network cost. The board's own dispatch takes
            // the same refresh through a command; these are the off-board
            // launches (dispatch_task, epic auto-dispatch chaining).
            drop(rt.exec_refresh_repo_sync(repo_path, false));
            vec![]
        }
        BoardEvent::AutoDispatchFailed {
            task_id,
            epic_id,
            reason,
        } => {
            // No refresh is spawned here: the chain sends TaskChanged
            // for the released subtask right behind this, so reloading
            // the row is already covered.
            app.update(Message::Task(
                crate::tui::messages::TaskMessage::AutoDispatchFailed {
                    task_id,
                    epic_id,
                    reason,
                },
            ))
        }
    }
}

/// Commands executed once, before the event loop's first iteration.
///
/// The list exists so "what runs at startup" is one value a test can read,
/// rather than a sequence of inline calls in `run_loop`. Everything here must be
/// something the tick loop would do anyway, just sooner: startup priming, never
/// startup-only behaviour.
pub(super) fn startup_commands() -> Vec<Command> {
    // Read the budget snapshot now instead of waiting out the first
    // BUDGET_POLL_TICKS, so a snapshot already on disk shows on the first frame.
    vec![Command::Budget(
        crate::tui::commands::BudgetCommand::Refresh,
    )]
}

pub(super) async fn run_loop<B: Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
    msg_rx: &mut mpsc::UnboundedReceiver<Message>,
    mcp_notify_rx: &mut mpsc::UnboundedReceiver<crate::board_event::BoardEvent>,
    tick_interval: &mut tokio::time::Interval,
    rt: &mut TuiRuntime,
) -> Result<()> {
    // Here (not in TuiRuntime::new) so tests that construct TuiRuntime directly
    // don't accidentally spawn background tasks. The invalidation sender is held
    // on `rt.feed_invalidate_tx` (cloned at construction), so it survives the
    // runner being moved into its background task here.
    if let Some(feed_runner) = rt.feed_runner.take() {
        feed_runner.start();
    }

    execute_commands(app, startup_commands(), rt, terminal, key_rx).await?;

    let mut last_render = std::time::Instant::now() - MIN_FRAME_INTERVAL; // allow first frame

    loop {
        // Redraw only when state changed since the last frame AND the frame interval has elapsed.
        // frame_ready coalesces rapid key-repeat events (holding j) into at most ~60 renders/s.
        if frame_ready(last_render.elapsed(), app.dirty) {
            terminal.draw(|frame| tui::ui::render(frame, app))?;
            app.dirty = false;
            last_render = std::time::Instant::now();
        }

        if app.should_quit() {
            break;
        }

        let event = next_loop_event(key_rx, msg_rx, mcp_notify_rx, tick_interval).await;
        let commands = apply_loop_event(app, event, rt);

        execute_commands(app, commands, rt, terminal, key_rx).await?;
    }

    Ok(())
}

/// Wait for the split-pane restores a quit issued, bounded.
///
/// Implements `QuitCompletesWithNothingToRestore`,
/// `QuitCompletesWhenRestoreSettles` and `QuitCompletesOnRestoreTimeout` in
/// `docs/specs/split-pane.allium`. Returns whether every restore reported back.
///
/// One bound covers the whole set, not one each: it is a deadline on the quit,
/// and a per-handle budget would multiply by however many exits overlapped. The
/// bound is a parameter so the abandonment path is testable without a
/// wall-clock wait.
pub(super) async fn await_split_restores(
    handles: Vec<tokio::task::JoinHandle<()>>,
    bound: Duration,
) -> bool {
    let settled = tokio::time::timeout(bound, async {
        for handle in handles {
            let _ = handle.await;
        }
    })
    .await;
    if settled.is_err() {
        // Logged rather than shown: the event loop has returned, so nothing
        // would draw a status message even though the board is still on screen.
        tracing::warn!(
            timeout_secs = bound.as_secs_f32(),
            "split-pane restore did not report back before quitting; a pinned agent's pane may still be inside the board's tmux window"
        );
    }
    settled.is_ok()
}

// ---------------------------------------------------------------------------
// execute_commands — run side effects for each Command
// ---------------------------------------------------------------------------

pub(super) async fn execute_commands<B: Backend>(
    app: &mut App,
    cmds: Vec<Command>,
    rt: &TuiRuntime,
    _terminal: &mut Terminal<B>,
    _key_rx: &mut mpsc::UnboundedReceiver<crossterm::event::KeyEvent>,
) -> Result<()> {
    let mut queue = std::collections::VecDeque::from(cmds);
    while let Some(command) = queue.pop_front() {
        let extra = commands::dispatch(command, app, rt).await;
        queue.extend(extra);
    }
    Ok(())
}

/// Returns `true` when the render loop should draw a new frame.
///
/// Both conditions must hold: the app state changed (`dirty`) *and* enough
/// time has elapsed since the last render (`elapsed >= MIN_FRAME_INTERVAL`).
/// The interval coalesces rapid key-repeat events (≥30/s) into at most
/// one render per 16 ms (~60 fps) without adding perceptible latency to
/// single keypresses.
pub(super) fn frame_ready(elapsed_since_render: Duration, dirty: bool) -> bool {
    dirty && elapsed_since_render >= MIN_FRAME_INTERVAL
}
