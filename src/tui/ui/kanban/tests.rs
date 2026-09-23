#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::super::palette::{CURSOR_BORDER, MUTED, PURPLE, SELECT_ALL_HIGHLIGHT_BG};
use super::super::shared::{render_folded_section_header, render_substatus_header};
use super::cards::render_epic_header_item;
use super::*;
use crate::models::{ColumnSection, EpicId, TaskTag};
use crate::tui::tests::make_epic_with_title;
use crate::tui::types::{FoldedHeader, SectionRef, TaskDraft};
use ratatui::buffer::Buffer;
use ratatui::widgets::ListItem;

fn make_test_app() -> App {
    App::new(vec![])
}

fn dummy_style() -> Style {
    Style::default()
}

fn dummy_styles() -> FormStyles {
    FormStyles {
        completed: dummy_style(),
        active: dummy_style(),
        hint: dummy_style(),
    }
}

#[test]
fn input_description_shows_tag_when_set() {
    let mut app = make_test_app();
    app.input.task_draft = Some(TaskDraft {
        title: "My task".into(),
        tag: Some(TaskTag::Bug),
        ..Default::default()
    });
    app.input.set_buffer("some desc".into());
    let lines = input_description_lines(&app, &dummy_styles());
    let text: String = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Tag: bug"), "expected tag line, got:\n{text}");
    assert!(text.contains("Title: My task"));
    assert!(text.contains("Description: opening $EDITOR"));
}

#[test]
fn input_description_shows_none_when_no_tag() {
    let mut app = make_test_app();
    app.input.task_draft = Some(TaskDraft {
        title: "No tag task".into(),
        tag: None,
        ..Default::default()
    });
    let lines = input_description_lines(&app, &dummy_styles());
    let text: String = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Tag: none"),
        "expected 'Tag: none', got:\n{text}"
    );
}

#[test]
fn input_repo_path_shows_tag_when_set() {
    let mut app = make_test_app();
    app.input.task_draft = Some(TaskDraft {
        title: "Feature task".into(),
        description: "A description".into(),
        tag: Some(TaskTag::Feature),
        ..Default::default()
    });
    app.input.set_buffer("/some/path".into());
    let area = Rect::new(0, 0, 80, 24);
    let lines = input_repo_path_lines(&app, area, &dummy_styles());
    let text: String = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Tag: feature"),
        "expected tag line, got:\n{text}"
    );
    assert!(text.contains("Title: Feature task"));
    assert!(text.contains("Description: A description"));
    assert!(text.contains("Repo path: /some/path"));
}

#[test]
fn input_repo_path_shows_none_when_no_tag() {
    let mut app = make_test_app();
    app.input.task_draft = Some(TaskDraft {
        title: "Plain task".into(),
        description: "desc".into(),
        tag: None,
        ..Default::default()
    });
    app.input.clear_buffer();
    let area = Rect::new(0, 0, 80, 24);
    let lines = input_repo_path_lines(&app, area, &dummy_styles());
    let text: String = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Tag: none"),
        "expected 'Tag: none', got:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Phoenix arming at the tag step (issue 4538). phoenix has no step of its own:
// `p` at the tag picker arms it and re-opens the same step. The armed flag must
// show inside the "New Task" panel, not only in the status bar (issue 4466).
// ---------------------------------------------------------------------------

/// The armed step renders one line more than the unarmed one — the settled
/// `Phoenix: yes` — so its reservation must cover the lines it draws plus the
/// two border rows. Anything less pushes the trailing `[Esc] cancel` hint
/// outside the panel.
#[test]
fn input_panel_height_covers_the_armed_tag_steps_lines_and_borders() {
    let mut app = make_test_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "My task".into(),
        phoenix: true,
        ..Default::default()
    });

    let reserved = input_panel_height(&app, 24);
    let needed = super::PHOENIX_ARMED_TAG_STEP_LINES + 2;
    assert!(
        reserved >= needed,
        "reserved {reserved} rows for a step needing at least {needed}"
    );
}

/// The clamp the armed arm's formula needs and the bare `8` arms do not: on a
/// terminal short enough that `max_height` floors at 8, the panel must not ask
/// the layout for more than it has, or the solver takes the row off the board's
/// stated minimum.
#[test]
fn input_panel_height_never_exceeds_the_layouts_budget_when_phoenix_is_armed() {
    let mut app = make_test_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "My task".into(),
        phoenix: true,
        ..Default::default()
    });

    for area_height in [10u16, 12, 17, 24, 40] {
        let max_height = area_height.saturating_sub(9).max(8);
        let reserved = input_panel_height(&app, area_height);
        assert!(
            reserved <= max_height,
            "area_height {area_height}: reserved {reserved} > budget {max_height}"
        );
    }
}

#[test]
fn render_input_form_draws_the_armed_phoenix_in_the_panel() {
    use ratatui::{backend::TestBackend, Terminal};

    let mut app = make_test_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "My task".into(),
        phoenix: true,
        ..Default::default()
    });

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut drew_form = false;
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 60, 12);
            drew_form = render_input_form(f, &app, area);
        })
        .unwrap();

    assert!(drew_form, "InputTag must render inside the form panel");

    let buf = terminal.backend().buffer().clone();
    let text: String = (0..buf.area().height)
        .map(|y| buf_row(&buf, y))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("New Task"), "got:\n{text}");
    assert!(text.contains("My task"), "got:\n{text}");
    assert!(text.contains("Phoenix: yes"), "got:\n{text}");
}

fn render_list_item_to_buf(item: ListItem<'static>, width: u16, height: u16) -> Buffer {
    use ratatui::{backend::TestBackend, widgets::List, Terminal};
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let list = List::new(vec![item]);
            f.render_widget(list, f.area());
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn buf_row(buf: &Buffer, y: u16) -> String {
    let area = buf.area();
    (area.left()..area.right())
        .map(|x| buf[(x, y)].symbol().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// render_substatus_header
// ---------------------------------------------------------------------------

/// An open header for `section` in the Running column — the shape every
/// pre-fold assertion below was written against.
fn open_header(section: ColumnSection) -> SectionRef {
    SectionRef::new(TaskStatus::Running, section)
}

/// A folded header for `section` in the Running column, hiding `hidden` cards.
fn folded_header(section: ColumnSection, hidden: usize) -> FoldedHeader {
    FoldedHeader {
        at: SectionRef::new(TaskStatus::Running, section),
        hidden,
    }
}

#[test]
fn substatus_header_has_two_lines() {
    let item = render_substatus_header(&open_header(ColumnSection::Active), false);
    let buf = render_list_item_to_buf(item, 40, 2);
    // Confirm both rows are allocated (height 2 means 2 rows rendered)
    assert_eq!(buf.area().height, 2);
}

#[test]
fn substatus_header_first_line_is_blank() {
    let item = render_substatus_header(&open_header(ColumnSection::Active), false);
    let buf = render_list_item_to_buf(item, 40, 2);
    let row0 = buf_row(&buf, 0);
    assert!(
        row0.trim().is_empty(),
        "first line should be blank spacer, got: {row0:?}"
    );
}

#[test]
fn substatus_header_second_line_contains_label() {
    let item = render_substatus_header(&open_header(ColumnSection::Active), false);
    let buf = render_list_item_to_buf(item, 40, 2);
    let row1 = buf_row(&buf, 1);
    assert!(
        row1.contains("active"),
        "second line should contain the section label, got: {row1:?}"
    );
}

#[test]
fn substatus_header_second_line_is_bold_and_bright() {
    let item = render_substatus_header(&open_header(ColumnSection::Active), false);
    let buf = render_list_item_to_buf(item, 40, 2);
    let area = buf.area();
    let first_content_x = (area.left()..area.right())
        .find(|&x| !buf[(x, 1)].symbol().trim().is_empty())
        .expect("row 1 should have content");
    let style = buf[(first_content_x, 1)].style();
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "header text should be BOLD"
    );
    assert_eq!(style.fg, Some(FG), "header text should use FG color");
}

#[test]
fn first_substatus_header_has_no_blank_line() {
    let item = render_substatus_header(&open_header(ColumnSection::AwaitingReview), true);
    assert_eq!(
        item.height(),
        1,
        "first header should have 1 line (no blank)"
    );
}

#[test]
fn subsequent_substatus_header_has_blank_line() {
    let item = render_substatus_header(&open_header(ColumnSection::AwaitingReview), false);
    assert_eq!(
        item.height(),
        2,
        "subsequent header should have 2 lines (blank + label)"
    );
}

#[test]
fn wrapped_line_count_empty_string_returns_zero() {
    assert_eq!(wrapped_line_count("", 80), 0);
}

#[test]
fn wrapped_line_count_width_zero_returns_zero() {
    assert_eq!(wrapped_line_count("hello", 0), 0);
}

#[test]
fn wrapped_line_count_single_line_shorter_than_width() {
    assert_eq!(wrapped_line_count("hello", 80), 1);
}

#[test]
fn wrapped_line_count_single_line_exactly_width() {
    assert_eq!(wrapped_line_count("hello", 5), 1);
}

#[test]
fn wrapped_line_count_single_line_longer_than_width_wraps() {
    // 10 chars, width 5 -> ceil(10/5) = 2 lines
    assert_eq!(wrapped_line_count("helloworld", 5), 2);
}

#[test]
fn wrapped_line_count_single_newline_counts_as_one() {
    assert_eq!(wrapped_line_count("\n", 80), 1);
}

#[test]
fn wrapped_line_count_multiline_text() {
    // "hello\nworld" -> 2 lines each 5 chars, width 80 -> 2 lines total
    assert_eq!(wrapped_line_count("hello\nworld", 80), 2);
}

#[test]
fn wrapped_line_count_multiline_with_wrapping() {
    // "aaaaaaaaaa\nbb" -> ceil(10/5)=2 + ceil(2/5)=1 = 3
    assert_eq!(wrapped_line_count("aaaaaaaaaa\nbb", 5), 3);
}

/// A folded header can hold the cursor, and it must be findable when it does.
/// A card shows the cursor on its frame; a header has none, so it takes the
/// cursor white plus the neutral lift behind it.
#[test]
fn a_cursored_folded_header_is_lifted() {
    let header = folded_header(ColumnSection::Active, 3);
    let item = render_folded_section_header(&header, true, true);
    let buf = render_list_item_to_buf(item, 40, 1);
    let area = buf.area();
    let x = (area.left()..area.right())
        .find(|&x| !buf[(x, 0)].symbol().trim().is_empty())
        .expect("the header row should have content");
    let style = buf[(x, 0)].style();
    assert_eq!(style.fg, Some(CURSOR_BORDER), "cursor fg");
    assert_eq!(style.bg, Some(SELECT_ALL_HIGHLIGHT_BG), "cursor bg");
}

/// An expanded header cannot hold the cursor, so it never carries the lift.
#[test]
fn an_uncursored_header_is_not_lifted() {
    let item = render_substatus_header(&open_header(ColumnSection::Active), true);
    let buf = render_list_item_to_buf(item, 40, 1);
    let area = buf.area();
    let x = (area.left()..area.right())
        .find(|&x| !buf[(x, 0)].symbol().trim().is_empty())
        .expect("the header row should have content");
    let style = buf[(x, 0)].style();
    assert_eq!(style.fg, Some(FG));
    assert_ne!(style.bg, Some(SELECT_ALL_HIGHLIGHT_BG));
}

/// The count and the marker belong to a folded header alone.
#[test]
fn a_folded_header_carries_its_count_and_marker() {
    let header = FoldedHeader {
        at: SectionRef::new(TaskStatus::Review, ColumnSection::Approved),
        hidden: 7,
    };
    let item = render_folded_section_header(&header, true, false);
    let buf = render_list_item_to_buf(item, 40, 1);
    assert!(
        buf_row(&buf, 0).contains("approved (7) \u{22ef}"),
        "got: {:?}",
        buf_row(&buf, 0)
    );
}

// ---------------------------------------------------------------------------
// render_epic_header_item (board-layout.allium, "Flattening": the
// epic-header id prefix)
// ---------------------------------------------------------------------------

/// The x position of `needle`'s first symbol in row `y`, if it occurs.
fn buf_find_char(buf: &Buffer, y: u16, needle: char) -> Option<u16> {
    let area = buf.area();
    (area.left()..area.right()).find(|&x| buf[(x, y)].symbol() == needle.to_string())
}

/// The rendered text of a single epic's header row.
fn epic_header_row(epic: &Epic, width: u16) -> String {
    buf_row(
        &render_list_item_to_buf(
            render_epic_header_item(epic, std::slice::from_ref(epic), width),
            width,
            1,
        ),
        0,
    )
}

/// A root epic's header row is prefixed with its own id.
#[test]
fn epic_header_row_shows_its_own_epic_id() {
    let row = epic_header_row(&make_epic_with_title(42, "Solo"), 40);
    assert!(row.contains("#42"), "got: {row:?}");
}

/// A nested epic's header row carries only its own id — not one per
/// ancestor named in the breadcrumb.
#[test]
fn epic_header_row_shows_leaf_id_only_for_nested_epic() {
    let root = make_epic_with_title(1, "Root");
    let mut mid = make_epic_with_title(2, "Mid");
    mid.parent_epic_id = Some(EpicId(1));
    let mut leaf = make_epic_with_title(3, "Leaf");
    leaf.parent_epic_id = Some(EpicId(2));
    let epics = vec![root, mid, leaf.clone()];

    let item = render_epic_header_item(&leaf, &epics, 60);
    let buf = render_list_item_to_buf(item, 60, 1);
    let row = buf_row(&buf, 0);

    assert!(row.contains("#3"), "missing leaf id, got: {row:?}");
    assert!(
        !row.contains("#1"),
        "root ancestor id leaked in, got: {row:?}"
    );
    assert!(
        !row.contains("#2"),
        "mid ancestor id leaked in, got: {row:?}"
    );
}

/// The id prefix is muted like every other id badge on the board, not the
/// row's purple identity hue — that stays on the breadcrumb title.
#[test]
fn epic_header_id_prefix_is_muted_not_purple() {
    let epic = make_epic_with_title(7, "Alpha");
    let item = render_epic_header_item(&epic, std::slice::from_ref(&epic), 40);
    let buf = render_list_item_to_buf(item, 40, 1);

    let hash_x = buf_find_char(&buf, 0, '#').expect("row should contain the id marker '#'");
    assert_eq!(
        buf[(hash_x, 0)].style().fg,
        Some(MUTED),
        "id prefix should be MUTED"
    );

    let title_x = buf_find_char(&buf, 0, 'A').expect("row should contain the breadcrumb title");
    assert_eq!(
        buf[(title_x, 0)].style().fg,
        Some(PURPLE),
        "breadcrumb title should stay PURPLE"
    );
}

/// Reserving width for the id prefix means a wider id leaves less room for
/// the title (board-layout.allium, "Flattening").
#[test]
fn epic_header_row_reserves_width_for_a_longer_id() {
    let width = 30;
    let long_title = "x".repeat(80);

    let short_row = epic_header_row(&make_epic_with_title(1, &long_title), width);
    let long_row = epic_header_row(&make_epic_with_title(123_456_789, &long_title), width);

    let short_visible = short_row.matches('x').count();
    let long_visible = long_row.matches('x').count();
    assert!(
        long_visible < short_visible,
        "a wider id prefix must leave less room for the title text: short id showed {short_visible} title chars, long id showed {long_visible} (short row: {short_row:?}, long row: {long_row:?})"
    );
}
