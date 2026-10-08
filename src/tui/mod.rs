mod columns;
pub mod commands;
mod dispatcher;
pub mod input;
pub mod messages;
mod search;
mod selection;
pub mod text_caret;
pub mod types;
pub mod ui;
pub mod update;

pub(in crate::tui) use search::*;
pub use types::*;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

#[cfg(test)]
use crate::models::ReviewDecision;
use crate::models::{
    section_sort_priority, ColumnSection, Epic, EpicId, SubStatus, Task, TaskId, TaskStatus,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How long a transient status message stays visible before auto-clearing.
pub(in crate::tui) const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

/// Maximum gap between two `g` presses for them to count as the `gg` chord
/// (jump to top of column). A single `g` outside this window falls back to
/// its normal action (jump to tmux window / enter epic).
pub(in crate::tui) const GG_CHORD_TIMEOUT: Duration = Duration::from_millis(150);

/// Interval between PR status polls for tasks in review.
pub(in crate::tui) const PR_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Consecutive permanent PR-read failures before polling gives up on a task and
/// marks it `pr_unreachable`.
///
/// Above one deliberately: a GitHub incident can briefly report a real
/// repository as unresolvable, and a single blip must not strand a task
/// (`core.allium`: `pr_poll_permanent_failure_threshold`).
pub(crate) const PR_POLL_PERMANENT_FAILURE_THRESHOLD: u32 = 3;

/// Ceiling on the transient-failure backoff. A PR whose reads keep failing for
/// a retryable reason is polled at a widening interval up to this cap, rather
/// than every `PR_POLL_INTERVAL` forever (`core.allium`:
/// `pr_poll_backoff_max`).
pub(in crate::tui) const PR_POLL_BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

/// How long a delivered message keeps flashing its task's card — warm fill,
/// envelope glyph, and a frame in the column's identity colour.
///
/// Long enough that a human whose attention is elsewhere still notices it; the
/// superseded 3-second window did not clear that bar (`board-visuals.allium`: "Message
/// flash").
///
/// This is the *single* home for the duration. It is read by both
/// `App::tick_message_flash`, which sweeps expired entries, and the card
/// renderer, which decides whether to draw the flash. Those two used to carry
/// the number independently and could silently disagree — leaving an entry in
/// the map that the card no longer drew, or the reverse.
pub(in crate::tui) const MESSAGE_FLASH_TTL: Duration = Duration::from_secs(30);

/// Number of ticks between budget-snapshot reads. At `TICK_INTERVAL` (2s) this
/// is 10s — mirrors config.budget_poll_interval (see docs/specs/core.allium
/// config and dispatch.allium: TokenBudgetIndicator).
pub(in crate::tui) const BUDGET_POLL_TICKS: u64 = 5;

/// Age after which the budget indicator dims and shows its age. Mirrors
/// config.budget_stale_after.
pub(in crate::tui) const BUDGET_STALE_AFTER: Duration = Duration::from_secs(600);

/// Whether the stale-learning cleanup background job runs.
/// Mirrors config.stale_learning_cleanup_enabled (see docs/specs/core.allium config).
pub(crate) const STALE_LEARNING_CLEANUP_ENABLED: bool = true;

/// Age after which an approved, non-positively-scored learning (upvote_count <= 0)
/// becomes eligible for auto-archival. Mirrors config.stale_learning_threshold
/// (90 days; see docs/specs/core.allium config and learnings.allium: ArchiveStaleLearning).
pub(crate) const STALE_LEARNING_THRESHOLD: Duration = Duration::from_secs(90 * 24 * 60 * 60);

/// Minimum wall-clock spacing between stale-learning cleanup sweeps, so the sweep
/// does not run on every 2s tick. Not a spec config value — an implementation-level
/// cadence for the tick-driven job (see learnings.allium: ArchiveStaleLearning).
pub(crate) const STALE_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Max character width for task titles shown in confirmation popups and status messages.
pub(in crate::tui) const TITLE_DISPLAY_LENGTH: usize = 30;

/// Maximum time a task may remain in the `dispatching` set before the watchdog
/// force-fails it. Defence-in-depth against a stuck dispatch worker.
///
/// Sized off `provision_worktree`'s real worst-case sequential subprocess
/// time, not a 1:1 mirror of `SUBPROCESS_TIMEOUT` (`src/process.rs`) — that
/// 1:1 relationship was itself the bug (#4201): `fetch_origin`'s retry budget
/// under `FetchPolicy::Required` can issue up to `PROVISION_MAX_SUBPROCESS_CALLS`
/// (`src/dispatch/worktree.rs`) sequential `SUBPROCESS_TIMEOUT`-bounded calls
/// before a fresh dispatch succeeds or gives up, so a watchdog sized for one
/// call could trip while the worker was still legitimately retrying within
/// policy. At current values this is `120s * 7 = 840s` (14 minutes) — a
/// deliberate trade-off, not incidental: it does not change worker behaviour
/// (the dispatch worker runs on a detached thread the watchdog never cancels
/// either way — see `DispatchingTimeout` in `docs/specs/dispatch.allium`),
/// only how long a *genuinely* stuck worker sits silently before the user
/// sees anything.
pub(in crate::tui) const DISPATCH_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(
    crate::process::SUBPROCESS_TIMEOUT.as_secs()
        * crate::dispatch::PROVISION_MAX_SUBPROCESS_CALLS as u64,
);

/// Number of braille spinner frames for the per-card "dispatching…" indicator.
/// Must match the length of `DISPATCHING_SPINNER` in `kanban.rs`.
pub(in crate::tui) const DISPATCH_SPINNER_FRAMES: u8 = 10;

/// The one phrasing of "another machine holds this task's worktree", shared by
/// every handler that refuses on `Task::is_locally_owned` — the activate key
/// (`src/tui/input.rs`) and both retry arms (`src/tui/update/retry.rs`). They
/// state one condition and had drifted into three near-identical strings.
///
/// `verb` names the action being refused ("resume", "retry") and produces
/// `Cannot <verb>: …`; `None` is the bare statement, for a caller whose
/// refused action is the keypress itself and has no verb to name.
///
/// Deliberately does not name the owning host's label — resolving a foreign
/// host id to a label needs the shared-host registry that is out of scope for
/// this session (see the distributed-dispatch design doc's "Explicitly not in
/// this session").
pub(in crate::tui) fn foreign_worktree_refusal(verb: Option<&str>) -> String {
    const PHRASE: &str = "this task's worktree is on another machine";
    match verb {
        Some(verb) => format!("Cannot {verb}: {PHRASE}"),
        None => {
            let mut sentence = PHRASE.to_string();
            sentence[..1].make_ascii_uppercase();
            sentence
        }
    }
}

// ---------------------------------------------------------------------------
// ReparentPickerState
// ---------------------------------------------------------------------------

/// State for the reparent-epic tree picker overlay.
/// Lives on `App` directly (not inside `InputState`) because `RefCell<TreeState>`
/// does not implement `Clone`, and `InputState` derives `Clone`.
pub(in crate::tui) struct ReparentPickerState {
    pub(in crate::tui) epic_id: EpicId,
    pub(in crate::tui) tree_state: std::cell::RefCell<tui_tree_widget::TreeState<String>>,
    /// Pre-built tree items. Computed once when the picker opens so the render
    /// path never calls `reparent_target_epics` or `build_reparent_tree` per frame.
    pub(in crate::tui) items: Vec<tui_tree_widget::TreeItem<'static, String>>,
}

/// State for the move-task-to-epic tree picker overlay (the `m` key on a task
/// card). Mirrors [`ReparentPickerState`] but targets a task instead of an epic.
pub(in crate::tui) struct MoveTaskPickerState {
    pub(in crate::tui) task_id: TaskId,
    pub(in crate::tui) tree_state: std::cell::RefCell<tui_tree_widget::TreeState<String>>,
    /// Pre-built tree items. Computed once when the picker opens so the render
    /// path never calls `move_task_target_epics` or `build_reparent_tree` per frame.
    pub(in crate::tui) items: Vec<tui_tree_widget::TreeItem<'static, String>>,
}

// ---------------------------------------------------------------------------
// InteractionState — transient overlay/picker state, one-at-a-time by construction
// ---------------------------------------------------------------------------

/// Transient overlay/picker UI state: at most one of these is meaningfully
/// active at a time (each is gated by a distinct `InputMode`). Grouped so `App`'s own field list only carries genuine
/// board/session state, not this long tail of "is some popup open" flags.
/// Not `Clone` (mirrors [`ReparentPickerState`]/[`MoveTaskPickerState`]:
/// their `RefCell<TreeState>` fields don't implement it).
#[derive(Default)]
pub(in crate::tui) struct InteractionState {
    pub(in crate::tui) reparent_picker: Option<ReparentPickerState>,
    pub(in crate::tui) move_task_picker: Option<MoveTaskPickerState>,
    /// A single `g` press awaiting a possible second `g` (the `gg` chord,
    /// jump to top of column) within [`GG_CHORD_TIMEOUT`]. Holds the press
    /// instant. Armed only on the board; resolved by the next keypress
    /// (`dispatch_key`) or, if the user goes idle after a lone `g`,
    /// by `handle_tick` as a backstop.
    pub(in crate::tui) pending_g: Option<Instant>,
    /// Scroll offset of the `?` help overlay: the body line shown at the top.
    /// Reset to 0 when the overlay opens; rendering clamps an offset past the
    /// end.
    pub(in crate::tui) help_scroll: usize,
    /// The largest offset the overlay can show, as the last render found it
    /// (`None` until it has rendered). Bounds scrolling down so `k` answers
    /// at once after reaching the end; a `Cell` because rendering reads `&App`.
    pub(in crate::tui) help_max_scroll: std::cell::Cell<Option<usize>>,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub(in crate::tui) board: BoardState,
    pub(in crate::tui) status: StatusState,
    pub(in crate::tui) should_quit: bool,
    pub(in crate::tui) notifications_enabled: bool,
    pub(in crate::tui) input: InputState,
    pub(in crate::tui) agents: AgentTracking,
    pub(in crate::tui) select: SelectionState,
    pub(in crate::tui) filter: FilterState,
    pub(in crate::tui) search: SearchState,
    /// Which sub-status sections the user has folded. A persisted preference —
    /// see [`SectionFoldState`].
    pub(in crate::tui) folds: SectionFoldState,
    /// Which flattened epic groups the user has folded. A persisted
    /// preference, independent of `folds` — see [`EpicFoldState`].
    pub(in crate::tui) epic_folds: EpicFoldState,
    /// Task IDs with an in-flight dispatch, mapped to their start time.
    /// Membership prevents duplicate dispatches; start times drive the 60-second watchdog.
    pub(in crate::tui) dispatching: HashMap<TaskId, Instant>,
    /// Spinner frame index (0..DISPATCH_SPINNER_FRAMES) for the per-card "dispatching…" indicator.
    /// Advanced by `Tick` only while `dispatching` is non-empty.
    pub(in crate::tui) spinner_tick: u8,
    /// Latest budget snapshot read from `<data_dir>/rate-limits.json`. `None`
    /// when absent or unreadable — the steady state for non-subscription auth.
    /// Derived live, never persisted (dispatch.allium: TokenBudgetIndicator).
    pub(in crate::tui) budget: Option<crate::models::budget::BudgetSnapshot>,
    /// The address of the store this board is connected to, drawn on the top
    /// row in every state (sync.allium: ConnectionIndicator,
    /// `TheStoreIsAlwaysNamed`). `None` only before bootstrap sets it.
    pub(in crate::tui) store_server: Option<String>,
    pub(in crate::tui) ticks_since_budget_poll: u64,
    /// Derived layout state (epic stats, anchor cache, task index, and their
    /// fingerprints) computed from `board.tasks`/`board.epics`. See
    /// [`LayoutCache`] for coherence details.
    pub(in crate::tui) layout: LayoutCache,
    /// Set to `true` whenever state changes that should trigger a redraw.
    /// The runtime skips `terminal.draw` on consecutive events that leave
    /// `dirty` false (e.g. an idle tick whose DB refresh found no changes).
    pub dirty: bool,
    /// Set to `true` when a `Persist` or `BatchPatchSubStatus` command
    /// completes, cleared when `handle_tick` emits `RefreshFromDb`.
    /// Ensures the board re-reads from DB promptly after any write.
    pub dirty_since_refresh: bool,
    /// Ticks elapsed since the last `RefreshFromDb` was emitted. Reset to 0
    /// on each refresh; the fallback fires when this reaches 5 (= 10 s).
    pub(in crate::tui) ticks_since_last_refresh: u64,
    /// Transient overlay/picker state (pickers, in-progress popup edits, the
    /// one-shot pending action). See [`InteractionState`].
    pub(in crate::tui) interaction: InteractionState,
    /// The table key presses are looked up in. Always
    /// [`crate::keybindings::KEY_BINDINGS`] outside tests.
    pub(in crate::tui) key_table: &'static [crate::keybindings::KeyBinding],
    /// Paths in `board.repo_paths` that do not exist on disk (`is_dir()` → false).
    /// Recomputed once in `handle_repo_paths_updated` so the render path is
    /// never blocked by filesystem syscalls on every frame.
    pub(in crate::tui) broken_repo_paths: HashSet<String>,
    /// Per-repository drift measurements, keyed by repo path
    /// (docs/specs/repo-sync.allium: entity RepoSyncState). Purely in-memory:
    /// every refresh point re-establishes it, and nothing is persisted.
    pub(in crate::tui) repo_sync: crate::repo_sync::RepoSyncCache,
    /// Wall-clock of the last stale-learning cleanup sweep. `None` = never run.
    /// `handle_tick` emits `LearningCommand::ArchiveStale` at most once per
    /// [`STALE_CLEANUP_INTERVAL`] by consulting this. See
    /// docs/specs/learnings.allium: ArchiveStaleLearning.
    pub(crate) last_stale_cleanup_at: Option<Instant>,
    /// This install's opaque Host id (`core/Host.id` in `docs/specs/core.allium`),
    /// used to evaluate `Task::is_locally_owned` in board handlers that cannot
    /// reach the database (they act on `board.tasks` synchronously). `None`
    /// until `TuiRuntime::bootstrap` sets it via [`Self::set_local_host_id`],
    /// having minted or read the identity from `settings` (see host.allium:
    /// MintHostIdentity); a real launch aborts rather than drawing a board
    /// with it still unset (startup.allium:
    /// AbortWhenTheHostIdentityStoreIsUnusable), so `None` is the state an
    /// `App` built directly — as tests do — is in.
    pub(in crate::tui) local_host_id: Option<String>,
}

/// FNV-1a offset basis, used as the seed for the layout-cache fingerprints
/// (`App::compute_layout_fingerprint`, `App::compute_task_ids_fingerprint`).
/// These are internal, non-adversarial fingerprints — a cheap fold is
/// plenty and much cheaper than `DefaultHasher` (SipHash) on the hot render
/// path.
fn fnv_seed() -> u64 {
    0xcbf29ce484222325
}

/// Fold one `u64` field into an FNV-1a-style accumulator.
pub(in crate::tui) fn fnv_fold(acc: u64, v: u64) -> u64 {
    const FNV_PRIME: u64 = 0x100000001b3;
    (acc ^ v).wrapping_mul(FNV_PRIME)
}

/// Hash a byte string on its own, for folding into a larger accumulator as a
/// single value.
fn fnv_bytes(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(fnv_seed(), |acc, b| fnv_fold(acc, *b as u64))
}

/// A card of a flattened column, decorated once with its sort keys:
/// (section, epic-group key, card key, task).
type FlatCard<'a> = (Option<ColumnSection>, CardOrderKey, CardOrderKey, &'a Task);

impl App {
    pub fn new(tasks: Vec<Task>) -> Self {
        let mut app = App {
            board: BoardState {
                tasks,
                epics: Vec::new(),
                view_mode: ViewMode::default(),
                repo_paths: Vec::new(),
                repo_base_branches: HashMap::new(),
                split: SplitState::default(),
                flattened: false,
            },
            status: StatusState::default(),
            should_quit: false,
            notifications_enabled: false,
            input: InputState::default(),
            agents: AgentTracking::new(),
            select: SelectionState::default(),
            filter: FilterState::default(),
            search: SearchState::default(),
            folds: SectionFoldState::default(),
            epic_folds: EpicFoldState::default(),
            dispatching: HashMap::new(),
            spinner_tick: 0,
            budget: None,
            store_server: None,
            ticks_since_budget_poll: 0,
            layout: LayoutCache::default(),
            dirty: true,
            dirty_since_refresh: true,
            ticks_since_last_refresh: 0,
            interaction: InteractionState::default(),
            key_table: crate::keybindings::KEY_BINDINGS,
            broken_repo_paths: HashSet::new(),
            repo_sync: crate::repo_sync::RepoSyncCache::default(),
            last_stale_cleanup_at: None,
            local_host_id: None,
        };
        // Prime all caches so the first render is a cache hit instead of recomputing.
        let _ = app.cached_epic_stats();
        app.update_anchor_from_current();
        app
    }

    /// Set this install's local Host id, read from `settings` once at
    /// startup (see `TuiRuntime::bootstrap`). Board handlers that gate on
    /// `Task::is_locally_owned` (RetryResume, RetryFresh, JumpToAgentWindow's
    /// priority-0 branch) read it back via [`Self::local_host_id`].
    pub fn set_local_host_id(&mut self, id: String) {
        self.local_host_id = Some(id);
    }

    /// This install's local Host id, or `None` before bootstrap has read it.
    /// See [`Self::set_local_host_id`].
    pub(in crate::tui) fn local_host_id(&self) -> Option<&str> {
        self.local_host_id.as_deref()
    }

    /// Returns true if the given task has an in-flight dispatch *started by
    /// this TUI process*. Not the whole picture — see
    /// [`Self::dispatch_may_be_in_flight`].
    pub fn is_dispatching(&self, id: TaskId) -> bool {
        self.dispatching.contains_key(&id)
    }

    /// Whether an unprovisioned task is unprovisioned because a dispatch is
    /// still running, rather than because one died.
    ///
    /// `dispatching` only holds dispatches this TUI started. The epic
    /// auto-dispatch chain claims its next subtask inside the MCP handler
    /// (`auto_dispatch_next`) and never enters that map, and a TUI restart
    /// mid-dispatch empties it — in both cases the row is `Running` with no
    /// worktree while an agent is genuinely being provisioned. So fall back to
    /// the row itself: every claim seeds `last_pre_tool_use_at`, and
    /// [`DISPATCH_WATCHDOG_TIMEOUT`] is already the line this codebase draws
    /// between "slow" and "dead" (see `DispatchingTimeout` in
    /// `docs/specs/dispatch.allium`).
    ///
    /// A missing stamp counts as not-in-flight, so an unstamped row surfaces
    /// immediately rather than hiding for a minute.
    ///
    /// Only meaningful for `task.is_unprovisioned()`; a provisioned task has
    /// its stamp refreshed by agent hooks and would always look "fresh".
    pub fn dispatch_may_be_in_flight(&self, task: &Task, now: DateTime<Utc>) -> bool {
        if self.is_dispatching(task.id) {
            return true;
        }
        task.last_pre_tool_use_at.is_some_and(|stamp| {
            now.signed_duration_since(stamp)
                .to_std()
                .is_ok_and(|elapsed| elapsed < DISPATCH_WATCHDOG_TIMEOUT)
        })
    }

    /// Get the current selection state (from whichever view mode is active).
    pub fn selection(&self) -> &BoardSelection {
        self.board.view_mode.selection()
    }

    /// Get mutable access to the current selection state.
    pub(in crate::tui) fn selection_mut(&mut self) -> &mut BoardSelection {
        self.board.view_mode.selection_mut()
    }

    // Read-only accessors for code outside the tui module
    pub fn tasks(&self) -> &[Task] {
        &self.board.tasks
    }
    pub fn should_quit(&self) -> bool {
        self.should_quit
    }
    pub fn selected_column(&self) -> usize {
        self.selection().column()
    }
    pub fn selected_row(&self) -> &[usize; TaskStatus::COLUMN_COUNT] {
        &self.selection().selected_row
    }
    pub fn view_mode(&self) -> &ViewMode {
        &self.board.view_mode
    }
    pub fn epics(&self) -> &[Epic] {
        &self.board.epics
    }
    pub fn mode(&self) -> &InputMode {
        &self.input.mode
    }
    pub fn split_active(&self) -> bool {
        self.board.split.active
    }
    pub fn split_focused(&self) -> bool {
        self.board.split.focused
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn status_message(&self) -> Option<&str> {
        self.status.message.as_deref()
    }
    pub fn input_mode(&self) -> &InputMode {
        &self.input.mode
    }
    pub fn error_popup(&self) -> Option<&str> {
        self.status.error_popup.as_deref()
    }
    pub fn repo_paths(&self) -> &[String] {
        &self.board.repo_paths
    }
    /// The most-recently-used base_branch history for `repo_path`, ordered
    /// most-recent-first. Empty when the repo has no recorded history.
    pub fn base_branches_for(&self, repo_path: &str) -> &[String] {
        self.board
            .repo_base_branches
            .get(repo_path)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// The candidate slice for the picker rendered by the current
    /// `InputMode`, if any: `InputBaseBranch` scopes to the draft's
    /// repo_path's history; `InputMode::is_repo_picker()` modes use the
    /// global saved repo-path list. `None` when the current mode has no
    /// picker candidates (e.g. plain text fields).
    pub(in crate::tui) fn picker_candidates(&self) -> Option<&[String]> {
        if matches!(self.input.mode, InputMode::InputBaseBranch) {
            let repo_path = self
                .input
                .task_draft
                .as_ref()
                .map(|d| d.repo_path.as_str())
                .unwrap_or("");
            Some(self.base_branches_for(repo_path))
        } else if self.input.mode.is_repo_picker() {
            Some(&self.board.repo_paths)
        } else {
            None
        }
    }
    pub fn task_draft(&self) -> Option<&TaskDraft> {
        self.input.task_draft.as_ref()
    }
    pub fn is_stale(&self, id: TaskId) -> bool {
        self.find_task(id)
            .is_some_and(|t| t.sub_status == SubStatus::Stale)
    }
    pub fn is_crashed(&self, id: TaskId) -> bool {
        self.find_task(id)
            .is_some_and(|t| t.sub_status == SubStatus::Crashed)
    }
    pub fn selected_tasks(&self) -> &HashSet<TaskId> {
        &self.select.tasks
    }
    pub fn selected_epics(&self) -> &HashSet<EpicId> {
        &self.select.epics
    }
    pub fn on_select_all(&self) -> bool {
        self.selection().on_select_all
    }
    pub fn has_selection(&self) -> bool {
        self.select.has_selection()
    }

    pub fn notifications_enabled(&self) -> bool {
        self.notifications_enabled
    }
    pub fn repo_filter(&self) -> &HashSet<String> {
        &self.filter.repos
    }
    pub fn repo_filter_mode(&self) -> RepoFilterMode {
        self.filter.mode
    }

    pub fn filter_only_active(&self) -> bool {
        self.filter.only_active
    }

    /// Bootstrap-only carve-out: set during runtime startup from the saved
    /// `notifications_enabled` setting before the message loop begins. After
    /// bootstrap completes, this state is mutated only via Messages. See the
    /// "Visibility Convention" section in CLAUDE.md.
    pub fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
    }

    /// Bootstrap-only carve-out, like [`Self::set_notifications_enabled`]:
    /// the store address is fixed for the board's lifetime.
    pub fn set_store_server(&mut self, server: Option<String>) {
        self.store_server = server;
    }

    /// The store address the top row names.
    pub fn store_server(&self) -> Option<&str> {
        self.store_server.as_deref()
    }

    pub fn set_repo_filter(&mut self, filter: HashSet<String>) {
        self.filter.repos = filter;
        self.sync_board_selection();
    }

    pub fn set_repo_filter_mode(&mut self, mode: RepoFilterMode) {
        self.filter.mode = mode;
        self.sync_board_selection();
    }

    /// Compute the sticky status text for the current `dispatching` set.
    /// Returns `None` when no dispatch is in flight.
    pub(in crate::tui) fn dispatching_status_text(&self) -> Option<String> {
        let count = self.dispatching.len();
        if count == 0 {
            return None;
        }
        if count == 1 {
            let (&id, _) = self.dispatching.iter().next()?;
            let label = self
                .find_task(id)
                .map(|t| {
                    let trimmed = t.title.trim();
                    if trimmed.is_empty() {
                        format!("task #{}", id.0)
                    } else if trimmed.chars().count() <= TITLE_DISPLAY_LENGTH {
                        format!("'{trimmed}'")
                    } else {
                        let truncated: String =
                            trimmed.chars().take(TITLE_DISPLAY_LENGTH - 1).collect();
                        format!("'{truncated}…'")
                    }
                })
                .unwrap_or_else(|| format!("task #{}", id.0));
            Some(format!("Dispatching {label}…"))
        } else {
            Some(format!("Dispatching {count} tasks…"))
        }
    }

    /// Mark a task as mid-dispatch and update the sticky status text.
    /// This is the single side-effect path for adding to `dispatching`.
    /// No-op if the task ID is not present in the task list.
    ///
    /// UI-only state update — does not perform dispatch. The caller (a
    /// `Command` handler) has already executed the side effect; this
    /// method only records the in-flight UI marker.
    /// Every production caller reaches this only for an unprovisioned task
    /// (`SpansTheClaim` in docs/specs/dispatch.allium) — the dispatch paths filter
    /// on Backlog, and retry-fresh clears the worktree first — but that is not
    /// asserted here. It is a property of the callers, not of this setter, and a
    /// `debug_assert` would fire on any test that drives the marker directly.
    pub(in crate::tui) fn mark_dispatching(&mut self, id: TaskId) {
        if self.find_task(id).is_none() {
            return;
        }
        self.dispatching.insert(id, Instant::now());
        // A retry is the resolution to a stalled chain, so starting one clears
        // the failure marker (PersistsUntilRedispatched in
        // docs/specs/epics.allium) — and keeps a stale marker from masking the
        // retry's own spinner.
        self.agents.auto_dispatch_failed.remove(&id);
        if let Some(msg) = self.dispatching_status_text() {
            self.status.set_sticky(msg);
        }
    }

    /// Remove a task from the dispatching map and recompute the sticky status.
    pub(in crate::tui) fn unmark_dispatching(&mut self, id: TaskId) {
        self.dispatching.remove(&id);
        self.refresh_dispatching_status();
    }

    /// Recompute the sticky status text after `dispatching` has been mutated.
    /// Clears the status if no dispatches remain.
    pub(in crate::tui) fn refresh_dispatching_status(&mut self) {
        match self.dispatching_status_text() {
            Some(msg) => self.status.set_sticky(msg),
            None => {
                if self.status.message_sticky {
                    self.status.clear();
                }
            }
        }
    }

    /// Replace the whole folded set, as the startup restore does. Not a
    /// toggle: it installs what storage held rather than editing it.
    pub fn set_section_folds(&mut self, folds: SectionFoldState) {
        self.folds = folds;
        self.invalidate_layout_cache();
    }

    /// Replace the whole folded-epic set, as the startup restore does. Not a
    /// toggle: it installs what storage held rather than editing it.
    pub fn set_epic_folds(&mut self, folds: EpicFoldState) {
        self.epic_folds = folds;
        self.invalidate_layout_cache();
    }

    /// The status of the column the cursor is in, or `None` on the select-all
    /// column.
    pub(in crate::tui) fn selected_column_status(&self) -> Option<TaskStatus> {
        let col = self.selection().column();
        TaskStatus::from_column_index(col.checked_sub(1)?)
    }

    /// Whether the cursor is in a column flattened mode applies to.
    pub(in crate::tui) fn cursor_in_flattened_column(&self) -> bool {
        self.selected_column_status()
            .is_some_and(|s| self.view().is_flattened_for_status(s))
    }

    pub(in crate::tui) fn find_task(&self, id: TaskId) -> Option<&Task> {
        self.board.tasks.iter().find(|t| t.id == id)
    }

    pub(in crate::tui) fn find_task_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        // Rebuild index if missing or stale (e.g. direct board.tasks mutation in
        // tests, or a wholesale same-length replacement of board.tasks with a
        // different id set — a length-only check would miss that).
        let fingerprint = self.view().compute_task_ids_fingerprint();
        if self.layout.task_index.is_none()
            || self.layout.task_index_fingerprint != Some(fingerprint)
        {
            self.layout.task_index = Some(
                self.board
                    .tasks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (t.id, i))
                    .collect(),
            );
            self.layout.task_index_fingerprint = Some(fingerprint);
        }
        let i = self.layout.task_index.as_ref()?.get(&id).copied()?;
        self.board.tasks.get_mut(i)
    }

    pub(in crate::tui) fn find_epic(&self, id: EpicId) -> Option<&Epic> {
        self.board.epics.iter().find(|e| e.id == id)
    }

    /// Remove all in-memory agent tracking state for a task.
    pub(in crate::tui) fn clear_agent_tracking(&mut self, id: TaskId) {
        self.agents.clear(id);
    }

    /// Take worktree/tmux fields from a task and build a Cleanup command.
    ///
    /// Returns `None` only for a task that owns **neither** a worktree nor a tmux
    /// window — there is nothing to tear down, so the caller's follow-up applies
    /// immediately instead. Anything else is queued, the window-only shape
    /// included: `TeardownIsOwedWheneverThereIsSomethingToRelease` in
    /// docs/specs/tasks.allium, whose gating on the worktree here is what leaked
    /// those windows on delete (#4096).
    ///
    /// Clearing the board's copy here is optimism, not the authority: the DB
    /// write that forgets the path is `follow_up`, applied only once the removal
    /// has succeeded (`WorktreeReleaseIsGated` in docs/specs/tasks.allium). A
    /// failed removal refreshes the board from the row it did not change.
    pub(in crate::tui) fn take_cleanup(
        task: &mut Task,
        follow_up: crate::tui::commands::CleanupFollowUp,
        guard: Option<crate::tui::commands::DeleteGuard>,
    ) -> Option<Command> {
        let worktree = task.worktree.take();
        let tmux_window = task.tmux_window.take();
        // Paired with `worktree` per core/Task's `HostTracksWorktree`
        // invariant (docs/specs/core.allium); the DB write that earns this
        // optimism is `clear_worktree_pointer` (src/runtime/tasks.rs).
        task.host = None;
        if worktree.is_none() && tmux_window.is_none() {
            return None;
        }
        Some(Command::Task(crate::tui::commands::TaskCommand::Cleanup {
            id: task.id,
            repo_path: task.repo_path.clone(),
            worktree,
            tmux_window,
            follow_up,
            guard,
        }))
    }

    /// Move a task's status on the board, in step with what the service layer
    /// will write for the same transition: `sub_status` resets to the new
    /// status's default, and a deferred Stop is voided when the card leaves
    /// Running (`clears_pending_stop`, mirroring `PendingStopOnlyWhileRunning`
    /// in `docs/specs/core.allium`).
    ///
    /// The `stop_pending` half is board coherence rather than correctness — the
    /// DB write `Persist` carries is the authority, and the tick reconciler's
    /// own write is conditional on the row. It exists so the board cannot show
    /// a state the row does not have between a move and the next refresh.
    ///
    /// Every board mutation that lands a task in a new status should go through
    /// here; the alternative is remembering two derived fields at each site.
    pub(in crate::tui) fn set_local_status(task: &mut Task, next: TaskStatus) {
        if crate::models::clears_pending_stop(task.status, next) {
            task.stop_pending = false;
        }
        task.status = next;
        task.sub_status = SubStatus::default_for(next);
    }

    /// Take the tmux_window from a task and build a KillTmuxWindow command.
    /// Leaves the worktree intact so the task can be resumed later.
    pub(in crate::tui) fn take_detach(task: &mut Task) -> Option<Command> {
        task.tmux_window.take().map(|window| {
            Command::Task(crate::tui::commands::TaskCommand::KillTmuxWindow { window })
        })
    }

    /// Process a message and return a list of side-effect commands.
    ///
    /// The routing match lives in `dispatcher.rs`; this method is a thin
    /// delegate so adding a `Message` variant is a two-file edit.
    pub fn update(&mut self, msg: Message) -> Vec<Command> {
        dispatcher::dispatch(self, msg)
    }

    // -----------------------------------------------------------------------
    // Per-message handlers
    // -----------------------------------------------------------------------

    pub(in crate::tui) fn handle_detach_tmux(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
        let detachable: Vec<TaskId> = ids
            .iter()
            .filter(|&&id| self.find_task(id).is_some_and(|t| t.tmux_window.is_some()))
            .copied()
            .collect();

        if detachable.is_empty() {
            return vec![];
        }

        let count = detachable.len();
        let msg = if count == 1 {
            "Detach tmux panel? [y/n]".to_string()
        } else {
            format!("Detach {count} tmux panels? [y/n]")
        };
        self.input.mode = InputMode::ConfirmDetachTmux(detachable);
        self.status.set(msg);
        vec![]
    }

    pub(in crate::tui) fn handle_confirm_detach_tmux(&mut self) -> Vec<Command> {
        let InputMode::ConfirmDetachTmux(ref ids) = self.input.mode else {
            return vec![];
        };
        let ids = ids.clone();
        self.input.mode = InputMode::Normal;
        self.status.clear();
        self.detach_tmux_panels(ids)
    }

    pub(in crate::tui) fn detach_tmux_panels(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
        let mut cmds = Vec::new();
        for id in ids {
            self.clear_agent_tracking(id);
            if let Some(task) = self.find_task_mut(id) {
                if let Some(window) = task.tmux_window.take() {
                    cmds.push(Command::Task(
                        crate::tui::commands::TaskCommand::KillTmuxWindow { window },
                    ));
                }
                // Reset sub_status when detaching (e.g. Stale/Crashed -> default)
                if task.sub_status == SubStatus::Stale || task.sub_status == SubStatus::Crashed {
                    task.sub_status = SubStatus::default_for(task.status);
                }
                let fields = crate::tui::commands::PersistFields::from_task(task);
                cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                    fields,
                )));
            }
            // Drain: the agent genuinely finished its turn, so a pending Stop
            // should land as the Review flip.
            cmds.push(Command::Task(
                crate::tui::commands::TaskCommand::ClearSubagents {
                    id,
                    mode: crate::models::DrainMode::Drain,
                },
            ));
        }
        cmds
    }

    pub(in crate::tui) fn finish_epic_creation(&mut self) -> Vec<Command> {
        let draft = self.input.epic_draft.take().unwrap_or_default();
        self.input.mode = InputMode::Normal;
        self.status.clear();
        vec![Command::Epic(crate::tui::commands::EpicCommand::Insert(
            draft,
        ))]
    }
}

#[cfg(test)]
mod tests;
