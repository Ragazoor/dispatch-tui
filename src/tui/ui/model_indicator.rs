//! Top-row active-model indicator rendering (docs/specs/dispatch.allium:
//! ActiveModelIndicator).
//!
//! Sourced from the same snapshot as the budget indicator (see
//! `super::budget`) — same file, same statusLine hook payload — but rendered
//! and degraded independently. `now` and `stale_after` are parameters rather
//! than wall-clock reads for the same testability reason `budget.rs` takes
//! them (docs/conventions.md: no sleeping in tests).

use super::palette::MUTED;
use crate::models::budget::BudgetSnapshot;
use ratatui::style::Style;
use ratatui::text::Span;
use std::time::Duration;

/// Build the indicator's spans: the model's display_name verbatim, with no
/// label prefix, dimmed when the snapshot is stale. Empty when there is no
/// snapshot or no model has been reported.
pub(in crate::tui::ui) fn model_spans(
    snapshot: Option<&BudgetSnapshot>,
    now: i64,
    stale_after: Duration,
) -> Vec<Span<'static>> {
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };
    let Some(model) = snapshot.model.as_ref() else {
        return Vec::new();
    };

    let style = if snapshot.is_stale(now, stale_after) {
        Style::default().fg(MUTED)
    } else {
        Style::default()
    };

    vec![Span::styled(model.clone(), style), Span::raw("  ")]
}

/// Compose the model badge together with the budget indicator's spans.
///
/// Applies the priority rule from dispatch.allium's ActiveModelIndicator
/// (`DegradesBeforeTokenBudgetIndicator`): the model badge is the newer of
/// the two occupants, so it is dropped first, in full, whenever it does not
/// also fit alongside `budget_spans`'s own best-fitting degradation level for
/// `width_budget`. Pre-existing badges in the row are never sacrificed for
/// either — that is the caller's job, via the `width_budget` it hands in.
pub(in crate::tui::ui) fn top_row_spans(
    snapshot: Option<&BudgetSnapshot>,
    now: i64,
    stale_after: Duration,
    width_budget: usize,
) -> Vec<Span<'static>> {
    let budget = super::budget::budget_spans(snapshot, now, stale_after, width_budget);
    let model = model_spans(snapshot, now, stale_after);

    let mut spans = Vec::new();
    if !model.is_empty()
        && super::shared::spans_width(&model) + super::shared::spans_width(&budget) <= width_budget
    {
        spans.extend(model);
    }
    spans.extend(budget);
    spans
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::models::budget::BudgetWindow;

    const STALE: Duration = Duration::from_secs(600);

    fn snap_with_model(model: Option<&str>, captured_at: i64) -> BudgetSnapshot {
        BudgetSnapshot {
            five_hour: Some(BudgetWindow {
                used_percentage: 10.0,
                resets_at: captured_at + 100,
            }),
            seven_day: None,
            model: model.map(|m| m.to_string()),
            captured_at,
        }
    }

    fn text_of(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn renders_display_name_verbatim_with_no_label() {
        let snap = snap_with_model(Some("Sonnet 5"), 0);
        let text = text_of(&model_spans(Some(&snap), 0, STALE));
        assert!(text.contains("Sonnet 5"), "got {text:?}");
        assert!(!text.contains(':'), "must have no label prefix: {text:?}");
        assert_eq!(
            text.matches("Sonnet 5").count(),
            1,
            "must not duplicate the model name: {text:?}"
        );
    }

    #[test]
    fn no_snapshot_renders_nothing() {
        assert!(model_spans(None, 0, STALE).is_empty());
    }

    #[test]
    fn snapshot_without_model_renders_nothing() {
        let snap = snap_with_model(None, 0);
        assert!(model_spans(Some(&snap), 0, STALE).is_empty());
    }

    #[test]
    fn stale_snapshot_is_dimmed() {
        let snap = snap_with_model(Some("Opus"), 0);
        let spans = model_spans(Some(&snap), 1_020, STALE);
        assert!(
            spans.iter().any(|s| s.style.fg == Some(MUTED)),
            "expected the readout dimmed when stale"
        );
    }

    #[test]
    fn fresh_snapshot_is_not_dimmed() {
        let snap = snap_with_model(Some("Opus"), 0);
        let spans = model_spans(Some(&snap), 60, STALE);
        assert!(
            !spans.iter().any(|s| s.style.fg == Some(MUTED)),
            "must not dim a fresh snapshot"
        );
    }

    // `top_row_spans`'s priority rule, exercised directly as a pure function
    // rather than only through the full-TUI render harness in
    // src/tui/tests/budget.rs::render_glue (which still covers the
    // used_width/width_budget glue those tests exist for).
    mod top_row_spans_tests {
        use super::*;
        use crate::models::budget::BudgetWindow;

        fn full_snapshot(model: Option<&str>) -> BudgetSnapshot {
            BudgetSnapshot {
                five_hour: Some(BudgetWindow {
                    used_percentage: 23.4,
                    resets_at: 8040,
                }),
                seven_day: Some(BudgetWindow {
                    used_percentage: 41.2,
                    resets_at: 345_600,
                }),
                model: model.map(|m| m.to_string()),
                captured_at: 0,
            }
        }

        fn text_of(spans: &[Span<'static>]) -> String {
            spans.iter().map(|s| s.content.as_ref()).collect()
        }

        #[test]
        fn model_precedes_budget_when_both_fit() {
            let snap = full_snapshot(Some("Sonnet 5"));
            let text = text_of(&top_row_spans(Some(&snap), 0, STALE, 100));
            assert!(text.starts_with("Sonnet 5"), "got {text:?}");
            assert!(text.contains("5h 23%"), "got {text:?}");
            assert!(text.contains("7d 41%"), "got {text:?}");
        }

        #[test]
        fn model_dropped_before_budget_degrades_when_room_is_tight() {
            // The full two-window+countdown budget text is 27 columns; with
            // the model badge (10 columns: "Sonnet 5" + trailing "  ") that's
            // 37, which doesn't fit a width_budget of 30 — so the model must
            // be dropped, and budget must render at full fidelity (still
            // with its countdown) rather than degrade itself instead.
            let snap = full_snapshot(Some("Sonnet 5"));
            let text = text_of(&top_row_spans(Some(&snap), 0, STALE, 30));
            assert!(
                !text.contains("Sonnet 5"),
                "model must be dropped: {text:?}"
            );
            assert!(text.contains("5h 23%"), "got {text:?}");
            assert!(text.contains("7d 41%"), "got {text:?}");
            assert!(
                text.contains('\u{00B7}'),
                "budget must render at full fidelity, not degrade: {text:?}"
            );
        }

        #[test]
        fn no_model_renders_budget_only() {
            let snap = full_snapshot(None);
            let text = text_of(&top_row_spans(Some(&snap), 0, STALE, 100));
            assert!(text.contains("5h 23%"), "got {text:?}");
            assert!(!text.contains("Sonnet"), "got {text:?}");
        }
    }
}
