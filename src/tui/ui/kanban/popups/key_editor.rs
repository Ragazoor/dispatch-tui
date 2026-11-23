//! The key editor (`KeyEditor` in `docs/specs/keybindings.allium`): every
//! row of the table with its current, default, pending and staged keys, the
//! problems the loader's checks find in the staged edits, and the editor's
//! keys in a footer built from the table.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{BorderType, Paragraph},
    Frame,
};

use super::help::wrap;
use crate::keybindings::{hint_key, hint_keys, KeyBinding, KeyNamespace as N, KEY_BINDINGS};
use crate::palette::{CYAN, FG, MUTED, RED, YELLOW};
use crate::tui::key_editor::KeyEditor;
use crate::tui::ui::shared::{centered_rect, open_overlay, titled_block};
use crate::tui::{App, InputMode};

pub(in crate::tui::ui::kanban) fn render_key_editor_overlay(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) {
    let Some(editor) = app.input.key_editor.as_deref() else {
        return;
    };
    let width = (area.width * 90 / 100).clamp(40, 120);
    let height = (area.height * 85 / 100).clamp(12, 50);
    let block = titled_block(
        CYAN,
        BorderType::Double,
        " Keys \u{2014} changes apply when the board next starts ".to_string(),
    );
    let inner = open_overlay(frame, centered_rect(area, width, height), block);
    let w = usize::from(inner.width).max(10);

    let head = header_lines(editor, w);
    let foot = footer_lines(
        app.key_table,
        editor,
        app.input.mode == InputMode::KeyEditor,
        w,
    );
    let (body, first, last) = row_lines(app.key_table, editor, w);
    let body_height = usize::from(inner.height).saturating_sub(head.len() + foot.len());
    // Keep the whole cursor row, its staged keys and problems included, in
    // view; a row taller than the body shows from its first line.
    let mut top = editor.scroll.get();
    if body_height > 0 && last >= top + body_height {
        top = last + 1 - body_height;
    }
    if first < top {
        top = first;
    }
    editor.scroll.set(top);

    let mut lines = head;
    let shown: Vec<Line<'static>> = body.into_iter().skip(top).take(body_height).collect();
    let pad = body_height.saturating_sub(shown.len());
    lines.extend(shown);
    lines.extend(std::iter::repeat_n(Line::raw(""), pad));
    lines.extend(foot);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn wrapped(text: &str, width: usize, indent: &str, style: Style) -> Vec<Line<'static>> {
    wrap(text, width.saturating_sub(indent.len()).max(10))
        .into_iter()
        .map(|l| Line::from(Span::styled(format!("{indent}{l}"), style)))
        .collect()
}

/// The lines above the rows: the file's own error or refused entries, the
/// problems no row owns, and the editor's last notice.
fn header_lines(editor: &KeyEditor, w: usize) -> Vec<Line<'static>> {
    let red = Style::default().fg(RED);
    let mut lines = Vec::new();
    if let Some(e) = editor.file_error() {
        lines.extend(wrapped(e, w, "", red));
    } else {
        for e in &editor.file_errors {
            lines.extend(wrapped(
                &format!("In the file, refused at load: {}", e.message),
                w,
                "",
                Style::default().fg(MUTED),
            ));
        }
    }
    for p in editor.problems.iter().filter(|p| p.row.is_none()) {
        lines.extend(wrapped(&p.message, w, "", red));
    }
    if let Some(n) = &editor.notice {
        lines.extend(wrapped(n, w, "", Style::default().fg(YELLOW)));
    }
    lines
}

/// The counts and, while the editor itself has the keys, its keys, built
/// from the table. A search, capture or confirmation over it names its own
/// keys on the status bar.
fn footer_lines(
    table: &'static [KeyBinding],
    editor: &KeyEditor,
    editor_keys: bool,
    w: usize,
) -> Vec<Line<'static>> {
    let counts = format!(
        "{} staged, {} problems",
        editor.staged.len(),
        editor.problems.len()
    );
    if !editor_keys {
        return wrapped(&counts, w, "", Style::default().fg(CYAN));
    }
    let ns = N::BoardKeyEditor;
    let key = |action, shown| hint_key(table, ns, None, action, shown);
    let text = format!(
        "{counts}   [{}] move  [{}] pick  [{}] add  [{}] chord  [{}] replace  [{}] remove  [{}] default  [{}] save  [{}] search  [{}] close",
        hint_keys(table, ns, &[("key_editor_row_down", "j"), ("key_editor_row_up", "k")]),
        hint_keys(
            table,
            ns,
            &[("key_editor_previous_key", "h"), ("key_editor_next_key", "l")]
        ),
        key("key_editor_add_key", "a"),
        key("key_editor_add_chord", "c"),
        key("key_editor_replace_key", "Enter"),
        key("key_editor_remove_key", "x"),
        key("key_editor_restore_default", "r"),
        key("key_editor_save", "w"),
        key("key_editor_search", "/"),
        key("key_editor_close", "Esc"),
    );
    wrapped(&text, w, "", Style::default().fg(CYAN))
}

/// The rows, grouped by namespace, and the first and last line of the
/// cursor row.
fn row_lines(
    table: &[KeyBinding],
    editor: &KeyEditor,
    w: usize,
) -> (Vec<Line<'static>>, usize, usize) {
    let indent = "      ";
    let mut lines = Vec::new();
    let (mut first, mut last) = (0, 0);
    let mut namespace = None;
    for (pos, row) in editor.visible(table).into_iter().enumerate() {
        let b = &table[row];
        if namespace != Some(b.namespace) {
            namespace = Some(b.namespace);
            lines.push(Line::from(Span::styled(
                format!("== {} ==", b.namespace.name()),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            )));
        }
        let at_cursor = pos == editor.cursor;
        if at_cursor {
            first = lines.len();
        }
        let style = if at_cursor {
            Style::default().fg(FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(FG)
        };
        let context = b
            .context
            .map(|c| format!(" ({})", c.words()))
            .unwrap_or_default();
        let marker = if at_cursor { "\u{25b8} " } else { "  " };
        let main = wrap(
            &format!(
                "{}  {}{context}: {}",
                b.keys.join(", "),
                b.action,
                b.description
            ),
            w.saturating_sub(4).max(10),
        );
        for (i, l) in main.into_iter().enumerate() {
            let lead = if i == 0 { marker } else { "    " };
            lines.push(Line::from(Span::styled(format!("{lead}{l}"), style)));
        }
        if at_cursor {
            lines.push(picked_line(editor, row, indent));
        }
        let mut facts = Vec::new();
        if b.keys != KEY_BINDINGS[row].keys {
            facts.push(format!("default: {}", KEY_BINDINGS[row].keys.join(", ")));
        }
        if !editor
            .file_keys(row)
            .iter()
            .map(String::as_str)
            .eq(b.keys.iter().copied())
        {
            facts.push(format!(
                "after restart: {}",
                editor.file_keys(row).join(", ")
            ));
        }
        if let Some(staged) = editor.staged.get(&row) {
            let shown = if staged.is_empty() {
                format!("{} (default)", KEY_BINDINGS[row].keys.join(", "))
            } else {
                staged.join(", ")
            };
            facts.push(format!("staged: {shown}"));
        }
        if !facts.is_empty() {
            lines.extend(wrapped(
                &facts.join("   "),
                w,
                indent,
                Style::default().fg(YELLOW),
            ));
        }
        for p in editor.problems_of(row) {
            lines.extend(wrapped(p, w, indent, Style::default().fg(RED)));
        }
        if at_cursor {
            last = lines.len().saturating_sub(1);
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No row matches the search.".to_string(),
            Style::default().fg(MUTED),
        )));
    }
    (lines, first, last)
}

/// The cursor row's keys as an edit starts from them, the picked one
/// marked.
fn picked_line(editor: &KeyEditor, row: usize, indent: &str) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{indent}editing: "),
        Style::default().fg(MUTED),
    )];
    for (i, k) in editor.editing_keys(row).iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        let style = if i == editor.picked {
            Style::default().fg(CYAN).add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(CYAN)
        };
        spans.push(Span::styled(k.clone(), style));
    }
    Line::from(spans)
}
