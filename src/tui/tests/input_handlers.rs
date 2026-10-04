use super::*;
use crate::models::{test_tmux_window, EpicId, SubStatus, TaskId, TaskStatus, TaskTag};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn task_created_adds_to_list() {
    let task = Task {
        title: "New Task".to_string(),
        description: "desc".to_string(),
        ..make_task(42, TaskStatus::Backlog)
    };
    let mut app = App::new(vec![]);
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Created {
        task: Box::new(task),
    }));
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(42));
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
    assert!(cmds.is_empty());
}

#[test]
fn repo_path_empty_uses_saved_path() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string()];

    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();

    let cmds = without_branch_probe(without_usage(app.handle_key(make_key(KeyCode::Enter))));
    // Now advances to InputBaseBranch with "main" pre-filled
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
    // Submitting base branch goes to wrap-up mode
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch("main".to_string()),
    ));
    assert_eq!(app.input.mode, InputMode::InputWrapUpMode);
    // Wrap-up is the form's last step: answering it creates the task.
    let cmds3 = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitWrapUpMode(None),
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds3.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Insert { ref draft, .. }) if draft.repo_path == "/tmp"
    )));
}

#[test]
fn repo_path_empty_no_saved_stays_in_mode() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec![]; // no saved paths

    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();

    let key = make_key(KeyCode::Enter);
    let _cmds = app.handle_key(key);

    // Should stay in InputRepoPath mode
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert!(app.status.message.is_some());
    assert_eq!(app.board.tasks.len(), 0); // no task created
}

#[test]
fn repo_path_nonexistent_shows_error() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/nonexistent/path".to_string()),
    ));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
    let msg = app.status.message.as_ref().unwrap().as_str();
    assert!(msg.contains("does not exist"), "got: {msg}");
}

#[test]
fn repo_path_nonempty_used_as_is() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string()];

    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.set_buffer("/tmp".to_string());

    // Submitting repo path now advances to InputBaseBranch
    let cmds = without_branch_probe(without_usage(
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    ));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
    // Submitting base branch goes to wrap-up mode
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch("main".to_string()),
    ));
    assert_eq!(app.input.mode, InputMode::InputWrapUpMode);
    // Wrap-up is the form's last step: answering it creates the task.
    let cmds3 = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitWrapUpMode(None),
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds3
        .iter()
        .any(|c| matches!(c, Command::Task(crate::tui::commands::TaskCommand::Insert { ref draft, .. }) if draft.repo_path == "/tmp")));
    assert_eq!(app.board.tasks.len(), 0); // task not added until TaskCreated
}

#[test]
fn task_edited_updates_fields() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.update(Message::Task(crate::tui::messages::TaskMessage::Edited(
        TaskEdit {
            id: TaskId(1),
            title: "New".into(),
            description: "Desc".into(),
            repo_path: "/new".into(),
            status: TaskStatus::Running,
            plan_path: Some("docs/plan.md".into()),
            tag: None,
            base_branch: None,
            wrap_up_mode: None,
            url: None,
            phoenix: true,
        },
    )));
    assert_eq!(app.board.tasks[0].title, "New");
    assert_eq!(app.board.tasks[0].description, "Desc");
    assert_eq!(app.board.tasks[0].repo_path, "/new");
    assert_eq!(app.board.tasks[0].status, TaskStatus::Running);
    assert_eq!(
        app.board.tasks[0].plan_path.as_deref(),
        Some("docs/plan.md")
    );
}

#[test]
fn repo_paths_updated_replaces_paths() {
    let mut app = App::new(vec![]);
    app.update(Message::RepoPathsUpdated(vec!["/a".into(), "/b".into()]));
    assert_eq!(app.board.repo_paths, vec!["/a", "/b"]);
}

#[test]
fn n_key_enters_title_mode() {
    let mut app = make_app();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputTitle);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert_eq!(app.status.message.as_deref(), Some("Enter title: "));
}

#[test]
fn backspace_pops_from_input_buffer() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("abc".to_string());
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn backspace_on_empty_buffer_is_noop() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Backspace));
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn enter_with_title_advances_to_tag() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("My Task".to_string());
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputTag);
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.task_draft.as_ref().unwrap().title, "My Task");
    assert_eq!(
        app.status.message.as_deref(),
        Some("Tag: [b]ug  [f]eature  [c]hore  pr-re[v]iew  [r]esearch  fi[x]  [p]hoenix  [Enter] none")
    );
}

#[test]
fn enter_with_empty_title_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn enter_with_whitespace_only_title_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("   ".to_string());
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
}

#[test]
fn enter_in_description_advances_to_repo_path() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("some desc".to_string());
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert!(app.input.buffer.is_empty());
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "some desc"
    );
    assert_eq!(app.status.message.as_deref(), Some("Enter repo path: "));
}

#[test]
fn number_key_out_of_range_appends_to_buffer() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer.clear();
    app.board.repo_paths = vec!["/repo1".to_string()]; // only 1 path
    app.handle_key(make_key(KeyCode::Char('5')));
    assert_eq!(app.input.buffer, "5");
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
}

#[test]
fn number_key_with_nonempty_buffer_appends() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("/my".to_string());
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.handle_key(make_key(KeyCode::Char('1')));
    assert_eq!(app.input.buffer, "/my1");
}

#[test]
fn zero_key_in_repo_path_appends_to_buffer() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer.clear();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.handle_key(make_key(KeyCode::Char('0')));
    assert_eq!(app.input.buffer, "0");
}

#[test]
fn escape_from_title_mode_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("partial".to_string());
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn escape_from_description_mode_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("partial".to_string());
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn escape_from_repo_path_mode_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.set_buffer("/partial".to_string());
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn confirm_delete_y_deletes_task() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDeleteTask(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(1))); // task 1 deleted
    assert!(matches!(
        &cmds[0],
        Command::Task(crate::tui::commands::TaskCommand::Delete(TaskId(1)))
    ));
    assert!(app.status.message.is_none());
}

#[test]
fn confirm_delete_uppercase_y_deletes_task() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDeleteTask(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(1)));
    assert!(matches!(
        &cmds[0],
        Command::Task(crate::tui::commands::TaskCommand::Delete(TaskId(1)))
    ));
}

#[test]
fn confirm_delete_n_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDeleteTask(TaskId(1));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks.len(), 4);
    assert!(cmds.is_empty());
    assert!(app.status.message.is_none());
}

#[test]
fn confirm_delete_esc_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDeleteTask(TaskId(1));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Esc)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks.len(), 4);
    assert!(cmds.is_empty());
}

#[test]
fn x_key_on_empty_column_is_noop() {
    let mut app = make_app();
    app.selection_mut().set_column(3); // Review column is empty
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::Normal); // did NOT enter a delete confirmation
}

#[test]
fn open_close_task_detail_via_messages() {
    let mut app = App::new(vec![]);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::CloseDetail,
    ));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn enter_key_on_empty_board_is_noop() {
    let mut app = App::new(vec![]);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn e_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
}

#[test]
fn e_key_directly_emits_edit_task() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selection_mut().set_column(1);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('e'))));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Editor(crate::tui::commands::EditorCommand::PopOut(EditKind::TaskEdit(t))) if t.id == TaskId(1))
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_retry_r_key_emits_resume() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("task-4"));
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Resume { .. })
    )));
}

#[test]
fn confirm_retry_f_key_emits_fresh() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("task-4"));
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::DispatchAgent { .. })
    )));
}

#[test]
fn confirm_retry_esc_returns_to_normal() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Esc)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty());
}

#[test]
fn start_new_task_enters_title_mode() {
    let mut app = make_app();
    app.update(Message::Input(
        crate::tui::messages::InputMessage::StartNewTask,
    ));
    assert_eq!(app.input.mode, InputMode::InputTitle);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert_eq!(app.status.message.as_deref(), Some("Enter title: "));
}

#[test]
fn cancel_input_returns_to_normal() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("partial".to_string());
    app.input.task_draft = Some(TaskDraft::default());
    app.status.message = Some("Enter title: ".to_string());
    app.update(Message::Input(
        crate::tui::messages::InputMessage::CancelInput,
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn submit_title_with_text_advances_to_tag() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitTitle("My Task".to_string()),
    ));
    assert_eq!(app.input.mode, InputMode::InputTag);
    assert_eq!(app.input.task_draft.as_ref().unwrap().title, "My Task");
    assert_eq!(
        app.status.message.as_deref(),
        Some("Tag: [b]ug  [f]eature  [c]hore  pr-re[v]iew  [r]esearch  fi[x]  [p]hoenix  [Enter] none")
    );
}

#[test]
fn submit_empty_title_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitTitle(String::new()),
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
}

#[test]
fn submit_tag_advances_to_description() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitTag(Some(TaskTag::Bug)),
    ));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::Editor(crate::tui::commands::EditorCommand::PopOut(
            EditKind::Description { is_epic: false }
        ))
    ));
    assert_eq!(app.input.mode, InputMode::InputDescription);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().tag,
        Some(TaskTag::Bug)
    );
    assert_eq!(
        app.status.message.as_deref(),
        Some("Opening editor for description...")
    );
}

#[test]
fn submit_description_advances_to_repo_path() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitDescription("my desc".to_string()),
    ));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "my desc"
    );
}

#[test]
fn description_editor_result_advances_to_repo_path() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::Editor(
        crate::tui::messages::EditorMessage::DescriptionResult("some desc".to_string()),
    ));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "some desc"
    );
}

#[test]
fn description_editor_result_multiline() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::Editor(
        crate::tui::messages::EditorMessage::DescriptionResult("Line 1\nLine 2".to_string()),
    ));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "Line 1\nLine 2"
    );
}

#[test]
fn editor_result_description_saved_advances_draft() {
    // EditorResult{Description, Saved(raw)} must parse sections out of the
    // raw editor output and feed the description into the existing
    // DescriptionEditorResult flow.
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::Editor(
        crate::tui::messages::EditorMessage::Result {
            kind: EditKind::Description { is_epic: false },
            outcome: EditorOutcome::Saved("--- DESCRIPTION ---\nhello from editor\n".to_string()),
        },
    ));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "hello from editor"
    );
}

#[test]
fn editor_result_description_cancelled_cancels_input() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::Editor(
        crate::tui::messages::EditorMessage::Result {
            kind: EditKind::Description { is_epic: false },
            outcome: EditorOutcome::Cancelled,
        },
    ));
    // Cancelling during description input returns to Normal mode.
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn editor_result_task_edit_returns_finalize_command() {
    // Non-description EditKind variants route through a FinalizeEditorResult
    // command so the runtime applies the edit via services.
    let task = Task {
        title: "t".into(),
        description: "d".into(),
        repo_path: "/r".into(),
        ..make_task(42, TaskStatus::Backlog)
    };
    let mut app = App::new(vec![task.clone()]);
    let cmds = app.update(Message::Editor(
        crate::tui::messages::EditorMessage::Result {
            kind: EditKind::TaskEdit(Box::new(task)),
            outcome: EditorOutcome::Saved("--- TITLE ---\nNew\n".into()),
        },
    ));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Editor(crate::tui::commands::EditorCommand::FinalizeResult {
                kind: EditKind::TaskEdit(t),
                outcome: EditorOutcome::Saved(_),
            }) if t.id == TaskId(42)
        )),
        "expected FinalizeEditorResult(TaskEdit(42)), got {:?}",
        cmds
    );
}

#[test]
fn submit_repo_path_advances_to_base_branch() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        tag: Some(TaskTag::Bug),
        ..Default::default()
    });
    let cmds = without_branch_probe(app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/tmp".to_string()),
    )));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
}

#[test]
fn submit_base_branch_sets_branch_and_advances_to_wrap_up_mode() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputBaseBranch;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        repo_path: "/tmp".to_string(),
        tag: Some(TaskTag::Bug),
        base_branch: "main".into(),
        ..Default::default()
    });
    app.input.set_buffer("develop".to_string());
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch("develop".to_string()),
    ));
    // Now transitions to wrap-up mode selection instead of creating the task directly.
    assert_eq!(app.input.mode, InputMode::InputWrapUpMode);
    assert!(
        cmds.is_empty(),
        "no Insert yet — wrap-up mode selection is next"
    );
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().base_branch,
        "develop"
    );
}

#[test]
fn submit_base_branch_empty_uses_draft_default() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputBaseBranch;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        repo_path: "/tmp".to_string(),
        base_branch: "main".into(),
        ..Default::default()
    });
    app.input.set_buffer(String::new());
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitBaseBranch(String::new()),
    ));
    assert_eq!(app.input.mode, InputMode::InputWrapUpMode);
    assert!(
        cmds.is_empty(),
        "no Insert yet — wrap-up mode selection is next"
    );
    assert_eq!(app.input.task_draft.as_ref().unwrap().base_branch, "main");
}

/// Type a string into the active text field, one key at a time.
pub(super) fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
}

mod base_branch_picker;
mod detected_prefill;
mod wrap_up_and_normal_mode;
