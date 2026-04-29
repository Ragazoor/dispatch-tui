use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::models::{Learning, LearningKind, LearningScope};
use crate::tui::ui::palette::{BORDER, CYAN, DARK_GRAY, GREEN, MAGENTA, RED, WHITE, YELLOW};
use crate::tui::{App, ViewMode};

fn kind_color(kind: LearningKind) -> Color {
    match kind {
        LearningKind::Pitfall => RED,
        LearningKind::Convention => CYAN,
        LearningKind::Preference => MAGENTA,
        LearningKind::Procedural => YELLOW,
        LearningKind::ToolRecommendation => GREEN,
        LearningKind::Episodic => DARK_GRAY,
    }
}

fn scope_label(learning: &Learning) -> String {
    match learning.scope {
        LearningScope::User => "user".to_string(),
        LearningScope::Project => format!(
            "project ({})",
            learning.scope_ref.as_deref().unwrap_or("?")
        ),
        LearningScope::Repo => format!(
            "repo ({})",
            learning.scope_ref.as_deref().unwrap_or("?")
        ),
        LearningScope::Epic => format!(
            "epic ({})",
            learning.scope_ref.as_deref().unwrap_or("?")
        ),
        LearningScope::Task => format!(
            "task ({})",
            learning.scope_ref.as_deref().unwrap_or("?")
        ),
    }
}

pub fn render_proposed_learnings(frame: &mut Frame, app: &App, area: Rect) {
    let ViewMode::ProposedLearnings {
        selected,
        ref learnings,
        ..
    } = app.board.view_mode
    else {
        return;
    };

    // Overlay: 90% width, 75% height, centered
    let w = (area.width * 9 / 10).max(40);
    let h = (area.height * 3 / 4).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let overlay_area = Rect::new(x, y, w, h);

    frame.render_widget(Clear, overlay_area);

    let title = format!(" Proposed Learnings ({}) ", learnings.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER));

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    if learnings.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No proposed learnings \u{2014} you're all caught up.",
                Style::default().fg(WHITE),
            )),
        ])
        .wrap(Wrap { trim: false });
        frame.render_widget(msg, inner);
        return;
    }

    // Split inner into content area (all but last row) and footer (last row)
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let content_area = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let footer_area = Rect {
        y: footer_y,
        height: 1,
        ..inner
    };

    // Build lines with scope group headers
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut last_scope: Option<String> = None;
    let summary_width = content_area.width.saturating_sub(30) as usize;

    for (idx, learning) in learnings.iter().enumerate() {
        let scope_key = scope_label(learning);
        if last_scope.as_deref() != Some(&scope_key) {
            let fill_len = (content_area.width as usize).saturating_sub(scope_key.len() + 5);
            let sep = format!("\u{2500}\u{2500} {} {}", scope_key, "\u{2500}".repeat(fill_len));
            lines.push(Line::from(Span::styled(sep, Style::default().fg(DARK_GRAY))));
            last_scope = Some(scope_key);
        }

        let is_selected = idx == selected;
        let cursor = if is_selected { "> " } else { "  " };
        let kind_str = format!("{:<18}", learning.kind.display_label());
        let summary: String = if learning.summary.chars().count() > summary_width {
            learning
                .summary
                .chars()
                .take(summary_width.saturating_sub(1))
                .collect::<String>()
                + "\u{2026}"
        } else {
            learning.summary.clone()
        };
        let tags: String = learning
            .tags
            .iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" ");

        let base_style = if is_selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled(cursor.to_string(), base_style.fg(CYAN)),
            Span::styled(format!("[{:>3}] ", learning.id), base_style.fg(DARK_GRAY)),
            Span::styled(kind_str, base_style.fg(kind_color(learning.kind))),
            Span::styled(summary, base_style.fg(WHITE)),
            Span::styled(format!("  {tags}"), base_style.fg(DARK_GRAY)),
        ]));
    }

    let content = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(content, content_area);

    // Footer hints
    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "[a]",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" approve  "),
        Span::styled(
            "[r]",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" reject  "),
        Span::styled(
            "[e]",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" edit  "),
        Span::styled(
            "[j/k]",
            Style::default()
                .fg(DARK_GRAY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" navigate  "),
        Span::styled(
            "[q]",
            Style::default()
                .fg(DARK_GRAY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" close"),
    ]));
    frame.render_widget(hints, footer_area);
}
