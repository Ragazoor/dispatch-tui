//! The epic group header and the end-of-epics rule, drawn alike by the
//! board's flattened columns (`board-layout.allium`, "Flattening") and the
//! agent-tree pane's agents section (`agent-tree.allium:
//! AgentListGroupsByEpic`), so neither reaches into the other.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::palette::{MUTED, PURPLE};
use crate::text::truncate;

/// Columns of indent per level of epic depth (`board-layout.allium`:
/// `config.epic_indent_step`).
pub(crate) const EPIC_INDENT_STEP: usize = 2;

/// The depth past which the indent stops growing (`board-layout.allium`:
/// `config.epic_indent_max_depth`).
pub(crate) const EPIC_INDENT_MAX_DEPTH: usize = 3;

/// The indent, in columns, of an epic group's rows at `depth`
/// (`board-layout.allium`: `EpicGroupTree.indent`).
pub(crate) fn epic_indent(depth: usize) -> usize {
    depth.min(EPIC_INDENT_MAX_DEPTH) * EPIC_INDENT_STEP
}

/// `<title> ───`, indented by [`epic_indent`]`(depth)` and filling `width`
/// less one cell: the rule muted, the title purple. No rule is drawn left of
/// the title and no id is shown (`board-visuals.allium:
/// EpicsAreShownByTitleNeverById`), so the indent is what shows the nesting. `title` is the epic's
/// own title, never its ancestors' (`board-layout.allium`, "Flattening"); it
/// ends in an ellipsis when it does not fit.
pub(crate) fn epic_header_line(title: &str, width: u16, depth: usize) -> Line<'static> {
    let indent = epic_indent(depth);
    // The row is indent + title + " " + rule, one cell short of `width` so
    // its right end lines up with a card's right margin; the title leaves
    // room for at least one rule cell.
    let fixed = indent + 2;
    let budget = usize::from(width).saturating_sub(fixed + 1);
    let title = truncate(title, budget);
    let rule_count = usize::from(width).saturating_sub(fixed + title.chars().count());
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
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
