pub mod input;
pub mod types;
pub mod ui;

pub use types::*;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::dispatch;
use crate::models::{
    epic_status, epic_substatus, DispatchMode, Epic, EpicId, EpicSubstatus, PrRef, ReviewDecision,
    SubStatus, Task, TaskId, TaskStatus, TaskTag, TaskUsage, VisualColumn,
    DEFAULT_QUICK_TASK_TITLE,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How long a transient status message stays visible before auto-clearing.
const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);

/// Interval between PR status polls for tasks in review.
const PR_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Interval between review board data refreshes.
const REVIEW_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// Interval between security board data refreshes (5 minutes).
const SECURITY_POLL_INTERVAL: Duration = Duration::from_secs(300);

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
    pub(in crate::tui) review: ReviewBoardState,
    pub(in crate::tui) security: SecurityBoardState,
    pub(in crate::tui) merge_queue: Option<MergeQueue>,
    /// Task IDs with an in-flight dispatch (worktree + tmux setup running).
    /// Prevents duplicate dispatches when the user presses Enter rapidly.
    pub(in crate::tui) dispatching: HashSet<TaskId>,
    /// Review agent dispatches in-flight, keyed by repo + PR number.
    pub(in crate::tui) dispatching_review: HashSet<PrRef>,
    /// Fix agent dispatches in-flight, keyed by repo + alert number + kind.
    pub(in crate::tui) dispatching_fix: HashSet<FixDispatchKey>,
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

impl App {
    pub fn new(tasks: Vec<Task>, inactivity_timeout: Duration) -> Self {
        App {
            board: BoardState {
                tasks,
                epics: Vec::new(),
                view_mode: ViewMode::default(),
                detail_visible: false,
                repo_paths: Vec::new(),
                usage: HashMap::new(),
                split: SplitState::default(),
            },
            status: StatusState::default(),
            should_quit: false,
            notifications_enabled: true,
            input: InputState::default(),
            agents: AgentTracking::new(inactivity_timeout),
            archive: ArchiveState::default(),
            select: SelectionState::default(),
            filter: FilterState::default(),
            review: ReviewBoardState::default(),
            security: SecurityBoardState::default(),
            merge_queue: None,
            dispatching: HashSet::new(),
            dispatching_review: HashSet::new(),
            dispatching_fix: HashSet::new(),
        }
    }

    /// Returns true if the given task has an in-flight dispatch.
    pub fn is_dispatching(&self, id: TaskId) -> bool {
        self.dispatching.contains(&id)
    }

    /// Returns true if a review agent dispatch is in-flight for the given PR.
    pub fn is_dispatching_review(&self, repo: &str, number: i64) -> bool {
        self.dispatching_review
            .contains(&PrRef::new(repo.to_string(), number))
    }

    /// Returns true if a fix agent dispatch is in-flight for the given alert.
    pub fn is_dispatching_fix(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
    ) -> bool {
        self.dispatching_fix
            .contains(&FixDispatchKey::new(repo.to_string(), number, kind))
    }

    /// Get the current selection state (from whichever view mode is active).
    pub fn selection(&self) -> &BoardSelection {
        match &self.board.view_mode {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
            ViewMode::ReviewBoard { saved_board, .. } => saved_board,
            ViewMode::SecurityBoard { saved_board, .. } => saved_board,
        }
    }

    /// Get mutable access to the current selection state.
    pub(in crate::tui) fn selection_mut(&mut self) -> &mut BoardSelection {
        match &mut self.board.view_mode {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
            ViewMode::ReviewBoard { saved_board, .. } => saved_board,
            ViewMode::SecurityBoard { saved_board, .. } => saved_board,
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
        self.archive.visible
    }
    pub fn selected_archive_row(&self) -> usize {
        self.archive.selected_row
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
    pub fn review_prs(&self) -> &[crate::models::ReviewPr] {
        &self.review.review.prs
    }
    pub fn review_board_loading(&self) -> bool {
        self.review.review.loading
    }
    pub fn last_review_error(&self) -> Option<&str> {
        self.review.review.last_error.as_deref()
    }
    pub fn review_detail_visible(&self) -> bool {
        self.review.detail_visible
    }
    pub fn review_repo_filter(&self) -> &HashSet<String> {
        &self.review.review.repo_filter
    }
    pub fn review_repo_filter_mode(&self) -> RepoFilterMode {
        self.review.review.repo_filter_mode
    }
    pub fn my_prs(&self) -> &[crate::models::ReviewPr] {
        &self.review.authored.prs
    }
    pub fn my_prs_loading(&self) -> bool {
        self.review.authored.loading
    }
    pub fn dispatch_pr_filter(&self) -> bool {
        self.review.dispatch_pr_filter
    }
    pub fn bot_prs(&self) -> &[crate::models::ReviewPr] {
        &self.review.bot.prs
    }
    pub fn bot_prs_loading(&self) -> bool {
        self.review.bot.loading
    }
    pub fn selected_bot_prs(&self) -> &HashSet<String> {
        &self.select.bot_prs
    }
    pub fn has_bot_pr_selection(&self) -> bool {
        self.select.has_bot_pr_selection()
    }

    /// Set of PR URLs from dispatch tasks (for matching against ReviewPr entries).
    pub fn dispatch_pr_urls(&self) -> HashSet<String> {
        self.board
            .tasks
            .iter()
            .filter_map(|t| t.pr_url.clone())
            .collect()
    }

    /// Get the review board selection state, if currently in review board mode.
    pub fn review_selection(&self) -> Option<&ReviewBoardSelection> {
        match &self.board.view_mode {
            ViewMode::ReviewBoard { selection, .. } => Some(selection),
            _ => None,
        }
    }

    pub(in crate::tui) fn review_selection_mut(&mut self) -> Option<&mut ReviewBoardSelection> {
        match &mut self.board.view_mode {
            ViewMode::ReviewBoard { selection, .. } => Some(selection),
            _ => None,
        }
    }

    pub fn security_selection(&self) -> Option<&SecurityBoardSelection> {
        match &self.board.view_mode {
            ViewMode::SecurityBoard { selection, .. } => Some(selection),
            _ => None,
        }
    }

    pub(in crate::tui) fn security_selection_mut(&mut self) -> Option<&mut SecurityBoardSelection> {
        match &mut self.board.view_mode {
            ViewMode::SecurityBoard { selection, .. } => Some(selection),
            _ => None,
        }
    }

    pub fn security_detail_visible(&self) -> bool {
        self.security.detail_visible
    }

    pub fn security_loading(&self) -> bool {
        self.security.loading
    }

    pub fn last_security_error(&self) -> Option<&str> {
        self.security.last_error.as_deref()
    }

    pub fn security_kind_filter(&self) -> Option<crate::models::AlertKind> {
        self.security.kind_filter
    }

    /// Return alerts filtered by the security board's filters.
    pub fn filtered_security_alerts(&self) -> Vec<&crate::models::SecurityAlert> {
        self.security.filtered_alerts()
    }

    /// Return alerts for a specific severity column using the active filter.
    pub fn security_alerts_for_column(&self, col: usize) -> Vec<&crate::models::SecurityAlert> {
        let mut alerts: Vec<_> = self
            .filtered_security_alerts()
            .into_iter()
            .filter(|a| a.severity.column_index() == col)
            .collect();
        alerts.sort_by(|a, b| a.repo.cmp(&b.repo));
        alerts
    }

    /// Get the currently selected SecurityAlert, if in security board mode.
    pub fn selected_security_alert(&self) -> Option<&crate::models::SecurityAlert> {
        let sel = self.security_selection()?;
        let col = sel.column();
        let row = sel.row(col);
        self.security_alerts_for_column(col).into_iter().nth(row)
    }

    pub fn active_security_repos(&self) -> &[String] {
        &self.security.repos
    }

    pub(in crate::tui) fn navigate_security_row(&mut self, delta: isize) {
        let (col, count) = match self.security_selection() {
            Some(sel) => {
                let col = sel.selected_column;
                let count = self.security_alerts_for_column(col).len();
                (col, count)
            }
            None => return,
        };
        if count == 0 {
            return;
        }
        if let Some(sel) = self.security_selection_mut() {
            let current = sel.selected_row[col] as isize;
            let new = (current + delta).clamp(0, (count - 1) as isize) as usize;
            sel.selected_row[col] = new;
        }
    }

    pub(in crate::tui) fn clamp_security_selection(&mut self) {
        let counts: [usize; crate::models::AlertSeverity::COLUMN_COUNT] =
            std::array::from_fn(|col| self.security_alerts_for_column(col).len());
        if let Some(sel) = self.security_selection_mut() {
            for (col, &count) in counts.iter().enumerate() {
                if count == 0 {
                    sel.selected_row[col] = 0;
                } else if sel.selected_row[col] >= count {
                    sel.selected_row[col] = count - 1;
                }
            }
        }
    }

    pub fn set_notifications_enabled(&mut self, enabled: bool) {
        self.notifications_enabled = enabled;
    }

    pub fn set_review_prs(&mut self, prs: Vec<crate::models::ReviewPr>) {
        self.review.review.set_prs(prs);
    }

    pub fn set_bot_prs(&mut self, prs: Vec<crate::models::ReviewPr>) {
        self.review.bot.set_prs(prs);
    }

    pub fn set_security_alerts(&mut self, alerts: Vec<crate::models::SecurityAlert>) {
        self.security.set_alerts(alerts);
    }

    pub fn set_repo_filter(&mut self, filter: HashSet<String>) {
        self.filter.repos = filter;
        self.clamp_selection();
    }

    pub fn set_repo_filter_mode(&mut self, mode: RepoFilterMode) {
        self.filter.mode = mode;
        self.clamp_selection();
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

    /// Return tasks visible in the current view.
    /// Board view: standalone tasks only (epic_id is None).
    /// Epic view: only subtasks of the active epic.
    pub fn tasks_for_current_view(&self) -> Vec<&Task> {
        let repo_match = |t: &&Task| self.repo_matches(&t.repo_path);
        match &self.board.view_mode {
            ViewMode::Board(_) => self
                .board
                .tasks
                .iter()
                .filter(|t| t.epic_id.is_none() && t.status != TaskStatus::Archived)
                .filter(repo_match)
                .collect(),
            ViewMode::Epic { epic_id, .. } => self
                .board
                .tasks
                .iter()
                .filter(|t| t.epic_id == Some(*epic_id) && t.status != TaskStatus::Archived)
                .filter(repo_match)
                .collect(),
            ViewMode::ReviewBoard { .. } | ViewMode::SecurityBoard { .. } => vec![],
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

        if matches!(self.board.view_mode, ViewMode::Board(_)) {
            for epic in &self.board.epics {
                if !self.repo_matches(&epic.repo_path) {
                    continue;
                }
                if epic_status(epic) == status {
                    items.push(ColumnItem::Epic(epic));
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
        if !matches!(self.board.view_mode, ViewMode::Board(_)) {
            return task_count;
        }
        let epic_count = self
            .board
            .epics
            .iter()
            .filter(|e| self.filter.matches(&e.repo_path) && epic_status(e) == status)
            .count();
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

        if matches!(self.board.view_mode, ViewMode::Board(_)) {
            let active_merge = self.merge_queue.as_ref().map(|q| q.epic_id);
            for epic in &self.board.epics {
                if !self.repo_matches(&epic.repo_path) {
                    continue;
                }
                let epic_parent = epic_status(epic);
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
        let status = TaskStatus::from_column_index(col)?;
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
        for (col, &status) in TaskStatus::ALL.iter().enumerate() {
            let count = self.column_item_count(status);
            let sel = self.selection_mut();
            if count == 0 {
                sel.set_row(col, 0);
            } else if sel.row(col) >= count {
                sel.set_row(col, count - 1);
            }
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
            // Board navigation, view toggles, system events
            msg @ (Message::Tick
            | Message::TerminalResized
            | Message::Quit
            | Message::NavigateColumn(_)
            | Message::NavigateRow(_)
            | Message::MoveTask { .. }
            | Message::ReorderItem(_)
            | Message::ToggleDetail
            | Message::ToggleHelp
            | Message::ToggleNotifications
            | Message::ToggleArchive
            | Message::ToggleSplitMode
            | Message::SwapSplitPane(_)
            | Message::SplitPaneOpened { .. }
            | Message::SplitPaneClosed
            | Message::RefreshTasks(_)
            | Message::RefreshUsage(_)
            | Message::Error(_)
            | Message::DismissError
            | Message::StatusInfo(_)
            | Message::RepoPathsUpdated(_)
            | Message::MessageReceived(_)
            | Message::OpenInBrowser { .. }
            | Message::TmuxOutput { .. }
            | Message::WindowGone(_)) => self.dispatch_board(msg),

            // Task lifecycle, dispatch, selection, wrap-up
            msg @ (Message::DispatchTask(..)
            | Message::Dispatched { .. }
            | Message::TaskCreated { .. }
            | Message::DeleteTask(_)
            | Message::ResumeTask(_)
            | Message::Resumed { .. }
            | Message::DispatchFailed(_)
            | Message::TaskEdited(_)
            | Message::StaleAgent(_)
            | Message::AgentCrashed(_)
            | Message::KillAndRetry(_)
            | Message::RetryResume(_)
            | Message::RetryFresh(_)
            | Message::ArchiveTask(_)
            | Message::QuickDispatch { .. }
            | Message::StartQuickDispatchSelection
            | Message::SelectQuickDispatchRepo(_)
            | Message::FinishComplete(_)
            | Message::FinishFailed { .. }
            | Message::ConfirmDone
            | Message::CancelDone
            | Message::StartWrapUp(_)
            | Message::WrapUpRebase
            | Message::WrapUpPr
            | Message::CancelWrapUp
            | Message::DetachTmux(_)
            | Message::BatchDetachTmux(_)
            | Message::ConfirmDetachTmux
            | Message::ToggleSelect(_)
            | Message::ClearSelection
            | Message::SelectAllColumn
            | Message::BatchMoveTasks { .. }
            | Message::BatchArchiveTasks(_)) => self.dispatch_task(msg),

            // Form input, text entry, creation flows
            msg @ (Message::StartNewTask
            | Message::CopyTask
            | Message::CancelInput
            | Message::ConfirmDeleteStart
            | Message::ConfirmDeleteYes
            | Message::CancelDelete
            | Message::SubmitTitle(_)
            | Message::SubmitDescription(_)
            | Message::DescriptionEditorResult(_)
            | Message::SubmitRepoPath(_)
            | Message::SubmitDispatchRepoPath(_)
            | Message::SubmitTag(_)
            | Message::InputChar(_)
            | Message::InputBackspace
            | Message::CancelRetry) => self.dispatch_input(msg),

            // Epic CRUD, lifecycle, wrap-up
            msg @ (Message::DispatchEpic(_)
            | Message::EnterEpic(_)
            | Message::ExitEpic
            | Message::RefreshEpics(_)
            | Message::EpicCreated(_)
            | Message::EditEpic(_)
            | Message::EpicEdited(_)
            | Message::DeleteEpic(_)
            | Message::ConfirmDeleteEpic
            | Message::MoveEpicStatus(_, _)
            | Message::ArchiveEpic(_)
            | Message::ConfirmArchiveEpic
            | Message::StartNewEpic
            | Message::SubmitEpicTitle(_)
            | Message::SubmitEpicDescription(_)
            | Message::SubmitEpicRepoPath(_)
            | Message::StartEpicWrapUp(_)
            | Message::EpicWrapUpRebase
            | Message::EpicWrapUpPr
            | Message::CancelEpicWrapUp
            | Message::CancelMergeQueue
            | Message::ToggleSelectEpic(_)
            | Message::BatchArchiveEpics(_)) => self.dispatch_epic(msg),

            // Review board, PR flow, review agents, bot PRs
            msg @ (Message::SwitchToReviewBoard
            | Message::SwitchToTaskBoard
            | Message::ToggleReviewBoardMode
            | Message::PrsLoaded(..)
            | Message::PrsFetchFailed(..)
            | Message::ToggleReviewDetail
            | Message::DispatchReviewAgent(_)
            | Message::ReviewAgentDispatched { .. }
            | Message::ReviewAgentFailed { .. }
            | Message::RefreshReviewPrs
            | Message::RefreshBotPrs
            | Message::ToggleSelectBotPr(_)
            | Message::SelectAllBotPrColumn
            | Message::ClearBotPrSelection
            | Message::StartBatchApprove
            | Message::StartBatchMerge
            | Message::ConfirmBatchApprove
            | Message::ConfirmBatchMerge
            | Message::CancelBatchOperation
            | Message::PrCreated { .. }
            | Message::PrFailed { .. }
            | Message::PrMerged(_)
            | Message::StartMergePr(_)
            | Message::ConfirmMergePr
            | Message::CancelMergePr
            | Message::MergePrFailed { .. }
            | Message::PrReviewState { .. }
            | Message::ReviewStatusUpdated { .. }
            | Message::DetachReviewAgent { .. }
            | Message::StartReviewRepoFilter
            | Message::CloseReviewRepoFilter
            | Message::ToggleReviewRepoFilter(_)
            | Message::ToggleAllReviewRepoFilter
            | Message::ToggleReviewRepoFilterMode
            | Message::ToggleDispatchPrFilter) => self.dispatch_review(msg),

            // Security board, fix agents, task filters, filter presets
            msg @ (Message::SwitchToSecurityBoard
            | Message::SecurityAlertsLoaded(_)
            | Message::SecurityAlertsFetchFailed(_)
            | Message::RefreshSecurityAlerts
            | Message::ToggleSecurityDetail
            | Message::ToggleSecurityKindFilter
            | Message::StartSecurityRepoFilter
            | Message::CloseSecurityRepoFilter
            | Message::ToggleSecurityRepoFilter(_)
            | Message::ToggleAllSecurityRepoFilter
            | Message::ToggleSecurityRepoFilterMode
            | Message::DispatchFixAgent { .. }
            | Message::FixAgentDispatched { .. }
            | Message::FixAgentFailed { .. }
            | Message::DetachFixAgent { .. }
            | Message::StartRepoFilter
            | Message::CloseRepoFilter
            | Message::ToggleRepoFilter(_)
            | Message::ToggleAllRepoFilter
            | Message::ToggleRepoFilterMode
            | Message::MoveRepoCursor(_)
            | Message::StartSavePreset
            | Message::SaveFilterPreset(_)
            | Message::LoadFilterPreset(_)
            | Message::StartDeletePreset
            | Message::DeleteFilterPreset(_)
            | Message::StartDeleteRepoPath
            | Message::DeleteRepoPath(_)
            | Message::CancelPresetInput
            | Message::FilterPresetsLoaded(_)) => self.dispatch_security_and_filters(msg),
        }
    }

    // -----------------------------------------------------------------------
    // Sub-dispatchers
    // -----------------------------------------------------------------------

    /// Board navigation, view toggles, and system events.
    fn dispatch_board(&mut self, msg: Message) -> Vec<Command> {
        match msg {
            Message::Tick => self.handle_tick(),
            Message::TerminalResized => vec![],
            Message::Quit => self.handle_quit(),
            Message::NavigateColumn(delta) => self.handle_navigate_column(delta),
            Message::NavigateRow(delta) => self.handle_navigate_row(delta),
            Message::MoveTask { id, direction } => self.handle_move_task(id, direction),
            Message::ReorderItem(dir) => self.handle_reorder_item(dir),
            Message::ToggleDetail => self.handle_toggle_detail(),
            Message::ToggleHelp => self.handle_toggle_help(),
            Message::ToggleNotifications => self.handle_toggle_notifications(),
            Message::ToggleArchive => self.handle_toggle_archive(),
            Message::ToggleSplitMode => self.handle_toggle_split_mode(),
            Message::SwapSplitPane(task_id) => self.handle_swap_split_pane(task_id),
            Message::SplitPaneOpened { pane_id, task_id } => {
                self.handle_split_pane_opened(pane_id, task_id)
            }
            Message::SplitPaneClosed => self.handle_split_pane_closed(),
            Message::RefreshTasks(tasks) => self.handle_refresh_tasks(tasks),
            Message::RefreshUsage(usage) => self.handle_refresh_usage(usage),
            Message::Error(msg) => self.handle_error(msg),
            Message::DismissError => self.handle_dismiss_error(),
            Message::StatusInfo(msg) => self.handle_status_info(msg),
            Message::RepoPathsUpdated(paths) => self.handle_repo_paths_updated(paths),
            Message::MessageReceived(id) => self.handle_message_received(id),
            Message::OpenInBrowser { url } => self.handle_open_in_browser(url),
            Message::TmuxOutput {
                id,
                output,
                activity_ts,
            } => self.handle_tmux_output(id, output, activity_ts),
            Message::WindowGone(id) => self.handle_window_gone(id),
            _ => unreachable!(),
        }
    }

    /// Task lifecycle: dispatch, resume, failure recovery, selection, wrap-up.
    fn dispatch_task(&mut self, msg: Message) -> Vec<Command> {
        match msg {
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
            _ => unreachable!(),
        }
    }

    /// Form input: text entry, task creation flows, confirmations.
    fn dispatch_input(&mut self, msg: Message) -> Vec<Command> {
        match msg {
            Message::StartNewTask => self.handle_start_new_task(),
            Message::CopyTask => self.handle_copy_task(),
            Message::CancelInput => self.handle_cancel_input(),
            Message::ConfirmDeleteStart => self.handle_confirm_delete_start(),
            Message::ConfirmDeleteYes => self.handle_confirm_delete_yes(),
            Message::CancelDelete => self.handle_cancel_delete(),
            Message::SubmitTitle(value) => self.handle_submit_title(value),
            Message::SubmitDescription(value) => self.handle_submit_description(value),
            Message::DescriptionEditorResult(value) => self.handle_description_editor_result(value),
            Message::SubmitRepoPath(value) => self.handle_submit_repo_path(value),
            Message::SubmitDispatchRepoPath(value) => self.handle_submit_dispatch_repo_path(value),
            Message::SubmitTag(tag) => self.handle_submit_tag(tag),
            Message::InputChar(c) => self.handle_input_char(c),
            Message::InputBackspace => self.handle_input_backspace(),
            Message::CancelRetry => self.handle_cancel_retry(),
            _ => unreachable!(),
        }
    }

    /// Epic CRUD, lifecycle, and wrap-up.
    fn dispatch_epic(&mut self, msg: Message) -> Vec<Command> {
        match msg {
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
            _ => unreachable!(),
        }
    }

    /// Review board: PR flow, review agents, bot PRs, review filters.
    fn dispatch_review(&mut self, msg: Message) -> Vec<Command> {
        match msg {
            Message::SwitchToReviewBoard => self.handle_switch_to_review_board(),
            Message::SwitchToTaskBoard => self.handle_switch_to_task_board(),
            Message::ToggleReviewBoardMode => self.handle_toggle_review_board_mode(),
            Message::PrsLoaded(kind, prs) => self.handle_prs_loaded(kind, prs),
            Message::PrsFetchFailed(kind, err) => self.handle_prs_fetch_failed(kind, err),
            Message::ToggleReviewDetail => self.handle_toggle_review_detail(),
            Message::DispatchReviewAgent(req) => self.handle_dispatch_review_agent(req),
            Message::ReviewAgentDispatched {
                github_repo,
                number,
                tmux_window,
                worktree,
            } => self.handle_review_agent_dispatched(github_repo, number, tmux_window, worktree),
            Message::ReviewAgentFailed {
                github_repo,
                number,
                error,
            } => self.handle_review_agent_failed(github_repo, number, error),
            Message::RefreshReviewPrs => self.handle_refresh_review_prs(),
            Message::RefreshBotPrs => self.handle_refresh_bot_prs(),
            Message::ToggleSelectBotPr(url) => self.handle_toggle_select_bot_pr(url),
            Message::SelectAllBotPrColumn => self.handle_select_all_bot_pr_column(),
            Message::ClearBotPrSelection => self.handle_clear_bot_pr_selection(),
            Message::StartBatchApprove => self.handle_start_batch_approve(),
            Message::StartBatchMerge => self.handle_start_batch_merge(),
            Message::ConfirmBatchApprove => self.handle_confirm_batch_approve(),
            Message::ConfirmBatchMerge => self.handle_confirm_batch_merge(),
            Message::CancelBatchOperation => self.handle_cancel_batch_operation(),
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
            Message::ReviewStatusUpdated {
                repo,
                number,
                status,
            } => self.handle_review_status_updated(repo, number, status),
            Message::DetachReviewAgent { repo, number } => {
                self.handle_detach_review_agent(repo, number)
            }
            Message::StartReviewRepoFilter => self.handle_start_review_repo_filter(),
            Message::CloseReviewRepoFilter => self.handle_close_review_repo_filter(),
            Message::ToggleReviewRepoFilter(repo) => self.handle_toggle_review_repo_filter(repo),
            Message::ToggleAllReviewRepoFilter => self.handle_toggle_all_review_repo_filter(),
            Message::ToggleReviewRepoFilterMode => self.handle_toggle_review_repo_filter_mode(),
            Message::ToggleDispatchPrFilter => self.handle_toggle_dispatch_pr_filter(),
            _ => unreachable!(),
        }
    }

    /// Security board, fix agents, task repo filters, and filter presets.
    fn dispatch_security_and_filters(&mut self, msg: Message) -> Vec<Command> {
        match msg {
            Message::SwitchToSecurityBoard => self.handle_switch_to_security_board(),
            Message::SecurityAlertsLoaded(alerts) => self.handle_security_alerts_loaded(alerts),
            Message::SecurityAlertsFetchFailed(err) => {
                self.handle_security_alerts_fetch_failed(err)
            }
            Message::RefreshSecurityAlerts => self.handle_refresh_security_alerts(),
            Message::ToggleSecurityDetail => self.handle_toggle_security_detail(),
            Message::ToggleSecurityKindFilter => self.handle_toggle_security_kind_filter(),
            Message::StartSecurityRepoFilter => self.handle_start_security_repo_filter(),
            Message::CloseSecurityRepoFilter => self.handle_close_security_repo_filter(),
            Message::ToggleSecurityRepoFilter(repo) => {
                self.handle_toggle_security_repo_filter(repo)
            }
            Message::ToggleAllSecurityRepoFilter => self.handle_toggle_all_security_repo_filter(),
            Message::ToggleSecurityRepoFilterMode => self.handle_toggle_security_repo_filter_mode(),
            Message::DispatchFixAgent(req) => self.handle_dispatch_fix_agent(req),
            Message::FixAgentDispatched {
                github_repo,
                number,
                kind,
                tmux_window,
                worktree,
            } => self.handle_fix_agent_dispatched(github_repo, number, kind, tmux_window, worktree),
            Message::FixAgentFailed {
                github_repo,
                number,
                kind,
                error,
            } => self.handle_fix_agent_failed(github_repo, number, kind, error),
            Message::DetachFixAgent { repo, number, kind } => {
                self.handle_detach_fix_agent(repo, number, kind)
            }
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
            _ => unreachable!(),
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
        let new_col = (self.selection().column() as isize + delta)
            .clamp(0, (TaskStatus::COLUMN_COUNT - 1) as isize) as usize;
        self.selection_mut().set_column(new_col);
        self.clamp_selection();
        vec![]
    }

    fn handle_navigate_row(&mut self, delta: isize) -> Vec<Command> {
        let col = self.selection().column();
        if col >= TaskStatus::COLUMN_COUNT {
            return vec![];
        }
        let status = match TaskStatus::from_column_index(col) {
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
        vec![]
    }

    fn handle_reorder_item(&mut self, direction: isize) -> Vec<Command> {
        let col = self.selection().column();
        let Some(status) = TaskStatus::from_column_index(col) else {
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
                let title = truncate_title(&task.title, 30);
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
            self.clamp_selection();

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
            }
        }
        self.select.tasks.clear();
        self.clamp_selection();
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
            self.agents.last_output_change.insert(id, Instant::now());
            self.clamp_selection();
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
        self.clamp_selection();
        vec![]
    }

    fn handle_delete_task(&mut self, id: TaskId) -> Vec<Command> {
        let cleanup = self.find_task_mut(id).and_then(Self::take_cleanup);
        self.clear_agent_tracking(id);
        self.board.tasks.retain(|t| t.id != id);
        self.clamp_selection();
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

    fn handle_toggle_review_detail(&mut self) -> Vec<Command> {
        self.review.detail_visible = !self.review.detail_visible;
        vec![]
    }

    fn handle_tmux_output(&mut self, id: TaskId, output: String, activity_ts: u64) -> Vec<Command> {
        let mut cmds = Vec::new();
        let activity_changed = self
            .agents
            .last_activity
            .get(&id)
            .is_none_or(|&prev| prev != activity_ts);
        if activity_changed {
            self.agents.last_output_change.insert(id, Instant::now());
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
            self.agents.last_activity.insert(id, activity_ts);
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
        self.clamp_selection();
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
        self.review
            .review_flash
            .retain(|_, t| t.elapsed().as_secs() < 3);
        self.security
            .review_flash
            .retain(|_, t| t.elapsed().as_secs() < 3);

        // Skip capturing the split-pinned task: its window has been joined as a
        // pane and is no longer visible to `has_window`, which would falsely
        // trigger WindowGone → Crashed.
        let split_pinned = self.board.split.pinned_task_id.filter(|_| self.board.split.active);

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
                    .last_output_change
                    .get(&t.id)
                    .is_some_and(|instant| instant.elapsed() > timeout)
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

        // Refresh review board data if stale (> 30s), regardless of active tab
        if self.review.review.needs_fetch(REVIEW_REFRESH_INTERVAL) && !self.review.review.loading {
            self.review.review.loading = true;
            cmds.push(Command::FetchPrs(PrListKind::Review));
        }

        // Also refresh my PRs data if stale (> 30s)
        if self.review.authored.needs_fetch(REVIEW_REFRESH_INTERVAL)
            && !self.review.authored.loading
        {
            self.review.authored.loading = true;
            cmds.push(Command::FetchPrs(PrListKind::Authored));
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
            .last_output_change
            .get(&id)
            .map(|t| t.elapsed().as_secs() / 60)
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

        let mut cmds = Vec::new();

        if let Some(task) = self.find_task_mut(id) {
            task.sub_status = SubStatus::Crashed;
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
            self.agents.last_output_change.insert(id, Instant::now());
            self.clamp_selection();
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
            t.updated_at = chrono::Utc::now();
        }
        self.clamp_selection();
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
            self.clamp_selection();

            let mut cmds = Vec::new();
            if let Some(c) = cleanup {
                cmds.push(c);
            }
            cmds.push(Command::PersistTask(task_clone));
            cmds
        } else {
            vec![]
        }
    }

    fn handle_toggle_archive(&mut self) -> Vec<Command> {
        self.archive.visible = !self.archive.visible;
        if self.archive.visible {
            self.archive.selected_row = 0;
            *self.archive.list_state.selected_mut() = Some(0);
        }
        vec![]
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
        let Some(status) = TaskStatus::from_column_index(col) else {
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
        self.input.pending_dispatch = None;
        self.clear_status();
        vec![]
    }

    fn handle_confirm_delete_start(&mut self) -> Vec<Command> {
        if let Some(task) = self.selected_task() {
            let title = truncate_title(&task.title, 30);
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
        self.input.mode = InputMode::InputRepoPath;
        self.set_status("Enter repo path: ".to_string());
        vec![]
    }

    fn handle_submit_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        let repo_path = if value.is_empty() {
            if let Some(first) = self.board.repo_paths.first() {
                first.clone()
            } else {
                self.set_status("Repo path required (no saved paths available)".to_string());
                return vec![];
            }
        } else {
            value
        };
        if let Err(msg) = crate::dispatch::validate_repo_path(&repo_path) {
            self.set_status(msg);
            return vec![];
        }
        self.finish_task_creation(repo_path)
    }

    fn handle_submit_tag(&mut self, tag: Option<TaskTag>) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.task_draft {
            draft.tag = tag;
        }
        self.input.mode = InputMode::InputDescription;
        self.set_status("Opening editor for description...".to_string());
        vec![Command::OpenDescriptionEditor { is_epic: false }]
    }

    fn handle_input_char(&mut self, c: char) -> Vec<Command> {
        // In repo path mode with empty buffer, 1-9 selects a saved path
        if (self.input.mode == InputMode::InputRepoPath
            || self.input.mode == InputMode::InputEpicRepoPath)
            && self.input.buffer.is_empty()
            && c.is_ascii_digit()
            && c != '0'
        {
            let idx = (c as usize) - ('1' as usize);
            if idx < self.board.repo_paths.len() {
                let repo_path = self.board.repo_paths[idx].clone();
                if self.input.mode == InputMode::InputEpicRepoPath {
                    return self.finish_epic_creation(repo_path);
                }
                return self.finish_task_creation(repo_path);
            }
        }
        self.input.buffer.push(c);
        vec![]
    }

    fn handle_input_backspace(&mut self) -> Vec<Command> {
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

    fn handle_toggle_split_mode(&mut self) -> Vec<Command> {
        if self.board.split.active {
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
        } else {
            vec![Command::EnterSplitMode]
        }
    }

    fn handle_swap_split_pane(&mut self, task_id: TaskId) -> Vec<Command> {
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
        self.board.split.right_pane_id = Some(pane_id);
        self.board.split.pinned_task_id = task_id;
        vec![]
    }

    fn handle_split_pane_closed(&mut self) -> Vec<Command> {
        self.board.split.active = false;
        self.board.split.right_pane_id = None;
        self.board.split.pinned_task_id = None;
        vec![]
    }

    fn finish_task_creation(&mut self, repo_path: String) -> Vec<Command> {
        let mut draft = self.input.task_draft.take().unwrap_or_default();
        draft.repo_path = repo_path.clone();
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
            self.clamp_selection();
            if !in_queue {
                self.set_status(format!("Task {} finished", id));
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
            self.clamp_selection();
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
        let title = truncate_title(&task.title, 30);

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
                        let title = t.title.clone();
                        let description = t.description.clone();
                        let tmux_window = t.tmux_window.clone();
                        branch.map(|b| (worktree, b, repo_path, title, description, tmux_window))
                    }
                    None => None,
                },
                _ => None,
            };

            let Some((worktree, branch, repo_path, title, description, tmux_window)) = task_data
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
                        worktree,
                        tmux_window,
                    }]
                }
                MergeAction::Pr => vec![Command::CreatePr {
                    id: next_id,
                    repo_path,
                    branch,
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
    // Review board handlers
    // -----------------------------------------------------------------------

    fn handle_switch_to_security_board(&mut self) -> Vec<Command> {
        let saved_board = match &self.board.view_mode {
            ViewMode::Board(sel) => sel.clone(),
            ViewMode::Epic { saved_board, .. } => saved_board.clone(),
            ViewMode::ReviewBoard { saved_board, .. } => saved_board.clone(),
            ViewMode::SecurityBoard { saved_board, .. } => saved_board.clone(),
        };
        self.board.view_mode = ViewMode::SecurityBoard {
            selection: SecurityBoardSelection::new(),
            saved_board,
        };
        if self.security.needs_fetch(SECURITY_POLL_INTERVAL) && !self.security.loading {
            self.security.loading = true;
            vec![Command::FetchSecurityAlerts]
        } else {
            vec![]
        }
    }

    fn handle_security_alerts_loaded(
        &mut self,
        alerts: Vec<crate::models::SecurityAlert>,
    ) -> Vec<Command> {
        let cmds = vec![Command::PersistSecurityAlerts(alerts.clone())];
        self.security.set_alerts(alerts);
        self.security.loading = false;
        self.security.last_fetch = Some(Instant::now());
        self.security.last_error = None;
        self.clamp_security_selection();
        cmds
    }

    fn handle_switch_to_review_board(&mut self) -> Vec<Command> {
        let saved_board = match &self.board.view_mode {
            ViewMode::Board(sel) => sel.clone(),
            ViewMode::Epic { saved_board, .. } => saved_board.clone(),
            ViewMode::ReviewBoard { saved_board, .. } => saved_board.clone(),
            ViewMode::SecurityBoard { saved_board, .. } => saved_board.clone(),
        };
        self.board.view_mode = ViewMode::ReviewBoard {
            mode: ReviewBoardMode::Reviewer,
            selection: ReviewBoardSelection::new(),
            saved_board,
        };
        if self.review.review.needs_fetch(REVIEW_REFRESH_INTERVAL) && !self.review.review.loading {
            self.review.review.loading = true;
            vec![Command::FetchPrs(PrListKind::Review)]
        } else {
            vec![]
        }
    }

    fn handle_switch_to_task_board(&mut self) -> Vec<Command> {
        match &self.board.view_mode {
            ViewMode::ReviewBoard { saved_board, .. }
            | ViewMode::SecurityBoard { saved_board, .. } => {
                self.board.view_mode = ViewMode::Board(saved_board.clone());
            }
            _ => {}
        }
        vec![]
    }

    fn handle_toggle_review_board_mode(&mut self) -> Vec<Command> {
        let ViewMode::ReviewBoard { mode, .. } = &mut self.board.view_mode else {
            return vec![];
        };
        *mode = match mode {
            ReviewBoardMode::Reviewer => ReviewBoardMode::Author,
            ReviewBoardMode::Author => ReviewBoardMode::Dependabot,
            ReviewBoardMode::Dependabot => ReviewBoardMode::Reviewer,
        };
        self.clamp_review_selection();
        let mut cmds = vec![];
        if let ViewMode::ReviewBoard { mode, .. } = &self.board.view_mode {
            match mode {
                ReviewBoardMode::Author => {
                    if self.review.authored.needs_fetch(REVIEW_REFRESH_INTERVAL)
                        && !self.review.authored.loading
                    {
                        self.review.authored.loading = true;
                        cmds.push(Command::FetchPrs(PrListKind::Authored));
                    }
                }
                ReviewBoardMode::Reviewer => {
                    if self.review.review.needs_fetch(REVIEW_REFRESH_INTERVAL)
                        && !self.review.review.loading
                    {
                        self.review.review.loading = true;
                        cmds.push(Command::FetchPrs(PrListKind::Review));
                    }
                }
                ReviewBoardMode::Dependabot => {
                    if self.review.bot.needs_fetch(REVIEW_REFRESH_INTERVAL)
                        && !self.review.bot.loading
                    {
                        self.review.bot.loading = true;
                        cmds.push(Command::FetchPrs(PrListKind::Bot));
                    }
                }
            }
        }
        cmds
    }

    fn handle_prs_loaded(
        &mut self,
        kind: PrListKind,
        prs: Vec<crate::models::ReviewPr>,
    ) -> Vec<Command> {
        let cmds = vec![Command::PersistPrs(kind, prs.clone())];
        let list = self.review.list_mut(kind);
        list.set_prs(prs);
        list.loading = false;
        list.last_fetch = Some(Instant::now());
        list.last_error = None;
        self.clamp_review_selection();
        cmds
    }

    fn clamp_review_selection(&mut self) {
        let mode = match &self.board.view_mode {
            ViewMode::ReviewBoard { mode, .. } => *mode,
            _ => ReviewBoardMode::Reviewer,
        };
        let filtered = self.active_review_prs();
        let col_count = mode.column_count();
        let counts: [usize; ReviewDecision::COLUMN_COUNT] = std::array::from_fn(|col| {
            if col >= col_count {
                return 0;
            }
            filtered
                .iter()
                .filter(|pr| mode.pr_column(pr) == col)
                .count()
        });
        if let Some(sel) = self.review_selection_mut() {
            for (col, &count) in counts.iter().enumerate() {
                if count == 0 {
                    sel.selected_row[col] = 0;
                } else if sel.selected_row[col] >= count {
                    sel.selected_row[col] = count - 1;
                }
            }
        }
    }

    fn handle_prs_fetch_failed(&mut self, kind: PrListKind, error: String) -> Vec<Command> {
        tracing::warn!(kind = kind.label(), error = %error, "PR fetch failed");
        let list = self.review.list_mut(kind);
        list.loading = false;
        list.last_error = Some(error.clone());
        self.set_status(format!("Failed to fetch {} PRs: {error}", kind.label()));
        vec![]
    }

    fn handle_dispatch_review_agent(&mut self, mut req: ReviewAgentRequest) -> Vec<Command> {
        let key = PrRef::new(req.github_repo.clone(), req.number);
        if self.dispatching_review.contains(&key) {
            return vec![];
        }
        let known = self.known_repo_paths();
        if let Some(path) = dispatch::resolve_repo_path(&req.repo, &known) {
            req.repo = path;
            self.dispatching_review.insert(key);
            self.set_status(format!("Dispatching review agent for #{}...", req.number));
            vec![Command::DispatchReviewAgent(req)]
        } else {
            self.set_status(format!(
                "No local repo found for {} — select a path",
                req.repo
            ));
            self.input.pending_dispatch = Some(PendingDispatch::Review(req));
            self.input.mode = InputMode::InputDispatchRepoPath;
            self.input.buffer.clear();
            self.input.repo_cursor = 0;
            vec![]
        }
    }

    fn handle_submit_dispatch_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        let repo_path = if value.is_empty() {
            if let Some(first) = self.board.repo_paths.first() {
                first.clone()
            } else {
                self.set_status("Repo path required (no saved paths available)".to_string());
                return vec![];
            }
        } else {
            value
        };
        if let Err(msg) = crate::dispatch::validate_repo_path(&repo_path) {
            self.set_status(msg);
            return vec![];
        }
        self.input.mode = InputMode::Normal;
        let pending = self.input.pending_dispatch.take();
        match pending {
            Some(PendingDispatch::Review(mut req)) => {
                let save = Command::SaveRepoPath(repo_path.clone());
                req.repo = repo_path;
                self.set_status(format!("Dispatching review agent for #{}...", req.number));
                vec![Command::DispatchReviewAgent(req), save]
            }
            Some(PendingDispatch::Fix(mut req)) => {
                self.set_status(format!(
                    "Dispatching fix agent for {}#{}...",
                    req.github_repo, req.number
                ));
                req.repo = repo_path.clone();
                vec![
                    Command::DispatchFixAgent(req),
                    Command::SaveRepoPath(repo_path),
                ]
            }
            None => {
                self.set_status("No pending dispatch".to_string());
                vec![]
            }
        }
    }

    /// Delegate to `ReviewBoardState::find_and_set_pr_agent`.
    pub(in crate::tui) fn find_and_set_pr_agent(
        &mut self,
        github_repo: &str,
        number: i64,
        tmux_window: &str,
        worktree: &str,
    ) -> crate::db::PrKind {
        self.review
            .find_and_set_pr_agent(github_repo, number, tmux_window, worktree)
    }

    /// Collect known local repo paths from saved paths and existing tasks.
    fn known_repo_paths(&self) -> Vec<String> {
        let mut known = self.board.repo_paths.clone();
        for t in &self.board.tasks {
            if !known.contains(&t.repo_path) {
                known.push(t.repo_path.clone());
            }
        }
        known
    }

    fn handle_select_all_bot_pr_column(&mut self) -> Vec<Command> {
        let mode = match &self.board.view_mode {
            ViewMode::ReviewBoard { mode, .. } => *mode,
            _ => return vec![],
        };
        let sel = match self.review_selection() {
            Some(s) => s.selected_column,
            None => return vec![],
        };
        let prs = self.filtered_bot_prs();
        let column_urls: Vec<String> = prs
            .iter()
            .filter(|pr| mode.pr_column(pr) == sel)
            .map(|pr| pr.url.clone())
            .collect();
        let all_selected = column_urls.iter().all(|u| self.select.bot_prs.contains(u));
        if all_selected {
            for u in &column_urls {
                self.select.bot_prs.remove(u);
            }
        } else {
            for u in column_urls {
                self.select.bot_prs.insert(u);
            }
        }
        vec![]
    }

    fn handle_start_batch_approve(&mut self) -> Vec<Command> {
        if self.select.bot_prs.is_empty() {
            return vec![];
        }
        let urls: Vec<String> = self.select.bot_prs.iter().cloned().collect();
        self.input.mode = InputMode::ConfirmBatchApprove(urls);
        vec![]
    }

    fn handle_start_batch_merge(&mut self) -> Vec<Command> {
        if self.select.bot_prs.is_empty() {
            return vec![];
        }
        // Only merge PRs that are CI-passing and approved
        let eligible: Vec<String> = self
            .review
            .bot
            .prs
            .iter()
            .filter(|pr| self.select.bot_prs.contains(&pr.url))
            .filter(|pr| {
                pr.ci_status == crate::models::CiStatus::Success
                    && pr.review_decision == crate::models::ReviewDecision::Approved
            })
            .map(|pr| pr.url.clone())
            .collect();
        if eligible.is_empty() {
            self.set_status("No eligible PRs to merge (need CI passing + approved)".into());
            return vec![];
        }
        self.input.mode = InputMode::ConfirmBatchMerge(eligible);
        vec![]
    }

    fn handle_confirm_batch_approve(&mut self) -> Vec<Command> {
        let urls = match std::mem::replace(&mut self.input.mode, InputMode::Normal) {
            InputMode::ConfirmBatchApprove(urls) => urls,
            other => {
                self.input.mode = other;
                return vec![];
            }
        };
        self.select.bot_prs.clear();
        self.set_status(format!("Approving {} PRs...", urls.len()));
        vec![Command::BatchApprovePrs(urls)]
    }

    fn handle_confirm_batch_merge(&mut self) -> Vec<Command> {
        let urls = match std::mem::replace(&mut self.input.mode, InputMode::Normal) {
            InputMode::ConfirmBatchMerge(urls) => urls,
            other => {
                self.input.mode = other;
                return vec![];
            }
        };
        self.select.bot_prs.clear();
        self.set_status(format!("Merging {} PRs...", urls.len()));
        vec![Command::BatchMergePrs(urls)]
    }

    /// Return review PRs filtered by the review repo filter.
    /// When the filter is empty, all PRs are returned.
    pub fn filtered_review_prs(&self) -> Vec<&crate::models::ReviewPr> {
        self.review.review.filtered()
    }

    pub fn filtered_my_prs(&self) -> Vec<&crate::models::ReviewPr> {
        let base = self.review.authored.filtered();
        if self.review.dispatch_pr_filter {
            let dispatch_urls = self.dispatch_pr_urls();
            base.into_iter()
                .filter(|pr| dispatch_urls.contains(&pr.url))
                .collect()
        } else {
            base
        }
    }

    pub fn filtered_bot_prs(&self) -> Vec<&crate::models::ReviewPr> {
        self.review.bot.filtered()
    }

    /// Return the PR list appropriate for the current review board mode.
    pub fn active_review_prs(&self) -> Vec<&crate::models::ReviewPr> {
        match &self.board.view_mode {
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Author,
                ..
            } => self.filtered_my_prs(),
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Dependabot,
                ..
            } => self.filtered_bot_prs(),
            _ => self.filtered_review_prs(),
        }
    }

    /// Sorted distinct repos for the currently active review board mode.
    pub fn active_review_repos(&self) -> &[String] {
        match &self.board.view_mode {
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Author,
                ..
            } => &self.review.authored.repos,
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Dependabot,
                ..
            } => &self.review.bot.repos,
            _ => &self.review.review.repos,
        }
    }

    /// Get PRs for a specific review decision column using the active PR list.
    pub fn active_prs_by_decision(
        &self,
        decision: crate::models::ReviewDecision,
    ) -> Vec<&crate::models::ReviewPr> {
        self.active_review_prs()
            .into_iter()
            .filter(|pr| pr.review_decision == decision)
            .collect()
    }

    /// Return PRs for a given column index, using the current mode's column mapping.
    pub fn active_prs_for_column(&self, col: usize) -> Vec<&crate::models::ReviewPr> {
        let mode = match &self.board.view_mode {
            ViewMode::ReviewBoard { mode, .. } => *mode,
            _ => ReviewBoardMode::Reviewer,
        };
        let mut prs: Vec<_> = self
            .active_review_prs()
            .into_iter()
            .filter(|pr| mode.pr_column(pr) == col)
            .collect();
        prs.sort_by(|a, b| a.repo.cmp(&b.repo));
        prs
    }

    /// Get the currently selected ReviewPr, if in review board mode.
    pub fn selected_review_pr(&self) -> Option<&crate::models::ReviewPr> {
        let sel = self.review_selection()?;
        let col = sel.column();
        let row = sel.row(col);
        self.active_prs_for_column(col).into_iter().nth(row)
    }

    pub(in crate::tui) fn navigate_review_row(&mut self, delta: isize) {
        let (col, count) = match self.review_selection() {
            Some(sel) => {
                let col = sel.selected_column;
                let count = self.active_prs_for_column(col).len();
                (col, count)
            }
            None => return,
        };
        if count == 0 {
            return;
        }
        if let Some(sel) = self.review_selection_mut() {
            let current = sel.selected_row[col] as isize;
            let new = (current + delta).clamp(0, (count - 1) as isize) as usize;
            sel.selected_row[col] = new;
        }
    }

    /// Get PRs for a specific review decision column (respects active filter).
    pub fn review_prs_by_decision(
        &self,
        decision: crate::models::ReviewDecision,
    ) -> Vec<&crate::models::ReviewPr> {
        self.filtered_review_prs()
            .into_iter()
            .filter(|pr| pr.review_decision == decision)
            .collect()
    }

    // -----------------------------------------------------------------------
    // Epic handlers
    // -----------------------------------------------------------------------

    fn handle_dispatch_epic(&mut self, id: EpicId) -> Vec<Command> {
        let Some(epic) = self.board.epics.iter().find(|e| e.id == id) else {
            return vec![];
        };
        let status = crate::models::epic_status(epic);

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

    fn handle_enter_epic(&mut self, epic_id: EpicId) -> Vec<Command> {
        let saved_board = match &self.board.view_mode {
            ViewMode::Board(sel) => sel.clone(),
            ViewMode::Epic { saved_board, .. } => saved_board.clone(),
            ViewMode::ReviewBoard { saved_board, .. } => saved_board.clone(),
            ViewMode::SecurityBoard { saved_board, .. } => saved_board.clone(),
        };
        self.board.view_mode = ViewMode::Epic {
            epic_id,
            selection: BoardSelection::new(),
            saved_board,
        };
        self.board.detail_visible = false;
        vec![]
    }

    fn handle_exit_epic(&mut self) -> Vec<Command> {
        if let ViewMode::Epic { saved_board, .. } = &self.board.view_mode {
            self.board.view_mode = ViewMode::Board(saved_board.clone());
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
            vec![Command::EditEpicInEditor(epic.clone())]
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
        self.clamp_selection();
        cmds.push(Command::DeleteEpic(id));
        cmds
    }

    fn handle_confirm_delete_epic(&mut self) -> Vec<Command> {
        if let Some(ColumnItem::Epic(epic)) = self.selected_column_item() {
            let title = truncate_title(&epic.title, 30);
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
        self.clamp_selection();
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
        self.clamp_selection();
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
        self.input.epic_draft = None;
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
            self.input.epic_draft = Some(EpicDraft {
                title: value,
                description: String::new(),
                repo_path: String::new(),
            });
            self.input.mode = InputMode::InputEpicDescription;
            self.set_status("Opening editor for description...".to_string());
            vec![Command::OpenDescriptionEditor { is_epic: true }]
        }
    }

    fn handle_submit_epic_description(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        if let Some(ref mut draft) = self.input.epic_draft {
            draft.description = value;
        }
        self.input.mode = InputMode::InputEpicRepoPath;
        self.set_status("Epic repo path: ".to_string());
        vec![]
    }

    fn handle_submit_epic_repo_path(&mut self, value: String) -> Vec<Command> {
        self.input.buffer.clear();
        let repo_path = if value.is_empty() {
            if let Some(first) = self.board.repo_paths.first() {
                first.clone()
            } else {
                self.set_status("Repo path required".to_string());
                return vec![];
            }
        } else {
            value
        };
        if let Err(msg) = crate::dispatch::validate_repo_path(&repo_path) {
            self.set_status(msg);
            return vec![];
        }
        self.finish_epic_creation(repo_path)
    }

    fn handle_start_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::RepoFilter;
        self.input.repo_cursor = 0;
        vec![]
    }

    fn handle_move_repo_cursor(&mut self, delta: isize) -> Vec<Command> {
        let count = self.board.repo_paths.len();
        if count == 0 {
            return vec![];
        }
        self.input.repo_cursor =
            (self.input.repo_cursor as isize + delta).rem_euclid(count as isize) as usize;
        vec![]
    }

    fn handle_close_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clamp_selection();
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
        self.clamp_selection();
        vec![]
    }

    fn handle_toggle_all_repo_filter(&mut self) -> Vec<Command> {
        if self.filter.repos.len() == self.board.repo_paths.len() {
            self.filter.repos.clear();
        } else {
            self.filter.repos = self.board.repo_paths.iter().cloned().collect();
        }
        self.clamp_selection();
        vec![]
    }

    fn handle_toggle_repo_filter_mode(&mut self) -> Vec<Command> {
        self.filter.mode = match self.filter.mode {
            RepoFilterMode::Include => RepoFilterMode::Exclude,
            RepoFilterMode::Exclude => RepoFilterMode::Include,
        };
        self.clamp_selection();
        vec![]
    }

    fn handle_start_review_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::ReviewRepoFilter;
        vec![]
    }

    fn handle_close_review_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clamp_review_selection();
        vec![]
    }

    fn handle_toggle_review_repo_filter(&mut self, repo: String) -> Vec<Command> {
        if !self.review.review.repo_filter.remove(&repo) {
            self.review.review.repo_filter.insert(repo);
        }
        self.clamp_review_selection();
        vec![]
    }

    fn handle_toggle_all_review_repo_filter(&mut self) -> Vec<Command> {
        let all_repos = self.active_review_repos();
        if self.review.review.repo_filter.len() == all_repos.len() {
            self.review.review.repo_filter.clear();
        } else {
            self.review.review.repo_filter = all_repos.iter().cloned().collect();
        }
        self.clamp_review_selection();
        vec![]
    }

    fn handle_toggle_review_repo_filter_mode(&mut self) -> Vec<Command> {
        self.review.review.repo_filter_mode = match self.review.review.repo_filter_mode {
            RepoFilterMode::Include => RepoFilterMode::Exclude,
            RepoFilterMode::Exclude => RepoFilterMode::Include,
        };
        self.clamp_review_selection();
        vec![]
    }

    fn handle_toggle_dispatch_pr_filter(&mut self) -> Vec<Command> {
        self.review.dispatch_pr_filter = !self.review.dispatch_pr_filter;
        self.clamp_review_selection();
        vec![]
    }

    fn handle_start_save_preset(&mut self) -> Vec<Command> {
        self.input.buffer.clear();
        self.input.mode = InputMode::InputPresetName;
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
            self.clamp_selection();
            self.set_status(format!("Loaded preset \"{name}\""));
        }
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

    fn handle_description_editor_result(&mut self, value: String) -> Vec<Command> {
        match self.input.mode {
            InputMode::InputDescription => self.handle_submit_description(value),
            InputMode::InputEpicDescription => self.handle_submit_epic_description(value),
            _ => vec![],
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

    fn handle_review_agent_dispatched(
        &mut self,
        github_repo: String,
        number: i64,
        tmux_window: String,
        worktree: String,
    ) -> Vec<Command> {
        self.dispatching_review
            .remove(&PrRef::new(github_repo.clone(), number));
        let repo_short = github_repo.split('/').next_back().unwrap_or(&github_repo);
        self.set_status(format!("Review agent dispatched for {repo_short}#{number}"));
        let pr_kind = self.find_and_set_pr_agent(&github_repo, number, &tmux_window, &worktree);
        vec![Command::PersistReviewAgent {
            pr_kind,
            github_repo,
            number,
            tmux_window,
            worktree,
        }]
    }

    fn handle_review_agent_failed(
        &mut self,
        github_repo: String,
        number: i64,
        error: String,
    ) -> Vec<Command> {
        self.dispatching_review
            .remove(&PrRef::new(github_repo, number));
        self.set_status(format!("Review dispatch failed: {error}"));
        vec![]
    }

    fn handle_refresh_review_prs(&mut self) -> Vec<Command> {
        let kind = match &self.board.view_mode {
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Author,
                ..
            } => PrListKind::Authored,
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Dependabot,
                ..
            } => PrListKind::Bot,
            _ => PrListKind::Review,
        };
        self.review.list_mut(kind).loading = true;
        vec![Command::FetchPrs(kind)]
    }

    fn handle_refresh_bot_prs(&mut self) -> Vec<Command> {
        self.review.bot.loading = true;
        vec![Command::FetchPrs(PrListKind::Bot)]
    }

    fn handle_toggle_select_bot_pr(&mut self, url: String) -> Vec<Command> {
        if !self.select.bot_prs.remove(&url) {
            self.select.bot_prs.insert(url);
        }
        vec![]
    }

    fn handle_clear_bot_pr_selection(&mut self) -> Vec<Command> {
        self.select.bot_prs.clear();
        vec![]
    }

    fn handle_cancel_batch_operation(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        vec![]
    }

    fn handle_review_status_updated(
        &mut self,
        repo: String,
        number: i64,
        status: crate::models::ReviewAgentStatus,
    ) -> Vec<Command> {
        // Update in-memory state across all PR lists
        for pr in self
            .review
            .review
            .prs
            .iter_mut()
            .chain(self.review.authored.prs.iter_mut())
            .chain(self.review.bot.prs.iter_mut())
        {
            if pr.repo == repo && pr.number == number {
                pr.agent_status = Some(status);
            }
        }
        for alert in self.security.alerts.iter_mut() {
            if alert.repo == repo && alert.number == number {
                alert.agent_status = Some(status);
            }
        }
        // Insert flash on findings_ready
        if status == crate::models::ReviewAgentStatus::FindingsReady {
            let now = std::time::Instant::now();
            self.review
                .review_flash
                .insert(crate::models::PrRef::new(repo.clone(), number), now);
            self.security
                .review_flash
                .insert(crate::models::PrRef::new(repo, number), now);
        }
        vec![]
    }

    fn handle_detach_review_agent(&mut self, repo: String, number: i64) -> Vec<Command> {
        let mut cmds = Vec::new();
        for pr in self
            .review
            .review
            .prs
            .iter_mut()
            .chain(self.review.authored.prs.iter_mut())
            .chain(self.review.bot.prs.iter_mut())
        {
            if pr.repo == repo && pr.number == number {
                if let Some(window) = pr.tmux_window.take() {
                    cmds.push(Command::KillTmuxWindow { window });
                }
                pr.worktree = None;
                pr.agent_status = None;
            }
        }
        cmds.push(Command::UpdateAgentStatus {
            repo,
            number,
            status: None,
        });
        cmds
    }

    fn handle_security_alerts_fetch_failed(&mut self, err: String) -> Vec<Command> {
        self.security.loading = false;
        self.security.last_error = Some(err);
        vec![]
    }

    fn handle_refresh_security_alerts(&mut self) -> Vec<Command> {
        self.security.loading = true;
        vec![Command::FetchSecurityAlerts]
    }

    fn handle_toggle_security_detail(&mut self) -> Vec<Command> {
        self.security.detail_visible = !self.security.detail_visible;
        vec![]
    }

    fn handle_toggle_security_kind_filter(&mut self) -> Vec<Command> {
        self.security.kind_filter = match self.security.kind_filter {
            None => Some(crate::models::AlertKind::Dependabot),
            Some(crate::models::AlertKind::Dependabot) => {
                Some(crate::models::AlertKind::CodeScanning)
            }
            Some(crate::models::AlertKind::CodeScanning) => None,
        };
        self.clamp_security_selection();
        vec![]
    }

    fn handle_start_security_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::SecurityRepoFilter;
        vec![]
    }

    fn handle_close_security_repo_filter(&mut self) -> Vec<Command> {
        self.input.mode = InputMode::Normal;
        self.clamp_security_selection();
        vec![]
    }

    fn handle_toggle_security_repo_filter(&mut self, repo: String) -> Vec<Command> {
        if !self.security.repo_filter.remove(&repo) {
            self.security.repo_filter.insert(repo);
        }
        self.clamp_security_selection();
        vec![]
    }

    fn handle_toggle_all_security_repo_filter(&mut self) -> Vec<Command> {
        let all_repos = self.security.repos.clone();
        if self.security.repo_filter.len() == all_repos.len() {
            self.security.repo_filter.clear();
        } else {
            self.security.repo_filter = all_repos.into_iter().collect();
        }
        self.clamp_security_selection();
        vec![]
    }

    fn handle_toggle_security_repo_filter_mode(&mut self) -> Vec<Command> {
        self.security.repo_filter_mode = match self.security.repo_filter_mode {
            RepoFilterMode::Include => RepoFilterMode::Exclude,
            RepoFilterMode::Exclude => RepoFilterMode::Include,
        };
        self.clamp_security_selection();
        vec![]
    }

    fn handle_dispatch_fix_agent(&mut self, mut req: FixAgentRequest) -> Vec<Command> {
        let fix_key = FixDispatchKey::new(req.github_repo.clone(), req.number, req.kind);
        if self.dispatching_fix.contains(&fix_key) {
            return vec![];
        }
        let known = self.known_repo_paths();
        if let Some(path) = dispatch::resolve_repo_path(&req.github_repo, &known) {
            self.dispatching_fix.insert(fix_key);
            self.set_status(format!(
                "Dispatching fix agent for {}#{}...",
                req.github_repo, req.number
            ));
            req.repo = path;
            vec![Command::DispatchFixAgent(req)]
        } else {
            self.set_status(format!(
                "No local repo found for {} — select a path",
                req.github_repo
            ));
            self.input.pending_dispatch = Some(PendingDispatch::Fix(req));
            self.input.mode = InputMode::InputDispatchRepoPath;
            self.input.buffer.clear();
            self.input.repo_cursor = 0;
            vec![]
        }
    }

    fn handle_fix_agent_dispatched(
        &mut self,
        github_repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        tmux_window: String,
        worktree: String,
    ) -> Vec<Command> {
        self.dispatching_fix
            .remove(&FixDispatchKey::new(github_repo.clone(), number, kind));
        let repo_short = github_repo.split('/').next_back().unwrap_or(&github_repo);
        self.set_status(format!("Fix agent dispatched for {repo_short}#{number}"));
        for alert in self.security.alerts.iter_mut() {
            if alert.repo == github_repo && alert.number == number && alert.kind == kind {
                alert.tmux_window = Some(tmux_window.clone());
                alert.worktree = Some(worktree.clone());
                alert.agent_status = Some(crate::models::ReviewAgentStatus::Reviewing);
                break;
            }
        }
        vec![Command::PersistFixAgent {
            github_repo,
            number,
            kind,
            tmux_window,
            worktree,
        }]
    }

    fn handle_fix_agent_failed(
        &mut self,
        github_repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        error: String,
    ) -> Vec<Command> {
        self.dispatching_fix
            .remove(&FixDispatchKey::new(github_repo, number, kind));
        self.set_status(format!("Fix agent failed: {error}"));
        vec![]
    }

    fn handle_detach_fix_agent(
        &mut self,
        repo: String,
        number: i64,
        kind: crate::models::AlertKind,
    ) -> Vec<Command> {
        let mut cmds = Vec::new();
        for alert in self.security.alerts.iter_mut() {
            if alert.repo == repo && alert.number == number && alert.kind == kind {
                if let Some(window) = alert.tmux_window.take() {
                    cmds.push(Command::KillTmuxWindow { window });
                }
                alert.worktree = None;
                alert.agent_status = None;
            }
        }
        cmds.push(Command::UpdateAgentStatus {
            repo,
            number,
            status: None,
        });
        cmds
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
