pub mod input;
pub mod types;
pub mod ui;
pub mod update;

pub use types::*;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::models::ReviewDecision;
use crate::models::{
    epic_substatus, Epic, EpicId, EpicSubstatus, Project, ProjectId, SubStatus, Task, TaskId,
    TaskStatus, VisualColumn,
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

/// Returns true for the two edge navigation columns (Projects=0 and Archive=5) that
/// don't hold regular task data and must be excluded from task-operation hotkeys.
pub(in crate::tui) fn is_edge_column(col: usize) -> bool {
    col == 0 || col == TaskStatus::COLUMN_COUNT + 1
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
    pub(in crate::tui) projects_panel: ProjectsPanelState,
    pub(in crate::tui) active_project: ProjectId,
    pub(in crate::tui) active_is_default: bool,
    pub(in crate::tui) select: SelectionState,
    pub(in crate::tui) filter: FilterState,
    /// Snapshot of (repos, mode) for projects other than the active one.
    /// On project switch, the outgoing project's tuple is stashed here and
    /// the incoming project's tuple is restored into `filter`. Only `repos`
    /// and `mode` swap — presets stay on `filter` and are global.
    pub(in crate::tui) per_project_filter: HashMap<ProjectId, (HashSet<String>, RepoFilterMode)>,
    pub(in crate::tui) merge_queue: Option<MergeQueue>,
    /// Task IDs with an in-flight dispatch, mapped to their start time.
    /// Membership prevents duplicate dispatches; start times drive the 60-second watchdog.
    pub(in crate::tui) dispatching: HashMap<TaskId, Instant>,
    /// Spinner frame index (0..DISPATCH_SPINNER_FRAMES) for the per-card "dispatching…" indicator.
    /// Advanced by `Tick` only while `dispatching` is non-empty.
    pub(in crate::tui) spinner_tick: u8,
    pub(in crate::tui) tips: Option<TipsOverlayState>,
    pub(in crate::tui) main_session: Option<String>,
    pub(in crate::tui) main_session_dir: Option<String>,
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

/// Returns true if every character in `query` appears in `path` as a
/// forward subsequence (case-insensitive). An empty query matches everything.
pub(in crate::tui) fn fuzzy_matches(path: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let path_lower = path.to_lowercase();
    let mut path_chars = path_lower.chars();
    let query_lower = query.to_lowercase();
    for qc in query_lower.chars() {
        if !path_chars.any(|pc| pc == qc) {
            return false;
        }
    }
    true
}

/// Returns the subset of `paths` that fuzzy-match `query`, preserving order.
pub(in crate::tui) fn filtered_repos(paths: &[String], query: &str) -> Vec<String> {
    paths
        .iter()
        .filter(|p| fuzzy_matches(p, query))
        .cloned()
        .collect()
}

impl App {
    pub fn new(
        tasks: Vec<Task>,
        default_project_id: ProjectId,
        inactivity_timeout: Duration,
    ) -> Self {
        let mut app = App {
            board: BoardState {
                tasks,
                epics: Vec::new(),
                projects: Vec::new(),
                view_mode: ViewMode::default(),
                repo_paths: Vec::new(),
                usage: HashMap::new(),
                split: SplitState::default(),
                flattened: false,
            },
            status: StatusState::default(),
            should_quit: false,
            notifications_enabled: false,
            input: InputState::default(),
            agents: AgentTracking::new(inactivity_timeout),
            archive: ArchiveState::default(),
            projects_panel: ProjectsPanelState::default(),
            active_project: default_project_id,
            active_is_default: false,
            select: SelectionState::default(),
            filter: FilterState::default(),
            per_project_filter: HashMap::new(),
            merge_queue: None,
            dispatching: HashMap::new(),
            spinner_tick: 0,
            tips: None,
            main_session: None,
            main_session_dir: None,
        };
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
    pub fn tmux_outputs(&self) -> &std::collections::HashMap<TaskId, String> {
        &self.agents.tmux_outputs
    }
    pub fn status_message(&self) -> Option<&str> {
        self.status.message.as_deref()
    }
    pub fn error_popup(&self) -> Option<&str> {
        self.status.error_popup.as_deref()
    }
    pub fn last_error(&self, id: TaskId) -> Option<&str> {
        self.agents.last_error.get(&id).map(|s| s.as_str())
    }
    pub fn repo_paths(&self) -> &[String] {
        &self.board.repo_paths
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
    pub fn inactivity_timeout(&self) -> Duration {
        self.agents.inactivity_timeout
    }
    pub fn show_archived(&self) -> bool {
        self.selection().column() == TaskStatus::COLUMN_COUNT + 1
    }
    pub fn selected_archive_row(&self) -> usize {
        self.selection().row(TaskStatus::COLUMN_COUNT + 1)
    }
    pub fn active_project(&self) -> ProjectId {
        self.active_project
    }
    pub fn projects(&self) -> &[Project] {
        &self.board.projects
    }
    pub fn projects_panel_visible(&self) -> bool {
        self.selection().column() == 0
    }
    pub fn selected_project_row(&self) -> usize {
        self.selection().row(0)
    }
    pub(in crate::tui) fn selected_project(&self) -> Option<&Project> {
        self.board.projects.get(self.selection().row(0))
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

    /// Set of PR URLs from dispatch tasks (for matching against ReviewPr entries).
    pub fn dispatch_pr_urls(&self) -> HashSet<String> {
        self.board
            .tasks
            .iter()
            .filter_map(|t| t.pr_url.clone())
            .collect()
    }

    pub fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
    }

    pub fn main_session(&self) -> Option<&str> {
        self.main_session.as_deref()
    }

    pub fn main_session_dir(&self) -> Option<&str> {
        self.main_session_dir.as_deref()
    }

    pub fn set_main_session(&mut self, window: Option<String>) {
        self.main_session = window;
    }

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

    /// Insert a saved filter snapshot for `project_id` into the per-project
    /// map. Used by the runtime loader to restore per-project filters at
    /// startup.
    pub(crate) fn set_per_project_filter(
        &mut self,
        project_id: ProjectId,
        repos: HashSet<String>,
        mode: RepoFilterMode,
    ) {
        self.per_project_filter.insert(project_id, (repos, mode));
    }

    pub(crate) fn has_per_project_filter(&self, project_id: ProjectId) -> bool {
        self.per_project_filter.contains_key(&project_id)
    }

    /// Copy the active project's saved filter from `per_project_filter` into
    /// `self.filter`. No-op if there is no entry for the active project.
    pub(crate) fn activate_filter_for_active_project(&mut self) {
        if let Some((repos, mode)) = self.per_project_filter.get(&self.active_project) {
            self.filter.repos = repos.clone();
            self.filter.mode = *mode;
            self.sync_board_selection();
        }
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

    pub(in crate::tui) fn project_matches(&self, project_id: ProjectId) -> bool {
        self.active_is_default || project_id == self.active_project
    }

    /// Return tasks visible in the current view.
    /// Board view: standalone tasks only (epic_id is None).
    /// Epic view: only subtasks of the active epic.
    pub fn tasks_for_current_view(&self) -> Vec<&Task> {
        let repo_match = |t: &&Task| self.repo_matches(&t.repo_path);
        let project_match = |t: &&Task| self.project_matches(t.project_id);
        match self.effective_view_mode() {
            ViewMode::Board(_) => self
                .board
                .tasks
                .iter()
                .filter(|t| {
                    t.status != TaskStatus::Archived
                        && (self.board.flattened || t.epic_id.is_none())
                })
                .filter(repo_match)
                .filter(project_match)
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
                        .filter(|t| subtree.contains(&t.id) && t.status != TaskStatus::Archived)
                        .filter(repo_match)
                        .filter(project_match)
                        .collect()
                } else {
                    self.board
                        .tasks
                        .iter()
                        .filter(|t| t.epic_id == Some(current) && t.status != TaskStatus::Archived)
                        .filter(repo_match)
                        .filter(project_match)
                        .collect()
                }
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail or Learnings")
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
            .filter(|t| self.project_matches(t.project_id))
            .collect()
    }

    /// Return all archived epics, ordered as they appear in self.board.epics.
    pub fn archived_epics(&self) -> Vec<&Epic> {
        self.board
            .epics
            .iter()
            .filter(|e| e.status == TaskStatus::Archived)
            .filter(|e| self.repo_matches(&e.repo_path))
            .filter(|e| self.project_matches(e.project_id))
            .collect()
    }

    /// Pre-compute subtask stats for all epics. Call once per render frame.
    pub fn compute_epic_stats(&self) -> EpicStatsMap {
        let active_merge = self.merge_queue.as_ref().map(|q| q.epic_id);
        self.board
            .epics
            .iter()
            .map(|e| {
                (
                    e.id,
                    SubtaskStats::for_epic(e, &self.board.tasks, active_merge),
                )
            })
            .collect()
    }

    /// Build a list of items (tasks + epics) for a column in the current view.
    /// In board view, epics are included (positioned by derived status).
    /// In epic view, only subtasks are included (no epic cards).
    ///
    /// **Test-only.** Passes `stats = None`, which causes epic sort order to be derived by
    /// cloning subtasks on every call. Use [`column_items_for_status_with_stats`] with
    /// pre-computed stats in production render paths to avoid per-frame allocations.
    pub fn column_items_for_status(&self, status: TaskStatus) -> Vec<ColumnItem<'_>> {
        self.column_items_for_status_with_stats(status, None)
    }

    /// Like `column_items_for_status` but uses pre-computed epic stats for sorting.
    pub fn column_items_for_status_with_stats<'a>(
        &'a self,
        status: TaskStatus,
        stats: Option<&EpicStatsMap>,
    ) -> Vec<ColumnItem<'a>> {
        let tasks = self.tasks_by_status(status);

        if self.board.flattened {
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
                    if !self.repo_matches(&epic.repo_path) {
                        continue;
                    }
                    if !self.project_matches(epic.project_id) {
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
                    if epic.status == status {
                        items.push(ColumnItem::Epic(epic));
                    }
                }
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail or Learnings")
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
                    let subtasks: Vec<Task> = self
                        .board
                        .tasks
                        .iter()
                        .filter(|t| t.epic_id == Some(e.id) && t.status != TaskStatus::Archived)
                        .cloned()
                        .collect();
                    let active_merge = self.merge_queue.as_ref().map(|q| q.epic_id);
                    epic_substatus(e, &subtasks, active_merge).column_priority()
                };
                (priority, e.sort_order.unwrap_or(e.id.0), e.id.0)
            }
            ColumnItem::EpicHeader(_) => {
                unreachable!("EpicHeader never produced in non-flat mode")
            }
            ColumnItem::SubstatusLabel(_) => {
                unreachable!("SubstatusLabel never produced in non-flat mode")
            }
        });

        items
    }

    /// Count column items for a status without sorting or allocating the full list.
    /// Used by `clamp_selection()` which only needs counts, not the sorted items.
    pub(in crate::tui) fn column_item_count(&self, status: TaskStatus) -> usize {
        let task_count = self.tasks_by_status(status).len();
        if self.board.flattened {
            return task_count;
        }
        let epic_count = match self.effective_view_mode() {
            ViewMode::Board(_) => self
                .board
                .epics
                .iter()
                .filter(|e| {
                    e.parent_epic_id.is_none()
                        && self.filter.matches(&e.repo_path)
                        && self.project_matches(e.project_id)
                        && e.status == status
                })
                .count(),
            ViewMode::Epic { epic_id, .. } => {
                let current = *epic_id;
                self.board
                    .epics
                    .iter()
                    .filter(|e| e.parent_epic_id == Some(current) && e.status == status)
                    .count()
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail or Learnings")
            }
        };
        task_count + epic_count
    }

    /// Build a list of items (tasks + epics) for a visual column.
    /// Tasks are filtered by parent_status and sub_status matching the visual column.
    /// Running epics are placed in Active or Blocked based on their substatus;
    /// other epics appear in the first visual column of their parent status group.
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
                .filter(|e| {
                    e.parent_epic_id.is_none()
                        && self.repo_matches(&e.repo_path)
                        && self.project_matches(e.project_id)
                })
                .collect(),
            ViewMode::Epic { epic_id, .. } => {
                let current = *epic_id;
                self.board
                    .epics
                    .iter()
                    .filter(|e| e.parent_epic_id == Some(current))
                    .collect()
            }
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail or Learnings")
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
                    let subtasks: Vec<Task> = self
                        .board
                        .tasks
                        .iter()
                        .filter(|t| t.epic_id == Some(epic.id) && t.status != TaskStatus::Archived)
                        .cloned()
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
                ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) => {
                    unreachable!("EpicHeader/SubstatusLabel never produced by column_items_for_visual_column")
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
    pub fn selected_column_item(&self) -> Option<ColumnItem<'_>> {
        if self.selection().on_select_all {
            return None;
        }
        let col = self.selection().column();
        if is_edge_column(col) {
            return None;
        }
        let status = TaskStatus::from_column_index(col - 1)?;
        let items = self.column_items_for_status(status);
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

    /// Move the projects-panel cursor to the row of `project_id`. No-op if
    /// the project is not in the list.
    pub(in crate::tui) fn sync_project_cursor(&mut self, project_id: ProjectId) {
        if let Some(idx) = self.board.projects.iter().position(|p| p.id == project_id) {
            self.selection_mut().set_row(0, idx);
            self.projects_panel.list_state.select(Some(idx));
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
    pub(in crate::tui) fn update_anchor_from_current(&mut self) {
        // Read immutable fields before taking the mutable borrow below.
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
        let status = match TaskStatus::from_column_index(col - 1) {
            Some(s) => s,
            None => return,
        };
        let new_anchor = self
            .column_items_for_status(status)
            .into_iter()
            .filter(|i| i.is_selectable())
            .nth(row)
            .map(|item| match item {
                ColumnItem::Task(t) => ColumnAnchor::Task(t.id),
                ColumnItem::Epic(e) => ColumnAnchor::Epic(e.id),
                ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) => unreachable!(),
            });
        self.selection_mut().anchor = new_anchor;
    }

    /// Restore cursor position from the anchor after a data change.
    /// Scans all columns for the anchor item and moves the cursor to its new
    /// position (following it across columns if needed).
    /// Falls back to index clamping if the anchor is not found.
    pub fn sync_board_selection(&mut self) {
        let current_col = self.selection().column();

        // If the cursor is on an edge column (Projects=0 or Archive=COLUMN_COUNT+1),
        // preserve the column and only clamp rows — don't jump to the task anchor.
        if current_col == 0 || current_col == TaskStatus::COLUMN_COUNT + 1 {
            self.clamp_selection();
            if current_col == 0 {
                let len = self.board.projects.len();
                let row = self.selection().row(0);
                let clamped = if len == 0 { 0 } else { row.min(len - 1) };
                self.selection_mut().set_row(0, clamped);
                self.projects_panel.list_state.select(Some(clamped));
            } else {
                let count = self.archived_tasks().len();
                let archive_col = TaskStatus::COLUMN_COUNT + 1;
                let row = self.selection().row(archive_col);
                let clamped = if count == 0 { 0 } else { row.min(count - 1) };
                self.selection_mut().set_row(archive_col, clamped);
                self.archive.list_state.select(Some(clamped));
            }
            return;
        }

        let anchor = match self.effective_view_mode() {
            ViewMode::Board(sel) | ViewMode::Epic { selection: sel, .. } => sel.anchor,
            ViewMode::TaskDetail { .. } | ViewMode::Learnings { .. } => {
                unreachable!("effective_view_mode never returns TaskDetail or Learnings")
            }
        };

        let Some(anchor) = anchor else {
            // on_select_all or no anchor set yet — just clamp
            return self.clamp_selection();
        };

        let stats = self.compute_epic_stats();
        let mut found: Option<(usize, usize)> = None;
        'outer: for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
            let nav_col = idx + 1;
            let items = self.column_items_for_status_with_stats(status, Some(&stats));
            let mut selectable_row: usize = 0;
            for item in items.into_iter() {
                let item_anchor = match item {
                    ColumnItem::Task(t) => ColumnAnchor::Task(t.id),
                    ColumnItem::Epic(e) => ColumnAnchor::Epic(e.id),
                    ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) => continue,
                };
                if item_anchor == anchor {
                    found = Some((nav_col, selectable_row));
                    break 'outer;
                }
                selectable_row += 1;
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

    pub(in crate::tui) fn find_task(&self, id: TaskId) -> Option<&Task> {
        self.board.tasks.iter().find(|t| t.id == id)
    }

    pub(in crate::tui) fn find_task_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        self.board.tasks.iter_mut().find(|t| t.id == id)
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
            Some(wt) => Some(Command::Cleanup {
                id: task.id,
                repo_path: task.repo_path.clone(),
                worktree: wt,
                tmux_window: task.tmux_window.take(),
            }),
            None => {
                task.tmux_window.take();
                None
            }
        }
    }

    /// Take the tmux_window from a task and build a KillTmuxWindow command.
    /// Leaves the worktree intact so the task can be resumed later.
    pub(in crate::tui) fn take_detach(task: &mut Task) -> Option<Command> {
        task.tmux_window
            .take()
            .map(|window| Command::KillTmuxWindow { window })
    }

    /// Process a message and return a list of side-effect commands.
    pub fn update(&mut self, msg: Message) -> Vec<Command> {
        match msg {
            // ── Board navigation, view toggles, system events ──
            Message::Tick => self.handle_tick(),
            Message::TerminalResized => vec![],
            Message::Quit => self.handle_quit(),
            Message::NavigateColumn(delta) => self.handle_navigate_column(delta),
            Message::NavigateRow(delta) => self.handle_navigate_row(delta),
            Message::MoveTask { id, direction } => self.handle_move_task(id, direction),
            Message::ReorderItem(dir) => self.handle_reorder_item(dir),
            Message::OpenTaskDetail(task_id) => self.handle_open_task_detail(task_id),
            Message::CloseTaskDetail => self.handle_close_task_detail(),
            Message::ToggleFlattened => self.handle_toggle_flattened(),
            Message::ToggleHelp => self.handle_toggle_help(),
            Message::ToggleNotifications => self.handle_toggle_notifications(),
            Message::ToggleSplitMode => self.handle_toggle_split_mode(),
            Message::SwapSplitPane(task_id) => self.handle_swap_split_pane(task_id),
            Message::SplitPaneOpened { pane_id, task_id } => {
                self.handle_split_pane_opened(pane_id, task_id)
            }
            Message::SplitPaneClosed => self.handle_split_pane_closed(),
            Message::FocusChanged(focused) => self.handle_focus_changed(focused),
            Message::RefreshTasks(tasks) => self.handle_refresh_tasks(tasks),
            Message::RefreshUsage(usage) => self.handle_refresh_usage(usage),
            Message::Error(text) => self.handle_error(text),
            Message::DismissError => self.handle_dismiss_error(),
            Message::StatusInfo(text) => self.handle_status_info(text),
            Message::RepoPathsUpdated(paths) => self.handle_repo_paths_updated(paths),
            Message::MessageReceived(id) => self.handle_message_received(id),
            Message::OpenInBrowser { url } => self.handle_open_in_browser(url),
            Message::TmuxOutput {
                id,
                output,
                activity_ts,
            } => self.handle_tmux_output(id, output, activity_ts),
            Message::WindowGone(id) => self.handle_window_gone(id),

            // ── Task lifecycle, dispatch, selection, wrap-up ──
            Message::DispatchTask(id, mode) => self.handle_dispatch_task(id, mode),
            Message::Dispatched {
                id,
                worktree,
                tmux_window,
                switch_focus,
            } => self.handle_dispatched(id, worktree, tmux_window, switch_focus),
            Message::TaskCreated { task } => self.handle_task_created(task),
            Message::DeleteTask(id) => self.handle_delete_task(id),
            Message::ResumeTask(id) => self.handle_resume_task(id),
            Message::Resumed { id, tmux_window } => self.handle_resumed(id, tmux_window),
            Message::DispatchFailed(id) => self.handle_dispatch_failed(id),
            Message::MarkDispatching(id) => self.handle_mark_dispatching(id),
            Message::TaskEdited(edit) => self.handle_task_edited(edit),
            Message::StaleAgent(id) => self.handle_stale_agent(id),
            Message::AgentCrashed(id) => self.handle_agent_crashed(id),
            Message::KillAndRetry(id) => self.handle_kill_and_retry(id),
            Message::RetryResume(id) => self.handle_retry_resume(id),
            Message::RetryFresh(id) => self.handle_retry_fresh(id),
            Message::ArchiveTask(id) => self.handle_archive_task(id),
            Message::QuickDispatch { repo_path, epic_id } => {
                self.handle_quick_dispatch(repo_path, epic_id)
            }
            Message::StartQuickDispatchSelection => self.handle_start_quick_dispatch_selection(),
            Message::SelectQuickDispatchRepo(idx) => self.handle_select_quick_dispatch_repo(idx),
            Message::FinishComplete(id) => self.handle_finish_complete(id),
            Message::FinishFailed {
                id,
                error,
                is_conflict,
            } => self.handle_finish_failed(id, error, is_conflict),
            Message::ConfirmDone => self.handle_confirm_done(),
            Message::CancelDone => self.handle_cancel_done(),
            Message::StartWrapUp(id) => self.handle_start_wrap_up(id),
            Message::WrapUpRebase => self.handle_wrap_up_rebase(),
            Message::CancelWrapUp => self.handle_cancel_wrap_up(),
            Message::DetachTmux(id) => self.handle_detach_tmux(vec![id]),
            Message::BatchDetachTmux(ids) => self.handle_detach_tmux(ids),
            Message::ConfirmDetachTmux => self.handle_confirm_detach_tmux(),
            Message::ToggleSelect(id) => self.handle_toggle_select(id),
            Message::ClearSelection => self.handle_clear_selection(),
            Message::SelectAllColumn => self.handle_select_all_column(),
            Message::BatchMoveTasks { ids, direction } => {
                self.handle_batch_move_tasks(ids, direction)
            }
            Message::BatchArchiveTasks(ids) => self.handle_batch_archive_tasks(ids),

            // ── Form input, text entry, creation flows ──
            Message::StartNewTask => self.handle_start_new_task(),
            Message::CopyTask => self.handle_copy_task(),
            Message::CancelInput => self.handle_cancel_input(),
            Message::ConfirmDeleteStart => self.handle_confirm_delete_start(),
            Message::ConfirmDeleteYes => self.handle_confirm_delete_yes(),
            Message::CancelDelete => self.handle_cancel_delete(),
            Message::SubmitTitle(value) => self.handle_submit_title(value),
            Message::SubmitDescription(value) => self.handle_submit_description(value),
            Message::DescriptionEditorResult(value) => self.handle_description_editor_result(value),
            Message::EditorResult { kind, outcome } => self.handle_editor_result(kind, outcome),
            Message::SubmitRepoPath(value) => self.handle_submit_repo_path(value),
            Message::SubmitTag(tag) => self.handle_submit_tag(tag),
            Message::SubmitBaseBranch(value) => self.handle_submit_base_branch(value),
            Message::InputChar(c) => self.handle_input_char(c),
            Message::InputBackspace => self.handle_input_backspace(),
            Message::CancelRetry => self.handle_cancel_retry(),

            // ── Epic CRUD, lifecycle, wrap-up ──
            Message::DispatchEpic(id) => self.handle_dispatch_epic(id),
            Message::EnterEpic(epic_id) => self.handle_enter_epic(epic_id),
            Message::ExitEpic => self.handle_exit_epic(),
            Message::RefreshEpics(epics) => self.handle_refresh_epics(epics),
            Message::EpicCreated(epic) => self.handle_epic_created(epic),
            Message::EditEpic(id) => self.handle_edit_epic(id),
            Message::EpicEdited(epic) => self.handle_epic_edited(epic),
            Message::DeleteEpic(id) => self.handle_delete_epic(id),
            Message::ConfirmDeleteEpic => self.handle_confirm_delete_epic(),
            Message::MoveEpicStatus(id, dir) => self.handle_move_epic_status(id, dir),
            Message::ArchiveEpic(id) => self.handle_archive_epic(id),
            Message::ConfirmArchiveEpic => self.handle_confirm_archive_epic(),
            Message::StartNewEpic => self.handle_start_new_epic(),
            Message::SubmitEpicTitle(v) => self.handle_submit_epic_title(v),
            Message::SubmitEpicDescription(v) => self.handle_submit_epic_description(v),
            Message::SubmitEpicRepoPath(v) => self.handle_submit_epic_repo_path(v),
            Message::StartEpicWrapUp(id) => self.handle_start_epic_wrap_up(id),
            Message::EpicWrapUpRebase => self.handle_epic_wrap_up(),
            Message::CancelEpicWrapUp => self.handle_cancel_epic_wrap_up(),
            Message::CancelMergeQueue => self.handle_cancel_merge_queue(),
            Message::ToggleSelectEpic(id) => self.handle_toggle_select_epic(id),
            Message::BatchArchiveEpics(ids) => self.handle_batch_archive_epics(ids),
            Message::ToggleEpicAutoDispatch(id) => self.handle_toggle_epic_auto_dispatch(id),

            // ── PR flow: creation, merge, review state ──
            Message::PrMerged(id) => self.handle_pr_merged(id),
            Message::StartMergePr(id) => self.handle_start_merge_pr(id),
            Message::ConfirmMergePr => self.handle_confirm_merge_pr(),
            Message::CancelMergePr => self.handle_cancel_merge_pr(),
            Message::MergePrFailed { id, error } => self.handle_merge_pr_failed(id, error),
            Message::PrReviewState {
                id,
                review_decision,
            } => self.handle_pr_review_state(id, review_decision),

            // ── Task repo filters and filter presets ──
            Message::StartRepoFilter => self.handle_start_repo_filter(),
            Message::CloseRepoFilter => self.handle_close_repo_filter(),
            Message::ToggleRepoFilter(path) => self.handle_toggle_repo_filter(path),
            Message::ToggleAllRepoFilter => self.handle_toggle_all_repo_filter(),
            Message::ToggleRepoFilterMode => self.handle_toggle_repo_filter_mode(),
            Message::MoveRepoCursor(delta) => self.handle_move_repo_cursor(delta),
            Message::StartSavePreset => self.handle_start_save_preset(),
            Message::SaveFilterPreset(name) => self.handle_save_filter_preset(name),
            Message::LoadFilterPreset(name) => self.handle_load_filter_preset(name),
            Message::StartDeletePreset => self.handle_start_delete_preset(),
            Message::DeleteFilterPreset(name) => self.handle_delete_filter_preset(name),
            Message::StartDeleteRepoPath => self.handle_start_delete_repo_path(),
            Message::DeleteRepoPath(path) => self.handle_delete_repo_path(path),
            Message::CancelPresetInput => self.handle_cancel_preset_input(),
            Message::FilterPresetsLoaded(presets) => self.handle_filter_presets_loaded(presets),

            // ── Tips overlay ──
            Message::ShowTips {
                tips,
                starting_index,
                max_seen_id,
                show_mode,
            } => self.handle_show_tips(tips, starting_index, max_seen_id, show_mode),
            Message::NextTip => self.handle_next_tip(),
            Message::PrevTip => self.handle_prev_tip(),
            Message::SetTipsMode(mode) => self.handle_set_tips_mode(mode),
            Message::CloseTips => self.handle_close_tips(),

            // ── Project messages ──
            Message::ProjectsUpdated(projects) => self.handle_projects_updated(projects),
            Message::SelectProject(project_id) => self.handle_select_project(project_id),
            Message::FollowProject(project_id) => self.handle_follow_project(project_id),
            Message::TriggerEpicFeed(id) => self.handle_trigger_epic_feed(id),
            Message::FeedRefreshed { epic_title, count } => {
                self.handle_feed_refreshed(epic_title, count)
            }
            Message::FeedFailed { epic_title, error } => self.handle_feed_failed(epic_title, error),
            Message::OpenLearnings => vec![Command::LoadLearnings],
            Message::ShowLearnings(learnings) => self.handle_show_learnings(learnings),
            Message::CloseLearnings => self.handle_close_learnings(),
            Message::NavigateLearning(delta) => self.handle_navigate_learning(delta),
            Message::ArchiveLearning(id) => self.handle_archive_learning(id),
            Message::ToggleLearningsView => self.handle_toggle_learnings_view(),
            Message::NavigateTreeLearning(nav) => self.handle_navigate_tree_learning(nav),
            Message::RejectLearning(id) => self.handle_reject_learning(id),
            Message::EditLearning(id) => self.handle_edit_learning(id),
            Message::LearningActioned(id) => self.handle_learning_actioned(id),
            Message::LearningEdited(updated) => self.handle_learning_edited(updated),

            // ── Main session ──
            Message::SubmitMainSessionDir(dir) => self.handle_submit_main_session_dir(dir),
            Message::MainSessionCreated(window) => self.handle_main_session_created(window),
        }
    }

    // -----------------------------------------------------------------------
    // Per-message handlers
    // -----------------------------------------------------------------------

    pub(in crate::tui) fn handle_detach_tmux(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
        let detachable: Vec<TaskId> = ids
            .iter()
            .filter(|&&id| {
                self.find_task(id)
                    .is_some_and(|t| t.status == TaskStatus::Review && t.tmux_window.is_some())
            })
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
                    cmds.push(Command::KillTmuxWindow { window });
                }
                // Reset sub_status when detaching (e.g. Stale/Crashed -> default)
                if task.sub_status == SubStatus::Stale || task.sub_status == SubStatus::Crashed {
                    task.sub_status = SubStatus::default_for(task.status);
                }
                let task_clone = task.clone();
                cmds.push(Command::PersistTask(task_clone));
            }
        }
        cmds
    }

    pub(in crate::tui) fn finish_epic_creation(&mut self, repo_path: String) -> Vec<Command> {
        let mut draft = self.input.epic_draft.take().unwrap_or_default();
        draft.repo_path = repo_path.clone();
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![Command::InsertEpic(draft), Command::SaveRepoPath(repo_path)]
    }
}

#[cfg(test)]
mod tests;
