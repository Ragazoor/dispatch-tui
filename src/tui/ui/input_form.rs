use crate::models::TaskId;
use crate::palette::{CYAN, MUTED, RED};
use crate::tui::{App, InputState};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};

/// Build the active input row for a single-line text field, drawing the caret
/// as a reversed block at `app.input.caret` and scrolling long values so the
/// caret stays visible. `prefix` includes the label and separator, e.g.
/// `"  Title: "`.
fn caret_field(prefix: &str, app: &App, area: Rect, active: Style) -> Line<'static> {
    let value_width = (area.width as usize)
        .saturating_sub(prefix.chars().count())
        .max(1);
    super::shared::caret_line(
        prefix.to_string(),
        &app.input.buffer,
        app.input.caret,
        value_width,
        active,
    )
}

/// A settled `label` + `value` line clipped to one terminal row (docs/specs/
/// tasks.allium: CreateTask, SettledLinesStayOneRow). `label` carries its own
/// indent and separator, e.g. `"  Title: "`. The label always survives; the
/// value is cut with a trailing "…" so a very long value (dictated text on a
/// single line) cannot wrap and push the picker below it off the panel.
fn settled_line(label: &str, value: &str, area: Rect, style: Style) -> Line<'static> {
    // The panel border takes two columns, matching `caret_field`'s budget.
    let width = (area.width as usize).saturating_sub(2);
    let value_width = width.saturating_sub(label.chars().count()).max(1);
    Line::from(Span::styled(
        format!("{label}{}", super::shared::truncate(value, value_width)),
        style,
    ))
}

/// The sizing and styling context every repo-path picker list shares.
///
/// `height_offset` is the number of rows the surrounding form already spends
/// above and below the list; `area_height` is the popup's total height. The
/// list gets whatever is left. Bundled into one struct because these three
/// always travel together and are constant for a given picker surface.
pub(in crate::tui::ui) struct RepoListCtx {
    pub height_offset: u16,
    pub area_height: u16,
    pub hint: Style,
}

impl RepoListCtx {
    /// Rows available to the list itself, never less than one.
    fn visible_rows(&self) -> usize {
        super::shared::visible_rows(self.area_height as usize, self.height_offset as usize)
    }
}

/// The already-answered form fields, rendered once for every step that shows
/// them back to the user. Owns the presentation fallbacks (`""` for a missing
/// title, `"none"` for a missing tag) so they cannot diverge per step.
struct DraftSummary {
    title: String,
    /// The settled tag, suffixed `" (phoenix)"` when the draft armed the flag.
    ///
    /// phoenix is answered at the tag step (CreateTask: PhoenixArming, in
    /// docs/specs/tasks.allium), so every step after it has the flag to show.
    /// It rides this one line rather than getting a line of its own so no
    /// step's height moves — `input_panel_height` (kanban/mod.rs) fixes those
    /// from literals.
    tag: String,
    /// The description's first line, suffixed `" ..."` when more lines follow.
    description_oneline: String,
}

impl DraftSummary {
    fn from_input(input: &InputState) -> Self {
        let draft = input.task_draft.as_ref();
        let title = draft.map(|d| d.title.clone()).unwrap_or_default();
        let tag_name = draft
            .and_then(|d| d.tag.as_ref())
            .map(|t| t.to_string())
            .unwrap_or_else(|| "none".to_string());
        let tag = if input.phoenix_armed() {
            format!("{tag_name} (phoenix)")
        } else {
            tag_name
        };
        let description = draft.map(|d| d.description.as_str()).unwrap_or("");
        let desc_first_line = description.lines().next().unwrap_or("").to_string();
        let description_oneline = if description.contains('\n') {
            format!("{desc_first_line} ...")
        } else {
            desc_first_line
        };
        Self {
            title,
            tag,
            description_oneline,
        }
    }
}

/// The three styles a form step draws with: settled fields above the cursor,
/// the active field, and trailing hint text. Bundled so the 11 step-renderer
/// functions below take one reference instead of transposable positional
/// `Style` params.
pub(in crate::tui::ui) struct FormStyles {
    pub completed: Style,
    pub active: Style,
    pub hint: Style,
}

/// Appends the filtered repo list and optional new-path entry to `lines`.
///
/// Shows existing paths that fuzzy-match `buffer`, then appends a selectable
/// new-path entry when `buffer` is non-empty and not an exact match for any
/// filtered item. This is the shared rendering contract for all
/// `RepoPathPicker` surfaces (InputRepoPath, QuickDispatch, InputBaseBranch).
fn append_filtered_repos_with_new_entry<'a>(
    lines: &mut Vec<Line<'a>>,
    filtered: &[String],
    buffer: &'a str,
    cursor: usize,
    ctx: &RepoListCtx,
) {
    let show_new = crate::tui::has_new_repo_option(buffer, filtered);
    let scroll_cursor = if show_new && !filtered.is_empty() && cursor == filtered.len() {
        filtered.len() - 1
    } else {
        cursor
    };
    if !filtered.is_empty() {
        append_repo_path_list(lines, filtered, scroll_cursor, ctx);
    }
    let hint = ctx.hint;
    if show_new {
        let cursor_style = Style::default().fg(CYAN).add_modifier(Modifier::BOLD);
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
pub(in crate::tui::ui) fn append_repo_path_list<'a>(
    lines: &mut Vec<Line<'a>>,
    repo_paths: &[String],
    cursor: usize,
    ctx: &RepoListCtx,
) {
    let hint = ctx.hint;
    let visible_repos = ctx.visible_rows();
    let scroll = super::shared::scroll_offset(cursor, repo_paths.len(), visible_repos);
    let cursor_style = Style::default().fg(CYAN).add_modifier(Modifier::BOLD);
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
    styles: &FormStyles,
) -> Vec<Line<'static>> {
    vec![
        caret_field("  Title: ", app, area, styles.active),
        Line::from(""),
        Line::from(Span::styled("  [Enter] confirm  [Esc] cancel", styles.hint)),
    ]
}

/// The tag keys both prompt variants below are built from. Every key sits
/// inside the label it selects (CreateTask: EveryKeyInItsName, in
/// docs/specs/tasks.allium), which is why PrReview reads `pr-re[v]iew` — `p`
/// belongs to `[p]hoenix`.
///
/// A macro rather than a `const` because `concat!` composes literals, not
/// const names, and composing is the point: the two variants differ only by the
/// `[p]hoenix` segment, so a tag renamed in one literal and not the other would
/// be the same drift `tag_prompt` exists to prevent, one level down. Adding a
/// tag now means editing this list and the handler's match, and both prompts
/// follow.
macro_rules! tag_keys {
    () => {
        "Tag: [b]ug  [f]eature  [c]hore  pr-re[v]iew  [r]esearch  fi[x]"
    };
}

/// The tag picker's prompt, first pass.
const TAG_PROMPT: &str = concat!(tag_keys!(), "  [p]hoenix  [Enter] none");

/// The same prompt on the second pass, after `p` armed the flag: `[p]hoenix` is
/// gone, because so is `p` from the accepted set (CreateTask: PhoenixArming).
/// The armed flag shows as a settled `Phoenix: yes` line above the picker
/// instead of being restated here.
const TAG_PROMPT_PHOENIX_ARMED: &str = concat!(tag_keys!(), "  [Enter] none");

/// The tag step's prompt, shared by all three surfaces that show it: the form
/// panel's active line (below), the status-bar hint (`kanban/status_bar.rs`)
/// and the transient status set on entering or re-entering the step
/// (`update/forms.rs`). One function because the three must agree, and the
/// accepted set now has two variants — a panel still offering `[p]hoenix`
/// under a status bar that has dropped it is a drift no test would catch if
/// each surface carried its own literal.
pub(in crate::tui) fn tag_prompt(phoenix_armed: bool) -> &'static str {
    if phoenix_armed {
        TAG_PROMPT_PHOENIX_ARMED
    } else {
        TAG_PROMPT
    }
}

/// The `Line` count [`input_tag_lines`] returns with phoenix armed: 2 settled
/// (title, phoenix) + active + blank + hint. Unarmed it returns one fewer.
///
/// `input_panel_height` (kanban/mod.rs) reserves rows without building the real
/// `Vec` on its per-frame path, so this constant is what ties the two modules
/// together — `input_tag_lines_returns_a_fixed_line_count_per_variant` below is
/// what catches it drifting from what the render actually produces.
pub(in crate::tui) const PHOENIX_ARMED_TAG_STEP_LINES: u16 = 5;

/// The tag picker, and the step where phoenix is armed.
///
/// Once armed, a settled `Phoenix: yes` line sits above the active picker line
/// and the prompt loses its `[p]hoenix` key — see
/// [`PHOENIX_ARMED_TAG_STEP_LINES`]. The step's reserved rows exceed its line
/// count either way, and that slack is not spare: the panel wraps
/// (`Wrap { trim: false }`), so a prompt too long for the terminal costs more
/// than one row, and the slack is what keeps the trailing `[Esc] cancel` hint
/// inside the border. Both variants survive down to roughly 40 columns, well
/// past the width a four-column board is legible at.
pub(in crate::tui) fn input_tag_lines(
    app: &App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'static>> {
    // Deliberately not a `DraftSummary`: this step renders the tag as a
    // *prompt*, never as a settled value, so the summary's tag and description
    // strings would be built and dropped every frame — and its phoenix suffix
    // would be formatted onto that dead path on exactly the second pass this
    // step now has.
    let draft = app.input.task_draft.as_ref();
    let title = draft.map(|d| d.title.as_str()).unwrap_or("");
    let phoenix_armed = app.input.phoenix_armed();
    let mut lines = Vec::with_capacity(PHOENIX_ARMED_TAG_STEP_LINES as usize);
    lines.push(settled_line("  Title: ", title, area, styles.completed));
    if phoenix_armed {
        lines.push(Line::from(Span::styled("  Phoenix: yes", styles.completed)));
    }
    lines.push(Line::from(Span::styled(
        format!("  {}", tag_prompt(phoenix_armed)),
        styles.active,
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Enter] skip  [Esc] cancel",
        styles.hint,
    )));
    lines
}

pub(in crate::tui) fn input_description_lines(
    app: &App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'static>> {
    let summary = DraftSummary::from_input(&app.input);
    vec![
        settled_line("  Title: ", &summary.title, area, styles.completed),
        settled_line("  Tag: ", &summary.tag, area, styles.completed),
        Line::from(Span::styled(
            "  Description: opening $EDITOR...".to_string(),
            styles.active,
        )),
        Line::from(""),
        Line::from(Span::styled("  [Esc] cancel", styles.hint)),
    ]
}

/// The hint every `RepoPathPicker` surface closes with. One literal rather than
/// three, so the pickers cannot drift apart on a 65-character string of glyphs.
const REPO_PICKER_HINT: &str = "  Type to filter · [↑/↓] navigate · [Enter] select · [Esc] cancel";

/// Append a picker's candidate list and its closing hint — the tail shared by
/// every `RepoPathPicker` surface (docs/specs/dispatch.allium: RepoPathPicker).
///
/// `height_offset` is the count of non-candidate rows the mode's own form
/// already occupies, and is the only thing that differs between the three
/// callers. `filtered` is passed in rather than derived here because the
/// candidate source differs too: the global repo-path set for two of them, a
/// per-repo branch history for `input_base_branch_lines`.
fn append_picker_candidates<'a>(
    lines: &mut Vec<Line<'a>>,
    filtered: &[String],
    app: &'a App,
    area: Rect,
    styles: &FormStyles,
    height_offset: u16,
) {
    append_filtered_repos_with_new_entry(
        lines,
        filtered,
        &app.input.buffer,
        app.input.repo_cursor,
        &RepoListCtx {
            height_offset,
            area_height: area.height,
            hint: styles.hint,
        },
    );
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(REPO_PICKER_HINT, styles.hint)));
}

pub(in crate::tui) fn input_repo_path_lines<'a>(
    app: &'a App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'a>> {
    let summary = DraftSummary::from_input(&app.input);
    let mut lines = vec![
        settled_line("  Title: ", &summary.title, area, styles.completed),
        settled_line("  Tag: ", &summary.tag, area, styles.completed),
        settled_line(
            "  Description: ",
            &summary.description_oneline,
            area,
            styles.completed,
        ),
        caret_field("  Repo path: ", app, area, styles.active),
    ];
    let filtered = crate::tui::filtered_repos(&app.board.repo_paths, &app.input.buffer);
    append_picker_candidates(&mut lines, &filtered, app, area, styles, 7);
    lines
}

pub(in crate::tui) fn input_base_branch_lines<'a>(
    app: &'a App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'a>> {
    let summary = DraftSummary::from_input(&app.input);
    let repo_path = app
        .input
        .task_draft
        .as_ref()
        .map(|d| d.repo_path.clone())
        .unwrap_or_default();
    let mut lines = vec![
        settled_line("  Title: ", &summary.title, area, styles.completed),
        settled_line("  Tag: ", &summary.tag, area, styles.completed),
        settled_line(
            "  Description: ",
            &summary.description_oneline,
            area,
            styles.completed,
        ),
        settled_line("  Repo path: ", &repo_path, area, styles.completed),
        caret_field("  Base branch: ", app, area, styles.active),
    ];
    let history = app.base_branches_for(&repo_path);
    let filtered = crate::tui::filtered_repos(history, &app.input.buffer);
    append_picker_candidates(&mut lines, &filtered, app, area, styles, 6);
    lines
}

/// The four steps answered before the wrap-up picker, restated above it.
///
/// Still split out rather than inlined into its one caller: it is the shape a
/// step added after wrap-up would build on, the same way the (now removed)
/// standalone phoenix step did. The sibling step renderers above build their
/// settled lines inline instead — none of them has a step after it that needs
/// to restate their answers.
fn answered_step_lines(app: &App, area: Rect, completed: Style) -> Vec<Line<'static>> {
    let summary = DraftSummary::from_input(&app.input);
    let draft = app.input.task_draft.as_ref();
    let repo_path = draft.map(|d| d.repo_path.clone()).unwrap_or_default();
    let base_branch = draft
        .map(|d| d.base_branch.clone())
        .unwrap_or_else(|| "main".to_string());
    vec![
        settled_line("  Title: ", &summary.title, area, completed),
        settled_line("  Tag: ", &summary.tag, area, completed),
        settled_line("  Repo: ", &repo_path, area, completed),
        settled_line("  Base branch: ", &base_branch, area, completed),
    ]
}

/// Close off a creation-form step page: the active line, a blank, the Esc hint.
///
/// A tail step is settled summaries, one active line, then this footer.
/// `active` is the part that differs per step — a styled span for a
/// single-key picker, a [`caret_field`] for a free-text one. The wrap-up
/// picker, the form's last step, uses it today; it is the shape a step added
/// after it would take.
fn form_step_page<'a>(mut settled: Vec<Line<'a>>, active: Line<'a>, hint: Style) -> Vec<Line<'a>> {
    settled.push(active);
    settled.push(Line::from(""));
    settled.push(Line::from(Span::styled("  [Esc] cancel", hint)));
    settled
}

/// The form's last step (see docs/specs/tasks.allium's CreateTask guidance):
/// restates the four answered steps — with phoenix riding the settled tag line
/// when it was armed at InputTag — then the wrap-up picker. Answering it
/// creates the task.
pub(in crate::tui) fn input_wrap_up_mode_lines(
    app: &App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'static>> {
    form_step_page(
        answered_step_lines(app, area, styles.completed),
        Line::from(Span::styled(
            "  Wrap-up: [r]ebase  [p]r  [d]one  [Enter] skip",
            styles.active,
        )),
        styles.hint,
    )
}

pub(in crate::tui) fn quick_dispatch_lines<'a>(
    app: &'a App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'a>> {
    let filtered = crate::tui::filtered_repos(&app.board.repo_paths, &app.input.buffer);
    let mut lines = vec![
        Line::from(Span::styled(
            "  Quick Dispatch — select repo:",
            styles.active,
        )),
        Line::from(""),
        caret_field("  Filter: ", app, area, styles.active),
    ];
    append_picker_candidates(&mut lines, &filtered, app, area, styles, 7);
    lines
}

pub(in crate::tui) fn confirm_retry_lines(app: &App, id: TaskId) -> Vec<Line<'static>> {
    let label = if app.is_crashed(id) {
        "crashed"
    } else {
        "stale"
    };
    let warning = Style::default().fg(RED).add_modifier(Modifier::BOLD);
    let hint = Style::default().fg(MUTED);
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
    styles: &FormStyles,
) -> Vec<Line<'static>> {
    vec![
        caret_field("  Title: ", app, area, styles.active),
        Line::from(""),
        Line::from(Span::styled("  [Enter] confirm  [Esc] cancel", styles.hint)),
    ]
}

pub(in crate::tui) fn input_epic_description_lines(
    app: &App,
    area: Rect,
    styles: &FormStyles,
) -> Vec<Line<'static>> {
    let title = app
        .input
        .epic_draft
        .as_ref()
        .map(|d| d.title.as_str())
        .unwrap_or("");
    vec![
        settled_line("  Title: ", title, area, styles.completed),
        Line::from(Span::styled(
            "  Description: opening $EDITOR...".to_string(),
            styles.active,
        )),
        Line::from(""),
        Line::from(Span::styled("  [Esc] cancel", styles.hint)),
    ]
}

#[cfg(test)]
mod tests;
