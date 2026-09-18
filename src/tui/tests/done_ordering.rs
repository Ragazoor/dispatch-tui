//! Ordering of the Done column: newest completion first.
//!
//! Obligations from `docs/specs/board-layout.allium`, "Done Column Ordering".
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{EpicId, TaskStatus};
use chrono::{DateTime, Utc};

/// A completion time, `seconds` past an arbitrary epoch. Larger is more recent,
/// and the Done column reads newest-first.
fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + seconds, 0).unwrap()
}

/// A done task stamped with a completion time.
fn done_at(id: i64, epic_id: Option<i64>, seconds: i64) -> crate::models::Task {
    crate::models::Task {
        epic_id: epic_id.map(EpicId),
        completed_at: Some(at(seconds)),
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

/// The baseline the epic rule has to match: plain task cards sort by
/// completion time, most recent first.
#[test]
fn done_task_cards_sort_most_recently_completed_first() {
    let app = App::new(vec![
        done_at(1, None, 100),
        done_at(2, None, 300),
        done_at(3, None, 200),
    ]);
    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)]
    );
}

/// Epic #10 is still Backlog, so it carries no completion of its own, but its
/// subtask finished more recently than one of the loose cards. It must sort by
/// that subtask, not sink below every dated card.
#[test]
fn an_undated_epic_card_sorts_by_its_newest_done_subtask() {
    let mut app = App::new(vec![
        done_at(1, None, 100),
        done_at(2, None, 400),
        done_at(3, Some(10), 300),
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("epic", 10), ("task", 1)],
        "epic #10 sorts by subtask #3's completion, between 400 and 100"
    );
}

/// An epic's OWN `completed_at` beats the key derived from its subtasks. It is
/// an explicit statement about this card — written by the epic finishing, or by
/// a manual reorder — and either outranks a derived one.
#[test]
fn an_epics_own_completion_wins_over_its_subtasks() {
    let mut app = App::new(vec![
        done_at(1, None, 300),
        // The subtask finished most recently of all, but the epic's own stamp
        // is older, and the epic's own stamp is what the column reads.
        done_at(2, Some(10), 500),
    ]);
    let mut epic = make_epic(10);
    epic.completed_at = Some(at(100));
    app.board.epics.push(epic);

    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "epic #10 sits at its own 100, below the loose task's 300"
    );
}

/// The key comes from the column's own slice of the subtree. Work the Done
/// column is not showing must not decide where the card sits in it.
#[test]
fn only_done_subtasks_feed_the_epics_done_key() {
    let mut app = App::new(vec![
        done_at(1, None, 200),
        done_at(2, Some(10), 100),
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            completed_at: Some(at(900)),
            ..make_task(3, TaskStatus::Running)
        },
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "the Running task's 900 must not lift the epic above task #1"
    );
}

/// A sub-epic's work places the parent's card, so it must also date it.
#[test]
fn a_sub_epics_done_task_dates_the_parent_epic_card() {
    let mut app = App::new(vec![done_at(1, None, 100), done_at(2, Some(11), 300)]);
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

/// A card the column cannot date sorts last, below every dated one — it is the
/// card the column knows least about. Should not occur after
/// `migrate_v99_add_completed_at`.
#[test]
fn an_undated_epic_with_no_dated_subtask_sorts_last() {
    let mut app = App::new(vec![
        done_at(1, None, 100),
        crate::models::Task {
            epic_id: Some(EpicId(10)),
            completed_at: None,
            ..make_task(2, TaskStatus::Done)
        },
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "epic #10 has no completion anywhere, so it sits below the dated card"
    );
}

/// Done only. Review keeps the generic key, so an epic placed there must not be
/// reordered by any completion time.
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
            completed_at: Some(at(900)),
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
        done_at(1, Some(10), 100),
        done_at(2, Some(11), 300),
        done_at(3, Some(10), 200),
    ]);
    app.board.epics.push(make_epic(10));
    app.board.epics.push(make_epic(11));
    app.board.flattened = true;

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)],
        "epic #11's group leads on 300; within #10, 200 leads 100"
    );
}

/// The flattened key is the DIRECT epic's, not the subtree's: a sub-epic's
/// tasks are their own group, so they must not lift the parent's group.
#[test]
fn a_flattened_group_is_keyed_on_the_direct_epic_not_the_subtree() {
    // Sub-epic #11 under parent #10, plus unrelated epic #12. Keyed on the
    // direct epic the groups are 11 (400), 12 (200), 10 (100). Keyed on the
    // subtree, #10 would inherit #11's 400 and lead instead.
    let mut app = App::new(vec![
        done_at(1, Some(10), 100),
        done_at(2, Some(11), 400),
        done_at(3, Some(12), 200),
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
        "epic #10's group keeps its own 100; #11's 400 does not lift it"
    );
}

/// A flattened group is keyed on its member tasks and NOT on the epic's own
/// `completed_at` — unlike the hierarchical epic card. The group stands for the
/// column's tasks that name this epic, not for the epic.
#[test]
fn a_flattened_group_ignores_the_epics_own_completion() {
    let mut app = App::new(vec![done_at(1, None, 200), done_at(2, Some(10), 300)]);
    let mut epic = make_epic(10);
    epic.completed_at = Some(at(50));
    app.board.epics.push(epic);
    app.board.flattened = true;

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 1)],
        "the group leads on its task's 300; the epic's own 50 is not consulted"
    );
}

/// An epicless task in a flattened Done column ranks with the epic groups
/// rather than sinking below them. Done draws no OrphanSeparator, so the
/// orphans-last rule has nothing to separate and only hides fresh work.
#[test]
fn a_flattened_done_orphan_ranks_by_its_own_completion() {
    let mut app = App::new(vec![
        done_at(1, Some(10), 100),
        done_at(2, None, 300),
        done_at(3, Some(10), 200),
    ]);
    app.board.epics.push(make_epic(10));
    app.board.flattened = true;

    assert_eq!(
        done_cards(&app),
        vec![("task", 2), ("task", 3), ("task", 1)],
        "the epicless #2 leads on 300 instead of sinking to the bottom"
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

/// A positive `sort_order` on a done task is ordinary feed or manual ordering.
/// It used to be folded in as if it were a completion rank, which sank the
/// epic's card below every genuinely ranked one. The fields are separate now,
/// so it has no say at all.
#[test]
fn a_feeds_sort_order_on_a_done_task_does_not_reach_the_done_key() {
    let mut app = App::new(vec![
        done_at(1, None, 100),
        crate::models::Task {
            sort_order: Some(9_000),
            ..done_at(2, Some(10), 300)
        },
    ]);
    app.board.epics.push(make_epic(10));

    assert_eq!(
        done_cards(&app),
        vec![("epic", 10), ("task", 1)],
        "epic #10 leads on its subtask's 300; the 9000 sort_order is irrelevant"
    );
}

// --- Manual reorder in Done -------------------------------------------------

/// A task card in Done reorders by swapping the two cards' completion times.
#[test]
fn reordering_a_task_card_in_done_swaps_completed_at() {
    let mut app = App::new(vec![done_at(1, None, 300), done_at(2, None, 100)]);

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
    let task = |id: i64| {
        app.tasks()
            .iter()
            .find(|t| t.id.0 == id)
            .unwrap()
            .completed_at
    };
    assert_eq!(task(1), Some(at(100)), "the two times were swapped");
    assert_eq!(task(2), Some(at(300)));
}

/// An epic card in Done now owns a field the column reads, so the reorder is
/// honoured rather than refused: it writes `epic.completed_at`, which
/// `EpicPlacement::sort_key` prefers over the derived subtask key.
#[test]
fn reordering_an_epic_card_in_done_writes_its_completed_at() {
    let mut app = App::new(vec![done_at(1, None, 100), done_at(2, Some(10), 300)]);
    app.board.epics.push(make_epic(10));

    let done_col = TaskStatus::Done.column_index() + 1;
    app.selection_mut().set_column(done_col);
    app.selection_mut().set_row(done_col, 0);
    assert_eq!(
        done_cards(&app),
        vec![("epic", 10), ("task", 1)],
        "precondition: the epic card leads, and is under the cursor"
    );

    let cmds = app.handle_reorder_item(1);
    assert!(!cmds.is_empty(), "the swap is persisted");
    assert_eq!(
        app.board.epics[0].completed_at,
        Some(at(100)),
        "the epic took the task's completion time as an override"
    );
    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "and the card actually moved"
    );
}

/// The override beats the derived key on the next render, which is the whole
/// reason the epic branch can be honoured at all. Without the precedence the
/// write would land and the card would sit exactly where it was.
#[test]
fn an_epics_reorder_override_survives_its_subtasks_key() {
    let mut app = App::new(vec![done_at(1, None, 100), done_at(2, Some(10), 300)]);
    app.board.epics.push(make_epic(10));

    let done_col = TaskStatus::Done.column_index() + 1;
    app.selection_mut().set_column(done_col);
    app.selection_mut().set_row(done_col, 0);
    app.handle_reorder_item(1);

    assert_eq!(
        app.board
            .tasks
            .iter()
            .find(|t| t.id.0 == 2)
            .unwrap()
            .completed_at,
        Some(at(300)),
        "the subtask's own completion is untouched by the epic's reorder"
    );
    assert_eq!(
        done_cards(&app),
        vec![("task", 1), ("epic", 10)],
        "yet the epic stays below it, on its override"
    );
}

/// Nothing to swap: a card with no completion time is refused, and no command
/// is emitted.
#[test]
fn reordering_an_undated_card_in_done_is_refused() {
    let mut app = App::new(vec![
        done_at(1, None, 300),
        crate::models::Task {
            completed_at: None,
            ..make_task(2, TaskStatus::Done)
        },
    ]);

    let done_col = TaskStatus::Done.column_index() + 1;
    app.selection_mut().set_column(done_col);
    app.selection_mut().set_row(done_col, 0);

    let cmds = app.handle_reorder_item(1);
    assert!(cmds.is_empty(), "no command: the reorder is refused");
    assert_eq!(
        app.tasks()
            .iter()
            .find(|t| t.id.0 == 1)
            .unwrap()
            .completed_at,
        Some(at(300)),
        "and nothing is written"
    );
}

/// Two cards completed in the same millisecond have nothing to swap, so the
/// moved one is nudged instead. Done sorts descending, so moving DOWN means an
/// EARLIER time.
#[test]
fn reordering_past_an_identical_completion_nudges_the_moved_card() {
    let mut app = App::new(vec![done_at(1, None, 100), done_at(2, None, 100)]);

    let done_col = TaskStatus::Done.column_index() + 1;
    app.selection_mut().set_column(done_col);
    app.selection_mut().set_row(done_col, 0);

    let cmds = app.handle_reorder_item(1);
    assert!(!cmds.is_empty(), "the nudge is persisted");
    let moved = app.tasks().iter().find(|t| t.id.0 == 1).unwrap();
    assert_eq!(
        moved.completed_at,
        Some(at(100) - chrono::Duration::milliseconds(1)),
        "moving down subtracts a millisecond"
    );
    assert_eq!(done_cards(&app), vec![("task", 2), ("task", 1)]);
}
