//! Epic cards are placed per column, not once.
//!
//! Obligations from `docs/specs/board-layout.allium`, "Epic Card Placement"
//! and `FlattenedView`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{ColumnSection, EpicId, SubStatus, TaskId, TaskStatus};
use crate::tui::types::RepoFilterMode;

/// The columns whose card list holds `epic_id`, in board order.
fn epic_columns(app: &App, epic_id: i64) -> Vec<TaskStatus> {
    TaskStatus::ALL
        .iter()
        .copied()
        .filter(|&status| {
            app.column_items_for_status(status)
                .iter()
                .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(epic_id)))
        })
        .collect()
}

/// A task owned by `epic_id`, in `status`.
fn epic_task(id: i64, epic_id: i64, status: TaskStatus) -> crate::models::Task {
    crate::models::Task {
        epic_id: Some(EpicId(epic_id)),
        ..make_task(id, status)
    }
}

/// One epic (#1) whose four subtasks sit one in each column.
fn app_with_epic_spread_over_all_columns() -> App {
    let mut app = App::new(vec![
        epic_task(1, 1, TaskStatus::Backlog),
        epic_task(2, 1, TaskStatus::Running),
        epic_task(3, 1, TaskStatus::Review),
        epic_task(4, 1, TaskStatus::Done),
    ]);
    app.board.epics.push(make_epic(1));
    app
}

#[test]
fn epic_card_appears_in_every_column_its_subtree_has_work() {
    let app = app_with_epic_spread_over_all_columns();
    assert_eq!(
        epic_columns(&app, 1),
        vec![
            TaskStatus::Backlog,
            TaskStatus::Running,
            TaskStatus::Review,
            TaskStatus::Done
        ]
    );
}

#[test]
fn epic_card_is_absent_from_columns_with_no_work() {
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Running)]);
    app.board.epics.push(make_epic(1));

    assert_eq!(epic_columns(&app, 1), vec![TaskStatus::Running]);
}

#[test]
fn a_sub_epics_task_places_the_parent_epics_card() {
    // Parent #1 owns no task directly; its only work sits in sub-epic #2.
    let mut app = App::new(vec![epic_task(1, 2, TaskStatus::Review)]);
    let mut sub = make_epic(2);
    sub.parent_epic_id = Some(EpicId(1));
    app.board.epics.push(make_epic(1));
    app.board.epics.push(sub);

    assert_eq!(epic_columns(&app, 1), vec![TaskStatus::Review]);
}

#[test]
fn the_epics_own_recorded_status_places_no_card() {
    // Recorded status says Done; every task says Backlog. The tasks decide.
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Backlog)]);
    let mut epic = make_epic(1);
    epic.status = TaskStatus::Done;
    app.board.epics.push(epic);

    assert_eq!(epic_columns(&app, 1), vec![TaskStatus::Backlog]);
}

#[test]
fn an_epic_with_no_tasks_falls_back_to_backlog() {
    for status in [
        TaskStatus::Backlog,
        TaskStatus::Running,
        TaskStatus::Review,
        TaskStatus::Done,
    ] {
        let mut app = App::new(vec![]);
        let mut epic = make_epic(1);
        epic.status = status;
        app.board.epics.push(epic);

        assert_eq!(
            epic_columns(&app, 1),
            vec![TaskStatus::Backlog],
            "recorded status {status:?}"
        );
    }
}

#[test]
fn a_column_the_repo_filter_emptied_loses_the_epic_card() {
    let mut app = App::new(vec![]);
    let mut backlog = epic_task(1, 1, TaskStatus::Backlog);
    backlog.repo_path = "/repo-a".to_string();
    let mut running = epic_task(2, 1, TaskStatus::Running);
    running.repo_path = "/repo-b".to_string();
    app.board.tasks = vec![backlog, running];
    app.board.epics.push(make_epic(1));

    assert_eq!(
        epic_columns(&app, 1),
        vec![TaskStatus::Backlog, TaskStatus::Running],
        "unfiltered"
    );

    app.filter.repos.insert("/repo-b".to_string());
    app.filter.mode = RepoFilterMode::Exclude;

    assert_eq!(
        epic_columns(&app, 1),
        vec![TaskStatus::Backlog],
        "the excluded repo's running task no longer places a card"
    );
}

#[test]
fn a_column_the_only_active_filter_emptied_loses_the_epic_card() {
    // The Review task is unprovisioned, so only-active drops it; the Running
    // one is provisioned and keeps both the epic and its own column alive.
    let mut app = App::new(vec![]);
    app.board.tasks = vec![
        epic_task(1, 1, TaskStatus::Running),
        crate::models::Task {
            epic_id: Some(EpicId(1)),
            ..make_unprovisioned_task(2, TaskStatus::Review)
        },
    ];
    app.board.epics.push(make_epic(1));

    assert_eq!(
        epic_columns(&app, 1),
        vec![TaskStatus::Running, TaskStatus::Review],
        "unfiltered"
    );

    app.filter.only_active = true;

    assert_eq!(
        epic_columns(&app, 1),
        vec![TaskStatus::Running],
        "the dropped review task no longer places a card"
    );
}

#[test]
fn an_epic_matched_by_title_alone_falls_back_to_backlog() {
    // The query matches the epic's own title but none of its tasks, so the
    // epic card is visible with no task placing it.
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Done)]);
    app.board.epics.push(make_epic_with_title(1, "Roadmap"));
    app.search.query = "Roadmap".to_string();

    assert!(
        visible_epic_ids(&app).contains(&1),
        "the epic is surfaced by its own title"
    );
    assert_eq!(epic_columns(&app, 1), vec![TaskStatus::Backlog]);
}

/// The section a copy lands in comes from that column's own tasks.
#[test]
fn each_copy_takes_its_section_from_its_own_columns_tasks() {
    let mut blocked = epic_task(1, 1, TaskStatus::Running);
    blocked.sub_status = SubStatus::NeedsInput;
    let review = epic_task(2, 1, TaskStatus::Review);
    let mut app = App::new(vec![blocked, review]);
    app.board.epics.push(make_epic(1));

    assert_eq!(
        epic_section(&app, TaskStatus::Running, 1),
        Some(ColumnSection::NeedsInput),
        "the Running copy follows its blocked running task"
    );
    assert_eq!(
        epic_section(&app, TaskStatus::Review, 1),
        Some(ColumnSection::AwaitingReview),
        "the Review copy is unaffected by the blocked task in Running"
    );
}

#[test]
fn a_running_copy_with_no_blocked_task_sits_in_active() {
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Running)]);
    app.board.epics.push(make_epic(1));

    assert_eq!(
        epic_section(&app, TaskStatus::Running, 1),
        Some(ColumnSection::Active)
    );
}

/// The section header the epic card for `epic_id` renders under in `status`.
fn epic_section(app: &App, status: TaskStatus, epic_id: i64) -> Option<ColumnSection> {
    let items = app.column_items_for_status(status);
    let mut current: Option<ColumnSection> = None;
    for item in &items {
        match item {
            ColumnItem::SubstatusLabel(at) => current = Some(at.section),
            ColumnItem::Epic(e) if e.id == EpicId(epic_id) => return current,
            _ => {}
        }
    }
    None
}

#[test]
fn sub_epic_cards_inside_an_epic_view_are_placed_per_column() {
    let mut app = App::new(vec![
        epic_task(1, 2, TaskStatus::Running),
        epic_task(2, 2, TaskStatus::Done),
    ]);
    let mut sub = make_epic(2);
    sub.parent_epic_id = Some(EpicId(1));
    app.board.epics.push(make_epic(1));
    app.board.epics.push(sub);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(1),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    assert_eq!(
        epic_columns(&app, 2),
        vec![TaskStatus::Running, TaskStatus::Done]
    );
}

// --- Flattened mode: Done now flattens too ---------------------------------

#[test]
fn flattened_done_column_surfaces_descendant_tasks_and_drops_the_epic_card() {
    let mut app = app_with_epic_spread_over_all_columns();
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Done);
    assert!(
        items
            .iter()
            .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(4))),
        "the epic's done task surfaces as its own card"
    );
    assert!(
        !items.iter().any(|i| matches!(i, ColumnItem::Epic(_))),
        "a flattened column draws no epic card"
    );
}

#[test]
fn flattened_backlog_keeps_its_epic_cards() {
    let mut app = app_with_epic_spread_over_all_columns();
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(
        items
            .iter()
            .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1))),
        "Backlog is the one unflattened column"
    );
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(1))),
        "its epic-owned task stays inside the epic card"
    );
}

#[test]
fn flattened_mode_draws_no_card_for_an_epic_with_no_backlog_work() {
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Done)]);
    app.board.epics.push(make_epic(1));
    app.board.flattened = true;

    assert!(
        epic_columns(&app, 1).is_empty(),
        "every column the epic has work in is flattened, so no card is drawn"
    );
}

/// `TaskStatus::column_index()` answers one past the end of the column array
/// for `Archived`, so both accessors must take that without panicking.
#[test]
fn archived_is_outside_the_placement_model() {
    let mut app = App::new(vec![]);
    let mut archived = epic_task(1, 1, TaskStatus::Backlog);
    archived.status = TaskStatus::Archived;
    app.board.tasks = vec![archived];
    app.board.epics.push(make_epic(1));

    let placements = app.compute_epic_placements();
    let placement = placements.get(&EpicId(1)).expect("epic has a placement");

    assert!(!placement.appears_in(TaskStatus::Archived));
    for status in [TaskStatus::Running, TaskStatus::Review, TaskStatus::Done] {
        assert!(
            !placement.appears_in(status),
            "an archived task places no card in {status:?}"
        );
    }
    assert_eq!(
        epic_columns(&app, 1),
        vec![TaskStatus::Backlog],
        "so the epic takes the empty-epic fallback"
    );
}

/// The placement cache decides which columns a card appears in, so a `&self`
/// reader must never be served one built before the last board or filter
/// change. `cached_epic_stats()` is where the cache self-heals, and a `&self`
/// caller cannot reach it — so the staleness check lives in the reader.
#[test]
fn a_stale_placement_cache_is_not_served() {
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Running)]);
    app.board.epics.push(make_epic(1));

    // Warm the cache, then move the task without invalidating.
    let _ = app.cached_epic_stats();
    assert!(app.cached_placements().is_some(), "cache is warm");
    app.board.tasks[0].status = TaskStatus::Review;

    assert!(
        app.cached_placements().is_none(),
        "a board change must make the warm map unreadable"
    );
    assert_eq!(
        epic_columns(&app, 1),
        vec![TaskStatus::Review],
        "so readers see the card where it actually belongs"
    );
}

/// The same hazard for a filter change, which the board fingerprint did not
/// cover until placement needed it to.
#[test]
fn a_search_query_change_invalidates_the_placement_cache() {
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Running)]);
    app.board.epics.push(make_epic_with_title(1, "Roadmap"));

    let _ = app.cached_epic_stats();
    assert!(app.cached_placements().is_some(), "cache is warm");

    // The query matches the epic's title but not its task, so placement falls
    // back to Backlog — a different answer from the cached one.
    app.search.query = "Roadmap".to_string();

    assert!(
        app.cached_placements().is_none(),
        "a query change must make the warm map unreadable"
    );
    assert_eq!(epic_columns(&app, 1), vec![TaskStatus::Backlog]);
}

/// Same again for the only-active filter, the other input the fingerprint gained.
#[test]
fn an_only_active_change_invalidates_the_placement_cache() {
    let mut app = App::new(vec![]);
    app.board.tasks = vec![crate::models::Task {
        epic_id: Some(EpicId(1)),
        ..make_unprovisioned_task(1, TaskStatus::Running)
    }];
    app.board.epics.push(make_epic(1));

    let _ = app.cached_epic_stats();
    assert!(app.cached_placements().is_some(), "cache is warm");

    app.filter.only_active = true;

    assert!(
        app.cached_placements().is_none(),
        "a filter change must make the warm map unreadable"
    );
}
