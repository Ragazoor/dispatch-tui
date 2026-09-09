//! Folded sub-status sections: the recorded set, and its serialised form.
//!
//! See "Collapsed Sections" in `docs/specs/board-layout.allium`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::models::{ColumnSection, TaskStatus};
use crate::tui::types::SectionFoldState;

#[test]
fn a_fresh_board_has_nothing_folded() {
    let app = App::new(vec![]);
    for &status in TaskStatus::ALL {
        for &section in ColumnSection::ALL {
            assert!(
                !app.is_section_collapsed(status, section),
                "{status:?}/{section:?}"
            );
        }
    }
}

#[test]
fn toggling_folds_then_unfolds() {
    let mut app = App::new(vec![]);
    app.toggle_section_collapse(TaskStatus::Review, ColumnSection::Approved);
    assert!(app.is_section_collapsed(TaskStatus::Review, ColumnSection::Approved));
    app.toggle_section_collapse(TaskStatus::Review, ColumnSection::Approved);
    assert!(!app.is_section_collapsed(TaskStatus::Review, ColumnSection::Approved));
}

/// `conflict` is a section in both Running and Review, and the two fold
/// independently — the fold is keyed on the column as well as the section.
#[test]
fn the_same_section_in_two_columns_folds_independently() {
    let mut app = App::new(vec![]);
    app.toggle_section_collapse(TaskStatus::Review, ColumnSection::Conflict);
    assert!(app.is_section_collapsed(TaskStatus::Review, ColumnSection::Conflict));
    assert!(!app.is_section_collapsed(TaskStatus::Running, ColumnSection::Conflict));
}

#[test]
fn only_the_column_holding_a_fold_reports_one() {
    let mut app = App::new(vec![]);
    app.toggle_section_collapse(TaskStatus::Review, ColumnSection::Approved);
    assert!(app.column_has_rendered_fold(TaskStatus::Review));
    assert!(!app.column_has_rendered_fold(TaskStatus::Running));
}

/// The predicate is about folds that *take effect*, so a live query — which
/// overrides every fold — makes it false. Stating the override once is what
/// keeps the render path and the item count agreeing about it.
#[test]
fn a_live_search_query_means_no_column_has_a_rendered_fold() {
    let mut app = App::new(vec![]);
    app.toggle_section_collapse(TaskStatus::Review, ColumnSection::Approved);
    app.search.query = "anything".to_string();
    assert!(!app.column_has_rendered_fold(TaskStatus::Review));
    assert!(
        app.is_section_collapsed(TaskStatus::Review, ColumnSection::Approved),
        "and the recorded fold is untouched"
    );
}

// --- serialised form ---

#[test]
fn the_serialised_form_round_trips() {
    let mut folds = SectionFoldState::default();
    folds.toggle(TaskStatus::Review, ColumnSection::Approved);
    folds.toggle(TaskStatus::Running, ColumnSection::Stale);
    folds.toggle(TaskStatus::Review, ColumnSection::ApprovedByMe);

    let text = folds.serialise();
    let back = SectionFoldState::parse(&text);
    assert_eq!(back, folds, "serialised as {text:?}");
}

/// Stable output order, so a settings row does not churn between runs and a
/// snapshot over it stays deterministic.
#[test]
fn the_serialised_form_is_ordered() {
    let mut a = SectionFoldState::default();
    a.toggle(TaskStatus::Review, ColumnSection::Approved);
    a.toggle(TaskStatus::Running, ColumnSection::Stale);

    let mut b = SectionFoldState::default();
    b.toggle(TaskStatus::Running, ColumnSection::Stale);
    b.toggle(TaskStatus::Review, ColumnSection::Approved);

    assert_eq!(a.serialise(), b.serialise());
}

#[test]
fn an_empty_set_serialises_to_the_empty_string() {
    assert_eq!(SectionFoldState::default().serialise(), "");
    assert_eq!(SectionFoldState::parse(""), SectionFoldState::default());
}

/// A stored entry this binary cannot resolve is skipped, not fatal: a fold
/// naming a section that no longer exists is a display preference that cannot
/// be honoured, never a corrupt task. The entries around it still load.
#[test]
fn an_unrecognised_entry_is_skipped_and_the_rest_load() {
    let text = "review/approved,review/invented_section,not_a_status/stale,malformed,running/stale";
    let folds = SectionFoldState::parse(text);

    let mut expected = SectionFoldState::default();
    expected.toggle(TaskStatus::Review, ColumnSection::Approved);
    expected.toggle(TaskStatus::Running, ColumnSection::Stale);
    assert_eq!(folds, expected);
}

/// A fold for a section that cannot occur in that column is inert rather than
/// rejected: it names nothing the board renders, so it neither errors nor
/// folds anything else.
#[test]
fn a_fold_naming_an_impossible_pair_is_inert() {
    let folds = SectionFoldState::parse("backlog/approved");
    assert!(!folds.is_collapsed(TaskStatus::Review, ColumnSection::Approved));
    assert!(!folds.is_collapsed(TaskStatus::Running, ColumnSection::Approved));
}
