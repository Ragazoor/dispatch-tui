//! The epic group header and the end-of-epics rule, drawn alike by the
//! board's flattened columns (`board-layout.allium`, "Flattening") and the
//! agent-tree pane's agents section (`agent-tree.allium:
//! AgentListGroupsByEpic`), so neither reaches into the other.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::models::EpicId;
use crate::palette::{MUTED, PURPLE};

/// `── #<id> <breadcrumb> ───`, filling `width`: the rule and id muted, the
/// breadcrumb purple. `breadcrumb` is the epic's ancestor titles, root first;
/// when it does not fit, every segment keeps a visible prefix
/// ([`fair_truncate_segments`]).
pub(crate) fn epic_header_line(id: EpicId, breadcrumb: &[&str], width: u16) -> Line<'static> {
    let id_prefix = format!("#{id} ");
    // "── " + text + " " + rule reserves the text's length plus 5, beside the
    // id prefix's own width.
    let reserved = 5 + id_prefix.chars().count();
    let budget = usize::from(width).saturating_sub(reserved);
    let title = fair_truncate_segments(breadcrumb, budget, BREADCRUMB_SEPARATOR);
    let rule_count = usize::from(width).saturating_sub(title.chars().count() + reserved);
    Line::from(vec![
        Span::styled("\u{2500}\u{2500} ", Style::default().fg(MUTED)),
        Span::styled(id_prefix, Style::default().fg(MUTED)),
        Span::styled(title, Style::default().fg(PURPLE)),
        Span::styled(
            format!(" {}", "\u{2500}".repeat(rule_count)),
            Style::default().fg(MUTED),
        ),
    ])
}

/// `╌╌ · ╌╌╌`, filling `width`, all muted: where the epic groups end and the
/// epicless tasks begin. Dashed so it never reads as an epic header or a
/// card's solid rule.
pub(crate) fn end_of_epics_line(width: u16) -> Line<'static> {
    const DASH: &str = "\u{254C}"; // ╌ BOX DRAWINGS LIGHT DOUBLE DASH HORIZONTAL
    let prefix = format!("{} \u{00B7} ", DASH.repeat(2));
    let fill_count = usize::from(width).saturating_sub(prefix.chars().count());
    Line::from(Span::styled(
        format!("{prefix}{}", DASH.repeat(fill_count)),
        Style::default().fg(MUTED),
    ))
}

/// Truncate a string to at most `max` characters, appending "…" if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

/// Separator joining ancestor-breadcrumb segments, shared by the flat-view
/// epic header (`fair_truncate_segments`) and the epic-view border title.
pub const BREADCRUMB_SEPARATOR: &str = " › ";

/// Join `segments` with `sep`, truncating so the joined result fits within
/// `budget` display characters while keeping a visible prefix of every segment.
///
/// Water-filling: segments shorter than the fair share are kept whole and their
/// unused width is redistributed to longer segments. Falls back to a
/// whole-string truncate when the budget is too small to give every segment at
/// least a one-char prefix plus ellipsis.
pub fn fair_truncate_segments(segments: &[&str], budget: usize, sep: &str) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let n = segments.len();
    let sep_cost = (n - 1) * sep.chars().count();
    let lens: Vec<usize> = segments.iter().map(|s| s.chars().count()).collect();
    let name_budget = budget.saturating_sub(sep_cost);

    // Everything already fits (separators included).
    if sep_cost + lens.iter().sum::<usize>() <= budget {
        return segments.join(sep);
    }
    // Too narrow to give each segment ≥1 char + ellipsis: truncate the whole string.
    if name_budget < 2 * n {
        return truncate(&segments.join(sep), budget);
    }

    // Water-fill: lock any segment at or under the current fair share, return its
    // slack to the pool, and repeat until only oversized segments remain.
    let mut alloc = vec![0usize; n];
    let mut settled = vec![false; n];
    let mut pool = name_budget;
    let mut remaining = n;
    loop {
        let fair = pool / remaining;
        let mut locked_any = false;
        for i in 0..n {
            if !settled[i] && lens[i] <= fair {
                alloc[i] = lens[i];
                settled[i] = true;
                pool -= lens[i];
                remaining -= 1;
                locked_any = true;
            }
        }
        if !locked_any || remaining == 0 {
            break;
        }
    }
    // Split the remaining pool across still-oversized segments; leftmost get the
    // remainder chars first.
    if let Some(base) = pool.checked_div(remaining) {
        let mut extra = pool % remaining;
        for i in 0..n {
            if !settled[i] {
                alloc[i] = base + usize::from(extra > 0);
                extra = extra.saturating_sub(1);
            }
        }
    }

    segments
        .iter()
        .enumerate()
        .map(|(i, s)| truncate(s, alloc[i]))
        .collect::<Vec<_>>()
        .join(sep)
}
