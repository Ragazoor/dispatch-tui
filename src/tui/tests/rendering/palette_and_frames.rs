use super::*;

/// Every column that renders a ground.
const GROUND_COLUMNS: [TaskStatus; 4] = [
    TaskStatus::Backlog,
    TaskStatus::Running,
    TaskStatus::Review,
    TaskStatus::Done,
];

/// Signed lightness on the shared scale whose zero point is the bare terminal
/// background (Tokyo Night `#1a1b26`). Mirrors `BoardNeutralRamp` in
/// board-visuals.allium: only the ordering of these numbers is normative, and values
/// below the terminal background are negative.
fn lightness_vs_terminal_bg(c: Color) -> i32 {
    const BG: i32 = 26 + 27 + 38;
    match c {
        Color::Rgb(r, g, b) => i32::from(r) + i32::from(g) + i32::from(b) - BG,
        other => panic!("expected an Rgb surface, got {other:?}"),
    }
}

#[tokio::test]
async fn board_ground_is_uniform_across_columns() {
    // board-visuals.allium "Column ground and card surface": every column renders the
    // *same* ground at a given focus state — there is no per-column tint and the
    // ground carries no hue. The superseded design derived each column's ground
    // from its identity colour; that is the regression this guards.
    for is_focused in [false, true] {
        let expected = ui::column_bg_color(TaskStatus::Backlog, is_focused);
        for status in GROUND_COLUMNS {
            assert_eq!(
                ui::column_bg_color(status, is_focused),
                expected,
                "{status:?} (focused={is_focused}) must share the uniform board ground"
            );
        }
    }
}

#[tokio::test]
async fn neutral_ramp_is_strictly_ascending() {
    // board-visuals.allium invariant NeutralRampIsStrictlyAscending:
    //   column_ground_unfocused < column_ground_focused < card_surface
    let unfocused = lightness_vs_terminal_bg(ui::column_bg_color(TaskStatus::Backlog, false));
    let focused = lightness_vs_terminal_bg(ui::column_bg_color(TaskStatus::Backlog, true));
    let card = lightness_vs_terminal_bg(ui::card_surface_color());

    assert!(
        unfocused < focused,
        "focused ground ({focused}) must be lighter than unfocused ({unfocused})"
    );
    assert!(
        focused < card,
        "card surface ({card}) must be lighter than the focused ground ({focused})"
    );
}

#[tokio::test]
async fn board_ground_is_recessed_below_terminal_background() {
    // board-visuals.allium invariant GroundIsRecessedBelowTerminalBackground: the ground
    // sits *below* the bare terminal background so cards read as raised rather
    // than inset. Assumes the dark terminal the palette is built for.
    let unfocused = lightness_vs_terminal_bg(ui::column_bg_color(TaskStatus::Backlog, false));
    assert!(
        unfocused < 0,
        "unfocused ground must be recessed below the terminal background, got {unfocused}"
    );
}

#[tokio::test]
async fn selection_does_not_lift_the_fill() {
    // board-visuals.allium invariant SelectionDoesNotLiftTheFill: a selected card's
    // surface is exactly a resting card's. Its emphasis lives in frame hue and
    // title weight, neither of which is a tint. A test asserting "selected is
    // lighter than resting" would be asserting something this design
    // deliberately does not do.
    assert_eq!(
        ui::selected_card_surface_color(),
        ui::card_surface_color(),
        "selection must not change the card fill"
    );
}

#[tokio::test]
async fn resting_card_border_is_neutral() {
    // board-visuals.allium "Task card frame": the frame colour is neutral for a resting
    // card and the column's identity colour only for the selected card. A
    // resting border must therefore never equal any column's identity colour.
    let border = ui::card_border_color();
    for status in GROUND_COLUMNS {
        assert_ne!(
            border,
            ui::column_color(status),
            "a resting card's border must not carry {status:?}'s identity colour"
        );
    }
}

/// A colour's channel signature: the RGB channels ranked brightest to dimmest.
///
/// This is a hue-family fingerprint that survives linear mixing toward any
/// neutral: `BLUE` is `b > g > r` and stays so whether dimmed toward the header
/// fill or brightened toward white, while `PURPLE` is `b > r > g` throughout.
/// Absolute distance does not work for this — a *dimmed* colour is by definition
/// far from its own undimmed source, so distance-to-own-hue is unsatisfiable.
fn channel_signature(c: Color) -> [usize; 3] {
    match c {
        Color::Rgb(r, g, b) => {
            let mut idx = [0usize, 1, 2];
            let v = [r, g, b];
            idx.sort_by_key(|&i| std::cmp::Reverse(v[i]));
            idx
        }
        other => panic!("expected an Rgb colour, got {other:?}"),
    }
}

#[tokio::test]
async fn each_header_label_keeps_its_column_hue_signature() {
    // The header labels are derived from `column_color` by a const mix, and this
    // asserts the property that derivation is *for*: whatever the mix does to
    // brightness, the label still reads as the same hue family as its source.
    //
    // The other three header tests — uniformity, per-column distinctness, and the
    // brightness ordering — would every one of them pass on a set of hand-picked
    // literals having nothing to do with the column hues. This is the one that
    // would not, because preserving all five channel signatures by accident is a
    // far stronger coincidence than being merely distinct and correctly ordered.
    for is_focused in [false, true] {
        for status in GROUND_COLUMNS {
            let hue = ui::column_color(status);
            let label = ui::column_header_fg(status, is_focused);
            assert_eq!(
                channel_signature(label),
                channel_signature(hue),
                "{status:?} (focused={is_focused}): label {label:?} does not share the \
                 channel signature of its hue {hue:?} — it is no longer derived from it"
            );
        }
    }
}

/// The position of the first cell whose symbol is exactly `sym`, scanning
/// top-to-bottom then left-to-right.
fn position_of_symbol(buf: &ratatui::buffer::Buffer, sym: &str) -> Option<(u16, u16)> {
    for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right() {
            if buf[(x, y)].symbol() == sym {
                return Some((x, y));
            }
        }
    }
    None
}

#[tokio::test]
async fn cards_are_inset_by_one_cell_of_column_ground() {
    // board-visuals.allium "Task card frame": the card is inset from the column by one
    // cell on each side, and those margin cells are ground, not card surface.
    //
    // This exists because the inset was implemented before it was specified, and
    // nothing enforced it — it could have been widened or dropped with a green
    // suite. The assertions below pin the margin's width, its colour, and the
    // fact that the rail immediately inside it is lit, in one place.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let buf = render_to_buffer(&mut app, 120, 30);

    let (cx, cy) = position_of_symbol(&buf, "\u{256d}").expect("expected a framed card");
    assert!(
        cx >= 1,
        "a card must not begin at the column's first cell; found the corner at x={cx}"
    );

    let ground = ui::column_bg_color(TaskStatus::Backlog, true);
    let surface = ui::card_surface_color();

    assert_eq!(
        buf[(cx - 1, cy)].bg,
        ground,
        "the cell left of a card's corner must be column ground, not card surface"
    );
    assert_eq!(
        buf[(cx, cy)].bg,
        surface,
        "the card's own corner must be lit by the card surface"
    );

    // The rail on the row below the top border: same column, and lit.
    assert_eq!(
        buf[(cx, cy + 1)].symbol(),
        "\u{2502}",
        "the row below a card's top border must open with its left rail"
    );
    assert_eq!(
        buf[(cx, cy + 1)].bg,
        surface,
        "a card's side rail must be lit by the card surface"
    );
    assert_eq!(
        buf[(cx - 1, cy + 1)].bg,
        ground,
        "the margin beside a card's rail must be column ground"
    );

    // Right margin: find the closing corner on the same row.
    let rx = (cx..buf.area.right())
        .find(|&x| buf[(x, cy)].symbol() == "\u{256e}")
        .expect("expected a closing top corner on the same row");
    assert_eq!(
        buf[(rx + 1, cy)].bg,
        ground,
        "the cell right of a card's closing corner must be column ground"
    );
}

#[tokio::test]
async fn every_card_frame_is_lit_by_the_card_surface() {
    // board-visuals.allium "Task card frame": the whole card is lit, frame included. The
    // border rows and side rails carry the *card surface* background, not the
    // column ground, so the card's boundary is the change of colour at its outer
    // edge. The rejected alternative painted the border on the ground.
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    let buf = render_to_buffer(&mut app, 120, 30);

    // All four corners, not just ╭ — the bottom border and the closing rails are
    // as much part of "the whole card is lit" as the top one. The side rails are
    // covered by `cards_are_inset_by_one_cell_of_column_ground`, which can tell a
    // card rail from the column separator by position; both use the │ glyph, so a
    // symbol scan alone cannot.
    let mut checked = 0usize;
    for glyph in ["\u{256d}", "\u{256e}", "\u{2570}", "\u{256f}"] {
        let corners = cells_with_symbol(&buf, glyph);
        assert!(
            !corners.is_empty(),
            "expected at least one card corner {glyph} to render"
        );
        for cell in corners {
            assert_eq!(
                cell.bg,
                ui::card_surface_color(),
                "card corner {glyph} must sit on the card surface, not the column ground"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 8,
        "expected both cards' four corners, saw {checked}"
    );
}

/// The cell holding an epic card's stripe, and the cell holding its title's
/// first character, on the row that starts with the given `#id` marker.
///
/// Located by scanning for the id rather than by fixed coordinates, so the test
/// does not silently start reading a different card when layout shifts.
fn epic_card_row(
    buf: &ratatui::buffer::Buffer,
    id: &str,
) -> Option<(ratatui::buffer::Cell, ratatui::buffer::Cell)> {
    for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right() {
            if buf[(x, y)].symbol() != "\u{258e}" {
                continue;
            }
            // "▎ #10 Epic 10" — the id begins two cells right of the stripe.
            let after: String = (x + 1..(x + 8).min(buf.area.right()))
                .map(|xx| buf[(xx, y)].symbol())
                .collect();
            if after.trim_start().starts_with(id) {
                // First title character: past "▎ #10 ".
                let title_x = x + 2 + id.len() as u16 + 1;
                return Some((buf[(x, y)].clone(), buf[(title_x, y)].clone()));
            }
        }
    }
    None
}

#[tokio::test]
async fn epic_cards_carry_purple_identity_and_a_bold_title_at_rest() {
    // board-visuals.allium "Epic cards": an epic is its own identity object. Its stripe is
    // PURPLE in every column rather than the column's hue, and its title is bold
    // *unconditionally* — which is why bold cannot be a cursor signal on an epic,
    // and why the frame is the only cursor cue an epic card has.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    // Cursor on the task at row 0, so the epic below it is at rest.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let buf = render_to_buffer(&mut app, 120, 30);

    // The palette PURPLE, written out rather than taken from `column_color(Review)`.
    // Review happens to share the token, but an epic's purple is its *own*
    // identity — sourcing it from a column would encode exactly the conflation this
    // test exists to disprove, and would keep passing if epics started following
    // their column.

    let (stripe, title) = epic_card_row(&buf, "#10").expect("expected the epic card to render");
    assert_eq!(
        stripe.fg, PURPLE,
        "an epic's stripe is PURPLE — its own identity, never its column's"
    );
    assert_ne!(
        stripe.fg,
        ui::column_color(TaskStatus::Backlog),
        "an epic sitting in Backlog must not take Backlog's hue"
    );
    assert!(
        title.modifier.contains(Modifier::BOLD),
        "a resting epic's title is bold unconditionally, so bold cannot mark the cursor"
    );
}

#[tokio::test]
async fn epic_view_tints_the_enclosing_panel_but_not_the_column_grounds() {
    // board-visuals.allium "Column ground and card surface": inside an epic the *enclosing
    // panel* is faintly purple as a mode signal — it says "you are inside an epic",
    // not "this column is purple" — while the column grounds within it stay the
    // uniform neutral.
    //
    // This is the only place purple means *mode* rather than *epic identity*, so a
    // regression would not read as obviously wrong to whoever found it. Nothing
    // asserted it: there is one production site and the snapshots carry no style.
    const PANEL_TINT: Color = Color::Rgb(24, 20, 34);

    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    app.board.view_mode = crate::tui::types::ViewMode::Epic {
        epic_id: crate::models::EpicId(10),
        selection: crate::tui::types::BoardSelection::new_for_epic(),
        parent: Box::new(crate::tui::types::ViewMode::Board(
            crate::tui::types::BoardSelection::new(),
        )),
    };
    let buf = render_to_buffer(&mut app, 160, 30);

    let panel_cells = (buf.area.top()..buf.area.bottom())
        .flat_map(|y| (buf.area.left()..buf.area.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].bg == PANEL_TINT)
        .count();
    assert!(
        panel_cells > 0,
        "epic view must tint its enclosing panel as a mode signal"
    );

    // ...and the grounds inside it are untouched.
    let ground = ui::column_bg_color(TaskStatus::Backlog, true);
    let ground_cells = (buf.area.top()..buf.area.bottom())
        .flat_map(|y| (buf.area.left()..buf.area.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].bg == ground)
        .count();
    assert!(
        ground_cells > 0,
        "the column grounds inside an epic view must stay the uniform neutral, not \
         take the panel's tint"
    );
}

/// Whether row `y` is painted with column ground colour anywhere across its
/// width — the board fills a column's whole area with its ground colour
/// regardless of card content, so this is true for any row genuinely inside
/// the kanban board and false for a row that belongs to another band (the
/// idle input panel's gap, or its bordered box when a form is active).
fn row_is_board_ground(buf: &Buffer, y: u16) -> bool {
    let unfocused = ui::column_bg_color(TaskStatus::Review, false);
    let focused = ui::column_bg_color(TaskStatus::Backlog, true);
    let area = buf.area();
    (area.left()..area.right()).any(|x| {
        let bg = buf[(x, y)].bg;
        bg == unfocused || bg == focused
    })
}

#[tokio::test]
async fn idle_input_panel_lets_columns_reach_the_status_bar() {
    // board-layout.allium "Board Vertical Layout": with no input mode active the input
    // panel is zero height, so the kanban board claims the full remaining
    // height and columns run uninterrupted down to the status bar — no empty
    // bordered box in between.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let (width, height) = (160, 30);
    let buf = render_to_buffer(&mut app, width, height);

    let status_bar_row = height - 1;
    let last_board_row = status_bar_row - 1;

    assert!(
        row_is_board_ground(&buf, last_board_row),
        "the row directly above the status bar must be board ground when idle, \
         not a gap left by the empty input panel"
    );
}

#[tokio::test]
async fn a_status_bar_confirmation_does_not_reserve_the_input_panel() {
    // board-layout.allium "Board Vertical Layout": a y/n confirmation is prompted in the
    // status bar, not the input panel, so it is idle from the panel's
    // perspective — the board must keep full height under it, the same as the
    // default Normal mode.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.input.mode = InputMode::ConfirmDeleteTask(TaskId(1));
    let (width, height) = (160, 30);
    let buf = render_to_buffer(&mut app, width, height);

    let status_bar_row = height - 1;
    let last_board_row = status_bar_row - 1;

    assert!(
        row_is_board_ground(&buf, last_board_row),
        "a status-bar-only confirmation must not leave an empty gap where the \
         input panel used to sit"
    );
}

#[tokio::test]
async fn active_input_mode_reserves_the_panel_and_shortens_the_board() {
    // board-layout.allium "Board Vertical Layout": once an input mode is active, the
    // panel reserves its computed height and renders a bordered, titled box —
    // and the kanban board shrinks to make room for it, rather than the two
    // bands overlapping or the board staying full height underneath it.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.input.mode = InputMode::InputTitle;
    let (width, height) = (160, 30);
    let buf = render_to_buffer(&mut app, width, height);

    assert!(
        buffer_contains(&buf, "New Task"),
        "the input panel must render its bordered, titled box while a form is active"
    );

    let status_bar_row = height - 1;
    let last_board_row = status_bar_row - 1;

    assert!(
        !row_is_board_ground(&buf, last_board_row),
        "the row directly above the status bar must belong to the reserved input \
         panel while a form is active, not the kanban board"
    );
}

#[tokio::test]
async fn the_cursor_card_title_is_bold_and_a_resting_one_is_not() {
    // board-visuals.allium "Selection": the cursor is marked by two things — the white frame
    // and a bold title. Only the frame was asserted. The *epic* bold title is
    // covered, deliberately, because it is unconditional there; that is what
    // disguised this omission, since a grep for bold coverage finds a hit.
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let buf = render_to_buffer(&mut app, 120, 30);

    let bold_for = |id: &str| -> bool {
        for y in buf.area.top()..buf.area.bottom() {
            for x in buf.area.left()..buf.area.right().saturating_sub(6) {
                let run: String = (x..x + 6).map(|xx| buf[(xx, y)].symbol()).collect();
                if run.starts_with(id) {
                    // Title begins one cell past "#N ".
                    let tx = x + id.len() as u16 + 1;
                    return buf[(tx, y)].modifier.contains(Modifier::BOLD);
                }
            }
        }
        panic!("card {id} did not render");
    };

    assert!(
        bold_for("#1"),
        "the cursor card's title must be bold — it is one of selection's two markers"
    );
    assert!(
        !bold_for("#2"),
        "a resting card's title must not be bold, or bold says nothing about the cursor"
    );
}

#[tokio::test]
async fn select_all_checkbox_fill_is_neutral_in_every_column() {
    // board-visuals.allium "Column header bar": the fill behind the focused column's
    // select-all checkbox is a single neutral shared by every column.
    //
    // It replaced a per-column *hued* ramp, and a hued checkbox is exactly what the
    // neutral-fill treatment exists to rule out — so a regression here restores the
    // thing that was deliberately removed, and does it silently. The header
    // snapshots cannot catch it: they are text, not style.
    for (nav, &status) in TaskStatus::ALL.iter().enumerate() {
        let mut app = App::new(vec![make_task(1, status)]);
        app.selection_mut().set_column(nav + 1);
        // Move up off the first row to park the cursor on the column header.
        app.update(Message::NavigateRow(-1));
        assert!(
            app.on_select_all(),
            "{status:?}: expected the cursor to land on the select-all header"
        );

        let buf = render_to_buffer(&mut app, 160, 30);
        let checkbox = (buf.area.left()..buf.area.right())
            .map(|x| buf[(x, 1)].clone())
            .find(|c| c.symbol() == "[")
            .unwrap_or_else(|| panic!("{status:?}: expected a select-all checkbox on the header"));

        // Asserted against `card_border_color`, which is what the spec claims the
        // value *is* — not against `select_all_highlight_bg`, which is the function
        // the renderer already calls. Comparing a render to its own source restates
        // the implementation instead of checking it: change the function and both
        // sides move together, and the test passes on a value nobody chose.
        assert_eq!(
            checkbox.bg,
            ui::card_border_color(),
            "{status:?}: the checkbox fill must be the same neutral as a resting \
             card's border"
        );
        for &other in TaskStatus::ALL.iter() {
            assert_ne!(
                checkbox.bg,
                ui::column_color(other),
                "{status:?}: the checkbox fill must not be any column's identity hue"
            );
        }
    }
}

#[tokio::test]
async fn a_card_spends_four_cells_of_its_column_on_chrome() {
    // board-visuals.allium "Task card frame": two ground margins plus two frame rails, and
    // every one of those cells comes out of the title budget.
    //
    // `cards_are_inset_by_one_cell_of_column_ground` pins the margin's width; this
    // pins the *total*, which is the number that actually reaches truncation. They
    // are separate claims: narrowing the rails while widening the margin would keep
    // the inset test green and silently change what fits on a card.
    const CHROME: u16 = 4;

    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let buf = render_to_buffer(&mut app, 160, 30);

    // The first column runs from the left edge to the first separator, so the
    // separator's x *is* that column's width.
    let probe_y = 15;
    let col_width = (buf.area.left()..buf.area.right())
        .find(|&x| {
            let c = &buf[(x, probe_y)];
            c.symbol() == "\u{2502}" && c.fg == BORDER
        })
        .expect("expected a column separator on an empty board row");

    let (cx, cy) = position_of_symbol(&buf, "\u{256d}").expect("expected a framed card");
    let rx = (cx..buf.area.right())
        .find(|&x| buf[(x, cy)].symbol() == "\u{256e}")
        .expect("expected the card's closing corner on the same row");

    let content = rx - cx - 1; // cells strictly between the two rails
    assert_eq!(
        content + CHROME,
        col_width,
        "a card must spend exactly {CHROME} cells of its {col_width}-cell column on \
         chrome; content measured {content}"
    );
}

#[tokio::test]
async fn flat_view_epic_breadcrumb_is_purple() {
    // board-visuals.allium "Epic cards": the breadcrumb row heading a group of epic-owned
    // tasks in flattened view carries epic purple on the same terms as the card
    // stripe — it is the second surface that claim covers.
    //
    // The flat-view snapshots render this row, but `.snap` files are text only and
    // carry no style, so a breadcrumb that lost its hue would leave every one of
    // them byte-identical. Nothing was checking the colour until this.

    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    let mut t1 = make_task(1, TaskStatus::Running);
    t1.epic_id = Some(crate::models::EpicId(10));
    t1.sort_order = Some(10);
    app.board.tasks = vec![t1];
    app.board.flattened = true;
    app.selection_mut().set_column(2); // Running
    let buf = render_to_buffer(&mut app, 120, 30);

    // Target the breadcrumb row itself. Counting purple anywhere in the buffer is
    // not enough: the epic *card* is purple too, so such a test passes even with
    // the breadcrumb drawn in grey — it measures the card and reports the
    // breadcrumb. Find the row that opens with the "── " rule and read the colour
    // of the title that follows it. The title sits after the rule's "── " and
    // the muted "#{id} " prefix (board-layout.allium, "Flattening") — search
    // for it rather than assuming a fixed offset, so the id prefix's width
    // doesn't matter here.
    let mut breadcrumb_title_colours: Vec<Color> = Vec::new();
    'rows: for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right().saturating_sub(3) {
            let is_rule_prefix = buf[(x, y)].symbol() == "\u{2500}"
                && buf[(x + 1, y)].symbol() == "\u{2500}"
                && buf[(x + 2, y)].symbol() == " ";
            if !is_rule_prefix {
                continue;
            }
            let search_start = x + 3;
            let window: String = (search_start..(search_start + 20).min(buf.area.right()))
                .map(|xx| buf[(xx, y)].symbol())
                .collect();
            if let Some(rel_offset) = window.find("Epic 10") {
                let title_start = search_start + rel_offset as u16;
                breadcrumb_title_colours = (title_start..title_start + 7)
                    .map(|xx| buf[(xx, y)].fg)
                    .collect();
                break 'rows;
            }
        }
    }

    assert!(
        !breadcrumb_title_colours.is_empty(),
        "expected a \"── Epic 10\" breadcrumb row in the flattened view"
    );
    for c in &breadcrumb_title_colours {
        assert_eq!(
            *c, PURPLE,
            "the breadcrumb's title must be epic purple, not {c:?}"
        );
    }
}

/// board-layout.allium "Epic View Panel Title": drilling into an epic
/// borders the columns panel with a breadcrumb title prefixed by that
/// epic's own numeric id, the same "epic's own id" rule as the flattened
/// epic-header row (see "Flattening").
#[tokio::test]
async fn epic_view_panel_title_shows_the_current_epics_id() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic_with_title(7, "Alpha")];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(7),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let buf = render_to_buffer(&mut app, 120, 30);

    assert!(
        buffer_contains(&buf, "#7 Alpha"),
        "expected the epic-view panel title to show the epic's own id before its breadcrumb"
    );
}

#[tokio::test]
async fn scroll_indicators_follow_the_column_top_rule() {
    // board-visuals.allium's named exception covers the scroll indicators as well as the top
    // rule: hued while focused, neutral grey while not. They share one colour in
    // the renderer, but that is an implementation fact rather than an asserted one,
    // so splitting them would otherwise be caught by nothing.

    // Enough cards in both columns to overflow a short board.
    let mut tasks = Vec::new();
    for id in 1..=12 {
        tasks.push(make_task(id, TaskStatus::Backlog));
    }
    for id in 13..=24 {
        tasks.push(make_task(id, TaskStatus::Running));
    }
    let mut app = App::new(tasks);
    let buf = render_to_buffer(&mut app, 160, 24);

    let mut arrows: Vec<Color> = Vec::new();
    for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right() {
            let sym = buf[(x, y)].symbol();
            if sym == "\u{25b2}" || sym == "\u{25bc}" {
                arrows.push(buf[(x, y)].fg);
            }
        }
    }
    assert!(
        arrows.len() >= 2,
        "expected an overflow indicator in both the focused and an unfocused column, \
         found {arrows:?}"
    );
    // Backlog is focused on a fresh board; Running is not.
    assert!(
        arrows.contains(&ui::column_color(TaskStatus::Backlog)),
        "the focused column's scroll indicator must carry its identity hue, got {arrows:?}"
    );
    assert!(
        arrows.contains(&MUTED),
        "an unfocused column's scroll indicator must be neutral grey, got {arrows:?}"
    );
}

#[tokio::test]
async fn selected_epic_frames_in_the_cursor_white_not_purple() {
    // board-visuals.allium "Epic cards": the cursor white applies to epics too, with no
    // exemption. A purple frame would put Review's own identity hue on a card
    // frame — the collision the white exists to prevent, surviving on the one card
    // type that had escaped it.
    let mut app = make_app_with_epic_selected();
    let buf = render_to_buffer(&mut app, 120, 30);

    let corners = cells_with_symbol(&buf, "\u{256d}");
    let purple = ui::column_color(TaskStatus::Review);
    let cursor = ui::cursor_border_color();
    let neutral = ui::card_border_color();

    assert!(
        corners.iter().any(|c| c.fg == cursor),
        "the selected epic's frame must be the cursor white"
    );
    assert!(
        !corners.iter().any(|c| c.fg == purple),
        "no card frame may be purple — an epic's identity stays on its stripe and title"
    );
    for c in &corners {
        assert!(
            c.fg == cursor || c.fg == neutral,
            "with one epic selected and one healthy task, every frame must be the \
             cursor white or the neutral; found {:?}",
            c.fg
        );
    }
}

#[tokio::test]
async fn an_epic_frame_never_takes_a_state_colour() {
    // board-visuals.allium "Epic cards" / EpicFramesAreOnlyWhiteOrNeutral: an epic
    // card carries no CardIndicator, so it claims no state colour. Its frame is the
    // cursor white or the resting neutral and nothing else, even sitting in a
    // column where a task is crashed.
    //
    // The epic is put in Running deliberately, beside the crashed task rather than
    // in a quiet column of its own: a renderer that read the *column's* worst state
    // onto every frame in it would still pass a version that parked the epic in
    // Backlog. The cursor is on a third card in a third column so the cursor rule
    // cannot mask the answer.
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;

    let mut crashed = make_task(1, TaskStatus::Running);
    crashed.sub_status = SubStatus::Crashed;
    crashed.worktree = Some("/repo/.worktrees/1-task".to_string());

    let mut app = App::new(vec![crashed, make_task(2, TaskStatus::Backlog)]);
    app.board.epics = vec![epic];
    let buf = render_to_buffer(&mut app, 120, 30);

    let frames: Vec<Color> = cells_with_symbol(&buf, "\u{256d}")
        .iter()
        .map(|c| c.fg)
        .collect();
    let cursor = ui::cursor_border_color();
    let neutral = ui::card_border_color();

    assert_eq!(
        frames.len(),
        3,
        "expected the epic, the crashed task and the cursor task, got {frames:?}"
    );
    assert_eq!(
        frames.iter().filter(|c| **c == RED).count(),
        1,
        "exactly one card is crashed, so exactly one frame may be red — an epic \
         sharing its column must not pick the colour up; frames were {frames:?}"
    );
    assert_eq!(
        frames.iter().filter(|c| **c == neutral).count(),
        1,
        "the epic is the one resting card, so exactly one frame must be neutral; \
         frames were {frames:?}"
    );
    assert_eq!(
        frames.iter().filter(|c| **c == cursor).count(),
        1,
        "the cursor sits on the healthy Backlog task; frames were {frames:?}"
    );
    // No amber assertion: three frames, one of each colour above, so the counts
    // already account for every frame on the board.
}

#[tokio::test]
async fn card_frame_carries_state_and_the_cursor_outranks_it() {
    // board-visuals.allium "Card border: cursor and state". Three claims in one board,
    // because they only mean anything together:
    //   - a hard failure borders red,
    //   - an attention state borders amber,
    //   - and the cursor outranks both, so an unhealthy card that is also the
    //     cursor shows white and reports its state on the indicator line instead.

    let mut crashed = make_task(1, TaskStatus::Running);
    crashed.sub_status = SubStatus::Crashed;
    crashed.worktree = Some("/repo/.worktrees/1-task".to_string());
    let mut blocked = make_task(2, TaskStatus::Running);
    blocked.sub_status = SubStatus::NeedsInput;
    blocked.worktree = Some("/repo/.worktrees/2-task".to_string());
    blocked.tmux_window = Some(test_tmux_window("task-2"));
    let mut healthy = make_task(3, TaskStatus::Running);
    healthy.sub_status = SubStatus::Active;
    healthy.worktree = Some("/repo/.worktrees/3-task".to_string());
    healthy.tmux_window = Some(test_tmux_window("task-3"));
    healthy.last_pre_tool_use_at = Some(Utc::now());

    // Cursor on the *crashed* card: the case where the two rules collide.
    let mut app = App::new(vec![crashed, blocked, healthy]);
    app.update(Message::NavigateColumn(1)); // Running
    let buf = render_to_buffer(&mut app, 120, 30);

    let frames: Vec<Color> = cells_with_symbol(&buf, "\u{256d}")
        .iter()
        .map(|c| c.fg)
        .collect();
    assert_eq!(
        frames.len(),
        3,
        "expected three framed cards, got {frames:?}"
    );

    assert!(
        frames.contains(&YELLOW),
        "the blocked card must border amber; frames were {frames:?}"
    );
    assert!(
        frames.contains(&ui::cursor_border_color()),
        "the cursor card must border white; frames were {frames:?}"
    );
    assert!(
        frames.contains(&ui::card_border_color()),
        "the healthy card must border neutral; frames were {frames:?}"
    );
    assert!(
        !frames.contains(&RED),
        "the only crashed card here is the cursor, so the cursor white must win and \
         no red may appear; frames were {frames:?}"
    );

    // Move the cursor off it and the red it was suppressing appears.
    app.update(Message::NavigateRow(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    let frames: Vec<Color> = cells_with_symbol(&buf, "\u{256d}")
        .iter()
        .map(|c| c.fg)
        .collect();
    assert!(
        frames.contains(&RED),
        "with the cursor moved away the crashed card must border red; frames were \
         {frames:?}"
    );
}

#[tokio::test]
async fn only_the_selected_card_has_the_cursor_white_frame() {
    // board-visuals.allium "Selection": the cursor's frame is a near-white owned by nothing
    // else on the board, and at most one card carries it. Healthy resting frames
    // are neutral.
    //
    // The card frame carries *state*, not identity — the cursor took a white of its
    // own precisely so it is not competing with the alarm colours. A test asserting
    // the cursor's frame is the *column hue* is asserting the superseded design.
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
    ]);
    let buf = render_to_buffer(&mut app, 120, 30);

    let cursor = ui::cursor_border_color();
    let neutral = ui::card_border_color();
    let hue = ui::column_color(TaskStatus::Backlog);
    let corners = cells_with_symbol(&buf, "\u{256d}"); // ╭
    let white = corners.iter().filter(|c| c.fg == cursor).count();
    let resting = corners.iter().filter(|c| c.fg == neutral).count();

    assert_eq!(
        white, 1,
        "exactly one card frame may carry the cursor white, found {white}"
    );
    assert!(
        resting >= 1,
        "healthy resting card frames must be neutral, found {resting} of {}",
        corners.len()
    );
    assert_eq!(
        white + resting,
        corners.len(),
        "with every task healthy, each frame must be the cursor white or the neutral"
    );
    assert!(
        !corners.iter().any(|c| c.fg == hue),
        "no card frame may carry the column's identity hue — the frame is a state \
         channel now, and identity lives on the stripe and the header label"
    );
}

#[tokio::test]
async fn column_top_rule_is_hued_only_while_focused() {
    // board-visuals.allium's named exception under "Focus is intensity, not
    // colour-vs-absence": the column's top rule and its scroll indicators take the
    // identity hue while focused and drop to a flat neutral grey while not. That is
    // the one place on the board where hue signals focus by presence rather than
    // intensity, and it was entirely unguarded — a change that flattened the
    // focused rule to grey, or gave the unfocused one a dimmed hue, passed either
    // way, in both cases silently erasing or contradicting the exception.
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Running),
        make_task(3, TaskStatus::Review),
    ]);
    let buf = render_to_buffer(&mut app, 160, 30);

    // Row 0 is the indicator bar, row 1 the summary; the board's first row is the
    // columns' TOP borders.
    // Collect the *distinct* colours: a rule spans dozens of cells, so reporting
    // every one of them buries the answer in a wall of repeats.
    let mut rules: Vec<Color> = Vec::new();
    for x in buf.area.left()..buf.area.right() {
        let cell = &buf[(x, 2)];
        if cell.symbol() == "\u{2500}" && !rules.contains(&cell.fg) {
            rules.push(cell.fg);
        }
    }
    assert!(
        !rules.is_empty(),
        "expected column top rules on the board's first row"
    );

    // Backlog is the focused column on a fresh board.
    let focused_hue = ui::column_color(TaskStatus::Backlog);
    assert!(
        rules.contains(&focused_hue),
        "the focused column's top rule must carry its identity hue {focused_hue:?}; \
         the rules on this row are {rules:?}"
    );
    assert!(
        rules.contains(&MUTED),
        "an unfocused column's top rule must drop to neutral grey; \
         the rules on this row are {rules:?}"
    );
    for c in &rules {
        assert!(
            *c == focused_hue || *c == MUTED,
            "a top rule must be either the focused column's hue {focused_hue:?} or the \
             neutral grey; found {c:?}, so another column's hue has leaked into a rule"
        );
    }
}

#[tokio::test]
async fn no_column_leaks_an_identity_hue_onto_a_card_frame() {
    // The cross-column companion: `only_the_selected_card_has_the_cursor_white_frame`
    // renders one column and so structurally cannot see a colour appearing in
    // another. With every task healthy, the whole board should show exactly one
    // cursor white and neutrals everywhere else — and, crucially, no column's
    // identity hue anywhere, since the frame stopped being an identity channel.
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Running),
        make_task(4, TaskStatus::Review),
        make_task(5, TaskStatus::Done),
    ]);
    let buf = render_to_buffer(&mut app, 160, 30);

    let neutral = ui::card_border_color();
    let cursor = ui::cursor_border_color();
    let corners = cells_with_symbol(&buf, "\u{256d}");
    assert!(corners.len() >= 5, "expected a card in every column");

    let non_neutral: Vec<Color> = corners
        .iter()
        .map(|c| c.fg)
        .filter(|fg| *fg != neutral)
        .collect();
    assert_eq!(
        non_neutral.len(),
        1,
        "with every task healthy exactly one frame may differ from the neutral; \
         found {non_neutral:?}"
    );
    assert_eq!(
        non_neutral[0], cursor,
        "the one non-neutral frame must be the cursor white"
    );
    for &status in TaskStatus::ALL.iter() {
        let hue = ui::column_color(status);
        assert!(
            !corners.iter().any(|c| c.fg == hue),
            "{status:?}'s identity hue appears on a card frame; the frame carries \
             state, not identity"
        );
    }
}

#[tokio::test]
async fn header_bar_stops_at_the_column_separators() {
    // The header bar must span exactly its own column. It used to be laid out by a
    // *different* constraint set than the board — the summary row divided the width
    // into N equal parts with no separator columns, while the board divided it into
    // N parts plus N-1 one-cell separators — so the two drifted apart and a header
    // fill bled across the separator into its neighbour.
    //
    // Checked at several widths because the drift depends on how the ratio rounding
    // falls, so a single width can happen to line up.
    let header_fills = [
        ui::column_header_bg(TaskStatus::Backlog, false),
        ui::column_header_bg(TaskStatus::Backlog, true),
    ];

    for width in [100u16, 120, 137, 200, 251] {
        let mut app = App::new(vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Running),
            make_task(3, TaskStatus::Review),
            make_task(4, TaskStatus::Done),
        ]);
        let buf = render_to_buffer(&mut app, width, 30);

        // Separator columns run the full height of the board in BORDER. Card rails
        // share the │ glyph but never that colour, and this row sits below the
        // cards, so anything matching here is a separator.
        let probe_y = 18;
        let sep_xs: Vec<u16> = (buf.area.left()..buf.area.right())
            .filter(|&x| {
                let c = &buf[(x, probe_y)];
                c.symbol() == "\u{2502}" && c.fg == BORDER
            })
            .collect();
        assert!(
            !sep_xs.is_empty(),
            "width {width}: expected column separators at y={probe_y}"
        );

        // The summary row sits directly under the top indicator row.
        for x in sep_xs {
            let cell = &buf[(x, 1)];
            assert!(
                !header_fills.contains(&cell.bg),
                "width {width}: a header fill covers the separator at x={x} — the bar \
                 is wider than its column"
            );
        }
    }
}

#[tokio::test]
async fn header_fill_is_uniform_across_columns() {
    // board-visuals.allium "Column header bar": the header fill carries no hue and is the
    // same in every column at a given focus state — identity moved to the label.
    // The superseded fill was a per-column darkened wash of the identity colour;
    // that is the regression this guards.
    for is_focused in [false, true] {
        let expected = ui::column_header_bg(TaskStatus::Backlog, is_focused);
        for status in GROUND_COLUMNS {
            assert_eq!(
                ui::column_header_bg(status, is_focused),
                expected,
                "{status:?} (focused={is_focused}) must share the uniform header fill"
            );
        }
    }
}

#[tokio::test]
async fn header_label_is_hued_in_every_column_at_both_focus_states() {
    // Identity rests on the label now, so it must be distinguishable per column at
    // both focus states. Two guards: no two columns may share a label colour, and
    // no label may collapse to a neutral.
    for is_focused in [false, true] {
        let mut seen: Vec<(TaskStatus, Color)> = Vec::new();
        for &status in TaskStatus::ALL.iter() {
            let fg = ui::column_header_fg(status, is_focused);
            assert_ne!(
                fg,
                ui::column_header_bg(status, is_focused),
                "{status:?} (focused={is_focused}) label must not vanish into the fill"
            );
            for (other, prev) in &seen {
                assert_ne!(
                    fg, *prev,
                    "{status:?} and {other:?} must not share a header label colour \
                     (focused={is_focused}) — the label is the only per-column signal left"
                );
            }
            seen.push((status, fg));
        }
    }
}

#[tokio::test]
async fn focused_header_label_is_brighter_than_unfocused() {
    // board-visuals.allium "Focus is intensity, not colour-vs-absence": the label keeps its
    // hue at both states and focus moves only its brightness. With the fill now
    // neutral, the label is the only place that intensity can be read.
    for &status in TaskStatus::ALL.iter() {
        let unfocused = lightness_vs_terminal_bg(ui::column_header_fg(status, false));
        let focused = lightness_vs_terminal_bg(ui::column_header_fg(status, true));
        assert!(
            unfocused < focused,
            "{status:?}: focused label ({focused}) must be brighter than unfocused ({unfocused})"
        );
    }
}

#[tokio::test]
async fn unfocused_column_header_keeps_its_identity_colour() {
    // board-visuals.allium: "the column's identity colour is always visible; focus
    // modulates emphasis only". The superseded behaviour flattened unfocused
    // headers to MUTED grey — that is the regression this guards.
    for &status in TaskStatus::ALL.iter() {
        let fg = ui::column_header_fg(status, false);
        assert_ne!(
            fg, MUTED,
            "{status:?} unfocused header must not collapse to MUTED grey"
        );
    }
}

#[tokio::test]
async fn focused_column_header_is_more_emphatic_than_unfocused() {
    // board-visuals.allium "Column header bar": the bar, not the ground, is where focus
    // is read as colour intensity. The header fill stays hued at both focus
    // states; the focused one is the brighter fill of the two.
    for &status in TaskStatus::ALL.iter() {
        let unfocused = lightness_vs_terminal_bg(ui::column_header_bg(status, false));
        let focused = lightness_vs_terminal_bg(ui::column_header_bg(status, true));
        assert!(
            unfocused < focused,
            "{status:?}: focused header fill ({focused}) must exceed unfocused ({unfocused})"
        );
    }
}

#[tokio::test]
async fn column_header_label_is_uppercased() {
    // board-visuals.allium: "It shows the column label, uppercased, followed by the
    // count of selectable items".
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "BACKLOG"),
        "column header should render the label uppercased"
    );
}

#[tokio::test]
async fn task_cards_render_a_complete_frame() {
    // board-visuals.allium: "Every card draws its own complete frame — rounded top and
    // bottom borders plus left and right rails ... no two cards share a border."
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let buf = render_to_buffer(&mut app, 120, 40);

    for glyph in ["\u{256d}", "\u{256e}", "\u{2570}", "\u{256f}"] {
        assert!(
            buffer_contains(&buf, glyph),
            "card frame should draw the rounded corner {glyph:?}"
        );
    }
    assert!(
        buffer_contains(&buf, "\u{2502}"),
        "card frame should draw left/right rails"
    );
}

#[tokio::test]
async fn task_card_frame_spans_four_lines_top_to_bottom() {
    // The frame costs one line over the old shared-rule presentation: top
    // border, title, metadata, bottom border (board-visuals.allium: "Task card frame"),
    // so the closing corner sits exactly 3 rows below the opening one.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let buf = render_to_buffer(&mut app, 120, 40);
    let area = buf.area();

    let row_has =
        |y: u16, glyph: &str| (area.left()..area.right()).any(|x| buf[(x, y)].symbol() == glyph);
    let top = (area.top()..area.bottom())
        .find(|&y| row_has(y, "\u{256d}"))
        .expect("a card top border");
    assert!(
        row_has(top + 3, "\u{2570}"),
        "the card's bottom border should sit 3 rows below its top border"
    );
}

/// The repo-filter overlay used to compute its height as
/// `.clamp(8, area.height - 4)`, which panics on `min > max` the moment the
/// board is shorter than 12 rows. Render code must never panic, so a short
/// board now shrinks the popup instead. Driven through the real render path
/// (not just the layout helper) so the whole overlay is exercised.
#[tokio::test]
async fn repo_filter_overlay_renders_on_a_board_too_short_for_its_minimum_height() {
    use crate::tui::messages::RepoFilterMessage;

    for height in [6_u16, 8, 10, 11, 12] {
        let mut app = App::new(vec![]);
        app.board.repo_paths = vec!["/repos/alpha".to_string(), "/repos/beta".to_string()];
        app.update(Message::RepoFilter(RepoFilterMessage::Start));
        // Panics here, not an assertion failure, are the regression.
        let _buf = render_to_buffer(&mut app, 60, height);
    }
}

/// The centred overlays are laid out from percentages of the board, so a
/// terminal far from the 120x40 the snapshots pin must still draw a complete,
/// on-screen frame. Substitutes for eyeballing the popup at an odd size.
#[tokio::test]
async fn repo_filter_overlay_stays_inside_a_narrow_board() {
    use crate::tui::messages::RepoFilterMessage;

    let mut app = App::new(vec![]);
    app.board.repo_paths = (0..30).map(|i| format!("/repos/r{i}")).collect();
    app.update(Message::RepoFilter(RepoFilterMessage::Start));
    let buf = render_to_buffer(&mut app, 46, 18);

    assert!(
        buffer_contains(&buf, "Repo Filter"),
        "the overlay title should be drawn"
    );
    // Double-line border corners: all four must be present, so the popup is
    // fully on-screen rather than clipped at an edge.
    for glyph in ["\u{2554}", "\u{2557}", "\u{255a}", "\u{255d}"] {
        assert!(
            buffer_contains(&buf, glyph),
            "the overlay border corner {glyph:?} should be on-screen"
        );
    }
}

/// Overlays size themselves as a clamped percentage of the board, and several
/// of those clamps have floors taller than a small terminal (the help overlay
/// floors at 25 rows). `Frame::render_widget` does no clipping and
/// `Clear` writes every cell it is handed, so before `centered_rect`/
/// `open_overlay` clamped, opening any of these on a short board panicked the
/// render thread with "index outside of buffer" rather than drawing something
/// small. One case per overlay, at a board shorter than every floor.
#[tokio::test]
async fn every_overlay_survives_a_board_shorter_than_its_own_minimum() {
    use crate::tui::messages::RepoFilterMessage;

    let short = (100_u16, 8_u16);

    // Help — floors at 25 rows.
    let mut app = App::new(vec![]);
    app.input.mode = crate::tui::InputMode::Help;
    let _ = render_to_buffer(&mut app, short.0, short.1);

    // Repo filter — floors at 8 rows.
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repos/alpha".to_string()];
    app.update(Message::RepoFilter(RepoFilterMessage::Start));
    let _ = render_to_buffer(&mut app, short.0, short.1);

    // Error popup — fixed 7 rows tall, top-pinned once the board is smaller.
    let mut app = App::new(vec![]);
    app.update(Message::System(crate::tui::messages::SystemMessage::Error(
        "boom".to_string(),
    )));
    let _ = render_to_buffer(&mut app, short.0, 4);
}

/// The repo-filter overlay's height budget and its visible-repo window are both
/// derived from `header.len() + footer.len()` — the rows the render body has
/// already built. This drives the real render path in every input mode the
/// overlay supports and asserts the footer's last row is on screen, which is
/// what a budget too small would clip. A layout tallied by hand (the `+7`/`+5`
/// literals this replaced) had nothing pinning it to the rows actually drawn.
///
/// The scrolling cases are the second half of the budget: the `↑ N more` /
/// `↓ N more` markers are content rows too, and the cursor positions below put
/// the window at the top (down-marker only), the middle (both) and the bottom
/// (up-marker only) of a list far longer than the popup.
#[tokio::test]
async fn repo_filter_renders_its_whole_footer_in_every_mode() {
    use crate::tui::messages::RepoFilterMessage;

    // (repo count, board height, repo cursor) — the first fits without
    // scrolling; the rest scroll, exercising each marker combination.
    let scenarios = [(12, 24, 0), (40, 20, 0), (40, 20, 20), (40, 20, 40)];

    for (repo_count, board_height, repo_cursor) in scenarios {
        let repos: Vec<String> = (0..repo_count).map(|i| format!("/repos/r{i}")).collect();

        // Match the overlay's own footer wording, not a bare verb — the board's
        // hint bar sits outside the popup and would satisfy a loose needle even
        // when the overlay's last row was clipped clean off.
        for (mode, expected_footer) in [
            (crate::tui::InputMode::RepoFilter, "[q/Esc] close"),
            (
                crate::tui::InputMode::ConfirmDeleteRepoPath,
                "n/Esc: cancel",
            ),
        ] {
            let mut app = App::new(vec![]);
            app.board.repo_paths = repos.clone();
            app.update(Message::RepoFilter(RepoFilterMessage::Start));
            app.input.mode = mode.clone();
            app.input.repo_cursor = repo_cursor;

            let case = format!(
                "{mode:?} with {repo_count} repos on a {board_height}-row board, \
                 cursor at {repo_cursor}"
            );
            let buf = render_to_buffer(&mut app, 100, board_height);
            assert!(
                buffer_contains(&buf, expected_footer),
                "footer text {expected_footer:?} was clipped in {case} — the layout \
                 budget disagrees with the rows the overlay renders"
            );
            // The bottom border must also survive: a footer that fits but a border
            // that doesn't means the popup is one row taller than it budgeted.
            assert!(
                buffer_contains(&buf, "\u{255a}"),
                "the overlay's bottom-left corner should be drawn in {case}"
            );
        }
    }
}
