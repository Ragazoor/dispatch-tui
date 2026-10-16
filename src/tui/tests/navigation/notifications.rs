use super::*;

#[test]
fn toggle_notifications_flips_state() {
    let mut app = make_app();
    assert!(!app.notifications_enabled()); // default: false
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleNotifications,
    ));
    assert!(app.notifications_enabled());
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleNotifications,
    ));
    assert!(!app.notifications_enabled());
}

#[test]
fn refresh_tasks_emits_notification_on_review_transition() {
    let mut app = make_app();
    app.set_notifications_enabled(true);
    // Task 3 starts as Running
    assert_eq!(app.board.tasks[2].status, TaskStatus::Running);

    // Simulate DB refresh where task 3 moved to Review
    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(notif_cmds.len(), 1);
    match &notif_cmds[0] {
        Command::System(crate::tui::commands::SystemCommand::SendNotification {
            title,
            urgent,
            ..
        }) => {
            assert!(title.contains("Task 3"));
            assert!(!urgent);
        }
        _ => unreachable!(),
    }
}

#[test]
fn refresh_tasks_emits_urgent_notification_on_needs_input() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(notif_cmds.len(), 1);
    match &notif_cmds[0] {
        Command::System(crate::tui::commands::SystemCommand::SendNotification {
            urgent, ..
        }) => {
            assert!(urgent);
        }
        _ => unreachable!(),
    }
}

#[test]
fn refresh_tasks_does_not_duplicate_notifications() {
    let mut app = make_app();

    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated.clone(),
    )));
    // Second refresh with same state should not re-notify
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn refresh_tasks_does_not_duplicate_needs_input_notifications() {
    let mut app = make_app();

    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated.clone(),
    )));
    // Second refresh with same state should not re-notify
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn refresh_tasks_renotifies_needs_input_after_clearing() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    // First transition to NeedsInput
    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated.clone(),
    )));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            ))
            .count(),
        1
    );

    // Clear NeedsInput (agent resumes)
    updated[2].sub_status = SubStatus::Active;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated.clone(),
    )));

    // Second transition to NeedsInput should re-notify
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            ))
            .count(),
        1
    );
}

#[test]
fn refresh_tasks_skips_notification_when_disabled() {
    let mut app = make_app();
    // notifications disabled by default — no toggle needed

    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn key_n_uppercase_toggles_notifications() {
    let mut app = make_app();
    assert!(!app.notifications_enabled()); // default: false
    let cmds = app.handle_key(make_key(KeyCode::Char('N')));
    assert!(app.notifications_enabled()); // toggled to enabled
                                          // Should emit PersistSetting command
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Settings(SettingsCommand::PersistSetting { .. }))));
    // Should show status message
    assert!(app.status.message.as_deref().unwrap().contains("enabled"));
}

#[test]
fn refresh_tasks_clears_notified_when_task_leaves_review() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    // Move to review — triggers notification
    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));

    // Move to done — should clear notified state
    let mut updated2 = app.board.tasks.to_vec();
    updated2[2].status = TaskStatus::Done;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated2,
    )));

    // Move back to review — should re-notify
    let mut updated3 = app.board.tasks.to_vec();
    updated3[2].status = TaskStatus::Review;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated3,
    )));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(notif_cmds.len(), 1);
}

#[test]
fn refresh_tasks_clears_notified_state_even_when_disabled() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    // Task transitions to review while notifications enabled — gets notified
    let mut updated = app.board.tasks.to_vec();
    updated[2].status = TaskStatus::Review;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated,
    )));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            ))
            .count(),
        1
    );

    // Disable notifications
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleNotifications,
    ));

    // Task leaves review while disabled
    let mut updated2 = app.board.tasks.to_vec();
    updated2[2].status = TaskStatus::Done;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated2,
    )));

    // Re-enable notifications
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleNotifications,
    ));

    // Task returns to review — should re-notify because notified state was cleared
    let mut updated3 = app.board.tasks.to_vec();
    updated3[2].status = TaskStatus::Review;
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        updated3,
    )));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::System(crate::tui::commands::SystemCommand::SendNotification { .. })
            )
        })
        .collect();
    assert_eq!(
        notif_cmds.len(),
        1,
        "Should re-notify after notified state was cleared while disabled"
    );
}

#[test]
fn summary_row_shows_bell_and_hint_when_notifications_enabled() {
    let mut app = make_app();
    app.set_notifications_enabled(true); // explicitly enable (default is false)
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "\u{1F514}")); // 🔔
    assert!(buffer_contains(&buf, "[N]"));
}

#[test]
fn summary_row_shows_muted_bell_and_hint_when_disabled() {
    let mut app = make_app();
    // notifications disabled by default — no toggle needed
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "\u{1F515}")); // 🔕
    assert!(buffer_contains(&buf, "[N]"));
}

#[test]
fn detail_panel_shows_pr_url() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    // Navigate to Review column (index 2) and open detail panel
    for _ in 0..2 {
        app.update(Message::NavigateColumn(1));
    }
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    let _buf = render_to_buffer(&mut app, 200, 20);
}

#[test]
fn summary_row_shows_filter_indicator() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.filter.repos.insert("/a".to_string());
    app.filter.repos.insert("/b".to_string());

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "2/3 repos"),
        "Expected filter indicator in summary"
    );
}

#[test]
fn summary_row_shows_excl_prefix_in_exclude_mode() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.filter.repos.insert("/a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "excl 1/3 repos"),
        "Expected excl prefix in filter indicator"
    );
}
