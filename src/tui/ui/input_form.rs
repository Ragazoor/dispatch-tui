use crate::models::TaskId;
use crate::tui::App;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Build the active input row for a single-line text field, drawing the caret
/// as a reversed block at `app.input.caret` and scrolling long values so the
/// caret stays visible. `prefix` includes the label and separator, e.g.
/// `"  Title: "`.
fn caret_field(prefix: &str, app: &App, area: Rect, active: Style) -> Line<'static> {
    super::caret_field_line(
        area.width,
        prefix,
        "",
        &app.input.buffer,
        app.input.caret,
        active,
    )
}

/// Appends the filtered repo list and optional new-path entry to `lines`.
///
/// Shows existing paths that fuzzy-match `buffer`, then appends a selectable
/// new-path entry when `buffer` is non-empty and not an exact match for any
/// filtered item. This is the shared rendering contract for all
/// `RepoPathPicker` surfaces (InputRepoPath, MainSessionDir, QuickDispatch).
fn append_filtered_repos_with_new_entry<'a>(
    lines: &mut Vec<Line<'a>>,
    filtered: &[String],
    buffer: &'a str,
    cursor: usize,
    height_offset: u16,
    area_height: u16,
    hint: Style,
) {
    let show_new = crate::tui::has_new_repo_option(buffer, filtered);
    let scroll_cursor = if show_new && !filtered.is_empty() && cursor == filtered.len() {
        filtered.len() - 1
    } else {
        cursor
    };
    if !filtered.is_empty() {
        append_repo_path_list(
            lines,
            filtered,
            scroll_cursor,
            height_offset,
            area_height,
            hint,
        );
    }
    if show_new {
        let cursor_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        if cursor == filtered.len() {
            lines.push(Line::from(vec![
                Span::styled("  ► ", cursor_style),
                Span::styled(buffer, cursor_style),
                Span::styled("  (new)", hint),
            ]));
        } else {
            lines.push(Line::from(Span::styled(format!("    + {buffer}"), hint)));
        }
    }
}

/// Appends a scrollable repo-path picker list to `lines`.
pub(in crate::tui) fn append_repo_path_list<'a>(
    lines: &mut Vec<Line<'a>>,
    repo_paths: &[String],
    cursor: usize,
    height_offset: u16,
    area_height: u16,
    hint: Style,
) {
    let repo_count = repo_paths.len();
    let visible_repos = (area_height.saturating_sub(height_offset) as usize).max(1);
    let scroll = if repo_count <= visible_repos {
        0
    } else {
        cursor
            .saturating_sub(visible_repos - 1)
            .min(repo_count - visible_repos)
    };
    let cursor_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    for (i, path) in repo_paths
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_repos)
    {
        if i == cursor {
            lines.push(Line::from(vec![
                Span::styled("  ► ".to_string(), cursor_style),
                Span::styled(path.to_string(), cursor_style),
            ]));
        } else {
            lines.push(Line::from(Span::styled(format!("    {path}"), hint)));
        }
    }
}

pub(in crate::tui) fn input_title_lines(
    app: &App,
    area: Rect,
    active: Style,
    hint: Style,
) -> Vec<Line<'static>> {
    vec![
        caret_field("  Title: ", app, area, active),
        Line::from(""),
        Line::from(Span::styled("  [Enter] confirm  [Esc] cancel", hint)),
    ]
}

pub(in crate::tui) fn input_tag_lines(
    app: &App,
    completed: Style,
    active: Style,
    hint: Style,
) -> Vec<Line<'static>> {
    let title = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.title.as_str())
        .unwrap_or("");
    vec![
        Line::from(Span::styled(format!("  Title: {title}"), completed)),
        Line::from(Span::styled(
            "  Tag: [b]ug  [f]eature  [c]hore  [e]pic  [p]r-review  [r]esearch  [x]fix  [Enter] none",
            active,
        )),
        Line::from(""),
        Line::from(Span::styled("  [Enter] skip  [Esc] cancel", hint)),
    ]
}

pub(in crate::tui) fn input_description_lines(
    app: &App,
    completed: Style,
    active: Style,
    hint: Style,
) -> Vec<Line<'static>> {
    let title = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.title.as_str())
        .unwrap_or("");
    let tag = app
        .input
        .task_draft
        .as_ref()
        .and_then(|d| d.tag.as_ref())
        .map(|t| t.to_string())
        .unwrap_or_else(|| "none".to_string());
    vec![
        Line::from(Span::styled(format!("  Title: {title}"), completed)),
        Line::from(Span::styled(format!("  Tag: {tag}"), completed)),
        Line::from(Span::styled(
            "  Description: opening $EDITOR...".to_string(),
            active,
        )),
        Line::from(""),
        Line::from(Span::styled("  [Esc] cancel", hint)),
    ]
}

pub(in crate::tui) fn input_repo_path_lines<'a>(
    app: &'a App,
    area: Rect,
    completed: Style,
    active: Style,
    hint: Style,
) -> Vec<Line<'a>> {
    let title = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.title.as_str())
        .unwrap_or("");
    let tag = app
        .input
        .task_draft
        .as_ref()
        .and_then(|d| d.tag.as_ref())
        .map(|t| t.to_string())
        .unwrap_or_else(|| "none".to_string());
    let description = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.description.as_str())
        .unwrap_or("");
    let desc_first_line = description.lines().next().unwrap_or("");
    let desc_display = if description.contains('\n') {
        format!("{desc_first_line} ...")
    } else {
        desc_first_line.to_string()
    };
    let mut lines = vec![
        Line::from(Span::styled(format!("  Title: {title}"), completed)),
        Line::from(Span::styled(format!("  Tag: {tag}"), completed)),
        Line::from(Span::styled(
            format!("  Description: {desc_display}"),
            completed,
        )),
        caret_field("  Repo path: ", app, area, active),
    ];
    let filtered = crate::tui::filtered_repos(&app.board.repo_paths, &app.input.buffer);
    append_filtered_repos_with_new_entry(
        &mut lines,
        &filtered,
        &app.input.buffer,
        app.input.repo_cursor,
        7,
        area.height,
        hint,
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Type to filter · [↑/↓] navigate · [Enter] select · [Esc] cancel",
        hint,
    )));
    lines
}

pub(in crate::tui) fn input_base_branch_lines<'a>(
    app: &'a App,
    area: Rect,
    completed: Style,
    active: Style,
    hint: Style,
) -> Vec<Line<'a>> {
    let title = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.title.clone())
        .unwrap_or_default();
    let tag = app
        .input
        .task_draft
        .as_ref()
        .and_then(|d| d.tag.as_ref())
        .map(|t| t.to_string())
        .unwrap_or_else(|| "none".to_string());
    let description = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.description.clone())
        .unwrap_or_default();
    let desc_first_line = description.lines().next().unwrap_or("").to_string();
    let desc_display = if description.contains('\n') {
        format!("{desc_first_line} ...")
    } else {
        desc_first_line
    };
    let repo_path = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.repo_path.clone())
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(Span::styled(format!("  Title: {title}"), completed)),
        Line::from(Span::styled(format!("  Tag: {tag}"), completed)),
        Line::from(Span::styled(
            format!("  Description: {desc_display}"),
            completed,
        )),
        Line::from(Span::styled(format!("  Repo path: {repo_path}"), completed)),
        caret_field("  Base branch: ", app, area, active),
    ];
    let history = app.base_branches_for(&repo_path);
    let filtered = crate::tui::filtered_repos(history, &app.input.buffer);
    append_filtered_repos_with_new_entry(
        &mut lines,
        &filtered,
        &app.input.buffer,
        app.input.repo_cursor,
        6,
        area.height,
        hint,
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Type to filter · [↑/↓] navigate · [Enter] select · [Esc] cancel",
        hint,
    )));
    lines
}

pub(in crate::tui) fn input_wrap_up_mode_lines(
    app: &App,
    completed: Style,
    active: Style,
    hint: Style,
) -> Vec<Line<'static>> {
    let draft = app.input.task_draft.as_ref();
    let title = draft.map(|d| d.title.clone()).unwrap_or_default();
    let tag = draft
        .and_then(|d| d.tag.as_ref())
        .map(|t| t.to_string())
        .unwrap_or_else(|| "none".to_string());
    let repo_path = draft.map(|d| d.repo_path.clone()).unwrap_or_default();
    let base_branch = draft
        .map(|d| d.base_branch.clone())
        .unwrap_or_else(|| "main".to_string());
    vec![
        Line::from(Span::styled(format!("  Title: {title}"), completed)),
        Line::from(Span::styled(format!("  Tag: {tag}"), completed)),
        Line::from(Span::styled(format!("  Repo: {repo_path}"), completed)),
        Line::from(Span::styled(
            format!("  Base branch: {base_branch}"),
            completed,
        )),
        Line::from(Span::styled(
            "  Wrap-up: [r]ebase  [p]r  [d]one  [Enter] skip",
            active,
        )),
        Line::from(""),
        Line::from(Span::styled("  [Esc] cancel", hint)),
    ]
}

fn repo_picker_lines<'a>(
    app: &'a App,
    area: Rect,
    header: &'a str,
    prefix: &'a str,
    hint_text: &'a str,
    active: Style,
    hint: Style,
) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::from(Span::styled(header, active)),
        Line::from(""),
        caret_field(&format!("  {prefix}: "), app, area, active),
    ];
    let filtered = crate::tui::filtered_repos(&app.board.repo_paths, &app.input.buffer);
    append_filtered_repos_with_new_entry(
        &mut lines,
        &filtered,
        &app.input.buffer,
        app.input.repo_cursor,
        7,
        area.height,
        hint,
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(hint_text, hint)));
    lines
}

pub(in crate::tui) fn main_session_dir_lines<'a>(
    app: &'a App,
    area: Rect,
    active: Style,
    hint: Style,
) -> Vec<Line<'a>> {
    repo_picker_lines(
        app,
        area,
        "  Main session — base repo:",
        "Path",
        "  Type to filter · [↑/↓] navigate · [Enter] select · [Esc] cancel",
        active,
        hint,
    )
}

pub(in crate::tui) fn quick_dispatch_lines<'a>(
    app: &'a App,
    area: Rect,
    active: Style,
    hint: Style,
) -> Vec<Line<'a>> {
    let filtered = crate::tui::filtered_repos(&app.board.repo_paths, &app.input.buffer);
    let mut lines = vec![
        Line::from(Span::styled("  Quick Dispatch — select repo:", active)),
        Line::from(""),
        caret_field("  Filter: ", app, area, active),
    ];
    append_filtered_repos_with_new_entry(
        &mut lines,
        &filtered,
        &app.input.buffer,
        app.input.repo_cursor,
        7,
        area.height,
        hint,
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Type to filter · [↑/↓] navigate · [Enter] select · [Esc] cancel",
        hint,
    )));
    lines
}

pub(in crate::tui) fn confirm_retry_lines(app: &App, id: TaskId) -> Vec<Line<'static>> {
    let label = if app.is_crashed(id) {
        "crashed"
    } else {
        "stale"
    };
    let warning = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let hint = Style::default().fg(Color::DarkGray);
    vec![
        Line::from(Span::styled(
            format!("  Agent is {label}. What do you want to do?"),
            warning,
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [r] Resume (--continue in existing worktree)",
            hint,
        )),
        Line::from(Span::styled(
            "  [f] Fresh start (clean worktree + new dispatch)",
            hint,
        )),
        Line::from(Span::styled("  [Esc] Cancel", hint)),
    ]
}

pub(in crate::tui) fn input_epic_title_lines(
    app: &App,
    area: Rect,
    active: Style,
    hint: Style,
) -> Vec<Line<'static>> {
    vec![
        caret_field("  Title: ", app, area, active),
        Line::from(""),
        Line::from(Span::styled("  [Enter] confirm  [Esc] cancel", hint)),
    ]
}

pub(in crate::tui) fn input_epic_description_lines(
    app: &App,
    completed: Style,
    active: Style,
    hint: Style,
) -> Vec<Line<'static>> {
    let title = app
        .input
        .epic_draft
        .as_ref()
        .map(|d| d.title.as_str())
        .unwrap_or("");
    vec![
        Line::from(Span::styled(format!("  Title: {title}"), completed)),
        Line::from(Span::styled(
            "  Description: opening $EDITOR...".to_string(),
            active,
        )),
        Line::from(""),
        Line::from(Span::styled("  [Esc] cancel", hint)),
    ]
}
