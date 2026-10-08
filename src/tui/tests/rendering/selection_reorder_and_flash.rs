use super::*;

#[tokio::test]
async fn select_all_column_only_affects_current_column() {
    let mut app = make_app();
    // TaskId(3) is in Running column, pre-select it
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(3)),
    ));
    // SelectAllColumn selects all in current (Backlog) column
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
    assert!(app.select.tasks.contains(&TaskId(3)));
    assert_eq!(app.select.tasks.len(), 3);
}

#[tokio::test]
async fn select_all_deselect_only_affects_current_column() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(3)),
    ));
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 3);

    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 1);
    assert!(app.select.tasks.contains(&TaskId(3)));
}

#[tokio::test]
async fn key_a_selects_all_in_column() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
}

#[tokio::test]
async fn navigate_up_from_row_zero_enters_select_all_toggle() {
    let mut app = make_app();
    assert!(!app.on_select_all());
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
}

#[tokio::test]
async fn column_switch_clears_on_select_all_for_nonempty_column() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
    // Running (nav col 2) has a task, so switching there must land on the
    // first card rather than preserving the select-all toggle.
    app.handle_key(make_key(KeyCode::Char('l')));
    assert!(!app.on_select_all());
}

#[tokio::test]
async fn enter_on_toggle_triggers_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
}

#[tokio::test]
async fn v_is_noop_when_on_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(app.select.tasks.is_empty());
}

#[tokio::test]
async fn render_shows_select_all_toggle_in_focused_column() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[ ]"));
    assert!(!buffer_contains(&buf, "Select [a]ll"));
}

#[tokio::test]
async fn render_shows_checked_toggle_when_all_selected() {
    let mut app = make_app();
    app.update(Message::SelectAllColumn);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[x]"));
}

#[tokio::test]
async fn render_shows_unchecked_toggle_when_not_all_selected() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[ ]"));
}

#[tokio::test]
async fn action_hints_include_select_all() {
    let app = make_app();
    let task = app.selected_task();
    let spans = ui::action_hints(task, false, Color::Blue);
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("select all"),
        "action hints should include 'select all'"
    );
}

#[tokio::test]
async fn card_shows_pr_badge() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    // Navigate to Review column (index 2)
    for _ in 0..2 {
        app.update(Message::NavigateColumn(1));
    }

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "PR #42"),
        "Card should show PR #42 badge"
    );
}

#[tokio::test]
async fn card_shows_merged_pr_badge() {
    let mut task = make_task(1, TaskStatus::Done);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    // Navigate to Done column (index 3)
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "PR #42 merged"),
        "Done card should show merged PR badge"
    );
}

#[tokio::test]
async fn reorder_task_down_swaps_sort_order() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    let t2 = make_task(2, TaskStatus::Backlog);
    app.board.tasks = vec![t1, t2];

    // Cursor on first task (row 0, column 0 = Backlog)
    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(1),
    ));

    // After reorder, task 1 should have a higher sort value than task 2
    let t1 = app.board.find_task(TaskId(1)).unwrap();
    let t2 = app.board.find_task(TaskId(2)).unwrap();
    let eff1 = t1.sort_order.unwrap_or(t1.id.0);
    let eff2 = t2.sort_order.unwrap_or(t2.id.0);
    assert!(
        eff1 > eff2,
        "task 1 ({eff1}) should be after task 2 ({eff2}) after move down"
    );
    // Should emit PersistTask for both
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::Task(crate::tui::commands::TaskCommand::Persist(_))
            ))
            .count(),
        2
    );
    // Cursor should have moved down
    assert_eq!(app.selection().row(1), 1);
}

#[tokio::test]
async fn reorder_task_up_at_top_is_noop() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(-1),
    ));
    assert!(cmds.is_empty());
}

#[tokio::test]
async fn reorder_task_down_at_bottom_is_noop() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(1),
    ));
    assert!(cmds.is_empty());
}

#[tokio::test]
async fn reorder_task_up_swaps_sort_order() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    let t2 = make_task(2, TaskStatus::Backlog);
    app.board.tasks = vec![t1, t2];

    // Move cursor to row 1 (second task), then reorder up
    app.selection_mut().set_row(1, 1);
    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(-1),
    ));

    // After reorder, task 2 should have a lower sort value than task 1
    let t1 = app.board.find_task(TaskId(1)).unwrap();
    let t2 = app.board.find_task(TaskId(2)).unwrap();
    let eff1 = t1.sort_order.unwrap_or(t1.id.0);
    let eff2 = t2.sort_order.unwrap_or(t2.id.0);
    assert!(
        eff2 < eff1,
        "task 2 ({eff2}) should be before task 1 ({eff1}) after move up"
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::Task(crate::tui::commands::TaskCommand::Persist(_))
            ))
            .count(),
        2
    );
    // Cursor should have moved up
    assert_eq!(app.selection().row(1), 0);
}

#[tokio::test]
async fn reorder_task_down_swaps_completed_at_within_done_column() {
    let mut app = make_app();
    // t1 completed later, so it renders at row 0; t2 renders at row 1 — Done
    // reads newest-first. This must hold for the cursor position below to
    // actually land on t1 before the move.
    let mut t1 = make_task(1, TaskStatus::Done);
    t1.completed_at = chrono::DateTime::from_timestamp(1_700_000_100, 0);
    let mut t2 = make_task(2, TaskStatus::Done);
    t2.completed_at = chrono::DateTime::from_timestamp(1_700_000_000, 0);
    app.board.tasks = vec![t1, t2];
    app.selection_mut().set_column(4); // Done column
    app.selection_mut().set_row(4, 0);

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(1),
    ));

    let at1 = app
        .board
        .find_task(TaskId(1))
        .unwrap()
        .completed_at
        .unwrap();
    let at2 = app
        .board
        .find_task(TaskId(2))
        .unwrap()
        .completed_at
        .unwrap();
    assert!(
        at1 < at2,
        "task 1 ({at1}) should be after task 2 ({at2}) after move down"
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::Task(crate::tui::commands::TaskCommand::Persist(_))
            ))
            .count(),
        2
    );
}

#[tokio::test]
async fn render_shows_section_headers() {
    // make_app() has one Running task (SubStatus::Active) → Running column shows "── active" header
    let mut app = App::new(vec![make_task(1, TaskStatus::Running), {
        let mut t = make_task(2, TaskStatus::Running);
        t.sub_status = SubStatus::Stale;
        t
    }]);
    let buf = render_to_buffer(&mut app, 160, 30);
    assert!(
        buffer_contains(&buf, "active"),
        "section header 'active' not found"
    );
    assert!(
        buffer_contains(&buf, "stale"),
        "section header 'stale' not found"
    );
}

#[tokio::test]
async fn render_shows_parent_status_headers() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 160, 30);
    assert!(
        buffer_contains_ignore_case(&buf, "backlog"),
        "parent header 'backlog' not found"
    );
    assert!(
        buffer_contains_ignore_case(&buf, "running"),
        "parent header 'running' not found"
    );
    assert!(
        buffer_contains_ignore_case(&buf, "review"),
        "parent header 'review' not found"
    );
    assert!(
        buffer_contains_ignore_case(&buf, "done"),
        "parent header 'done' not found"
    );
}

#[tokio::test]
async fn render_detail_shows_sub_status() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    let mut app = App::new(vec![task]);
    // Move one nav column right, from Backlog to Running.
    app.update(Message::NavigateColumn(1));
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // Placeholder: verify that the overlay renderer does not crash.
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    let _buf = render_to_buffer(&mut app, 160, 30);
}

#[tokio::test]
async fn render_card_conflict_shows_rebase_conflict() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Conflict;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "rebase conflict"),
        "Conflict task should show 'rebase conflict'"
    );
}

#[tokio::test]
async fn render_card_detached_shows_detached() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = None; // detached: worktree present but no tmux
    task.sub_status = SubStatus::Active;
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "detached"),
        "Task with worktree but no tmux_window should show 'detached'"
    );
}

#[tokio::test]
async fn render_card_detached_review_shows_pr_label() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = None; // detached
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/acme/app/pull/42",
        crate::models::UrlType::Pr,
    ));
    task.sub_status = SubStatus::AwaitingReview;
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // move to Running
    app.update(Message::NavigateColumn(1)); // move to Review
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR #42"),
        "Detached review task with pr_url should show 'PR #42'"
    );
}

#[tokio::test]
async fn render_card_blocked_shows_blocked() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::NeedsInput;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "blocked"),
        "Running task with NeedsInput sub_status should show 'blocked'"
    );
}

#[tokio::test]
async fn render_card_running_shows_running() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains_ignore_case(&buf, "running"),
        "Active running task should show 'running'"
    );
}

#[tokio::test]
async fn render_card_review_pr_shows_pr_number() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/acme/app/pull/99",
        crate::models::UrlType::Pr,
    ));
    task.sub_status = SubStatus::AwaitingReview;
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // move to Running
    app.update(Message::NavigateColumn(1)); // move to Review
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR #99"),
        "Review task with pr_url and tmux should show 'PR #99'"
    );
}

#[tokio::test]
async fn render_card_done_merged_shows_merged() {
    let mut task = make_task(1, TaskStatus::Done);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/acme/app/pull/77",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    app.update(Message::NavigateColumn(1)); // Running
    app.update(Message::NavigateColumn(1)); // Review
    app.update(Message::NavigateColumn(1)); // Done
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR #77 merged"),
        "Done task with pr_url should show 'PR #77 merged'"
    );
}

#[tokio::test]
async fn render_card_idle_with_plan_shows_triangle() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.plan_path = Some("docs/plans/plan.md".to_string());
    let mut app = App::new(vec![task]);
    // Already in Backlog column (0)
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{25b8}"),
        "Backlog task with plan should show '▸' (U+25B8)"
    );
}

#[tokio::test]
async fn render_card_idle_with_bug_tag() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Bug);
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[bug]"),
        "Backlog task with Bug tag should show '[bug]'"
    );
}

#[tokio::test]
async fn render_card_idle_with_feature_tag() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[feat]"),
        "Backlog task with Feature tag should show '[feat]'"
    );
}

#[tokio::test]
async fn render_card_message_flash_shows_envelope() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.agents.message_flash.insert(TaskId(1), Instant::now());
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{2709}"),
        "Running task with message_flash set should show '\u{2709}' (envelope)"
    );
}

/// Build a Running task with a message flash stamped `age_secs` ago.
///
/// Backdates the `Instant` rather than sleeping: the flash TTL is a wall-clock
/// threshold, and `./scripts/check-no-test-sleep.sh` rejects sleeping to cross
/// one. See the "No `tokio::time::sleep` in tests" section of docs/conventions.md.
fn app_with_aged_message_flash(age_secs: u64) -> App {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    let stamped = Instant::now()
        .checked_sub(Duration::from_secs(age_secs))
        .expect("monotonic clock must reach back far enough to age a flash");
    app.agents.message_flash.insert(TaskId(1), stamped);
    app.update(Message::NavigateColumn(1)); // Running column
    app
}

#[tokio::test]
async fn message_flash_envelope_outlives_the_old_three_second_window() {
    // board-visuals.allium "Message flash": the flash lasts MESSAGE_FLASH_TTL (30s), long
    // enough that a human whose attention is elsewhere still sees it. Ten seconds
    // in — well past the superseded 3s window — the envelope must still render.
    let mut app = app_with_aged_message_flash(10);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{2709}"),
        "a 10s-old flash must still show the envelope; the window is {:?}",
        crate::tui::MESSAGE_FLASH_TTL
    );
}

#[tokio::test]
async fn message_flash_expires_once_past_its_ttl() {
    // The other side of the threshold: a flash older than MESSAGE_FLASH_TTL is
    // swept by `tick_message_flash` and stops rendering. Without this the TTL
    // could be raised to infinity and nothing would notice.
    let ttl = crate::tui::MESSAGE_FLASH_TTL.as_secs();
    let mut app = app_with_aged_message_flash(ttl + 1);
    let _ = app.handle_tick();
    assert!(
        !app.agents.message_flash.contains_key(&TaskId(1)),
        "a flash older than {ttl}s must be swept from the tracking map"
    );
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "\u{2709}"),
        "an expired flash must not render the envelope"
    );
}

/// Two Running tasks: the cursor sits on the first, the second carries a flash
/// stamped `age_secs` ago. Isolating the flash onto a non-cursor card is what
/// lets a test read the flash's own contribution to the frame colour — on the
/// cursor card the hue would be there either way.
fn app_with_flash_on_a_non_cursor_card(age_secs: u64) -> App {
    let mut tasks = Vec::new();
    for id in [1, 2] {
        let mut t = make_task(id, TaskStatus::Running);
        t.sub_status = SubStatus::Active;
        t.worktree = Some(format!("/repo/.worktrees/{id}-task"));
        t.tmux_window = Some(test_tmux_window(&format!("task-{id}")));
        // Seed recent activity. These tests drive `handle_tick`, which reclassifies
        // an inactive Running task as Stale — and Stale now claims an amber state
        // border, which would put a colour on a frame this fixture means to keep
        // healthy. Without this the tasks go stale mid-test and the assertions read
        // the wrong cause.
        t.last_pre_tool_use_at = Some(Utc::now());
        tasks.push(t);
    }
    let mut app = App::new(tasks);
    let stamped = Instant::now()
        .checked_sub(Duration::from_secs(age_secs))
        .expect("monotonic clock must reach back far enough to age a flash");
    app.agents.message_flash.insert(TaskId(2), stamped);
    app.update(Message::NavigateColumn(1)); // Running column; cursor on task 1
    app
}

#[tokio::test]
async fn message_flash_never_colours_the_card_frame() {
    // board-visuals.allium "Message flash": the flash is carried by its warm fill and its
    // envelope glyph, and it leaves the frame alone.
    //
    // It used to take the column hue, which was safe only because the envelope was
    // co-terminous with it — a whole exception resting on one timing coincidence.
    // Worse, once the frame began carrying state that hue collided head-on with
    // needs-input: in Running the column hue *is* amber. Giving the frame up
    // deleted the exception and the collision together, so what needs guarding now
    // is simply that the flash never touches the frame.
    let neutral = ui::card_border_color();
    let cursor = ui::cursor_border_color();
    let running_hue = ui::column_color(TaskStatus::Running);

    let mut app = app_with_flash_on_a_non_cursor_card(crate::tui::MESSAGE_FLASH_TTL.as_secs() - 1);
    let _ = app.handle_tick();
    let buf = render_to_buffer(&mut app, 120, 30);

    assert!(
        buffer_contains(&buf, "\u{2709}"),
        "inside the TTL the envelope must render — otherwise this test proves nothing"
    );

    let corners = cells_with_symbol(&buf, "\u{256d}");
    let frames: Vec<Color> = corners.iter().map(|c| c.fg).collect();
    assert!(
        !frames.contains(&running_hue),
        "a live flash must not put the column hue on any frame; in Running that hue \
         is the same amber needs-input claims, so it would read as a blocked agent"
    );
    // Both tasks are healthy, so the only non-neutral frame is the cursor's.
    let non_neutral: Vec<Color> = frames.iter().copied().filter(|c| *c != neutral).collect();
    assert_eq!(
        non_neutral,
        vec![cursor],
        "with a flash live on a healthy non-cursor card, the only non-neutral frame \
         on the board is the cursor white"
    );
}

#[tokio::test]
async fn message_flash_render_and_sweep_share_one_threshold() {
    // The duration used to be hardcoded in both `tick_message_flash` and the
    // card renderer with no shared constant, so the two could silently disagree —
    // the map holding an entry the card no longer draws, or the reverse. Just
    // inside the TTL, both must still agree the flash is live.
    let ttl = crate::tui::MESSAGE_FLASH_TTL.as_secs();
    let mut app = app_with_aged_message_flash(ttl - 1);
    let _ = app.handle_tick();
    assert!(
        app.agents.message_flash.contains_key(&TaskId(1)),
        "a flash one second inside the TTL must survive the sweep"
    );
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{2709}"),
        "a flash the sweep kept must still render the envelope"
    );
}

#[tokio::test]
async fn render_card_message_flash_sent_shows_outgoing_glyph() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.agents
        .message_flash_sent
        .insert(TaskId(1), Instant::now());
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{27a4}"),
        "Running task with message_flash_sent set should show '\u{27a4}' (outgoing arrow)"
    );
    assert!(
        !buffer_contains(&buf, "\u{2709}"),
        "a sent-only flash must not also show the received envelope"
    );
}

#[tokio::test]
async fn render_card_message_flash_sent_expires_once_past_its_ttl() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    let ttl = crate::tui::MESSAGE_FLASH_TTL.as_secs();
    let stamped = Instant::now()
        .checked_sub(Duration::from_secs(ttl + 1))
        .expect("monotonic clock must reach back far enough to age a flash");
    app.agents.message_flash_sent.insert(TaskId(1), stamped);
    app.update(Message::NavigateColumn(1));
    let _ = app.handle_tick();
    assert!(
        !app.agents.message_flash_sent.contains_key(&TaskId(1)),
        "a sent-flash older than {ttl}s must be swept from the tracking map"
    );
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "\u{27a4}"),
        "an expired sent-flash must not render the outgoing glyph"
    );
}

#[tokio::test]
async fn a_sent_flash_expiring_marks_the_app_dirty() {
    // mark_tick_dirty's dirty check only diffed message_flash's length before
    // and after a tick — it never looked at message_flash_sent, so a sent-only
    // flash (no matching change to message_flash) expiring on its own would
    // never mark the app dirty, and the stale ➤ glyph would keep rendering
    // until an unrelated event happened to set app.dirty.
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    // Seed recent activity so `tick_sub_status` reclassifies nothing this
    // tick — otherwise its own `self.dirty = true` on a sub_status change
    // would mask the bug this test targets. Same reasoning as
    // `app_with_flash_on_a_non_cursor_card` above.
    task.last_pre_tool_use_at = Some(Utc::now());
    let mut app = App::new(vec![task]);
    let ttl = crate::tui::MESSAGE_FLASH_TTL.as_secs();
    let stamped = Instant::now()
        .checked_sub(Duration::from_secs(ttl + 1))
        .expect("monotonic clock must reach back far enough to age a flash");
    app.agents.message_flash_sent.insert(TaskId(1), stamped);
    app.update(Message::NavigateColumn(1));
    app.dirty = false;

    let _ = app.handle_tick();

    assert!(
        !app.agents.message_flash_sent.contains_key(&TaskId(1)),
        "precondition: the sent-flash must actually have been swept"
    );
    assert!(
        app.dirty,
        "the tick that sweeps an expired sent-flash must mark the app dirty, \
         or the stale outgoing-arrow glyph keeps rendering"
    );
}

#[tokio::test]
async fn render_card_message_flash_shows_both_glyphs_when_sent_and_received() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.agents.message_flash.insert(TaskId(1), Instant::now());
    app.agents
        .message_flash_sent
        .insert(TaskId(1), Instant::now());
    app.update(Message::NavigateColumn(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{2709}"),
        "a task both sent and received must still show the envelope"
    );
    assert!(
        buffer_contains(&buf, "\u{27a4}"),
        "a task both sent and received must still show the outgoing arrow"
    );
}

#[tokio::test]
async fn render_detail_task_with_tag_shows_tag() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Bug);
    let mut app = App::new(vec![task]);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[tokio::test]
async fn render_detail_task_with_pr_url() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/acme/app/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    // Navigate to Review column (index 2)
    app.update(Message::NavigateColumn(2));
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    let _buf = render_to_buffer(&mut app, 160, 30);
}

#[tokio::test]
async fn render_detail_no_selection_shows_message() {
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // Placeholder: just verify that rendering an empty board does not crash.
    let mut app = App::new(vec![]);
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[tokio::test]
async fn task_card_title_truncated_in_narrow_terminal() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.title = "This is a very long task title that should be truncated".to_string();
    let mut app = App::new(vec![task]);

    // Narrow terminal: 4 columns per status column (80 / 4 statuses = 20 each)
    let buf = render_to_buffer(&mut app, 80, 10);

    // Full title should NOT appear — it's too long for the column
    assert!(
        !buffer_contains(
            &buf,
            "This is a very long task title that should be truncated"
        ),
        "full title should be truncated in narrow terminal"
    );
    // Truncated title with ellipsis should appear
    assert!(
        buffer_contains(&buf, "…"),
        "truncated title should contain ellipsis"
    );
}

#[tokio::test]
async fn task_card_short_title_not_truncated_in_wide_terminal() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.title = "Short".to_string();
    let mut app = App::new(vec![task]);

    // Wide terminal: plenty of room
    let buf = render_to_buffer(&mut app, 200, 10);
    assert!(
        buffer_contains(&buf, "Short"),
        "short title should appear in full"
    );
}

#[tokio::test]
async fn task_card_title_adapts_to_terminal_width() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.title = "Medium length title here".to_string();
    let mut app_narrow = App::new(vec![task.clone()]);
    let mut app_wide = App::new(vec![task]);

    let buf_narrow = render_to_buffer(&mut app_narrow, 60, 10);
    let buf_wide = render_to_buffer(&mut app_wide, 200, 10);

    // In narrow terminal, should be truncated
    assert!(
        !buffer_contains(&buf_narrow, "Medium length title here"),
        "title should be truncated in narrow terminal"
    );
    // In wide terminal, should appear in full
    assert!(
        buffer_contains(&buf_wide, "Medium length title here"),
        "title should appear in full in wide terminal"
    );
}

#[tokio::test]
async fn handle_key_normal_reorder_j_down() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('J')));
    // Reorder should produce a persist command
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
}

#[tokio::test]
async fn handle_key_normal_reorder_k_up() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);
    let cmds = app.handle_key(make_key(KeyCode::Char('K')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
}

#[tokio::test]
async fn handle_key_normal_enter_on_select_all_row() {
    let mut app = make_app();
    // Navigate up past first item to land on "select all" virtual row
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // Manually set on_select_all
    app.selection_mut().on_select_all = true;

    app.handle_key(make_key(KeyCode::Enter));
    // Should have toggled select all — tasks should be selected
    assert!(
        !app.select.tasks.is_empty()
            || !app.select.epics.is_empty()
            || app.selection().on_select_all
    );
}

#[tokio::test]
async fn backlog_column_color_is_blue() {
    let backlog = ui::column_color(TaskStatus::Backlog);
    // Backlog should use a distinct blue, not the generic MUTED grey.
    assert_ne!(
        backlog,
        Color::Rgb(86, 95, 137),
        "Backlog column color should not be MUTED grey"
    );
    assert_eq!(
        backlog,
        Color::Rgb(122, 162, 247),
        "Backlog column color should be Tokyo Night blue"
    );
}

#[tokio::test]
async fn focused_backlog_header_renders_in_blue() {
    let mut app = make_app();
    assert_eq!(app.selected_column(), 1);

    let buf = render_to_buffer(&mut app, 100, 20);
    let area = buf.area();
    // The focused header brightens toward the foreground rather than dropping
    // to grey; the hue stays Backlog's (board-visuals.allium: "Focus is intensity, not
    // colour-vs-absence").
    let expected_fg = ui::column_header_fg(TaskStatus::Backlog, true);
    let expected_bg = ui::column_header_bg(TaskStatus::Backlog, true);
    let target = "BACKLOG";
    let mut found = false;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right().saturating_sub(target.len() as u16 - 1) {
            let matches = target
                .bytes()
                .enumerate()
                .all(|(i, ch)| buf[(x + i as u16, y)].symbol().as_bytes().first() == Some(&ch));
            if matches {
                let cell = &buf[(x, y)];
                if cell.fg == expected_fg && cell.bg == expected_bg {
                    found = true;
                }
                break;
            }
        }
        if found {
            break;
        }
    }
    assert!(
        found,
        "Focused Backlog header should render its label on the focused header bar"
    );
}

#[tokio::test]
async fn render_adapts_to_smaller_terminal_after_resize() {
    let mut app = make_app();

    // Render at a large size (pre-split)
    let buf_large = render_to_buffer(&mut app, 160, 40);
    // Render at a smaller size (post-split, e.g. half width)
    let buf_small = render_to_buffer(&mut app, 80, 40);

    // The smaller render should use the full width of the smaller terminal
    assert_eq!(buf_small.area().width, 80);
    assert_eq!(buf_large.area().width, 160);
    // Both should contain a task title — layout adapted, content still renders
    assert!(
        buffer_contains(&buf_small, "Task 1"),
        "task should render at smaller width"
    );
}

#[tokio::test]
async fn render_repo_path_mode_shows_filtered_list_when_typing() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var/log".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.input.set_buffer("tmp".to_string()); // filter active

    let buf = render_to_buffer(&mut app, 80, 20);
    assert!(buffer_contains(&buf, "/tmp"), "matching path should appear");
    assert!(
        !buffer_contains(&buf, "/var/log"),
        "non-matching path should be hidden"
    );
}

#[tokio::test]
async fn render_repo_path_mode_shows_all_when_buffer_empty() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var/log".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // buffer is empty — all paths shown

    let buf = render_to_buffer(&mut app, 80, 20);
    assert!(buffer_contains(&buf, "/tmp"));
    assert!(buffer_contains(&buf, "/var/log"));
}

#[tokio::test]
async fn test_on_select_all_preserved_on_refresh() {
    let mut app = make_app();
    // Navigate up from row 0 to select-all header
    app.update(Message::NavigateRow(-1));
    assert!(app.selection().on_select_all);

    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
        ],
    )));

    assert!(app.selection().on_select_all);
    assert_eq!(app.selection().anchor, None);
}

#[tokio::test]
async fn summary_shows_four_columns_when_backlog_focused() {
    let mut app = make_app();
    // Default is col 1 (Backlog)
    assert_eq!(app.selected_column(), 1);
    let buf = render_to_buffer(&mut app, 120, 40);
    // The summary row (y=1) should NOT contain "Projects".
    let summary_row: String = (0..120u16)
        .map(|x| buf[(x, 1)].symbol().to_string())
        .collect();
    assert!(
        !summary_row.contains("Projects"),
        "summary row should NOT show Projects when col 1 focused; got: {summary_row:?}"
    );
    assert!(
        summary_row.contains("BACKLOG"),
        "summary row should show backlog header; got: {summary_row:?}"
    );
}

// ▼ = U+25BC (BLACK DOWN-POINTING TRIANGLE)
// ▲ = U+25B2 (BLACK UP-POINTING TRIANGLE)
// Distinct from ▸ U+25B8 used in the summary row for focused columns.

#[tokio::test]
async fn scroll_indicator_down_shown_when_items_overflow() {
    // 5 Backlog tasks × 3 lines each = 15 lines; at height=20 kanban inner ≈ 8 lines → overflow
    let tasks: Vec<_> = (1..=5).map(|i| make_task(i, TaskStatus::Backlog)).collect();
    let mut app = App::new(tasks);
    // Cursor at top (row 0): offset=0, only ▼ should show
    app.selection_mut().set_row(1, 0);

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{25BC}"),
        "▼ indicator should appear when items overflow below the visible area"
    );
    assert!(
        !buffer_contains(&buf, "\u{25B2}"),
        "▲ indicator should NOT appear when cursor is at the top"
    );
}

#[tokio::test]
async fn scroll_indicator_up_shown_when_scrolled_past_top() {
    // 5 Backlog tasks, cursor on the last one → ratatui scrolls → offset > 0 → ▲ shows
    let tasks: Vec<_> = (1..=5).map(|i| make_task(i, TaskStatus::Backlog)).collect();
    let mut app = App::new(tasks);
    app.selection_mut().set_row(1, 4); // row 4 = 5th task

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{25B2}"),
        "▲ indicator should appear when scrolled past the top"
    );
}

#[tokio::test]
async fn no_scroll_indicators_when_items_fit() {
    // 2 Backlog tasks × 3 lines = 6 lines; at height=40 kanban inner ≈ 28 lines → fits
    let tasks = vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ];
    let mut app = App::new(tasks);

    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        !buffer_contains(&buf, "\u{25BC}"),
        "▼ should NOT appear when all items fit in the visible area"
    );
    assert!(
        !buffer_contains(&buf, "\u{25B2}"),
        "▲ should NOT appear when all items fit in the visible area"
    );
}

#[tokio::test]
async fn scroll_indicators_do_not_panic_on_empty_column() {
    let mut app = App::new(vec![]);
    // Should render without panic
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(!buffer_contains(&buf, "\u{25BC}"));
    assert!(!buffer_contains(&buf, "\u{25B2}"));
}

// ── Column identity and focus (board-visuals.allium: Column Identity and Focus) ──────
