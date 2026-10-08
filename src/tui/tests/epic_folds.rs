//! Folded epic groups in a flattened column: the recorded set, its serialised
//! form, the `Z` key, and how it composes with section folding.
//!
//! See "Epic Folding" in `docs/specs/board-layout.allium` and `ToggleEpicFold`
//! in `docs/specs/tasks.allium`.
use super::*;
use crate::models::{ColumnSection, EpicId, SubStatus, TaskStatus};
use crate::tui::types::{ColumnAnchor, EpicFoldRef, EpicFoldState, FoldedEpicHeader};
use crossterm::event::KeyCode;

/// A Running task in `sub_status`, owned by `epic_id`.
fn owned(id: i64, sub_status: SubStatus, epic_id: i64) -> Task {
    let mut t = make_task(id, TaskStatus::Running);
    t.sub_status = sub_status;
    t.epic_id = Some(EpicId(epic_id));
    t
}

// --- the recorded set ---

#[test]
fn a_fresh_board_has_nothing_folded() {
    let app = App::new(vec![]);
    for &status in TaskStatus::ALL {
        assert!(!app.epic_folds.is_folded(status, EpicId(10)), "{status:?}");
    }
}

#[test]
fn toggling_folds_then_unfolds() {
    let mut app = App::new(vec![]);
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
}

/// Folding is per (column, epic), not board-wide: the same epic folds
/// independently in Running and in Review.
#[test]
fn the_same_epic_in_two_columns_folds_independently() {
    let mut app = App::new(vec![]);
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
    assert!(!app.epic_folds.is_folded(TaskStatus::Review, EpicId(10)));
}

#[test]
fn only_the_column_holding_a_fold_reports_one() {
    let mut app = App::new(vec![]);
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert!(app.view().column_has_rendered_fold(TaskStatus::Running));
    assert!(!app.view().column_has_rendered_fold(TaskStatus::Review));
}

/// The same override Collapsed Sections gets: a live query forces every fold
/// open, epic folds included, without touching what is recorded.
#[test]
fn a_live_search_query_means_no_column_has_a_rendered_fold() {
    let mut app = App::new(vec![]);
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    app.search.query = "anything".to_string();
    assert!(!app.view().column_has_rendered_fold(TaskStatus::Running));
    assert!(
        app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)),
        "and the recorded fold is untouched"
    );
}

// --- serialised form ---

#[test]
fn the_serialised_form_round_trips() {
    let mut folds = EpicFoldState::default();
    folds.toggle(TaskStatus::Running, EpicId(10));
    folds.toggle(TaskStatus::Review, EpicId(20));

    let text = folds.serialise();
    let back = EpicFoldState::parse(&text);
    assert_eq!(back, folds, "serialised as {text:?}");
}

#[test]
fn an_empty_set_serialises_to_the_empty_string() {
    assert_eq!(EpicFoldState::default().serialise(), "");
    assert_eq!(EpicFoldState::parse(""), EpicFoldState::default());
}

/// A stored entry this binary cannot resolve (an unknown status, or garbage
/// where an epic id belongs) is skipped, not fatal — a display preference
/// that cannot be honoured, never a corrupt task.
#[test]
fn an_unrecognised_entry_is_skipped_and_the_rest_load() {
    let text = "running/10,not_a_status/10,running/not_a_number,malformed,review/20";
    let folds = EpicFoldState::parse(text);

    let mut expected = EpicFoldState::default();
    expected.toggle(TaskStatus::Running, EpicId(10));
    expected.toggle(TaskStatus::Review, EpicId(20));
    assert_eq!(folds, expected);
}

// --- rendering: a flattened column's epic groups ---

#[test]
fn folding_an_epic_hides_its_cards_but_keeps_its_header() {
    let mut app = App::new(vec![
        owned(1, SubStatus::Active, 10),
        owned(2, SubStatus::Active, 10),
        owned(3, SubStatus::Active, 20),
    ]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.board.flattened = true;
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));

    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    let task_ids: Vec<i64> = items
        .iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(t.id.0),
            _ => None,
        })
        .collect();
    assert_eq!(task_ids, vec![3], "epic 10's cards are hidden");

    let folded: Vec<&FoldedEpicHeader> = items
        .iter()
        .filter_map(|i| match i {
            ColumnItem::FoldedEpic(h) => Some(h),
            _ => None,
        })
        .collect();
    assert_eq!(folded.len(), 1);
    assert_eq!(folded[0].at.epic, EpicId(10));
    assert_eq!(folded[0].hidden, 2);

    assert!(
        items
            .iter()
            .any(|i| matches!(i, ColumnItem::EpicHeader(e) if e.id == EpicId(20))),
        "epic 20's own header is unaffected"
    );
}

/// Scope is per (column, epic), not per (column, section, epic): folding an
/// epic whose tasks straddle two substatus sections in the same column hides
/// every one of its cards, in every section.
#[test]
fn folding_an_epic_hides_it_across_every_section_in_the_column() {
    let mut app = App::new(vec![
        owned(1, SubStatus::Active, 10),
        owned(2, SubStatus::NeedsInput, 10),
    ]);
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));

    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    assert!(
        !items.iter().any(|i| matches!(i, ColumnItem::Task(_))),
        "both sections' cards are hidden: {items:?}"
    );
    let hidden: usize = items
        .iter()
        .filter_map(|i| match i {
            ColumnItem::FoldedEpic(h) => Some(h.hidden),
            _ => None,
        })
        .sum();
    assert_eq!(
        hidden, 2,
        "one folded header per section, hiding one card each"
    );
}

/// An orphan task (no epic) is never part of a fold: nothing here applies to
/// it.
#[test]
fn an_orphan_tasks_card_is_unaffected_by_any_epic_fold() {
    let mut app = App::new(vec![owned(1, SubStatus::Active, 10), running(2)]);
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));

    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    assert!(items
        .iter()
        .any(|i| matches!(i, ColumnItem::Task(t) if t.id.0 == 2)));
}

fn running(id: i64) -> Task {
    let mut t = make_task(id, TaskStatus::Running);
    t.sub_status = SubStatus::Active;
    t
}

/// Only meaningful where the flattened epic-header row renders at all. An
/// unflattened board draws real epic cards instead, which this feature does
/// not touch.
#[test]
fn an_epic_fold_does_nothing_in_an_unflattened_column() {
    let mut app = App::new(vec![owned(1, SubStatus::Active, 10)]);
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = false;
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));

    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    assert!(
        !items.iter().any(|i| matches!(i, ColumnItem::FoldedEpic(_))),
        "hierarchical mode never renders a folded epic header: {items:?}"
    );
}

// --- selectability and anchoring ---

#[test]
fn only_a_folded_epic_is_selectable() {
    let epic = make_epic(10);
    let at = EpicFoldRef::new(TaskStatus::Running, EpicId(10));
    assert!(!ColumnItem::EpicHeader(&epic).is_selectable());
    assert!(ColumnItem::FoldedEpic(FoldedEpicHeader {
        at,
        epic: &epic,
        hidden: 2
    })
    .is_selectable());
}

#[test]
fn anchor_of_a_folded_epic_header_is_the_epic_fold_ref() {
    let epic = make_epic(10);
    let at = EpicFoldRef::new(TaskStatus::Running, EpicId(10));
    let item = ColumnItem::FoldedEpic(FoldedEpicHeader {
        at,
        epic: &epic,
        hidden: 2,
    });
    assert_eq!(item.anchor(), Some(ColumnAnchor::EpicFold(at)));
}

// --- the Z key ---

/// Board with a flattened Running column holding two epics, cursor on epic
/// 10's first card.
fn folding_app() -> App {
    let mut app = App::new(vec![
        owned(1, SubStatus::Active, 10),
        owned(2, SubStatus::Active, 10),
        owned(3, SubStatus::Active, 20),
    ]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.board.flattened = true;
    app.selection_mut().set_column(2); // Running = nav col 2
    app
}

#[test]
fn shift_z_on_a_card_folds_that_cards_epic() {
    let mut app = folding_app();
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(20)));
}

#[test]
fn shift_z_on_the_resulting_header_unfolds_it() {
    let mut app = folding_app();
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
}

/// Folding leaves the cursor on the epic's own folded header, not wherever a
/// numerically-unchanged row would land.
#[test]
fn folding_from_the_second_card_leaves_the_cursor_on_that_epics_header() {
    let mut app = folding_app();
    app.update(Message::NavigateRow(1)); // second card of epic 10
    app.handle_key(make_shift_key(KeyCode::Char('Z')));

    match app.selected_column_item() {
        Some(ColumnItem::FoldedEpic(h)) => {
            assert_eq!(h.at.epic, EpicId(10));
            assert_eq!(h.hidden, 2);
        }
        other => panic!("cursor should rest on the folded epic header, got {other:?}"),
    }
}

#[test]
fn unfolding_leaves_the_cursor_on_the_epics_first_card() {
    let mut app = folding_app();
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    match app.selected_column_item() {
        Some(ColumnItem::Task(t)) => assert_eq!(t.id.0, 1),
        other => panic!("cursor should rest on the first card, got {other:?}"),
    }
}

#[test]
fn space_and_enter_unfold_a_folded_epic_header() {
    for key in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut app = folding_app();
        app.handle_key(make_shift_key(KeyCode::Char('Z')));
        assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
        app.handle_key(make_key(key));
        assert!(
            !app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)),
            "{key:?} should unfold"
        );
    }
}

/// A card with no epic has no group for `Z` to fold.
#[test]
fn shift_z_is_a_no_op_on_a_card_with_no_epic() {
    let mut app = App::new(vec![running(1)]);
    app.board.flattened = true;
    app.selection_mut().set_column(2);
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
}

#[test]
fn shift_z_is_a_no_op_in_an_unflattened_column() {
    let mut app = folding_app();
    app.board.flattened = false;
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
}

#[test]
fn shift_z_is_a_no_op_on_the_select_all_cursor_position() {
    let mut app = folding_app();
    app.selection_mut().on_select_all = true;
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    assert!(!app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
}

/// Selection is untouched: a selected card stays selected while its epic is
/// folded, and select-all reaches only the cards the column still renders.
#[test]
fn a_selected_card_stays_selected_while_its_epic_is_folded() {
    let mut app = folding_app();
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 3);
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert_eq!(app.select.tasks.len(), 3, "folding must not deselect");
}

#[test]
fn select_all_skips_a_folded_epics_cards() {
    let mut app = folding_app();
    app.handle_key(make_shift_key(KeyCode::Char('Z')));
    app.update(Message::SelectAllColumn);
    assert_eq!(
        app.select.tasks.iter().map(|t| t.0).collect::<Vec<_>>(),
        vec![3],
        "only epic 20's card is selected"
    );
}

// --- independent of section folding ---

/// An epic fold and a section fold are separate recorded sets: folding the
/// epic does not fold the section, and vice versa.
#[test]
fn an_epic_fold_and_a_section_fold_are_independent_state() {
    let mut app = folding_app();
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    assert!(!app
        .folds
        .is_collapsed(TaskStatus::Running, ColumnSection::Active));

    app.folds.toggle(TaskStatus::Running, ColumnSection::Active);
    assert!(app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)));
    assert!(app
        .folds
        .is_collapsed(TaskStatus::Running, ColumnSection::Active));
}

/// A folded section hides its epic groups along with everything else in the
/// run; the epic fold underneath is untouched and resumes the moment the
/// section reopens.
#[test]
fn a_folded_section_hides_a_folded_epics_header_too() {
    let mut app = folding_app();
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    app.folds.toggle(TaskStatus::Running, ColumnSection::Active);

    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    assert!(
        !items.iter().any(|i| matches!(i, ColumnItem::FoldedEpic(_))),
        "the section's own folded header stands in for everything in the run: {items:?}"
    );

    app.folds.toggle(TaskStatus::Running, ColumnSection::Active);
    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    assert!(
        items
            .iter()
            .any(|i| matches!(i, ColumnItem::FoldedEpic(h) if h.at.epic == EpicId(10))),
        "reopening the section reveals the epic still folded: {items:?}"
    );
}

// --- the search override ---

/// A search that hid its own results would be worse than no search: a folded
/// epic holding a match renders open, without touching the recorded fold.
#[test]
fn a_live_search_query_reopens_a_folded_epic_holding_a_match() {
    let mut needle = owned(1, SubStatus::Active, 10);
    needle.title = "findme".to_string();
    let mut app = App::new(vec![needle, owned(2, SubStatus::Active, 10)]);
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;
    app.epic_folds.toggle(TaskStatus::Running, EpicId(10));
    app.search.query = "findme".to_string();

    let items = app
        .view()
        .column_items_for_status_with_placements(TaskStatus::Running, None);
    assert!(
        items
            .iter()
            .any(|i| matches!(i, ColumnItem::Task(t) if t.id.0 == 1)),
        "the matching card is visible: {items:?}"
    );
    assert!(
        app.epic_folds.is_folded(TaskStatus::Running, EpicId(10)),
        "the recorded fold is untouched"
    );
}

// --- persistence ---

#[test]
fn toggling_persists_the_serialised_set() {
    let mut app = folding_app();
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('Z')));
    let persisted = cmds.iter().find_map(|c| match c {
        Command::Settings(crate::tui::commands::SettingsCommand::PersistStringSetting {
            key,
            value,
        }) if key == crate::tui::COLLAPSED_EPICS_KEY => Some(value.clone()),
        _ => None,
    });
    assert_eq!(persisted.as_deref(), Some("running/10"));
}
