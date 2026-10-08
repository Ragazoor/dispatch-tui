use super::*;

// ---------------------------------------------------------------------------
// InputWrapUpMode tests
// ---------------------------------------------------------------------------

#[test]
fn submit_base_branch_transitions_to_wrap_up_mode() {
    let mut app = make_app();
    app.input.mode = InputMode::InputBaseBranch;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        repo_path: "/repo".to_string(),
        base_branch: "main".into(),
        ..Default::default()
    });
    app.input.set_buffer("main".to_string());

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));

    assert_eq!(
        app.input.mode,
        InputMode::InputWrapUpMode,
        "expected InputWrapUpMode after submitting base branch, got {:?}",
        app.input.mode
    );
    assert!(
        cmds.is_empty(),
        "no commands should be emitted before wrap-up mode selection"
    );
}

/// A draft parked at the wrap-up step — the shared starting point for every
/// test of the form's tail. Wrap-up is the form's LAST step: answering it is
/// what creates the task. phoenix is decided earlier, at InputTag.
fn app_at_wrap_up_step() -> App {
    let mut app = make_app();
    app.input.mode = InputMode::InputWrapUpMode;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        repo_path: "/repo".to_string(),
        base_branch: "main".into(),
        ..Default::default()
    });
    app
}

/// The draft the form asked to create, or `None` if it emitted no Insert.
fn insert_draft(cmds: &[Command]) -> Option<&TaskDraft> {
    cmds.iter().find_map(|c| match c {
        Command::Task(crate::tui::commands::TaskCommand::Insert { draft, .. }) => Some(draft),
        _ => None,
    })
}

#[test]
fn wrap_up_mode_r_selects_rebase_and_creates_task() {
    let mut app = app_at_wrap_up_step();

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));

    assert_eq!(app.input.mode, InputMode::Normal);
    let draft = insert_draft(&cmds).expect("expected Insert command");
    assert_eq!(
        draft.wrap_up_mode,
        Some(crate::models::WrapUpMode::Rebase),
        "expected Rebase wrap_up_mode"
    );
}

#[test]
fn wrap_up_mode_p_selects_pr_and_creates_task() {
    let mut app = app_at_wrap_up_step();

    let cmds = app.handle_key(make_key(KeyCode::Char('p')));

    assert_eq!(app.input.mode, InputMode::Normal);
    let draft = insert_draft(&cmds).expect("expected Insert command");
    assert_eq!(
        draft.wrap_up_mode,
        Some(crate::models::WrapUpMode::Pr),
        "expected Pr wrap_up_mode"
    );
}

#[test]
fn wrap_up_mode_d_selects_done_and_creates_task() {
    let mut app = app_at_wrap_up_step();

    let cmds = app.handle_key(make_key(KeyCode::Char('d')));

    assert_eq!(app.input.mode, InputMode::Normal);
    let draft = insert_draft(&cmds).expect("expected Insert command");
    assert_eq!(
        draft.wrap_up_mode,
        Some(crate::models::WrapUpMode::Done),
        "expected Done wrap_up_mode"
    );
}

#[test]
fn wrap_up_mode_enter_skips_and_creates_task_with_no_mode() {
    let mut app = app_at_wrap_up_step();

    let cmds = app.handle_key(make_key(KeyCode::Enter));

    assert_eq!(app.input.mode, InputMode::Normal);
    let draft = insert_draft(&cmds).expect("expected Insert command");
    assert_eq!(
        draft.wrap_up_mode, None,
        "Enter should create task with no wrap-up mode"
    );
}

#[test]
fn wrap_up_mode_enter_keeps_prefilled_value_from_copy_task() {
    // Regression guard: CopyTask prefills wrap_up_mode from the source task,
    // but the picker's Enter previously always submitted None, silently
    // clearing it. Enter (no explicit r/p/d pick) must keep whatever the
    // draft already carries.
    let mut app = app_at_wrap_up_step();
    if let Some(draft) = app.input.task_draft.as_mut() {
        draft.title = "Copy of: T".to_string();
        draft.wrap_up_mode = Some(crate::models::WrapUpMode::Pr);
    }

    let cmds = app.handle_key(make_key(KeyCode::Enter));

    let draft = insert_draft(&cmds).expect("expected Insert command");
    assert_eq!(
        draft.wrap_up_mode,
        Some(crate::models::WrapUpMode::Pr),
        "Enter should keep the copied wrap_up_mode, not clear it"
    );
    assert!(
        !draft.phoenix,
        "CopyTask does not carry the flag — the copy answers the tag step fresh"
    );
}

// ---------------------------------------------------------------------------
// Normal-mode handler coverage for extracted methods
// ---------------------------------------------------------------------------

#[test]
fn space_key_on_split_pinned_task_focuses_pane() {
    let task = make_task(4, TaskStatus::Running);
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));
    app.selection_mut().set_column(2);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::FocusPane { pane_id }) if pane_id == "%42"
        )),
        "pinned task should focus the split pane, got {cmds:?}"
    );
}

#[test]
fn space_key_on_epic_enters_epic_view() {
    let mut app = make_app_with_epic_selected();
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        matches!(app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(10)),
        "space on epic should enter ViewMode::Epic, got {:?}",
        app.board.view_mode
    );
}

#[test]
fn space_on_task_in_active_split_swaps_pane() {
    let task = make_task(3, TaskStatus::Running);
    let mut app = App::new(vec![task]);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%10".to_string());
    app.selection_mut().set_column(2);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Split(crate::tui::commands::SplitCommand::Swap { .. })
        )),
        "Space on task with split active should swap pane, got {cmds:?}"
    );
}

#[test]
fn capital_s_is_inert() {
    // [S] retired in favour of Space-in-split-mode: no arm, no hint.
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tmux_window = Some(test_tmux_window("task-1"));
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(1);

    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert!(cmds.is_empty(), "S must emit no commands, got {cmds:?}");
    assert!(
        app.status.message.is_none(),
        "S must show no hint, got {:?}",
        app.status.message
    );
}

#[test]
fn capital_g_on_epic_is_noop() {
    let anchor_task = make_task(1, TaskStatus::Backlog);
    let mut running_blocked = make_task(2, TaskStatus::Running);
    running_blocked.epic_id = Some(EpicId(10));
    running_blocked.sub_status = SubStatus::Stale;
    running_blocked.tmux_window = Some(test_tmux_window("task-blocked"));

    let mut app = App::new(vec![anchor_task, running_blocked]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('G'))));
    assert!(cmds.is_empty(), "G on an epic must emit no commands");
}

#[test]
fn r_key_on_feed_epic_triggers_feed() {
    // task id=1 at row 0, epic id=10 at row 1 in Backlog column
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let mut epic = make_epic(10);
    epic.feed_command = Some("gh api ...".to_string());
    app.board.epics = vec![epic];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1); // cursor on epic at row 1

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('r'))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Feed(crate::tui::commands::FeedCommand::TriggerEpic { epic_id, .. }) if *epic_id == EpicId(10)
        )),
        "r on feed epic should trigger feed, got {cmds:?}"
    );
}

#[test]
fn r_key_inside_epic_view_with_feed_triggers_feed() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let mut epic = make_epic(10);
    epic.feed_command = Some("gh api ...".to_string());
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('r'))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Feed(crate::tui::commands::FeedCommand::TriggerEpic { epic_id, .. }) if *epic_id == EpicId(10)
        )),
        "r inside epic view with feed should trigger feed, got {cmds:?}"
    );
}

#[test]
fn r_key_without_feed_epic_is_noop() {
    let mut app = make_app(); // tasks, no feed epics
    app.selection_mut().set_column(1);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('r'))));
    assert!(cmds.is_empty(), "r without a feed epic should be noop");
}

#[test]
fn capital_d_with_no_repos_opens_picker() {
    // With no saved repos, D should open the QuickDispatch picker so the user
    // can type a new repo path — the old "No saved repo paths" error is gone.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec![];
    app.selection_mut().set_column(1);

    without_usage(app.handle_key(make_key(KeyCode::Char('D'))));
    assert!(
        matches!(app.input.mode, InputMode::QuickDispatch),
        "D with no repos should open the picker, got {:?}",
        app.input.mode
    );
}

#[test]
fn capital_d_with_one_repo_quick_dispatches() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.selection_mut().set_column(1);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('D'))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::QuickDispatch { .. })
        )),
        "D with 1 repo should emit QuickDispatch, got {cmds:?}"
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn capital_d_with_multiple_repos_opens_selection() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.selection_mut().set_column(1);

    without_usage(app.handle_key(make_key(KeyCode::Char('D'))));
    assert_eq!(
        app.input.mode,
        InputMode::QuickDispatch,
        "D with multiple repos should open selection UI"
    );
}

#[test]
fn enter_key_when_on_select_all_deselects_column() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    // Navigate up from row 0 to reach the select-all toggle
    app.update(Message::NavigateRow(-1));
    assert!(app.on_select_all(), "precondition: on_select_all");

    // First, select all
    app.update(Message::SelectAllColumn);
    assert!(!app.select.tasks.is_empty(), "precondition: tasks selected");

    // Enter should deselect
    app.handle_key(make_key(KeyCode::Enter));
    assert!(
        app.select.tasks.is_empty(),
        "Enter on select-all should deselect all"
    );
}

// ── repo-path bug: new path must be selectable even when existing paths fuzzy-match ──

#[test]
fn repo_path_cursor_count_includes_new_path_slot() {
    // When buffer is non-empty and doesn't exactly match an existing path,
    // the cursor range should include a slot for the new path entry.
    // Existing path "/tmp/other" fuzzy-matches "/tmp", so filtered is non-empty,
    // but the new-path slot must still exist.
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("/tmp".to_string());
    app.input.repo_cursor = 0;

    // Down arrow should move to cursor 1 (the new-path slot) because
    // has_new_repo_option("/tmp", ["/tmp/other"]) is true.
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 1);

    // Down again wraps back to 0 (only 2 effective entries).
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn repo_path_enter_at_new_path_slot_submits_typed_value() {
    // With buffer "/tmp" that fuzzy-matches existing "/tmp/other", navigating
    // to the new-path slot (cursor 1) and pressing Enter should submit "/tmp",
    // not "/tmp/other".
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("/tmp".to_string());
    app.input.repo_cursor = 1; // new-path slot

    let _cmds = app.handle_key(make_key(KeyCode::Enter));
    // Should have advanced to InputBaseBranch with "/tmp" as the repo path.
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.task_draft.as_ref().unwrap().repo_path, "/tmp");
}

#[test]
fn quick_dispatch_zero_repos_opens_picker() {
    // With no saved repos, pressing D should open the picker (QuickDispatch mode)
    // so the user can type a new path — not show a "no saved paths" error.
    let mut app = App::new(vec![]);
    // no repo_paths
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('D'))));

    assert!(
        matches!(app.input.mode, InputMode::QuickDispatch),
        "expected QuickDispatch mode after D with no repos, got {:?}",
        app.input.mode
    );
    // No QuickDispatch command yet — just the picker opened.
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::QuickDispatch { .. })
        )),
        "should not emit QuickDispatch command immediately"
    );
}

#[test]
fn quick_dispatch_zero_repos_new_path_entry_accepted() {
    // With no saved repos, the user opens the picker, types "/tmp", and presses
    // Enter — this should emit a QuickDispatch command for the new path.
    let mut app = App::new(vec![]);
    app.handle_key(make_key(KeyCode::Char('D'))); // open picker
    assert!(matches!(app.input.mode, InputMode::QuickDispatch));

    // Type "/tmp" character by character.
    type_text(&mut app, "/tmp");
    assert_eq!(app.input.buffer, "/tmp");

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::QuickDispatch { draft, .. })
                if draft.repo_path == "/tmp"
        )),
        "expected QuickDispatch command with /tmp, got {:?}",
        cmds
    );
}

#[test]
fn slash_enters_search_mode_and_snapshots_query() {
    let mut app = App::new(vec![]);
    app.search.query = "old".to_string();
    app.handle_key(make_key(KeyCode::Char('/')));
    assert_eq!(app.input.mode, InputMode::SearchTasks);
    assert_eq!(app.search.saved, Some("old".to_string()));
}

#[test]
fn typing_in_search_updates_query_live() {
    let mut app = App::new(vec![]);
    app.handle_key(make_key(KeyCode::Char('/')));
    app.handle_key(make_key(KeyCode::Char('a')));
    app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(app.search.query, "ab");
    assert_eq!(app.input.mode, InputMode::SearchTasks);
}

#[test]
fn backspace_in_search_removes_last_char() {
    let mut app = App::new(vec![]);
    app.handle_key(make_key(KeyCode::Char('/')));
    app.handle_key(make_key(KeyCode::Char('a')));
    app.handle_key(make_key(KeyCode::Char('b')));
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.search.query, "a");
}

#[test]
fn enter_commits_search_and_keeps_query() {
    let mut app = App::new(vec![]);
    app.handle_key(make_key(KeyCode::Char('/')));
    app.handle_key(make_key(KeyCode::Char('a')));
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.search.query, "a");
    assert_eq!(app.search.saved, None);
}

#[test]
fn esc_in_search_restores_snapshot() {
    let mut app = App::new(vec![]);
    app.search.query = "old".to_string();
    app.handle_key(make_key(KeyCode::Char('/')));
    app.handle_key(make_key(KeyCode::Char('x')));
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.search.query, "old");
    assert_eq!(app.search.saved, None);
}

#[test]
fn esc_in_normal_clears_active_search() {
    let mut app = App::new(vec![]);
    app.search.query = "active".to_string();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.search.query, "");
}

// -- Phoenix arming at the tag step ---------------------------------------
//
// phoenix has no step of its own. It is armed by `p` at the tag picker, which
// then re-opens the SAME step with `p` dropped from the accepted set so the
// operator picks the task's real tag (CreateTask: PhoenixArming, in
// docs/specs/tasks.allium).

/// An app parked on the tag picker with a filled draft, as the form reaches it
/// after InputTitle.
fn app_on_tag_step() -> App {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Weekly dep audit".to_string(),
        repo_path: "/tmp".to_string(),
        ..Default::default()
    });
    app
}

fn inserted_draft(cmds: &[Command]) -> Option<TaskDraft> {
    cmds.iter().find_map(|c| match c {
        Command::Task(crate::tui::commands::TaskCommand::Insert { draft, .. }) => {
            Some(draft.clone())
        }
        _ => None,
    })
}

fn draft_of(app: &App) -> &TaskDraft {
    app.input.task_draft.as_ref().expect("the form has a draft")
}

/// Wrap-up used to advance to a phoenix step. It is the form's last step now.
#[test]
fn submitting_wrap_up_mode_creates_the_task() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputWrapUpMode;
    app.input.task_draft = Some(TaskDraft::default());

    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitWrapUpMode(None),
    ));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        inserted_draft(&cmds).is_some(),
        "wrap-up is the last step; answering it creates the task"
    );
}

#[test]
fn p_arms_phoenix_and_reopens_the_tag_step() {
    let mut app = app_on_tag_step();

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('p'))));

    assert!(draft_of(&app).phoenix, "p arms the recurrence");
    assert_eq!(
        app.input.mode,
        InputMode::InputTag,
        "the picker re-opens so the real tag can still be picked"
    );
    assert_eq!(draft_of(&app).tag, None, "p is not a tag");
    assert!(
        cmds.is_empty(),
        "arming phoenix advances nothing, so it opens no editor and creates no task"
    );
}

/// The prompt loses `[p]hoenix` on the second pass, and so does the accepted
/// set: a second `p` joins the silently-ignored keys.
#[test]
fn a_second_p_is_ignored_once_phoenix_is_armed() {
    let mut app = app_on_tag_step();
    app.handle_key(make_key(KeyCode::Char('p')));

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('p'))));

    assert_eq!(
        app.input.mode,
        InputMode::InputTag,
        "the second p must leave the step open"
    );
    assert_eq!(draft_of(&app).tag, None);
    assert!(cmds.is_empty(), "the second p must advance nothing");
}

#[test]
fn the_second_pass_picks_a_tag_and_keeps_the_armed_flag() {
    let mut app = app_on_tag_step();
    app.handle_key(make_key(KeyCode::Char('p')));

    app.handle_key(make_key(KeyCode::Char('b')));

    let draft = draft_of(&app);
    assert_eq!(draft.tag, Some(TaskTag::Bug));
    assert!(draft.phoenix, "picking a tag must not disarm phoenix");
    assert_eq!(
        app.input.mode,
        InputMode::InputDescription,
        "the second pass advances the form like the first would have"
    );
}

/// A phoenix task with no tag is legal: Enter on the second pass means "no
/// explicit pick", exactly as it does on the first.
#[test]
fn the_second_pass_accepts_enter_for_no_tag() {
    let mut app = app_on_tag_step();
    app.handle_key(make_key(KeyCode::Char('p')));

    app.handle_key(make_key(KeyCode::Enter));

    let draft = draft_of(&app);
    assert_eq!(draft.tag, None);
    assert!(draft.phoenix);
    assert_eq!(app.input.mode, InputMode::InputDescription);
}

/// Esc is not a step-back that disarms phoenix — the re-opened picker is the
/// same step, so Esc cancels the whole form as it does everywhere else.
#[test]
fn esc_after_arming_phoenix_cancels_the_whole_form() {
    let mut app = app_on_tag_step();
    app.handle_key(make_key(KeyCode::Char('p')));

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Esc)));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        app.input.task_draft.is_none(),
        "Esc discards the draft, armed flag and all"
    );
    assert!(inserted_draft(&cmds).is_none(), "Esc creates nothing");
}

/// EnterKeepsTheDraft (CreateTask in docs/specs/tasks.allium): Enter means "no
/// explicit pick", not "clear the tag". It matters for CopyTask, which seeds
/// the draft's tag from the source task.
#[test]
fn enter_at_the_tag_step_keeps_a_prefilled_tag() {
    let mut app = app_on_tag_step();
    if let Some(draft) = app.input.task_draft.as_mut() {
        draft.title = "Copy of: Weekly dep audit".to_string();
        draft.tag = Some(TaskTag::Chore);
    }

    app.handle_key(make_key(KeyCode::Enter));

    assert_eq!(
        draft_of(&app).tag,
        Some(TaskTag::Chore),
        "Enter must keep the copied tag, not clear it"
    );
}

/// A finished copy must not leave the copy marker set: the next new-task form
/// would then take the tag step's copy branch and skip the description editor.
#[test]
fn finishing_a_copy_clears_the_copy_marker() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('c')));
    app.handle_key(make_key(KeyCode::Enter));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/tmp".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch("main".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitWrapUpMode(None),
    ));
    assert_eq!(app.input.mode, InputMode::Normal, "the copy was created");

    // A fresh new-task form must reach the description editor as usual.
    app.update(Message::Input(
        crate::tui::messages::InputMessage::StartNewTask,
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitTitle("T".to_string()),
    ));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));

    assert_eq!(app.input.mode, InputMode::InputDescription);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Editor(crate::tui::commands::EditorCommand::PopOut(
                EditKind::Description { is_epic: false }
            ))
        )),
        "the new task must still pop the description editor, got {cmds:?}"
    );
}

/// The whole form, end to end, with phoenix armed at the tag step. Guards the
/// flag surviving the four steps that follow it.
#[test]
fn a_draft_armed_at_the_tag_step_is_created_as_a_phoenix() {
    let mut app = app_on_tag_step();

    app.handle_key(make_key(KeyCode::Char('p')));
    app.handle_key(make_key(KeyCode::Char('c')));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitDescription("desc".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/tmp".to_string()),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch("main".to_string()),
    ));
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitWrapUpMode(None),
    ));

    let draft = inserted_draft(&cmds).expect("the form creates the task");
    assert!(draft.phoenix, "the armed flag must survive to creation");
    assert_eq!(draft.tag, Some(TaskTag::Chore));
}
