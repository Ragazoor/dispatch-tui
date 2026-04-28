#![allow(unused_imports)]

use super::*;
use crate::models::{
    DispatchMode, Epic, EpicId, SubStatus, TaskId, TaskStatus, TaskTag, DEFAULT_QUICK_TASK_TITLE,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    Terminal,
};
use std::time::{Duration, Instant};

#[test]
fn task_created_adds_to_list() {
    let now = chrono::Utc::now();
    let task = Task {
        id: TaskId(42),
        title: "New Task".to_string(),
        description: "desc".to_string(),
        repo_path: "/repo".to_string(),
        status: TaskStatus::Backlog,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: SubStatus::None,
        pr_url: None,
        tag: None,
        sort_order: None,
        base_branch: "main".to_string(),
        external_id: None,
        created_at: now,
        updated_at: now,
        project_id: 1,
    };
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    let cmds = app.update(Message::TaskCreated { task });
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(42));
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
    assert!(cmds.is_empty());
}

#[test]
fn repo_path_empty_uses_saved_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string()];

    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    // Now advances to InputBaseBranch with "main" pre-filled
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
    // Submitting base branch completes creation
    let cmds2 = app.update(Message::SubmitBaseBranch("main".to_string()));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds2.iter().any(|c| matches!(
        c,
        Command::InsertTask { ref draft, .. } if draft.repo_path == "/tmp"
    )));
}

#[test]
fn repo_path_empty_no_saved_stays_in_mode() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    let cmds = app.update(Message::SubmitRepoPath("/nonexistent/path".to_string()));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
    let msg = app.status.message.as_ref().unwrap().as_str();
    assert!(msg.contains("does not exist"), "got: {msg}");
}

#[test]
fn repo_path_nonempty_used_as_is() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string()];

    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer = "/tmp".to_string();

    // Submitting repo path now advances to InputBaseBranch
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
    // Submitting base branch completes creation
    let cmds2 = app.update(Message::SubmitBaseBranch("main".to_string()));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds2
        .iter()
        .any(|c| matches!(c, Command::InsertTask { ref draft, .. } if draft.repo_path == "/tmp")));
    assert_eq!(app.board.tasks.len(), 0); // task not added until TaskCreated
}

#[test]
fn task_edited_updates_fields() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.update(Message::TaskEdited(TaskEdit {
        id: TaskId(1),
        title: "New".into(),
        description: "Desc".into(),
        repo_path: "/new".into(),
        status: TaskStatus::Running,
        plan_path: Some("docs/plan.md".into()),
        tag: None,
        base_branch: None,
    }));
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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.update(Message::RepoPathsUpdated(vec!["/a".into(), "/b".into()]));
    assert_eq!(app.board.repo_paths, vec!["/a", "/b"]);
}

#[test]
fn n_key_enters_title_mode() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputTitle);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert_eq!(app.status.message.as_deref(), Some("Enter title: "));
}

#[test]
fn backspace_pops_from_input_buffer() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "abc".to_string();
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn backspace_on_empty_buffer_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Backspace));
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn enter_with_title_advances_to_tag() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "My Task".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputTag);
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.task_draft.as_ref().unwrap().title, "My Task");
    assert_eq!(
        app.status.message.as_deref(),
        Some("Tag: [b]ug  [f]eature  [c]hore  [e]pic  [Enter] none")
    );
}

#[test]
fn enter_with_empty_title_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn enter_with_whitespace_only_title_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "   ".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
}

#[test]
fn enter_in_description_advances_to_repo_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer = "some desc".to_string();
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
fn number_key_in_repo_path_selects_saved_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "d".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    // Use real directories so validate_repo_path passes
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string()];
    // Number key selects repo, advances to InputBaseBranch
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
    // Confirming base branch creates the task
    let cmds2 = app.update(Message::SubmitBaseBranch("main".to_string()));
    assert!(cmds2
        .iter()
        .any(|c| matches!(c, Command::InsertTask { ref draft, .. } if draft.repo_path == "/var")));
}

#[test]
fn number_key_out_of_range_appends_to_buffer() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer = "/my".to_string();
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.handle_key(make_key(KeyCode::Char('1')));
    assert_eq!(app.input.buffer, "/my1");
}

#[test]
fn zero_key_in_repo_path_appends_to_buffer() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "partial".to_string();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn escape_from_description_mode_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer = "partial".to_string();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn escape_from_repo_path_mode_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer = "/partial".to_string();
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
    app.input.mode = InputMode::ConfirmDelete;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(1))); // task 1 deleted
    assert!(matches!(&cmds[0], Command::DeleteTask(TaskId(1))));
    assert!(app.status.message.is_none());
}

#[test]
fn confirm_delete_uppercase_y_deletes_task() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDelete;
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(1)));
    assert!(matches!(&cmds[0], Command::DeleteTask(TaskId(1))));
}

#[test]
fn confirm_delete_n_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDelete;
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks.len(), 4);
    assert!(cmds.is_empty());
    assert!(app.status.message.is_none());
}

#[test]
fn confirm_delete_esc_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.input.mode = InputMode::ConfirmDelete;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks.len(), 4);
    assert!(cmds.is_empty());
}

#[test]
fn x_key_on_empty_column_is_noop() {
    let mut app = make_app();
    app.selection_mut().set_column(3); // Review column is empty
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::Normal); // did NOT enter ConfirmArchive
}

#[test]
fn open_close_task_detail_via_messages() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
    app.update(Message::OpenTaskDetail(1));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));
    app.update(Message::CloseTaskDetail);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn enter_key_on_empty_board_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn e_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
}

#[test]
fn e_key_enters_confirm_edit_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1);
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmEditTask(TaskId(1))
    ));
    assert!(app.status.message.is_some());
}

#[test]
fn e_key_confirm_y_emits_edit_task() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1);
    app.handle_key(make_key(KeyCode::Char('e')));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PopOutEditor(EditKind::TaskEdit(t)) if t.id == TaskId(1)));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn e_key_confirm_n_cancels() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(1);
    app.handle_key(make_key(KeyCode::Char('e')));
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
}

#[test]
fn confirm_retry_r_key_emits_resume() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(c, Command::Resume { .. })));
}

#[test]
fn confirm_retry_f_key_emits_fresh() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { .. })));
}

#[test]
fn confirm_retry_esc_returns_to_normal() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty());
}

#[test]
fn start_new_task_enters_title_mode() {
    let mut app = make_app();
    app.update(Message::StartNewTask);
    assert_eq!(app.input.mode, InputMode::InputTitle);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert_eq!(app.status.message.as_deref(), Some("Enter title: "));
}

#[test]
fn cancel_input_returns_to_normal() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "partial".to_string();
    app.input.task_draft = Some(TaskDraft::default());
    app.status.message = Some("Enter title: ".to_string());
    app.update(Message::CancelInput);
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn submit_title_with_text_advances_to_tag() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::SubmitTitle("My Task".to_string()));
    assert_eq!(app.input.mode, InputMode::InputTag);
    assert_eq!(app.input.task_draft.as_ref().unwrap().title, "My Task");
    assert_eq!(
        app.status.message.as_deref(),
        Some("Tag: [b]ug  [f]eature  [c]hore  [e]pic  [Enter] none")
    );
}

#[test]
fn submit_empty_title_cancels() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::SubmitTitle(String::new()));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
}

#[test]
fn submit_tag_advances_to_description() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    let cmds = app.update(Message::SubmitTag(Some(TaskTag::Bug)));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::PopOutEditor(EditKind::Description { is_epic: false })
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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::SubmitDescription("my desc".to_string()));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "my desc"
    );
}

#[test]
fn description_editor_result_advances_to_repo_path() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::DescriptionEditorResult("some desc".to_string()));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "some desc"
    );
}

#[test]
fn description_editor_result_multiline() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::DescriptionEditorResult(
        "Line 1\nLine 2".to_string(),
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
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::EditorResult {
        kind: EditKind::Description { is_epic: false },
        outcome: EditorOutcome::Saved("--- DESCRIPTION ---\nhello from editor\n".to_string()),
    });
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().description,
        "hello from editor"
    );
}

#[test]
fn editor_result_description_cancelled_cancels_input() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.update(Message::EditorResult {
        kind: EditKind::Description { is_epic: false },
        outcome: EditorOutcome::Cancelled,
    });
    // Cancelling during description input returns to Normal mode.
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn editor_result_task_edit_returns_finalize_command() {
    // Non-description EditKind variants route through a FinalizeEditorResult
    // command so the runtime applies the edit via services.
    use crate::models::{Task, TaskId, TaskStatus};
    let task = Task {
        id: TaskId(42),
        title: "t".into(),
        description: "d".into(),
        repo_path: "/r".into(),
        status: TaskStatus::Backlog,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: crate::models::SubStatus::None,
        pr_url: None,
        tag: None,
        sort_order: None,
        base_branch: "main".to_string(),
        external_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        project_id: 1,
    };
    let mut app = App::new(vec![task.clone()], 1, TEST_TIMEOUT);
    let cmds = app.update(Message::EditorResult {
        kind: EditKind::TaskEdit(task),
        outcome: EditorOutcome::Saved("--- TITLE ---\nNew\n".into()),
    });
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::FinalizeEditorResult {
                kind: EditKind::TaskEdit(t),
                outcome: EditorOutcome::Saved(_),
            } if t.id == TaskId(42)
        )),
        "expected FinalizeEditorResult(TaskEdit(42)), got {:?}",
        cmds
    );
}

#[test]
fn submit_repo_path_advances_to_base_branch() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        tag: Some(TaskTag::Bug),
        ..Default::default()
    });
    let cmds = app.update(Message::SubmitRepoPath("/tmp".to_string()));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.buffer, "main");
    assert!(cmds.is_empty());
}

#[test]
fn submit_base_branch_creates_task_with_branch() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputBaseBranch;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        repo_path: "/tmp".to_string(),
        tag: Some(TaskTag::Bug),
        base_branch: "main".to_string(),
    });
    app.input.buffer = "develop".to_string();
    let cmds = app.update(Message::SubmitBaseBranch("develop".to_string()));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::InsertTask { ref draft, .. }
            if draft.repo_path == "/tmp"
                && draft.tag == Some(TaskTag::Bug)
                && draft.base_branch == "develop"
    )));
}

#[test]
fn submit_base_branch_empty_uses_draft_default() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::InputBaseBranch;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        repo_path: "/tmp".to_string(),
        base_branch: "main".to_string(),
        ..Default::default()
    });
    app.input.buffer = String::new();
    let cmds = app.update(Message::SubmitBaseBranch(String::new()));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::InsertTask { ref draft, .. } if draft.base_branch == "main"
    )));
}

#[test]
fn confirm_delete_start_enters_mode() {
    let mut app = make_app();
    app.update(Message::ConfirmDeleteStart);
    assert_eq!(app.input.mode, InputMode::ConfirmDelete);
    // make_app() selects column 0, row 0 = Task 1 (Backlog)
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete \"Task 1\" [backlog]? [y/n]")
    );
}

#[test]
fn cancel_delete_returns_to_normal() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmDelete;
    app.status.message = Some("Delete \"Task 1\" [backlog]? [y/n]".to_string());
    app.update(Message::CancelDelete);
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
}

#[test]
fn cancel_retry_returns_to_normal() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));
    app.status.message = Some("Agent stale".to_string());
    app.update(Message::CancelRetry);
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
}

#[test]
fn esc_clears_selection() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));
    assert_eq!(app.select.tasks.len(), 2);

    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn esc_with_no_selection_is_noop() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn x_key_with_selection_shows_count_in_confirm() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert!(matches!(app.input.mode, InputMode::ConfirmArchive(None)));
    assert_eq!(
        app.status.message.as_deref(),
        Some("Archive 2 items? [y/n]")
    );
}

#[test]
fn enter_on_task_does_not_enter_task_detail_without_input_routing() {
    // Enter in Normal mode on a task is currently a no-op (input routing added in Task 5).
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn left_arrow_navigates_column() {
    let mut app = make_app();
    app.selection_mut().set_column(2);
    app.handle_key(make_key(KeyCode::Left));
    assert_eq!(app.selection().column(), 1);
}

#[test]
fn right_arrow_navigates_column() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.handle_key(make_key(KeyCode::Right));
    assert_eq!(app.selection().column(), 2);
}

#[test]
fn down_arrow_navigates_row() {
    let mut app = make_app();
    app.selection_mut().set_column(1); // Backlog has 2 tasks
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.selection().row(1), 1);
}

#[test]
fn up_arrow_navigates_row() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);
    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.selection().row(1), 0);
}

#[test]
fn confirm_retry_unrecognized_key_is_noop() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn esc_dismisses_help() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_delete_start_running_with_worktree_shows_warning() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt/4-test".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    // Task is in Running column (column 2), navigate there
    app.selection_mut().set_column(2);
    app.update(Message::ConfirmDeleteStart);
    assert_eq!(app.input.mode, InputMode::ConfirmDelete);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete \"Task 4\" [running] (has worktree)? [y/n]")
    );
}

#[test]
fn esc_clears_selection_and_exits_toggle() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.tasks.is_empty());
    assert!(!app.on_select_all());
}

#[test]
fn handle_key_dismisses_error_popup() {
    let mut app = make_app();
    app.status.error_popup = Some("something went wrong".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Char('q')));
    assert!(app.status.error_popup.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_normal_navigation() {
    let mut app = make_app();
    // Start at column 1 (Backlog), row 0
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    // 'l' moves right
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_eq!(app.selection().column(), 2);

    // 'h' moves left
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.selection().column(), 1);

    // 'j' moves down
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.selection().row(1), 1);

    // 'k' moves up
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.selection().row(1), 0);
}

#[test]
fn handle_key_normal_q_opens_projects_panel() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(
        app.projects_panel_visible(),
        "q should open projects panel, not quit"
    );
    assert!(!app.should_quit());
}

#[test]
fn confirm_quit_y_quits() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmQuit;
    app.handle_key(make_key(KeyCode::Char('y')));
    assert!(app.should_quit);
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_quit_n_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmQuit;
    app.handle_key(make_key(KeyCode::Char('n')));
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_quit_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmQuit;
    app.handle_key(make_key(KeyCode::Esc));
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn handle_key_normal_new_task() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::InputTitle);
}

#[test]
fn handle_key_normal_toggle_help() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(*app.mode(), InputMode::Help);
}

#[test]
fn handle_key_help_dismiss() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    // '?' toggles help off
    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_help_esc_dismiss() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_text_input_char_and_backspace() {
    let mut app = make_app();
    // Enter title input mode
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::InputTitle);

    // Type characters
    app.handle_key(make_key(KeyCode::Char('H')));
    app.handle_key(make_key(KeyCode::Char('i')));
    assert_eq!(app.input.buffer, "Hi");

    // Backspace removes last char
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "H");
}

#[test]
fn handle_key_text_input_esc_cancels() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::InputTitle);

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_text_input_enter_advances_to_tag() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('n')));
    app.handle_key(make_key(KeyCode::Char('T')));
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::InputTag);
}

#[test]
fn handle_key_confirm_retry_resume() {
    let mut app = make_app();
    let mut task = make_task(10, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/10-test".to_string());
    task.tmux_window = Some("main:10-test".to_string());
    app.board.tasks.push(task);
    app.input.mode = InputMode::ConfirmRetry(TaskId(10));

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    // Should produce KillTmuxWindow + Resume
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::KillTmuxWindow { .. })));
    assert!(cmds.iter().any(|c| matches!(c, Command::Resume { .. })));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_confirm_retry_fresh() {
    let mut app = make_app();
    let mut task = make_task(10, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/10-test".to_string());
    task.tmux_window = Some("main:10-test".to_string());
    app.board.tasks.push(task);
    app.input.mode = InputMode::ConfirmRetry(TaskId(10));

    let cmds = app.handle_key(make_key(KeyCode::Char('f')));
    // Should produce Cleanup + Dispatch
    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { .. })));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_confirm_retry_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmRetry(TaskId(10));
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_quick_dispatch_digit_selects() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::QuickDispatch;

    let cmds = app.handle_key(make_key(KeyCode::Char('1')));
    // Should produce a QuickDispatch command
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::QuickDispatch { .. })));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_quick_dispatch_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::QuickDispatch;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_tag_selects_bug() {
    let mut app = make_app();
    // Tag comes right after title, before description/repo
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    let cmds = app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::PopOutEditor(EditKind::Description { is_epic: false })
    ));
    assert_eq!(*app.mode(), InputMode::InputDescription);
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().tag,
        Some(TaskTag::Bug)
    );
}

#[test]
fn handle_key_tag_skip_with_enter() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::PopOutEditor(EditKind::Description { is_epic: false })
    ));
    assert_eq!(*app.mode(), InputMode::InputDescription);
    assert_eq!(app.input.task_draft.as_ref().unwrap().tag, None);
}

#[test]
fn handle_key_tag_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_normal_dispatch_backlog_task() {
    let mut app = make_app();
    // Select task 1 (backlog)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { .. })));
}

#[test]
fn handle_key_normal_dispatch_running_task_with_window_shows_info() {
    let mut app = make_app();
    // Select running task (column 2)
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    // Give running task a window
    let task_3 = app
        .board
        .tasks
        .iter_mut()
        .find(|t| t.id == TaskId(3))
        .unwrap();
    task_3.tmux_window = Some("main:task-3".to_string());

    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    // Should just show status info, no dispatch
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_normal_enter_is_noop_without_task_detail_routing() {
    // Enter key routing for TaskDetail is added in Task 5.
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_normal_jump_to_tmux() {
    let mut app = make_app();
    // Give task 3 (running) a tmux window
    let task = app
        .board
        .tasks
        .iter_mut()
        .find(|t| t.id == TaskId(3))
        .unwrap();
    task.tmux_window = Some("main:task-3".to_string());
    // Select running column
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::JumpToTmux { window } if window == "main:task-3")));
}

#[test]
fn handle_key_normal_open_pr_url() {
    let mut app = make_app();
    let task = app
        .board
        .tasks
        .iter_mut()
        .find(|t| t.id == TaskId(1))
        .unwrap();
    task.pr_url = Some("https://github.com/example/repo/pull/42".to_string());
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::OpenInBrowser { url } if url == "https://github.com/example/repo/pull/42"
    )));
}

#[test]
fn handle_key_normal_open_pr_url_missing() {
    let mut app = make_app();
    // task 1 has no pr_url by default
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds.is_empty());
    assert!(app.status.message.as_deref().unwrap().contains("No PR URL"));
}

#[test]
fn esc_clears_mixed_selection() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.tasks.is_empty());
    assert!(app.select.epics.is_empty());
}

#[test]
fn confirm_detach_tmux_clears_window() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], 1, TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-1".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.agents
        .tmux_outputs
        .insert(TaskId(1), "some output".to_string());

    app.update(Message::DetachTmux(TaskId(1)));
    let cmds = app.update(Message::ConfirmDetachTmux);

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(
        app.board.tasks[0].tmux_window.is_none(),
        "tmux_window should be cleared"
    );
    assert_ne!(
        app.find_task(TaskId(1)).unwrap().sub_status,
        SubStatus::Stale,
        "stale tracking should be cleared"
    );
    assert!(
        !app.agents.tmux_outputs.contains_key(&TaskId(1)),
        "tmux output should be cleared"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "task-1")),
        "should emit KillTmuxWindow for task-1"
    );
    assert!(
        cmds.iter().any(|c| matches!(c, Command::PersistTask(_))),
        "should emit PersistTask"
    );
}

#[test]
fn confirm_edit_task_y_emits_editor_command() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEditTask(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PopOutEditor(EditKind::TaskEdit(_)))));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_edit_task_n_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEditTask(TaskId(1));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_edit_task_y_with_missing_task_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEditTask(TaskId(999));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_detach_tmux_y_detaches() {
    let mut task = make_task(3, TaskStatus::Review);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmDetachTmux(vec![TaskId(3)]);
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    // Should produce KillTmuxWindow + PatchSubStatus commands
    assert!(!cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_detach_tmux_n_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDetachTmux(vec![TaskId(3)]);
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn handle_key_normal_copy_task() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    app.handle_key(make_key(KeyCode::Char('c')));
    // CopyTask skips title/tag and goes straight to repo path with pre-filled buffer
    assert_eq!(*app.mode(), InputMode::InputRepoPath);
    assert!(app
        .input
        .task_draft
        .as_ref()
        .unwrap()
        .title
        .contains("Task 1"));
}

#[test]
fn handle_key_normal_toggle_notifications() {
    let mut app = make_app();
    let before = app.notifications_enabled;
    app.handle_key(make_key(KeyCode::Char('N')));
    assert_ne!(app.notifications_enabled, before);
}

#[test]
fn handle_key_normal_move_forward_via_handle_key() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    // Task 1 should move from Backlog to Running
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(1) && t.status == TaskStatus::Running)));
}

#[test]
fn handle_key_normal_move_backward_via_handle_key() {
    let mut app = make_app();
    // Select running task (column 2)
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    // Task 3 should move from Running to Backlog
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3) && t.status == TaskStatus::Backlog)));
}

#[test]
fn handle_key_normal_detach_tmux_review_task() {
    let mut task = make_task(10, TaskStatus::Review);
    task.tmux_window = Some("main:10-test".to_string());
    let mut app = App::new(vec![task], 1, TEST_TIMEOUT);
    app.selection_mut().set_column(3);
    app.selection_mut().set_row(3, 0);
    app.handle_key(make_key(KeyCode::Char('T')));
    assert!(matches!(*app.mode(), InputMode::ConfirmDetachTmux(_)));
}

#[test]
fn handle_key_normal_detach_tmux_no_window_is_noop() {
    let mut app = make_app();
    // Task 1 has no tmux window
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('T')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_normal_unknown_key_is_noop() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_text_input_repo_j_types_into_buffer() {
    // j should be a typeable character in the repo path search box
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.buffer, "j");
    assert_eq!(app.input.repo_cursor, 0); // cursor resets on query change
}

#[test]
fn handle_key_text_input_repo_k_types_into_buffer() {
    // k should be a typeable character in the repo path search box
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.buffer, "k");
    assert_eq!(app.input.repo_cursor, 0); // cursor resets on query change
}

#[test]
fn handle_key_text_input_repo_jk_typed_together() {
    // Typing "jk" should appear in the search buffer
    let mut app = make_app();
    app.board.repo_paths = vec!["/jk-repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();

    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.buffer, "jk");
}

#[test]
fn handle_key_text_input_repo_arrow_down_navigates() {
    // Arrow keys should still navigate the list
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn handle_key_text_input_repo_arrow_up_navigates() {
    // Arrow keys should still navigate the list
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 1;

    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn handle_key_text_input_repo_enter_selects_cursor_repo() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    app.input.repo_cursor = 1;

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    // Now advances to InputBaseBranch; task not created until base branch submitted
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert!(cmds.is_empty());
    let cmds2 = app.update(Message::SubmitBaseBranch("main".to_string()));
    assert!(cmds2
        .iter()
        .any(|c| matches!(c, Command::InsertTask { .. })));
}

#[test]
fn handle_key_text_input_enter_submits_typed_text() {
    let mut app = make_app();
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        description: "desc".to_string(),
        ..Default::default()
    });
    app.input.buffer = "/tmp".to_string();

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    // Now advances to InputBaseBranch; task not created until base branch submitted
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert!(cmds.is_empty());
    let cmds2 = app.update(Message::SubmitBaseBranch("main".to_string()));
    assert!(cmds2
        .iter()
        .any(|c| matches!(c, Command::InsertTask { .. })));
}

#[test]
fn handle_key_quick_dispatch_j_moves_cursor_down() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn handle_key_quick_dispatch_k_moves_cursor_up() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 1;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn handle_key_quick_dispatch_enter_selects_current() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::QuickDispatch { .. })));
}

#[test]
fn handle_key_quick_dispatch_down_arrow() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn handle_key_quick_dispatch_unknown_key_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_tag_selects_feature() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().tag,
        Some(TaskTag::Feature)
    );
}

#[test]
fn handle_key_tag_selects_chore() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    app.handle_key(make_key(KeyCode::Char('c')));
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().tag,
        Some(TaskTag::Chore)
    );
}

#[test]
fn handle_key_tag_unknown_key_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
    assert_eq!(*app.mode(), InputMode::InputTag);
}

#[test]
fn handle_key_confirm_detach_tmux_non_matching_mode_is_noop() {
    let mut app = make_app();
    // Mode is Normal but we call handle_key_confirm_detach_tmux indirectly
    // This shouldn't happen in practice, but confirms guard clause
    app.input.mode = InputMode::Normal;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    // In Normal mode, 'y' is unrecognized — noop
    assert!(cmds.is_empty());
}

/// In Normal mode on the Board view, known keys produce commands/state changes
/// and unknown keys produce no commands.
#[test]
fn handle_key_normal_board_known_keys_produce_effects() {
    let mut app = make_app();
    // 'n' starts new task (switches to InputTitle mode)
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty()); // inline mutation, no commands
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn handle_key_normal_board_unknown_key_is_noop() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputTitle mode routes to the text input handler.
#[test]
fn handle_key_input_title_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTitle;
    // Esc cancels input
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputDescription mode routes to the text input handler.
#[test]
fn handle_key_input_description_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputDescription;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputRepoPath mode routes to the text input handler.
#[test]
fn handle_key_input_repo_path_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputRepoPath;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDelete mode routes to the confirm-delete handler.
#[test]
fn handle_key_confirm_delete_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDelete;
    // 'n' cancels the delete
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputTag mode routes to the tag handler.
#[test]
fn handle_key_input_tag_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    // Esc cancels tag input
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// QuickDispatch mode routes to the quick-dispatch handler.
#[test]
fn handle_key_quick_dispatch_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::QuickDispatch;
    // Esc cancels quick dispatch
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmRetry mode routes to the confirm-retry handler.
#[test]
fn handle_key_confirm_retry_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmRetry(TaskId(1));
    // Esc cancels retry
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDetachTmux mode routes correctly.
#[test]
fn handle_key_confirm_detach_tmux_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDetachTmux(vec![TaskId(1)]);
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmEditTask mode routes correctly.
#[test]
fn handle_key_confirm_edit_task_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEditTask(TaskId(1));
    // Esc cancels
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// Help mode routes to the help handler.
#[test]
fn handle_key_help_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;
    // Any key exits help
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmQuit mode routes correctly.
#[test]
fn handle_key_confirm_quit_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmQuit;
    // 'n' cancels quit
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// Error popup dismisses on any key before routing to normal handler.
#[test]
fn handle_key_error_popup_dismisses_first() {
    let mut app = make_app();
    app.status.error_popup = Some("Some error".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert!(app.status.error_popup.is_none());
}

#[test]
fn confirm_quit_without_split_emits_no_extra_commands() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmQuit;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));

    assert!(app.should_quit);
    assert!(cmds.is_empty(), "no commands when split is not active");
}

#[test]
fn number_key_selects_from_filtered_list() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    // Use two real existing dirs that both fuzzy-match "tmp" (both contain t, m, p)
    // /tmp exists; /var/tmp also exists and contains t, m, p
    app.board.repo_paths = vec![
        "/tmp".to_string(),
        "/var".to_string(),
        "/var/tmp".to_string(),
    ];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Type "tmp" — filtered = ["/tmp", "/var/tmp"]
    for c in "tmp".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    // Press '2' — selects /var/tmp (2nd in filtered)
    app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    let cmds = app.update(Message::SubmitBaseBranch("main".to_string()));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::InsertTask { ref draft, .. } if draft.repo_path == "/var/tmp"
    )));
}

#[test]
fn enter_with_typed_filter_selects_filtered_item() {
    let mut app = App::new(vec![], 1, TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Type "var" — only /var matches, cursor = 0
    for c in "var".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    // Enter selects /var
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert_eq!(app.input.task_draft.as_ref().unwrap().repo_path, "/var");
}
