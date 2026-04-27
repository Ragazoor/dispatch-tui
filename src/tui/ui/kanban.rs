use super::palette::{
    ARCHIVE_COL_BG, ARCHIVE_STRIPE, BLUE, BORDER, CYAN, FG, FLASH_BG, GREEN, MUTED, MUTED_LIGHT,
    PROJECTS_COL_BG, PROJECTS_CURSOR_BG, PURPLE, YELLOW,
};
use super::shared::{
    push_hint_spans, render_substatus_header, render_tab_bar, staleness_color, truncate,
};

use crate::dispatch;
use crate::models::{
    format_age, Epic, EpicId, EpicSubstatus, Project, Staleness, SubStatus, Task, TaskStatus,
    TaskUsage,
};
use crate::tui::{
    is_edge_column, App, ColumnItem, ColumnLayout, EpicStatsMap, InputMode, RepoFilterMode,
    ViewMode,
};
use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Column color per status
pub(in crate::tui) fn column_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Backlog => BLUE,
        TaskStatus::Running => YELLOW,
        TaskStatus::Review => PURPLE,
        TaskStatus::Done => GREEN,
        TaskStatus::Archived => MUTED,
    }
}

/// Tinted background for the cursor card in each column.
pub(in crate::tui) fn cursor_bg_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Backlog => Color::Rgb(34, 38, 66),
        TaskStatus::Running => Color::Rgb(62, 50, 28),
        TaskStatus::Review => Color::Rgb(50, 34, 66),
        TaskStatus::Done => Color::Rgb(32, 52, 36),
        TaskStatus::Archived => Color::Rgb(34, 38, 66),
    }
}

/// Faint background wash for the focused column, tinted to the column color.
/// Must be just barely visible against the terminal bg (~26,27,38) so the
/// cursor card highlight (cursor_bg_color) stands out clearly on top of it.
pub(in crate::tui) fn column_bg_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Backlog => Color::Rgb(28, 30, 44),
        TaskStatus::Running => Color::Rgb(38, 34, 26),
        TaskStatus::Review => Color::Rgb(34, 28, 44),
        TaskStatus::Done => Color::Rgb(27, 36, 30),
        TaskStatus::Archived => Color::Rgb(28, 30, 44),
    }
}

/// Unicode status icon for the metadata line of each card.
fn status_icon(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "◦",
        TaskStatus::Running => "◉",
        TaskStatus::Review => "◎",
        TaskStatus::Done => "✓",
        TaskStatus::Archived => "◦",
    }
}

fn truncate_for_detail(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

/// Compute how tall the detail/input panel should be based on the current input mode.
/// Expands when a repo list is being shown so all repos (plus cursor) are visible.
fn input_panel_height(app: &App, area_height: u16) -> u16 {
    // Fixed overhead: tab_bar(1) + summary(1) + kanban_min(6) + status_bar(1) = 9
    let overhead: u16 = 9;
    let max_height = area_height.saturating_sub(overhead).max(8);
    match &app.input.mode {
        InputMode::QuickDispatch => {
            // header(1) + blank(1) + repos(N) + blank(1) + hint(1) + borders(2) = N + 6
            let rows = app.board.repo_paths.len() as u16 + 6;
            rows.clamp(8, max_height)
        }
        InputMode::InputRepoPath | InputMode::InputEpicRepoPath if app.input.buffer.is_empty() => {
            // title(1) + desc(1) + path_input(1) + repos(N) + blank(1) + hint(1) + borders(2) = N + 7
            let rows = app.board.repo_paths.len() as u16 + 7;
            rows.clamp(8, max_height)
        }
        _ => 8,
    }
}

/// Top-level render function.
pub fn render(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();
    let now = Utc::now();

    // When split mode is active, wrap everything in a focus border.
    let area = if app.split_active() {
        let border_color = if app.split_focused() { CYAN } else { BORDER };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(border_color));
        frame.render_widget(block, full_area);
        Rect {
            x: full_area.x + 1,
            y: full_area.y + 1,
            width: full_area.width.saturating_sub(2),
            height: full_area.height.saturating_sub(2),
        }
    } else {
        full_area
    };

    let panel_h = input_panel_height(app, area.height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1),       // tab bar
            Constraint::Length(1),       // summary row
            Constraint::Min(6),          // kanban board
            Constraint::Length(panel_h), // detail panel
            Constraint::Length(1),       // status bar
        ])
        .split(area);

    let epic_stats = app.compute_epic_stats();
    render_tab_bar(frame, app, vertical[0]);
    render_summary(frame, app, &epic_stats, vertical[1]);
    render_columns(frame, app, &epic_stats, vertical[2], now);
    render_detail(frame, app, vertical[3], now);
    render_status_bar(frame, app, vertical[4]);

    render_error_popup(frame, app, area);
    render_help_overlay(frame, app, area);
    render_repo_filter_overlay(frame, app, area);
    render_tips_overlay(frame, app, area);
}

/// Returns the layout constraints for the summary row based on which column is focused.
/// When an edge column (Projects=0 or Archive=5) is focused, 5 segments are shown.
/// When a task column (1–4) is focused, 4 segments are shown (task columns only).
fn column_layout_constraints(selected_col: usize) -> Vec<Constraint> {
    let n = if is_edge_column(selected_col) {
        5u32
    } else {
        4u32
    };
    vec![Constraint::Ratio(1, n); n as usize]
}

fn render_summary(frame: &mut Frame, app: &App, epic_stats: &EpicStatsMap, area: Rect) {
    let sel = app.selected_column();
    let constraints = column_layout_constraints(sel);
    let col_segments = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    let layout = ColumnLayout::build(app, epic_stats);

    // Build segments: (label, color, is_focused, checkbox_info)
    // checkbox_info is Some((all_selected, on_select_all, status)) for focused task columns.
    enum CheckboxInfo {
        Task {
            all_selected: bool,
            on_select_all: bool,
            status: TaskStatus,
        },
        None,
    }

    struct Segment {
        label: String,
        color: Color,
        is_focused: bool,
        checkbox: CheckboxInfo,
    }

    let mut segments: Vec<Segment> = Vec::new();

    // Edge column: Projects (only shown when col 0 is focused)
    if sel == 0 {
        let count = app.projects().len();
        segments.push(Segment {
            label: format!("\u{25b8} Projects {}", count),
            color: PURPLE,
            is_focused: true,
            checkbox: CheckboxInfo::None,
        });
    }

    // Task columns 1–4
    for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
        let nav_col = idx + 1;
        let items = layout.get(status);
        let count = items.len();
        let is_focused = sel == nav_col;
        let color = column_color(status);
        let prefix = if is_focused { "\u{25b8} " } else { "\u{25e6} " };
        let label = format!("{}{} {}", prefix, status.as_str(), count);

        let checkbox = if is_focused {
            let all_selected = !items.is_empty()
                && items.iter().all(|item| match item {
                    ColumnItem::Task(t) => app.selected_tasks().contains(&t.id),
                    ColumnItem::Epic(e) => app.selected_epics().contains(&e.id),
                });
            CheckboxInfo::Task {
                all_selected,
                on_select_all: app.on_select_all(),
                status,
            }
        } else {
            CheckboxInfo::None
        };

        segments.push(Segment {
            label,
            color,
            is_focused,
            checkbox,
        });
    }

    if sel == TaskStatus::COLUMN_COUNT + 1 {
        let count = app.archived_tasks().len();
        segments.push(Segment {
            label: format!("\u{25b8} Archive {}", count),
            color: ARCHIVE_STRIPE,
            is_focused: true,
            checkbox: CheckboxInfo::None,
        });
    }

    debug_assert_eq!(
        segments.len(),
        col_segments.len(),
        "summary segment count must match layout constraint count"
    );
    for (i, seg) in segments.iter().enumerate() {
        let label_style = if seg.is_focused {
            Style::default()
                .fg(seg.color)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::default().fg(MUTED)
        };

        let spans = match &seg.checkbox {
            CheckboxInfo::Task {
                all_selected,
                on_select_all,
                status,
            } => {
                let checkbox = if *all_selected { " [x]" } else { " [ ]" };
                let checkbox_style = if *on_select_all {
                    Style::default()
                        .bg(cursor_bg_color(*status))
                        .fg(FG)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(MUTED)
                };
                vec![
                    Span::styled(seg.label.clone(), label_style),
                    Span::styled(checkbox, checkbox_style),
                ]
            }
            CheckboxInfo::None => {
                vec![Span::styled(seg.label.clone(), label_style)]
            }
        };

        let paragraph = Paragraph::new(Line::from(spans)).alignment(Alignment::Center);
        frame.render_widget(paragraph, col_segments[i]);
    }
}

/// Format the title text for a task card (line 1 only — status annotations are on line 2).
fn format_task_title(task: &Task, max_title: usize) -> String {
    truncate(&task.title, max_title)
}

// ---------------------------------------------------------------------------
// CardIndicator — what to show on line 2 of a task card
// ---------------------------------------------------------------------------

/// Classifies a task's current state into a single display indicator.
/// Priority order matters: conflict > detached-review > crashed > stale >
/// blocked > detached-running > running > review-pr > done-merged > idle.
enum CardIndicator {
    Conflict,
    DetachedReview {
        pr_label: String,
    },
    Detached,
    Crashed,
    Stale {
        inactive_mins: u64,
    },
    Blocked,
    Running,
    ReviewPr {
        pr_label: String,
    },
    DoneMerged {
        pr_label: String,
    },
    Idle {
        status: TaskStatus,
        age: String,
        staleness: Staleness,
        plan_indicator: &'static str,
        tag_suffix: &'static str,
    },
}

fn classify_card_indicator(
    task: &Task,
    status: TaskStatus,
    app: &App,
    now: DateTime<Utc>,
) -> CardIndicator {
    if task.sub_status == SubStatus::Conflict {
        return CardIndicator::Conflict;
    }
    if task.is_detached() {
        if let (TaskStatus::Review, Some(pr_url)) = (status, task.pr_url.as_deref()) {
            let pr_label = crate::models::pr_number_from_url(pr_url)
                .map_or("PR".to_string(), |n| format!("PR #{n}"));
            return CardIndicator::DetachedReview { pr_label };
        }
        return CardIndicator::Detached;
    }
    if task.sub_status == SubStatus::Crashed {
        return CardIndicator::Crashed;
    }
    if task.sub_status == SubStatus::Stale {
        let inactive_mins = app
            .agents
            .inactive_duration(task.id)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        return CardIndicator::Stale { inactive_mins };
    }
    if status == TaskStatus::Running && task.sub_status == SubStatus::NeedsInput {
        return CardIndicator::Blocked;
    }
    if status == TaskStatus::Running {
        return CardIndicator::Running;
    }
    if let (TaskStatus::Review, Some(pr_url)) = (status, task.pr_url.as_deref()) {
        let pr_label = crate::models::pr_number_from_url(pr_url)
            .map_or("PR".to_string(), |n| format!("PR #{n}"));
        return CardIndicator::ReviewPr { pr_label };
    }
    if let (TaskStatus::Done, Some(pr_url)) = (status, task.pr_url.as_deref()) {
        let pr_label = crate::models::pr_number_from_url(pr_url)
            .map_or("PR".to_string(), |n| format!("PR #{n}"));
        return CardIndicator::DoneMerged { pr_label };
    }

    let age = format_age(task.updated_at, now);
    let staleness = Staleness::from_age(task.updated_at, now);
    let plan_indicator = if task.plan_path.is_some() && status == TaskStatus::Backlog {
        "▸ "
    } else {
        ""
    };
    let tag_suffix = match task.tag {
        Some(crate::models::TaskTag::Bug) => " [bug]",
        Some(crate::models::TaskTag::Feature) => " [feat]",
        Some(crate::models::TaskTag::Chore) => " [chore]",
        Some(crate::models::TaskTag::Epic) => " [epic]",
        None => "",
    };
    CardIndicator::Idle {
        status,
        age,
        staleness,
        plan_indicator,
        tag_suffix,
    }
}

fn render_card_indicator(indicator: CardIndicator) -> Line<'static> {
    let (label, color) = match indicator {
        CardIndicator::Conflict => ("\u{26a0} rebase conflict".to_string(), Color::Red),
        CardIndicator::DetachedReview { pr_label } => (format!("\u{25cb} {pr_label}"), Color::Cyan),
        CardIndicator::Detached => ("\u{25cb} detached".to_string(), MUTED),
        CardIndicator::Crashed => ("\u{26a0} crashed".to_string(), Color::Red),
        CardIndicator::Stale { inactive_mins } => (
            format!("\u{25c9} stale \u{00b7} {}m", inactive_mins),
            Color::Yellow,
        ),
        CardIndicator::Blocked => ("\u{25c9} blocked".to_string(), Color::Yellow),
        CardIndicator::Running => (
            format!("{} running", status_icon(TaskStatus::Running)),
            CYAN,
        ),
        CardIndicator::ReviewPr { pr_label } => (format!("\u{25cf} {pr_label}"), Color::Cyan),
        CardIndicator::DoneMerged { pr_label } => {
            (format!("\u{2714} {pr_label} merged"), Color::Green)
        }
        CardIndicator::Idle {
            status,
            age,
            staleness,
            plan_indicator,
            tag_suffix,
        } => {
            let icon = status_icon(status);
            (
                format!("{plan_indicator}{icon} {age}{tag_suffix}"),
                staleness_color(staleness),
            )
        }
    };
    Line::from(vec![
        Span::raw("   "),
        Span::styled(label, Style::default().fg(color)),
    ])
}

/// Build a styled two-line ListItem for a task card in a kanban column.
/// Line 1: stripe + title
/// Line 2: status icon + age/activity metadata
fn build_task_list_item<'a>(
    task: &Task,
    status: TaskStatus,
    app: &App,
    now: DateTime<Utc>,
    is_cursor: bool,
    col_color: Color,
    col_width: u16,
) -> ListItem<'a> {
    let is_batch_selected = app.selected_tasks().contains(&task.id);
    let select_prefix = if is_batch_selected { "* " } else { "  " };

    let has_message_flash = app
        .agents
        .message_flash
        .get(&task.id)
        .is_some_and(|t| t.elapsed().as_secs() < 3);

    // Prefix: select(2) + stripe(1) + " #NNN "(id_len+3) + optional flash(" ✉", 2)
    let id_len = format!("{}", task.id).len();
    let flash_width = if has_message_flash { 2 } else { 0 };
    let prefix_width = 2 + 1 + 3 + id_len + flash_width;
    let max_title = (col_width as usize).saturating_sub(prefix_width);
    let title_text = format_task_title(task, max_title);

    // Line 1: prefix + stripe + title
    // Cursor gets a thicker stripe (▌) as a left accent bar
    let stripe_char = if is_cursor { "\u{258c}" } else { "\u{258e}" };
    let stripe_style = Style::default().fg(col_color);
    let title_style = if is_batch_selected {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let mut line1_spans = vec![
        Span::styled(select_prefix.to_string(), title_style),
        Span::styled(stripe_char, stripe_style),
        Span::styled(format!(" #{} ", task.id), Style::default().fg(MUTED)),
        Span::styled(title_text.to_string(), title_style),
    ];
    if has_message_flash {
        line1_spans.push(Span::styled(
            " \u{2709}",
            Style::default().fg(Color::Yellow),
        ));
    }

    let line1 = Line::from(line1_spans);

    let mut line2 = render_card_indicator(classify_card_indicator(task, status, app, now));

    // When flattened, append the task's epic id to line 2 (purple) so epic
    // membership remains visible on the card.
    if app.board.flattened {
        if let Some(eid) = task.epic_id {
            line2.spans.push(Span::raw("  "));
            line2.spans.push(Span::styled(
                format!("#{}", eid.0),
                Style::default().fg(PURPLE),
            ));
        }
    }

    let mut item = ListItem::new(vec![line1, line2]);

    // Flash bg takes priority over cursor — it's transient (3s) and meant to grab attention
    if has_message_flash {
        item = item.style(
            Style::default()
                .bg(FLASH_BG)
                .fg(FG)
                .add_modifier(Modifier::BOLD),
        );
    } else if is_cursor {
        item = item.style(
            Style::default()
                .bg(cursor_bg_color(status))
                .fg(FG)
                .add_modifier(Modifier::BOLD),
        );
    }

    item
}

fn render_columns(
    frame: &mut Frame,
    app: &mut App,
    epic_stats: &EpicStatsMap,
    area: Rect,
    now: DateTime<Utc>,
) {
    // In Epic mode, wrap the whole board in a purple rounded border with a
    // subtle purple background hint.
    let board_area = if let ViewMode::Epic {
        epic_id, parent, ..
    } = app.view_mode()
    {
        let title = {
            // Walk the parent chain to collect all ancestor IDs (innermost first),
            // then reverse for display order (root → … → current).
            let mut ids: Vec<EpicId> = vec![*epic_id];
            let mut cursor: &ViewMode = parent.as_ref();
            while let ViewMode::Epic {
                epic_id: pid,
                parent: grandparent,
                ..
            } = cursor
            {
                ids.push(*pid);
                cursor = grandparent.as_ref();
            }
            ids.reverse();
            let segments: Vec<String> = ids
                .iter()
                .map(|id| {
                    app.epics()
                        .iter()
                        .find(|e| e.id == *id)
                        .map(|e| truncate(&e.title, 30))
                        .unwrap_or_default()
                })
                .collect();
            format!(" {} ", segments.join(" > "))
        };
        let block = Block::default()
            .title(title)
            .title_style(Style::default().fg(PURPLE).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(PURPLE))
            .style(Style::default().bg(Color::Rgb(24, 20, 34)));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };

    let sel = app.selected_column();
    let constraints = column_layout_constraints(sel);
    let column_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(board_area);

    let mut area_idx = 0usize;

    if sel == 0 {
        render_projects_column(frame, app, column_areas[area_idx]);
        area_idx += 1;
    }

    for (task_col_idx, &status) in TaskStatus::ALL.iter().enumerate() {
        let nav_col = task_col_idx + 1;
        render_task_column(
            frame,
            app,
            column_areas[area_idx],
            now,
            status,
            nav_col,
            epic_stats,
        );
        area_idx += 1;
    }

    if sel == TaskStatus::COLUMN_COUNT + 1 {
        render_archive_column(frame, app, column_areas[area_idx], now);
    }
}

/// Render a single task-status column (Backlog/Running/Review/Done).
///
/// `nav_col` is the navigation column index (1–4) used for focus detection and
/// derives the 0-based array index (`nav_col - 1`) for `selected_row`/`list_states`.
fn render_task_column(
    frame: &mut Frame,
    app: &mut App,
    col_area: Rect,
    now: DateTime<Utc>,
    status: TaskStatus,
    nav_col: usize,
    epic_stats: &EpicStatsMap,
) {
    let col_idx = nav_col - 1;
    let is_focused = app.selected_column() == nav_col;
    let color = column_color(status);
    // Only Running and Review benefit from substatus grouping headers.
    let show_headers = matches!(status, TaskStatus::Running | TaskStatus::Review);

    let column_items = app.column_items_for_status_with_stats(status, Some(epic_stats));
    let selected_row = app.selected_row()[col_idx];

    let mut list_items: Vec<ListItem> = Vec::new();
    let mut list_selection_idx: Option<usize> = None;
    let mut current_priority: Option<u8> = None;

    for (item_idx, item) in column_items.iter().enumerate() {
        if show_headers {
            let priority = match item {
                ColumnItem::Task(t) => t.sub_status.column_priority_detached(t.is_detached()),
                ColumnItem::Epic(e) => epic_stats
                    .get(&e.id)
                    .map(|s| s.substatus.column_priority())
                    .unwrap_or(0),
            };
            if Some(priority) != current_priority {
                current_priority = Some(priority);
                let label = match item {
                    ColumnItem::Task(t) => t
                        .sub_status
                        .header_label_detached(t.is_detached())
                        .to_string(),
                    ColumnItem::Epic(e) => epic_stats
                        .get(&e.id)
                        .map(|s| s.substatus.header_label())
                        .unwrap_or_default()
                        .to_string(),
                };
                list_items.push(render_substatus_header(&label, list_items.is_empty()));
            }
        }

        if item_idx == selected_row {
            list_selection_idx = Some(list_items.len());
        }

        let is_cursor = is_focused && !app.on_select_all() && item_idx == selected_row;
        list_items.push(match item {
            ColumnItem::Task(task) => {
                build_task_list_item(task, status, app, now, is_cursor, color, col_area.width)
            }
            ColumnItem::Epic(epic) => {
                render_epic_item(epic, is_cursor, app, epic_stats, status, col_area.width)
            }
        });
    }

    let on_select_all = app.on_select_all();
    let sel = app.selection_mut();
    if is_focused {
        *sel.list_states[col_idx].selected_mut() = if on_select_all {
            None
        } else {
            list_selection_idx
        };
    }

    if is_focused {
        let block = Block::default().style(Style::default().bg(column_bg_color(status)));
        let inner = block.inner(col_area);
        frame.render_widget(block, col_area);
        frame.render_stateful_widget(List::new(list_items), inner, &mut sel.list_states[col_idx]);
    } else {
        frame.render_stateful_widget(
            List::new(list_items),
            col_area,
            &mut sel.list_states[col_idx],
        );
    }
}

fn render_archive_column(frame: &mut Frame, app: &mut App, area: Rect, now: DateTime<Utc>) {
    let archived = app.archived_tasks();
    let sel_row = app.selected_archive_row();
    let color = ARCHIVE_STRIPE;

    let bg_block = Block::default().style(Style::default().bg(ARCHIVE_COL_BG));
    frame.render_widget(bg_block, area);

    let items: Vec<ListItem> = archived
        .iter()
        .enumerate()
        .map(|(idx, task)| {
            let is_cursor = idx == sel_row;
            build_task_list_item(
                task,
                TaskStatus::Archived,
                app,
                now,
                is_cursor,
                color,
                area.width,
            )
        })
        .collect();

    let title = format!(" Archive ({}) ", archived.len());
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD));
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut app.archive.list_state);
}

fn epic_substatus_color(substatus: &EpicSubstatus) -> Color {
    match substatus {
        EpicSubstatus::Blocked(_) => Color::Yellow,
        EpicSubstatus::InReview => CYAN,
        EpicSubstatus::WrappingUp => GREEN,
        EpicSubstatus::Active | EpicSubstatus::Unplanned | EpicSubstatus::Planned => MUTED,
        EpicSubstatus::Done => MUTED,
    }
}

fn render_epic_item(
    epic: &Epic,
    is_cursor: bool,
    app: &App,
    epic_stats: &EpicStatsMap,
    status: TaskStatus,
    col_width: u16,
) -> ListItem<'static> {
    let stats = epic_stats.get(&epic.id);

    let plan_indicator = if epic.plan_path.is_some() && status == TaskStatus::Backlog {
        " \u{25b8}" // ▸
    } else {
        ""
    };

    // Prefix: select(2) + stripe(1) + " #NNN "(id_len+3) + plan_indicator
    let id_len = format!("{}", epic.id).len();
    let prefix_width = 2 + 1 + 3 + id_len + plan_indicator.chars().count();
    let max_title = (col_width as usize).saturating_sub(prefix_width);
    let title_text = truncate(&epic.title, max_title);

    let is_batch_selected = app.selected_epics().contains(&epic.id);
    let select_prefix = if is_batch_selected { "* " } else { "  " };

    // Line 1: stripe + title (thicker stripe for cursor)
    let stripe_char = if is_cursor { "\u{258c}" } else { "\u{258e}" };
    let title_style = Style::default().fg(PURPLE).add_modifier(Modifier::BOLD);
    let line1 = Line::from(vec![
        Span::raw(select_prefix.to_string()),
        Span::styled(stripe_char, Style::default().fg(PURPLE)),
        Span::styled(format!(" #{} ", epic.id), Style::default().fg(MUTED)),
        Span::styled(format!("{title_text}{plan_indicator}"), title_style),
    ]);

    // Line 2: colored status indicators + substatus label
    let line2 = if let Some(s) = stats.filter(|s| s.total > 0) {
        let mut spans = vec![Span::raw("    ".to_string())];
        let indicators: &[(usize, Color)] = &[
            (s.backlog, column_color(TaskStatus::Backlog)),
            (s.running, column_color(TaskStatus::Running)),
            (s.review, column_color(TaskStatus::Review)),
            (s.done, column_color(TaskStatus::Done)),
        ];
        for (count, color) in indicators {
            if *count > 0 {
                spans.push(Span::styled(
                    format!("\u{25cf}{count} "),
                    Style::default().fg(*color),
                ));
            }
        }
        spans.push(Span::styled(
            s.substatus.label(),
            Style::default().fg(epic_substatus_color(&s.substatus)),
        ));
        Line::from(spans)
    } else {
        Line::from(vec![
            Span::raw("    "),
            Span::styled("no subtasks", Style::default().fg(MUTED)),
        ])
    };

    let mut item = ListItem::new(vec![line1, line2]);

    if is_cursor {
        item = item.style(
            Style::default()
                .bg(cursor_bg_color(status))
                .fg(FG)
                .add_modifier(Modifier::BOLD),
        );
    }

    item
}

fn build_project_list_item<'a>(
    project: &Project,
    task_count: usize,
    is_cursor: bool,
    is_active: bool,
    col_width: u16,
) -> ListItem<'a> {
    let stripe_color = if is_active {
        PURPLE
    } else {
        Color::Rgb(120, 100, 160)
    };
    let stripe = if is_cursor { "▌ " } else { "▎ " };

    let name = truncate(&project.name, col_width.saturating_sub(4) as usize);
    let active_marker = if is_active { " ◉" } else { "" };
    let name_line = Line::from(vec![
        Span::styled(stripe, Style::default().fg(stripe_color)),
        Span::styled(
            format!("{}{}", name, active_marker),
            if is_cursor {
                Style::default()
                    .bg(PROJECTS_CURSOR_BG)
                    .fg(FG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            },
        ),
    ]);

    let meta_line = Line::from(Span::styled(
        format!("   {} tasks", task_count),
        Style::default().fg(MUTED),
    ));

    ListItem::new(vec![name_line, meta_line])
}

fn render_projects_column(frame: &mut Frame, app: &mut App, area: Rect) {
    let sel_row = app.selected_project_row();
    let active_project = app.active_project();

    let bg_block = Block::default().style(Style::default().bg(PROJECTS_COL_BG));
    frame.render_widget(bg_block, area);

    let task_counts: std::collections::HashMap<i64, usize> =
        app.tasks()
            .iter()
            .filter(|t| t.status != TaskStatus::Archived)
            .fold(std::collections::HashMap::new(), |mut acc, t| {
                *acc.entry(t.project_id).or_insert(0) += 1;
                acc
            });

    let items: Vec<ListItem> = app
        .projects()
        .iter()
        .enumerate()
        .map(|(idx, project)| {
            let task_count = task_counts.get(&project.id).copied().unwrap_or(0);
            build_project_list_item(
                project,
                task_count,
                idx == sel_row,
                project.id == active_project,
                area.width,
            )
        })
        .collect();

    let title = format!(" Projects ({}) ", app.projects().len());
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(PURPLE).add_modifier(Modifier::BOLD));
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut app.projects_panel.list_state);
}

fn format_tokens(n: i64) -> String {
    if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn format_usage(u: &TaskUsage) -> String {
    format!(
        "${:.2} \u{00b7} {} in / {} out",
        u.cost_usd,
        format_tokens(u.input_tokens),
        format_tokens(u.output_tokens),
    )
}

// ── Detail-panel component functions ────────────────────────────────

pub(in crate::tui) fn task_detail_lines(app: &App, task: &Task) -> Vec<Line<'static>> {
    let status_color = column_color(task.status);

    // Line 1: title (bold, colored) + inline metadata (dim)
    let mut line1_spans = vec![
        Span::styled(
            task.title.clone(),
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if task.sub_status != SubStatus::None {
                format!(
                    " \u{00b7} #{} \u{00b7} {} ({}) \u{00b7} {}",
                    task.id,
                    task.status.as_str(),
                    task.sub_status.as_str(),
                    task.repo_path
                )
            } else {
                format!(
                    " \u{00b7} #{} \u{00b7} {} \u{00b7} {}",
                    task.id,
                    task.status.as_str(),
                    task.repo_path
                )
            },
            Style::default().fg(MUTED),
        ),
    ];

    // Add crash/stale suffix
    if app.is_crashed(task.id) {
        line1_spans.push(Span::styled(
            " (crashed)",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    } else if app.is_stale(task.id) {
        let mins = app
            .agents
            .inactive_duration(task.id)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        line1_spans.push(Span::styled(
            format!(" (stale \u{00b7} {}m)", mins),
            Style::default().fg(Color::Yellow),
        ));
    }

    if let Some(tag) = &task.tag {
        line1_spans.push(Span::styled(
            format!(" \u{00b7} [{tag}]"),
            Style::default().fg(Color::Cyan),
        ));
    }

    if let Some(pr_url) = &task.pr_url {
        line1_spans.push(Span::styled(
            format!(" \u{00b7} PR: {pr_url}"),
            Style::default().fg(Color::Cyan),
        ));
    }

    if let Some(epic_id) = task.epic_id {
        if let Some(epic_title) = app.epic_title(epic_id) {
            line1_spans.push(Span::styled(
                format!(" \u{00b7} Epic: {epic_title} (#{epic_id})"),
                Style::default().fg(Color::Magenta),
            ));
        }
    }

    let desc_style = Style::default().fg(MUTED_LIGHT);
    let mut lines = vec![Line::from(line1_spans)];
    for desc_line in task.description.lines() {
        lines.push(Line::from(Span::styled(desc_line.to_string(), desc_style)));
    }
    if task.description.is_empty() {
        lines.push(Line::from(Span::styled(String::new(), desc_style)));
    }
    if let Some(u) = app.board.usage.get(&task.id) {
        lines.push(Line::from(Span::styled(
            format_usage(u),
            Style::default().fg(MUTED),
        )));
    }
    if let Some(error) = app.last_error(task.id) {
        lines.push(Line::from(Span::styled(
            format!("Last error: {error}"),
            Style::default().fg(Color::Red),
        )));
    }
    lines
}

fn epic_detail_lines(app: &App, epic: &Epic) -> Vec<Line<'static>> {
    let epic_id = epic.id;
    let line1 = Line::from(vec![
        Span::styled(
            epic.title.clone(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" \u{00b7} #{} \u{00b7} {}", epic.id, epic.repo_path),
            Style::default().fg(MUTED),
        ),
    ]);
    let desc_style = Style::default().fg(MUTED_LIGHT);
    let mut lines = vec![line1];
    for desc_line in epic.description.lines() {
        lines.push(Line::from(Span::styled(desc_line.to_string(), desc_style)));
    }
    if epic.description.is_empty() {
        lines.push(Line::from(Span::styled(String::new(), desc_style)));
    }
    if let Some(plan) = &epic.plan_path {
        lines.push(Line::from(Span::styled(
            format!("plan: {plan}"),
            Style::default().fg(MUTED),
        )));
    }

    // Subtask status list
    let mut subtasks: Vec<&Task> = app
        .tasks()
        .iter()
        .filter(|t| t.epic_id == Some(epic_id) && t.status != TaskStatus::Archived)
        .collect();
    subtasks.sort_by_key(|t| (t.status.column_index(), t.sort_order.unwrap_or(t.id.0)));

    if !subtasks.is_empty() {
        lines.push(Line::from(""));
        for task in &subtasks {
            let icon = status_icon(task.status);
            let icon_color = column_color(task.status);
            let mut spans = vec![
                Span::styled(format!("  {icon} "), Style::default().fg(icon_color)),
                Span::styled(
                    truncate_for_detail(&task.title, 40),
                    Style::default().fg(Color::Rgb(180, 184, 200)),
                ),
            ];
            if let Some(wt) = &task.worktree {
                if let Some(branch) = dispatch::branch_from_worktree(wt) {
                    spans.push(Span::styled(
                        format!(" ({branch})"),
                        Style::default().fg(Color::Rgb(86, 95, 137)),
                    ));
                }
            }
            if task.sub_status == SubStatus::Conflict {
                spans.push(Span::styled(
                    " \u{26a0} conflict",
                    Style::default().fg(Color::Red),
                ));
            }
            if let Some(pr_url) = &task.pr_url {
                spans.push(Span::styled(
                    format!(" \u{00b7} PR: {}", truncate_for_detail(pr_url, 30)),
                    Style::default().fg(Color::Cyan),
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    lines
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect, _now: DateTime<Utc>) {
    // When in input mode, show the input form instead of detail
    if render_input_form(frame, app, area) {
        return;
    }

    // Top border separator
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER));

    if !app.board.detail_visible {
        let paragraph = Paragraph::new("").block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    let lines: Vec<Line> = if let Some(task) = app.selected_task() {
        task_detail_lines(app, task)
    } else if let Some(ColumnItem::Epic(epic)) = app.selected_column_item() {
        epic_detail_lines(app, epic)
    } else {
        vec![Line::from(Span::styled(
            "No task selected",
            Style::default().fg(MUTED),
        ))]
    };

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

use super::input_form::{
    confirm_retry_lines, input_base_branch_lines, input_description_lines,
    input_epic_description_lines, input_epic_repo_path_lines, input_epic_title_lines,
    input_repo_path_lines, input_tag_lines, input_title_lines, quick_dispatch_lines,
};

fn render_input_form(frame: &mut Frame, app: &App, area: Rect) -> bool {
    let completed = Style::default().fg(Color::White);
    let active = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let hint = Style::default().fg(Color::DarkGray);

    let lines: Vec<Line> = match &app.input.mode {
        InputMode::InputTitle => input_title_lines(app, active, hint),
        InputMode::InputTag => input_tag_lines(app, completed, active, hint),
        InputMode::InputDescription => input_description_lines(app, completed, active, hint),
        InputMode::InputRepoPath => input_repo_path_lines(app, area, completed, active, hint),
        InputMode::InputBaseBranch => input_base_branch_lines(app, completed, active, hint),
        InputMode::QuickDispatch => quick_dispatch_lines(app, area, active, hint),
        InputMode::ConfirmRetry(id) => confirm_retry_lines(app, *id),
        InputMode::InputEpicTitle => input_epic_title_lines(app, active, hint),
        InputMode::InputEpicDescription => {
            input_epic_description_lines(app, completed, active, hint)
        }
        InputMode::InputEpicRepoPath => {
            input_epic_repo_path_lines(app, area, completed, active, hint)
        }
        _ => return false,
    };

    let is_epic_input = matches!(
        app.input.mode,
        InputMode::InputEpicTitle | InputMode::InputEpicDescription | InputMode::InputEpicRepoPath
    );

    let block_title = match &app.input.mode {
        InputMode::QuickDispatch => " Quick Dispatch ",
        InputMode::ConfirmRetry(_) => " Retry Agent ",
        _ if is_epic_input => " New Epic ",
        _ => " New Task ",
    };

    let border_color = match &app.input.mode {
        InputMode::ConfirmRetry(_) => Color::Red,
        _ if is_epic_input => Color::Magenta,
        _ => Color::Yellow,
    };

    let block = Block::default()
        .title(block_title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
    true
}

fn render_error_popup(frame: &mut Frame, app: &App, area: Rect) {
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

fn render_tips_overlay(frame: &mut Frame, app: &App, area: Rect) {
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

fn render_help_overlay(frame: &mut Frame, app: &App, area: Rect) {
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
            Span::styled(" detail panel     ", desc),
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
            Span::styled("[m]", key),
            Span::styled(" move fwd   ", desc),
            Span::styled("[M]", key),
            Span::styled(" move back", desc),
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
            Span::styled("[V]", key),
            Span::styled(" epic done  ", desc),
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
            Span::styled("(Review tasks, supports batch)", note),
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

fn render_repo_filter_overlay(frame: &mut Frame, app: &App, area: Rect) {
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
                Span::styled("[Tab]", key_style),
                Span::styled(" incl/excl", note_style),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  [s]", key_style),
                Span::styled(" save preset  ", note_style),
                Span::styled("[x]", key_style),
                Span::styled(" del preset  ", note_style),
                Span::styled("[Bksp]", key_style),
                Span::styled(" del repo  ", note_style),
                Span::styled("[q/Esc]", key_style),
                Span::styled(" close", note_style),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(msg) = &app.status.message {
        let bar = Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Yellow));
        frame.render_widget(bar, area);
        return;
    }

    // Archive mode status bar
    if app.show_archived() {
        let key_color = MUTED;
        let label_style = Style::default().fg(MUTED);
        let spans = vec![
            Span::styled(
                "[x]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" delete  ", label_style),
            Span::styled(
                "[e]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" edit  ", label_style),
            Span::styled(
                "[H]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" close  ", label_style),
            Span::styled(
                "[q]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" quit  ", label_style),
        ];
        let bar = Paragraph::new(Line::from(spans));
        frame.render_widget(bar, area);
        return;
    }

    match &app.input.mode {
        InputMode::Normal => {
            let key_color = CYAN;
            let mut spans = if app.has_selection() {
                let count = app.selected_tasks().len() + app.selected_epics().len();
                let has_tasks = !app.selected_tasks().is_empty();
                batch_action_hints(count, key_color, has_tasks)
            } else if let Some(ColumnItem::Epic(epic)) = app.selected_column_item() {
                epic_action_hints(epic, key_color)
            } else {
                action_hints(app.selected_task(), app.selected_column(), key_color)
            };
            if app.split_active() {
                let mut prefix = vec![
                    Span::styled(
                        "[S]",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("plit ", Style::default().fg(Color::Green)),
                ];
                prefix.append(&mut spans);
                spans = prefix;
            }
            if app.board.flattened {
                let mut prefix = vec![Span::styled(
                    "[flat] ",
                    Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
                )];
                prefix.append(&mut spans);
                spans = prefix;
            }
            let bar = Paragraph::new(Line::from(spans));
            frame.render_widget(bar, area);
        }
        InputMode::InputTitle => {
            let bar = Paragraph::new("Creating task: enter title")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputDescription => {
            let bar = Paragraph::new("Creating task: opening $EDITOR for description")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputRepoPath => {
            let bar = Paragraph::new("Creating task: enter repo path")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputTag => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Tag: [b]ug  [f]eature  [c]hore  [e]pic  [Enter] none");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDelete => {
            let text = app.status.message.as_deref().unwrap_or("Delete? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::QuickDispatch => {
            let bar = Paragraph::new("Quick dispatch: select repo path")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmRetry(_) => {
            let bar = Paragraph::new("[r] Resume  [f] Fresh start  [Esc] Cancel")
                .style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmArchive(_) => {
            let bar =
                Paragraph::new("Archive task? [y/n]").style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDone(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Move to Done? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputEpicTitle => {
            let bar = Paragraph::new("Creating epic: enter title")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::InputEpicDescription => {
            let bar = Paragraph::new("Creating epic: opening $EDITOR for description")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::InputEpicRepoPath => {
            let bar = Paragraph::new("Creating epic: enter repo path")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeleteEpic => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Delete epic and subtasks? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmArchiveEpic => {
            let bar = Paragraph::new("Archive epic and subtasks? [y/n]")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::Help => {
            let bar = Paragraph::new("[?] or [Esc] to close help")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::RepoFilter => {
            let bar = Paragraph::new("Filter repos: [1-9] toggle  [a] all  [q/Esc] close")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmMergePr(_) => {
            let text = app.status.message.as_deref().unwrap_or("Merge PR? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Green));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmWrapUp(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Wrap up: [r] rebase  [p] create PR  [Esc] cancel");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputPresetName => {
            let bar = Paragraph::new("Enter preset name, [Enter] save, [Esc] cancel")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeletePreset => {
            let bar = Paragraph::new("[A-Z] delete preset  [Esc] cancel")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeleteRepoPath => {
            let bar = Paragraph::new("Delete repo path? y to confirm, any key to cancel")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmEpicWrapUp(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Epic wrap up: [r] rebase all  [p] PR all  [Esc] cancel");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDetachTmux(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Detach tmux panel? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmEditTask(_) => {
            let text = app.status.message.as_deref().unwrap_or("Edit task? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmQuit => {
            let bar =
                Paragraph::new("Quit dispatch? [y/n]").style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputBaseBranch => {
            let text = app.status.message.as_deref().unwrap_or("Base branch: ");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputProjectName { .. }
        | InputMode::ConfirmDeleteProject1 { .. }
        | InputMode::ConfirmDeleteProject2 { .. } => {}
    }
}

/// Build context-sensitive keybinding hint spans for the status bar.
/// Returns styled spans showing available actions for the selected task.
pub(in crate::tui) fn action_hints(
    task: Option<&Task>,
    selected_column: usize,
    key_color: Color,
) -> Vec<Span<'static>> {
    let label_style = Style::default().fg(MUTED);

    let mut spans: Vec<Span<'static>> = Vec::new();

    let mut push_hint = |key: &'static str, label: &'static str| {
        push_hint_spans(&mut spans, key, label, key_color, label_style);
    };

    if let Some(task) = task {
        match task.status {
            TaskStatus::Backlog => {
                let d_label = if task.plan_path.is_some() {
                    "dispatch"
                } else {
                    "brainstorm"
                };
                push_hint("d", d_label);
                push_hint("e", "edit");
                push_hint("L", "move");
                push_hint("x", "archive");
                push_hint("h", "projects");
            }
            TaskStatus::Running => {
                if task.tmux_window.is_some() {
                    push_hint("g", "session");
                } else if task.worktree.is_some() {
                    push_hint("d", "resume");
                }
                push_hint("e", "edit");
                push_hint("L", "move");
                push_hint("H", "back");
                push_hint("x", "archive");
            }
            TaskStatus::Review => {
                if task.worktree.is_some() {
                    push_hint("W", "wrap up");
                }
                if task.tmux_window.is_some() {
                    push_hint("g", "session");
                    push_hint("T", "detach");
                } else if task.worktree.is_some() {
                    push_hint("d", "resume");
                }
                push_hint("e", "edit");
                push_hint("L", "move");
                push_hint("H", "back");
                push_hint("x", "archive");
            }
            TaskStatus::Done => {
                push_hint("e", "edit");
                push_hint("H", "back");
                push_hint("x", "archive");
            }
            TaskStatus::Archived => {}
        }
        if task.pr_url.is_some() {
            push_hint("p", "open PR");
            if task.sub_status == SubStatus::Approved {
                push_hint("P", "merge");
            }
        }
    }

    if task.is_some() {
        push_hint("Enter", "detail");
        push_hint("c", "copy");
    }
    if task.is_none() && selected_column == 0 {
        push_hint("h", "projects");
    }
    push_hint("a", "select all");
    push_hint("n", "new");
    push_hint("E", "epic");
    push_hint("D", "quick");
    push_hint("S", "split");
    push_hint("F", "flat");
    push_hint("f", "filter");
    push_hint("?", "help");
    push_hint("q", "quit");

    spans
}

/// Build context-sensitive keybinding hints for a selected epic.
pub(in crate::tui) fn epic_action_hints(epic: &Epic, key_color: Color) -> Vec<Span<'static>> {
    let label_style = Style::default().fg(MUTED);

    let mut spans: Vec<Span<'static>> = Vec::new();

    let mut push_hint = |key: &'static str, label: &'static str| {
        push_hint_spans(&mut spans, key, label, key_color, label_style);
    };

    if epic.plan_path.is_some() {
        push_hint("d", "dispatch");
    } else {
        push_hint("d", "plan");
    }
    push_hint("g", "board");
    push_hint("G", "session");
    push_hint("Enter", "detail");
    push_hint("e", "edit");
    push_hint("W", "wrap up");
    push_hint("U", "auto dispatch");
    push_hint("L", "status \u{2192}");
    push_hint("H", "status \u{2190}");
    push_hint("x", "archive");

    push_hint("a", "select all");
    push_hint("n", "new");
    push_hint("E", "epic");
    push_hint("D", "quick");
    push_hint("F", "flat");
    push_hint("f", "filter");
    push_hint("?", "help");
    push_hint("q", "quit");

    spans
}

/// Build status bar hints when tasks are batch-selected.
fn batch_action_hints(count: usize, key_color: Color, has_tasks: bool) -> Vec<Span<'static>> {
    let label_style = Style::default().fg(MUTED);
    let count_style = Style::default().fg(YELLOW).add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(format!("{count} selected  "), count_style));

    let mut push_hint = |key: &'static str, label: &'static str| {
        push_hint_spans(&mut spans, key, label, key_color, label_style);
    };

    if has_tasks {
        push_hint("L", "move");
        push_hint("H", "back");
    }
    push_hint("x", "archive");
    push_hint("a", "select all");
    push_hint("F", "flat");
    push_hint("Space", "toggle");
    push_hint("Esc", "clear");
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskTag;
    use crate::tui::types::TaskDraft;
    use ratatui::buffer::Buffer;
    use std::time::Duration;

    fn make_test_app() -> App {
        App::new(vec![], 1, Duration::from_secs(300))
    }

    fn dummy_style() -> Style {
        Style::default()
    }

    #[test]
    fn input_description_shows_tag_when_set() {
        let mut app = make_test_app();
        app.input.task_draft = Some(TaskDraft {
            title: "My task".into(),
            tag: Some(TaskTag::Bug),
            ..Default::default()
        });
        app.input.buffer = "some desc".into();
        let lines = input_description_lines(&app, dummy_style(), dummy_style(), dummy_style());
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
        let lines = input_description_lines(&app, dummy_style(), dummy_style(), dummy_style());
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
        app.input.buffer = "/some/path".into();
        let area = Rect::new(0, 0, 80, 24);
        let lines = input_repo_path_lines(&app, area, dummy_style(), dummy_style(), dummy_style());
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
        assert!(text.contains("Repo path: /some/path_"));
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
        app.input.buffer.clear();
        let area = Rect::new(0, 0, 80, 24);
        let lines = input_repo_path_lines(&app, area, dummy_style(), dummy_style(), dummy_style());
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

    #[test]
    fn substatus_header_has_two_lines() {
        let item = render_substatus_header("my-repo", false);
        let buf = render_list_item_to_buf(item, 40, 2);
        // Confirm both rows are allocated (height 2 means 2 rows rendered)
        assert_eq!(buf.area().height, 2);
    }

    #[test]
    fn substatus_header_first_line_is_blank() {
        let item = render_substatus_header("my-repo", false);
        let buf = render_list_item_to_buf(item, 40, 2);
        let row0 = buf_row(&buf, 0);
        assert!(
            row0.trim().is_empty(),
            "first line should be blank spacer, got: {row0:?}"
        );
    }

    #[test]
    fn substatus_header_second_line_contains_label() {
        let item = render_substatus_header("my-repo", false);
        let buf = render_list_item_to_buf(item, 40, 2);
        let row1 = buf_row(&buf, 1);
        assert!(
            row1.contains("my-repo"),
            "second line should contain label, got: {row1:?}"
        );
    }

    #[test]
    fn substatus_header_second_line_is_bold_and_bright() {
        let item = render_substatus_header("my-repo", false);
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
    fn format_tokens_below_1000() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn format_tokens_at_and_above_1000() {
        assert_eq!(format_tokens(1000), "1k");
        assert_eq!(format_tokens(1999), "1k");
        assert_eq!(format_tokens(12_345), "12k");
    }

    #[test]
    fn format_usage_compact() {
        use crate::models::TaskId;
        use chrono::Utc;
        let u = TaskUsage {
            task_id: TaskId(1),
            cost_usd: 0.45,
            input_tokens: 12_345,
            output_tokens: 2_000,
            cache_read_tokens: 500,
            cache_write_tokens: 100,
            updated_at: Utc::now(),
        };
        assert_eq!(format_usage(&u), "$0.45 \u{00b7} 12k in / 2k out");
    }

    #[test]
    fn first_substatus_header_has_no_blank_line() {
        let item = render_substatus_header("awaiting review", true);
        assert_eq!(
            item.height(),
            1,
            "first header should have 1 line (no blank)"
        );
    }

    #[test]
    fn subsequent_substatus_header_has_blank_line() {
        let item = render_substatus_header("in review", false);
        assert_eq!(
            item.height(),
            2,
            "subsequent header should have 2 lines (blank + label)"
        );
    }
}
