//! Ordering of the Done column: newest completion first.
//!
//! Obligations from `docs/specs/board-layout.allium`, "Done Column Ordering".
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{EpicId, TaskStatus};

/// A done task ranked by `sort_order` — the negated-millisecond completion
/// rank every transition into Done stamps, so a *lower* value is more recent.
fn done_at(id: i64, epic_id: Option<i64>, rank: i64) -> crate::models::Task {
    crate::models::Task {
        epic_id: epic_id.map(EpicId),
        sort_order: Some(rank),
        ..make_task(id, TaskStatus::Done)
    }
}

/// The Done column's cards, in render order, as `("task"|"epic", id)`.
fn done_cards(app: &App) -> Vec<(&'static str, i64)> {
    app.column_items_for_status(TaskStatus::Done)
        .into_iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(("task", t.id.0)),
            ColumnItem::Epic(e) => Some(("epic", e.id.0)),
            ColumnItem::EpicHeader(e) => Some(("header", e.id.0)),
            _ => None,
        })
        .collect()
}

/// The baseline the epic rule has to match: plain task cards already sort by
/// completion rank, most recent first.
#[test]
fn done_task_cards_sort_most_recently_completed_first() {
    let app = App::new(vec![
        done_at(1, None, -100),
        done_at(2, None, -300),
        done_at(3, None, -200),
    ]);
    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)]
    );
}

/// The bug. Epic #10 is still Backlog, so it carries no rank of its own, but
/// its subtask finished more recently than any loose card. Sorting it by the
/// generic `sort_order ?? id` fallback sank it below every ranked card.
#[test]
fn an_unranked_epic_card_sorts_by_its_newest_done_subtask() {
    let mut app = App::new(vec![
        done_at(1, None, -100),
        done_at(2, None, -400),
        done_at(3, Some(10), -300),
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("epic", 10), ("task", 1)],
        "epic #10 sorts by subtask #3's rank (-300), between -400 and -100"
    );
}

/// The key comes from the column's own slice of the subtree. Work the Done
/// column is not showing must not decide where the card sits in it.
#[test]
fn only_done_subtasks_feed_the_epics_done_key() {
    let mut app = App::new(vec![
        done_at(1, None, -200),
        done_at(2, Some(10), -100),
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            sort_order: Some(-900),
            ..make_task(3, TaskStatus::Running)
        },
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "the Running task's -900 must not lift the epic above task #1"
    );
}

/// A sub-epic's work places the parent's card, so it must also rank it.
#[test]
fn a_sub_epics_done_task_ranks_the_parent_epic_card() {
    let mut app = App::new(vec![done_at(1, None, -100), done_at(2, Some(11), -300)]);
    let mut sub = make_epic(11);
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics.push(make_epic(10));
    app.board.epics.push(sub);

    let cards = done_cards(&app);
    let parent = cards.iter().position(|c| *c == ("epic", 10)).unwrap();
    let loose = cards.iter().position(|c| *c == ("task", 1)).unwrap();
    assert!(
        parent < loose,
        "parent epic #10 must outrank the older loose task: {cards:?}"
    );
}

/// An epic whose Done slice holds no ranked task falls back to its own key —
/// the pre-rank legacy row, and nothing else.
#[test]
fn an_epic_with_no_ranked_done_subtask_falls_back_to_its_own_key() {
    let mut app = App::new(vec![
        done_at(1, None, -100),
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            sort_order: None,
            ..make_task(2, TaskStatus::Done)
        },
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "epic #10 falls back to its id (10), which sorts after every rank"
    );
}

/// Done only. Review keeps the generic key, so an epic placed there by a task
/// carrying a stray rank must not be reordered by it.
#[test]
fn the_done_key_does_not_reach_other_columns() {
    let mut app = App::new(vec![
        crate::models::Task {
            sort_order: Some(-500),
            ..make_task(1, TaskStatus::Backlog)
        },
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            sort_order: Some(-900),
            ..make_task(2, TaskStatus::Backlog)
        },
    ]);
    app.board.epics.push(make_epic(10));

    let backlog: Vec<i64> = app
        .column_items_for_status(TaskStatus::Backlog)
        .into_iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(t.id.0),
            ColumnItem::Epic(e) => Some(e.id.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        backlog,
        vec![1, 10],
        "Backlog still orders the epic by its own key (id 10), not by task #2"
    );
}

/// Flattened Done draws no epic card, and no epic header either — Done has no
/// sections, and the header is emitted inside a section run. The epic grouping
/// survives as card *order* alone, so the group carrying the freshest task must
/// lead, with its own tasks newest-first inside it.
#[test]
fn flattened_done_groups_sort_by_their_newest_task() {
    let mut app = App::new(vec![
        done_at(1, Some(10), -100),
        done_at(2, Some(11), -300),
        done_at(3, Some(10), -200),
    ]);
    app.board.epics.push(make_epic(10));
    app.board.epics.push(make_epic(11));
    app.board.flattened = true;

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)],
        "epic #11's group leads on -300; within #10, -200 leads -100"
    );
}

/// The flattened key is the DIRECT epic's, not the subtree's: a sub-epic's
/// tasks are their own group, so they must not lift the parent's group.
#[test]
fn a_flattened_group_is_keyed_on_the_direct_epic_not_the_subtree() {
    // Sub-epic #11 under parent #10, plus unrelated epic #12. Keyed on the
    // direct epic the groups are 11 (-400), 12 (-200), 10 (-100). Keyed on the
    // subtree, #10 would inherit #11's -400 and lead instead.
    let mut app = App::new(vec![
        done_at(1, Some(10), -100),
        done_at(2, Some(11), -400),
        done_at(3, Some(12), -200),
    ]);
    let mut sub = make_epic(11);
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics.push(make_epic(10));
    app.board.epics.push(sub);
    app.board.epics.push(make_epic(12));
    app.board.flattened = true;

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)],
        "epic #10's group keeps its own -100; #11's -400 does not lift it"
    );
}

/// An epicless task in a flattened Done column ranks with the epic groups
/// rather than sinking below them. Done draws no OrphanSeparator, so the
/// orphans-last rule has nothing to separate and only hides fresh work.
#[test]
fn a_flattened_done_orphan_ranks_by_its_own_completion() {
    let mut app = App::new(vec![
        done_at(1, Some(10), -100),
        done_at(2, None, -300),
        done_at(3, Some(10), -200),
    ]);
    app.board.epics.push(make_epic(10));
    app.board.flattened = true;

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)],
        "the epicless #2 leads on -300 instead of sinking to the bottom"
    );
}

/// The orphans-last rule still holds everywhere it draws a separator.
#[test]
fn a_flattened_orphan_still_sorts_last_outside_done() {
    let mut app = App::new(vec![
        crate::models::Task {
            sort_order: Some(-300),
            ..make_task(1, TaskStatus::Running)
        },
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            sort_order: Some(-100),
            ..make_task(2, TaskStatus::Running)
        },
    ]);
    app.board.epics.push(make_epic(10));
    app.board.flattened = true;

    let running: Vec<i64> = app
        .column_items_for_status(TaskStatus::Running)
        .into_iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(t.id.0),
            _ => None,
        })
        .collect();
    assert_eq!(running, vec![2, 1], "the epicless #1 still sorts last");
}

/// A task created straight into Done never took a rank, so it reaches the
/// epic's own-key fallback exactly as a pre-rank row does.
#[test]
fn a_done_task_that_never_took_a_rank_reaches_the_fallback() {
    let mut app = App::new(vec![
        done_at(1, None, -100),
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            sort_order: None,
            ..make_task(2, TaskStatus::Done)
        },
        done_at(3, Some(10), -50),
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "the unranked task is ignored; #10 takes its only rank, -50"
    );
}

/// An epic card in Done owns no field the column reads, so reordering it by
/// hand is refused rather than writing a `sort_order` nothing renders.
#[test]
fn reordering_an_epic_card_in_done_is_refused() {
    let mut app = App::new(vec![done_at(1, None, -100), done_at(2, Some(10), -300)]);
    app.board.epics.push(make_epic(10));

    let done_col = TaskStatus::Done.column_index() + 1;
    app.selection_mut().set_column(done_col);
    app.selection_mut().set_row(done_col, 0);
    assert!(
        matches!(
            app.column_items_for_status(TaskStatus::Done).first(),
            Some(ColumnItem::Epic(e)) if e.id == EpicId(10)
        ),
        "the epic card must be the one under the cursor"
    );

    let cmds = app.handle_reorder_item(1);
    assert!(cmds.is_empty(), "no command: the reorder is refused");
    assert_eq!(
        app.board.epics[0].sort_order, None,
        "and nothing is written to the epic"
    );
}

/// A task card in Done still reorders: its own rank is the key, so the swap
/// moves it. The refusal above is about epic cards alone.
#[test]
fn reordering_a_task_card_in_done_still_works() {
    let mut app = App::new(vec![done_at(1, None, -300), done_at(2, None, -100)]);

    let done_col = TaskStatus::Done.column_index() + 1;
    app.selection_mut().set_column(done_col);
    app.selection_mut().set_row(done_col, 0);

    let cmds = app.handle_reorder_item(1);
    assert!(!cmds.is_empty(), "the swap is persisted");
    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 1)],
        "task #1 moved down past #2"
    );
}
