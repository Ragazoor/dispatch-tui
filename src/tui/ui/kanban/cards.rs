//! Task and epic card rendering.

use chrono::{DateTime, TimeZone, Utc};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};

use crate::models::{format_age, Epic, EpicSubstatus, Staleness, SubStatus, Task, TaskStatus};
use crate::tui::{App, EpicStatsMap};

use super::super::shared::{staleness_color, truncate};
use super::{
    card_border_color, card_surface_color, column_color, cursor_border_color,
    selected_card_surface_color, status_icon,
};
use crate::palette::{CYAN, FG, FLASH_BG, GREEN, MUTED, PURPLE, RED, SIGNAL, YELLOW};

/// Format the title text for a task card (line 1 only — status annotations are on line 2).
fn format_task_title(task: &Task, max_title: usize) -> String {
    truncate(&task.title, max_title)
}

// ---------------------------------------------------------------------------
// CardIndicator — what to show on line 2 of a task card
// ---------------------------------------------------------------------------

/// Classifies a task's current state into a single display indicator.
/// Priority order matters: dispatching > unprovisioned > conflict >
/// detached-review > crashed > stale > blocked > detached-running > running >
/// review-pr > done-merged > idle. The `Dispatching` variant covers a task from
/// before its claim until the dispatch worker reports success or failure, so it
/// is reachable for a Backlog task (not yet claimed) *and* for a Running one
/// with no worktree (claimed, being provisioned). Its top priority is what keeps
/// that second state from rendering as a live agent — see `SpansTheClaim` on the
/// `DispatchingFeedback` surface in `docs/specs/dispatch.allium`.
///
/// `Unprovisioned` sits directly below it because every indicator beneath
/// describes a state that presupposes a worktree. It is also gated on
/// [`App::dispatch_may_be_in_flight`], because `Dispatching` membership only
/// spans the claims this TUI made: the epic chain claims inside the MCP close
/// path and a board restart empties the set, so the freshness gate covers the
/// dispatches `SpansTheClaim` cannot. See `UnprovisionedIndicator` in
/// `docs/specs/dispatch.allium`.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
enum CardIndicator {
    Dispatching {
        spinner_frame: u8,
    },
    Unprovisioned,
    AutoDispatchFailed,
    Conflict,
    DetachedReview {
        pr_label: String,
    },
    Detached,
    Crashed,
    Stale {
        /// `None` when the task has no recorded PreToolUse timestamp (e.g.
        /// pre-v50 rows, or transient state right after a manual transition
        /// into Running before the SeedActivity write commits). The card
        /// omits "· Xm" in that case rather than rendering a misleading "0m".
        inactive_mins: Option<u64>,
    },
    Blocked,
    Running {
        subagents: u32,
    },
    ReviewPr {
        pr_label: String,
    },
    DoneMerged {
        pr_label: String,
    },
    /// A phoenix task left sitting in Done with its flag intact: `PhoenixRespawn`
    /// clears the flag exactly when the successor row is created, so this IS the
    /// "the copy did not land" state (`Task::respawn_failed`). See "Phoenix
    /// marker" in `docs/specs/board-visuals.allium`.
    RespawnFailed,
    /// A scheduled launch failed and the task is back in backlog
    /// (`Task::schedule_failed`). See "Schedule marker" in
    /// `docs/specs/board-visuals.allium`.
    ScheduleFailed {
        /// [`schedule_suffix`]: a phoenix task re-armed after the failure
        /// still says when it next runs.
        schedule_suffix: String,
    },
    Idle {
        status: TaskStatus,
        age: String,
        staleness: Staleness,
        plan_indicator: &'static str,
        tag_suffix: String,
        /// [`schedule_suffix`]: the pending scheduled run, or `""`.
        schedule_suffix: String,
    },
}

/// The pending scheduled run on a card's metadata line (`board-visuals.allium`:
/// "Schedule marker"): ` · ◷ <Mon> <d> <HH:MM>` in `tz`, or ` · ◷ due` once
/// passed, or `""` with nothing pending. The board passes the machine's local
/// zone; the schedule itself is read in UTC, but the card answers "when will
/// this run" in the operator's terms.
fn schedule_suffix<Tz: TimeZone>(
    next_due: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    tz: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match next_due {
        None => String::new(),
        Some(due) if due <= now => " \u{00b7} \u{25f7} due".to_string(),
        Some(due) => format!(
            " \u{00b7} \u{25f7} {}",
            due.with_timezone(tz).format("%b %-d %H:%M")
        ),
    }
}

fn classify_card_indicator(
    task: &Task,
    status: TaskStatus,
    app: &App,
    now: DateTime<Utc>,
) -> CardIndicator {
    if app.is_dispatching(task.id) {
        // No assertion here on purpose. Membership implies "not yet provisioned",
        // but only up to message-queue latency (`SpansTheClaim` in
        // docs/specs/dispatch.allium), so a board refresh can briefly pair a
        // worktree-bearing row with a live membership — and a render function is
        // the wrong place to panic over a state the spec calls racy. The
        // invariant is asserted where it is definitionally true instead, in
        // `mark_dispatching`.
        return CardIndicator::Dispatching {
            spinner_frame: app.spinner_tick,
        };
    }
    // A claim that is still being provisioned looks identical to one whose
    // worker died. Only the second is worth alarming about, so keep rendering
    // the ordinary running card until the claim ages past the dispatch
    // watchdog window.
    if task.is_unprovisioned() && !app.dispatch_may_be_in_flight(task, now) {
        return CardIndicator::Unprovisioned;
    }
    // A subtask an epic chain claimed and then failed to provision. It sits in
    // backlog looking exactly like one that was never dispatched, so without
    // this the stalled epic is invisible (AutoDispatchFailureIndicator in
    // docs/specs/epics.allium). Below the dispatching check on purpose: a retry
    // in flight outranks the failure it is resolving.
    if app.auto_dispatch_failed(task.id) {
        return CardIndicator::AutoDispatchFailed;
    }
    if task.sub_status == SubStatus::Conflict {
        return CardIndicator::Conflict;
    }
    if task.is_detached() {
        if let (TaskStatus::Review, Some(u)) = (status, task.url.as_ref()) {
            let pr_label = u.label();
            return CardIndicator::DetachedReview { pr_label };
        }
        return CardIndicator::Detached;
    }
    if task.sub_status == SubStatus::Crashed {
        return CardIndicator::Crashed;
    }
    if task.sub_status == SubStatus::Stale {
        // Source of truth matches ClassifyAgentActivity so the label survives
        // TUI restart. None handling lives on the `inactive_mins` field doc.
        let inactive_mins = task.last_pre_tool_use_at.map(|ts| {
            now.signed_duration_since(ts)
                .num_minutes()
                .max(0)
                .unsigned_abs()
        });
        return CardIndicator::Stale { inactive_mins };
    }
    if status == TaskStatus::Running && task.sub_status == SubStatus::NeedsInput {
        return CardIndicator::Blocked;
    }
    if status == TaskStatus::Running {
        return CardIndicator::Running {
            subagents: task.live_subagents.max(0) as u32,
        };
    }
    if let (TaskStatus::Review, Some(u)) = (status, task.url.as_ref()) {
        let pr_label = u.label();
        return CardIndicator::ReviewPr { pr_label };
    }
    // Above DoneMerged deliberately, and that is the only arm it competes with:
    // every indicator classified earlier presupposes Running or Review (a
    // worktree, a live agent, a sub-status Done resets), so this cannot mask
    // one. A task completed by a PR merge whose respawn then failed must report
    // the failure rather than the merge.
    if task.respawn_failed() {
        return CardIndicator::RespawnFailed;
    }
    if let (TaskStatus::Done, Some(u)) = (status, task.url.as_ref()) {
        let pr_label = u.label();
        return CardIndicator::DoneMerged { pr_label };
    }
    // Backlog only, so it competes with nothing above: every earlier arm
    // presupposes Running, Review or Done, or a dispatch in flight, which
    // outranks the failure it may be retrying.
    // The board's own zone, once, for both arms that show it.
    let schedule_suffix = schedule_suffix(task.next_due, now, &chrono::Local);
    if task.schedule_failed() {
        return CardIndicator::ScheduleFailed { schedule_suffix };
    }

    let age = format_age(task.updated_at, now);
    let staleness = Staleness::from_age(task.updated_at, now);
    let plan_indicator = if task.plan_path.is_some() && status == TaskStatus::Backlog {
        "▸ "
    } else {
        ""
    };
    let tag_suffix = task
        .tag
        .map(|t| format!(" [{}]", t.short_label()))
        .unwrap_or_default();
    CardIndicator::Idle {
        status,
        age,
        staleness,
        plan_indicator,
        tag_suffix,
        schedule_suffix,
    }
}

/// The border colour a card's state claims, or `None` if it claims none.
///
/// The card frame carries *state*, not identity (`board-visuals.allium`: "Selection" and
/// "Border as state"). Red is the six hard failures — the same states whose
/// indicator renders a `⚠`. Note that is an agreement between two independent
/// exhaustive matches over one enum, not a derivation: `render_card_indicator`
/// picks the glyph, this picks the border, and only
/// `every_indicator_claims_the_border_its_severity_earns` keeps them in step. A
/// `CardIndicator::severity()` consumed by both would make it true by
/// construction. Amber is the three that want a human's
/// attention without being broken.
///
/// `Dispatching` is deliberately absent despite rendering an amber indicator: it
/// is a transient expected state, and bordering it would make every ordinary
/// dispatch alarm for its duration.
///
/// Matched exhaustively on purpose — a new `CardIndicator` variant must decide
/// whether it is alarming rather than defaulting to silence.
fn state_border_color(indicator: &CardIndicator) -> Option<Color> {
    match indicator {
        CardIndicator::Unprovisioned
        | CardIndicator::AutoDispatchFailed
        | CardIndicator::Conflict
        | CardIndicator::Crashed
        | CardIndicator::RespawnFailed
        | CardIndicator::ScheduleFailed { .. } => Some(RED),
        CardIndicator::Blocked | CardIndicator::Stale { .. } => Some(YELLOW),
        CardIndicator::Dispatching { .. }
        | CardIndicator::DetachedReview { .. }
        | CardIndicator::Detached
        | CardIndicator::Running { .. }
        | CardIndicator::ReviewPr { .. }
        | CardIndicator::DoneMerged { .. }
        | CardIndicator::Idle { .. } => None,
    }
}

/// The card frame's colour: cursor, then state, then the resting neutral.
///
/// Shared by task and epic cards so the precedence is stated once. Epic cards
/// pass `None` — they carry no `CardIndicator` and so claim no state colour — but
/// the cursor rule is identical for both, and the point of the cursor white is
/// that it is the same everywhere.
fn resolve_frame_color(is_cursor: bool, state: Option<Color>) -> Color {
    if is_cursor {
        cursor_border_color()
    } else {
        state.unwrap_or_else(card_border_color)
    }
}

/// Braille spinner glyphs (10 frames). Indexed by `App::spinner_tick`,
/// advanced once per Tick while a dispatch is in flight.
const DISPATCHING_SPINNER: [&str; 10] = [
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280F}",
];

/// `"N {singular}"` / `"N {plural}"`, or `None` at zero (the card omits the
/// suffix entirely rather than rendering e.g. "running · 0 agents").
fn count_suffix(n: u32, singular: &str, plural: &str) -> Option<String> {
    match n {
        0 => None,
        1 => Some(format!("1 {singular}")),
        n => Some(format!("{n} {plural}")),
    }
}

/// Line 2 of a task card: the state indicator, then the chips.
fn render_card_indicator(indicator: CardIndicator, labels: &[String]) -> Line<'static> {
    let (label, color) = match indicator {
        CardIndicator::Dispatching { spinner_frame } => {
            let glyph = DISPATCHING_SPINNER
                [(spinner_frame as usize) % crate::tui::DISPATCH_SPINNER_FRAMES as usize];
            (format!("{glyph} dispatching\u{2026}"), YELLOW)
        }
        CardIndicator::Unprovisioned => ("\u{26a0} no worktree".to_string(), RED),
        CardIndicator::AutoDispatchFailed => ("\u{26a0} auto-dispatch failed".to_string(), RED),
        CardIndicator::Conflict => ("\u{26a0} rebase conflict".to_string(), RED),
        CardIndicator::DetachedReview { pr_label } => (format!("\u{25cb} {pr_label}"), CYAN),
        CardIndicator::Detached => ("\u{25cb} detached".to_string(), MUTED),
        CardIndicator::Crashed => ("\u{26a0} crashed".to_string(), RED),
        CardIndicator::Stale { inactive_mins } => {
            let label = match inactive_mins {
                Some(m) => format!("\u{25c9} stale \u{00b7} {m}m"),
                None => "\u{25c9} stale".to_string(),
            };
            (label, YELLOW)
        }
        CardIndicator::Blocked => ("\u{25c9} blocked".to_string(), YELLOW),
        CardIndicator::Running { subagents } => {
            let icon = status_icon(TaskStatus::Running);
            let mut label = format!("{icon} running");
            if let Some(suffix) = count_suffix(subagents, "agent", "agents") {
                label.push_str(" \u{00b7} ");
                label.push_str(&suffix);
            }
            (label, CYAN)
        }
        CardIndicator::ReviewPr { pr_label } => (format!("\u{25cf} {pr_label}"), CYAN),
        CardIndicator::DoneMerged { pr_label } => (format!("\u{2714} {pr_label} merged"), GREEN),
        CardIndicator::RespawnFailed => ("\u{26a0} respawn failed".to_string(), RED),
        CardIndicator::ScheduleFailed { schedule_suffix } => {
            (format!("\u{26a0} schedule failed{schedule_suffix}"), RED)
        }
        CardIndicator::Idle {
            status,
            age,
            staleness,
            plan_indicator,
            tag_suffix,
            schedule_suffix,
        } => {
            let icon = status_icon(status);
            (
                format!("{plan_indicator}{icon} {age}{tag_suffix}{schedule_suffix}"),
                staleness_color(staleness),
            )
        }
    };
    let mut spans = vec![
        Span::raw("   "),
        Span::styled(label, Style::default().fg(color)),
    ];
    for chip in labels {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("[{chip}]"),
            Style::default().fg(label_color(chip)),
        ));
    }
    Line::from(spans)
}

/// Colour for one `[label]` badge (board-visuals.allium "Card label badges"). Labels are
/// muted grey — context, not state — except the three CI-status texts, which
/// take the same colours the card indicator uses for the same three meanings.
///
/// Matched by EXACT text, deliberately not by a `ci:` prefix: an unrecognised
/// value from a newer feed script renders as an ordinary muted badge rather
/// than guessing a colour. Same leniency feed ingest applies to unknown
/// signals.
fn label_color(chip: &str) -> Color {
    match chip {
        "ci:pass" => GREEN,
        "ci:fail" => RED,
        "ci:pending" => YELLOW,
        _ => MUTED,
    }
}

/// Render a decorative epic-header separator row (non-selectable).
/// Shows the epic's own id, then its ancestor breadcrumb:
/// `── #NNN root › … › self ──────────`.
pub(super) fn render_epic_header_item(
    epic: &Epic,
    epics: &[Epic],
    col_width: u16,
) -> ListItem<'static> {
    // The id is the epic's own — the one whose task group this header
    // precedes — never one per breadcrumb ancestor (board-layout.allium,
    // "Flattening"). Digit count matches the `id_len` convention
    // `render_epic_item`/`build_task_list_item` use for their own id badge.
    let id_digits = epic.id.0.unsigned_abs().max(1).ilog10() as usize + 1;
    let id_prefix = format!("#{} ", epic.id);
    let id_prefix_len = id_digits + 2; // '#' and the trailing space

    // Text budget matches the prior single-title layout: "── " + text + " " + rule
    // reserves `text_len + 5`, plus the id prefix's own width, so the text may
    // use up to `col_width - 5 - id_prefix_len` chars.
    let reserved = 5 + id_prefix_len;
    let budget = (col_width as usize).saturating_sub(reserved);
    let segments = crate::models::epics::ancestor_titles(epic, epics);
    let title = crate::tui::ui::shared::fair_truncate_segments(
        &segments,
        budget,
        crate::tui::ui::shared::BREADCRUMB_SEPARATOR,
    );
    let rule_count = (col_width as usize).saturating_sub(title.chars().count() + reserved);
    let right_rule = "\u{2500}".repeat(rule_count);
    ListItem::new(Line::from(vec![
        Span::styled("\u{2500}\u{2500} ", Style::default().fg(MUTED)),
        Span::styled(id_prefix, Style::default().fg(MUTED)),
        Span::styled(title, Style::default().fg(PURPLE)),
        Span::styled(format!(" {}", right_rule), Style::default().fg(MUTED)),
    ]))
}

/// A folded epic group's header: the same id and ancestor breadcrumb
/// [`render_epic_header_item`] draws, plus the count of cards it is hiding
/// and a fold marker — same relationship as `render_folded_section_header`
/// has to `render_substatus_header` (board-layout.allium: "Epic Folding").
///
/// Drawn in the same single-style row every folded header uses
/// (`section_header_item`), rather than `render_epic_header_item`'s coloured
/// spans: a folded header is a summary line, not a card, and only one of the
/// two needs to hold the cursor.
pub(super) fn render_folded_epic_header(
    header: &crate::tui::types::FoldedEpicHeader<'_>,
    epics: &[Epic],
    col_width: u16,
    first: bool,
    is_cursor: bool,
) -> ListItem<'static> {
    let epic = header.epic;
    let id_digits = epic.id.0.unsigned_abs().max(1).ilog10() as usize + 1;
    let id_prefix = format!("#{} ", epic.id);
    let id_prefix_len = id_digits + 2;
    let suffix = format!(" ({}) \u{22ef}", header.hidden);

    let reserved = "\u{2500}\u{2500} ".chars().count() + id_prefix_len + suffix.chars().count();
    let budget = (col_width as usize).saturating_sub(reserved);
    let segments = crate::models::epics::ancestor_titles(epic, epics);
    let title = crate::tui::ui::shared::fair_truncate_segments(
        &segments,
        budget,
        crate::tui::ui::shared::BREADCRUMB_SEPARATOR,
    );

    crate::tui::ui::shared::section_header_item(
        format!("\u{2500}\u{2500} {}{}{}", id_prefix, title, suffix),
        first,
        is_cursor,
    )
}

/// Per-column rendering context shared by every card in a column.
///
/// Bundles the column-level parameters threaded through the card/epic
/// renderers: the stripe colour, the column width, and the column's neutral
/// ground.
///
/// All three are constant across a column. `ground` is needed because a card is
/// inset by [`CARD_MARGIN`] on each side, so the renderer has to paint the
/// margin cells in the column's own ground rather than let them inherit the card
/// surface (`board-visuals.allium`: "Task card frame").
pub(super) struct ColRenderCtx {
    pub color: Color,
    pub width: u16,
    pub ground: Color,
}

/// The marker a phoenix task's title line carries (`♻`, U+267B). See "Phoenix
/// marker" in `docs/specs/board-visuals.allium`: it says "this task still owes a
/// respawn" — normal in backlog, a failure in done.
pub(super) const PHOENIX_GLYPH: &str = "\u{267b}";

/// Cells the marker costs on line 1: a leading space plus the glyph.
///
/// The glyph counts as ONE `char` and draws as TWO columns — it has emoji
/// presentation in most terminals — so this is the one place on the board where
/// a `chars().count()` budget is wrong. Every other card glyph (`▎ ✉ ➤ ▸ ✓ ⚠`)
/// is a single-column BMP character, which is why the rest of the prefix maths
/// counts characters and gets away with it.
const PHOENIX_MARKER_WIDTH: usize = 3;

/// The new-task marker (`◆`, U+25C6), single-column BMP like the flash glyphs.
/// See "New-task signal" in `docs/specs/board-visuals.allium`.
pub(super) const UNSEEN_GLYPH: &str = "\u{25c6}";

/// Cells the marker costs on line 1: a leading space plus the glyph.
const UNSEEN_MARKER_WIDTH: usize = 2;

/// Columns of column-ground left visible on each side of a card, so cards float
/// inside the column instead of tiling flush against its edges.
const CARD_MARGIN: usize = 1;

/// Total horizontal cells a card spends on chrome: two ground margins plus two
/// frame rails. Subtracted from the column width to get the content budget.
pub(super) const CARD_CHROME_WIDTH: usize = CARD_MARGIN * 2 + 2;

/// Wrap a card's two content lines in a complete rounded frame.
///
/// Returns the four lines of a framed card — top border, the two content lines
/// each flanked by rails, and the bottom border (`board-visuals.allium`: "Task card
/// frame"). Every card carries its own full frame; borders are never shared
/// between neighbours.
///
/// The whole card is lit, frame included: the caller sets the card surface as
/// the `ListItem`'s base background, so the border rows and rails inherit it and
/// a card's boundary is the change of colour at its outer edge. Only the margin
/// spans override that, back to the column ground.
///
/// `frame_color` is the caller's already-resolved state: the column's identity
/// colour for the selected card, the resting neutral otherwise.
fn frame_card(
    line1: Line<'static>,
    line2: Line<'static>,
    col_width: u16,
    frame_color: Color,
    ground: Color,
) -> Vec<Line<'static>> {
    let card_width = (col_width as usize).saturating_sub(CARD_MARGIN * 2);
    let content_width = card_width.saturating_sub(2);
    let style = Style::default().fg(frame_color);
    // A `&'static str`, not `" ".repeat(CARD_MARGIN)`: the repeat heap-allocates a
    // fresh String every call, and this closure runs eight times per card — twice
    // per rail and twice per border — on every frame.
    const MARGIN_STR: &str = " ";
    debug_assert_eq!(
        MARGIN_STR.len(),
        CARD_MARGIN,
        "margin literal must match CARD_MARGIN"
    );
    let margin = || Span::styled(MARGIN_STR, Style::default().bg(ground));
    let horizontal = "\u{2500}".repeat(content_width);

    let rail = |inner: Line<'static>| -> Line<'static> {
        let pad = content_width.saturating_sub(inner.width());
        let mut spans = vec![margin(), Span::styled("\u{2502}", style)];
        spans.extend(inner.spans);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        spans.push(Span::styled("\u{2502}", style));
        spans.push(margin());
        Line::from(spans)
    };

    let border = |left: &str, right: &str| -> Line<'static> {
        Line::from(vec![
            margin(),
            Span::styled(format!("{left}{horizontal}{right}"), style),
            margin(),
        ])
    };

    vec![
        border("\u{256d}", "\u{256e}"),
        rail(line1),
        rail(line2),
        border("\u{2570}", "\u{256f}"),
    ]
}

/// The marker a task's title line carries for its phoenix flag, or `""`.
fn phoenix_marker(task: &Task) -> &'static str {
    if task.phoenix {
        PHOENIX_GLYPH
    } else {
        ""
    }
}

/// Every cell on a task card's line 1 other than the title itself, so the
/// title's budget is what remains.
///
/// Kept as one function because the terms are not uniform: most are character
/// counts, but the phoenix marker is a *rendered* width (see
/// `PHOENIX_MARKER_WIDTH`). Splitting them across the call site is how the two
/// measures get confused.
fn task_card_prefix_width(task: &Task, flash_received: bool, flash_sent: bool) -> usize {
    // select(2) + stripe(1) + " <lead> "(lead_len+2) + flash glyphs (" ✉"/" ➤",
    // 2 each) + the phoenix marker + the card's own chrome. Agent names are
    // lowercase ASCII, so their character count is their rendered width.
    let lead_len = match &task.agent_name {
        Some(name) => name.chars().count(),
        None => task.id.0.unsigned_abs().max(1).ilog10() as usize + 2,
    };
    let flash_width = if flash_received { 2 } else { 0 } + if flash_sent { 2 } else { 0 };
    let phoenix_width = if task.phoenix {
        PHOENIX_MARKER_WIDTH
    } else {
        0
    };
    2 + 1 + 2 + lead_len + flash_width + phoenix_width + CARD_CHROME_WIDTH
}

/// Whether the task has a live received / sent message flash.
fn message_flashes(task: &Task, app: &App) -> (bool, bool) {
    // Same threshold the expiry sweep uses (`App::tick_message_flash`), read from
    // the one constant so the two cannot drift apart.
    let has_message_flash = app
        .agents
        .message_flash
        .get(&task.id)
        .is_some_and(|t| t.elapsed() < crate::tui::MESSAGE_FLASH_TTL);
    // Sent-flash sibling (task #4098): a distinct glyph, same fill/TTL. Both
    // can be true at once — a task that sent and received within the same
    // window shows both glyphs, per board-visuals.allium's "Message flash".
    let has_message_flash_sent = app
        .agents
        .message_flash_sent
        .get(&task.id)
        .is_some_and(|t| t.elapsed() < crate::tui::MESSAGE_FLASH_TTL);
    (has_message_flash, has_message_flash_sent)
}

/// Line 1 of a task card: select prefix, stripe, lead word, title, flash glyphs and
/// the phoenix marker. The title is bold when the card is batch-selected or
/// under the cursor.
fn card_title_line<'a>(
    task: &Task,
    title_text: &str,
    is_batch_selected: bool,
    is_cursor: bool,
    col_color: Color,
    (has_message_flash, has_message_flash_sent): (bool, bool),
) -> Line<'a> {
    // Line 1: prefix + stripe + title.
    // One quarter block on every card, cursor included (board-visuals.allium: "Card
    // stripe"). Stripe weight no longer moves with the cursor — selection is
    // carried by the frame hue and the bold title.
    let select_prefix = if is_batch_selected { "* " } else { "  " };
    let stripe_char = "\u{258e}";
    let stripe_style = Style::default().fg(col_color);
    // Bold marks the selected card's title (board-visuals.allium: "Selection"). Its fill
    // is unchanged from a resting card's, by design.
    let title_style = if is_batch_selected || is_cursor {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let mut line1_spans = vec![
        Span::styled(select_prefix.to_string(), title_style),
        Span::styled(stripe_char, stripe_style),
        Span::styled(format!(" {} ", task.lead()), Style::default().fg(MUTED)),
        Span::styled(title_text.to_string(), title_style),
    ];
    if has_message_flash {
        line1_spans.push(Span::styled(" \u{2709}", Style::default().fg(YELLOW)));
    }
    if has_message_flash_sent {
        line1_spans.push(Span::styled(" \u{27a4}", Style::default().fg(YELLOW)));
    }
    // The marker is an attribute of the task, not a state, so it takes no
    // colour of its own and contributes nothing to the border — exactly like the
    // flash glyphs above. In done, the *indicator* beneath says the respawn
    // failed and the frame turns red; see "Phoenix marker" in board-visuals.allium.
    let marker = phoenix_marker(task);
    if !marker.is_empty() {
        line1_spans.push(Span::styled(
            format!(" {marker}"),
            Style::default().fg(MUTED),
        ));
    }

    Line::from(line1_spans)
}

/// Build a styled framed ListItem for a task card in a kanban column.
/// Line 1: stripe + title
/// Line 2: status icon + age/activity metadata
pub(super) fn build_task_list_item<'a>(
    task: &Task,
    status: TaskStatus,
    app: &App,
    now: DateTime<Utc>,
    is_cursor: bool,
    ctx: &ColRenderCtx,
) -> ListItem<'a> {
    let col_color = ctx.color;
    let col_width = ctx.width;

    let is_batch_selected = app.selected_tasks().contains(&task.id);

    let (has_message_flash, has_message_flash_sent) = message_flashes(task, app);
    let any_message_flash = has_message_flash || has_message_flash_sent;

    let unseen = app.board.unseen_tasks.contains(&task.id);
    let prefix_width = task_card_prefix_width(task, has_message_flash, has_message_flash_sent)
        + if unseen { UNSEEN_MARKER_WIDTH } else { 0 };
    let max_title = (col_width as usize).saturating_sub(prefix_width);
    let title_text = format_task_title(task, max_title);

    let mut line1 = card_title_line(
        task,
        &title_text,
        is_batch_selected,
        is_cursor,
        col_color,
        (has_message_flash, has_message_flash_sent),
    );
    // A glyph only: the stripe carries column identity and the frame carries
    // state, so neither is touched ("New-task signal", board-visuals.allium).
    if unseen {
        line1.spans.push(Span::styled(
            format!(" {UNSEEN_GLYPH}"),
            Style::default().fg(SIGNAL),
        ));
    }

    let indicator = classify_card_indicator(task, status, app, now);
    // Read the severity off the indicator before it is consumed, so the border and
    // the glyph beneath it come from one classification rather than two.
    let state_border = state_border_color(&indicator);
    let line2 = render_card_indicator(indicator, &task.labels);

    // Precedence: cursor, then state, then neutral (`board-visuals.allium`: "Selection").
    //
    // The frame carries *state*, not identity — a hued border means something is
    // wrong, not that this is the column's colour. The cursor takes a white of its
    // own precisely so it is not competing for the alarm hues.
    //
    // The cursor winning means an unhealthy card that is *also* the cursor shows
    // white rather than its state colour. That is the accepted cost: its indicator
    // line still says so directly beneath, and the alternative hides the cursor on
    // the card you just navigated to, which is worse.
    //
    // The flash contributes nothing here. It is carried by its fill and its
    // glyph(s) — envelope for received, outgoing arrow for sent — which are
    // the only things no other card has.
    let frame_color = resolve_frame_color(is_cursor, state_border);
    // A flash replaces the card surface for its duration; that differing fill is
    // what keeps it distinguishable from the selection despite sharing the hue.
    // Sent and received share one fill — the glyph, not the fill, carries the
    // direction.
    //
    // The selected card asks for its own surface even though
    // `SelectionDoesNotLiftTheFill` pins it equal to a resting card's. Routing
    // the call this way keeps the equality load-bearing: if the two ever diverge,
    // the render follows and `selection_does_not_lift_the_fill` fails.
    let surface = if any_message_flash {
        FLASH_BG
    } else if is_cursor {
        selected_card_surface_color()
    } else {
        card_surface_color()
    };
    let mut item = ListItem::new(frame_card(line1, line2, col_width, frame_color, ctx.ground));

    // Base style for the whole card, so the frame rows inherit the surface and
    // the card reads as one lit object. Margin spans override back to ground.
    let mut style = Style::default().bg(surface).fg(FG);
    if any_message_flash {
        style = style.add_modifier(Modifier::BOLD);
    }
    item = item.style(style);

    item
}

fn epic_substatus_color(substatus: &EpicSubstatus) -> Color {
    match substatus {
        EpicSubstatus::Blocked(_) => YELLOW,
        EpicSubstatus::InReview => CYAN,
        EpicSubstatus::Active | EpicSubstatus::Unplanned | EpicSubstatus::Planned => MUTED,
        EpicSubstatus::Done => MUTED,
    }
}

pub(super) fn render_epic_item(
    epic: &Epic,
    is_cursor: bool,
    app: &App,
    epic_stats: &EpicStatsMap,
    // `substatus` is the state this copy of the card is in, resolved by the
    // caller because only the caller knows which column it is drawing — its
    // own placement there, not the epic's board-wide substatus.
    substatus: EpicSubstatus,
    status: TaskStatus,
    ctx: &ColRenderCtx,
) -> ListItem<'static> {
    let col_width = ctx.width;
    let stats = epic_stats.get(&epic.id);

    let plan_indicator = if epic.plan_path.is_some() && status == TaskStatus::Backlog {
        " \u{25b8}" // ▸
    } else {
        ""
    };

    // Prefix: select(2) + stripe(1) + " #NNN "(id_len+3) + plan_indicator
    let id_len = epic.id.0.unsigned_abs().max(1).ilog10() as usize + 1;
    let unseen = app.board.epic_has_unseen(epic.id);
    let prefix_width = 2
        + 1
        + 3
        + id_len
        + plan_indicator.chars().count()
        + if unseen { UNSEEN_MARKER_WIDTH } else { 0 }
        + CARD_CHROME_WIDTH;
    let max_title = (col_width as usize).saturating_sub(prefix_width);
    let title_text = truncate(&epic.title, max_title);

    let is_batch_selected = app.selected_epics().contains(&epic.id);
    let select_prefix = if is_batch_selected { "* " } else { "  " };

    // Line 1: stripe + title. One quarter block on every card, cursor included
    // (board-visuals.allium: "Card stripe").
    let stripe_char = "\u{258e}";
    let title_style = Style::default().fg(PURPLE).add_modifier(Modifier::BOLD);
    // A marked epic's stripe takes SIGNAL in place of PURPLE, and its title line
    // ends in the glyph that says why ("New-task signal", board-visuals.allium).
    let stripe_color = if unseen { SIGNAL } else { PURPLE };
    let mut line1_spans = vec![
        Span::raw(select_prefix.to_string()),
        Span::styled(stripe_char, Style::default().fg(stripe_color)),
        Span::styled(format!(" #{} ", epic.id), Style::default().fg(MUTED)),
        Span::styled(format!("{title_text}{plan_indicator}"), title_style),
    ];
    if unseen {
        line1_spans.push(Span::styled(
            format!(" {UNSEEN_GLYPH}"),
            Style::default().fg(SIGNAL),
        ));
    }
    let line1 = Line::from(line1_spans);

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
        // The counts are the whole subtree and read the same on every copy of
        // this card; the label is per column, because a card sitting in Running
        // labelled "done" would contradict the column it is in
        // (board-layout.allium, "Epic Card Placement").
        spans.push(Span::styled(
            substatus.label(),
            Style::default().fg(epic_substatus_color(&substatus)),
        ));
        Line::from(spans)
    } else {
        Line::from(vec![
            Span::raw("    "),
            Span::styled("no subtasks", Style::default().fg(MUTED)),
        ])
    };

    // The cursor white applies to epic cards too, so "white frame" means cursor
    // everywhere rather than on task cards only. An epic's own PURPLE identity
    // stays on its stripe and title, which it keeps at rest.
    //
    // This matters more on an epic than on a task: an epic's title is bold
    // unconditionally, so the frame is the *only* cursor signal an epic card has.
    // Epics carry no CardIndicator and so claim no state colour; a resting epic
    // frame is neutral like any other.
    // Epics carry no CardIndicator, so they claim no state colour — but the
    // precedence itself is shared, so the rule lives in one place.
    let frame_color = resolve_frame_color(is_cursor, None);
    ListItem::new(frame_card(line1, line2, col_width, frame_color, ctx.ground))
        .style(Style::default().bg(card_surface_color()).fg(FG))
}

/// A work package's card (`work-packages.allium: PackageCardPlacement`).
///
/// Shaped like an epic card so it reads as a container, but in CYAN rather
/// than an epic's PURPLE and marked with a square glyph, so the two kinds of
/// grouping are never mistaken for each other. Line 2 counts the linked items
/// the board holds and how many of them are done; a package has no status of
/// its own to show.
pub(super) fn render_package_item(
    package: &crate::models::WorkPackage,
    is_cursor: bool,
    progress: (usize, usize),
    ctx: &ColRenderCtx,
) -> ListItem<'static> {
    let col_width = ctx.width;
    // Prefix: select(2) + stripe(1) + " ▣ "(3)
    let prefix_width = 2 + 1 + 3 + CARD_CHROME_WIDTH;
    let max_title = (col_width as usize).saturating_sub(prefix_width);
    let title_text = truncate(&package.title, max_title);

    let line1 = Line::from(vec![
        Span::raw("  "),
        Span::styled("\u{258e}", Style::default().fg(CYAN)),
        Span::styled(" \u{25a3} ", Style::default().fg(CYAN)),
        Span::styled(
            title_text,
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ),
    ]);
    let (done, total) = progress;
    let summary = match total {
        0 => "no items".to_string(),
        1 => format!("1 item \u{b7} {done} done"),
        n => format!("{n} items \u{b7} {done} done"),
    };
    let line2 = Line::from(vec![
        Span::raw("    "),
        Span::styled(summary, Style::default().fg(MUTED)),
    ]);
    let frame_color = resolve_frame_color(is_cursor, None);
    ListItem::new(frame_card(line1, line2, col_width, frame_color, ctx.ground))
        .style(Style::default().bg(card_surface_color()).fg(FG))
}

#[cfg(test)]
mod tests;
