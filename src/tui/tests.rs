use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    Terminal,
};
use std::time::{Duration, Instant};

use super::*;
use crate::models::{
    DispatchMode, Epic, EpicId, SubStatus, TaskId, TaskStatus, TaskTag, DEFAULT_QUICK_TASK_TITLE,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Check whether a rendered buffer contains the given text anywhere.
fn buffer_contains(buf: &Buffer, text: &str) -> bool {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buf[(x, y)].symbol());
        }
        if line.contains(text) {
            return true;
        }
    }
    false
}

/// Helper: render the app into a test terminal and return the buffer.
fn render_to_buffer(app: &mut App, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn make_task(id: i64, status: TaskStatus) -> Task {
    let now = chrono::Utc::now();
    Task {
        id: TaskId(id),
        title: format!("Task {id}"),
        description: String::new(),
        repo_path: String::from("/repo"),
        status,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: SubStatus::default_for(status),
        pr_url: None,
        tag: None,
        sort_order: None,
        base_branch: "main".to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn make_app() -> App {
    App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
            make_task(3, TaskStatus::Running),
            make_task(4, TaskStatus::Done),
        ],
        TEST_TIMEOUT,
    )
}

#[test]
fn tasks_by_status_filters() {
    let app = make_app();
    let backlog = app.tasks_by_status(TaskStatus::Backlog);
    assert_eq!(backlog.len(), 2);
    assert_eq!(backlog[0].id, TaskId(1));
    assert_eq!(backlog[1].id, TaskId(2));

    let running = app.tasks_by_status(TaskStatus::Running);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, TaskId(3));

    let review = app.tasks_by_status(TaskStatus::Review);
    assert_eq!(review.len(), 0);
}

#[test]
fn move_task_forward() {
    let mut app = make_app();
    // Task 1 is in Backlog; move it forward -> Running
    let cmds = app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    });
    assert_eq!(
        app.board
            .tasks
            .iter()
            .find(|t| t.id == TaskId(1))
            .unwrap()
            .status,
        TaskStatus::Running
    );
    // Should produce a PersistTask command
    assert!(matches!(cmds[0], Command::PersistTask(_)));
}

#[test]
fn move_task_backward_at_start_is_noop() {
    let mut app = make_app();
    // Task 1 is in Backlog; prev() stays Backlog
    let cmds = app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    });
    assert_eq!(
        app.board
            .tasks
            .iter()
            .find(|t| t.id == TaskId(1))
            .unwrap()
            .status,
        TaskStatus::Backlog
    );
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_only_backlog_tasks() {
    let mut app = make_app();

    // Task 1 is Backlog — should dispatch
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));

    // Task 3 is Running — should not dispatch
    let cmds = app.update(Message::DispatchTask(TaskId(3), DispatchMode::Dispatch));
    assert!(cmds.is_empty());

    // Task 4 is Done — should not dispatch
    let cmds = app.update(Message::DispatchTask(TaskId(4), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn quit_enters_confirm_mode() {
    let mut app = make_app();
    assert!(!app.should_quit);
    app.update(Message::Quit);
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn navigate_column_clamps() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.update(Message::NavigateColumn(-1));
    assert_eq!(app.selection().column(), 0); // can't go below 0

    app.selection_mut().set_column(TaskStatus::COLUMN_COUNT - 1);
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selection().column(), TaskStatus::COLUMN_COUNT - 1); // can't go above max
}

#[test]
fn navigate_column_moves_through_visual_columns() {
    let mut app = make_app();
    assert_eq!(app.selected_column(), 0); // Backlog
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 1); // Active
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 2); // Blocked
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 3); // Stale
}

#[test]
fn navigate_column_clamps_at_visual_column_max() {
    let mut app = make_app();
    app.selection_mut().set_column(TaskStatus::COLUMN_COUNT - 1); // Done column (3)
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), TaskStatus::COLUMN_COUNT - 1); // stays at 3
}

#[test]
fn navigate_row_clamps() {
    let mut app = make_app();
    // Backlog has 2 tasks (id 1, 2). Selected row starts at 0.
    app.selection_mut().set_column(0);
    app.update(Message::NavigateRow(-1));
    // Navigating up from row 0 now moves to the select-all toggle
    assert!(app.on_select_all());

    // Navigate back down to tasks and then past the end
    app.update(Message::NavigateRow(1));
    assert!(!app.on_select_all());
    app.update(Message::NavigateRow(10));
    assert_eq!(app.selection().row(0), 1); // clamps to last item index
}

#[test]
fn tick_produces_capture_for_running_tasks_with_window() {
    let mut task4 = make_task(4, TaskStatus::Running);
    task4.tmux_window = Some("main:task-4".to_string());
    let mut app = App::new(vec![task4], TEST_TIMEOUT);
    let cmds = app.update(Message::Tick);
    // Should have CaptureTmux + FetchReviewPrs + FetchMyPrs + RefreshFromDb
    assert_eq!(cmds.len(), 4);
    assert!(
        matches!(&cmds[0], Command::CaptureTmux { id: TaskId(4), window } if window == "main:task-4")
    );
    assert!(matches!(&cmds[1], Command::FetchPrs(PrListKind::Review)));
    assert!(matches!(&cmds[2], Command::FetchPrs(PrListKind::Authored)));
    assert!(matches!(&cmds[3], Command::RefreshFromDb));
}

#[test]
fn tick_captures_review_task_with_live_window() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Tick);

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(5), .. })));
}

#[test]
fn tick_fetches_my_prs_when_stale() {
    let mut app = make_app();
    assert!(app.review.authored.last_fetch.is_none());
    let cmds = app.update(Message::Tick);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Authored))));
}

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
        created_at: now,
        updated_at: now,
    };
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let cmds = app.update(Message::TaskCreated { task });
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(42));
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
    assert!(cmds.is_empty());
}

#[test]
fn delete_task_with_worktree_emits_cleanup() {
    let mut app = make_app();
    let task = app.find_task_mut(TaskId(4)).unwrap();
    task.worktree = Some("/repo/.worktrees/4-task".to_string());
    task.tmux_window = Some("task-4".to_string());

    let cmds = app.update(Message::DeleteTask(TaskId(4)));
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(4)));
    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteTask(TaskId(4)))));
}

#[test]
fn delete_task_without_worktree_no_cleanup() {
    let mut app = make_app();
    let cmds = app.update(Message::DeleteTask(TaskId(1)));
    assert!(!cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
}

#[test]
fn error_sets_error_popup() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::Error("Something went wrong".to_string()));
    assert_eq!(
        app.status.error_popup.as_deref(),
        Some("Something went wrong")
    );
}

#[test]
fn dispatch_from_running_is_noop() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchTask(TaskId(4), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_from_review_is_noop() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchTask(TaskId(5), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn move_backward_from_running_detaches_but_keeps_worktree() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::MoveTask {
        id: TaskId(4),
        direction: MoveDirection::Backward,
    });

    // Should emit KillTmuxWindow then PersistTask (no Cleanup)
    assert_eq!(cmds.len(), 2);
    assert!(matches!(&cmds[0], Command::KillTmuxWindow { window } if window == "task-4"));
    assert!(matches!(&cmds[1], Command::PersistTask(_)));

    // Worktree preserved, tmux_window cleared
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(4)).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.worktree.as_deref(), Some("/repo/.worktrees/4-task-4"));
    assert!(task.tmux_window.is_none());
}

#[test]
fn move_backward_from_running_without_dispatch_fields() {
    let task = make_task(3, TaskStatus::Running);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::MoveTask {
        id: TaskId(3),
        direction: MoveDirection::Backward,
    });
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::PersistTask(_)));
}

#[test]
fn repo_path_empty_uses_saved_path() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
fn dispatch_repo_path_nonexistent_shows_error() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);
    app.handle_key(KeyEvent::from(KeyCode::Char('d')));

    let cmds = app.update(Message::SubmitDispatchRepoPath("origin".to_string()));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
    let msg = app.status.message.as_ref().unwrap().as_str();
    assert!(msg.contains("does not exist"), "got: {msg}");
}

#[test]
fn repo_path_nonempty_used_as_is() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::RepoPathsUpdated(vec!["/a".into(), "/b".into()]));
    assert_eq!(app.board.repo_paths, vec!["/a", "/b"]);
}

#[test]
fn move_forward_to_done_enters_confirm_mode() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = None; // session closed, but worktree remains
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::MoveTask {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    });

    // Should enter confirmation mode, not move immediately
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(5))));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(5)).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    // Worktree preserved — not taken during confirmation
    assert!(task.worktree.is_some());
}

#[test]
fn move_forward_to_done_with_live_window_enters_confirm_mode() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::MoveTask {
        id: TaskId(5),
        direction: MoveDirection::Forward,
    });

    // Should enter confirmation mode, not move immediately
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(5))));
}

#[test]
fn d_key_on_backlog_with_plan_dispatches() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(0); // Backlog column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(&cmds[0], Command::DispatchAgent { .. }));
}

#[test]
fn d_key_on_running_with_window_shows_warning() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("already running"));
}

#[test]
fn d_key_on_running_no_window_resumes() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(&cmds[0], Command::Resume { .. }));
}

#[test]
fn d_key_on_backlog_brainstorms() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Epic); // tag=epic triggers brainstorm
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(0); // Backlog column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::DispatchAgent { task, mode: DispatchMode::Brainstorm } if task.id == TaskId(1))
    );
}

#[test]
fn d_key_on_done_shows_warning() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)], TEST_TIMEOUT);
    app.selection_mut().set_column(3); // Done column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
}

#[test]
fn d_key_on_running_no_worktree_no_window_shows_warning() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = None;
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No worktree"));
}

#[test]
fn g_key_with_live_window_jumps() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "task-4"));
}

#[test]
fn brainstorm_only_backlog_tasks() {
    let mut app = make_app();

    // Task 1 is Backlog — should brainstorm
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Brainstorm));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::DispatchAgent { task, mode: DispatchMode::Brainstorm } if task.id == TaskId(1))
    );

    // Task 3 is Running — should not brainstorm
    let cmds = app.update(Message::DispatchTask(TaskId(3), DispatchMode::Brainstorm));
    assert!(cmds.is_empty());

    // Task 4 is Done — should not brainstorm
    let cmds = app.update(Message::DispatchTask(TaskId(4), DispatchMode::Brainstorm));
    assert!(cmds.is_empty());
}

#[test]
fn g_key_without_window_shows_message() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No active session"));
}

// --- Task creation key flow ---

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
fn typing_appends_to_input_buffer() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.handle_key(make_key(KeyCode::Char('H')));
    app.handle_key(make_key(KeyCode::Char('i')));
    assert_eq!(app.input.buffer, "Hi");
}

#[test]
fn backspace_pops_from_input_buffer() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "abc".to_string();
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn backspace_on_empty_buffer_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Backspace));
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn enter_with_title_advances_to_tag() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
    assert!(app.status.message.is_none());
}

#[test]
fn enter_with_whitespace_only_title_cancels() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "   ".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
}

#[test]
fn enter_in_description_advances_to_repo_path() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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

// --- Delete confirmation flow (via ConfirmDelete mode directly) ---

#[test]
fn confirm_delete_y_deletes_task() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
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
    app.selection_mut().set_column(0);
    app.input.mode = InputMode::ConfirmDelete;
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.tasks.iter().all(|t| t.id != TaskId(1)));
    assert!(matches!(&cmds[0], Command::DeleteTask(TaskId(1))));
}

#[test]
fn confirm_delete_n_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
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
    app.selection_mut().set_column(0);
    app.input.mode = InputMode::ConfirmDelete;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks.len(), 4);
    assert!(cmds.is_empty());
}

// --- Archive confirmation flow (x key) ---

#[test]
fn x_key_enters_confirm_archive_mode() {
    let mut app = make_app();
    app.selection_mut().set_column(0); // Backlog has tasks
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmArchive);
    assert_eq!(app.status.message.as_deref(), Some("Archive task? [y/n]"));
}

#[test]
fn confirm_archive_y_emits_archive_task() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.handle_key(make_key(KeyCode::Char('x')));
    let _ = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    // Task 1 should now be Archived
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Archived);
}

#[test]
fn confirm_archive_n_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.handle_key(make_key(KeyCode::Char('x')));
    let _ = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    // Task 1 still in Backlog
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
}

#[test]
fn x_key_on_empty_column_is_noop() {
    let mut app = make_app();
    app.selection_mut().set_column(2); // Review column is empty
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::Normal); // did NOT enter ConfirmArchive
}

// --- H key toggles archive panel ---

#[test]
fn shift_h_toggles_archive() {
    let mut app = make_app();
    assert!(!app.archive.visible);
    app.handle_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(app.archive.visible);
    app.handle_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(!app.archive.visible);
}

// --- Error popup dismissal ---

#[test]
fn any_key_clears_error_popup() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.status.error_popup = Some("boom".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.status.error_popup.is_none());
    assert!(cmds.is_empty());
}

// --- QuickDispatch ---

fn make_shift_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

#[test]
fn shift_d_with_one_repo_emits_quick_dispatch() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo".to_string()];
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None } if draft.repo_path == "/repo")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn shift_d_with_no_repos_shows_error() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec![];
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn shift_d_with_multiple_repos_enters_quick_dispatch_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_mode_number_selects_repo() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None } if draft.repo_path == "/repo2")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_mode_esc_cancels() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_mode_invalid_number_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('3')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_message_emits_command() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let cmds = app.update(Message::QuickDispatch {
        repo_path: "/my/repo".to_string(),
        epic_id: None,
    });
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None }
        if draft.title == DEFAULT_QUICK_TASK_TITLE && draft.repo_path == "/my/repo")
    );
}

#[test]
fn shift_d_in_epic_view_quick_dispatches_subtask_single_repo() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/my/repo".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0],
        Command::QuickDispatch { ref draft, epic_id: Some(EpicId(10)) }
        if draft.repo_path == "/my/repo"
    ));
}

#[test]
fn shift_d_in_epic_view_shows_repo_selection_with_multiple_repos() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert_eq!(app.input.pending_epic_id, Some(EpicId(10)));
}

#[test]
fn shift_d_in_epic_view_repo_selection_dispatches_with_epic_id() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    // Enter selection mode
    app.handle_key(make_shift_key(KeyCode::Char('D')));
    // Select second repo
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0],
        Command::QuickDispatch { ref draft, epic_id: Some(EpicId(10)) }
        if draft.repo_path == "/repo/b"
    ));
}

#[test]
fn error_popup_blocks_normal_key_handling() {
    let mut app = make_app();
    app.status.error_popup = Some("boom".to_string());
    app.handle_key(make_key(KeyCode::Char('q'))); // would normally quit
    assert!(app.status.error_popup.is_none());
    assert!(!app.should_quit); // quit was NOT processed
}

// --- Toggle detail ---

#[test]
fn toggle_detail_flips_visibility() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    assert!(!app.board.detail_visible);
    app.update(Message::ToggleDetail);
    assert!(app.board.detail_visible);
    app.update(Message::ToggleDetail);
    assert!(!app.board.detail_visible);
}

#[test]
fn stale_agent_detected_after_timeout() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(4), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    assert!(app.is_stale(TaskId(4)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(4), .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(4))));
}

#[test]
fn window_gone_on_running_task_marks_crashed() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());

    let cmds = app.update(Message::WindowGone(TaskId(4)));
    assert!(app.is_crashed(TaskId(4)));
    // tmux_window should be cleared — the window is gone by definition
    assert!(app.board.tasks[0].tmux_window.is_none());
    // Should emit PersistTask with cleared tmux_window
    assert!(cmds.iter().any(
        |c| matches!(c, Command::PersistTask(t) if t.id == TaskId(4) && t.tmux_window.is_none())
    ));
}

#[test]
fn window_gone_on_review_task_clears_window() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Review)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());

    let cmds = app.update(Message::WindowGone(TaskId(4)));
    assert!(!app.is_crashed(TaskId(4)));
    assert!(app.board.tasks[0].tmux_window.is_none());
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
}

#[test]
fn tmux_output_change_resets_staleness_timer() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(4), Instant::now() - Duration::from_secs(301));
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);

    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "output".to_string(),
        activity_ts: 1001,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn tmux_output_same_activity_does_not_reset_timer() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    let old_instant = Instant::now() - Duration::from_secs(200);
    app.agents.last_active_at.insert(TaskId(4), old_instant);
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);

    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "output".to_string(),
        activity_ts: 1000,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed >= Duration::from_secs(199));
}

#[test]
fn activity_ts_change_with_same_output_resets_timer() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(4), Instant::now() - Duration::from_secs(301));
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);
    app.agents
        .tmux_outputs
        .insert(TaskId(4), "same output".to_string());

    // Same display text, but tmux reports new activity
    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "same output".to_string(),
        activity_ts: 1001,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn activity_ts_same_with_different_output_no_reset() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    let old_instant = Instant::now() - Duration::from_secs(200);
    app.agents.last_active_at.insert(TaskId(4), old_instant);
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);
    app.agents
        .tmux_outputs
        .insert(TaskId(4), "old text".to_string());

    // Different display text, but same activity timestamp
    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "new text".to_string(),
        activity_ts: 1000,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed >= Duration::from_secs(199));
    // Display output is still updated for rendering
    assert_eq!(app.agents.tmux_outputs.get(&TaskId(4)).unwrap(), "new text");
}

#[test]
fn enter_key_toggles_detail() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    assert!(!app.board.detail_visible);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.board.detail_visible);
}

// --- Async message handlers ---

#[test]
fn dispatched_sets_fields_and_transitions_to_running() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: false,
    });
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(3)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/wt"));
    assert_eq!(task.tmux_window.as_deref(), Some("win"));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
}

#[test]
fn dispatched_with_switch_focus_emits_jump() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: true,
    });
    assert_eq!(cmds.len(), 2);
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
    assert!(matches!(&cmds[1], Command::JumpToTmux { window } if window == "win"));
}

#[test]
fn dispatched_unknown_id_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    let cmds = app.update(Message::Dispatched {
        id: TaskId(999),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: false,
    });
    assert!(cmds.is_empty());
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
}

#[test]
fn resumed_sets_tmux_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "win-4".to_string(),
    });
    assert_eq!(app.board.tasks[0].tmux_window.as_deref(), Some("win-4"));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
}

#[test]
fn resumed_unknown_id_is_noop() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    let cmds = app.update(Message::Resumed {
        id: TaskId(999),
        tmux_window: "win".to_string(),
    });
    assert!(cmds.is_empty());
}

#[test]
fn resumed_sets_status_to_running() {
    let mut task = make_task(4, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "task-4".to_string(),
    });

    let task = app.board.tasks.iter().find(|t| t.id == TaskId(4)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.tmux_window.as_deref(), Some("task-4"));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(t) if t.status == TaskStatus::Running));
}

#[test]
fn resumed_sets_success_status_message() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "win-4".to_string(),
    });

    assert_eq!(app.status.message.as_deref(), Some("Task 4 resumed"),);
}

#[test]
fn tmux_output_stores_in_map() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)], TEST_TIMEOUT);
    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(1),
        output: "hello".to_string(),
        activity_ts: 1000,
    });
    assert_eq!(app.agents.tmux_outputs.get(&TaskId(1)).unwrap(), "hello");
    assert!(cmds.is_empty());
}

#[test]
fn tmux_output_overwrites_previous() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)], TEST_TIMEOUT);
    app.update(Message::TmuxOutput {
        id: TaskId(1),
        output: "first".to_string(),
        activity_ts: 1000,
    });
    app.update(Message::TmuxOutput {
        id: TaskId(1),
        output: "second".to_string(),
        activity_ts: 1001,
    });
    assert_eq!(app.agents.tmux_outputs.get(&TaskId(1)).unwrap(), "second");
}

#[test]
fn refresh_tasks_replaces_and_clamps() {
    let mut app = make_app();
    app.selection_mut().set_row(0, 1); // row 1 of Backlog (has 2 items)
    app.update(Message::RefreshTasks(vec![make_task(
        10,
        TaskStatus::Backlog,
    )]));
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(10));
    assert_eq!(app.selection().row(0), 0); // clamped from 1 to 0
}

#[test]
fn refresh_tasks_empty_clamps_all_rows_to_zero() {
    let mut app = make_app();
    app.selection_mut().set_row(0, 1);
    app.selection_mut().set_row(1, 1);
    app.update(Message::RefreshTasks(vec![]));
    assert!(app.board.tasks.is_empty());
    assert!(app.selection().selected_row.iter().all(|&r| r == 0));
}

// --- Key actions on Review status ---

#[test]
fn d_key_on_review_with_window_shows_warning() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = Some("task-5".to_string());
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("already running"));
}

#[test]
fn d_key_on_review_no_window_with_worktree_resumes() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(&cmds[0], Command::Resume { .. }));
}

#[test]
fn d_key_on_review_no_worktree_no_window_shows_warning() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = None;
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No worktree"));
}

// --- Actions on empty columns ---

#[test]
fn d_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
}

#[test]
fn g_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds.is_empty());
}

#[test]
fn m_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert!(cmds.is_empty());
}

#[test]
fn shift_m_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('M')));
    assert!(cmds.is_empty());
}

#[test]
fn e_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
}

// --- action_hints ---

#[test]
fn action_hints_backlog_task() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        keys.contains(&"[d]"),
        "should have dispatch/brainstorm hint"
    );
    assert!(keys.contains(&"[e]"), "should have edit hint");
    assert!(keys.contains(&"[m]"), "should have move hint");
    assert!(!keys.contains(&"[M]"), "backlog has no back movement");
    assert!(keys.contains(&"[x]"), "should have archive hint");
    assert!(keys.contains(&"[n]"), "should have new hint");
    assert!(keys.contains(&"[q]"), "should have quit hint");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("brainstorm"),
        "backlog dispatch means brainstorm"
    );
}

#[test]
fn action_hints_backlog_task_with_plan() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[d]"), "should have dispatch hint");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("ispatch"),
        "backlog with plan dispatch means dispatch"
    );
}

#[test]
fn action_hints_running_with_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("win-4".to_string());
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[g]"), "should have go-to-session hint");
    assert!(
        !keys.contains(&"[d]"),
        "should not have dispatch/resume when window exists"
    );
}

#[test]
fn action_hints_running_with_worktree_no_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/tmp/wt".to_string());
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[d]"), "should have resume hint");
    assert!(!keys.contains(&"[g]"), "no go-to-session without window");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("resume"), "d means resume here");
}

#[test]
fn action_hints_running_no_worktree_no_window() {
    let task = make_task(4, TaskStatus::Running);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        !keys.contains(&"[d]"),
        "no dispatch/resume without worktree"
    );
    assert!(!keys.contains(&"[g]"), "no go-to-session without window");
    assert!(keys.contains(&"[e]"), "still has edit");
}

#[test]
fn action_hints_review_with_window() {
    let mut task = make_task(6, TaskStatus::Review);
    task.tmux_window = Some("win-6".to_string());
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        keys.contains(&"[g]"),
        "review with window shows go-to-session"
    );
}

#[test]
fn action_hints_done_task() {
    let task = make_task(5, TaskStatus::Done);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[e]"), "done has edit");
    assert!(keys.contains(&"[M]"), "done has back");
    assert!(keys.contains(&"[x]"), "done has archive");
    assert!(!keys.contains(&"[m]"), "done has no forward move");
    assert!(!keys.contains(&"[d]"), "done has no dispatch");
}

#[test]
fn action_hints_no_task() {
    let hints = ui::action_hints(None, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[n]"), "no-task shows new");
    assert!(keys.contains(&"[q]"), "no-task shows quit");
    assert!(!keys.contains(&"[d]"), "no-task has no dispatch");
    assert!(!keys.contains(&"[e]"), "no-task has no edit");
}

// --- epic_action_hints ---

#[test]
fn epic_action_hints_not_done() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Enter]"), "epic shows detail");
    assert!(keys.contains(&"[m]"), "epic shows status forward");
    assert!(keys.contains(&"[M]"), "epic shows status backward");
    assert!(keys.contains(&"[x]"), "epic shows archive");
    assert!(keys.contains(&"[q]"), "epic shows quit");
}

#[test]
fn epic_action_hints_done() {
    let mut epic = make_epic(1);
    epic.status = TaskStatus::Done;
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[m]"), "done epic shows status forward");
    assert!(keys.contains(&"[M]"), "done epic shows status backward");
}

// --- action_hints: missing hints ---

/// Extract bold key spans (like "[d]", "[Tab]") from hint spans.
fn hint_keys<'a>(hints: &'a [ratatui::text::Span<'static>]) -> Vec<&'a str> {
    hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect()
}

#[test]
fn action_hints_backlog_shows_enter_detail() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[Enter]"), "should show Enter/detail hint");
}

#[test]
fn action_hints_shows_filter_help() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[f]"), "should show filter hint");
    assert!(keys.contains(&"[?]"), "should show help hint");
}

#[test]
fn action_hints_shows_copy_and_split() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[c]"), "should show copy hint");
    assert!(keys.contains(&"[S]"), "should show split hint");
}

#[test]
fn action_hints_no_ctrl_g_outside_epic() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[^g]"),
        "should not show ^g back outside epic view"
    );
}

// --- epic_action_hints: missing hints ---

#[test]
fn epic_action_hints_shows_filter_help() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[f]"), "epic should show filter hint");
    assert!(keys.contains(&"[?]"), "epic should show help hint");
}

// --- review_action_hints: missing hints ---

#[test]
fn review_hints_shows_backtab_and_filter() {
    let hints = ui::review_action_hints(true, false, None);
    let keys = hint_keys(&hints);
    assert!(
        keys.contains(&"[BackTab]"),
        "review should show BackTab hint"
    );
    assert!(keys.contains(&"[f]"), "review should show filter hint");
}

// --- bot_action_hints: missing hints ---

#[test]
fn bot_hints_shows_approve_merge_when_selected() {
    let hints = ui::bot_action_hints(true, None, true);
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[A]"), "bot should show batch approve hint");
    assert!(keys.contains(&"[m]"), "bot should show batch merge hint");
}

#[test]
fn bot_hints_hides_approve_merge_when_no_selection() {
    let hints = ui::bot_action_hints(true, None, false);
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[A]"),
        "bot should not show approve without selection"
    );
    assert!(
        !keys.contains(&"[m]"),
        "bot should not show merge without selection"
    );
}

#[test]
fn bot_hints_shows_filter_queries_help_quit_backtab() {
    let hints = ui::bot_action_hints(true, None, false);
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[f]"), "bot should show filter hint");
    assert!(keys.contains(&"[e]"), "bot should show edit queries hint");
    assert!(keys.contains(&"[?]"), "bot should show help hint");
    assert!(keys.contains(&"[q]"), "bot should show quit hint");
    assert!(keys.contains(&"[BackTab]"), "bot should show BackTab hint");
}

// --- security_action_hints: missing hints ---

#[test]
fn security_hints_shows_quit() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.view_mode = ViewMode::SecurityBoard {
        selection: crate::tui::types::SecurityBoardSelection::new(),
        saved_board: crate::tui::types::BoardSelection::default(),
    };
    let hints = ui::security_action_hints(&app, false, None);
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[q]"), "security should show quit hint");
}

// --- Edit key ---

#[test]
fn e_key_enters_confirm_edit_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
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
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    app.handle_key(make_key(KeyCode::Char('e')));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EditTaskInEditor(t) if t.id == TaskId(1)));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn e_key_confirm_n_cancels() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    app.handle_key(make_key(KeyCode::Char('e')));
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
}

#[test]
fn new_app_has_empty_agent_tracking() {
    let app = App::new(vec![], TEST_TIMEOUT);
    // stale/crashed state is now on the task's sub_status field, not in AgentTracking
    assert!(app.agents.prev_tmux_activity.is_empty());
}

#[test]
fn kill_and_retry_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;

    app.update(Message::KillAndRetry(TaskId(4)));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn retry_resume_emits_kill_and_resume() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::RetryResume(TaskId(4)));

    // After retry resume, sub_status is no longer stale/crashed
    assert!(!app.is_stale(TaskId(4)));
    assert!(!app.is_crashed(TaskId(4)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::KillTmuxWindow { .. })));
    assert!(cmds.iter().any(|c| matches!(c, Command::Resume { .. })));
}

#[test]
fn retry_fresh_emits_cleanup_and_dispatch() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::RetryFresh(TaskId(4)));

    assert!(!app.is_stale(TaskId(4)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { .. })));
}

#[test]
fn d_key_on_stale_running_task_enters_retry_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    // Navigate to Running column (index 1)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn d_key_on_crashed_running_task_enters_retry_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    // Navigate to Running column (index 1)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn confirm_retry_r_key_emits_resume() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(c, Command::Resume { .. })));
}

#[test]
fn confirm_retry_f_key_emits_fresh() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
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
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty());
}

// --- Message-level tests for new input routing handlers ---

#[test]
fn dismiss_error_clears_popup() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.status.error_popup = Some("boom".to_string());
    app.update(Message::DismissError);
    assert!(app.status.error_popup.is_none());
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::SubmitTitle(String::new()));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.task_draft.is_none());
}

#[test]
fn submit_tag_advances_to_description() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    let cmds = app.update(Message::SubmitTag(Some(TaskTag::Bug)));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::OpenDescriptionEditor { is_epic: false }
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
fn description_editor_result_for_epic() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: String::new(),
        repo_path: String::new(),
    });
    app.update(Message::DescriptionEditorResult(
        "epic desc\nline 2".to_string(),
    ));
    assert_eq!(app.input.mode, InputMode::InputEpicRepoPath);
    assert_eq!(
        app.input.epic_draft.as_ref().unwrap().description,
        "epic desc\nline 2"
    );
}

#[test]
fn submit_repo_path_advances_to_base_branch() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
fn input_char_appends_to_buffer() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::InputChar('H'));
    app.update(Message::InputChar('i'));
    assert_eq!(app.input.buffer, "Hi");
}

#[test]
fn start_repo_filter_enters_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.update(Message::StartRepoFilter);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn toggle_repo_filter_adds_and_removes() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.update(Message::ToggleRepoFilter("/repo-a".to_string()));
    assert!(app.filter.repos.contains("/repo-a"));
    assert!(!app.filter.repos.contains("/repo-b"));

    app.update(Message::ToggleRepoFilter("/repo-a".to_string()));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn toggle_all_repo_filter_selects_all_then_clears() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    // Toggle all on
    app.update(Message::ToggleAllRepoFilter);
    assert_eq!(app.filter.repos.len(), 2);

    // Toggle all off
    app.update(Message::ToggleAllRepoFilter);
    assert!(app.filter.repos.is_empty());
}

#[test]
fn close_repo_filter_returns_to_normal() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.update(Message::CloseRepoFilter);
    assert_eq!(app.input.mode, InputMode::Normal);
    // Should emit PersistStringSetting
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistStringSetting { .. })));
}

#[test]
fn input_backspace_removes_last_char() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.buffer = "abc".to_string();
    app.update(Message::InputBackspace);
    assert_eq!(app.input.buffer, "ab");
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmDelete;
    app.status.message = Some("Delete \"Task 1\" [backlog]? [y/n]".to_string());
    app.update(Message::CancelDelete);
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
}

#[test]
fn status_info_sets_message() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::StatusInfo("hello".to_string()));
    assert_eq!(app.status.message.as_deref(), Some("hello"));
}

#[test]
fn start_quick_dispatch_selection_enters_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::StartQuickDispatchSelection);
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert!(app.status.message.is_some());
}

#[test]
fn select_quick_dispatch_repo_dispatches() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    let cmds = app.update(Message::SelectQuickDispatchRepo(1));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(
        |c| matches!(c, Command::QuickDispatch { ref draft, .. } if draft.repo_path == "/repo2")
    ));
}

#[test]
fn select_quick_dispatch_repo_out_of_range_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.update(Message::SelectQuickDispatchRepo(5));
    assert!(cmds.is_empty());
    // Mode is not changed by the handler (stays as-is)
}

#[test]
fn cancel_retry_returns_to_normal() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));
    app.status.message = Some("Agent stale".to_string());
    app.update(Message::CancelRetry);
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
}

// --- Archive ---

#[test]
fn archive_task_sets_status_and_emits_persist() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)], TEST_TIMEOUT);
    let cmds = app.update(Message::ArchiveTask(TaskId(1)));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Archived);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn archive_task_with_worktree_emits_cleanup() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/wt/1-test".to_string());
    task.tmux_window = Some("dev:1-test".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::ArchiveTask(TaskId(1)));

    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Archived);
    assert!(task.worktree.is_none());
    assert!(task.tmux_window.is_none());
}

#[test]
fn archive_task_without_worktree_no_cleanup() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    let cmds = app.update(Message::ArchiveTask(TaskId(1)));
    assert!(!cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn archive_clears_agent_tracking() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("dev:1-test".to_string());
    task.sub_status = SubStatus::Stale;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.agents
        .tmux_outputs
        .insert(TaskId(1), "output".to_string());
    app.agents.prev_tmux_activity.insert(TaskId(1), 1000);

    app.update(Message::ArchiveTask(TaskId(1)));

    // stale/crashed state is now on the task's sub_status field
    assert!(!app.agents.tmux_outputs.contains_key(&TaskId(1)));
    assert!(!app.agents.prev_tmux_activity.contains_key(&TaskId(1)));
}

// --- Archive panel key handling ---

#[test]
fn archive_panel_j_k_navigation() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Archived),
            make_task(2, TaskStatus::Archived),
            make_task(3, TaskStatus::Archived),
        ],
        TEST_TIMEOUT,
    );
    app.archive.visible = true;
    assert_eq!(app.archive.selected_row, 0);

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.archive.selected_row, 1);

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.archive.selected_row, 2);

    // Clamp at end
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.archive.selected_row, 2);

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.archive.selected_row, 1);
}

#[test]
fn archive_panel_h_closes() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;

    app.handle_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(!app.archive.visible);
}

#[test]
fn archive_panel_x_enters_confirm_delete() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmDelete);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete \"Task 1\"? [y/n]")
    );
}

#[test]
fn archive_panel_confirm_delete_removes_task() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;

    app.handle_key(make_key(KeyCode::Char('x')));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(app.board.tasks.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteTask(TaskId(1)))));
}

#[test]
fn archived_tasks_not_in_kanban_columns() {
    let app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Archived),
        ],
        TEST_TIMEOUT,
    );

    for &status in TaskStatus::ALL {
        let tasks = app.tasks_by_status(status);
        for t in &tasks {
            assert_ne!(
                t.status,
                TaskStatus::Archived,
                "archived task should not appear in {} column",
                status.as_str()
            );
        }
    }

    let archived = app.archived_tasks();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, TaskId(2));
}

// --- End-to-end archive flow ---

#[test]
fn full_archive_flow() {
    // Create a running task with worktree
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/wt/1-test".to_string());
    task.tmux_window = Some("dev:1-test".to_string());
    let mut app = App::new(vec![task, make_task(2, TaskStatus::Backlog)], TEST_TIMEOUT);

    // Navigate to Running column (column 1)
    app.handle_key(make_key(KeyCode::Right));

    // Press x to archive
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmArchive);

    // Confirm
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);

    // Task should be archived with cleanup
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Archived);
    assert!(task.worktree.is_none());
    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));

    // Toggle archive panel
    app.handle_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT));
    assert!(app.archive.visible);

    // Should see 1 archived task
    assert_eq!(app.archived_tasks().len(), 1);

    // Hard delete from archive
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmDelete);

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteTask(TaskId(1)))));
    assert!(app.archived_tasks().is_empty());
}

// -----------------------------------------------------------------------
// Batch selection tests
// -----------------------------------------------------------------------

#[test]
fn space_toggles_task_selection() {
    let mut app = make_app();
    // Select task 1 in Backlog
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.tasks.contains(&TaskId(1)));

    // Toggle off
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(!app.select.tasks.contains(&TaskId(1)));
}

#[test]
fn space_on_empty_column_is_noop() {
    let mut app = make_app();
    // Navigate to Review column (empty)
    app.update(Message::NavigateColumn(2));
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.tasks.is_empty());
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
fn batch_move_forward_moves_all_selected() {
    let mut app = make_app();
    // Select both Backlog tasks
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    // Press m to batch move forward
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));

    // Both should now be Running
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
    assert_eq!(
        app.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Running
    );
    // Should have PersistTask commands
    let persist_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::PersistTask(_)))
        .count();
    assert_eq!(persist_count, 2);
}

#[test]
fn batch_move_clears_selection() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    app.handle_key(make_key(KeyCode::Char('m')));

    assert!(app.select.tasks.is_empty());
}

#[test]
fn batch_move_multiple_steps() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    // Move Backlog -> Running (clears selection)
    app.handle_key(make_key(KeyCode::Char('m')));

    // Re-select and move Running -> Review
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));
    app.handle_key(make_key(KeyCode::Char('m')));

    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Review);
    assert_eq!(app.find_task(TaskId(2)).unwrap().status, TaskStatus::Review);
}

#[test]
fn batch_move_backward() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Done),
            make_task(2, TaskStatus::Done),
            make_task(3, TaskStatus::Done),
        ],
        TEST_TIMEOUT,
    );

    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    app.handle_key(make_key(KeyCode::Char('M')));

    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Review);
    assert_eq!(app.find_task(TaskId(2)).unwrap().status, TaskStatus::Review);
    // Task 3 not selected, should remain Done
    assert_eq!(app.find_task(TaskId(3)).unwrap().status, TaskStatus::Done);
}

#[test]
fn batch_archive_archives_all_and_clears_selection() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Done),
            make_task(2, TaskStatus::Done),
            make_task(3, TaskStatus::Backlog),
        ],
        TEST_TIMEOUT,
    );

    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    let cmds = app.update(Message::BatchArchiveTasks(vec![TaskId(1), TaskId(2)]));

    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Archived
    );
    assert_eq!(
        app.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Archived
    );
    assert_eq!(
        app.find_task(TaskId(3)).unwrap().status,
        TaskStatus::Backlog
    );
    // Selection should be cleared after archive
    assert!(app.select.tasks.is_empty());
    // Should have PersistTask commands
    let persist_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::PersistTask(_)))
        .count();
    assert_eq!(persist_count, 2);
}

#[test]
fn x_key_with_selection_shows_count_in_confirm() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmArchive);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Archive 2 items? [y/n]")
    );
}

#[test]
fn confirm_archive_with_selection_dispatches_batch() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Done),
            make_task(2, TaskStatus::Done),
        ],
        TEST_TIMEOUT,
    );

    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));
    app.input.mode = InputMode::ConfirmArchive;

    app.handle_key(make_key(KeyCode::Char('y')));

    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Archived
    );
    assert_eq!(
        app.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Archived
    );
    assert!(app.select.tasks.is_empty());
}

#[test]
fn single_task_operations_work_without_selection() {
    let mut app = make_app();
    assert!(app.select.tasks.is_empty());

    // Single move should still work
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
    assert!(!cmds.is_empty());
}

#[test]
fn refresh_tasks_prunes_stale_selections() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(99))); // non-existent

    // Refresh with only task 1
    app.update(Message::RefreshTasks(vec![make_task(
        1,
        TaskStatus::Backlog,
    )]));

    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(!app.select.tasks.contains(&TaskId(99)));
}

// ---------------------------------------------------------------------------
// Rendering tests
// ---------------------------------------------------------------------------

#[test]
fn render_empty_board_shows_all_column_headers() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "backlog"));
    assert!(buffer_contains(&buf, "running"));
    assert!(buffer_contains(&buf, "review"));
    assert!(buffer_contains(&buf, "done"));
}

#[test]
fn render_shows_task_titles_in_columns() {
    let tasks = vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Running),
        make_task(3, TaskStatus::Review),
    ];
    let mut app = App::new(tasks, TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "Task 1"));
    assert!(buffer_contains(&buf, "Task 2"));
    assert!(buffer_contains(&buf, "Task 3"));
}

#[test]
fn render_error_popup_shows_message() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::Error("Something went wrong".to_string()));
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "Something went wrong"));
}

#[test]
fn render_status_bar_shows_keybindings() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 200, 20);
    assert!(buffer_contains(&buf, "uit"));
}

#[test]
fn render_crashed_task_shows_label() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("win-1".to_string());
    task.sub_status = SubStatus::Crashed;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "crashed"));
}

#[test]
fn render_stale_task_shows_label() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("win-1".to_string());
    task.sub_status = SubStatus::Stale;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "stale"));
}

// ---------------------------------------------------------------------------
// ui.rs — detached indicator
// ---------------------------------------------------------------------------

#[test]
fn running_card_with_worktree_no_window_shows_detached() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ detached"), "expected '○ detached'");
}

#[test]
fn running_card_with_window_shows_running_not_detached() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some("1-fix".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "◉ running"), "expected '◉ running'");
    assert!(
        !buffer_contains(&buf, "detached"),
        "should not show detached"
    );
}

#[test]
fn crashed_card_with_no_window_shows_detached_not_crashed() {
    // Detached out-prioritizes Crashed when tmux_window is None
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Crashed;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ detached"), "expected '○ detached'");
    assert!(
        !buffer_contains(&buf, "\u{26a0} crashed"),
        "should not show ⚠ crashed"
    );
}

#[test]
fn review_card_with_pr_detached_shows_circle_prefix() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ PR #42"), "expected '○ PR #42'");
}

#[test]
fn review_card_with_pr_attached_shows_filled_circle() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some("1-fix".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "● PR #42"), "expected '● PR #42'");
}

#[test]
fn render_does_not_panic_on_small_terminal() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    // Very small terminal — should not panic
    let _ = render_to_buffer(&mut app, 20, 5);
}

#[test]
fn render_input_mode_shows_prompt() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::StartNewTask);
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "Title"));
}

#[test]
fn truncate_respects_max_length() {
    assert_eq!(ui::truncate("short", 10), "short");
    assert_eq!(
        ui::truncate("hello world this is long", 10).chars().count(),
        10
    );
    assert!(ui::truncate("hello world this is long", 10).ends_with('…'));
}

// ---------------------------------------------------------------------------
// Rendering tests — v2.0 cosmetic redesign
// ---------------------------------------------------------------------------

#[test]
fn render_v2_task_card_shows_stripe() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    // Cursor card uses thicker stripe ▌ (U+258C), non-cursor uses ▎ (U+258E)
    assert!(
        buffer_contains(&buf, "\u{258c}") || buffer_contains(&buf, "\u{258e}"),
        "task card should have stripe character"
    );
}

#[test]
fn render_v2_backlog_task_shows_status_icon() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{25e6}"),
        "backlog task should show \u{25e6} icon"
    );
}

#[test]
fn render_v2_running_task_shows_status_icon() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("win-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{25c9}"),
        "running task should show \u{25c9} icon"
    );
}

#[test]
fn render_v2_focused_column_shows_arrow() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    // Default focus is on first column (Backlog), should show \u{25b8}
    assert!(
        buffer_contains(&buf, "\u{25b8}"),
        "focused column should show \u{25b8} indicator"
    );
}

#[test]
fn render_v2_unfocused_columns_show_dot() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    // Unfocused columns should show \u{25e6}
    assert!(
        buffer_contains(&buf, "\u{25e6}"),
        "unfocused columns should show \u{25e6} indicator"
    );
}

#[test]
fn render_v2_detail_panel_shows_inline_metadata() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.update(Message::ToggleDetail);
    let buf = render_to_buffer(&mut app, 120, 20);
    // The compact detail panel shows "title \u{00b7} #id \u{00b7} status \u{00b7} repo" on one line
    // Check for the middle-dot separator which is new in v2
    assert!(
        buffer_contains(&buf, "\u{00b7}"),
        "detail panel should use \u{00b7} separator"
    );
    assert!(
        buffer_contains(&buf, "#1"),
        "detail panel should show task ID with # prefix"
    );
}

#[test]
fn render_status_bar_uses_bracket_format() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 220, 20);
    // Hints should use [key] bracket format
    assert!(
        buffer_contains(&buf, "[n]"),
        "status bar should use [key] bracket format"
    );
    assert!(
        buffer_contains(&buf, "[q]"),
        "status bar should use [key] bracket format"
    );
    // Should also contain the action words (embedded format: [n]ew, [q]uit)
    assert!(
        buffer_contains(&buf, "[n]ew"),
        "status bar should show 'new' hint"
    );
    assert!(
        buffer_contains(&buf, "[q]uit"),
        "status bar should show 'quit' hint"
    );
}

#[test]
fn render_v2_done_task_shows_checkmark() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)], TEST_TIMEOUT);
    // Navigate to Done column (index 3)
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{2713}"),
        "done task should show \u{2713} icon"
    );
}

// ---------------------------------------------------------------------------
// Rendering tests — layout correctness
// ---------------------------------------------------------------------------

#[test]
fn render_columns_appear_left_to_right() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 30);

    // Find the leftmost x-position where each header appears
    let headers = ["backlog", "running", "review", "done"];
    let mut positions: Vec<Option<u16>> = Vec::new();
    for header in &headers {
        let mut found = None;
        for y in 0..2u16 {
            for x in 0..120u16 {
                let remaining = (120 - x) as usize;
                if remaining < header.len() {
                    continue;
                }
                let segment: String = (0..header.len() as u16)
                    .map(|dx| buf[(x + dx, y)].symbol().to_string())
                    .collect();
                if segment == *header {
                    found = Some(x);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        positions.push(found);
    }

    // All headers must render
    for (i, header) in headers.iter().enumerate() {
        assert!(
            positions[i].is_some(),
            "column header '{header}' not found in rendered output"
        );
    }

    // Verify strict left-to-right ordering
    let xs: Vec<u16> = positions.into_iter().flatten().collect();
    for pair in xs.windows(2) {
        assert!(
            pair[0] < pair[1],
            "columns must be ordered left to right, got positions: {xs:?}"
        );
    }
}

#[test]
fn render_columns_fill_terminal_width() {
    // Regression test: columns must use the full terminal width, not leave a gap on the right.
    // A previous bug reserved a 34-char right sidebar in the column content area.
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)], TEST_TIMEOUT);
    let width: u16 = 120;
    let buf = render_to_buffer(&mut app, width, 20);

    // Find the rightmost x-position where "done" header text appears
    let header = "done";
    let mut header_x = None;
    'outer: for y in 0..3u16 {
        for x in (0..width).rev() {
            let remaining = (width - x) as usize;
            if remaining < header.len() {
                continue;
            }
            let segment: String = (0..header.len() as u16)
                .map(|dx| buf[(x + dx, y)].symbol().to_string())
                .collect();
            if segment == header {
                header_x = Some(x);
                break 'outer;
            }
        }
    }
    let done_col_x = header_x.expect("'done' column header not found");

    // The "done" column header should be centered in the last quarter of the terminal.
    // With 4 columns at width=120, each column is 30 chars wide, so the last column
    // starts at x=90. The header should be somewhere after x=90.
    // If the old bug exists (34-char sidebar), each column is only ~21 chars and the
    // header would be well before x=90.
    let expected_min_x = width * 3 / 4;
    assert!(
        done_col_x >= expected_min_x,
        "last column header 'done' at x={done_col_x}, expected >= {expected_min_x} — \
         columns are not filling the terminal width"
    );
}

#[test]
fn render_help_overlay_shows_keybindings_help() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::ToggleHelp);
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Navigation"),
        "help overlay should show Navigation section"
    );
    assert!(
        buffer_contains(&buf, "Actions"),
        "help overlay should show Actions section"
    );
}

#[test]
fn render_help_overlay_in_review_board_shows_review_shortcuts() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::ToggleHelp);
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Review Board"),
        "review help should have Review Board section"
    );
    assert!(
        buffer_contains(&buf, "open PR"),
        "review help should mention open PR"
    );
    assert!(
        buffer_contains(&buf, "dispatch review agent"),
        "review help should mention dispatch review agent"
    );
    assert!(
        !buffer_contains(&buf, "new task"),
        "review help should not show task board new task key"
    );
}

#[test]
fn render_1x1_terminal_does_not_panic() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)], TEST_TIMEOUT);
    let _ = render_to_buffer(&mut app, 1, 1);
}

#[test]
fn render_archive_overlay_shows_archived_tasks() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.status = TaskStatus::Archived;
    task.title = "Archived Item".to_string();
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::ToggleArchive);
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Archived Item"),
        "archive overlay should show archived task title"
    );
}

// ---------------------------------------------------------------------------
// Stress tests
// ---------------------------------------------------------------------------

#[test]
fn stress_large_task_list_navigation() {
    let tasks: Vec<_> = (1..=1000)
        .map(|i| make_task(i, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks, TEST_TIMEOUT);

    assert_eq!(app.board.tasks.len(), 1000);

    // Navigate through all rows
    for _ in 0..999 {
        app.update(Message::NavigateRow(1));
    }
    assert_eq!(app.selected_row()[0], 999);

    // Navigate back
    for _ in 0..999 {
        app.update(Message::NavigateRow(-1));
    }
    assert_eq!(app.selected_row()[0], 0);
}

#[test]
fn stress_large_task_list_rendering() {
    let mut tasks: Vec<_> = (1..=200)
        .map(|i| make_task(i, TaskStatus::Backlog))
        .collect();
    // Spread tasks across all columns
    for (i, task) in tasks.iter_mut().enumerate() {
        task.status = match i % 4 {
            0 => TaskStatus::Backlog,
            1 => TaskStatus::Running,
            2 => TaskStatus::Review,
            _ => TaskStatus::Done,
        };
    }
    let mut app = App::new(tasks, TEST_TIMEOUT);

    // Render at various sizes — must not panic
    for width in [40, 80, 120, 200] {
        for height in [10, 24, 50] {
            let _ = render_to_buffer(&mut app, width, height);
        }
    }
}

#[test]
fn stress_rapid_status_transitions() {
    let tasks = vec![make_task(1, TaskStatus::Backlog)];
    let mut app = App::new(tasks, TEST_TIMEOUT);

    // Rapidly move task through all statuses and back.
    // Moving forward will stop at Review because Done requires confirmation.
    for _ in 0..100 {
        app.update(Message::MoveTask {
            id: TaskId(1),
            direction: MoveDirection::Forward,
        });
    }
    // Should be at Review (blocked by Done confirmation)
    assert_eq!(app.board.tasks[0].status, TaskStatus::Review);
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(1))));

    // Confirm the Done transition
    app.update(Message::ConfirmDone);
    assert_eq!(app.board.tasks[0].status, TaskStatus::Done);

    for _ in 0..100 {
        app.update(Message::MoveTask {
            id: TaskId(1),
            direction: MoveDirection::Backward,
        });
    }
    // Should be at Backlog (clamped)
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
}

#[test]
fn stress_db_with_many_tasks() {
    let db = crate::db::Database::open_in_memory().unwrap();
    use crate::db::TaskCrud;
    for i in 0..500 {
        db.create_task(
            &format!("Task {i}"),
            "stress test",
            "/repo",
            None,
            TaskStatus::Backlog,
            "main",
        )
        .unwrap();
    }
    let tasks = db.list_all().unwrap();
    assert_eq!(tasks.len(), 500);

    // Create app from DB tasks and verify navigation works
    let mut app = App::new(tasks, TEST_TIMEOUT);
    for _ in 0..499 {
        app.update(Message::NavigateRow(1));
    }
    assert_eq!(app.selected_row()[0], 499);
}

// --- Epic helpers ---

fn make_epic(id: i64) -> Epic {
    let now = chrono::Utc::now();
    Epic {
        id: EpicId(id),
        title: format!("Epic {id}"),
        description: String::new(),
        repo_path: "/repo".to_string(),
        status: TaskStatus::Backlog,
        plan_path: None,
        sort_order: None,
        auto_dispatch: true,
        created_at: now,
        updated_at: now,
    }
}

// --- tasks_for_current_view ---

#[test]
fn tasks_for_current_view_board_excludes_epic_tasks() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(1));
}

#[test]
fn tasks_for_current_view_epic_shows_only_subtasks() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];

    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(2));
}

// --- enter/exit epic ---

#[test]
fn enter_on_epic_toggles_detail() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Epic is at row 0 in Backlog column (no standalone tasks)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    assert!(!app.board.detail_visible);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(
        app.board.detail_visible,
        "Enter on epic should toggle detail panel"
    );
    assert!(
        matches!(app.board.view_mode, ViewMode::Board(_)),
        "Should stay in board view"
    );
}

#[test]
fn e_on_epic_opens_editor() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(matches!(&cmds[0], Command::EditEpicInEditor(e) if e.id == EpicId(10)));
}

#[test]
fn enter_on_task_still_toggles_detail() {
    let mut app = make_app();
    assert!(!app.board.detail_visible);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(
        app.board.detail_visible,
        "Enter on task should still toggle detail"
    );
}

#[test]
fn e_on_task_enters_confirm_then_edits() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmEditTask(_)));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::EditTaskInEditor(_))));
}

#[test]
fn enter_epic_switches_to_epic_view() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(2);

    app.update(Message::EnterEpic(EpicId(10)));

    match &app.board.view_mode {
        ViewMode::Epic {
            epic_id,
            saved_board,
            ..
        } => {
            assert_eq!(*epic_id, EpicId(10));
            assert_eq!(
                saved_board.column(),
                2,
                "board selection should be preserved"
            );
        }
        _ => panic!("Expected ViewMode::Epic"),
    }
}

#[test]
fn exit_epic_restores_board_selection() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(3);

    app.update(Message::EnterEpic(EpicId(10)));
    app.selection_mut().set_column(1);

    app.update(Message::ExitEpic);

    match &app.board.view_mode {
        ViewMode::Board(sel) => {
            assert_eq!(sel.column(), 3, "board selection should be restored");
        }
        _ => panic!("Expected ViewMode::Board"),
    }
}

#[test]
fn exit_epic_when_on_board_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::ExitEpic);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

// --- ColumnItem ---

#[test]
fn column_items_board_view_includes_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)]; // epic with no subtasks = Backlog

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 2); // 1 task + 1 epic
                                // Same priority (5), so task (id=1) sorts before epic (id=10)
    assert!(matches!(items[0], ColumnItem::Task(_)));
    assert!(matches!(items[1], ColumnItem::Epic(_)));
}

#[test]
fn column_items_epic_view_no_epics() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    app.board.epics = vec![make_epic(10)];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(items.iter().all(|i| matches!(i, ColumnItem::Task(_))));
}

#[test]
fn selected_column_item_returns_epic() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    // Same priority (5), task (id=1) at row 0, epic (id=10) at row 1
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1);

    match app.selected_column_item() {
        Some(ColumnItem::Epic(e)) => assert_eq!(e.id, EpicId(10)),
        other => panic!("Expected Epic, got {:?}", other),
    }
}

// --- Epic CRUD ---

#[test]
fn start_new_epic_sets_input_mode() {
    let mut app = make_app();
    app.update(Message::StartNewEpic);
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);
}

#[test]
fn epic_created_adds_to_state() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(1);
    app.update(Message::EpicCreated(epic));
    assert_eq!(app.board.epics.len(), 1);
}

#[test]
fn delete_epic_removes_from_state_and_tasks() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask, make_task(2, TaskStatus::Backlog)];

    let cmds = app.update(Message::DeleteEpic(EpicId(10)));
    assert!(app.board.epics.is_empty());
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(2));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn move_epic_status_forward() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)]; // starts as Backlog
    let cmds = app.update(Message::MoveEpicStatus(EpicId(10), MoveDirection::Forward));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Running),
            ..
        }
    )));
}

#[test]
fn move_epic_status_backward() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    let cmds = app.update(Message::MoveEpicStatus(EpicId(10), MoveDirection::Backward));
    assert_eq!(app.board.epics[0].status, TaskStatus::Review);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Review),
            ..
        }
    )));
}

// ---------------------------------------------------------------------------
// input.rs — Normal mode: Epic interactions
// ---------------------------------------------------------------------------

/// Helper: create an app with one task + one epic in Backlog, cursor on the epic.
fn make_app_with_epic_selected() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Same priority (5), task (id=1) at row 0, epic (id=10) at row 1
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1);
    app
}

#[test]
fn m_key_on_epic_moves_status_forward() {
    let mut app = make_app_with_epic_selected();
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistEpic { .. })));
}

#[test]
fn shift_m_key_on_backlog_epic_stays_backlog() {
    let mut app = make_app_with_epic_selected();
    let cmds = app.handle_key(make_key(KeyCode::Char('M')));
    // Already at Backlog, can't go backward
    assert_eq!(app.board.epics[0].status, TaskStatus::Backlog);
    assert!(cmds.is_empty());
}

#[test]
fn shift_m_on_done_epic_moves_to_review() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Done);
            t.epic_id = Some(EpicId(10));
            t
        }],
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    // Done epic → column 3
    app.selection_mut().set_column(3);
    app.selection_mut().set_row(3, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('M')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Review);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Review),
            ..
        }
    )));
}

#[test]
fn shift_e_key_starts_new_epic() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('E')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);
}

#[test]
fn shift_v_key_on_task_is_noop() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('V')));
    assert!(cmds.is_empty());
}

#[test]
fn shift_v_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('V')));
    assert!(cmds.is_empty());
}

#[test]
fn x_key_on_epic_enters_confirm_archive_epic() {
    let mut app = make_app_with_epic_selected();
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmArchiveEpic);
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("Archive epic"));
}

#[test]
fn x_key_on_epic_with_non_done_subtasks_rejects_archive() {
    let mut app = App::new(
        vec![
            {
                let mut t = make_task(1, TaskStatus::Backlog);
                t.epic_id = Some(EpicId(10));
                t
            },
            {
                let mut t = make_task(2, TaskStatus::Running);
                t.epic_id = Some(EpicId(10));
                t
            },
        ],
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];
    // Subtasks are hidden in board view. Epic status is Running (col 1).
    // Epic is the only item in Running column → row 0.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("Cannot archive epic"));
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("2 subtasks not done"));
}

#[test]
fn x_key_on_epic_with_mixed_subtasks_rejects_archive_with_count() {
    let mut app = App::new(
        vec![
            {
                let mut t = make_task(1, TaskStatus::Done);
                t.epic_id = Some(EpicId(10));
                t
            },
            {
                let mut t = make_task(2, TaskStatus::Done);
                t.epic_id = Some(EpicId(10));
                t
            },
            {
                let mut t = make_task(3, TaskStatus::Running);
                t.epic_id = Some(EpicId(10));
                t
            },
        ],
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];
    // 2 Done + 1 Running → epic status Running (col 1). Epic is only item → row 0.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("1 subtask not done"));
}

#[test]
fn x_key_on_epic_with_all_done_subtasks_allows_archive() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Done);
            t.epic_id = Some(EpicId(10));
            t
        }],
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    // All done → epic status Done (column 3). Epic is only item → row 0.
    app.selection_mut().set_column(3);
    app.selection_mut().set_row(3, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmArchiveEpic);
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("Archive epic"));
}

#[test]
fn confirm_archive_epic_no_subtasks_allows_archive() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // No subtasks → derived status Backlog (col 0). Epic is only item → row 0.
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    let cmds = app.update(Message::ConfirmArchiveEpic);
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmArchiveEpic);
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("Archive epic"));
}

#[test]
fn g_key_on_epic_from_board_enters_epic_view() {
    let mut app = make_app_with_epic_selected();
    app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Epic {
            epic_id: EpicId(10),
            ..
        }
    ));
}

#[test]
fn e_key_in_epic_view_edits_epic() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EditEpicInEditor(e) if e.id == EpicId(10)));
}

#[test]
fn e_key_on_task_in_epic_view_edits_task_not_epic() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask];
    app.update(Message::EnterEpic(EpicId(10)));

    // Cursor on the subtask in the Backlog column (col 0, row 0)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmEditTask(TaskId(1))
    ));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(cmds.len(), 1, "expected exactly one command");
    assert!(
        matches!(&cmds[0], Command::EditTaskInEditor(t) if t.id == TaskId(1)),
        "expected EditTaskInEditor(task 1), got {:?}",
        cmds
    );
}

#[test]
fn esc_in_epic_view_exits_to_board() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn d_key_on_backlog_epic_dispatches_epic() {
    let mut app = make_app_with_epic_selected(); // epic at row 1 in Backlog
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::DispatchEpic { ref epic } if epic.id == EpicId(10)));
}

#[test]
fn d_key_in_epic_view_with_no_subtasks_dispatches_epic() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.update(Message::EnterEpic(EpicId(10)));

    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchEpic { ref epic } if epic.id == EpicId(10))));
}

// ---------------------------------------------------------------------------
// ToggleEpicAutoDispatch — U key in epic view
// ---------------------------------------------------------------------------

#[test]
fn shift_u_in_epic_view_toggles_auto_dispatch() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(42);
    epic.auto_dispatch = true;
    app.board.epics = vec![epic];

    // Enter epic view
    app.update(Message::EnterEpic(EpicId(42)));

    // Press Shift+U — should return ToggleEpicAutoDispatch command with auto_dispatch = false
    let cmds = app.handle_key(make_key(KeyCode::Char('U')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::ToggleEpicAutoDispatch {
            id: EpicId(42),
            auto_dispatch: false
        }
    )));

    // Also verify in-memory state was updated
    assert!(!app.board.epics[0].auto_dispatch);
}

// ---------------------------------------------------------------------------
// DispatchEpic message
// ---------------------------------------------------------------------------

#[test]
fn dispatch_epic_on_backlog_epic_produces_command() {
    let mut app = make_app_with_epic_selected(); // epic at row 1, Backlog column
    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::DispatchEpic { ref epic } if epic.id == EpicId(10)));
}

#[test]
fn dispatch_epic_on_non_backlog_shows_status() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Running);
            t.epic_id = Some(EpicId(10));
            t
        }],
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    // Epic status is Running (not Backlog) — dispatch should be rejected
    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_ref()
        .unwrap()
        .contains("No backlog tasks"));
}

#[test]
fn split_focused_defaults_to_true() {
    let app = make_app();
    assert!(app.split_focused());
}

#[test]
fn focus_changed_updates_split_focused_when_split_active() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("pane1".to_string());

    let cmds = app.update(Message::FocusChanged(false));
    assert!(cmds.is_empty());
    assert!(!app.split_focused());

    let cmds = app.update(Message::FocusChanged(true));
    assert!(cmds.is_empty());
    assert!(app.split_focused());
}

#[test]
fn split_pane_opened_resets_focused_to_true() {
    let mut app = make_app();
    // Simulate having lost focus before entering split
    app.board.split.focused = false;

    let _cmds = app.update(Message::SplitPaneOpened {
        pane_id: "pane1".to_string(),
        task_id: None,
    });
    assert!(app.split_active());
    assert!(app.split_focused());
}

#[test]
fn split_pane_closed_resets_focused_to_true() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("pane1".to_string());
    app.board.split.focused = false;

    let _cmds = app.update(Message::SplitPaneClosed);
    assert!(!app.split_active());
    assert!(app.split_focused());
}

#[test]
fn render_shows_border_when_split_active_and_focused() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.focused = true;
    app.board.split.right_pane_id = Some("pane1".to_string());

    let buf = render_to_buffer(&mut app, 80, 24);
    // Top-left corner should be a border character (┌)
    assert_eq!(
        buf[(0, 0)].symbol(),
        "┌",
        "Expected border corner when split active"
    );
}

#[test]
fn render_no_border_when_split_inactive() {
    let mut app = make_app();
    assert!(!app.split_active());

    let buf = render_to_buffer(&mut app, 80, 24);
    // Top-left corner should NOT be a border character
    assert_ne!(
        buf[(0, 0)].symbol(),
        "┌",
        "No border expected when split inactive"
    );
}

#[test]
fn focus_changed_ignored_when_split_inactive() {
    let mut app = make_app();
    assert!(!app.split_active());

    let cmds = app.update(Message::FocusChanged(false));
    assert!(cmds.is_empty());
    assert!(app.split_focused()); // still true — ignored
}

#[test]
fn dispatch_epic_with_plan_dispatches_next_backlog_subtask() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plan.md".to_string());
    app.board.epics = vec![epic];

    // Add two backlog subtasks for this epic
    let mut task1 = make_task(1, TaskStatus::Backlog);
    task1.epic_id = Some(EpicId(10));
    task1.plan_path = Some("plan1.md".to_string());
    let mut task2 = make_task(2, TaskStatus::Backlog);
    task2.epic_id = Some(EpicId(10));

    app.board.tasks = vec![task1.clone(), task2];

    // Select the epic (only item in backlog column at row 0)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    // Should dispatch task1 (first backlog subtask, has plan)
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::DispatchAgent { ref task, .. } if task.id == TaskId(1)));
}

#[test]
fn dispatch_epic_with_plan_brainstorms_subtask_without_plan() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plan.md".to_string());
    app.board.epics = vec![epic];

    // Subtask without a plan, tagged as "epic" to trigger brainstorm
    let mut task1 = make_task(1, TaskStatus::Backlog);
    task1.epic_id = Some(EpicId(10));
    task1.tag = Some(TaskTag::Epic);
    app.board.tasks = vec![task1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(cmds[0], Command::DispatchAgent { ref task, mode: DispatchMode::Brainstorm } if task.id == TaskId(1))
    );
}

#[test]
fn dispatch_epic_with_plan_no_backlog_subtasks_shows_status() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plan.md".to_string());
    app.board.epics = vec![epic];

    // Only an archived subtask — archived tasks are excluded from epic_status
    // so the epic stays Backlog, but there are no backlog subtasks to dispatch
    let mut task1 = make_task(1, TaskStatus::Archived);
    task1.epic_id = Some(EpicId(10));
    app.board.tasks = vec![task1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------------------
// input.rs — Normal mode: Arrow key variants
// ---------------------------------------------------------------------------

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
    app.selection_mut().set_column(0); // Backlog has 2 tasks
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.selection().row(0), 1);
}

#[test]
fn up_arrow_navigates_row() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1);
    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.selection().row(0), 0);
}

// ---------------------------------------------------------------------------
// input.rs — handle_key_epic_text_input
// ---------------------------------------------------------------------------

#[test]
fn epic_title_esc_cancels() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "partial".to_string();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
}

#[test]
fn epic_title_enter_with_text_advances_to_description() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "My Epic".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicDescription);
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.epic_draft.as_ref().unwrap().title, "My Epic");
}

#[test]
fn epic_title_enter_empty_cancels() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn epic_description_enter_advances_to_repo_path() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        ..Default::default()
    });
    app.input.buffer = "epic desc".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicRepoPath);
    assert!(app.input.buffer.is_empty());
    assert_eq!(
        app.input.epic_draft.as_ref().unwrap().description,
        "epic desc"
    );
}

#[test]
fn epic_repo_path_enter_with_text_completes() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer = "/tmp".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::InsertEpic(ref d) if d.repo_path == "/tmp")));
}

#[test]
fn epic_repo_path_enter_empty_uses_saved_path() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::InsertEpic(ref d) if d.repo_path == "/tmp")));
}

#[test]
fn epic_repo_path_enter_empty_no_saved_stays() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec![];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    let _cmds = app.handle_key(make_key(KeyCode::Enter));
    // Should stay in repo path mode since there's no fallback
    assert!(app.status.message.is_some());
}

#[test]
fn epic_text_input_char_appends() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.handle_key(make_key(KeyCode::Char('A')));
    app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(app.input.buffer, "Ab");
}

#[test]
fn epic_text_input_backspace_removes() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "abc".to_string();
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn epic_text_input_unrecognized_key_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "x".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "x");
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);
}

#[test]
fn epic_repo_path_digit_quick_selects() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/first".to_string(), "/second".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer.clear();
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::InsertEpic(ref d) if d.repo_path == "/second")));
}

#[test]
fn epic_repo_path_digit_with_nonempty_buffer_appends() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/first".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });
    app.input.buffer = "/my".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Char('1')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "/my1");
}

// ---------------------------------------------------------------------------
// input.rs — handle_key_confirm_delete_epic
// ---------------------------------------------------------------------------

fn make_app_confirm_delete_epic() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1); // cursor on epic (same priority as task, sorts after by id)
    app.input.mode = InputMode::ConfirmDeleteEpic;
    app.status.message = Some("Delete epic \"Epic 10\" and subtasks? [y/n]".to_string());
    app
}

#[test]
fn confirm_delete_epic_enters_mode_with_title() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1); // cursor on epic (same priority as task, sorts after by id)
    app.update(Message::ConfirmDeleteEpic);
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteEpic);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete epic \"Epic 10\" and subtasks? [y/n]")
    );
}

#[test]
fn confirm_delete_epic_y_deletes() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn confirm_delete_epic_uppercase_y_deletes() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn confirm_delete_epic_other_key_cancels() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert_eq!(app.board.epics.len(), 1); // not deleted
    assert!(cmds.is_empty());
}

#[test]
fn confirm_delete_epic_no_epic_selected_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0); // cursor on task, not epic
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty()); // no deletion happened
}

// ---------------------------------------------------------------------------
// input.rs — handle_key_confirm_archive_epic
// ---------------------------------------------------------------------------

fn make_app_confirm_archive_epic() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1); // cursor on epic (same priority as task, sorts after by id)
    app.input.mode = InputMode::ConfirmArchiveEpic;
    app.status.message = Some("Archive epic and all subtasks? [y/n]".to_string());
    app
}

#[test]
fn confirm_archive_epic_y_archives() {
    let mut app = make_app_confirm_archive_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert!(app.board.epics.is_empty()); // removed
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn confirm_archive_epic_uppercase_y_archives() {
    let mut app = make_app_confirm_archive_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteEpic(id) if *id == EpicId(10))));
}

#[test]
fn confirm_archive_epic_other_key_cancels() {
    let mut app = make_app_confirm_archive_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert_eq!(app.board.epics.len(), 1); // not removed
    assert!(cmds.is_empty());
}

#[test]
fn confirm_archive_epic_no_epic_selected_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    app.input.mode = InputMode::ConfirmArchiveEpic;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------------------
// input.rs — g key on epic
// ---------------------------------------------------------------------------

#[test]
fn g_key_on_epic_enters_epic_view() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];

    // Even with subtasks that have tmux windows, g enters epic view
    let mut subtask = make_task(1, TaskStatus::Review);
    subtask.epic_id = Some(EpicId(10));
    subtask.tmux_window = Some("win-1".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(10)));
}

#[test]
fn shift_g_on_epic_jumps_to_review_subtask() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Review);
    subtask.epic_id = Some(EpicId(10));
    subtask.tmux_window = Some("win-1".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-1"));
}

#[test]
fn shift_g_on_epic_no_session_shows_status() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let _cmds = app.handle_key(make_key(KeyCode::Char('G')));
    // Should NOT enter epic view — shows status info instead
    assert!(!matches!(app.board.view_mode, ViewMode::Epic { .. }));
}

#[test]
fn shift_g_on_epic_jumps_to_blocked_running_subtask() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.sub_status = SubStatus::NeedsInput;
    subtask.tmux_window = Some("win-blocked".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-blocked"));
}

#[test]
fn shift_g_on_epic_skips_active_running_subtask() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.sub_status = SubStatus::Active;
    subtask.tmux_window = Some("win-running".to_string());
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let _cmds = app.handle_key(make_key(KeyCode::Char('G')));
    // Active running subtask is skipped, no session found => status info
    assert!(!matches!(app.board.view_mode, ViewMode::Epic { .. }));
}

#[test]
fn shift_g_on_epic_prefers_blocked_running_over_review() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut review_task = make_task(1, TaskStatus::Review);
    review_task.epic_id = Some(EpicId(10));
    review_task.tmux_window = Some("win-review".to_string());

    let mut running_task = make_task(2, TaskStatus::Running);
    running_task.epic_id = Some(EpicId(10));
    running_task.sub_status = SubStatus::NeedsInput;
    running_task.tmux_window = Some("win-running".to_string());

    app.board.tasks = vec![review_task, running_task];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-running"));
}

#[test]
fn shift_g_on_epic_active_running_falls_through_to_review() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut review_task = make_task(1, TaskStatus::Review);
    review_task.epic_id = Some(EpicId(10));
    review_task.tmux_window = Some("win-review".to_string());

    let mut running_task = make_task(2, TaskStatus::Running);
    running_task.epic_id = Some(EpicId(10));
    running_task.sub_status = SubStatus::Active;
    running_task.tmux_window = Some("win-running".to_string());

    app.board.tasks = vec![review_task, running_task];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-review"));
}

#[test]
fn shift_g_on_epic_picks_lowest_sort_order() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut task_high = make_task(1, TaskStatus::Running);
    task_high.epic_id = Some(EpicId(10));
    task_high.sub_status = SubStatus::NeedsInput;
    task_high.sort_order = Some(5);
    task_high.tmux_window = Some("win-high".to_string());

    let mut task_low = make_task(2, TaskStatus::Running);
    task_low.epic_id = Some(EpicId(10));
    task_low.sub_status = SubStatus::Stale;
    task_low.sort_order = Some(1);
    task_low.tmux_window = Some("win-low".to_string());

    app.board.tasks = vec![task_high, task_low];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('G')));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "win-low"));
}

// ---------------------------------------------------------------------------
// input.rs — Archive panel extras
// ---------------------------------------------------------------------------

#[test]
fn archive_panel_down_arrow_navigates() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Archived),
            make_task(2, TaskStatus::Archived),
        ],
        TEST_TIMEOUT,
    );
    app.archive.visible = true;
    assert_eq!(app.archive.selected_row, 0);
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.archive.selected_row, 1);
}

#[test]
fn archive_panel_up_arrow_navigates() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Archived),
            make_task(2, TaskStatus::Archived),
        ],
        TEST_TIMEOUT,
    );
    app.archive.visible = true;
    app.archive.selected_row = 1;
    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.archive.selected_row, 0);
}

#[test]
fn archive_panel_esc_closes() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Esc));
    assert!(!app.archive.visible);
}

#[test]
fn archive_panel_e_edits_task() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmEditTask(TaskId(1))
    ));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EditTaskInEditor(t) if t.id == TaskId(1)));
}

#[test]
fn archive_panel_e_on_empty_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.archive.visible = true;
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds.is_empty());
}

#[test]
fn archive_panel_x_on_empty_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::Normal); // did not enter ConfirmDelete
}

#[test]
fn archive_panel_q_enters_confirm_quit() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn archive_panel_unrecognized_key_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    app.archive.visible = true;
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
    assert!(app.archive.visible);
}

// ---------------------------------------------------------------------------
// input.rs — Confirm archive extras
// ---------------------------------------------------------------------------

#[test]
fn confirm_archive_uppercase_y_archives() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.input.mode = InputMode::ConfirmArchive;
    app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Archived);
}

#[test]
fn confirm_archive_esc_cancels() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.input.mode = InputMode::ConfirmArchive;
    app.status.message = Some("Archive task? [y/n]".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert!(cmds.is_empty());
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog); // unchanged
}

// ---------------------------------------------------------------------------
// input.rs — Quick dispatch extras
// ---------------------------------------------------------------------------

#[test]
fn quick_dispatch_zero_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('0')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_non_digit_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

// ---------------------------------------------------------------------------
// input.rs — Other edge cases
// ---------------------------------------------------------------------------

#[test]
fn confirm_retry_unrecognized_key_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));
    let cmds = app.handle_key(make_key(KeyCode::Char('x')));
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn normal_mode_unrecognized_key_is_noop() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
    assert!(!app.should_quit);
}

#[test]
fn text_input_unrecognized_key_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "x".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "x");
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn d_key_on_archived_shows_warning() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Archived)], TEST_TIMEOUT);
    // Archived tasks don't appear in columns, but test dispatch routing directly
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    // No task selected (archived tasks hidden from kanban) → noop
    assert!(cmds.is_empty());
}

#[test]
fn question_mark_toggles_help_mode() {
    let mut app = make_app();
    assert_eq!(app.input.mode, InputMode::Normal);

    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Help);
}

#[test]
fn question_mark_dismisses_help() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn esc_dismisses_help() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn help_mode_ignores_other_keys() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::Help);
    assert!(!app.should_quit);
}

#[test]
fn help_overlay_renders_when_active() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    let buf = render_to_buffer(&mut app, 80, 35);
    assert!(buffer_contains(&buf, "Navigation"));
    assert!(buffer_contains(&buf, "Actions"));
    assert!(buffer_contains(&buf, "General"));
}

#[test]
fn help_overlay_hidden_in_normal_mode() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 80, 30);
    assert!(!buffer_contains(&buf, "Navigation"));
}

// ---------------------------------------------------------------------------
// Finish task tests
// ---------------------------------------------------------------------------

#[test]
fn finish_complete_moves_to_done() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    let cmds = app.update(Message::FinishComplete(TaskId(1)));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    // Worktree is preserved — will be cleaned up during archive
    assert!(task.worktree.is_some());
    assert!(task.tmux_window.is_none());
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn finish_failed_with_conflict_sets_flag() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.update(Message::FinishFailed {
        id: TaskId(1),
        error: "Rebase conflict".to_string(),
        is_conflict: true,
    });
    assert!(app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
    assert!(app
        .status
        .message
        .as_ref()
        .unwrap()
        .contains("Rebase conflict"));
}

#[test]
fn finish_failed_without_conflict_does_not_set_flag() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.update(Message::FinishFailed {
        id: TaskId(1),
        error: "Not on main".to_string(),
        is_conflict: false,
    });
    assert!(!app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn conflict_flag_clears_on_dispatch() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.update(Message::FinishFailed {
        id: TaskId(1),
        error: "conflict".to_string(),
        is_conflict: true,
    });
    assert!(app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));

    app.update(Message::Resumed {
        id: TaskId(1),
        tmux_window: "task-1".to_string(),
    });
    assert!(!app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn conflict_flag_clears_on_move_backward() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.update(Message::FinishFailed {
        id: TaskId(1),
        error: "conflict".to_string(),
        is_conflict: true,
    });

    app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    });
    assert!(!app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

// --- truncate_title ---

#[test]
fn truncate_title_short() {
    assert_eq!(super::truncate_title("Fix bug", 30), "\"Fix bug\"");
}

#[test]
fn truncate_title_exact_limit() {
    let title = "a".repeat(30);
    assert_eq!(super::truncate_title(&title, 30), format!("\"{}\"", title));
}

#[test]
fn truncate_title_over_limit() {
    let title = "Refactor the authentication middleware system";
    assert_eq!(
        super::truncate_title(title, 30),
        "\"Refactor the authentication...\""
    );
}

#[test]
fn truncate_title_multibyte_chars() {
    // Multi-byte UTF-8 characters must not panic on truncation
    let title = "Fix the caf\u{00e9} rendering bug now";
    // 31 chars, should truncate at char boundary not byte boundary
    assert!(super::truncate_title(title, 10).ends_with("...\""));
}

#[test]
fn confirm_delete_start_running_with_worktree_shows_warning() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt/4-test".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Task is in Running column (column 1), navigate there
    app.selection_mut().set_column(1);
    app.update(Message::ConfirmDeleteStart);
    assert_eq!(app.input.mode, InputMode::ConfirmDelete);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete \"Task 4\" [running] (has worktree)? [y/n]")
    );
}

#[test]
fn focused_column_has_tinted_background() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Running),
        ],
        TEST_TIMEOUT,
    );
    // Use wider terminal so 8 columns have enough room for content.
    // Columns use Ratio constraints (3/18, 2/18, ...) so they aren't equal width.
    let buf = render_to_buffer(&mut app, 240, 30);

    // Focused column (Backlog, col 0) should have a tinted bg.
    // Check a row well below the cursor card to avoid cursor highlight.
    let expected_bg = Color::Rgb(28, 30, 44);
    let cell = &buf[(1, 15)];
    // Backlog is 3/18 of 240 = 40px. Check well past that at x=120 (middle of board).
    let cell2 = &buf[(120, 15)];

    assert_eq!(
        cell.bg, expected_bg,
        "Focused column should have tinted background"
    );
    assert_ne!(
        cell2.bg, expected_bg,
        "Unfocused column should NOT have tinted background"
    );
}

// ---------------------------------------------------------------------------
// Done confirmation tests
// ---------------------------------------------------------------------------

#[test]
fn move_review_to_done_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column

    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert!(cmds.is_empty());
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(1))));
    assert!(app.status.message.as_deref().unwrap().contains("Done"));
}

#[test]
fn confirm_done_y_moves_task() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], TEST_TIMEOUT);
    app.selection_mut().set_column(2);

    app.input.mode = InputMode::ConfirmDone(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn confirm_done_n_cancels() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], TEST_TIMEOUT);
    app.selection_mut().set_column(2);

    app.input.mode = InputMode::ConfirmDone(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert!(cmds.is_empty());
}

#[test]
fn move_backlog_to_running_no_confirmation() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.selection_mut().set_column(0); // Backlog column

    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn confirm_done_kills_tmux_but_preserves_worktree() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-test".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    app.selection_mut().set_column(2);

    // Enter confirm mode and confirm
    app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Forward,
    });
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(TaskId(1))));

    let cmds = app.update(Message::ConfirmDone);
    // No Cleanup command — worktree stays for archive to clean up later
    assert!(!cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    // Tmux window should be killed
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::KillTmuxWindow { .. })));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    // Worktree is preserved (not taken), tmux_window cleared
    assert!(task.worktree.is_some());
    assert!(task.tmux_window.is_none());
}

#[test]
fn batch_move_with_review_tasks_enters_confirm_done() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Review),
            make_task(2, TaskStatus::Review),
        ],
        TEST_TIMEOUT,
    );
    app.selection_mut().set_column(2);
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert!(cmds.is_empty());
    assert!(app.status.message.as_deref().unwrap().contains("2 tasks"));
    assert!(app.status.message.as_deref().unwrap().contains("Done"));
}

#[test]
fn batch_confirm_done_moves_all_review_tasks() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Review),
            make_task(2, TaskStatus::Review),
        ],
        TEST_TIMEOUT,
    );
    app.selection_mut().set_column(2);
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    // Trigger batch move
    app.update(Message::BatchMoveTasks {
        ids: vec![TaskId(1), TaskId(2)],
        direction: MoveDirection::Forward,
    });
    // Confirm
    let cmds = app.update(Message::ConfirmDone);
    assert_eq!(app.input.mode, InputMode::Normal);
    for id in [TaskId(1), TaskId(2)] {
        let task = app.board.tasks.iter().find(|t| t.id == id).unwrap();
        assert_eq!(task.status, TaskStatus::Done);
    }
    assert!(cmds.len() >= 2); // two PersistTask commands
}

#[test]
fn batch_move_mixed_statuses_moves_non_review_immediately() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Running),
            make_task(2, TaskStatus::Review),
        ],
        TEST_TIMEOUT,
    );
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelect(TaskId(2)));

    let cmds = app.update(Message::BatchMoveTasks {
        ids: vec![TaskId(1), TaskId(2)],
        direction: MoveDirection::Forward,
    });
    // Running→Review moved immediately
    let t1 = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(t1.status, TaskStatus::Review);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(1))));

    // Review→Done waiting for confirmation
    let t2 = app.board.tasks.iter().find(|t| t.id == TaskId(2)).unwrap();
    assert_eq!(t2.status, TaskStatus::Review); // not moved yet
    assert!(matches!(app.input.mode, InputMode::ConfirmDone(_)));
}

// --- Status message auto-clear ---

#[test]
fn status_message_clears_after_timeout_on_tick() {
    let mut app = make_app();
    // Simulate a status message that was set 6 seconds ago
    app.status.message = Some("Task 1 finished".to_string());
    app.status.message_set_at = Some(Instant::now() - Duration::from_secs(6));

    // Tick should clear it since it's past the 5-second timeout
    app.update(Message::Tick);
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
    app.update(Message::Tick);
    assert_eq!(app.status.message.as_deref(), Some("Task 1 finished"));
}

#[test]
fn status_message_does_not_clear_during_interactive_mode() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDelete;
    app.status.message = Some("Delete task? [y/n]".to_string());
    app.status.message_set_at = Some(Instant::now() - Duration::from_secs(10));

    // Tick should NOT clear it during an interactive mode
    app.update(Message::Tick);
    assert!(
        app.status.message.is_some(),
        "should not clear during interactive mode"
    );
}

// ---------------------------------------------------------------------------
// Select-all toggle
// ---------------------------------------------------------------------------

#[test]
fn on_select_all_defaults_to_false() {
    let app = make_app();
    assert!(!app.on_select_all());
}

#[test]
fn select_all_column_selects_all_tasks_in_column() {
    let mut app = make_app();
    // Cursor is on Backlog (column 0) which has tasks 1, 2
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
    assert_eq!(app.select.tasks.len(), 2);
}

#[test]
fn select_all_column_deselects_when_all_selected() {
    let mut app = make_app();
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 2);

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
}

#[test]
fn select_all_column_selects_remaining_when_partially_selected() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(1)));
    assert_eq!(app.select.tasks.len(), 1);

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
    assert_eq!(app.select.tasks.len(), 2);
}

#[test]
fn select_all_column_noop_on_empty_column() {
    let mut app = make_app();
    // Navigate to Review column (empty in make_app)
    app.update(Message::NavigateColumn(2));
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
}

#[test]
fn select_all_column_only_affects_current_column() {
    let mut app = make_app();
    // TaskId(3) is in Running column, pre-select it
    app.update(Message::ToggleSelect(TaskId(3)));
    // SelectAllColumn selects all in current (Backlog) column
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
    assert!(app.select.tasks.contains(&TaskId(3)));
    assert_eq!(app.select.tasks.len(), 3);
}

#[test]
fn select_all_deselect_only_affects_current_column() {
    let mut app = make_app();
    app.update(Message::ToggleSelect(TaskId(3)));
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 3);

    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 1);
    assert!(app.select.tasks.contains(&TaskId(3)));
}

#[test]
fn key_a_selects_all_in_column() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
}

#[test]
fn key_a_toggles_off_when_all_selected() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.select.tasks.len(), 2);
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn navigate_up_from_row_zero_enters_select_all_toggle() {
    let mut app = make_app();
    assert!(!app.on_select_all());
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
}

#[test]
fn navigate_down_from_toggle_exits_to_row_zero() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(!app.on_select_all());
    assert_eq!(app.selected_row()[0], 0);
}

#[test]
fn column_switch_preserves_on_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
    app.handle_key(make_key(KeyCode::Char('l')));
    assert!(app.on_select_all());
}

#[test]
fn enter_on_toggle_triggers_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
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
fn space_is_noop_when_on_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn dispatch_is_noop_when_on_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
}

#[test]
fn render_shows_select_all_toggle_in_focused_column() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[ ]"));
    assert!(!buffer_contains(&buf, "Select [a]ll"));
}

#[test]
fn render_shows_checked_toggle_when_all_selected() {
    let mut app = make_app();
    app.update(Message::SelectAllColumn);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[x]"));
}

#[test]
fn render_shows_unchecked_toggle_when_not_all_selected() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[ ]"));
}

#[test]
fn action_hints_include_select_all() {
    let app = make_app();
    let task = app.selected_task();
    let spans = ui::action_hints(task, Color::Blue);
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("select all"),
        "action hints should include 'select all'"
    );
}

// ---------------------------------------------------------------------------
// Column scrolling tests
// ---------------------------------------------------------------------------

#[test]
fn column_scrolls_to_keep_cursor_visible() {
    // Create 20 backlog tasks — more than fit in a 20-row terminal
    let tasks: Vec<Task> = (1..=20)
        .map(|id| make_task(id, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks, TEST_TIMEOUT);

    // Navigate down to the last task (row 19, past visible area)
    for _ in 0..19 {
        app.update(Message::NavigateRow(1));
    }

    // Render in a small terminal (height 20 with header/detail/status ~10 lines
    // leaves roughly 10 lines for the column, fitting ~4-5 two-line task cards)
    let buf = render_to_buffer(&mut app, 120, 20);

    // The cursor should be on "Task 20" and it should be visible in the buffer
    assert!(
        buffer_contains(&buf, "Task 20"),
        "cursor task should be visible after scrolling down"
    );
}

#[test]
fn column_scrolls_back_up_when_cursor_moves_up() {
    let tasks: Vec<Task> = (1..=20)
        .map(|id| make_task(id, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks, TEST_TIMEOUT);

    // Navigate to the bottom
    for _ in 0..19 {
        app.update(Message::NavigateRow(1));
    }
    // Render once to establish scroll state
    let _ = render_to_buffer(&mut app, 120, 20);

    // Navigate back to the top
    for _ in 0..19 {
        app.update(Message::NavigateRow(-1));
    }
    let buf = render_to_buffer(&mut app, 120, 20);

    assert!(
        buffer_contains(&buf, "Task 1"),
        "first task should be visible after scrolling back up"
    );
}

#[test]
fn notifications_disabled_by_default() {
    let app = make_app();
    assert!(!app.notifications_enabled());
}

#[test]
fn toggle_notifications_flips_state() {
    let mut app = make_app();
    assert!(!app.notifications_enabled()); // default: false
    app.update(Message::ToggleNotifications);
    assert!(app.notifications_enabled());
    app.update(Message::ToggleNotifications);
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
    let cmds = app.update(Message::RefreshTasks(updated));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 1);
    match &notif_cmds[0] {
        Command::SendNotification { title, urgent, .. } => {
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
    let cmds = app.update(Message::RefreshTasks(updated));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 1);
    match &notif_cmds[0] {
        Command::SendNotification { urgent, .. } => {
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
    app.update(Message::RefreshTasks(updated.clone()));
    // Second refresh with same state should not re-notify
    let cmds = app.update(Message::RefreshTasks(updated));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
        .collect();
    assert_eq!(notif_cmds.len(), 0);
}

#[test]
fn refresh_tasks_does_not_duplicate_needs_input_notifications() {
    let mut app = make_app();

    let mut updated = app.board.tasks.to_vec();
    updated[2].sub_status = SubStatus::NeedsInput;
    app.update(Message::RefreshTasks(updated.clone()));
    // Second refresh with same state should not re-notify
    let cmds = app.update(Message::RefreshTasks(updated));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
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
    let cmds = app.update(Message::RefreshTasks(updated.clone()));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::SendNotification { .. }))
            .count(),
        1
    );

    // Clear NeedsInput (agent resumes)
    updated[2].sub_status = SubStatus::Active;
    app.update(Message::RefreshTasks(updated.clone()));

    // Second transition to NeedsInput should re-notify
    updated[2].sub_status = SubStatus::NeedsInput;
    let cmds = app.update(Message::RefreshTasks(updated));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::SendNotification { .. }))
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
    let cmds = app.update(Message::RefreshTasks(updated));

    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
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
        .any(|c| matches!(c, Command::PersistSetting { .. })));
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
    app.update(Message::RefreshTasks(updated));

    // Move to done — should clear notified state
    let mut updated2 = app.board.tasks.to_vec();
    updated2[2].status = TaskStatus::Done;
    app.update(Message::RefreshTasks(updated2));

    // Move back to review — should re-notify
    let mut updated3 = app.board.tasks.to_vec();
    updated3[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated3));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
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
    let cmds = app.update(Message::RefreshTasks(updated));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::SendNotification { .. }))
            .count(),
        1
    );

    // Disable notifications
    app.update(Message::ToggleNotifications);

    // Task leaves review while disabled
    let mut updated2 = app.board.tasks.to_vec();
    updated2[2].status = TaskStatus::Done;
    app.update(Message::RefreshTasks(updated2));

    // Re-enable notifications
    app.update(Message::ToggleNotifications);

    // Task returns to review — should re-notify because notified state was cleared
    let mut updated3 = app.board.tasks.to_vec();
    updated3[2].status = TaskStatus::Review;
    let cmds = app.update(Message::RefreshTasks(updated3));
    let notif_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Command::SendNotification { .. }))
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

// -----------------------------------------------------------------------
// PR handler tests
// -----------------------------------------------------------------------

#[test]
fn pr_created_stores_url() {
    let task = make_task(1, TaskStatus::Review);
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::PrCreated {
        id: TaskId(1),
        pr_url: "https://github.com/org/repo/pull/42".to_string(),
    });

    let task = app.find_task(TaskId(1)).unwrap();
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/42")
    );
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn pr_failed_shows_error() {
    let task = make_task(1, TaskStatus::Review);
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    app.update(Message::PrFailed {
        id: TaskId(1),
        error: "Push failed".to_string(),
    });

    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("Push failed"));
}

#[test]
fn pr_merged_moves_to_done_and_detaches() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    let task = app.find_task(TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none(), "tmux window should be cleared");
    assert!(task.worktree.is_some(), "worktree should be preserved");
    assert!(task.pr_url.is_some(), "pr_url should be preserved");
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn pr_merged_preserves_worktree() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    // Should NOT emit a Cleanup command
    assert!(!cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
}

#[test]
fn pr_merged_kills_matching_review_board_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Load a review PR that matches the task's PR URL
    let mut review_pr = make_review_pr_for_repo(42, "alice", ReviewDecision::Approved, "org/repo");
    review_pr.tmux_window = Some("review:pr-42".to_string());
    review_pr.worktree = Some("/repo/.worktrees/review-42".to_string());
    app.update(Message::PrsLoaded(PrListKind::Review, vec![review_pr]));

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "review:pr-42")),
        "should kill review board PR window"
    );
    assert!(
        cmds.iter().any(
            |c| matches!(c, Command::UpdateAgentStatus { repo, number, status: None }
                if repo == "org/repo" && *number == 42)
        ),
        "should clear review agent status"
    );
    // Review PR state should be cleared in-memory
    assert!(app.review.review.prs[0].tmux_window.is_none());
    assert!(app.review.review.prs[0].worktree.is_none());
    assert!(app.review.review.prs[0].agent_status.is_none());
}

#[test]
fn pr_merged_no_review_board_match_is_ok() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Load a review PR with a DIFFERENT number — should not be cleaned up
    let mut other_pr =
        make_review_pr_for_repo(99, "bob", ReviewDecision::ReviewRequired, "org/repo");
    other_pr.tmux_window = Some("review:pr-99".to_string());
    app.update(Message::PrsLoaded(PrListKind::Review, vec![other_pr]));

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    // The task should still move to Done
    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Done);
    // The unrelated review window should NOT be killed
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "review:pr-99")),
        "should not kill unrelated review board PR window"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::UpdateAgentStatus { .. })),
        "should not emit UpdateAgentStatus when no review board PR matches"
    );
}

#[test]
fn pr_merged_skips_update_agent_status_when_review_pr_has_no_agent_state() {
    // Review PR matches by (repo, number) but has no active agent session.
    // cleanup_review_board_pr should NOT emit UpdateAgentStatus in this case —
    // it is a no-op write to clear already-null fields.
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Review PR with no tmux_window, worktree, or agent_status
    let review_pr = make_review_pr_for_repo(42, "alice", ReviewDecision::Approved, "org/repo");
    app.update(Message::PrsLoaded(PrListKind::Review, vec![review_pr]));

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    assert_eq!(app.find_task(TaskId(1)).unwrap().status, TaskStatus::Done);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::UpdateAgentStatus { .. })),
        "should not emit UpdateAgentStatus when review PR has no active agent state"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { .. })),
        "should not kill any window when review PR has no active agent state"
    );
}

#[test]
fn pr_merged_kills_both_task_and_review_windows() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let mut review_pr = make_review_pr_for_repo(42, "alice", ReviewDecision::Approved, "org/repo");
    review_pr.tmux_window = Some("review:pr-42".to_string());
    app.update(Message::PrsLoaded(PrListKind::Review, vec![review_pr]));

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "task-1")),
        "should kill task's own tmux window"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "review:pr-42")),
        "should kill review board PR window"
    );
}

#[test]
fn card_shows_pr_badge() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
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

#[test]
fn card_shows_merged_pr_badge() {
    let mut task = make_task(1, TaskStatus::Done);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Navigate to Done column (visual index 7)
    for _ in 0..7 {
        app.update(Message::NavigateColumn(1));
    }

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "PR #42 merged"),
        "Done card should show merged PR badge"
    );
}

#[test]
fn status_bar_shows_wrap_up_hint_for_review_task() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Navigate to Review column (index 2)
    for _ in 0..2 {
        app.update(Message::NavigateColumn(1));
    }

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "[W]rap up"),
        "Status bar should show wrap up hint for Review tasks"
    );
}

#[test]
fn detail_panel_shows_pr_url() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Navigate to Review column (index 2) and open detail panel
    for _ in 0..2 {
        app.update(Message::NavigateColumn(1));
    }
    app.update(Message::ToggleDetail);

    let buf = render_to_buffer(&mut app, 200, 20);
    assert!(
        buffer_contains(&buf, "PR:"),
        "Detail panel should show PR label"
    );
    assert!(
        buffer_contains(&buf, "pull/42"),
        "Detail panel should show PR URL"
    );
}

#[test]
fn pr_polling_skips_done_tasks() {
    let mut task = make_task(1, TaskStatus::Done);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Tick);
    // Should NOT contain any CheckPrStatus command
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CheckPrStatus { .. })));
}

#[test]
fn pr_polling_emits_check_for_review_tasks() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Tick);
    assert!(cmds.iter().any(|c| matches!(c, Command::CheckPrStatus { ref pr_url, .. } if pr_url == "https://github.com/org/repo/pull/42")));
}

// --- repo_filter ---

#[test]
fn repo_filter_empty_shows_all_tasks() {
    let app = make_app();
    // repo_filter is empty by default => all tasks visible
    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 4); // tasks 1,2,3,4 (Done tasks are visible, only Archived are excluded)
}

#[test]
fn repo_filter_hides_non_matching_tasks() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.repos.insert("/repo-a".to_string());

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(1));
}

#[test]
fn repo_filter_applies_to_epics_in_column_items() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let now = chrono::Utc::now();
    app.board.epics = vec![
        Epic {
            id: EpicId(1),
            title: "A".into(),
            description: "".into(),
            repo_path: "/repo-a".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            created_at: now,
            updated_at: now,
        },
        Epic {
            id: EpicId(2),
            title: "B".into(),
            description: "".into(),
            repo_path: "/repo-b".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            created_at: now,
            updated_at: now,
        },
    ];
    app.filter.repos.insert("/repo-a".to_string());

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 1); // only epic A
}

#[test]
fn repo_filter_applies_to_archived_tasks() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Archived);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Archived);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.repos.insert("/repo-a".to_string());

    let archived = app.archived_tasks();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, TaskId(1));
}

// --- repo filter keybindings ---

#[test]
fn f_key_opens_repo_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn repo_filter_number_key_toggles_repo() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('1')));
    assert!(app.filter.repos.contains("/repo-a"));

    app.handle_key(make_key(KeyCode::Char('1')));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn repo_filter_a_key_toggles_all() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.filter.repos.len(), 2);
}

#[test]
fn repo_filter_enter_closes() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistStringSetting { .. })));
}

#[test]
fn repo_filter_esc_closes() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn repo_filter_out_of_range_number_ignored() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('5')));
    assert!(app.filter.repos.is_empty());
}

#[test]
fn summary_row_shows_filter_indicator() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.filter.repos.insert("/a".to_string());
    app.filter.repos.insert("/b".to_string());

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "2/3 repos"),
        "Expected filter indicator in summary"
    );
}

// --- repo filter exclude mode ---

#[test]
fn repo_filter_exclude_hides_matching_tasks() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(2));
}

#[test]
fn repo_filter_exclude_empty_shows_all() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.mode = RepoFilterMode::Exclude;

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 2);
}

#[test]
fn repo_filter_exclude_applies_to_epics() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let now = chrono::Utc::now();
    app.board.epics = vec![
        Epic {
            id: EpicId(1),
            title: "A".into(),
            description: "".into(),
            repo_path: "/repo-a".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            created_at: now,
            updated_at: now,
        },
        Epic {
            id: EpicId(2),
            title: "B".into(),
            description: "".into(),
            repo_path: "/repo-b".into(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            created_at: now,
            updated_at: now,
        },
    ];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 1);
    match &items[0] {
        ColumnItem::Epic(e) => assert_eq!(e.id, EpicId(2)),
        _ => panic!("Expected epic"),
    }
}

#[test]
fn repo_filter_exclude_applies_to_archived() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut t1 = make_task(1, TaskStatus::Archived);
    t1.repo_path = "/repo-a".to_string();
    let mut t2 = make_task(2, TaskStatus::Archived);
    t2.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![t1, t2];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let archived = app.archived_tasks();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, TaskId(2));
}

#[test]
fn toggle_repo_filter_mode_switches() {
    let mut app = make_app();
    assert_eq!(app.filter.mode, RepoFilterMode::Include);
    app.update(Message::ToggleRepoFilterMode);
    assert_eq!(app.filter.mode, RepoFilterMode::Exclude);
    app.update(Message::ToggleRepoFilterMode);
    assert_eq!(app.filter.mode, RepoFilterMode::Include);
}

#[test]
fn tab_key_toggles_repo_filter_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Tab));
    assert_eq!(app.filter.mode, RepoFilterMode::Exclude);
}

#[test]
fn close_repo_filter_persists_mode() {
    let mut app = make_app();
    app.filter.mode = RepoFilterMode::Exclude;
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.update(Message::CloseRepoFilter);
    assert!(cmds.iter().any(|c| matches!(c,
        Command::PersistStringSetting { key, value } if key == "repo_filter_mode" && value == "exclude"
    )));
}

#[test]
fn save_filter_preset_stores_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;
    app.input.mode = InputMode::InputPresetName;

    let cmds = app.update(Message::SaveFilterPreset("excl-preset".to_string()));
    assert_eq!(app.filter.presets[0].2, RepoFilterMode::Exclude);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistFilterPreset {
            mode: RepoFilterMode::Exclude,
            ..
        }
    )));
}

#[test]
fn load_filter_preset_restores_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("excl".to_string(), repos, RepoFilterMode::Exclude)];

    app.update(Message::LoadFilterPreset("excl".to_string()));
    assert_eq!(app.filter.mode, RepoFilterMode::Exclude);
    assert!(app.filter.repos.contains("/repo-a"));
}

#[test]
fn summary_row_shows_excl_prefix_in_exclude_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.filter.repos.insert("/a".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "excl 1/3 repos"),
        "Expected excl prefix in filter indicator"
    );
}

#[test]
fn repo_filter_overlay_shows_mode_in_title() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.filter.mode = RepoFilterMode::Exclude;
    app.input.mode = InputMode::RepoFilter;

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(
        buffer_contains(&buf, "exclude"),
        "Expected 'exclude' in overlay title"
    );
}

// --- wrap up ---

#[test]
fn w_key_on_review_task_with_worktree_enters_wrap_up() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    // Navigate to Review column (index 2)
    app.update(Message::NavigateColumn(2));

    app.handle_key(make_key(KeyCode::Char('W')));
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmWrapUp(TaskId(1))
    ));
}

#[test]
fn w_key_on_non_review_task_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);

    app.handle_key(make_key(KeyCode::Char('W')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn wrap_up_r_emits_finish_command() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    app.update(Message::NavigateColumn(4));

    app.update(Message::StartWrapUp(TaskId(1)));
    let cmds = app.update(Message::WrapUpRebase);
    assert!(cmds.iter().any(|c| matches!(c, Command::Finish { .. })));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn wrap_up_p_emits_create_pr_command() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    app.update(Message::NavigateColumn(4));

    app.update(Message::StartWrapUp(TaskId(1)));
    let cmds = app.update(Message::WrapUpPr);
    assert!(cmds.iter().any(|c| matches!(c, Command::CreatePr { .. })));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn wrap_up_esc_cancels() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    app.update(Message::NavigateColumn(4));

    app.update(Message::StartWrapUp(TaskId(1)));
    app.update(Message::CancelWrapUp);
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn wrap_up_rebase_clears_conflict_flag() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.find_task_mut(TaskId(1)).unwrap().sub_status = SubStatus::Conflict;
    app.update(Message::StartWrapUp(TaskId(1)));
    app.update(Message::WrapUpRebase);
    assert!(!app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn wrap_up_available_on_running_blocked() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.find_task_mut(id).unwrap().sub_status = SubStatus::NeedsInput;
    app.find_task_mut(id).unwrap().worktree = Some("/tmp/wt".to_string());
    app.selection_mut().set_column(2); // Blocked column
    app.update(Message::StartWrapUp(id));
    assert!(matches!(app.mode(), InputMode::ConfirmWrapUp(_)));
}

#[test]
fn wrap_up_available_on_running_active() {
    let mut app = make_app();
    let id = TaskId(3); // Running, Active by default
    app.find_task_mut(id).unwrap().worktree = Some("/tmp/wt".to_string());
    app.update(Message::StartWrapUp(id));
    assert!(matches!(app.mode(), InputMode::ConfirmWrapUp(_)));
}

// --- sort_order ---

#[test]
fn column_items_sorted_by_sort_order() {
    let mut app = make_app();
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.title = "First".to_string();
    t1.sort_order = Some(200);
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.title = "Second".to_string();
    t2.sort_order = Some(100);
    app.board.tasks = vec![t1, t2];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 2);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Second"),
        _ => panic!("expected task"),
    }
    match &items[1] {
        ColumnItem::Task(t) => assert_eq!(t.title, "First"),
        _ => panic!("expected task"),
    }
}

#[test]
fn column_items_null_sort_order_uses_id() {
    let mut app = make_app();
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.title = "High ID".to_string();
    t1.sort_order = None;
    let mut t2 = make_task(5, TaskStatus::Backlog);
    t2.title = "Low ID".to_string();
    t2.sort_order = None;
    app.board.tasks = vec![t1, t2];

    let items = app.column_items_for_status(TaskStatus::Backlog);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Low ID"),
        _ => panic!("expected task"),
    }
}

// ---------------------------------------------------------------------------
// Reorder item (J/K) tests
// ---------------------------------------------------------------------------

#[test]
fn reorder_task_down_swaps_sort_order() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    let t2 = make_task(2, TaskStatus::Backlog);
    app.board.tasks = vec![t1, t2];

    // Cursor on first task (row 0, column 0 = Backlog)
    let cmds = app.update(Message::ReorderItem(1));

    // After reorder, task 1 should have a higher sort value than task 2
    let t1 = app.find_task(TaskId(1)).unwrap();
    let t2 = app.find_task(TaskId(2)).unwrap();
    let eff1 = t1.sort_order.unwrap_or(t1.id.0);
    let eff2 = t2.sort_order.unwrap_or(t2.id.0);
    assert!(
        eff1 > eff2,
        "task 1 ({eff1}) should be after task 2 ({eff2}) after move down"
    );
    // Should emit PersistTask for both
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::PersistTask(_)))
            .count(),
        2
    );
    // Cursor should have moved down
    assert_eq!(app.selection().row(0), 1);
}

#[test]
fn reorder_task_up_at_top_is_noop() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::ReorderItem(-1));
    assert!(cmds.is_empty());
}

#[test]
fn reorder_task_down_at_bottom_is_noop() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::ReorderItem(1));
    assert!(cmds.is_empty());
}

#[test]
fn reorder_task_up_swaps_sort_order() {
    let mut app = make_app();
    let t1 = make_task(1, TaskStatus::Backlog);
    let t2 = make_task(2, TaskStatus::Backlog);
    app.board.tasks = vec![t1, t2];

    // Move cursor to row 1 (second task), then reorder up
    app.selection_mut().set_row(0, 1);
    let cmds = app.update(Message::ReorderItem(-1));

    // After reorder, task 2 should have a lower sort value than task 1
    let t1 = app.find_task(TaskId(1)).unwrap();
    let t2 = app.find_task(TaskId(2)).unwrap();
    let eff1 = t1.sort_order.unwrap_or(t1.id.0);
    let eff2 = t2.sort_order.unwrap_or(t2.id.0);
    assert!(
        eff2 < eff1,
        "task 2 ({eff2}) should be before task 1 ({eff1}) after move up"
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Command::PersistTask(_)))
            .count(),
        2
    );
    // Cursor should have moved up
    assert_eq!(app.selection().row(0), 0);
}

// --- Epic dispatch: next backlog subtask ---

#[test]
fn dispatch_epic_with_backlog_subtasks_dispatches_first_by_sort_order() {
    let mut app = make_app();

    // Create epic with a plan so subtask dispatch path is taken
    let mut epic = make_epic(1);
    epic.plan_path = Some("docs/plans/epic-1.md".to_string());
    app.board.epics = vec![epic];

    // Create two backlog subtasks with different sort orders (both have plans)
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.epic_id = Some(EpicId(1));
    t1.sort_order = Some(200);
    t1.title = "Second task".to_string();
    t1.plan_path = Some("docs/plans/task-10.md".to_string());
    let mut t2 = make_task(11, TaskStatus::Backlog);
    t2.epic_id = Some(EpicId(1));
    t2.sort_order = Some(100);
    t2.title = "First task".to_string();
    t2.plan_path = Some("docs/plans/task-11.md".to_string());
    app.board.tasks = vec![t1, t2];

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));

    // Should dispatch the task with lower sort_order (task 11, sort_order=100)
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { task, .. } if task.id == TaskId(11))));
}

#[test]
fn dispatch_epic_no_subtasks_falls_back_to_planning() {
    let mut app = make_app();

    let epic = make_epic(1);
    app.board.epics = vec![epic];
    // No subtasks

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));

    // Should fall back to planning dispatch
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchEpic { .. })));
}

#[test]
fn dispatch_epic_no_plan_with_subtasks_does_not_create_planning() {
    let mut app = make_app();

    let epic = make_epic(1); // no plan
    app.board.epics = vec![epic];

    // Epic has an active (running) subtask — should not spawn planning
    let mut t1 = make_task(10, TaskStatus::Running);
    t1.epic_id = Some(EpicId(1));
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));
    // Epic status is Running, so it's blocked by the Backlog check
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_epic_no_plan_with_backlog_subtask_does_not_create_planning() {
    let mut app = make_app();

    let epic = make_epic(1); // no plan
    app.board.epics = vec![epic];

    // Epic has a backlog subtask — epic status is Backlog but has subtasks
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.epic_id = Some(EpicId(1));
    app.board.tasks = vec![t1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));
    // Should NOT create planning subtask since subtasks already exist
    assert!(cmds.is_empty());
    assert!(app.status.message.as_deref().unwrap().contains("no plan"));
}

#[test]
fn dispatch_epic_all_done_shows_message() {
    let mut app = make_app();

    let mut epic = make_epic(1);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];

    let mut t1 = make_task(10, TaskStatus::Done);
    t1.epic_id = Some(EpicId(1));
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));

    // Epic status is Done — should not dispatch
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No backlog tasks"));
}

// ---------------------------------------------------------------------------
// Review board tests
// ---------------------------------------------------------------------------

fn make_review_pr(number: i64, author: &str, decision: ReviewDecision) -> crate::models::ReviewPr {
    make_review_pr_for_repo(number, author, decision, "acme/app")
}

fn make_review_pr_for_repo(
    number: i64,
    author: &str,
    decision: ReviewDecision,
    repo: &str,
) -> crate::models::ReviewPr {
    crate::models::ReviewPr {
        number,
        title: format!("PR {number}"),
        author: author.to_string(),
        repo: repo.to_string(),
        url: format!("https://github.com/{repo}/pull/{number}"),
        is_draft: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        additions: 10,
        deletions: 5,
        review_decision: decision,
        labels: vec![],
        body: String::new(),
        head_ref: String::new(),
        ci_status: crate::models::CiStatus::None,
        reviewers: vec![],
        tmux_window: None,
        worktree: None,
        agent_status: None,
    }
}

fn make_bot_pr(
    number: i64,
    decision: crate::models::ReviewDecision,
    agent_status: Option<crate::models::ReviewAgentStatus>,
    ci: crate::models::CiStatus,
) -> crate::models::ReviewPr {
    let mut pr = make_review_pr_for_repo(number, "app/dependabot", decision, "acme/app");
    pr.agent_status = agent_status;
    pr.ci_status = ci;
    pr
}

fn make_security_alert(
    number: i64,
    repo: &str,
    severity: crate::models::AlertSeverity,
) -> crate::models::SecurityAlert {
    crate::models::SecurityAlert {
        number,
        repo: repo.to_string(),
        severity,
        kind: crate::models::AlertKind::Dependabot,
        title: format!("Alert {number}"),
        package: Some("some-pkg".to_string()),
        vulnerable_range: None,
        fixed_version: None,
        cvss_score: None,
        url: format!("https://github.com/{repo}/security/dependabot/{number}"),
        created_at: chrono::Utc::now(),
        state: "open".to_string(),
        description: String::new(),
        tmux_window: None,
        worktree: None,
        agent_status: None,
    }
}

#[test]
fn switch_to_review_board_preserves_task_selection() {
    let mut app = make_app();
    // Move cursor to column 1
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selected_column(), 1);

    // Switch to review board
    app.update(Message::SwitchToReviewBoard);
    assert!(matches!(app.board.view_mode, ViewMode::ReviewBoard { .. }));

    // Switch back
    app.update(Message::SwitchToTaskBoard);
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
    // Task board cursor should be restored
    assert_eq!(app.selected_column(), 1);
}

#[test]
fn review_prs_loaded_updates_state() {
    let mut app = make_app();
    let prs = vec![make_review_pr(42, "alice", ReviewDecision::ReviewRequired)];
    app.update(Message::PrsLoaded(PrListKind::Review, prs));
    assert_eq!(app.review_prs().len(), 1);
    assert_eq!(app.review_prs()[0].number, 42);
    assert!(!app.review_board_loading());
}

#[test]
fn review_prs_fetch_failed_sets_error() {
    let mut app = make_app();
    app.update(Message::PrsFetchFailed(
        PrListKind::Review,
        "auth error".to_string(),
    ));
    assert!(!app.review_board_loading());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("auth error"));
    assert_eq!(app.last_review_error(), Some("auth error"));
}

#[test]
fn switch_to_review_board_sets_loading() {
    let mut app = make_app();
    let cmds = app.update(Message::SwitchToReviewBoard);
    assert!(app.review_board_loading());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Review))));
}

#[test]
fn tab_switches_to_review_board() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(app.board.view_mode, ViewMode::ReviewBoard { .. }));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Review))));
}

#[test]
fn tab_in_review_board_switches_back() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Tab)); // to review board
    app.handle_key(make_key(KeyCode::Tab)); // to security board
    assert!(matches!(
        app.board.view_mode,
        ViewMode::SecurityBoard { .. }
    ));
    app.handle_key(make_key(KeyCode::Tab)); // back to task board
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn esc_in_review_board_switches_back() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Tab)); // to review board
    app.handle_key(make_key(KeyCode::Esc)); // back
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn review_board_navigation() {
    let mut app = make_app();
    // Load some PRs
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr(1, "alice", ReviewDecision::ReviewRequired),
            make_review_pr(2, "bob", ReviewDecision::ReviewRequired),
            make_review_pr(3, "carol", ReviewDecision::ChangesRequested),
        ],
    ));
    app.handle_key(make_key(KeyCode::Tab)); // to review board
    assert_eq!(app.review_selection().unwrap().column(), 0);

    app.handle_key(make_key(KeyCode::Char('l'))); // move right
    assert_eq!(app.review_selection().unwrap().column(), 1);

    app.handle_key(make_key(KeyCode::Char('l'))); // move right
    assert_eq!(app.review_selection().unwrap().column(), 2);

    app.handle_key(make_key(KeyCode::Char('l'))); // move right
    assert_eq!(app.review_selection().unwrap().column(), 3);

    app.handle_key(make_key(KeyCode::Char('l'))); // clamp at 3
    assert_eq!(app.review_selection().unwrap().column(), 3);
}

#[test]
fn review_board_enter_toggles_detail() {
    let mut app = make_app();
    app.review.review.set_prs(vec![make_review_pr(
        1,
        "alice",
        ReviewDecision::ReviewRequired,
    )]);
    app.update(Message::SwitchToReviewBoard);
    assert!(!app.review.detail_visible);

    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.review.detail_visible);

    app.handle_key(make_key(KeyCode::Enter));
    assert!(!app.review.detail_visible);
}

#[test]
fn review_board_p_opens_browser() {
    let mut app = make_app();
    app.review.review.set_prs(vec![make_review_pr(
        1,
        "alice",
        ReviewDecision::ReviewRequired,
    )]);
    app.update(Message::SwitchToReviewBoard);

    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::OpenInBrowser { .. })));
}

#[test]
fn review_board_renders_pr_titles() {
    let mut app = make_app();
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr(42, "alice", ReviewDecision::ReviewRequired),
            make_review_pr(50, "bob", ReviewDecision::Approved),
        ],
    ));
    app.update(Message::SwitchToReviewBoard);

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Needs Review"),
        "Should show column header"
    );
    assert!(buffer_contains(&buf, "PR 42"), "Should show PR title");
}

#[test]
fn review_board_renders_loading_state() {
    let mut app = make_app();
    // SwitchToReviewBoard triggers a fetch, so review_board_loading becomes true
    app.update(Message::SwitchToReviewBoard);
    assert!(app.review_board_loading());

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Loading..."),
        "Should show loading text while fetching"
    );
    assert!(
        !buffer_contains(&buf, "No PRs found"),
        "Should not show empty-state text while loading"
    );
}

#[test]
fn review_tab_shows_loading_indicator_during_refresh() {
    let mut app = make_app();
    // Load some PRs first
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_review_pr(1, "alice", ReviewDecision::ReviewRequired)],
    ));
    assert!(!app.review_board_loading());

    // Trigger a refresh — loading indicator should appear in the tab bar
    app.update(Message::RefreshReviewPrs);
    assert!(app.review_board_loading());

    let buf = render_to_buffer(&mut app, 120, 30);
    // ↻ (U+21BB) is the loading indicator shown in the tab label
    assert!(
        buffer_contains(&buf, "\u{21bb}"),
        "Tab bar should show loading indicator while refreshing"
    );
}

#[test]
fn review_tab_hides_loading_indicator_after_fetch() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_review_pr(1, "alice", ReviewDecision::ReviewRequired)],
    ));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "\u{21bb}"),
        "Tab bar should not show loading indicator after fetch completes"
    );
}

#[test]
fn review_board_renders_empty_state_after_fetch() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    assert!(!app.review_board_loading());

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "No PRs found"),
        "Should show empty state after fetch with no results"
    );
    assert!(
        !buffer_contains(&buf, "Loading..."),
        "Should not show loading text after fetch completes"
    );
}

#[test]
fn review_board_renders_persistent_error() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsFetchFailed(
        PrListKind::Review,
        "not authenticated".to_string(),
    ));
    assert_eq!(app.last_review_error(), Some("not authenticated"));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "not authenticated"),
        "Should show persistent error text in review board"
    );
}

#[test]
fn review_prs_loaded_clears_last_error() {
    let mut app = make_app();
    app.update(Message::PrsFetchFailed(
        PrListKind::Review,
        "auth error".to_string(),
    ));
    assert!(app.last_review_error().is_some());

    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    assert!(
        app.last_review_error().is_none(),
        "Successful fetch should clear last error"
    );
}

#[test]
fn handle_refresh_usage_stores_by_task_id() {
    use crate::models::TaskUsage;
    let mut app = make_app();
    let usage = vec![TaskUsage {
        task_id: TaskId(1),
        cost_usd: 0.42,
        input_tokens: 10_000,
        output_tokens: 2_000,
        cache_read_tokens: 500,
        cache_write_tokens: 100,
        updated_at: chrono::Utc::now(),
    }];
    app.update(Message::RefreshUsage(usage));
    assert!(app.board.usage.contains_key(&TaskId(1)));
    assert!((app.board.usage[&TaskId(1)].cost_usd - 0.42).abs() < 1e-9);
}

// --- Filter preset tests ---

#[test]
fn load_filter_preset_replaces_repo_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.filter.repos.insert("/repo-a".to_string());

    let preset_repos: HashSet<String> = ["/repo-b".to_string()].into_iter().collect();
    app.filter.presets = vec![("backend".to_string(), preset_repos, RepoFilterMode::Include)];

    app.update(Message::LoadFilterPreset("backend".to_string()));
    assert!(app.filter.repos.contains("/repo-b"));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn save_filter_preset_stores_and_persists() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.filter.repos.insert("/repo-a".to_string());
    app.input.mode = InputMode::RepoFilter;

    app.update(Message::StartSavePreset);
    assert_eq!(app.input.mode, InputMode::InputPresetName);

    let cmds = app.update(Message::SaveFilterPreset("frontend".to_string()));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert_eq!(app.filter.presets.len(), 1);
    assert_eq!(app.filter.presets[0].0, "frontend");
    assert!(app.filter.presets[0].1.contains("/repo-a"));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistFilterPreset { .. })));
}

#[test]
fn save_filter_preset_empty_name_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.update(Message::SaveFilterPreset("  ".to_string()));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(app.filter.presets.is_empty());
}

#[test]
fn save_filter_preset_overwrites_existing() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    let old: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("frontend".to_string(), old, RepoFilterMode::Include)];

    app.filter.repos.insert("/repo-b".to_string());
    app.update(Message::SaveFilterPreset("frontend".to_string()));
    assert_eq!(app.filter.presets.len(), 1);
    assert!(app.filter.presets[0].1.contains("/repo-b"));
}

#[test]
fn delete_filter_preset_removes_and_returns_command() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("frontend".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::ConfirmDeletePreset;

    let cmds = app.update(Message::DeleteFilterPreset("frontend".to_string()));
    assert!(app.filter.presets.is_empty());
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteFilterPreset(_))));
}

#[test]
fn cancel_preset_input_returns_to_repo_filter() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "draft".to_string();
    app.update(Message::CancelPresetInput);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(app.input.buffer.is_empty());
}

#[test]
fn filter_presets_loaded_sets_state() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.update(Message::FilterPresetsLoaded(vec![(
        "frontend".to_string(),
        repos.clone(),
        RepoFilterMode::Include,
    )]));
    assert_eq!(app.filter.presets.len(), 1);
    assert_eq!(app.filter.presets[0].0, "frontend");
}

#[test]
fn load_filter_preset_unknown_name_is_noop() {
    let mut app = make_app();
    app.filter.repos.insert("/repo-a".to_string());
    app.update(Message::LoadFilterPreset("nonexistent".to_string()));
    assert!(app.filter.repos.contains("/repo-a"));
}

#[test]
fn load_filter_preset_skips_stale_paths() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    // Preset contains a path that no longer exists in repo_paths
    let preset_repos: HashSet<String> = ["/repo-a".to_string(), "/gone".to_string()]
        .into_iter()
        .collect();
    app.filter.presets = vec![("stale".to_string(), preset_repos, RepoFilterMode::Include)];

    app.update(Message::LoadFilterPreset("stale".to_string()));
    assert!(app.filter.repos.contains("/repo-a"));
    assert!(
        !app.filter.repos.contains("/gone"),
        "Stale path should be excluded"
    );
}

#[test]
fn start_delete_preset_with_no_presets_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.update(Message::StartDeletePreset);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn repo_filter_s_key_starts_save_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Char('s')));
    assert_eq!(app.input.mode, InputMode::InputPresetName);
}

#[test]
fn repo_filter_x_key_starts_delete_preset() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo".to_string()].into_iter().collect();
    app.filter.presets = vec![("test".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmDeletePreset);
}

#[test]
fn repo_filter_shift_a_loads_first_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    let repos: HashSet<String> = ["/repo-b".to_string()].into_iter().collect();
    app.filter.presets = vec![("backend".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert!(app.filter.repos.contains("/repo-b"));
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn input_preset_name_enter_saves() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.filter.repos.insert("/repo-a".to_string());
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "mypreset".to_string();
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert_eq!(app.filter.presets.len(), 1);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistFilterPreset { .. })));
}

#[test]
fn input_preset_name_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "draft".to_string();
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn input_preset_name_typing_works() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.handle_key(make_key(KeyCode::Char('a')));
    app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(app.input.buffer, "ab");
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "a");
}

#[test]
fn confirm_delete_preset_letter_deletes() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo".to_string()].into_iter().collect();
    app.filter.presets = vec![("alpha".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::ConfirmDeletePreset;
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
    assert!(app.filter.presets.is_empty());
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteFilterPreset(_))));
}

#[test]
fn confirm_delete_preset_esc_cancels() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo".to_string()].into_iter().collect();
    app.filter.presets = vec![("alpha".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::ConfirmDeletePreset;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert_eq!(app.filter.presets.len(), 1);
}

#[test]
fn confirm_delete_preset_out_of_range_ignored() {
    let mut app = make_app();
    let repos: HashSet<String> = ["/repo".to_string()].into_iter().collect();
    app.filter.presets = vec![("alpha".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::ConfirmDeletePreset;
    app.handle_key(KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT));
    assert_eq!(app.input.mode, InputMode::ConfirmDeletePreset);
    assert_eq!(app.filter.presets.len(), 1);
}

// --- Overlay rendering tests ---

#[test]
fn repo_filter_overlay_shows_presets() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("frontend".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::RepoFilter;

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(buffer_contains(&buf, "A"), "Expected preset letter A");
    assert!(
        buffer_contains(&buf, "frontend"),
        "Expected preset name 'frontend'"
    );
}

#[test]
fn repo_filter_overlay_shows_name_input() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "myfilter".to_string();

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(buffer_contains(&buf, "Name:"), "Expected name input prompt");
    assert!(buffer_contains(&buf, "myfilter"), "Expected buffer content");
}

#[test]
fn repo_filter_overlay_shows_delete_help() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo-a".to_string()];
    let repos: HashSet<String> = ["/repo-a".to_string()].into_iter().collect();
    app.filter.presets = vec![("test".to_string(), repos, RepoFilterMode::Include)];
    app.input.mode = InputMode::ConfirmDeletePreset;

    let buf = render_to_buffer(&mut app, 80, 25);
    assert!(
        buffer_contains(&buf, "delete preset"),
        "Expected delete help text"
    );
}

// --- Epic batch wrap-up ---

fn make_review_subtask(id: i64, epic_id: i64, sort_order: i64) -> Task {
    let mut task = make_task(id, TaskStatus::Review);
    task.epic_id = Some(EpicId(epic_id));
    task.worktree = Some(format!("/repo/.worktrees/{id}-task-{id}"));
    task.sort_order = Some(sort_order);
    task
}

#[test]
fn w_key_on_epic_starts_epic_wrap_up() {
    let mut app = App::new(vec![make_review_subtask(1, 10, 1)], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];
    // Epic is in Review column (column 2)
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    app.handle_key(make_key(KeyCode::Char('W')));

    assert!(matches!(app.input.mode, InputMode::ConfirmEpicWrapUp(_)));
}

#[test]
fn epic_wrap_up_with_review_tasks_enters_confirm() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 1), make_review_subtask(2, 10, 2)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];

    app.update(Message::StartEpicWrapUp(EpicId(10)));

    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmEpicWrapUp(EpicId(10))
    ));
}

#[test]
fn epic_wrap_up_without_review_tasks_shows_info() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.epic_id = Some(EpicId(10));
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    app.update(Message::StartEpicWrapUp(EpicId(10)));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app
        .status
        .message
        .as_ref()
        .unwrap()
        .contains("No review tasks"));
}

#[test]
fn epic_wrap_up_rebase_creates_queue_and_emits_first_finish() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));

    let cmds = app.update(Message::EpicWrapUpRebase);

    assert_eq!(app.input.mode, InputMode::Normal);
    let queue = app.merge_queue.as_ref().expect("merge queue should exist");
    assert_eq!(queue.action, MergeAction::Rebase);
    // Task 2 has sort_order 1, so it comes first
    assert_eq!(queue.task_ids, vec![TaskId(2), TaskId(1)]);
    assert_eq!(queue.current, Some(TaskId(2)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Finish { id, .. } if *id == TaskId(2))));
}

#[test]
fn epic_wrap_up_finish_complete_advances_queue() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));
    app.update(Message::EpicWrapUpRebase);

    // First task completes
    let cmds = app.update(Message::FinishComplete(TaskId(2)));

    let queue = app.merge_queue.as_ref().expect("queue should still exist");
    assert_eq!(queue.completed, 1);
    assert_eq!(queue.current, Some(TaskId(1)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Finish { id, .. } if *id == TaskId(1))));
}

#[test]
fn epic_wrap_up_all_complete_clears_queue() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));
    app.update(Message::EpicWrapUpRebase);

    app.update(Message::FinishComplete(TaskId(2)));
    app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        app.merge_queue.is_none(),
        "queue should be cleared after all tasks complete"
    );
}

#[test]
fn epic_wrap_up_finish_failed_pauses_queue() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));
    app.update(Message::EpicWrapUpRebase);

    app.update(Message::FinishFailed {
        id: TaskId(2),
        error: "rebase conflict".to_string(),
        is_conflict: true,
    });

    let queue = app.merge_queue.as_ref().expect("queue should still exist");
    assert_eq!(queue.failed, Some(TaskId(2)));
    assert!(queue.current.is_none());
}

#[test]
fn epic_wrap_up_cancel_clears_queue() {
    let mut app = App::new(vec![make_review_subtask(1, 10, 1)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.merge_queue = Some(MergeQueue {
        epic_id: EpicId(10),
        action: MergeAction::Rebase,
        task_ids: vec![TaskId(1)],
        completed: 0,
        current: Some(TaskId(1)),
        failed: None,
    });

    app.update(Message::CancelMergeQueue);

    assert!(app.merge_queue.is_none());
}

#[test]
fn epic_wrap_up_pr_mode_advances_on_pr_created() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));
    app.update(Message::EpicWrapUpPr);

    let cmds = app.update(Message::PrCreated {
        id: TaskId(2),
        pr_url: "https://github.com/org/repo/pull/1".to_string(),
    });

    let queue = app.merge_queue.as_ref().expect("queue should still exist");
    assert_eq!(queue.completed, 1);
    assert_eq!(queue.current, Some(TaskId(1)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CreatePr { id, .. } if *id == TaskId(1))));
}

// ---------------------------------------------------------------------------
// SubStatus stale/crashed detection, escalation, and recovery
// ---------------------------------------------------------------------------

#[test]
fn stale_detection_sets_substatus_and_persists() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Stale);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn crashed_detection_sets_substatus_and_persists() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Crashed);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn stale_does_not_overwrite_crashed() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Crashed;

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Crashed); // unchanged
    assert!(cmds.is_empty()); // no persist needed
}

#[test]
fn stale_skips_non_running_task() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Backlog)], TEST_TIMEOUT);

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::None); // unchanged
    assert!(cmds.is_empty());
}

#[test]
fn crashed_skips_non_running_task() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Review)], TEST_TIMEOUT);

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::AwaitingReview); // unchanged
    assert!(cmds.is_empty());
}

#[test]
fn recovery_from_stale_resets_substatus_to_active() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(3),
        output: "new output".to_string(),
        activity_ts: 1,
    });
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn recovery_from_crashed_resets_substatus_to_active() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(3),
        output: "new output".to_string(),
        activity_ts: 1,
    });
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Active);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn active_task_output_does_not_emit_persist() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Active;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(3),
        output: "output".to_string(),
        activity_ts: 1,
    });
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Active); // unchanged
                                                    // No PersistTask since sub_status didn't change
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn stale_notification_sent_when_enabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { urgent: false, .. })));
}

#[test]
fn stale_notification_not_sent_when_disabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(false);

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn crashed_notification_sent_urgent_when_enabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { urgent: true, .. })));
}

#[test]
fn crashed_notification_not_sent_when_disabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(false);

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn tick_skips_already_stale_tasks() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    // Tick should NOT re-emit PersistTask for already-stale tasks
    // (only CaptureTmux and RefreshFromDb expected)
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn tick_skips_already_crashed_tasks() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn tick_skips_conflict_tasks_for_stale_detection() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Conflict;
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    assert!(!cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
    assert_eq!(app.board.tasks[0].sub_status, SubStatus::Conflict);
}

#[test]
fn refresh_from_stale_to_active_resets_last_active_at() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(300));

    let mut refreshed = make_task(3, TaskStatus::Running);
    refreshed.sub_status = SubStatus::Active;
    refreshed.tmux_window = Some("win-3".to_string());

    app.update(Message::RefreshTasks(vec![refreshed]));
    let elapsed = app.agents.last_active_at[&TaskId(3)].elapsed();
    assert!(elapsed < Duration::from_secs(1), "timer should be reset");
}

#[test]
fn refresh_staying_stale_does_not_reset_last_active_at() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    let old_instant = Instant::now() - Duration::from_secs(300);
    app.agents.last_active_at.insert(TaskId(3), old_instant);

    let mut refreshed = make_task(3, TaskStatus::Running);
    refreshed.sub_status = SubStatus::Stale;
    refreshed.tmux_window = Some("win-3".to_string());

    app.update(Message::RefreshTasks(vec![refreshed]));
    let elapsed = app.agents.last_active_at[&TaskId(3)].elapsed();
    assert!(
        elapsed > Duration::from_secs(200),
        "timer should NOT be reset when staying stale"
    );
}

#[test]
fn refresh_from_crashed_to_active_resets_last_active_at() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(3), Instant::now() - Duration::from_secs(300));

    let mut refreshed = make_task(3, TaskStatus::Running);
    refreshed.sub_status = SubStatus::Active;
    refreshed.tmux_window = Some("win-3".to_string());

    app.update(Message::RefreshTasks(vec![refreshed]));
    let elapsed = app.agents.last_active_at[&TaskId(3)].elapsed();
    assert!(elapsed < Duration::from_secs(1), "timer should be reset");
}

#[test]
fn move_task_forward_resets_substatus() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.find_task_mut(id).unwrap().sub_status = SubStatus::Stale;
    app.update(Message::MoveTask {
        id,
        direction: MoveDirection::Forward,
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
}

#[test]
fn move_task_backward_resets_substatus() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.update(Message::MoveTask {
        id,
        direction: MoveDirection::Backward,
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn render_shows_subcolumn_headers() {
    // make_app() has one Running task (SubStatus::Active) → Running column shows "── active" header
    let mut app = App::new(
        vec![make_task(1, TaskStatus::Running), {
            let mut t = make_task(2, TaskStatus::Running);
            t.sub_status = SubStatus::Stale;
            t
        }],
        TEST_TIMEOUT,
    );
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

#[test]
fn render_shows_parent_status_headers() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 160, 30);
    assert!(
        buffer_contains(&buf, "backlog"),
        "parent header 'backlog' not found"
    );
    assert!(
        buffer_contains(&buf, "running"),
        "parent header 'running' not found"
    );
    assert!(
        buffer_contains(&buf, "review"),
        "parent header 'review' not found"
    );
    assert!(
        buffer_contains(&buf, "done"),
        "parent header 'done' not found"
    );
}

#[test]
fn render_detail_shows_sub_status() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Navigate to the Active visual column (index 1)
    app.update(Message::NavigateColumn(1));
    // Open the detail panel
    app.update(Message::ToggleDetail);
    let buf = render_to_buffer(&mut app, 160, 30);
    assert!(
        buffer_contains(&buf, "(active)"),
        "detail panel should show sub-status '(active)'"
    );
}

// ---------------------------------------------------------------------------
// PrReviewState message handling
// ---------------------------------------------------------------------------

#[test]
fn pr_review_state_updates_substatus() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::AwaitingReview;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::Approved),
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.sub_status, SubStatus::Approved);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn pr_review_state_noop_when_unchanged() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::AwaitingReview;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: None, // maps to AwaitingReview
    });
    assert!(cmds.is_empty()); // no change, no persist
}

#[test]
fn pr_review_state_changes_requested() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::AwaitingReview;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::ChangesRequested),
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.sub_status, SubStatus::ChangesRequested);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn pr_review_state_ignores_non_review_task() {
    let mut app = make_app();
    let id = TaskId(3);
    // Task 3 is Running by default in make_app
    assert_eq!(app.find_task(id).unwrap().status, TaskStatus::Running);
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::Approved),
    });
    assert!(cmds.is_empty());
    // sub_status should not have changed
    assert_ne!(app.find_task(id).unwrap().sub_status, SubStatus::Approved);
}

#[test]
fn pr_review_state_preserves_conflict_substatus() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::Conflict;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::Approved),
    });
    assert!(cmds.is_empty());
    assert_eq!(app.find_task(id).unwrap().sub_status, SubStatus::Conflict);
}

// =====================================================================
// Input handler tests (tui/input.rs)
// =====================================================================

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
    // Start at column 0, row 0
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    // 'l' moves right
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_eq!(app.selection().column(), 1);

    // 'h' moves left
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.selection().column(), 0);

    // 'j' moves down
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.selection().row(0), 1);

    // 'k' moves up
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.selection().row(0), 0);
}

#[test]
fn handle_key_normal_quit_enters_confirm() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(!app.should_quit);
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
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
fn handle_key_normal_new_epic() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('E')));
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);
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
fn handle_key_confirm_archive_yes() {
    let mut app = make_app();
    // Select task 1 (backlog)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    app.input.mode = InputMode::ConfirmArchive;

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(*app.mode(), InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.status == TaskStatus::Archived)));
}

#[test]
fn handle_key_confirm_archive_cancel() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmArchive;

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
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
fn handle_key_confirm_done_yes() {
    let mut app = make_app();
    // Move task 3 (Running) to Review so ConfirmDone makes sense
    let task_3 = app
        .board
        .tasks
        .iter_mut()
        .find(|t| t.id == TaskId(3))
        .unwrap();
    task_3.status = TaskStatus::Review;
    app.input.mode = InputMode::ConfirmDone(TaskId(3));

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(*app.mode(), InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3) && t.status == TaskStatus::Done)));
}

#[test]
fn handle_key_confirm_done_cancel() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDone(TaskId(3));
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_confirm_wrap_up_rebase() {
    let mut app = make_app();
    let mut task = make_task(10, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/10-test".to_string());
    task.tmux_window = Some("main:10-test".to_string());
    app.board.tasks.push(task);
    app.input.mode = InputMode::ConfirmWrapUp(TaskId(10));

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Finish { id, .. } if *id == TaskId(10))));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_confirm_wrap_up_pr() {
    let mut app = make_app();
    let mut task = make_task(10, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/10-test".to_string());
    task.tmux_window = Some("main:10-test".to_string());
    app.board.tasks.push(task);
    app.input.mode = InputMode::ConfirmWrapUp(TaskId(10));

    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CreatePr { id, .. } if *id == TaskId(10))));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_confirm_wrap_up_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmWrapUp(TaskId(10));
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
        Command::OpenDescriptionEditor { is_epic: false }
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
        Command::OpenDescriptionEditor { is_epic: false }
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
fn handle_key_repo_filter_toggle() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('1')));
    assert!(app.filter.repos.contains("/repo"));
}

#[test]
fn handle_key_repo_filter_close_enter() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_repo_filter_close_esc() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_repo_filter_close_q() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_normal_dispatch_backlog_task() {
    let mut app = make_app();
    // Select task 1 (backlog)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { .. })));
}

#[test]
fn handle_key_normal_dispatch_running_task_with_window_shows_info() {
    let mut app = make_app();
    // Select running task (column 1)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
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
fn handle_key_normal_toggle_archive() {
    let mut app = make_app();
    assert!(!app.archive.visible);
    app.handle_key(make_key(KeyCode::Char('H')));
    assert!(app.archive.visible);
}

#[test]
fn handle_key_normal_enter_toggles_detail() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    assert!(!app.board.detail_visible);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.board.detail_visible);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(!app.board.detail_visible);
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
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

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
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

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
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds.is_empty());
    assert!(app.status.message.as_deref().unwrap().contains("No PR URL"));
}

#[test]
fn handle_key_normal_tab_switches_to_review_board() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(app.board.view_mode, ViewMode::ReviewBoard { .. }));
}

#[test]
fn handle_key_review_board_tab_switches_back() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Tab)); // to review board
    assert!(matches!(app.board.view_mode, ViewMode::ReviewBoard { .. }));
    app.handle_key(make_key(KeyCode::Tab)); // to security board
    assert!(matches!(
        app.board.view_mode,
        ViewMode::SecurityBoard { .. }
    ));
    app.handle_key(make_key(KeyCode::Tab)); // back to task board
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_epic_text_input_char_and_enter() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('E'))); // start epic creation
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);

    app.handle_key(make_key(KeyCode::Char('X')));
    assert_eq!(app.input.buffer, "X");

    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::InputEpicDescription);
}

#[test]
fn handle_key_epic_text_input_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_input_preset_name_enter_saves() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "my-preset".to_string();

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistFilterPreset { .. })));
}

#[test]
fn handle_key_input_preset_name_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::RepoFilter);
}

#[test]
fn handle_key_confirm_delete_preset_selects() {
    let mut app = make_app();
    app.filter.presets = vec![(
        "preset-a".to_string(),
        std::collections::HashSet::new(),
        RepoFilterMode::Include,
    )];
    app.input.mode = InputMode::ConfirmDeletePreset;

    let cmds = app.handle_key(make_key(KeyCode::Char('A')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteFilterPreset(_))));
}

#[test]
fn handle_key_confirm_delete_preset_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeletePreset;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::RepoFilter);
}

// ---------------------------------------------------------------------------
// Epic selection tests
// ---------------------------------------------------------------------------

#[test]
fn space_toggles_epic_selection() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Epic is at row 0 in Backlog column (no standalone tasks)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.epics.contains(&EpicId(10)));
}

#[test]
fn space_on_epic_toggle_off() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    // Select
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.epics.contains(&EpicId(10)));

    // Deselect
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(!app.select.epics.contains(&EpicId(10)));
}

#[test]
fn space_on_empty_column_no_epics_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    // Navigate to Review column (empty)
    app.update(Message::NavigateColumn(2));
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.select.epics.is_empty());
    assert!(app.select.tasks.is_empty());
}

#[test]
fn select_all_column_includes_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.epics.contains(&EpicId(10)));
}

#[test]
fn select_all_deselects_all_including_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    // Select all
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 1);
    assert_eq!(app.select.epics.len(), 1);

    // Deselect all
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
    assert!(app.select.epics.is_empty());
}

#[test]
fn select_all_column_with_only_epics() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10), make_epic(20)];

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
    assert_eq!(app.select.epics.len(), 2);
    assert!(app.select.epics.contains(&EpicId(10)));
    assert!(app.select.epics.contains(&EpicId(20)));
}

#[test]
fn esc_clears_epic_selection() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    assert_eq!(app.select.epics.len(), 1);

    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.epics.is_empty());
}

#[test]
fn esc_clears_mixed_selection() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.tasks.is_empty());
    assert!(app.select.epics.is_empty());
}

#[test]
fn batch_archive_selected_epics() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10), make_epic(20)];

    let cmds = app.update(Message::BatchArchiveEpics(vec![EpicId(10), EpicId(20)]));
    assert!(app.board.epics.is_empty(), "Both epics should be removed");
    assert!(!cmds.is_empty(), "Should emit commands");
}

#[test]
fn batch_archive_skips_epics_with_non_done_subtasks() {
    let mut task = make_task(1, TaskStatus::Running);
    task.epic_id = Some(EpicId(10));
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    let cmds = app.update(Message::BatchArchiveEpics(vec![EpicId(10)]));
    assert_eq!(
        app.board.epics.len(),
        1,
        "Epic with non-done subtask should not be archived"
    );
    assert!(cmds.is_empty(), "Should not emit commands for skipped epic");
}

#[test]
fn x_key_with_epic_selection_shows_count_in_confirm() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    app.update(Message::ToggleSelectEpic(EpicId(20)));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmArchive);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Archive 2 items? [y/n]")
    );
}

#[test]
fn batch_archive_mixed_tasks_and_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(app.input.mode, InputMode::ConfirmArchive);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Archive 2 items? [y/n]")
    );

    // Confirm
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Archived
    );
    assert!(app.board.epics.is_empty(), "Epic should be removed");
    assert!(app.select.tasks.is_empty());
    assert!(app.select.epics.is_empty());
    assert!(!cmds.is_empty());
}

#[test]
fn confirm_archive_y_archives_selected_epics() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    app.input.mode = InputMode::ConfirmArchive;

    app.handle_key(make_key(KeyCode::Char('y')));
    assert!(app.board.epics.is_empty());
    assert!(app.select.epics.is_empty());
}

#[test]
fn m_on_epic_moves_status_forward() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Cursor on Backlog column, row 0 (the epic)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::PersistEpic {
            id: EpicId(10),
            status: Some(TaskStatus::Running),
            ..
        }
    )));
}

#[test]
fn m_with_mixed_selection_moves_tasks_only() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelect(TaskId(1)));
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    // Cursor on the task (row 0) so 'm' triggers batch move, not epic move
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    app.handle_key(make_key(KeyCode::Char('m')));
    // Task should move forward
    assert_eq!(
        app.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
}

#[test]
fn render_selected_epic_shows_star_prefix() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "* "),
        "Selected epic should show * prefix"
    );
    assert!(
        buffer_contains(&buf, "Epic 10"),
        "Epic title should be visible"
    );
}

#[test]
fn render_unselected_epic_no_star() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Epic 10"),
        "Epic title should be visible"
    );
    // The epic renders with "  " prefix (2 spaces), not "* "
    assert!(
        !buffer_contains(&buf, "* "),
        "Unselected epic should not show * prefix"
    );
}

#[test]
fn render_batch_hints_with_epic_selection() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "1 selected"),
        "Should show selection count"
    );
    assert!(buffer_contains(&buf, "archive"), "Should show archive hint");
}

#[test]
fn render_column_header_checked_with_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];

    // Select both the task and the epic
    app.update(Message::SelectAllColumn);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[x]"),
        "Checkbox should be checked when all items selected"
    );
}

#[test]
fn refresh_epics_prunes_stale_epic_selections() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::ToggleSelectEpic(EpicId(10)));
    app.update(Message::ToggleSelectEpic(EpicId(99))); // non-existent

    // Refresh with only epic 10
    app.update(Message::RefreshEpics(vec![make_epic(10)]));
    assert!(app.select.epics.contains(&EpicId(10)));
    assert!(!app.select.epics.contains(&EpicId(99)));
}

#[test]
fn detach_tmux_single_sets_confirm_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-1".to_string());

    app.update(Message::DetachTmux(TaskId(1)));

    assert!(
        matches!(&app.input.mode, InputMode::ConfirmDetachTmux(ids) if ids == &[TaskId(1)]),
        "Expected ConfirmDetachTmux([1]), got {:?}",
        app.input.mode
    );
    assert!(app.status.message.is_some());
}

#[test]
fn confirm_detach_tmux_clears_window() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], TEST_TIMEOUT);
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
fn detach_tmux_noop_on_task_without_window() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)], TEST_TIMEOUT);
    // tmux_window is None by default from make_task

    let cmds = app.update(Message::DetachTmux(TaskId(1)));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty(), "should produce no commands");
}

#[test]
fn batch_detach_tmux() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Review),
            make_task(2, TaskStatus::Review),
        ],
        TEST_TIMEOUT,
    );
    app.board.tasks[0].tmux_window = Some("task-1".to_string());
    app.board.tasks[1].tmux_window = Some("task-2".to_string());

    app.update(Message::BatchDetachTmux(vec![TaskId(1), TaskId(2)]));
    let cmds = app.update(Message::ConfirmDetachTmux);

    assert!(
        app.board.tasks[0].tmux_window.is_none(),
        "task 1 window should be cleared"
    );
    assert!(
        app.board.tasks[1].tmux_window.is_none(),
        "task 2 window should be cleared"
    );

    let kill_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::KillTmuxWindow { .. }))
        .count();
    assert_eq!(kill_count, 2, "should kill 2 windows");

    let persist_count = cmds
        .iter()
        .filter(|c| matches!(c, Command::PersistTask(_)))
        .count();
    assert_eq!(persist_count, 2, "should persist 2 tasks");
}

// --- Repo cursor navigation ---

#[test]
fn quick_dispatch_j_moves_cursor_down() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn move_repo_cursor_down_wraps() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 1; // last
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 0, "should wrap to first");
}

#[test]
fn move_repo_cursor_up_wraps() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0; // first
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 1, "should wrap to last");
}

#[test]
fn quick_dispatch_enter_selects_cursor_repo() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec![
        "/repo1".to_string(),
        "/repo2".to_string(),
        "/repo3".to_string(),
    ];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 2; // third repo
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None } if draft.repo_path == "/repo3")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn review_board_d_dispatches_review_agent_when_path_known() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/home/user/Code/repo".to_string()];
    let mut pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    pr.repo = "org/repo".to_string();
    pr.head_ref = "fix-bug".to_string();
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert!(cmds.iter().any(
        |c| matches!(c, Command::DispatchReviewAgent(req) if req.repo == "/home/user/Code/repo")
    ));
}

#[test]
fn review_board_d_enters_repo_input_when_path_unknown() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);
    assert!(app.input.pending_dispatch.is_some());
}

#[test]
fn submit_dispatch_repo_path_dispatches_review_agent() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);

    // Trigger dispatch — no known paths, enters input mode
    app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);

    // Submit a repo path
    let cmds = app.update(Message::SubmitDispatchRepoPath("/tmp".to_string()));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(req) if req.repo == "/tmp")));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SaveRepoPath(p) if p == "/tmp")));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn fix_agent_dispatch_enters_repo_input_when_path_unknown() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/my-repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "CVE-2025-1234".to_string(),
        description: "desc".to_string(),
        package: Some("serde".to_string()),
        fixed_version: Some("1.0.1".to_string()),
    }));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);
    assert!(app.input.pending_dispatch.is_some());
}

#[test]
fn fix_agent_dispatch_resolves_known_path() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/home/user/Code/my-repo".to_string()];
    let cmds = app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/my-repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "CVE-2025-1234".to_string(),
        description: "desc".to_string(),
        package: Some("serde".to_string()),
        fixed_version: Some("1.0.1".to_string()),
    }));
    assert!(cmds.iter().any(
        |c| matches!(c, Command::DispatchFixAgent(req) if req.repo == "/home/user/Code/my-repo")
    ));
}

#[test]
fn cancel_dispatch_repo_path_clears_pending() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);
    app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);

    app.update(Message::CancelInput);
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.pending_dispatch.is_none());
}

#[test]
fn submit_dispatch_repo_path_dispatches_fix_agent() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    // Enter InputDispatchRepoPath via fix agent with unknown repo
    app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/my-repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "CVE-2025-1234".to_string(),
        description: "desc".to_string(),
        package: Some("serde".to_string()),
        fixed_version: Some("1.0.1".to_string()),
    }));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);

    // Submit a repo path
    let cmds = app.update(Message::SubmitDispatchRepoPath("/tmp".to_string()));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchFixAgent(req) if req.repo == "/tmp")));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SaveRepoPath(p) if p == "/tmp")));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn dispatch_repo_path_cursor_navigation() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".into(), "/b".into(), "/c".into()];
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);

    // Enter dispatch repo path mode
    app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);
    assert_eq!(app.input.repo_cursor, 0);

    // Navigate down with j
    app.handle_key(KeyEvent::from(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);

    // Navigate down again
    app.handle_key(KeyEvent::from(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 2);

    // Navigate up with k
    app.handle_key(KeyEvent::from(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn dispatch_repo_path_enter_selects_cursor_item() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".into(), "/var".into()];
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);

    // Enter dispatch repo path mode
    app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);

    // Navigate to second item
    app.handle_key(KeyEvent::from(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);

    // Press Enter to select
    let cmds = app.handle_key(KeyEvent::from(KeyCode::Enter));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(req) if req.repo == "/var")));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn dispatch_repo_path_empty_submit_no_paths_stays_in_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.review.review.set_prs(vec![pr]);
    app.update(Message::SwitchToReviewBoard);

    // Enter dispatch repo path mode (no saved paths)
    app.handle_key(KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);

    // Submit empty — should stay in mode since no paths available
    let cmds = app.update(Message::SubmitDispatchRepoPath(String::new()));
    assert!(cmds.is_empty());
    // Mode should NOT have changed to Normal — user needs to type a path or cancel
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);
}

#[test]
fn review_agent_dispatched_sets_status() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let cmds = app.update(Message::ReviewAgentDispatched {
        github_repo: "org/my-repo".to_string(),
        number: 99,
        tmux_window: "review-my-repo-99".to_string(),
        worktree: "/tmp/worktree".to_string(),
    });
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistReviewAgent { .. })));
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("my-repo#99"));
}

#[test]
fn review_agent_failed_sets_status() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let cmds = app.update(Message::ReviewAgentFailed {
        github_repo: "acme/app".to_string(),
        number: 42,
        error: "git fetch failed".to_string(),
    });
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("git fetch failed"));
}

#[test]
fn clamp_review_selection_clamps_approved_column() {
    // The Approved column is at index 3. Load a PR into it, set the row
    // selection beyond the end, clear the PR list, and verify the selection
    // is clamped to 0.
    let mut app = make_app();

    // Switch to the review board so that review_selection_mut() returns Some.
    app.update(Message::SwitchToReviewBoard);

    // Manually push a PR into the Approved column (index 3).
    app.review
        .review
        .set_prs(vec![make_review_pr(1, "alice", ReviewDecision::Approved)]);

    // Set the row selection for the Approved column to an out-of-bounds value.
    if let Some(sel) = app.review_selection_mut() {
        sel.selected_row[3] = 5;
    }

    // Now remove all PRs and trigger a clamp via ReviewPrsLoaded with an
    // empty list (which calls clamp_review_selection internally).
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));

    // The Approved column selection must have been clamped to 0.
    let row = app.review_selection().unwrap().selected_row[3];
    assert_eq!(
        row, 0,
        "Approved column (index 3) selection was not clamped"
    );
}

#[test]
fn review_repo_filter_hides_prs() {
    let mut app = make_app();
    let mut pr1 = make_review_pr(1, "alice", ReviewDecision::ReviewRequired);
    pr1.repo = "org/repo-a".to_string();
    let mut pr2 = make_review_pr(2, "bob", ReviewDecision::ReviewRequired);
    pr2.repo = "org/repo-b".to_string();
    app.review.review.set_prs(vec![pr1, pr2]);
    app.update(Message::SwitchToReviewBoard);

    // No filter — both visible
    assert_eq!(app.filtered_review_prs().len(), 2);

    // Filter to repo-a only
    app.review
        .review
        .repo_filter
        .insert("org/repo-a".to_string());
    assert_eq!(app.filtered_review_prs().len(), 1);
    assert_eq!(app.filtered_review_prs()[0].repo, "org/repo-a");
}

#[test]
fn review_repo_filter_f_opens_filter() {
    let mut app = make_app();
    app.review.review.set_prs(vec![make_review_pr(
        1,
        "alice",
        ReviewDecision::ReviewRequired,
    )]);
    app.update(Message::SwitchToReviewBoard);

    app.handle_key(KeyEvent::from(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);
}

#[test]
fn review_repo_filter_esc_closes() {
    let mut app = make_app();
    app.review.review.set_prs(vec![make_review_pr(
        1,
        "alice",
        ReviewDecision::ReviewRequired,
    )]);
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::StartReviewRepoFilter);
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);

    app.handle_key(KeyEvent::from(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn repo_cursor_resets_on_quick_dispatch_entry() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.repo_cursor = 1;
    app.update(Message::StartQuickDispatchSelection);
    assert_eq!(
        app.input.repo_cursor, 0,
        "cursor should reset to 0 on mode entry"
    );
}

#[test]
fn repo_filter_j_moves_cursor_down() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn repo_filter_space_toggles_cursor_repo() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 1; // pointing at /repo-b
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(
        app.filter.repos.contains("/repo-b"),
        "cursor repo should be toggled"
    );
    assert!(!app.filter.repos.contains("/repo-a"));
}

#[test]
fn review_repo_filter_toggle_all() {
    let mut app = make_app();
    let mut pr1 = make_review_pr(1, "alice", ReviewDecision::ReviewRequired);
    pr1.repo = "org/repo-a".to_string();
    let mut pr2 = make_review_pr(2, "bob", ReviewDecision::ReviewRequired);
    pr2.repo = "org/repo-b".to_string();
    app.review.review.set_prs(vec![pr1, pr2]);
    app.update(Message::SwitchToReviewBoard);

    // Toggle all on
    app.update(Message::ToggleAllReviewRepoFilter);
    assert_eq!(app.review.review.repo_filter.len(), 2);

    // Toggle all off
    app.update(Message::ToggleAllReviewRepoFilter);
    assert!(app.review.review.repo_filter.is_empty());
}

#[test]
fn review_repo_filter_toggle_single() {
    let mut app = make_app();
    let mut pr1 = make_review_pr(1, "alice", ReviewDecision::ReviewRequired);
    pr1.repo = "org/repo-a".to_string();
    app.review.review.set_prs(vec![pr1]);
    app.update(Message::SwitchToReviewBoard);

    // Toggle on
    app.update(Message::ToggleReviewRepoFilter("org/repo-a".to_string()));
    assert!(app.review.review.repo_filter.contains("org/repo-a"));

    // Toggle off
    app.update(Message::ToggleReviewRepoFilter("org/repo-a".to_string()));
    assert!(!app.review.review.repo_filter.contains("org/repo-a"));
}

#[test]
fn review_repo_filter_clamps_selection() {
    let mut app = make_app();
    let mut pr1 = make_review_pr(1, "alice", ReviewDecision::ReviewRequired);
    pr1.repo = "org/repo-a".to_string();
    let mut pr2 = make_review_pr(2, "bob", ReviewDecision::ReviewRequired);
    pr2.repo = "org/repo-b".to_string();
    app.review.review.set_prs(vec![pr1, pr2]);
    app.update(Message::SwitchToReviewBoard);

    // Select the second row
    if let Some(sel) = app.review_selection_mut() {
        sel.selected_row[0] = 1;
    }

    // Filter to only one PR, selection should clamp
    app.update(Message::ToggleReviewRepoFilter("org/repo-a".to_string()));
    let row = app.review_selection().unwrap().selected_row[0];
    assert_eq!(row, 0);
}

#[test]
fn review_repo_filter_selected_pr_uses_filter() {
    let mut app = make_app();
    let mut pr1 = make_review_pr(1, "alice", ReviewDecision::ReviewRequired);
    pr1.repo = "org/repo-a".to_string();
    let mut pr2 = make_review_pr(2, "bob", ReviewDecision::ReviewRequired);
    pr2.repo = "org/repo-b".to_string();
    app.review.review.set_prs(vec![pr1, pr2]);
    app.update(Message::SwitchToReviewBoard);

    // Without filter, first PR is selected
    let selected = app.selected_review_pr().unwrap();
    assert_eq!(selected.number, 1);

    // Filter to repo-b only, first visible PR should be #2
    app.review
        .review
        .repo_filter
        .insert("org/repo-b".to_string());
    let selected = app.selected_review_pr().unwrap();
    assert_eq!(selected.number, 2);
}

#[test]
fn review_board_default_mode_is_reviewer() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    match app.board.view_mode {
        ViewMode::ReviewBoard { mode, .. } => {
            assert_eq!(mode, ReviewBoardMode::Reviewer);
        }
        _ => panic!("expected ReviewBoard"),
    }
}

#[test]
fn my_prs_loaded_updates_state() {
    let mut app = make_app();
    let prs = vec![make_review_pr(101, "me", ReviewDecision::ReviewRequired)];
    app.update(Message::PrsLoaded(PrListKind::Authored, prs));
    assert_eq!(app.my_prs().len(), 1);
    assert_eq!(app.my_prs()[0].number, 101);
    assert!(!app.my_prs_loading());
}

#[test]
fn shift_tab_toggles_review_board_mode() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    match app.board.view_mode {
        ViewMode::ReviewBoard { mode, .. } => assert_eq!(mode, ReviewBoardMode::Reviewer),
        _ => panic!("expected ReviewBoard"),
    }
    // Toggle to Author
    app.update(Message::ToggleReviewBoardMode);
    match app.board.view_mode {
        ViewMode::ReviewBoard { mode, .. } => assert_eq!(mode, ReviewBoardMode::Author),
        _ => panic!("expected ReviewBoard"),
    }
    // Toggle to Dependabot
    app.update(Message::ToggleReviewBoardMode);
    match app.board.view_mode {
        ViewMode::ReviewBoard { mode, .. } => assert_eq!(mode, ReviewBoardMode::Dependabot),
        _ => panic!("expected ReviewBoard"),
    }
    // Toggle back to Reviewer
    app.update(Message::ToggleReviewBoardMode);
    match app.board.view_mode {
        ViewMode::ReviewBoard { mode, .. } => assert_eq!(mode, ReviewBoardMode::Reviewer),
        _ => panic!("expected ReviewBoard"),
    }
}

// ---------------------------------------------------------------------------
// Dependabot column placement
// ---------------------------------------------------------------------------

#[test]
fn dependabot_col_approved_wins_over_agent_status() {
    let mode = ReviewBoardMode::Dependabot;
    let pr = make_bot_pr(
        1,
        crate::models::ReviewDecision::Approved,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        crate::models::CiStatus::None,
    );
    assert_eq!(mode.pr_column(&pr), 2, "Approved should be column 2");
}

#[test]
fn dependabot_col_in_review_when_reviewing() {
    let mode = ReviewBoardMode::Dependabot;
    let pr = make_bot_pr(
        1,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        crate::models::CiStatus::None,
    );
    assert_eq!(mode.pr_column(&pr), 1, "Reviewing agent should be column 1");
}

#[test]
fn dependabot_col_in_review_when_findings_ready() {
    let mode = ReviewBoardMode::Dependabot;
    let pr = make_bot_pr(
        1,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::FindingsReady),
        crate::models::CiStatus::Success,
    );
    assert_eq!(mode.pr_column(&pr), 1, "FindingsReady should be column 1");
}

#[test]
fn dependabot_col_backlog_when_no_agent() {
    let mode = ReviewBoardMode::Dependabot;
    let pr = make_bot_pr(
        1,
        crate::models::ReviewDecision::ReviewRequired,
        None,
        crate::models::CiStatus::Success,
    );
    assert_eq!(
        mode.pr_column(&pr),
        0,
        "No agent should be column 0 (Backlog)"
    );
}

#[test]
fn dependabot_col_backlog_when_agent_idle() {
    let mode = ReviewBoardMode::Dependabot;
    let pr = make_bot_pr(
        1,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::Idle),
        crate::models::CiStatus::None,
    );
    assert_eq!(
        mode.pr_column(&pr),
        0,
        "Idle agent should be column 0 (Backlog)"
    );
}

#[test]
fn dependabot_sort_key_findings_ready_before_reviewing() {
    let mode = ReviewBoardMode::Dependabot;
    let findings = make_bot_pr(
        1,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::FindingsReady),
        crate::models::CiStatus::None,
    );
    let reviewing = make_bot_pr(
        2,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        crate::models::CiStatus::None,
    );
    assert!(
        mode.dependabot_sort_key(&findings) < mode.dependabot_sort_key(&reviewing),
        "FindingsReady should sort before Reviewing"
    );
    // Non-Dependabot modes return 0 for all PRs (no reordering)
    assert_eq!(ReviewBoardMode::Reviewer.dependabot_sort_key(&findings), 0);
    assert_eq!(ReviewBoardMode::Reviewer.dependabot_sort_key(&reviewing), 0);
}

#[test]
fn dependabot_column_count_is_3() {
    assert_eq!(ReviewBoardMode::Dependabot.column_count(), 3);
    assert_eq!(ReviewBoardMode::Reviewer.column_count(), 4);
    assert_eq!(ReviewBoardMode::Author.column_count(), 4);
}

#[test]
fn toggle_to_author_fetches_my_prs() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let cmds = app.update(Message::ToggleReviewBoardMode);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Authored))));
}

#[test]
fn toggle_review_board_mode_outside_review_board_is_noop() {
    let mut app = make_app();
    let cmds = app.update(Message::ToggleReviewBoardMode);
    assert!(cmds.is_empty());
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn active_review_prs_returns_reviewer_prs_in_reviewer_mode() {
    let mut app = make_app();
    app.review.review.set_prs(vec![make_review_pr(
        1,
        "alice",
        ReviewDecision::ReviewRequired,
    )]);
    app.review
        .authored
        .set_prs(vec![make_review_pr(2, "me", ReviewDecision::Approved)]);
    app.update(Message::SwitchToReviewBoard);
    assert_eq!(app.active_review_prs().len(), 1);
    assert_eq!(app.active_review_prs()[0].number, 1);
}

#[test]
fn active_review_prs_returns_my_prs_in_author_mode() {
    let mut app = make_app();
    app.review.review.set_prs(vec![make_review_pr(
        1,
        "alice",
        ReviewDecision::ReviewRequired,
    )]);
    app.review
        .authored
        .set_prs(vec![make_review_pr(2, "me", ReviewDecision::Approved)]);
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::ToggleReviewBoardMode);
    assert_eq!(app.active_review_prs().len(), 1);
    assert_eq!(app.active_review_prs()[0].number, 2);
}
// --- detach-aware section headers ---

#[test]
fn detached_review_task_shows_awaiting_merge_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.pr_url = Some("https://github.com/org/repo/pull/10".to_string());
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None; // detached
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting merge"),
        "detached review task should show 'awaiting merge' section header"
    );
}

#[test]
fn live_review_task_shows_awaiting_review_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.pr_url = Some("https://github.com/org/repo/pull/10".to_string());
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some("1-fix".to_string()); // live
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "live review task should show 'awaiting review' section header"
    );
    assert!(
        !buffer_contains(&buf, "awaiting merge"),
        "live review task should not show 'awaiting merge'"
    );
}

#[test]
fn detached_and_live_review_tasks_get_separate_sections() {
    // Live task (has tmux window)
    let mut live = make_task(1, TaskStatus::Review);
    live.sub_status = SubStatus::AwaitingReview;
    live.pr_url = Some("https://github.com/org/repo/pull/10".to_string());
    live.worktree = Some("/repo/.worktrees/1-fix".to_string());
    live.tmux_window = Some("1-fix".to_string());

    // Detached task (no tmux window)
    let mut detached = make_task(2, TaskStatus::Review);
    detached.sub_status = SubStatus::AwaitingReview;
    detached.pr_url = Some("https://github.com/org/repo/pull/11".to_string());
    detached.worktree = Some("/repo/.worktrees/2-feat".to_string());
    detached.tmux_window = None;

    let mut app = App::new(vec![live, detached], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "should show 'awaiting review' for live task"
    );
    assert!(
        buffer_contains(&buf, "awaiting merge"),
        "should show 'awaiting merge' for detached task"
    );
}

#[test]
fn is_detached_returns_true_for_review_without_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    assert!(task.is_detached());
}

#[test]
fn is_detached_returns_false_with_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some("1-fix".to_string());
    assert!(!task.is_detached());
}

#[test]
fn is_detached_returns_false_for_conflict() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Conflict;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    assert!(!task.is_detached());
}

// --- dispatch PR filter ---

#[test]
fn dispatch_pr_filter_toggles_on_d_in_author_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::ToggleReviewBoardMode); // switch to Author
    assert!(!app.dispatch_pr_filter());

    app.handle_key(make_key(KeyCode::Char('D')));
    assert!(
        app.dispatch_pr_filter(),
        "D should toggle dispatch filter on"
    );

    app.handle_key(make_key(KeyCode::Char('D')));
    assert!(!app.dispatch_pr_filter(), "D again should toggle it off");
}

#[test]
fn dispatch_pr_filter_noop_in_reviewer_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::SwitchToReviewBoard);
    // Default is Reviewer mode
    app.handle_key(make_key(KeyCode::Char('D')));
    assert!(
        !app.dispatch_pr_filter(),
        "D should be noop in Reviewer mode"
    );
}

#[test]
fn dispatch_pr_filter_filters_my_prs() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/acme/app/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Add two PRs: one matching a dispatch task, one not
    let matching_pr = make_review_pr(42, "me", ReviewDecision::ReviewRequired);
    let other_pr = make_review_pr(99, "me", ReviewDecision::ReviewRequired);
    app.review.authored.prs = vec![matching_pr, other_pr];

    // Without filter: both visible
    assert_eq!(app.filtered_my_prs().len(), 2);

    // With filter: only the matching one
    app.review.dispatch_pr_filter = true;
    let filtered = app.filtered_my_prs();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].number, 42);
}

// ---------------------------------------------------------------------------
// CardIndicator rendering tests
// ---------------------------------------------------------------------------

#[test]
fn render_card_conflict_shows_rebase_conflict() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Conflict;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "rebase conflict"),
        "Conflict task should show 'rebase conflict'"
    );
}

#[test]
fn render_card_detached_shows_detached() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = None; // detached: worktree present but no tmux
    task.sub_status = SubStatus::Active;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "detached"),
        "Task with worktree but no tmux_window should show 'detached'"
    );
}

#[test]
fn render_card_detached_review_shows_pr_label() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = None; // detached
    task.pr_url = Some("https://github.com/acme/app/pull/42".to_string());
    task.sub_status = SubStatus::AwaitingReview;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // move to Running
    app.update(Message::NavigateColumn(1)); // move to Review
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR #42"),
        "Detached review task with pr_url should show 'PR #42'"
    );
}

#[test]
fn render_card_blocked_shows_blocked() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::NeedsInput;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "blocked"),
        "Running task with NeedsInput sub_status should show 'blocked'"
    );
}

#[test]
fn render_card_running_shows_running() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "running"),
        "Active running task should show 'running'"
    );
}

#[test]
fn render_card_review_pr_shows_pr_number() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some("task-1".to_string());
    task.pr_url = Some("https://github.com/acme/app/pull/99".to_string());
    task.sub_status = SubStatus::AwaitingReview;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // move to Running
    app.update(Message::NavigateColumn(1)); // move to Review
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR #99"),
        "Review task with pr_url and tmux should show 'PR #99'"
    );
}

#[test]
fn render_card_done_merged_shows_merged() {
    let mut task = make_task(1, TaskStatus::Done);
    task.pr_url = Some("https://github.com/acme/app/pull/77".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.update(Message::NavigateColumn(1)); // Running
    app.update(Message::NavigateColumn(1)); // Review
    app.update(Message::NavigateColumn(1)); // Done
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR #77 merged"),
        "Done task with pr_url should show 'PR #77 merged'"
    );
}

#[test]
fn render_card_idle_with_plan_shows_triangle() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.plan_path = Some("docs/plans/plan.md".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Already in Backlog column (0)
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{25b8}"),
        "Backlog task with plan should show '▸' (U+25B8)"
    );
}

#[test]
fn render_card_idle_with_bug_tag() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Bug);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[bug]"),
        "Backlog task with Bug tag should show '[bug]'"
    );
}

#[test]
fn render_card_idle_with_feature_tag() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[feat]"),
        "Backlog task with Feature tag should show '[feat]'"
    );
}

#[test]
fn render_card_message_flash_shows_envelope() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Active;
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.agents.message_flash.insert(TaskId(1), Instant::now());
    app.update(Message::NavigateColumn(1)); // Running column
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "\u{2709}"),
        "Running task with message_flash set should show '\u{2709}' (envelope)"
    );
}

// ---------------------------------------------------------------------------
// Status bar rendering tests for all InputMode variants
// ---------------------------------------------------------------------------

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
fn render_status_bar_confirm_archive() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmArchive;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Archive task?"),
        "ConfirmArchive should show 'Archive task?'"
    );
}

#[test]
fn render_status_bar_confirm_done() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDone(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Done?"),
        "ConfirmDone should show 'Done?'"
    );
}

#[test]
fn render_status_bar_confirm_wrap_up() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmWrapUp(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "rebase"),
        "ConfirmWrapUp should show 'rebase'"
    );
    assert!(
        buffer_contains(&buf, "PR"),
        "ConfirmWrapUp should show 'PR'"
    );
}

#[test]
fn render_status_bar_confirm_delete_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Delete epic"),
        "ConfirmDeleteEpic should show 'Delete epic'"
    );
}

#[test]
fn render_status_bar_confirm_archive_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmArchiveEpic;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Archive epic"),
        "ConfirmArchiveEpic should show 'Archive epic'"
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
fn render_status_bar_epic_title() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: enter title"),
        "InputEpicTitle should show 'Creating epic: enter title'"
    );
}

#[test]
fn render_status_bar_epic_description() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: opening $EDITOR for description"),
        "InputEpicDescription should show 'Creating epic: opening $EDITOR for description'"
    );
}

#[test]
fn render_status_bar_epic_repo_path() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicRepoPath;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: enter repo path"),
        "InputEpicRepoPath should show 'Creating epic: enter repo path'"
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
fn render_status_bar_repo_filter() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Filter repos"),
        "RepoFilter mode should show 'Filter repos'"
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
fn render_status_bar_input_preset_name() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Enter preset name"),
        "InputPresetName mode should show 'Enter preset name'"
    );
}

#[test]
fn render_status_bar_confirm_delete_preset() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeletePreset;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "delete preset"),
        "ConfirmDeletePreset should show 'delete preset'"
    );
}

#[test]
fn render_status_bar_confirm_epic_wrap_up() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Epic wrap up"),
        "ConfirmEpicWrapUp should show 'Epic wrap up'"
    );
}

#[test]
fn render_status_bar_review_repo_filter() {
    let mut app = make_app();
    // Set mode directly (render_status_bar handles the text regardless of view)
    app.input.mode = InputMode::ReviewRepoFilter;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Filter repos"),
        "ReviewRepoFilter mode should show 'Filter repos'"
    );
}

#[test]
fn render_status_bar_confirm_edit_task() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEditTask(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Edit task?"),
        "ConfirmEditTask should show 'Edit task?'"
    );
}

#[test]
fn render_status_bar_confirm_batch_approve() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmBatchApprove(vec![
        "https://github.com/org/repo/pull/1".to_string(),
        "https://github.com/org/repo/pull/2".to_string(),
    ]);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Approve 2 PRs?"),
        "ConfirmBatchApprove with 2 URLs should show 'Approve 2 PRs?'"
    );
}

#[test]
fn render_status_bar_confirm_batch_merge() {
    let mut app = make_app();
    app.input.mode =
        InputMode::ConfirmBatchMerge(vec!["https://github.com/org/repo/pull/1".to_string()]);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Merge 1 PRs?"),
        "ConfirmBatchMerge with 1 URL should show 'Merge 1 PRs?'"
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

// ---------------------------------------------------------------------------
// Input form rendering tests
// ---------------------------------------------------------------------------

#[test]
fn render_input_form_title_shows_new_task_block() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTitle;
    app.input.buffer = "My new task".to_string();
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
        base_branch: "main".to_string(),
        ..Default::default()
    });
    app.input.buffer = "main".to_string();
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
    app.input.buffer = String::new();
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
    let now = chrono::Utc::now();
    let crashed_task = Task {
        id: TaskId(5),
        title: "Crashed task".to_string(),
        description: String::new(),
        repo_path: "/repo".to_string(),
        status: TaskStatus::Running,
        worktree: Some("/tmp/wt".to_string()),
        tmux_window: Some("win5".to_string()),
        plan_path: None,
        epic_id: None,
        sub_status: SubStatus::Crashed,
        pr_url: None,
        tag: None,
        sort_order: None,
        base_branch: "main".to_string(),
        created_at: now,
        updated_at: now,
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
fn render_input_form_epic_title_shows_new_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer = "My epic".to_string();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Title:"),
        "'Title:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "My epic"),
        "buffer text 'My epic' should be visible"
    );
}

#[test]
fn render_input_form_epic_description_shows_fields() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic title".to_string(),
        ..Default::default()
    });
    app.input.buffer = "Epic desc".to_string();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Epic title"),
        "completed title 'Epic title' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Description:"),
        "'Description:' label should be visible"
    );
}

#[test]
fn render_input_form_epic_repo_path_shows_repos() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic title".to_string(),
        description: "Epic desc".to_string(),
        ..Default::default()
    });
    app.input.buffer = String::new();
    app.board.repo_paths = vec!["/repo/x".to_string()];
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Repo path:"),
        "'Repo path:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "/repo/x"),
        "repo path '/repo/x' should be listed"
    );
}

// --- render_epic_banner tests ---

#[test]
fn render_epic_banner_shows_title() {
    let mut app = make_app();
    let mut epic = make_epic(10);
    epic.title = "Auth Refactor".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Auth Refactor"),
        "epic banner should show the epic title 'Auth Refactor'"
    );
}

#[test]
fn render_epic_banner_not_shown_in_board_view() {
    let mut app = make_app();
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    // Stay in default Board view — do not switch to ViewMode::Epic
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "Esc to return"),
        "epic banner should not be shown in Board view"
    );
}

// --- render_detail tests (task and epic) ---

#[test]
fn render_detail_task_with_tag_shows_tag() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Bug);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[bug]"),
        "detail panel should show '[bug]' tag for a task with tag=Bug"
    );
}

#[test]
fn render_detail_task_with_pr_url() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/acme/app/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Navigate to Review column (index 2)
    app.update(Message::NavigateColumn(2));
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 160, 30);
    assert!(
        buffer_contains(&buf, "PR: https://github.com/acme/app/pull/42"),
        "detail panel should show the PR URL"
    );
}

#[test]
fn render_detail_task_with_epic_reference() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.epic_id = Some(EpicId(10));
    let mut epic = make_epic(10);
    epic.title = "Auth Epic".to_string();
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.epics = vec![epic];
    // Switch to Epic view so the subtask is visible (Board view hides epic subtasks)
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 160, 30);
    assert!(
        buffer_contains(&buf, "Epic: Auth Epic"),
        "detail panel should show 'Epic: Auth Epic' for a task linked to that epic"
    );
}

#[test]
fn render_detail_task_with_usage_shows_cost() {
    use crate::models::TaskUsage;
    let task = make_task(1, TaskStatus::Running);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Navigate to Running column (index 1)
    app.update(Message::NavigateColumn(1));
    app.board.detail_visible = true;
    app.board.usage.insert(
        TaskId(1),
        TaskUsage {
            task_id: TaskId(1),
            cost_usd: 1.23,
            input_tokens: 50_000,
            output_tokens: 10_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            updated_at: chrono::Utc::now(),
        },
    );
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "$1.23"),
        "detail panel should show usage cost '$1.23'"
    );
}

#[test]
fn render_detail_epic_shows_title_and_id() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.title = "Platform Migration".to_string();
    app.board.epics = vec![epic];
    // Epic is the only item in Backlog column (no standalone tasks)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Platform Migration"),
        "epic detail should show the title 'Platform Migration'"
    );
    assert!(
        buffer_contains(&buf, "#10"),
        "epic detail should show the id '#10'"
    );
}

#[test]
fn render_detail_epic_with_plan_shows_path() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plans/migration.md".to_string());
    app.board.epics = vec![epic];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "plan: docs/plans/migration.md"),
        "epic detail should show 'plan: docs/plans/migration.md'"
    );
}

#[test]
fn render_detail_epic_shows_subtask_list() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    let mut t1 = make_task(101, TaskStatus::Done);
    t1.title = "Subtask Alpha".to_string();
    t1.epic_id = Some(EpicId(10));
    let mut t2 = make_task(102, TaskStatus::Running);
    t2.title = "Subtask Beta".to_string();
    t2.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t1, t2];

    // Epic is in Backlog; subtasks are in other columns so won't appear as
    // standalone items in column 0. The epic itself is the first item.
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Subtask Alpha"),
        "epic detail should list subtask 'Subtask Alpha'"
    );
    assert!(
        buffer_contains(&buf, "Subtask Beta"),
        "epic detail should list subtask 'Subtask Beta'"
    );
}

#[test]
fn render_detail_epic_subtask_conflict_shows_warning() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    let mut t1 = make_task(201, TaskStatus::Running);
    t1.title = "Conflicted Task".to_string();
    t1.epic_id = Some(EpicId(10));
    t1.sub_status = SubStatus::Conflict;
    app.board.tasks = vec![t1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "conflict"),
        "epic detail should show 'conflict' warning for subtask with Conflict sub_status"
    );
}

#[test]
fn render_detail_no_selection_shows_message() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.detail_visible = true;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "No task selected"),
        "detail panel should show 'No task selected' when there are no items"
    );
}

// ---------------------------------------------------------------------------
// Repo filter overlay tests
// ---------------------------------------------------------------------------

#[test]
fn render_repo_filter_overlay_shows_title() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Repo Filter"),
        "repo filter overlay should show 'Repo Filter' title"
    );
}

#[test]
fn render_repo_filter_overlay_shows_repos() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/alpha".to_string(), "/repo/beta".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "/repo/alpha"),
        "repo filter overlay should show '/repo/alpha'"
    );
    assert!(
        buffer_contains(&buf, "/repo/beta"),
        "repo filter overlay should show '/repo/beta'"
    );
}

#[test]
fn render_repo_filter_overlay_shows_include_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "include"),
        "repo filter overlay should show 'include' as default mode"
    );
}

#[test]
fn render_repo_filter_overlay_shows_navigate_hint() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.input.mode = InputMode::RepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "navigate"),
        "repo filter overlay should show 'navigate' hint"
    );
}

#[test]
fn render_repo_filter_input_preset_name() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string()];
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "my-preset".to_string();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Name:"),
        "preset name input should show 'Name:' label"
    );
    assert!(
        buffer_contains(&buf, "my-preset"),
        "preset name input should show the buffer content 'my-preset'"
    );
}

#[test]
fn render_repo_filter_confirm_delete_preset() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo/a".to_string()];
    app.input.mode = InputMode::ConfirmDeletePreset;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "delete preset"),
        "confirm delete mode should show 'delete preset' text"
    );
}

// ---------------------------------------------------------------------------
// Review repo filter overlay tests
// ---------------------------------------------------------------------------

#[test]
fn render_review_repo_filter_overlay_shows_title() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_review_pr(1, "alice", ReviewDecision::ReviewRequired)],
    ));
    app.input.mode = InputMode::ReviewRepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Review Repo Filter"),
        "review repo filter overlay should show 'Review Repo Filter' title"
    );
}

#[test]
fn render_review_repo_filter_overlay_shows_repos() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![make_review_pr(1, "alice", ReviewDecision::ReviewRequired)],
    ));
    app.input.mode = InputMode::ReviewRepoFilter;
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "acme/app"),
        "review repo filter overlay should show 'acme/app' repo from loaded PRs"
    );
}

// ---------------------------------------------------------------------------
// Tab bar mode tests
// ---------------------------------------------------------------------------

#[test]
fn render_tab_bar_epic_mode_shows_epic_title() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.title = "Platform Work".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Platform Work"),
        "tab bar in epic mode should show the epic title"
    );
}

#[test]
fn render_tab_bar_epic_mode_replaces_tasks_tab() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.title = "Platform Work".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        !buffer_contains(&buf, "Tasks"),
        "epic tab should replace the Tasks tab, not appear alongside it"
    );
}

#[test]
fn render_tab_bar_board_mode_shows_tasks_label() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Tasks"),
        "tab bar in board mode should show 'Tasks' label"
    );
}

#[test]
fn render_tab_bar_review_board_shows_reviews_active() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Reviews"),
        "tab bar in review board mode should show 'Reviews' label"
    );
}

#[test]
fn render_tab_bar_review_board_shows_pr_count() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr(1, "alice", ReviewDecision::ReviewRequired),
            make_review_pr(2, "bob", ReviewDecision::ReviewRequired),
        ],
    ));
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Reviews (2)"),
        "tab bar in review board mode should show 'Reviews (2)' when 2 PRs loaded"
    );
}

#[test]
fn render_tab_bar_review_board_my_prs_tab() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(
        PrListKind::Authored,
        vec![make_review_pr(1, "me", ReviewDecision::Approved)],
    ));
    // Toggle to Author mode so My PRs tab is active
    app.update(Message::ToggleReviewBoardMode);
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "My PRs (1)"),
        "tab bar in review board mode should show 'My PRs (1)' when 1 author PR loaded"
    );
}

#[test]
fn render_tab_bar_review_board_dependabot_tab() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(
        PrListKind::Bot,
        vec![
            make_review_pr(1, "dependabot", ReviewDecision::ReviewRequired),
            make_review_pr(2, "dependabot", ReviewDecision::ReviewRequired),
        ],
    ));
    // Toggle to Dependabot mode (Reviewer → Author → Dependabot)
    app.update(Message::ToggleReviewBoardMode);
    app.update(Message::ToggleReviewBoardMode);
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        buffer_contains(&buf, "Dependabot (2)"),
        "tab bar in review board mode should show 'Dependabot (2)' when 2 bot PRs loaded"
    );
}

// ---------------------------------------------------------------------------
// Tab bar key hint highlighting
// ---------------------------------------------------------------------------

/// Find a text span in the buffer and return the style of its first character.
fn find_style_of(buf: &Buffer, text: &str) -> Option<ratatui::style::Style> {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let remaining = (area.right() - x) as usize;
            if remaining < text.len() {
                break;
            }
            let segment: String = (0..text.len() as u16)
                .map(|dx| buf[(x + dx, y)].symbol().to_string())
                .collect();
            if segment == text {
                return Some(buf[(x, y)].style());
            }
        }
    }
    None
}

#[test]
fn tab_bar_board_mode_highlights_tab_key() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 100, 30);
    let style = find_style_of(&buf, "[Tab]").expect("[Tab] text not found in buffer");
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "[Tab] should be bold"
    );
    assert_eq!(
        style.fg,
        Some(Color::Rgb(120, 124, 153)),
        "[Tab] should use MUTED_LIGHT color"
    );
}

#[test]
fn tab_bar_review_board_highlights_keys() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    let buf = render_to_buffer(&mut app, 120, 30);

    let tab_style = find_style_of(&buf, "[Tab]").expect("[Tab] not found");
    assert!(tab_style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(tab_style.fg, Some(Color::Rgb(120, 124, 153)));

    let stab_style = find_style_of(&buf, "[S-Tab]").expect("[S-Tab] not found");
    assert!(stab_style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(stab_style.fg, Some(Color::Rgb(120, 124, 153)));

    // Description text "security" should use MUTED (not highlighted)
    let sec_style = find_style_of(&buf, "security").expect("'security' not found");
    assert_eq!(
        sec_style.fg,
        Some(Color::Rgb(86, 95, 137)),
        "description text should use MUTED color"
    );
}

#[test]
fn tab_bar_security_board_highlights_keys() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    let buf = render_to_buffer(&mut app, 120, 30);

    let tab_style = find_style_of(&buf, "[Tab]").expect("[Tab] not found");
    assert!(tab_style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(tab_style.fg, Some(Color::Rgb(120, 124, 153)));

    let esc_style = find_style_of(&buf, "[Esc]").expect("[Esc] not found");
    assert!(esc_style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(esc_style.fg, Some(Color::Rgb(120, 124, 153)));

    // Description text "back" should use MUTED
    let back_style = find_style_of(&buf, "back").expect("'back' not found");
    assert_eq!(
        back_style.fg,
        Some(Color::Rgb(86, 95, 137)),
        "description text should use MUTED color"
    );
}

// ---------------------------------------------------------------------------
// Review board Author and Dependabot mode rendering tests
// ---------------------------------------------------------------------------

#[test]
fn render_review_board_author_shows_my_pr_titles() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(
        PrListKind::Authored,
        vec![make_review_pr(42, "me", ReviewDecision::ReviewRequired)],
    ));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR 42"),
        "author mode should show 'PR 42' for the loaded my-PR"
    );
}

#[test]
fn render_review_board_author_shows_column_headers() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(
        PrListKind::Authored,
        vec![make_review_pr(42, "me", ReviewDecision::ReviewRequired)],
    ));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Needs Review"),
        "author mode should show 'Needs Review' column header"
    );
}

#[test]
fn render_review_board_dependabot_shows_bot_prs() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(
        PrListKind::Bot,
        vec![make_review_pr(
            100,
            "dependabot",
            ReviewDecision::ReviewRequired,
        )],
    ));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    app.update(Message::ToggleReviewBoardMode); // Author → Dependabot
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "PR 100"),
        "dependabot mode should show 'PR 100' for the loaded bot-PR"
    );
}

#[test]
fn render_review_board_dependabot_shows_lifecycle_columns() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(
        PrListKind::Bot,
        vec![make_review_pr(
            100,
            "dependabot",
            ReviewDecision::ReviewRequired,
        )],
    ));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    app.update(Message::ToggleReviewBoardMode); // Author → Dependabot
    let buf = render_to_buffer(&mut app, 120, 30);
    let has_lifecycle_column = buffer_contains(&buf, "Backlog")
        || buffer_contains(&buf, "In Review")
        || buffer_contains(&buf, "Approved");
    assert!(
        has_lifecycle_column,
        "dependabot mode should show lifecycle column headers (Backlog, In Review, or Approved)"
    );
}

#[test]
fn render_review_board_author_empty_shows_no_prs() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(PrListKind::Authored, vec![]));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "No PRs found"),
        "author mode with no my-PRs should show 'No PRs found'"
    );
}

#[test]
fn render_review_board_dependabot_empty_shows_no_prs() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![]));
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![]));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    app.update(Message::ToggleReviewBoardMode); // Author → Dependabot
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "No PRs found"),
        "dependabot mode with no bot-PRs should show 'No PRs found'"
    );
}

// ---------------------------------------------------------------------------
// Bot error / not-configured status bar tests
// ---------------------------------------------------------------------------

#[test]
fn last_bot_error_returns_bot_list_error() {
    let mut app = make_app();
    app.update(Message::PrsFetchFailed(
        PrListKind::Bot,
        "not configured".to_string(),
    ));
    assert_eq!(app.last_bot_error(), Some("not configured"));
}

#[test]
fn render_review_board_dependabot_shows_not_configured_in_status_bar() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsFetchFailed(
        PrListKind::Bot,
        "Bot queries not configured — press [e] to add your org filter".to_string(),
    ));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    app.update(Message::ToggleReviewBoardMode); // Author → Dependabot
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "not configured"),
        "Dependabot mode should show persistent not-configured error in status bar"
    );
}

#[test]
fn render_review_board_reviewer_mode_does_not_show_bot_error() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsFetchFailed(
        PrListKind::Bot,
        "bot error should not appear".to_string(),
    ));
    // Stay in Reviewer mode (no ToggleReviewBoardMode)
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "bot error should not appear"),
        "Reviewer mode must not show the bot error"
    );
}

#[test]
fn render_review_board_author_mode_shows_review_error_not_bot_error() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsFetchFailed(
        PrListKind::Review,
        "review fetch error".to_string(),
    ));
    app.update(Message::PrsFetchFailed(
        PrListKind::Bot,
        "bot error should not appear".to_string(),
    ));
    app.update(Message::ToggleReviewBoardMode); // Reviewer → Author
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "bot error should not appear"),
        "Author mode must not show the bot error"
    );
}

// ---------------------------------------------------------------------------
// Merge PR tests
// ---------------------------------------------------------------------------

fn make_approved_review_task(id: i64) -> Task {
    let mut task = make_task(id, TaskStatus::Review);
    task.pr_url = Some(format!("https://github.com/owner/repo/pull/{id}"));
    task.sub_status = SubStatus::Approved;
    task.worktree = Some(format!("/repo/.worktrees/{id}-task-{id}"));
    task
}

#[test]
fn merge_pr_key_on_approved_task_enters_confirm_mode() {
    let mut app = App::new(vec![make_approved_review_task(1)], TEST_TIMEOUT);
    // Navigate to review column
    app.update(Message::NavigateColumn(1)); // running
    app.update(Message::NavigateColumn(1)); // review

    let cmds = app.handle_key(make_key(KeyCode::Char('P')));
    assert!(cmds.is_empty());
    assert!(matches!(
        app.input.mode,
        InputMode::ConfirmMergePr(TaskId(1))
    ));
}

#[test]
fn merge_pr_key_on_non_review_task_shows_status() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);

    let cmds = app.handle_key(make_key(KeyCode::Char('P')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("not in review"));
}

#[test]
fn merge_pr_key_on_review_without_pr_url_shows_status() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.sub_status = SubStatus::Approved;
            t
        }],
        TEST_TIMEOUT,
    );
    app.update(Message::NavigateColumn(1)); // running
    app.update(Message::NavigateColumn(1)); // review

    let cmds = app.handle_key(make_key(KeyCode::Char('P')));
    assert!(cmds.is_empty());
    assert!(app.status.message.as_deref().unwrap().contains("no PR"));
}

#[test]
fn merge_pr_key_on_awaiting_review_shows_status() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.pr_url = Some("https://github.com/owner/repo/pull/1".to_string());
            t.sub_status = SubStatus::AwaitingReview;
            t
        }],
        TEST_TIMEOUT,
    );
    app.update(Message::NavigateColumn(1)); // running
    app.update(Message::NavigateColumn(1)); // review

    let cmds = app.handle_key(make_key(KeyCode::Char('P')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("awaiting review"));
}

#[test]
fn confirm_merge_pr_emits_merge_command() {
    let mut app = App::new(vec![make_approved_review_task(1)], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmMergePr(TaskId(1));

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::MergePr { id: TaskId(1), pr_url } if pr_url == "https://github.com/owner/repo/pull/1"
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn cancel_merge_pr_resets_mode() {
    let mut app = App::new(vec![make_approved_review_task(1)], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmMergePr(TaskId(1));

    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn merge_pr_failed_sets_status_message() {
    let mut app = App::new(vec![make_approved_review_task(1)], TEST_TIMEOUT);

    let cmds = app.update(Message::MergePrFailed {
        id: TaskId(1),
        error: "CI checks failing".to_string(),
    });
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("CI checks failing"));
}

// ---------------------------------------------------------------------------
// Title truncation — cards must truncate titles to fit column width
// ---------------------------------------------------------------------------

#[test]
fn task_card_title_truncated_in_narrow_terminal() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.title = "This is a very long task title that should be truncated".to_string();
    let mut app = App::new(vec![task], TEST_TIMEOUT);

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

#[test]
fn task_card_short_title_not_truncated_in_wide_terminal() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.title = "Short".to_string();
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Wide terminal: plenty of room
    let buf = render_to_buffer(&mut app, 200, 10);
    assert!(
        buffer_contains(&buf, "Short"),
        "short title should appear in full"
    );
}

#[test]
fn task_card_title_adapts_to_terminal_width() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.title = "Medium length title here".to_string();
    let mut app_narrow = App::new(vec![task.clone()], TEST_TIMEOUT);
    let mut app_wide = App::new(vec![task], TEST_TIMEOUT);

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

#[test]
fn epic_card_title_truncated_in_narrow_terminal() {
    let mut epic = make_epic(1);
    epic.title = "This is a very long epic title that should be truncated to fit".to_string();
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.update(Message::RefreshEpics(vec![epic]));

    let buf = render_to_buffer(&mut app, 80, 10);
    assert!(
        !buffer_contains(
            &buf,
            "This is a very long epic title that should be truncated to fit"
        ),
        "full epic title should be truncated in narrow terminal"
    );
}

// ---------------------------------------------------------------------------
// Repo grouping in review/security columns
// ---------------------------------------------------------------------------

#[test]
fn active_prs_for_column_sorts_by_repo() {
    let mut app = make_app();
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr_for_repo(1, "alice", ReviewDecision::ReviewRequired, "org/zebra"),
            make_review_pr_for_repo(2, "bob", ReviewDecision::ReviewRequired, "org/alpha"),
            make_review_pr_for_repo(3, "carol", ReviewDecision::ReviewRequired, "org/middle"),
        ],
    ));
    app.update(Message::SwitchToReviewBoard);

    let col = ReviewDecision::ReviewRequired.column_index();
    let prs = app.active_prs_for_column(col);
    assert_eq!(prs.len(), 3);
    assert_eq!(prs[0].repo, "org/alpha");
    assert_eq!(prs[1].repo, "org/middle");
    assert_eq!(prs[2].repo, "org/zebra");
}

#[test]
fn selected_review_pr_agrees_with_sorted_order() {
    let mut app = make_app();
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr_for_repo(1, "alice", ReviewDecision::ReviewRequired, "org/zebra"),
            make_review_pr_for_repo(2, "bob", ReviewDecision::ReviewRequired, "org/alpha"),
        ],
    ));
    app.update(Message::SwitchToReviewBoard);

    // Row 0 should be "org/alpha" (sorted first), row 1 should be "org/zebra"
    let pr0 = app.selected_review_pr().unwrap();
    assert_eq!(
        pr0.repo, "org/alpha",
        "row 0 should be the alphabetically first repo"
    );

    app.navigate_review_row(1);
    let pr1 = app.selected_review_pr().unwrap();
    assert_eq!(
        pr1.repo, "org/zebra",
        "row 1 should be the alphabetically second repo"
    );
}

#[test]
fn active_prs_for_column_preserves_order_within_same_repo() {
    let mut app = make_app();
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr_for_repo(10, "alice", ReviewDecision::ReviewRequired, "org/alpha"),
            make_review_pr_for_repo(5, "bob", ReviewDecision::ReviewRequired, "org/alpha"),
            make_review_pr_for_repo(20, "carol", ReviewDecision::ReviewRequired, "org/alpha"),
        ],
    ));
    app.update(Message::SwitchToReviewBoard);

    let col = ReviewDecision::ReviewRequired.column_index();
    let prs = app.active_prs_for_column(col);
    assert_eq!(prs.len(), 3);
    // Stable sort: original insertion order preserved within same repo
    assert_eq!(prs[0].number, 10);
    assert_eq!(prs[1].number, 5);
    assert_eq!(prs[2].number, 20);
}

#[test]
fn security_alerts_for_column_sorts_by_repo() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    app.update(Message::SecurityAlertsLoaded(vec![
        make_security_alert(1, "org/zebra", crate::models::AlertSeverity::Critical),
        make_security_alert(2, "org/alpha", crate::models::AlertSeverity::Critical),
        make_security_alert(3, "org/middle", crate::models::AlertSeverity::Critical),
    ]));

    let col = crate::models::AlertSeverity::Critical.column_index();
    let alerts = app.security_alerts_for_column(col);
    assert_eq!(alerts.len(), 3);
    assert_eq!(alerts[0].repo, "org/alpha");
    assert_eq!(alerts[1].repo, "org/middle");
    assert_eq!(alerts[2].repo, "org/zebra");
}

#[test]
fn selected_security_alert_agrees_with_sorted_order() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    app.update(Message::SecurityAlertsLoaded(vec![
        make_security_alert(1, "org/zebra", crate::models::AlertSeverity::Critical),
        make_security_alert(2, "org/alpha", crate::models::AlertSeverity::Critical),
    ]));

    // Default selection is column 0 (Critical). Row 0 should be "org/alpha" (sorted first).
    let a0 = app.selected_security_alert().unwrap();
    assert_eq!(
        a0.repo, "org/alpha",
        "row 0 should be the alphabetically first repo"
    );

    app.navigate_security_row(1);
    let a1 = app.selected_security_alert().unwrap();
    assert_eq!(
        a1.repo, "org/zebra",
        "row 1 should be the alphabetically second repo"
    );
}

// ---------------------------------------------------------------------------
// Delete repo path tests
// ---------------------------------------------------------------------------

#[test]
fn start_delete_repo_path_enters_confirm_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.update(Message::StartDeleteRepoPath);
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn start_delete_repo_path_no_repos_is_noop() {
    let mut app = make_app();
    app.board.repo_paths = vec![];
    app.input.mode = InputMode::RepoFilter;
    app.update(Message::StartDeleteRepoPath);
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn confirm_delete_repo_path_emits_command() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 1;
    let cmds = app.update(Message::DeleteRepoPath("/repo-b".to_string()));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteRepoPath(p) if p == "/repo-b")));
}

#[test]
fn cancel_delete_repo_path_returns_to_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

#[test]
fn delete_repo_path_removes_from_active_filter() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.filter.repos.insert("/repo-a".to_string());
    app.filter.repos.insert("/repo-b".to_string());
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.update(Message::DeleteRepoPath("/repo-a".to_string()));
    assert!(!app.filter.repos.contains("/repo-a"));
    assert!(app.filter.repos.contains("/repo-b"));
}

#[test]
fn delete_repo_path_clamps_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
    app.input.repo_cursor = 1;
    // Simulate the path being removed (RepoPathsUpdated would do this in practice)
    app.update(Message::RepoPathsUpdated(vec!["/repo-a".to_string()]));
    assert!(
        app.input.repo_cursor < app.board.repo_paths.len(),
        "cursor should be clamped after repo list shrinks"
    );
}

#[test]
fn backspace_in_repo_filter_starts_delete() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn delete_key_in_repo_filter_starts_delete() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.handle_key(make_key(KeyCode::Delete));
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn y_in_confirm_delete_repo_path_confirms() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 0;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DeleteRepoPath(p) if p == "/repo-a")));
}

#[test]
fn n_in_confirm_delete_repo_path_cancels() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo-a".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

// ---------------------------------------------------------------------------
// Security board input handler tests
// ---------------------------------------------------------------------------

/// Helper: put app into SecurityBoard view with alerts loaded.
fn make_security_board_app() -> App {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    app.update(Message::SecurityAlertsLoaded(vec![
        make_security_alert(1, "org/alpha", crate::models::AlertSeverity::Critical),
        make_security_alert(2, "org/beta", crate::models::AlertSeverity::High),
        make_security_alert(3, "org/gamma", crate::models::AlertSeverity::Critical),
    ]));
    app
}

#[test]
fn security_board_q_quits() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn security_board_tab_switches_to_task_board() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn security_board_esc_switches_to_task_board() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn security_board_h_navigates_column_left() {
    let mut app = make_security_board_app();
    // Move to column 1 first, then h should go back to 0
    if let Some(sel) = app.security_selection_mut() {
        sel.set_column(1);
    }
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.security_selection().unwrap().column(), 0);
}

#[test]
fn security_board_l_navigates_column_right() {
    let mut app = make_security_board_app();
    assert_eq!(app.security_selection().unwrap().column(), 0);
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_eq!(app.security_selection().unwrap().column(), 1);
}

#[test]
fn security_board_left_arrow_navigates_column() {
    let mut app = make_security_board_app();
    if let Some(sel) = app.security_selection_mut() {
        sel.set_column(1);
    }
    app.handle_key(make_key(KeyCode::Left));
    assert_eq!(app.security_selection().unwrap().column(), 0);
}

#[test]
fn security_board_right_arrow_navigates_column() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Right));
    assert_eq!(app.security_selection().unwrap().column(), 1);
}

#[test]
fn security_board_column_clamps_at_zero() {
    let mut app = make_security_board_app();
    assert_eq!(app.security_selection().unwrap().column(), 0);
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.security_selection().unwrap().column(), 0);
}

#[test]
fn security_board_column_clamps_at_max() {
    let mut app = make_security_board_app();
    let max_col = crate::models::AlertSeverity::COLUMN_COUNT - 1;
    if let Some(sel) = app.security_selection_mut() {
        sel.set_column(max_col);
    }
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_eq!(app.security_selection().unwrap().column(), max_col);
}

#[test]
fn security_board_j_navigates_row_down() {
    let mut app = make_security_board_app();
    // Column 0 (Critical) has 2 alerts
    assert_eq!(app.security_selection().unwrap().row(0), 0);
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.security_selection().unwrap().row(0), 1);
}

#[test]
fn security_board_k_navigates_row_up() {
    let mut app = make_security_board_app();
    if let Some(sel) = app.security_selection_mut() {
        sel.set_row(0, 1);
    }
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.security_selection().unwrap().row(0), 0);
}

#[test]
fn security_board_enter_toggles_detail() {
    let mut app = make_security_board_app();
    assert!(!app.security_detail_visible());
    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.security_detail_visible());
    app.handle_key(make_key(KeyCode::Enter));
    assert!(!app.security_detail_visible());
}

#[test]
fn security_board_p_opens_alert_in_browser() {
    let mut app = make_security_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::OpenInBrowser { url } if url.contains("security"))));
}

#[test]
fn security_board_p_with_no_alert_is_noop() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    // No alerts loaded
    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds.is_empty());
}

#[test]
fn security_board_d_dispatches_fix_agent() {
    let mut app = make_security_board_app();
    // Set repo_paths so resolve_repo_path matches "org/alpha"
    app.board.repo_paths = vec!["/path/to/alpha".to_string()];
    // Move to Critical column row 0, which has alert from "org/alpha"
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchFixAgent(..))));
}

#[test]
fn security_board_d_falls_back_to_repo_input_when_no_match() {
    let mut app = make_security_board_app();
    // No matching repo path — should prompt for repo path input
    app.handle_key(make_key(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);
}

#[test]
fn security_board_d_with_no_alert_is_noop() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
}

#[test]
fn security_board_r_without_idle_agent_is_noop() {
    let mut app = make_security_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert!(
        cmds.is_empty(),
        "r without idle agent should do nothing (refresh removed)"
    );
}

#[test]
fn security_board_f_opens_repo_filter() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::SecurityRepoFilter);
}

#[test]
fn security_board_t_toggles_kind_filter() {
    let mut app = make_security_board_app();
    assert!(app.security_kind_filter().is_none());
    app.handle_key(make_key(KeyCode::Char('t')));
    assert!(app.security_kind_filter().is_some());
}

#[test]
fn security_board_question_mark_toggles_help() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Help);
}

#[test]
fn security_board_unknown_key_is_noop() {
    let mut app = make_security_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------------------
// Security repo filter input handler tests
// ---------------------------------------------------------------------------

#[test]
fn security_repo_filter_enter_closes() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn security_repo_filter_esc_closes() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn security_repo_filter_tab_toggles_mode() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Char('f'))); // Enter filter
    assert_eq!(app.input.mode, InputMode::SecurityRepoFilter);
    app.handle_key(make_key(KeyCode::Tab));
    // Mode should have toggled (include -> exclude or vice versa)
    assert_eq!(app.input.mode, InputMode::SecurityRepoFilter);
}

#[test]
fn security_repo_filter_a_toggles_all() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    app.handle_key(make_key(KeyCode::Char('a')));
    // After toggling all, the filter should have entries
    assert_eq!(app.input.mode, InputMode::SecurityRepoFilter);
}

#[test]
fn security_repo_filter_digit_toggles_repo() {
    let mut app = make_security_board_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    // Press '1' to toggle the first repo
    app.handle_key(make_key(KeyCode::Char('1')));
    assert_eq!(app.input.mode, InputMode::SecurityRepoFilter);
}

// ---------------------------------------------------------------------------
// Review board input handler tests
// ---------------------------------------------------------------------------

/// Helper: put app into ReviewBoard view with PRs loaded.
fn make_review_board_app() -> App {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr(1, "alice", ReviewDecision::ReviewRequired),
            make_review_pr(2, "bob", ReviewDecision::Approved),
            make_review_pr(3, "carol", ReviewDecision::ReviewRequired),
        ],
    ));
    app
}

#[test]
fn review_board_q_quits() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn review_board_tab_switches_to_security_board() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::SecurityBoard { .. }
    ));
}

#[test]
fn review_board_esc_switches_to_task_board() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn review_board_h_navigates_column_left() {
    let mut app = make_review_board_app();
    if let Some(sel) = app.review_selection_mut() {
        sel.set_column(1);
    }
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.review_selection().unwrap().column(), 0);
}

#[test]
fn review_board_l_navigates_column_right() {
    let mut app = make_review_board_app();
    assert_eq!(app.review_selection().unwrap().column(), 0);
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_eq!(app.review_selection().unwrap().column(), 1);
}

#[test]
fn review_board_left_right_arrows_navigate_columns() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Right));
    assert_eq!(app.review_selection().unwrap().column(), 1);
    app.handle_key(make_key(KeyCode::Left));
    assert_eq!(app.review_selection().unwrap().column(), 0);
}

#[test]
fn review_board_column_clamps_at_zero() {
    let mut app = make_review_board_app();
    assert_eq!(app.review_selection().unwrap().column(), 0);
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(app.review_selection().unwrap().column(), 0);
}

#[test]
fn review_board_column_clamps_at_max() {
    let mut app = make_review_board_app();
    let max_col = ReviewDecision::COLUMN_COUNT - 1;
    if let Some(sel) = app.review_selection_mut() {
        sel.set_column(max_col);
    }
    app.handle_key(make_key(KeyCode::Char('l')));
    assert_eq!(app.review_selection().unwrap().column(), max_col);
}

#[test]
fn review_board_j_navigates_row_down() {
    let mut app = make_review_board_app();
    // Column 0 (ReviewRequired) has 2 PRs
    assert_eq!(app.review_selection().unwrap().row(0), 0);
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.review_selection().unwrap().row(0), 1);
}

#[test]
fn review_board_k_navigates_row_up() {
    let mut app = make_review_board_app();
    if let Some(sel) = app.review_selection_mut() {
        sel.set_row(0, 1);
    }
    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.review_selection().unwrap().row(0), 0);
}

#[test]
fn review_board_down_up_arrows_navigate_rows() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.review_selection().unwrap().row(0), 1);
    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.review_selection().unwrap().row(0), 0);
}

#[test]
fn review_board_p_opens_pr_in_browser() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::OpenInBrowser { url } if url.contains("pull"))));
}

#[test]
fn review_board_p_with_no_prs_is_noop() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_r_without_idle_agent_is_noop() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert!(
        cmds.is_empty(),
        "r without idle agent should do nothing (refresh removed)"
    );
}

#[test]
fn review_board_f_opens_repo_filter() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);
}

#[test]
fn review_board_d_dispatches_review_agent() {
    let mut app = make_review_board_app();
    // Set repo_paths so resolve_repo_path matches "acme/app"
    app.board.repo_paths = vec!["/path/to/app".to_string()];
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(_))));
}

#[test]
fn review_board_d_falls_back_to_repo_input_when_no_match() {
    let mut app = make_review_board_app();
    // No matching repo path
    app.handle_key(make_key(KeyCode::Char('d')));
    assert_eq!(app.input.mode, InputMode::InputDispatchRepoPath);
}

#[test]
fn review_board_d_with_no_prs_is_noop() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_question_mark_toggles_help() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Help);
}

#[test]
fn review_board_backtab_toggles_mode() {
    let mut app = make_review_board_app();
    assert!(matches!(
        app.board.view_mode,
        ViewMode::ReviewBoard {
            mode: ReviewBoardMode::Reviewer,
            ..
        }
    ));
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::ReviewBoard {
            mode: ReviewBoardMode::Author,
            ..
        }
    ));
}

#[test]
fn review_board_e_edits_github_queries() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('e')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::EditGithubQueries(ReviewBoardMode::Reviewer))));
}

#[test]
fn refresh_review_prs_returns_fetch_command() {
    let mut app = make_review_board_app();
    let cmds = app.update(Message::RefreshReviewPrs);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Review))));
}

#[test]
fn refresh_review_prs_in_author_mode_returns_fetch_my_prs() {
    let mut app = make_review_board_app();
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    let cmds = app.update(Message::RefreshReviewPrs);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Authored))));
}

#[test]
fn refresh_bot_prs_returns_fetch_bot_prs() {
    let mut app = make_review_board_app();
    let cmds = app.update(Message::RefreshBotPrs);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::FetchPrs(PrListKind::Bot))));
}

#[test]
fn bot_prs_merged_kills_active_review_window() {
    let mut app = App::new(vec![], TEST_TIMEOUT);

    let mut pr = make_bot_pr(
        42,
        ReviewDecision::Approved,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        crate::models::CiStatus::Success,
    );
    pr.tmux_window = Some("review:pr-42".to_string());
    pr.worktree = Some("/repo/.worktrees/review-42".to_string());
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr]));

    let cmds = app.update(Message::BotPrsMerged(vec![
        "https://github.com/acme/app/pull/42".to_string(),
    ]));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { window } if window == "review:pr-42")),
        "should kill review board PR window"
    );
    assert!(
        cmds.iter().any(
            |c| matches!(c, Command::UpdateAgentStatus { repo, number, status: None }
                if repo == "acme/app" && *number == 42)
        ),
        "should clear review agent status"
    );
    assert!(app.review.bot.prs[0].tmux_window.is_none());
    assert!(app.review.bot.prs[0].worktree.is_none());
    assert!(app.review.bot.prs[0].agent_status.is_none());
}

#[test]
fn bot_prs_merged_noop_when_no_active_window() {
    let mut app = App::new(vec![], TEST_TIMEOUT);

    // Bot PR with no review agent
    let pr = make_bot_pr(
        42,
        ReviewDecision::Approved,
        None,
        crate::models::CiStatus::Success,
    );
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr]));

    let cmds = app.update(Message::BotPrsMerged(vec![
        "https://github.com/acme/app/pull/42".to_string(),
    ]));

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::KillTmuxWindow { .. })),
        "no window to kill — should emit no KillTmuxWindow"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::UpdateAgentStatus { .. })),
        "no agent to clear — should emit no UpdateAgentStatus"
    );
}

#[test]
fn review_board_unknown_key_is_noop() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_d_capital_toggles_dispatch_filter_in_author_mode() {
    let mut app = make_review_board_app();
    // Switch to Author mode
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::ReviewBoard {
            mode: ReviewBoardMode::Author,
            ..
        }
    ));
    assert!(!app.dispatch_pr_filter());
    app.handle_key(make_key(KeyCode::Char('D')));
    assert!(app.dispatch_pr_filter());
}

#[test]
fn review_board_d_capital_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_esc_clears_bot_pr_selection_first() {
    use crate::models::CiStatus;
    let mut app = make_review_board_app();
    // Switch to Dependabot mode and select a PR
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)); // Author
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)); // Dependabot
    let mut pr = make_review_pr(10, "dependabot", ReviewDecision::ReviewRequired);
    pr.ci_status = CiStatus::Success; // Column 0 (CI Passing)
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr]));
    // Select a bot PR (cursor is at column 0 where the PR is)
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.has_bot_pr_selection());
    // Esc should clear selection, not exit board
    app.handle_key(make_key(KeyCode::Esc));
    assert!(!app.has_bot_pr_selection());
    assert!(matches!(app.board.view_mode, ViewMode::ReviewBoard { .. }));
}

// ---------------------------------------------------------------------------
// Review board dependabot-specific input tests
// ---------------------------------------------------------------------------

/// Helper: app in Dependabot review board mode with bot PRs loaded.
fn make_dependabot_board_app() -> App {
    use crate::models::CiStatus;
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    // Cycle to Dependabot mode: Reviewer -> Author -> Dependabot
    app.update(Message::ToggleReviewBoardMode);
    app.update(Message::ToggleReviewBoardMode);
    // Use CiStatus::Success so PRs land in column 0 (CI Passing)
    let mut pr1 = make_review_pr(10, "dependabot[bot]", ReviewDecision::ReviewRequired);
    pr1.ci_status = CiStatus::Success;
    let mut pr2 = make_review_pr(11, "dependabot[bot]", ReviewDecision::ReviewRequired);
    pr2.ci_status = CiStatus::Success;
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr1, pr2]));
    app
}

#[test]
fn dependabot_space_toggles_select_pr() {
    let mut app = make_dependabot_board_app();
    assert!(!app.has_bot_pr_selection());
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.has_bot_pr_selection());
}

#[test]
fn dependabot_space_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(cmds.is_empty());
}

#[test]
fn dependabot_a_selects_all_column() {
    let mut app = make_dependabot_board_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.has_bot_pr_selection());
}

#[test]
fn dependabot_a_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(cmds.is_empty());
}

#[test]
fn dependabot_capital_a_starts_batch_approve() {
    let mut app = make_dependabot_board_app();
    // Select some PRs first
    app.handle_key(make_key(KeyCode::Char('a'))); // select all
    app.handle_key(make_key(KeyCode::Char('A')));
    assert!(matches!(app.input.mode, InputMode::ConfirmBatchApprove(_)));
}

#[test]
fn dependabot_capital_a_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('A')));
    assert!(cmds.is_empty());
}

#[test]
fn dependabot_m_starts_batch_merge() {
    use crate::models::CiStatus;
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::ToggleReviewBoardMode); // Author
    app.update(Message::ToggleReviewBoardMode); // Dependabot
                                                // Merge requires CI-passing + approved
    let mut pr = make_review_pr(10, "dependabot[bot]", ReviewDecision::Approved);
    pr.ci_status = CiStatus::Success;
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr]));
    // Select the PR — it's in Approved column (2)
    if let Some(sel) = app.review_selection_mut() {
        sel.set_column(2);
    }
    app.handle_key(make_key(KeyCode::Char(' '))); // select
    assert!(app.has_bot_pr_selection());
    app.handle_key(make_key(KeyCode::Char('m')));
    assert!(matches!(app.input.mode, InputMode::ConfirmBatchMerge(_)));
}

#[test]
fn dependabot_m_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------------------
// Review repo filter input handler tests
// ---------------------------------------------------------------------------

#[test]
fn review_repo_filter_enter_closes() {
    let mut app = make_review_board_app();
    app.input.mode = InputMode::ReviewRepoFilter;
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn review_repo_filter_tab_toggles_mode() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Char('f'))); // Enter filter
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);
    app.handle_key(make_key(KeyCode::Tab));
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);
}

#[test]
fn review_repo_filter_a_toggles_all() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);
}

#[test]
fn review_repo_filter_digit_toggles_repo() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    app.handle_key(make_key(KeyCode::Char('1')));
    assert_eq!(app.input.mode, InputMode::ReviewRepoFilter);
}

// ---------------------------------------------------------------------------
// Confirm batch input handler tests
// ---------------------------------------------------------------------------

#[test]
fn confirm_batch_approve_y_confirms() {
    let mut app = make_dependabot_board_app();
    app.handle_key(make_key(KeyCode::Char('a'))); // select all
    app.handle_key(make_key(KeyCode::Char('A'))); // start batch approve
    assert!(matches!(app.input.mode, InputMode::ConfirmBatchApprove(_)));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::BatchApprovePrs(_))));
}

#[test]
fn confirm_batch_approve_n_cancels() {
    let mut app = make_dependabot_board_app();
    app.handle_key(make_key(KeyCode::Char('a'))); // select all
    app.handle_key(make_key(KeyCode::Char('A'))); // start batch approve
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_batch_merge_y_confirms() {
    use crate::models::CiStatus;
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::ToggleReviewBoardMode); // Author
    app.update(Message::ToggleReviewBoardMode); // Dependabot
    let mut pr = make_review_pr(10, "dependabot[bot]", ReviewDecision::Approved);
    pr.ci_status = CiStatus::Success;
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr]));
    if let Some(sel) = app.review_selection_mut() {
        sel.set_column(2); // Approved column
    }
    app.handle_key(make_key(KeyCode::Char(' '))); // select
    app.handle_key(make_key(KeyCode::Char('m'))); // start batch merge
    assert!(matches!(app.input.mode, InputMode::ConfirmBatchMerge(_)));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds.iter().any(|c| matches!(c, Command::BatchMergePrs(_))));
}

#[test]
fn confirm_batch_merge_n_cancels() {
    let mut app = make_dependabot_board_app();
    app.handle_key(make_key(KeyCode::Char('a'))); // select all
    app.handle_key(make_key(KeyCode::Char('m'))); // start batch merge
    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

// ---------------------------------------------------------------------------
// Confirm epic wrap-up input handler tests
// ---------------------------------------------------------------------------

#[test]
fn confirm_epic_wrap_up_r_sends_rebase() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    // Should produce an effect related to epic wrap-up rebase
    assert!(!cmds.is_empty() || app.input.mode == InputMode::Normal);
}

#[test]
fn confirm_epic_wrap_up_p_sends_pr() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('p')));
    assert!(!cmds.is_empty() || app.input.mode == InputMode::Normal);
}

#[test]
fn confirm_epic_wrap_up_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(1));
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn confirm_epic_wrap_up_unknown_key_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmEpicWrapUp(EpicId(1)));
}

// ---------------------------------------------------------------------------
// Confirm edit task and confirm detach tmux via handle_key
// ---------------------------------------------------------------------------

#[test]
fn confirm_edit_task_y_emits_editor_command() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEditTask(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::EditTaskInEditor(_))));
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
    let mut app = App::new(vec![task], TEST_TIMEOUT);
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

// ---------------------------------------------------------------------------
// Archive overlay key handler tests
// ---------------------------------------------------------------------------

#[test]
fn archive_esc_closes_overlay() {
    let mut app = make_app();
    // Archive a task first
    app.update(Message::ArchiveTask(TaskId(1)));
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Esc));
    assert!(!app.archive.visible);
}

#[test]
fn archive_e_enters_edit_confirm() {
    let mut app = make_app();
    app.update(Message::ArchiveTask(TaskId(1)));
    app.archive.visible = true;
    app.archive.selected_row = 0;
    app.handle_key(make_key(KeyCode::Char('e')));
    assert!(matches!(app.input.mode, InputMode::ConfirmEditTask(_)));
}

#[test]
fn archive_q_quits() {
    let mut app = make_app();
    app.update(Message::ArchiveTask(TaskId(1)));
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::ConfirmQuit);
}

#[test]
fn g_on_review_board_jumps_to_agent() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);

    let mut pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    pr.tmux_window = Some("dispatch:review-42".to_string());
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('g')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::JumpToTmux { window } if window == "dispatch:review-42")));
}

#[test]
fn g_on_review_board_without_agent_shows_status() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);

    let pr = make_review_pr(42, "alice", ReviewDecision::ReviewRequired);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('g')));
    assert!(cmds.is_empty()); // StatusInfo is handled inline via self.update(), returns empty
}

#[test]
fn g_on_security_board_jumps_to_agent() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);

    let mut alert = make_security_alert(1, "acme/app", crate::models::AlertSeverity::Critical);
    alert.tmux_window = Some("dispatch:fix-1".to_string());
    app.update(Message::SecurityAlertsLoaded(vec![alert]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('g')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::JumpToTmux { window } if window == "dispatch:fix-1")));
}

#[test]
fn g_on_security_board_without_agent_shows_status() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);

    let alert = make_security_alert(1, "acme/app", crate::models::AlertSeverity::Critical);
    app.update(Message::SecurityAlertsLoaded(vec![alert]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('g')));
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------------------
// ReviewAgentStatus lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn review_status_updated_sets_agent_status_on_pr() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    app.update(Message::ReviewStatusUpdated {
        repo: "acme/app".to_string(),
        number: 42,
        status: crate::models::ReviewAgentStatus::FindingsReady,
    });

    let prs = &app.review_prs();
    let pr = prs.iter().find(|p| p.number == 42).unwrap();
    assert_eq!(
        pr.agent_status,
        Some(crate::models::ReviewAgentStatus::FindingsReady)
    );
}

#[test]
fn review_status_updated_sets_agent_status_on_security_alert() {
    let mut app = make_app();
    app.update(Message::SwitchToSecurityBoard);
    let mut alert = make_security_alert(1, "acme/app", crate::models::AlertSeverity::High);
    alert.tmux_window = Some("dispatch:fix-1".to_string());
    app.update(Message::SecurityAlertsLoaded(vec![alert]));

    app.update(Message::ReviewStatusUpdated {
        repo: "acme/app".to_string(),
        number: 1,
        status: crate::models::ReviewAgentStatus::Idle,
    });

    let alerts = app.filtered_security_alerts();
    let alert = alerts.iter().find(|a| a.number == 1).unwrap();
    assert_eq!(
        alert.agent_status,
        Some(crate::models::ReviewAgentStatus::Idle)
    );
}

#[test]
fn detach_review_agent_clears_fields_and_returns_commands() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let mut pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    pr.tmux_window = Some("dispatch:review-42".to_string());
    pr.worktree = Some("/tmp/wt".to_string());
    pr.agent_status = Some(crate::models::ReviewAgentStatus::FindingsReady);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.update(Message::DetachReviewAgent {
        repo: "acme/app".to_string(),
        number: 42,
    });

    // Should have kill tmux and update agent status commands
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::KillTmuxWindow { .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::UpdateAgentStatus { .. })));

    // In-memory PR should be cleared
    let prs = &app.review_prs();
    let pr = prs.iter().find(|p| p.number == 42).unwrap();
    assert!(pr.tmux_window.is_none());
    assert!(pr.agent_status.is_none());
}

#[test]
fn review_agent_dispatched_sets_agent_status_reviewing() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let pr = make_review_pr_for_repo(
        99,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "org/my-repo",
    );
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    app.update(Message::ReviewAgentDispatched {
        github_repo: "org/my-repo".to_string(),
        number: 99,
        tmux_window: "review-my-repo-99".to_string(),
        worktree: "/tmp/worktree".to_string(),
    });

    let prs = &app.review_prs();
    let pr = prs.iter().find(|p| p.number == 99).unwrap();
    assert_eq!(
        pr.agent_status,
        Some(crate::models::ReviewAgentStatus::Reviewing)
    );
}

// ---------------------------------------------------------------------------
// Review/Security board key binding tests for r and T
// ---------------------------------------------------------------------------

#[test]
fn review_board_r_on_idle_agent_emits_re_review() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let mut pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    pr.tmux_window = Some("dispatch:review-42".to_string());
    pr.agent_status = Some(crate::models::ReviewAgentStatus::Idle);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('r')));
    assert!(cmds.iter().any(|c| matches!(c, Command::ReReview { .. })));
}

#[test]
fn review_board_r_without_agent_does_nothing() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('r')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_r_on_reviewing_agent_does_nothing() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let mut pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    pr.tmux_window = Some("dispatch:review-42".to_string());
    pr.agent_status = Some(crate::models::ReviewAgentStatus::Reviewing);
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('r')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_t_on_agent_emits_detach() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let mut pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    pr.tmux_window = Some("dispatch:review-42".to_string());
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('T')));
    // DetachReviewAgent is a Message, not a Command — so it's handled inline
    // and should produce KillTmuxWindow + UpdateAgentStatus commands
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::KillTmuxWindow { .. })));
}

#[test]
fn review_board_t_without_agent_does_nothing() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    let pr = make_review_pr_for_repo(
        42,
        "alice",
        crate::models::ReviewDecision::ReviewRequired,
        "acme/app",
    );
    app.update(Message::PrsLoaded(PrListKind::Review, vec![pr]));

    let cmds = app.handle_key(KeyEvent::from(KeyCode::Char('T')));
    assert!(cmds.is_empty());
}

/// Extract the foreground color of the first `[` bracket in the given row.
fn first_bracket_fg(buf: &Buffer, row: u16) -> Option<Color> {
    let area = buf.area();
    for x in area.left()..area.right() {
        if buf[(x, row)].symbol() == "[" {
            return Some(buf[(x, row)].fg);
        }
    }
    None
}

#[test]
fn status_bar_key_color_is_consistent_across_columns() {
    let mut app = App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Running),
            make_task(3, TaskStatus::Review),
            make_task(4, TaskStatus::Done),
        ],
        TEST_TIMEOUT,
    );

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

// ---------------------------------------------------------------------------
// In-flight dispatch deduplication
// ---------------------------------------------------------------------------

#[test]
fn dispatch_in_flight_blocks_second_dispatch() {
    let mut app = make_app();
    // First dispatch succeeds
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
    // Second dispatch of same task is blocked
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn brainstorm_in_flight_blocks_second_brainstorm() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // First brainstorm succeeds (feature without plan → brainstorm)
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Brainstorm));
    assert!(matches!(
        cmds[0],
        Command::DispatchAgent {
            mode: DispatchMode::Brainstorm,
            ..
        }
    ));
    // Second brainstorm of same task is blocked
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Brainstorm));
    assert!(cmds.is_empty());
}

#[test]
fn plan_in_flight_blocks_second_plan() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Plan));
    assert!(matches!(
        cmds[0],
        Command::DispatchAgent {
            mode: DispatchMode::Plan,
            ..
        }
    ));
    // Second plan of same task is blocked
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Plan));
    assert!(cmds.is_empty());
}

#[test]
fn dispatched_clears_in_flight() {
    let mut app = make_app();
    // Dispatch task 1
    app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    // Dispatched message clears the in-flight guard
    app.update(Message::Dispatched {
        id: TaskId(1),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: false,
    });
    // Task is now Running, so dispatch is a no-op for a different reason,
    // but the in-flight set should be clear
    assert!(!app.is_dispatching(TaskId(1)));
}

#[test]
fn dispatch_failed_clears_in_flight() {
    let mut app = make_app();
    // Dispatch task 1
    app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(app.is_dispatching(TaskId(1)));
    // DispatchFailed clears the in-flight guard
    app.update(Message::DispatchFailed(TaskId(1)));
    assert!(!app.is_dispatching(TaskId(1)));
    // Can dispatch again
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
}

#[test]
fn dispatch_different_tasks_both_succeed() {
    let mut app = make_app();
    // Dispatch task 1
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
    // Dispatch task 2 — different task, should succeed
    let cmds = app.update(Message::DispatchTask(TaskId(2), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
}

// ---------------------------------------------------------------------------
// Review agent in-flight dispatch deduplication
// ---------------------------------------------------------------------------

fn make_review_agent_req(repo: &str, number: i64) -> ReviewAgentRequest {
    ReviewAgentRequest {
        github_repo: repo.to_string(),
        number,
        head_ref: "main".to_string(),
        repo: "/home/user/Code/repo".to_string(),
        is_dependabot: false,
    }
}

#[test]
fn review_agent_dispatch_in_flight_blocks_second_dispatch() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/home/user/Code/repo".to_string()];
    let req = make_review_agent_req("acme/app", 42);
    // First dispatch succeeds
    let cmds = app.update(Message::DispatchReviewAgent(req.clone()));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(_))));
    // Second dispatch of same PR is blocked
    let cmds = app.update(Message::DispatchReviewAgent(req));
    assert!(cmds.is_empty());
}

#[test]
fn review_agent_dispatched_clears_in_flight() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/home/user/Code/repo".to_string()];
    let req = make_review_agent_req("acme/app", 42);
    app.update(Message::DispatchReviewAgent(req.clone()));
    assert!(app.is_dispatching_review("acme/app", 42));
    // Success message clears the guard
    app.update(Message::ReviewAgentDispatched {
        github_repo: "acme/app".to_string(),
        number: 42,
        tmux_window: "review-42".to_string(),
        worktree: "/wt".to_string(),
    });
    assert!(!app.is_dispatching_review("acme/app", 42));
}

#[test]
fn review_agent_failed_clears_in_flight() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/home/user/Code/repo".to_string()];
    let req = make_review_agent_req("acme/app", 42);
    app.update(Message::DispatchReviewAgent(req.clone()));
    assert!(app.is_dispatching_review("acme/app", 42));
    // Failure clears the guard
    app.update(Message::ReviewAgentFailed {
        github_repo: "acme/app".to_string(),
        number: 42,
        error: "boom".to_string(),
    });
    assert!(!app.is_dispatching_review("acme/app", 42));
    // Can dispatch again
    let cmds = app.update(Message::DispatchReviewAgent(req));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(_))));
}

#[test]
fn review_agent_different_prs_both_dispatch() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/home/user/Code/repo".to_string()];
    let cmds = app.update(Message::DispatchReviewAgent(make_review_agent_req(
        "acme/app", 42,
    )));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(_))));
    let cmds = app.update(Message::DispatchReviewAgent(make_review_agent_req(
        "acme/app", 43,
    )));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(_))));
}

// ---------------------------------------------------------------------------
// Fix agent in-flight dispatch deduplication
// ---------------------------------------------------------------------------

#[test]
fn fix_agent_dispatch_in_flight_blocks_second_dispatch() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/path/to/repo".to_string()];
    let msg = Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "Alert 1".to_string(),
        description: String::new(),
        package: None,
        fixed_version: None,
    });
    // First dispatch succeeds
    let cmds = app.update(msg.clone());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchFixAgent(..))));
    // Second dispatch of same alert is blocked
    let cmds = app.update(msg);
    assert!(cmds.is_empty());
}

#[test]
fn fix_agent_dispatched_clears_in_flight() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/path/to/repo".to_string()];
    app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "Alert 1".to_string(),
        description: String::new(),
        package: None,
        fixed_version: None,
    }));
    assert!(app.is_dispatching_fix("org/repo", 1, crate::models::AlertKind::Dependabot));
    // Success clears the guard
    app.update(Message::FixAgentDispatched {
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        tmux_window: "fix-1".to_string(),
        worktree: "/wt".to_string(),
    });
    assert!(!app.is_dispatching_fix("org/repo", 1, crate::models::AlertKind::Dependabot));
}

#[test]
fn fix_agent_failed_clears_in_flight() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/path/to/repo".to_string()];
    app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "Alert 1".to_string(),
        description: String::new(),
        package: None,
        fixed_version: None,
    }));
    assert!(app.is_dispatching_fix("org/repo", 1, crate::models::AlertKind::Dependabot));
    // Failure clears the guard
    app.update(Message::FixAgentFailed {
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        error: "boom".to_string(),
    });
    assert!(!app.is_dispatching_fix("org/repo", 1, crate::models::AlertKind::Dependabot));
}

#[test]
fn fix_agent_different_alerts_both_dispatch() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/path/to/repo".to_string()];
    // Dependabot alert
    let cmds = app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::Dependabot,
        title: "Alert 1".to_string(),
        description: String::new(),
        package: None,
        fixed_version: None,
    }));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchFixAgent(..))));
    // CodeScanning alert on same repo+number — different kind, should succeed
    let cmds = app.update(Message::DispatchFixAgent(FixAgentRequest {
        repo: String::new(),
        github_repo: "org/repo".to_string(),
        number: 1,
        kind: crate::models::AlertKind::CodeScanning,
        title: "Alert 1".to_string(),
        description: String::new(),
        package: None,
        fixed_version: None,
    }));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchFixAgent(..))));
}

// ---------------------------------------------------------------------------
// Split mode tests
// ---------------------------------------------------------------------------

#[test]
fn toggle_split_mode_emits_enter_command() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EnterSplitMode));
}

#[test]
fn toggle_split_mode_emits_exit_command() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = None;
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::ExitSplitMode { pane_id, restore_window } if pane_id == "%42" && restore_window.is_none())
    );
}

#[test]
fn toggle_split_exit_restores_pinned_task_window() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::ExitSplitMode { pane_id, restore_window } if pane_id == "%42" && restore_window.as_deref() == Some("task-3"))
    );
}

#[test]
fn g_on_already_pinned_task_emits_focus_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4)); // same task selected
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::FocusSplitPane { pane_id } if pane_id == "%42"
    ));
}

#[test]
fn g_in_split_mode_emits_swap_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::SwapSplitPane {
            task_id,
            new_window,
            ..
        } if *task_id == TaskId(4) && new_window == "task-4"
    ));
}

#[test]
fn g_without_split_mode_emits_jump_command() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(matches!(
        &cmds[0],
        Command::JumpToTmux { window } if window == "task-4"
    ));
}

#[test]
fn split_pane_opened_updates_state() {
    let mut app = make_app();
    assert!(!app.board.split.active);
    app.update(Message::SplitPaneOpened {
        pane_id: "%42".to_string(),
        task_id: Some(TaskId(3)),
    });
    assert!(app.board.split.active);
    assert_eq!(app.board.split.right_pane_id.as_deref(), Some("%42"));
    assert_eq!(app.board.split.pinned_task_id, Some(TaskId(3)));
}

#[test]
fn split_pane_closed_resets_state() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));
    app.update(Message::SplitPaneClosed);
    assert!(!app.board.split.active);
    assert!(app.board.split.right_pane_id.is_none());
    assert!(app.board.split.pinned_task_id.is_none());
}

#[test]
fn tick_with_active_split_checks_pane() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    let cmds = app.update(Message::Tick);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::CheckSplitPaneExists { pane_id } if pane_id == "%42"
    )));
}

#[test]
fn tick_without_split_does_not_check_pane() {
    let mut app = make_app();
    let cmds = app.update(Message::Tick);
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CheckSplitPaneExists { .. })));
}

#[test]
fn tick_skips_capture_for_split_pinned_task() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    let cmds = app.update(Message::Tick);

    // Should NOT emit CaptureTmux for the pinned task (its window is a pane now)
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(4), .. })),
        "split-pinned task should be excluded from CaptureTmux"
    );
}

#[test]
fn tick_captures_non_pinned_tasks_in_split_mode() {
    let mut task3 = make_task(3, TaskStatus::Running);
    task3.tmux_window = Some("task-3".to_string());
    let mut task4 = make_task(4, TaskStatus::Running);
    task4.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task3, task4], TEST_TIMEOUT);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    let cmds = app.update(Message::Tick);

    // Task 3 (not pinned) should still get captured
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(3), .. })));
    // Task 4 (pinned in split) should NOT
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(4), .. })));
}

#[test]
fn window_gone_ignored_for_split_pinned_task() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    // Even if WindowGone fires for the pinned task, it should NOT crash
    app.update(Message::WindowGone(TaskId(4)));
    assert!(
        !app.is_crashed(TaskId(4)),
        "split-pinned task should not be marked as crashed"
    );
}

#[test]
fn toggle_split_with_selected_tmux_task_emits_enter_with_task() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Command::EnterSplitModeWithTask { task_id, window }
            if *task_id == TaskId(3) && window == "task-3"
    ));
}

#[test]
fn toggle_split_without_tmux_task_emits_plain_enter() {
    let task = make_task(3, TaskStatus::Running);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column, task has no tmux_window
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EnterSplitMode));
}

#[test]
fn toggle_split_no_selection_emits_plain_enter() {
    // make_app has tasks but default selection is on Backlog column — task 1 has no tmux_window
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::EnterSplitMode));
}

// =====================================================================
// Input handler coverage: normal mode keys
// =====================================================================

#[test]
fn handle_key_normal_copy_task() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
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
fn handle_key_normal_reorder_j_down() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('J')));
    // Reorder should produce a persist command
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn handle_key_normal_reorder_k_up() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 1);
    let cmds = app.handle_key(make_key(KeyCode::Char('K')));
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn handle_key_normal_start_repo_filter() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('f')));
    assert_eq!(*app.mode(), InputMode::RepoFilter);
}

#[test]
fn handle_key_normal_wrap_up_task() {
    let mut task = make_task(10, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/10-test".to_string());
    task.tmux_window = Some("main:10-test".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // Select the review column
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char('W')));
    assert!(matches!(*app.mode(), InputMode::ConfirmWrapUp(TaskId(10))));
}

#[test]
fn handle_key_normal_wrap_up_epic() {
    let mut subtask = make_task(20, TaskStatus::Review);
    subtask.epic_id = Some(EpicId(10));
    subtask.worktree = Some("/repo/.worktrees/20-test".to_string());
    let mut app = App::new(vec![subtask], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];
    // Epic is in Review column
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char('W')));
    assert!(matches!(
        *app.mode(),
        InputMode::ConfirmEpicWrapUp(EpicId(10))
    ));
}

#[test]
fn handle_key_normal_wrap_up_on_empty_is_noop() {
    let mut app = make_app();
    // Navigate to an empty column (Review has no tasks by default)
    app.selection_mut().set_column(2);
    let cmds = app.handle_key(make_key(KeyCode::Char('W')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_normal_move_forward_via_handle_key() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    // Task 1 should move from Backlog to Running
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(1) && t.status == TaskStatus::Running)));
}

#[test]
fn handle_key_normal_move_backward_via_handle_key() {
    let mut app = make_app();
    // Select running task (column 1)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('M')));
    // Task 3 should move from Running to Backlog
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3) && t.status == TaskStatus::Backlog)));
}

#[test]
fn handle_key_normal_start_merge_pr() {
    let mut task = make_task(10, TaskStatus::Review);
    task.pr_url = Some("https://github.com/example/repo/pull/42".to_string());
    task.sub_status = SubStatus::Approved;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char('P')));
    assert!(matches!(*app.mode(), InputMode::ConfirmMergePr(TaskId(10))));
}

#[test]
fn handle_key_normal_toggle_split_mode() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('S')));
    assert!(cmds.iter().any(|c| matches!(c, Command::EnterSplitMode)));
}

#[test]
fn handle_key_normal_detach_tmux_review_task() {
    let mut task = make_task(10, TaskStatus::Review);
    task.tmux_window = Some("main:10-test".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    app.handle_key(make_key(KeyCode::Char('T')));
    assert!(matches!(*app.mode(), InputMode::ConfirmDetachTmux(_)));
}

#[test]
fn handle_key_normal_detach_tmux_no_window_is_noop() {
    let mut app = make_app();
    // Task 1 has no tmux window
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('T')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_normal_esc_in_epic_view_exits() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { .. }));

    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_normal_q_in_epic_view_exits() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_normal_enter_on_select_all_row() {
    let mut app = make_app();
    // Navigate up past first item to land on "select all" virtual row
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
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

#[test]
fn handle_key_normal_unknown_key_is_noop() {
    let mut app = make_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: archive overlay keys
// =====================================================================

#[test]
fn handle_key_archive_j_navigates_down() {
    let mut app = make_app();
    // Add archived tasks
    let mut t1 = make_task(100, TaskStatus::Archived);
    t1.title = "Archived 1".to_string();
    let mut t2 = make_task(101, TaskStatus::Archived);
    t2.title = "Archived 2".to_string();
    app.board.tasks.push(t1);
    app.board.tasks.push(t2);
    app.archive.visible = true;
    app.archive.selected_row = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.archive.selected_row, 1);
}

#[test]
fn handle_key_archive_k_navigates_up() {
    let mut app = make_app();
    let mut t1 = make_task(100, TaskStatus::Archived);
    t1.title = "Archived 1".to_string();
    let mut t2 = make_task(101, TaskStatus::Archived);
    t2.title = "Archived 2".to_string();
    app.board.tasks.push(t1);
    app.board.tasks.push(t2);
    app.archive.visible = true;
    app.archive.selected_row = 1;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.archive.selected_row, 0);
}

#[test]
fn handle_key_archive_k_clamps_at_zero() {
    let mut app = make_app();
    let t = make_task(100, TaskStatus::Archived);
    app.board.tasks.push(t);
    app.archive.visible = true;
    app.archive.selected_row = 0;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.archive.selected_row, 0);
}

#[test]
fn handle_key_archive_down_arrow_navigates() {
    let mut app = make_app();
    let t1 = make_task(100, TaskStatus::Archived);
    let t2 = make_task(101, TaskStatus::Archived);
    app.board.tasks.push(t1);
    app.board.tasks.push(t2);
    app.archive.visible = true;
    app.archive.selected_row = 0;

    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.archive.selected_row, 1);
}

#[test]
fn handle_key_archive_up_arrow_navigates() {
    let mut app = make_app();
    let t1 = make_task(100, TaskStatus::Archived);
    let t2 = make_task(101, TaskStatus::Archived);
    app.board.tasks.push(t1);
    app.board.tasks.push(t2);
    app.archive.visible = true;
    app.archive.selected_row = 1;

    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.archive.selected_row, 0);
}

#[test]
fn handle_key_archive_x_enters_confirm_delete() {
    let mut app = make_app();
    let t = make_task(100, TaskStatus::Archived);
    app.board.tasks.push(t);
    app.archive.visible = true;
    app.archive.selected_row = 0;

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(*app.mode(), InputMode::ConfirmDelete);
}

#[test]
fn handle_key_archive_e_enters_confirm_edit() {
    let mut app = make_app();
    let t = make_task(100, TaskStatus::Archived);
    app.board.tasks.push(t);
    app.archive.visible = true;
    app.archive.selected_row = 0;

    app.handle_key(make_key(KeyCode::Char('e')));
    assert!(matches!(
        *app.mode(),
        InputMode::ConfirmEditTask(TaskId(100))
    ));
}

#[test]
fn handle_key_archive_h_closes() {
    let mut app = make_app();
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Char('H')));
    assert!(!app.archive.visible);
}

#[test]
fn handle_key_archive_esc_closes() {
    let mut app = make_app();
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Esc));
    assert!(!app.archive.visible);
}

#[test]
fn handle_key_archive_q_quits() {
    let mut app = make_app();
    app.archive.visible = true;
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(*app.mode(), InputMode::ConfirmQuit);
}

#[test]
fn handle_key_archive_unknown_key_is_noop() {
    let mut app = make_app();
    app.archive.visible = true;
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: text input repo path navigation
// =====================================================================

#[test]
fn handle_key_text_input_repo_j_navigates_when_buffer_empty() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn handle_key_text_input_repo_k_navigates_when_buffer_empty() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 1;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn handle_key_text_input_repo_j_navigates_when_buffer_non_empty() {
    // j/k now always navigate the filtered list in repo modes, even with non-empty buffer
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.buffer = "x".to_string();
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    // buffer unchanged — j navigates, does not type
    assert_eq!(app.input.buffer, "x");
    // filtered list for "x" against ["/repo", "/other"] is empty, so cursor stays 0
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
fn handle_key_text_input_dispatch_repo_path_enter_selects_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/tmp".to_string()];
    app.input.mode = InputMode::InputDispatchRepoPath;
    app.input.buffer.clear();
    app.input.repo_cursor = 0;
    // Set up a pending dispatch
    app.input.pending_dispatch = Some(PendingDispatch::Review(ReviewAgentRequest {
        repo: "acme/app".to_string(),
        github_repo: "acme/app".to_string(),
        number: 1,
        head_ref: String::new(),
        is_dependabot: false,
    }));

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchReviewAgent(_))));
}

#[test]
fn digit_key_selects_in_dispatch_repo_path_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string()];
    app.input.mode = InputMode::InputDispatchRepoPath;
    app.input.buffer.clear();
    app.input.pending_dispatch = Some(PendingDispatch::Review(ReviewAgentRequest {
        repo: "acme/app".to_string(),
        github_repo: "acme/app".to_string(),
        number: 1,
        head_ref: String::new(),
        is_dependabot: false,
    }));
    // With empty buffer, filtered = ["/tmp", "/var"]; pressing '1' should select /tmp, not append
    let cmds = app.handle_key(make_key(KeyCode::Char('1')));
    assert!(
        !app.input.buffer.contains('1'),
        "digit should not append to buffer when a filtered match exists"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::DispatchReviewAgent(_))),
        "digit '1' should dispatch the first matching repo path"
    );
}

#[test]
fn handle_key_epic_repo_path_enter_selects_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/tmp".to_string()];
    app.input.mode = InputMode::InputEpicRepoPath;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic".to_string(),
        description: "desc".to_string(),
        repo_path: String::new(),
    });
    app.input.buffer.clear();
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert!(cmds.iter().any(|c| matches!(c, Command::InsertEpic(_))));
}

// =====================================================================
// Input handler coverage: quick dispatch navigation
// =====================================================================

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

// =====================================================================
// Input handler coverage: repo filter operations
// =====================================================================

#[test]
fn handle_key_repo_filter_tab_toggles_mode() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;
    let initial_mode = app.filter.mode;

    app.handle_key(make_key(KeyCode::Tab));
    assert_ne!(app.filter.mode, initial_mode);
}

#[test]
fn handle_key_repo_filter_a_toggles_all() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('a')));
    // Should toggle all repos in filter
    assert!(!app.filter.repos.is_empty());
}

#[test]
fn handle_key_repo_filter_space_toggles_cursor_item() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.filter.repos.contains("/repo"));
}

#[test]
fn handle_key_repo_filter_j_moves_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn handle_key_repo_filter_k_moves_cursor() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 1;

    app.handle_key(make_key(KeyCode::Char('k')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn handle_key_repo_filter_backspace_starts_delete_repo_path() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;
    app.input.repo_cursor = 0;

    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(*app.mode(), InputMode::ConfirmDeleteRepoPath);
}

#[test]
fn handle_key_repo_filter_s_starts_save_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('s')));
    assert_eq!(*app.mode(), InputMode::InputPresetName);
}

#[test]
fn handle_key_repo_filter_x_starts_delete_preset() {
    let mut app = make_app();
    app.filter.presets = vec![(
        "preset-a".to_string(),
        std::collections::HashSet::new(),
        RepoFilterMode::Include,
    )];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(*app.mode(), InputMode::ConfirmDeletePreset);
}

#[test]
fn handle_key_repo_filter_uppercase_loads_preset() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.filter.presets = vec![(
        "preset-a".to_string(),
        {
            let mut s = std::collections::HashSet::new();
            s.insert("/repo".to_string());
            s
        },
        RepoFilterMode::Include,
    )];
    app.input.mode = InputMode::RepoFilter;

    app.handle_key(make_key(KeyCode::Char('A')));
    assert!(app.filter.repos.contains("/repo"));
}

#[test]
fn handle_key_repo_filter_uppercase_out_of_range_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    // No presets
    let cmds = app.handle_key(make_key(KeyCode::Char('A')));
    assert!(cmds.is_empty());
}

#[test]
fn handle_key_repo_filter_unknown_key_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: confirm delete repo path
// =====================================================================

#[test]
fn handle_key_confirm_delete_repo_path_y_deletes() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string(), "/other".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds.iter().any(|c| matches!(c, Command::DeleteRepoPath(_))));
}

#[test]
fn handle_key_confirm_delete_repo_path_other_cancels() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;

    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::RepoFilter);
}

#[test]
fn handle_key_confirm_delete_repo_path_uppercase_y() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    app.input.repo_cursor = 0;

    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert!(cmds.iter().any(|c| matches!(c, Command::DeleteRepoPath(_))));
}

// =====================================================================
// Input handler coverage: confirm merge PR
// =====================================================================

#[test]
fn handle_key_confirm_merge_pr_y_merges() {
    let mut task = make_task(10, TaskStatus::Review);
    task.pr_url = Some("https://github.com/test/repo/pull/1".to_string());
    task.sub_status = SubStatus::Approved;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.input.mode = InputMode::ConfirmMergePr(TaskId(10));

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::MergePr { id, .. } if *id == TaskId(10))));
}

#[test]
fn handle_key_confirm_merge_pr_other_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmMergePr(TaskId(1));

    app.handle_key(make_key(KeyCode::Char('n')));
    assert_eq!(*app.mode(), InputMode::Normal);
}

// =====================================================================
// Input handler coverage: review repo filter
// =====================================================================

#[test]
fn handle_key_review_repo_filter_enter_closes() {
    let mut app = make_review_board_app();
    app.input.mode = InputMode::ReviewRepoFilter;

    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_review_repo_filter_esc_closes() {
    let mut app = make_review_board_app();
    app.input.mode = InputMode::ReviewRepoFilter;

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_review_repo_filter_tab_toggles_mode() {
    let mut app = make_review_board_app();
    app.input.mode = InputMode::ReviewRepoFilter;
    let initial_mode = app.review.review.repo_filter_mode;

    app.handle_key(make_key(KeyCode::Tab));
    assert_ne!(app.review.review.repo_filter_mode, initial_mode);
}

#[test]
fn handle_key_review_repo_filter_a_toggles_all() {
    let mut app = make_review_board_app();
    app.input.mode = InputMode::ReviewRepoFilter;

    app.handle_key(make_key(KeyCode::Char('a')));
    // All repos should be toggled
    assert!(!app.review.review.repo_filter.is_empty());
}

#[test]
fn handle_key_review_repo_filter_unknown_key_is_noop() {
    let mut app = make_review_board_app();
    app.input.mode = InputMode::ReviewRepoFilter;

    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: security repo filter
// =====================================================================

#[test]
fn handle_key_security_repo_filter_enter_closes() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;

    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_security_repo_filter_esc_closes() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;

    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn handle_key_security_repo_filter_tab_toggles_mode() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;
    let initial_mode = app.security.repo_filter_mode;

    app.handle_key(make_key(KeyCode::Tab));
    assert_ne!(app.security.repo_filter_mode, initial_mode);
}

#[test]
fn handle_key_security_repo_filter_a_toggles_all() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;

    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(!app.security.repo_filter.is_empty());
}

#[test]
fn handle_key_security_repo_filter_digit_toggles_repo() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;

    // There are repos loaded from security alerts
    if !app.active_security_repos().is_empty() {
        app.handle_key(make_key(KeyCode::Char('1')));
        assert!(!app.security.repo_filter.is_empty());
    }
}

#[test]
fn handle_key_security_repo_filter_unknown_key_is_noop() {
    let mut app = make_security_board_app();
    app.input.mode = InputMode::SecurityRepoFilter;

    let cmds = app.handle_key(make_key(KeyCode::Char('z')));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: security board gap tests (g, T, r)
// =====================================================================

#[test]
fn security_board_g_jumps_to_tmux_window() {
    let mut app = make_security_board_app();
    // Give first alert a tmux window
    if let Some(alert) = app.security.alerts.first_mut() {
        alert.tmux_window = Some("sec:alert-1".to_string());
    }

    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::JumpToTmux { window } if window == "sec:alert-1")));
}

#[test]
fn security_board_g_no_window_shows_status() {
    let mut app = make_security_board_app();
    // Alerts have no tmux window by default
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or("")
        .contains("No active session"));
}

#[test]
fn security_board_capital_t_detaches_agent() {
    let mut app = make_security_board_app();
    if let Some(alert) = app.security.alerts.first_mut() {
        alert.tmux_window = Some("sec:alert-1".to_string());
    }

    let cmds = app.handle_key(make_key(KeyCode::Char('T')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::UpdateAgentStatus { .. })));
}

#[test]
fn security_board_capital_t_no_window_is_noop() {
    let mut app = make_security_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('T')));
    assert!(cmds.is_empty());
}

#[test]
fn security_board_r_with_idle_agent_emits_re_review() {
    let mut app = make_security_board_app();
    if let Some(alert) = app.security.alerts.first_mut() {
        alert.agent_status = Some(crate::models::ReviewAgentStatus::Idle);
        alert.tmux_window = Some("sec:alert-1".to_string());
    }

    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert!(cmds.iter().any(|c| matches!(c, Command::ReReview { .. })));
}

#[test]
fn security_board_r_without_idle_agent_no_window_is_noop() {
    let mut app = make_security_board_app();
    // No agent status set, so not idle
    let cmds = app.handle_key(make_key(KeyCode::Char('r')));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: review board gap tests (dependabot-specific)
// =====================================================================

fn make_dependabot_app() -> App {
    let mut app = make_review_board_app();
    // Toggle to Author, then Dependabot
    app.update(Message::ToggleReviewBoardMode); // Reviewer -> Author
    app.update(Message::ToggleReviewBoardMode); // Author -> Dependabot
                                                // Load bot PRs with CI Success so they land in column 0
    let mut pr1 = make_review_pr(10, "dependabot[bot]", ReviewDecision::ReviewRequired);
    pr1.ci_status = crate::models::CiStatus::Success;
    let mut pr2 = make_review_pr(11, "dependabot[bot]", ReviewDecision::ReviewRequired);
    pr2.ci_status = crate::models::CiStatus::Success;
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr1, pr2]));
    app
}

#[test]
fn review_board_space_selects_bot_pr_in_dependabot_mode() {
    let mut app = make_dependabot_app();
    if let Some(pr) = app.selected_review_pr() {
        let _url = pr.url.clone();
    }
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(app.has_bot_pr_selection());
}

#[test]
fn review_board_space_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_a_selects_all_bot_prs_in_dependabot() {
    let mut app = make_dependabot_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.has_bot_pr_selection());
}

#[test]
fn review_board_a_is_noop_in_reviewer_mode() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_capital_a_starts_batch_approve_in_dependabot() {
    let mut app = make_dependabot_app();
    // Select a PR first
    if let Some(pr) = app.selected_review_pr() {
        let url = pr.url.clone();
        app.update(Message::ToggleSelectBotPr(url));
    }
    app.handle_key(make_key(KeyCode::Char('A')));
    assert!(matches!(*app.mode(), InputMode::ConfirmBatchApprove(_)));
}

#[test]
fn review_board_m_starts_batch_merge_in_dependabot() {
    let mut app = make_review_board_app();
    app.update(Message::ToggleReviewBoardMode); // Reviewer -> Author
    app.update(Message::ToggleReviewBoardMode); // Author -> Dependabot
                                                // Need Approved + CI Success PRs for merge eligibility
    let mut pr = make_review_pr(10, "dependabot[bot]", ReviewDecision::Approved);
    pr.ci_status = crate::models::CiStatus::Success;
    app.update(Message::PrsLoaded(PrListKind::Bot, vec![pr]));
    // Select the PR (navigate to Approved column = 2)
    if let Some(sel) = app.review_selection_mut() {
        sel.selected_column = 2;
    }
    if let Some(pr) = app.selected_review_pr() {
        let url = pr.url.clone();
        app.update(Message::ToggleSelectBotPr(url));
    }
    app.handle_key(make_key(KeyCode::Char('m')));
    assert!(matches!(*app.mode(), InputMode::ConfirmBatchMerge(_)));
}

#[test]
fn review_board_g_jumps_to_tmux_window() {
    let mut app = make_review_board_app();
    if let Some(pr) = app.review.review.prs.first_mut() {
        pr.tmux_window = Some("review:pr-1".to_string());
    }
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::JumpToTmux { window } if window == "review:pr-1")));
}

#[test]
fn review_board_g_no_window_shows_status() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('g')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or("")
        .contains("No active session"));
}

#[test]
fn review_board_capital_t_detaches_agent() {
    let mut app = make_review_board_app();
    if let Some(pr) = app.review.review.prs.first_mut() {
        pr.tmux_window = Some("review:pr-1".to_string());
    }
    let cmds = app.handle_key(make_key(KeyCode::Char('T')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::UpdateAgentStatus { .. })));
}

#[test]
fn review_board_capital_t_no_window_is_noop() {
    let mut app = make_review_board_app();
    let cmds = app.handle_key(make_key(KeyCode::Char('T')));
    assert!(cmds.is_empty());
}

#[test]
fn review_board_enter_toggles_detail_via_handle_key() {
    let mut app = make_review_board_app();
    app.handle_key(make_key(KeyCode::Enter));
    assert!(app.review.detail_visible);
    app.handle_key(make_key(KeyCode::Enter));
    assert!(!app.review.detail_visible);
}

// =====================================================================
// Input handler coverage: tag input mode completeness
// =====================================================================

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
fn handle_key_tag_selects_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    app.handle_key(make_key(KeyCode::Char('e')));
    assert_eq!(
        app.input.task_draft.as_ref().unwrap().tag,
        Some(TaskTag::Epic)
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

// =====================================================================
// Input handler coverage: input preset name completeness
// =====================================================================

#[test]
fn handle_key_input_preset_name_char() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer.clear();

    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.input.buffer, "a");
}

#[test]
fn handle_key_input_preset_name_backspace() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    app.input.buffer = "ab".to_string();

    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "a");
}

#[test]
fn handle_key_input_preset_name_unknown_key_is_noop() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
}

// =====================================================================
// Input handler coverage: confirm detach tmux dialog
// =====================================================================

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

// =====================================================================
// Input handler coverage: epic-specific dispatch keys
// =====================================================================

#[test]
fn handle_key_normal_dispatch_in_epic_view_with_no_items() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::EnterEpic(EpicId(10)));
    // No subtasks, cursor on empty column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    // Should dispatch the epic itself
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchEpic { .. })));
}

#[test]
fn handle_key_normal_m_on_epic_moves_status() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('m')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistEpic { .. })));
}

#[test]
fn handle_key_normal_uppercase_m_on_epic_moves_backward() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('M')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistEpic { .. })));
}

// ── Backlog / Needs-Review color harmonization ──────────────────────

use crate::models::ReviewDecision;

#[test]
fn backlog_column_color_is_blue() {
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

#[test]
fn focused_backlog_header_renders_in_blue() {
    let mut app = make_app();
    assert_eq!(app.selected_column(), 0);

    let buf = render_to_buffer(&mut app, 100, 20);
    let area = buf.area();
    let blue = Color::Rgb(122, 162, 247);
    let target = "backlog";
    let mut found = false;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right().saturating_sub(target.len() as u16 - 1) {
            let matches = target
                .bytes()
                .enumerate()
                .all(|(i, ch)| buf[(x + i as u16, y)].symbol().as_bytes().first() == Some(&ch));
            if matches {
                let fg = buf[(x, y)].fg;
                if fg == blue {
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
        "Focused Backlog header should render with blue foreground color"
    );
}

#[test]
fn needs_review_column_color_matches_backlog() {
    let backlog = ui::column_color(TaskStatus::Backlog);
    let needs_review = ui::review_column_color(ReviewDecision::ReviewRequired);
    assert_eq!(
        backlog, needs_review,
        "Needs Review column color should match Backlog column color"
    );
}

#[test]
fn needs_review_cursor_bg_matches_backlog() {
    let backlog = ui::cursor_bg_color(TaskStatus::Backlog);
    let needs_review = ui::review_cursor_bg_color(ReviewDecision::ReviewRequired);
    assert_eq!(
        backlog, needs_review,
        "Needs Review cursor bg should match Backlog cursor bg"
    );
}

#[test]
fn needs_review_column_bg_matches_backlog() {
    let backlog = ui::column_bg_color(TaskStatus::Backlog);
    let needs_review = ui::review_column_bg_color(ReviewDecision::ReviewRequired);
    assert_eq!(
        backlog, needs_review,
        "Needs Review column bg should match Backlog column bg"
    );
}

// ---------------------------------------------------------------------------
// Input routing tests — verify handle_key() dispatches to correct handler
// ---------------------------------------------------------------------------

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

/// In Normal mode on the ReviewBoard view, keys route to the review board handler.
#[test]
fn handle_key_normal_review_board_routes_correctly() {
    let mut app = make_app();
    app.board.view_mode = ViewMode::ReviewBoard {
        mode: ReviewBoardMode::Reviewer,
        selection: ReviewBoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    // Tab should switch to security board (may emit refresh commands)
    app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::SecurityBoard { .. }
    ));
}

/// In Normal mode on the SecurityBoard view, keys route to the security board handler.
#[test]
fn handle_key_normal_security_board_routes_correctly() {
    let mut app = make_app();
    app.board.view_mode = ViewMode::SecurityBoard {
        selection: SecurityBoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    // Tab should switch away from security board
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
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

/// InputDispatchRepoPath mode routes to the text input handler.
#[test]
fn handle_key_input_dispatch_repo_path_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputDispatchRepoPath;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputEpicTitle mode routes to the text input handler.
#[test]
fn handle_key_input_epic_title_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputEpicDescription mode routes to the text input handler.
#[test]
fn handle_key_input_epic_description_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputEpicRepoPath mode routes to the text input handler.
#[test]
fn handle_key_input_epic_repo_path_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicRepoPath;
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

/// ConfirmArchive mode routes to the confirm-archive handler.
#[test]
fn handle_key_confirm_archive_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmArchive;
    // 'n' cancels
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDeleteEpic mode routes correctly.
#[test]
fn handle_key_confirm_delete_epic_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmArchiveEpic mode routes correctly.
#[test]
fn handle_key_confirm_archive_epic_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmArchiveEpic;
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDone mode routes correctly.
#[test]
fn handle_key_confirm_done_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDone(TaskId(1));
    // 'n' cancels
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmMergePr mode routes correctly.
#[test]
fn handle_key_confirm_merge_pr_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmMergePr(TaskId(1));
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmWrapUp mode routes correctly.
#[test]
fn handle_key_confirm_wrap_up_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmWrapUp(TaskId(1));
    // Esc cancels wrap-up
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmEpicWrapUp mode routes correctly.
#[test]
fn handle_key_confirm_epic_wrap_up_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(1));
    // Esc cancels epic wrap-up
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

/// RepoFilter mode routes correctly.
#[test]
fn handle_key_repo_filter_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::RepoFilter;
    // Esc closes the filter (may emit refresh commands)
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ReviewRepoFilter mode routes correctly.
#[test]
fn handle_key_review_repo_filter_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ReviewRepoFilter;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// SecurityRepoFilter mode routes correctly.
#[test]
fn handle_key_security_repo_filter_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::SecurityRepoFilter;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputPresetName mode routes to the preset name handler.
#[test]
fn handle_key_input_preset_name_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::InputPresetName;
    // Esc cancels preset input, returns to RepoFilter
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

/// ConfirmDeletePreset mode routes correctly.
#[test]
fn handle_key_confirm_delete_preset_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeletePreset;
    // Esc cancels, returns to RepoFilter
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

/// ConfirmDeleteRepoPath mode routes correctly.
#[test]
fn handle_key_confirm_delete_repo_path_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteRepoPath;
    // Any non-y key returns to RepoFilter
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::RepoFilter);
}

/// ConfirmBatchApprove mode routes correctly.
#[test]
fn handle_key_confirm_batch_approve_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmBatchApprove(vec!["url".to_string()]);
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmBatchMerge mode routes correctly.
#[test]
fn handle_key_confirm_batch_merge_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmBatchMerge(vec!["url".to_string()]);
    let cmds = app.handle_key(make_key(KeyCode::Char('n')));
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
fn agent_crashed_stores_last_error_from_tmux_output() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents.tmux_outputs.insert(
        TaskId(4),
        "Error: connection refused\npanicked at main.rs:42".to_string(),
    );

    app.update(Message::AgentCrashed(TaskId(4)));

    assert_eq!(
        app.agents.last_error.get(&TaskId(4)).map(|s| s.as_str()),
        Some("Error: connection refused\npanicked at main.rs:42"),
    );
}

#[test]
fn resumed_clears_last_error() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.agents
        .last_error
        .insert(TaskId(4), "some crash".to_string());

    app.update(Message::Resumed {
        id: TaskId(4),
        tmux_window: "win-4".to_string(),
    });

    assert!(!app.agents.last_error.contains_key(&TaskId(4)));
}

#[test]
fn retry_fresh_clears_last_error() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.agents
        .last_error
        .insert(TaskId(4), "some crash".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    app.update(Message::RetryFresh(TaskId(4)));

    assert!(!app.agents.last_error.contains_key(&TaskId(4)));
}

#[test]
fn task_detail_lines_include_last_error() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_error
        .insert(TaskId(4), "Error: something went wrong".to_string());
    app.board.detail_visible = true;

    let task = app.board.tasks[0].clone();
    let lines = super::ui::task_detail_lines(&app, &task);

    let error_text: String = lines
        .iter()
        .flat_map(|line| line.spans.iter().map(|s| s.content.to_string()))
        .collect::<Vec<_>>()
        .join("");
    assert!(
        error_text.contains("Error: something went wrong"),
        "Expected last_error in detail lines, got: {error_text}"
    );
}

/// Normal mode on Epic view routes to the board handler (not review/security).
#[test]
fn handle_key_normal_epic_view_routes_correctly() {
    let mut app = make_app();
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(1),
        selection: BoardSelection::new(),
        saved_board: BoardSelection::new(),
    };
    // 'q' in epic view exits to board (doesn't quit)
    let cmds = app.handle_key(make_key(KeyCode::Char('q')));
    assert!(cmds.is_empty());
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

// ---------------------------------------------------------------------------
// Terminal resize — re-render on size change
// ---------------------------------------------------------------------------

#[test]
fn terminal_resized_returns_no_commands() {
    let mut app = make_app();
    let cmds = app.update(Message::TerminalResized);
    assert!(
        cmds.is_empty(),
        "resize should produce no commands, just trigger a re-draw"
    );
}

#[test]
fn render_adapts_to_smaller_terminal_after_resize() {
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

#[test]
fn confirm_quit_with_active_split_emits_exit_split_mode() {
    let mut task = make_task(3, TaskStatus::Running);
    task.tmux_window = Some("task-3".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Set up active split with a pinned task
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(3));

    // Enter confirm quit, then confirm with 'y'
    app.input.mode = InputMode::ConfirmQuit;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));

    assert!(app.should_quit);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::ExitSplitMode {
                pane_id,
                restore_window: Some(w),
            } if pane_id == "%42" && w == "task-3"
        )),
        "should emit ExitSplitMode to restore task window before quitting"
    );
}

// ---------------------------------------------------------------------------
// Split pane respawn on task lifecycle events
// ---------------------------------------------------------------------------

#[test]
fn finish_complete_respawns_split_pane_for_pinned_task() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should emit RespawnSplitPane for the pinned pane"
    );
    assert_eq!(
        app.board.split.pinned_task_id, None,
        "pinned_task_id should be cleared"
    );
    assert!(app.board.split.active, "split mode should remain active");
    assert_eq!(
        app.board.split.right_pane_id.as_deref(),
        Some("%5"),
        "pane_id should be preserved"
    );
}

#[test]
fn finish_complete_no_respawn_for_non_pinned_task() {
    let mut app = App::new(
        vec![
            {
                let mut t = make_task(1, TaskStatus::Review);
                t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
                t.tmux_window = Some("task-1".to_string());
                t
            },
            {
                let mut t = make_task(2, TaskStatus::Running);
                t.tmux_window = Some("task-2".to_string());
                t
            },
        ],
        TEST_TIMEOUT,
    );
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(2));

    let cmds = app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { .. })),
        "should NOT respawn when a different task finishes"
    );
    assert_eq!(
        app.board.split.pinned_task_id,
        Some(TaskId(2)),
        "pinned task should be unchanged"
    );
}

#[test]
fn finish_complete_no_respawn_without_split() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t.tmux_window = Some("task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );
    // split is NOT active (default)

    let cmds = app.update(Message::FinishComplete(TaskId(1)));

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { .. })),
        "should NOT respawn when split mode is inactive"
    );
}

#[test]
fn pr_merged_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task's PR is merged"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn confirm_done_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));
    app.input.mode = InputMode::ConfirmDone(TaskId(1));

    let cmds = app.update(Message::ConfirmDone);

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task is confirmed done"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn archive_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Done);
    task.tmux_window = Some("task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::ArchiveTask(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task is archived"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn retry_resume_respawns_split_pane() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.sub_status = SubStatus::Crashed;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));
    app.input.mode = InputMode::ConfirmRetry(TaskId(1));

    let cmds = app.update(Message::RetryResume(TaskId(1)));

    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { pane_id } if pane_id == "%5")),
        "should respawn split pane when pinned task is retried"
    );
    assert_eq!(app.board.split.pinned_task_id, None);
    assert!(app.board.split.active);
}

#[test]
fn pr_created_does_not_respawn_split_pane() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(1));

    let cmds = app.update(Message::PrCreated {
        id: TaskId(1),
        pr_url: "https://github.com/org/repo/pull/42".to_string(),
    });

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::RespawnSplitPane { .. })),
        "should NOT respawn — agent keeps running after PR creation"
    );
    assert_eq!(
        app.board.split.pinned_task_id,
        Some(TaskId(1)),
        "pinned task should remain"
    );
}

#[test]
fn confirm_quit_with_split_no_pinned_task_kills_pane() {
    let mut app = make_app();

    // Split active but no pinned task (empty split)
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%99".to_string());
    app.board.split.pinned_task_id = None;

    app.input.mode = InputMode::ConfirmQuit;
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));

    assert!(app.should_quit);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::ExitSplitMode {
                pane_id,
                restore_window: None,
            } if pane_id == "%99"
        )),
        "should emit ExitSplitMode with no restore_window for empty split"
    );
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
fn epic_wrap_up_respawns_split_pane_only_once() {
    let mut app = App::new(
        vec![make_review_subtask(1, 10, 2), make_review_subtask(2, 10, 1)],
        TEST_TIMEOUT,
    );
    app.board.epics = vec![make_epic(10)];
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%5".to_string());
    app.board.split.pinned_task_id = Some(TaskId(2));
    app.input.mode = InputMode::ConfirmEpicWrapUp(EpicId(10));
    app.update(Message::EpicWrapUpRebase);

    // First task completes — this is the pinned one
    let cmds1 = app.update(Message::FinishComplete(TaskId(2)));
    let respawn_count_1 = cmds1
        .iter()
        .filter(|c| matches!(c, Command::RespawnSplitPane { .. }))
        .count();
    assert_eq!(respawn_count_1, 1, "should respawn once for pinned task");
    assert_eq!(app.board.split.pinned_task_id, None);

    // Second task completes — no longer pinned
    let cmds2 = app.update(Message::FinishComplete(TaskId(1)));
    let respawn_count_2 = cmds2
        .iter()
        .filter(|c| matches!(c, Command::RespawnSplitPane { .. }))
        .count();
    assert_eq!(respawn_count_2, 0, "should NOT respawn for non-pinned task");
}

// ---------------------------------------------------------------------------
// AgentTracking helper methods
// ---------------------------------------------------------------------------

#[test]
fn mark_active_sets_last_active_at_to_now() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    assert!(tracking.last_active_at.get(&TaskId(1)).is_none());

    tracking.mark_active(TaskId(1));

    let elapsed = tracking.last_active_at[&TaskId(1)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn mark_active_overwrites_previous_value() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    tracking
        .last_active_at
        .insert(TaskId(1), Instant::now() - Duration::from_secs(300));

    tracking.mark_active(TaskId(1));

    let elapsed = tracking.last_active_at[&TaskId(1)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn inactive_duration_returns_none_for_unknown_task() {
    let tracking = AgentTracking::new(TEST_TIMEOUT);
    assert!(tracking.inactive_duration(TaskId(99)).is_none());
}

#[test]
fn inactive_duration_returns_elapsed_time() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    tracking
        .last_active_at
        .insert(TaskId(1), Instant::now() - Duration::from_secs(60));

    let duration = tracking.inactive_duration(TaskId(1)).unwrap();
    assert!(duration >= Duration::from_secs(59));
    assert!(duration < Duration::from_secs(62));
}

#[test]
fn inactive_duration_near_zero_after_mark_active() {
    let mut tracking = AgentTracking::new(TEST_TIMEOUT);
    tracking.mark_active(TaskId(1));

    let duration = tracking.inactive_duration(TaskId(1)).unwrap();
    assert!(duration < Duration::from_secs(1));
}

#[test]
fn dependabot_right_arrow_clamps_at_column_2() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    // Switch to Dependabot mode: Reviewer → Author → Dependabot
    app.update(Message::ToggleReviewBoardMode);
    app.update(Message::ToggleReviewBoardMode);
    // Navigate right 10 times — should stop at column 2
    for _ in 0..10 {
        app.handle_key(make_key(KeyCode::Right));
    }
    let col = app.review_selection().unwrap().column();
    assert_eq!(
        col, 2,
        "Right arrow should clamp at column 2 in Dependabot mode"
    );
}

#[test]
fn dependabot_in_review_column_sorts_findings_ready_first() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    // Switch to Dependabot mode
    app.update(Message::ToggleReviewBoardMode);
    app.update(Message::ToggleReviewBoardMode);

    let reviewing_pr = make_bot_pr(
        10,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::Reviewing),
        crate::models::CiStatus::None,
    );
    let findings_pr = make_bot_pr(
        20,
        crate::models::ReviewDecision::ReviewRequired,
        Some(crate::models::ReviewAgentStatus::FindingsReady),
        crate::models::CiStatus::Success,
    );
    // Load in order: Reviewing first, FindingsReady second
    app.update(Message::PrsLoaded(
        PrListKind::Bot,
        vec![reviewing_pr, findings_pr],
    ));

    // Column 1 = In Review
    let prs = app.active_prs_for_column(1);
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0].number, 20, "FindingsReady (#20) should be first");
    assert_eq!(prs[1].number, 10, "Reviewing (#10) should be second");
}

#[test]
fn dependabot_board_shows_lifecycle_column_labels() {
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard);
    app.update(Message::ToggleReviewBoardMode); // → Author
    app.update(Message::ToggleReviewBoardMode); // → Dependabot
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Backlog"),
        "Backlog column label should be visible"
    );
    assert!(
        buffer_contains(&buf, "In Review"),
        "In Review column label should be visible"
    );
    assert!(
        !buffer_contains(&buf, "CI Passing"),
        "old CI Passing label should not appear"
    );
    assert!(
        !buffer_contains(&buf, "CI Failing"),
        "old CI Failing label should not appear"
    );
    assert!(
        !buffer_contains(&buf, "CI Pending"),
        "old CI Pending label should not appear"
    );
}

#[test]
fn reviewer_mode_column_sort_unaffected_by_dependabot_sort_key() {
    // Confirms that dependabot_sort_key returning 0 for all Reviewer PRs
    // leaves the existing repo-alphabetical sort intact.
    let mut app = make_app();
    app.update(Message::SwitchToReviewBoard); // starts in Reviewer mode
    app.update(Message::PrsLoaded(
        PrListKind::Review,
        vec![
            make_review_pr_for_repo(
                1,
                "alice",
                crate::models::ReviewDecision::ReviewRequired,
                "org/zebra",
            ),
            make_review_pr_for_repo(
                2,
                "bob",
                crate::models::ReviewDecision::ReviewRequired,
                "org/alpha",
            ),
        ],
    ));

    let col = crate::models::ReviewDecision::ReviewRequired.column_index();
    let prs = app.active_prs_for_column(col);
    assert_eq!(prs.len(), 2);
    assert_eq!(
        prs[0].repo, "org/alpha",
        "alphabetical sort should still apply"
    );
    assert_eq!(prs[1].repo, "org/zebra");
}

#[test]
fn epic_view_header_shows_auto_dispatch_indicator() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(1);
    epic.auto_dispatch = true;
    app.board.epics = vec![epic];
    app.update(Message::EnterEpic(EpicId(1)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "auto dispatch [U]"),
        "Expected 'auto dispatch [U]' in header"
    );
}

#[test]
fn epic_view_header_shows_manual_dispatch_indicator() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(1);
    epic.auto_dispatch = false;
    app.board.epics = vec![epic];
    app.update(Message::EnterEpic(EpicId(1)));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "manual dispatch [U]"),
        "Expected 'manual dispatch [U]' in header"
    );
}

#[test]
fn repo_cursor_resets_on_entering_repo_path_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.repo_cursor = 2; // cursor was left at position 2
    app.input.mode = InputMode::InputDescription;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: String::new(),
        ..Default::default()
    });
    app.input.buffer = "some desc".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(app.input.repo_cursor, 0, "cursor should reset to top");
}

#[test]
fn repo_cursor_resets_on_entering_epic_repo_path_mode() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.repo_cursor = 1;
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(crate::tui::types::EpicDraft {
        title: "E".to_string(),
        description: String::new(),
        repo_path: String::new(),
    });
    app.input.buffer = "epic desc".to_string();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicRepoPath);
    assert_eq!(app.input.repo_cursor, 0, "cursor should reset to top");
}

#[test]
fn fuzzy_matches_empty_query_matches_anything() {
    assert!(super::fuzzy_matches("/some/path", ""));
    assert!(super::fuzzy_matches("", ""));
}

#[test]
fn fuzzy_matches_subsequence() {
    assert!(super::fuzzy_matches("/tmp", "tmp"));
    assert!(super::fuzzy_matches("/home/tmp", "tmp"));
    assert!(super::fuzzy_matches("/home/ragge/proj", "ragge"));
}

#[test]
fn fuzzy_matches_case_insensitive() {
    assert!(super::fuzzy_matches("/TMP", "tmp"));
    assert!(super::fuzzy_matches("/tmp", "TMP"));
}

#[test]
fn fuzzy_matches_no_match() {
    assert!(!super::fuzzy_matches("/var", "tmp"));
}

#[test]
fn fuzzy_matches_chars_must_be_in_order() {
    // "tp" is a valid subsequence of "/tmp" (t at 1, p at 3)
    assert!(super::fuzzy_matches("/tmp", "tp"));
    // "pt" requires p before t: p is at 3, t is at 1 -> false
    assert!(!super::fuzzy_matches("/tmp", "pt"));
}

#[test]
fn filtered_repos_empty_query_returns_all() {
    let paths = vec!["/a".to_string(), "/b".to_string()];
    assert_eq!(super::filtered_repos(&paths, ""), vec!["/a", "/b"]);
}

#[test]
fn filtered_repos_narrows_by_query() {
    let paths = vec![
        "/home/ragge/proj".to_string(),
        "/var/log".to_string(),
        "/home/other".to_string(),
    ];
    let result = super::filtered_repos(&paths, "home");
    assert_eq!(result, vec!["/home/ragge/proj", "/home/other"]);
}

#[test]
fn filtered_repos_no_matches_returns_empty() {
    let paths = vec!["/tmp".to_string(), "/var".to_string()];
    assert_eq!(super::filtered_repos(&paths, "xyz"), Vec::<String>::new());
}

#[test]
fn repo_cursor_resets_on_copy_task() {
    let mut app = make_app();
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.repo_cursor = 2; // cursor was left at position 2
                               // Copy the first task (select it and press 'c')
    app.handle_key(make_key(KeyCode::Char('c')));
    assert_eq!(app.input.mode, InputMode::InputRepoPath);
    assert_eq!(
        app.input.repo_cursor, 0,
        "cursor should reset to top on copy"
    );
}

// =====================================================================
// Fuzzy filter wiring into input handlers (Task 3)
// =====================================================================

#[test]
fn move_repo_cursor_wraps_within_filtered_list() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var".to_string(), "/home".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Type "tmp" to filter — only /tmp matches (1 item)
    for c in "tmp".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    assert_eq!(app.input.repo_cursor, 0);
    // j wraps in a list of 1 — cursor stays at 0
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn number_key_selects_from_filtered_list() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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
    let mut app = App::new(vec![], TEST_TIMEOUT);
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

#[test]
fn render_repo_path_mode_shows_filtered_list_when_typing() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/tmp".to_string(), "/var/log".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    app.input.buffer = "tmp".to_string(); // filter active

    let buf = render_to_buffer(&mut app, 80, 20);
    assert!(buffer_contains(&buf, "/tmp"), "matching path should appear");
    assert!(
        !buffer_contains(&buf, "/var/log"),
        "non-matching path should be hidden"
    );
}

#[test]
fn render_repo_path_mode_shows_all_when_buffer_empty() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
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

#[test]
fn typing_resets_repo_cursor_to_zero() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        ..Default::default()
    });
    // Navigate to position 2
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 2);
    // Type a character — cursor should reset
    app.handle_key(make_key(KeyCode::Char('/')));
    assert_eq!(app.input.repo_cursor, 0);
}
