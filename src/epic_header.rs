//! The epic group header and the end-of-epics rule, drawn alike by the
//! board's flattened columns (`board-layout.allium`, "Flattening") and the
//! agent-tree pane's agents section (`agent-tree.allium:
//! AgentListGroupsByEpic`), so neither reaches into the other.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::models::EpicId;
use crate::palette::{MUTED, PURPLE};
use crate::text::{fair_truncate_segments, BREADCRUMB_SEPARATOR};

/// `── #<id> <breadcrumb> ───`, filling `width`: the rule and id muted, the
/// breadcrumb purple. `breadcrumb` is the epic's ancestor titles, root first;
/// when it does not fit, every segment keeps a visible prefix
/// ([`fair_truncate_segments`]).
pub(crate) fn epic_header_line<S: AsRef<str>>(
    id: EpicId,
    breadcrumb: &[S],
    width: u16,
) -> Line<'static> {
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
