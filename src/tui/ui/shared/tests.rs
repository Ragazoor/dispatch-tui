use super::*;

/// Concatenate a line's span contents back into a plain string.
fn line_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The reversed caret cell's content (there is exactly one reversed span).
fn caret_cell(line: &Line) -> String {
    line.spans
        .iter()
        .find(|s| s.style.add_modifier.contains(Modifier::REVERSED))
        .map(|s| s.content.to_string())
        .expect("a caret cell")
}

#[test]
fn caret_line_mid_string_highlights_char_at_caret() {
    let line = caret_line("  Title: ".to_string(), "abc", 1, 80, Style::default());
    // full text = prefix + buffer (no scrolling at width 80)
    assert_eq!(line_text(&line), "  Title: abc");
    // caret at index 1 -> highlights 'b'
    assert_eq!(caret_cell(&line), "b");
}

#[test]
fn caret_line_at_end_appends_block_cell() {
    let line = caret_line(String::new(), "ab", 2, 80, Style::default());
    // trailing blank caret cell
    assert_eq!(line_text(&line), "ab ");
    assert_eq!(caret_cell(&line), " ");
}

#[test]
fn caret_line_scrolls_to_keep_caret_visible_at_end() {
    // 20-char buffer, width 10, caret at end: window shows the tail and the
    // caret cell is present.
    let buf = "0123456789abcdefghij";
    let caret = buf.chars().count(); // 20, at end
    let line = caret_line(String::new(), buf, caret, 10, Style::default());
    let text = line_text(&line);
    // exactly `width` cells rendered (9 tail chars + 1 caret space)
    assert_eq!(text.chars().count(), 10);
    // the beginning is scrolled off
    assert!(!text.contains('0'));
    // the caret (blank) cell is the last cell
    assert!(text.ends_with(' '));
    assert_eq!(caret_cell(&line), " ");
}

#[test]
fn caret_line_shows_start_when_caret_at_zero() {
    let buf = "0123456789abcdefghij";
    let line = caret_line(String::new(), buf, 0, 10, Style::default());
    let text = line_text(&line);
    assert!(text.starts_with('0'));
    assert_eq!(caret_cell(&line), "0");
}

// ---- centered_rect / centered_x --------------------------------------

#[test]
fn centered_rect_centers_within_area() {
    let area = Rect::new(0, 0, 100, 40);
    let r = centered_rect(area, 60, 10);
    assert_eq!(r, Rect::new(20, 15, 60, 10));
    // Equal slack on both sides.
    assert_eq!(r.x - area.x, area.right() - r.right());
    assert_eq!(r.y - area.y, area.bottom() - r.bottom());
}

#[test]
fn centered_rect_honours_a_non_zero_area_origin() {
    let area = Rect::new(5, 3, 20, 10);
    assert_eq!(centered_rect(area, 10, 4), Rect::new(10, 6, 10, 4));
}

#[test]
fn centered_rect_pins_to_origin_when_box_is_larger_than_area() {
    // Saturating: an over-sized box must not wrap around to the far edge.
    let area = Rect::new(4, 2, 10, 6);
    let r = centered_rect(area, 30, 20);
    assert_eq!(r.x, 4);
    assert_eq!(r.y, 2);
}

#[test]
fn centered_rect_never_exceeds_its_area() {
    // Overlays size themselves as a clamped percentage of the board, and
    // those clamps have floors taller than a short terminal. The box has to
    // shrink rather than hang off the edge — an over-hanging rect panics
    // `Clear` with "index outside of buffer".
    let area = Rect::new(0, 0, 10, 6);
    let r = centered_rect(area, 40, 25);
    assert_eq!(r, area);
    assert!(area.union(r) == area, "{r:?} must stay inside {area:?}");
}

#[test]
fn centered_x_matches_centered_rect() {
    let area = Rect::new(7, 0, 51, 40);
    assert_eq!(centered_x(area, 20), centered_rect(area, 20, 10).x);
}

// ---- scroll_offset ----------------------------------------------------

#[test]
fn scroll_offset_is_zero_when_everything_fits() {
    assert_eq!(scroll_offset(2, 3, 10), 0);
    assert_eq!(scroll_offset(9, 10, 10), 0);
}

#[test]
fn scroll_offset_keeps_cursor_on_the_last_visible_row() {
    // 10 items, 3 visible, cursor at 4 → window is [2, 5).
    assert_eq!(scroll_offset(4, 10, 3), 2);
}

#[test]
fn scroll_offset_never_scrolls_past_the_end() {
    // cursor at the last item: window is the final `visible` rows.
    assert_eq!(scroll_offset(9, 10, 3), 7);
    // even a cursor beyond the list clamps to the last window.
    assert_eq!(scroll_offset(99, 10, 3), 7);
}

#[test]
fn scroll_offset_zero_visible_rows_is_zero() {
    assert_eq!(scroll_offset(5, 10, 0), 0);
}

#[test]
fn feed_role_label_none_is_hidden() {
    assert_eq!(feed_role_label(FeedRole::None), None);
}

#[test]
fn feed_role_label_shows_kebab_role() {
    assert_eq!(
        feed_role_label(FeedRole::ReviewsParent).as_deref(),
        Some("role:reviews-parent  ")
    );
    assert_eq!(
        feed_role_label(FeedRole::MyReviews).as_deref(),
        Some("role:my-reviews  ")
    );
    assert_eq!(
        feed_role_label(FeedRole::Cve).as_deref(),
        Some("role:cve  ")
    );
}

#[test]
fn empty_is_empty() {
    assert_eq!(fair_truncate_segments::<&str>(&[], 20, " › "), "");
}

#[test]
fn single_segment_uses_plain_truncate() {
    assert_eq!(fair_truncate_segments(&["Hello"], 20, " › "), "Hello");
    assert_eq!(fair_truncate_segments(&["Hello World"], 4, " › "), "Hel…");
}

#[test]
fn fits_whole_joins_unchanged() {
    let out = fair_truncate_segments(&["A", "B", "C"], 20, " › ");
    assert_eq!(out, "A › B › C");
}

#[test]
fn equal_segments_split_evenly() {
    // budget 15, sep " › " x2 = 6, name_budget = 9, 3 segments → 3 chars each.
    let out = fair_truncate_segments(&["Alpha", "Bravo", "Charl"], 15, " › ");
    assert_eq!(out, "Al… › Br… › Ch…");
}

#[test]
fn short_segment_donates_slack_to_long() {
    // "PR" is short; its slack goes to the long segment.
    // budget 21, seps=6, name_budget=15. Segments: "A"(1), "PR Reviews"(10), "Bots PR"(7).
    let out = fair_truncate_segments(&["A", "PR Reviews", "Bots PR"], 21, " › ");
    // "A" whole (1); remaining pool 14 across 2 long segments → 7 each.
    assert_eq!(out, "A › PR Rev… › Bots PR");
}

#[test]
fn narrow_budget_falls_back_to_whole_truncate() {
    // name_budget < 2 * n → whole-string truncate, never a bare ellipsis segment.
    let out = fair_truncate_segments(&["Alpha", "Bravo", "Charlie"], 8, " › ");
    assert_eq!(out, "Alpha ›…");
    assert!(out.chars().count() <= 8);
}

#[test]
fn empty_segments_never_exceed_budget() {
    let out = fair_truncate_segments(&["", ""], 1, " › ");
    assert!(out.chars().count() <= 1);
}

#[test]
fn push_hint_spans_embeds_key_in_non_ascii_label() {
    // 'é' is two bytes; slicing the label at byte 1 would panic.
    let mut spans = Vec::new();
    push_hint_spans(&mut spans, "é", "énorme", Color::Red, Style::default());
    assert_eq!(spans[0].content, "[é]");
    assert_eq!(spans[1].content, "norme  ");
}
