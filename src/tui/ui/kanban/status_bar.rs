//! Status bar at the bottom of the kanban board.
//!
//! Renders one of two flavours depending on app state:
//! * a transient status message, or
//! * mode-specific hints (Normal mode delegates to `action_hints` /
//!   `epic_action_hints` / `batch_action_hints`).

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::super::shared::{text_field_hint, HintBar};
use super::{action_hints, epic_action_hints};
use crate::keybindings::{
    hint_key, hint_keys, hint_row_keys, keys_of, yes_no_hint, KeyContext as C, KeyNamespace as N,
};
use crate::palette::{CYAN, GREEN, MUTED, PURPLE, RED, YELLOW};
use crate::tui::{App, ColumnItem, InputMode};

pub(super) fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let (line, style) = status_line(app);
    frame.render_widget(Paragraph::new(line).style(style), area);
}

/// Prepend `prefix` spans to `spans` in place, preserving order.
fn prepend(spans: &mut Vec<Span<'static>>, mut prefix: Vec<Span<'static>>) {
    prefix.append(spans);
    *spans = prefix;
}

/// A simple text status line with a single foreground colour, falling back to
/// `app.status.message` when set. (`app.status.message` being `Some` is already
/// short-circuited by `status_line`, so this always resolves to `default`; the
/// override arms retain this shape for clarity.)
fn hint_text(app: &App, default: &str, color: Color) -> (Line<'static>, Style) {
    let text = app.status.message.as_deref().unwrap_or(default).to_string();
    (Line::from(text), Style::default().fg(color))
}

/// A fixed text status line with a single foreground colour. Accepts anything
/// that converts into a `Line<'static>`, so string literals borrow without an
/// allocation while owned `String`s (e.g. a formatted search prompt) move in.
fn hint(text: impl Into<Line<'static>>, color: Color) -> (Line<'static>, Style) {
    (text.into(), Style::default().fg(color))
}

/// Compute the status bar content (a styled `Line` plus a base paragraph style)
/// for the current app state. Rendering happens once, in `render_status_bar`.
///
/// The composed Normal-mode hint line is the one structurally-heavier flavour;
/// it lives in its own builder (`normal_status_line`), and everything else is
/// a fixed per-mode hint.
pub(in crate::tui) fn status_line(app: &App) -> (Line<'static>, Style) {
    if let Some(msg) = &app.status.message {
        return (Line::from(msg.clone()), Style::default().fg(YELLOW));
    }

    let t = app.key_table;
    let key = |ns, action, shown| hint_key(t, ns, None, action, shown);
    let yes_no = |ns| yes_no_hint(t, ns);
    let text_keys = |verb: &str| text_field_hint(t, verb, "cancel");
    let editor_cancel = || key(N::BoardDescriptionEditor, "cancel_input", "Esc");
    match &app.input.mode {
        InputMode::Normal => normal_status_line(app),
        InputMode::SearchTasks => hint(
            format!(
                "Search board: {}_   [{}] keep  [{}] cancel",
                app.search.query,
                key(N::BoardSearch, "search_commit", "Enter"),
                key(N::BoardSearch, "search_cancel", "Esc"),
            ),
            CYAN,
        ),
        InputMode::InputTitle => hint("Creating task: enter title", YELLOW),
        InputMode::InputDescription => hint(
            format!(
                "Creating task: opening $EDITOR for description  [{}] cancel",
                editor_cancel()
            ),
            YELLOW,
        ),
        InputMode::InputRepoPath => hint("Creating task: enter repo path", YELLOW),
        InputMode::InputTag => hint_text(
            app,
            &crate::tui::ui::tag_prompt(t, app.input.phoenix_armed()),
            YELLOW,
        ),
        InputMode::ConfirmDeleteTask(_) => hint_text(
            app,
            &format!("Delete? {}", yes_no(N::BoardConfirmDeleteTask)),
            RED,
        ),
        InputMode::QuickDispatch => hint("Quick dispatch: select repo path", YELLOW),
        InputMode::ConfirmRetry(_) => hint(crate::tui::ui::retry_keys(t, true), RED),
        InputMode::ConfirmBatchDelete => hint_text(
            app,
            &format!("Delete items? {}", yes_no(N::BoardConfirmBatchDelete)),
            RED,
        ),
        InputMode::ConfirmDone => hint_text(
            app,
            &format!("Move to Done? {}", yes_no(N::BoardConfirmDone)),
            YELLOW,
        ),
        InputMode::InputEpicTitle => hint("Creating epic: enter title", PURPLE),
        InputMode::InputEpicDescription => hint(
            format!(
                "Creating epic: opening $EDITOR for description  [{}] cancel",
                editor_cancel()
            ),
            PURPLE,
        ),
        InputMode::ConfirmDeleteEpic => hint_text(
            app,
            &format!(
                "Delete epic and subtasks? {}",
                yes_no(N::BoardConfirmDeleteEpic)
            ),
            RED,
        ),
        InputMode::Help => {
            let keys: Vec<String> = keys_of(t, N::BoardHelp, "close_help")
                .iter()
                .map(|k| format!("[{k}]"))
                .collect();
            hint(format!("{} to close help", keys.join(" or ")), CYAN)
        }
        InputMode::BoardFilter => hint(
            format!(
                "Board filter: [{}] next tab  [{}] toggle  [{}] close",
                key(N::BoardFilter, "filter_next_tab", "Tab"),
                key(N::BoardFilter, "filter_toggle_row", "Space"),
                hint_keys(t, N::BoardFilter, &[("filter_close", "q"), ("filter_back", "Esc")]),
            ),
            CYAN,
        ),
        InputMode::BoardFilterSearch => hint(
            format!(
                "Search epics: {}_   [{}] keep  [{}] clear",
                app.filter_overlay.epic_query,
                key(N::BoardFilterSearch, "filter_search_commit", "Enter"),
                key(N::BoardFilterSearch, "filter_search_cancel", "Esc"),
            ),
            CYAN,
        ),
        InputMode::ConfirmDeleteRepoPath => hint(
            format!(
                "Delete repo path? {} to confirm, any key to cancel",
                key(N::BoardConfirmDeleteRepoPath, "confirm_delete_repo_path_yes", "y")
            ),
            YELLOW,
        ),
        InputMode::ConfirmDetachTmux(_) => hint_text(
            app,
            &format!("Detach tmux panel? {}", yes_no(N::BoardConfirmDetachTmux)),
            YELLOW,
        ),
        InputMode::ConfirmOverrideFeedOwner { .. } => hint_text(
            app,
            &format!(
                "Take over polling for this feed? {}",
                yes_no(N::BoardConfirmOverrideFeedOwner)
            ),
            YELLOW,
        ),
        InputMode::ConfirmQuit => hint(
            format!("Quit dispatch? {}", yes_no(N::BoardConfirmQuit)),
            YELLOW,
        ),
        InputMode::InputBaseBranch => hint_text(app, "Base branch: ", YELLOW),
        InputMode::InputWrapUpMode => {
            hint_text(app, &crate::tui::ui::wrap_up_prompt(t), YELLOW)
        }
        InputMode::InputPlacement => {
            let ns = N::BoardPickerPlacement;
            hint(
                format!(
                    "[{}] public/private  [{}] workspace  [{}] create  [{}] cancel",
                    key(ns, "placement_picker_visibility", "Tab"),
                    hint_keys(
                        t,
                        ns,
                        &[
                            ("placement_picker_workspace_down", "j"),
                            ("placement_picker_workspace_up", "k")
                        ]
                    ),
                    hint_row_keys(t, ns, "placement_picker_create", &["Space", "Enter"]),
                    key(ns, "placement_picker_cancel", "Esc"),
                ),
                YELLOW,
            )
        }
        InputMode::InputModel => {
            let seeded = app.input.task_draft.as_ref().and_then(|d| d.model);
            hint_text(app, &crate::tui::ui::model_prompt(t, seeded), YELLOW)
        }
        InputMode::ReparentEpic(_) => hint(
            format!(
                "Select new parent: navigate tree above, {} to select",
                key(N::BoardPickerReparentEpic, "reparent_picker_confirm", "Enter")
            ),
            PURPLE,
        ),
        InputMode::ConfirmReparentEpic { .. } => hint_text(
            app,
            &format!("Reparent epic? {}", yes_no(N::BoardConfirmReparentEpic)),
            PURPLE,
        ),
        InputMode::MoveTaskToEpic(_) => hint(
            format!(
                "Select target epic: navigate tree above, {} to select",
                key(N::BoardPickerMoveToEpic, "move_to_epic_picker_confirm", "Enter")
            ),
            PURPLE,
        ),
        InputMode::ConfirmMoveTaskToEpic { .. } => hint_text(
            app,
            &format!("Move task to epic? {}", yes_no(N::BoardConfirmMoveToEpic)),
            PURPLE,
        ),
        InputMode::ConfirmTrustRepo { .. } | InputMode::ConfirmTrustRepoQuickDispatch { .. } => {
            let ns = if matches!(app.input.mode, InputMode::ConfirmTrustRepo { .. }) {
                N::BoardConfirmTrustRepo
            } else {
                N::BoardConfirmTrustRepoQuickDispatch
            };
            hint_text(
                app,
                &format!("Repo not trusted — trust it? {}", yes_no(ns)),
                YELLOW,
            )
        }
        InputMode::LinkWorkPackage { .. } => {
            let ns = N::BoardPickerWorkPackage;
            hint(
                format!(
                    "Link to work package: [{}] move  [{}] toggle  [{}] close",
                    hint_keys(
                        t,
                        ns,
                        &[
                            ("work_package_picker_down", "j"),
                            ("work_package_picker_up", "k")
                        ]
                    ),
                    hint_row_keys(t, ns, "work_package_picker_toggle", &["Space", "Enter"]),
                    key(ns, "work_package_picker_close", "Esc"),
                ),
                CYAN,
            )
        }
        InputMode::InputWorkPackageTitle(_) => hint(
            format!(
                "New work package: type a title  {}",
                text_field_hint(t, "create", "back")
            ),
            CYAN,
        ),
        InputMode::Admin => hint(
            format!(
                "Admin: [{}] pane  [{}] move  [{}] close  (keys for the pane are shown on the screen)",
                key(N::BoardAdmin, "admin_next_pane", "Tab"),
                hint_keys(
                    t,
                    N::BoardAdmin,
                    &[("admin_navigate_down", "j"), ("admin_navigate_up", "k")]
                ),
                key(N::BoardAdmin, "admin_close", "Esc"),
            ),
            CYAN,
        ),
        InputMode::InputAdminText => hint(format!("Admin: type a name  {}", text_keys("send")), CYAN),
        InputMode::ConfirmAdmin => hint(
            format!(
                "Admin: confirm with [{}], any other key cancels",
                key(N::BoardConfirmAdmin, "confirm_admin_yes", "y")
            ),
            YELLOW,
        ),
        InputMode::InputDisplayName => hint(
            format!("Display name: type a name  {}", text_keys("save")),
            CYAN,
        ),
        InputMode::RenameWorkPackage(_) => hint(
            format!("Rename work package: {}", text_keys("save")),
            CYAN,
        ),
        InputMode::ConfirmDeleteWorkPackage(id) => {
            // The status message auto-clears after STATUS_MESSAGE_TTL while
            // the mode lives on, so rebuild the titled prompt.
            let fallback = app.board.find_work_package(*id).map_or_else(
                || format!("Delete work package? Its tasks and epics are kept {}", yes_no(N::BoardConfirmDeleteWorkPackage)),
                |package| crate::tui::update::delete_work_package_prompt(t, package),
            );
            hint_text(app, &fallback, RED)
        }
    }
}

/// Normal-mode status bar: the base action hints (batch / epic / task) with the
/// active-mode badges (split, flat, active-filter, search) composed around
/// them.
fn normal_status_line(app: &App) -> (Line<'static>, Style) {
    let key_color = CYAN;
    let t = app.key_table;
    if matches!(app.board.view_mode, crate::tui::ViewMode::TaskDetail { .. }) {
        // The detail answers board.detail's keys, not the board's.
        let mut bar = HintBar::new(t, N::BoardDetail, key_color);
        bar.push_keys(
            &[("scroll_detail_down", "j"), ("scroll_detail_up", "k")],
            "scroll",
        );
        bar.push(None, "zoom_detail", "z", "zoom");
        bar.push(None, "close_detail", "q", "close");
        return (Line::from(bar.spans), Style::default());
    }
    let mut spans = if app.has_selection() {
        let count = app.selected_tasks().len() + app.selected_epics().len();
        let has_tasks = !app.selected_tasks().is_empty();
        batch_action_hints(t, count, key_color, has_tasks)
    } else {
        // One column build for the cursor's item, not one per question asked
        // of it.
        match app.selected_column_item() {
            Some(ColumnItem::Epic(epic)) => epic_action_hints(t, epic, key_color),
            Some(ColumnItem::WorkPackage(_)) => super::package_action_hints(t, key_color),
            item => {
                let task = match item {
                    Some(ColumnItem::Task(task)) => Some(task),
                    _ => None,
                };
                let now = chrono::Utc::now();
                let in_flight = task.is_some_and(|t| app.dispatch_may_be_in_flight(t, now));
                action_hints(
                    t,
                    app.activation_context(),
                    task,
                    in_flight,
                    app.cursor_in_flattened_column(),
                    key_color,
                )
            }
        }
    };
    if app.split_active() {
        let mut split = HintBar::new(t, N::BoardNormal, GREEN);
        split.push_styled(
            None,
            "toggle_split_mode",
            "s",
            "split",
            Style::default().fg(GREEN),
        );
        prepend(&mut spans, split.spans);
    }
    let (hidden_epics, hidden_packages) = app.live_hidden_counts();
    if let Some(badge) = hidden_badge(hidden_epics, hidden_packages) {
        prepend(
            &mut spans,
            vec![Span::styled(
                badge,
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            )],
        );
    }
    if app.filter.lens.flattened() {
        prepend(
            &mut spans,
            vec![Span::styled(
                "[flat] ",
                Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
            )],
        );
    }
    if app.filter.lens.only_active() {
        prepend(
            &mut spans,
            vec![Span::styled(
                "[active] ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            )],
        );
    }
    if app.view().search_active() {
        prepend(
            &mut spans,
            vec![Span::styled(
                format!("[/{}] ", app.search.query),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            )],
        );
    }
    (Line::from(spans), Style::default())
}

/// `[hidden: 2 epics, 1 package] `, or `None` when nothing is hidden. Keeps a
/// forgotten hide from reading as missing work (`board-layout.allium`, "Hidden
/// Epics and Work Packages").
fn hidden_badge(epics: usize, packages: usize) -> Option<String> {
    let count = |n: usize, one: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {one}s")),
    };
    let parts: Vec<String> = [count(epics, "epic"), count(packages, "package")]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| format!("[hidden: {}] ", parts.join(", ")))
}

/// Build status bar hints when tasks are batch-selected.
pub(in crate::tui) fn batch_action_hints(
    table: &'static [crate::keybindings::KeyBinding],
    count: usize,
    key_color: Color,
    has_tasks: bool,
) -> Vec<Span<'static>> {
    let count_style = Style::default().fg(YELLOW).add_modifier(Modifier::BOLD);

    let mut bar = HintBar::new(table, N::BoardNormal, key_color);
    bar.spans
        .push(Span::styled(format!("{count} selected  "), count_style));

    if has_tasks {
        bar.push(Some(C::OffEpicCard), "move_task_forward", "L", "move");
        bar.push(Some(C::OffEpicCard), "move_task_backward", "H", "back");
    }
    // 'x' completes tasks that aren't Done yet and deletes the rest, so with
    // tasks selected the label can't commit to one verb. An epics-only
    // selection always deletes.
    bar.push(
        None,
        "delete_task",
        "x",
        if has_tasks { "done/delete" } else { "delete" },
    );
    bar.push(None, "select_all", "a", "select all");
    bar.push(None, "cycle_lens_forward", "Tab", "lens");
    bar.push(None, "toggle_select", "v", "toggle");
    bar.push(Some(C::WithSelection), "clear_selection", "Esc", "clear");
    bar.spans
}
