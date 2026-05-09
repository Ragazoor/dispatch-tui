//! Confirmation overlays and popup helpers (error, tips, help, repo filter, task detail).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::{App, InputMode, RepoFilterMode, ViewMode};

use super::super::palette::{BORDER, FG, MUTED, MUTED_LIGHT};
use super::wrapped_line_count;

pub(super) fn render_error_popup(frame: &mut Frame, app: &App, area: Rect) {
    let Some(error_msg) = &app.status.error_popup else {
        return;
    };

    let popup_width = (area.width * 60 / 100).clamp(30, 60);
    let popup_height = 7_u16;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Error ")
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(Color::Red))
        .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            error_msg.as_str(),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to dismiss",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_tips_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let Some(overlay) = &app.tips else {
        return;
    };
    let Some(tip) = overlay.current_tip() else {
        return;
    };

    // Center: 70% width, 60% height
    let popup_width = (area.width * 70 / 100).clamp(40, 80);
    let popup_height = (area.height * 60 / 100).clamp(12, 30);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let total = overlay.tips.len();
    let current = overlay.index + 1;
    let title = format!(" Tips & Tricks ({current} / {total}) ");

    let block = Block::default()
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Reserve bottom 2 rows: 1 blank + 1 footer
    let content_height = inner.height.saturating_sub(2);
    let content_area = Rect::new(inner.x, inner.y, inner.width, content_height);
    let footer_area = Rect::new(inner.x, inner.y + content_height + 1, inner.width, 1);

    // Content: tip title (bold) + blank line + body
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            tip.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for body_line in tip.body.lines() {
        lines.push(Line::from(Span::styled(
            body_line.to_string(),
            Style::default().fg(Color::Gray),
        )));
    }

    // NEW badge: bottom-right of content area when tip is unseen
    if overlay.is_new() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(inner.width.saturating_sub(5) as usize)),
            Span::styled(
                "NEW",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    let content = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(content, content_area);

    // Footer: key hints
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("h/←", Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled("l/→", Style::default().fg(Color::Yellow)),
        Span::raw(" browse  "),
        Span::styled("[n]", Style::default().fg(Color::Yellow)),
        Span::raw(" new only  "),
        Span::styled("[x]", Style::default().fg(Color::Yellow)),
        Span::raw(" never  "),
        Span::styled("[q]", Style::default().fg(Color::Yellow)),
        Span::raw(" close"),
    ]));
    frame.render_widget(footer, footer_area);
}

pub(super) fn render_help_overlay(frame: &mut Frame, app: &App, area: Rect) {
    if app.input.mode != InputMode::Help {
        return;
    }

    let popup_width = (area.width * 80 / 100).clamp(40, 72);
    let popup_height = (area.height * 80 / 100).clamp(25, 36);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let header = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(Color::Gray);
    let note = Style::default().fg(Color::DarkGray);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Navigation", header)),
        Line::from(vec![
            Span::styled("  [h/\u{2190}]", key),
            Span::styled(" prev column     ", desc),
            Span::styled("[j/\u{2193}]", key),
            Span::styled(" next task", desc),
        ]),
        Line::from(vec![
            Span::styled("  [l/\u{2192}]", key),
            Span::styled(" next column     ", desc),
            Span::styled("[k/\u{2191}]", key),
            Span::styled(" prev task", desc),
        ]),
        Line::from(vec![
            Span::styled("  [Enter]", key),
            Span::styled(" task detail      ", desc),
            Span::styled("[e]", key),
            Span::styled(" edit / enter epic", desc),
        ]),
        Line::from(vec![
            Span::styled("  [q]", key),
            Span::styled(" exit epic (in epic)   ", desc),
            Span::styled("[Esc]", key),
            Span::styled(" clear selection", desc),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Actions", header)),
        Line::from(vec![
            Span::styled("  [n]", key),
            Span::styled(" new task   ", desc),
            Span::styled("[E]", key),
            Span::styled(" new epic   ", desc),
            Span::styled("[N]", key),
            Span::styled(" notifications", desc),
        ]),
        Line::from(vec![
            Span::styled("  [d]", key),
            Span::styled(" dispatch*  ", desc),
            Span::styled("[H/L]", key),
            Span::styled(" move task/epic backward/forward", desc),
        ]),
        Line::from(vec![
            Span::styled("  [x]", key),
            Span::styled(" archive    ", desc),
            Span::styled("[D]", key),
            Span::styled(" quick dsp  ", desc),
            Span::styled("[g]", key),
            Span::styled(" session/board", desc),
        ]),
        Line::from(vec![
            Span::styled("  [G]", key),
            Span::styled(" session    ", desc),
            Span::styled("(epic: jump to subtask tmux)", note),
        ]),
        Line::from(vec![
            Span::styled("  [h/\u{2190}]", key),
            Span::styled(" Projects  ", desc),
            Span::styled("[l/\u{2192}]", key),
            Span::styled(" Archive   ", desc),
            Span::styled("[a]", key),
            Span::styled(" select all", desc),
        ]),
        Line::from(vec![
            Span::styled("  [Space]", key),
            Span::styled(" select  ", desc),
            Span::styled("[f]", key),
            Span::styled(" filter repos  ", desc),
            Span::styled("[W]", key),
            Span::styled(" wrap up  ", desc),
            Span::styled("(task/epic)", note),
        ]),
        Line::from(vec![
            Span::styled("  [T]", key),
            Span::styled(" detach tmux panel  ", desc),
            Span::styled("(any task with a tmux window, supports batch)", note),
        ]),
        Line::from(vec![
            Span::styled("  [S]", key),
            Span::styled(" toggle split mode  ", desc),
            Span::styled("(side-by-side with agent)", note),
        ]),
        Line::from(vec![
            Span::styled("  [P]", key),
            Span::styled(" merge PR  ", desc),
            Span::styled("[p]", key),
            Span::styled(" open PR in browser", desc),
        ]),
        Line::from(vec![
            Span::styled("  [J/K]", key),
            Span::styled(" reorder item up/down in column", desc),
        ]),
        Line::from(""),
        Line::from(Span::styled("  * [d] is context-dependent:", note)),
        Line::from(Span::styled(
            "    Backlog (no plan) \u{2192} brainstorm",
            note,
        )),
        Line::from(Span::styled(
            "    Backlog (has plan) \u{2192} dispatch",
            note,
        )),
        Line::from(Span::styled(
            "    Running \u{2192} resume (if window gone)",
            note,
        )),
        Line::from(Span::styled(
            "    Epic \u{2192} dispatch next backlog subtask",
            note,
        )),
        Line::from(""),
        Line::from(Span::styled("  General", header)),
        Line::from(vec![
            Span::styled("  [?]", key),
            Span::styled(" this help  ", desc),
            Span::styled("[N]", key),
            Span::styled(" notify on/off  ", desc),
            Span::styled("[q]", key),
            Span::styled(" quit (or exit epic)", desc),
        ]),
        Line::from(""),
        Line::from(Span::styled("  [?] or [Esc] to close", note)),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_repo_filter_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let is_filter_mode = matches!(
        app.mode(),
        InputMode::RepoFilter
            | InputMode::InputPresetName
            | InputMode::ConfirmDeletePreset
            | InputMode::ConfirmDeleteRepoPath
    );
    if !is_filter_mode {
        return;
    }

    let repo_count = app.board.repo_paths.len();
    let preset_count = app.filter_presets().len();
    let preset_lines = if preset_count > 0 {
        preset_count + 2
    } else {
        0
    }; // header + presets + blank line
    let input_line = if matches!(app.mode(), InputMode::InputPresetName) {
        1
    } else {
        0
    };
    // Cap popup height to screen minus 4; repos may scroll if they don't fit
    // +6: blank(1) + preset_lines + blank(1) + 2_help_lines(2) + borders(2)
    let popup_height = (repo_count as u16 + preset_lines as u16 + input_line as u16 + 6)
        .clamp(7, area.height.saturating_sub(4));
    let popup_width = (area.width * 70 / 100).clamp(30, 60);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // How many repo rows fit: content height minus non-repo fixed lines
    // non_repo = blank(1) + preset_lines + blank(1) + 2_help_lines(2) = 4 + preset_lines
    let content_height = popup_height.saturating_sub(2) as usize; // minus borders
    let non_repo_lines = preset_lines + input_line + 4; // blank + presets + blank + 2 help lines
    let visible_repos = content_height.saturating_sub(non_repo_lines).max(1);

    let cursor = app.input.repo_cursor;
    let scroll = if repo_count <= visible_repos {
        0
    } else {
        cursor
            .saturating_sub(visible_repos - 1)
            .min(repo_count - visible_repos)
    };

    frame.render_widget(Clear, popup_area);

    let mode_label = app.repo_filter_mode().as_str();
    let block = Block::default()
        .title(format!(" Repo Filter ({mode_label}) "))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::Gray);
    let note_style = Style::default().fg(Color::DarkGray);
    let cursor_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let mut lines = vec![Line::from("")];

    // Presets section
    if !app.filter_presets().is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  Presets:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )]));
        for (i, (name, _, mode)) in app.filter_presets().iter().enumerate() {
            let letter = (b'A' + i as u8) as char;
            let mode_tag = match mode {
                RepoFilterMode::Include => "",
                RepoFilterMode::Exclude => " (excl)",
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {letter}"), key_style),
                Span::styled(format!(". {name}{mode_tag}"), desc_style),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Repo list (scrollable)
    if scroll > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ↑ {} more", scroll),
            note_style,
        )));
    }
    let broken_style = Style::default().fg(Color::DarkGray);
    for (i, path) in app
        .repo_paths()
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_repos)
    {
        let checked = if app.repo_filter().contains(path) {
            "x"
        } else {
            " "
        };
        let is_broken = !std::path::Path::new(path).is_dir();
        let broken_mark = if is_broken { " [!]" } else { "" };
        if i == cursor {
            let style = if is_broken {
                broken_style
            } else {
                cursor_style
            };
            lines.push(Line::from(vec![
                Span::styled("  ►", style),
                Span::styled(format!(" [{checked}] {path}{broken_mark}"), style),
            ]));
        } else {
            let num = i + 1;
            let style = if is_broken { broken_style } else { desc_style };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {num}"),
                    if is_broken { broken_style } else { key_style },
                ),
                Span::styled(format!(". [{checked}] {path}{broken_mark}"), style),
            ]));
        }
    }
    let remaining = repo_count.saturating_sub(scroll + visible_repos);
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ↓ {} more", remaining),
            note_style,
        )));
    }

    lines.push(Line::from(""));

    // Input line for preset name
    if matches!(app.mode(), InputMode::InputPresetName) {
        lines.push(Line::from(vec![
            Span::styled("  Name: ", key_style),
            Span::styled(app.input_buffer(), Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::DarkGray)),
        ]));
    }

    // Help text
    let all_selected = app.repo_filter().len() == app.board.repo_paths.len();
    let a_label = if all_selected {
        "clear all"
    } else {
        "select all"
    };
    match app.mode() {
        InputMode::InputPresetName => {
            lines.push(Line::from(vec![
                Span::styled("  [Enter]", key_style),
                Span::styled(" save  ", note_style),
                Span::styled("[Esc]", key_style),
                Span::styled(" cancel", note_style),
            ]));
        }
        InputMode::ConfirmDeletePreset => {
            lines.push(Line::from(vec![
                Span::styled("  [A-Z]", key_style),
                Span::styled(" delete preset  ", note_style),
                Span::styled("[Esc]", key_style),
                Span::styled(" cancel", note_style),
            ]));
        }
        InputMode::ConfirmDeleteRepoPath => {
            let path_label = app
                .repo_paths()
                .get(app.input.repo_cursor)
                .map(|p| p.as_str())
                .unwrap_or("?");
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  Delete {path_label}?  "),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("y", key_style),
                Span::styled(": yes  ", note_style),
                Span::styled("n/Esc", key_style),
                Span::styled(": cancel", note_style),
            ]));
        }
        _ => {
            lines.push(Line::from(vec![
                Span::styled("  [j/k]", key_style),
                Span::styled(" navigate  ", note_style),
                Span::styled("[Space]", key_style),
                Span::styled(" toggle  ", note_style),
                Span::styled("[a]", key_style),
                Span::styled(format!(" {a_label}  "), note_style),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  [Tab]", key_style),
                Span::styled(" incl/excl  ", note_style),
                Span::styled("[s]", key_style),
                Span::styled(" save preset  ", note_style),
                Span::styled("[x]", key_style),
                Span::styled(" del preset  ", note_style),
                Span::styled("[q/Esc]", key_style),
                Span::styled(" close", note_style),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

pub(super) fn render_task_detail_overlay(frame: &mut Frame, app: &mut App, area: Rect) {
    let (task_id, scroll, zoomed) = match &app.board.view_mode {
        ViewMode::TaskDetail {
            task_id,
            scroll,
            zoomed,
            ..
        } => (*task_id, *scroll, *zoomed),
        _ => return,
    };

    let Some(task) = app.board.tasks.iter().find(|t| t.id.0 == task_id).cloned() else {
        return;
    };

    // Compute overlay area
    let overlay_height = if zoomed {
        area.height.saturating_sub(1) // full height minus status bar
    } else {
        area.height / 2
    };
    let overlay_y = area.bottom().saturating_sub(overlay_height + 1); // above status bar
    let overlay_area = Rect {
        x: area.x,
        y: overlay_y,
        width: area.width,
        height: overlay_height,
    };

    frame.render_widget(Clear, overlay_area);

    // ── Header lines (metadata) ──────────────────────────────────────────────
    let label_style = Style::default().fg(MUTED);
    let value_style = Style::default().fg(FG);
    let mut header_lines: Vec<Line> = Vec::with_capacity(4);
    let mut field = |label: &'static str, value: String| {
        header_lines.push(Line::from(vec![
            Span::styled(label, label_style),
            Span::styled(value, value_style),
        ]));
    };

    field("Repo:  ", task.repo_path.clone());

    if let Some(epic_id) = task.epic_id {
        let epic_title = app.epic_title(epic_id).unwrap_or("").to_string();
        field("Epic:  ", format!("#{} — {}", epic_id, epic_title));
    }

    if let Some(pr_url) = &task.pr_url {
        let field_label = match crate::models::url_type(pr_url) {
            "PR" => "PR:    ",
            "Issue" => "Issue: ",
            _ => "Link:  ",
        };
        field(field_label, pr_url.clone());
    }

    if let Some(plan_path) = &task.plan_path {
        field("Plan:  ", plan_path.clone());
    }

    let header_height = header_lines.len() as u16 + 1; // +1 for separator line

    // ── Compute body area and scroll clamping ────────────────────────────────
    let body_height = overlay_area.height.saturating_sub(2 + header_height + 1); // borders(2) + header + separator(1)
    let body_width = overlay_area.width.saturating_sub(2) as usize;

    let desc_wrapped = wrapped_line_count(&task.description, body_width);
    let new_max_scroll = desc_wrapped.saturating_sub(body_height as usize) as u16;

    if let ViewMode::TaskDetail {
        ref mut max_scroll, ..
    } = app.board.view_mode
    {
        if *max_scroll != new_max_scroll {
            *max_scroll = new_max_scroll;
        }
    }

    // ── Block with hints ─────────────────────────────────────────────────────
    let hint_style = Style::default().fg(MUTED);
    let block = Block::default()
        .title(format!(" Task #{task_id} "))
        .title_bottom(Line::from(Span::styled(
            " j/k scroll · z zoom · q/Esc/Enter close ",
            hint_style,
        )))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(BORDER));

    let inner = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    // ── Render header inside block ────────────────────────────────────────────
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(header_height), Constraint::Min(0)])
        .split(inner);

    let header_block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(BORDER));
    frame.render_widget(Paragraph::new(header_lines).block(header_block), layout[0]);

    // ── Render scrollable description ─────────────────────────────────────────
    let desc_lines: Vec<Line> = task
        .description
        .lines()
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(MUTED_LIGHT),
            ))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(desc_lines)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        layout[1],
    );
}
