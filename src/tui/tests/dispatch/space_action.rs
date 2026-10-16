use super::*;

// ---------------------------------------------------------------------------
// Unified Space "activate task" behavior (docs/specs/split-pane.allium:
// JumpToAgentWindow). Space jumps to a live window when one exists; otherwise
// it dispatches / resumes / opens the retry dialog. `d` is no longer bound.
// The window-present cases already jump under the pre-existing Space handler,
// so the behavioral change is confined to the no-window branches below.
// ---------------------------------------------------------------------------

#[test]
fn space_on_backlog_no_window_dispatches() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(1); // Backlog column
    app.selection_mut().set_row(1, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    // Space on a windowless Backlog task routes through the repo-trust gate
    // (a CheckTrustAndDispatch command) rather than dispatching or showing
    // "No active session" synchronously.
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::CheckTrustAndDispatch { id: TaskId(1), .. })
        )),
        "expected CheckTrustAndDispatch command, got {cmds:?}"
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn space_on_running_no_window_with_worktree_resumes() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column
    app.selection_mut().set_row(2, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Resume { .. }))),
        "Space on a windowless Running task with a worktree should resume, got {cmds:?}"
    );
}

#[test]
fn space_on_stale_running_no_window_enters_retry_mode() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = None;
    task.sub_status = SubStatus::Stale;
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char(' ')));
    // Windowless Stale/Crashed is the only path that still opens the retry dialog.
    assert!(
        matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))),
        "expected ConfirmRetry mode, got {:?}",
        app.input.mode
    );
}

#[test]
fn space_on_crashed_running_no_window_enters_retry_mode() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = None;
    task.sub_status = SubStatus::Crashed;
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))),
        "expected ConfirmRetry mode, got {:?}",
        app.input.mode
    );
}

/// `@guarantee RetryReachableInPlace` in docs/specs/dispatch.allium — an
/// unprovisioned Running task offers kill-and-retry whatever its sub_status,
/// rather than a dead-end hint. The fixture's sub_status is the default
/// `Active`: the stale/crashed tick classifications both skip windowless
/// tasks, so such a task never reaches either.
#[test]
fn space_on_unprovisioned_running_enters_retry_mode() {
    let task = make_unprovisioned_task(4, TaskStatus::Running);
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(2); // Running column
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))),
        "expected ConfirmRetry mode, got {:?}",
        app.input.mode
    );
}

/// `@guarantee DispatchingOutranksIt` — the claim writes Running before the
/// worktree exists, so an in-flight task is unprovisioned by construction.
/// Offering retry there would move it back to Backlog and fire a SECOND
/// DispatchAgent alongside the one already running.
#[test]
fn space_on_dispatching_unprovisioned_running_does_not_offer_retry() {
    let task = make_unprovisioned_task(4, TaskStatus::Running);
    let mut app = App::new(vec![task]);
    app.mark_dispatching(TaskId(4));
    app.selection_mut().set_column(2); // Running column
    app.selection_mut().set_row(2, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        !matches!(app.input.mode, InputMode::ConfirmRetry(_)),
        "must not open the retry dialog mid-dispatch, got {:?}",
        app.input.mode
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::DispatchAgent { .. }))),
        "must not fire a second dispatch, got {cmds:?}"
    );
    assert!(
        app.status
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Dispatch in progress"),
        "expected an in-progress hint, got {:?}",
        app.status.message
    );
}

/// The epic auto-dispatch chain claims its subtask inside the MCP handler, so
/// it is never in `app.dispatching` — the map alone would leave a chained
/// subtask one keypress from a duplicate agent for its whole provisioning
/// window. A fresh claim stamp closes that hole.
#[test]
fn space_on_freshly_claimed_task_outside_dispatching_map_does_not_offer_retry() {
    let mut task = make_unprovisioned_task(4, TaskStatus::Running);
    task.last_pre_tool_use_at = Some(chrono::Utc::now() - chrono::Duration::seconds(5));
    let mut app = App::new(vec![task]);
    assert!(!app.is_dispatching(TaskId(4)), "not in the map");
    app.selection_mut().set_column(2); // Running column
    app.selection_mut().set_row(2, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        !matches!(app.input.mode, InputMode::ConfirmRetry(_)),
        "must not open the retry dialog while provisioning, got {:?}",
        app.input.mode
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::DispatchAgent { .. }))),
        "must not fire a second dispatch, got {cmds:?}"
    );
}

/// The hint bar must agree with the key: no `[Space] retry` while in flight.
#[test]
fn action_hints_hide_retry_while_dispatch_in_flight() {
    let task = make_unprovisioned_task(4, TaskStatus::Running);
    let hints =
        crate::tui::ui::action_hints(Some(&task), true, ratatui::style::Color::Rgb(122, 162, 247));
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(!text.contains("retry"), "got {text:?}");
}

/// The dialog's `[r] Resume` branch dead-ends for an unprovisioned task (there
/// is no worktree to resume into), so it must not be advertised.
#[test]
fn retry_dialog_for_unprovisioned_offers_fresh_start_only() {
    let task = make_unprovisioned_task(4, TaskStatus::Running);
    let mut app = App::new(vec![task]);
    app.update(Message::Task(TaskMessage::KillAndRetry(TaskId(4))));
    let msg = app.status.message.as_deref().unwrap_or("");
    assert!(msg.contains("[f] Fresh start"), "got {msg:?}");
    assert!(!msg.contains("[r] Resume"), "got {msg:?}");
    assert!(
        !msg.contains("stale") && !msg.contains("crashed"),
        "an unprovisioned task is neither stale nor crashed, got {msg:?}"
    );
}

/// The widening is scoped to Running: `RetryFresh` refuses every other status,
/// so a Review task keeps the hint rather than getting a dialog that no-ops.
#[test]
fn space_on_review_no_window_no_worktree_shows_no_worktree_hint() {
    let task = make_unprovisioned_task(5, TaskStatus::Review);
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(3); // Review column
    app.selection_mut().set_row(3, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(cmds.is_empty());
    assert!(
        !matches!(app.input.mode, InputMode::ConfirmRetry(_)),
        "Review has no retry path, got {:?}",
        app.input.mode
    );
    assert!(
        app.status
            .message
            .as_deref()
            .unwrap_or("")
            .contains("No worktree"),
        "expected a 'No worktree' hint, got {:?}",
        app.status.message
    );
}

// Window-present cases: Space jumps regardless of status or sub_status. The
// window check wins over the Stale/Crashed problematic check.

#[test]
fn space_on_stale_running_with_window_jumps() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::JumpToTmux { window }) if window == "task-4"
        )),
        "Space on a Stale task with a window should jump, not open retry, got {cmds:?}"
    );
    assert!(
        !matches!(app.input.mode, InputMode::ConfirmRetry(_)),
        "Space on a windowed Stale task must not open the retry dialog"
    );
}

#[test]
fn space_on_crashed_running_with_window_jumps() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::JumpToTmux { window }) if window == "task-4"
        )),
        "Space on a Crashed task with a window should jump, got {cmds:?}"
    );
    assert!(!matches!(app.input.mode, InputMode::ConfirmRetry(_)));
}

#[test]
fn space_on_review_with_window_jumps() {
    let mut app = App::new(vec![make_task(5, TaskStatus::Review)]);
    app.selection_mut().set_column(3); // Review column
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::JumpToTmux { window }) if window == "task-5"
        )),
        "Space on a Review task with a window should jump, got {cmds:?}"
    );
}

#[test]
fn space_on_done_with_window_jumps() {
    let mut task = make_task(1, TaskStatus::Done);
    task.tmux_window = Some(test_tmux_window("task-1"));
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(4); // Done column
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::JumpToTmux { window }) if window == "task-1"
        )),
        "Space on a Done task with a window should jump, got {cmds:?}"
    );
}

#[test]
fn space_on_review_no_window_with_worktree_resumes() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(3); // Review column
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Resume { .. }))),
        "Space on a windowless Review task with a worktree should resume, got {cmds:?}"
    );
}

#[test]
fn space_on_done_no_window_with_worktree_resumes() {
    let mut task = make_task(1, TaskStatus::Done);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(4); // Done column
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Resume { .. }))),
        "Space on a windowless Done task with a worktree should resume, got {cmds:?}"
    );
}

#[test]
fn space_on_empty_column_is_noop() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(1);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(cmds.is_empty());
}
