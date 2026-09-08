//! Sub-status section headers as data-layer items, and what folding one does
//! to a column's contents.
//!
//! See "Column Sections" and "Collapsed Sections" in `docs/specs/core.allium`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{ColumnSection, SubStatus, TaskStatus};
use crate::tui::types::{FoldedHeader, SectionRef};
use crossterm::event::KeyCode;

/// A Running task in `sub_status`, with no epic.
fn running(id: i64, sub_status: SubStatus) -> crate::models::Task {
    let mut t = make_task(id, TaskStatus::Running);
    t.sub_status = sub_status;
    t
}

/// Every section header a column yields, in order, as
/// `(section, hidden)` — `hidden` is `None` for an open header. The one walk
/// the three projections below share.
fn section_headers(app: &App, status: TaskStatus) -> Vec<(ColumnSection, Option<usize>)> {
    app.column_items_for_status_with_stats(status, None)
        .into_iter()
        .filter_map(|i| match i {
            ColumnItem::SubstatusLabel(at) => Some((at.section, None)),
            ColumnItem::FoldedSection(h) => Some((h.at.section, Some(h.hidden))),
            _ => None,
        })
        .collect()
}

/// The sections a column yields a header for, in order.
fn headers(app: &App, status: TaskStatus) -> Vec<ColumnSection> {
    section_headers(app, status)
        .into_iter()
        .map(|(section, _)| section)
        .collect()
}

/// The hidden-card count on each folded header, in order.
fn hidden_counts(app: &App, status: TaskStatus) -> Vec<(ColumnSection, usize)> {
    section_headers(app, status)
        .into_iter()
        .filter_map(|(section, hidden)| hidden.map(|n| (section, n)))
        .collect()
}

/// The task ids a column renders, in order.
fn task_ids(app: &App, status: TaskStatus) -> Vec<i64> {
    app.column_items_for_status_with_stats(status, None)
        .into_iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(t.id.0),
            _ => None,
        })
        .collect()
}

// --- headers come from the data layer now, not the renderer ---

/// The non-flattened path used to leave headers to the renderer. They are
/// items now, so they can be counted and hold the cursor.
#[test]
fn a_sectioned_column_yields_a_header_per_section() {
    let app = App::new(vec![
        running(1, SubStatus::Active),
        running(2, SubStatus::NeedsInput),
    ]);
    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::NeedsInput, ColumnSection::Active],
        "urgent section first"
    );
}

/// Cards are ordered by their section's urgency first, and by (sort_order, id)
/// only within a section — so a lower-id Stale card must not outrank a
/// higher-id Conflict one. Sibling of the header-order check above: that one
/// pins where the headers go, this one pins where the cards go.
#[test]
fn cards_sort_by_section_urgency_before_id() {
    let app = App::new(vec![
        running(1, SubStatus::Stale),
        running(2, SubStatus::Conflict),
    ]);
    assert_eq!(
        task_ids(&app, TaskStatus::Running),
        vec![2, 1],
        "Conflict (id 2) must sort before Stale (id 1) by urgency, not id"
    );
}

#[test]
fn one_header_covers_every_card_in_its_section() {
    let app = App::new(vec![
        running(1, SubStatus::Active),
        running(2, SubStatus::Active),
        running(3, SubStatus::Active),
    ]);
    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::Active]
    );
}

/// Backlog and Done have no sections: every card there holds `sub_status =
/// none`, so a header would name nothing.
#[test]
fn an_unsectioned_column_yields_no_header() {
    let app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Done),
    ]);
    assert!(headers(&app, TaskStatus::Backlog).is_empty());
    assert!(headers(&app, TaskStatus::Done).is_empty());
}

/// `stale` and `shell stale` are one section, so they share one header — and
/// it is named for the section, not for whichever card sorts first.
#[test]
fn stale_and_shell_stale_share_a_single_header() {
    let app = App::new(vec![
        running(1, SubStatus::StaleShell),
        running(2, SubStatus::Stale),
    ]);
    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::Stale]
    );
}

/// An epic card sits in a section too, so an epic and a task in the same state
/// group under one header rather than the epic floating above every one.
#[test]
fn an_epic_card_groups_under_the_same_header_as_its_state() {
    let mut app = App::new(vec![running(1, SubStatus::Active)]);
    app.board.epics = vec![make_epic(10)];
    // make_epic defaults to Backlog; put it in Running so it shares the column.
    app.board.epics[0].status = TaskStatus::Running;
    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::Active]
    );
}

// --- folding hides the cards ---

#[test]
fn a_folded_section_keeps_its_header_and_drops_its_cards() {
    let mut app = App::new(vec![
        running(1, SubStatus::Active),
        running(2, SubStatus::NeedsInput),
        running(3, SubStatus::NeedsInput),
    ]);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);

    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::NeedsInput, ColumnSection::Active],
        "the folded section keeps its header"
    );
    assert_eq!(
        task_ids(&app, TaskStatus::Running),
        vec![1],
        "only the unfolded section's card is rendered"
    );
}

#[test]
fn a_folded_header_reports_how_many_cards_it_hides() {
    let mut app = App::new(vec![
        running(1, SubStatus::NeedsInput),
        running(2, SubStatus::NeedsInput),
        running(3, SubStatus::Active),
    ]);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    assert_eq!(
        hidden_counts(&app, TaskStatus::Running),
        vec![(ColumnSection::NeedsInput, 2)]
    );
}

/// The count is of cards the column would otherwise have drawn. A card the
/// repo filter already dropped is not one the fold is hiding.
#[test]
fn the_hidden_count_excludes_filtered_out_cards() {
    let mut a = running(1, SubStatus::NeedsInput);
    a.repo_path = "/keep".to_string();
    let mut b = running(2, SubStatus::NeedsInput);
    b.repo_path = "/drop".to_string();

    let mut app = App::new(vec![a, b]);
    app.set_repo_filter(["/keep".to_string()].into_iter().collect());
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);

    assert_eq!(
        hidden_counts(&app, TaskStatus::Running),
        vec![(ColumnSection::NeedsInput, 1)]
    );
}

/// A section with no cards has no header, folded or not — so a fold recorded
/// for it is simply not visible until a card lands there.
#[test]
fn a_folded_section_with_no_cards_renders_no_header() {
    let mut app = App::new(vec![running(1, SubStatus::Active)]);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::Crashed);
    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::Active]
    );
}

/// In a flattened column the epic headers and the orphan separator are
/// decoration on cards. Fold the cards away and they go too.
#[test]
fn folding_a_flattened_section_takes_its_decoration_with_it() {
    let mut owned = running(1, SubStatus::NeedsInput);
    owned.epic_id = Some(EpicId(10));
    let orphan = running(2, SubStatus::NeedsInput);

    let mut app = App::new(vec![owned, orphan, running(3, SubStatus::Active)]);
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);

    let items = app.column_items_for_status_with_stats(TaskStatus::Running, None);
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, ColumnItem::EpicHeader(_) | ColumnItem::OrphanSeparator)),
        "the folded section's decoration should be gone: {items:?}"
    );
    assert_eq!(task_ids(&app, TaskStatus::Running), vec![3]);
}

// --- the search override ---

/// A search that hid its own results would be worse than no search, so a live
/// query forces a folded section open.
#[test]
fn a_search_match_forces_its_folded_section_open() {
    let mut a = running(1, SubStatus::NeedsInput);
    a.title = "needle".to_string();
    let mut app = App::new(vec![a, running(2, SubStatus::Active)]);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    app.search.query = "needle".to_string();

    assert_eq!(
        task_ids(&app, TaskStatus::Running),
        vec![1],
        "the folded section's matching card is rendered"
    );
    assert!(
        hidden_counts(&app, TaskStatus::Running).is_empty(),
        "and its header is not drawn as folded"
    );
}

/// Display-only: the recorded fold is untouched, so clearing the query folds
/// the section again.
#[test]
fn the_search_override_does_not_clear_the_recorded_fold() {
    let mut a = running(1, SubStatus::NeedsInput);
    a.title = "needle".to_string();
    let mut app = App::new(vec![a]);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    app.search.query = "needle".to_string();
    let _ = task_ids(&app, TaskStatus::Running);

    assert!(app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));

    app.search.query.clear();
    assert_eq!(
        hidden_counts(&app, TaskStatus::Running),
        vec![(ColumnSection::NeedsInput, 1)],
        "clearing the query folds it again"
    );
}

// --- the z key ---

/// Board with a Running column holding two sections: `needs input` (two cards)
/// then `active` (one). The cursor starts on Running row 0.
fn folding_app() -> App {
    let mut app = App::new(vec![
        running(1, SubStatus::NeedsInput),
        running(2, SubStatus::NeedsInput),
        running(3, SubStatus::Active),
    ]);
    app.selection_mut().set_column(2); // Running = nav col 2
    app
}

/// The section under the cursor, if the cursor is on a card or a folded header.
fn cursor_section(app: &App) -> Option<ColumnSection> {
    match app.selected_column_item()? {
        ColumnItem::Task(t) => ColumnSection::for_task(t),
        ColumnItem::FoldedSection(h) => Some(h.at.section),
        _ => None,
    }
}

#[test]
fn z_on_a_card_folds_that_cards_section() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));
    assert!(!app.is_section_collapsed(TaskStatus::Running, ColumnSection::Active));
}

#[test]
fn z_on_the_resulting_header_unfolds_it() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(!app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));
}

/// Folding leaves the cursor on the folded section's own header. Asserted from
/// the *second* card of the section: the first card's row index coincides with
/// the header's, so a test that only covers it would pass on a cursor that
/// never moved.
#[test]
fn folding_from_mid_section_leaves_the_cursor_on_that_sections_header() {
    let mut app = folding_app();
    app.update(Message::NavigateRow(1)); // second `needs input` card
    assert_eq!(cursor_section(&app), Some(ColumnSection::NeedsInput));

    app.handle_key(make_key(KeyCode::Char('z')));

    match app.selected_column_item() {
        Some(ColumnItem::FoldedSection(h)) => {
            assert_eq!(h.at.section, ColumnSection::NeedsInput);
            assert!(h.hidden > 0);
        }
        other => panic!("cursor should rest on the folded header, got {other:?}"),
    }
}

#[test]
fn unfolding_leaves_the_cursor_on_the_sections_first_card() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));
    app.handle_key(make_key(KeyCode::Char('z')));
    match app.selected_column_item() {
        Some(ColumnItem::Task(t)) => assert_eq!(t.id.0, 1),
        other => panic!("cursor should rest on the first card, got {other:?}"),
    }
}

#[test]
fn space_and_enter_unfold_a_folded_header() {
    for key in [KeyCode::Char(' '), KeyCode::Enter] {
        let mut app = folding_app();
        app.handle_key(make_key(KeyCode::Char('z')));
        assert!(app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));
        app.handle_key(make_key(key));
        assert!(
            !app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput),
            "{key:?} should unfold"
        );
    }
}

/// Space and Enter keep their card meanings — the header routing must not
/// swallow them.
#[test]
fn enter_on_a_card_still_opens_the_detail_panel() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Enter));
    assert!(matches!(app.view_mode(), ViewMode::TaskDetail { .. }));
}

/// Backlog and Done have no sections, so there is nothing for `z` to fold.
#[test]
fn z_is_a_no_op_in_a_column_without_sections() {
    for (nav_col, status) in [(1, TaskStatus::Backlog), (4, TaskStatus::Done)] {
        let mut app = App::new(vec![make_task(1, status)]);
        app.selection_mut().set_column(nav_col);
        app.handle_key(make_key(KeyCode::Char('z')));
        for &section in ColumnSection::ALL {
            assert!(
                !app.is_section_collapsed(status, section),
                "{status:?}/{section:?}"
            );
        }
    }
}

#[test]
fn z_is_a_no_op_on_the_select_all_cursor_position() {
    let mut app = folding_app();
    app.selection_mut().on_select_all = true;
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(!app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));
}

#[test]
fn z_is_a_no_op_in_an_empty_column() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(2);
    app.handle_key(make_key(KeyCode::Char('z')));
    for &section in ColumnSection::ALL {
        assert!(!app.is_section_collapsed(TaskStatus::Running, section));
    }
}

/// `z` writes the recorded set, not what is on screen. A section held open by
/// a live query is still recorded folded, so `z` there records "unfolded" —
/// which shows the moment the query is cleared.
#[test]
fn z_toggles_the_recorded_state_under_a_search_override() {
    let mut a = running(1, SubStatus::NeedsInput);
    a.title = "needle".to_string();
    let mut app = App::new(vec![a]);
    app.selection_mut().set_column(2);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    app.search.query = "needle".to_string();

    // The card is rendered (override), so the cursor is on it.
    assert_eq!(cursor_section(&app), Some(ColumnSection::NeedsInput));
    app.handle_key(make_key(KeyCode::Char('z')));

    assert!(
        !app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput),
        "z should have recorded the section as unfolded"
    );
}

/// Nothing else acts on a header: there is no task and no epic under it.
#[test]
fn card_actions_are_no_ops_on_a_folded_header() {
    for key in [
        KeyCode::Char('x'),
        KeyCode::Char('v'),
        KeyCode::Char('e'),
        KeyCode::Char('L'),
        KeyCode::Char('H'),
        KeyCode::Char('J'),
        KeyCode::Char('K'),
        KeyCode::Char('p'),
    ] {
        let mut app = folding_app();
        app.handle_key(make_key(KeyCode::Char('z')));
        let before = app.board.tasks.clone();
        app.handle_key(make_key(key));
        assert!(
            app.select.tasks.is_empty() && app.select.epics.is_empty(),
            "{key:?} selected something on a header"
        );
        assert_eq!(
            app.board.tasks.len(),
            before.len(),
            "{key:?} changed the board on a header"
        );
        assert!(
            app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput),
            "{key:?} should not have unfolded the section"
        );
    }
}

/// Select-all reaches the cards the column renders, which does not include a
/// folded section's. The header itself is not a card and is never selected.
#[test]
fn select_all_skips_a_folded_sections_cards() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));
    app.update(Message::SelectAllColumn);
    assert_eq!(
        app.select.tasks.iter().map(|t| t.0).collect::<Vec<_>>(),
        vec![3],
        "only the unfolded section's card is selected"
    );
}

/// Folding hides cards; it does not deselect them.
#[test]
fn a_selected_card_stays_selected_while_its_section_is_folded() {
    let mut app = folding_app();
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 3);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    assert_eq!(app.select.tasks.len(), 3, "folding must not deselect");
}

// --- selectability, counting and navigation ---

/// A folded section holds the cursor; an open header never does. The two are
/// separate `ColumnItem` variants precisely so this is a fact about the
/// variant rather than a runtime flag every consumer must re-check.
#[test]
fn only_a_folded_section_is_selectable() {
    let at = SectionRef::new(TaskStatus::Running, ColumnSection::Active);
    assert!(!ColumnItem::SubstatusLabel(at).is_selectable());
    assert!(ColumnItem::FoldedSection(FoldedHeader { at, hidden: 3 }).is_selectable());

    // And through the real board, to pin that the builder emits the right one.
    let mut app = folding_app();
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    let items = app.column_items_for_status_with_stats(TaskStatus::Running, None);
    let kinds: Vec<bool> = items
        .iter()
        .filter(|i| {
            matches!(
                i,
                ColumnItem::SubstatusLabel(_) | ColumnItem::FoldedSection(_)
            )
        })
        .map(|i| i.is_selectable())
        .collect();
    assert_eq!(kinds, vec![true, false], "folded first, then open");
}

/// `anchor()` is `Some` exactly where `is_selectable()` is true — the
/// anchor-cache builder relies on the two being one fact.
#[test]
fn anchor_is_some_exactly_where_an_item_is_selectable() {
    let mut app = folding_app();
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    for status in [TaskStatus::Running, TaskStatus::Backlog] {
        for item in app.column_items_for_status_with_stats(status, None) {
            assert_eq!(
                item.anchor().is_some(),
                item.is_selectable(),
                "{item:?} in {status:?}"
            );
        }
    }
}

/// The navigable count follows what is rendered: the folded header joins in,
/// its hidden cards drop out.
#[test]
fn the_item_count_counts_a_folded_header_and_not_its_cards() {
    let mut app = folding_app();
    assert_eq!(app.column_item_count(TaskStatus::Running), 3);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    assert_eq!(
        app.column_item_count(TaskStatus::Running),
        2,
        "one folded header plus the one visible card"
    );
}

#[test]
fn j_and_k_step_onto_a_folded_header_and_past_an_expanded_one() {
    let mut app = folding_app();
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    app.selection_mut().set_row(2, 0);

    // Row 0 is the folded header.
    assert!(matches!(
        app.selected_column_item(),
        Some(ColumnItem::FoldedSection(_))
    ));
    // Down goes straight to the card under the *expanded* `active` header,
    // which is skipped.
    app.update(Message::NavigateRow(1));
    match app.selected_column_item() {
        Some(ColumnItem::Task(t)) => assert_eq!(t.id.0, 3),
        other => panic!("expected the active card, got {other:?}"),
    }
    app.update(Message::NavigateRow(-1));
    assert!(matches!(
        app.selected_column_item(),
        Some(ColumnItem::FoldedSection(_))
    ));
}

/// The header has no entity behind it, so the cursor could not survive a
/// refresh on one without its own anchor.
#[test]
fn the_cursor_survives_a_refresh_while_on_a_folded_header() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));

    let tasks = app.board.tasks.clone();
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        tasks,
    )));

    match app.selected_column_item() {
        Some(ColumnItem::FoldedSection(h)) => {
            assert_eq!(h.at.section, ColumnSection::NeedsInput)
        }
        other => panic!("cursor should still be on the folded header, got {other:?}"),
    }
}

#[test]
fn there_is_no_task_or_epic_under_a_folded_header() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(app.selected_task().is_none());
    assert!(app.selected_epic_id().is_none());
}

/// The header goes when its last card does, and the cursor clamps inside the
/// same column rather than following the vanished section anywhere.
#[test]
fn a_folded_section_losing_its_last_card_drops_the_header() {
    let mut app = folding_app();
    app.handle_key(make_key(KeyCode::Char('z')));

    // Both `needs input` cards move to Review, emptying the folded section.
    let moved: Vec<_> = app
        .board
        .tasks
        .iter()
        .map(|t| {
            let mut t = t.clone();
            if t.sub_status == SubStatus::NeedsInput {
                t.status = TaskStatus::Review;
                t.sub_status = SubStatus::AwaitingReview;
            }
            t
        })
        .collect();
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        moved,
    )));

    assert_eq!(
        headers(&app, TaskStatus::Running),
        vec![ColumnSection::Active],
        "the emptied section's header is gone"
    );
    assert!(
        app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput),
        "and the fold is still recorded, so the header returns folded"
    );
    let col = app.selection().column();
    let row = app.selection().row(col);
    assert!(
        row < app.column_item_count(TaskStatus::Running).max(1),
        "cursor should have clamped inside the column, got row {row}"
    );
}

// --- rendering ---

/// The header bar count answers how much work is in the column, so folding —
/// a choice about screen space — must not move it. And a header is not a card.
#[test]
fn the_column_header_count_ignores_folding() {
    let mut app = folding_app();
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains_ignore_case(&buf, "RUNNING 3"),
        "expected RUNNING 3 before folding"
    );

    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains_ignore_case(&buf, "RUNNING 3"),
        "the count must not move when a section folds"
    );
}

/// Regression guard: the select-all checkbox used to filter on `is_selectable`
/// and then match with an `unreachable!()`, which a folded header reaches.
#[test]
fn rendering_a_folded_section_in_the_focused_column_does_not_panic() {
    let mut app = folding_app();
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::Active);
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(buffer_contains(&buf, "needs input"));
}

#[test]
fn a_folded_header_draws_its_count_and_marker() {
    let mut app = folding_app();
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "needs input (2) \u{22ef}"),
        "expected the folded header to carry its count and marker"
    );
}

#[test]
fn an_expanded_header_draws_neither_count_nor_marker() {
    let mut app = folding_app();
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(buffer_contains(&buf, "needs input"));
    assert!(
        !buffer_contains(&buf, "needs input ("),
        "an expanded header carries no count"
    );
    assert!(
        !buffer_contains(&buf, "\u{22ef}"),
        "an expanded header carries no fold marker"
    );
}

/// The spec says folds apply wherever sections render, off one recorded set.
/// Drilling into an epic must not quietly unfold everything.
#[test]
fn a_fold_holds_inside_an_epic_view() {
    let mut a = running(1, SubStatus::NeedsInput);
    a.epic_id = Some(EpicId(10));
    let mut b = running(2, SubStatus::Active);
    b.epic_id = Some(EpicId(10));

    let mut app = App::new(vec![a, b]);
    app.board.epics = vec![make_epic(10)];
    app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));

    assert_eq!(
        hidden_counts(&app, TaskStatus::Running),
        vec![(ColumnSection::NeedsInput, 1)],
        "the fold recorded on the board still holds inside the epic"
    );
    assert_eq!(task_ids(&app, TaskStatus::Running), vec![2]);
}

/// And the reverse: a fold made inside an epic view is the same recorded
/// entry, so it is still folded on the way back out.
#[test]
fn a_fold_made_inside_an_epic_view_holds_on_the_board() {
    let mut a = running(1, SubStatus::NeedsInput);
    a.epic_id = Some(EpicId(10));
    let mut app = App::new(vec![a, running(2, SubStatus::NeedsInput)]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    app.selection_mut().set_column(2);
    app.handle_key(make_key(KeyCode::Char('z')));

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));
    assert!(app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));
}

/// Split view insets the same four columns inside a focus border, so their
/// sections fold there exactly as they do with the pane closed.
#[test]
fn folding_works_in_split_view() {
    let mut app = folding_app();
    app.board.split.active = true;
    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(app.is_section_collapsed(TaskStatus::Running, ColumnSection::NeedsInput));

    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "needs input (2) \u{22ef}"),
        "the folded header should render inside the split border"
    );
}

/// An epic card sits in a section, so `z` on one folds that section — and it
/// must work off a cold layout cache too, since nothing guarantees the cache
/// is warm when a key arrives.
#[test]
fn z_on_an_epic_card_folds_its_section_with_a_cold_cache() {
    let mut app = App::new(vec![running(1, SubStatus::Active)]);
    app.board.epics = vec![make_epic(10)];
    app.board.epics[0].status = TaskStatus::Running;
    app.selection_mut().set_column(2);
    app.invalidate_layout_cache();

    // Put the cursor on the epic card rather than the task.
    let items = app.column_items_for_status_with_stats(TaskStatus::Running, None);
    let epic_row = items
        .iter()
        .filter(|i| i.is_selectable())
        .position(|i| matches!(i, ColumnItem::Epic(_)))
        .expect("the epic card should be selectable");
    app.selection_mut().set_row(2, epic_row);
    app.invalidate_layout_cache();

    app.handle_key(make_key(KeyCode::Char('z')));
    assert!(
        app.is_section_collapsed(TaskStatus::Running, ColumnSection::Active),
        "z on an epic card should fold the section it renders under"
    );
}
