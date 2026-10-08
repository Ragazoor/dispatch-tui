//! Help overlay, rendered from the keybinding table
//! (`HelpOverlayIsTheTable` in `docs/specs/keybindings.allium`).

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{BorderType, Paragraph},
    Frame,
};

use crate::keybindings::{bindings_in, KeyNamespace, KeyReceiver};
use crate::palette::CYAN;
use crate::tui::ui::shared::{centered_rect, open_overlay, titled_block, HintStyles};
use crate::tui::{App, InputMode};

/// Break `text` at spaces into lines of at most `width` characters. A word
/// longer than the width stands on a line of its own.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The overlay body: every row of the table, grouped by namespace in
/// `KeyNamespace` order, wrapped to `width`.
fn body_lines(width: usize, styles: &HintStyles) -> Vec<Line<'static>> {
    let (header, key, desc, note) = (styles.accent, styles.accent, styles.desc, styles.note);
    let indent = "    ";
    let wrap_width = width.saturating_sub(indent.len()).max(10);
    let mut lines = Vec::new();
    for ns in KeyNamespace::ALL {
        let mut rows = bindings_in(ns).peekable();
        if rows.peek().is_none() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        let mut title = format!("  == {} ==", ns.name());
        if ns.receiver() == KeyReceiver::Tmux {
            title.push_str(" (handled by tmux, not the board)");
        }
        lines.push(Line::from(Span::styled(title, header)));
        for b in rows {
            lines.push(Line::from(Span::styled(
                format!("  {}", b.keys.join(", ")),
                key,
            )));
            for l in wrap(b.description, wrap_width) {
                lines.push(Line::from(Span::styled(format!("{indent}{l}"), desc)));
            }
            if let Some(c) = b.context {
                for l in wrap(&format!("({})", c.words()), wrap_width) {
                    lines.push(Line::from(Span::styled(format!("{indent}{l}"), note)));
                }
            }
            if let Some(n) = b.note {
                for l in wrap(&format!("note: {n}"), wrap_width) {
                    lines.push(Line::from(Span::styled(format!("{indent}{l}"), note)));
                }
            }
        }
    }
    lines
}

pub(in crate::tui::ui::kanban) fn render_help_overlay(frame: &mut Frame, app: &App, area: Rect) {
    if app.input.mode != InputMode::Help {
        return;
    }

    let popup_width = (area.width * 80 / 100).clamp(40, 72);
    let popup_height = (area.height * 80 / 100).clamp(25, 36);
    let popup_area = centered_rect(area, popup_width, popup_height);

    // The hints sit on the border: every line inside it is body, so a scroll
    // offset of n puts body line n on the first row.
    let block = titled_block(
        CYAN,
        BorderType::Double,
        crate::keybindings::help_overlay_title(app.key_table),
    );
    let styles = HintStyles::new(CYAN);
    let inner = open_overlay(frame, popup_area, block);

    let lines = body_lines(inner.width as usize, &styles);
    let max_scroll = lines.len().saturating_sub(inner.height as usize);
    app.interaction.help_max_scroll.set(Some(max_scroll));
    let offset = app.interaction.help_scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
        inner,
    );
}
