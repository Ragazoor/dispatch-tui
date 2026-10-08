use super::*;
use crossterm::event::KeyModifiers;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// A pane with `rows` of height, already drawn once — the half-page motions
/// resolve against the LAST render's height, so a key test has to draw
/// before pressing anything.
struct Rig {
    lines: Vec<DiffLine>,
    state: DiffState,
    terminal: Terminal<TestBackend>,
}

impl Rig {
    fn new(line_count: usize, rows: u16) -> Self {
        let lines = (0..line_count)
            .map(|n| DiffLine {
                kind: DiffLineKind::Context,
                text: format!("line{n:02}"),
            })
            .collect();
        let mut rig = Self {
            lines,
            state: DiffState::new(),
            terminal: Terminal::new(TestBackend::new(40, rows)).unwrap(),
        };
        rig.draw();
        rig
    }

    fn draw(&mut self) {
        let lines = &self.lines;
        let state = &mut self.state;
        self.terminal
            .draw(|frame| render(frame, frame.area(), lines, state))
            .unwrap();
    }

    fn press(&mut self, code: KeyCode) -> DiffKeyAction {
        self.press_with(code, KeyModifiers::NONE)
    }

    fn press_ctrl(&mut self, code: KeyCode) -> DiffKeyAction {
        self.press_with(code, KeyModifiers::CONTROL)
    }

    fn press_with(&mut self, code: KeyCode, modifiers: KeyModifiers) -> DiffKeyAction {
        let action = handle_key(
            &mut self.state,
            self.lines.len(),
            KeyEvent::new(code, modifiers),
        );
        self.draw();
        action
    }

    fn rendered(&self) -> String {
        let buffer = self.terminal.backend().buffer();
        let area = buffer.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// -- exit ---------------------------------------------------------------

#[test]
fn q_and_ctrl_c_leave_the_renderer() {
    let mut rig = Rig::new(50, 10);
    assert_eq!(rig.press(KeyCode::Char('q')), DiffKeyAction::Exit);
    assert_eq!(rig.press_ctrl(KeyCode::Char('c')), DiffKeyAction::Exit);
}

#[test]
fn a_bare_c_does_not_leave_the_renderer() {
    let mut rig = Rig::new(50, 10);
    assert_eq!(rig.press(KeyCode::Char('c')), DiffKeyAction::Continue);
}

// -- scrolling ----------------------------------------------------------

#[test]
fn j_and_down_both_scroll_one_line() {
    for code in [KeyCode::Char('j'), KeyCode::Down] {
        let mut rig = Rig::new(50, 10);
        rig.press(code);
        assert_eq!(rig.state.offset(), 1, "{code:?}");
    }
}

#[test]
fn k_and_up_both_scroll_back_one_line() {
    for code in [KeyCode::Char('k'), KeyCode::Up] {
        let mut rig = Rig::new(50, 10);
        rig.press(KeyCode::Char('j'));
        rig.press(KeyCode::Char('j'));
        rig.press(code);
        assert_eq!(rig.state.offset(), 1, "{code:?}");
    }
}

/// Half of the VISIBLE height, which is the pane less its two borders — so
/// a 10-row pane moves by four, and dragging the pane taller moves further.
#[test]
fn ctrl_d_and_ctrl_u_move_half_the_visible_height() {
    let mut rig = Rig::new(50, 10);
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.state.offset(), 4);
    rig.press_ctrl(KeyCode::Char('u'));
    assert_eq!(rig.state.offset(), 0);
}

/// A pane too short to show two rows still moves by one. Halving to zero
/// would read as a broken key rather than as a small pane.
#[test]
fn a_pane_too_short_to_halve_still_moves_by_one() {
    let mut rig = Rig::new(50, 3);
    rig.press_ctrl(KeyCode::Char('d'));
    assert_eq!(rig.state.offset(), 1);
}

// -- clamping -----------------------------------------------------------

/// Scrolling past the end into blank rows would let the user lose the
/// document and have to guess their way back.
#[test]
fn scrolling_down_stops_with_the_last_line_on_screen() {
    let mut rig = Rig::new(12, 10);
    for _ in 0..50 {
        rig.press(KeyCode::Char('j'));
    }
    // 12 lines, 8 visible rows: the furthest useful offset is 4.
    assert_eq!(rig.state.offset(), 4);
    assert!(rig.rendered().contains("line11"), "{}", rig.rendered());
}

#[test]
fn scrolling_up_stops_at_the_top() {
    let mut rig = Rig::new(50, 10);
    for _ in 0..50 {
        rig.press(KeyCode::Char('k'));
    }
    assert_eq!(rig.state.offset(), 0);
}

/// A document shorter than the pane cannot scroll at all.
#[test]
fn a_document_that_fits_does_not_scroll() {
    let mut rig = Rig::new(3, 20);
    rig.press(KeyCode::Char('j'));
    rig.press_ctrl(KeyCode::Char('d'));
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.state.offset(), 0);
}

#[test]
fn an_empty_document_cannot_scroll() {
    let mut rig = Rig::new(0, 10);
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.state.offset(), 0);
}

// -- jumps --------------------------------------------------------------

#[test]
fn capital_g_jumps_to_the_end() {
    let mut rig = Rig::new(12, 10);
    rig.press(KeyCode::Char('G'));
    assert_eq!(rig.state.offset(), 4);
}

#[test]
fn gg_jumps_to_the_top() {
    let mut rig = Rig::new(50, 10);
    rig.press(KeyCode::Char('G'));
    assert!(rig.state.offset() > 0);

    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.press(KeyCode::Char('g')), DiffKeyAction::Continue);
    assert_eq!(rig.state.offset(), 0);
}

/// A lone `g` arms the chord and moves nothing, and it never expires —
/// exactly as in the tree pane (AgentTreeGgChordNeverExpires).
#[test]
fn a_lone_g_moves_nothing() {
    let mut rig = Rig::new(50, 10);
    rig.press(KeyCode::Char('j'));
    rig.press(KeyCode::Char('g'));
    assert_eq!(rig.state.offset(), 1);
}

/// Any other key disarms the chord and is then handled normally, so the
/// swallowed `g` is the only trace a lone press leaves.
#[test]
fn a_key_between_the_two_gs_disarms_the_chord() {
    let mut rig = Rig::new(50, 10);
    rig.press(KeyCode::Char('G'));
    let before = rig.state.offset();

    rig.press(KeyCode::Char('g'));
    rig.press(KeyCode::Char('k'));
    rig.press(KeyCode::Char('g'));

    assert_eq!(rig.state.offset(), before - 1);
}

// -- the pane has no toggle of its own ----------------------------------

/// Space, Enter and the all-files key do nothing here. The open set is
/// decided in the tree and only in the tree, which is what makes one file
/// have one open state — see DiffPaneHasNoToggleOfItsOwn in the spec.
#[test]
fn the_trees_toggle_keys_do_nothing_in_this_pane() {
    for code in [KeyCode::Char(' '), KeyCode::Enter, KeyCode::Char('a')] {
        let mut rig = Rig::new(50, 10);
        rig.press(KeyCode::Char('j'));
        assert_eq!(rig.press(code), DiffKeyAction::Continue, "{code:?}");
        assert_eq!(rig.state.offset(), 1, "{code:?}");
    }
}

// -- notices ------------------------------------------------------------

#[test]
fn a_notice_is_shown_and_cleared_by_the_next_key() {
    let mut rig = Rig::new(50, 10);
    rig.state.notice = Some("git: index.lock".to_string());
    rig.draw();
    assert!(rig.rendered().contains("index.lock"), "{}", rig.rendered());

    rig.press(KeyCode::Char('j'));
    assert!(rig.state.notice.is_none());
}

// -- truncation ---------------------------------------------------------

/// A long line is cut at the pane's edge, not wrapped onto a second row.
/// Wrapping would let one minified line push several files off screen — see
/// DiffLinesTruncateRatherThanWrap in docs/specs/agent-tree.allium.
#[test]
fn a_long_line_is_truncated_rather_than_wrapped() {
    let mut rig = Rig::new(0, 6);
    rig.lines = vec![
        DiffLine {
            kind: DiffLineKind::Added,
            text: format!("+{}", "x".repeat(200)),
        },
        DiffLine {
            kind: DiffLineKind::Context,
            text: "second".to_string(),
        },
    ];
    rig.draw();

    let rendered = rig.rendered();
    assert!(
        rendered.contains("second"),
        "the next line must still be on screen; got:\n{rendered}"
    );
}

/// press_every_row_key / RecordedActionMatchesRow for the diff pane.
#[test]
fn pressing_each_key_of_each_diff_row_records_the_rows_action() {
    use crate::keybindings::{bindings_in, KeyNamespace};
    let mut pressed = 0;
    for binding in bindings_in(KeyNamespace::AgentDiff) {
        assert_eq!(binding.context, None);
        for key in binding.keys {
            let mut rig = Rig::new(60, 12);
            if *key == "gg" {
                rig.press(KeyCode::Char('g'));
                assert!(rig.state.usage.is_empty(), "first g is pending input");
                rig.press(KeyCode::Char('g'));
            } else {
                let ev = crate::cli::test_key_event(key);
                handle_key(&mut rig.state, rig.lines.len(), ev);
            }
            pressed += 1;
            let detail = match *key {
                k if k.starts_with("Ctrl+") => k[5..].to_lowercase(),
                k => k.to_string(),
            };
            let got: Vec<_> = rig
                .state
                .usage
                .iter()
                .map(|e| (e.action.clone(), e.detail.clone()))
                .collect();
            assert_eq!(
                got,
                vec![(binding.action.to_string(), Some(detail))],
                "{key}"
            );
        }
    }
    assert!(pressed >= 10, "{pressed}");
}

/// Keys with no row — Space, Enter, `a`, Tab, and a modified `j` — do
/// nothing and record nothing.
#[test]
fn a_press_with_no_row_does_nothing_in_the_diff_pane() {
    for key in ["Space", "Enter", "a", "Tab", "Ctrl+J", "Ctrl+G", "Ctrl+Q"] {
        let mut rig = Rig::new(60, 12);
        let ev = crate::cli::test_key_event(key);
        assert_eq!(
            handle_key(&mut rig.state, rig.lines.len(), ev),
            DiffKeyAction::Continue,
            "{key}"
        );
        assert!(rig.state.usage.is_empty(), "{key}");
        assert_eq!(rig.state.offset, 0, "{key}");
    }
}
