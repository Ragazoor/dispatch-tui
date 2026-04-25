use crossterm::event::KeyCode;

use super::super::{App, Command, InputMode, Message, ViewMode};
use super::{make_app, make_epic, make_feed_epic, make_key};
use crate::models::{EpicId, TaskId, TaskStatus};

/// Drives an `App` through a sequence of key events, collecting all `Command`s emitted.
struct Scenario {
    app: App,
    commands: Vec<Command>,
}

impl Scenario {
    fn new() -> Self {
        Self {
            app: make_app(),
            commands: vec![],
        }
    }

    fn with_app(app: App) -> Self {
        Self {
            app,
            commands: vec![],
        }
    }

    fn key(&mut self, code: KeyCode) -> &mut Self {
        let cmds = self.app.handle_key(make_key(code));
        self.commands.extend(cmds);
        self
    }

    fn char_keys(&mut self, s: &str) -> &mut Self {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
        self
    }
}

#[test]
fn scenario_task_creation_dialog_enters_input_title_mode() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('n'));
    assert!(
        matches!(s.app.input.mode, InputMode::InputTitle),
        "expected InputTitle after pressing n, got {:?}",
        s.app.input.mode
    );
}

#[test]
fn scenario_task_creation_empty_title_returns_to_normal() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('n'));
    s.key(KeyCode::Enter);
    assert!(
        matches!(s.app.input.mode, InputMode::Normal),
        "empty title should return to Normal, got {:?}",
        s.app.input.mode
    );
}

#[test]
fn scenario_task_creation_typing_title_advances_to_tag_input() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('n'));
    s.char_keys("My Task");
    s.key(KeyCode::Enter);
    assert!(
        matches!(s.app.input.mode, InputMode::InputTag),
        "expected InputTag after submitting title, got {:?}",
        s.app.input.mode
    );
}

#[test]
fn scenario_task_creation_esc_cancels_from_title_input() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('n'));
    s.char_keys("partial");
    s.key(KeyCode::Esc);
    assert!(
        matches!(s.app.input.mode, InputMode::Normal),
        "Esc should cancel back to Normal, got {:?}",
        s.app.input.mode
    );
    assert!(
        s.app.input.buffer.is_empty(),
        "buffer should be cleared after cancel"
    );
}

#[test]
fn scenario_quick_dispatch_with_repo_path_emits_command() {
    let mut app = make_app();
    app.update(Message::RepoPathsUpdated(vec!["/repo".to_string()]));

    let mut s = Scenario::with_app(app);
    s.key(KeyCode::Char('D'));

    assert!(
        s.commands
            .iter()
            .any(|c| matches!(c, Command::QuickDispatch { .. })),
        "expected QuickDispatch command, got {:?}",
        s.commands
    );
}

#[test]
fn scenario_quick_dispatch_without_repo_path_shows_no_dispatch_command() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('D'));

    assert!(
        !s.commands
            .iter()
            .any(|c| matches!(c, Command::QuickDispatch { .. })),
        "should not emit QuickDispatch without a repo path"
    );
}

#[test]
fn scenario_tab_from_board_without_feed_epics_is_noop_for_board_switch() {
    // Without feed epics, Tab from Board is a no-op — nothing to cycle to.
    let mut s = Scenario::new();
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Board(_)),
        "should start on kanban board"
    );
    s.key(KeyCode::Tab);
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Board(_)),
        "Tab from Board with no feed epics should stay on Board, got {:?}",
        s.app.board.view_mode
    );
}

#[test]
fn scenario_help_overlay_opens_on_question_mark() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('?'));
    assert!(
        matches!(s.app.input.mode, InputMode::Help),
        "expected Help mode after '?', got {:?}",
        s.app.input.mode
    );
}

#[test]
fn scenario_help_overlay_toggles_closed_on_second_question_mark() {
    let mut s = Scenario::new();
    s.key(KeyCode::Char('?'));
    s.key(KeyCode::Char('?'));
    assert!(
        matches!(s.app.input.mode, InputMode::Normal),
        "expected Normal mode after closing help, got {:?}",
        s.app.input.mode
    );
}

#[test]
fn scenario_move_key_advances_selected_task_to_next_column() {
    // Backlog column is selected by default; 'm' moves task 1 forward (Backlog → Running).
    let mut s = Scenario::new();
    s.key(KeyCode::Char('m'));

    let task1 = s
        .app
        .board
        .tasks
        .iter()
        .find(|t| t.id == TaskId(1))
        .expect("task 1 should exist");
    assert_eq!(
        task1.status,
        TaskStatus::Running,
        "task 1 should be moved to Running after 'm'"
    );
    assert!(
        s.commands
            .iter()
            .any(|c| matches!(c, Command::PersistTask(_))),
        "move should emit PersistTask command"
    );
}

// ---------------------------------------------------------------------------
// Tab key feed-epic cycling
// ---------------------------------------------------------------------------

#[test]
fn scenario_tab_from_board_with_no_feed_epics_is_noop() {
    let mut s = Scenario::new();
    // make_app() has no epics at all
    s.key(KeyCode::Tab);
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Board(_)),
        "Tab with no feed epics should stay on Board, got {:?}",
        s.app.board.view_mode
    );
}

#[test]
fn scenario_tab_from_board_enters_first_feed_epic() {
    let mut app = make_app();
    app.board.epics = vec![
        make_feed_epic(1, "Reviews", 1),
        make_feed_epic(2, "Security", 2),
    ];
    let mut s = Scenario::with_app(app);
    s.key(KeyCode::Tab);
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(1)),
        "Tab from Board should enter first feed epic (id=1), got {:?}",
        s.app.board.view_mode
    );
}

#[test]
fn scenario_tab_from_middle_feed_epic_goes_to_next() {
    let mut app = make_app();
    app.board.epics = vec![
        make_feed_epic(1, "Reviews", 1),
        make_feed_epic(2, "Security", 2),
    ];
    app.update(Message::EnterEpic(EpicId(1)));
    let mut s = Scenario::with_app(app);
    s.key(KeyCode::Tab);
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(2)),
        "Tab from first feed epic should go to second, got {:?}",
        s.app.board.view_mode
    );
}

#[test]
fn scenario_tab_from_last_feed_epic_returns_to_board() {
    let mut app = make_app();
    app.board.epics = vec![
        make_feed_epic(1, "Reviews", 1),
        make_feed_epic(2, "Security", 2),
    ];
    app.update(Message::EnterEpic(EpicId(2)));
    let mut s = Scenario::with_app(app);
    s.key(KeyCode::Tab);
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Board(_)),
        "Tab from last feed epic should return to Board, got {:?}",
        s.app.board.view_mode
    );
}

#[test]
fn scenario_tab_from_non_feed_epic_is_noop() {
    let mut app = make_app();
    // A regular epic with no feed_command — Tab should do nothing
    let regular_epic = make_epic(99);
    app.board.epics = vec![regular_epic, make_feed_epic(1, "Reviews", 1)];
    app.update(Message::EnterEpic(EpicId(99)));
    let mut s = Scenario::with_app(app);
    s.key(KeyCode::Tab);
    assert!(
        matches!(s.app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(99)),
        "Tab from a non-feed epic should be a no-op, got {:?}",
        s.app.board.view_mode
    );
}
