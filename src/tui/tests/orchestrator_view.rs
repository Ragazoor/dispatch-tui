//! The orchestrator view: the third lens on the main board
//! (`orchestrators.allium: OrchestratorView`; `board-layout.allium`,
//! "Board Lens", "Orchestrator View", "Every epic stays reachable").

use super::*;
use crate::models::{
    EpicOrchestration, LogEntryKind, OrchestrationBoard, Orchestrator, OrchestratorLogEntry,
    StartRule, TmuxWindow, WaitingItem,
};
use crate::tui::commands::{EpicCommand, TaskCommand};
use crate::tui::messages::EpicMessage;
use crate::tui::types::Lens;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const HERE: &str = "here";

fn switch(epic: i64) -> EpicOrchestration {
    EpicOrchestration {
        epic_id: EpicId(epic),
        start_rule: StartRule::MarkedOnly,
        max_parallel: None,
        switched_on_at: chrono::Utc::now(),
    }
}

fn orchestrator(epic: i64, live: bool) -> Orchestrator {
    Orchestrator {
        id: epic * 100,
        epic_id: EpicId(epic),
        host: HERE.to_string(),
        name: format!("orch{epic}"),
        tmux_window: live.then(|| TmuxWindow::for_orchestrator(EpicId(epic))),
        started_at: None,
        restarted_once: false,
        session_died: false,
        last_turn_at: None,
    }
}

fn item(id: i64, epic: i64) -> WaitingItem {
    WaitingItem {
        id,
        epic_id: EpicId(epic),
        host: HERE.to_string(),
        task_id: None,
        about: format!("question {id}"),
        tried: "asked".to_string(),
        recommendation: "say yes".to_string(),
        raised_at: chrono::Utc::now(),
        resolved_at: None,
    }
}

fn running_here(id: i64, epic: i64) -> Task {
    let mut t = epic_task(id, epic, TaskStatus::Running);
    t.host = Some(HERE.to_string());
    t.tmux_window = Some(crate::models::test_tmux_window(&format!("task-{id}")));
    t
}

fn key(app: &mut App, code: KeyCode) -> Vec<Command> {
    let mods = match code {
        KeyCode::Char(c) if c.is_ascii_uppercase() => KeyModifiers::SHIFT,
        KeyCode::BackTab => KeyModifiers::SHIFT,
        _ => KeyModifiers::NONE,
    };
    without_usage(app.handle_key(KeyEvent::new(code, mods)))
}

fn refresh(app: &mut App, board: OrchestrationBoard) {
    app.update(Message::Epic(EpicMessage::OrchestrationRefreshed(
        Box::new(board),
    )));
}

/// The board, this machine named, on the orchestrators lens.
fn orchestrator_app(tasks: Vec<Task>, epics: Vec<Epic>, board: OrchestrationBoard) -> App {
    let mut app = App::new(tasks);
    app.board.epics = epics;
    app.set_local_host_id(HERE.to_string());
    refresh(&mut app, board);
    cycle_lens_to(&mut app, Lens::Orchestrators);
    app
}

/// The epic ids of the orchestrator cards in each column, Backlog first.
fn cards(app: &App) -> Vec<Vec<i64>> {
    TaskStatus::ALL
        .iter()
        .map(|s| {
            app.view()
                .column_items_for_status(*s)
                .iter()
                .map(|i| match i {
                    ColumnItem::Orchestrator(e) => e.id.0,
                    other => panic!("the orchestrator view drew {other:?}"),
                })
                .collect()
        })
        .collect()
}

/// A column's navigation index, 1-based.
fn nav(status: TaskStatus) -> usize {
    status.column_index() + 1
}

fn on_ten() -> OrchestrationBoard {
    OrchestrationBoard {
        orchestrations: vec![switch(10)],
        orchestrators: vec![orchestrator(10, true)],
        ..OrchestrationBoard::default()
    }
}

/// Put the cursor on the first card of the first column that has one.
fn on_first_card(app: &mut App) {
    for col in 1..=TaskStatus::COLUMN_COUNT {
        app.selection_mut().set_column(col);
        app.selection_mut().set_row(col, 0);
        if let Some(ColumnItem::Orchestrator(e)) = app.selected_column_item() {
            let id = e.id;
            app.selection_mut().anchor = Some(crate::tui::types::ColumnAnchor::Epic(id));
            return;
        }
    }
    panic!("no orchestrator card on the board");
}

// ---- LensOrder ----

#[test]
fn the_board_starts_on_flat() {
    let app = App::new(vec![]);
    assert_eq!(app.filter.lens, Lens::Flat);
}

#[test]
fn tab_steps_flat_flat_active_orchestrators_and_wraps() {
    let mut app = App::new(vec![]);
    let mut seen = vec![];
    for _ in 0..3 {
        key(&mut app, KeyCode::Tab);
        seen.push(app.filter.lens);
    }
    assert_eq!(
        seen,
        vec![Lens::FlatActive, Lens::Orchestrators, Lens::Flat]
    );
}

#[test]
fn back_tab_steps_the_other_way() {
    let mut app = App::new(vec![]);
    let mut seen = vec![];
    for _ in 0..3 {
        key(&mut app, KeyCode::BackTab);
        seen.push(app.filter.lens);
    }
    assert_eq!(
        seen,
        vec![Lens::Orchestrators, Lens::FlatActive, Lens::Flat]
    );
}

#[test]
fn the_orchestrators_lens_shows_its_badge_on_the_board_and_inside_an_epic() {
    let mut app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Backlog)],
        vec![make_epic(10)],
        on_ten(),
    );
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(buffer_contains(&buf, "[orch]"));
    assert!(!buffer_contains(&buf, "[flat]"));
    app.update(Message::Epic(EpicMessage::Enter(EpicId(10))));
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(buffer_contains(&buf, "[orch]"), "the badge names the lens");
}

// ---- OneCardPerOrchestratedEpic ----

#[test]
fn the_view_holds_one_card_per_orchestrated_epic_and_nothing_else() {
    let mut sub = make_epic(30);
    sub.parent_epic_id = Some(EpicId(20));
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10), switch(30)],
        orchestrators: vec![orchestrator(10, false), orchestrator(30, false)],
        ..OrchestrationBoard::default()
    };
    let app = orchestrator_app(
        vec![
            make_task(1, TaskStatus::Running),
            epic_task(2, 10, TaskStatus::Backlog),
            epic_task(3, 20, TaskStatus::Backlog),
            epic_task(4, 30, TaskStatus::Backlog),
        ],
        vec![make_epic(10), make_epic(20), sub],
        board,
    );
    let all: Vec<i64> = cards(&app).into_iter().flatten().collect();
    assert_eq!(
        all,
        vec![10, 30],
        "a sub-epic's orchestrator has a card too"
    );
}

#[test]
fn an_epic_with_no_orchestrator_row_here_shows_as_resting_in_backlog() {
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10)],
        ..OrchestrationBoard::default()
    };
    let mut app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Backlog)],
        vec![make_epic_with_title(10, "Payments")],
        board,
    );
    assert_eq!(cards(&app)[0], vec![10]);
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(buffer_contains(&buf, "resting"));
}

#[test]
fn a_hidden_epic_and_one_the_repo_filter_excludes_draw_no_card() {
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10), switch(20)],
        ..OrchestrationBoard::default()
    };
    let mut other = epic_task(2, 20, TaskStatus::Backlog);
    other.repo_path = "/other".to_string();
    let mut app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Backlog), other],
        vec![make_epic(10), make_epic(20)],
        board,
    );
    app.filter.hidden_epics.insert(EpicId(10));
    app.filter.repos.insert("/repo".to_string());
    app.sync_board_selection();
    let all: Vec<i64> = cards(&app).into_iter().flatten().collect();
    assert!(all.is_empty(), "{all:?}");
}

#[test]
fn the_search_narrows_the_cards_by_the_epics_own_title_only() {
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10), switch(20)],
        ..OrchestrationBoard::default()
    };
    let mut task = epic_task(1, 20, TaskStatus::Backlog);
    task.title = "Payments follow-up".to_string();
    let mut app = orchestrator_app(
        vec![task],
        vec![
            make_epic_with_title(10, "Payments"),
            make_epic_with_title(20, "Search"),
        ],
        board,
    );
    app.search.query = "payments".to_string();
    app.sync_board_selection();
    let all: Vec<i64> = cards(&app).into_iter().flatten().collect();
    assert_eq!(
        all,
        vec![10],
        "a task's match does not keep its epic's card"
    );
}

#[test]
fn cards_follow_the_epics_card_order() {
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10), switch(20)],
        ..OrchestrationBoard::default()
    };
    let mut first = make_epic(20);
    first.sort_order = Some(1);
    let mut second = make_epic(10);
    second.sort_order = Some(2);
    let app = orchestrator_app(
        vec![
            epic_task(1, 10, TaskStatus::Backlog),
            epic_task(2, 20, TaskStatus::Backlog),
        ],
        vec![second, first],
        board,
    );
    assert_eq!(cards(&app)[0], vec![20, 10]);
}

// ---- EpicCardInOrchestratorView ----

#[test]
fn an_open_item_puts_the_card_in_review_even_with_a_task_running_here() {
    let board = OrchestrationBoard {
        open_items: vec![item(1, 10)],
        ..on_ten()
    };
    let app = orchestrator_app(vec![running_here(1, 10)], vec![make_epic(10)], board);
    assert_eq!(cards(&app), vec![vec![], vec![], vec![10], vec![]]);
}

#[test]
fn a_task_running_here_puts_the_card_in_running() {
    let app = orchestrator_app(vec![running_here(1, 10)], vec![make_epic(10)], on_ten());
    assert_eq!(cards(&app), vec![vec![], vec![10], vec![], vec![]]);
}

#[test]
fn a_task_running_on_another_machine_does_not() {
    let mut elsewhere = running_here(1, 10);
    elsewhere.host = Some("other".to_string());
    let app = orchestrator_app(vec![elsewhere], vec![make_epic(10)], on_ten());
    assert_eq!(cards(&app), vec![vec![10], vec![], vec![], vec![]]);
}

#[test]
fn an_epic_whose_tasks_are_all_done_is_in_done() {
    let app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Done)],
        vec![make_epic(10)],
        on_ten(),
    );
    assert_eq!(cards(&app), vec![vec![], vec![], vec![], vec![10]]);
}

#[test]
fn an_epic_with_no_tasks_is_in_backlog() {
    let app = orchestrator_app(vec![], vec![make_epic(10)], on_ten());
    assert_eq!(cards(&app), vec![vec![10], vec![], vec![], vec![]]);
}

#[test]
fn the_card_follows_a_refresh_to_its_new_column() {
    let mut app = orchestrator_app(vec![running_here(1, 10)], vec![make_epic(10)], on_ten());
    on_first_card(&mut app);
    assert_eq!(app.selected_column_status(), Some(TaskStatus::Running));
    refresh(
        &mut app,
        OrchestrationBoard {
            open_items: vec![item(1, 10)],
            ..on_ten()
        },
    );
    assert_eq!(cards(&app), vec![vec![], vec![], vec![10], vec![]]);
    assert!(matches!(
        app.selected_column_item(),
        Some(ColumnItem::Orchestrator(e)) if e.id == EpicId(10)
    ));
}

#[test]
fn the_cursor_keys_move_between_orchestrator_cards() {
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10), switch(20), switch(30)],
        orchestrators: vec![orchestrator(30, true)],
        ..OrchestrationBoard::default()
    };
    let mut app = orchestrator_app(
        vec![running_here(1, 30)],
        vec![make_epic(10), make_epic(20), make_epic(30)],
        board,
    );
    assert_eq!(cards(&app), vec![vec![10, 20], vec![30], vec![], vec![]]);
    app.selection_mut().set_column(nav(TaskStatus::Backlog));
    app.selection_mut().set_row(nav(TaskStatus::Backlog), 0);
    let on = |app: &App| match app.selected_column_item() {
        Some(ColumnItem::Orchestrator(e)) => Some(e.id.0),
        _ => None,
    };
    assert_eq!(on(&app), Some(10));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(on(&app), Some(20));
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(on(&app), Some(20), "the last card holds the cursor");
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(on(&app), Some(30));
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(on(&app), Some(10), "a new column starts at its first card");
    key(&mut app, KeyCode::Char('j'));
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(on(&app), Some(10));
}

// ---- CardAnswersDoINeedToLook ----

#[test]
fn the_card_shows_state_waiting_count_limit_and_newest_log_line() {
    let board = OrchestrationBoard {
        open_items: vec![item(1, 10)],
        log: vec![OrchestratorLogEntry {
            id: 7,
            epic_id: EpicId(10),
            host: HERE.to_string(),
            at: chrono::Utc::now(),
            kind: LogEntryKind::StartedTask,
            task_id: Some(TaskId(1)),
            text: "started the parser".to_string(),
            question: None,
            trouble: None,
        }],
        ..on_ten()
    };
    let mut app = orchestrator_app(
        vec![running_here(1, 10), epic_task(2, 10, TaskStatus::Backlog)],
        vec![make_epic_with_title(10, "Payments")],
        board,
    );
    let buf = render_to_buffer(&mut app, 160, 40);
    for want in [
        "Payments",
        "working",
        "1 waiting",
        "1/3",
        "started the parser",
    ] {
        assert!(buffer_contains(&buf, want), "missing {want:?}");
    }
}

#[test]
fn a_down_orchestrator_reads_down_and_no_waiting_count_at_zero() {
    let board = OrchestrationBoard {
        orchestrations: vec![switch(10)],
        orchestrators: vec![orchestrator(10, false)],
        ..OrchestrationBoard::default()
    };
    let mut app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Backlog)],
        vec![make_epic(10)],
        board,
    );
    let buf = render_to_buffer(&mut app, 160, 40);
    assert!(buffer_contains(&buf, "down"));
    assert!(!buffer_contains(&buf, "waiting"));
}

// ---- KeysOnTheCard ----

#[test]
fn enter_opens_the_epic_view_flat_and_q_comes_back_to_the_orchestrator_view() {
    let mut app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Backlog), running_here(2, 10)],
        vec![make_epic(10)],
        on_ten(),
    );
    on_first_card(&mut app);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.current_epic_id(), Some(EpicId(10)));
    assert_eq!(app.filter.lens, Lens::Orchestrators);
    let running: Vec<TaskId> = app
        .view()
        .column_items_for_status(TaskStatus::Running)
        .iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(t.id),
            _ => None,
        })
        .collect();
    assert_eq!(running, vec![TaskId(2)], "the epic view reads flat");

    key(&mut app, KeyCode::Char('q'));
    assert_eq!(app.current_epic_id(), None);
    assert_eq!(cards(&app), vec![vec![], vec![10], vec![], vec![]]);
}

#[test]
fn space_opens_the_epic_view_too() {
    let mut app = orchestrator_app(vec![], vec![make_epic(10)], on_ten());
    on_first_card(&mut app);
    key(&mut app, KeyCode::Char(' '));
    assert_eq!(app.current_epic_id(), Some(EpicId(10)));
}

#[test]
fn the_orchestrator_key_opens_its_window() {
    let mut app = orchestrator_app(vec![running_here(1, 10)], vec![make_epic(10)], on_ten());
    on_first_card(&mut app);
    let cmds = key(&mut app, KeyCode::Char('O'));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::JumpToTmux { .. }))),
        "{cmds:?}"
    );
}

#[test]
fn the_edit_key_opens_the_epics_edit_form() {
    let mut app = orchestrator_app(vec![], vec![make_epic(10)], on_ten());
    on_first_card(&mut app);
    let cmds = key(&mut app, KeyCode::Char('e'));
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::Editor(crate::tui::commands::EditorCommand::PopOut(
                crate::tui::types::EditKind::EpicEdit(epic, Some(_)),
            ))] if epic.id == EpicId(10)
        ),
        "{cmds:?}"
    );
}

#[test]
fn the_hint_bar_names_the_cards_keys() {
    let mut app = orchestrator_app(vec![], vec![make_epic(10)], on_ten());
    on_first_card(&mut app);
    let buf = render_to_buffer(&mut app, 160, 40);
    for want in ["[Enter] open", "[e]dit", "[O]rchestrator"] {
        assert!(buffer_contains(&buf, want), "missing {want:?}");
    }
    assert!(!buffer_contains(&buf, "[x] delete"));
}

#[test]
fn a_selection_from_flat_is_kept_but_not_acted_on_in_the_orchestrator_view() {
    let mut app = App::new(vec![epic_task(1, 10, TaskStatus::Running)]);
    app.board.epics = vec![make_epic(10)];
    app.set_local_host_id(HERE.to_string());
    refresh(&mut app, on_ten());
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    assert!(app.selected_tasks().contains(&TaskId(1)));
    cycle_lens_to(&mut app, Lens::Orchestrators);
    on_first_card(&mut app);
    for c in ['x', 'H', 'L', 'm'] {
        let cmds = key(&mut app, KeyCode::Char(c));
        assert!(cmds.is_empty(), "{c} sent {cmds:?}");
        assert!(matches!(app.input.mode, InputMode::Normal), "{c}");
    }
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.board.tasks[0].status, TaskStatus::Running);
    cycle_lens_to(&mut app, Lens::Flat);
    assert!(
        app.selected_tasks().contains(&TaskId(1)),
        "the selection is back in flat"
    );
}

#[test]
fn the_orchestrator_view_has_no_select_all_row() {
    let mut app = orchestrator_app(vec![], vec![make_epic(10)], on_ten());
    on_first_card(&mut app);
    key(&mut app, KeyCode::Char('k'));
    assert!(!app.selection().on_select_all);
    assert!(matches!(
        app.selected_column_item(),
        Some(ColumnItem::Orchestrator(_))
    ));
    let buf = render_to_buffer(&mut app, 160, 40);
    assert!(!buffer_contains(&buf, "[ ]"), "no select-all checkbox");
}

#[test]
fn a_detached_running_task_takes_no_slot_and_keeps_the_card_idle_in_backlog() {
    let mut detached = epic_task(1, 10, TaskStatus::Running);
    detached.host = Some(HERE.to_string());
    detached.worktree = Some("/wt/1".to_string());
    detached.tmux_window = None;
    let mut app = orchestrator_app(vec![detached], vec![make_epic(10)], on_ten());
    assert_eq!(cards(&app), vec![vec![10], vec![], vec![], vec![]]);
    let buf = render_to_buffer(&mut app, 160, 40);
    assert!(buffer_contains(&buf, "idle"));
    assert!(buffer_contains(&buf, "0/3"));
}

#[test]
fn every_other_card_key_does_nothing() {
    let mut app = orchestrator_app(
        vec![epic_task(1, 10, TaskStatus::Backlog)],
        vec![make_epic(10)],
        on_ten(),
    );
    on_first_card(&mut app);
    for c in [
        'x', 'm', 'w', 'v', 'H', 'L', 'J', 'K', 'b', 'i', 'Y', 'z', 'Z',
    ] {
        let cmds = key(&mut app, KeyCode::Char(c));
        assert!(
            cmds.iter()
                .all(|c| !matches!(c, Command::Epic(EpicCommand::SwitchOrchestratorOn(_)))),
            "{c}"
        );
        assert!(cmds.is_empty(), "{c} sent {cmds:?}");
        assert!(
            matches!(app.input.mode, InputMode::Normal),
            "{c} opened {:?}",
            app.input.mode
        );
        assert!(app.selected_epics().is_empty(), "{c} selected the card");
        assert_eq!(app.current_epic_id(), None, "{c}");
    }
}

// ---- Every epic stays reachable (board-layout.allium) ----

/// The ids a flat board can enter from: epic cards in Backlog, and the
/// epics of the task cards in the other columns.
fn enterable_epics(app: &App) -> Vec<i64> {
    let mut ids: Vec<i64> = TaskStatus::ALL
        .iter()
        .flat_map(|s| app.view().column_items_for_status(*s))
        .filter_map(|i| match i {
            ColumnItem::Epic(e) => Some(e.id.0),
            ColumnItem::Task(t) => t.epic_id.map(|e| e.0),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[test]
fn an_epic_with_no_tasks_has_a_backlog_card_on_flat() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    let backlog = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(matches!(backlog.as_slice(), [ColumnItem::Epic(e)] if e.id == EpicId(10)));
}

#[test]
fn a_sub_epic_whose_work_has_all_started_is_reached_through_its_task_inside_its_parent() {
    let mut sub = make_epic(20);
    sub.parent_epic_id = Some(EpicId(10));
    let mut app = App::new(vec![
        epic_task(1, 10, TaskStatus::Backlog),
        epic_task(2, 20, TaskStatus::Review),
    ]);
    app.board.epics = vec![make_epic(10), sub];
    app.update(Message::Epic(EpicMessage::Enter(EpicId(10))));
    let review = app.view().column_items_for_status(TaskStatus::Review);
    assert!(
        review
            .iter()
            .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(2))),
        "the sub-epic's task is a card in Review"
    );
    let backlog = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(
        !backlog.iter().any(|i| matches!(i, ColumnItem::Epic(_))),
        "no sub-epic card: its work has all started"
    );
    app.selection_mut().set_column(nav(TaskStatus::Review));
    app.selection_mut().set_row(nav(TaskStatus::Review), 0);
    while !matches!(app.selected_column_item(), Some(ColumnItem::Task(_))) {
        let row = app.selection().row(nav(TaskStatus::Review)) + 1;
        app.selection_mut().set_row(nav(TaskStatus::Review), row);
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.current_epic_id(), Some(EpicId(20)));
}

#[test]
fn a_finished_epic_is_reached_through_a_done_task() {
    let mut app = App::new(vec![epic_task(1, 10, TaskStatus::Done)]);
    app.board.epics = vec![make_epic(10)];
    assert_eq!(enterable_epics(&app), vec![10]);
    let done = nav(TaskStatus::Done);
    app.selection_mut().set_column(done);
    app.selection_mut().set_row(done, 0);
    while !matches!(app.selected_column_item(), Some(ColumnItem::Task(_))) {
        let row = app.selection().row(done) + 1;
        app.selection_mut().set_row(done, row);
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.current_epic_id(), Some(EpicId(10)));
}

#[test]
fn a_finished_epic_has_its_card_in_done_and_nowhere_else() {
    let mut app = App::new(vec![epic_task(1, 10, TaskStatus::Done)]);
    app.board.epics = vec![make_epic(10)];
    let epic_columns: Vec<TaskStatus> = TaskStatus::ALL
        .iter()
        .copied()
        .filter(|s| {
            app.view()
                .column_items_for_status(*s)
                .iter()
                .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(10)))
        })
        .collect();
    assert_eq!(epic_columns, vec![TaskStatus::Done]);
}

#[test]
fn the_cursor_keys_reach_a_finished_epics_card_between_done_tasks() {
    let done_at = |id, epic: Option<i64>, secs: i64| {
        let mut t = make_task(id, TaskStatus::Done);
        t.epic_id = epic.map(EpicId);
        t.completed_at = chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0);
        t
    };
    let mut app = App::new(vec![
        done_at(1, None, 300),
        done_at(2, Some(10), 200),
        done_at(3, None, 100),
    ]);
    app.board.epics = vec![make_epic(10)];
    let done = nav(TaskStatus::Done);
    app.selection_mut().set_column(done);
    app.selection_mut().set_row(done, 0);
    let mut seen = vec![];
    for _ in 0..4 {
        seen.push(match app.selected_column_item() {
            Some(ColumnItem::Task(t)) => format!("t{}", t.id.0),
            Some(ColumnItem::Epic(e)) => format!("e{}", e.id.0),
            other => format!("{other:?}"),
        });
        key(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(seen, vec!["t1", "e10", "t2", "t3"]);
}

#[test]
fn an_epic_with_tasks_only_in_running_or_review_has_no_card() {
    let mut app = App::new(vec![
        epic_task(1, 10, TaskStatus::Running),
        epic_task(2, 10, TaskStatus::Review),
        epic_task(3, 10, TaskStatus::Done),
    ]);
    app.board.epics = vec![make_epic(10)];
    let cards: Vec<_> = TaskStatus::ALL
        .iter()
        .flat_map(|s| app.view().column_items_for_status(*s))
        .filter(|i| matches!(i, ColumnItem::Epic(_)))
        .collect();
    assert!(cards.is_empty(), "{cards:?}");
    assert_eq!(
        enterable_epics(&app),
        vec![10],
        "its task cards still lead to it"
    );
}

#[test]
fn the_filters_hide_an_epics_way_in_and_nothing_else() {
    let mut other = epic_task(2, 20, TaskStatus::Running);
    other.repo_path = "/other".to_string();
    let mut app = App::new(vec![epic_task(1, 10, TaskStatus::Running), other]);
    app.board.epics = vec![make_epic(10), make_epic(20), make_epic(30)];
    assert_eq!(enterable_epics(&app), vec![10, 20, 30]);
    app.filter.repos.insert("/repo".to_string());
    app.filter.hidden_epics.insert(EpicId(30));
    app.sync_board_selection();
    assert_eq!(
        enterable_epics(&app),
        vec![10],
        "the repo filter and a hide take their epics' way in, and only theirs"
    );
}
