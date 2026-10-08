//! The `*State` structs `App` is made of, and the view-mode and selection
//! types they hold.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ratatui::widgets::ListState;

use super::{ColumnAnchor, EpicDraft, InputMode, RepoFilterMode, TaskDraft};
use crate::models::{Epic, EpicId, Task, TaskId, TaskStatus};

// BoardState — tasks, epics, view mode, and related board data
// ---------------------------------------------------------------------------

pub struct BoardState {
    pub(in crate::tui) tasks: Vec<Task>,
    pub(in crate::tui) epics: Vec<Epic>,
    pub(in crate::tui) view_mode: ViewMode,
    pub(in crate::tui) repo_paths: Vec<String>,
    /// Per-repo most-recently-used base_branch history, keyed by repo_path,
    /// each list ordered most-recent-first (see docs/specs/dispatch.allium:
    /// surface BaseBranchPicker).
    pub(in crate::tui) repo_base_branches: std::collections::HashMap<String, Vec<String>>,
    pub(in crate::tui) split: SplitState,
    /// Flattened rendering mode: when true, epic cards are hidden and every
    /// descendant task of the current view surfaces directly in its status
    /// column. Preserved across navigation, session-scoped.
    pub(in crate::tui) flattened: bool,
}

// ---------------------------------------------------------------------------
// StatusState — transient status messages and error popups
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct StatusState {
    pub(in crate::tui) message: Option<String>,
    pub(in crate::tui) message_set_at: Option<Instant>,
    pub(in crate::tui) error_popup: Option<String>,
    /// When true, the status message survives the [`STATUS_MESSAGE_TTL`]
    /// auto-clear in `handle_tick`. Used for in-flight dispatch feedback —
    /// the message must persist for the multi-second `git fetch` window
    /// rather than vanish mid-flight.
    pub(in crate::tui) message_sticky: bool,
}

impl StatusState {
    /// Set a transient status message with auto-clear timestamp.
    pub(in crate::tui) fn set(&mut self, msg: String) {
        self.message = Some(msg);
        self.message_set_at = Some(Instant::now());
        self.message_sticky = false;
    }

    /// Set a sticky status message that bypasses the 5-second TTL.
    /// The message persists until `clear` is called explicitly.
    pub(in crate::tui) fn set_sticky(&mut self, msg: String) {
        self.message = Some(msg);
        self.message_set_at = Some(Instant::now());
        self.message_sticky = true;
    }

    /// Clear the status message and its timestamp.
    pub(in crate::tui) fn clear(&mut self) {
        self.message = None;
        self.message_set_at = None;
        self.message_sticky = false;
    }
}

// ---------------------------------------------------------------------------
// AgentTracking — agent health state for dispatched agents
// ---------------------------------------------------------------------------

/// Per-agent health tracking for dispatched agents. Stale detection is derived
/// from `task.last_pre_tool_use_at` by `ClassifyAgentActivity` on each tick;
/// this struct retains state the classifier cannot reconstruct from the
/// database — notification de-duplication, PR poll cadence, and message-flash
/// decay.
#[derive(Debug, Default)]
pub struct AgentTracking {
    pub notified_review: HashSet<TaskId>,
    pub notified_needs_input: HashSet<TaskId>,
    pub last_pr_poll: HashMap<TaskId, Instant>,
    /// Per-task PR-poll failure bookkeeping. Absent until a task's first
    /// failure; see [`PrPollState`].
    pub pr_poll: HashMap<TaskId, PrPollState>,
    /// A task that just *received* a native peer message — envelope glyph.
    pub message_flash: HashMap<TaskId, Instant>,
    /// A task that just *sent* one — its own glyph, same TTL and fill as
    /// [`Self::message_flash`]. See `docs/specs/board-visuals.allium`'s "Message
    /// flash".
    pub message_flash_sent: HashMap<TaskId, Instant>,
    /// Subtasks whose epic auto-dispatch chain claimed them and then failed to
    /// provision them, mapped to why (`AutoDispatchFailureIndicator` in
    /// docs/specs/epics.allium). Unlike every other entry here this one carries
    /// no timestamp: a stalled chain stays stalled until a human acts, so the
    /// marker decays on re-dispatch rather than on a clock.
    pub auto_dispatch_failed: HashMap<TaskId, String>,
}

impl AgentTracking {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove all tracking state for a task.
    pub fn clear(&mut self, id: TaskId) {
        self.notified_review.remove(&id);
        self.notified_needs_input.remove(&id);
        self.last_pr_poll.remove(&id);
        self.pr_poll.remove(&id);
        self.message_flash.remove(&id);
        self.message_flash_sent.remove(&id);
        self.auto_dispatch_failed.remove(&id);
    }
}

/// Per-task PR-poll bookkeeping: how many times in a row reading this task's PR
/// failed permanently, when the next attempt is allowed, and whether polling has
/// given up altogether.
///
/// Session-scoped and never persisted, which is deliberate rather than an
/// oversight. `SubStatus::PrUnreachable` is the durable, user-visible shadow of
/// `gave_up`; this struct is the mechanism. Because it dies with the process, a
/// restart polls a `pr_unreachable` task once more — and that is the only
/// recovery path the user has for the failure that motivated it, since "no
/// account has access" is fixed on GitHub, not on the task. See `PrPollState`
/// in `docs/specs/core.allium`.
#[derive(Debug, Default, Clone)]
pub struct PrPollState {
    /// Reset to zero by any successful poll, and deliberately untouched by
    /// transient failures — a long GitHub outage must not strand the board.
    pub consecutive_permanent_failures: u32,
    /// Consecutive transient failures, counted separately so the backoff can
    /// widen without ever advancing the give-up counter above.
    pub consecutive_transient_failures: u32,
    /// Transient-failure backoff deadline. `None` means "eligible on the next
    /// tick that clears `PR_POLL_INTERVAL`", the unthrottled default.
    pub next_poll_at: Option<Instant>,
}

impl PrPollState {
    /// Whether polling has stopped for this task for the rest of the session.
    ///
    /// Derived rather than stored: it is always exactly
    /// `consecutive_permanent_failures >= PR_POLL_PERMANENT_FAILURE_THRESHOLD`,
    /// the only two places that touch either one (`clear_pr_poll_failures` and
    /// the permanent-failure branch of `handle_pr_check_failed`) already keep
    /// them in lockstep by hand. A stored bool alongside the counter it
    /// mirrors can only ever drift from it or duplicate it — and did let a
    /// test construct a `PrPollState` with `gave_up: true` but a zero counter,
    /// a state that should be unrepresentable.
    pub fn gave_up(&self) -> bool {
        self.consecutive_permanent_failures >= crate::tui::PR_POLL_PERMANENT_FAILURE_THRESHOLD
    }
}

// ---------------------------------------------------------------------------
// InputState — current input mode and draft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct InputState {
    pub mode: InputMode,
    pub buffer: String,
    /// Text caret: a **character** index into `buffer` (count of chars left of
    /// the caret), invariant `0..=buffer.chars().count()`. All edits go through
    /// `crate::tui::text_caret` and all buffer writes through
    /// [`InputState::set_buffer`] / [`InputState::clear_buffer`] so the caret
    /// never drifts out of range or onto a non-char boundary.
    pub caret: usize,
    pub task_draft: Option<TaskDraft>,
    pub epic_draft: Option<EpicDraft>,
    pub repo_cursor: usize,
    /// Tracks epic_id during quick-dispatch repo selection in epic view.
    pub pending_epic_id: Option<EpicId>,
    /// True while the draft came from `CopyTask` rather than the new-task form.
    ///
    /// Both flows run InputTag, but they leave it for different steps: a new
    /// task goes on to InputDescription, a copy straight to InputRepoPath
    /// (its description is copied outright). See CopyTask in
    /// `docs/specs/tasks.allium`.
    ///
    /// Set by `handle_copy_task` and consumed — `mem::take` — by the one
    /// handler that reads it, so it cannot outlive the draft it describes. A
    /// stale `true` would silently skip the description editor on the next new
    /// task, which is a failure with no error to notice.
    pub copy_flow: bool,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mode: InputMode::Normal,
            buffer: String::new(),
            caret: 0,
            task_draft: None,
            epic_draft: None,
            repo_cursor: 0,
            pending_epic_id: None,
            copy_flow: false,
        }
    }
}

impl InputState {
    /// Replace the buffer and land the caret at the end (natural for editing an
    /// existing value, e.g. a copied repo path or the default base branch).
    pub fn set_buffer(&mut self, s: String) {
        self.buffer = s;
        self.caret = self.buffer.chars().count();
    }

    /// Clear the buffer and reset the caret to the start.
    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.caret = 0;
    }

    /// Reset the picker's list cursor when the query is about to change.
    fn reset_picker_cursor(&mut self) {
        if self.mode.is_repo_picker() {
            self.repo_cursor = 0;
        }
    }

    /// Type `c` at the caret. In a repo picker every printable character
    /// filters (RepoPathPicker.NoPrintableShortcut) and the list cursor
    /// returns to 0.
    pub(in crate::tui) fn insert_char(&mut self, c: char) {
        self.reset_picker_cursor();
        self.caret = crate::tui::text_caret::insert(&mut self.buffer, self.caret, c);
    }

    pub(in crate::tui) fn backspace(&mut self) {
        self.reset_picker_cursor();
        self.caret = crate::tui::text_caret::delete_before(&mut self.buffer, self.caret);
    }

    pub(in crate::tui) fn delete_forward(&mut self) {
        self.reset_picker_cursor();
        self.caret = crate::tui::text_caret::delete_after(&mut self.buffer, self.caret);
    }

    pub(in crate::tui) fn cursor_left(&mut self) {
        self.caret = crate::tui::text_caret::move_left(self.caret);
    }

    pub(in crate::tui) fn cursor_right(&mut self) {
        self.caret = crate::tui::text_caret::move_right(&self.buffer, self.caret);
    }

    pub(in crate::tui) fn cursor_word_left(&mut self) {
        self.caret = crate::tui::text_caret::word_left(&self.buffer, self.caret);
    }

    pub(in crate::tui) fn cursor_word_right(&mut self) {
        self.caret = crate::tui::text_caret::word_right(&self.buffer, self.caret);
    }

    pub(in crate::tui) fn cursor_home(&mut self) {
        self.caret = crate::tui::text_caret::home();
    }

    pub(in crate::tui) fn cursor_end(&mut self) {
        self.caret = crate::tui::text_caret::end(&self.buffer);
    }

    /// Whether the in-flight draft has armed the phoenix flag.
    ///
    /// One accessor because four surfaces must agree on this bit — the tag
    /// picker's accepted key set, its two prompt surfaces, and the panel rows
    /// reserved for the step. A panel offering `[p]hoenix` under a status bar
    /// that has dropped it is exactly the drift `ui::tag_prompt` exists to
    /// prevent, and it would come straight back if each surface derived the
    /// selector itself. The flag lives on the draft (it has to reach
    /// creation); this only single-sources the projection.
    pub fn phoenix_armed(&self) -> bool {
        self.task_draft.as_ref().is_some_and(|d| d.phoenix)
    }
}

// ---------------------------------------------------------------------------
// SplitState — tmux split mode state
// ---------------------------------------------------------------------------

/// How a rearrangement ended, as its own producer reports it.
///
/// Carries the tag and the outcome together, so the two cannot disagree: an
/// entry's report can only describe an entry. `Some` means tmux produced a
/// pane; `None` is a failure, which settles the rearrangement all the same —
/// one left in flight would make `[s]` (or swapping) dead for the rest of the
/// session.
///
/// A swap's task is not optional, unlike an entry's: entry can open a bare,
/// unpinned shell pane, whereas a swap always names the task it swapped in.
///
/// See `SplitPaneEntrySettles` and `SplitPaneSwapSettles` in
/// `docs/specs/split-pane.allium`, which the spec has always modelled as two
/// events.
#[derive(Debug)]
pub(in crate::tui) enum Settled {
    Entry(Option<(String, Option<TaskId>)>),
    Swap(Option<(String, TaskId)>),
}

/// Which rearrangement is in flight, and what only that one can hold.
///
/// The spec's `kind` discriminator (`core.allium`: `Rearrangement`, with its
/// `EntryInFlight` and `SwapInFlight` variants).
#[derive(Debug)]
pub(in crate::tui) enum Rearrangement {
    /// Split-mode entry: the gap between asking for a pane and having one.
    /// Runs while `active` is false, which is what distinguishes it from a
    /// swap.
    Entry,
    /// Exchanging which task's tmux window the existing pane shows. Requires
    /// `active`.
    Swap {
        /// The swap requested while this one was in flight, held until it
        /// settles. At most one: a further request replaces it, because each
        /// is the same instruction and only the newest reflects what the user
        /// wants.
        ///
        /// On this variant rather than beside the holds below because a swap
        /// request can only arrive while split mode is active, and an entry
        /// runs while it is not — so a swap held during an entry is a state
        /// the rules never produce. See `DeferSwapWhileSwapInFlight` in
        /// `docs/specs/split-pane.allium`.
        pending_swap: Option<TaskId>,
    },
}

/// A tmux rearrangement that has been started and has not yet settled, and the
/// requests held for it.
///
/// While this is `Some`, `SplitState`'s `active`, `right_pane_id` and
/// `pinned_task_id` describe the state BEFORE the rearrangement, so nothing new
/// may be started against them and anything the user asks for meanwhile is held
/// here instead. The settle ends the rearrangement by dropping this whole
/// value, which is what makes "each hold is acted on at most once" structural
/// rather than a list of clearing statements a later field could be left out
/// of.
///
/// Both holds sit here rather than on either variant: each is held for
/// whichever rearrangement is in flight, and both settles replay them the same
/// way. Tying either to the entry would drop it on the swap path, and a dropped
/// quit is one the user cannot notice, having already asked the application to
/// close. See `Rearrangement` in `docs/specs/core.allium`.
#[derive(Debug)]
pub struct InFlight {
    pub(in crate::tui) kind: Rearrangement,
    /// A toggle press that arrived while this rearrangement was in flight,
    /// replayed by the settle as an exit.
    ///
    /// Held, not queued: a further press replaces it rather than accumulating,
    /// so any burst during one rearrangement settles as a single held toggle.
    pub(in crate::tui) pending_toggle: bool,
    /// A confirmed quit that arrived while this rearrangement was in flight.
    ///
    /// Quitting mid-entry would issue no restore at all; quitting mid-swap
    /// would tear the process down between the pane exchange and the rename,
    /// leaving two windows sharing a name. See
    /// `HoldQuitWhileRearrangementInFlight` in `docs/specs/split-pane.allium`.
    pub(in crate::tui) pending_quit: bool,
}

impl InFlight {
    /// A rearrangement just started, holding nothing yet.
    fn new(kind: Rearrangement) -> Self {
        Self {
            kind,
            pending_toggle: false,
            pending_quit: false,
        }
    }

    /// A split-mode entry, holding nothing yet.
    pub(in crate::tui) fn entry() -> Self {
        Self::new(Rearrangement::Entry)
    }

    /// A swap, holding nothing yet.
    pub(in crate::tui) fn swap() -> Self {
        Self::new(Rearrangement::Swap { pending_swap: None })
    }

    /// Whether `settled` is this rearrangement's own report.
    ///
    /// An entry and a swap never overlap, so a report that does not match is
    /// unreachable; this is what makes the guard an assertion rather than the
    /// dispatch mechanism. See `HoldToggleWhileRearrangementInFlight` in
    /// `docs/specs/split-pane.allium`.
    pub(in crate::tui) fn settles(&self, settled: &Settled) -> bool {
        matches!(
            (&self.kind, settled),
            (Rearrangement::Entry, Settled::Entry(_))
                | (Rearrangement::Swap { .. }, Settled::Swap(_))
        )
    }

    /// What this rearrangement forgets when the split pane closes.
    ///
    /// A close says a pane went away, not that the tmux work already under way
    /// will not report back — so the rearrangement itself survives, holds and
    /// all. A held *swap* does not: it names an occupant the user asked to see,
    /// and there is no occupant. The rule lives here, beside the fields it
    /// governs, so a variant added later meets it rather than defaulting
    /// silently to "survives". See `SplitPaneClosedResets` in
    /// `docs/specs/split-pane.allium`.
    pub(in crate::tui) fn pane_closed(&mut self) {
        if let Rearrangement::Swap { pending_swap } = &mut self.kind {
            *pending_swap = None;
        }
    }
}

/// Split-pane mode's whole state: what the pane shows, and what tmux work is
/// outstanding against it.
#[derive(Debug)]
pub struct SplitState {
    pub(in crate::tui) active: bool,
    pub(in crate::tui) focused: bool,
    pub(in crate::tui) right_pane_id: Option<String>,
    pub(in crate::tui) pinned_task_id: Option<TaskId>,
    /// The rearrangement in flight, if any.
    ///
    /// `None` means the pane is at rest: the four fields above describe what is
    /// actually there, and a fresh `[s]` press or swap request may start work
    /// against them. `Some` means they describe the previous state, nothing new
    /// may be started, and the requests that arrived meanwhile are held inside.
    ///
    /// Survives a pane close, unlike the rest of this state — a close says a
    /// pane went away, not that the tmux work already under way will not report
    /// back, and it is that report which ends the rearrangement and performs
    /// anything held for it. A held *swap* is the one part a close does clear,
    /// because it names an occupant rather than work in flight. See
    /// `SplitPaneClosedResets` in `docs/specs/split-pane.allium`.
    pub(in crate::tui) in_flight: Option<InFlight>,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            active: false,
            focused: true,
            right_pane_id: None,
            pinned_task_id: None,
            in_flight: None,
        }
    }
}

// ---------------------------------------------------------------------------
// SelectionState — multi-select state for batch operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    pub tasks: HashSet<TaskId>,
    pub epics: HashSet<EpicId>,
    pub pending_done: Vec<TaskId>,
}

impl SelectionState {
    pub fn has_selection(&self) -> bool {
        !self.tasks.is_empty() || !self.epics.is_empty()
    }

    pub fn clear(&mut self) {
        self.tasks.clear();
        self.epics.clear();
    }
}

// ---------------------------------------------------------------------------
// FilterState — repo filter for the task board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FilterState {
    pub repos: HashSet<String>,
    pub mode: RepoFilterMode,
    pub only_active: bool,
}

impl FilterState {
    pub fn matches(&self, repo_path: &str) -> bool {
        if self.repos.is_empty() {
            return true;
        }
        match self.mode {
            RepoFilterMode::Include => self.repos.contains(repo_path),
            RepoFilterMode::Exclude => !self.repos.contains(repo_path),
        }
    }

    /// Returns false when `only_active` is set and the task has no tmux window.
    pub fn task_matches(&self, task: &crate::models::Task) -> bool {
        !self.only_active || task.tmux_window.is_some()
    }
}

// ---------------------------------------------------------------------------
// SearchState — live title/id search over the task board
// ---------------------------------------------------------------------------

/// Task-search state: the query matches a task title (fuzzy subsequence) or a
/// task id (digit prefix, optional leading `#`). `query` empty = no filtering.
/// `saved` holds the query to restore if the user cancels the search bar with Esc.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub(in crate::tui) saved: Option<String>,
}

// ---------------------------------------------------------------------------
// BoardSelection — column + row selection state for a kanban view
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BoardSelection {
    pub(in crate::tui) selected_column: usize,
    pub(in crate::tui) selected_row: [usize; TaskStatus::COLUMN_COUNT],
    pub(in crate::tui) on_select_all: bool,
    pub(in crate::tui) list_states: [ListState; TaskStatus::COLUMN_COUNT],
    pub(in crate::tui) anchor: Option<ColumnAnchor>,
}

impl BoardSelection {
    pub fn new() -> Self {
        Self {
            selected_column: 1,
            selected_row: [0; TaskStatus::COLUMN_COUNT],
            on_select_all: false,
            list_states: std::array::from_fn(|_| ListState::default()),
            anchor: None,
        }
    }

    pub fn new_for_board() -> Self {
        Self::new()
    }

    pub fn new_for_epic() -> Self {
        Self::new()
    }

    pub fn column(&self) -> usize {
        self.selected_column
    }

    /// Row cursor for the given navigation column (1–4; board-layout.allium).
    pub fn row(&self, col: usize) -> usize {
        match col {
            1..=4 => self.selected_row[col - 1],
            _ => 0,
        }
    }

    pub fn set_column(&mut self, col: usize) {
        self.selected_column = col;
    }

    pub fn set_row(&mut self, col: usize, row: usize) {
        if let 1..=4 = col {
            self.selected_row[col - 1] = row;
        }
    }

    /// Reset the cursor for `col` to the top row, clear the select-all
    /// toggle, and (for task columns) scroll that column's list back to the
    /// top. Used when the cursor enters a column and should never land on a
    /// row remembered from a prior visit.
    pub fn reset_to_top(&mut self, col: usize) {
        self.set_row(col, 0);
        self.on_select_all = false;
        if let 1..=4 = col {
            *self.list_states[col - 1].offset_mut() = 0;
        }
    }
}

impl Default for BoardSelection {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ViewMode — board vs epic view with preserved selection state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ViewMode {
    Board(BoardSelection),
    Epic {
        epic_id: EpicId,
        selection: BoardSelection,
        /// The view to restore when exiting this epic.
        /// For a root epic entered from the board, this is `ViewMode::Board(...)`.
        /// For a nested sub-epic, this is `ViewMode::Epic { ... }` of the parent.
        parent: Box<ViewMode>,
    },
    TaskDetail {
        task_id: TaskId,
        scroll: u16,
        zoomed: bool,
        /// Scroll limit — updated by the renderer each frame from the actual wrapped line count.
        /// Do not treat this as authoritative input state; it is renderer-managed.
        max_scroll: u16,
        previous: Box<ViewMode>,
    },
}

impl ViewMode {
    pub(in crate::tui) fn selection(&self) -> &BoardSelection {
        match self {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
            ViewMode::TaskDetail { previous, .. } => previous.selection(),
        }
    }

    pub(in crate::tui) fn selection_mut(&mut self) -> &mut BoardSelection {
        match self {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
            ViewMode::TaskDetail { previous, .. } => previous.selection_mut(),
        }
    }
}

impl Default for ViewMode {
    fn default() -> Self {
        ViewMode::Board(BoardSelection::new_for_board())
    }
}

// ---------------------------------------------------------------------------
// BoardViewMode — the board-column-relevant subset of ViewMode
// ---------------------------------------------------------------------------

/// `ViewMode` narrowed to the two variants that carry board-column layout:
/// `Board` and `Epic`. Returned by `App::effective_view_mode()`, which peels
/// away the `TaskDetail` overlay variant. Column-builder
/// callers match exhaustively on this with no `unreachable!` fallback.
pub(in crate::tui) enum BoardViewMode<'a> {
    Board(&'a BoardSelection),
    Epic {
        epic_id: EpicId,
        selection: &'a BoardSelection,
    },
}

// ---------------------------------------------------------------------------
