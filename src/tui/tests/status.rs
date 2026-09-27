#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{test_tmux_window, SubStatus, TaskId, TaskStatus};
use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

#[test]
fn resumed_sets_success_status_message() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task]);

    app.update(Message::Task(crate::tui::messages::TaskMessage::Resumed {
        id: TaskId(4),
        tmux_window: test_tmux_window("win-4"),
    }));

    assert_eq!(app.status.message.as_deref(), Some("Task 4 resumed"),);
}

#[test]
fn batch_move_multiple_steps() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    // Move Backlog -> Running (clears selection)
    app.handle_key(make_key(KeyCode::Char('L')));

    // Re-select and move Running -> Review
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));
    app.handle_key(make_key(KeyCode::Char('L')));

    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Review);
    assert_eq!(app.find_task(TaskId(2)).unwrap().status, TaskStatus::Review);
}

#[test]
fn render_status_bar_shows_keybindings() {
    let mut app = App::new(vec![]);
    let buf = render_to_buffer(&mut app, 200, 20);
    assert!(buffer_contains(&buf, "[n]"));
}

#[test]
fn render_status_bar_uses_bracket_format() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let buf = render_to_buffer(&mut app, 220, 20);
    // Hints should use [key] bracket format
    assert!(
        buffer_contains(&buf, "[n]"),
        "status bar should use [key] bracket format"
    );
    assert!(
        buffer_contains(&buf, "[n]ew"),
        "status bar should show 'new' hint"
    );
}

#[test]
fn status_message_clears_after_timeout_on_tick() {
    let mut app = make_app();
    // Simulate a status message that was set 6 seconds ago
    app.status.message = Some("Task 1 finished".to_string());
    app.status.message_set_at = Some(Instant::now() - Duration::from_secs(6));

    // Tick should clear it since it's past the 5-second timeout
    app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(
        app.status.message.is_none(),
        "status_message should auto-clear after timeout"
    );
}

#[test]
fn status_message_persists_before_timeout() {
    let mut app = make_app();
    // Set a message just now
    app.status.message = Some("Task 1 finished".to_string());
    app.status.message_set_at = Some(Instant::now());

    // Tick should NOT clear it since timeout hasn't elapsed
    app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert_eq!(app.status.message.as_deref(), Some("Task 1 finished"));
}

#[test]
fn status_message_does_not_clear_during_interactive_mode() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteTask(TaskId(1));
    app.status.message = Some("Delete task? [y/n]".to_string());
    app.status.message_set_at = Some(Instant::now() - Duration::from_secs(10));

    // Tick should NOT clear it during an interactive mode
    app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(
        app.status.message.is_some(),
        "should not clear during interactive mode"
    );
}

#[test]
fn notifications_disabled_by_default() {
    let app = make_app();
    assert!(!app.notifications_enabled());
}

#[test]
fn render_input_form_shows_during_input_tag() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "My task".to_string(),
        ..Default::default()
    });

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Task"),
        "form overlay title should be visible"
    );
    assert!(
        buffer_contains(&buf, "My task"),
        "draft title should be shown as completed"
    );
    assert!(
        buffer_contains(&buf, "[b]ug"),
        "tag options should be visible"
    );
}

#[test]
fn render_status_bar_input_title() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTitle;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating task: enter title"),
        "InputTitle mode should show 'Creating task: enter title'"
    );
}

#[test]
fn render_status_bar_input_description() {
    let mut app = make_app();
    app.input.mode = InputMode::InputDescription;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating task: opening $EDITOR for description"),
        "InputDescription mode should show 'Creating task: opening $EDITOR for description'"
    );
}

#[test]
fn render_status_bar_input_repo_path() {
    let mut app = make_app();
    app.input.mode = InputMode::InputRepoPath;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating task: enter repo path"),
        "InputRepoPath mode should show 'Creating task: enter repo path'"
    );
}

#[test]
fn render_status_bar_input_tag() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Tag:"),
        "InputTag mode should show 'Tag:'"
    );
}

#[test]
fn render_status_bar_confirm_retry() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmRetry(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Resume"),
        "ConfirmRetry should show 'Resume'"
    );
    assert!(
        buffer_contains(&buf, "Fresh start"),
        "ConfirmRetry should show 'Fresh start'"
    );
}

#[test]
fn render_status_bar_confirm_detach_tmux() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDetachTmux(vec![TaskId(1)]);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Detach tmux"),
        "ConfirmDetachTmux should show 'Detach tmux'"
    );
}

#[test]
fn render_status_bar_help_mode() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[Esc] to close help"),
        "Help mode should show '[Esc] to close help'"
    );
}

#[test]
fn render_status_bar_quick_dispatch() {
    let mut app = make_app();
    app.input.mode = InputMode::QuickDispatch;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Quick dispatch"),
        "QuickDispatch mode should show 'Quick dispatch'"
    );
}

#[test]
fn render_status_bar_status_message_overrides() {
    let mut app = make_app();
    app.status.message = Some("Custom status message".to_string());
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Custom status message"),
        "status_message should override normal status bar text"
    );
}

#[test]
fn render_input_form_title_shows_new_task_block() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("My new task".to_string());
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Task"),
        "block title 'New Task' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Title:"),
        "'Title:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "My new task"),
        "buffer text 'My new task' should be visible"
    );
}

#[test]
fn render_input_form_description_shows_completed_title() {
    let mut app = make_app();
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "Draft title".to_string(),
        ..Default::default()
    });
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Draft title"),
        "completed title 'Draft title' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Description: opening $EDITOR"),
        "'Description: opening $EDITOR...' should be visible"
    );
}

#[test]
fn render_input_form_base_branch_shows_prompt() {
    let mut app = make_app();
    app.input.mode = InputMode::InputBaseBranch;
    app.input.task_draft = Some(TaskDraft {
        title: "My task".to_string(),
        description: "Desc".to_string(),
        repo_path: "/tmp".to_string(),
        base_branch: "main".into(),
        ..Default::default()
    });
    app.input.set_buffer("main".to_string());
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Base branch:"),
        "'Base branch:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "main"),
        "pre-filled branch 'main' should be visible"
    );
}

#[test]
fn render_input_form_repo_path_shows_repo_list() {
    let mut app = make_app();
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test task".to_string(),
        description: "Test desc".to_string(),
        ..Default::default()
    });
    app.input.set_buffer(String::new());
    app.board.repo_paths = vec!["/repo/alpha".to_string(), "/repo/beta".to_string()];
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Repo path:"),
        "'Repo path:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "/repo/alpha"),
        "first repo path '/repo/alpha' should be listed"
    );
    assert!(
        buffer_contains(&buf, "/repo/beta"),
        "second repo path '/repo/beta' should be listed"
    );
}

#[test]
fn render_input_form_quick_dispatch_shows_repo_selection() {
    let mut app = make_app();
    app.input.mode = InputMode::QuickDispatch;
    app.board.repo_paths = vec!["/repo/one".to_string(), "/repo/two".to_string()];
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Quick Dispatch"),
        "block title 'Quick Dispatch' should be visible"
    );
    assert!(
        buffer_contains(&buf, "/repo/one"),
        "first repo path '/repo/one' should be visible"
    );
}

#[test]
fn render_input_form_confirm_retry_shows_options() {
    let mut app = make_app();
    // Replace task 5 as a crashed Running task with worktree and tmux
    let crashed_task = Task {
        title: "Crashed task".to_string(),
        worktree: Some("/tmp/wt".to_string()),
        tmux_window: Some(test_tmux_window("win5")),
        sub_status: SubStatus::Crashed,
        ..make_task(5, TaskStatus::Running)
    };
    app.board.tasks.push(crashed_task);
    app.input.mode = InputMode::ConfirmRetry(TaskId(5));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Retry Agent"),
        "block title 'Retry Agent' should be visible"
    );
    assert!(
        buffer_contains(&buf, "crashed"),
        "'crashed' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "Resume"),
        "'Resume' option should be visible"
    );
    assert!(
        buffer_contains(&buf, "Fresh start"),
        "'Fresh start' option should be visible"
    );
}

#[test]
fn render_tab_bar_board_mode_has_no_tasks_label() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        !buffer_contains(&buf, "Tasks"),
        "tab bar should not show a 'Tasks' tab label"
    );
}

#[test]
fn status_bar_key_color_is_consistent_across_columns() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Running),
        make_task(3, TaskStatus::Review),
        make_task(4, TaskStatus::Done),
    ]);

    let width = 160;
    let height = 30;
    let status_row = height - 1;

    // Collect the key color from the status bar for each column
    let mut colors = Vec::new();
    for _ in 0..4 {
        let buf = render_to_buffer(&mut app, width, status_row + 1);
        if let Some(color) = first_bracket_fg(&buf, status_row) {
            colors.push(color);
        }
        // Move to next column
        app.handle_key(make_key(KeyCode::Right));
    }

    assert!(
        colors.len() >= 2,
        "should have rendered hints in at least 2 columns"
    );
    let first = colors[0];
    for (i, color) in colors.iter().enumerate() {
        assert_eq!(
            *color, first,
            "column {i} key color {color:?} differs from column 0 color {first:?}"
        );
    }
}

#[test]
fn terminal_resized_returns_no_commands() {
    let mut app = make_app();
    let cmds = app.update(Message::System(
        crate::tui::messages::SystemMessage::TerminalResized,
    ));
    assert!(
        cmds.is_empty(),
        "resize should produce no commands, just trigger a re-draw"
    );
}

// ---------------------------------------------------------------------------
// Sticky dispatching status (Task #500)
// ---------------------------------------------------------------------------

#[test]
fn set_status_sticky_overrides_existing_status() {
    let mut app = make_app();
    app.set_status("plain message".to_string());
    assert!(!app.status.message_sticky);

    app.set_status_sticky("sticky message".to_string());
    assert_eq!(app.status.message.as_deref(), Some("sticky message"));
    assert!(app.status.message_sticky);
}

#[test]
fn clear_status_resets_message_sticky() {
    let mut app = make_app();
    app.set_status_sticky("sticky".to_string());
    assert!(app.status.message_sticky);

    app.clear_status();
    assert!(app.status.message.is_none());
    assert!(!app.status.message_sticky);
}

#[test]
fn dispatching_status_message_set_on_mark_dispatching() {
    let mut app = make_app(); // task 1 has title "Task 1"
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));

    let msg = app.status.message.as_deref().expect("status set");
    assert!(msg.contains("Task 1"), "got: {msg}");
    assert!(msg.contains("Dispatching"), "got: {msg}");
    assert!(app.status.message_sticky);
}

#[test]
fn dispatching_status_does_not_expire_on_tick() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    // Backdate so a non-sticky message would auto-clear
    app.status.message_set_at = Some(Instant::now() - Duration::from_secs(10));

    app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(
        app.status.message.is_some(),
        "sticky dispatching status should survive Tick"
    );
}

#[test]
fn dispatching_status_cleared_on_dispatched() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    assert!(app.status.message.is_some());

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::Dispatched {
            id: TaskId(1),
            worktree: "/wt".to_string(),
            tmux_window: test_tmux_window("win-1"),
            switch_focus: false,
        },
    ));

    assert!(app.status.message.is_none());
    assert!(!app.status.message_sticky);
}

#[test]
fn dispatching_status_cleared_on_dispatch_failed() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    assert!(app.status.message.is_some());

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::DispatchFailed(TaskId(1)),
    ));

    assert!(app.status.message.is_none());
    assert!(!app.status.message_sticky);
}

#[test]
fn dispatching_status_pluralizes_when_multiple() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(2)),
    ));

    let msg = app.status.message.as_deref().expect("status set");
    assert!(msg.contains("2 tasks"), "got: {msg}");
    assert!(msg.contains("Dispatching"), "got: {msg}");
}

#[test]
fn dispatching_status_persists_when_one_completes() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(2)),
    ));
    assert!(app.status.message.as_deref().unwrap().contains("2 tasks"));

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::Dispatched {
            id: TaskId(1),
            worktree: "/wt".to_string(),
            tmux_window: test_tmux_window("win-1"),
            switch_focus: false,
        },
    ));

    let msg = app.status.message.as_deref().expect("still set");
    assert!(msg.contains("Task 2"), "got: {msg}");
    assert!(app.status.message_sticky);
}

#[test]
fn dispatching_status_handles_empty_title() {
    let mut app = make_app();
    if let Some(t) = app.find_task_mut(TaskId(1)) {
        t.title = "   ".to_string();
    }

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));

    let msg = app.status.message.as_deref().expect("status set");
    assert!(msg.contains("#1"), "expected ID fallback, got: {msg}");
    assert!(msg.contains("Dispatching"), "got: {msg}");
}

#[test]
fn dispatching_status_skips_deleted_task() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    // Task is deleted while dispatching is in flight.
    app.board.tasks.retain(|t| t.id != TaskId(1));

    app.update(Message::System(crate::tui::messages::SystemMessage::Tick));

    assert!(
        !app.dispatching.contains_key(&TaskId(1)),
        "Tick should drop dispatching IDs that no longer exist in tasks"
    );
    assert!(
        app.status.message.is_none(),
        "with no remaining dispatching IDs, sticky status should clear"
    );
}

#[test]
fn mark_dispatching_for_unknown_task_id_is_noop() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(9999)),
    ));

    // Unknown ID should not pollute the dispatching set with a phantom entry
    // that the next Tick would have to clean up.
    assert!(
        !app.dispatching.contains_key(&TaskId(9999)),
        "MarkDispatching for unknown task should be a no-op"
    );
    assert!(app.status.message.is_none());
}

mod property_tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum DispatchOp {
        Mark(i64),
        Done(i64),
        Failed(i64),
    }

    fn op_strategy() -> impl Strategy<Value = DispatchOp> {
        prop_oneof![
            (1i64..=4).prop_map(DispatchOp::Mark),
            (1i64..=4).prop_map(DispatchOp::Done),
            (1i64..=4).prop_map(DispatchOp::Failed),
        ]
    }

    proptest! {
        #[test]
        fn sticky_status_iff_dispatching_nonempty(
            ops in proptest::collection::vec(op_strategy(), 0..30)
        ) {
            let mut app = make_app();
            for op in ops {
                match op {
                    DispatchOp::Mark(id) => {
                        app.update(Message::Task(crate::tui::messages::TaskMessage::MarkDispatching(TaskId(id))));
                    }
                    DispatchOp::Done(id) => {
                        app.update(Message::Task(crate::tui::messages::TaskMessage::Dispatched {
                            id: TaskId(id),
                            worktree: format!("/wt/{id}"),
                            tmux_window: test_tmux_window(&format!("win-{id}")),
                            switch_focus: false,
                        }));
                    }
                    DispatchOp::Failed(id) => {
                        app.update(Message::Task(crate::tui::messages::TaskMessage::DispatchFailed(TaskId(id))));
                    }
                }
            }

            // Invariant: status is sticky exactly when dispatching is non-empty
            prop_assert_eq!(
                app.dispatching.is_empty(),
                !app.status.message_sticky,
                "dispatching set membership and sticky-flag must agree"
            );
            // When sticky, a message is present; when not, nothing claims to be sticky
            if app.status.message_sticky {
                prop_assert!(app.status.message.is_some());
            }
        }
    }
}
