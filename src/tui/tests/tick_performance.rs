//! Tests for per-tick performance improvements:
//! - Batch tmux window check (one fork per tick instead of N)
//! - Skip cache invalidation when refresh tasks are unchanged
//! - Batch sub-status writes (one BatchPatchSubStatus instead of N Persist)

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::helpers::{make_app, make_task};
use crate::models::{SubStatus, TaskId, TaskStatus};
use crate::tui::messages::{SystemMessage, TaskMessage};
use crate::tui::types::Message;

// ---------------------------------------------------------------------------
// Batch window checks
// ---------------------------------------------------------------------------

#[test]
fn handle_tick_emits_one_batch_window_check_for_multiple_windowed_tasks() {
    let mut app = make_app();
    // Give three tasks tmux windows.
    for task in app.board.tasks.iter_mut().take(3) {
        task.tmux_window = Some(format!("win-{}", task.id.0));
    }

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let batch_count = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                crate::tui::Command::Task(
                    crate::tui::commands::TaskCommand::BatchCheckWindows { .. }
                )
            )
        })
        .count();

    assert_eq!(
        batch_count, 1,
        "exactly one BatchCheckWindows command per tick, got {batch_count}"
    );

    let individual_count = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                crate::tui::Command::Task(crate::tui::commands::TaskCommand::CheckWindow { .. })
            )
        })
        .count();

    assert_eq!(
        individual_count, 0,
        "no individual CheckWindow commands should be emitted when BatchCheckWindows is used"
    );
}

#[test]
fn handle_tick_emits_no_batch_check_when_no_tasks_have_windows() {
    let mut app = make_app();
    // Ensure no tasks have tmux windows.
    for task in app.board.tasks.iter_mut() {
        task.tmux_window = None;
    }

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let batch_count = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                crate::tui::Command::Task(
                    crate::tui::commands::TaskCommand::BatchCheckWindows { .. }
                )
            )
        })
        .count();

    assert_eq!(
        batch_count, 0,
        "no BatchCheckWindows command when no tasks have windows"
    );
}

#[test]
fn handle_tick_batch_window_check_contains_all_windowed_tasks() {
    let mut app = make_app();
    let mut windowed_ids = Vec::new();
    for task in app.board.tasks.iter_mut().take(3) {
        task.tmux_window = Some(format!("win-{}", task.id.0));
        windowed_ids.push(task.id);
    }

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let batch_cmd = cmds.iter().find(|c| {
        matches!(
            c,
            crate::tui::Command::Task(crate::tui::commands::TaskCommand::BatchCheckWindows { .. })
        )
    });

    let Some(crate::tui::Command::Task(crate::tui::commands::TaskCommand::BatchCheckWindows {
        windows,
    })) = batch_cmd
    else {
        panic!("expected BatchCheckWindows command");
    };

    let batch_ids: Vec<TaskId> = windows.iter().map(|(id, _)| *id).collect();
    for id in windowed_ids {
        assert!(
            batch_ids.contains(&id),
            "task {id:?} should be in the batch window check"
        );
    }
}

// ---------------------------------------------------------------------------
// Skip refresh when tasks unchanged
// ---------------------------------------------------------------------------

#[test]
fn handle_refresh_tasks_unchanged_does_not_replace_board_tasks() {
    // The optimization: when the loaded tasks are identical to board.tasks
    // (same IDs, same max updated_at), skip the reassignment entirely.
    // Observable: board.tasks Vec buffer pointer stays the same.
    let mut app = make_app();
    let original_ptr = app.board.tasks.as_ptr();

    // Refresh with an identical clone — same length, same IDs, same timestamps.
    let same_tasks = app.board.tasks.clone();
    app.update(Message::Task(TaskMessage::Refresh(same_tasks)));

    assert_eq!(
        app.board.tasks.as_ptr(),
        original_ptr,
        "board.tasks buffer must not be reallocated on an unchanged refresh"
    );
}

#[test]
fn handle_refresh_tasks_changed_does_replace_board_tasks() {
    let mut app = make_app();
    let original_ptr = app.board.tasks.as_ptr();

    // Add a new task so the refresh is genuinely different.
    let mut new_tasks = app.board.tasks.clone();
    new_tasks.push(make_task(99, TaskStatus::Running));
    app.update(Message::Task(TaskMessage::Refresh(new_tasks)));

    assert_ne!(
        app.board.tasks.as_ptr(),
        original_ptr,
        "board.tasks buffer must be replaced when tasks change"
    );
}

// ---------------------------------------------------------------------------
// Batch sub-status writes
// ---------------------------------------------------------------------------

/// When multiple running tasks need their sub_status reclassified on the same
/// tick, the tick should emit exactly one `BatchPatchSubStatus` command (not N
/// individual `Persist` commands) for efficiency.
#[test]
fn tick_emits_single_batch_patch_sub_status_for_multiple_updates() {
    let mut app = make_app();

    // Make two running tasks with stale last_pre_tool_use_at so their
    // sub_status will be reclassified to Stale on the next tick.
    let stale_time = chrono::Utc::now() - chrono::Duration::hours(2);
    for task in app.board.tasks.iter_mut().filter(|t| t.status == TaskStatus::Running) {
        task.tmux_window = Some("win".into());
        task.last_pre_tool_use_at = Some(stale_time);
        task.sub_status = SubStatus::Active; // will be reclassified to Stale
    }

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let batch_count = cmds.iter().filter(|c| {
        matches!(
            c,
            crate::tui::Command::Task(crate::tui::commands::TaskCommand::BatchPatchSubStatus { .. })
        )
    }).count();

    let persist_sub_status_count = cmds.iter().filter(|c| {
        matches!(c, crate::tui::Command::Task(crate::tui::commands::TaskCommand::Persist(_)))
    }).count();

    assert_eq!(
        batch_count, 1,
        "tick should emit exactly one BatchPatchSubStatus, got {batch_count}"
    );
    assert_eq!(
        persist_sub_status_count, 0,
        "tick must not emit Persist for sub-status reclassifications (use BatchPatchSubStatus)"
    );
}

/// When no running tasks need reclassification, no `BatchPatchSubStatus` is emitted.
#[test]
fn tick_emits_no_batch_patch_sub_status_when_no_sub_status_changes() {
    let mut app = make_app();
    // Running task already active with a recent timestamp — no reclassification needed.
    for task in app.board.tasks.iter_mut().filter(|t| t.status == TaskStatus::Running) {
        task.tmux_window = Some("win".into());
        task.last_pre_tool_use_at = Some(chrono::Utc::now());
        task.sub_status = SubStatus::Active;
    }

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let batch_count = cmds.iter().filter(|c| {
        matches!(
            c,
            crate::tui::Command::Task(crate::tui::commands::TaskCommand::BatchPatchSubStatus { .. })
        )
    }).count();

    assert_eq!(
        batch_count, 0,
        "tick must not emit BatchPatchSubStatus when no sub_status changes are needed"
    );
}

/// The `BatchPatchSubStatus` command must carry all pending updates.
#[test]
fn tick_batch_patch_sub_status_contains_all_pending_updates() {
    let mut app = make_app();
    let stale_time = chrono::Utc::now() - chrono::Duration::hours(2);

    // Give all running tasks a stale timestamp so they get reclassified.
    let mut reclassified_ids = Vec::new();
    for task in app.board.tasks.iter_mut().filter(|t| t.status == TaskStatus::Running) {
        task.tmux_window = Some("win".into());
        task.last_pre_tool_use_at = Some(stale_time);
        task.sub_status = SubStatus::Active;
        reclassified_ids.push(task.id);
    }

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let batch_cmd = cmds.iter().find(|c| {
        matches!(
            c,
            crate::tui::Command::Task(crate::tui::commands::TaskCommand::BatchPatchSubStatus { .. })
        )
    });

    let Some(crate::tui::Command::Task(crate::tui::commands::TaskCommand::BatchPatchSubStatus {
        updates,
    })) = batch_cmd
    else {
        panic!("expected BatchPatchSubStatus command");
    };

    let batch_ids: Vec<TaskId> = updates.iter().map(|(id, _)| *id).collect();
    for id in &reclassified_ids {
        assert!(
            batch_ids.contains(id),
            "task {id:?} must be in BatchPatchSubStatus updates"
        );
    }
}
