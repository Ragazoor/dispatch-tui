//! Task and epic card rendering.

use chrono::{DateTime, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};

use crate::models::{format_age, Epic, EpicSubstatus, Staleness, SubStatus, Task, TaskStatus};
use crate::tui::{App, EpicStatsMap};

use super::super::palette::{CYAN, FG, FLASH_BG, GREEN, MUTED, PURPLE};
use super::super::shared::{staleness_color, truncate};
use super::{column_color, cursor_bg_color, status_icon};

/// Format the title text for a task card (line 1 only — status annotations are on line 2).
fn format_task_title(task: &Task, max_title: usize) -> String {
    truncate(&task.title, max_title)
}

// ---------------------------------------------------------------------------
// CardIndicator — what to show on line 2 of a task card
// ---------------------------------------------------------------------------

/// Classifies a task's current state into a single display indicator.
/// Priority order matters: dispatching > conflict > detached-review >
/// crashed > stale > blocked > detached-running > running > review-pr >
/// done-merged > idle. The `Dispatching` variant is reachable only for
/// pre-dispatch (Backlog) tasks and is removed automatically when the
/// dispatch worker reports success or failure.
enum CardIndicator {
    Dispatching {
        spinner_frame: u8,
    },
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
    if app.dispatching.contains_key(&task.id) {
        debug_assert_eq!(
            task.status,
            TaskStatus::Backlog,
            "dispatching set should only contain pre-dispatch (Backlog) tasks"
        );
        return CardIndicator::Dispatching {
            spinner_frame: app.spinner_tick,
        };
    }
    if task.sub_status == SubStatus::Conflict {
        return CardIndicator::Conflict;
    }
    if task.is_detached() {
        if let (TaskStatus::Review, Some(pr_url)) = (status, task.pr_url.as_deref()) {
            let pr_label = crate::models::url_label(pr_url);
            return CardIndicator::DetachedReview { pr_label };
        }
        return CardIndicator::Detached;
    }
    if task.sub_status == SubStatus::Crashed {
        return CardIndicator::Crashed;
    }
    if task.sub_status == SubStatus::Stale {
        // Derive from the hook timestamp so the label matches
        // ClassifyAgentActivity's source of truth and survives TUI restart.
        let inactive_mins = task
            .last_pre_tool_use_at
            .map(|ts| {
                now.signed_duration_since(ts)
                    .num_minutes()
                    .max(0)
                    .unsigned_abs()
            })
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
        Some(crate::models::TaskTag::PrReview) => " [pr-rev]",
        Some(crate::models::TaskTag::Research) => " [research]",
        Some(crate::models::TaskTag::Fix) => " [fix]",
        Some(crate::models::TaskTag::Dependabot) => " [dep]",
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

/// Braille spinner glyphs (10 frames). Indexed by `App::spinner_tick`,
/// advanced once per Tick while a dispatch is in flight.
const DISPATCHING_SPINNER: [&str; 10] = [
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280F}",
];

fn render_card_indicator(indicator: CardIndicator, labels: &[String]) -> Line<'static> {
    let (label, color) = match indicator {
        CardIndicator::Dispatching { spinner_frame } => {
            let glyph = DISPATCHING_SPINNER
                [(spinner_frame as usize) % crate::tui::DISPATCH_SPINNER_FRAMES as usize];
            (format!("{glyph} dispatching\u{2026}"), Color::Yellow)
        }
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
    let mut spans = vec![
        Span::raw("   "),
        Span::styled(label, Style::default().fg(color)),
    ];
    for label in labels {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("[{label}]"),
            Style::default().fg(MUTED),
        ));
    }
    Line::from(spans)
}

/// Returns a horizontal rule `Line` of box-drawing dashes spanning `width` columns.
pub(super) fn card_rule_line(color: Color, width: u16) -> Line<'static> {
    let rule = "\u{2500}".repeat(width as usize); // ─
    Line::from(Span::styled(rule, Style::default().fg(color)))
}

/// Render a decorative epic-header separator row (non-selectable).
/// Looks like: ── Epic Title ──────────────
pub(super) fn render_epic_header_item(epic: &Epic, col_width: u16) -> ListItem<'static> {
    let title = crate::tui::ui::shared::truncate(&epic.title, 38);
    let rule_count = (col_width as usize).saturating_sub(title.chars().count() + 5);
    let right_rule = "\u{2500}".repeat(rule_count);
    ListItem::new(Line::from(vec![
        Span::styled("\u{2500}\u{2500} ", Style::default().fg(MUTED)),
        Span::styled(title, Style::default().fg(PURPLE)),
        Span::styled(format!(" {}", right_rule), Style::default().fg(MUTED)),
    ]))
}

/// Build a styled two-line ListItem for a task card in a kanban column.
/// Line 1: stripe + title
/// Line 2: status icon + age/activity metadata
pub(super) fn build_task_list_item<'a>(
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

    let line2 = render_card_indicator(
        classify_card_indicator(task, status, app, now),
        &task.labels,
    );

    let rule_color = if is_cursor || has_message_flash {
        col_color
    } else {
        MUTED
    };
    let rule_line = card_rule_line(rule_color, col_width);
    let mut item = ListItem::new(vec![rule_line, line1, line2]);

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

fn epic_substatus_color(substatus: &EpicSubstatus) -> Color {
    match substatus {
        EpicSubstatus::Blocked(_) => Color::Yellow,
        EpicSubstatus::InReview => CYAN,
        EpicSubstatus::WrappingUp => GREEN,
        EpicSubstatus::Active | EpicSubstatus::Unplanned | EpicSubstatus::Planned => MUTED,
        EpicSubstatus::Done => MUTED,
    }
}

pub(super) fn render_epic_item(
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

    let rule_color = if is_cursor { PURPLE } else { MUTED };
    let rule_line = card_rule_line(rule_color, col_width);
    let mut item = ListItem::new(vec![rule_line, line1, line2]);

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
