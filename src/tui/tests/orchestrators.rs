//! The orchestrator on the board (`orchestrators.allium: OrchestratorInEpicView`,
//! `OrchestratorNotifications`; `epics.allium: OrchestratorHeader`;
//! `board-visuals.allium`, "Ready marker").

use super::*;
use crate::models::{
    EpicOrchestration, LogEntryKind, OrchestrationBoard, Orchestrator, OrchestratorLogEntry,
    ReadyMark, ReadyMarker, StartRule, TmuxWindow, WaitingItem,
};
use crate::tui::commands::{EpicCommand, SystemCommand, TaskCommand};
use crate::tui::messages::EpicMessage;

const HERE: &str = "here";

fn switch(epic: i64) -> EpicOrchestration {
    EpicOrchestration {
        epic_id: EpicId(epic),
        start_rule: StartRule::MarkedOnly,
        max_parallel: None,
        switched_on_at: chrono::Utc::now(),
    }
}

fn orchestrator(epic: i64, host: &str, live: bool) -> Orchestrator {
    Orchestrator {
        id: epic * 100,
        epic_id: EpicId(epic),
        host: host.to_string(),
        name: format!("orch{epic}"),
        tmux_window: live.then(|| TmuxWindow::for_orchestrator(EpicId(epic))),
        started_at: None,
        restarted_once: false,
        session_died: false,
        last_turn_at: None,
    }
}

fn item(id: i64, epic: i64, host: &str, task: Option<i64>) -> WaitingItem {
    WaitingItem {
        id,
        epic_id: EpicId(epic),
        host: host.to_string(),
        task_id: task.map(TaskId),
        about: format!("question {id}"),
        tried: "asked".to_string(),
        recommendation: "say yes".to_string(),
        raised_at: chrono::Utc::now(),
        resolved_at: None,
    }
}

fn entry(id: i64, epic: i64, text: &str) -> OrchestratorLogEntry {
    OrchestratorLogEntry {
        id,
        epic_id: EpicId(epic),
        host: HERE.to_string(),
        at: chrono::Utc::now(),
        kind: LogEntryKind::StartedTask,
        task_id: Some(TaskId(1)),
        text: text.to_string(),
        question: None,
        trouble: None,
    }
}

/// Epic 10 with a backlog task #1 and a running task #2, orchestrated, its
/// orchestrator on this machine with no session.
fn covered_app(board: OrchestrationBoard) -> App {
    let mut app = App::new(vec![
        epic_task(1, 10, TaskStatus::Backlog),
        epic_task(2, 10, TaskStatus::Running),
    ]);
    app.board.epics = vec![make_epic_with_title(10, "Payments")];
    app.set_local_host_id(HERE.to_string());
    app.update(Message::Epic(EpicMessage::OrchestrationRefreshed(
        Box::new(board),
    )));
    app
}

fn on_ten(live: bool) -> OrchestrationBoard {
    OrchestrationBoard {
        orchestrations: vec![switch(10)],
        orchestrators: vec![orchestrator(10, HERE, live)],
        ..OrchestrationBoard::default()
    }
}

fn enter(app: &mut App, epic: i64) {
    app.update(Message::Epic(EpicMessage::Enter(EpicId(epic))));
}

fn press(app: &mut App, message: EpicMessage) -> Vec<Command> {
    without_usage(app.update(Message::Epic(message)))
}

/// Put the cursor on task `id` inside epic 10's view, where its card is.
fn select_task(app: &mut App, id: i64) {
    enter(app, 10);
    'search: for col in 0..5 {
        for row in 0..5 {
            app.selection_mut().set_column(col);
            app.selection_mut().set_row(col, row);
            if app.selected_task().map(|t| t.id) == Some(TaskId(id)) {
                break 'search;
            }
        }
    }
    assert_eq!(app.selected_task().map(|t| t.id), Some(TaskId(id)));
}

// ---- the ready key and the marker ----

#[test]
fn the_ready_key_marks_a_backlog_task_and_clears_it_again() {
    let mut app = covered_app(on_ten(false));
    select_task(&mut app, 1);

    let cmds = press(&mut app, EpicMessage::ToggleReady);
    assert!(
        matches!(cmds.as_slice(), [Command::Epic(EpicCommand::SetReady { task, ready: true })] if *task == TaskId(1)),
        "{cmds:?}"
    );
    assert!(app.board.orchestration.ready_mark(TaskId(1)).is_some());

    let cmds = press(&mut app, EpicMessage::ToggleReady);
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::Epic(EpicCommand::SetReady { ready: false, .. })]
        ),
        "{cmds:?}"
    );
    assert!(app.board.orchestration.ready_mark(TaskId(1)).is_none());
}

#[test]
fn the_ready_key_on_a_task_out_of_backlog_changes_nothing() {
    let mut app = covered_app(on_ten(false));
    select_task(&mut app, 2);

    let cmds = press(&mut app, EpicMessage::ToggleReady);

    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("Only a backlog task"));
}

fn marked(task: i64, agent: Option<&str>) -> ReadyMark {
    ReadyMark {
        task_id: TaskId(task),
        marked_by: if agent.is_some() {
            ReadyMarker::Agent
        } else {
            ReadyMarker::Operator
        },
        agent_name: agent.map(str::to_string),
        marked_at: chrono::Utc::now(),
    }
}

#[test]
fn a_ready_backlog_card_says_so_and_names_the_agent_that_marked_it() {
    let mut board = on_ten(false);
    board.ready_marks = vec![marked(1, Some("alina2"))];
    let mut app = covered_app(board);
    enter(&mut app, 10);

    let buf = render_to_buffer(&mut app, 160, 30);

    assert!(buffer_contains(&buf, "\u{2713} ready \u{00b7} alina2"));
}

#[test]
fn only_a_real_mark_shows_not_a_start_rule_of_everything() {
    let mut board = on_ten(false);
    board.orchestrations[0].start_rule = StartRule::Everything;
    let mut app = covered_app(board);
    enter(&mut app, 10);

    let buf = render_to_buffer(&mut app, 160, 30);

    let title = app.board.find_task(TaskId(1)).unwrap().title.clone();
    assert!(buffer_contains(&buf, &title), "the card is drawn");
    assert!(!buffer_contains(&buf, "\u{2713} ready"));
}

// ---- notifications ----

fn notifications(cmds: &[Command]) -> Vec<(String, String)> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::System(SystemCommand::SendNotification { title, body, .. }) => {
                Some((title.clone(), body.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn an_item_notifies_once_on_its_own_machine_and_the_first_reload_is_the_baseline() {
    let mut board = on_ten(false);
    board.open_items = vec![item(1, 10, HERE, None)];
    let mut app = covered_app(board.clone());
    app.set_notifications_enabled(true);

    board.open_items.push(item(2, 10, HERE, Some(1)));
    board.open_items.push(item(3, 10, "elsewhere", None));
    let cmds = press(
        &mut app,
        EpicMessage::OrchestrationRefreshed(Box::new(board.clone())),
    );
    let sent = notifications(&cmds);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].0.contains("Payments"), "{sent:?}");
    assert!(sent[0].1.contains("question 2"), "{sent:?}");

    let cmds = press(
        &mut app,
        EpicMessage::OrchestrationRefreshed(Box::new(board)),
    );
    assert!(notifications(&cmds).is_empty(), "an item notifies once");
}

#[test]
fn nothing_notifies_with_notifications_off() {
    let mut app = covered_app(on_ten(false));
    let mut board = on_ten(false);
    board.open_items = vec![item(2, 10, HERE, None)];

    let cmds = press(
        &mut app,
        EpicMessage::OrchestrationRefreshed(Box::new(board)),
    );
    assert!(notifications(&cmds).is_empty());
    assert!(press(&mut app, EpicMessage::EpicFinished(EpicId(10))).is_empty());
}

#[test]
fn a_finished_epic_notifies() {
    let mut app = covered_app(on_ten(false));
    app.set_notifications_enabled(true);

    let cmds = press(&mut app, EpicMessage::EpicFinished(EpicId(10)));

    assert_eq!(
        notifications(&cmds),
        vec![("Payments".to_string(), "The epic is finished".to_string())]
    );
}

// ---- the orchestrator key ----

#[test]
fn the_orchestrator_key_outside_an_epic_view_and_a_covered_task_says_where_it_works() {
    let mut app = covered_app(OrchestrationBoard::default());

    let cmds = press(&mut app, EpicMessage::OpenOrchestrator);

    assert!(cmds.is_empty());
    assert_eq!(
        app.status.message.as_deref(),
        Some(crate::tui::update::orchestrators::OPEN_ORCHESTRATOR_NOWHERE)
    );
}

#[test]
fn in_an_epic_nothing_orchestrates_it_asks_and_y_switches_one_on() {
    let mut app = covered_app(OrchestrationBoard::default());
    enter(&mut app, 10);

    press(&mut app, EpicMessage::OpenOrchestrator);
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmStartOrchestrator(EpicId(10))
    );
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or_default()
        .starts_with("Start an orchestrator for this epic?"));

    let cmds = app.handle_key_confirm_start_orchestrator("y", true, EpicId(10));
    assert!(cmds.iter().any(
        |c| matches!(c, Command::Epic(EpicCommand::SwitchOrchestratorOn(e)) if *e == EpicId(10))
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn a_live_orchestrator_opens_its_window() {
    let mut app = covered_app(on_ten(true));
    enter(&mut app, 10);

    let cmds = press(&mut app, EpicMessage::OpenOrchestrator);

    assert!(
        matches!(cmds.as_slice(), [Command::Task(TaskCommand::JumpToTmux { window })] if *window == TmuxWindow::for_orchestrator(EpicId(10))),
        "{cmds:?}"
    );
}

#[test]
fn an_orchestrator_with_no_session_is_restarted_from_a_covered_task_card() {
    let mut app = covered_app(on_ten(false));
    select_task(&mut app, 1);

    let cmds = press(&mut app, EpicMessage::OpenOrchestrator);

    assert!(
        matches!(cmds.as_slice(), [Command::Epic(EpicCommand::RestartOrchestrator { epic, id: 1000 })] if *epic == EpicId(10)),
        "{cmds:?}"
    );
}

#[test]
fn an_orchestrator_whose_epic_has_no_open_work_rests() {
    let mut app = covered_app(on_ten(false));
    for t in &mut app.board.tasks {
        t.status = TaskStatus::Done;
    }
    enter(&mut app, 10);

    let cmds = press(&mut app, EpicMessage::OpenOrchestrator);

    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("rests"));
}

#[test]
fn a_view_above_one_orchestrator_opens_it_and_above_several_names_them() {
    let mut board = OrchestrationBoard {
        orchestrations: vec![switch(11)],
        orchestrators: vec![orchestrator(11, HERE, true)],
        ..OrchestrationBoard::default()
    };
    let mut app = covered_app(board.clone());
    let child = |id: i64, title: &str| {
        let mut e = make_epic_with_title(id, title);
        e.parent_epic_id = Some(EpicId(10));
        e
    };
    app.board.epics.push(child(11, "Card flow"));
    app.board.epics.push(child(12, "Refunds"));
    enter(&mut app, 10);

    let cmds = press(&mut app, EpicMessage::OpenOrchestrator);
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::Task(TaskCommand::JumpToTmux { .. })]
        ),
        "{cmds:?}"
    );

    board.orchestrations.push(switch(12));
    press(
        &mut app,
        EpicMessage::OrchestrationRefreshed(Box::new(board)),
    );
    let cmds = press(&mut app, EpicMessage::OpenOrchestrator);
    assert!(cmds.is_empty(), "{cmds:?}");
    let said = app.status.message.clone().unwrap_or_default();
    assert!(
        said.contains("Card flow") && said.contains("Refunds"),
        "{said}"
    );
}

// ---- the resolve key ----

#[test]
fn the_resolve_key_asks_about_the_oldest_open_item_and_y_resolves_it() {
    let mut board = on_ten(false);
    board.open_items = vec![item(5, 10, HERE, Some(1)), item(7, 10, HERE, None)];
    let mut app = covered_app(board);
    enter(&mut app, 10);

    press(&mut app, EpicMessage::ResolveItemKey);
    assert_eq!(app.input.mode, InputMode::ConfirmResolveItem(5));
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or_default()
        .starts_with("Resolve item #5: question 5?"));

    let cmds = app.handle_key_confirm_resolve_item("y", true, 5);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Epic(EpicCommand::ResolveWaitingItem(5)))));

    press(&mut app, EpicMessage::ResolveItemKey);
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmResolveItem(7),
        "a second press asks for the next"
    );
}

#[test]
fn the_resolve_key_with_nothing_open_says_so() {
    let mut app = covered_app(on_ten(false));
    enter(&mut app, 10);

    press(&mut app, EpicMessage::ResolveItemKey);

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("Nothing waits for you"));
}

// ---- the header ----

#[test]
fn the_header_shows_the_orchestrator_its_items_and_its_log() {
    let mut board = on_ten(true);
    board.orchestrations[0].start_rule = StartRule::Everything;
    board.open_items = vec![item(5, 10, HERE, Some(1))];
    board.log = vec![entry(9, 10, "started the payments task")];
    let mut app = covered_app(board);
    enter(&mut app, 10);

    let buf = render_to_buffer(&mut app, 160, 40);

    assert!(buffer_contains(&buf, "orchestrator orch10"));
    assert!(buffer_contains(&buf, "everything"));
    assert!(buffer_contains(&buf, "/3 running"));
    assert!(buffer_contains(&buf, "1 waiting"));
    assert!(
        buffer_contains(&buf, "#5 #1: question 5"),
        "a task with no agent shows by id"
    );
    assert!(buffer_contains(&buf, "started the payments task"));
}

#[test]
fn the_header_names_a_task_by_its_agent() {
    let mut board = on_ten(true);
    board.open_items = vec![item(5, 10, HERE, Some(1))];
    let mut app = covered_app(board);
    app.board.tasks[0].agent_name = Some("ada".to_string());
    enter(&mut app, 10);

    let buf = render_to_buffer(&mut app, 160, 40);

    assert!(buffer_contains(&buf, "#5 ada: question 5"));
}

#[test]
fn the_header_is_cut_to_a_third_of_the_board_with_a_more_row() {
    let mut board = on_ten(true);
    board.log = (1..=10)
        .map(|i| entry(i, 10, &format!("entry {i}")))
        .collect();
    let mut app = covered_app(board);
    enter(&mut app, 10);

    // 15 rows leave the board 12, so the header takes at most 4: its
    // first row, two log rows and "+8 more".
    let buf = render_to_buffer(&mut app, 120, 15);

    assert!(buffer_contains(&buf, "entry 10"));
    assert!(buffer_contains(&buf, "entry 9"));
    assert!(!buffer_contains(&buf, "entry 8"));
    assert!(buffer_contains(&buf, "+8 more"));
}

#[test]
fn the_header_stays_under_an_open_task_detail() {
    let mut app = covered_app(on_ten(true));
    select_task(&mut app, 1);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    assert!(matches!(app.board.view_mode, ViewMode::TaskDetail { .. }));

    let buf = render_to_buffer(&mut app, 160, 40);

    assert!(buffer_contains(&buf, "orchestrator orch10"));
}

#[test]
fn a_short_terminal_keeps_the_columns_their_six_rows() {
    let mut board = on_ten(true);
    board.log = (1..=10)
        .map(|i| entry(i, 10, &format!("entry {i}")))
        .collect();
    let mut app = covered_app(board);
    enter(&mut app, 10);

    // 10 rows leave the board 7: the header gets one, the columns six.
    let buf = render_to_buffer(&mut app, 120, 10);

    assert!(buffer_contains(&buf, "orchestrator orch10"));
    assert!(!buffer_contains(&buf, "entry 10"));
}

#[test]
fn a_reload_with_new_log_entries_and_no_new_item_notifies_nothing() {
    let mut app = covered_app(on_ten(true));
    app.set_notifications_enabled(true);
    let mut board = on_ten(true);
    board.log = vec![entry(1, 10, "answered a question")];

    let cmds = press(
        &mut app,
        EpicMessage::OrchestrationRefreshed(Box::new(board)),
    );

    assert!(notifications(&cmds).is_empty(), "{cmds:?}");
}

#[test]
fn no_header_in_an_epic_no_orchestrator_covers() {
    let mut app = covered_app(OrchestrationBoard::default());
    enter(&mut app, 10);

    let buf = render_to_buffer(&mut app, 160, 30);

    assert!(!buffer_contains(&buf, "orchestrator"));
}

#[test]
fn a_sub_epics_view_shows_the_orchestrator_above_it() {
    let mut app = covered_app(on_ten(true));
    let mut sub = make_epic_with_title(11, "Card flow");
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics.push(sub);
    enter(&mut app, 11);

    let buf = render_to_buffer(&mut app, 160, 30);

    assert!(buffer_contains(&buf, "orchestrator orch10"));
    assert!(buffer_contains(&buf, "(from Payments)"));
}
