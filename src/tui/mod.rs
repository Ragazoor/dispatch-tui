pub mod input;
pub mod types;
pub mod ui;

pub use types::*;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::dispatch;
use crate::models::{
    epic_substatus, DispatchMode, Epic, EpicId, EpicSubstatus, Project, ProjectId, ReviewDecision,
    SubStatus, Task, TaskId, TaskStatus, TaskTag, TaskUsage, VisualColumn, DEFAULT_BASE_BRANCH,
    DEFAULT_QUICK_TASK_TITLE,
};

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How long a transient status message stays visible before auto-clearing.
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

/// Interval between PR status polls for tasks in review.
const PR_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Max character width for task titles shown in confirmation popups and status messages.
pub(in crate::tui) const TITLE_DISPLAY_LENGTH: usize = 30;

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
    pub(in crate::tui) select: SelectionState,
    pub(in crate::tui) filter: FilterState,
    pub(in crate::tui) merge_queue: Option<MergeQueue>,
    /// Task IDs with an in-flight dispatch (worktree + tmux setup running).
    /// Prevents duplicate dispatches when the user presses Enter rapidly.
    pub(in crate::tui) dispatching: HashSet<TaskId>,
    pub(in crate::tui) tips: Option<TipsOverlayState>,
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
                detail_visible: false,
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
            select: SelectionState::default(),
            filter: FilterState::default(),
            merge_queue: None,
            dispatching: HashSet::new(),
            tips: None,
        };
        app.update_anchor_from_current();
        app
    }

    /// Returns true if the given task has an in-flight dispatch.
    pub fn is_dispatching(&self, id: TaskId) -> bool {
        self.dispatching.contains(&id)
    }

    /// Get the current selection state (from whichever view mode is active).
    pub fn selection(&self) -> &BoardSelection {
        match &self.board.view_mode {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
        }
    }

    /// Get mutable access to the current selection state.
    pub(in crate::tui) fn selection_mut(&mut self) -> &mut BoardSelection {
        match &mut self.board.view_mode {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
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
    pub fn detail_visible(&self) -> bool {
        self.board.detail_visible
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
        self.selection().column() == 5
    }
    pub fn selected_archive_row(&self) -> usize {
        self.selection().row(5)
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
    }

    /// Clear the status message and its timestamp.
    pub(in crate::tui) fn clear_status(&mut self) {
        self.status.message = None;
        self.status.message_set_at = None;
    }

    fn repo_matches(&self, repo_path: &str) -> bool {
        self.filter.matches(repo_path)
    }

    fn project_matches(&self, project_id: ProjectId) -> bool {
        project_id == self.active_project
    }

    /// Return tasks visible in the current view.
    /// Board view: standalone tasks only (epic_id is None).
    /// Epic view: only subtasks of the active epic.
    pub fn tasks_for_current_view(&self) -> Vec<&Task> {
        let repo_match = |t: &&Task| self.repo_matches(&t.repo_path);
        let project_match = |t: &&Task| self.project_matches(t.project_id);
        match &self.board.view_mode {
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
        let mut items: Vec<ColumnItem<'_>> = tasks.into_iter().map(ColumnItem::Task).collect();

        if !self.board.flattened {
            match &self.board.view_mode {
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
        });

        items
    }

    /// Count column items for a status without sorting or allocating the full list.
    /// Used by `clamp_selection()` which only needs counts, not the sorted items.
    fn column_item_count(&self, status: TaskStatus) -> usize {
        let task_count = self.tasks_by_status(status).len();
        if self.board.flattened {
            return task_count;
        }
        let epic_count = match &self.board.view_mode {
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

        let epics_to_show: Vec<&Epic> = match &self.board.view_mode {
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
            };
            (sort_order.unwrap_or(i64::MAX), id)
        });
        items
    }

    /// Get the statuses of all subtasks belonging to an epic.
    fn subtask_statuses(&self, epic_id: EpicId) -> Vec<TaskStatus> {
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
        // Edge columns (Projects=0, Archive=5) have no task/epic column items.
        if col == 0 || col == TaskStatus::COLUMN_COUNT + 1 {
            return None;
        }
        // Task columns 1–4: offset by 1 to get the 0-based task status index.
        let status = TaskStatus::from_column_index(col - 1)?;
        let items = self.column_items_for_status(status);
        let row = self.selection().row(col);
        items.into_iter().nth(row)
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
        // Task columns use nav col offset: Backlog=1, Running=2, Review=3, Done=4.
        for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
            let nav_col = idx + 1; // convert 0-based task index to nav col
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
        // Edge columns (Projects=0, Archive=5) have no anchoring in the task board.
        if col == 0 || col > TaskStatus::COLUMN_COUNT {
            return;
        }
        let row = self.selection().row(col);
        // Task columns 1–4: offset by 1 to get the 0-based task status index.
        let status = match TaskStatus::from_column_index(col - 1) {
            Some(s) => s,
            None => return,
        };
        let new_anchor = self
            .column_items_for_status(status)
            .into_iter()
            .nth(row)
            .map(|item| match item {
                ColumnItem::Task(t) => ColumnAnchor::Task(t.id),
                ColumnItem::Epic(e) => ColumnAnchor::Epic(e.id),
            });
        self.selection_mut().anchor = new_anchor;
    }

    /// Restore cursor position from the anchor after a data change.
    /// Scans all columns for the anchor item and moves the cursor to its new
    /// position (following it across columns if needed).
    /// Falls back to index clamping if the anchor is not found.
    pub fn sync_board_selection(&mut self) {
        let anchor = match &self.board.view_mode {
            ViewMode::Board(sel) | ViewMode::Epic { selection: sel, .. } => sel.anchor,
        };

        let Some(anchor) = anchor else {
            // on_select_all or no anchor set yet — just clamp
            return self.clamp_selection();
        };

        let stats = self.compute_epic_stats();
        let mut found: Option<(usize, usize)> = None;
        // Task columns use nav col offset: Backlog=1, Running=2, Review=3, Done=4.
        'outer: for (idx, &status) in TaskStatus::ALL.iter().enumerate() {
            let nav_col = idx + 1;
            let items = self.column_items_for_status_with_stats(status, Some(&stats));
            for (row, item) in items.into_iter().enumerate() {
                let item_anchor = match item {
                    ColumnItem::Task(t) => ColumnAnchor::Task(t.id),
                    ColumnItem::Epic(e) => ColumnAnchor::Epic(e.id),
                };
                if item_anchor == anchor {
                    found = Some((nav_col, row));
                    break 'outer;
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

    fn find_task(&self, id: TaskId) -> Option<&Task> {
        self.board.tasks.iter().find(|t| t.id == id)
    }

    fn find_task_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        self.board.tasks.iter_mut().find(|t| t.id == id)
    }

    /// Remove all in-memory agent tracking state for a task.
    fn clear_agent_tracking(&mut self, id: TaskId) {
        self.agents.clear(id);
    }

    /// Take worktree/tmux fields from a task and build a Cleanup command.
    /// Returns `None` if the task has no worktree (still clears tmux_window).
    fn take_cleanup(task: &mut Task) -> Option<Command> {
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
    fn take_detach(task: &mut Task) -> Option<Command> {
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
            Message::ToggleDetail => self.handle_toggle_detail(),
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
            Message::TabCycle => self.handle_tab_cycle(),

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
            Message::WrapUpPr => self.handle_wrap_up_pr(),
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
            Message::EpicWrapUpRebase => self.handle_epic_wrap_up(MergeAction::Rebase),
            Message::EpicWrapUpPr => self.handle_epic_wrap_up(MergeAction::Pr),
            Message::CancelEpicWrapUp => self.handle_cancel_epic_wrap_up(),
            Message::CancelMergeQueue => self.handle_cancel_merge_queue(),
            Message::ToggleSelectEpic(id) => self.handle_toggle_select_epic(id),
            Message::BatchArchiveEpics(ids) => self.handle_batch_archive_epics(ids),
            Message::ToggleEpicAutoDispatch(id) => self.handle_toggle_epic_auto_dispatch(id),

            // ── PR flow: creation, merge, review state ──
            Message::PrCreated { id, pr_url } => self.handle_pr_created(id, pr_url),
            Message::PrFailed { id, error } => self.handle_pr_failed(id, error),
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
            } => {
                self.tips = Some(TipsOverlayState {
                    index: starting_index,
                    max_seen_id,
                    show_mode,
                    tips,
                });
                vec![]
            }
            Message::NextTip => {
                if let Some(overlay) = &mut self.tips {
                    let len = overlay.tips.len();
                    if len > 0 {
                        overlay.index = (overlay.index + 1) % len;
                    }
                }
                vec![]
            }
            Message::PrevTip => {
                if let Some(overlay) = &mut self.tips {
                    let len = overlay.tips.len();
                    if len > 0 {
                        overlay.index = (overlay.index + len - 1) % len;
                    }
                }
                vec![]
            }
            Message::SetTipsMode(mode) => {
                if let Some(overlay) = &mut self.tips {
                    overlay.show_mode = mode;
                }
                vec![]
            }
            Message::CloseTips => {
                if let Some(overlay) = self.tips.take() {
                    let seen_up_to = overlay
                        .current_tip()
                        .map(|t| t.id.max(overlay.max_seen_id))
                        .unwrap_or(overlay.max_seen_id);
                    vec![Command::SaveTipsState {
                        seen_up_to,
                        show_mode: overlay.show_mode,
                    }]
                } else {
                    vec![]
                }
            }

            // ── Project messages ──
            Message::ProjectsUpdated(projects) => {
                self.board.projects = projects;
                vec![]
            }
            Message::SelectProject(project_id) => {
                self.active_project = project_id;
                self.sync_board_selection();
                if let Some(idx) = self.board.projects.iter().position(|p| p.id == project_id) {
                    self.projects_panel.list_state.select(Some(idx));
                }
                vec![]
            }
        }
    }

    // -----------------------------------------------------------------------
    // Per-message handlers
    // -----------------------------------------------------------------------

    fn handle_detach_tmux(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
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

    fn handle_confirm_detach_tmux(&mut self) -> Vec<Command> {
        let InputMode::ConfirmDetachTmux(ref ids) = self.input.mode else {
            return vec![];
        };
        let ids = ids.clone();
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.detach_tmux_panels(ids)
    }

    fn detach_tmux_panels(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
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

    fn handle_quit(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::ConfirmQuit;
        vec![]
    }

    fn handle_navigate_column(&mut self, delta: isize) -> Vec<Command> {
        // Column range [0, 5]: 0=Projects, 1=Backlog, 2=Running, 3=Review, 4=Done, 5=Archive.
        // In Epic view, Projects and Archive are not shown; clamp to [1, COLUMN_COUNT].
        let (min_col, max_col) = if matches!(self.board.view_mode, ViewMode::Epic { .. }) {
            (1isize, TaskStatus::COLUMN_COUNT as isize) // [1, 4] in epic view
        } else {
            (0isize, TaskStatus::COLUMN_COUNT as isize + 1) // [0, 5] on main board
        };
        let new_col = (self.selection().column() as isize + delta)
            .clamp(min_col, max_col) as usize;
        self.selection_mut().set_column(new_col);

        // Reset archive list state when entering the archive column.
        if new_col == TaskStatus::COLUMN_COUNT + 1 {
            self.archive.selected_row = 0;
            *self.archive.list_state.selected_mut() = Some(0);
        }

        self.clamp_selection();
        self.update_anchor_from_current();
        vec![]
    }

    fn handle_navigate_row(&mut self, delta: isize) -> Vec<Command> {
        let col = self.selection().column();

        // Edge columns: Projects (0) and Archive (5)
        if col == 0 {
            let count = self.board.projects.len();
            if count == 0 {
                return vec![];
            }
            let new_row = (self.selection().row(0) as isize + delta)
                .clamp(0, count as isize - 1) as usize;
            self.selection_mut().set_row(0, new_row);
            self.projects_panel.list_state.select(Some(new_row));
            return vec![];
        }
        if col == TaskStatus::COLUMN_COUNT + 1 {
            let count = self.archived_tasks().len();
            if count == 0 {
                return vec![];
            }
            let new_row = (self.selection().row(TaskStatus::COLUMN_COUNT + 1) as isize + delta)
                .clamp(0, count as isize - 1) as usize;
            self.selection_mut()
                .set_row(TaskStatus::COLUMN_COUNT + 1, new_row);
            self.archive.list_state.select(Some(new_row));
            return vec![];
        }

        // Task columns 1–4: pass col-1 to from_column_index
        let status = match TaskStatus::from_column_index(col - 1) {
            Some(s) => s,
            None => return vec![],
        };
        let count = self.column_items_for_status(status).len();

        if self.selection().on_select_all {
            // On the toggle row
            if delta > 0 && count > 0 {
                // Move down into task list
                self.selection_mut().on_select_all = false;
                self.selection_mut().set_row(col, 0);
            }
            // delta <= 0 or empty column: stay on toggle (already at top)
        } else if count > 0 {
            let current = self.selection().row(col);
            if current == 0 && delta < 0 {
                // Move up from first task to toggle row
                self.selection_mut().on_select_all = true;
            } else {
                let new_row = (current as isize + delta).clamp(0, count as isize - 1) as usize;
                self.selection_mut().set_row(col, new_row);
            }
        } else {
            // Empty column: move to toggle
            if delta < 0 {
                self.selection_mut().on_select_all = true;
            }
        }
        self.update_anchor_from_current();
        vec![]
    }

    fn handle_reorder_item(&mut self, direction: isize) -> Vec<Command> {
        let col = self.selection().column();
        // Edge columns (Projects=0, Archive=5) don't support reorder
        if col == 0 || col == TaskStatus::COLUMN_COUNT + 1 {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let row = self.selection().row(col);
        let items = self.column_items_for_status(status);
        let target_row = row as isize + direction;
        if target_row < 0 || target_row >= items.len() as isize {
            return vec![];
        }
        let target_row = target_row as usize;

        // Get IDs and effective sort values
        let (a_task_id, a_epic_id, a_eff) = match &items[row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_order.unwrap_or(t.id.0)),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_order.unwrap_or(e.id.0)),
        };
        let (b_task_id, b_epic_id, b_eff) = match &items[target_row] {
            ColumnItem::Task(t) => (Some(t.id), None, t.sort_order.unwrap_or(t.id.0)),
            ColumnItem::Epic(e) => (None, Some(e.id), e.sort_order.unwrap_or(e.id.0)),
        };

        // Swap effective values; offset if equal
        let (new_a, new_b) = if a_eff == b_eff {
            if direction > 0 {
                (a_eff + 1, b_eff)
            } else {
                (a_eff - 1, b_eff)
            }
        } else {
            (b_eff, a_eff)
        };

        // Drop the borrowed items before mutating
        drop(items);

        let mut cmds = vec![];

        if let Some(tid) = a_task_id {
            if let Some(t) = self.find_task_mut(tid) {
                t.sort_order = Some(new_a);
                cmds.push(Command::PersistTask(t.clone()));
            }
        }
        if let Some(eid) = a_epic_id {
            if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                e.sort_order = Some(new_a);
                cmds.push(Command::PersistEpic {
                    id: eid,
                    status: None,
                    sort_order: Some(new_a),
                });
            }
        }
        if let Some(tid) = b_task_id {
            if let Some(t) = self.find_task_mut(tid) {
                t.sort_order = Some(new_b);
                cmds.push(Command::PersistTask(t.clone()));
            }
        }
        if let Some(eid) = b_epic_id {
            if let Some(e) = self.board.epics.iter_mut().find(|e2| e2.id == eid) {
                e.sort_order = Some(new_b);
                cmds.push(Command::PersistEpic {
                    id: eid,
                    status: None,
                    sort_order: Some(new_b),
                });
            }
        }

        // Cursor follows the moved item
        self.selection_mut().set_row(col, target_row);

        cmds
    }

    fn handle_move_task(&mut self, id: TaskId, direction: MoveDirection) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            let new_status = match direction {
                MoveDirection::Forward => task.status.next(),
                MoveDirection::Backward => task.status.prev(),
            };
            if new_status == task.status {
                return vec![];
            }

            // Confirm before moving to Done
            if new_status == TaskStatus::Done {
                let title = truncate_title(&task.title, TITLE_DISPLAY_LENGTH);
                self.input.mode = InputMode::ConfirmDone(id);
                self.set_status(format!("Move {title} to Done? [y/n]"));
                return vec![];
            }

            // Kill tmux window when moving backward, but keep worktree for resume
            let detach = if matches!(direction, MoveDirection::Backward) {
                Self::take_detach(task)
            } else {
                None
            };

            task.status = new_status;
            task.sub_status = SubStatus::default_for(new_status);
            let task_clone = task.clone();
            self.clear_agent_tracking(id);
            self.sync_board_selection();

            let mut cmds = Vec::new();
            if let Some(c) = detach {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone));
            cmds
        } else {
            vec![]
        }
    }

    fn handle_confirm_done(&mut self) -> Vec<Command> {
        let ids = if !self.select.pending_done.is_empty() {
            std::mem::take(&mut self.select.pending_done)
        } else {
            match self.input.mode {
                InputMode::ConfirmDone(id) => vec![id],
                _ => return vec![],
            }
        };
        self.input.mode = InputMode::Normal;
        self.clear_status();

        let mut cmds = Vec::new();
        for id in ids {
            if let Some(task) = self.find_task_mut(id) {
                if task.status != TaskStatus::Review {
                    continue;
                }
                let detach = Self::take_detach(task);
                task.status = TaskStatus::Done;
                task.sub_status = SubStatus::default_for(TaskStatus::Done);
                let task_clone = task.clone();
                self.clear_agent_tracking(id);
                if let Some(c) = detach {
                    cmds.push(c);
                }
                cmds.push(Command::PersistTask(task_clone));
                cmds.extend(self.maybe_respawn_split_pane(id));
            }
        }
        self.select.tasks.clear();
        self.sync_board_selection();
        cmds
    }

    fn handle_cancel_done(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.select.pending_done.clear();
        vec![]
    }

    fn handle_toggle_notifications(&mut self) -> Vec<Command> {
        self.notifications_enabled = !self.notifications_enabled;
        let label = if self.notifications_enabled {
            "Notifications enabled"
        } else {
            "Notifications disabled"
        };
        self.set_status(label.to_string());
        vec![Command::PersistSetting {
            key: "notifications_enabled".to_string(),
            value: self.notifications_enabled,
        }]
    }

    fn handle_dispatch_task(&mut self, id: TaskId, mode: DispatchMode) -> Vec<Command> {
        if self.dispatching.contains(&id) {
            return vec![];
        }
        let task = self
            .find_task(id)
            .filter(|t| t.status == TaskStatus::Backlog)
            .cloned();
        if let Some(task) = task {
            self.dispatching.insert(id);
            return vec![Command::DispatchAgent { task, mode }];
        }
        vec![]
    }

    fn handle_dispatched(
        &mut self,
        id: TaskId,
        worktree: String,
        tmux_window: String,
        switch_focus: bool,
    ) -> Vec<Command> {
        self.dispatching.remove(&id);
        if let Some(task) = self.find_task_mut(id) {
            task.worktree = Some(worktree);
            task.tmux_window = Some(tmux_window.clone());
            task.status = TaskStatus::Running;
            task.sub_status = SubStatus::default_for(TaskStatus::Running);
            let task_clone = task.clone();
            self.agents.mark_active(id);
            self.sync_board_selection();
            let mut cmds = vec![Command::PersistTask(task_clone)];
            if switch_focus {
                cmds.push(Command::JumpToTmux {
                    window: tmux_window,
                });
            }
            cmds
        } else {
            vec![]
        }
    }

    fn handle_task_created(&mut self, task: Task) -> Vec<Command> {
        self.board.tasks.push(task);
        self.sync_board_selection();
        vec![]
    }

    fn handle_delete_task(&mut self, id: TaskId) -> Vec<Command> {
        let cleanup = self.find_task_mut(id).and_then(Self::take_cleanup);
        self.clear_agent_tracking(id);
        self.board.tasks.retain(|t| t.id != id);
        self.sync_board_selection();
        let archive_count = self.archived_tasks().len();
        if self.archive.selected_row >= archive_count && archive_count > 0 {
            self.archive.selected_row = archive_count - 1;
        }
        *self.archive.list_state.selected_mut() = Some(self.archive.selected_row);
        let mut cmds = Vec::new();
        if let Some(c) = cleanup {
            cmds.push(c);
        }
        cmds.push(Command::DeleteTask(id));
        cmds
    }

    fn handle_toggle_detail(&mut self) -> Vec<Command> {
        self.board.detail_visible = !self.board.detail_visible;
        vec![]
    }

    fn handle_toggle_flattened(&mut self) -> Vec<Command> {
        self.board.flattened = !self.board.flattened;
        // Column item counts change when toggling (epics hidden / shown, and
        // tasks from the subtree merged in / split out), so selection row
        // indices may be out of bounds. Sync to follow the anchor.
        self.sync_board_selection();
        vec![]
    }

    fn handle_tmux_output(&mut self, id: TaskId, output: String, activity_ts: u64) -> Vec<Command> {
        let mut cmds = Vec::new();
        let activity_changed = self
            .agents
            .prev_tmux_activity
            .get(&id)
            .is_none_or(|&prev| prev != activity_ts);
        if activity_changed {
            self.agents.mark_active(id);
            // Recovery: reset stale/crashed sub_status when activity resumes
            let needs_recovery = self
                .find_task(id)
                .is_some_and(|t| matches!(t.sub_status, SubStatus::Stale | SubStatus::Crashed));
            if needs_recovery {
                if let Some(task) = self.find_task_mut(id) {
                    task.sub_status = SubStatus::Active;
                }
                if let Some(task) = self.find_task(id) {
                    cmds.push(Command::PersistTask(task.clone()));
                }
            }
            self.agents.prev_tmux_activity.insert(id, activity_ts);
        }
        self.agents.tmux_outputs.insert(id, output);
        cmds
    }

    fn handle_window_gone(&mut self, id: TaskId) -> Vec<Command> {
        // Ignore WindowGone for the split-pinned task — its window is joined as
        // a pane and isn't missing, just not a standalone window right now.
        if self.board.split.active && self.board.split.pinned_task_id == Some(id) {
            return vec![];
        }
        if let Some(task) = self.find_task(id) {
            if task.status == TaskStatus::Running {
                // Running task lost its window — likely crashed
                return self.handle_agent_crashed(id);
            }
        }
        // Non-running task: existing behavior
        if let Some(task) = self.find_task_mut(id) {
            task.tmux_window = None;
            let task_clone = task.clone();
            vec![Command::PersistTask(task_clone)]
        } else {
            vec![]
        }
    }

    fn handle_refresh_tasks(&mut self, new_tasks: Vec<Task>) -> Vec<Command> {
        let mut cmds = Vec::new();

        for new_task in &new_tasks {
            // Extract old state before any mutable borrows
            let old_task = self.find_task(new_task.id);
            let was_needs_input = old_task.is_some_and(|t| t.sub_status == SubStatus::NeedsInput);
            let was_review = old_task.is_some_and(|t| t.status == TaskStatus::Review);

            // Reset stale timer when a task recovers from Stale/Crashed via DB refresh
            let was_stale_or_crashed = old_task
                .is_some_and(|t| matches!(t.sub_status, SubStatus::Stale | SubStatus::Crashed));
            let is_recovered = !matches!(
                new_task.sub_status,
                SubStatus::Stale | SubStatus::Crashed | SubStatus::Conflict
            );
            if was_stale_or_crashed && is_recovered {
                self.agents.mark_active(new_task.id);
            }

            if self.notifications_enabled {
                // Detect NeedsInput transition (running tasks only)
                if new_task.sub_status == SubStatus::NeedsInput
                    && !was_needs_input
                    && new_task.status == TaskStatus::Running
                    && !self.agents.notified_needs_input.contains(&new_task.id)
                {
                    self.agents.notified_needs_input.insert(new_task.id);
                    cmds.push(Command::SendNotification {
                        title: format!("Task #{}: {}", new_task.id.0, new_task.title),
                        body: "Agent needs your input".to_string(),
                        urgent: true,
                    });
                }

                // Detect review transition (notification)
                if new_task.status == TaskStatus::Review
                    && !was_review
                    && !self.agents.notified_review.contains(&new_task.id)
                {
                    self.agents.notified_review.insert(new_task.id);
                    cmds.push(Command::SendNotification {
                        title: format!("Task #{}: {}", new_task.id.0, new_task.title),
                        body: "Ready for review".to_string(),
                        urgent: false,
                    });
                }
            }

            // Always clear notified state when task leaves the triggering state,
            // even when notifications are disabled. This prevents stale entries from
            // suppressing notifications after re-enabling.
            if new_task.status != TaskStatus::Review {
                self.agents.notified_review.remove(&new_task.id);
            }
            if new_task.sub_status != SubStatus::NeedsInput {
                self.agents.notified_needs_input.remove(&new_task.id);
            }
        }

        // Merge DB state into in-memory state, preserving tmux_outputs
        // Prune selections for tasks that no longer exist
        let valid_ids: HashSet<TaskId> = new_tasks.iter().map(|t| t.id).collect();
        self.select.tasks.retain(|id| valid_ids.contains(id));
        self.board.tasks = new_tasks;
        self.sync_board_selection();
        cmds
    }

    fn handle_tick(&mut self) -> Vec<Command> {
        // Auto-clear transient status messages after 5 seconds (only in Normal mode)
        if self.input.mode == InputMode::Normal {
            if let Some(set_at) = self.status.message_set_at {
                if set_at.elapsed() > STATUS_MESSAGE_TTL {
                    self.clear_status();
                }
            }
        }

        // Clear expired message flash indicators
        self.agents
            .message_flash
            .retain(|_, t| t.elapsed().as_secs() < 3);

        // Skip capturing the split-pinned task: its window has been joined as a
        // pane and is no longer visible to `has_window`, which would falsely
        // trigger WindowGone → Crashed.
        let split_pinned = self
            .board
            .split
            .pinned_task_id
            .filter(|_| self.board.split.active);

        let mut cmds: Vec<Command> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.tmux_window.is_some())
            .filter(|t| Some(t.id) != split_pinned)
            .filter_map(|t| {
                t.tmux_window
                    .clone()
                    .map(|window| Command::CaptureTmux { id: t.id, window })
            })
            .collect();

        // Check for stale agents
        let timeout = self.agents.inactivity_timeout;
        let newly_stale: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Running && t.tmux_window.is_some())
            .filter(|t| {
                !matches!(
                    t.sub_status,
                    SubStatus::Stale | SubStatus::Crashed | SubStatus::Conflict
                )
            })
            .filter(|t| {
                self.agents
                    .inactive_duration(t.id)
                    .is_some_and(|d| d > timeout)
            })
            .map(|t| t.id)
            .collect();

        for id in newly_stale {
            let stale_cmds = self.handle_stale_agent(id);
            cmds.extend(stale_cmds);
        }

        // Poll PR status for review tasks with open PRs
        let pr_tasks: Vec<(TaskId, String)> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Review)
            .filter(|t| {
                self.agents
                    .last_pr_poll
                    .get(&t.id)
                    .is_none_or(|last| last.elapsed() > PR_POLL_INTERVAL)
            })
            .filter_map(|t| t.pr_url.clone().map(|url| (t.id, url)))
            .collect();

        for (id, pr_url) in pr_tasks {
            self.agents.last_pr_poll.insert(id, Instant::now());
            cmds.push(Command::CheckPrStatus { id, pr_url });
        }

        // Check if split mode right pane still exists
        if self.board.split.active {
            if let Some(pane_id) = &self.board.split.right_pane_id {
                cmds.push(Command::CheckSplitPaneExists {
                    pane_id: pane_id.clone(),
                });
            }
        }

        cmds.push(Command::RefreshFromDb);
        cmds
    }

    fn handle_stale_agent(&mut self, id: TaskId) -> Vec<Command> {
        // Only applies to Running tasks
        let dominated = match self.find_task(id) {
            Some(t) if t.status == TaskStatus::Running => {
                // Escalation only: don't downgrade Crashed to Stale
                t.sub_status == SubStatus::Crashed
            }
            _ => return vec![],
        };
        if dominated {
            return vec![];
        }

        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            task.sub_status = SubStatus::Stale;
        }
        let elapsed = self
            .agents
            .inactive_duration(id)
            .map(|d| d.as_secs() / 60)
            .unwrap_or(0);
        if let Some(task) = self.find_task(id) {
            cmds.push(Command::PersistTask(task.clone()));
        }
        self.set_status(format!(
            "Task {id} inactive for {elapsed}m - press d to retry",
        ));

        if self.notifications_enabled {
            if let Some(task) = self.find_task(id) {
                cmds.push(Command::SendNotification {
                    title: format!("Task #{}: {}", task.id.0, task.title),
                    body: format!("Agent inactive for {elapsed}m"),
                    urgent: false,
                });
            }
        }
        cmds
    }

    fn handle_agent_crashed(&mut self, id: TaskId) -> Vec<Command> {
        // Only applies to Running tasks
        if !self
            .find_task(id)
            .is_some_and(|t| t.status == TaskStatus::Running)
        {
            return vec![];
        }

        // Capture last tmux output as crash context
        if let Some(output) = self.agents.tmux_outputs.get(&id) {
            if !output.is_empty() {
                self.agents.last_error.insert(id, output.clone());
            }
        }

        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            task.sub_status = SubStatus::Crashed;
            task.tmux_window = None;
        }
        if let Some(task) = self.find_task(id) {
            cmds.push(Command::PersistTask(task.clone()));
        }
        self.set_status(format!("Task {id} agent crashed - press d to retry",));

        if self.notifications_enabled {
            if let Some(task) = self.find_task(id) {
                cmds.push(Command::SendNotification {
                    title: format!("Task #{}: {}", task.id.0, task.title),
                    body: "Agent crashed".to_string(),
                    urgent: true,
                });
            }
        }
        cmds
    }

    fn handle_resume_task(&mut self, id: TaskId) -> Vec<Command> {
        if let Some(task) = self.find_task(id) {
            if !matches!(task.status, TaskStatus::Running | TaskStatus::Review) {
                return vec![];
            }
            if task.worktree.is_some() && task.tmux_window.is_none() {
                vec![Command::Resume { task: task.clone() }]
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }

    fn handle_resumed(&mut self, id: TaskId, tmux_window: String) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            task.tmux_window = Some(tmux_window);
            task.status = TaskStatus::Running;
            task.sub_status = SubStatus::Active;
            let task_clone = task.clone();
            self.agents.mark_active(id);
            self.agents.last_error.remove(&id);
            self.sync_board_selection();
            self.set_status(format!("Task {id} resumed"));
            vec![Command::PersistTask(task_clone)]
        } else {
            vec![]
        }
    }

    fn handle_error(&mut self, msg: String) -> Vec<Command> {
        self.status.error_popup = Some(msg);
        vec![]
    }

    fn handle_task_edited(&mut self, edit: TaskEdit) -> Vec<Command> {
        if let Some(t) = self.find_task_mut(edit.id) {
            t.title = edit.title;
            t.description = edit.description;
            t.repo_path = edit.repo_path;
            t.status = edit.status;
            t.plan_path = edit.plan_path;
            t.tag = edit.tag;
            if let Some(bb) = edit.base_branch {
                t.base_branch = bb;
            }
            t.updated_at = chrono::Utc::now();
        }
        self.sync_board_selection();
        vec![]
    }

    fn handle_repo_paths_updated(&mut self, paths: Vec<String>) -> Vec<Command> {
        self.board.repo_paths = paths;
        if !self.board.repo_paths.is_empty() {
            self.input.repo_cursor = self.input.repo_cursor.min(self.board.repo_paths.len() - 1);
        } else {
            self.input.repo_cursor = 0;
        }
        vec![]
    }

    fn handle_quick_dispatch(
        &mut self,
        repo_path: String,
        epic_id: Option<EpicId>,
    ) -> Vec<Command> {
        vec![Command::QuickDispatch {
            draft: TaskDraft {
                title: DEFAULT_QUICK_TASK_TITLE.to_string(),
                description: String::new(),
                repo_path,
                tag: None,
                base_branch: DEFAULT_BASE_BRANCH.to_string(),
            },
            epic_id,
        }]
    }

    fn handle_kill_and_retry(&mut self, id: TaskId) -> Vec<Command> {
        self.input.mode = InputMode::ConfirmRetry(id);
        let label = if self
            .find_task(id)
            .is_some_and(|t| t.sub_status == SubStatus::Crashed)
        {
            "crashed"
        } else {
            "stale"
        };
        self.set_status(format!(
            "Agent {} - [r] Resume  [f] Fresh start  [Esc] Cancel",
            label
        ));
        vec![]
    }

    fn handle_retry_resume(&mut self, id: TaskId) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.clear_agent_tracking(id);

        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Running {
                return vec![];
            }
            if task.worktree.is_none() {
                self.set_status("Cannot resume: task has no worktree".to_string());
                return vec![];
            }
            task.sub_status = SubStatus::Active;
            let old_window = task.tmux_window.take();
            let task_clone = task.clone();

            let mut cmds = Vec::new();
            if let Some(window) = old_window {
                cmds.push(Command::KillTmuxWindow { window });
            }
            cmds.push(Command::Resume { task: task_clone });
            cmds.extend(self.maybe_respawn_split_pane(id));
            cmds
        } else {
            vec![]
        }
    }

    fn handle_retry_fresh(&mut self, id: TaskId) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        self.clear_agent_tracking(id);

        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Running {
                return vec![];
            }
            let cleanup = Self::take_cleanup(task);
            task.status = TaskStatus::Backlog;
            task.sub_status = SubStatus::None;
            let task_clone = task.clone();

            let mut cmds = Vec::new();
            if let Some(c) = cleanup {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone.clone()));
            self.dispatching.insert(id);
            cmds.push(Command::DispatchAgent {
                task: task_clone,
                mode: DispatchMode::Dispatch,
            });
            cmds
        } else {
            vec![]
        }
    }

    fn handle_archive_task(&mut self, id: TaskId) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            if task.status == TaskStatus::Archived {
                return vec![];
            }
            let cleanup = Self::take_cleanup(task);
            task.status = TaskStatus::Archived;
            task.sub_status = SubStatus::default_for(TaskStatus::Archived);
            let task_clone = task.clone();
            self.clear_agent_tracking(id);
            self.sync_board_selection();

            let mut cmds = Vec::new();
            if let Some(c) = cleanup {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone));
            cmds.extend(self.maybe_respawn_split_pane(id));
            cmds
        } else {
            vec![]
        }
    }

    fn handle_toggle_select(&mut self, id: TaskId) -> Vec<Command> {
        if self.select.tasks.contains(&id) {
            self.select.tasks.remove(&id);
        } else {
            self.select.tasks.insert(id);
        }
        vec![]
    }

    fn handle_toggle_select_epic(&mut self, id: EpicId) -> Vec<Command> {
        if self.select.epics.contains(&id) {
            self.select.epics.remove(&id);
        } else {
            self.select.epics.insert(id);
        }
        vec![]
    }

    fn handle_clear_selection(&mut self) -> Vec<Command> {
        self.select.tasks.clear();
        self.select.epics.clear();
        self.selection_mut().on_select_all = false;
        vec![]
    }

    fn handle_select_all_column(&mut self) -> Vec<Command> {
        let col = self.selection().column();
        // Edge columns (Projects=0, Archive=5) don't support select-all.
        if col == 0 || col == TaskStatus::COLUMN_COUNT + 1 {
            return vec![];
        }
        let Some(status) = TaskStatus::from_column_index(col - 1) else {
            return vec![];
        };
        let items = self.column_items_for_status(status);
        let mut task_ids = Vec::new();
        let mut epic_ids = Vec::new();
        for item in &items {
            match item {
                ColumnItem::Task(t) => task_ids.push(t.id),
                ColumnItem::Epic(e) => epic_ids.push(e.id),
            }
        }
        if task_ids.is_empty() && epic_ids.is_empty() {
            return vec![];
        }
        let all_tasks_selected = task_ids.iter().all(|id| self.select.tasks.contains(id));
        let all_epics_selected = epic_ids.iter().all(|id| self.select.epics.contains(id));
        if all_tasks_selected && all_epics_selected {
            for id in &task_ids {
                self.select.tasks.remove(id);
            }
            for id in &epic_ids {
                self.select.epics.remove(id);
            }
        } else {
            for id in task_ids {
                self.select.tasks.insert(id);
            }
            for id in epic_ids {
                self.select.epics.insert(id);
            }
        }
        vec![]
    }

    fn handle_batch_archive_epics(&mut self, ids: Vec<EpicId>) -> Vec<Command> {
        let mut cmds = Vec::new();
        let mut skipped = 0usize;
        for id in ids {
            let not_done = self
                .subtask_statuses(id)
                .iter()
                .filter(|s| **s != TaskStatus::Done)
                .count();
            if not_done > 0 {
                skipped += 1;
                continue;
            }
            cmds.extend(self.handle_archive_epic(id));
        }
        if skipped > 0 {
            let noun = if skipped == 1 { "epic" } else { "epics" };
            self.set_status(format!("Skipped {skipped} {noun} with non-done subtasks"));
        }
        self.select.epics.clear();
        self.select.tasks.clear();
        cmds
    }

    fn handle_toggle_epic_auto_dispatch(&mut self, id: EpicId) -> Vec<Command> {
        if let Some(epic) = self.board.epics.iter_mut().find(|e| e.id == id) {
            let new_val = !epic.auto_dispatch;
            epic.auto_dispatch = new_val;
            vec![Command::ToggleEpicAutoDispatch {
                id,
                auto_dispatch: new_val,
            }]
        } else {
            vec![]
        }
    }

    fn handle_batch_move_tasks(
        &mut self,
        ids: Vec<TaskId>,
        direction: MoveDirection,
    ) -> Vec<Command> {
        if matches!(direction, MoveDirection::Forward) {
            let review_ids: Vec<TaskId> = ids
                .iter()
                .copied()
                .filter(|id| {
                    self.find_task(*id)
                        .is_some_and(|t| t.status == TaskStatus::Review)
                })
                .collect();

            if !review_ids.is_empty() {
                // Move non-Review tasks immediately
                let mut cmds = Vec::new();
                for id in &ids {
                    if !review_ids.contains(id) {
                        cmds.extend(self.handle_move_task(*id, direction));
                    }
                }
                // Enter confirmation for Review→Done tasks
                self.select.pending_done = review_ids;
                let count = self.select.pending_done.len();
                self.input.mode = InputMode::ConfirmDone(self.select.pending_done[0]);
                self.set_status(format!(
                    "Move {} {} to Done? [y/n]",
                    count,
                    if count == 1 { "task" } else { "tasks" }
                ));
                return cmds;
            }
        }

        let mut cmds = Vec::new();
        for id in ids {
            cmds.extend(self.handle_move_task(id, direction));
        }
        self.select.tasks.clear();
        cmds
    }

    fn handle_batch_archive_tasks(&mut self, ids: Vec<TaskId>) -> Vec<Command> {
        let mut cmds = Vec::new();
        for id in ids {
            cmds.extend(self.handle_archive_task(id));
        }
        self.select.tasks.clear();
        cmds
    }

    fn handle_dismiss_error(&mut self) -> Vec<Command> {
        self.status.error_popup = None;
        vec![]
    }

    fn handle_copy_task(&mut self) -> Vec<Command> {
        let task = match self.selected_task() {
            Some(t) => t,
            None => return vec![],
        };
        let title = format!("Copy of: {}", task.title);
        let description = task.description.clone();
        let repo_path = task.repo_path.clone();
        let tag = task.tag;
        self.input.task_draft = Some(TaskDraft {
            title,
            description,
            tag,
            ..Default::default()
        });
        self.input.buffer = repo_path;
        self.input.repo_cursor = 0;
        self.input.mode = InputMode::InputRepoPath;
        self.set_status("Enter repo path: ".to_string());
        vec![]
    }

    fn handle_start_new_task(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::InputTitle;
        self.input.buffer.clear();
        self.input.task_draft = None;
        self.set_status("Enter title: ".to_string());
        vec![]
    }

    fn handle_cancel_input(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.input.buffer.clear();
        self.input.task_draft = None;
        self.input.pending_epic_id = None;
        self.clear_status();
        vec![]
    }

    fn handle_confirm_delete_start(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            let title = truncate_title(&task.title, TITLE_DISPLAY_LENGTH);
            let status = task.status.as_str();
            let warning = if task.worktree.is_some() {
                " (has worktree)"
            } else {
                ""
            };
            self.input.mode = InputMode::ConfirmDelete;
            self.set_status(format!("Delete {title} [{status}]{warning}? [y/n]"));
        }
        vec![]
    }

    fn handle_confirm_delete_yes(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        if let Some(task) = self.selected_task() {
            let id = task.id;
            self.handle_delete_task(id)
        } else {
            vec![]
        }
    }

    fn handle_cancel_delete(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    fn handle_submit_title(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.input.mode = InputMode::Normal;
            self.input.task_draft = None;
            self.clear_status();
        } else {
            self.input.task_draft = Some(TaskDraft {
                title: value,
                description: String::new(),
                repo_path: String::new(),
                tag: None,
                base_branch: "main".to_string(),
            });
            self.input.mode = InputMode::InputTag;
            self.set_status("Tag: [b]ug  [f]eature  [c]hore  [e]pic  [Enter] none".to_string());
        }
        vec![]
    }

    fn handle_submit_description(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.task_draft {
            draft.description = value;
        }
        self.input.repo_cursor = 0;
        self.input.mode = InputMode::InputRepoPath;
        self.set_status("Enter repo path: ".to_string());
        vec![]
    }

    fn handle_submit_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.set_status("Repo path required (no saved paths available)".to_string());
            return vec![];
        }
        if let Err(msg) = crate::dispatch::validate_repo_path(&value) {
            self.set_status(msg);
            return vec![];
        }
        if let Some(ref mut draft) = self.input.task_draft {
            draft.repo_path = value;
        }
        self.input.buffer = self
            .input
            .task_draft
            .as_ref()
            .map(|d| d.base_branch.clone())
            .unwrap_or_else(|| "main".to_string());
        self.input.mode = InputMode::InputBaseBranch;
        self.set_status("Base branch: ".to_string());
        vec![]
    }

    fn handle_submit_base_branch(&mut self, value: String) -> Vec<Command> {
        let base_branch = if value.is_empty() {
            self.input
                .task_draft
                .as_ref()
                .map(|d| d.base_branch.clone())
                .unwrap_or_else(|| "main".to_string())
        } else {
            value
        };
        if let Some(ref mut draft) = self.input.task_draft {
            draft.base_branch = base_branch;
        }
        let repo_path = self
            .input
            .task_draft
            .as_ref()
            .map(|d| d.repo_path.clone())
            .unwrap_or_default();
        self.input.buffer.clear();
        self.finish_task_creation(repo_path)
    }

    fn handle_submit_tag(&mut self, tag: Option<TaskTag>) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.task_draft {
            draft.tag = tag;
        }
        self.input.mode = InputMode::InputDescription;
        self.set_status("Opening editor for description...".to_string());
        vec![Command::PopOutEditor(EditKind::Description {
            is_epic: false,
        })]
    }

    fn handle_input_char(&mut self, c: char) -> Vec<Command> {
        let is_repo_mode = matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        );
        if is_repo_mode && c.is_ascii_digit() && c != '0' {
            let idx = (c as usize) - ('1' as usize);
            let filtered = filtered_repos(&self.board.repo_paths, &self.input.buffer);
            if idx < filtered.len() {
                let repo_path = filtered[idx].clone();
                self.input.buffer.clear();
                return match self.input.mode {
                    InputMode::InputEpicRepoPath => self.finish_epic_creation(repo_path),
                    _ => self.update(Message::SubmitRepoPath(repo_path)),
                };
            }
        }
        // Per spec: cursor resets to 0 whenever the query changes
        if matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        ) {
            self.input.repo_cursor = 0;
        }
        self.input.buffer.push(c);
        vec![]
    }

    fn handle_input_backspace(&mut self) -> Vec<Command> {
        // Per spec: cursor resets to 0 whenever the query changes
        if matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        ) {
            self.input.repo_cursor = 0;
        }
        self.input.buffer.pop();
        vec![]
    }

    fn handle_start_quick_dispatch_selection(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::QuickDispatch;
        self.input.repo_cursor = 0;
        self.set_status("j/k navigate · Enter select · 1-9 shortcut · Esc cancel".to_string());
        vec![]
    }

    fn handle_select_quick_dispatch_repo(&mut self, idx: usize) -> Vec<Command> {
        if idx < self.board.repo_paths.len() {
            let repo_path = self.board.repo_paths[idx].clone();
            let epic_id = self.input.pending_epic_id.take();
            self.input.mode = InputMode::Normal;
            self.clear_status();
            self.handle_quick_dispatch(repo_path, epic_id)
        } else {
            vec![]
        }
    }

    fn handle_cancel_retry(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    fn handle_status_info(&mut self, msg: String) -> Vec<Command> {
        self.set_status(msg);
        vec![]
    }

    fn handle_toggle_help(&mut self) -> Vec<Command> {
        if self.input.mode == InputMode::Help {
            self.input.mode = InputMode::Normal;
        } else {
            self.input.mode = InputMode::Help;
        }
        vec![]
    }

    fn exit_split_if_active(&mut self) -> Vec<Command> {
        if !self.board.split.active {
            return vec![];
        }
        let pane_id = match self.board.split.right_pane_id.take() {
            Some(id) => id,
            None => return vec![],
        };
        let restore_window = self
            .board
            .split
            .pinned_task_id
            .and_then(|id| self.find_task(id))
            .and_then(|t| t.tmux_window.clone());
        vec![Command::ExitSplitMode {
            pane_id,
            restore_window,
        }]
    }

    fn handle_toggle_split_mode(&mut self) -> Vec<Command> {
        if self.board.split.active {
            self.exit_split_if_active()
        } else if let Some(window) = self.selected_task().and_then(|t| t.tmux_window.clone()) {
            let task_id = self.selected_task().unwrap().id;
            vec![Command::EnterSplitModeWithTask { task_id, window }]
        } else {
            vec![Command::EnterSplitMode]
        }
    }

    fn handle_swap_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
        // Already pinned — nothing to do
        if self.board.split.pinned_task_id == Some(task_id) {
            return vec![];
        }

        let task = match self.find_task(task_id) {
            Some(t) => t,
            None => return vec![],
        };
        let new_window = match &task.tmux_window {
            Some(w) => w.clone(),
            None => {
                return self.update(Message::StatusInfo(
                    "No agent session for this task".to_string(),
                ))
            }
        };
        let old_pane_id = self.board.split.right_pane_id.clone();
        let old_window = self
            .board
            .split
            .pinned_task_id
            .and_then(|id| self.find_task(id))
            .and_then(|t| t.tmux_window.clone());
        vec![Command::SwapSplitPane {
            task_id,
            new_window,
            old_pane_id,
            old_window,
        }]
    }

    fn handle_split_pane_opened(
        &mut self,
        pane_id: String,
        task_id: Option<TaskId>,
    ) -> Vec<Command> {
        self.board.split.active = true;
        self.board.split.focused = true;
        self.board.split.right_pane_id = Some(pane_id);
        self.board.split.pinned_task_id = task_id;
        vec![]
    }

    fn handle_focus_changed(&mut self, focused: bool) -> Vec<Command> {
        if self.board.split.active {
            self.board.split.focused = focused;
        }
        vec![]
    }

    fn handle_split_pane_closed(&mut self) -> Vec<Command> {
        self.board.split.active = false;
        self.board.split.focused = true;
        self.board.split.right_pane_id = None;
        self.board.split.pinned_task_id = None;
        vec![]
    }

    /// If `task_id` is the split-pinned task, clear the pin and respawn the
    /// pane with a fresh shell.  Split mode stays active.
    fn maybe_respawn_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
        if self.board.split.active && self.board.split.pinned_task_id == Some(task_id) {
            self.board.split.pinned_task_id = None;
            if let Some(pane_id) = self.board.split.right_pane_id.clone() {
                return vec![Command::RespawnSplitPane { pane_id }];
            }
        }
        vec![]
    }

    fn finish_task_creation(&mut self, repo_path: String) -> Vec<Command> {
        let draft = self.input.task_draft.take().unwrap_or_default();
        self.input.mode = InputMode::Normal;
        self.clear_status();
        let epic_id = match &self.board.view_mode {
            ViewMode::Epic { epic_id, .. } => Some(*epic_id),
            _ => None,
        };
        vec![
            Command::InsertTask { draft, epic_id },
            Command::SaveRepoPath(repo_path),
        ]
    }

    // -----------------------------------------------------------------------
    // Finish handlers (rebase + cleanup)
    // -----------------------------------------------------------------------

    fn handle_finish_complete(&mut self, id: TaskId) -> Vec<Command> {
        let in_queue = self
            .merge_queue
            .as_ref()
            .is_some_and(|q| q.current == Some(id));

        let mut cmds = if let Some(task) = self.find_task_mut(id) {
            task.tmux_window = None;
            task.status = TaskStatus::Done;
            task.sub_status = SubStatus::None;
            let task_clone = task.clone();
            self.clear_agent_tracking(id);
            self.sync_board_selection();
            if !in_queue {
                self.set_status(format!("Task {} finished", id));
            }
            vec![Command::PersistTask(task_clone)]
        } else {
            vec![]
        };

        cmds.extend(self.maybe_respawn_split_pane(id));

        if in_queue {
            if let Some(q) = &mut self.merge_queue {
                q.completed += 1;
                q.current = None;
            }
            cmds.extend(self.advance_merge_queue());
        }

        cmds
    }

    fn handle_finish_failed(
        &mut self,
        id: TaskId,
        error: String,
        is_conflict: bool,
    ) -> Vec<Command> {
        let mut cmds = Vec::new();

        if is_conflict {
            if let Some(task) = self.find_task_mut(id) {
                task.sub_status = SubStatus::Conflict;
            }
            cmds.push(Command::PatchSubStatus {
                id,
                sub_status: SubStatus::Conflict,
            });
        }

        if let Some(q) = &mut self.merge_queue {
            if q.current == Some(id) {
                q.current = None;
                q.failed = Some(id);
                let completed = q.completed;
                let total = q.task_ids.len();
                self.set_status(format!(
                    "Epic merge paused ({completed}/{total}): #{id} \u{2014} {error}"
                ));
                return cmds;
            }
        }

        self.set_status(error);
        cmds
    }

    // -----------------------------------------------------------------------
    // PR handlers
    // -----------------------------------------------------------------------

    fn handle_pr_created(&mut self, id: TaskId, pr_url: String) -> Vec<Command> {
        let in_queue = self
            .merge_queue
            .as_ref()
            .is_some_and(|q| q.current == Some(id));

        let mut cmds = if let Some(task) = self.find_task_mut(id) {
            task.pr_url = Some(pr_url.clone());
            task.status = TaskStatus::Review;
            task.sub_status = SubStatus::default_for(TaskStatus::Review);
            let task_clone = task.clone();
            if !in_queue {
                let pr_num = crate::models::pr_number_from_url(&pr_url);
                let label = pr_num.map_or("PR".to_string(), |n| format!("PR #{n}"));
                self.set_status(format!("{label} created: {pr_url}"));
            }
            vec![Command::PersistTask(task_clone)]
        } else {
            vec![]
        };

        if in_queue {
            if let Some(q) = &mut self.merge_queue {
                q.completed += 1;
                q.current = None;
            }
            cmds.extend(self.advance_merge_queue());
        }

        cmds
    }

    fn handle_pr_failed(&mut self, id: TaskId, error: String) -> Vec<Command> {
        if let Some(q) = &mut self.merge_queue {
            if q.current == Some(id) {
                q.current = None;
                q.failed = Some(id);
                let completed = q.completed;
                let total = q.task_ids.len();
                self.set_status(format!(
                    "Epic merge paused ({completed}/{total}): PR #{id} \u{2014} {error}"
                ));
                return vec![];
            }
        }

        self.set_status(error);
        vec![]
    }

    fn handle_pr_merged(&mut self, id: TaskId) -> Vec<Command> {
        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Review {
                return cmds;
            }

            let pr_label = task
                .pr_url
                .as_deref()
                .and_then(crate::models::pr_number_from_url)
                .map_or("PR".to_string(), |n| format!("PR #{n}"));
            let title = task.title.clone();

            // Detach: kill tmux window but preserve worktree
            if let Some(window) = task.tmux_window.take() {
                cmds.push(Command::KillTmuxWindow { window });
            }
            task.status = TaskStatus::Done;
            task.sub_status = SubStatus::default_for(TaskStatus::Done);
            let task_clone = task.clone();

            self.clear_agent_tracking(id);
            self.sync_board_selection();
            self.set_status(format!(
                "{pr_label} merged \u{2014} task #{id} moved to Done"
            ));

            cmds.push(Command::PersistTask(task_clone));

            if self.notifications_enabled {
                cmds.push(Command::SendNotification {
                    title: "PR merged".to_string(),
                    body: format!("{pr_label} merged: {title}"),
                    urgent: false,
                });
            }
        }

        cmds.extend(self.maybe_respawn_split_pane(id));

        cmds
    }

    fn handle_start_merge_pr(&mut self, id: TaskId) -> Vec<Command> {
        let task = match self.find_task(id) {
            Some(t) => t,
            None => return vec![],
        };

        if task.status != TaskStatus::Review {
            return self.update(Message::StatusInfo("Task is not in review".to_string()));
        }
        if task.pr_url.is_none() {
            return self.update(Message::StatusInfo("Task has no PR".to_string()));
        }
        if task.sub_status != SubStatus::Approved {
            let label = match task.sub_status {
                SubStatus::AwaitingReview => "awaiting review",
                SubStatus::ChangesRequested => "changes requested",
                _ => "not approved",
            };
            return self.update(Message::StatusInfo(format!("Cannot merge: PR is {label}")));
        }

        let pr_label = task
            .pr_url
            .as_deref()
            .and_then(crate::models::pr_number_from_url)
            .map_or("PR".to_string(), |n| format!("PR #{n}"));
        let title = truncate_title(&task.title, TITLE_DISPLAY_LENGTH);

        self.input.mode = InputMode::ConfirmMergePr(id);
        self.set_status(format!("Merge {pr_label} for {title}? [y/n]"));
        vec![]
    }

    fn handle_confirm_merge_pr(&mut self) -> Vec<Command> {
        let id = match self.input.mode {
            InputMode::ConfirmMergePr(id) => id,
            _ => return vec![],
        };
        self.input.mode = InputMode::Normal;

        let pr_url = match self.find_task(id).and_then(|t| t.pr_url.clone()) {
            Some(url) => url,
            None => {
                self.clear_status();
                return vec![];
            }
        };

        let pr_label = crate::models::pr_number_from_url(&pr_url)
            .map_or("PR".to_string(), |n| format!("PR #{n}"));
        self.set_status(format!("Merging {pr_label}..."));
        vec![Command::MergePr { id, pr_url }]
    }

    fn handle_cancel_merge_pr(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    fn handle_merge_pr_failed(&mut self, _id: TaskId, error: String) -> Vec<Command> {
        self.set_status(format!("Merge failed: {error}"));
        vec![]
    }

    fn handle_pr_review_state(
        &mut self,
        id: TaskId,
        review_decision: Option<ReviewDecision>,
    ) -> Vec<Command> {
        if let Some(task) = self.find_task_mut(id) {
            if task.status != TaskStatus::Review {
                return vec![];
            }
            // Don't overwrite attention-requiring substatuses
            if task.sub_status == SubStatus::Conflict {
                return vec![];
            }
            let new_sub = match review_decision {
                Some(ReviewDecision::Approved) => SubStatus::Approved,
                Some(ReviewDecision::ChangesRequested) => SubStatus::ChangesRequested,
                _ => SubStatus::AwaitingReview,
            };
            if task.sub_status != new_sub {
                task.sub_status = new_sub;
                let task_clone = task.clone();
                return vec![Command::PersistTask(task_clone)];
            }
        }
        vec![]
    }

    // -----------------------------------------------------------------------
    // Wrap up handlers
    // -----------------------------------------------------------------------

    fn handle_start_wrap_up(&mut self, id: TaskId) -> Vec<Command> {
        let branch = match self.find_task(id) {
            Some(t) if dispatch::is_wrappable(t) => {
                match t
                    .worktree
                    .as_deref()
                    .and_then(dispatch::branch_from_worktree)
                {
                    Some(b) => b,
                    None => return vec![],
                }
            }
            _ => return vec![],
        };

        self.input.mode = InputMode::ConfirmWrapUp(id);
        self.set_status(format!(
            "Wrap up {}: [r] rebase onto main  [p] create PR  [Esc] cancel",
            branch
        ));
        vec![]
    }

    fn handle_wrap_up_rebase(&mut self) -> Vec<Command> {
        let id = match self.input.mode {
            InputMode::ConfirmWrapUp(id) => id,
            _ => return vec![],
        };
        self.input.mode = InputMode::Normal;
        self.set_status("Rebasing...".to_string());
        // Optimistically clear conflict substatus — FinishComplete will persist it.
        if let Some(task) = self.find_task_mut(id) {
            if task.sub_status == SubStatus::Conflict {
                task.sub_status = SubStatus::default_for(task.status);
            }
        }

        if let Some(task) = self.find_task(id) {
            let worktree = match &task.worktree {
                Some(wt) => wt.clone(),
                None => return vec![],
            };
            let branch = match dispatch::branch_from_worktree(&worktree) {
                Some(b) => b,
                None => return vec![],
            };
            vec![Command::Finish {
                id,
                repo_path: task.repo_path.clone(),
                branch,
                base_branch: task.base_branch.clone(),
                worktree,
                tmux_window: task.tmux_window.clone(),
            }]
        } else {
            vec![]
        }
    }

    fn handle_wrap_up_pr(&mut self) -> Vec<Command> {
        let id = match self.input.mode {
            InputMode::ConfirmWrapUp(id) => id,
            _ => return vec![],
        };
        self.input.mode = InputMode::Normal;
        self.set_status("Creating PR...".to_string());

        if let Some(task) = self.find_task(id) {
            let worktree = match &task.worktree {
                Some(wt) => wt.clone(),
                None => return vec![],
            };
            let branch = match dispatch::branch_from_worktree(&worktree) {
                Some(b) => b,
                None => return vec![],
            };
            vec![Command::CreatePr {
                id,
                repo_path: task.repo_path.clone(),
                branch,
                base_branch: task.base_branch.clone(),
                title: task.title.clone(),
                description: task.description.clone(),
            }]
        } else {
            vec![]
        }
    }

    fn handle_cancel_wrap_up(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    // -----------------------------------------------------------------------
    // Epic batch wrap-up handlers
    // -----------------------------------------------------------------------

    fn handle_start_epic_wrap_up(&mut self, epic_id: EpicId) -> Vec<Command> {
        let review_count = self
            .board
            .tasks
            .iter()
            .filter(|t| {
                t.epic_id == Some(epic_id) && t.status == TaskStatus::Review && t.worktree.is_some()
            })
            .count();

        if review_count == 0 {
            return self.update(Message::StatusInfo(
                "No review tasks to wrap up".to_string(),
            ));
        }

        self.input.mode = InputMode::ConfirmEpicWrapUp(epic_id);
        self.set_status(format!(
            "Wrap up {} review task{}: [r] rebase all  [p] PR all  [Esc] cancel",
            review_count,
            if review_count == 1 { "" } else { "s" },
        ));
        vec![]
    }

    fn handle_epic_wrap_up(&mut self, action: MergeAction) -> Vec<Command> {
        let epic_id = match self.input.mode {
            InputMode::ConfirmEpicWrapUp(id) => id,
            _ => return vec![],
        };
        self.input.mode = InputMode::Normal;

        let mut review_tasks: Vec<&Task> = self
            .board
            .tasks
            .iter()
            .filter(|t| {
                t.epic_id == Some(epic_id) && t.status == TaskStatus::Review && t.worktree.is_some()
            })
            .collect();
        review_tasks.sort_by_key(|t| t.sort_order.unwrap_or(t.id.0));

        let task_ids: Vec<TaskId> = review_tasks.iter().map(|t| t.id).collect();

        if task_ids.is_empty() {
            return vec![];
        }

        self.merge_queue = Some(MergeQueue {
            epic_id,
            action,
            task_ids,
            completed: 0,
            current: None,
            failed: None,
        });

        self.advance_merge_queue()
    }

    fn advance_merge_queue(&mut self) -> Vec<Command> {
        loop {
            let (total, next_idx, next_id, action) = match &self.merge_queue {
                Some(q) if q.completed < q.task_ids.len() => (
                    q.task_ids.len(),
                    q.completed,
                    q.task_ids[q.completed],
                    q.action.clone(),
                ),
                Some(q) => {
                    let total = q.task_ids.len();
                    self.merge_queue = None;
                    self.set_status(format!("Epic merge complete: {total}/{total} done"));
                    return vec![];
                }
                None => return vec![],
            };

            // Validate the task is still eligible
            let task_data = match self.find_task(next_id) {
                Some(t) if t.status == TaskStatus::Review => match t.worktree {
                    Some(ref worktree) => {
                        let worktree = worktree.clone();
                        let branch = dispatch::branch_from_worktree(&worktree);
                        let repo_path = t.repo_path.clone();
                        let base_branch = t.base_branch.clone();
                        let title = t.title.clone();
                        let description = t.description.clone();
                        let tmux_window = t.tmux_window.clone();
                        branch.map(|b| {
                            (
                                worktree,
                                b,
                                repo_path,
                                base_branch,
                                title,
                                description,
                                tmux_window,
                            )
                        })
                    }
                    None => None,
                },
                _ => None,
            };

            let Some((worktree, branch, repo_path, base_branch, title, description, tmux_window)) =
                task_data
            else {
                // Skip this task — no longer eligible
                if let Some(q) = &mut self.merge_queue {
                    q.completed += 1;
                }
                continue;
            };

            if let Some(q) = &mut self.merge_queue {
                q.current = Some(next_id);
            }

            self.set_status(format!(
                "Epic merge: {next_idx}/{total} done \u{2014} processing #{}",
                next_id
            ));

            return match action {
                MergeAction::Rebase => {
                    vec![Command::Finish {
                        id: next_id,
                        repo_path,
                        branch,
                        base_branch,
                        worktree,
                        tmux_window,
                    }]
                }
                MergeAction::Pr => vec![Command::CreatePr {
                    id: next_id,
                    repo_path,
                    branch,
                    base_branch,
                    title,
                    description,
                }],
            };
        }
    }

    fn handle_cancel_epic_wrap_up(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![]
    }

    fn handle_cancel_merge_queue(&mut self) -> Vec<Command> {
        self.merge_queue = None;
        self.set_status("Merge queue cancelled".to_string());
        vec![]
    }

    // -----------------------------------------------------------------------
    // Epic handlers
    // -----------------------------------------------------------------------

    fn handle_dispatch_epic(&mut self, id: EpicId) -> Vec<Command> {
        let Some(epic) = self.board.epics.iter().find(|e| e.id == id) else {
            return vec![];
        };
        let status = epic.status;

        if status != TaskStatus::Backlog {
            self.set_status("No backlog tasks in epic".to_string());
            return vec![];
        }

        if epic.plan_path.is_some() {
            // Epic has a plan — dispatch the next backlog subtask sorted by sort_order
            let mut backlog_subtasks: Vec<&Task> = self
                .board
                .tasks
                .iter()
                .filter(|t| {
                    t.epic_id == Some(id)
                        && t.status == TaskStatus::Backlog
                        && !self.dispatching.contains(&t.id)
                })
                .collect();
            backlog_subtasks.sort_by_key(|t| (t.sort_order.unwrap_or(t.id.0), t.id.0));

            match backlog_subtasks.first() {
                Some(task) => {
                    self.dispatching.insert(task.id);
                    let mode = DispatchMode::for_task(task);
                    vec![Command::DispatchAgent {
                        task: (*task).clone(),
                        mode,
                    }]
                }
                None => {
                    self.set_status("No backlog subtasks in epic".to_string());
                    vec![]
                }
            }
        } else {
            // No plan — only spawn planning subtask if epic has no active subtasks
            let has_subtasks = self
                .board
                .tasks
                .iter()
                .any(|t| t.epic_id == Some(id) && t.status != TaskStatus::Archived);
            if has_subtasks {
                self.set_status("Epic has subtasks but no plan".to_string());
                vec![]
            } else {
                vec![Command::DispatchEpic { epic: epic.clone() }]
            }
        }
    }

    fn handle_tab_cycle(&mut self) -> Vec<Command> {
        let feed_ids: Vec<EpicId> = self
            .board
            .epics
            .iter()
            .filter(|e| e.feed_command.is_some())
            .map(|e| e.id)
            .collect();

        // Clone to release the borrow before calling &mut self methods below.
        match self.board.view_mode.clone() {
            ViewMode::Board(_) => {
                if let Some(&first_id) = feed_ids.first() {
                    return self.handle_enter_epic(first_id);
                }
            }
            ViewMode::Epic { epic_id, .. } => {
                if let Some(pos) = feed_ids.iter().position(|&id| id == epic_id) {
                    if let Some(&next_id) = feed_ids.get(pos + 1) {
                        let _ = self.handle_exit_epic(); // always vec![], exit before entering next
                        return self.handle_enter_epic(next_id);
                    } else {
                        return self.handle_exit_epic();
                    }
                }
                // Not a feed epic — no-op
            }
        }
        vec![]
    }

    fn handle_enter_epic(&mut self, epic_id: EpicId) -> Vec<Command> {
        let parent = Box::new(self.board.view_mode.clone());
        self.board.view_mode = ViewMode::Epic {
            epic_id,
            selection: BoardSelection::new_for_epic(),
            parent,
        };
        self.board.detail_visible = false;
        vec![]
    }

    fn handle_exit_epic(&mut self) -> Vec<Command> {
        if let ViewMode::Epic { parent, .. } = &self.board.view_mode {
            self.board.view_mode = *parent.clone();
        }
        self.board.detail_visible = false;
        vec![]
    }

    fn handle_refresh_epics(&mut self, epics: Vec<Epic>) -> Vec<Command> {
        self.board.epics = epics;
        let valid_ids: HashSet<EpicId> = self.board.epics.iter().map(|e| e.id).collect();
        self.select.epics.retain(|id| valid_ids.contains(id));
        vec![]
    }

    fn handle_refresh_usage(&mut self, usage: Vec<TaskUsage>) -> Vec<Command> {
        self.board.usage = usage.into_iter().map(|u| (u.task_id, u)).collect();
        vec![]
    }

    fn handle_epic_created(&mut self, epic: Epic) -> Vec<Command> {
        self.board.epics.push(epic);
        vec![]
    }

    fn handle_edit_epic(&mut self, id: EpicId) -> Vec<Command> {
        if let Some(epic) = self.board.epics.iter().find(|e| e.id == id) {
            vec![Command::PopOutEditor(EditKind::EpicEdit(epic.clone()))]
        } else {
            vec![]
        }
    }

    fn handle_epic_edited(&mut self, epic: Epic) -> Vec<Command> {
        if let Some(e) = self.board.epics.iter_mut().find(|e| e.id == epic.id) {
            e.title = epic.title;
            e.description = epic.description;
            e.repo_path = epic.repo_path;
            e.updated_at = chrono::Utc::now();
        }
        vec![]
    }

    fn handle_delete_epic(&mut self, id: EpicId) -> Vec<Command> {
        let mut cmds = Vec::new();
        // Clean up worktrees/tmux for subtasks before deleting
        let subtask_ids: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(id))
            .map(|t| t.id)
            .collect();
        for task_id in subtask_ids {
            if let Some(task) = self.find_task_mut(task_id) {
                let cleanup = Self::take_cleanup(task);
                if let Some(c) = cleanup {
                    cmds.push(c);
                }
                self.clear_agent_tracking(task_id);
            }
        }
        self.board.epics.retain(|e| e.id != id);
        self.board.tasks.retain(|t| t.epic_id != Some(id));
        // If we were viewing this epic, exit
        if matches!(&self.board.view_mode, ViewMode::Epic { epic_id, .. } if *epic_id == id) {
            self.handle_exit_epic();
        }
        self.sync_board_selection();
        cmds.push(Command::DeleteEpic(id));
        cmds
    }

    fn handle_confirm_delete_epic(&mut self) -> Vec<Command> {
        if let Some(ColumnItem::Epic(epic)) = self.selected_column_item() {
            let title = truncate_title(&epic.title, TITLE_DISPLAY_LENGTH);
            self.input.mode = InputMode::ConfirmDeleteEpic;
            self.set_status(format!("Delete epic {title} and subtasks? [y/n]"));
        }
        vec![]
    }

    fn handle_move_epic_status(&mut self, id: EpicId, direction: MoveDirection) -> Vec<Command> {
        let Some(epic) = self.board.epics.iter_mut().find(|e| e.id == id) else {
            return vec![];
        };
        let new_status = match direction {
            MoveDirection::Forward => epic.status.next(),
            MoveDirection::Backward => epic.status.prev(),
        };
        if new_status == epic.status {
            return vec![];
        }
        epic.status = new_status;
        let mut cmds = vec![Command::PersistEpic {
            id,
            status: Some(new_status),
            sort_order: None,
        }];

        // Moving to Done cleans up all subtask tmux windows
        if new_status == TaskStatus::Done {
            let subtask_ids: Vec<TaskId> = self
                .board
                .tasks
                .iter()
                .filter(|t| t.epic_id == Some(id) && t.tmux_window.is_some())
                .map(|t| t.id)
                .collect();
            for task_id in subtask_ids {
                if let Some(task) = self.find_task_mut(task_id) {
                    if let Some(window) = task.tmux_window.take() {
                        cmds.push(Command::KillTmuxWindow { window });
                        cmds.push(Command::PersistTask(task.clone()));
                    }
                }
            }
        }
        self.sync_board_selection();
        cmds
    }

    fn handle_archive_epic(&mut self, id: EpicId) -> Vec<Command> {
        let mut cmds = Vec::new();
        let subtask_ids: Vec<TaskId> = self
            .board
            .tasks
            .iter()
            .filter(|t| t.epic_id == Some(id) && t.status != TaskStatus::Archived)
            .map(|t| t.id)
            .collect();
        for task_id in subtask_ids {
            cmds.extend(self.handle_archive_task(task_id));
        }
        self.board.epics.retain(|e| e.id != id);
        if matches!(&self.board.view_mode, ViewMode::Epic { epic_id, .. } if *epic_id == id) {
            self.handle_exit_epic();
        }
        self.sync_board_selection();
        cmds.push(Command::DeleteEpic(id));
        cmds
    }

    fn handle_confirm_archive_epic(&mut self) -> Vec<Command> {
        if let Some(ColumnItem::Epic(epic)) = self.selected_column_item() {
            let id = epic.id;
            let not_done_count = self
                .subtask_statuses(id)
                .iter()
                .filter(|s| **s != TaskStatus::Done)
                .count();
            if not_done_count > 0 {
                let noun = if not_done_count == 1 {
                    "subtask"
                } else {
                    "subtasks"
                };
                self.set_status(format!(
                    "Cannot archive epic: {} {} not done",
                    not_done_count, noun
                ));
                return vec![];
            }
            self.input.mode = InputMode::ConfirmArchiveEpic;
            self.set_status("Archive epic and all subtasks? [y/n]".to_string());
        }
        vec![]
    }

    fn handle_start_new_epic(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::InputEpicTitle;
        self.input.buffer.clear();
        let parent_epic_id = if let ViewMode::Epic { epic_id, .. } = self.board.view_mode {
            Some(epic_id)
        } else {
            None
        };
        self.input.epic_draft = Some(EpicDraft {
            parent_epic_id,
            ..Default::default()
        });
        self.set_status("Epic title: ".to_string());
        vec![]
    }

    fn handle_submit_epic_title(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.input.mode = InputMode::Normal;
            self.clear_status();
            vec![]
        } else {
            let parent_epic_id = self
                .input
                .epic_draft
                .as_ref()
                .and_then(|d| d.parent_epic_id);
            self.input.epic_draft = Some(EpicDraft {
                title: value,
                description: String::new(),
                repo_path: String::new(),
                parent_epic_id,
            });
            self.input.mode = InputMode::InputEpicDescription;
            self.set_status("Opening editor for description...".to_string());
            vec![Command::PopOutEditor(EditKind::Description {
                is_epic: true,
            })]
        }
    }

    fn handle_submit_epic_description(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.epic_draft {
            draft.description = value;
        }
        self.input.repo_cursor = 0;
        self.input.mode = InputMode::InputEpicRepoPath;
        self.set_status("Epic repo path: ".to_string());
        vec![]
    }

    fn handle_submit_epic_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if value.is_empty() {
            self.set_status("Repo path required".to_string());
            return vec![];
        }
        if let Err(msg) = crate::dispatch::validate_repo_path(&value) {
            self.set_status(msg);
            return vec![];
        }
        self.finish_epic_creation(value)
    }

    fn handle_start_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::RepoFilter;
        self.input.repo_cursor = 0;
        vec![]
    }

    fn handle_move_repo_cursor(&mut self, delta: isize) -> Vec<Command> {
        let count = if matches!(
            self.input.mode,
            InputMode::InputRepoPath | InputMode::InputEpicRepoPath
        ) {
            filtered_repos(&self.board.repo_paths, &self.input.buffer).len()
        } else {
            self.board.repo_paths.len()
        };
        if count == 0 {
            return vec![];
        }
        self.input.repo_cursor =
            (self.input.repo_cursor as isize + delta).rem_euclid(count as isize) as usize;
        vec![]
    }

    fn handle_close_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.sync_board_selection();
        let mut paths: Vec<_> = self.filter.repos.iter().cloned().collect();
        paths.sort();
        let value = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_string());
        let mode_value = self.filter.mode.as_str();
        vec![
            Command::PersistStringSetting {
                key: "repo_filter".to_string(),
                value,
            },
            Command::PersistStringSetting {
                key: "repo_filter_mode".to_string(),
                value: mode_value.to_string(),
            },
        ]
    }

    fn handle_toggle_repo_filter(&mut self, path: String) -> Vec<Command> {
        if self.filter.repos.contains(&path) {
            self.filter.repos.remove(&path);
        } else {
            self.filter.repos.insert(path);
        }
        self.sync_board_selection();
        vec![]
    }

    fn handle_toggle_all_repo_filter(&mut self) -> Vec<Command> {
        if self.filter.repos.len() == self.board.repo_paths.len() {
            self.filter.repos.clear();
        } else {
            self.filter.repos = self.board.repo_paths.iter().cloned().collect();
        }
        self.sync_board_selection();
        vec![]
    }

    fn handle_toggle_repo_filter_mode(&mut self) -> Vec<Command> {
        self.filter.mode = match self.filter.mode {
            RepoFilterMode::Include => RepoFilterMode::Exclude,
            RepoFilterMode::Exclude => RepoFilterMode::Include,
        };
        self.sync_board_selection();
        vec![]
    }

    fn handle_save_filter_preset(&mut self, name: String) -> Vec<Command> {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.input.mode = InputMode::RepoFilter;
            return vec![];
        }
        let repos: HashSet<String> = self.filter.repos.clone();
        let mode = self.filter.mode;
        // Update or insert in the presets list
        if let Some(existing) = self.filter.presets.iter_mut().find(|(n, _, _)| *n == name) {
            existing.1.clone_from(&repos);
            existing.2 = mode;
        } else {
            self.filter.presets.push((name.clone(), repos, mode));
            self.filter.presets.sort_by(|a, b| a.0.cmp(&b.0));
        }
        self.input.buffer.clear();
        self.input.mode = InputMode::RepoFilter;
        self.set_status(format!("Saved preset \"{name}\""));
        let mut paths: Vec<_> = self.filter.repos.iter().cloned().collect();
        paths.sort();
        vec![Command::PersistFilterPreset {
            name,
            repo_paths: paths,
            mode,
        }]
    }

    fn handle_load_filter_preset(&mut self, name: String) -> Vec<Command> {
        if let Some((_, repos, mode)) = self.filter.presets.iter().find(|(n, _, _)| *n == name) {
            // Intersect with known repo_paths to skip stale entries
            let known: HashSet<&String> = self.board.repo_paths.iter().collect();
            self.filter.repos = repos
                .iter()
                .filter(|p| known.contains(p))
                .cloned()
                .collect();
            self.filter.mode = *mode;
            self.sync_board_selection();
            self.set_status(format!("Loaded preset \"{name}\""));
        }
        vec![]
    }

    fn handle_start_save_preset(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::InputPresetName;
        vec![]
    }

    fn handle_start_delete_preset(&mut self) -> Vec<Command> {
        if self.filter.presets.is_empty() {
            return vec![];
        }
        self.input.mode = InputMode::ConfirmDeletePreset;
        vec![]
    }

    fn handle_delete_filter_preset(&mut self, name: String) -> Vec<Command> {
        self.filter.presets.retain(|(n, _, _)| *n != name);
        self.input.mode = InputMode::RepoFilter;
        self.set_status(format!("Deleted preset \"{name}\""));
        vec![Command::DeleteFilterPreset(name)]
    }

    fn handle_start_delete_repo_path(&mut self) -> Vec<Command> {
        if self.board.repo_paths.is_empty() {
            return vec![];
        }
        self.input.mode = InputMode::ConfirmDeleteRepoPath;
        vec![]
    }

    fn handle_delete_repo_path(&mut self, path: String) -> Vec<Command> {
        self.filter.repos.remove(&path);
        self.input.mode = InputMode::RepoFilter;
        self.set_status("Deleted repo path".to_string());
        vec![Command::DeleteRepoPath(path)]
    }

    fn handle_cancel_preset_input(&mut self) -> Vec<Command> {
        self.input.buffer.clear();
        self.input.mode = InputMode::RepoFilter;
        vec![]
    }

    fn handle_filter_presets_loaded(
        &mut self,
        presets: Vec<(String, HashSet<String>, RepoFilterMode)>,
    ) -> Vec<Command> {
        self.filter.presets = presets;
        vec![]
    }

    // -----------------------------------------------------------------------
    // Extracted handlers (previously inline in update())
    // -----------------------------------------------------------------------

    fn handle_dispatch_failed(&mut self, id: TaskId) -> Vec<Command> {
        self.dispatching.remove(&id);
        vec![]
    }

    fn handle_mark_dispatching(&mut self, id: TaskId) -> Vec<Command> {
        self.dispatching.insert(id);
        vec![]
    }

    fn handle_description_editor_result(&mut self, value: String) -> Vec<Command> {
        match self.input.mode {
            InputMode::InputDescription => self.handle_submit_description(value),
            InputMode::InputEpicDescription => self.handle_submit_epic_description(value),
            _ => vec![],
        }
    }

    /// Router for editor results that come back from a pop-out editor. Each
    /// `EditKind` is finalized by a `FinalizeEditorResult` command dispatched
    /// to the runtime, except the `Description` variant which threads straight
    /// through the existing description-flow messages.
    fn handle_editor_result(&mut self, kind: EditKind, outcome: EditorOutcome) -> Vec<Command> {
        match (&kind, &outcome) {
            (EditKind::Description { .. }, EditorOutcome::Saved(text)) => {
                let text = crate::editor::parse_description_editor_output(text);
                self.update(Message::DescriptionEditorResult(text))
            }
            (EditKind::Description { .. }, EditorOutcome::Cancelled) => {
                self.update(Message::CancelInput)
            }
            _ => vec![Command::FinalizeEditorResult { kind, outcome }],
        }
    }

    fn handle_message_received(&mut self, id: TaskId) -> Vec<Command> {
        self.agents
            .message_flash
            .insert(id, std::time::Instant::now());
        vec![]
    }

    fn handle_open_in_browser(&self, url: String) -> Vec<Command> {
        vec![Command::OpenInBrowser { url }]
    }
    fn finish_epic_creation(&mut self, repo_path: String) -> Vec<Command> {
        let mut draft = self.input.epic_draft.take().unwrap_or_default();
        draft.repo_path = repo_path.clone();
        self.input.mode = InputMode::Normal;
        self.clear_status();
        vec![Command::InsertEpic(draft), Command::SaveRepoPath(repo_path)]
    }
}

#[cfg(test)]
mod tests;
