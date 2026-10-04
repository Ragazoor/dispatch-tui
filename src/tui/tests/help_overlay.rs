//! `HelpOverlayIsTheTable` (`docs/specs/keybindings.allium`): the `?` overlay
//! is rendered from `KEY_BINDINGS` — every row, grouped under its namespace's
//! published name, keys beside description, context words and note where
//! present — and scrolls (board.help's `scroll_help` row) so that every row
//! is reachable however short the popup.
//!
//! These replace the hand-written-overlay checks that parsed `[k]` legends out
//! of the popup and compared them with the board.normal handler's source:
//! with the overlay generated from the table there is no second key list to
//! drift.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::keybindings::{bindings_in, KeyBinding, KeyNamespace, KEY_BINDINGS};
use crate::tui::commands::UsageCommand;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 40;

fn help_app() -> App {
    let mut app = App::new(vec![]);
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleHelp,
    ));
    assert_eq!(app.input.mode, InputMode::Help);
    app
}

/// The text lines *inside* the help popup's double border. The board renders
/// behind the overlay, so the popup is located by its `╔`/`╝` corners rather
/// than by recomputing the overlay's geometry.
fn help_popup_lines(buf: &ratatui::buffer::Buffer) -> Vec<String> {
    let area = buf.area();
    let mut top_left = None;
    let mut bottom_right = None;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            match buf[(x, y)].symbol() {
                "\u{2554}" => top_left = Some((x, y)),
                "\u{255d}" => bottom_right = Some((x, y)),
                _ => {}
            }
        }
    }
    let (x0, y0) = top_left.expect("help popup's top-left double-border corner not found");
    let (x1, y1) = bottom_right.expect("help popup's bottom-right double-border corner not found");
    (y0 + 1..y1)
        .map(|y| (x0 + 1..x1).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// One page of the overlay at scroll offset `offset`.
fn page(app: &mut App, offset: usize) -> Vec<String> {
    app.interaction.help_scroll = offset;
    help_popup_lines(&render_to_buffer(app, WIDTH, HEIGHT))
}

/// The overlay's whole body, read page by page: scrolling by one moves line
/// `n` of the body to the top of the popup, so the body is the first line of
/// each page up to the last offset that still scrolls, then that last page in
/// full. The rendering clamps an offset past the end, which is how the end is
/// found: the page stops changing.
fn overlay_body() -> Vec<String> {
    let mut app = help_app();
    let mut pages: Vec<Vec<String>> = Vec::new();
    for offset in 0..5000 {
        let p = page(&mut app, offset);
        if pages.last() == Some(&p) {
            let last = pages.pop().unwrap();
            let mut body: Vec<String> = pages.into_iter().map(|p| p[0].clone()).collect();
            body.extend(last);
            return body;
        }
        pages.push(p);
    }
    panic!("the help overlay kept changing for 5000 scroll steps; rendering must clamp the offset");
}

/// A whitespace-separated token of the body and the line it sits on.
struct Token {
    text: String,
    line: usize,
}

fn tokens_of(body: &[String]) -> Vec<Token> {
    body.iter()
        .enumerate()
        .flat_map(|(line, l)| {
            l.split_whitespace().map(move |t| Token {
                text: t.to_string(),
                line,
            })
        })
        .collect()
}

/// Every place `phrase` occurs among `tokens[from..to]`, as (first, last)
/// token indices. Its words must appear in order, each within a few tokens of
/// the previous one, so a phrase wrapped beside a keys column still matches;
/// the first word may carry leading decoration and the last trailing
/// punctuation.
fn phrase_matches(tokens: &[Token], phrase: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    const MAX_GAP: usize = 6;
    let words: Vec<&str> = phrase.split_whitespace().collect();
    let to = to.min(tokens.len());
    let mut found = Vec::new();
    if words.is_empty() {
        return found;
    }
    for start in from..to {
        let t = &tokens[start].text;
        let first_ok = if words.len() == 1 {
            t.contains(words[0])
        } else {
            t.ends_with(words[0])
        };
        if !first_ok {
            continue;
        }
        let mut at = start;
        let mut ok = true;
        for (i, w) in words.iter().enumerate().skip(1) {
            let last = i == words.len() - 1;
            let next = (at + 1..(at + 1 + MAX_GAP).min(to)).find(|&j| {
                let t = &tokens[j].text;
                if last {
                    t.starts_with(w)
                } else {
                    t == w
                }
            });
            match next {
                Some(j) => at = j,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            found.push((start, at));
        }
    }
    found
}

/// Whether a line is the group header for `ns`: one of its tokens, stripped of
/// decoration, is the namespace's published name.
fn is_header_for(line: &str, ns: KeyNamespace) -> bool {
    line.split_whitespace().any(|t| {
        t.trim_matches(|c: char| !(c.is_alphanumeric() || c == '.' || c == '_')) == ns.name()
    })
}

fn namespaces_with_rows() -> Vec<KeyNamespace> {
    KeyNamespace::ALL
        .iter()
        .copied()
        .filter(|ns| bindings_in(*ns).next().is_some())
        .collect()
}

/// Each namespace with rows mapped to the body lines of its group: from its
/// header to the next group's header. Headers must appear in KeyNamespace
/// order.
fn group_spans(body: &[String]) -> Vec<(KeyNamespace, usize, usize)> {
    let namespaces = namespaces_with_rows();
    let mut headers = Vec::new();
    let mut from = 0;
    for ns in &namespaces {
        let line = (from..body.len())
            .find(|&i| is_header_for(&body[i], *ns))
            .unwrap_or_else(|| {
                panic!(
                    "no `{}` group header after line {from} — groups must be headed by the \
                     published name, in KeyNamespace order",
                    ns.name()
                )
            });
        headers.push(line);
        from = line + 1;
    }
    namespaces
        .iter()
        .enumerate()
        .map(|(i, ns)| {
            let end = headers.get(i + 1).copied().unwrap_or(body.len());
            (*ns, headers[i] + 1, end)
        })
        .collect()
}

/// The token range of the lines `[from_line, to_line)`.
fn token_range(tokens: &[Token], from_line: usize, to_line: usize) -> (usize, usize) {
    let start = tokens
        .iter()
        .position(|t| t.line >= from_line)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .position(|t| t.line >= to_line)
        .unwrap_or(tokens.len());
    (start, end)
}

/// Why `binding` is not rendered within its group, or `None` if it is: its
/// description, with every key on the lines around it, its context's words
/// near it and its note just after it.
fn row_problem(
    body: &[String],
    tokens: &[Token],
    binding: &KeyBinding,
    span: (usize, usize),
) -> Option<String> {
    let (g0, g1) = token_range(tokens, span.0, span.1);
    let candidates = phrase_matches(tokens, binding.description, g0, g1);
    if candidates.is_empty() {
        return Some(format!("description {:?} not found", binding.description));
    }
    let mut last_problem = String::new();
    for (s, e) in candidates {
        let first_line = tokens[s].line.saturating_sub(2).max(span.0);
        let last_line = (tokens[e].line + 2).min(span.1.saturating_sub(1));
        let region = body[first_line..=last_line].join("\n");
        if let Some(k) = binding.keys.iter().find(|k| !region.contains(**k)) {
            last_problem = format!("key {k:?} not beside its description");
            continue;
        }
        if let Some(ctx) = binding.context {
            if phrase_matches(
                tokens,
                ctx.words(),
                s.saturating_sub(40).max(g0),
                (e + 40).min(g1),
            )
            .is_empty()
            {
                last_problem = format!("context {:?} not beside its description", ctx.words());
                continue;
            }
        }
        if let Some(note) = binding.note {
            if phrase_matches(tokens, note, s.saturating_sub(40).max(g0), (e + 80).min(g1))
                .is_empty()
            {
                last_problem = format!("note {note:?} not beside its description");
                continue;
            }
        }
        return None;
    }
    Some(last_problem)
}

/// HelpOverlayIsTheTable / OverlayIsGeneratedNotWritten: scrolling through
/// the overlay at 120x40 shows every row of KEY_BINDINGS under its
/// namespace's published name, keys beside description, with its context and
/// note.
#[test]
fn every_row_of_the_table_is_in_the_overlay_under_its_namespace() {
    let body = overlay_body();
    let tokens = tokens_of(&body);
    let spans = group_spans(&body);
    let mut failures = Vec::new();
    for binding in KEY_BINDINGS {
        let &(_, from, to) = spans
            .iter()
            .find(|(ns, _, _)| *ns == binding.namespace)
            .unwrap();
        if let Some(problem) = row_problem(&body, &tokens, binding, (from, to)) {
            failures.push(format!(
                "{} {} {:?}: {problem}",
                binding.namespace.name(),
                binding.action,
                binding.keys
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "rows the help overlay does not show:\n{}\n--- overlay body ---\n{}",
        failures.join("\n"),
        body.join("\n")
    );
}

/// The `G` row's note is the warning that motivated the table; it must be on
/// screen in the overlay, not only in list_keybindings.
#[test]
fn the_last_row_keys_warning_is_in_the_overlay() {
    let body = overlay_body();
    let tokens = tokens_of(&body);
    let g = bindings_in(KeyNamespace::BoardNormal)
        .find(|b| b.action == "navigate_row_last")
        .unwrap();
    let note = g.note.unwrap();
    let spans = group_spans(&body);
    let &(_, from, to) = spans
        .iter()
        .find(|(ns, _, _)| *ns == KeyNamespace::BoardNormal)
        .unwrap();
    let (t0, t1) = token_range(&tokens, from, to);
    assert!(
        !phrase_matches(&tokens, note, t0, t1).is_empty(),
        "G's note {note:?} is not in the board.normal group:\n{}",
        body.join("\n")
    );
    assert_eq!(row_problem(&body, &tokens, g, (from, to)), None);
}

/// tmux.global is the last group: reachable by scrolling, and marked by its
/// published name.
#[test]
fn tmux_global_rows_are_in_the_overlay() {
    let body = overlay_body();
    let tokens = tokens_of(&body);
    let spans = group_spans(&body);
    let &(_, from, to) = spans
        .iter()
        .find(|(ns, _, _)| *ns == KeyNamespace::TmuxGlobal)
        .expect("a tmux.global group");
    let rows: Vec<_> = bindings_in(KeyNamespace::TmuxGlobal).collect();
    assert!(rows.len() >= 2);
    for b in rows {
        assert_eq!(
            row_problem(&body, &tokens, b, (from, to)),
            None,
            "{}",
            b.action
        );
    }
    let group = body[from..to].join("\n");
    assert!(group.contains("Prefix+Space"), "{group}");
    assert!(group.contains("Prefix+e"), "{group}");
}

/// The overlay is longer than the popup at 120x40, and an offset of one moves
/// the body up one line.
#[test]
fn the_overlay_scrolls_one_line_per_step() {
    let mut app = help_app();
    let first = page(&mut app, 0);
    let second = page(&mut app, 1);
    assert_ne!(
        first, second,
        "the table does not fit one page, so offset 1 must scroll"
    );
    assert_eq!(first[1], second[0]);
}

/// An offset past the end renders the last page rather than an empty popup
/// or a panic: the rendering clamps.
#[test]
fn an_offset_past_the_end_is_clamped_to_the_last_page() {
    let mut app = help_app();
    let far = page(&mut app, 100_000);
    let text = far.join("\n");
    let last = KEY_BINDINGS.last().unwrap();
    assert!(
        text.contains(last.keys[0]),
        "the last page should show the table's last row ({:?}):\n{text}",
        last.keys
    );
    assert_eq!(far, page(&mut app, 200_000));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn recorded(cmds: &[Command]) -> Vec<(String, Option<String>)> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Usage(UsageCommand::Record(e)) => Some((e.action.clone(), e.detail.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn j_and_down_scroll_the_overlay_down_k_and_up_scroll_it_up() {
    let mut app = help_app();
    assert_eq!(app.interaction.help_scroll, 0);
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.interaction.help_scroll, 1);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.interaction.help_scroll, 2);
    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.interaction.help_scroll, 1);
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.interaction.help_scroll, 0);
    // Scrolling up from the top stays at the top.
    let cmds = app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.interaction.help_scroll, 0);
    assert_eq!(app.input.mode, InputMode::Help);
    assert_eq!(
        recorded(&cmds),
        vec![("scroll_help".to_string(), Some("k".to_string()))],
        "a press is recorded like any other key"
    );
}

#[test]
fn opening_the_overlay_starts_at_the_top() {
    let mut app = help_app();
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.interaction.help_scroll, 2);
    // Close with `?`, reopen from the board with `?`.
    app.handle_key(key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Normal);
    app.handle_key(key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Help);
    assert_eq!(app.interaction.help_scroll, 0);
}

/// Scrolling with the keys reaches what rendering at that offset shows.
#[test]
fn scrolling_with_keys_moves_the_rendered_page() {
    let mut app = help_app();
    let top = help_popup_lines(&render_to_buffer(&mut app, WIDTH, HEIGHT));
    app.handle_key(key(KeyCode::Char('j')));
    let next = help_popup_lines(&render_to_buffer(&mut app, WIDTH, HEIGHT));
    assert_eq!(top[1], next[0]);
}
