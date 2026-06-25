pub mod commands;
mod dispatcher;
pub mod input;
pub mod messages;
pub mod types;
pub mod ui;
pub mod update;

pub use types::*;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::models::ReviewDecision;
use crate::models::{
    epic_substatus, Epic, EpicId, EpicSubstatus, SubStatus, Task, TaskId, TaskStatus, VisualColumn,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How long a transient status message stays visible before auto-clearing.
pub(in crate::tui) const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

/// Interval between PR status polls for tasks in review.
pub(in crate::tui) const PR_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Max character width for task titles shown in confirmation popups and status messages.
pub(in crate::tui) const TITLE_DISPLAY_LENGTH: usize = 30;

/// Maximum time a task may remain in the `dispatching` set before the watchdog
/// force-fails it. Defence-in-depth against a stuck dispatch worker.
pub(in crate::tui) const DISPATCH_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(60);

/// Number of braille spinner frames for the per-card "dispatching…" indicator.
/// Must match the length of `DISPATCHING_SPINNER` in `kanban.rs`.
pub(in crate::tui) const DISPATCH_SPINNER_FRAMES: u8 = 10;

/// Returns true for the Archive edge column that doesn't hold regular task data
/// and must be excluded from task-operation hotkeys.
pub(in crate::tui) fn is_edge_column(col: usize) -> bool {
    col == TaskStatus::COLUMN_COUNT + 1
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
}

/// State for the move-task-to-epic tree picker overlay (the `m` key on a task
/// card). Mirrors [`ReparentPickerState`] but targets a task instead of an epic.
pub(in crate::tui) struct MoveTaskPickerState {
    pub(in crate::tui) task_id: TaskId,
    pub(in crate::tui) tree_state: std::cell::RefCell<tui_tree_widget::TreeState<String>>,
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
    pub(in crate::tui) archive: ArchiveState,
    pub(in crate::tui) select: SelectionState,
    pub(in crate::tui) filter: FilterState,
    pub(in crate::tui) search: SearchState,
    pub(in crate::tui) merge_queue: Option<MergeQueue>,
    /// Task IDs with an in-flight dispatch, mapped to their start time.
    /// Membership prevents duplicate dispatches; start times drive the 60-second watchdog.
    pub(in crate::tui) dispatching: HashMap<TaskId, Instant>,
    /// Spinner frame index (0..DISPATCH_SPINNER_FRAMES) for the per-card "dispatching…" indicator.
    /// Advanced by `Tick` only while `dispatching` is non-empty.
    pub(in crate::tui) spinner_tick: u8,
    pub(in crate::tui) tips: Option<TipsOverlayState>,
    pub(in crate::tui) main_session_dir: Option<String>,
    /// Number of approved-pool learnings currently in `NeedsReview` status.
    /// Drives the `[KB:N]` badge on the kanban status bar; refreshed alongside
    /// epics/usage in `exec_refresh_from_db`.
    pub(in crate::tui) needs_review_count: i64,
    /// Cached result of `compute_epic_stats()`, wrapped in an `Arc` so that
    /// `cached_epic_stats()` returns a reference-counted handle (O(1) clone)
    /// rather than cloning the full `HashMap` on every call.  Cleared by
    /// `invalidate_layout_cache()`, which every board-mutation handler calls
    /// (directly or via `sync_board_selection`).
    pub(in crate::tui) epic_stats_cache: Option<Arc<EpicStatsMap>>,
    /// Parent→children adjacency map over `board.epics`. Built once alongside
    /// `epic_stats_cache` in `cached_epic_stats()`; passed into
    /// `compute_epic_stats()` so the map is not rebuilt for each epic.
    /// Cleared by `invalidate_layout_cache()`.
    pub(in crate::tui) children_map_cache: Option<HashMap<EpicId, Vec<EpicId>>>,
    /// Pre-sorted selectable items (tasks + epics) per status in display order.
    /// Built once alongside `epic_stats_cache`; `update_anchor_from_current`
    /// reads from this (O(1) per nav event) instead of re-sorting the column.
    pub(in crate::tui) column_anchor_cache: Option<HashMap<TaskStatus, Vec<ColumnAnchor>>>,
    /// Per-epic `(epic_repo_matches, epic_matches)` results, built once per render frame
    /// inside `cached_epic_stats()` using a single shared `build_children_map()` call.
    /// Cleared by `invalidate_layout_cache()`.
    pub(in crate::tui) epic_filter_cache: Option<HashMap<EpicId, (bool, bool)>>,
    /// TaskId → Vec index for O(1) lookups in `find_task_mut`. Not primed in
    /// `App::new()` to avoid staleness when tests mutate `board.tasks` directly.
    /// Rebuilt lazily in `find_task_mut` when None or when length mismatches
    /// `board.tasks`. Cleared by `invalidate_layout_cache()`.
    pub(in crate::tui) task_index: Option<HashMap<TaskId, usize>>,
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
    pub(in crate::tui) reparent_picker: Option<ReparentPickerState>,
    pub(in crate::tui) move_task_picker: Option<MoveTaskPickerState>,
    /// Persisted managed-feed settings, snapshotted so the config popup opens
    /// without a DB round-trip. Loaded at startup, refreshed after a save.
    pub(in crate::tui) managed_feed_settings: ManagedFeedSettings,
    /// In-progress edit buffer for the managed-feed config popup; `Some` only
    /// while the popup is open.
    pub(in crate::tui) managed_feed_config: Option<ManagedFeedConfigState>,
    /// ID of the todo item being edited in `InputMode::TodoTitle`; `None` means
    /// a new item is being created (add flow).
    pub(in crate::tui) pending_todo_edit: Option<crate::models::TodoId>,
    /// ID of the todo item awaiting confirmation in `InputMode::ConfirmDeleteTodo`.
    pub(in crate::tui) pending_todo_delete: Option<crate::models::TodoId>,
    /// Link (task or epic) to attach to the next quick-add todo; set by the `[t]`
    /// key handler when a task/epic is selected, cleared after the submit.
    pub(in crate::tui) pending_todo_link: Option<crate::models::TodoLink>,
}

/// Format a title for display in confirmation prompts, truncating if longer than `max_len` chars.
pub(in crate::tui) fn truncate_title(title: &str, max_len: usize) -> String {
    if title.chars().count() <= max_len {
        format!("\"{title}\"")
    } else {
        let truncated: String = title.chars().take(max_len.saturating_sub(3)).collect();
        format!("\"{truncated}...\"")
    }
}

/// Returns true if every character in `query_lower` (already lowercased) appears in
/// `path` as a forward subsequence (case-insensitive on `path`).
/// An empty query matches everything.
pub(in crate::tui) fn fuzzy_matches_lower(path: &str, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }
    let path_lower = path.to_lowercase();
    let mut path_chars = path_lower.chars();
    for qc in query_lower.chars() {
        if !path_chars.any(|pc| pc == qc) {
            return false;
        }
    }
    true
}

/// Returns true if every character in `query` appears in `path` as a
/// forward subsequence (case-insensitive). An empty query matches everything.
pub(in crate::tui) fn fuzzy_matches(path: &str, query: &str) -> bool {
    fuzzy_matches_lower(path, &query.to_lowercase())
}

/// Returns the subset of `paths` that fuzzy-match `query`, preserving order.
pub(in crate::tui) fn filtered_repos(paths: &[String], query: &str) -> Vec<String> {
    paths
        .iter()
        .filter(|p| fuzzy_matches(p, query))
        .cloned()
        .collect()
}

/// Returns true when the buffer should be offered as a selectable "new path"
/// entry: the buffer is non-empty and is not already an exact member of
/// `filtered` (the user is typing a path that doesn't exist in the saved list).
pub(in crate::tui) fn has_new_repo_option(buffer: &str, filtered: &[String]) -> bool {
    !buffer.is_empty() && !filtered.iter().any(|p| p == buffer)
}

impl App {
    pub fn new(tasks: Vec<Task>) -> Self {
        let mut app = App {
            board: BoardState {
                tasks,
                epics: Vec::new(),
                view_mode: ViewMode::default(),
                repo_paths: Vec::new(),
                split: SplitState::default(),
                flattened: false,
                todo_open_count: 0,
            },
            status: StatusState::default(),
            should_quit: false,
            notifications_enabled: false,
            input: InputState::default(),
            agents: AgentTracking::new(),
            archive: ArchiveState::default(),
            select: SelectionState::default(),
            filter: FilterState::default(),
            search: SearchState::default(),
            merge_queue: None,
            dispatching: HashMap::new(),
            spinner_tick: 0,
            tips: None,
            main_session_dir: None,
            needs_review_count: 0,
            epic_stats_cache: None,
            children_map_cache: None,
            column_anchor_cache: None,
            epic_filter_cache: None,
            task_index: None,
            dirty: true,
            dirty_since_refresh: true,
            ticks_since_last_refresh: 0,
            reparent_picker: None,
            move_task_picker: None,
            managed_feed_settings: ManagedFeedSettings::default(),
            managed_feed_config: None,
            pending_todo_edit: None,
            pending_todo_delete: None,
            pending_todo_link: None,
        };
        // Prime all caches so the first render is a cache hit instead of recomputing.
        let _ = app.cached_epic_stats();
        app.update_anchor_from_current();
        app
    }

    /// Returns true if the given task has an in-flight dispatch.
    pub fn is_dispatching(&self, id: TaskId) -> bool {
        self.dispatching.contains_key(&id)
    }

    /// Get the current selection state (from whichever view mode is active).
    pub fn selection(&self) -> &BoardSelection {
        self.board.view_mode.selection()
    }

    /// Get mutable access to the current selection state.
    pub(in crate::tui) fn selection_mut(&mut self) -> &mut BoardSelection {
        self.board.view_mode.selection_mut()
    }

    /// When in TaskDetail overlay, returns the board mode beneath (Board or Epic).
    pub(in crate::tui) fn effective_view_mode(&self) -> &ViewMode {
        match &self.board.view_mode {
            ViewMode::TaskDetail { previous, .. } => previous.as_ref(),
            ViewMode::Learnings { previous, .. } => previous.as_ref(),
            ViewMode::Todos { previous, .. } => previous.as_ref(),
            other => other,
        }
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
    pub fn input_buffer(&self) -> &str {
        &self.input.buffer
    }
    pub fn split_active(&self) -> bool {
        self.board.split.active
    }
    pub fn split_focused(&self) -> bool {
        self.board.split.focused
    }
    pub fn split_pinned_task_id(&self) -> Option<TaskId> {
        self.board.split.pinned_task_id
    }
    pub fn status_message(&self) -> Option<&str> {
        self.status.message.as_deref()
    }
    pub fn error_popup(&self) -> Option<&str> {
        self.status.error_popup.as_deref()
    }
    pub fn repo_paths(&self) -> &[String] {
        &self.board.repo_paths
    }
    pub fn todo_open_count(&self) -> i64 {
        self.board.todo_open_count
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
    pub fn show_archived(&self) -> bool {
        self.selection().column() == TaskStatus::COLUMN_COUNT + 1
    }
    pub fn selected_archive_row(&self) -> usize {
        self.selection().row(TaskStatus::COLUMN_COUNT + 1)
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

    pub fn merge_queue(&self) -> Option<&MergeQueue> {
        self.merge_queue.as_ref()
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
    pub fn filter_presets(&self) -> &[(String, HashSet<String>, RepoFilterMode)] {
        &self.filter.presets
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

    pub fn main_session_dir(&self) -> Option<&str> {
        self.main_session_dir.as_deref()
    }

    /// Bootstrap-only carve-out: populated by the runtime loader from the four
    /// managed-feed settings at startup, and re-set in-memory after a save so
    /// the config popup re-opens with fresh values. See the config popup
    /// handlers in `update/managed_feeds.rs`.
    pub fn set_managed_feed_settings(&mut self, settings: ManagedFeedSettings) {
        self.managed_feed_settings = settings;
    }

    /// Read-only access to the in-progress config edit buffer (test/render use).
    pub fn managed_feed_config(&self) -> Option<&ManagedFeedConfigState> {
        self.managed_feed_config.as_ref()
    }

    /// Bootstrap-only carve-out: populated by the runtime loader from
    /// `main_session.dir` setting at startup. After bootstrap, set via Messages.
    pub fn set_main_session_dir(&mut self, dir: Option<String>) {
        self.main_session_dir = dir;
    }

    pub fn set_repo_filter(&mut self, filter: HashSet<String>) {
        self.filter.repos = filter;
        self.sync_board_selection();
    }

    pub fn set_repo_filter_mode(&mut self, mode: RepoFilterMode) {
        self.filter.mode = mode;
        self.sync_board_selection();
    }

    /// Set a transient status message with auto-clear timestamp.
    pub(in crate::tui) fn set_status(&mut self, msg: String) {
        self.status.message = Some(msg);
        self.status.message_set_at = Some(Instant::now());
        self.status.message_sticky = false;
    }

    /// Set a sticky status message that bypasses the 5-second TTL.
    /// The message persists until `clear_status` is called explicitly.
    pub(in crate::tui) fn set_status_sticky(&mut self, msg: String) {
        self.status.message = Some(msg);
        self.status.message_set_at = Some(Instant::now());
        self.status.message_sticky = true;
    }

    /// Clear the status message and its timestamp.
    pub(in crate::tui) fn clear_status(&mut self) {
        self.status.message = None;
        self.status.message_set_at = None;
        self.status.message_sticky = false;
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
    pub(in crate::tui) fn mark_dispatching(&mut self, id: TaskId) {
        if self.find_task(id).is_none() {
            return;
        }
        self.dispatching.insert(id, Instant::now());
        if let Some(msg) = self.dispatching_status_text() {
            self.set_status_sticky(msg);
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
            Some(msg) => self.set_status_sticky(msg),
            None => {
                if self.status.message_sticky {
                    self.clear_status();
                }
            }
        }
    }

    pub(in crate::tui) fn repo_matches(&self, repo_path: &str) -> bool {
        self.filter.matches(repo_path)
    }

    /// Returns whether the given epic should be shown under the current repo filter.
    /// An epic matches if:
    /// - No repo filter is active, OR
    /// - The epic has no non-archived subtasks (always show empty epics), OR
    /// - At least one non-archived subtask's repo_path matches the filter.
    ///
    pub(in crate::tui) fn epic_repo_matches(&self, epic_id: EpicId) -> bool {
        if let Some(ref cache) = self.epic_filter_cache {
            if let Some(&(repo_matches, _)) = cache.get(&epic_id) {
                return repo_matches;
            }
        }
        if self.filter.repos.is_empty() {
            return true;
        }
        let epic_ids = crate::models::descendant_epic_ids(epic_id, &self.board.epics);
        let has_active_tasks = self.board.tasks.iter().any(|t| {
            matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid))
                && t.status != TaskStatus::Archived
        });
        if !has_active_tasks {
            return true;
        }
        self.board.tasks.iter().any(|t| {
            matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid))
                && t.status != TaskStatus::Archived
                && self.repo_matches(&t.repo_path)
        })
    }

    pub(in crate::tui) fn epic_matches(&self, epic_id: EpicId) -> bool {
        if let Some(ref cache) = self.epic_filter_cache {
            if let Some(&(_, active_matches)) = cache.get(&epic_id) {
                return active_matches;
            }
        }
        if !self.filter.only_active {
            return true;
        }
        let epic_ids = crate::models::descendant_epic_ids(epic_id, &self.board.epics);
        self.board.tasks.iter().any(|t| {
            matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid)) && t.tmux_window.is_some()
        })
    }

    /// Epics eligible as reparent targets for `target`.
    ///
    /// Excludes the target epic and its descendants (cycle prevention), epics in
    /// `Done`/`Archived` status, and epics filtered out by the active repo /
    /// only-active filters (using the same predicates the board uses to decide
    /// epic visibility).
    pub(in crate::tui) fn reparent_target_epics(&self, target: EpicId) -> Vec<&Epic> {
        let excluded = crate::models::descendant_epic_ids(target, &self.board.epics);
        self.board
            .epics
            .iter()
            .filter(|e| {
                !excluded.contains(&e.id)
                    && !matches!(e.status, TaskStatus::Done | TaskStatus::Archived)
                    && self.epic_matches(e.id)
                    && self.epic_repo_matches(e.id)
            })
            .collect()
    }

    /// Epics eligible as move-to-epic targets for a task.
    ///
    /// Unlike [`Self::reparent_target_epics`], there is no descendant exclusion
    /// (a task can never be an ancestor of an epic, so no cycle is possible).
    /// Excludes epics in `Done`/`Archived` status and epics hidden by the
    /// active repo / only-active filters, using the same visibility predicates
    /// the board uses.
    pub(in crate::tui) fn move_task_target_epics(&self) -> Vec<&Epic> {
        self.board
            .epics
            .iter()
            .filter(|e| {
                !matches!(e.status, TaskStatus::Done | TaskStatus::Archived)
                    && self.epic_matches(e.id)
                    && self.epic_repo_matches(e.id)
            })
            .collect()
    }

    /// True when a title-search query is active (non-empty).
    pub(in crate::tui) fn search_active(&self) -> bool {
        !self.search.query.is_empty()
    }

    /// Whether flattened mode applies to `status`. Backlog is excluded from
    /// flattening so that epic cards remain visible in the backlog column.
    fn is_flattened_for_status(&self, status: TaskStatus) -> bool {
        self.board.flattened && status != TaskStatus::Backlog
    }

    /// Return tasks visible in the current view.
    /// Board view: standalone tasks only (epic_id is None).
    /// Epic view: only subtasks of the active epic.
    pub fn tasks_for_current_view(&self) -> Vec<&Task> {
        let repo_match = |t: &&Task| self.repo_matches(&t.repo_path);
        let active_match = |t: &&Task| self.filter.task_matches(t);
        let query_lower = self.search.query.to_lowercase();
        let search_match = |t: &&Task| fuzzy_matches_lower(&t.title, &query_lower);
        match self.effective_view_mode() {
            ViewMode::Board(_) => self
                .board
                .tasks
                .iter()
                .filter(|t| {
                    t.status != TaskStatus::Archived
                        && (self.is_flattened_for_status(t.status) || t.epic_id.is_none())
                })
                .filter(repo_match)
                .filter(active_match)
                .filter(search_match)
                .collect(),
            ViewMode::Epic { epic_id, .. } => {
                let current = *epic_id;
                if self.board.flattened {
                    let subtree = crate::models::descendant_task_ids(
                        current,
                        &self.board.epics,
                        &self.board.tasks,
                    );
                    self.board
                        .tasks
                        .iter()
                        .filter(|t| {
                            t.status != TaskStatus::Archived
                                && if self.is_flattened_for_status(t.status) {
                                    subtree.contains(&t.id)
                                } else {
                                    // Backlog excluded from flattening: only direct children
                                    t.epic_id == Some(current)
                                }
                        })
                        .filter(repo_match)
                        .filter(active_match)
                        .filter(search_match)
                        .collect()
                } else {
                    self.board
                        .tasks
                        .iter()
                        .filter(|t| t.epic_id == Some(current) && t.status != TaskStatus::Archived)
                        .filter(repo_match)
                        .filter(active_match)
                        .filter(search_match)
                        .collect()
                }
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } | ViewMode::Todos { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail, Learnings, or Todos")
            }
        }
    }

    /// Return tasks for a given status in the current view.
    pub fn tasks_by_status(&self, status: TaskStatus) -> Vec<&Task> {
        self.tasks_for_current_view()
            .into_iter()
            .filter(|t| t.status == status)
            .collect()
    }

    /// Return all archived tasks, ordered as they appear in self.board.tasks.
    pub fn archived_tasks(&self) -> Vec<&Task> {
        self.board
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Archived)
            .filter(|t| self.repo_matches(&t.repo_path))
            .collect()
    }

    /// Return all archived epics, ordered as they appear in self.board.epics.
    pub fn archived_epics(&self) -> Vec<&Epic> {
        self.board
            .epics
            .iter()
            .filter(|e| e.status == TaskStatus::Archived)
            .collect()
    }

    /// Pre-compute subtask stats for all epics using a pre-built children map.
    /// The `children_map` argument avoids rebuilding the adjacency map per epic.
    fn compute_epic_stats_with_map(
        &self,
        children_map: &HashMap<EpicId, Vec<EpicId>>,
    ) -> EpicStatsMap {
        let active_merge = self.merge_queue.as_ref().map(|q| q.epic_id);
        self.board
            .epics
            .iter()
            .map(|e| {
                (
                    e.id,
                    SubtaskStats::for_epic(e, &self.board.tasks, children_map, active_merge),
                )
            })
            .collect()
    }

    /// Pre-compute subtask stats for all epics. Call once per render frame.
    pub fn compute_epic_stats(&self) -> EpicStatsMap {
        // Build the parent→children map once so each for_epic call is O(depth)
        // rather than O(epics) — total cost goes from O(epics²) to O(epics).
        let children_map = crate::models::build_children_map(&self.board.epics);
        self.compute_epic_stats_with_map(&children_map)
    }

    /// Return an `Arc`-wrapped `EpicStatsMap`, computing and caching on first call.
    ///
    /// Cloning the returned `Arc` is O(1) (atomic ref-count); the underlying
    /// `HashMap` is not copied.  Also populates `children_map_cache`,
    /// `column_anchor_cache`, and `epic_filter_cache` on first call so that
    /// rendering and navigation handlers can do O(1) lookups without re-scanning.
    ///
    /// Call `invalidate_layout_cache()` whenever `board.tasks` or `board.epics`
    /// are mutated to force a fresh computation on the next call.
    pub(in crate::tui) fn cached_epic_stats(&mut self) -> Arc<EpicStatsMap> {
        if self.epic_stats_cache.is_none() {
            // Build the children map once; store it so callers can reuse it.
            let children_map = crate::models::build_children_map(&self.board.epics);
            let stats = Arc::new(self.compute_epic_stats_with_map(&children_map));

            // Build epic_filter_cache: (epic_repo_matches, epic_matches) per epic,
            // using the already-built children_map so descendant traversal is O(1) per epic.
            // Computed before children_map is moved into children_map_cache.
            let filter_cache: HashMap<EpicId, (bool, bool)> = {
                let tasks = &self.board.tasks;
                let filter = &self.filter;
                self.board
                    .epics
                    .iter()
                    .map(|e| {
                        let epic_ids =
                            crate::models::descendant_epic_ids_with_map(e.id, &children_map);
                        let repo_matches = if filter.repos.is_empty() {
                            true
                        } else {
                            let has_active = tasks.iter().any(|t| {
                                matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid))
                                    && t.status != TaskStatus::Archived
                            });
                            if !has_active {
                                true
                            } else {
                                tasks.iter().any(|t| {
                                    matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid))
                                        && t.status != TaskStatus::Archived
                                        && filter.matches(&t.repo_path)
                                })
                            }
                        };
                        let active_matches = if !filter.only_active {
                            true
                        } else {
                            tasks.iter().any(|t| {
                                matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid))
                                    && t.tmux_window.is_some()
                            })
                        };
                        (e.id, (repo_matches, active_matches))
                    })
                    .collect()
            };
            self.epic_filter_cache = Some(filter_cache);
            self.children_map_cache = Some(children_map);

            // Build column_anchor_cache: sorted selectable items per status.
            // Hoist tasks_for_current_view() out of the loop so it's computed once,
            // not once per status.
            let view_tasks = self.tasks_for_current_view();
            let mut anchor_cache: HashMap<TaskStatus, Vec<ColumnAnchor>> = HashMap::new();
            for &status in TaskStatus::ALL.iter() {
                let anchors: Vec<ColumnAnchor> = self
                    .column_items_for_status_with_view_tasks(status, Some(&*stats), &view_tasks)
                    .into_iter()
                    .filter(|i| i.is_selectable())
                    .map(|item| match item {
                        ColumnItem::Task(t) => ColumnAnchor::Task(t.id),
                        ColumnItem::Epic(e) => ColumnAnchor::Epic(e.id),
                        ColumnItem::EpicHeader(_)
                        | ColumnItem::SubstatusLabel(_)
                        | ColumnItem::OrphanSeparator => {
                            unreachable!("is_selectable filters these out")
                        }
                    })
                    .collect();
                anchor_cache.insert(status, anchors);
            }
            self.column_anchor_cache = Some(anchor_cache);

            self.epic_stats_cache = Some(Arc::clone(&stats));
            return stats;
        }
        if let Some(ref arc) = self.epic_stats_cache {
            Arc::clone(arc)
        } else {
            unreachable!("epic_stats_cache is set in the branch above")
        }
    }

    /// Discard all layout caches so the next `cached_epic_stats()` call
    /// recomputes from the current board state.  Every handler that mutates
    /// `board.tasks` or `board.epics` must call this (directly or via
    /// `sync_board_selection`).
    pub(in crate::tui) fn invalidate_layout_cache(&mut self) {
        self.epic_stats_cache = None;
        self.children_map_cache = None;
        self.column_anchor_cache = None;
        self.epic_filter_cache = None;
        self.task_index = None;
    }

    /// Build a list of items (tasks + epics) for a column in the current view.
    /// In board view, epics are included (positioned by derived status).
    /// In epic view, only subtasks are included (no epic cards).
    ///
    /// Passes `stats = None`: in non-flat mode with epics, epic sort order is derived
    /// by cloning all non-archived subtasks per epic. Prefer
    /// [`Self::column_items_for_status_with_stats`] with pre-computed stats whenever
    /// `compute_epic_stats()` can be called at the same site.
    pub fn column_items_for_status(&self, status: TaskStatus) -> Vec<ColumnItem<'_>> {
        self.column_items_for_status_with_stats(status, None)
    }

    /// Like `column_items_for_status` but uses pre-computed epic stats for sorting.
    ///
    /// Shares the epic filter+sort pipeline with [`Self::column_items_for_visual_column`]:
    /// both apply view-mode epic filtering and sort by `(sort_order, id)`. This method
    /// handles the flat-board layout; the visual-column path handles split-pane layout.
    pub fn column_items_for_status_with_stats<'a>(
        &'a self,
        status: TaskStatus,
        stats: Option<&EpicStatsMap>,
    ) -> Vec<ColumnItem<'a>> {
        let view_tasks = self.tasks_for_current_view();
        self.column_items_for_status_with_view_tasks(status, stats, &view_tasks)
    }

    /// Like `column_items_for_status_with_stats` but accepts pre-computed view tasks,
    /// allowing `tasks_for_current_view()` to be called once and reused across all
    /// columns (e.g. in `ColumnLayout::build`).
    pub(in crate::tui) fn column_items_for_status_with_view_tasks<'a>(
        &'a self,
        status: TaskStatus,
        stats: Option<&EpicStatsMap>,
        view_tasks: &[&'a Task],
    ) -> Vec<ColumnItem<'a>> {
        let tasks: Vec<&'a Task> = view_tasks
            .iter()
            .filter(|t| t.status == status)
            .copied()
            .collect();

        if self.is_flattened_for_status(status) {
            let epic_lookup: HashMap<EpicId, &Epic> =
                self.board.epics.iter().map(|e| (e.id, e)).collect();

            // SubstatusLabel items only make sense in Running/Review columns.
            let show_substatus_labels = matches!(status, TaskStatus::Running | TaskStatus::Review);

            // Sort: (substatus_priority, epic_sort_key, task_sort_key, task_id).
            // Orphan tasks (epic not in board) sort last within each substatus group.
            let mut sorted_tasks = tasks;
            sorted_tasks.sort_by_key(|t| {
                let priority = t.sub_status.column_priority_detached(t.is_detached());
                let epic_sk = match t.epic_id.and_then(|eid| epic_lookup.get(&eid)) {
                    Some(e) => e.sort_order.unwrap_or(e.id.0),
                    None => i64::MAX,
                };
                (priority, epic_sk, t.sort_order.unwrap_or(t.id.0), t.id.0)
            });

            // Single pass: emit SubstatusLabel on priority change (Running/Review only),
            // EpicHeader when (priority, epic_id) changes, then the task itself.
            // Tasks are sorted so all items in the same (priority, epic) group are
            // contiguous — no HashSet needed, just track the last-seen pair.
            let mut items: Vec<ColumnItem<'_>> = Vec::new();
            let mut current_priority: Option<u8> = None;
            let mut current_epic_id: Option<EpicId> = None;

            for t in sorted_tasks {
                let detached = t.is_detached();
                let priority = t.sub_status.column_priority_detached(detached);
                let priority_changed = Some(priority) != current_priority;
                if priority_changed {
                    current_priority = Some(priority);
                    current_epic_id = None;
                    if show_substatus_labels {
                        items.push(ColumnItem::SubstatusLabel(
                            t.sub_status.header_label_detached(detached),
                        ));
                    }
                }

                // Emit OrphanSeparator when transitioning from an epic group to no-epic tasks.
                if t.epic_id.is_none() && current_epic_id.is_some() {
                    items.push(ColumnItem::OrphanSeparator);
                    current_epic_id = None;
                }

                if let Some(eid) = t.epic_id {
                    if let Some(&epic) = epic_lookup.get(&eid) {
                        if Some(eid) != current_epic_id {
                            current_epic_id = Some(eid);
                            items.push(ColumnItem::EpicHeader(epic));
                        }
                    }
                }

                items.push(ColumnItem::Task(t));
            }

            return items;
        }

        // --- Non-flat path (unchanged) ---
        let mut items: Vec<ColumnItem<'_>> = tasks.into_iter().map(ColumnItem::Task).collect();

        match self.effective_view_mode() {
            ViewMode::Board(_) => {
                // Main board: show only root epics (no parent)
                for epic in &self.board.epics {
                    if epic.parent_epic_id.is_some() {
                        continue;
                    }
                    if !self.epic_matches(epic.id) || !self.epic_repo_matches(epic.id) {
                        continue;
                    }
                    if epic.status == status {
                        items.push(ColumnItem::Epic(epic));
                    }
                }
            }
            ViewMode::Epic { epic_id, .. } => {
                // Inside an epic: show sub-epics whose parent_epic_id matches
                let current = *epic_id;
                for epic in &self.board.epics {
                    if epic.parent_epic_id != Some(current) {
                        continue;
                    }
                    if !self.epic_matches(epic.id) || !self.epic_repo_matches(epic.id) {
                        continue;
                    }
                    if epic.status == status {
                        items.push(ColumnItem::Epic(epic));
                    }
                }
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } | ViewMode::Todos { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail, Learnings, or Todos")
            }
        }

        items.sort_by_key(|item| match item {
            ColumnItem::Task(t) => (
                t.sub_status.column_priority_detached(t.is_detached()),
                t.sort_order.unwrap_or(t.id.0),
                t.id.0,
            ),
            ColumnItem::Epic(e) => {
                let priority = if let Some(s) = stats.and_then(|m| m.get(&e.id)) {
                    s.substatus.column_priority()
                } else {
                    let subtasks: Vec<&Task> = self
                        .board
                        .tasks
                        .iter()
                        .filter(|t| t.epic_id == Some(e.id) && t.status != TaskStatus::Archived)
                        .collect();
                    let active_merge = self.merge_queue.as_ref().map(|q| q.epic_id);
                    epic_substatus(e, &subtasks, active_merge).column_priority()
                };
                (priority, e.sort_order.unwrap_or(e.id.0), e.id.0)
            }
            ColumnItem::EpicHeader(_) | ColumnItem::OrphanSeparator => {
                unreachable!("EpicHeader/OrphanSeparator never produced in non-flat mode")
            }
            ColumnItem::SubstatusLabel(_) => {
                unreachable!("SubstatusLabel never produced in non-flat mode")
            }
        });

        items
    }

    /// Count selectable column items (tasks + epics) for a status without sorting or
    /// allocating the full item list. Use this wherever only a count is needed —
    /// navigation bounds, clamp guards — rather than calling
    /// `column_items_for_status(s).len()`, which includes non-selectable decorators
    /// (`EpicHeader`, `SubstatusLabel`, `OrphanSeparator`) in flat mode and is O(n log n).
    /// Used by `clamp_selection()` and `handle_navigate_row()`.
    pub(in crate::tui) fn column_item_count(&self, status: TaskStatus) -> usize {
        let task_count = self.tasks_by_status(status).len();
        if self.is_flattened_for_status(status) {
            return task_count;
        }
        let epic_count = match self.effective_view_mode() {
            ViewMode::Board(_) => self
                .board
                .epics
                .iter()
                .filter(|e| {
                    e.parent_epic_id.is_none()
                        && e.status == status
                        && self.epic_matches(e.id)
                        && self.epic_repo_matches(e.id)
                })
                .count(),
            ViewMode::Epic { epic_id, .. } => {
                let current = *epic_id;
                self.board
                    .epics
                    .iter()
                    .filter(|e| {
                        e.parent_epic_id == Some(current)
                            && e.status == status
                            && self.epic_matches(e.id)
                            && self.epic_repo_matches(e.id)
                    })
                    .count()
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } | ViewMode::Todos { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail, Learnings, or Todos")
            }
        };
        task_count + epic_count
    }

    /// Build a list of items (tasks + epics) for a visual column.
    /// Tasks are filtered by parent_status and sub_status matching the visual column.
    /// Running epics are placed in Active or Blocked based on their substatus;
    /// other epics appear in the first visual column of their parent status group.
    ///
    /// Shares the epic filter+sort pipeline with [`Self::column_items_for_status_with_stats`]:
    /// both apply view-mode epic filtering and sort by `(sort_order, id)`. This method
    /// handles the split-pane layout; the status-based path handles the flat-board layout.
    pub fn column_items_for_visual_column(&self, vcol_idx: usize) -> Vec<ColumnItem<'_>> {
        let vcol = &VisualColumn::ALL[vcol_idx];
        let tasks: Vec<&Task> = self
            .tasks_for_current_view()
            .into_iter()
            .filter(|t| t.status == vcol.parent_status && vcol.contains(t.sub_status))
            .collect();

        let mut items: Vec<ColumnItem<'_>> = tasks.into_iter().map(ColumnItem::Task).collect();

        let epics_to_show: Vec<&Epic> = match self.effective_view_mode() {
            ViewMode::Board(_) => self
                .board
                .epics
                .iter()
                .filter(|e| e.parent_epic_id.is_none() && self.epic_matches(e.id))
                .collect(),
            ViewMode::Epic { epic_id, .. } => {
                let current = *epic_id;
                self.board
                    .epics
                    .iter()
                    .filter(|e| e.parent_epic_id == Some(current) && self.epic_matches(e.id))
                    .collect()
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } | ViewMode::Todos { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail, Learnings, or Todos")
            }
        };

        if !epics_to_show.is_empty() {
            let active_merge = self.merge_queue.as_ref().map(|q| q.epic_id);
            for epic in epics_to_show {
                let epic_parent = epic.status;
                if epic_parent != vcol.parent_status {
                    continue;
                }
                if epic_parent == TaskStatus::Running {
                    let subtasks: Vec<&Task> = self
                        .board
                        .tasks
                        .iter()
                        .filter(|t| t.epic_id == Some(epic.id) && t.status != TaskStatus::Archived)
                        .collect();
                    let substatus = epic_substatus(epic, &subtasks, active_merge);
                    let target_col = if matches!(substatus, EpicSubstatus::Blocked(_)) {
                        2
                    } else {
                        1
                    };
                    if vcol_idx == target_col {
                        items.push(ColumnItem::Epic(epic));
                    }
                } else if vcol_idx == VisualColumn::parent_group_start(epic_parent) {
                    items.push(ColumnItem::Epic(epic));
                }
            }
        }

        items.sort_by_key(|item| {
            let (sort_order, id) = match item {
                ColumnItem::Task(t) => (t.sort_order, t.id.0),
                ColumnItem::Epic(e) => (e.sort_order, e.id.0),
                ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) | ColumnItem::OrphanSeparator => {
                    unreachable!("EpicHeader/SubstatusLabel/OrphanSeparator never produced by column_items_for_visual_column")
                }
            };
            (sort_order.unwrap_or(i64::MAX), id)
        });
        items
    }

    /// Get the statuses of all subtasks belonging to an epic.
    pub(in crate::tui) fn subtask_statuses(&self, epic_id: EpicId) -> Vec<TaskStatus> {
        self.board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(epic_id) && t.status != TaskStatus::Archived)
            .map(|t| t.status)
            .collect()
    }

    /// Return the item (task or epic) currently under the cursor.
    ///
    /// Uses the cached `EpicStatsMap` when available (avoids the O(subtasks)
    /// clone that `column_items_for_status` incurs with `stats=None`).
    pub fn selected_column_item(&self) -> Option<ColumnItem<'_>> {
        if self.selection().on_select_all {
            return None;
        }
        let col = self.selection().column();
        if col == 0 || is_edge_column(col) {
            return None;
        }
        let status = TaskStatus::from_column_index(col - 1)?;
        let items = self.column_items_for_status_with_stats(status, self.epic_stats_cache.as_deref());
        let row = self.selection().row(col);
        items.into_iter().filter(|i| i.is_selectable()).nth(row)
    }

    /// Look up the title of an epic by ID.
    pub fn epic_title(&self, id: EpicId) -> Option<&str> {
        self.board
            .epics
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.title.as_str())
    }

    /// Return the currently selected task (if the cursor is on a task), or None
    /// if the cursor is on an epic or the column is empty.
    pub fn selected_task(&self) -> Option<&Task> {
        match self.selected_column_item() {
            Some(ColumnItem::Task(task)) => Some(task),
            _ => None,
        }
    }

    /// Clamp all selected_row values to be within bounds for each column.
    pub fn clamp_selection(&mut self) {
        for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
            let nav_col = idx + 1;
            let count = self.column_item_count(status);
            let sel = self.selection_mut();
            if count == 0 {
                sel.set_row(nav_col, 0);
            } else if sel.row(nav_col) >= count {
                sel.set_row(nav_col, count - 1);
            }
        }
    }

    /// Set the selection anchor to the item currently under the cursor.
    /// Called after every navigation keystroke so that subsequent data refreshes
    /// can restore the cursor to this item.
    /// Sets anchor to None when the cursor is on the select-all header.
    ///
    /// Warms the layout cache if needed, then reads from `column_anchor_cache`
    /// in O(1).
    pub(in crate::tui) fn update_anchor_from_current(&mut self) {
        let on_select_all = self.selection().on_select_all;
        if on_select_all {
            self.selection_mut().anchor = None;
            return;
        }
        let col = self.selection().column();
        if col == 0 || col > TaskStatus::COLUMN_COUNT {
            return;
        }
        let row = self.selection().row(col);
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return;
        };

        let _ = self.cached_epic_stats(); // warms column_anchor_cache if cold
        let new_anchor = self
            .column_anchor_cache
            .as_ref()
            .and_then(|m| m.get(&status))
            .and_then(|v| v.get(row))
            .copied();
        self.selection_mut().anchor = new_anchor;
    }

    /// Restore cursor position from the anchor after a data change.
    /// Scans all columns for the anchor item and moves the cursor to its new
    /// position (following it across columns if needed).
    /// Falls back to index clamping if the anchor is not found.
    pub fn sync_board_selection(&mut self) {
        // Board data has changed; discard stale stats and recompute below.
        self.invalidate_layout_cache();

        let current_col = self.selection().column();

        // If the cursor is on the Archive edge column, preserve the column and only clamp rows.
        if current_col == TaskStatus::COLUMN_COUNT + 1 {
            self.clamp_selection();
            let count = self.archived_tasks().len();
            let archive_col = TaskStatus::COLUMN_COUNT + 1;
            let row = self.selection().row(archive_col);
            let clamped = if count == 0 { 0 } else { row.min(count - 1) };
            self.selection_mut().set_row(archive_col, clamped);
            self.archive.list_state.select(Some(clamped));
            return;
        }

        let anchor = match self.effective_view_mode() {
            ViewMode::Board(sel) | ViewMode::Epic { selection: sel, .. } => sel.anchor,
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } | ViewMode::Todos { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail, Learnings, or Todos")
            }
        };

        let Some(anchor) = anchor else {
            // on_select_all or no anchor set yet — just clamp
            return self.clamp_selection();
        };

        // Rebuild all layout caches for the fresh board state.
        let _ = self.cached_epic_stats();
        // Search for the anchor in the pre-sorted anchor cache (avoids re-sorting each column).
        let mut found: Option<(usize, usize)> = None;
        if let Some(anchor_map) = &self.column_anchor_cache {
            'outer: for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
                let nav_col = idx + 1;
                if let Some(anchors) = anchor_map.get(&status) {
                    for (row, &item_anchor) in anchors.iter().enumerate() {
                        if item_anchor == anchor {
                            found = Some((nav_col, row));
                            break 'outer;
                        }
                    }
                }
            }
        }

        if let Some((found_col, found_row)) = found {
            for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
                let nav_col = idx + 1;
                if nav_col == found_col {
                    continue;
                }
                let count = self.column_item_count(status);
                let sel = self.selection_mut();
                if count == 0 {
                    sel.set_row(nav_col, 0);
                } else if sel.row(nav_col) >= count {
                    sel.set_row(nav_col, count - 1);
                }
            }
            let sel = self.selection_mut();
            sel.set_column(found_col);
            sel.set_row(found_col, found_row);
            sel.on_select_all = false;
        } else {
            self.clamp_selection();
        }
    }

    pub(in crate::tui) fn reset_column_scroll(&mut self) {
        for state in &mut self.selection_mut().list_states {
            *state.offset_mut() = 0;
        }
    }

    pub(in crate::tui) fn find_task(&self, id: TaskId) -> Option<&Task> {
        // Use cached index for O(1) lookup when available and not stale.
        if let Some(ref idx) = self.task_index {
            if idx.len() == self.board.tasks.len() {
                return idx.get(&id).and_then(|&i| self.board.tasks.get(i));
            }
        }
        self.board.tasks.iter().find(|t| t.id == id)
    }

    pub(in crate::tui) fn find_task_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        // Rebuild index if missing or if length mismatches (task inserted/removed without
        // going through invalidate_layout_cache — e.g. a direct board.tasks mutation in tests).
        let stale = self
            .task_index
            .as_ref()
            .is_none_or(|idx| idx.len() != self.board.tasks.len());
        if stale {
            self.task_index = Some(
                self.board
                    .tasks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (t.id, i))
                    .collect(),
            );
        }
        let i = self.task_index.as_ref()?.get(&id).copied()?;
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
    /// Returns `None` if the task has no worktree (still clears tmux_window).
    pub(in crate::tui) fn take_cleanup(task: &mut Task) -> Option<Command> {
        match task.worktree.take() {
            Some(wt) => Some(Command::Task(crate::tui::commands::TaskCommand::Cleanup {
                id: task.id,
                repo_path: task.repo_path.clone(),
                worktree: wt,
                tmux_window: task.tmux_window.take(),
            })),
            None => {
                task.tmux_window.take();
                None
            }
        }
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
        self.set_status(msg);
        vec![]
    }

    pub(in crate::tui) fn handle_confirm_detach_tmux(&mut self) -> Vec<Command> {
        let InputMode::ConfirmDetachTmux(ref ids) = self.input.mode else {
            return vec![];
        };
        let ids = ids.clone();
        self.input.mode = InputMode::Normal;
        self.clear_status();
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
                let task_clone = task.clone();
                cmds.push(Command::Task(crate::tui::commands::TaskCommand::Persist(
                    task_clone,
                )));
            }
        }
        cmds
    }

    pub(in crate::tui) fn finish_epic_creation(&mut self) -> Vec<Command> {
        let draft = self.input.epic_draft.take().unwrap_or_default();
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![Command::Epic(crate::tui::commands::EpicCommand::Insert(
            draft,
        ))]
    }
}

#[cfg(test)]
mod tests;
