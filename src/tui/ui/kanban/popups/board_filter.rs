//! Board filter overlay: the Repos, Epics and Work packages tabs.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{BorderType, Paragraph},
    Frame,
};

use crate::palette::{CYAN, MUTED, YELLOW};
use crate::tui::types::{FilterEpicMark, FilterEpicRow, FilterTab};
use crate::tui::ui::shared::{
    centered_rect, open_overlay, scroll_offset, titled_block, visible_rows, HintStyles,
};
use crate::tui::{App, InputMode};

/// Top and bottom border rows of the popup block.
const BORDER_ROWS: usize = 2;

/// Shortest popup worth drawing, when the board has the room for it.
const MIN_POPUP_HEIGHT: u16 = 8;

/// The most marker rows the repo list can add: "↑ N more" and "↓ N more".
const MAX_MARKER_ROWS: usize = 2;

/// Everything the overlay's geometry needs, derived once.
struct FilterLayout {
    popup_area: Rect,
    /// List rows that fit in the list window (at least one).
    visible: usize,
    /// Index of the first row shown.
    scroll: usize,
    /// Whether the "↑ N more" marker row is drawn above the list.
    show_scroll_up: bool,
    /// Whether the "↓ N more" marker row is drawn below the list.
    show_scroll_down: bool,
}

/// Derive the overlay's popup rect and its scrolling list window.
///
/// `non_list_rows` is the number of content rows the caller has *already built*
/// — the tab header, any search row and the blank spacer above the list, plus
/// the footer block below it. It is counted from those line vectors rather
/// than hand-tallied, so the popup height (which adds it) and the visible-row
/// window (which subtracts it) cannot drift
/// apart, and neither can drift from what the overlay actually draws. The two
/// hand-counted literals this replaced (`+7` and `+5`, budgeted from opposite
/// directions with nothing checking they agreed) were exactly that hazard.
///
/// The rows left over after `non_list_rows` are the list's whole budget, and
/// the scroll markers come out of it too — see [`settle_list_window`], which
/// also decides whether each marker is drawn at all. Deriving that here rather
/// than in [`append_windowed_list`] is what keeps the footer on screen: a marker the
/// renderer added on its own would be a row nothing had reserved.
///
/// `list_len` is the showing tab's row count and `cursor` its cursor;
/// `tallest_list` is the longest list any tab can show, which sizes the popup
/// so it keeps one height across tabs.
fn filter_layout(
    area: Rect,
    list_len: usize,
    tallest_list: usize,
    non_list_rows: usize,
    cursor: usize,
) -> FilterLayout {
    let wanted_height = tallest_list.max(list_len) + non_list_rows + BORDER_ROWS;
    // The board keeps two rows of margin above and below the popup. On a board
    // too short to honour `MIN_POPUP_HEIGHT`, the ceiling wins — a bare
    // `.clamp(MIN_POPUP_HEIGHT, ceiling)` would panic on `min > max`, and
    // render code must never panic.
    let ceiling = area.height.saturating_sub(4);
    let popup_height = u16::try_from(wanted_height)
        .unwrap_or(u16::MAX)
        .clamp(MIN_POPUP_HEIGHT.min(ceiling), ceiling);
    // In u32: `width * 70` overflows u16 on a terminal past 936 columns.
    let popup_width = u16::try_from(u32::from(area.width) * 70 / 100)
        .unwrap_or(u16::MAX)
        .clamp(30, 60);

    let content_height = popup_height.saturating_sub(BORDER_ROWS as u16) as usize;
    let budget = content_height.saturating_sub(non_list_rows);
    let window = settle_list_window(budget, list_len, cursor);

    FilterLayout {
        popup_area: centered_rect(area, popup_width, popup_height),
        visible: window.visible,
        scroll: window.scroll,
        show_scroll_up: window.up,
        show_scroll_down: window.down,
    }
}

/// The list's window: the rows it shows and the markers bracketing them.
struct ListWindow {
    visible: usize,
    scroll: usize,
    up: bool,
    down: bool,
}

impl ListWindow {
    fn markers(&self) -> usize {
        usize::from(self.up) + usize::from(self.down)
    }
}

/// Settle the repo window against its own scroll markers.
///
/// The `↑ N more` / `↓ N more` rows are content like any other, so they have to
/// come out of `budget` — but sizing them is circular: whether a marker appears
/// depends on the scroll offset, which depends on how many repos are visible,
/// which depends on how many marker rows were reserved.
///
/// Resolved as a fixed point over the reservation, smallest first: take the
/// first `reserved` whose window draws no more markers than it set aside, so
/// `visible + markers <= budget` holds. Reserving both markers always passes
/// that test — there are only two — so the search terminates, and it is the
/// fallback. Trying smaller reservations first matters: at either end of a long
/// list only one marker draws, and settling on one keeps a repo row that a
/// blanket two-row reservation would have spent on nothing.
fn settle_list_window(budget: usize, list_len: usize, cursor: usize) -> ListWindow {
    let window_for = |reserved: usize| {
        let visible = visible_rows(budget, reserved);
        let scroll = scroll_offset(cursor, list_len, visible);
        ListWindow {
            visible,
            scroll,
            up: scroll > 0,
            down: list_len > scroll + visible,
        }
    };

    let mut window = (0..MAX_MARKER_ROWS)
        .map(|reserved| (reserved, window_for(reserved)))
        .find(|(reserved, candidate)| candidate.markers() <= *reserved)
        .map(|(_, candidate)| candidate)
        .unwrap_or_else(|| window_for(MAX_MARKER_ROWS));

    // `visible_rows` floors at one row, so on a board whose chrome already eats
    // the whole budget the repo row alone overflows and no reservation can
    // help. Drop the markers there rather than pile two more rows on top: the
    // repo the cursor is on is the more useful of the three.
    if window.visible + window.markers() > budget {
        window.up = false;
        window.down = false;
    }
    window
}

pub(in crate::tui::ui::kanban) fn render_board_filter_overlay(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) {
    let is_filter_mode = matches!(
        app.mode(),
        InputMode::BoardFilter | InputMode::BoardFilterSearch | InputMode::ConfirmDeleteRepoPath
    );
    if !is_filter_mode {
        return;
    }

    let styles = HintStyles::new(CYAN);
    let tab = app.filter_overlay.tab;
    // Built once per frame: the Epics tab and the tab header both read it.
    let epic_rows = app.filter_epic_rows();
    let (rows, cursor) = match tab {
        FilterTab::Repos => (repo_rows(app, &styles), app.input.repo_cursor),
        FilterTab::Epics => (
            epic_list_rows(&epic_rows, app.filter_overlay.epic_cursor, &styles),
            app.filter_overlay.epic_cursor,
        ),
        FilterTab::Packages => (
            package_rows(app, &styles),
            app.filter_overlay.package_cursor,
        ),
    };
    // Sized by the longest list any tab can show, so switching tabs does not
    // make the popup jump.
    let tallest = app
        .repo_paths()
        .len()
        .max(app.board.epics.len())
        .max(app.board.work_packages.len());

    // Build everything except the list first, so the layout can be sized from
    // the rows that will actually be drawn rather than from a tally that has
    // to be kept in step by hand.
    let mut header = vec![tab_header_line(app, &styles)];
    if let Some(line) = epic_search_line(app, &styles) {
        header.push(line);
    }
    header.push(Line::from(""));

    let mut footer = vec![Line::from("")];
    append_help_lines(&mut footer, app, &styles);

    let layout = filter_layout(
        area,
        rows.len(),
        tallest,
        header.len() + footer.len(),
        cursor,
    );

    let block = titled_block(CYAN, BorderType::Double, " Board Filter ".to_string());

    let mut lines = header;
    append_windowed_list(&mut lines, rows, &layout, &styles);
    lines.append(&mut footer);

    let inner = open_overlay(frame, layout.popup_area, block);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `Repos (include) │ Epics ·2 │ Work packages`, the showing tab highlighted;
/// `·N` counts the tab's hidden rows. Kept short to fit the popup's 60-column
/// cap with both counts showing.
fn tab_header_line<'a>(app: &'a App, styles: &HintStyles) -> Line<'a> {
    let mut spans = vec![Span::raw("  ")];
    for (i, tab) in FilterTab::ALL.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", styles.note));
        }
        let detail = match tab {
            FilterTab::Repos => format!(" ({})", app.repo_filter_mode().as_str()),
            FilterTab::Epics => hidden_suffix(app.live_hidden_counts().0),
            FilterTab::Packages => hidden_suffix(app.live_hidden_counts().1),
        };
        let style = if tab == app.filter_overlay.tab {
            styles
                .accent
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            styles.desc
        };
        spans.push(Span::styled(format!(" {}{detail} ", tab.label()), style));
    }
    Line::from(spans)
}

fn hidden_suffix(count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        format!(" ·{count}")
    }
}

/// The Epics tab's title-search row, while a query stands or is being typed.
fn epic_search_line<'a>(app: &'a App, styles: &HintStyles) -> Option<Line<'a>> {
    let typing = app.mode() == &InputMode::BoardFilterSearch;
    let query = &app.filter_overlay.epic_query;
    if app.filter_overlay.tab != FilterTab::Epics || (!typing && query.is_empty()) {
        return None;
    }
    let caret = if typing { "_" } else { "" };
    Some(Line::from(vec![
        Span::styled("  /", styles.accent),
        Span::styled(format!("{query}{caret}"), styles.desc),
    ]))
}

/// The repo checkbox list, every row. The cursor row carries a `►`; the others
/// their `1`-`9` shortcut number.
fn repo_rows<'a>(app: &'a App, styles: &HintStyles) -> Vec<Line<'a>> {
    let broken_style = Style::default().fg(MUTED);
    let cursor = app.input.repo_cursor;
    app.repo_paths()
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let checked = if app.repo_filter().contains(path) {
                "x"
            } else {
                " "
            };
            let is_broken = app.broken_repo_paths.contains(path);
            let broken_mark = if is_broken { " [!]" } else { "" };
            if i == cursor {
                let style = if is_broken {
                    broken_style
                } else {
                    styles.accent
                };
                Line::from(vec![
                    Span::styled("  ►", style),
                    Span::styled(format!(" [{checked}] {path}{broken_mark}"), style),
                ])
            } else {
                let num = i + 1;
                let style = if is_broken { broken_style } else { styles.desc };
                Line::from(vec![
                    Span::styled(
                        format!("  {num}"),
                        if is_broken {
                            broken_style
                        } else {
                            styles.accent
                        },
                    ),
                    Span::styled(format!(". [{checked}] {path}{broken_mark}"), style),
                ])
            }
        })
        .collect()
}

/// The epic tree: `[x]` hidden, `[-]` hidden because an ancestor is, `[ ]`
/// shown; children indented two spaces per level.
fn epic_list_rows<'a>(rows: &[FilterEpicRow], cursor: usize, styles: &HintStyles) -> Vec<Line<'a>> {
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let mark = match row.mark {
                FilterEpicMark::Shown => "[ ]",
                FilterEpicMark::Hidden => "[x]",
                FilterEpicMark::HiddenByParent => "[-]",
            };
            let indent = "  ".repeat(row.depth);
            let style = if i == cursor {
                styles.accent
            } else if row.mark == FilterEpicMark::HiddenByParent {
                Style::default().fg(MUTED)
            } else {
                styles.desc
            };
            let pointer = if i == cursor { "  ►" } else { "   " };
            Line::from(vec![
                Span::styled(pointer, style),
                Span::styled(
                    format!(" {indent}{mark} #{} {}", row.id.0, row.title),
                    style,
                ),
            ])
        })
        .collect()
}

/// One row per work package: `[x]` hidden, `[ ]` shown.
fn package_rows<'a>(app: &'a App, styles: &HintStyles) -> Vec<Line<'a>> {
    let cursor = app.filter_overlay.package_cursor;
    app.board
        .work_packages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mark = if app.hidden_packages().contains(&p.id) {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if i == cursor {
                styles.accent
            } else {
                styles.desc
            };
            let pointer = if i == cursor { "  ►" } else { "   " };
            Line::from(vec![
                Span::styled(pointer, style),
                Span::styled(format!(" {mark} {}", p.title), style),
            ])
        })
        .collect()
}

/// The windowed slice of `rows`, bracketed by "↑ N more" / "↓ N more" markers
/// when the window doesn't cover the whole list. Whether a marker is drawn is
/// the layout's call, not a second derivation here: the marker rows come out
/// of the same budget the footer does.
fn append_windowed_list<'a>(
    lines: &mut Vec<Line<'a>>,
    rows: Vec<Line<'a>>,
    layout: &FilterLayout,
    styles: &HintStyles,
) {
    let total = rows.len();
    if total == 0 {
        lines.push(Line::from(Span::styled("  (nothing to list)", styles.note)));
        return;
    }
    if layout.show_scroll_up {
        lines.push(Line::from(Span::styled(
            format!("  ↑ {} more", layout.scroll),
            styles.note,
        )));
    }
    lines.extend(rows.into_iter().skip(layout.scroll).take(layout.visible));
    if layout.show_scroll_down {
        let remaining = total.saturating_sub(layout.scroll + layout.visible);
        lines.push(Line::from(Span::styled(
            format!("  ↓ {} more", remaining),
            styles.note,
        )));
    }
}

/// Footer help, one branch per input mode the overlay can be in.
fn append_help_lines<'a>(lines: &mut Vec<Line<'a>>, app: &'a App, styles: &HintStyles) {
    match app.mode() {
        InputMode::ConfirmDeleteRepoPath => lines.push(delete_repo_path_help_line(app, styles)),
        _ => append_browse_help_lines(lines, app.filter_overlay.tab, styles),
    }
}

fn delete_repo_path_help_line<'a>(app: &'a App, styles: &HintStyles) -> Line<'a> {
    let path_label = app
        .repo_paths()
        .get(app.input.repo_cursor)
        .map(|p| p.as_str())
        .unwrap_or("?");
    Line::from(vec![
        Span::styled(
            format!("  Delete {path_label}?  "),
            Style::default().fg(YELLOW),
        ),
        Span::styled("y", styles.accent),
        Span::styled(": yes  ", styles.note),
        Span::styled("n/Esc", styles.accent),
        Span::styled(": cancel", styles.note),
    ])
}

/// The two-row default footer, the same height on every tab so switching
/// tabs never resizes the popup. The confirm mode renders one row; the layout
/// budget is counted from whichever this produces, not assumed.
fn append_browse_help_lines<'a>(lines: &mut Vec<Line<'a>>, tab: FilterTab, styles: &HintStyles) {
    let hint = |key: &'static str, label: &'static str| {
        [
            Span::styled(key, styles.accent),
            Span::styled(label, styles.note),
        ]
    };
    let first: Vec<Span> = match tab {
        FilterTab::Repos => [hint("  [j/k]", " navigate  "), hint("[Space]", " toggle  ")].concat(),
        FilterTab::Epics => [
            hint("  [Space]", " hide/show  "),
            hint("[d]", " done  "),
            hint("[/]", " search  "),
        ]
        .concat(),
        FilterTab::Packages => [
            hint("  [j/k]", " navigate  "),
            hint("[Space]", " hide/show  "),
        ]
        .concat(),
    };
    let mut second: Vec<Span> = vec![];
    if tab == FilterTab::Repos {
        second.extend(hint("  [i]", " incl/excl  "));
        second.extend(hint("[Tab]", " next tab  "));
    } else {
        second.extend(hint("  [Tab]", " next tab  "));
    }
    second.extend(hint("[q/Esc]", " close"));
    lines.extend([Line::from(first), Line::from(second)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board tall enough that the popup height never hits its clamp.
    const ROOMY: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 60,
    };

    /// With the budget counted from the rows the caller actually built, an
    /// unclamped popup shows every repo with nothing left over — no scroll
    /// markers, no blank filler row. This is the invariant the old `+7`/`+5`
    /// pair of hand-counted literals had no way to guarantee.
    ///
    /// On its own this is close to an identity; the check with teeth is
    /// `repo_filter_renders_its_whole_footer_in_every_mode`, which drives the real
    /// render path and so pins `non_repo_rows` against the lines drawn.
    #[test]
    fn layout_reserves_exactly_the_rows_the_popup_budgets() {
        for repo_count in 1..=12 {
            for non_repo_rows in [5, 6, 8, 10] {
                let l = filter_layout(ROOMY, repo_count, repo_count, non_repo_rows, 0);
                assert_eq!(
                    l.visible, repo_count,
                    "repo_count={repo_count} non_repo_rows={non_repo_rows}"
                );
                assert_eq!(l.scroll, 0);
            }
        }
    }

    #[test]
    fn popup_height_grows_with_the_non_repo_rows() {
        let base = filter_layout(ROOMY, 4, 4, 5, 0).popup_area.height;
        assert_eq!(filter_layout(ROOMY, 4, 4, 8, 0).popup_area.height, base + 3);
        assert_eq!(filter_layout(ROOMY, 4, 4, 6, 0).popup_area.height, base + 1);
    }

    #[test]
    fn popup_is_centered_and_clamped_to_the_board() {
        // 40 repos cannot fit in a 20-row board: height clamps to height-4.
        let area = Rect::new(0, 0, 100, 20);
        let l = filter_layout(area, 40, 40, 5, 0);
        assert_eq!(l.popup_area.height, 16);
        assert_eq!(l.popup_area.width, 60); // 70% of 100 clamped to 60
        assert_eq!(l.popup_area, centered_rect(area, 60, 16));
    }

    #[test]
    fn popup_height_has_a_floor_of_eight_rows() {
        // No repos at all still leaves a usable box.
        assert_eq!(filter_layout(ROOMY, 0, 0, 5, 0).popup_area.height, 8);
    }

    #[test]
    fn window_scrolls_to_keep_the_repo_cursor_visible() {
        let area = Rect::new(0, 0, 100, 20);
        let l = filter_layout(area, 40, 40, 5, 39);
        // Content height 14, minus the 5 non-repo rows → a 9-row budget. The
        // cursor sits on the last repo, so only the "↑ more" marker draws;
        // one row of the budget pays for it and 8 repos fit.
        assert_eq!(l.visible, 8);
        assert_eq!(l.scroll, 40 - 8);
        assert!(l.show_scroll_up);
        assert!(!l.show_scroll_down);
    }

    /// The bug this module's layout exists to prevent: the scroll markers are
    /// content rows like any other, so `visible_repos` plus whichever markers
    /// the layout reports must fit the row budget the popup reserved. Before
    /// the fixed-point settle, a scrolling list drew both markers on top of a
    /// budget that had reserved neither, and the footer fell off the bottom.
    #[test]
    fn scroll_markers_stay_inside_the_row_budget() {
        for board_height in [16, 20, 24, 40] {
            let area = Rect::new(0, 0, 100, board_height);
            for non_repo_rows in [5, 6, 7] {
                for repo_cursor in 0..40 {
                    let l = filter_layout(area, 40, 40, non_repo_rows, repo_cursor);
                    let budget = (l.popup_area.height as usize)
                        .saturating_sub(BORDER_ROWS)
                        .saturating_sub(non_repo_rows);
                    let drawn =
                        l.visible + usize::from(l.show_scroll_up) + usize::from(l.show_scroll_down);
                    assert!(
                        drawn <= budget.max(1),
                        "board_height={board_height} non_repo_rows={non_repo_rows} \
                         repo_cursor={repo_cursor}: drew {drawn} rows into a {budget}-row budget"
                    );
                }
            }
        }
    }

    /// The markers the layout reports are the markers the window implies —
    /// the renderer trusts these flags instead of re-deriving the condition.
    #[test]
    fn marker_flags_match_the_settled_window() {
        let area = Rect::new(0, 0, 100, 20);
        for repo_cursor in 0..40 {
            let l = filter_layout(area, 40, 40, 5, repo_cursor);
            assert_eq!(l.show_scroll_up, l.scroll > 0);
            assert_eq!(l.show_scroll_down, 40 > l.scroll + l.visible);
        }
    }

    /// A list that fits pays nothing for markers it will not draw.
    #[test]
    fn a_list_that_fits_reserves_no_marker_rows() {
        let l = filter_layout(ROOMY, 12, 12, 5, 0);
        assert_eq!(l.visible, 12);
        assert!(!l.show_scroll_up);
        assert!(!l.show_scroll_down);
    }

    #[test]
    fn visible_repos_never_falls_below_one() {
        // A tiny board where the chrome alone exceeds the content height.
        let area = Rect::new(0, 0, 100, 10);
        let l = filter_layout(area, 30, 30, 14, 0);
        assert_eq!(l.visible, 1);
    }

    /// A board too short for `MIN_POPUP_HEIGHT` must yield the tallest popup
    /// that fits, not panic. Render code has no room to unwind.
    #[test]
    fn short_boards_shrink_the_popup_instead_of_panicking() {
        for height in 0..=12u16 {
            let l = filter_layout(Rect::new(0, 0, 100, height), 5, 5, 5, 0);
            assert!(l.popup_area.height <= height.saturating_sub(4));
            assert!(l.visible >= 1);
        }
    }
}
