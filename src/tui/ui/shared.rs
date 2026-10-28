use crate::palette::{
    CURSOR_BORDER, FG, GREEN, MUTED, MUTED_LIGHT, RED, SELECT_ALL_HIGHLIGHT_BG, YELLOW,
};

use crate::models::{FeedRole, Staleness};
use crate::tui::types::{FoldedHeader, SectionRef};
use crate::tui::{App, RepoFilterMode, ViewMode};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph},
    Frame,
};
use std::time::{Duration, Instant};

/// Returns (text, color) for the 1-line refresh status row shown on Review and Security boards.
///
/// Thresholds are relative to `interval` so both boards stay consistent regardless of their
/// different poll rates.
pub fn refresh_status(
    last_fetch: Option<Instant>,
    loading: bool,
    interval: Duration,
) -> (String, Color) {
    if loading {
        return ("Refreshing...  [r] refresh".to_string(), MUTED);
    }
    let Some(last) = last_fetch else {
        return ("Never fetched  [r] refresh".to_string(), MUTED);
    };
    let elapsed = last.elapsed();
    let elapsed_str = if elapsed.as_secs() < 60 {
        format!("{}s ago", elapsed.as_secs())
    } else {
        format!(
            "{}m {}s ago",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60
        )
    };
    let text = format!("Updated {elapsed_str}  [r] refresh");
    let color = if elapsed >= interval * 4 {
        RED
    } else if elapsed >= interval * 2 {
        YELLOW
    } else {
        FG
    };
    (text, color)
}

/// Map a staleness tier to a terminal color.
/// Uses indexed terminal colors (not palette constants) so these adapt to the
/// user's terminal theme rather than being locked to Tokyo Night RGB values.
pub(in crate::tui::ui) fn staleness_color(staleness: Staleness) -> Color {
    match staleness {
        Staleness::Fresh => Color::Green,
        Staleness::Aging => Color::Yellow,
        Staleness::Stale => Color::Red,
    }
}

/// A `Block` with all four sides and rounded corners, border-styled with `color`.
/// Callers chain `.title(...)` / `.title_style(...)` / `.style(...)` as needed.
pub(in crate::tui::ui) fn rounded_block(color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
}

/// A fully-bordered block whose border and title share `color`, with the title
/// bolded — the frame every overlay in this codebase draws.
///
/// [`rounded_block`] is the untitled `BorderType::Rounded` case; pass
/// `BorderType::Rounded` here when a rounded overlay also wants a title.
pub(in crate::tui::ui) fn titled_block(
    color: Color,
    border_type: BorderType,
    title: String,
) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(color))
        .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
}

/// The style set for keyed hint rows — the `[key] description` tables the help
/// and repo-filter overlays are built from.
///
/// `accent` covers everything that should stand out against the body text:
/// key glyphs, section headers, and the selection cursor. They were three
/// separately-declared locals with byte-identical values before, in two
/// different files; one field means a recolour cannot make them disagree.
pub(in crate::tui::ui) struct HintStyles {
    pub accent: Style,
    pub desc: Style,
    pub note: Style,
}

impl HintStyles {
    pub fn new(accent_color: Color) -> Self {
        Self {
            accent: Style::default()
                .fg(accent_color)
                .add_modifier(Modifier::BOLD),
            desc: Style::default().fg(MUTED_LIGHT),
            note: Style::default().fg(MUTED),
        }
    }
}

/// Rows left for a scrolling list once `reserved` rows of surrounding chrome
/// are taken out of `total`, floored at one so a list is never invisible.
///
/// Paired with [`scroll_offset`]: every caller computes this, then feeds it
/// straight in. Keeping them adjacent means a change to the floor lands on both.
pub(in crate::tui::ui) fn visible_rows(total: usize, reserved: usize) -> usize {
    total.saturating_sub(reserved).max(1)
}

/// Left edge of a `width`-wide box centred horizontally inside `area`.
///
/// Saturating so an over-wide box pins to `area.x` rather than wrapping.
pub(in crate::tui::ui) fn centered_x(area: Rect, width: u16) -> u16 {
    area.x + area.width.saturating_sub(width) / 2
}

/// A `width` × `height` `Rect` centred inside `area`, never larger than it.
///
/// The single home for the centred-overlay offset arithmetic that the error,
/// help and repo-filter overlays all need. Overlays that pin one axis
/// (the tree pickers pin their top edge) use [`centered_x`] directly.
///
/// Every caller sizes itself as a clamped percentage of the board, and those
/// clamps have floors (the help overlay's is 25 rows) that exceed a short
/// terminal — so the requested box is capped to `area` here rather than at each
/// call site. [`open_overlay`] repeats the guard against the real buffer.
pub(in crate::tui::ui) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        centered_x(area, width),
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// Open an overlay: clear `popup_area`, draw `block` into it, and return the
/// block's inner area for the caller's content.
///
/// Keeps the clear-then-block order (and the inner-area derivation) in one
/// place so every overlay frames its content identically.
///
/// `popup_area` is intersected with the frame first. `Frame::render_widget`
/// does no clipping of its own and `Clear` writes every cell it is given, so an
/// overlay whose height floor exceeds the terminal's would otherwise panic the
/// render thread with "index outside of buffer". Because every overlay's
/// content rect derives from the returned inner area, one guard here covers the
/// whole class — including the top-pinned tree pickers, which `centered_rect`
/// never sees.
pub(in crate::tui::ui) fn open_overlay<'a>(
    frame: &mut Frame,
    popup_area: Rect,
    block: Block<'a>,
) -> Rect {
    let popup_area = popup_area.intersection(frame.area());
    frame.render_widget(Clear, popup_area);
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    inner
}

/// First visible index for a list of `count` items showing `visible` rows with
/// the cursor at `cursor`.
///
/// The window scrolls only as far as needed to keep the cursor on the last
/// visible row, and never past the end of the list. Shared by the repo-path
/// picker (`input_form`) and the repo-filter overlay, which previously carried
/// verbatim copies of this formula.
pub(in crate::tui::ui) fn scroll_offset(cursor: usize, count: usize, visible: usize) -> usize {
    if visible == 0 || count <= visible {
        0
    } else {
        cursor.saturating_sub(visible - 1).min(count - visible)
    }
}

pub use crate::epic_header::{fair_truncate_segments, truncate, BREADCRUMB_SEPARATOR};

/// Compact indicator for an epic's feed routing role, shown in the epic header.
/// `None` for ordinary epics (`FeedRole::None`); `Some("role:<role>  ")` for
/// managed feed epics so the routing parent and its role sub-epics are
/// identifiable at a glance.
/// Total codepoint width of a span sequence — the measure every top-row
/// degradation decision (budget's own levels, and the model-vs-budget
/// priority in `model_indicator::top_row_spans`) fits against a width budget.
pub(in crate::tui::ui) fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

pub(in crate::tui::ui) fn feed_role_label(role: FeedRole) -> Option<String> {
    match role {
        FeedRole::None => None,
        other => Some(format!("role:{}  ", other.as_str())),
    }
}

pub(in crate::tui::ui) fn render_top_indicators(frame: &mut Frame, app: &App, area: Rect) {
    let mut parts: Vec<Span> = Vec::new();
    // The store this board is connected to, in every state and in the same
    // muted style as the other badges (sync.allium: ConnectionIndicator,
    // `TheStoreIsAlwaysNamed`). A board on the wrong store otherwise looks
    // exactly like a healthy one (task #28710).
    if let Some(server) = app.store_server.as_deref() {
        parts.push(Span::styled(
            format!("store {}  ", store_address_label(server)),
            Style::default().fg(MUTED),
        ));
    }
    // Auto dispatch indicator — only in epic view
    if let ViewMode::Epic { epic_id, .. } = app.view_mode() {
        if let Some(epic) = app.epics().iter().find(|e| e.id == *epic_id) {
            let (label, style) = if epic.auto_dispatch {
                ("auto dispatch [U]  ", Style::default().fg(GREEN))
            } else {
                ("manual dispatch [U]  ", Style::default().fg(MUTED))
            };
            parts.push(Span::styled(label, style));

            // Feed routing role — for managed feed epics (parent + role sub-epics)
            if let Some(role_label) = feed_role_label(epic.feed_role) {
                parts.push(Span::styled(role_label, Style::default().fg(MUTED)));
            }

            // Append-only marker — read-only, and shown ONLY when set
            // (epics.allium: AppendOnlyEpicIndicator). No bracketed key hint:
            // a bracket here means "press this to toggle", and the OFF
            // direction of this flag hands the next feed cycle licence to
            // remove every accumulated task the emission does not name
            // (feeds.allium: AppendOnlyFeed), so it is wiring, not a keypress.
            if epic.feed_append_only {
                parts.push(Span::styled("append-only  ", Style::default().fg(GREEN)));
            }

            // Group-by-repo indicator — shown for all epics
            let (label, style) = if epic.group_by_repo {
                ("group:on [R]  ", Style::default().fg(GREEN))
            } else {
                ("group:off [R]  ", Style::default().fg(MUTED))
            };
            parts.push(Span::styled(label, style));
        }
    }
    if !app.repo_filter().is_empty() {
        let active = app.repo_filter().len();
        let total = app.board.repo_paths.len();
        let label = match app.repo_filter_mode() {
            RepoFilterMode::Include => format!("[{active}/{total} repos]  "),
            RepoFilterMode::Exclude => format!("[excl {active}/{total} repos]  "),
        };
        parts.push(Span::styled(label, Style::default().fg(MUTED)));
    }
    if app.notifications_enabled() {
        parts.push(Span::styled("\u{1F514} [N]", Style::default().fg(YELLOW)));
    } else {
        parts.push(Span::styled("\u{1F515} [N]", Style::default().fg(MUTED)));
    }

    // The model and budget indicators are prepended so they sit left of
    // everything else, and are given only the width the existing badges
    // leave over — they are never allowed to push those badges off-screen
    // (dispatch.allium: TokenBudgetIndicator/ActiveModelIndicator degradation).
    let used_width = spans_width(&parts);
    // `used_width` sums codepoints, but the bell/no-bell badge
    // ("\u{1F514} [N]" / "\u{1F515} [N]") is one codepoint narrower than its
    // display width — the emoji renders double-width under ratatui's
    // unicode-width measurement, which `Paragraph`/`Alignment::Right` actually
    // use to lay out and truncate the line. Reserve one extra column here so
    // that undercount can never make the composed line one column too wide
    // and get its right edge (the pre-existing badges) clipped. Do not remove
    // this as a stray off-by-one — see docs/specs/dispatch.allium's
    // `@guarantee DegradesWhenRowTooNarrow`.
    let indicators_width_budget = (area.width as usize).saturating_sub(used_width + 1);
    let indicators = super::model_indicator::top_row_spans(
        app.budget.as_ref(),
        chrono::Utc::now().timestamp(),
        crate::tui::BUDGET_STALE_AFTER,
        indicators_width_budget,
    );
    parts.splice(0..0, indicators);

    let line = Line::from(parts);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Right), area);
}

/// An open sub-status section header — decoration, so it never draws a cursor.
///
/// `first` — when true, omits the leading blank line so the top of the column
/// doesn't have an awkward gap before the very first group.
pub(in crate::tui::ui) fn render_substatus_header(
    at: &SectionRef,
    first: bool,
) -> ListItem<'static> {
    section_header_item(
        format!("  \u{2500}\u{2500} {} ", at.section.header_label()),
        first,
        false,
    )
}

/// A folded section's header: the label, the count of cards it is hiding, and
/// a fold marker.
///
/// The marker is U+22EF and not the U+25B8 triangle, which already means
/// "focused column" and "this epic has a plan" elsewhere on the board — a
/// third meaning would make all three ambiguous. An ellipsis reads as "more
/// here, elided" on its own.
///
/// Only a folded header can hold the cursor, which is why this is the one of
/// the two that takes `is_cursor` (board-layout.allium: "Collapsed Sections").
pub(in crate::tui::ui) fn render_folded_section_header(
    header: &FoldedHeader,
    first: bool,
    is_cursor: bool,
) -> ListItem<'static> {
    section_header_item(
        format!(
            "  \u{2500}\u{2500} {} ({}) \u{22ef}",
            header.at.section.header_label(),
            header.hidden
        ),
        first,
        is_cursor,
    )
}

/// The row both section headers are drawn as, so their spacing and their
/// cursor treatment cannot drift apart.
///
/// A card shows the cursor on its frame (`resolve_frame_color`). A header has
/// no frame, and a brightness step alone on one already-bold line is too small
/// to find — so it takes the cursor near-white on the text *and* the neutral
/// lift the select-all checkbox uses behind it. Hue-free, like every other
/// cursor on this board.
pub(in crate::tui::ui) fn section_header_item(
    text: String,
    first: bool,
    is_cursor: bool,
) -> ListItem<'static> {
    let mut style = Style::default().fg(FG).add_modifier(Modifier::BOLD);
    if is_cursor {
        style = style.fg(CURSOR_BORDER).bg(SELECT_ALL_HIGHLIGHT_BG);
    }
    let line = Line::from(Span::styled(text, style));
    if first {
        ListItem::new(vec![line])
    } else {
        ListItem::new(vec![Line::raw(""), line])
    }
}

/// Push a keybinding hint as styled spans.
///
/// When the key is a single char matching the label's first letter (e.g. `d` / `dispatch`),
/// renders the compact `[d]ispatch` form. Otherwise renders `[key] label`.
pub(in crate::tui::ui) fn push_hint_spans(
    spans: &mut Vec<Span<'static>>,
    key: &str,
    label: &str,
    key_color: Color,
    label_style: Style,
) {
    let mut key_chars = key.chars();
    let key_char = key_chars.next();
    let key_is_single = key_char.is_some() && key_chars.next().is_none();
    let mut label_chars = label.chars();
    let label_first = label_chars.next();
    let can_embed = key_is_single
        && label_first
            .zip(key_char)
            .is_some_and(|(l, k)| l.eq_ignore_ascii_case(&k));

    spans.push(Span::styled(
        format!("[{key}]"),
        Style::default().fg(key_color).add_modifier(Modifier::BOLD),
    ));
    let label_text = if can_embed {
        format!("{}  ", label_chars.as_str())
    } else {
        format!(" {label}  ")
    };
    spans.push(Span::styled(label_text, label_style));
}

/// Render a single-line text field with the caret drawn as a reversed block
/// cell at its position, replacing the old trailing-`_` fake caret.
///
/// `prefix` is the (already-styled-as-`base`) label, e.g. `"  Title: "`.
/// `caret` is a **char** index into `buffer` (`0..=chars().count()`).
/// `value_width` is the number of terminal columns available for the value
/// after the prefix; when the buffer is longer, the value is horizontally
/// scrolled so the caret stays visible (the caret is anchored near the right
/// edge of the window). A trailing space cell carries the caret when it sits at
/// the end of the buffer.
pub(in crate::tui) fn caret_line(
    prefix: String,
    buffer: &str,
    caret: usize,
    value_width: usize,
    base: Style,
) -> Line<'static> {
    let caret_style = base.add_modifier(Modifier::REVERSED);
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    let caret = caret.min(len);
    let width = value_width.max(1);

    // Lay out one cell per char, plus a trailing blank cell for the caret when
    // it sits at the end. Scroll so the caret is always inside [start, end).
    // The window right-anchors the caret when scrolled: as the user arrows left
    // past the left edge, `start` decreases one cell at a time (the caret sits
    // at the rightmost visible cell, then the text shifts). Simple and always
    // keeps the caret visible; no leading margin before scrolling kicks in.
    let total = if caret == len { len + 1 } else { len };
    let start = if total > width {
        caret.saturating_sub(width - 1)
    } else {
        0
    };
    let end = (start + width).min(total);

    let mut before = String::new();
    let mut after = String::new();
    let mut caret_cell = ' ';
    // Index loop is intentional: `i` ranges up to `len` (one past the last char)
    // to place the caret cell when the caret is at the end, and is compared to
    // `caret`/`len` — not just used to index — so an iterator rewrite would drop
    // the at-end cell.
    #[allow(clippy::needless_range_loop)]
    for i in start..end {
        if i < caret {
            before.push(chars[i]);
        } else if i == caret {
            caret_cell = if i < len { chars[i] } else { ' ' };
        } else {
            after.push(chars[i]);
        }
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix, base));
    }
    if !before.is_empty() {
        spans.push(Span::styled(before, base));
    }
    spans.push(Span::styled(caret_cell.to_string(), caret_style));
    if !after.is_empty() {
        spans.push(Span::styled(after, base));
    }
    Line::from(spans)
}

/// The store address as the top row shows it: the scheme and a trailing slash
/// are noise next to the host and port that tell two stores apart.
fn store_address_label(server: &str) -> &str {
    let trimmed = server.trim().trim_end_matches('/');
    trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed)
}

#[cfg(test)]
mod tests;
