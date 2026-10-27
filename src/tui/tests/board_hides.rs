//! Hidden epics and work packages (`board-layout.allium`, "Hidden Epics and
//! Work Packages"; `work-packages.allium`, PackageCardPlacement).

use super::*;
use crate::models::{EpicId, TaskId, TaskStatus, WorkPackageId, WorkPackageItem};

fn sub_epic(id: i64, parent: i64) -> crate::models::Epic {
    crate::models::Epic {
        parent_epic_id: Some(EpicId(parent)),
        ..make_epic(id)
    }
}

/// Every card id drawn anywhere in the current view, by kind.
fn cards(app: &App) -> (Vec<EpicId>, Vec<TaskId>, Vec<WorkPackageId>) {
    let (mut epics, mut tasks, mut packages) = (vec![], vec![], vec![]);
    for status in TaskStatus::ALL {
        for item in app.view().column_items_for_status(*status) {
            match item {
                ColumnItem::Epic(e) => epics.push(e.id),
                ColumnItem::Task(t) => tasks.push(t.id),
                ColumnItem::WorkPackage(p) => packages.push(p.id),
                _ => {}
            }
        }
    }
    (epics, tasks, packages)
}

fn package_columns(app: &App, id: i64) -> Vec<TaskStatus> {
    TaskStatus::ALL
        .iter()
        .copied()
        .filter(|&status| {
            app.view()
                .column_items_for_status(status)
                .iter()
                .any(|i| matches!(i, ColumnItem::WorkPackage(p) if p.id == WorkPackageId(id)))
        })
        .collect()
}

/// Root epic 1 → sub-epic 2; task 10 under 1, task 20 under 2, standalone 30.
fn tree_app() -> App {
    let mut app = App::new(vec![
        epic_task(10, 1, TaskStatus::Running),
        epic_task(20, 2, TaskStatus::Running),
        make_task(30, TaskStatus::Running),
    ]);
    app.board.epics = vec![make_epic(1), sub_epic(2, 1)];
    app
}

#[test]
fn hiding_an_epic_removes_its_card() {
    let mut app = tree_app();
    assert!(cards(&app).0.contains(&EpicId(1)));
    app.filter.hidden_epics.insert(EpicId(1));
    assert!(!cards(&app).0.contains(&EpicId(1)));
}

#[test]
fn hiding_an_epic_hides_every_task_in_its_subtree_in_the_flat_lens() {
    let mut app = tree_app();
    app.filter.lens = app.filter.lens.with_flattened(true);
    assert!(cards(&app).1.contains(&TaskId(20)));

    app.filter.hidden_epics.insert(EpicId(1));
    let tasks = cards(&app).1;
    assert!(!tasks.contains(&TaskId(10)));
    assert!(
        !tasks.contains(&TaskId(20)),
        "a sub-epic's task is hidden too"
    );
    assert!(
        tasks.contains(&TaskId(30)),
        "a standalone task is untouched"
    );
}

#[test]
fn a_hidden_sub_epic_draws_no_card_inside_its_parents_view() {
    let mut app = tree_app();
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    assert!(cards(&app).0.contains(&EpicId(2)));

    app.filter.hidden_epics.insert(EpicId(2));
    let (epics, tasks, _) = cards(&app);
    assert!(!epics.contains(&EpicId(2)));
    assert!(tasks.contains(&TaskId(10)), "the parent's own task stays");
}

#[test]
fn a_hidden_childs_tasks_do_not_place_the_parents_card() {
    // The parent's only Running work sits in the hidden child, so its card
    // falls back to Backlog instead of claiming Running.
    let mut app = App::new(vec![epic_task(20, 2, TaskStatus::Running)]);
    app.board.epics = vec![make_epic(1), sub_epic(2, 1)];
    app.filter.hidden_epics.insert(EpicId(2));

    let running = app.view().column_items_for_status(TaskStatus::Running);
    assert!(!running
        .iter()
        .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1))));
    let backlog = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(backlog
        .iter()
        .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1))));
}

#[test]
fn a_search_cannot_bring_back_a_hidden_epic() {
    let mut app = tree_app();
    app.board.epics[0].title = "needle".into();
    app.search.query = "needle".into();
    assert!(cards(&app).0.contains(&EpicId(1)));

    app.filter.hidden_epics.insert(EpicId(1));
    assert!(!cards(&app).0.contains(&EpicId(1)));
}

#[test]
fn a_task_a_visible_package_links_stays_hidden_under_a_hidden_epic() {
    let mut app = tree_app();
    app.filter.lens = app.filter.lens.with_flattened(true);
    app.board.work_packages = vec![make_package(7, "P7")];
    app.board.work_package_links = vec![link(7, WorkPackageItem::Task(TaskId(10)))];
    app.filter.hidden_epics.insert(EpicId(1));

    assert!(!app
        .view()
        .tasks_for_current_view()
        .iter()
        .any(|t| t.id == TaskId(10)));
    // The hidden task still places the card: it is Running, so Backlog.
    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Backlog]);
}

#[test]
fn hiding_a_finished_packages_only_epic_leaves_it_in_done() {
    let mut app = App::new(vec![epic_task(10, 1, TaskStatus::Done)]);
    app.board.epics = vec![make_epic(1)];
    app.board.work_packages = vec![make_package(7, "P7")];
    app.board.work_package_links = vec![link(7, WorkPackageItem::Epic(EpicId(1)))];
    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Done]);

    app.filter.hidden_epics.insert(EpicId(1));
    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Done]);
}

#[test]
fn hiding_a_package_removes_its_card_and_nothing_else() {
    let mut app = tree_app();
    app.board.work_packages = vec![make_package(7, "P7"), make_package(8, "P8")];
    app.board.work_package_links = vec![link(7, WorkPackageItem::Epic(EpicId(1)))];

    app.filter.hidden_packages.insert(WorkPackageId(7));
    let (epics, _, packages) = cards(&app);
    assert_eq!(packages, vec![WorkPackageId(8)]);
    assert!(
        epics.contains(&EpicId(1)),
        "the linked epic stays on the board"
    );
}

#[test]
fn a_hidden_id_that_names_nothing_is_inert() {
    let mut app = tree_app();
    let before = cards(&app);
    app.filter.hidden_epics.insert(EpicId(999));
    app.filter.hidden_packages.insert(WorkPackageId(999));
    assert_eq!(cards(&app), before);
}

#[test]
fn hiding_an_epic_changes_the_layout_fingerprint() {
    let mut app = tree_app();
    let before = app.view().compute_layout_fingerprint();
    app.filter.hidden_epics.insert(EpicId(1));
    let after_epic = app.view().compute_layout_fingerprint();
    assert_ne!(before, after_epic);
    app.filter.hidden_packages.insert(WorkPackageId(7));
    assert_ne!(after_epic, app.view().compute_layout_fingerprint());
}

#[test]
fn closing_the_filter_overlay_persists_both_hide_lists_sorted() {
    use crate::tui::commands::SettingsCommand;
    let mut app = tree_app();
    app.filter.hidden_epics.extend([EpicId(9), EpicId(2)]);
    app.filter.hidden_packages.insert(WorkPackageId(4));
    app.input.mode = InputMode::BoardFilter;
    let cmds = app.update(Message::BoardFilter(
        crate::tui::messages::BoardFilterMessage::Close,
    ));
    let saved = |want: &str| {
        cmds.iter().find_map(|c| match c {
            Command::Settings(SettingsCommand::PersistStringSetting { key, value })
                if key == want =>
            {
                Some(value.clone())
            }
            _ => None,
        })
    };
    assert_eq!(saved("hidden_epics").as_deref(), Some("[2,9]"));
    assert_eq!(saved("hidden_packages").as_deref(), Some("[4]"));
}

#[test]
fn the_status_bar_names_what_is_hidden() {
    let mut app = tree_app();
    app.board.work_packages = vec![make_package(7, "P7")];
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "[hidden"),
        "nothing hidden, no badge"
    );

    app.filter.hidden_epics.extend([EpicId(1), EpicId(2)]);
    app.filter.hidden_packages.insert(WorkPackageId(7));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[hidden: 2 epics, 1 package]"));

    app.filter.hidden_epics.remove(&EpicId(2));
    app.filter.hidden_packages.clear();
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(buffer_contains(&buf, "[hidden: 1 epic]"));
}

#[test]
fn a_hidden_sub_epic_cannot_keep_its_parent_alive_under_a_search() {
    let mut app = tree_app();
    app.board.epics[1].title = "needle".into();
    app.search.query = "needle".into();
    assert!(
        cards(&app).0.contains(&EpicId(1)),
        "the child's match keeps the parent"
    );

    app.filter.hidden_epics.insert(EpicId(2));
    assert!(!cards(&app).0.contains(&EpicId(1)));
}

#[test]
fn the_badge_counts_only_hides_that_name_something() {
    let mut app = tree_app();
    app.filter.hidden_epics.insert(EpicId(999));
    app.filter.hidden_packages.insert(WorkPackageId(999));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(!buffer_contains(&buf, "[hidden"));
}

#[test]
fn closing_the_overlay_leaves_an_epic_view_the_user_just_hid() {
    let mut app = tree_app();
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(2),
    )));
    app.update(Message::BoardFilter(
        crate::tui::messages::BoardFilterMessage::Start,
    ));
    app.filter.hidden_epics.insert(EpicId(1));
    app.update(Message::BoardFilter(
        crate::tui::messages::BoardFilterMessage::Close,
    ));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn a_live_task_under_a_hidden_child_does_not_keep_the_parent_under_the_active_lens() {
    let mut live = epic_task(20, 2, TaskStatus::Running);
    live.tmux_window = Some(crate::models::test_tmux_window("task-20"));
    let mut app = App::new(vec![live]);
    app.board.epics = vec![make_epic(1), sub_epic(2, 1)];
    app.filter.lens = crate::tui::types::Lens::Active;
    assert!(cards(&app).0.contains(&EpicId(1)));

    app.filter.hidden_epics.insert(EpicId(2));
    app.invalidate_layout_cache();
    let _ = app.cached_epic_stats();
    assert!(!cards(&app).0.contains(&EpicId(1)), "cached path");
    app.invalidate_layout_cache();
    assert!(!cards(&app).0.contains(&EpicId(1)), "uncached path");
}

#[test]
fn space_that_drops_a_row_keeps_the_epic_cursor_on_the_list() {
    let mut app = tree_app();
    app.board.epics.push(crate::models::Epic {
        status: TaskStatus::Done,
        ..make_epic(3)
    });
    app.filter.hidden_epics.insert(EpicId(3));
    app.update(Message::BoardFilter(
        crate::tui::messages::BoardFilterMessage::Start,
    ));
    app.filter_overlay.tab = crate::tui::types::FilterTab::Epics;
    app.filter_overlay.epic_cursor = 2; // the hidden done epic, last row
    app.update(Message::BoardFilter(
        crate::tui::messages::BoardFilterMessage::ToggleEpic(EpicId(3)),
    ));
    assert_eq!(app.filter_epic_rows().len(), 2);
    assert_eq!(app.filter_overlay.epic_cursor, 1);
}
